//! Oxide terrain (Unity `Terrain` + `TerrainCollider`): a `TerrainData` asset rendered as
//! LOD'd chunks with crack-hiding skirts and the splat-layer shader, trees as meshes up close
//! and billboards (baked by the engine from each prototype) far away, bending in Wind Zones,
//! and a heightfield collider kept in sync.
//! Reference: <https://docs.unity3d.com/Manual/script-Terrain.html>

// Submesh lists are `Vec<Range<u32>>`; a single-entry list is the normal case.
#![allow(clippy::single_range_in_vec_init)]

use oxide_assets::{
    AssetId, AssetServer, Handle, Mesh, TerrainData, TerrainLayer, Texture2D,
    terrain::ground_texture,
};
use oxide_core::Reflect;
use oxide_math::{Matrix4x4, Matrix4x4Ext, Quaternion, QuaternionExt, Vector2, Vector3, Vector4};
use oxide_physics::TerrainCollider;
use oxide_render::{
    DrawItem, GrassBatch, GrassInstance, ParticleBatch, ParticleBlend, ParticleInstance,
    ParticleMode, TerrainDraw,
};
use oxide_scene::{Component, GameObjectId, System, World};
use std::collections::HashMap;

pub mod toolbox;

/// Cells per chunk side at full detail.
pub const CHUNK_CELLS: u32 = 32;
pub const LODS: usize = 3;

/// Unity `Terrain`.
#[derive(Clone, Debug, Reflect)]
pub struct Terrain {
    pub data: Option<Handle<TerrainData>>,
    /// Draw the heightmap (Unity Draw).
    pub draw_heightmap: bool,
    pub cast_shadows: bool,
    /// Distance (m) where chunks drop to half detail (and again at 2.5× this).
    pub lod_distance: f32,
    /// Draw trees and detail objects (Unity Draw Trees and Foliage).
    pub draw_trees: bool,
    /// Trees beyond this distance from the camera are not drawn.
    pub tree_distance: f32,
    /// Trees beyond this distance are drawn as billboards (Unity Billboard Start).
    pub billboard_start: f32,
    /// Distance over which trees cross-fade from mesh to billboard.
    pub fade_length: f32,
    /// At most this many trees are drawn as meshes (the nearest; Unity Max Mesh Trees).
    pub max_mesh_trees: u32,
    /// Grass and detail meshes are drawn up to this distance.
    pub detail_distance: f32,
    /// Scales how many detail instances are drawn (0..1).
    #[reflect(range(0, 1))]
    pub detail_density: f32,
    /// Terrains with the same Grouping ID and Auto Connect on are neighbours: brushes paint
    /// across them and their edges stay stitched.
    pub grouping_id: i32,
    pub allow_auto_connect: bool,
    /// Terrain Layer assets painted on this terrain; when set they replace the
    /// TerrainData's own layers.
    pub layers: Vec<Handle<TerrainLayer>>,
    /// In Play mode, drop this tile's data and meshes when the camera is farther than
    /// `stream_distance` from it, and reload them from its file when it comes back.
    pub streaming: bool,
    pub stream_distance: f32,
}

impl Default for Terrain {
    fn default() -> Self {
        Self {
            data: None,
            draw_heightmap: true,
            cast_shadows: true,
            lod_distance: 120.0,
            draw_trees: true,
            tree_distance: 2000.0,
            billboard_start: 50.0,
            fade_length: 5.0,
            max_mesh_trees: 50,
            detail_distance: 80.0,
            detail_density: 1.0,
            grouping_id: 0,
            allow_auto_connect: true,
            layers: Vec::new(),
            streaming: false,
            stream_distance: 1500.0,
        }
    }
}

/// A point on a terrain: the object, world position and normalized `(u, v)`.
#[derive(Clone, Copy, Debug)]
pub struct TerrainHit {
    pub terrain: GameObjectId,
    pub point: Vector3,
    pub uv: Vector2,
    pub distance: f32,
}

/// Casts a ray against every terrain's heightfield (closest hit).
pub fn raycast(world: &World, ray: &oxide_math::Ray, max_distance: f32) -> Option<TerrainHit> {
    let assets = world.resource::<AssetServer>();
    let mut best: Option<TerrainHit> = None;
    for (id, t) in world.query::<Terrain>() {
        let Some(d) = t.data.as_ref().and_then(|h| assets.terrains.get(h)) else {
            continue;
        };
        if !world.get(id).is_some_and(|o| o.active_in_hierarchy()) {
            continue;
        }
        let o = world.position(id);
        // Clip the ray to the terrain's box.
        let (lo, hi) = (o, o + d.size);
        let inv = Vector3::new(
            1.0 / ray.direction.x,
            1.0 / ray.direction.y,
            1.0 / ray.direction.z,
        );
        let t0 = (lo - ray.origin) * inv;
        let t1 = (hi - ray.origin) * inv;
        let tmin = t0.min(t1).max_element().max(0.0);
        let tmax = t0.max(t1).min_element().min(max_distance);
        if tmin > tmax {
            continue;
        }
        let step =
            (d.size.x / (d.resolution - 1) as f32).min(d.size.z / (d.resolution - 1) as f32) * 0.5;
        let above = |t: f32| {
            let p = ray.origin + ray.direction * t;
            let (u, v) = ((p.x - o.x) / d.size.x, (p.z - o.z) / d.size.z);
            p.y - (o.y + d.interpolated_height(u, v))
        };
        let mut t = tmin;
        let mut prev = above(t);
        let mut hit = None;
        while t < tmax {
            let nt = (t + step.max(0.05)).min(tmax);
            let cur = above(nt);
            if prev >= 0.0 && cur < 0.0 {
                // Refine by bisection.
                let (mut a, mut b) = (t, nt);
                for _ in 0..16 {
                    let m = (a + b) * 0.5;
                    if above(m) >= 0.0 { a = m } else { b = m }
                }
                hit = Some((a + b) * 0.5);
                break;
            }
            if nt >= tmax {
                break;
            }
            t = nt;
            prev = cur;
        }
        let Some(dist) = hit else { continue };
        let p = ray.origin + ray.direction * dist;
        let uv = Vector2::new((p.x - o.x) / d.size.x, (p.z - o.z) / d.size.z);
        if !d.is_solid_at(uv.x, uv.y) {
            continue;
        }
        if best.is_none_or(|b| dist < b.distance) {
            best = Some(TerrainHit {
                terrain: id,
                point: p,
                uv,
                distance: dist,
            });
        }
    }
    best
}

