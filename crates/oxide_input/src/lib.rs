//! Unity's `Input` class (Input Manager): keys, mouse buttons, mouse delta, virtual axes
//! (`Horizontal`, `Vertical`, `Mouse X`, ...), buttons (`Jump`, `Fire1`) and gamepads.
//!
//! The platform layer feeds raw events in ([`Input::key_event`], [`Input::mouse_move`], ...)
//! and calls [`Input::end_frame`] after `LateUpdate`, so "down"/"up" last exactly one frame.
//!
//! References: <https://docs.unity3d.com/Manual/class-InputManager.html>,
//! <https://docs.unity3d.com/Manual/InputLegacy.html>

use oxide_math::Vector2;
use std::collections::{HashMap, HashSet};

/// Unity `KeyCode` (the subset with physical keys).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum KeyCode {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    Alpha0,
    Alpha1,
    Alpha2,
    Alpha3,
    Alpha4,
    Alpha5,
    Alpha6,
    Alpha7,
    Alpha8,
    Alpha9,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    Space,
    Return,
    Escape,
    Backspace,
    Tab,
    Delete,
    Insert,
    Home,
    End,
    PageUp,
    PageDown,
    UpArrow,
    DownArrow,
    LeftArrow,
    RightArrow,
    LeftShift,
    RightShift,
    LeftControl,
    RightControl,
    LeftAlt,
    RightAlt,
    LeftCommand,
    RightCommand,
    Minus,
    Equals,
    LeftBracket,
    RightBracket,
    Backslash,
    Semicolon,
    Quote,
    BackQuote,
    Comma,
    Period,
    Slash,
    CapsLock,
    Keypad0,
    Keypad1,
    Keypad2,
    Keypad3,
    Keypad4,
    Keypad5,
    Keypad6,
    Keypad7,
    Keypad8,
    Keypad9,
    KeypadEnter,
    KeypadPlus,
    KeypadMinus,
}

impl KeyCode {
    /// Every key, in declaration order.
    pub const ALL: &[KeyCode] = {
        use KeyCode::*;
        &[
            A,
            B,
            C,
            D,
            E,
            F,
            G,
            H,
            I,
            J,
            K,
            L,
            M,
            N,
            O,
            P,
            Q,
            R,
            S,
            T,
            U,
            V,
            W,
            X,
            Y,
            Z,
            Alpha0,
            Alpha1,
            Alpha2,
            Alpha3,
            Alpha4,
            Alpha5,
            Alpha6,
            Alpha7,
            Alpha8,
            Alpha9,
            F1,
            F2,
            F3,
            F4,
            F5,
            F6,
            F7,
            F8,
            F9,
            F10,
            F11,
            F12,
            Space,
            Return,
            Escape,
            Backspace,
            Tab,
            Delete,
            Insert,
            Home,
            End,
            PageUp,
            PageDown,
            UpArrow,
            DownArrow,
            LeftArrow,
            RightArrow,
            LeftShift,
            RightShift,
            LeftControl,
            RightControl,
            LeftAlt,
            RightAlt,
            LeftCommand,
            RightCommand,
            Minus,
            Equals,
            LeftBracket,
            RightBracket,
            Backslash,
            Semicolon,
            Quote,
            BackQuote,
            Comma,
            Period,
            Slash,
            CapsLock,
            Keypad0,
            Keypad1,
            Keypad2,
            Keypad3,
            Keypad4,
            Keypad5,
            Keypad6,
            Keypad7,
            Keypad8,
            Keypad9,
            KeypadEnter,
            KeypadPlus,
            KeypadMinus,
        ]
    };

