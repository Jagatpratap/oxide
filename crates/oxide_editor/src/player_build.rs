//! Player builds (Unity's Build Settings → Build): compiles the project's Scripts into a
//! standalone executable with the release engine and packs the assets next to it.
//!
//! The build has its own Cargo workspace in `Library/Build/player-ws`: a copy of
//! `Scripts/src` compiled against `oxide_player` (static, no editor) plus a tiny `main`.
//! Output: `<Product>.app` on macOS (data in `Contents/Resources/Data`), else `<Product>`
//! and `<Product>_Data/` side by side.
//! Reference: <https://docs.unity3d.com/Manual/BuildSettings.html>

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::Instant;

/// Stored in `ProjectSettings/BuildSettings.yaml`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BuildSettings {
    pub product_name: String,
    /// Scenes in the build; the first opens at startup.
    pub scenes: Vec<String>,
    pub width: u32,
    pub height: u32,
    pub fullscreen: bool,
    /// Output folder, relative to the project.
    pub output: String,
}

impl BuildSettings {
    fn path(project: &Path) -> PathBuf {
        project.join("ProjectSettings/BuildSettings.yaml")
    }

    pub fn load(project: &Path) -> Self {
        std::fs::read_to_string(Self::path(project))
            .ok()
            .and_then(|t| serde_yaml_ng::from_str(&t).ok())
            .unwrap_or_else(|| {
                let name = project
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("Game")
                    .to_owned();
                Self {
                    output: format!("Builds/{name}"),
                    product_name: name,
                    scenes: Vec::new(),
                    width: 1280,
                    height: 720,
                    fullscreen: false,
                }
            })
    }

    pub fn save(&self, project: &Path) -> Result<(), String> {
        let p = Self::path(project);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let text = serde_yaml_ng::to_string(self).map_err(|e| e.to_string())?;
        std::fs::write(&p, text).map_err(|e| e.to_string())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum BuildState {
    Idle,
    Building,
    Succeeded(PathBuf),
    Failed(String),
}

pub struct PlayerBuilder {
    pub state: BuildState,
    job: Option<(Receiver<Result<PathBuf, String>>, bool)>,
    started: Option<Instant>,
}

impl Default for PlayerBuilder {
    fn default() -> Self {
        Self {
            state: BuildState::Idle,
            job: None,
            started: None,
        }
    }
}

impl PlayerBuilder {
    pub fn is_building(&self) -> bool {
        self.job.is_some()
    }

    pub fn elapsed(&self) -> f32 {
        self.started.map_or(0.0, |t| t.elapsed().as_secs_f32())
    }

    /// Starts a build in the background; `run` launches the game when it succeeds.
    pub fn start(
        &mut self,
        project: PathBuf,
        settings: BuildSettings,
        run: bool,
    ) -> Result<(), String> {
        if self.job.is_some() {
            return Err("a build is already running".into());
        }
        if settings.scenes.is_empty() {
            return Err(
                "add at least one scene to the build (Build Settings → Add Open Scene)".into(),
            );
        }
        let engine = engine_root(&project)?;
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(build(&project, &engine, &settings));
        });
        self.job = Some((rx, run));
        self.started = Some(Instant::now());
        self.state = BuildState::Building;
        oxide_core::debug_log!(
            "Building player (release; the first build compiles the engine and takes a few minutes)..."
        );
        Ok(())
    }

    /// Call every frame.
    pub fn poll(&mut self) {
        let Some((rx, run)) = &self.job else { return };
        let Ok(result) = rx.try_recv() else { return };
        let run = *run;
        self.job = None;
        match result {
            Ok(exe) => {
                oxide_core::debug_log!(
                    "Build succeeded in {:.0}s: {}",
                    self.elapsed(),
                    exe.display()
                );
                if run {
                    match std::process::Command::new(&exe).spawn() {
                        Ok(_) => oxide_core::debug_log!("Running {}", exe.display()),
                        Err(e) => oxide_core::debug_log_error!("cannot run {}: {e}", exe.display()),
                    }
                }
                self.state = BuildState::Succeeded(exe);
            }
            Err(e) => {
                oxide_core::debug_log_error!("Build failed: {e}");
                self.state = BuildState::Failed(e);
            }
        }
    }
}

/// The engine source tree the project was generated from.
pub fn engine_root(project: &Path) -> Result<PathBuf, String> {
    std::fs::read_to_string(project.join("Library/Build/EngineRoot.txt"))
        .map(|s| PathBuf::from(s.trim()))
        .map_err(|_| "builds need a project opened with `oxide open <dir>`".to_string())
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

fn copy_dir(from: &Path, to: &Path) -> Result<u64, String> {
    let mut bytes = 0;
    std::fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    for e in std::fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))? {
        let e = e.map_err(|e| e.to_string())?;
        let name = e.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let (src, dst) = (e.path(), to.join(&name));
        if src.is_dir() {
            bytes += copy_dir(&src, &dst)?;
        } else {
            bytes += std::fs::copy(&src, &dst).map_err(|e| format!("{}: {e}", src.display()))?;
        }
    }
    Ok(bytes)
}

/// A file-name-safe product name.
fn file_name(product: &str) -> String {
    let s: String = product
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || " -_".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.trim().is_empty() {
        "Game".into()
    } else {
        s.trim().to_owned()
    }
}

