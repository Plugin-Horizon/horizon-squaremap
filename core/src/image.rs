//! Region images and their tiles: squaremap's `RegionImage` (MIT, Copyright (c) 2021-2024
//! Jason Penilla). A region is 512x512 blocks; zoom level `max` holds it at one pixel a
//! block in `<max>/<rx>_<rz>.png`, each lower level halves it into the tile that covers it.
use crate::scan::UNSET;
use std::fs;
use std::io::{self, BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};

pub const SIZE: usize = 512;

/// The deflate level tiles are written with; [`DEFAULT_LEVEL`]: the encoder's default.
static LEVEL: AtomicU8 = AtomicU8::new(DEFAULT_LEVEL);
const DEFAULT_LEVEL: u8 = u8::MAX;

/// `settings.image-quality.compress-images`: ImageIO's PNG writer, which squaremap uses,
/// compresses at deflate level `9 * (1 - quality)` and squaremap passes `1 - value`.
pub fn set_compression(enabled: bool, value: f64) {
    let level = if enabled { (9.0 * value.clamp(0.0, 1.0)) as u8 } else { DEFAULT_LEVEL };
    LEVEL.store(level, Ordering::Relaxed);
}

pub struct RegionImage {
    pub rx: i32,
    pub rz: i32,
    /// ARGB, `z * 512 + x`, [`UNSET`] where nothing was rendered.
    pixels: Vec<u32>,
}

impl RegionImage {
    pub fn new(rx: i32, rz: i32) -> Self {
        Self { rx, rz, pixels: vec![UNSET; SIZE * SIZE] }
    }

    /// Writes a chunk's pixels (`z << 4 | x`), skipping unset ones.
    pub fn put_chunk(&mut self, cx: i32, cz: i32, px: &[u32; 256]) {
        let (bx, bz) = ((cx & 31) as usize * 16, (cz & 31) as usize * 16);
        for z in 0..16 {
            for x in 0..16 {
                let c = px[z << 4 | x];
                if c != UNSET {
                    self.pixels[(bz + z) * SIZE + bx + x] = c;
                }
            }
        }
    }

    /// The tile files of every zoom level, finest first.
    pub fn tile_paths(&self, dir: &Path, max_zoom: i32) -> Vec<(PathBuf, usize)> {
        (0..=max_zoom.max(0))
            .map(|zoom| {
                let step = 1i32 << zoom;
                let name = format!("{}_{}.png", self.rx.div_euclid(step), self.rz.div_euclid(step));
                (dir.join((max_zoom - zoom).to_string()).join(name), step as usize)
            })
            .collect()
    }

    /// Updates the tiles on disk: each existing tile is read and the rendered pixels
    /// (every `step`-th of the region) laid over it.
    pub fn save(&self, dir: &Path, max_zoom: i32) -> io::Result<()> {
        for (file, step) in self.tile_paths(dir, max_zoom) {
            let size = SIZE / step;
            let mut image = read_tile(&file);
            let base_x = (self.rx * size as i32) as usize & (SIZE - 1);
            let base_z = (self.rz * size as i32) as usize & (SIZE - 1);
            for z in (0..SIZE).step_by(step) {
                for x in (0..SIZE).step_by(step) {
                    let c = self.pixels[z * SIZE + x];
                    if c != UNSET {
                        image[(base_z + z / step) * SIZE + base_x + x / step] = c;
                    }
                }
            }
            write_png(&file, &image)?;
        }
        Ok(())
    }
}

/// A tile's ARGB pixels, or a transparent one when missing or unreadable.
fn read_tile(file: &Path) -> Vec<u32> {
    fs::read(file)
        .ok()
        .and_then(|bytes| decode_rgb(&bytes).ok())
        .filter(|(w, h, _)| *w == SIZE && *h == SIZE)
        .map(|(_, _, px)| px)
        .unwrap_or_else(|| vec![0; SIZE * SIZE])
}

/// Decodes a PNG to ARGB pixels (alpha 255 where it has none).
pub fn decode_rgb(bytes: &[u8]) -> Result<(usize, usize, Vec<u32>), String> {
    let mut decoder = png::Decoder::new(BufReader::new(io::Cursor::new(bytes)));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or("image too large")?];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let data = &buf[..info.buffer_size()];
    let px = match info.color_type {
        png::ColorType::Rgba => data
            .chunks_exact(4)
            .map(|p| u32::from(p[3]) << 24 | u32::from(p[0]) << 16 | u32::from(p[1]) << 8 | u32::from(p[2]))
            .collect(),
        png::ColorType::Rgb => data
            .chunks_exact(3)
            .map(|p| 0xFF00_0000 | u32::from(p[0]) << 16 | u32::from(p[1]) << 8 | u32::from(p[2]))
            .collect(),
        png::ColorType::GrayscaleAlpha => data
            .chunks_exact(2)
            .map(|p| u32::from(p[1]) << 24 | u32::from(p[0]) * 0x01_0101)
            .collect(),
        png::ColorType::Grayscale => data.iter().map(|&g| 0xFF00_0000 | u32::from(g) * 0x01_0101).collect(),
        png::ColorType::Indexed => return Err("unexpanded palette".into()),
    };
    Ok((info.width as usize, info.height as usize, px))
}

