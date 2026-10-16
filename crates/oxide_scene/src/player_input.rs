//! Unity's `PlayerInput` component: enables an Input Action Asset's map so scripts can read
//! its actions with `ctx.input().action("Move")`.
//! <https://docs.unity3d.com/Packages/com.unity.inputsystem@1.11/manual/PlayerInput.html>

use crate::component::Component;
use oxide_assets::Handle;
use oxide_core::Reflect;
use oxide_input::InputActionAsset;

#[derive(Clone, Debug, Reflect)]
pub struct PlayerInput {
    /// The Input Action Asset (`.inputactions`).
    pub actions: Option<Handle<InputActionAsset>>,
    /// The map that is enabled (Unity "Default Map"); empty enables every map.
    pub default_map: String,
    /// Only bindings of this control scheme are used (Unity "Default Scheme"); empty uses
    /// every binding.
    pub default_scheme: String,
}

impl Default for PlayerInput {
    fn default() -> Self {
        Self {
            actions: None,
            default_map: "Player".into(),
            default_scheme: String::new(),
        }
    }
}

impl PlayerInput {
    /// Unity `SwitchCurrentActionMap`.
    pub fn switch_current_action_map(&mut self, map: &str) {
        self.default_map = map.to_owned();
    }
}

impl Component for PlayerInput {
    fn allow_multiple(&self) -> bool {
        false
    }
}
