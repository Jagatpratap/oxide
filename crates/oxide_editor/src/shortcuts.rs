//! Keyboard shortcuts and the Shortcuts Manager (Unity Edit → Shortcuts...).
//!
//! Every shortcut is a [`Command`] with default bindings. Rebinding is stored per user in
//! Preferences (only the changed commands), so menus, toolbar tooltips and dispatch all
//! read the same [`Keymap`]. The window lists commands by category with search, shows a
//! keyboard with the bound keys for the held modifiers, and catches binding conflicts.
//! Reference: <https://docs.unity3d.com/Manual/ShortcutsManager.html>

use crate::{Action, Create, EditorState, PlayState, Tool};
use egui::{Color32, Event, Key, Modifiers, RichText};
use oxide_engine::Engine;
use oxide_input::{CursorLockMode, Input};
use std::collections::BTreeMap;

/// A key with modifiers. `command` is ⌘ on macOS and Ctrl elsewhere (Unity's "Action").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Binding {
    pub command: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: Key,
}

const fn b(command: bool, shift: bool, alt: bool, key: Key) -> Binding {
    Binding {
        command,
        shift,
        alt,
        key,
    }
}

const fn plain(key: Key) -> Binding {
    b(false, false, false, key)
}

const fn cmd(key: Key) -> Binding {
    b(true, false, false, key)
}

const fn cmd_shift(key: Key) -> Binding {
    b(true, true, false, key)
}

const fn cmd_alt(key: Key) -> Binding {
    b(true, false, true, key)
}

impl Binding {
    /// From a key event, or `None` for combinations bindings can't express (Ctrl on macOS).
    pub fn from_event(key: Key, m: Modifiers) -> Option<Self> {
        if cfg!(target_os = "macos") && m.ctrl {
            return None;
        }
        Some(b(m.command, m.shift, m.alt, key))
    }

    /// Settings text: `Cmd+Shift+Z` (`Cmd` is ⌘ on macOS, Ctrl elsewhere).
    pub fn to_config(self) -> String {
        let mut s = String::new();
        for (on, name) in [
            (self.command, "Cmd+"),
            (self.alt, "Alt+"),
            (self.shift, "Shift+"),
        ] {
            if on {
                s.push_str(name);
            }
        }
        s + self.key.name()
    }

    pub fn parse(text: &str) -> Option<Self> {
        let mut out = plain(Key::A);
        let mut key = None;
        for part in text.split('+').map(str::trim) {
            match part.to_ascii_lowercase().as_str() {
                "cmd" | "command" | "ctrl" | "control" | "action" => out.command = true,
                "shift" => out.shift = true,
                "alt" | "option" | "opt" => out.alt = true,
                "" => {}
                _ => key = Some(Key::from_name(part)?),
            }
        }
        out.key = key?;
        Some(out)
    }

    /// How menus show it: `⇧⌘Z` on macOS, `Ctrl+Shift+Z` elsewhere.
    pub fn label(self) -> String {
        let mac = cfg!(target_os = "macos");
        let key = match self.key {
            // The editor font has no ⌫ or ⌥ glyphs.
            Key::Backspace => "Backspace",
            Key::Delete => "Del",
            Key::Comma => ",",
            Key::OpenBracket => "[",
            Key::CloseBracket => "]",
            k => k.symbol_or_name(),
        };
        if mac {
            let mut s = String::new();
            if self.alt {
                s.push_str("Opt+");
            }
            if self.shift {
                s.push('⇧');
            }
            if self.command {
                s.push('⌘');
            }
            s + key
        } else {
            let mut s = String::new();
            for (on, name) in [
                (self.command, "Ctrl+"),
                (self.alt, "Alt+"),
                (self.shift, "Shift+"),
            ] {
                if on {
                    s.push_str(name);
                }
            }
            s + key
        }
    }
}

/// What a command does.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    Action(Action),
    /// Scene view Pivot / Center (Z).
    TogglePivot,
    /// Scene view Global / Local (X).
    ToggleLocal,
}

#[derive(Clone, Debug)]
pub struct Command {
    /// `Main Menu/Edit/Undo`, `Tools/Move`... The first segment is the category.
    pub id: String,
    pub defaults: Vec<Binding>,
    pub effect: Effect,
    /// Works while the Game view has the keyboard in Play mode (Play, Pause, Step).
    pub always: bool,
    /// Fires again while the key is held (Undo, Redo).
    pub repeat: bool,
    /// Off while flying the Scene view camera (WASD/QE are camera keys then).
    pub scene_key: bool,
}

