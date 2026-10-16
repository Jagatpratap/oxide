//! The Input Actions editor (Unity's Input Actions window): Action Maps | Actions with
//! their bindings | Properties. Bindings pick a control from a device menu or by
//! "Listen" (press the key or gamepad button). Edits save to the `.inputactions` asset
//! (Auto-Save, or Save Asset).
//! Reference: <https://docs.unity3d.com/Packages/com.unity.inputsystem@1.11/manual/ActionsEditor.html>

use crate::EditorState;
use egui::{Color32, RichText};
use oxide_assets::AssetServer;
use oxide_engine::Engine;
use oxide_input::actions::{key_path, parse_path};
use oxide_input::{
    GamepadButton, Input, InputAction, InputActionAsset, InputActionMap, InputActionType,
    InputBinding, InputComposite, InputControlScheme, InputControlType, KeyCode,
};

/// Which path field "Listen" fills in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Part {
    Path,
    Up,
    Down,
    Left,
    Right,
    Negative,
    Positive,
}

#[derive(Default)]
pub struct InputActionsWindow {
    /// The asset being edited and its last saved contents.
    path: Option<String>,
    asset: InputActionAsset,
    saved: InputActionAsset,
    map: usize,
    action: Option<usize>,
    binding: Option<usize>,
    pub auto_save: bool,
    listening: Option<Part>,
    /// Control scheme filter for the binding list (None = all).
    scheme: Option<String>,
    renaming: Option<(Target, String)>,
    new_scheme: String,
    error: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Target {
    Map(usize),
    Action(usize),
}

impl InputActionsWindow {
    /// Loads `path` for editing (keeps the current one if it is the same asset).
    pub fn open(&mut self, engine: &mut Engine, path: &str) {
        if self.path.as_deref() == Some(path) {
            return;
        }
        let assets = engine.world.resource_mut::<AssetServer>();
        let h = oxide_assets::Handle::<InputActionAsset>::from_path(path);
        match assets.data_asset(&h).cloned() {
            Some(a) => {
                *self = Self {
                    path: Some(path.to_owned()),
                    saved: a.clone(),
                    asset: a,
                    auto_save: true,
                    ..Default::default()
                };
            }
            None => self.error = Some(format!("cannot load {path}")),
        }
    }

    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    fn dirty(&self) -> bool {
        self.asset != self.saved
    }

