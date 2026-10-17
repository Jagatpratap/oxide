//! Entry point of a project's editor executable (generated in `Library/Build/editor`).
//!
//! ```text
//! oxide-project-editor [--project DIR] [--scene Assets/Scenes/Main.scene]
//!                      [--mcp-addr 127.0.0.1:47474] [--no-mcp] [--size 1600x960]
//! ```
//! Opens the scene given, else the last scene opened in this project, else
//! `Assets/Scenes/Main.scene`, else a new default scene.

use crate::Editor;
use crate::scripting::ScriptHost;
use oxide_core::debug_log_error;
use oxide_engine::{App, EngineConfig, objects};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "usage: oxide-project-editor [--project DIR] [--scene PATH] [--mcp-addr HOST:PORT] [--no-mcp] [--size WxH]";

/// File remembering the last scene opened in a project.
pub fn last_scene_file(project: &Path) -> PathBuf {
    project.join("Library/Oxide/LastScene.txt")
}

/// Scenes listed in the Hierarchy but not loaded, handed from the launcher to the editor.
pub struct PendingUnloadedScenes(pub Vec<String>);

/// Remembers the open scenes for the next launch (Unity's scene setup): the main scene on
/// the first line, then `+path` for additive scenes, `-path` for unloaded ones and
/// `*path` for the active scene.
pub fn remember_scene_setup(world: &oxide_scene::World, unloaded: &[String]) {
    let Some(main) = world.scene_path.clone() else {
        return;
    };
    let project = world.resource::<oxide_assets::AssetServer>().full_path("");
    let mut text = main;
    for sc in world.loaded_scenes().into_iter().skip(1) {
        if let Some(p) = sc.path {
            text.push_str(&format!("\n+{p}"));
        }
    }
    for p in unloaded {
        text.push_str(&format!("\n-{p}"));
    }
    if let Some(p) = world.scene_info(world.active_scene()).and_then(|s| s.path) {
        text.push_str(&format!("\n*{p}"));
    }
    let f = last_scene_file(&project);
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(f, text);
}

/// The saved scene setup: (additive, unloaded, active).
fn saved_setup(project: &Path) -> (Vec<String>, Vec<String>, Option<String>) {
    let text = std::fs::read_to_string(last_scene_file(project)).unwrap_or_default();
    let mut add = Vec::new();
    let mut unloaded = Vec::new();
    let mut active = None;
    for line in text.lines().skip(1).map(str::trim) {
        let exists = |p: &str| project.join(p).is_file();
        if let Some(p) = line.strip_prefix('+').filter(|p| exists(p)) {
            add.push(p.to_owned());
        } else if let Some(p) = line.strip_prefix('-').filter(|p| exists(p)) {
            unloaded.push(p.to_owned());
        } else if let Some(p) = line.strip_prefix('*') {
            active = Some(p.to_owned());
        }
    }
    (add, unloaded, active)
}

fn initial_scene(project: &Path, arg: Option<String>) -> Option<String> {
    arg.or_else(|| {
        std::fs::read_to_string(last_scene_file(project))
            .ok()
            .and_then(|s| s.lines().next().map(|l| l.trim().to_owned()))
            .filter(|s| project.join(s).is_file())
    })
    .or_else(|| {
        let main = "Assets/Scenes/Main.scene";
        project.join(main).is_file().then(|| main.to_owned())
    })
}

/// Opens the editor's first scene: `scene` if given, otherwise the scene setup from the
/// last session (main scene, additive and unloaded scenes, active scene), otherwise
/// `Assets/Scenes/Main.scene`, otherwise a new default scene.
/// The editor's first AssetDatabase scan (before the first scene loads, so references to
/// assets moved while the editor was closed resolve); the editor fixes files on its first frame.
pub struct StartupRefresh(pub oxide_assets::database::RefreshReport);

pub fn load_initial_scenes(world: &mut oxide_scene::World, project: &Path, scene: Option<String>) {
    let report = world
        .resource_mut::<oxide_assets::AssetServer>()
        .database
        .refresh();
    world.resources.insert(StartupRefresh(report));
    // The saved multi-scene setup applies only when reopening the last scene.
    let (additive, unloaded, active) = if scene.is_none() {
        saved_setup(project)
    } else {
        Default::default()
    };
    let Some(path) = initial_scene(project, scene) else {
        objects::create_default_scene(world);
        return;
    };
    if let Err(e) = oxide_engine::load_scene(world, &path, false) {
        debug_log_error!("{e}");
        objects::create_default_scene(world);
        return;
    }
    for p in &additive {
        if let Err(e) = oxide_engine::load_scene(world, p, true) {
            debug_log_error!("{e}");
        }
    }
    if let Some(h) = active.as_deref().and_then(|p| world.find_scene(p)) {
        let _ = world.set_active_scene(h);
    }
    world.resources.insert(PendingUnloadedScenes(unloaded));
}

/// Runs the editor on a project whose Scripts link the same engine library (`build_id`).
pub fn run_project_editor(build_id: &'static str) -> ExitCode {
    let mut config = EngineConfig {
        title: "Oxide Editor".into(),
        width: 1600,
        height: 960,
        ..Default::default()
    };
    let mut scene = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut value = |name: &str| {
            args.next()
                .ok_or_else(|| format!("{name} needs a value\n{USAGE}"))
        };
        let r: Result<(), String> = match a.as_str() {
            "--project" => value("--project").map(|v| config.project_root = v.into()),
            "--scene" => value("--scene").map(|v| scene = Some(v)),
            "--mcp-addr" => value("--mcp-addr").map(|v| config.mcp_addr = Some(v)),
            "--no-mcp" => {
                config.mcp_addr = None;
                Ok(())
            }
            "--size" => value("--size").and_then(|v| {
                let (w, h) = v.split_once('x').ok_or("--size must look like 1600x960")?;
                config.width = w.parse().map_err(|_| "bad width")?;
                config.height = h.parse().map_err(|_| "bad height")?;
                Ok(())
            }),
            "-h" | "--help" => Err(USAGE.into()),
            other => Err(format!("unknown argument `{other}`\n{USAGE}")),
        };
        if let Err(e) = r {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    }
    let project =
        std::fs::canonicalize(&config.project_root).unwrap_or(config.project_root.clone());
    config.project_root = project.clone();
    if let Some(name) = project.file_name() {
        config.title = format!("{} - Oxide Editor", name.to_string_lossy());
    }
    // List it in the Hub, however it was opened.
    crate::hub::remember_project(&project);
    let project_dir = project.clone();
    let editor = Editor::new().with_scripts(ScriptHost::new(project.clone(), build_id));
    let app = App::with_config(config)
        .shell(Box::new(editor))
        .mcp_tools(crate::mcp::register)
        .setup(move |world| load_initial_scenes(world, &project_dir, scene));
    match app.run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("oxide: {e}");
            ExitCode::FAILURE
        }
    }
}
