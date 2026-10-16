//! Convex polygons from contours: ear-clipping triangulation, then merging triangles into
//! polygons of up to `nvp` vertices (Recast's `rcBuildPolyMesh`).

use super::contour::Contour;
use std::collections::HashMap;

/// What lies across a polygon edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nei {
    /// A wall.
    None,
    /// Another polygon of the same tile.
    Poly(u32),
    /// The tile border in direction 0 = -x, 1 = +z, 2 = +x, 3 = -z.
    Portal(u8),
}

pub(crate) struct Poly {
    pub verts: Vec<u32>,
    pub nei: Vec<Nei>,
    /// Nav area index (0 = Walkable).
    pub area: u8,
}

pub(crate) struct PolyMesh {
    /// `[x, y, z]` in cells.
    pub verts: Vec<[i32; 3]>,
    pub polys: Vec<Poly>,
}

/// Builds polygons; `w`/`h` are the tile size in cells (border removed), used to find edges
/// on the tile border.
pub(crate) fn build_poly_mesh(contours: &[Contour], nvp: usize, w: i32, h: i32) -> PolyMesh {
    let mut verts: Vec<[i32; 3]> = Vec::new();
    let mut buckets: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
    let mut add_vertex = |v: [i32; 3], verts: &mut Vec<[i32; 3]>| -> u32 {
        let list = buckets.entry((v[0], v[2])).or_default();
        for &i in list.iter() {
            if (verts[i as usize][1] - v[1]).abs() <= 2 {
                return i;
            }
        }
        let i = verts.len() as u32;
        verts.push(v);
        list.push(i);
        i
    };
    let mut polys: Vec<Poly> = Vec::new();
    for c in contours {
        if c.verts.len() < 3 {
            continue;
        }
        let pts: Vec<[i32; 3]> = c.verts.iter().map(|v| [v[0], v[1], v[2]]).collect();
        let tris = triangulate(&pts);
        if tris.is_empty() {
            continue;
        }
        let map: Vec<u32> = pts.iter().map(|&p| add_vertex(p, &mut verts)).collect();
        let mut cp: Vec<Vec<u32>> = Vec::new();
        for t in tris {
            let (a, b, c) = (map[t[0]], map[t[1]], map[t[2]]);
            if a != b && b != c && a != c {
                cp.push(vec![a, b, c]);
            }
        }
        if nvp > 3 {
            loop {
                let mut best = (0i32, 0usize, 0usize, 0usize, 0usize);
                for j in 0..cp.len() {
                    for k in j + 1..cp.len() {
                        if let Some((v, ea, eb)) = merge_value(&cp[j], &cp[k], &verts, nvp)
                            && v > best.0
                        {
                            best = (v, j, k, ea, eb);
                        }
                    }
                }
                if best.0 <= 0 {
                    break;
                }
                let (_, a, b, ea, eb) = best;
                let (pa, pb) = (&cp[a], &cp[b]);
                let (na, nb) = (pa.len(), pb.len());
                let mut merged = Vec::with_capacity(na + nb - 2);
                for i in 0..na - 1 {
                    merged.push(pa[(ea + 1 + i) % na]);
                }
                for i in 0..nb - 1 {
                    merged.push(pb[(eb + 1 + i) % nb]);
                }
                cp[a] = merged;
                cp.swap_remove(b);
            }
        }
        for p in cp {
            let n = p.len();
            polys.push(Poly {
                verts: p,
                nei: vec![Nei::None; n],
                area: c.area.saturating_sub(1),
            });
        }
    }
    // Adjacency through shared edges.
    let mut edges: HashMap<(u32, u32), (u32, usize)> = HashMap::new();
    for pi in 0..polys.len() {
        let n = polys[pi].verts.len();
        for e in 0..n {
            let a = polys[pi].verts[e];
            let b = polys[pi].verts[(e + 1) % n];
            let key = (a.min(b), a.max(b));
            if let Some(&(pj, ej)) = edges.get(&key) {
                if pj as usize != pi {
                    polys[pi].nei[e] = Nei::Poly(pj);
                    polys[pj as usize].nei[ej] = Nei::Poly(pi as u32);
                }
            } else {
                edges.insert(key, (pi as u32, e));
            }
        }
    }
    // Edges lying on the tile border are portals to the neighbouring tile.
    for p in &mut polys {
        let n = p.verts.len();
        for e in 0..n {
            if p.nei[e] != Nei::None {
                continue;
            }
            let a = verts[p.verts[e] as usize];
            let b = verts[p.verts[(e + 1) % n] as usize];
            p.nei[e] = if a[0] == 0 && b[0] == 0 {
                Nei::Portal(0)
            } else if a[2] == h && b[2] == h {
                Nei::Portal(1)
            } else if a[0] == w && b[0] == w {
                Nei::Portal(2)
            } else if a[2] == 0 && b[2] == 0 {
                Nei::Portal(3)
            } else {
                Nei::None
            };
        }
    }
    PolyMesh { verts, polys }
}

