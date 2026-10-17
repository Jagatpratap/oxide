//! Oxide's lightmapper (Unity's Progressive Lightmapper, on the CPU).
//!
//! Static objects that receive lightmaps get a rectangle in a lightmap atlas sized by their
//! surface area × texels per unit; their lightmap UVs (`uv2`) are rasterized into it, and
//! each texel gathers:
//! - direct light from **Baked** lights (shadow rays against all static geometry),
//! - indirect light: cosine-weighted paths that bounce off static surfaces (lit by Baked and
//!   Mixed lights, plus emission) and pick up the sky when they escape.
//!
//! Indirect light is denoised with an edge-aware blur, gutters are dilated, and the result is
//! stored RGBM-encoded (gamma space, range 8) like Unity's mobile lightmaps. Light probes get
//! the same incoming light as L2 spherical harmonics, cosine-convolved so `Σ c·Y(n)` is the
//! diffuse light for normal `n`, in the units of the renderer's ambient SH.
//! <https://docs.unity3d.com/Manual/progressive-lightmapper.html>

use oxide_assets::{Mesh, Texture2D};
use oxide_math::{Matrix4x4, Vector2, Vector3, Vector4};
use oxide_render::sky::SkyModel;
use rapier3d::parry::query::{Ray, RayCast};
use rapier3d::parry::shape::TriMesh;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// RGBM range of gamma-encoded lightmaps (linear values up to `RGBM_RANGE²`).
pub const RGBM_RANGE: f32 = 2.828_427;

#[derive(Clone, Debug)]
pub struct BakeSettings {
    /// Lightmap texels per world unit (Unity "Lightmap Resolution").
    pub texels_per_unit: f32,
    /// Atlas size (Unity "Max Lightmap Size").
    pub max_lightmap_size: u32,
    /// Indirect paths per texel (Unity "Indirect Samples").
    pub samples: u32,
    /// Indirect bounces (Unity "Max Bounces").
    pub bounces: u32,
    /// Texels between objects in the atlas.
    pub padding: u32,
    /// Edge-aware blur of indirect light (Unity "Filtering").
    pub denoise: bool,
}

