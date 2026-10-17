//! A compact ASTC LDR encoder (4×4, 6×6 and 8×8 blocks) for mobile texture imports.
//!
//! Every block uses one partition, one weight plane and direct endpoints (CEM 8, RGB, or
//! CEM 12, RGBA): the endpoints are the extremes of the block's colours along their
//! principal axis, and each texel's weight is its position between them. Larger blocks
//! use a 5×5 weight grid that the GPU interpolates (ASTC's weight infill).
//! Spec: Khronos Data Format Specification, "ASTC Compressed Texture Image Formats".

/// One ASTC block's settings.
#[derive(Clone, Copy)]
struct Mode {
    /// Weight grid size.
    gw: u32,
    gh: u32,
    /// Weight levels (a power of two here: plain bits).
    weight_levels: u32,
    /// The 11-bit block mode for that grid and range.
    block_mode: u32,
}

fn mode_for(bw: u32, _bh: u32) -> Mode {
    if bw == 4 {
        // 4×4 grid, 8 levels (3 bits): R = 0b111, H = 0, A = 2, B = 0.
        Mode {
            gw: 4,
            gh: 4,
            weight_levels: 8,
            block_mode: 0x053,
        }
    } else {
        // 5×5 grid, 4 levels (2 bits): R = 0b100, H = 0, A = 3, B = 1.
        Mode {
            gw: 5,
            gh: 5,
            weight_levels: 4,
            block_mode: 0x0E2,
        }
    }
}

/// Integer sequence encoding of a range: (trits, quints, bits per value).
fn ise_shape(levels: u32) -> (bool, bool, u32) {
    match levels {
        3 => (true, false, 0),
        5 => (false, true, 0),
        6 => (true, false, 1),
        10 => (false, true, 1),
        12 => (true, false, 2),
        20 => (false, true, 2),
        24 => (true, false, 3),
        40 => (false, true, 3),
        48 => (true, false, 4),
        80 => (false, true, 4),
        96 => (true, false, 5),
        160 => (false, true, 5),
        192 => (true, false, 6),
        n => (false, false, n.trailing_zeros()),
    }
}

/// Bits taken by `count` values of a range.
fn ise_bits(levels: u32, count: u32) -> u32 {
    let (t, q, b) = ise_shape(levels);
    count * b
        + if t {
            (count * 8).div_ceil(5)
        } else if q {
            (count * 7).div_ceil(3)
        } else {
            0
        }
}

/// Colour endpoint ranges, smallest first.
const COLOR_LEVELS: [u32; 21] = [
    2, 3, 4, 5, 6, 8, 10, 12, 16, 20, 24, 32, 40, 48, 64, 80, 96, 128, 160, 192, 256,
];

/// The colour range a decoder picks: the largest whose values fit in `bits`.
fn color_levels(values: u32, bits: u32) -> u32 {
    COLOR_LEVELS
        .iter()
        .rev()
        .copied()
        .find(|&l| ise_bits(l, values) <= bits)
        .unwrap_or(2)
}

/// Trits of an 8-bit trit-block code (spec "Integer Sequence Encoding").
fn decode_trits(t: u32) -> [u32; 5] {
    let bit = |x: u32, i: u32| (x >> i) & 1;
    let (c, t4, t3);
    if (t >> 2) & 7 == 7 {
        c = ((t >> 5) & 7) << 2 | (t & 3);
        t4 = 2;
        t3 = 2;
    } else {
        c = t & 0x1F;
        if (t >> 5) & 3 == 3 {
            t4 = 2;
            t3 = bit(t, 7);
        } else {
            t4 = bit(t, 7);
            t3 = (t >> 5) & 3;
        }
    }
    let (t2, t1, t0);
    if c & 3 == 3 {
        t2 = 2;
        t1 = bit(c, 4);
        t0 = bit(c, 3) << 1 | (bit(c, 2) & !bit(c, 3) & 1);
    } else if (c >> 2) & 3 == 3 {
        t2 = 2;
        t1 = 2;
        t0 = c & 3;
    } else {
        t2 = bit(c, 4);
        t1 = (c >> 2) & 3;
        t0 = bit(c, 1) << 1 | (bit(c, 0) & !bit(c, 1) & 1);
    }
    [t0, t1, t2, t3, t4]
}

