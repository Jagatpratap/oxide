//! 2D extraction: SpriteRenderers become sprite draws (Simple, Sliced and Tiled draw
//! modes, flips, sorting layers).

use oxide_assets::{AssetServer, Sprite};
use oxide_math::{Matrix4x4, Vector2, Vector3};
use oxide_render::{SpriteDraw, SpriteInstance};
use oxide_scene::{Grid, SpriteDrawMode, SpriteRenderer, Tilemap, TilemapRenderer, World};

/// A quad of `size` at `offset` (local units) under `model`, showing sprite pixels
/// `(x, y, w, h)`.
fn quad(
    model: Matrix4x4,
    offset: Vector2,
    size: Vector2,
    s: &Sprite,
    px: [f32; 4],
) -> SpriteInstance {
    let local = Matrix4x4::from_translation(Vector3::new(offset.x, offset.y, 0.0))
        * Matrix4x4::from_scale(Vector3::new(size.x, size.y, 1.0));
    SpriteInstance {
        model: (model * local).to_cols_array_2d(),
        uv_rect: s.uv_rect_of(px[0], px[1], px[2], px[3]),
        color: [1.0; 4],
    }
}

/// Quads for one sprite drawn at `size` (world units) in `mode`, before flips: local
/// space has the sprite's pivot at the origin.
pub fn sprite_quads(
    model: Matrix4x4,
    s: &Sprite,
    mode: SpriteDrawMode,
    size: Vector2,
) -> Vec<SpriteInstance> {
    let ppu = s.pixels_per_unit.max(0.0001);
    let (pw, ph) = (s.rect.z, s.rect.w);
    match mode {
        SpriteDrawMode::Simple => {
            let sz = s.size();
            vec![quad(model, -s.pivot * sz, sz, s, [0.0, 0.0, pw, ph])]
        }
        SpriteDrawMode::Sliced => {
            // Borders keep their pixel size; the middle stretches (Unity 9-slice).
            let (l, b, r, t) = (s.border.x, s.border.y, s.border.z, s.border.w);
            let xs_px = [0.0, l, pw - r, pw];
            let ys_px = [0.0, b, ph - t, ph];
            let size = Vector2::new(size.x.abs(), size.y.abs());
            let xs = [0.0, l / ppu, size.x - r / ppu, size.x];
            let ys = [0.0, b / ppu, size.y - t / ppu, size.y];
            let origin = -s.pivot * size;
            let mut out = Vec::new();
            for j in 0..3 {
                for i in 0..3 {
                    let (w, h) = (xs[i + 1] - xs[i], ys[j + 1] - ys[j]);
                    let (pw_, ph_) = (xs_px[i + 1] - xs_px[i], ys_px[j + 1] - ys_px[j]);
                    if w <= 0.0 || h <= 0.0 || pw_ <= 0.0 || ph_ <= 0.0 {
                        continue;
                    }
                    out.push(quad(
                        model,
                        origin + Vector2::new(xs[i], ys[j]),
                        Vector2::new(w, h),
                        s,
                        [xs_px[i], ys_px[j], pw_, ph_],
                    ));
                }
            }
            out
        }
        SpriteDrawMode::Tiled => {
            let tile = s.size();
            let size = Vector2::new(size.x.abs(), size.y.abs());
            let origin = -s.pivot * size;
            let nx = (size.x / tile.x).ceil().clamp(1.0, 256.0) as usize;
            let ny = (size.y / tile.y).ceil().clamp(1.0, 256.0) as usize;
            let mut out = Vec::new();
            for j in 0..ny {
                for i in 0..nx {
                    let x = i as f32 * tile.x;
                    let y = j as f32 * tile.y;
                    // The last row and column show only the part that fits.
                    let w = (size.x - x).min(tile.x);
                    let h = (size.y - y).min(tile.y);
                    out.push(quad(
                        model,
                        origin + Vector2::new(x, y),
                        Vector2::new(w, h),
                        s,
                        [0.0, 0.0, w * ppu, h * ppu],
                    ));
                }
            }
            out
        }
    }
}

/// Tilemaps: every painted cell becomes a sprite quad at its cell (Grid cell size and
/// gap, Tilemap tile anchor), sorted by the TilemapRenderer.
fn extract_tilemaps(world: &World, out: &mut Vec<SpriteDraw>) {
    let assets = world.resource::<AssetServer>();
    let time = world
        .try_resource::<oxide_core::Time>()
        .map_or(0.0, |t| t.time());
    for (id, map) in world.query::<Tilemap>() {
        if !world.get(id).is_some_and(|o| o.active_in_hierarchy()) {
            continue;
        }
        let Some(r) = world.get_component::<TilemapRenderer>(id) else {
            continue;
        };
        let grid = world
            .ancestor_with::<Grid>(id)
            .and_then(|g| world.get_component::<Grid>(g))
            .cloned()
            .unwrap_or_default();
        let (cs, gap) = (grid.cell_size, grid.cell_gap);
        let model = world.local_to_world(id);
        let layer = world.sorting_layer_index(&r.sorting_layer);
        let cells = map.cell_map();
        let tint = map.color;
        for c in &map.tiles {
            let Some(tile) = &c.tile else { continue };
            let Some((sh, color)) = oxide_scene::tilemap::tile_sprite(
                assets,
                &cells,
                tile.path(),
                c.x,
                c.y,
                time * map.animation_frame_rate,
            ) else {
                continue;
            };
            let Some(s) = assets.sprites.get(&sh) else {
                continue;
            };
            let at = Vector3::new(
                c.x as f32 * (cs.x + gap.x) + map.tile_anchor.x * cs.x,
                c.y as f32 * (cs.y + gap.y) + map.tile_anchor.y * cs.y,
                0.0,
            );
            let m = model * Matrix4x4::from_translation(at);
            let col = Color4::mul(tint, color);
            for mut inst in sprite_quads(m, s, SpriteDrawMode::Simple, Vector2::ONE) {
                inst.color = col;
                out.push(SpriteDraw {
                    texture: Some(s.texture.id()),
                    sorting_layer: layer,
                    order: r.order_in_layer,
                    instance: inst,
                });
            }
        }
    }
}

