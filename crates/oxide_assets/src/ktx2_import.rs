//! KTX2 textures (Khronos container for GPU-ready textures): BC1/3/4/5/7, ASTC 4×4/6×6/8×8,
//! ETC2/EAC or RGBA8
//! payloads with all their mips, optionally Zstandard-supercompressed. Block-compressed
//! KTX2 textures go to the GPU as they are and have no CPU pixel copy (like Unity textures
//! without Read/Write). Basis Universal payloads need transcoding and aren't supported yet.
//! <https://registry.khronos.org/KTX/specs/2.0/ktxspec.v2.html>

use crate::texture::{CompressedFormat, CompressedMips, FilterMode, Texture2D, TextureWrapMode};
use ktx2::{Format, SupercompressionScheme};

pub fn decode(name: &str, bytes: &[u8]) -> Result<Texture2D, String> {
    let reader = ktx2::Reader::new(bytes).map_err(|e| format!("{name}: {e:?}"))?;
    let h = reader.header();
    if h.face_count > 1 || h.layer_count > 1 || h.pixel_depth > 1 {
        return Err(format!(
            "{name}: only 2D KTX2 textures are supported (no cubemaps or arrays)"
        ));
    }
    let format = h.format.ok_or_else(|| {
        format!("{name}: Basis Universal KTX2 needs transcoding (not supported yet)")
    })?;
    let (kind, srgb) = match format {
        Format::BC1_RGBA_UNORM_BLOCK => (Some(CompressedFormat::Bc1), false),
        Format::BC1_RGBA_SRGB_BLOCK => (Some(CompressedFormat::Bc1), true),
        Format::BC3_UNORM_BLOCK => (Some(CompressedFormat::Bc3), false),
        Format::BC3_SRGB_BLOCK => (Some(CompressedFormat::Bc3), true),
        Format::BC4_UNORM_BLOCK => (Some(CompressedFormat::Bc4), false),
        Format::BC5_UNORM_BLOCK => (Some(CompressedFormat::Bc5), false),
        Format::BC7_UNORM_BLOCK => (Some(CompressedFormat::Bc7), false),
        Format::BC7_SRGB_BLOCK => (Some(CompressedFormat::Bc7), true),
        Format::ASTC_4x4_UNORM_BLOCK => (Some(CompressedFormat::Astc4x4), false),
        Format::ASTC_4x4_SRGB_BLOCK => (Some(CompressedFormat::Astc4x4), true),
        Format::ASTC_6x6_UNORM_BLOCK => (Some(CompressedFormat::Astc6x6), false),
        Format::ASTC_6x6_SRGB_BLOCK => (Some(CompressedFormat::Astc6x6), true),
        Format::ASTC_8x8_UNORM_BLOCK => (Some(CompressedFormat::Astc8x8), false),
        Format::ASTC_8x8_SRGB_BLOCK => (Some(CompressedFormat::Astc8x8), true),
        Format::ETC2_R8G8B8_UNORM_BLOCK => (Some(CompressedFormat::Etc2Rgb8), false),
        Format::ETC2_R8G8B8_SRGB_BLOCK => (Some(CompressedFormat::Etc2Rgb8), true),
        Format::ETC2_R8G8B8A8_UNORM_BLOCK => (Some(CompressedFormat::Etc2Rgba8), false),
        Format::ETC2_R8G8B8A8_SRGB_BLOCK => (Some(CompressedFormat::Etc2Rgba8), true),
        Format::EAC_R11_UNORM_BLOCK => (Some(CompressedFormat::EacR11), false),
        Format::EAC_R11G11_UNORM_BLOCK => (Some(CompressedFormat::EacRg11), false),
        Format::R8G8B8A8_UNORM => (None, false),
        Format::R8G8B8A8_SRGB => (None, true),
        other => return Err(format!("{name}: KTX2 format {other:?} is not supported")),
    };
    let mut levels = Vec::new();
    for level in reader.levels() {
        let data = match h.supercompression_scheme {
            None => level.data.to_vec(),
            Some(SupercompressionScheme::Zstandard) => {
                use std::io::Read;
                let mut src = level.data;
                let mut dec = ruzstd::decoding::StreamingDecoder::new(&mut src)
                    .map_err(|e| format!("{name}: zstd: {e}"))?;
                let mut out = Vec::with_capacity(level.uncompressed_byte_length as usize);
                dec.read_to_end(&mut out)
                    .map_err(|e| format!("{name}: zstd: {e}"))?;
                out
            }
            Some(other) => {
                return Err(format!(
                    "{name}: KTX2 supercompression {other:?} is not supported"
                ));
            }
        };
        levels.push(data);
    }
    let (width, height) = (h.pixel_width.max(1), h.pixel_height.max(1));
    let (mips, compressed) = match kind {
        Some(format) => (
            Vec::new(),
            Some(CompressedMips {
                format,
                mips: levels,
            }),
        ),
        None => (levels, None),
    };
    Ok(Texture2D {
        name: name.to_owned(),
        width,
        height,
        mips,
        srgb,
        wrap_mode: TextureWrapMode::Repeat,
        filter_mode: FilterMode::Trilinear,
        aniso_level: 8,
        compressed,
    })
}

