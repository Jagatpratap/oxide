//! Console window: log/warning/error filters, collapse, search, Error Pause, Clear on Play /
//! Build / Recompile, timestamps, one- or two-line entries, Copy, context objects (clicking an
//! entry logged with one pings it in the Hierarchy), Editor.log / Player.log, and a detail
//! pane with the call site and stack trace (double-click opens the source).
//! Reference: <https://docs.unity3d.com/Manual/Console.html>

use crate::EditorState;
use egui::{Color32, RichText, Sense, vec2};
use oxide_core::debug::{self, LogEntry, LogType};
use oxide_engine::Engine;
use std::path::Path;

const KEEP: usize = 5000;

pub struct ConsoleState {
    entries: Vec<LogEntry>,
    last_seq: u64,
    show: [bool; 3],
    collapse: bool,
    /// Pause Play mode when an error is logged (Unity "Error Pause").
    pub error_pause: bool,
    search: String,
    selected: Option<u64>,
    /// Unity's ⋮ menu: Show Timestamp, Log Entry 1 or 2 lines.
    show_timestamp: bool,
    two_lines: bool,
}

impl Default for ConsoleState {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            last_seq: 0,
            show: [true; 3],
            collapse: false,
            error_pause: false,
            search: String::new(),
            selected: None,
            show_timestamp: true,
            two_lines: false,
        }
    }
}

/// 0 = log, 1 = warning, 2 = error/exception.
fn level(t: LogType) -> usize {
    match t {
        LogType::Log => 0,
        LogType::Warning => 1,
        _ => 2,
    }
}

const COLORS: [Color32; 3] = [
    Color32::from_gray(200),
    Color32::from_rgb(255, 200, 60),
    Color32::from_rgb(255, 90, 80),
];

impl ConsoleState {
    pub fn clear(&mut self) {
        self.entries.clear();
        self.selected = None;
    }

    /// Fetches new entries; returns whether any of them is an error.
    fn pull(&mut self) -> bool {
        let new = debug::entries_since(self.last_seq, LogType::Log, 1000);
        if let Some(l) = new.last() {
            self.last_seq = l.seq;
        }
        let error = new.iter().any(|e| e.log_type >= LogType::Error);
        self.entries.extend(new);
        if self.entries.len() > KEEP {
            let n = self.entries.len() - KEEP;
            self.entries.drain(..n);
        }
        error
    }

    /// The newest entry (for the status bar).
    pub fn last(&mut self) -> Option<&LogEntry> {
        self.pull();
        self.entries.last()
    }
}

/// Opens `file:line` (as logged) in the script editor from Preferences.
fn open_source(prefs: &crate::prefs::EditorPrefs, project: &Path, file: &str, line: u32) -> String {
    match crate::prefs::resolve_source(project, file) {
        Some(p) => match prefs.open_file(&p, line) {
            Ok(()) => format!("Opened {}:{line}", p.display()),
            Err(e) => e,
        },
        None => format!("source file `{file}` not found"),
    }
}

/// Unity's `Editor.log` (`~/Library/Logs/Oxide/Editor.log` on macOS).
pub fn editor_log_path() -> std::path::PathBuf {
    debug::log_dir().join("Oxide/Editor.log")
}

/// The newest `Player.log` of a game built by Oxide (`<logs>/<Company>/<Product>/Player.log`).
fn player_log_path(engine: &Engine) -> std::path::PathBuf {
    let a = engine.world.resource::<oxide_scene::Application>();
    let company = if a.company_name.is_empty() {
        "DefaultCompany"
    } else {
        &a.company_name
    };
    debug::log_dir()
        .join(company)
        .join(&a.product_name)
        .join("Player.log")
}

fn entry_text(e: &LogEntry, with_stack: bool) -> String {
    let mut s = e.message.clone();
    if !e.file.is_empty() {
        s.push_str(&format!("\n(at {}:{})", e.file, e.line));
    }
    if with_stack && !e.stack_trace.is_empty() {
        s.push('\n');
        s.push_str(&e.stack_trace);
    }
    s
}

/// `path:line:col` at the end of a stack frame's "at" line.
fn frame_location(at: &str) -> Option<(&str, u32)> {
    let mut parts = at.rsplitn(3, ':');
    let a = parts.next()?;
    let b = parts.next()?;
    match parts.next() {
        Some(file) => Some((file, b.parse().ok()?)),
        None => Some((b, a.parse().ok()?)),
    }
}

