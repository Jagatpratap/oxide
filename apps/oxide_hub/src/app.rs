//! The Hub window: Projects, New project and Installs pages.

use crate::installs::{self, Install, InstallJob, Stage};
use crate::manifest::{self, Channel, Manifest};
use crate::projects::{self, HubData, HubProject, OpenJob};
use egui::{Align, Color32, CornerRadius, Layout, RichText, Sense, Stroke, vec2};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

/// Rust orange, the colour of the Hub's box icon.
const ACCENT: Color32 = Color32::from_rgb(196, 86, 44);
const ACCENT_HOVER: Color32 = Color32::from_rgb(214, 104, 60);
const SIDEBAR: Color32 = Color32::from_rgb(28, 28, 30);
const PAGE: Color32 = Color32::from_rgb(38, 38, 40);
const CARD: Color32 = Color32::from_rgb(46, 46, 48);
const ROW_HOVER: Color32 = Color32::from_rgb(52, 52, 55);
const LINE: Color32 = Color32::from_rgb(56, 56, 58);
const DIM: Color32 = Color32::from_rgb(150, 150, 155);
const ERROR: Color32 = Color32::from_rgb(240, 110, 100);
const WARN: Color32 = Color32::from_rgb(232, 180, 80);

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Projects,
    NewProject,
    Installs,
}

#[derive(Clone, Copy, PartialEq)]
enum InstallsTab {
    Installed,
    Available,
}

#[derive(Clone, Copy)]
enum PickFor {
    AddProject,
    Location,
    Locate,
}

/// What a dialog button does once the dialog has closed.
type AfterDialog = Box<dyn FnOnce(&mut HubApp)>;

enum Dialog {
    /// The project's version isn't installed.
    MissingVersion {
        dir: PathBuf,
        version: String,
        other: Option<String>,
    },
    /// Opening a project with a different version than the one it was made with.
    ChangeVersion {
        dir: PathBuf,
        from: String,
        to: String,
    },
    Uninstall(Install),
}

pub struct HubApp {
    data: HubData,
    data_path: PathBuf,
    editors_dir: PathBuf,
    installs: Vec<Install>,
    manifest: Option<Manifest>,
    manifest_error: Option<String>,
    manifest_rx: Option<Receiver<Result<Manifest, String>>>,
    page: Page,
    search: String,
    new_name: String,
    new_location: String,
    new_version: Option<String>,
    installs_tab: InstallsTab,
    channel: Channel,
    downloads: Vec<InstallJob>,
    /// Open this project once this version finishes installing.
    open_after_install: Option<(PathBuf, String)>,
    picking: Option<(Receiver<Option<PathBuf>>, PickFor)>,
    job: Option<OpenJob>,
    dialog: Option<Dialog>,
    notice: Option<(String, bool)>,
    notice_at: Option<(String, Instant)>,
    rust: bool,
    icon: egui::TextureHandle,
    /// Test driver steps (`$OXIDE_HUB_AUTOMATE`), see [`HubApp::automate`].
    script: Vec<String>,
    step: usize,
    screenshot_to: Option<PathBuf>,
}

impl HubApp {
    pub fn new(ctx: &egui::Context) -> Self {
        style(ctx);
        let data_path = HubData::path();
        let mut data = HubData::load_from(&data_path);
        if let Ok(exe) = std::env::current_exe() {
            data.hub_exe = Some(exe);
            let _ = data.save_to(&data_path);
        }
        let img = image::load_from_memory(include_bytes!("../assets/icon.png"))
            .expect("icon")
            .resize(128, 128, image::imageops::FilterType::Lanczos3)
            .to_rgba8();
        let icon = ctx.load_texture(
            "hub-icon",
            egui::ColorImage::from_rgba_unmultiplied(
                [img.width() as usize, img.height() as usize],
                &img,
            ),
            egui::TextureOptions::LINEAR,
        );
        let mut hub = Self {
            data,
            data_path,
            editors_dir: installs::editors_dir(),
            installs: Vec::new(),
            manifest: None,
            manifest_error: None,
            manifest_rx: None,
            page: Page::Projects,
            search: String::new(),
            new_name: String::new(),
            new_location: String::new(),
            new_version: None,
            installs_tab: InstallsTab::Installed,
            channel: Channel::Production,
            downloads: Vec::new(),
            open_after_install: None,
            picking: None,
            job: None,
            dialog: None,
            notice: None,
            notice_at: None,
            rust: projects::rust_installed(),
            icon,
            script: std::env::var_os("OXIDE_HUB_AUTOMATE")
                .and_then(|p| std::fs::read_to_string(p).ok())
                .map(|t| {
                    t.lines()
                        .map(str::trim)
                        .filter(|l| !l.is_empty() && !l.starts_with('#'))
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            step: 0,
            screenshot_to: None,
        };
        hub.rescan();
        hub.refresh_manifest(ctx);
        hub
    }

    fn rescan(&mut self) {
        self.installs = installs::scan(&self.editors_dir);
        if let Some(i) = source_build()
            && !self.installs.iter().any(|x| x.root == i.root)
        {
            self.installs.push(i);
        }
    }

    fn refresh_manifest(&mut self, ctx: &egui::Context) {
        if self.manifest_rx.is_some() {
            return;
        }
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(manifest::fetch(&manifest::source()));
            ctx.request_repaint();
        });
        self.manifest_rx = Some(rx);
    }

    fn save(&mut self) {
        if let Err(e) = self.data.save_to(&self.data_path) {
            self.notice = Some((format!("Cannot save the project list: {e}"), true));
        }
    }

    fn installed(&self, version: &str) -> Option<&Install> {
        self.installs.iter().find(|i| i.version == version)
    }

    fn newest_install(&self) -> Option<&Install> {
        // Prefer production versions, as Unity Hub does for new projects.
        self.installs
            .iter()
            .find(|i| i.channel() == Channel::Production)
            .or(self.installs.first())
    }

