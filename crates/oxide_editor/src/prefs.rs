//! Preferences window (Unity Edit → Preferences): per-user editor settings, shared by every
//! project and saved outside them (`~/Library/Application Support/Oxide/EditorPrefs.yaml` on
//! macOS, `%APPDATA%\Oxide` on Windows, `~/.config/oxide` elsewhere).
//! Reference: <https://docs.unity3d.com/Manual/Preferences.html>

use crate::EditorState;
use egui::Color32;
use oxide_core::debug::{self, LogType};
use oxide_engine::Engine;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorPrefs {
    // General
    /// Reimport assets edited outside the editor (Unity "Auto Refresh").
    pub auto_refresh: bool,
    /// Editor UI scale (Unity "UI Scaling").
    pub ui_scale: f32,
    // Console
    pub clear_on_play: bool,
    pub clear_on_build: bool,
    pub clear_on_recompile: bool,
    /// Record a call stack for logs / warnings / errors (Unity "Stack Trace Logging").
    pub stack_trace_logs: bool,
    pub stack_trace_warnings: bool,
    pub stack_trace_errors: bool,
    // Scene View (Grid and Snap)
    pub move_snap: f32,
    pub rotate_snap: f32,
    pub scale_snap: f32,
    // Colors
    pub play_mode_tint: [u8; 3],
    // External Tools
    /// Command that opens a source file; `{file}` and `{line}` are replaced. Empty opens
    /// files with the system's default app.
    pub script_editor: String,
    // Shortcuts
    /// Rebound shortcuts: command id → bindings (`Cmd+Shift+Z`). Only changed commands.
    pub shortcuts: std::collections::BTreeMap<String, Vec<String>>,
    // Game view sizes added with the size menu's "+"
    pub game_sizes: Vec<crate::game_view::CustomGameSize>,
    // Scene view overlays
    pub overlays: Vec<crate::overlays::OverlayLayout>,
    pub overlay_presets: std::collections::BTreeMap<String, Vec<crate::overlays::OverlayLayout>>,
}

impl Default for EditorPrefs {
    fn default() -> Self {
        Self {
            auto_refresh: true,
            ui_scale: 1.0,
            clear_on_play: true,
            clear_on_build: true,
            clear_on_recompile: false,
            stack_trace_logs: false,
            stack_trace_warnings: false,
            stack_trace_errors: true,
            move_snap: 0.25,
            rotate_snap: 15.0,
            scale_snap: 0.1,
            play_mode_tint: [62, 78, 102],
            script_editor: String::new(),
            shortcuts: Default::default(),
            game_sizes: Vec::new(),
            overlays: crate::overlays::default_layouts(),
            overlay_presets: Default::default(),
        }
    }
}

/// This user's Oxide settings folder, shared by every project and the Hub.
pub fn config_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let base = if cfg!(target_os = "macos") {
        home.map(|h| h.join("Library/Application Support/Oxide"))
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("Oxide"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(|x| PathBuf::from(x).join("oxide"))
            .or_else(|| home.map(|h| h.join(".config/oxide")))
    };
    base.unwrap_or_else(std::env::temp_dir)
}

