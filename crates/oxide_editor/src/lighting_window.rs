//! Window → Rendering → Lighting (Unity's Lighting window: Scene, Environment and Baked
//! Lightmaps tabs, Generate Lighting) and Window → Rendering → Light Explorer (tables of the
//! scene's lights, reflection probes, light probe groups and static emissive objects).
//! References: <https://docs.unity3d.com/Manual/lighting-window.html>,
//! <https://docs.unity3d.com/Manual/LightExplorer.html>

use crate::{Action, EditorState};
use oxide_assets::{AssetId, AssetServer, Handle};
use oxide_engine::Engine;
use oxide_scene::{
    GameObjectId, Light, LightProbeGroup, LightingSettings, ReflectionProbe, RenderSettings,
    StaticEditorFlags, World,
};
use oxide_ui::egui::{self, Color32, RichText};
use std::collections::HashMap;

#[derive(Default)]
pub struct LightingWindow {
    tab: usize,
    explorer_tab: usize,
    /// Baked Lightmaps previews: (texture version, image) by texture.
    previews: HashMap<AssetId, (u64, egui::TextureHandle)>,
}

/// RenderSettings fields shown on the Scene tab when there is no Lighting Settings asset.
const SCENE_FIELDS: [&str; 8] = [
    "lighting_mode",
    "directional_mode",
    "realtime_shadow_color",
    "lightmap_resolution",
    "max_lightmap_size",
    "bake_samples",
    "bake_bounces",
    "lightmap_padding",
];

/// RenderSettings fields on the Environment tab.
const ENVIRONMENT_FIELDS: [&str; 15] = [
    "skybox_material",
    "sun",
    "sky_tint",
    "ground_color",
    "sky_exposure",
    "sun_size",
    "atmosphere_thickness",
    "ambient_intensity",
    "reflection_intensity",
    "fog",
    "fog_mode",
    "fog_color",
    "fog_density",
    "fog_start_distance",
    "fog_end_distance",
];

pub fn window_ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    ui.horizontal_wrapped(|ui| {
        for (i, name) in ["Scene", "Environment", "Baked Lightmaps"]
            .iter()
            .enumerate()
        {
            ui.selectable_value(&mut st.lighting_window.tab, i, *name);
        }
    });
    ui.separator();
    egui::ScrollArea::vertical().show(ui, |ui| match st.lighting_window.tab {
        0 => scene_tab(st, engine, ui),
        1 => {
            render_settings(st, engine, ui, &ENVIRONMENT_FIELDS, "lighting-env");
        }
        _ => baked_tab(st, engine, ui),
    });
    ui.separator();
    bake_bar(st, engine, ui);
}

/// Edits some RenderSettings fields (one undo step per change).
fn render_settings(
    st: &mut EditorState,
    engine: &mut Engine,
    ui: &mut egui::Ui,
    fields: &[&str],
    salt: &str,
) -> bool {
    let w = &mut engine.world;
    let mut rs = w.resource::<RenderSettings>().clone();
    let changed =
        crate::inspector::reflect_grid_where(ui, w, &mut rs, salt, &|f| fields.contains(&f));
    if changed {
        let now = ui.input(|i| i.time);
        st.record(w, "Lighting Settings", Some(salt.len() as u64), now);
        *w.resource_mut::<RenderSettings>() = rs;
    }
    changed
}

fn scene_tab(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    ui.label(RichText::new("Lighting Settings").strong());
    render_settings(st, engine, ui, &["lighting_settings"], "lighting-asset");
    let asset = engine
        .world
        .resource::<RenderSettings>()
        .lighting_settings
        .clone();
    match asset {
        Some(h) => {
            let assets = engine.world.resource_mut::<AssetServer>();
            let Some(mut s) = assets.data_asset(&h).cloned() else {
                ui.colored_label(Color32::from_rgb(230, 120, 100), "Can't load the asset");
                return;
            };
            ui.weak(format!("Shared settings in {}", h.path()));
            if crate::inspector::reflect_grid(ui, &engine.world, &mut s, "lighting-settings") {
                let assets = engine.world.resource_mut::<AssetServer>();
                if let Some(v) = assets.data.get_mut(&h) {
                    *v = s;
                }
                if let Err(e) = assets.save_data_in_place(h.path()) {
                    st.status = e;
                }
            }
        }
        None => {
            ui.horizontal_wrapped(|ui| {
                ui.weak("Scene settings (no Lighting Settings asset).");
                if ui
                    .button("New Lighting Settings")
                    .on_hover_text("Save these settings as a .lighting asset shared by scenes")
                    .clicked()
                {
                    match new_lighting_settings(&mut engine.world) {
                        Ok(p) => st.status = format!("Created {p}"),
                        Err(e) => st.status = e,
                    }
                }
            });
            ui.add_space(4.0);
            ui.label(RichText::new("Mixed Lighting and Lightmapping").strong());
            render_settings(st, engine, ui, &SCENE_FIELDS, "lighting-scene");
        }
    }
}

