//! Project layout on disk and the files Oxide generates for it (Unity's project template).
//!
//! ```text
//! MyGame/
//!   Assets/                 scenes, prefabs, models, materials, audio...
//!   Scripts/                the game's Rust code (a `dylib` crate named `scripts`)
//!   Cargo.toml              Cargo workspace: Scripts + the editor build
//!   .cargo/config.toml      build output goes to the shared build folder
//!   Library/                generated, safe to delete (Unity's Library folder)
//!     Build/editor/         the project's editor crate (engine source checkouts only)
//!   AGENTS.md, CLAUDE.md    notes for AI coding agents (written once, then the user's)
//!   .claude/skills/oxide/   the Oxide skill for agents, refreshed on every open
//!   .mcp.json, .cursor/mcp.json, .vscode/mcp.json   point MCP clients at this project's editor
//! ```
//!
//! A release ships the editor prebuilt ([`prebuilt`]): opening a project starts
//! `bin/oxide-editor` straight away and the Scripts compile against the release's engine
//! library, so nothing of the engine is compiled. An engine source checkout instead builds
//! each project's editor (`Library/Build/editor`) with Cargo.
//!
//! Every project builds into one shared folder ([`shared_build_dir`]), so with a source
//! checkout the engine is compiled once for all of them. Cargo
//! keeps each project's crates apart by their paths; the Scripts library and the editor
//! executable get per-project names ([`project_id`]) because Cargo doesn't add a hash to
//! those file names.

use std::path::{Path, PathBuf};

/// The engine source tree that projects build against. `$OXIDE_ENGINE_ROOT` if set; else, in
/// an unpacked release (`<root>/bin/oxide`), the folder around the program; else the tree this
/// program was compiled from.
pub fn engine_root() -> PathBuf {
    let is_engine = |p: &Path| p.join("crates/oxide_dylib/Cargo.toml").is_file();
    if let Some(p) = std::env::var_os("OXIDE_ENGINE_ROOT").map(PathBuf::from)
        && is_engine(&p)
    {
        return std::fs::canonicalize(&p).unwrap_or(p);
    }
    if let Ok(exe) = std::env::current_exe().and_then(std::fs::canonicalize)
        && let Some(root) = exe.parent().and_then(Path::parent)
        && is_engine(root)
    {
        return root.to_path_buf();
    }
    let built = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::canonicalize(&built).unwrap_or(built)
}

/// The prebuilt editor of a release: `bin/oxide-editor`, plus the engine library in `lib/`
/// that a project's Scripts compile against through the `sdk/oxide_dylib` crate.
pub struct Prebuilt {
    pub editor: PathBuf,
    /// The crate projects depend on as `oxide` (it re-exports the prebuilt library).
    pub engine_crate: PathBuf,
    /// The Rust version the library was built with (`1.99.0`), a rustup channel.
    pub channel: String,
    /// `rustc --version` of that compiler; Scripts only link when compiled with exactly it.
    pub rustc: String,
}

impl Prebuilt {
    /// The rustup toolchain the project's Scripts compile with (its `rust-toolchain.toml`): an
    /// installed one with exactly this compiler (often `stable-<host>`, saving a download), else the
    /// version itself ([`ensure_toolchain`] installs it).
    pub fn toolchain(&self) -> String {
        let installed = rustup_output(&["toolchain", "list"]).unwrap_or_default();
        for name in installed
            .lines()
            .filter_map(|l| l.split_whitespace().next())
        {
            if rustup_output(&["run", name, "rustc", "--version"])
                .as_deref()
                .map(str::trim)
                == Some(self.rustc.as_str())
            {
                return name.to_owned();
            }
        }
        self.channel.clone()
    }
}

/// rustup's default install (`~/.cargo/bin/rustup`), else the one on `PATH`.
fn rustup_command() -> PathBuf {
    let exe = format!("rustup{}", std::env::consts::EXE_SUFFIX);
    std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo")))
        .or_else(|| std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join(".cargo")))
        .map(|h| h.join("bin").join(&exe))
        .filter(|r| r.is_file())
        .unwrap_or_else(|| exe.into())
}

