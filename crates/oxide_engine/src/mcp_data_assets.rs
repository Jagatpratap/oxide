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

    tool(
        s,
        bus,
        ToolDef::new(
            "input_actions",
            "Input System actions",
            "Live Input System action states (what scripts read with ctx.input().action(\"Move\")): value, \
             phase, pressed and this-frame flags for every action enabled by a PlayerInput component.",
            schema(json!({}), &[]),
        ),
        |_args, e| {
            let input = e.world.resource::<oxide_input::Input>();
            let mut states: Vec<(&String, &oxide_input::ActionState)> =
                input.action_states().iter().collect();
            states.sort_by(|a, b| a.0.cmp(b.0));
            let list: serde_json::Map<String, Json> = states
                .into_iter()
                .map(|(k, v)| (k.clone(), serde_json::to_value(v).unwrap_or(Json::Null)))
                .collect();
            ToolResult::json(json!({ "actions": list }))
        },
    );

    tool(
        s,
        bus,
        ToolDef::new(
            "tilemap",
            "Tilemap",
            "Edits a Tilemap (Unity Tilemap API): `set` [{x, y, tile}] sets cells (tile = a .tile/.ruletile/\
             .animatedtile path, or null to erase), `box_fill` {from: [x, y], to: [x, y], tile}, `flood_fill` \
             {x, y, tile}, `clear` true. Returns the painted cells.",
            schema(
                json!({ "object": {}, "set": { "type": "array" }, "box_fill": { "type": "object" },
                        "flood_fill": { "type": "object" }, "clear": { "type": "boolean" } }),
                &["object"],
            ),
        ),
        |args, e| {
            use oxide_scene::{TileBase, Tilemap};
            let Some(id) = args
                .get("object")
                .and_then(|o| crate::json_values::resolve_object(&e.world, o))
            else {
                return ToolResult::error("no such GameObject");
            };
            let tile = |v: Option<&Json>| -> Option<oxide_assets::Handle<TileBase>> {
                v.and_then(Json::as_str)
                    .map(|p| oxide_assets::Handle::from_path(p.to_owned()))
            };
            // Tiles must be loaded to draw.
            let mut paths: Vec<String> = Vec::new();
            for c in args
                .get("set")
                .and_then(Json::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(p) = c.get("tile").and_then(Json::as_str) {
                    paths.push(p.to_owned());
                }
            }
            for k in ["box_fill", "flood_fill"] {
                if let Some(p) = args
                    .get(k)
                    .and_then(|b| b.get("tile"))
                    .and_then(Json::as_str)
                {
                    paths.push(p.to_owned());
                }
            }
            for p in paths {
                if let Err(err) = e.world.resource_mut::<AssetServer>().ensure_loaded(&p) {
                    return ToolResult::error(err);
                }
            }
            let Some(m) = e.world.get_component_mut::<Tilemap>(id) else {
                return ToolResult::error("the object has no Tilemap");
            };
            if args.get("clear").and_then(Json::as_bool) == Some(true) {
                m.clear_all_tiles();
            }
            for c in args
                .get("set")
                .and_then(Json::as_array)
                .into_iter()
                .flatten()
            {
                let (Some(x), Some(y)) = (
                    c.get("x").and_then(Json::as_i64),
                    c.get("y").and_then(Json::as_i64),
                ) else {
                    continue;
                };
                m.set_tile(x as i32, y as i32, tile(c.get("tile")));
            }
            let pair = |v: Option<&Json>| -> Option<(i32, i32)> {
                let a = v?.as_array()?;
                Some((a.first()?.as_i64()? as i32, a.get(1)?.as_i64()? as i32))
            };
            if let Some(b) = args.get("box_fill")
                && let (Some(a), Some(z)) = (pair(b.get("from")), pair(b.get("to")))
            {
                m.box_fill(a, z, tile(b.get("tile")));
            }
            if let Some(f) = args.get("flood_fill")
                && let (Some(x), Some(y)) = (
                    f.get("x").and_then(Json::as_i64),
                    f.get("y").and_then(Json::as_i64),
                )
            {
                let c = (x as i32, y as i32);
                let b = m.cell_bounds().unwrap_or((c, c));
                let bounds = (
                    (b.0.0.min(c.0), b.0.1.min(c.1)),
                    (b.1.0.max(c.0), b.1.1.max(c.1)),
                );
                m.flood_fill(c.0, c.1, tile(f.get("tile")), bounds);
            }
            let cells: Vec<Json> = m
                .tiles
                .iter()
                .map(|c| json!({ "x": c.x, "y": c.y, "tile": c.tile.as_ref().map(|t| t.path().to_owned()) }))
                .collect();
            ToolResult::json(
                json!({ "count": cells.len(), "bounds": m.cell_bounds(), "cells": cells }),
            )
        },
    );
}