/// Terrains connected to `id` (same Grouping ID, Auto Connect on, edges touching), with
/// their data handles and world origins — what a brush paints across.
pub fn connected(
    world: &World,
    id: GameObjectId,
) -> Vec<(GameObjectId, Handle<TerrainData>, Vector3)> {
    let Some(t) = world.get_component::<Terrain>(id) else {
        return vec![];
    };
    let mut out = vec![];
    if let Some(h) = &t.data {
        out.push((id, h.clone(), world.position(id)));
    }
    if !t.allow_auto_connect {
        return out;
    }
    let group = t.grouping_id;
    for (other, ot) in world.query::<Terrain>() {
        if other == id || !ot.allow_auto_connect || ot.grouping_id != group {
            continue;
        }
        if let Some(h) = &ot.data {
            out.push((other, h.clone(), world.position(other)));
        }
    }
    out
}

/// The empty neighbour slots (left, right, back, front) of a terrain tile, as world origins.
pub fn free_neighbour_slots(world: &World, id: GameObjectId) -> Vec<Vector3> {
    let assets = world.resource::<AssetServer>();
    let Some(d) = world
        .get_component::<Terrain>(id)
        .and_then(|t| t.data.as_ref())
        .and_then(|h| assets.terrains.get(h))
    else {
        return vec![];
    };
    let o = world.position(id);
    let taken: Vec<Vector3> = world
        .query::<Terrain>()
        .map(|(t, _)| world.position(t))
        .collect();
    [
        Vector3::new(-d.size.x, 0.0, 0.0),
        Vector3::new(d.size.x, 0.0, 0.0),
        Vector3::new(0.0, 0.0, -d.size.z),
        Vector3::new(0.0, 0.0, d.size.z),
    ]
    .into_iter()
    .map(|off| o + off)
    .filter(|p| !taken.iter().any(|q| q.distance(*p) < 0.5))
    .collect()
}

/// The first mesh (and its materials) of a model, prefab or the built-in tree — what a
/// tree or detail-mesh prototype draws.
pub fn resolve_mesh_source(
    world: &mut World,
    src: &str,
) -> Result<(Handle<Mesh>, Vec<Handle<oxide_assets::Material>>), String> {
    if src == "builtin:tree/simple" {
        register_builtin_tree(world.resource_mut::<AssetServer>());
        return Ok((
            Handle::from_path("builtin:tree/simple"),
            vec![
                Handle::from_path("builtin:tree/bark.mat"),
                Handle::from_path("builtin:tree/leaves.mat"),
            ],
        ));
    }
    if oxide_scene::prefab::is_prefab_path(src) {
        // Instantiate, read its first MeshFilter / MeshRenderer, throw it away.
        let root = world.instantiate_prefab(src, None)?;
        let mut stack = vec![root];
        let mut found = None;
        while let Some(o) = stack.pop() {
            if let (Some(f), Some(r)) = (
                world.get_component::<oxide_scene::MeshFilter>(o),
                world.get_component::<oxide_scene::MeshRenderer>(o),
            ) && let Some(m) = &f.mesh
            {
                found = Some((m.clone(), r.materials.clone()));
                break;
            }
            stack.extend_from_slice(world.children(o));
        }
        world.destroy_immediate(root);
        return found.ok_or_else(|| format!("{src} has no MeshFilter / MeshRenderer"));
    }
    let assets = world.resource_mut::<AssetServer>();
    assets.ensure_loaded(src)?;
    if let Some(h) = assets.meshes.handle_of(src) {
        return Ok((h, vec![]));
    }
    let model = assets
        .models
        .handle_of(src)
        .and_then(|h| assets.models.get(&h))
        .ok_or_else(|| format!("{src} is not a model"))?;
    let node = model
        .nodes
        .iter()
        .find(|n| n.mesh.is_some())
        .ok_or_else(|| format!("{src} has no mesh"))?;
    Ok((node.mesh.clone().expect("mesh"), node.materials.clone()))
}

/// Unity GameObject → 3D Object → Terrain: a flat TerrainData saved as a `.terraindata`
/// file at `path` (when given) with the built-in layers, and a GameObject using it.
pub fn create_terrain_object(
    world: &mut World,
    name: &str,
    size: Vector3,
    resolution: u32,
    path: Option<&str>,
) -> Result<GameObjectId, String> {
    let assets = world.resource_mut::<AssetServer>();
    register_builtin_textures(assets);
    let mut data = TerrainData::flat(size, resolution);
    data.name = name.into();
    data.splat_resolution = 512;
    for l in default_layers().into_iter().take(1) {
        data.add_layer(l, None);
    }
    data.fit_alphamaps();
    let h = match path {
        Some(p) => assets.save_terrain_data(p, &data)?,
        None => assets.terrains.add(data),
    };
    let id = world.create(name);
    world.add_component(
        id,
        Terrain {
            data: Some(h),
            ..Default::default()
        },
    );
    Ok(id)
}

impl Component for Terrain {
    fn allow_multiple(&self) -> bool {
        false
    }
}

struct Chunk {
    center: Vector3,
    lods: [Handle<Mesh>; LODS],
    /// Occluder meshes for near and far chunks (see [`occluder_mesh`]).
    occluders: [Handle<Mesh>; 2],
}

/// Grid step, in heightmap cells, of the near and far occluder meshes.
const OCCLUDER_STEPS: [u32; 2] = [4, 16];

/// A coarse chunk surface for occlusion culling. Each vertex takes the lowest height within
/// one step around it, so the surface never rises above the real ground and can't hide
/// anything that is actually visible.
fn occluder_mesh(d: &TerrainData, cx: u32, cz: u32, step: u32) -> Mesh {
    let n = d.resolution;
    let x0 = cx * CHUNK_CELLS;
    let z0 = cz * CHUNK_CELLS;
    let x1 = (x0 + CHUNK_CELLS).min(n - 1);
    let z1 = (z0 + CHUNK_CELLS).min(n - 1);
    let axis = |a: u32, b: u32| -> Vec<u32> {
        (a..=b)
            .step_by(step as usize)
            .chain(std::iter::once(b))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    };
    let (xs, zs) = (axis(x0, x1), axis(z0, z1));
    let cell = Vector2::new(d.size.x / (n - 1) as f32, d.size.z / (n - 1) as f32);
    let lowest = |x: u32, z: u32| {
        let mut h = f32::MAX;
        for sz in z.saturating_sub(step)..=(z + step).min(n - 1) {
            for sx in x.saturating_sub(step)..=(x + step).min(n - 1) {
                h = h.min(d.sample(sx, sz));
            }
        }
        h * d.size.y
    };
    let mut m = Mesh {
        name: format!("Terrain occluder {cx},{cz} step{step}"),
        ..Default::default()
    };
    let w = xs.len() as u32;
    for &z in &zs {
        for &x in &xs {
            m.vertices.push(Vector3::new(
                x as f32 * cell.x,
                lowest(x, z),
                z as f32 * cell.y,
            ));
        }
    }
    for j in 0..zs.len() as u32 - 1 {
        for i in 0..w - 1 {
            // Holes: drop quads whose heightmap cells are all holes (finer holes are cut
            // per pixel by the shader).
            if !d.holes.is_empty() {
                let any_solid =
                    (xs[i as usize]..xs[i as usize + 1].max(xs[i as usize] + 1)).any(|cx| {
                        (zs[j as usize]..zs[j as usize + 1].max(zs[j as usize] + 1))
                            .any(|cz| d.is_solid(cx, cz))
                    });
                if !any_solid {
                    continue;
                }
            }
            let bl = j * w + i;
            m.indices
                .extend_from_slice(&[bl, bl + w, bl + w + 1, bl, bl + w + 1, bl + 1]);
        }
    }
    m.submeshes = vec![0..m.indices.len() as u32];
    m.recalculate_bounds();
    m
}

