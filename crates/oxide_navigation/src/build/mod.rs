//! NavMesh baking, following Recast (which Unity's NavMesh builder is based on): triangles
//! are voxelized per tile, filtered by the agent's height, step height and slope, eroded by
//! its radius, split into regions, traced into contours and turned into convex polygons.

mod compact;
mod contour;
mod heightfield;
mod polymesh;

pub use compact::AreaVolume;
pub use polymesh::Nei;

use crate::data::{HeightGrid, NavMeshTile, NavPoly};
use heightfield::Heightfield;
use oxide_math::Vector3;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Unity `NavMeshBuildSettings`: the agent the NavMesh is built for and voxel resolution.
#[derive(Clone, Debug, PartialEq)]
pub struct BuildSettings {
    pub agent_radius: f32,
    pub agent_height: f32,
    /// Step height (m).
    pub agent_climb: f32,
    /// Max slope (degrees).
    pub agent_slope: f32,
    /// Voxel size (m); Unity's default is a third of the agent radius.
    pub voxel_size: f32,
    /// Tile size in voxels.
    pub tile_size: u32,
    /// Islands smaller than this (m²) are dropped.
    pub min_region_area: f32,
}

impl Default for BuildSettings {
    fn default() -> Self {
        Self {
            agent_radius: 0.5,
            agent_height: 2.0,
            agent_climb: 0.75,
            agent_slope: 45.0,
            voxel_size: 0.5 / 3.0,
            tile_size: 256,
            min_region_area: 2.0,
        }
    }
}

impl BuildSettings {
    pub fn cell_height(&self) -> f32 {
        self.voxel_size * 0.5
    }
    pub fn tile_world_size(&self) -> f32 {
        self.tile_size.max(8) as f32 * self.voxel_size
    }
}

/// Triangles to build from, already in the NavMesh's space.
#[derive(Clone, Debug, Default)]
pub struct BuildSource {
    pub verts: Vec<Vector3>,
    pub tris: Vec<[u32; 3]>,
    /// Nav area per triangle; `None` for Not Walkable (it still blocks).
    pub areas: Vec<Option<u8>>,
}

impl BuildSource {
    pub fn add_triangles(&mut self, verts: &[Vector3], tris: &[[u32; 3]], area: Option<u8>) {
        let base = self.verts.len() as u32;
        self.verts.extend_from_slice(verts);
        self.tris
            .extend(tris.iter().map(|t| [t[0] + base, t[1] + base, t[2] + base]));
        self.areas.extend(std::iter::repeat_n(area, tris.len()));
    }

    pub fn bounds(&self) -> Option<(Vector3, Vector3)> {
        let mut it = self.tris.iter().flatten().map(|&i| self.verts[i as usize]);
        let first = it.next()?;
        Some(it.fold((first, first), |(a, b), p| (a.min(p), b.max(p))))
    }
}

/// The tile grid a NavMesh is built on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TileGrid {
    pub origin: Vector3,
    pub tile_size: f32,
    pub tiles_x: i32,
    pub tiles_z: i32,
    pub ymin: f32,
    pub ymax: f32,
}

impl TileGrid {
    pub fn new(bmin: Vector3, bmax: Vector3, s: &BuildSettings) -> Self {
        let ts = s.tile_world_size();
        let size = bmax - bmin;
        Self {
            origin: bmin,
            tile_size: ts,
            tiles_x: ((size.x / ts).ceil() as i32).max(1),
            tiles_z: ((size.z / ts).ceil() as i32).max(1),
            ymin: bmin.y - 1.0,
            ymax: bmax.y + s.agent_height + 1.0,
        }
    }

    /// Tiles whose area overlaps the x/z box.
    pub fn tiles_overlapping(
        &self,
        min: Vector3,
        max: Vector3,
    ) -> impl Iterator<Item = (i32, i32)> + use<> {
        let ts = self.tile_size;
        let x0 = (((min.x - self.origin.x) / ts).floor() as i32).max(0);
        let x1 = (((max.x - self.origin.x) / ts).floor() as i32).min(self.tiles_x - 1);
        let z0 = (((min.z - self.origin.z) / ts).floor() as i32).max(0);
        let z1 = (((max.z - self.origin.z) / ts).floor() as i32).min(self.tiles_z - 1);
        (z0..=z1).flat_map(move |z| (x0..=x1).map(move |x| (x, z)))
    }
}