impl Default for BakeSettings {
    fn default() -> Self {
        Self {
            texels_per_unit: 10.0,
            max_lightmap_size: 1024,
            samples: 64,
            bounces: 2,
            padding: 2,
            denoise: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct BakeMaterial {
    /// Linear base color.
    pub albedo: Vector3,
    pub texture: Option<Arc<Texture2D>>,
    /// Tiling xy, offset zw of `texture`.
    pub tiling: Vector4,
    /// Linear emission (× intensity): emissive surfaces light their surroundings.
    pub emission: Vector3,
}

impl Default for BakeMaterial {
    fn default() -> Self {
        Self {
            albedo: Vector3::splat(0.8),
            texture: None,
            tiling: Vector4::new(1.0, 1.0, 0.0, 0.0),
            emission: Vector3::ZERO,
        }
    }
}

/// A static renderer (Contribute GI). All of them block and bounce light; the ones with
/// `receive_lightmap` also get a lightmap.
#[derive(Clone, Debug)]
pub struct BakeObject {
    pub mesh: Arc<Mesh>,
    pub model: Matrix4x4,
    /// One per submesh (the last repeats).
    pub materials: Vec<BakeMaterial>,
    pub receive_lightmap: bool,
    /// Unity "Scale In Lightmap".
    pub scale_in_lightmap: f32,
}

#[derive(Clone, Copy, Debug)]
pub enum BakeLightKind {
    /// `direction` = where the light travels.
    Directional {
        direction: Vector3,
    },
    Point {
        position: Vector3,
        range: f32,
    },
    Spot {
        position: Vector3,
        direction: Vector3,
        range: f32,
        /// cos(outer angle / 2), 1 / (cos(inner / 2) - cos(outer / 2)).
        cos_outer: f32,
        inv_cone: f32,
    },
    /// Rectangle or disc area light emitting along `normal` (one-sided, Lambertian): the
    /// light's colour is its radiance × π, so a large one gives irradiance ≈ colour.
    Area {
        position: Vector3,
        /// Half-extent vectors (a disc's radius along both).
        right: Vector3,
        up: Vector3,
        normal: Vector3,
        disc: bool,
        range: f32,
    },
}

impl BakeLightKind {
    /// Rays from `p` to the light: (direction to the light, distance, irradiance factor
    /// before the surface cosine). Area lights give one per sample of their surface.
    pub fn rays(&self, p: Vector3) -> Vec<(Vector3, f32, f32)> {
        let window = |d2: f32, range: f32| {
            let k = d2 / (range * range);
            let w = (1.0 - k * k).clamp(0.0, 1.0);
            w * w
        };
        match *self {
            BakeLightKind::Directional { direction } => {
                vec![(-direction.normalize_or(Vector3::NEG_Y), 1e4, 1.0)]
            }
            BakeLightKind::Point { position, range }
            | BakeLightKind::Spot {
                position, range, ..
            } => {
                let d = position - p;
                let d2 = d.length_squared();
                if d2 > range * range {
                    return Vec::new();
                }
                let dir = d / d2.sqrt().max(1e-4);
                let mut a = window(d2, range) / d2.max(0.01);
                if let BakeLightKind::Spot {
                    direction,
                    cos_outer,
                    inv_cone,
                    ..
                } = *self
                {
                    let s = ((direction.dot(-dir) - cos_outer) * inv_cone).clamp(0.0, 1.0);
                    a *= s * s;
                }
                vec![(dir, d2.sqrt(), a)]
            }
            BakeLightKind::Area {
                position,
                right,
                up,
                normal,
                disc,
                range,
            } => {
                const N: usize = 4;
                let area = if disc {
                    std::f32::consts::PI * right.length() * up.length()
                } else {
                    4.0 * right.length() * up.length()
                };
                let da = area / (N * N) as f32;
                let mut out = Vec::with_capacity(N * N);
                for i in 0..N {
                    for j in 0..N {
                        // Stratified cell centres; discs by the concentric mapping.
                        let (mut u, mut v) = (
                            (i as f32 + 0.5) / N as f32 * 2.0 - 1.0,
                            (j as f32 + 0.5) / N as f32 * 2.0 - 1.0,
                        );
                        if disc {
                            let (r, phi) = if u.abs() > v.abs() {
                                (u, std::f32::consts::FRAC_PI_4 * (v / u))
                            } else {
                                (
                                    v,
                                    std::f32::consts::FRAC_PI_2
                                        - std::f32::consts::FRAC_PI_4 * (u / v),
                                )
                            };
                            (u, v) = (r * phi.cos(), r * phi.sin());
                        }
                        let q = position + right * u + up * v;
                        let d = q - p;
                        let d2 = d.length_squared();
                        if d2 > range * range || d2 < 1e-8 {
                            continue;
                        }
                        let dir = d / d2.sqrt();
                        let cos_l = normal.dot(-dir);
                        if cos_l <= 0.0 {
                            continue;
                        }
                        let a =
                            cos_l * da / (std::f32::consts::PI * d2.max(1e-4)) * window(d2, range);
                        out.push((dir, d2.sqrt(), a));
                    }
                }
                out
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BakeLight {
    pub kind: BakeLightKind,
    /// Linear color × intensity.
    pub color: Vector3,
    /// Baked lights put their direct light in lightmaps and probes; Mixed lights only bounce
    /// (their direct light stays realtime).
    pub baked_direct: bool,
    pub cast_shadows: bool,
}

pub struct BakeInput {
    pub objects: Vec<BakeObject>,
    pub lights: Vec<BakeLight>,
    pub sky: SkyModel,
    /// Multiplies the sky's contribution (Environment Lighting Intensity).
    pub ambient_intensity: f32,
    pub probes: Vec<Vector3>,
    pub settings: BakeSettings,
}

#[derive(Clone, Debug)]
pub struct Lightmap {
    pub size: u32,
    /// RGBM, gamma space, top row first.
    pub rgbm: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
pub struct BakeOutput {
    pub lightmaps: Vec<Lightmap>,
    /// Per input object: lightmap index and Unity `lightmapScaleOffset`.
    pub placements: Vec<Option<(u32, Vector4)>>,
    /// Per probe: L2 SH, cosine-convolved, divided by π (renderer ambient units).
    pub probes: Vec<[Vector3; 9]>,
}

/// Shared progress of a running bake.
#[derive(Default)]
pub struct Progress {
    pub done: AtomicU64,
    pub total: AtomicU64,
    pub cancel: AtomicBool,
}

impl Progress {
    pub fn fraction(&self) -> f32 {
        let t = self.total.load(Ordering::Relaxed).max(1);
        self.done.load(Ordering::Relaxed) as f32 / t as f32
    }
}

/// Encodes linear `c` as gamma-space RGBM.
pub fn encode_rgbm(c: Vector3) -> [u8; 4] {
    let s = c.max(Vector3::ZERO).powf(0.5);
    let m = (s.max_element() / RGBM_RANGE).clamp(1.0 / 255.0, 1.0);
    let m = (m * 255.0).ceil() / 255.0;
    let rgb = (s / (m * RGBM_RANGE)).clamp(Vector3::ZERO, Vector3::ONE);
    [
        (rgb.x * 255.0).round() as u8,
        (rgb.y * 255.0).round() as u8,
        (rgb.z * 255.0).round() as u8,
        (m * 255.0) as u8,
    ]
}

/// Decodes [`encode_rgbm`] (as the shader does).
pub fn decode_rgbm(p: [u8; 4]) -> Vector3 {
    let s = Vector3::new(p[0] as f32, p[1] as f32, p[2] as f32) / 255.0
        * (p[3] as f32 / 255.0)
        * RGBM_RANGE;
    s * s
}

// ---- random numbers ---------------------------------------------------------------------

/// Small deterministic generator (PCG) so bakes are reproducible.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        let mut r = Self(
            seed.wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407),
        );
        r.next();
        r
    }
    fn next(&mut self) -> u32 {
        let old = self.0;
        self.0 = old
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let x = (((old >> 18) ^ old) >> 27) as u32;
        x.rotate_right((old >> 59) as u32)
    }
    fn f32(&mut self) -> f32 {
        (self.next() >> 8) as f32 / (1u32 << 24) as f32
    }
}

fn basis(n: Vector3) -> (Vector3, Vector3) {
    let up = if n.y.abs() < 0.999 {
        Vector3::Y
    } else {
        Vector3::X
    };
    let t = up.cross(n).normalize();
    (t, n.cross(t))
}

fn cosine_dir(n: Vector3, rng: &mut Rng) -> Vector3 {
    let (u1, u2) = (rng.f32(), rng.f32());
    let r = u1.sqrt();
    let phi = 2.0 * std::f32::consts::PI * u2;
    let (t, b) = basis(n);
    (t * (r * phi.cos()) + b * (r * phi.sin()) + n * (1.0 - u1).max(0.0).sqrt()).normalize()
}

fn sphere_dir(rng: &mut Rng) -> Vector3 {
    let z = 1.0 - 2.0 * rng.f32();
    let r = (1.0 - z * z).max(0.0).sqrt();
    let phi = 2.0 * std::f32::consts::PI * rng.f32();
    Vector3::new(r * phi.cos(), r * phi.sin(), z)
}

// ---- the static scene as one ray-traceable mesh -----------------------------------------

struct Scene<'a> {
    mesh: TriMesh,
    /// Per triangle: (object, material slot).
    owner: Vec<(u32, u32)>,
    uv: Vec<[Vector2; 3]>,
    normals: Vec<[Vector3; 3]>,
    positions: Vec<[Vector3; 3]>,
    input: &'a BakeInput,
}

struct Hit {
    point: Vector3,
    normal: Vector3,
    albedo: Vector3,
    emission: Vector3,
    /// The ray hit the back of the surface (inside a closed object).
    back_face: bool,
}

fn bary(p: Vector3, [a, b, c]: [Vector3; 3]) -> Vector3 {
    let (v0, v1, v2) = (b - a, c - a, p - a);
    let (d00, d01, d11) = (v0.dot(v0), v0.dot(v1), v1.dot(v1));
    let (d20, d21) = (v2.dot(v0), v2.dot(v1));
    let den = (d00 * d11 - d01 * d01).max(1e-20);
    let v = (d11 * d20 - d01 * d21) / den;
    let w = (d00 * d21 - d01 * d20) / den;
    Vector3::new(1.0 - v - w, v, w)
}

fn sample_texture(t: &Texture2D, uv: Vector2) -> Vector3 {
    let Some(px) = t.mips.first() else {
        return Vector3::ONE;
    };
    // Nearest texel, repeat wrap, bottom-left UV origin.
    let x = ((uv.x.rem_euclid(1.0)) * t.width as f32) as u32 % t.width.max(1);
    let y = ((1.0 - uv.y.rem_euclid(1.0)) * t.height as f32) as u32 % t.height.max(1);
    let i = ((y * t.width + x) * 4) as usize;
    let c = |v: u8| {
        let c = v as f32 / 255.0;
        if !t.srgb {
            c
        } else if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    Vector3::new(c(px[i]), c(px[i + 1]), c(px[i + 2]))
}

impl<'a> Scene<'a> {
    fn build(input: &'a BakeInput) -> Result<Self, String> {
        let mut verts = Vec::new();
        let mut tris = Vec::new();
        let mut owner = Vec::new();
        let mut uv = Vec::new();
        let mut normals = Vec::new();
        let mut positions = Vec::new();
        for (oi, o) in input.objects.iter().enumerate() {
            let m = &o.mesh;
            let nm = oxide_math::glam::Mat3::from_mat4(o.model)
                .inverse()
                .transpose();
            let base = verts.len() as u32;
            verts.extend(m.vertices.iter().map(|v| o.model.transform_point3(*v)));
            for (si, range) in m.submeshes.iter().enumerate() {
                for t in m.indices[range.start as usize..range.end as usize]
                    .as_chunks::<3>()
                    .0
                {
                    let idx = [t[0], t[1], t[2]];
                    tris.push(idx.map(|i| base + i));
                    owner.push((oi as u32, si as u32));
                    uv.push(idx.map(|i| m.uv.get(i as usize).copied().unwrap_or_default()));
                    normals.push(idx.map(|i| {
                        (nm * m.normals.get(i as usize).copied().unwrap_or(Vector3::Y))
                            .normalize_or(Vector3::Y)
                    }));
                    positions.push(idx.map(|i| verts[(base + i) as usize]));
                }
            }
        }
        if tris.is_empty() {
            return Err("nothing to bake: no static renderers (mark objects Contribute GI)".into());
        }
        let mesh = TriMesh::new(verts, tris).map_err(|e| format!("bake scene: {e:?}"))?;
        Ok(Self {
            mesh,
            owner,
            uv,
            normals,
            positions,
            input,
        })
    }

    fn occluded(&self, origin: Vector3, dir: Vector3, max: f32) -> bool {
        self.mesh.intersects_local_ray(&Ray::new(origin, dir), max)
    }

    fn trace(&self, origin: Vector3, dir: Vector3) -> Option<Hit> {
        let hit = self
            .mesh
            .cast_local_ray_and_get_normal(&Ray::new(origin, dir), 1e5, false)?;
        // The hit triangle's index in the mesh (`feature` is the face within that triangle).
        let tri = hit.subshape as usize;
        if tri >= self.owner.len() {
            return None;
        }
        let point = origin + dir * hit.time_of_impact;
        let w = bary(point, self.positions[tri]);
        let [n0, n1, n2] = self.normals[tri];
        let mut normal = (n0 * w.x + n1 * w.y + n2 * w.z).normalize_or(Vector3::Y);
        let [a, b, c] = self.positions[tri];
        let geo = (b - a).cross(c - a).normalize_or(normal);
        // Mesh winding is clockwise (Unity), so the geometric normal points inward; orient
        // it with the shading normal.
        let geo = if geo.dot(normal) < 0.0 { -geo } else { geo };
        let back_face = geo.dot(dir) > 0.0;
        if back_face {
            normal = -normal;
        }
        let (oi, si) = self.owner[tri];
        let o = &self.input.objects[oi as usize];
        let mat = o
            .materials
            .get(si as usize)
            .or(o.materials.last())
            .cloned()
            .unwrap_or_default();
        let mut albedo = mat.albedo;
        if let Some(t) = &mat.texture {
            let [u0, u1, u2] = self.uv[tri];
            let uv = u0 * w.x + u1 * w.y + u2 * w.z;
            let uv = uv * Vector2::new(mat.tiling.x, mat.tiling.y)
                + Vector2::new(mat.tiling.z, mat.tiling.w);
            albedo *= sample_texture(t, uv);
        }
        Some(Hit {
            point,
            normal,
            albedo,
            emission: mat.emission,
            back_face,
        })
    }

    /// Direct light (E/π units, like the shader) at `p` with normal `n` from lights selected
    /// by `include`.
    fn direct(&self, p: Vector3, n: Vector3, include: impl Fn(&BakeLight) -> bool) -> Vector3 {
        let mut sum = Vector3::ZERO;
        let o = p + n * 2e-3;
        for l in self.input.lights.iter().filter(|l| include(l)) {
            for (to_light, dist, atten) in l.kind.rays(p) {
                let nl = n.dot(to_light);
                if nl <= 0.0 || atten <= 0.0 {
                    continue;
                }
                if l.cast_shadows && self.occluded(o, to_light, (dist - 2e-3).max(0.0)) {
                    continue;
                }
                sum += l.color * (nl * atten);
            }
        }
        sum
    }

    /// Radiance arriving at `origin` from `dir` after up to `bounces` diffuse bounces.
    fn radiance(&self, origin: Vector3, dir: Vector3, bounces: u32, rng: &mut Rng) -> Vector3 {
        let Some(h) = self.trace(origin, dir) else {
            return self.input.sky.radiance(dir) * self.input.ambient_intensity;
        };
        if h.back_face {
            return Vector3::ZERO;
        }
        // Every baked-GI light bounces (Mixed and Baked).
        let mut e = self.direct(h.point, h.normal, |_| true);
        if bounces > 0 {
            let d = cosine_dir(h.normal, rng);
            e += self.radiance(h.point + h.normal * 2e-3, d, bounces - 1, rng);
        }
        h.emission + h.albedo * e
    }
}

// ---- atlas packing ----------------------------------------------------------------------

fn triangle_area(a: Vector3, b: Vector3, c: Vector3) -> f32 {
    (b - a).cross(c - a).length() * 0.5
}

/// Where an object's lightmap goes: atlas index and `[x, y, side]` in texels.
type Placement = Option<(u32, [u32; 3])>;

/// Places each receiving object's rectangle in an atlas (shelf packing, biggest first).
fn pack(input: &BakeInput) -> (Vec<Placement>, u32) {
    let s = &input.settings;
    let size = s.max_lightmap_size.clamp(32, 8192);
    let pad = s.padding;
    let sides: Vec<Option<u32>> = input
        .objects
        .iter()
        .map(|o| {
            if !o.receive_lightmap || !o.mesh.has_lightmap_uvs() {
                return None;
            }
            let m = &o.mesh;
            let area: f32 = m
                .indices
                .as_chunks::<3>()
                .0
                .iter()
                .map(|t| {
                    let p = |i: u32| o.model.transform_point3(m.vertices[i as usize]);
                    triangle_area(p(t[0]), p(t[1]), p(t[2]))
                })
                .sum();
            let side =
                (area.sqrt() * s.texels_per_unit * o.scale_in_lightmap.max(0.0)).ceil() as u32;
            Some(side.clamp(4, size - 2 * pad))
        })
        .collect();
    let mut order: Vec<usize> = (0..sides.len()).filter(|&i| sides[i].is_some()).collect();
    order.sort_by(|&a, &b| sides[b].cmp(&sides[a]).then(a.cmp(&b)));
    let mut out = vec![None; sides.len()];
    let (mut atlas, mut x, mut y, mut shelf) = (0u32, pad, pad, 0u32);
    for i in order {
        let side = sides[i].expect("filtered");
        if x + side + pad > size {
            x = pad;
            y += shelf + pad;
            shelf = 0;
        }
        if y + side + pad > size {
            atlas += 1;
            x = pad;
            y = pad;
            shelf = 0;
        }
        out[i] = Some((atlas, [x, y, side]));
        x += side + pad;
        shelf = shelf.max(side);
    }
    (out, size)
}

// ---- the bake -----------------------------------------------------------------------------

/// A lightmap texel to light.
#[derive(Clone, Copy)]
struct Texel {
    atlas: u32,
    x: u32,
    y: u32,
    pos: Vector3,
    normal: Vector3,
}

fn rasterize(input: &BakeInput, placements: &[Placement], size: u32) -> Vec<Texel> {
    let mut texels = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (oi, place) in placements.iter().enumerate() {
        let Some((atlas, [rx, ry, side])) = *place else {
            continue;
        };
        let o = &input.objects[oi];
        let m = &o.mesh;
        let nm = oxide_math::glam::Mat3::from_mat4(o.model)
            .inverse()
            .transpose();
        for t in m.indices.as_chunks::<3>().0 {
            let idx = [t[0], t[1], t[2]].map(|i| i as usize);
            // Lightmap UV (bottom-left origin) → atlas pixels (top row first).
            let px = idx.map(|i| {
                let u = m.uv2[i];
                Vector2::new(
                    rx as f32 + u.x * side as f32,
                    ry as f32 + (1.0 - u.y) * side as f32,
                )
            });
            let wp = idx.map(|i| o.model.transform_point3(m.vertices[i]));
            let wn = idx.map(|i| {
                (nm * m.normals.get(i).copied().unwrap_or(Vector3::Y)).normalize_or(Vector3::Y)
            });
            let edge = |a: Vector2, b: Vector2, c: Vector2| {
                (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
            };
            let area = edge(px[0], px[1], px[2]);
            if area.abs() < 1e-8 {
                continue;
            }
            let lo = px[0].min(px[1]).min(px[2]).floor().max(Vector2::ZERO);
            let hi = px[0]
                .max(px[1])
                .max(px[2])
                .ceil()
                .min(Vector2::splat(size as f32));
            for y in lo.y as u32..hi.y as u32 {
                for x in lo.x as u32..hi.x as u32 {
                    let c = Vector2::new(x as f32 + 0.5, y as f32 + 0.5);
                    let w = [
                        edge(px[1], px[2], c) / area,
                        edge(px[2], px[0], c) / area,
                        edge(px[0], px[1], c) / area,
                    ];
                    // Slightly generous so texels on chart edges are covered.
                    if w.iter().any(|&v| v < -0.02) || !seen.insert((atlas, x, y)) {
                        continue;
                    }
                    let pos = wp[0] * w[0] + wp[1] * w[1] + wp[2] * w[2];
                    let normal =
                        (wn[0] * w[0] + wn[1] * w[1] + wn[2] * w[2]).normalize_or(Vector3::Y);
                    texels.push(Texel {
                        atlas,
                        x,
                        y,
                        pos,
                        normal,
                    });
                }
            }
        }
    }
    texels
}

/// Runs `f` over `items` on all cores, in order-preserving chunks.
fn parallel_map<T: Sync, R: Send + Default + Clone>(
    items: &[T],
    progress: &Progress,
    f: impl Fn(usize, &T) -> R + Sync,
) -> Vec<R> {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let chunk = items.len().div_ceil(threads * 8).max(1);
    let mut out = vec![R::default(); items.len()];
    let next = std::sync::atomic::AtomicUsize::new(0);
    let slots: Vec<std::sync::Mutex<&mut [R]>> =
        out.chunks_mut(chunk).map(std::sync::Mutex::new).collect();
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    let c = next.fetch_add(1, Ordering::Relaxed);
                    if c >= slots.len() || progress.cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let mut slot = slots[c].lock().expect("chunk lock");
                    for (k, r) in slot.iter_mut().enumerate() {
                        let i = c * chunk + k;
                        *r = f(i, &items[i]);
                    }
                    progress
                        .done
                        .fetch_add(slot.len() as u64, Ordering::Relaxed);
                }
            });
        }
    });
    drop(slots);
    out
}

