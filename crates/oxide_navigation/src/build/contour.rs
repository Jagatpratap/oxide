//! Region outlines traced along span edges and simplified (Recast's `rcBuildContours`).

use super::compact::{BORDER_REG, Compact};
use super::heightfield::{DX, DZ};

const BORDER_VERTEX: i32 = 0x10000;
const AREA_BORDER: i32 = 0x20000;
const REG_MASK: i32 = 0xffff;

pub(crate) struct Contour {
    /// `[x, y, z, flags]` in cells, tile border removed; `flags & 0xffff` is the region on
    /// the other side of the edge starting at this vertex.
    pub verts: Vec<[i32; 4]>,
    pub area: u8,
}

pub(crate) fn build_contours(chf: &Compact, max_error: f32, max_edge_len: i32) -> Vec<Contour> {
    let (w, h) = (chf.w, chf.h);
    let mut flags = vec![0u8; chf.spans.len()];
    for z in 0..h {
        for x in 0..w {
            for i in chf.cell(x, z) {
                let s = &chf.spans[i];
                if s.reg == 0 || s.reg & BORDER_REG != 0 {
                    continue;
                }
                let mut res = 0u8;
                for d in 0..4 {
                    let r = chf.neighbour(x, z, i, d).map_or(0, |a| chf.spans[a].reg);
                    if r == s.reg {
                        res |= 1 << d;
                    }
                }
                // Bits set on the edges that are boundaries.
                flags[i] = res ^ 0xf;
            }
        }
    }
    let mut out = Vec::new();
    for z in 0..h {
        for x in 0..w {
            for i in chf.cell(x, z) {
                if flags[i] == 0 || flags[i] == 0xf {
                    flags[i] = 0;
                    continue;
                }
                let reg = chf.spans[i].reg;
                if reg == 0 || reg & BORDER_REG != 0 {
                    continue;
                }
                let area = chf.areas[i];
                let raw = walk_contour(chf, x, z, i, &mut flags);
                let mut simp = simplify(&raw, max_error, max_edge_len);
                remove_degenerate(&mut simp);
                if simp.len() >= 3 {
                    for v in &mut simp {
                        v[0] -= chf.border;
                        v[2] -= chf.border;
                    }
                    out.push(Contour { verts: simp, area });
                }
            }
        }
    }
    out
}

fn walk_contour(
    chf: &Compact,
    mut x: i32,
    mut z: i32,
    mut i: usize,
    flags: &mut [u8],
) -> Vec<[i32; 4]> {
    let mut dir = 0usize;
    while flags[i] & (1 << dir) == 0 {
        dir += 1;
    }
    let start_dir = dir;
    let start_i = i;
    let area = chf.areas[i];
    let mut pts = Vec::new();
    for _ in 0..40000 {
        if flags[i] & (1 << dir) != 0 {
            let (py, is_border_vertex) = corner_height(chf, x, z, i, dir);
            let (mut px, mut pz) = (x, z);
            match dir {
                0 => pz += 1,
                1 => {
                    px += 1;
                    pz += 1
                }
                2 => px += 1,
                _ => {}
            }
            let mut r = 0i32;
            let mut is_area_border = false;
            if let Some(a) = chf.neighbour(x, z, i, dir) {
                r = chf.spans[a].reg as i32;
                if area != chf.areas[a] {
                    is_area_border = true;
                }
            }
            if is_border_vertex {
                r |= BORDER_VERTEX;
            }
            if is_area_border {
                r |= AREA_BORDER;
            }
            pts.push([px, py, pz, r]);
            flags[i] &= !(1 << dir);
            dir = (dir + 1) & 3;
        } else {
            let Some(ni) = chf.neighbour(x, z, i, dir) else {
                break;
            };
            x += DX[dir];
            z += DZ[dir];
            i = ni;
            dir = (dir + 3) & 3;
        }
        if i == start_i && dir == start_dir {
            break;
        }
    }
    pts
}

/// Height of the corner shared by span `i` and its neighbours in `dir` and `dir + 1`, and
/// whether it is a tile-border vertex that could be removed.
fn corner_height(chf: &Compact, x: i32, z: i32, i: usize, dir: usize) -> (i32, bool) {
    let s = &chf.spans[i];
    let mut ch = s.y as i32;
    let dirp = (dir + 1) & 3;
    let key = |k: usize| chf.spans[k].reg as u32 | ((chf.areas[k] as u32) << 16);
    let mut regs = [0u32; 4];
    regs[0] = key(i);
    if let Some(a) = chf.neighbour(x, z, i, dir) {
        let (ax, az) = (x + DX[dir], z + DZ[dir]);
        ch = ch.max(chf.spans[a].y as i32);
        regs[1] = key(a);
        if let Some(a2) = chf.neighbour(ax, az, a, dirp) {
            ch = ch.max(chf.spans[a2].y as i32);
            regs[2] = key(a2);
        }
    }
    if let Some(a) = chf.neighbour(x, z, i, dirp) {
        let (ax, az) = (x + DX[dirp], z + DZ[dirp]);
        ch = ch.max(chf.spans[a].y as i32);
        regs[3] = key(a);
        if let Some(a2) = chf.neighbour(ax, az, a, dir) {
            ch = ch.max(chf.spans[a2].y as i32);
            regs[2] = key(a2);
        }
    }
    let border = BORDER_REG as u32;
    let mut is_border = false;
    for j in 0..4 {
        let (a, b, c, d) = (j, (j + 1) & 3, (j + 2) & 3, (j + 3) & 3);
        let two_same_exts = regs[a] & regs[b] & border != 0 && regs[a] == regs[b];
        let two_ints = (regs[c] | regs[d]) & border == 0;
        let ints_same_area = regs[c] >> 16 == regs[d] >> 16;
        let no_zeros = regs.iter().all(|&r| r != 0);
        if two_same_exts && two_ints && ints_same_area && no_zeros {
            is_border = true;
            break;
        }
    }
    (ch, is_border)
}

