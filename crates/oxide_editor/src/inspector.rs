//! Inspector window: GameObject header, Transform, and every component's reflected fields,
//! plus Add Component and the per-component context menu.
//! Reference: <https://docs.unity3d.com/Manual/UsingTheInspector.html>

use crate::hierarchy::HierarchyDrag;
use crate::project::AssetPayload;
use crate::{Action, EditorState};
use egui::{Color32, RichText};
use oxide_assets::AssetServer;
use oxide_core::reflect::{FieldInfo, Value, ValueKind};
use oxide_engine::Engine;
use oxide_math::{Quaternion, QuaternionExt, Vector3};
use oxide_scene::{ComponentId, GameObjectId, ObjectProperty, PrefabOverride, World};
use std::collections::{BTreeMap, HashMap};

#[derive(Default)]
pub struct InspectorState {
    /// Euler angles shown for each object's rotation, kept so values like 180 or 370 survive
    /// the round trip through a quaternion (Unity's rotation hint).
    euler: HashMap<GameObjectId, (Quaternion, Vector3)>,
    add_search: String,
    /// Copied component values (Copy/Paste Component Values).
    clipboard: Option<(String, BTreeMap<String, Value>)>,
    /// Overrides of the selected object's prefab instance: (instance root, when computed).
    overrides: Option<(GameObjectId, f64, Vec<PrefabOverride>)>,
}

impl InspectorState {
    pub fn invalidate_overrides(&mut self) {
        self.overrides = None;
    }

    fn overrides(&self) -> &[PrefabOverride] {
        self.overrides.as_ref().map_or(&[], |o| &o.2)
    }
}

/// `field_of_view` / `fieldOfView` → `Field Of View` (Unity `ObjectNames.NicifyVariableName`).
pub fn nicify(name: &str) -> String {
    let mut out = String::new();
    let mut prev_lower = false;
    for (i, ch) in name.chars().enumerate() {
        if ch == '_' {
            if !out.ends_with(' ') && !out.is_empty() {
                out.push(' ');
            }
            prev_lower = false;
            continue;
        }
        if ch.is_uppercase() && prev_lower {
            out.push(' ');
        }
        if i == 0 || out.ends_with(' ') {
            out.extend(ch.to_uppercase());
        } else {
            out.push(ch);
        }
        prev_lower = ch.is_lowercase() || ch.is_ascii_digit();
    }
    out
}

/// An edit to apply after the UI pass (so the world isn't borrowed while drawing).
enum Edit {
    Field(ComponentId, String, Value),
    Enabled(ComponentId, bool),
    Remove(ComponentId),
    Move(ComponentId, i32),
    Paste(ComponentId, BTreeMap<String, Value>),
    Reset(ComponentId),
    Name(String),
    Active(bool),
    Static(oxide_scene::StaticEditorFlags),
    Tag(String),
    Layer(u8),
    Position(Vector3),
    Rotation(Quaternion),
    Scale(Vector3),
    ResetTransform,
}

pub fn ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    let Some(id) = st.active().filter(|&id| engine.world.is_alive(id)) else {
        ui.add_space(8.0);
        ui.weak("Select a GameObject to inspect it.");
        return;
    };
    refresh_overrides(st, &mut engine.world, id);
    let mut edits: Vec<(Edit, u64)> = Vec::new();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let w = &engine.world;
            if st.selection.len() > 1 {
                ui.weak(format!(
                    "{} objects selected; showing the active one.",
                    st.selection.len()
                ));
            }
            if header(ui, w, id, &mut edits) {
                st.actions.push(Action::OpenProjectSettings(
                    crate::settings_window::Page::TagsAndLayers,
                ));
            }
            prefab_header(ui, st, w, id);
            ui.separator();
            transform(ui, st, w, id, &mut edits);
            components(ui, st, w, id, &mut edits);
            ui.add_space(8.0);
            add_component(ui, st, w);
            ui.add_space(24.0);
        });
    apply(st, &mut engine.world, id, edits);
}

fn key(id: GameObjectId, part: &str) -> u64 {
    egui::Id::new((id.to_bits(), part)).value()
}

