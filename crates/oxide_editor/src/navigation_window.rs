//! Window → AI → Navigation (Unity's Navigation window): agent types, areas and their costs,
//! baking every NavMeshSurface, and the Scene view's NavMesh display options.
//! <https://docs.unity3d.com/Packages/com.unity.ai.navigation@2.0/manual/NavigationWindow.html>

use crate::{Action, EditorState};
use oxide_engine::Engine;
use oxide_engine::nav::{NavMesh, NavMeshSurface, NavigationSettings};
use oxide_ui::egui::{self, Color32, RichText};

#[derive(Default)]
pub struct NavigationWindow {
    tab: usize,
    agent: usize,
}

pub fn window_ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        for (i, name) in ["Agents", "Areas", "Bake"].iter().enumerate() {
            ui.selectable_value(&mut st.nav_window.tab, i, *name);
        }
    });
    ui.separator();
    let w = &mut engine.world;
    let mut settings = w
        .try_resource::<NavigationSettings>()
        .cloned()
        .unwrap_or_default();
    let before = settings.clone();
    match st.nav_window.tab {
        0 => agents(ui, &mut st.nav_window.agent, &mut settings),
        1 => areas(ui, &mut settings),
        _ => bake(st, engine, ui),
    }
    if settings != before {
        let w = &mut engine.world;
        w.resources.insert(settings);
        if let Err(e) = oxide_engine::project_settings::save(w) {
            oxide_core::debug_log_error!("Could not save navigation settings: {e}");
        }
    }
}

fn agents(ui: &mut egui::Ui, sel: &mut usize, s: &mut NavigationSettings) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_width(150.0);
            for (i, a) in s.agents.iter().enumerate() {
                ui.selectable_value(sel, i, &a.name);
            }
            ui.horizontal(|ui| {
                if ui
                    .small_button("+")
                    .on_hover_text("Add an agent type")
                    .clicked()
                {
                    s.add_agent("New Agent");
                    *sel = s.agents.len() - 1;
                }
                let can_remove =
                    s.agents.len() > 1 && s.agents.get(*sel).is_some_and(|a| a.id != 0);
                if ui
                    .add_enabled(can_remove, egui::Button::new("−").small())
                    .on_hover_text("Remove the agent type (Humanoid cannot be removed)")
                    .clicked()
                {
                    s.agents.remove(*sel);
                    *sel = sel.saturating_sub(1);
                }
            });
        });
        ui.separator();
        let Some(a) = s.agents.get_mut(*sel) else {
            return;
        };
        ui.vertical(|ui| {
            diagram(ui, a.radius, a.height, a.step_height, a.max_slope);
            egui::Grid::new("nav_agent")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Name");
                    ui.add(egui::TextEdit::singleline(&mut a.name).desired_width(160.0));
                    ui.end_row();
                    ui.label("Radius");
                    ui.add(
                        egui::DragValue::new(&mut a.radius)
                            .speed(0.01)
                            .range(0.05..=50.0),
                    );
                    ui.end_row();
                    ui.label("Height");
                    ui.add(
                        egui::DragValue::new(&mut a.height)
                            .speed(0.01)
                            .range(0.1..=100.0),
                    );
                    ui.end_row();
                    ui.label("Step Height");
                    ui.add(
                        egui::DragValue::new(&mut a.step_height)
                            .speed(0.01)
                            .range(0.0..=a.height.max(0.0)),
                    );
                    ui.end_row();
                    ui.label("Max Slope");
                    ui.add(egui::Slider::new(&mut a.max_slope, 0.0..=60.0).suffix("°"));
                    ui.end_row();
                    ui.label("Agent Type ID");
                    ui.weak(a.id.to_string());
                    ui.end_row();
                });
            ui.add_space(6.0);
            ui.weak("Rebake NavMeshSurfaces that use this agent type for changes to take effect.");
        });
    });
}

/// Side view of the agent cylinder and the max slope, like Unity's agent diagram.
fn diagram(ui: &mut egui::Ui, radius: f32, height: f32, step: f32, slope: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(260.0, 120.0), egui::Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 4.0, Color32::from_gray(38));
    let ground = rect.bottom() - 18.0;
    let scale = (90.0 / height.max(radius * 2.0).max(0.1)).min(60.0);
    let cx = rect.left() + 70.0;
    let (w, h) = (radius * scale, height * scale);
    let body = egui::Rect::from_min_max(egui::pos2(cx - w, ground - h), egui::pos2(cx + w, ground));
    p.rect_filled(body, 2.0, Color32::from_rgba_unmultiplied(0, 160, 230, 90));
    p.rect_stroke(
        body,
        2.0,
        egui::Stroke::new(1.0, Color32::from_rgb(0, 190, 255)),
        egui::StrokeKind::Inside,
    );
    let line = egui::Stroke::new(1.0, Color32::from_gray(140));
    p.line_segment(
        [
            egui::pos2(rect.left() + 6.0, ground),
            egui::pos2(rect.right() - 6.0, ground),
        ],
        line,
    );
    // Step.
    let sx = cx + w + 14.0;
    let sh = step * scale;
    p.line_segment([egui::pos2(sx, ground), egui::pos2(sx, ground - sh)], line);
    p.line_segment(
        [
            egui::pos2(sx, ground - sh),
            egui::pos2(sx + 24.0, ground - sh),
        ],
        line,
    );
    // Slope.
    let x0 = rect.right() - 110.0;
    let a = slope.to_radians();
    let len = 90.0;
    p.line_segment(
        [
            egui::pos2(x0, ground),
            egui::pos2(x0 + len * a.cos(), ground - len * a.sin()),
        ],
        egui::Stroke::new(1.5, Color32::from_rgb(230, 180, 60)),
    );
    let small = egui::FontId::proportional(10.0);
    p.text(
        egui::pos2(cx, ground - h - 2.0),
        egui::Align2::CENTER_BOTTOM,
        format!("H {height:.2}"),
        small.clone(),
        Color32::GRAY,
    );
    p.text(
        egui::pos2(cx, ground + 2.0),
        egui::Align2::CENTER_TOP,
        format!("R {radius:.2}"),
        small.clone(),
        Color32::GRAY,
    );
    p.text(
        egui::pos2(sx + 2.0, ground - sh - 2.0),
        egui::Align2::LEFT_BOTTOM,
        format!("{step:.2}"),
        small.clone(),
        Color32::GRAY,
    );
    p.text(
        egui::pos2(x0 + 8.0, ground + 2.0),
        egui::Align2::LEFT_TOP,
        format!("{slope:.0}°"),
        small,
        Color32::GRAY,
    );
}

