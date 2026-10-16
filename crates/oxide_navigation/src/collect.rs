//! Gathers the geometry a NavMeshSurface bakes from (Unity's `NavMeshBuilder.CollectSources`):
//! render meshes and terrains, or physics colliders, filtered by layer, collection mode and
//! NavMeshModifiers, in the surface's local space.

use crate::build::{AreaVolume, BuildSettings, BuildSource, TileGrid};
use crate::components::{
    CollectObjects, NavMeshAgent, NavMeshCollectGeometry, NavMeshModifier, NavMeshModifierVolume,
    NavMeshObstacle, NavMeshSurface,
};
use crate::settings::{NOT_WALKABLE, NavigationSettings};
use oxide_assets::{AssetServer, Mesh};
use oxide_math::{Matrix4x4, Quaternion, Vector3};
use oxide_physics::{
    BoxCollider, CapsuleCollider, CapsuleDirection, MeshCollider, SphereCollider, TerrainCollider,
};
use oxide_scene::{GameObjectId, MeshFilter, MeshRenderer, SkinnedMeshRenderer, World};

/// Everything needed to (re)build a surface's tiles.
#[derive(Clone, Debug)]
pub struct SurfaceBuildInput {
    pub source: BuildSource,
    pub volumes: Vec<AreaVolume>,
    pub settings: BuildSettings,
    pub grid: TileGrid,
    pub agent_type: i32,
    pub position: Vector3,
    pub rotation: Quaternion,
}

fn to_area(a: i32) -> Option<u8> {
    (a != NOT_WALKABLE && (0..32).contains(&a)).then_some(a as u8)
}

/// Build settings for a surface (its agent type plus its overrides).
pub fn surface_settings(world: &World, s: &NavMeshSurface) -> BuildSettings {
    let mut b = world.resources.get::<NavigationSettings>().map_or_else(
        || NavigationSettings::default().build_settings(s.agent_type_id),
        |n| n.build_settings(s.agent_type_id),
    );
    if s.override_voxel_size {
        b.voxel_size = s.voxel_size.max(0.01);
    }
    if s.override_tile_size {
        b.tile_size = s.tile_size.clamp(16, 1024);
    }
    b.min_region_area = s.min_region_area.max(0.0);
    b
}

/// The modifier affecting `id`: its own, or the nearest ancestor's that applies to children.
fn modifier(world: &World, id: GameObjectId) -> Option<&NavMeshModifier> {
    if let Some(m) = world.get_component::<NavMeshModifier>(id) {
        return Some(m);
    }
    let mut p = world.parent(id);
    while let Some(pid) = p {
        if let Some(m) = world.get_component::<NavMeshModifier>(pid)
            && m.apply_to_children
        {
            return Some(m);
        }
        p = world.parent(pid);
    }
    None
}

/// Agents and obstacles (and their children) are dynamic and never baked.
fn is_dynamic(world: &World, id: GameObjectId) -> bool {
    let mut cur = Some(id);
    while let Some(c) = cur {
        if world.get_component::<NavMeshAgent>(c).is_some()
            || world.get_component::<NavMeshObstacle>(c).is_some()
        {
            return true;
        }
        cur = world.parent(c);
    }
    false
}

fn is_descendant(world: &World, id: GameObjectId, root: GameObjectId) -> bool {
    let mut cur = Some(id);
    while let Some(c) = cur {
        if c == root {
            return true;
        }
        cur = world.parent(c);
    }
    false
}