fn rustup_output(args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(rustup_command())
        .args(args)
        .env_remove("RUSTUP_TOOLCHAIN")
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The prebuilt editor in `engine_root`, if it is a release (`lib/sdk.txt`). `None` for an
/// engine source checkout, whose projects build their own editor.
pub fn prebuilt(engine_root: &Path) -> Option<Prebuilt> {
    let info = std::fs::read_to_string(engine_root.join("lib/sdk.txt")).ok()?;
    let value = |key: &str| {
        info.lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
            .map(|v| v.trim().to_owned())
    };
    let (channel, rustc) = (value("channel")?, value("rustc")?);
    let editor = engine_root.join(format!("bin/oxide-editor{}", std::env::consts::EXE_SUFFIX));
    let engine_crate = engine_root.join("sdk/oxide_dylib");
    (editor.is_file() && engine_crate.join("Cargo.toml").is_file()).then_some(Prebuilt {
        editor,
        engine_crate,
        channel,
        rustc,
    })
}

/// The engine crate a project's Scripts, assemblies and packages use as `oxide`.
pub fn engine_crate(engine_root: &Path) -> PathBuf {
    prebuilt(engine_root)
        .map(|p| p.engine_crate)
        .unwrap_or_else(|| engine_root.join("crates/oxide_dylib"))
}

/// The project's Cargo workspace members besides its assembly crates.
pub fn base_members(dir: &Path) -> Vec<String> {
    let mut m = vec!["Scripts".to_owned()];
    if dir.join(EDITOR_CRATE).join("Cargo.toml").is_file() {
        m.push(EDITOR_CRATE.to_owned());
    }
    m
}

/// The project's own editor crate (engine source checkouts).
const EDITOR_CRATE: &str = "Library/Build/editor";

fn write(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Writes `text` unless the file already holds it (so watchers don't see a change).
fn write_if_changed(path: &Path, text: &str) -> Result<(), String> {
    if std::fs::read_to_string(path).is_ok_and(|old| old == text) {
        Ok(())
    } else {
        write(path, text)
    }
}

fn write_if_missing(path: &Path, text: &str) -> Result<(), String> {
    if path.exists() {
        Ok(())
    } else {
        write(path, text)
    }
}

/// Path written into generated manifests (forward slashes, absolute).
fn manifest_path(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// The folder every project's engine, editor and Scripts build into (Cargo's target
/// directory). `$OXIDE_BUILD_DIR` overrides it.
pub fn shared_build_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("OXIDE_BUILD_DIR") {
        return d.into();
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let base = if cfg!(target_os = "macos") {
        home.map(|h| h.join("Library/Caches/Oxide/Build"))
    } else if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(|a| PathBuf::from(a).join("Oxide/Build"))
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .map(|x| PathBuf::from(x).join("oxide/build"))
            .or_else(|| home.map(|h| h.join(".cache/oxide/build")))
    };
    base.unwrap_or_else(|| std::env::temp_dir().join("oxide-build"))
}

/// A short id for the project folder (a hash of its absolute path), used to give its
/// Scripts library and editor executable names no other project shares. Moving the
/// project changes it; the next open renames them and recompiles only the Scripts.
pub fn project_id(dir: &Path) -> String {
    let dir = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_owned());
    // FNV-1a: stable across Rust versions, unlike `DefaultHasher`.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in manifest_path(&dir).bytes() {
        h = (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:08x}", (h ^ (h >> 32)) as u32)
}

/// Where the project's editor serves MCP: a port picked from the project's path, so two open
/// projects never share one and the project's MCP config files always reach its own editor.
pub fn mcp_addr(dir: &Path) -> String {
    let id = u32::from_str_radix(&project_id(dir), 16).unwrap_or(0);
    format!("127.0.0.1:{}", 47500 + id % 1000)
}

/// Name of the project's editor executable (the binary of `Library/Build/editor`).
pub fn editor_bin_name(dir: &Path) -> String {
    format!("oxide-editor-{}", project_id(dir))
}

/// The project's built editor executable in the shared build folder.
pub fn editor_exe(dir: &Path) -> PathBuf {
    shared_build_dir().join("debug").join(format!(
        "{}{}",
        editor_bin_name(dir),
        std::env::consts::EXE_SUFFIX
    ))
}

/// The library name of the project's Scripts crate (`[lib] name` in
/// `Scripts/Cargo.toml`), which is also its crate name in Rust paths.
pub fn scripts_lib_name(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("Scripts/Cargo.toml"))
        .ok()
        .and_then(|t| lib_name_in(&t))
        .unwrap_or_else(|| "scripts".into())
}

/// `name = "..."` inside the manifest's `[lib]` table.
fn lib_name_in(manifest: &str) -> Option<String> {
    let mut in_lib = false;
    for line in manifest.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_lib = t == "[lib]";
        } else if in_lib
            && let Some(v) = t.strip_prefix("name").map(str::trim_start)
            && let Some(v) = v.strip_prefix('=')
        {
            return Some(v.trim().trim_matches('"').to_owned());
        }
    }
    None
}

