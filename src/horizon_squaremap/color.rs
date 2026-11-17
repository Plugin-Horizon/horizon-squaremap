//! ARGB colour helpers, ported from squaremap's `Colors` and `ColorBlender`
//! (MIT, Copyright (c) 2021-2024 Jason Penilla, 2020-2021 William Blake Galbreath).
//! The arithmetic keeps Java's `float`/`int` steps so pixels match squaremap's.

/// `MapColor.PLANT.col`.
pub const PLANT: u32 = 0x007C00;

/// `MapColor.col` by map colour id (`MapColor.MATERIAL_COLORS`).
pub const MAP_COLORS: [u32; 62] = [
    0, 8368696, 16247203, 13092807, 16711680, 10526975, 10987431, 31744, 16777215, 10791096,
    9923917, 7368816, 4210943, 9402184, 16776437, 14188339, 11685080, 6724056, 15066419, 8375321,
    15892389, 5000268, 10066329, 5013401, 8339378, 3361970, 6704179, 6717235, 10040115, 1644825,
    16445005, 6085589, 4882687, 55610, 8476209, 7340544, 13742497, 10441252, 9787244, 7367818,
    12223780, 6780213, 10505550, 3746083, 8874850, 5725276, 8014168, 4996700, 4993571, 5001770,
    9321518, 2430480, 12398641, 9715553, 6035741, 1474182, 3837580, 5647422, 1356933, 6579300,
    14200723, 8365974,
];

/// The map colour id of water (`MapColor.WATER`).
pub const WATER_MAP_COLOR: u8 = 12;

pub fn map_color(id: u8) -> u32 {
    MAP_COLORS.get(usize::from(id)).copied().unwrap_or(0)
}

pub fn remove_alpha(color: u32) -> u32 {
    0xFF00_0000 | (color & 0x00FF_FFFF)
}

/// `Colors.shade(color, 0|1|2)`: the map's three brightness steps.
pub fn shade_step(color: u32, step: u8) -> u32 {
    let ratio = match step {
        0 => 180.0f32 / 255.0,
        1 => 220.0f32 / 255.0,
        _ => 1.0f32,
    };
    shade(color, ratio)
}

pub fn shade(color: u32, ratio: f32) -> u32 {
    let ch = |s: u32| (((color >> s & 0xFF) as f32 * ratio) as i32 as u32) & 0xFF;
    0xFF00_0000 | ch(16) << 16 | ch(8) << 8 | ch(0)
}

/// `Colors.mix`: `c2` with weight `ratio` over `c1`.
pub fn mix(c1: u32, c2: u32, ratio: f32) -> u32 {
    if ratio >= 1.0 {
        return c2;
    } else if ratio <= 0.0 {
        return c1;
    }
    let i_ratio = 1.0f32 - ratio;
    let ch = |s: u32| {
        let a = (c1 >> s & 0xFF) as f32;
        let b = (c2 >> s & 0xFF) as f32;
        ((a * i_ratio + b * ratio) as i32 as u32) & 0xFF
    };
    0xFF00_0000 | ch(16) << 16 | ch(8) << 8 | ch(0)
}

/// `Colors.parseHex`: `#RRGGBB` (no alpha) or `#RRGGBBAA`.
pub fn parse_hex(text: &str) -> Option<u32> {
    let hex = text.trim().trim_start_matches('#');
    let v = u32::from_str_radix(hex, 16).ok()?;
    match hex.len() {
        8 => Some((v & 0xFF) << 24 | v >> 8),
        6 => Some(v),
        _ => None,
    }
}

/// `ColorBlender`: the channel-wise integer mean of the colours added.
#[derive(Default)]
pub struct Blender {
    sums: [u32; 4],
    count: u32,
}

impl Blender {
    pub fn add(&mut self, color: u32) {
        for (i, s) in [24, 16, 8, 0].into_iter().enumerate() {
            self.sums[i] += color >> s & 0xFF;
        }
        self.count += 1;
    }
    pub fn result(&self) -> u32 {
        let c = self.count.max(1);
        let [a, r, g, b] = self.sums.map(|s| s / c);
        a << 24 | r << 16 | g << 8 | b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shade_matches_java_float_steps() {
        // (int) (0x7F * (180F / 255F)) = 89 in Java.
        assert_eq!(shade_step(0x7F7F7F, 0), 0xFF59_5959);
        assert_eq!(shade_step(0x123456, 2), 0xFF12_3456);
    }

    #[test]
    fn mix_and_hex() {
        assert_eq!(mix(0x000000, 0xFFFFFF, 0.5), 0xFF7F_7F7F);
        assert_eq!(mix(1, 2, 1.0), 2);
        assert_eq!(parse_hex("#9B221A"), Some(0x9B221A));
        assert_eq!(parse_hex("#11223344"), Some(0x4411_2233));
    }

    #[test]
    fn blender_is_integer_mean() {
        let mut b = Blender::default();
        b.add(0x00_0A_0A_0A);
        b.add(0x00_0B_0B_0B);
        assert_eq!(b.result(), 0x00_0A_0A_0A);
    }
}
