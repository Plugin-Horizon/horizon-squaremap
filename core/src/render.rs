//! Full and radius renders: squaremap's `RenderPlan`, `RenderJob`, `RenderCheckpointStore`,
//! `RenderProgressReporter` and `SpiralIterator` (MIT, Copyright (c) 2021-2024 Jason
//! Penilla). A plan lists, per region, the chunks to draw in full and the chunks whose
//! north row is redrawn (its shading reads the chunk north of it). A job runs the plan on
//! its own thread from the region files, saving progress after every region so a stopped
//! render resumes on the next start.
use crate::config::WorldConfig;
use crate::image::RegionImage;
use crate::region::{DiskCache, WorldEnv};
use crate::scan::scan_chunk_with;
use crate::tables::Tables;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Full,
    Radius,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Mode::Full => "FULL",
            Mode::Radius => "RADIUS",
        }
    }
}

/// One region's work: bit `(x & 31) * 32 + (z & 31)` per chunk.
#[derive(Clone, Debug, PartialEq)]
pub struct RegionWork {
    pub rx: i32,
    pub rz: i32,
    full: [u64; 16],
    rows: [u64; 16],
    pub completed: bool,
}

fn bit(cx: i32, cz: i32) -> usize {
    ((cx & 31) * 32 + (cz & 31)) as usize
}

fn set(bits: &mut [u64; 16], i: usize) {
    bits[i / 64] |= 1 << (i % 64);
}

fn get(bits: &[u64; 16], i: usize) -> bool {
    bits[i / 64] >> (i % 64) & 1 != 0
}

fn count(bits: &[u64; 16]) -> usize {
    bits.iter().map(|w| w.count_ones() as usize).sum()
}

impl RegionWork {
    fn new(rx: i32, rz: i32) -> Self {
        Self { rx, rz, full: [0; 16], rows: [0; 16], completed: false }
    }

    fn chunk(&self, i: usize) -> (i32, i32) {
        (self.rx * 32 + (i / 32) as i32, self.rz * 32 + (i % 32) as i32)
    }

    /// Chunks drawn in full.
    pub fn chunk_count(&self) -> usize {
        count(&self.full)
    }

    /// Every chunk of the region's work in squaremap's order (bit order: columns of
    /// increasing x, each north to south), with whether only its north row is drawn.
    pub fn chunks(&self) -> Vec<(i32, i32, bool)> {
        (0..1024)
            .filter_map(|i| {
                if get(&self.full, i) {
                    Some((self.chunk(i), false))
                } else if get(&self.rows, i) {
                    Some((self.chunk(i), true))
                } else {
                    None
                }
            })
            .map(|((x, z), row)| (x, z, row))
            .collect()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub regions: Vec<RegionWork>,
}

impl Plan {
    /// `RenderPlan.regions`: every visible chunk of the listed regions, in full.
    pub fn regions(list: &[(i32, i32)], visible: impl Fn(i32, i32) -> bool) -> Plan {
        let mut regions: Vec<RegionWork> = Vec::new();
        for &(rx, rz) in list {
            if regions.iter().any(|r| (r.rx, r.rz) == (rx, rz)) {
                continue;
            }
            let mut w = RegionWork::new(rx, rz);
            for x in 0..32 {
                for z in 0..32 {
                    let (cx, cz) = (rx * 32 + x, rz * 32 + z);
                    if visible(cx, cz) {
                        set(&mut w.full, bit(cx, cz));
                    }
                }
            }
            regions.push(w);
        }
        Plan { regions }
    }