fn dist_pt_seg2(x: i32, z: i32, px: i32, pz: i32, qx: i32, qz: i32) -> f32 {
    let pqx = (qx - px) as f32;
    let pqz = (qz - pz) as f32;
    let dx = (x - px) as f32;
    let dz = (z - pz) as f32;
    let d = pqx * pqx + pqz * pqz;
    let mut t = pqx * dx + pqz * dz;
    if d > 0.0 {
        t /= d;
    }
    let t = t.clamp(0.0, 1.0);
    let dx = px as f32 + t * pqx - x as f32;
    let dz = pz as f32 + t * pqz - z as f32;
    dx * dx + dz * dz
}

fn simplify(pts: &[[i32; 4]], max_error: f32, max_edge_len: i32) -> Vec<[i32; 4]> {
    let pn = pts.len();
    if pn == 0 {
        return Vec::new();
    }
    // `[x, y, z, raw index]`
    let mut s: Vec<[i32; 4]> = Vec::new();
    let has_connections = pts.iter().any(|p| p[3] & REG_MASK != 0);
    if has_connections {
        for i in 0..pn {
            let ii = (i + 1) % pn;
            let different_regs = pts[i][3] & REG_MASK != pts[ii][3] & REG_MASK;
            let area_borders = pts[i][3] & AREA_BORDER != pts[ii][3] & AREA_BORDER;
            if different_regs || area_borders {
                s.push([pts[i][0], pts[i][1], pts[i][2], i as i32]);
            }
        }
    }
    if s.is_empty() {
        let (mut ll, mut ur) = (0usize, 0usize);
        for (i, p) in pts.iter().enumerate() {
            let (l, u) = (pts[ll], pts[ur]);
            if p[0] < l[0] || (p[0] == l[0] && p[2] < l[2]) {
                ll = i;
            }
            if p[0] > u[0] || (p[0] == u[0] && p[2] > u[2]) {
                ur = i;
            }
        }
        s.push([pts[ll][0], pts[ll][1], pts[ll][2], ll as i32]);
        s.push([pts[ur][0], pts[ur][1], pts[ur][2], ur as i32]);
    }
    // Add the raw points furthest from each simplified segment until all are within error.
    let mut i = 0;
    while i < s.len() {
        let ii = (i + 1) % s.len();
        let (mut ax, mut az, ai) = (s[i][0], s[i][2], s[i][3] as usize);
        let (mut bx, mut bz, bi) = (s[ii][0], s[ii][2], s[ii][3] as usize);
        let (mut ci, cinc, endi);
        if bx > ax || (bx == ax && bz > az) {
            cinc = 1;
            ci = (ai + cinc) % pn;
            endi = bi;
        } else {
            cinc = pn - 1;
            ci = (bi + cinc) % pn;
            endi = ai;
            std::mem::swap(&mut ax, &mut bx);
            std::mem::swap(&mut az, &mut bz);
        }
        let mut maxd = 0.0;
        let mut maxi: Option<usize> = None;
        if pts[ci][3] & REG_MASK == 0 || pts[ci][3] & AREA_BORDER != 0 {
            while ci != endi {
                let d = dist_pt_seg2(pts[ci][0], pts[ci][2], ax, az, bx, bz);
                if d > maxd {
                    maxd = d;
                    maxi = Some(ci);
                }
                ci = (ci + cinc) % pn;
            }
        }
        match maxi {
            Some(m) if maxd > max_error * max_error => {
                s.insert(i + 1, [pts[m][0], pts[m][1], pts[m][2], m as i32]);
            }
            _ => i += 1,
        }
    }
    // Split wall edges longer than max_edge_len.
    if max_edge_len > 0 {
        let mut i = 0;
        while i < s.len() {
            let ii = (i + 1) % s.len();
            let (ax, az, ai) = (s[i][0], s[i][2], s[i][3] as usize);
            let (bx, bz, bi) = (s[ii][0], s[ii][2], s[ii][3] as usize);
            let ci = (ai + 1) % pn;
            let mut maxi = None;
            if pts[ci][3] & REG_MASK == 0 {
                let dx = bx - ax;
                let dz = bz - az;
                if dx * dx + dz * dz > max_edge_len * max_edge_len {
                    let n = if bi < ai { bi + pn - ai } else { bi - ai };
                    if n > 1 {
                        maxi = Some(if bx > ax || (bx == ax && bz > az) {
                            (ai + n / 2) % pn
                        } else {
                            (ai + n.div_ceil(2)) % pn
                        });
                    }
                }
            }
            match maxi {
                Some(m) => s.insert(i + 1, [pts[m][0], pts[m][1], pts[m][2], m as i32]),
                None => i += 1,
            }
        }
    }
    for v in &mut s {
        // Edge flags come from the next raw point, the border-vertex flag from this one.
        let ai = (v[3] as usize + 1) % pn;
        let bi = v[3] as usize;
        v[3] = (pts[ai][3] & (REG_MASK | AREA_BORDER)) | (pts[bi][3] & BORDER_VERTEX);
    }
    s
}

fn remove_degenerate(s: &mut Vec<[i32; 4]>) {
    let mut i = 0;
    while i < s.len() && s.len() > 1 {
        let ni = (i + 1) % s.len();
        if s[i][0] == s[ni][0] && s[i][2] == s[ni][2] {
            s.remove(ni);
        } else {
            i += 1;
        }
    }
}
