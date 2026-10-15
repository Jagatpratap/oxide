//! MCP tools for prefabs: create, instantiate, overrides, Apply/Revert/Unpack, variants.

use crate::bus::Bus;
use crate::mcp_tools::{obj_arg, object_json, schema, tool, vec3_arg};
use crate::tri;
use oxide_core::reflect::Value;
use oxide_mcp::{McpServer, ToolDef, ToolResult, json};
use oxide_scene::{ObjectProperty, PrefabOverride};
use serde_json::Value as Json;
use std::sync::Arc;

fn path_arg(args: &crate::mcp_tools::Args, key: &str) -> Result<String, ToolResult> {
    let p = args
        .get(key)
        .and_then(Json::as_str)
        .ok_or_else(|| ToolResult::error(format!("`{key}` is required")))?;
    Ok(if p.to_ascii_lowercase().ends_with(".prefab") {
        p.to_owned()
    } else {
        format!("{p}.prefab")
    })
}

pub fn override_json(o: &PrefabOverride) -> Json {
    let v = |v: &Value| crate::json_values::value_to_json(None, v);
    match o {
        PrefabOverride::Property { object, property } => {
            let (name, value) = match property {
                ObjectProperty::Name(x) => ("name", json!(x)),
                ObjectProperty::Active(x) => ("active", json!(x)),
                ObjectProperty::Tag(x) => ("tag", json!(x)),
                ObjectProperty::Layer(x) => ("layer", json!(x)),
                ObjectProperty::Static(x) => ("static", json!(x)),
                ObjectProperty::Position(x) => ("position", json!(x)),
                ObjectProperty::Rotation(x) => ("rotation", json!(x)),
                ObjectProperty::Scale(x) => ("scale", json!(x)),
                ObjectProperty::Parent(x) => ("parent", json!(x)),
            };
            json!({ "kind": "property", "object": object, "property": name, "value": value })
        }
        PrefabOverride::Field {
            object,
            component,
            index,
            field,
            value,
        } => json!({
            "kind": "field", "object": object, "component": component, "index": index,
            "field": field, "value": v(value),
        }),
        PrefabOverride::Enabled {
            object,
            component,
            index,
            enabled,
        } => json!({
            "kind": "enabled", "object": object, "component": component, "index": index, "enabled": enabled,
        }),
        PrefabOverride::AddedComponent { object, component } => json!({
            "kind": "added_component", "object": object, "component": component.type_name,
        }),
        PrefabOverride::RemovedComponent {
            object,
            component,
            index,
        } => json!({
            "kind": "removed_component", "object": object, "component": component, "index": index,
        }),
        PrefabOverride::AddedObject { data } => {
            json!({ "kind": "added_object", "name": data.name })
        }
        PrefabOverride::RemovedObject { object } => {
            json!({ "kind": "removed_object", "object": object })
        }
    }
}

