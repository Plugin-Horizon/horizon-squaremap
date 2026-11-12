//! The running map: which chunks wait to be rendered, the render thread that turns chunk
//! snapshots into tiles, full and radius renders from the region files, the JSON documents
//! and the web server. Every public function here is called from the server thread; the
//! render, job and web threads only get owned or shared data.
use crate::config::{Config, Hide, WorldConfig};
use crate::image::RegionImage;
use crate::json::{self, Player, WORLDS};
use crate::region::{region_dir, DiskCache, Ids, Shape, WorldEnv};
use crate::render::{self, Control, Mode, Plan};
use crate::scan::{scan_chunk_with, ChunkView};
use crate::tables::{RawBiome, RawState, Tables};
use crate::web::{JsonStore, WebServer};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Snapshots handed to the render thread and not yet drawn. Each holds its chunk and
/// neighbours, so this bounds the memory the queue keeps alive.
pub const MAX_IN_FLIGHT: usize = 256;
/// Region images are written at least this often while chunks keep coming.
const FLUSH_EVERY: Duration = Duration::from_secs(5);
/// The render thread reopens region files this often (for biomes next to a snapshot).
const DISK_REFRESH: Duration = Duration::from_secs(30);

static CONFIG: Mutex<Option<Config>> = Mutex::new(None);
static RUNTIME: Mutex<Option<Runtime>> = Mutex::new(None);
/// Chunks generated before the map started (the spawn area comes before the world is ready).
static EARLY: Mutex<Vec<(usize, i32, i32)>> = Mutex::new(Vec::new());
/// Chunks loaded now, per level: a render job draws these from memory.
static LOADED: Mutex<Option<HashSet<(usize, i32, i32)>>> = Mutex::new(None);
/// Loaded chunks a render job handed over, drawn with the next batch.
static HANDOFF: Mutex<Vec<(usize, i32, i32)>> = Mutex::new(Vec::new());
/// Each level's height range as it last ticked.
static SHAPES: Mutex<[Option<Shape>; 3]> = Mutex::new([None; 3]);
/// What `start` was given, for `/squaremap reload`.
static STARTED: Mutex<Option<StartArgs>> = Mutex::new(None);

#[derive(Clone)]
struct StartArgs {
    dir: PathBuf,
    states: Vec<RawState>,
    biomes: Vec<RawBiome>,
    env: Arc<WorldEnv>,
}

struct JobHandle {
    mode: Mode,
    control: Arc<Control>,
    thread: Option<thread::JoinHandle<()>>,
}

struct World {
    id: &'static str,
    config: Arc<WorldConfig>,
    tables: Arc<Tables>,
    /// Chunks changed since they were last drawn.
    dirty: HashSet<(i32, i32)>,
    /// Dirty chunks that were unloaded before their turn: drawn when they load again.
    stale: HashSet<(i32, i32)>,
    /// This interval's batch, drawn as the level ticks.
    ready: VecDeque<(i32, i32)>,
    next_batch: Instant,
    next_markers: Instant,
    markers: String,
    /// The world border (`[min_x, min_z, max_x, max_z]`) as last published.
    border: Option<[f64; 4]>,
    /// `pauserender`: pauses this level's full, radius and background renders.
    paused: Arc<AtomicBool>,
    job: Option<JobHandle>,
}

struct Runtime {
    web: PathBuf,
    dir: PathBuf,
    config: Config,
    raw: (Vec<RawState>, Vec<RawBiome>),
    ids: Arc<Ids>,
    env: Arc<WorldEnv>,
    worlds: Vec<World>,
    jobs: mpsc::Sender<Work>,
    render: Option<thread::JoinHandle<()>>,
    in_flight: Arc<AtomicUsize>,
    json: Arc<JsonStore>,
    server: Option<WebServer>,
    next_players: Instant,
    players: String,
}

enum Work {
    Chunk {
        world: usize,
        view: Box<dyn ChunkView>,
        tables: Arc<Tables>,
        config: Arc<WorldConfig>,
        region_dir: Arc<PathBuf>,
        ids: Arc<Ids>,
    },
    /// A finished region of a full or radius render.
    Region { world: usize, image: RegionImage, max_zoom: i32, unsaved: Arc<AtomicUsize> },
    /// `resetmap`: drop the level's pending images and empty its tiles folder.
    Reset { world: usize, done: mpsc::Sender<std::io::Result<()>> },
    Stop,
}

