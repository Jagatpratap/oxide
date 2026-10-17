//! GPU block compression of textures at import (Unity's Texture Importer "Compression" and
//! platform formats): BC7 for color, BC5 for normal maps, BC1/BC3 for low quality on
//! desktop; ASTC 4×4/6×6/8×8 and ETC2/EAC on mobile. Uses Intel's ISPC texture compressor
//! (BC, ETC1 — valid ETC2 RGB), Oxide's ASTC encoder and a small EAC encoder here.
//! <https://docs.unity3d.com/Manual/class-TextureImporterOverride.html>

use crate::texture::{CompressedFormat, CompressedMips, TextureCompression, TextureImporterType};

/// Bytes per block.
pub fn block_bytes(f: CompressedFormat) -> usize {
    use CompressedFormat as C;
    match f {
        C::Bc1 | C::Bc4 | C::Etc2Rgb8 | C::EacR11 => 8,
        C::Bc3
        | C::Bc5
        | C::Bc7
        | C::Astc4x4
        | C::Astc6x6
        | C::Astc8x8
        | C::Etc2Rgba8
        | C::EacRg11 => 16,
    }
}

/// Size in bytes of a `w`×`h` level (whole blocks).
pub fn level_size(f: CompressedFormat, w: u32, h: u32) -> usize {
    let (bw, bh) = f.block();
    (w.div_ceil(bw) * h.div_ceil(bh)) as usize * block_bytes(f)
}

/// The format a texture gets, or `None` to keep it uncompressed.
pub fn choose_format(
    compression: TextureCompression,
    kind: TextureImporterType,
    has_alpha: bool,
) -> Option<CompressedFormat> {
    match (compression, kind) {
        (TextureCompression::None, _) => None,
        (_, TextureImporterType::NormalMap) => Some(CompressedFormat::Bc5),
        (TextureCompression::LowQuality, _) if has_alpha => Some(CompressedFormat::Bc3),
        (TextureCompression::LowQuality, _) => Some(CompressedFormat::Bc1),
        (TextureCompression::NormalQuality | TextureCompression::HighQuality, _) => {
            Some(CompressedFormat::Bc7)
        }
    }
}

/// Copies `rgba` (`w`×`h`) into a buffer padded to whole `bw`×`bh` blocks, repeating edge
/// pixels.
fn padded(rgba: &[u8], w: u32, h: u32, (bw, bh): (u32, u32)) -> (Vec<u8>, u32, u32) {
    let (pw, ph) = (w.div_ceil(bw) * bw, h.div_ceil(bh) * bh);
    if (pw, ph) == (w, h) {
        return (rgba.to_vec(), w, h);
    }
    let mut out = vec![0u8; (pw * ph * 4) as usize];
    for y in 0..ph {
        for x in 0..pw {
            let s = ((y.min(h - 1) * w + x.min(w - 1)) * 4) as usize;
            let d = ((y * pw + x) * 4) as usize;
            out[d..d + 4].copy_from_slice(&rgba[s..s + 4]);
        }
    }
    (out, pw, ph)
}