/// Makes a Lighting Settings asset from the scene's values and uses it.
fn new_lighting_settings(w: &mut World) -> Result<String, String> {
    let rs = w.resource::<RenderSettings>().clone();
    let folder = w
        .scene_info(oxide_scene::SceneHandle::MAIN)
        .and_then(|s| s.path)
        .and_then(|p| p.rsplit_once('/').map(|(d, _)| d.to_owned()))
        .filter(|d| d.starts_with("Assets"))
        .unwrap_or_else(|| "Assets".into());
    let assets = w.resource_mut::<AssetServer>();
    let mut path = format!("{folder}/LightingSettings.lighting");
    let mut n = 1;
    while assets.full_path(&path).exists() {
        path = format!("{folder}/LightingSettings {n}.lighting");
        n += 1;
    }
    let settings = LightingSettings {
        lightmap_resolution: rs.lightmap_resolution,
        max_lightmap_size: rs.max_lightmap_size,
        indirect_samples: rs.bake_samples,
        max_bounces: rs.bake_bounces,
        lightmap_padding: rs.lightmap_padding,
        lighting_mode: rs.lighting_mode,
        directional_mode: rs.directional_mode,
        realtime_shadow_color: rs.realtime_shadow_color,
        ..Default::default()
    };
    assets.save_data(&path, Box::new(settings))?;
    w.resource_mut::<RenderSettings>().lighting_settings = Some(Handle::from_path(path.clone()));
    Ok(path)
}

fn baked_tab(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    let rs = engine.world.resource::<RenderSettings>().clone();
    if rs.lightmaps.is_empty() {
        ui.weak("No baked lightmaps. Mark objects Static (Contribute GI) and Generate Lighting.");
    }
    ui.label(format!(
        "Lighting Mode: {:?}   Directional: {}   Light probes: {}",
        rs.baked_lighting_mode,
        if rs.lightmap_directions.is_empty() {
            "no"
        } else {
            "yes"
        },
        rs.light_probe_positions.len()
    ));
    let ctx = ui.ctx().clone();
    let assets = engine.world.resource_mut::<AssetServer>();
    for (i, lm) in rs.lightmaps.iter().enumerate() {
        ui.add_space(6.0);
        ui.label(RichText::new(format!("Lightmap {i}")).strong())
            .on_hover_text(lm.path());
        ui.horizontal(|ui| {
            let maps = [
                Some((lm, true)),
                rs.lightmap_directions.get(i).map(|d| (d, false)),
                rs.shadowmasks.get(i).map(|m| (m, false)),
            ];
            for (h, rgbm) in maps.into_iter().flatten() {
                if assets.textures.handle_of(h.path()).is_none() {
                    let _ = assets.load_texture(h.path());
                }
                let id = h.id();
                let Some(t) = assets.textures.get_by_id(id).filter(|t| !t.mips.is_empty()) else {
                    continue;
                };
                let version = assets.textures.version(id).unwrap_or(0);
                let entry = st.lighting_window.previews.get(&id);
                let tex = match entry {
                    Some((v, tex)) if *v == version => tex.clone(),
                    _ => {
                        // RGBM lightmaps shown decoded; masks and directions as they are.
                        let px: Vec<u8> = t.mips[0]
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .flat_map(|p| {
                                if rgbm {
                                    let s = p[3] as f32 / 255.0 * 2.828;
                                    p[..3]
                                        .iter()
                                        .map(|&c| (c as f32 * s).min(255.0) as u8)
                                        .chain([255])
                                        .collect::<Vec<_>>()
                                } else {
                                    vec![p[0], p[1], p[2], 255]
                                }
                            })
                            .collect();
                        let img = egui::ColorImage::from_rgba_unmultiplied(
                            [t.width as usize, t.height as usize],
                            &px,
                        );
                        let tex = ctx.load_texture(h.path(), img, egui::TextureOptions::LINEAR);
                        st.lighting_window
                            .previews
                            .insert(id, (version, tex.clone()));
                        tex
                    }
                };
                ui.vertical(|ui| {
                    ui.add(egui::Image::new((tex.id(), egui::vec2(128.0, 128.0))));
                    let name = h.path().rsplit('/').next().unwrap_or(h.path());
                    ui.weak(format!("{name}\n{}×{}", t.width, t.height));
                });
            }
        });
    }
}

