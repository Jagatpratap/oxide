//! The Sprite Editor: slices a texture imported as Sprite into sprites. Automatic slicing
//! (islands of opaque pixels), grid by cell size or count, drawing new rects, dragging
//! edges, pivots and 9-slice borders, and per-sprite name/rect/border/pivot. Apply writes
//! the slices into the texture's import settings.
//! Reference: <https://docs.unity3d.com/Manual/sprite/sprite-editor/sprite-editor-landing.html>

use crate::EditorState;
use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};
use oxide_assets::{
    AssetServer, SpriteImportMode, SpriteRect, TextureImporter, TextureImporterType,
};
use oxide_engine::Engine;
use oxide_math::{Vector2, Vector4};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SliceType {
    #[default]
    Automatic,
    GridByCellSize,
    GridByCellCount,
}

#[derive(Clone, Copy, Debug)]
enum Drag {
    /// Move the whole rect.
    Rect,
    /// Edges: which of left, bottom, right, top move.
    Edges([bool; 4]),
    Pivot,
    /// Border line: 0 left, 1 bottom, 2 right, 3 top.
    Border(usize),
    /// Drawing a new rect from this texture pixel.
    New(Vector2),
}

pub struct SpriteEditorWindow {
    path: Option<String>,
    /// The import settings being edited (Apply writes them).
    importer: Option<TextureImporter>,
    /// Texture pixels (RGBA8, top row first), width, height.
    pixels: Option<(Vec<u8>, u32, u32)>,
    image: Option<egui::TextureHandle>,
    selected: Option<usize>,
    zoom: f32,
    pan: egui::Vec2,
    drag: Option<(Drag, usize)>,
    slice_type: SliceType,
    cell: [u32; 2],
    count: [u32; 2],
    offset: [u32; 2],
    padding: [u32; 2],
    keep_empty: bool,
    min_size: u32,
    error: Option<String>,
}

impl Default for SpriteEditorWindow {
    fn default() -> Self {
        Self {
            path: None,
            importer: None,
            pixels: None,
            image: None,
            selected: None,
            zoom: 0.0,
            pan: egui::Vec2::ZERO,
            drag: None,
            slice_type: SliceType::Automatic,
            cell: [32, 32],
            count: [4, 4],
            offset: [0, 0],
            padding: [0, 0],
            keep_empty: false,
            min_size: 4,
            error: None,
        }
    }
}

impl SpriteEditorWindow {
    pub fn open(&mut self, path: &str) {
        if self.path.as_deref() != Some(path) {
            *self = Self {
                path: Some(path.to_owned()),
                ..Default::default()
            };
        }
    }
}

// ---- slicing ----------------------------------------------------------------------------

fn alpha_at(px: &[u8], w: u32, x: u32, y_top: u32) -> u8 {
    px[((y_top * w + x) * 4 + 3) as usize]
}

/// Unity's Automatic slicing: bounding boxes of connected opaque areas (8-neighbour),
/// ignoring ones smaller than `min_size` pixels across, ordered top to bottom then left
/// to right. Rects are in bottom-left-origin pixels.
pub fn slice_automatic(px: &[u8], w: u32, h: u32, min_size: u32) -> Vec<Vector4> {
    let mut seen = vec![false; (w * h) as usize];
    let mut boxes: Vec<(u32, u32, u32, u32)> = Vec::new(); // x0, y0 (top), x1, y1 inclusive
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            if seen[i] || alpha_at(px, w, x, y) == 0 {
                continue;
            }
            let (mut x0, mut y0, mut x1, mut y1) = (x, y, x, y);
            let mut stack = vec![(x, y)];
            seen[i] = true;
            while let Some((cx, cy)) = stack.pop() {
                x0 = x0.min(cx);
                y0 = y0.min(cy);
                x1 = x1.max(cx);
                y1 = y1.max(cy);
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        let (nx, ny) = (cx as i32 + dx, cy as i32 + dy);
                        if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                            continue;
                        }
                        let j = (ny as u32 * w + nx as u32) as usize;
                        if !seen[j] && alpha_at(px, w, nx as u32, ny as u32) > 0 {
                            seen[j] = true;
                            stack.push((nx as u32, ny as u32));
                        }
                    }
                }
            }
            if x1 - x0 + 1 >= min_size || y1 - y0 + 1 >= min_size {
                boxes.push((x0, y0, x1, y1));
            }
        }
    }
    // Rows by top edge (with some slack), then left to right.
    boxes.sort_by_key(|b| (b.1 / 8, b.0));
    boxes
        .into_iter()
        .map(|(x0, y0, x1, y1)| {
            let rw = x1 - x0 + 1;
            let rh = y1 - y0 + 1;
            Vector4::new(x0 as f32, (h - 1 - y1) as f32, rw as f32, rh as f32)
        })
        .collect()
}

