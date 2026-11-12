//! Chunks read straight from a world's region files (Anvil `.mca`), for rendering what is
//! saved but not loaded: squaremap's full and radius renders read unloaded chunks from disk
//! the same way (`ChunkMap.read`). Only what the scanner needs is decoded: block states,
//! biomes and the `WORLD_SURFACE` heightmap.
use crate::nbt::{self, Tag};
use crate::scan::ChunkView;
use crate::tables::RawState;
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A level's height range and whether it has a ceiling (the nether).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shape {
    pub min_y: i32,
    /// One above the highest block Y.
    pub top_y: i32,
    pub ceiling: bool,
}

impl Shape {
    /// Vanilla's dimension types, for a level that hasn't ticked yet.
    pub fn vanilla(world: &str) -> Shape {
        match world {
            "minecraft:the_nether" => Shape { min_y: 0, top_y: 256, ceiling: true },
            "minecraft:the_end" => Shape { min_y: 0, top_y: 256, ceiling: false },
            _ => Shape { min_y: -64, top_y: 320, ceiling: false },
        }
    }
}

/// What the game knows that region files don't: the biome zoom (`BiomeManager`, seeded per
/// world) and the swamp's grass noise.
pub struct WorldEnv {
    pub zoom: Box<dyn Fn(i32, i32, i32) -> [i32; 3] + Send + Sync>,
    pub swamp_grass: Box<dyn Fn(u16, i32, i32) -> u32 + Send + Sync>,
}

impl WorldEnv {
    /// Vanilla's zoom with the seed's hash unknown: the quart cell a block lies in, unfuzzed.
    /// Only for tests and for a game that offers no zoom.
    pub fn plain() -> WorldEnv {
        WorldEnv {
            zoom: Box::new(|x, y, z| [x >> 2, y >> 2, z >> 2]),
            swamp_grass: Box::new(|_, _, _| 0x6A7039),
        }
    }
}

/// The region folder of a level: `dimensions/<namespace>/<path>/region` (26.x), else the
/// older `region`, `DIM-1/region`, `DIM1/region`.
pub fn region_dir(world_dir: &Path, world: &str) -> PathBuf {
    let (ns, path) = world.split_once(':').unwrap_or(("minecraft", world));
    let modern = world_dir.join("dimensions").join(ns).join(path).join("region");
    if modern.is_dir() {
        return modern;
    }
    match world {
        "minecraft:overworld" => world_dir.join("region"),
        "minecraft:the_nether" => world_dir.join("DIM-1").join("region"),
        "minecraft:the_end" => world_dir.join("DIM1").join("region"),
        _ => modern,
    }
}

/// The regions a folder has files for (`r.<x>.<z>.mca`, not empty), as squaremap lists them.
pub fn list_regions(dir: &Path) -> Vec<(i32, i32)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(coords) = name.strip_prefix("r.").and_then(|n| n.strip_suffix(".mca")) else { continue };
        let Some((x, z)) = coords.split_once('.') else { continue };
        let (Ok(x), Ok(z)) = (x.parse(), z.parse()) else { continue };
        if e.metadata().is_ok_and(|m| m.len() > 0) {
            out.push((x, z));
        }
    }
    out
}

/// An open region file and its location table.
pub struct RegionFile {
    dir: PathBuf,
    file: File,
    offsets: Box<[u32; 1024]>,
}

impl RegionFile {
    /// None when the file is missing or shorter than its header.
    pub fn open(dir: &Path, rx: i32, rz: i32) -> io::Result<Option<RegionFile>> {
        let path = dir.join(format!("r.{rx}.{rz}.mca"));
        let mut file = match File::open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let mut header = vec![0u8; 4096];
        if file.read_exact(&mut header).is_err() {
            return Ok(None);
        }
        let mut offsets = Box::new([0u32; 1024]);
        for (i, o) in offsets.iter_mut().enumerate() {
            *o = u32::from_be_bytes(header[i * 4..i * 4 + 4].try_into().unwrap());
        }
        Ok(Some(RegionFile { dir: dir.to_path_buf(), file, offsets }))
    }

    fn index(cx: i32, cz: i32) -> usize {
        ((cz & 31) * 32 + (cx & 31)) as usize
    }

    pub fn has(&self, cx: i32, cz: i32) -> bool {
        self.offsets[Self::index(cx, cz)] != 0
    }

