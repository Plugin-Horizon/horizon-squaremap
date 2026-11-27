//! The mod's config: squaremap's `config.yml` and `advanced.yml` as one TOML file, with the
//! same key names. `world-settings.default` applies to every world; a table named after a
//! world (`[world-settings."minecraft:the_nether".map]`) overrides keys of it.
use crate::horizon_squaremap::visibility::{self, Shape};
use serde::Deserialize;

/// Written to `config/squaremap-server.toml` when it is missing.
pub const DEFAULT_TOML: &str = include_str!("../../data/squaremap/squaremap-server.toml");

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Config {
    pub settings: Settings,
    /// Raw, merged per world by [`Config::world`].
    pub world_settings: toml::Table,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Settings {
    pub debug_mode: bool,
    pub update_checker: bool,
    /// The language file under `plugins/squaremap/locale/` (`settings.language-file`).
    pub language_file: String,
    pub web_address: String,
    pub web_directory: WebDirectory,
    pub internal_webserver: Webserver,
    pub ui: Ui,
    pub image_quality: ImageQuality,
    pub render_progress_logging: ProgressLogging,
    pub commands: Commands,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct ImageQuality {
    pub compress_images: Compress,
}

/// `value` 0.0 to 1.0: how hard tiles are compressed (squaremap hands `1 - value` to
/// ImageIO as the quality, which the PNG writer turns into deflate level `9 * value`).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Compress {
    pub enabled: bool,
    pub value: f64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct ProgressLogging {
    pub enabled: bool,
    pub interval_seconds: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Commands {
    pub main_command_label: String,
    pub main_command_aliases: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct WebDirectory {
    /// Relative to the world folder, or absolute.
    pub path: String,
    pub auto_update: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Webserver {
    pub enabled: bool,
    pub bind: String,
    pub port: u16,
    pub flush_json_immediately: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Ui {
    pub coordinates: Toggle,
    pub link: Toggle,
    pub sidebar: Sidebar,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Toggle {
    pub enabled: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Sidebar {
    pub pinned: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct WorldConfig {
    pub map: MapConfig,
    pub player_tracker: PlayerTracker,
    pub invisible_blocks: Vec<String>,
    pub iterate_up_base_blocks: Vec<String>,
    pub color_overrides: ColorOverrides,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct MapConfig {
    pub enabled: bool,
    pub display_name: String,
    pub order: i32,
    pub icon: String,
    pub iterate_up: bool,
    pub max_height: i32,
    pub biomes: Biomes,
    pub glass: Glass,
    pub lava: Checker,
    pub water: Water,
    pub zoom: Zoom,
    pub markers: Markers,
    pub visibility_limits: Vec<VisibilityLimit>,
    pub render: Render,
    pub image_saving: ImageSaving,
    /// Before squaremap 1.4: `map.max-render-threads` (-1 = default), the workers of full and radius renders.
    pub max_render_threads: Option<Auto>,
    /// The limits as shapes, with the world border of when they were resolved
    /// ([`MapConfig::resolve`]).
    #[serde(skip)]
    pub shapes: Vec<Shape>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Biomes {
    pub enabled: bool,
    pub blend_biomes: i32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Glass {
    pub clear: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Checker {
    pub checkerboard: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Water {
    pub clear_depth: bool,
    pub checkerboard: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Zoom {
    pub maximum: i32,
    pub default: i32,
    pub extra: i32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Markers {
    pub update_interval_seconds: u64,
    pub spawn_icon: Layer,
    pub world_border: Layer,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Layer {
    pub enabled: bool,
    pub show_controls: bool,
    pub default_hidden: bool,
    pub layer_priority: i32,
    pub z_index: i32,
}

/// One of `rectangle` (`min-x`..`max-z`, inclusive), `circle` (`center-x`, `center-z`,
/// `radius`), `polygon` (`points`, `"x, z"` strings) or `world-border`. Like squaremap, a
/// shape without `enabled` is on.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct VisibilityLimit {
    #[serde(rename = "type")]
    pub kind: String,
    pub enabled: bool,
    pub min_x: i32,
    pub min_z: i32,
    pub max_x: i32,
    pub max_z: i32,
    pub center_x: i32,
    pub center_z: i32,
    pub radius: i32,
    pub points: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Render {
    pub full: RenderLimits,
    pub radius: RenderLimits,
    pub background: Background,
}

/// An integer setting that may also read `"default"` (`RenderConfig.automatic`): the raw value as the file has it.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Auto {
    Number(i64),
    Text(String),
}

impl Default for Auto {
    fn default() -> Self {
        Auto::Text("default".into())
    }
}

impl Auto {
    /// `RenderConfig.parse`: `default` (when `automatic`) and anything unusable give `fallback`, with a warning for the
    /// unusable ones; a value under `minimum` too.
    pub fn resolve(&self, path: &str, fallback: usize, minimum: usize, automatic: bool) -> usize {
        let parsed = match self {
            Auto::Number(n) => Some(*n),
            Auto::Text(t) if automatic && t.eq_ignore_ascii_case("default") => return fallback,
            Auto::Text(t) => t.trim().parse::<i64>().ok(),
        };
        let shown = match self {
            Auto::Number(n) => n.to_string(),
            Auto::Text(t) => t.clone(),
        };
        match parsed {
            None => {
                let or_default = if automatic { " or 'default'" } else { "" };
                eprintln!("[squaremap] {path} must be an integer{or_default}; using {fallback} instead of '{shown}'");
                fallback
            }
            Some(n) if n < minimum as i64 => {
                eprintln!("[squaremap] {path} must be at least {minimum}; using {fallback} instead of {n}");
                fallback
            }
            Some(n) => n as usize,
        }
    }
}

/// `RenderConfig.automaticWorkers`: half the processors, then at most half of that above four.
pub fn automatic_workers() -> usize {
    let half = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2) / 2;
    if half <= 4 {
        if half <= 3 { 1 } else { 2 }
    } else {
        half / 2
    }
}

/// Execution limits of one render mode (`RenderConfig`), as the file has them.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct RenderLimits {
    pub workers: Auto,
    pub max_chunk_requests: Auto,
    pub max_active_chunks: Auto,
    pub max_active_regions: Option<i64>,
    pub max_column_chunks: Option<i64>,
    pub idle_chunk_cache_size: Auto,
}

/// The limits of a mode once resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub workers: usize,
    pub max_chunk_requests: usize,
    pub max_active_chunks: usize,
    pub max_active_regions: usize,
    pub max_column_chunks: usize,
    pub idle_chunk_cache_size: usize,
}

impl RenderLimits {
    /// `new RenderConfig(config, path, defaultWorkers)`. `legacy_workers` is the pre-1.4 `map.max-render-threads`
    /// (`-1` meant default), used where `workers` is not set.
    pub fn resolve(&self, path: &str, default_workers: usize, legacy_workers: Option<&Auto>) -> Limits {
        let workers_raw = match (&self.workers, legacy_workers) {
            (Auto::Text(t), Some(old)) if t.eq_ignore_ascii_case("default") => match old {
                Auto::Number(-1) => Auto::default(),
                other => other.clone(),
            },
            (w, _) => w.clone(),
        };
        let workers = workers_raw.resolve(&format!("{path}.workers"), default_workers, 1, true);
        let fixed = |v: Option<i64>, key: &str, fallback: usize, min: usize| {
            v.map(Auto::Number).unwrap_or(Auto::Number(fallback as i64)).resolve(&format!("{path}.{key}"), fallback, min, false)
        };
        let max_active_chunks =
            self.max_active_chunks.resolve(&format!("{path}.max-active-chunks"), workers * 64, 1, true);
        Limits {
            workers,
            max_chunk_requests: self.max_chunk_requests.resolve(&format!("{path}.max-chunk-requests"), workers * 96, 1, true),
            max_active_chunks,
            max_active_regions: fixed(self.max_active_regions, "max-active-regions", 2, 1),
            max_column_chunks: fixed(self.max_column_chunks, "max-column-chunks", 16, 1),
            idle_chunk_cache_size: self.idle_chunk_cache_size.resolve(
                &format!("{path}.idle-chunk-cache-size"),
                max_active_chunks,
                0,
                true,
            ),
        }
    }
}

/// `map.image-saving` (`ImageSavingConfig`).
#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct ImageSaving {
    pub workers: Option<i64>,
    pub max_pending_images: Option<i64>,
}

impl Default for ImageSaving {
    fn default() -> Self {
        Self { workers: None, max_pending_images: None }
    }
}

impl ImageSaving {
    /// (workers, max pending images), each at least 1, defaults 2 and 100.
    pub fn resolve(&self) -> (usize, usize) {
        let get = |v: Option<i64>, key: &str, fallback: usize| {
            v.map(Auto::Number)
                .unwrap_or(Auto::Number(fallback as i64))
                .resolve(&format!("map.image-saving.{key}"), fallback, 1, false)
        };
        (get(self.workers, "workers", 2), get(self.max_pending_images, "max-pending-images", 100))
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Background {
    pub enabled: bool,
    pub interval_seconds: u64,
    pub max_chunks_per_interval: usize,
    pub workers: Auto,
}

impl Background {
    /// The background mode's workers: `max(1, automatic * 2 / 3)` unless set.
    pub fn resolved_workers(&self, legacy: Option<&Auto>) -> usize {
        let default = std::cmp::max(1, automatic_workers() * 2 / 3);
        let raw = match (&self.workers, legacy) {
            (Auto::Text(t), Some(old)) if t.eq_ignore_ascii_case("default") => match old {
                Auto::Number(-1) => Auto::default(),
                other => other.clone(),
            },
            (w, _) => w.clone(),
        };
        raw.resolve("map.render.background.workers", default, 1, true)
    }
}

impl MapConfig {
    /// The limits of a full render.
    pub fn full_limits(&self) -> Limits {
        self.render.full.resolve("map.render.full", automatic_workers(), self.max_render_threads.as_ref())
    }

    /// The limits of a radius render.
    pub fn radius_limits(&self) -> Limits {
        self.render.radius.resolve("map.render.radius", automatic_workers(), self.max_render_threads.as_ref())
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct PlayerTracker {
    pub enabled: bool,
    pub update_interval_seconds: u64,
    pub show_controls: bool,
    pub default_hidden: bool,
    pub layer_priority: i32,
    pub z_index: i32,
    pub use_display_names: bool,
    pub nameplate: Nameplate,
    pub hide: Hide,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Nameplate {
    pub enabled: bool,
    pub show_head: bool,
    pub heads_url: String,
    pub show_armor: bool,
    pub show_health: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Hide {
    pub invisible: bool,
    pub spectators: bool,
    pub map_invisibility_equipment: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct ColorOverrides {
    pub blocks: std::collections::BTreeMap<String, String>,
    pub biomes: BiomeOverrides,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct BiomeOverrides {
    pub grass: std::collections::BTreeMap<String, String>,
    pub foliage: std::collections::BTreeMap<String, String>,
    pub dry_foliage: std::collections::BTreeMap<String, String>,
    pub water: std::collections::BTreeMap<String, String>,
}

impl Default for Config {
    /// Built-in values only; the shipped file is [`DEFAULT_TOML`].
    fn default() -> Self {
        Self { settings: Settings::default(), world_settings: toml::Table::new() }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            debug_mode: false,
            update_checker: true,
            language_file: "lang-en.yml".into(),
            web_address: "http://localhost:8080".into(),
            web_directory: WebDirectory { path: "web".into(), auto_update: true },
            internal_webserver: Webserver::default(),
            ui: Ui::default(),
            image_quality: ImageQuality::default(),
            render_progress_logging: ProgressLogging::default(),
            commands: Commands::default(),
        }
    }
}

impl Default for ProgressLogging {
    fn default() -> Self {
        Self { enabled: true, interval_seconds: 1 }
    }
}

impl Default for Commands {
    fn default() -> Self {
        Self { main_command_label: "squaremap".into(), main_command_aliases: vec!["map".into()] }
    }
}

impl Default for WebDirectory {
    fn default() -> Self {
        Settings::default().web_directory
    }
}

impl Default for Webserver {
    fn default() -> Self {
        Self { enabled: true, bind: "127.0.0.1".into(), port: 8080, flush_json_immediately: false }
    }
}

impl Default for Ui {
    fn default() -> Self {
        Self {
            coordinates: Toggle { enabled: true },
            link: Toggle { enabled: true },
            sidebar: Sidebar::default(),
        }
    }
}

impl Default for Toggle {
    fn default() -> Self {
        Self { enabled: true }
    }
}

impl Default for Sidebar {
    fn default() -> Self {
        Self { pinned: "unpinned".into() }
    }
}

impl Default for WorldConfig {
    fn default() -> Self {
        Self {
            map: MapConfig::default(),
            player_tracker: PlayerTracker::default(),
            invisible_blocks: [
                "minecraft:tall_grass",
                "minecraft:fern",
                "minecraft:short_grass",
                "minecraft:large_fern",
                "minecraft:bush",
                "minecraft:short_dry_grass",
                "minecraft:tall_dry_grass",
            ]
            .map(String::from)
            .to_vec(),
            iterate_up_base_blocks: [
                "minecraft:netherrack",
                "minecraft:glowstone",
                "minecraft:soul_sand",
                "minecraft:soul_soil",
                "minecraft:gravel",
                "minecraft:warped_nylium",
                "minecraft:crimson_nylium",
                "minecraft:nether_gold_ore",
                "minecraft:ancient_debris",
                "minecraft:nether_quartz_ore",
                "minecraft:magma_block",
                "minecraft:basalt",
            ]
            .map(String::from)
            .to_vec(),
            color_overrides: ColorOverrides::default(),
        }
    }
}

impl Default for MapConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            display_name: "{world}".into(),
            order: 0,
            icon: String::new(),
            iterate_up: false,
            max_height: -1,
            biomes: Biomes::default(),
            glass: Glass { clear: true },
            lava: Checker { checkerboard: true },
            water: Water::default(),
            zoom: Zoom::default(),
            markers: Markers::default(),
            visibility_limits: Vec::new(),
            render: Render::default(),
            image_saving: ImageSaving::default(),
            max_render_threads: None,
            shapes: Vec::new(),
        }
    }
}

impl Default for Biomes {
    fn default() -> Self {
        Self { enabled: true, blend_biomes: 3 }
    }
}

impl Default for Glass {
    fn default() -> Self {
        Self { clear: true }
    }
}

impl Default for Checker {
    fn default() -> Self {
        Self { checkerboard: true }
    }
}

impl Default for Water {
    fn default() -> Self {
        Self { clear_depth: true, checkerboard: false }
    }
}

impl Default for Zoom {
    fn default() -> Self {
        Self { maximum: 3, default: 3, extra: 2 }
    }
}

impl Default for Markers {
    fn default() -> Self {
        Self {
            update_interval_seconds: 5,
            spawn_icon: Layer { layer_priority: 0, z_index: 0, ..Layer::default() },
            world_border: Layer { layer_priority: 1, z_index: 1, ..Layer::default() },
        }
    }
}

impl Default for Layer {
    fn default() -> Self {
        Self { enabled: true, show_controls: true, default_hidden: false, layer_priority: 0, z_index: 0 }
    }
}

impl Default for VisibilityLimit {
    fn default() -> Self {
        Self {
            kind: String::new(),
            enabled: true,
            min_x: 0,
            min_z: 0,
            max_x: 0,
            max_z: 0,
            center_x: 0,
            center_z: 0,
            radius: 0,
            points: Vec::new(),
        }
    }
}

impl Default for Background {
    fn default() -> Self {
        Self { enabled: true, interval_seconds: 15, max_chunks_per_interval: 2048, workers: Auto::default() }
    }
}

impl Default for PlayerTracker {
    fn default() -> Self {
        Self {
            enabled: true,
            update_interval_seconds: 1,
            show_controls: true,
            default_hidden: false,
            layer_priority: 2,
            z_index: 2,
            use_display_names: false,
            nameplate: Nameplate::default(),
            hide: Hide::default(),
        }
    }
}

impl Default for Nameplate {
    fn default() -> Self {
        Self {
            enabled: true,
            show_head: true,
            heads_url: "https://mc-heads.net/avatar/{uuid}/16".into(),
            show_armor: true,
            show_health: true,
        }
    }
}

impl Default for Hide {
    fn default() -> Self {
        Self { invisible: true, spectators: true, map_invisibility_equipment: true }
    }
}

impl Config {
    pub fn parse(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    /// The TOML text with squaremap's own `config.yml` and `advanced.yml` (when the server has them) laid over it, in
    /// that order: a key the YAML files set wins.
    pub fn parse_layers(text: &str, yaml: &[String]) -> Result<Self, String> {
        let mut table: toml::Table = toml::from_str(text).map_err(|e| e.to_string())?;
        for layer in yaml {
            crate::horizon_squaremap::yaml::overlay(&mut table, &crate::horizon_squaremap::yaml::parse(layer)?);
        }
        toml::Value::Table(table).try_into().map_err(|e: toml::de::Error| e.to_string())
    }

    /// `world-settings.default` with the world's own table merged over it.
    pub fn world(&self, world: &str) -> WorldConfig {
        let mut merged = toml::Value::Table(toml::Table::new());
        for key in ["default", world] {
            if let Some(v) = self.world_settings.get(key) {
                merge(&mut merged, v);
            }
        }
        let mut w: WorldConfig = merged.try_into().unwrap_or_default();
        w.map.resolve(None);
        w
    }
}

fn merge(into: &mut toml::Value, from: &toml::Value) {
    match (into, from) {
        (toml::Value::Table(a), toml::Value::Table(b)) => {
            for (k, v) in b {
                match a.get_mut(k) {
                    Some(slot) => merge(slot, v),
                    None => {
                        a.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (slot, v) => *slot = v.clone(),
    }
}

impl MapConfig {
    /// Builds [`MapConfig::shapes`]; `border` is the world border now (`[min_x, min_z,
    /// max_x, max_z]`), for a `world-border` limit.
    pub fn resolve(&mut self, border: Option<[f64; 4]>) {
        self.shapes = visibility::resolve(&self.visibility_limits, border.map(visibility::border_shape));
    }

    /// `VisibilityLimitImpl.shouldRenderColumn`: inside any enabled shape, or no shape at all.
    pub fn visible(&self, x: i32, z: i32) -> bool {
        visibility::any(&self.shapes, |s| s.column(x, z))
    }

    /// `shouldRenderChunk`.
    pub fn chunk_visible(&self, cx: i32, cz: i32) -> bool {
        visibility::any(&self.shapes, |s| s.chunk(cx, cz))
    }

    /// `shouldRenderRegion`.
    pub fn region_visible(&self, rx: i32, rz: i32) -> bool {
        visibility::any(&self.shapes, |s| s.region(rx, rz))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_defaults_match_the_original() {
        let w = WorldConfig::default();
        assert_eq!(w.invisible_blocks.len(), 7);
        assert!(w.invisible_blocks.contains(&"minecraft:tall_dry_grass".to_string()));
        let c = Config::parse("[settings]\ndebug-mode = true\nupdate-checker = false\n[settings.internal-webserver]\nflush-json-immediately = true\n").unwrap();
        assert!(c.settings.debug_mode && !c.settings.update_checker);
        assert!(c.settings.internal_webserver.flush_json_immediately);
    }

    #[test]
    fn render_limits_follow_render_config() {
        // defaults: workers 3 -> chunk requests 288, active chunks 192, cache size = active chunks
        let l = RenderLimits::default();
        let r = Auto::Number(3);
        let one = RenderLimits { workers: r, ..RenderLimits::default() }.resolve("map.render.full", 1, None);
        assert_eq!((one.workers, one.max_chunk_requests, one.max_active_chunks), (3, 288, 192));
        assert_eq!((one.max_active_regions, one.max_column_chunks, one.idle_chunk_cache_size), (2, 16, 192));
        let d = l.resolve("map.render.full", 4, None);
        assert_eq!((d.workers, d.max_active_chunks), (4, 256));
        // an invalid value falls back; zero is allowed only for the idle cache
        let bad = RenderLimits {
            workers: Auto::Text("many".into()),
            max_active_regions: Some(0),
            idle_chunk_cache_size: Auto::Number(0),
            ..RenderLimits::default()
        }
        .resolve("map.render.radius", 2, None);
        assert_eq!((bad.workers, bad.max_active_regions, bad.idle_chunk_cache_size), (2, 2, 0));
        // the pre-1.4 key
        let legacy = l.resolve("map.render.full", 4, Some(&Auto::Number(6)));
        assert_eq!(legacy.workers, 6);
        assert_eq!(l.resolve("map.render.full", 4, Some(&Auto::Number(-1))).workers, 4);
    }

    #[test]
    fn render_limits_are_read_from_the_file() {
        let c = Config::parse(
            "[world-settings.default.map.render.full]\nworkers = 5\nmax-active-regions = 3\n[world-settings.default.map.render.background]\nworkers = \"default\"\n[world-settings.default.map.image-saving]\nworkers = 4\nmax-pending-images = 7\n",
        )
        .unwrap();
        let w = c.world("minecraft:overworld");
        let f = w.map.full_limits();
        assert_eq!((f.workers, f.max_active_regions, f.max_active_chunks), (5, 3, 320));
        assert_eq!(w.map.image_saving.resolve(), (4, 7));
        assert_eq!(WorldConfig::default().map.image_saving.resolve(), (2, 100));
        assert!(w.map.render.background.resolved_workers(None) >= 1);
    }

    #[test]
    fn default_file_has_the_server_values() {
        let c = Config::parse(DEFAULT_TOML).unwrap();
        assert_eq!(c.settings.internal_webserver.bind, "127.0.0.1");
        let w = c.world("minecraft:overworld");
        assert_eq!(w.map.biomes.blend_biomes, 3);
        assert_eq!(w.map.render.background.interval_seconds, 15);
        assert_eq!(w.invisible_blocks.len(), 7);
        assert_eq!(w.color_overrides.blocks["minecraft:lava"], "#EA5C0F");
        // the shipped rectangle example is disabled, as in squaremap, so nothing is limited
        assert!(w.map.visible(10_001, 0));
        assert!(!w.map.visibility_limits[0].enabled);
    }

    #[test]
    fn a_world_table_overrides_default_keys_only() {
        let c = Config::parse(
            "[world-settings.default.map]\norder = 1\n[world-settings.\"minecraft:the_nether\".map]\norder = 5\n",
        )
        .unwrap();
        assert_eq!(c.world("minecraft:the_nether").map.order, 5);
        assert_eq!(c.world("minecraft:overworld").map.order, 1);
        assert!(c.world("minecraft:the_nether").map.glass.clear);
    }

    #[test]
    fn a_custom_level_has_its_own_section_and_the_default_for_the_rest() {
        let c = Config::parse(
            "[world-settings.default.map]\norder = 1\n[world-settings.\"mod:castle\".map]\nenabled = false\n",
        )
        .unwrap();
        assert!(!c.world("mod:castle").map.enabled);
        assert_eq!(c.world("mod:castle").map.order, 1);
        assert!(c.world("minecraft:the_end").map.enabled);
        assert_eq!(c.world("minecraft:the_end").map.order, 1);
    }
}