/// Progress of a build that can run on another thread.
#[derive(Default)]
pub struct BuildProgress {
    pub done: AtomicUsize,
    pub total: AtomicUsize,
    pub cancel: AtomicBool,
}

impl BuildProgress {
    pub fn fraction(&self) -> f32 {
        let t = self.total.load(Ordering::Relaxed);
        if t == 0 {
            0.0
        } else {
            self.done.load(Ordering::Relaxed) as f32 / t as f32
        }
    }
}

/// Builds every tile of the grid (using all cores).
pub fn build_tiles(
    src: &BuildSource,
    volumes: &[AreaVolume],
    s: &BuildSettings,
    grid: &TileGrid,
    progress: &BuildProgress,
) -> Result<Vec<NavMeshTile>, String> {
    // Bucket triangles by tile (with the border margin) so each tile only sees its own.
    let border = (s.agent_radius / s.voxel_size).ceil() + 3.0;
    let margin = border * s.voxel_size;
    let nt = (grid.tiles_x * grid.tiles_z) as usize;
    let mut buckets: Vec<Vec<u32>> = vec![Vec::new(); nt];
    for (ti, t) in src.tris.iter().enumerate() {
        let v = t.map(|i| src.verts[i as usize]);
        let min = v[0].min(v[1]).min(v[2]) - Vector3::splat(margin);
        let max = v[0].max(v[1]).max(v[2]) + Vector3::splat(margin);
        for (x, z) in grid.tiles_overlapping(min, max) {
            buckets[(x + z * grid.tiles_x) as usize].push(ti as u32);
        }
    }
    progress.total.store(nt, Ordering::Relaxed);
    progress.done.store(0, Ordering::Relaxed);
    let next = AtomicUsize::new(0);
    let workers = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .min(nt.max(1));
    let results: Vec<Option<Vec<NavMeshTile>>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(|| {
                    let mut out = Vec::new();
                    loop {
                        if progress.cancel.load(Ordering::Relaxed) {
                            break;
                        }
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        if i >= nt {
                            break;
                        }
                        let (x, z) = (i as i32 % grid.tiles_x, i as i32 / grid.tiles_x);
                        if !buckets[i].is_empty()
                            && let Some(t) = build_tile(src, &buckets[i], volumes, s, grid, x, z)
                        {
                            out.push(t);
                        }
                        progress.done.fetch_add(1, Ordering::Relaxed);
                    }
                    out
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().ok()).collect()
    });
    if progress.cancel.load(Ordering::Relaxed) {
        return Err("cancelled".into());
    }
    let mut tiles = Vec::new();
    for r in results {
        tiles.extend(r.ok_or("a NavMesh build thread crashed")?);
    }
    tiles.sort_by_key(|t| (t.z, t.x));
    Ok(tiles)
}

/// Triangles of `src` that can affect tile `(x, z)`.
pub fn tile_triangles(
    src: &BuildSource,
    s: &BuildSettings,
    grid: &TileGrid,
    x: i32,
    z: i32,
) -> Vec<u32> {
    let border = (s.agent_radius / s.voxel_size).ceil() + 3.0;
    let margin = border * s.voxel_size;
    let tmin = grid.origin
        + Vector3::new(
            x as f32 * grid.tile_size - margin,
            0.0,
            z as f32 * grid.tile_size - margin,
        );
    let tmax = tmin
        + Vector3::new(
            grid.tile_size + 2.0 * margin,
            0.0,
            grid.tile_size + 2.0 * margin,
        );
    src.tris
        .iter()
        .enumerate()
        .filter(|(_, t)| {
            let v = t.map(|i| src.verts[i as usize]);
            let min = v[0].min(v[1]).min(v[2]);
            let max = v[0].max(v[1]).max(v[2]);
            min.x <= tmax.x && max.x >= tmin.x && min.z <= tmax.z && max.z >= tmin.z
        })
        .map(|(i, _)| i as u32)
        .collect()
}