    /// The chunk's NBT, decompressed; None where the region holds no chunk there.
    pub fn read(&mut self, cx: i32, cz: i32) -> Result<Option<Vec<u8>>, String> {
        let entry = self.offsets[Self::index(cx, cz)];
        if entry == 0 {
            return Ok(None);
        }
        let (sector, count) = (u64::from(entry >> 8), u64::from(entry & 0xFF));
        if sector < 2 || count == 0 {
            return Err(format!("bad location {sector}/{count}"));
        }
        self.file.seek(SeekFrom::Start(sector * 4096)).map_err(|e| e.to_string())?;
        let mut head = [0u8; 5];
        self.file.read_exact(&mut head).map_err(|e| e.to_string())?;
        let length = u32::from_be_bytes(head[..4].try_into().unwrap()) as u64;
        let kind = head[4];
        if length == 0 || length > count * 4096 {
            return Err(format!("bad chunk length {length}"));
        }
        let data = if kind & 0x80 != 0 {
            // Stored in its own file (a chunk over 1 MiB).
            std::fs::read(self.dir.join(format!("c.{cx}.{cz}.mcc"))).map_err(|e| e.to_string())?
        } else {
            let mut data = vec![0u8; (length - 1) as usize];
            self.file.read_exact(&mut data).map_err(|e| e.to_string())?;
            data
        };
        decompress(kind & 0x7F, &data).map(Some)
    }
}

/// `RegionFileVersion`: 1 gzip, 2 zlib, 3 none. LZ4 (4) and custom (127) are not read.
pub fn decompress(kind: u8, data: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    match kind {
        1 => flate2::read::GzDecoder::new(data).read_to_end(&mut out).map_err(|e| e.to_string())?,
        2 => flate2::read::ZlibDecoder::new(data).read_to_end(&mut out).map_err(|e| e.to_string())?,
        3 => return Ok(data.to_vec()),
        k => return Err(format!("chunk compression {k} is not supported")),
    };
    Ok(out)
}

/// Block state ids by `name[key=value,...]` (keys sorted), and biome ids by name, from
/// the tables the game handed over.
pub struct Ids {
    states: HashMap<String, u32>,
    /// The first state of each block, for a saved state whose properties don't match.
    blocks: HashMap<String, u32>,
    air: Vec<bool>,
    biomes: HashMap<String, u16>,
}

fn state_key(name: &str, props: &mut [(String, String)]) -> String {
    props.sort();
    if props.is_empty() {
        return name.to_owned();
    }
    let inner: Vec<String> = props.iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!("{name}[{}]", inner.join(","))
}

impl Ids {
    pub fn new(states: &[RawState], biomes: &[String]) -> Ids {
        let mut by_key = HashMap::with_capacity(states.len());
        let mut blocks = HashMap::new();
        let mut air = Vec::with_capacity(states.len());
        for (id, s) in states.iter().enumerate() {
            let mut props = s.properties.clone();
            by_key.entry(state_key(&s.name, &mut props)).or_insert(id as u32);
            blocks.entry(s.name.clone()).or_insert(id as u32);
            air.push(matches!(s.name.as_str(), "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"));
        }
        let biomes = biomes.iter().enumerate().map(|(i, n)| (n.clone(), i as u16)).collect();
        Ids { states: by_key, blocks, air, biomes }
    }

    /// A saved palette entry's state id; an unknown block is air ([`u32::MAX`]).
    fn state(&self, entry: &Tag) -> u32 {
        let Some((name, mut props)) = palette_entry(entry) else { return u32::MAX };
        let key = state_key(&name, &mut props);
        self.states.get(&key).or_else(|| self.blocks.get(&name)).copied().unwrap_or(u32::MAX)
    }

    fn is_air(&self, id: u32) -> bool {
        self.air.get(id as usize).copied().unwrap_or(true)
    }

    fn biome(&self, name: &str) -> u16 {
        self.biomes.get(name).or_else(|| self.biomes.get("minecraft:plains")).copied().unwrap_or(0)
    }
}

/// NBT's mixed lists: an entry wrapped as `{"": value}` because its neighbours have
/// another type.
fn unwrap_mixed(tag: &Tag) -> &Tag {
    match tag {
        Tag::Compound(m) if m.len() == 1 => m.get("").unwrap_or(tag),
        _ => tag,
    }
}