/// Compresses one RGBA8 level.
pub fn compress_level(
    format: CompressedFormat,
    quality: TextureCompression,
    rgba: &[u8],
    w: u32,
    h: u32,
) -> Vec<u8> {
    use intel_tex_2 as it;
    let (data, pw, ph) = padded(rgba, w, h, format.block());
    let surface = it::RgbaSurface {
        data: &data,
        width: pw,
        height: ph,
        stride: pw * 4,
    };
    match format {
        CompressedFormat::Bc1 => it::bc1::compress_blocks(&surface),
        CompressedFormat::Bc3 => it::bc3::compress_blocks(&surface),
        CompressedFormat::Bc7 => {
            let has_alpha = data.as_chunks::<4>().0.iter().any(|p| p[3] < 255);
            let settings = match (quality, has_alpha) {
                (TextureCompression::HighQuality, false) => it::bc7::opaque_slow_settings(),
                (TextureCompression::HighQuality, true) => it::bc7::alpha_slow_settings(),
                (_, false) => it::bc7::opaque_fast_settings(),
                (_, true) => it::bc7::alpha_fast_settings(),
            };
            it::bc7::compress_blocks(&settings, &surface)
        }
        CompressedFormat::Bc4 => {
            let r: Vec<u8> = data.as_chunks::<4>().0.iter().map(|p| p[0]).collect();
            it::bc4::compress_blocks(&it::RSurface {
                data: &r,
                width: pw,
                height: ph,
                stride: pw,
            })
        }
        CompressedFormat::Bc5 => {
            let rg: Vec<u8> = data
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[1]])
                .collect();
            it::bc5::compress_blocks(&it::RgSurface {
                data: &rg,
                width: pw,
                height: ph,
                stride: pw * 2,
            })
        }
        CompressedFormat::Astc4x4 | CompressedFormat::Astc6x6 | CompressedFormat::Astc8x8 => {
            // intel_tex_2's ASTC path is unimplemented; Oxide has its own encoder.
            let (bw, bh) = format.block();
            crate::astc::encode(&data, pw, ph, bw, bh)
        }
        CompressedFormat::Etc2Rgb8 => etc1(&surface),
        CompressedFormat::Etc2Rgba8 => {
            // Per block: the EAC alpha block, then the color block.
            let color = etc1(&surface);
            let mut out = Vec::with_capacity(color.len() * 2);
            for (i, c) in color.as_chunks::<8>().0.iter().enumerate() {
                let (bx, by) = (i as u32 % (pw / 4), i as u32 / (pw / 4));
                out.extend_from_slice(&eac_block(&channel_block(&data, pw, bx, by, 3), false));
                out.extend_from_slice(c);
            }
            out
        }
        CompressedFormat::EacR11 | CompressedFormat::EacRg11 => {
            let channels = if format == CompressedFormat::EacR11 {
                1
            } else {
                2
            };
            let mut out = Vec::new();
            for by in 0..ph / 4 {
                for bx in 0..pw / 4 {
                    for c in 0..channels {
                        out.extend_from_slice(&eac_block(
                            &channel_block(&data, pw, bx, by, c),
                            true,
                        ));
                    }
                }
            }
            out
        }
    }
}

/// ETC1 blocks (valid ETC2 RGB8: ETC1 blocks never use ETC2's extra modes).
fn etc1(surface: &intel_tex_2::RgbaSurface) -> Vec<u8> {
    let (bw, bh) = (surface.width / 4, surface.height / 4);
    let mut out = Vec::with_capacity((bw * bh * 8) as usize);
    for by in 0..bh {
        for bx in 0..bw {
            let px: [[u8; 3]; 16] = std::array::from_fn(|i| {
                let (x, y) = (bx * 4 + i as u32 % 4, by * 4 + i as u32 / 4);
                let o = (y * surface.stride + x * 4) as usize;
                [surface.data[o], surface.data[o + 1], surface.data[o + 2]]
            });
            out.extend_from_slice(&etc1_block(&px));
        }
    }
    out
}

/// ETC1 modifier tables (index 0..3 = +a, +b, -a, -b).
const ETC1_TABLES: [[i32; 2]; 8] = [
    [2, 8],
    [5, 17],
    [9, 29],
    [13, 42],
    [18, 60],
    [24, 80],
    [33, 106],
    [47, 183],
];