    fn save(&mut self, engine: &mut Engine) {
        let Some(path) = self.path.clone() else {
            return;
        };
        match engine
            .world
            .resource_mut::<AssetServer>()
            .save_data(&path, Box::new(self.asset.clone()))
        {
            Ok(()) => {
                self.saved = self.asset.clone();
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
    }
}

/// Human-readable control name (`Space [Keyboard]`, `Left Stick [Gamepad]`).
pub fn display_path(path: &str) -> String {
    if path.is_empty() {
        return "<No Binding>".into();
    }
    match path.strip_prefix('<').and_then(|p| p.split_once(">/")) {
        Some((device, control)) => {
            let mut name = String::new();
            for (i, c) in control.replace('/', " ").chars().enumerate() {
                if i == 0 {
                    name.extend(c.to_uppercase());
                } else if c.is_uppercase() {
                    name.push(' ');
                    name.push(c);
                } else {
                    name.push(c);
                }
            }
            format!("{name} [{device}]")
        }
        None => path.to_owned(),
    }
}

/// An egui key as an input `KeyCode`.
fn key_code(k: egui::Key) -> Option<KeyCode> {
    let name = k.name();
    let mapped = match name {
        "ArrowUp" => "UpArrow",
        "ArrowDown" => "DownArrow",
        "ArrowLeft" => "LeftArrow",
        "ArrowRight" => "RightArrow",
        "Enter" => "Return",
        "OpenBracket" => "LeftBracket",
        "CloseBracket" => "RightBracket",
        "Backtick" => "BackQuote",
        n if n.len() == 1 && n.as_bytes()[0].is_ascii_digit() => {
            return KeyCode::parse(&format!("Alpha{n}"));
        }
        n if n.len() == 4 && n.starts_with("Num") => {
            return KeyCode::parse(&format!("Alpha{}", &n[3..]));
        }
        n => n,
    };
    KeyCode::parse(mapped)
}

fn gamepad_path(b: GamepadButton) -> &'static str {
    use GamepadButton::*;
    match b {
        South => "<Gamepad>/buttonSouth",
        East => "<Gamepad>/buttonEast",
        West => "<Gamepad>/buttonWest",
        North => "<Gamepad>/buttonNorth",
        LeftShoulder => "<Gamepad>/leftShoulder",
        RightShoulder => "<Gamepad>/rightShoulder",
        LeftTrigger => "<Gamepad>/leftTrigger",
        RightTrigger => "<Gamepad>/rightTrigger",
        Select => "<Gamepad>/select",
        Start => "<Gamepad>/start",
        LeftStick => "<Gamepad>/leftStickPress",
        RightStick => "<Gamepad>/rightStickPress",
        DpadUp => "<Gamepad>/dpad/up",
        DpadDown => "<Gamepad>/dpad/down",
        DpadLeft => "<Gamepad>/dpad/left",
        DpadRight => "<Gamepad>/dpad/right",
    }
}

/// Picks the keys shown in one Keyboard submenu.
type KeyFilter = dyn Fn(&KeyCode) -> bool;

/// Control picker: Keyboard / Mouse / Gamepad submenus. Returns the chosen path.
fn path_menu(ui: &mut egui::Ui) -> Option<String> {
    let mut picked = None;
    ui.menu_button("Keyboard", |ui| {
        let groups: [(&str, &KeyFilter); 5] = [
            ("Letters", &|k| format!("{k:?}").len() == 1),
            ("Digits", &|k| format!("{k:?}").starts_with("Alpha")),
            ("Function Keys", &|k| {
                let n = format!("{k:?}");
                n.starts_with('F') && n.len() <= 3 && n[1..].chars().all(|c| c.is_ascii_digit())
            }),
            ("Arrows & Navigation", &|k| {
                let n = format!("{k:?}");
                n.ends_with("Arrow")
                    || ["Home", "End", "PageUp", "PageDown", "Insert", "Delete"]
                        .contains(&n.as_str())
            }),
            ("Modifiers", &|k| {
                let n = format!("{k:?}");
                n.contains("Shift")
                    || n.contains("Control")
                    || n.contains("Alt")
                    || n.contains("Command")
            }),
        ];
        for (name, f) in groups {
            ui.menu_button(name, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .show(ui, |ui| {
                        for k in KeyCode::ALL.iter().filter(|k| f(k)) {
                            let p = key_path(*k);
                            if ui.button(display_path(&p)).clicked() {
                                picked = Some(p);
                                ui.close();
                            }
                        }
                    });
            });
        }
        ui.menu_button("Other", |ui| {
            egui::ScrollArea::vertical()
                .max_height(320.0)
                .show(ui, |ui| {
                    for k in KeyCode::ALL
                        .iter()
                        .filter(|k| !groups.iter().any(|(_, f)| f(k)))
                    {
                        let p = key_path(*k);
                        if ui.button(display_path(&p)).clicked() {
                            picked = Some(p);
                            ui.close();
                        }
                    }
                });
        });
    });
    ui.menu_button("Mouse", |ui| {
        for c in [
            "leftButton",
            "rightButton",
            "middleButton",
            "delta",
            "scroll",
            "position",
        ] {
            let p = format!("<Mouse>/{c}");
            if ui.button(display_path(&p)).clicked() {
                picked = Some(p);
                ui.close();
            }
        }
    });
    ui.menu_button("Gamepad", |ui| {
        for c in [
            "leftStick",
            "rightStick",
            "dpad",
            "buttonSouth",
            "buttonEast",
            "buttonWest",
            "buttonNorth",
            "leftShoulder",
            "rightShoulder",
            "leftTrigger",
            "rightTrigger",
            "start",
            "select",
            "leftStickPress",
            "rightStickPress",
            "leftStick/x",
            "leftStick/y",
            "rightStick/x",
            "rightStick/y",
            "dpad/up",
            "dpad/down",
            "dpad/left",
            "dpad/right",
        ] {
            let p = format!("<Gamepad>/{c}");
            if ui.button(display_path(&p)).clicked() {
                picked = Some(p);
                ui.close();
            }
        }
    });
    picked
}

pub fn window_ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    let w = &mut st.input_actions;
    let Some(path) = w.path.clone() else {
        ui.weak(
            "Double-click an Input Actions asset (.inputactions) in the Project window to edit it.",
        );
        return;
    };
    let before = w.asset.clone();