    /// `RenderPlan.chunks`: the visible chunks, grouped by region in first-seen order,
    /// plus the north row of each one's southern neighbour that isn't drawn in full.
    pub fn chunks(chunks: impl IntoIterator<Item = (i32, i32)>, visible: impl Fn(i32, i32) -> bool) -> Plan {
        let mut index: HashMap<(i32, i32), usize> = HashMap::new();
        let mut regions: Vec<RegionWork> = Vec::new();
        let mut region = |regions: &mut Vec<RegionWork>, cx: i32, cz: i32| -> usize {
            *index.entry((cx >> 5, cz >> 5)).or_insert_with(|| {
                regions.push(RegionWork::new(cx >> 5, cz >> 5));
                regions.len() - 1
            })
        };
        for (cx, cz) in chunks {
            if visible(cx, cz) {
                let r = region(&mut regions, cx, cz);
                set(&mut regions[r].full, bit(cx, cz));
            }
        }
        for r in 0..regions.len() {
            for i in 0..1024 {
                if !get(&regions[r].full, i) {
                    continue;
                }
                let (cx, cz) = regions[r].chunk(i);
                if visible(cx, cz + 1) {
                    let s = region(&mut regions, cx, cz + 1);
                    set(&mut regions[s].rows, bit(cx, cz + 1));
                }
            }
        }
        for w in &mut regions {
            for i in 0..16 {
                w.rows[i] &= !w.full[i];
            }
        }
        Plan { regions }
    }

    /// Drops work that is no longer visible; never adds any.
    pub fn retain(&mut self, visible: impl Fn(i32, i32) -> bool) {
        for w in &mut self.regions {
            for i in 0..1024 {
                let (cx, cz) = w.chunk(i);
                if !visible(cx, cz) {
                    w.full[i / 64] &= !(1 << (i % 64));
                    w.rows[i / 64] &= !(1 << (i % 64));
                }
            }
        }
    }

    pub fn total_chunks(&self) -> usize {
        self.regions.iter().map(RegionWork::chunk_count).sum()
    }

    /// squaremap's `resume_render.json` (version 1): `BitSet.toLongArray` words.
    pub fn to_json(&self, mode: Mode) -> String {
        let words = |b: &[u64; 16]| {
            let n = b.iter().rposition(|&w| w != 0).map_or(0, |i| i + 1);
            b[..n].iter().map(|&w| w as i64).collect::<Vec<_>>()
        };
        let regions: Vec<Value> = self
            .regions
            .iter()
            .map(|w| {
                json!({
                    "coordinate": {"x": w.rx, "z": w.rz},
                    "full": words(&w.full),
                    "rows": words(&w.rows),
                    "completed": w.completed,
                })
            })
            .collect();
        json!({"version": 1, "mode": mode.name(), "regions": regions}).to_string()
    }

