//! MCP tools for multi-scene loading and world streaming.

use crate::bus::Bus;
use crate::mcp_tools::{schema, tool};
use crate::streaming::{SplitOptions, split_scene_into_cells};
use oxide_mcp::{McpServer, ToolDef, ToolResult, json};
use oxide_scene::{SceneHandle, World};
use oxide_standard::WorldStreamer;
use serde_json::Value as Json;
use std::sync::Arc;

/// A loaded scene by handle number, name or path.
fn scene_arg(world: &World, v: &Json) -> Result<SceneHandle, String> {
    match v {
        Json::Number(n) => {
            let h = SceneHandle(n.as_u64().unwrap_or(u64::MAX) as u32);
            world
                .scene_info(h)
                .map(|_| h)
                .ok_or_else(|| format!("no loaded scene with handle {n}"))
        }
        Json::String(s) => world
            .find_scene(s)
            .ok_or_else(|| format!("no loaded scene `{s}`")),
        _ => Err("scene: expected a handle number, name or path".into()),
    }
}

fn status(world: &World) -> Json {
    let mut counts = std::collections::HashMap::new();
    for (_, o) in world.objects() {
        *counts.entry(o.scene()).or_insert(0usize) += 1;
    }
    let scenes: Vec<Json> = world
        .loaded_scenes()
        .into_iter()
        .map(|s| {
            json!({
                "handle": s.handle.0,
                "name": s.name,
                "path": s.path,
                "objects": counts.get(&s.handle).copied().unwrap_or(0),
                "main": s.handle == SceneHandle::MAIN,
            })
        })
        .collect();
    let streamers: Vec<Json> = world
        .query::<WorldStreamer>()
        .map(|(id, ws)| {
            let states = ws.cell_states();
            let mut by_state = std::collections::BTreeMap::new();
            for s in &states {
                *by_state.entry(*s).or_insert(0usize) += 1;
            }
            json!({
                "object": id.to_bits(),
                "cells": ws.cells.len(),
                "states": by_state,
                "load_distance": ws.load_distance,
                "unload_distance": ws.unload_distance,
                "loaded": ws.cells.iter().zip(&states)
                    .filter(|(_, s)| **s == "loaded")
                    .map(|(c, _)| c.scene.clone())
                    .collect::<Vec<_>>(),
            })
        })
        .collect();
    let pending = world
        .try_resource::<oxide_scene::SceneOperations>()
        .map_or(0, |o| o.pending_count());
    json!({ "scenes": scenes, "pending_operations": pending, "streamers": streamers })
}

pub fn register(s: &mut McpServer, bus: &Arc<Bus>) {
    tool(
        s,
        bus,
        ToolDef::new(
            "scenes",
            "Loaded scenes",
            "Lists the loaded scenes (handle, name, path, object count; the main scene is the one saved by \
             save_scene), pending background operations and WorldStreamer cell states. Optional actions: \
             `load` = scene path (additive unless `additive: false`; `background: true` uses LoadSceneAsync and \
             returns before it finishes), `unload` = scene handle/name/path, `save` = scene handle/name/path \
             (with optional `path` to save to).",
            schema(
                json!({
                    "load": { "type": "string" },
                    "additive": { "type": "boolean" },
                    "background": { "type": "boolean" },
                    "unload": {},
                    "save": {},
                    "path": { "type": "string" }
                }),
                &[],
            ),
        ),
        |args, e| {
            if let Some(path) = args.get("load").and_then(Json::as_str) {
                let additive = args.get("additive").and_then(Json::as_bool).unwrap_or(true);
                if args.get("background").and_then(Json::as_bool) == Some(true) {
                    e.world.load_scene_async(path, additive);
                } else {
                    if let Err(err) = crate::load_scene(&mut e.world, path, additive) {
                        return ToolResult::error(err);
                    }
                    if !additive {
                        e.start_systems();
                    }
                }
            }
            if let Some(v) = args.get("unload") {
                let r = scene_arg(&e.world, v).and_then(|h| e.world.unload_scene(h));
                if let Err(err) = r {
                    return ToolResult::error(err);
                }
            }
            if let Some(v) = args.get("save") {
                let h = match scene_arg(&e.world, v) {
                    Ok(h) => h,
                    Err(err) => return ToolResult::error(err),
                };
                let Some(path) = args
                    .get("path")
                    .and_then(Json::as_str)
                    .map(str::to_owned)
                    .or_else(|| e.world.scene_info(h).and_then(|i| i.path))
                else {
                    return ToolResult::error(
                        "`path` is required for a scene that was never saved",
                    );
                };
                if let Err(err) = crate::save_scene_as(&mut e.world, h, &path) {
                    return ToolResult::error(err);
                }
            }
            ToolResult::json(status(&e.world))
        },
    );

    tool(
        s,
        bus,
        ToolDef::new(
            "split_scene_into_cells",
            "Split scene into streaming cells",
            "For big worlds: moves the main scene's root objects into one scene per `cell_size` m grid cell \
             (by position), saves the cells to `folder` (default Assets/Scenes/<scene>_Cells), unloads them, and \
             adds a WorldStreamer that loads cells near the camera in Play mode. Cameras, directional lights, \
             terrains and streamers stay. `only_static` (default true) moves only objects with static flags. \
             Save the main scene afterwards to keep the change.",
            schema(
                json!({
                    "cell_size": { "type": "number", "minimum": 1 },
                    "folder": { "type": "string" },
                    "only_static": { "type": "boolean" }
                }),
                &["cell_size"],
            ),
        )
        .destructive(),
        |args, e| {
            let folder = args
                .get("folder")
                .and_then(Json::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| format!("Assets/Scenes/{}_Cells", e.world.scene_name));
            let opts = SplitOptions {
                cell_size: args.get("cell_size").and_then(Json::as_f64).unwrap_or(200.0) as f32,
                folder,
                only_static: args.get("only_static").and_then(Json::as_bool).unwrap_or(true),
            };
            match split_scene_into_cells(&mut e.world, &opts) {
                Ok(report) => ToolResult::json(json!({ "split": report, "status": status(&e.world) })),
                Err(err) => ToolResult::error(err),
            }
        },
    );
}