/// Points the Scripts manifest's engine dependency at the workspace's (older projects named
/// the engine path there, which went stale when the project moved to another editor version).
fn with_workspace_engine(manifest: &str) -> String {
    let mut out: String = manifest
        .lines()
        .map(|l| {
            let t = l.trim_start();
            let engine_line = t
                .strip_prefix("oxide")
                .is_some_and(|r| r.trim_start().starts_with('='))
                && t.contains("oxide_dylib")
                && t.contains("path");
            if engine_line {
                "oxide = { workspace = true }"
            } else {
                l
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    if manifest.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Sets `[lib] name` in a Scripts manifest, keeping everything else the user wrote.
fn with_lib_name(manifest: &str, name: &str) -> String {
    let entry = format!("name = \"{name}\"");
    let mut out = Vec::new();
    let (mut in_lib, mut seen_lib, mut done) = (false, false, false);
    for line in manifest.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            if in_lib && !done {
                out.push(entry.clone());
                done = true;
            }
            in_lib = t == "[lib]";
            seen_lib |= in_lib;
            out.push(line.to_owned());
            if in_lib {
                out.push(entry.clone());
                done = true;
            }
            continue;
        }
        if in_lib && t.starts_with("name") && t[4..].trim_start().starts_with('=') {
            continue;
        }
        out.push(line.to_owned());
    }
    if !seen_lib {
        out.push(String::new());
        out.push("[lib]".into());
        out.push(entry);
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

/// The `cargo` to run: `$CARGO` (set when running under cargo), else `cargo` on the PATH,
/// else rustup's default install (`~/.cargo/bin/cargo`), so launching from a desktop
/// shortcut without a shell PATH still works.
pub fn cargo_command() -> PathBuf {
    if let Some(c) = std::env::var_os("CARGO") {
        return c.into();
    }
    let exe = format!("cargo{}", std::env::consts::EXE_SUFFIX);
    let on_path = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(&exe).is_file()))
        .unwrap_or(false);
    if !on_path {
        let home = std::env::var_os("CARGO_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo")))
            .or_else(|| std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join(".cargo")));
        if let Some(c) = home
            .map(|h| h.join("bin").join(&exe))
            .filter(|c| c.is_file())
        {
            return c;
        }
    }
    exe.into()
}

/// Installs the Rust toolchain `name` through rustup if it is missing (printing rustup's
/// progress). Without rustup there is nothing to do here: the Scripts build reports a
/// wrong compiler version itself.
pub fn ensure_toolchain(name: &str) -> Result<(), String> {
    let Some(list) = rustup_output(&["toolchain", "list"]) else {
        return Ok(());
    };
    // Listed with the host: `1.99.0-aarch64-apple-darwin`.
    let prefix = format!("{name}-");
    if list
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .any(|t| t == name || t.starts_with(&prefix))
    {
        return Ok(());
    }
    eprintln!("Installing Rust {name} (the version this editor was built with)...");
    let status = std::process::Command::new(rustup_command())
        .args(["toolchain", "install", name, "--profile", "minimal"])
        .stdout(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("cannot run rustup: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "could not install Rust {name}; run `rustup toolchain install {name}` and open the project again"
        ))
    }
}

/// Why `cargo` couldn't start, in words for someone who may not have Rust installed.
pub fn cargo_missing_message(e: &std::io::Error) -> String {
    if e.kind() == std::io::ErrorKind::NotFound {
        "Rust is not installed (cargo was not found). Oxide projects are Rust code: install Rust \
         from https://rustup.rs, then open the project again."
            .into()
    } else {
        format!("cannot run cargo: {e}")
    }
}