fn uleft(a: [i32; 3], b: [i32; 3], c: [i32; 3]) -> bool {
    area2(a, b, c) < 0
}

/// Squared length of the shared edge if `pa` and `pb` can merge into a convex polygon.
fn merge_value(
    pa: &[u32],
    pb: &[u32],
    verts: &[[i32; 3]],
    nvp: usize,
) -> Option<(i32, usize, usize)> {
    let (na, nb) = (pa.len(), pb.len());
    if na + nb - 2 > nvp {
        return None;
    }
    let mut shared = None;
    'outer: for i in 0..na {
        let (a0, a1) = (pa[i], pa[(i + 1) % na]);
        let ka = (a0.min(a1), a0.max(a1));
        for j in 0..nb {
            let (b0, b1) = (pb[j], pb[(j + 1) % nb]);
            if ka == (b0.min(b1), b0.max(b1)) {
                shared = Some((i, j));
                break 'outer;
            }
        }
    }
    let (ea, eb) = shared?;
    let v = |i: u32| verts[i as usize];
    if !uleft(v(pa[(ea + na - 1) % na]), v(pa[ea]), v(pb[(eb + 2) % nb])) {
        return None;
    }
    if !uleft(v(pb[(eb + nb - 1) % nb]), v(pb[eb]), v(pa[(ea + 2) % na])) {
        return None;
    }
    let (a, b) = (v(pa[ea]), v(pa[(ea + 1) % na]));
    let dx = b[0] - a[0];
    let dz = b[2] - a[2];
    Some((dx * dx + dz * dz, ea, eb))
}

// ---- Triangulation (Recast's, on integer x/z) ----

fn area2(a: [i32; 3], b: [i32; 3], c: [i32; 3]) -> i64 {
    (b[0] - a[0]) as i64 * (c[2] - a[2]) as i64 - (c[0] - a[0]) as i64 * (b[2] - a[2]) as i64
}
fn left(a: [i32; 3], b: [i32; 3], c: [i32; 3]) -> bool {
    area2(a, b, c) < 0
}
fn left_on(a: [i32; 3], b: [i32; 3], c: [i32; 3]) -> bool {
    area2(a, b, c) <= 0
}
fn collinear(a: [i32; 3], b: [i32; 3], c: [i32; 3]) -> bool {
    area2(a, b, c) == 0
}
fn xorb(x: bool, y: bool) -> bool {
    x != y
}
fn intersect_prop(a: [i32; 3], b: [i32; 3], c: [i32; 3], d: [i32; 3]) -> bool {
    if collinear(a, b, c) || collinear(a, b, d) || collinear(c, d, a) || collinear(c, d, b) {
        return false;
    }
    xorb(left(a, b, c), left(a, b, d)) && xorb(left(c, d, a), left(c, d, b))
}
fn between(a: [i32; 3], b: [i32; 3], c: [i32; 3]) -> bool {
    if !collinear(a, b, c) {
        return false;
    }
    if a[0] != b[0] {
        (a[0] <= c[0] && c[0] <= b[0]) || (a[0] >= c[0] && c[0] >= b[0])
    } else {
        (a[2] <= c[2] && c[2] <= b[2]) || (a[2] >= c[2] && c[2] >= b[2])
    }
}
fn intersect(a: [i32; 3], b: [i32; 3], c: [i32; 3], d: [i32; 3]) -> bool {
    intersect_prop(a, b, c, d)
        || between(a, b, c)
        || between(a, b, d)
        || between(c, d, a)
        || between(c, d, b)
}
fn vequal(a: [i32; 3], b: [i32; 3]) -> bool {
    a[0] == b[0] && a[2] == b[2]
}

const IDX: usize = 0x0fff_ffff;
const CAN_REMOVE: usize = 0x8000_0000;