    /// Reads [`Plan::to_json`], and squaremap's older format (a list of `[region, done]`).
    pub fn from_json(text: &str) -> Result<(Mode, Plan), String> {
        let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let coord = |c: &Value| -> Result<(i32, i32), String> {
            let n = |k: &str| c.get(k).and_then(Value::as_i64).map(|n| n as i32).ok_or("bad region coordinate");
            Ok((n("x")?, n("z")?))
        };
        let words = |v: Option<&Value>| -> Result<[u64; 16], String> {
            let mut out = [0u64; 16];
            for (i, w) in v.and_then(Value::as_array).map_or(&[][..], Vec::as_slice).iter().enumerate() {
                *out.get_mut(i).ok_or("output mask longer than a region")? = w.as_i64().ok_or("bad mask word")? as u64;
            }
            Ok(out)
        };
        let mut plan = Plan::default();
        let mode = if let Some(list) = v.as_array() {
            for e in list {
                let (rx, rz) = coord(&e[0])?;
                let mut w = RegionWork::new(rx, rz);
                w.full = [u64::MAX; 16];
                w.completed = e[1].as_bool().unwrap_or(false);
                plan.regions.push(w);
            }
            Mode::Full
        } else {
            if v["version"].as_i64() != Some(1) {
                return Err("unsupported render progress version".into());
            }
            let mode = match v["mode"].as_str() {
                Some("FULL") => Mode::Full,
                Some("RADIUS") => Mode::Radius,
                _ => return Err("unsupported render mode".into()),
            };
            for r in v["regions"].as_array().ok_or("no regions")? {
                let (rx, rz) = coord(&r["coordinate"])?;
                let mut w = RegionWork::new(rx, rz);
                w.full = words(r.get("full"))?;
                w.rows = words(r.get("rows"))?;
                if (0..16).any(|i| w.full[i] & w.rows[i] != 0) {
                    return Err("invalid region output masks".into());
                }
                w.completed = r["completed"].as_bool().unwrap_or(false);
                plan.regions.push(w);
            }
            mode
        };
        let mut seen = std::collections::HashSet::new();
        if !plan.regions.iter().all(|w| seen.insert((w.rx, w.rz))) {
            return Err("duplicate region".into());
        }
        Ok((mode, plan))
    }
}

/// `SpiralIterator`: from the centre right, down, left, up, in growing legs, covering the
/// square of `radius` around it.
pub fn spiral(x: i32, z: i32, radius: i32) -> impl Iterator<Item = (i32, i32)> {
    let total = (i64::from(radius) * 2 + 1).pow(2);
    let (mut x, mut z) = (x, z);
    let (mut leg_axis, mut step_leg, mut layer, mut dir) = (0, 0i64, 0i64, 0);
    (0..total).map(move |_| {
        let here = (x, z);
        match dir {
            0 => x += 1,
            1 => z += 1,
            2 => x -= 1,
            _ => z -= 1,
        }
        step_leg += 1;
        if step_leg > layer {
            dir = (dir + 1) % 4;
            step_leg = 0;
            leg_axis += 1;
            if leg_axis > 1 {
                leg_axis = 0;
                layer += 1;
            }
        }
        here
    })
}

/// `RenderFactory.findRegions`: the regions in spiral order from 0,0, the rest after.
pub fn order_regions(mut found: Vec<(i32, i32)>) -> Vec<(i32, i32)> {
    let radius = found.iter().map(|(x, z)| x.abs().max(z.abs())).max().unwrap_or(0);
    let mut remaining: std::collections::HashSet<(i32, i32)> = found.iter().copied().collect();
    let mut ordered = Vec::with_capacity(found.len());
    let mut misses = 0;
    for r in spiral(0, 0, radius) {
        if remaining.is_empty() || misses > 500_000 {
            break;
        }
        if remaining.remove(&r) {
            ordered.push(r);
            misses = 0;
        } else {
            misses += 1;
        }
    }
    found.retain(|r| remaining.contains(r));
    ordered.extend(found);
    ordered
}

/// Shared state of a running job: pause, stop, counters for the progress line.
#[derive(Default)]
pub struct Control {
    /// Stop and drop the saved progress (`cancelrender`).
    pub cancel: AtomicBool,
    /// Stop and keep the saved progress (the server stops).
    pub stop: AtomicBool,
    pub paused: Arc<AtomicBool>,
    pub finished: AtomicBool,
    pub done_chunks: AtomicUsize,
    pub done_regions: AtomicUsize,
    pub total_chunks: AtomicUsize,
    pub total_regions: AtomicUsize,
    /// Images handed to the render thread and not yet written.
    pub unsaved: Arc<AtomicUsize>,
    /// Restart progress logging now (`progresslogging` changed).
    pub log_now: AtomicBool,
    pub failed_chunks: AtomicU64,
}

impl Control {
    fn halted(&self) -> bool {
        self.cancel.load(Ordering::Relaxed) || self.stop.load(Ordering::Relaxed)
    }
}

/// `settings.render-progress-logging`, changed by `/squaremap progresslogging`.
pub static PROGRESS_LOGGING: AtomicBool = AtomicBool::new(true);
pub static PROGRESS_INTERVAL: AtomicU64 = AtomicU64::new(1);

/// What a job needs; everything is owned or shared, nothing borrows the runtime.
pub struct Job {
    pub world_id: &'static str,
    pub mode: Mode,
    pub plan: Plan,
    pub checkpoint: PathBuf,
    pub cache: DiskCache,
    pub env: Arc<WorldEnv>,
    pub tables: Arc<Tables>,
    pub config: Arc<WorldConfig>,
    /// Whether a chunk is loaded now (it is drawn from memory instead, see `live`).
    pub loaded: Box<dyn Fn(i32, i32) -> bool + Send>,
    /// Hands a loaded chunk to the live renderer.
    pub live: Box<dyn Fn(i32, i32) + Send>,
    /// Writes a finished region image (on the render thread, which orders all writes).
    pub save: Box<dyn Fn(RegionImage) + Send>,
    pub control: Arc<Control>,
}

/// At most this many of a job's region images wait to be written.
const MAX_UNSAVED: usize = 4;

/// Runs the job to its end; true when it completed.
pub fn run(mut job: Job) -> bool {
    let c = job.control.clone();
    c.total_chunks.store(job.plan.total_chunks(), Ordering::Relaxed);
    c.total_regions.store(job.plan.regions.len(), Ordering::Relaxed);
    let done: usize = job.plan.regions.iter().filter(|w| w.completed).map(RegionWork::chunk_count).sum();
    c.done_chunks.store(done, Ordering::Relaxed);
    c.done_regions.store(job.plan.regions.iter().filter(|w| w.completed).count(), Ordering::Relaxed);
    save_checkpoint(&job.checkpoint, job.mode, &job.plan);
    let mut progress = Progress::new(done);
    let mut outcome_complete = true;
    'regions: for r in 0..job.plan.regions.len() {
        if job.plan.regions[r].completed {
            continue;
        }
        let work = job.plan.regions[r].clone();
        let mut image = RegionImage::new(work.rx, work.rz);
        let mut drew = false;
        // Region files change under a running server: read their headers afresh.
        job.cache.forget_files();
        for (cx, cz, row_only) in work.chunks() {
            loop {
                if c.halted() {
                    outcome_complete = false;
                    break 'regions;
                }
                if !c.paused.load(Ordering::Relaxed) && c.unsaved.load(Ordering::Relaxed) < MAX_UNSAVED {
                    break;
                }
                progress.tick(&c, job.world_id);
                std::thread::sleep(Duration::from_millis(100));
            }
            if (job.loaded)(cx, cz) {
                (job.live)(cx, cz);
            } else if job.cache.exists(cx, cz) {
                let view = job.cache.view(cx, cz, job.env.clone());
                if let Some(view) = view {
                    let cache = RefCell::new(&mut job.cache);
                    let fallback = |qx: i32, qy: i32, qz: i32| cache.borrow_mut().noise_biome(qx, qy, qz);
                    let px = scan_chunk_with(&view, &job.tables, &job.config.map, row_only, Some(&fallback));
                    image.put_chunk(cx, cz, &px);
                    drew = true;
                }
            }
            for ((x, z), e) in job.cache.errors.drain(..) {
                if c.failed_chunks.fetch_add(1, Ordering::Relaxed) < 20 {
                    eprintln!("[squaremap] exception mapping chunk at [{x}, {z}] in {}: {e}", job.world_id);
                }
            }
            if !row_only {
                c.done_chunks.fetch_add(1, Ordering::Relaxed);
            }
            progress.tick(&c, job.world_id);
        }
        if drew {
            c.unsaved.fetch_add(1, Ordering::Relaxed);
            (job.save)(image);
        }
        job.plan.regions[r].completed = true;
        c.done_regions.fetch_add(1, Ordering::Relaxed);
        save_checkpoint(&job.checkpoint, job.mode, &job.plan);
    }
    progress.close(&c, job.world_id);
    if outcome_complete || c.cancel.load(Ordering::Relaxed) {
        let _ = std::fs::remove_file(&job.checkpoint);
    }
    c.finished.store(true, Ordering::Relaxed);
    outcome_complete
}

