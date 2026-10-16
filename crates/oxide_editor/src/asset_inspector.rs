//! The Inspector for an asset selected in the Project window: texture import settings
//! (with Apply / Revert), the material editor, render texture settings, model, prefab and
//! scene summaries, a read-only view of scripts and shaders, Asset Labels and a preview.
//! Reference: <https://docs.unity3d.com/Manual/class-TextureImporter.html>,
//! <https://docs.unity3d.com/Manual/AssetLabels.html>

use crate::{Action, EditorState};
use egui::{Color32, RichText};
use oxide_assets::{AssetServer, TextureImporter};
use oxide_core::reflect::{Reflect, Value};
use oxide_engine::Engine;
use std::path::Path;

#[derive(Default)]
pub struct AssetInspectorState {
    /// Texture import settings being edited (applied with Apply), by asset path.
    importer: Option<(String, TextureImporter)>,
    /// Text shown for scripts and shaders, by path and file time.
    text: Option<(String, Option<std::time::SystemTime>, String)>,
    new_label: String,
}

const TEXT_EXTENSIONS: [&str; 9] = [
    "rs", "wgsl", "txt", "md", "json", "yaml", "toml", "shader", "hlsl",
];

pub fn ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui, path: &str) {
    let root = engine.world.resource::<AssetServer>().full_path("");
    let full = root.join(path);
    if !full.exists() {
        ui.weak(format!("{path} no longer exists."));
        return;
    }
    let dir = full.is_dir();
    let name = path.rsplit('/').next().unwrap_or(path).to_owned();
    let type_name = if dir {
        "Folder"
    } else {
        crate::project::kind(&name).0
    };
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();

    // Header: thumbnail, name, type and Open.
    ui.horizontal(|ui| {
        let icon = egui::vec2(40.0, 40.0);
        match (!dir).then(|| st.previews.get(&root, path)).flatten() {
            Some(t) => {
                ui.add(egui::Image::new((t, icon)));
            }
            None => {
                ui.allocate_exact_size(icon, egui::Sense::hover());
            }
        }
        ui.vertical(|ui| {
            ui.label(RichText::new(&name).strong().size(14.0));
            ui.weak(match type_name {
                "Texture" | "Model" | "AudioClip" => format!("{type_name} Import Settings"),
                t => t.to_owned(),
            });
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Open").clicked()
                && let Err(e) = crate::project::open(st, engine, &root, path)
            {
                st.status = e;
            }
        });
    });
    ui.separator();

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().interact_size.x = 24.0;
            match type_name {
                "Texture" => texture(st, engine, ui, path),
                "Material" => material(st, engine, ui, path),
                "RenderTexture" => render_texture(st, engine, ui, path),
                "Model" => model(st, engine, ui, path),
                "Prefab" => prefab(st, engine, ui, path),
                "Scene" => {
                    if ui.button("Open Scene").clicked() {
                        st.actions.push(Action::OpenScene(path.to_owned()));
                    }
                }
                "InputActionAsset" => input_actions(st, engine, ui, path),
                t if engine.world.resource::<AssetServer>().types.is_data(t) => {
                    data_asset(st, engine, ui, path)
                }
                "Folder" => {
                    let n = std::fs::read_dir(&full)
                        .map(|d| {
                            d.flatten()
                                .filter(|e| !e.file_name().to_string_lossy().ends_with(".meta"))
                                .count()
                        })
                        .unwrap_or(0);
                    ui.label(format!("{n} items"));
                }
                _ => {}
            }
            if TEXT_EXTENSIONS.contains(&ext.as_str()) {
                text_view(st, ui, &full, path);
            }
            ui.add_space(8.0);
            file_info(ui, &full);
            ui.separator();
            labels(st, engine, ui, path);
            preview(st, ui, &root, path);
        });
}

fn file_info(ui: &mut egui::Ui, full: &Path) {
    if let Ok(m) = std::fs::metadata(full)
        && m.is_file()
    {
        let size = m.len() as f64;
        let s = if size > 1024.0 * 1024.0 {
            format!("{:.1} MB", size / (1024.0 * 1024.0))
        } else if size > 1024.0 {
            format!("{:.1} KB", size / 1024.0)
        } else {
            format!("{size} bytes")
        };
        ui.weak(format!("File size: {s}"));
    }
}

