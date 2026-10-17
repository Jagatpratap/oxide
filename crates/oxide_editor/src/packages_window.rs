//! Unity's Export Package and Import Package windows (Assets menu): asset packages
//! (`.oxidepackage`, and `.unitypackage` on import) with a checklist of the assets, their
//! dependencies, and what importing each one would do.
//! Reference: <https://docs.unity3d.com/Manual/AssetPackagesCreate.html>,
//! <https://docs.unity3d.com/Manual/AssetPackagesImport.html>

use crate::EditorState;
use egui::{Color32, RichText};
use oxide_assets::AssetServer;
use oxide_assets::package::{self, ImportStatus, PackageItem};
use oxide_engine::Engine;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct ExportDialog {
    /// What the user selected (folders expand to their contents).
    roots: Vec<String>,
    include_dependencies: bool,
    /// Every candidate path and whether it's ticked.
    items: Vec<(String, bool)>,
}

pub struct ImportDialog {
    file: PathBuf,
    items: Vec<(PackageItem, ImportStatus, bool)>,
}

#[derive(Default)]
pub struct PackageDialogs {
    pub export: Option<ExportDialog>,
    pub import: Option<ImportDialog>,
}

fn candidates(engine: &Engine, roots: &[String], deps: bool) -> Vec<(String, bool)> {
    let assets = engine.world.resource::<AssetServer>();
    package::collect(&assets.full_path(""), &assets.database, roots, deps)
        .into_iter()
        .map(|p| (p, true))
        .collect()
}

/// Opens Export Package for `paths` (the Project window selection).
pub fn open_export(
    st: &mut EditorState,
    engine: &mut Engine,
    paths: Vec<String>,
) -> Result<String, String> {
    let paths: Vec<String> = paths
        .into_iter()
        .filter(|p| p.starts_with("Assets/") || p == "Assets")
        .collect();
    let roots = if paths.is_empty() || paths.iter().any(|p| p == "Assets") {
        vec!["Assets".to_owned()]
    } else {
        paths
    };
    let assets = engine.world.resource_mut::<AssetServer>();
    if !assets.database.is_scanned() {
        assets.database.refresh();
    }
    let roots = if roots == ["Assets"] {
        // The whole project: every top-level entry under Assets.
        assets
            .database
            .all()
            .into_iter()
            .map(|(p, _)| p.to_owned())
            .filter(|p| p.matches('/').count() == 1)
            .collect()
    } else {
        roots
    };
    let items = candidates(engine, &roots, true);
    let n = items.len();
    st.packages.export = Some(ExportDialog {
        roots,
        include_dependencies: true,
        items,
    });
    Ok(format!("Export Package: {n} assets"))
}

/// Opens Import Package for a package file (asks for one when `None`).
pub fn open_import(
    st: &mut EditorState,
    engine: &mut Engine,
    file: Option<PathBuf>,
) -> Result<String, String> {
    let file = match file {
        Some(f) => f,
        None => match pick_package() {
            Some(f) => f,
            None => return Ok(String::new()),
        },
    };
    let items = package::read(&file)?;
    let assets = engine.world.resource_mut::<AssetServer>();
    if !assets.database.is_scanned() {
        assets.database.refresh();
    }
    let root = assets.full_path("");
    let items: Vec<(PackageItem, ImportStatus, bool)> = items
        .into_iter()
        .map(|i| {
            let s = package::status(&root, &assets.database, &i);
            (i, s, s != ImportStatus::Unchanged)
        })
        .collect();
    let n = items.len();
    st.packages.import = Some(ImportDialog { file, items });
    Ok(format!("Import Package: {n} assets"))
}

/// Writes the package (MCP and the Export button).
pub fn export(engine: &mut Engine, paths: &[String], out: &Path) -> Result<usize, String> {
    let assets = engine.world.resource_mut::<AssetServer>();
    let root = assets.full_path("");
    let items = package::items_for(&root, &mut assets.database, paths)?;
    package::write(&items, out)?;
    Ok(items.len())
}

