//! `oxide` — the Oxide editor, with the MCP server on.
//!
//! ```text
//! oxide                            the Oxide Hub (Unity Hub): open, add or create projects
//! oxide hub [--mcp-addr HOST:PORT] the Hub, optionally with MCP (for automation)
//! oxide new  <DIR> [editor args]   create a game project and open it
//! oxide open <DIR> [editor args]   open a game project (its Scripts hot-reload)
//! oxide [--scene Assets/Scenes/Main.scene] [--sample demo|vehicle|character|open_world|city] [--project DIR]
//!       [--player] [--mcp-addr 127.0.0.1:47474] [--no-mcp] [--size 1280x720]
//! ```
//! `new`/`open` are the project launcher (Unity Hub): they build the project's own editor,
//! which links the engine as a shared library so the project's Rust scripts can be compiled
//! and reloaded while it runs. Without them, `oxide` is the engine's sandbox editor (no
//! project scripts).
//! With no scene it opens Unity's default new scene (Main Camera + Directional Light);
//! `--sample` opens a built-in test scene. `--player` runs the game alone in the window, as a
//! built game would, instead of inside the editor.

mod samples;

use oxide_engine::prelude::*;
use oxide_engine::{App, EngineConfig};
use std::process::ExitCode;

const USAGE: &str = "usage: oxide [--scene PATH] [--sample demo|vehicle|character|open_world|city] [--project DIR] [--player] [--mcp-addr HOST:PORT] [--no-mcp] [--size WxH]";

struct Args {
    config: EngineConfig,
    scene: Option<String>,
    sample: Option<String>,
    player: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        config: EngineConfig::default(),
        scene: None,
        sample: None,
        player: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--mcp-addr" => {
                a.config.mcp_addr = Some(args.next().ok_or("--mcp-addr needs a value")?)
            }
            "--no-mcp" => a.config.mcp_addr = None,
            "--player" => a.player = true,
            "--scene" => a.scene = Some(args.next().ok_or("--scene needs a path")?),
            "--project" => {
                a.config.project_root = args.next().ok_or("--project needs a folder")?.into()
            }
            "--demo" => a.sample = Some("demo".into()),
            "--sample" => {
                a.sample = Some(
                    args.next()
                        .ok_or("--sample needs a name: demo, vehicle, character")?,
                )
            }
            "--size" => {
                let v = args.next().ok_or("--size needs a value like 1280x720")?;
                let (w, h) = v.split_once('x').ok_or("--size must look like 1280x720")?;
                a.config.width = w.parse().map_err(|_| "bad width")?;
                a.config.height = h.parse().map_err(|_| "bad height")?;
            }
            "-h" | "--help" => return Err(USAGE.into()),
            other => return Err(format!("unknown argument `{other}`\n{USAGE}")),
        }
    }
    Ok(a)
}

/// The engine source tree this launcher was built from.
fn engine_root() -> std::path::PathBuf {
    let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = here.join("../..");
    std::fs::canonicalize(&root).unwrap_or(root)
}

/// `oxide new|open DIR [args]`: prepares the project and runs its editor through cargo.
fn launch_project(create: bool, dir: &str, rest: Vec<String>) -> ExitCode {
    let dir = std::path::PathBuf::from(dir);
    let engine = engine_root();
    let prepared = if create {
        oxide_editor::project_files::create_project(&dir, &engine)
    } else if !dir.join("Scripts/Cargo.toml").is_file() && !dir.join("Assets").is_dir() {
        Err(format!(
            "{} is not an Oxide project (create one with `oxide new {}`)",
            dir.display(),
            dir.display()
        ))
    } else {
        oxide_editor::project_files::write_build_files(&dir, &engine)
    };
    if let Err(e) = prepared {
        eprintln!("oxide: {e}");
        return ExitCode::FAILURE;
    }
    let dir = std::fs::canonicalize(&dir).unwrap_or(dir);
    if create {
        println!("Created project {}", dir.display());
    }
    if !oxide_editor::project_files::editor_exe(&dir).is_file() {
        println!(
            "Building the project's editor (the first build on this machine compiles the engine and takes a few minutes)..."
        );
    }
    let status = std::process::Command::new(oxide_editor::project_files::cargo_command())
        .args(["run", "-q", "-p", "oxide-project-editor", "--"])
        .arg("--project")
        .arg(&dir)
        .args(rest)
        .current_dir(&dir)
        .env_remove("CARGO_TARGET_DIR")
        .status();
    match status {
        Ok(s) if s.success() => ExitCode::SUCCESS,
        Ok(s) => ExitCode::from(s.code().unwrap_or(1).clamp(1, 255) as u8),
        Err(e) => {
            eprintln!("oxide: could not run cargo: {e}");
            ExitCode::FAILURE
        }
    }
}

/// The Oxide Hub window (what `oxide` opens with no arguments).
fn run_hub(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut config = EngineConfig {
        title: "Oxide Hub".into(),
        width: 1100,
        height: 680,
        mcp_addr: None,
        ..Default::default()
    };
    while let Some(a) = args.next() {
        match a.as_str() {
            "--mcp-addr" => config.mcp_addr = args.next(),
            other => {
                eprintln!("unknown argument `{other}`\nusage: oxide hub [--mcp-addr HOST:PORT]");
                return ExitCode::from(2);
            }
        }
    }
    let app = App::with_config(config)
        .shell(Box::new(oxide_editor::hub::Hub::new(engine_root())))
        .mcp_tools(oxide_editor::mcp::register_window_tools);
    match app.run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("oxide: {e}");
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    if std::env::args().len() == 1 {
        return run_hub(std::iter::empty());
    }
    let mut raw = std::env::args().skip(1);
    if raw.next().as_deref() == Some("hub") {
        return run_hub(raw);
    }
    let mut raw = std::env::args().skip(1);
    if let Some(cmd @ ("new" | "open")) = raw.next().as_deref() {
        let Some(dir) = raw.next() else {
            eprintln!("usage: oxide {cmd} <DIR> [editor args]");
            return ExitCode::from(2);
        };
        return launch_project(cmd == "new", &dir, raw.collect());
    }
    let args = match parse_args() {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::from(2);
        }
    };
    let mut config = args.config;
    if !args.player && config.width == EngineConfig::default().width {
        // The editor needs room for its windows around the views.
        (config.width, config.height) = (1600, 960);
    }
    let mut app = App::with_config(config).register::<samples::CarHud>("Samples/Car HUD");
    if !args.player {
        app = app
            .shell(Box::new(oxide_editor::Editor::new()))
            .mcp_tools(oxide_editor::mcp::register);
    }

    if args.sample.is_none() && !args.player {
        // The editor reopens the last scene setup, like a project editor.
        let project = app.config.project_root.clone();
        let scene = args.scene;
        app = app
            .setup(move |world| oxide_editor::launch::load_initial_scenes(world, &project, scene));
    } else if let Some(path) = args.scene {
        app = app.setup(move |world| {
            if let Err(e) = oxide_engine::load_scene(world, &path, false) {
                debug_log_error!("{e}");
                oxide_engine::objects::create_default_scene(world);
            }
        });
    } else if let Some(name) = args.sample {
        let f: fn(&mut World) = match name.as_str() {
            "demo" => samples::demo,
            "vehicle" => samples::vehicle,
            "character" => samples::character,
            "open_world" => samples::open_world,
            "city" => samples::city,

            other => {
                eprintln!("unknown sample `{other}` (demo, vehicle, character)");
                return ExitCode::from(2);
            }
        };
        app = app.setup(f);
    }
    match app.run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("oxide: {e}");
            ExitCode::FAILURE
        }
    }
}