    // -----------------------------------------------------------------------------------
    // Background work.

    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.manifest_rx
            && let Ok(r) = rx.try_recv()
        {
            self.manifest_rx = None;
            match r {
                Ok(m) => {
                    self.manifest = Some(m);
                    self.manifest_error = None;
                }
                Err(e) => self.manifest_error = Some(e),
            }
        }
        if let Some((rx, what)) = &self.picking
            && let Ok(answer) = rx.try_recv()
        {
            let what = *what;
            self.picking = None;
            if let Some(dir) = answer {
                match what {
                    PickFor::Location => self.new_location = dir.display().to_string(),
                    PickFor::AddProject => self.add_existing(&dir),
                    PickFor::Locate => match installs::locate(&dir) {
                        Ok(i) => {
                            self.notice = Some((format!("Added Oxide {}.", i.version), false));
                            self.rescan();
                        }
                        Err(e) => self.notice = Some((e, true)),
                    },
                }
            }
        }
        let mut finished = Vec::new();
        self.downloads.retain(|job| match job.stage() {
            Stage::Done(i) => {
                finished.push(Ok(i));
                false
            }
            Stage::Failed(e) => {
                finished.push(Err((job.version.clone(), e)));
                false
            }
            _ => true,
        });
        for r in finished {
            self.rescan();
            match r {
                Ok(i) => {
                    self.notice = Some((format!("Installed Oxide {}.", i.version), false));
                    if let Some((dir, v)) = self.open_after_install.take() {
                        if v == i.version {
                            self.launch(&dir, &v);
                        } else {
                            self.open_after_install = Some((dir, v));
                        }
                    }
                }
                Err((v, e)) => {
                    self.notice = Some((format!("Installing Oxide {v} failed: {e}"), true));
                    if self.open_after_install.as_ref().is_some_and(|o| o.1 == v) {
                        self.open_after_install = None;
                    }
                }
            }
        }
        if let Some(job) = self.job.as_mut()
            && job.error.is_none()
        {
            match job.poll() {
                None => {}
                Some(Err(e)) => job.error = Some(e),
                Some(Ok(())) => match projects::start_editor(&job.install, &job.dir) {
                    Ok(()) => {
                        self.job = None;
                        // Like Unity Hub after opening a project: get out of the way.
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    Err(e) => job.error = Some(e),
                },
            }
        }
        if !self.downloads.is_empty()
            || self.job.is_some()
            || self.picking.is_some()
            || self.manifest_rx.is_some()
        {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    fn pick(&mut self, what: PickFor, prompt: &'static str, start: Option<PathBuf>) {
        if self.picking.is_some() {
            return;
        }
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(projects::pick_folder(prompt, start.as_deref()));
        });
        self.picking = Some((rx, what));
    }

