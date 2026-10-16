//! GPU block compression of textures at import (Unity's Texture Importer "Compression"):
//! BC7 for color, BC5 for normal maps, BC1/BC3 for low quality. Uses Intel's ISPC texture
//! compressor. <https://docs.unity3d.com/Manual/class-TextureImporterOverride.html>

use crate::texture::{CompressedFormat, CompressedMips, TextureCompression, TextureImporterType};

/// Bytes per 4×4 block.
pub fn block_bytes(f: CompressedFormat) -> usize {
    match f {
        CompressedFormat::Bc1 | CompressedFormat::Bc4 => 8,
        CompressedFormat::Bc3
        | CompressedFormat::Bc5
        | CompressedFormat::Bc7
        | CompressedFormat::Astc4x4 => 16,
    }
}

/// Size in bytes of a `w`×`h` level (whole 4×4 blocks).
pub fn level_size(f: CompressedFormat, w: u32, h: u32) -> usize {
    (w.div_ceil(4) * h.div_ceil(4)) as usize * block_bytes(f)
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

/// Copies `rgba` (`w`×`h`) into a buffer padded to whole 4×4 blocks, repeating edge pixels.
fn padded(rgba: &[u8], w: u32, h: u32) -> (Vec<u8>, u32, u32) {
    let (pw, ph) = (w.div_ceil(4) * 4, h.div_ceil(4) * 4);
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
    let (data, pw, ph) = padded(rgba, w, h);
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
        CompressedFormat::Astc4x4 => {
            let settings = it::astc::alpha_fast_settings(4, 4);
            it::astc::compress_blocks(&settings, &surface)
        }
    }
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
}