/// Grid slicing (by cell size): cells left to right, top to bottom; empty cells are
/// skipped unless `keep_empty`.
pub fn slice_grid(
    px: &[u8],
    w: u32,
    h: u32,
    cell: [u32; 2],
    offset: [u32; 2],
    padding: [u32; 2],
    keep_empty: bool,
) -> Vec<Vector4> {
    let (cw, ch) = (cell[0].max(1), cell[1].max(1));
    let mut out = Vec::new();
    let mut y = offset[1];
    while y + ch <= h {
        let mut x = offset[0];
        while x + cw <= w {
            let empty = (y..y + ch).all(|yy| (x..x + cw).all(|xx| alpha_at(px, w, xx, yy) == 0));
            if keep_empty || !empty {
                out.push(Vector4::new(
                    x as f32,
                    (h - y - ch) as f32,
                    cw as f32,
                    ch as f32,
                ));
            }
            x += cw + padding[0];
        }
        y += ch + padding[1];
    }
    out
}

fn named(stem: &str, rects: Vec<Vector4>, pivot: Vector2) -> Vec<SpriteRect> {
    rects
        .into_iter()
        .enumerate()
        .map(|(i, rect)| SpriteRect {
            name: format!("{stem}_{i}"),
            rect,
            pivot,
            border: Vector4::ZERO,
        })
        .collect()
}

// ---- window -----------------------------------------------------------------------------

/// Unity's pivot presets.
const PIVOTS: [(&str, [f32; 2]); 9] = [
    ("Center", [0.5, 0.5]),
    ("Top Left", [0.0, 1.0]),
    ("Top", [0.5, 1.0]),
    ("Top Right", [1.0, 1.0]),
    ("Left", [0.0, 0.5]),
    ("Right", [1.0, 0.5]),
    ("Bottom Left", [0.0, 0.0]),
    ("Bottom", [0.5, 0.0]),
    ("Bottom Right", [1.0, 0.0]),
];

