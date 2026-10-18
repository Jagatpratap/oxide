//! The editor's side of the Oxide Hub. The Hub itself is its own program (`apps/oxide_hub`,
//! `oxide-hub`): it installs editor versions and opens projects with them.
//!
//! The project list is kept per user in `<config>/Hub.yaml` (see [`crate::prefs::config_dir`]),
//! shared with the Hub. Every project editor adds itself to it at startup, so projects
//! opened with `oxide open` show up in the Hub too. Keep the fields in step with
//! `oxide_hub::projects::HubData`.

use crate::prefs;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------------------
// The project list on disk.

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct HubProject {
    pub path: PathBuf,
    /// Seconds since the Unix epoch when the project was last opened.
    #[serde(default)]
    pub opened: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HubData {
    #[serde(default)]
    pub projects: Vec<HubProject>,
    /// Folder the last new project was created in (the New project page's Location).
    #[serde(default)]
    pub location: Option<PathBuf>,
    /// The Hub executable, so a project's editor can bring the Hub back (File > Open Project).
    #[serde(default)]
    pub hub_exe: Option<PathBuf>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl HubData {
    pub fn path() -> PathBuf {
        prefs::config_dir().join("Hub.yaml")
    }

    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_yaml_ng::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        self.save_to(&Self::path())
    }

    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let text = serde_yaml_ng::to_string(self).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| e.to_string())
    }

    /// Puts `dir` at the top of the list, marked as opened now.
    pub fn remember(&mut self, dir: &Path) {
        let dir = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_owned());
        self.projects.retain(|p| p.path != dir);
        self.projects.insert(
            0,
            HubProject {
                path: dir,
                opened: now(),
            },
        );
    }

    pub fn forget(&mut self, dir: &Path) {
        self.projects.retain(|p| p.path != dir);
    }
}

/// Adds `dir` to the Hub's list (called by a project's editor at startup).
pub fn remember_project(dir: &Path) {
    let mut data = HubData::load();
    data.remember(dir);
    let _ = data.save();
}

/// The Hub program: the one that last ran (`hub_exe`), else one next to this program (a
/// source build), else the macOS app in Applications.
pub fn hub_exe() -> Option<PathBuf> {
    let exe = format!("oxide-hub{}", std::env::consts::EXE_SUFFIX);
    let mut candidates: Vec<PathBuf> = HubData::load().hub_exe.into_iter().collect();
    if let Ok(me) = std::env::current_exe()
        && let Some(dir) = me.parent()
    {
        candidates.push(dir.join(&exe));
    }
    if cfg!(target_os = "macos") {
        let app = "Oxide Hub.app/Contents/MacOS/oxide-hub";
        candidates.push(Path::new("/Applications").join(app));
        if let Some(h) = std::env::var_os("HOME") {
            candidates.push(PathBuf::from(h).join("Applications").join(app));
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Starts the Hub (the editor's File > New Project / Open Project, like Unity).
pub fn open_hub() -> Result<(), String> {
    let exe = hub_exe().ok_or(
        "the Oxide Hub isn't installed: download it from the Oxide releases page \
         (https://github.com/Jagatpratap/oxide/releases)",
    )?;
    Command::new(&exe)
        .stdin(Stdio::null())
        .spawn()
        .map(drop)
        .map_err(|e| format!("cannot start {}: {e}", exe.display()))
}

/// Whether `dir` holds an Oxide project (a Unity project has `Assets/`; ours also `Scripts/`).
pub fn is_project(dir: &Path) -> bool {
    dir.join("Assets").is_dir() || dir.join("Scripts/Cargo.toml").is_file()
}

/// Shows a folder in Finder / Explorer (Unity Hub's "Show in Explorer").
pub fn show_folder(dir: &Path) -> Result<(), String> {
    let mut cmd = if cfg!(target_os = "macos") {
        Command::new("open")
    } else if cfg!(windows) {
        Command::new("explorer")
    } else {
        Command::new("xdg-open")
    };
    cmd.arg(dir)
        .spawn()
        .map(drop)
        .map_err(|e| format!("cannot open the file browser: {e}"))
}

/// The system's folder chooser (blocks until the user picks a folder or cancels).
pub(crate) fn pick_folder(prompt: &str, start: Option<&Path>) -> Option<PathBuf> {
    let out = if cfg!(target_os = "macos") {
        let mut script = format!("POSIX path of (choose folder with prompt \"{prompt}\"");
        if let Some(s) = start.filter(|s| s.is_dir()) {
            script += &format!(
                " default location (POSIX file \"{}\")",
                s.display().to_string().replace('"', "")
            );
        }
        script.push(')');
        Command::new("osascript").args(["-e", &script]).output()
    } else if cfg!(windows) {
        let script = format!(
            "Add-Type -AssemblyName System.Windows.Forms; \
             $d = New-Object System.Windows.Forms.FolderBrowserDialog; \
             $d.Description = '{prompt}'; \
             if ($d.ShowDialog() -eq 'OK') {{ $d.SelectedPath }}"
        );
        Command::new("powershell")
            .args(["-NoProfile", "-STA", "-Command", &script])
            .output()
    } else {
        Command::new("zenity")
            .args(["--file-selection", "--directory", "--title", prompt])
            .output()
            .or_else(|_| {
                Command::new("kdialog")
                    .args(["--getexistingdirectory", "."])
                    .output()
            })
    };
    let out = out.ok().filter(|o| o.status.success())?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    match text.trim_end_matches(['/', '\\']) {
        "" if text.is_empty() => None,
        "" => Some(PathBuf::from(&text)),
        t => Some(PathBuf::from(t)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("oxide-hub-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn project_list_round_trips_and_orders_by_last_opened() {
        let d = temp("list");
        let (a, b) = (d.join("A"), d.join("B"));
        for p in [&a, &b] {
            std::fs::create_dir_all(p.join("Assets")).unwrap();
        }
        let mut data = HubData::default();
        data.remember(&a);
        data.remember(&b);
        data.remember(&a);
        assert_eq!(data.projects.len(), 2);
        assert!(data.projects[0].path.ends_with("A"));
        let file = d.join("Hub.yaml");
        data.save_to(&file).unwrap();
        let back = HubData::load_from(&file);
        assert_eq!(back.projects, data.projects);
        let mut back = back;
        back.forget(&data.projects[1].path);
        assert_eq!(back.projects.len(), 1);
        assert!(is_project(&a));
        assert!(!is_project(&d));
    }
}
