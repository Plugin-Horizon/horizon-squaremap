//! squaremap as an arismoko native mod (trait mod on `mod_api`). The renderer, tile and JSON
//! writers, commands and the internal web server are the unchanged `horizon-squaremap` crate
//! (`core/`); this file only wires it to hooks and host methods. See PORT_NOTES.md for what is
//! missing on arismoko and what each fallback loses.
//!
//! The map starts in `server_started` only once the block-state and biome tables can be read
//! (`render_tables`). Until then the commands answer, the config is read and the web server does not start.
//!
//! Config: `config/squaremap-server.toml` (registered as a Server config; a world's
//! `serverconfig/squaremap-server.toml` wins). The web server binds 127.0.0.1 by default.

pub mod horizon_squaremap;
mod chunky_border;
use horizon_squaremap as sm;
use mod_api::{
    BlockChangedEvent, BlockMetadata, ChunkLoadEvent, ChunkUnloadEvent, Command, CommandArgumentValue, CommandSource,
    HostBlockStates,
    Config, ConfigKind, ConfigLoadingEvent, ConfigReloadingEvent, EquipmentSlot, Flow, HostSide, Hooks, Identifier,
    LevelLoadEvent, LevelUnloadEvent, LivingRef, NodeDefault, RegistryEntry,
    PermissionGatherNodesEvent, PermissionLevel, PermissionNode, PermissionValueType, PlayerCommandEvent,
    RegisterCommandsEvent, RegisterNodeCommandsEvent, RegisterConfigsEvent, RunModCommandEvent, ServerHost, ServerStartedEvent,
    ChunkRef, Heightmap, LevelRef, LevelTickPreEvent,
    ModServiceCallEvent, ServerStoppingEvent, ServerTickEvent, ServerTickPhase, Suggestion, TabCompleteEvent, Text, WorldSavedEvent,
};
use mod_api::text::{TextClick, TextHover, TextStyle};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const ID: &str = "squaremap";

/// Whether the chunky mod has accepted `subscribe squaremap` (its border events then arrive at `squaremap:chunky_event`).
static CHUNKY_SUBSCRIBED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Ticks between two tries to subscribe while the chunky mod is absent (five seconds).
const SUBSCRIBE_EVERY: u64 = 100;

/// ChunkyBorder's `map-options`, read when a border is drawn (`config/chunkyborder/config.yml` or `.toml`).
fn border_options() -> chunky_border::Options {
    let dir = Path::new("config/chunkyborder");
    let text = std::fs::read_to_string(dir.join("config.toml")).or_else(|_| std::fs::read_to_string(dir.join("config.yml")));
    text.map(|t| chunky_border::Options::parse(&t)).unwrap_or_default()
}

/// Subscribes to the chunky mod's border events once it is there, and draws the borders it already has.
/// Every failure (no such mod, no such service) is silent and tried again later.
fn subscribe_to_chunky(host: &mut dyn ServerHost) {
    use std::sync::atomic::Ordering;
    if CHUNKY_SUBSCRIBED.load(Ordering::Relaxed) {
        return;
    }
    if host.commands().call_mod_service("chunky:api", format!("subscribe\t{ID}").as_bytes()).is_err() {
        return;
    }
    CHUNKY_SUBSCRIBED.store(true, Ordering::Relaxed);
    let options = border_options();
    for (_, dimension) in sm::open_levels() {
        let line = format!("border_get\t{}", chunky_border::world_of(&dimension));
        let Ok(reply) = host.commands().call_mod_service("chunky:api", line.as_bytes()) else { continue };
        let Ok(reply) = String::from_utf8(reply) else { continue };
        let fields: Vec<&str> = reply.trim().split('\t').collect();
        chunky_border::apply(&dimension, chunky_border::parse_border(&fields).as_ref(), &options);
    }
}

/// Every permission node the map checks: the subcommands' (`squaremap.command.<sub>`, the ones
/// `commands::usages` lists) and the two `.others` nodes. Declared in `permission_gather_nodes`.
const NODES: [&str; 13] = [
    "squaremap.command.help",
    "squaremap.command.cancelrender",
    "squaremap.command.fullrender",
    "squaremap.command.pauserender",
    "squaremap.command.radiusrender",
    "squaremap.command.resetmap",
    "squaremap.command.progresslogging",
    "squaremap.command.hide",
    "squaremap.command.show",
    "squaremap.command.reload",
    "squaremap.command.link",
    "squaremap.command.hide.others",
    "squaremap.command.show.others",
];

