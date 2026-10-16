//! Game view: the game as players see it, with Unity's control bar (aspect ratios and fixed
//! resolutions, Scale, Maximize On Play, Mute Audio, Stats).
//! Reference: <https://docs.unity3d.com/Manual/GameView.html>

use crate::EditorState;
use oxide_engine::Engine;

/// Unity's Game view size menu: free, an aspect ratio, or a fixed resolution.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GameSize {
    Free,
    Aspect(f32),
    Fixed(u32, u32),
}

pub const SIZES: &[(&str, GameSize)] = &[
    ("Free Aspect", GameSize::Free),
    ("16:9 Aspect", GameSize::Aspect(16.0 / 9.0)),
    ("16:10 Aspect", GameSize::Aspect(16.0 / 10.0)),
    ("4:3 Aspect", GameSize::Aspect(4.0 / 3.0)),
    ("21:9 Aspect", GameSize::Aspect(21.0 / 9.0)),
    ("9:16 Portrait", GameSize::Aspect(9.0 / 16.0)),
    ("Full HD (1920x1080)", GameSize::Fixed(1920, 1080)),
    ("HD (1280x720)", GameSize::Fixed(1280, 720)),
    ("QHD (2560x1440)", GameSize::Fixed(2560, 1440)),
    ("4K UHD (3840x2160)", GameSize::Fixed(3840, 2160)),
    ("Phone Portrait (1080x2400)", GameSize::Fixed(1080, 2400)),
    ("Phone Landscape (2400x1080)", GameSize::Fixed(2400, 1080)),
];

pub struct GameViewState {
    pub size: GameSize,
    /// Zoom: 1 = the largest size that fits; above 1 magnifies the center.
    pub scale: f32,
    pub maximize_on_play: bool,
    pub mute_audio: bool,
    pub stats: bool,
    /// Game pixels per window pixel for the last frame (mouse mapping).
    pub pixel_scale: f32,
}

impl Default for GameViewState {
    fn default() -> Self {
        Self {
            size: GameSize::Free,
            scale: 1.0,
            maximize_on_play: false,
            mute_audio: false,
            stats: false,
            pixel_scale: 1.0,
        }
    }
}

impl GameViewState {
    fn size_label(&self) -> String {
        SIZES
            .iter()
            .find(|(_, s)| *s == self.size)
            .map_or_else(|| format!("{:?}", self.size), |(n, _)| (*n).to_owned())
    }
}

/// Where the game image goes in `avail` (points) and the render size (pixels), for a size
/// setting, zoom and pixels-per-point.
pub fn layout(avail: egui::Rect, size: GameSize, scale: f32, ppp: f32) -> (egui::Rect, [u32; 2]) {
    let fit = |aspect: f32| {
        if avail.width() / avail.height().max(1.0) > aspect {
            egui::vec2(avail.height() * aspect, avail.height())
        } else {
            egui::vec2(avail.width(), avail.width() / aspect)
        }
    };
    let (shown, pixels) = match size {
        GameSize::Free => {
            let px = [
                (avail.width() * ppp).round() as u32,
                (avail.height() * ppp).round() as u32,
            ];
            (avail.size(), px)
        }
        GameSize::Aspect(a) => {
            let s = fit(a);
            (s, [(s.x * ppp).round() as u32, (s.y * ppp).round() as u32])
        }
        GameSize::Fixed(w, h) => (fit(w as f32 / h as f32), [w, h]),
    };
    let rect = egui::Rect::from_center_size(avail.center(), shown * scale.max(0.1));
    (rect, pixels)
}

