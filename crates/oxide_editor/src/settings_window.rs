//! Project Settings window (Unity Edit → Project Settings): Tags and Layers, Time, Physics,
//! Quality, Input Manager and Player. Edits apply to the running engine at once and are saved
//! to `ProjectSettings/` shortly after the last change.
//! Reference: <https://docs.unity3d.com/Manual/comp-ManagerGroup.html>

use crate::EditorState;
use crate::player_build::BuildSettings;
use egui::{Color32, RichText, vec2};
use oxide_engine::Engine;
use oxide_engine::project_settings::{self, ProjectSettings};
use oxide_input::{
    AxisSource, GamepadAxis, GamepadButton, InputAxis, InputButton, KeyCode, MouseButton,
};
use oxide_scene::{BUILTIN_TAGS, QualityLevel};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Page {
    #[default]
    TagsAndLayers,
    Time,
    Physics,
    Quality,
    Input,
    Player,
}

pub const PAGES: [(Page, &str); 6] = [
    (Page::Input, "Input Manager"),
    (Page::Physics, "Physics"),
    (Page::Player, "Player"),
    (Page::Quality, "Quality"),
    (Page::TagsAndLayers, "Tags and Layers"),
    (Page::Time, "Time"),
];

/// Unity's built-in layers can't be renamed.
const BUILTIN_LAYERS: [usize; 5] = [0, 1, 2, 4, 5];
/// Seconds after the last edit before files are written.
const SAVE_DELAY: f64 = 0.5;

#[derive(Default)]
pub struct SettingsState {
    pub page: Page,
    quality_selected: Option<usize>,
    new_tag: String,
    save_at: Option<f64>,
    build: Option<BuildSettings>,
}

/// Page list on the left, the chosen page on the right (Project Settings and Preferences).
pub fn two_pane<P: Copy + PartialEq>(
    ui: &mut egui::Ui,
    salt: &str,
    pages: &[(P, &str)],
    current: &mut P,
    body: impl FnOnce(&mut egui::Ui, P),
) {
    let rect = ui.available_rect_before_wrap();
    let list_w = 150.0f32.min(rect.width() * 0.35);
    let (left, right) = rect.split_left_right_at_x(rect.left() + list_w);
    ui.painter().rect_filled(left, 0.0, Color32::from_gray(48));
    ui.scope_builder(egui::UiBuilder::new().max_rect(left.shrink(4.0)), |ui| {
        for (p, name) in pages {
            if ui
                .add_sized(
                    vec2(ui.available_width(), 20.0),
                    egui::Button::selectable(*current == *p, *name),
                )
                .clicked()
            {
                *current = *p;
            }
        }
    });
    let title = pages
        .iter()
        .find(|(p, _)| p == current)
        .map_or("", |(_, n)| *n);
    ui.scope_builder(
        egui::UiBuilder::new().max_rect(right.shrink2(vec2(10.0, 6.0))),
        |ui| {
            ui.label(RichText::new(title).size(16.0).strong());
            ui.add_space(4.0);
            egui::ScrollArea::vertical()
                .id_salt((salt, "page"))
                .auto_shrink([false, false])
                .show(ui, |ui| body(ui, *current));
        },
    );
    ui.allocate_rect(rect, egui::Sense::hover());
}

pub fn ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    let now = oxide_core::time::realtime_since_startup();
    let w = &mut engine.world;
    let mut s = ProjectSettings::capture(w);
    let before = s.clone();
    let project = w.resource::<oxide_assets::AssetServer>().full_path("");
    let ss = &mut st.settings;
    let build = ss
        .build
        .get_or_insert_with(|| BuildSettings::load(&project));
    let build_before = build.clone();
    let layer_names: Vec<(usize, String)> = s
        .tags_and_layers
        .layers
        .iter()
        .enumerate()
        .filter(|(_, n)| !n.is_empty())
        .map(|(i, n)| (i, n.clone()))
        .collect();
    let mut page = ss.page;
    two_pane(ui, "project-settings", &PAGES, &mut page, |ui, p| match p {
        Page::TagsAndLayers => tags_and_layers(ui, &mut s, &mut ss.new_tag),
        Page::Time => time(ui, &mut s),
        Page::Physics => physics(ui, &mut s, &layer_names),
        Page::Quality => quality(ui, w, &mut s, &mut ss.quality_selected),
        Page::Input => input(ui, &mut s),
        Page::Player => player(ui, &mut s, build),
    });
    ss.page = page;
    if *build != build_before
        && let Err(e) = build.save(&project)
    {
        st.status = format!("cannot save Build Settings: {e}");
    }
    if s != before {
        s.apply(w);
        ss.save_at = Some(now + SAVE_DELAY);
    }
    flush(st, engine, now);
}