/// Quints of a 7-bit quint-block code.
fn decode_quints(q: u32) -> [u32; 3] {
    let bit = |x: u32, i: u32| (x >> i) & 1;
    if (q >> 1) & 3 == 3 && (q >> 5) & 3 == 0 {
        let q2 = bit(q, 0) << 2 | (bit(q, 4) & !bit(q, 0) & 1) << 1 | (bit(q, 3) & !bit(q, 0) & 1);
        return [4, 4, q2];
    }
    let (q2, c);
    if (q >> 1) & 3 == 3 {
        q2 = 4;
        c = ((q >> 3) & 3) << 3 | (!(q >> 5) & 3) << 1 | bit(q, 0);
    } else {
        q2 = (q >> 5) & 3;
        c = q & 0x1F;
    }
    let (q1, q0) = if c & 7 == 5 {
        (4, (c >> 3) & 3)
    } else {
        ((c >> 3) & 3, c & 7)
    };
    [q0, q1, q2]
}

/// Little-endian bit writer over a 128-bit block.
struct Bits(u128);

impl Bits {
    fn put(&mut self, at: &mut u32, value: u32, n: u32) {
        if n > 0 {
            self.0 |= ((value as u128) & ((1u128 << n) - 1)) << *at;
            *at += n;
        }
    }
}

/// Integer-sequence-encodes `values` (each < `levels`) as a bit string of exactly
/// `ise_bits(levels, values.len())` bits, returned as (bits, length).
fn ise_encode(levels: u32, values: &[u32]) -> (u128, u32) {
    let (trits, quints, b) = ise_shape(levels);
    let total = ise_bits(levels, values.len() as u32);
    let mut out = Bits(0);
    let mut at = 0;
    let low = |v: u32| v & ((1 << b) - 1);
    if trits {
        for chunk in values.chunks(5) {
            let want: Vec<u32> = chunk.iter().map(|v| v >> b).collect();
            // The trit-block code whose trits match (unused trailing ones are free; the
            // truncated bits of a partial block must be zero).
            let start = at;
            let fits = |code: u32| {
                let t = decode_trits(code);
                want.iter().enumerate().all(|(i, &w)| t[i] == w)
            };
            let used = [2, 2, 1, 2, 1];
            let code_bits = |n: usize| used[..n].iter().sum::<u32>();
            let keep = code_bits(chunk.len());
            let code = (0..256u32)
                .find(|&c| fits(c) && c >> keep == 0)
                .or_else(|| (0..256).find(|&c| fits(c)))
                .expect("trit code");
            let mut shift = 0;
            for (i, &v) in chunk.iter().enumerate() {
                out.put(&mut at, low(v), b);
                out.put(&mut at, code >> shift, used[i]);
                shift += used[i];
            }
            let _ = start;
        }
    } else if quints {
        for chunk in values.chunks(3) {
            let want: Vec<u32> = chunk.iter().map(|v| v >> b).collect();
            let used = [3, 2, 2];
            let keep: u32 = used[..chunk.len()].iter().sum();
            let fits = |code: u32| {
                let q = decode_quints(code);
                want.iter().enumerate().all(|(i, &w)| q[i] == w)
            };
            let code = (0..128u32)
                .find(|&c| fits(c) && c >> keep == 0)
                .or_else(|| (0..128).find(|&c| fits(c)))
                .expect("quint code");
            let mut shift = 0;
            for (i, &v) in chunk.iter().enumerate() {
                out.put(&mut at, low(v), b);
                out.put(&mut at, code >> shift, used[i]);
                shift += used[i];
            }
        }
    } else {
        for &v in values {
            out.put(&mut at, v, b);
        }
    }
    // Partial blocks wrote the bits a decoder doesn't read as well; drop them.
    let mask = if total >= 128 {
        u128::MAX
    } else {
        (1u128 << total) - 1
    };
    (out.0 & mask, total)
}