    fn add_existing(&mut self, dir: &Path) {
        if !projects::is_project(dir) {
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
        self.notice = Some((format!("Added {}.", projects::project_name(dir)), false));
    }

    fn start_install(&mut self, version: &str) {
        if self.downloads.iter().any(|j| j.version == version) {
            return;
        }
        let Some(dl) = self
            .manifest
            .as_ref()
            .and_then(|m| m.find(version))
            .and_then(|r| r.download())
            .cloned()
        else {
            self.notice = Some((
                format!(
                    "Oxide {version} has no download for this computer ({}).",
                    manifest::TARGET
                ),
                true,
            ));
            return;
        };
        self.downloads.push(InstallJob::start(
            version.to_owned(),
            dl,
            self.editors_dir.clone(),
        ));
    }

    /// Opens a project, with `chosen` if given (asking first when that changes the
    /// project's version), else with the version it was last opened with.
    fn request_open(&mut self, dir: &Path, chosen: Option<String>) {
        if self.job.is_some() {
            return;
        }
        if !projects::is_project(dir) {
            self.notice = Some((format!("{} can't be found.", dir.display()), true));
            return;
        }
        let current = projects::project_version(dir);
        if let (Some(to), Some(from)) = (&chosen, &current)
            && to != from
        {
            self.dialog = Some(Dialog::ChangeVersion {
                dir: dir.to_owned(),
                from: from.clone(),
                to: to.clone(),
            });
            return;
        }
        let Some(version) = chosen
            .or(current)
            .or_else(|| self.newest_install().map(|i| i.version.clone()))
        else {
            self.notice = Some(("Install an editor version first.".into(), true));
            self.page = Page::Installs;
            self.installs_tab = InstallsTab::Available;
            return;
        };
        if self.installed(&version).is_some() {
            self.launch(dir, &version);
        } else {
            self.dialog = Some(Dialog::MissingVersion {
                dir: dir.to_owned(),
                version,
                other: self.newest_install().map(|i| i.version.clone()),
            });
        }
    }

    fn launch(&mut self, dir: &Path, version: &str) {
        let Some(install) = self.installed(version).cloned() else {
            return;
        };
        if !self.rust {
            self.rust = projects::rust_installed();
        }
        self.data.remember(dir);
        self.save();
        self.job = Some(OpenJob::start(dir.to_owned(), install));
    }

    fn open_new_page(&mut self) {
        let location = self
            .data
            .location
            .clone()
            .filter(|l| l.is_dir())
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .unwrap_or_default();
        self.new_name = projects::free_name(&location, "My project");
        self.new_location = location.display().to_string();
        if self
            .new_version
            .as_deref()
            .and_then(|v| self.installed(v))
            .is_none()
        {
            self.new_version = self.newest_install().map(|i| i.version.clone());
        }
        self.page = Page::NewProject;
    }

    fn create(&mut self) {
        if let Some(p) = projects::new_project_problem(&self.new_location, &self.new_name) {
            self.notice = Some((p, true));
            return;
        }
        let Some(install) = self
            .new_version
            .as_deref()
            .and_then(|v| self.installed(v))
            .cloned()
        else {
            self.notice = Some(("Choose an editor version.".into(), true));
            return;
        };
        let location = PathBuf::from(self.new_location.trim());
        let dir = location.join(self.new_name.trim());
        if let Err(e) = projects::create_project(&install, &dir) {
            self.notice = Some((format!("Cannot create the project: {e}"), true));
            return;
        }
        self.data.location = Some(location);
        self.data.remember(&dir);
        self.save();
        self.page = Page::Projects;
        self.launch(&dir, &install.version);
    }

    // -----------------------------------------------------------------------------------
    // Pages.

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            ui.add(egui::Image::new(&self.icon).fit_to_exact_size(vec2(28.0, 28.0)));
            ui.label(
                RichText::new("Oxide Hub")
                    .size(17.0)
                    .strong()
                    .color(Color32::WHITE),
            );
        });
        ui.add_space(20.0);
        for (page, label) in [(Page::Projects, "Projects"), (Page::Installs, "Installs")] {
            let current =
                self.page == page || (page == Page::Projects && self.page == Page::NewProject);
            if nav_item(ui, label, current).clicked() {
                self.page = page;
            }
        }
        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.add_space(16.0);
                ui.label(
                    RichText::new(format!("Hub {}", env!("CARGO_PKG_VERSION")))
                        .size(11.0)
                        .color(DIM),
                );
            });
            if let Some(h) = self.manifest.as_ref().and_then(|m| m.hub_update()) {
                let v = h.version.clone();
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    let b = ui
                        .add(
                            egui::Button::new(
                                RichText::new(format!("⬆ Hub {v} is available"))
                                    .size(11.5)
                                    .color(WARN),
                            )
                            .frame(false),
                        )
                        .on_hover_text("Open the download page");
                    if b.clicked() {
                        projects::open_url(&format!(
                            "https://github.com/Jagatpratap/oxide/releases/tag/v{v}"
                        ));
                    }
                });
            }
        });
    }

    fn rust_banner(&mut self, ui: &mut egui::Ui) {
        if self.rust {
            return;
        }
        egui::Frame::new()
            .fill(Color32::from_rgb(70, 52, 30))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(egui::Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(
                            "Rust isn't installed. Oxide projects are Rust code, so the editor \
                             needs it to open them.",
                        )
                        .color(Color32::WHITE),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Check again").clicked() {
                            self.rust = projects::rust_installed();
                        }
                        if ui.button("Get Rust (rustup.rs)").clicked() {
                            projects::open_url("https://rustup.rs");
                        }
                    });
                });
            });
        ui.add_space(12.0);
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
                    .add_enabled(!busy, egui::Button::new("Open").min_size(vec2(0.0, 30.0)))
                    .on_hover_text("Add an existing project folder to this list")
                    .clicked()
                {
                    self.pick(PickFor::AddProject, "Select an Oxide project folder", None);
                }
            });
        });
        ui.add_space(12.0);
        self.rust_banner(ui);
        ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .hint_text("🔍 Search")
                .desired_width(280.0)
                .min_size(vec2(0.0, 26.0)),
        );
        ui.add_space(10.0);
        if self.data.projects.is_empty() {
            empty_state(
                ui,
                "No projects yet",
                "Create a new project, or Open a project folder you already have.",
            );
            return;
        }
        let w = ui.available_width();
        let (hdr, _) = ui.allocate_exact_size(vec2(w, 22.0), Sense::hover());
        let cols = columns(hdr);
        let p = ui.painter();
        let font = egui::FontId::proportional(11.5);
        for (r, t) in [
            (cols.name, "NAME"),
            (cols.version, "EDITOR VERSION"),
            (cols.opened, "LAST OPENED"),
        ] {
            p.text(
                r.left_center(),
                egui::Align2::LEFT_CENTER,
                t,
                font.clone(),
                DIM,
            );
        }
        p.hline(hdr.x_range(), hdr.bottom(), Stroke::new(1.0, LINE));

        let filter = self.search.trim().to_lowercase();
        let rows: Vec<HubProject> = self
            .data
            .projects
            .iter()
            .filter(|p| {
                filter.is_empty()
                    || p.path
                        .display()
                        .to_string()
                        .to_lowercase()
                        .contains(&filter)
            })
            .cloned()
            .collect();
        let versions: Vec<String> = self.installs.iter().map(|i| i.version.clone()).collect();
        let mut action = None;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                for proj in &rows {
                    let version = projects::project_version(&proj.path);
                    let installed = version
                        .as_deref()
                        .is_none_or(|v| versions.iter().any(|x| x == v));
                    if let Some(a) = project_row(ui, proj, version.as_deref(), installed, &versions)
                    {
                        action = Some((proj.path.clone(), a));
                    }
                }
                if rows.is_empty() {
                    ui.add_space(20.0);
                    ui.label(RichText::new("No projects match your search.").color(DIM));
                }
            });
        match action {
            None => {}
            Some((dir, RowAction::Open(with))) => self.request_open(&dir, with),
            Some((dir, RowAction::Show)) => {
                if let Err(e) = projects::show_folder(&dir) {
                    self.notice = Some((e, true));
                }
            }
            Some((dir, RowAction::Forget)) => {
                self.data.forget(&dir);
                self.save();
                self.notice = Some((
                    format!(
                        "Removed {} from the list (its files are kept).",
                        projects::project_name(&dir)
                    ),
                    false,
                ));
            }
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
        let area = ui.available_rect_before_wrap();
        let body =
            egui::Rect::from_min_max(area.min, egui::pos2(area.right(), area.bottom() - 50.0));
        let settings_w = 340.0;
        let settings =
            egui::Rect::from_min_max(egui::pos2(body.right() - settings_w, body.top()), body.max);
        let templates =
            egui::Rect::from_min_max(body.min, egui::pos2(settings.left() - 20.0, body.bottom()));
        ui.scope_builder(egui::UiBuilder::new().max_rect(templates), |ui| {
            ui.label(RichText::new("TEMPLATES").size(11.5).color(DIM));
            ui.add_space(6.0);
            template_card(
                ui,
                &self.icon,
                "3D (Core)",
                "An empty 3D project: a scene with a Main Camera and a Directional Light, and a \
                 Rust Scripts crate with an example script, ready to hot-reload.",
            );
        });
        ui.painter()
            .rect_filled(settings, CornerRadius::same(6), CARD);
        let mut browse = false;
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(settings.shrink(16.0)),
            |ui| {
                ui.label(RichText::new("PROJECT SETTINGS").size(11.5).color(DIM));
                ui.add_space(12.0);
                ui.label("Editor version");
                if self.installs.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("No editor installed.").color(ERROR));
                        if ui.link("Install one").clicked() {
                            self.page = Page::Installs;
                            self.installs_tab = InstallsTab::Available;
                        }
                    });
                } else {
                    let shown = self.new_version.clone().unwrap_or_default();
                    egui::ComboBox::from_id_salt("new-version")
                        .selected_text(version_label(&shown))
                        .width(ui.available_width())
                        .show_ui(ui, |ui| {
                            for i in &self.installs {
                                ui.selectable_value(
                                    &mut self.new_version,
                                    Some(i.version.clone()),
                                    version_label(&i.version),
                                );
                            }
                        });
                }
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
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_location)
                            .desired_width(ui.available_width() - 74.0)
                            .min_size(vec2(0.0, 24.0)),
                    );
                    browse = ui
                        .add_enabled(
                            self.picking.is_none(),
                            egui::Button::new("Browse...").min_size(vec2(68.0, 24.0)),
                        )
                        .clicked();
                });
                ui.add_space(10.0);
                let full = Path::new(self.new_location.trim()).join(self.new_name.trim());
                ui.add(
                    egui::Label::new(
                        RichText::new(projects::short_path(&full))
                            .size(11.0)
                            .color(DIM),
                    )
                    .wrap(),
                );
                if let Some(p) = projects::new_project_problem(&self.new_location, &self.new_name) {
                    ui.add_space(6.0);
                    ui.add(egui::Label::new(RichText::new(p).size(11.5).color(ERROR)).wrap());
                }
            },
        );
        if browse {
            let start = PathBuf::from(self.new_location.trim());
            self.pick(
                PickFor::Location,
                "Choose where to create the project",
                Some(start),
            );
        }
        ui.advance_cursor_after_rect(body);
        ui.separator();
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let ok = projects::new_project_problem(&self.new_location, &self.new_name)
                    .is_none()
                    && self.new_version.is_some()
                    && self.job.is_none();
                if ui.add_enabled(ok, primary("Create project")).clicked() {
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
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Installs")
                    .size(24.0)
                    .strong()
                    .color(Color32::WHITE),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if primary_button(ui, "Install Editor").clicked() {
                    self.installs_tab = InstallsTab::Available;
                }
                if ui
                    .add_enabled(
                        self.picking.is_none(),
                        egui::Button::new("Locate").min_size(vec2(0.0, 30.0)),
                    )
                    .on_hover_text(
                        "Add an editor folder you already have: an unpacked release download, or \
                         the engine source built with `cargo build -p oxide`",
                    )
                    .clicked()
                {
                    self.pick(PickFor::Locate, "Select an Oxide editor folder", None);
                }
            });
        });
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            for (tab, label) in [
                (
                    InstallsTab::Installed,
                    format!("Installed ({})", self.installs.len()),
                ),
                (InstallsTab::Available, "Available".to_owned()),
            ] {
                if tab_button(ui, &label, self.installs_tab == tab).clicked() {
                    self.installs_tab = tab;
                }
            }
        });
        ui.add_space(10.0);
        match self.installs_tab {
            InstallsTab::Installed => self.installed_list(ui),
            InstallsTab::Available => self.available_list(ui, &ctx),
        }
    }

    fn installed_list(&mut self, ui: &mut egui::Ui) {
        if self.installs.is_empty() && self.downloads.is_empty() {
            empty_state(
                ui,
                "No editors installed",
                "Install an editor version from the Available tab, or Locate one on disk.",
            );
            return;
        }
        let mut uninstall = None;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                for job in &self.downloads {
                    card(ui, |ui| {
                        ui.add(egui::Image::new(&self.icon).fit_to_exact_size(vec2(34.0, 34.0)));
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new(format!("Oxide {}", job.version))
                                    .size(15.0)
                                    .strong()
                                    .color(Color32::WHITE),
                            );
                            progress(ui, &job.stage());
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.button("Cancel").clicked() {
                                job.cancel();
                            }
                        });
                    });
                }
                for i in &self.installs {
                    card(ui, |ui| {
                        ui.add(egui::Image::new(&self.icon).fit_to_exact_size(vec2(34.0, 34.0)));
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!("Oxide {}", i.version))
                                        .size(15.0)
                                        .strong()
                                        .color(Color32::WHITE),
                                );
                                channel_badge(ui, i.channel());
                                if !i.managed {
                                    badge(ui, "Located", Color32::from_gray(90));
                                }
                            });
                            ui.label(
                                RichText::new(projects::short_path(&i.root))
                                    .size(11.5)
                                    .color(DIM),
                            );
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let menu = ui.add(egui::Button::new("•••").frame(false));
                            egui::Popup::menu(&menu).show(|ui| {
                                ui.set_min_width(170.0);
                                if ui.button(reveal_label()).clicked()
                                    && let Err(e) = projects::show_folder(&i.root)
                                {
                                    self.notice = Some((e, true));
                                }
                                if let Some(notes) = self
                                    .manifest
                                    .as_ref()
                                    .and_then(|m| m.find(&i.version))
                                    .map(|r| r.notes.clone())
                                    .filter(|n| !n.is_empty())
                                    && ui.button("Release notes").clicked()
                                {
                                    projects::open_url(&notes);
                                }
                                ui.separator();
                                let label = if i.managed {
                                    "Uninstall"
                                } else {
                                    "Remove from list"
                                };
                                if ui.button(label).clicked() {
                                    uninstall = Some(i.clone());
                                }
                            });
                        });
                    });
                }
            });
        if let Some(i) = uninstall {
            if i.managed {
                self.dialog = Some(Dialog::Uninstall(i));
            } else {
                match installs::remove(&i) {
                    Ok(()) => self.notice = Some((format!("Removed Oxide {}.", i.version), false)),
                    Err(e) => self.notice = Some((e, true)),
                }
                self.rescan();
            }
        }
    }

    fn available_list(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            for c in Channel::ALL {
                let n = self.manifest.as_ref().map_or(0, |m| m.offered(c).count());
                if tab_button(ui, &format!("{} ({n})", c.label()), self.channel == c).clicked() {
                    self.channel = c;
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let busy = self.manifest_rx.is_some();
                if ui
                    .add_enabled(!busy, egui::Button::new("⟳ Refresh"))
                    .clicked()
                {
                    self.refresh_manifest(ctx);
                }
                if busy {
                    ui.add(egui::Spinner::new().size(14.0));
                }
            });
        });
        ui.label(
            RichText::new(match self.channel {
                Channel::Production => {
                    "Production releases are tested and recommended for your projects."
                }
                Channel::Beta => {
                    "Beta releases preview the next version. Back up projects before opening them."
                }
                Channel::Alpha => {
                    "Alpha releases are early and may be unstable. Use them for testing only."
                }
            })
            .size(11.5)
            .color(DIM),
        );
        ui.add_space(8.0);
        if let Some(e) = self.manifest_error.clone() {
            ui.label(RichText::new(e).color(ERROR));
            ui.add_space(6.0);
        }
        let Some(m) = &self.manifest else {
            if self.manifest_rx.is_some() {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new());
                    ui.label("Loading the release list...");
                });
            }
            return;
        };
        let releases: Vec<_> = m.offered(self.channel).cloned().collect();
        if releases.is_empty() {
            empty_state(
                ui,
                &format!("No {} releases yet", self.channel.label().to_lowercase()),
                &format!(
                    "Nothing in this channel has a download for this computer ({}).",
                    manifest::TARGET
                ),
            );
            return;
        }
        let mut install = None;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                for r in &releases {
                    let job = self.downloads.iter().find(|j| j.version == r.version);
                    let have = self.installed(&r.version).is_some();
                    card(ui, |ui| {
                        ui.add(egui::Image::new(&self.icon).fit_to_exact_size(vec2(34.0, 34.0)));
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!("Oxide {}", r.version))
                                        .size(15.0)
                                        .strong()
                                        .color(Color32::WHITE),
                                );
                                channel_badge(ui, r.channel);
                            });
                            if let Some(job) = job {
                                progress(ui, &job.stage());
                            } else {
                                let size = r.download().map_or(0, |d| d.size);
                                let mut meta = r.date.clone();
                                if size > 0 {
                                    meta += &format!("  ·  {}", megabytes(size));
                                }
                                ui.label(RichText::new(meta).size(11.5).color(DIM));
                            }
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if job.is_some() {
                                ui.add(egui::Spinner::new());
                            } else if have {
                                ui.label(RichText::new("✔ Installed").color(DIM));
                            } else if primary_button(ui, "Install").clicked() {
                                install = Some(r.version.clone());
                            }
                            if !r.notes.is_empty() && ui.link("Release notes").clicked() {
                                projects::open_url(&r.notes);
                            }
                        });
                    });
                }
            });
        if let Some(v) = install {
            self.start_install(&v);
        }
    }

    // -----------------------------------------------------------------------------------
    // Dialogs.

    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(job) = self.job.as_mut() {
            let mut close = false;
            egui::Modal::new(egui::Id::new("hub-open")).show(ctx, |ui| {
                ui.set_width(480.0);
                ui.label(
                    RichText::new(format!("Opening {}", job.name))
                        .size(17.0)
                        .strong()
                        .color(Color32::WHITE),
                );
                ui.label(RichText::new(format!("with Oxide {}", job.install.version)).color(DIM));
                ui.add_space(8.0);
                if let Some(e) = &job.error {
                    ui.add(egui::Label::new(RichText::new(e).color(ERROR)).wrap());
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical()
                        .max_height(220.0)
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            let tail = job.log.len().saturating_sub(60);
                            ui.label(RichText::new(job.log[tail..].join("\n")).monospace().size(11.0));
                        });
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Close").clicked() {
                            close = true;
                        }
                        if e.starts_with("Rust is not installed") && ui.button("Get Rust").clicked() {
                            projects::open_url("https://rustup.rs");
                        }
                    });
                    return;
                }
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(18.0));
                    ui.add(egui::Label::new(&job.status).truncate());
                });
                ui.add_space(6.0);
                let secs = job.started.elapsed().as_secs();
                let mut detail = format!("{}:{:02}", secs / 60, secs % 60);
                if job.compiled > 0 {
                    detail += &format!(
                        " · {} crates compiled · an editor built from source compiles its engine on the first open, which takes a few minutes",
                        job.compiled
                    );
                }
                ui.add(egui::Label::new(RichText::new(detail).size(11.5).color(DIM)).wrap());
                ui.add_space(10.0);
                if ui.button("Cancel").clicked() {
                    job.cancel();
                    close = true;
                }
            });
            if close {
                self.job = None;
            }
            return;
        }
        let Some(dialog) = self.dialog.take() else {
            return;
        };
        let mut keep = true;
        let mut then: Option<AfterDialog> = None;
        egui::Modal::new(egui::Id::new("hub-dialog")).show(ctx, |ui| {
            ui.set_width(440.0);
            match &dialog {
                Dialog::MissingVersion { dir, version, other } => {
                    title(ui, "Editor version not installed");
                    ui.add(egui::Label::new(format!(
                        "{} uses Oxide {version}, which isn't installed.",
                        projects::project_name(dir)
                    )).wrap());
                    let downloadable = self
                        .manifest
                        .as_ref()
                        .and_then(|m| m.find(version))
                        .and_then(|r| r.download())
                        .is_some();
                    if !downloadable {
                        ui.label(
                            RichText::new("It isn't in the release list for this computer.").color(DIM),
                        );
                    }
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if downloadable && ui.add(primary(&format!("Install {version}"))).clicked() {
                            let (dir, version) = (dir.clone(), version.clone());
                            then = Some(Box::new(move |h: &mut Self| {
                                h.start_install(&version);
                                h.open_after_install = Some((dir, version));
                                h.notice = Some((
                                    "Installing... the project opens when it's done.".into(),
                                    false,
                                ));
                            }));
                            keep = false;
                        }
                        if let Some(o) = other
                            && ui.button(format!("Open with {o}")).clicked()
                        {
                            let (dir, o) = (dir.clone(), o.clone());
                            then = Some(Box::new(move |h: &mut Self| h.request_open(&dir, Some(o))));
                            keep = false;
                        }
                        if ui.button("Cancel").clicked() {
                            keep = false;
                        }
                    });
                }
                Dialog::ChangeVersion { dir, from, to } => {
                    title(ui, "Change editor version?");
                    ui.add(egui::Label::new(format!(
                        "{} was last opened with Oxide {from}. Opening it with Oxide {to} \
                         rebuilds its editor and Scripts against that version, and the project \
                         is saved for {to} from then on.",
                        projects::project_name(dir)
                    )).wrap());
                    if manifest::compare_versions(to, from).is_lt() {
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("This is an older version; files saved by newer versions may not load.")
                                .color(WARN),
                        );
                    }
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.add(primary(&format!("Open with {to}"))).clicked() {
                            let (dir, to) = (dir.clone(), to.clone());
                            then = Some(Box::new(move |h: &mut Self| h.launch(&dir, &to)));
                            keep = false;
                        }
                        if ui.button("Cancel").clicked() {
                            keep = false;
                        }
                    });
                }
                Dialog::Uninstall(i) => {
                    title(ui, &format!("Uninstall Oxide {}?", i.version));
                    ui.add(egui::Label::new(format!(
                        "Deletes {}. Projects that use this version can still be opened after \
                         installing it again.",
                        projects::short_path(&i.root)
                    )).wrap());
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        let danger = egui::Button::new(RichText::new("Uninstall").color(Color32::WHITE))
                            .fill(Color32::from_rgb(170, 60, 50));
                        if ui.add(danger).clicked() {
                            let i = i.clone();
                            then = Some(Box::new(move |h: &mut Self| {
                                match installs::remove(&i) {
                                    Ok(()) => {
                                        h.notice = Some((format!("Uninstalled Oxide {}.", i.version), false))
                                    }
                                    Err(e) => h.notice = Some((e, true)),
                                }
                                h.rescan();
                            }));
                            keep = false;
                        }
                        if ui.button("Cancel").clicked() {
                            keep = false;
                        }
                    });
                }
            }
        });
        if keep {
            self.dialog = Some(dialog);
        }
        if let Some(f) = then {
            f(self);
        }
    }

    fn notice_bar(&mut self, root: &mut egui::Ui) {
        let text = self.notice.as_ref().map(|n| n.0.clone());
        if self.notice_at.as_ref().map(|n| &n.0) != text.as_ref() {
            self.notice_at = text.map(|t| (t, Instant::now()));
        } else if let (Some((_, false)), Some((_, t))) = (&self.notice, &self.notice_at)
            && t.elapsed().as_secs() >= 6
        {
            self.notice = None;
        }
        let Some((text, error)) = self.notice.clone() else {
            return;
        };
        root.ctx().request_repaint_after(Duration::from_millis(500));
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
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::Label::new(RichText::new(text).color(Color32::WHITE)).wrap());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("×").clicked() {
                            self.notice = None;
                        }
                    });
                });
            });
    }
}

