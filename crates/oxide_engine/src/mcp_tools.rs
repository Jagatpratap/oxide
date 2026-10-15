//! The engine's MCP tools. Each tool runs on the engine thread through the [`Bus`] and works
//! on the live World — the same operations as the Unity Editor's Hierarchy, Inspector,
//! Project window, Lighting window, Console and Play controls.

use crate::bus::Bus;
use crate::engine::Engine;
use crate::extract;
use crate::json_values::*;
use oxide_assets::{AssetServer, Material, PrimitiveType};
use oxide_core::Time;
use oxide_core::debug::{self, LogType};
use oxide_input::{Input, KeyCode, MouseButton};
use oxide_math::{Quaternion, QuaternionExt, Vector3};
use oxide_mcp::{McpServer, ToolDef, ToolResult, json};
use oxide_render::{CameraView, ClearMode, ProjectionKind};
use oxide_scene::{GameObjectId, RenderSettings, SceneFile, World};
use serde_json::{Map, Value as Json};
use std::sync::Arc;

const INSTRUCTIONS: &str = "\
Oxide is a Rust game engine modeled on Unity: GameObjects with a Transform hierarchy and \
Components (Camera, Light, MeshFilter, MeshRenderer, Rigidbody, colliders, scripts...). Space is \
left-handed, +Y up, +Z forward, angles in degrees, colors sRGB 0-1 or \"#RRGGBB\". \
Typical flow: get_hierarchy → create_object / add_component / set_component → capture_screenshot \
to look at the result (pass camera_position/look_at for any viewpoint) → get_console_logs \
(min_type=Warning) to check for problems. Objects are referenced by id, name or \"Parent/Child\" path.";

type Args = Map<String, Json>;

pub(crate) fn tool<F>(s: &mut McpServer, bus: &Arc<Bus>, def: ToolDef, f: F)
where
    F: Fn(&Args, &mut Engine) -> ToolResult + Send + Sync + 'static,
{
    let f = Arc::new(f);
    let b = bus.clone();
    s.add_tool(def, move |args| {
        let f = f.clone();
        let args = args.clone();
        b.run(move |e| f(&args, e))
    });
}

pub(crate) fn schema(props: Json, required: &[&str]) -> Json {
    json!({ "type": "object", "properties": props, "required": required, "additionalProperties": false })
}

fn obj_arg(args: &Args, world: &World, key: &str) -> Result<GameObjectId, ToolResult> {
    let v = args
        .get(key)
        .ok_or_else(|| ToolResult::error(format!("`{key}` is required")))?;
    resolve_object(world, v).ok_or_else(|| ToolResult::error(format!("no GameObject matches {v}")))
}

pub(crate) fn vec3_arg(args: &Args, key: &str) -> Result<Option<Vector3>, ToolResult> {
    match args.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(v) => {
            let a = v
                .as_array()
                .filter(|a| a.len() == 3)
                .ok_or_else(|| ToolResult::error(format!("`{key}` must be [x, y, z]")))?;
            let f: Vec<f32> = a
                .iter()
                .filter_map(Json::as_f64)
                .map(|x| x as f32)
                .collect();
            if f.len() != 3 {
                return Err(ToolResult::error(format!("`{key}` must be three numbers")));
            }
            Ok(Some(Vector3::new(f[0], f[1], f[2])))
        }
    }
}

#[macro_export]
#[doc(hidden)]
macro_rules! tri {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(r) => return r,
        }
    };
}

/// Inspector view of one object.
pub fn object_json(w: &World, id: GameObjectId) -> Json {
    let Some(o) = w.get(id) else {
        return Json::Null;
    };
    let t = &o.transform;
    let comps: Vec<Json> = o
        .component_list()
        .into_iter()
        .map(|(cid, name, enabled)| {
            json!({
                "type": name,
                "enabled": enabled,
                "fields": w.component_dyn(id, cid).map(|c| reflect_to_json(Some(w), c)),
            })
        })
        .collect();
    let r = |v: Vector3| [v.x, v.y, v.z].map(|x| (x * 1e4).round() / 1e4);
    json!({
        "id": id.to_bits(),
        "name": o.name,
        "path": w.path_of(id),
        "active_self": o.active_self(),
        "active_in_hierarchy": o.active_in_hierarchy(),
        "tag": o.tag,
        "layer": o.layer,
        "static": o.is_static,
        "transform": {
            "local_position": r(t.local_position),
            "local_euler_angles": r(t.local_rotation.euler_angles()),
            "local_scale": r(t.local_scale),
            "position": r(w.position(id)),
            "euler_angles": r(w.euler_angles(id)),
        },
        "parent": o.parent().map(|p| p.to_bits()),
        "children": o.children().iter().map(|c| json!({"id": c.to_bits(), "name": w.get(*c).map(|c| c.name.clone())})).collect::<Vec<_>>(),
        "components": comps,
    })
}