struct Color4;

impl Color4 {
    /// Linear RGB and alpha of two multiplied sRGB tints.
    fn mul(a: oxide_math::Color, b: oxide_math::Color) -> [f32; 4] {
        let c = oxide_math::Color::new(a.r * b.r, a.g * b.g, a.b * b.b, a.a * b.a).linear();
        [c.r, c.g, c.b, a.a * b.a]
    }
}

pub fn extract(world: &World, out: &mut Vec<SpriteDraw>) {
    extract_tilemaps(world, out);
    let assets = world.resource::<AssetServer>();
    for (id, r) in world.query::<SpriteRenderer>() {
        if !world.get(id).is_some_and(|o| o.active_in_hierarchy()) {
            continue;
        }
        // A Sprite Resolver decides the sprite (also in Edit mode, like Unity).
        let resolved = oxide_scene::sprite_library::resolved_sprite(world, id);
        let Some(s) = resolved
            .as_ref()
            .or(r.sprite.as_ref())
            .and_then(|h| assets.sprites.get(h))
        else {
            continue;
        };
        let flip = Matrix4x4::from_scale(Vector3::new(
            if r.flip_x { -1.0 } else { 1.0 },
            if r.flip_y { -1.0 } else { 1.0 },
            1.0,
        ));
        let model = world.local_to_world(id) * flip;
        let c = r.color.linear();
        let layer = world.sorting_layer_index(&r.sorting_layer);
        for mut inst in sprite_quads(model, s, r.draw_mode, r.size) {
            inst.color = [c.r, c.g, c.b, r.color.a];
            out.push(SpriteDraw {
                texture: Some(s.texture.id()),
                sorting_layer: layer,
                order: r.order_in_layer,
                instance: inst,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxide_assets::Handle;
    use oxide_math::Vector4;

    fn sprite(border: Vector4) -> Sprite {
        Sprite {
            name: "s".into(),
            texture: Handle::from_path("t"),
            rect: Vector4::new(0.0, 0.0, 30.0, 30.0),
            pivot: Vector2::new(0.5, 0.5),
            pixels_per_unit: 10.0,
            border,
            texture_size: Vector2::new(30.0, 30.0),
        }
    }

    fn corner(i: &SpriteInstance) -> Vector3 {
        Vector3::new(i.model[3][0], i.model[3][1], i.model[3][2])
    }

    #[test]
    fn simple_is_centred_on_the_pivot() {
        let q = sprite_quads(
            Matrix4x4::IDENTITY,
            &sprite(Vector4::ZERO),
            SpriteDrawMode::Simple,
            Vector2::ONE,
        );
        assert_eq!(q.len(), 1);
        assert_eq!(corner(&q[0]), Vector3::new(-1.5, -1.5, 0.0));
        assert_eq!(q[0].uv_rect, [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn sliced_keeps_borders_and_stretches_the_middle() {
        let s = sprite(Vector4::new(10.0, 10.0, 10.0, 10.0));
        let q = sprite_quads(
            Matrix4x4::IDENTITY,
            &s,
            SpriteDrawMode::Sliced,
            Vector2::new(10.0, 4.0),
        );
        assert_eq!(q.len(), 9);
        // Bottom-left corner: 1×1 units (10 px at 10 ppu) at the rect's corner.
        assert_eq!(corner(&q[0]), Vector3::new(-5.0, -2.0, 0.0));
        assert!((q[0].model[0][0] - 1.0).abs() < 1e-5);
        // Middle: stretched to 8×2.
        assert!((q[4].model[0][0] - 8.0).abs() < 1e-5 && (q[4].model[1][1] - 2.0).abs() < 1e-5);
    }

    #[test]
    fn tiled_repeats_and_crops_the_last_tile() {
        let s = sprite(Vector4::ZERO);
        let q = sprite_quads(
            Matrix4x4::IDENTITY,
            &s,
            SpriteDrawMode::Tiled,
            Vector2::new(7.5, 3.0),
        );
        assert_eq!(q.len(), 3, "3 units per tile: 2 full + 1 partial");
        assert!(
            (q[2].model[0][0] - 1.5).abs() < 1e-5,
            "last tile is cropped"
        );
        assert!(
            (q[2].uv_rect[2] - 0.5).abs() < 1e-5,
            "and shows half the texture"
        );
    }
}