impl HubApp {
    /// Test driver: runs one step of the `$OXIDE_HUB_AUTOMATE` script per frame, through the
    /// same functions the buttons call, and prints `hub: ...` lines to stdout. Steps:
    /// `page projects|new|installs`, `tab installed|available`, `channel production|beta|alpha`,
    /// `install VERSION`, `uninstall VERSION`, `locate DIR`, `new LOCATION NAME VERSION`
    /// (then `create`), `open DIR [VERSION]`, `confirm` (a dialog's main button), `wait`
    /// (until downloads, the release list and opening finish), `state`, `screenshot FILE`,
    /// `quit`.
    fn automate(&mut self, ctx: &egui::Context) {
        if let Some(path) = &self.screenshot_to {
            let shot = ctx.input(|i| {
                i.raw.events.iter().find_map(|e| match e {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
            });
            match shot {
                Some(img) => {
                    let [w, h] = img.size;
                    let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
                    let r = image::save_buffer(
                        path,
                        &rgba,
                        w as u32,
                        h as u32,
                        image::ColorType::Rgba8,
                    );
                    println!("hub: screenshot {} {r:?}", path.display());
                    self.screenshot_to = None;
                }
                None => {
                    // A request made while the window was still hidden is dropped: ask again.
                    if ctx.cumulative_frame_nr().is_multiple_of(30) {
                        ctx.send_viewport_cmd(
                            egui::ViewportCommand::Screenshot(Default::default()),
                        );
                    }
                    ctx.request_repaint();
                    return;
                }
            }
        }
        let Some(line) = self.script.get(self.step).cloned() else {
            return;
        };
        ctx.request_repaint_after(Duration::from_millis(50));
        let words: Vec<&str> = line.split('|').map(str::trim).collect();
        let (cmd, args) = (words[0], &words[1..]);
        let arg = |i: usize| args.get(i).copied().unwrap_or("").to_owned();
        if cmd == "wait" {
            let busy = !self.downloads.is_empty()
                || self.manifest_rx.is_some()
                || self.job.as_ref().is_some_and(|j| j.error.is_none());
            if busy {
                return;
            }
        }
        self.step += 1;
        println!("hub: > {line}");
        match cmd {
            "page" => {
                self.page = match arg(0).as_str() {
                    "new" => {
                        self.open_new_page();
                        Page::NewProject
                    }
                    "installs" => Page::Installs,
                    _ => Page::Projects,
                }
            }
            "tab" => {
                self.installs_tab = if arg(0) == "available" {
                    InstallsTab::Available
                } else {
                    InstallsTab::Installed
                }
            }
            "channel" => {
                self.channel = match arg(0).as_str() {
                    "beta" => Channel::Beta,
                    "alpha" => Channel::Alpha,
                    _ => Channel::Production,
                }
            }
            "install" => self.start_install(&arg(0)),
            "uninstall" => {
                if let Some(i) = self.installed(&arg(0)).cloned() {
                    self.dialog = Some(Dialog::Uninstall(i));
                }
            }
            "locate" => match installs::locate(Path::new(&arg(0))) {
                Ok(_) => self.rescan(),
                Err(e) => self.notice = Some((e, true)),
            },
            "new" => {
                self.open_new_page();
                self.new_location = arg(0);
                self.new_name = arg(1);
                self.new_version = Some(arg(2));
            }
            "create" => self.create(),
            "open" => {
                let v = arg(1);
                self.request_open(Path::new(&arg(0)), (!v.is_empty()).then_some(v));
            }
            "confirm" => match self.dialog.take() {
                Some(Dialog::ChangeVersion { dir, to, .. }) => self.launch(&dir, &to),
                Some(Dialog::MissingVersion { dir, version, .. }) => {
                    self.start_install(&version);
                    self.open_after_install = Some((dir, version));
                }
                Some(Dialog::Uninstall(i)) => {
                    let _ = installs::remove(&i);
                    self.rescan();
                }
                None => println!("hub: no dialog"),
            },
            "state" => {
                let installs: Vec<_> = self
                    .installs
                    .iter()
                    .map(|i| format!("{}{}", i.version, if i.managed { "" } else { " (located)" }))
                    .collect();
                println!("hub: installs {installs:?}");
                if let Some(m) = &self.manifest {
                    for c in Channel::ALL {
                        let v: Vec<_> = m.offered(c).map(|r| r.version.as_str()).collect();
                        println!("hub: {} {v:?}", c.label());
                    }
                }
                println!("hub: manifest error {:?}", self.manifest_error);
                for p in self.data.projects.iter().take(5) {
                    println!(
                        "hub: project {} version {:?}",
                        p.path.display(),
                        projects::project_version(&p.path)
                    );
                }
                println!("hub: notice {:?}", self.notice);
                println!(
                    "hub: dialog {}",
                    match &self.dialog {
                        None => "none".into(),
                        Some(Dialog::MissingVersion { version, .. }) =>
                            format!("missing {version}"),
                        Some(Dialog::ChangeVersion { from, to, .. }) =>
                            format!("change {from} -> {to}"),
                        Some(Dialog::Uninstall(i)) => format!("uninstall {}", i.version),
                    }
                );
                if let Some(j) = &self.job {
                    println!(
                        "hub: job {} status {:?} error {:?}",
                        j.name, j.status, j.error
                    );
                }
            }
            "screenshot" => {
                // eframe only captures a window that's on screen.
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                self.screenshot_to = Some(PathBuf::from(arg(0)));
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
            "quit" => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            "wait" => {}
            other => println!("hub: unknown step `{other}`"),
        }
    }
}

impl eframe::App for HubApp {
    /// Background work runs here, not in `ui`: eframe skips `ui` while the window is hidden
    /// behind others, and a finished build must still start the editor.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll(ctx);
    }

    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        self.automate(&ctx);
        egui::Panel::left(egui::Id::new("hub-sidebar"))
            .exact_size(200.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(SIDEBAR))
            .show(root, |ui| self.sidebar(ui));
        self.notice_bar(root);
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(PAGE)
                    .inner_margin(egui::Margin::symmetric(28, 22)),
            )
            .show(root, |ui| match self.page {
                Page::Projects => self.projects_page(ui),
                Page::NewProject => self.new_project_page(ui),
                Page::Installs => self.installs_page(ui),
            });
        self.dialogs(&ctx);
    }
}

