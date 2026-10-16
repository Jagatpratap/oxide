//! Unity's Input System actions: an Input Action Asset (`.inputactions`) holds action maps
//! ("Player", "UI"), each with actions ("Move", "Jump") bound to controls by path
//! (`<Keyboard>/space`, `<Gamepad>/leftStick`, `<Mouse>/delta`), including 2D Vector and
//! 1D Axis composites, processors and interactions. Every frame the enabled maps are
//! evaluated into [`ActionState`]s that scripts read through [`crate::Input::action`].
//! Reference: <https://docs.unity3d.com/Packages/com.unity.inputsystem@1.11/manual/Actions.html>

use crate::{GamepadAxis, GamepadButton, Input, KeyCode, MouseButton};
use oxide_core::Reflect;
use oxide_math::Vector2;
use std::collections::HashMap;

/// Unity `InputActionType`.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InputActionType {
    /// Continuous value (sticks, WASD): performed whenever it changes while actuated.
    #[default]
    Value,
    /// Performed once per press (with the binding's interaction).
    Button,
    /// Every change, without start/cancel phases.
    PassThrough,
}

/// Unity's "Control Type" of an action: what `read_value` returns.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InputControlType {
    #[default]
    Button,
    Axis,
    Vector2,
}

/// A binding's composite: one control, or parts that make up a value.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InputComposite {
    #[default]
    None,
    /// Up/down/left/right parts (WASD, arrows, D-pad).
    Vector2D,
    /// Negative/positive parts.
    Axis1D,
}

/// One binding of an action (Unity `InputBinding`): a control path, or a composite's parts.
#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct InputBinding {
    pub composite: InputComposite,
    /// Control path for a plain binding, e.g. `<Keyboard>/space`, `<Gamepad>/leftStick`.
    pub path: String,
    /// Vector2D parts.
    pub up: String,
    pub down: String,
    pub left: String,
    pub right: String,
    /// Axis1D parts.
    pub negative: String,
    pub positive: String,
    /// `normalize`, `invert`, `invertX`, `invertY`, `scale(factor=2)`,
    /// `scaleVector2(x=1,y=2)`, `stickDeadzone(min=0.125,max=0.925)`,
    /// `axisDeadzone(min=0.125,max=0.925)`, `clamp(min=0,max=1)`; comma separated.
    pub processors: String,
    /// `press`, `hold(duration=0.4)`, `tap(duration=0.2)`.
    pub interactions: String,
    /// Control schemes this binding belongs to (`Keyboard&Mouse;Gamepad`); empty = all.
    pub groups: String,
}

impl InputBinding {
    pub fn path(path: &str) -> Self {
        Self {
            path: path.into(),
            ..Default::default()
        }
    }

    /// A WASD-style 2D Vector composite.
    pub fn vector2(up: &str, down: &str, left: &str, right: &str) -> Self {
        Self {
            composite: InputComposite::Vector2D,
            up: up.into(),
            down: down.into(),
            left: left.into(),
            right: right.into(),
            ..Default::default()
        }
    }

    pub fn axis(negative: &str, positive: &str) -> Self {
        Self {
            composite: InputComposite::Axis1D,
            negative: negative.into(),
            positive: positive.into(),
            ..Default::default()
        }
    }

    pub fn with_groups(mut self, groups: &str) -> Self {
        self.groups = groups.into();
        self
    }

    pub fn with_processors(mut self, p: &str) -> Self {
        self.processors = p.into();
        self
    }

    /// Short text for lists (`<Keyboard>/space`, `2D Vector`).
    pub fn display(&self) -> String {
        match self.composite {
            InputComposite::None => self.path.clone(),
            InputComposite::Vector2D => "2D Vector".into(),
            InputComposite::Axis1D => "1D Axis".into(),
        }
    }
}

/// Unity `InputAction`.
#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct InputAction {
    pub name: String,
    pub action_type: InputActionType,
    pub control_type: InputControlType,
    pub bindings: Vec<InputBinding>,
    /// Applied to every binding (after the binding's own).
    pub processors: String,
    pub interactions: String,
}

