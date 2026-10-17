//! MCP tool for the AssetDatabase (GUIDs, FindAssets, dependencies).

use crate::bus::Bus;
use crate::mcp_tools::{schema, tool};
use oxide_assets::AssetServer;
use oxide_mcp::{McpServer, ToolDef, ToolResult, json};
use serde_json::Value as Json;
use std::sync::Arc;

pub fn register(s: &mut McpServer, bus: &Arc<Bus>) {
    tool(
        s,
        bus,
        ToolDef::new(
            "asset_database",
            "AssetDatabase",
            "Unity's AssetDatabase: every asset under Assets/ has a GUID in its .meta file. `action`: refresh \
             (scan for new, moved and deleted assets; returns what changed), guid (`path` → GUID), path (`guid` \
             → path), find (`filter` like FindAssets: name words and `t:Type`, optional `folders`), dependencies \
             (`path`, `recursive`), dependents (assets that refer to `path`), list (every path and GUID).",
            schema(
                json!({
                    "action": { "type": "string", "enum": ["refresh", "guid", "path", "find", "dependencies", "dependents", "list"] },
                    "path": { "type": "string" },
                    "guid": { "type": "string" },
                    "filter": { "type": "string" },
                    "folders": { "type": "array", "items": { "type": "string" } },
                    "recursive": { "type": "boolean" }
                }),
                &["action"],
            ),
        ),
        |args, e| {
            let assets = e.world.resource_mut::<AssetServer>();
            let action = args.get("action").and_then(Json::as_str).unwrap_or("");
            let path = args.get("path").and_then(Json::as_str);
            if action == "refresh" || !assets.database.is_scanned() {
                let r = assets.database.refresh();
                if action == "refresh" {
                    return ToolResult::json(json!({
                        "assets": assets.database.len(),
                        "imported": r.imported, "moved": r.moved, "deleted": r.deleted, "new_guids": r.new_guids,
                    }));
                }
            }
            let db = &assets.database;
            match action {
                "guid" => match path.and_then(|p| db.guid_of(p)) {
                    Some(g) => ToolResult::json(json!({ "path": path, "guid": g })),
                    None => ToolResult::error("no asset at that `path`"),
                },
                "path" => {
                    let g = args.get("guid").and_then(Json::as_str).unwrap_or("");
                    match db.path_of(g) {
                        Some(p) => ToolResult::json(json!({ "guid": g, "path": p })),
                        None => ToolResult::error("no asset has that `guid`"),
                    }
                }
                "find" => {
                    let filter = args.get("filter").and_then(Json::as_str).unwrap_or("");
                    let folders: Vec<String> = args
                        .get("folders")
                        .and_then(Json::as_array)
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_str().map(str::to_owned))
                                .collect()
                        })
                        .unwrap_or_default();
                    let types = &assets.types;
                    let found = db.find_assets(filter, &folders, &|p| {
                        types.for_path(p).map(|t| t.type_name.to_owned())
                    });
                    let out: Vec<Json> = found
                        .iter()
                        .map(|p| json!({ "path": p, "guid": db.guid_of(p) }))
                        .collect();
                    ToolResult::json(json!({ "assets": out }))
                }
                "dependencies" => {
                    let Some(p) = path else {
                        return ToolResult::error("`path` is required");
                    };
                    let recursive = args
                        .get("recursive")
                        .and_then(Json::as_bool)
                        .unwrap_or(true);
                    ToolResult::json(
                        json!({ "path": p, "dependencies": db.dependencies(p, recursive) }),
                    )
                }
                "dependents" => {
                    let Some(p) = path else {
                        return ToolResult::error("`path` is required");
                    };
                    ToolResult::json(json!({ "path": p, "dependents": db.dependents(p) }))
                }
                "list" => {
                    let all: Vec<Json> = db
                        .all()
                        .into_iter()
                        .map(|(p, g)| json!({ "path": p, "guid": g }))
                        .collect();
                    ToolResult::json(json!({ "assets": all }))
                }
                other => ToolResult::error(format!("unknown action `{other}`")),
            }
        },
    );
}