    // Toolbar.
    ui.horizontal(|ui| {
        let name = path.rsplit('/').next().unwrap_or(&path);
        ui.label(
            RichText::new(if w.dirty() {
                format!("{name} *")
            } else {
                name.to_owned()
            })
            .strong(),
        );
        ui.separator();
        let schemes: Vec<String> = w
            .asset
            .control_schemes
            .iter()
            .map(|s| s.name.clone())
            .collect();
        egui::ComboBox::from_id_salt("ia-scheme")
            .selected_text(
                w.scheme
                    .clone()
                    .unwrap_or_else(|| "All Control Schemes".into()),
            )
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(w.scheme.is_none(), "All Control Schemes")
                    .clicked()
                {
                    w.scheme = None;
                }
                for s in &schemes {
                    if ui
                        .selectable_label(w.scheme.as_ref() == Some(s), s)
                        .clicked()
                    {
                        w.scheme = Some(s.clone());
                    }
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut w.new_scheme)
                            .hint_text("New scheme")
                            .desired_width(110.0),
                    );
                    if ui.button("Add").clicked() && !w.new_scheme.trim().is_empty() {
                        w.asset.control_schemes.push(InputControlScheme {
                            name: w.new_scheme.trim().to_owned(),
                            devices: vec!["<Keyboard>".into(), "<Mouse>".into()],
                        });
                        w.new_scheme.clear();
                    }
                });
            });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.checkbox(&mut w.auto_save, "Auto-Save");
            if ui
                .add_enabled(w.dirty(), egui::Button::new("Save Asset"))
                .clicked()
            {
                w.save(engine);
            }
        });
    });
    if let Some(e) = &w.error {
        ui.colored_label(Color32::from_rgb(230, 120, 100), e);
    }
    ui.separator();

    let w = &mut st.input_actions;
    w.map = w.map.min(w.asset.maps.len().saturating_sub(1));
    let h = ui.available_height();
    let total = ui.available_width();
    let maps_w = (total * 0.22).clamp(140.0, 220.0);
    let props_w = (total * 0.36).clamp(240.0, 380.0);
    let actions_w = (total - maps_w - props_w - 24.0).max(160.0);
    ui.horizontal_top(|ui| {
        column(ui, maps_w, h, "Action Maps", |ui| maps_column(ui, w));
        ui.separator();
        column(ui, actions_w, h, "Actions", |ui| actions_column(ui, w));
        ui.separator();
        column(ui, props_w, h, "Properties", |ui| {
            properties_column(ui, w, engine)
        });
    });

    let w = &mut st.input_actions;
    if w.asset != before && w.auto_save {
        w.save(engine);
    }
}