fn command(id: &str, defaults: &[Binding], action: Action) -> Command {
    Command {
        id: id.to_owned(),
        defaults: defaults.to_vec(),
        effect: Effect::Action(action),
        always: false,
        repeat: false,
        scene_key: false,
    }
}

/// Every command that can have a shortcut, with Unity's default bindings.
pub fn commands() -> Vec<Command> {
    let mac = cfg!(target_os = "macos");
    let mut v = vec![
        command("Main Menu/File/New Scene", &[cmd(Key::N)], Action::NewScene),
        command(
            "Main Menu/File/Open Scene",
            &[cmd(Key::O)],
            Action::OpenSceneDialog,
        ),
        command("Main Menu/File/Save", &[cmd(Key::S)], Action::SaveScene),
        command(
            "Main Menu/File/Save As",
            &[cmd_shift(Key::S)],
            Action::SaveSceneAsDialog,
        ),
        command(
            "Main Menu/File/Build Settings",
            &[cmd_shift(Key::B)],
            Action::BuildSettingsDialog,
        ),
        command(
            "Main Menu/File/Build And Run",
            &[cmd(Key::B)],
            Action::BuildPlayer(true),
        ),
        command("Main Menu/Edit/Undo", &[cmd(Key::Z)], Action::Undo),
        command(
            "Main Menu/Edit/Redo",
            &[cmd_shift(Key::Z), cmd(Key::Y)],
            Action::Redo,
        ),
        command(
            "Main Menu/Edit/Select All",
            &[cmd(Key::A)],
            Action::SelectAll,
        ),
        command(
            "Main Menu/Edit/Duplicate",
            &[cmd(Key::D)],
            Action::Duplicate,
        ),
        command(
            "Main Menu/Edit/Delete",
            &if mac {
                vec![
                    cmd(Key::Backspace),
                    plain(Key::Delete),
                    plain(Key::Backspace),
                ]
            } else {
                vec![plain(Key::Delete)]
            },
            Action::Delete,
        ),
        command("Main Menu/Edit/Rename", &[plain(Key::F2)], Action::Rename),
        command(
            "Main Menu/Edit/Frame Selected",
            &[plain(Key::F)],
            Action::FrameSelected,
        ),
        command(
            "Main Menu/Edit/Selection/Back",
            &[cmd(Key::OpenBracket)],
            Action::SelectionBack,
        ),
        command(
            "Main Menu/Edit/Selection/Forward",
            &[cmd(Key::CloseBracket)],
            Action::SelectionForward,
        ),
        command("Main Menu/Edit/Play", &[cmd(Key::P)], Action::TogglePlay),
        command(
            "Main Menu/Edit/Pause",
            &[cmd_shift(Key::P)],
            Action::TogglePause,
        ),
        command("Main Menu/Edit/Step", &[cmd_alt(Key::P)], Action::Step),
        command(
            "Main Menu/Edit/Preferences",
            &if mac { vec![cmd(Key::Comma)] } else { vec![] },
            Action::OpenPreferences,
        ),
        command(
            "Main Menu/Edit/Shortcuts",
            &[],
            Action::OpenTab(crate::Tab::Shortcuts),
        ),
        command(
            "Main Menu/Assets/Refresh",
            &[cmd(Key::R)],
            Action::RefreshAssets,
        ),
        command(
            "Main Menu/GameObject/Create Empty",
            &[cmd_shift(Key::N)],
            Action::Create(Create::Empty),
        ),
        command(
            "Main Menu/GameObject/Create Empty Child",
            &[b(false, true, true, Key::N)],
            Action::Create(Create::EmptyChild),
        ),
    ];
    let nums = [
        Key::Num1,
        Key::Num2,
        Key::Num3,
        Key::Num4,
        Key::Num5,
        Key::Num6,
        Key::Num7,
        Key::Num8,
        Key::Num9,
    ];
    for (i, k) in nums.into_iter().enumerate() {
        v.push(command(
            &format!("Main Menu/Edit/Selection/Save Selection {}", i + 1),
            &[cmd_alt(k)],
            Action::SaveSelection(i + 1),
        ));
        v.push(command(
            &format!("Main Menu/Edit/Selection/Load Selection {}", i + 1),
            &[cmd_shift(k)],
            Action::LoadSelection(i + 1),
        ));
    }
    for (name, key, tool) in [
        ("View", Key::Q, Tool::Hand),
        ("Move", Key::W, Tool::Move),
        ("Rotate", Key::E, Tool::Rotate),
        ("Scale", Key::R, Tool::Scale),
        ("Rect", Key::T, Tool::Rect),
        ("Transform", Key::Y, Tool::Transform),
    ] {
        let mut c = command(
            &format!("Tools/{name}"),
            &[plain(key)],
            Action::SetTool(tool),
        );
        c.scene_key = true;
        v.push(c);
    }
    for (id, key, effect) in [
        ("Tools/Toggle Pivot Position", Key::Z, Effect::TogglePivot),
        (
            "Tools/Toggle Pivot Orientation",
            Key::X,
            Effect::ToggleLocal,
        ),
    ] {
        v.push(Command {
            id: id.into(),
            defaults: vec![plain(key)],
            effect,
            always: false,
            repeat: false,
            scene_key: true,
        });
    }
    for c in &mut v {
        if matches!(
            c.effect,
            Effect::Action(Action::TogglePlay | Action::TogglePause | Action::Step)
        ) {
            c.always = true;
        }
        if matches!(
            c.effect,
            Effect::Action(
                Action::Undo | Action::Redo | Action::SelectionBack | Action::SelectionForward
            )
        ) {
            c.repeat = true;
        }
    }
    v
}

