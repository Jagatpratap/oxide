//! Compact heightfield: the open space above walkable spans with links to the neighbouring
//! spans an agent can step to; eroded by the agent radius and split into regions
//! (Recast's `rcBuildCompactHeightfield`, `rcErodeWalkableArea`, `rcBuildRegionsMonotone`).

use super::heightfield::{DX, DZ, Heightfield, NULL_AREA, SPAN_MAX};
use oxide_math::{Quaternion, Vector3};

pub(crate) const NOT_CONNECTED: u8 = 0xff;
/// Region id flag for spans in the tile border (they continue in the neighbouring tile).
pub(crate) const BORDER_REG: u16 = 0x8000;

#[derive(Clone, Copy, Debug)]
pub(crate) struct CSpan {
    /// Floor height (top of the solid span below).
    pub y: u16,
    /// Free space above the floor.
    pub h: u16,
    /// Layer index of the connected span in each neighbour column, or [`NOT_CONNECTED`].
    pub con: [u8; 4],
    pub reg: u16,
    /// Exact floor height (world).
    pub top: f32,
}

pub(crate) struct Compact {
    pub w: i32,
    pub h: i32,
    pub border: i32,
    pub bmin: Vector3,
    pub cs: f32,
    pub ch: f32,
    /// (first span index, span count) per column.
    pub cells: Vec<(u32, u32)>,
    pub spans: Vec<CSpan>,
    pub areas: Vec<u8>,
}

/// A box (or upright cylinder) that sets the area of the spans inside it.
#[derive(Clone, Copy, Debug)]
pub struct AreaVolume {
    pub center: Vector3,
    /// Half extents in the volume's local space; for a cylinder `x` is the radius.
    pub half: Vector3,
    pub rotation: Quaternion,
    pub cylinder: bool,
    /// Nav area index, or `None` to make the volume not walkable (a hole).
    pub area: Option<u8>,
}

impl AreaVolume {
    fn contains(&self, p: Vector3) -> bool {
        let l = self.rotation.inverse() * (p - self.center);
        if l.y.abs() > self.half.y {
            return false;
        }
        if self.cylinder {
            l.x * l.x + l.z * l.z <= self.half.x * self.half.x
        } else {
            l.x.abs() <= self.half.x && l.z.abs() <= self.half.z
        }
    }
}

impl Compact {
    pub fn build(hf: &Heightfield, walkable_height: i32, walkable_climb: i32, border: i32) -> Self {
        let (w, h) = (hf.w, hf.h);
        let mut cells = Vec::with_capacity((w * h) as usize);
        let mut spans = Vec::new();
        let mut areas = Vec::new();
        for col in &hf.cols {
            let start = spans.len() as u32;
            for (i, s) in col.iter().enumerate() {
                if s.area == NULL_AREA {
                    continue;
                }
                let bot = s.smax as i32;
                let top = col.get(i + 1).map_or(SPAN_MAX, |n| n.smin as i32);
                spans.push(CSpan {
                    y: bot.clamp(0, SPAN_MAX) as u16,
                    h: (top - bot).clamp(0, SPAN_MAX) as u16,
                    con: [NOT_CONNECTED; 4],
                    reg: 0,
                    top: s.top,
                });
                areas.push(s.area);
            }
            cells.push((start, spans.len() as u32 - start));
        }
        let mut c = Self {
            w,
            h,
            border,
            bmin: hf.bmin,
            cs: hf.cs,
            ch: hf.ch,
            cells,
            spans,
            areas,
        };
        for z in 0..h {
            for x in 0..w {
                let (start, count) = c.cells[(x + z * w) as usize];
                for i in start..start + count {
                    let s = c.spans[i as usize];
                    for d in 0..4 {
                        let nx = x + DX[d];
                        let nz = z + DZ[d];
                        if nx < 0 || nz < 0 || nx >= w || nz >= h {
                            continue;
                        }
                        let (ns, nc) = c.cells[(nx + nz * w) as usize];
                        for k in 0..nc {
                            let n = c.spans[(ns + k) as usize];
                            let bot = (s.y as i32).max(n.y as i32);
                            let top = (s.y as i32 + s.h as i32).min(n.y as i32 + n.h as i32);
                            if top - bot >= walkable_height
                                && (n.y as i32 - s.y as i32).abs() <= walkable_climb
                            {
                                if k < NOT_CONNECTED as u32 {
                                    c.spans[i as usize].con[d] = k as u8;
                                }
                                break;
                            }
                        }
                    }
                }
            }
        }
        c
    }