fn column(
    ui: &mut egui::Ui,
    width: f32,
    height: f32,
    title: &str,
    body: impl FnOnce(&mut egui::Ui),
) {
    ui.allocate_ui(egui::vec2(width, height), |ui| {
        ui.set_width(width);
        ui.vertical(|ui| {
            ui.label(RichText::new(title).strong());
            egui::ScrollArea::vertical()
                .id_salt(title)
                .auto_shrink([false, false])
                .show(ui, body);
        });
    });
}

fn unique(name: &str, taken: impl Iterator<Item = String>) -> String {
    let taken: Vec<String> = taken.collect();
    if !taken.iter().any(|t| t == name) {
        return name.to_owned();
    }
    (1..)
        .map(|i| format!("{name}{i}"))
        .find(|n| !taken.contains(n))
        .unwrap_or_else(|| name.to_owned())
}

/// Inline rename field; returns true when done.
fn rename_field(ui: &mut egui::Ui, text: &mut String) -> bool {
    let out = egui::TextEdit::singleline(text)
        .desired_width(f32::INFINITY)
        .show(ui);
    let r = out.response;
    if !r.has_focus() && !r.lost_focus() {
        r.request_focus();
        let mut state = out.state;
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(text.chars().count()),
            )));
        state.store(ui.ctx(), r.id);
    }
    r.lost_focus()
}

fn maps_column(ui: &mut egui::Ui, w: &mut InputActionsWindow) {
    if ui.small_button("+ Add Action Map").clicked() {
        let name = unique(
            "New action map",
            w.asset.maps.iter().map(|m| m.name.clone()),
        );
        w.asset.maps.push(InputActionMap {
            name,
            actions: Vec::new(),
        });
        w.map = w.asset.maps.len() - 1;
        w.action = None;
        w.binding = None;
        w.renaming = Some((Target::Map(w.map), w.asset.maps[w.map].name.clone()));
    }
    let mut remove = None;
    let mut duplicate = None;
    for i in 0..w.asset.maps.len() {
        if let Some((Target::Map(r), text)) = &mut w.renaming
            && *r == i
        {
            if rename_field(ui, text) {
                if !text.trim().is_empty() {
                    w.asset.maps[i].name = text.trim().to_owned();
                }
                w.renaming = None;
            }
            continue;
        }
        let r = ui.selectable_label(w.map == i, &w.asset.maps[i].name);
        if r.clicked() {
            w.map = i;
            w.action = None;
            w.binding = None;
        }
        if r.double_clicked() {
            w.renaming = Some((Target::Map(i), w.asset.maps[i].name.clone()));
        }
        r.context_menu(|ui| {
            if ui.button("Rename").clicked() {
                w.renaming = Some((Target::Map(i), w.asset.maps[i].name.clone()));
                ui.close();
            }
            if ui.button("Duplicate").clicked() {
                duplicate = Some(i);
                ui.close();
            }
            if ui.button("Delete").clicked() {
                remove = Some(i);
                ui.close();
            }
        });
    }
    if let Some(i) = duplicate {
        let mut m = w.asset.maps[i].clone();
        m.name = unique(&m.name, w.asset.maps.iter().map(|m| m.name.clone()));
        w.asset.maps.insert(i + 1, m);
    }
    if let Some(i) = remove {
        w.asset.maps.remove(i);
        w.map = w.map.min(w.asset.maps.len().saturating_sub(1));
        w.action = None;
        w.binding = None;
    }
}

fn in_scheme(b: &InputBinding, scheme: &Option<String>) -> bool {
    match scheme {
        None => true,
        Some(s) => b.groups.is_empty() || b.groups.split(';').any(|g| g.trim() == s),
    }
}