/// The 8-bit value a colour endpoint code decodes to (spec "Color Endpoint
/// Unquantization").
fn unquantize_color(levels: u32, v: u32) -> u32 {
    let (trits, quints, b) = ise_shape(levels);
    if b == 0 && (trits || quints) {
        // Ranges 3 and 5: evenly spread.
        return v * 255 / (levels - 1);
    }
    if !trits && !quints {
        // Bit replication to 8 bits.
        let mut out = 0;
        let mut have = 0;
        while have < 8 {
            out = (out << b) | v;
            have += b;
        }
        return out >> (have - 8);
    }
    let d = v >> b;
    let m = v & ((1 << b) - 1);
    let bit = |i: u32| (m >> i) & 1;
    let a = if bit(0) == 1 { 0x1FF } else { 0 };
    let (bb, c) = if trits {
        match b {
            1 => (0, 204),
            2 => {
                let x = bit(1);
                (x << 8 | x << 4 | x << 2 | x << 1, 93)
            }
            3 => {
                let (x, y) = (bit(1), bit(2));
                (y << 8 | x << 7 | y << 3 | x << 2 | y << 1 | x, 44)
            }
            4 => {
                let (x, y, z) = (bit(1), bit(2), bit(3));
                (z << 8 | y << 7 | x << 6 | z << 2 | y << 1 | x, 22)
            }
            5 => {
                let (x, y, z, w) = (bit(1), bit(2), bit(3), bit(4));
                (w << 8 | z << 7 | y << 6 | x << 5 | w << 1 | z, 11)
            }
            _ => {
                let (x, y, z, w, u) = (bit(1), bit(2), bit(3), bit(4), bit(5));
                (u << 8 | w << 7 | z << 6 | y << 5 | x << 4 | u, 5)
            }
        }
    } else {
        match b {
            1 => (0, 113),
            2 => {
                let x = bit(1);
                (x << 8 | x << 3 | x << 2, 54)
            }
            3 => {
                let (x, y) = (bit(1), bit(2));
                (y << 8 | x << 7 | y << 2 | x << 1 | y, 26)
            }
            4 => {
                let (x, y, z) = (bit(1), bit(2), bit(3));
                (z << 8 | y << 7 | x << 6 | z << 1 | y, 13)
            }
            _ => {
                let (x, y, z, w) = (bit(1), bit(2), bit(3), bit(4));
                (w << 8 | z << 7 | y << 6 | x << 5 | w, 6)
            }
        }
    };
    let t = (d * c + bb) ^ a;
    (a & 0x80) | (t >> 2)
}

/// A weight code's value in 0..=64 (plain-bit ranges only).
fn unquantize_weight(levels: u32, v: u32) -> u32 {
    let b = levels.trailing_zeros();
    let mut out = 0;
    let mut have = 0;
    while have < 6 {
        out = (out << b) | v;
        have += b;
    }
    let w = out >> (have - 6);
    if w > 32 { w + 1 } else { w }
}

/// For each texel, the four grid weights it reads and their factors (spec "Weight
/// Infill").
fn infill(bw: u32, bh: u32, gw: u32, gh: u32) -> Vec<[(usize, u32); 4]> {
    let ds = (1024 + bw / 2) / (bw - 1);
    let dt = (1024 + bh / 2) / (bh - 1);
    let mut out = Vec::with_capacity((bw * bh) as usize);
    for t in 0..bh {
        for s in 0..bw {
            let gs = (ds * s * (gw - 1) + 32) >> 6;
            let gt = (dt * t * (gh - 1) + 32) >> 6;
            let (js, fs, jt, ft) = (gs >> 4, gs & 15, gt >> 4, gt & 15);
            let v0 = (js + jt * gw) as usize;
            let w11 = (fs * ft + 8) >> 4;
            let w10 = ft - w11;
            let w01 = fs - w11;
            let w00 = 16 + w11 - fs - ft;
            let gw = gw as usize;
            out.push([(v0, w00), (v0 + 1, w01), (v0 + gw, w10), (v0 + gw + 1, w11)]);
        }
    }
    out
}

