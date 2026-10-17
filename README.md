# Oxide

A native Rust game engine and editor modeled on Unity's features and workflow, with a built-in
MCP server so AI agents can drive it.

- **Roadmap:** [`docs/ROADMAP.md`](docs/ROADMAP.md). Done: P0 foundation, P1 scenes/prefabs,
  P2 editor; runtime systems from P4–P8 (rendering, physics, vehicles, audio, animation, UI,
  terrain, particles) are in and being filled out. Game scripting with hot reload (P6) works.
- **Unity feature checklist (3,604 manual pages):** [`docs/UNITY_FEATURE_CHECKLIST.md`](docs/UNITY_FEATURE_CHECKLIST.md)
- **MCP:** [`docs/MCP.md`](docs/MCP.md)

## Make a game

```sh
cargo run -p oxide                            # the Oxide Hub: your projects, New project, Add from disk
cargo run -p oxide -- new ~/Games/MyRacer     # or from the command line: create a project and open it
cargo run -p oxide -- open ~/Games/MyRacer    # open it again later
```

Every project builds into one shared folder (`~/Library/Caches/Oxide/Build` on macOS,
`$OXIDE_BUILD_DIR` to change it), so the engine is compiled once per machine: the very first
project takes a few minutes, and every project after that opens in seconds and only compiles its
own scripts. The folder is a cache (about 3 GB) and safe to delete. A project looks like Unity's:

| Path | What |
|---|---|
| `Assets/` | Scenes (`.scene`), prefabs (`.prefab`), materials (`.mat`), models (`.glb`), textures, audio |
| `Scripts/` | Your game code: a Rust crate of components (`#[derive(Reflect)]` + `impl Component`) |
| `Library/` | Generated build files and editor state; safe to delete |

Save a script and the editor recompiles it in the background and reloads it, in Edit or Play
mode, keeping every component's field values. Compile errors go to the Console with file and
line; the last good scripts keep running. Create scripts from Project → + → Rust Script.

```rust
use oxide::prelude::*;

#[derive(Clone, Default, Reflect)]
pub struct Spinner {
    pub speed: f32,
}

impl Component for Spinner {
    fn update(&mut self, ctx: &mut Ctx) {
        let a = self.speed * ctx.delta_time();
        ctx.transform().rotate(Vector3::new(0.0, a, 0.0));
    }
}
```

## Engine sandbox

```sh
cargo run -p oxide -- --project .          # the editor on a default scene (no arguments opens the Hub)
cargo run -p oxide -- --sample vehicle     # built-in samples: demo, vehicle, character, open_world
cargo run -p oxide -- --player --sample vehicle   # the game alone, without the editor
cargo test --workspace
```

Options: `--scene PATH`, `--project DIR`, `--size 1600x900`, `--mcp-addr 127.0.0.1:5000`,
`--no-mcp`. MCP is on at `http://127.0.0.1:47474/mcp`.

## Layout

| Path | What |
|---|---|
| `crates/oxide_math` | Unity math: `Vector3`, `Quaternion`, `Matrix4x4`, `Mathf`, `Bounds`, `Ray`, `Plane`, `Rect`, `Color` (glam-backed; left-handed, Y-up, degrees) |
| `crates/oxide_core` | `Time`, `Debug.Log` Console, reflection |
| `crates/oxide_derive` | `#[derive(Reflect)]` |
| `crates/oxide_assets` | Asset server: meshes, primitives, textures, materials (`.mat`), glTF import, animation clips, terrain data |
| `crates/oxide_scene` | GameObjects, components and their lifecycle, scenes, prefabs (overrides, variants) |
| `crates/oxide_render` | wgpu renderer: PBR, cascaded shadows, sky, fog, bloom/tonemapping, skinning, terrain, particles |
| `crates/oxide_physics` | Rapier: Rigidbody, colliders, joints, raycasts, CharacterController, WheelCollider |
| `crates/oxide_audio` | kira: AudioSource/AudioListener, 3D sound |
| `crates/oxide_animation` | Animator state machines, blend trees, legacy Animation |
| `crates/oxide_ui` | Canvas UI and immediate-mode GUI (egui) |
| `crates/oxide_terrain`, `crates/oxide_particles` | Terrain and Particle System |
| `crates/oxide_standard` | Standard assets: car controller, character controllers, cameras |
| `crates/oxide_input` | Keyboard, mouse, gamepads, Unity's input axes |
| `crates/oxide_mcp` | MCP protocol + localhost HTTP transport |
| `crates/oxide_engine` | Window, player loop, scene extraction, MCP tools |
| `crates/oxide_editor` | The editor, script hot reload, project templates |
| `crates/oxide_dylib` | Engine + editor as one shared library (what project Scripts link) |
| `apps/oxide` | The `oxide` binary (the Oxide Hub, project launcher and editor sandbox) |
| `apps/oxide_mcp_stdio` | `oxide-mcp-stdio` bridge for stdio-only MCP clients |

Oxide copies Unity's concepts and layout, not its name, logos, icons or assets.