fn actions_column(ui: &mut egui::Ui, w: &mut InputActionsWindow) {
    let Some(map) = w.asset.maps.get_mut(w.map) else {
        ui.weak("Add an action map first.");
        return;
    };
    if ui.small_button("+ Add Action").clicked() {
        let name = unique("New action", map.actions.iter().map(|a| a.name.clone()));
        map.actions.push(
            InputAction::new(&name, InputActionType::Button, InputControlType::Button)
                .bind(InputBinding::default()),
        );
        w.action = Some(map.actions.len() - 1);
        w.binding = None;
        w.renaming = Some((Target::Action(map.actions.len() - 1), name));
    }
    let mut remove_action = None;
    let mut remove_binding = None;
    let mut duplicate = None;
    for ai in 0..map.actions.len() {
        if let Some((Target::Action(r), text)) = &mut w.renaming
            && *r == ai
        {
            if rename_field(ui, text) {
                if !text.trim().is_empty() {
                    map.actions[ai].name = text.trim().to_owned();
                }
                w.renaming = None;
            }
            continue;
        }
        let id = ui.id().with(("ia-action", w.map, ai));
        let state =
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true);
        let header = state.show_header(ui, |ui| {
            let sel = w.action == Some(ai) && w.binding.is_none();
            let r = ui.selectable_label(sel, RichText::new(&map.actions[ai].name).strong());
            if r.clicked() {
                w.action = Some(ai);
                w.binding = None;
            }
            if r.double_clicked() {
                w.renaming = Some((Target::Action(ai), map.actions[ai].name.clone()));
            }
            r.context_menu(|ui| {
                if ui.button("Rename").clicked() {
                    w.renaming = Some((Target::Action(ai), map.actions[ai].name.clone()));
                    ui.close();
                }
                if ui.button("Duplicate").clicked() {
                    duplicate = Some(ai);
                    ui.close();
                }
                if ui.button("Delete").clicked() {
                    remove_action = Some(ai);
                    ui.close();
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button("+", |ui| {
                    let a = &mut map.actions[ai];
                    let mut add = |b: InputBinding| {
                        a.bindings.push(b);
                        w.action = Some(ai);
                        w.binding = Some(a.bindings.len() - 1);
                    };
                    if ui.button("Add Binding").clicked() {
                        add(InputBinding::default());
                        ui.close();
                    }
                    if ui.button("Add Up\\Down\\Left\\Right Composite").clicked() {
                        add(InputBinding::vector2("", "", "", ""));
                        ui.close();
                    }
                    if ui.button("Add Positive\\Negative Binding").clicked() {
                        add(InputBinding::axis("", ""));
                        ui.close();
                    }
                });
            });
        });
        header.body(|ui| {
            let a = &map.actions[ai];
            for (bi, b) in a.bindings.iter().enumerate() {
                if !in_scheme(b, &w.scheme) {
                    continue;
                }
                let sel = w.action == Some(ai) && w.binding == Some(bi);
                let text = match b.composite {
                    InputComposite::None => display_path(&b.path),
                    _ => b.display(),
                };
                let r = ui.selectable_label(sel, text);
                if r.clicked() {
                    w.action = Some(ai);
                    w.binding = Some(bi);
                }
                r.context_menu(|ui| {
                    if ui.button("Delete").clicked() {
                        remove_binding = Some((ai, bi));
                        ui.close();
                    }
                });
                // Composite parts, indented under the composite.
                let parts: Vec<(&str, &String)> = match b.composite {
                    InputComposite::Vector2D => vec![
                        ("Up", &b.up),
                        ("Down", &b.down),
                        ("Left", &b.left),
                        ("Right", &b.right),
                    ],
                    InputComposite::Axis1D => {
                        vec![("Negative", &b.negative), ("Positive", &b.positive)]
                    }
                    InputComposite::None => vec![],
                };
                for (name, p) in parts {
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        ui.weak(format!("{name}: {}", display_path(p)));
                    });
                }
            }
        });
    }
    if let Some(i) = duplicate {
        let mut a = map.actions[i].clone();
        a.name = unique(&a.name, map.actions.iter().map(|a| a.name.clone()));
        map.actions.insert(i + 1, a);
    }
    if let Some(i) = remove_action {
        map.actions.remove(i);
        w.action = None;
        w.binding = None;
    }
    if let Some((ai, bi)) = remove_binding {
        map.actions[ai].bindings.remove(bi);
        w.binding = None;
    }
}

