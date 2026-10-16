//! Solid heightfield: triangles rasterized into columns of solid spans, then filtered for
//! walkability (Recast's `rcRasterizeTriangles` and `rcFilter*`).

use oxide_math::Vector3;

/// Area of a span nothing can walk on. Walkable spans store `nav area index + 1`.
pub(crate) const NULL_AREA: u8 = 0;
pub(crate) const SPAN_MAX: i32 = u16::MAX as i32;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Span {
    pub smin: u16,
    pub smax: u16,
    pub area: u8,
    /// Exact height of the surface on top (voxel heights are rounded up).
    pub top: f32,
}

pub(crate) struct Heightfield {
    pub w: i32,
    pub h: i32,
    pub bmin: Vector3,
    pub bmax: Vector3,
    pub cs: f32,
    pub ch: f32,
    /// Spans per column (`x + z * w`), sorted bottom to top, never overlapping.
    pub cols: Vec<Vec<Span>>,
}

impl Heightfield {
    pub fn new(w: i32, h: i32, bmin: Vector3, bmax: Vector3, cs: f32, ch: f32) -> Self {
        Self {
            w,
            h,
            bmin,
            bmax,
            cs,
            ch,
            cols: vec![Vec::new(); (w * h) as usize],
        }
    }

    /// Adds a solid span, merging it with the spans it touches. When the tops of merged spans
    /// are within `merge_thr` the higher area wins (walkable over not walkable).
    pub fn add_span(&mut self, x: i32, z: i32, mut s: Span, merge_thr: i32) {
        let col = &mut self.cols[(x + z * self.w) as usize];
        let mut i = 0;
        while i < col.len() {
            let c = col[i];
            if c.smin > s.smax {
                break;
            }
            if c.smax < s.smin {
                i += 1;
                continue;
            }
            s.smin = s.smin.min(c.smin);
            if c.smax > s.smax {
                s.top = c.top;
            } else if c.smax == s.smax {
                s.top = s.top.max(c.top);
            }
            s.smax = s.smax.max(c.smax);
            if (s.smax as i32 - c.smax as i32).abs() <= merge_thr {
                s.area = s.area.max(c.area);
            }
            col.remove(i);
        }
        col.insert(i, s);
    }

    /// Rasterizes one triangle.
    pub fn rasterize_triangle(&mut self, v: [Vector3; 3], area: u8, merge_thr: i32) {
        let tmin = v[0].min(v[1]).min(v[2]);
        let tmax = v[0].max(v[1]).max(v[2]);
        if tmin.x > self.bmax.x
            || tmax.x < self.bmin.x
            || tmin.y > self.bmax.y
            || tmax.y < self.bmin.y
            || tmin.z > self.bmax.z
            || tmax.z < self.bmin.z
        {
            return;
        }
        let ics = 1.0 / self.cs;
        let ich = 1.0 / self.ch;
        let by = self.bmax.y - self.bmin.y;
        let z0 = (((tmin.z - self.bmin.z) * ics).floor() as i32).clamp(-1, self.h - 1);
        let z1 = (((tmax.z - self.bmin.z) * ics).floor() as i32).clamp(0, self.h - 1);
        let mut rest: Vec<Vector3> = v.to_vec();
        for z in z0..=z1 {
            let cz = self.bmin.z + z as f32 * self.cs;
            let (row, above) = divide_poly(&rest, cz + self.cs, 2);
            rest = above;
            if row.len() < 3 || z < 0 {
                continue;
            }
            let (mut minx, mut maxx) = (row[0].x, row[0].x);
            for p in &row[1..] {
                minx = minx.min(p.x);
                maxx = maxx.max(p.x);
            }
            let x0 = ((minx - self.bmin.x) * ics).floor() as i32;
            let x1 = ((maxx - self.bmin.x) * ics).floor() as i32;
            if x1 < 0 || x0 >= self.w {
                continue;
            }
            let x0 = x0.clamp(-1, self.w - 1);
            let x1 = x1.clamp(0, self.w - 1);
            let mut rrest = row;
            for x in x0..=x1 {
                let cx = self.bmin.x + x as f32 * self.cs;
                let (cell, right) = divide_poly(&rrest, cx + self.cs, 0);
                rrest = right;
                if cell.len() < 3 || x < 0 {
                    continue;
                }
                let (mut smin, mut smax) = (cell[0].y, cell[0].y);
                for p in &cell[1..] {
                    smin = smin.min(p.y);
                    smax = smax.max(p.y);
                }
                let top = smax;
                smin -= self.bmin.y;
                smax -= self.bmin.y;
                if smax < 0.0 || smin > by {
                    continue;
                }
                let smin = smin.max(0.0);
                let smax = smax.min(by);
                let ismin = ((smin * ich).floor() as i32).clamp(0, SPAN_MAX - 1);
                let ismax = ((smax * ich).ceil() as i32).clamp(ismin + 1, SPAN_MAX);
                let span = Span {
                    smin: ismin as u16,
                    smax: ismax as u16,
                    area,
                    top,
                };
                self.add_span(x, z, span, merge_thr);
            }
        }
    }