/// A Boolean permission node named `name`, as the map's commands ask it.
fn node(name: &str) -> PermissionNode {
    PermissionNode::new(name, PermissionValueType::Boolean)
}

/// squaremap's own data folder: its `config.yml`, `advanced.yml` and `locale/lang-*.yml` are read from there when the
/// server has them (a server moved over from Paper keeps its files).
const DATA: &str = "plugins/squaremap";

/// `config.yml`, then `advanced.yml`, laid over the TOML config (`sm::set_yaml_layers`).
fn read_yaml_layers() -> Vec<String> {
    ["config.yml", "advanced.yml"].iter().filter_map(|f| std::fs::read_to_string(Path::new(DATA).join(f)).ok()).collect()
}

/// Writes a setting back (`progresslogging toggle|rate`): into the TOML config, and into `config.yml` where it has the
/// key (it is laid over the TOML one). Text edits, so comments stay. A world's `serverconfig/` copy is kept in step.
fn persist_setting(path: &[&str], value: &str) {
    let toml_files = [PathBuf::from("config/squaremap-server.toml"), level_dir().join("serverconfig/squaremap-server.toml")];
    let (key, section) = (path[path.len() - 1], path[..path.len() - 1].join("."));
    for file in toml_files {
        if let Ok(text) = std::fs::read_to_string(&file) {
            if let Some(new) = sm::persist::set_toml(&text, &section, key, value) {
                let _ = std::fs::write(&file, new);
            }
        }
    }
    let yaml = Path::new(DATA).join("config.yml");
    if let Ok(text) = std::fs::read_to_string(&yaml) {
        if let Some(new) = sm::persist::set_yaml(&text, path, value) {
            let _ = std::fs::write(&yaml, new);
        }
    }
}

/// The language file `settings.language-file` names, from `locale/` (its file name only); none leaves the English text.
fn load_language() {
    let name = sm::language_file();
    let file = Path::new(&name).file_name().map(|n| n.to_owned());
    let text = file.and_then(|f| std::fs::read_to_string(Path::new(DATA).join("locale").join(f)).ok());
    match text {
        Some(text) => {
            if let Err(e) = sm::commands::set_language(&text) {
                log(&format!("language file {name} not read: {e}"));
            }
        }
        None => sm::commands::clear_language(),
    }
}

/// The TOML config text with squaremap's YAML files laid over it, and the language file.
fn apply_config(text: &str) {
    sm::set_yaml_layers(read_yaml_layers());
    sm::config_loaded(text);
    load_language();
}

fn log(line: &str) {
    eprintln!("[squaremap] {line}");
}

/// One `key=value` line of `server.properties` (the key is never a secret here: `level-name`,
/// `max-players`).
fn property(key: &str) -> Option<String> {
    let text = std::fs::read_to_string("server.properties").ok()?;
    text.lines()
        .find_map(|line| line.strip_prefix(key)?.strip_prefix('=').map(|v| v.trim().to_owned()))
        .filter(|v| !v.is_empty())
}

/// The world folder, as the Paper plugin reads `level-name`.
fn level_dir() -> PathBuf {
    property("level-name").unwrap_or_else(|| "world".into()).into()
}

/// `max-players`, for the player count squaremap shows: the server's live limit, the file when none.
fn max_players(host: &dyn ServerHost) -> usize {
    host.server_max_players()
        .map(|n| n as usize)
        .or_else(|| property("max-players").and_then(|v| v.parse().ok()))
        .unwrap_or(20)
}

/// The game's resource folder: the engine's `--resources <dir>` argument (or `HORIZON_RESOURCES`).
fn resources_dir() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("HORIZON_RESOURCES") {
        return Some(PathBuf::from(p));
    }
    let raw = std::fs::read("/proc/self/cmdline").ok()?;
    let args: Vec<String> = raw.split(|b| *b == 0).map(|a| String::from_utf8_lossy(a).into_owned()).collect();
    args.iter()
        .enumerate()
        .find_map(|(i, a)| match a.strip_prefix("--resources=") {
            Some(v) => Some(v.to_owned()),
            None => (a.as_str() == "--resources").then(|| args.get(i + 1).cloned()).flatten(),
        })
        .map(PathBuf::from)
}