/// `PlayerController` → `player_controller`.
pub fn snake_case(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else if c.is_alphanumeric() {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    out
}

/// A valid Rust type name from user input (`player controller` → `PlayerController`).
pub fn type_name(input: &str) -> Option<String> {
    let mut out = String::new();
    for word in input.split(|c: char| !c.is_alphanumeric()) {
        let mut chars = word.chars();
        if let Some(f) = chars.next() {
            out.extend(f.to_uppercase());
            out.extend(chars);
        }
    }
    (!out.is_empty() && out.chars().next().is_some_and(|c| c.is_alphabetic())).then_some(out)
}

const LIB_RS: &str = r#"//! Game scripts. Save any file here and the editor recompiles and reloads them; script
//! components keep their field values. Each script is a component: a struct deriving
//! `Reflect` (its fields show in the Inspector and are saved in scenes) that implements
//! `Component` (Unity's MonoBehaviour messages: start, update, on_collision_enter, ...).

mod spinner;
mod tests;

pub use spinner::Spinner;

// Every script component, with its Add Component menu path (the editor adds a line here
// when you create a script from the Project window), then the play-mode tests.
oxide::scripts! {
    Spinner => "Scripts/Spinner";
    tests: tests::spinner_turns
}
"#;

const TESTS_RS: &str = r#"//! Tests for the Test Runner (Window → General → Test Runner).
//!
//! Play-mode tests (Unity `[UnityTest]`) are `async fn`s listed under `tests:` in lib.rs: each
//! runs in Play mode on a fresh scene, across frames, and fails by panicking. Edit-mode tests
//! (Unity `[Test]`) are ordinary `#[test]` functions.

use oxide::prelude::*;

/// A Spinner turns its object while the game runs.
pub async fn spinner_turns(co: Co) {
    let id = co.with(|ctx| {
        let id = ctx.world.create("Spinning");
        ctx.world.add_component(id, crate::Spinner::default());
        id
    });
    co.wait_for_seconds(0.5).await;
    let yaw = co.with(|ctx| ctx.world.euler_angles(id).y);
    assert!(yaw.abs() > 1.0, "the spinner should have turned: {yaw}");
}

#[cfg(test)]
mod edit_mode {
    #[test]
    fn spinner_defaults_to_90_degrees_per_second() {
        assert_eq!(crate::Spinner::default().speed, 90.0);
    }
}
"#;

const SPINNER_RS: &str = r#"use oxide::prelude::*;

/// Rotates its object around the Y axis.
#[derive(Clone, Reflect)]
pub struct Spinner {
    /// Degrees per second.
    pub speed: f32,
}

impl Default for Spinner {
    fn default() -> Self {
        Self { speed: 90.0 }
    }
}

impl Component for Spinner {
    fn update(&mut self, ctx: &mut Ctx) {
        let angle = self.speed * ctx.delta_time();
        ctx.transform().rotate(Vector3::new(0.0, angle, 0.0));
    }
}
"#;

fn script_template(type_name: &str) -> String {
    format!(
        r#"use oxide::prelude::*;

#[derive(Clone, Default, Reflect)]
pub struct {type_name} {{
    pub speed: f32,
}}

impl Component for {type_name} {{
    /// Called before the first frame update.
    fn start(&mut self, _ctx: &mut Ctx) {{}}

    /// Called once per frame.
    fn update(&mut self, _ctx: &mut Ctx) {{}}
}}
"#
    )
}

/// `ProjectSettings/ProjectVersion.txt`: the engine version that last opened the project.
pub fn project_version_file(dir: &Path) -> PathBuf {
    dir.join("ProjectSettings/ProjectVersion.txt")
}

/// The engine version recorded in the project (`m_EditorVersion: 0.2.0`), if any.
pub fn project_version(dir: &Path) -> Option<String> {
    std::fs::read_to_string(project_version_file(dir))
        .ok()?
        .lines()
        .find_map(|l| l.trim().strip_prefix("m_EditorVersion:"))
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

/// Creates a new project at `dir` (if missing) and writes its build files.
pub fn create_project(dir: &Path, engine_root: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir.join("Assets/Scenes")).map_err(|e| e.to_string())?;
    let scripts = dir.join("Scripts");
    write_if_missing(&scripts.join("src/lib.rs"), LIB_RS)?;
    write_if_missing(&scripts.join("src/spinner.rs"), SPINNER_RS)?;
    write_if_missing(&scripts.join("src/tests.rs"), TESTS_RS)?;
    write_if_missing(
        &dir.join(".gitignore"),
        "/Library/\n/Logs/\n/UserSettings/\n",
    )?;
    // Start from the engine's resolved dependency versions.
    let lock = dir.join("Cargo.lock");
    if !lock.exists() {
        let _ = std::fs::copy(engine_root.join("Cargo.lock"), &lock);
    }
    write_build_files(dir, engine_root)
}

/// (Re)writes the generated workspace files, pointing at the engine at `engine_root`.
pub fn write_build_files(dir: &Path, engine_root: &Path) -> Result<(), String> {
    let prebuilt = prebuilt(engine_root);
    let engine = manifest_path(&engine_crate(engine_root));
    let members = if prebuilt.is_some() {
        r#""Scripts""#
    } else {
        r#""Scripts", "Library/Build/editor""#
    };
    write(
        &dir.join("Cargo.toml"),
        &format!(
            r#"# Generated by Oxide: the project's Cargo workspace (your Scripts + the editor build).
[workspace]
resolver = "3"
members = [{members}]

# Package crates (in Packages/ or Library/PackageCache) use the engine with
# `oxide = {{ workspace = true }}`.
[workspace.dependencies]
oxide = {{ package = "oxide_dylib", path = "{engine}" }}

# Engine and third-party code is optimized with line tables only (small, fast builds);
# your Scripts compile quickly with full debug info.
[profile.dev.package."*"]
opt-level = 2
debug = "line-tables-only"
"#
        ),
    )?;
    let target = shared_build_dir();
    write(
        &dir.join(".cargo/config.toml"),
        &format!(
            "# Generated by Oxide: every project shares one build folder, so the engine is\n# compiled once.\n[build]\ntarget-dir = \"{}\"\n",
            manifest_path(&target)
        ),
    )?;
    // Projects made before the shared build folder had their own (about 3 GB).
    let old_target = dir.join("Library/Build/target");
    if old_target.is_dir() && old_target != target {
        let _ = std::fs::remove_dir_all(&old_target);
    }
    // The engine version the project was last opened with (Unity's ProjectVersion.txt); the
    // Hub opens the project with that version.
    write(
        &project_version_file(dir),
        &format!("m_EditorVersion: {}\n", oxide_core::ENGINE_VERSION),
    )?;
    write_agent_files(dir)?;
    // A prebuilt engine library only links with the compiler that built it.
    let toolchain = dir.join("rust-toolchain.toml");
    match &prebuilt {
        Some(p) => write(
            &toolchain,
            &format!(
                "# Generated by Oxide: a Rust toolchain with the compiler this editor was built with,\n# {}. Your Scripts compile against its engine library, which needs\n# exactly that compiler.\n[toolchain]\nchannel = \"{}\"\n",
                p.rustc,
                p.toolchain()
            ),
        )?,
        None => {
            if std::fs::read_to_string(&toolchain)
                .is_ok_and(|t| t.starts_with("# Generated by Oxide"))
            {
                let _ = std::fs::remove_file(&toolchain);
            }
        }
    }
    let id = project_id(dir);
    // Player builds need the engine source too.
    write(
        &dir.join("Library/Build/EngineRoot.txt"),
        &manifest_path(engine_root),
    )?;
    write_if_missing(
        &dir.join("Scripts/Cargo.toml"),
        &format!(
            r#"[package]
name = "scripts"
version = "0.1.0"
edition = "2024"

[lib]
name = "scripts_{id}"
crate-type = ["dylib"]

[dependencies]
oxide = {{ workspace = true }}
"#
        ),
    )?;
    // Scripts/Cargo.toml is the user's; only its library name and engine line are kept up
    // to date.
    let manifest = dir.join("Scripts/Cargo.toml");
    if let Ok(text) = std::fs::read_to_string(&manifest) {
        let name = format!("scripts_{id}");
        let mut new = with_workspace_engine(&text);
        if lib_name_in(&new).as_deref() != Some(name.as_str()) {
            new = with_lib_name(&new, &name);
        }
        if new != text {
            write(&manifest, &new)?;
        }
    }
    if prebuilt.is_some() {
        // The release's prebuilt editor runs the project; it has no editor crate of its own.
        let _ = std::fs::remove_dir_all(dir.join(EDITOR_CRATE));
        return Ok(());
    }
    write(
        &dir.join("Library/Build/editor/Cargo.toml"),
        &format!(
            r#"# Generated by Oxide. The project's editor; links the engine library dynamically so
# Scripts can be hot-reloaded into it.
[package]
name = "oxide-project-editor"
version = "0.1.0"
edition = "2024"
publish = false

[[bin]]
name = "oxide-editor-{id}"
path = "src/main.rs"

[dependencies]
oxide = {{ package = "oxide_dylib", path = "{engine}" }}
"#
        ),
    )?;
    write(
        &dir.join("Library/Build/editor/src/main.rs"),
        r#"// Generated by Oxide.
fn main() -> std::process::ExitCode {
    oxide::editor::launch::run_project_editor(oxide::loaded_build_id())
}
"#,
    )
}

const SKILL: &[(&str, &str)] = &[
    ("SKILL.md", include_str!("../../../skills/oxide/SKILL.md")),
    (
        "references/getting-started.md",
        include_str!("../../../docs/GETTING_STARTED.md"),
    ),
    ("references/mcp.md", include_str!("../../../docs/MCP.md")),
];

const AGENTS_MD: &str = r#"# Notes for AI agents

This is an [Oxide](https://github.com/Jagatpratap/oxide) game project: a Unity-style editor
whose game scripts are Rust.

- Read `.claude/skills/oxide/SKILL.md` before changing anything. It covers the project layout,
  how to write and register scripts, and how to drive the editor.
- Game code lives in `Scripts/src/`; every component must be listed in the `oxide::scripts!`
  macro in `Scripts/src/lib.rs`. Saving a file hot-reloads it into the open editor.
- While the editor is open it serves MCP (see `.mcp.json`). Use its tools to change scenes,
  objects, materials and prefabs instead of editing those files by hand, then check
  `get_console_logs` and `capture_screenshot`.
- Don't edit generated files: `Library/`, the root `Cargo.toml`, `.cargo/`,
  `rust-toolchain.toml`, `ProjectSettings/ProjectVersion.txt` and `.claude/skills/oxide/`.

## This game

Describe the game, its conventions and anything an agent should know here.
"#;

/// The files that tell AI coding agents about Oxide: the skill (refreshed every open, so it
/// matches the editor version), `AGENTS.md` / `CLAUDE.md` (written once, then the user's) and
/// MCP client configs pointing at this project's editor (only the `oxide` entry is touched).
fn write_agent_files(dir: &Path) -> Result<(), String> {
    let skill = dir.join(".claude/skills/oxide");
    for (name, text) in SKILL {
        write_if_changed(&skill.join(name), text)?;
    }
    write_if_missing(&dir.join("AGENTS.md"), AGENTS_MD)?;
    write_if_missing(&dir.join("CLAUDE.md"), "@AGENTS.md\n")?;
    let url = format!("http://{}/mcp", mcp_addr(dir));
    let configs = [
        // Claude Code
        (
            ".mcp.json",
            "mcpServers",
            serde_json::json!({ "type": "http", "url": url }),
        ),
        // Cursor
        (
            ".cursor/mcp.json",
            "mcpServers",
            serde_json::json!({ "url": url }),
        ),
        // VS Code (GitHub Copilot)
        (
            ".vscode/mcp.json",
            "servers",
            serde_json::json!({ "type": "http", "url": url }),
        ),
    ];
    for (file, key, entry) in configs {
        let path = dir.join(file);
        let mut doc = match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<serde_json::Value>(&text) {
                Ok(v) if v.is_object() => v,
                // Not JSON we understand (comments, say): leave the user's file alone.
                _ => continue,
            },
            Err(_) => serde_json::json!({}),
        };
        let servers = &mut doc[key];
        if !servers.is_object() {
            *servers = serde_json::json!({});
        }
        servers["oxide"] = entry;
        let text = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())? + "\n";
        write_if_changed(&path, &text)?;
    }
    Ok(())
}