/// The default `config/squaremap-server.toml`.
pub fn default_config() -> &'static str {
    crate::config::DEFAULT_TOML
}

/// The config file was (re)read. A running map picks up everything but the web server.
pub fn config_loaded(text: &str) {
    let config = match Config::parse(text) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[squaremap] config not loaded, keeping the previous one: {e}");
            return;
        }
    };
    let q = &config.settings.image_quality.compress_images;
    crate::image::set_compression(q.enabled, q.value);
    let p = &config.settings.render_progress_logging;
    render::PROGRESS_LOGGING.store(p.enabled, Ordering::Relaxed);
    render::PROGRESS_INTERVAL.store(p.interval_seconds, Ordering::Relaxed);
    *CONFIG.lock().unwrap() = Some(config.clone());
    if let Some(rt) = RUNTIME.lock().unwrap().as_mut() {
        rt.config = config;
        rt.apply_config();
    }
}

pub(crate) fn config() -> Config {
    CONFIG.lock().unwrap().clone().unwrap_or_else(|| Config::parse(default_config()).unwrap_or_default())
}

pub fn running() -> bool {
    RUNTIME.lock().unwrap().is_some()
}

/// The index of a level (`minecraft:the_nether`) among [`WORLDS`].
pub fn world_index(id: &str) -> Option<usize> {
    WORLDS.iter().position(|(w, _)| *w == id)
}

/// A level's height range and ceiling, as it ticks (render jobs read the region files of
/// levels that aren't open, with the shape they last had, else vanilla's).
pub fn level_shape(world: usize, min_y: i32, top_y: i32, ceiling: bool) {
    if let Some(slot) = SHAPES.lock().unwrap().get_mut(world) {
        *slot = Some(Shape { min_y, top_y, ceiling });
    }
}

fn shape(world: usize) -> Shape {
    SHAPES.lock().unwrap().get(world).copied().flatten().unwrap_or_else(|| Shape::vanilla(WORLDS[world].0))
}

/// Opens the map for a world folder with the game's block states and biomes, and what the
/// game knows of biomes beyond the saved data (`env`). Resumes renders a stop interrupted.
pub fn start(world_dir: &Path, states: Vec<RawState>, biomes: Vec<RawBiome>, env: WorldEnv) -> Result<(), String> {
    let args = StartArgs { dir: world_dir.to_path_buf(), states, biomes, env: Arc::new(env) };
    *STARTED.lock().unwrap() = Some(args.clone());
    start_with(args)
}

fn start_with(args: StartArgs) -> Result<(), String> {
    stop();
    let StartArgs { dir: world_dir, states, biomes, env } = args;
    let config = config();
    let web_path = Path::new(&config.settings.web_directory.path);
    let web = if web_path.is_absolute() { web_path.to_path_buf() } else { world_dir.join(web_path) };
    if config.settings.web_directory.auto_update {
        write_frontend(&web).map_err(|e| format!("writing the web folder {}: {e}", web.display()))?;
    }
    for (key, png) in crate::api::icons() {
        write_icon_into(&web, &key, &png);
    }
    let json = Arc::new(JsonStore::default());
    let server = if config.settings.internal_webserver.enabled {
        let s = &config.settings.internal_webserver;
        let server = WebServer::start(&s.bind, s.port, web.clone(), json.clone())
            .map_err(|e| format!("web server on {}:{}: {e}", s.bind, s.port))?;
        eprintln!("[squaremap] web server on http://{}", server.addr());
        Some(server)
    } else {
        None
    };
    let (jobs, rx) = mpsc::channel();
    let in_flight = Arc::new(AtomicUsize::new(0));
    let tiles = web.join("tiles");
    let counter = in_flight.clone();
    let render = thread::Builder::new()
        .name("squaremap-render".into())
        .spawn(move || render_loop(rx, tiles, counter))
        .map_err(|e| e.to_string())?;
    let biome_names: Vec<String> = biomes.iter().map(|b| b.name.clone()).collect();
    let ids = Arc::new(Ids::new(&states, &biome_names));
    let data = world_dir.join("squaremap").join("data");
    crate::api::load_hidden(data.join("hidden-players.json"));
    let now = Instant::now();
    let mut rt = Runtime {
        web,
        dir: world_dir,
        config,
        raw: (states, biomes),
        ids,
        env,
        worlds: Vec::new(),
        jobs,
        render: Some(render),
        in_flight,
        json,
        server,
        next_players: now,
        players: String::new(),
    };
    rt.apply_config();
    for (w, cx, cz) in EARLY.lock().unwrap().drain(..) {
        if let Some(w) = rt.worlds.get_mut(w) {
            w.dirty.insert((cx, cz));
        }
    }
    for w in 0..rt.worlds.len() {
        if !rt.worlds[w].config.map.enabled {
            continue;
        }
        if let Some((mode, mut plan)) = render::read_checkpoint(&rt.checkpoint(w)) {
            let config = rt.worlds[w].config.clone();
            plan.retain(|x, z| config.map.chunk_visible(x, z));
            eprintln!("[squaremap] Rendering map for {} has been resumed", WORLDS[w].0);
            rt.spawn_job(w, mode, Box::new(move |_| plan));
        }
    }
    *RUNTIME.lock().unwrap() = Some(rt);
    Ok(())
}