fn save_checkpoint(file: &Path, mode: Mode, plan: &Plan) {
    let write = || -> std::io::Result<()> {
        if let Some(p) = file.parent() {
            std::fs::create_dir_all(p)?;
        }
        let tmp = file.with_extension("json.tmp");
        std::fs::write(&tmp, plan.to_json(mode))?;
        std::fs::rename(tmp, file)
    };
    if let Err(e) = write() {
        eprintln!("[squaremap] failed to save render progress to '{}': {e}", file.display());
    }
}

/// Reads a saved plan; an unreadable one is logged and deleted (`RenderCheckpointStore.read`).
pub fn read_checkpoint(file: &Path) -> Option<(Mode, Plan)> {
    let text = std::fs::read_to_string(file).ok()?;
    match Plan::from_json(&text) {
        Ok(p) => Some(p),
        Err(e) => {
            eprintln!("[squaremap] discarding unreadable render progress from '{}': {e}", file.display());
            let _ = std::fs::remove_file(file);
            None
        }
    }
}

/// `RenderProgressReporter`: a line every `interval-seconds` while logging is on.
struct Progress {
    start: Instant,
    last_sample: Instant,
    prev: usize,
    window: VecDeque<usize>,
    sampled: usize,
    samples: u64,
    until_log: u64,
}

