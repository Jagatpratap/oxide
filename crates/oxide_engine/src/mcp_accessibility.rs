//! MCP tool for accessibility: what a screen reader sees and acting like one.

use crate::bus::Bus;
use crate::mcp_tools::{schema, tool};
use oxide_mcp::{McpServer, ToolDef, ToolResult, json};
use oxide_ui::accessibility::{AccessKitSnapshot, AccessibilityTarget, AssistiveSupport};
use oxide_ui::egui::accesskit;
use serde_json::Value as Json;
use std::sync::Arc;

pub fn register(s: &mut McpServer, bus: &Arc<Bus>) {
    tool(
        s,
        bus,
        ToolDef::new(
            "accessibility",
            "Accessibility",
            "What screen readers see: whether one is on, the script AccessibilityHierarchy, and the \
             AccessKit tree the engine last gave the platform (labelled nodes: role, label, value, \
             hint, actions; game UI and editor alike). `screen_reader`: true/false turns the tree on \
             or off as a screen reader would (it shows up on the next frame). `act` {label, action: \
             click | increment | decrement | focus} performs a screen reader action on the first node \
             with that label. Also returns the AssistiveSupport events not yet taken by scripts.",
            schema(
                json!({
                    "screen_reader": { "type": "boolean" },
                    "act": { "type": "object" }
                }),
                &[],
            ),
        ),
        |args, e| {
            if let Some(on) = args.get("screen_reader").and_then(Json::as_bool) {
                let ctx = e.gui.state.egui_ctx().clone();
                if on {
                    ctx.enable_accesskit();
                } else {
                    ctx.disable_accesskit();
                    e.world.resources.remove::<AccessKitSnapshot>();
                }
            }
            if let Some(act) = args.get("act") {
                let label = act.get("label").and_then(Json::as_str).unwrap_or("");
                let action = match act.get("action").and_then(Json::as_str).unwrap_or("click") {
                    "click" => accesskit::Action::Click,
                    "increment" => accesskit::Action::Increment,
                    "decrement" => accesskit::Action::Decrement,
                    "focus" => accesskit::Action::Focus,
                    other => return ToolResult::error(format!("unknown action `{other}`")),
                };
                let Some(node) = e
                    .world
                    .try_resource::<AccessKitSnapshot>()
                    .and_then(|s| s.nodes.iter().find(|n| n.label == label))
                    .map(|n| n.id)
                else {
                    return ToolResult::error(format!(
                        "no node labelled `{label}` (turn the tree on with screen_reader: true first)"
                    ));
                };
                e.gui
                    .state
                    .on_accesskit_action_request(accesskit::ActionRequest {
                        action,
                        target_tree: accesskit::TreeId::ROOT,
                        target_node: node,
                        data: None,
                    });
            }
            let support = e.world.try_resource::<AssistiveSupport>();
            let hierarchy: Vec<Json> = support
                .and_then(|s| s.active_hierarchy.as_ref())
                .map(|h| {
                    h.nodes_in_order()
                        .into_iter()
                        .filter_map(|id| h.try_get_node(id))
                        .map(|n| {
                            json!({
                                "id": n.id, "label": n.label, "value": n.value, "hint": n.hint,
                                "role": format!("{:?}", n.role), "parent": n.parent(),
                                "frame": [n.frame.x, n.frame.y, n.frame.width, n.frame.height],
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            let tree: Vec<Json> = e
                .world
                .try_resource::<AccessKitSnapshot>()
                .map(|s| {
                    s.nodes
                        .iter()
                        .map(|n| {
                            json!({
                                "role": n.role, "label": n.label, "value": n.value,
                                "hint": n.description, "actions": n.actions,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            let world = &e.world;
            let events: Vec<Json> = world
                .try_resource::<AssistiveSupport>()
                .map(|s| s.pending_events())
                .unwrap_or_default()
                .iter()
                .map(|ev| {
                    let target = match ev.target {
                        AccessibilityTarget::Node(n) => json!({ "node": n }),
                        AccessibilityTarget::Object(o) => json!({
                            "object": o.to_bits(),
                            "name": world.get(o).map(|g| g.name.clone()),
                        }),
                    };
                    json!({ "target": target, "action": format!("{:?}", ev.action) })
                })
                .collect();
            ToolResult::json(json!({
                "screen_reader_enabled": support.is_some_and(|s| s.is_screen_reader_enabled()),
                "hierarchy": hierarchy,
                "tree": tree,
                "events": events,
            }))
        },
    );
}
