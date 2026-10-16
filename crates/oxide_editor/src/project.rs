//! Project window: browses the project's `Assets/` folder. Scenes open on double-click;
//! models can be dragged into the Scene view or Hierarchy, and assets onto Inspector fields.
//! Reference: <https://docs.unity3d.com/Manual/ProjectView.html>

use crate::{Action, EditorState};
use egui::{Color32, Sense, vec2};
use oxide_assets::AssetServer;
use oxide_engine::Engine;
use std::path::{Path, PathBuf};

/// Drag payload: a project-relative asset path (`Assets/Cars/car.glb`).
#[derive(Clone, Debug)]
pub struct AssetPayload(pub String);

pub struct ProjectState {
    /// Open folder, relative to the project root.
    pub folder: String,
    pub selected: Option<String>,
    search: String,
    new_folder: Option<String>,
    new_script: Option<String>,
    /// Cached listing and when it was read (seconds).
    pub cache: Option<(String, f64, Vec<Entry>)>,
}

impl Default for ProjectState {
    fn default() -> Self {
        Self {
            folder: "Assets".into(),
            selected: None,
            search: String::new(),
            new_folder: None,
            new_script: None,
            cache: None,
        }
    }
}

#[derive(Clone)]
pub struct Entry {
    name: String,
    /// Project-relative path with `/` separators.
    path: String,
    dir: bool,
}

fn kind(name: &str) -> (&'static str, Color32) {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "scene" => ("Scene", Color32::from_rgb(120, 200, 255)),
        "glb" | "gltf" | "fbx" | "obj" => ("Model", Color32::from_rgb(120, 220, 160)),
        "png" | "jpg" | "jpeg" | "tga" | "bmp" | "hdr" | "exr" => {
            ("Texture", Color32::from_rgb(230, 190, 110))
        }
        "wav" | "ogg" | "mp3" | "flac" => ("Audio", Color32::from_rgb(230, 140, 200)),
        "rs" => ("Script", Color32::from_rgb(240, 120, 90)),
        "prefab" => ("Prefab", Color32::from_rgb(140, 180, 255)),
        "mat" => ("Material", Color32::from_rgb(200, 160, 255)),
        "rendertexture" => ("Render Texture", Color32::from_rgb(230, 190, 110)),
        _ => ("File", Color32::from_gray(170)),
    }
}