/// Writes pending changes once edits have paused.
pub fn flush(st: &mut EditorState, engine: &Engine, now: f64) {
    if st.settings.save_at.is_some_and(|t| now >= t) {
        st.settings.save_at = None;
        st.status = match project_settings::save(&engine.world) {
            Ok(()) => "Saved Project Settings".into(),
            Err(e) => format!("cannot save Project Settings: {e}"),
        };
    }
}

fn grid(ui: &mut egui::Ui, salt: &str, f: impl FnOnce(&mut egui::Ui)) {
    egui::Grid::new(salt)
        .num_columns(2)
        .spacing([16.0, 6.0])
        .min_col_width(150.0)
        .show(ui, f);
}

fn tags_and_layers(ui: &mut egui::Ui, s: &mut ProjectSettings, new_tag: &mut String) {
    let tl = &mut s.tags_and_layers;
    egui::CollapsingHeader::new("Tags")
        .default_open(true)
        .show(ui, |ui| {
            for t in BUILTIN_TAGS {
                ui.add_enabled(false, egui::Label::new(t));
            }
            let mut remove = None;
            for (i, t) in tl.tags.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(t);
                    if ui.small_button("−").on_hover_text("Remove tag").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                tl.tags.remove(i);
            }
            ui.horizontal(|ui| {
                let r = ui.add(egui::TextEdit::singleline(new_tag).hint_text("New Tag Name"));
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let name = new_tag.trim().to_owned();
                let valid = !name.is_empty()
                    && !BUILTIN_TAGS.contains(&name.as_str())
                    && !tl.tags.contains(&name);
                if (ui.add_enabled(valid, egui::Button::new("+")).clicked() || enter) && valid {
                    tl.tags.push(name);
                    new_tag.clear();
                }
            });
        });
    egui::CollapsingHeader::new("Layers")
        .default_open(true)
        .show(ui, |ui| {
            tl.layers.resize(32, String::new());
            grid(ui, "layers", |ui| {
                for (i, name) in tl.layers.iter_mut().enumerate() {
                    let builtin = BUILTIN_LAYERS.contains(&i);
                    ui.label(format!(
                        "{} Layer {i}",
                        if builtin { "Builtin" } else { "User" }
                    ));
                    ui.add_enabled(!builtin, egui::TextEdit::singleline(name));
                    ui.end_row();
                }
            });
        });
}

fn time(ui: &mut egui::Ui, s: &mut ProjectSettings) {
    let t = &mut s.time;
    grid(ui, "time", |ui| {
        ui.label("Fixed Timestep")
            .on_hover_text("Seconds between FixedUpdate and physics steps");
        ui.add(
            egui::DragValue::new(&mut t.fixed_timestep)
                .range(0.0001..=1.0)
                .speed(0.001)
                .max_decimals(5),
        );
        ui.end_row();
        ui.label("Maximum Allowed Timestep")
            .on_hover_text("Longest frame time the game sees (caps catch-up after a hitch)");
        ui.add(
            egui::DragValue::new(&mut t.maximum_allowed_timestep)
                .range(0.0001..=10.0)
                .speed(0.001)
                .max_decimals(5),
        );
        ui.end_row();
        ui.label("Time Scale");
        ui.add(egui::Slider::new(&mut t.time_scale, 0.0..=4.0));
        ui.end_row();
    });
}

