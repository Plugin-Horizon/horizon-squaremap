//! What the game knows that region files don't, computed here: `BiomeManager`'s zoom (the biome a
//! block shows is the closest of eight jittered noise cells, seeded by the SHA-256 of the world
//! seed) and the swamp's grass noise (`Biome.BIOME_INFO_NOISE`, a one-octave simplex noise seeded
//! with 2345). Both follow Minecraft 26.3.

/// `BiomeManager.obfuscateSeed`: the first eight bytes of SHA-256 over the little-endian seed,
/// read as a little-endian signed long.
pub fn obfuscate_seed(seed: i64) -> i64 {
    let digest = sha256(&seed.to_le_bytes());
    i64::from_le_bytes(digest[..8].try_into().unwrap())
}

/// SHA-256 of `data` (FIPS 180-4), only for the seed hash above.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
        0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
        0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
        0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
        0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
        0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] =
        [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in message.chunks(64) {
        let mut w = [0u32; 64];
        for (i, word) in block.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes(word.try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, v) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(v);
        }
    }
    let mut out = [0u8; 32];
    for (i, v) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}

/// `BiomeManager.getBiome`'s zoom: the noise (quart) cell whose biome the block at (x, y, z)
/// shows, from the obfuscated seed.
pub fn quart_at_block(biome_seed: i64, x: i32, y: i32, z: i32) -> [i32; 3] {
    const A: i64 = 6364136223846793005;
    const B: i64 = 1442695040888963407;
    // `LinearCongruentialGenerator.next`
    let next = |seed: i64, salt: i64| seed.wrapping_mul(seed.wrapping_mul(A).wrapping_add(B)).wrapping_add(salt);
    let fiddle = |l: i64| (((l >> 24) & 1023) as f64 / 1024.0 - 0.5) * 0.9;
    let shifted = [x.wrapping_sub(2), y.wrapping_sub(2), z.wrapping_sub(2)];
    let base = shifted.map(|v| v >> 2);
    let frac = shifted.map(|v| f64::from(v & 3) / 4.0);
    let mut best = f64::INFINITY;
    let mut chosen = 0usize;
    for corner in 0..8usize {
        let off = [(corner >> 2) & 1, (corner >> 1) & 1, corner & 1];
        let c: [i64; 3] = std::array::from_fn(|i| i64::from(base[i]) + off[i] as i64);
        let mut l = next(biome_seed, c[0]);
        l = next(l, c[1]);
        l = next(l, c[2]);
        l = next(l, c[0]);
        l = next(l, c[1]);
        l = next(l, c[2]);
        let dx = fiddle(l);
        l = next(l, biome_seed);
        let dy = fiddle(l);
        l = next(l, biome_seed);
        let dz = fiddle(l);
        let ex = frac[0] - off[0] as f64 + dx;
        let ey = frac[1] - off[1] as f64 + dy;
        let ez = frac[2] - off[2] as f64 + dz;
        let d = ez * ez + ey * ey + ex * ex;
        if best > d {
            best = d;
            chosen = corner;
        }
    }
    [
        base[0].wrapping_add(((chosen >> 2) & 1) as i32),
        base[1].wrapping_add(((chosen >> 1) & 1) as i32),
        base[2].wrapping_add((chosen & 1) as i32),
    ]
}

/// `java.util.Random` / `LegacyRandomSource`.
struct LegacyRandom(i64);

impl LegacyRandom {
    fn new(seed: i64) -> Self {
        Self((seed ^ 0x5DEECE66D) & ((1i64 << 48) - 1))
    }
    fn next(&mut self, bits: u32) -> i32 {
        self.0 = (self.0.wrapping_mul(0x5DEECE66D).wrapping_add(0xB)) & ((1i64 << 48) - 1);
        (self.0 >> (48 - bits)) as i32
    }
    fn next_int(&mut self, bound: i32) -> i32 {
        if bound & (bound - 1) == 0 {
            return ((i64::from(bound) * i64::from(self.next(31))) >> 31) as i32;
        }
        loop {
            let bits = self.next(31);
            let val = bits % bound;
            if bits.wrapping_sub(val).wrapping_add(bound - 1) >= 0 {
                return val;
            }
        }
    }
    fn next_double(&mut self) -> f64 {
        let hi = i64::from(self.next(26)) << 27;
        (hi + i64::from(self.next(27))) as f64 * 1.1102230246251565E-16
    }
}

const GRADIENTS: [[i32; 3]; 12] =
    [[1, 1, 0], [-1, 1, 0], [1, -1, 0], [-1, -1, 0], [1, 0, 1], [-1, 0, 1], [1, 0, -1], [-1, 0, -1], [0, 1, 1], [0, -1, 1], [0, 1, -1], [0, -1, -1]];