/// Draws what is queued, writes the JSON documents and stops the threads. A full or
/// radius render in progress keeps its saved progress and resumes on the next start.
pub fn stop() {
    let Some(mut rt) = RUNTIME.lock().unwrap().take() else { return };
    for w in &mut rt.worlds {
        if let Some(j) = w.job.as_mut() {
            j.control.stop.store(true, Ordering::Relaxed);
        }
    }
    for w in &mut rt.worlds {
        if let Some(t) = w.job.as_mut().and_then(|j| j.thread.take()) {
            let _ = t.join();
        }
    }
    let _ = rt.jobs.send(Work::Stop);
    if let Some(t) = rt.render.take() {
        let _ = t.join();
    }
    rt.server = None;
    if let Err(e) = rt.json.flush(&rt.web) {
        eprintln!("[squaremap] writing the JSON documents: {e}");
    }
    HANDOFF.lock().unwrap().clear();
}

/// `/squaremap reload`: the map stops and starts again with the config as it is now (the
/// web server's address included); hides that weren't persistent are dropped.
pub fn reload() -> Result<(), String> {
    let args = STARTED.lock().unwrap().clone().ok_or("the map has not started")?;
    crate::api::clear_temporary_hides();
    start_with(args)
}

/// The world closed: forget its loaded chunks too.
pub fn world_closed() {
    stop();
    *LOADED.lock().unwrap() = None;
}

fn with<T>(f: impl FnOnce(&mut Runtime) -> T) -> Option<T> {
    RUNTIME.lock().unwrap().as_mut().map(f)
}

/// A chunk joined level `world`: a newly generated one is drawn
/// (`ChunkPopulateEvent`), one that was dirty when it unloaded is drawn now.
pub fn chunk_loaded(world: usize, cx: i32, cz: i32, new: bool) {
    LOADED.lock().unwrap().get_or_insert_with(HashSet::new).insert((world, cx, cz));
    let started = with(|rt| {
        if let Some(w) = rt.worlds.get_mut(world) {
            if new || w.stale.remove(&(cx, cz)) {
                w.dirty.insert((cx, cz));
            }
        }
    });
    if started.is_none() && new {
        EARLY.lock().unwrap().push((world, cx, cz));
    }
}

/// A chunk left level `world`.
pub fn chunk_unloaded(world: usize, cx: i32, cz: i32) {
    if let Some(l) = LOADED.lock().unwrap().as_mut() {
        l.remove(&(world, cx, cz));
    }
}

fn is_loaded(world: usize, cx: i32, cz: i32) -> bool {
    LOADED.lock().unwrap().as_ref().is_some_and(|l| l.contains(&(world, cx, cz)))
}

