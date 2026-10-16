//! Custom Inspectors for script components (Unity's `[CustomEditor]` and `OnInspectorGUI`).
//!
//! A component implements [`CustomEditor`] and is listed under `editors:` in its Scripts
//! library's `scripts!` block. The editor then draws it with
//! [`on_inspector_gui`](CustomEditor::on_inspector_gui) instead of the default field list.
//! Changes made there go through Undo, prefab overrides and saving like any other edit.
//! Player builds compile the same code and never call it.
//!
//! ```ignore
//! impl CustomEditor for Spinner {
//!     fn on_inspector_gui(&mut self, gui: &mut InspectorGui) {
//!         gui.draw_default_inspector(self);
//!         if self.speed > 500.0 {
//!             gui.help_box("Very fast spinners may tunnel through colliders.", MessageType::Warning);
//!         }
//!         if gui.button("Reset Speed") {
//!             self.speed = 90.0;
//!         }
//!     }
//! }
//!
//! oxide::scripts! { Spinner => "Scripts/Spinner"; editors: Spinner }
//! ```
//! Reference: <https://docs.unity3d.com/Manual/editor-CustomEditors.html>

use oxide_core::reflect::Reflect;
use oxide_scene::component::downcast_mut;
use oxide_scene::{Component, GameObjectId, World};
use oxide_ui::egui;
use std::collections::HashMap;

/// Unity `MessageType` for [`InspectorGui::help_box`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageType {
    Info,
    Warning,
    Error,
}

/// Draws the default field rows of a reflected value (supplied by the editor).
pub type DefaultInspectorFn<'a> = &'a dyn Fn(&mut egui::Ui, &World, &mut dyn Reflect, &str) -> bool;

/// What a custom Inspector draws with: the egui `Ui` plus Unity-style helpers
/// (`EditorGUILayout`).
pub struct InspectorGui<'a> {
    ui: &'a mut egui::Ui,
    world: &'a World,
    object: GameObjectId,
    draw_default: DefaultInspectorFn<'a>,
    salt: String,
}

impl<'a> InspectorGui<'a> {
    /// Used by the editor.
    pub fn new(
        ui: &'a mut egui::Ui,
        world: &'a World,
        object: GameObjectId,
        draw_default: DefaultInspectorFn<'a>,
        salt: String,
    ) -> Self {
        Self {
            ui,
            world,
            object,
            draw_default,
            salt,
        }
    }

    /// The egui `Ui` for anything the helpers don't cover.
    pub fn ui(&mut self) -> &mut egui::Ui {
        self.ui
    }

    /// The scene, read-only (edit the component through `self`).
    pub fn world(&self) -> &World {
        self.world
    }

    /// The object being inspected (Unity `target.gameObject`).
    pub fn game_object(&self) -> GameObjectId {
        self.object
    }

    /// Unity `DrawDefaultInspector`: the usual field rows. Returns whether a field changed.
    pub fn draw_default_inspector(&mut self, target: &mut dyn Reflect) -> bool {
        (self.draw_default)(self.ui, self.world, target, &self.salt)
    }

