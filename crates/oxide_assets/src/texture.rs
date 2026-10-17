//! Unity `Texture2D`: 8-bit RGBA pixels with a full mip chain, plus sampler settings.
//! Reference: <https://docs.unity3d.com/Manual/class-TextureImporter.html>

use crate::handle::Asset;
use oxide_core::Reflect;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Reflect)]
pub enum TextureWrapMode {
    #[default]
    Repeat,
    Clamp,
    Mirror,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Reflect)]
pub enum FilterMode {
    Point,
    Bilinear,
    #[default]
    Trilinear,
}

/// Unity `TextureImporterType` (the kinds that change how pixels are stored).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Reflect)]
pub enum TextureImporterType {
    #[default]
    Default,
    /// Linear data, compressed as two channels (BC5); the shader rebuilds Z.
    NormalMap,
    /// Unity "Sprite (2D and UI)": makes Sprite sub-assets for SpriteRenderers, tiles and UI.
    Sprite,
}

/// Unity `TextureImporterCompression`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Reflect)]
pub enum TextureCompression {
    /// RGBA8.
    None,
    /// BC1 (opaque) / BC3 (alpha): 4–8 bits per pixel.
    LowQuality,
    /// BC7 (BC5 for normal maps): 8 bits per pixel.
    #[default]
    NormalQuality,
    /// BC7 with the slowest, best encoder settings.
    HighQuality,
}

/// Unity `TextureImporterShape`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Reflect)]
pub enum TextureShape {
    #[default]
    Texture2D,
    /// Six faces made from the image ([`CubemapMapping`]): a [`TextureCube`] asset.
    Cube,
    /// Layers cut from a grid of cells (Columns × Rows): a [`Texture2DArray`] asset.
    Texture2DArray,
}

/// Unity cubemap "Mapping": how the image holds the six faces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Reflect)]
pub enum CubemapMapping {
    /// From the aspect ratio: 6 frames (cross, strip) or a 2:1 panorama.
    #[default]
    Auto,
    /// A horizontal or vertical cross, or a row or column of six faces.
    SixFramesLayout,
    /// An equirectangular (latitude-longitude) panorama.
    LatitudeLongitude,
}

/// GPU block-compressed formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Reflect)]
pub enum CompressedFormat {
    Bc1,
    Bc3,
    Bc4,
    Bc5,
    Bc7,
    Astc4x4,
    Astc6x6,
    Astc8x8,
    /// ETC2 RGB (4 bits per pixel; ETC1 blocks are valid ETC2).
    Etc2Rgb8,
    /// ETC2 RGB plus an EAC alpha block (8 bits per pixel).
    Etc2Rgba8,
    /// One channel (EAC R11).
    EacR11,
    /// Two channels (EAC RG11), for normal maps.
    EacRg11,
}

impl CompressedFormat {
    /// Block size in pixels.
    pub fn block(self) -> (u32, u32) {
        match self {
            CompressedFormat::Astc6x6 => (6, 6),
            CompressedFormat::Astc8x8 => (8, 8),
            _ => (4, 4),
        }
    }
}

/// Unity `TextureImporterFormat`: a platform's explicit texture format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Reflect)]
pub enum TextureImporterFormat {
    /// The platform's default for the Compression setting.
    #[default]
    Automatic,
    RGBA32,
    DXT1,
    DXT5,
    BC4,
    BC5,
    BC7,
    Etc2Rgb4,
    Etc2Rgba8,
    EacR,
    EacRg,
    Astc4x4,
    Astc6x6,
    Astc8x8,
}

/// The platform textures are imported for (Unity's build target groups).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Reflect)]
pub enum BuildTarget {
    /// Windows, macOS and Linux.
    #[default]
    Standalone,
    Android,
    IOS,
    WebGL,
}

impl BuildTarget {
    /// The platform this program runs on.
    pub fn current() -> Self {
        if cfg!(target_os = "android") {
            BuildTarget::Android
        } else if cfg!(target_os = "ios") {
            BuildTarget::IOS
        } else if cfg!(target_arch = "wasm32") {
            BuildTarget::WebGL
        } else {
            BuildTarget::Standalone
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            BuildTarget::Standalone => "Standalone",
            BuildTarget::Android => "Android",
            BuildTarget::IOS => "iOS",
            BuildTarget::WebGL => "WebGL",
        }
    }
}