fn physics(ui: &mut egui::Ui, s: &mut ProjectSettings, layers: &[(usize, String)]) {
    let p = &mut s.physics;
    grid(ui, "physics", |ui| {
        ui.label("Gravity");
        ui.horizontal(|ui| {
            for (axis, v) in ["X", "Y", "Z"].iter().zip(p.gravity.iter_mut()) {
                ui.label(*axis);
                ui.add(egui::DragValue::new(v).speed(0.05));
            }
        });
        ui.end_row();
        ui.label("Queries Hit Triggers");
        ui.checkbox(&mut p.queries_hit_triggers, "");
        ui.end_row();
    });
    ui.add_space(8.0);
    ui.label(RichText::new("Layer Collision Matrix").strong());
    // Unity's triangle: rows are layers top-down, columns the same layers right-to-left,
    // so each pair appears once.
    let cell = 18.0;
    let label_w = layers
        .iter()
        .map(|(_, n)| n.len() as f32 * 6.5)
        .fold(40.0, f32::max)
        + 8.0;
    let n = layers.len();
    let size = vec2(label_w + n as f32 * cell + 4.0, label_w + n as f32 * cell);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let font = egui::FontId::proportional(11.5);
    let text_color = ui.visuals().text_color();
    for (col, (_, name)) in layers.iter().rev().enumerate() {
        let x = rect.left() + label_w + col as f32 * cell + cell * 0.5;
        let galley = painter.layout_no_wrap(name.clone(), font.clone(), text_color);
        let pos = egui::pos2(x - galley.size().y * 0.5, rect.top() + label_w - 4.0);
        painter.add(
            egui::epaint::TextShape::new(pos, galley, text_color)
                .with_angle(-std::f32::consts::FRAC_PI_2),
        );
    }
    for (row, (a, name)) in layers.iter().enumerate() {
        let y = rect.top() + label_w + row as f32 * cell;
        painter.text(
            egui::pos2(rect.left() + label_w - 6.0, y + cell * 0.5),
            egui::Align2::RIGHT_CENTER,
            name,
            font.clone(),
            text_color,
        );
        for (col, (b, _)) in layers.iter().rev().enumerate() {
            if col >= n - row {
                break;
            }
            let r = egui::Rect::from_min_size(
                egui::pos2(rect.left() + label_w + col as f32 * cell, y),
                vec2(cell, cell),
            );
            let mut on = p.collides(*a, *b);
            let resp = ui
                .put(r, egui::Checkbox::without_text(&mut on))
                .on_hover_text(format!(
                    "{name} / {}",
                    layers
                        .iter()
                        .find(|l| l.0 == *b)
                        .map_or("", |l| l.1.as_str())
                ));
            if resp.changed() {
                p.set_collides(*a, *b, on);
            }
        }
    }
}

fn quality(
    ui: &mut egui::Ui,
    w: &oxide_scene::World,
    s: &mut ProjectSettings,
    selected: &mut Option<usize>,
) {
    let q = &mut s.quality;
    let sel = selected
        .unwrap_or(q.current)
        .min(q.levels.len().saturating_sub(1));
    *selected = Some(sel);
    ui.label("Levels (the radio button is the active level)");
    let mut remove = None;
    egui::Frame::new()
        .fill(Color32::from_gray(42))
        .inner_margin(4)
        .show(ui, |ui| {
            for i in 0..q.levels.len() {
                ui.horizontal(|ui| {
                    if ui
                        .radio(q.current == i, "")
                        .on_hover_text("Make active")
                        .clicked()
                    {
                        q.current = i;
                    }
                    if ui.selectable_label(sel == i, &q.levels[i].name).clicked() {
                        *selected = Some(i);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if q.levels.len() > 1
                            && ui.small_button("🗑").on_hover_text("Delete level").clicked()
                        {
                            remove = Some(i);
                        }
                    });
                });
            }
        });
    if let Some(i) = remove {
        q.levels.remove(i);
        if q.current >= i && q.current > 0 {
            q.current -= 1;
        }
        *selected = Some(q.current);
    }
    if ui.button("Add Quality Level").clicked() {
        let mut l = q.levels.get(sel).cloned().unwrap_or_default();
        l.name = format!("Level {}", q.levels.len());
        q.levels.push(l);
        *selected = Some(q.levels.len() - 1);
    }
    let sel = selected.unwrap_or(0).min(q.levels.len() - 1);
    ui.add_space(8.0);
    let level: &mut QualityLevel = &mut q.levels[sel];
    ui.label(RichText::new(format!("{} settings", level.name)).strong());
    crate::inspector::reflect_grid(ui, w, level, "quality-level");
}