/// Generate Lighting / Clear Baked Data and the bake's progress.
fn bake_bar(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    let status = oxide_engine::lighting::status(&engine.world);
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(!status.running, egui::Button::new("Generate Lighting"))
            .clicked()
        {
            st.actions.push(Action::GenerateLighting);
        }
        if ui.button("Clear Baked Data").clicked() {
            st.actions.push(Action::ClearBakedLighting);
        }
    });
    ui.horizontal_wrapped(|ui| {
        if status.running {
            ui.add(
                egui::ProgressBar::new(status.progress)
                    .desired_width(160.0)
                    .text(format!("Baking… {:.0} s", status.seconds)),
            );
            if ui.button("Cancel").clicked() {
                oxide_engine::lighting::cancel(&mut engine.world);
            }
        } else {
            if let Some(e) = &status.error {
                ui.colored_label(Color32::from_rgb(230, 120, 100), e);
            } else if let Some(m) = &status.last {
                ui.weak(m);
            }
        }
    });
}

// ---- Light Explorer ----------------------------------------------------------------------

pub fn explorer_ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    ui.horizontal_wrapped(|ui| {
        for (i, name) in [
            "Lights",
            "Reflection Probes",
            "Light Probes",
            "Static Emissives",
        ]
        .iter()
        .enumerate()
        {
            ui.selectable_value(&mut st.lighting_window.explorer_tab, i, *name);
        }
    });
    ui.separator();
    egui::ScrollArea::both().show(ui, |ui| match st.lighting_window.explorer_tab {
        0 => lights_table(st, engine, ui),
        1 => probes_table(st, engine, ui),
        2 => light_probes_table(st, engine, ui),
        _ => emissives_table(st, engine, ui),
    });
}

/// The object's name as a button that selects it.
fn name_cell(st: &mut EditorState, w: &World, ui: &mut egui::Ui, id: GameObjectId) {
    let name = w.get(id).map_or("?", |o| o.name.as_str()).to_owned();
    let selected = st.selection.contains(&id);
    if ui.selectable_label(selected, name).clicked() {
        st.select(id);
    }
}

/// An enabled checkbox for component `T` of `id`.
fn enabled_cell<T: oxide_scene::Component>(
    st: &mut EditorState,
    w: &mut World,
    ui: &mut egui::Ui,
    id: GameObjectId,
) {
    let Some(cid) = w.component_id::<T>(id) else {
        ui.label("");
        return;
    };
    let mut on = w.is_component_enabled(id, cid);
    if ui.checkbox(&mut on, "").changed() {
        let now = ui.input(|i| i.time);
        st.record(w, "Toggle Component", None, now);
        w.set_component_enabled(id, cid, on);
    }
}

/// Every object with component `T` (enabled or not), in hierarchy order.
fn with_component<T: oxide_scene::Component>(w: &World) -> Vec<GameObjectId> {
    w.hierarchy_order()
        .into_iter()
        .filter(|&id| w.component_id::<T>(id).is_some())
        .collect()
}