pub fn ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    let project = engine
        .world
        .resource::<oxide_assets::AssetServer>()
        .full_path("");
    let playing = st.play == crate::PlayState::Playing;
    let c = &mut st.console;
    if c.pull() && c.error_pause && playing && !engine.paused {
        engine.paused = true;
        st.status = "Paused on error (Error Pause)".into();
    }
    let player_log = player_log_path(engine);
    let c = &mut st.console;
    let mut open: Option<(String, u32)> = None;
    let mut ping: Option<(u64, bool)> = None;
    let mut counts = [0usize; 3];
    for e in &c.entries {
        counts[level(e.log_type)] += 1;
    }
    ui.horizontal(|ui| {
        if ui.button("Clear").clicked() {
            debug::clear();
            c.clear();
        }
        ui.menu_button("▾", |ui| {
            let p = &mut st.prefs;
            let mut ch = false;
            ch |= ui.checkbox(&mut p.clear_on_play, "Clear on Play").changed();
            ch |= ui
                .checkbox(&mut p.clear_on_build, "Clear on Build")
                .changed();
            ch |= ui
                .checkbox(&mut p.clear_on_recompile, "Clear on Recompile")
                .changed();
            st.prefs_changed |= ch;
        })
        .response
        .on_hover_text("Clear options");
        ui.toggle_value(&mut c.collapse, "Collapse");
        ui.toggle_value(&mut c.error_pause, "Error Pause")
            .on_hover_text("Pause Play mode when an error is logged");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.menu_button("⋮", |ui| {
                if ui.button("Open Player Log").clicked() {
                    let r = crate::asset_ops::reveal(&player_log);
                    st.status = match r {
                        Ok(()) => format!("{}", player_log.display()),
                        Err(e) => e,
                    };
                    ui.close();
                }
                if ui.button("Open Editor Log").clicked() {
                    st.status = match crate::asset_ops::reveal(&editor_log_path()) {
                        Ok(()) => format!("{}", editor_log_path().display()),
                        Err(e) => e,
                    };
                    ui.close();
                }
                ui.separator();
                ui.checkbox(&mut c.show_timestamp, "Show Timestamp");
                ui.menu_button("Log Entry", |ui| {
                    ui.radio_value(&mut c.two_lines, false, "1 Line");
                    ui.radio_value(&mut c.two_lines, true, "2 Lines");
                });
                ui.menu_button("Stack Trace Logging", |ui| {
                    let p = &mut st.prefs;
                    let mut ch = false;
                    ch |= ui.checkbox(&mut p.stack_trace_logs, "Log").changed();
                    ch |= ui
                        .checkbox(&mut p.stack_trace_warnings, "Warning")
                        .changed();
                    ch |= ui.checkbox(&mut p.stack_trace_errors, "Error").changed();
                    if ch {
                        debug::set_stack_trace_logging(LogType::Log, p.stack_trace_logs);
                        debug::set_stack_trace_logging(LogType::Warning, p.stack_trace_warnings);
                        debug::set_stack_trace_logging(LogType::Error, p.stack_trace_errors);
                        st.prefs_changed = true;
                    }
                });
            });
            for (i, label) in [(2usize, "Errors"), (1, "Warnings"), (0, "Logs")] {
                let text = RichText::new(format!("{label} {}", counts[i])).color(COLORS[i]);
                ui.toggle_value(&mut c.show[i], text);
            }
            ui.add(
                egui::TextEdit::singleline(&mut c.search)
                    .hint_text("Search")
                    .desired_width(160.0),
            );
        });
    });
    ui.separator();

    let q = c.search.trim().to_lowercase();
    let visible: Vec<&LogEntry> = c
        .entries
        .iter()
        .filter(|e| c.show[level(e.log_type)])
        .filter(|e| q.is_empty() || e.message.to_lowercase().contains(&q))
        .collect();
    // Collapse: one row per distinct (type, message), with a count, in first-seen order.
    let rows: Vec<(&LogEntry, usize)> = if c.collapse {
        let mut out: Vec<(&LogEntry, usize)> = Vec::new();
        let mut index: std::collections::HashMap<(usize, &str), usize> =
            std::collections::HashMap::new();
        for e in visible {
            match index.get(&(level(e.log_type), e.message.as_str())) {
                Some(&i) => out[i].1 += 1,
                None => {
                    index.insert((level(e.log_type), e.message.as_str()), out.len());
                    out.push((e, 1));
                }
            }
        }
        out
    } else {
        visible.into_iter().map(|e| (e, 1)).collect()
    };

    let detail_h = if c.selected.is_some() { 140.0 } else { 0.0 };
    let list_h = (ui.available_height() - detail_h).max(40.0);
    let mut selected = c.selected;
    let row_h = if c.two_lines { 34.0 } else { 20.0 };
    let (show_time, two_lines) = (c.show_timestamp, c.two_lines);
    let mut copy: Option<String> = None;
    egui::ScrollArea::vertical()
        .id_salt("console-list")
        .max_height(list_h)
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show_rows(ui, row_h, rows.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (n, (e, count)) in rows[range.clone()].iter().enumerate() {
                let (rect, resp) =
                    ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
                let bg = if selected == Some(e.seq) {
                    crate::theme::SELECTION_BG
                } else if (range.start + n) % 2 == 0 {
                    Color32::from_gray(52)
                } else {
                    Color32::from_gray(56)
                };
                ui.painter().rect_filled(rect, 0.0, bg);
                let lv = level(e.log_type);
                let top = rect.left_top() + vec2(0.0, 10.0);
                ui.painter()
                    .circle_filled(top + vec2(9.0, 0.0), 4.0, COLORS[lv]);
                let mut lines = e.message.lines();
                let first_line = lines.next().unwrap_or("");
                let stamp = if show_time {
                    format!("[{:>7.2}] ", e.time)
                } else {
                    String::new()
                };
                ui.painter().text(
                    top + vec2(20.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    format!("{stamp}{first_line}"),
                    egui::FontId::proportional(12.0),
                    COLORS[lv],
                );
                if two_lines {
                    let second = lines
                        .next()
                        .map(str::to_owned)
                        .or_else(|| (!e.file.is_empty()).then(|| format!("{}:{}", e.file, e.line)))
                        .unwrap_or_default();
                    ui.painter().text(
                        top + vec2(20.0, 14.0),
                        egui::Align2::LEFT_CENTER,
                        second,
                        egui::FontId::proportional(11.0),
                        Color32::from_gray(140),
                    );
                }
                if e.context.is_some() {
                    ui.painter().text(
                        rect.right_center() - vec2(if *count > 1 { 34.0 } else { 8.0 }, 0.0),
                        egui::Align2::RIGHT_CENTER,
                        "⊙",
                        egui::FontId::proportional(11.0),
                        Color32::from_gray(150),
                    );
                }
                if *count > 1 {
                    ui.painter().text(
                        rect.right_center() - vec2(8.0, 0.0),
                        egui::Align2::RIGHT_CENTER,
                        count.to_string(),
                        egui::FontId::proportional(11.0),
                        Color32::from_gray(160),
                    );
                }
                if resp.clicked() {
                    selected = Some(e.seq);
                    if let Some(ctx) = e.context {
                        ping = Some((ctx, false));
                    }
                }
                resp.context_menu(|ui| {
                    if ui.button("Copy").clicked() {
                        copy = Some(entry_text(e, false));
                        ui.close();
                    }
                    if ui.button("Copy with Stack Trace").clicked() {
                        copy = Some(entry_text(e, true));
                        ui.close();
                    }
                    if let Some(ctx) = e.context
                        && ui.button("Select Context Object").clicked()
                    {
                        ping = Some((ctx, true));
                        ui.close();
                    }
                });
                if resp.double_clicked() && !e.file.is_empty() {
                    open = Some((e.file.clone(), e.line));
                }
            }
        });
    c.selected = selected;
    if let Some(t) = copy {
        ui.ctx().copy_text(t);
    }
    if let Some(seq) = c.selected
        && let Some(e) = c.entries.iter().find(|e| e.seq == seq)
    {
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("console-detail")
            .max_height(detail_h)
            .show(ui, |ui| {
                ui.label(RichText::new(&e.message).color(COLORS[level(e.log_type)]));
                ui.horizontal(|ui| {
                    ui.weak(format!(
                        "{:?} from {} at {:.3}s",
                        e.log_type, e.source, e.time
                    ));
                    if let Some(ctx) = e.context {
                        let id = oxide_scene::GameObjectId::from_bits(ctx);
                        let name = engine
                            .world
                            .get(id)
                            .map_or("(destroyed)".to_owned(), |o| o.name.clone());
                        if ui
                            .link(format!("Context: {name}"))
                            .on_hover_text("Select this object")
                            .clicked()
                        {
                            ping = Some((ctx, true));
                        }
                    }
                    if !e.file.is_empty()
                        && ui
                            .link(format!("{}:{}", e.file, e.line))
                            .on_hover_text("Open in the script editor")
                            .clicked()
                    {
                        open = Some((e.file.clone(), e.line));
                    }
                });
                if !e.stack_trace.is_empty() {
                    ui.add_space(4.0);
                    for line in e.stack_trace.lines() {
                        match line.trim_start().strip_prefix("at ") {
                            Some(at) => match frame_location(at) {
                                Some((file, l)) => {
                                    if ui
                                        .add(
                                            egui::Label::new(
                                                RichText::new(format!("    at {at}"))
                                                    .monospace()
                                                    .size(11.0)
                                                    .color(Color32::from_rgb(110, 160, 230)),
                                            )
                                            .sense(Sense::click()),
                                        )
                                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                                        .clicked()
                                    {
                                        open = Some((file.to_owned(), l));
                                    }
                                }
                                None => {
                                    ui.monospace(line);
                                }
                            },
                            None => {
                                ui.label(RichText::new(line).monospace().size(11.0));
                            }
                        }
                    }
                }
            });
    }
    if let Some((file, line)) = open {
        st.status = open_source(&st.prefs, &project, &file, line);
    }
    // Unity pings the context object in the Hierarchy; Select makes it the selection.
    if let Some((bits, select)) = ping {
        let id = oxide_scene::GameObjectId::from_bits(bits);
        if engine.world.is_alive(id) {
            st.hierarchy.reveal(&engine.world, id);
            if select {
                st.select(id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn frame_locations() {
        use super::frame_location;
        assert_eq!(frame_location("./src/a.rs:12:5"), Some(("./src/a.rs", 12)));
        assert_eq!(frame_location("C:\\x\\a.rs:3:1"), Some(("C:\\x\\a.rs", 3)));
        assert_eq!(frame_location("src/a.rs:7"), Some(("src/a.rs", 7)));
        assert_eq!(frame_location("nothing"), None);
    }
}