/// The commands and their current bindings (defaults plus the user's changes).
#[derive(Clone, Debug)]
pub struct Keymap {
    pub commands: Vec<Command>,
    pub bindings: Vec<Vec<Binding>>,
}

impl Default for Keymap {
    fn default() -> Self {
        Self::new(&BTreeMap::new())
    }
}

impl Keymap {
    /// Defaults with `overrides` (command id → bindings, as saved in Preferences) applied.
    pub fn new(overrides: &BTreeMap<String, Vec<String>>) -> Self {
        let commands = commands();
        let bindings = commands
            .iter()
            .map(|c| match overrides.get(&c.id) {
                Some(list) => list.iter().filter_map(|s| Binding::parse(s)).collect(),
                None => c.defaults.clone(),
            })
            .collect();
        Self { commands, bindings }
    }

    /// What Preferences stores: only commands whose bindings differ from the defaults.
    pub fn overrides(&self) -> BTreeMap<String, Vec<String>> {
        self.commands
            .iter()
            .zip(&self.bindings)
            .filter(|(c, b)| c.defaults != **b)
            .map(|(c, b)| (c.id.clone(), b.iter().map(|x| x.to_config()).collect()))
            .collect()
    }

    pub fn find(&self, id: &str) -> Option<usize> {
        self.commands.iter().position(|c| c.id == id)
    }

    /// The command bound to `binding` (first one, if there's a conflict).
    pub fn command_for(&self, binding: Binding) -> Option<usize> {
        self.bindings.iter().position(|b| b.contains(&binding))
    }

    /// Other commands using one of command `i`'s bindings.
    pub fn conflicts(&self, i: usize) -> Vec<usize> {
        (0..self.commands.len())
            .filter(|&j| {
                j != i
                    && self.bindings[j]
                        .iter()
                        .any(|b| self.bindings[i].contains(b))
            })
            .collect()
    }

    /// Menu text for the command that runs `effect` (its first binding).
    pub fn text_for(&self, effect: &Effect) -> String {
        self.commands
            .iter()
            .zip(&self.bindings)
            .find(|(c, _)| c.effect == *effect)
            .and_then(|(_, b)| b.first())
            .map(|b| b.label())
            .unwrap_or_default()
    }

    pub fn action_text(&self, action: &Action) -> String {
        self.text_for(&Effect::Action(action.clone()))
    }

    /// Binds `binding` to command `i` (added to its bindings, or replacing them all when
    /// `replace`), removing it from `steal_from` commands (Unity's "Reassign").
    pub fn bind(&mut self, i: usize, binding: Binding, replace: bool, steal: bool) {
        if steal {
            for (j, list) in self.bindings.iter_mut().enumerate() {
                if j != i {
                    list.retain(|b| *b != binding);
                }
            }
        }
        let list = &mut self.bindings[i];
        if replace {
            list.clear();
        }
        if !list.contains(&binding) {
            list.push(binding);
        }
    }
}

