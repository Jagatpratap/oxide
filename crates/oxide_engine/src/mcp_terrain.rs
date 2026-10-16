//! MCP tools for terrain.

use crate::bus::Bus;
use crate::mcp_tools::{object_json, schema, tool, vec3_arg};
use crate::tri;
use oxide_assets::AssetServer;
use oxide_math::Vector3;
use oxide_mcp::{McpServer, ToolDef, ToolResult, json};
use serde_json::Value as Json;
use std::sync::Arc;

/// Unwraps a `Result<_, impl Into<String>>`, returning a tool error.
macro_rules! ok {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(e) => return ToolResult::error(e),
        }
    };
}

pub fn register(s: &mut McpServer, bus: &Arc<Bus>) {
    tool(
        s,
        bus,
        ToolDef::new(
            "create_terrain",
            "Create terrain",
            "Creates a generated terrain (noise heights, auto grass/rock/sand/snow texturing, heightfield collider). \
         `size` [x, max_height, z] (default [1000, 120, 1000]), `resolution` heightmap samples per side \
         (default 513), `seed`, `position` of the corner, optional `flatten` [u, v, radius_m] areas in 0..1 coords, \
         `save_path` (a `.terraindata` file to store the data in, so the scene keeps it).",
            schema(
                json!({ "name": { "type": "string" }, "size": { "type": "array" }, "resolution": { "type": "integer", "minimum": 33, "maximum": 4097 },
                       "seed": { "type": "integer" }, "position": { "type": "array" }, "flatten": { "type": "array" },
                       "save_path": { "type": "string" } }),
                &[],
            ),
        ),
        |args, e| {
            let size = tri!(vec3_arg(args, "size")).unwrap_or(Vector3::new(1000.0, 120.0, 1000.0));
            let res = args.get("resolution").and_then(Json::as_u64).unwrap_or(513) as u32;
            let seed = args.get("seed").and_then(Json::as_u64).unwrap_or(1) as u32;
            let name = args
                .get("name")
                .and_then(Json::as_str)
                .unwrap_or("Terrain")
                .to_owned();
            let id = oxide_terrain::create_terrain(&mut e.world, &name, size, res, seed);
            if let Some(p) = tri!(vec3_arg(args, "position")) {
                e.world.set_position(id, p);
            }
            if let Some(areas) = args.get("flatten").and_then(Json::as_array) {
                let h = e
                    .world
                    .get_component::<oxide_terrain::Terrain>(id)
                    .and_then(|t| t.data.clone());
                if let Some(h) = h
                    && let Some(d) = e.world.resource_mut::<AssetServer>().terrains.get_mut(&h)
                {
                    for a in areas.iter().filter_map(Json::as_array) {
                        let f = |i: usize| a.get(i).and_then(Json::as_f64).unwrap_or(0.0) as f32;
                        let (u, v, r) = (f(0), f(1), f(2));
                        let hh = d.interpolated_height(u, v) / d.size.y;
                        d.flatten(u, v, r, r * 0.6, hh);
                    }
                    d.auto_splat(0.06, 0.82, 32.0);
                }
            }
            if let Some(path) = args.get("save_path").and_then(Json::as_str) {
                let h = e
                    .world
                    .get_component::<oxide_terrain::Terrain>(id)
                    .and_then(|t| t.data.clone());
                if let Some(h) = h {
                    let assets = e.world.resource_mut::<AssetServer>();
                    let Some(d) = assets.terrains.get(&h).cloned() else {
                        return ToolResult::error("terrain data missing");
                    };
                    let nh = ok!(assets.save_terrain_data(path, &d));
                    if let Some(t) = e.world.get_component_mut::<oxide_terrain::Terrain>(id) {
                        t.data = Some(nh);
                    }
                }
            }
            ToolResult::json(object_json(&e.world, id))
        },
    );

    tool(s, bus, ToolDef::new("terrain_height", "Sample terrain height",
        "Unity Terrain.SampleHeight: world Y of the terrain surface at `x`, `z` (null off-terrain).",
        schema(json!({ "x": { "type": "number" }, "z": { "type": "number" } }), &["x", "z"])).read_only(),
        |args, e| {
            let x = args.get("x").and_then(Json::as_f64).unwrap_or(0.0) as f32;
            let z = args.get("z").and_then(Json::as_f64).unwrap_or(0.0) as f32;
            ToolResult::json(json!({ "height": oxide_terrain::sample_height(&e.world, Vector3::new(x, 0.0, z)) }))
        });
}