/// The block-state table, biome table and world env the core starts with. arismoko has no registry
/// listing, so the block ids come from the resource folder's `registries.json` and the biomes from its
/// worldgen files (`sm::vanilla`); the host answers each block's states and their map colours.
/// Cause of "not started" before this: the function returned None unconditionally, so `server_started`
/// never reached `sm::start` (no hook, level-name or capability problem).
/// The world env (biome zoom and swamp grass noise) is computed from the overworld's seed (`sm::noise`).
fn render_tables(host: &dyn ServerHost) -> Option<(Vec<sm::RawState>, Vec<sm::RawBiome>, sm::WorldEnv)> {
    let dir = resources_dir()?;
    let mut states = Vec::new();
    // The public host path (`block_key`, `block_states_of` and `block_state_map_color` only exist on the
    // hidden backend, arismoko/horizon#3927): every key by its registered name, then the metadata view.
    let meta = host.block_states();
    let keys: std::collections::HashMap<String, _> = host
        .block_keys()
        .into_iter()
        .filter_map(|key| meta.name(key).ok().map(|n| (n.to_owned(), key)))
        .collect();
    for name in sm::vanilla::block_names(&dir) {
        let Some(&key) = keys.get(&name).or_else(|| keys.get(&format!("minecraft:{name}"))) else { continue };
        let Some(range) = meta.states_of(key) else { continue };
        let default_map_color = host
            .block_state_default(key)
            .and_then(|s| meta.map_color(s))
            .unwrap_or(0);
        for state in range.iter() {
            let Some(text) = host.block_state_string(state) else { continue };
            let (block, properties) = sm::vanilla::parse_state(&text);
            let prop = |k: &str| properties.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
            let age = prop("age").and_then(|v| v.parse::<u8>().ok());
            let fluid = if block == "minecraft:lava" {
                sm::Fluid::Lava
            } else if block == "minecraft:water"
                || prop("waterlogged") == Some("true")
                || matches!(
                    block.as_str(),
                    "minecraft:bubble_column" | "minecraft:kelp" | "minecraft:kelp_plant" | "minecraft:seagrass" | "minecraft:tall_seagrass"
                )
            {
                sm::Fluid::Water
            } else {
                sm::Fluid::None
            };
            states.push(sm::RawState {
                name: block,
                properties,
                age,
                map_color: meta.map_color(state).unwrap_or(0),
                default_map_color,
                map_rgb: None,
                fluid,
            });
        }
    }
    let biomes = sm::vanilla::biomes(&dir);
    if states.is_empty() || biomes.is_empty() {
        return None;
    }
    Some((states, biomes, world_env(host)))
}

/// `BiomeManager`'s zoom and the swamp grass noise for chunks read from the region files. The zoom is seeded by
/// the world seed (one for every level); without a loaded overworld the plain quart cell stands in.
fn world_env(host: &dyn ServerHost) -> sm::WorldEnv {
    let seed = host.level("minecraft:overworld").and_then(|l| l.seed(host));
    let swamp = sm::noise::SwampNoise::new();
    match seed {
        Some(seed) => {
            let biome_seed = sm::noise::obfuscate_seed(seed);
            sm::WorldEnv {
                zoom: Box::new(move |x, y, z| sm::noise::quart_at_block(biome_seed, x, y, z)),
                swamp_grass: Box::new(move |_, x, z| swamp.grass(x, z)),
            }
        }
        None => {
            let mut env = sm::WorldEnv::plain();
            env.swamp_grass = Box::new(move |_, x, z| swamp.grass(x, z));
            env
        }
    }
}

/// The border of a dimension's level and the world spawn column (`LevelData.getRespawnData`, one spawn
/// for every level), when the level is loaded.
fn level_border(host: &mut dyn ServerHost, dimension: &str) -> Option<([f64; 4], Option<[i32; 2]>)> {
    let level = host.level(dimension)?;
    let border = level.world_border(&*host)?;
    let spawn = level.respawn_data(&*host).map(|r| [r.pos[0], r.pos[2]]);
    Some((border, spawn))
}