/// Returns true when "Add Tag..." / "Add Layer..." was clicked (opens Tags and Layers).
fn header(ui: &mut egui::Ui, w: &World, id: GameObjectId, edits: &mut Vec<(Edit, u64)>) -> bool {
    let o = w.get(id).expect("alive");
    let mut open_tags = false;
    ui.horizontal(|ui| {
        let mut active = o.active_self();
        if ui.checkbox(&mut active, "").changed() {
            edits.push((Edit::Active(active), key(id, "active")));
        }
        let mut name = o.name.clone();
        let r = ui
            .add(egui::TextEdit::singleline(&mut name).desired_width(ui.available_width() - 70.0));
        if r.changed() {
            edits.push((Edit::Name(name), key(id, "name")));
        }
        // Unity's Static checkbox (all flags) plus the per-system flags dropdown.
        use oxide_scene::StaticEditorFlags as F;
        let flags = o.static_flags;
        let mut all = flags == F::EVERYTHING;
        let r = ui.add(
            egui::Checkbox::new(&mut all, "Static")
                .indeterminate(flags != F::NOTHING && flags != F::EVERYTHING),
        );
        if r.changed() {
            let f = if all { F::EVERYTHING } else { F::NOTHING };
            edits.push((Edit::Static(f), key(id, "static")));
        }
        ui.menu_button("▾", |ui| {
            for (flag, name) in F::ALL {
                let mut on = flags.contains(flag);
                if ui.checkbox(&mut on, name).changed() {
                    let mut f = flags;
                    f.set(flag, on);
                    edits.push((Edit::Static(f), key(id, "static")));
                }
            }
        })
        .response
        .on_hover_text("Static flags");
    });
    ui.horizontal(|ui| {
        ui.label("Tag");
        let mut tag = o.tag.clone();
        egui::ComboBox::from_id_salt(("tag", id.to_bits()))
            .selected_text(&tag)
            .width(110.0)
            .show_ui(ui, |ui| {
                for t in w.all_tags() {
                    ui.selectable_value(&mut tag, t.clone(), t);
                }
                ui.separator();
                if ui.button("Add Tag...").clicked() {
                    open_tags = true;
                }
            });
        if tag != o.tag {
            edits.push((Edit::Tag(tag), key(id, "tag")));
        }
        ui.label("Layer");
        let mut layer = o.layer;
        let names = &w.layers.0;
        let current = &names[layer as usize];
        egui::ComboBox::from_id_salt(("layer", id.to_bits()))
            .selected_text(if current.is_empty() {
                format!("{layer}")
            } else {
                current.clone()
            })
            .width(110.0)
            .show_ui(ui, |ui| {
                for (i, n) in names.iter().enumerate() {
                    if !n.is_empty() {
                        ui.selectable_value(&mut layer, i as u8, format!("{i}: {n}"));
                    }
                }
                ui.separator();
                if ui.button("Add Layer...").clicked() {
                    open_tags = true;
                }
            });
        if layer != o.layer {
            edits.push((Edit::Layer(layer), key(id, "layer")));
        }
    });
    open_tags
}

/// Recomputes the prefab overrides shown for the selection (twice a second at most).
fn refresh_overrides(st: &mut EditorState, w: &mut World, id: GameObjectId) {
    let Some(root) = w.prefab_instance_root(id) else {
        st.inspector.overrides = None;
        return;
    };
    let now = oxide_core::time::realtime_since_startup();
    let fresh = st
        .inspector
        .overrides
        .as_ref()
        .is_some_and(|(r, t, _)| *r == root && now - t < 0.5);
    if !fresh {
        let ovs = w.prefab_overrides(root).unwrap_or_default();
        st.inspector.overrides = Some((root, now, ovs));
    }
}

const PREFAB_BLUE: Color32 = Color32::from_rgb(140, 180, 255);

fn describe(o: &PrefabOverride) -> String {
    match o {
        PrefabOverride::Property { property, .. } => format!("{property:?}")
            .split('(')
            .next()
            .unwrap_or("Property")
            .to_owned(),
        PrefabOverride::Field {
            component, field, ..
        } => {
            format!("{} › {}", nicify(component), nicify(field))
        }
        PrefabOverride::Enabled { component, .. } => format!("{} › Enabled", nicify(component)),
        PrefabOverride::AddedComponent { component, .. } => {
            format!("+ {}", nicify(&component.type_name))
        }
        PrefabOverride::RemovedComponent { component, .. } => format!("− {}", nicify(component)),
        PrefabOverride::AddedObject { data } => format!("+ {}", data.name),
        PrefabOverride::RemovedObject { .. } => "− (object)".into(),
    }
}

/// Unity's prefab bar: asset, Overrides menu (Apply All / Revert All), Unpack, variants.
fn prefab_header(ui: &mut egui::Ui, st: &mut EditorState, w: &World, id: GameObjectId) {
    let Some(root) = w.prefab_instance_root(id) else {
        if let Some(src) = w.get(id).and_then(|o| o.prefab_source.clone()) {
            ui.horizontal(|ui| {
                ui.label("Model");
                ui.label(RichText::new(src).color(PREFAB_BLUE));
            });
        }
        return;
    };
    let path = w
        .get(root)
        .and_then(|o| o.prefab_source.clone())
        .unwrap_or_default();
    ui.horizontal(|ui| {
        ui.label("Prefab");
        if ui
            .link(RichText::new(&path).color(PREFAB_BLUE))
            .on_hover_text("Show in Project window")
            .clicked()
        {
            st.project.selected = Some(path.clone());
            if let Some((dir, _)) = path.rsplit_once('/') {
                st.project.folder = dir.to_owned();
            }
            st.actions.push(Action::OpenTab(crate::Tab::Project));
        }
    });
    if root != id {
        ui.horizontal(|ui| {
            ui.weak(format!(
                "Part of prefab instance {}",
                w.get(root).map_or("", |o| o.name.as_str())
            ));
            if ui.small_button("Select Root").clicked() {
                st.select(root);
            }
        });
    }
    let count = st.inspector.overrides().len();
    ui.horizontal(|ui| {
        let label = if count == 0 {
            "No Overrides".to_owned()
        } else {
            format!("Overrides ({count})")
        };
        ui.menu_button(label, |ui| {
            ui.set_min_width(240.0);
            if count == 0 {
                ui.weak("This instance matches its prefab.");
            }
            egui::ScrollArea::vertical()
                .max_height(240.0)
                .show(ui, |ui| {
                    for o in st.inspector.overrides() {
                        ui.label(describe(o));
                    }
                });
            ui.separator();
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(count > 0, egui::Button::new("Revert All"))
                    .clicked()
                {
                    st.actions.push(Action::RevertPrefab);
                    ui.close();
                }
                if ui
                    .add_enabled(count > 0, egui::Button::new("Apply All"))
                    .clicked()
                {
                    st.actions.push(Action::ApplyPrefab);
                    ui.close();
                }
            });
        });
        if ui
            .button("Unpack")
            .on_hover_text("Unpack Completely")
            .clicked()
        {
            st.actions.push(Action::UnpackPrefab);
        }
        if ui
            .button("Create Variant")
            .on_hover_text("Save this instance as a variant of its prefab")
            .clicked()
        {
            let base = path.trim_end_matches(".prefab");
            st.actions.push(Action::CreatePrefabVariant(format!(
                "{base} Variant.prefab"
            )));
        }
    });
}

