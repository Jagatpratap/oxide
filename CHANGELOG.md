# Changelog

All notable changes to Oxide are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow
[Semantic Versioning](https://semver.org/). Before 1.0, minor versions (0.x) may break
project files and the scripting API; the notes say when they do.

The release workflow copies the section for a version into its GitHub Release, so add your
changes under **Unreleased** as you make them, and rename that heading to the version when
you tag it.

## [Unreleased]

### Added
- **Oxide Hub as its own app** (`oxide-hub`, Lucide box icon in rust colour) and the main
  download on every release: installs editor versions from the release list in Production,
  Beta and Alpha channels (download with progress, SHA-256 check, side-by-side installs,
  Uninstall, Locate), lists projects with the editor version each uses, opens a project with
  its version (or installs it first), switches a project's version after asking, and creates
  new projects with a chosen version.
- `releases.json`: the release list the Hub reads, updated by the release workflow.
- `ProjectSettings/ProjectVersion.txt` records the editor version that last opened a project.
- `oxide create DIR` and `oxide open DIR --build-only`; release builds report the tag's
  version (`oxide --version`).

- **Prebuilt editor in every release** (`bin/oxide-editor` and the engine library in `lib/`,
  made by `scripts/build_sdk.py`): opening a project starts it at once and compiles nothing
  of the engine; only the project's Scripts compile, against the prebuilt library (through
  `sdk/oxide_dylib`). Projects get a `rust-toolchain.toml` for the Rust version the editor was
  built with, which the first open installs if needed.

### Changed
- `oxide` with no arguments starts the Hub program instead of a Hub built into the editor.
- `Scripts/Cargo.toml` takes the engine from the project workspace (`oxide = { workspace =
  true }`); older projects are updated when opened, so switching editor versions no longer
  leaves the Scripts on the old engine.

## [0.1.0] - Unreleased

The first public version: a Unity-style editor and runtime written in Rust.

### Editor
- `oxide new` / `oxide open` project launcher.
- Dockable windows with saved layouts: Hierarchy, Scene, Game, Inspector, Project, Console,
  plus 30+ tool windows.
- Scene view tools (Hand, Move, Rotate, Scale, Rect, Transform), pivot/center, global/local,
  snapping, draggable overlays, picking, scene visibility, draw modes, 2D mode.
- Inspector with reflected fields, multi-object editing, presets, custom editors, and asset
  inspectors and import settings for textures, models and audio.
- Unity-style drag and drop between the Project window, Hierarchy, Scene view and Inspector.
- Undo/Redo for every edit, Play mode with scene restore, multi-scene editing.
- Prefabs with nested prefabs, variants, overrides, Apply/Revert/Unpack.
- GUID AssetDatabase (`.meta` files), file watching and background re-import,
  `.unitypackage`-style asset packages, and a Package Manager (registry, git, local, embedded,
  built-in packages).
- Editor extension API: custom windows, menu items, property drawers, overlays, Scene view tools.
- Profiler, Frame Debugger, Project Auditor, Test Runner and Adaptive Performance windows.

### Runtime
- Rendering on wgpu (Metal, Vulkan, Direct3D 12): Forward+ and Deferred paths on a render graph,
  Lit / Simple Lit / Baked Lit / Unlit shaders, shader keywords, cascaded shadows, SSAO, decals,
  TAA / FXAA / SMAA, HDR post-processing with volumes, skyboxes, reflection probes, lightmapping
  with shadowmask modes, light probes, area lights, cookies, camera stacking, custom render
  pipelines.
- Shader Graph and VFX Graph with their node libraries.
- Physics (Rapier): rigidbodies, colliders, joints, raycasts, character controller, wheel
  colliders. NavMesh baking and agents.
- Animation: Animator state machines, blend trees, humanoid retargeting, Timeline, constraints.
- Audio (kira): 3D sources, mixer groups with sends and ducking, random containers.
- UI: Canvas UI with layout groups, UI Toolkit (UXML/USS), TextMeshPro-style text with
  sprites, links and gradients, Localization, accessibility for screen readers.
- Terrain with painting tools, trees with billboards, wind, details and streaming; particle
  systems; ProBuilder modeling; video playback (H.264 MP4, GIF).
- Rust scripting with hot reload: components, coroutines, jobs, timers, web requests, native
  plugins, play-mode tests.
- Multiplayer basics (renet): NetworkManager, NetworkObject, NetworkTransform, RPCs.
- An MCP server in every running engine, so AI agents can inspect and drive the editor.
- Standalone player builds for the desktop platform you build on.

### Packages
- `com.oxide.blenderkit`: a BlenderKit browser that adds models, materials and HDRIs to a scene.

[Unreleased]: https://github.com/Jagatpratap/oxide/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/Jagatpratap/oxide/releases/tag/v0.1.0
