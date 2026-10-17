//! NavMesh display for the Scene view (Unity's AI Navigation overlay): the NavMesh as a
//! see-through surface coloured by area with its edges, NavMeshLinks, agent paths, obstacles
//! and volumes. And the `navmesh` MCP tool.

use crate::bus::Bus;
use crate::mcp_tools::{obj_arg, schema, tool, vec3_arg};
use crate::tri;
use oxide_assets::{AssetServer, Material, Mesh, SurfaceType};
use oxide_math::{Color, Matrix4x4, Vector3};
use oxide_mcp::{McpServer, ToolDef, ToolResult, json};
use oxide_navigation::{
    NavMesh, NavMeshAgent, NavMeshLink, NavMeshModifierVolume, NavMeshObstacle,
    NavMeshObstacleShape, NavMeshSurface, NavigationSettings, QueryFilter,
};
use oxide_render::{DrawItem, RenderFrame};
use oxide_scene::{GameObjectId, World};
use serde_json::Value as Json;
use std::sync::Arc;

/// What the overlay shows (Unity's AI Navigation overlay toggles).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NavMeshDisplay {
    pub surfaces: bool,
    /// Only when a NavMesh object is selected (Unity's "Selected" option).
    pub only_selected: bool,
    pub agent_paths: bool,
    pub obstacles: bool,
}

impl Default for NavMeshDisplay {
    fn default() -> Self {
        Self {
            surfaces: true,
            only_selected: false,
            agent_paths: true,
            obstacles: true,
        }
    }
}

const MESH: &str = "runtime:NavMeshDisplay";

/// Colour of a navigation area (Unity: Walkable light blue, Jump gold, others by hue).
pub fn area_color(area: u8) -> Color {
    match area {
        0 => Color::rgb(0.0, 0.75, 1.0),
        1 => Color::rgb(0.6, 0.6, 0.6),
        2 => Color::rgb(1.0, 0.8, 0.2),
        a => {
            let h = (a as f32 * 0.618_034).fract();
            let (r, g, b) = hsv(h, 0.7, 1.0);
            Color::rgb(r, g, b)
        }
    }
}

fn hsv(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - f * s), v * (1.0 - (1.0 - f) * s));
    match i as i32 % 6 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    }
}

/// Rebuilt when the NavMesh changes.
#[derive(Default)]
struct OverlayCache {
    version: Option<u64>,
    /// (submesh index, area) per submesh of the display mesh.
    submeshes: Vec<u8>,
    edges: Vec<(Vector3, Vector3, bool)>,
}

const LIFT: f32 = 0.04;

fn rebuild(world: &mut World) {
    let Some(nav) = world.resources.get::<NavMesh>() else {
        return;
    };
    let version = nav.version;
    if world
        .resources
        .get::<OverlayCache>()
        .is_some_and(|c| c.version == Some(version))
    {
        return;
    }
    // One submesh per area so each gets its own colour.
    let (verts, tris, tri_areas) = nav.display_mesh(1.5);
    let mut by_area: std::collections::BTreeMap<u8, Vec<u32>> = Default::default();
    for (t, a) in tris.iter().zip(&tri_areas) {
        by_area.entry(*a).or_default().extend_from_slice(t);
    }
    let verts: Vec<Vector3> = verts.into_iter().map(|p| p + Vector3::Y * LIFT).collect();
    let edges = nav.debug_edges(1.5);
    let mut indices = Vec::new();
    let mut ranges = Vec::new();
    let mut areas = Vec::new();
    for (area, idx) in by_area {
        let start = indices.len() as u32;
        indices.extend(idx);
        ranges.push(start..indices.len() as u32);
        areas.push(area);
    }
    let mut mesh = Mesh {
        name: "NavMesh".into(),
        vertices: verts,
        indices,
        submeshes: ranges,
        ..Default::default()
    };
    mesh.finish();
    let assets = world.resource_mut::<AssetServer>();
    assets.meshes.insert(MESH, mesh);
    for &a in &areas {
        let path = format!("runtime:NavMeshArea/{a}");
        if assets.materials.handle_of(&path).is_none() {
            let c = area_color(a);
            let mut m = Material::unlit("NavMesh Area", Color::new(c.r, c.g, c.b, 0.3));
            m.surface_type = SurfaceType::Transparent;
            assets.materials.insert(path, m);
        }
    }
    world.resources.insert(OverlayCache {
        version: Some(version),
        submeshes: areas,
        edges,
    });
}