/// Encodes one block of `bw`×`bh` RGBA8 texels (row-major).
pub fn encode_block(texels: &[[u8; 4]], bw: u32, bh: u32) -> [u8; 16] {
    let mode = mode_for(bw, bh);
    let alpha = texels.iter().any(|p| p[3] < 255);
    let channels = if alpha { 4 } else { 3 };
    let values = 2 * channels as u32;
    let weight_count = mode.gw * mode.gh;
    let weight_bits = ise_bits(mode.weight_levels, weight_count);
    let levels = color_levels(values, 128 - 17 - weight_bits);

    // Principal axis of the colours (power iteration), then the extremes along it.
    let px: Vec<[f32; 4]> = texels
        .iter()
        .map(|p| [p[0] as f32, p[1] as f32, p[2] as f32, p[3] as f32])
        .collect();
    let n = px.len() as f32;
    let mut mean = [0f32; 4];
    for p in &px {
        for c in 0..4 {
            mean[c] += p[c] / n;
        }
    }
    let mut cov = [[0f32; 4]; 4];
    for p in &px {
        for i in 0..channels {
            for j in 0..channels {
                cov[i][j] += (p[i] - mean[i]) * (p[j] - mean[j]);
            }
        }
    }
    let mut axis = [1f32, 1.0, 1.0, if alpha { 1.0 } else { 0.0 }];
    for _ in 0..8 {
        let mut next = [0f32; 4];
        for i in 0..channels {
            for j in 0..channels {
                next[i] += cov[i][j] * axis[j];
            }
        }
        let len = next.iter().map(|v| v * v).sum::<f32>().sqrt();
        if len < 1e-6 {
            break;
        }
        axis = next.map(|v| v / len);
    }
    let proj = |p: &[f32; 4]| {
        (0..channels)
            .map(|c| (p[c] - mean[c]) * axis[c])
            .sum::<f32>()
    };
    let (lo, hi) = px.iter().fold((f32::MAX, f32::MIN), |(l, h), p| {
        let t = proj(p);
        (l.min(t), h.max(t))
    });
    let end = |t: f32| -> [f32; 4] {
        let mut e = [255f32; 4];
        for c in 0..channels {
            e[c] = (mean[c] + axis[c] * t).clamp(0.0, 255.0);
        }
        e
    };
    let (mut e0, mut e1) = (end(lo), end(hi));

    // Quantize the endpoints; a decoder swaps (blue-contracts) them when the second sums to
    // less than the first, so order them the other way round when needed.
    let decoded: Vec<u32> = (0..levels).map(|v| unquantize_color(levels, v)).collect();
    let nearest = |x: f32| -> u32 {
        (0..levels)
            .min_by(|&a, &b| {
                (decoded[a as usize] as f32 - x)
                    .abs()
                    .total_cmp(&(decoded[b as usize] as f32 - x).abs())
            })
            .unwrap()
    };
    let quant =
        |e: &[f32; 4]| -> [u32; 4] { [nearest(e[0]), nearest(e[1]), nearest(e[2]), nearest(e[3])] };
    let mut q0 = quant(&e0);
    let mut q1 = quant(&e1);
    let sum = |q: &[u32; 4]| (0..3).map(|c| decoded[q[c] as usize]).sum::<u32>();
    if sum(&q1) < sum(&q0) {
        std::mem::swap(&mut q0, &mut q1);
        std::mem::swap(&mut e0, &mut e1);
    }
    let d0: [f32; 4] = std::array::from_fn(|c| decoded[q0[c] as usize] as f32);
    let d1: [f32; 4] = std::array::from_fn(|c| decoded[q1[c] as usize] as f32);
    let _ = (e0, e1);

    // Ideal weight (0..1) of each texel between the decoded endpoints.
    let dir: Vec<f32> = (0..channels).map(|c| d1[c] - d0[c]).collect();
    let len2: f32 = dir.iter().map(|v| v * v).sum();
    let ideal: Vec<f32> = px
        .iter()
        .map(|p| {
            if len2 < 1e-6 {
                0.0
            } else {
                ((0..channels).map(|c| (p[c] - d0[c]) * dir[c]).sum::<f32>() / len2).clamp(0.0, 1.0)
            }
        })
        .collect();
    // Grid weights: each texel's ideal weight spread back by its infill factors.
    let grid: Vec<f32> = if (mode.gw, mode.gh) == (bw, bh) {
        ideal.clone()
    } else {
        let mut acc = vec![(0f32, 0f32); weight_count as usize];
        for (i, taps) in infill(bw, bh, mode.gw, mode.gh).iter().enumerate() {
            for &(g, f) in taps {
                if f > 0 {
                    acc[g].0 += ideal[i] * f as f32;
                    acc[g].1 += f as f32;
                }
            }
        }
        acc.iter()
            .map(|&(s, w)| if w > 0.0 { s / w } else { 0.0 })
            .collect()
    };
    let wl = mode.weight_levels;
    let weights: Vec<u32> = grid
        .iter()
        .map(|&t| {
            (0..wl)
                .min_by(|&a, &b| {
                    (unquantize_weight(wl, a) as f32 - t * 64.0)
                        .abs()
                        .total_cmp(&(unquantize_weight(wl, b) as f32 - t * 64.0).abs())
                })
                .unwrap()
        })
        .collect();

    // Block: mode, one partition, CEM, endpoints from bit 17, weights reversed from the top.
    let mut block = Bits(0);
    let mut at = 0;
    block.put(&mut at, mode.block_mode, 11);
    block.put(&mut at, 0, 2);
    block.put(&mut at, if alpha { 12 } else { 8 }, 4);
    let mut color = Vec::with_capacity(values as usize);
    for c in 0..channels {
        color.push(q0[c]);
        color.push(q1[c]);
    }
    let (bits, n) = ise_encode(levels, &color);
    block.0 |= bits << at;
    let _ = n;
    let (wbits, wn) = ise_encode(wl, &weights);
    for i in 0..wn {
        if (wbits >> i) & 1 == 1 {
            block.0 |= 1u128 << (127 - i);
        }
    }
    block.0.to_le_bytes()
}

