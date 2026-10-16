//! Unity `TerrainData` (`.terraindata`): a heightmap, alphamaps blending any number of
//! Terrain Layers (four per control map), holes, tree prototypes and instances, and detail
//! (grass / detail mesh) prototypes and density layers. Includes Unity's terrain painting
//! operations (raise / lower, set height, smooth, stamp, paint texture, paint holes, paint
//! trees, paint details) as brush strokes, plus generators for noise terrain, slope/height
//! texturing and procedural layer textures.
//! Reference: <https://docs.unity3d.com/Manual/terrain-UsingTerrains.html>

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
    /// Tangent-space normal map.
    pub normal_map: Option<Handle<Texture2D>>,
    #[reflect(range(0, 4))]
    pub normal_scale: f32,
    /// World size of one texture repeat.
    pub tile_size: Vector2,
    pub tile_offset: Vector2,
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
            normal_map: None,
            normal_scale: 1.0,
            tile_size: Vector2::new(10.0, 10.0),
            tile_offset: Vector2::ZERO,
            tint: Color::WHITE,
            smoothness: 0.0,
            metallic: 0.0,
        }
    }
}

impl Asset for TerrainLayer {
    const TYPE_NAME: &'static str = "TerrainLayer";
}

/// Unity `TreePrototype`: what to draw for each tree instance. `source` is the model or
/// prefab it came from (its first mesh and materials are used); built-in trees use
/// `builtin:` meshes.
#[derive(Clone, Debug, PartialEq)]
pub struct TreePrototype {
    pub source: Option<String>,
    pub mesh: Handle<Mesh>,
    pub materials: Vec<Handle<Material>>,
    /// How much wind bends the tree (Unity Bend Factor).
    pub bend_factor: f32,
}

impl TreePrototype {
    pub fn new(mesh: Handle<Mesh>, materials: Vec<Handle<Material>>) -> Self {
        Self {
            source: None,
            mesh,
            materials,
            bend_factor: 0.0,
        }
    }

    pub fn name(&self) -> String {
        let p = self.source.as_deref().unwrap_or(self.mesh.path());
        p.rsplit('/').next().unwrap_or(p).to_owned()
    }
}

/// Unity `TreeInstance`: normalized position (x, z in 0..1; y = height fraction, filled on add).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TreeInstance {
    pub position: Vector3,
    pub prototype: u32,
    pub width_scale: f32,
    pub height_scale: f32,
    /// Yaw in degrees.
    pub rotation: f32,
    /// Tint (Unity `color`, used for colour variation).
    pub color: Color,
}

/// Unity `DetailRenderMode`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DetailRenderMode {
    /// Two crossed quads with a grass texture, bending in the wind.
    #[default]
    Grass,
    /// One camera-facing quad.
    GrassBillboard,
    /// A detail mesh (rocks, flowers), lit like other meshes.
    VertexLit,
}

/// Unity `DetailPrototype`: a grass texture or detail mesh scattered by a density layer.
#[derive(Clone, Debug, PartialEq)]
pub struct DetailPrototype {
    pub texture: Option<Handle<Texture2D>>,
    /// Model or mesh for `VertexLit` details.
    pub mesh: Option<Handle<Mesh>>,
    pub materials: Vec<Handle<Material>>,
    pub source: Option<String>,
    pub render_mode: DetailRenderMode,
    pub min_width: f32,
    pub max_width: f32,
    pub min_height: f32,
    pub max_height: f32,
    /// Size of the noise pattern that varies size and colour (world units).
    pub noise_spread: f32,
    pub healthy_color: Color,
    pub dry_color: Color,
    /// Instances per density unit (Unity Density).
    pub density: f32,
    pub align_to_ground: f32,
}

impl Default for DetailPrototype {
    fn default() -> Self {
        Self {
            texture: None,
            mesh: None,
            materials: Vec::new(),
            source: None,
            render_mode: DetailRenderMode::Grass,
            min_width: 1.0,
            max_width: 2.0,
            min_height: 1.0,
            max_height: 2.0,
            noise_spread: 10.0,
            healthy_color: Color::rgb(0.26, 0.98, 0.16),
            dry_color: Color::rgb(0.80, 0.74, 0.10),
            density: 1.0,
            align_to_ground: 0.0,
        }
    }
}

impl DetailPrototype {
    pub fn name(&self) -> String {
        let p = self
            .source
            .as_deref()
            .or(self.texture.as_ref().map(Handle::path))
            .or(self.mesh.as_ref().map(Handle::path))
            .unwrap_or("Detail");
        p.rsplit('/').next().unwrap_or(p).to_owned()
    }
}

/// Unity's terrain wind settings for grass (Wind Speed, Size, Bending, Grass Tint).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindSettings {
    pub speed: f32,
    pub size: f32,
    pub bending: f32,
    pub grass_tint: Color,
}

