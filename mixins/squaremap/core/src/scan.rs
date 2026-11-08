//! A chunk's 16x16 map pixels: squaremap's `ChunkScanner` and `BiomeColorSampler`
//! (MIT, Copyright (c) 2021-2024 Jason Penilla), over a read-only view of the level.
use crate::color::{self, Blender};
use crate::config::MapConfig;
use crate::tables::{Fluid, GrassModifier, StateInfo, Tables, Tint, AIR, BASE, GLASS, INVISIBLE, PLAIN_GLASS};

/// A pixel no scan wrote (`Integer.MIN_VALUE` in squaremap).
pub const UNSET: u32 = 0x8000_0000;

/// A snapshot of the chunk to render and its neighbours, read off the server thread.
pub trait ChunkView: Send {
    fn chunk(&self) -> (i32, i32);
    fn min_y(&self) -> i32;
    /// One above the highest block Y.
    fn top_y(&self) -> i32;
    fn has_ceiling(&self) -> bool;
    /// `Heightmap.WORLD_SURFACE`'s first free Y (one above the highest non-air block, or
    /// the minimum Y); None where the column's chunk isn't in the snapshot.
    fn surface(&self, x: i32, z: i32) -> Option<i32>;
    /// The state id at a block of a chunk in the snapshot (air elsewhere).
    fn state(&self, x: i32, y: i32, z: i32) -> u32;
    /// `BiomeManager.getBiome`'s zoom: the noise (quart) cell whose biome a block shows.
    fn quart_at_block(&self, x: i32, y: i32, z: i32) -> [i32; 3];
    /// The biome id of a noise (quart) cell; None outside the snapshot.
    fn noise_biome(&self, qx: i32, qy: i32, qz: i32) -> Option<u16>;
    /// The swamp's noise grass colour at a column (`GrassColorModifier.SWAMP`).
    fn swamp_grass(&self, biome: u16, x: i32, z: i32) -> u32;
}

/// The swamp's grass colour (`GrassColorModifier.SWAMP`) of every column of a snapshot's
/// 3x3 chunks, taken on the server thread from the game's tint (`native_biome_tint`), so the
/// render thread reads it without the game. Origin: the north-west column of the chunk
/// north-west of the centre; columns past the edge read the nearest one.
#[derive(Clone)]
pub struct SwampGrid {
    x0: i32,
    z0: i32,
    cells: Box<[u32]>,
}

impl SwampGrid {
    pub const SIDE: i32 = 48;

    /// The grid around chunk `cx, cz`, `grass(x, z)` asked once per column.
    pub fn take(cx: i32, cz: i32, mut grass: impl FnMut(i32, i32) -> u32) -> SwampGrid {
        let (x0, z0) = ((cx - 1) * 16, (cz - 1) * 16);
        let mut cells = Vec::with_capacity((Self::SIDE * Self::SIDE) as usize);
        for z in 0..Self::SIDE {
            for x in 0..Self::SIDE {
                cells.push(grass(x0 + x, z0 + z));
            }
        }
        SwampGrid { x0, z0, cells: cells.into_boxed_slice() }
    }

    pub fn get(&self, x: i32, z: i32) -> u32 {
        let dx = (x - self.x0).clamp(0, Self::SIDE - 1);
        let dz = (z - self.z0).clamp(0, Self::SIDE - 1);
        self.cells[(dz * Self::SIDE + dx) as usize]
    }
}

/// Where the biome of a noise cell outside the view comes from (saved chunks).
pub type BiomeFallback<'a> = &'a dyn Fn(i32, i32, i32) -> Option<u16>;

/// Renders the chunk `view.chunk()` names: 256 ARGB pixels, `z << 4 | x`.
pub fn scan_chunk(view: &dyn ChunkView, tables: &Tables, map: &MapConfig) -> [u32; 256] {
    scan_chunk_with(view, tables, map, false, None)
}

/// [`scan_chunk`], only the north row (`z = 0`) with `top_row_only` (squaremap redraws it
/// when the chunk north of it changed: its shading reads that chunk's heights), and with
/// biomes outside the view read from `fallback`.
pub fn scan_chunk_with(
    view: &dyn ChunkView,
    tables: &Tables,
    map: &MapConfig,
    top_row_only: bool,
    fallback: Option<BiomeFallback<'_>>,
) -> [u32; 256] {
    let mut s = Scanner { view, tables, map, min_y: view.min_y(), fallback };
    let (cx, cz) = view.chunk();
    let mut out = [UNSET; 256];
    let mut last_y = s.bottom_row_heights(cx, cz - 1);
    let rows = if top_row_only { 1 } else { 16 };
    for x in 0..16 {
        for z in 0..rows {
            let (bx, bz) = (cx * 16 + x, cz * 16 + z);
            if map.visible(bx, bz) {
                out[(z << 4 | x) as usize] = s.scan_block(bx, bz, x, z, &mut last_y);
            }
        }
    }
    out
}