/// The ids of the open levels from the host (`Levels::list_levels`), with `add` in it (a level that just opened) and
/// without `remove` (one that just closed).
fn open_level_ids(host: &dyn ServerHost, add: Option<&str>, remove: Option<&str>) -> Vec<String> {
    let mut ids: Vec<String> = host.levels().list_levels().into_iter().map(|l| l.dimension_id().to_owned()).collect();
    if let Some(a) = add {
        if !ids.iter().any(|i| i == a) {
            ids.push(a.to_owned());
        }
    }
    if let Some(r) = remove {
        ids.retain(|i| i != r);
    }
    ids
}

/// The map's level registry follows the open levels: their ids, their height ranges (`LevelRef.height`), and the
/// renders a stop interrupted resume. Called at start and whenever a level loads or unloads.
fn sync_levels(host: &mut dyn ServerHost, add: Option<&str>, remove: Option<&str>) {
    let ids = open_level_ids(&*host, add, remove);
    sm::set_open_levels(&ids);
    for id in &ids {
        let (Some(w), Some(level)) = (sm::world_index(id), host.level(id)) else { continue };
        // The dimension type (`LevelRef.dimension_type`) gives the ceiling flag and the build height of a custom
        // level; the plain height range and the nether's id stand in when the host has none.
        if let Some(ty) = level.dimension_type(&*host) {
            sm::level_shape(w, ty.min_y, ty.min_y + ty.height, ty.has_ceiling);
        } else if let Some((min_y, height)) = level.height(&*host) {
            sm::level_shape(w, min_y, min_y + height, id.as_str() == "minecraft:the_nether");
        }
    }
    sm::resume_renders();
}

/// The item tag `minecraft:map_invisibility_equipment` (`ItemTags.MAP_INVISIBILITY_EQUIPMENT`).
const MAP_INVISIBILITY_TAG: &str = "minecraft:map_invisibility_equipment";

/// The tag's one vanilla entry: the answer when the host gives none.
const MAP_INVISIBILITY_FALLBACK: &str = "minecraft:carved_pumpkin";

/// Whether the item `id` is in `minecraft:map_invisibility_equipment`, read from the host's item tags
/// (`registry_entry_in_tag`, as veinminer's item tag check); the vanilla entry answers when the host has none.
fn is_map_invisibility_item(host: &dyn ServerHost, id: &str) -> bool {
    let fallback = id == MAP_INVISIBILITY_FALLBACK;
    let (Ok(registry), Ok(entry)) = (Identifier::parse("minecraft:item"), Identifier::parse(id)) else {
        return fallback;
    };
    host.registry_entry_in_tag(&RegistryEntry::new(registry, entry), MAP_INVISIBILITY_TAG).unwrap_or(fallback)
}

/// `MapItemSavedData.hasMapInvisibilityItemEquipped`: an item of the tag in an armour or body slot.
fn has_map_invisibility_equipment(host: &dyn ServerHost, living: LivingRef) -> bool {
    [EquipmentSlot::Feet, EquipmentSlot::Legs, EquipmentSlot::Chest, EquipmentSlot::Head, EquipmentSlot::Body, EquipmentSlot::Saddle]
        .into_iter()
        .any(|slot| living.equipment(host, slot).is_some_and(|stack| is_map_invisibility_item(host, &stack.item().id(host))))
}

/// The players squaremap draws, less the ones a level's `player-tracker.hide` rules leave out:
/// spectators, invisible players (the invisibility effect) and map invisibility equipment.
fn publish_players(host: &dyn ServerHost) {
    let mut list = Vec::new();
    for p in host.players().server_players() {
        let (Some(name), Some(world), Some(position)) =
            (p.name(host), p.dimension(host), p.entity().position(host))
        else {
            continue;
        };
        let world = world.to_string();
        let rules = sm::hide_rules(&world);
        if rules.spectators && p.is_spectator(host) == Some(true) {
            continue;
        }
        let entity = p.entity();
        let living = LivingRef::new(entity);
        if rules.invisible && entity.is_invisible(host) == Some(true) {
            continue;
        }
        if rules.map_invisibility_equipment && has_map_invisibility_equipment(host, living) {
            continue;
        }
        list.push(sm::Player {
            uuid: p.uuid(),
            name,
            world,
            position,
            yaw: entity.rotation(host).map_or(0.0, |r| r[0]),
            health: living.health(host).unwrap_or(0.0),
            // `Attributes.ARMOR` with every modifier, as the game answers it.
            armor: living.attribute_value(host, "minecraft:armor").map(|v| v.floor() as i32),
            // `use-display-names`: the nickname `player_set_nickname` shows in place of the name, if any.
            display_name: p.nickname(host).flatten(),
        });
    }
    sm::publish_players(&list, max_players(host));
}