/// Encodes 16 RGB texels (row-major) as one ETC1 block: both sub-block splits, the
/// differential or individual base colours, and every modifier table, keeping the best.
pub fn etc1_block(px: &[[u8; 3]; 16]) -> [u8; 8] {
    let modifier = |t: usize, i: usize| match i {
        0 => ETC1_TABLES[t][0],
        1 => ETC1_TABLES[t][1],
        2 => -ETC1_TABLES[t][0],
        _ => -ETC1_TABLES[t][1],
    };
    // Best table and indices of one sub-block for a base colour: (error, table, indices).
    let fit = |members: &[usize], base: [i32; 3]| -> (i64, usize, Vec<(usize, usize)>) {
        let mut best = (i64::MAX, 0, Vec::new());
        for t in 0..8 {
            let mut err = 0i64;
            let mut idx = Vec::with_capacity(members.len());
            for &p in members {
                let (mut be, mut bi) = (i64::MAX, 0);
                for i in 0..4 {
                    let m = modifier(t, i);
                    let e: i64 = (0..3)
                        .map(|c| {
                            let d = ((base[c] + m).clamp(0, 255) - px[p][c] as i32) as i64;
                            d * d
                        })
                        .sum();
                    if e < be {
                        (be, bi) = (e, i);
                    }
                }
                err += be;
                idx.push((p, bi));
            }
            if err < best.0 {
                best = (err, t, idx);
            }
        }
        best
    };
    let mut best: Option<(i64, u64)> = None;
    for flip in [false, true] {
        let halves: [Vec<usize>; 2] = std::array::from_fn(|h| {
            (0..16)
                .filter(|&i| {
                    let (x, y) = (i % 4, i / 4);
                    (if flip { y } else { x }) / 2 == h
                })
                .collect()
        });
        let avg: [[f32; 3]; 2] = std::array::from_fn(|h| {
            let mut a = [0f32; 3];
            for &p in &halves[h] {
                for c in 0..3 {
                    a[c] += px[p][c] as f32 / 8.0;
                }
            }
            a
        });
        // Differential: 5-bit bases with a 3-bit signed delta; else individual 4-bit bases.
        let q5: [[i32; 3]; 2] = avg.map(|a| a.map(|v| (v * 31.0 / 255.0).round() as i32));
        let delta: [i32; 3] = std::array::from_fn(|c| q5[1][c] - q5[0][c]);
        let mut modes = Vec::new();
        if delta.iter().all(|d| (-4..=3).contains(d)) {
            let e5 = |v: i32| (v << 3) | (v >> 2);
            modes.push((true, [q5[0].map(e5), q5[1].map(e5)]));
        }
        let q4: [[i32; 3]; 2] = avg.map(|a| a.map(|v| (v * 15.0 / 255.0).round() as i32));
        modes.push((false, [q4[0].map(|v| v * 17), q4[1].map(|v| v * 17)]));
        for (diff, bases) in modes {
            let (e0, t0, i0) = fit(&halves[0], bases[0]);
            let (e1, t1, i1) = fit(&halves[1], bases[1]);
            let err = e0 + e1;
            if best.is_some_and(|(b, _)| b <= err) {
                continue;
            }
            let mut bits: u64 = 0;
            if diff {
                for c in 0..3 {
                    let shift = 59 - 8 * c as u32;
                    bits |= (q5[0][c] as u64) << shift;
                    bits |= ((delta[c] & 7) as u64) << (shift - 3);
                }
            } else {
                for (c, (a, b)) in q4[0].iter().zip(&q4[1]).enumerate() {
                    let shift = 60 - 8 * c as u32;
                    bits |= (*a as u64) << shift;
                    bits |= (*b as u64) << (shift - 4);
                }
            }
            bits |= (t0 as u64) << 37 | (t1 as u64) << 34;
            bits |= (diff as u64) << 33 | (flip as u64) << 32;
            for (p, i) in i0.into_iter().chain(i1) {
                let (x, y) = (p % 4, p / 4);
                let at = x * 4 + y;
                bits |= ((i >> 1) as u64) << (16 + at) | ((i & 1) as u64) << at;
            }
            best = Some((err, bits));
        }
    }
    best.expect("a mode").1.to_be_bytes()
}

/// One channel of the 4×4 block at block `(bx, by)` of a `pw`-wide RGBA8 image, in EAC's
/// pixel order (column by column).
fn channel_block(rgba: &[u8], pw: u32, bx: u32, by: u32, c: usize) -> [u8; 16] {
    let mut v = [0u8; 16];
    for x in 0..4 {
        for y in 0..4 {
            let i = (((by * 4 + y) * pw + bx * 4 + x) * 4) as usize + c;
            v[(x * 4 + y) as usize] = rgba[i];
        }
    }
    v
}

/// EAC modifier tables (ETC2 spec, table C.12).
const EAC_TABLES: [[i32; 8]; 16] = [
    [-3, -6, -9, -15, 2, 5, 8, 14],
    [-3, -7, -10, -13, 2, 6, 9, 12],
    [-2, -5, -8, -13, 1, 4, 7, 12],
    [-2, -4, -6, -13, 1, 3, 5, 12],
    [-3, -6, -8, -12, 2, 5, 7, 11],
    [-3, -7, -9, -11, 2, 6, 8, 10],
    [-4, -7, -8, -11, 3, 6, 7, 10],
    [-3, -5, -8, -11, 2, 4, 7, 10],
    [-2, -6, -8, -10, 1, 5, 7, 9],
    [-2, -5, -8, -10, 1, 4, 7, 9],
    [-2, -4, -8, -10, 1, 3, 7, 9],
    [-2, -5, -7, -10, 1, 4, 6, 9],
    [-3, -4, -7, -10, 2, 3, 6, 9],
    [-1, -2, -3, -10, 0, 1, 2, 9],
    [-4, -6, -8, -9, 3, 5, 7, 8],
    [-3, -5, -7, -9, 2, 4, 6, 8],
];