/// A block state as a palette saves it: 26.x writes `"minecraft:stone"` for a block
/// without properties and `{id, properties}` otherwise (in a mixed list, see
/// [`unwrap_mixed`]); older versions `{Name, Properties}`.
pub fn palette_entry(entry: &Tag) -> Option<(String, Vec<(String, String)>)> {
    let entry = unwrap_mixed(entry);
    if let Some(name) = entry.as_str() {
        return Some((name.to_owned(), Vec::new()));
    }
    let name = entry.get("id").or_else(|| entry.get("Name")).and_then(Tag::as_str)?;
    let props = match entry.get("properties").or_else(|| entry.get("Properties")) {
        Some(Tag::Compound(m)) => m.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned()))).collect(),
        _ => Vec::new(),
    };
    Some((name.to_owned(), props))
}

/// A paletted container as saved: `bits` per entry, entries never span two longs.
#[derive(Debug)]
struct Paletted<T> {
    palette: Vec<T>,
    bits: u32,
    data: Vec<u64>,
}

impl<T: Copy> Paletted<T> {
    fn get(&self, index: usize) -> T {
        if self.bits == 0 || self.palette.len() == 1 {
            return self.palette[0];
        }
        let per_long = (64 / self.bits) as usize;
        let word = self.data[index / per_long];
        let shift = (index % per_long) as u32 * self.bits;
        let i = ((word >> shift) & ((1u64 << self.bits) - 1)) as usize;
        self.palette.get(i).copied().unwrap_or(self.palette[0])
    }

    /// `min_bits` is 4 for block states, 0 for biomes; a data array of another length tells
    /// the bits it was written with.
    fn new(palette: Vec<T>, data: Option<&[i64]>, entries: usize, min_bits: u32) -> Result<Self, String> {
        if palette.is_empty() {
            return Err("empty palette".into());
        }
        let data: Vec<u64> = data.unwrap_or(&[]).iter().map(|&v| v as u64).collect();
        if palette.len() == 1 {
            return Ok(Paletted { palette, bits: 0, data });
        }
        let longs = |bits: u32| entries.div_ceil((64 / bits) as usize);
        let mut bits = ceil_log2(palette.len()).max(min_bits).max(1);
        if data.len() != longs(bits) {
            bits = (1..=32).find(|&b| longs(b) == data.len()).ok_or("data does not fit its palette")?;
        }
        Ok(Paletted { palette, bits, data })
    }
}

fn ceil_log2(n: usize) -> u32 {
    usize::BITS - (n.max(1) - 1).leading_zeros()
}

#[derive(Debug)]
struct Section {
    states: Paletted<u32>,
    biomes: Paletted<u16>,
}

/// One saved chunk, as the scanner reads it.
#[derive(Debug)]
pub struct DiskChunk {
    min_section: i32,
    sections: Vec<Option<Section>>,
    /// `WORLD_SURFACE`: the first free Y of each column (`z * 16 + x`).
    surface: Vec<i32>,
}

impl DiskChunk {
    /// None for a chunk that isn't fully generated (squaremap's `ChunkSnapshotEligibility`)
    /// or saved under another position.
    pub fn decode(tag: &Tag, ids: &Ids, shape: Shape, cx: i32, cz: i32) -> Result<Option<DiskChunk>, String> {
        let status = tag.get("Status").and_then(Tag::as_str).unwrap_or("");
        if status != "minecraft:full" && status != "full" {
            return Ok(None);
        }
        let pos = |k: &str| tag.get(k).and_then(Tag::as_int);
        if let (Some(x), Some(z)) = (pos("xPos"), pos("zPos")) {
            if (x, z) != (i64::from(cx), i64::from(cz)) {
                return Ok(None);
            }
        }
        let min_section = shape.min_y >> 4;
        let count = ((shape.top_y - shape.min_y) >> 4).max(0) as usize;
        let mut sections: Vec<Option<Section>> = (0..count).map(|_| None).collect();
        for s in tag.get("sections").map_or(&[][..], Tag::as_list) {
            let Some(y) = s.get("Y").and_then(Tag::as_int) else { continue };
            let Some(slot) = usize::try_from(y - i64::from(min_section)).ok().filter(|&i| i < count) else { continue };
            let Some(bs) = s.get("block_states") else { continue };
            let palette: Vec<u32> = bs.get("palette").map_or(&[][..], Tag::as_list).iter().map(|e| ids.state(e)).collect();
            if palette.is_empty() {
                continue;
            }
            let states = Paletted::new(palette, bs.get("data").and_then(Tag::as_longs), 4096, 4)?;
            let biomes = match s.get("biomes") {
                Some(b) => {
                    let palette: Vec<u16> =
                        b.get("palette").map_or(&[][..], Tag::as_list).iter().filter_map(|t| unwrap_mixed(t).as_str()).map(|n| ids.biome(n)).collect();
                    if palette.is_empty() {
                        Paletted { palette: vec![ids.biome("minecraft:plains")], bits: 0, data: Vec::new() }
                    } else {
                        Paletted::new(palette, b.get("data").and_then(Tag::as_longs), 64, 0)?
                    }
                }
                None => Paletted { palette: vec![ids.biome("minecraft:plains")], bits: 0, data: Vec::new() },
            };
            sections[slot] = Some(Section { states, biomes });
        }
        let mut chunk = DiskChunk { min_section, sections, surface: Vec::new() };
        let height = shape.top_y - shape.min_y;
        chunk.surface = tag
            .get("Heightmaps")
            .and_then(|h| h.get("WORLD_SURFACE"))
            .and_then(Tag::as_longs)
            .and_then(|longs| {
                let bits = ceil_log2(height as usize + 1);
                let h = Paletted { palette: (0..1u32 << bits).collect::<Vec<u32>>(), bits, data: longs.iter().map(|&v| v as u64).collect() };
                (longs.len() == 256usize.div_ceil((64 / bits) as usize))
                    .then(|| (0..256).map(|i| shape.min_y + h.get(i) as i32).collect())
            })
            .unwrap_or_else(|| chunk.compute_surface(ids, shape));
        Ok(Some(chunk))
    }