impl Progress {
    fn new(done: usize) -> Self {
        let now = Instant::now();
        Self { start: now, last_sample: now, prev: done, window: VecDeque::new(), sampled: 0, samples: 0, until_log: 0 }
    }

    /// Takes a sample once a second (not while paused) and logs when due.
    fn tick(&mut self, c: &Control, world: &str) {
        if self.last_sample.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.last_sample = Instant::now();
        if c.paused.load(Ordering::Relaxed) {
            return;
        }
        let cur = c.done_chunks.load(Ordering::Relaxed);
        let diff = cur.saturating_sub(self.prev);
        self.prev = cur;
        self.window.push_back(diff);
        if self.window.len() > 20 {
            self.window.pop_front();
        }
        self.sampled += diff;
        self.samples += 1;
        if c.log_now.swap(false, Ordering::Relaxed) {
            self.until_log = 0;
        }
        if !PROGRESS_LOGGING.load(Ordering::Relaxed) {
            return;
        }
        if self.until_log > 0 {
            self.until_log -= 1;
            return;
        }
        self.until_log = PROGRESS_INTERVAL.load(Ordering::Relaxed).max(1) - 1;
        eprintln!("[squaremap] {}", self.line(c, world, cur));
    }

    fn close(&mut self, c: &Control, world: &str) {
        if PROGRESS_LOGGING.load(Ordering::Relaxed) {
            eprintln!("[squaremap] {}", self.line(c, world, c.done_chunks.load(Ordering::Relaxed)));
        }
    }

    fn line(&self, c: &Control, world: &str, cur: usize) -> String {
        let total = c.total_chunks.load(Ordering::Relaxed);
        let average = if self.samples == 0 { 0.0 } else { self.sampled as f64 / self.samples as f64 };
        let rate = if self.window.is_empty() { 0.0 } else { self.window.iter().sum::<usize>() as f64 / self.window.len() as f64 };
        progress_line(
            world,
            (c.done_regions.load(Ordering::Relaxed), c.total_regions.load(Ordering::Relaxed)),
            (cur, total),
            self.start.elapsed().as_millis() as u64,
            average,
            rate,
        )
    }
}

