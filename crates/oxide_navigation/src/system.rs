//! Keeps the runtime NavMesh in sync with the scene: surfaces' data, NavMeshLinks, carving
//! NavMeshObstacles; and runs the agents after `Update`.

use crate::build::{AreaVolume, build_tile, tile_triangles};
use crate::collect::{SurfaceBuildInput, collect};
use crate::components::{NavMeshLink, NavMeshObstacle, NavMeshObstacleShape, NavMeshSurface};
use crate::data::NavMeshData;
use crate::navmesh::{NavMesh, NavMeshDataInstance, OffMeshLink};
use crate::settings::NavigationSettings;
use oxide_assets::AssetServer;
use oxide_core::Time;
use oxide_math::{Quaternion, Vector3};
use oxide_scene::{ComponentId, GameObjectId, System, World};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

struct SurfaceEntry {
    instance: NavMeshDataInstance,
    data: usize,
    position: Vector3,
    rotation: Quaternion,
}

type LinkKey = (Vector3, Vector3, u32, u32, bool, i32, i32, bool);
type SurfaceKey = (GameObjectId, ComponentId);

#[derive(Default)]
struct CarveState {
    last_pos: Option<Vector3>,
    still_for: f32,
    /// The volume currently cut into the NavMesh.
    carved: Option<AreaVolume>,
}

/// World resource: loaded NavMesh files and what is registered on the NavMesh.
#[derive(Default)]
pub struct NavigationRuntime {
    /// Loaded `.navmesh` files by asset path.
    pub cache: HashMap<String, Arc<NavMeshData>>,
    /// Geometry of baked surfaces, for rebuilding tiles when obstacles carve.
    pub sources: HashMap<GameObjectId, Arc<SurfaceBuildInput>>,
    surfaces: HashMap<(GameObjectId, ComponentId), SurfaceEntry>,
    links: HashMap<(GameObjectId, ComponentId), (u32, LinkKey)>,
    carving: HashMap<(GameObjectId, ComponentId), CarveState>,
    failed: HashSet<String>,
    settings_seen: Option<NavigationSettings>,
}

pub struct NavigationSystem;

impl System for NavigationSystem {
    fn name(&self) -> &'static str {
        "Navigation"
    }

    fn always(&mut self, world: &mut World) {
        if let Some(r) = crate::bake::poll(world) {
            match r {
                Ok(m) => oxide_core::debug_log!("{m}"),
                Err(e) => oxide_core::debug_log_error!("NavMesh bake failed: {e}"),
            }
        }
        sync(world);
    }

    fn update(&mut self, world: &mut World) {
        let dt = world
            .resources
            .get::<Time>()
            .map_or(0.0, |t| t.delta_time());
        carve(world, dt);
        let Some(nav) = world.resources.remove::<NavMesh>() else {
            return;
        };
        if dt > 0.0 {
            crate::agent::update_agents(world, &nav, dt);
        }
        world.resources.insert(nav);
    }
}

/// Loads (once) the NavMesh file a surface references.
fn load(world: &mut World, path: &str) -> Option<Arc<NavMeshData>> {
    let rt = world.resources.get::<NavigationRuntime>()?;
    if let Some(d) = rt.cache.get(path) {
        return Some(d.clone());
    }
    if rt.failed.contains(path) {
        return None;
    }
    let root = world.resources.get::<AssetServer>()?.root.clone();
    let res = std::fs::read(root.join(path))
        .map_err(|e| e.to_string())
        .and_then(|b| NavMeshData::from_bytes(&b));
    let rt = world.resources.get_mut::<NavigationRuntime>()?;
    match res {
        Ok(d) => {
            let d = Arc::new(d);
            rt.cache.insert(path.to_owned(), d.clone());
            Some(d)
        }
        Err(e) => {
            oxide_core::debug_log_error!("Could not load NavMesh {path}: {e}");
            rt.failed.insert(path.to_owned());
            None
        }
    }
}