fn key_list(ui: &mut egui::Ui, keys: &mut Vec<KeyCode>, salt: &str) {
    ui.horizontal_wrapped(|ui| {
        let mut remove = None;
        for (i, k) in keys.iter().enumerate() {
            if ui
                .small_button(format!("{k:?} ✕"))
                .on_hover_text("Remove")
                .clicked()
            {
                remove = Some(i);
            }
        }
        if let Some(i) = remove {
            keys.remove(i);
        }
        egui::ComboBox::from_id_salt(salt)
            .selected_text("+")
            .width(40.0)
            .height(300.0)
            .show_ui(ui, |ui| {
                for k in KeyCode::ALL {
                    if !keys.contains(k) && ui.selectable_label(false, format!("{k:?}")).clicked() {
                        keys.push(*k);
                    }
                }
            });
    });
}

fn enum_list<T: Copy + PartialEq + std::fmt::Debug>(
    ui: &mut egui::Ui,
    items: &mut Vec<T>,
    all: &[T],
    salt: &str,
) {
    ui.horizontal_wrapped(|ui| {
        let mut remove = None;
        for (i, k) in items.iter().enumerate() {
            if ui.small_button(format!("{k:?} ✕")).clicked() {
                remove = Some(i);
            }
        }
        if let Some(i) = remove {
            items.remove(i);
        }
        egui::ComboBox::from_id_salt(salt)
            .selected_text("+")
            .width(40.0)
            .show_ui(ui, |ui| {
                for k in all {
                    if !items.contains(k) && ui.selectable_label(false, format!("{k:?}")).clicked()
                    {
                        items.push(*k);
                    }
                }
            });
    });
}

fn pick<T: Copy + PartialEq + std::fmt::Debug>(
    ui: &mut egui::Ui,
    v: &mut T,
    all: &[T],
    salt: impl std::hash::Hash + std::fmt::Debug,
) {
    egui::ComboBox::from_id_salt(egui::Id::new(salt))
        .selected_text(format!("{v:?}"))
        .show_ui(ui, |ui| {
            for k in all {
                ui.selectable_value(v, *k, format!("{k:?}"));
            }
        });
}

const PAD_AXES: [GamepadAxis; 6] = [
    GamepadAxis::LeftStickX,
    GamepadAxis::LeftStickY,
    GamepadAxis::RightStickX,
    GamepadAxis::RightStickY,
    GamepadAxis::LeftTrigger,
    GamepadAxis::RightTrigger,
];
const PAD_BUTTONS: [GamepadButton; 16] = [
    GamepadButton::South,
    GamepadButton::East,
    GamepadButton::West,
    GamepadButton::North,
    GamepadButton::LeftShoulder,
    GamepadButton::RightShoulder,
    GamepadButton::LeftTrigger,
    GamepadButton::RightTrigger,
    GamepadButton::Select,
    GamepadButton::Start,
    GamepadButton::LeftStick,
    GamepadButton::RightStick,
    GamepadButton::DpadUp,
    GamepadButton::DpadDown,
    GamepadButton::DpadLeft,
    GamepadButton::DpadRight,
];
const MOUSE: [MouseButton; 3] = [MouseButton::Left, MouseButton::Right, MouseButton::Middle];