/// Runs the shortcuts pressed this frame. Editor shortcuts are off while typing in a field
/// or while the Game view has the keyboard in Play mode (Esc gives it back); Play, Pause
/// and Step always work.
pub fn dispatch(st: &mut EditorState, engine: &mut Engine, ctx: &egui::Context) {
    if st.shortcuts_ui.capturing.is_some() {
        return;
    }
    let game_keys = st.play == PlayState::Playing && st.game_focused;
    if game_keys && ctx.input(|i| i.key_pressed(Key::Escape)) {
        st.game_focused = false;
        let input = engine.world.resource_mut::<Input>();
        input.cursor_lock = CursorLockMode::None;
        input.cursor_visible = true;
        input.release_all();
        return;
    }
    let typing = ctx.egui_wants_keyboard_input();
    let fired =
        ctx.input(|i| triggered(&st.keymap, &i.events, game_keys, typing, st.scene_ui.flying));
    for (binding, i) in fired {
        // Take the key so widgets don't also react to it.
        ctx.input_mut(|inp| {
            inp.events.retain(|e| {
                !matches!(e, Event::Key { key, pressed: true, modifiers, .. }
                    if Binding::from_event(*key, *modifiers) == Some(binding))
            })
        });
        match st.keymap.commands[i].effect.clone() {
            Effect::Action(a) => st.actions.push(a),
            Effect::TogglePivot => st.center_handles = !st.center_handles,
            Effect::ToggleLocal => st.local_handles = !st.local_handles,
        }
    }
}

/// The commands `events` trigger: (binding, command index). `game_keys`: the Game view has
/// the keyboard in Play mode; `typing`: a text field has focus; `flying`: the Scene view
/// camera is flying (its WASD/QE keys win).
pub fn triggered(
    km: &Keymap,
    events: &[Event],
    game_keys: bool,
    typing: bool,
    flying: bool,
) -> Vec<(Binding, usize)> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Key {
                key,
                pressed: true,
                repeat,
                modifiers,
                ..
            } => Binding::from_event(*key, *modifiers).map(|b| (b, *repeat)),
            _ => None,
        })
        .filter_map(|(binding, repeat)| {
            let i = km.command_for(binding)?;
            let c = &km.commands[i];
            let blocked = (repeat && !c.repeat)
                || (!c.always && (game_keys || typing))
                || (c.scene_key && flying);
            (!blocked).then_some((binding, i))
        })
        .collect()
}

// ---- Shortcuts Manager window ------------------------------------------------------------

#[derive(Default)]
pub struct ShortcutsWindow {
    category: String,
    search: String,
    /// Waiting for a key for this command (replace = rebind, else add another).
    pub capturing: Option<(usize, bool)>,
    /// A key that is already used: (command, binding, replace, other commands).
    conflict: Option<(usize, Binding, bool, Vec<usize>)>,
    /// Keyboard view: modifiers to show and the key picked to filter the list.
    show_cmd: bool,
    show_shift: bool,
    show_alt: bool,
    key_filter: Option<Key>,
}

const KEYBOARD: [&[(Key, f32)]; 5] = [
    &[
        (Key::Escape, 1.0),
        (Key::F1, 1.0),
        (Key::F2, 1.0),
        (Key::F3, 1.0),
        (Key::F4, 1.0),
        (Key::F5, 1.0),
        (Key::F6, 1.0),
        (Key::F7, 1.0),
        (Key::F8, 1.0),
        (Key::F9, 1.0),
        (Key::F10, 1.0),
        (Key::F11, 1.0),
        (Key::F12, 1.0),
    ],
    &[
        (Key::Backtick, 1.0),
        (Key::Num1, 1.0),
        (Key::Num2, 1.0),
        (Key::Num3, 1.0),
        (Key::Num4, 1.0),
        (Key::Num5, 1.0),
        (Key::Num6, 1.0),
        (Key::Num7, 1.0),
        (Key::Num8, 1.0),
        (Key::Num9, 1.0),
        (Key::Num0, 1.0),
        (Key::Minus, 1.0),
        (Key::Equals, 1.0),
        (Key::Backspace, 2.0),
    ],
    &[
        (Key::Tab, 1.5),
        (Key::Q, 1.0),
        (Key::W, 1.0),
        (Key::E, 1.0),
        (Key::R, 1.0),
        (Key::T, 1.0),
        (Key::Y, 1.0),
        (Key::U, 1.0),
        (Key::I, 1.0),
        (Key::O, 1.0),
        (Key::P, 1.0),
        (Key::OpenBracket, 1.0),
        (Key::CloseBracket, 1.0),
        (Key::Backslash, 1.5),
    ],
    &[
        (Key::A, 1.0),
        (Key::S, 1.0),
        (Key::D, 1.0),
        (Key::F, 1.0),
        (Key::G, 1.0),
        (Key::H, 1.0),
        (Key::J, 1.0),
        (Key::K, 1.0),
        (Key::L, 1.0),
        (Key::Semicolon, 1.0),
        (Key::Quote, 1.0),
        (Key::Enter, 2.25),
    ],
    &[
        (Key::Z, 1.0),
        (Key::X, 1.0),
        (Key::C, 1.0),
        (Key::V, 1.0),
        (Key::B, 1.0),
        (Key::N, 1.0),
        (Key::M, 1.0),
        (Key::Comma, 1.0),
        (Key::Period, 1.0),
        (Key::Slash, 1.0),
        (Key::Space, 2.5),
        (Key::Delete, 1.25),
    ],
];

