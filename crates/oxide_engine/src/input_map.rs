//! Platform input → Unity `Input`: winit keys/mouse and gilrs gamepads.

use oxide_input::{GamepadAxis, GamepadButton, Input, KeyCode as K, MouseButton};
use winit::keyboard::KeyCode as W;

pub fn key(code: W) -> Option<K> {
    Some(match code {
        W::KeyA => K::A,
        W::KeyB => K::B,
        W::KeyC => K::C,
        W::KeyD => K::D,
        W::KeyE => K::E,
        W::KeyF => K::F,
        W::KeyG => K::G,
        W::KeyH => K::H,
        W::KeyI => K::I,
        W::KeyJ => K::J,
        W::KeyK => K::K,
        W::KeyL => K::L,
        W::KeyM => K::M,
        W::KeyN => K::N,
        W::KeyO => K::O,
        W::KeyP => K::P,
        W::KeyQ => K::Q,
        W::KeyR => K::R,
        W::KeyS => K::S,
        W::KeyT => K::T,
        W::KeyU => K::U,
        W::KeyV => K::V,
        W::KeyW => K::W,
        W::KeyX => K::X,
        W::KeyY => K::Y,
        W::KeyZ => K::Z,
        W::Digit0 => K::Alpha0,
        W::Digit1 => K::Alpha1,
        W::Digit2 => K::Alpha2,
        W::Digit3 => K::Alpha3,
        W::Digit4 => K::Alpha4,
        W::Digit5 => K::Alpha5,
        W::Digit6 => K::Alpha6,
        W::Digit7 => K::Alpha7,
        W::Digit8 => K::Alpha8,
        W::Digit9 => K::Alpha9,
        W::F1 => K::F1,
        W::F2 => K::F2,
        W::F3 => K::F3,
        W::F4 => K::F4,
        W::F5 => K::F5,
        W::F6 => K::F6,
        W::F7 => K::F7,
        W::F8 => K::F8,
        W::F9 => K::F9,
        W::F10 => K::F10,
        W::F11 => K::F11,
        W::F12 => K::F12,
        W::Space => K::Space,
        W::Enter => K::Return,
        W::Escape => K::Escape,
        W::Backspace => K::Backspace,
        W::Tab => K::Tab,
        W::Delete => K::Delete,
        W::Insert => K::Insert,
        W::Home => K::Home,
        W::End => K::End,
        W::PageUp => K::PageUp,
        W::PageDown => K::PageDown,
        W::ArrowUp => K::UpArrow,
        W::ArrowDown => K::DownArrow,
        W::ArrowLeft => K::LeftArrow,
        W::ArrowRight => K::RightArrow,
        W::ShiftLeft => K::LeftShift,
        W::ShiftRight => K::RightShift,
        W::ControlLeft => K::LeftControl,
        W::ControlRight => K::RightControl,
        W::AltLeft => K::LeftAlt,
        W::AltRight => K::RightAlt,
        W::SuperLeft => K::LeftCommand,
        W::SuperRight => K::RightCommand,
        W::Minus => K::Minus,
        W::Equal => K::Equals,
        W::BracketLeft => K::LeftBracket,
        W::BracketRight => K::RightBracket,
        W::Backslash => K::Backslash,
        W::Semicolon => K::Semicolon,
        W::Quote => K::Quote,
        W::Backquote => K::BackQuote,
        W::Comma => K::Comma,
        W::Period => K::Period,
        W::Slash => K::Slash,
        W::CapsLock => K::CapsLock,
        W::Numpad0 => K::Keypad0,
        W::Numpad1 => K::Keypad1,
        W::Numpad2 => K::Keypad2,
        W::Numpad3 => K::Keypad3,
        W::Numpad4 => K::Keypad4,
        W::Numpad5 => K::Keypad5,
        W::Numpad6 => K::Keypad6,
        W::Numpad7 => K::Keypad7,
        W::Numpad8 => K::Keypad8,
        W::Numpad9 => K::Keypad9,
        W::NumpadEnter => K::KeypadEnter,
        W::NumpadAdd => K::KeypadPlus,
        W::NumpadSubtract => K::KeypadMinus,
        _ => return None,
    })
}

