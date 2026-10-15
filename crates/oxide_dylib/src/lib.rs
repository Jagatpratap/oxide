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

/// Declares a Scripts library's components: `Type => "Menu/Path"` for Add Component.
/// Expands to the entry points the editor calls after loading the library.
#[macro_export]
macro_rules! scripts {
    ($($ty:ty => $menu:expr),* $(,)?) => {
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
    };
}