/// A block of level `world` changed.
pub fn block_changed(world: usize, x: i32, z: i32) {
    with(|rt| {
        if let Some(w) = rt.worlds.get_mut(world) {
            w.dirty.insert((x >> 4, z >> 4));
        }
    });
}

/// The chunks of level `world` to snapshot now: every `interval-seconds` up to
/// `max-chunks-per-interval` dirty chunks join the batch, which is handed out as
/// the render thread keeps up. Loaded chunks a render job handed over come first.
pub fn due_chunks(world: usize) -> Vec<(i32, i32)> {
    let handed: Vec<(i32, i32)> = {
        let mut h = HANDOFF.lock().unwrap();
        let (mine, rest): (Vec<_>, Vec<_>) = h.drain(..).partition(|(w, _, _)| *w == world);
        *h = rest;
        mine.into_iter().map(|(_, x, z)| (x, z)).collect()
    };
    with(|rt| {
        let free = MAX_IN_FLIGHT.saturating_sub(rt.in_flight.load(Ordering::Relaxed));
        let Some(w) = rt.worlds.get_mut(world) else { return Vec::new() };
        if w.job.as_ref().is_some_and(|j| j.control.finished.load(Ordering::Relaxed)) {
            if let Some(t) = w.job.take().and_then(|mut j| j.thread.take()) {
                let _ = t.join();
            }
        }
        for c in handed {
            if !w.ready.contains(&c) {
                w.ready.push_front(c);
            }
        }
        if w.paused.load(Ordering::Relaxed) {
            return Vec::new();
        }
        let bg = &w.config.map.render.background;
        let now = Instant::now();
        if bg.enabled && now >= w.next_batch {
            w.next_batch = now + Duration::from_secs(bg.interval_seconds.max(1));
            let take: Vec<_> = w.dirty.iter().copied().take(bg.max_chunks_per_interval).collect();
            for c in take {
                w.dirty.remove(&c);
                if w.config.map.chunk_visible(c.0, c.1) {
                    w.ready.push_back(c);
                }
            }
        }
        let n = free.min(w.ready.len());
        w.ready.drain(..n).collect()
    })
    .unwrap_or_default()
}

/// A due chunk wasn't loaded: drawn once it loads again.
pub fn not_loaded(world: usize, cx: i32, cz: i32) {
    with(|rt| {
        if let Some(w) = rt.worlds.get_mut(world) {
            w.stale.insert((cx, cz));
        }
    });
}

/// Hands a snapshot of a due chunk to the render thread.
pub fn submit(world: usize, view: Box<dyn ChunkView>) {
    with(|rt| {
        let Some(w) = rt.worlds.get(world) else { return };
        let job = Work::Chunk {
            world,
            view,
            tables: w.tables.clone(),
            config: w.config.clone(),
            region_dir: Arc::new(region_dir(&rt.dir, w.id)),
            ids: rt.ids.clone(),
        };
        rt.in_flight.fetch_add(1, Ordering::Relaxed);
        if rt.jobs.send(job).is_err() {
            rt.in_flight.fetch_sub(1, Ordering::Relaxed);
        }
    });
}

/// Whether the player list is due (`player-tracker.update-interval-seconds`).
pub fn players_due() -> bool {
    with(|rt| {
        let now = Instant::now();
        let every = rt.worlds.first().map_or(1, |w| w.config.player_tracker.update_interval_seconds);
        let due = now >= rt.next_players;
        if due {
            rt.next_players = now + Duration::from_secs(every.max(1));
        }
        due
    })
    .unwrap_or(false)
}

/// The hide rules of a level's player tracker.
pub fn hide_rules(world: &str) -> Hide {
    config().world(world).player_tracker.hide
}

/// Publishes `/tiles/players.json` (players already filtered by [`hide_rules`]; players
/// hidden with `/squaremap hide` or the API are left out here).
pub fn publish_players(list: &[Player], max: usize) {
    with(|rt| {
        let shown: Vec<Player> = list.iter().filter(|p| !crate::api::player_hidden(p.uuid)).cloned().collect();
        let doc = json::players(&shown, max, |w| {
            world_index(w).and_then(|i| rt.worlds.get(i)).map_or((false, false), |w| {
                (w.config.player_tracker.enabled, w.config.player_tracker.use_display_names)
            })
        })
        .to_string();
        if doc != rt.players {
            rt.players = doc.clone();
            rt.put("/tiles/players.json", doc);
        }
    });
}

