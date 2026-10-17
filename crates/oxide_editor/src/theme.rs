//! Unity's dark editor skin.

use egui::{Color32, CornerRadius, Margin, Stroke};

pub const SELECTION_BG: Color32 = Color32::from_rgb(44, 93, 135);
const PANEL: Color32 = Color32::from_rgb(56, 56, 56);
const BAR: Color32 = Color32::from_rgb(40, 40, 40);
const FIELD: Color32 = Color32::from_rgb(42, 42, 42);
const TEXT: Color32 = Color32::from_rgb(210, 210, 210);

pub fn apply(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = PANEL;
    v.window_fill = PANEL;
    v.faint_bg_color = Color32::from_rgb(60, 60, 60);
    v.extreme_bg_color = FIELD;
    v.selection.bg_fill = SELECTION_BG;
    v.selection.stroke = Stroke::new(1.0, Color32::from_rgb(120, 170, 230));
    v.hyperlink_color = Color32::from_rgb(120, 170, 230);
    let r = CornerRadius::same(2);
    v.widgets.noninteractive.bg_fill = PANEL;
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(35, 35, 35));
    v.widgets.inactive.bg_fill = Color32::from_rgb(88, 88, 88);
    v.widgets.inactive.weak_bg_fill = Color32::from_rgb(70, 70, 70);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.inactive.corner_radius = r;
    // Unity's fields and buttons have a thin dark outline.
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(33, 33, 33));
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(33, 33, 33));
    // No guide line down the left of indented content (component fields, foldouts).
    v.indent_has_left_vline = false;
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(90, 90, 90);
    v.widgets.hovered.bg_fill = Color32::from_rgb(103, 103, 103);
    v.widgets.hovered.corner_radius = r;
    v.widgets.active.weak_bg_fill = Color32::from_rgb(70, 96, 124);
    v.widgets.active.bg_fill = Color32::from_rgb(70, 96, 124);
    v.widgets.active.corner_radius = r;
    v.widgets.open.weak_bg_fill = Color32::from_rgb(70, 96, 124);
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        // Unity's labels aren't text: dragging across them must not select their words.
        s.interaction.selectable_labels = false;
        s.interaction.multi_widget_text_select = false;
        s.spacing.item_spacing = egui::vec2(6.0, 3.0);
        s.spacing.interact_size.y = 18.0;
        s.spacing.button_padding = egui::vec2(6.0, 2.0);
        for (style, size) in [
            (egui::TextStyle::Body, 12.5),
            (egui::TextStyle::Button, 12.5),
            (egui::TextStyle::Small, 10.5),
            (egui::TextStyle::Monospace, 12.0),
            (egui::TextStyle::Heading, 16.0),
        ] {
            if let Some(f) = s.text_styles.get_mut(&style) {
                f.size = size;
            }
        }
    });
}

/// Frame for the menu bar, toolbar and status bar (tinted in Play mode, like Unity).
/// Menu/tool/status bar; `tint` is the Play-mode tint (Preferences → Colors).
pub fn bar_frame(tint: Option<Color32>) -> egui::Frame {
    egui::Frame::new()
        .fill(tint.unwrap_or(BAR))
        .inner_margin(Margin::symmetric(6, 3))
}

pub fn dock_style(style: &egui::Style) -> egui_dock::Style {
    let mut s = egui_dock::Style::from_egui(style);
    s.tab_bar.bg_fill = BAR;
    s.tab_bar.height = 22.0;
    s.tab_bar.corner_radius = CornerRadius::ZERO;
    // The overflow scroll bar is drawn over the window's first row; scroll tabs with the wheel.
    s.tab_bar.show_scroll_bar_on_overflow = false;
    s.tab.tab_body.bg_fill = PANEL;
    s.tab.tab_body.inner_margin = Margin::same(2);
    s.tab.hline_below_active_tab_name = false;
    for t in [
        &mut s.tab.active,
        &mut s.tab.focused,
        &mut s.tab.active_with_kb_focus,
        &mut s.tab.focused_with_kb_focus,
    ] {
        t.bg_fill = PANEL;
        t.text_color = Color32::WHITE;
        t.corner_radius = CornerRadius {
            nw: 3,
            ne: 3,
            sw: 0,
            se: 0,
        };
    }
    for t in [
        &mut s.tab.inactive,
        &mut s.tab.inactive_with_kb_focus,
        &mut s.tab.hovered,
    ] {
        t.bg_fill = Color32::from_rgb(48, 48, 48);
        t.text_color = Color32::from_gray(170);
    }
    s.separator.color_idle = Color32::from_rgb(25, 25, 25);
    s.separator.width = 2.0;
    s
}

/// Unity's padlock button, drawn rather than taken from the font (the emoji font's padlock
/// is unreadable at Inspector size). Highlighted when `locked`; toggle on `clicked()`.
pub fn lock_button(ui: &mut egui::Ui, locked: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::click());
    let p = ui.painter();
    if locked {
        p.rect_filled(rect, CornerRadius::same(2), SELECTION_BG);
    } else if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(2), Color32::from_gray(80));
    }
    let col = if locked || resp.hovered() {
        Color32::WHITE
    } else {
        Color32::from_gray(190)
    };
    let c = rect.center();
    // Body.
    let body = egui::Rect::from_center_size(c + egui::vec2(0.0, 2.5), egui::vec2(9.0, 6.5));
    p.rect_filled(body, CornerRadius::same(1), col);
    // Shackle: closed when locked, swung open (raised on the right) when not.
    let lift = if locked { 0.0 } else { 2.0 };
    let (l, r, top) = (body.left() + 2.0, body.right() - 2.0, body.top() - 4.5);
    let s = Stroke::new(1.4, col);
    p.line_segment([egui::pos2(l, body.top()), egui::pos2(l, top + 1.5)], s);
    p.line_segment(
        [
            egui::pos2(r, body.top() - lift),
            egui::pos2(r, top + 1.5 - lift),
        ],
        s,
    );
    let arc: Vec<egui::Pos2> = (0..=8)
        .map(|i| {
            let a = std::f32::consts::PI * (1.0 + i as f32 / 8.0);
            let cx = (l + r) * 0.5;
            let rad = (r - l) * 0.5;
            let t = i as f32 / 8.0;
            egui::pos2(cx + rad * a.cos(), top + 1.5 + rad * a.sin() - lift * t)
        })
        .collect();
    p.add(egui::Shape::line(arc, s));
    resp
}