fn lights_table(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    use oxide_scene::{LightShadows, LightType, LightmapBakeType};
    let ids = with_component::<Light>(&engine.world);
    if ids.is_empty() {
        ui.weak("No lights in the scene.");
        return;
    }
    egui::Grid::new("light-explorer")
        .striped(true)
        .num_columns(8)
        .spacing([10.0, 4.0])
        .show(ui, |ui| {
            for h in [
                "On",
                "Name",
                "Type",
                "Mode",
                "Color",
                "Intensity",
                "Range",
                "Shadows",
            ] {
                ui.label(RichText::new(h).strong());
            }
            ui.end_row();
            for id in ids {
                let w = &mut engine.world;
                enabled_cell::<Light>(st, w, ui, id);
                name_cell(st, w, ui, id);
                let Some(mut l) = w.get_component::<Light>(id).cloned() else {
                    ui.end_row();
                    continue;
                };
                let before = l.clone();
                combo(
                    ui,
                    ("lt", id),
                    &mut l.light_type,
                    &[
                        LightType::Directional,
                        LightType::Point,
                        LightType::Spot,
                        LightType::Rectangle,
                        LightType::Disc,
                    ],
                );
                combo(
                    ui,
                    ("lm", id),
                    &mut l.lightmap_bake_type,
                    &[
                        LightmapBakeType::Realtime,
                        LightmapBakeType::Mixed,
                        LightmapBakeType::Baked,
                    ],
                );
                let mut c = [l.color.r, l.color.g, l.color.b];
                if ui.color_edit_button_rgb(&mut c).changed() {
                    l.color = oxide_math::Color::new(c[0], c[1], c[2], l.color.a);
                }
                ui.add(
                    egui::DragValue::new(&mut l.intensity)
                        .speed(0.01)
                        .range(0.0..=1000.0),
                );
                ui.add_enabled(
                    l.light_type != LightType::Directional,
                    egui::DragValue::new(&mut l.range)
                        .speed(0.1)
                        .range(0.0..=10000.0),
                );
                combo(
                    ui,
                    ("sh", id),
                    &mut l.shadows,
                    &[LightShadows::None, LightShadows::Hard, LightShadows::Soft],
                );
                ui.end_row();
                if !lights_equal(&l, &before)
                    && let Some(dst) = w.get_component_mut::<Light>(id)
                {
                    *dst = l;
                    let now = ui.input(|i| i.time);
                    st.record(w, "Edit Light", Some(id.to_bits()), now);
                }
            }
        });
}

fn lights_equal(a: &Light, b: &Light) -> bool {
    (a.light_type, a.lightmap_bake_type, a.shadows)
        == (b.light_type, b.lightmap_bake_type, b.shadows)
        && a.color == b.color
        && a.intensity == b.intensity
        && a.range == b.range
}

/// A dropdown of `choices` (shown by their Debug names).
fn combo<T: Copy + PartialEq + std::fmt::Debug>(
    ui: &mut egui::Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    value: &mut T,
    choices: &[T],
) {
    egui::ComboBox::from_id_salt(salt)
        .selected_text(format!("{value:?}"))
        .width(90.0)
        .show_ui(ui, |ui| {
            for c in choices {
                ui.selectable_value(value, *c, format!("{c:?}"));
            }
        });
}

fn probes_table(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    use oxide_scene::ReflectionProbeMode;
    let ids = with_component::<ReflectionProbe>(&engine.world);
    if ids.is_empty() {
        ui.weak("No reflection probes in the scene.");
        return;
    }
    egui::Grid::new("probe-explorer")
        .striped(true)
        .num_columns(7)
        .spacing([10.0, 4.0])
        .show(ui, |ui| {
            for h in [
                "On",
                "Name",
                "Mode",
                "Importance",
                "Intensity",
                "Box Projection",
                "Baked",
            ] {
                ui.label(RichText::new(h).strong());
            }
            ui.end_row();
            for id in ids {
                let w = &mut engine.world;
                enabled_cell::<ReflectionProbe>(st, w, ui, id);
                name_cell(st, w, ui, id);
                let Some(mut p) = w.get_component::<ReflectionProbe>(id).cloned() else {
                    ui.end_row();
                    continue;
                };
                let before = (p.mode, p.importance, p.intensity, p.box_projection);
                combo(
                    ui,
                    ("pm", id),
                    &mut p.mode,
                    &[
                        ReflectionProbeMode::Realtime,
                        ReflectionProbeMode::Baked,
                        ReflectionProbeMode::Custom,
                    ],
                );
                ui.add(egui::DragValue::new(&mut p.importance).range(0..=1000));
                ui.add(
                    egui::DragValue::new(&mut p.intensity)
                        .speed(0.01)
                        .range(0.0..=4.0),
                );
                ui.checkbox(&mut p.box_projection, "");
                ui.weak(match (&p.mode, &p.baked_texture, &p.custom_baked_texture) {
                    (ReflectionProbeMode::Baked, Some(t), _)
                    | (ReflectionProbeMode::Custom, _, Some(t)) => {
                        t.path().rsplit('/').next().unwrap_or("").to_owned()
                    }
                    (ReflectionProbeMode::Baked, None, _) => "not baked".into(),
                    (ReflectionProbeMode::Custom, _, None) => "no cubemap".into(),
                    _ => "realtime".into(),
                });
                ui.end_row();
                if (p.mode, p.importance, p.intensity, p.box_projection) != before
                    && let Some(dst) = w.get_component_mut::<ReflectionProbe>(id)
                {
                    *dst = p;
                    let now = ui.input(|i| i.time);
                    st.record(w, "Edit Reflection Probe", Some(id.to_bits()), now);
                }
            }
        });
}