fn hierarchy_json(w: &World, id: GameObjectId, depth: usize, max_depth: usize) -> Json {
    let Some(o) = w.get(id) else {
        return Json::Null;
    };
    let comps: Vec<&str> = o.component_list().into_iter().map(|c| c.1).collect();
    let mut j = json!({ "id": id.to_bits(), "name": o.name, "components": comps });
    if !o.active_self() {
        j["active"] = json!(false);
    }
    if !o.children().is_empty() {
        if depth < max_depth {
            j["children"] = json!(
                o.children()
                    .iter()
                    .map(|&c| hierarchy_json(w, c, depth + 1, max_depth))
                    .collect::<Vec<_>>()
            );
        } else {
            j["child_count"] = json!(o.children().len());
        }
    }
    j
}

fn time_json(e: &Engine) -> Json {
    let t = e.world.resource::<Time>();
    json!({
        "time": t.time(), "delta_time": t.delta_time(), "unscaled_time": t.unscaled_time(),
        "fixed_time": t.fixed_time(), "fixed_delta_time": t.fixed_delta_time(),
        "maximum_delta_time": t.maximum_delta_time(), "time_scale": t.time_scale(),
        "frame_count": t.frame_count(), "fixed_update_calls": t.fixed_step_count(), "paused": e.paused,
    })
}

/// Applies `fields` to the first component of `type_name` on `id` (adding it if `add`).
fn set_component(
    w: &mut World,
    id: GameObjectId,
    type_name: &str,
    fields: &Map<String, Json>,
    index: usize,
) -> Result<Vec<String>, String> {
    let cids = w.components_named(id, type_name);
    let cid = *cids.get(index).ok_or_else(|| {
        let have: Vec<_> = w
            .get(id)
            .map(|o| o.component_list().into_iter().map(|c| c.1).collect())
            .unwrap_or_default();
        format!("object has no {type_name} (index {index}); components: {have:?}")
    })?;
    let info = w
        .component_dyn(id, cid)
        .ok_or("component busy")?
        .type_info();
    let (values, mut errors) = parse_fields(
        &info,
        fields,
        w,
        w.component_dyn(id, cid)
            .map(|c| c as &dyn oxide_core::reflect::Reflect),
    );
    let comp = w.component_dyn_mut(id, cid).ok_or("component busy")?;
    set_fields(comp, values, &mut errors);
    Ok(errors)
}

