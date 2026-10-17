//! Window → Package Manager: the project's packages (manifest dependencies, embedded
//! folders and what they depend on), their details and samples, and the + menu (Add package
//! from disk, from git URL, by name). Resolution (git clones, registry downloads) runs on a
//! background thread.
//! Reference: <https://docs.unity3d.com/Manual/upm-ui.html>

use crate::EditorState;
use oxide_assets::AssetServer;
use oxide_assets::packages::{self, AddRequest, NetFetcher, PackageInfo, PackageSource};
use oxide_engine::Engine;
use oxide_ui::egui::{self, RichText};
use std::path::PathBuf;
use std::sync::mpsc;

/// What a background job hands back: the resolved packages, problems, and a message.
type Outcome = (Vec<PackageInfo>, Vec<String>, Result<String, String>);

#[derive(Default)]
pub struct PackageManager {
    packages: Vec<PackageInfo>,
    errors: Vec<String>,
    selected: Option<String>,
    search: String,
    /// The + menu's text field (git URL, or `name[@version]`) and which one is open.
    adding: Option<(bool, String)>,
    job: Option<mpsc::Receiver<Outcome>>,
    loaded: bool,
    message: Option<(bool, String)>,
}

enum Job {
    Refresh,
    Add(AddRequest),
    Remove(String),
}

fn start(pm: &mut PackageManager, project: PathBuf, job: Job) {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let msg = match &job {
            Job::Refresh => Ok(String::new()),
            Job::Add(req) => {
                packages::add(&project, req, &mut NetFetcher).map(|n| format!("Added {n}"))
            }
            Job::Remove(n) => packages::remove(&project, n).map(|_| format!("Removed {n}")),
        };
        let (list, errors) = packages::resolve(&project, &mut NetFetcher);
        let msg = match (msg, packages::sync_script_dependencies(&project, &list)) {
            (Ok(m), Ok(true)) => Ok(format!(
                "{m}{}Scripts crate dependencies updated",
                if m.is_empty() { "" } else { " · " }
            )),
            (Ok(m), Err(e)) => Err(format!("{m} · Scripts/Cargo.toml: {e}")),
            (m, _) => m,
        };
        let _ = tx.send((list, errors, msg));
    });
    pm.job = Some(rx);
}