fn light_probes_table(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    let ids = with_component::<LightProbeGroup>(&engine.world);
    if ids.is_empty() {
        ui.weak("No Light Probe Groups in the scene.");
        return;
    }
    let baked = engine
        .world
        .resource::<RenderSettings>()
        .light_probe_positions
        .len();
    ui.weak(format!("{baked} probes baked"));
    egui::Grid::new("lightprobe-explorer")
        .striped(true)
        .num_columns(3)
        .spacing([10.0, 4.0])
        .show(ui, |ui| {
            for h in ["On", "Name", "Probes"] {
                ui.label(RichText::new(h).strong());
            }
            ui.end_row();
            for id in ids {
                let w = &mut engine.world;
                enabled_cell::<LightProbeGroup>(st, w, ui, id);
                name_cell(st, w, ui, id);
                let n = w
                    .get_component::<LightProbeGroup>(id)
                    .map_or(0, |g| g.probe_positions.len());
                ui.label(n.to_string());
                ui.end_row();
            }
        });
}

/// Static (Contribute GI) renderers whose materials emit light.
fn emissives_table(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    let w = &engine.world;
    let assets = w.resource::<AssetServer>();
    let rows: Vec<(GameObjectId, String, oxide_math::Color, f32)> = w
        .hierarchy_order()
        .into_iter()
        .filter(|&id| {
            w.get(id)
                .is_some_and(|o| o.static_flags.contains(StaticEditorFlags::CONTRIBUTE_GI))
        })
        .filter_map(|id| {
            w.get_component::<oxide_scene::MeshRenderer>(id)
                .map(|r| (id, r))
        })
        .flat_map(|(id, r)| {
            r.materials
                .iter()
                .filter_map(|h| {
                    let m = assets.materials.get(h)?;
                    let lit = m.emission_intensity > 0.0
                        && (m.emission_color.r + m.emission_color.g + m.emission_color.b) > 0.0;
                    lit.then(|| {
                        (
                            id,
                            h.path().to_owned(),
                            m.emission_color,
                            m.emission_intensity,
                        )
                    })
                })
                .collect::<Vec<_>>()
        })
        .collect();
    if rows.is_empty() {
        ui.weak("No static emissive objects (Contribute GI renderers with an emissive material).");
        return;
    }
    egui::Grid::new("emissive-explorer")
        .striped(true)
        .num_columns(4)
        .spacing([10.0, 4.0])
        .show(ui, |ui| {
            for h in ["Name", "Material", "Emission", "Intensity"] {
                ui.label(RichText::new(h).strong());
            }
            ui.end_row();
            for (id, material, color, intensity) in rows {
                name_cell(st, &engine.world, ui, id);
                ui.label(material.rsplit('/').next().unwrap_or(&material).to_owned());
                let c = color;
                let (r, g, b) = (
                    (c.r.clamp(0.0, 1.0) * 255.0) as u8,
                    (c.g.clamp(0.0, 1.0) * 255.0) as u8,
                    (c.b.clamp(0.0, 1.0) * 255.0) as u8,
                );
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(40.0, 14.0), egui::Sense::hover());
                ui.painter()
                    .rect_filled(rect, 2.0, Color32::from_rgb(r, g, b));
                ui.label(format!("{intensity:.2}"));
                ui.end_row();
            }
        });
}