fn vec3_row(ui: &mut egui::Ui, label: &str, v: &mut Vector3, speed: f64, bold: bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let text = if bold {
            RichText::new(label).strong().color(Color32::WHITE)
        } else {
            RichText::new(label)
        };
        ui.add_sized([62.0, 18.0], egui::Label::new(text));
        // Each axis: letter + spacing + field + spacing.
        let gap = ui.spacing().item_spacing.x;
        let wdt = ((ui.available_width() - 3.0 * (10.0 + 2.0 * gap)) / 3.0).max(30.0);
        for (i, axis) in ["X", "Y", "Z"].iter().enumerate() {
            ui.label(RichText::new(*axis).color(crate::scene_view::AXIS_COLORS[i]));
            let r = ui.add_sized(
                [wdt, 18.0],
                egui::DragValue::new(&mut v[i]).speed(speed).max_decimals(4),
            );
            changed |= r.changed();
        }
    });
    changed
}

fn transform(
    ui: &mut egui::Ui,
    st: &mut EditorState,
    w: &World,
    id: GameObjectId,
    edits: &mut Vec<(Edit, u64)>,
) {
    let t = *w.transform(id).expect("alive");
    let link = w.get(id).and_then(|o| o.prefab_link);
    let has = |want: fn(&ObjectProperty) -> bool| {
        st.inspector.overrides().iter().any(|o| {
            matches!(o, PrefabOverride::Property { object, property } if Some(*object) == link && want(property))
        })
    };
    let bold = [
        has(|p| matches!(p, ObjectProperty::Position(_))),
        has(|p| matches!(p, ObjectProperty::Rotation(_))),
        has(|p| matches!(p, ObjectProperty::Scale(_))),
    ];
    let resp = egui::CollapsingHeader::new(RichText::new("Transform").strong())
        .id_salt("transform")
        .default_open(true)
        .show(ui, |ui| {
            let mut p = t.local_position;
            if vec3_row(ui, "Position", &mut p, 0.05, bold[0]) {
                edits.push((Edit::Position(p), key(id, "position")));
            }
            let euler = match st.inspector.euler.get(&id) {
                Some((q, e)) if q.abs_diff_eq(t.local_rotation, 1e-6) => *e,
                _ => {
                    let e = t.local_rotation.euler_angles();
                    // Show -10 rather than 350, like Unity does for fresh values.
                    let e = Vector3::new(wrap180(e.x), wrap180(e.y), wrap180(e.z));
                    st.inspector.euler.insert(id, (t.local_rotation, e));
                    e
                }
            };
            let mut e = euler;
            if vec3_row(ui, "Rotation", &mut e, 0.5, bold[1]) {
                let q = Quaternion::euler_v(e);
                st.inspector.euler.insert(id, (q, e));
                edits.push((Edit::Rotation(q), key(id, "rotation")));
            }
            let mut s = t.local_scale;
            if vec3_row(ui, "Scale", &mut s, 0.02, bold[2]) {
                edits.push((Edit::Scale(s), key(id, "scale")));
            }
        });
    resp.header_response.context_menu(|ui| {
        if ui.button("Reset").clicked() {
            edits.push((Edit::ResetTransform, 0));
            ui.close();
        }
    });
}

fn wrap180(a: f32) -> f32 {
    let a = a.rem_euclid(360.0);
    let a = if a > 180.0 { a - 360.0 } else { a };
    (a * 1e4).round() / 1e4
}

