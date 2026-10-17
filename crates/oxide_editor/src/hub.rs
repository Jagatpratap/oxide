//! Oxide Hub (Unity Hub): the window `oxide` opens first. It lists the projects opened
//! before, creates new ones from a template, adds existing project folders, and opens a
//! project in its own editor (building that editor first, with progress).
//!
//! The project list is kept per user in `<config>/Hub.yaml` (see [`crate::prefs::config_dir`]).
//! Every project editor adds itself to it at startup, so projects opened with `oxide open`
//! show up too.

use crate::{prefs, project_files, theme};
use egui::{Align, Color32, CornerRadius, Layout, RichText, Sense, Stroke, vec2};
use oxide_engine::{Engine, Shell};
use oxide_scene::World;
use oxide_ui::Gui;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const ACCENT: Color32 = Color32::from_rgb(33, 150, 243);
const SIDEBAR: Color32 = Color32::from_rgb(32, 32, 32);
const PAGE: Color32 = Color32::from_rgb(40, 40, 40);
const ROW_HOVER: Color32 = Color32::from_rgb(52, 52, 52);
const DIM: Color32 = Color32::from_rgb(150, 150, 150);
const ERROR: Color32 = Color32::from_rgb(240, 110, 100);

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

/// Starts the Hub (the editor's File > New Project / Open Project, like Unity).
pub fn open_hub() -> Result<(), String> {
    let exe = HubData::load()
        .hub_exe
        .filter(|p| p.is_file())
        .ok_or("the Oxide Hub has not been run on this computer yet (run `oxide`)")?;
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
fn pick_folder(prompt: &str, start: Option<&Path>) -> Option<PathBuf> {
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

/// A path with the home folder written as `~`, like a shell prompt.
fn short_path(p: &Path) -> String {
    let full = p.display().to_string();
    match std::env::var_os("HOME").map(PathBuf::from) {
        Some(home) if !home.as_os_str().is_empty() => match p.strip_prefix(&home) {
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => full,
        },
        _ => full,
    }
}

/// `text` cut in the middle with "…" so it fits `max` points in `font`.
fn elide_middle(ui: &egui::Ui, text: &str, font: &egui::FontId, max: f32) -> String {
    let width = |t: &str| {
        ui.fonts_mut(|f| {
            f.layout_no_wrap(t.to_owned(), font.clone(), Color32::WHITE)
                .size()
                .x
        })
    };
    if width(text) <= max {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0, chars.len());
    // Largest number of kept characters that fits, split evenly around the cut.
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let head: String = chars[..mid / 2].iter().collect();
        let tail: String = chars[chars.len() - (mid - mid / 2)..].iter().collect();
        if width(&format!("{head}…{tail}")) <= max {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let head: String = chars[..lo / 2].iter().collect();
    let tail: String = chars[chars.len() - (lo - lo / 2)..].iter().collect();
    format!("{head}…{tail}")
}

/// "5 minutes ago", like Unity Hub's Modified column.
fn ago(secs: u64) -> String {
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

/// "My project", or "My project (1)"... when that folder already exists in `location`.
fn free_name(location: &Path, base: &str) -> String {
    if !location.join(base).exists() {
        return base.to_owned();
    }
    (1..)
        .map(|i| format!("{base} ({i})"))
        .find(|n| !location.join(n).exists())
        .unwrap_or_else(|| base.to_owned())
}

/// Why a new project can't be created at `location/name`, if it can't.
fn new_project_problem(location: &str, name: &str) -> Option<String> {
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

// ---------------------------------------------------------------------------------------
// Opening a project: build its editor (with progress), then start it.

enum JobMsg {
    Line(String),
    Done(Result<(), String>),
}

struct OpenJob {
    name: String,
    dir: PathBuf,
    started: Instant,
    rx: Receiver<JobMsg>,
    child: Arc<Mutex<Option<Child>>>,
    /// Last cargo status line ("Compiling oxide_render ...").
    status: String,
    compiled: usize,
    log: Vec<String>,
    error: Option<String>,
}

impl OpenJob {
    fn start(dir: PathBuf) -> Self {
        let name = project_name(&dir);
        let (tx, rx) = channel();
        let child = Arc::new(Mutex::new(None));
        let slot = child.clone();
        let build_dir = dir.clone();
        std::thread::spawn(move || {
            let spawned = Command::new(project_files::cargo_command())
                .args(["build", "-p", "oxide-project-editor", "--color", "never"])
                .current_dir(&build_dir)
                .env_remove("CARGO_TARGET_DIR")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn();
            let mut c = match spawned {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.send(JobMsg::Done(Err(format!("cannot run cargo: {e}"))));
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
            started: Instant::now(),
            rx,
            child,
            status: "Checking the project's editor...".into(),
            compiled: 0,
            log: Vec::new(),
            error: None,
        }
    }

    fn cancel(&self) {
        if let Some(c) = self.child.lock().unwrap().as_mut() {
            let _ = c.kill();
        }
    }

    /// Reads build output; `Some(Ok)` once the editor is built.
    fn poll(&mut self) -> Option<Result<(), String>> {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                JobMsg::Line(l) => {
                    let t = l.trim();
                    if t.starts_with("Compiling") {
                        self.compiled += 1;
                    }
                    // Projects share one build folder; Cargo lets one build use it at a time.
                    if t.starts_with("Blocking waiting for file lock") {
                        self.status = "Waiting for another project's build to finish...".into();
                    }
                    if [
                        "Compiling",
                        "Building",
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
                    self.log.push(l);
                    if self.log.len() > 400 {
                        self.log.drain(..100);
                    }
                }
                JobMsg::Done(r) => return Some(r),
            }
        }
        None
    }
}

/// Starts the built editor on `dir`. `cargo run` is used (not the executable directly)
/// because it sets the library path the engine's shared library is found through.
fn start_editor(dir: &Path) -> Result<(), String> {
    Command::new(project_files::cargo_command())
        .args(["run", "-q", "-p", "oxide-project-editor", "--", "--project"])
        .arg(dir)
        .current_dir(dir)
        .env_remove("CARGO_TARGET_DIR")
        .stdin(Stdio::null())
        .spawn()
        .map(drop)
        .map_err(|e| format!("cannot start the editor: {e}"))
}

/// Whether the project's editor has been built (so opening it is quick).
fn editor_built(dir: &Path) -> bool {
    project_files::editor_exe(dir).is_file()
}

fn project_name(dir: &Path) -> String {
    dir.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| dir.display().to_string())
}

// ---------------------------------------------------------------------------------------
// The Hub window.

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Projects,
    NewProject,
    Installs,
}

pub struct Hub {
    engine_root: PathBuf,
    data: HubData,
    data_path: PathBuf,
    page: Page,
    search: String,
    /// New project page.
    new_name: String,
    new_location: String,
    /// A folder chooser running on another thread, and what its answer is for.
    picking: Option<(Receiver<Option<PathBuf>>, PickFor)>,
    job: Option<OpenJob>,
    /// Message shown in the bar at the bottom (errors, "Removed ...").
    notice: Option<(String, bool)>,
    /// The notice shown and when it appeared.
    notice_at: Option<(String, Instant)>,
    styled: bool,
    /// Leave once the editor has been started (off in tests).
    pub exit_on_open: bool,
}

#[derive(Clone, Copy)]
enum PickFor {
    AddProject,
    Location,
}

impl Hub {
    pub fn new(engine_root: PathBuf) -> Self {
        let data_path = HubData::path();
        let mut data = HubData::load_from(&data_path);
        if let Ok(exe) = std::env::current_exe() {
            data.hub_exe = Some(exe);
            let _ = data.save_to(&data_path);
        }
        Self {
            engine_root,
            data,
            data_path,
            page: Page::Projects,
            search: String::new(),
            new_name: String::new(),
            new_location: String::new(),
            picking: None,
            job: None,
            notice: None,
            notice_at: None,
            styled: false,
            exit_on_open: true,
        }
    }

    fn save(&mut self) {
        if let Err(e) = self.data.save_to(&self.data_path) {
            self.notice = Some((format!("Cannot save the project list: {e}"), true));
        }
    }

    fn pick(&mut self, what: PickFor, prompt: &'static str, start: Option<PathBuf>) {
        if self.picking.is_some() {
            return;
        }
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(pick_folder(prompt, start.as_deref()));
        });
        self.picking = Some((rx, what));
    }

    fn poll_pick(&mut self) {
        let Some((rx, what)) = &self.picking else {
            return;
        };
        let Ok(answer) = rx.try_recv() else {
            return;
        };
        let what = *what;
        self.picking = None;
        let Some(dir) = answer else {
            return;
        };
        match what {
            PickFor::Location => self.new_location = dir.display().to_string(),
            PickFor::AddProject => self.add_existing(&dir),
        }
    }

    /// "Add project from disk".
    pub fn add_existing(&mut self, dir: &Path) {
        if !is_project(dir) {
            self.notice = Some((
                format!(
                    "{} isn't an Oxide project (it has no Assets or Scripts folder).",
                    dir.display()
                ),
                true,
            ));
            return;
        }
        self.data.remember(dir);
        self.data.projects[0].opened = 0;
        self.save();
        self.notice = Some((format!("Added {}.", project_name(dir)), false));
    }

    fn open_new_page(&mut self) {
        let location = self
            .data
            .location
            .clone()
            .filter(|l| l.is_dir())
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .unwrap_or_default();
        self.new_name = free_name(&location, "My project");
        self.new_location = location.display().to_string();
        self.page = Page::NewProject;
    }

    /// Creates the project from the New project page and opens it.
    pub fn create(&mut self) {
        if let Some(p) = new_project_problem(&self.new_location, &self.new_name) {
            self.notice = Some((p, true));
            return;
        }
        let location = PathBuf::from(self.new_location.trim());
        let dir = location.join(self.new_name.trim());
        if let Err(e) = project_files::create_project(&dir, &self.engine_root) {
            self.notice = Some((format!("Cannot create the project: {e}"), true));
            return;
        }
        self.data.location = Some(location);
        self.data.remember(&dir);
        self.save();
        self.page = Page::Projects;
        self.open(&dir);
    }

    /// Opens a project: refreshes its build files, builds its editor, then starts it.
    pub fn open(&mut self, dir: &Path) {
        if self.job.is_some() {
            return;
        }
        if !is_project(dir) {
            self.notice = Some((format!("{} can't be found.", dir.display()), true));
            return;
        }
        if let Err(e) = project_files::write_build_files(dir, &self.engine_root) {
            self.notice = Some((format!("Cannot prepare the project: {e}"), true));
            return;
        }
        self.data.remember(dir);
        self.save();
        self.job = Some(OpenJob::start(dir.to_owned()));
    }

    fn poll_job(&mut self) {
        let Some(job) = self.job.as_mut() else {
            return;
        };
        if job.error.is_some() {
            return;
        }
        match job.poll() {
            None => {}
            Some(Err(e)) => job.error = Some(e),
            Some(Ok(())) => match start_editor(&job.dir) {
                Ok(()) if self.exit_on_open => std::process::exit(0),
                Ok(()) => self.job = None,
                Err(e) => job.error = Some(e),
            },
        }
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            logo(ui, 22.0);
            ui.label(
                RichText::new("Oxide Hub")
                    .size(17.0)
                    .strong()
                    .color(Color32::WHITE),
            );
        });
        ui.add_space(18.0);
        for (page, label) in [(Page::Projects, "Projects"), (Page::Installs, "Installs")] {
            let current =
                self.page == page || (page == Page::Projects && self.page == Page::NewProject);
            if nav_item(ui, label, current).clicked() {
                self.page = page;
            }
        }
    }

    fn projects_page(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Projects")
                    .size(24.0)
                    .strong()
                    .color(Color32::WHITE),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if primary_button(ui, "New project").clicked() {
                    self.open_new_page();
                }
                let busy = self.picking.is_some();
                if ui
                    .add_enabled(
                        !busy,
                        egui::Button::new("Add project from disk").min_size(vec2(0.0, 30.0)),
                    )
                    .on_hover_text("Add an existing project folder to this list")
                    .clicked()
                {
                    self.pick(PickFor::AddProject, "Select an Oxide project folder", None);
                }
            });
        });
        ui.add_space(12.0);
        ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .hint_text("🔍 Search")
                .desired_width(280.0)
                .min_size(vec2(0.0, 26.0)),
        );
        ui.add_space(10.0);

        if self.data.projects.is_empty() {
            ui.add_space(60.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new("No projects yet")
                        .size(18.0)
                        .color(Color32::WHITE),
                );
                ui.add_space(6.0);
                ui.label(
                    RichText::new(
                        "Create a new project, or add a project folder you already have.",
                    )
                    .color(DIM),
                );
            });
            return;
        }

        // Column headers.
        let w = ui.available_width();
        let (hdr, _) = ui.allocate_exact_size(vec2(w, 22.0), Sense::hover());
        let cols = columns(hdr);
        let p = ui.painter();
        let font = egui::FontId::proportional(11.5);
        p.text(
            cols.name.left_center(),
            egui::Align2::LEFT_CENTER,
            "NAME",
            font.clone(),
            DIM,
        );
        p.text(
            cols.opened.left_center(),
            egui::Align2::LEFT_CENTER,
            "LAST OPENED",
            font.clone(),
            DIM,
        );
        p.text(
            cols.status.left_center(),
            egui::Align2::LEFT_CENTER,
            "EDITOR",
            font,
            DIM,
        );
        p.hline(
            hdr.x_range(),
            hdr.bottom(),
            Stroke::new(1.0, Color32::from_rgb(60, 60, 60)),
        );

        let filter = self.search.trim().to_lowercase();
        let projects: Vec<HubProject> = self
            .data
            .projects
            .iter()
            .filter(|p| {
                filter.is_empty()
                    || project_name(&p.path).to_lowercase().contains(&filter)
                    || p.path
                        .display()
                        .to_string()
                        .to_lowercase()
                        .contains(&filter)
            })
            .cloned()
            .collect();
        let mut open = None;
        let mut forget = None;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                for proj in &projects {
                    match project_row(ui, proj) {
                        RowAction::None => {}
                        RowAction::Open => open = Some(proj.path.clone()),
                        RowAction::Show => {
                            if let Err(e) = show_folder(&proj.path) {
                                self.notice = Some((e, true));
                            }
                        }
                        RowAction::Forget => forget = Some(proj.path.clone()),
                    }
                }
                if projects.is_empty() {
                    ui.add_space(20.0);
                    ui.label(RichText::new("No projects match your search.").color(DIM));
                }
            });
        if let Some(dir) = forget {
            self.data.forget(&dir);
            self.save();
            self.notice = Some((
                format!(
                    "Removed {} from the list (its files are kept).",
                    project_name(&dir)
                ),
                false,
            ));
        }
        if let Some(dir) = open {
            self.open(&dir);
        }
    }

    fn new_project_page(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("‹ Back").clicked() {
                self.page = Page::Projects;
            }
            ui.label(
                RichText::new("New project")
                    .size(24.0)
                    .strong()
                    .color(Color32::WHITE),
            );
        });
        ui.add_space(16.0);
        // Templates on the left, the project's name and location in a fixed panel on the right.
        let area = ui.available_rect_before_wrap();
        let body =
            egui::Rect::from_min_max(area.min, egui::pos2(area.right(), area.bottom() - 50.0));
        let settings_w = 320.0;
        let settings =
            egui::Rect::from_min_max(egui::pos2(body.right() - settings_w, body.top()), body.max);
        let templates =
            egui::Rect::from_min_max(body.min, egui::pos2(settings.left() - 20.0, body.bottom()));
        ui.scope_builder(egui::UiBuilder::new().max_rect(templates), |ui| {
            ui.label(RichText::new("TEMPLATES").size(11.5).color(DIM));
            ui.add_space(6.0);
            template_card(
                ui,
                "3D (Core)",
                "An empty 3D project: a scene with a Main Camera and a Directional Light, and a \
                 Rust Scripts crate with an example script, ready to hot-reload.",
            );
        });
        ui.painter().rect_filled(
            settings,
            CornerRadius::same(6),
            Color32::from_rgb(46, 46, 46),
        );
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(settings.shrink(16.0)),
            |ui| {
                ui.label(RichText::new("PROJECT SETTINGS").size(11.5).color(DIM));
                ui.add_space(12.0);
                ui.label("Project name");
                ui.add(
                    egui::TextEdit::singleline(&mut self.new_name)
                        .desired_width(f32::INFINITY)
                        .min_size(vec2(0.0, 24.0)),
                );
                ui.add_space(12.0);
                ui.label("Location");
                ui.horizontal(|ui| {
                    let busy = self.picking.is_some();
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_location)
                            .desired_width(ui.available_width() - 74.0)
                            .min_size(vec2(0.0, 24.0)),
                    );
                    let browse = ui
                        .add_enabled(
                            !busy,
                            egui::Button::new("Browse...").min_size(vec2(68.0, 24.0)),
                        )
                        .on_hover_text("Choose the folder to create the project in");
                    if browse.clicked() {
                        let start = PathBuf::from(self.new_location.trim());
                        self.pick(
                            PickFor::Location,
                            "Choose where to create the project",
                            Some(start),
                        );
                    }
                });
                ui.add_space(10.0);
                let full = Path::new(self.new_location.trim()).join(self.new_name.trim());
                ui.add(
                    egui::Label::new(RichText::new(short_path(&full)).size(11.0).color(DIM)).wrap(),
                );
                if let Some(p) = new_project_problem(&self.new_location, &self.new_name) {
                    ui.add_space(6.0);
                    ui.add(egui::Label::new(RichText::new(p).size(11.5).color(ERROR)).wrap());
                }
            },
        );
        ui.advance_cursor_after_rect(body);
        ui.separator();
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let ok = new_project_problem(&self.new_location, &self.new_name).is_none();
                let create = ui.add_enabled(ok, primary(ui, "Create project"));
                if create.clicked() {
                    self.create();
                }
                if ui
                    .add(egui::Button::new("Cancel").min_size(vec2(0.0, 30.0)))
                    .clicked()
                {
                    self.page = Page::Projects;
                }
            });
        });
    }

    fn installs_page(&mut self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new("Installs")
                .size(24.0)
                .strong()
                .color(Color32::WHITE),
        );
        ui.add_space(16.0);
        egui::Frame::new()
            .fill(Color32::from_rgb(46, 46, 46))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(egui::Margin::same(14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    logo(ui, 34.0);
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new(format!("Oxide {}", env!("CARGO_PKG_VERSION")))
                                .size(15.0)
                                .strong()
                                .color(Color32::WHITE),
                        );
                        ui.label(RichText::new(self.engine_root.display().to_string()).color(DIM));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Show in Finder").clicked()
                            && let Err(e) = show_folder(&self.engine_root)
                        {
                            self.notice = Some((e, true));
                        }
                    });
                });
            });
        ui.add_space(8.0);
        ui.label(RichText::new("New projects use this engine.").color(DIM));
        ui.add_space(16.0);
        let build = project_files::shared_build_dir();
        egui::Frame::new()
            .fill(Color32::from_rgb(46, 46, 46))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(egui::Margin::same(14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("Shared build folder")
                                .size(15.0)
                                .strong()
                                .color(Color32::WHITE),
                        );
                        ui.label(RichText::new(build.display().to_string()).color(DIM));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add_enabled(build.is_dir(), egui::Button::new("Show in Finder"))
                            .clicked()
                            && let Err(e) = show_folder(&build)
                        {
                            self.notice = Some((e, true));
                        }
                    });
                });
            });
        ui.add_space(8.0);
        ui.label(
            RichText::new(
                "Every project builds into this folder, so the engine is compiled once and a \
                 new project only compiles its own scripts. It's a cache: deleting it is safe \
                 and the next open rebuilds it.",
            )
            .color(DIM),
        );
    }

    fn job_window(&mut self, ctx: &egui::Context) {
        let Some(job) = self.job.as_mut() else {
            return;
        };
        let mut close = false;
        egui::Modal::new(egui::Id::new("hub-open")).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.label(
                RichText::new(format!("Opening {}", job.name))
                    .size(17.0)
                    .strong()
                    .color(Color32::WHITE),
            );
            ui.add_space(8.0);
            if let Some(e) = &job.error {
                ui.label(RichText::new(e).color(ERROR));
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .max_height(220.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        let tail = job.log.len().saturating_sub(60);
                        ui.label(
                            RichText::new(job.log[tail..].join("\n"))
                                .monospace()
                                .size(11.0),
                        );
                    });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                    if ui.button("Show in Finder").clicked() {
                        let _ = show_folder(&job.dir);
                    }
                });
                return;
            }
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(18.0));
                ui.label(&job.status);
            });
            ui.add_space(6.0);
            let secs = job.started.elapsed().as_secs();
            let mut detail = format!("{}:{:02}", secs / 60, secs % 60);
            if job.compiled > 0 {
                detail += &format!(" · {} crates compiled", job.compiled);
                if !editor_built(&job.dir) {
                    detail += " · the first build on this machine compiles the engine and takes a few minutes";
                }
            }
            ui.label(RichText::new(detail).size(11.5).color(DIM));
            ui.add_space(10.0);
            if ui.button("Cancel").clicked() {
                job.cancel();
                close = true;
            }
        });
        if close {
            self.job = None;
        }
    }

    fn draw(&mut self, ctx: &egui::Context) {
        self.poll_pick();
        self.poll_job();
        // Information notices go away by themselves; errors stay until closed.
        let text = self.notice.as_ref().map(|n| n.0.clone());
        if self.notice_at.as_ref().map(|n| &n.0) != text.as_ref() {
            self.notice_at = text.map(|t| (t, Instant::now()));
        } else if let (Some((_, false)), Some((_, t))) = (&self.notice, &self.notice_at)
            && t.elapsed().as_secs() >= 6
        {
            self.notice = None;
        }
        if self.notice.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
        if self.job.is_some() || self.picking.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        let mut root = egui::Ui::new(
            ctx.clone(),
            egui::Id::new("oxide-hub"),
            egui::UiBuilder::new()
                .layer_id(egui::LayerId::background())
                .max_rect(ctx.content_rect()),
        );
        egui::Panel::left(egui::Id::new("hub-sidebar"))
            .exact_size(190.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(SIDEBAR))
            .show(&mut root, |ui| self.sidebar(ui));
        if let Some((text, error)) = self.notice.clone() {
            egui::Panel::bottom(egui::Id::new("hub-notice"))
                .frame(
                    egui::Frame::new()
                        .fill(if error {
                            Color32::from_rgb(80, 40, 38)
                        } else {
                            Color32::from_rgb(38, 58, 44)
                        })
                        .inner_margin(egui::Margin::symmetric(14, 8)),
                )
                .show(&mut root, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(text).color(Color32::WHITE));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.small_button("✕").clicked() {
                                self.notice = None;
                            }
                        });
                    });
                });
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(PAGE)
                    .inner_margin(egui::Margin::symmetric(28, 22)),
            )
            .show(&mut root, |ui| match self.page {
                Page::Projects => self.projects_page(ui),
                Page::NewProject => self.new_project_page(ui),
                Page::Installs => self.installs_page(ui),
            });
        self.job_window(ctx);
    }

    /// For tests and MCP: the projects listed, newest first.
    pub fn projects(&self) -> &[HubProject] {
        &self.data.projects
    }
}