/// The engine tree around a Hub built from source (`<tree>/target/<profile>/oxide-hub` next
/// to a built `oxide`), listed as an install without having to Locate it.
fn source_build() -> Option<Install> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let dir = exe.parent()?;
    let root = dir.parent()?.parent()?;
    let launcher = dir.join(installs::EXE);
    if !launcher.is_file() || !root.join("crates/oxide_dylib/Cargo.toml").is_file() {
        return None;
    }
    let version = installs::launcher_version(&launcher, root).ok()?;
    Some(Install {
        version,
        root: root.to_owned(),
        exe: launcher,
        managed: false,
    })
}

// ---------------------------------------------------------------------------------------
// Look and widgets.

fn style(ctx: &egui::Context) {
    ctx.set_visuals(egui::Visuals::dark());
    ctx.global_style_mut(|s| {
        s.spacing.button_padding = vec2(10.0, 5.0);
        s.spacing.item_spacing = vec2(8.0, 6.0);
        let v = &mut s.visuals;
        v.panel_fill = PAGE;
        v.window_fill = CARD;
        v.extreme_bg_color = Color32::from_rgb(30, 30, 32);
        v.selection.bg_fill = ACCENT;
        v.hyperlink_color = Color32::from_rgb(232, 140, 100);
        v.widgets.inactive.corner_radius = CornerRadius::same(4);
        v.widgets.hovered.corner_radius = CornerRadius::same(4);
        v.widgets.active.corner_radius = CornerRadius::same(4);
        v.window_corner_radius = CornerRadius::same(8);
    });
}

