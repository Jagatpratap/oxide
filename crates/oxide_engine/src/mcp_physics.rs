//! MCP tools for physics: raycasts and physics settings.

use crate::bus::Bus;
use crate::mcp_tools::{schema, tool, vec3_arg};
use crate::tri;
use oxide_math::Vector3;
use oxide_mcp::{McpServer, ToolDef, ToolResult, json};
use oxide_physics::{DEFAULT_RAYCAST_LAYERS, PhysicsExt, RaycastHit};
use serde_json::Value as Json;
use std::sync::Arc;

pub fn register(s: &mut McpServer, bus: &Arc<Bus>) {
    tool(s, bus, ToolDef::new("raycast", "Physics raycast",
        "Unity Physics.Raycast from `origin` along `direction`: the first collider hit (object, point, normal, distance). \
         `all: true` returns every hit sorted by distance.",
        schema(json!({ "origin": { "type": "array" }, "direction": { "type": "array" }, "max_distance": { "type": "number" },
                       "layer_mask": { "type": "integer" }, "all": { "type": "boolean" } }), &["origin", "direction"])).read_only(),
        |args, e| {
            let o = tri!(vec3_arg(args, "origin")).unwrap_or_default();
            let d = tri!(vec3_arg(args, "direction")).unwrap_or(Vector3::NEG_Y);
            let max = args.get("max_distance").and_then(Json::as_f64).unwrap_or(1000.0) as f32;
            let mask = args.get("layer_mask").and_then(Json::as_u64).map_or(DEFAULT_RAYCAST_LAYERS, |m| m as u32);
            let w = &e.world;
            let hit_json = |h: &RaycastHit| json!({
                "object": { "id": h.game_object.to_bits(), "path": w.path_of(h.game_object) },
                "rigidbody": h.rigidbody.map(|r| w.path_of(r)),
                "point": h.point.to_array(), "normal": h.normal.to_array(), "distance": h.distance,
            });
            if args.get("all").and_then(Json::as_bool).unwrap_or(false) {
                ToolResult::json(json!(w.physics().raycast_all(o, d, max, mask).iter().map(hit_json).collect::<Vec<_>>()))
            } else {
                match w.physics().raycast(o, d, max, mask) {
                    Some(h) => ToolResult::json(hit_json(&h)),
                    None => ToolResult::json(json!({ "hit": false })),
                }
            }
        });

    tool(
        s,
        bus,
        ToolDef::new(
            "physics_settings",
            "Physics settings",
            "Reads and optionally changes physics settings: gravity [x,y,z], debug_draw (collider wireframes), \
         queries_hit_triggers, ignore_layer_collision [layerA, layerB, ignore]. Returns body/collider counts.",
            schema(
                json!({ "gravity": { "type": "array" }, "debug_draw": { "type": "boolean" }, "queries_hit_triggers": { "type": "boolean" },
                       "ignore_layer_collision": { "type": "array" } }),
                &[],
            ),
        ),
        |args, e| {
            let g = tri!(vec3_arg(args, "gravity"));
            let p = e.world.physics_mut();
            if let Some(g) = g {
                p.set_gravity(g)
            }
            if let Some(d) = args.get("debug_draw").and_then(Json::as_bool) {
                p.debug_draw = d
            }
            if let Some(q) = args.get("queries_hit_triggers").and_then(Json::as_bool) {
                p.queries_hit_triggers = q
            }
            if let Some(a) = args.get("ignore_layer_collision").and_then(Json::as_array) {
                let n = |i: usize| a.get(i).and_then(Json::as_u64).unwrap_or(0) as u8;
                p.ignore_layer_collision(
                    n(0),
                    n(1),
                    a.get(2).and_then(Json::as_bool).unwrap_or(true),
                );
            }
            let (bodies, colliders) = p.counts();
            ToolResult::json(
                json!({ "gravity": p.gravity().to_array(), "debug_draw": p.debug_draw,
                "queries_hit_triggers": p.queries_hit_triggers, "bodies": bodies, "colliders": colliders }),
            )
        },
    );
}