fn diagonalie(i: usize, j: usize, idx: &[usize], v: &[[i32; 3]], loose: bool) -> bool {
    let d0 = v[idx[i] & IDX];
    let d1 = v[idx[j] & IDX];
    let n = idx.len();
    for k in 0..n {
        let k1 = (k + 1) % n;
        if k == i || k1 == i || k == j || k1 == j {
            continue;
        }
        let p0 = v[idx[k] & IDX];
        let p1 = v[idx[k1] & IDX];
        if vequal(d0, p0) || vequal(d1, p0) || vequal(d0, p1) || vequal(d1, p1) {
            continue;
        }
        if if loose {
            intersect_prop(d0, d1, p0, p1)
        } else {
            intersect(d0, d1, p0, p1)
        } {
            return false;
        }
    }
    true
}

fn in_cone(i: usize, j: usize, idx: &[usize], v: &[[i32; 3]], loose: bool) -> bool {
    let n = idx.len();
    let pi = v[idx[i] & IDX];
    let pj = v[idx[j] & IDX];
    let pi1 = v[idx[(i + 1) % n] & IDX];
    let pin1 = v[idx[(i + n - 1) % n] & IDX];
    if left_on(pin1, pi, pi1) {
        if loose {
            return left_on(pi, pj, pin1) && left_on(pj, pi, pi1);
        }
        return left(pi, pj, pin1) && left(pj, pi, pi1);
    }
    !(left_on(pi, pj, pi1) && left_on(pj, pi, pin1))
}

fn diagonal(i: usize, j: usize, idx: &[usize], v: &[[i32; 3]], loose: bool) -> bool {
    in_cone(i, j, idx, v, loose) && diagonalie(i, j, idx, v, loose)
}

/// Triangles as indices into `verts` (a simple polygon).
fn triangulate(verts: &[[i32; 3]]) -> Vec<[usize; 3]> {
    let mut idx: Vec<usize> = (0..verts.len()).collect();
    let mut tris = Vec::new();
    let next = |i: usize, n: usize| (i + 1) % n;
    let prev = |i: usize, n: usize| (i + n - 1) % n;
    let n0 = idx.len();
    for i in 0..n0 {
        let i2 = next(next(i, n0), n0);
        if diagonal(i, i2, &idx, verts, false) {
            let i1 = next(i, n0);
            idx[i1] |= CAN_REMOVE;
        }
    }
    while idx.len() > 3 {
        let n = idx.len();
        let mut min_len = -1i64;
        let mut mini = None;
        for i in 0..n {
            let i1 = next(i, n);
            if idx[i1] & CAN_REMOVE != 0 {
                let p0 = verts[idx[i] & IDX];
                let p2 = verts[idx[next(i1, n)] & IDX];
                let dx = (p2[0] - p0[0]) as i64;
                let dz = (p2[2] - p0[2]) as i64;
                let len = dx * dx + dz * dz;
                if min_len < 0 || len < min_len {
                    min_len = len;
                    mini = Some(i);
                }
            }
        }
        if mini.is_none() {
            // Tolerate overlapping segments (collinear contours).
            for i in 0..n {
                let i1 = next(i, n);
                let i2 = next(i1, n);
                if diagonal(i, i2, &idx, verts, true) {
                    let p0 = verts[idx[i] & IDX];
                    let p2 = verts[idx[i2] & IDX];
                    let dx = (p2[0] - p0[0]) as i64;
                    let dz = (p2[2] - p0[2]) as i64;
                    let len = dx * dx + dz * dz;
                    if min_len < 0 || len < min_len {
                        min_len = len;
                        mini = Some(i);
                    }
                }
            }
        }
        let Some(mut i) = mini else {
            // Badly formed contour: keep what was triangulated.
            return tris;
        };
        let mut i1 = next(i, n);
        let i2 = next(i1, n);
        tris.push([idx[i] & IDX, idx[i1] & IDX, idx[i2] & IDX]);
        idx.remove(i1);
        let n = idx.len();
        if i1 >= n {
            i1 = 0;
        }
        i = prev(i1, n);
        let pi = prev(i, n);
        if diagonal(pi, i1, &idx, verts, false) {
            idx[i] |= CAN_REMOVE;
        } else {
            idx[i] &= IDX;
        }
        let ni1 = next(i1, n);
        if diagonal(i, ni1, &idx, verts, false) {
            idx[i1] |= CAN_REMOVE;
        } else {
            idx[i1] &= IDX;
        }
    }
    if idx.len() == 3 {
        tris.push([idx[0] & IDX, idx[1] & IDX, idx[2] & IDX]);
    }
    tris
}
