//! MCP tools for animation.

use crate::bus::Bus;
use crate::mcp_tools::{obj_arg, schema, tool};
use crate::tri;
use oxide_animation::{Animator, AvatarIKGoal};
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
            "animator",
            "Control an Animator",
            "Reads an object's Animator (layers with their current state, normalized time and \
         weight; parameters; root motion; humanoid mapping) and optionally changes it: \
         `parameters` {name: number|bool} (SetFloat/SetBool), `trigger` name (SetTrigger), `play` state name, \
         `cross_fade` [state, seconds], `speed`, `layer_weights` {layer name or index: 0..1}, \
         `apply_root_motion`, `look_at` [x,y,z] (weight 1; null turns it off), \
         `ik` {LeftHand|RightHand|LeftFoot|RightFoot: [x,y,z] or null} (position weight 1). \
         IK and LookAt apply only on layers with IK Pass: `ik_pass` {layer name or index: bool} \
         sets it on the controller.",
            schema(
                json!({ "object": {}, "parameters": { "type": "object" }, "trigger": { "type": "string" },
                       "play": { "type": "string" }, "cross_fade": { "type": "array" }, "speed": { "type": "number" },
                       "layer_weights": { "type": "object" }, "apply_root_motion": { "type": "boolean" },
                       "look_at": {}, "ik": { "type": "object" }, "ik_pass": { "type": "object" } }),
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
            if let Some(rm) = args.get("apply_root_motion").and_then(Json::as_bool) {
                a.apply_root_motion = rm;
            }
            if let Some(lw) = args.get("layer_weights").and_then(Json::as_object) {
                for (k, v) in lw {
                    let Some(i) = k.parse::<usize>().ok().or_else(|| a.get_layer_index(k)) else {
                        return ToolResult::error(format!("no layer `{k}`"));
                    };
                    a.set_layer_weight(i, v.as_f64().unwrap_or(1.0) as f32);
                }
            }
            let vec3 = |v: &Json| -> Option<Vector3> {
                let a = v.as_array()?;
                Some(Vector3::new(
                    a.first()?.as_f64()? as f32,
                    a.get(1)?.as_f64()? as f32,
                    a.get(2)?.as_f64()? as f32,
                ))
            };
            if let Some(v) = args.get("look_at") {
                match vec3(v) {
                    Some(p) => {
                        a.set_look_at_position(p);
                        a.set_look_at_weight(1.0, 0.3, 1.0, 0.5);
                    }
                    None => a.set_look_at_weight(0.0, 0.0, 0.0, 0.5),
                }
            }
            if let Some(ik) = args.get("ik").and_then(Json::as_object) {
                for (k, v) in ik {
                    let goal = match k.as_str() {
                        "LeftHand" => AvatarIKGoal::LeftHand,
                        "RightHand" => AvatarIKGoal::RightHand,
                        "LeftFoot" => AvatarIKGoal::LeftFoot,
                        "RightFoot" => AvatarIKGoal::RightFoot,
                        _ => return ToolResult::error(format!("unknown IK goal `{k}`")),
                    };
                    match vec3(v) {
                        Some(p) => {
                            a.set_ik_position(goal, p);
                            a.set_ik_position_weight(goal, 1.0);
                        }
                        None => a.set_ik_position_weight(goal, 0.0),
                    }
                }
            }
            let a = a.clone();
            if let Some(ip) = args.get("ik_pass").and_then(Json::as_object)
                && let Some(h) = a.controller.clone()
            {
                let assets = e.world.resource_mut::<AssetServer>();
                let Some(c) = assets.controllers.get_mut(&h) else {
                    return ToolResult::error("the controller is not loaded");
                };
                for (k, v) in ip {
                    let Some(i) = k.parse::<usize>().ok().or_else(|| c.layer_index(k)) else {
                        return ToolResult::error(format!("no layer `{k}`"));
                    };
                    if let Some(l) = c.layers.get_mut(i) {
                        l.ik_pass = v.as_bool().unwrap_or(true);
                    }
                }
            }
            let ctrl = a
                .controller
                .as_ref()
                .and_then(|h| e.world.resource::<AssetServer>().controllers.get(h));
            let states: Vec<String> = ctrl
                .map(|c| c.states().iter().map(|s| s.name.clone()).collect())
                .unwrap_or_default();
            let layers: Vec<Json> = (0..a.layer_count())
                .map(|i| {
                    let info = a.get_current_animator_state_info(i);
                    json!({
                        "name": a.get_layer_name(i),
                        "weight": ctrl.and_then(|c| c.layers.get(i)).map(|l| {
                            if i == 0 { 1.0 } else { a.rt.layer_weights.get(&i).copied().unwrap_or(l.default_weight) }
                        }),
                        "ik_pass": ctrl.and_then(|c| c.layers.get(i)).map(|l| l.ik_pass),
                        "current_state": info.name, "normalized_time": info.normalized_time,
                        "length": info.length,
                        "next_state": a.get_next_animator_state_info(i).map(|n| n.name),
                    })
                })
                .collect();
            let mut bones: Vec<String> = a.rt.human.keys().map(|b| b.name()).collect();
            bones.sort();
            ToolResult::json(json!({
                "controller": a.controller.as_ref().map(|h| h.path().to_owned()),
                "states": states, "layers": layers,
                "current_state": a.get_current_animator_state_info(0).name,
                "in_transition": a.is_in_transition(),
                "parameters": a.rt.params, "speed": a.speed,
                "apply_root_motion": a.apply_root_motion,
                "delta_position": a.delta_position().to_array(),
                "velocity": a.velocity().to_array(),
                "is_human": a.is_human(), "human_bones": bones,
            }))
        },
    );
}

pub fn register_ragdoll(s: &mut McpServer, bus: &Arc<Bus>) {
    tool(
        s,
        bus,
        ToolDef::new(
            "ragdoll",
            "Create or switch a ragdoll",
            "Ragdolls on humanoid characters (Unity's Ragdoll Wizard). `action`: `create` adds \
             Rigidbodies, colliders and CharacterJoints to the mapped bones (`mass` kg, default 70; \
             kinematic while an Animator drives them), `activate` stops the Animator and lets the \
             bodies fall, `deactivate` returns control to the Animator. `object` is the character root.",
            schema(
                json!({ "object": {}, "action": { "enum": ["create", "activate", "deactivate"] },
                        "mass": { "type": "number" } }),
                &["object", "action"],
            ),
        ),
        |args, e| {
            let id = tri!(obj_arg(args, &e.world, "object"));
            match args.get("action").and_then(Json::as_str) {
                Some("create") => {
                    let mass = args.get("mass").and_then(Json::as_f64).unwrap_or(70.0) as f32;
                    match oxide_standard::create_ragdoll(&mut e.world, id, mass) {
                        Ok(bodies) => {
                            let names: Vec<String> = bodies
                                .iter()
                                .filter_map(|&b| e.world.get(b).map(|o| o.name.clone()))
                                .collect();
                            ToolResult::json(json!({ "bodies": names }))
                        }
                        Err(err) => ToolResult::error(err),
                    }
                }
                Some(a @ ("activate" | "deactivate")) => {
                    let n = oxide_standard::set_ragdoll_active(&mut e.world, id, a == "activate");
                    if n == 0 {
                        return ToolResult::error(
                            "no Rigidbodies below this object; create the ragdoll first",
                        );
                    }
                    ToolResult::json(json!({ "bodies": n, "active": a == "activate" }))
                }
                _ => ToolResult::error("`action` must be create, activate or deactivate"),
            }
        },
    );
}