/// The players the command line may name (`/squaremap hide <player>`).
fn online(host: &dyn ServerHost) -> Vec<(u128, String)> {
    host.players()
        .server_players()
        .into_iter()
        .filter_map(|p| Some((p.uuid(), p.name(host)?)))
        .collect()
}

/// squaremap's `[squaremap]` prefix (`Messages.COMMAND_PREFIX`): a gradient on the word, and a
/// hover and a click (`/<label> help`) on each piece.
fn prefix() -> Vec<Text> {
    const FROM: (u32, u32, u32) = (0xC0, 0x28, 0xFF);
    const TO: (u32, u32, u32) = (0x5B, 0x00, 0xFF);
    let click = TextClick::RunCommand(format!("/{} help", sm::commands::main_label()));
    let hover = Some("Click for help");
    let mut out = vec![styled("[", Some(0xFFFFFF), Some(click.clone()), hover)];
    let name = "squaremap";
    let last = (name.chars().count() - 1) as f64;
    for (i, c) in name.chars().enumerate() {
        let t = i as f64 / last;
        let mix = |a: u32, b: u32| (a as f64 + (b as f64 - a as f64) * t).round() as u32;
        let color = (mix(FROM.0, TO.0) << 16) | (mix(FROM.1, TO.1) << 8) | mix(FROM.2, TO.2);
        out.push(styled(c.to_string().as_str(), Some(color), Some(click.clone()), hover));
    }
    out.push(styled("]", Some(0xFFFFFF), Some(click.clone()), hover));
    out.push(styled(" ", Some(0xFFFFFF), None, None));
    out
}

/// One piece of text with squaremap's colour, click and hover.
fn styled(text: &str, color: Option<u32>, click: Option<TextClick>, hover: Option<&str>) -> Text {
    let mut style = TextStyle::default();
    // `TextStyle.color` takes `#RRGGBB`.
    style.color = color.map(|c| format!("#{c:06X}"));
    style.click = click;
    style.hover = hover.map(|h| TextHover::Text(Box::new(Text::literal(h))));
    Text::literal(text).with_style(style)
}

/// One of squaremap's lines as Horizon text: the prefix on errors, then its parts.
fn line_text(line: &sm::commands::Line) -> Text {
    let mut text = Text::literal("");
    if line.prefixed {
        for piece in prefix() {
            text = text.append(piece);
        }
    }
    for part in &line.expanded() {
        let click = part.click.as_ref().map(|c| match c {
            sm::commands::Click::Run(command) => TextClick::RunCommand(command.clone()),
            sm::commands::Click::Open(url) => TextClick::OpenUrl(url.clone()),
        });
        text = text.append(styled(&part.text, part.color, click, part.hover.as_deref()));
    }
    text
}

/// The chunks decoded for this level tick: a due chunk's snapshot shares its neighbours with the next one's.
#[derive(Default)]
struct ChunkCache {
    chunks: std::collections::HashMap<(i32, i32), Option<Arc<sm::region::DiskChunk>>>,
}

/// A loaded chunk as the scanner reads it: its sections and `WORLD_SURFACE` heights, decoded once per tick.
fn decode_loaded(
    host: &dyn ServerHost,
    level: LevelRef,
    cx: i32,
    cz: i32,
    ids: &sm::region::Ids,
    shape: sm::region::Shape,
    cache: &mut ChunkCache,
) -> Option<Arc<sm::region::DiskChunk>> {
    if let Some(hit) = cache.chunks.get(&(cx, cz)) {
        return hit.clone();
    }
    let chunk = ChunkRef::new(level, [cx, cz]);
    let decoded = chunk.sections(host).and_then(|sections| {
        let sections = sections
            .into_iter()
            .map(|s| sm::region::LiveSection {
                y: s.y,
                block_palette: s.block_palette,
                blocks: s.blocks,
                biome_palette: s.biome_palette,
                biomes: s.biomes,
            })
            .collect();
        let heights = chunk.heightmap(host, Heightmap::WorldSurface);
        sm::region::DiskChunk::from_live(sections, heights.as_deref(), ids, shape).ok().map(Arc::new)
    });
    if cache.chunks.len() >= 64 {
        cache.chunks.clear();
    }
    cache.chunks.insert((cx, cz), decoded.clone());
    decoded
}