/// Builds the server with every engine tool registered.
pub fn build_server(bus: Arc<Bus>) -> McpServer {
    let mut s = McpServer::new("oxide", oxide_core::ENGINE_VERSION, INSTRUCTIONS);
    let empty = schema(json!({}), &[]);
    let bus = &bus;

    tool(&mut s, bus, ToolDef::new("get_engine_info", "Get engine info",
        "Engine version, GPU, window size, frame rate, render stats (draw calls, triangles), object count, scene name.", empty.clone()).read_only(),
        |_, e| {
            let gpu = e.gpu.adapter_info();
            let (w, h) = e.surface.size();
            let (logs, warnings, errors) = debug::counts();
            ToolResult::json(json!({
                "engine": "Oxide", "version": oxide_core::ENGINE_VERSION,
                "gpu": { "name": gpu.name, "backend": format!("{:?}", gpu.backend) },
                "window": { "width": w, "height": h },
                "fps": e.fps, "paused": e.paused,
                "render": e.stats, "msaa": e.renderer.sample_count(),
                "scene": e.world.scene_name, "objects": e.world.len(),
                "systems": e.systems.iter().map(|s| s.name()).collect::<Vec<_>>(),
                "console": { "logs": logs, "warnings": warnings, "errors": errors },
                "mcp_url": e.mcp_url,
            }))
        });

    s.add_tool(ToolDef::new("get_console_logs", "Read the Console",
        "Console messages oldest first. Pass the returned `last_seq` as `after_seq` to get only new ones.",
        schema(json!({
            "after_seq": { "type": "integer", "minimum": 0 },
            "min_type": { "type": "string", "enum": ["Log", "Warning", "Error", "Exception"] },
            "limit": { "type": "integer", "minimum": 1, "maximum": 1000 }
        }), &[])).read_only(),
        |args| {
            let after = args.get("after_seq").and_then(Json::as_u64).unwrap_or(0);
            let min_type = match args.get("min_type").and_then(Json::as_str) {
                Some(s) => match LogType::parse(s) { Some(t) => t, None => return ToolResult::error(format!("unknown min_type `{s}`")) },
                None => LogType::Log,
            };
            let limit = args.get("limit").and_then(Json::as_u64).unwrap_or(100).clamp(1, 1000) as usize;
            let entries = debug::entries_since(after, min_type, limit);
            let last_seq = entries.last().map_or(after, |e| e.seq);
            ToolResult::json(json!({ "entries": entries, "last_seq": last_seq }))
        });

    s.add_tool(
        ToolDef::new(
            "clear_console",
            "Clear the Console",
            "Removes all Console messages.",
            empty.clone(),
        ),
        |_| {
            debug::clear();
            ToolResult::text("Console cleared.")
        },
    );

    tool(
        &mut s,
        bus,
        ToolDef::new(
            "get_time",
            "Get Time",
            "Unity Time values and FixedUpdate count.",
            empty.clone(),
        )
        .read_only(),
        |_, e| ToolResult::json(time_json(e)),
    );

    tool(
        &mut s,
        bus,
        ToolDef::new(
            "set_time",
            "Set Time settings",
            "Changes Time.timeScale (0 freezes), fixedDeltaTime, maximumDeltaTime.",
            schema(
                json!({
                    "time_scale": { "type": "number", "minimum": 0, "maximum": 100 },
                    "fixed_delta_time": { "type": "number", "minimum": 0.0001, "maximum": 1 },
                    "maximum_delta_time": { "type": "number", "minimum": 0.0001, "maximum": 10 }
                }),
                &[],
            ),
        ),
        |args, e| {
            let get = |k: &str| args.get(k).and_then(Json::as_f64).map(|v| v as f32);
            let t = e.world.resource_mut::<Time>();
            if let Some(v) = get("time_scale") {
                t.set_time_scale(v)
            }
            if let Some(v) = get("fixed_delta_time") {
                t.set_fixed_delta_time(v)
            }
            if let Some(v) = get("maximum_delta_time") {
                t.set_maximum_delta_time(v)
            }
            ToolResult::json(time_json(e))
        },
    );

    tool(
        &mut s,
        bus,
        ToolDef::new(
            "set_paused",
            "Pause or resume",
            "Pauses/resumes the game loop (editor Pause). Rendering continues.",
            schema(json!({ "paused": { "type": "boolean" } }), &["paused"]),
        ),
        |args, e| {
            let Some(p) = args.get("paused").and_then(Json::as_bool) else {
                return ToolResult::error("`paused` is required");
            };
            e.paused = p;
            e.pending_steps = 0;
            ToolResult::text(if p { "Paused." } else { "Resumed." })
        },
    );

    tool(
        &mut s,
        bus,
        ToolDef::new(
            "step_frames",
            "Step frames",
            "Advances the simulation by `count` frames of 1/60 s immediately (works paused or not), then returns the new Time.",
            schema(
                json!({ "count": { "type": "integer", "minimum": 1, "maximum": 3600 } }),
                &[],
            ),
        ),
        |args, e| {
            let n = args
                .get("count")
                .and_then(Json::as_u64)
                .unwrap_or(1)
                .clamp(1, 3600);
            for _ in 0..n {
                e.tick(1.0 / 60.0);
                e.world.resource_mut::<Input>().end_frame();
            }
            ToolResult::json(time_json(e))
        },
    );

    tool(&mut s, bus, ToolDef::new("capture_screenshot", "Capture screenshot",
        "Renders a PNG. By default through the scene cameras; pass `camera_position` and `look_at` (world coords) \
         to view from anywhere (like the Scene view), or `camera` to use one object's Camera. Long side capped by `max_size` (default 1024).",
        schema(json!({
            "width": { "type": "integer", "minimum": 16, "maximum": 4096 },
            "height": { "type": "integer", "minimum": 16, "maximum": 4096 },
            "max_size": { "type": "integer", "minimum": 64, "maximum": 4096 },
            "camera_position": { "type": "array", "items": { "type": "number" } },
            "look_at": { "type": "array", "items": { "type": "number" } },
            "field_of_view": { "type": "number" },
            "camera": { "description": "Object with a Camera to render from." }
        }), &[])).read_only(),
        |args, e| {
            let get = |k: &str| args.get(k).and_then(Json::as_u64).map(|v| v.clamp(16, 4096) as u32);
            let (ww, wh) = e.surface.size();
            let (mut w, mut h) = (get("width").unwrap_or(ww), get("height").unwrap_or(wh));
            let max = get("max_size").unwrap_or(1024);
            let long = w.max(h);
            if long > max {
                w = (w as u64 * max as u64 / long as u64).max(16) as u32;
                h = (h as u64 * max as u64 / long as u64).max(16) as u32;
            }
            let pos = tri!(vec3_arg(args, "camera_position"));
            let target = tri!(vec3_arg(args, "look_at"));
            let cams = if let Some(p) = pos {
                let t = target.unwrap_or(p + Vector3::Z);
                let fov = args.get("field_of_view").and_then(Json::as_f64).unwrap_or(60.0) as f32;
                Some(vec![CameraView {
                    position: p,
                    rotation: Quaternion::look_rotation((t - p).normalize_or(Vector3::Z), Vector3::Y),
                    projection: ProjectionKind::Perspective { fov },
                    near: 0.1, far: 5000.0, viewport: [0.0, 0.0, 1.0, 1.0], clear: ClearMode::Sky, culling_mask: u32::MAX,
                }])
            } else if args.contains_key("camera") {
                let id = tri!(obj_arg(args, &e.world, "camera"));
                let Some(c) = e.world.get_component::<oxide_scene::Camera>(id) else { return ToolResult::error("that object has no Camera") };
                let mut v = extract::camera_view(&e.world, id, c);
                v.viewport = [0.0, 0.0, 1.0, 1.0];
                Some(vec![v])
            } else {
                None
            };
            match e.capture(w, h, cams).and_then(|img| img.to_png()) {
                Ok(png) => ToolResult::png(png).with_text(format!("{w}x{h}, frame {}", e.time().frame_count())),
                Err(err) => ToolResult::error(err),
            }
        });

    tool(&mut s, bus, ToolDef::new("get_hierarchy", "Get Hierarchy",
        "The scene's GameObject tree (ids, names, component types). `root` limits it to one subtree; `max_depth` defaults to 6.",
        schema(json!({ "root": {}, "max_depth": { "type": "integer", "minimum": 0, "maximum": 64 } }), &[])).read_only(),
        |args, e| {
            let w = &e.world;
            let depth = args.get("max_depth").and_then(Json::as_u64).unwrap_or(6) as usize;
            let roots: Vec<GameObjectId> = match args.get("root") {
                Some(_) => vec![tri!(obj_arg(args, w, "root"))],
                None => w.roots().to_vec(),
            };
            ToolResult::json(json!({
                "scene": w.scene_name, "object_count": w.len(),
                "roots": roots.iter().map(|&r| hierarchy_json(w, r, 0, depth)).collect::<Vec<_>>(),
            }))
        });

    tool(&mut s, bus, ToolDef::new("inspect_object", "Inspect object",
        "Everything the Inspector shows for one GameObject: transform (local + world), tag, layer, children, every component with all field values.",
        schema(json!({ "object": { "description": "id, name or path" } }), &["object"])).read_only(),
        |args, e| {
            let id = tri!(obj_arg(args, &e.world, "object"));
            ToolResult::json(object_json(&e.world, id))
        });

    tool(
        &mut s,
        bus,
        ToolDef::new(
            "create_object",
            "Create GameObject",
            "Creates an empty object or a primitive (Cube, Sphere, Capsule, Cylinder, Plane, Quad — with a matching collider), \
         or a Camera / Directional Light / Point Light / Spot Light. Optional transform, parent and `color` (sets a new material).",
            schema(
                json!({
                    "name": { "type": "string" },
                    "kind": { "type": "string", "enum": ["Empty", "Cube", "Sphere", "Capsule", "Cylinder", "Plane", "Quad", "Camera", "DirectionalLight", "PointLight", "SpotLight"] },
                    "parent": {}, "position": { "type": "array" }, "euler_angles": { "type": "array" }, "scale": { "type": "array" },
                    "color": {}
                }),
                &[],
            ),
        ),
        |args, e| {
            let w = &mut e.world;
            let kind = args.get("kind").and_then(Json::as_str).unwrap_or("Empty");
            let id = if let Some(p) = PrimitiveType::parse(kind) {
                crate::objects::create_primitive(w, p)
            } else {
                match kind.to_ascii_lowercase().as_str() {
                    "empty" => w.create("GameObject"),
                    "camera" => crate::objects::create_camera(w, "Camera"),
                    "directionallight" => crate::objects::create_directional_light(w),
                    "pointlight" | "spotlight" => {
                        let id = w.create(if kind.eq_ignore_ascii_case("spotlight") {
                            "Spot Light"
                        } else {
                            "Point Light"
                        });
                        let mut l = oxide_scene::Light::default();
                        if kind.eq_ignore_ascii_case("spotlight") {
                            l.light_type = oxide_scene::LightType::Spot;
                        }
                        w.add_component(id, l);
                        id
                    }
                    _ => return ToolResult::error(format!("unknown kind `{kind}`")),
                }
            };
            if let Some(n) = args.get("name").and_then(Json::as_str) {
                w.get_mut(id).expect("created").name = n.to_owned();
            }
            if args.contains_key("parent") {
                let p = tri!(obj_arg(args, w, "parent"));
                let _ = w.set_parent(id, Some(p), false);
            }
            let t = &mut w.get_mut(id).expect("created").transform;
            if let Some(v) = tri!(vec3_arg(args, "position")) {
                t.local_position = v
            }
            if let Some(v) = tri!(vec3_arg(args, "euler_angles")) {
                t.local_rotation = Quaternion::euler_v(v)
            }
            if let Some(v) = tri!(vec3_arg(args, "scale")) {
                t.local_scale = v
            }
            if let Some(c) = args.get("color") {
                let rgba = match c {
                    Json::String(s) => tri!(parse_hex_color(s).map_err(ToolResult::error)),
                    Json::Array(a) => {
                        let f: Vec<f32> = a
                            .iter()
                            .filter_map(Json::as_f64)
                            .map(|x| x as f32)
                            .collect();
                        [
                            f.first().copied().unwrap_or(1.0),
                            f.get(1).copied().unwrap_or(1.0),
                            f.get(2).copied().unwrap_or(1.0),
                            f.get(3).copied().unwrap_or(1.0),
                        ]
                    }
                    _ => return ToolResult::error("`color` must be \"#RRGGBB\" or [r, g, b]"),
                };
                let name = w.get(id).map(|o| o.name.clone()).unwrap_or_default();
                let m = crate::objects::color_material(
                    w,
                    &name,
                    oxide_math::Color::new(rgba[0], rgba[1], rgba[2], rgba[3]),
                );
                crate::objects::set_material(w, id, m);
            }
            ToolResult::json(object_json(w, id))
        },
    );

    tool(
        &mut s,
        bus,
        ToolDef::new(
            "destroy_object",
            "Destroy GameObject",
            "Destroys an object and its children.",
            schema(json!({ "object": {} }), &["object"]),
        )
        .destructive(),
        |args, e| {
            let id = tri!(obj_arg(args, &e.world, "object"));
            let name = e.world.path_of(id);
            e.world.destroy_immediate(id);
            ToolResult::text(format!("Destroyed {name}."))
        },
    );

    tool(
        &mut s,
        bus,
        ToolDef::new(
            "set_object",
            "Edit GameObject",
            "Edits an object's header and Transform: name, active, tag, layer, static, parent (null = root), \
         position/euler_angles/scale (local by default; `space: \"world\"` for world position/rotation).",
            schema(
                json!({
                    "object": {}, "name": { "type": "string" }, "active": { "type": "boolean" }, "tag": { "type": "string" },
                    "layer": { "type": "integer", "minimum": 0, "maximum": 31 }, "static": { "type": "boolean" }, "parent": {},
                    "position": { "type": "array" }, "euler_angles": { "type": "array" }, "scale": { "type": "array" },
                    "space": { "type": "string", "enum": ["local", "world"] }
                }),
                &["object"],
            ),
        ),
        |args, e| {
            let w = &mut e.world;
            let id = tri!(obj_arg(args, w, "object"));
            if let Some(p) = args.get("parent") {
                let parent = if p.is_null() {
                    None
                } else {
                    Some(tri!(obj_arg(args, w, "parent")))
                };
                if let Err(err) = w.set_parent(id, parent, true) {
                    return ToolResult::error(err);
                }
            }
            if let Some(a) = args.get("active").and_then(Json::as_bool) {
                w.set_active(id, a)
            }
            let world_space = args.get("space").and_then(Json::as_str) == Some("world");
            let pos = tri!(vec3_arg(args, "position"));
            let rot = tri!(vec3_arg(args, "euler_angles"));
            let scale = tri!(vec3_arg(args, "scale"));
            if world_space {
                if let Some(r) = rot {
                    w.set_euler_angles(id, r)
                }
                if let Some(p) = pos {
                    w.set_position(id, p)
                }
            }
            let o = w.get_mut(id).expect("alive");
            if let Some(n) = args.get("name").and_then(Json::as_str) {
                o.name = n.to_owned()
            }
            if let Some(t) = args.get("tag").and_then(Json::as_str) {
                o.tag = t.to_owned()
            }
            if let Some(l) = args.get("layer").and_then(Json::as_u64) {
                o.layer = l as u8
            }
            if let Some(st) = args.get("static").and_then(Json::as_bool) {
                o.is_static = st
            }
            if !world_space {
                if let Some(p) = pos {
                    o.transform.local_position = p
                }
                if let Some(r) = rot {
                    o.transform.local_rotation = Quaternion::euler_v(r)
                }
            }
            if let Some(sc) = scale {
                o.transform.local_scale = sc
            }
            ToolResult::json(object_json(w, id))
        },
    );

    tool(
        &mut s,
        bus,
        ToolDef::new(
            "add_component",
            "Add Component",
            "Adds a component by type name (see list_component_types) with optional initial `fields`.",
            schema(
                json!({ "object": {}, "type": { "type": "string" }, "fields": { "type": "object" } }),
                &["object", "type"],
            ),
        ),
        |args, e| {
            let w = &mut e.world;
            let id = tri!(obj_arg(args, w, "object"));
            let ty = args.get("type").and_then(Json::as_str).unwrap_or_default();
            let Some(mut comp) = w.registry.create(ty) else {
                let names: Vec<_> = w.registry.iter().map(|r| r.info.name).collect();
                return ToolResult::error(format!(
                    "unknown component `{ty}`; available: {names:?}"
                ));
            };
            let mut errors = Vec::new();
            if let Some(f) = args.get("fields").and_then(Json::as_object) {
                let info = comp.type_info();
                let (values, errs) = parse_fields(
                    &info,
                    f,
                    w,
                    Some(comp.as_ref() as &dyn oxide_core::reflect::Reflect),
                );
                errors = errs;
                set_fields(comp.as_mut(), values, &mut errors);
            }
            let name = comp.type_info().name;
            if !comp.allow_multiple() && !w.components_named(id, name).is_empty() {
                return ToolResult::error(format!(
                    "{name} already exists on this object (only one allowed)"
                ));
            }
            w.add_component_boxed(id, comp);
            ToolResult::json(json!({ "object": object_json(w, id), "errors": errors }))
        },
    );

    tool(&mut s, bus, ToolDef::new("remove_component", "Remove Component", "Removes a component (by type name, `index` for duplicates).",
        schema(json!({ "object": {}, "type": { "type": "string" }, "index": { "type": "integer", "minimum": 0 } }), &["object", "type"])).destructive(),
        |args, e| {
            let w = &mut e.world;
            let id = tri!(obj_arg(args, w, "object"));
            let ty = args.get("type").and_then(Json::as_str).unwrap_or_default();
            let i = args.get("index").and_then(Json::as_u64).unwrap_or(0) as usize;
            let Some(&cid) = w.components_named(id, ty).get(i) else { return ToolResult::error(format!("no {ty} on that object")) };
            w.remove_component_now(id, cid);
            ToolResult::json(object_json(w, id))
        });

    tool(
        &mut s,
        bus,
        ToolDef::new(
            "set_component",
            "Set component fields",
            "Edits component fields like the Inspector: `fields` is {name: value}. Vectors are [x,y,z], colors [r,g,b,a] or \"#RRGGBB\", \
         enums by variant name, assets by path (\"builtin:Sphere\"), object references by id/name/path. \
         `enabled` toggles the component.",
            schema(
                json!({ "object": {}, "type": { "type": "string" }, "index": { "type": "integer", "minimum": 0 }, "fields": { "type": "object" }, "enabled": { "type": "boolean" } }),
                &["object", "type"],
            ),
        ),
        |args, e| {
            let w = &mut e.world;
            let id = tri!(obj_arg(args, w, "object"));
            let ty = args
                .get("type")
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_owned();
            let index = args.get("index").and_then(Json::as_u64).unwrap_or(0) as usize;
            let empty = Map::new();
            let fields = args
                .get("fields")
                .and_then(Json::as_object)
                .unwrap_or(&empty);
            let errors = match set_component(w, id, &ty, fields, index) {
                Ok(errs) => errs,
                Err(err) => return ToolResult::error(err),
            };
            if let Some(en) = args.get("enabled").and_then(Json::as_bool)
                && let Some(&cid) = w.components_named(id, &ty).get(index)
            {
                w.set_component_enabled(id, cid, en);
            }
            let mut out = object_json(w, id);
            if !errors.is_empty() {
                out["errors"] = json!(errors);
            }
            ToolResult::json(out)
        },
    );

    tool(&mut s, bus, ToolDef::new("list_component_types", "List component types",
        "Every component type that can be added (built-ins and registered scripts) with its fields, kinds and enum options.", empty.clone()).read_only(),
        |_, e| ToolResult::json(json!(e.world.registry.iter().map(|r| json!({
            "type": r.info.name, "menu": r.menu, "fields": schema_to_json(&r.info),
        })).collect::<Vec<_>>())));

    tool(
        &mut s,
        bus,
        ToolDef::new(
            "instantiate",
            "Instantiate",
            "Unity Instantiate: deep-copies an object (and children); optional position/euler_angles for the copy.",
            schema(
                json!({ "object": {}, "position": { "type": "array" }, "euler_angles": { "type": "array" } }),
                &["object"],
            ),
        ),
        |args, e| {
            let w = &mut e.world;
            let src = tri!(obj_arg(args, w, "object"));
            let Some(id) = w.instantiate(src) else {
                return ToolResult::error("instantiate failed");
            };
            if let Some(p) = tri!(vec3_arg(args, "position")) {
                w.set_position(id, p)
            }
            if let Some(r) = tri!(vec3_arg(args, "euler_angles")) {
                w.set_euler_angles(id, r)
            }
            ToolResult::json(object_json(w, id))
        },
    );

    tool(
        &mut s,
        bus,
        ToolDef::new(
            "save_scene",
            "Save scene",
            "Saves the open scene as YAML (path relative to the project, e.g. Assets/Scenes/Main.scene).",
            schema(json!({ "path": { "type": "string" } }), &[]),
        ),
        |args, e| {
            let w = &mut e.world;
            let Some(path) = args
                .get("path")
                .and_then(Json::as_str)
                .map(str::to_owned)
                .or_else(|| w.scene_path.clone())
            else {
                return ToolResult::error("`path` is required for a new scene");
            };
            let name = std::path::Path::new(&path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Scene")
                .to_owned();
            let file = w.serialize(&[], &name);
            let full = w.resource::<AssetServer>().full_path(&path);
            if let Some(dir) = full.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            match file
                .to_yaml()
                .and_then(|y| std::fs::write(&full, y).map_err(|e| e.to_string()))
            {
                Ok(()) => {
                    w.scene_name = name;
                    w.scene_path = Some(path.clone());
                    ToolResult::text(format!(
                        "Saved {} objects to {}",
                        file.objects.len(),
                        full.display()
                    ))
                }
                Err(err) => ToolResult::error(err),
            }
        },
    );

    tool(
        &mut s,
        bus,
        ToolDef::new(
            "load_scene",
            "Load scene",
            "Replaces the open scene with a saved one (or `additive: true` to add it).",
            schema(
                json!({ "path": { "type": "string" }, "additive": { "type": "boolean" } }),
                &["path"],
            ),
        )
        .destructive(),
        |args, e| {
            let path = args
                .get("path")
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_owned();
            let additive = args
                .get("additive")
                .and_then(Json::as_bool)
                .unwrap_or(false);
            match crate::load_scene(&mut e.world, &path, additive) {
                Ok(warnings) => {
                    e.start_systems();
                    ToolResult::json(
                        json!({ "loaded": path, "objects": e.world.len(), "warnings": warnings }),
                    )
                }
                Err(err) => ToolResult::error(err),
            }
        },
    );

    tool(&mut s, bus, ToolDef::new("new_scene", "New scene", "Clears the scene and creates Unity's defaults (Main Camera, Directional Light) unless `empty`.",
        schema(json!({ "empty": { "type": "boolean" } }), &[])).destructive(),
        |args, e| {
            e.world.clear(true);
            e.world.scene_name = "Untitled".into();
            e.world.scene_path = None;
            if !args.get("empty").and_then(Json::as_bool).unwrap_or(false) {
                crate::objects::create_default_scene(&mut e.world);
            }
            e.start_systems();
            ToolResult::text("New scene.")
        });

    tool(&mut s, bus, ToolDef::new("get_render_settings", "Get Lighting settings",
        "Environment settings (Lighting window): procedural sky, ambient, fog, exposure, bloom, shadow distance, vignette.", empty.clone()).read_only(),
        |_, e| {
            let rs = e.world.resource::<RenderSettings>();
            ToolResult::json(json!({ "fields": reflect_to_json(Some(&e.world), rs), "schema": schema_to_json(&oxide_core::reflect::Reflect::type_info(rs)) }))
        });

    tool(
        &mut s,
        bus,
        ToolDef::new(
            "set_render_settings",
            "Set Lighting settings",
            "Edits RenderSettings fields (see get_render_settings).",
            schema(json!({ "fields": { "type": "object" } }), &["fields"]),
        ),
        |args, e| {
            let empty = Map::new();
            let fields = args
                .get("fields")
                .and_then(Json::as_object)
                .unwrap_or(&empty);
            let info =
                oxide_core::reflect::Reflect::type_info(e.world.resource::<RenderSettings>());
            let (values, mut errors) = parse_fields(
                &info,
                fields,
                &e.world,
                Some(e.world.resource::<RenderSettings>() as &dyn oxide_core::reflect::Reflect),
            );
            set_fields(
                e.world.resource_mut::<RenderSettings>(),
                values,
                &mut errors,
            );
            ToolResult::json(
                json!({ "fields": reflect_to_json(Some(&e.world), e.world.resource::<RenderSettings>()), "errors": errors }),
            )
        },
    );

    tool(&mut s, bus, ToolDef::new("list_assets", "List assets",
        "Project files under Assets/ plus loaded meshes, materials, textures, models and animation clips (asset paths usable in fields).", empty.clone()).read_only(),
        |_, e| {
            let a = e.world.resource::<AssetServer>();
            let paths = |it: Vec<&str>| { let mut v: Vec<String> = it.into_iter().map(str::to_owned).collect(); v.sort(); v };
            ToolResult::json(json!({
                "files": a.list_files(),
                "meshes": paths(a.meshes.iter().map(|x| x.0).collect()),
                "materials": paths(a.materials.iter().map(|x| x.0).collect()),
                "textures": paths(a.textures.iter().map(|x| x.0).collect()),
                "models": paths(a.models.iter().map(|x| x.0).collect()),
                "animations": paths(a.animations.iter().map(|x| x.0).collect()),
                "audio": paths(a.audio.iter().map(|x| x.0).collect()),
            }))
        });

    tool(
        &mut s,
        bus,
        ToolDef::new(
            "create_material",
            "Create/edit material",
            "Creates a material at `path` (or edits it if it exists) with Lit/Unlit fields: base_color, base_map (texture path), metallic, \
         smoothness, normal_map, emission_color, emission_intensity, surface_type (Opaque/Cutout/Transparent), tiling, double_sided...",
            schema(
                json!({ "path": { "type": "string" }, "fields": { "type": "object" } }),
                &["path"],
            ),
        ),
        |args, e| {
            let path = args
                .get("path")
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_owned();
            let empty = Map::new();
            let fields = args
                .get("fields")
                .and_then(Json::as_object)
                .unwrap_or(&empty)
                .clone();
            for v in fields.values() {
                if let Some(p) = v.as_str().filter(|p| p.starts_with("Assets/")) {
                    if let Err(err) = e.world.resource_mut::<AssetServer>().ensure_loaded(p) {
                        return ToolResult::error(err);
                    }
                }
            }
            let mut mat = e
                .world
                .resource::<AssetServer>()
                .materials
                .handle_of(&path)
                .and_then(|h| e.world.resource::<AssetServer>().materials.get(&h).cloned())
                .unwrap_or_else(|| Material {
                    name: path.clone(),
                    ..Default::default()
                });
            let info = oxide_core::reflect::Reflect::type_info(&mat);
            let (values, mut errors) = parse_fields(
                &info,
                &fields,
                &e.world,
                Some(&mat as &dyn oxide_core::reflect::Reflect),
            );
            set_fields(&mut mat, values, &mut errors);
            let json = reflect_to_json(None, &mat);
            e.world
                .resource_mut::<AssetServer>()
                .materials
                .insert(path.clone(), mat);
            ToolResult::json(json!({ "path": path, "fields": json, "errors": errors }))
        },
    );

    tool(
        &mut s,
        bus,
        ToolDef::new(
            "import_model",
            "Import model",
            "Loads a glTF/GLB model from the project and instantiates it in the scene (meshes, materials, textures, skinning, animations).",
            schema(
                json!({ "path": { "type": "string" }, "position": { "type": "array" }, "euler_angles": { "type": "array" }, "scale": { "type": "array" } }),
                &["path"],
            ),
        ),
        |args, e| {
            let path = args.get("path").and_then(Json::as_str).unwrap_or_default();
            let h = match e.world.resource_mut::<AssetServer>().load_model(path) {
                Ok(h) => h,
                Err(err) => return ToolResult::error(err),
            };
            let Some(id) = crate::objects::instantiate_model(&mut e.world, &h) else {
                return ToolResult::error("model has no nodes");
            };
            let t = &mut e.world.get_mut(id).expect("created").transform;
            if let Some(v) = tri!(vec3_arg(args, "position")) {
                t.local_position = v
            }
            if let Some(v) = tri!(vec3_arg(args, "euler_angles")) {
                t.local_rotation = Quaternion::euler_v(v)
            }
            if let Some(v) = tri!(vec3_arg(args, "scale")) {
                t.local_scale = v
            }
            let clips: Vec<String> = e
                .world
                .resource::<AssetServer>()
                .models
                .get(&h)
                .map(|m| m.animations.iter().map(|a| a.path().to_owned()).collect())
                .unwrap_or_default();
            ToolResult::json(
                json!({ "root": hierarchy_json(&e.world, id, 0, 3), "animations": clips }),
            )
        },
    );

    tool(
        &mut s,
        bus,
        ToolDef::new(
            "simulate_input",
            "Simulate input",
            "Presses/releases keys or mouse buttons as if from the keyboard (for testing game controls). `action`: press, release or tap \
         (press for `frames` frames then release; default 1). Keys use Unity KeyCode names (W, Space, LeftShift, UpArrow...); \
         mouse buttons: MouseLeft, MouseRight, MouseMiddle. `mouse_delta` [dx, dy] moves the mouse for one frame.",
            schema(
                json!({
                    "keys": { "type": "array", "items": { "type": "string" } },
                    "action": { "type": "string", "enum": ["press", "release", "tap"] },
                    "frames": { "type": "integer", "minimum": 1, "maximum": 3600 },
                    "mouse_delta": { "type": "array" }
                }),
                &[],
            ),
        ),
        |args, e| {
            let action = args.get("action").and_then(Json::as_str).unwrap_or("tap");
            let mut keys = Vec::new();
            let mut buttons = Vec::new();
            for k in args
                .get("keys")
                .and_then(Json::as_array)
                .into_iter()
                .flatten()
                .filter_map(Json::as_str)
            {
                match k.to_ascii_lowercase().as_str() {
                    "mouseleft" => buttons.push(MouseButton::Left),
                    "mouseright" => buttons.push(MouseButton::Right),
                    "mousemiddle" => buttons.push(MouseButton::Middle),
                    _ => match KeyCode::parse(k) {
                        Some(kc) => keys.push(kc),
                        None => return ToolResult::error(format!("unknown key `{k}`")),
                    },
                }
            }
            let set = |e: &mut Engine, down: bool| {
                let input = e.world.resource_mut::<Input>();
                for &k in &keys {
                    input.key_event(k, down)
                }
                for &b in &buttons {
                    input.mouse_button_event(b, down)
                }
            };
            if let Some(d) = args.get("mouse_delta").and_then(Json::as_array) {
                let dx = d.first().and_then(Json::as_f64).unwrap_or(0.0) as f32;
                let dy = d.get(1).and_then(Json::as_f64).unwrap_or(0.0) as f32;
                e.world.resource_mut::<Input>().mouse_motion(dx, dy);
            }
            match action {
                "press" => set(e, true),
                "release" => set(e, false),
                _ => {
                    let frames = args
                        .get("frames")
                        .and_then(Json::as_u64)
                        .unwrap_or(1)
                        .clamp(1, 3600);
                    set(e, true);
                    for _ in 0..frames {
                        e.tick(1.0 / 60.0);
                        e.world.resource_mut::<Input>().end_frame();
                    }
                    set(e, false);
                    e.tick(1.0 / 60.0);
                    e.world.resource_mut::<Input>().end_frame();
                }
            }
            ToolResult::json(
                json!({ "held": e.world.resource::<Input>().pressed_keys(), "time": e.time().time() }),
            )
        },
    );

    crate::mcp_physics::register(&mut s, bus);
    crate::mcp_audio::register(&mut s, bus);

    tool(
        &mut s,
        bus,
        ToolDef::new("quit", "Quit engine", "Closes the window and exits.", empty).destructive(),
        |_, e| {
            e.quit_requested = true;
            ToolResult::text("Quitting.")
        },
    );

    s
}

#[allow(dead_code)]
fn _scene_file_type(_: SceneFile) {}