pub fn window_ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    let w = &mut st.sprite_editor;
    let Some(path) = w.path.clone() else {
        ui.weak("Select a texture imported as Sprite and press Sprite Editor in its Inspector.");
        return;
    };
    let assets = engine.world.resource_mut::<AssetServer>();
    let saved = assets.texture_importer(&path);
    if w.importer.is_none() {
        w.importer = Some(saved.clone());
    }
    if w.pixels.is_none() {
        // The source image at full size (import may shrink the GPU copy).
        match std::fs::read(assets.full_path(&path))
            .map_err(|e| e.to_string())
            .and_then(|b| image::load_from_memory(&b).map_err(|e| e.to_string()))
        {
            Ok(img) => {
                let rgba = img.to_rgba8();
                let (iw, ih) = rgba.dimensions();
                let raw = rgba.into_raw();
                w.image = Some(ui.ctx().load_texture(
                    format!("sprite-editor:{path}"),
                    egui::ColorImage::from_rgba_unmultiplied([iw as usize, ih as usize], &raw),
                    egui::TextureOptions::NEAREST,
                ));
                w.pixels = Some((raw, iw, ih));
            }
            Err(e) => {
                ui.colored_label(Color32::from_rgb(230, 120, 100), format!("{path}: {e}"));
                return;
            }
        }
    }
    let (tw, th) = w.pixels.as_ref().map_or((1, 1), |p| (p.1, p.2));
    let stem = path
        .rsplit('/')
        .next()
        .unwrap_or(&path)
        .rsplit_once('.')
        .map_or(path.as_str(), |(s, _)| s)
        .rsplit('/')
        .next()
        .unwrap_or("Sprite")
        .to_owned();
    let Some(mut imp) = w.importer.clone() else {
        return;
    };
    let dirty = imp != saved;

    // Toolbar.
    ui.horizontal(|ui| {
        ui.label(RichText::new(path.rsplit('/').next().unwrap_or(&path)).strong());
        if imp.texture_type != TextureImporterType::Sprite {
            ui.colored_label(
                Color32::from_rgb(230, 190, 90),
                "Texture Type isn't Sprite: Apply sets it.",
            );
        }
        ui.separator();
        egui::ComboBox::from_id_salt("sprite-mode")
            .selected_text(format!("{:?}", imp.sprite_mode))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut imp.sprite_mode, SpriteImportMode::Single, "Single");
                ui.selectable_value(&mut imp.sprite_mode, SpriteImportMode::Multiple, "Multiple");
            });
        ui.add_enabled_ui(imp.sprite_mode == SpriteImportMode::Multiple, |ui| {
            ui.menu_button("Slice", |ui| {
                egui::ComboBox::from_id_salt("slice-type")
                    .selected_text(match w.slice_type {
                        SliceType::Automatic => "Automatic",
                        SliceType::GridByCellSize => "Grid By Cell Size",
                        SliceType::GridByCellCount => "Grid By Cell Count",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut w.slice_type, SliceType::Automatic, "Automatic");
                        ui.selectable_value(
                            &mut w.slice_type,
                            SliceType::GridByCellSize,
                            "Grid By Cell Size",
                        );
                        ui.selectable_value(
                            &mut w.slice_type,
                            SliceType::GridByCellCount,
                            "Grid By Cell Count",
                        );
                    });
                let pair = |ui: &mut egui::Ui, label: &str, v: &mut [u32; 2]| {
                    ui.horizontal(|ui| {
                        ui.label(label);
                        ui.add(egui::DragValue::new(&mut v[0]).prefix("X "));
                        ui.add(egui::DragValue::new(&mut v[1]).prefix("Y "));
                    });
                };
                match w.slice_type {
                    SliceType::Automatic => {
                        ui.horizontal(|ui| {
                            ui.label("Minimum size");
                            ui.add(egui::DragValue::new(&mut w.min_size).range(1..=4096));
                        });
                    }
                    SliceType::GridByCellSize => {
                        pair(ui, "Pixel Size", &mut w.cell);
                        pair(ui, "Offset", &mut w.offset);
                        pair(ui, "Padding", &mut w.padding);
                        ui.checkbox(&mut w.keep_empty, "Keep Empty Rects");
                    }
                    SliceType::GridByCellCount => {
                        pair(ui, "Column & Row", &mut w.count);
                        ui.checkbox(&mut w.keep_empty, "Keep Empty Rects");
                    }
                }
                if ui.button("Slice").clicked()
                    && let Some((px, pw, ph)) = &w.pixels
                {
                    let rects = match w.slice_type {
                        SliceType::Automatic => slice_automatic(px, *pw, *ph, w.min_size),
                        SliceType::GridByCellSize => {
                            slice_grid(px, *pw, *ph, w.cell, w.offset, w.padding, w.keep_empty)
                        }
                        SliceType::GridByCellCount => {
                            let cell = [*pw / w.count[0].max(1), *ph / w.count[1].max(1)];
                            slice_grid(px, *pw, *ph, cell, [0, 0], [0, 0], w.keep_empty)
                        }
                    };
                    imp.sprites = named(&stem, rects, imp.sprite_pivot);
                    w.selected = None;
                    ui.close();
                }
            });
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled(
                    dirty || imp.texture_type != TextureImporterType::Sprite,
                    egui::Button::new("Apply"),
                )
                .clicked()
            {
                imp.texture_type = TextureImporterType::Sprite;
                match engine
                    .world
                    .resource_mut::<AssetServer>()
                    .set_texture_importer(&path, &imp)
                {
                    Ok(_) => {
                        w.error = None;
                        st.previews.clear();
                    }
                    Err(e) => w.error = Some(e),
                }
            }
            if ui.add_enabled(dirty, egui::Button::new("Revert")).clicked() {
                imp = saved.clone();
                w.selected = None;
            }
        });
    });
    if let Some(e) = &w.error {
        ui.colored_label(Color32::from_rgb(230, 120, 100), e);
    }
    ui.separator();

    let total = ui.available_size();
    let side = 270.0f32.min(total.x * 0.35);
    ui.horizontal_top(|ui| {
        let canvas_size = vec2((total.x - side - 12.0).max(120.0), total.y);
        ui.allocate_ui(canvas_size, |ui| {
            canvas(ui, w, &mut imp, tw, th, &stem, canvas_size)
        });
        ui.separator();
        ui.allocate_ui(vec2(side, total.y), |ui| {
            ui.set_width(side);
            ui.vertical(|ui| sprite_panel(ui, w, &mut imp, tw, th));
        });
    });
    st.sprite_editor.importer = Some(imp);
}