impl InputAction {
    pub fn new(name: &str, action_type: InputActionType, control_type: InputControlType) -> Self {
        Self {
            name: name.into(),
            action_type,
            control_type,
            ..Default::default()
        }
    }

    pub fn bind(mut self, b: InputBinding) -> Self {
        self.bindings.push(b);
        self
    }
}

/// Unity `InputActionMap`.
#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct InputActionMap {
    pub name: String,
    pub actions: Vec<InputAction>,
}

/// Unity `InputControlScheme`: a named set of devices (`<Keyboard>`, `<Mouse>`, `<Gamepad>`).
#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct InputControlScheme {
    pub name: String,
    pub devices: Vec<String>,
}

/// Unity's Input Action Asset (`.inputactions`).
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct InputActionAsset {
    pub maps: Vec<InputActionMap>,
    pub control_schemes: Vec<InputControlScheme>,
}

impl Default for InputActionAsset {
    /// Unity's default actions: a Player map (Move, Look, Attack, Interact, Crouch, Jump,
    /// Sprint) and a UI map, with Keyboard&Mouse and Gamepad schemes.
    fn default() -> Self {
        use InputActionType::*;
        use InputControlType as C;
        let km = "Keyboard&Mouse";
        let pad = "Gamepad";
        let player = InputActionMap {
            name: "Player".into(),
            actions: vec![
                InputAction::new("Move", Value, C::Vector2)
                    .bind(
                        InputBinding::vector2(
                            "<Keyboard>/w",
                            "<Keyboard>/s",
                            "<Keyboard>/a",
                            "<Keyboard>/d",
                        )
                        .with_groups(km),
                    )
                    .bind(
                        InputBinding::vector2(
                            "<Keyboard>/upArrow",
                            "<Keyboard>/downArrow",
                            "<Keyboard>/leftArrow",
                            "<Keyboard>/rightArrow",
                        )
                        .with_groups(km),
                    )
                    .bind(InputBinding::path("<Gamepad>/leftStick").with_groups(pad)),
                InputAction::new("Look", Value, C::Vector2)
                    .bind(InputBinding::path("<Mouse>/delta").with_groups(km))
                    .bind(
                        InputBinding::path("<Gamepad>/rightStick")
                            .with_groups(pad)
                            .with_processors("scaleVector2(x=300,y=300)"),
                    ),
                InputAction::new("Attack", Button, C::Button)
                    .bind(InputBinding::path("<Mouse>/leftButton").with_groups(km))
                    .bind(InputBinding::path("<Gamepad>/rightTrigger").with_groups(pad)),
                InputAction::new("Interact", Button, C::Button)
                    .bind(InputBinding::path("<Keyboard>/e").with_groups(km))
                    .bind(InputBinding::path("<Gamepad>/buttonNorth").with_groups(pad)),
                InputAction::new("Crouch", Button, C::Button)
                    .bind(InputBinding::path("<Keyboard>/c").with_groups(km))
                    .bind(InputBinding::path("<Gamepad>/buttonEast").with_groups(pad)),
                InputAction::new("Jump", Button, C::Button)
                    .bind(InputBinding::path("<Keyboard>/space").with_groups(km))
                    .bind(InputBinding::path("<Gamepad>/buttonSouth").with_groups(pad)),
                InputAction::new("Sprint", Button, C::Button)
                    .bind(InputBinding::path("<Keyboard>/leftShift").with_groups(km))
                    .bind(InputBinding::path("<Gamepad>/leftStickPress").with_groups(pad)),
            ],
        };
        let ui = InputActionMap {
            name: "UI".into(),
            actions: vec![
                InputAction::new("Navigate", PassThrough, C::Vector2)
                    .bind(
                        InputBinding::vector2(
                            "<Keyboard>/upArrow",
                            "<Keyboard>/downArrow",
                            "<Keyboard>/leftArrow",
                            "<Keyboard>/rightArrow",
                        )
                        .with_groups(km),
                    )
                    .bind(InputBinding::path("<Gamepad>/dpad").with_groups(pad)),
                InputAction::new("Submit", Button, C::Button)
                    .bind(InputBinding::path("<Keyboard>/enter").with_groups(km))
                    .bind(InputBinding::path("<Gamepad>/buttonSouth").with_groups(pad)),
                InputAction::new("Cancel", Button, C::Button)
                    .bind(InputBinding::path("<Keyboard>/escape").with_groups(km))
                    .bind(InputBinding::path("<Gamepad>/buttonEast").with_groups(pad)),
            ],
        };
        Self {
            maps: vec![player, ui],
            control_schemes: vec![
                InputControlScheme {
                    name: km.into(),
                    devices: vec!["<Keyboard>".into(), "<Mouse>".into()],
                },
                InputControlScheme {
                    name: pad.into(),
                    devices: vec!["<Gamepad>".into()],
                },
            ],
        }
    }
}

