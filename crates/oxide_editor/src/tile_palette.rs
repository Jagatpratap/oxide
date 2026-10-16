//! The Tile Palette window and Scene view tile painting: pick an active Tilemap and a
//! palette, choose a tile, and paint with Paint (B), Box (U), Pick (I), Erase (D) or Fill
//! (G) by clicking and dragging in the Scene view. Dropping tiles or sprite textures from
//! the Project window onto the palette adds them (sprites become Tile assets, as Unity
//! does). Painting is undoable.
//! Reference: <https://docs.unity3d.com/Manual/tilemaps/work-with-tilemaps/tile-palette-landing.html>

use crate::{EditorState, project::AssetPayload, view::Projector};
use egui::{Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use oxide_assets::{AssetServer, Handle, Sprite};
use oxide_engine::Engine;
use oxide_math::{Matrix4x4, Vector3};
use oxide_scene::{Grid, Tile, TileBase, TileCell, TilePalette, Tilemap, World};
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PaintTool {
    Paint,
    Box,
    Pick,
    Erase,
    Fill,
}

#[derive(Default)]
pub struct TilePaletteWindow {
    palette: Option<String>,
    pub tilemap: Option<oxide_scene::GameObjectId>,
    pub tool: Option<PaintTool>,
    /// The selected tile (the brush).
    brush: Option<String>,
    box_start: Option<(i32, i32)>,
    stroke: bool,
    /// Sprite textures for previews: texture path → (version, image).
    textures: HashMap<String, (u64, egui::TextureHandle)>,
    error: Option<String>,
}

const CELL: f32 = 52.0;

/// The sprite a tile shows in previews.
fn tile_preview(assets: &AssetServer, tile: &str) -> Option<Handle<Sprite>> {
    let map = HashMap::new();
    oxide_scene::tilemap::tile_sprite(assets, &map, tile, 0, 0, 0.0).map(|(s, _)| s)
}

/// Draws a sprite into `rect` (keeping its aspect), loading its texture as an egui image.
fn draw_sprite(
    ui: &egui::Ui,
    textures: &mut HashMap<String, (u64, egui::TextureHandle)>,
    assets: &AssetServer,
    sprite: &Handle<Sprite>,
    rect: Rect,
) {
    let Some(s) = assets.sprites.get(sprite) else {
        return;
    };
    let tp = s.texture.path().to_owned();
    let version = assets.textures.version(s.texture.id()).unwrap_or(0);
    if textures.get(&tp).is_none_or(|(v, _)| *v != version)
        && let Some(t) = assets.textures.get(&s.texture)
        && let Some(px) = t.mips.first()
    {
        let img =
            egui::ColorImage::from_rgba_unmultiplied([t.width as usize, t.height as usize], px);
        textures.insert(
            tp.clone(),
            (
                version,
                ui.ctx()
                    .load_texture(format!("tile:{tp}"), img, egui::TextureOptions::NEAREST),
            ),
        );
    }
    let Some((_, img)) = textures.get(&tp) else {
        return;
    };
    let uv = s.uv_rect();
    let aspect = s.rect.z / s.rect.w.max(1.0);
    let size = if aspect >= 1.0 {
        vec2(rect.width(), rect.width() / aspect)
    } else {
        vec2(rect.height() * aspect, rect.height())
    };
    let r = Rect::from_center_size(rect.center(), size);
    ui.painter().image(
        img.id(),
        r,
        Rect::from_min_size(pos2(uv[0], uv[1]), vec2(uv[2], uv[3])),
        Color32::WHITE,
    );
}

/// Tile assets made from a texture imported as Sprite (one per sprite), under
/// `Assets/Tiles`. Returns their paths.
fn tiles_from_texture(assets: &mut AssetServer, texture: &str) -> Result<Vec<String>, String> {
    assets.ensure_loaded(texture)?;
    let mut sprites: Vec<String> = assets
        .sprites
        .iter()
        .filter(|(p, _)| p.starts_with(&format!("{texture}#")))
        .map(|(p, _)| p.to_owned())
        .collect();
    sprites.sort();
    if sprites.is_empty() {
        return Err(format!(
            "{texture} has no sprites: set its Texture Type to Sprite"
        ));
    }
    let root = assets.full_path("");
    let mut out = Vec::new();
    for s in sprites {
        let name = s.rsplit('#').next().unwrap_or("Tile");
        let path = crate::asset_ops::unique_path(&root, &format!("Assets/Tiles/{name}.tile"));
        assets.save_data(
            &path,
            Box::new(Tile {
                sprite: Some(Handle::from_path(s.clone())),
                ..Default::default()
            }),
        )?;
        out.push(path);
    }
    Ok(out)
}

fn is_tile(assets: &AssetServer, path: &str) -> bool {
    assets
        .types
        .for_path(path)
        .is_some_and(|t| matches!(t.type_name, "Tile" | "AnimatedTile" | "RuleTile"))
}

pub fn window_ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    let tilemaps: Vec<(oxide_scene::GameObjectId, String)> = engine
        .world
        .query::<Tilemap>()
        .map(|(id, _)| {
            (
                id,
                engine
                    .world
                    .get(id)
                    .map_or(String::new(), |o| o.name.clone()),
            )
        })
        .collect();
    let w = &mut st.tile_palette;
    if w.tilemap.is_none_or(|t| !engine.world.is_alive(t)) {
        w.tilemap = tilemaps.first().map(|t| t.0);
    }
    let palettes = engine
        .world
        .resource::<AssetServer>()
        .paths_of_type("TilePalette");
    if w.palette.as_ref().is_none_or(|p| !palettes.contains(p)) {
        w.palette = palettes.first().cloned();
    }

    ui.horizontal(|ui| {
        ui.label("Active Tilemap");
        egui::ComboBox::from_id_salt("tp-tilemap")
            .selected_text(
                w.tilemap
                    .and_then(|t| tilemaps.iter().find(|x| x.0 == t))
                    .map_or("None".into(), |x| x.1.clone()),
            )
            .show_ui(ui, |ui| {
                for (id, name) in &tilemaps {
                    if ui.selectable_label(w.tilemap == Some(*id), name).clicked() {
                        w.tilemap = Some(*id);
                    }
                }
            });
        if tilemaps.is_empty() {
            ui.weak("(GameObject > 2D Object > Tilemap > Rectangular)");
        }
    });
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("tp-palette")
            .selected_text(
                w.palette
                    .as_deref()
                    .map_or("No palette", |p| p.rsplit('/').next().unwrap_or(p)),
            )
            .show_ui(ui, |ui| {
                for p in &palettes {
                    if ui
                        .selectable_label(
                            w.palette.as_ref() == Some(p),
                            p.rsplit('/').next().unwrap_or(p),
                        )
                        .clicked()
                    {
                        w.palette = Some(p.clone());
                    }
                }
            });
        if ui.button("Create New Palette").clicked() {
            let assets = engine.world.resource_mut::<AssetServer>();
            let root = assets.full_path("");
            let path = crate::asset_ops::unique_path(&root, "Assets/Palettes/New Palette.palette");
            match assets.save_data(&path, Box::new(TilePalette::default())) {
                Ok(()) => {
                    w.palette = Some(path);
                    st.project.invalidate();
                }
                Err(e) => w.error = Some(e),
            }
        }
    });
    let w = &mut st.tile_palette;
    ui.horizontal(|ui| {
        let tools = [
            (None, "Select", "Normal Scene view selection"),
            (
                Some(PaintTool::Paint),
                "Paint",
                "Paint with the selected tile (B)",
            ),
            (Some(PaintTool::Box), "Box", "Fill a rectangle (U)"),
            (
                Some(PaintTool::Pick),
                "Pick",
                "Pick a tile from the Tilemap (I)",
            ),
            (Some(PaintTool::Erase), "Erase", "Erase tiles (D)"),
            (Some(PaintTool::Fill), "Fill", "Flood fill (G)"),
        ];
        for (t, name, tip) in tools {
            if ui
                .selectable_label(w.tool == t, name)
                .on_hover_text(tip)
                .clicked()
            {
                w.tool = t;
            }
        }
    });
    if ui.rect_contains_pointer(ui.max_rect()) {
        // Unity's palette shortcuts while the window is under the pointer.
        let key = ui.input(|i| {
            [
                (egui::Key::B, PaintTool::Paint),
                (egui::Key::U, PaintTool::Box),
                (egui::Key::I, PaintTool::Pick),
                (egui::Key::D, PaintTool::Erase),
                (egui::Key::G, PaintTool::Fill),
            ]
            .into_iter()
            .find(|(k, _)| i.key_pressed(*k))
            .map(|(_, t)| t)
        });
        if let Some(t) = key {
            w.tool = Some(t);
        }
        crate::shortcuts::claim_keys(ui.ctx());
    }
    if let Some(e) = &w.error {
        ui.colored_label(Color32::from_rgb(230, 120, 100), e);
    }
    ui.separator();

    let Some(path) = w.palette.clone() else {
        ui.weak(
            "Create a palette, then drag tiles or sprite textures from the Project window onto it.",
        );
        return;
    };
    let assets = engine.world.resource_mut::<AssetServer>();
    let h: Handle<TilePalette> = Handle::from_path(path.clone());
    let Some(mut pal) = assets.data_asset(&h).cloned() else {
        return;
    };
    for c in &pal.tiles {
        if let Some(t) = &c.tile {
            let _ = assets.ensure_loaded(t.path());
        }
    }
    let before = pal.clone();
    let cols = ((ui.available_width() / CELL).floor() as i32).max(4);
    let rows = ((pal.tiles.iter().map(|c| c.y).max().unwrap_or(0) + 2)
        .max((ui.available_height() / CELL) as i32))
    .max(3);
    let (rect, resp) =
        ui.allocate_exact_size(vec2(cols as f32 * CELL, rows as f32 * CELL), Sense::click());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::from_gray(36));
    let cell_rect = |x: i32, y: i32| {
        Rect::from_min_size(
            rect.min + vec2(x as f32 * CELL, y as f32 * CELL),
            vec2(CELL, CELL),
        )
    };
    for y in 0..rows {
        for x in 0..cols {
            painter.rect_stroke(
                cell_rect(x, y),
                0.0,
                Stroke::new(1.0, Color32::from_gray(48)),
                StrokeKind::Inside,
            );
        }
    }
    let assets = engine.world.resource::<AssetServer>();
    for c in &pal.tiles {
        let Some(t) = &c.tile else { continue };
        let r = cell_rect(c.x, c.y);
        if let Some(sp) = tile_preview(assets, t.path()) {
            draw_sprite(ui, &mut w.textures, assets, &sp, r.shrink(4.0));
        }
        if w.brush.as_deref() == Some(t.path()) {
            painter.rect_stroke(
                r,
                2.0,
                Stroke::new(2.0, Color32::from_rgb(80, 170, 255)),
                StrokeKind::Inside,
            );
        }
    }
    let at = |p: egui::Pos2| {
        (
            ((p.x - rect.left()) / CELL).floor() as i32,
            ((p.y - rect.top()) / CELL).floor() as i32,
        )
    };
    if resp.clicked()
        && let Some(p) = resp.interact_pointer_pos()
    {
        let (x, y) = at(p);
        let tile = pal
            .tiles
            .iter()
            .find(|c| c.x == x && c.y == y)
            .and_then(|c| c.tile.as_ref());
        w.brush = tile.map(|t| t.path().to_owned());
        if w.brush.is_some() && w.tool.is_none() {
            w.tool = Some(PaintTool::Paint);
        }
    }
    resp.context_menu(|ui| {
        if ui.button("Remove selected tile from palette").clicked() {
            if let Some(b) = &w.brush {
                pal.tiles
                    .retain(|c| c.tile.as_ref().is_none_or(|t| t.path() != b));
            }
            ui.close();
        }
    });
    if let Some(payload) = resp.dnd_release_payload::<AssetPayload>()
        && let Some(p) = resp.hover_pos()
    {
        let assets = engine.world.resource_mut::<AssetServer>();
        let dropped: Result<Vec<String>, String> = if is_tile(assets, &payload.0) {
            Ok(vec![payload.0.clone()])
        } else {
            tiles_from_texture(assets, &payload.0)
        };
        match dropped {
            Ok(tiles) => {
                let (mut x, mut y) = at(p);
                for t in tiles {
                    while pal.tiles.iter().any(|c| c.x == x && c.y == y) {
                        x += 1;
                        if x >= cols {
                            x = 0;
                            y += 1;
                        }
                    }
                    let _ = assets.ensure_loaded(&t);
                    pal.tiles.push(TileCell {
                        x,
                        y,
                        tile: Some(Handle::from_path(t)),
                    });
                }
                st.project.invalidate();
            }
            Err(e) => st.tile_palette.error = Some(e),
        }
    }
    if pal != before
        && let Err(e) = engine
            .world
            .resource_mut::<AssetServer>()
            .save_data(&path, Box::new(pal))
    {
        st.tile_palette.error = Some(e);
    }
    ui.add_space(4.0);
    let w = &st.tile_palette;
    ui.weak(match (&w.brush, w.tool) {
        (None, _) => "Click a tile to paint with it.".to_owned(),
        (Some(b), Some(t)) => format!(
            "{t:?}: {} — click or drag in the Scene view",
            b.rsplit('/').next().unwrap_or(b)
        ),
        (Some(b), None) => format!("Selected {}", b.rsplit('/').next().unwrap_or(b)),
    });
}