pub fn collect(
    world: &World,
    surface_id: GameObjectId,
    s: &NavMeshSurface,
) -> Result<SurfaceBuildInput, String> {
    let settings = surface_settings(world, s);
    let position = world.position(surface_id);
    let rotation = world.rotation(surface_id);
    let inv = rotation.inverse();
    let to_local = |p: Vector3| inv * (p - position);
    let assets = world
        .resources
        .get::<AssetServer>()
        .ok_or("no AssetServer")?;
    let mut src = BuildSource::default();
    let objects: Vec<GameObjectId> = world
        .ids()
        .into_iter()
        .filter(|&id| world.get(id).is_some_and(|o| o.active_in_hierarchy()))
        .filter(|&id| match s.collect_objects {
            CollectObjects::Children => is_descendant(world, id, surface_id),
            CollectObjects::MarkedWithModifier => modifier(world, id).is_some(),
            _ => true,
        })
        .filter(|&id| {
            world
                .get(id)
                .is_some_and(|o| s.layer_mask & (1 << (o.layer & 31)) != 0)
        })
        .filter(|&id| !is_dynamic(world, id))
        .collect();
    let add_mesh = |src: &mut BuildSource, m: &Mesh, model: Matrix4x4, area: Option<u8>| {
        let verts: Vec<Vector3> = m
            .vertices
            .iter()
            .map(|&v| to_local(model.transform_point3(v)))
            .collect();
        let tris: Vec<[u32; 3]> = m
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .filter(|t| t.iter().all(|&i| (i as usize) < verts.len()))
            .copied()
            .collect();
        src.add_triangles(&verts, &tris, area);
    };
    for id in objects {
        let m = modifier(world, id);
        if m.is_some_and(|m| m.ignore_from_build) {
            continue;
        }
        let area = to_area(match m {
            Some(m) if m.override_area => m.area,
            _ => s.default_area,
        });
        let model = world.local_to_world(id);
        match s.use_geometry {
            NavMeshCollectGeometry::RenderMeshes => {
                if world.get_component::<SkinnedMeshRenderer>(id).is_some() {
                    continue;
                }
                if world.get_component::<MeshRenderer>(id).is_some()
                    && let Some(mesh) = world
                        .get_component::<MeshFilter>(id)
                        .and_then(|f| f.mesh.as_ref())
                        .and_then(|h| assets.meshes.get(h))
                {
                    add_mesh(&mut src, mesh, model, area);
                }
                if let Some(t) = world.get_component::<oxide_terrain::Terrain>(id)
                    && let Some(d) = t.data.as_ref().and_then(|h| assets.terrains.get(h))
                {
                    let (v, i) = heightfield_tris(&d.heights, d.resolution, d.size);
                    let o = world.position(id);
                    let v: Vec<Vector3> = v.into_iter().map(|p| to_local(p + o)).collect();
                    src.add_triangles(&v, &i, area);
                }
            }
            NavMeshCollectGeometry::PhysicsColliders => {
                let mut shapes: Vec<(Vec<Vector3>, Vec<[u32; 3]>)> = Vec::new();
                if let Some(b) = world.get_component::<BoxCollider>(id)
                    && !b.is_trigger
                {
                    shapes.push(box_tris(b.center, b.size * 0.5));
                }
                if let Some(c) = world.get_component::<SphereCollider>(id)
                    && !c.is_trigger
                {
                    shapes.push(capsule_tris(
                        c.center,
                        c.radius,
                        0.0,
                        CapsuleDirection::YAxis,
                    ));
                }
                if let Some(c) = world.get_component::<CapsuleCollider>(id)
                    && !c.is_trigger
                {
                    let half = (c.height * 0.5 - c.radius).max(0.0);
                    shapes.push(capsule_tris(c.center, c.radius, half, c.direction));
                }
                if let Some(c) = world.get_component::<MeshCollider>(id)
                    && !c.is_trigger
                    && let Some(mesh) = c.mesh.as_ref().and_then(|h| assets.meshes.get(h))
                {
                    add_mesh(&mut src, mesh, model, area);
                }
                for (v, i) in shapes {
                    let v: Vec<Vector3> = v
                        .into_iter()
                        .map(|p| to_local(model.transform_point3(p)))
                        .collect();
                    src.add_triangles(&v, &i, area);
                }
                if let Some(t) = world.get_component::<TerrainCollider>(id) {
                    let (v, i) = heightfield_tris(&t.heights, t.resolution, t.size);
                    let o = world.position(id);
                    let v: Vec<Vector3> = v.into_iter().map(|p| to_local(p + o)).collect();
                    src.add_triangles(&v, &i, area);
                }
            }
        }
    }
    let volumes = world
        .query::<NavMeshModifierVolume>()
        .map(|(id, v)| {
            let scale = world.lossy_scale(id).abs();
            AreaVolume {
                center: to_local(world.transform_point(id, v.center)),
                half: v.size * scale * 0.5,
                rotation: inv * world.rotation(id),
                cylinder: false,
                area: to_area(v.area),
            }
        })
        .collect();
    let (mut bmin, mut bmax) = src.bounds().ok_or(
        "nothing to bake: no geometry found (check Use Geometry, layers and Collect Objects)",
    )?;
    if s.collect_objects == CollectObjects::Volume {
        let half = s.size.abs() * 0.5;
        bmin = bmin.max(s.center - half);
        bmax = bmax.min(s.center + half);
        if bmin.x >= bmax.x || bmin.z >= bmax.z {
            return Err("the surface's volume contains no geometry".into());
        }
    }
    let grid = TileGrid::new(bmin, bmax, &settings);
    Ok(SurfaceBuildInput {
        source: src,
        volumes,
        settings,
        grid,
        agent_type: s.agent_type_id,
        position,
        rotation,
    })
}