/// Generated chunk meshes per TerrainData.
#[derive(Default)]
pub struct TerrainMeshes {
    built: HashMap<AssetId, Built>,
}

impl TerrainMeshes {
    /// Frees a terrain's chunk meshes (streaming).
    pub fn release(&mut self, assets: &mut AssetServer, data: AssetId) {
        if let Some(b) = self.built.remove(&data) {
            for c in b.chunks {
                for h in c.lods.iter().chain(c.occluders.iter()) {
                    assets.meshes.remove(h);
                }
            }
        }
    }

    /// Whether a terrain's meshes are built.
    pub fn is_built(&self, data: AssetId) -> bool {
        self.built.contains_key(&data)
    }
}

struct Built {
    /// Heights / holes versions the meshes match.
    heights: u64,
    holes: u64,
    size: (Vector3, u32),
    chunks: Vec<Chunk>,
}

fn chunk_mesh(d: &TerrainData, cx: u32, cz: u32, lod: u32) -> Mesh {
    let n = d.resolution;
    let step = 1u32 << lod;
    let x0 = cx * CHUNK_CELLS;
    let z0 = cz * CHUNK_CELLS;
    let x1 = (x0 + CHUNK_CELLS).min(n - 1);
    let z1 = (z0 + CHUNK_CELLS).min(n - 1);
    let xs: Vec<u32> = (x0..=x1)
        .step_by(step as usize)
        .chain(std::iter::once(x1))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let zs: Vec<u32> = (z0..=z1)
        .step_by(step as usize)
        .chain(std::iter::once(z1))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let cell = Vector2::new(d.size.x / (n - 1) as f32, d.size.z / (n - 1) as f32);
    let pos = |x: u32, z: u32| {
        Vector3::new(
            x as f32 * cell.x,
            d.sample(x, z) * d.size.y,
            z as f32 * cell.y,
        )
    };
    let normal = |x: u32, z: u32| {
        let h = |a: i64, b: i64| {
            d.sample(
                a.clamp(0, n as i64 - 1) as u32,
                b.clamp(0, n as i64 - 1) as u32,
            ) * d.size.y
        };
        let (x, z) = (x as i64, z as i64);
        Vector3::new(
            (h(x - 1, z) - h(x + 1, z)) / (2.0 * cell.x),
            1.0,
            (h(x, z - 1) - h(x, z + 1)) / (2.0 * cell.y),
        )
        .normalize()
    };
    let uv = |x: u32, z: u32| Vector2::new(x as f32 / (n - 1) as f32, z as f32 / (n - 1) as f32);
    let mut m = Mesh {
        name: format!("Terrain {cx},{cz} lod{lod}"),
        ..Default::default()
    };
    let w = xs.len() as u32;
    for &z in &zs {
        for &x in &xs {
            m.vertices.push(pos(x, z));
            m.normals.push(normal(x, z));
            m.uv.push(uv(x, z));
        }
    }
    for j in 0..zs.len() as u32 - 1 {
        for i in 0..w - 1 {
            // Holes: drop quads whose heightmap cells are all holes (finer holes are cut
            // per pixel by the shader).
            if !d.holes.is_empty() {
                let any_solid =
                    (xs[i as usize]..xs[i as usize + 1].max(xs[i as usize] + 1)).any(|cx| {
                        (zs[j as usize]..zs[j as usize + 1].max(zs[j as usize] + 1))
                            .any(|cz| d.is_solid(cx, cz))
                    });
                if !any_solid {
                    continue;
                }
            }
            let bl = j * w + i;
            m.indices
                .extend_from_slice(&[bl, bl + w, bl + w + 1, bl, bl + w + 1, bl + 1]);
        }
    }
    // Skirts hide cracks where neighbouring chunks use a different LOD.
    let depth = d.size.y * 0.02 + cell.x * 4.0;
    let mut skirt = |edge: Vec<(u32, u32)>| {
        let base = m.vertices.len() as u32;
        for &(x, z) in &edge {
            let p = pos(x, z);
            let nrm = normal(x, z);
            m.vertices.push(p);
            m.normals.push(nrm);
            m.uv.push(uv(x, z));
            m.vertices.push(p - Vector3::Y * depth);
            m.normals.push(nrm);
            m.uv.push(uv(x, z));
        }
        for k in 0..edge.len() as u32 - 1 {
            let (a, b, c, e) = (
                base + k * 2,
                base + k * 2 + 1,
                base + k * 2 + 2,
                base + k * 2 + 3,
            );
            // Both windings so the skirt is visible from either side.
            m.indices
                .extend_from_slice(&[a, c, b, b, c, e, a, b, c, b, e, c]);
        }
    };
    skirt(xs.iter().map(|&x| (x, z0)).collect());
    skirt(xs.iter().map(|&x| (x, z1)).collect());
    skirt(zs.iter().map(|&z| (x0, z)).collect());
    skirt(zs.iter().map(|&z| (x1, z)).collect());
    m.submeshes = vec![0..m.indices.len() as u32];
    m.tangents = vec![Vector4::new(1.0, 0.0, 0.0, 1.0); m.vertices.len()];
    m.recalculate_bounds();
    m
}

fn build_chunk(assets: &mut AssetServer, d: &TerrainData, id: AssetId, cx: u32, cz: u32) -> Chunk {
    let lods: [Handle<Mesh>; LODS] = std::array::from_fn(|l| {
        let mesh = chunk_mesh(d, cx, cz, l as u32);
        assets
            .meshes
            .insert(format!("terrain:{}/{cx}_{cz}_lod{l}", id.0), mesh)
    });
    let b = assets
        .meshes
        .get(&lods[0])
        .map(|m| m.bounds.center)
        .unwrap_or_default();
    let occluders: [Handle<Mesh>; 2] = std::array::from_fn(|i| {
        let step = OCCLUDER_STEPS[i];
        assets.meshes.insert(
            format!("terrain:{}/{cx}_{cz}_occluder{step}", id.0),
            occluder_mesh(d, cx, cz, step),
        )
    });
    Chunk {
        center: b,
        lods,
        occluders,
    }
}