/// The cell under a Scene view ray, in a tilemap's grid.
fn cell_at(
    world: &World,
    tilemap: oxide_scene::GameObjectId,
    ray: &oxide_math::Ray,
) -> Option<(i32, i32)> {
    let m = world.local_to_world(tilemap);
    let inv = m.inverse();
    let o = inv.transform_point3(ray.origin);
    let d = inv.transform_vector3(ray.direction);
    if d.z.abs() < 1e-6 {
        return None;
    }
    let t = -o.z / d.z;
    if t < 0.0 {
        return None;
    }
    let p = o + d * t;
    let (cs, gap) = grid_of(world, tilemap);
    Some((
        (p.x / (cs.x + gap.x)).floor() as i32,
        (p.y / (cs.y + gap.y)).floor() as i32,
    ))
}

fn grid_of(world: &World, tilemap: oxide_scene::GameObjectId) -> (Vector3, Vector3) {
    let g = world
        .ancestor_with::<Grid>(tilemap)
        .and_then(|g| world.get_component::<Grid>(g))
        .cloned()
        .unwrap_or_default();
    let cs = Vector3::new(g.cell_size.x.max(0.001), g.cell_size.y.max(0.001), 0.0);
    (cs, g.cell_gap)
}

/// Outline of cells `a..=b` in the Scene view.
fn draw_cells(
    world: &World,
    tilemap: oxide_scene::GameObjectId,
    proj: &Projector,
    painter: &egui::Painter,
    a: (i32, i32),
    b: (i32, i32),
    color: Color32,
) {
    let m: Matrix4x4 = world.local_to_world(tilemap);
    let (cs, gap) = grid_of(world, tilemap);
    let (x0, x1) = (a.0.min(b.0), a.0.max(b.0) + 1);
    let (y0, y1) = (a.1.min(b.1), a.1.max(b.1) + 1);
    let corner = |x: i32, y: i32| {
        m.transform_point3(Vector3::new(
            x as f32 * (cs.x + gap.x),
            y as f32 * (cs.y + gap.y),
            0.0,
        ))
    };
    let pts = [
        corner(x0, y0),
        corner(x1, y0),
        corner(x1, y1),
        corner(x0, y1),
    ];
    for i in 0..4 {
        if let Some(s) = proj.segment(pts[i], pts[(i + 1) % 4]) {
            painter.line_segment(s, Stroke::new(2.0, color));
        }
    }
}