/// `LOG_RENDER_PROGRESS_WITH_REGIONS`.
pub fn progress_line(world: &str, regions: (usize, usize), chunks: (usize, usize), elapsed_ms: u64, average: f64, rate: f64) -> String {
    let left = chunks.1.saturating_sub(chunks.0);
    let eta = if left == 0 {
        hms(0)
    } else if average > 0.0 {
        hms((left as f64 * 1000.0 / average) as u64)
    } else {
        "unknown".into()
    };
    let percent = if chunks.1 == 0 { 1.0 } else { chunks.0 as f64 / chunks.1 as f64 };
    format!(
        "({:.2}%) World: {world} Regions: {}/{} Chunks: {}/{} Elapsed: {} ETA: {eta} Rate: {rate:.1} cps",
        percent * 100.0,
        regions.0,
        regions.1,
        chunks.0,
        chunks.1,
        hms(elapsed_ms),
    )
}

fn hms(ms: u64) -> String {
    let s = ms / 1000;
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, DEFAULT_TOML};
    use crate::region::testdata::{chunk, region};
    use crate::region::{Ids, Shape};
    use crate::tables::RawState;
    use std::sync::Mutex;

    #[test]
    fn spiral_matches_squaremap() {
        let s: Vec<_> = spiral(0, 0, 1).collect();
        assert_eq!(s, [(0, 0), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1)]);
        assert_eq!(spiral(5, 5, 2).count(), 25);
        assert_eq!(order_regions(vec![(3, 0), (0, 0), (-1, 0), (900_000, 0)])[..3], [(0, 0), (-1, 0), (3, 0)]);
    }

    #[test]
    fn chunk_plans_add_the_southern_north_rows() {
        let p = Plan::chunks([(0, 0), (0, 1), (5, 31)], |_, _| true);
        // (0, 2) and (5, 32) get their north row; (0, 1) is drawn in full already.
        assert_eq!(p.regions.len(), 2);
        assert_eq!(p.regions[0].chunks(), [(0, 0, false), (0, 1, false), (0, 2, true), (5, 31, false)]);
        assert_eq!(p.regions[1].chunks(), [(5, 32, true)]);
        assert_eq!(p.total_chunks(), 3);
        let hidden = Plan::chunks([(0, 0)], |_, z| z == 0);
        assert_eq!(hidden.regions[0].chunks(), [(0, 0, false)]);
    }

    #[test]
    fn region_plans_and_retain() {
        let mut p = Plan::regions(&[(-1, 0), (-1, 0)], |x, _| x >= -2);
        assert_eq!(p.regions.len(), 1);
        assert_eq!(p.total_chunks(), 64);
        assert_eq!(p.regions[0].chunks()[0], (-2, 0, false));
        p.retain(|_, z| z < 10);
        assert_eq!(p.total_chunks(), 20);
    }

    #[test]
    fn checkpoints_round_trip_and_read_the_old_format() {
        let mut p = Plan::chunks([(0, 0), (40, -3)], |_, _| true);
        p.regions[1].completed = true;
        let text = p.to_json(Mode::Radius);
        assert_eq!(Plan::from_json(&text).unwrap(), (Mode::Radius, p));
        let (mode, old) = Plan::from_json(r#"[[{"x":1,"z":2},true],[{"x":0,"z":0},false]]"#).unwrap();
        assert_eq!(mode, Mode::Full);
        assert_eq!(old.total_chunks(), 2048);
        assert!(old.regions[0].completed && !old.regions[1].completed);
        assert!(Plan::from_json(r#"{"version":2}"#).is_err());
        assert!(Plan::from_json(r#"{"version":1,"mode":"FULL","regions":[{"coordinate":{"x":0,"z":0},"full":[1],"rows":[1]}]}"#).is_err());
    }

    #[test]
    fn progress_line_reads_like_squaremap() {
        let l = progress_line("minecraft:overworld", (1, 4), (512, 2048), 61_000, 10.0, 12.34);
        assert_eq!(l, "(25.00%) World: minecraft:overworld Regions: 1/4 Chunks: 512/2048 Elapsed: 00:01:01 ETA: 00:02:33 Rate: 12.3 cps");
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("test-scratch").join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_job_draws_saved_chunks_hands_over_loaded_ones_and_clears_its_checkpoint() {
        let dir = scratch("job");
        let palette: &[(&str, &[(&str, &str)])] = &[("minecraft:air", &[]), ("minecraft:stone", &[])];
        let c = |cx, cz| chunk(cx, cz, 0, palette, |_, y, _| u64::from(y <= 2), "minecraft:plains", None);
        std::fs::write(dir.join("r.0.0.mca"), region(&[((0, 0), c(0, 0)), ((1, 0), c(1, 0)), ((0, 1), c(0, 1))])).unwrap();
        let states = vec![
            RawState { name: "minecraft:air".into(), ..Default::default() },
            RawState { name: "minecraft:stone".into(), map_color: 11, default_map_color: 11, ..Default::default() },
        ];
        let biomes = vec![crate::tables::RawBiome { name: "minecraft:plains".into(), ..Default::default() }];
        let config = Config::parse(DEFAULT_TOML).unwrap().world("minecraft:overworld");
        let tables = Arc::new(Tables::build(&states, &biomes, &config));
        let ids = Arc::new(Ids::new(&states, &["minecraft:plains".into()]));
        let shape = Shape { min_y: 0, top_y: 32, ceiling: false };
        let saved = Arc::new(Mutex::new(Vec::new()));
        let live = Arc::new(Mutex::new(Vec::new()));
        let control = Arc::new(Control::default());
        let (s, l, u) = (saved.clone(), live.clone(), control.unsaved.clone());
        let job = Job {
            world_id: "minecraft:overworld",
            mode: Mode::Radius,
            plan: Plan::chunks(spiral(0, 0, 1), |_, _| true),
            checkpoint: dir.join("data/resume_render.json"),
            cache: DiskCache::new(dir.clone(), ids, shape),
            env: Arc::new(WorldEnv::plain()),
            tables,
            config: Arc::new(config),
            loaded: Box::new(|x, z| (x, z) == (1, 0)),
            live: Box::new(move |x, z| l.lock().unwrap().push((x, z))),
            save: Box::new(move |img| {
                s.lock().unwrap().push(img);
                u.fetch_sub(1, Ordering::Relaxed);
            }),
            control: control.clone(),
        };
        PROGRESS_LOGGING.store(false, Ordering::Relaxed);
        assert!(run(job));
        assert_eq!(*live.lock().unwrap(), [(1, 0)]);
        let saved = saved.lock().unwrap();
        assert_eq!(saved.len(), 1); // region 0,0 only: the other regions have no saved chunks
        assert_eq!(control.done_chunks.load(Ordering::Relaxed), 9);
        assert!(!dir.join("data/resume_render.json").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_stopped_job_keeps_its_checkpoint() {
        let dir = scratch("job-stop");
        let control = Arc::new(Control::default());
        control.stop.store(true, Ordering::Relaxed);
        let config = Config::parse(DEFAULT_TOML).unwrap().world("minecraft:overworld");
        let job = Job {
            world_id: "minecraft:overworld",
            mode: Mode::Full,
            plan: Plan::regions(&[(0, 0)], |_, _| true),
            checkpoint: dir.join("resume_render.json"),
            cache: DiskCache::new(dir.clone(), Arc::new(Ids::new(&[], &[])), Shape::vanilla("minecraft:overworld")),
            env: Arc::new(WorldEnv::plain()),
            tables: Arc::new(Tables::build(&[], &[], &config)),
            config: Arc::new(config),
            loaded: Box::new(|_, _| false),
            live: Box::new(|_, _| {}),
            save: Box::new(|_| {}),
            control,
        };
        assert!(!run(job));
        let (mode, plan) = read_checkpoint(&dir.join("resume_render.json")).unwrap();
        assert_eq!((mode, plan.total_chunks()), (Mode::Full, 1024));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
