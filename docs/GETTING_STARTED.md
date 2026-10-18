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

### Install the Oxide Hub

Oxide installs like Unity: the **Oxide Hub** is the app you download, and it installs editor
versions and opens your projects.

1. Download the Hub for your computer from the
   [latest release](https://github.com/Jagatpratap/oxide/releases/latest):
   `oxide-hub-<version>-aarch64-apple-darwin.zip` (Apple Silicon Mac),
   `oxide-hub-<version>-x86_64-apple-darwin.zip` (Intel Mac),
   `oxide-hub-<version>-x86_64-pc-windows-msvc.zip` (Windows) or
   `oxide-hub-<version>-x86_64-unknown-linux-gnu.tar.gz` (Linux).
2. Optionally check the download against its `.sha256` file:
   `shasum -a 256 -c oxide-hub-*.sha256` (Windows: `certutil -hashfile <file> SHA256`).
3. Install it:
   - **macOS:** unzip and drag **Oxide Hub** into Applications. The Hub isn't notarized yet, so
     the first time, right-click it → **Open** (or run
     `xattr -dr com.apple.quarantine "/Applications/Oxide Hub.app"`).
   - **Windows:** unzip anywhere and run `Oxide Hub\oxide-hub.exe`.
   - **Linux:** unpack, then run `./install.sh` to add it to your applications menu (or just
     run `./oxide-hub`).
4. Open the Hub, go to **Installs → Install Editor**, and install a version (see below).

The `oxide-<version>-<platform>` archives on a release page are the editors themselves: the
prebuilt editor (`bin/oxide-editor`) with its engine library (`lib/`), the `oxide` launcher,
and the engine source (player builds compile it). The Hub downloads, checks and unpacks them for
you, but you can also unpack one by hand and add it with **Installs → Locate**.

### Install Oxide from source

```sh
git clone https://github.com/Jagatpratap/oxide.git
cd oxide
cargo build --release -p oxide       # the editor
cargo run --release -p oxide_hub     # the Hub, which lists this checkout as an editor
```

Projects opened with that editor compile against your checkout, so engine changes you make
show up in your projects the next time they open.

## 2. The Oxide Hub

### Installs

**Installs → Available** lists the versions in the release list, in three channels:

| Channel | What it is |
|---|---|
| **Production** | Tested releases (`v0.2.0`). Use these for your projects. |
| **Beta** | Previews of the next version (`v0.3.0-beta.1`, `v0.3.0-rc.1`). Back up projects before opening them. |
| **Alpha** | Early builds (`v0.3.0-alpha.1`), for testing only. |

Click **Install** to download a version. The Hub shows the progress, checks the download's
SHA-256, and unpacks it into `~/Library/Application Support/Oxide/Editors/<version>` (macOS),
`%LOCALAPPDATA%\Oxide\Editors\<version>` (Windows) or `~/.local/share/oxide/editors/<version>`
(Linux). Set `OXIDE_EDITORS_DIR` to install somewhere else. Versions install side by side.

**Installs → Installed** lists what you have. Each version's ••• menu has Show in Finder,
Release notes and **Uninstall** (which deletes it). **Locate** adds an editor folder you already
have (an unpacked editor archive, or an engine checkout built with `cargo build -p oxide`);
removing a located editor only takes it off the list.

The release list is [`releases.json`](../releases.json) in the repository, which the release
workflow updates whenever a version is published. **Refresh** reads it again. A newer Hub shows
up as a link at the bottom of the sidebar.

### Projects

- **Projects** lists every project you've opened, with the **editor version** it uses (read
  from `ProjectSettings/ProjectVersion.txt`, which the editor writes) and when you last opened
  it. Click a project to open it with its version.
- The version cell is a menu: pick another installed version to open the project with it. The
  Hub asks first, because the project is then rebuilt and saved for that version.
