# Oxide Engine — Roadmap

Oxide is a native Rust game engine and editor modeled on Unity's feature set and workflow
(GameObjects + Components, Scene/Hierarchy/Inspector/Project/Console editor, Scriptable Render
Pipeline, Prefabs, Mecanim-style animation, etc.).

- **Feature source of truth:** the Unity Manual — https://docs.unity3d.com/Manual/index.html
- **Full page-by-page checklist (3,604 pages):** [`UNITY_FEATURE_CHECKLIST.md`](UNITY_FEATURE_CHECKLIST.md)
  (regenerate with `scripts/gen_checklist.py`)
- **This file:** the build order. Each phase lists what we build, the Unity pages that define the
  behavior, and an exit criterion that must work before moving on.

> **Ground rule:** match Unity's *concepts, workflow and layout*, but not its name, logo, icons,
> or copyrighted assets/source. Panels laid out like Unity's are fine; Unity's artwork isn't.

---

## 0. Architecture

```
oxide/                       Cargo workspace
├── crates/
│   ├── oxide_core           app loop, time, logging, jobs/threads, reflection, serialization
│   ├── oxide_math           Vector3, Quaternion, Matrix4x4, Bounds, Ray, Color (glam-backed)
│   ├── oxide_scene          ECS storage + GameObject/Component/Transform API, Scenes, Prefabs
│   ├── oxide_asset          AssetDatabase, GUIDs + .meta files, importers, hot reload
│   ├── oxide_render         wgpu backend, RenderGraph, Scriptable Render Pipeline, shaders
│   ├── oxide_physics        3D (rapier3d) + 2D (rapier2d), Rigidbody/Collider/Joint components
│   ├── oxide_navigation     AI Navigation: NavMesh baking (Recast-style), agents, obstacles, links
│   ├── oxide_input          keyboard/mouse/gamepad/touch, Input System action maps
│   ├── oxide_audio          AudioSource/Listener/Mixer, 3D spatialization
│   ├── oxide_anim           Animator, AnimationClip, state machines, blend trees, skinning
│   ├── oxide_ui             runtime UI (UI Toolkit-like retained UI + uGUI-like canvas)
│   ├── oxide_script         user gameplay code: MonoBehaviour-style trait, hot reload
│   ├── oxide_mcp            built-in MCP server (AI agents drive the engine/editor) ← since P0
│   ├── oxide_engine         window + main loop runtime; registers the engine's MCP tools ← since P0
│   ├── oxide_editor         the editor app (egui + docking)
│   ├── oxide_player         standalone runtime that loads built content
│   └── oxide_build          build pipeline, asset bundling, platform targets
├── apps/
│   ├── oxide                engine sandbox binary (becomes the editor launcher in P2)
│   └── oxide_mcp_stdio      stdio ↔ HTTP bridge for stdio-only MCP clients
└── docs/                    ROADMAP, UNITY_FEATURE_CHECKLIST, MCP
```

### Technology choices

| Concern | Choice | Why |
|---|---|---|
| Windowing / events | `winit` | Cross-platform, the standard |
| GPU | `wgpu` (Vulkan / Metal / DX12 / WebGPU) | One API for every platform incl. web |
| Shaders | WGSL + `naga`; HLSL/GLSL import via naga frontends | Native to wgpu; reflection for material properties |
| Math | `glam` wrapped in Unity-style types | SIMD, fast, well-tested |
| Data model | Archetype ECS internally, GameObject/Component API on top | Unity's ergonomics, data-oriented speed (like Unity's own DOTS split) |
| Serialization | `serde` → YAML text scenes/prefabs (Unity-style), binary for builds | Diffable, mergeable, like UnityYAML |
| Editor UI | `egui` + `egui_dock` | Fast to build a dockable, Unity-like editor; custom inspectors are trivial |
| Physics | `rapier3d` / `rapier2d` | Mature, deterministic option, both dimensions |
| Audio | `cpal` + `symphonia` (decode), own mixer graph | Low-level control needed for Audio Mixer parity |
| Models / textures | `gltf`, `image`, `ktx2`, `basis-universal`; FBX via `ufbx` bindings | Covers Unity's main import formats |
| Gameplay scripting | **Rust** (decided). Scripts compiled to a `cdylib`, hot-reloaded by the editor | Equivalent of Unity's C# assemblies + domain reload |
| File watching | `notify` | Asset re-import on change |
| Profiling | `tracing` + `puffin` / Tracy | Backbone of the Profiler window |
| AI integration | Built-in **MCP server** (Streamable HTTP on `127.0.0.1:47474/mcp`, `tiny_http`) | Agents can inspect, edit and screenshot the running engine from day one; see [`MCP.md`](MCP.md) |

### MCP rule (applies to every phase)
Oxide ships a Model Context Protocol server inside the engine process. **Every phase adds MCP
tools for the features it builds**, so whatever a person can do in the editor, an AI agent can do
over MCP too: P1 adds GameObject/Hierarchy tools, P2 editor/selection/undo tools, P3 asset tools,
and so on. A feature isn't done until its MCP tools exist and are covered by a test.

### Scripting decision: Rust

Game scripts are written in Rust. The language differs from Unity, but the API matches it
(`start`, `update`, `get_component`, `instantiate`, coroutines, serialized fields in the Inspector).

```rust
use oxide::prelude::*;

#[derive(Script, Default)]
pub struct PlayerController {
    #[serialize] #[range(0.0, 20.0)] speed: f32,   // shows in the Inspector
    #[serialize] jump_force: f32,
    body: Option<ComponentRef<Rigidbody>>,
}

impl MonoBehaviour for PlayerController {
    fn start(&mut self, ctx: &mut Ctx) {
        self.body = ctx.get_component::<Rigidbody>();
    }
    fn update(&mut self, ctx: &mut Ctx) {
        let dir = Vec3::new(ctx.input.axis("Horizontal"), 0.0, ctx.input.axis("Vertical"));
        ctx.transform().translate(dir * self.speed * ctx.time.delta_time());
        if ctx.input.get_button_down("Jump") {
            if let Some(rb) = &self.body { rb.get(ctx).add_force(Vec3::Y * self.jump_force); }
        }
    }
}
```

How it works:
- **Project layout:** each game project has a `Scripts/` Cargo crate (`crate-type = ["cdylib"]`)
  that depends on the `oxide` API crate. The editor generates and manages it, like Unity's
  `Assembly-CSharp`.
- **Registration:** `#[derive(Script)]` registers the type, its fields and reflection data so the
  editor can list it under *Add Component* and draw its fields in the Inspector.
- **ABI safety:** Rust has no stable ABI, so the editor and the script dylib must be built with the
  same `rustc` and the same `oxide` version. A build hash is embedded in both and checked on load;
  a mismatch triggers a rebuild instead of a crash.
- **Hot reload (domain-reload equivalent):** on save → `cargo build` in the background → errors shown
  in the Console → serialize every script component's fields (serde, matched by field name like
  Unity) → unload the old dylib → load a copy of the new one (copying avoids file locks on
  Windows/macOS) → restore fields. Works in Edit mode and during Play mode.
- **Player builds:** scripts are linked statically into the game executable, with no dylib and no
  reload code (like IL2CPP).
- **Coroutines:** Rust `async` driven by the engine's frame loop (`ctx.wait_seconds(1.0).await`).

---

## Phases

Legend: ☐ not started · ◐ in progress · ☑ done

### P0 — Foundation (no Unity equivalent; prerequisite for everything)
- ☑ Cargo workspace (P0 crates: core, math, render, mcp, engine) + CI (fmt, clippy, test) on
  macOS/Windows/Linux. Later crates are added in the phase that needs them.