fn components(
    ui: &mut egui::Ui,
    st: &mut EditorState,
    w: &World,
    id: GameObjectId,
    edits: &mut Vec<(Edit, u64)>,
) {
    let o = w.get(id).expect("alive");
    let link = o.prefab_link;
    let list = o.component_list();
    let count = list.len();
    let mut per_type: HashMap<&str, usize> = HashMap::new();
    for (index, (cid, type_name, enabled)) in list.into_iter().enumerate() {
        let nth = {
            let n = per_type.entry(type_name).or_default();
            *n += 1;
            *n - 1
        };
        let overridden: Vec<String> = st
            .inspector
            .overrides()
            .iter()
            .filter_map(|ov| match ov {
                PrefabOverride::Field {
                    object,
                    component,
                    index,
                    field,
                    ..
                } if Some(*object) == link && component == type_name && *index == nth => {
                    Some(field.clone())
                }
                _ => None,
            })
            .collect();
        let Some(comp) = w.component_dyn(id, cid) else {
            continue;
        };
        let info = comp.type_info();
        let values: Vec<(FieldInfo, Value)> = info
            .fields
            .iter()
            .filter_map(|f| comp.get_field(f.name).map(|v| (*f, v)))
            .collect();
        ui.separator();
        let state = egui::collapsing_header::CollapsingState::load_with_default_open(
            ui.ctx(),
            egui::Id::new(("component", id.to_bits(), cid.0)),
            true,
        );
        let missing = w
            .component::<oxide_scene::MissingScript>(id, cid)
            .map(|m| m.script.clone());
        let header = state.show_header(ui, |ui| {
            let mut en = enabled;
            if ui.checkbox(&mut en, "").changed() {
                edits.push((Edit::Enabled(cid, en), key(id, &format!("en{}", cid.0))));
            }
            match &missing {
                Some(script) => ui.label(
                    RichText::new(format!("{} (Missing Script)", nicify(script)))
                        .strong()
                        .color(Color32::from_rgb(255, 170, 60)),
                ),
                None => ui.label(RichText::new(nicify(type_name)).strong()),
            };
        });
        let (_, header_resp, _) = header.body(|ui| {
            if let Some(script) = &missing {
                ui.weak(format!(
                    "The script `{script}` is not loaded (still compiling, renamed or deleted). Its saved values are kept."
                ));
            }
            if type_name == "LODGroup" {
                lod_group_editor(ui, w, id, cid, &st.selection, edits);
                return;
            }
            if values.is_empty() {
                ui.weak("No editable fields.");
            }
            for (f, v) in values {
                let mut v2 = v.clone();
                let label = nicify(f.name);
                let is_override = overridden.iter().any(|x| x == f.name);
                let changed = ui
                    .horizontal(|ui| {
                        let text = if is_override {
                            RichText::new(&label).strong().color(Color32::WHITE)
                        } else {
                            RichText::new(&label)
                        };
                        let lr = ui.add_sized([130.0, 18.0], egui::Label::new(text).truncate());
                        if is_override {
                            // Unity marks overridden properties with a blue bar.
                            let r = lr.rect;
                            ui.painter().rect_filled(
                                egui::Rect::from_min_size(
                                    egui::pos2(ui.max_rect().left() - 4.0, r.top()),
                                    egui::vec2(2.0, r.height()),
                                ),
                                0.0,
                                Color32::from_rgb(60, 140, 230),
                            );
                        }
                        if let Some(t) = f.tooltip {
                            lr.on_hover_text(t);
                        }
                        value_ui(
                            ui,
                            w,
                            &f,
                            f.kind,
                            f.type_name,
                            &mut v2,
                            &format!("{}{}", cid.0, f.name),
                        )
                    })
                    .inner;
                if changed && v2 != v {
                    edits.push((
                        Edit::Field(cid, f.name.to_owned(), v2),
                        key(id, &format!("{}.{}", cid.0, f.name)),
                    ));
                }
            }
        });
        header_resp.response.context_menu(|ui| {
            if ui.button("Reset").clicked() {
                edits.push((Edit::Reset(cid), 0));
                ui.close();
            }
            if ui.button("Remove Component").clicked() {
                edits.push((Edit::Remove(cid), 0));
                ui.close();
            }
            if ui
                .add_enabled(index > 0, egui::Button::new("Move Up"))
                .clicked()
            {
                edits.push((Edit::Move(cid, -1), 0));
                ui.close();
            }
            if ui
                .add_enabled(index + 1 < count, egui::Button::new("Move Down"))
                .clicked()
            {
                edits.push((Edit::Move(cid, 1), 0));
                ui.close();
            }
            ui.separator();
            if ui.button("Copy Component").clicked()
                && let Some(c) = w.component_dyn(id, cid)
            {
                let fields = c
                    .fields()
                    .into_iter()
                    .map(|(k, v)| (k.to_owned(), v))
                    .collect();
                st.inspector.clipboard = Some((type_name.to_owned(), fields));
                ui.close();
            }
            let can_paste = st
                .inspector
                .clipboard
                .as_ref()
                .is_some_and(|(t, _)| t == type_name);
            if ui
                .add_enabled(can_paste, egui::Button::new("Paste Component Values"))
                .clicked()
                && let Some((_, f)) = &st.inspector.clipboard
            {
                edits.push((Edit::Paste(cid, f.clone()), 0));
                ui.close();
            }
        });
    }
}

/// Display name of an asset: its own name for materials and meshes, else the file part of
/// the path (`Assets/Cars/car.glb#mesh0` → `car.glb#mesh0`).
fn asset_label(assets: &AssetServer, path: &str) -> String {
    let named = |n: &str| (!n.is_empty()).then(|| n.to_owned());
    let own = if let Some(h) = assets.materials.handle_of(path) {
        assets.materials.get(&h).and_then(|m| named(&m.name))
    } else if let Some(h) = assets.meshes.handle_of(path) {
        assets.meshes.get(&h).and_then(|m| named(&m.name))
    } else {
        None
    };
    own.unwrap_or_else(|| path.rsplit('/').next().unwrap_or(path).to_owned())
}