/// Triangles of a terrain height grid (object space).
fn heightfield_tris(heights: &[f32], res: u32, size: Vector3) -> (Vec<Vector3>, Vec<[u32; 3]>) {
    let n = res.max(2) as usize;
    if heights.len() < n * n {
        return (Vec::new(), Vec::new());
    }
    let mut v = Vec::with_capacity(n * n);
    for z in 0..n {
        for x in 0..n {
            v.push(Vector3::new(
                x as f32 / (n - 1) as f32 * size.x,
                heights[z * n + x] * size.y,
                z as f32 / (n - 1) as f32 * size.z,
            ));
        }
    }
    let mut t = Vec::with_capacity((n - 1) * (n - 1) * 2);
    for z in 0..n - 1 {
        for x in 0..n - 1 {
            let a = (z * n + x) as u32;
            let b = a + n as u32;
            // Up-facing (clockwise seen from above).
            t.push([a, b, b + 1]);
            t.push([a, b + 1, a + 1]);
        }
    }
    (v, t)
}

/// Orients each triangle of a convex shape outwards.
fn outward(v: &[Vector3], tris: &mut [[u32; 3]], center: Vector3) {
    for t in tris.iter_mut() {
        let [a, b, c] = t.map(|i| v[i as usize]);
        let n = (b - a).cross(c - a);
        if n.dot((a + b + c) / 3.0 - center) < 0.0 {
            t.swap(1, 2);
        }
    }
}

fn box_tris(c: Vector3, h: Vector3) -> (Vec<Vector3>, Vec<[u32; 3]>) {
    let v: Vec<Vector3> = (0..8)
        .map(|i| {
            c + h * Vector3::new(
                if i & 1 != 0 { 1.0 } else { -1.0 },
                if i & 2 != 0 { 1.0 } else { -1.0 },
                if i & 4 != 0 { 1.0 } else { -1.0 },
            )
        })
        .collect();
    let mut t = vec![
        [0, 1, 3],
        [0, 3, 2],
        [4, 6, 7],
        [4, 7, 5],
        [0, 4, 5],
        [0, 5, 1],
        [2, 3, 7],
        [2, 7, 6],
        [0, 2, 6],
        [0, 6, 4],
        [1, 5, 7],
        [1, 7, 3],
    ];
    outward(&v, &mut t, c);
    (v, t)
}

/// A sphere (`half = 0`) or capsule with `half` the distance from the centre to each cap.
fn capsule_tris(
    c: Vector3,
    r: f32,
    half: f32,
    dir: CapsuleDirection,
) -> (Vec<Vector3>, Vec<[u32; 3]>) {
    let (rings, segs) = (8usize, 12usize);
    let mut v = Vec::new();
    for i in 0..=rings {
        let phi = i as f32 / rings as f32 * std::f32::consts::PI;
        let y = r * phi.cos() + if i <= rings / 2 { half } else { -half };
        let rr = r * phi.sin();
        for j in 0..segs {
            let th = j as f32 / segs as f32 * std::f32::consts::TAU;
            let p = Vector3::new(rr * th.cos(), y, rr * th.sin());
            let p = match dir {
                CapsuleDirection::XAxis => Vector3::new(p.y, p.x, p.z),
                CapsuleDirection::YAxis => p,
                CapsuleDirection::ZAxis => Vector3::new(p.x, p.z, p.y),
            };
            v.push(c + p);
        }
    }
    let mut t = Vec::new();
    for i in 0..rings {
        for j in 0..segs {
            let a = (i * segs + j) as u32;
            let b = (i * segs + (j + 1) % segs) as u32;
            let cc = a + segs as u32;
            let d = b + segs as u32;
            t.push([a, cc, d]);
            t.push([a, d, b]);
        }
    }
    outward(&v, &mut t, c);
    (v, t)
}