/// The texture with sprite rects; drawing, moving and resizing them.
fn canvas(
    ui: &mut egui::Ui,
    w: &mut SpriteEditorWindow,
    imp: &mut TextureImporter,
    tw: u32,
    th: u32,
    stem: &str,
    size: egui::Vec2,
) {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    // Checkerboard for transparency.
    painter.rect_filled(rect, 0.0, Color32::from_gray(34));
    if w.zoom <= 0.0 {
        w.zoom = ((rect.width() - 40.0) / tw as f32)
            .min((rect.height() - 40.0) / th as f32)
            .max(0.05);
        w.pan = egui::Vec2::ZERO;
    }
    let img_size = vec2(tw as f32, th as f32) * w.zoom;
    let origin = rect.center() - img_size / 2.0 + w.pan;
    let img_rect = Rect::from_min_size(origin, img_size);
    let cell = (8.0 * w.zoom).max(6.0);
    let (cols, rows) = (
        (img_size.x / cell).ceil() as i32,
        (img_size.y / cell).ceil() as i32,
    );
    if cols * rows < 40_000 {
        for j in 0..rows {
            for i in 0..cols {
                if (i + j) % 2 == 0 {
                    let r = Rect::from_min_size(
                        origin + vec2(i as f32 * cell, j as f32 * cell),
                        vec2(cell, cell),
                    )
                    .intersect(img_rect);
                    painter.rect_filled(r, 0.0, Color32::from_gray(60));
                }
            }
        }
    }
    if let Some(img) = &w.image {
        painter.image(
            img.id(),
            img_rect,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    painter.rect_stroke(
        img_rect,
        0.0,
        Stroke::new(1.0, Color32::from_gray(90)),
        StrokeKind::Outside,
    );

    // Texture pixel (bottom-left origin) <-> screen.
    let zoom = w.zoom;
    let to_screen = |x: f32, y: f32| pos2(origin.x + x * zoom, origin.y + (th as f32 - y) * zoom);
    let to_px =
        |p: Pos2| Vector2::new((p.x - origin.x) / zoom, th as f32 - (p.y - origin.y) / zoom);
    let rect_screen =
        |r: Vector4| Rect::from_two_pos(to_screen(r.x, r.y), to_screen(r.x + r.z, r.y + r.w));

    let rects: Vec<Vector4> = match imp.sprite_mode {
        SpriteImportMode::Single => vec![Vector4::new(0.0, 0.0, tw as f32, th as f32)],
        SpriteImportMode::Multiple => imp.sprites.iter().map(|s| s.rect).collect(),
    };
    for (i, r) in rects.iter().enumerate() {
        let sel = w.selected == Some(i) || imp.sprite_mode == SpriteImportMode::Single;
        let sr = rect_screen(*r);
        painter.rect_stroke(
            sr,
            0.0,
            Stroke::new(
                if sel { 2.0 } else { 1.0 },
                if sel {
                    Color32::from_rgb(80, 170, 255)
                } else {
                    Color32::from_gray(220)
                },
            ),
            StrokeKind::Middle,
        );
        if sel {
            // Pivot and 9-slice borders.
            let (pivot, border) = match imp.sprite_mode {
                SpriteImportMode::Single => (imp.sprite_pivot, imp.sprite_border),
                SpriteImportMode::Multiple => (imp.sprites[i].pivot, imp.sprites[i].border),
            };
            let pp = to_screen(r.x + pivot.x * r.z, r.y + pivot.y * r.w);
            painter.circle_stroke(pp, 5.0, Stroke::new(2.0, Color32::from_rgb(80, 170, 255)));
            let green = Stroke::new(1.0, Color32::from_rgb(90, 220, 120));
            let (l, b, rr, t) = (border.x, border.y, border.z, border.w);
            if l > 0.0 {
                let x = to_screen(r.x + l, 0.0).x;
                painter.line_segment([pos2(x, sr.top()), pos2(x, sr.bottom())], green);
            }
            if rr > 0.0 {
                let x = to_screen(r.x + r.z - rr, 0.0).x;
                painter.line_segment([pos2(x, sr.top()), pos2(x, sr.bottom())], green);
            }
            if b > 0.0 {
                let y = to_screen(0.0, r.y + b).y;
                painter.line_segment([pos2(sr.left(), y), pos2(sr.right(), y)], green);
            }
            if t > 0.0 {
                let y = to_screen(0.0, r.y + r.w - t).y;
                painter.line_segment([pos2(sr.left(), y), pos2(sr.right(), y)], green);
            }
            for c in [
                sr.left_top(),
                sr.right_top(),
                sr.left_bottom(),
                sr.right_bottom(),
            ] {
                painter.rect_filled(
                    Rect::from_center_size(c, vec2(7.0, 7.0)),
                    1.0,
                    Color32::from_rgb(80, 170, 255),
                );
            }
        }
    }

    // Zoom (scroll) and pan (middle / Alt drag).
    if resp.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            w.zoom = (w.zoom * (scroll * 0.004).exp()).clamp(0.02, 64.0);
        }
        crate::shortcuts::claim_keys(ui.ctx());
        if ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
            && imp.sprite_mode == SpriteImportMode::Multiple
            && let Some(i) = w.selected.take()
            && i < imp.sprites.len()
        {
            imp.sprites.remove(i);
        }
    }
    let press = ui.input(|i| i.pointer.press_origin());
    if resp.drag_started()
        && let Some(p) = press
    {
        let panning = ui.input(|i| i.pointer.middle_down() || i.modifiers.alt);
        w.drag = if panning {
            None
        } else {
            pick(
                p,
                &rects,
                w.selected
                    .or((imp.sprite_mode == SpriteImportMode::Single).then_some(0)),
                imp,
                &rect_screen,
                &to_screen,
            )
            .or_else(|| {
                (imp.sprite_mode == SpriteImportMode::Multiple)
                    .then(|| (Drag::New(to_px(p)), usize::MAX))
            })
        };
        if let Some((Drag::Rect | Drag::Edges(_), i)) = w.drag {
            w.selected = Some(i);
        }
    }
    if resp.dragged() {
        let d = resp.drag_delta();
        match w.drag {
            None => w.pan += d,
            Some((Drag::New(start), _)) => {
                if let Some(p) = resp.interact_pointer_pos() {
                    let end = to_px(p);
                    let (x0, y0) = (
                        start.x.min(end.x).floor().max(0.0),
                        start.y.min(end.y).floor().max(0.0),
                    );
                    let (x1, y1) = (
                        start.x.max(end.x).ceil().min(tw as f32),
                        start.y.max(end.y).ceil().min(th as f32),
                    );
                    painter.rect_stroke(
                        Rect::from_two_pos(to_screen(x0, y0), to_screen(x1, y1)),
                        0.0,
                        Stroke::new(1.5, Color32::from_rgb(240, 200, 90)),
                        StrokeKind::Middle,
                    );
                }
            }
            Some((kind, i)) => {
                if let Some(p) = resp.interact_pointer_pos() {
                    let px = to_px(p);
                    edit_drag(imp, kind, i, px, tw, th);
                }
            }
        }
    }
    if resp.drag_stopped() {
        if let Some((Drag::New(start), _)) = w.drag
            && let Some(p) = resp.interact_pointer_pos()
        {
            let end = to_px(p);
            let (x0, y0) = (
                start.x.min(end.x).floor().max(0.0),
                start.y.min(end.y).floor().max(0.0),
            );
            let (x1, y1) = (
                start.x.max(end.x).ceil().min(tw as f32),
                start.y.max(end.y).ceil().min(th as f32),
            );
            if x1 - x0 >= 1.0 && y1 - y0 >= 1.0 {
                let n = imp.sprites.len();
                let mut name = format!("{stem}_{n}");
                let mut k = n;
                while imp.sprites.iter().any(|s| s.name == name) {
                    k += 1;
                    name = format!("{stem}_{k}");
                }
                imp.sprites.push(SpriteRect {
                    name,
                    rect: Vector4::new(x0, y0, x1 - x0, y1 - y0),
                    pivot: imp.sprite_pivot,
                    border: Vector4::ZERO,
                });
                w.selected = Some(imp.sprites.len() - 1);
            }
        }
        w.drag = None;
    }
    if resp.clicked()
        && let Some(p) = resp.interact_pointer_pos()
    {
        w.selected = rects.iter().rposition(|r| rect_screen(*r).contains(p));
    }
}