    pub fn parse(s: &str) -> Option<KeyCode> {
        use KeyCode::*;
        Self::ALL
            .iter()
            .copied()
            .find(|k| format!("{k:?}").eq_ignore_ascii_case(s))
            .or(match s.to_ascii_lowercase().as_str() {
                "enter" => Some(Return),
                "esc" => Some(Escape),
                "up" => Some(UpArrow),
                "down" => Some(DownArrow),
                "left" => Some(LeftArrow),
                "right" => Some(RightArrow),
                "shift" => Some(LeftShift),
                "ctrl" | "control" => Some(LeftControl),
                _ => None,
            })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

impl MouseButton {
    pub fn index(self) -> usize {
        match self {
            MouseButton::Left => 0,
            MouseButton::Right => 1,
            MouseButton::Middle => 2,
        }
    }
}

/// Gamepad buttons, named like Unity's Input System `Gamepad` (south = A / Cross).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum GamepadButton {
    South,
    East,
    West,
    North,
    LeftShoulder,
    RightShoulder,
    LeftTrigger,
    RightTrigger,
    Select,
    Start,
    LeftStick,
    RightStick,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum GamepadAxis {
    LeftStickX,
    LeftStickY,
    RightStickX,
    RightStickY,
    LeftTrigger,
    RightTrigger,
}

/// What drives a virtual axis or button.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum AxisSource {
    /// Negative/positive key pair, smoothed with `gravity`/`sensitivity` like Unity.
    Keys {
        negative: Vec<KeyCode>,
        positive: Vec<KeyCode>,
    },
    MouseX,
    MouseY,
    ScrollWheel,
    Gamepad(GamepadAxis),
    /// Two gamepad buttons acting as -1/+1.
    GamepadButtons {
        negative: GamepadButton,
        positive: GamepadButton,
    },
}

/// Unity Input Manager axis entry.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct InputAxis {
    pub name: String,
    pub sources: Vec<AxisSource>,
    /// Units per second the key value falls back to 0.
    pub gravity: f32,
    /// Units per second the key value moves towards the target.
    pub sensitivity: f32,
    pub dead: f32,
    /// Jump to 0 when the opposite key is pressed.
    pub snap: bool,
    pub invert: bool,
}

impl InputAxis {
    /// A keyboard axis with Unity's defaults (gravity 3, sensitivity 3, dead 0.001, snap).
    pub fn new(name: &str) -> Self {
        let mut a = Self::keys(name, &[], &[], None);
        a.dead = 0.001;
        a
    }

    fn keys(name: &str, neg: &[KeyCode], pos: &[KeyCode], pad: Option<GamepadAxis>) -> Self {
        let mut sources = vec![AxisSource::Keys {
            negative: neg.to_vec(),
            positive: pos.to_vec(),
        }];
        if let Some(a) = pad {
            sources.push(AxisSource::Gamepad(a));
        }
        Self {
            name: name.into(),
            sources,
            gravity: 3.0,
            sensitivity: 3.0,
            dead: 0.19,
            snap: true,
            invert: false,
        }
    }

    fn raw(name: &str, src: AxisSource, sensitivity: f32) -> Self {
        Self {
            name: name.into(),
            sources: vec![src],
            gravity: 0.0,
            sensitivity,
            dead: 0.0,
            snap: false,
            invert: false,
        }
    }
}

/// A named button (`Jump`, `Fire1`, ...): any listed key, mouse button or pad button.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct InputButton {
    pub name: String,
    pub keys: Vec<KeyCode>,
    pub mouse: Vec<MouseButton>,
    pub pad: Vec<GamepadButton>,
}

#[derive(Clone, Debug, Default)]
pub struct GamepadState {
    pub name: String,
    pub connected: bool,
    pub buttons: HashSet<GamepadButton>,
    pub buttons_down: HashSet<GamepadButton>,
    pub buttons_up: HashSet<GamepadButton>,
    pub axes: HashMap<GamepadAxis, f32>,
}

impl GamepadState {
    pub fn axis(&self, a: GamepadAxis) -> f32 {
        self.axes.get(&a).copied().unwrap_or(0.0)
    }
}

/// Unity `CursorLockMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize)]
pub enum CursorLockMode {
    #[default]
    None,
    Locked,
    Confined,
}

