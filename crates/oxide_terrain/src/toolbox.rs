//! Unity's Terrain Toolbox (the package's Create New Terrain, Terrain Settings and Terrain
//! Utilities): grids of connected terrain tiles, optionally from one heightmap image split
//! across them; settings applied to many terrains at once; heightmap and splatmap export.
//! And terrain streaming: far tiles drop their TerrainData and meshes, and reload from
//! their files when the camera comes back.

use crate::{Terrain, TerrainMeshes};
use oxide_assets::{AssetServer, Handle, TerrainData};
use oxide_math::Vector3;
use oxide_scene::{GameObjectId, World};
use std::collections::HashMap;

/// Create New Terrain settings.
#[derive(Clone, Debug)]
pub struct NewTerrainSettings {
    pub name: String,
    /// Tiles along X and Z.
    pub tiles_x: u32,
    pub tiles_z: u32,
    /// One tile's width, length and height (m).
    pub tile_size: Vector3,
    /// Heightmap samples per tile side (2^n + 1).
    pub heightmap_resolution: u32,
    pub grouping_id: i32,
    /// Where the first tile's corner goes.
    pub start: Vector3,
    /// Folder for the `.terraindata` files.
    pub folder: String,
}

impl Default for NewTerrainSettings {
    fn default() -> Self {
        Self {
            name: "Terrain".into(),
            tiles_x: 2,
            tiles_z: 2,
            tile_size: Vector3::new(500.0, 200.0, 500.0),
            heightmap_resolution: 257,
            grouping_id: 0,
            start: Vector3::ZERO,
            folder: "Assets/Terrains".into(),
        }
    }
}

/// A heightmap image: square, values 0..1.
#[derive(Clone, Debug, PartialEq)]
pub struct Heightmap {
    pub resolution: u32,
    pub heights: Vec<f32>,
}

impl Heightmap {
    /// Reads a 16-bit (or 8-bit) grayscale PNG, or a square little-endian 16-bit RAW.
    pub fn decode(bytes: &[u8], raw: bool) -> Result<Self, String> {
        if raw {
            let n = ((bytes.len() / 2) as f64).sqrt() as u32;
            if n < 2 || (n * n * 2) as usize != bytes.len() {
                return Err("not a square 16-bit RAW heightmap".into());
            }
            let heights = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| u16::from_le_bytes(*b) as f32 / 65535.0)
                .collect();
            return Ok(Self {
                resolution: n,
                heights,
            });
        }
        let img = image::load_from_memory(bytes)
            .map_err(|e| e.to_string())?
            .into_luma16();
        let (w, h) = img.dimensions();
        if w != h || w < 2 {
            return Err(format!("heightmaps must be square ({w}×{h})"));
        }
        Ok(Self {
            resolution: w,
            heights: img.pixels().map(|p| p.0[0] as f32 / 65535.0).collect(),
        })
    }

    /// Bilinear sample at `(u, v)` in 0..1 (v down the rows).
    pub fn sample(&self, u: f32, v: f32) -> f32 {
        let n = self.resolution as usize;
        let (x, y) = (
            u.clamp(0.0, 1.0) * (n - 1) as f32,
            v.clamp(0.0, 1.0) * (n - 1) as f32,
        );
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(n - 1), (y0 + 1).min(n - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let h = |x: usize, y: usize| self.heights[y * n + x];
        let a = h(x0, y0) + (h(x1, y0) - h(x0, y0)) * fx;
        let b = h(x0, y1) + (h(x1, y1) - h(x0, y1)) * fx;
        a + (b - a) * fy
    }

    /// Tile `(tx, tz)` of a `tiles_x` × `tiles_z` grid, resampled to `res` per side; edges
    /// are shared with the neighbours so the seams match.
    pub fn tile(&self, tx: u32, tz: u32, tiles_x: u32, tiles_z: u32, res: u32) -> Vec<f32> {
        let mut out = Vec::with_capacity((res * res) as usize);
        for z in 0..res {
            for x in 0..res {
                let u = (tx as f32 + x as f32 / (res - 1) as f32) / tiles_x as f32;
                let v = (tz as f32 + z as f32 / (res - 1) as f32) / tiles_z as f32;
                out.push(self.sample(u, v));
            }
        }
        out
    }
}

