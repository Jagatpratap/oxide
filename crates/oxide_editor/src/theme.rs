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
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(90, 90, 90);
    v.widgets.hovered.bg_fill = Color32::from_rgb(103, 103, 103);
    v.widgets.hovered.corner_radius = r;
    v.widgets.active.weak_bg_fill = Color32::from_rgb(70, 96, 124);
    v.widgets.active.bg_fill = Color32::from_rgb(70, 96, 124);
    v.widgets.active.corner_radius = r;
    v.widgets.open.weak_bg_fill = Color32::from_rgb(70, 96, 124);
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
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