/// Unity `TextureImporterPlatformSettings`: a platform's override of the import settings.
#[derive(Clone, Debug, Default, PartialEq, Reflect)]
#[reflect(default)]
pub struct TexturePlatformSettings {
    pub platform: BuildTarget,
    /// Off: the platform uses the default settings.
    pub overridden: bool,
    pub max_texture_size: u32,
    pub format: TextureImporterFormat,
    pub compression: TextureCompression,
}

/// A texture's block-compressed mip chain (what the GPU gets when it supports the format).
#[derive(Clone, Debug)]
pub struct CompressedMips {
    pub format: CompressedFormat,
    /// Largest first, each in whole 4×4 blocks.
    pub mips: Vec<Vec<u8>>,
}

/// Unity Texture Import Settings, kept in `<texture>.meta`.
/// <https://docs.unity3d.com/Manual/class-TextureImporter.html>
#[derive(Clone, Debug, PartialEq, Reflect)]
#[reflect(default)]
pub struct TextureImporter {
    pub texture_type: TextureImporterType,
    /// Color data (sRGB) rather than masks or normals.
    pub srgb: bool,
    /// Larger textures are scaled down to this on import.
    pub max_size: u32,
    pub generate_mip_maps: bool,
    pub wrap_mode: TextureWrapMode,
    pub filter_mode: FilterMode,
    #[reflect(range(0, 16))]
    pub aniso_level: u32,
    pub compression: TextureCompression,
    /// Sprite type: one sprite, or a sheet sliced in the Sprite Editor.
    pub sprite_mode: crate::sprite::SpriteImportMode,
    /// Texture pixels per world unit (Unity default 100).
    pub pixels_per_unit: f32,
    /// Single sprite's normalized pivot.
    pub sprite_pivot: oxide_math::Vector2,
    /// Single sprite's 9-slice border (left, bottom, right, top pixels).
    pub sprite_border: oxide_math::Vector4,
    /// Multiple: the slices (Sprite Editor).
    pub sprites: Vec<crate::sprite::SpriteRect>,
    /// 2D, Cube or 2D Array (Unity "Texture Shape").
    pub texture_shape: TextureShape,
    /// Cube: how the image holds the faces.
    pub cubemap_mapping: CubemapMapping,
    /// 2D Array: the grid of layers (left to right, top to bottom).
    pub columns: u32,
    pub rows: u32,
    /// Per-platform overrides (Unity's platform tabs: max size, format, compression).
    pub platform_settings: Vec<TexturePlatformSettings>,
}

impl Default for TextureImporter {
    fn default() -> Self {
        Self {
            texture_type: TextureImporterType::Default,
            srgb: true,
            max_size: 2048,
            generate_mip_maps: true,
            wrap_mode: TextureWrapMode::Repeat,
            filter_mode: FilterMode::Trilinear,
            aniso_level: 8,
            compression: TextureCompression::NormalQuality,
            sprite_mode: crate::sprite::SpriteImportMode::Single,
            pixels_per_unit: 100.0,
            sprite_pivot: oxide_math::Vector2::new(0.5, 0.5),
            sprite_border: oxide_math::Vector4::ZERO,
            sprites: Vec::new(),
            texture_shape: TextureShape::Texture2D,
            cubemap_mapping: CubemapMapping::Auto,
            columns: 1,
            rows: 1,
            platform_settings: Vec::new(),
        }
    }
}

impl TextureImporter {
    /// The settings a platform imports with: its override, else the defaults.
    pub fn platform(&self, target: BuildTarget) -> TexturePlatformSettings {
        self.platform_settings
            .iter()
            .find(|p| p.platform == target && p.overridden)
            .cloned()
            .unwrap_or(TexturePlatformSettings {
                platform: target,
                overridden: false,
                max_texture_size: self.max_size,
                format: TextureImporterFormat::Automatic,
                compression: self.compression,
            })
    }