impl Default for WindSettings {
    fn default() -> Self {
        Self {
            speed: 0.5,
            size: 0.5,
            bending: 0.5,
            grass_tint: Color::rgb(0.7, 0.7, 0.7),
        }
    }
}

/// What changed in a TerrainData (so meshes, colliders and GPU data refresh only what they
/// must). Every edit also bumps `TerrainData::version`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TerrainVersions {
    pub heights: u64,
    pub alphamaps: u64,
    pub layers: u64,
    pub holes: u64,
    pub trees: u64,
    pub details: u64,
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
    /// Control (splat) map resolution (Unity alphamap resolution).
    pub splat_resolution: u32,
    /// One RGBA control map per four layers: map `i` holds the weights of layers
    /// `4i..4i+3`. Row-major, rows along +Z.
    pub alphamaps: Vec<Arc<Vec<[u8; 4]>>>,
    pub layers: Vec<TerrainLayer>,
    /// The `.terrainlayer` file each layer came from (saved instead of a copy).
    pub layer_assets: Vec<Option<String>>,
    /// One byte per heightmap cell ((resolution - 1)²): 0 = hole. Empty = no holes.
    pub holes: Arc<Vec<u8>>,
    pub tree_prototypes: Vec<TreePrototype>,
    pub trees: Vec<TreeInstance>,
    pub detail_prototypes: Vec<DetailPrototype>,
    /// Detail density map resolution (cells per side).
    pub detail_resolution: u32,
    /// Per detail prototype: density 0..16 per cell, row-major along +Z.
    pub details: Vec<Arc<Vec<u8>>>,
    pub wind: WindSettings,
    /// Bumped on every edit so meshes, colliders and GPU data refresh.
    pub version: u64,
    pub versions: TerrainVersions,
    /// Heightmap rectangles edited since the terrain system last rebuilt meshes
    /// ([x0, z0, x1, z1] in samples, inclusive). Empty with a new heights version = all.
    pub dirty_heights: Vec<[u32; 4]>,
    /// While true (during a brush stroke) the collider keeps its old shape; it updates
    /// when the stroke ends (Unity's delayed heightmap sync).
    pub hold_collider: bool,
}

impl Asset for TerrainData {
    const TYPE_NAME: &'static str = "TerrainData";
}

/// Unity's brush shapes (the built-in brushes of the terrain tools).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BrushShape {
    /// Smooth falloff to the edge.
    #[default]
    Soft,
    /// Full strength to a slightly softened edge.
    Hard,
    /// Linear (cone) falloff.
    Linear,
    /// Soft circle broken up by noise.
    Noise,
    /// Soft square.
    Square,
    /// Five-point star.
    Star,
}

impl BrushShape {
    pub const ALL: [Self; 6] = [
        Self::Soft,
        Self::Hard,
        Self::Linear,
        Self::Noise,
        Self::Square,
        Self::Star,
    ];
}

/// A terrain brush: shape, world-space size (diameter), strength and rotation.
#[derive(Clone, Debug, PartialEq)]
pub struct Brush {
    pub shape: BrushShape,
    /// A custom brush texture (alpha, else luminance); overrides `shape` when set.
    pub texture: Option<Arc<(u32, u32, Vec<u8>)>>,
    pub size: f32,
    pub strength: f32,
    /// Degrees.
    pub rotation: f32,
}

impl Default for Brush {
    fn default() -> Self {
        Self {
            shape: BrushShape::Soft,
            texture: None,
            size: 25.0,
            strength: 0.5,
            rotation: 0.0,
        }
    }
}

