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

pub fn remember_scene(project: &Path, scene: &str) {
    let f = last_scene_file(project);
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(f, scene);
}

fn initial_scene(project: &Path, arg: Option<String>) -> Option<String> {
    arg.or_else(|| {
        std::fs::read_to_string(last_scene_file(project))
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| project.join(s).is_file())
    })
    .or_else(|| {
        let main = "Assets/Scenes/Main.scene";
        project.join(main).is_file().then(|| main.to_owned())
    })
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
    let scene = initial_scene(&project, scene);
    let editor = Editor::new().with_scripts(ScriptHost::new(project.clone(), build_id));
    let app = App::with_config(config)
        .shell(Box::new(editor))
        .mcp_tools(crate::mcp::register)
        .setup(move |world| match scene {
            Some(path) => {
                if let Err(e) = oxide_engine::load_scene(world, &path, false) {
                    debug_log_error!("{e}");
                    objects::create_default_scene(world);
                }
            }
            None => objects::create_default_scene(world),
        });
    match app.run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("oxide: {e}");
            ExitCode::FAILURE
        }
    }
}
