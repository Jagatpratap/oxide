//! Unity `Cubemap` and `Texture2DArray` assets, made by the Texture Importer's "Texture
//! Shape" (Cube: six faces from a cross, a strip or a latitude-longitude panorama; 2D
//! Array: layers cut from a grid of cells).
//! References: <https://docs.unity3d.com/Manual/class-Cubemap.html>,
//! <https://docs.unity3d.com/Manual/class-Texture2DArray.html>

use crate::handle::Asset;
use crate::texture::{CubemapMapping, Texture2D};

/// Cube faces in Unity's (and the GPU's) order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CubemapFace {
    PositiveX,
    NegativeX,
    PositiveY,
    NegativeY,
    PositiveZ,
    NegativeZ,
}

impl CubemapFace {
    pub const ALL: [CubemapFace; 6] = [
        CubemapFace::PositiveX,
        CubemapFace::NegativeX,
        CubemapFace::PositiveY,
        CubemapFace::NegativeY,
        CubemapFace::PositiveZ,
        CubemapFace::NegativeZ,
    ];

    /// World direction through texel `(u, v)` (0..1, v down) of this face: the D3D / Vulkan /
    /// Metal convention, which matches Unity's left-handed world.
    pub fn direction(self, u: f32, v: f32) -> [f32; 3] {
        let (a, b) = (u * 2.0 - 1.0, v * 2.0 - 1.0);
        let d = match self {
            CubemapFace::PositiveX => [1.0, -b, -a],
            CubemapFace::NegativeX => [-1.0, -b, a],
            CubemapFace::PositiveY => [a, 1.0, b],
            CubemapFace::NegativeY => [a, -1.0, -b],
            CubemapFace::PositiveZ => [a, -b, 1.0],
            CubemapFace::NegativeZ => [-a, -b, -1.0],
        };
        let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        [d[0] / l, d[1] / l, d[2] / l]
    }
}

/// Unity `Cubemap`: six square faces (each a texture with its mips and optional GPU
/// compression), in [`CubemapFace`] order.
#[derive(Clone, Debug)]
pub struct TextureCube {
    pub name: String,
    pub faces: Vec<Texture2D>,
}

impl Asset for TextureCube {
    const TYPE_NAME: &'static str = "Cubemap";
}

impl TextureCube {
    /// Face size in pixels.
    pub fn size(&self) -> u32 {
        self.faces.first().map_or(0, |f| f.width)
    }

    /// A cubemap with six solid faces.
    pub fn solid(name: &str, rgba: [u8; 4], srgb: bool) -> Self {
        Self {
            name: name.into(),
            faces: (0..6).map(|_| Texture2D::solid(name, rgba, srgb)).collect(),
        }
    }
}

/// Unity `Texture2DArray`: same-size layers sampled by index.
#[derive(Clone, Debug)]
pub struct Texture2DArray {
    pub name: String,
    pub layers: Vec<Texture2D>,
}

impl Asset for Texture2DArray {
    const TYPE_NAME: &'static str = "Texture2DArray";
}

impl Texture2DArray {
    pub fn depth(&self) -> u32 {
        self.layers.len() as u32
    }
}

/// How a cube image is laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CubeLayout {
    /// 4×3 cells: `-X +Z +X -Z` in the middle row, `+Y` above and `-Y` below `+Z`.
    HorizontalCross,
    /// 3×4 cells: `+Y`, then `-X +Z +X`, `-Y`, and `-Z` rotated 180° at the bottom.
    VerticalCross,
    /// Six faces left to right in face order.
    HorizontalStrip,
    /// Six faces top to bottom in face order.
    VerticalStrip,
    /// 2:1 equirectangular panorama.
    LatLong,
}

/// The layout of a `w`×`h` image for a mapping (Unity's Auto picks by aspect ratio).
pub fn cube_layout(w: u32, h: u32, mapping: CubemapMapping) -> Result<CubeLayout, String> {
    let six = match (w, h) {
        _ if w * 3 == h * 4 => Some(CubeLayout::HorizontalCross),
        _ if w * 4 == h * 3 => Some(CubeLayout::VerticalCross),
        _ if w == h * 6 => Some(CubeLayout::HorizontalStrip),
        _ if h == w * 6 => Some(CubeLayout::VerticalStrip),
        _ => None,
    };
    match mapping {
        CubemapMapping::LatitudeLongitude => Ok(CubeLayout::LatLong),
        CubemapMapping::SixFramesLayout => six.ok_or_else(|| {
            format!("{w}×{h} isn't a 6-frame layout (4:3 or 3:4 cross, 6:1 or 1:6 strip)")
        }),
        CubemapMapping::Auto => Ok(six.unwrap_or(CubeLayout::LatLong)),
    }
}