fn areas(ui: &mut egui::Ui, s: &mut NavigationSettings) {
    s.areas.resize(32, Default::default());
    egui::ScrollArea::vertical().show(ui, |ui| {
        egui::Grid::new("nav_areas")
            .num_columns(4)
            .striped(true)
            .spacing([10.0, 3.0])
            .show(ui, |ui| {
                ui.label("");
                ui.strong("Name");
                ui.strong("Cost");
                ui.label("");
                ui.end_row();
                for (i, a) in s.areas.iter_mut().enumerate() {
                    let c = oxide_engine::navigation::area_color(i as u8);
                    let (r, _) =
                        ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                    ui.painter().rect_filled(
                        r,
                        2.0,
                        Color32::from_rgb(
                            (c.r * 255.0) as u8,
                            (c.g * 255.0) as u8,
                            (c.b * 255.0) as u8,
                        ),
                    );
                    let builtin = i < 3;
                    if builtin {
                        ui.label(format!("Built-in {i}  {}", a.name));
                    } else {
                        ui.add(
                            egui::TextEdit::singleline(&mut a.name)
                                .hint_text(format!("User {i}"))
                                .desired_width(160.0),
                        );
                    }
                    if i == 1 {
                        ui.weak("—");
                    } else {
                        ui.add(
                            egui::DragValue::new(&mut a.cost)
                                .speed(0.05)
                                .range(1.0..=10000.0),
                        );
                    }
                    ui.weak(i.to_string());
                    ui.end_row();
                }
            });
    });
}

fn bake(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    let w = &engine.world;
    let status = oxide_engine::nav::bake::status(w);
    let surfaces: Vec<_> = w
        .query::<NavMeshSurface>()
        .map(|(id, s)| {
            (
                id,
                w.get(id).map(|o| o.name.clone()).unwrap_or_default(),
                s.agent_type_id,
                s.navmesh_data.as_ref().map(|h| h.path().to_owned()),
            )
        })
        .collect();
    let settings = w
        .try_resource::<NavigationSettings>()
        .cloned()
        .unwrap_or_default();
    let nav = w.try_resource::<NavMesh>();
    ui.label(format!(
        "{} polygons in {} tiles, {} links",
        nav.map_or(0, |n| n.polygon_count()),
        nav.map_or(0, |n| n.tile_count()),
        nav.map_or(0, |n| n.links().count())
    ));
    ui.add_space(4.0);
    if surfaces.is_empty() {
        ui.weak("No NavMeshSurface in the scene. Add one with GameObject → AI → NavMesh Surface.");
    }
    egui::Grid::new("nav_surfaces")
        .num_columns(3)
        .striped(true)
        .show(ui, |ui| {
            for (id, name, agent, data) in &surfaces {
                if ui.link(name).clicked() {
                    st.select(*id);
                }
                ui.label(
                    settings
                        .agent(*agent)
                        .map_or("?".to_owned(), |a| a.name.clone()),
                );
                match data {
                    Some(p) => ui.weak(p.rsplit('/').next().unwrap_or(p)),
                    None => {
                        ui.label(RichText::new("Not baked").color(Color32::from_rgb(230, 180, 60)))
                    }
                };
                ui.end_row();
            }
        });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if status.running {
            ui.add(
                egui::ProgressBar::new(status.progress)
                    .desired_width(180.0)
                    .show_percentage(),
            );
            if ui.button("Cancel").clicked() {
                st.actions.push(Action::CancelNavMeshBake);
            }
        } else {
            if ui
                .add_enabled(!surfaces.is_empty(), egui::Button::new("Bake All"))
                .clicked()
            {
                st.actions.push(Action::BakeNavMesh(None));
            }
            if ui
                .add_enabled(!surfaces.is_empty(), egui::Button::new("Clear All"))
                .clicked()
            {
                for (id, ..) in &surfaces {
                    st.actions.push(Action::ClearNavMesh(*id));
                }
            }
        }
    });
    if let Some(e) = &status.error {
        ui.colored_label(Color32::from_rgb(230, 90, 80), e);
    } else if let Some(m) = &status.last {
        ui.weak(m);
    }
    ui.add_space(8.0);
    ui.separator();
    ui.strong("Scene View Display");
    let d = &mut st.nav_display;
    ui.checkbox(&mut d.surfaces, "Show NavMesh");
    ui.add_enabled(
        d.surfaces,
        egui::Checkbox::new(
            &mut d.only_selected,
            "Only when a navigation object is selected",
        ),
    );
    ui.checkbox(&mut d.agent_paths, "Show agent paths and velocities");
    ui.checkbox(&mut d.obstacles, "Show obstacles");
}