    /// Unity `EditorGUILayout.HelpBox`.
    pub fn help_box(&mut self, text: &str, kind: MessageType) {
        let (color, icon) = match kind {
            MessageType::Info => (egui::Color32::from_rgb(120, 170, 230), "ℹ"),
            MessageType::Warning => (egui::Color32::from_rgb(240, 190, 80), "⚠"),
            MessageType::Error => (egui::Color32::from_rgb(240, 110, 90), "⊗"),
        };
        egui::Frame::NONE
            .fill(egui::Color32::from_gray(48))
            .stroke(egui::Stroke::new(1.0, color.gamma_multiply(0.6)))
            .corner_radius(3.0)
            .inner_margin(6.0)
            .show(self.ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(color, icon);
                    ui.label(text);
                });
            });
    }

    /// Unity `[Header]` / `EditorGUILayout.LabelField` with a bold style.
    pub fn header(&mut self, text: &str) {
        self.ui.add_space(4.0);
        self.ui.label(egui::RichText::new(text).strong());
    }

    pub fn label(&mut self, text: &str) {
        self.ui.label(text);
    }

    pub fn space(&mut self) {
        self.ui.add_space(8.0);
    }

    /// Unity `GUILayout.Button`.
    pub fn button(&mut self, text: &str) -> bool {
        self.ui.button(text).clicked()
    }

    fn row<R>(&mut self, label: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
        self.ui
            .horizontal(|ui| {
                ui.add_sized([130.0, 18.0], egui::Label::new(label).truncate());
                add(ui)
            })
            .inner
    }

    /// Unity `EditorGUILayout.FloatField`.
    pub fn float_field(&mut self, label: &str, value: &mut f32) -> bool {
        self.row(label, |ui| {
            ui.add(egui::DragValue::new(value).speed(0.05)).changed()
        })
    }

    /// Unity `EditorGUILayout.IntField`.
    pub fn int_field(&mut self, label: &str, value: &mut i32) -> bool {
        self.row(label, |ui| ui.add(egui::DragValue::new(value)).changed())
    }

    /// Unity `EditorGUILayout.Slider`.
    pub fn slider(&mut self, label: &str, value: &mut f32, min: f32, max: f32) -> bool {
        self.row(label, |ui| {
            ui.add(egui::Slider::new(value, min..=max)).changed()
        })
    }

    /// Unity `EditorGUILayout.Toggle`.
    pub fn toggle(&mut self, label: &str, value: &mut bool) -> bool {
        self.row(label, |ui| ui.checkbox(value, "").changed())
    }

    /// Unity `EditorGUILayout.TextField`.
    pub fn text_field(&mut self, label: &str, value: &mut String) -> bool {
        self.row(label, |ui| ui.text_edit_singleline(value).changed())
    }

    /// Unity `EditorGUILayout.Vector3Field`.
    pub fn vector3_field(&mut self, label: &str, value: &mut oxide_math::Vector3) -> bool {
        self.row(label, |ui| {
            let mut changed = false;
            for v in [&mut value.x, &mut value.y, &mut value.z] {
                changed |= ui.add(egui::DragValue::new(v).speed(0.05)).changed();
            }
            changed
        })
    }

    /// Unity `EditorGUILayout.ColorField` (sRGB).
    pub fn color_field(&mut self, label: &str, value: &mut oxide_math::Color) -> bool {
        self.row(label, |ui| {
            let mut c = [value.r, value.g, value.b, value.a];
            let changed = ui.color_edit_button_rgba_unmultiplied(&mut c).changed();
            if changed {
                *value = oxide_math::Color::new(c[0], c[1], c[2], c[3]);
            }
            changed
        })
    }

    /// Unity `EditorGUILayout.Popup`: picks one of `options`.
    pub fn popup(&mut self, label: &str, index: &mut usize, options: &[&str]) -> bool {
        let salt = format!("{}{label}", self.salt);
        self.row(label, |ui| {
            let before = *index;
            egui::ComboBox::from_id_salt(salt)
                .selected_text(options.get(*index).copied().unwrap_or(""))
                .show_ui(ui, |ui| {
                    for (i, o) in options.iter().enumerate() {
                        ui.selectable_value(index, i, *o);
                    }
                });
            *index != before
        })
    }
}

/// A component with its own Inspector (Unity `Editor.OnInspectorGUI`).
pub trait CustomEditor: Component {
    fn on_inspector_gui(&mut self, gui: &mut InspectorGui);
}

/// Runs the custom Inspector of a type-erased component.
pub type InspectorFn = fn(&mut dyn Component, &mut InspectorGui);

fn call<T: CustomEditor>(c: &mut dyn Component, gui: &mut InspectorGui) {
    if let Some(t) = downcast_mut::<T>(c) {
        t.on_inspector_gui(gui);
    }
}

/// Custom Inspectors by component type name (a World resource).
#[derive(Default)]
pub struct CustomEditors {
    editors: HashMap<String, InspectorFn>,
}

impl CustomEditors {
    pub fn register<T: CustomEditor + Default>(&mut self) {
        let name = T::default().type_info().name.to_owned();
        self.editors.insert(name, call::<T>);
    }

    pub fn get(&self, type_name: &str) -> Option<InspectorFn> {
        self.editors.get(type_name).copied()
    }

    /// Forgets the editors of these types (a script library about to be reloaded).
    pub fn remove(&mut self, type_names: &[String]) {
        self.editors.retain(|n, _| !type_names.contains(n));
    }

    pub fn len(&self) -> usize {
        self.editors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.editors.is_empty()
    }
}