/// Row indents of the keyboard picture (in key widths), like a real keyboard's stagger.
const KEYBOARD_INDENT: [f32; 5] = [0.0, 0.0, 0.0, 1.75, 2.25];

fn category_of(id: &str) -> &str {
    id.split('/').next().unwrap_or(id)
}

/// The command's name without its category (`Edit/Undo`).
fn short_name(id: &str) -> &str {
    id.split_once('/').map_or(id, |(_, r)| r)
}

pub fn window_ui(st: &mut EditorState, ui: &mut egui::Ui) {
    let mut changed = false;
    capture_keys(st, ui, &mut changed);
    let w = &mut st.shortcuts_ui;
    let km = &mut st.keymap;
    if w.category.is_empty() {
        w.category = "All Commands".into();
    }

    ui.horizontal(|ui| {
        ui.label("Profile:");
        ui.strong("Default");
        ui.weak("(your changes are saved in Preferences)");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Reset All to Default").clicked() {
                *km = Keymap::default();
                changed = true;
            }
        });
    });
    ui.separator();
    keyboard(ui, w, km);
    ui.separator();

    let mut cats: Vec<String> = vec!["All Commands".into()];
    for c in &km.commands {
        let cat = category_of(&c.id).to_owned();
        if !cats.contains(&cat) {
            cats.push(cat);
        }
    }
    cats.push("Binding Conflicts".into());
    egui::Panel::left(egui::Id::new("shortcut-categories"))
        .resizable(true)
        .default_size(160.0)
        .show(ui, |ui| {
            for c in &cats {
                let n = if c == "Binding Conflicts" {
                    let n = (0..km.commands.len())
                        .filter(|&i| !km.conflicts(i).is_empty())
                        .count();
                    format!("{c} ({n})")
                } else {
                    c.clone()
                };
                if ui.selectable_label(w.category == *c, n).clicked() {
                    w.category = c.clone();
                }
            }
        });
    egui::CentralPanel::default().show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut w.search)
                    .hint_text("Search commands or keys")
                    .desired_width(240.0),
            );
            if let Some(k) = w.key_filter
                && ui
                    .button(format!("Key: {}  ×", k.symbol_or_name()))
                    .on_hover_text("Show all keys")
                    .clicked()
            {
                w.key_filter = None;
            }
        });
        ui.add_space(4.0);
        let q = w.search.to_lowercase();
        let rows: Vec<usize> = (0..km.commands.len())
            .filter(|&i| {
                let c = &km.commands[i];
                let in_cat = match w.category.as_str() {
                    "All Commands" => true,
                    "Binding Conflicts" => !km.conflicts(i).is_empty(),
                    cat => category_of(&c.id) == cat,
                };
                let labels: Vec<String> = km.bindings[i]
                    .iter()
                    .map(|b| b.label().to_lowercase())
                    .collect();
                let matches = q.is_empty()
                    || c.id.to_lowercase().contains(&q)
                    || labels.iter().any(|l| l.contains(&q));
                let key_ok = w
                    .key_filter
                    .is_none_or(|k| km.bindings[i].iter().any(|b| b.key == k));
                in_cat && matches && key_ok
            })
            .collect();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("shortcut-rows")
                    .num_columns(2)
                    .striped(true)
                    .spacing([24.0, 4.0])
                    .min_col_width(260.0)
                    .show(ui, |ui| {
                        ui.strong("Command");
                        ui.strong("Shortcut");
                        ui.end_row();
                        for i in rows {
                            let c = &km.commands[i];
                            let conflicts = km.conflicts(i);
                            let name = if w.category == "All Commands"
                                || w.category == "Binding Conflicts"
                            {
                                c.id.clone()
                            } else {
                                short_name(&c.id).to_owned()
                            };
                            let mut label = RichText::new(name);
                            if km.bindings[i] != c.defaults {
                                label = label.strong();
                            }
                            let r = ui.label(label);
                            if km.bindings[i] != c.defaults {
                                r.on_hover_text("Changed from the default");
                            }
                            ui.horizontal(|ui| {
                                let capturing = w.capturing.is_some_and(|(j, _)| j == i);
                                let text = if capturing {
                                    "Press a key…  (Esc cancels)".to_owned()
                                } else if km.bindings[i].is_empty() {
                                    "—".to_owned()
                                } else {
                                    km.bindings[i]
                                        .iter()
                                        .map(|b| b.label())
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                };
                                let mut t = RichText::new(text).monospace();
                                if !conflicts.is_empty() {
                                    t = t.color(Color32::from_rgb(240, 120, 90));
                                }
                                let btn = ui.add(
                                    egui::Button::new(t)
                                        .selected(capturing)
                                        .min_size(egui::vec2(150.0, 0.0)),
                                );
                                if btn.clicked() {
                                    w.capturing = Some((i, true));
                                }
                                let btn = if conflicts.is_empty() {
                                    btn.on_hover_text(
                                        "Click to set a new shortcut. Right-click for more.",
                                    )
                                } else {
                                    let names: Vec<&str> = conflicts
                                        .iter()
                                        .map(|&j| km.commands[j].id.as_str())
                                        .collect();
                                    btn.on_hover_text(format!(
                                        "Conflicts with {}",
                                        names.join(", ")
                                    ))
                                };
                                btn.context_menu(|ui| {
                                    if ui.button("Add Another Shortcut").clicked() {
                                        w.capturing = Some((i, false));
                                        ui.close();
                                    }
                                    if ui
                                        .add_enabled(
                                            !km.bindings[i].is_empty(),
                                            egui::Button::new("Remove Shortcut"),
                                        )
                                        .clicked()
                                    {
                                        km.bindings[i].clear();
                                        changed = true;
                                        ui.close();
                                    }
                                    if ui
                                        .add_enabled(
                                            km.bindings[i] != km.commands[i].defaults,
                                            egui::Button::new("Reset to Default"),
                                        )
                                        .clicked()
                                    {
                                        km.bindings[i] = km.commands[i].defaults.clone();
                                        changed = true;
                                        ui.close();
                                    }
                                });
                                if !conflicts.is_empty() {
                                    ui.colored_label(Color32::from_rgb(240, 120, 90), "conflict");
                                }
                            });
                            ui.end_row();
                        }
                    });
            });
    });
    conflict_dialog(st, ui.ctx(), &mut changed);
    if changed {
        st.prefs.shortcuts = st.keymap.overrides();
        st.prefs_changed = true;
    }
}

