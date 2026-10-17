//! Window → Analysis → Profiler (Unity's Profiler: CPU Usage with Timeline and Hierarchy
//! views, GPU Usage per render pass, and the Rendering, Physics, Audio, Memory, Scripts and
//! Video modules' counters over the recorded frames) and Window → Analysis → Frame Debugger
//! (the frame's draw events; stepping through them draws the frame up to that event).
//! References: <https://docs.unity3d.com/Manual/Profiler.html>,
//! <https://docs.unity3d.com/Manual/FrameDebugger.html>

use crate::EditorState;
use oxide_core::profiler::{FrameData, Profiler};
use oxide_engine::Engine;
use oxide_ui::egui::{self, Color32, RichText};
use std::sync::Arc;

const MODULES: [&str; 8] = [
    "CPU Usage",
    "GPU Usage",
    "Rendering",
    "Scripts",
    "Physics",
    "Audio",
    "Memory",
    "Video",
];

#[derive(Default)]
pub struct ProfilerWindow {
    module: usize,
    /// Selected frame (index into the history); None follows the newest.
    selected: Option<usize>,
    hierarchy: bool,
    // Frame Debugger.
    fd_selected: usize,
    fd_paused_play: bool,
}

/// Colours of the top-level CPU categories.
fn category_color(name: &str) -> Color32 {
    match name {
        "PlayerLoop" => Color32::from_rgb(110, 170, 255),
        "Rendering" => Color32::from_rgb(140, 220, 110),
        "Editor" => Color32::from_rgb(200, 160, 255),
        "UI" => Color32::from_rgb(255, 200, 90),
        "Input" => Color32::from_rgb(255, 130, 130),
        "Systems" => Color32::from_rgb(120, 220, 210),
        _ => Color32::from_rgb(170, 170, 170),
    }
}

fn sample_color(name: &str, depth: u16) -> Color32 {
    let h = name
        .bytes()
        .fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    let base = [
        Color32::from_rgb(110, 170, 255),
        Color32::from_rgb(140, 220, 110),
        Color32::from_rgb(255, 200, 90),
        Color32::from_rgb(200, 160, 255),
        Color32::from_rgb(120, 220, 210),
        Color32::from_rgb(255, 150, 150),
    ][(h as usize) % 6];
    base.gamma_multiply(1.0 - (depth as f32 * 0.08).min(0.5))
}

pub fn window_ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    let pw = &mut st.profiler;
    let frames = Profiler::frames();
    ui.horizontal(|ui| {
        let mut rec = Profiler::enabled();
        let label = if rec { "● Recording" } else { "○ Record" };
        if ui
            .toggle_value(&mut rec, label)
            .on_hover_text("Record profiling data (Unity's Record button)")
            .changed()
        {
            Profiler::set_enabled(rec);
        }
        if ui.button("Clear").clicked() {
            Profiler::clear();
            pw.selected = None;
        }
        ui.separator();
        let n = frames.len();
        let cur = pw.selected.unwrap_or(n.saturating_sub(1));
        if ui.small_button("◀").clicked() && n > 0 {
            pw.selected = Some(cur.saturating_sub(1));
        }
        if ui.small_button("▶").clicked() && n > 0 {
            pw.selected = Some((cur + 1).min(n - 1));
        }
        let mut follow = pw.selected.is_none();
        if ui.checkbox(&mut follow, "Current").changed() {
            pw.selected = if follow { None } else { Some(cur) };
        }
        if let Some(f) = frames.get(cur) {
            let gpu = if f.gpu.is_empty() {
                "—".to_owned()
            } else {
                format!("{:.2} ms", f.gpu_ms())
            };
            ui.label(format!(
                "Frame {}   CPU {:.2} ms   GPU {gpu}",
                f.index, f.cpu_ms
            ));
        }
    });
    if frames.is_empty() {
        ui.weak("Press Record to start profiling (Play mode or Edit mode).");
        return;
    }
    let sel = pw
        .selected
        .unwrap_or(frames.len() - 1)
        .min(frames.len() - 1);
    ui.separator();
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(120.0);
            for (i, m) in MODULES.iter().enumerate() {
                ui.selectable_value(&mut pw.module, i, *m);
            }
        });
        // (A vertical separator here would grow to the window's height.)
        ui.add_space(8.0);
        ui.vertical(|ui| {
            if let Some(f) = chart(ui, &frames, pw.module, sel) {
                pw.selected = Some(f);
            }
        });
    });
    ui.separator();
    let frame = frames[sel].clone();
    egui::ScrollArea::vertical()
        .id_salt("profiler-details")
        .show(ui, |ui| match pw.module {
            0 => {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut pw.hierarchy, false, "Timeline");
                    ui.selectable_value(&mut pw.hierarchy, true, "Hierarchy");
                });
                if pw.hierarchy {
                    hierarchy_ui(ui, &frame);
                } else {
                    timeline_ui(ui, &frame);
                }
            }
            1 => gpu_ui(ui, &frame),
            m => counters_ui(ui, &frame, MODULES[m]),
        });
    let _ = engine;
}