/// Particle effect presets.
/// The data handle and origin of a terrain object.
fn terrain_of(
    world: &oxide_scene::World,
    id: oxide_scene::GameObjectId,
) -> Result<(oxide_assets::Handle<oxide_assets::TerrainData>, Vector3), String> {
    let h = world
        .get_component::<oxide_terrain::Terrain>(id)
        .and_then(|t| t.data.clone())
        .ok_or("the object has no Terrain with data")?;
    Ok((h, world.position(id)))
}

fn terrain_info(world: &oxide_scene::World, id: oxide_scene::GameObjectId) -> Json {
    let Ok((h, o)) = terrain_of(world, id) else {
        return Json::Null;
    };
    let Some(d) = world.resource::<AssetServer>().terrains.get(&h) else {
        return Json::Null;
    };
    json!({
        "data": h.path(),
        "origin": o.to_array(),
        "size": d.size.to_array(),
        "heightmap_resolution": d.resolution,
        "control_resolution": d.splat_resolution,
        "detail_resolution": d.detail_resolution,
        "layers": d.layers.iter().map(|l| json!({"name": l.name, "diffuse": l.diffuse.as_ref().map(|h| h.path()), "tile_size": l.tile_size.to_array()})).collect::<Vec<_>>(),
        "tree_prototypes": d.tree_prototypes.iter().map(|p| p.name()).collect::<Vec<_>>(),
        "trees": d.trees.len(),
        "detail_prototypes": d.detail_prototypes.iter().map(|p| json!({"name": p.name(), "mode": format!("{:?}", p.render_mode)})).collect::<Vec<_>>(),
        "has_holes": !d.holes.is_empty(),
        "height_range": [
            d.heights.iter().copied().fold(f32::MAX, f32::min) * d.size.y,
            d.heights.iter().copied().fold(f32::MIN, f32::max) * d.size.y,
        ],
        "wind": [d.wind.speed, d.wind.size, d.wind.bending],
    })
}

