//! Baked NavMesh data (Unity `NavMeshData`), saved as a `.navmesh` binary file next to the
//! scene. Coordinates are in the NavMeshSurface's local space, so a surface that moves (or a
//! prefab) carries its NavMesh with it.

use crate::build::{BuildSettings, Nei, TileGrid};
use oxide_math::{Quaternion, Vector3};

#[derive(Clone, Debug, PartialEq)]
pub struct NavPoly {
    /// Indices into the tile's vertices, convex, at most 6.
    pub verts: Vec<u16>,
    /// What lies across each edge (edge `i` runs from vertex `i` to `i + 1`).
    pub neis: Vec<Nei>,
    /// Nav area index (0 Walkable, 2 Jump, 3+ user areas).
    pub area: u8,
}

/// Floor heights of a tile's voxel columns.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct HeightGrid {
    pub origin: Vector3,
    pub cs: f32,
    /// Height units: `y = origin.y + ys[i] * ch`.
    pub ch: f32,
    pub size: u32,
    /// (first height, count) per cell, row-major from `origin`.
    pub cells: Vec<(u32, u8)>,
    pub ys: Vec<u16>,
}

impl HeightGrid {
    /// Floor height at the cell containing `(x, z)` closest to `guess`.
    fn cell_height(&self, cx: i32, cz: i32, guess: f32, tol: f32) -> Option<f32> {
        if cx < 0 || cz < 0 || cx >= self.size as i32 || cz >= self.size as i32 {
            return None;
        }
        let (s, n) = self.cells[(cx + cz * self.size as i32) as usize];
        let mut best: Option<f32> = None;
        for k in s..s + n as u32 {
            let y = self.origin.y + self.ys[k as usize] as f32 * self.ch;
            if (y - guess).abs() <= tol
                && best.is_none_or(|b| (y - guess).abs() < (b - guess).abs())
            {
                best = Some(y);
            }
        }
        best
    }