    /// Max size and GPU format for a platform (`None`: uncompressed RGBA8).
    pub fn resolve(&self, target: BuildTarget, has_alpha: bool) -> (u32, Option<CompressedFormat>) {
        use CompressedFormat as C;
        use TextureImporterFormat as F;
        let p = self.platform(target);
        let normal = self.texture_type == TextureImporterType::NormalMap;
        let format = match p.format {
            F::Automatic => match (target, p.compression) {
                (_, TextureCompression::None) => None,
                // Unity's mobile default: ASTC, block size by quality.
                (BuildTarget::Android | BuildTarget::IOS, q) => Some(match q {
                    TextureCompression::LowQuality => C::Astc8x8,
                    TextureCompression::HighQuality => C::Astc4x4,
                    _ if normal => C::Astc4x4,
                    _ => C::Astc6x6,
                }),
                (_, q) => crate::texture_compress::choose_format(q, self.texture_type, has_alpha),
            },
            F::RGBA32 => None,
            F::DXT1 => Some(C::Bc1),
            F::DXT5 => Some(C::Bc3),
            F::BC4 => Some(C::Bc4),
            F::BC5 => Some(C::Bc5),
            F::BC7 => Some(C::Bc7),
            F::Etc2Rgb4 => Some(C::Etc2Rgb8),
            F::Etc2Rgba8 => Some(C::Etc2Rgba8),
            F::EacR => Some(C::EacR11),
            F::EacRg => Some(C::EacRg11),
            F::Astc4x4 => Some(C::Astc4x4),
            F::Astc6x6 => Some(C::Astc6x6),
            F::Astc8x8 => Some(C::Astc8x8),
        };
        (p.max_texture_size.max(1), format)
    }

    /// Defaults for a file, guessing data maps from its name like before (normal maps are
    /// Normal Map type; masks, roughness, metallic and height are linear).
    pub fn guess(path: &str) -> Self {
        let lower = path.to_ascii_lowercase();
        let normal = ["normal", "_n.", "_nrm", "_norm"]
            .iter()
            .any(|k| lower.contains(k));
        let linear = ["mask", "rough", "metal", "_orm", "height"]
            .iter()
            .any(|k| lower.contains(k));
        Self {
            texture_type: if normal {
                TextureImporterType::NormalMap
            } else {
                TextureImporterType::Default
            },
            srgb: !(normal || linear),
            ..Default::default()
        }
    }
}

#[derive(Clone, Debug)]
pub struct Texture2D {
    pub name: String,
    pub width: u32,
    pub height: u32,
    /// Mip levels, largest first; each is tightly packed RGBA8, top row first. Empty for
    /// textures loaded already block-compressed (KTX2): those have no CPU pixel copy.
    pub mips: Vec<Vec<u8>>,
    /// Color data (sRGB) vs. data maps such as normals or masks (linear).
    pub srgb: bool,
    pub wrap_mode: TextureWrapMode,
    pub filter_mode: FilterMode,
    pub aniso_level: u8,
    /// Block-compressed copy for the GPU (the RGBA8 `mips` stay for CPU readers).
    pub compressed: Option<CompressedMips>,
}

impl Asset for Texture2D {
    const TYPE_NAME: &'static str = "Texture2D";
}

impl Texture2D {
    /// From RGBA8 pixels (top row first); generates mipmaps.
    pub fn from_rgba8(name: &str, width: u32, height: u32, rgba: Vec<u8>, srgb: bool) -> Self {
        assert_eq!(
            rgba.len(),
            (width * height * 4) as usize,
            "pixel buffer size"
        );
        let mut t = Self {
            name: name.into(),
            width,
            height,
            mips: vec![rgba],
            srgb,
            wrap_mode: TextureWrapMode::Repeat,
            filter_mode: FilterMode::Trilinear,
            aniso_level: 8,
            compressed: None,
        };
        t.generate_mips();
        t
    }

    /// 1×1 texture of one color (Unity's `Texture2D.whiteTexture` etc.).
    pub fn solid(name: &str, rgba: [u8; 4], srgb: bool) -> Self {
        Self::from_rgba8(name, 1, 1, rgba.to_vec(), srgb)
    }

