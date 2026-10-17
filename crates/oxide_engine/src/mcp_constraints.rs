//! MCP tool for constraints (Position, Rotation, Scale, Parent, Aim, Look At).

use crate::bus::Bus;
use crate::json_values::resolve_object;
use crate::mcp_tools::{schema, tool};
use oxide_mcp::{McpServer, ToolDef, ToolResult, json};
use oxide_scene::constraints::{self, ConstraintKind};
use oxide_scene::{
    AimConstraint, ConstraintSource, GameObjectId, LookAtConstraint, ParentConstraint,
    PositionConstraint, RotationConstraint, ScaleConstraint, World,
};
use serde_json::Value as Json;
use std::sync::Arc;

fn name(w: &World, id: Option<GameObjectId>) -> Json {
    id.and_then(|i| w.get(i))
        .map_or(Json::Null, |o| Json::String(o.name.clone()))
}

/// `(kind, active, locked, weight, sources)` of every constraint on `id`.
fn constraints_json(w: &World, id: GameObjectId) -> Vec<Json> {
    let mut out = Vec::new();
    macro_rules! each {
        ($t:ty, $k:expr) => {
            if let Some(c) = w.get_component::<$t>(id) {
                out.push(json!({
                    "kind": $k.type_name(),
                    "active": c.constraint_active,
                    "locked": c.locked,
                    "weight": c.weight,
                    "sources": c.sources.iter().map(|s| json!({
                        "object": name(w, s.source_transform),
                        "weight": s.weight,
                    })).collect::<Vec<_>>(),
                }));
            }
        };
    }
    each!(PositionConstraint, ConstraintKind::Position);
    each!(RotationConstraint, ConstraintKind::Rotation);
    each!(ScaleConstraint, ConstraintKind::Scale);
    each!(ParentConstraint, ConstraintKind::Parent);
    each!(AimConstraint, ConstraintKind::Aim);
    each!(LookAtConstraint, ConstraintKind::LookAt);
    out
}

fn add_source(
    w: &mut World,
    id: GameObjectId,
    kind: ConstraintKind,
    src: ConstraintSource,
) -> bool {
    macro_rules! push {
        ($t:ty) => {
            match w.get_component_mut::<$t>(id) {
                Some(c) => {
                    c.sources.push(src);
                    true
                }
                None => false,
            }
        };
    }
    match kind {
        ConstraintKind::Position => push!(PositionConstraint),
        ConstraintKind::Rotation => push!(RotationConstraint),
        ConstraintKind::Scale => push!(ScaleConstraint),
        ConstraintKind::Parent => push!(ParentConstraint),
        ConstraintKind::Aim => push!(AimConstraint),
        ConstraintKind::LookAt => push!(LookAtConstraint),
    }
}

pub fn register(s: &mut McpServer, bus: &Arc<Bus>) {
    tool(
        s,
        bus,
        ToolDef::new(
            "constraints",
            "Constraints",
            "Unity's constraint components. Without `object`: every constrained object. With `object` and \
             `kind` (Position | Rotation | Scale | Parent | Aim | LookAt): `action` add (adds the component if \
             missing), add_source (`source` object, `weight`), activate (keep the current pose, compute offsets; \
             Unity's Activate button) or zero (clear offsets, snap onto the sources). Edit other fields \
             (weight, locked, axes, aim/up vectors, world up type, offsets) with set_component_field.",
            schema(
                json!({
                    "object": {},
                    "kind": { "type": "string", "enum": ["Position", "Rotation", "Scale", "Parent", "Aim", "LookAt"] },
                    "action": { "type": "string", "enum": ["add", "add_source", "activate", "zero"] },
                    "source": {},
                    "weight": { "type": "number" }
                }),
                &[],
            ),
        ),
        |args, e| {
            let w = &mut e.world;
            let Some(v) = args.get("object") else {
                let ids: Vec<GameObjectId> = w.objects().map(|(id, _)| id).collect();
                let all: Vec<Json> = ids
                    .into_iter()
                    .filter_map(|id| {
                        let c = constraints_json(w, id);
                        (!c.is_empty()).then(|| json!({ "object": name(w, Some(id)), "id": id.to_bits(), "constraints": c }))
                    })
                    .collect();
                return ToolResult::json(json!({ "objects": all }));
            };
            let Some(id) = resolve_object(w, v) else {
                return ToolResult::error(format!("no GameObject matches {v}"));
            };
            if let Some(action) = args.get("action").and_then(Json::as_str) {
                let Some(kind) = args
                    .get("kind")
                    .and_then(Json::as_str)
                    .and_then(ConstraintKind::from_type_name)
                else {
                    return ToolResult::error(
                        "`kind` must be Position, Rotation, Scale, Parent, Aim or LookAt",
                    );
                };
                let ok = match action {
                    "add" => {
                        if constraints_json(w, id)
                            .iter()
                            .all(|c| c["kind"] != kind.type_name())
                            && let Err(err) = w.add_component_by_name(id, kind.type_name()) {
                                return ToolResult::error(err);
                            }
                        true
                    }
                    "add_source" => {
                        let Some(src) = args.get("source").and_then(|s| resolve_object(w, s))
                        else {
                            return ToolResult::error("`source` must name a GameObject");
                        };
                        let weight =
                            args.get("weight").and_then(Json::as_f64).unwrap_or(1.0) as f32;
                        add_source(w, id, kind, ConstraintSource::new(src, weight))
                    }
                    "activate" => constraints::activate(w, id, kind),
                    "zero" => constraints::zero(w, id, kind),
                    a => return ToolResult::error(format!("unknown action `{a}`")),
                };
                if !ok {
                    return ToolResult::error(format!("that object has no {}", kind.type_name()));
                }
                constraints::evaluate_constraints(w);
            }
            let t = w.transform(id).cloned();
            ToolResult::json(json!({
                "object": name(w, Some(id)),
                "constraints": constraints_json(w, id),
                "position": t.as_ref().map(|_| w.position(id).to_array()),
                "euler_angles": t.as_ref().map(|_| w.euler_angles(id).to_array()),
            }))
        },
    );
}