impl Shell for Hub {
    fn install(&mut self, world: &mut World) {
        world.run_scripts = false;
    }

    fn ui(&mut self, engine: &mut Engine) {
        let ctx = engine.world.resource::<Gui>().ctx().clone();
        if !self.styled {
            theme::apply(&ctx);
            self.styled = true;
        }
        self.draw(&ctx);
    }

    fn render(&mut self, _engine: &mut Engine) {}

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

// ---------------------------------------------------------------------------------------
// Widgets.

struct Columns {
    name: egui::Rect,
    opened: egui::Rect,
    status: egui::Rect,
    menu: egui::Rect,
}

fn columns(row: egui::Rect) -> Columns {
    let menu_w = 36.0;
    let status_w = 150.0;
    let opened_w = 150.0;
    let r = row.right();
    let menu = egui::Rect::from_x_y_ranges(r - menu_w..=r, row.y_range());
    let status = egui::Rect::from_x_y_ranges(r - menu_w - status_w..=r - menu_w, row.y_range());
    let opened = egui::Rect::from_x_y_ranges(
        r - menu_w - status_w - opened_w..=r - menu_w - status_w,
        row.y_range(),
    );
    let name = egui::Rect::from_x_y_ranges(row.left() + 8.0..=opened.left() - 8.0, row.y_range());
    Columns {
        name,
        opened,
        status,
        menu,
    }
}

enum RowAction {
    None,
    Open,
    Show,
    Forget,
}

fn project_row(ui: &mut egui::Ui, proj: &HubProject) -> RowAction {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 52.0), Sense::click());
    let cols = columns(rect);
    let exists = is_project(&proj.path);
    // The ⋮ button sits on the row, so it takes the click before the row does.
    let menu_rect = egui::Rect::from_center_size(cols.menu.center(), vec2(26.0, 26.0));
    let menu = ui.put(menu_rect, egui::Button::new("⋮").frame(false));
    let mut action = RowAction::None;
    egui::Popup::menu(&menu).show(|ui| {
        ui.set_min_width(170.0);
        if ui.add_enabled(exists, egui::Button::new("Open")).clicked() {
            action = RowAction::Open;
        }
        if ui
            .add_enabled(proj.path.is_dir(), egui::Button::new("Show in Finder"))
            .clicked()
        {
            action = RowAction::Show;
        }
        ui.separator();
        if ui.button("Remove from list").clicked() {
            action = RowAction::Forget;
        }
    });
    let hovered = resp.hovered() || menu.hovered();
    let p = ui.painter();
    if hovered {
        p.rect_filled(rect, CornerRadius::same(4), ROW_HOVER);
    }
    p.hline(
        rect.x_range(),
        rect.bottom(),
        Stroke::new(1.0, Color32::from_rgb(50, 50, 50)),
    );
    // Icon: the project's initial on a colour from its name.
    let name = project_name(&proj.path);
    let icon = egui::Rect::from_min_size(
        egui::pos2(cols.name.left(), rect.center().y - 15.0),
        vec2(30.0, 30.0),
    );
    let hue = name
        .bytes()
        .fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32))
        % 360;
    let fill: Color32 = egui::ecolor::Hsva::new(hue as f32 / 360.0, 0.45, 0.55, 1.0).into();
    p.rect_filled(
        icon,
        CornerRadius::same(5),
        if exists { fill } else { Color32::from_gray(70) },
    );
    p.text(
        icon.center(),
        egui::Align2::CENTER_CENTER,
        name.chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_default(),
        egui::FontId::proportional(15.0),
        Color32::WHITE,
    );
    let text_x = icon.right() + 12.0;
    let clip = egui::Rect::from_x_y_ranges(text_x..=cols.name.right(), rect.y_range());
    let pc = p.with_clip_rect(clip);
    pc.text(
        egui::pos2(text_x, rect.center().y - 8.0),
        egui::Align2::LEFT_CENTER,
        &name,
        egui::FontId::proportional(14.0),
        if exists { Color32::WHITE } else { DIM },
    );
    pc.text(
        egui::pos2(text_x, rect.center().y + 9.0),
        egui::Align2::LEFT_CENTER,
        elide_middle(
            ui,
            &short_path(&proj.path),
            &egui::FontId::proportional(11.0),
            clip.width(),
        ),
        egui::FontId::proportional(11.0),
        DIM,
    );
    p.text(
        cols.opened.left_center(),
        egui::Align2::LEFT_CENTER,
        ago(proj.opened),
        egui::FontId::proportional(12.5),
        Color32::from_gray(200),
    );
    let (status, color) = if !exists {
        ("Folder not found", ERROR)
    } else if editor_built(&proj.path) {
        ("Ready", Color32::from_gray(200))
    } else {
        ("Builds on first open", DIM)
    };
    p.text(
        cols.status.left_center(),
        egui::Align2::LEFT_CENTER,
        status,
        egui::FontId::proportional(12.5),
        color,
    );
    let resp = if exists {
        resp.on_hover_text("Open in the editor")
    } else {
        resp.on_hover_text("This folder was moved or deleted. Use ⋮ › Remove from list.")
    };
    if matches!(action, RowAction::None) && resp.clicked() && exists {
        action = RowAction::Open;
    }
    action
}

