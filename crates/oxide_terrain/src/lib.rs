//! Oxide terrain (Unity `Terrain` + `TerrainCollider`): a `TerrainData` asset rendered as
//! LOD'd chunks with crack-hiding skirts and the splat-layer shader, trees drawn instanced
//! with distance culling, and a heightfield collider kept in sync.
//! Reference: <https://docs.unity3d.com/Manual/script-Terrain.html>

// Submesh lists are `Vec<Range<u32>>`; a single-entry list is the normal case.
#![allow(clippy::single_range_in_vec_init)]

use oxide_assets::{
    AssetId, AssetServer, Handle, Mesh, TerrainData, TerrainLayer, Texture2D,
    terrain::ground_texture,
};
use oxide_core::Reflect;
use oxide_math::{
    Color, Matrix4x4, Matrix4x4Ext, Quaternion, QuaternionExt, Vector2, Vector3, Vector4,
};
use oxide_physics::TerrainCollider;
use oxide_render::{DrawItem, TerrainDraw};
use oxide_scene::{Component, GameObjectId, System, World};
use std::collections::HashMap;

/// Cells per chunk side at full detail.
pub const CHUNK_CELLS: u32 = 32;
pub const LODS: usize = 3;

/// Unity `Terrain`.
#[derive(Clone, Debug, Reflect)]
pub struct Terrain {
    pub data: Option<Handle<TerrainData>>,
    pub cast_shadows: bool,
    /// Distance (m) where chunks drop to half detail (and again at twice this).
    pub lod_distance: f32,
    pub draw_trees: bool,
    /// Trees beyond this distance from the camera are not drawn.
    pub tree_distance: f32,
}

impl Default for Terrain {
    fn default() -> Self {
        Self {
            data: None,
            cast_shadows: true,
            lod_distance: 120.0,
            draw_trees: true,
            tree_distance: 400.0,
        }
    }
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
    built: HashMap<AssetId, (u64, Vec<Chunk>)>,
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

fn build(assets: &mut AssetServer, id: AssetId) -> Option<(u64, Vec<Chunk>)> {
    let d = assets.terrains.get_by_id(id)?.clone();
    let chunks_x = (d.resolution - 1).div_ceil(CHUNK_CELLS);
    let mut chunks = Vec::new();
    for cz in 0..chunks_x {
        for cx in 0..chunks_x {
            let lods: [Handle<Mesh>; LODS] = std::array::from_fn(|l| {
                let mesh = chunk_mesh(&d, cx, cz, l as u32);
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
                    occluder_mesh(&d, cx, cz, step),
                )
            });
            chunks.push(Chunk {
                center: b,
                lods,
                occluders,
            });
        }
    }
    Some((d.version, chunks))
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
        let mut meshes = world
            .resources
            .remove::<TerrainMeshes>()
            .unwrap_or_default();
        for (id, h) in terrains {
            let Some(data) = assets.terrains.get(&h) else {
                continue;
            };
            let version = data.version;
            let (heights, size, res) = (data.heights.clone(), data.size, data.resolution);
            if meshes.built.get(&h.id()).is_none_or(|b| b.0 != version)
                && let Some(b) = build(&mut assets, h.id())
            {
                meshes.built.insert(h.id(), b);
            }
            // Heightfield collider.
            let needs = world.get_component::<TerrainCollider>(id).is_none_or(|c| {
                c.version != version || !std::sync::Arc::ptr_eq(&c.heights, &heights)
            });
            if needs {
                let col = TerrainCollider {
                    size,
                    resolution: res,
                    heights,
                    version,
                    ..Default::default()
                };
                match world.get_component_mut::<TerrainCollider>(id) {
                    Some(c) => {
                        *c = TerrainCollider {
                            friction: c.friction,
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
        let Some((_, chunks)) = meshes.built.get(&h.id()) else {
            continue;
        };
        let Some(data) = assets.terrains.get(h) else {
            continue;
        };
        let origin = world.position(id);
        let model = Matrix4x4::from_translation(origin);
        let layer = world.get(id).map_or(0, |o| o.layer);
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
                });
            }
        }
        if !t.draw_trees {
            continue;
        }
        let max2 = t.tree_distance * t.tree_distance;
        for tree in &data.trees {
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
            let m = Matrix4x4::trs(
                p,
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
                });
            }
        }
    }
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

/// Built-in procedural layer textures: `builtin:terrain/grass`, `rock`, `sand`, `snow`.
pub fn register_builtin_textures(assets: &mut AssetServer) {
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
        tint: Color::WHITE,
        smoothness: smooth,
        metallic: 0.0,
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
    world.registry.register::<Terrain>("3D Object/Terrain");
    world.resources.insert(TerrainMeshes::default());
    if let Some(a) = world.try_resource_mut::<AssetServer>() {
        register_builtin_textures(a);
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