/// Runs the whole build. Returns the executable.
fn build(project: &Path, engine: &Path, s: &BuildSettings) -> Result<PathBuf, String> {
    let ws = project.join("Library/Build/player-ws");
    let player_dep = engine
        .join("crates/oxide_player")
        .to_string_lossy()
        .replace('\\', "/");
    write(
        &ws.join("Cargo.toml"),
        r#"# Generated by Oxide for player builds.
[workspace]
resolver = "3"
members = ["player", "scripts"]

[profile.release]
debug = false
"#,
    )?;
    write(
        &ws.join(".cargo/config.toml"),
        "[build]\ntarget-dir = \"../player-target\"\n",
    )?;
    let lock = ws.join("Cargo.lock");
    if !lock.exists() {
        let _ = std::fs::copy(project.join("Cargo.lock"), &lock);
    }
    // Scripts, compiled statically against the player engine.
    let scripts_src = project.join("Scripts/src");
    let has_scripts = scripts_src.join("lib.rs").is_file();
    let _ = std::fs::remove_dir_all(ws.join("scripts/src"));
    if has_scripts {
        copy_dir(&scripts_src, &ws.join("scripts/src"))?;
    } else {
        write(&ws.join("scripts/src/lib.rs"), "oxide::scripts! {}\n")?;
    }
    write(
        &ws.join("scripts/Cargo.toml"),
        &format!(
            r#"[package]
name = "scripts"
version = "0.1.0"
edition = "2024"

[dependencies]
oxide = {{ package = "oxide_player", path = "{player_dep}" }}
"#
        ),
    )?;
    write(
        &ws.join("player/Cargo.toml"),
        &format!(
            r#"[package]
name = "player"
version = "0.1.0"
edition = "2024"

# The texture compressor's prebuilt objects make the macOS linker print a harmless note.
[lints.rust]
linker_messages = "allow"

[dependencies]
oxide = {{ package = "oxide_player", path = "{player_dep}" }}
scripts = {{ path = "../scripts" }}
"#
        ),
    )?;
    write(
        &ws.join("player/src/main.rs"),
        r#"// Generated by Oxide.
#![windows_subsystem = "windows"]

fn main() -> std::process::ExitCode {
    oxide::run(scripts::oxide_register_scripts)
}
"#,
    )?;

    let out = std::process::Command::new(crate::project_files::cargo_command())
        .args([
            "build",
            "--release",
            "-p",
            "player",
            "--message-format=short",
        ])
        .current_dir(&ws)
        .env_remove("CARGO_TARGET_DIR")
        .output()
        .map_err(|e| format!("could not run cargo: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let lines: Vec<&str> = err
            .lines()
            .filter(|l| l.contains("error") || l.starts_with("  -->"))
            .take(20)
            .collect();
        return Err(if lines.is_empty() {
            err.into_owned()
        } else {
            lines.join("\n")
        });
    }
    let built = project
        .join("Library/Build/player-target/release")
        .join(format!("player{}", std::env::consts::EXE_SUFFIX));

    // Package.
    let product = file_name(&s.product_name);
    let out_dir = project.join(&s.output);
    let (exe, data) = if cfg!(target_os = "macos") {
        let app = out_dir.join(format!("{product}.app/Contents"));
        let _ = std::fs::remove_dir_all(out_dir.join(format!("{product}.app")));
        write(
            &app.join("Info.plist"),
            &format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>{name}</string>
  <key>CFBundleExecutable</key><string>{product}</string>
  <key>CFBundleIdentifier</key><string>com.oxide.{id}</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>1.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
"#,
                name = s.product_name,
                id = product.to_lowercase().replace(' ', "-"),
            ),
        )?;
        (app.join("MacOS").join(&product), app.join("Resources/Data"))
    } else {
        let exe = out_dir.join(format!("{product}{}", std::env::consts::EXE_SUFFIX));
        let data = out_dir.join(format!("{product}_Data"));
        let _ = std::fs::remove_dir_all(&data);
        (exe, data)
    };
    if let Some(d) = exe.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::copy(&built, &exe).map_err(|e| format!("cannot copy {}: {e}", built.display()))?;
    copy_dir(&project.join("Assets"), &data.join("Assets"))?;
    if project.join("ProjectSettings").is_dir() {
        copy_dir(
            &project.join("ProjectSettings"),
            &data.join("ProjectSettings"),
        )?;
    }
    let info = format!(
        "product_name: {:?}\nscenes:\n{}width: {}\nheight: {}\nfullscreen: {}\n",
        s.product_name,
        s.scenes
            .iter()
            .map(|p| format!("  - {p:?}\n"))
            .collect::<String>(),
        s.width,
        s.height,
        s.fullscreen
    );
    write(&data.join("build.yaml"), &info)?;
    Ok(exe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_default_and_round_trip() {
        let dir = std::env::temp_dir().join(format!("oxide-build-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = BuildSettings::load(&dir);
        assert!(s.output.starts_with("Builds/"));
        s.scenes.push("Assets/Scenes/Main.scene".into());
        s.save(&dir).unwrap();
        assert_eq!(BuildSettings::load(&dir), s);
        assert_eq!(file_name("My: Racer!"), "My_ Racer_");
        let _ = std::fs::remove_dir_all(dir);
    }
}
