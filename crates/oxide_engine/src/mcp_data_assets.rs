//! MCP tools for asset types and data assets (Unity ScriptableObjects).

use crate::bus::Bus;
use crate::json_values::{
    load_referenced_assets, parse_fields, reflect_to_json, schema_to_json, set_fields,
};
use crate::mcp_tools::{schema, tool};
use oxide_assets::{AssetServer, CreateKind};
use oxide_mcp::{McpServer, ToolDef, ToolResult, json};
use serde_json::Value as Json;
use std::sync::Arc;

pub fn register(s: &mut McpServer, bus: &Arc<Bus>) {
    tool(
        s,
        bus,
        ToolDef::new(
            "asset_types",
            "Asset types",
            "Every asset type the project knows (the asset type registry): type name, file extensions, \
             Create menu path, whether it is a data asset (ScriptableObject, edited with `data_asset`) and, \
             for data assets, the fields of a new one.",
            schema(json!({ "type": { "type": "string" } }), &[]),
        ),
        |args, e| {
            let assets = e.world.resource::<AssetServer>();
            let only = args.get("type").and_then(Json::as_str);
            let list: Vec<Json> = assets
                .types
                .iter()
                .filter(|t| only.is_none_or(|o| o.eq_ignore_ascii_case(t.type_name)))
                .map(|t| {
                    let data = assets.types.is_data(t.type_name);
                    let mut j = json!({
                        "type": t.type_name,
                        "extensions": t.extensions,
                        "create_menu": t.create_menu,
                        "creatable": t.create != CreateKind::None && t.create_menu.is_some(),
                        "data_asset": data,
                    });
                    if data
                        && only.is_some()
                        && let Some(v) = assets.types.create_data(t.type_name)
                    {
                        j["schema"] = schema_to_json(&v.as_reflect().type_info());
                        j["defaults"] = reflect_to_json(Some(&e.world), v.as_reflect());
                    }
                    j
                })
                .collect();
            ToolResult::json(json!({ "types": list }))
        },
    );

    tool(
        s,
        bus,
        ToolDef::new(
            "data_asset",
            "Data asset",
            "Creates, reads or changes a data asset file (Unity ScriptableObject: Physics Material, Volume \
             Profile, script-defined data). `action`: get (default) | create (needs `type`; `path` like \
             Assets/Data/Ice.physicMaterial) | set. `fields` sets values on create/set; the file is saved.",
            schema(
                json!({
                    "action": { "type": "string", "enum": ["get", "create", "set"] },
                    "path": { "type": "string" },
                    "type": { "type": "string" },
                    "fields": { "type": "object" }
                }),
                &["path"],
            ),
        ),
        |args, e| {
            let path = args
                .get("path")
                .and_then(Json::as_str)
                .unwrap_or("")
                .to_owned();
            let action = args.get("action").and_then(Json::as_str).unwrap_or("get");
            let assets = e.world.resource_mut::<AssetServer>();
            match action {
                "create" => {
                    let Some(t) = args.get("type").and_then(Json::as_str) else {
                        return ToolResult::error("create needs `type` (see asset_types)");
                    };
                    let name = assets
                        .types
                        .iter()
                        .find(|x| x.type_name.eq_ignore_ascii_case(t))
                        .map(|x| x.type_name);
                    let Some(name) = name.filter(|n| assets.types.is_data(n)) else {
                        return ToolResult::error(format!("`{t}` is not a data asset type"));
                    };
                    if let Err(err) = assets.create_data_asset(name, &path) {
                        return ToolResult::error(err);
                    }
                }
                "get" | "set" => {
                    if !assets.data.contains(&path)
                        && let Err(err) = assets.load_data(&path)
                    {
                        return ToolResult::error(err);
                    }
                }
                other => return ToolResult::error(format!("unknown action `{other}`")),
            }
            let mut errors = Vec::new();
            if let Some(fields) = args.get("fields").and_then(Json::as_object) {
                let Some(mut value) = e
                    .world
                    .resource::<AssetServer>()
                    .data
                    .get_dyn(&path)
                    .map(|v| v.clone_box())
                else {
                    return ToolResult::error(format!("{path} is not loaded"));
                };
                let info = value.as_reflect().type_info();
                let (parsed, errs) =
                    parse_fields(&info, fields, &e.world, Some(value.as_reflect()));
                errors.extend(errs);
                load_referenced_assets(&mut e.world, &parsed);
                set_fields(value.as_reflect_mut(), parsed, &mut errors);
                if let Err(err) = e
                    .world
                    .resource_mut::<AssetServer>()
                    .save_data(&path, value)
                {
                    return ToolResult::error(err);
                }
            }
            let assets = e.world.resource::<AssetServer>();
            let Some(v) = assets.data.get_dyn(&path) else {
                return ToolResult::error(format!("{path} is not loaded"));
            };
            ToolResult::json(json!({
                "path": path,
                "type": assets.data.type_of(&path),
                "fields": reflect_to_json(Some(&e.world), v.as_reflect()),
                "errors": errors,
            }))
        },
    );
}
