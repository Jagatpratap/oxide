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
    /// Texture importer platform tab (None: Default).
    platform_tab: Option<oxide_assets::BuildTarget>,
    /// Text shown for scripts and shaders, by path and file time.
    text: Option<(String, Option<std::time::SystemTime>, String)>,
    new_label: String,
    /// Packed atlas preview: (atlas path, texture version, image).
    atlas_preview: Option<(String, u64, egui::TextureHandle)>,
    /// Model / Audio import settings being edited (applied with Apply), by asset path.
    model_importer: Option<(String, oxide_assets::importers::ModelImporter)>,
    audio_importer: Option<(String, oxide_assets::importers::AudioImporter)>,
    plugin_importer: Option<(String, oxide_assets::importers::PluginImporter)>,
    /// The `.asmdef` being edited (applied with Apply): (path, edited, as loaded).
    asmdef: Option<(String, crate::assemblies::AsmDef, crate::assemblies::AsmDef)>,
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
                "NativePlugin" => "Plugin Import Settings".to_owned(),
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
                "Model" => {
                    let mut slot = st.asset_inspector.model_importer.take();
                    importer_settings(st, engine, ui, path, &mut slot, "model-importer");
                    st.asset_inspector.model_importer = slot;
                    ui.separator();
                    model(st, engine, ui, path)
                }
                "AudioClip" => {
                    let mut slot = st.asset_inspector.audio_importer.take();
                    importer_settings(st, engine, ui, path, &mut slot, "audio-importer");
                    st.asset_inspector.audio_importer = slot;
                }
                // Unity's Plugin Inspector: the platforms it is included for.
                "NativePlugin" => {
                    let mut slot = st.asset_inspector.plugin_importer.take();
                    importer_settings(st, engine, ui, path, &mut slot, "plugin-importer");
                    st.asset_inspector.plugin_importer = slot;
                    ui.separator();
                    let stem = std::path::Path::new(path)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("");
                    let name = stem
                        .strip_prefix(std::env::consts::DLL_PREFIX)
                        .filter(|_| !std::env::consts::DLL_PREFIX.is_empty())
                        .unwrap_or(stem);
                    let loaded = oxide_engine::native_plugins::NativePlugins::loaded()
                        .iter()
                        .any(|n| n == name);
                    ui.label(format!("Plugin name: \"{name}\""));
                    ui.weak(if loaded {
                        "Loaded in the editor"
                    } else {
                        "Not loaded yet (loads on first call)"
                    });
                    let host = path
                        .to_ascii_lowercase()
                        .ends_with(std::env::consts::DLL_SUFFIX);
                    if !host {
                        ui.weak(format!(
                            "Built for another OS: this editor and its players use {} libraries.",
                            std::env::consts::DLL_SUFFIX
                        ));
                    }
                }
                "Prefab" => prefab(st, engine, ui, path),
                "Scene" => {
                    if ui.button("Open Scene").clicked() {
                        st.actions.push(Action::OpenScene(path.to_owned()));
                    }
                }
                "InputActionAsset" => input_actions(st, engine, ui, path),
                "SpriteAtlas" => sprite_atlas(st, engine, ui, path),
                "AnimationClip" => crate::anim_inspectors::clip(st, engine, ui, path),
                "AnimatorController" => crate::anim_inspectors::controller(st, engine, ui, path),
                "AnimatorOverrideController" => {
                    crate::anim_inspectors::override_controller(st, engine, ui, path)
                }
                "AvatarMask" => crate::anim_inspectors::avatar_mask(st, engine, ui, path),
                "Preset" => preset(st, engine, ui, path),
                "ShaderGraph" | "ShaderSubGraph" => shader_graph_asset(st, engine, ui, path),
                "AssemblyDefinitionAsset" => asmdef(st, engine, ui, path),
                "AssemblyDefinitionReferenceAsset" => asmref(st, engine, ui, path),
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
    let mut slot = st.asset_inspector.importer.take();
    if slot.as_ref().is_none_or(|(p, _)| p != path) {
        slot = Some((path.to_owned(), saved.clone()));
    }
    let Some((_, imp)) = slot.as_mut() else {
        return;
    };
    // Fields of the current Texture Shape and Texture Type only.
    let (shape, kind) = (imp.texture_shape, imp.texture_type);
    crate::inspector::reflect_grid_where(ui, &engine.world, imp, "texture-importer", &|f| {
        use oxide_assets::{TextureImporterType as K, TextureShape as S};
        match f {
            "cubemap_mapping" => shape == S::Cube,
            "columns" | "rows" => shape == S::Texture2DArray,
            "sprite_mode" | "pixels_per_unit" | "sprite_pivot" | "sprite_border" => {
                kind == K::Sprite
            }
            // Edited in the Sprite Editor and the platform tabs below.
            "sprites" | "platform_settings" => false,
            _ => true,
        }
    });
    platform_tabs(ui, &mut st.asset_inspector.platform_tab, imp);
    let changed = *imp != saved;
    ui.add_space(4.0);
    let mut apply = false;
    let mut revert = false;
    let mut sprite_editor = false;
    ui.horizontal(|ui| {
        importer_presets(st, engine, ui, path, imp);
        if imp.texture_type == oxide_assets::TextureImporterType::Sprite {
            sprite_editor = ui.button("Sprite Editor").clicked();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            apply = ui
                .add_enabled(changed, egui::Button::new("Apply"))
                .clicked();
            revert = ui
                .add_enabled(changed, egui::Button::new("Revert"))
                .clicked();
        });
    });
    if sprite_editor {
        st.sprite_editor.open(path);
        st.actions.push(Action::OpenTab(crate::Tab::SpriteEditor));
    }
    let imp = imp.clone();
    st.asset_inspector.importer = slot;
    if revert {
        st.asset_inspector.importer = Some((path.to_owned(), saved));
    } else if apply {
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
            "{}×{}  {format}  {mips} mip levels  ({} build)",
            t.width,
            t.height,
            assets.build_target.name()
        ));
    }
    let id = oxide_assets::Handle::<oxide_assets::TextureCube>::from_path(path.to_owned());
    if let Some(c) = assets.cubemaps.get(&id) {
        let format = c.faces[0]
            .compressed
            .as_ref()
            .map_or("RGBA32".into(), |c| format!("{:?}", c.format));
        ui.weak(format!("Cubemap: 6 faces of {0}×{0}  {format}", c.size()));
    }
    let id = oxide_assets::Handle::<oxide_assets::Texture2DArray>::from_path(path.to_owned());
    if let Some(a) = assets.texture_arrays.get(&id) {
        ui.weak(format!(
            "2D Array: {} layers of {}×{}",
            a.depth(),
            a.layers[0].width,
            a.layers[0].height
        ));
    }
}