/// Builds one tile from the given triangles; `None` if nothing walkable is in it.
pub fn build_tile(
    src: &BuildSource,
    tris: &[u32],
    volumes: &[AreaVolume],
    s: &BuildSettings,
    grid: &TileGrid,
    tx: i32,
    tz: i32,
) -> Option<NavMeshTile> {
    let cs = s.voxel_size;
    let ch = s.cell_height();
    let walkable_height = (s.agent_height / ch).ceil() as i32;
    let walkable_climb = (s.agent_climb / ch).floor() as i32;
    let walkable_radius = (s.agent_radius / cs).ceil() as i32;
    let border = walkable_radius + 3;
    let tile_cells = s.tile_size.max(8) as i32;
    let w = tile_cells + border * 2;
    let tile_min =
        grid.origin + Vector3::new(tx as f32 * grid.tile_size, 0.0, tz as f32 * grid.tile_size);
    let bmin = Vector3::new(
        tile_min.x - border as f32 * cs,
        grid.ymin,
        tile_min.z - border as f32 * cs,
    );
    let bmax = Vector3::new(
        tile_min.x + (tile_cells + border) as f32 * cs,
        grid.ymax,
        tile_min.z + (tile_cells + border) as f32 * cs,
    );
    let mut hf = Heightfield::new(w, w, bmin, bmax, cs, ch);
    let walk_cos = s.agent_slope.clamp(0.0, 89.9).to_radians().cos();
    for &ti in tris {
        let t = src.tris[ti as usize];
        let v = t.map(|i| src.verts[i as usize]);
        let n = (v[1] - v[0]).cross(v[2] - v[0]);
        let len = n.length();
        if len <= 1e-12 {
            continue;
        }
        let area = match src.areas[ti as usize] {
            Some(a) if n.y / len > walk_cos => a + 1,
            _ => heightfield::NULL_AREA,
        };
        hf.rasterize_triangle(v, area, walkable_climb);
    }
    hf.filter_low_hanging_obstacles(walkable_climb);
    hf.filter_ledge_spans(walkable_height, walkable_climb);
    hf.filter_low_height_spans(walkable_height);
    let mut chf = compact::Compact::build(&hf, walkable_height, walkable_climb, border);
    drop(hf);
    for v in volumes {
        chf.mark_volume(v);
    }
    chf.erode(walkable_radius);
    let min_area = (s.min_region_area / (cs * cs)).round() as i32;
    chf.build_regions(min_area);
    let max_edge = (12.0 / cs) as i32;
    let contours = contour::build_contours(&chf, 1.3, max_edge);
    if contours.is_empty() {
        return None;
    }
    let pm = polymesh::build_poly_mesh(&contours, 6, tile_cells, tile_cells);
    if pm.polys.is_empty() {
        return None;
    }
    let origin = Vector3::new(tile_min.x, bmin.y, tile_min.z);
    let verts = pm
        .verts
        .iter()
        .map(|v| origin + Vector3::new(v[0] as f32 * cs, v[1] as f32 * ch, v[2] as f32 * cs))
        .collect();
    let polys = pm
        .polys
        .into_iter()
        .map(|p| NavPoly {
            verts: p.verts.iter().map(|&v| v as u16).collect(),
            neis: p.nei,
            area: p.area,
        })
        .collect();
    // Floor heights of the tile's cells, for placing agents on the real ground.
    // Exact heights quantized over the tile's height range (about 1.5 cm per 1 km).
    let hscale = ((grid.ymax - grid.ymin) / 65535.0).max(1e-4);
    let mut cells = Vec::with_capacity((tile_cells * tile_cells) as usize);
    let mut ys = Vec::new();
    for z in border..border + tile_cells {
        for x in border..border + tile_cells {
            let start = ys.len() as u32;
            for i in chf.cell(x, z) {
                let q = ((chf.spans[i].top - origin.y) / hscale).round();
                ys.push(q.clamp(0.0, 65535.0) as u16);
            }
            cells.push((start, (ys.len() as u32 - start).min(255) as u8));
        }
    }
    Some(NavMeshTile {
        x: tx,
        z: tz,
        verts,
        polys,
        heights: HeightGrid {
            origin,
            cs,
            ch: hscale,
            size: tile_cells as u32,
            cells,
            ys,
        },
    })
}
