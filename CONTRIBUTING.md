# Contributing to Oxide

Thanks for helping. Bug reports, feature ideas, docs fixes and code are all welcome.

- [Ways to help](#ways-to-help)
- [Set up](#set-up)
- [How the code is organised](#how-the-code-is-organised)
- [Making a change](#making-a-change)
- [Tests](#tests)
- [Style](#style)
- [Pull requests](#pull-requests)
- [Releasing](#releasing)

## Ways to help

- **Report a bug** with the [bug report form](https://github.com/Jagatpratap/oxide/issues/new?template=bug_report.yml).
  Include your OS, GPU, Oxide version (`oxide --version`) and the editor log.
- **Ask for a feature**, ideally naming the Unity feature it matches and linking its manual page.
- **Pick up an item** from the [roadmap](docs/ROADMAP.md): ☐ is not started, ◐ is partly done.
  Comment on or open an issue first for anything large, so work isn't duplicated.
- **Improve the docs**: the README, [Getting started](docs/GETTING_STARTED.md) and the
  [MCP reference](docs/MCP.md).
- **Write a package**: editor tools and content can live outside the engine (see the
  [BlenderKit package](packages/com.oxide.blenderkit/README.md)).

## Set up

1. Install Rust 1.88+ from [rustup.rs](https://rustup.rs). On Linux, also install
   `pkg-config libxkbcommon-dev libwayland-dev libx11-dev libasound2-dev libdbus-1-dev libudev-dev`.
2. Clone and run:

   ```sh
   git clone https://github.com/Jagatpratap/oxide.git
   cd oxide
   cargo run -p oxide -- --sample vehicle   # the editor on a sample scene
   cargo test --workspace
   ```

3. To try a change in a real project, create one from your checkout:
   `cargo run -p oxide -- new /tmp/TestGame`. It builds against your working tree.

**Disk space:** a full debug build with tests uses several GB in `target/`, and old artifacts
pile up. If you run low, delete `target/debug/incremental` (or run `cargo clean`); both are
safe.

## How the code is organised

The [README's layout table](README.md#repository-layout) lists every crate. The ones you'll
touch most:

| Crate | Holds |
|---|---|
| `oxide_scene` | GameObjects, components, the lifecycle, scenes, prefabs, `Ctx` (the scripting API) |
| `oxide_assets` | The asset server, importers, materials, the AssetDatabase |
| `oxide_render` | The renderer and its WGSL shaders (`src/shaders/`) |
| `oxide_engine` | The player loop, extraction from the scene into render data, MCP tools (`mcp_tools.rs`) |
| `oxide_editor` | Every editor window; `lib.rs` holds `EditorState` and the `Action`s windows queue |

A few principles run through the code:

- **Unity is the spec.** Names, defaults and behaviour follow Unity's (`local_position`,
  `on_trigger_enter`, "Order in Layer"). When Unity's behaviour is surprising, match it and
  explain why in a comment. Module docs link the Unity manual pages they implement.
- **Everything is reflected.** Components derive `Reflect`, so the Inspector, scene files,
  Undo, prefab overrides and MCP all work from the same field list. Prefer adding a reflected
  field over special-casing a window.
- **Edits go through Undo.** Editor code calls `st.record(world, "Label", ...)` before changing
  the world, so every edit is one Undo step.
- **Agents can do what people can.** A new editor feature usually gets an MCP tool or action,
  documented in [docs/MCP.md](docs/MCP.md).

## Making a change

1. Branch from `main`.
2. Make the change, with tests (see below).
3. Update the docs it affects: the [roadmap](docs/ROADMAP.md) status, [MCP.md](docs/MCP.md) for
   tools, the guides for anything user-facing.
4. Add a line under **Unreleased** in [CHANGELOG.md](CHANGELOG.md) for anything a user would
   notice. Say clearly if it breaks project files or the scripting API.
5. Run the checks CI runs:

   ```sh
   cargo fmt --all
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   ```

6. For editor changes, try them in the running editor. The MCP tools help here:
   `scripts/mcp.py editor_capture --png shot.png` takes a screenshot, and `editor_ui_input`
   clicks and drags like a user.

## Tests

- Unit tests live next to the code (`#[cfg(test)] mod tests`); larger scenarios live in each
  crate's `tests/` folder.
- **GPU tests** render real frames and compare pixels. They skip themselves when no GPU
  adapter is available (as on Linux CI), so run them locally before changing the renderer.
- Shaders are validated by naga in the tests; Windows CI also catches Direct3D-only shader
  problems.
- The BlenderKit package's tests run separately, because Cargo can't link the engine library
  into them inside a workspace test run:
  `cargo test -p oxide_blenderkit --test package` (add `-- --ignored` for the live API tests).
- Avoid tests that need the network or a specific machine; mark those `#[ignore]` with a
  reason.

## Style

- `cargo fmt` formatting and no clippy warnings.
- Comments explain *why* and what Unity does, in plain sentences; the code says *what*.
- User-facing text (editor labels, errors, docs) is short and plain: say what happened and
  what to do next.
- Keep new dependencies few. Prefer what's already in `Cargo.lock`, and pure-Rust crates.
- Don't copy Unity's code, assets, icons or documentation text; describe the behaviour in your
  own words and link the manual page.

## Pull requests

- One topic per pull request, with a description of what changed and how you tested it
  (screenshots for UI changes).
- CI must pass on macOS, Linux and Windows.
- Contributions are licensed under MIT OR Apache-2.0, like the rest of Oxide (see
  [License](README.md#license)).

## Releasing

Maintainers publish releases from tags; the
[release workflow](.github/workflows/release.yml) does the rest.

1. In `CHANGELOG.md`, rename **Unreleased** to the new version and date
   (`## [0.2.0] - 2026-11-01`), add a fresh empty **Unreleased** above it, and update the
   compare links at the bottom.
2. Set `version` in the root `Cargo.toml` (`[workspace.package]`), run `cargo build` so
   `Cargo.lock` updates, and commit.
3. Tag and push:

   ```sh
   git tag v0.2.0
   git push origin main v0.2.0
   ```

4. The workflow builds macOS (Apple Silicon and Intel), Linux and Windows downloads, each with
   the engine source and prebuilt `oxide` and `oxide-mcp-stdio`, and publishes a GitHub Release
   whose notes come from the changelog section. Tags with a hyphen (`v0.2.0-beta.1`) become
   pre-releases.

To test packaging without publishing, run the workflow from the Actions tab
(**Release → Run workflow**); the downloads appear as build artifacts. Locally:

```sh
cargo build --release -p oxide -p oxide_mcp_stdio
python3 scripts/package_release.py --target <your target triple> --version v0.0.0-test --bin-dir target/release
```