/// Unity's platform tabs under the texture settings: Default, then each platform's
/// override (max size, format, compression).
fn platform_tabs(
    ui: &mut egui::Ui,
    tab: &mut Option<oxide_assets::BuildTarget>,
    imp: &mut TextureImporter,
) {
    use oxide_assets::{BuildTarget as B, TextureCompression, TextureImporterFormat as F};
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.selectable_value(tab, None, "Default");
        for p in [B::Standalone, B::Android, B::IOS, B::WebGL] {
            let overridden = imp
                .platform_settings
                .iter()
                .any(|s| s.platform == p && s.overridden);
            let label = if overridden {
                format!("{} •", p.name())
            } else {
                p.name().to_owned()
            };
            ui.selectable_value(tab, Some(p), label);
        }
    });
    let Some(p) = *tab else {
        ui.weak("Max Size and Compression above apply to every platform without an override.");
        return;
    };
    let i = match imp.platform_settings.iter().position(|s| s.platform == p) {
        Some(i) => i,
        None => {
            let mut s = imp.platform(p);
            s.overridden = false;
            imp.platform_settings.push(s);
            imp.platform_settings.len() - 1
        }
    };
    let s = &mut imp.platform_settings[i];
    ui.checkbox(&mut s.overridden, format!("Override For {}", p.name()));
    ui.add_enabled_ui(s.overridden, |ui| {
        egui::Grid::new("tex-platform")
            .num_columns(2)
            .show(ui, |ui| {
                ui.label("Max Size");
                egui::ComboBox::from_id_salt("tex-platform-size")
                    .selected_text(s.max_texture_size.to_string())
                    .show_ui(ui, |ui| {
                        for v in [32, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384] {
                            ui.selectable_value(&mut s.max_texture_size, v, v.to_string());
                        }
                    });
                ui.end_row();
                ui.label("Format");
                egui::ComboBox::from_id_salt("tex-platform-format")
                    .selected_text(format!("{:?}", s.format))
                    .show_ui(ui, |ui| {
                        for f in [
                            F::Automatic,
                            F::RGBA32,
                            F::DXT1,
                            F::DXT5,
                            F::BC4,
                            F::BC5,
                            F::BC7,
                            F::Etc2Rgb4,
                            F::Etc2Rgba8,
                            F::EacR,
                            F::EacRg,
                            F::Astc4x4,
                            F::Astc6x6,
                            F::Astc8x8,
                        ] {
                            ui.selectable_value(&mut s.format, f, format!("{f:?}"));
                        }
                    });
                ui.end_row();
                if s.format == F::Automatic {
                    ui.label("Compression");
                    egui::ComboBox::from_id_salt("tex-platform-compression")
                        .selected_text(format!("{:?}", s.compression))
                        .show_ui(ui, |ui| {
                            for c in [
                                TextureCompression::None,
                                TextureCompression::LowQuality,
                                TextureCompression::NormalQuality,
                                TextureCompression::HighQuality,
                            ] {
                                ui.selectable_value(&mut s.compression, c, format!("{c:?}"));
                            }
                        });
                    ui.end_row();
                }
            });
    });
    // Untouched tabs don't add entries to the .meta file.
    let default = {
        let mut d = imp.clone();
        d.platform_settings.clear();
        let mut d = d.platform(p);
        d.overridden = false;
        d
    };
    if imp.platform_settings[i] == default {
        imp.platform_settings.remove(i);
    }
}