    /// The heightmap from the blocks (a chunk saved without one).
    fn compute_surface(&self, ids: &Ids, shape: Shape) -> Vec<i32> {
        let mut out = vec![shape.min_y; 256];
        for (i, slot) in out.iter_mut().enumerate() {
            let (x, z) = ((i & 15) as i32, (i >> 4) as i32);
            for y in (shape.min_y..shape.top_y).rev() {
                if !ids.is_air(self.state(x, y, z)) {
                    *slot = y + 1;
                    break;
                }
            }
        }
        out
    }

    fn section(&self, y: i32) -> Option<&Section> {
        let i = usize::try_from((y >> 4) - self.min_section).ok()?;
        self.sections.get(i)?.as_ref()
    }

    pub fn state(&self, x: i32, y: i32, z: i32) -> u32 {
        self.section(y).map_or(u32::MAX, |s| s.states.get((((y & 15) << 8) | ((z & 15) << 4) | (x & 15)) as usize))
    }

    /// The biome of a quart cell; the quart Y is clamped to the level as vanilla clamps it.
    pub fn biome(&self, qx: i32, qy: i32, qz: i32) -> Option<u16> {
        let lo = self.min_section * 4;
        let hi = lo + self.sections.len() as i32 * 4 - 1;
        let qy = qy.clamp(lo, hi.max(lo));
        let s = self.section(qy << 2)?;
        Some(s.biomes.get((((qy & 3) << 4) | ((qz & 3) << 2) | (qx & 3)) as usize))
    }

    pub fn surface(&self, x: i32, z: i32) -> i32 {
        self.surface[((z & 15) * 16 + (x & 15)) as usize]
    }
}

/// Decoded chunks around the ones being drawn, and the open region files they come from.
pub struct DiskCache {
    dir: PathBuf,
    ids: Arc<Ids>,
    shape: Shape,
    chunks: HashMap<(i32, i32), (Option<Arc<DiskChunk>>, u64)>,
    files: HashMap<(i32, i32), (Option<RegionFile>, u64)>,
    clock: u64,
    /// Chunks that failed to read, with the reason (logged by the caller).
    pub errors: Vec<((i32, i32), String)>,
}

const CACHED_CHUNKS: usize = 384;
const CACHED_FILES: usize = 9;

impl DiskCache {
    pub fn new(dir: PathBuf, ids: Arc<Ids>, shape: Shape) -> DiskCache {
        DiskCache { dir, ids, shape, chunks: HashMap::new(), files: HashMap::new(), clock: 0, errors: Vec::new() }
    }

    pub fn shape(&self) -> Shape {
        self.shape
    }

    /// Closes the region files, so their location tables are read again (the server may
    /// have moved chunks since).
    pub fn forget_files(&mut self) {
        self.files.clear();
    }

    /// Whether the region file holds a chunk there (without reading it).
    pub fn exists(&mut self, cx: i32, cz: i32) -> bool {
        self.file(cx >> 5, cz >> 5).is_some_and(|f| f.has(cx, cz))
    }

