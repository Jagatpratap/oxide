//! MCP tool for the Profiler and Frame Debugger.

use crate::bus::Bus;
use crate::mcp_tools::{schema, tool};
use oxide_core::profiler::Profiler;
use oxide_mcp::{McpServer, ToolDef, ToolResult, json};
use serde_json::Value as Json;
use std::sync::Arc;

pub fn register(s: &mut McpServer, bus: &Arc<Bus>) {
    tool(
        s,
        bus,
        ToolDef::new(
            "profiler",
            "Profiler",
            "Unity's Profiler: `record` true/false starts/stops recording (`clear` empties it). \
             Returns the recorded frame count, average and worst CPU / GPU ms over the last `frames` \
             (default 60), and for the newest frame with data: the CPU Hierarchy (top `rows`, \
             default 25: name, total ms, self ms, calls), GPU time per render pass and every module \
             counter. `frame_debugger`: true/false turns the Frame Debugger on/off; `step` n draws \
             the frame up to its nth object draw (null = all); its events are returned while on.",
            schema(
                json!({
                    "record": { "type": "boolean" }, "clear": { "type": "boolean" },
                    "frames": { "type": "integer" }, "rows": { "type": "integer" },
                    "frame_debugger": { "type": "boolean" }, "step": {}
                }),
                &[],
            ),
        ),
        |args, e| {
            if let Some(r) = args.get("record").and_then(Json::as_bool) {
                Profiler::set_enabled(r);
            }
            if args.get("clear").and_then(Json::as_bool) == Some(true) {
                Profiler::clear();
            }
            if let Some(on) = args.get("frame_debugger").and_then(Json::as_bool) {
                e.renderer.frame_debugger = on;
                if !on {
                    e.renderer.frame_debug_limit = None;
                }
            }
            if let Some(step) = args.get("step") {
                e.renderer.frame_debug_limit = step.as_u64().map(|n| n as usize);
            }
            let frames = Profiler::frames();
            let n = args.get("frames").and_then(Json::as_u64).unwrap_or(60) as usize;
            let recent: Vec<_> = frames.iter().rev().take(n).collect();
            let avg = |f: &dyn Fn(&oxide_core::profiler::FrameData) -> f64| {
                if recent.is_empty() {
                    0.0
                } else {
                    recent.iter().map(|x| f(x)).sum::<f64>() / recent.len() as f64
                }
            };
            let worst = |f: &dyn Fn(&oxide_core::profiler::FrameData) -> f64| {
                recent.iter().map(|x| f(x)).fold(0.0, f64::max)
            };
            let rows = args.get("rows").and_then(Json::as_u64).unwrap_or(25) as usize;
            let newest = frames.iter().rev().find(|f| !f.samples.is_empty());
            let gpu = frames.iter().rev().find(|f| !f.gpu.is_empty());
            let hierarchy: Vec<Json> = newest
                .map(|f| {
                    let mut h = f.hierarchy(0);
                    h.sort_by(|a, b| b.total.total_cmp(&a.total));
                    h.into_iter()
                        .take(rows)
                        .map(|r| json!({ "name": r.name, "depth": r.depth, "total_ms": r.total, "self_ms": r.self_time, "calls": r.calls }))
                        .collect()
                })
                .unwrap_or_default();
            let counters: Vec<Json> = newest
                .map(|f| {
                    f.counters
                        .iter()
                        .map(|c| json!({ "module": c.category, "name": c.name, "value": c.value }))
                        .collect()
                })
                .unwrap_or_default();
            let events: Vec<Json> = if e.renderer.frame_debugger {
                e.renderer
                    .frame_events()
                    .iter()
                    .map(|ev| json!({ "pass": ev.pass, "name": ev.name, "draw_calls": ev.draw_calls, "triangles": ev.triangles }))
                    .collect()
            } else {
                Vec::new()
            };
            ToolResult::json(json!({
                "recording": Profiler::enabled(),
                "frames": frames.len(),
                "cpu_ms": { "avg": avg(&|f| f.cpu_ms), "max": worst(&|f| f.cpu_ms) },
                "gpu_ms": { "avg": avg(&|f| f.gpu_ms()), "max": worst(&|f| f.gpu_ms()) },
                "hierarchy": hierarchy,
                "gpu_passes": gpu.map(|f| f.gpu.iter().map(|(n, t)| json!({ "pass": n, "ms": t })).collect::<Vec<_>>()).unwrap_or_default(),
                "counters": counters,
                "frame_debugger": { "enabled": e.renderer.frame_debugger, "step": e.renderer.frame_debug_limit, "events": events },
            }))
        },
    );
}
