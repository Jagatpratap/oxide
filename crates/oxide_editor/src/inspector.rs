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
    /// Debug mode (Unity's Inspector ⋮ → Debug): ids, lifecycle and raw values.
    pub debug: bool,
    /// Locked Inspector: keeps showing these objects whatever is selected.
    pub locked: Option<Vec<GameObjectId>>,
    /// A component type to expand and scroll into view on the next frame (MCP).
    pub scroll_to: Option<String>,
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

/// A component on every inspected object: the `nth` component of this type (so multi-object
/// edits reach the matching component on each object).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Slot {
    type_name: &'static str,
    nth: usize,
}

impl Slot {
    fn resolve(self, w: &World, id: GameObjectId) -> Option<ComponentId> {
        w.get(id)?
            .component_list()
            .into_iter()
            .filter(|c| c.1 == self.type_name)
            .nth(self.nth)
            .map(|c| c.0)
    }
}

/// An edit to apply after the UI pass (so the world isn't borrowed while drawing). Applied
/// to every inspected object.
enum Edit {
    Field(Slot, String, Value),
    Enabled(Slot, bool),
    Remove(Slot),
    Move(Slot, i32),
    Paste(Slot, BTreeMap<String, Value>),
    Reset(Slot),
    Name(String),
    Active(bool),
    Static(oxide_scene::StaticEditorFlags),
    Tag(String),
    Layer(u8),
    /// New values for the axes set in the mask (other axes keep each object's own value).
    Position(Vector3, [bool; 3]),
    /// Euler angles (degrees) for the masked axes.
    Rotation(Vector3, [bool; 3]),
    Scale(Vector3, [bool; 3]),
    /// Debug mode: the raw local rotation.
    RotationQuat(Quaternion),
    ResetTransform,
}

pub fn ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    // An asset clicked in the Project window, until a GameObject is selected.
    if !st.selection.is_empty() {
        st.project.inspecting = false;
    }
    if st.inspector.locked.is_none()
        && st.project.inspecting
        && let Some(path) = st.project.selected.clone()
    {
        crate::asset_inspector::ui(st, engine, ui, &path);
        return;
    }
    // Lock and Debug (Unity's padlock and ⋮ → Normal / Debug).
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mut locked = st.inspector.locked.is_some();
            let icon = if locked { "🔒" } else { "🔓" };
            if ui
                .toggle_value(&mut locked, icon)
                .on_hover_text("Lock: keep inspecting these objects when the selection changes")
                .changed()
            {
                st.inspector.locked = locked.then(|| st.selection.clone());
            }
            ui.toggle_value(&mut st.inspector.debug, "Debug")
                .on_hover_text("Debug mode: ids, lifecycle and raw values");
        });
    });
    let w = &engine.world;
    let source = st
        .inspector
        .locked
        .clone()
        .unwrap_or_else(|| st.selection.clone());
    let targets: Vec<GameObjectId> = source.into_iter().filter(|&id| w.is_alive(id)).collect();
    let Some(&id) = targets.last() else {
        ui.add_space(8.0);
        ui.weak(if st.inspector.locked.is_some() {
            "The locked objects were destroyed. Unlock to follow the selection."
        } else {
            "Select a GameObject to inspect it."
        });
        return;
    };
    refresh_overrides(st, &mut engine.world, id);
    let mut edits: Vec<(Edit, u64)> = Vec::new();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Narrow number boxes are allowed (egui's default minimum is 40 px).
            ui.spacing_mut().interact_size.x = 24.0;
            let w = &engine.world;
            if targets.len() > 1 {
                ui.weak(format!("{} objects selected", targets.len()));
            }
            if header(ui, w, &targets, &mut edits) {
                st.actions.push(Action::OpenProjectSettings(
                    crate::settings_window::Page::TagsAndLayers,
                ));
            }
            if st.inspector.debug {
                debug_info(ui, w, id);
            }
            if targets.len() == 1 {
                prefab_header(ui, st, w, id);
            }
            ui.separator();
            transform(ui, st, w, &targets, &mut edits);
            components(ui, st, w, &targets, &mut edits);
            ui.add_space(8.0);
            add_component(ui, st, w);
            ui.add_space(24.0);
        });
    apply(st, &mut engine.world, &targets, edits);
}

/// A script's own Inspector (Unity `[CustomEditor]`). It draws on a copy of the component;
/// fields it changed become ordinary edits (Undo, prefab overrides, multi-scene saving).
fn custom_inspector(
    ui: &mut egui::Ui,
    w: &World,
    id: GameObjectId,
    cid: ComponentId,
    slot: Slot,
    editor: oxide_engine::custom_editor::InspectorFn,
    edits: &mut Vec<(Edit, u64)>,
) {
    let Some(comp) = w.component_dyn(id, cid) else {
        return;
    };
    let before = comp.fields();
    let mut copy = comp.clone_box();
    let draw_default = |ui: &mut egui::Ui,
                        w: &World,
                        obj: &mut dyn oxide_core::reflect::Reflect,
                        salt: &str| { reflect_grid(ui, w, obj, salt) };
    let salt = format!("custom-{}-{}", id.to_bits(), cid.0);
    let mut gui = oxide_engine::InspectorGui::new(ui, w, id, &draw_default, salt);
    // A panicking Inspector must not take the editor down with it.
    let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        editor(&mut *copy, &mut gui);
    }));
    if ran.is_err() {
        ui.colored_label(
            Color32::from_rgb(240, 110, 90),
            "This component's Inspector panicked; see the Console.",
        );
        return;
    }
    for (k, v) in copy.fields() {
        if before.iter().any(|(bk, bv)| *bk == k && *bv != v) {
            edits.push((Edit::Field(slot, k.to_owned(), v), key(id, k)));
        }
    }
}