    fn file(&mut self, rx: i32, rz: i32) -> Option<&mut RegionFile> {
        self.clock += 1;
        let now = self.clock;
        if !self.files.contains_key(&(rx, rz)) {
            if self.files.len() >= CACHED_FILES {
                if let Some(old) = self.files.iter().min_by_key(|(_, (_, t))| *t).map(|(k, _)| *k) {
                    self.files.remove(&old);
                }
            }
            let f = RegionFile::open(&self.dir, rx, rz).unwrap_or(None);
            self.files.insert((rx, rz), (f, now));
        }
        let entry = self.files.get_mut(&(rx, rz))?;
        entry.1 = now;
        entry.0.as_mut()
    }

    /// The decoded chunk; None where nothing (fully generated) is saved.
    pub fn chunk(&mut self, cx: i32, cz: i32) -> Option<Arc<DiskChunk>> {
        self.clock += 1;
        let now = self.clock;
        if let Some(e) = self.chunks.get_mut(&(cx, cz)) {
            e.1 = now;
            return e.0.clone();
        }
        let (ids, shape) = (self.ids.clone(), self.shape);
        let loaded = match self.file(cx >> 5, cz >> 5).map(|f| f.read(cx, cz)) {
            None | Some(Ok(None)) => None,
            Some(Ok(Some(bytes))) => match nbt::read(&bytes).and_then(|t| DiskChunk::decode(&t, &ids, shape, cx, cz)) {
                Ok(c) => c.map(Arc::new),
                Err(e) => {
                    self.errors.push(((cx, cz), e));
                    None
                }
            },
            Some(Err(e)) => {
                self.errors.push(((cx, cz), e));
                None
            }
        };
        if self.chunks.len() >= CACHED_CHUNKS {
            if let Some(old) = self.chunks.iter().min_by_key(|(_, (_, t))| *t).map(|(k, _)| *k) {
                self.chunks.remove(&old);
            }
        }
        self.chunks.insert((cx, cz), (loaded.clone(), now));
        loaded
    }

    /// A chunk and its eight neighbours as a [`ChunkView`]; None when the chunk itself
    /// isn't saved.
    pub fn view(&mut self, cx: i32, cz: i32, env: Arc<WorldEnv>) -> Option<DiskView> {
        let centre = self.chunk(cx, cz)?;
        let chunks = std::array::from_fn(|i| {
            let (dx, dz) = (i as i32 % 3 - 1, i as i32 / 3 - 1);
            if (dx, dz) == (0, 0) { Some(centre.clone()) } else { self.chunk(cx + dx, cz + dz) }
        });
        Some(DiskView { cx, cz, chunks, shape: self.shape, env })
    }

    /// A noise biome from the saved chunks (for a live snapshot whose neighbour is unloaded).
    pub fn noise_biome(&mut self, qx: i32, qy: i32, qz: i32) -> Option<u16> {
        self.chunk(qx >> 2, qz >> 2)?.biome(qx, qy, qz)
    }
}

/// A saved chunk with its neighbours.
pub struct DiskView {
    cx: i32,
    cz: i32,
    chunks: [Option<Arc<DiskChunk>>; 9],
    shape: Shape,
    env: Arc<WorldEnv>,
}

impl DiskView {
    fn at(&self, x: i32, z: i32) -> Option<&DiskChunk> {
        let (dx, dz) = ((x >> 4) - self.cx, (z >> 4) - self.cz);
        if dx.abs() > 1 || dz.abs() > 1 {
            return None;
        }
        self.chunks[((dz + 1) * 3 + dx + 1) as usize].as_deref()
    }
}

impl ChunkView for DiskView {
    fn chunk(&self) -> (i32, i32) {
        (self.cx, self.cz)
    }
    fn min_y(&self) -> i32 {
        self.shape.min_y
    }
    fn top_y(&self) -> i32 {
        self.shape.top_y
    }
    fn has_ceiling(&self) -> bool {
        self.shape.ceiling
    }
    fn surface(&self, x: i32, z: i32) -> Option<i32> {
        self.at(x, z).map(|c| c.surface(x, z))
    }
    fn state(&self, x: i32, y: i32, z: i32) -> u32 {
        match self.at(x, z) {
            Some(c) if (self.shape.min_y..self.shape.top_y).contains(&y) => c.state(x, y, z),
            _ => u32::MAX,
        }
    }
    fn quart_at_block(&self, x: i32, y: i32, z: i32) -> [i32; 3] {
        (self.env.zoom)(x, y, z)
    }
    fn noise_biome(&self, qx: i32, qy: i32, qz: i32) -> Option<u16> {
        self.at(qx << 2, qz << 2).and_then(|c| c.biome(qx, qy, qz))
    }
    fn swamp_grass(&self, biome: u16, x: i32, z: i32) -> u32 {
        (self.env.swamp_grass)(biome, x, z)
    }
}