/// Unity's import settings section for Model and Audio importers: the reflected fields,
/// Apply / Revert (Apply re-imports), and Presets (save these settings, or apply a saved
/// importer Preset).
fn importer_settings<T>(
    st: &mut EditorState,
    engine: &mut Engine,
    ui: &mut egui::Ui,
    path: &str,
    slot: &mut Option<(String, T)>,
    salt: &str,
) where
    T: Reflect + Default + Clone + PartialEq + 'static,
{
    let saved: T = engine.world.resource::<AssetServer>().importer(path);
    if slot.as_ref().is_none_or(|(p, _)| p != path) {
        *slot = Some((path.to_owned(), saved.clone()));
    }
    let Some((_, imp)) = slot.as_mut() else {
        return;
    };
    crate::inspector::reflect_grid(ui, &engine.world, imp, salt);
    let changed = *imp != saved;
    let (mut apply, mut revert) = (false, false);
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        importer_presets(st, engine, ui, path, imp);
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
        *slot = Some((path.to_owned(), saved));
    } else if apply {
        let imp = imp.clone();
        st.status = match engine
            .world
            .resource_mut::<AssetServer>()
            .set_importer(path, &imp)
        {
            Ok(()) => format!("Reimported {path}"),
            Err(e) => e,
        };
    }
}

