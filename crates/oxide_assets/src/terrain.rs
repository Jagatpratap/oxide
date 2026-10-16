//! Unity `TerrainData`: a heightmap, a splat map blending up to four texture layers, and tree
//! instances. Includes generators for noise terrain, slope/height texturing and procedural
//! layer textures. Reference: <https://docs.unity3d.com/Manual/terrain-UsingTerrains.html>

use crate::handle::{Asset, Handle};
use crate::material::Material;
use crate::mesh::Mesh;
use crate::texture::Texture2D;
use oxide_math::{Color, Vector2, Vector3};
use std::sync::Arc;

/// Unity `TerrainLayer`: one paintable surface texture. Also an asset file
/// (`.terrainlayer`) that Terrains can share.
#[derive(Clone, Debug, PartialEq, oxide_core::Reflect)]
pub struct TerrainLayer {
    pub name: String,
    pub diffuse: Option<Handle<Texture2D>>,
    /// World size of one texture repeat.
    pub tile_size: Vector2,
    pub tint: Color,
    #[reflect(range(0, 1))]
    pub smoothness: f32,
    #[reflect(range(0, 1))]
    pub metallic: f32,
}

impl Default for TerrainLayer {
    fn default() -> Self {
        Self {
            name: String::new(),
            diffuse: None,
            tile_size: Vector2::new(10.0, 10.0),
            tint: Color::WHITE,
            smoothness: 0.0,
            metallic: 0.0,
        }
    }
}

impl Asset for TerrainLayer {
    const TYPE_NAME: &'static str = "TerrainLayer";
}

/// Unity `TreePrototype`: what to draw for each tree instance.
#[derive(Clone, Debug)]
pub struct TreePrototype {
    pub mesh: Handle<Mesh>,
    pub materials: Vec<Handle<Material>>,
}

/// Unity `TreeInstance`: normalized position (x, z in 0..1; y = height fraction, filled on add).
#[derive(Clone, Copy, Debug)]
pub struct TreeInstance {
    pub position: Vector3,
    pub prototype: u32,
    pub width_scale: f32,
    pub height_scale: f32,
    /// Yaw in degrees.
    pub rotation: f32,
}

#[derive(Clone, Debug)]
pub struct TerrainData {
    pub name: String,
    /// Width (x), maximum height (y), length (z) in world units.
    pub size: Vector3,
    /// Heightmap samples per side (Unity: 2^n + 1, e.g. 513).
    pub resolution: u32,
    /// Row-major, rows along +Z, normalized 0..1.
    pub heights: Arc<Vec<f32>>,
    pub splat_resolution: u32,
    /// Row-major RGBA weights for layers 0..3.
    pub splat: Arc<Vec<[u8; 4]>>,
    pub layers: Vec<TerrainLayer>,
    pub tree_prototypes: Vec<TreePrototype>,
    pub trees: Vec<TreeInstance>,
    /// Bumped on every edit so meshes, colliders and GPU data refresh.
    pub version: u64,
}

impl Asset for TerrainData {
    const TYPE_NAME: &'static str = "TerrainData";
}

impl TerrainData {
    /// Flat terrain.
    pub fn flat(size: Vector3, resolution: u32) -> Self {
        let n = resolution.max(2) as usize;
        Self {
            name: "Terrain".into(),
            size,
            resolution: n as u32,
            heights: Arc::new(vec![0.0; n * n]),
            splat_resolution: 256,
            splat: Arc::new(vec![[255, 0, 0, 0]; 256 * 256]),
            layers: Vec::new(),
            tree_prototypes: Vec::new(),
            trees: Vec::new(),
            version: 1,
        }
    }

    /// Normalized height at grid sample `(x, z)`.
    pub fn sample(&self, x: u32, z: u32) -> f32 {
        let n = self.resolution;
        self.heights[(z.min(n - 1) * n + x.min(n - 1)) as usize]
    }

    /// World height (relative to the terrain origin) at normalized coords `u, v` in 0..1
    /// (Unity `TerrainData.GetInterpolatedHeight`).
    pub fn interpolated_height(&self, u: f32, v: f32) -> f32 {
        let n = self.resolution as f32 - 1.0;
        let x = (u.clamp(0.0, 1.0) * n).min(n - 1e-4);
        let z = (v.clamp(0.0, 1.0) * n).min(n - 1e-4);
        let (x0, z0) = (x.floor() as u32, z.floor() as u32);
        let (fx, fz) = (x - x0 as f32, z - z0 as f32);
        let h = |a, b| self.sample(a, b);
        let top = h(x0, z0) * (1.0 - fx) + h(x0 + 1, z0) * fx;
        let bot = h(x0, z0 + 1) * (1.0 - fx) + h(x0 + 1, z0 + 1) * fx;
        (top * (1.0 - fz) + bot * fz) * self.size.y
    }

