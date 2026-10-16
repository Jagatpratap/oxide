//! Unity's Tilemap system: a Grid (cell size) with Tilemaps of tiles, drawn by a
//! TilemapRenderer, and the tile assets they hold: Tile (a sprite), Animated Tile (sprites
//! over time) and Rule Tile (a sprite chosen by which neighbours are the same tile).
//! Tile Palettes (`.palette`) hold tiles for painting.
//! Reference: <https://docs.unity3d.com/Manual/tilemaps/tilemaps-landing.html>

use crate::component::Component;
use oxide_assets::{Asset, AssetServer, Handle, Sprite};
use oxide_core::Reflect;
use oxide_math::{Color, Vector3};
use std::collections::HashMap;

/// Any tile asset (Tile, Animated Tile, Rule Tile), referenced by path.
#[derive(Clone, Debug)]
pub struct TileBase;

impl Asset for TileBase {
    const TYPE_NAME: &'static str = "TileBase";
}

/// Unity `Tile.ColliderType`.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TileColliderType {
    None,
    #[default]
    Sprite,
    Grid,
}

/// Unity `Tile` (`.tile`): one sprite with a tint.
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct Tile {
    pub sprite: Option<Handle<Sprite>>,
    pub color: Color,
    pub collider_type: TileColliderType,
}

impl Default for Tile {
    fn default() -> Self {
        Self {
            sprite: None,
            color: Color::WHITE,
            collider_type: TileColliderType::Sprite,
        }
    }
}

impl Asset for Tile {
    const TYPE_NAME: &'static str = "Tile";
}

/// Unity `AnimatedTile` (2D Tilemap Extras, `.animatedtile`).
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct AnimatedTile {
    pub sprites: Vec<Handle<Sprite>>,
    /// Frames per second (a random speed between min and max per tile).
    pub min_speed: f32,
    pub max_speed: f32,
    /// Start offset in frames.
    pub start_time: f32,
}

impl Default for AnimatedTile {
    fn default() -> Self {
        Self {
            sprites: Vec::new(),
            min_speed: 4.0,
            max_speed: 4.0,
            start_time: 0.0,
        }
    }
}

impl Asset for AnimatedTile {
    const TYPE_NAME: &'static str = "AnimatedTile";
}

/// What a Rule Tile rule wants in one neighbouring cell.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RuleNeighbor {
    #[default]
    DontCare,
    /// The same Rule Tile.
    This,
    /// Anything else (or empty).
    NotThis,
}

/// How a matching rule picks its sprite.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RuleOutput {
    #[default]
    Single,
    /// One of `sprites`, stable per cell.
    Random,
    /// `sprites` animated at `speed` frames per second.
    Animation,
}

/// One rule (Unity `RuleTile.TilingRule`).
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct TilingRule {
    /// Top-left, top, top-right, left, right, bottom-left, bottom, bottom-right.
    pub neighbors: Vec<RuleNeighbor>,
    pub sprites: Vec<Handle<Sprite>>,
    pub output: RuleOutput,
    pub speed: f32,
}

impl Default for TilingRule {
    fn default() -> Self {
        Self {
            neighbors: vec![RuleNeighbor::DontCare; 8],
            sprites: Vec::new(),
            output: RuleOutput::Single,
            speed: 4.0,
        }
    }
}

/// Unity `RuleTile` (2D Tilemap Extras, `.ruletile`): the first rule whose neighbours
/// match decides the sprite; otherwise `default_sprite`.
#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct RuleTile {
    pub default_sprite: Option<Handle<Sprite>>,
    pub rules: Vec<TilingRule>,
}

impl Asset for RuleTile {
    const TYPE_NAME: &'static str = "RuleTile";
}

/// Neighbour offsets in rule order.
pub const NEIGHBORS: [(i32, i32); 8] = [
    (-1, 1),
    (0, 1),
    (1, 1),
    (-1, 0),
    (1, 0),
    (-1, -1),
    (0, -1),
    (1, -1),
];

/// Unity `GridLayout.CellLayout` (rectangular grids for now).
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GridCellLayout {
    #[default]
    Rectangle,
}

/// Unity `Grid`: the cell size of the Tilemaps below it.
#[derive(Reflect, Clone, Debug)]
pub struct Grid {
    pub cell_size: Vector3,
    pub cell_gap: Vector3,
    pub cell_layout: GridCellLayout,
}

impl Default for Grid {
    fn default() -> Self {
        Self {
            cell_size: Vector3::new(1.0, 1.0, 0.0),
            cell_gap: Vector3::ZERO,
            cell_layout: GridCellLayout::Rectangle,
        }
    }
}

impl Component for Grid {
    fn allow_multiple(&self) -> bool {
        false
    }
}

/// One painted cell.
#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct TileCell {
    pub x: i32,
    pub y: i32,
    pub tile: Option<Handle<TileBase>>,
}