fn nav_item(ui: &mut egui::Ui, label: &str, current: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
    let p = ui.painter();
    let inner = rect.shrink2(vec2(8.0, 2.0));
    if current {
        p.rect_filled(inner, CornerRadius::same(4), Color32::from_rgb(52, 52, 52));
        p.rect_filled(
            egui::Rect::from_min_size(inner.left_top(), vec2(3.0, inner.height())),
            CornerRadius::same(1),
            ACCENT,
        );
    } else if resp.hovered() {
        p.rect_filled(inner, CornerRadius::same(4), Color32::from_rgb(42, 42, 42));
    }
    p.text(
        egui::pos2(inner.left() + 16.0, inner.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(13.5),
        if current {
            Color32::WHITE
        } else {
            Color32::from_gray(190)
        },
    );
    resp
}

fn primary<'a>(_ui: &egui::Ui, text: &'a str) -> egui::Button<'a> {
    egui::Button::new(RichText::new(text).color(Color32::WHITE).strong())
        .fill(ACCENT)
        .min_size(vec2(0.0, 30.0))
}

fn primary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(primary(ui, text))
}

/// The Oxide mark: a rust-coloured rounded square with an "O".
fn logo(ui: &mut egui::Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let p = ui.painter();
    p.rect_filled(
        rect,
        CornerRadius::same((size * 0.22) as u8),
        Color32::from_rgb(196, 86, 44),
    );
    p.circle_stroke(
        rect.center(),
        size * 0.27,
        Stroke::new(size * 0.11, Color32::WHITE),
    );
}