pub struct Input {
    keys: HashSet<KeyCode>,
    keys_down: HashSet<KeyCode>,
    keys_up: HashSet<KeyCode>,
    mouse: [bool; 3],
    mouse_down: [bool; 3],
    mouse_up: [bool; 3],
    mouse_position: Vector2,
    mouse_delta: Vector2,
    scroll: Vector2,
    text: String,
    axes: Vec<InputAxis>,
    axis_values: HashMap<String, f32>,
    buttons: Vec<InputButton>,
    pub gamepads: Vec<GamepadState>,
    /// Unity `Cursor.lockState` / `Cursor.visible`; the platform layer applies them.
    pub cursor_lock: CursorLockMode,
    pub cursor_visible: bool,
    /// Screen height in pixels (to flip mouse Y to Unity's bottom-left origin).
    pub screen_height: f32,
    /// False while the editor (not the game) has keyboard focus.
    pub enabled: bool,
}

impl Default for Input {
    fn default() -> Self {
        Self::new()
    }
}

impl Input {
    /// Input with Unity's default Input Manager axes and buttons.
    pub fn new() -> Self {
        use KeyCode::*;
        let axes = vec![
            InputAxis::keys(
                "Horizontal",
                &[A, LeftArrow],
                &[D, RightArrow],
                Some(GamepadAxis::LeftStickX),
            ),
            InputAxis::keys(
                "Vertical",
                &[S, DownArrow],
                &[W, UpArrow],
                Some(GamepadAxis::LeftStickY),
            ),
            InputAxis::raw("Mouse X", AxisSource::MouseX, 0.1),
            InputAxis::raw("Mouse Y", AxisSource::MouseY, 0.1),
            InputAxis::raw("Mouse ScrollWheel", AxisSource::ScrollWheel, 0.1),
            InputAxis::raw("Look X", AxisSource::Gamepad(GamepadAxis::RightStickX), 1.0),
            InputAxis::raw("Look Y", AxisSource::Gamepad(GamepadAxis::RightStickY), 1.0),
            InputAxis::raw(
                "Throttle",
                AxisSource::Gamepad(GamepadAxis::RightTrigger),
                1.0,
            ),
            InputAxis::raw("Brake", AxisSource::Gamepad(GamepadAxis::LeftTrigger), 1.0),
        ];
        let b = |name: &str, keys: &[KeyCode], mouse: &[MouseButton], pad: &[GamepadButton]| {
            InputButton {
                name: name.into(),
                keys: keys.to_vec(),
                mouse: mouse.to_vec(),
                pad: pad.to_vec(),
            }
        };
        let buttons = vec![
            b(
                "Fire1",
                &[LeftControl],
                &[MouseButton::Left],
                &[GamepadButton::RightTrigger],
            ),
            b(
                "Fire2",
                &[LeftAlt],
                &[MouseButton::Right],
                &[GamepadButton::LeftTrigger],
            ),
            b(
                "Fire3",
                &[LeftShift],
                &[MouseButton::Middle],
                &[GamepadButton::West],
            ),
            b("Jump", &[Space], &[], &[GamepadButton::South]),
            b(
                "Submit",
                &[Return, KeypadEnter],
                &[],
                &[GamepadButton::South],
            ),
            b("Cancel", &[Escape], &[], &[GamepadButton::East]),
            b("Sprint", &[LeftShift], &[], &[GamepadButton::LeftStick]),
            b("Crouch", &[C, LeftControl], &[], &[GamepadButton::East]),
            b("Reload", &[R], &[], &[GamepadButton::West]),
            b("Interact", &[E, F], &[], &[GamepadButton::North]),
            b("Handbrake", &[Space], &[], &[GamepadButton::East]),
            b("Shift Up", &[PageUp], &[], &[GamepadButton::RightShoulder]),
            b(
                "Shift Down",
                &[PageDown],
                &[],
                &[GamepadButton::LeftShoulder],
            ),
        ];
        Self {
            keys: HashSet::new(),
            keys_down: HashSet::new(),
            keys_up: HashSet::new(),
            mouse: [false; 3],
            mouse_down: [false; 3],
            mouse_up: [false; 3],
            mouse_position: Vector2::ZERO,
            mouse_delta: Vector2::ZERO,
            scroll: Vector2::ZERO,
            text: String::new(),
            axes,
            axis_values: HashMap::new(),
            buttons,
            gamepads: Vec::new(),
            cursor_lock: CursorLockMode::None,
            cursor_visible: true,
            screen_height: 0.0,
            enabled: true,
        }
    }