fn sh_basis(d: Vector3) -> [f32; 9] {
    [
        0.282095,
        0.488603 * d.y,
        0.488603 * d.z,
        0.488603 * d.x,
        1.092548 * d.x * d.y,
        1.092548 * d.y * d.z,
        0.315392 * (3.0 * d.z * d.z - 1.0),
        1.092548 * d.x * d.z,
        0.546274 * (d.x * d.x - d.y * d.y),
    ]
}

/// Bakes lightmaps and light probes. Cancelling (through `progress`) returns an error.
pub fn bake(input: &BakeInput, progress: &Progress) -> Result<BakeOutput, String> {
    let scene = Scene::build(input)?;
    let (places, size) = pack(input);
    let texels = rasterize(input, &places, size);
    let probe_samples = (input.settings.samples * 4).max(64);
    progress.done.store(0, Ordering::Relaxed);
    progress.total.store(
        (texels.len() + input.probes.len()) as u64,
        Ordering::Relaxed,
    );

    let bounces = input.settings.bounces;
    let samples = input.settings.samples.max(1);
    // Per texel: (baked direct, indirect).
    let lit: Vec<(Vector3, Vector3)> = parallel_map(&texels, progress, |i, t| {
        let mut rng = Rng::new(i as u64 + 1);
        let direct = scene.direct(t.pos, t.normal, |l| l.baked_direct);
        let o = t.pos + t.normal * 2e-3;
        let mut indirect = Vector3::ZERO;
        for _ in 0..samples {
            let d = cosine_dir(t.normal, &mut rng);
            indirect += scene.radiance(o, d, bounces, &mut rng);
        }
        (direct, indirect / samples as f32)
    });
    let probes: Vec<[Vector3; 9]> = parallel_map(&input.probes, progress, |i, &p| {
        let mut rng = Rng::new(0x9e37_79b9 ^ i as u64);
        let mut acc = [Vector3::ZERO; 9];
        let w = 4.0 * std::f32::consts::PI / probe_samples as f32;
        for _ in 0..probe_samples {
            let d = sphere_dir(&mut rng);
            let l = scene.radiance(p, d, bounces, &mut rng);
            for (k, y) in sh_basis(d).iter().enumerate() {
                acc[k] += l * (y * w);
            }
        }
        // Baked lights' direct light as impulses (radiance π·I toward the light).
        for l in input.lights.iter().filter(|l| l.baked_direct) {
            for (dir, dist, atten) in l.kind.rays(p) {
                if atten <= 0.0 || (l.cast_shadows && scene.occluded(p, dir, dist)) {
                    continue;
                }
                for (k, y) in sh_basis(dir).iter().enumerate() {
                    acc[k] += l.color * (std::f32::consts::PI * atten * y);
                }
            }
        }
        // Cosine-lobe convolution per band, divided by π.
        let band = [
            1.0,
            2.0 / 3.0,
            2.0 / 3.0,
            2.0 / 3.0,
            0.25,
            0.25,
            0.25,
            0.25,
            0.25,
        ];
        std::array::from_fn(|k| acc[k] * band[k])
    });
    if progress.cancel.load(Ordering::Relaxed) {
        return Err("bake cancelled".into());
    }

    // Assemble atlases: direct + (denoised) indirect, then dilate into gutters.
    let atlases = places
        .iter()
        .flatten()
        .map(|(a, _)| a + 1)
        .max()
        .unwrap_or(0);
    let n = (size * size) as usize;
    let mut lightmaps = Vec::new();
    for a in 0..atlases {
        let mut direct = vec![Vector3::ZERO; n];
        let mut indirect = vec![Vector3::ZERO; n];
        let mut normal = vec![Vector3::ZERO; n];
        let mut valid = vec![false; n];
        for (t, (d, i)) in texels.iter().zip(&lit) {
            if t.atlas != a {
                continue;
            }
            let k = (t.y * size + t.x) as usize;
            direct[k] = *d;
            indirect[k] = *i;
            normal[k] = t.normal;
            valid[k] = true;
        }
        if input.settings.denoise {
            indirect = denoise(&indirect, &normal, &valid, size);
        }
        let mut color: Vec<Vector3> = direct.iter().zip(&indirect).map(|(d, i)| *d + *i).collect();
        dilate(
            &mut color,
            &mut valid,
            size,
            input.settings.padding.max(1) + 1,
        );
        let rgbm = color.iter().flat_map(|c| encode_rgbm(*c)).collect();
        lightmaps.push(Lightmap { size, rgbm });
    }
    let placements = places
        .iter()
        .map(|p| {
            p.map(|(a, [x, y, side])| {
                let s = side as f32 / size as f32;
                // uv2 (bottom-left origin) → atlas UV (bottom-left origin).
                let off_x = x as f32 / size as f32;
                let off_y = 1.0 - (y + side) as f32 / size as f32;
                (a, Vector4::new(s, s, off_x, off_y))
            })
        })
        .collect();
    Ok(BakeOutput {
        lightmaps,
        placements,
        probes,
    })
}