fn texture(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui, path: &str) {
    let saved = engine
        .world
        .resource::<AssetServer>()
        .texture_importer(path);
    let ia = &mut st.asset_inspector;
    if ia.importer.as_ref().is_none_or(|(p, _)| p != path) {
        ia.importer = Some((path.to_owned(), saved.clone()));
    }
    let Some((_, imp)) = ia.importer.as_mut() else {
        return;
    };
    crate::inspector::reflect_grid(ui, &engine.world, imp, "texture-importer");
    let changed = *imp != saved;
    ui.add_space(4.0);
    let mut apply = false;
    let mut revert = false;
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            apply = ui
                .add_enabled(changed, egui::Button::new("Apply"))
                .clicked();
            revert = ui
                .add_enabled(changed, egui::Button::new("Revert"))
                .clicked();
        });
    });
    if revert {
        ia.importer = Some((path.to_owned(), saved));
    } else if apply {
        let imp = imp.clone();
        match engine
            .world
            .resource_mut::<AssetServer>()
            .set_texture_importer(path, &imp)
        {
            Ok(_) => st.status = format!("Reimported {path}"),
            Err(e) => st.status = e,
        }
    }
    // What the import produced.
    let assets = engine.world.resource_mut::<AssetServer>();
    let h = match assets.textures.handle_of(path) {
        Some(h) => Some(h),
        None => assets.load_texture(path).ok(),
    };
    if let Some(t) = h.and_then(|h| assets.textures.get(&h)) {
        let format = match &t.compressed {
            Some(c) => format!("{:?}", c.format),
            None if t.srgb => "RGBA32 (sRGB)".into(),
            None => "RGBA32".into(),
        };
        let mips = t.compressed.as_ref().map_or(t.mips.len(), |c| c.mips.len());
        ui.weak(format!(
            "{}×{}  {format}  {mips} mip levels",
            t.width, t.height
        ));
    }
}

/// Edits a reflected asset in place; `true` when a field changed. Asset references picked
/// in the fields are loaded.
fn edit_fields(ui: &mut egui::Ui, engine: &mut Engine, obj: &mut dyn Reflect, salt: &str) -> bool {
    let changed = crate::inspector::reflect_grid(ui, &engine.world, obj, salt);
    if changed {
        let assets = engine.world.resource_mut::<AssetServer>();
        for (_, v) in obj.fields() {
            if let Value::Asset(p) = v
                && let Err(e) = assets.ensure_loaded(&p)
            {
                oxide_core::debug_log_warning!("{e}");
            }
        }
    }
    changed
}

/// Input Actions: Unity's "Edit Asset" button and a summary of maps and actions.
fn input_actions(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui, path: &str) {
    if ui.button("Edit Asset").clicked() {
        st.input_actions.open(engine, path);
        st.actions.push(Action::OpenTab(crate::Tab::InputActions));
    }
    let h = oxide_assets::Handle::<oxide_input::InputActionAsset>::from_path(path);
    let Some(a) = engine
        .world
        .resource_mut::<AssetServer>()
        .data_asset(&h)
        .cloned()
    else {
        return;
    };
    ui.add_space(4.0);
    for m in &a.maps {
        let names: Vec<&str> = m.actions.iter().map(|x| x.name.as_str()).collect();
        ui.label(RichText::new(&m.name).strong());
        ui.weak(names.join(", "));
    }
    if !a.control_schemes.is_empty() {
        let s: Vec<&str> = a.control_schemes.iter().map(|x| x.name.as_str()).collect();
        ui.weak(format!("Control schemes: {}", s.join(", ")));
    }
}

/// A data asset (ScriptableObject): its reflected fields, saved to the file on every edit.
fn data_asset(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui, path: &str) {
    let assets = engine.world.resource_mut::<AssetServer>();
    if !assets.data.contains(path)
        && let Err(e) = assets.load_data(path)
    {
        ui.colored_label(Color32::from_rgb(230, 120, 100), e);
        return;
    }
    let Some(mut value) = assets.data.get_dyn(path).map(|v| v.clone_box()) else {
        return;
    };
    let salt = format!("data:{path}");
    if edit_fields(ui, engine, value.as_reflect_mut(), &salt)
        && let Err(e) = engine
            .world
            .resource_mut::<AssetServer>()
            .save_data(path, value)
    {
        st.status = e;
    }
}

