//! `oxide` — the Oxide editor, with the MCP server on.
//!
//! ```text
//! oxide [--scene Assets/Scenes/Main.scene] [--sample demo|vehicle|character|open_world] [--project DIR]
//!       [--player] [--mcp-addr 127.0.0.1:47474] [--no-mcp] [--size 1280x720]
//! ```
//! With no scene it opens Unity's default new scene (Main Camera + Directional Light);
//! `--sample` opens a built-in test scene. `--player` runs the game alone in the window, as a
//! built game would, instead of inside the editor.

mod samples;

use oxide_engine::prelude::*;
use oxide_engine::{App, EngineConfig};
use std::process::ExitCode;

const USAGE: &str = "usage: oxide [--scene PATH] [--sample demo|vehicle|character|open_world] [--project DIR] [--player] [--mcp-addr HOST:PORT] [--no-mcp] [--size WxH]";

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

fn main() -> ExitCode {
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

    if let Some(path) = args.scene {
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
