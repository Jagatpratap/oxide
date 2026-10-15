//! `oxide` — runs the engine in a window with the MCP server on.
//!
//! ```text
//! oxide [--scene Assets/Scenes/Main.scene] [--demo] [--project DIR]
//!       [--mcp-addr 127.0.0.1:47474] [--no-mcp] [--size 1280x720]
//! ```
//! With no scene it opens Unity's default new scene (Main Camera + Directional Light);
//! `--demo` builds a small test scene.

use oxide_engine::prelude::*;
use oxide_engine::{App, EngineConfig};
use std::process::ExitCode;

const USAGE: &str = "usage: oxide [--scene PATH] [--demo] [--project DIR] [--mcp-addr HOST:PORT] [--no-mcp] [--size WxH]";

struct Args {
    config: EngineConfig,
    scene: Option<String>,
    demo: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        config: EngineConfig::default(),
        scene: None,
        demo: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--mcp-addr" => {
                a.config.mcp_addr = Some(args.next().ok_or("--mcp-addr needs a value")?)
            }
            "--no-mcp" => a.config.mcp_addr = None,
            "--scene" => a.scene = Some(args.next().ok_or("--scene needs a path")?),
            "--project" => {
                a.config.project_root = args.next().ok_or("--project needs a folder")?.into()
            }
            "--demo" => a.demo = true,
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

/// A small scene exercising lighting, shadows and materials.
fn demo_scene(world: &mut World) {
    oxide_engine::objects::create_default_scene(world);
    let cam = world.find("Main Camera").expect("camera");
    world.set_position(cam, Vector3::new(0.0, 4.0, -12.0));
    world.look_at(cam, Vector3::new(0.0, 1.0, 0.0), Vector3::Y);

    let ground = create_primitive(world, PrimitiveType::Plane);
    world.get_mut(ground).unwrap().name = "Ground".into();
    world.transform_mut(ground).unwrap().local_scale = Vector3::new(5.0, 1.0, 5.0);
    let grid = Handle::from_path("builtin:grid");
    let mat = {
        let assets = world.resource_mut::<AssetServer>();
        assets.materials.add(Material {
            name: "Ground".into(),
            base_map: Some(grid),
            tiling: Vector2::new(10.0, 10.0),
            smoothness: 0.2,
            ..Default::default()
        })
    };
    set_material(world, ground, mat);

    let items = [
        (
            PrimitiveType::Cube,
            Vector3::new(-4.5, 0.5, 0.0),
            Color::rgb(0.9, 0.2, 0.15),
            0.0,
            0.4,
        ),
        (
            PrimitiveType::Sphere,
            Vector3::new(-1.5, 0.5, 0.0),
            Color::rgb(1.0, 0.85, 0.4),
            1.0,
            0.85,
        ),
        (
            PrimitiveType::Capsule,
            Vector3::new(1.5, 1.0, 0.0),
            Color::rgb(0.2, 0.5, 0.95),
            0.0,
            0.7,
        ),
        (
            PrimitiveType::Cylinder,
            Vector3::new(4.5, 1.0, 0.0),
            Color::rgb(0.3, 0.8, 0.35),
            0.5,
            0.5,
        ),
    ];
    for (p, pos, color, metallic, smoothness) in items {
        let id = create_primitive(world, p);
        world.transform_mut(id).unwrap().local_position = pos;
        let m = world.resource_mut::<AssetServer>().materials.add(Material {
            name: p.name().into(),
            base_color: color,
            metallic,
            smoothness,
            ..Default::default()
        });
        set_material(world, id, m);
    }
    create_point_light(
        world,
        Vector3::new(0.0, 2.0, -2.5),
        Color::rgb(1.0, 0.5, 0.2),
        8.0,
        4.0,
    );
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::from(2);
        }
    };
    let mut app = App::with_config(args.config);
    if let Some(path) = args.scene {
        app = app.setup(move |world| {
            if let Err(e) = oxide_engine::load_scene(world, &path, false) {
                debug_log_error!("{e}");
                oxide_engine::objects::create_default_scene(world);
            }
        });
    } else if args.demo {
        app = app.setup(demo_scene);
    }
    match app.run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("oxide: {e}");
            ExitCode::FAILURE
        }
    }
}
