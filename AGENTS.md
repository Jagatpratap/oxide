# Notes for AI agents working on the Oxide engine

This repository is the engine and editor. To build a game with Oxide instead, read
[`skills/oxide/SKILL.md`](skills/oxide/SKILL.md); every project gets a copy of it.

- [CONTRIBUTING.md](CONTRIBUTING.md) explains the layout of `crates/` and `apps/`, the
  architecture rules, tests and style. [docs/ROADMAP.md](docs/ROADMAP.md) tracks what's done.
- Oxide follows Unity: use Unity's names and behaviour for features, menus and APIs unless
  there's a reason not to.
- Every engine feature ships with its MCP tools (`crates/oxide_engine/src/mcp_tools.rs`, editor
  tools in `crates/oxide_editor/src/mcp.rs`) and a row in [docs/MCP.md](docs/MCP.md).
- Before committing: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`
  and `cargo test --workspace`. Package crates have their own runner:
  `cargo test -p oxide_blenderkit --test package`.
- Check UI and rendering changes in a running editor (`cargo run -p oxide`), driving it through
  its MCP server at `http://127.0.0.1:47474/mcp` (`scripts/mcp.py TOOL '{json}'` calls a tool).
- `skills/oxide/SKILL.md`, `docs/GETTING_STARTED.md` and `docs/MCP.md` are compiled into the
  editor and written into every project, so keep them accurate.