/// While capturing, the next key press becomes the binding.
fn capture_keys(st: &mut EditorState, ui: &mut egui::Ui, changed: &mut bool) {
    let w = &mut st.shortcuts_ui;
    let Some((i, replace)) = w.capturing else {
        return;
    };
    let press = ui.input(|inp| {
        inp.events.iter().find_map(|e| match e {
            Event::Key {
                key,
                pressed: true,
                repeat: false,
                modifiers,
                ..
            } => Some((*key, *modifiers)),
            _ => None,
        })
    });
    let Some((key, mods)) = press else {
        return;
    };
    ui.input_mut(|inp| inp.events.retain(|e| !matches!(e, Event::Key { .. })));
    w.capturing = None;
    if key == Key::Escape && mods.is_none() {
        return;
    }
    let Some(binding) = Binding::from_event(key, mods) else {
        st.status = "Ctrl shortcuts aren't supported on macOS; use ⌘".into();
        return;
    };
    let km = &mut st.keymap;
    let others: Vec<usize> = (0..km.commands.len())
        .filter(|&j| j != i && km.bindings[j].contains(&binding))
        .collect();
    if others.is_empty() {
        km.bind(i, binding, replace, false);
        *changed = true;
    } else {
        w.conflict = Some((i, binding, replace, others));
    }
}

