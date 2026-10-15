//! MCP tools for animation.

use crate::bus::Bus;
use crate::mcp_tools::{obj_arg, schema, tool};
use crate::tri;
use oxide_animation::Animator;
use oxide_assets::AssetServer;
use oxide_mcp::{McpServer, ToolDef, ToolResult, json};
use serde_json::Value as Json;
use std::sync::Arc;

pub fn register(s: &mut McpServer, bus: &Arc<Bus>) {
    tool(
        s,
        bus,
        ToolDef::new(
            "animator",
            "Control an Animator",
            "Reads an object's Animator (states, parameters, current state) and optionally changes it: \
         `parameters` {name: number|bool} (SetFloat/SetBool), `trigger` name (SetTrigger), `play` state name, \
         `cross_fade` [state, seconds], `speed`.",
            schema(
                json!({ "object": {}, "parameters": { "type": "object" }, "trigger": { "type": "string" },
                       "play": { "type": "string" }, "cross_fade": { "type": "array" }, "speed": { "type": "number" } }),
                &["object"],
            ),
        ),
        |args, e| {
            let id = tri!(obj_arg(args, &e.world, "object"));
            let Some(a) = e.world.get_component_mut::<Animator>(id) else {
                return ToolResult::error("that object has no Animator");
            };
            if let Some(p) = args.get("parameters").and_then(Json::as_object) {
                for (k, v) in p {
                    match v {
                        Json::Bool(b) => a.set_bool(k, *b),
                        Json::Number(n) => a.set_float(k, n.as_f64().unwrap_or(0.0) as f32),
                        _ => {
                            return ToolResult::error(format!(
                                "parameter `{k}` must be a number or boolean"
                            ));
                        }
                    }
                }
            }
            if let Some(t) = args.get("trigger").and_then(Json::as_str) {
                a.set_trigger(t)
            }
            if let Some(st) = args.get("play").and_then(Json::as_str) {
                a.play(st)
            }
            if let Some(cf) = args.get("cross_fade").and_then(Json::as_array) {
                let st = cf
                    .first()
                    .and_then(Json::as_str)
                    .unwrap_or_default()
                    .to_owned();
                a.cross_fade(&st, cf.get(1).and_then(Json::as_f64).unwrap_or(0.25) as f32);
            }
            if let Some(sp) = args.get("speed").and_then(Json::as_f64) {
                a.speed = sp as f32
            }
            let a = a.clone();
            let ctrl = a
                .controller
                .as_ref()
                .and_then(|h| e.world.resource::<AssetServer>().controllers.get(h));
            let states: Vec<String> = ctrl
                .map(|c| c.states.iter().map(|s| s.name.clone()).collect())
                .unwrap_or_default();
            let current =
                a.rt.current
                    .as_ref()
                    .and_then(|p| states.get(p.state).cloned());
            ToolResult::json(json!({
                "controller": a.controller.as_ref().map(|h| h.path().to_owned()),
                "states": states, "current_state": current,
                "in_transition": a.is_in_transition(),
                "time": a.rt.current.as_ref().map(|p| p.time),
                "parameters": a.rt.params, "speed": a.speed,
            }))
        },
    );
}