/// Whether level `world`'s markers are due (`markers.update-interval-seconds`).
pub fn markers_due(world: usize) -> bool {
    with(|rt| {
        let now = Instant::now();
        rt.worlds.get_mut(world).is_some_and(|w| {
            let due = now >= w.next_markers;
            if due {
                w.next_markers = now + Duration::from_secs(w.config.map.markers.update_interval_seconds.max(1));
            }
            due
        })
    })
    .unwrap_or(false)
}

/// Publishes level `world`'s spawn and border (`[min_x, min_z, max_x, max_z]`) and the
/// layers other mods registered: its `markers.json`, and its `settings.json`, which
/// carries the spawn. A `world-border` visibility limit follows the border from here.
pub fn publish_markers(world: usize, spawn: Option<[i32; 2]>, border: [f64; 4]) {
    with(|rt| {
        let Some(w) = rt.worlds.get_mut(world) else { return };
        if w.border != Some(border) {
            w.border = Some(border);
            if w.config.map.visibility_limits.iter().any(|l| l.kind == "world-border") {
                let mut c = (*w.config).clone();
                c.map.resolve(Some(border));
                w.config = Arc::new(c);
            }
        }
        let key = format!("{spawn:?}{border:?}{}", crate::api::generation());
        if key == w.markers {
            return;
        }
        w.markers = key;
        let name = json::web_name(w.id);
        let settings = json::world_settings(&w.config, spawn.unwrap_or_default()).to_string();
        let mut layers = json::markers(&w.config, spawn, border, now_millis());
        if let serde_json::Value::Array(list) = &mut layers {
            list.extend(crate::api::layers_json(w.id));
        }
        rt.put(&format!("/tiles/{name}/settings.json"), settings);
        rt.put(&format!("/tiles/{name}/markers.json"), layers.to_string());
    });
}

fn now_millis() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// Why a render command was refused, in squaremap's words.
#[derive(Debug, PartialEq, Eq)]
pub enum RenderRefused {
    NotRunning,
    NotEnabled,
    InProgress,
    NotInProgress,
}

/// `fullrender`: every saved chunk of the level, region by region.
pub fn start_full_render(world: usize) -> Result<(), RenderRefused> {
    with(|rt| {
        rt.check_idle(world)?;
        let dir = region_dir(&rt.dir, WORLDS[world].0);
        let config = rt.worlds[world].config.clone();
        let id = WORLDS[world].0;
        eprintln!("[squaremap] Started full map render for {id}");
        rt.spawn_job(
            world,
            Mode::Full,
            Box::new(move |_| {
                eprintln!("[squaremap] Scanning region files for {id}... (this may take a moment)");
                let found: Vec<_> =
                    crate::region::list_regions(&dir).into_iter().filter(|&(x, z)| config.map.region_visible(x, z)).collect();
                let plan = Plan::regions(&render::order_regions(found), |x, z| config.map.chunk_visible(x, z));
                eprintln!("[squaremap] Found {} region files for {id}", plan.regions.len());
                plan
            }),
        );
        Ok(())
    })
    .unwrap_or(Err(RenderRefused::NotRunning))
}

/// `radiusrender`: the chunks within `radius` blocks (a square) of a block column.
pub fn start_radius_render(world: usize, center: [i32; 2], radius: i32) -> Result<(), RenderRefused> {
    with(|rt| {
        rt.check_idle(world)?;
        let config = rt.worlds[world].config.clone();
        eprintln!("[squaremap] Started radius map render for {}", WORLDS[world].0);
        rt.spawn_job(
            world,
            Mode::Radius,
            Box::new(move |_| {
                Plan::chunks(render::spiral(center[0] >> 4, center[1] >> 4, radius.max(1) >> 4), |x, z| {
                    config.map.chunk_visible(x, z)
                })
            }),
        );
        Ok(())
    })
    .unwrap_or(Err(RenderRefused::NotRunning))
}