/// Debug mode: what Unity's Debug Inspector shows beyond the fields.
fn debug_info(ui: &mut egui::Ui, w: &World, id: GameObjectId) {
    let Some(o) = w.get(id) else { return };
    egui::CollapsingHeader::new(RichText::new("Debug").strong())
        .default_open(true)
        .show(ui, |ui| {
            egui::Grid::new("inspector-debug")
                .num_columns(2)
                .spacing([12.0, 2.0])
                .show(ui, |ui| {
                    let mut row = |k: &str, v: String| {
                        ui.label(k);
                        ui.label(RichText::new(v).monospace());
                        ui.end_row();
                    };
                    row("Instance ID", format!("{}", id.to_bits()));
                    let scene = w.scene_info(o.scene());
                    row(
                        "Scene",
                        scene.map_or_else(|| format!("#{}", o.scene().0), |s| s.name),
                    );
                    row("Active In Hierarchy", o.active_in_hierarchy().to_string());
                    row("Children", o.children().len().to_string());
                    row(
                        "Prefab Source",
                        o.prefab_source.clone().unwrap_or_else(|| "—".into()),
                    );
                    row(
                        "Prefab Link",
                        o.prefab_link.map_or_else(|| "—".into(), |l| l.to_string()),
                    );
                    let p = w.position(id);
                    row(
                        "World Position",
                        format!("({:.3}, {:.3}, {:.3})", p.x, p.y, p.z),
                    );
                    let e = w.rotation(id).euler_angles();
                    row(
                        "World Rotation",
                        format!("({:.2}, {:.2}, {:.2})", e.x, e.y, e.z),
                    );
                    let sc = w.lossy_scale(id);
                    row(
                        "Lossy Scale",
                        format!("({:.3}, {:.3}, {:.3})", sc.x, sc.y, sc.z),
                    );
                });
        });
}

/// Width left on this row, up to the visible right edge of the Inspector (less room for
/// the scroll bar). Unlike `available_width`, it never grows when an earlier row overflowed,
/// so one wide widget cannot push every row below it off the panel.
pub fn avail(ui: &egui::Ui) -> f32 {
    let edge = ui.clip_rect().right() - SCROLL_BAR_ROOM;
    ui.available_width().min(edge - ui.cursor().left()).max(0.0)
}

/// Space kept free at the right for the floating scroll bar.
const SCROLL_BAR_ROOM: f32 = 10.0;

/// Height of one Inspector row.
const ROW_H: f32 = 20.0;

/// Width of the label column for a row of `width` (Unity's `EditorGUIUtility.labelWidth`:
/// about 45% of the Inspector, never narrower than the longest common label needs).
pub fn label_width(width: f32) -> f32 {
    (width * 0.42).clamp(72.0, 190.0)
}

/// A left-aligned property label in the label column; long labels are cut with "…" and
/// show in full on hover.
pub fn prop_label(ui: &mut egui::Ui, text: impl Into<egui::WidgetText>) -> egui::Response {
    let w = label_width(avail(ui));
    prop_label_w(ui, text, w)
}

/// [`prop_label`] with an explicit column width (rows whose values need more room).
fn prop_label_w(ui: &mut egui::Ui, text: impl Into<egui::WidgetText>, w: f32) -> egui::Response {
    let text: egui::WidgetText = text.into();
    let full = text.text().to_owned();
    let r = ui
        .allocate_ui_with_layout(
            egui::vec2(w, ROW_H),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_min_size(egui::vec2(w, ROW_H));
                ui.add(egui::Label::new(text).truncate())
            },
        )
        .inner;
    if r.hovered()
        && ui.fonts_mut(|f| {
            f.layout_no_wrap(full.clone(), egui::FontId::default(), Color32::WHITE)
                .size()
                .x
        }) > w
    {
        r.clone().on_hover_text(full)
    } else {
        r
    }
}

fn key(id: GameObjectId, part: &str) -> u64 {
    egui::Id::new((id.to_bits(), part)).value()
}