/// Unity's "Binding Conflict" dialog: reassign, keep both, or cancel.
fn conflict_dialog(st: &mut EditorState, ctx: &egui::Context, changed: &mut bool) {
    let Some((i, binding, replace, others)) = st.shortcuts_ui.conflict.clone() else {
        return;
    };
    let km = &mut st.keymap;
    let mut close = false;
    egui::Modal::new(egui::Id::new("shortcut-conflict")).show(ctx, |ui| {
        ui.set_width(420.0);
        ui.heading("Binding Conflict");
        ui.add_space(4.0);
        let names: Vec<&str> = others.iter().map(|&j| km.commands[j].id.as_str()).collect();
        ui.label(format!(
            "{} is already used by {}.",
            binding.label(),
            names.join(", ")
        ));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui
                .button("Reassign")
                .on_hover_text(format!("Move it to {}", km.commands[i].id))
                .clicked()
            {
                km.bind(i, binding, replace, true);
                *changed = true;
                close = true;
            }
            if ui
                .button("Create Conflict")
                .on_hover_text("Keep both (only the first command will run)")
                .clicked()
            {
                km.bind(i, binding, replace, false);
                *changed = true;
                close = true;
            }
            if ui.button("Cancel").clicked() {
                close = true;
            }
        });
    });
    if close {
        st.shortcuts_ui.conflict = None;
    }
}

