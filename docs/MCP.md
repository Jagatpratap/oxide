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

51 tools (generated from the running editor's `tools/list`). Tools named `editor_*` exist
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
| `play_sound` | Plays an audio clip once: at a world `position` (3D, like AudioSource.PlayClipAtPoint) or 2D if omitted | yes |
| `animator` | Reads an object's Animator (states, parameters, current state) and optionally changes it: `parameters` {name: number\|bool} (SetFloat/SetBool), `trigger` name (SetTrigger), `play` state name, `cross_fade` [state, seconds], `speed` | yes |
| `create_terrain` | Creates a generated terrain (noise heights, auto grass/rock/sand/snow texturing, heightfield collider) | yes |
| `terrain_height` | Unity Terrain.SampleHeight: world Y of the terrain surface at `x`, `z` (null off-terrain) | no |
| `spawn_effect` | Creates a ParticleSystem from a preset (smoke, fire, explosion, muzzle_flash, dust, rain) at `position` | yes |
| `quit` | Closes the window and exits | yes (destructive) |
| `editor_get_state` | Editor mode (edit/playing/paused), open scene and unsaved flag, selection, active tool, open windows, undo/redo history and the Scene view camera | no |
| `editor_select` | Sets the editor selection (what the Inspector shows and the handles move) | yes |
| `editor_play` | Unity's Play/Pause/Step buttons | yes |
| `editor_undo` | Undoes (or with `redo: true`, redoes) the last `steps` editor edits (default 1) | yes |
| `editor_execute_menu` | Runs a main-menu item by path, like Unity's EditorApplication.ExecuteMenuItem: e.g | yes |
| `editor_set_tool` | Selects the Scene view tool (hand, move, rotate, scale) and handle orientation (global/local) | yes |
| `editor_scene_camera` | Positions the Scene view camera: `position` + `look_at`, or `pivot`/`yaw`/`pitch`/`distance`, or `frame` (an object to frame, like pressing F) | yes |
| `editor_capture` | PNG of the whole editor window (menus, Hierarchy, Scene/Game views, Inspector, ...), as the user sees it | no |

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