/// The module's chart over the recorded frames; returns a clicked frame.
fn chart(ui: &mut egui::Ui, frames: &[Arc<FrameData>], module: usize, sel: usize) -> Option<usize> {
    let size = egui::vec2(ui.available_width().max(200.0), 110.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 2.0, Color32::from_gray(28));
    let n = frames.len().max(1);
    let bw = rect.width() / n as f32;
    let category = MODULES[module];
    // Series values per frame.
    let series: Vec<(String, Vec<f64>)> = match module {
        0 => {
            // Top-level samples by category; the per-system `*.Always` ones count as Systems.
            let cat = |n: &str| {
                if n.ends_with(".Always") {
                    "Systems".to_owned()
                } else {
                    n.to_owned()
                }
            };
            let mut cats: Vec<String> = Vec::new();
            for f in frames {
                for s in f.samples.iter().filter(|s| s.depth == 0 && s.thread == 0) {
                    let c = cat(&s.name);
                    if !cats.contains(&c) {
                        cats.push(c);
                    }
                }
            }
            cats.into_iter()
                .map(|c| {
                    let v = frames
                        .iter()
                        .map(|f| {
                            f.samples
                                .iter()
                                .filter(|s| s.depth == 0 && s.thread == 0 && cat(&s.name) == c)
                                .map(|s| s.duration)
                                .fold(0.0, |a, b| a + b)
                        })
                        .collect();
                    (c, v)
                })
                .collect()
        }
        1 => vec![("GPU".into(), frames.iter().map(|f| f.gpu_ms()).collect())],
        _ => {
            let mut names: Vec<String> = Vec::new();
            for f in frames {
                for c in f.counters.iter().filter(|c| c.category == category) {
                    if !names.contains(&c.name.to_string()) {
                        names.push(c.name.to_string());
                    }
                }
            }
            names
                .into_iter()
                .map(|nm| {
                    let v = frames
                        .iter()
                        .map(|f| f.counter(category, &nm).unwrap_or(0.0))
                        .collect();
                    (nm, v)
                })
                .collect()
        }
    };
    if module <= 1 {
        // Stacked bars (ms), scaled to the slowest frame (at least 16.7 ms).
        let max = (0..frames.len())
            .map(|i| series.iter().map(|(_, v)| v[i]).sum::<f64>())
            .fold(16.7, f64::max) as f32;
        for i in 0..frames.len() {
            let mut y = rect.bottom();
            for (name, v) in &series {
                let h = v[i] as f32 / max * rect.height();
                let x0 = rect.left() + i as f32 * bw;
                let r = egui::Rect::from_min_max(
                    egui::pos2(x0, y - h),
                    egui::pos2(x0 + bw.max(1.0), y),
                );
                p.rect_filled(
                    r,
                    0.0,
                    if module == 0 {
                        category_color(name)
                    } else {
                        Color32::from_rgb(140, 220, 110)
                    },
                );
                y -= h;
            }
        }
        // 16 ms / 33 ms guides.
        for ms in [16.7f32, 33.3] {
            if ms < max {
                let y = rect.bottom() - ms / max * rect.height();
                p.hline(
                    rect.x_range(),
                    y,
                    egui::Stroke::new(1.0, Color32::from_gray(70)),
                );
                p.text(
                    egui::pos2(rect.left() + 2.0, y - 1.0),
                    egui::Align2::LEFT_BOTTOM,
                    format!("{ms:.0} ms"),
                    egui::FontId::proportional(10.0),
                    Color32::from_gray(140),
                );
            }
        }
    } else {
        // Lines, each scaled to its own maximum.
        for (k, (_, v)) in series.iter().enumerate() {
            let max = v.iter().copied().fold(1e-9, f64::max);
            let color = sample_color(&format!("{k}"), 0);
            let pts: Vec<egui::Pos2> = v
                .iter()
                .enumerate()
                .map(|(i, x)| {
                    egui::pos2(
                        rect.left() + (i as f32 + 0.5) * bw,
                        rect.bottom() - (*x / max) as f32 * (rect.height() - 4.0),
                    )
                })
                .collect();
            p.add(egui::Shape::line(pts, egui::Stroke::new(1.5, color)));
        }
    }
    // The selected frame.
    let x = rect.left() + (sel as f32 + 0.5) * bw;
    p.vline(x, rect.y_range(), egui::Stroke::new(1.0, Color32::WHITE));
    // Legend with the selected frame's values.
    ui.horizontal_wrapped(|ui| {
        for (k, (name, v)) in series.iter().enumerate() {
            let c = if module == 0 {
                category_color(name)
            } else if module == 1 {
                Color32::from_rgb(140, 220, 110)
            } else {
                sample_color(&format!("{k}"), 0)
            };
            ui.colored_label(c, "■");
            let val = v.get(sel).copied().unwrap_or(0.0);
            ui.label(if module <= 1 {
                format!("{name} {val:.2} ms")
            } else {
                format!("{name} {val:.1}")
            });
        }
    });
    let pointer = resp.interact_pointer_pos()?;
    (resp.clicked() || resp.dragged())
        .then(|| (((pointer.x - rect.left()) / bw) as usize).min(n - 1))
}