fn list_dir(root: &Path, rel: &str) -> Vec<Entry> {
    let mut v: Vec<Entry> = std::fs::read_dir(root.join(rel))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name.ends_with(".meta") {
                return None;
            }
            Some(Entry {
                path: format!("{rel}/{name}"),
                dir: e.file_type().is_ok_and(|t| t.is_dir()),
                name,
            })
        })
        .collect();
    v.sort_by(|a, b| {
        b.dir
            .cmp(&a.dir)
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    v
}

/// A file name made from an object name (no path separators).
fn sanitize(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    if s.trim().is_empty() {
        "Prefab".into()
    } else {
        s.trim().to_owned()
    }
}

/// Project-relative paths of files with `ext` under `Assets/`.
pub fn find_files(root: &Path, ext: &str) -> Vec<String> {
    fn walk(root: &Path, rel: &str, ext: &str, out: &mut Vec<String>, depth: usize) {
        if depth > 12 {
            return;
        }
        for e in list_dir(root, rel) {
            if e.dir {
                walk(root, &e.path, ext, out, depth + 1);
            } else if e.name.to_ascii_lowercase().ends_with(&format!(".{ext}")) {
                out.push(e.path);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, "Assets", ext, &mut out, 0);
    out.sort();
    out
}

fn folder_tree(ui: &mut egui::Ui, root: &Path, rel: &str, open: &mut String, depth: usize) {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    let subdirs: Vec<Entry> = list_dir(root, rel).into_iter().filter(|e| e.dir).collect();
    let selected = open == rel;
    if subdirs.is_empty() || depth > 10 {
        if ui.selectable_label(selected, format!("  {name}")).clicked() {
            *open = rel.to_owned();
        }
        return;
    }
    let id = ui.make_persistent_id(("folder", rel));
    egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, depth == 0)
        .show_header(ui, |ui| {
            if ui.selectable_label(selected, name).clicked() {
                *open = rel.to_owned();
            }
        })
        .body(|ui| {
            for d in subdirs {
                folder_tree(ui, root, &d.path, open, depth + 1);
            }
        });
}

pub fn ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    let root: PathBuf = engine.world.resource::<AssetServer>().full_path("");
    let assets_dir = root.join("Assets");
    if !assets_dir.is_dir() {
        ui.label(format!("No Assets folder in {}", root.display()));
        if ui.button("Create Assets folder").clicked() {
            let _ = std::fs::create_dir_all(&assets_dir);
        }
        return;
    }
    let ps = &mut st.project;
    ui.horizontal(|ui| {
        ui.menu_button("+", |ui| {
            if ui.button("Folder").clicked() {
                ps.new_folder = Some("New Folder".into());
                ui.close();
            }
            // Asset files get a free "New X" name in the open folder, like Unity.
            let free = |stem: &str, ext: &str| {
                (0..)
                    .map(|i| match i {
                        0 => format!("{}/{stem}.{ext}", ps.folder),
                        n => format!("{}/{stem} {n}.{ext}", ps.folder),
                    })
                    .find(|p| !root.join(p).exists())
                    .expect("unbounded")
            };
            if ui.button("Material").clicked() {
                let path = free("New Material", "mat");
                let assets = engine.world.resource_mut::<AssetServer>();
                let m = oxide_assets::Material::lit("New Material", oxide_math::Color::WHITE);
                if let Err(e) = assets.save_material(&path, &m) {
                    oxide_core::debug_log_error!("{e}");
                }
                ps.cache = None;
                ui.close();
            }
            if ui.button("Render Texture").clicked() {
                let path = free("New Render Texture", "renderTexture");
                let assets = engine.world.resource_mut::<AssetServer>();
                if let Err(e) =
                    assets.save_render_texture(&path, &oxide_assets::RenderTexture::default())
                {
                    oxide_core::debug_log_error!("{e}");
                }
                ps.cache = None;
                ui.close();
            }
            if ui
                .add_enabled(st.scripts_available, egui::Button::new("Rust Script"))
                .on_disabled_hover_text("Open a project created with `oxide new` to write scripts")
                .clicked()
            {
                ps.new_script = Some("NewScript".into());
                ui.close();
            }
        });
        // Breadcrumbs.
        let parts: Vec<String> = ps.folder.split('/').map(str::to_owned).collect();
        for (i, part) in parts.iter().enumerate() {
            if i > 0 {
                ui.weak(">");
            }
            if ui.link(part).clicked() {
                ps.folder = parts[..=i].join("/");
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut ps.search)
                    .hint_text("Search")
                    .desired_width(160.0),
            );
        });
    });
    let mut script_action = None;
    if let Some(name) = ps.new_script.as_mut() {
        let mut done = false;
        ui.horizontal(|ui| {
            ui.label("New script:");
            let r = ui.text_edit_singleline(name);
            r.request_focus();
            if ui.button("Create").clicked()
                || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
            {
                script_action = Some(Action::CreateScript(name.trim().to_owned()));
                done = true;
            }
            if ui.button("Cancel").clicked() {
                done = true;
            }
        });
        if done {
            ps.new_script = None;
        }
    }
    if let Some(name) = ps.new_folder.as_mut() {
        let mut done = false;
        ui.horizontal(|ui| {
            ui.label("New folder:");
            let r = ui.text_edit_singleline(name);
            r.request_focus();
            if ui.button("Create").clicked()
                || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
            {
                let _ = std::fs::create_dir_all(root.join(&ps.folder).join(name.trim()));
                ps.cache = None;
                done = true;
            }
            if ui.button("Cancel").clicked() {
                done = true;
            }
        });
        if done {
            ps.new_folder = None;
        }
    }
    ui.separator();

    let now = oxide_core::time::realtime_since_startup();
    let stale = ps
        .cache
        .as_ref()
        .is_none_or(|(f, t, _)| *f != ps.folder || now - t > 1.0);
    if stale {
        ps.cache = Some((ps.folder.clone(), now, list_dir(&root, &ps.folder)));
    }
    let query = ps.search.trim().to_lowercase();
    let entries: Vec<Entry> = if query.is_empty() {
        ps.cache.as_ref().map(|c| c.2.clone()).unwrap_or_default()
    } else {
        let mut all = Vec::new();
        for ext in [
            "scene", "glb", "gltf", "png", "jpg", "wav", "ogg", "mp3", "rs", "prefab",
        ] {
            all.extend(find_files(&root, ext));
        }
        all.into_iter()
            .filter(|p| p.to_lowercase().contains(&query))
            .map(|p| Entry {
                name: p.rsplit('/').next().unwrap_or(&p).to_owned(),
                path: p,
                dir: false,
            })
            .collect()
    };

    let mut action = None;
    egui::Panel::left(egui::Id::new("project-folders"))
        .resizable(true)
        .default_size(150.0)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| folder_tree(ui, &root, "Assets", &mut ps.folder, 0));
        });
    let folder = ps.folder.clone();
    let mut new_prefab = None;
    let central = egui::CentralPanel::default().show(ui, |ui| {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if entries.is_empty() {
                    ui.weak("This folder is empty.");
                }
                for e in &entries {
                    let (label, color) = if e.dir {
                        ("Folder", Color32::from_rgb(220, 200, 120))
                    } else {
                        kind(&e.name)
                    };
                    let selected = ps.selected.as_deref() == Some(e.path.as_str());
                    let (rect, resp) = ui.allocate_exact_size(
                        vec2(ui.available_width(), 18.0),
                        Sense::click_and_drag(),
                    );
                    if selected {
                        ui.painter()
                            .rect_filled(rect, 0.0, crate::theme::SELECTION_BG);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(rect, 0.0, Color32::from_gray(69));
                    }
                    let icon =
                        egui::Rect::from_min_size(rect.min + vec2(4.0, 4.0), vec2(10.0, 10.0));
                    if e.dir {
                        ui.painter().rect_filled(icon, 1.0, color);
                    } else {
                        ui.painter().rect_stroke(
                            icon,
                            1.0,
                            egui::Stroke::new(1.5, color),
                            egui::StrokeKind::Inside,
                        );
                    }
                    ui.painter().text(
                        rect.left_center() + vec2(20.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        &e.name,
                        egui::FontId::proportional(12.5),
                        Color32::from_gray(215),
                    );
                    ui.painter().text(
                        rect.right_center() - vec2(6.0, 0.0),
                        egui::Align2::RIGHT_CENTER,
                        label,
                        egui::FontId::proportional(11.0),
                        Color32::from_gray(130),
                    );
                    if resp.clicked() {
                        ps.selected = Some(e.path.clone());
                    }
                    if resp.double_clicked() {
                        if e.dir {
                            ps.folder = e.path.clone();
                            ps.search.clear();
                        } else {
                            match kind(&e.name).0 {
                                "Scene" => action = Some(Action::OpenScene(e.path.clone())),
                                "Model" => action = Some(Action::InstantiateModel(e.path.clone())),
                                "Prefab" => {
                                    action = Some(Action::InstantiatePrefab(e.path.clone()))
                                }
                                _ => {}
                            }
                        }
                    }
                    if !e.dir && resp.drag_started() {
                        resp.dnd_set_drag_payload(AssetPayload(e.path.clone()));
                    }
                    if !e.dir {
                        resp.on_hover_text(&e.path);
                    }
                }
            });
    });
    // Drop a Hierarchy object here to make it a prefab (Unity's way of creating prefabs).
    if let Some(payload) = central
        .response
        .dnd_release_payload::<crate::hierarchy::HierarchyDrag>()
        && let Some(&id) = payload.0.first()
        && let Some(o) = engine.world.get(id)
    {
        let base = format!("{folder}/{}", sanitize(&o.name));
        let mut path = format!("{base}.prefab");
        let mut n = 1;
        while root.join(&path).exists() {
            path = format!("{base} {n}.prefab");
            n += 1;
        }
        new_prefab = Some((id, path));
    }
    if let Some((id, path)) = new_prefab {
        st.select(id);
        st.actions.push(Action::CreatePrefab(path));
    }
    if let Some(a) = action {
        st.actions.push(a);
    }
    if let Some(a) = script_action {
        st.actions.push(a);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_scenes() {
        let dir = std::env::temp_dir().join(format!("oxide-project-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Assets/Scenes")).unwrap();
        std::fs::write(dir.join("Assets/Scenes/Main.scene"), "x").unwrap();
        std::fs::write(dir.join("Assets/readme.txt"), "x").unwrap();
        assert_eq!(find_files(&dir, "scene"), vec!["Assets/Scenes/Main.scene"]);
        let top = list_dir(&dir, "Assets");
        assert!(top[0].dir, "folders first");
        let _ = std::fs::remove_dir_all(dir);
    }
}