/// The Presets menu of an importer: save the settings as a Preset, or apply a saved one to
/// the edited settings (Apply then re-imports).
fn importer_presets(
    st: &mut EditorState,
    engine: &mut Engine,
    ui: &mut egui::Ui,
    path: &str,
    imp: &mut dyn Reflect,
) {
    let type_name = imp.type_info().name;
    let presets = engine
        .world
        .resource::<AssetServer>()
        .presets_for(type_name);
    let (mut save_preset, mut use_preset) = (false, None);
    ui.menu_button("Presets", |ui| {
        if ui.button("Save current to…").clicked() {
            save_preset = true;
            ui.close();
        }
        if !presets.is_empty() {
            ui.separator();
        }
        for p in &presets {
            if ui.button(p.rsplit('/').next().unwrap_or(p)).clicked() {
                use_preset = Some(p.clone());
                ui.close();
            }
        }
    })
    .response
    .on_hover_text("Save these settings as a Preset, or apply one");
    if let Some(p) = use_preset {
        let assets = engine.world.resource_mut::<AssetServer>();
        let h = oxide_assets::Handle::<oxide_assets::Preset>::from_path(p.as_str());
        match assets.data_asset(&h).cloned() {
            Some(preset) => {
                let skipped = preset.apply(imp);
                st.status = if skipped.is_empty() {
                    format!("Applied {p} (press Apply to re-import)")
                } else {
                    format!("Applied {p}; skipped {}", skipped.join(", "))
                };
            }
            None => st.status = format!("cannot load {p}"),
        }
    }
    if save_preset {
        let folder = path.rsplit_once('/').map_or("Assets", |(d, _)| d);
        let stem = path.rsplit('/').next().unwrap_or(path);
        let root = engine.world.resource::<AssetServer>().full_path("");
        let new =
            crate::asset_ops::unique_path(&root, &format!("{folder}/{stem} {type_name}.preset"));
        let preset = oxide_assets::Preset::of(&*imp);
        match engine
            .world
            .resource_mut::<AssetServer>()
            .save_data(&new, Box::new(preset))
        {
            Ok(()) => {
                st.status = format!("Saved {new}");
                st.project.invalidate();
            }
            Err(e) => st.status = e,
        }
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

/// A Preset: its saved values shown with the target component's own fields.
fn preset(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui, path: &str) {
    let h = oxide_assets::Handle::<oxide_assets::Preset>::from_path(path);
    let Some(p) = engine
        .world
        .resource_mut::<AssetServer>()
        .data_asset(&h)
        .cloned()
    else {
        ui.colored_label(
            Color32::from_rgb(230, 120, 100),
            format!("cannot load {path}"),
        );
        return;
    };
    ui.label(format!(
        "Preset for {}",
        crate::inspector::nicify(&p.target_type)
    ));
    let Some(mut c) = engine.world.registry.create(&p.target_type) else {
        ui.weak(format!(
            "No component type `{}` is registered.",
            p.target_type
        ));
        return;
    };
    p.apply(&mut *c);
    ui.add_space(4.0);
    if crate::inspector::reflect_grid(ui, &engine.world, &mut *c, &format!("preset:{path}")) {
        let edited = oxide_assets::Preset::of(&*c);
        if let Err(e) = engine
            .world
            .resource_mut::<AssetServer>()
            .save_data(path, Box::new(edited))
        {
            st.status = e;
        }
    }
    ui.add_space(4.0);
    ui.weak("Apply it from a component's menu (right-click its header) > Apply Preset.");
}

/// A Sprite Atlas: its settings, Pack Preview and the packed texture.
fn sprite_atlas(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui, path: &str) {
    data_asset(st, engine, ui, path);
    ui.add_space(4.0);
    if ui.button("Pack Preview").clicked() {
        st.status = match engine
            .world
            .resource_mut::<AssetServer>()
            .pack_sprite_atlas(path)
        {
            Ok(r) => format!(
                "Packed {} sprite(s) into {}×{}",
                r.sprites, r.width, r.height
            ),
            Err(e) => e,
        };
    }
    let assets = engine.world.resource::<AssetServer>();
    let tex_path = format!("{path}#Texture");
    let Some(h) = assets.textures.handle_of(&tex_path) else {
        ui.weak("Not packed yet.");
        return;
    };
    let version = assets.textures.version(h.id()).unwrap_or(0);
    let Some(t) = assets.textures.get(&h) else {
        return;
    };
    let packed = assets
        .atlas_slots
        .values()
        .filter(|s| s.atlas == path)
        .count();
    ui.weak(format!("{packed} sprite(s), {}×{}", t.width, t.height));
    let stale = st
        .asset_inspector
        .atlas_preview
        .as_ref()
        .is_none_or(|(p, v, _)| p != path || *v != version);
    if stale && let Some(px) = t.mips.first() {
        let img =
            egui::ColorImage::from_rgba_unmultiplied([t.width as usize, t.height as usize], px);
        let handle =
            ui.ctx()
                .load_texture(format!("atlas:{path}"), img, egui::TextureOptions::NEAREST);
        st.asset_inspector.atlas_preview = Some((path.to_owned(), version, handle));
    }
    if let Some((_, _, img)) = &st.asset_inspector.atlas_preview {
        let w = crate::inspector::avail(ui).min(256.0);
        let h = w * t.height as f32 / t.width.max(1) as f32;
        ui.add(egui::Image::new((img.id(), egui::vec2(w, h))));
    }
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
    let mut changed = shader_picker(st, engine, ui, &mut m);
    if m.shader_graph.is_some() {
        changed |= graph_properties(engine, ui, &mut m);
        ui.collapsing("Advanced Options", |ui| {
            changed |= crate::inspector::reflect_grid_where(
                ui,
                &engine.world,
                &mut m,
                "material-adv",
                &|f| matches!(f, "sorting_priority" | "render_queue" | "name"),
            );
        });
    } else {
        // Shader Graph fields only apply to graph materials; each built-in shader shows only
        // its own inputs (Unity's per-shader material GUI).
        use oxide_assets::{ShaderKind as K, WorkflowMode as Wm};
        let (kind, workflow) = (m.shader, m.workflow_mode);
        changed |=
            crate::inspector::reflect_grid_where(ui, &engine.world, &mut m, "material", &|f| {
                // Skybox shaders: Tint, Exposure, Rotation and their textures only.
                let sky_field = matches!(
                    f,
                    "exposure"
                        | "rotation"
                        | "cubemap"
                        | "front_tex"
                        | "back_tex"
                        | "left_tex"
                        | "right_tex"
                        | "up_tex"
                        | "down_tex"
                );
                if kind.is_skybox() {
                    return f == "name"
                        || match kind {
                            K::SkyboxProcedural => false,
                            K::SkyboxCubemap => {
                                matches!(f, "base_color" | "exposure" | "rotation" | "cubemap")
                            }
                            K::SkyboxPanoramic => {
                                matches!(f, "base_color" | "exposure" | "rotation" | "base_map")
                            }
                            _ => f == "base_color" || (sky_field && f != "cubemap"),
                        };
                }
                if sky_field {
                    return false;
                }
                let shown_for_kind = match f {
                    "workflow_mode" => kind == K::Lit,
                    "metallic" => kind == K::Lit && workflow == Wm::Metallic,
                    "specular_color" => {
                        kind == K::SimpleLit || (kind == K::Lit && workflow == Wm::Specular)
                    }
                    "smoothness" | "metallic_gloss_map" => matches!(kind, K::Lit | K::SimpleLit),
                    "normal_map" | "normal_scale" => kind != K::Unlit,
                    "occlusion_map" | "occlusion_strength" => kind == K::Lit,
                    _ => true,
                };
                // Built-in shaders set their keywords from the material's maps and options.
                shown_for_kind
                    && !matches!(f, "shader" | "shader_graph" | "properties" | "keywords")
            });
    }
    if changed {
        for p in m.properties.iter().filter_map(|p| p.texture.as_ref()) {
            if let Err(e) = engine
                .world
                .resource_mut::<AssetServer>()
                .ensure_loaded(p.path())
            {
                oxide_core::debug_log_warning!("{e}");
            }
        }
    }
    if changed {
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

/// Unity's Assembly Definition Inspector: General options, references, platforms and
/// define constraints, applied to the `.asmdef` with Apply.
fn asmdef(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui, path: &str) {
    use crate::assemblies::{AsmDef, PLATFORMS};
    let project = engine.world.resource::<AssetServer>().full_path("");
    if st
        .asset_inspector
        .asmdef
        .as_ref()
        .is_none_or(|(p, _, _)| p != path)
    {
        let text = std::fs::read_to_string(project.join(path)).unwrap_or_default();
        match serde_json::from_str::<AsmDef>(&text) {
            Ok(d) => st.asset_inspector.asmdef = Some((path.to_owned(), d.clone(), d)),
            Err(e) => {
                ui.colored_label(Color32::from_rgb(230, 120, 100), format!("{path}: {e}"));
                return;
            }
        }
    }
    let (assemblies, errors) = crate::assemblies::discover(&project);
    let Some((_, d, original)) = st.asset_inspector.asmdef.as_mut() else {
        return;
    };
    ui.label(RichText::new("General").strong());
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "Name");
        ui.text_edit_singleline(&mut d.name);
    });
    ui.weak(format!("Crate: {}", crate::assemblies::crate_name(&d.name)));
    ui.checkbox(&mut d.allow_unsafe_code, "Allow 'unsafe' Code");
    ui.checkbox(&mut d.auto_referenced, "Auto Referenced");
    ui.checkbox(&mut d.no_engine_references, "No Engine References");
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "Root Namespace");
        ui.text_edit_singleline(&mut d.root_namespace);
    });
    ui.separator();
    ui.label(RichText::new("Define Constraints").strong());
    let mut remove = None;
    for (k, c) in d.define_constraints.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.text_edit_singleline(c);
            if ui.small_button("-").clicked() {
                remove = Some(k);
            }
        });
    }
    if let Some(k) = remove {
        d.define_constraints.remove(k);
    }
    if ui.small_button("+").clicked() {
        d.define_constraints.push(String::new());
    }
    ui.separator();
    ui.label(RichText::new("Assembly Definition References").strong());
    let mut remove = None;
    for (k, r) in d.references.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(r);
            if !assemblies.iter().any(|a| a.def.name == *r) {
                ui.colored_label(Color32::from_rgb(230, 120, 100), "missing");
            }
            if ui.small_button("-").clicked() {
                remove = Some(k);
            }
        });
    }
    if let Some(k) = remove {
        d.references.remove(k);
    }
    let candidates: Vec<String> = assemblies
        .iter()
        .map(|a| a.def.name.clone())
        .filter(|n| *n != d.name && !d.references.contains(n))
        .collect();
    ui.menu_button("+ Add Reference", |ui| {
        for n in candidates {
            if ui.button(&n).clicked() {
                d.references.push(n);
                ui.close();
            }
        }
    });
    ui.separator();
    ui.label(RichText::new("Platforms").strong());
    let mut any = d.include_platforms.is_empty();
    if ui.checkbox(&mut any, "Any Platform").changed() {
        if any {
            d.include_platforms.clear();
        } else {
            d.include_platforms = PLATFORMS.iter().map(|(p, _)| (*p).to_owned()).collect();
            d.exclude_platforms.clear();
        }
    }
    ui.weak(if any {
        "Exclude Platforms"
    } else {
        "Include Platforms"
    });
    for (p, _) in PLATFORMS {
        let list = if any {
            &mut d.exclude_platforms
        } else {
            &mut d.include_platforms
        };
        let mut on = list.iter().any(|x| x == p);
        if ui.checkbox(&mut on, *p).changed() {
            if on {
                list.push((*p).to_owned());
            } else {
                list.retain(|x| x != p);
            }
        }
    }
    ui.separator();
    if let Some(a) = assemblies.iter().find(|a| a.file == path) {
        ui.label(RichText::new(format!("{} script(s)", a.sources.len())).strong());
        for s in &a.sources {
            ui.weak(s.strip_prefix(&project).unwrap_or(s).to_string_lossy());
        }
    }
    for e in errors.iter().filter(|e| e.starts_with(path)) {
        ui.colored_label(Color32::from_rgb(230, 120, 100), e);
    }
    let changed = d != original;
    ui.horizontal(|ui| {
        if ui
            .add_enabled(changed, egui::Button::new("Revert"))
            .clicked()
        {
            *d = original.clone();
        }
        if ui
            .add_enabled(changed, egui::Button::new("Apply"))
            .clicked()
        {
            match serde_json::to_string_pretty(&*d) {
                Ok(text) => match std::fs::write(project.join(path), text) {
                    Ok(()) => {
                        *original = d.clone();
                        st.status = format!("Applied {path}; scripts recompile");
                    }
                    Err(e) => st.status = e.to_string(),
                },
                Err(e) => st.status = e.to_string(),
            }
        }
    });
}

