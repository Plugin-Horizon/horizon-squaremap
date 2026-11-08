//! The mod's config: squaremap's `config.yml` and `advanced.yml` as one TOML file, with the
//! same key names. `world-settings.default` applies to every world; a table named after a
//! world (`[world-settings."minecraft:the_nether".map]`) overrides keys of it.
use crate::visibility::{self, Shape};
use serde::Deserialize;

/// Written to `config/squaremap-server.toml` when it is missing.
pub const DEFAULT_TOML: &str = include_str!("../squaremap-server.toml");

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

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Render {
    pub background: Background,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Background {
    pub enabled: bool,
    pub interval_seconds: u64,
    pub max_chunks_per_interval: usize,
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
            web_address: "http://localhost:8080".into(),
            web_directory: WebDirectory { path: "squaremap/web".into(), auto_update: true },
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
        Self { enabled: true, bind: "127.0.0.1".into(), port: 8080 }
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

impl Default for Render {
    fn default() -> Self {
        Self { background: Background::default() }
    }
}

impl Default for Background {
    fn default() -> Self {
        Self { enabled: true, interval_seconds: 15, max_chunks_per_interval: 2048 }
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
}