fn material(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui, path: &str) {
    let assets = engine.world.resource_mut::<AssetServer>();
    let h = match assets.materials.handle_of(path) {
        Some(h) => h,
        None => match assets.load_material(path) {
            Ok(h) => h,
            Err(e) => {
                ui.colored_label(Color32::from_rgb(230, 120, 100), e);
                return;
            }
        },
    };
    let Some(mut m) = assets.materials.get(&h).cloned() else {
        return;
    };
    ui.weak("Shader: Oxide/Lit");
    if edit_fields(ui, engine, &mut m, "material") {
        // Unity saves material edits straight to the asset.
        if let Err(e) = engine
            .world
            .resource_mut::<AssetServer>()
            .save_material(path, &m)
        {
            st.status = e;
        }
    }
}

fn render_texture(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui, path: &str) {
    let assets = engine.world.resource_mut::<AssetServer>();
    let h = match assets.render_textures.handle_of(path) {
        Some(h) => h,
        None => match assets.load_render_texture(path) {
            Ok(h) => h,
            Err(e) => {
                ui.colored_label(Color32::from_rgb(230, 120, 100), e);
                return;
            }
        },
    };
    let Some(mut rt) = assets.render_textures.get(&h).cloned() else {
        return;
    };
    if edit_fields(ui, engine, &mut rt, "render-texture")
        && let Err(e) = engine
            .world
            .resource_mut::<AssetServer>()
            .save_render_texture(path, &rt)
    {
        st.status = e;
    }
}

fn model(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui, path: &str) {
    let assets = engine.world.resource_mut::<AssetServer>();
    let h = match assets.models.handle_of(path) {
        Some(h) => h,
        None => match assets.load_model(path) {
            Ok(h) => h,
            Err(e) => {
                ui.colored_label(Color32::from_rgb(230, 120, 100), e);
                return;
            }
        },
    };
    let Some(m) = assets.models.get(&h) else {
        return;
    };
    let meshes = m.nodes.iter().filter(|n| n.mesh.is_some()).count();
    let mut materials: Vec<&str> = m
        .nodes
        .iter()
        .flat_map(|n| n.materials.iter().map(|h| h.path()))
        .collect();
    materials.sort_unstable();
    materials.dedup();
    let triangles: usize = m
        .nodes
        .iter()
        .filter_map(|n| n.mesh.as_ref().and_then(|h| assets.meshes.get(h)))
        .map(|mesh| mesh.indices.len() / 3)
        .sum();
    egui::Grid::new("model-info")
        .num_columns(2)
        .spacing([16.0, 4.0])
        .show(ui, |ui| {
            let mut row = |k: &str, v: String| {
                ui.label(k);
                ui.label(v);
                ui.end_row();
            };
            row("Nodes", m.nodes.len().to_string());
            row("Meshes", meshes.to_string());
            row("Triangles", triangles.to_string());
            row("Materials", materials.len().to_string());
            row("Skins", m.skins.len().to_string());
            row("Animations", m.animations.len().to_string());
        });
    if !m.animations.is_empty() {
        egui::CollapsingHeader::new("Animation clips").show(ui, |ui| {
            for a in &m.animations {
                let name = assets
                    .animations
                    .get(a)
                    .map_or_else(|| a.path().to_owned(), |c| c.name.clone());
                ui.label(name);
            }
        });
    }
    ui.add_space(4.0);
    if ui.button("Add to Scene").clicked() {
        st.actions.push(Action::InstantiateModel(path.to_owned()));
    }
}

fn prefab(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui, path: &str) {
    match engine.world.load_prefab(path) {
        Ok(file) => {
            if let Some(b) = &file.base {
                ui.horizontal(|ui| {
                    ui.label("Variant of");
                    if ui.link(b).clicked() {
                        st.project.show(b);
                    }
                });
            }
            let root = file.objects.iter().find(|o| o.parent.is_none());
            ui.label(format!(
                "Root: {}   {} objects",
                root.map_or("—", |o| o.name.as_str()),
                file.objects.len()
            ));
            let mut types: Vec<&str> = file
                .objects
                .iter()
                .flat_map(|o| o.components.iter().map(|c| c.type_name.as_str()))
                .collect();
            types.sort_unstable();
            types.dedup();
            ui.weak(format!("Components: {}", types.join(", ")));
        }
        Err(e) => {
            ui.colored_label(Color32::from_rgb(230, 120, 100), e);
        }
    }
    ui.add_space(4.0);
    if ui.button("Add to Scene").clicked() {
        st.actions.push(Action::InstantiatePrefab(path.to_owned()));
    }
}