/// An Assembly Definition Reference: which assembly its folder's scripts join.
fn asmref(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui, path: &str) {
    let project = engine.world.resource::<AssetServer>().full_path("");
    let text = std::fs::read_to_string(project.join(path)).unwrap_or_default();
    let mut r: crate::assemblies::AsmRef = serde_json::from_str(&text).unwrap_or_default();
    let (assemblies, _) = crate::assemblies::discover(&project);
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "Assembly Definition");
        egui::ComboBox::from_id_salt("asmref")
            .selected_text(if r.reference.is_empty() {
                "None"
            } else {
                &r.reference
            })
            .show_ui(ui, |ui| {
                for a in &assemblies {
                    if ui
                        .selectable_label(r.reference == a.def.name, &a.def.name)
                        .clicked()
                    {
                        r.reference = a.def.name.clone();
                        if let Ok(t) = serde_json::to_string_pretty(&r)
                            && let Err(e) = std::fs::write(project.join(path), t)
                        {
                            st.status = e.to_string();
                        }
                    }
                }
            });
    });
    ui.weak("Scripts in this folder compile into that assembly.");
}

/// A Shader Graph or Sub Graph: Open Shader Editor, its properties and compile errors.
fn shader_graph_asset(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui, path: &str) {
    use oxide_shadergraph::{ShaderGraph, ShaderSubGraph};
    if ui.button("Open Shader Editor").clicked() {
        match st.shader_graph.open(engine, path) {
            Ok(()) => st.actions.push(Action::OpenTab(crate::Tab::ShaderGraph)),
            Err(e) => st.status = e,
        }
    }
    let assets = engine.world.resource_mut::<AssetServer>();
    if !assets.data.contains(path)
        && let Err(e) = assets.load_data(path)
    {
        ui.colored_label(Color32::from_rgb(230, 120, 100), e);
        return;
    }
    let h = |p: &str| oxide_assets::Handle::<ShaderGraph>::from_path(p.to_owned());
    let (props, extra): (Vec<(String, String)>, Vec<String>) =
        if let Some(g) = assets.data.get(&h(path)) {
            (
                g.properties
                    .iter()
                    .map(|p| (p.name.clone(), p.kind.kind_name().to_owned()))
                    .collect(),
                vec![format!(
                    "{:?} · {:?}{}",
                    g.settings.material,
                    g.settings.surface,
                    if g.settings.alpha_clipping {
                        " · Alpha Clipping"
                    } else {
                        ""
                    }
                )],
            )
        } else if let Some(s) = assets
            .data
            .get(&oxide_assets::Handle::<ShaderSubGraph>::from_path(
                path.to_owned(),
            ))
        {
            (
                s.properties
                    .iter()
                    .map(|p| (p.name.clone(), p.kind.kind_name().to_owned()))
                    .collect(),
                s.outputs
                    .iter()
                    .map(|o| format!("Output {} ({})", o.name, o.kind.kind_name()))
                    .collect(),
            )
        } else {
            return;
        };
    for l in extra {
        ui.weak(l);
    }
    ui.label(RichText::new("Properties").strong());
    if props.is_empty() {
        ui.weak("None");
    }
    for (name, kind) in props {
        ui.horizontal(|ui| {
            crate::inspector::prop_label(ui, name);
            ui.weak(kind);
        });
    }
    if let Some(s) = assets
        .shaders
        .handle_of(path)
        .and_then(|h| assets.shaders.get(&h))
        .and_then(|s| s.surface.as_ref())
    {
        for (n, e) in &s.errors {
            ui.colored_label(Color32::from_rgb(240, 110, 100), format!("Node {n}: {e}"));
        }
    }
}