    /// A non-walkable span whose top is within `climb` of the walkable span below it becomes
    /// walkable (curbs, stairs).
    pub fn filter_low_hanging_obstacles(&mut self, climb: i32) {
        for col in &mut self.cols {
            let mut prev_walkable = false;
            let mut prev_area = NULL_AREA;
            let mut prev_max = 0i32;
            for s in col.iter_mut() {
                let walkable = s.area != NULL_AREA;
                if !walkable && prev_walkable && (s.smax as i32 - prev_max).abs() <= climb {
                    s.area = prev_area;
                }
                prev_walkable = walkable;
                prev_area = s.area;
                prev_max = s.smax as i32;
            }
        }
    }

    /// Spans at a drop higher than `climb` (ledges), or on steep stepped ground, become
    /// not walkable.
    pub fn filter_ledge_spans(&mut self, height: i32, climb: i32) {
        let (w, h) = (self.w, self.h);
        let mut out = Vec::new();
        for z in 0..h {
            for x in 0..w {
                let col = &self.cols[(x + z * w) as usize];
                for (si, s) in col.iter().enumerate() {
                    if s.area == NULL_AREA {
                        continue;
                    }
                    let bot = s.smax as i32;
                    let top = col.get(si + 1).map_or(SPAN_MAX, |n| n.smin as i32);
                    let mut minh = SPAN_MAX;
                    let (mut asmin, mut asmax) = (bot, bot);
                    for d in 0..4 {
                        let nx = x + DX[d];
                        let nz = z + DZ[d];
                        if nx < 0 || nz < 0 || nx >= w || nz >= h {
                            minh = minh.min(-climb - bot);
                            continue;
                        }
                        let ncol = &self.cols[(nx + nz * w) as usize];
                        let nbot = -climb;
                        let ntop = ncol.first().map_or(SPAN_MAX, |n| n.smin as i32);
                        if top.min(ntop) - bot.max(nbot) > height {
                            minh = minh.min(nbot - bot);
                        }
                        for (ni, ns) in ncol.iter().enumerate() {
                            let nbot = ns.smax as i32;
                            let ntop = ncol.get(ni + 1).map_or(SPAN_MAX, |n| n.smin as i32);
                            if top.min(ntop) - bot.max(nbot) > height {
                                minh = minh.min(nbot - bot);
                                if (nbot - bot).abs() <= climb {
                                    asmin = asmin.min(nbot);
                                    asmax = asmax.max(nbot);
                                }
                            }
                        }
                    }
                    if minh < -climb || asmax - asmin > climb {
                        out.push(((x + z * w) as usize, si));
                    }
                }
            }
        }
        for (c, s) in out {
            self.cols[c][s].area = NULL_AREA;
        }
    }

    /// Spans with less than `height` free space above become not walkable.
    pub fn filter_low_height_spans(&mut self, height: i32) {
        for col in &mut self.cols {
            for i in 0..col.len() {
                let bot = col[i].smax as i32;
                let top = col.get(i + 1).map_or(SPAN_MAX, |n| n.smin as i32);
                if top - bot < height {
                    col[i].area = NULL_AREA;
                }
            }
        }
    }
}

/// Neighbour offsets per direction: 0 = -x, 1 = +z, 2 = +x, 3 = -z.
pub(crate) const DX: [i32; 4] = [-1, 0, 1, 0];
pub(crate) const DZ: [i32; 4] = [0, 1, 0, -1];

/// Splits a convex polygon by the plane `p[axis] = at` into (`<= at`, `>= at`).
fn divide_poly(inp: &[Vector3], at: f32, axis: usize) -> (Vec<Vector3>, Vec<Vector3>) {
    if inp.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let mut below = Vec::with_capacity(inp.len() + 2);
    let mut above = Vec::with_capacity(inp.len() + 2);
    let d: Vec<f32> = inp.iter().map(|p| at - p[axis]).collect();
    let n = inp.len();
    let mut j = n - 1;
    for i in 0..n {
        let ina = d[j] >= 0.0;
        let inb = d[i] >= 0.0;
        if ina != inb {
            let s = d[j] / (d[j] - d[i]);
            let p = inp[j] + (inp[i] - inp[j]) * s;
            below.push(p);
            above.push(p);
            if d[i] > 0.0 {
                below.push(inp[i]);
            } else if d[i] < 0.0 {
                above.push(inp[i]);
            }
        } else {
            if d[i] >= 0.0 {
                below.push(inp[i]);
                if d[i] != 0.0 {
                    j = i;
                    continue;
                }
            }
            above.push(inp[i]);
        }
        j = i;
    }
    (below, above)
}