fn source_kind(s: &AxisSource) -> &'static str {
    match s {
        AxisSource::Keys { .. } => "Key or Mouse Button",
        AxisSource::MouseX => "Mouse X",
        AxisSource::MouseY => "Mouse Y",
        AxisSource::ScrollWheel => "Scroll Wheel",
        AxisSource::Gamepad(_) => "Joystick Axis",
        AxisSource::GamepadButtons { .. } => "Joystick Buttons",
    }
}

fn source_ui(ui: &mut egui::Ui, src: &mut AxisSource, salt: &str) {
    let mut kind = source_kind(src);
    egui::ComboBox::from_id_salt((salt, "kind"))
        .selected_text(kind)
        .show_ui(ui, |ui| {
            for k in [
                "Key or Mouse Button",
                "Mouse X",
                "Mouse Y",
                "Scroll Wheel",
                "Joystick Axis",
                "Joystick Buttons",
            ] {
                ui.selectable_value(&mut kind, k, k);
            }
        });
    if kind != source_kind(src) {
        *src = match kind {
            "Mouse X" => AxisSource::MouseX,
            "Mouse Y" => AxisSource::MouseY,
            "Scroll Wheel" => AxisSource::ScrollWheel,
            "Joystick Axis" => AxisSource::Gamepad(GamepadAxis::LeftStickX),
            "Joystick Buttons" => AxisSource::GamepadButtons {
                negative: GamepadButton::DpadLeft,
                positive: GamepadButton::DpadRight,
            },
            _ => AxisSource::Keys {
                negative: Vec::new(),
                positive: Vec::new(),
            },
        };
    }
    match src {
        AxisSource::Keys { negative, positive } => {
            ui.horizontal(|ui| {
                ui.label("Negative");
                key_list(ui, negative, &format!("{salt}-neg"));
            });
            ui.horizontal(|ui| {
                ui.label("Positive");
                key_list(ui, positive, &format!("{salt}-pos"));
            });
        }
        AxisSource::Gamepad(a) => pick(ui, a, &PAD_AXES, (salt, "axis")),
        AxisSource::GamepadButtons { negative, positive } => {
            ui.horizontal(|ui| {
                ui.label("Negative");
                pick(ui, negative, &PAD_BUTTONS, (salt, "nb"));
                ui.label("Positive");
                pick(ui, positive, &PAD_BUTTONS, (salt, "pb"));
            });
        }
        _ => {}
    }
}

fn axis_ui(ui: &mut egui::Ui, a: &mut InputAxis, i: usize) {
    grid(ui, &format!("axis-{i}"), |ui| {
        ui.label("Name");
        ui.text_edit_singleline(&mut a.name);
        ui.end_row();
        ui.label("Gravity")
            .on_hover_text("Units per second the value falls back to 0");
        ui.add(egui::DragValue::new(&mut a.gravity).speed(0.05));
        ui.end_row();
        ui.label("Dead");
        ui.add(
            egui::DragValue::new(&mut a.dead)
                .speed(0.005)
                .range(0.0..=1.0),
        );
        ui.end_row();
        ui.label("Sensitivity");
        ui.add(egui::DragValue::new(&mut a.sensitivity).speed(0.05));
        ui.end_row();
        ui.label("Snap");
        ui.checkbox(&mut a.snap, "");
        ui.end_row();
        ui.label("Invert");
        ui.checkbox(&mut a.invert, "");
        ui.end_row();
    });
    ui.label("Sources");
    let mut remove = None;
    for (j, src) in a.sources.iter_mut().enumerate() {
        egui::Frame::new()
            .fill(Color32::from_gray(46))
            .inner_margin(4)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .small_button("✕")
                        .on_hover_text("Remove source")
                        .clicked()
                    {
                        remove = Some(j);
                    }
                    ui.vertical(|ui| source_ui(ui, src, &format!("axis{i}-src{j}")));
                });
            });
    }
    if let Some(j) = remove {
        a.sources.remove(j);
    }
    if ui.small_button("Add Source").clicked() {
        a.sources.push(AxisSource::Keys {
            negative: Vec::new(),
            positive: Vec::new(),
        });
    }
}