pub fn window_ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    let pm = &mut st.package_manager;
    let project = engine.world.resource::<AssetServer>().root.clone();
    if !pm.loaded {
        pm.loaded = true;
        start(pm, project.clone(), Job::Refresh);
    }
    if let Some(rx) = &pm.job {
        match rx.try_recv() {
            Ok((list, errors, msg)) => {
                engine.world.resource_mut::<AssetServer>().package_roots = packages::roots(&list);
                pm.packages = list;
                pm.errors = errors;
                pm.message = match msg {
                    Ok(m) if m.is_empty() => None,
                    Ok(m) => Some((true, m)),
                    Err(e) => Some((false, e)),
                };
                pm.job = None;
            }
            Err(mpsc::TryRecvError::Empty) => ui.ctx().request_repaint(),
            Err(mpsc::TryRecvError::Disconnected) => pm.job = None,
        }
    }
    let busy = pm.job.is_some();

    // Toolbar: + menu, refresh, search.
    let mut job = None;
    ui.horizontal(|ui| {
        ui.add_enabled_ui(!busy, |ui| {
            ui.menu_button("+", |ui| {
                if ui.button("Install package from disk…").clicked() {
                    ui.close();
                    if let Some(dir) =
                        crate::hub::pick_folder("Select a package folder", Some(&project))
                    {
                        job = Some(Job::Add(AddRequest::Disk(dir)));
                    }
                }
                if ui.button("Install package from git URL…").clicked() {
                    ui.close();
                    pm.adding = Some((true, String::new()));
                }
                if ui.button("Install package by name…").clicked() {
                    ui.close();
                    pm.adding = Some((false, String::new()));
                }
            });
            if ui.button("⟳ Refresh").clicked() {
                job = Some(Job::Refresh);
            }
        });
        ui.label("Packages: In Project");
        if busy {
            ui.spinner();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut pm.search)
                    .hint_text("Search")
                    .desired_width(160.0),
            );
        });
    });
    if let Some((git, text)) = &mut pm.adding {
        let mut close = false;
        ui.horizontal(|ui| {
            ui.label(if *git { "Git URL" } else { "Name" });
            let r = ui.add(
                egui::TextEdit::singleline(text)
                    .hint_text(if *git {
                        "https://host/repo.git#revision"
                    } else {
                        "com.company.package@1.0.0"
                    })
                    .desired_width(320.0),
            );
            let go = ui
                .add_enabled(
                    !text.trim().is_empty() && !busy,
                    egui::Button::new("Install"),
                )
                .clicked()
                || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            if go && !text.trim().is_empty() {
                let t = text.trim().to_owned();
                job = Some(Job::Add(if *git {
                    AddRequest::Git(t)
                } else {
                    let (name, version) = match t.split_once('@') {
                        Some((n, v)) => (n.to_owned(), Some(v.to_owned())),
                        None => (t, None),
                    };
                    AddRequest::Name { name, version }
                }));
                close = true;
            }
            if ui.button("Cancel").clicked() {
                close = true;
            }
        });
        if close {
            pm.adding = None;
        }
    }
    if let Some((ok, m)) = &pm.message {
        ui.colored_label(
            if *ok {
                ui.visuals().text_color()
            } else {
                ui.visuals().error_fg_color
            },
            m,
        );
    }
    for e in &pm.errors {
        ui.colored_label(ui.visuals().warn_fg_color, format!("⚠ {e}"));
    }
    ui.separator();

    let q = pm.search.to_lowercase();
    egui::Panel::left(egui::Id::new("package_list"))
        .resizable(true)
        .default_size(260.0)
        .min_size(200.0)
        .show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                if pm.packages.is_empty() && !busy {
                    ui.weak("No packages. Use + to install one, or put a folder with a package.json in Packages/.");
                }
                for p in &pm.packages {
                    if !q.is_empty()
                        && !p.name.to_lowercase().contains(&q)
                        && !p.display_name.to_lowercase().contains(&q)
                    {
                        continue;
                    }
                    let sel = pm.selected.as_deref() == Some(&p.name);
                    let mut text = egui::text::LayoutJob::default();
                    let color = ui.visuals().text_color();
                    let weak = ui.visuals().weak_text_color();
                    let font = egui::TextStyle::Body.resolve(ui.style());
                    text.append(&p.display_name, 0.0, egui::TextFormat::simple(font.clone(), color));
                    text.append(&p.version, 8.0, egui::TextFormat::simple(font, weak));
                    let r = ui.add(egui::Button::selectable(sel, text).min_size(egui::vec2(ui.available_width(), 0.0)));
                    let r = if p.direct {
                        r
                    } else {
                        r.on_hover_text("Installed as a dependency of another package")
                    };
                    if r.clicked() {
                        pm.selected = Some(p.name.clone());
                    }
                }
            });
        });
    egui::CentralPanel::default().show(ui, |ui| {
        let Some(p) = pm
            .selected
            .as_ref()
            .and_then(|n| pm.packages.iter().find(|p| &p.name == n))
            .cloned()
        else {
            ui.weak("Select a package.");
            return;
        };
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.label(RichText::new(&p.display_name).heading().strong());
            ui.horizontal(|ui| {
                ui.label(RichText::new(&p.version).strong());
                ui.weak("·");
                ui.label(p.source.label());
                if !p.author.is_empty() {
                    ui.weak("·");
                    ui.label(&p.author);
                }
            });
            ui.monospace(&p.name);
            match &p.source {
                PackageSource::Git { url, revision } => {
                    ui.label(format!(
                        "{url}{}",
                        revision
                            .as_ref()
                            .map(|r| format!(" #{r}"))
                            .unwrap_or_default()
                    ));
                }
                PackageSource::Registry { registry } => {
                    ui.label(format!("Registry: {registry}"));
                }
                _ => {}
            }
            ui.weak(p.root.display().to_string());
            ui.add_space(6.0);
            if !p.description.is_empty() {
                ui.label(&p.description);
                ui.add_space(6.0);
            }
            if let Some((name, _)) = &p.rust_crate {
                ui.label(format!(
                    "Rust crate `{name}` (a dependency of the Scripts crate)"
                ));
            }
            if !p.dependencies.is_empty() {
                ui.label(RichText::new("Dependencies").strong());
                for (d, v) in &p.dependencies {
                    ui.label(format!("  {d}  {v}"));
                }
            }
            if !p.samples.is_empty() {
                ui.add_space(6.0);
                ui.label(RichText::new("Samples").strong());
                for s in &p.samples {
                    ui.horizontal(|ui| {
                        ui.label(&s.display_name);
                        if !s.description.is_empty() {
                            ui.weak(&s.description);
                        }
                        let imported = project
                            .join(format!(
                                "Assets/Samples/{}/{}/{}",
                                p.display_name, p.version, s.display_name
                            ))
                            .is_dir();
                        if ui
                            .button(if imported { "Reimport" } else { "Import" })
                            .clicked()
                        {
                            pm.message = Some(match packages::import_sample(&project, &p, s) {
                                Ok(rel) => (true, format!("Imported into {rel}")),
                                Err(e) => (false, e),
                            });
                        }
                    });
                }
            }
            ui.add_space(10.0);
            ui.add_enabled_ui(!busy, |ui| match p.source {
                PackageSource::Embedded => {
                    ui.weak(
                        "Embedded package (a folder in Packages/): delete the folder to remove it.",
                    );
                }
                _ if !p.direct => {
                    ui.weak("Installed as a dependency of another package.");
                }
                _ => {
                    if ui.button("Remove").clicked() {
                        job = Some(Job::Remove(p.name.clone()));
                    }
                }
            });
        });
    });
    if let Some(j) = job {
        start(pm, project, j);
    }
}