/// Adds a script file and its line in `lib.rs`. Returns the new file.
pub fn add_script(scripts: &Path, type_name: &str) -> Result<PathBuf, String> {
    let snake = snake_case(type_name);
    let file = scripts.join(format!("src/{snake}.rs"));
    if file.exists() {
        return Err(format!("{} already exists", file.display()));
    }
    let lib_path = scripts.join("src/lib.rs");
    let lib = std::fs::read_to_string(&lib_path).map_err(|e| e.to_string())?;
    let Some(block) = lib.find("oxide::scripts!") else {
        return Err("Scripts/src/lib.rs has no `oxide::scripts! { ... }` block".into());
    };
    let Some(open) = lib[block..].find('{').map(|i| block + i) else {
        return Err("malformed `oxide::scripts!` block".into());
    };
    // `mod` + `pub use` go after the last existing ones (else just before the macro); the
    // registration line goes first in the block.
    let head = &lib[..block];
    let after_line = |i: usize| head[i..].find('\n').map_or(head.len(), |n| i + n + 1);
    let insert_at = [head.rfind("\npub use "), head.rfind("\nmod ")]
        .into_iter()
        .flatten()
        .max()
        .map_or(block, |i| after_line(i + 1));
    let mut new = String::new();
    new.push_str(&lib[..insert_at]);
    new.push_str(&format!("mod {snake};\npub use {snake}::{type_name};\n"));
    new.push_str(&lib[insert_at..=open]);
    new.push_str(&format!("\n    {type_name} => \"Scripts/{type_name}\","));
    new.push_str(&lib[open + 1..]);
    write(&file, &script_template(type_name))?;
    write(&lib_path, &new)?;
    Ok(file)
}