/// Unity `Tilemap`: tiles by cell.
#[derive(Reflect, Clone, Debug)]
pub struct Tilemap {
    pub tiles: Vec<TileCell>,
    /// Tints every tile.
    pub color: Color,
    /// Where in its cell a tile's sprite pivot sits (0.5, 0.5 = centre).
    pub tile_anchor: Vector3,
    /// Multiplies Animated Tile speeds.
    pub animation_frame_rate: f32,
}

impl Default for Tilemap {
    fn default() -> Self {
        Self {
            tiles: Vec::new(),
            color: Color::WHITE,
            tile_anchor: Vector3::new(0.5, 0.5, 0.0),
            animation_frame_rate: 1.0,
        }
    }
}

impl Component for Tilemap {
    fn allow_multiple(&self) -> bool {
        false
    }
}

impl Tilemap {
    pub fn get_tile(&self, x: i32, y: i32) -> Option<&Handle<TileBase>> {
        self.tiles
            .iter()
            .find(|c| c.x == x && c.y == y)
            .and_then(|c| c.tile.as_ref())
    }

    /// Unity `SetTile` (`None` erases).
    pub fn set_tile(&mut self, x: i32, y: i32, tile: Option<Handle<TileBase>>) {
        self.tiles.retain(|c| c.x != x || c.y != y);
        if tile.is_some() {
            self.tiles.push(TileCell { x, y, tile });
        }
    }

    /// Unity `BoxFill`: every cell between the two corners.
    pub fn box_fill(&mut self, a: (i32, i32), b: (i32, i32), tile: Option<Handle<TileBase>>) {
        for x in a.0.min(b.0)..=a.0.max(b.0) {
            for y in a.1.min(b.1)..=a.1.max(b.1) {
                self.set_tile(x, y, tile.clone());
            }
        }
    }

    /// Unity `FloodFill`: replaces the connected area of the same tile as `(x, y)` within
    /// `bounds` (min, max) — empty areas are bounded by the given box.
    pub fn flood_fill(
        &mut self,
        x: i32,
        y: i32,
        tile: Option<Handle<TileBase>>,
        bounds: ((i32, i32), (i32, i32)),
    ) {
        let target = self.get_tile(x, y).map(|h| h.path().to_owned());
        if target.as_deref() == tile.as_ref().map(|h| h.path()) {
            return;
        }
        let map = self.cell_map();
        let mut stack = vec![(x, y)];
        let mut seen = std::collections::HashSet::new();
        let ((x0, y0), (x1, y1)) = bounds;
        while let Some((cx, cy)) = stack.pop() {
            if cx < x0 || cy < y0 || cx > x1 || cy > y1 || !seen.insert((cx, cy)) {
                continue;
            }
            if map.get(&(cx, cy)).map(|s| s.as_str()) != target.as_deref() {
                continue;
            }
            self.set_tile(cx, cy, tile.clone());
            stack.extend([(cx + 1, cy), (cx - 1, cy), (cx, cy + 1), (cx, cy - 1)]);
        }
    }

    pub fn clear_all_tiles(&mut self) {
        self.tiles.clear();
    }

    /// Cell → tile path.
    pub fn cell_map(&self) -> HashMap<(i32, i32), String> {
        self.tiles
            .iter()
            .filter_map(|c| Some(((c.x, c.y), c.tile.as_ref()?.path().to_owned())))
            .collect()
    }

    /// Cells covered: (min, max), or `None` when empty.
    pub fn cell_bounds(&self) -> Option<((i32, i32), (i32, i32))> {
        let first = self.tiles.first()?;
        let mut b = ((first.x, first.y), (first.x, first.y));
        for c in &self.tiles {
            b.0 = (b.0.0.min(c.x), b.0.1.min(c.y));
            b.1 = (b.1.0.max(c.x), b.1.1.max(c.y));
        }
        Some(b)
    }
}

/// Unity `TilemapRenderer`.
#[derive(Reflect, Clone, Debug)]
pub struct TilemapRenderer {
    pub sorting_layer: String,
    pub order_in_layer: i32,
}

impl Default for TilemapRenderer {
    fn default() -> Self {
        Self {
            sorting_layer: "Default".into(),
            order_in_layer: 0,
        }
    }
}

impl Component for TilemapRenderer {
    fn allow_multiple(&self) -> bool {
        false
    }
}

/// A Tile Palette (`.palette`): tiles laid out for painting (Unity keeps palettes as
/// prefabs; here they are a small data asset).
#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct TilePalette {
    pub tiles: Vec<TileCell>,
}

impl Asset for TilePalette {
    const TYPE_NAME: &'static str = "TilePalette";
}

/// A stable small hash of a cell (Random rules, animation speeds).
fn cell_hash(x: i32, y: i32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x9E37_79B1) ^ (y as u32).wrapping_mul(0x85EB_CA77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^ (h >> 12)
}