pub fn register(s: &mut McpServer, bus: &Arc<Bus>) {
    tool(
        s,
        bus,
        ToolDef::new(
            "create_prefab",
            "Create prefab",
            "Saves an object and its children as a prefab asset (e.g. `Assets/Prefabs/Car.prefab`) and makes the object \
         an instance of it, like dragging it into Unity's Project window.",
            schema(
                json!({ "object": {}, "path": { "type": "string" } }),
                &["object", "path"],
            ),
        ),
        |args, e| {
            let path = tri!(path_arg(args, "path"));
            let id = tri!(obj_arg(args, &e.world, "object"));
            match e.world.create_prefab(id, &path) {
                Ok(()) => ToolResult::text(format!("Created {path}")),
                Err(err) => ToolResult::error(err),
            }
        },
    );

    tool(
        s,
        bus,
        ToolDef::new(
            "instantiate_prefab",
            "Instantiate prefab",
            "Unity Instantiate(prefab): creates a linked instance of a prefab file, optionally under `parent` and at \
         `position`/`euler_angles` (world space).",
            schema(
                json!({ "path": { "type": "string" }, "parent": {}, "position": { "type": "array" },
                       "euler_angles": { "type": "array" } }),
                &["path"],
            ),
        ),
        |args, e| {
            let path = tri!(path_arg(args, "path"));
            let w = &mut e.world;
            let parent = match args.get("parent") {
                Some(_) => Some(tri!(obj_arg(args, w, "parent"))),
                None => None,
            };
            let id = match w.instantiate_prefab(&path, parent) {
                Ok(id) => id,
                Err(err) => return ToolResult::error(err),
            };
            if let Some(p) = tri!(vec3_arg(args, "position")) {
                w.set_position(id, p);
            }
            if let Some(r) = tri!(vec3_arg(args, "euler_angles")) {
                w.set_euler_angles(id, r);
            }
            ToolResult::json(object_json(w, id))
        },
    );

    tool(s, bus, ToolDef::new("get_prefab_overrides", "Get prefab overrides",
        "How a prefab instance differs from its prefab (changed fields and properties, added/removed components and \
         objects). Object ids in the result are ids inside the prefab. Placement of the root is not an override.",
        schema(json!({ "object": {} }), &["object"])).read_only(),
        |args, e| {
            let id = tri!(obj_arg(args, &e.world, "object"));
            let root = e.world.prefab_instance_root(id);
            match e.world.prefab_overrides(id) {
                Ok(ovs) => ToolResult::json(json!({
                    "prefab": root.and_then(|r| e.world.get(r)).and_then(|o| o.prefab_source.clone()),
                    "instance_root": root.map(|r| e.world.path_of(r)),
                    "overrides": ovs.iter().map(override_json).collect::<Vec<_>>(),
                })),
                Err(err) => ToolResult::error(err),
            }
        });

    tool(s, bus, ToolDef::new("apply_prefab", "Apply prefab overrides",
        "Unity Overrides > Apply All: writes the instance back into its prefab file; other instances in the scene \
         update and keep their own overrides.",
        schema(json!({ "object": {} }), &["object"])).destructive(),
        |args, e| {
            let id = tri!(obj_arg(args, &e.world, "object"));
            match e.world.apply_prefab_instance(id) {
                Ok(n) => ToolResult::text(format!("Applied; {n} other instance(s) updated")),
                Err(err) => ToolResult::error(err),
            }
        });

    tool(
        s,
        bus,
        ToolDef::new(
            "revert_prefab",
            "Revert prefab overrides",
            "Unity Overrides > Revert All: makes the instance match its prefab again (it stays where it is).",
            schema(json!({ "object": {} }), &["object"]),
        ),
        |args, e| {
            let id = tri!(obj_arg(args, &e.world, "object"));
            match e.world.revert_prefab_instance(id) {
                Ok(()) => ToolResult::text("Reverted"),
                Err(err) => ToolResult::error(err),
            }
        },
    );

    tool(
        s,
        bus,
        ToolDef::new(
            "unpack_prefab",
            "Unpack prefab",
            "Unity Unpack Completely: the instance becomes plain objects, no longer linked to the prefab.",
            schema(json!({ "object": {} }), &["object"]),
        ),
        |args, e| {
            let id = tri!(obj_arg(args, &e.world, "object"));
            let Some(root) = e.world.prefab_instance_root(id) else {
                return ToolResult::error("the object is not part of a prefab instance");
            };
            e.world.unpack_prefab(root);
            ToolResult::text(format!("Unpacked {}", e.world.path_of(root)))
        },
    );

    tool(
        s,
        bus,
        ToolDef::new(
            "create_prefab_variant",
            "Create prefab variant",
            "Saves a prefab instance as a variant of its prefab at `path`: a prefab that inherits from the base and \
         keeps the instance's overrides. Later changes to the base reach the variant.",
            schema(
                json!({ "object": {}, "path": { "type": "string" } }),
                &["object", "path"],
            ),
        ),
        |args, e| {
            let path = tri!(path_arg(args, "path"));
            let id = tri!(obj_arg(args, &e.world, "object"));
            let Some(root) = e.world.prefab_instance_root(id) else {
                return ToolResult::error("the object is not part of a prefab instance");
            };
            match e.world.create_prefab_variant(root, &path) {
                Ok(()) => ToolResult::text(format!("Created variant {path}")),
                Err(err) => ToolResult::error(err),
            }
        },
    );
}