fn assets_of_type(assets: &AssetServer, type_name: Option<&str>) -> Vec<String> {
    let mut v: Vec<String> = match type_name.unwrap_or("") {
        "Mesh" => assets.meshes.iter().map(|(p, _)| p.to_owned()).collect(),
        "Material" => assets.materials.iter().map(|(p, _)| p.to_owned()).collect(),
        // Render Textures work wherever a texture does.
        "Texture2D" | "Texture" => assets
            .textures
            .iter()
            .map(|(p, _)| p.to_owned())
            .chain(assets.render_textures.iter().map(|(p, _)| p.to_owned()))
            .collect(),
        "RenderTexture" => assets
            .render_textures
            .iter()
            .map(|(p, _)| p.to_owned())
            .collect(),
        "AudioClip" => assets.audio.iter().map(|(p, _)| p.to_owned()).collect(),
        "AnimationClip" => assets
            .animations
            .iter()
            .map(|(p, _)| p.to_owned())
            .collect(),
        "AnimatorController" | "RuntimeAnimatorController" => assets
            .controllers
            .iter()
            .map(|(p, _)| p.to_owned())
            .collect(),
        "Model" => assets.models.iter().map(|(p, _)| p.to_owned()).collect(),
        "TerrainData" => assets.terrains.iter().map(|(p, _)| p.to_owned()).collect(),
        _ => Vec::new(),
    };
    v.sort();
    v
}

/// Draws one value. Returns true if the user changed it.
/// Every reflected field of `obj` as a label / value grid, edits written back (Project
/// Settings and other non-component objects). Returns whether anything changed.
pub fn reflect_grid(
    ui: &mut egui::Ui,
    w: &World,
    obj: &mut dyn oxide_core::reflect::Reflect,
    salt: &str,
) -> bool {
    let info = obj.type_info();
    let mut changed = false;
    egui::Grid::new(salt)
        .num_columns(2)
        .spacing([16.0, 6.0])
        .min_col_width(150.0)
        .show(ui, |ui| {
            for f in &info.fields {
                let Some(v) = obj.get_field(f.name) else {
                    continue;
                };
                let lr = ui.label(nicify(f.name));
                if let Some(t) = f.tooltip {
                    lr.on_hover_text(t);
                }
                let mut v2 = v.clone();
                if value_ui(
                    ui,
                    w,
                    f,
                    f.kind,
                    f.type_name,
                    &mut v2,
                    &format!("{salt}{}", f.name),
                ) && v2 != v
                    && obj.set_field(f.name, v2).is_ok()
                {
                    changed = true;
                }
                ui.end_row();
            }
        });
    changed
}

fn value_ui(
    ui: &mut egui::Ui,
    w: &World,
    f: &FieldInfo,
    kind: ValueKind,
    type_name: Option<&str>,
    v: &mut Value,
    salt: &str,
) -> bool {
    match v {
        Value::Bool(b) => ui.checkbox(b, "").changed(),
        Value::Int(i) => match f.range {
            Some((lo, hi)) if kind == ValueKind::Int => ui
                .add(egui::Slider::new(i, lo as i64..=hi as i64))
                .changed(),
            _ => ui.add(egui::DragValue::new(i)).changed(),
        },
        Value::Float(x) => match f.range {
            Some((lo, hi)) if kind == ValueKind::Float => {
                ui.add(egui::Slider::new(x, lo..=hi)).changed()
            }
            _ => {
                let speed = (x.abs() * 0.01).clamp(0.001, 10.0);
                ui.add(egui::DragValue::new(x).speed(speed).max_decimals(4))
                    .changed()
            }
        },
        Value::String(s) => ui
            .add(egui::TextEdit::singleline(s).desired_width(f32::INFINITY))
            .changed(),
        Value::Vector2(a) => drag_array(ui, a),
        Value::Vector3(a) => drag_array(ui, a),
        Value::Vector4(a) => drag_array(ui, a),
        Value::Quaternion(q) => {
            let quat = Quaternion::from_array(*q);
            let e = quat.euler_angles();
            let mut a = [wrap180(e.x), wrap180(e.y), wrap180(e.z)];
            if drag_array(ui, &mut a) {
                *q = Quaternion::euler(a[0], a[1], a[2]).to_array();
                true
            } else {
                false
            }
        }
        Value::Color(c) => ui.color_edit_button_rgba_unmultiplied(c).changed(),
        Value::Enum(e) => {
            let mut changed = false;
            egui::ComboBox::from_id_salt(("enum", salt))
                .selected_text(nicify(e))
                .show_ui(ui, |ui| {
                    for o in f.options {
                        if ui.selectable_label(e == o, nicify(o)).clicked() {
                            *e = (*o).to_owned();
                            changed = true;
                        }
                    }
                });
            changed
        }
        Value::Asset(_) | Value::Null if kind == ValueKind::Asset => {
            asset_field(ui, w, type_name, v, salt)
        }
        Value::ObjectRef(_) | Value::Null if kind == ValueKind::ObjectRef => {
            object_field(ui, w, v, salt)
        }
        Value::List(items) => {
            let mut changed = false;
            let elem = f.element.unwrap_or(ValueKind::Null);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.weak(format!("{} item(s)", items.len()));
                    if ui.small_button("+").clicked() {
                        let new = items.last().cloned().unwrap_or(match elem {
                            ValueKind::Float => Value::Float(0.0),
                            ValueKind::Int => Value::Int(0),
                            ValueKind::Bool => Value::Bool(false),
                            ValueKind::String => Value::String(String::new()),
                            ValueKind::Vector3 => Value::Vector3([0.0; 3]),
                            _ => Value::Null,
                        });
                        items.push(new);
                        changed = true;
                    }
                    if ui.small_button("-").clicked() && items.pop().is_some() {
                        changed = true;
                    }
                });
                for (n, item) in items.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.weak(format!("{n}"));
                        changed |=
                            value_ui(ui, w, f, elem, type_name, item, &format!("{salt}[{n}]"));
                    });
                }
            });
            changed
        }
        // A Volume parameter: Unity's override checkbox, then the value (greyed out while
        // not overridden). The field's own info (range, enum options) applies to the value.
        Value::Object(map)
            if map.len() == 2
                && matches!(map.get("override"), Some(Value::Bool(_)))
                && map.contains_key("value") =>
        {
            let mut changed = false;
            ui.horizontal(|ui| {
                if let Some(Value::Bool(on)) = map.get_mut("override") {
                    changed |= ui
                        .checkbox(on, "")
                        .on_hover_text("Override this parameter")
                        .changed();
                }
                let on = matches!(map.get("override"), Some(Value::Bool(true)));
                if let Some(inner) = map.get_mut("value") {
                    let k = inner.kind();
                    ui.add_enabled_ui(on, |ui| {
                        changed |= value_ui(ui, w, f, k, type_name, inner, &format!("{salt}.v"));
                    });
                }
            });
            changed
        }
        Value::Object(map) => {
            let mut changed = false;
            ui.vertical(|ui| {
                for (k, item) in map.iter_mut() {
                    ui.horizontal(|ui| {
                        ui.add_sized([110.0, 18.0], egui::Label::new(nicify(k)).truncate());
                        let k2 = item.kind();
                        let sub = FieldInfo {
                            name: "",
                            kind: k2,
                            options: &[],
                            type_name: None,
                            element: None,
                            range: None,
                            tooltip: None,
                        };
                        changed |= value_ui(ui, w, &sub, k2, None, item, &format!("{salt}.{k}"));
                    });
                }
            });
            changed
        }
        Value::ObjectRef(_) => object_field(ui, w, v, salt),
        Value::Asset(_) => asset_field(ui, w, type_name, v, salt),
        Value::Null => {
            ui.weak("None");
            false
        }
    }
}

