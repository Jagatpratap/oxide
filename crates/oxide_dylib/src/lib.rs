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
/// optionally `; editors: Type, ...` for components with a custom Inspector,
/// `; assets: Type => ("ext", "Create/Menu")` for data asset types (ScriptableObjects), and
/// `; behaviours: Type, ...` for StateMachineBehaviours (Add Behaviour on Animator states),
/// `; playables: Type, ...` for Timeline `PlayableBehaviour`s (Playable track clips), and
/// `; pipelines: Type, ...` for custom render pipelines (`RenderPipeline`),
/// `; tests: async_fn, ...` for play-mode tests, and `; extensions: register_fn, ...` for
/// editor extensions (`fn(&mut EditorExtensions)`: windows, menu items, property drawers,
/// overlays, Scene view tools).
/// Expands to the entry points the editor calls after loading the library.
#[macro_export]
macro_rules! scripts {
    ($($ty:ty => $menu:expr),* $(,)?
     $(; editors: $($ed:ty),* $(,)?)?
     $(; assets: $($at:ty => ($ext:expr, $amenu:expr)),* $(,)?)?
     $(; behaviours: $($bt:ty),* $(,)?)?
     $(; playables: $($pt:ty),* $(,)?)?
     $(; pipelines: $($rpt:ty),* $(,)?)?
     $(; tests: $($tf:path),* $(,)?)?
     $(; extensions: $($xf:path),* $(,)?)?) => {
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

        /// Registers the data asset types (`assets:` — Unity ScriptableObjects with
        /// `[CreateAssetMenu]`): `Type => ("extension", "Create/Menu/Path")`.
        #[unsafe(no_mangle)]
        pub fn oxide_register_assets(assets: &mut $crate::assets::AssetServer) {
            let _ = &assets;
            $($( $crate::register_script_asset::<$at>(assets, $ext, $amenu); )*)?
        }

        /// Registers the StateMachineBehaviour types (`behaviours:`).
        #[unsafe(no_mangle)]
        pub fn oxide_register_behaviours(b: &mut $crate::animation::StateMachineBehaviours) {
            let _ = &b;
            $($( b.register::<$bt>(); )*)?
        }

        /// Registers the Timeline PlayableBehaviour types (`playables:`).
        #[unsafe(no_mangle)]
        pub fn oxide_register_playables(p: &mut $crate::timeline::PlayableBehaviours) {
            let _ = &p;
            $($( p.register::<$pt>(); )*)?
        }

        /// Registers the custom render pipelines (`pipelines:`, Unity SRP `RenderPipeline`s).
        #[unsafe(no_mangle)]
        pub fn oxide_register_pipelines(p: &mut $crate::render_pipeline::RenderPipelines) {
            let _ = &p;
            $($( p.register::<$rpt>(); )*)?
        }

        /// Registers the play-mode tests (`tests:` — `async fn name(co: Co)`, Unity
        /// `[UnityTest]`), run from the editor's Test Runner.
        #[unsafe(no_mangle)]
        pub fn oxide_register_tests(t: &mut $crate::scene::PlayModeTests) {
            let _ = &t;
            $($( t.register(stringify!($tf), |co| ::std::boxed::Box::pin($tf(co))); )*)?
        }

        /// Registers the editor extensions (`extensions:` — custom windows, menu items,
        /// property drawers, overlays and tools).
        #[unsafe(no_mangle)]
        pub fn oxide_register_editor_extensions(x: &mut $crate::EditorExtensions) {
            let _ = &x;
            $($( $xf(x); )*)?
        }
    };
}

/// Registers a script's data asset type (what `scripts! { ...; assets: ... }` expands to).
pub fn register_script_asset<T>(
    assets: &mut oxide_engine::assets::AssetServer,
    extension: &'static str,
    menu: &'static str,
) where
    T: core::reflect::Reflect + Clone + Default + Send + Sync + 'static,
{
    let name = T::default().type_info().name;
    let extensions: &'static [&'static str] = Box::leak(Box::new([extension]));
    let base = menu.rsplit('/').next().unwrap_or(name);
    let default_name: &'static str = Box::leak(format!("New {base}").into_boxed_str());
    assets.register_data_asset::<T>(
        oxide_engine::assets::AssetTypeInfo::new(name, extensions)
            .menu(menu, default_name, 300)
            .color(150, 200, 140),
    );
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

    #[derive(Clone, Default, Reflect)]
    pub struct LootTable {
        pub drops: Vec<String>,
        pub chance: f32,
    }

    async fn spawns_and_waits(co: crate::prelude::Co) {
        co.next_frame().await;
    }

    crate::scripts! {
        Dial => "Scripts/Dial", Plain => "";
        editors: Dial;
        assets: LootTable => ("loot", "Game/Loot Table");
        tests: spawns_and_waits
    }

    #[test]
    fn scripts_macro_registers_play_mode_tests() {
        let mut t = crate::scene::PlayModeTests::default();
        oxide_register_tests(&mut t);
        assert_eq!(t.names(), ["spawns_and_waits"]);
    }

    #[test]
    fn scripts_macro_registers_data_assets() {
        let dir = std::env::temp_dir().join(format!("oxide-dylib-assets-{}", std::process::id()));
        let mut a = crate::assets::AssetServer::new(&dir);
        oxide_register_assets(&mut a);
        let t = a.types.get("LootTable").expect("registered");
        assert_eq!(t.create_menu, Some("Game/Loot Table"));
        assert_eq!(t.default_file_name(), "New Loot Table.loot");
        assert!(a.types.is_data("LootTable"));
        a.create_data_asset("LootTable", "Assets/Boss.loot")
            .unwrap();
        assert_eq!(
            a.types.for_path("Assets/Boss.loot").unwrap().type_name,
            "LootTable"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

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