struct Scanner<'a> {
    view: &'a dyn ChunkView,
    tables: &'a Tables,
    map: &'a MapConfig,
    min_y: i32,
    fallback: Option<BiomeFallback<'a>>,
}

impl Scanner<'_> {
    fn info(&self, x: i32, y: i32, z: i32) -> StateInfo {
        self.tables.state(self.view.state(x, y, z))
    }

    fn max_height(&self) -> i32 {
        if self.map.max_height == -1 { self.view.top_y() } else { self.map.max_height }
    }

    /// The heights of the north neighbour's last row, for the first row's shading.
    fn bottom_row_heights(&self, cx: i32, cz: i32) -> [i32; 16] {
        let mut last = [0; 16];
        if self.view.surface(cx * 16, cz * 16 + 15).is_none() {
            return last;
        }
        for (x, slot) in last.iter_mut().enumerate() {
            let (bx, bz) = (cx * 16 + x as i32, cz * 16 + 15);
            let top = self.view.surface(bx, bz).unwrap_or(self.min_y);
            let mut y = top.min(self.max_height());
            let state = self.iterate(bx, &mut y, bz);
            if self.map.glass.clear && state.flags & GLASS != 0 {
                self.handle_glass(bx, &mut y, bz);
            }
            *slot = y;
        }
        last
    }

    fn scan_block(&mut self, x: i32, z: i32, lx: i32, lz: i32, last_y: &mut [i32; 16]) -> u32 {
        let top = self.view.surface(x, z).unwrap_or(self.min_y);
        if top <= self.min_y {
            return 0;
        }
        let mut y = top.min(self.max_height());
        let state = self.iterate(x, &mut y, z);
        if self.map.glass.clear && state.flags & GLASS != 0 {
            let glass_color = state.color;
            let alpha = if state.flags & PLAIN_GLASS != 0 { 0.25 } else { 0.5 };
            let below = self.handle_glass(x, &mut y, z);
            let c = self.color(x, y, z, lx, lz, last_y, below);
            return color::mix(c, glass_color, alpha);
        }
        self.color(x, y, z, lx, lz, last_y, state)
    }

    #[allow(clippy::too_many_arguments)]
    fn color(&self, x: i32, y: i32, z: i32, lx: i32, lz: i32, last_y: &mut [i32; 16], state: StateInfo) -> u32 {
        let mut c = state.color;
        if self.map.biomes.enabled {
            c = self.biome_color(c, state, x, y, z);
        }
        let odd = (lx + lz) & 1;
        let previous = last_y[lx as usize];
        last_y[lx as usize] = y;
        if let Some((depth, under)) = self.fluid_depth(x, y, z, state) {
            return self.fluid_color(depth, c, state, under, odd);
        }
        let diff = f64::from(y - previous) + (f64::from(odd) - 0.5) * 0.4;
        color::shade_step(c, if diff > 0.6 { 2 } else if diff < -0.6 { 0 } else { 1 })
    }

    fn iterate(&self, x: i32, y: &mut i32, z: i32) -> StateInfo {
        if self.map.iterate_up { self.iterate_up(x, y, z) } else { self.iterate_down(x, y, z) }
    }

    /// `ChunkScanner.iterateUp` (`map.iterate-up`): in a level with a ceiling, climb from
    /// the bottom out of the first solid layer, then on to the first base block
    /// (`iterate-up-base-blocks`), and draw what is below that.
    fn iterate_up(&self, x: i32, y: &mut i32, z: i32) -> StateInfo {
        let height = *y;
        *y = self.min_y;
        if self.view.has_ceiling() {
            loop {
                *y += 1;
                if self.info(x, *y, z).flags & AIR != 0 || *y >= height {
                    break;
                }
            }
            loop {
                *y += 1;
                if self.info(x, *y, z).flags & BASE != 0 || *y >= height {
                    break;
                }
            }
        }
        loop {
            *y -= 1;
            let state = self.info(x, *y, z);
            let clear = state.color == 0 || state.flags & INVISIBLE != 0;
            if !clear || *y <= self.min_y {
                return state;
            }
        }
    }

    fn iterate_down(&self, x: i32, y: &mut i32, z: i32) -> StateInfo {
        let mut state;
        if self.view.has_ceiling() {
            loop {
                *y -= 1;
                state = self.info(x, *y, z);
                if state.flags & AIR != 0 || *y <= self.min_y {
                    break;
                }
            }
        }
        loop {
            *y -= 1;
            state = self.info(x, *y, z);
            let clear = state.color == 0 || state.flags & INVISIBLE != 0;
            if !clear || *y <= self.min_y {
                return state;
            }
        }
    }

    fn handle_glass(&self, x: i32, y: &mut i32, z: i32) -> StateInfo {
        let mut state = self.info(x, *y, z);
        while state.flags & GLASS != 0 {
            state = self.iterate_down(x, y, z);
        }
        state
    }

    fn fluid_depth(&self, x: i32, y: i32, z: i32, state: StateInfo) -> Option<(i32, StateInfo)> {
        if y <= self.min_y || state.fluid == Fluid::None {
            return None;
        }
        let mut depth = 0;
        let mut yy = y - 1;
        loop {
            let under = self.info(x, yy, z);
            yy -= 1;
            depth += 1;
            if !(yy > self.min_y && depth <= 10 && under.fluid != Fluid::None) {
                return Some((depth, under));
            }
        }
    }

    fn fluid_color(&self, depth: i32, mut c: u32, state: StateInfo, under: StateInfo, odd: i32) -> u32 {
        let checker = |c: u32| {
            let diff = f64::from(depth) * 0.1 + f64::from(odd) * 0.2;
            color::shade_step(c, if diff < 0.5 { 2 } else if diff > 0.9 { 0 } else { 1 })
        };
        let mut shaded = false;
        match state.fluid {
            Fluid::Water => {
                if self.map.water.checkerboard {
                    c = checker(c);
                    shaded = true;
                }
                if self.map.water.clear_depth {
                    if !self.map.water.checkerboard {
                        c = color::shade(c, 0.85f32 - depth as f32 * 0.01f32);
                    }
                    c = color::mix(c, under.color, 0.20f32 / (depth as f32 / 2.0f32));
                    shaded = true;
                }
            }
            Fluid::Lava if self.map.lava.checkerboard => {
                c = checker(c);
                shaded = true;
            }
            _ => {}
        }
        if shaded { c } else { color::remove_alpha(c) }
    }

    /// `BiomeColorSampler.modifyColorFromBiome`.
    fn biome_color(&self, c: u32, state: StateInfo, x: i32, y: i32, z: i32) -> u32 {
        let blend = self.map.biomes.blend_biomes;
        let t = self.tables;
        match state.tint {
            Tint::None => c,
            Tint::Grass => {
                if blend > 0 {
                    match self.uniform(x, y, z, blend) {
                        Some(b) if t.biome(b).modifier != GrassModifier::Swamp => self.grass(b, x, z),
                        _ => self.blend(x, y, z, blend, |b, sx, sz| self.grass(b, sx, sz)),
                    }
                } else {
                    self.grass(self.biome_at(x, y, z), x, z)
                }
            }
            Tint::Foliage => self.sample(x, y, z, blend, |b| t.biome(b).foliage),
            Tint::DryFoliage => self.sample(x, y, z, blend, |b| t.biome(b).dry_foliage),
            Tint::Water => color::mix(c, self.sample(x, y, z, blend, |b| t.biome(b).water), 0.8),
        }
    }

    fn sample(&self, x: i32, y: i32, z: i32, blend: i32, f: impl Fn(u16) -> u32) -> u32 {
        if blend > 0 {
            match self.uniform(x, y, z, blend) {
                Some(b) => f(b),
                None => self.blend(x, y, z, blend, |b, _, _| f(b)),
            }
        } else {
            f(self.biome_at(x, y, z))
        }
    }

    /// A noise cell's biome: the view's, else the fallback's.
    fn noise(&self, qx: i32, qy: i32, qz: i32) -> Option<u16> {
        self.view.noise_biome(qx, qy, qz).or_else(|| self.fallback.and_then(|f| f(qx, qy, qz)))
    }

    /// `BiomeManager.getBiome`. A cell neither the view nor the fallback has reads the
    /// nearest cell of the chunk itself (squaremap asks the biome source there).
    fn biome_at(&self, x: i32, y: i32, z: i32) -> u16 {
        let [qx, qy, qz] = self.view.quart_at_block(x, y, z);
        if let Some(b) = self.noise(qx, qy, qz) {
            return b;
        }
        let (cx, cz) = self.view.chunk();
        self.view.noise_biome(qx.clamp(cx * 4, cx * 4 + 3), qy, qz.clamp(cz * 4, cz * 4 + 3)).unwrap_or(0)
    }

    fn grass(&self, biome: u16, x: i32, z: i32) -> u32 {
        let tint = self.tables.biome(biome);
        match tint.modifier {
            GrassModifier::None => tint.grass,
            GrassModifier::DarkForest => ((tint.grass & 0xFE_FEFE) + 0x28_340A) >> 1,
            GrassModifier::Swamp => self.view.swamp_grass(biome, x, z),
        }
    }

    /// `sampleNeighbors`: the mean over `[x - r, x + r)` x `[z - r, z + r)` at `y`.
    fn blend(&self, x: i32, y: i32, z: i32, r: i32, f: impl Fn(u16, i32, i32) -> u32) -> u32 {
        let mut b = Blender::default();
        for sx in x - r..x + r {
            for sz in z - r..z + r {
                b.add(f(self.biome_at(sx, y, sz), sx, sz));
            }
        }
        b.result()
    }

    /// The one biome every zoomed sample of the blend can pick, if the noise cells the
    /// zoom reaches all hold the same (the same result as sampling, without the zoom).
    fn uniform(&self, x: i32, y: i32, z: i32, r: i32) -> Option<u16> {
        let q = |v: i32| (v - 2) >> 2;
        let (qy0, qx0, qz0) = (q(y), q(x - r), q(z - r));
        let (qx1, qz1) = (q(x + r - 1) + 1, q(z + r - 1) + 1);
        let mut found = None;
        for qx in qx0..=qx1 {
            for qz in qz0..=qz1 {
                for qy in qy0..=qy0 + 1 {
                    let b = self.noise(qx, qy, qz)?;
                    if *found.get_or_insert(b) != b {
                        return None;
                    }
                }
            }
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, DEFAULT_TOML};
    use crate::tables::{RawBiome, RawState};

    /// A flat test level: per column a list of (y, state) from the bottom, one biome.
    struct Flat {
        columns: std::collections::HashMap<(i32, i32), Vec<u32>>,
        min_y: i32,
        ceiling: bool,
        biome: u16,
    }

    impl ChunkView for Flat {
        fn chunk(&self) -> (i32, i32) {
            (0, 0)
        }
        fn min_y(&self) -> i32 {
            self.min_y
        }
        fn top_y(&self) -> i32 {
            self.min_y + 32
        }
        fn has_ceiling(&self) -> bool {
            self.ceiling
        }
        fn surface(&self, x: i32, z: i32) -> Option<i32> {
            let col = self.columns.get(&(x, z))?;
            let top = col.iter().rposition(|&s| s != 0).map_or(self.min_y, |i| self.min_y + i as i32 + 1);
            Some(top)
        }
        fn state(&self, x: i32, y: i32, z: i32) -> u32 {
            self.columns
                .get(&(x, z))
                .and_then(|c| c.get((y - self.min_y) as usize))
                .copied()
                .unwrap_or(0)
        }
        fn quart_at_block(&self, x: i32, y: i32, z: i32) -> [i32; 3] {
            [x >> 2, y >> 2, z >> 2]
        }
        fn noise_biome(&self, _: i32, _: i32, _: i32) -> Option<u16> {
            Some(self.biome)
        }
        fn swamp_grass(&self, _: u16, _: i32, _: i32) -> u32 {
            0x4C763C
        }
    }

    // 0 air, 1 stone, 2 water, 3 grass block, 4 glass, 5 short grass (invisible)
    fn tables() -> (Tables, MapConfig) {
        let w = Config::parse(DEFAULT_TOML).unwrap().world("minecraft:overworld");
        let raw = [
            RawState { name: "minecraft:air".into(), ..Default::default() },
            RawState { name: "minecraft:stone".into(), map_color: 11, default_map_color: 11, ..Default::default() },
            RawState { name: "minecraft:water".into(), map_color: 12, default_map_color: 12, fluid: Fluid::Water, ..Default::default() },
            RawState { name: "minecraft:grass_block".into(), map_color: 1, default_map_color: 1, ..Default::default() },
            RawState { name: "minecraft:glass".into(), ..Default::default() },
            RawState { name: "minecraft:short_grass".into(), map_color: 7, default_map_color: 7, ..Default::default() },
        ];
        let biomes = [RawBiome { name: "minecraft:plains".into(), grass: 0x91BD59, water: 0x3F76E4, ..Default::default() }];
        (Tables::build(&raw, &biomes, &w), w.map)
    }

    fn level(fill: impl Fn(i32, i32) -> Vec<u32>) -> Flat {
        let mut columns = std::collections::HashMap::new();
        for x in 0..16 {
            for z in -1..16 {
                columns.insert((x, z), fill(x, z));
            }
        }
        Flat { columns, min_y: 0, ceiling: false, biome: 0 }
    }

    #[test]
    fn flat_grass_is_the_biome_colour_at_middle_shade() {
        let (t, map) = tables();
        let px = scan_chunk(&level(|_, _| vec![1, 1, 3, 5]), &t, &map);
        // Same height as the north row: diff = (odd - 0.5) * 0.4 = +-0.2 -> shade 1.
        assert_eq!(px[0], color::shade_step(0x91BD59, 1));
        assert_eq!(px[17], color::shade_step(0x91BD59, 1));
    }

    #[test]
    fn a_step_up_is_bright_and_down_is_dark() {
        let (t, map) = tables();
        let px = scan_chunk(&level(|_, z| if z == 3 { vec![1, 1, 1, 1] } else { vec![1, 1] }), &t, &map);
        let stone = color::map_color(11);
        assert_eq!(px[3 << 4], color::shade_step(stone, 2));
        assert_eq!(px[4 << 4], color::shade_step(stone, 0));
    }

    #[test]
    fn water_mixes_depth_and_floor() {
        let (t, map) = tables();
        let px = scan_chunk(&level(|_, _| vec![1, 1, 1, 2, 2]), &t, &map);
        let water = color::mix(color::map_color(12), 0x3F76E4, 0.8);
        // depth 2 (one water below, then stone): shade 0.83, then 0.2 of the floor.
        let expect = color::mix(color::shade(water, 0.85 - 2.0 * 0.01), color::map_color(11), 0.2 / (2.0 / 2.0));
        assert_eq!(px[0], expect);
    }

    #[test]
    fn glass_is_cleared_and_tints_what_is_under_it() {
        let (t, map) = tables();
        let px = scan_chunk(&level(|_, _| vec![1, 1, 4]), &t, &map);
        let under = color::shade_step(color::map_color(11), 1);
        assert_eq!(px[0], color::mix(under, 0xFFFFFF, 0.25));
    }

    #[test]
    fn empty_columns_are_transparent_and_limits_skip_columns() {
        let (t, mut map) = tables();
        assert_eq!(scan_chunk(&level(|_, _| vec![]), &t, &map)[5], 0);
        map.visibility_limits[0].enabled = true;
        map.visibility_limits[0].max_x = 3;
        map.resolve(None);
        let px = scan_chunk(&level(|_, _| vec![1]), &t, &map);
        assert_ne!(px[3], UNSET);
        assert_eq!(px[4], UNSET);
    }

    #[test]
    fn ceiling_levels_render_below_the_roof() {
        let (t, map) = tables();
        let mut l = level(|_, _| vec![1, 3, 0, 0, 1, 1]);
        l.ceiling = true;
        let px = scan_chunk(&l, &t, &map);
        assert_eq!(px[0], color::shade_step(0x91BD59, 1));
    }

    #[test]
    fn the_games_map_colour_reaches_the_pixel() {
        let w = Config::parse(DEFAULT_TOML).unwrap().world("minecraft:overworld");
        let raw = [
            RawState { name: "minecraft:air".into(), ..Default::default() },
            RawState { name: "minecraft:stone".into(), map_color: 11, map_rgb: Some(0x112233), ..Default::default() },
        ];
        let t = Tables::build(&raw, &[RawBiome::default()], &w);
        let px = scan_chunk(&level(|_, _| vec![1, 1]), &t, &w.map);
        assert_eq!(px[0], color::shade_step(0x112233, 1));
    }

    #[test]
    fn swamp_grid_covers_the_neighbours_and_clamps_past_them() {
        let mut asked = 0;
        let g = SwampGrid::take(2, -1, |x, z| {
            asked += 1;
            if (x + z) % 2 == 0 { 0x4C763C } else { 0x6A7039 }
        });
        assert_eq!(asked, SwampGrid::SIDE * SwampGrid::SIDE);
        // Chunk 2,-1: its neighbours span x 16..64, z -32..16.
        assert_eq!(g.get(16, -32), 0x4C763C);
        assert_eq!(g.get(17, -32), 0x6A7039);
        assert_eq!(g.get(63, 15), 0x4C763C);
        // Past the edge: the nearest column.
        assert_eq!(g.get(-100, -32), g.get(16, -32));
        assert_eq!(g.get(1000, 1000), g.get(63, 15));
    }
}