/// One due chunk of level `w`: see `level_tick_pre`.
fn draw_due_chunk(host: &mut dyn ServerHost, level: LevelRef, w: usize, cx: i32, cz: i32, cache: &mut ChunkCache) {
    if sm::is_loaded(w, cx, cz) {
        if let Some((ids, env)) = sm::snapshot_tables() {
            let shape = sm::level_shape_of(w);
            let centre = decode_loaded(&*host, level, cx, cz, &ids, shape, cache);
            if centre.is_some() {
                let chunks = std::array::from_fn(|i| {
                    let (dx, dz) = (i as i32 % 3 - 1, i as i32 / 3 - 1);
                    if (dx, dz) == (0, 0) { centre.clone() } else { decode_loaded(&*host, level, cx + dx, cz + dz, &ids, shape, cache) }
                });
                sm::drawn(w, cx, cz);
                sm::submit(w, Box::new(sm::region::DiskView::new(cx, cz, chunks, shape, env)));
                return;
            }
        }
        // No snapshot: the chunk is drawn from the region files after the next world save.
        sm::due_chunk(w, cx, cz);
        return;
    }
    // Unloaded: its save may still wait for the region writer; draw it once the save is as new as the change.
    let changed = sm::changed_second(w, cx, cz);
    match level.saved_chunk_modified(&*host, [cx, cz]) {
        Some(saved) if saved < changed => sm::redirty(w, cx, cz),
        _ => {
            sm::drawn(w, cx, cz);
            sm::due_chunk(w, cx, cz);
        }
    }
}

#[derive(Default)]
struct Mod;

#[mod_api::horizon_mod]
impl Hooks for Mod {
    fn register_configs(&self, _: HostSide, event: &mut RegisterConfigsEvent<'_>) {
        // `config/squaremap-server.toml`, written with the default when missing.
        event.configs.push(Config::new(ID, ConfigKind::Server, sm::default_config()));
    }

    fn config_loading(&self, _: HostSide, event: &mut ConfigLoadingEvent<'_>) {
        if event.config.mod_id == ID {
            apply_config(&event.config.text);
        }
    }

    fn config_reloading(&self, _: HostSide, event: &mut ConfigReloadingEvent<'_>) {
        if event.config.mod_id == ID {
            apply_config(&event.config.text);
        }
    }

    fn server_started(&self, host: &mut dyn ServerHost, _: &mut ServerStartedEvent) {
        sm::set_persist(persist_setting);
        sync_levels(host, None, None);
        match render_tables(&*host) {
            Some((states, biomes, env)) => match sm::start(&level_dir(), states, biomes, env) {
                Ok(()) => log("map started"),
                Err(e) => log(&format!("not started: {e}")),
            },
            None => log(
                "not started: block-state or biome tables could not be built (resource folder with \
                 registries.json and data/minecraft/worldgen/biome not found, or the host lists no states)",
            ),
        }
    }

    fn server_stopping(&self, _: &mut dyn ServerHost, _: &mut ServerStoppingEvent) {
        sm::world_closed();
    }

    /// The service other mods call: `squaremap:chunky_event`, one event line of the chunky mod (a border that changed
    /// or went away is drawn or removed as a layer).
    fn mod_service_call(&self, _: &mut dyn ServerHost, event: &mut ModServiceCallEvent<'_>) -> Option<mod_api::PayloadBytes> {
        if event.service != "squaremap:chunky_event" {
            return None;
        }
        let line = std::str::from_utf8(&event.payload.0).ok()?;
        if let Some((world, border)) = chunky_border::parse_event(line)
            && let Some(dimension) = chunky_border::dimension_of(&world)
        {
            chunky_border::apply(&dimension, border.as_ref(), &border_options());
        }
        Some(mod_api::PayloadBytes(b"ok".to_vec()))
    }