    pub fn cell(&self, x: i32, z: i32) -> std::ops::Range<usize> {
        let (s, n) = self.cells[(x + z * self.w) as usize];
        s as usize..(s + n) as usize
    }

    /// Index of the span connected to span `i` (in column `x, z`) in direction `d`.
    pub fn neighbour(&self, x: i32, z: i32, i: usize, d: usize) -> Option<usize> {
        let c = self.spans[i].con[d];
        if c == NOT_CONNECTED {
            return None;
        }
        let (s, _) = self.cells[(x + DX[d] + (z + DZ[d]) * self.w) as usize];
        Some(s as usize + c as usize)
    }

    /// Makes every span closer than `radius` cells to a non-walkable edge not walkable.
    pub fn erode(&mut self, radius: i32) {
        let n = self.spans.len();
        let mut dist = vec![255u8; n];
        let (w, h) = (self.w, self.h);
        for z in 0..h {
            for x in 0..w {
                for i in self.cell(x, z) {
                    if self.areas[i] == NULL_AREA {
                        dist[i] = 0;
                        continue;
                    }
                    let nc = (0..4)
                        .filter(|&d| {
                            self.neighbour(x, z, i, d)
                                .is_some_and(|a| self.areas[a] != NULL_AREA)
                        })
                        .count();
                    if nc != 4 {
                        dist[i] = 0;
                    }
                }
            }
        }
        let relax = |dist: &mut [u8], i: usize, a: usize, add: u8| {
            let nd = dist[a].saturating_add(add);
            if nd < dist[i] {
                dist[i] = nd;
            }
        };
        for z in 0..h {
            for x in 0..w {
                for i in self.cell(x, z) {
                    if let Some(a) = self.neighbour(x, z, i, 0) {
                        relax(&mut dist, i, a, 2);
                        if let Some(aa) = self.neighbour(x - 1, z, a, 3) {
                            relax(&mut dist, i, aa, 3);
                        }
                    }
                    if let Some(a) = self.neighbour(x, z, i, 3) {
                        relax(&mut dist, i, a, 2);
                        if let Some(aa) = self.neighbour(x, z - 1, a, 2) {
                            relax(&mut dist, i, aa, 3);
                        }
                    }
                }
            }
        }
        for z in (0..h).rev() {
            for x in (0..w).rev() {
                for i in self.cell(x, z) {
                    if let Some(a) = self.neighbour(x, z, i, 2) {
                        relax(&mut dist, i, a, 2);
                        if let Some(aa) = self.neighbour(x + 1, z, a, 1) {
                            relax(&mut dist, i, aa, 3);
                        }
                    }
                    if let Some(a) = self.neighbour(x, z, i, 1) {
                        relax(&mut dist, i, a, 2);
                        if let Some(aa) = self.neighbour(x, z + 1, a, 0) {
                            relax(&mut dist, i, aa, 3);
                        }
                    }
                }
            }
        }
        let thr = (radius * 2).min(255) as u8;
        for (a, d) in self.areas.iter_mut().zip(&dist) {
            if *d < thr {
                *a = NULL_AREA;
            }
        }
    }

    /// Applies an area volume to the walkable spans whose floor is inside it.
    pub fn mark_volume(&mut self, v: &AreaVolume) {
        for z in 0..self.h {
            for x in 0..self.w {
                for i in self.cell(x, z) {
                    if self.areas[i] == NULL_AREA {
                        continue;
                    }
                    let p = self.bmin
                        + Vector3::new(
                            (x as f32 + 0.5) * self.cs,
                            self.spans[i].y as f32 * self.ch,
                            (z as f32 + 0.5) * self.cs,
                        );
                    if v.contains(p) {
                        self.areas[i] = v.area.map_or(NULL_AREA, |a| a + 1);
                    }
                }
            }
        }
    }