fn reveal_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "Show in Finder"
    } else if cfg!(windows) {
        "Show in Explorer"
    } else {
        "Show in file manager"
    }
}

fn version_label(v: &str) -> String {
    match Channel::of_version(v) {
        Channel::Production => v.to_owned(),
        c => format!("{v}  ({})", c.label()),
    }
}

fn megabytes(bytes: u64) -> String {
    format!("{:.0} MB", bytes as f64 / 1e6)
}

fn title(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .size(17.0)
            .strong()
            .color(Color32::WHITE),
    );
    ui.add_space(6.0);
}

fn empty_state(ui: &mut egui::Ui, heading: &str, text: &str) {
    ui.add_space(60.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(heading).size(18.0).color(Color32::WHITE));
        ui.add_space(6.0);
        ui.label(RichText::new(text).color(DIM));
    });
}

fn card(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(CARD)
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(add);
        });
    ui.add_space(8.0);
}

fn badge(ui: &mut egui::Ui, text: &str, fill: Color32) {
    egui::Frame::new()
        .fill(fill)
        .corner_radius(CornerRadius::same(3))
        .inner_margin(egui::Margin::symmetric(6, 1))
        .show(ui, |ui| {
            ui.label(
                RichText::new(text)
                    .size(10.5)
                    .strong()
                    .color(Color32::WHITE),
            );
        });
}