    fn server_tick(&self, host: &mut dyn ServerHost, event: &mut ServerTickEvent<'_>) {
        if let ServerTickPhase::ServerTickEnd { world_running: true, .. } = event.phase
            && !CHUNKY_SUBSCRIBED.load(std::sync::atomic::Ordering::Relaxed)
            && sm::running()
        {
            static CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            if CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % SUBSCRIBE_EVERY == 0 {
                subscribe_to_chunky(host);
            }
        }
        if !matches!(event.phase, ServerTickPhase::ServerTickEnd { world_running: true, .. }) || !sm::running() {
            return;
        }
        for (w, dimension) in sm::open_levels() {
            // A level whose map is disabled is not drawn and publishes nothing.
            if !sm::world_enabled(w) {
                continue;
            }
            if sm::markers_due(w) {
                if let Some((border, spawn)) = level_border(host, &dimension) {
                    sm::publish_markers(w, spawn, border);
                }
            }
        }
        if sm::players_due() {
            publish_players(&*host);
        }
    }

    /// The changed chunks of a level are drawn as it ticks, with the level in place: a loaded one from a snapshot of
    /// its sections and heights (`chunk_sections`, `chunk.heightmap`, and its eight neighbours), at once; an unloaded
    /// one from the region files, once its save is newer than its change (`level.saved_chunk_modified`).
    fn level_tick_pre(&self, host: HostSide, event: &mut LevelTickPreEvent) {
        let HostSide::Server(host) = host else { return };
        if !sm::running() {
            return;
        }
        let level = event.level;
        let dimension = level.dimension_id().to_string();
        let Some(w) = sm::open_world_index(&dimension) else { return };
        if !sm::world_enabled(w) {
            return;
        }
        let mut cache = ChunkCache::default();
        for (cx, cz) in sm::due_chunks(w) {
            draw_due_chunk(host, level, w, cx, cz, &mut cache);
        }
    }

    /// A world save wrote the loaded chunks to the region files: the changed ones that waited for it are drawn.
    fn world_saved(&self, _: &mut dyn ServerHost, _: &mut WorldSavedEvent) {
        sm::world_saved();
    }

    /// A level opened (before its first chunk): it joins the registry, and its map is set up if it is enabled.
    fn level_load(&self, host: &mut dyn ServerHost, event: &mut LevelLoadEvent<'_>) {
        sync_levels(host, Some(event.dimension), None);
    }

    /// A level closed after its last save: it leaves the registry (its index and queued work stay).
    fn level_unload(&self, host: &mut dyn ServerHost, event: &mut LevelUnloadEvent<'_>) {
        sync_levels(host, None, Some(event.dimension));
    }

    fn chunk_load(&self, _: &mut dyn ServerHost, e: &mut ChunkLoadEvent<'_>) {
        if let Some(w) = sm::world_index(e.dimension) {
            sm::chunk_loaded(w, e.chunk[0], e.chunk[1], e.new);
        }
    }

    fn chunk_unload(&self, _: &mut dyn ServerHost, e: &mut ChunkUnloadEvent<'_>) {
        if let Some(w) = sm::world_index(e.dimension) {
            sm::chunk_unloaded(w, e.chunk[0], e.chunk[1]);
        }
    }

    fn block_changed(&self, _: &dyn mod_api::LevelHost, e: &mut BlockChangedEvent) {
        if let Some(w) = sm::world_index(e.dimension.key().id()) {
            sm::block_changed(w, e.pos.x, e.pos.z);
        }
    }

    fn permission_gather_nodes(&self, event: &mut PermissionGatherNodesEvent<'_>) {
        let level = NodeDefault::Level(PermissionLevel::from_level(i32::from(sm::commands::OP_LEVEL)));
        for name in NODES {
            let _ = event.nodes.add(node(name), level.clone());
        }
    }

    fn register_commands(&self, _: &mut dyn ServerHost, event: &mut RegisterCommandsEvent<'_>) {
        for (usage, level) in sm::commands::usages() {
            if sm::commands::permission_node(&usage).is_none() {
                event.commands.push(Command::op(&usage.replace("word?", "word"), level));
            }
        }
    }