/// Mirrors surfaces, links and project settings onto the NavMesh.
pub(crate) fn sync(world: &mut World) {
    if !world.resources.contains::<NavigationRuntime>() {
        world.resources.insert(NavigationRuntime::default());
    }
    if !world.resources.contains::<NavMesh>() {
        world.resources.insert(NavMesh::new());
    }
    // Project settings → area costs and agent types.
    let settings = world
        .resources
        .get::<NavigationSettings>()
        .cloned()
        .unwrap_or_default();
    let changed = world
        .resources
        .get::<NavigationRuntime>()
        .is_none_or(|rt| rt.settings_seen.as_ref() != Some(&settings));
    if changed {
        if let Some(nav) = world.resources.get_mut::<NavMesh>() {
            nav.area_costs = settings.costs();
            nav.area_names = settings.areas.iter().map(|a| a.name.clone()).collect();
            nav.agent_types = settings
                .agents
                .iter()
                .map(|a| (a.id, settings.build_settings(a.id)))
                .collect();
            nav.version += 1;
        }
        if let Some(rt) = world.resources.get_mut::<NavigationRuntime>() {
            rt.settings_seen = Some(settings);
        }
    }
    // Surfaces.
    let mut wanted: Vec<(SurfaceKey, Arc<NavMeshData>, Vector3, Quaternion)> = Vec::new();
    for (id, cid) in world.query_ids::<NavMeshSurface>() {
        let Some(s) = world.component::<NavMeshSurface>(id, cid) else {
            continue;
        };
        let data = match (&s.runtime.data, &s.navmesh_data) {
            (Some(d), _) => Some(d.clone()),
            (None, Some(h)) => {
                let p = h.path().to_owned();
                load(world, &p)
            }
            _ => None,
        };
        if let Some(d) = data {
            wanted.push(((id, cid), d, world.position(id), world.rotation(id)));
        }
    }
    let Some(mut rt) = world.resources.remove::<NavigationRuntime>() else {
        return;
    };
    let Some(mut nav) = world.resources.remove::<NavMesh>() else {
        world.resources.insert(rt);
        return;
    };
    let keys: HashSet<_> = wanted.iter().map(|w| w.0).collect();
    rt.surfaces.retain(|k, e| {
        let keep = keys.contains(k);
        if !keep {
            nav.remove_data(e.instance);
        }
        keep
    });
    for (key, data, pos, rot) in wanted {
        let ptr = Arc::as_ptr(&data) as usize;
        if let Some(e) = rt.surfaces.get(&key) {
            if e.data == ptr
                && e.position.distance(pos) < 1e-4
                && e.rotation.dot(rot).abs() > 0.99999
                && nav.is_valid(e.instance)
            {
                continue;
            }
            nav.remove_data(e.instance);
        }
        let instance = nav.add_data(data, pos, rot);
        rt.surfaces.insert(
            key,
            SurfaceEntry {
                instance,
                data: ptr,
                position: pos,
                rotation: rot,
            },
        );
    }
    // Links.
    let mut seen = HashSet::new();
    for (id, cid) in world.query_ids::<NavMeshLink>() {
        let Some(l) = world.component::<NavMeshLink>(id, cid) else {
            continue;
        };
        let start = world.transform_point(id, l.start_point);
        let end = world.transform_point(id, l.end_point);
        let key: LinkKey = (
            start,
            end,
            l.width.to_bits(),
            l.cost_modifier.to_bits(),
            l.bidirectional,
            l.area,
            l.agent_type_id,
            l.activated,
        );
        seen.insert((id, cid));
        if let Some((lid, k)) = rt.links.get(&(id, cid))
            && *k == key
            && nav.link(*lid).is_some()
        {
            continue;
        }
        if let Some((lid, _)) = rt.links.remove(&(id, cid)) {
            nav.remove_link(lid);
        }
        let lid = nav.add_link(OffMeshLink {
            start,
            end,
            width: l.width.max(0.0),
            bidirectional: l.bidirectional,
            area: l.area.clamp(0, 31) as u8,
            cost_modifier: l.cost_modifier,
            agent_type: l.agent_type_id,
            activated: l.activated,
            owner: id.to_bits(),
            start_poly: None,
            end_poly: None,
        });
        rt.links.insert((id, cid), (lid, key));
    }
    rt.links.retain(|k, (lid, _)| {
        let keep = seen.contains(k);
        if !keep {
            nav.remove_link(*lid);
        }
        keep
    });
    world.resources.insert(nav);
    world.resources.insert(rt);
}

/// The volume an obstacle cuts out, in world space.
fn obstacle_volume(world: &World, id: GameObjectId, o: &NavMeshObstacle) -> AreaVolume {
    let s = world.lossy_scale(id).abs();
    let center = world.transform_point(id, o.center);
    match o.shape {
        NavMeshObstacleShape::Box => AreaVolume {
            center,
            half: o.size.abs() * s * 0.5,
            rotation: world.rotation(id),
            cylinder: false,
            area: None,
        },
        NavMeshObstacleShape::Capsule => AreaVolume {
            center,
            half: Vector3::new(o.radius * s.x.max(s.z), o.height * s.y * 0.5, 0.0),
            rotation: Quaternion::IDENTITY,
            cylinder: true,
            area: None,
        },
    }
}

fn volume_bounds(v: &AreaVolume) -> (Vector3, Vector3) {
    let ext = if v.cylinder {
        Vector3::new(v.half.x, v.half.y, v.half.x)
    } else {
        let m = oxide_math::glam::Mat3::from_quat(v.rotation);
        m.x_axis.abs() * v.half.x + m.y_axis.abs() * v.half.y + m.z_axis.abs() * v.half.z
    };
    (v.center - ext, v.center + ext)
}