/// Writers for tests: a chunk's NBT and a region file holding chunks.
#[cfg(test)]
pub mod testdata {
    use crate::nbt::write::{compound, root};
    use crate::nbt::Tag;

    /// Packs `values` with `bits` each, none spanning two longs.
    pub fn pack(values: &[u64], bits: u32) -> Vec<i64> {
        let per = (64 / bits) as usize;
        values
            .chunks(per)
            .map(|c| c.iter().enumerate().fold(0u64, |w, (i, &v)| w | v << (i as u32 * bits)) as i64)
            .collect()
    }

    fn entry(name: &str, props: &[(&str, &str)]) -> Tag {
        let mut e = vec![("Name", Tag::String(name.into()))];
        if !props.is_empty() {
            e.push(("Properties", compound(props.iter().map(|(k, v)| (*k, Tag::String((*v).into()))).collect())));
        }
        compound(e)
    }

    /// A full chunk with one section at `section_y`: `blocks(x, y, z)` indexes into `palette`.
    pub fn chunk(
        cx: i32,
        cz: i32,
        section_y: i8,
        palette: &[(&str, &[(&str, &str)])],
        blocks: impl Fn(i32, i32, i32) -> u64,
        biome: &str,
        heightmap: Option<Vec<i64>>,
    ) -> Vec<u8> {
        let values: Vec<u64> = (0..4096).map(|i| blocks(i & 15, i >> 8, (i >> 4) & 15)).collect();
        let bits = (usize::BITS - (palette.len().max(2) - 1).leading_zeros()).max(4);
        let mut states = vec![("palette", Tag::List(palette.iter().map(|(n, p)| entry(n, p)).collect()))];
        if palette.len() > 1 {
            states.push(("data", Tag::LongArray(pack(&values, bits))));
        }
        let section = compound(vec![
            ("Y", Tag::Byte(section_y)),
            ("block_states", compound(states)),
            ("biomes", compound(vec![("palette", Tag::List(vec![Tag::String(biome.into())]))])),
        ]);
        let mut root_entries = vec![
            ("Status", Tag::String("minecraft:full".into())),
            ("xPos", Tag::Int(cx)),
            ("zPos", Tag::Int(cz)),
            ("sections", Tag::List(vec![section])),
        ];
        if let Some(h) = heightmap {
            root_entries.push(("Heightmaps", compound(vec![("WORLD_SURFACE", Tag::LongArray(h))])));
        }
        root(&compound(root_entries))
    }

    /// A region file with zlib chunks at their positions.
    pub fn region(chunks: &[((i32, i32), Vec<u8>)]) -> Vec<u8> {
        use std::io::Write;
        let mut header = vec![0u8; 8192];
        let mut body = Vec::new();
        for ((cx, cz), nbt) in chunks {
            let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            z.write_all(nbt).unwrap();
            let data = z.finish().unwrap();
            let sector = 2 + body.len() / 4096;
            let mut blob = ((data.len() + 1) as u32).to_be_bytes().to_vec();
            blob.push(2);
            blob.extend_from_slice(&data);
            let sectors = blob.len().div_ceil(4096);
            blob.resize(sectors * 4096, 0);
            body.extend_from_slice(&blob);
            let i = (((cz & 31) * 32 + (cx & 31)) * 4) as usize;
            header[i..i + 4].copy_from_slice(&((sector as u32) << 8 | sectors as u32).to_be_bytes());
        }
        header.extend_from_slice(&body);
        header
    }
}

#[cfg(test)]
mod tests {
    use super::testdata::*;
    use super::*;

    fn raw(name: &str, props: &[(&str, &str)]) -> RawState {
        RawState {
            name: name.into(),
            properties: props.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect(),
            ..Default::default()
        }
    }