impl oxide_assets::Asset for InputActionAsset {
    const TYPE_NAME: &'static str = "InputActionAsset";
}

impl InputActionAsset {
    pub fn map(&self, name: &str) -> Option<&InputActionMap> {
        self.maps.iter().find(|m| m.name.eq_ignore_ascii_case(name))
    }
}

// ---------------------------------------------------------------------------------------
// Control paths

/// A control a binding path names.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Control {
    Key(KeyCode),
    Mouse(MouseButton),
    MouseDelta,
    MouseScroll,
    MousePosition,
    PadButton(GamepadButton),
    PadAxis(GamepadAxis),
    /// Left or right stick as a vector.
    Stick(bool),
    Dpad,
}

/// Parses a Unity control path (`<Keyboard>/space`, `<Gamepad>/leftStick/x`, ...).
pub fn parse_path(path: &str) -> Option<Control> {
    let p = path.trim();
    let (device, rest) = p.strip_prefix('<')?.split_once(">/")?;
    let rest_l = rest.to_ascii_lowercase();
    match device.to_ascii_lowercase().as_str() {
        "keyboard" => {
            let key = match rest_l.as_str() {
                d if d.len() == 1 && d.as_bytes()[0].is_ascii_digit() => {
                    KeyCode::parse(&format!("Alpha{d}"))
                }
                "enter" => Some(KeyCode::Return),
                "numpadenter" => Some(KeyCode::KeypadEnter),
                "backquote" => Some(KeyCode::BackQuote),
                "leftmeta" | "leftcommand" => Some(KeyCode::LeftCommand),
                "rightmeta" | "rightcommand" => Some(KeyCode::RightCommand),
                "leftctrl" => Some(KeyCode::LeftControl),
                "rightctrl" => Some(KeyCode::RightControl),
                _ => KeyCode::parse(rest),
            }?;
            Some(Control::Key(key))
        }
        "mouse" => match rest_l.as_str() {
            "leftbutton" => Some(Control::Mouse(MouseButton::Left)),
            "rightbutton" => Some(Control::Mouse(MouseButton::Right)),
            "middlebutton" => Some(Control::Mouse(MouseButton::Middle)),
            "delta" => Some(Control::MouseDelta),
            "scroll" => Some(Control::MouseScroll),
            "position" => Some(Control::MousePosition),
            _ => None,
        },
        "gamepad" => {
            use GamepadButton as B;
            Some(match rest_l.as_str() {
                "buttonsouth" => Control::PadButton(B::South),
                "buttoneast" => Control::PadButton(B::East),
                "buttonwest" => Control::PadButton(B::West),
                "buttonnorth" => Control::PadButton(B::North),
                "leftshoulder" => Control::PadButton(B::LeftShoulder),
                "rightshoulder" => Control::PadButton(B::RightShoulder),
                "lefttrigger" => Control::PadAxis(GamepadAxis::LeftTrigger),
                "righttrigger" => Control::PadAxis(GamepadAxis::RightTrigger),
                "select" => Control::PadButton(B::Select),
                "start" => Control::PadButton(B::Start),
                "leftstickpress" => Control::PadButton(B::LeftStick),
                "rightstickpress" => Control::PadButton(B::RightStick),
                "dpad/up" => Control::PadButton(B::DpadUp),
                "dpad/down" => Control::PadButton(B::DpadDown),
                "dpad/left" => Control::PadButton(B::DpadLeft),
                "dpad/right" => Control::PadButton(B::DpadRight),
                "dpad" => Control::Dpad,
                "leftstick" => Control::Stick(true),
                "rightstick" => Control::Stick(false),
                "leftstick/x" => Control::PadAxis(GamepadAxis::LeftStickX),
                "leftstick/y" => Control::PadAxis(GamepadAxis::LeftStickY),
                "rightstick/x" => Control::PadAxis(GamepadAxis::RightStickX),
                "rightstick/y" => Control::PadAxis(GamepadAxis::RightStickY),
                _ => return None,
            })
        }
        _ => None,
    }
}

