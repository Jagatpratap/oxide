//! The project list and opening projects.
//!
//! The list is `<config>/Hub.yaml`, shared with the editor: every project editor adds itself
//! at startup, and File > Open Project starts the Hub from `hub_exe`. Keep the fields in step
//! with `oxide_editor::hub::HubData`.

use crate::installs::{Install, config_dir};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

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
    /// Folder the last new project was created in.
    #[serde(default)]
    pub location: Option<PathBuf>,
    /// The Hub program, so an editor's File > Open Project can start it.
    #[serde(default)]
    pub hub_exe: Option<PathBuf>,
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl HubData {
    pub fn path() -> PathBuf {
        config_dir().join("Hub.yaml")
    }

    pub fn load_from(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_yaml_ng::from_str(&t).ok())
            .unwrap_or_default()
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

/// Whether `dir` holds an Oxide project.
pub fn is_project(dir: &Path) -> bool {
    dir.join("Assets").is_dir() || dir.join("Scripts/Cargo.toml").is_file()
}

/// The engine version recorded in `ProjectSettings/ProjectVersion.txt`
/// (`m_EditorVersion: 0.2.0`), written by the editor that last opened the project.
pub fn project_version(dir: &Path) -> Option<String> {
    std::fs::read_to_string(dir.join("ProjectSettings/ProjectVersion.txt"))
        .ok()?
        .lines()
        .find_map(|l| l.trim().strip_prefix("m_EditorVersion:"))
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

pub fn project_name(dir: &Path) -> String {
    dir.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| dir.display().to_string())
}

/// "My project", or "My project (1)"... when that folder already exists in `location`.
pub fn free_name(location: &Path, base: &str) -> String {
    if !location.join(base).exists() {
        return base.to_owned();
    }
    (1..)
        .map(|i| format!("{base} ({i})"))
        .find(|n| !location.join(n).exists())
        .unwrap_or_else(|| base.to_owned())
}

/// Why a new project can't be created at `location/name`, if it can't.
pub fn new_project_problem(location: &str, name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() {
        return Some("Enter a project name.".into());
    }
    if name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']) || name.starts_with('.') {
        return Some("The name can't contain / \\ : * ? \" < > | or start with a dot.".into());
    }
    let location = location.trim();
    if location.is_empty() {
        return Some("Choose a location.".into());
    }
    let dir = Path::new(location).join(name);
    if dir.exists()
        && std::fs::read_dir(&dir)
            .map(|mut d| d.next().is_some())
            .unwrap_or(true)
    {
        return Some(format!("{} already exists and isn't empty.", dir.display()));
    }
    None
}

/// Creates a project with `install`'s template (`oxide create DIR`).
pub fn create_project(install: &Install, dir: &Path) -> Result<(), String> {
    let out = install
        .command()
        .arg("create")
        .arg(dir)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("cannot run {}: {e}", install.exe.display()))?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(err.trim().trim_start_matches("oxide: ").to_owned())
    }
}

/// Whether cargo (Rust) can be found, as the launcher looks for it.
pub fn rust_installed() -> bool {
    if std::env::var_os("CARGO").is_some() {
        return true;
    }
    let exe = format!("cargo{}", std::env::consts::EXE_SUFFIX);
    let on_path = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(&exe).is_file()))
        .unwrap_or(false);
    on_path
        || std::env::var_os("CARGO_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo")))
            .or_else(|| std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join(".cargo")))
            .is_some_and(|h| h.join("bin").join(&exe).is_file())
}

// ---------------------------------------------------------------------------------------
// Opening: build the project's editor with the chosen version (showing progress), then
// start it.

enum JobMsg {
    Line(String),
    Done(Result<(), String>),
}

pub struct OpenJob {
    pub name: String,
    pub dir: PathBuf,
    pub install: Install,
    pub started: Instant,
    rx: Receiver<JobMsg>,
    child: Arc<Mutex<Option<Child>>>,
    /// Last cargo status line ("Compiling oxide_render ...").
    pub status: String,
    pub compiled: usize,
    pub log: Vec<String>,
    pub error: Option<String>,
}

impl OpenJob {
    pub fn start(dir: PathBuf, install: Install) -> Self {
        let name = project_name(&dir);
        let (tx, rx) = channel();
        let child = Arc::new(Mutex::new(None));
        let slot = child.clone();
        let mut cmd = install.command();
        cmd.arg("open")
            .arg(&dir)
            .arg("--build-only")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        std::thread::spawn(move || {
            let mut c = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.send(JobMsg::Done(Err(format!("cannot start the editor: {e}"))));
                    return;
                }
            };
            let err = c.stderr.take();
            *slot.lock().unwrap() = Some(c);
            if let Some(err) = err {
                for line in BufReader::new(err).lines().map_while(Result::ok) {
                    let _ = tx.send(JobMsg::Line(line));
                }
            }
            let status = slot.lock().unwrap().take().map(|mut c| c.wait());
            let r = match status {
                Some(Ok(s)) if s.success() => Ok(()),
                Some(Ok(_)) => Err("The project's editor failed to build.".into()),
                Some(Err(e)) => Err(e.to_string()),
                None => Err("Cancelled.".into()),
            };
            let _ = tx.send(JobMsg::Done(r));
        });
        Self {
            name,
            dir,
            install,
            started: Instant::now(),
            rx,
            child,
            status: "Checking the project's editor...".into(),
            compiled: 0,
            log: Vec::new(),
            error: None,
        }
    }

    pub fn cancel(&self) {
        if let Some(c) = self.child.lock().unwrap().as_mut() {
            let _ = c.kill();
        }
    }

    /// Reads build output; `Some(Ok)` once the editor is built.
    pub fn poll(&mut self) -> Option<Result<(), String>> {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                JobMsg::Line(l) => {
                    let t = l.trim().trim_start_matches("oxide: ");
                    if t.starts_with("Compiling") {
                        self.compiled += 1;
                    }
                    if t.starts_with("Blocking waiting for file lock") {
                        self.status = "Waiting for another project's build to finish...".into();
                    } else if [
                        "Compiling",
                        "Building",
                        "Installing",
                        "Checking",
                        "Finished",
                        "error",
                        "warning",
                    ]
                    .iter()
                    .any(|p| t.starts_with(p))
                    {
                        self.status = t.to_owned();
                    }
                    // The launcher's own errors (no Rust, not a project) are the message.
                    if t.starts_with("Rust is not installed")
                        || t.contains("is not an Oxide project")
                    {
                        self.error = Some(t.to_owned());
                    }
                    self.log.push(l);
                    if self.log.len() > 400 {
                        self.log.drain(..100);
                    }
                }
                JobMsg::Done(r) => {
                    return Some(match (r, self.error.take()) {
                        (Err(_), Some(e)) => Err(e),
                        (r, _) => r,
                    });
                }
            }
        }
        None
    }
}