/// Scene view painting. Returns true while a palette tool owns the mouse (no selection
/// or handles then).
pub fn scene_paint(
    st: &mut EditorState,
    engine: &mut Engine,
    ui: &egui::Ui,
    resp: &egui::Response,
    proj: &Projector,
    painter: &egui::Painter,
    palette_open: bool,
) -> bool {
    let w = &mut st.tile_palette;
    let (Some(tool), Some(tm), true) = (w.tool, w.tilemap, palette_open) else {
        return false;
    };
    if !engine.world.is_alive(tm) || !engine.world.has_component::<Tilemap>(tm) {
        return false;
    }
    let pointer = resp.hover_pos().or(resp.interact_pointer_pos());
    let cell = pointer.and_then(|p| cell_at(&engine.world, tm, &proj.ray(p)));
    let color = match tool {
        PaintTool::Erase => Color32::from_rgb(240, 90, 90),
        PaintTool::Pick => Color32::from_rgb(240, 200, 90),
        _ => Color32::from_rgb(90, 200, 255),
    };
    if let Some(c) = cell {
        let a = if tool == PaintTool::Box {
            w.box_start.unwrap_or(c)
        } else {
            c
        };
        draw_cells(&engine.world, tm, proj, painter, a, c, color);
    }
    let now = oxide_core::time::realtime_since_startup();
    let down = ui.input(|i| i.pointer.primary_down());
    let pressed = resp.drag_started() || resp.clicked();
    let brush: Option<Handle<TileBase>> = w.brush.as_ref().map(|b| Handle::from_path(b.clone()));
    if let Some(c) = cell {
        match tool {
            PaintTool::Paint | PaintTool::Erase
                if (pressed || resp.dragged()) && down || resp.clicked() =>
            {
                let tile = if tool == PaintTool::Erase {
                    None
                } else {
                    brush.clone()
                };
                if tool == PaintTool::Paint && tile.is_none() {
                    return true;
                }
                let same = engine
                    .world
                    .get_component::<Tilemap>(tm)
                    .and_then(|m| m.get_tile(c.0, c.1).map(|h| h.path().to_owned()))
                    == tile.as_ref().map(|h| h.path().to_owned());
                if !same {
                    if !w.stroke {
                        w.stroke = true;
                        st.record(
                            &engine.world,
                            if tool == PaintTool::Erase {
                                "Erase Tiles"
                            } else {
                                "Paint Tiles"
                            },
                            None,
                            now,
                        );
                    }
                    if let Some(m) = engine.world.get_component_mut::<Tilemap>(tm) {
                        m.set_tile(c.0, c.1, tile);
                    }
                }
            }
            PaintTool::Pick if resp.clicked() => {
                w.brush = engine
                    .world
                    .get_component::<Tilemap>(tm)
                    .and_then(|m| m.get_tile(c.0, c.1).map(|h| h.path().to_owned()));
                if w.brush.is_some() {
                    w.tool = Some(PaintTool::Paint);
                }
            }
            PaintTool::Fill if resp.clicked() => {
                if brush.is_some() {
                    st.record(&engine.world, "Fill Tiles", None, now);
                    if let Some(m) = engine.world.get_component_mut::<Tilemap>(tm) {
                        let b = m.cell_bounds().unwrap_or((c, c));
                        let bounds = (
                            (b.0.0.min(c.0), b.0.1.min(c.1)),
                            (b.1.0.max(c.0), b.1.1.max(c.1)),
                        );
                        m.flood_fill(c.0, c.1, brush, bounds);
                    }
                }
            }
            PaintTool::Box => {
                if resp.drag_started() {
                    w.box_start = Some(c);
                }
                if resp.drag_stopped() || (resp.clicked() && w.box_start.is_none()) {
                    let start = w.box_start.take().unwrap_or(c);
                    if brush.is_some() {
                        st.record(&engine.world, "Box Fill Tiles", None, now);
                        if let Some(m) = engine.world.get_component_mut::<Tilemap>(tm) {
                            m.box_fill(start, c, brush);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    if !down {
        st.tile_palette.stroke = false;
    }
    true
}

/// GameObject → 2D Object → Tilemap → Rectangular: a Grid with a Tilemap child.
pub fn create_tilemap(
    world: &mut World,
    parent: Option<oxide_scene::GameObjectId>,
) -> oxide_scene::GameObjectId {
    let grid = world.create("Grid");
    if let Some(p) = parent {
        let _ = world.set_parent(grid, Some(p), false);
    }
    world.add_component(grid, Grid::default());
    let tm = world.create("Tilemap");
    let _ = world.set_parent(tm, Some(grid), false);
    world.add_component(tm, Tilemap::default());
    world.add_component(tm, oxide_scene::TilemapRenderer::default());
    tm
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rays_hit_the_right_cells() {
        let mut w = World::new();
        oxide_scene::register_builtin(&mut w.registry);
        let tm = create_tilemap(&mut w, None);
        let g = w.parent(tm).unwrap();
        w.get_component_mut::<Grid>(g).unwrap().cell_size = Vector3::new(2.0, 1.0, 0.0);
        // Straight down the -Z axis onto (3.1, -0.5).
        let ray = oxide_math::Ray {
            origin: Vector3::new(3.1, -0.5, -10.0),
            direction: Vector3::Z,
        };
        assert_eq!(cell_at(&w, tm, &ray), Some((1, -1)));
        // Moving the tilemap moves its cells.
        w.set_position(tm, Vector3::new(1.0, 0.0, 0.0));
        assert_eq!(cell_at(&w, tm, &ray), Some((1, -1)));
        w.set_position(tm, Vector3::new(2.0, 0.0, 0.0));
        assert_eq!(cell_at(&w, tm, &ray), Some((0, -1)));
    }
}