/// 5×5 blur that only mixes texels of similar orientation.
fn denoise(src: &[Vector3], normal: &[Vector3], valid: &[bool], size: u32) -> Vec<Vector3> {
    let s = size as i32;
    let mut out = src.to_vec();
    for y in 0..s {
        for x in 0..s {
            let k = (y * s + x) as usize;
            if !valid[k] {
                continue;
            }
            let (mut sum, mut wsum) = (Vector3::ZERO, 0.0);
            for dy in -2..=2 {
                for dx in -2..=2 {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx < 0 || ny < 0 || nx >= s || ny >= s {
                        continue;
                    }
                    let j = (ny * s + nx) as usize;
                    if !valid[j] {
                        continue;
                    }
                    let w = normal[k].dot(normal[j]).max(0.0).powi(8)
                        * (-((dx * dx + dy * dy) as f32) / 4.5).exp();
                    sum += src[j] * w;
                    wsum += w;
                }
            }
            if wsum > 0.0 {
                out[k] = sum / wsum;
            }
        }
    }
    out
}

/// Fills empty texels next to filled ones (so bilinear filtering at chart edges and mips
/// don't pull in black), `passes` texels deep.
fn dilate(color: &mut [Vector3], valid: &mut [bool], size: u32, passes: u32) {
    let s = size as i32;
    for _ in 0..passes {
        let mut fill = Vec::new();
        for y in 0..s {
            for x in 0..s {
                let k = (y * s + x) as usize;
                if valid[k] {
                    continue;
                }
                let (mut sum, mut n) = (Vector3::ZERO, 0);
                for (dx, dy) in [
                    (-1, 0),
                    (1, 0),
                    (0, -1),
                    (0, 1),
                    (-1, -1),
                    (1, 1),
                    (-1, 1),
                    (1, -1),
                ] {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx >= 0 && ny >= 0 && nx < s && ny < s && valid[(ny * s + nx) as usize] {
                        sum += color[(ny * s + nx) as usize];
                        n += 1;
                    }
                }
                if n > 0 {
                    fill.push((k, sum / n as f32));
                }
            }
        }
        for (k, c) in fill {
            color[k] = c;
            valid[k] = true;
        }
    }
}

#[cfg(test)]
mod tests;