/// Starts the (built) editor on `dir` and leaves it running on its own.
pub fn start_editor(install: &Install, dir: &Path) -> Result<(), String> {
    install
        .command()
        .arg("open")
        .arg(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
        .map_err(|e| format!("cannot start the editor: {e}"))
}

// ---------------------------------------------------------------------------------------
// Desktop helpers.

/// Shows a folder in Finder / Explorer.
pub fn show_folder(dir: &Path) -> Result<(), String> {
    let mut cmd = if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else if cfg!(windows) {
        std::process::Command::new("explorer")
    } else {
        std::process::Command::new("xdg-open")
    };
    cmd.arg(dir)
        .spawn()
        .map(drop)
        .map_err(|e| format!("cannot open the file browser: {e}"))
}

/// Opens a web page in the default browser.
pub fn open_url(url: &str) {
    let mut cmd = if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else if cfg!(windows) {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", ""]);
        c
    } else {
        std::process::Command::new("xdg-open")
    };
    let _ = cmd.arg(url).spawn();
}

/// The system's folder chooser (blocks until the user picks a folder or cancels).
pub fn pick_folder(prompt: &str, start: Option<&Path>) -> Option<PathBuf> {
    use std::process::Command;
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

/// A path with the home folder written as `~`.
pub fn short_path(p: &Path) -> String {
    let full = p.display().to_string();
    match std::env::var_os("HOME").map(PathBuf::from) {
        Some(home) if !home.as_os_str().is_empty() => match p.strip_prefix(&home) {
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => full,
        },
        _ => full,
    }
}

/// "5 minutes ago".
pub fn ago(secs: u64) -> String {
    if secs == 0 {
        return "—".into();
    }
    let d = now().saturating_sub(secs);
    let plural = |n: u64, unit: &str| format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" });
    match d {
        0..60 => "just now".into(),
        60..3600 => plural(d / 60, "minute"),
        3600..86_400 => plural(d / 3600, "hour"),
        86_400..2_592_000 => plural(d / 86_400, "day"),
        2_592_000..31_536_000 => plural(d / 2_592_000, "month"),
        _ => plural(d / 31_536_000, "year"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("oxide-hub-proj-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn project_list_round_trips_and_orders_by_last_opened() {
        let d = temp("list");
        let (a, b) = (d.join("A"), d.join("B"));
        std::fs::create_dir_all(a.join("Assets")).unwrap();
        std::fs::create_dir_all(b.join("Assets")).unwrap();
        let mut data = HubData::default();
        data.remember(&a);
        data.remember(&b);
        data.remember(&a);
        let file = d.join("Hub.yaml");
        data.save_to(&file).unwrap();
        let back = HubData::load_from(&file);
        let names: Vec<_> = back
            .projects
            .iter()
            .map(|p| project_name(&p.path))
            .collect();
        assert_eq!(names, ["A", "B"]);
        // The editor writes the same file; its fields must read back here.
        std::fs::write(
            &file,
            "projects:\n- path: /x/Game\n  opened: 5\nlocation: /x\nhub_exe: /bin/hub\n",
        )
        .unwrap();
        let editor = HubData::load_from(&file);
        assert_eq!(editor.projects[0].opened, 5);
        assert_eq!(editor.hub_exe.as_deref(), Some(Path::new("/bin/hub")));
    }

    #[test]
    fn reads_the_project_version() {
        let d = temp("version");
        assert_eq!(project_version(&d), None);
        std::fs::create_dir_all(d.join("ProjectSettings")).unwrap();
        std::fs::write(
            d.join("ProjectSettings/ProjectVersion.txt"),
            "m_EditorVersion: 0.2.0-beta.1\n",
        )
        .unwrap();
        assert_eq!(project_version(&d).as_deref(), Some("0.2.0-beta.1"));
    }

    #[test]
    fn new_project_names_and_checks() {
        let d = temp("names");
        assert_eq!(free_name(&d, "My project"), "My project");
        std::fs::create_dir_all(d.join("My project/Assets")).unwrap();
        assert_eq!(free_name(&d, "My project"), "My project (1)");
        let loc = d.display().to_string();
        assert!(new_project_problem(&loc, "My project").is_some());
        assert!(new_project_problem(&loc, "a/b").is_some());
        assert!(new_project_problem("", "Game").is_some());
        assert!(new_project_problem(&loc, "Game").is_none());
    }
}