/// Imports package items, then refreshes so they get loaded (MCP and the Import button).
pub fn import(
    st: &mut EditorState,
    engine: &mut Engine,
    items: &[PackageItem],
) -> Result<Vec<String>, String> {
    let assets = engine.world.resource::<AssetServer>();
    let written = package::import(&assets.full_path(""), &assets.database, items)?;
    if let Some(msg) = crate::asset_ops::refresh_database(st, engine) {
        oxide_core::debug_log!("{msg}");
    }
    engine.world.resource_mut::<AssetServer>().reload_changed();
    st.project.invalidate();
    st.previews.clear();
    Ok(written)
}

pub fn ui(st: &mut EditorState, engine: &mut Engine, ctx: &egui::Context) {
    export_ui(st, engine, ctx);
    import_ui(st, engine, ctx);
}

fn export_ui(st: &mut EditorState, engine: &mut Engine, ctx: &egui::Context) {
    let Some(d) = st.packages.export.as_mut() else {
        return;
    };
    let mut close = false;
    let mut go = false;
    let mut deps_changed = false;
    egui::Modal::new(egui::Id::new("export-package")).show(ctx, |ui| {
        ui.set_width(460.0);
        ui.heading("Exporting package");
        ui.weak("Items to export");
        ui.separator();
        egui::ScrollArea::vertical()
            .max_height(320.0)
            .show(ui, |ui| {
                for (p, on) in &mut d.items {
                    let depth = p.matches('/').count().saturating_sub(1);
                    ui.horizontal(|ui| {
                        ui.add_space(depth as f32 * 12.0);
                        ui.checkbox(on, p.rsplit('/').next().unwrap_or(p));
                    });
                }
            });
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("All").clicked() {
                d.items.iter_mut().for_each(|(_, on)| *on = true);
            }
            if ui.button("None").clicked() {
                d.items.iter_mut().for_each(|(_, on)| *on = false);
            }
            deps_changed = ui
                .checkbox(&mut d.include_dependencies, "Include dependencies")
                .changed();
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                go = ui.button("Export…").clicked();
                close = ui.button("Cancel").clicked();
            });
        });
    });
    if deps_changed {
        let (roots, deps) = (d.roots.clone(), d.include_dependencies);
        d.items = candidates(engine, &roots, deps);
    }
    if go {
        let paths: Vec<String> = d
            .items
            .iter()
            .filter(|(_, on)| *on)
            .map(|(p, _)| p.clone())
            .collect();
        let default = d
            .roots
            .first()
            .and_then(|r| r.rsplit('/').next())
            .map(|n| n.split('.').next().unwrap_or(n))
            .unwrap_or("Package")
            .to_owned();
        if let Some(out) = save_package_as(&default) {
            st.status = match export(engine, &paths, &out) {
                Ok(n) => format!("Exported {n} assets to {}", out.display()),
                Err(e) => e,
            };
            close = true;
        }
    }
    if close {
        st.packages.export = None;
    }
}