/// `SimplexNoise` (2D, no offsets: `PerlinSimplexNoise.getValue(x, y, false)` of one octave).
pub struct SimplexNoise {
    p: [u8; 256],
}

impl SimplexNoise {
    pub fn new(seed: i64) -> Self {
        let mut random = LegacyRandom::new(seed);
        // The three offsets are drawn (and unused with `useNoiseOffsets` false).
        for _ in 0..3 {
            random.next_double();
        }
        let mut p: [u8; 256] = std::array::from_fn(|i| i as u8);
        for i in 0..256 {
            let j = random.next_int(256 - i as i32) as usize;
            p.swap(i, i + j);
        }
        Self { p }
    }

    fn perm(&self, i: i32) -> i32 {
        i32::from(self.p[(i & 255) as usize])
    }

    fn corner(g: usize, x: f64, y: f64) -> f64 {
        let t = 0.5 - x * x - y * y;
        if t < 0.0 {
            0.0
        } else {
            let t = t * t;
            t * t * (f64::from(GRADIENTS[g][0]) * x + f64::from(GRADIENTS[g][1]) * y)
        }
    }

    pub fn get2(&self, x: f64, y: f64) -> f64 {
        let root3 = 3.0_f64.sqrt();
        let skew = 0.5 * (root3 - 1.0);
        let unskew = (3.0 - root3) / 6.0;
        let s = (x + y) * skew;
        let i = (x + s).floor() as i32;
        let j = (y + s).floor() as i32;
        let t = f64::from(i.wrapping_add(j)) * unskew;
        let dx = x - (f64::from(i) - t);
        let dy = y - (f64::from(j) - t);
        let (a, b) = if dx > dy { (1, 0) } else { (0, 1) };
        let (i, j) = (i & 255, j & 255);
        let grad = |da: i32, db: i32| (self.perm(i + da + self.perm(j + db)) % 12) as usize;
        let n0 = Self::corner(grad(0, 0), dx, dy);
        let n1 = Self::corner(grad(a, b), dx - f64::from(a) + unskew, dy - f64::from(b) + unskew);
        let n2 = Self::corner(grad(1, 1), dx - 1.0 + 2.0 * unskew, dy - 1.0 + 2.0 * unskew);
        70.0 * (n0 + n1 + n2)
    }
}

/// `GrassColorModifier.SWAMP`: `BIOME_INFO_NOISE` below -0.1 gives the dark green.
pub struct SwampNoise(SimplexNoise);

impl SwampNoise {
    pub fn new() -> Self {
        Self(SimplexNoise::new(2345))
    }

    pub fn grass(&self, x: i32, z: i32) -> u32 {
        let d = self.0.get2(f64::from(x) * 0.0225, f64::from(z) * 0.0225);
        if d < -0.1 { 5011004 } else { 6975545 }
    }
}

impl Default for SwampNoise {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn sha256_matches_the_fips_vectors() {
        assert_eq!(hex(&sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(hex(&sha256(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    }

    #[test]
    fn obfuscated_seed_is_the_first_eight_digest_bytes_little_endian() {
        let digest = sha256(&12345i64.to_le_bytes());
        assert_eq!(obfuscate_seed(12345), i64::from_le_bytes(digest[..8].try_into().unwrap()));
    }

    #[test]
    fn java_random_matches_known_values() {
        // new Random(0).nextInt() == -1155484576
        assert_eq!(LegacyRandom::new(0).next(32), -1155484576);
    }

    #[test]
    fn zoom_stays_within_one_cell_of_the_plain_quart() {
        let seed = obfuscate_seed(-4172144997902289642);
        for (x, y, z) in [(0, 64, 0), (-17, 5, 33), (1000, -60, -1000), (3, 0, 3)] {
            let q = quart_at_block(seed, x, y, z);
            let plain = [(x - 2) >> 2, (y - 2) >> 2, (z - 2) >> 2];
            for k in 0..3 {
                assert!(q[k] == plain[k] || q[k] == plain[k] + 1, "{q:?} vs {plain:?}");
            }
        }
        // Deterministic.
        assert_eq!(quart_at_block(seed, 7, 7, 7), quart_at_block(seed, 7, 7, 7));
    }

    #[test]
    fn swamp_grass_is_one_of_the_two_vanilla_colours_and_varies() {
        let n = SwampNoise::new();
        let mut seen = std::collections::HashSet::new();
        for x in (-400..400).step_by(7) {
            for z in (-400..400).step_by(7) {
                let c = n.grass(x, z);
                assert!(c == 5011004 || c == 6975545);
                seen.insert(c);
            }
        }
        assert_eq!(seen.len(), 2, "the noise crosses -0.1 somewhere in 800x800 blocks");
    }
}