/// A keyboard: keys bound with the chosen modifiers are highlighted; clicking one filters
/// the command list to it.
fn keyboard(ui: &mut egui::Ui, w: &mut ShortcutsWindow, km: &Keymap) {
    let mac = cfg!(target_os = "macos");
    ui.horizontal(|ui| {
        ui.label("Modifiers:");
        ui.toggle_value(&mut w.show_cmd, if mac { "⌘ Command" } else { "Ctrl" });
        ui.toggle_value(&mut w.show_shift, "Shift");
        ui.toggle_value(&mut w.show_alt, if mac { "Option" } else { "Alt" });
    });
    let unit = ((ui.available_width() - 20.0) / 15.5).clamp(22.0, 40.0);
    let gap = 3.0;
    for (row, indent) in KEYBOARD.iter().zip(KEYBOARD_INDENT) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            if indent > 0.0 {
                ui.add_space(indent * unit * 0.5);
            }
            for &(key, width) in row.iter() {
                let binding = b(w.show_cmd, w.show_shift, w.show_alt, key);
                let users: Vec<&str> = km
                    .bindings
                    .iter()
                    .enumerate()
                    .filter(|(_, list)| list.contains(&binding))
                    .map(|(i, _)| km.commands[i].id.as_str())
                    .collect();
                let size = egui::vec2(unit * width + gap * (width - 1.0), unit);
                let (rect, r) = ui.allocate_exact_size(size, egui::Sense::click());
                let fill = match users.len() {
                    0 => Color32::from_gray(64),
                    1 => Color32::from_rgb(52, 92, 132),
                    _ => Color32::from_rgb(150, 70, 55),
                };
                let fill = if w.key_filter == Some(key) {
                    Color32::from_rgb(90, 140, 190)
                } else if r.hovered() {
                    fill.gamma_multiply(1.3)
                } else {
                    fill
                };
                ui.painter().rect_filled(rect, 3.0, fill);
                let name = match key {
                    Key::Backspace => "Backspace",
                    Key::Escape => "Esc",
                    Key::Enter => "Enter",
                    Key::Delete => "Del",
                    Key::Space => "Space",
                    Key::Tab => "Tab",
                    k => k.symbol_or_name(),
                };
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    name,
                    egui::FontId::proportional((unit * 0.36).max(9.0)),
                    Color32::from_gray(225),
                );
                if r.clicked() {
                    w.key_filter = (w.key_filter != Some(key)).then_some(key);
                }
                if !users.is_empty() {
                    r.on_hover_text(format!("{}: {}", binding.label(), users.join(", ")));
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bindings_round_trip_through_settings_text() {
        for b in [
            cmd_shift(Key::Z),
            plain(Key::F2),
            cmd_alt(Key::Num3),
            b(false, true, true, Key::N),
        ] {
            assert_eq!(Binding::parse(&b.to_config()), Some(b), "{}", b.to_config());
        }
        assert_eq!(Binding::parse("ctrl+shift+s"), Some(cmd_shift(Key::S)));
        assert_eq!(Binding::parse("Cmd+"), None);
        assert_eq!(Binding::parse("Cmd+NotAKey"), None);
    }

    #[test]
    fn defaults_have_no_conflicts_and_overrides_only_store_changes() {
        let mut km = Keymap::default();
        for i in 0..km.commands.len() {
            assert!(
                km.conflicts(i).is_empty(),
                "{} conflicts",
                km.commands[i].id
            );
        }
        assert!(km.overrides().is_empty());
        let undo = km.find("Main Menu/Edit/Undo").unwrap();
        let save = km.find("Main Menu/File/Save").unwrap();
        // Reassign ⌘S to Undo: Save loses it.
        km.bind(undo, cmd(Key::S), true, true);
        assert_eq!(km.bindings[undo], vec![cmd(Key::S)]);
        assert!(km.bindings[save].is_empty());
        assert_eq!(km.command_for(cmd(Key::S)), Some(undo));
        assert_eq!(km.action_text(&Action::Undo), cmd(Key::S).label());
        let o = km.overrides();
        assert_eq!(o.len(), 2, "{o:?}");
        // Saved and loaded again.
        let again = Keymap::new(&o);
        assert_eq!(again.bindings, km.bindings);
        // "Create Conflict" keeps both.
        let mut km = Keymap::default();
        let dup = km.find("Main Menu/Edit/Duplicate").unwrap();
        km.bind(dup, cmd(Key::S), false, false);
        assert_eq!(km.conflicts(dup), vec![save]);
        assert_eq!(km.bindings[dup].len(), 2);
    }

    fn key(k: Key, m: Modifiers, repeat: bool) -> Event {
        Event::Key {
            key: k,
            physical_key: None,
            pressed: true,
            repeat,
            modifiers: m,
        }
    }

    #[test]
    fn dispatch_rules() {
        let mut km = Keymap::default();
        let ids = |v: Vec<(Binding, usize)>, km: &Keymap| -> Vec<String> {
            v.into_iter()
                .map(|(_, i)| km.commands[i].id.clone())
                .collect()
        };
        let cmd_m = Modifiers::COMMAND;
        let shift_cmd = Modifiers::COMMAND | Modifiers::SHIFT;
        // Exact modifiers: ⇧⌘Z is Redo, not Undo; plain W is the Move tool.
        let ev = [
            key(Key::Z, shift_cmd, false),
            key(Key::W, Modifiers::NONE, false),
        ];
        assert_eq!(
            ids(triggered(&km, &ev, false, false, false), &km),
            ["Main Menu/Edit/Redo", "Tools/Move"]
        );
        // Shift+W is not bound (W alone must not fire with Shift held).
        assert!(
            triggered(
                &km,
                &[key(Key::W, Modifiers::SHIFT, false)],
                false,
                false,
                false
            )
            .is_empty()
        );
        // Typing in a field or playing with the Game view focused: only Play/Pause/Step.
        let ev = [key(Key::S, cmd_m, false), key(Key::P, cmd_m, false)];
        assert_eq!(
            ids(triggered(&km, &ev, true, false, false), &km),
            ["Main Menu/Edit/Play"]
        );
        assert_eq!(
            ids(triggered(&km, &ev, false, true, false), &km),
            ["Main Menu/Edit/Play"]
        );
        // Held keys repeat Undo but not Duplicate; flying the camera blocks tool keys.
        let ev = [
            key(Key::Z, cmd_m, true),
            key(Key::D, cmd_m, true),
            key(Key::E, Modifiers::NONE, false),
        ];
        assert_eq!(
            ids(triggered(&km, &ev, false, false, true), &km),
            ["Main Menu/Edit/Undo"]
        );
        // After rebinding Save to ⌘K, ⌘S does nothing and ⌘K saves.
        let save = km.find("Main Menu/File/Save").unwrap();
        km.bind(save, cmd(Key::K), true, true);
        assert!(triggered(&km, &[key(Key::S, cmd_m, false)], false, false, false).is_empty());
        assert_eq!(
            ids(
                triggered(&km, &[key(Key::K, cmd_m, false)], false, false, false),
                &km
            ),
            ["Main Menu/File/Save"]
        );
    }
}
