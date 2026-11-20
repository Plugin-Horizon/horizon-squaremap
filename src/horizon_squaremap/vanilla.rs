//! The block and biome lists the map starts with, read from the game's resource folder (the one
//! the engine was started with: `--resources <dir>`). arismoko has no registry listing, so the
//! names come from `registries.json` and `data/minecraft/worldgen/biome/*.json`; the host then
//! answers each block's states and map colours (see the glue in `src/lib.rs`).
use crate::horizon_squaremap::tables::{GrassModifier, RawBiome};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

/// Every block id (`minecraft:stone`), sorted, from `registries.json` (`minecraft:block`).
pub fn block_names(resources: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(resources.join("registries.json")) else { return Vec::new() };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else { return Vec::new() };
    let mut names: Vec<String> = value
        .get("minecraft:block")
        .and_then(|b| b.get("entries"))
        .and_then(|e| e.as_object())
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    names.sort();
    names
}

/// `minecraft:oak_log[axis=y]` as its name and properties; a state with no brackets has none.
pub fn parse_state(text: &str) -> (String, Vec<(String, String)>) {
    let Some((name, rest)) = text.split_once('[') else { return (text.to_owned(), Vec::new()) };
    let props = rest
        .trim_end_matches(']')
        .split(',')
        .filter_map(|p| p.split_once('=').map(|(k, v)| (k.to_owned(), v.to_owned())))
        .collect();
    (name.to_owned(), props)
}

/// `#RRGGBB` (or a plain integer) as 0xRRGGBB.
fn colour(v: &serde_json::Value) -> Option<u32> {
    match v {
        serde_json::Value::String(s) => u32::from_str_radix(s.trim_start_matches('#'), 16).ok(),
        serde_json::Value::Number(n) => n.as_u64().map(|n| n as u32 & 0xFF_FFFF),
        _ => None,
    }
}

/// The game's three biome colour maps (`assets/minecraft/textures/colormap/<name>.png`), read from the
/// resource folder at start: the mod ships no Minecraft textures. Empty until [`load_colormaps`] ran or
/// when a file is missing; a biome then gets the flat default colour of its kind.
static COLORMAPS: Mutex<BTreeMap<String, Vec<u32>>> = Mutex::new(BTreeMap::new());

/// Reads `grass`, `foliage` and `dry_foliage` from the resource folder (replacing earlier ones).
pub fn load_colormaps(resources: &Path) {
    let dir = resources.join("assets").join("minecraft").join("textures").join("colormap");
    let mut maps = BTreeMap::new();
    for name in ["grass", "foliage", "dry_foliage"] {
        let Ok(bytes) = std::fs::read(dir.join(format!("{name}.png"))) else { continue };
        if let Ok((_, _, px)) = crate::horizon_squaremap::image::decode_rgb(&bytes) {
            maps.insert(name.to_owned(), px);
        }
    }
    *COLORMAPS.lock().unwrap_or_else(|e| e.into_inner()) = maps;
}

/// Replaces the colour maps (tests hand in a synthetic 256x256 map).
#[cfg(test)]
pub fn set_colormap_for_tests(name: &str, px: Vec<u32>) {
    COLORMAPS.lock().unwrap_or_else(|e| e.into_inner()).insert(name.to_owned(), px);
}

/// `GrassColor.get` / `FoliageColor.get` on one of the colour maps; `None` when it is not loaded.
pub fn colormap(name: &str, temperature: f32, downfall: f32) -> Option<u32> {
    let t = f64::from(temperature.clamp(0.0, 1.0));
    let h = f64::from(downfall.clamp(0.0, 1.0)) * t;
    let i = ((1.0 - t) * 255.0) as usize;
    let j = ((1.0 - h) * 255.0) as usize;
    COLORMAPS.lock().unwrap_or_else(|e| e.into_inner()).get(name)?.get(j << 8 | i).map(|c| c & 0xFF_FFFF)
}

/// Every biome of `data/minecraft/worldgen/biome`, by file name, with the climate and the colours
/// its file sets (the colormaps' otherwise). Sorted by name. Biomes of data packs are not listed.
pub fn biomes(resources: &Path) -> Vec<RawBiome> {
    load_colormaps(resources);
    let dir = resources.join("data").join("minecraft").join("worldgen").join("biome");
    let Ok(read) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut out = Vec::new();
    for entry in read.flatten() {
        let path = entry.path();
        let (Some(stem), Some("json")) = (path.file_stem().and_then(|s| s.to_str()), path.extension().and_then(|e| e.to_str())) else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
        let temperature = v.get("temperature").and_then(|t| t.as_f64()).unwrap_or(0.5) as f32;
        let downfall = v.get("downfall").and_then(|t| t.as_f64()).unwrap_or(0.5) as f32;
        let fx = v.get("effects");
        let set = |key: &str| fx.and_then(|e| e.get(key)).and_then(colour);
        let modifier = match fx.and_then(|e| e.get("grass_color_modifier")).and_then(|m| m.as_str()) {
            Some("dark_forest") => GrassModifier::DarkForest,
            Some("swamp") => GrassModifier::Swamp,
            _ => GrassModifier::None,
        };
        out.push(RawBiome {
            name: format!("minecraft:{stem}"),
            temperature,
            downfall,
            grass: set("grass_color").or_else(|| colormap("grass", temperature, downfall)).unwrap_or(0x91BD59),
            foliage: set("foliage_color").or_else(|| colormap("foliage", temperature, downfall)).unwrap_or(0x77AB2F),
            dry_foliage: set("dry_foliage_color").or_else(|| colormap("dry_foliage", temperature, downfall)).unwrap_or(0x7B5334),
            water: set("water_color").unwrap_or(0x3F76E4),
            modifier,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_state_string_splits_into_name_and_properties() {
        assert_eq!(parse_state("minecraft:stone"), ("minecraft:stone".to_owned(), vec![]));
        let (name, props) = parse_state("minecraft:oak_log[axis=y]");
        assert_eq!(name, "minecraft:oak_log");
        assert_eq!(props, vec![("axis".to_owned(), "y".to_owned())]);
        let (_, props) = parse_state("minecraft:wheat[age=3,x=1]");
        assert_eq!(props.len(), 2);
    }

    #[test]
    fn colours_parse_from_hex_text() {
        assert_eq!(colour(&serde_json::json!("#6a7039")), Some(0x6A7039));
        assert_eq!(colour(&serde_json::json!(5)), Some(5));
    }

    #[test]
    fn colormaps_are_read_from_the_resource_folder() {
        // Needs a prepared game resource folder; without one the check is skipped.
        let Some(dir) = std::env::var_os("HORIZON_TEST_RESOURCES") else { return };
        load_colormaps(std::path::Path::new(&dir));
        let plains = colormap("grass", 0.8, 0.4).expect("grass colormap");
        let desert = colormap("grass", 2.0, 0.0).expect("grass colormap");
        assert_ne!(plains, desert);
        assert!(colormap("foliage", 0.8, 0.4).is_some() && colormap("dry_foliage", 0.8, 0.4).is_some());
    }
}