    fn register_node_commands(&self, _: &mut dyn ServerHost, event: &mut RegisterNodeCommandsEvent<'_>) {
        // squaremap's `word?` (the client asks for world names) is plain `word` here; the
        // names come from `tab_complete` below.
        // Each subcommand path carries squaremap's node (`squaremap.command.fullrender`); the level
        // decides when no permissions mod answers. The bare label and `confirm` have none (register_commands).
        for (usage, level) in sm::commands::usages() {
            if let Some(name) = sm::commands::permission_node(&usage) {
                event.commands.push(Command::op(&usage.replace("word?", "word"), level).with_node(&node(&name)));
            }
        }
    }

    fn player_command(&self, host: &mut dyn ServerHost, event: &mut PlayerCommandEvent<'_>) -> Flow {
        // A player without the sub-command's node (`squaremap.command.<sub>`, operators by default) gets
        // squaremap's no-permission line instead of the engine's "Unknown command".
        if !sm::commands::is_ours(event.command) {
            return Flow::Continue;
        }
        let Some(name) = sm::commands::permission_node(event.command) else { return Flow::Continue };
        let node = node(&name);
        let allowed = host.player(event.player).is_some_and(|p| p.permission_bool(&mut *host, &node).unwrap_or(false));
        if allowed {
            return Flow::Continue;
        }
        event.replies.push(line_text(&sm::commands::no_permission()));
        event.cancelled = true;
        Flow::Cancel
    }

    fn run_mod_command(&self, host: &mut dyn ServerHost, event: &mut RunModCommandEvent<'_>) -> Flow {
        // The command line as squaremap parses it: the literals, then the arguments in order.
        let mut words = event.command.literals.clone();
        for argument in &event.command.arguments {
            match &argument.value {
                CommandArgumentValue::String(s) => words.push(s.clone()),
                CommandArgumentValue::Integer(i) => words.push(i.to_string()),
                CommandArgumentValue::Player(p) => words.extend(p.name(&*host)),
                _ => {}
            }
        }
        let line = words.join(" ");
        if !sm::commands::is_ours(&line) {
            return Flow::Continue;
        }
        let sender = match &event.source {
            CommandSource::Player { player, permission } => {
                let (Some(name), Some(world), Some(at)) =
                    (player.name(&*host), player.dimension(&*host), player.entity().position(&*host))
                else {
                    return Flow::Continue;
                };
                // The usage's node already gated the command (`with_node`); the core is told 4 when the node
                // (or, without one, the level) lets the player through. `hide`/`show` for others asks its own node.
                let allowed = match sm::commands::permission_node(&line) {
                    Some(name) => player.permission_bool(&mut *host, &node(&name)).unwrap_or(false),
                    None => permission.level() >= sm::commands::OP_LEVEL,
                };
                let op = if allowed { 4 } else { 0 };
                let others = sm::commands::others_node(&line)
                    .is_some_and(|n| player.permission_bool(&mut *host, &node(&n)).unwrap_or(false));
                sm::commands::Sender {
                    player: Some(sm::commands::SenderPlayer {
                        uuid: player.uuid(),
                        name,
                        world: world.to_string(),
                        x: at[0],
                        z: at[2],
                        yaw: player.entity().rotation(&*host).map_or(0.0, |r| f64::from(r[0])),
                    }),
                    permission: op,
                    others,
                }
            }
            CommandSource::Console { .. } => {
                sm::commands::Sender { player: None, permission: 4, others: true }
            }
            _ => return Flow::Continue,
        };
        let lines = sm::commands::command(&line, &sender, &online(&*host));
        if line.split_whitespace().nth(1).is_some_and(|w| w.eq_ignore_ascii_case("reload")) {
            // `reload` also reads the language file again.
            load_language();
        }
        for l in &lines {
            let _ = host.commands().feedback(line_text(l));
        }
        Flow::Cancel
    }

    fn tab_complete(&self, _: &mut dyn ServerHost, e: &mut TabCompleteEvent<'_>) {
        for text in sm::commands::world_completions(e.command) {
            if !e.suggestions.iter().any(|s| s.text == text) {
                e.suggestions.push(Suggestion::new((e.range[0], e.range[1]), text));
            }
        }
    }
}