/// Read-only source of scripts and shaders (Unity's TextAsset / MonoScript Inspector).
fn text_view(st: &mut EditorState, ui: &mut egui::Ui, full: &Path, path: &str) {
    let mtime = std::fs::metadata(full).and_then(|m| m.modified()).ok();
    let ia = &mut st.asset_inspector;
    if ia
        .text
        .as_ref()
        .is_none_or(|(p, t, _)| p != path || *t != mtime)
    {
        let mut text = std::fs::read_to_string(full).unwrap_or_default();
        const MAX: usize = 64 * 1024;
        if text.len() > MAX {
            let mut cut = MAX;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
            text.push_str("\n…");
        }
        ia.text = Some((path.to_owned(), mtime, text));
    }
    if let Some((_, _, text)) = &ia.text {
        egui::Frame::NONE
            .fill(Color32::from_rgb(42, 42, 42))
            .inner_margin(6.0)
            .show(ui, |ui| {
                egui::ScrollArea::both()
                    .id_salt("asset-text")
                    .max_height(360.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(RichText::new(text.as_str()).monospace().size(11.5))
                                .extend(),
                        );
                    });
            });
    }
}

/// Asset Labels: chips with remove buttons, and a menu of existing labels plus a field
/// for a new one.
fn labels(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui, path: &str) {
    let root = engine.world.resource::<AssetServer>().full_path("");
    let current = engine.world.resource::<AssetServer>().asset_labels(path);
    let known = crate::project::all_labels(&mut st.project, &root);
    let mut next: Option<Vec<String>> = None;
    ui.horizontal_wrapped(|ui| {
        ui.label("Asset Labels");
        for l in &current {
            let chip = egui::Button::new(RichText::new(format!("{l}  ×")).size(11.0))
                .fill(Color32::from_rgb(60, 80, 100))
                .corner_radius(8.0);
            if ui.add(chip).on_hover_text("Remove label").clicked() {
                next = Some(current.iter().filter(|x| *x != l).cloned().collect());
            }
        }
        ui.menu_button("+", |ui| {
            for l in &known {
                let mut on = current.contains(l);
                if ui.checkbox(&mut on, l).changed() {
                    let mut v = current.clone();
                    if on {
                        v.push(l.clone());
                    } else {
                        v.retain(|x| x != l);
                    }
                    next = Some(v);
                }
            }
            if !known.is_empty() {
                ui.separator();
            }
            let ia = &mut st.asset_inspector;
            let r = ui.add(
                egui::TextEdit::singleline(&mut ia.new_label)
                    .hint_text("New label")
                    .desired_width(140.0),
            );
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                let l = std::mem::take(&mut ia.new_label);
                if !l.trim().is_empty() {
                    let mut v = current.clone();
                    v.push(l.trim().to_owned());
                    next = Some(v);
                }
                ui.close();
            }
        })
        .response
        .on_hover_text("Add label");
    });
    if let Some(v) = next {
        match engine
            .world
            .resource::<AssetServer>()
            .set_asset_labels(path, &v)
        {
            Ok(()) => st.project.invalidate(),
            Err(e) => st.status = e,
        }
    }
}

/// The large preview at the bottom of the Inspector.
fn preview(st: &mut EditorState, ui: &mut egui::Ui, root: &Path, path: &str) {
    let Some(t) = st.previews.get(root, path) else {
        return;
    };
    ui.separator();
    ui.weak("Preview");
    let side = crate::inspector::avail(ui).min(256.0);
    let tex_size = ui
        .ctx()
        .tex_manager()
        .read()
        .meta(t)
        .map(|m| m.size)
        .unwrap_or([1, 1]);
    let aspect = tex_size[0].max(1) as f32 / tex_size[1].max(1) as f32;
    let size = if aspect >= 1.0 {
        egui::vec2(side, side / aspect)
    } else {
        egui::vec2(side * aspect, side)
    };
    ui.vertical_centered(|ui| {
        ui.add(egui::Image::new((t, size)));
    });
}