fn wire_box(rf: &mut RenderFrame, m: &Matrix4x4, c: Vector3, size: Vector3, col: Color) {
    let h = size * 0.5;
    let p = |x: f32, y: f32, z: f32| m.transform_point3(c + h * Vector3::new(x, y, z));
    let pts = [
        p(-1., -1., -1.),
        p(1., -1., -1.),
        p(1., -1., 1.),
        p(-1., -1., 1.),
        p(-1., 1., -1.),
        p(1., 1., -1.),
        p(1., 1., 1.),
        p(-1., 1., 1.),
    ];
    for i in 0..4 {
        rf.line(pts[i], pts[(i + 1) % 4], col);
        rf.line(pts[i + 4], pts[(i + 1) % 4 + 4], col);
        rf.line(pts[i], pts[i + 4], col);
    }
}

fn circle(rf: &mut RenderFrame, c: Vector3, r: f32, col: Color) {
    let n = 24;
    for i in 0..n {
        let a0 = i as f32 / n as f32 * std::f32::consts::TAU;
        let a1 = (i + 1) as f32 / n as f32 * std::f32::consts::TAU;
        rf.line(
            c + Vector3::new(a0.cos() * r, 0.0, a0.sin() * r),
            c + Vector3::new(a1.cos() * r, 0.0, a1.sin() * r),
            col,
        );
    }
}

fn is_nav_object(world: &World, id: GameObjectId) -> bool {
    world.get_component::<NavMeshSurface>(id).is_some()
        || world.get_component::<NavMeshAgent>(id).is_some()
        || world.get_component::<NavMeshObstacle>(id).is_some()
        || world.get_component::<NavMeshLink>(id).is_some()
        || world.get_component::<NavMeshModifierVolume>(id).is_some()
}

