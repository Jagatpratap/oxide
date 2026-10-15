//! Console window: log/warning/error filters, collapse, search, and a detail pane.
//! Reference: <https://docs.unity3d.com/Manual/Console.html>

use crate::EditorState;
use egui::{Color32, RichText, Sense, vec2};
use oxide_core::debug::{self, LogEntry, LogType};

const KEEP: usize = 5000;

pub struct ConsoleState {
    entries: Vec<LogEntry>,
    last_seq: u64,
    show: [bool; 3],
    collapse: bool,
    pub clear_on_play: bool,
    search: String,
    selected: Option<u64>,
}

impl Default for ConsoleState {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            last_seq: 0,
            show: [true; 3],
            collapse: false,
            clear_on_play: true,
            search: String::new(),
            selected: None,
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

    fn pull(&mut self) {
        let new = debug::entries_since(self.last_seq, LogType::Log, 1000);
        if let Some(l) = new.last() {
            self.last_seq = l.seq;
        }
        self.entries.extend(new);
        if self.entries.len() > KEEP {
            let n = self.entries.len() - KEEP;
            self.entries.drain(..n);
        }
    }

    /// The newest entry (for the status bar).
    pub fn last(&mut self) -> Option<&LogEntry> {
        self.pull();
        self.entries.last()
    }
}

pub fn ui(st: &mut EditorState, ui: &mut egui::Ui) {
    let c = &mut st.console;
    c.pull();
    let mut counts = [0usize; 3];
    for e in &c.entries {
        counts[level(e.log_type)] += 1;
    }
    ui.horizontal(|ui| {
        if ui.button("Clear").clicked() {
            debug::clear();
            c.clear();
        }
        ui.toggle_value(&mut c.collapse, "Collapse");
        ui.toggle_value(&mut c.clear_on_play, "Clear on Play");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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

    let detail_h = if c.selected.is_some() { 90.0 } else { 0.0 };
    let list_h = (ui.available_height() - detail_h).max(40.0);
    let mut selected = c.selected;
    egui::ScrollArea::vertical()
        .id_salt("console-list")
        .max_height(list_h)
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show_rows(ui, 20.0, rows.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (n, (e, count)) in rows[range.clone()].iter().enumerate() {
                let (rect, resp) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::click());
                let bg = if selected == Some(e.seq) {
                    crate::theme::SELECTION_BG
                } else if (range.start + n) % 2 == 0 {
                    Color32::from_gray(52)
                } else {
                    Color32::from_gray(56)
                };
                ui.painter().rect_filled(rect, 0.0, bg);
                let lv = level(e.log_type);
                ui.painter()
                    .circle_filled(rect.left_center() + vec2(9.0, 0.0), 4.0, COLORS[lv]);
                let first_line = e.message.lines().next().unwrap_or("");
                ui.painter().text(
                    rect.left_center() + vec2(20.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    format!("[{:>7.2}] {first_line}", e.time),
                    egui::FontId::proportional(12.0),
                    COLORS[lv],
                );
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
                }
            }
        });
    c.selected = selected;
    if let Some(seq) = c.selected
        && let Some(e) = c.entries.iter().find(|e| e.seq == seq)
    {
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("console-detail")
            .max_height(detail_h)
            .show(ui, |ui| {
                ui.label(RichText::new(&e.message).color(COLORS[level(e.log_type)]));
                ui.weak(format!(
                    "{:?} from {} at {:.3}s",
                    e.log_type, e.source, e.time
                ));
            });
    }
}