pub fn register_terrain_edit(s: &mut McpServer, bus: &Arc<Bus>) {
    tool(
        s,
        bus,
        ToolDef::new(
            "terrain_edit",
            "Edit a terrain (Unity terrain tools)",
            "Reads or edits a Terrain like Unity's terrain tools. `object` is the terrain. `action`: \
             info (default) | paint | add_layer | remove_layer | add_tree | add_detail | mass_place_trees | wind | save. \
             paint: `tool` raise | lower | set_height | smooth | stamp | texture | holes | fill_holes | trees | erase_trees | \
             details | erase_details, `position` [x,y,z] world, `radius` m (default 10), `strength` 0..1 (default 0.5), \
             `steps` repeats (default 10), `height` m for set_height / stamp, `layer` for texture, `prototype` for \
             trees / details, `target` (texture 0..1, details 0..16), `density` trees per 100 m². \
             add_layer: `texture` (or `layer_asset` a .terrainlayer), `tile_size`. add_tree: `source` (model, prefab or \
             builtin:tree/simple). add_detail: `texture` (grass; builtin:terrain/grass_blades) or `source` (detail mesh). \
             mass_place_trees: `count`, `keep`. wind: `speed`, `size`, `bending`. save: writes the .terraindata \
             (`path` to save under a new file). Painting across neighbouring terrains (same Grouping ID) is done for you.",
            schema(
                json!({
                    "object": {}, "action": { "type": "string" }, "tool": { "type": "string" },
                    "position": { "type": "array" }, "radius": { "type": "number" }, "strength": { "type": "number" },
                    "steps": { "type": "integer", "minimum": 1, "maximum": 1000 }, "height": { "type": "number" },
                    "layer": { "type": "integer" }, "prototype": { "type": "integer" }, "target": { "type": "number" },
                    "density": { "type": "number" }, "texture": { "type": "string" }, "layer_asset": { "type": "string" },
                    "tile_size": { "type": "number" }, "source": { "type": "string" }, "count": { "type": "integer" },
                    "keep": { "type": "boolean" }, "speed": { "type": "number" }, "size": { "type": "number" },
                    "bending": { "type": "number" }, "path": { "type": "string" }
                }),
                &["object"],
            ),
        ),
        |args, e| {
            let id = tri!(crate::mcp_tools::obj_arg(args, &e.world, "object"));
            let (h, origin) = ok!(terrain_of(&e.world, id));
            let num = |k: &str| args.get(k).and_then(Json::as_f64).map(|v| v as f32);
            let action = args.get("action").and_then(Json::as_str).unwrap_or("info");
            match action {
                "info" => {}
                "paint" => {
                    let tool = args.get("tool").and_then(Json::as_str).unwrap_or("raise");
                    let Some(p) = tri!(vec3_arg(args, "position")) else {
                        return ToolResult::error("`position` is required");
                    };
                    let steps = args.get("steps").and_then(Json::as_u64).unwrap_or(10) as usize;
                    let brush = oxide_assets::Brush {
                        size: num("radius").unwrap_or(10.0) * 2.0,
                        strength: num("strength").unwrap_or(0.5),
                        ..Default::default()
                    };
                    let mut rng = 0x1234_5678u32;
                    let terrains = oxide_terrain::connected(&e.world, id);
                    let base_y = origin.y;
                    let assets = e.world.resource_mut::<AssetServer>();
                    for (_, th, to) in terrains {
                        let Some(d) = assets.terrains.get_mut(&th) else {
                            continue;
                        };
                        let (u, v) = ((p.x - to.x) / d.size.x, (p.z - to.z) / d.size.z);
                        let r = brush.size * 0.5;
                        if u * d.size.x < -r
                            || v * d.size.z < -r
                            || (u - 1.0) * d.size.x > r
                            || (v - 1.0) * d.size.z > r
                        {
                            continue;
                        }
                        for _ in 0..steps {
                            match tool {
                                "raise" | "lower" => {
                                    let m = d.size.y * 0.25 / 30.0
                                        * if tool == "lower" { -1.0 } else { 1.0 };
                                    d.paint_height(u, v, &brush, oxide_assets::HeightOp::Raise(m));
                                }
                                "set_height" => {
                                    let target = num("height").unwrap_or(0.0) + base_y - to.y;
                                    d.paint_height(
                                        u,
                                        v,
                                        &brush,
                                        oxide_assets::HeightOp::SetHeight(target),
                                    );
                                }
                                "smooth" => {
                                    d.paint_height(u, v, &brush, oxide_assets::HeightOp::Smooth)
                                }
                                "stamp" => d.paint_height(
                                    u,
                                    v,
                                    &brush,
                                    oxide_assets::HeightOp::Stamp {
                                        height: num("height").unwrap_or(10.0),
                                        add: false,
                                    },
                                ),
                                "texture" => d.paint_texture(
                                    u,
                                    v,
                                    &brush,
                                    args.get("layer").and_then(Json::as_u64).unwrap_or(0) as usize,
                                    num("target").unwrap_or(1.0),
                                ),
                                "holes" | "fill_holes" => {
                                    d.paint_holes(u, v, &brush, tool == "fill_holes")
                                }
                                "trees" | "erase_trees" => d.paint_trees(
                                    u,
                                    v,
                                    &brush,
                                    args.get("prototype").and_then(Json::as_u64).map_or(
                                        if tool == "erase_trees" { u32::MAX } else { 0 },
                                        |x| x as u32,
                                    ),
                                    num("density").unwrap_or(3.0),
                                    (0.9, 1.2),
                                    (0.9, 1.2),
                                    0.1,
                                    true,
                                    tool == "erase_trees",
                                    &mut rng,
                                ),
                                "details" | "erase_details" => d.paint_details(
                                    u,
                                    v,
                                    &brush,
                                    args.get("prototype").and_then(Json::as_u64).unwrap_or(0)
                                        as usize,
                                    num("target").unwrap_or(8.0),
                                    tool == "erase_details",
                                ),
                                other => {
                                    return ToolResult::error(format!(
                                        "unknown paint tool {other}"
                                    ));
                                }
                            }
                        }
                        d.snap_trees_to_ground();
                    }
                }
                "add_layer" => {
                    let assets = e.world.resource_mut::<AssetServer>();
                    let (layer, asset) = match args.get("layer_asset").and_then(Json::as_str) {
                        Some(a) => {
                            let lh: oxide_assets::Handle<oxide_assets::TerrainLayer> =
                                oxide_assets::Handle::from_path(a);
                            (
                                ok!(assets
                                    .data_asset(&lh)
                                    .cloned()
                                    .ok_or("cannot load the Terrain Layer")),
                                Some(a.to_owned()),
                            )
                        }
                        None => {
                            let t = args
                                .get("texture")
                                .and_then(Json::as_str)
                                .unwrap_or("builtin:terrain/grass");
                            if !t.starts_with("builtin:") {
                                ok!(assets.ensure_loaded(t));
                            } else {
                                oxide_terrain::register_builtin_textures(assets);
                            }
                            let tile = num("tile_size").unwrap_or(10.0);
                            (
                                oxide_assets::TerrainLayer {
                                    name: t
                                        .rsplit('/')
                                        .next()
                                        .unwrap_or(t)
                                        .split('.')
                                        .next()
                                        .unwrap_or(t)
                                        .to_owned(),
                                    diffuse: Some(oxide_assets::Handle::from_path(t)),
                                    tile_size: oxide_math::Vector2::splat(tile),
                                    ..Default::default()
                                },
                                None,
                            )
                        }
                    };
                    let d = ok!(assets.terrains.get_mut(&h).ok_or("no data"));
                    d.add_layer(layer, asset);
                }
                "remove_layer" => {
                    let i = args.get("layer").and_then(Json::as_u64).unwrap_or(0) as usize;
                    let d = ok!(e
                        .world
                        .resource_mut::<AssetServer>()
                        .terrains
                        .get_mut(&h)
                        .ok_or("no data"));
                    d.remove_layer(i);
                }
                "add_tree" => {
                    let src = args
                        .get("source")
                        .and_then(Json::as_str)
                        .unwrap_or("builtin:tree/simple");
                    let (mesh, mats) = ok!(oxide_terrain::resolve_mesh_source(&mut e.world, src));
                    let d = ok!(e
                        .world
                        .resource_mut::<AssetServer>()
                        .terrains
                        .get_mut(&h)
                        .ok_or("no data"));
                    let mut p = oxide_assets::TreePrototype::new(mesh, mats);
                    p.source = Some(src.to_owned());
                    d.tree_prototypes.push(p);
                    d.touch_trees();
                }
                "add_detail" => {
                    let mut p = oxide_assets::DetailPrototype::default();
                    if let Some(src) = args.get("source").and_then(Json::as_str) {
                        let (mesh, mats) =
                            ok!(oxide_terrain::resolve_mesh_source(&mut e.world, src));
                        p.source = Some(src.to_owned());
                        p.mesh = Some(mesh);
                        p.materials = mats;
                        p.render_mode = oxide_assets::DetailRenderMode::VertexLit;
                    } else {
                        let t = args
                            .get("texture")
                            .and_then(Json::as_str)
                            .unwrap_or("builtin:terrain/grass_blades");
                        let assets = e.world.resource_mut::<AssetServer>();
                        if t.starts_with("builtin:") {
                            oxide_terrain::register_builtin_textures(assets);
                        } else {
                            ok!(assets.ensure_loaded(t));
                        }
                        p.texture = Some(oxide_assets::Handle::from_path(t));
                    }
                    let d = ok!(e
                        .world
                        .resource_mut::<AssetServer>()
                        .terrains
                        .get_mut(&h)
                        .ok_or("no data"));
                    d.detail_prototypes.push(p);
                    d.fit_details();
                    d.touch_details();
                }
                "mass_place_trees" => {
                    let count = args.get("count").and_then(Json::as_u64).unwrap_or(1000) as usize;
                    let keep = args.get("keep").and_then(Json::as_bool).unwrap_or(true);
                    let d = ok!(e
                        .world
                        .resource_mut::<AssetServer>()
                        .terrains
                        .get_mut(&h)
                        .ok_or("no data"));
                    d.mass_place_trees(count, 77, keep);
                }
                "wind" => {
                    let d = ok!(e
                        .world
                        .resource_mut::<AssetServer>()
                        .terrains
                        .get_mut(&h)
                        .ok_or("no data"));
                    if let Some(v) = num("speed") {
                        d.wind.speed = v;
                    }
                    if let Some(v) = num("size") {
                        d.wind.size = v;
                    }
                    if let Some(v) = num("bending") {
                        d.wind.bending = v;
                    }
                    d.touch_details();
                }
                "save" => {
                    let path = args
                        .get("path")
                        .and_then(Json::as_str)
                        .map_or_else(|| h.path().to_owned(), str::to_owned);
                    if !path.ends_with(".terraindata") {
                        return ToolResult::error(
                            "save needs a .terraindata path (this data was created in memory)",
                        );
                    }
                    let assets = e.world.resource_mut::<AssetServer>();
                    let d = ok!(assets.terrains.get(&h).cloned().ok_or("no data"));
                    let nh = ok!(assets.save_terrain_data(&path, &d));
                    if let Some(t) = e.world.get_component_mut::<oxide_terrain::Terrain>(id) {
                        t.data = Some(nh);
                    }
                }
                other => return ToolResult::error(format!("unknown action {other}")),
            }
            ToolResult::json(terrain_info(&e.world, id))
        },
    );
}

