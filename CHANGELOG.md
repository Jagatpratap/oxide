# Changelog

All notable changes to Oxide are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow
[Semantic Versioning](https://semver.org/). Before 1.0, minor versions (0.x) may break
project files and the scripting API; the notes say when they do.

The release workflow copies the section for a version into its GitHub Release, so add your
changes under **Unreleased** as you make them, and rename that heading to the version when
you tag it.

## [Unreleased]

## [0.1.0] - Unreleased

The first public version: a Unity-style editor and runtime written in Rust.

### Editor
- Oxide Hub (projects, new project, add from disk, installs) and `oxide new` / `oxide open`.
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