impl EditorPrefs {
    /// Where preferences live for this user.
    pub fn default_path() -> PathBuf {
        config_dir().join("EditorPrefs.yaml")
    }

    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_yaml_ng::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let text = serde_yaml_ng::to_string(self).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| e.to_string())
    }

    pub fn play_tint(&self) -> Color32 {
        let [r, g, b] = self.play_mode_tint;
        Color32::from_rgb(r, g, b)
    }

    /// Pushes the settings that live elsewhere (Console stack traces, UI scale).
    pub fn apply(&self, ctx: &egui::Context) {
        debug::set_stack_trace_logging(LogType::Log, self.stack_trace_logs);
        debug::set_stack_trace_logging(LogType::Warning, self.stack_trace_warnings);
        debug::set_stack_trace_logging(LogType::Error, self.stack_trace_errors);
        debug::set_stack_trace_logging(LogType::Exception, self.stack_trace_errors);
        let scale = self.ui_scale.clamp(0.5, 3.0);
        if (ctx.zoom_factor() - scale).abs() > 1e-3 {
            ctx.set_zoom_factor(scale);
        }
    }

    /// The command line that opens `file` at `line` in the script editor.
    pub fn open_command(&self, file: &Path, line: u32) -> (String, Vec<String>) {
        let f = file.to_string_lossy();
        let template = self.script_editor.trim();
        if template.is_empty() {
            return if cfg!(target_os = "macos") {
                ("open".into(), vec![f.into_owned()])
            } else if cfg!(windows) {
                (
                    "cmd".into(),
                    vec!["/C".into(), "start".into(), String::new(), f.into_owned()],
                )
            } else {
                ("xdg-open".into(), vec![f.into_owned()])
            };
        }
        let mut parts: Vec<String> = template
            .split_whitespace()
            .map(|p| p.replace("{file}", &f).replace("{line}", &line.to_string()))
            .collect();
        if !template.contains("{file}") {
            parts.push(f.into_owned());
        }
        let program = parts.remove(0);
        (program, parts)
    }

    /// Opens a source file in the script editor (Console double-click, Project window).
    pub fn open_file(&self, file: &Path, line: u32) -> Result<(), String> {
        let (program, args) = self.open_command(file, line);
        std::process::Command::new(&program)
            .args(&args)
            .spawn()
            .map(drop)
            .map_err(|e| format!("cannot run `{program}`: {e}"))
    }
}