/// Decodes one EAC value: 8-bit alpha, or R11 (`r11`, 0..2047).
pub fn eac_decode(base: i32, mult: i32, modifier: i32, r11: bool) -> i32 {
    if r11 {
        let m = if mult == 0 {
            modifier
        } else {
            modifier * mult * 8
        };
        (base * 8 + 4 + m).clamp(0, 2047)
    } else {
        (base + modifier * mult).clamp(0, 255)
    }
}

/// Encodes 16 values (column order) as one EAC block: an alpha block, or an R11 block when
/// `r11` (same layout, 11-bit decode).
pub fn eac_block(values: &[u8; 16], r11: bool) -> [u8; 8] {
    let target = |v: u8| if r11 { v as i32 * 2047 / 255 } else { v as i32 };
    let (lo, hi) = values
        .iter()
        .fold((255u8, 0u8), |(l, h), &v| (l.min(v), h.max(v)));
    let mut best = (i64::MAX, 0i32, 0i32, 0usize, [0u8; 16]);
    for (t, table) in EAC_TABLES.iter().enumerate() {
        let span = (table[7] - table[3]) as f32;
        let ideal = (hi as f32 - lo as f32) / span;
        let center = (hi as f32 + lo as f32) * 0.5 - (table[7] + table[3]) as f32 * 0.5 * ideal;
        let m0 = ideal.round() as i32;
        for mult in m0 - 2..=m0 + 2 {
            let mult = mult.clamp(if r11 { 0 } else { 1 }, 15);
            let b0 = center.round() as i32;
            for base in b0 - 2..=b0 + 2 {
                let base = base.clamp(0, 255);
                let mut err = 0i64;
                let mut idx = [0u8; 16];
                for (i, &v) in values.iter().enumerate() {
                    let want = target(v);
                    let (mut be, mut bi) = (i64::MAX, 0);
                    for (k, &m) in table.iter().enumerate() {
                        let d = (eac_decode(base, mult, m, r11) - want) as i64;
                        if d * d < be {
                            (be, bi) = (d * d, k);
                        }
                    }
                    err += be;
                    idx[i] = bi as u8;
                }
                if err < best.0 {
                    best = (err, base, mult, t, idx);
                }
            }
        }
    }
    let (_, base, mult, table, idx) = best;
    let mut bits: u64 = (base as u64) << 56 | (mult as u64) << 52 | (table as u64) << 48;
    for (i, &k) in idx.iter().enumerate() {
        bits |= (k as u64) << (45 - 3 * i);
    }
    bits.to_be_bytes()
}

