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

#[derive(Clone, Debug)]
pub struct Texture2D {
    pub name: String,
    pub width: u32,
    pub height: u32,
    /// Mip levels, largest first; each is tightly packed RGBA8, top row first.
    pub mips: Vec<Vec<u8>>,
    /// Color data (sRGB) vs. data maps such as normals or masks (linear).
    pub srgb: bool,
    pub wrap_mode: TextureWrapMode,
    pub filter_mode: FilterMode,
    pub aniso_level: u8,
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
        let img = image::load_from_memory(bytes).map_err(|e| format!("{name}: {e}"))?;
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        Ok(Self::from_rgba8(name, w, h, rgba.into_raw(), srgb))
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
                let c = if ((x / cell) + (y / cell)) % 2 == 0 {
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
