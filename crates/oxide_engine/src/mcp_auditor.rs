//! MCP tool for the Project Auditor.

use crate::bus::Bus;
use crate::mcp_tools::{schema, tool};
use oxide_mcp::{McpServer, ToolDef, ToolResult, json};
use serde_json::Value as Json;
use std::sync::Arc;

pub fn register(s: &mut McpServer, bus: &Arc<Bus>) {
    tool(
        s,
        bus,
        ToolDef::new(
            "project_auditor",
            "Project Auditor",
            "Unity's Project Auditor: analyzes the project's scripts (per-frame scene searches, \
             allocations, logging, blocking, file I/O in update callbacks), assets (uncompressed or \
             oversized textures, missing mipmaps, normal maps imported as color, preloaded long \
             audio, uncompressed WAV, generated mesh colliders, assets no Build Settings scene \
             uses) and Project Settings (physics rate, collision matrix, mobile quality, Adaptive \
             Performance, Deferred + MSAA…). Returns counts per severity and the issues (id, \
             category, area, severity, title, path, line, recommendation), most severe first. \
             Filters: `category` (Code, Assets, ProjectSettings), `area` (CPU, GPU, Memory, \
             BuildSize, LoadTime, Quality), `min_severity` (Critical…Info), `limit` (default 100).",
            schema(
                json!({
                    "category": { "type": "string" }, "area": { "type": "string" },
                    "min_severity": { "type": "string" }, "limit": { "type": "integer" }
                }),
                &[],
            ),
        ),
        |args, e| {
            let r = crate::project_auditor::audit_project(&e.world);
            let want = |k: &str, v: &str| {
                args.get(k)
                    .and_then(Json::as_str)
                    .is_none_or(|w| w.eq_ignore_ascii_case(v))
            };
            let order = ["Critical", "Major", "Moderate", "Minor", "Info"];
            let min = args
                .get("min_severity")
                .and_then(Json::as_str)
                .and_then(|m| order.iter().position(|o| o.eq_ignore_ascii_case(m)))
                .unwrap_or(4);
            let limit = args.get("limit").and_then(Json::as_u64).unwrap_or(100) as usize;
            let issues: Vec<_> = r
                .issues
                .iter()
                .filter(|i| {
                    want("category", &format!("{:?}", i.category))
                        && want("area", &format!("{:?}", i.area))
                        && (i.severity as usize) <= min
                })
                .collect();
            use crate::project_auditor::Severity as S;
            ToolResult::json(json!({
                "scripts_scanned": r.scripts_scanned,
                "assets_scanned": r.assets_scanned,
                "seconds": r.seconds,
                "counts": {
                    "Critical": r.count(S::Critical), "Major": r.count(S::Major),
                    "Moderate": r.count(S::Moderate), "Minor": r.count(S::Minor), "Info": r.count(S::Info)
                },
                "matching": issues.len(),
                "issues": issues.into_iter().take(limit).collect::<Vec<_>>()
            }))
        },
    );
}
