# Oxide

A native Rust game engine and editor modeled on Unity's features and workflow, with a built-in
MCP server so AI agents can drive it.

- **Roadmap:** [`docs/ROADMAP.md`](docs/ROADMAP.md). Current phase: **P0 (foundation) done**, P1 next.
- **Unity feature checklist (3,604 manual pages):** [`docs/UNITY_FEATURE_CHECKLIST.md`](docs/UNITY_FEATURE_CHECKLIST.md)
- **MCP:** [`docs/MCP.md`](docs/MCP.md)

## Run

```sh
cargo run -p oxide          # window + spinning triangle, MCP on http://127.0.0.1:47474/mcp
cargo test --workspace
```

Options: `--size 1600x900`, `--mcp-addr 127.0.0.1:5000`, `--no-mcp`.

## Layout

| Path | What |
|---|---|
| `crates/oxide_math` | Unity math: `Vector3`, `Quaternion`, `Matrix4x4`, `Mathf`, `Bounds`, `Ray`, `Plane`, `Rect`, `Color` (glam-backed; left-handed, Y-up, degrees) |
| `crates/oxide_core` | `Time` (fixed timestep, time scale), `Debug.Log` Console buffer, reflection |
| `crates/oxide_render` | wgpu device/surface, test triangle renderer, frame capture to PNG |
| `crates/oxide_mcp` | MCP protocol + localhost HTTP transport |
| `crates/oxide_engine` | Window, main loop (FixedUpdate → Update → LateUpdate → render), MCP tools |
| `apps/oxide` | The `oxide` binary |
| `apps/oxide_mcp_stdio` | `oxide-mcp-stdio` bridge for stdio-only MCP clients |

Oxide copies Unity's concepts and layout, not its name, logos, icons or assets.
