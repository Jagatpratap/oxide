//! MCP tool for Adaptive Performance.

use crate::adaptive_performance::{
    AdaptivePerformance, AdaptivePerformanceSettings, PerformanceBottleneck, ScalerKind,
    ThermalMetrics, WarningLevel,
};
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
            "adaptive_performance",
            "Adaptive Performance",
            "Unity Adaptive Performance (runs in Play mode and players). Returns whether it is \
             active, thermal metrics, the performance bottleneck, frame timing (ms), target frame \
             rate, the indexer's action, each scaler's level/max/scale/override and the quality \
             the scalers produce (render scale, LOD bias, shadow distance…). `override`: \
             {scaler: level|null} pins scaler levels (names like `resolution`, `lod`, \
             `shadow_distance`, `shadow_quality`, `msaa`, `lights`, `texture_quality`, \
             `framerate`). `simulate_thermal`: {warning_level: NoWarning|ThrottlingImminent|\
             Throttling, temperature_level: 0..1} or null; `simulate_bottleneck`: CPU|GPU|\
             TargetFrameRate or null. `reset`: true puts every scaler back to level 0. Settings \
             are the `adaptive_performance` section of the project_settings tool.",
            schema(
                json!({
                    "override": { "type": "object" },
                    "simulate_thermal": {},
                    "simulate_bottleneck": {},
                    "reset": { "type": "boolean" }
                }),
                &[],
            ),
        ),
        |args, e| {
            let w = &mut e.world;
            let settings = w
                .try_resource::<AdaptivePerformanceSettings>()
                .cloned()
                .unwrap_or_default();
            if w.try_resource::<AdaptivePerformance>().is_none() {
                w.resources.insert(AdaptivePerformance::default());
            }
            let ap = w.resource_mut::<AdaptivePerformance>();
            if args.get("reset").and_then(Json::as_bool) == Some(true) {
                ap.reset();
                for k in ScalerKind::ALL {
                    ap.override_level(k, None);
                }
            }
            if let Some(o) = args.get("override").and_then(Json::as_object) {
                for (name, v) in o {
                    let Some(k) = ScalerKind::from_name(name) else {
                        return ToolResult::error(format!("unknown scaler `{name}`"));
                    };
                    ap.override_level(k, v.as_u64().map(|l| l as u32));
                }
            }
            if let Some(v) = args.get("simulate_thermal") {
                ap.simulation.thermal = if v.is_null() {
                    None
                } else {
                    let level = match v.get("warning_level").and_then(Json::as_str) {
                        Some("ThrottlingImminent") => WarningLevel::ThrottlingImminent,
                        Some("Throttling") => WarningLevel::Throttling,
                        _ => WarningLevel::NoWarning,
                    };
                    Some(ThermalMetrics {
                        warning_level: level,
                        temperature_level: v
                            .get("temperature_level")
                            .and_then(Json::as_f64)
                            .unwrap_or(0.5) as f32,
                        temperature_trend: 0.0,
                    })
                };
            }
            if let Some(v) = args.get("simulate_bottleneck") {
                ap.simulation.bottleneck = match v.as_str() {
                    Some("CPU") => Some(PerformanceBottleneck::CPU),
                    Some("GPU") => Some(PerformanceBottleneck::GPU),
                    Some("TargetFrameRate") => Some(PerformanceBottleneck::TargetFrameRate),
                    None if v.is_null() => None,
                    _ => {
                        return ToolResult::error(
                            "simulate_bottleneck: CPU, GPU, TargetFrameRate or null",
                        );
                    }
                };
            }
            let ap = w.resource::<AdaptivePerformance>().clone();
            let t = ap.frame_timing();
            let th = ap.thermal_metrics();
            let scalers: Vec<Json> = ScalerKind::ALL
                .iter()
                .map(|&k| {
                    let c = settings.scaler(k);
                    json!({
                        "scaler": k.name(), "enabled": c.enabled, "level": ap.level(k),
                        "max_level": c.max_level, "scale": c.scale(ap.level(k)),
                        "override": ap.overridden(k), "visual_impact": format!("{:?}", c.visual_impact)
                    })
                })
                .collect();
            let q = oxide_scene::QualitySettings::of(w);
            ToolResult::json(json!({
                "enabled": settings.enabled,
                "active": ap.active(),
                "thermal": { "warning_level": format!("{:?}", th.warning_level), "temperature_level": th.temperature_level, "temperature_trend": th.temperature_trend },
                "bottleneck": format!("{:?}", ap.performance_bottleneck()),
                "target_frame_rate": ap.target_frame_rate(),
                "frame_ms": t.average_frame_time * 1000.0,
                "cpu_ms": t.average_cpu_frame_time * 1000.0,
                "gpu_ms": t.average_gpu_frame_time * 1000.0,
                "action": format!("{:?}", ap.action()),
                "scalers": scalers,
                "effective_quality": {
                    "level": q.name, "render_scale": q.render_scale, "lod_bias": q.lod_bias,
                    "shadow_distance": q.shadow_distance, "shadows": format!("{:?}", q.shadows),
                    "msaa": format!("{:?}", q.msaa), "texture_quality": format!("{:?}", q.texture_quality),
                    "additional_lights_limit": q.additional_lights_limit,
                    "target_frame_rate": w.resource::<crate::AppControl>().target_frame_rate
                },
                "events": ap.events()
            }))
        },
    );
}