/// Writes a 512x512 ARGB tile as an RGBA PNG through a temporary file and a rename.
pub fn write_png(file: &Path, px: &[u32]) -> io::Result<()> {
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = file.with_extension("png.tmp");
    {
        let out = BufWriter::new(fs::File::create(&tmp)?);
        let mut encoder = png::Encoder::new(out, SIZE as u32, SIZE as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        match LEVEL.load(Ordering::Relaxed) {
            DEFAULT_LEVEL => {}
            0 => encoder.set_deflate_compression(png::DeflateCompression::NoCompression),
            n => encoder.set_deflate_compression(png::DeflateCompression::Level(n)),
        }
        let mut writer = encoder.write_header().map_err(io::Error::other)?;
        let mut data = Vec::with_capacity(px.len() * 4);
        for &c in px {
            data.extend_from_slice(&[(c >> 16) as u8, (c >> 8) as u8, c as u8, (c >> 24) as u8]);
        }
        writer.write_image_data(&data).map_err(io::Error::other)?;
        writer.finish().map_err(io::Error::other)?;
    }
    fs::rename(&tmp, file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("test-scratch").join(name);
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn chunks_land_at_their_offset_and_unset_pixels_keep_what_was_there() {
        let mut r = RegionImage::new(-1, 0);
        let mut first = [0xFF00_0001; 256];
        first[0] = UNSET;
        // Chunk -1,0 is the region's column 31, row 0 (cx & 31).
        r.put_chunk(-1, 0, &first);
        assert_eq!(r.pixels[31 * 16], UNSET);
        assert_eq!(r.pixels[31 * 16 + 1], 0xFF00_0001);
        assert_eq!(r.pixels[15 * SIZE + 31 * 16 + 15], 0xFF00_0001);
        let mut second = [UNSET; 256];
        second[1] = 0xFF00_0002;
        r.put_chunk(-1, 0, &second);
        assert_eq!(r.pixels[31 * 16 + 1], 0xFF00_0002);
        assert_eq!(r.pixels[31 * 16 + 2], 0xFF00_0001);
        // A chunk of the next region over (cx 31 of region 0) wraps into the same columns.
        let mut other = RegionImage::new(0, 0);
        other.put_chunk(31, 1, &[0xFF00_0003; 256]);
        assert_eq!(other.pixels[16 * SIZE + 31 * 16], 0xFF00_0003);
    }

    #[test]
    fn tile_names_follow_squaremap() {
        let r = RegionImage::new(-1, 2);
        let names: Vec<String> = r
            .tile_paths(Path::new("t"), 3)
            .into_iter()
            .map(|(p, s)| format!("{} {s}", p.display()))
            .collect();
        assert_eq!(names, ["t/3/-1_2.png 1", "t/2/-1_1.png 2", "t/1/-1_0.png 4", "t/0/-1_0.png 8"]);
    }

    #[test]
    fn save_writes_all_zooms_and_keeps_unrendered_pixels() {
        let dir = scratch("zoom");
        let mut r = RegionImage::new(-1, 0);
        let mut px = [UNSET; 256];
        px[0] = 0xFF11_2233;
        px[2] = 0xFF44_5566;
        r.put_chunk(-1, 0, &px); // chunk (31, 0) of region -1: block x = 496
        r.save(&dir, 3).unwrap();
        let full = read_tile(&dir.join("3/-1_0.png"));
        assert_eq!(full[496], 0xFF11_2233);
        assert_eq!(full[498], 0xFF44_5566);
        assert_eq!(full[497], 0);
        // Zoom 2: tile -1_0 holds regions -2 and -1; region -1 is its right half.
        let half = read_tile(&dir.join("2/-1_0.png"));
        assert_eq!(half[256 + 248], 0xFF11_2233);
        assert_eq!(half[256 + 249], 0xFF44_5566);
        // A second region drawn into the same zoom-2 tile keeps the first one's pixels.
        let mut left = RegionImage::new(-2, 0);
        let mut p = [UNSET; 256];
        p[0] = 0xFF77_8899;
        left.put_chunk(-64, 0, &p);
        left.save(&dir, 3).unwrap();
        let half = read_tile(&dir.join("2/-1_0.png"));
        assert_eq!(half[0], 0xFF77_8899);
        assert_eq!(half[256 + 248], 0xFF11_2233);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn compression_changes_the_file_not_the_pixels() {
        let dir = scratch("compress");
        let px: Vec<u32> = (0..SIZE * SIZE).map(|i| 0xFF00_0000 | (i as u32 / 7) & 0xFF_FFFF).collect();
        let size = |level: Option<f64>| {
            set_compression(level.is_some(), level.unwrap_or(0.0));
            let f = dir.join("t.png");
            write_png(&f, &px).unwrap();
            assert_eq!(read_tile(&f), px);
            fs::metadata(&f).unwrap().len()
        };
        let (none, best) = (size(Some(0.0)), size(Some(1.0)));
        size(None);
        assert!(none > best, "{none} <= {best}");
        fs::remove_dir_all(dir).unwrap();
    }
}