    // ---- platform feed -------------------------------------------------------------

    pub fn key_event(&mut self, key: KeyCode, pressed: bool) {
        if pressed {
            if self.keys.insert(key) {
                self.keys_down.insert(key);
            }
        } else if self.keys.remove(&key) {
            self.keys_up.insert(key);
        }
    }

    pub fn mouse_button_event(&mut self, b: MouseButton, pressed: bool) {
        let i = b.index();
        if pressed && !self.mouse[i] {
            self.mouse_down[i] = true;
        }
        if !pressed && self.mouse[i] {
            self.mouse_up[i] = true;
        }
        self.mouse[i] = pressed;
    }

    /// Cursor position in window pixels, origin top-left.
    pub fn mouse_move(&mut self, x: f32, y: f32) {
        self.mouse_position = Vector2::new(x, self.screen_height - y);
    }

    /// Raw relative motion (works while the cursor is locked). +Y is down, as reported.
    pub fn mouse_motion(&mut self, dx: f32, dy: f32) {
        self.mouse_delta += Vector2::new(dx, -dy);
    }

    pub fn scroll_event(&mut self, dx: f32, dy: f32) {
        self.scroll += Vector2::new(dx, dy);
    }

    pub fn text_event(&mut self, s: &str) {
        self.text.push_str(s);
    }

    /// Releases everything (window lost focus).
    pub fn release_all(&mut self) {
        let held: Vec<_> = self.keys.iter().copied().collect();
        for k in held {
            self.key_event(k, false);
        }
        for b in [MouseButton::Left, MouseButton::Right, MouseButton::Middle] {
            self.mouse_button_event(b, false);
        }
    }

    /// Advances smoothed axes by `dt`; call once per frame before `Update`.
    pub fn update_axes(&mut self, dt: f32) {
        for axis in &self.axes {
            let prev = self.axis_values.get(&axis.name).copied().unwrap_or(0.0);
            let mut value = 0.0f32;
            let mut digital_target = None;
            for src in &axis.sources {
                match src {
                    AxisSource::Keys { negative, positive } => {
                        let n = negative.iter().any(|k| self.keys.contains(k));
                        let p = positive.iter().any(|k| self.keys.contains(k));
                        let t = (p as i32 - n as i32) as f32;
                        if n || p {
                            digital_target = Some(t);
                        }
                    }
                    AxisSource::MouseX => value += self.mouse_delta.x * axis.sensitivity,
                    AxisSource::MouseY => value += self.mouse_delta.y * axis.sensitivity,
                    AxisSource::ScrollWheel => value += self.scroll.y * axis.sensitivity,
                    AxisSource::Gamepad(a) => {
                        for pad in self.gamepads.iter().filter(|p| p.connected) {
                            let v = pad.axis(*a);
                            if v.abs() > axis.dead && v.abs() > value.abs() {
                                value = v;
                            }
                        }
                    }
                    AxisSource::GamepadButtons { negative, positive } => {
                        for pad in self.gamepads.iter().filter(|p| p.connected) {
                            let t = pad.buttons.contains(positive) as i32
                                - pad.buttons.contains(negative) as i32;
                            if t != 0 {
                                value = t as f32;
                            }
                        }
                    }
                }
            }
            let has_keys = axis
                .sources
                .iter()
                .any(|s| matches!(s, AxisSource::Keys { .. }));
            let v = if has_keys && value == 0.0 {
                // Unity-style smoothing for digital keys.
                match digital_target {
                    Some(t) => {
                        let start = if axis.snap && prev * t < 0.0 {
                            0.0
                        } else {
                            prev
                        };
                        move_towards(start, t, axis.sensitivity * dt)
                    }
                    None => move_towards(prev, 0.0, axis.gravity * dt),
                }
            } else {
                value
            };
            let v = if axis.invert { -v } else { v };
            self.axis_values.insert(axis.name.clone(), v);
        }
    }