/// Builds (or partly rebuilds) a TerrainData's chunk meshes: after a brush stroke only the
/// chunks under its dirty rectangles are rebuilt.
fn build(assets: &mut AssetServer, id: AssetId, old: Option<Built>) -> Option<Built> {
    let d = assets.terrains.get_by_id(id)?.clone();
    let per_side = (d.resolution - 1).div_ceil(CHUNK_CELLS);
    let shape = (d.size, d.resolution);
    let partial = old
        .filter(|o| o.size == shape && o.holes == d.versions.holes && !d.dirty_heights.is_empty());
    let chunks = match partial {
        Some(mut o) => {
            for cz in 0..per_side {
                for cx in 0..per_side {
                    // Chunk cells plus one sample around them (normals use neighbours).
                    let (x0, z0) = (
                        (cx * CHUNK_CELLS).saturating_sub(1),
                        (cz * CHUNK_CELLS).saturating_sub(1),
                    );
                    let (x1, z1) = ((cx + 1) * CHUNK_CELLS + 1, (cz + 1) * CHUNK_CELLS + 1);
                    let hit = d
                        .dirty_heights
                        .iter()
                        .any(|r| r[0] <= x1 && r[2] >= x0 && r[1] <= z1 && r[3] >= z0);
                    if hit {
                        o.chunks[(cz * per_side + cx) as usize] =
                            build_chunk(assets, &d, id, cx, cz);
                    }
                }
            }
            o.chunks
        }
        None => {
            let mut chunks = Vec::new();
            for cz in 0..per_side {
                for cx in 0..per_side {
                    chunks.push(build_chunk(assets, &d, id, cx, cz));
                }
            }
            chunks
        }
    };
    if let Some(dm) = assets.terrains.get_by_id_mut(id) {
        dm.dirty_heights.clear();
    }
    Some(Built {
        heights: d.versions.heights,
        holes: d.versions.holes,
        size: shape,
        chunks,
    })
}

/// Keeps chunk meshes and the TerrainCollider in sync with each Terrain's data.
#[derive(Default)]
pub struct TerrainSystem;

impl System for TerrainSystem {
    fn name(&self) -> &'static str {
        "Terrain"
    }

    /// Builds meshes and colliders before the first physics step.
    fn start(&mut self, world: &mut World) {
        self.always(world);
    }

    /// Streaming terrains load and unload around the main camera (Play mode).
    fn update(&mut self, world: &mut World) {
        let camera = world
            .find_with_tag("MainCamera")
            .into_iter()
            .next()
            .or_else(|| {
                world
                    .query::<oxide_scene::Camera>()
                    .map(|(id, _)| id)
                    .next()
            })
            .map(|id| world.position(id));
        if let Some(c) = camera {
            let (loaded, unloaded) = toolbox::stream(world, c);
            for p in loaded {
                oxide_core::debug_log!("Terrain streaming: loaded {p}");
            }
            for p in unloaded {
                oxide_core::debug_log!("Terrain streaming: unloaded {p}");
            }
        }
    }

    fn always(&mut self, world: &mut World) {
        let terrains: Vec<(GameObjectId, Handle<TerrainData>)> = world
            .query::<Terrain>()
            .filter_map(|(id, t)| t.data.clone().map(|d| (id, d)))
            .collect();
        if terrains.is_empty() {
            return;
        }
        let Some(mut assets) = world.resources.remove::<AssetServer>() else {
            return;
        };
        // Terrain Layer assets replace the data's layers (rebuilt when they change).
        for (_, t) in world.query::<Terrain>() {
            let (Some(h), false) = (t.data.clone(), t.layers.is_empty()) else {
                continue;
            };
            let wanted: Vec<TerrainLayer> = t
                .layers
                .iter()
                .map(|l| {
                    let _ = assets.data_asset(l);
                    let mut layer = assets.data.get(l).cloned().unwrap_or_default();
                    if layer.name.is_empty() {
                        layer.name = l.path().rsplit('/').next().unwrap_or("").to_owned();
                    }
                    layer
                })
                .collect();
            for l in &wanted {
                if let Some(d) = &l.diffuse
                    && let Err(e) = assets.ensure_loaded(d.path())
                {
                    oxide_core::debug_log_warning!("{e}");
                }
            }
            if assets.terrains.get(&h).is_some_and(|d| d.layers != wanted)
                && let Some(d) = assets.terrains.get_mut(&h)
            {
                d.layers = wanted;
                d.version += 1;
            }
        }
        let mut meshes = world
            .resources
            .remove::<TerrainMeshes>()
            .unwrap_or_default();
        for (id, h) in terrains {
            let Some(data) = assets.terrains.get(&h) else {
                continue;
            };
            let (hv, holes_v) = (data.versions.heights, data.versions.holes);
            let (heights, holes, size, res) = (
                data.heights.clone(),
                data.holes.clone(),
                data.size,
                data.resolution,
            );
            let hold = data.hold_collider;
            let version = hv.wrapping_mul(1_000_003).wrapping_add(holes_v);
            if meshes
                .built
                .get(&h.id())
                .is_none_or(|b| b.heights != hv || b.holes != holes_v || b.size != (size, res))
            {
                let old = meshes.built.remove(&h.id());
                if let Some(b) = build(&mut assets, h.id(), old) {
                    meshes.built.insert(h.id(), b);
                }
            }
            // Tree meshes and materials (models load on first use).
            if let Some(data) = assets.terrains.get(&h) {
                let paths: Vec<String> = data
                    .tree_prototypes
                    .iter()
                    .flat_map(|p| {
                        std::iter::once(p.mesh.path().to_owned())
                            .chain(p.materials.iter().map(|m| m.path().to_owned()))
                    })
                    .collect();
                for p in paths {
                    if !p.starts_with("builtin:")
                        && !p.starts_with("runtime:")
                        && !p.starts_with("terrain:")
                        && let Err(e) = assets.ensure_loaded(&p)
                    {
                        oxide_core::debug_log_warning!("{e}");
                    }
                }
            }
            // Heightfield collider (kept as is during a brush stroke).
            let needs = !hold
                && world.get_component::<TerrainCollider>(id).is_none_or(|c| {
                    c.version != version || !std::sync::Arc::ptr_eq(&c.heights, &heights)
                });
            if needs {
                let col = TerrainCollider {
                    size,
                    resolution: res,
                    heights,
                    holes,
                    version,
                    ..Default::default()
                };
                match world.get_component_mut::<TerrainCollider>(id) {
                    Some(c) => {
                        *c = TerrainCollider {
                            friction: c.friction,
                            material: c.material.clone(),
                            ..col
                        }
                    }
                    None => {
                        world.add_component(id, col);
                    }
                }
            }
        }
        world.resources.insert(meshes);
        world.resources.insert(assets);
    }
}