fn button_ui(ui: &mut egui::Ui, b: &mut InputButton, i: usize) {
    grid(ui, &format!("button-{i}"), |ui| {
        ui.label("Name");
        ui.text_edit_singleline(&mut b.name);
        ui.end_row();
        ui.label("Keys");
        key_list(ui, &mut b.keys, &format!("btn{i}-keys"));
        ui.end_row();
        ui.label("Mouse Buttons");
        enum_list(ui, &mut b.mouse, &MOUSE, &format!("btn{i}-mouse"));
        ui.end_row();
        ui.label("Joystick Buttons");
        enum_list(ui, &mut b.pad, &PAD_BUTTONS, &format!("btn{i}-pad"));
        ui.end_row();
    });
}

fn input(ui: &mut egui::Ui, s: &mut ProjectSettings) {
    let inp = &mut s.input;
    ui.weak("Read with Input::get_axis / get_button by name.");
    ui.label(RichText::new(format!("Axes ({})", inp.axes.len())).strong());
    let mut remove = None;
    for (i, a) in inp.axes.iter_mut().enumerate() {
        let r = egui::CollapsingHeader::new(&a.name)
            .id_salt(("axis", i))
            .show(ui, |ui| axis_ui(ui, a, i));
        r.header_response.context_menu(|ui| {
            if ui.button("Delete Axis").clicked() {
                remove = Some(i);
                ui.close();
            }
        });
    }
    if let Some(i) = remove {
        inp.axes.remove(i);
    }
    if ui.button("Add Axis").clicked() {
        inp.axes
            .push(InputAxis::new(&format!("Axis {}", inp.axes.len())));
    }
    ui.add_space(8.0);
    ui.label(RichText::new(format!("Buttons ({})", inp.buttons.len())).strong());
    let mut remove = None;
    for (i, b) in inp.buttons.iter_mut().enumerate() {
        let r = egui::CollapsingHeader::new(&b.name)
            .id_salt(("button", i))
            .show(ui, |ui| button_ui(ui, b, i));
        r.header_response.context_menu(|ui| {
            if ui.button("Delete Button").clicked() {
                remove = Some(i);
                ui.close();
            }
        });
    }
    if let Some(i) = remove {
        inp.buttons.remove(i);
    }
    if ui.button("Add Button").clicked() {
        inp.buttons.push(InputButton {
            name: format!("Button {}", inp.buttons.len()),
            ..Default::default()
        });
    }
    ui.add_space(4.0);
    if ui.button("Reset to Defaults").clicked() {
        *inp = Default::default();
    }
}

fn player(ui: &mut egui::Ui, s: &mut ProjectSettings, build: &mut BuildSettings) {
    let p = &mut s.player;
    grid(ui, "player", |ui| {
        ui.label("Company Name");
        ui.text_edit_singleline(&mut p.company_name);
        ui.end_row();
        ui.label("Product Name");
        let mut name = if p.product_name.is_empty() {
            build.product_name.clone()
        } else {
            p.product_name.clone()
        };
        if ui.text_edit_singleline(&mut name).changed() {
            p.product_name = name.clone();
            build.product_name = name;
        }
        ui.end_row();
        ui.label("Version");
        ui.text_edit_singleline(&mut p.version);
        ui.end_row();
    });
    ui.add_space(8.0);
    ui.label(RichText::new("Resolution and Presentation").strong());
    grid(ui, "player-res", |ui| {
        ui.label("Fullscreen Mode");
        egui::ComboBox::from_id_salt("fullscreen")
            .selected_text(if build.fullscreen {
                "Fullscreen Window"
            } else {
                "Windowed"
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut build.fullscreen, true, "Fullscreen Window");
                ui.selectable_value(&mut build.fullscreen, false, "Windowed");
            });
        ui.end_row();
        ui.label("Default Screen Width");
        ui.add(egui::DragValue::new(&mut build.width).range(320..=7680));
        ui.end_row();
        ui.label("Default Screen Height");
        ui.add(egui::DragValue::new(&mut build.height).range(240..=4320));
        ui.end_row();
    });
}