/// Unity's Shader dropdown: the built-in shaders and every Shader Graph in the project.
fn shader_picker(
    st: &mut EditorState,
    engine: &mut Engine,
    ui: &mut egui::Ui,
    m: &mut oxide_assets::Material,
) -> bool {
    use oxide_assets::ShaderKind;
    let graphs = engine
        .world
        .resource::<AssetServer>()
        .paths_of_type("ShaderGraph");
    let stem = |p: &str| {
        let n = p.rsplit('/').next().unwrap_or(p);
        n.split('.').next().unwrap_or(n).to_owned()
    };
    let current = match (&m.shader_graph, m.shader) {
        (Some(g), _) => format!("Shader Graphs/{}", stem(g.path())),
        (None, ShaderKind::Unlit) => "Universal Render Pipeline/Unlit".into(),
        (None, ShaderKind::SimpleLit) => "Universal Render Pipeline/Simple Lit".into(),
        (None, ShaderKind::BakedLit) => "Universal Render Pipeline/Baked Lit".into(),
        (None, ShaderKind::Lit) => "Universal Render Pipeline/Lit".into(),
        (None, ShaderKind::SkyboxProcedural) => "Skybox/Procedural".into(),
        (None, ShaderKind::Skybox6Sided) => "Skybox/6 Sided".into(),
        (None, ShaderKind::SkyboxCubemap) => "Skybox/Cubemap".into(),
        (None, ShaderKind::SkyboxPanoramic) => "Skybox/Panoramic".into(),
    };
    let mut changed = false;
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "Shader");
        egui::ComboBox::from_id_salt("material-shader")
            .selected_text(&current)
            .width(crate::inspector::avail(ui) - 50.0)
            .show_ui(ui, |ui| {
                for (label, kind) in [
                    ("Universal Render Pipeline/Lit", ShaderKind::Lit),
                    (
                        "Universal Render Pipeline/Simple Lit",
                        ShaderKind::SimpleLit,
                    ),
                    ("Universal Render Pipeline/Baked Lit", ShaderKind::BakedLit),
                    ("Universal Render Pipeline/Unlit", ShaderKind::Unlit),
                    ("Skybox/Procedural", ShaderKind::SkyboxProcedural),
                    ("Skybox/6 Sided", ShaderKind::Skybox6Sided),
                    ("Skybox/Cubemap", ShaderKind::SkyboxCubemap),
                    ("Skybox/Panoramic", ShaderKind::SkyboxPanoramic),
                ] {
                    if ui.selectable_label(current == label, label).clicked() {
                        m.shader = kind;
                        m.shader_graph = None;
                        changed = true;
                    }
                }
                for g in &graphs {
                    let label = format!("Shader Graphs/{}", stem(g));
                    if ui
                        .selectable_label(current == label, &label)
                        .on_hover_text(g)
                        .clicked()
                    {
                        m.shader_graph = Some(oxide_assets::Handle::from_path(g.clone()));
                        changed = true;
                    }
                }
            });
        if let Some(g) = m.shader_graph.clone()
            && ui.button("Edit...").clicked()
        {
            match st.shader_graph.open(engine, g.path()) {
                Ok(()) => st.actions.push(Action::OpenTab(crate::Tab::ShaderGraph)),
                Err(e) => st.status = e,
            }
        }
    });
    if changed && let Some(g) = m.shader_graph.clone() {
        let assets = engine.world.resource_mut::<AssetServer>();
        if let Err(e) = assets.ensure_loaded(g.path()) {
            st.status = e;
        }
        oxide_shadergraph::compile_asset(assets, g.path());
        if let Some(graph) = assets.data.get(
            &oxide_assets::Handle::<oxide_shadergraph::ShaderGraph>::from_path(g.path().to_owned()),
        ) {
            graph.apply_default_keywords(m);
        }
    }
    changed
}