/// Detail cells per patch side (instances are generated and cached per patch).
const PATCH: u32 = 32;

/// One cached patch of detail instances (relative to the terrain origin).
#[derive(Default)]
struct DetailPatch {
    grass: Vec<(usize, GrassInstance)>,
    meshes: Vec<(usize, Matrix4x4)>,
}

/// Cached detail patches per TerrainData (rebuilt when the details or heights change).
#[derive(Default)]
pub struct DetailCache {
    inner: std::sync::Mutex<HashMap<AssetId, DetailEntry>>,
}

#[derive(Default)]
struct DetailEntry {
    key: (u64, u64, u64),
    patches: HashMap<(u32, u32), DetailPatch>,
}

fn hash3(x: u32, z: u32, i: u32) -> u32 {
    let mut h =
        x.wrapping_mul(0x8da6_b343) ^ z.wrapping_mul(0xd816_3841) ^ i.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^ (h >> 15)
}

fn unit(h: u32) -> f32 {
    (h & 0xffffff) as f32 / 16_777_215.0
}

/// Unity's detail scattering for one patch: per cell, the painted density instances, placed
/// at random inside the cell, sized and coloured by noise.
fn build_patch(d: &TerrainData, px: u32, pz: u32, density_scale: f32) -> DetailPatch {
    let mut out = DetailPatch::default();
    let s = d.detail_resolution.max(1);
    let cell = Vector2::new(d.size.x / s as f32, d.size.z / s as f32);
    for (pi, proto) in d.detail_prototypes.iter().enumerate() {
        let Some(map) = d.details.get(pi) else {
            continue;
        };
        let grass = proto.render_mode != oxide_assets::DetailRenderMode::VertexLit;
        let healthy = proto.healthy_color.linear();
        let dry = proto.dry_color.linear();
        let tint = d.wind.grass_tint.linear();
        for cz in pz * PATCH..((pz + 1) * PATCH).min(s) {
            for cx in px * PATCH..((px + 1) * PATCH).min(s) {
                let v = map[(cz * s + cx) as usize];
                if v == 0 {
                    continue;
                }
                let n = (v as f32 * proto.density * density_scale).round() as u32;
                for i in 0..n {
                    let h = hash3(cx, cz, i * 31 + pi as u32);
                    let fx = (cx as f32 + unit(h)) * cell.x;
                    let fz = (cz as f32 + unit(h.rotate_left(11))) * cell.y;
                    let (u, w) = (fx / d.size.x, fz / d.size.z);
                    if !d.is_solid_at(u, w) {
                        continue;
                    }
                    let y = d.interpolated_height(u, w);
                    let noise = oxide_assets::terrain::value_noise(
                        fx / proto.noise_spread.max(0.01),
                        fz / proto.noise_spread.max(0.01),
                        pi as u32 * 17 + 3,
                    ) * 0.5
                        + 0.5;
                    let width = proto.min_width + (proto.max_width - proto.min_width) * noise;
                    let height = proto.min_height + (proto.max_height - proto.min_height) * noise;
                    let rot = unit(h.rotate_left(22)) * std::f32::consts::TAU;
                    if grass {
                        let c = oxide_math::Color::lerp(healthy, dry, noise.clamp(0.0, 1.0));
                        let k = if proto.render_mode == oxide_assets::DetailRenderMode::Grass {
                            2.0
                        } else {
                            1.0
                        };
                        out.grass.push((
                            pi,
                            GrassInstance {
                                position: [fx, y, fz],
                                width,
                                height,
                                rotation: rot,
                                wind: d.wind.bending * 0.35,
                                mode: if k > 1.5 { 0.0 } else { 1.0 },
                                color: [
                                    c.r * tint.r * 1.6,
                                    c.g * tint.g * 1.6,
                                    c.b * tint.b * 1.6,
                                    d.wind.speed,
                                ],
                            },
                        ));
                    } else {
                        let up = if proto.align_to_ground > 0.0 {
                            Vector3::Y
                                .lerp(d.interpolated_normal(u, w), proto.align_to_ground)
                                .normalize()
                        } else {
                            Vector3::Y
                        };
                        let q = Quaternion::from_rotation_arc(Vector3::Y, up)
                            * Quaternion::from_rotation_y(rot);
                        out.meshes.push((
                            pi,
                            Matrix4x4::from_scale_rotation_translation(
                                Vector3::new(width, height, width),
                                q,
                                Vector3::new(fx, y, fz),
                            ),
                        ));
                    }
                }
            }
        }
    }
    out
}