/// `cancelrender`: stops the level's full or radius render and drops its progress.
pub fn cancel_render(world: usize) -> Result<(), RenderRefused> {
    let handle = with(|rt| {
        let w = rt.worlds.get_mut(world).ok_or(RenderRefused::NotEnabled)?;
        match w.job.take() {
            Some(j) if !j.control.finished.load(Ordering::Relaxed) => {
                j.control.cancel.store(true, Ordering::Relaxed);
                Ok(j)
            }
            _ => Err(RenderRefused::NotInProgress),
        }
    })
    .unwrap_or(Err(RenderRefused::NotRunning))?;
    // Joined outside the lock: the job reads the loaded chunks through other locks only,
    // but a paused job must see the cancel first.
    if let Some(t) = handle.thread {
        let _ = t.join();
    }
    Ok(())
}

/// `pauserender`: toggles the level's pause; true when it is now paused.
pub fn toggle_pause(world: usize) -> Option<bool> {
    with(|rt| {
        let w = rt.worlds.get(world)?;
        let now = !w.paused.load(Ordering::Relaxed);
        w.paused.store(now, Ordering::Relaxed);
        Some(now)
    })
    .flatten()
}

/// What a level's render is doing: (mode, chunks done, chunks in all, paused).
pub fn render_status(world: usize) -> Option<(Mode, usize, usize, bool)> {
    with(|rt| {
        let w = rt.worlds.get(world)?;
        let j = w.job.as_ref().filter(|j| !j.control.finished.load(Ordering::Relaxed))?;
        let c = &j.control;
        Some((j.mode, c.done_chunks.load(Ordering::Relaxed), c.total_chunks.load(Ordering::Relaxed), w.paused.load(Ordering::Relaxed)))
    })
    .flatten()
}

/// `progresslogging toggle|rate`: the running jobs log their next line now.
pub fn restart_progress_logging() {
    with(|rt| {
        for j in rt.worlds.iter().filter_map(|w| w.job.as_ref()) {
            j.control.log_now.store(true, Ordering::Relaxed);
        }
    });
}

/// `resetmap`: deletes the level's tiles (refused while a render runs).
pub fn reset_map(world: usize) -> Result<(), String> {
    let done = with(|rt| {
        let w = rt.worlds.get_mut(world).ok_or("unknown world")?;
        if w.job.as_ref().is_some_and(|j| !j.control.finished.load(Ordering::Relaxed)) {
            return Err(format!("A render is already in progress on {}", w.id));
        }
        w.dirty.clear();
        w.ready.clear();
        w.stale.clear();
        w.markers.clear();
        let (tx, rx) = mpsc::channel();
        rt.jobs.send(Work::Reset { world, done: tx }).map_err(|e| e.to_string())?;
        Ok(rx)
    })
    .unwrap_or(Err("the map has not started".into()))?;
    done.recv().map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
    with(|rt| {
        let name = json::web_name(WORLDS[world].0);
        let settings = json::world_settings(&rt.worlds[world].config, [0, 0]).to_string();
        rt.put(&format!("/tiles/{name}/settings.json"), settings);
    });
    Ok(())
}

/// Writes an icon an API user registered.
pub(crate) fn write_icon(key: &str, png: &[u8]) {
    if let Some(web) = with(|rt| rt.web.clone()) {
        write_icon_into(&web, key, png);
    }
}

pub(crate) fn remove_icon(key: &str) {
    if let Some(web) = with(|rt| rt.web.clone()) {
        let _ = std::fs::remove_file(web.join("images/icon/registered").join(format!("{key}.png")));
    }
}

fn write_icon_into(web: &Path, key: &str, png: &[u8]) {
    let dir = web.join("images/icon/registered");
    if let Err(e) = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(dir.join(format!("{key}.png")), png)) {
        eprintln!("[squaremap] writing icon {key}: {e}");
    }
}

impl Runtime {
    fn put(&self, path: &str, doc: String) {
        self.json.put(path, doc);
        if self.server.is_none() {
            let _ = self.json.flush(&self.web);
        }
    }

    fn checkpoint(&self, world: usize) -> PathBuf {
        self.dir.join("squaremap").join("data").join(json::web_name(WORLDS[world].0)).join("resume_render.json")
    }