/// The six faces (RGBA8, `size`×`size` each) of a cube image.
pub fn cube_faces(
    rgba: &[u8],
    w: u32,
    h: u32,
    mapping: CubemapMapping,
) -> Result<(u32, Vec<Vec<u8>>), String> {
    let layout = cube_layout(w, h, mapping)?;
    let cell = |cx: u32, cy: u32, size: u32, rotate: bool| -> Vec<u8> {
        let mut out = Vec::with_capacity((size * size * 4) as usize);
        for y in 0..size {
            for x in 0..size {
                let (sx, sy) = if rotate {
                    (size - 1 - x, size - 1 - y)
                } else {
                    (x, y)
                };
                let i = (((cy * size + sy) * w + cx * size + sx) * 4) as usize;
                out.extend_from_slice(&rgba[i..i + 4]);
            }
        }
        out
    };
    let faces = match layout {
        CubeLayout::HorizontalCross => {
            let s = w / 4;
            // +X, -X, +Y, -Y, +Z, -Z
            let cells = [(2, 1), (0, 1), (1, 0), (1, 2), (1, 1), (3, 1)];
            (
                s,
                cells.iter().map(|&(x, y)| cell(x, y, s, false)).collect(),
            )
        }
        CubeLayout::VerticalCross => {
            let s = w / 3;
            let cells = [(2, 1), (0, 1), (1, 0), (1, 2), (1, 1)];
            let mut f: Vec<Vec<u8>> = cells.iter().map(|&(x, y)| cell(x, y, s, false)).collect();
            f.push(cell(1, 3, s, true));
            (s, f)
        }
        CubeLayout::HorizontalStrip => {
            let s = h;
            (s, (0..6).map(|i| cell(i, 0, s, false)).collect())
        }
        CubeLayout::VerticalStrip => {
            let s = w;
            (s, (0..6).map(|i| cell(0, i, s, false)).collect())
        }
        CubeLayout::LatLong => {
            // Faces a quarter of the panorama's width, like Unity.
            let s = (w / 4).max(1);
            let faces = CubemapFace::ALL
                .iter()
                .map(|&f| {
                    let mut out = Vec::with_capacity((s * s * 4) as usize);
                    for y in 0..s {
                        for x in 0..s {
                            let d = f.direction(
                                (x as f32 + 0.5) / s as f32,
                                (y as f32 + 0.5) / s as f32,
                            );
                            out.extend_from_slice(&sample_latlong(rgba, w, h, d));
                        }
                    }
                    out
                })
                .collect();
            (s, faces)
        }
    };
    Ok(faces)
}

/// Bilinear sample of an equirectangular image in direction `d` (+Z at the image centre,
/// +Y at the top).
pub fn sample_latlong(rgba: &[u8], w: u32, h: u32, d: [f32; 3]) -> [u8; 4] {
    let lon = d[0].atan2(d[2]);
    let lat = d[1].clamp(-1.0, 1.0).asin();
    let u = 0.5 + lon / std::f32::consts::TAU;
    let v = 0.5 - lat / std::f32::consts::PI;
    let fx = u * w as f32 - 0.5;
    let fy = (v * h as f32 - 0.5).clamp(0.0, h as f32 - 1.0);
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = (fx - x0, fy - y0);
    let px = |x: f32, y: f32| -> [f32; 4] {
        let xi = (x as i64).rem_euclid(w as i64) as u32;
        let yi = (y as i64).clamp(0, h as i64 - 1) as u32;
        let i = ((yi * w + xi) * 4) as usize;
        [
            rgba[i] as f32,
            rgba[i + 1] as f32,
            rgba[i + 2] as f32,
            rgba[i + 3] as f32,
        ]
    };
    let (a, b, c, e) = (
        px(x0, y0),
        px(x0 + 1.0, y0),
        px(x0, y0 + 1.0),
        px(x0 + 1.0, y0 + 1.0),
    );
    let mut out = [0u8; 4];
    for k in 0..4 {
        let top = a[k] + (b[k] - a[k]) * tx;
        let bottom = c[k] + (e[k] - c[k]) * tx;
        out[k] = (top + (bottom - top) * ty).round().clamp(0.0, 255.0) as u8;
    }
    out
}