/// Grass and detail meshes within each terrain's detail distance of `camera`.
pub fn extract_details(
    world: &World,
    camera: Vector3,
    grass: &mut Vec<GrassBatch>,
    draws: &mut Vec<DrawItem>,
) {
    let Some(cache) = world.try_resource::<DetailCache>() else {
        return;
    };
    let assets = world.resource::<AssetServer>();
    let mut cache = cache
        .inner
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for (id, t) in world.query::<Terrain>() {
        if !t.draw_trees || t.detail_distance <= 0.0 {
            continue;
        }
        let Some(h) = &t.data else { continue };
        let Some(d) = assets.terrains.get(h) else {
            continue;
        };
        if d.detail_prototypes.is_empty() {
            continue;
        }
        let origin = world.position(id);
        let layer = world.get(id).map_or(0, |o| o.layer);
        let entry = cache.entry(h.id()).or_default();
        let key = (
            d.versions.details,
            d.versions.heights.wrapping_add(d.versions.holes << 32),
            (t.detail_density * 1000.0) as u64,
        );
        if entry.key != key {
            entry.key = key;
            entry.patches.clear();
        }
        let s = d.detail_resolution.max(1);
        let patches = s.div_ceil(PATCH);
        let patch_size = Vector2::new(d.size.x / patches as f32, d.size.z / patches as f32);
        let local = camera - origin;
        let r = t.detail_distance;
        let p0x = ((local.x - r) / patch_size.x).floor().max(0.0) as u32;
        let p1x = (((local.x + r) / patch_size.x).ceil().max(0.0) as u32).min(patches);
        let p0z = ((local.z - r) / patch_size.y).floor().max(0.0) as u32;
        let p1z = (((local.z + r) / patch_size.y).ceil().max(0.0) as u32).min(patches);
        let mut per_proto: Vec<Vec<GrassInstance>> = vec![Vec::new(); d.detail_prototypes.len()];
        // Wind Zones make the grass sway harder (Unity: grass follows the terrain's wind
        // settings, scaled by the zones blowing over it).
        let wind_k = {
            let time = world
                .try_resource::<oxide_core::Time>()
                .map_or(0.0, |t| t.time());
            let center = origin + Vector3::new(d.size.x * 0.5, 0.0, d.size.z * 0.5);
            1.0 + oxide_scene::wind::wind_at(world, center, time).length() * 0.5
        };
        let r2 = r * r;
        for pz in p0z..p1z {
            for px in p0x..p1x {
                let patch = entry
                    .patches
                    .entry((px, pz))
                    .or_insert_with(|| build_patch(d, px, pz, t.detail_density));
                for (pi, g) in &patch.grass {
                    let p = Vector3::from_array(g.position) + origin;
                    let dist2 = p.distance_squared(camera);
                    if dist2 > r2 {
                        continue;
                    }
                    let mut g = *g;
                    g.position = p.to_array();
                    g.wind *= wind_k;
                    g.color[3] *= wind_k;
                    // Shrink into the ground near the end of the range (no popping).
                    let fade = (1.0 - (dist2.sqrt() / r - 0.8) / 0.2).clamp(0.0, 1.0);
                    g.height *= fade;
                    per_proto[*pi].push(g);
                }
                for (pi, m) in &patch.meshes {
                    let Some(proto) = d.detail_prototypes.get(*pi) else {
                        continue;
                    };
                    let Some(mesh) = &proto.mesh else { continue };
                    let model = Matrix4x4::from_translation(origin) * *m;
                    if model.w_axis.truncate().distance_squared(camera) > r2 {
                        continue;
                    }
                    let subs = assets
                        .meshes
                        .get(mesh)
                        .map_or(1, |m| m.submeshes.len().max(1));
                    for sub in 0..subs {
                        let mat = proto
                            .materials
                            .get(sub)
                            .or(proto.materials.last())
                            .map_or(AssetId::from_path(oxide_assets::DEFAULT_MATERIAL), |m| {
                                m.id()
                            });
                        draws.push(DrawItem {
                            id: 0,
                            mesh: mesh.id(),
                            submesh: sub as u32,
                            material: mat,
                            model,
                            cast_shadows: false,
                            shadows_only: false,
                            receive_shadows: true,
                            layer,
                            joint_offset: None,
                            occluder: false,
                            occludee: true,
                            reflection_probe_static: false,
                            lightmap: None,
                            light_probe: None,
                            sorting: Default::default(),
                            rendering_layers: u32::MAX,
                        });
                    }
                }
            }
        }
        for (pi, instances) in per_proto.into_iter().enumerate() {
            if instances.is_empty() {
                continue;
            }
            let texture = d.detail_prototypes[pi].texture.as_ref().map(|t| t.id());
            grass.push(GrassBatch { texture, instances });
        }
    }
}

/// Adds terrain chunks and trees to a render frame, choosing LODs for `camera`.
pub fn extract(
    world: &World,
    camera: Vector3,
    terrains: &mut Vec<TerrainDraw>,
    draws: &mut Vec<DrawItem>,
) {
    let Some(meshes) = world.try_resource::<TerrainMeshes>() else {
        return;
    };
    let assets = world.resource::<AssetServer>();
    for (id, t) in world.query::<Terrain>() {
        let Some(h) = &t.data else { continue };
        let Some(chunks) = meshes.built.get(&h.id()).map(|b| &b.chunks) else {
            continue;
        };
        let Some(data) = assets.terrains.get(h) else {
            continue;
        };
        let origin = world.position(id);
        let model = Matrix4x4::from_translation(origin);
        let layer = world.get(id).map_or(0, |o| o.layer);
        let chunks: &[Chunk] = if t.draw_heightmap { chunks } else { &[] };
        for c in chunks {
            let d = (origin + c.center).distance(camera);
            let lod = if d < t.lod_distance {
                0
            } else if d < t.lod_distance * 2.5 {
                1
            } else {
                2
            };
            terrains.push(TerrainDraw {
                data: h.id(),
                mesh: c.lods[lod].id(),
                model,
                receive_shadows: true,
                occluder: Some(c.occluders[(lod > 1) as usize].id()),
            });
            if t.cast_shadows {
                draws.push(DrawItem {
                    id: 0,
                    mesh: c.lods[(lod + 1).min(LODS - 1)].id(),
                    submesh: 0,
                    material: AssetId::from_path(oxide_assets::DEFAULT_MATERIAL),
                    model,
                    cast_shadows: true,
                    shadows_only: true,
                    receive_shadows: false,
                    layer,
                    joint_offset: None,
                    occluder: false,
                    occludee: false,
                    reflection_probe_static: true,
                    lightmap: None,
                    light_probe: None,
                    sorting: Default::default(),
                    rendering_layers: u32::MAX,
                });
            }
        }
        if !t.draw_trees {
            continue;
        }
        let max2 = t.tree_distance * t.tree_distance;
        let as_mesh = mesh_trees(t, data, origin, camera);
        let billboards = world.try_resource::<TreeBillboards>();
        for (ti, tree) in data.trees.iter().enumerate() {
            let Some(proto) = data.tree_prototypes.get(tree.prototype as usize) else {
                continue;
            };
            let local = Vector3::new(
                tree.position.x * data.size.x,
                0.0,
                tree.position.z * data.size.z,
            );
            let y = data.interpolated_height(tree.position.x, tree.position.z);
            let p = origin + Vector3::new(local.x, y, local.z);
            if p.distance_squared(camera) > max2 {
                continue;
            }
            // Far trees are billboards (when the prototype has one).
            if !as_mesh.contains(&ti)
                && billboards.is_some_and(|b| b.by_mesh.contains_key(&proto.mesh.id()))
            {
                continue;
            }
            let m = Matrix4x4::trs(p, tree_bend(world, proto.bend_factor, p, ti), Vector3::ONE)
                * Matrix4x4::trs(
                    Vector3::ZERO,
                    Quaternion::euler(0.0, tree.rotation, 0.0),
                    Vector3::new(tree.width_scale, tree.height_scale, tree.width_scale),
                );
            let subs = assets
                .meshes
                .get(&proto.mesh)
                .map_or(1, |m| m.submeshes.len().max(1));
            for s in 0..subs {
                let mat = proto
                    .materials
                    .get(s)
                    .or(proto.materials.last())
                    .map_or(AssetId::from_path(oxide_assets::DEFAULT_MATERIAL), |m| {
                        m.id()
                    });
                draws.push(DrawItem {
                    id: 0,
                    mesh: proto.mesh.id(),
                    submesh: s as u32,
                    material: mat,
                    model: m,
                    cast_shadows: true,
                    shadows_only: false,
                    receive_shadows: true,
                    layer,
                    joint_offset: None,
                    occluder: false,
                    occludee: true,
                    reflection_probe_static: true,
                    lightmap: None,
                    light_probe: None,
                    sorting: Default::default(),
                    rendering_layers: u32::MAX,
                });
            }
        }
    }
}