    /// Ground height under `p` (local space), interpolated between cell centres; `guess` picks
    /// the floor when several are stacked.
    pub fn height(&self, p: Vector3, guess: f32, tol: f32) -> Option<f32> {
        if self.cells.is_empty() {
            return None;
        }
        let fx = (p.x - self.origin.x) / self.cs - 0.5;
        let fz = (p.z - self.origin.z) / self.cs - 0.5;
        let x0 = fx.floor() as i32;
        let z0 = fz.floor() as i32;
        let (tx, tz) = (fx - x0 as f32, fz - z0 as f32);
        let mut acc = 0.0;
        let mut wsum = 0.0;
        for (dx, dz, w) in [
            (0, 0, (1.0 - tx) * (1.0 - tz)),
            (1, 0, tx * (1.0 - tz)),
            (0, 1, (1.0 - tx) * tz),
            (1, 1, tx * tz),
        ] {
            if w <= 0.0 {
                continue;
            }
            if let Some(y) = self.cell_height(x0 + dx, z0 + dz, guess, tol) {
                acc += y * w;
                wsum += w;
            }
        }
        (wsum > 1e-4).then(|| acc / wsum)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NavMeshTile {
    pub x: i32,
    pub z: i32,
    pub verts: Vec<Vector3>,
    pub polys: Vec<NavPoly>,
    pub heights: HeightGrid,
}

/// Unity `NavMeshData`.
#[derive(Clone, Debug)]
pub struct NavMeshData {
    pub agent_type_id: i32,
    pub settings: BuildSettings,
    pub grid: TileGrid,
    /// Where the surface was when baked (the data is in its local space).
    pub position: Vector3,
    pub rotation: Quaternion,
    pub tiles: Vec<NavMeshTile>,
}

impl oxide_assets::Asset for NavMeshData {
    const TYPE_NAME: &'static str = "NavMeshData";
}

impl NavMeshData {
    pub fn polygon_count(&self) -> usize {
        self.tiles.iter().map(|t| t.polys.len()).sum()
    }

    /// Local-space bounds of all polygons.
    pub fn bounds(&self) -> Option<(Vector3, Vector3)> {
        let mut it = self.tiles.iter().flat_map(|t| t.verts.iter().copied());
        let f = it.next()?;
        Some(it.fold((f, f), |(a, b), p| (a.min(p), b.max(p))))
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = Writer(Vec::new());
        w.0.extend_from_slice(MAGIC);
        w.u32(VERSION);
        w.i32(self.agent_type_id);
        let s = &self.settings;
        for f in [
            s.agent_radius,
            s.agent_height,
            s.agent_climb,
            s.agent_slope,
            s.voxel_size,
            s.min_region_area,
        ] {
            w.f32(f);
        }
        w.u32(s.tile_size);
        let g = &self.grid;
        w.v3(g.origin);
        w.f32(g.tile_size);
        w.i32(g.tiles_x);
        w.i32(g.tiles_z);
        w.f32(g.ymin);
        w.f32(g.ymax);
        w.v3(self.position);
        for f in self.rotation.to_array() {
            w.f32(f);
        }
        w.u32(self.tiles.len() as u32);
        for t in &self.tiles {
            w.i32(t.x);
            w.i32(t.z);
            w.u32(t.verts.len() as u32);
            for v in &t.verts {
                w.v3(*v);
            }
            w.u32(t.polys.len() as u32);
            for p in &t.polys {
                w.0.push(p.verts.len() as u8);
                w.0.push(p.area);
                for &v in &p.verts {
                    w.u16(v);
                }
                for n in &p.neis {
                    match n {
                        Nei::None => w.u32(0),
                        Nei::Poly(i) => w.u32(i + 1),
                        Nei::Portal(d) => w.u32(0x8000_0000 | *d as u32),
                    }
                }
            }
            let h = &t.heights;
            w.v3(h.origin);
            w.f32(h.cs);
            w.f32(h.ch);
            w.u32(h.size);
            w.u32(h.cells.len() as u32);
            for &(s, n) in &h.cells {
                w.u32(s);
                w.0.push(n);
            }
            w.u32(h.ys.len() as u32);
            for &y in &h.ys {
                w.u16(y);
            }
        }
        w.0
    }

    pub fn from_bytes(b: &[u8]) -> Result<Self, String> {
        let mut r = Reader { b, at: 0 };
        if r.take(4)? != MAGIC {
            return Err("not a NavMesh file".into());
        }
        let ver = r.u32()?;
        if ver != VERSION {
            return Err(format!(
                "NavMesh file version {ver} is not supported (rebake it)"
            ));
        }
        let agent_type_id = r.i32()?;
        let mut f = [0.0f32; 6];
        for x in &mut f {
            *x = r.f32()?;
        }
        let settings = BuildSettings {
            agent_radius: f[0],
            agent_height: f[1],
            agent_climb: f[2],
            agent_slope: f[3],
            voxel_size: f[4],
            min_region_area: f[5],
            tile_size: r.u32()?,
        };
        let grid = TileGrid {
            origin: r.v3()?,
            tile_size: r.f32()?,
            tiles_x: r.i32()?,
            tiles_z: r.i32()?,
            ymin: r.f32()?,
            ymax: r.f32()?,
        };
        let position = r.v3()?;
        let rotation = Quaternion::from_array([r.f32()?, r.f32()?, r.f32()?, r.f32()?]);
        let nt = r.u32()? as usize;
        let mut tiles = Vec::with_capacity(nt.min(1 << 16));
        for _ in 0..nt {
            let x = r.i32()?;
            let z = r.i32()?;
            let nv = r.u32()? as usize;
            let mut verts = Vec::with_capacity(nv.min(1 << 20));
            for _ in 0..nv {
                verts.push(r.v3()?);
            }
            let np = r.u32()? as usize;
            let mut polys = Vec::with_capacity(np.min(1 << 20));
            for _ in 0..np {
                let n = r.u8()? as usize;
                let area = r.u8()?;
                let mut pv = Vec::with_capacity(n);
                for _ in 0..n {
                    let v = r.u16()?;
                    if v as usize >= nv {
                        return Err("corrupt NavMesh file".into());
                    }
                    pv.push(v);
                }
                let mut neis = Vec::with_capacity(n);
                for _ in 0..n {
                    let v = r.u32()?;
                    neis.push(if v == 0 {
                        Nei::None
                    } else if v & 0x8000_0000 != 0 {
                        Nei::Portal((v & 3) as u8)
                    } else if ((v - 1) as usize) < np {
                        Nei::Poly(v - 1)
                    } else {
                        return Err("corrupt NavMesh file".into());
                    });
                }
                polys.push(NavPoly {
                    verts: pv,
                    neis,
                    area,
                });
            }
            let origin = r.v3()?;
            let cs = r.f32()?;
            let ch = r.f32()?;
            let size = r.u32()?;
            let nc = r.u32()? as usize;
            let mut cells = Vec::with_capacity(nc.min(1 << 22));
            for _ in 0..nc {
                cells.push((r.u32()?, r.u8()?));
            }
            let ny = r.u32()? as usize;
            let mut ys = Vec::with_capacity(ny.min(1 << 22));
            for _ in 0..ny {
                ys.push(r.u16()?);
            }
            if cells
                .iter()
                .any(|&(s, n)| s as usize + n as usize > ys.len())
                || nc != (size * size) as usize && nc != 0
            {
                return Err("corrupt NavMesh file".into());
            }
            tiles.push(NavMeshTile {
                x,
                z,
                verts,
                polys,
                heights: HeightGrid {
                    origin,
                    cs,
                    ch,
                    size,
                    cells,
                    ys,
                },
            });
        }
        Ok(Self {
            agent_type_id,
            settings,
            grid,
            position,
            rotation,
            tiles,
        })
    }
}

const MAGIC: &[u8; 4] = b"ONAV";
const VERSION: u32 = 2;

struct Writer(Vec<u8>);
impl Writer {
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn v3(&mut self, v: Vector3) {
        self.f32(v.x);
        self.f32(v.y);
        self.f32(v.z);
    }
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}
impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let s = self
            .b
            .get(self.at..self.at + n)
            .ok_or("truncated NavMesh file")?;
        self.at += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(
            self.take(2)?.try_into().unwrap_or_default(),
        ))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().unwrap_or_default(),
        ))
    }
    fn i32(&mut self) -> Result<i32, String> {
        Ok(i32::from_le_bytes(
            self.take(4)?.try_into().unwrap_or_default(),
        ))
    }
    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_le_bytes(
            self.take(4)?.try_into().unwrap_or_default(),
        ))
    }
    fn v3(&mut self) -> Result<Vector3, String> {
        Ok(Vector3::new(self.f32()?, self.f32()?, self.f32()?))
    }
}
