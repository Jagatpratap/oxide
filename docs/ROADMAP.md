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
- ◐ Scenes: create/load/save, additive multi-scene
  ([Scenes](https://docs.unity3d.com/Manual/working-with-scenes.html),
  [Multi-scene](https://docs.unity3d.com/Manual/MultiSceneEditing.html))
- ◐ Text YAML scene format with fileIDs + GUID references
  ([Text-based scene files](https://docs.unity3d.com/Manual/TextSceneFormat.html),
  [UnityYAML](https://docs.unity3d.com/Manual/UnityYAML.html))
- ☑ Prefabs: create, instantiate, nested, variants, overrides, apply/revert, unpack
  ([Prefabs](https://docs.unity3d.com/Manual/Prefabs.html),
  [Nested](https://docs.unity3d.com/Manual/NestedPrefabs.html),
  [Variants](https://docs.unity3d.com/Manual/PrefabVariants.html),
  [Overrides](https://docs.unity3d.com/Manual/prefabs-override.html))
- ☐ Constraints (Position/Rotation/Scale/Parent/Aim/LookAt)
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
- ◐ **Game** view: play/pause/step, resolution/aspect, stats, maximize on play
  ([Game view](https://docs.unity3d.com/Manual/GameView.html))
- ◐ **Console**: collapse, filter, stack trace, click-to-source
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
- ✓ Project launcher (Hub equivalent): `oxide new <dir>` / `oxide open <dir>`

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
MCP: `editor_*` tools drive all of it (`editor_project`, `editor_shortcuts`, `editor_search` added). Not yet: movable overlay panels, Scene view audio
toggle, Rect tool for multiple objects at once (it resizes the active object).

### P3 — Asset pipeline
- ☐ AssetDatabase with GUIDs, `.meta` files, Library cache, dependency tracking
  ([Asset Database](https://docs.unity3d.com/Manual/AssetDatabase.html),
  [Caching](https://docs.unity3d.com/Manual/importing-caching-assets.html))
- ◐ Importers: textures (PNG/JPG/TGA/EXR/HDR/PSD-flat), models (glTF, FBX, OBJ), audio (WAV/OGG/MP3/FLAC), fonts, shaders
  ([Asset types](https://docs.unity3d.com/Manual/AssetTypes.html),
  [Importing](https://docs.unity3d.com/Manual/import-assets.html))
- ◐ Import settings: Texture Importer settings in `<texture>.meta` (type, sRGB, max size, mipmaps,
  wrap/filter/aniso, compression), editable over MCP (`texture_import_settings`) and in the Inspector
  with Apply/Revert ☑; Presets ☐
  ([Texture Importer](https://docs.unity3d.com/Manual/class-TextureImporter.html),
  [Presets](https://docs.unity3d.com/Manual/Presets.html))
- ◐ File watching: textures (and their .meta), materials, shaders and render textures re-import
  when edited on disk (polled twice a second); models/audio, background re-import ☐. Asset Labels ☑
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
- ☐ Package export/import (`.oxidepackage`)
  ([Asset packages](https://docs.unity3d.com/Manual/AssetPackages.html))

**Exit:** drop a glTF + textures into Assets/, it appears in Project window with thumbnail and drags into the scene.

### P4 — Rendering core (Scriptable Render Pipeline + "URP"-equivalent)
- ◐ Render graph: passes declare reads/writes, unused passes culled, pooled transient textures
  aliased by lifetime (wgpu handles barriers). Post-processing runs on it; the scene passes don't
  yet ([SRP fundamentals](https://docs.unity3d.com/Manual/scriptable-render-pipeline-introduction.html))
- ◐ Extending the pipeline: URP renderer features — Full Screen Pass (user WGSL, validated, hot
  reloaded) and Render Objects (layer redraw with override material / depth test) at
  After Opaques / After Transparents / Before / After Post-processing. A whole custom
  `RenderPipeline` (Rust trait + pipeline asset) ☐
  ([Render pipelines](https://docs.unity3d.com/Manual/render-pipelines.html),
  [Custom rendering in URP](https://docs.unity3d.com/Manual/urp/customizing-urp.html))
- ◐ Universal pipeline: Forward+ (clustered lights: 16×9×24 clusters, up to 1024 lights) ☑;
  Deferred ☐
  ([URP](https://docs.unity3d.com/Manual/universal-render-pipeline.html),
  [Rendering paths](https://docs.unity3d.com/Manual/rendering-paths-introduction.html))
- ◐ MeshFilter / MeshRenderer / SkinnedMeshRenderer, submeshes, GPU instancing, SRP-batcher-style persistent buffers, frustum culling, LOD groups
- ◐ Cameras: perspective/ortho, viewport rect, multiple cameras/stacking, render textures, physical camera, clear flags, culling mask
  ([Cameras](https://docs.unity3d.com/Manual/Cameras.html),
  [Multiple cameras](https://docs.unity3d.com/Manual/MultipleCameras-landing.html),
  [Physical cameras](https://docs.unity3d.com/Manual/PhysicalCameras.html))
- ◐ Render queues & sorting (opaque front-to-back, transparent back-to-front)
  ([Render queues](https://docs.unity3d.com/Manual/built-in-rendering-order.html))
- ◐ Materials & shader property system, keywords/variants, shader import + reflection
  ([Materials](https://docs.unity3d.com/Manual/Materials.html),
  [Custom shaders](https://docs.unity3d.com/Manual/Shaders.html))
- ◐ Built-in shaders: Lit (metallic/specular PBR), Simple Lit, Unlit, Particles, Sprite, Skybox
  ([Prebuilt materials and shaders](https://docs.unity3d.com/Manual/built-in-materials-and-shaders.html))
- ◐ Textures: mipmaps, import-time BC7/BC5/BC1/BC3 compression (Intel ISPC, cached in
  `Library/TextureCache`), KTX2 load/save (BC1–7, ASTC 4×4, RGBA8, Zstandard), sampler states,
  Render Textures + `Camera.targetTexture` ☑; ETC2, Basis Universal, user cubemap/array assets ☐
  ([Textures](https://docs.unity3d.com/Manual/Textures-landing.html))
- ☑ Linear color space, HDR render targets
  ([Color and HDR](https://docs.unity3d.com/Manual/graphics-color.html))
- ◐ Anti-aliasing: MSAA (off/2x/4x/8x setting), FXAA, TAA (jitter, motion vectors incl.
  skinned, variance clipping) ☑; SMAA ☐
  ([URP anti-aliasing](https://docs.unity3d.com/Manual/urp/anti-aliasing.html))
- ✔ Occlusion culling: runtime CPU occluder depth buffer + min-depth pyramid (no bake, works
  with streaming); Occluder/Occludee Static flags, `Camera.useOcclusionCulling`,
  `allowOcclusionWhenDynamic`, conservative terrain occluders, Occlusion Portals. Baked PVS /
  Occlusion Areas not planned unless runtime culling proves insufficient
  ([Occlusion culling](https://docs.unity3d.com/Manual/OcclusionCulling-landing.html))

**Exit:** Sponza-class glTF scene renders with PBR, multiple cameras, transparent sorting, 60 fps on a laptop GPU.

### P5 — Lighting, shadows, post-processing
- ◐ Lights: Directional, Point, Spot, Area (baked); intensity units, cookies, layers
  ([Light sources](https://docs.unity3d.com/Manual/lighting-light-sources.html))
- ☑ Shadows: 4 sun cascades, soft PCF, point/spot shadows in a 4096 atlas (nearest lights
  first), shadow distance/bias/normal bias
  ([Shadows](https://docs.unity3d.com/Manual/Shadows.html))
- ◐ Environment: procedural skybox, sky captured to a prefiltered cubemap, ambient from L2
  spherical harmonics, fog ☑; 6-sided / cubemap / panoramic skyboxes ☐
  ([Sky](https://docs.unity3d.com/Manual/sky-landing.html))
- ◐ Reflection probes: Realtime (On Awake / Every Frame / Via Scripting) and Baked (static
  objects only; captured at load, not saved to disk), importance, blend distance, box
  projection, up to 15 + sky, GGX-prefiltered cube array ☑; custom cubemap probes ☐
  ([Reflections](https://docs.unity3d.com/Manual/reflections-landing.html))
- ◐ Global illumination ☑: CPU path-traced lightmapper (`oxide_lightmapper`): Baked / Mixed
  (Baked Indirect) / Realtime light modes, Contribute GI static objects, generated lightmap UVs
  (planar charts), atlas packing by surface area × Lightmap Resolution, shadows, bounces, sky,
  emission, edge-aware denoise, RGBM lightmaps saved next to the scene; Light Probe Groups baked
  to L2 SH and interpolated for dynamic objects; background bake with progress (Window →
  Rendering → Lighting, MCP `bake_lighting`); SSAO. ☐: GPU baking, shadowmask / subtractive
  modes, directional lightmaps, probe tetrahedralization, Adaptive Probe Volumes
  ([Direct and indirect lighting](https://docs.unity3d.com/Manual/direct-and-indirect-lighting.html))
- ◐ Lighting window: Generate Lighting / Clear Baked Data menu items, settings in
  RenderSettings (Inspector), progress in the status bar; a dedicated window and Light Explorer ☐
  ([Light Explorer](https://docs.unity3d.com/Manual/LightingExplorer-landing.html))
- ☑ Volume framework: global and local (collider) Volumes, weight, priority, blend distance,
  per-parameter override checkboxes; the overrides on the Volume's object are its profile
- ◐ Post FX ☑: Bloom (scatter, tint, clamp), Tonemapping (None/Neutral/ACES), Color
  Adjustments, Lift Gamma Gain, White Balance, Channel Mixer, Vignette, Chromatic Aberration,
  Film Grain, Lens Distortion, Depth of Field (Gaussian/Bokeh), Motion Blur, Split Toning,
  Shadows/Midtones/Highlights. ☐: Color Curves, Panini Projection, LUT texture, bloom dirt
  ([Post-processing in URP](https://docs.unity3d.com/Manual/urp/post-processing-and-full-screen-effects-urp.html),
  [Effect availability](https://docs.unity3d.com/Manual/post-processing-effect-availability-reference.html))
- ☑ Custom full-screen passes / renderer features (see P4)
  ([Custom rendering in URP](https://docs.unity3d.com/Manual/urp/customizing-urp.html))

**Exit:** day/night outdoor scene with CSM, baked GI, reflection probes and a post-FX volume matching a Unity reference screenshot side-by-side.

### P6 — Scripting & gameplay programming
- ◐ Script API: `GetComponent`, `AddComponent`, `Instantiate`, `Destroy`, `Find`, `SendMessage`-equivalent, events
  ([Get started with programming](https://docs.unity3d.com/Manual/scripting-get-started.html))
- ◐ Serialized fields → Inspector (`#[serialize]`, `#[range]`, `#[header]`, `#[tooltip]` attributes)
- ☑ Hot reload of script dylib with state preservation (domain-reload equivalent)
  ([Code reload and serialization](https://docs.unity3d.com/Manual/code-reload-serialization.html))
- ☐ Coroutines (`yield WaitForSeconds`, `WaitForEndOfFrame`) via async
  ([Coroutines](https://docs.unity3d.com/Manual/coroutines-section.html))
- ☐ Execution order control
  ([Control when your code updates](https://docs.unity3d.com/Manual/managing-update-order.html))
- ☐ Job system + parallel iteration
  ([Distribute work across threads](https://docs.unity3d.com/Manual/programming-distribute-work-threads.html))
- ☐ Web requests
  ([Web servers](https://docs.unity3d.com/Manual/web-request.html))
- ☐ Test framework (edit-mode & play-mode tests)
  ([Testing your code](https://docs.unity3d.com/Manual/test-framework/test-framework-introduction.html))
- ☐ Native plugin interface
  ([Plug-ins](https://docs.unity3d.com/Manual/plug-ins.html))

**Exit:** write a player controller script in the editor's script folder, save, it hot-reloads mid-play without losing scene state.

**Status:** the exit test passes. `oxide new/open` creates and opens a project whose `Scripts`
crate is compiled in the background and hot-reloaded in Edit and Play mode; component values,
objects and physics state survive, scripts get OnDisable/OnEnable across the reload, compile
errors go to the Console, panics are caught and logged, and scripts that fail to load keep
their data as Missing Script. The engine is one shared library (`oxide_dylib`) that the
project's editor and its Scripts both link. Not yet: coroutines, execution order, jobs, test
runner, native plugins.

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
- ◐ Animation: AnimationClip ☑, Animation window (dopesheet/curves, keyframe recording, any-property and object-reference keys) ☑, Animator Controller (state machine, transitions, parameters, layers, masks, blend trees 1D/2D) ☑ with `.controller` files, Override Controllers, Avatar Masks and the Animator window ☑, humanoid bone mapping ☑ / retargeting ☐, root motion ☑, IK (goals, hints, LookAt) ☑, animation events ☑, update modes ☑
  ([Mecanim](https://docs.unity3d.com/Manual/animation-mecanim.html))
- ☐ Timeline-style sequencer for cutscenes
- ◐ Audio: AudioClip import settings, AudioSource/Listener, 3D rolloff, Doppler, Audio Mixer (groups, snapshots, effects, exposed parameters ☑; sends, ducking ☐), reverb zones, filters, random containers
  ([Audio](https://docs.unity3d.com/Manual/Audio.html),
  [Audio Mixer](https://docs.unity3d.com/Manual/audio-mixer-landing.html),
  [Audio reference](https://docs.unity3d.com/Manual/AudioReference.html))
- ☐ Runtime UI (UI Toolkit equivalent): UXML-like layout files, USS-like styling, flexbox layout (`taffy`), events, data binding, UI Builder
  ([UI Toolkit](https://docs.unity3d.com/Manual/UIElements.html))
- ◐ Canvas UI (uGUI equivalent): Canvas, RectTransform, anchors, Image, Text, Button, Slider, ScrollRect, layout groups, EventSystem
  ([uGUI](https://docs.unity3d.com/Manual/com.unity.ugui.html),
  [UI comparison](https://docs.unity3d.com/Manual/UI-system-compare.html))
- ☐ Text rendering: SDF fonts (TextMeshPro equivalent), rich text
- ☐ Accessibility: screen reader hierarchy
  ([Accessibility](https://docs.unity3d.com/Manual/accessibility/_index.html))
- ◐ Particle System (Shuriken-style modules: emission, shape, velocity/size/color over lifetime, noise, collision, sub-emitters, trails, texture sheet) + editor
  ([Particle effects](https://docs.unity3d.com/Manual/ParticleSystems.html))
- ☐ GPU VFX graph (node-based, compute) — later
  ([Visual Effect Graph](https://docs.unity3d.com/Manual/VFXGraph.html))
- ☐ Decals, lens flares, Line/Trail renderers, billboards
  ([Decals](https://docs.unity3d.com/Manual/visual-effects-decals.html),
  [Lines and trails](https://docs.unity3d.com/Manual/visual-effects-lines-trails-billboards.html))
- ◐ Terrain: heightmap, sculpt/paint tools, layers, trees, details/grass, wind zones, LOD
  ([Terrain](https://docs.unity3d.com/Manual/script-Terrain.html))
- ☐ Video Player (decode via `ffmpeg` bindings or platform decoders) to texture/camera
  ([Video Player](https://docs.unity3d.com/Manual/VideoPlayer.html))

**Exit:** animated humanoid with blend tree locomotion, mixed audio, HUD menu, particle FX, on a terrain level.

### P9 — Tooling: Packages, Shader Graph, Profiler, Editor extensibility
- ☐ Package manager: manifest, registry, git deps, local packages, embedded packages
  ([Packages](https://docs.unity3d.com/Manual/PackagesList.html))
- ☐ Node graph framework → Shader Graph, VFX Graph, Animator, visual scripting all built on it
  ([Graph Toolkit](https://docs.unity3d.com/Manual/gtk/gtk-index.html))
- ◐ Editor extension API: custom inspectors ☑ (`CustomEditor` + `InspectorGui`); custom windows, property drawers,
  overlays, tools, menu items ☐
  ([Extending the Editor](https://docs.unity3d.com/Manual/extending-the-editor.html),
  [Custom overlay](https://docs.unity3d.com/Manual/overlays-custom.html),
  [Custom tools](https://docs.unity3d.com/Manual/UsingCustomEditorTools.html))
- ☐ Profiler: CPU timeline/hierarchy, GPU, memory, rendering, physics, audio modules; Frame Debugger; Memory Profiler
  ([Profiler](https://docs.unity3d.com/Manual/Profiler.html),
  [Graphics performance](https://docs.unity3d.com/Manual/graphics-performance-profiling.html),
  [Memory](https://docs.unity3d.com/Manual/performance-memory.html))
- ☐ Quality levels / adaptive performance scaling
  ([Runtime performance scaling](https://docs.unity3d.com/Manual/runtime-performance-scaling.html))
- ☐ Project auditor (static checks)
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
| Character controller + animation | | ✔ | ◐ CharacterController; Animator with layers (Override/Additive, Avatar Masks), 1D/2D blend trees, root motion, animation events, humanoid bone mapping, IK goals/hints + LookAt; retargeting between rigs, Animator window ☐ |
| Terrain + foliage | ✔ | ✔ | ◐ chunked LOD terrain, trees; missing grass/details, sculpt tools, streaming |
| Big worlds (8×8 km) | | ✔ | ◐ LOD Groups (+ `_LODn` model import, LOD bias); occlusion culling ✔; world streaming ✔ (scene cells; assets still load on the main thread); floating origin ☐ |
| Weapons, hits, ragdolls | | ✔ | ◐ raycasts, CharacterJoint, Ragdoll Wizard (from the humanoid map), animated ↔ limp switch; hit boxes ☐ |
| Game scripts in the editor | ✔ | ✔ | ☑ project Scripts crate, hot reload in Edit and Play mode |
| HUD, menus | ✔ | ✔ | ◐ Canvas UI + immediate GUI; UI Toolkit-style layout ☐ |
| Audio (engine, 3D, mixer) | ✔ | ✔ | ◐ 3D sources, car audio; mixer ☐ |
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