/// A Shader Graph material's exposed properties (values saved on the material).
fn graph_properties(
    engine: &mut Engine,
    ui: &mut egui::Ui,
    m: &mut oxide_assets::Material,
) -> bool {
    use oxide_shadergraph::{FloatMode, PropertyType, ShaderGraph};
    let Some(gh) = m.shader_graph.clone() else {
        return false;
    };
    let assets = engine.world.resource_mut::<AssetServer>();
    if !assets.data.contains(gh.path())
        && let Err(e) = assets.load_data(gh.path())
    {
        ui.colored_label(Color32::from_rgb(230, 120, 100), e);
        return false;
    }
    let Some(graph) = assets
        .data
        .get(&oxide_assets::Handle::<ShaderGraph>::from_path(
            gh.path().to_owned(),
        ))
        .cloned()
    else {
        return false;
    };
    let mut changed = false;
    ui.label(RichText::new("Surface Inputs").strong());
    for p in graph.properties.iter().filter(|p| p.exposed) {
        let cur = m.property(&p.reference).cloned();
        let mut v = cur.as_ref().map_or(p.default, |c| c.value).to_array();
        ui.horizontal(|ui| {
            crate::inspector::prop_label(ui, &p.name).on_hover_text(&p.reference);
            let w = crate::inspector::avail(ui);
            match p.kind {
                PropertyType::Float => {
                    let r = match p.float_mode {
                        FloatMode::Slider => ui.add_sized(
                            [w, 18.0],
                            egui::Slider::new(&mut v[0], p.range.x..=p.range.y),
                        ),
                        FloatMode::Integer => {
                            let mut i = v[0].round() as i64;
                            let r = ui.add(egui::DragValue::new(&mut i));
                            v[0] = i as f32;
                            r
                        }
                        FloatMode::Default => ui.add(egui::DragValue::new(&mut v[0]).speed(0.01)),
                    };
                    if r.changed() {
                        m.set_float(&p.reference, v[0]);
                        changed = true;
                    }
                }
                PropertyType::Vector2 | PropertyType::Vector3 | PropertyType::Vector4 => {
                    let n = match p.kind {
                        PropertyType::Vector2 => 2,
                        PropertyType::Vector3 => 3,
                        _ => 4,
                    };
                    let mut any = false;
                    for x in v.iter_mut().take(n) {
                        any |= ui.add(egui::DragValue::new(x).speed(0.01)).changed();
                    }
                    if any {
                        m.set_vector(&p.reference, oxide_math::Vector4::from_array(v));
                        changed = true;
                    }
                }
                PropertyType::Color => {
                    let mut c = egui::Color32::from_rgba_unmultiplied(
                        (v[0].clamp(0.0, 1.0) * 255.0) as u8,
                        (v[1].clamp(0.0, 1.0) * 255.0) as u8,
                        (v[2].clamp(0.0, 1.0) * 255.0) as u8,
                        (v[3].clamp(0.0, 1.0) * 255.0) as u8,
                    );
                    if egui::color_picker::color_edit_button_srgba(
                        ui,
                        &mut c,
                        egui::color_picker::Alpha::OnlyBlend,
                    )
                    .changed()
                    {
                        let [r, g, b, a] = c.to_srgba_unmultiplied();
                        let k = if p.hdr {
                            v[0].max(v[1]).max(v[2]).max(1.0)
                        } else {
                            1.0
                        };
                        m.set_vector(
                            &p.reference,
                            oxide_math::Vector4::new(
                                r as f32 / 255.0 * k,
                                g as f32 / 255.0 * k,
                                b as f32 / 255.0 * k,
                                a as f32 / 255.0,
                            ),
                        );
                        changed = true;
                    }
                }
                PropertyType::Boolean => {
                    let mut b = v[0] > 0.5;
                    if ui.checkbox(&mut b, "").changed() {
                        m.set_float(&p.reference, b as u32 as f32);
                        changed = true;
                    }
                }
                PropertyType::Texture2D => {
                    let tex = cur.and_then(|c| c.texture).or(p.default_texture.clone());
                    let mut val = match &tex {
                        Some(t) => oxide_core::reflect::Value::Asset(t.path().to_owned()),
                        None => oxide_core::reflect::Value::Null,
                    };
                    if crate::inspector::asset_field(
                        ui,
                        &engine.world,
                        Some("Texture"),
                        &mut val,
                        &format!("mat-tex-{}", p.reference),
                    ) {
                        let t = match val {
                            oxide_core::reflect::Value::Asset(path) => {
                                Some(oxide_assets::Handle::from_path(path))
                            }
                            _ => None,
                        };
                        m.set_texture(&p.reference, t);
                        changed = true;
                    }
                }
            }
        });
    }
    // Local keywords (Global ones are set from scripts with `Shader.EnableKeyword`).
    for k in graph
        .keywords
        .iter()
        .filter(|k| k.exposed && k.scope == oxide_shadergraph::KeywordScope::Local)
    {
        let mut state = k.state_on(m);
        ui.horizontal(|ui| {
            crate::inspector::prop_label(ui, &k.name).on_hover_text(&k.reference);
            let before = state;
            match k.keyword_type {
                oxide_shadergraph::KeywordType::Boolean => {
                    let mut b = state != 0;
                    ui.checkbox(&mut b, "");
                    state = b as u32;
                }
                oxide_shadergraph::KeywordType::Enum => {
                    egui::ComboBox::from_id_salt(("mat-kw", &k.reference))
                        .selected_text(k.entries.get(state as usize).map_or("", |e| e.as_str()))
                        .show_ui(ui, |ui| {
                            for (i, e) in k.entries.iter().enumerate() {
                                ui.selectable_value(&mut state, i as u32, e);
                            }
                        });
                }
            }
            if state != before {
                m.set_keyword_state(&k.reference, &k.entries, state);
                changed = true;
            }
        });
    }
    if m.properties.iter().any(|v| {
        !graph.properties.iter().any(|p| p.reference == v.name)
            && !graph.keywords.iter().any(|k| k.reference == v.name)
    }) && ui
        .small_button("Remove values of deleted properties")
        .clicked()
    {
        let keep: Vec<String> = graph
            .properties
            .iter()
            .map(|p| p.reference.clone())
            .chain(graph.keywords.iter().map(|k| k.reference.clone()))
            .collect();
        m.properties.retain(|v| keep.contains(&v.name));
        changed = true;
    }
    changed
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
    let has_clips = !m.animations.is_empty();
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui.button("Add to Scene").clicked() {
            st.actions.push(Action::InstantiateModel(path.to_owned()));
        }
        if has_clips
            && ui
                .button("Extract Animations")
                .on_hover_text("Copy the clips to editable .anim files beside the model")
                .clicked()
        {
            st.status = match crate::anim_inspectors::extract_model_clips(engine, path) {
                Ok(v) => {
                    st.project.invalidate();
                    format!("Extracted {} clip(s)", v.len())
                }
                Err(e) => e,
            };
        }
    });
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