/// A binding path row: the control, a device menu and Listen.
fn path_row(
    ui: &mut egui::Ui,
    label: &str,
    path: &mut String,
    part: Part,
    listening: &mut Option<Part>,
    engine: &Engine,
) {
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, label);
        if *listening == Some(part) {
            ui.label(
                RichText::new("Listening… press a key or button")
                    .color(Color32::from_rgb(240, 200, 90)),
            );
            // Keyboard keys (and Shift/Ctrl/Alt/Cmd on their own), or gamepad buttons.
            let picked = ui.input(|i| {
                i.events
                    .iter()
                    .find_map(|e| match e {
                        egui::Event::Key {
                            key, pressed: true, ..
                        } => key_code(*key).map(key_path),
                        _ => None,
                    })
                    .or_else(|| {
                        let m = i.modifiers;
                        if m.shift {
                            Some(key_path(KeyCode::LeftShift))
                        } else if m.alt {
                            Some(key_path(KeyCode::LeftAlt))
                        } else if m.mac_cmd {
                            Some(key_path(KeyCode::LeftCommand))
                        } else if m.ctrl {
                            Some(key_path(KeyCode::LeftControl))
                        } else {
                            None
                        }
                    })
            });
            let pad = engine
                .world
                .resource::<Input>()
                .gamepads
                .iter()
                .find_map(|g| g.buttons_down.iter().next().copied())
                .map(|b| gamepad_path(b).to_owned());
            if let Some(p) = picked.or(pad) {
                *path = p;
                *listening = None;
            }
            if ui.small_button("Cancel").clicked() {
                *listening = None;
            }
            crate::shortcuts::claim_keys(ui.ctx());
            return;
        }
        let valid = path.is_empty() || parse_path(path).is_some();
        let text = RichText::new(display_path(path));
        ui.menu_button(
            if valid {
                text
            } else {
                text.color(Color32::from_rgb(230, 120, 100))
            },
            |ui| {
                if let Some(p) = path_menu(ui) {
                    *path = p;
                }
            },
        )
        .response
        .on_hover_text(path.as_str());
        if ui.small_button("Listen").clicked() {
            *listening = Some(part);
        }
    });
}

fn text_row(ui: &mut egui::Ui, label: &str, text: &mut String, hint: &str) {
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, label);
        ui.add(
            egui::TextEdit::singleline(text)
                .hint_text(hint)
                .desired_width(f32::INFINITY),
        );
    });
}