fn template_card(ui: &mut egui::Ui, title: &str, text: &str) {
    egui::Frame::new()
        .fill(Color32::from_rgb(46, 46, 46))
        .stroke(Stroke::new(2.0, ACCENT))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width().min(420.0));
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::hover());
                let p = ui.painter();
                p.rect_filled(rect, CornerRadius::same(4), Color32::from_rgb(60, 70, 84));
                // A little cube.
                let c = rect.center();
                let s = 14.0;
                let top = [
                    c + vec2(0.0, -s),
                    c + vec2(s, -s * 0.5),
                    c,
                    c + vec2(-s, -s * 0.5),
                ];
                let left = [
                    c + vec2(-s, -s * 0.5),
                    c,
                    c + vec2(0.0, s),
                    c + vec2(-s, s * 0.5),
                ];
                let right = [
                    c,
                    c + vec2(s, -s * 0.5),
                    c + vec2(s, s * 0.5),
                    c + vec2(0.0, s),
                ];
                for (pts, g) in [(top, 220), (left, 150), (right, 110)] {
                    p.add(egui::Shape::convex_polygon(
                        pts.to_vec(),
                        Color32::from_gray(g),
                        Stroke::NONE,
                    ));
                }
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(title)
                            .size(14.0)
                            .strong()
                            .color(Color32::WHITE),
                    );
                    ui.add(egui::Label::new(RichText::new(text).color(DIM)).wrap());
                });
            });
        });
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
        assert_eq!(project_name(&data.projects[0].path), "A");
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

    #[test]
    fn new_project_names_and_checks() {
        let d = temp("names");
        let loc = d.display().to_string();
        assert_eq!(free_name(&d, "My project"), "My project");
        std::fs::create_dir_all(d.join("My project/Assets")).unwrap();
        assert_eq!(free_name(&d, "My project"), "My project (1)");
        assert!(new_project_problem(&loc, "My project").is_some());
        assert!(new_project_problem(&loc, "My project (1)").is_none());
        assert!(new_project_problem(&loc, "").is_some());
        assert!(new_project_problem(&loc, "a/b").is_some());
        assert!(new_project_problem("", "Game").is_some());
        // An existing empty folder is fine, like Unity Hub.
        std::fs::create_dir_all(d.join("Empty")).unwrap();
        assert!(new_project_problem(&loc, "Empty").is_none());
    }

    #[test]
    fn creating_a_project_lists_it() {
        let d = temp("create");
        let engine = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut hub = Hub {
            engine_root: engine,
            data: HubData::default(),
            data_path: d.join("Hub.yaml"),
            page: Page::NewProject,
            search: String::new(),
            new_name: "Racer".into(),
            new_location: d.display().to_string(),
            picking: None,
            job: None,
            notice: None,
            notice_at: None,
            styled: false,
            exit_on_open: false,
        };
        // `create` would also start a build; this does its first half.
        let dir = d.join("Racer");
        project_files::create_project(&dir, &hub.engine_root).unwrap();
        hub.data.remember(&dir);
        hub.save();
        assert!(is_project(&dir));
        assert_eq!(HubData::load_from(&d.join("Hub.yaml")).projects.len(), 1);
        assert!(dir.join("Scripts/src/lib.rs").is_file());
        assert_eq!(hub.projects().len(), 1);
        hub.add_existing(&d);
        assert!(
            hub.notice.as_ref().is_some_and(|n| n.1),
            "a non-project folder is refused"
        );
        assert_eq!(hub.projects().len(), 1);
    }
}
