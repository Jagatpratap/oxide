# Getting started with Oxide

This guide takes you from nothing installed to a running game, and covers the parts of Oxide
that differ from Unity. If you know Unity, most of the editor will feel familiar: the windows,
menus, shortcuts and concepts use Unity's names.

- [1. Install](#1-install)
- [2. The Oxide Hub](#2-the-oxide-hub)
- [3. The editor](#3-the-editor)
- [4. Assets](#4-assets)
- [5. Scripting](#scripting)
- [6. Packages and editor extensions](#6-packages-and-editor-extensions)
- [7. Building a game](#7-building-a-game)
- [8. AI agents (MCP)](#8-ai-agents-mcp)
- [9. Updating Oxide](#9-updating-oxide)
- [10. Troubleshooting](#10-troubleshooting)

## 1. Install

### Requirements

| | Minimum |
|---|---|
| OS | macOS 12+, Windows 10/11 (64-bit), or a recent 64-bit Linux with X11 or Wayland |
| GPU | Metal (any Mac from 2016 on), Direct3D 12 or Vulkan 1.1 |
| Rust | 1.88 or newer, from [rustup.rs](https://rustup.rs) |
| Disk | About 5 GB: 3 GB for the shared build cache plus your projects |
| Memory | 8 GB; 16 GB is more comfortable while the engine compiles |

**Why Rust is required:** your game code is a Rust crate that the editor compiles and
hot-reloads. The editor itself is compiled together with your scripts, so that both use exactly
the same compiler and engine build (Rust has no portable format like C#'s, so they can't be
mixed). That happens automatically; you only need Rust installed.

### Install Rust

- **macOS / Linux:** run `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh` and
  accept the defaults. On macOS, install Xcode's command line tools when asked
  (`xcode-select --install`).
- **Windows:** download and run `rustup-init.exe` from [rustup.rs](https://rustup.rs). It asks
  to install the Visual Studio C++ Build Tools; say yes.
- **Linux:** also install the development packages Oxide compiles against. On Debian/Ubuntu:

  ```sh
  sudo apt install build-essential pkg-config libxkbcommon-dev libwayland-dev libx11-dev \
    libasound2-dev libdbus-1-dev libudev-dev
  ```

  On Fedora: `sudo dnf install gcc pkgconf libxkbcommon-devel wayland-devel libX11-devel
  alsa-lib-devel dbus-devel systemd-devel`.

Check it worked with `cargo --version`.

### Install Oxide from a release

1. Download the archive for your computer from the
   [Releases page](https://github.com/Jagatpratap/oxide/releases):
   `aarch64-apple-darwin` (Apple Silicon Mac), `x86_64-apple-darwin` (Intel Mac),
   `x86_64-unknown-linux-gnu` (Linux) or `x86_64-pc-windows-msvc` (Windows).
2. Optionally check the download against its `.sha256` file:
   `shasum -a 256 -c oxide-*.tar.gz.sha256` (Windows: `certutil -hashfile <file> SHA256`).
3. Unpack it somewhere permanent, for example `~/Applications/Oxide` or `C:\Oxide`.
4. **macOS only:** clear the download flag so macOS lets the program run:
   `xattr -dr com.apple.quarantine <the unpacked folder>`.
5. Run `bin/oxide` (Windows: `bin\oxide.exe`). Optionally add the `bin` folder to your `PATH`
   so `oxide` works in any terminal.

The unpacked folder is the engine: it holds the engine source next to the `oxide` program, and
projects compile against it. Run `oxide --version` to see which folder is in use. If you move
the folder, projects follow it the next time you open them from the moved `oxide`. To point
`oxide` at a different engine folder, set `OXIDE_ENGINE_ROOT`.

### Install Oxide from source

```sh
git clone https://github.com/Jagatpratap/oxide.git
cd oxide
cargo run --release -p oxide     # opens the Hub
```

Projects you create this way compile against your checkout, so engine changes you make show up
in your projects the next time they open.

## 2. The Oxide Hub

Running `oxide` with no arguments opens the Hub.

- **Projects:** every project you've opened. Click one to open it. Its ⋮ menu has Show in
  Finder and Remove from list.
- **New project:** choose a name, a location and a template, then **Create**.
- **Add:** open an existing project folder from disk.
- **Installs:** which engine folder is in use, and the shared build cache with its size.

The first project you open on a machine compiles the engine into the shared build cache, which
takes a few minutes; the Hub shows the progress. After that, every project opens in seconds and
only compiles its own scripts. Two projects can't compile at the same moment; the second one
waits and says so.

From a terminal:

```sh
oxide new ~/Games/MyGame      # create and open
oxide open ~/Games/MyGame     # open
oxide hub                     # the Hub
oxide --version               # version and engine folder
```

## 3. The editor

The default layout is Unity's: **Hierarchy** on the left, **Scene** and **Game** views in the
middle, **Inspector** on the right, **Project** and **Console** at the bottom. Drag a tab to
rearrange windows; the layout is saved per project. **Window → Layouts → Default** resets it.
More windows are under **Window** (Animation, Animator, Timeline, Lighting, Shader Graph,
Profiler, Package Manager and others).

### Scene view

| Action | How |
|---|---|
| Orbit | Alt + left drag |
| Pan | Middle drag, or left drag with the Hand tool (Q) |
| Zoom | Scroll |
| Fly | Hold right mouse button, then W/A/S/D/Q/E |
| Frame the selection | F |
| Tools | Q Hand, W Move, E Rotate, R Scale, T Rect, Y Transform |
| Pivot / Center, Global / Local | Z, X, or the toolbar buttons |

The Scene view's panels (tools, view options, the orientation gizmo) are overlays: drag them
by their grip, dock them to an edge, or show and hide them from the `` ` `` key menu.

### Drag and drop

Drag and drop follows Unity's rules. A label next to the cursor names what you're dragging; a
blue outline shows where it can go, and a "not allowed" cursor shows where it can't. Press Esc
to cancel.

| Drag | Onto | Result |
|---|---|---|
| Prefab, model or audio clip | Scene view | A preview follows the surface under the mouse; release to place it |
| Prefab, model or audio clip | Hierarchy row | Becomes a child of that object (middle of the row) or a sibling (top/bottom edge) |
| Material | An object in the Scene view | Previews while hovering; release to assign it to the part under the mouse |
| Material | Hierarchy row, or empty Inspector space | Assigned to that object's renderer |
| Script | An object, a Hierarchy row, or empty Inspector space | Adds its component |
| Scene | Hierarchy | Opens additively |
| Any asset | A matching Inspector field | Assigns it (wrong types are refused) |
| Assets or objects | A list's header in the Inspector | Appended to the list |
| Hierarchy object | Project window | Creates a prefab (or, onto an existing prefab, offers to replace it) |
| Asset | A folder in the Project window | Moves it (references keep working) |
| Files from Finder/Explorer | Project window | Copied into the open folder and imported |

### Play mode

Press **Play** (Cmd/Ctrl+P) to run the scene in the Game view. Changes you make while playing
are undone when you stop, as in Unity, and the toolbar tints to remind you. Pause with
Cmd/Ctrl+Shift+P and step one frame with Cmd/Ctrl+Alt+P.

### Undo and saving

Every edit can be undone with Cmd/Ctrl+Z and redone with Cmd/Ctrl+Shift+Z. Save the scene with Cmd/Ctrl+S. A `*` after a scene's name means it has
unsaved changes.

## 4. Assets

Everything in `Assets/` is an asset. Each gets a `.meta` file with a GUID the first time Oxide
sees it; scenes and prefabs refer to assets by GUID, so you can move and rename files (in the
editor or outside it) without breaking references. Keep `.meta` files in version control.

| Kind | Formats |
|---|---|
| Models | glTF / GLB, FBX, OBJ (with skinning and animation where the format has them) |
| Textures | PNG, JPEG, TGA, BMP, PSD, HDR, EXR, WebP, KTX2 |
| Audio | WAV, OGG, MP3, FLAC |
| Video | MP4 / MOV (H.264), GIF |
| Fonts | TTF, OTF |
| Oxide assets | `.scene`, `.prefab`, `.mat`, `.controller`, `.anim`, `.playable`, `.shadergraph`, `.vfx`, `.uxml`, `.uss` and more |

Select an asset to see its import settings in the Inspector (texture compression and max size
per platform, model scale and normals, audio load type), then click **Apply**. Changed files
re-import automatically.

**Assets → Export Package** and **Import Package** move assets between projects with their
GUIDs, like Unity's `.unitypackage`.

## Scripting

### A component

```rust
use oxide::prelude::*;

#[derive(Clone, Default, Reflect)]
pub struct Mover {
    /// Metres per second (shown as a tooltip in the Inspector).
    pub speed: f32,
    pub target: Option<GameObjectId>,
}

impl Component for Mover {
    fn start(&mut self, ctx: &mut Ctx) {
        debug_log!("Mover started on {}", ctx.name());
    }

    fn update(&mut self, ctx: &mut Ctx) {
        let input = ctx.input();
        let dir = Vector3::new(input.get_axis("Horizontal"), 0.0, input.get_axis("Vertical"));
        let step = dir * self.speed * ctx.delta_time();
        ctx.transform().translate(step);
    }
}
```

- Public fields of supported types appear in the Inspector and are saved with the scene:
  numbers, `bool`, `String`, vectors, colours, enums, `Option<T>`, `Vec<T>`, nested structs,
  asset handles (`Handle<Material>`, `Handle<AudioClip>`, ...), `GameObjectId` (a scene object)
  and `Prefab` (a prefab asset).
- Field attributes work like Unity's: `#[reflect(range(0, 10))]`, `#[reflect(tooltip = "...")]`,
  `#[reflect(header = "Movement")]`, `#[reflect(skip)]` (HideInInspector) and others.
- `Component` methods are Unity's messages: `awake`, `on_enable`, `start`, `fixed_update`,
  `update`, `late_update`, `on_disable`, `on_destroy`, `on_collision_enter`, `on_trigger_enter`
  and the rest.

### Registering scripts

`Scripts/src/lib.rs` lists your components. Creating a script from **Project → + → Rust Script**
adds the line for you:

```rust
mod mover;
pub use mover::Mover;

oxide::scripts! {
    Mover => "Scripts/Mover";
}
```

The same macro takes optional sections for other kinds of code:

| Section | For |
|---|---|
| `editors: T, ...` | Custom Inspectors (`CustomEditor`) |
| `assets: T => ("ext", "Create/Menu"), ...` | Data assets (Unity's ScriptableObjects) |
| `behaviours: T, ...` | Animator state machine behaviours |
| `playables: T, ...` | Timeline playable behaviours |
| `pipelines: T, ...` | Custom render pipelines |
| `tests: async_fn, ...` | Play-mode tests for the Test Runner |
| `extensions: register_fn, ...` | Editor windows, menu items, drawers, overlays and tools |

### Hot reload

Save any file in `Scripts/` and the editor recompiles in the background (the status bar shows
progress) and swaps in the new code, keeping every component's field values. This works in Play
mode too. If the code doesn't compile, the errors appear in the Console (click one to open the
file at that line) and the previous version keeps running.

Open the scripts in any Rust editor; VS Code with rust-analyzer or RustRover work well. Point
it at the project folder, which is a Cargo workspace.

### Useful APIs

| Unity | Oxide |
|---|---|
| `Instantiate(prefab)` | `ctx.instantiate_prefab(&self.prefab)` |
| `Destroy(obj, t)` | `ctx.destroy(id)`, `ctx.destroy_after(id, t)` |
| `GetComponent<T>()` | `ctx.get_component::<T>()`, `ctx.world.get_component::<T>(id)` |
| `StartCoroutine` | `ctx.start_coroutine(\|co\| async move { co.wait_for_seconds(1.0).await; })` |
| `Invoke` / `InvokeRepeating` | `ctx.invoke(...)`, `ctx.invoke_repeating(...)` |
| `Physics.Raycast` | `ctx.physics().raycast(...)` |
| `Input.GetAxis` / `GetButtonDown` | `ctx.input().get_axis(...)`, `get_button_down(...)` |
| `SceneManager.LoadScene` | `ctx.load_scene("Level2")` |
| `Debug.Log` | `debug_log!(...)`, `debug_log_warning!(...)`, `debug_log_error!(...)` |
| `PlayerPrefs` | `PlayerPrefs::set_int(...)` and friends |
| `UnityWebRequest` | `UnityWebRequest::get(url)` |

### Tests

**Window → General → Test Runner** runs Edit-mode tests (ordinary `#[test]` functions in the
Scripts crate) and Play-mode tests (`async fn` listed under `tests:`, run across frames in Play
mode).

## 6. Packages and editor extensions

**Window → Package Manager** adds packages to a project from a git URL, a folder on disk, an
npm-style registry, or the built-in list (for example **BlenderKit**, a browser for free and
paid 3D models, materials and HDRIs). Packages are listed in `Packages/manifest.json` and pinned
in `Packages/packages-lock.json`.

A package is a folder with a `package.json`. It can hold assets, a Rust crate of components,
and editor extensions that load automatically. The
[BlenderKit package README](../packages/com.oxide.blenderkit/README.md) explains the layout and
is a complete working example.

## 7. Building a game

1. **File → Build Settings**: add your scenes (the first one loads at start), set the product
   name and output folder (`Builds/<name>` by default).
2. Click **Build**. Oxide compiles a release build of your scripts with the engine, without the
   editor, and copies the assets it needs.
3. The result is a `.app` on macOS, or a program plus a `_Data` folder on Windows and Linux.

Builds target the platform you're on. Building for another desktop OS means building on that
OS. Mobile, web and console targets aren't available yet.

## 8. AI agents (MCP)

Every running editor serves MCP at `http://127.0.0.1:47474/mcp`. An agent connected to it can
read the Console, inspect and edit objects, create materials and prefabs, take screenshots of
the editor and the game, enter Play mode and click through the editor UI.

```sh
claude mcp add --transport http oxide http://127.0.0.1:47474/mcp   # Claude Code
```

Run several editors at once with `--mcp-addr 127.0.0.1:47475` (and so on). For stdio-only
clients, point them at `bin/oxide-mcp-stdio`. See [MCP.md](MCP.md) for every tool.

## 9. Updating Oxide

- **Release install:** unpack the new version next to the old one and open your projects from
  the new `bin/oxide`. The first project you open compiles the new engine (a few minutes), then
  the others open quickly again. Delete the old folder when you're happy. Read the
  [changelog](../CHANGELOG.md) for breaking changes first.
- **Source install:** `git pull`, then reopen your projects.

## 10. Troubleshooting

| Problem | What to do |
|---|---|
| "Rust is not installed (cargo was not found)" | Install Rust from [rustup.rs](https://rustup.rs), then open a new terminal (or log out and in) so `cargo` is on your `PATH`. |
| macOS says `oxide` "can't be opened" | Run `xattr -dr com.apple.quarantine` on the unpacked folder. |
| The first open takes several minutes | Expected once per machine and engine version: the engine is compiling. Later projects open in seconds. |
| Disk is filling up | Delete the build cache (Hub → Installs shows where; it's safe), or move it with `OXIDE_BUILD_DIR`. |
| Linux build fails on `alsa-sys`, `libudev-sys` or `libdbus-sys` | Install the development packages listed in [Install Rust](#install-rust). |
| Scripts don't reload | Look at the Console for compile errors. A component type that isn't listed in `oxide::scripts!` won't appear in Add Component. |
| A project won't open after moving the Oxide folder | Open it from the moved `bin/oxide`, or set `OXIDE_ENGINE_ROOT` to the new folder. |
| Something else | Check the editor log (`~/Library/Logs/Oxide/Editor.log` on macOS, `%LOCALAPPDATA%\Oxide\Editor.log` on Windows, `~/.local/share/Oxide/Editor.log` on Linux) and [open an issue](https://github.com/Jagatpratap/oxide/issues/new/choose) with it. |