/// Adds the NavMesh display to a Scene view frame.
pub fn draw_overlay(
    world: &mut World,
    rf: &mut RenderFrame,
    d: &NavMeshDisplay,
    selected: &[GameObjectId],
) {
    let any_selected = selected.iter().any(|&id| is_nav_object(world, id));
    if d.surfaces && (!d.only_selected || any_selected) {
        rebuild(world);
        if let Some(cache) = world.resources.get::<OverlayCache>() {
            let mesh = oxide_assets::AssetId::from_path(MESH);
            for (i, &area) in cache.submeshes.iter().enumerate() {
                rf.draws.push(DrawItem {
                    id: 0,
                    mesh,
                    submesh: i as u32,
                    material: oxide_assets::AssetId::from_path(&format!(
                        "runtime:NavMeshArea/{area}"
                    )),
                    model: Matrix4x4::IDENTITY,
                    cast_shadows: false,
                    shadows_only: false,
                    receive_shadows: false,
                    layer: 0,
                    joint_offset: None,
                    occluder: false,
                    occludee: false,
                    reflection_probe_static: false,
                    lightmap: None,
                    light_probe: None,
                    sorting: Default::default(),
                    rendering_layers: u32::MAX,
                });
            }
            let lift = Vector3::Y * LIFT;
            for &(a, b, boundary) in cache.edges.iter().take(400_000) {
                let col = if boundary {
                    Color::new(0.0, 0.25, 0.45, 0.9)
                } else {
                    Color::new(0.0, 0.5, 0.8, 0.25)
                };
                rf.line(a + lift, b + lift, col);
            }
        }
        if let Some(nav) = world.resources.get::<NavMesh>() {
            for (_, l) in nav.links() {
                let col = if !l.activated {
                    Color::new(0.5, 0.5, 0.5, 0.8)
                } else if l.is_connected() {
                    Color::new(1.0, 0.85, 0.1, 1.0)
                } else {
                    Color::new(1.0, 0.25, 0.2, 1.0)
                };
                // An arc from start to end, raised by a quarter of its length.
                let hgt = l.start.distance(l.end) * 0.25 + 0.3;
                let n = 16;
                for i in 0..n {
                    let t0 = i as f32 / n as f32;
                    let t1 = (i + 1) as f32 / n as f32;
                    let p =
                        |t: f32| l.start.lerp(l.end, t) + Vector3::Y * (4.0 * t * (1.0 - t) * hgt);
                    rf.line(p(t0), p(t1), col);
                }
                circle(rf, l.start + Vector3::Y * LIFT, 0.15 + l.width * 0.5, col);
                circle(rf, l.end + Vector3::Y * LIFT, 0.15 + l.width * 0.5, col);
            }
        }
    }
    if d.agent_paths {
        for (id, a) in world.query::<NavMeshAgent>() {
            let pos = world.position(id);
            let sel = selected.contains(&id);
            circle(
                rf,
                a.next_position() + Vector3::Y * 0.02,
                a.radius,
                if sel {
                    Color::new(1.0, 0.9, 0.2, 1.0)
                } else {
                    Color::new(0.9, 0.9, 0.9, 0.5)
                },
            );
            if !sel && d.only_selected {
                continue;
            }
            let corners = a.path_corners();
            for w in corners.windows(2) {
                rf.line(
                    w[0] + Vector3::Y * 0.1,
                    w[1] + Vector3::Y * 0.1,
                    Color::new(1.0, 0.9, 0.2, 1.0),
                );
            }
            let v = a.velocity();
            if v.length() > 0.01 {
                rf.line(pos, pos + v, Color::new(0.2, 1.0, 0.3, 1.0));
            }
            let dv = a.desired_velocity();
            if dv.length() > 0.01 {
                rf.line(pos, pos + dv, Color::new(1.0, 0.3, 0.3, 0.8));
            }
        }
    }
    if d.obstacles {
        for (id, o) in world.query::<NavMeshObstacle>() {
            let m = Matrix4x4::from_scale_rotation_translation(
                world.lossy_scale(id),
                world.rotation(id),
                world.position(id),
            );
            let col = if o.carve {
                Color::new(1.0, 0.45, 0.2, 1.0)
            } else {
                Color::new(1.0, 0.75, 0.3, 0.8)
            };
            match o.shape {
                NavMeshObstacleShape::Box => wire_box(rf, &m, o.center, o.size, col),
                NavMeshObstacleShape::Capsule => {
                    let c = m.transform_point3(o.center);
                    let s = world.lossy_scale(id);
                    let r = o.radius * s.x.abs().max(s.z.abs());
                    let h = o.height * s.y.abs() * 0.5;
                    circle(rf, c - Vector3::Y * h, r, col);
                    circle(rf, c + Vector3::Y * h, r, col);
                }
            }
        }
    }
    for &id in selected {
        let m = Matrix4x4::from_scale_rotation_translation(
            world.lossy_scale(id),
            world.rotation(id),
            world.position(id),
        );
        if let Some(v) = world.get_component::<NavMeshModifierVolume>(id) {
            let c = area_color(v.area.clamp(0, 31) as u8);
            wire_box(rf, &m, v.center, v.size, Color::new(c.r, c.g, c.b, 1.0));
        }
        if let Some(s) = world.get_component::<NavMeshSurface>(id)
            && s.collect_objects == oxide_navigation::CollectObjects::Volume
        {
            let m = Matrix4x4::from_rotation_translation(world.rotation(id), world.position(id));
            wire_box(rf, &m, s.center, s.size, Color::new(0.4, 0.9, 1.0, 1.0));
        }
        if let Some(l) = world.get_component::<NavMeshLink>(id) {
            let (a, b) = (
                world.transform_point(id, l.start_point),
                world.transform_point(id, l.end_point),
            );
            rf.line(a, b, Color::new(1.0, 0.85, 0.1, 0.6));
        }
    }
}

// ---- MCP ----

fn filter_args(args: &crate::mcp_tools::Args) -> QueryFilter {
    let mut f = QueryFilter::new(args.get("agent_type").and_then(Json::as_i64).unwrap_or(0) as i32);
    if let Some(m) = args.get("area_mask").and_then(Json::as_u64) {
        f.area_mask = m as u32;
    }
    f
}

fn v3(v: Vector3) -> Json {
    json!([
        (v.x * 1000.0).round() / 1000.0,
        (v.y * 1000.0).round() / 1000.0,
        (v.z * 1000.0).round() / 1000.0
    ])
}

fn agent_json(world: &World, id: GameObjectId, a: &NavMeshAgent) -> Json {
    json!({
        "name": world.get(id).map(|o| o.name.clone()),
        "id": id.to_bits(),
        "position": v3(world.position(id)),
        "on_navmesh": a.is_on_nav_mesh(),
        "has_path": a.has_path(),
        "path_pending": a.path_pending(),
        "path_status": format!("{:?}", a.path_status()),
        "destination": v3(a.destination()),
        "remaining_distance": (a.remaining_distance() * 1000.0).round() / 1000.0,
        "velocity": v3(a.velocity()),
        "speed": (a.velocity().length() * 1000.0).round() / 1000.0,
        "is_stopped": a.is_stopped(),
        "on_off_mesh_link": a.is_on_off_mesh_link(),
        "corners": a.path_corners().into_iter().map(v3).collect::<Vec<_>>(),
    })
}