/// What a press at `p` grabs: the selected rect's pivot, borders or edges, or a rect.
fn pick(
    p: Pos2,
    rects: &[Vector4],
    selected: Option<usize>,
    imp: &TextureImporter,
    rect_screen: &dyn Fn(Vector4) -> Rect,
    to_screen: &dyn Fn(f32, f32) -> Pos2,
) -> Option<(Drag, usize)> {
    if let Some(i) = selected
        && let Some(r) = rects.get(i)
    {
        let sr = rect_screen(*r);
        let (pivot, border) = match imp.sprite_mode {
            SpriteImportMode::Single => (imp.sprite_pivot, imp.sprite_border),
            SpriteImportMode::Multiple => (imp.sprites[i].pivot, imp.sprites[i].border),
        };
        if to_screen(r.x + pivot.x * r.z, r.y + pivot.y * r.w).distance(p) < 8.0 {
            return Some((Drag::Pivot, i));
        }
        let near = |a: f32, b: f32| (a - b).abs() < 5.0;
        let lx = to_screen(r.x + border.x, 0.0).x;
        let rx = to_screen(r.x + r.z - border.z, 0.0).x;
        let by = to_screen(0.0, r.y + border.y).y;
        let ty = to_screen(0.0, r.y + r.w - border.w).y;
        let inside = sr.expand(2.0).contains(p);
        if inside && border.x > 0.0 && near(p.x, lx) {
            return Some((Drag::Border(0), i));
        }
        if inside && border.z > 0.0 && near(p.x, rx) {
            return Some((Drag::Border(2), i));
        }
        if inside && border.y > 0.0 && near(p.y, by) {
            return Some((Drag::Border(1), i));
        }
        if inside && border.w > 0.0 && near(p.y, ty) {
            return Some((Drag::Border(3), i));
        }
        let edges = [
            near(p.x, sr.left()),
            near(p.y, sr.bottom()),
            near(p.x, sr.right()),
            near(p.y, sr.top()),
        ];
        if sr.expand(5.0).contains(p)
            && edges.iter().any(|e| *e)
            && imp.sprite_mode == SpriteImportMode::Multiple
        {
            return Some((Drag::Edges(edges), i));
        }
    }
    if imp.sprite_mode == SpriteImportMode::Multiple
        && let Some(i) = rects.iter().rposition(|r| rect_screen(*r).contains(p))
    {
        return Some((Drag::Rect, i));
    }
    None
}