/// Writes `tex` as KTX2: its block-compressed mips if it has them, else RGBA8. With `zstd`
/// each level is Zstandard-supercompressed.
pub fn encode(tex: &Texture2D, zstd: bool) -> Vec<u8> {
    let (format, levels): (Format, &[Vec<u8>]) = match &tex.compressed {
        Some(c) => (
            match (c.format, tex.srgb) {
                (CompressedFormat::Bc1, false) => Format::BC1_RGBA_UNORM_BLOCK,
                (CompressedFormat::Bc1, true) => Format::BC1_RGBA_SRGB_BLOCK,
                (CompressedFormat::Bc3, false) => Format::BC3_UNORM_BLOCK,
                (CompressedFormat::Bc3, true) => Format::BC3_SRGB_BLOCK,
                (CompressedFormat::Bc4, _) => Format::BC4_UNORM_BLOCK,
                (CompressedFormat::Bc5, _) => Format::BC5_UNORM_BLOCK,
                (CompressedFormat::Bc7, false) => Format::BC7_UNORM_BLOCK,
                (CompressedFormat::Bc7, true) => Format::BC7_SRGB_BLOCK,
                (CompressedFormat::Astc4x4, false) => Format::ASTC_4x4_UNORM_BLOCK,
                (CompressedFormat::Astc4x4, true) => Format::ASTC_4x4_SRGB_BLOCK,
                (CompressedFormat::Astc6x6, false) => Format::ASTC_6x6_UNORM_BLOCK,
                (CompressedFormat::Astc6x6, true) => Format::ASTC_6x6_SRGB_BLOCK,
                (CompressedFormat::Astc8x8, false) => Format::ASTC_8x8_UNORM_BLOCK,
                (CompressedFormat::Astc8x8, true) => Format::ASTC_8x8_SRGB_BLOCK,
                (CompressedFormat::Etc2Rgb8, false) => Format::ETC2_R8G8B8_UNORM_BLOCK,
                (CompressedFormat::Etc2Rgb8, true) => Format::ETC2_R8G8B8_SRGB_BLOCK,
                (CompressedFormat::Etc2Rgba8, false) => Format::ETC2_R8G8B8A8_UNORM_BLOCK,
                (CompressedFormat::Etc2Rgba8, true) => Format::ETC2_R8G8B8A8_SRGB_BLOCK,
                (CompressedFormat::EacR11, _) => Format::EAC_R11_UNORM_BLOCK,
                (CompressedFormat::EacRg11, _) => Format::EAC_R11G11_UNORM_BLOCK,
            },
            &c.mips,
        ),
        None => (
            if tex.srgb {
                Format::R8G8B8A8_SRGB
            } else {
                Format::R8G8B8A8_UNORM
            },
            &tex.mips,
        ),
    };
    let payloads: Vec<Vec<u8>> = levels
        .iter()
        .map(|l| {
            if zstd {
                ruzstd::encoding::compress_to_vec(
                    &l[..],
                    ruzstd::encoding::CompressionLevel::Fastest,
                )
            } else {
                l.clone()
            }
        })
        .collect();
    let index_len = ktx2::LevelIndex::LENGTH * levels.len();
    let dfd_offset = ktx2::Header::LENGTH + index_len;
    // An empty data format descriptor: just its total size.
    let dfd = 4u32.to_le_bytes();
    let mut at = (dfd_offset + dfd.len()) as u64;
    let mut index = Vec::new();
    for (raw, p) in levels.iter().zip(&payloads) {
        index.extend_from_slice(
            &ktx2::LevelIndex {
                byte_offset: at,
                byte_length: p.len() as u64,
                uncompressed_byte_length: raw.len() as u64,
            }
            .as_bytes(),
        );
        at += p.len() as u64;
    }
    let header = ktx2::Header {
        format: Some(format),
        type_size: 1,
        pixel_width: tex.width,
        pixel_height: tex.height,
        pixel_depth: 0,
        layer_count: 0,
        face_count: 1,
        level_count: levels.len() as u32,
        supercompression_scheme: zstd.then_some(SupercompressionScheme::Zstandard),
        index: ktx2::Index {
            dfd_byte_offset: dfd_offset as u32,
            dfd_byte_length: dfd.len() as u32,
            kvd_byte_offset: 0,
            kvd_byte_length: 0,
            sgd_byte_offset: 0,
            sgd_byte_length: 0,
        },
    };
    let mut out = header.as_bytes().to_vec();
    out.extend_from_slice(&index);
    out.extend_from_slice(&dfd);
    for p in &payloads {
        out.extend_from_slice(p);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::texture::TextureCompression;

    #[test]
    fn bc7_round_trips_through_zstd_ktx2() {
        let mut t = Texture2D::checker("c", 32, 4, [255, 0, 0, 255], [0, 0, 255, 255]);
        t.compressed = Some(crate::texture_compress::compress_mips(
            CompressedFormat::Bc7,
            TextureCompression::NormalQuality,
            &t.mips,
            t.width,
            t.height,
        ));
        for zstd in [false, true] {
            let back = decode("c", &encode(&t, zstd)).unwrap();
            assert_eq!((back.width, back.height, back.srgb), (32, 32, true));
            assert!(back.mips.is_empty(), "no CPU copy for compressed KTX2");
            let (a, b) = (t.compressed.as_ref().unwrap(), back.compressed.unwrap());
            assert_eq!(b.format, CompressedFormat::Bc7);
            assert_eq!(a.mips, b.mips);
        }
        // Uncompressed RGBA8 keeps CPU pixels.
        t.compressed = None;
        let back = decode("c", &encode(&t, true)).unwrap();
        assert_eq!(back.mips, t.mips);
    }
}