    /// Decodes PNG/JPEG/TGA/BMP/HDR bytes.
    pub fn decode(name: &str, bytes: &[u8], srgb: bool) -> Result<Self, String> {
        if bytes.starts_with(b"8BPS") {
            let (w, h, px) = crate::psd::decode(bytes).map_err(|e| format!("{name}: {e}"))?;
            return Ok(Self::from_rgba8(name, w, h, px, srgb));
        }
        let img = image::load_from_memory(bytes).map_err(|e| format!("{name}: {e}"))?;
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        Ok(Self::from_rgba8(name, w, h, rgba.into_raw(), srgb))
    }

    /// Scales the largest side down to `max` (box filter), keeping the aspect ratio.
    pub fn limit_size(&mut self, max: u32) {
        let max = max.max(1);
        while self.width.max(self.height) > max && self.mips.len() > 1 {
            self.mips.remove(0);
            self.width = (self.width / 2).max(1);
            self.height = (self.height / 2).max(1);
        }
    }

    pub fn mip_count(&self) -> u32 {
        self.mips.len() as u32
    }

    /// Box-filtered mip chain (averaged in linear space for sRGB textures).
    pub fn generate_mips(&mut self) {
        self.mips.truncate(1);
        let (mut w, mut h) = (self.width, self.height);
        let to_lin: Vec<f32> = (0..256)
            .map(|i| {
                let c = i as f32 / 255.0;
                if self.srgb {
                    if c <= 0.04045 {
                        c / 12.92
                    } else {
                        ((c + 0.055) / 1.055).powf(2.4)
                    }
                } else {
                    c
                }
            })
            .collect();
        let srgb = self.srgb;
        let to_byte = |v: f32| -> u8 {
            let c = if srgb {
                if v <= 0.0031308 {
                    v * 12.92
                } else {
                    1.055 * v.powf(1.0 / 2.4) - 0.055
                }
            } else {
                v
            };
            (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
        };
        while w > 1 || h > 1 {
            let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
            let src = self.mips.last().expect("level 0");
            let mut dst = vec![0u8; (nw * nh * 4) as usize];
            for y in 0..nh {
                for x in 0..nw {
                    let mut acc = [0f32; 4];
                    let mut n = 0.0;
                    for dy in 0..2 {
                        for dx in 0..2 {
                            let sx = (x * 2 + dx).min(w - 1);
                            let sy = (y * 2 + dy).min(h - 1);
                            let i = ((sy * w + sx) * 4) as usize;
                            for c in 0..3 {
                                acc[c] += to_lin[src[i + c] as usize];
                            }
                            acc[3] += src[i + 3] as f32 / 255.0;
                            n += 1.0;
                        }
                    }
                    let o = ((y * nw + x) * 4) as usize;
                    for c in 0..3 {
                        dst[o + c] = to_byte(acc[c] / n);
                    }
                    dst[o + 3] = ((acc[3] / n).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                }
            }
            self.mips.push(dst);
            w = nw;
            h = nh;
        }
    }

    /// A procedural checkerboard (handy for prototyping, like ProBuilder's grid material).
    pub fn checker(name: &str, size: u32, cells: u32, a: [u8; 4], b: [u8; 4]) -> Self {
        let mut px = Vec::with_capacity((size * size * 4) as usize);
        let cell = (size / cells).max(1);
        for y in 0..size {
            for x in 0..size {
                let c = if ((x / cell) + (y / cell)).is_multiple_of(2) {
                    a
                } else {
                    b
                };
                px.extend_from_slice(&c);
            }
        }
        Self::from_rgba8(name, size, size, px, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mip_chain() {
        let t = Texture2D::checker("c", 64, 8, [255; 4], [0, 0, 0, 255]);
        assert_eq!(t.mip_count(), 7);
        assert_eq!(t.mips.last().unwrap().len(), 4);
        let t = Texture2D::from_rgba8("r", 5, 3, vec![128; 60], false);
        assert_eq!(t.mip_count(), 3); // 5x3, 2x1, 1x1
    }
}
