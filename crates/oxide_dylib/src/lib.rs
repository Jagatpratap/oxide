//! The Oxide engine and editor as one Rust `dylib`.
//!
//! A project's editor executable and its `Scripts` library both link this, so they share
//! one copy of every engine type and static (the World, resources, the Console). That is
//! what lets the editor load a freshly compiled Scripts library into the running process
//! (Unity's domain reload). Scripts use it as `oxide`:
//!
//! ```ignore
//! use oxide::prelude::*;
//!
//! #[derive(Clone, Default, Reflect)]
//! pub struct Spinner { pub speed: f32 }
//!
//! impl Component for Spinner {
//!     fn update(&mut self, ctx: &mut Ctx) {
//!         let a = self.speed * ctx.delta_time();
//!         ctx.transform().rotate(Vector3::new(0.0, a, 0.0));
//!     }
//! }
//!
//! oxide::scripts! { Spinner => "Scripts/Spinner" }
//! ```

pub use oxide_editor as editor;
pub use oxide_engine::*;

/// Identifies this build of the engine library (see [`scripts!`]).
pub const BUILD_ID: &str = env!("OXIDE_BUILD_ID");

/// The build id of the engine library loaded in this process. Compared with the
/// [`BUILD_ID`] a Scripts library was compiled with.
#[inline(never)]
pub fn loaded_build_id() -> &'static str {
    BUILD_ID
}

/// Declares a Scripts library's components: `Type => "Menu/Path"` for Add Component, then
/// optionally `; editors: Type, ...` for components with a custom Inspector.
/// Expands to the entry points the editor calls after loading the library.
#[macro_export]
macro_rules! scripts {
    ($($ty:ty => $menu:expr),* $(,)? $(; editors: $($ed:ty),* $(,)?)?) => {
        /// Engine build these scripts were compiled against.
        #[unsafe(no_mangle)]
        pub fn oxide_scripts_build_id() -> &'static str {
            $crate::BUILD_ID
        }

        /// Registers every script component with the editor.
        #[unsafe(no_mangle)]
        pub fn oxide_register_scripts(registry: &mut $crate::scene::ComponentRegistry) {
            $( registry.register::<$ty>($menu); )*
        }

        /// Registers the custom Inspectors (`editors:` types implement `CustomEditor`).
        #[unsafe(no_mangle)]
        pub fn oxide_register_editors(editors: &mut $crate::CustomEditors) {
            let _ = &editors;
            $($( editors.register::<$ed>(); )*)?
        }
    };
}

#[cfg(test)]
mod tests {
    use crate::prelude::*;

    #[derive(Clone, Default, Reflect)]
    pub struct Dial {
        pub value: f32,
    }

    impl Component for Dial {}

    impl CustomEditor for Dial {
        fn on_inspector_gui(&mut self, gui: &mut InspectorGui) {
            gui.slider("Value", &mut self.value, 0.0, 1.0);
        }
    }

    #[derive(Clone, Default, Reflect)]
    pub struct Plain {
        pub on: bool,
    }

    impl Component for Plain {}

    crate::scripts! { Dial => "Scripts/Dial", Plain => ""; editors: Dial }

    #[test]
    fn scripts_macro_registers_components_and_editors() {
        let mut reg = crate::scene::ComponentRegistry::default();
        oxide_register_scripts(&mut reg);
        assert!(reg.get("Dial").is_some() && reg.get("Plain").is_some());
        let mut editors = crate::CustomEditors::default();
        oxide_register_editors(&mut editors);
        assert!(editors.get("Dial").is_some());
        assert!(editors.get("Plain").is_none());
    }
}