    fn check_idle(&self, world: usize) -> Result<(), RenderRefused> {
        let w = self.worlds.get(world).ok_or(RenderRefused::NotEnabled)?;
        if !w.config.map.enabled {
            return Err(RenderRefused::NotEnabled);
        }
        if w.job.as_ref().is_some_and(|j| !j.control.finished.load(Ordering::Relaxed)) {
            return Err(RenderRefused::InProgress);
        }
        Ok(())
    }

    /// Starts a job thread for `world`; `plan` runs on that thread (listing region files
    /// can take a while).
    fn spawn_job(&mut self, world: usize, mode: Mode, plan: Box<dyn FnOnce(()) -> Plan + Send>) {
        let checkpoint = self.checkpoint(world);
        let id = WORLDS[world].0;
        let cache = DiskCache::new(region_dir(&self.dir, id), self.ids.clone(), shape(world));
        let (env, sender) = (self.env.clone(), self.jobs.clone());
        let w = &mut self.worlds[world];
        if let Some(t) = w.job.take().and_then(|mut j| j.thread.take()) {
            let _ = t.join(); // a finished one
        }
        let control = Arc::new(Control { paused: w.paused.clone(), ..Control::default() });
        let (tables, config) = (w.tables.clone(), w.config.clone());
        let c = control.clone();
        let max_zoom = config.map.zoom.maximum;
        let thread = thread::Builder::new().name(format!("squaremap-render-[{id}]")).spawn(move || {
            let unsaved = c.unsaved.clone();
            let job = render::Job {
                world_id: id,
                mode,
                plan: plan(()),
                checkpoint,
                cache,
                env,
                tables,
                config,
                loaded: Box::new(move |x, z| is_loaded(world, x, z)),
                live: Box::new(move |x, z| HANDOFF.lock().unwrap().push((world, x, z))),
                save: Box::new(move |image| {
                    let work = Work::Region { world, image, max_zoom, unsaved: unsaved.clone() };
                    if sender.send(work).is_err() {
                        unsaved.fetch_sub(1, Ordering::Relaxed);
                    }
                }),
                control: c.clone(),
            };
            let complete = render::run(job);
            if complete {
                eprintln!("[squaremap] Finished rendering map for {id}");
            } else if c.cancel.load(Ordering::Relaxed) {
                eprintln!("[squaremap] Rendering map for {id} has been interrupted");
            }
        });
        match thread {
            Ok(t) => w.job = Some(JobHandle { mode, control, thread: Some(t) }),
            Err(e) => eprintln!("[squaremap] could not start the render of {id}: {e}"),
        }
    }

    /// Builds each level's settings and tables from the config.
    fn apply_config(&mut self) {
        let now = Instant::now();
        let mut old = std::mem::take(&mut self.worlds);
        for (i, (id, _)) in WORLDS.iter().enumerate() {
            let mut config = self.config.world(id);
            let previous = old.get_mut(i);
            let border = previous.as_ref().and_then(|o| o.border);
            if border.is_some() {
                config.map.resolve(border);
            }
            let tables = Tables::build(&self.raw.0, &self.raw.1, &config);
            let mut w = World {
                id,
                config: Arc::new(config),
                tables: Arc::new(tables),
                dirty: HashSet::new(),
                stale: HashSet::new(),
                ready: VecDeque::new(),
                next_batch: now,
                next_markers: now,
                markers: String::new(),
                border,
                paused: Arc::new(AtomicBool::new(false)),
                job: None,
            };
            if let Some(o) = previous {
                w.dirty = std::mem::take(&mut o.dirty);
                w.stale = std::mem::take(&mut o.stale);
                w.ready = std::mem::take(&mut o.ready);
                w.next_batch = o.next_batch;
                w.paused = o.paused.clone();
                w.job = o.job.take();
            }
            let name = json::web_name(id);
            self.json.put(&format!("/tiles/{name}/settings.json"), json::world_settings(&w.config, [0, 0]).to_string());
            self.worlds.push(w);
        }
        self.put("/tiles/settings.json", json::settings(&self.config).to_string());
    }
}