impl Brush {
    /// The brush value at `(x, y)` in brush space (-1..1 across the brush).
    pub fn weight(&self, x: f32, y: f32) -> f32 {
        let (s, c) = self.rotation.to_radians().sin_cos();
        let (x, y) = (x * c - y * s, x * s + y * c);
        let r = (x * x + y * y).sqrt();
        if let Some(t) = &self.texture {
            let (w, h, px) = (t.0, t.1, &t.2);
            if !(-1.0..=1.0).contains(&x) || !(-1.0..=1.0).contains(&y) || w == 0 || h == 0 {
                return 0.0;
            }
            let tx = (((x * 0.5 + 0.5) * w as f32) as u32).min(w - 1);
            let ty = (((0.5 - y * 0.5) * h as f32) as u32).min(h - 1);
            let i = ((ty * w + tx) * 4) as usize;
            let a = px[i + 3] as f32 / 255.0;
            let lum = (px[i] as f32 + px[i + 1] as f32 + px[i + 2] as f32) / (3.0 * 255.0);
            // Opaque textures use their brightness; others their alpha.
            return if px.chunks(4).all(|p| p[3] == 255) {
                lum
            } else {
                a
            };
        }
        let smooth = |t: f32| {
            let t = t.clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        match self.shape {
            BrushShape::Soft => {
                if r >= 1.0 {
                    0.0
                } else {
                    smooth(1.0 - r)
                }
            }
            BrushShape::Hard => smooth((1.0 - r) / 0.1),
            BrushShape::Linear => (1.0 - r).max(0.0),
            BrushShape::Noise => {
                if r >= 1.0 {
                    0.0
                } else {
                    let n = value_noise(x * 6.0 + 17.0, y * 6.0 - 3.0, 9) * 0.5 + 0.5;
                    smooth(1.0 - r) * smooth((n - 0.25) * 2.0)
                }
            }
            BrushShape::Square => smooth((1.0 - x.abs().max(y.abs())) / 0.25),
            BrushShape::Star => {
                let a = y.atan2(x);
                let star = 0.55 + 0.45 * (a * 5.0).cos().max(0.0).powf(1.5);
                smooth((star - r) / 0.15)
            }
        }
    }

    /// Visits every sample of a `res`×`res` grid covering `size` under the brush centred on
    /// normalized `(u, v)`: (x, z, weight).
    fn cells(&self, u: f32, v: f32, size: Vector3, res: u32, mut f: impl FnMut(u32, u32, f32)) {
        if res < 2 {
            return;
        }
        let n = (res - 1) as f32;
        let rx = self.size * 0.5 / size.x.max(1e-4) * n;
        let rz = self.size * 0.5 / size.z.max(1e-4) * n;
        let (cx, cz) = (u * n, v * n);
        let x0 = (cx - rx).floor().max(0.0) as u32;
        let x1 = ((cx + rx).ceil() as i64).clamp(0, res as i64 - 1) as u32;
        let z0 = (cz - rz).floor().max(0.0) as u32;
        let z1 = ((cz + rz).ceil() as i64).clamp(0, res as i64 - 1) as u32;
        if cx + rx < 0.0 || cz + rz < 0.0 || cx - rx > n || cz - rz > n {
            return;
        }
        for z in z0..=z1 {
            for x in x0..=x1 {
                let w = self.weight(
                    (x as f32 - cx) / rx.max(1e-4),
                    (z as f32 - cz) / rz.max(1e-4),
                );
                if w > 0.0 {
                    f(x, z, w);
                }
            }
        }
    }

    /// The sample rectangle a stroke touches ([x0, z0, x1, z1]).
    fn rect(&self, u: f32, v: f32, size: Vector3, res: u32) -> [u32; 4] {
        let n = (res - 1) as f32;
        let rx = self.size * 0.5 / size.x.max(1e-4) * n;
        let rz = self.size * 0.5 / size.z.max(1e-4) * n;
        let c = |a: f32| (a.max(0.0) as u32).min(res - 1);
        [
            c((u * n - rx).floor()),
            c((v * n - rz).floor()),
            c((u * n + rx).ceil()),
            c((v * n + rz).ceil()),
        ]
    }
}

/// Unity's Paint Terrain height tools.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HeightOp {
    /// Raise (or lower with a negative strength); `strength` is metres per stroke step at
    /// full brush weight.
    Raise(f32),
    /// Move toward a height (metres above the terrain origin).
    SetHeight(f32),
    /// Average with the neighbours.
    Smooth,
    /// Stamp: raise to at least `height` metres times the brush shape (Unity Stamp
    /// Terrain); `max_add` adds instead of taking the maximum.
    Stamp { height: f32, add: bool },
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
            alphamaps: vec![Arc::new(vec![[255, 0, 0, 0]; 256 * 256])],
            layers: Vec::new(),
            layer_assets: Vec::new(),
            holes: Arc::new(Vec::new()),
            tree_prototypes: Vec::new(),
            trees: Vec::new(),
            detail_prototypes: Vec::new(),
            detail_resolution: 512,
            details: Vec::new(),
            wind: WindSettings::default(),
            version: 1,
            versions: TerrainVersions::default(),
            dirty_heights: Vec::new(),
            hold_collider: false,
        }
    }

    /// Marks heights changed in a rectangle (or everywhere with `None`).
    pub fn touch_heights(&mut self, rect: Option<[u32; 4]>) {
        match rect {
            Some(r) => self.dirty_heights.push(r),
            None => self.dirty_heights.clear(),
        }
        self.versions.heights += 1;
        self.version += 1;
    }

    pub fn touch_alphamaps(&mut self) {
        self.versions.alphamaps += 1;
        self.version += 1;
    }

    pub fn touch_layers(&mut self) {
        self.versions.layers += 1;
        self.versions.alphamaps += 1;
        self.version += 1;
    }

    pub fn touch_holes(&mut self) {
        self.versions.holes += 1;
        self.dirty_heights.clear();
        self.versions.heights += 1;
        self.version += 1;
    }

    pub fn touch_trees(&mut self) {
        self.versions.trees += 1;
        self.version += 1;
    }

    pub fn touch_details(&mut self) {
        self.versions.details += 1;
        self.version += 1;
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
        self.touch_heights(None);
    }

    /// Unity `GetHeights(x, z, w, h)`: a block of normalized heights, row-major.
    pub fn get_heights(&self, x: u32, z: u32, w: u32, h: u32) -> Vec<f32> {
        let mut out = Vec::with_capacity((w * h) as usize);
        for zz in z..z + h {
            for xx in x..x + w {
                out.push(self.sample(xx, zz));
            }
        }
        out
    }

    /// Unity `SetHeights(x, z, heights)`: writes a `w`-wide block of normalized heights.
    pub fn set_heights_region(&mut self, x: u32, z: u32, w: u32, block: &[f32]) {
        if w == 0 {
            return;
        }
        let n = self.resolution;
        let h = block.len() as u32 / w;
        let heights = Arc::make_mut(&mut self.heights);
        for j in 0..h {
            for i in 0..w {
                let (xx, zz) = (x + i, z + j);
                if xx < n && zz < n {
                    heights[(zz * n + xx) as usize] = block[(j * w + i) as usize].clamp(0.0, 1.0);
                }
            }
        }
        self.touch_heights(Some([
            x.min(n - 1),
            z.min(n - 1),
            (x + w).min(n) - 1,
            (z + h.max(1)).min(n) - 1,
        ]));
    }

    /// Resamples the heightmap to `resolution` (Unity Heightmap Resolution).
    pub fn resize_heightmap(&mut self, resolution: u32) {
        let resolution = resolution.max(2);
        if resolution == self.resolution {
            return;
        }
        let m = (resolution - 1) as f32;
        let mut h = Vec::with_capacity((resolution * resolution) as usize);
        for z in 0..resolution {
            for x in 0..resolution {
                h.push(
                    self.interpolated_height(x as f32 / m, z as f32 / m) / self.size.y.max(1e-6),
                );
            }
        }
        let old_holes = std::mem::take(Arc::make_mut(&mut self.holes));
        let old_res = self.resolution;
        self.resolution = resolution;
        self.heights = Arc::new(h);
        if !old_holes.is_empty() {
            let (oc, nc) = (old_res - 1, resolution - 1);
            let holes = (0..nc * nc)
                .map(|i| {
                    let (x, z) = (i % nc, i / nc);
                    old_holes[((z * oc / nc) * oc + x * oc / nc) as usize]
                })
                .collect();
            self.holes = Arc::new(holes);
        }
        self.touch_holes();
    }

    // ---- alphamaps ---------------------------------------------------------------------

    /// Number of control maps the layers need.
    pub fn alphamap_count(&self) -> usize {
        self.layers.len().max(1).div_ceil(4)
    }

    /// Makes `alphamaps` match the layer count and resolution (new maps start empty; the
    /// first starts fully layer 0).
    pub fn fit_alphamaps(&mut self) {
        let s = (self.splat_resolution * self.splat_resolution) as usize;
        let want = self.alphamap_count();
        if self.alphamaps.len() > want {
            self.alphamaps.truncate(want);
        }
        while self.alphamaps.len() < want {
            let first = self.alphamaps.is_empty();
            self.alphamaps.push(Arc::new(vec![
                if first { [255, 0, 0, 0] } else { [0; 4] };
                s
            ]));
        }
        // Maps of another resolution are resampled (nearest).
        let res = self.splat_resolution as usize;
        for m in &mut self.alphamaps {
            if m.len() != s {
                let old = (m.len() as f64).sqrt() as usize;
                *m = Arc::new(if old * old == m.len() && old > 0 {
                    (0..s)
                        .map(|i| {
                            let (x, z) = (i % res, i / res);
                            m[(z * old / res) * old + x * old / res]
                        })
                        .collect()
                } else {
                    vec![[0; 4]; s]
                });
            }
        }
    }

    /// Weight (0..1) of `layer` at control map texel `(x, z)`.
    pub fn alpha(&self, layer: usize, x: u32, z: u32) -> f32 {
        let s = self.splat_resolution;
        self.alphamaps
            .get(layer / 4)
            .and_then(|m| m.get((z.min(s - 1) * s + x.min(s - 1)) as usize))
            .map_or(0.0, |px| px[layer % 4] as f32 / 255.0)
    }

    /// Unity `GetAlphamaps` for one texel: every layer's weight.
    pub fn alphas_at(&self, x: u32, z: u32) -> Vec<f32> {
        (0..self.layers.len().max(1))
            .map(|l| self.alpha(l, x, z))
            .collect()
    }

    /// Sets every layer's weight at one texel (normalized to sum to 1).
    pub fn set_alphas_at(&mut self, x: u32, z: u32, weights: &[f32]) {
        let s = self.splat_resolution;
        let i = (z.min(s - 1) * s + x.min(s - 1)) as usize;
        let sum: f32 = weights.iter().sum::<f32>().max(1e-6);
        for (m, map) in self.alphamaps.iter_mut().enumerate() {
            let px = &mut Arc::make_mut(map)[i];
            #[allow(clippy::needless_range_loop)]
            for c in 0..4 {
                let w = weights.get(m * 4 + c).copied().unwrap_or(0.0) / sum;
                px[c] = (w * 255.0 + 0.5) as u8;
            }
        }
    }

    /// Adds a layer (and its control map channel); returns its index.
    pub fn add_layer(&mut self, layer: TerrainLayer, asset: Option<String>) -> usize {
        self.layers.push(layer);
        self.layer_assets.push(asset);
        self.fit_alphamaps();
        self.touch_layers();
        self.layers.len() - 1
    }

    /// Removes a layer; its weight goes to the remaining layers.
    pub fn remove_layer(&mut self, index: usize) {
        if index >= self.layers.len() {
            return;
        }
        let s = self.splat_resolution;
        let n = self.layers.len();
        let mut all: Vec<Vec<f32>> = Vec::with_capacity((s * s) as usize);
        for z in 0..s {
            for x in 0..s {
                let mut w = self.alphas_at(x, z);
                w.resize(n, 0.0);
                w.remove(index);
                all.push(w);
            }
        }
        self.layers.remove(index);
        if index < self.layer_assets.len() {
            self.layer_assets.remove(index);
        }
        self.alphamaps.clear();
        self.fit_alphamaps();
        for z in 0..s {
            for x in 0..s {
                let w = &all[(z * s + x) as usize];
                if w.iter().sum::<f32>() > 0.0 {
                    self.set_alphas_at(x, z, w);
                } else if !w.is_empty() {
                    let mut first = vec![0.0; w.len()];
                    first[0] = 1.0;
                    self.set_alphas_at(x, z, &first);
                }
            }
        }
        self.touch_layers();
    }

    /// Resamples the control maps (Unity Control Texture Resolution).
    pub fn resize_alphamaps(&mut self, resolution: u32) {
        let resolution = resolution.max(16);
        if resolution == self.splat_resolution {
            return;
        }
        let old = self.splat_resolution;
        let maps: Vec<Vec<[u8; 4]>> = self
            .alphamaps
            .iter()
            .map(|m| {
                (0..resolution * resolution)
                    .map(|i| {
                        let (x, z) = (i % resolution, i / resolution);
                        m[((z * old / resolution) * old + x * old / resolution) as usize]
                    })
                    .collect()
            })
            .collect();
        self.splat_resolution = resolution;
        self.alphamaps = maps.into_iter().map(Arc::new).collect();
        self.touch_alphamaps();
    }

    // ---- holes -------------------------------------------------------------------------

    /// Whether heightmap cell `(x, z)` is solid ground (Unity `GetHoles`: true = surface).
    pub fn is_solid(&self, x: u32, z: u32) -> bool {
        let c = self.resolution - 1;
        self.holes.is_empty() || self.holes[(z.min(c - 1) * c + x.min(c - 1)) as usize] != 0
    }

    /// Whether the cell under normalized `(u, v)` is solid.
    pub fn is_solid_at(&self, u: f32, v: f32) -> bool {
        let c = (self.resolution - 1) as f32;
        self.is_solid(
            (u.clamp(0.0, 1.0) * c) as u32,
            (v.clamp(0.0, 1.0) * c) as u32,
        )
    }

    // ---- brush strokes (Unity's terrain tools) ------------------------------------------

    /// One stroke step of a height tool under `brush` at normalized `(u, v)`.
    pub fn paint_height(&mut self, u: f32, v: f32, brush: &Brush, op: HeightOp) {
        let n = self.resolution;
        let sy = self.size.y.max(1e-4);
        let src = self.heights.clone();
        let rect = brush.rect(u, v, self.size, n);
        let mut writes = vec![];
        brush.cells(u, v, self.size, n, |x, z, w| {
            let i = (z * n + x) as usize;
            let h = src[i];
            let k = w * brush.strength;
            let nh = match op {
                HeightOp::Raise(m) => h + m / sy * k,
                HeightOp::SetHeight(target) => h + (target / sy - h) * k.min(1.0),
                HeightOp::Smooth => {
                    let mut sum = 0.0;
                    let mut cnt = 0.0;
                    for dz in -1i64..=1 {
                        for dx in -1i64..=1 {
                            let (xx, zz) = (x as i64 + dx, z as i64 + dz);
                            if (0..n as i64).contains(&xx) && (0..n as i64).contains(&zz) {
                                sum += src[(zz as u32 * n + xx as u32) as usize];
                                cnt += 1.0;
                            }
                        }
                    }
                    h + (sum / cnt - h) * k.min(1.0)
                }
                HeightOp::Stamp { height, add } => {
                    let s = height / sy * w;
                    if add {
                        h + s * brush.strength
                    } else {
                        h.max(s)
                    }
                }
            };
            writes.push((i, nh.clamp(0.0, 1.0)));
        });
        if writes.is_empty() {
            return;
        }
        let heights = Arc::make_mut(&mut self.heights);
        for (i, h) in writes {
            heights[i] = h;
        }
        self.touch_heights(Some(rect));
    }

    /// One stroke step of Paint Texture: moves weight toward `layer` up to `target`.
    pub fn paint_texture(&mut self, u: f32, v: f32, brush: &Brush, layer: usize, target: f32) {
        if layer >= self.layers.len().max(1) {
            return;
        }
        self.fit_alphamaps();
        let s = self.splat_resolution;
        let n = self.layers.len().max(1);
        let mut edits = vec![];
        brush.cells(u, v, self.size, s, |x, z, w| edits.push((x, z, w)));
        for (x, z, w) in edits {
            let mut ws = self.alphas_at(x, z);
            ws.resize(n, 0.0);
            let k = (w * brush.strength).min(1.0);
            let cur = ws[layer];
            let new = cur + (target - cur) * k;
            // Scale the other layers so the weights still sum to 1.
            let others: f32 = ws.iter().sum::<f32>() - cur;
            for (i, wi) in ws.iter_mut().enumerate() {
                if i == layer {
                    *wi = new;
                } else if others > 1e-6 {
                    *wi *= (1.0 - new) / others;
                }
            }
            if others <= 1e-6 && new < 1.0 && n > 1 {
                ws[(layer + 1) % n] = 1.0 - new;
            }
            self.set_alphas_at(x, z, &ws);
        }
        self.touch_alphamaps();
    }

    /// One stroke step of Paint Holes (`erase` fills holes back in).
    pub fn paint_holes(&mut self, u: f32, v: f32, brush: &Brush, erase: bool) {
        let c = self.resolution - 1;
        if self.holes.is_empty() {
            if erase {
                return;
            }
            self.holes = Arc::new(vec![1; (c * c) as usize]);
        }
        let mut cells = vec![];
        // Cells are sampled at their centres on a (c + 1) grid, like the heightmap.
        brush.cells(u, v, self.size, c + 1, |x, z, w| {
            if x < c && z < c && w > 0.5 {
                cells.push((x, z));
            }
        });
        let holes = Arc::make_mut(&mut self.holes);
        for (x, z) in cells {
            holes[(z * c + x) as usize] = u8::from(erase);
        }
        if holes.iter().all(|&h| h != 0) {
            holes.clear();
        }
        self.touch_holes();
    }

    /// One stroke step of Paint Trees: adds instances of `prototype` up to `density` trees
    /// per 100 m² under the brush (or removes them with `erase`). `rng` drives placement.
    #[allow(clippy::too_many_arguments)]
    pub fn paint_trees(
        &mut self,
        u: f32,
        v: f32,
        brush: &Brush,
        prototype: u32,
        density: f32,
        height: (f32, f32),
        width: (f32, f32),
        color_variation: f32,
        random_rotation: bool,
        erase: bool,
        rng: &mut u32,
    ) {
        let r = brush.size * 0.5;
        let (ru, rv) = (r / self.size.x.max(1e-4), r / self.size.z.max(1e-4));
        let inside = |t: &TreeInstance| {
            let dx = (t.position.x - u) / ru;
            let dz = (t.position.z - v) / rv;
            dx * dx + dz * dz <= 1.0
        };
        if erase {
            let before = self.trees.len();
            self.trees
                .retain(|t| !(inside(t) && (t.prototype == prototype || prototype == u32::MAX)));
            if self.trees.len() != before {
                self.touch_trees();
            }
            return;
        }
        let mut next = || {
            *rng ^= *rng << 13;
            *rng ^= *rng >> 17;
            *rng ^= *rng << 5;
            *rng as f32 / u32::MAX as f32
        };
        let area = std::f32::consts::PI * r * r;
        let want = (area / 100.0 * density * brush.strength).ceil() as usize;
        let have = self.trees.iter().filter(|t| inside(t)).count();
        let spacing = (100.0 / density.max(0.01)).sqrt() * 0.5;
        let mut added = 0;
        for _ in 0..want.saturating_sub(have) * 4 {
            if have + added >= want {
                break;
            }
            let a = next() * std::f32::consts::TAU;
            let d = next().sqrt();
            let (pu, pv) = (u + a.cos() * d * ru, v + a.sin() * d * rv);
            if !(0.0..=1.0).contains(&pu) || !(0.0..=1.0).contains(&pv) || !self.is_solid_at(pu, pv)
            {
                continue;
            }
            if brush.weight(a.cos() * d, a.sin() * d) < next() {
                continue;
            }
            let p = Vector2::new(pu * self.size.x, pv * self.size.z);
            let too_close = self.trees.iter().any(|t| {
                Vector2::new(t.position.x * self.size.x, t.position.z * self.size.z).distance(p)
                    < spacing
            });
            if too_close {
                continue;
            }
            let hs = height.0 + (height.1 - height.0) * next();
            let ws = width.0 + (width.1 - width.0) * next();
            let shade = 1.0 - color_variation * next();
            self.trees.push(TreeInstance {
                position: Vector3::new(
                    pu,
                    self.interpolated_height(pu, pv) / self.size.y.max(1e-4),
                    pv,
                ),
                prototype,
                width_scale: ws,
                height_scale: hs,
                rotation: if random_rotation { next() * 360.0 } else { 0.0 },
                color: Color::rgb(shade, shade, shade),
            });
            added += 1;
        }
        if added > 0 {
            self.touch_trees();
        }
    }

    /// Unity Mass Place Trees: `count` trees spread over the whole terrain.
    pub fn mass_place_trees(&mut self, count: usize, seed: u32, keep_existing: bool) {
        if self.tree_prototypes.is_empty() {
            return;
        }
        if !keep_existing {
            self.trees.clear();
        }
        let protos = self.tree_prototypes.len() as u32;
        let mut rng = seed.max(1);
        let mut next = || {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            rng
        };
        for _ in 0..count {
            let u = (next() % 100_000) as f32 / 100_000.0;
            let v = (next() % 100_000) as f32 / 100_000.0;
            if !self.is_solid_at(u, v) {
                continue;
            }
            let s = 0.8 + (next() % 1000) as f32 / 1000.0 * 0.4;
            self.trees.push(TreeInstance {
                position: Vector3::new(
                    u,
                    self.interpolated_height(u, v) / self.size.y.max(1e-4),
                    v,
                ),
                prototype: next() % protos,
                width_scale: s,
                height_scale: s,
                rotation: (next() % 360) as f32,
                color: Color::WHITE,
            });
        }
        self.touch_trees();
    }

    /// Moves every tree onto the current ground (after height edits).
    pub fn snap_trees_to_ground(&mut self) {
        let sy = self.size.y.max(1e-4);
        let ys: Vec<f32> = self
            .trees
            .iter()
            .map(|t| self.interpolated_height(t.position.x, t.position.z) / sy)
            .collect();
        for (t, y) in self.trees.iter_mut().zip(ys) {
            t.position.y = y;
        }
    }

    /// Makes `details` match the prototypes and resolution.
    pub fn fit_details(&mut self) {
        let s = (self.detail_resolution * self.detail_resolution) as usize;
        self.details.truncate(self.detail_prototypes.len());
        while self.details.len() < self.detail_prototypes.len() {
            self.details.push(Arc::new(vec![0; s]));
        }
        for d in &mut self.details {
            if d.len() != s {
                *d = Arc::new(vec![0; s]);
            }
        }
    }

    /// One stroke step of Paint Details: moves the density of `prototype` toward
    /// `target` (0..16); `erase` toward 0.
    pub fn paint_details(
        &mut self,
        u: f32,
        v: f32,
        brush: &Brush,
        prototype: usize,
        target: f32,
        erase: bool,
    ) {
        if prototype >= self.detail_prototypes.len() {
            return;
        }
        self.fit_details();
        let s = self.detail_resolution;
        let mut edits = vec![];
        brush.cells(u, v, self.size, s, |x, z, w| edits.push((x, z, w)));
        let map = Arc::make_mut(&mut self.details[prototype]);
        let goal = if erase { 0.0 } else { target.clamp(0.0, 16.0) };
        for (x, z, w) in edits {
            let i = (z * s + x) as usize;
            let cur = map[i] as f32;
            let k = (w * brush.strength).min(1.0);
            map[i] = (cur + (goal - cur) * k).round().clamp(0.0, 16.0) as u8;
        }
        self.touch_details();
    }

    /// Detail density (0..16) of `prototype` at normalized `(u, v)`.
    pub fn detail_density(&self, prototype: usize, u: f32, v: f32) -> u8 {
        let s = self.detail_resolution;
        self.details.get(prototype).map_or(0, |d| {
            let x = ((u.clamp(0.0, 1.0) * s as f32) as u32).min(s - 1);
            let z = ((v.clamp(0.0, 1.0) * s as f32) as u32).min(s - 1);
            d.get((z * s + x) as usize).copied().unwrap_or(0)
        })
    }

    // ---- generators --------------------------------------------------------------------

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

    /// Paints the control map from height and slope: layer 0 low/flat (grass), 1 steep
    /// (rock), 2 near the bottom (sand/shore), 3 high (snow/dirt).
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
        self.fit_alphamaps();
        self.alphamaps[0] = Arc::new(out);
        for m in self.alphamaps.iter_mut().skip(1) {
            *m = Arc::new(vec![[0; 4]; s * s]);
        }
        self.touch_alphamaps();
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
                color: Color::WHITE,
            });
            placed += 1;
        }
        self.touch_trees();
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
            t.alphamaps[0]
                .iter()
                .all(|w| w.iter().map(|&x| x as u32).sum::<u32>() >= 250)
        );
    }

    fn brush(size: f32, strength: f32) -> Brush {
        Brush {
            size,
            strength,
            ..Default::default()
        }
    }

    #[test]
    fn height_tools() {
        let mut t = TerrainData::flat(Vector3::new(100.0, 50.0, 100.0), 101);
        t.paint_height(0.5, 0.5, &brush(20.0, 1.0), HeightOp::Raise(5.0));
        assert!((t.interpolated_height(0.5, 0.5) - 5.0).abs() < 0.05);
        assert_eq!(t.interpolated_height(0.1, 0.1), 0.0, "outside the brush");
        assert_eq!(t.dirty_heights.len(), 1);
        // Lower = negative raise.
        t.paint_height(0.5, 0.5, &brush(20.0, 1.0), HeightOp::Raise(-2.0));
        assert!((t.interpolated_height(0.5, 0.5) - 3.0).abs() < 0.05);
        // Set Height pulls toward the target.
        t.paint_height(0.2, 0.2, &brush(10.0, 1.0), HeightOp::SetHeight(10.0));
        assert!((t.interpolated_height(0.2, 0.2) - 10.0).abs() < 0.05);
        // Smooth lowers a spike.
        let before = t.interpolated_height(0.2, 0.2);
        t.paint_height(0.2, 0.2, &brush(30.0, 1.0), HeightOp::Smooth);
        assert!(t.interpolated_height(0.2, 0.2) < before);
        // Stamp raises to at least the stamp height.
        t.paint_height(
            0.8,
            0.8,
            &brush(20.0, 1.0),
            HeightOp::Stamp {
                height: 20.0,
                add: false,
            },
        );
        assert!((t.interpolated_height(0.8, 0.8) - 20.0).abs() < 0.1);
    }

    #[test]
    fn texture_painting_keeps_weights_normalized() {
        let mut t = TerrainData::flat(Vector3::new(100.0, 50.0, 100.0), 65);
        for i in 0..6 {
            t.add_layer(
                TerrainLayer {
                    name: format!("L{i}"),
                    ..Default::default()
                },
                None,
            );
        }
        assert_eq!(t.alphamaps.len(), 2, "6 layers need two control maps");
        t.paint_texture(0.5, 0.5, &brush(30.0, 1.0), 5, 1.0);
        let s = t.splat_resolution;
        let w = t.alphas_at(s / 2, s / 2);
        assert!(w[5] > 0.95, "{w:?}");
        assert!((w.iter().sum::<f32>() - 1.0).abs() < 0.02);
        let w = t.alphas_at(2, 2);
        assert!(w[0] > 0.99, "untouched: {w:?}");
        t.remove_layer(0);
        assert_eq!(t.layers.len(), 5);
        let w = t.alphas_at(s / 2, s / 2);
        assert!(w[4] > 0.95, "layer 5 moved down: {w:?}");
    }

    #[test]
    fn holes_trees_and_details() {
        let mut t = TerrainData::flat(Vector3::new(100.0, 50.0, 100.0), 65);
        t.paint_holes(0.5, 0.5, &brush(10.0, 1.0), false);
        assert!(!t.is_solid_at(0.5, 0.5));
        assert!(t.is_solid_at(0.1, 0.1));
        t.paint_holes(0.5, 0.5, &brush(20.0, 1.0), true);
        assert!(t.holes.is_empty(), "filling every hole drops the hole map");
        t.tree_prototypes.push(TreePrototype::new(
            Handle::from_path("builtin:tree"),
            vec![],
        ));
        let mut rng = 7;
        t.paint_trees(
            0.5,
            0.5,
            &brush(40.0, 1.0),
            0,
            5.0,
            (1.0, 1.0),
            (1.0, 1.0),
            0.0,
            true,
            false,
            &mut rng,
        );
        let n = t.trees.len();
        assert!(n > 10, "{n}");
        for _ in 0..30 {
            t.paint_trees(
                0.5,
                0.5,
                &brush(40.0, 1.0),
                0,
                5.0,
                (1.0, 1.0),
                (1.0, 1.0),
                0.0,
                true,
                false,
                &mut rng,
            );
        }
        // 5 trees per 100 m² over a 20 m radius: about 63 at most.
        assert!(
            t.trees.len() >= n && t.trees.len() <= 63,
            "{}",
            t.trees.len()
        );
        t.paint_trees(
            0.5,
            0.5,
            &brush(60.0, 1.0),
            0,
            5.0,
            (1.0, 1.0),
            (1.0, 1.0),
            0.0,
            true,
            true,
            &mut rng,
        );
        assert!(t.trees.is_empty());
        t.detail_prototypes.push(DetailPrototype::default());
        t.paint_details(0.5, 0.5, &brush(10.0, 1.0), 0, 8.0, false);
        assert_eq!(t.detail_density(0, 0.5, 0.5), 8);
        assert_eq!(t.detail_density(0, 0.1, 0.1), 0);
    }

    #[test]
    fn brush_shapes() {
        for shape in BrushShape::ALL {
            let b = Brush {
                shape,
                ..Default::default()
            };
            assert!(b.weight(0.0, 0.0) > 0.5, "{shape:?} centre");
            assert!(b.weight(1.2, 1.2) < 1e-3, "{shape:?} outside");
        }
    }
}
