//! Window > Analysis > Project Auditor: runs the static checks and lists the issues with
//! filters by view (Code, Assets, Project Settings), area and severity. Clicking an issue
//! opens the script at its line or selects the asset.

use crate::{Action, EditorState};
use egui::{Color32, RichText};
use oxide_engine::Engine;
use oxide_engine::project_auditor::{self, Area, AuditReport, Category, Issue, Severity};

#[derive(Default)]
pub struct AuditorWindow {
    report: Option<AuditReport>,
    category: Option<Category>,
    area: Option<Area>,
    /// Least severe level shown.
    min_severity: Option<Severity>,
    search: String,
}

fn color(s: Severity) -> Color32 {
    match s {
        Severity::Critical => Color32::from_rgb(235, 80, 80),
        Severity::Major => Color32::from_rgb(235, 150, 70),
        Severity::Moderate => Color32::from_rgb(225, 200, 90),
        Severity::Minor => Color32::from_rgb(140, 180, 230),
        Severity::Info => Color32::from_gray(160),
    }
}

pub fn window_ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    let a = &mut st.auditor;
    ui.horizontal(|ui| {
        if ui.button("Analyze").clicked() {
            a.report = Some(project_auditor::audit_project(&engine.world));
        }
        if let Some(r) = &a.report {
            ui.label(format!(
                "{} scripts, {} assets in {:.2} s",
                r.scripts_scanned, r.assets_scanned, r.seconds
            ));
        }
    });
    let Some(report) = &a.report else {
        ui.add_space(8.0);
        ui.label(
            "Checks scripts, assets and Project Settings for common performance, memory and \
             build size problems. Nothing is changed. Press Analyze to start.",
        );
        return;
    };
    ui.horizontal_wrapped(|ui| {
        for s in [
            Severity::Critical,
            Severity::Major,
            Severity::Moderate,
            Severity::Minor,
            Severity::Info,
        ] {
            ui.label(RichText::new(format!("{s:?} {}", report.count(s))).color(color(s)));
        }
    });
    ui.horizontal_wrapped(|ui| {
        for (c, name) in [
            (None, "All"),
            (Some(Category::Code), "Code"),
            (Some(Category::Assets), "Assets"),
            (Some(Category::ProjectSettings), "Project Settings"),
        ] {
            let n = report
                .issues
                .iter()
                .filter(|i| c.is_none_or(|c| i.category == c))
                .count();
            ui.selectable_value(&mut a.category, c, format!("{name} ({n})"));
        }
        ui.separator();
        egui::ComboBox::from_id_salt("auditor-area")
            .selected_text(a.area.map_or("All areas".into(), |x| format!("{x:?}")))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut a.area, None, "All areas");
                for x in [
                    Area::CPU,
                    Area::GPU,
                    Area::Memory,
                    Area::BuildSize,
                    Area::LoadTime,
                    Area::Quality,
                ] {
                    ui.selectable_value(&mut a.area, Some(x), format!("{x:?}"));
                }
            });
        egui::ComboBox::from_id_salt("auditor-severity")
            .selected_text(
                a.min_severity
                    .map_or("Any severity".into(), |x| format!("{x:?} and up")),
            )
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut a.min_severity, None, "Any severity");
                for x in [
                    Severity::Critical,
                    Severity::Major,
                    Severity::Moderate,
                    Severity::Minor,
                ] {
                    ui.selectable_value(&mut a.min_severity, Some(x), format!("{x:?} and up"));
                }
            });
        ui.add(
            egui::TextEdit::singleline(&mut a.search)
                .hint_text("Search")
                .desired_width(140.0),
        );
    });
    ui.separator();
    let search = a.search.to_lowercase();
    let shown: Vec<&Issue> = report
        .issues
        .iter()
        .filter(|i| a.category.is_none_or(|c| i.category == c))
        .filter(|i| a.area.is_none_or(|x| i.area == x))
        .filter(|i| a.min_severity.is_none_or(|m| i.severity <= m))
        .filter(|i| {
            search.is_empty()
                || i.title.to_lowercase().contains(&search)
                || i.path.to_lowercase().contains(&search)
        })
        .collect();
    if shown.is_empty() {
        ui.label("No issues.");
    }
    let mut clicked = None;
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            for (n, i) in shown.iter().enumerate() {
                let location = match i.line {
                    Some(l) => format!("{}:{l}", i.path),
                    None => i.path.clone(),
                };
                egui::Frame::new()
                    .fill(if n % 2 == 0 {
                        Color32::from_gray(44)
                    } else {
                        Color32::from_gray(40)
                    })
                    .inner_margin(4)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                RichText::new(format!("{:?}", i.severity))
                                    .color(color(i.severity))
                                    .strong(),
                            );
                            ui.label(RichText::new(format!("{:?}", i.area)).weak());
                            ui.label(&i.title);
                        });
                        ui.horizontal_wrapped(|ui| {
                            if ui.link(&location).on_hover_text(i.id).clicked() {
                                clicked = Some((*i).clone());
                            }
                        });
                        ui.label(RichText::new(i.recommendation).weak().italics());
                    });
            }
        });
    if let Some(i) = clicked {
        match i.category {
            Category::Assets => st.actions.push(Action::PingAsset(i.path)),
            Category::Code => {
                let root = engine
                    .world
                    .resource::<oxide_assets::AssetServer>()
                    .full_path("");
                if let Err(e) = st.prefs.open_file(&root.join(&i.path), i.line.unwrap_or(1)) {
                    st.status = e;
                }
            }
            Category::ProjectSettings => st
                .actions
                .push(Action::OpenTab(crate::Tab::ProjectSettings)),
        }
    }
}