fn drag_array<const N: usize>(ui: &mut egui::Ui, a: &mut [f32; N]) -> bool {
    let mut changed = false;
    let gap = ui.spacing().item_spacing.x;
    let wdt = ((ui.available_width() - N as f32 * gap) / N as f32).max(30.0);
    for x in a.iter_mut() {
        changed |= ui
            .add_sized(
                [wdt, 18.0],
                egui::DragValue::new(x).speed(0.02).max_decimals(4),
            )
            .changed();
    }
    changed
}

fn asset_field(
    ui: &mut egui::Ui,
    w: &World,
    type_name: Option<&str>,
    v: &mut Value,
    salt: &str,
) -> bool {
    let assets = w.resource::<AssetServer>();
    let current = match v {
        Value::Asset(p) => asset_label(assets, p),
        _ => format!("None ({})", type_name.unwrap_or("Asset")),
    };
    let mut changed = false;
    let r = egui::ComboBox::from_id_salt(("asset", salt))
        .selected_text(current)
        .width(ui.available_width().max(60.0))
        .show_ui(ui, |ui| {
            if ui
                .selectable_label(matches!(v, Value::Null), "None")
                .clicked()
            {
                *v = Value::Null;
                changed = true;
            }
            for p in assets_of_type(assets, type_name) {
                let sel = matches!(v, Value::Asset(c) if *c == p);
                if ui
                    .selectable_label(sel, asset_label(assets, &p))
                    .on_hover_text(&p)
                    .clicked()
                {
                    *v = Value::Asset(p);
                    changed = true;
                }
            }
        });
    if let Some(payload) = r.response.dnd_release_payload::<AssetPayload>() {
        *v = Value::Asset(payload.0.clone());
        changed = true;
    }
    changed
}

fn object_field(ui: &mut egui::Ui, w: &World, v: &mut Value, salt: &str) -> bool {
    let current = match v {
        Value::ObjectRef(bits) => {
            let id = GameObjectId::from_bits(*bits);
            w.get(id)
                .map_or_else(|| "Missing".to_owned(), |o| o.name.clone())
        }
        _ => "None (GameObject)".to_owned(),
    };
    let mut changed = false;
    let r = egui::ComboBox::from_id_salt(("object", salt))
        .selected_text(current)
        .width(ui.available_width().max(60.0))
        .show_ui(ui, |ui| {
            if ui
                .selectable_label(matches!(v, Value::Null), "None")
                .clicked()
            {
                *v = Value::Null;
                changed = true;
            }
            for id in w.hierarchy_order().into_iter().take(2000) {
                let sel = matches!(v, Value::ObjectRef(b) if *b == id.to_bits());
                if ui.selectable_label(sel, w.path_of(id)).clicked() {
                    *v = Value::ObjectRef(id.to_bits());
                    changed = true;
                }
            }
        });
    if let Some(payload) = r.response.dnd_release_payload::<HierarchyDrag>()
        && let Some(id) = payload.0.first()
    {
        *v = Value::ObjectRef(id.to_bits());
        changed = true;
    }
    changed
}