    /// Clears per-frame state; call after `LateUpdate`.
    pub fn end_frame(&mut self) {
        self.keys_down.clear();
        self.keys_up.clear();
        self.mouse_down = [false; 3];
        self.mouse_up = [false; 3];
        self.mouse_delta = Vector2::ZERO;
        self.scroll = Vector2::ZERO;
        self.text.clear();
        for p in &mut self.gamepads {
            p.buttons_down.clear();
            p.buttons_up.clear();
        }
    }

    pub fn gamepad_mut(&mut self, index: usize) -> &mut GamepadState {
        if self.gamepads.len() <= index {
            self.gamepads.resize_with(index + 1, GamepadState::default);
        }
        &mut self.gamepads[index]
    }

    pub fn gamepad_button_event(&mut self, index: usize, b: GamepadButton, pressed: bool) {
        let p = self.gamepad_mut(index);
        if pressed {
            if p.buttons.insert(b) {
                p.buttons_down.insert(b);
            }
        } else if p.buttons.remove(&b) {
            p.buttons_up.insert(b);
        }
    }

    // ---- Unity API -----------------------------------------------------------------

    pub fn get_key(&self, k: KeyCode) -> bool {
        self.enabled && self.keys.contains(&k)
    }
    pub fn get_key_down(&self, k: KeyCode) -> bool {
        self.enabled && self.keys_down.contains(&k)
    }
    pub fn get_key_up(&self, k: KeyCode) -> bool {
        self.enabled && self.keys_up.contains(&k)
    }
    pub fn any_key(&self) -> bool {
        self.enabled && (!self.keys.is_empty() || self.mouse.iter().any(|&b| b))
    }
    pub fn any_key_down(&self) -> bool {
        self.enabled && (!self.keys_down.is_empty() || self.mouse_down.iter().any(|&b| b))
    }
    pub fn get_mouse_button(&self, b: MouseButton) -> bool {
        self.enabled && self.mouse[b.index()]
    }
    pub fn get_mouse_button_down(&self, b: MouseButton) -> bool {
        self.enabled && self.mouse_down[b.index()]
    }
    pub fn get_mouse_button_up(&self, b: MouseButton) -> bool {
        self.enabled && self.mouse_up[b.index()]
    }
    /// Pixels, origin bottom-left (Unity `Input.mousePosition`).
    pub fn mouse_position(&self) -> Vector2 {
        self.mouse_position
    }
    /// Raw pixels moved this frame, +Y up.
    pub fn mouse_delta(&self) -> Vector2 {
        if self.enabled {
            self.mouse_delta
        } else {
            Vector2::ZERO
        }
    }
    pub fn mouse_scroll_delta(&self) -> Vector2 {
        if self.enabled {
            self.scroll
        } else {
            Vector2::ZERO
        }
    }
    /// Characters typed this frame (Unity `Input.inputString`).
    pub fn input_string(&self) -> &str {
        &self.text
    }

    /// Smoothed virtual axis in -1..1 (Unity `Input.GetAxis`). Unknown names return 0.
    pub fn get_axis(&self, name: &str) -> f32 {
        if !self.enabled {
            return 0.0;
        }
        self.axis_values.get(name).copied().unwrap_or(0.0)
    }

    /// Unsmoothed axis: -1, 0 or 1 for keys (Unity `Input.GetAxisRaw`).
    pub fn get_axis_raw(&self, name: &str) -> f32 {
        if !self.enabled {
            return 0.0;
        }
        let Some(axis) = self.axes.iter().find(|a| a.name == name) else {
            return 0.0;
        };
        for src in &axis.sources {
            if let AxisSource::Keys { negative, positive } = src {
                let n = negative.iter().any(|k| self.keys.contains(k));
                let p = positive.iter().any(|k| self.keys.contains(k));
                if n || p {
                    return (p as i32 - n as i32) as f32;
                }
            }
        }
        self.get_axis(name)
    }