pub fn ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    let g = &mut st.game;
    ui.horizontal(|ui| {
        ui.label("Display 1");
        egui::ComboBox::from_id_salt("game-size")
            .selected_text(g.size_label())
            .width(170.0)
            .show_ui(ui, |ui| {
                for (name, s) in SIZES {
                    ui.selectable_value(&mut g.size, *s, *name);
                }
            });
        ui.label("Scale");
        ui.add(
            egui::Slider::new(&mut g.scale, 1.0..=5.0)
                .max_decimals(1)
                .suffix("x"),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.toggle_value(&mut g.stats, "Stats");
            ui.toggle_value(&mut g.mute_audio, "Mute Audio");
            ui.toggle_value(&mut g.maximize_on_play, "Maximize On Play")
                .on_hover_text("Fill the editor with the Game view while playing");
        });
    });
    if let Some(a) = engine
        .world
        .try_resource_mut::<oxide_engine::prelude::Audio>()
    {
        a.mute = g.mute_audio;
    }
    let avail = ui.available_rect_before_wrap();
    let resp = ui.allocate_rect(avail, egui::Sense::click());
    ui.painter().rect_filled(avail, 0.0, egui::Color32::BLACK);
    let ppp = ui.ctx().pixels_per_point();
    let (rect, [pw, ph]) = layout(avail, g.size, g.scale, ppp);
    if rect.width() < 2.0 || rect.height() < 2.0 || pw < 2 || ph < 2 {
        return;
    }
    g.pixel_scale = pw as f32 / (rect.width() * ppp);
    let stats = g.stats;
    let id = st.game_view.texture_id(engine, pw, ph);
    // The whole image rect drives input mapping; only the visible part is painted.
    st.game_view.rect = Some(rect);
    let painter = ui.painter_at(avail);
    painter.image(
        id,
        rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );
    if oxide_engine::extract::cameras(&engine.world).is_empty() {
        painter.text(
            avail.center(),
            egui::Align2::CENTER_CENTER,
            "Display 1\nNo cameras rendering",
            egui::FontId::proportional(16.0),
            egui::Color32::from_gray(200),
        );
    }
    if resp.clicked() || resp.secondary_clicked() {
        st.game_focused = true;
    }
    if st.play == crate::PlayState::Playing && st.game_focused {
        painter.rect_stroke(
            avail.shrink(0.5),
            0.0,
            egui::Stroke::new(1.0, egui::Color32::from_rgb(58, 121, 187)),
            egui::StrokeKind::Inside,
        );
    }
    if stats {
        let s = &engine.stats;
        let text = format!(
            "Graphics: {:.0} FPS ({:.1} ms)\n\
             Batches: {}   Instances: {}\n\
             Tris: {}\n\
             Shadow casters drawn: {}\n\
             Culled: {} frustum, {} occlusion\n\
             Cameras: {}   Reflection probes: {}\n\
             Screen: {pw}x{ph}",
            engine.fps,
            1000.0 / engine.fps.max(1.0),
            s.draw_calls,
            s.instances,
            s.triangles,
            s.shadow_draw_calls,
            s.culled,
            s.occlusion_culled,
            s.cameras,
            s.reflection_probes,
        );
        let galley =
            painter.layout_no_wrap(text, egui::FontId::monospace(12.0), egui::Color32::WHITE);
        let pos = avail.right_top() + egui::vec2(-8.0, 8.0);
        let bg = egui::Rect::from_min_size(
            pos - egui::vec2(galley.size().x + 12.0, 0.0),
            galley.size() + egui::vec2(12.0, 8.0),
        );
        painter.rect_filled(bg, 3.0, egui::Color32::from_black_alpha(170));
        painter.galley(bg.min + egui::vec2(6.0, 4.0), galley, egui::Color32::WHITE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_resolution_fits_and_keeps_pixels() {
        let avail = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        let (r, px) = layout(avail, GameSize::Fixed(1920, 1080), 1.0, 2.0);
        assert_eq!(px, [1920, 1080]);
        assert!((r.width() - 800.0).abs() < 1e-3 && (r.height() - 450.0).abs() < 1e-3);
        let (r2, _) = layout(avail, GameSize::Fixed(1920, 1080), 2.0, 2.0);
        assert!((r2.width() - 1600.0).abs() < 1e-3);
        assert_eq!(r2.center(), avail.center());
        let (_, free) = layout(avail, GameSize::Free, 1.0, 2.0);
        assert_eq!(free, [1600, 1200]);
    }
}