/// The Unity path of a keyboard key (`<Keyboard>/leftShift`, `<Keyboard>/1`).
pub fn key_path(k: KeyCode) -> String {
    let name = format!("{k:?}");
    let control = match k {
        KeyCode::Return => "enter".to_owned(),
        _ if name.starts_with("Alpha") => name["Alpha".len()..].to_owned(),
        _ => {
            let mut c = name.chars();
            match c.next() {
                Some(f) => f.to_lowercase().chain(c).collect(),
                None => name,
            }
        }
    };
    format!("<Keyboard>/{control}")
}

/// Device of a path (`<Keyboard>`) for control scheme matching.
fn device_of(path: &str) -> &str {
    path.split_once('/').map_or(path, |(d, _)| d)
}

impl Input {
    /// Current value of a control: buttons and axes in `x`, sticks/delta in both.
    fn control_value(&self, c: Control) -> Vector2 {
        let pad = |f: &dyn Fn(&crate::GamepadState) -> f32| {
            self.gamepads
                .iter()
                .filter(|g| g.connected)
                .map(f)
                .fold(0.0f32, |a, b| if b.abs() > a.abs() { b } else { a })
        };
        let b = |on: bool| Vector2::new(if on { 1.0 } else { 0.0 }, 0.0);
        match c {
            Control::Key(k) => b(self.get_key(k)),
            Control::Mouse(m) => b(self.get_mouse_button(m)),
            Control::MouseDelta => self.mouse_delta(),
            Control::MouseScroll => self.mouse_scroll_delta(),
            Control::MousePosition => self.mouse_position(),
            Control::PadButton(pb) => b(self
                .gamepads
                .iter()
                .any(|g| g.connected && g.buttons.contains(&pb))),
            Control::PadAxis(a) => Vector2::new(pad(&|g| g.axis(a)), 0.0),
            Control::Stick(left) => {
                let (x, y) = if left {
                    (GamepadAxis::LeftStickX, GamepadAxis::LeftStickY)
                } else {
                    (GamepadAxis::RightStickX, GamepadAxis::RightStickY)
                };
                Vector2::new(pad(&|g| g.axis(x)), pad(&|g| g.axis(y)))
            }
            Control::Dpad => {
                let on = |pb| {
                    self.gamepads
                        .iter()
                        .any(|g| g.connected && g.buttons.contains(&pb))
                };
                let f = |p: bool| if p { 1.0 } else { 0.0 };
                Vector2::new(
                    f(on(GamepadButton::DpadRight)) - f(on(GamepadButton::DpadLeft)),
                    f(on(GamepadButton::DpadUp)) - f(on(GamepadButton::DpadDown)),
                )
            }
        }
    }

    fn path_value(&self, path: &str) -> Vector2 {
        parse_path(path).map_or(Vector2::ZERO, |c| self.control_value(c))
    }

    /// A binding's value before processors.
    fn binding_value(&self, b: &InputBinding) -> Vector2 {
        let x = |p: &str| self.path_value(p).x;
        match b.composite {
            InputComposite::None => self.path_value(&b.path),
            InputComposite::Vector2D => {
                Vector2::new(x(&b.right) - x(&b.left), x(&b.up) - x(&b.down))
            }
            InputComposite::Axis1D => Vector2::new(x(&b.positive) - x(&b.negative), 0.0),
        }
    }
}

/// Splits `name(a=1,b=2)` into the name and its numeric parameters.
fn parse_call(s: &str) -> (String, HashMap<String, f32>) {
    let s = s.trim();
    let (name, args) = match s.split_once('(') {
        Some((n, rest)) => (n, rest.trim_end_matches(')')),
        None => (s, ""),
    };
    let params = args
        .split(',')
        .filter_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            Some((k.trim().to_ascii_lowercase(), v.trim().parse().ok()?))
        })
        .collect();
    (name.trim().to_ascii_lowercase(), params)
}