pub fn register(s: &mut McpServer, bus: &Arc<Bus>) {
    tool(
        s,
        bus,
        ToolDef::new(
            "navmesh",
            "NavMesh",
            "Unity AI Navigation. `action`: `status` (NavMesh size, surfaces, agents, links, bake progress); \
             `bake` (all active NavMeshSurfaces, or `objects` [names/ids]; runs in the background and saves \
             `<scene folder>/NavMesh-<name>.navmesh`; poll `status`); `build` (bake `object` now, blocking, not \
             saved: Unity BuildNavMesh); `cancel`; `clear` (`object`: forget and delete its NavMesh); \
             `calculate_path` (`from`, `to` → corners, status, length); `sample_position` (`position`, \
             `max_distance`); `raycast` (`from`, `to`; hit=true if blocked); `closest_edge` (`position`); \
             `agents` (every NavMeshAgent's state); `agent` (`object`, plus optional `set_destination` [x,y,z], \
             `stop`/`resume`, `warp` [x,y,z], `reset_path`). Queries take `agent_type` (default 0 Humanoid) and \
             `area_mask`. Agent types and area costs are in project_settings section `navigation`.",
            schema(
                json!({
                    "action": { "type": "string", "enum": ["status", "bake", "build", "cancel", "clear", "calculate_path",
                        "sample_position", "raycast", "closest_edge", "agents", "agent"] },
                    "objects": { "type": "array", "items": {} },
                    "object": {},
                    "from": { "type": "array", "items": { "type": "number" } },
                    "to": { "type": "array", "items": { "type": "number" } },
                    "position": { "type": "array", "items": { "type": "number" } },
                    "max_distance": { "type": "number" },
                    "agent_type": { "type": "integer" },
                    "area_mask": { "type": "integer" },
                    "set_destination": { "type": "array", "items": { "type": "number" } },
                    "warp": { "type": "array", "items": { "type": "number" } },
                    "stop": { "type": "boolean" },
                    "resume": { "type": "boolean" },
                    "reset_path": { "type": "boolean" }
                }),
                &["action"],
            ),
        ),
        |args, e| {
            let action = args
                .get("action")
                .and_then(Json::as_str)
                .unwrap_or("status");
            let w = &mut e.world;
            match action {
                "status" => {
                    let nav = w.resources.get::<NavMesh>();
                    let surfaces: Vec<Json> = w
                        .query::<NavMeshSurface>()
                        .map(|(id, s)| {
                            json!({
                                "name": w.get(id).map(|o| o.name.clone()),
                                "agent_type": s.agent_type_id,
                                "navmesh_data": s.navmesh_data.as_ref().map(|h| h.path().to_owned()),
                                "built_at_runtime": s.runtime.data.is_some(),
                            })
                        })
                        .collect();
                    ToolResult::json(json!({
                        "polygons": nav.map_or(0, |n| n.polygon_count()),
                        "tiles": nav.map_or(0, |n| n.tile_count()),
                        "instances": nav.map_or(0, |n| n.instance_count()),
                        "links": nav.map_or(0, |n| n.links().count()),
                        "agents": w.query::<NavMeshAgent>().count(),
                        "obstacles": w.query::<NavMeshObstacle>().count(),
                        "surfaces": surfaces,
                        "bake": oxide_navigation::bake::status(w),
                    }))
                }
                "bake" => {
                    let only = match args.get("objects").and_then(Json::as_array) {
                        Some(list) => {
                            let mut ids = Vec::new();
                            for v in list {
                                match crate::json_values::resolve_object(w, v) {
                                    Some(id) => ids.push(id),
                                    None => {
                                        return ToolResult::error(format!(
                                            "no GameObject matches {v}"
                                        ));
                                    }
                                }
                            }
                            Some(ids)
                        }
                        None => None,
                    };
                    match oxide_navigation::bake::start(w, only.as_deref(), true) {
                        Ok(n) => ToolResult::json(json!({ "started": n })),
                        Err(err) => ToolResult::error(err),
                    }
                }
                "build" => {
                    let id = tri!(obj_arg(args, w, "object"));
                    let t = std::time::Instant::now();
                    match oxide_navigation::bake::build_now(w, id) {
                        Ok(n) => ToolResult::json(
                            json!({ "polygons": n, "seconds": t.elapsed().as_secs_f32() }),
                        ),
                        Err(err) => ToolResult::error(err),
                    }
                }
                "cancel" => {
                    oxide_navigation::bake::cancel(w);
                    ToolResult::text("cancelling")
                }
                "clear" => {
                    let id = tri!(obj_arg(args, w, "object"));
                    match oxide_navigation::bake::clear(w, id) {
                        Ok(()) => ToolResult::text("cleared"),
                        Err(err) => ToolResult::error(err),
                    }
                }
                "calculate_path" => {
                    let (Some(a), Some(b)) =
                        (tri!(vec3_arg(args, "from")), tri!(vec3_arg(args, "to")))
                    else {
                        return ToolResult::error("`from` and `to` are required");
                    };
                    let p = w
                        .resource::<NavMesh>()
                        .calculate_path(a, b, &filter_args(args));
                    ToolResult::json(json!({
                        "status": format!("{:?}", p.status),
                        "length": (p.length() * 1000.0).round() / 1000.0,
                        "corners": p.corners.iter().map(|c| v3(*c)).collect::<Vec<_>>(),
                    }))
                }
                "sample_position" => {
                    let Some(p) = tri!(vec3_arg(args, "position")) else {
                        return ToolResult::error("`position` is required");
                    };
                    let d = args
                        .get("max_distance")
                        .and_then(Json::as_f64)
                        .unwrap_or(1.0) as f32;
                    match w
                        .resource::<NavMesh>()
                        .sample_position(p, d, &filter_args(args))
                    {
                        Some(h) => ToolResult::json(
                            json!({ "hit": true, "position": v3(h.position), "distance": h.distance, "mask": h.mask }),
                        ),
                        None => ToolResult::json(json!({ "hit": false })),
                    }
                }
                "raycast" => {
                    let (Some(a), Some(b)) =
                        (tri!(vec3_arg(args, "from")), tri!(vec3_arg(args, "to")))
                    else {
                        return ToolResult::error("`from` and `to` are required");
                    };
                    let h = w.resource::<NavMesh>().raycast(a, b, &filter_args(args));
                    ToolResult::json(
                        json!({ "hit": h.hit, "position": v3(h.position), "normal": v3(h.normal), "distance": h.distance }),
                    )
                }
                "closest_edge" => {
                    let Some(p) = tri!(vec3_arg(args, "position")) else {
                        return ToolResult::error("`position` is required");
                    };
                    match w
                        .resource::<NavMesh>()
                        .find_closest_edge(p, &filter_args(args))
                    {
                        Some(h) => ToolResult::json(
                            json!({ "position": v3(h.position), "normal": v3(h.normal), "distance": h.distance }),
                        ),
                        None => ToolResult::error("no NavMesh near that position"),
                    }
                }
                "agents" => {
                    let list: Vec<Json> = w
                        .query::<NavMeshAgent>()
                        .map(|(id, a)| agent_json(w, id, a))
                        .collect();
                    ToolResult::json(json!(list))
                }
                "agent" => {
                    let id = tri!(obj_arg(args, w, "object"));
                    let dest = tri!(vec3_arg(args, "set_destination"));
                    let warp = tri!(vec3_arg(args, "warp"));
                    let Some(a) = w.get_component_mut::<NavMeshAgent>(id) else {
                        return ToolResult::error("that object has no NavMeshAgent");
                    };
                    if args.get("reset_path").and_then(Json::as_bool) == Some(true) {
                        a.reset_path();
                    }
                    if let Some(p) = warp {
                        a.warp(p);
                    }
                    if let Some(p) = dest {
                        a.set_destination(p);
                    }
                    if args.get("stop").and_then(Json::as_bool) == Some(true) {
                        a.set_is_stopped(true);
                    }
                    if args.get("resume").and_then(Json::as_bool) == Some(true) {
                        a.set_is_stopped(false);
                    }
                    let a = w
                        .get_component::<NavMeshAgent>(id)
                        .cloned()
                        .unwrap_or_default();
                    ToolResult::json(agent_json(w, id, &a))
                }
                other => ToolResult::error(format!("unknown action `{other}`")),
            }
        },
    );
}

/// Area names for mask/area pickers (empty names skipped).
pub fn area_names(world: &World) -> Vec<(usize, String)> {
    world
        .resources
        .get::<NavigationSettings>()
        .map(|s| {
            s.areas
                .iter()
                .enumerate()
                .filter(|(_, a)| !a.name.is_empty())
                .map(|(i, a)| (i, a.name.clone()))
                .collect()
        })
        .unwrap_or_default()
}