fn timeline_ui(ui: &mut egui::Ui, f: &FrameData) {
    let threads: Vec<u16> = {
        let mut t: Vec<u16> = f.samples.iter().map(|s| s.thread).collect();
        t.sort_unstable();
        t.dedup();
        t
    };
    let span = f
        .samples
        .iter()
        .map(|s| s.start + s.duration)
        .fold(f.cpu_ms.max(0.1), f64::max);
    let row = 18.0;
    for t in threads {
        let name = f
            .threads
            .get(t as usize)
            .cloned()
            .unwrap_or_else(|| format!("Thread {t}"));
        ui.label(RichText::new(name).strong());
        let depth = f
            .samples
            .iter()
            .filter(|s| s.thread == t)
            .map(|s| s.depth)
            .max()
            .unwrap_or(0);
        let size = egui::vec2(ui.available_width().max(200.0), (depth as f32 + 1.0) * row);
        let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::hover());
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 2.0, Color32::from_gray(24));
        let mut hovered = None;
        for s in f.samples.iter().filter(|s| s.thread == t) {
            let x0 = rect.left() + (s.start / span) as f32 * rect.width();
            let w = ((s.duration / span) as f32 * rect.width()).max(1.0);
            let y0 = rect.top() + s.depth as f32 * row;
            let r = egui::Rect::from_min_size(egui::pos2(x0, y0 + 1.0), egui::vec2(w, row - 2.0));
            p.rect_filled(r, 1.0, sample_color(&s.name, s.depth));
            if w > 30.0 {
                p.text(
                    r.left_center() + egui::vec2(3.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    format!("{} ({:.2} ms)", s.name, s.duration),
                    egui::FontId::proportional(11.0),
                    Color32::BLACK,
                );
            }
            if resp.hover_pos().is_some_and(|h| r.contains(h)) {
                hovered = Some(s.clone());
            }
        }
        if let Some(s) = hovered {
            resp.on_hover_text(format!(
                "{}\n{:.3} ms (at {:.2} ms)",
                s.name, s.duration, s.start
            ));
        }
    }
}

fn hierarchy_ui(ui: &mut egui::Ui, f: &FrameData) {
    let rows = f.hierarchy(0);
    let total = f.cpu_ms.max(1e-6);
    egui::Grid::new("profiler-hierarchy")
        .striped(true)
        .num_columns(5)
        .show(ui, |ui| {
            for h in ["Overview", "Total %", "Total ms", "Self ms", "Calls"] {
                ui.label(RichText::new(h).strong());
            }
            ui.end_row();
            // Parents first, children under them, biggest first.
            fn walk(
                rows: &[oxide_core::profiler::HierarchyRow],
                parent: Option<usize>,
                out: &mut Vec<usize>,
            ) {
                let mut kids: Vec<usize> = (0..rows.len())
                    .filter(|&i| rows[i].parent == parent)
                    .collect();
                kids.sort_by(|a, b| rows[*b].total.total_cmp(&rows[*a].total));
                for k in kids {
                    out.push(k);
                    walk(rows, Some(k), out);
                }
            }
            let mut order = Vec::new();
            walk(&rows, None, &mut order);
            for i in order {
                let r = &rows[i];
                ui.label(format!("{}{}", "    ".repeat(r.depth as usize), r.name));
                ui.label(format!("{:.1}%", r.total / total * 100.0));
                ui.label(format!("{:.3}", r.total));
                ui.label(format!("{:.3}", r.self_time));
                ui.label(r.calls.to_string());
                ui.end_row();
            }
        });
}

