//! The standalone player: a built game's executable links this (statically) together with
//! the project's Scripts. It opens the first scene of the build and runs it full speed,
//! without the editor. Scripts use it as `oxide`, exactly as they use `oxide_dylib` in the
//! editor, so the same source compiles into both.
//!
//! Data layout next to the executable (Unity's `<Game>_Data` / app bundle resources):
//! `build.yaml` (product name, scenes, window) and `Assets/`.

pub use oxide_engine::*;

/// Scripts compiled into a player never meet a different engine build.
pub const BUILD_ID: &str = "player";

/// Same as `oxide_dylib::scripts!`: declares a Scripts library's components.
#[macro_export]
macro_rules! scripts {
    // Custom Inspectors (`editors:`) are editor-only: accepted and ignored here.
    ($($ty:ty => $menu:expr),* $(,)? $(; editors: $($ed:ty),* $(,)?)?) => {
        pub fn oxide_scripts_build_id() -> &'static str {
            $crate::BUILD_ID
        }

        pub fn oxide_register_scripts(registry: &mut $crate::scene::ComponentRegistry) {
            $( registry.register::<$ty>($menu); )*
        }
    };
}

/// `build.yaml`: what the Build Settings chose.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct BuildInfo {
    pub product_name: String,
    /// Scene paths, first one opens at startup (`SceneManager` build indices).
    pub scenes: Vec<String>,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub fullscreen: bool,
}

/// Where the game's data is: `Contents/Resources/Data` in a macOS app bundle, else
/// `<exe name>_Data` next to the executable.
pub fn data_dir() -> std::path::PathBuf {
    let exe = std::env::current_exe().unwrap_or_default();
    let dir = exe
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_default();
    let bundle = dir.join("../Resources/Data");
    if bundle.join("build.yaml").is_file() {
        return bundle;
    }
    let stem = exe.file_stem().and_then(|s| s.to_str()).unwrap_or("game");
    let beside = dir.join(format!("{stem}_Data"));
    if beside.join("build.yaml").is_file() {
        return beside;
    }
    std::env::current_dir().unwrap_or(dir)
}

/// Runs the game. `register` adds the Scripts' components. Pass `--mcp` to turn the MCP
/// server on (off by default in a shipped game).
pub fn run(register: fn(&mut scene::ComponentRegistry)) -> std::process::ExitCode {
    run_scripts(register, |_| {})
}

/// [`run`] with the Scripts library's data asset types (`scripts! { ...; assets: ... }`).
pub fn run_scripts(
    register: fn(&mut scene::ComponentRegistry),
    register_assets: fn(&mut assets::AssetServer),
) -> std::process::ExitCode {
    let data = data_dir();
    let info: BuildInfo = match std::fs::read_to_string(data.join("build.yaml"))
        .map_err(|e| e.to_string())
        .and_then(|t| serde_yaml_ng::from_str(&t).map_err(|e| e.to_string()))
    {
        Ok(i) => i,
        Err(e) => {
            eprintln!("cannot read {}: {e}", data.join("build.yaml").display());
            return std::process::ExitCode::FAILURE;
        }
    };
    let mcp = std::env::args().any(|a| a == "--mcp");
    let config = EngineConfig {
        title: info.product_name.clone(),
        width: info.width,
        height: info.height,
        fullscreen: info.fullscreen,
        mcp_addr: mcp.then(|| oxide_mcp_default_addr().to_owned()),
        project_root: data,
    };
    let mut app = App::with_config(config);
    register(&mut app.world_mut().registry);
    register_assets(app.world_mut().resource_mut::<assets::AssetServer>());
    app.world_mut().resource_mut::<AppControl>().build_scenes = info.scenes.clone();
    let first = info.scenes.first().cloned();
    let app = app.setup(move |world| match first {
        Some(scene) => {
            if let Err(e) = load_scene(world, &scene, false) {
                self::core::debug_log_error!("{e}");
            }
        }
        None => objects::create_default_scene(world),
    });
    match app.run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{}: {e}", info.product_name);
            std::process::ExitCode::FAILURE
        }
    }
}

fn oxide_mcp_default_addr() -> &'static str {
    "127.0.0.1:47474"
}