fn edit_drag(imp: &mut TextureImporter, kind: Drag, i: usize, px: Vector2, tw: u32, th: u32) {
    let (tw, th) = (tw as f32, th as f32);
    let single = imp.sprite_mode == SpriteImportMode::Single;
    let r = if single {
        Vector4::new(0.0, 0.0, tw, th)
    } else {
        imp.sprites[i].rect
    };
    match kind {
        Drag::Pivot => {
            let pivot = Vector2::new(
                ((px.x - r.x) / r.z).clamp(0.0, 1.0),
                ((px.y - r.y) / r.w).clamp(0.0, 1.0),
            );
            if single {
                imp.sprite_pivot = pivot;
            } else {
                imp.sprites[i].pivot = pivot;
            }
        }
        Drag::Border(side) => {
            let b = if single {
                &mut imp.sprite_border
            } else {
                &mut imp.sprites[i].border
            };
            let v = match side {
                0 => (px.x - r.x).round().clamp(0.0, r.z - b.z),
                1 => (px.y - r.y).round().clamp(0.0, r.w - b.w),
                2 => (r.x + r.z - px.x).round().clamp(0.0, r.z - b.x),
                _ => (r.y + r.w - px.y).round().clamp(0.0, r.w - b.y),
            };
            match side {
                0 => b.x = v,
                1 => b.y = v,
                2 => b.z = v,
                _ => b.w = v,
            }
        }
        Drag::Edges(e) => {
            let s = &mut imp.sprites[i];
            let (mut x0, mut y0, mut x1, mut y1) = (r.x, r.y, r.x + r.z, r.y + r.w);
            let (x, y) = (px.x.round().clamp(0.0, tw), px.y.round().clamp(0.0, th));
            if e[0] {
                x0 = x.min(x1 - 1.0);
            }
            if e[1] {
                y0 = y.min(y1 - 1.0);
            }
            if e[2] {
                x1 = x.max(x0 + 1.0);
            }
            if e[3] {
                y1 = y.max(y0 + 1.0);
            }
            s.rect = Vector4::new(x0, y0, x1 - x0, y1 - y0);
        }
        Drag::Rect => {
            let s = &mut imp.sprites[i];
            let x = (px.x - r.z / 2.0).round().clamp(0.0, tw - r.z);
            let y = (px.y - r.w / 2.0).round().clamp(0.0, th - r.w);
            s.rect = Vector4::new(x, y, r.z, r.w);
        }
        Drag::New(_) => {}
    }
}