/// Returns true when "Add Tag..." / "Add Layer..." was clicked (opens Tags and Layers).
/// With several objects, a mixed value shows as indeterminate or "—" and an edit sets it on all.
fn header(
    ui: &mut egui::Ui,
    w: &World,
    targets: &[GameObjectId],
    edits: &mut Vec<(Edit, u64)>,
) -> bool {
    let id = *targets.last().expect("a target");
    let o = w.get(id).expect("alive");
    let objs: Vec<_> = targets.iter().filter_map(|&t| w.get(t)).collect();
    let mixed = |f: &dyn Fn(&oxide_scene::GameObject) -> String| objs.iter().any(|x| f(x) != f(o));
    let mut open_tags = false;
    ui.horizontal(|ui| {
        let mut active = o.active_self();
        let mixed_active = objs.iter().any(|x| x.active_self() != active);
        if ui
            .add(egui::Checkbox::without_text(&mut active).indeterminate(mixed_active))
            .changed()
        {
            edits.push((Edit::Active(active), key(id, "active")));
        }
        // Static and its flags dropdown sit at the right; the name takes what is left.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            use oxide_scene::StaticEditorFlags as F;
            let flags = o.static_flags;
            let same_flags = objs.iter().all(|x| x.static_flags == flags);
            ui.menu_button("⏷", |ui| {
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
            // Unity's Static checkbox (all flags).
            let mut all = same_flags && flags == F::EVERYTHING;
            let r = ui.add(
                egui::Checkbox::new(&mut all, "Static")
                    .indeterminate(!same_flags || (flags != F::NOTHING && flags != F::EVERYTHING)),
            );
            if r.changed() {
                let f = if all { F::EVERYTHING } else { F::NOTHING };
                edits.push((Edit::Static(f), key(id, "static")));
            }
            let mixed_name = mixed(&|x| x.name.clone());
            let mut name = if mixed_name {
                String::new()
            } else {
                o.name.clone()
            };
            let r = ui.add(
                egui::TextEdit::singleline(&mut name)
                    .hint_text(if mixed_name { "—" } else { "" })
                    .desired_width(avail(ui)),
            );
            if r.changed() {
                edits.push((Edit::Name(name), key(id, "name")));
            }
        });
    });
    ui.horizontal(|ui| {
        let gap = ui.spacing().item_spacing.x;
        let label_w = ui.fonts_mut(|f| {
            ["Tag", "Layer"].map(|t| {
                f.layout_no_wrap(t.into(), egui::FontId::default(), Color32::WHITE)
                    .size()
                    .x
            })
        });
        let combo_w = ((avail(ui) - label_w[0] - label_w[1] - 3.0 * gap) / 2.0).max(50.0);
        ui.label("Tag");
        let mixed_tag = mixed(&|x| x.tag.clone());
        let orig = if mixed_tag {
            String::new()
        } else {
            o.tag.clone()
        };
        let mut tag = orig.clone();
        egui::ComboBox::from_id_salt(("tag", id.to_bits()))
            .selected_text(if mixed_tag { "—" } else { tag.as_str() })
            .width(combo_w)
            .show_ui(ui, |ui| {
                for t in w.all_tags() {
                    ui.selectable_value(&mut tag, t.clone(), t);
                }
                ui.separator();
                if ui.button("Add Tag...").clicked() {
                    open_tags = true;
                }
            });
        if tag != orig {
            edits.push((Edit::Tag(tag), key(id, "tag")));
        }
        ui.label("Layer");
        let mixed_layer = objs.iter().any(|x| x.layer != o.layer);
        let orig = (!mixed_layer).then_some(o.layer);
        let mut layer = orig;
        let names = &w.layers.0;
        let current = &names[o.layer as usize];
        egui::ComboBox::from_id_salt(("layer", id.to_bits()))
            .selected_text(if mixed_layer {
                "—".to_owned()
            } else if current.is_empty() {
                format!("{}", o.layer)
            } else {
                current.clone()
            })
            .width(combo_w)
            .show_ui(ui, |ui| {
                for (i, n) in names.iter().enumerate() {
                    if !n.is_empty() {
                        ui.selectable_value(&mut layer, Some(i as u8), format!("{i}: {n}"));
                    }
                }
                ui.separator();
                if ui.button("Add Layer...").clicked() {
                    open_tags = true;
                }
            });
        if layer != orig
            && let Some(l) = layer
        {
            edits.push((Edit::Layer(l), key(id, "layer")));
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
            st.project.show(&path);
            st.project.inspecting = false;
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

/// Label + X/Y/Z fields. `mixed` axes show "—". Returns which axes were edited.
fn vec3_row(
    ui: &mut egui::Ui,
    label: &str,
    v: &mut Vector3,
    mixed: [bool; 3],
    speed: f64,
    bold: bool,
) -> [bool; 3] {
    let mut changed = [false; 3];
    ui.horizontal(|ui| {
        let text = if bold {
            RichText::new(label).strong().color(Color32::WHITE)
        } else {
            RichText::new(label)
        };
        // Each axis: letter, field; `gap` between every widget. On a narrow Inspector the
        // label column gives up width so three readable fields still fit.
        let gap = ui.spacing().item_spacing.x;
        let letter = 9.0;
        let fields = 3.0 * (letter + 34.0) + 6.0 * gap;
        let row = avail(ui);
        prop_label_w(ui, text, label_width(row).min(row - fields).max(36.0));
        let wdt = ((avail(ui) - 3.0 * letter - 5.0 * gap) / 3.0).max(24.0);
        for (i, axis) in ["X", "Y", "Z"].iter().enumerate() {
            ui.add_sized(
                [letter, ROW_H],
                egui::Label::new(RichText::new(*axis).color(crate::scene_view::AXIS_COLORS[i])),
            );
            let mut d = egui::DragValue::new(&mut v[i]).speed(speed).max_decimals(4);
            if mixed[i] {
                d = d.custom_formatter(|_, _| "—".into());
            }
            changed[i] = ui.add_sized([wdt, ROW_H], d).changed();
        }
    });
    changed
}

/// The Euler angles shown for an object (kept while its rotation is unchanged, so typing
/// 190 doesn't jump to -170).
fn shown_euler(st: &mut EditorState, id: GameObjectId, q: Quaternion) -> Vector3 {
    match st.inspector.euler.get(&id) {
        Some((cq, e)) if cq.abs_diff_eq(q, 1e-6) => *e,
        _ => {
            let e = q.euler_angles();
            // Show -10 rather than 350, like Unity does for fresh values.
            let e = Vector3::new(wrap180(e.x), wrap180(e.y), wrap180(e.z));
            st.inspector.euler.insert(id, (q, e));
            e
        }
    }
}

/// Axes where any target differs from the first.
fn mixed_axes(values: &[Vector3]) -> [bool; 3] {
    let first = values.first().copied().unwrap_or_default();
    std::array::from_fn(|i| values.iter().any(|v| (v[i] - first[i]).abs() > 1e-5))
}

fn transform(
    ui: &mut egui::Ui,
    st: &mut EditorState,
    w: &World,
    targets: &[GameObjectId],
    edits: &mut Vec<(Edit, u64)>,
) {
    let id = *targets.last().expect("a target");
    let t = *w.transform(id).expect("alive");
    let ts: Vec<_> = targets
        .iter()
        .filter_map(|&x| w.transform(x).copied())
        .collect();
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
    let eulers: Vec<Vector3> = targets
        .iter()
        .zip(&ts)
        .map(|(&x, tr)| shown_euler(st, x, tr.local_rotation))
        .collect();
    let debug = st.inspector.debug;
    let resp = egui::CollapsingHeader::new(RichText::new("Transform").strong())
        .id_salt("transform")
        .default_open(true)
        .show(ui, |ui| {
            let mut p = t.local_position;
            let positions: Vec<_> = ts.iter().map(|x| x.local_position).collect();
            let m = vec3_row(
                ui,
                "Position",
                &mut p,
                mixed_axes(&positions),
                0.05,
                bold[0],
            );
            if m.contains(&true) {
                edits.push((Edit::Position(p, m), key(id, "position")));
            }
            if debug {
                // Unity's Debug Inspector shows the stored quaternion.
                let mut q = t.local_rotation.to_array();
                ui.horizontal(|ui| {
                    prop_label(ui, "Rotation");
                    let gap = ui.spacing().item_spacing.x;
                    let wdt = ((avail(ui) - 4.0 * 9.0 - 7.0 * gap) / 4.0).max(24.0);
                    let mut changed = false;
                    for (axis, c) in ["X", "Y", "Z", "W"].iter().zip(q.iter_mut()) {
                        ui.add_sized([9.0, ROW_H], egui::Label::new(*axis));
                        changed |= ui
                            .add_sized(
                                [wdt, ROW_H],
                                egui::DragValue::new(c).speed(0.005).max_decimals(5),
                            )
                            .changed();
                    }
                    if changed {
                        let nq = Quaternion::from_array(q).normalize();
                        edits.push((Edit::RotationQuat(nq), key(id, "rotation")));
                    }
                });
            } else {
                let mut e = *eulers.last().expect("a target");
                let m = vec3_row(ui, "Rotation", &mut e, mixed_axes(&eulers), 0.5, bold[1]);
                if m.contains(&true) {
                    edits.push((Edit::Rotation(e, m), key(id, "rotation")));
                }
            }
            let mut sc = t.local_scale;
            let scales: Vec<_> = ts.iter().map(|x| x.local_scale).collect();
            let m = vec3_row(ui, "Scale", &mut sc, mixed_axes(&scales), 0.02, bold[2]);
            if m.contains(&true) {
                edits.push((Edit::Scale(sc, m), key(id, "scale")));
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

/// Components of the active object that every target has (Unity shows only the shared ones
/// when several objects are selected); edits go to the matching component on each target.
fn components(
    ui: &mut egui::Ui,
    st: &mut EditorState,
    w: &World,
    targets: &[GameObjectId],
    edits: &mut Vec<(Edit, u64)>,
) {
    let id = *targets.last().expect("a target");
    let o = w.get(id).expect("alive");
    let link = o.prefab_link;
    let list = o.component_list();
    let count = list.len();
    let debug = st.inspector.debug;
    let mut hidden = 0;
    let mut per_type: HashMap<&str, usize> = HashMap::new();
    for (index, (cid, type_name, enabled)) in list.into_iter().enumerate() {
        let nth = {
            let n = per_type.entry(type_name).or_default();
            *n += 1;
            *n - 1
        };
        let slot = Slot { type_name, nth };
        let on_targets: Option<Vec<ComponentId>> =
            targets.iter().map(|&t| slot.resolve(w, t)).collect();
        let Some(on_targets) = on_targets else {
            hidden += 1;
            continue;
        };
        let target_comps: Vec<_> = targets
            .iter()
            .zip(&on_targets)
            .filter_map(|(&t, &c)| w.component_dyn(t, c))
            .collect();
        let mixed_enabled = targets.iter().zip(&on_targets).any(|(&t, &c)| {
            w.get(t).is_some_and(|x| {
                x.component_list()
                    .iter()
                    .any(|e| e.0 == c && e.2 != enabled)
            })
        });
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
        let values: Vec<(FieldInfo, Value, bool)> = info
            .fields
            .iter()
            .filter_map(|f| {
                let v = comp.get_field(f.name)?;
                let mixed = target_comps
                    .iter()
                    .any(|c| c.get_field(f.name).as_ref() != Some(&v));
                Some((*f, v, mixed))
            })
            .collect();
        ui.separator();
        let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
            ui.ctx(),
            egui::Id::new(("component", id.to_bits(), cid.0)),
            true,
        );
        if st.inspector.scroll_to.as_deref().is_some_and(|t| {
            t.eq_ignore_ascii_case(type_name) || t.eq_ignore_ascii_case(&nicify(type_name))
        }) {
            st.inspector.scroll_to = None;
            state.set_open(true);
            ui.scroll_to_cursor(Some(egui::Align::TOP));
        }
        let missing = w
            .component::<oxide_scene::MissingScript>(id, cid)
            .map(|m| m.script.clone());
        let header = state.show_header(ui, |ui| {
            let mut en = enabled;
            if ui
                .add(egui::Checkbox::without_text(&mut en).indeterminate(mixed_enabled))
                .changed()
            {
                edits.push((Edit::Enabled(slot, en), key(id, &format!("en{}", cid.0))));
            }
            match &missing {
                Some(script) => ui.label(
                    RichText::new(format!("{} (Missing Script)", nicify(script)))
                        .strong()
                        .color(Color32::from_rgb(255, 170, 60)),
                ),
                None => ui.label(RichText::new(nicify(type_name)).strong()),
            };
            if debug {
                ui.weak(format!("id {}", cid.0));
            }
        });
        let (_, header_resp, _) = header.body(|ui| {
            if let Some(script) = &missing {
                ui.weak(format!(
                    "The script `{script}` is not loaded (still compiling, renamed or deleted). Its saved values are kept."
                ));
            }
            if type_name == "LODGroup" && !debug {
                if targets.len() > 1 {
                    ui.weak("Multi-object editing of LOD Groups is not supported.");
                } else {
                    lod_group_editor(ui, w, id, cid, &st.selection, edits);
                }
                return;
            }
            if !debug
                && let Some(editor) = w
                    .try_resource::<oxide_engine::CustomEditors>()
                    .and_then(|e| e.get(type_name))
            {
                if targets.len() > 1 {
                    ui.weak("Multi-object editing is not supported by this component's Inspector.");
                } else {
                    custom_inspector(ui, w, id, cid, slot, editor, edits);
                }
                return;
            }
            if values.is_empty() {
                ui.weak("No editable fields.");
            }
            for (f, v, mixed) in values {
                let mut v2 = v.clone();
                // Numbers and checkboxes show "—" themselves; other mixed fields mark the label.
                let marks_itself = matches!(v, Value::Bool(_) | Value::Int(_) | Value::Float(_));
                let label = if mixed && !marks_itself {
                    format!("{} —", nicify(f.name))
                } else {
                    nicify(f.name)
                };
                let is_override = overridden.iter().any(|x| x == f.name);
                let changed = ui
                    .horizontal(|ui| {
                        let text = if is_override {
                            RichText::new(&label).strong().color(Color32::WHITE)
                        } else {
                            RichText::new(&label)
                        };
                        let lr = prop_label(ui, text);
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
                            mixed,
                            &format!("{}{}", cid.0, f.name),
                        )
                    })
                    .inner;
                if changed && v2 != v {
                    edits.push((
                        Edit::Field(slot, f.name.to_owned(), v2),
                        key(id, &format!("{}.{}", cid.0, f.name)),
                    ));
                }
            }
        });
        header_resp.response.context_menu(|ui| {
            if ui.button("Reset").clicked() {
                edits.push((Edit::Reset(slot), 0));
                ui.close();
            }
            if ui.button("Remove Component").clicked() {
                edits.push((Edit::Remove(slot), 0));
                ui.close();
            }
            if ui
                .add_enabled(index > 0, egui::Button::new("Move Up"))
                .clicked()
            {
                edits.push((Edit::Move(slot, -1), 0));
                ui.close();
            }
            if ui
                .add_enabled(index + 1 < count, egui::Button::new("Move Down"))
                .clicked()
            {
                edits.push((Edit::Move(slot, 1), 0));
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
                edits.push((Edit::Paste(slot, f.clone()), 0));
                ui.close();
            }
        });
    }
    if hidden > 0 {
        ui.separator();
        ui.weak(format!(
            "{hidden} component(s) not on every selected object are hidden."
        ));
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
        "Shader" => assets.shaders.iter().map(|(p, _)| p.to_owned()).collect(),
        _ => Vec::new(),
    };
    // Project files of the type too, loaded or not (they load when picked).
    let file_type = match type_name.unwrap_or("") {
        "Texture2D" => "Texture",
        "RuntimeAnimatorController" => "AnimatorController",
        t => t,
    };
    if assets.types.get(file_type).is_some() {
        v.extend(assets.paths_of_type(file_type));
    }
    v.sort();
    v.dedup();
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
    ui.push_id(salt, |ui| {
        for f in &info.fields {
            let Some(v) = obj.get_field(f.name) else {
                continue;
            };
            let mut v2 = v.clone();
            let edited = ui
                .horizontal(|ui| {
                    let lr = prop_label(ui, nicify(f.name));
                    if let Some(t) = f.tooltip {
                        lr.on_hover_text(t);
                    }
                    value_ui(
                        ui,
                        w,
                        f,
                        f.kind,
                        f.type_name,
                        &mut v2,
                        false,
                        &format!("{salt}{}", f.name),
                    )
                })
                .inner;
            if edited && v2 != v && obj.set_field(f.name, v2).is_ok() {
                changed = true;
            }
        }
    });
    changed
}

#[allow(clippy::too_many_arguments)]
fn value_ui(
    ui: &mut egui::Ui,
    w: &World,
    f: &FieldInfo,
    kind: ValueKind,
    type_name: Option<&str>,
    v: &mut Value,
    mixed: bool,
    salt: &str,
) -> bool {
    fn dash(d: egui::DragValue<'_>, mixed: bool) -> egui::DragValue<'_> {
        if mixed {
            d.custom_formatter(|_, _| "—".into())
        } else {
            d
        }
    }
    match v {
        Value::Bool(b) => ui
            .add(egui::Checkbox::without_text(b).indeterminate(mixed))
            .changed(),
        Value::Int(i) => match f.range {
            Some((lo, hi)) if kind == ValueKind::Int && !mixed => {
                fit_slider(ui);
                ui.add(egui::Slider::new(i, lo as i64..=hi as i64))
                    .changed()
            }
            _ => ui
                .add_sized([avail(ui), ROW_H], dash(egui::DragValue::new(i), mixed))
                .changed(),
        },
        Value::Float(x) => match f.range {
            Some((lo, hi)) if kind == ValueKind::Float && !mixed => {
                fit_slider(ui);
                ui.add(egui::Slider::new(x, lo..=hi)).changed()
            }
            _ => {
                let speed = (x.abs() * 0.01).clamp(0.001, 10.0);
                ui.add_sized(
                    [avail(ui), ROW_H],
                    dash(egui::DragValue::new(x).speed(speed).max_decimals(4), mixed),
                )
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
                .width(avail(ui))
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
                        changed |= value_ui(
                            ui,
                            w,
                            f,
                            elem,
                            type_name,
                            item,
                            false,
                            &format!("{salt}[{n}]"),
                        );
                    });
                }
            });
            changed
        }
        Value::Object(_) if crate::curve_editor::handles(type_name) => {
            crate::curve_editor::value_field(ui, type_name, v, salt)
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
                        changed |=
                            value_ui(ui, w, f, k, type_name, inner, false, &format!("{salt}.v"));
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
                        prop_label(ui, nicify(k));
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
                        changed |=
                            value_ui(ui, w, &sub, k2, None, item, false, &format!("{salt}.{k}"));
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

/// Makes the next slider fill the row, leaving room for its number box.
fn fit_slider(ui: &mut egui::Ui) {
    // The number box grows with its text ("0.000"), so keep room for about six digits.
    let number = ui.spacing().interact_size.x.max(52.0) + ui.spacing().item_spacing.x;
    ui.spacing_mut().slider_width = (avail(ui) - number).max(40.0);
}

fn drag_array<const N: usize>(ui: &mut egui::Ui, a: &mut [f32; N]) -> bool {
    let mut changed = false;
    let gap = ui.spacing().item_spacing.x;
    let wdt = ((avail(ui) - (N - 1) as f32 * gap) / N as f32).max(24.0);
    for x in a.iter_mut() {
        changed |= ui
            .add_sized(
                [wdt, ROW_H],
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
        .width(avail(ui).max(40.0))
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
        .width(avail(ui).max(40.0))
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

fn apply(st: &mut EditorState, w: &mut World, targets: &[GameObjectId], edits: Vec<(Edit, u64)>) {
    if edits.is_empty() {
        return;
    }
    let now = oxide_core::time::realtime_since_startup();
    for (edit, k) in edits {
        let label = match &edit {
            Edit::Field(_, f, _) => format!("Modify {}", nicify(f)),
            Edit::Remove(_) => "Remove Component".into(),
            Edit::Name(_) => "Rename".into(),
            Edit::Position(..)
            | Edit::Rotation(..)
            | Edit::Scale(..)
            | Edit::RotationQuat(_)
            | Edit::ResetTransform => "Transform".into(),
            _ => "Inspector".into(),
        };
        st.record(w, &label, (k != 0).then_some(k), now);
        for &id in targets {
            apply_one(st, w, id, &edit);
        }
    }
}

/// One edit on one object.
fn apply_one(st: &mut EditorState, w: &mut World, id: GameObjectId, edit: &Edit) {
    let set_axes = |cur: Vector3, v: Vector3, mask: [bool; 3]| {
        Vector3::new(
            if mask[0] { v.x } else { cur.x },
            if mask[1] { v.y } else { cur.y },
            if mask[2] { v.z } else { cur.z },
        )
    };
    let slot_id = |w: &World, s: &Slot| s.resolve(w, id);
    match edit {
        Edit::Field(slot, name, value) => {
            if let Some(cid) = slot_id(w, slot)
                && let Some(c) = w.component_dyn_mut(id, cid)
                && let Err(e) = c.set_field(name, value.clone())
            {
                st.status = format!("{name}: {e}");
            }
        }
        Edit::Enabled(slot, en) => {
            if let Some(cid) = slot_id(w, slot) {
                w.set_component_enabled(id, cid, *en);
            }
        }
        Edit::Remove(slot) => {
            if let Some(cid) = slot_id(w, slot) {
                w.remove_component_now(id, cid);
            }
        }
        Edit::Move(slot, d) => {
            if let Some(cid) = slot_id(w, slot) {
                w.move_component(id, cid, *d);
            }
        }
        Edit::Paste(slot, fields) => {
            if let Some(cid) = slot_id(w, slot)
                && let Some(c) = w.component_dyn_mut(id, cid)
            {
                for (k, v) in fields {
                    let _ = c.set_field(k, v.clone());
                }
            }
        }
        Edit::Reset(slot) => {
            let fresh = w.registry.create(slot.type_name);
            if let (Some(fresh), Some(cid)) = (fresh, slot_id(w, slot))
                && let Some(c) = w.component_dyn_mut(id, cid)
            {
                for (k, v) in fresh.fields() {
                    let _ = c.set_field(k, v);
                }
            }
        }
        Edit::Name(n) => {
            if let Some(o) = w.get_mut(id) {
                o.name = n.clone();
            }
        }
        Edit::Active(a) => w.set_active(id, *a),
        Edit::Static(s) => {
            if let Some(o) = w.get_mut(id) {
                o.static_flags = *s;
            }
        }
        Edit::Tag(t) => {
            if let Some(o) = w.get_mut(id) {
                o.tag = t.clone();
            }
        }
        Edit::Layer(l) => {
            if let Some(o) = w.get_mut(id) {
                o.layer = *l;
            }
        }
        Edit::Position(p, mask) => {
            if let Some(t) = w.transform_mut(id) {
                t.local_position = set_axes(t.local_position, *p, *mask);
            }
        }
        Edit::Rotation(e, mask) => {
            let Some(q0) = w.transform(id).map(|t| t.local_rotation) else {
                return;
            };
            let shown = set_axes(shown_euler(st, id, q0), *e, *mask);
            let q = Quaternion::euler_v(shown);
            st.inspector.euler.insert(id, (q, shown));
            if let Some(t) = w.transform_mut(id) {
                t.local_rotation = q;
            }
        }
        Edit::RotationQuat(q) => {
            if let Some(t) = w.transform_mut(id) {
                t.local_rotation = *q;
            }
        }
        Edit::Scale(sc, mask) => {
            if let Some(t) = w.transform_mut(id) {
                t.local_scale = set_axes(t.local_scale, *sc, *mask);
            }
        }
        Edit::ResetTransform => {
            if let Some(t) = w.transform_mut(id) {
                *t = oxide_scene::Transform::default();
            }
        }
    }
}

const LOD_SLOT: Slot = Slot {
    type_name: "LODGroup",
    nth: 0,
};

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
    let (bar, _) = ui.allocate_exact_size(egui::vec2(avail(ui), 26.0), egui::Sense::hover());
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
                ui.set_min_width(avail(ui));
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
                Edit::Field(LOD_SLOT, "size".into(), Value::Float(size as f64)),
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
        edits.push((Edit::Field(LOD_SLOT, "lods".into(), value), key(id, "lods")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_object_edits_reach_every_target() {
        let mut w = World::new();
        oxide_scene::register_builtin(&mut w.registry);
        let a = w.create_at("A", Vector3::new(1.0, 2.0, 3.0), Quaternion::IDENTITY);
        let b = w.create_at(
            "B",
            Vector3::new(4.0, 5.0, 6.0),
            Quaternion::euler(0.0, 90.0, 0.0),
        );
        w.add_component(a, oxide_scene::Light::default());
        w.add_component(b, oxide_scene::Light::default());
        let mut st = EditorState::new();
        let targets = [a, b];
        let light = Slot {
            type_name: "Light",
            nth: 0,
        };
        apply(
            &mut st,
            &mut w,
            &targets,
            vec![
                // Only X changes; each object keeps its own Y and Z.
                (
                    Edit::Position(Vector3::new(9.0, 0.0, 0.0), [true, false, false]),
                    0,
                ),
                // Only the X angle; B keeps its 90° yaw.
                (
                    Edit::Rotation(Vector3::new(30.0, 0.0, 0.0), [true, false, false]),
                    0,
                ),
                (Edit::Field(light, "intensity".into(), Value::Float(3.0)), 0),
                (Edit::Tag("Player".into()), 0),
            ],
        );
        assert_eq!(
            w.transform(a).unwrap().local_position,
            Vector3::new(9.0, 2.0, 3.0)
        );
        assert_eq!(
            w.transform(b).unwrap().local_position,
            Vector3::new(9.0, 5.0, 6.0)
        );
        let eb = w.transform(b).unwrap().local_rotation.euler_angles();
        assert!(
            (eb.x - 30.0).abs() < 1e-3 && (eb.y - 90.0).abs() < 1e-3,
            "{eb:?}"
        );
        for id in targets {
            assert_eq!(
                w.get_component::<oxide_scene::Light>(id).unwrap().intensity,
                3.0
            );
            assert_eq!(w.get(id).unwrap().tag, "Player");
        }
        // Undo restores both objects (each edit is one step; undo recreates the objects).
        let mut sel = targets.to_vec();
        for _ in 0..4 {
            sel = st.undo.undo(&mut w, &sel).expect("an undo step").1;
        }
        let b = w.find("B").expect("B restored");
        assert_eq!(
            w.transform(b).unwrap().local_position,
            Vector3::new(4.0, 5.0, 6.0)
        );
        assert_eq!(w.get(b).unwrap().tag, "Untagged");
    }

    #[test]
    fn mixed_axes_flags_differences() {
        let m = mixed_axes(&[Vector3::new(1.0, 2.0, 3.0), Vector3::new(1.0, 2.5, 3.0)]);
        assert_eq!(m, [false, true, false]);
    }

    #[test]
    fn nicify_names() {
        assert_eq!(nicify("field_of_view"), "Field Of View");
        assert_eq!(nicify("fieldOfView"), "Field Of View");
        assert_eq!(nicify("MeshRenderer"), "Mesh Renderer");
        assert_eq!(nicify("speed"), "Speed");
        assert_eq!(nicify("uv2"), "Uv2");
    }

    #[derive(Clone, Default, oxide_core::Reflect)]
    struct Speedy {
        speed: f32,
        note: String,
    }

    impl oxide_scene::Component for Speedy {}

    impl oxide_engine::CustomEditor for Speedy {
        fn on_inspector_gui(&mut self, gui: &mut oxide_engine::InspectorGui) {
            gui.draw_default_inspector(self);
            if self.speed > 10.0 {
                gui.help_box("Fast", oxide_engine::MessageType::Warning);
            }
            // As if a "Max Speed" button had been clicked.
            self.speed = 42.0;
        }
    }

    #[test]
    fn custom_inspectors_edit_through_the_normal_path() {
        let mut w = World::new();
        oxide_scene::register_builtin(&mut w.registry);
        w.registry.register::<Speedy>("Scripts/Speedy");
        let mut editors = oxide_engine::CustomEditors::default();
        editors.register::<Speedy>();
        w.resources.insert(editors);
        let id = w.create("Car");
        w.add_component(
            id,
            Speedy {
                speed: 12.0,
                note: "keep".into(),
            },
        );
        let cid = w.get(id).unwrap().component_list()[0].0;
        let editor = w
            .resource::<oxide_engine::CustomEditors>()
            .get("Speedy")
            .expect("registered");
        let slot = Slot {
            type_name: "Speedy",
            nth: 0,
        };
        let ctx = egui::Context::default();
        let mut edits = Vec::new();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            custom_inspector(ui, &w, id, cid, slot, editor, &mut edits);
        });
        out.textures_delta.clear();
        // Only the changed field becomes an edit; the component itself is untouched until
        // the edits are applied (with Undo).
        // egui may run the first frame twice (a sizing pass), repeating the same edit.
        let mut changed: Vec<String> = edits
            .iter()
            .map(|e| match &e.0 {
                Edit::Field(_, f, v) => format!("{f}={v:?}"),
                _ => "other".into(),
            })
            .collect();
        changed.dedup();
        assert_eq!(changed, ["speed=Float(42.0)"]);
        assert!(
            matches!(&edits[0].0, Edit::Field(s, f, Value::Float(v)) if *s == slot && f == "speed" && (*v - 42.0).abs() < 1e-6)
        );
        let c = w.get_component::<Speedy>(id).unwrap();
        assert_eq!((c.speed, c.note.as_str()), (12.0, "keep"));
        // Unloading the script library forgets its Inspector.
        let mut e = oxide_engine::CustomEditors::default();
        e.register::<Speedy>();
        e.remove(&["Speedy".to_owned()]);
        assert!(e.is_empty());
    }
}