/// Trees drawn as meshes: the nearest within the billboard distance (plus the fade), at
/// most `max_mesh_trees`.
fn mesh_trees(
    t: &Terrain,
    data: &TerrainData,
    origin: Vector3,
    camera: Vector3,
) -> std::collections::HashSet<usize> {
    let range = t.billboard_start + t.fade_length.max(0.0);
    let mut near: Vec<(f32, usize)> = data
        .trees
        .iter()
        .enumerate()
        .filter_map(|(i, tree)| {
            let d2 = tree_position(data, origin, tree).distance_squared(camera);
            (d2 <= range * range).then_some((d2, i))
        })
        .collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    near.truncate(t.max_mesh_trees as usize);
    near.into_iter().map(|(_, i)| i).collect()
}

fn tree_position(
    data: &TerrainData,
    origin: Vector3,
    tree: &oxide_assets::terrain::TreeInstance,
) -> Vector3 {
    origin
        + Vector3::new(
            tree.position.x * data.size.x,
            data.interpolated_height(tree.position.x, tree.position.z),
            tree.position.z * data.size.z,
        )
}

/// How a tree leans in the wind: toward the wind, by its prototype's bend factor, swaying.
fn tree_bend(world: &World, bend: f32, at: Vector3, index: usize) -> Quaternion {
    if bend <= 0.0 {
        return Quaternion::IDENTITY;
    }
    let time = world
        .try_resource::<oxide_core::Time>()
        .map_or(0.0, |t| t.time());
    let wind = oxide_scene::wind::wind_at(world, at, time);
    let strength = wind.length();
    if strength < 1e-4 {
        return Quaternion::IDENTITY;
    }
    let phase = index as f32 * 1.618;
    let sway = 0.75 + 0.25 * (time * 1.7 + phase).sin();
    let angle = (strength * bend * 0.03 * sway).min(0.35);
    let axis = Vector3::Y.cross(wind / strength).normalize_or_zero();
    if axis == Vector3::ZERO {
        return Quaternion::IDENTITY;
    }
    Quaternion::from_axis_angle(axis, angle)
}

/// A tree prototype's billboard: an atlas of views around the tree (baked by the engine).
#[derive(Clone, Debug)]
pub struct TreeBillboard {
    pub texture: AssetId,
    pub width: f32,
    pub height: f32,
    /// How far below the tree's origin its image starts.
    pub bottom: f32,
    /// Texture rectangle per view angle, from the tree's front around +Y.
    pub images: Vec<[f32; 4]>,
}

/// Billboards of tree prototypes by mesh (a World resource).
#[derive(Default)]
pub struct TreeBillboards {
    pub by_mesh: HashMap<AssetId, TreeBillboard>,
    /// Prototypes that couldn't be baked (drawn as meshes at every distance).
    pub failed: std::collections::HashSet<AssetId>,
}

/// Distant terrain trees as camera-facing billboards (upright), fading in over the
/// terrain's fade length while the mesh is still drawn.
pub fn extract_tree_billboards(world: &World, camera: Vector3, out: &mut Vec<ParticleBatch>) {
    let Some(bb) = world.try_resource::<TreeBillboards>() else {
        return;
    };
    let assets = world.resource::<AssetServer>();
    let mut batches: HashMap<AssetId, Vec<ParticleInstance>> = HashMap::new();
    for (id, t) in world.query::<Terrain>() {
        if !t.draw_trees || !world.get(id).is_some_and(|o| o.active_in_hierarchy()) {
            continue;
        }
        let Some(data) = t.data.as_ref().and_then(|h| assets.terrains.get(h)) else {
            continue;
        };
        let origin = world.position(id);
        let max2 = t.tree_distance * t.tree_distance;
        let fade = t.fade_length.max(1e-3);
        let as_mesh = mesh_trees(t, data, origin, camera);
        for (ti, tree) in data.trees.iter().enumerate() {
            let Some(proto) = data.tree_prototypes.get(tree.prototype as usize) else {
                continue;
            };
            let Some(b) = bb.by_mesh.get(&proto.mesh.id()) else {
                continue;
            };
            let p = tree_position(data, origin, tree);
            let d2 = p.distance_squared(camera);
            if d2 > max2 {
                continue;
            }
            let alpha = if as_mesh.contains(&ti) {
                ((d2.sqrt() - t.billboard_start) / fade).clamp(0.0, 1.0)
            } else {
                1.0
            };
            if alpha <= 0.0 {
                continue;
            }
            let rot = Quaternion::euler(0.0, tree.rotation, 0.0);
            let uv = pick_image(&b.images, rot.inverse() * (camera - p));
            let (w, h) = (b.width * tree.width_scale, b.height * tree.height_scale);
            let center = p + Vector3::Y * (h * 0.5 - b.bottom * tree.height_scale);
            batches.entry(b.texture).or_default().push(
                ParticleInstance::billboard(center.to_array(), w, [1.0, 1.0, 1.0, alpha], 0.0)
                    .with_mode(ParticleMode::Vertical, [0.0; 3])
                    .with_uv(uv)
                    .with_aspect(h / w.max(1e-4)),
            );
        }
    }
    for (texture, particles) in batches {
        out.push(ParticleBatch {
            texture: Some(texture),
            blend: ParticleBlend::Alpha,
            particles,
        });
    }
}

/// The view a billboard shows for a camera in direction `to_camera` (tree space).
pub fn pick_image(images: &[[f32; 4]], to_camera: Vector3) -> [f32; 4] {
    if images.is_empty() {
        return [0.0, 0.0, 1.0, 1.0];
    }
    let angle = to_camera
        .x
        .atan2(to_camera.z)
        .rem_euclid(std::f32::consts::TAU);
    let i = ((angle / std::f32::consts::TAU * images.len() as f32).round() as usize) % images.len();
    images[i]
}

/// Unity `Terrain.SampleHeight`: world-space Y of the ground under `position` (None if off
/// every terrain).
pub fn sample_height(world: &World, position: Vector3) -> Option<f32> {
    let assets = world.resource::<AssetServer>();
    for (id, t) in world.query::<Terrain>() {
        let Some(data) = t.data.as_ref().and_then(|h| assets.terrains.get(h)) else {
            continue;
        };
        let o = world.position(id);
        let u = (position.x - o.x) / data.size.x;
        let v = (position.z - o.z) / data.size.z;
        if (0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v) {
            return Some(o.y + data.interpolated_height(u, v));
        }
    }
    None
}

/// The built-in tree (`builtin:tree/simple` with `builtin:tree/bark.mat` and
/// `builtin:tree/leaves.mat`), for the Add Tree menu and new terrains.
pub fn register_builtin_tree(assets: &mut AssetServer) {
    if assets.meshes.handle_of("builtin:tree/simple").is_some() {
        return;
    }
    assets
        .meshes
        .insert("builtin:tree/simple".to_owned(), simple_tree_mesh(9.0));
    for (path, c) in [
        (
            "builtin:tree/bark.mat",
            oxide_math::Color::rgb(0.33, 0.22, 0.14),
        ),
        (
            "builtin:tree/leaves.mat",
            oxide_math::Color::rgb(0.16, 0.36, 0.12),
        ),
    ] {
        let m = oxide_assets::Material {
            name: path
                .rsplit('/')
                .next()
                .unwrap_or(path)
                .trim_end_matches(".mat")
                .to_owned(),
            base_color: c,
            ..Default::default()
        };
        assets.materials.insert(path.to_owned(), m);
    }
}