fn channel_badge(ui: &mut egui::Ui, c: Channel) {
    match c {
        Channel::Production => {}
        Channel::Beta => badge(ui, "BETA", Color32::from_rgb(52, 110, 170)),
        Channel::Alpha => badge(ui, "ALPHA", Color32::from_rgb(150, 80, 160)),
    }
}

fn progress(ui: &mut egui::Ui, stage: &Stage) {
    let (fraction, text) = match stage {
        Stage::Downloading { done, total } if *total > 0 => (
            *done as f32 / *total as f32,
            format!("Downloading {} of {}", megabytes(*done), megabytes(*total)),
        ),
        Stage::Downloading { done, .. } => (0.0, format!("Downloading {}", megabytes(*done))),
        Stage::Verifying => (1.0, "Checking the download...".to_owned()),
        Stage::Unpacking { files } => (1.0, format!("Unpacking ({files} files)...")),
        Stage::Done(_) => (1.0, "Installed".to_owned()),
        Stage::Failed(e) => (0.0, e.clone()),
    };
    ui.add(
        egui::ProgressBar::new(fraction)
            .desired_width(320.0)
            .desired_height(6.0)
            .fill(ACCENT),
    );
    ui.label(RichText::new(text).size(11.5).color(DIM));
}

struct Columns {
    name: egui::Rect,
    version: egui::Rect,
    opened: egui::Rect,
    menu: egui::Rect,
}

