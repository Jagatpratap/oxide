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

## Tools (P0)

| Tool | What it does | Changes state |
|---|---|---|
| `get_engine_info` | Version, GPU/backend, window size, FPS, paused, Console counts | no |
| `get_console_logs` | Console entries; filter by `min_type`, page with `after_seq` | no |
| `clear_console` | Clears the Console | yes |
| `debug_log` | Writes a Log/Warning/Error to the Console | yes |
| `get_time` | Unity `Time` values + FixedUpdate/Update/LateUpdate call counts | no |
| `set_time` | `time_scale`, `fixed_delta_time`, `maximum_delta_time` | yes |
| `set_paused` | Pause/resume the game loop (rendering continues) | yes |
| `step_frames` | While paused, run N frames of 1/60 s | yes |
| `capture_screenshot` | Renders the current frame and returns a PNG | no |
| `list_objects` | Objects in the scene (`Main Camera`, `Triangle` in P0) | no |
| `inspect_object` | All fields with types and values, like the Inspector | no |
| `set_object_field` | Set one field; vectors as `[x,y,z]`/`{"x":..}`, colors as `[r,g,b,a]` or `"#RRGGBB"` | yes |
| `list_types` | Types registered for reflection and their fields | no |
| `quit` | Closes the engine | yes (destructive) |

Tools that touch engine state run on the main thread between frames, so they always see a
consistent frame. A call waits up to 15 s for the engine.

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