fn sprite_panel(
    ui: &mut egui::Ui,
    w: &mut SpriteEditorWindow,
    imp: &mut TextureImporter,
    tw: u32,
    th: u32,
) {
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "Pixels Per Unit");
        ui.add(egui::DragValue::new(&mut imp.pixels_per_unit).range(0.01..=10_000.0));
    });
    ui.separator();
    let single = imp.sprite_mode == SpriteImportMode::Single;
    let idx = if single {
        Some(0)
    } else {
        w.selected.filter(|&i| i < imp.sprites.len())
    };
    let Some(i) = idx else {
        ui.weak(format!(
            "{} sprite(s). Select one, or drag on empty space to draw a new one.",
            imp.sprites.len()
        ));
        return;
    };
    ui.label(RichText::new("Sprite").strong());
    if !single {
        ui.horizontal(|ui| {
            crate::inspector::prop_label(ui, "Name");
            ui.text_edit_singleline(&mut imp.sprites[i].name);
        });
        let r = &mut imp.sprites[i].rect;
        let mut v = [r.x, r.y, r.z, r.w];
        ui.horizontal(|ui| {
            crate::inspector::prop_label(ui, "Position");
            ui.add(
                egui::DragValue::new(&mut v[0])
                    .prefix("X ")
                    .range(0.0..=tw as f32),
            );
            ui.add(
                egui::DragValue::new(&mut v[1])
                    .prefix("Y ")
                    .range(0.0..=th as f32),
            );
        });
        ui.horizontal(|ui| {
            crate::inspector::prop_label(ui, "");
            ui.add(
                egui::DragValue::new(&mut v[2])
                    .prefix("W ")
                    .range(1.0..=tw as f32),
            );
            ui.add(
                egui::DragValue::new(&mut v[3])
                    .prefix("H ")
                    .range(1.0..=th as f32),
            );
        });
        *r = Vector4::new(v[0].round(), v[1].round(), v[2].round(), v[3].round());
    }
    let (pivot, border) = if single {
        (&mut imp.sprite_pivot, &mut imp.sprite_border)
    } else {
        let s = &mut imp.sprites[i];
        (&mut s.pivot, &mut s.border)
    };
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "Border");
        ui.add(
            egui::DragValue::new(&mut border.x)
                .prefix("L ")
                .range(0.0..=4096.0),
        );
        ui.add(
            egui::DragValue::new(&mut border.w)
                .prefix("T ")
                .range(0.0..=4096.0),
        );
    });
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "");
        ui.add(
            egui::DragValue::new(&mut border.z)
                .prefix("R ")
                .range(0.0..=4096.0),
        );
        ui.add(
            egui::DragValue::new(&mut border.y)
                .prefix("B ")
                .range(0.0..=4096.0),
        );
    });
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "Pivot");
        let current = PIVOTS
            .iter()
            .find(|(_, p)| (p[0] - pivot.x).abs() < 1e-4 && (p[1] - pivot.y).abs() < 1e-4)
            .map_or("Custom", |(n, _)| n);
        egui::ComboBox::from_id_salt("pivot")
            .selected_text(current)
            .show_ui(ui, |ui| {
                for (name, p) in PIVOTS {
                    if ui.selectable_label(current == name, name).clicked() {
                        *pivot = Vector2::new(p[0], p[1]);
                    }
                }
            });
    });
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "Custom Pivot");
        ui.add(
            egui::DragValue::new(&mut pivot.x)
                .speed(0.01)
                .range(0.0..=1.0),
        );
        ui.add(
            egui::DragValue::new(&mut pivot.y)
                .speed(0.01)
                .range(0.0..=1.0),
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 12×4 image (top row first) with a 2×2 blob at the top left and a 3×2 one at the
    /// bottom right.
    fn image() -> (Vec<u8>, u32, u32) {
        let (w, h) = (12u32, 4u32);
        let mut px = vec![0u8; (w * h * 4) as usize];
        let mut set = |x: u32, y: u32| px[((y * w + x) * 4 + 3) as usize] = 255;
        for (x, y) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            set(x, y);
        }
        for (x, y) in [(8, 2), (9, 2), (10, 2), (8, 3), (9, 3), (10, 3)] {
            set(x, y);
        }
        (px, w, h)
    }

    #[test]
    fn automatic_finds_islands_in_bottom_left_pixels() {
        let (px, w, h) = image();
        let r = slice_automatic(&px, w, h, 1);
        assert_eq!(
            r,
            vec![
                Vector4::new(0.0, 2.0, 2.0, 2.0),
                Vector4::new(8.0, 0.0, 3.0, 2.0)
            ]
        );
        assert!(
            slice_automatic(&px, w, h, 3).len() == 1,
            "the 2×2 island is under the minimum size"
        );
    }

    #[test]
    fn grid_skips_empty_cells_unless_kept() {
        let (px, w, h) = image();
        let r = slice_grid(&px, w, h, [4, 4], [0, 0], [0, 0], false);
        assert_eq!(
            r,
            vec![
                Vector4::new(0.0, 0.0, 4.0, 4.0),
                Vector4::new(8.0, 0.0, 4.0, 4.0)
            ]
        );
        assert_eq!(slice_grid(&px, w, h, [4, 4], [0, 0], [0, 0], true).len(), 3);
    }

    #[test]
    fn dragging_edges_and_pivot() {
        let mut imp = TextureImporter {
            sprite_mode: SpriteImportMode::Multiple,
            sprites: vec![SpriteRect {
                rect: Vector4::new(2.0, 2.0, 4.0, 4.0),
                ..Default::default()
            }],
            ..Default::default()
        };
        edit_drag(
            &mut imp,
            Drag::Edges([false, false, true, true]),
            0,
            Vector2::new(9.2, 7.8),
            16,
            16,
        );
        assert_eq!(imp.sprites[0].rect, Vector4::new(2.0, 2.0, 7.0, 6.0));
        edit_drag(&mut imp, Drag::Pivot, 0, Vector2::new(2.0, 2.0), 16, 16);
        assert_eq!(imp.sprites[0].pivot, Vector2::new(0.0, 0.0));
        edit_drag(&mut imp, Drag::Border(0), 0, Vector2::new(4.0, 5.0), 16, 16);
        assert_eq!(imp.sprites[0].border.x, 2.0);
    }
}