pub fn register_effects(s: &mut McpServer, bus: &Arc<Bus>) {
    tool(
        s,
        bus,
        ToolDef::new(
            "spawn_effect",
            "Spawn particle effect",
            "Creates a ParticleSystem from a preset (smoke, fire, explosion, muzzle_flash, dust, rain) at `position`. \
         One-shot presets (explosion, muzzle_flash) remove themselves when done unless `keep` is true.",
            schema(
                json!({ "preset": { "type": "string", "enum": ["smoke", "fire", "explosion", "muzzle_flash", "dust", "rain"] },
                       "position": { "type": "array" }, "parent": {}, "keep": { "type": "boolean" } }),
                &["preset"],
            ),
        ),
        |args, e| {
            use oxide_particles::ParticleSystem as P;
            let preset = args.get("preset").and_then(Json::as_str).unwrap_or("smoke");
            let mut ps = match preset {
                "smoke" => P::smoke(),
                "fire" => P::fire(),
                "explosion" => P::explosion(),
                "muzzle_flash" => P::muzzle_flash(),
                "dust" => P::dust(),
                "rain" => P::rain(),
                other => return ToolResult::error(format!("unknown preset `{other}`")),
            };
            ps.play();
            let pos = tri!(vec3_arg(args, "position")).unwrap_or(Vector3::ZERO);
            let id = e.world.create_at(
                &format!("Effect ({preset})"),
                pos,
                oxide_math::Quaternion::IDENTITY,
            );
            if args.contains_key("parent") {
                let p = tri!(crate::mcp_tools::obj_arg(args, &e.world, "parent"));
                let _ = e.world.set_parent(id, Some(p), false);
            }
            let one_shot = !ps.looping;
            let life = ps.duration + ps.start_lifetime.y + 0.2;
            e.world.add_component(id, ps);
            if one_shot && !args.get("keep").and_then(Json::as_bool).unwrap_or(false) {
                let t = e.time().time() + life;
                e.world
                    .resource_mut::<oxide_scene::Timers>()
                    .destroy_at(id, t);
            }
            ToolResult::json(object_json(&e.world, id))
        },
    );
}