- ☑ `winit` window + `wgpu` device/surface, resize, vsync, clear color
- ☑ Main loop with Unity's update order: `FixedUpdate` → `Update` → `LateUpdate` → render
  ([Order of execution](https://docs.unity3d.com/Manual/execution-order.html),
  [Time & frame rate](https://docs.unity3d.com/Manual/managing-time-and-frame-rate.html))
- ☑ Logging (`Debug.Log` / Warning / Error) feeding a future Console
- ☑ Math types: Vector2/3/4, Quaternion, Matrix4x4, Mathf, Bounds, Ray, Plane, Rect, Color, Color32
  ([Programming with math](https://docs.unity3d.com/Manual/programming-math.html))
- ☑ Reflection/type registry: runtime `Reflect` trait, `TypeRegistry`, `#[derive(Reflect)]`.
- ☑ MCP server with 14 engine tools (info, Console, Time, pause/step, screenshot, inspect/edit
  objects, reflected types, quit). See [`MCP.md`](MCP.md).

**Exit:** a window renders a spinning triangle; logs print; fixed and variable timesteps tick correctly.
**Status:** passed. 60 fps on Metal (Apple M4), ~50 FixedUpdates per second of game time,
pause/step advances exactly N frames. Checked live over MCP, including screenshots.

### P1 — GameObjects, Components, Scenes, Prefabs
- ☑ GameObject: name, active/inactive, tag, layer, static flags
  ([GameObject class](https://docs.unity3d.com/Manual/class-GameObject.html),
  [Deactivate](https://docs.unity3d.com/Manual/DeactivatingGameObjects.html),
  [Static](https://docs.unity3d.com/Manual/StaticObjects.html),
  [Tags](https://docs.unity3d.com/Manual/Tags.html),
  [Layers](https://docs.unity3d.com/Manual/Layers.html))
- ☑ Transform hierarchy: local/world TRS, parenting, dirty propagation
  ([Transforms](https://docs.unity3d.com/Manual/class-Transform.html))
- ☑ Component model + MonoBehaviour-style lifecycle (`Awake`, `OnEnable`, `Start`, `Update`, `OnDisable`, `OnDestroy`)
  ([Components](https://docs.unity3d.com/Manual/Components.html),
  [Create components with scripts](https://docs.unity3d.com/Manual/CreatingComponents.html))
- ☑ Primitives: Cube, Sphere, Capsule, Cylinder, Plane, Quad
  ([Primitive objects](https://docs.unity3d.com/Manual/PrimitiveObjects.html))
- ☑ Scenes: create/load/save, additive multi-scene; multi-scene editing in the editor (scene headers in the
  Hierarchy with Set Active Scene / Save / Save As / Save All / Unload / Remove / Select Scene Asset / Add New
  Scene, active scene in bold receives new objects, per-scene `*`, unloaded scenes greyed with Load Scene, drag
  objects onto a scene header to move them, Open Scene Additive from the Project window or by dragging a scene
  into the Hierarchy, File > Save saves every modified scene, the scene setup reopens on launch, Undo and Play
  keep it; `SceneManager.GetActiveScene/SetActiveScene`; MCP `editor_scenes`)
  ([Scenes](https://docs.unity3d.com/Manual/working-with-scenes.html),
  [Multi-scene](https://docs.unity3d.com/Manual/MultiSceneEditing.html))
- ☑ Text YAML scene format with fileIDs + GUID references (asset references stay readable paths; each scene
  and prefab records the GUID of every asset it uses, so references survive assets moved outside the editor)
  ([Text-based scene files](https://docs.unity3d.com/Manual/TextSceneFormat.html),
  [UnityYAML](https://docs.unity3d.com/Manual/UnityYAML.html))
- ☑ Prefabs: create, instantiate, nested, variants, overrides, apply/revert, unpack
  ([Prefabs](https://docs.unity3d.com/Manual/Prefabs.html),
  [Nested](https://docs.unity3d.com/Manual/NestedPrefabs.html),
  [Variants](https://docs.unity3d.com/Manual/PrefabVariants.html),
  [Overrides](https://docs.unity3d.com/Manual/prefabs-override.html))
- ☑ Constraints: Position, Rotation, Scale, Parent, Aim and Look At (`oxide_scene::constraints`): weighted
  sources, Weight, at-rest values, offsets, per-axis freeze, Lock (unlocked edits update the offsets),
  Aim world-up types (Scene/Object/Object Rotation/Vector/None), Look At roll and up object; evaluated after
  animation and live in Edit mode; Activate / Zero buttons in the Inspector; `constraints` MCP tool
  ([Constraint components](https://docs.unity3d.com/Manual/constraint-components.html))

**Exit:** build a scene in code, save to YAML, reload it identically; prefab variant override round-trips.

**Status:** done except constraints and multi-scene editing. Prefabs: linked instances,
field-level overrides (bold in the Inspector), Apply/Revert/Unpack, variants, and scenes that
pick up prefab changes on load. Nested prefabs are stored flattened (an inner prefab's later
changes don't reach the outer one yet). Asset references are by path, not GUID (GUIDs come
with `.meta` files in P3).

### P2 — Editor shell (the Unity layout)
- ☑ Dockable window system, saveable layouts
  ([Customize workspace](https://docs.unity3d.com/Manual/CustomizingYourWorkspace.html))
- ☑ **Hierarchy** window: tree, drag-reparent, rename, multi-select, search
  ([Hierarchy](https://docs.unity3d.com/Manual/hierarchy-window.html))
- ◐ **Scene view**: fly camera, orbit/pan/zoom, grid, gizmos, move/rotate/scale/rect/transform handles, pivot/center, snapping, picking, scene visibility, 2D/3D toggle, draw modes, lighting/effects toggles, overlays
  ([Scene view](https://docs.unity3d.com/Manual/UsingTheSceneView.html),
  [Positioning](https://docs.unity3d.com/Manual/PositioningGameObjects.html),
  [Picking](https://docs.unity3d.com/Manual/ScenePicking.html),
  [Grid snapping](https://docs.unity3d.com/Manual/GridSnapping.html),
  [Overlays](https://docs.unity3d.com/Manual/overlays.html),
  [Gizmos & handles](https://docs.unity3d.com/Manual/gizmos-and-handles.html))
- ☑ **Inspector**: reflected fields, add/remove component, multi-object edit, lock, debug mode, custom editors
  for script components (`CustomEditor`), asset Inspectors (texture import settings, materials, models, prefabs,
  scripts/shaders, labels, preview)
  ([Inspector](https://docs.unity3d.com/Manual/UsingTheInspector.html))
- ☑ **Project** window: folder tree, Favorites, thumbnails (textures, materials, models, prefabs), grid/list zoom,
  search with `t:`/`l:`, Asset Labels, rename/duplicate/delete/move (references rewritten), drag-into-scene
  ([Project window](https://docs.unity3d.com/Manual/ProjectView.html))
- ☑ **Game** view: play/pause/step, Display 1-8 (`Camera.targetDisplay`), aspect ratios and fixed resolutions
  plus custom sizes (Add dialog, saved in Preferences), Low Resolution Aspect Ratios, VSync (Game view only),
  Scale, Play Focused / Maximized / Unfocused, Mute Audio, Stats (graphics, culling, skinned meshes, animators,
  audio), Gizmos
  ([Game view](https://docs.unity3d.com/Manual/GameView.html))
- ☑ **Console**: collapse, filter, search, Error Pause, stack trace, click-to-source; Clear on Play / Build /
  Recompile; Show Timestamp; 1- or 2-line entries; Copy / Copy with Stack Trace; context objects
  (`debug_log_context!(obj, ...)`, Unity `Debug.Log(message, context)`) ping or select their object; Stack Trace
  Logging per type; `Editor.log` and `Player.log` (Unity's locations) with Open Editor Log / Open Player Log
  ([Console](https://docs.unity3d.com/Manual/console-window.html))
- ☑ Toolbar, status bar, Undo/Redo + Undo History, selection history, Shortcuts Manager (rebinding, conflicts,
  keyboard view), Search window (⌘K: assets, objects, menus, settings)
  ([Toolbar](https://docs.unity3d.com/Manual/toolbar-lp.html),
  [Undo](https://docs.unity3d.com/Manual/UndoWindow.html),
  [Shortcuts](https://docs.unity3d.com/Manual/ShortcutsManager.html),
  [Search](https://docs.unity3d.com/Manual/editor-searching.html))
- ☑ Play mode: snapshot scene → run → restore on stop
- ☑ Preferences + Project Settings windows (Tags & Layers, Time, Physics, Quality, Graphics, Input, Player)
  ([Preferences](https://docs.unity3d.com/Manual/Preferences.html),
  [Project Settings](https://docs.unity3d.com/Manual/comp-ManagerGroup.html))
- ✓ Project launcher: the **Oxide Hub** window (`oxide` with no arguments, or File > New/Open Project in the editor): recent projects with last-opened time and missing-folder warnings, search, New project (template, name, location with the system folder picker), Add project from disk, Show in Finder / Remove from list, Installs page; opening builds the project's editor with progress, then starts it. Also `oxide new <dir>` / `oxide open <dir>`

**Exit:** create a project, place/move cubes with gizmos, edit in Inspector, Play/Stop restores state, Undo works for every edit.

**Status:** usable. `oxide` opens the editor (`--player` runs the game alone). Docked
Hierarchy / Scene / Game / Inspector / Project / Console with a saved layout; Scene view
fly/orbit/pan, grid, picking, icons, outlines, collider/light/camera gizmos and Move/Rotate/
Scale handles with snapping; Inspector for every reflected field with Add Component;
Undo/Redo; Play/Pause/Step with scene restore; File/Edit/GameObject/Component/Window menus.
Scene view draw modes (Shaded, Wireframe, Shaded Wireframe, Albedo, Normals, Lighting Only,
Shadow Cascades, Baked Lightmap); multi-object Inspector with lock and Debug mode; Undo History
and selection history; Project Settings and Preferences windows; Console call sites and stack
traces; Game view control bar. Rect (T) and Transform (Y) tools, Pivot/Center (Z) and
Global/Local (X) handle modes; 2D mode; Scene lighting and Effects (skybox, fog, particles,
post-processing) toggles; Hierarchy eye/hand toggles for scene visibility and pickability.
Project window thumbnails, grid/list zoom, Favorites, Unity search syntax, Asset Labels and asset
operations with reference fix-up; asset Inspectors; Shortcuts Manager; Search window (⌘K); custom
Inspectors for scripts (`CustomEditor`, `InspectorGui`, `scripts! { ...; editors: T }`).
Scene view overlays ☑ (`overlays.rs`): Tools, Tool Settings, Grid and Snap, View Options, Scene Camera,
Orientation, Cameras (live preview of a scene camera) and ProBuilder; drag by the grip, drop near an edge to dock
(top toolbar wraps onto more rows), double-click or right-click to collapse, Dock menu, Hide; Overlay menu on `` ` ``
with Show All / Hide All and saved presets; layouts kept in Preferences; MCP `editor_overlays`.
MCP: `editor_*` tools drive all of it (`editor_project`, `editor_shortcuts`, `editor_search` added). Not yet: Scene view
audio toggle. The Rect tool works on several objects ☑ (one rect around the selection; resizing scales the group
from the fixed edge and each object along its matching local axis).

### P3 — Asset pipeline
- ☑ AssetDatabase with GUIDs (`oxide_assets::database`): every file and folder under `Assets/` gets a
  `.meta` with a 32-digit `guid` (other `.meta` fields kept), scan cached in `Library/AssetDatabase.yaml`,
  moves made outside the editor (Finder, git) detected by GUID and their references fixed in every text asset,
  copied `.meta` files get new GUIDs, orphan `.meta` files removed, duplicates get fresh GUIDs;
  `GUIDToAssetPath` / `AssetPathToGUID` / `FindAssets` (`t:Type`, folders) / `GetDependencies` (recursive) /
  dependents; refreshed at editor start and every 3 s; MCP `asset_database`
  ([Asset Database](https://docs.unity3d.com/Manual/AssetDatabase.html),
  [Caching](https://docs.unity3d.com/Manual/importing-caching-assets.html))
- ☑ Importers: textures (PNG/JPG/TGA/BMP/EXR/HDR/TIFF/GIF/WebP/KTX2, PSD/PSB composite with matte removal),
  models (glTF; FBX binary 6.1-7.7 and ASCII with axis/unit conversion, hierarchy, pre/post rotation,
  geometric transforms, normals/UV sets/vertex colours, multi-material sub-meshes, materials with external or
  embedded textures, skinning and animation stacks baked to clips; OBJ with MTL), audio (WAV/OGG/MP3/FLAC),
  fonts, shaders
  ([Asset types](https://docs.unity3d.com/Manual/AssetTypes.html),
  [Importing](https://docs.unity3d.com/Manual/import-assets.html))
- ☑ Import settings: Texture Importer settings in `<texture>.meta` (type, sRGB, max size, mipmaps,
  wrap/filter/aniso, compression), editable over MCP (`texture_import_settings`) and in the Inspector
  with Apply/Revert; Model Importer (scale factor, convert units, generate colliders, normals
  Import/Calculate with smoothing angle, swap UVs, lightmap UVs, rig None/Legacy/Generic/Humanoid,
  import animation, loop time, import materials) and Audio Importer (force to mono, normalize, sample rate
  preserve/optimize/override, preload) in `.meta` with only changed fields, MCP `import_settings`; importer
  Presets (save / apply from the Inspector) and the Preset Manager (Project Settings: default presets per
  importer with path filters and per component, applied on first import / Add Component)
  ([Texture Importer](https://docs.unity3d.com/Manual/class-TextureImporter.html),
  [Presets](https://docs.unity3d.com/Manual/Presets.html))
- ☑ File watching: every asset type re-imports when edited on disk (polled twice a second): textures
  (and their .meta) in the background (the old texture stays until the worker finishes), materials,
  shaders, render textures, models, audio, animation assets, terrain data, data assets, fonts (their
  Font Assets rebuild); prefabs update their instances in the open scenes keeping overrides; an open scene
  changed on disk reloads (or warns when it has unsaved changes); our own saves aren't mistaken for
  outside edits. Asset Labels ☑
  (in `.meta`, searchable with `l:`)
- ◐ Runtime loading: multi-scene (`SceneHandle`, additive scenes, `MoveGameObjectToScene`),
  `LoadSceneAsync` / `UnloadSceneAsync` with `allowSceneActivation` (files parsed on a background
  thread, activated one per frame); `WorldStreamer` cells + Tools → World Streaming → Split Scene
  into Cells. Missing: background asset decoding (assets still load on the main thread),
  Resources-style folders, addressable bundles
  ([Managing assets at runtime](https://docs.unity3d.com/Manual/assets-managing-runtime.html))
- ☑ ScriptableObject-equivalent data assets: asset type registry (Create menu, icons, extensions),
  `register_data_asset::<T>()` for engine and script types (`scripts! { ...; assets: T => ("ext", "Menu") }`),
  YAML files edited in the Inspector and over MCP (`asset_types`, `data_asset`); Presets ☑
- ☑ Asset kinds with editors: Physics Material, Volume Profile, Lighting Settings, Lightmap Parameters,
  Input Actions (+ window), Animation Clip / Controller / Override Controller / Avatar Mask (+ Animation
  and Animator windows), Audio Mixer (+ window), Sprite Atlas, Tile / Animated Tile / Rule Tile,
  Tile Palette (+ window), Sprite Library, Prefab Variants
- ☑ Package export/import (`.oxidepackage`, Unity's `.unitypackage` layout: per-GUID `asset`, `asset.meta`,
  `pathname` in a gzipped tar; `.unitypackage` files import too): Export Package window with Include dependencies,
  Import Package window with New / Changed / Moved / Renamed status per asset, GUIDs preserved (an asset already
  in the project is updated where it is, a taken path gets a free name, folders merge); Assets menu and Project
  window entries; MCP `asset_packages`
  ([Asset packages](https://docs.unity3d.com/Manual/AssetPackages.html))

**Exit:** drop a glTF + textures into Assets/, it appears in Project window with thumbnail and drags into the scene.

### P4 — Rendering core (Scriptable Render Pipeline + "URP"-equivalent)
- ☑ Render graph: passes declare reads/writes, unused passes culled, pooled transient textures
  aliased by lifetime (wgpu handles barriers). The scene runs on it (Shadow Cascades, Shadow Atlas,
  Depth Prepass, SSAO, Draw Objects; the TAA resolve follows) and so does post-processing; the passes
  that ran each frame are recorded for the Frame Debugger ([SRP fundamentals](https://docs.unity3d.com/Manual/scriptable-render-pipeline-introduction.html))
- ☑ Extending the pipeline: URP renderer features — Full Screen Pass (user WGSL, validated, hot
  reloaded) and Render Objects (layer redraw with override material / depth test) at
  After Opaques / After Transparents / Before / After Post-processing. Universal Render Pipeline
  Asset (`.renderpipeline`: rendering path, MSAA, opaque/transparent layer masks, post-processing) picked in
  Project Settings > Graphics and overridable per Quality level; custom render pipelines from scripts
  (`RenderPipeline` trait, `ScriptableRenderContext`: clear, skybox, draw renderers with filtering and override
  material, terrain, effects, sprites, gizmos; `scripts! { ...; pipelines: T }`, hot reloaded) ☑
  ([Render pipelines](https://docs.unity3d.com/Manual/render-pipelines.html),
  [Custom rendering in URP](https://docs.unity3d.com/Manual/urp/customizing-urp.html))
- ☑ Universal pipeline: Forward+ (clustered lights: 16×9×24 clusters, up to 1024 lights);
  Deferred (G-buffer with per-object indirect light, full-screen lighting pass, forward-only objects after;
  one camera, no MSAA, like URP)
  ([URP](https://docs.unity3d.com/Manual/universal-render-pipeline.html),
  [Rendering paths](https://docs.unity3d.com/Manual/rendering-paths-introduction.html))
- ◐ MeshFilter / MeshRenderer / SkinnedMeshRenderer, submeshes, GPU instancing, SRP-batcher-style persistent buffers, frustum culling, LOD groups
- ☑ Cameras: perspective/ortho, viewport rect, multiple cameras, render textures, clear flags, culling mask; URP
  camera stacking (Render Type Base/Overlay, the Base camera's Stack, Overlay Clear Depth, overlays in the Base
  camera's viewport and render texture, post-processing at the end of the stack); Physical Camera (focal length,
  sensor size, lens shift, Gate Fit Vertical/Horizontal/Fill/Overscan/None, ISO/shutter/aperture/focus/blades/
  curvature/barrel clipping/anamorphism, `FocalLengthToFieldOfView`), Inspector shows the fields for the mode
  ([Cameras](https://docs.unity3d.com/Manual/Cameras.html),
  [Multiple cameras](https://docs.unity3d.com/Manual/MultipleCameras-landing.html),
  [Physical cameras](https://docs.unity3d.com/Manual/PhysicalCameras.html))
- ☑ Render queues & sorting: `Material.renderQueue` (From Shader = Geometry/AlphaTest/Transparent + URP Sorting
  Priority, or an explicit queue), renderer Sorting Layer / Order in Layer / Priority on Mesh and Skinned Mesh
  Renderers; opaques by sorting layer, queue and state, transparents by sorting layer, order, queue, priority, then
  back to front (Unity's CommonOpaque / CommonTransparent)
  ([Render queues](https://docs.unity3d.com/Manual/built-in-rendering-order.html))
- ◐ Materials & shader property system, shader import + reflection
- ☑ Shader keywords and variants: material keywords (`Material.EnableKeyword`/`DisableKeyword`/
  `IsKeywordEnabled`, `shaderKeywords`), global keywords (`Shader.EnableKeyword`), the built-in Lit shader's
  `_NORMALMAP` / `_METALLICSPECGLOSSMAP` / `_OCCLUSIONMAP` / `_EMISSION` / `_ALPHATEST_ON` variants, Shader Graph
  Boolean and Enum keywords with Shader Feature / Multi Compile (a pipeline variant per state, WGSL `override`
  constants, dead branches compiled away) or Dynamic Branch (a material value), Local/Global scope; variants
  are made on first use and cached
  ([Shader keywords](https://docs.unity3d.com/Manual/shader-keywords.html),
  [Keywords in Shader Graph](https://docs.unity3d.com/Packages/com.unity.shadergraph@17.0/manual/Keywords.html))
  ([Materials](https://docs.unity3d.com/Manual/Materials.html),
  [Custom shaders](https://docs.unity3d.com/Manual/Shaders.html))
- ☑ Built-in shaders: Lit (Metallic and Specular workflows), Simple Lit (Blinn-Phong), Baked Lit (lightmaps and
  probes only), Unlit, Particles, Sprite, Skybox; all work in Forward+ and Deferred; the material Inspector shows
  each shader's own inputs
  ([Prebuilt materials and shaders](https://docs.unity3d.com/Manual/built-in-materials-and-shaders.html))
- ☑ Textures: mipmaps, import-time compression cached in `Library/TextureCache` — BC7/BC5/BC1/BC3/BC4
  (Intel ISPC) on desktop, ASTC 4×4/6×6/8×8 (Oxide's own encoder) and ETC2 RGB/RGBA + EAC R/RG (Oxide's ETC1
  and EAC encoders) for mobile, all checked against GPU hardware decoders; Texture Importer platform tabs
  (Standalone/Android/iOS/WebGL overrides of max size, format, compression; Unity's mobile ASTC default) and
  the active build target; Texture Shape Cube (horizontal/vertical cross, strips, latitude-longitude
  panorama) and 2D Array (columns × rows) making `Cubemap` / `Texture2DArray` assets uploaded as cube and
  array views; KTX2 load/save (BC, ASTC, ETC2/EAC, RGBA8, Zstandard), sampler states, Render Textures +
  `Camera.targetTexture`. Not yet: Basis Universal, 3D textures
  ([Textures](https://docs.unity3d.com/Manual/Textures-landing.html))
- ☑ Linear color space, HDR render targets
  ([Color and HDR](https://docs.unity3d.com/Manual/graphics-color.html))
- ☑ Anti-aliasing: MSAA (off/2x/4x/8x setting), FXAA, TAA (jitter, motion vectors incl.
  skinned, variance clipping), SMAA (Low/Medium/High: luma edges with local contrast adaptation, L/U/Z
  shape blend weights, neighbourhood blending in linear light; coverage computed analytically instead of
  SMAA's area texture, no diagonal patterns)
  ([URP anti-aliasing](https://docs.unity3d.com/Manual/urp/anti-aliasing.html))
- ✔ Occlusion culling: runtime CPU occluder depth buffer + min-depth pyramid (no bake, works
  with streaming); Occluder/Occludee Static flags, `Camera.useOcclusionCulling`,
  `allowOcclusionWhenDynamic`, conservative terrain occluders, Occlusion Portals. Baked PVS /
  Occlusion Areas not planned unless runtime culling proves insufficient
  ([Occlusion culling](https://docs.unity3d.com/Manual/OcclusionCulling-landing.html))

**Exit:** Sponza-class glTF scene renders with PBR, multiple cameras, transparent sorting, 60 fps on a laptop GPU.

### P5 — Lighting, shadows, post-processing
- ☑ Lights: Directional, Point, Spot; Area lights (Rectangle / Disc, baked only as in URP: sampled emitters in
  lightmaps and light probes, one-sided, gizmos, GameObject > Light > Area Light); light cookies on Spot
  (projected through the cone) and Directional lights (Cookie Size / Offset tiling) from a cookie texture array;
  Rendering Layers (light Rendering Layers × renderer Rendering Layer Mask, 8 layers, Forward+ and Deferred).
  Not yet: Point light (cubemap) cookies, physical intensity units
  ([Light sources](https://docs.unity3d.com/Manual/lighting-light-sources.html))
- ☑ Shadows: 4 sun cascades, soft PCF, point/spot shadows in a 4096 atlas (nearest lights
  first), shadow distance/bias/normal bias
  ([Shadows](https://docs.unity3d.com/Manual/Shadows.html))
- ☑ Environment: procedural skybox, Skybox Material (Skybox/Procedural, 6 Sided, Cubemap, Panoramic shaders with
  Unity's Tint × 2 × Exposure and Rotation; RenderSettings `skybox_material`), sky captured to a prefiltered
  cubemap (re-captured when the skybox or its textures change), ambient from L2 spherical harmonics, the
  lightmapper's sky matching the skybox, fog
  ([Sky](https://docs.unity3d.com/Manual/sky-landing.html))
- ☑ Reflection probes: Realtime (On Awake / Every Frame / Via Scripting), Baked (Reflection Probe Static
  objects; Generate Lighting captures each and saves it as `ReflectionProbe-<n>.png` — RGBM HDR in a cross,
  imported as a Cube — set as its `baked_texture` and used from then on) and Custom (a Cubemap asset);
  importance, blend distance, box projection, up to 15 + sky, GGX-prefiltered cube array; Inspector per Type
  ([Reflections](https://docs.unity3d.com/Manual/reflections-landing.html))
- ☑ Global illumination: CPU path-traced lightmapper (`oxide_lightmapper`): Baked / Mixed
  (Baked Indirect) / Realtime light modes, Contribute GI static objects, generated lightmap UVs
  (planar charts), atlas packing by surface area × Lightmap Resolution, shadows, bounces, sky,
  emission, edge-aware denoise, RGBM lightmaps saved next to the scene; Light Probe Groups baked
  to L2 SH and interpolated for dynamic objects; background bake with progress (Window →
  Rendering → Lighting, MCP `bake_lighting`); SSAO; Lighting Modes — Baked Indirect, Shadowmask
  (static shadows of up to 4 Mixed lights per channel, `Light.shadowmask_channel`, combined with realtime
  shadows) and Subtractive (Mixed direct baked, the main light's realtime shadow subtracted down to the
  Realtime Shadow Color) — in Forward+ and Deferred; Directional lightmaps (dominant direction + rebalancing,
  Unity's decode). ☐: GPU baking, probe tetrahedralization, Adaptive Probe Volumes, Shadowmask light-probe
  occlusion for dynamic objects
  ([Direct and indirect lighting](https://docs.unity3d.com/Manual/direct-and-indirect-lighting.html))
- ☑ Lighting window (Window → Rendering → Lighting): Scene tab (Lighting Settings asset field + New Lighting
  Settings, Lighting Mode, Directional Mode, Realtime Shadow Color, lightmapping settings), Environment tab
  (Skybox Material, Sun Source, sky/ambient/reflection intensity, fog), Baked Lightmaps tab (lightmap,
  direction and shadowmask previews), Generate Lighting / Clear Baked Data with progress; Light Explorer
  (floating window: Lights, Reflection Probes, Light Probes and Static Emissives tables with editable
  type/mode/colour/intensity/range/shadows, enable toggles, click to select)
  ([Light Explorer](https://docs.unity3d.com/Manual/LightingExplorer-landing.html))
- ☑ Volume framework: global and local (collider) Volumes, weight, priority, blend distance,
  per-parameter override checkboxes; the overrides on the Volume's object are its profile
- ☑ Post FX: Bloom (scatter, tint, clamp, lens dirt texture + intensity), Tonemapping (None/Neutral/ACES),
  Color Adjustments, Lift Gamma Gain, White Balance, Channel Mixer, Vignette, Chromatic Aberration, Film Grain,
  Lens Distortion, Depth of Field (Gaussian/Bokeh), Motion Blur, Split Toning, Shadows/Midtones/Highlights,
  Screen Space Lens Flare, Color Curves (Master/Red/Green/Blue, Hue vs Hue, Hue vs Sat, Sat vs Sat, Lum vs
  Sat; blended per sample across volumes), Color Lookup (Unity strip LUTs, contribution), Panini Projection
  (distance, crop to fit)
  ([Post-processing in URP](https://docs.unity3d.com/Manual/urp/post-processing-and-full-screen-effects-urp.html),
  [Effect availability](https://docs.unity3d.com/Manual/post-processing-effect-availability-reference.html))
- ☑ Custom full-screen passes / renderer features (see P4)
  ([Custom rendering in URP](https://docs.unity3d.com/Manual/urp/customizing-urp.html))

**Exit:** day/night outdoor scene with CSM, baked GI, reflection probes and a post-FX volume matching a Unity reference screenshot side-by-side.

### P6 — Scripting & gameplay programming
- ☑ Script API: `GetComponent(s)`, `…InChildren`, `…InParent`, `AddComponent`, `Instantiate`, `Destroy`,
  `Find`, `FindObjectsByType`, `FindGameObjectWithTag`, `SendMessage`, `BroadcastMessage`, `SendMessageUpwards`,
  `Invoke`, `InvokeRepeating`, `CancelInvoke`, `IsInvoking`, `DontDestroyOnLoad`, tag / layer / active state,
  the Transform API; `Random` (seedable: value, Range, insideUnitSphere/Circle, onUnitSphere, rotation,
  ColorHSV), `PlayerPrefs` (int/float/string in the persistent data folder, saved on quit), `Screen` (size,
  dpi, full screen, SetResolution), `Application` (platform, isEditor, persistentDataPath, dataPath,
  targetFrameRate)
  ([Get started with programming](https://docs.unity3d.com/Manual/scripting-get-started.html))
- ☑ Serialized fields → Inspector: `#[reflect(...)]` attributes `range`, `min`, `tooltip`, `header`, `space`,
  `hide`, `multiline`, `hdr`, `delayed`, `label`, `formerly` (FormerlySerializedAs), `skip` (NonSerialized);
  component hooks `allow_multiple` (DisallowMultipleComponent), `requires` (RequireComponent, enforced on add
  and remove), `context_menu` / `on_context_menu` (ContextMenu items in the Inspector), `execute_always`
  (ExecuteAlways: Awake/OnEnable/Start/Update in Edit mode)
- ☑ Hot reload of script dylib with state preservation (domain-reload equivalent)
  ([Code reload and serialization](https://docs.unity3d.com/Manual/code-reload-serialization.html))
- ☑ Assembly Definitions: `.asmdef` (Unity's JSON) under `Assets/` make crates from their folder's scripts
  (`Library/ScriptAssemblies/<crate>`), `.asmref` adds folders, `references` → Cargo dependencies,
  auto-referenced assemblies → Scripts dependencies, platform include/exclude → target `cfg`, Editor-only
  assemblies left out of player builds, `allowUnsafeCode: false` → `forbid(unsafe_code)`, own `lib.rs` as the
  crate root if present; Unity's Inspector (Apply/Revert), Create → Scripting entries, Rust Script created in
  an assembly folder joins it, `assembly_definitions` MCP tool; only the changed assembly and its dependents
  rebuild. Not yet: define constraints / version defines, GUID references, precompiled references
  ([Assembly definitions](https://docs.unity3d.com/Manual/ScriptCompilationAssemblyDefinitionFiles.html))
- ☑ Coroutines as `async` blocks: `ctx.start_coroutine(|co| async move { ... })` with `co.wait_for_seconds`,
  `wait_for_seconds_realtime`, `next_frame` (yield null), `wait_until` / `wait_while`, `wait_for_end_of_frame`,
  `wait_for_fixed_update`, nested coroutines by `.await`, `co.with(|ctx| ...)` / `co.with_component` between
  waits; resumed where Unity resumes them (after Update, after each FixedUpdate, after rendering);
  `stop_coroutine`, `stop_all_coroutines`; stopped when the object is deactivated or destroyed and on script
  reload; panics logged like exceptions
  ([Coroutines](https://docs.unity3d.com/Manual/coroutines-section.html))
- ☑ Execution order control: Project Settings → Script Execution Order (list with orders around Default Time,
  `ProjectSettings/ScriptExecutionOrder.yaml`, MCP `project_settings` section `script_execution_order`) and
  `default_execution_order()` (Unity `[DefaultExecutionOrder]`); applies to Awake/OnEnable, Start, FixedUpdate,
  Update and LateUpdate, scene order within equal orders
  ([Control when your code updates](https://docs.unity3d.com/Manual/managing-update-order.html))
- ☑ Job system + parallel iteration: a worker pool (one thread per core but the main one), `Jobs::schedule`
  (IJob) and `Jobs::schedule_parallel_for` (IJobParallelFor, batch size) with `JobHandle` dependencies,
  `complete` / `is_completed` / `combine`, `Jobs::run` with a result, blocking `Jobs::parallel_for` and
  `par_for_each_mut` over borrowed data; Rust's `Send`/`Sync` take the place of Unity's safety checks; jobs
  that panic are logged and still complete; waiting threads help run queued jobs (no pool deadlocks)
  ([Distribute work across threads](https://docs.unity3d.com/Manual/programming-distribute-work-threads.html))
- ☑ Web requests: `UnityWebRequest` (GET, POST raw/form/JSON, PUT, DELETE, HEAD, request headers, timeout,
  download to file) over HTTP and HTTPS (ureq + rustls), sent in the background; `WebRequestOperation` with
  `is_done`, `result` (Success / ConnectionError / ProtocolError / DataProcessingError), `response_code`,
  `error`, response headers, `text` / `data` / `json` / `texture`, awaitable in coroutines (`op.wait().await`)
  ([Web servers](https://docs.unity3d.com/Manual/web-request.html))
- ☑ Test framework: play-mode tests (`async fn t(co: Co)` listed under `scripts! { ...; tests: t }`, Unity
  `[UnityTest]`: run in Play mode on a fresh scene each, across frames, failing by panic, 30 s timeout) and
  edit-mode tests (`#[test]` in the Scripts crate, run with `cargo test` in the background); Window → General
  → Test Runner (PlayMode / EditMode tabs, Run All / Run Selected / Rerun Failed, status, durations, failure
  output), MCP `editor_test_runner`; new projects get an example of each
  ([Testing your code](https://docs.unity3d.com/Manual/test-framework/test-framework-introduction.html))
- ☑ Native plugin interface: C ABI libraries (`.dylib` / `.dll` / `.so`) anywhere under `Assets/`, found by name
  (`lib` prefix optional), loaded once; `native_plugin! { unsafe "lib" { fn f(a: i32) -> i32; } }` (Unity's
  `[DllImport]`, bound on first call, DllNotFoundException-style panics); `NativePlugins::load` / `symbol`
  for manual use; Plugin Inspector (Any Platform, Editor, Standalone, Android, iOS, Load on startup) in the
  `.meta`; player builds leave out other-OS libraries and ones excluded from Standalone. Not yet: Unity's
  low-level native rendering plugin interface
  ([Plug-ins](https://docs.unity3d.com/Manual/plug-ins.html))

**Exit:** write a player controller script in the editor's script folder, save, it hot-reloads mid-play without losing scene state.

**Status:** the exit test passes. `oxide new/open` creates and opens a project whose `Scripts`
crate is compiled in the background and hot-reloaded in Edit and Play mode; component values,
objects and physics state survive, scripts get OnDisable/OnEnable across the reload, compile
errors go to the Console, panics are caught and logged, and scripts that fail to load keep
their data as Missing Script. The engine is one shared library (`oxide_dylib`) that the
project's editor and its Scripts both link.

### P7 — Physics, Input, 2D
- ◐ 3D: Rigidbody, Box/Sphere/Capsule/Mesh colliders, triggers, layers collision matrix, raycasts/overlaps, physics materials, joints (Fixed/Hinge/Character ☑), CharacterController, articulations, ragdoll wizard ☑, cloth (later)
  ([Built-in 3D physics](https://docs.unity3d.com/Manual/PhysicsOverview.html),
  [Character control](https://docs.unity3d.com/Manual/character-control-section.html),
  [Joints](https://docs.unity3d.com/Manual/joints-section.html),
  [Physics Debug](https://docs.unity3d.com/Manual/PhysicsDebugVisualization.html))
- ☑ Curves and gradients (AnimationCurve, Gradient, MinMaxCurve) with Curve and Gradient Editors, used by
  WheelCollider friction curves, the CarController torque/steering curves, particles and audio rolloff
- ◐ AI Navigation: NavMeshSurface baking (Recast-style voxelization, tiled, multi-threaded, from render meshes/terrain or colliders; `.navmesh` files) ☑, agent types and 32 areas with costs ☑, NavMeshModifier / Modifier Volume ☑, NavMeshAgent (path following, braking, area masks/costs, avoidance with priorities, `update_position`/`update_rotation`, off-mesh link traversal) ☑, NavMeshObstacle with carving ☑, NavMeshLink ☑, `NavMesh.CalculatePath / SamplePosition / Raycast / FindClosestEdge` ☑, runtime `BuildNavMesh` ☑, Navigation window + Scene view display ☑; auto-generated jump/drop links, height mesh, async carving, NavMesh Components for 2D ☐
  ([AI Navigation](https://docs.unity3d.com/Packages/com.unity.ai.navigation@2.0/manual/index.html))
- ☑ Collision callbacks: `OnCollisionEnter/Stay/Exit`, `OnTriggerEnter/Stay/Exit`
  ([Collision](https://docs.unity3d.com/Manual/collision-section.html))
- ◐ Input: legacy axes + action-map system, rebinding, gamepad, touch, IME text input
  ([Input](https://docs.unity3d.com/Manual/Input.html))
- ◐ 2D: Sprites ☑, Sprite Renderer (simple/sliced/tiled) ☑, Sprite Atlas ☑, Sprite Editor (auto/grid slicing, pivots, 9-slice borders) ☑, sorting layers ☑, Sprite Library/Resolver ☑, sprite flipbook animation ☑; sorting groups, Sprite Shape, skinning ☐
  ([Sprites](https://docs.unity3d.com/Manual/sprite/sprite-landing.html))
- ◐ Tilemaps: Grid, Tilemap, TilemapRenderer, Tile Palette window + Scene painting (brush, box, fill, erase, picker), Animated and Rule Tiles ☑; isometric/hex, Tilemap Collider ☐
  ([Tilemaps](https://docs.unity3d.com/Manual/tilemaps/tilemaps-landing.html))
- ☐ 2D physics (Rigidbody2D, colliders, effectors, joints)
  ([2D physics](https://docs.unity3d.com/Manual/2d-physics/2d-physics.html))
- ☐ 2D lights & shadows (URP 2D renderer)
  ([2D in URP](https://docs.unity3d.com/Manual/2d-urp-landing.html))

**Exit:** a playable 3D third-person controller demo and a 2D tilemap platformer, both built only in the editor.

### P8 — Animation, Audio, UI, VFX, Terrain, Video
- ◐ Animation: AnimationClip ☑, Animation window (dopesheet/curves, keyframe recording, any-property and object-reference keys) ☑, Animator Controller ☑: sub-state machines with Entry/Exit/Any State and entry / state machine transitions, transitions with exit time, fixed or normalized duration, offset, interruption sources and ordered interruption, solo/mute; blend trees 1D, 2D (simple directional, freeform directional, freeform cartesian) and Direct, nested, with per-motion time scale, cycle offset and mirror; state speed multiplier, motion time, mirror and cycle offset parameters, Write Defaults; layers with weights, Avatar Masks (transforms and humanoid body parts), Override/Additive, Sync (with timing), IK pass; StateMachineBehaviours (`scripts! { ...; behaviours: T }`); `.controller` files, Override Controllers, Avatar Masks; Animator window with layers/parameters panels (live values in Play), graph (zoom, box select, copy/paste, sub-machine breadcrumbs), blend tree graph with live weights, transition timeline, Compute Thresholds, undo/redo ☑, humanoid bone mapping ☑, humanoid retargeting ☑ (Avatars from the model and the scene rig; clips made for another humanoid rig — other names, bone axes, proportions, armature scale — are retargeted in model space against both rest poses when an Animator plays them, hips motion scaled by height), root motion ☑, IK (goals, hints, LookAt) ☑, animation events ☑, update modes ☑
  ([Mecanim](https://docs.unity3d.com/Manual/animation-mecanim.html))
- ☑ Timeline: `.playable` assets with Animation (eases, overlap blends, extrapolation, speed, clip-in, track and clip offsets, avatar masks), Activation (post-playback state), Audio, Signal (Signal Emitters, `.signal` assets, Signal Receiver reactions, retroactive / emit once), Control (activation, nested Directors, particles, prefabs) and Playable tracks (script `PlayableBehaviour`s via `scripts! { ...; playables: T }`), Track Groups, mute/lock; PlayableDirector (play on awake, wrap Hold/Loop/None, update modes, bindings, exposed references, Play/Pause/Resume/Stop/Evaluate/time); Timeline window (tracks, binding fields, move/trim/ease/drag between tracks, snapping, markers, properties panel, Edit-mode preview with restore, Record into clips, drops from Project/Hierarchy, undo/redo) ☑; Cinemachine tracks ☐
- ◐ Audio: AudioClip import settings, AudioSource/Listener, 3D rolloff, Doppler, Audio Mixer (groups, snapshots, effects, exposed parameters, Send/Receive aux buses, Duck Volume sidechain ducking ☑), Audio Random Container (sequential/shuffle/random with no-repeat, volume/pitch/time randomisation, manual or automatic Pulse/Offset trigger, Infinite/Clips/Cycles loops ☑), reverb zones, filters
  ([Audio](https://docs.unity3d.com/Manual/Audio.html),
  [Audio Mixer](https://docs.unity3d.com/Manual/audio-mixer-landing.html),
  [Audio reference](https://docs.unity3d.com/Manual/AudioReference.html))
- ◐ Runtime UI (UI Toolkit): `oxide_uitk` — `.uxml` documents (Unity's `ui:` elements, `<Style src>`), `.uss`
  style sheets (type/`#name`/`.class`/pseudo-class selectors with descendant and child combinators,
  specificity, `--variables`/`var()`, ~60 USS properties, inheritance, Default Runtime Theme), flexbox layout
  (`taffy`), Panel Settings (constant pixel size / scale with screen size), UI Document component (plays in the
  Game view in Edit mode, hot reload), controls (VisualElement, Label, Button, Toggle, RadioButton, Slider,
  SliderInt, ProgressBar, DropdownField, TextField, IntegerField, FloatField, Foldout, ScrollView, GroupBox,
  Image) with hover/active/focus/checked states, click/change/submit events and a script API (clicked, changed,
  value, set_text, classes, display, inline styles), UI Builder window (StyleSheets, Hierarchy, Library,
  Viewport with Preview, Inspector), `ui_toolkit` and `editor_ui_builder` MCP tools. Not yet: data binding,
  templates (`<Template>`/`<Instance>`), ListView/TreeView/MultiColumnListView, transitions, USS transforms on
  painting (rotate/scale), world-space panels, rich text in labels
  ([UI Toolkit](https://docs.unity3d.com/Manual/UIElements.html))
- ◐ Localization: locales and Localization Settings (Project Settings page, system-language startup,
  fallback chains), String Table Collections (`.stringtable`, Smart Strings with plural/choose/cond, CSV
  import/export), Asset Table Collections (`.assettable` for textures, sprites, audio), Localize String Event
  (TextMeshPro, UI Text, UI Toolkit elements) and Localize Asset Event components, script API
  (`world.localized`, `set_locale`), Localization Tables window with preview locale, system fallback fonts for
  CJK/Indic scripts in egui, `localization` MCP tool. Not yet: pseudo-localization, per-locale Font Assets
  switching, metadata, Google Sheets sync, plural rules beyond English-style
  ([Localization](https://docs.unity3d.com/Packages/com.unity.localization@1.5/manual/index.html))
- ◐ Canvas UI (uGUI equivalent): Canvas + Canvas Scaler, Image, Text, Button, Slider, Toggle ☑; RectTransform (anchors, pivot, size delta, nesting) and auto layout (Horizontal / Vertical / Grid Layout Groups with padding, spacing, alignment, control child size, force expand, reverse; Layout Element; Content Size Fitter) ☑; ScrollRect, EventSystem ☐
  ([uGUI](https://docs.unity3d.com/Manual/com.unity.ugui.html),
  [UI comparison](https://docs.unity3d.com/Manual/UI-system-compare.html))
- ◐ Text rendering (TextMeshPro): `.fontasset` Font Assets from `.ttf`/`.otf` (SDF atlas, Dynamic/Static population, multi-page atlases, fallback fonts, bold/italic/tab settings) ☑, rich text (`b i u s color alpha size cspace mark sub sup uppercase lowercase smallcaps br noparse`) ☑, layout (wrapping, alignment incl. Justified/Flush, Overflow/Ellipsis/Truncate, spacing, auto size, kerning) ☑, SDF material (face dilate, softness, centred outline, underlay) ☑, TextMeshPro (3D) and TextMeshPro - Text (UI) components ☑, GameObject menu entries, Create Font Asset from font files, Font Asset previews ☑, Sprite Assets with `<sprite>` (index, name, asset, tint, color; fallbacks; Create Sprite Asset from sliced textures) ☑, `<link>` with hit testing (`TextMesh::find_link`, UI hover and `OnLinkClicked`) ☑, Color Gradients (four-corner vertex gradients, presets, `<gradient>`) ☑, Material presets as assets ☑, Font Asset Creator window (character sets, decimal/hex ranges, file, auto sizing, report, atlas preview, save) ☑
- ☑ Accessibility: platform screen readers through AccessKit (VoiceOver, Narrator, Orca; the game UI and the editor), Unity's AccessibilityHierarchy / AccessibilityNode (roles, states, frames, hints, values) and AssistiveSupport (active hierarchy, isScreenReaderEnabled, announcements, invoked / increment / decrement / focus events), `Accessible` component for Canvas UI, `accessibility` MCP tool
  ([Accessibility](https://docs.unity3d.com/Manual/accessibility/_index.html))
- ☑ Particle System (Shuriken): main module (prewarm, start delay, simulation speed, unscaled time, stop actions, random seed), emission with Bursts (cycles, interval, probability), shapes (cone, sphere, hemisphere, box, point, circle, donut, edge, rectangle, mesh; arc, thickness, shape transform, randomize/spherize direction, align to direction), Velocity / Limit Velocity / Force / Color / Size / Rotation over Lifetime and by Speed, Inherit Velocity, Lifetime by Emitter Speed, External Forces with Particle System Force Fields, Noise (fractal 3D), Collision (planes, physics world, bounce/dampen/lifetime loss/kill speeds, OnParticleCollision), Triggers, Sub Emitters (birth, collision, death, trigger, manual), Texture Sheet Animation, Lights, Trails, Renderer modes (billboard, stretched, horizontal, vertical) + editor
  ([Particle effects](https://docs.unity3d.com/Manual/ParticleSystems.html))
- ☑ VFX Graph (GPU particles): `.vfx` assets with Spawn → Initialize Particle → Update Particle → Output
  Particle Quad contexts joined by flow, ~45 of Unity's blocks (spawn rates/bursts on the CPU; Set attribute
  blocks with Composition, random variants, position shapes, Gravity/Force/Linear Drag/Turbulence/Conform to
  Sphere/Vortex, collisions with plane/sphere/box, kill volumes, Color/Size/Alpha over Life with gradients and
  curves, Flipbook Player), operators (Get Attribute, Random Number, the Shader Graph math library), exposed
  properties; compute-shader simulation with a dead-particle stack, billboards drawn from the particle buffer
  (alpha/additive/opaque, face camera or along velocity, flipbooks, soft default dot), local or world space;
  Visual Effect component (Play/Stop/Reinit/pause/play rate/SetFloat/SetVector, plays in Edit mode), VFX Graph
  window with live apply, `visual_effect` and `editor_vfx_graph` MCP tools. Not yet: multiple outputs per
  system, mesh/strip/line outputs, GPU events, particle sorting, bounds culling, readback of the alive count
  ([Visual Effect Graph](https://docs.unity3d.com/Manual/VFXGraph.html))
- ☑ Decals, lens flares, Line/Trail renderers, billboards: Lens Flare (SRP) ☑ (`.lensflare` data assets with Image/Circle/Polygon/Ring elements, blend modes, multiple elements with Uniform/Curve/Random distribution, radial distortion; Lens Flare component following Lights with distance, screen and spot-cone attenuation and GPU depth occlusion; previews in the Project window and Inspector), Screen Space Lens Flare Volume override ☑ (ghosts, warped halo, streaks, chromatic aberration from the bloom mips); URP Decal Projector (DBuffer: depth prepass, projected boxes blended into Lit/terrain albedo in Forward+ and Deferred; size, pivot, UV scale/offset, opacity, distance and angle fade) ☑, Line Renderer and Trail Renderer (camera-facing or Transform Z ribbons, width curves, colour gradients, corner and end-cap vertices, loop, Stretch/Tile/Distribute/Repeat texture modes, emitting, autodestruct) ☑, Billboard Renderer + Billboard Asset (images per view angle) ☑, material Blending Mode (Alpha, Premultiply, Additive, Multiply) ☑
  ([Lens flares](https://docs.unity3d.com/Manual/urp/shared/lens-flare/lens-flare.html),
  [Decals](https://docs.unity3d.com/Manual/visual-effects-decals.html),
  [Lines and trails](https://docs.unity3d.com/Manual/visual-effects-lines-trails-billboards.html))
- ☑ Terrain: `.terraindata` files (16-bit heights, control maps, holes, trees, details) ☑, up to 16 layers with normal maps ☑, holes (render + collider) ☑, chunked LOD with partial rebuilds ☑, Terrain tools in the Inspector (Raise/Lower, Paint Holes, Paint Texture, Set Height, Smooth, Stamp; brushes with shapes/textures; Paint Trees with density/size/colour variation and Mass Place; Paint Details; Terrain Settings: wind, sizes, resolutions, RAW import/export) ☑, neighbour terrains with Auto Connect and cross-tile brushes ☑, grass and detail meshes with wind ☑, Undo for strokes ☑, MCP `terrain_edit` ☑, tree billboards baked from each prototype (8 views) with Billboard Start / Fade Length / Max Mesh Trees ☑, Wind Zones (directional/spherical, pulse, turbulence) bending trees, driving grass and pushing particles ☑, Terrain Toolbox (Create New Terrain grids from heightmaps, batch Terrain Settings, heightmap/splatmap export) ☑, terrain streaming (far tiles unload data and meshes, reload from file) ☑
  ([Terrain](https://docs.unity3d.com/Manual/script-Terrain.html))
- ☑ Video Player: VideoClip assets (H.264 MP4/MOV via OpenH264 built from source, animated GIF), background decoding with seeking and B-frame reordering, render modes (Camera Far/Near Plane with aspect ratios and alpha, Render Texture, Material Override, API Only) through Render Textures written in place, Play On Awake / Wait For First Frame / Loop / Skip On Drop / playback speed, prepare/play/pause/stop/seek API, prepared/started/loop point/error messages, MP4 audio (AAC) through the audio system in sync (Direct or via an Audio Source), `video` MCP tool, MP4 writing (`encode_mp4`)
  ([Video Player](https://docs.unity3d.com/Manual/VideoPlayer.html))

**Exit:** animated humanoid with blend tree locomotion, mixed audio, HUD menu, particle FX, on a terrain level.

### P9 — Tooling: Packages, Shader Graph, Profiler, Editor extensibility
- ☑ Package manager: `Packages/manifest.json` (registry versions over the npm protocol, scoped registries, `file:` local packages, git URLs with `#revision`), embedded packages in `Packages/`, transitive dependencies, `Library/PackageCache`, `packages-lock.json`; package assets at `Packages/<name>/...` (Project window root, player builds); package Rust crates join the Scripts crate; Package Manager window (In Project list, details, samples import, + from disk / git URL / name, remove); `packages` MCP tool
  ([Packages](https://docs.unity3d.com/Manual/PackagesList.html))
- ◐ Node graph framework → Shader Graph, VFX Graph, Animator, visual scripting all built on it
  ([Graph Toolkit](https://docs.unity3d.com/Manual/gtk/gtk-index.html)): the node canvas ☑ (`node_canvas.rs`:
  typed ports, wires, pan/zoom, box select, Create Node searcher, drop-a-wire-to-create, groups, sticky notes,
  copy/paste/duplicate, framing, the window's own undo); Shader Graph ☑; VFX Graph ☑; visual scripting ☐
- ☑ Shader Graph (URP Lit/Unlit targets): `.shadergraph` / `.shadersubgraph` assets, ~120 of Unity's nodes with
  Unity's formulas (inputs, geometry, textures, math, UV, noise/Voronoi/shapes, artistic blend/normal/colour,
  logic, Custom Function, Sub Graph, Boolean/Enum keywords), master stack (Vertex: Position/Normal/Tangent; Fragment:
  Base Color … Alpha Clip Threshold), Graph Settings (material, surface, render face, alpha clipping, cast
  shadows, normal space), Blackboard properties (Float/Vector/Color/Boolean/Texture2D, sliders, HDR, exposed),
  compiled to WGSL hooks of the Lit shader (shadow and depth passes included), magenta error shader, live node
  previews (2D quads / 3D spheres like Unity) and Main Preview, materials with Unity's Shader dropdown and
  Surface Inputs, Create Material from a graph, `editor_shader_graph` MCP tool. Not yet: Gradient/Matrix types, sampler states, Shader Graph-driven decals/sprites/terrain, Convert to Sub
  Graph
  ([Shader Graph](https://docs.unity3d.com/Packages/com.unity.shadergraph@17.0/manual/index.html))
- ◐ ProBuilder: `ProBuilder Mesh` component (n-gon faces over shared positions, material slots, smoothing groups,
  Auto UVs, vertex colours) rebuilt into the MeshFilter/MeshCollider whenever it changes and saved with the
  scene; Shape tool with Unity's shapes (Cube, Plane, Cylinder, Cone, Prism, Stairs, Arch, Pipe, Torus, Sphere,
  Door) whose parameters stay editable until the geometry is edited; ProBuilderize; Object/Vertex/Edge/Face
  modes with Scene view picking (click, Shift to add), highlights and a move handle; Extrude, Inset, Subdivide,
  Merge, Triangulate, Flip Normals, Delete, Detach faces, Bridge Edges, Weld/Collapse Vertices, Center Pivot,
  Apply Material, Smoothing Group, Vertex Color, Export OBJ; undo; ProBuilder window and Scene view overlay;
  `probuilder` MCP tool. Not yet: UV Editor (manual UVs), Poly Shape / Bezier Shape, Boolean, edge loops and
  rings, rotate/scale handles for elements, Lightmap UV generation
  ([ProBuilder](https://docs.unity3d.com/Packages/com.unity.probuilder@6.0/manual/index.html))
- ☑ Editor extension API: custom inspectors (`CustomEditor` + `InspectorGui`); `scripts! { ...; extensions: register }`
  with `EditorExtensions`: custom windows (`EditorWindow`: dockable tabs, own window like `GetWindow`, `on_enable`,
  `on_selection_change`, state kept until closed), menu items (`[MenuItem]` paths incl. new top-level menus,
  validate functions grey items out, priorities, `Ctrl+Shift+K` shortcuts), property drawers (`[CustomPropertyDrawer]`
  by nested type or `#[reflect(drawer = "...")]`, `PropertyGui` with the Inspector's label column), Scene view overlays
  (draggable/dockable/hideable, contextual `applies`), Scene view tools (`EditorTool` in the Tools overlay, Tool
  Settings contents, `SceneGui` handles: position/slider/free-move/button, lines, discs, labels); `EditorContext`
  (selection, Undo with drag coalescing, open windows, run menus, status); hot reload swaps them;
  `editor_get_state` lists them, `editor_set_tool` / `editor_open_window` / `editor_execute_menu` use them
  ([Extending the Editor](https://docs.unity3d.com/Manual/extending-the-editor.html),
  [Custom overlay](https://docs.unity3d.com/Manual/overlays-custom.html),
  [Custom tools](https://docs.unity3d.com/Manual/UsingCustomEditorTools.html))
- ◐ Profiler: Unity's Profiler API (`Profiler.BeginSample/EndSample`, `ProfilerMarker`, `profile_scope!`, counters) with every system, script callback (per type), phase and render step sampled; Profiler window with CPU Usage (stacked chart, Timeline per thread, Hierarchy with total/self/calls), GPU Usage per render pass (timestamp queries), Rendering, Scripts, Physics, Audio, Memory and Video modules over 300 frames ☑; Frame Debugger (draw events per pass with mesh, material, shader, blend/cull/zwrite state; step through to draw the frame up to an event) ☑; `profiler` MCP tool ☑; standalone Memory Profiler snapshots ☐
  ([Profiler](https://docs.unity3d.com/Manual/Profiler.html),
  [Graphics performance](https://docs.unity3d.com/Manual/graphics-performance-profiling.html),
  [Memory](https://docs.unity3d.com/Manual/performance-memory.html))
- ◐ Quality levels / adaptive performance scaling: Quality levels gain Render Scale (game cameras render smaller and are stretched, bilinear or point; UI and Scene view stay full size), Texture Quality (global mipmap limit), Anisotropic Textures (Disable / Per Texture / Force Enable), an additional lights limit (brightest and nearest kept) and per-platform default levels applied to player builds ☑; Adaptive Performance: thermal state (macOS `NSProcessInfo.thermalState`), CPU/GPU frame times (GPU timestamp queries) and bottleneck detection, an indexer with Unity's Decrease / FastDecrease / Increase actions and delays, and Resolution, LOD, Shadow Distance, Shadow Quality, MSAA, Lights, Texture Quality and Framerate scalers (visual impact, targets, bounds, levels, `OverrideLevel`), events, Project Settings page, Window > Analysis > Adaptive Performance with a thermal/bottleneck simulator, `adaptive_performance` MCP tool ☑; Android/iOS thermal providers and CPU/GPU performance levels (need those platforms) ☐
  ([Runtime performance scaling](https://docs.unity3d.com/Manual/runtime-performance-scaling.html))
- ☑ Project auditor (static checks): scripts (scene searches, `get_component`, allocations, logging, instantiate, messages, blocking and file I/O inside `update`/`fixed_update`/`late_update`/`on_gui`, with line numbers), assets (uncompressed or oversized textures, sizes that can't be block-compressed, missing mipmaps, mipmapped sprites, normal maps imported as color, preloaded long audio, large WAVs, generated mesh colliders, assets no Build Settings scene uses) and Project Settings (physics rate, max timestep, collision matrix, mobile default quality, render scale > 1, Adaptive Performance off, Deferred + MSAA, default company name); issues have category, area, severity and a recommendation; Window > Analysis > Project Auditor (views, area/severity filters, search, click to open the script line or select the asset), `project_auditor` MCP tool
  ([Project Auditor](https://docs.unity3d.com/Manual/project-auditor/project-auditor.html))

### P10 — Build & platforms
- ◐ Build Profiles window: scene list, target, dev build, scripts-only build, clean build
  ([Create a build](https://docs.unity3d.com/Manual/BuildSettings.html))
- ☐ Content cooking: strip unused assets, compress textures per platform, pack into archives
  ([Content output](https://docs.unity3d.com/Manual/build-content-output.html),
  [Reduce file size](https://docs.unity3d.com/Manual/ReducingFilesize.html))
- ☐ Command-line builds + editor CLI args
  ([CLI builds](https://docs.unity3d.com/Manual/build-command-line.html),
  [Editor CLI](https://docs.unity3d.com/Manual/EditorCommandLineArguments.html))
- ◐ Player settings (icon, splash, resolution, company/product name)
- ◐ Targets in order: macOS → Windows → Linux → Web (wgpu/WebGPU) → Android → iOS → Dedicated Server
  ([Platform development](https://docs.unity3d.com/Manual/PlatformSpecific.html))
- ☐ Deterministic builds, build reports
  ([Deterministic builds](https://docs.unity3d.com/Manual/build-deterministic-builds.html),
  [Analyze builds](https://docs.unity3d.com/Manual/build-analyze-builds.html))

**Status:** desktop builds work. File → Build Settings (scene list, product name,
resolution, fullscreen, output) → Build / Build And Run compiles a release player
(`oxide_player`, static, no editor) with the project's Scripts and copies `Assets/` next to it
(`<Product>.app` on macOS, `<Product>` + `<Product>_Data/` elsewhere). Scripts switch scenes
with `ctx.load_scene` and quit with `ctx.quit`. MCP is off in shipped games (`--mcp` turns it
on). Not yet: stripping unused assets, texture compression, icons/splash, cross-compiling,
web/mobile, dedicated server.

### P11 — Extended (after a usable 1.0)
- ☐ XR via OpenXR
  ([XR](https://docs.unity3d.com/Manual/XR.html))
- ◐ Networking / multiplayer
  ([Multiplayer](https://docs.unity3d.com/Manual/multiplayer.html),
  [Netcode for GameObjects](https://docs.unity3d.com/Packages/com.unity.netcode.gameobjects@latest))
  - ☑ `oxide_network` over UDP (renet): host / server / client, NetworkManager with a player
    prefab, NetworkObject spawn/despawn from prefabs (late joiners get everything),
    NetworkTransform (server or owner authority, interpolation), NetworkRigidbody,
    ServerRpc / ClientRpc, client shuts down cleanly when the server goes away; MCP `network`
  - ☐ NetworkVariables, client-side prediction and reconciliation, lag compensation for hits,
    interest management (only send what's near), relay / NAT punch-through, lobby,
    headless dedicated-server build
- ☐ HDRP-class high-end features: ray tracing, volumetrics, SSR, SSGI, hair/skin, water
- ☐ Embedded/consoles


## Target: a racing game and a BGMI-style battle royale

The goal is an engine on which those games *could* be built (we don't build them). What each
needs, and where the engine is:

| Need | Racing | Battle royale | Status |
|---|---|---|---|
| Editor to build levels | ✔ | ✔ | ☑ P2 editor, prefabs |
| Vehicle physics (WheelCollider) | ✔ | ✔ (vehicles) | ☑ Rapier ray-cast vehicles, CarController |
| Character controller + animation | | ✔ | ◐ CharacterController; Animator with layers (Override/Additive, Avatar Masks), 1D/2D blend trees, root motion, animation events, humanoid bone mapping, IK goals/hints + LookAt, sub-state machines, interruption, Direct/nested blend trees, sync layers, StateMachineBehaviours, Animator window ☑; retargeting between humanoid rigs ☑ |
| Terrain + foliage | ✔ | ✔ | ✔ terrain files, sculpt/paint tools, 16 layers, holes, trees with billboards, Wind Zones, grass/details, neighbours, Terrain Toolbox, streaming |
| Big worlds (8×8 km) | | ✔ | ◐ LOD Groups (+ `_LODn` model import, LOD bias); occlusion culling ✔; world streaming ✔ (scene cells; assets still load on the main thread); floating origin ☐ |
| Weapons, hits, ragdolls | | ✔ | ◐ raycasts, CharacterJoint, Ragdoll Wizard (from the humanoid map), animated ↔ limp switch; hit boxes ☐ |
| Game scripts in the editor | ✔ | ✔ | ☑ project Scripts crate, hot reload in Edit and Play mode |
| HUD, menus | ✔ | ✔ | ◐ Canvas UI + immediate GUI + UI Toolkit (UXML/USS, flexbox, controls, UI Builder) |
| Audio (engine, 3D, mixer) | ✔ | ✔ | ◐ 3D sources, car audio, Audio Mixer (groups, effects, snapshots, exposed parameters, sends, ducking), Audio Random Containers |
| AI (opponents, bots) | ✔ | ✔ | ☑ WaypointCircuit + CarAIControl racing AI; NavMesh baking over terrain, NavMeshAgent with avoidance, obstacles with carving, links (50 agents at 60 FPS in a debug build); behaviour trees / perception ☐ |
| Multiplayer | ◐ | ✔ | ◐ UDP netcode: spawning, NetworkTransform with interpolation, NetworkRigidbody, RPCs; prediction, lag compensation, interest management, dedicated server ☐ |
| Ship the game | ✔ | ✔ | ◐ desktop player builds (macOS/Windows/Linux); Android/iOS ☐ |

Build order from here: netcode (prediction, interest management) and dedicated server → mobile.

---

## How we work through this
1. Finish phases in order; the exit criterion is the gate.
2. For each feature, open the linked Unity page, write down the behaviors/settings it describes, implement, then tick the matching box(es) in `UNITY_FEATURE_CHECKLIST.md`.
3. Every feature lands with a sample scene in `examples/` that demonstrates it.
4. Every feature lands with its MCP tools (see "MCP rule" above).