/// The sprite (and tint) a tile shows at `(x, y)` at `time`, given the map of cells.
pub fn tile_sprite(
    assets: &AssetServer,
    map: &HashMap<(i32, i32), String>,
    path: &str,
    x: i32,
    y: i32,
    time: f32,
) -> Option<(Handle<Sprite>, Color)> {
    let data = &assets.data;
    match data.type_of(path)? {
        "Tile" => {
            let t = data.get(&Handle::<Tile>::from_path(path))?;
            Some((t.sprite.clone()?, t.color))
        }
        "AnimatedTile" => {
            let t = data.get(&Handle::<AnimatedTile>::from_path(path))?;
            let n = t.sprites.len();
            if n == 0 {
                return None;
            }
            let k = (cell_hash(x, y) % 1000) as f32 / 1000.0;
            let speed = t.min_speed + (t.max_speed - t.min_speed) * k;
            let frame = (time * speed + t.start_time).floor().max(0.0) as usize % n;
            Some((t.sprites[frame].clone(), Color::WHITE))
        }
        "RuleTile" => {
            let t = data.get(&Handle::<RuleTile>::from_path(path))?;
            let same = |dx: i32, dy: i32| map.get(&(x + dx, y + dy)).is_some_and(|p| p == path);
            for rule in &t.rules {
                let matches = rule
                    .neighbors
                    .iter()
                    .zip(NEIGHBORS)
                    .all(|(n, (dx, dy))| match n {
                        RuleNeighbor::DontCare => true,
                        RuleNeighbor::This => same(dx, dy),
                        RuleNeighbor::NotThis => !same(dx, dy),
                    });
                if !matches || rule.sprites.is_empty() {
                    continue;
                }
                let n = rule.sprites.len();
                let i = match rule.output {
                    RuleOutput::Single => 0,
                    RuleOutput::Random => cell_hash(x, y) as usize % n,
                    RuleOutput::Animation => (time * rule.speed).floor().max(0.0) as usize % n,
                };
                return Some((rule.sprites[i].clone(), Color::WHITE));
            }
            Some((t.default_sprite.clone()?, Color::WHITE))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(p: &str) -> Option<Handle<TileBase>> {
        Some(Handle::from_path(p))
    }

    #[test]
    fn set_box_and_flood_fill() {
        let mut m = Tilemap::default();
        m.box_fill((0, 0), (2, 1), h("grass"));
        assert_eq!(m.tiles.len(), 6);
        m.set_tile(1, 0, None);
        assert_eq!(m.tiles.len(), 5);
        // Flood the 2×3 area of grass (minus the hole) with water.
        m.flood_fill(0, 0, h("water"), ((-5, -5), (5, 5)));
        assert_eq!(m.get_tile(2, 1).unwrap().path(), "water");
        assert!(m.get_tile(1, 0).is_none(), "the erased cell stays empty");
        assert_eq!(m.cell_bounds(), Some(((0, 0), (2, 1))));
    }

    #[test]
    fn rule_tiles_pick_by_neighbours_and_animated_tiles_by_time() {
        let mut a = AssetServer::new(".");
        let s = |n: &str| Handle::<Sprite>::from_path(format!("test:{n}"));
        let edge = TilingRule {
            // Nothing above: the top edge sprite.
            neighbors: vec![
                RuleNeighbor::DontCare,
                RuleNeighbor::NotThis,
                RuleNeighbor::DontCare,
                RuleNeighbor::DontCare,
                RuleNeighbor::DontCare,
                RuleNeighbor::DontCare,
                RuleNeighbor::DontCare,
                RuleNeighbor::DontCare,
            ],
            sprites: vec![s("top")],
            ..Default::default()
        };
        a.data.insert(
            "Assets/Ground.ruletile",
            "RuleTile",
            Box::new(RuleTile {
                default_sprite: Some(s("fill")),
                rules: vec![edge],
            }),
        );
        a.data.insert(
            "Assets/Water.animatedtile",
            "AnimatedTile",
            Box::new(AnimatedTile {
                sprites: vec![s("w0"), s("w1"), s("w2")],
                ..Default::default()
            }),
        );
        let mut m = Tilemap::default();
        m.box_fill((0, 0), (0, 1), h("Assets/Ground.ruletile"));
        let map = m.cell_map();
        let top = tile_sprite(&a, &map, "Assets/Ground.ruletile", 0, 1, 0.0)
            .unwrap()
            .0;
        let below = tile_sprite(&a, &map, "Assets/Ground.ruletile", 0, 0, 0.0)
            .unwrap()
            .0;
        assert_eq!(top.path(), "test:top");
        assert_eq!(
            below.path(),
            "test:fill",
            "a tile above: the rule doesn't match"
        );
        let f0 = tile_sprite(&a, &map, "Assets/Water.animatedtile", 3, 3, 0.1)
            .unwrap()
            .0;
        let f1 = tile_sprite(&a, &map, "Assets/Water.animatedtile", 3, 3, 0.3)
            .unwrap()
            .0;
        assert_eq!(
            (f0.path(), f1.path()),
            ("test:w0", "test:w1"),
            "4 frames per second"
        );
    }
}
