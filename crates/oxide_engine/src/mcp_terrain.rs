//! MCP tools for terrain.

use crate::bus::Bus;
use crate::mcp_tools::{object_json, schema, tool, vec3_arg};
use crate::tri;
use oxide_assets::AssetServer;
use oxide_math::Vector3;
use oxide_mcp::{McpServer, ToolDef, ToolResult, json};
use serde_json::Value as Json;
use std::sync::Arc;

pub fn register(s: &mut McpServer, bus: &Arc<Bus>) {
    tool(
        s,
        bus,
        ToolDef::new(
            "create_terrain",
            "Create terrain",
            "Creates a generated terrain (noise heights, auto grass/rock/sand/snow texturing, heightfield collider). \
         `size` [x, max_height, z] (default [1000, 120, 1000]), `resolution` heightmap samples per side \
         (default 513), `seed`, `position` of the corner, optional `flatten` [u, v, radius_m] areas in 0..1 coords.",
            schema(
                json!({ "name": { "type": "string" }, "size": { "type": "array" }, "resolution": { "type": "integer", "minimum": 33, "maximum": 4097 },
                       "seed": { "type": "integer" }, "position": { "type": "array" }, "flatten": { "type": "array" } }),
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
