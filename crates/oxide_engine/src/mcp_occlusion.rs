//! MCP tools for occlusion culling.

use crate::bus::Bus;
use crate::mcp_tools::{schema, tool, vec3_arg};
use crate::tri;
use oxide_math::{Quaternion, QuaternionExt, Vector3};
use oxide_mcp::{McpServer, ToolDef, ToolResult, json};
use oxide_render::{CameraView, CapturedImage, ClearMode, ProjectionKind, RenderStats};
use oxide_scene::StaticEditorFlags;
use serde_json::Value as Json;
use std::sync::Arc;

fn stats_json(s: &RenderStats) -> Json {
    json!({
        "draw_calls": s.draw_calls,
        "triangles": s.triangles,
        "frustum_culled": s.culled,
        "occlusion_culled": s.occlusion_culled,
        "occluders": s.occluders,
        "occluder_triangles": s.occluder_triangles,
        "occlusion_ms": (s.occlusion_ms * 1000.0).round() / 1000.0,
    })
}

/// Fraction of pixels whose color differs by more than a few levels.
fn difference(a: &CapturedImage, b: &CapturedImage) -> f64 {
    let n = a.rgba.len().min(b.rgba.len()) / 4;
    let differ = (0..n)
        .filter(|&i| (0..3).any(|c| a.rgba[i * 4 + c].abs_diff(b.rgba[i * 4 + c]) > 8))
        .count();
    differ as f64 / n.max(1) as f64
}

pub fn register(s: &mut McpServer, bus: &Arc<Bus>) {
    tool(
        s,
        bus,
        ToolDef::new(
            "occlusion",
            "Occlusion culling",
            "Occlusion culling stats. Occluders are renderers on Occluder Static objects, terrain and closed Occlusion \
             Portals; renderers on Occludee Static objects or with `allow_occlusion_when_dynamic` can be hidden. \
             Without a camera: the last rendered frame's stats. With `camera_position` (+ `look_at`, `field_of_view`): \
             renders that view with occlusion on and off and reports both, plus `pixel_difference` (should be ~0: \
             occlusion must never hide anything visible). `image` = buffer returns the occlusion depth buffer \
             (brighter = nearer), `render` the occlusion-on render, `difference` that render with differing pixels in magenta.",
            schema(
                json!({
                    "camera_position": { "type": "array", "items": { "type": "number" } },
                    "look_at": { "type": "array", "items": { "type": "number" } },
                    "field_of_view": { "type": "number" },
                    "image": { "type": "string", "enum": ["none", "buffer", "render", "difference"] }
                }),
                &[],
            ),
        )
        .read_only(),
        |args, e| {
            let image = args.get("image").and_then(Json::as_str).unwrap_or("none");
            let mut out = json!({
                "occluder_static_objects": e.world.objects()
                    .filter(|(_, o)| o.static_flags.contains(StaticEditorFlags::OCCLUDER_STATIC))
                    .count(),
            });
            let mut render_png = None;
            if let Some(p) = tri!(vec3_arg(args, "camera_position")) {
                let t = tri!(vec3_arg(args, "look_at")).unwrap_or(p + Vector3::Z);
                let fov = args.get("field_of_view").and_then(Json::as_f64).unwrap_or(60.0) as f32;
                let view = |occlusion_culling| CameraView {
                    position: p,
                    rotation: Quaternion::look_rotation((t - p).normalize_or(Vector3::Z), Vector3::Y),
                    projection: ProjectionKind::Perspective { fov },
                    near: 0.1,
                    far: 5000.0,
                    viewport: [0.0, 0.0, 1.0, 1.0],
                    clear: ClearMode::Sky,
                    culling_mask: u32::MAX,
                    occlusion_culling,
                };
                let off = match e.capture(640, 360, Some(vec![view(false)])) {
                    Ok(i) => i,
                    Err(err) => return ToolResult::error(err),
                };
                let off_stats = e.renderer.last_stats;
                let on = match e.capture(640, 360, Some(vec![view(true)])) {
                    Ok(i) => i,
                    Err(err) => return ToolResult::error(err),
                };
                out["occlusion_on"] = stats_json(&e.renderer.last_stats);
                out["occlusion_off"] = stats_json(&off_stats);
                out["pixel_difference"] = json!(difference(&on, &off));
                if image == "render" {
                    render_png = Some(on.to_png());
                } else if image == "difference" {
                    let mut marked = CapturedImage { width: on.width, height: on.height, rgba: on.rgba.clone() };
                    for (i, px) in marked.rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                        if (0..3).any(|c| px[c].abs_diff(off.rgba[i * 4 + c]) > 8) {
                            px.copy_from_slice(&[255, 0, 255, 255]);
                        }
                    }
                    render_png = Some(marked.to_png());
                }
            } else {
                out["last_frame"] = stats_json(&e.renderer.last_stats);
            }
            let png = match image {
                "render" | "difference" => render_png,
                "buffer" => e.renderer.occlusion_buffer(0).map(|b| {
                    CapturedImage {
                        width: b.width(),
                        height: b.height(),
                        rgba: b.to_grayscale().into_iter().flat_map(|g| [g, g, g, 255]).collect(),
                    }
                    .to_png()
                }),
                _ => None,
            };
            let text = serde_json::to_string_pretty(&out).unwrap_or_default();
            match png {
                Some(Ok(png)) => ToolResult::png(png).with_text(text),
                Some(Err(err)) => ToolResult::error(err),
                None if image == "buffer" => {
                    ToolResult::error("no occlusion buffer: the last camera had occlusion culling off")
                }
                None => ToolResult::json(out),
            }
        },
    );
}