/// Unity's Create New Terrain: a grid of connected tiles saved as `.terraindata` files.
pub fn create_terrain_grid(
    world: &mut World,
    s: &NewTerrainSettings,
    heightmap: Option<&Heightmap>,
) -> Result<Vec<GameObjectId>, String> {
    let res = s.heightmap_resolution.clamp(33, 4097);
    let parent = world.create(&s.name);
    world
        .transform_mut(parent)
        .ok_or("no transform")?
        .local_position = s.start;
    let mut out = Vec::new();
    for tz in 0..s.tiles_z.max(1) {
        for tx in 0..s.tiles_x.max(1) {
            let name = format!("{}_{tx}_{tz}", s.name);
            let path = format!("{}/{name}.terraindata", s.folder.trim_end_matches('/'));
            let id = crate::create_terrain_object(world, &name, s.tile_size, res, Some(&path))?;
            world.set_parent(id, Some(parent), false)?;
            world
                .transform_mut(id)
                .ok_or("no transform")?
                .local_position =
                Vector3::new(tx as f32 * s.tile_size.x, 0.0, tz as f32 * s.tile_size.z);
            if let Some(t) = world.get_component_mut::<Terrain>(id) {
                t.grouping_id = s.grouping_id;
            }
            if let Some(hm) = heightmap {
                let h = world
                    .get_component::<Terrain>(id)
                    .and_then(|t| t.data.clone())
                    .ok_or("no data")?;
                let assets = world.resource_mut::<AssetServer>();
                let d = assets.terrains.get_mut(&h).ok_or("no data")?;
                d.set_heights(hm.tile(tx, tz, s.tiles_x.max(1), s.tiles_z.max(1), res));
                let data = d.clone();
                assets.save_terrain_data(&path, &data)?;
            }
            out.push(id);
        }
    }
    Ok(out)
}

/// The Terrain Toolbox's Terrain Settings: what to apply to every terrain.
#[derive(Clone, Debug)]
pub struct BatchSettings {
    pub template: Terrain,
    /// Which fields to apply (by name), leaving the others as they are.
    pub fields: Vec<&'static str>,
}

/// Applies the chosen fields of `s.template` to `targets`; returns how many changed.
pub fn apply_settings(world: &mut World, targets: &[GameObjectId], s: &BatchSettings) -> usize {
    use oxide_core::reflect::Reflect as _;
    let mut n = 0;
    for &id in targets {
        let Some(t) = world.get_component_mut::<Terrain>(id) else {
            continue;
        };
        for f in &s.fields {
            if let Some(v) = s.template.get_field(f) {
                let _ = t.set_field(f, v);
            }
        }
        n += 1;
    }
    n
}