fn add_component(ui: &mut egui::Ui, st: &mut EditorState, w: &World) {
    ui.vertical_centered(|ui| {
        ui.menu_button(RichText::new("  Add Component  ").strong(), |ui| {
            ui.set_min_width(220.0);
            let r = ui.add(
                egui::TextEdit::singleline(&mut st.inspector.add_search)
                    .hint_text("Search")
                    .desired_width(f32::INFINITY),
            );
            r.request_focus();
            let q = st.inspector.add_search.trim().to_lowercase();
            let mut regs: Vec<(String, &'static str)> = w
                .registry
                .iter()
                .filter(|r| r.info.name != "Transform")
                .map(|r| (r.menu.clone(), r.info.name))
                .collect();
            regs.sort();
            let mut pick = None;
            if !q.is_empty() {
                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .show(ui, |ui| {
                        for (menu, name) in &regs {
                            if (menu.to_lowercase().contains(&q)
                                || name.to_lowercase().contains(&q))
                                && ui.button(nicify(name)).on_hover_text(menu).clicked()
                            {
                                pick = Some(*name);
                            }
                        }
                    });
                if ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && let Some((_, name)) = regs.iter().find(|(m, n)| {
                        m.to_lowercase().contains(&q) || n.to_lowercase().contains(&q)
                    })
                {
                    pick = Some(*name);
                }
            } else {
                let mut groups: BTreeMap<String, Vec<(String, &'static str)>> = BTreeMap::new();
                for (menu, name) in &regs {
                    let (group, leaf) = menu.rsplit_once('/').unwrap_or(("Miscellaneous", menu));
                    groups
                        .entry(group.to_owned())
                        .or_default()
                        .push((leaf.to_owned(), name));
                }
                for (group, items) in groups {
                    ui.menu_button(group, |ui| {
                        for (leaf, name) in items {
                            if ui.button(leaf).clicked() {
                                pick = Some(name);
                            }
                        }
                    });
                }
            }
            if let Some(name) = pick {
                st.actions.push(Action::AddComponent(name.to_owned()));
                st.inspector.add_search.clear();
                ui.close();
            }
        });
    });
}

fn apply(st: &mut EditorState, w: &mut World, id: GameObjectId, edits: Vec<(Edit, u64)>) {
    if edits.is_empty() {
        return;
    }
    let now = oxide_core::time::realtime_since_startup();
    for (edit, k) in edits {
        let label = match &edit {
            Edit::Field(_, f, _) => format!("Modify {}", nicify(f)),
            Edit::Remove(_) => "Remove Component".into(),
            Edit::Name(_) => "Rename".into(),
            Edit::Position(_) | Edit::Rotation(_) | Edit::Scale(_) | Edit::ResetTransform => {
                "Transform".into()
            }
            _ => "Inspector".into(),
        };
        st.record(w, &label, (k != 0).then_some(k), now);
        match edit {
            Edit::Field(cid, name, value) => {
                if let Some(c) = w.component_dyn_mut(id, cid)
                    && let Err(e) = c.set_field(&name, value)
                {
                    st.status = format!("{name}: {e}");
                }
            }
            Edit::Enabled(cid, en) => w.set_component_enabled(id, cid, en),
            Edit::Remove(cid) => w.remove_component_now(id, cid),
            Edit::Move(cid, d) => w.move_component(id, cid, d),
            Edit::Paste(cid, fields) => {
                if let Some(c) = w.component_dyn_mut(id, cid) {
                    for (k, v) in fields {
                        let _ = c.set_field(&k, v);
                    }
                }
            }
            Edit::Reset(cid) => {
                let type_name = w
                    .get(id)
                    .and_then(|o| o.component_list().into_iter().find(|c| c.0 == cid))
                    .map(|c| c.1);
                let fresh = type_name.and_then(|t| w.registry.create(t));
                if let (Some(fresh), Some(c)) = (fresh, w.component_dyn_mut(id, cid)) {
                    for (k, v) in fresh.fields() {
                        let _ = c.set_field(k, v);
                    }
                }
            }
            Edit::Name(n) => {
                if let Some(o) = w.get_mut(id) {
                    o.name = n;
                }
            }
            Edit::Active(a) => w.set_active(id, a),
            Edit::Static(s) => {
                if let Some(o) = w.get_mut(id) {
                    o.static_flags = s;
                }
            }
            Edit::Tag(t) => {
                if let Some(o) = w.get_mut(id) {
                    o.tag = t;
                }
            }
            Edit::Layer(l) => {
                if let Some(o) = w.get_mut(id) {
                    o.layer = l;
                }
            }
            Edit::Position(p) => {
                if let Some(t) = w.transform_mut(id) {
                    t.local_position = p;
                }
            }
            Edit::Rotation(q) => {
                if let Some(t) = w.transform_mut(id) {
                    t.local_rotation = q;
                }
            }
            Edit::Scale(s) => {
                if let Some(t) = w.transform_mut(id) {
                    t.local_scale = s;
                }
            }
            Edit::ResetTransform => {
                if let Some(t) = w.transform_mut(id) {
                    *t = oxide_scene::Transform::default();
                }
            }
        }
    }
}

const LOD_COLORS: [Color32; 6] = [
    Color32::from_rgb(60, 140, 60),
    Color32::from_rgb(50, 110, 160),
    Color32::from_rgb(140, 120, 50),
    Color32::from_rgb(120, 70, 150),
    Color32::from_rgb(60, 130, 130),
    Color32::from_rgb(150, 90, 60),
];

/// Unity's LOD Group inspector: the level bar, transition heights, and per-level renderers
/// (drop objects from the Hierarchy, or add the selection).
fn lod_group_editor(
    ui: &mut egui::Ui,
    w: &World,
    id: GameObjectId,
    cid: ComponentId,
    selection: &[GameObjectId],
    edits: &mut Vec<(Edit, u64)>,
) {
    let Some(g) = w.component::<oxide_scene::LODGroup>(id, cid) else {
        return;
    };
    let mut lods = g.lods.clone();
    let mut size = g.size;
    let mut changed = false;

    // Bar: each level's share of screen heights 100% → its transition, then Culled.
    let (bar, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::hover());
    let p = ui.painter();
    let mut top = 1.0f32;
    for (i, l) in lods.iter().enumerate() {
        let h = l.screen_relative_height.clamp(0.0, top);
        let x0 = bar.left() + (1.0 - top) * bar.width();
        let x1 = bar.left() + (1.0 - h) * bar.width();
        let r = egui::Rect::from_min_max(egui::pos2(x0, bar.top()), egui::pos2(x1, bar.bottom()));
        p.rect_filled(r, 0.0, LOD_COLORS[i % LOD_COLORS.len()]);
        p.text(
            r.left_center() + egui::vec2(4.0, 0.0),
            egui::Align2::LEFT_CENTER,
            format!("LOD {i}\n{:.0}%", top * 100.0),
            egui::FontId::proportional(10.0),
            Color32::WHITE,
        );
        top = h;
    }
    let culled = egui::Rect::from_min_max(
        egui::pos2(bar.left() + (1.0 - top) * bar.width(), bar.top()),
        bar.right_bottom(),
    );
    p.rect_filled(culled, 0.0, Color32::from_rgb(130, 45, 45));
    p.text(
        culled.center(),
        egui::Align2::CENTER_CENTER,
        "Culled",
        egui::FontId::proportional(10.0),
        Color32::WHITE,
    );

    let mut remove_level = None;
    for (i, l) in lods.iter_mut().enumerate() {
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("LOD {i}"))
                    .strong()
                    .color(LOD_COLORS[i % LOD_COLORS.len()]),
            );
            ui.label("shown above");
            let mut pct = l.screen_relative_height * 100.0;
            if ui
                .add(
                    egui::DragValue::new(&mut pct)
                        .range(0.0..=100.0)
                        .speed(0.2)
                        .suffix("%"),
                )
                .changed()
            {
                l.screen_relative_height = pct / 100.0;
                changed = true;
            }
            ui.label("of screen height");
            if ui
                .small_button("x")
                .on_hover_text("Remove this level")
                .clicked()
            {
                remove_level = Some(i);
            }
        });
        let mut drop_target = egui::Frame::new()
            .fill(Color32::from_gray(44))
            .inner_margin(4)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                if l.renderers.is_empty() {
                    ui.weak("Drop objects here");
                }
                let mut gone = None;
                for (n, r) in l.renderers.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(w.get(*r).map_or("Missing", |o| o.name.as_str()));
                        if ui.small_button("x").clicked() {
                            gone = Some(n);
                        }
                    });
                }
                if let Some(n) = gone {
                    l.renderers.remove(n);
                    changed = true;
                }
            })
            .response;
        drop_target = drop_target.interact(egui::Sense::hover());
        if let Some(payload) = drop_target.dnd_release_payload::<HierarchyDrag>() {
            for o in &payload.0 {
                if !l.renderers.contains(o) {
                    l.renderers.push(*o);
                }
            }
            changed = true;
        }
        let picks: Vec<GameObjectId> = selection
            .iter()
            .copied()
            .filter(|s| *s != id && !l.renderers.contains(s))
            .collect();
        if ui
            .add_enabled(!picks.is_empty(), egui::Button::new("Add Selected"))
            .on_hover_text(
                "Add the objects selected in the Hierarchy (Cmd/Shift-click to select several)",
            )
            .clicked()
        {
            l.renderers.extend(picks);
            changed = true;
        }
    }
    if let Some(i) = remove_level {
        lods.remove(i);
        changed = true;
    }
    ui.separator();
    ui.horizontal(|ui| {
        if ui.button("Add Level").clicked() {
            let h = lods.last().map_or(0.5, |l| l.screen_relative_height * 0.5);
            lods.push(oxide_scene::Lod {
                screen_relative_height: h,
                renderers: Vec::new(),
            });
            changed = true;
        }
        ui.label("Size");
        let r = ui
            .add(
                egui::DragValue::new(&mut size)
                    .range(0.0..=10_000.0)
                    .speed(0.05),
            )
            .on_hover_text("Object size for the screen-height test (0 = from LOD 0's bounds)");
        if r.changed() {
            edits.push((
                Edit::Field(cid, "size".into(), Value::Float(size as f64)),
                key(id, "lod-size"),
            ));
        }
    });
    if changed {
        let value = Value::List(
            lods.iter()
                .map(|l| {
                    Value::Object(
                        [
                            (
                                "screen_relative_height".to_owned(),
                                Value::Float(l.screen_relative_height as f64),
                            ),
                            (
                                "renderers".to_owned(),
                                Value::List(
                                    l.renderers
                                        .iter()
                                        .map(|r| Value::ObjectRef(r.to_bits()))
                                        .collect(),
                                ),
                            ),
                        ]
                        .into_iter()
                        .collect(),
                    )
                })
                .collect(),
        );
        edits.push((Edit::Field(cid, "lods".into(), value), key(id, "lods")));
    }
}

#[cfg(test)]
mod tests {
    use super::nicify;

    #[test]
    fn nicify_names() {
        assert_eq!(nicify("field_of_view"), "Field Of View");
        assert_eq!(nicify("fieldOfView"), "Field Of View");
        assert_eq!(nicify("MeshRenderer"), "Mesh Renderer");
        assert_eq!(nicify("speed"), "Speed");
        assert_eq!(nicify("uv2"), "Uv2");
    }
}