    fn button(
        &self,
        name: &str,
        keys: impl Fn(&KeyCode) -> bool,
        mouse: impl Fn(usize) -> bool,
        pad: impl Fn(&GamepadState, &GamepadButton) -> bool,
    ) -> bool {
        if !self.enabled {
            return false;
        }
        self.buttons.iter().filter(|b| b.name == name).any(|b| {
            b.keys.iter().any(&keys)
                || b.mouse.iter().any(|m| mouse(m.index()))
                || b.pad
                    .iter()
                    .any(|pb| self.gamepads.iter().any(|g| g.connected && pad(g, pb)))
        })
    }

    pub fn get_button(&self, name: &str) -> bool {
        self.button(
            name,
            |k| self.keys.contains(k),
            |i| self.mouse[i],
            |g, b| g.buttons.contains(b),
        )
    }
    pub fn get_button_down(&self, name: &str) -> bool {
        self.button(
            name,
            |k| self.keys_down.contains(k),
            |i| self.mouse_down[i],
            |g, b| g.buttons_down.contains(b),
        )
    }
    pub fn get_button_up(&self, name: &str) -> bool {
        self.button(
            name,
            |k| self.keys_up.contains(k),
            |i| self.mouse_up[i],
            |g, b| g.buttons_up.contains(b),
        )
    }

    pub fn axes(&self) -> &[InputAxis] {
        &self.axes
    }
    pub fn add_axis(&mut self, axis: InputAxis) {
        self.axes.retain(|a| a.name != axis.name);
        self.axes.push(axis);
    }
    pub fn buttons(&self) -> &[InputButton] {
        &self.buttons
    }
    pub fn add_button(&mut self, b: InputButton) {
        self.buttons.push(b);
    }
    /// Replaces the Input Manager's axes and buttons (Project Settings → Input Manager).
    pub fn set_mappings(&mut self, axes: Vec<InputAxis>, buttons: Vec<InputButton>) {
        self.axes = axes;
        self.buttons = buttons;
        self.axis_values.clear();
    }
    pub fn pressed_keys(&self) -> Vec<KeyCode> {
        let mut v: Vec<_> = self.keys.iter().copied().collect();
        v.sort();
        v
    }
}

fn move_towards(current: f32, target: f32, max_delta: f32) -> f32 {
    if (target - current).abs() <= max_delta {
        target
    } else {
        current + (target - current).signum() * max_delta
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_down_lasts_one_frame() {
        let mut i = Input::new();
        i.key_event(KeyCode::W, true);
        assert!(i.get_key_down(KeyCode::W) && i.get_key(KeyCode::W));
        i.end_frame();
        assert!(!i.get_key_down(KeyCode::W) && i.get_key(KeyCode::W));
        i.key_event(KeyCode::W, false);
        assert!(i.get_key_up(KeyCode::W));
    }

    #[test]
    fn axis_smoothing() {
        let mut i = Input::new();
        i.key_event(KeyCode::D, true);
        i.update_axes(0.1);
        assert!((i.get_axis("Horizontal") - 0.3).abs() < 1e-5);
        assert_eq!(i.get_axis_raw("Horizontal"), 1.0);
        for _ in 0..10 {
            i.update_axes(0.1);
        }
        assert_eq!(i.get_axis("Horizontal"), 1.0);
        i.key_event(KeyCode::D, false);
        i.update_axes(0.1);
        assert!((i.get_axis("Horizontal") - 0.7).abs() < 1e-5);
    }

    #[test]
    fn buttons() {
        let mut i = Input::new();
        i.key_event(KeyCode::Space, true);
        assert!(i.get_button_down("Jump"));
        i.mouse_button_event(MouseButton::Left, true);
        assert!(i.get_button("Fire1"));
        assert_eq!(KeyCode::parse("space"), Some(KeyCode::Space));
    }
}