fn properties_column(ui: &mut egui::Ui, w: &mut InputActionsWindow, engine: &Engine) {
    let schemes: Vec<String> = w
        .asset
        .control_schemes
        .iter()
        .map(|s| s.name.clone())
        .collect();
    let Some(map) = w.asset.maps.get_mut(w.map) else {
        return;
    };
    let Some(ai) = w.action.filter(|&a| a < map.actions.len()) else {
        ui.weak("Select an action or binding.");
        return;
    };
    let action = &mut map.actions[ai];
    match w.binding.filter(|&b| b < action.bindings.len()) {
        None => {
            ui.label(RichText::new("Action").strong());
            ui.horizontal(|ui| {
                crate::inspector::prop_label(ui, "Action Type");
                egui::ComboBox::from_id_salt("ia-type")
                    .selected_text(format!("{:?}", action.action_type))
                    .show_ui(ui, |ui| {
                        for t in [
                            InputActionType::Value,
                            InputActionType::Button,
                            InputActionType::PassThrough,
                        ] {
                            ui.selectable_value(&mut action.action_type, t, format!("{t:?}"));
                        }
                    });
            });
            ui.horizontal(|ui| {
                crate::inspector::prop_label(ui, "Control Type");
                egui::ComboBox::from_id_salt("ia-control")
                    .selected_text(format!("{:?}", action.control_type))
                    .show_ui(ui, |ui| {
                        for t in [
                            InputControlType::Button,
                            InputControlType::Axis,
                            InputControlType::Vector2,
                        ] {
                            ui.selectable_value(&mut action.control_type, t, format!("{t:?}"));
                        }
                    });
            });
            text_row(
                ui,
                "Interactions",
                &mut action.interactions,
                "hold(duration=0.4), tap",
            );
            text_row(
                ui,
                "Processors",
                &mut action.processors,
                "normalize, invertY, scale(factor=2)",
            );
        }
        Some(bi) => {
            let b = &mut action.bindings[bi];
            ui.label(RichText::new("Binding").strong());
            ui.horizontal(|ui| {
                crate::inspector::prop_label(ui, "Composite");
                egui::ComboBox::from_id_salt("ia-composite")
                    .selected_text(match b.composite {
                        InputComposite::None => "None",
                        InputComposite::Vector2D => "2D Vector",
                        InputComposite::Axis1D => "1D Axis",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut b.composite, InputComposite::None, "None");
                        ui.selectable_value(
                            &mut b.composite,
                            InputComposite::Vector2D,
                            "2D Vector",
                        );
                        ui.selectable_value(&mut b.composite, InputComposite::Axis1D, "1D Axis");
                    });
            });
            let l = &mut w.listening;
            match b.composite {
                InputComposite::None => path_row(ui, "Path", &mut b.path, Part::Path, l, engine),
                InputComposite::Vector2D => {
                    path_row(ui, "Up", &mut b.up, Part::Up, l, engine);
                    path_row(ui, "Down", &mut b.down, Part::Down, l, engine);
                    path_row(ui, "Left", &mut b.left, Part::Left, l, engine);
                    path_row(ui, "Right", &mut b.right, Part::Right, l, engine);
                }
                InputComposite::Axis1D => {
                    path_row(ui, "Negative", &mut b.negative, Part::Negative, l, engine);
                    path_row(ui, "Positive", &mut b.positive, Part::Positive, l, engine);
                }
            }
            text_row(
                ui,
                "Interactions",
                &mut b.interactions,
                "hold(duration=0.4), tap",
            );
            text_row(
                ui,
                "Processors",
                &mut b.processors,
                "normalize, invertY, stickDeadzone",
            );
            if !schemes.is_empty() {
                ui.add_space(6.0);
                ui.label(RichText::new("Use in control scheme").strong());
                for s in &schemes {
                    let mut groups: Vec<String> = b
                        .groups
                        .split(';')
                        .map(|g| g.trim().to_owned())
                        .filter(|g| !g.is_empty())
                        .collect();
                    let mut on = groups.contains(s);
                    if ui.checkbox(&mut on, s).changed() {
                        if on {
                            groups.push(s.clone());
                        } else {
                            groups.retain(|g| g != s);
                        }
                        b.groups = groups.join(";");
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_names() {
        assert_eq!(display_path("<Keyboard>/space"), "Space [Keyboard]");
        assert_eq!(display_path("<Gamepad>/leftStick"), "Left Stick [Gamepad]");
        assert_eq!(display_path("<Gamepad>/dpad/up"), "Dpad up [Gamepad]");
        assert_eq!(display_path(""), "<No Binding>");
    }

    #[test]
    fn egui_keys_map_to_key_codes() {
        assert_eq!(key_code(egui::Key::A), Some(KeyCode::A));
        assert_eq!(key_code(egui::Key::Num1), Some(KeyCode::Alpha1));
        assert_eq!(key_code(egui::Key::ArrowUp), Some(KeyCode::UpArrow));
        assert_eq!(key_code(egui::Key::Enter), Some(KeyCode::Return));
        assert_eq!(key_code(egui::Key::Space), Some(KeyCode::Space));
    }
}
