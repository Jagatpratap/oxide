//! Project layout on disk and the files Oxide generates for it (Unity's project template).
//!
//! ```text
//! MyGame/
//!   Assets/                 scenes, prefabs, models, materials, audio...
//!   Scripts/                the game's Rust code (a `dylib` crate named `scripts`)
//!   Cargo.toml              Cargo workspace: Scripts + the editor build
//!   .cargo/config.toml      build output goes to Library/Build/target
//!   Library/                generated, safe to delete (Unity's Library folder)
//!     Build/editor/         the project's editor executable
//! ```

use std::path::{Path, PathBuf};

fn write(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
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

pub use spinner::Spinner;

// Every script component, with its Add Component menu path. The editor adds a line here
// when you create a script from the Project window.
oxide::scripts! {
    Spinner => "Scripts/Spinner",
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

/// Creates a new project at `dir` (if missing) and writes its build files.
pub fn create_project(dir: &Path, engine_root: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir.join("Assets/Scenes")).map_err(|e| e.to_string())?;
    let scripts = dir.join("Scripts");
    write_if_missing(&scripts.join("src/lib.rs"), LIB_RS)?;
    write_if_missing(&scripts.join("src/spinner.rs"), SPINNER_RS)?;
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
    let engine = manifest_path(&engine_root.join("crates/oxide_dylib"));
    write(
        &dir.join("Cargo.toml"),
        r#"# Generated by Oxide: the project's Cargo workspace (your Scripts + the editor build).
[workspace]
resolver = "3"
members = ["Scripts", "Library/Build/editor"]

# Engine and third-party code is optimized with line tables only (small, fast builds);
# your Scripts compile quickly with full debug info.
[profile.dev.package."*"]
opt-level = 2
debug = "line-tables-only"
"#,
    )?;
    write(
        &dir.join(".cargo/config.toml"),
        "[build]\ntarget-dir = \"Library/Build/target\"\n",
    )?;
    write_if_missing(
        &dir.join("Scripts/Cargo.toml"),
        &format!(
            r#"[package]
name = "scripts"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["dylib"]

[dependencies]
oxide = {{ package = "oxide_dylib", path = "{engine}" }}
"#
        ),
    )?;
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
        assert!(cargo.contains("/engine/crates/oxide_dylib"));

        add_script(&dir.join("Scripts"), "PlayerController").unwrap();
        let lib = std::fs::read_to_string(dir.join("Scripts/src/lib.rs")).unwrap();
        assert!(lib.contains("pub use spinner::Spinner;\nmod player_controller;\npub use player_controller::PlayerController;\n"), "{lib}");
        assert!(lib.contains("PlayerController => \"Scripts/PlayerController\","));
        assert!(lib.contains("Spinner => \"Scripts/Spinner\","));
        assert!(add_script(&dir.join("Scripts"), "PlayerController").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