/// 16-bit grayscale PNG of a terrain's heights.
pub fn heightmap_png(d: &TerrainData) -> Result<Vec<u8>, String> {
    let n = d.resolution;
    let img = image::ImageBuffer::<image::Luma<u16>, Vec<u16>>::from_raw(
        n,
        n,
        d.heights
            .iter()
            .map(|h| (h.clamp(0.0, 1.0) * 65535.0).round() as u16)
            .collect(),
    )
    .ok_or("bad heightmap size")?;
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

/// Splatmaps as RGBA PNGs: layers 0-3 in the first, 4-7 in the second, ...
pub fn splatmap_pngs(d: &TerrainData) -> Result<Vec<Vec<u8>>, String> {
    let n = d.splat_resolution.max(1);
    let layers = d.alphamap_count();
    let mut out = Vec::new();
    for map in 0..layers.div_ceil(4) {
        let mut px = Vec::with_capacity((n * n * 4) as usize);
        for z in 0..n {
            for x in 0..n {
                for c in 0..4 {
                    let l = map * 4 + c;
                    let w = if l < layers { d.alpha(l, x, z) } else { 0.0 };
                    px.push((w.clamp(0.0, 1.0) * 255.0).round() as u8);
                }
            }
        }
        let img = image::RgbaImage::from_raw(n, n, px).ok_or("bad splatmap size")?;
        let mut bytes = std::io::Cursor::new(Vec::new());
        img.write_to(&mut bytes, image::ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        out.push(bytes.into_inner());
    }
    Ok(out)
}

// ---- streaming -----------------------------------------------------------------------------

/// Terrains whose data is unloaded, with their size (to measure the distance) — a World
/// resource.
#[derive(Default)]
pub struct TerrainStreaming {
    pub unloaded: HashMap<String, Vector3>,
}

/// Horizontal distance from `p` to a terrain's rectangle.
fn distance_to(origin: Vector3, size: Vector3, p: Vector3) -> f32 {
    let dx = (origin.x - p.x).max(0.0).max(p.x - (origin.x + size.x));
    let dz = (origin.z - p.z).max(0.0).max(p.z - (origin.z + size.z));
    (dx * dx + dz * dz).sqrt()
}

/// Unloads streaming terrains farther than their stream distance (plus 10%) from `camera`
/// and reloads (one per call) those that came within it. Returns (loaded, unloaded) paths.
pub fn stream(world: &mut World, camera: Vector3) -> (Vec<String>, Vec<String>) {
    let terrains: Vec<(GameObjectId, Handle<TerrainData>, f32)> = world
        .query::<Terrain>()
        .filter(|(_, t)| t.streaming)
        .filter_map(|(id, t)| t.data.clone().map(|h| (id, h, t.stream_distance)))
        .filter(|(_, h, _)| h.path().ends_with(".terraindata"))
        .collect();
    if terrains.is_empty() {
        return (Vec::new(), Vec::new());
    }
    if world.try_resource::<TerrainStreaming>().is_none() {
        world.resources.insert(TerrainStreaming::default());
    }
    let mut state = world
        .resources
        .remove::<TerrainStreaming>()
        .expect("inserted");
    let mut meshes = world
        .resources
        .remove::<TerrainMeshes>()
        .unwrap_or_default();
    let (mut loaded, mut unloaded) = (Vec::new(), Vec::new());
    for (id, h, range) in terrains {
        let origin = world.position(id);
        let path = h.path().to_owned();
        let assets = world.resource_mut::<AssetServer>();
        match assets.terrains.get(&h).map(|d| d.size) {
            Some(size) => {
                if distance_to(origin, size, camera) > range * 1.1 {
                    assets.terrains.remove(&h);
                    meshes.release(assets, h.id());
                    state.unloaded.insert(path.clone(), size);
                    unloaded.push(path);
                }
            }
            None => {
                let Some(size) = state.unloaded.get(&path).copied() else {
                    continue;
                };
                if loaded.is_empty() && distance_to(origin, size, camera) <= range {
                    match assets.load_terrain_data(&path) {
                        Ok(_) => {
                            state.unloaded.remove(&path);
                            loaded.push(path);
                        }
                        Err(e) => oxide_core::debug_log_warning!("terrain streaming: {e}"),
                    }
                }
            }
        }
    }
    world.resources.insert(meshes);
    world.resources.insert(state);
    (loaded, unloaded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heightmaps_split_into_tiles_with_matching_edges() {
        // A 5×5 ramp from 0 (west) to 1 (east).
        let hm = Heightmap {
            resolution: 5,
            heights: (0..25).map(|i| (i % 5) as f32 / 4.0).collect(),
        };
        let left = hm.tile(0, 0, 2, 1, 3);
        let right = hm.tile(1, 0, 2, 1, 3);
        // The west tile's east edge is the east tile's west edge.
        for z in 0..3 {
            assert!((left[z * 3 + 2] - right[z * 3]).abs() < 1e-5);
        }
        assert_eq!(left[0], 0.0);
        assert!((right[2] - 1.0).abs() < 1e-5);
        assert!((left[2] - 0.5).abs() < 1e-5);
        // 16-bit RAW round trip.
        let raw: Vec<u8> = [0u16, 65535, 32768, 0]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let d = Heightmap::decode(&raw, true).unwrap();
        assert_eq!(d.resolution, 2);
        assert_eq!(d.heights[1], 1.0);
        assert!(Heightmap::decode(&raw[..6], true).is_err());
    }

    #[test]
    fn png_exports_decode_back() {
        let mut d = TerrainData::flat(Vector3::new(10.0, 5.0, 10.0), 9);
        d.set_heights((0..81).map(|i| i as f32 / 80.0).collect());
        let png = heightmap_png(&d).unwrap();
        let back = Heightmap::decode(&png, false).unwrap();
        assert_eq!(back.resolution, 9);
        assert!((back.heights[40] - 0.5).abs() < 1e-4);
        d.splat_resolution = 8;
        d.add_layer(Default::default(), None);
        d.add_layer(Default::default(), None);
        d.fit_alphamaps();
        let maps = splatmap_pngs(&d).unwrap();
        assert_eq!(maps.len(), 1, "two layers fit one RGBA map");
        let img = image::load_from_memory(&maps[0]).unwrap().into_rgba8();
        assert_eq!(img.dimensions(), (8, 8));
        assert_eq!(
            img.get_pixel(3, 3).0[0],
            255,
            "the first layer covers everything"
        );
    }

    #[test]
    fn distance_to_a_tile() {
        let (o, s) = (Vector3::ZERO, Vector3::new(100.0, 10.0, 100.0));
        assert_eq!(distance_to(o, s, Vector3::new(50.0, 0.0, 50.0)), 0.0);
        assert_eq!(distance_to(o, s, Vector3::new(130.0, 0.0, 50.0)), 30.0);
        assert!((distance_to(o, s, Vector3::new(-30.0, 0.0, -40.0)) - 50.0).abs() < 1e-4);
    }
}