fn columns(row: egui::Rect) -> Columns {
    let (menu_w, opened_w, version_w) = (36.0, 140.0, 190.0);
    let r = row.right();
    let menu = egui::Rect::from_x_y_ranges(r - menu_w..=r, row.y_range());
    let opened = egui::Rect::from_x_y_ranges(r - menu_w - opened_w..=r - menu_w, row.y_range());
    let version = egui::Rect::from_x_y_ranges(
        opened.left() - version_w..=opened.left() - 10.0,
        row.y_range(),
    );
    let name = egui::Rect::from_x_y_ranges(row.left() + 8.0..=version.left() - 8.0, row.y_range());
    Columns {
        name,
        version,
        opened,
        menu,
    }
}

enum RowAction {
    /// Open, with this version if one was picked from the version menu.
    Open(Option<String>),
    Show,
    Forget,
}

fn project_row(
    ui: &mut egui::Ui,
    proj: &HubProject,
    version: Option<&str>,
    installed: bool,
    versions: &[String],
) -> Option<RowAction> {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 54.0), Sense::click());
    let cols = columns(rect);
    let exists = projects::is_project(&proj.path);
    let mut action = None;

    // The version cell is a menu of installed versions (Unity Hub's version dropdown).
    let vtext = match version {
        Some(v) if installed => version_label(v),
        Some(v) => format!("⚠ {v}"),
        None => "Not set".to_owned(),
    };
    let vrect = egui::Rect::from_min_size(
        egui::pos2(cols.version.left(), rect.center().y - 13.0),
        vec2(cols.version.width(), 26.0),
    );
    let vbtn = ui
        .put(
            vrect,
            egui::Button::new(RichText::new(format!("{vtext}  ⏷")).size(12.5).color(
                if installed {
                    Color32::from_gray(215)
                } else {
                    WARN
                },
            ))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::new(1.0, LINE)),
        )
        .on_hover_text(if installed {
            "Choose the editor version to open this project with"
        } else {
            "This version isn't installed: pick another, or install it from Installs"
        });
    egui::Popup::menu(&vbtn).show(|ui| {
        ui.set_min_width(200.0);
        ui.label(RichText::new("Open with").size(11.0).color(DIM));
        if versions.is_empty() {
            ui.label(RichText::new("No editor installed").color(DIM));
        }
        for v in versions {
            let current = version == Some(v.as_str());
            if ui
                .add_enabled(exists, egui::Button::selectable(current, version_label(v)))
                .clicked()
            {
                action = Some(RowAction::Open(Some(v.clone())));
            }
        }
    });

    let menu_rect = egui::Rect::from_center_size(cols.menu.center(), vec2(26.0, 26.0));
    let menu = ui.put(menu_rect, egui::Button::new("•••").frame(false));
    egui::Popup::menu(&menu).show(|ui| {
        ui.set_min_width(170.0);
        if ui.add_enabled(exists, egui::Button::new("Open")).clicked() {
            action = Some(RowAction::Open(None));
        }
        if ui
            .add_enabled(proj.path.is_dir(), egui::Button::new(reveal_label()))
            .clicked()
        {
            action = Some(RowAction::Show);
        }
        ui.separator();
        if ui.button("Remove from list").clicked() {
            action = Some(RowAction::Forget);
        }
    });

    let p = ui.painter();
    if resp.hovered() || menu.hovered() || vbtn.hovered() {
        p.rect_filled(rect, CornerRadius::same(4), ROW_HOVER);
    }
    p.hline(
        rect.x_range(),
        rect.bottom(),
        Stroke::new(1.0, Color32::from_rgb(50, 50, 52)),
    );
    let name = projects::project_name(&proj.path);
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
    let pc = p.with_clip_rect(egui::Rect::from_x_y_ranges(
        text_x..=cols.name.right(),
        rect.y_range(),
    ));
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
        if exists {
            projects::short_path(&proj.path)
        } else {
            format!("Not found: {}", projects::short_path(&proj.path))
        },
        egui::FontId::proportional(11.0),
        if exists { DIM } else { ERROR },
    );
    p.text(
        cols.opened.left_center(),
        egui::Align2::LEFT_CENTER,
        projects::ago(proj.opened),
        egui::FontId::proportional(12.5),
        Color32::from_gray(200),
    );
    let resp = if exists {
        resp.on_hover_text("Open in the editor")
    } else {
        resp.on_hover_text("This folder was moved or deleted. Use ⋮ › Remove from list.")
    };
    if action.is_none() && resp.clicked() && exists {
        action = Some(RowAction::Open(None));
    }
    action
}

fn nav_item(ui: &mut egui::Ui, label: &str, current: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
    let p = ui.painter();
    let inner = rect.shrink2(vec2(8.0, 2.0));
    if current {
        p.rect_filled(inner, CornerRadius::same(4), Color32::from_rgb(48, 48, 50));
        p.rect_filled(
            egui::Rect::from_min_size(inner.left_top(), vec2(3.0, inner.height())),
            CornerRadius::same(1),
            ACCENT,
        );
    } else if resp.hovered() {
        p.rect_filled(inner, CornerRadius::same(4), Color32::from_rgb(40, 40, 42));
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

fn tab_button(ui: &mut egui::Ui, label: &str, current: bool) -> egui::Response {
    let text = RichText::new(label).size(13.0).color(if current {
        Color32::WHITE
    } else {
        Color32::from_gray(170)
    });
    let resp = ui.add(
        egui::Button::new(text)
            .frame(false)
            .min_size(vec2(0.0, 26.0)),
    );
    if current {
        let r = resp.rect;
        ui.painter()
            .hline(r.x_range(), r.bottom() + 1.0, Stroke::new(2.0, ACCENT));
    }
    resp
}

fn primary(text: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(text).color(Color32::WHITE).strong())
        .fill(ACCENT)
        .min_size(vec2(0.0, 30.0))
}

fn primary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let r = ui.add(primary(text));
    if r.hovered() {
        ui.painter().rect_stroke(
            r.rect,
            CornerRadius::same(4),
            Stroke::new(1.0, ACCENT_HOVER),
            egui::StrokeKind::Inside,
        );
    }
    r
}

fn template_card(ui: &mut egui::Ui, icon: &egui::TextureHandle, title: &str, text: &str) {
    egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(2.0, ACCENT))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width().min(440.0));
            ui.horizontal(|ui| {
                ui.add(egui::Image::new(icon).fit_to_exact_size(vec2(64.0, 64.0)));
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(title)
                            .size(15.0)
                            .strong()
                            .color(Color32::WHITE),
                    );
                    ui.add(egui::Label::new(RichText::new(text).color(DIM)).wrap());
                });
            });
        });
}