/// Layers of a 2D Array image: `columns`×`rows` cells, left to right, top to bottom.
pub fn array_layers(
    rgba: &[u8],
    w: u32,
    h: u32,
    columns: u32,
    rows: u32,
) -> Result<(u32, u32, Vec<Vec<u8>>), String> {
    let (columns, rows) = (columns.max(1), rows.max(1));
    if !w.is_multiple_of(columns) || !h.is_multiple_of(rows) {
        return Err(format!(
            "{w}×{h} doesn't divide into {columns}×{rows} equal cells"
        ));
    }
    let (cw, ch) = (w / columns, h / rows);
    let mut layers = Vec::new();
    for r in 0..rows {
        for c in 0..columns {
            let mut out = Vec::with_capacity((cw * ch * 4) as usize);
            for y in 0..ch {
                let i = (((r * ch + y) * w + c * cw) * 4) as usize;
                out.extend_from_slice(&rgba[i..i + (cw * 4) as usize]);
            }
            layers.push(out);
        }
    }
    Ok((cw, ch, layers))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 4×3 cross whose cells are filled with their face index.
    fn cross(s: u32) -> Vec<u8> {
        let (w, h) = (4 * s, 3 * s);
        let cells = [(2, 1), (0, 1), (1, 0), (1, 2), (1, 1), (3, 1)];
        let mut img = vec![0u8; (w * h * 4) as usize];
        for (f, (cx, cy)) in cells.iter().enumerate() {
            for y in 0..s {
                for x in 0..s {
                    let i = (((cy * s + y) * w + cx * s + x) * 4) as usize;
                    img[i..i + 4].copy_from_slice(&[f as u8 * 40, x as u8, y as u8, 255]);
                }
            }
        }
        img
    }

    #[test]
    fn crosses_strips_and_panoramas_make_six_faces() {
        let s = 8;
        let img = cross(s);
        assert_eq!(
            cube_layout(4 * s, 3 * s, CubemapMapping::Auto),
            Ok(CubeLayout::HorizontalCross)
        );
        let (size, faces) = cube_faces(&img, 4 * s, 3 * s, CubemapMapping::Auto).unwrap();
        assert_eq!((size, faces.len()), (s, 6));
        for (f, face) in faces.iter().enumerate() {
            assert_eq!(face[0], f as u8 * 40, "face {f}");
            // Not flipped: texel (3, 5) keeps its coordinates.
            let i = ((5 * s + 3) * 4) as usize;
            assert_eq!((face[i + 1], face[i + 2]), (3, 5));
        }
        assert!(cube_layout(10, 7, CubemapMapping::SixFramesLayout).is_err());
        assert_eq!(
            cube_layout(64, 32, CubemapMapping::Auto),
            Ok(CubeLayout::LatLong)
        );
        // Panorama, left half red and right half blue: the image centre is +Z, so +X (to
        // its right) is blue and -X red.
        let (w, h) = (64u32, 32u32);
        let pano: Vec<u8> = (0..w * h)
            .flat_map(|i| {
                if (i % w) < w / 2 {
                    [255, 0, 0, 255]
                } else {
                    [0, 0, 255, 255]
                }
            })
            .collect();
        let (size, faces) = cube_faces(&pano, w, h, CubemapMapping::Auto).unwrap();
        assert_eq!(size, 16);
        let centre = |f: usize| {
            let i = (((size / 2) * size + size / 2) * 4) as usize;
            faces[f][i..i + 4].to_vec()
        };
        assert_eq!(centre(0), vec![0, 0, 255, 255], "+X");
        assert_eq!(centre(1), vec![255, 0, 0, 255], "-X");
    }

    #[test]
    fn array_cells_become_layers() {
        let (w, h) = (8u32, 4u32);
        let img: Vec<u8> = (0..w * h)
            .flat_map(|i| {
                let (x, y) = (i % w, i / w);
                [(x / 4 + 2 * (y / 2)) as u8, 0, 0, 255]
            })
            .collect();
        let (cw, ch, layers) = array_layers(&img, w, h, 2, 2).unwrap();
        assert_eq!((cw, ch, layers.len()), (4, 2, 4));
        for (i, l) in layers.iter().enumerate() {
            assert!(l.chunks(4).all(|p| p[0] == i as u8), "layer {i}");
        }
        assert!(array_layers(&img, w, h, 3, 1).is_err());
    }
}