- If a project's version isn't installed, the Hub offers to **install** it (then opens the
  project when it's done) or to open it with a version you have.
- **New project:** pick the editor version, a name, a location and a template, then **Create
  project**.
- **Open:** add a project folder from disk.
- A project's ••• menu has Open, Show in Finder and Remove from list.

An installed version's editor is prebuilt, so a project opens right away: nothing of the engine
is compiled. The editor then compiles the project's scripts in the background against its
engine library (a few seconds the first time, then only what changed), and the Console shows
when they're loaded. Scripts must be compiled with the same Rust version the editor was built
with, so the project gets a `rust-toolchain.toml` naming it, and the first open installs that
version through rustup if you don't have it (the Hub shows the progress). An editor built from
source (a **Located** checkout) has no prebuilt editor: its first open compiles the engine into
the shared build cache, which takes a few minutes. Once the editor starts, the Hub closes; **File → New Project / Open
Project** in the editor brings it back.

If Rust isn't installed, the Hub says so at the top of the Projects page, with a link to
rustup.rs.

### From a terminal

Each editor's `oxide` program (in its `bin/` folder) also works on its own:

```sh
oxide new ~/Games/MyGame      # create and open
oxide open ~/Games/MyGame     # open
oxide create ~/Games/MyGame   # create without opening
oxide --version               # version and engine folder
oxide                         # starts the Hub
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

Every project is ready for AI coding agents (Claude Code, Cursor, VS Code Copilot, Codex,
Gemini CLI and others). Oxide writes these into the project:

- **`AGENTS.md`** (and `CLAUDE.md`, which includes it): what an agent should know about an Oxide
  project. Add notes about your own game under "This game".
- **`.claude/skills/oxide/`**: the Oxide skill. It teaches an agent the project layout, how to
  write and register scripts, and how to drive the editor. It's updated whenever the project
  opens.
- **`.mcp.json`, `.cursor/mcp.json`, `.vscode/mcp.json`**: connect the agent to this project's
  editor. Each project's editor serves MCP on its own port, so several open projects don't get
  mixed up.

To use it: open the project in the Oxide editor, open the same folder in your agent, and approve
the `oxide` MCP server when it asks. The agent can then read the Console, inspect and edit
objects, create materials and prefabs, take screenshots of the editor and the game, enter Play
mode and click through the editor UI, while editing your scripts as files.

See [MCP.md](MCP.md) for other clients and every tool.

## 9. Updating Oxide

- **New editor version:** install it from **Installs → Available**, then pick it in a project's
  version menu. The project opens right away; its scripts recompile against the new version
  in the background. Read the [changelog](../CHANGELOG.md) for breaking changes
  first, and uninstall the old version when you no longer need it.
- **New Hub:** when the release list has a newer Hub, the sidebar shows a link to download it.
  Replace the old app with the new one; your installs and project list stay.
- **Source install:** `git pull`, rebuild, then reopen your projects.

## 10. Troubleshooting

| Problem | What to do |
|---|---|
| "Rust is not installed (cargo was not found)" | Install Rust from [rustup.rs](https://rustup.rs), then open a new terminal (or log out and in) so `cargo` is on your `PATH`. |
| macOS says "Oxide Hub" "can't be opened" | Right-click it → **Open**, or run `xattr -dr com.apple.quarantine "/Applications/Oxide Hub.app"`. |
| "Cannot download the release list" in Installs | Check your internet connection and click **Refresh**. You can still open projects with versions you have, and add editors with **Locate**. |
| A project's version shows ⚠ | That version isn't installed. Open the project and choose **Install**, or pick another version from its version menu. |
| The first open takes several minutes | With an installed version, this is rustup downloading the Rust version the editor was built with (once per version). With an editor built from source, the engine is compiling (once per machine and engine version). |
| Console: "this Oxide editor was built with rustc …, but the Scripts are being compiled with …" | Run the `rustup toolchain install` command it names. Don't delete the project's `rust-toolchain.toml`; the editor writes it to pick that version. |
| Disk is filling up | Uninstall editor versions you don't use, and delete the build cache (`~/Library/Caches/Oxide/Build`, `%LOCALAPPDATA%\Oxide\Build` or `~/.cache/oxide/build`; it's safe), or move it with `OXIDE_BUILD_DIR`. |
| Linux build fails on `alsa-sys`, `libudev-sys` or `libdbus-sys` | Install the development packages listed in [Install Rust](#install-rust). |
| Scripts don't reload | Look at the Console for compile errors. A component type that isn't listed in `oxide::scripts!` won't appear in Add Component. |
| A project won't open after moving an editor folder | Add the moved folder with **Installs → Locate**, or set `OXIDE_ENGINE_ROOT` to it. |
| Something else | Check the editor log (`~/Library/Logs/Oxide/Editor.log` on macOS, `%LOCALAPPDATA%\Oxide\Editor.log` on Windows, `~/.local/share/Oxide/Editor.log` on Linux) and [open an issue](https://github.com/Jagatpratap/oxide/issues/new/choose) with it. |