/// Finds a logged source path (`file!()` is relative to whichever workspace compiled it):
/// as given, then under the project, its Scripts crate, and the engine's source tree.
pub fn resolve_source(project: &Path, file: &str) -> Option<PathBuf> {
    if file.is_empty() {
        return None;
    }
    let p = Path::new(file);
    let engine_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    [
        p.to_path_buf(),
        project.join(p),
        project.join("Scripts").join(p),
        engine_root.join(p),
    ]
    .into_iter()
    .find(|c| c.is_file())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Page {
    #[default]
    General,
    Console,
    SceneView,
    Colors,
    ExternalTools,
}

pub const PAGES: [(Page, &str); 5] = [
    (Page::General, "General"),
    (Page::Console, "Console"),
    (Page::SceneView, "Scene View"),
    (Page::Colors, "Colors"),
    (Page::ExternalTools, "External Tools"),
];

pub fn ui(st: &mut EditorState, _engine: &mut Engine, ui: &mut egui::Ui) {
    let before = st.prefs.clone();
    let p = &mut st.prefs;
    crate::settings_window::two_pane(ui, "prefs", &PAGES, &mut st.prefs_page, |ui, page| {
        egui::Grid::new(("prefs-grid", page as u8))
            .num_columns(2)
            .spacing([16.0, 6.0])
            .show(ui, |ui| match page {
                Page::General => {
                    ui.label("Auto Refresh")
                        .on_hover_text("Reimport assets edited outside the editor");
                    ui.checkbox(&mut p.auto_refresh, "");
                    ui.end_row();
                    ui.label("UI Scaling");
                    ui.add(egui::Slider::new(&mut p.ui_scale, 0.75..=2.0).step_by(0.25));
                    ui.end_row();
                }
                Page::Console => {
                    ui.label("Clear on Play");
                    ui.checkbox(&mut p.clear_on_play, "");
                    ui.end_row();
                    ui.label("Stack Trace Logging");
                    ui.end_row();
                    for (label, v) in [
                        ("    Log", &mut p.stack_trace_logs),
                        ("    Warning", &mut p.stack_trace_warnings),
                        ("    Error", &mut p.stack_trace_errors),
                    ] {
                        ui.label(label);
                        egui::ComboBox::from_id_salt(label)
                            .selected_text(if *v { "Full" } else { "None" })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(v, false, "None");
                                ui.selectable_value(v, true, "Full");
                            });
                        ui.end_row();
                    }
                }
                Page::SceneView => {
                    ui.label("Move Snap");
                    ui.add(
                        egui::DragValue::new(&mut p.move_snap)
                            .range(0.001..=100.0)
                            .speed(0.01),
                    );
                    ui.end_row();
                    ui.label("Rotate Snap (°)");
                    ui.add(egui::DragValue::new(&mut p.rotate_snap).range(0.1..=180.0));
                    ui.end_row();
                    ui.label("Scale Snap");
                    ui.add(
                        egui::DragValue::new(&mut p.scale_snap)
                            .range(0.001..=10.0)
                            .speed(0.01),
                    );
                    ui.end_row();
                    ui.weak("Hold Ctrl (⌘ on Mac) while dragging a handle to snap.");
                    ui.end_row();
                }
                Page::Colors => {
                    ui.label("Playmode tint");
                    let mut c = p.play_tint();
                    if ui.color_edit_button_srgba(&mut c).changed() {
                        p.play_mode_tint = [c.r(), c.g(), c.b()];
                    }
                    ui.end_row();
                }
                Page::ExternalTools => {
                    ui.label("Script Editor");
                    ui.add(
                        egui::TextEdit::singleline(&mut p.script_editor)
                            .hint_text("system default")
                            .desired_width(280.0),
                    );
                    ui.end_row();
                    ui.label("");
                    ui.weak("e.g. `code -g {file}:{line}` or `zed {file}:{line}`");
                    ui.end_row();
                }
            });
        ui.add_space(12.0);
        if ui.button("Use Defaults").clicked() {
            let page_defaults = EditorPrefs::default();
            match page {
                Page::General => {
                    p.auto_refresh = page_defaults.auto_refresh;
                    p.ui_scale = page_defaults.ui_scale;
                }
                Page::Console => {
                    p.clear_on_play = page_defaults.clear_on_play;
                    p.stack_trace_logs = page_defaults.stack_trace_logs;
                    p.stack_trace_warnings = page_defaults.stack_trace_warnings;
                    p.stack_trace_errors = page_defaults.stack_trace_errors;
                }
                Page::SceneView => {
                    p.move_snap = page_defaults.move_snap;
                    p.rotate_snap = page_defaults.rotate_snap;
                    p.scale_snap = page_defaults.scale_snap;
                }
                Page::Colors => p.play_mode_tint = page_defaults.play_mode_tint,
                Page::ExternalTools => p.script_editor = page_defaults.script_editor,
            }
        }
    });
    if st.prefs != before {
        st.prefs_changed = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_commands() {
        let path = std::env::temp_dir().join(format!("oxide_prefs_{}.yaml", std::process::id()));
        let mut p = EditorPrefs {
            move_snap: 0.5,
            script_editor: "code -g {file}:{line}".into(),
            ..Default::default()
        };
        p.save(&path).unwrap();
        assert_eq!(EditorPrefs::load(&path), p);
        let (prog, args) = p.open_command(Path::new("/x/a.rs"), 12);
        assert_eq!(prog, "code");
        assert_eq!(args, vec!["-g", "/x/a.rs:12"]);
        p.script_editor = "subl".into();
        assert_eq!(p.open_command(Path::new("/x/a.rs"), 1).1, vec!["/x/a.rs"]);
        let _ = std::fs::remove_file(&path);
        // A missing file gives defaults.
        assert_eq!(EditorPrefs::load(&path), EditorPrefs::default());
    }

    #[test]
    fn resolves_engine_sources() {
        let p = resolve_source(
            Path::new("/nonexistent"),
            "crates/oxide_editor/src/prefs.rs",
        );
        assert!(p.is_some());
        assert!(resolve_source(Path::new("/nonexistent"), "nope.rs").is_none());
    }
}