    /// Surface normal at normalized coords (Unity `GetInterpolatedNormal`).
    pub fn interpolated_normal(&self, u: f32, v: f32) -> Vector3 {
        let cx = self.size.x / (self.resolution - 1) as f32;
        let cz = self.size.z / (self.resolution - 1) as f32;
        let du = 1.0 / (self.resolution - 1) as f32;
        let hl = self.interpolated_height(u - du, v);
        let hr = self.interpolated_height(u + du, v);
        let hd = self.interpolated_height(u, v - du);
        let hu = self.interpolated_height(u, v + du);
        Vector3::new((hl - hr) / (2.0 * cx), 1.0, (hd - hu) / (2.0 * cz)).normalize()
    }

    /// Steepness in degrees at normalized coords.
    pub fn steepness(&self, u: f32, v: f32) -> f32 {
        self.interpolated_normal(u, v)
            .y
            .clamp(-1.0, 1.0)
            .acos()
            .to_degrees()
    }

    pub fn set_heights(&mut self, heights: Vec<f32>) {
        assert_eq!(heights.len(), (self.resolution * self.resolution) as usize);
        self.heights = Arc::new(heights);
        self.version += 1;
    }

    /// Fractal (fBm) value-noise terrain with smooth valleys and mountains.
    pub fn generate(&mut self, seed: u32, octaves: u32, feature_size: f32, mountains: f32) {
        let n = self.resolution as usize;
        let mut h = vec![0.0f32; n * n];
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for z in 0..n {
            for x in 0..n {
                let wx = x as f32 / (n - 1) as f32 * self.size.x;
                let wz = z as f32 / (n - 1) as f32 * self.size.z;
                let mut amp = 1.0;
                let mut freq = 1.0 / feature_size.max(1.0);
                let mut v = 0.0;
                for o in 0..octaves.max(1) {
                    v += amp * value_noise(wx * freq, wz * freq, seed.wrapping_add(o * 7919));
                    amp *= 0.5;
                    freq *= 2.0;
                }
                // Ridged mountains where a low-frequency mask is high.
                let mask = value_noise(
                    wx / (feature_size * 4.0),
                    wz / (feature_size * 4.0),
                    seed ^ 0xabcd,
                ) * 0.5
                    + 0.5;
                let ridge = 1.0
                    - value_noise(
                        wx * 2.0 / feature_size,
                        wz * 2.0 / feature_size,
                        seed ^ 0x1234,
                    )
                    .abs();
                v += mountains * mask * mask * ridge * ridge;
                h[z * n + x] = v;
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
        let span = (hi - lo).max(1e-6);
        for v in &mut h {
            *v = (*v - lo) / span;
        }
        self.set_heights(h);
    }

    /// Flattens a circular area to `height` (normalized) with a smooth falloff — for
    /// airfields, towns, race tracks.
    pub fn flatten(
        &mut self,
        center_u: f32,
        center_v: f32,
        radius: f32,
        falloff: f32,
        height: f32,
    ) {
        let n = self.resolution as usize;
        let mut h = (*self.heights).clone();
        for z in 0..n {
            for x in 0..n {
                let u = x as f32 / (n - 1) as f32;
                let v = z as f32 / (n - 1) as f32;
                let d = Vector2::new((u - center_u) * self.size.x, (v - center_v) * self.size.z)
                    .length();
                let t = ((d - radius) / falloff.max(1e-3)).clamp(0.0, 1.0);
                let t = t * t * (3.0 - 2.0 * t);
                h[z * n + x] = height + (h[z * n + x] - height) * t;
            }
        }
        self.set_heights(h);
    }

    /// Paints the splat map from height and slope: layer 0 low/flat (grass), 1 steep (rock),
    /// 2 near the bottom (sand/shore), 3 high (snow/dirt).
    pub fn auto_splat(&mut self, shore: f32, snow: f32, rock_angle: f32) {
        let s = self.splat_resolution as usize;
        let mut out = vec![[0u8; 4]; s * s];
        for z in 0..s {
            for x in 0..s {
                let u = x as f32 / (s - 1) as f32;
                let v = z as f32 / (s - 1) as f32;
                let h = self.interpolated_height(u, v) / self.size.y.max(1e-4);
                let steep = self.steepness(u, v);
                let n = value_noise(u * 40.0, v * 40.0, 77) * 0.08;
                let rock = ((steep - rock_angle + 8.0) / 16.0).clamp(0.0, 1.0);
                let sand = (1.0 - ((h - shore + n) / 0.03).clamp(0.0, 1.0)) * (1.0 - rock);
                let high = ((h - snow + n) / 0.05).clamp(0.0, 1.0) * (1.0 - rock);
                let grass = (1.0 - rock - sand - high).max(0.0);
                let sum = (grass + rock + sand + high).max(1e-4);
                let q = |w: f32| ((w / sum) * 255.0 + 0.5) as u8;
                out[z * s + x] = [q(grass), q(rock), q(sand), q(high)];
            }
        }
        self.splat = Arc::new(out);
        self.version += 1;
    }

    /// Scatters trees of `prototype` where the ground is flat-ish and in a height band.
    pub fn scatter_trees(
        &mut self,
        prototype: u32,
        count: usize,
        seed: u32,
        max_steepness: f32,
        height_range: (f32, f32),
    ) {
        let mut rng = seed.max(1);
        let mut next = || {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            rng as f32 / u32::MAX as f32
        };
        let mut placed = 0;
        let mut tries = 0;
        while placed < count && tries < count * 20 {
            tries += 1;
            let (u, v) = (next(), next());
            let h = self.interpolated_height(u, v) / self.size.y.max(1e-4);
            if self.steepness(u, v) > max_steepness || h < height_range.0 || h > height_range.1 {
                continue;
            }
            // Clumping: keep trees mostly where a noise field is high.
            if value_noise(u * 12.0, v * 12.0, seed ^ 99) < -0.1 {
                continue;
            }
            let s = 0.8 + next() * 0.5;
            self.trees.push(TreeInstance {
                position: Vector3::new(u, h, v),
                prototype,
                width_scale: s,
                height_scale: s * (0.9 + next() * 0.3),
                rotation: next() * 360.0,
            });
            placed += 1;
        }
        self.version += 1;
    }
}

fn hash(x: i32, z: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6b343)
        ^ (z as u32).wrapping_mul(0xd8163841)
        ^ seed.wrapping_mul(0xcb1ab31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1e995);
    h ^= h >> 15;
    (h as f32 / u32::MAX as f32) * 2.0 - 1.0
}

/// Smooth value noise in -1..1.
pub fn value_noise(x: f32, z: f32, seed: u32) -> f32 {
    let (x0, z0) = (x.floor(), z.floor());
    let (fx, fz) = (x - x0, z - z0);
    let (ix, iz) = (x0 as i32, z0 as i32);
    let s = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let (sx, sz) = (s(fx), s(fz));
    let a = hash(ix, iz, seed);
    let b = hash(ix + 1, iz, seed);
    let c = hash(ix, iz + 1, seed);
    let d = hash(ix + 1, iz + 1, seed);
    let top = a + (b - a) * sx;
    let bot = c + (d - c) * sx;
    top + (bot - top) * sz
}

/// Procedural tiling ground texture (`base` color with noise variation and speckles).
pub fn ground_texture(
    name: &str,
    size: u32,
    base: [f32; 3],
    variation: f32,
    seed: u32,
    streaks: bool,
) -> Texture2D {
    let mut px = Vec::with_capacity((size * size * 4) as usize);
    let s = size as f32;
    for y in 0..size {
        for x in 0..size {
            let (u, v) = (x as f32 / s, y as f32 / s);
            // Tileable noise: sample on a torus-like wrap by mixing shifted samples.
            let tile = |f: f32, sd: u32| {
                let a = value_noise(u * f, v * f, sd);
                let b = value_noise((u - 1.0) * f, v * f, sd);
                let c = value_noise(u * f, (v - 1.0) * f, sd);
                let d = value_noise((u - 1.0) * f, (v - 1.0) * f, sd);
                let (wu, wv) = (u, v);
                (a * (1.0 - wu) + b * wu) * (1.0 - wv) + (c * (1.0 - wu) + d * wu) * wv
            };
            let mut n =
                tile(4.0, seed) * 0.5 + tile(16.0, seed + 1) * 0.3 + tile(64.0, seed + 2) * 0.2;
            if streaks {
                n += (tile(2.0, seed + 3) * 8.0 + v * 30.0).sin() * 0.15;
            }
            let k = 1.0 + n * variation;
            for c in base {
                px.push(((c * k).clamp(0.0, 1.0) * 255.0) as u8);
            }
            px.push(255);
        }
    }
    Texture2D::from_rgba8(name, size, size, px, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heights_and_normals() {
        let mut t = TerrainData::flat(Vector3::new(100.0, 50.0, 100.0), 65);
        assert_eq!(t.interpolated_height(0.5, 0.5), 0.0);
        assert!((t.interpolated_normal(0.3, 0.3) - Vector3::Y).length() < 1e-5);
        // A ramp rising along +X.
        let n = 65;
        t.set_heights(
            (0..n * n)
                .map(|i| (i % n) as f32 / (n - 1) as f32)
                .collect(),
        );
        assert!((t.interpolated_height(0.5, 0.2) - 25.0).abs() < 1e-3);
        let nrm = t.interpolated_normal(0.5, 0.5);
        assert!(nrm.x < -0.3 && nrm.y > 0.5, "{nrm}");
        t.generate(1, 5, 30.0, 0.5);
        assert!(t.heights.iter().all(|h| (0.0..=1.0).contains(h)));
        t.auto_splat(0.1, 0.8, 35.0);
        assert!(
            t.splat
                .iter()
                .all(|w| w.iter().map(|&x| x as u32).sum::<u32>() >= 250)
        );
    }
}