fn import_ui(st: &mut EditorState, engine: &mut Engine, ctx: &egui::Context) {
    let Some(d) = st.packages.import.as_mut() else {
        return;
    };
    let mut close = false;
    let mut go = false;
    egui::Modal::new(egui::Id::new("import-package")).show(ctx, |ui| {
        ui.set_width(520.0);
        ui.heading("Import Package");
        ui.weak(d.file.display().to_string());
        ui.separator();
        egui::ScrollArea::vertical()
            .max_height(340.0)
            .show(ui, |ui| {
                for (it, status, on) in &mut d.items {
                    let depth = it.path.matches('/').count().saturating_sub(1);
                    ui.horizontal(|ui| {
                        ui.add_space(depth as f32 * 12.0);
                        let label = it.path.rsplit('/').next().unwrap_or(&it.path);
                        ui.add_enabled(
                            *status != ImportStatus::Unchanged,
                            egui::Checkbox::new(on, label),
                        );
                        let (text, col) = match status {
                            ImportStatus::New => ("NEW", Color32::from_rgb(110, 200, 120)),
                            ImportStatus::Changed => ("CHANGED", Color32::from_rgb(230, 190, 80)),
                            ImportStatus::Moved => ("MOVED", Color32::from_rgb(120, 170, 240)),
                            ImportStatus::Conflict => ("RENAMED", Color32::from_rgb(230, 120, 100)),
                            ImportStatus::Unchanged => ("", Color32::GRAY),
                        };
                        if !text.is_empty() {
                            ui.label(RichText::new(text).small().color(col))
                                .on_hover_text(match status {
                                    ImportStatus::New => "New to this project",
                                    ImportStatus::Changed => {
                                        "Replaces the project's copy of this asset"
                                    }
                                    ImportStatus::Moved => {
                                        "This asset is elsewhere in the project; it's updated there"
                                    }
                                    ImportStatus::Conflict => {
                                        "Another asset has this path; imported under a new name"
                                    }
                                    ImportStatus::Unchanged => "",
                                });
                        }
                    });
                }
            });
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("All").clicked() {
                d.items
                    .iter_mut()
                    .filter(|(_, s, _)| *s != ImportStatus::Unchanged)
                    .for_each(|(_, _, on)| *on = true);
            }
            if ui.button("None").clicked() {
                d.items.iter_mut().for_each(|(_, _, on)| *on = false);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                go = ui.button("Import").clicked();
                close = ui.button("Cancel").clicked();
            });
        });
    });
    if go {
        let items: Vec<PackageItem> = d
            .items
            .iter()
            .filter(|(_, _, on)| *on)
            .map(|(i, _, _)| i.clone())
            .collect();
        st.status = match import(st, engine, &items) {
            Ok(w) => format!("Imported {} assets", w.len()),
            Err(e) => e,
        };
        close = true;
    }
    if close {
        st.packages.import = None;
    }
}

// ---- system file dialogs --------------------------------------------------------------------

/// The system's open-file dialog for a package (blocks until chosen or cancelled).
fn pick_package() -> Option<PathBuf> {
    let out = if cfg!(target_os = "macos") {
        Command::new("osascript")
            .args([
                "-e",
                "POSIX path of (choose file with prompt \"Import package\" of type {\"oxidepackage\", \"unitypackage\"})",
            ])
            .output()
    } else if cfg!(windows) {
        Command::new("powershell")
            .args([
                "-NoProfile",
                "-STA",
                "-Command",
                "Add-Type -AssemblyName System.Windows.Forms; $d = New-Object System.Windows.Forms.OpenFileDialog; \
                 $d.Filter = 'Packages (*.oxidepackage;*.unitypackage)|*.oxidepackage;*.unitypackage'; \
                 if ($d.ShowDialog() -eq 'OK') { $d.FileName }",
            ])
            .output()
    } else {
        Command::new("zenity")
            .args([
                "--file-selection",
                "--file-filter=*.oxidepackage *.unitypackage",
            ])
            .output()
    };
    let text = String::from_utf8_lossy(&out.ok().filter(|o| o.status.success())?.stdout)
        .trim()
        .to_owned();
    (!text.is_empty()).then(|| PathBuf::from(text))
}

/// The system's save dialog for a new package.
fn save_package_as(default_name: &str) -> Option<PathBuf> {
    let name = format!("{}.oxidepackage", default_name.replace('"', ""));
    let out = if cfg!(target_os = "macos") {
        Command::new("osascript")
            .args([
                "-e",
                &format!("POSIX path of (choose file name with prompt \"Export package\" default name \"{name}\")"),
            ])
            .output()
    } else if cfg!(windows) {
        Command::new("powershell")
            .args([
                "-NoProfile",
                "-STA",
                "-Command",
                &format!(
                    "Add-Type -AssemblyName System.Windows.Forms; $d = New-Object System.Windows.Forms.SaveFileDialog; \
                     $d.FileName = '{name}'; if ($d.ShowDialog() -eq 'OK') {{ $d.FileName }}"
                ),
            ])
            .output()
    } else {
        Command::new("zenity")
            .args(["--file-selection", "--save", &format!("--filename={name}")])
            .output()
    };
    let text = String::from_utf8_lossy(&out.ok().filter(|o| o.status.success())?.stdout)
        .trim()
        .to_owned();
    if text.is_empty() {
        return None;
    }
    let mut p = PathBuf::from(text);
    if p.extension().is_none_or(|e| e != "oxidepackage") {
        p.set_extension("oxidepackage");
    }
    Some(p)
}
