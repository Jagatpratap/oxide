# Oxide MCP server

Every running Oxide engine serves the [Model Context Protocol](https://modelcontextprotocol.io)
so AI agents (Claude Code, Claude Desktop, Cursor, ...) can inspect and drive it: read the Console,
change object fields, pause/step time and look at rendered frames.

- **Endpoint:** `http://127.0.0.1:47474/mcp` (Streamable HTTP, JSON responses)
- **Change address:** `OXIDE_MCP_ADDR=127.0.0.1:5000 cargo run -p oxide` or `--mcp-addr 127.0.0.1:5000`
- **Turn off:** `cargo run -p oxide -- --no-mcp`
- **Protocol versions:** 2025-11-25, 2025-06-18, 2025-03-26, 2024-11-05
- **Code:** protocol and transport in `crates/oxide_mcp`, the engine's tools in
  `crates/oxide_engine/src/mcp_tools.rs`

## Connect a client

Start the engine first: `cargo run -p oxide`.

**Claude Code:** this repo's `.mcp.json` already registers the server as `oxide`; approve it when
Claude Code asks. To add it elsewhere:

```sh
claude mcp add --transport http oxide http://127.0.0.1:47474/mcp
```

**stdio-only clients** (for example Claude Desktop's `claude_desktop_config.json`): build the bridge
with `cargo build -p oxide_mcp_stdio`, then point the client at the binary:

```json
{
  "mcpServers": {
    "oxide": { "command": "/path/to/oxide/target/debug/oxide-mcp-stdio" }
  }
}
```

If the engine isn't running, the bridge returns an error that says how to start it.

## Tools

79 tools (generated from the running editor's `tools/list`). Tools named `editor_*` exist
only when the editor is running (not with `--player`).

| Tool | What it does | Changes state |
|---|---|---|
| `get_engine_info` | Engine version, GPU, window size, frame rate, render stats (draw calls, triangles), object count, scene name | no |
| `get_console_logs` | Console messages oldest first | no |
| `clear_console` | Removes all Console messages | yes |
| `get_time` | Unity Time values and FixedUpdate count | no |
| `set_time` | Changes Time.timeScale (0 freezes), fixedDeltaTime, maximumDeltaTime | yes |
| `set_paused` | Pauses/resumes the game loop (editor Pause) | yes |
| `step_frames` | Advances the simulation by `count` frames of 1/60 s immediately (works paused or not), then returns the new Time | yes |
| `capture_screenshot` | Renders a PNG | no |
| `get_hierarchy` | The scene's GameObject tree (ids, names, component types) | no |
| `inspect_object` | Everything the Inspector shows for one GameObject: transform (local + world), tag, layer, children, every component with all field values | no |
| `create_object` | Creates an empty object or a primitive (Cube, Sphere, Capsule, Cylinder, Plane, Quad — with a matching collider), or a Camera / Directional Light / Point Light / Spot Light | yes |
| `destroy_object` | Destroys an object and its children | yes (destructive) |
| `set_object` | Edits an object's header and Transform: name, active, tag, layer, static, parent (null = root), position/euler_angles/scale (local by default; `space: "world"` for world position/rotation) | yes |
| `add_component` | Adds a component by type name (see list_component_types) with optional initial `fields` | yes |
| `remove_component` | Removes a component (by type name, `index` for duplicates) | yes (destructive) |
| `set_component` | Edits component fields like the Inspector: `fields` is {name: value} | yes |
| `list_component_types` | Every component type that can be added (built-ins and registered scripts) with its fields, kinds and enum options | no |
| `instantiate` | Unity Instantiate: deep-copies an object (and children); optional position/euler_angles for the copy | yes |
| `save_scene` | Saves the open scene as YAML (path relative to the project, e.g | yes |
| `load_scene` | Replaces the open scene with a saved one (or `additive: true` to add it) | yes (destructive) |
| `new_scene` | Clears the scene and creates Unity's defaults (Main Camera, Directional Light) unless `empty` | yes (destructive) |
| `get_render_settings` | Environment settings (Lighting window): procedural sky, ambient, fog, exposure, bloom, shadow distance, vignette | no |
| `set_render_settings` | Edits RenderSettings fields (see get_render_settings) | yes |
| `list_assets` | Project files under Assets/ plus loaded meshes, materials, textures, models and animation clips (asset paths usable in fields) | no |
| `create_material` | Creates a material at `path` (or edits it if it exists) with Lit/Unlit fields: base_color, base_map (texture path), metallic, smoothness, normal_map, emission_color, emission_intensity, surface_type (Opaque/Cutout/Transparent), tiling, double_sided | yes |
| `import_model` | Loads a glTF/GLB model from the project and instantiates it in the scene (meshes, materials, textures, skinning, animations) | yes |
| `simulate_input` | Presses/releases keys or mouse buttons as if from the keyboard (for testing game controls) | yes |
| `raycast` | Unity Physics.Raycast from `origin` along `direction`: the first collider hit (object, point, normal, distance) | no |
| `physics_settings` | Reads and optionally changes physics settings: gravity [x,y,z], debug_draw (collider wireframes), queries_hit_triggers, ignore_layer_collision [layerA, layerB, ignore] | yes |
| `create_prefab` | Saves an object and its children as a prefab asset (e.g | yes |
| `instantiate_prefab` | Unity Instantiate(prefab): creates a linked instance of a prefab file, optionally under `parent` and at `position`/`euler_angles` (world space) | yes |
| `get_prefab_overrides` | How a prefab instance differs from its prefab (changed fields and properties, added/removed components and objects) | no |
| `apply_prefab` | Unity Overrides > Apply All: writes the instance back into its prefab file; other instances in the scene update and keep their own overrides | yes (destructive) |
| `revert_prefab` | Unity Overrides > Revert All: makes the instance match its prefab again (it stays where it is) | yes |
| `unpack_prefab` | Unity Unpack Completely: the instance becomes plain objects, no longer linked to the prefab | yes |
| `create_prefab_variant` | Saves a prefab instance as a variant of its prefab at `path`: a prefab that inherits from the base and keeps the instance's overrides | yes |
| `audio_info` | Audio device availability, playing voices, master volume and loaded clips | yes |
| `network` | Multiplayer status (mode, client id, clients, RTT, spawned network objects); optional `start` host/server/client or `stop` | yes |
| `occlusion` | Occlusion culling stats; with `camera_position` renders the view with occlusion on and off and reports both plus `pixel_difference`; `image` = buffer / render / difference | no |
| `navmesh` | AI Navigation: `status`; `bake` (background, saves `.navmesh` files) / `build` (blocking, not saved) / `cancel` / `clear`; queries `calculate_path`, `sample_position`, `raycast`, `closest_edge` (with `agent_type`, `area_mask`); `agents` lists every NavMeshAgent; `agent` sets a destination, stops/resumes, warps or resets one | no |
| `scenes` | Loaded scenes (handle, name, path, objects), pending background operations and WorldStreamer cell states; optional `load` (additive by default, `background` for LoadSceneAsync), `unload`, `save` | yes |
| `split_scene_into_cells` | Moves the main scene's (static) root objects into one scene per `cell_size` grid cell, saves them and adds a WorldStreamer | yes (destructive) |
| `play_sound` | Plays an audio clip once: at a world `position` (3D, like AudioSource.PlayClipAtPoint) or 2D if omitted | yes |
| `animator` | Reads an object's Animator (layers with current state, normalized time and weight; parameters; root motion velocity; humanoid bone mapping) and optionally changes it: `parameters` {name: number\|bool}, `trigger`, `play`, `cross_fade` [state, seconds], `speed`, `layer_weights`, `apply_root_motion`, `ik_pass` {layer: bool}, `ik` {LeftHand\|RightHand\|LeftFoot\|RightFoot: [x,y,z] or null}, `look_at` [x,y,z] or null | yes |
| `timeline` | Lists every PlayableDirector (asset, state, time, duration, bindings), or controls one: `object`, `action` play \| pause \| resume \| stop \| evaluate, `time` | yes |
| `constraints` | Lists constrained objects, or on one `object` with `kind` (Position \| Rotation \| Scale \| Parent \| Aim \| LookAt): `action` add \| add_source (`source`, `weight`) \| activate \| zero | yes |
| `asset_database` | AssetDatabase: `action` refresh \| guid (`path`) \| path (`guid`) \| find (`filter` with `t:Type`, `folders`) \| dependencies (`path`, `recursive`) \| dependents \| list | yes |
| `import_settings` | Model / Audio import settings of `path` (kept in its .meta); `settings` changes fields and re-imports | yes |
| `asset_packages` | Packages: `action` export (`paths`, `include_dependencies`, `out`) \| list (`package`, with import status) \| import (`package`, optional `paths`) \| open_export \| open_import | yes |
| `visual_effect` | Visual Effects (VFX Graph): lists each effect (asset, playing, time, particles spawned, systems, exposed properties with values, compile errors); with `object`: `action` play/stop/pause/resume/reinit/send_event, `set` {reference: value} overrides (Unity SetFloat/SetVector/SetBool), `reset`, `play_rate`, `asset` | yes |
| `ui_toolkit` | UI Toolkit documents: lists each UI Document with elements (type, name, classes, text, value, rect) and last events; with `object`: `set_text`, `set_value`, `add_class`, `remove_class`, `display`, `style` (by element name), `click` (queues a Click event for scripts) | yes |
| `localization` | Localization: locales, selected locale and String Table Collections (name, path, keys); `set_locale`; `get` {table, key, args?} (fallbacks and Smart Strings); `set` [{table, key, locale, value, smart?}] (creates the table if needed); locales are edited with `project_settings` section `localization` | yes |
| `ragdoll` | Ragdolls on humanoids (Ragdoll Wizard): `create` (Rigidbodies, colliders, CharacterJoints on the mapped bones; `mass`), `activate` (Animator off, bodies fall), `deactivate` | yes |
| `create_terrain` | Creates a generated terrain (noise heights, auto grass/rock/sand/snow texturing, heightfield collider) | yes |
| `terrain_edit` | Unity terrain tools on a Terrain: info, paint (raise, lower, set_height, smooth, stamp, texture, holes, trees, details and erasers) across neighbours, add/remove layers, add tree / detail prototypes, mass place trees, wind, save to `.terraindata` | yes |
| `terrain_height` | Unity Terrain.SampleHeight: world Y of the terrain surface at `x`, `z` (null off-terrain) | no |
| `spawn_effect` | Creates a ParticleSystem from a preset (smoke, fire, explosion, muzzle_flash, dust, rain) at `position` | yes |
| `log_message` | Logs a message like Unity's Debug.Log / LogWarning / LogError (errors also record a stack trace) | yes |
| `project_settings` | Unity Project Settings (tags, layers, sorting layers, physics, quality, navigation agent types and areas, ...), saved in ProjectSettings/*.yaml | yes |
| `asset_types` | Every asset type the project knows (the asset type registry): type name, file extensions, Create menu path, whether it is a data asset (ScriptableObject, edited with `data_asset`) and, for data assets, the fields of a new one | no |
| `data_asset` | Creates, reads or changes a data asset file (Unity ScriptableObject: Physics Material, Volume Profile, Lighting Settings, Input Actions, Tiles, Sprite Atlas, Sprite Library, script-defined data) | yes |
| `input_actions` | Live Input System action states (what scripts read with `ctx.input().action("Move")`): value, phase, pressed and this-frame flags for every action enabled by a PlayerInput component | no |
| `audio_mixer` | An Audio Mixer at runtime (Unity AudioMixer): `set` {"MusicVol": -10} sets exposed parameters (dB for volumes), `clear` removes script values, `snapshot` + `seconds` transitions to a snapshot | yes |
| `tilemap` | Edits a Tilemap (Unity Tilemap API): `set` [{x, y, tile}] sets cells (tile = a .tile/.ruletile/.animatedtile path, or null to erase), `box_fill` {from, to, tile}, `flood_fill` {x, y, tile}, `clear` | yes |
| `quit` | Closes the window and exits | yes (destructive) |
| `editor_get_state` | Editor mode (edit/playing/paused), open scene and unsaved flag, selection, active tool, open windows, undo/redo history and the Scene view camera | no |
| `editor_select` | Sets the editor selection (what the Inspector shows and the handles move) | yes |
| `editor_play` | Unity's Play/Pause/Step buttons | yes |
| `editor_undo` | Undoes (or with `redo: true`, redoes) the last `steps` editor edits (default 1) | yes |
| `editor_execute_menu` | Runs a main-menu item by path, like Unity's EditorApplication.ExecuteMenuItem: e.g | yes |
| `editor_set_tool` | Selects the Scene view tool (hand, move, rotate, scale, rect, transform), handle orientation (global/local) and handle position (pivot/center) | yes |
| `editor_scene_camera` | Positions the Scene view camera: `position` + `look_at`, or `pivot`/`yaw`/`pitch`/`distance`, or `frame` (an object to frame, like pressing F); also `draw_mode`, `mode_2d`, `lighting` and `effects` | yes |
| `editor_scene_visibility` | The Hierarchy's eye and hand toggles: hide/show objects in the Scene view, disable/enable picking, `show_all`, or turn scene visibility off | yes |
| `editor_project` | Project window: list a folder, search (`car t:Prefab l:Vehicle`), select, create folders, rename, move, duplicate, delete, read/set Asset Labels, icon zoom. Moves rewrite references | yes |
| `editor_shortcuts` | Shortcuts Manager: list commands with their shortcuts, defaults and conflicts; set (`Cmd+Shift+K`, optionally reassigning) or reset; saved in Preferences | yes |
| `editor_search` | Search window: find assets, scene objects (`t:Camera`), menu items and settings (`p:` `h:` `m:` `set:`); activate a result or show the popup | yes |
| `editor_ui_input` | Clicks, double/right-clicks, drags, scrolls, key presses and text in the editor UI at window coordinates (the same as `editor_capture` pixels), for testing windows that need the pointer such as the Curve and Gradient Editors | yes |
| `editor_open_window` | Opens (or focuses) an editor window; `asset` opens Input Actions / Sprite Editor / Animator / Audio Mixer on a file | yes |
| `editor_game_view` | Game view control bar: `size` (aspect or fixed resolution), `scale`, maximize on play, stats, gizmos | yes |
| `editor_inspector` | Locks the Inspector to the current selection (`lock`: true) or unlocks it, toggles Debug mode, `scroll_to` a component, and returns which objects the Inspector shows | yes |
| `editor_animation` | The Animation window (it edits the clips of the selected object's Animator): target, clip, playhead, record and preview state; `time`, `record`, `preview`, `clip`, and `add_sprites` (keys a sprite sheet as a 12 fps flipbook) | yes |
| `editor_animator` | The Animator window: open controller, layer, sub-state machine path, blend tree view, selection and every drawn node with its screen rectangle; `open`, `layer`, `machine` (sub-machine names), `open_tree` / `close_tree`, `select` (node name), `select_transition`, `frame` | yes |
| `editor_shader_graph` | The Shader Graph window (`.shadergraph` / `.shadersubgraph`): `open`, `library` (every node type with ports and options); returns nodes, edges, Blackboard properties, keywords and compile errors; edits `add_property`, `set_property`, `add_keyword` (`entries` for Enum, `global`), `settings` (Graph Settings), `add_nodes` [{kind, position?, options?, values?}], `set_nodes`, `connect` [{from, output?, to, input}] (nodes by id, `"$k"` for new ones, or `Fragment`/`Vertex`/`Output`), `disconnect`, `delete`; `save`, `wgsl`, `undo`/`redo`, `frame` | yes |
| `assembly_definitions` | Assembly Definitions: lists assemblies (name, file, crate, references, platforms, scripts) and problems; `create` {folder, name, references?, includePlatforms?, excludePlatforms?, allowUnsafeCode?, autoReferenced?, noEngineReferences?}, `set` {name, rename?, fields}, `asmref` {folder, reference} | yes |
| `probuilder` | ProBuilder: `create` {shape, size?, sides?, segments?/steps?, thickness?, angle?, subdivisions?, smooth?, position?}; `object` selects a ProBuilder object; `probuilderize`; `operation` extrude/inset/subdivide/merge/delete/flip/triangulate (`faces`, `amount`), weld/collapse (`vertices`), translate (`delta`), smoothing, material; `mode` Object/Vertex/Edge/Face with `faces`/`vertices` sets the Scene view selection; returns faces (vertices, normal, center, material) and counts | yes |
| `editor_ui_builder` | The UI Builder (`.uxml` + style sheets): `open`; returns elements (path, type, name, classes, attributes, inline style) and style sheet rules; edits `add` [{type, parent?, name?, classes?, attributes?, style?}], `set` {element, ...}, `delete`, `rule` {selector, styles}; `undo`; `save` (UI Documents reload) | yes |
| `editor_vfx_graph` | The VFX Graph window (`.vfx`): `open`, `library` (contexts with settings, blocks with inputs/options, operators); returns contexts with their blocks, operators, edges, properties and compile errors; edits `add_property`, `add_nodes` (contexts or operators), `settings` {node, Capacity/Space/Blend Mode/Orient/Main Texture/Flipbook}, `add_blocks` [{context, kind, values?, options?}], `set_blocks`, `remove_blocks`, `connect` (flow: `Flow Out` → `Flow In`; block inputs: `{to: context, block, input}`), `delete`; changes apply to effects in the scene at once; `save` | yes |
| `editor_timeline` | The Timeline window: Director, asset, playhead, preview/record state and every track with clips and markers; `create` (Director + `.playable` on an object), `open`, `director`; edits `add_track` {type, name?, group?} (+ `bind`), `add_clip` {track, source, start, duration?, ease_in?, ease_out?, clip_in?, speed?}, `add_marker` {track?, time, signal}, `set_clip`, `delete`, `bind` + `track`; preview `time`, `preview`, `play`, `record` (track index or false) | yes |
| `editor_scenes` | Multi-scene editing: lists open scenes (active, dirty, roots) and unloaded ones; `action` open_additive \| new \| set_active \| save \| save_as \| save_all \| unload \| remove \| load \| move (`objects` into `scene`) | yes |
| `editor_test_runner` | Unity's Test Runner: play-mode tests (`scripts! { ...; tests: f }`) and edit-mode tests (#[test] in the Scripts crate) with status, seconds and failure messages; `run` all / failed / a name in `mode` PlayMode or EditMode (asynchronous: call again for results) | yes |
| `editor_overlays` | Scene view overlays: list, or set one `id`'s `visible` / `collapsed` / `dock` / `pos`; `preset` applies Default or a saved preset, `save_preset` saves one | yes |
| `editor_preferences` | Reads the per-user editor Preferences; `values` changes some | yes |
| `editor_capture` | PNG of the whole editor window (menus, Hierarchy, Scene/Game views, Inspector, ...), as the user sees it | no |
| `reflection_probes` | Lists Reflection Probes (box, mode, refresh mode, importance) and the last frame's reflection stats (`active` = probes with a captured cubemap, up to 15; `captures` = cubemaps captured that frame, sky included). | yes |
| `create_render_texture` | Creates a `.renderTexture` asset (Unity RenderTexture) for a Camera's `target_texture` and for materials (mirrors, screens, scopes) | yes |
| `texture_import_settings` | Gets a texture's import settings (its `.meta` file, Unity's Texture Importer) and what the import produced (size, mips, GPU format, cube/array shape); `settings` edits them (incl. `texture_shape`, `platform_settings` overrides) and re-imports; `build_target` switches the import platform (Standalone/Android/IOS/WebGL) and re-imports textures. | yes |
| `editor_scripts` | The project's Rust scripts: `status` (compile state, errors, script component types), `recompile`, or `create` a new script component named `name` (Unity Create > Script). | yes |
| `editor_build` | Unity Build Settings: `status` (default), `settings` (read; or change with `scenes`, `product_name`, `width`, `height`, `fullscreen`, `output`), `build` or `build_and_run` (starts in the background; poll `status`). | yes |
| `bake_lighting` | Unity's Lighting window: `start` bakes lightmaps and light probes in the background, `status` (progress, last result), `cancel`, `clear` | yes |

## Try it with curl

```sh
curl -s http://127.0.0.1:47474/mcp -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"set_object_field",
       "arguments":{"name":"Triangle","field":"spin_speed","value":360}}}'
```

## Security

- Binds to loopback only; starting it on a non-loopback address is refused.
- Requests whose `Host` or `Origin` isn't `localhost`/`127.0.0.1`/`::1` get 403, which blocks
  DNS-rebinding attacks from web pages.
- No authentication: any local process can call the tools. Use `--no-mcp` on shared machines.

## Adding a tool

Register it in `crates/oxide_engine/src/mcp_tools.rs`:

```rust
let b = bus.clone();
s.add_tool(
    ToolDef::new("get_fps", "Get FPS", "Current smoothed frame rate.",
                 json!({ "type": "object", "properties": {} })).read_only(),
    move |_args| b.run(|engine| ToolResult::json(json!({ "fps": engine.fps }))),
);
```

Parse and validate arguments in the outer closure (it runs on the HTTP thread), then do engine work
inside `b.run(..)` (it runs on the main thread). Return `ToolResult::error(..)` for problems the
agent can fix; don't panic (a panic is caught and reported as an error anyway).

Rule from the roadmap: **every new engine feature ships with its MCP tools.**