    fn ids() -> Ids {
        let states = [
            raw("minecraft:air", &[]),
            raw("minecraft:stone", &[]),
            raw("minecraft:oak_log", &[("axis", "x")]),
            raw("minecraft:oak_log", &[("axis", "y")]),
            raw("minecraft:water", &[("level", "0")]),
        ];
        Ids::new(&states, &["minecraft:plains".into(), "minecraft:desert".into()])
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("test-scratch").join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const OVERWORLD: Shape = Shape { min_y: -64, top_y: 320, ceiling: false };

    #[test]
    fn decodes_palette_properties_biomes_and_computed_heights() {
        let palette: &[(&str, &[(&str, &str)])] =
            &[("minecraft:air", &[]), ("minecraft:stone", &[]), ("minecraft:oak_log", &[("axis", "y")])];
        // Section Y=0 (blocks 0..16): stone up to y=3, a log at x=2 z=5 y=4.
        let bytes = chunk(3, -2, 0, palette, |x, y, z| if y <= 3 { 1 } else if (x, y, z) == (2, 4, 5) { 2 } else { 0 }, "minecraft:desert", None);
        let tag = nbt::read(&bytes).unwrap();
        let c = DiskChunk::decode(&tag, &ids(), OVERWORLD, 3, -2).unwrap().unwrap();
        assert_eq!(c.state(0, 3, 0), 1);
        assert_eq!(c.state(2, 4, 5), 3);
        assert_eq!(c.state(0, 40, 0), u32::MAX); // a section that isn't saved
        assert_eq!(c.surface(0, 0), 4);
        assert_eq!(c.surface(2, 5), 5);
        assert_eq!(c.surface(7, 7), 4);
        assert_eq!(c.biome(12, 0, -8), Some(1));
        assert_eq!(c.biome(12, -500, -8), None); // clamped to the bottom section, not saved
        // Saved under another position: skipped.
        assert!(DiskChunk::decode(&tag, &ids(), OVERWORLD, 0, 0).unwrap().is_none());
    }

    #[test]
    fn a_saved_heightmap_wins_and_unfinished_chunks_are_skipped() {
        // 384 high: 9 bits, 7 per long, 37 longs. Column 0 says 100 -> first free y 36.
        let mut values = vec![70u64; 256];
        values[0] = 100;
        let h = pack(&values, 9);
        assert_eq!(h.len(), 37);
        let bytes = chunk(0, 0, 0, &[("minecraft:stone", &[])], |_, _, _| 0, "minecraft:plains", Some(h));
        let c = DiskChunk::decode(&nbt::read(&bytes).unwrap(), &ids(), OVERWORLD, 0, 0).unwrap().unwrap();
        assert_eq!(c.surface(0, 0), 36);
        assert_eq!(c.surface(1, 0), 6);
        let mut tag = nbt::read(&bytes).unwrap();
        if let Tag::Compound(m) = &mut tag {
            m.insert("Status".into(), Tag::String("minecraft:features".into()));
        }
        assert!(DiskChunk::decode(&tag, &ids(), OVERWORLD, 0, 0).unwrap().is_none());
    }

    #[test]
    fn unknown_properties_fall_back_to_the_block_and_unknown_blocks_to_air() {
        let i = ids();
        let e = |n: &str, p: &[(&str, &str)]| {
            crate::nbt::write::compound(vec![
                ("Name", Tag::String(n.into())),
                ("Properties", crate::nbt::write::compound(p.iter().map(|(k, v)| (*k, Tag::String((*v).into()))).collect())),
            ])
        };
        assert_eq!(i.state(&e("minecraft:oak_log", &[("axis", "x")])), 2);
        assert_eq!(i.state(&e("minecraft:oak_log", &[("axis", "q")])), 2);
        assert_eq!(i.state(&e("minecraft:water", &[("level", "0")])), 4);
        assert_eq!(i.state(&e("modded:thing", &[])), u32::MAX);
        // 26.x: plain names, {id, properties}, and both wrapped in a mixed list.
        let c = crate::nbt::write::compound;
        assert_eq!(i.state(&Tag::String("minecraft:stone".into())), 1);
        let new = c(vec![("id", Tag::String("minecraft:oak_log".into())), ("properties", c(vec![("axis", Tag::String("y".into()))]))]);
        assert_eq!(i.state(&new), 3);
        assert_eq!(i.state(&c(vec![("", new)])), 3);
        assert_eq!(i.state(&c(vec![("", Tag::String("minecraft:stone".into()))])), 1);
    }

    #[test]
    fn reads_chunks_from_a_region_file_and_views_them() {
        let dir = scratch("region");
        let palette: &[(&str, &[(&str, &str)])] = &[("minecraft:air", &[]), ("minecraft:stone", &[])];
        let a = chunk(-1, 0, 0, palette, |_, y, _| u64::from(y <= 5), "minecraft:plains", None);
        let b = chunk(-2, 0, 0, palette, |_, y, _| u64::from(y <= 1), "minecraft:plains", None);
        std::fs::write(dir.join("r.-1.0.mca"), region(&[((-1, 0), a), ((-2, 0), b)])).unwrap();
        std::fs::write(dir.join("r.5.5.mca"), b"").unwrap();
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        assert_eq!(list_regions(&dir), vec![(-1, 0)]);
        let mut cache = DiskCache::new(dir.clone(), Arc::new(ids()), OVERWORLD);
        assert!(cache.exists(-1, 0) && !cache.exists(-3, 0) && !cache.exists(40, 40));
        let v = cache.view(-1, 0, Arc::new(WorldEnv::plain())).unwrap();
        assert_eq!(v.surface(-16, 3), Some(6));
        assert_eq!(v.surface(-17, 3), Some(2)); // the west neighbour
        assert_eq!(v.surface(0, 3), None); // east: not saved
        assert_eq!(v.state(-16, 5, 3), 1);
        assert!(cache.view(-3, 0, Arc::new(WorldEnv::plain())).is_none());
        assert!(cache.errors.is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A real region file (`SQUAREMAP_REGION=<file.mca> cargo test -- --ignored`): every
    /// full chunk decodes, and the heights its blocks give match its saved heightmap.
    #[test]
    #[ignore]
    fn probe_a_real_region_file() {
        let Ok(path) = std::env::var("SQUAREMAP_REGION") else { return };
        let path = PathBuf::from(path);
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        let c: Vec<i32> = name.trim_start_matches("r.").trim_end_matches(".mca").split('.').map(|n| n.parse().unwrap()).collect();
        let mut file = RegionFile::open(path.parent().unwrap(), c[0], c[1]).unwrap().unwrap();
        let shape = Shape::vanilla(&std::env::var("SQUAREMAP_LEVEL").unwrap_or("minecraft:overworld".into()));
        // First pass: every palette entry becomes a state.
        let mut tags = Vec::new();
        let mut states: Vec<RawState> = vec![raw("minecraft:air", &[])];
        let mut seen = std::collections::HashSet::new();
        for i in 0..1024 {
            let (cx, cz) = (c[0] * 32 + (i % 32), c[1] * 32 + (i / 32));
            let Some(bytes) = file.read(cx, cz).unwrap() else { continue };
            let tag = nbt::read(&bytes).unwrap();
            for s in tag.get("sections").map_or(&[][..], Tag::as_list) {
                for e in s.get("block_states").and_then(|b| b.get("palette")).map_or(&[][..], Tag::as_list) {
                    let (name, mut props) = palette_entry(e).expect("a palette entry");
                    if seen.insert(state_key(&name, &mut props)) {
                        states.push(RawState { name, properties: props, ..Default::default() });
                    }
                }
            }
            tags.push((cx, cz, tag));
        }
        let ids = Ids::new(&states, &[]);
        let (mut full, mut columns, mut mismatches) = (0, 0, 0);
        for (cx, cz, tag) in &tags {
            let Some(chunk) = DiskChunk::decode(tag, &ids, shape, *cx, *cz).unwrap() else { continue };
            full += 1;
            let computed = chunk.compute_surface(&ids, shape);
            for (i, &h) in computed.iter().enumerate() {
                columns += 1;
                if chunk.surface[i] != h {
                    mismatches += 1;
                    if mismatches < 5 {
                        eprintln!("chunk {cx},{cz} column {i}: saved {} computed {h}", chunk.surface[i]);
                    }
                }
            }
        }
        eprintln!("{} chunks, {full} full, {} states, {columns} columns, {mismatches} height mismatches", tags.len(), states.len());
        assert!(full > 0);
        assert_eq!(mismatches, 0);
    }

    #[test]
    fn region_dir_prefers_the_26_layout() {
        let dir = scratch("dirs");
        assert_eq!(region_dir(&dir, "minecraft:the_nether"), dir.join("DIM-1/region"));
        std::fs::create_dir_all(dir.join("dimensions/minecraft/the_nether/region")).unwrap();
        assert_eq!(region_dir(&dir, "minecraft:the_nether"), dir.join("dimensions/minecraft/the_nether/region"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