pub fn mouse(b: winit::event::MouseButton) -> Option<MouseButton> {
    match b {
        winit::event::MouseButton::Left => Some(MouseButton::Left),
        winit::event::MouseButton::Right => Some(MouseButton::Right),
        winit::event::MouseButton::Middle => Some(MouseButton::Middle),
        _ => None,
    }
}

/// Polls gilrs and feeds connected gamepads into `Input`.
pub struct Gamepads {
    gilrs: Option<gilrs::Gilrs>,
}

impl Gamepads {
    pub fn new() -> Self {
        let gilrs = match gilrs::Gilrs::new() {
            Ok(g) => Some(g),
            Err(e) => {
                oxide_core::debug_log_warning!("gamepads unavailable: {e}");
                None
            }
        };
        Self { gilrs }
    }

    pub fn poll(&mut self, input: &mut Input) {
        let Some(g) = self.gilrs.as_mut() else { return };
        while let Some(ev) = g.next_event() {
            let index: usize = ev.id.into();
            use gilrs::EventType as E;
            match ev.event {
                E::Connected => {
                    let name = g.gamepad(ev.id).name().to_owned();
                    let p = input.gamepad_mut(index);
                    p.connected = true;
                    p.name = name;
                }
                E::Disconnected => input.gamepad_mut(index).connected = false,
                E::ButtonPressed(b, _) => {
                    if let Some(b) = button(b) {
                        input.gamepad_mut(index).connected = true;
                        input.gamepad_button_event(index, b, true);
                    }
                }
                E::ButtonReleased(b, _) => {
                    if let Some(b) = button(b) {
                        input.gamepad_button_event(index, b, false);
                    }
                }
                E::ButtonChanged(b, v, _) => {
                    let a = match b {
                        gilrs::Button::LeftTrigger2 => Some(GamepadAxis::LeftTrigger),
                        gilrs::Button::RightTrigger2 => Some(GamepadAxis::RightTrigger),
                        _ => None,
                    };
                    if let Some(a) = a {
                        input.gamepad_mut(index).axes.insert(a, v);
                    }
                }
                E::AxisChanged(a, v, _) => {
                    let a = match a {
                        gilrs::Axis::LeftStickX => GamepadAxis::LeftStickX,
                        gilrs::Axis::LeftStickY => GamepadAxis::LeftStickY,
                        gilrs::Axis::RightStickX => GamepadAxis::RightStickX,
                        gilrs::Axis::RightStickY => GamepadAxis::RightStickY,
                        _ => continue,
                    };
                    let p = input.gamepad_mut(index);
                    p.connected = true;
                    p.axes.insert(a, v);
                }
                _ => {}
            }
        }
    }
}

impl Default for Gamepads {
    fn default() -> Self {
        Self::new()
    }
}

fn button(b: gilrs::Button) -> Option<GamepadButton> {
    use gilrs::Button as B;
    Some(match b {
        B::South => GamepadButton::South,
        B::East => GamepadButton::East,
        B::West => GamepadButton::West,
        B::North => GamepadButton::North,
        B::LeftTrigger => GamepadButton::LeftShoulder,
        B::RightTrigger => GamepadButton::RightShoulder,
        B::LeftTrigger2 => GamepadButton::LeftTrigger,
        B::RightTrigger2 => GamepadButton::RightTrigger,
        B::Select => GamepadButton::Select,
        B::Start => GamepadButton::Start,
        B::LeftThumb => GamepadButton::LeftStick,
        B::RightThumb => GamepadButton::RightStick,
        B::DPadUp => GamepadButton::DpadUp,
        B::DPadDown => GamepadButton::DpadDown,
        B::DPadLeft => GamepadButton::DpadLeft,
        B::DPadRight => GamepadButton::DpadRight,
        _ => return None,
    })
}