/// Tracks obstacle movement and rebuilds the NavMesh tiles that carving obstacles touch.
fn carve(world: &mut World, dt: f32) {
    let ids = world.query_ids::<NavMeshObstacle>();
    let Some(mut rt) = world.resources.remove::<NavigationRuntime>() else {
        return;
    };
    let mut dirty: Vec<(Vector3, Vector3)> = Vec::new();
    let mut live = HashSet::new();
    for (id, cid) in ids {
        live.insert((id, cid));
        let pos = world.position(id);
        let Some(o) = world.component::<NavMeshObstacle>(id, cid).cloned() else {
            continue;
        };
        let st = rt.carving.entry((id, cid)).or_default();
        let vel = match (st.last_pos, dt > 0.0) {
            (Some(l), true) => (pos - l) / dt,
            _ => Vector3::ZERO,
        };
        st.last_pos = Some(pos);
        if vel.length() < 0.05 {
            st.still_for += dt;
        } else {
            st.still_for = 0.0;
        }
        let vol = obstacle_volume(world, id, &o);
        let want = o.carve && (!o.carve_only_stationary || st.still_for >= o.time_to_stationary);
        let moved = st.carved.is_some_and(|c| {
            c.center.distance(vol.center) > o.move_threshold
                || c.rotation.dot(vol.rotation).abs() < 0.9999
                || (c.half - vol.half).length() > 1e-3
        });
        if (want && (st.carved.is_none() || moved)) || (!want && st.carved.is_some()) {
            if let Some(old) = st.carved.take() {
                dirty.push(volume_bounds(&old));
            }
            if want {
                dirty.push(volume_bounds(&vol));
                st.carved = Some(vol);
            }
        }
        let carved = st.carved.is_some();
        if let Some(c) = world.component_mut::<NavMeshObstacle>(id, cid) {
            c.runtime.velocity = vel;
            c.runtime.carved_at = carved.then_some((vol.center, vol.rotation));
        }
    }
    rt.carving.retain(|k, st| {
        let keep = live.contains(k);
        if !keep && let Some(c) = st.carved.take() {
            dirty.push(volume_bounds(&c));
        }
        keep
    });
    if dirty.is_empty() {
        world.resources.insert(rt);
        return;
    }
    let holes: Vec<AreaVolume> = rt.carving.values().filter_map(|s| s.carved).collect();
    let Some(mut nav) = world.resources.remove::<NavMesh>() else {
        world.resources.insert(rt);
        return;
    };
    let surfaces: Vec<((GameObjectId, ComponentId), NavMeshDataInstance)> =
        rt.surfaces.iter().map(|(k, e)| (*k, e.instance)).collect();
    for ((sid, scid), inst) in surfaces {
        let Some(data) = nav.instance_data(inst).cloned() else {
            continue;
        };
        let input = match rt.sources.get(&sid) {
            Some(i) => i.clone(),
            None => {
                let Some(s) = world.component::<NavMeshSurface>(sid, scid) else {
                    continue;
                };
                match collect(world, sid, s) {
                    Ok(i) => {
                        let i = Arc::new(i);
                        rt.sources.insert(sid, i.clone());
                        i
                    }
                    Err(e) => {
                        oxide_core::debug_log_warning!("NavMeshObstacle carving: {e}");
                        continue;
                    }
                }
            }
        };
        let inv = data.rotation.inverse();
        let local = |v: &AreaVolume| AreaVolume {
            center: inv * (v.center - data.position),
            rotation: inv * v.rotation,
            ..*v
        };
        // Base volumes (modifier volumes) plus every carved hole, in the data's space.
        let mut vols = input.volumes.clone();
        vols.extend(holes.iter().map(&local));
        let margin = data.settings.agent_radius + data.settings.voxel_size * 4.0;
        let mut tiles = HashSet::new();
        for (min, max) in &dirty {
            let c = local(&AreaVolume {
                center: (*min + *max) * 0.5,
                half: (*max - *min) * 0.5,
                rotation: Quaternion::IDENTITY,
                cylinder: false,
                area: None,
            });
            let (lmin, lmax) = volume_bounds(&AreaVolume {
                rotation: c.rotation,
                ..c
            });
            for t in data
                .grid
                .tiles_overlapping(lmin - Vector3::splat(margin), lmax + Vector3::splat(margin))
            {
                tiles.insert(t);
            }
        }
        for (x, z) in tiles {
            let tris = tile_triangles(&input.source, &data.settings, &data.grid, x, z);
            let tile = build_tile(
                &input.source,
                &tris,
                &vols,
                &data.settings,
                &data.grid,
                x,
                z,
            );
            nav.replace_tile(inst, (x, z), tile);
        }
    }
    world.resources.insert(nav);
    world.resources.insert(rt);
}
