//! What the renderer knows of each block state and biome, built once per world from the
//! game's registries (handed over as [`RawState`]/[`RawBiome`]) and the world's config.
//! The colour rules are squaremap's `BlockColors`, `BiomeColorTables` and
//! `BiomeColorSampler` (MIT, Copyright (c) 2021-2024 Jason Penilla).
use crate::color::{self, WATER_MAP_COLOR};
use crate::config::WorldConfig;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Fluid {
    #[default]
    None,
    Water,
    Lava,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GrassModifier {
    #[default]
    None,
    DarkForest,
    Swamp,
}

/// One block state as the game describes it.
#[derive(Clone, Debug, Default)]
pub struct RawState {
    /// `minecraft:oak_log`, without properties.
    pub name: String,
    /// The state's properties (`axis` = `y`), to find it from a saved chunk's palette.
    pub properties: Vec<(String, String)>,
    /// The `age` property, where the state has one.
    pub age: Option<u8>,
    /// `getMapColor(..).id` of this state and of its block's default state.
    pub map_color: u8,
    pub default_map_color: u8,
    /// `getMapColor(..).col` (0xRRGGBB) as the game answers it (`native_block_map_color`);
    /// None falls back to the vanilla table in `color::MAP_COLORS` by `map_color`.
    pub map_rgb: Option<u32>,
    /// Its fluid state (a waterlogged block holds water).
    pub fluid: Fluid,
}

/// One biome as the game describes it; colours are 0xRRGGBB as the game tints with them
/// (an override from the biome file, or the colormap's). Where `grass` comes from the game's
/// own tint (`native_biome_tint`) the dark forest's modifier is already applied: such a biome
/// is handed over with `GrassModifier::None`, see [`grass_modifier_after_tint`].
#[derive(Clone, Debug, Default)]
pub struct RawBiome {
    pub name: String,
    pub temperature: f32,
    pub downfall: f32,
    pub grass: u32,
    pub foliage: u32,
    pub dry_foliage: u32,
    pub water: u32,
    pub modifier: GrassModifier,
}

pub const AIR: u8 = 1;
pub const GLASS: u8 = 2;
pub const PLAIN_GLASS: u8 = 4;
pub const INVISIBLE: u8 = 8;
/// One of `iterate-up-base-blocks`.
pub const BASE: u8 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tint {
    #[default]
    None,
    Grass,
    Foliage,
    DryFoliage,
    Water,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StateInfo {
    /// `BlockColors.color`: an override, a state-dependent colour, else the map colour.
    pub color: u32,
    pub flags: u8,
    pub tint: Tint,
    pub fluid: Fluid,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BiomeTint {
    pub grass: u32,
    pub foliage: u32,
    pub dry_foliage: u32,
    pub water: u32,
    pub modifier: GrassModifier,
}

/// A world's lookup tables, by state id and biome id.
pub struct Tables {
    pub states: Vec<StateInfo>,
    pub biomes: Vec<BiomeTint>,
}

const GRASS_BLOCKS: [&str; 8] = [
    "minecraft:grass_block",
    "minecraft:short_grass",
    "minecraft:tall_grass",
    "minecraft:fern",
    "minecraft:large_fern",
    "minecraft:potted_fern",
    "minecraft:sugar_cane",
    "minecraft:bush",
];
const FOLIAGE_BLOCKS: [&str; 6] = [
    "minecraft:vine",
    "minecraft:oak_leaves",
    "minecraft:jungle_leaves",
    "minecraft:acacia_leaves",
    "minecraft:dark_oak_leaves",
    "minecraft:mangrove_leaves",
];

impl Tables {
    pub fn build(states: &[RawState], biomes: &[RawBiome], config: &WorldConfig) -> Self {
        let blocks = &config.color_overrides.blocks;
        let states = states
            .iter()
            .map(|s| {
                let name = s.name.as_str();
                let air = matches!(name, "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air");
                let plain_glass = name == "minecraft:glass";
                let glass = plain_glass || name.ends_with("_stained_glass");
                let mut flags = 0;
                if air {
                    flags |= AIR;
                }
                if glass {
                    flags |= GLASS;
                }
                if plain_glass {
                    flags |= PLAIN_GLASS;
                }
                if config.invisible_blocks.iter().any(|b| b == name) {
                    flags |= INVISIBLE;
                }
                if config.iterate_up_base_blocks.iter().any(|b| b == name) {
                    flags |= BASE;
                }
                let tint = if GRASS_BLOCKS.contains(&name) {
                    Tint::Grass
                } else if FOLIAGE_BLOCKS.contains(&name) {
                    Tint::Foliage
                } else if name == "minecraft:leaf_litter" {
                    Tint::DryFoliage
                } else if s.default_map_color == WATER_MAP_COLOR {
                    Tint::Water
                } else {
                    Tint::None
                };
                let color = blocks
                    .get(name)
                    .and_then(|hex| color::parse_hex(hex))
                    .or_else(|| dynamic_color(name, s.age))
                    .unwrap_or_else(|| s.map_rgb.unwrap_or_else(|| color::map_color(s.map_color)));
                StateInfo { color, flags, tint, fluid: s.fluid }
            })
            .collect();
        let o = &config.color_overrides.biomes;
        let pick = |map: &std::collections::BTreeMap<String, String>, name: &str, base: u32| {
            map.get(name).and_then(|hex| color::parse_hex(hex)).unwrap_or(base)
        };
        let biomes = biomes
            .iter()
            .map(|b| {
                // squaremap blends the plant colour into a biome's default foliage
                // (one the biome file doesn't set).
                let sampled = colormap_foliage(b.temperature, b.downfall);
                let foliage = if Some(b.foliage) == sampled {
                    color::mix(color::PLANT, b.foliage, 0.85) & 0xFF_FFFF
                } else {
                    b.foliage
                };
                BiomeTint {
                    grass: pick(&o.grass, &b.name, b.grass),
                    foliage: pick(&o.foliage, &b.name, foliage),
                    dry_foliage: pick(&o.dry_foliage, &b.name, b.dry_foliage),
                    water: pick(&o.water, &b.name, b.water),
                    modifier: b.modifier,
                }
            })
            .collect();
        Self { states, biomes }
    }

    #[inline]
    pub fn state(&self, id: u32) -> StateInfo {
        self.states.get(id as usize).copied().unwrap_or(StateInfo { flags: AIR, ..StateInfo::default() })
    }

    #[inline]
    pub fn biome(&self, id: u16) -> BiomeTint {
        self.biomes.get(usize::from(id)).copied().unwrap_or_default()
    }
}

/// The modifier the scanner still has to apply to a biome whose grass colour came from the
/// game's tint: that tint already holds the dark forest's step, and a swamp's grass is
/// per column (read from the snapshot's [`crate::scan::SwampGrid`] or the world env).
pub fn grass_modifier_after_tint(game: GrassModifier) -> GrassModifier {
    match game {
        GrassModifier::Swamp => GrassModifier::Swamp,
        GrassModifier::None | GrassModifier::DarkForest => GrassModifier::None,
    }
}

/// `BlockColors`' state-dependent colours.
fn dynamic_color(name: &str, age: Option<u8>) -> Option<u32> {
    let age = u32::from(age?);
    match name {
        "minecraft:melon_stem" | "minecraft:pumpkin_stem" => {
            Some((age * 32) << 16 | (255 - age * 8) << 8 | age * 4)
        }
        "minecraft:wheat" => {
            Some(color::mix(color::PLANT, 0xDCBB65, (age + 1) as f32 / 8.0))
        }
        _ => None,
    }
}

/// `FoliageColor.get` from the colormap the web frontend ships (`images/foliage.png`).
fn colormap_foliage(temperature: f32, downfall: f32) -> Option<u32> {
    static MAP: OnceLock<Vec<u32>> = OnceLock::new();
    let map = MAP.get_or_init(|| {
        crate::image::decode_rgb(crate::assets::file("images/foliage.png").unwrap_or_default())
            .map(|(_, _, px)| px)
            .unwrap_or_default()
    });
    let t = f64::from(temperature.clamp(0.0, 1.0));
    let h = f64::from(downfall.clamp(0.0, 1.0));
    let i = ((1.0 - t) * 255.0) as usize;
    let j = ((1.0 - h * t) * 255.0) as usize;
    map.get(j << 8 | i).map(|c| c & 0xFF_FFFF)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, DEFAULT_TOML};

    fn world() -> WorldConfig {
        Config::parse(DEFAULT_TOML).unwrap().world("minecraft:overworld")
    }

    #[test]
    fn overrides_dynamic_and_flags() {
        let raw = [
            RawState { name: "minecraft:air".into(), ..Default::default() },
            RawState { name: "minecraft:poppy".into(), map_color: 7, ..Default::default() },
            RawState { name: "minecraft:wheat".into(), age: Some(7), map_color: 7, ..Default::default() },
            RawState { name: "minecraft:glass".into(), ..Default::default() },
            RawState { name: "minecraft:short_grass".into(), map_color: 7, ..Default::default() },
            RawState { name: "minecraft:stone".into(), map_color: 11, ..Default::default() },
            RawState { name: "minecraft:water".into(), map_color: 12, default_map_color: 12, fluid: Fluid::Water, ..Default::default() },
        ];
        let t = Tables::build(&raw, &[], &world());
        assert_eq!(t.state(0).flags & AIR, AIR);
        assert_eq!(t.state(1).color, 0xED302C);
        assert_eq!(t.state(2).color, 0xDCBB65); // age 7: mix ratio 1.0 returns the crop colour as is
        assert_eq!(t.state(3).flags, GLASS | PLAIN_GLASS);
        assert_eq!(t.state(4).flags & INVISIBLE, INVISIBLE);
        assert_eq!(t.state(4).tint, Tint::Grass);
        assert_eq!(t.state(5).color, 7368816);
        assert_eq!(t.state(6).tint, Tint::Water);
        assert_eq!(t.state(99).flags, AIR);
    }

    #[test]
    fn the_games_map_colour_wins_over_the_table_and_overrides_win_over_both() {
        let raw = [
            RawState { name: "minecraft:stone".into(), map_color: 11, map_rgb: Some(0x123456), ..Default::default() },
            RawState { name: "minecraft:dirt".into(), map_color: 10, map_rgb: None, ..Default::default() },
            RawState { name: "minecraft:poppy".into(), map_color: 7, map_rgb: Some(0x00FF00), ..Default::default() },
        ];
        let t = Tables::build(&raw, &[], &world());
        assert_eq!(t.state(0).color, 0x123456);
        assert_eq!(t.state(1).color, color::map_color(10));
        // The default config overrides the poppy's colour.
        assert_eq!(t.state(2).color, 0xED302C);
    }

    #[test]
    fn a_tinted_dark_forest_is_not_stepped_twice() {
        assert_eq!(grass_modifier_after_tint(GrassModifier::DarkForest), GrassModifier::None);
        assert_eq!(grass_modifier_after_tint(GrassModifier::Swamp), GrassModifier::Swamp);
        assert_eq!(grass_modifier_after_tint(GrassModifier::None), GrassModifier::None);
        let raw = [RawBiome {
            name: "minecraft:dark_forest".into(),
            grass: 0x507A32,
            modifier: grass_modifier_after_tint(GrassModifier::DarkForest),
            ..Default::default()
        }];
        let t = Tables::build(&[], &raw, &world());
        assert_eq!(t.biome(0).grass, 0x507A32);
        assert_eq!(t.biome(0).modifier, GrassModifier::None);
    }

    #[test]
    fn foliage_default_is_mixed_with_plant_and_overrides_win() {
        let sampled = colormap_foliage(0.8, 0.4).expect("colormap present");
        let raw = [
            RawBiome { name: "minecraft:plains".into(), temperature: 0.8, downfall: 0.4, foliage: sampled, ..Default::default() },
            RawBiome { name: "minecraft:jungle".into(), temperature: 0.95, downfall: 0.9, foliage: 1, ..Default::default() },
            RawBiome { name: "minecraft:swamp".into(), temperature: 0.8, downfall: 0.9, foliage: 0x6A7039, ..Default::default() },
        ];
        let t = Tables::build(&[], &raw, &world());
        assert_eq!(t.biome(0).foliage, color::mix(color::PLANT, sampled, 0.85) & 0xFF_FFFF);
        assert_eq!(t.biome(1).foliage, 0x1F8907);
        assert_eq!(t.biome(2).foliage, 0x6A7039);
    }
}