/// Writes the bundled frontend into the web folder, plus the spawn icon squaremap registers.
fn write_frontend(web: &Path) -> std::io::Result<()> {
    for (path, bytes) in crate::assets::FILES {
        let file = web.join(path);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(file, bytes)?;
    }
    let icon = web.join("images/icon/registered");
    std::fs::create_dir_all(&icon)?;
    std::fs::write(icon.join("squaremap-spawn_icon.png"), crate::assets::file("images/icon/spawn.png").unwrap_or_default())
}

/// Deletes what a tiles folder holds, keeping the folder.
fn empty_dir(dir: &Path) -> std::io::Result<()> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Ok(()) };
    for e in entries {
        let p = e?.path();
        if p.is_dir() {
            std::fs::remove_dir_all(p)?;
        } else {
            std::fs::remove_file(p)?;
        }
    }
    Ok(())
}

type Pending = HashMap<(usize, i32, i32), (RegionImage, i32)>;

fn render_loop(rx: mpsc::Receiver<Work>, tiles: PathBuf, in_flight: Arc<AtomicUsize>) {
    let mut regions: Pending = HashMap::new();
    // Per level: saved chunks for biomes next to a snapshot, reopened every DISK_REFRESH.
    let mut disks: HashMap<usize, (DiskCache, Instant)> = HashMap::new();
    let mut last_flush = Instant::now();
    let save = |world: usize, image: &RegionImage, max_zoom: i32| {
        let dir = tiles.join(json::web_name(WORLDS[world].0));
        if let Err(e) = image.save(&dir, max_zoom) {
            eprintln!("[squaremap] writing region {} {}: {e}", image.rx, image.rz);
        }
    };
    let flush = |regions: &mut Pending| {
        for ((world, _, _), (image, max_zoom)) in regions.drain() {
            save(world, &image, max_zoom);
        }
    };
    loop {
        let job = match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(job) => Some(job),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => Some(Work::Stop),
        };
        match job {
            Some(Work::Chunk { world, view, tables, config, region_dir, ids }) => {
                let (cx, cz) = view.chunk();
                let shape = Shape { min_y: view.min_y(), top_y: view.top_y(), ceiling: view.has_ceiling() };
                let fresh = |ids: Arc<Ids>| (DiskCache::new((*region_dir).clone(), ids, shape), Instant::now());
                let disk = disks.entry(world).or_insert_with(|| fresh(ids.clone()));
                if disk.1.elapsed() >= DISK_REFRESH || disk.0.shape() != shape {
                    *disk = fresh(ids);
                }
                let px = {
                    let cache = RefCell::new(&mut disk.0);
                    let fallback = |qx: i32, qy: i32, qz: i32| cache.borrow_mut().noise_biome(qx, qy, qz);
                    let px = scan_chunk_with(&*view, &tables, &config.map, false, Some(&fallback));
                    cache.borrow_mut().errors.clear();
                    px
                };
                drop(view);
                let (rx_, rz_) = (cx >> 5, cz >> 5);
                let entry = regions
                    .entry((world, rx_, rz_))
                    .or_insert_with(|| (RegionImage::new(rx_, rz_), config.map.zoom.maximum));
                entry.0.put_chunk(cx, cz, &px);
                in_flight.fetch_sub(1, Ordering::Relaxed);
            }
            Some(Work::Region { world, image, max_zoom, unsaved }) => {
                // Pending live pixels of the same region are newer: written over it.
                save(world, &image, max_zoom);
                if let Some((live, zoom)) = regions.remove(&(world, image.rx, image.rz)) {
                    save(world, &live, zoom);
                }
                unsaved.fetch_sub(1, Ordering::Relaxed);
            }
            Some(Work::Reset { world, done }) => {
                regions.retain(|(w, _, _), _| *w != world);
                let _ = done.send(empty_dir(&tiles.join(json::web_name(WORLDS[world].0))));
            }
            Some(Work::Stop) => {
                flush(&mut regions);
                return;
            }
            None => {}
        }
        let idle = in_flight.load(Ordering::Relaxed) == 0;
        if !regions.is_empty() && (idle || last_flush.elapsed() >= FLUSH_EVERY) {
            flush(&mut regions);
            last_flush = Instant::now();
        }
    }
}