/// Splits a processors/interactions list at top-level commas.
fn split_list(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out.into_iter()
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .collect()
}

/// Applies Unity processors to a value.
pub fn apply_processors(mut v: Vector2, processors: &str) -> Vector2 {
    for p in split_list(processors) {
        let (name, a) = parse_call(p);
        let get = |k: &str, d: f32| a.get(k).copied().unwrap_or(d);
        match name.as_str() {
            "normalize" | "normalizevector2" => {
                if v.length() > 1e-6 {
                    v = v.normalize();
                }
            }
            "invert" => v = -v,
            "invertx" => v.x = -v.x,
            "inverty" => v.y = -v.y,
            "invertvector2" => {
                if get("invertx", 1.0) != 0.0 {
                    v.x = -v.x;
                }
                if get("inverty", 1.0) != 0.0 {
                    v.y = -v.y;
                }
            }
            "scale" => v *= get("factor", 1.0),
            "scalevector2" => v = Vector2::new(v.x * get("x", 1.0), v.y * get("y", 1.0)),
            "clamp" => {
                let (lo, hi) = (get("min", 0.0), get("max", 1.0));
                v = Vector2::new(v.x.clamp(lo, hi), v.y.clamp(lo, hi));
            }
            "stickdeadzone" => {
                let (lo, hi) = (get("min", 0.125), get("max", 0.925));
                let m = v.length();
                v = if m < lo {
                    Vector2::ZERO
                } else {
                    v / m * ((m - lo) / (hi - lo)).min(1.0)
                };
            }
            "axisdeadzone" => {
                let (lo, hi) = (get("min", 0.125), get("max", 0.925));
                let f = |x: f32| {
                    if x.abs() < lo {
                        0.0
                    } else {
                        x.signum() * ((x.abs() - lo) / (hi - lo)).min(1.0)
                    }
                };
                v = Vector2::new(f(v.x), f(v.y));
            }
            _ => {}
        }
    }
    v
}

/// Unity `InputActionPhase`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub enum InputActionPhase {
    #[default]
    Waiting,
    Started,
    Performed,
    Canceled,
}

/// An action's state this frame, as scripts read it.
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct ActionState {
    pub value: Vector2,
    pub phase: InputActionPhase,
    /// Actuated past the press point (0.5).
    pub pressed: bool,
    pub pressed_this_frame: bool,
    pub released_this_frame: bool,
    /// Performed this frame (Unity `triggered` / `WasPerformedThisFrame`).
    pub performed_this_frame: bool,
    /// Seconds the current press has lasted.
    pub held_time: f32,
}

impl ActionState {
    pub fn read_vector2(&self) -> Vector2 {
        self.value
    }
    pub fn read_float(&self) -> f32 {
        if self.value.y != 0.0 {
            self.value.length()
        } else {
            self.value.x
        }
    }
    pub fn is_pressed(&self) -> bool {
        self.pressed
    }
    pub fn was_pressed_this_frame(&self) -> bool {
        self.pressed_this_frame
    }
    pub fn was_released_this_frame(&self) -> bool {
        self.released_this_frame
    }
    pub fn was_performed_this_frame(&self) -> bool {
        self.performed_this_frame
    }
    pub fn triggered(&self) -> bool {
        self.performed_this_frame
    }
}

/// Press point for buttons and button-like use of values (Unity's default 0.5).
const PRESS_POINT: f32 = 0.5;

