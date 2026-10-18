# Oxide

**A native Rust game engine and editor that works the way Unity does.**

Oxide gives you Unity's editor (Hierarchy, Scene and Game views, Inspector, Project window,
Prefabs, Play mode) and its runtime concepts (GameObjects, components, the player loop,
materials, physics, animation, UI) with game code written in Rust instead of C#. Scripts
hot-reload while the editor runs. Every running engine also serves the
[Model Context Protocol](https://modelcontextprotocol.io), so AI agents can inspect and drive
the editor.

[![CI](https://github.com/Jagatpratap/oxide/actions/workflows/ci.yml/badge.svg)](https://github.com/Jagatpratap/oxide/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/Jagatpratap/oxide?include_prereleases)](https://github.com/Jagatpratap/oxide/releases)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)

> **Status: early (0.x).** Oxide is under active development. Most of the editor and runtime
> work, project files and the scripting API may still change between minor versions, and some
> Unity features are partly done or missing ([roadmap](docs/ROADMAP.md)). It is developed and
> tested day to day on macOS (Apple Silicon). Linux and Windows build and pass the automated
> tests in CI but get less hands-on testing. Android and iOS are not supported yet.

## Contents

- [Install](#install)
- [Your first project](#your-first-project)
- [Writing scripts](#writing-scripts)
- [Features](#features)
- [Using Oxide with AI agents (MCP)](#using-oxide-with-ai-agents-mcp)
- [How Oxide compares to Unity](#how-oxide-compares-to-unity)
- [Repository layout](#repository-layout)
- [Building from source](#building-from-source)
- [Documentation](#documentation)
- [Contributing](#contributing)
- [License](#license)

## Install

Oxide projects are Rust code, so you need **Rust** on every machine that opens the editor.

1. **Install Rust** from [rustup.rs](https://rustup.rs) (Rust 1.88 or newer). On Windows, rustup
   also asks for the Visual Studio C++ Build Tools; install them when it does.
2. **Get Oxide**, one of two ways:
   - **Download a release** (recommended): open the
     [Releases page](https://github.com/Jagatpratap/oxide/releases), download the archive for
     your computer and unpack it anywhere.

     | Computer | File |
     |---|---|
     | Mac with Apple Silicon (M1 or later) | `oxide-<version>-aarch64-apple-darwin.tar.gz` |
     | Mac with Intel | `oxide-<version>-x86_64-apple-darwin.tar.gz` |
     | Linux (x86-64) | `oxide-<version>-x86_64-unknown-linux-gnu.tar.gz` |
     | Windows (x86-64) | `oxide-<version>-x86_64-pc-windows-msvc.zip` |

   - **Build from source**: see [Building from source](#building-from-source).
3. **Start the Oxide Hub**: run `bin/oxide` in the unpacked folder (Windows: `bin\oxide.exe`).

The archive contains the engine source next to the prebuilt `oxide` program. Your projects
compile against that source, so keep the folder where it is (or set `OXIDE_ENGINE_ROOT` to its
new location if you move it). `oxide --version` shows which engine folder is in use.

**First launch on macOS:** the downloads aren't notarized yet, so macOS may refuse to open
`oxide` from a browser download. Remove the download flag once:

```sh
xattr -dr com.apple.quarantine ~/Downloads/oxide-*-apple-darwin
```

**Linux** needs a GPU driver with Vulkan support, and the development packages Rust uses to
compile your project (on Debian/Ubuntu:
`sudo apt install build-essential pkg-config libxkbcommon-dev libwayland-dev libx11-dev libasound2-dev libdbus-1-dev libudev-dev`).

The full walkthrough, including troubleshooting, is in
[docs/GETTING_STARTED.md](docs/GETTING_STARTED.md).

## Your first project

1. In the Hub, click **New project**, choose a name and folder, and click **Create**.
2. The first project on a machine compiles the engine, which takes a few minutes. Every
   project after that opens in seconds, because all projects share one build cache.
3. The editor opens on a starter scene. From here it works like Unity:
   - **GameObject** menu → create cubes, lights, cameras, terrain, UI.
   - Drag models (`.glb`, `.gltf`, `.fbx`, `.obj`), textures and audio from Finder or Explorer
     into the **Project** window to import them.
   - Drag prefabs and models into the **Scene** view or **Hierarchy**; drag materials onto
     objects; drag scripts onto objects to add them as components.
   - Press **Play** to run the game in the **Game** view; press it again to stop, and the
     scene goes back to how it was.
   - **File → Build Settings** builds a standalone game for the computer you're on.

You can do the same from a terminal:

```sh
oxide new ~/Games/MyRacer     # create a project and open it
oxide open ~/Games/MyRacer    # open it again later
oxide                         # the Hub
```

A project looks like this:

| Path | What it holds |
|---|---|
| `Assets/` | Scenes (`.scene`), prefabs (`.prefab`), materials (`.mat`), models, textures, audio, every other asset, each with a `.meta` file holding its GUID |
| `Scripts/` | Your game code: a Rust crate of components |
| `Packages/` | `manifest.json` listing the packages the project uses (created when you add one) |
| `ProjectSettings/` | Tags and layers, input, physics, quality, graphics and other settings (created when you change one) |
| `Cargo.toml`, `Cargo.lock` | A small Rust workspace that ties your scripts to the engine; written by Oxide |
| `Library/` | Generated files and editor state; safe to delete |

Put everything except `Library/` under version control.

All projects build into one shared cache: `~/Library/Caches/Oxide/Build` on macOS,
`%LOCALAPPDATA%\Oxide\Build` on Windows and `~/.cache/oxide/build` on Linux. It uses about 3 GB
and is safe to delete; set `OXIDE_BUILD_DIR` to put it somewhere else.

## Writing scripts

A script is a component: a struct that derives `Reflect` (its fields appear in the Inspector
and are saved with the scene) and implements `Component` (Unity's MonoBehaviour messages).

```rust
use oxide::prelude::*;

#[derive(Clone, Default, Reflect)]
pub struct Spinner {
    #[reflect(range(0, 720))]
    pub speed: f32,
}

impl Component for Spinner {
    fn update(&mut self, ctx: &mut Ctx) {
        let a = self.speed * ctx.delta_time();
        ctx.transform().rotate(Vector3::new(0.0, a, 0.0));
    }
}
```

Create scripts from **Project → + → Rust Script**; the editor writes the file and registers it
in `Scripts/src/lib.rs`:

```rust
oxide::scripts! {
    Spinner => "Scripts/Spinner";
}
```

Save a script and the editor recompiles it in the background and reloads it, in Edit or Play
mode, keeping every component's field values. Compile errors appear in the Console with file
and line, and the last working version keeps running.

A prefab reference works like Unity's `public GameObject prefab;`: drag a `.prefab` from the
Project window onto the field in the Inspector.

```rust
#[derive(Clone, Default, Reflect)]
pub struct Gun {
    pub bullet: Option<Prefab>,
    pub fire_rate: f32,
}

impl Component for Gun {
    fn update(&mut self, ctx: &mut Ctx) {
        if ctx.input().get_button_down("Fire1")
            && let Some(bullet) = &self.bullet
        {
            ctx.instantiate_prefab(bullet);
        }
    }
}
```

Scripts can also add editor tools (custom windows, menu items, property drawers, Scene view
overlays and tools), data assets (ScriptableObjects), custom Inspectors, Timeline playables,
custom render pipelines and play-mode tests. See [docs/GETTING_STARTED.md](docs/GETTING_STARTED.md#scripting).

## Features

Each item links to Unity's own concept where it follows it. The [roadmap](docs/ROADMAP.md)
tracks what is done, partly done and missing in detail.

**Editor**
- Oxide Hub, dockable windows with saved layouts, Unity's menus and shortcuts.
- Scene view with Hand / Move / Rotate / Scale / Rect / Transform tools, snapping, overlays,
  picking, visibility, draw modes and 2D mode.
- Inspector with multi-object editing, presets, custom editors, import settings and previews.
- Drag and drop between all windows, following Unity's rules.
- Undo/Redo, Play mode, multi-scene editing, Prefabs (nested, variants, overrides).
- AssetDatabase with GUIDs, file watching, asset packages and a Package Manager.
- Profiler, Frame Debugger, Project Auditor, Test Runner, Console with log files.

**Graphics**
- wgpu renderer (Metal, Vulkan, Direct3D 12) with Forward+ and Deferred paths on a render graph.
- Lit, Simple Lit, Baked Lit and Unlit shaders, shader keywords and variants.
- Cascaded shadows, SSAO, decals, TAA / FXAA / SMAA, HDR post-processing with volumes.
- Skyboxes, reflection probes, a lightmapper with shadowmask modes, light probes, area lights,
  light cookies.
- Camera stacking, physical cameras, render queues, custom render pipelines.
- Shader Graph and VFX Graph.

**Gameplay**
- Physics (Rapier): rigidbodies, colliders, joints, raycasts, character and wheel colliders.
- Animation: Animator state machines, blend trees, humanoid retargeting, Timeline, constraints.
- Audio: 3D sound, mixers with effects, sends and ducking, random containers.
- UI: Canvas UI with layout groups, UI Toolkit (UXML and USS), TextMeshPro-style text,
  Localization, screen reader support.
- Terrain with painting, trees, wind, details and streaming; particle systems; NavMesh; video.
- ProBuilder modeling inside the editor.
- Multiplayer basics: NetworkManager, NetworkObject, NetworkTransform and RPCs.

**Scripting**
- Rust components with hot reload, coroutines, a job system, timers, web requests, native
  plugins and play-mode tests.
- An editor extension API and a package system for sharing tools.

## Using Oxide with AI agents (MCP)

Every running editor or game serves MCP at `http://127.0.0.1:47474/mcp`. Agents can read the
Console, list and edit objects and components, create assets, take screenshots, press Play and
click through the editor UI. With Claude Code:

```sh
claude mcp add --transport http oxide http://127.0.0.1:47474/mcp
```

Use `--mcp-addr HOST:PORT` to change the address or `--no-mcp` to turn it off. For clients that
only speak stdio, use the bundled `oxide-mcp-stdio` bridge. Every tool is listed in
[docs/MCP.md](docs/MCP.md).

## How Oxide compares to Unity

| | Unity | Oxide |
|---|---|---|
| Game code | C# | Rust (hot-reloaded) |
| Editor | Closed source | Open source, in Rust, extendable with Rust |
| Licence | Proprietary, per-seat plans | MIT or Apache-2.0, free |
| Platforms | Desktop, mobile, consoles, web, XR | Desktop (macOS, Windows, Linux) for now |
| Asset Store | Large marketplace | Packages from git, folders or npm-style registries; BlenderKit built in |
| Maturity | Over 15 years of production use | Early; expect gaps and breaking changes |

Oxide copies Unity's concepts and layout, not its name, logos, icons or assets. It is not
affiliated with or endorsed by Unity Technologies.

## Repository layout

| Path | What |
|---|---|
| `apps/oxide` | The `oxide` program: Hub, project launcher, editor sandbox and player |
| `apps/oxide_mcp_stdio` | The `oxide-mcp-stdio` bridge for stdio-only MCP clients |
| `crates/oxide_math` | Unity math types (`Vector3`, `Quaternion`, `Mathf`, `Bounds`, ...) on glam |
| `crates/oxide_core` | Time, the Console log, reflection |
| `crates/oxide_derive` | `#[derive(Reflect)]` |
| `crates/oxide_assets` | Asset server, importers (glTF, FBX, OBJ, PSD, ...), materials, AssetDatabase, packages |
| `crates/oxide_scene` | GameObjects, components and their lifecycle, scenes, prefabs, coroutines |
| `crates/oxide_render` | The wgpu renderer |
| `crates/oxide_engine` | Window, player loop, scene extraction, built-in objects, MCP tools |
| `crates/oxide_editor` | The editor and Hub |
| `crates/oxide_physics`, `oxide_audio`, `oxide_animation`, `oxide_timeline` | Physics, audio, animation, Timeline |
| `crates/oxide_ui`, `oxide_uitk`, `oxide_text`, `oxide_localization` | Canvas UI, UI Toolkit, text, Localization |
| `crates/oxide_shadergraph`, `oxide_vfx`, `oxide_particles`, `oxide_lightmapper` | Shader Graph, VFX Graph, particles, lightmapper |
| `crates/oxide_terrain`, `oxide_navigation`, `oxide_probuilder`, `oxide_video` | Terrain, NavMesh, ProBuilder, video |
| `crates/oxide_input`, `oxide_network`, `oxide_standard` | Input, multiplayer, standard controllers |
| `crates/oxide_dylib` | Engine and editor as one shared library, which project scripts link |
| `crates/oxide_player` | What a built game links (no editor) |
| `packages/` | Built-in packages (BlenderKit) |
| `docs/` | Roadmap, MCP reference, getting-started guide, Unity feature checklist |
| `scripts/` | Development helpers (MCP client, release packaging) |

## Building from source

```sh
git clone https://github.com/Jagatpratap/oxide.git
cd oxide
cargo run -p oxide                         # the Hub
cargo run -p oxide -- new ~/Games/MyGame   # create a project with this engine checkout
cargo run -p oxide -- --sample vehicle     # the editor on a built-in sample
cargo run -p oxide -- --player --sample vehicle   # the sample as a game, without the editor
cargo test --workspace
```

On Linux, install the development headers first:

```sh
sudo apt install pkg-config libxkbcommon-dev libwayland-dev libx11-dev libasound2-dev libdbus-1-dev libudev-dev
```

Built-in samples: `demo`, `vehicle`, `character`, `open_world`, `city`. Other options:
`--scene PATH`, `--project DIR`, `--size 1600x900`, `--mcp-addr HOST:PORT`, `--no-mcp`.

## Documentation

- [Getting started](docs/GETTING_STARTED.md): install, the editor, scripting, building games,
  troubleshooting.
- [Roadmap](docs/ROADMAP.md): every feature area and its status.
- [MCP reference](docs/MCP.md): connecting agents and every tool.
- [Unity feature checklist](docs/UNITY_FEATURE_CHECKLIST.md): Unity's manual, page by page.
- [Changelog](CHANGELOG.md).
- [BlenderKit package](packages/com.oxide.blenderkit/README.md), also an example of writing a
  package with editor windows.

## Contributing

Bug reports, ideas and pull requests are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) for
how to set up, the code style, testing and how releases are made. To report a security problem
privately, see [SECURITY.md](SECURITY.md).

## License

Oxide is licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option. Unless you state otherwise, any contribution you submit for inclusion in Oxide
is licensed as above, without any additional terms or conditions.
