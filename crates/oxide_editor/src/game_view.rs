//! Game view: the game as players see it, with Unity's control bar: Display 1-8, aspect ratios
//! and fixed resolutions (plus your own), Low Resolution Aspect Ratios, Scale, Play Focused /
//! Maximized / Unfocused, Mute Audio, VSync, Stats and Gizmos.
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

/// A size added with the size menu's "+" (saved in Preferences).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CustomGameSize {
    pub label: String,
    /// A fixed resolution in pixels; otherwise an aspect ratio `width:height`.
    pub fixed: bool,
    pub width: u32,
    pub height: u32,
}

impl CustomGameSize {
    pub fn size(&self) -> GameSize {
        let (w, h) = (self.width.max(1), self.height.max(1));
        if self.fixed {
            GameSize::Fixed(w, h)
        } else {
            GameSize::Aspect(w as f32 / h as f32)
        }
    }

    pub fn name(&self) -> String {
        let dims = if self.fixed {
            format!("{}x{}", self.width, self.height)
        } else {
            format!("{}:{}", self.width, self.height)
        };
        if self.label.trim().is_empty() {
            dims
        } else {
            format!("{} ({dims})", self.label.trim())
        }
    }
}

/// Unity's "Enter Play Mode" behaviour of the Game view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EnterPlayMode {
    /// Switch to the Game view and give it keyboard focus.
    #[default]
    Focused,
    /// Fill the editor with the Game view.
    Maximized,
    /// Keep the current tab and focus (the game still runs).
    Unfocused,
}

impl EnterPlayMode {
    pub const ALL: [Self; 3] = [Self::Focused, Self::Maximized, Self::Unfocused];

    pub fn name(self) -> &'static str {
        match self {
            Self::Focused => "Play Focused",
            Self::Maximized => "Play Maximized",
            Self::Unfocused => "Play Unfocused",
        }
    }
}

pub struct GameViewState {
    pub size: GameSize,
    /// Zoom: 1 = the largest size that fits; above 1 magnifies the center.
    pub scale: f32,
    pub play_mode: EnterPlayMode,
    pub mute_audio: bool,
    pub stats: bool,
    /// Draw gizmos (Debug lines, component gizmos) over the game.
    pub gizmos: bool,
    /// Display 1-8 (0-based): which cameras render here (`Camera.targetDisplay`).
    pub display: u32,
    /// Render free and aspect sizes at one pixel per point on high-DPI screens.
    pub low_resolution: bool,
    /// VSync while the Game view is shown (Unity's "VSync (Game view only)").
    pub vsync: bool,
    /// The "+" form in the size menu.
    adding: Option<CustomGameSize>,
    /// Game pixels per window pixel for the last frame (mouse mapping).
    pub pixel_scale: f32,
}

impl Default for GameViewState {
    fn default() -> Self {
        Self {
            size: GameSize::Free,
            scale: 1.0,
            play_mode: EnterPlayMode::Focused,
            mute_audio: false,
            stats: false,
            gizmos: false,
            display: 0,
            low_resolution: false,
            vsync: false,
            adding: None,
            pixel_scale: 1.0,
        }
    }
}