/// Evaluates enabled action maps into action states. `previous` holds last frame's.
pub fn evaluate(
    input: &Input,
    maps: &[(&InputActionMap, Option<&InputControlScheme>)],
    previous: &HashMap<String, ActionState>,
    dt: f32,
) -> HashMap<String, ActionState> {
    let mut out = HashMap::new();
    for (map, scheme) in maps {
        for action in &map.actions {
            let mut value = Vector2::ZERO;
            let mut interactions = action.interactions.clone();
            for b in &action.bindings {
                // Only bindings of the active control scheme (or in no scheme).
                if let Some(s) = scheme
                    && !b.groups.is_empty()
                    && !b
                        .groups
                        .split(';')
                        .any(|g| g.trim().eq_ignore_ascii_case(&s.name))
                {
                    continue;
                }
                if let Some(s) = scheme
                    && b.composite == InputComposite::None
                    && !s.devices.is_empty()
                    && !s
                        .devices
                        .iter()
                        .any(|d| d.eq_ignore_ascii_case(device_of(&b.path)))
                {
                    continue;
                }
                let v = apply_processors(input.binding_value(b), &b.processors);
                // The binding with the largest actuation drives the action (Unity's
                // conflict resolution).
                if v.length() > value.length() {
                    value = v;
                    if !b.interactions.is_empty() {
                        interactions = b.interactions.clone();
                    }
                }
            }
            value = apply_processors(value, &action.processors);
            if action.control_type != InputControlType::Vector2 {
                value.y = 0.0;
            }
            let key = format!("{}/{}", map.name, action.name);
            let prev = previous.get(&key).copied().unwrap_or_default();
            let magnitude = value.length();
            let pressed = magnitude >= PRESS_POINT;
            let mut st = ActionState {
                value,
                pressed,
                pressed_this_frame: pressed && !prev.pressed,
                released_this_frame: !pressed && prev.pressed,
                held_time: if pressed { prev.held_time + dt } else { 0.0 },
                ..Default::default()
            };
            let interaction = split_list(&interactions)
                .first()
                .map(|i| parse_call(i))
                .unwrap_or_default();
            let duration = |d: f32| interaction.1.get("duration").copied().unwrap_or(d);
            match action.action_type {
                InputActionType::PassThrough => {
                    st.phase = InputActionPhase::Performed;
                    st.performed_this_frame = value != prev.value;
                }
                InputActionType::Value => {
                    let active = magnitude > 1e-6;
                    st.phase = if active {
                        InputActionPhase::Performed
                    } else if prev.phase == InputActionPhase::Performed {
                        InputActionPhase::Canceled
                    } else {
                        InputActionPhase::Waiting
                    };
                    st.performed_this_frame = active && value != prev.value;
                }
                InputActionType::Button => match interaction.0.as_str() {
                    "hold" => {
                        let d = duration(0.4);
                        let reached = pressed && st.held_time >= d;
                        st.phase = if reached {
                            InputActionPhase::Performed
                        } else if pressed {
                            InputActionPhase::Started
                        } else if prev.pressed {
                            InputActionPhase::Canceled
                        } else {
                            InputActionPhase::Waiting
                        };
                        st.performed_this_frame = reached && prev.held_time < d;
                    }
                    "tap" => {
                        let d = duration(0.2);
                        st.phase = if pressed {
                            InputActionPhase::Started
                        } else if st.released_this_frame && prev.held_time <= d {
                            InputActionPhase::Performed
                        } else if st.released_this_frame {
                            InputActionPhase::Canceled
                        } else {
                            InputActionPhase::Waiting
                        };
                        st.performed_this_frame = st.phase == InputActionPhase::Performed;
                    }
                    _ => {
                        st.phase = if pressed {
                            InputActionPhase::Performed
                        } else if prev.pressed {
                            InputActionPhase::Canceled
                        } else {
                            InputActionPhase::Waiting
                        };
                        st.performed_this_frame = st.pressed_this_frame;
                    }
                },
            }
            out.insert(key, st);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_parse_and_round_trip() {
        assert_eq!(
            parse_path("<Keyboard>/space"),
            Some(Control::Key(KeyCode::Space))
        );
        assert_eq!(
            parse_path("<Keyboard>/leftShift"),
            Some(Control::Key(KeyCode::LeftShift))
        );
        assert_eq!(
            parse_path("<Keyboard>/1"),
            Some(Control::Key(KeyCode::Alpha1))
        );
        assert_eq!(
            parse_path("<Keyboard>/enter"),
            Some(Control::Key(KeyCode::Return))
        );
        assert_eq!(parse_path("<Mouse>/delta"), Some(Control::MouseDelta));
        assert_eq!(
            parse_path("<Gamepad>/leftStick"),
            Some(Control::Stick(true))
        );
        assert_eq!(
            parse_path("<Gamepad>/dpad/up"),
            Some(Control::PadButton(GamepadButton::DpadUp))
        );
        assert_eq!(parse_path("<Joystick>/x"), None);
        for k in KeyCode::ALL {
            assert_eq!(parse_path(&key_path(*k)), Some(Control::Key(*k)), "{k:?}");
        }
    }

    #[test]
    fn processors() {
        let v = apply_processors(Vector2::new(3.0, 4.0), "normalize, scale(factor=2)");
        assert!((v.length() - 2.0).abs() < 1e-5);
        assert_eq!(
            apply_processors(Vector2::new(1.0, 1.0), "invertY"),
            Vector2::new(1.0, -1.0)
        );
        assert_eq!(
            apply_processors(Vector2::new(0.1, 0.0), "stickDeadzone"),
            Vector2::ZERO
        );
        let full = apply_processors(Vector2::new(1.0, 0.0), "stickDeadzone(min=0.2,max=0.9)");
        assert!((full.x - 1.0).abs() < 1e-5);
        assert_eq!(
            apply_processors(Vector2::new(2.0, 3.0), "scaleVector2(x=2,y=0.5)"),
            Vector2::new(4.0, 1.5)
        );
    }

    #[test]
    fn wasd_composite_buttons_and_hold() {
        let asset = InputActionAsset::default();
        let player = asset.map("Player").unwrap();
        let km = &asset.control_schemes[0];
        let mut input = Input::new();
        let mut prev = HashMap::new();
        let step = |input: &mut Input, prev: &mut HashMap<String, ActionState>| {
            let s = evaluate(input, &[(player, Some(km))], prev, 0.1);
            input.end_frame();
            *prev = s.clone();
            s
        };
        input.key_event(KeyCode::W, true);
        input.key_event(KeyCode::D, true);
        input.key_event(KeyCode::Space, true);
        let s = step(&mut input, &mut prev);
        assert_eq!(s["Player/Move"].value, Vector2::new(1.0, 1.0));
        assert!(s["Player/Move"].performed_this_frame);
        assert!(s["Player/Jump"].triggered(), "pressed this frame");
        let s = step(&mut input, &mut prev);
        assert!(!s["Player/Jump"].triggered(), "only once per press");
        assert!(s["Player/Jump"].is_pressed());
        input.key_event(KeyCode::Space, false);
        let s = step(&mut input, &mut prev);
        assert!(s["Player/Jump"].was_released_this_frame());
        assert_eq!(s["Player/Jump"].phase, InputActionPhase::Canceled);

        // A Hold interaction performs after its duration.
        let mut hold = player.clone();
        hold.actions[5].interactions = "hold(duration=0.25)".into();
        let mut prev = HashMap::new();
        input.key_event(KeyCode::Space, true);
        let mut performed_at = None;
        for i in 0..6 {
            let s = evaluate(&input, &[(&hold, Some(km))], &prev, 0.1);
            input.end_frame();
            if s["Player/Jump"].triggered() {
                performed_at = Some(i);
            }
            prev = s;
        }
        assert_eq!(performed_at, Some(2), "after 0.3 s of holding");
    }

    #[test]
    fn control_schemes_filter_bindings() {
        let asset = InputActionAsset::default();
        let player = asset.map("Player").unwrap();
        let pad = &asset.control_schemes[1];
        let mut input = Input::new();
        input.key_event(KeyCode::Space, true);
        let s = evaluate(&input, &[(player, Some(pad))], &HashMap::new(), 0.0);
        assert!(
            !s["Player/Jump"].is_pressed(),
            "keyboard ignored in the Gamepad scheme"
        );
        input.gamepad_mut(0).connected = true;
        input.gamepad_button_event(0, GamepadButton::South, true);
        let s = evaluate(&input, &[(player, Some(pad))], &HashMap::new(), 0.0);
        assert!(s["Player/Jump"].is_pressed());
        let s = evaluate(&input, &[(player, None)], &HashMap::new(), 0.0);
        assert!(s["Player/Jump"].is_pressed(), "no scheme: every binding");
    }
}