/// Built-in procedural layer textures: `builtin:terrain/grass`, `rock`, `sand`, `snow`,
/// `dirt`, and the `grass_blades` detail texture.
pub fn register_builtin_textures(assets: &mut AssetServer) {
    if assets
        .textures
        .handle_of("builtin:terrain/grass_blades")
        .is_none()
    {
        assets.textures.insert(
            "builtin:terrain/grass_blades".to_owned(),
            grass_blades_texture(128),
        );
    }
    for (name, color, var, seed, streaks) in [
        ("grass", [0.24, 0.36, 0.12], 0.45, 11, false),
        ("rock", [0.42, 0.40, 0.38], 0.55, 23, true),
        ("sand", [0.72, 0.64, 0.46], 0.25, 37, false),
        ("snow", [0.92, 0.93, 0.96], 0.08, 41, false),
        ("dirt", [0.36, 0.27, 0.18], 0.4, 53, false),
    ] {
        let path = format!("builtin:terrain/{name}");
        if assets.textures.handle_of(&path).is_none() {
            assets
                .textures
                .insert(path, ground_texture(name, 512, color, var, seed, streaks));
        }
    }
}

pub fn default_layers() -> Vec<TerrainLayer> {
    let l = |name: &str, tile: f32, smooth: f32| TerrainLayer {
        name: name.into(),
        diffuse: Some(Handle::<Texture2D>::from_path(format!(
            "builtin:terrain/{name}"
        ))),
        tile_size: Vector2::splat(tile),
        smoothness: smooth,
        ..Default::default()
    };
    vec![
        l("grass", 8.0, 0.1),
        l("rock", 12.0, 0.25),
        l("sand", 6.0, 0.15),
        l("snow", 10.0, 0.4),
    ]
}

/// Creates a generated terrain object (Unity: GameObject > 3D Object > Terrain, then
/// sculpting): noise heights, auto texturing and a collider.
pub fn create_terrain(
    world: &mut World,
    name: &str,
    size: Vector3,
    resolution: u32,
    seed: u32,
) -> GameObjectId {
    let assets = world.resource_mut::<AssetServer>();
    register_builtin_textures(assets);
    let mut data = TerrainData::flat(size, resolution);
    data.name = name.into();
    data.generate(seed, 6, size.x.max(size.z) / 6.0, 0.6);
    data.splat_resolution = 512;
    data.auto_splat(0.06, 0.82, 32.0);
    data.layers = default_layers();
    let h = assets.terrains.add(data);
    let id = world.create(name);
    world.add_component(
        id,
        Terrain {
            data: Some(h),
            ..Default::default()
        },
    );
    id
}

/// Installs the terrain component, mesh cache and textures; returns the system.
pub fn install(world: &mut World) -> Box<dyn System> {
    world.resources.insert(DetailCache::default());
    world.registry.register::<Terrain>("3D Object/Terrain");
    world.resources.insert(TerrainMeshes::default());
    if let Some(a) = world.try_resource_mut::<AssetServer>() {
        register_builtin_textures(a);
        a.register_data_asset::<TerrainLayer>(
            oxide_assets::AssetTypeInfo::new("TerrainLayer", &["terrainlayer"])
                .menu("Terrain Layer", "New Terrain Layer", 45)
                .color(150, 200, 110),
        );
    }
    Box::new(TerrainSystem)
}

/// A simple low-poly tree mesh (trunk + layered canopy) with two sub-meshes:
/// 0 = bark, 1 = leaves. Handy for prototyping forests.
pub fn simple_tree_mesh(height: f32) -> Mesh {
    use oxide_assets::primitives::{cylinder, sphere};
    let place = |mut m: Mesh, pos: Vector3, scale: Vector3| {
        let mat = Matrix4x4::trs(pos, Quaternion::IDENTITY, scale);
        for v in &mut m.vertices {
            *v = mat.transform_point3(*v);
        }
        for n in &mut m.normals {
            *n = (*n / scale).normalize();
        }
        m.recalculate_tangents();
        m
    };
    let trunk_h = height * 0.45;
    let trunk = place(
        cylinder(0.5, 2.0, 8),
        Vector3::new(0.0, trunk_h * 0.5, 0.0),
        Vector3::new(height * 0.06, trunk_h * 0.5, height * 0.06),
    );
    let mut leaves = place(
        sphere(0.5, 10, 6),
        Vector3::new(0.0, height * 0.55, 0.0),
        Vector3::new(height * 0.5, height * 0.45, height * 0.5),
    );
    leaves.append(&place(
        sphere(0.5, 10, 6),
        Vector3::new(0.0, height * 0.82, 0.0),
        Vector3::new(height * 0.32, height * 0.3, height * 0.32),
    ));
    // Merge the canopy into one sub-mesh.
    let n = leaves.indices.len() as u32;
    leaves.submeshes = vec![0..n];
    let mut tree = trunk;
    tree.append(&leaves);
    tree.name = "Tree".into();
    tree
}

/// A clump of grass blades on transparency (the built-in grass detail texture).
pub fn grass_blades_texture(size: u32) -> Texture2D {
    let mut px = vec![0u8; (size * size * 4) as usize];
    let mut rng = 1234u32;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        rng as f32 / u32::MAX as f32
    };
    let s = size as f32;
    for _ in 0..28 {
        let base = 0.15 + next() * 0.7;
        let tip = base + (next() - 0.5) * 0.35;
        let height = 0.55 + next() * 0.45;
        let width = 0.025 + next() * 0.02;
        let shade = 0.75 + next() * 0.25;
        for y in 0..size {
            // Row 0 is the top of the image; blades grow up from the bottom.
            let t = 1.0 - y as f32 / s;
            if t > height {
                continue;
            }
            let k = t / height;
            let cx = base + (tip - base) * k * k;
            let w = width * (1.0 - k).max(0.05);
            for x in 0..size {
                let d = (x as f32 / s - cx).abs();
                if d < w {
                    let i = ((y * size + x) * 4) as usize;
                    let g = (0.55 + 0.45 * k) * shade;
                    px[i] = (70.0 * g) as u8;
                    px[i + 1] = (150.0 * g) as u8;
                    px[i + 2] = (40.0 * g) as u8;
                    px[i + 3] = 255;
                }
            }
        }
    }
    Texture2D::from_rgba8("Grass", size, size, px, true)
}