    /// Partitions walkable spans into monotone regions (no holes), then removes isolated
    /// islands smaller than `min_area` cells.
    pub fn build_regions(&mut self, min_area: i32) {
        let (w, h, b) = (self.w, self.h, self.border);
        let n = self.spans.len();
        let mut reg = vec![0u16; n];
        let mut id: u16 = 1;
        if b > 0 {
            let bw = b.min(w);
            let bh = b.min(h);
            for (x0, x1, z0, z1) in [
                (0, bw, 0, h),
                (w - bw, w, 0, h),
                (0, w, 0, bh),
                (0, w, h - bh, h),
            ] {
                for z in z0..z1 {
                    for x in x0..x1 {
                        for i in self.cell(x, z) {
                            if self.areas[i] != NULL_AREA {
                                reg[i] = id | BORDER_REG;
                            }
                        }
                    }
                }
                id += 1;
            }
        }
        #[derive(Clone, Copy, Default)]
        struct Sweep {
            id: u16,
            ns: u16,
            nei: u16,
        }
        const NULL_NEI: u16 = 0xffff;
        let mut sweeps: Vec<Sweep> = Vec::new();
        let mut prev: Vec<u16> = Vec::new();
        for z in b..h - b {
            prev.clear();
            prev.resize(id as usize + 1, 0);
            sweeps.clear();
            sweeps.push(Sweep::default());
            let mut rid: u16 = 1;
            for x in b..w - b {
                for i in self.cell(x, z) {
                    if self.areas[i] == NULL_AREA {
                        continue;
                    }
                    let mut previd = 0;
                    if let Some(a) = self.neighbour(x, z, i, 0)
                        && reg[a] & BORDER_REG == 0
                        && self.areas[i] == self.areas[a]
                    {
                        previd = reg[a];
                    }
                    if previd == 0 {
                        previd = rid;
                        rid += 1;
                        sweeps.push(Sweep::default());
                    }
                    reg[i] = previd;
                    if let Some(a) = self.neighbour(x, z, i, 3) {
                        let nr = reg[a];
                        if nr != 0 && nr & BORDER_REG == 0 && self.areas[i] == self.areas[a] {
                            let s = &mut sweeps[previd as usize];
                            if s.nei == 0 || s.nei == nr {
                                s.nei = nr;
                                s.ns += 1;
                                if prev.len() <= nr as usize {
                                    prev.resize(nr as usize + 1, 0);
                                }
                                prev[nr as usize] += 1;
                            } else {
                                s.nei = NULL_NEI;
                            }
                        }
                    }
                }
            }
            for s in sweeps.iter_mut().skip(1) {
                if s.nei != NULL_NEI && s.nei != 0 && prev[s.nei as usize] == s.ns {
                    s.id = s.nei;
                } else {
                    s.id = id;
                    id = id.checked_add(1).unwrap_or(id);
                }
            }
            for x in b..w - b {
                for i in self.cell(x, z) {
                    if reg[i] > 0 && reg[i] < rid {
                        reg[i] = sweeps[reg[i] as usize].id;
                    }
                }
            }
        }
        self.filter_small_regions(&mut reg, id as usize, min_area);
        for (s, r) in self.spans.iter_mut().zip(reg) {
            s.reg = r;
        }
    }

    /// Removes groups of connected regions smaller than `min_area` spans that do not reach
    /// the tile border (those may continue in the neighbouring tile).
    fn filter_small_regions(&self, reg: &mut [u16], nreg: usize, min_area: i32) {
        let mut count = vec![0i32; nreg + 1];
        let mut adj: Vec<Vec<u16>> = vec![Vec::new(); nreg + 1];
        let mut touches_border = vec![false; nreg + 1];
        for z in 0..self.h {
            for x in 0..self.w {
                for i in self.cell(x, z) {
                    let r = reg[i];
                    if r == 0 || r & BORDER_REG != 0 {
                        continue;
                    }
                    count[r as usize] += 1;
                    for d in 0..4 {
                        if let Some(a) = self.neighbour(x, z, i, d) {
                            let nr = reg[a];
                            if nr & BORDER_REG != 0 {
                                touches_border[r as usize] = true;
                            } else if nr != 0 && nr != r && !adj[r as usize].contains(&nr) {
                                adj[r as usize].push(nr);
                            }
                        }
                    }
                }
            }
        }
        let mut seen = vec![false; nreg + 1];
        let mut remove = vec![false; nreg + 1];
        for start in 1..=nreg {
            if seen[start] || count[start] == 0 {
                continue;
            }
            let mut group = vec![start];
            seen[start] = true;
            let mut k = 0;
            while k < group.len() {
                let r = group[k];
                for &n in &adj[r] {
                    if !seen[n as usize] {
                        seen[n as usize] = true;
                        group.push(n as usize);
                    }
                }
                k += 1;
            }
            let total: i32 = group.iter().map(|&r| count[r]).sum();
            let border = group.iter().any(|&r| touches_border[r]);
            if total < min_area && !border {
                for r in group {
                    remove[r] = true;
                }
            }
        }
        for r in reg.iter_mut() {
            if *r & BORDER_REG == 0 && remove[*r as usize] {
                *r = 0;
            }
        }
    }
}