impl GameViewState {
    fn size_label(&self, custom: &[CustomGameSize]) -> String {
        SIZES
            .iter()
            .find(|(_, s)| *s == self.size)
            .map(|(n, _)| (*n).to_owned())
            .or_else(|| {
                custom
                    .iter()
                    .find(|c| c.size() == self.size)
                    .map(CustomGameSize::name)
            })
            .unwrap_or_else(|| format!("{:?}", self.size))
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
    let custom = st.prefs.game_sizes.clone();
    let mut prefs_changed = false;
    let g = &mut st.game;
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("game-display")
            .selected_text(format!("Display {}", g.display + 1))
            .width(80.0)
            .show_ui(ui, |ui| {
                for d in 0..8 {
                    ui.selectable_value(&mut g.display, d, format!("Display {}", d + 1));
                }
            });
        egui::ComboBox::from_id_salt("game-size")
            .selected_text(g.size_label(&custom))
            .width(170.0)
            .height(520.0)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show_ui(ui, |ui| {
                let before = g.size;
                ui.checkbox(&mut g.low_resolution, "Low Resolution Aspect Ratios");
                ui.checkbox(&mut g.vsync, "VSync (Game view only)");
                ui.separator();
                for (name, s) in SIZES {
                    ui.selectable_value(&mut g.size, *s, *name);
                }
                if !custom.is_empty() {
                    ui.separator();
                }
                for (i, c) in custom.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut g.size, c.size(), c.name());
                        if ui
                            .small_button("🗑")
                            .on_hover_text("Remove this size")
                            .clicked()
                        {
                            st_remove_size(i);
                        }
                    });
                }
                ui.separator();
                if ui.button("+ Add size…").clicked() {
                    g.adding = Some(CustomGameSize {
                        label: String::new(),
                        fixed: true,
                        width: 1280,
                        height: 720,
                    });
                    ui.close();
                }
                if g.size != before {
                    ui.close();
                }
            });
        // Unity's "Add" dialog for a new Game view size.
        if let Some(c) = &mut g.adding {
            let mut close = false;
            egui::Modal::new(egui::Id::new("game-add-size")).show(ui.ctx(), |ui| {
                ui.heading("Add Game View Size");
                egui::Grid::new("add-size").num_columns(2).show(ui, |ui| {
                    ui.label("Label");
                    ui.add(egui::TextEdit::singleline(&mut c.label).desired_width(160.0));
                    ui.end_row();
                    ui.label("Type");
                    egui::ComboBox::from_id_salt("add-size-type")
                        .selected_text(if c.fixed {
                            "Fixed Resolution"
                        } else {
                            "Aspect Ratio"
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut c.fixed, true, "Fixed Resolution");
                            ui.selectable_value(&mut c.fixed, false, "Aspect Ratio");
                        });
                    ui.end_row();
                    ui.label("Width & Height");
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut c.width).range(1..=16384));
                        ui.add(egui::DragValue::new(&mut c.height).range(1..=16384));
                    });
                    ui.end_row();
                });
                ui.horizontal(|ui| {
                    if ui.button("OK").clicked() {
                        ADDED.with(|a| *a.borrow_mut() = Some(c.clone()));
                        close = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
            });
            if close
                && let Some(c) = g.adding.take()
                    && ADDED.with(|a| a.borrow().is_some()) {
                        g.size = c.size();
                    }
        }
        ui.label("Scale");
        ui.add(
            egui::Slider::new(&mut g.scale, 1.0..=5.0)
                .max_decimals(1)
                .suffix("x"),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.toggle_value(&mut g.gizmos, "Gizmos");
            ui.toggle_value(&mut g.stats, "Stats");
            ui.toggle_value(&mut g.mute_audio, "Mute Audio");
            egui::ComboBox::from_id_salt("game-play-mode")
                .selected_text(g.play_mode.name())
                .width(120.0)
                .show_ui(ui, |ui| {
                    for m in EnterPlayMode::ALL {
                        ui.selectable_value(&mut g.play_mode, m, m.name());
                    }
                })
                .response
                .on_hover_text("What entering Play mode does to the Game view");
        });
    });
    if let Some(c) = ADDED.with(|a| a.borrow_mut().take()) {
        st.prefs.game_sizes.push(c);
        prefs_changed = true;
    }
    if let Some(i) = REMOVED.with(|r| r.borrow_mut().take())
        && i < st.prefs.game_sizes.len()
    {
        st.prefs.game_sizes.remove(i);
        prefs_changed = true;
    }
    st.prefs_changed |= prefs_changed;
    let g = &mut st.game;
    engine.vsync_override = g.vsync.then_some(true);
    engine
        .world
        .resources
        .insert(oxide_engine::extract::ActiveDisplay(g.display));
    if let Some(a) = engine
        .world
        .try_resource_mut::<oxide_engine::prelude::Audio>()
    {
        a.mute = g.mute_audio;
    }
    let avail = ui.available_rect_before_wrap();
    let resp = ui.allocate_rect(avail, egui::Sense::click());
    ui.painter().rect_filled(avail, 0.0, egui::Color32::BLACK);
    // Low Resolution Aspect Ratios: one game pixel per point on high-DPI screens.
    let ppp = if g.low_resolution {
        1.0
    } else {
        ui.ctx().pixels_per_point()
    };
    let (rect, [pw, ph]) = layout(avail, g.size, g.scale, ppp);
    if rect.width() < 2.0 || rect.height() < 2.0 || pw < 2 || ph < 2 {
        return;
    }
    g.pixel_scale = pw as f32 / (rect.width() * ui.ctx().pixels_per_point());
    let stats = g.stats;
    let display = g.display;
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
            format!("Display {}\nNo cameras rendering", display + 1),
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
             Visible skinned meshes: {}   Animators: {}\n\
             Audio: {} playing{}\n\
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
            engine
                .world
                .query::<oxide_scene::SkinnedMeshRenderer>()
                .count(),
            engine
                .world
                .query::<oxide_engine::prelude::Animator>()
                .count(),
            engine
                .world
                .try_resource::<oxide_engine::prelude::Audio>()
                .map_or(0, |a| a.active_voices()),
            if st.game.mute_audio { " (muted)" } else { "" },
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

thread_local! {
    /// A size added or removed in the size menu this frame (applied after the menu closes,
    /// since Preferences are borrowed elsewhere while it draws).
    static ADDED: std::cell::RefCell<Option<CustomGameSize>> = const { std::cell::RefCell::new(None) };
    static REMOVED: std::cell::RefCell<Option<usize>> = const { std::cell::RefCell::new(None) };
}

fn st_remove_size(i: usize) {
    REMOVED.with(|r| *r.borrow_mut() = Some(i));
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