/// Compresses every mip (RGBA8, largest first) of a `w`×`h` texture.
pub fn compress_mips(
    format: CompressedFormat,
    quality: TextureCompression,
    mips: &[Vec<u8>],
    w: u32,
    h: u32,
) -> CompressedMips {
    let mips = mips
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let (lw, lh) = ((w >> i).max(1), (h >> i).max(1));
            compress_level(format, quality, m, lw, lh)
        })
        .collect();
    CompressedMips { format, mips }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compressed_sizes_match_block_math() {
        let (w, h) = (16u32, 8u32);
        let rgba: Vec<u8> = (0..w * h)
            .flat_map(|i| [i as u8, 0, 255 - i as u8, 255])
            .collect();
        for f in [
            CompressedFormat::Bc1,
            CompressedFormat::Bc3,
            CompressedFormat::Bc4,
            CompressedFormat::Bc5,
            CompressedFormat::Bc7,
        ] {
            let out = compress_level(f, TextureCompression::NormalQuality, &rgba, w, h);
            assert_eq!(out.len(), level_size(f, w, h), "{f:?}");
        }
        // A 2×2 mip still takes one whole block.
        let tiny = compress_level(
            CompressedFormat::Bc7,
            TextureCompression::NormalQuality,
            &[255; 16],
            2,
            2,
        );
        assert_eq!(tiny.len(), 16);
    }

    #[test]
    fn normal_maps_use_bc5_and_low_quality_alpha_uses_bc3() {
        assert_eq!(
            choose_format(
                TextureCompression::NormalQuality,
                TextureImporterType::NormalMap,
                false
            ),
            Some(CompressedFormat::Bc5)
        );
        assert_eq!(
            choose_format(
                TextureCompression::LowQuality,
                TextureImporterType::Default,
                true
            ),
            Some(CompressedFormat::Bc3)
        );
        assert_eq!(
            choose_format(
                TextureCompression::None,
                TextureImporterType::Default,
                false
            ),
            None
        );
    }

    /// Decodes an EAC block back to 16 values (column order), for tests.
    fn eac_unpack(b: [u8; 8], r11: bool) -> Vec<i32> {
        let bits = u64::from_be_bytes(b);
        let (base, mult, table) = (
            (bits >> 56) as i32,
            (bits >> 52 & 15) as i32,
            (bits >> 48 & 15) as usize,
        );
        (0..16)
            .map(|i| {
                let k = (bits >> (45 - 3 * i) & 7) as usize;
                eac_decode(base, mult, EAC_TABLES[table][k], r11)
            })
            .collect()
    }

    #[test]
    fn eac_blocks_round_trip_closely() {
        let mut seed = 7u32;
        let mut rnd = || {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            (seed >> 16) as u8
        };
        for case in 0..200 {
            // Smooth gradients, flat blocks and noise.
            let lo = rnd();
            let span = if case % 3 == 0 { 0 } else { rnd() / 2 };
            let vals: [u8; 16] = std::array::from_fn(|i| {
                if case % 5 == 4 {
                    rnd()
                } else {
                    lo.saturating_add((span as u32 * i as u32 / 15) as u8)
                }
            });
            for r11 in [false, true] {
                let back = eac_unpack(eac_block(&vals, r11), r11);
                let scale = if r11 { 2047.0 / 255.0 } else { 1.0 };
                let noisy = case % 5 == 4;
                // Eight levels per block: at best half a step off.
                let bound = if noisy {
                    40.0
                } else {
                    span as f32 / 14.0 + 2.0
                };
                for (v, b) in vals.iter().zip(&back) {
                    let err = (*v as f32 * scale - *b as f32).abs() / scale;
                    assert!(err <= bound, "case {case} r11 {r11}: {vals:?} -> {back:?}");
                }
            }
        }
    }

    #[test]
    fn mobile_formats_have_the_right_sizes() {
        let (w, h) = (24u32, 24u32);
        let rgba: Vec<u8> = (0..w * h).flat_map(|i| [i as u8, 7, 200, 128]).collect();
        for f in [
            CompressedFormat::Etc2Rgb8,
            CompressedFormat::Etc2Rgba8,
            CompressedFormat::EacR11,
            CompressedFormat::EacRg11,
            CompressedFormat::Astc4x4,
            CompressedFormat::Astc6x6,
            CompressedFormat::Astc8x8,
        ] {
            let out = compress_level(f, TextureCompression::NormalQuality, &rgba, w, h);
            assert_eq!(out.len(), level_size(f, w, h), "{f:?}");
        }
        // 24/8 = 3 blocks across.
        assert_eq!(level_size(CompressedFormat::Astc8x8, 24, 24), 9 * 16);
        assert_eq!(level_size(CompressedFormat::Astc6x6, 24, 24), 16 * 16);
    }

    #[test]
    fn platform_overrides_pick_formats() {
        use crate::texture::{
            BuildTarget, TextureImporter, TextureImporterFormat, TexturePlatformSettings,
        };
        let mut imp = TextureImporter::default();
        assert_eq!(
            imp.resolve(BuildTarget::Standalone, false),
            (2048, Some(CompressedFormat::Bc7))
        );
        // Unity's mobile default: ASTC 6×6 at Normal Quality.
        assert_eq!(
            imp.resolve(BuildTarget::Android, false).1,
            Some(CompressedFormat::Astc6x6)
        );
        imp.platform_settings.push(TexturePlatformSettings {
            platform: BuildTarget::Android,
            overridden: true,
            max_texture_size: 512,
            format: TextureImporterFormat::Etc2Rgba8,
            compression: TextureCompression::NormalQuality,
        });
        assert_eq!(
            imp.resolve(BuildTarget::Android, true),
            (512, Some(CompressedFormat::Etc2Rgba8))
        );
        // Not overridden: the defaults.
        imp.platform_settings[0].overridden = false;
        assert_eq!(imp.resolve(BuildTarget::Android, true).0, 2048);
        assert_eq!(
            imp.resolve(BuildTarget::IOS, false).1,
            Some(CompressedFormat::Astc6x6)
        );
    }
}