/// Encodes a `w`×`h` RGBA8 image (dimensions whole blocks) in `bw`×`bh` blocks.
pub fn encode(rgba: &[u8], w: u32, h: u32, bw: u32, bh: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(((w / bw) * (h / bh) * 16) as usize);
    let mut texels = Vec::with_capacity((bw * bh) as usize);
    for by in 0..h / bh {
        for bx in 0..w / bw {
            texels.clear();
            for y in 0..bh {
                for x in 0..bw {
                    let i = (((by * bh + y) * w + bx * bw + x) * 4) as usize;
                    texels.push([rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]);
                }
            }
            out.extend_from_slice(&encode_block(&texels, bw, bh));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trit_and_quint_codes_cover_every_combination() {
        let mut seen = std::collections::HashSet::new();
        for c in 0..256 {
            seen.insert(decode_trits(c));
        }
        assert_eq!(seen.len(), 243, "3^5 trit combinations");
        let mut seen = std::collections::HashSet::new();
        for c in 0..128 {
            seen.insert(decode_quints(c));
        }
        assert_eq!(seen.len(), 125, "5^3 quint combinations");
    }

    #[test]
    fn color_unquantization_spans_the_range_in_order() {
        for &l in &COLOR_LEVELS {
            // Codes are interleaved for trit/quint ranges, but the extremes hit 0 and 255.
            let vals: Vec<u32> = (0..l).map(|v| unquantize_color(l, v)).collect();
            assert_eq!(*vals.iter().min().unwrap(), 0, "{l}");
            assert_eq!(*vals.iter().max().unwrap(), 255, "{l}");
            let distinct: std::collections::HashSet<_> = vals.iter().collect();
            assert_eq!(distinct.len(), l as usize, "{l}");
        }
        assert_eq!(color_levels(6, 63), 256);
        assert_eq!(color_levels(8, 63), 192);
    }
}