/// A script inside an Assembly Definition's folder: the file goes in `folder`, and its
/// registration (`crate::Type => "Scripts/Type"`) into the Scripts `scripts!` block.
pub fn add_assembly_script(
    scripts: &Path,
    folder: &Path,
    crate_name: &str,
    type_name: &str,
) -> Result<PathBuf, String> {
    let file = folder.join(format!("{}.rs", snake_case(type_name)));
    if file.exists() {
        return Err(format!("{} already exists", file.display()));
    }
    let lib_path = scripts.join("src/lib.rs");
    let lib = std::fs::read_to_string(&lib_path).map_err(|e| e.to_string())?;
    let Some(block) = lib.find("oxide::scripts!") else {
        return Err("Scripts/src/lib.rs has no `oxide::scripts! { ... }` block".into());
    };
    let Some(open) = lib[block..].find('{').map(|i| block + i) else {
        return Err("malformed `oxide::scripts!` block".into());
    };
    let mut new = String::new();
    new.push_str(&lib[..=open]);
    new.push_str(&format!(
        "\n    {crate_name}::{type_name} => \"Scripts/{type_name}\","
    ));
    new.push_str(&lib[open + 1..]);
    write(&file, &script_template(type_name))?;
    write(&lib_path, &new)?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(snake_case("PlayerController"), "player_controller");
        assert_eq!(
            type_name("player controller").as_deref(),
            Some("PlayerController")
        );
        assert_eq!(type_name("9lives"), None);
    }

    #[test]
    fn new_project_and_scripts() {
        let dir = std::env::temp_dir().join(format!("oxide-newproj-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        create_project(&dir, Path::new("/engine")).unwrap();
        assert!(dir.join("Scripts/src/spinner.rs").exists());
        let cargo = std::fs::read_to_string(dir.join("Scripts/Cargo.toml")).unwrap();
        assert!(cargo.contains("oxide = { workspace = true }"), "{cargo}");
        let workspace = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap();
        assert!(workspace.contains("/engine/crates/oxide_dylib"));

        add_script(&dir.join("Scripts"), "PlayerController").unwrap();
        let lib = std::fs::read_to_string(dir.join("Scripts/src/lib.rs")).unwrap();
        assert!(lib.contains("pub use spinner::Spinner;\nmod player_controller;\npub use player_controller::PlayerController;\n"), "{lib}");
        assert!(lib.contains("PlayerController => \"Scripts/PlayerController\","));
        assert!(lib.contains("Spinner => \"Scripts/Spinner\";\n    tests: tests::spinner_turns"));
        assert!(dir.join("Scripts/src/tests.rs").exists());
        assert!(add_script(&dir.join("Scripts"), "PlayerController").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn projects_tell_ai_agents_about_oxide() {
        let dir = std::env::temp_dir().join(format!("oxide-agents-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // The user already has another MCP server and their own AGENTS.md.
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(".mcp.json"),
            r#"{"mcpServers":{"github":{"type":"http","url":"https://example.com/mcp"}}}"#,
        )
        .unwrap();
        std::fs::write(dir.join("AGENTS.md"), "my notes\n").unwrap();
        create_project(&dir, Path::new("/engine")).unwrap();

        let skill = std::fs::read_to_string(dir.join(".claude/skills/oxide/SKILL.md")).unwrap();
        // Windows checkouts may have CRLF line endings.
        let skill = skill.replace("\r\n", "\n");
        assert!(skill.starts_with("---\nname: oxide\n"), "{skill}");
        assert!(dir.join(".claude/skills/oxide/references/mcp.md").exists());
        assert_eq!(
            std::fs::read_to_string(dir.join("AGENTS.md")).unwrap(),
            "my notes\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("CLAUDE.md")).unwrap(),
            "@AGENTS.md\n"
        );

        let url = format!("http://{}/mcp", mcp_addr(&dir));
        let read = |f: &str| -> serde_json::Value {
            serde_json::from_str(&std::fs::read_to_string(dir.join(f)).unwrap()).unwrap()
        };
        let claude = read(".mcp.json");
        assert_eq!(claude["mcpServers"]["oxide"]["url"], url.as_str());
        assert_eq!(
            claude["mcpServers"]["github"]["url"],
            "https://example.com/mcp"
        );
        assert_eq!(
            read(".cursor/mcp.json")["mcpServers"]["oxide"]["url"],
            url.as_str()
        );
        assert_eq!(read(".vscode/mcp.json")["servers"]["oxide"]["type"], "http");

        // Another folder gets another port (and the default 47474 stays free).
        assert_ne!(mcp_addr(&dir), mcp_addr(&dir.join("Other")));
        assert_ne!(mcp_addr(&dir), "127.0.0.1:47474");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn scripts_get_a_per_project_library_name() {
        let dir = std::env::temp_dir().join(format!("oxide-libname-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // A project from before the shared build folder, with the user's own additions.
        std::fs::create_dir_all(dir.join("Scripts/src")).unwrap();
        std::fs::create_dir_all(dir.join("Library/Build/target/debug")).unwrap();
        let old = "[package]\nname = \"scripts\"\n\n[lib]\ncrate-type = [\"dylib\"]\n\n[dependencies]\nrand = \"0.8\"\n";
        std::fs::write(dir.join("Scripts/Cargo.toml"), old).unwrap();
        write_build_files(&dir, Path::new("/engine")).unwrap();

        let id = project_id(&dir);
        assert_eq!(scripts_lib_name(&dir), format!("scripts_{id}"));
        let cargo = std::fs::read_to_string(dir.join("Scripts/Cargo.toml")).unwrap();
        assert!(cargo.contains("rand = \"0.8\""), "{cargo}");
        assert!(cargo.contains("crate-type = [\"dylib\"]"), "{cargo}");
        assert!(!dir.join("Library/Build/target").exists());
        let editor = std::fs::read_to_string(dir.join("Library/Build/editor/Cargo.toml")).unwrap();
        assert!(editor.contains(&format!("name = \"oxide-editor-{id}\"")));
        let config = std::fs::read_to_string(dir.join(".cargo/config.toml")).unwrap();
        assert!(config.contains(&manifest_path(&shared_build_dir())));

        // Unchanged on the next open (so the Scripts don't look edited and rebuild).
        let before = std::fs::metadata(dir.join("Scripts/Cargo.toml"))
            .unwrap()
            .modified()
            .unwrap();
        write_build_files(&dir, Path::new("/engine")).unwrap();
        let after = std::fs::metadata(dir.join("Scripts/Cargo.toml"))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(before, after);

        // Two folders never share an id.
        assert_ne!(project_id(&dir), project_id(&dir.join("Other")));
        assert_eq!(
            with_lib_name("[package]\nname = \"x\"\n", "s"),
            "[package]\nname = \"x\"\n\n[lib]\nname = \"s\"\n"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A release folder with a prebuilt editor (what scripts/build_sdk.py adds).
    fn fake_release(root: &Path) {
        std::fs::create_dir_all(root.join("lib")).unwrap();
        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::fs::create_dir_all(root.join("sdk/oxide_dylib")).unwrap();
        std::fs::write(
            root.join("lib/sdk.txt"),
            "rustc=rustc 0.0.1 (000000000 2000-01-01)\nchannel=0.0.1\ntarget=x\n",
        )
        .unwrap();
        std::fs::write(
            root.join(format!("bin/oxide-editor{}", std::env::consts::EXE_SUFFIX)),
            "",
        )
        .unwrap();
        std::fs::write(root.join("sdk/oxide_dylib/Cargo.toml"), "").unwrap();
    }

    #[test]
    fn release_projects_use_the_prebuilt_editor() {
        let tmp = std::env::temp_dir().join(format!("oxide-prebuilt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let (release, source) = (tmp.join("release"), tmp.join("source"));
        fake_release(&release);
        std::fs::create_dir_all(&source).unwrap();
        assert!(prebuilt(&source).is_none());
        let p = prebuilt(&release).expect("release");
        assert_eq!(p.channel, "0.0.1");
        // No installed toolchain has this compiler, so the version itself is pinned.
        assert_eq!(p.toolchain(), "0.0.1");

        // A project made by an editor built from source, before Scripts used the workspace's
        // engine line.
        let dir = tmp.join("Game");
        create_project(&dir, &source).unwrap();
        let manifest = dir.join("Scripts/Cargo.toml");
        let old = std::fs::read_to_string(&manifest).unwrap().replace(
            "oxide = { workspace = true }",
            "oxide = { package = \"oxide_dylib\", path = \"/old/crates/oxide_dylib\" }\nrand = \"0.8\"",
        );
        std::fs::write(&manifest, old).unwrap();
        assert!(dir.join("Library/Build/editor/Cargo.toml").is_file());

        // Opened with the release: no editor crate, the prebuilt engine, the pinned compiler.
        write_build_files(&dir, &release).unwrap();
        assert!(!dir.join("Library/Build/editor").exists());
        let workspace = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap();
        assert!(workspace.contains("members = [\"Scripts\"]"), "{workspace}");
        assert!(workspace.contains(&manifest_path(&release.join("sdk/oxide_dylib"))));
        let scripts = std::fs::read_to_string(&manifest).unwrap();
        assert!(
            scripts.contains("oxide = { workspace = true }\nrand = \"0.8\""),
            "{scripts}"
        );
        assert!(!scripts.contains("/old/"));
        let toolchain = std::fs::read_to_string(dir.join("rust-toolchain.toml")).unwrap();
        assert!(toolchain.contains("channel = \"0.0.1\""), "{toolchain}");
        assert_eq!(base_members(&dir), ["Scripts"]);

        // Back to the source build: its own editor again, and no pinned compiler.
        write_build_files(&dir, &source).unwrap();
        assert!(!dir.join("rust-toolchain.toml").exists());
        assert_eq!(base_members(&dir), ["Scripts", "Library/Build/editor"]);
        let _ = std::fs::remove_dir_all(tmp);
    }
}