fn gpu_ui(ui: &mut egui::Ui, f: &FrameData) {
    if f.gpu.is_empty() {
        ui.weak("No GPU times for this frame (they arrive a few frames late, and need a GPU with timestamp queries).");
        return;
    }
    let total = f.gpu_ms().max(1e-6);
    egui::Grid::new("profiler-gpu")
        .striped(true)
        .num_columns(3)
        .show(ui, |ui| {
            for h in ["Render pass", "ms", "%"] {
                ui.label(RichText::new(h).strong());
            }
            ui.end_row();
            let mut passes = f.gpu.clone();
            passes.sort_by(|a, b| b.1.total_cmp(&a.1));
            for (name, ms) in passes {
                ui.label(name);
                ui.label(format!("{ms:.3}"));
                ui.label(format!("{:.1}%", ms / total * 100.0));
                ui.end_row();
            }
        });
}

fn counters_ui(ui: &mut egui::Ui, f: &FrameData, category: &str) {
    egui::Grid::new("profiler-counters")
        .striped(true)
        .num_columns(2)
        .show(ui, |ui| {
            let mut any = false;
            for c in f.counters.iter().filter(|c| c.category == category) {
                ui.label(c.name.as_ref());
                ui.label(if c.value.fract() == 0.0 {
                    format!("{}", c.value as i64)
                } else {
                    format!("{:.2}", c.value)
                });
                ui.end_row();
                any = true;
            }
            if !any {
                ui.weak("No data for this frame.");
            }
        });
}

// ---- Frame Debugger ----------------------------------------------------------------------

pub fn frame_debugger_ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    let pw = &mut st.profiler;
    let enabled = engine.renderer.frame_debugger;
    ui.horizontal(|ui| {
        let mut on = enabled;
        let label = if on { "Disable" } else { "Enable" };
        if ui.toggle_value(&mut on, label).clicked() {
            engine.renderer.frame_debugger = on;
            if on {
                // Unity pauses the game so the frame holds still.
                if engine.simulate && !engine.paused {
                    engine.paused = true;
                    pw.fd_paused_play = true;
                }
            } else {
                engine.renderer.frame_debug_limit = None;
                if std::mem::take(&mut pw.fd_paused_play) {
                    engine.paused = false;
                }
            }
        }
        ui.weak(
            "Steps through the frame's draws; the views show the frame up to the selected one.",
        );
    });
    if !enabled {
        return;
    }
    let events = engine.renderer.frame_events().to_vec();
    if events.is_empty() {
        ui.weak("No events yet.");
        return;
    }
    let n = events.len();
    pw.fd_selected = pw.fd_selected.min(n - 1);
    ui.horizontal(|ui| {
        ui.add(egui::Slider::new(&mut pw.fd_selected, 0..=n - 1).text(format!("of {n}")));
        if ui.small_button("◀").clicked() {
            pw.fd_selected = pw.fd_selected.saturating_sub(1);
        }
        if ui.small_button("▶").clicked() {
            pw.fd_selected = (pw.fd_selected + 1).min(n - 1);
        }
    });
    // Draw up to the selected event (only object draws can be cut).
    let limit = events[..=pw.fd_selected]
        .iter()
        .filter(|e| e.limitable)
        .count();
    let total = events.iter().filter(|e| e.limitable).count();
    engine.renderer.frame_debug_limit = (limit < total).then_some(limit);
    ui.separator();
    ui.columns(2, |cols| {
        egui::ScrollArea::vertical()
            .id_salt("fd-events")
            .show(&mut cols[0], |ui| {
                let mut last_pass = String::new();
                for (i, e) in events.iter().enumerate() {
                    if e.pass != last_pass {
                        ui.label(RichText::new(&e.pass).strong());
                        last_pass = e.pass.clone();
                    }
                    if ui
                        .selectable_label(pw.fd_selected == i, format!("  {}", e.name))
                        .clicked()
                    {
                        pw.fd_selected = i;
                    }
                }
            });
        let e = &events[pw.fd_selected];
        let ui = &mut cols[1];
        ui.label(RichText::new(&e.name).strong());
        ui.label(format!("Pass: {}", e.pass));
        ui.label(format!(
            "Draw calls: {}   Triangles: {}",
            e.draw_calls, e.triangles
        ));
        egui::Grid::new("fd-details")
            .striped(true)
            .num_columns(2)
            .show(ui, |ui| {
                for (k, v) in &e.details {
                    ui.label(k);
                    ui.label(v);
                    ui.end_row();
                }
            });
    });
}
