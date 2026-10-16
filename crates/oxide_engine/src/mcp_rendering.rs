//! MCP tools for rendering features: reflection probes.

use crate::bus::Bus;
use crate::mcp_tools::{obj_arg, schema, tool};
use crate::tri;
use oxide_mcp::{McpServer, ToolDef, ToolResult, json};
use oxide_scene::ReflectionProbe;
use serde_json::Value as Json;
use std::sync::Arc;

pub fn register(s: &mut McpServer, bus: &Arc<Bus>) {
    tool(
        s,
        bus,
        ToolDef::new(
            "reflection_probes",
            "Reflection probes",
            "Lists Reflection Probes (box, mode, refresh mode, importance) and the last frame's reflection stats \
             (`active` = probes with a captured cubemap, up to 15; `captures` = cubemaps captured that frame, sky \
             included). `render` = an object with a probe: capture it again next frame (Unity `RenderProbe()`, the \
             only trigger for ViaScripting probes). Add probes with add_component \"ReflectionProbe\"; renderers \
             inside a probe's box reflect it, the sky elsewhere.",
            schema(
                json!({ "render": { "description": "Object id, name or path" } }),
                &[],
            ),
        ),
        |args, e| {
            if args.contains_key("render") {
                let id = tri!(obj_arg(args, &e.world, "render"));
                match e.world.get_component_mut::<ReflectionProbe>(id) {
                    Some(p) => p.render_probe(),
                    None => return ToolResult::error("that object has no ReflectionProbe"),
                }
            }
            let probes: Vec<Json> = e
                .world
                .query::<ReflectionProbe>()
                .map(|(id, p)| {
                    let (min, max) = p.bounds(e.world.position(id));
                    json!({
                        "id": id.to_bits(),
                        "name": e.world.get(id).map(|o| o.name.clone()),
                        "mode": format!("{:?}", p.mode),
                        "refresh_mode": format!("{:?}", p.refresh_mode),
                        "importance": p.importance,
                        "intensity": p.intensity,
                        "box_projection": p.box_projection,
                        "box_min": min.to_array(),
                        "box_max": max.to_array(),
                    })
                })
                .collect();
            let s = e.renderer.last_stats;
            ToolResult::json(json!({
                "probes": probes,
                "last_frame": { "active": s.reflection_probes, "captures": s.probe_captures },
            }))
        },
    );
}
