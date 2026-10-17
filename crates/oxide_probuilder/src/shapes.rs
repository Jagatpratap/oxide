//! Unity ProBuilder's shapes (the Shape tool), fitted to a size box centered on the origin.
//! Reference: <https://docs.unity3d.com/Packages/com.unity.probuilder@6.0/manual/shape-reference.html>

use crate::pbmesh::{Face, PbMesh};
use oxide_core::Reflect;
use oxide_math::{Vector2, Vector3};
use std::f32::consts::{PI, TAU};

#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShapeType {
    #[default]
    Cube,
    Plane,
    Cylinder,
    Cone,
    Prism,
    Stairs,
    Arch,
    Pipe,
    Torus,
    Sphere,
    Door,
    /// Unity's Poly Shape: an outline drawn on the ground, extruded up.
    PolyShape,
    /// Unity's Bezier Shape: a tube along a smooth path.
    BezierShape,
}

impl ShapeType {
    pub const ALL: [ShapeType; 13] = [
        ShapeType::Cube,
        ShapeType::Plane,
        ShapeType::Cylinder,
        ShapeType::Cone,
        ShapeType::Prism,
        ShapeType::Stairs,
        ShapeType::Arch,
        ShapeType::Pipe,
        ShapeType::Torus,
        ShapeType::Sphere,
        ShapeType::Door,
        ShapeType::PolyShape,
        ShapeType::BezierShape,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ShapeType::Cube => "Cube",
            ShapeType::Plane => "Plane",
            ShapeType::Cylinder => "Cylinder",
            ShapeType::Cone => "Cone",
            ShapeType::Prism => "Prism",
            ShapeType::Stairs => "Stairs",
            ShapeType::Arch => "Arch",
            ShapeType::Pipe => "Pipe",
            ShapeType::Torus => "Torus",
            ShapeType::Sphere => "Sphere",
            ShapeType::Door => "Door",
            ShapeType::PolyShape => "Poly Shape",
            ShapeType::BezierShape => "Bezier Shape",
        }
    }
}

/// Shape settings (Unity's per-shape properties, shared where they mean the same).
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct ShapeParams {
    pub shape: ShapeType,
    /// The bounding box the shape fills.
    pub size: Vector3,
    /// Sides around (Cylinder, Cone, Pipe, Arch, Torus columns).
    pub sides: u32,
    /// Height segments, stairs steps, plane subdivisions, torus rows.
    pub segments: u32,
    /// Wall thickness (Pipe, Arch, Door frame), as a fraction of the radius or width.
    pub thickness: f32,
    /// Arch angle in degrees.
    pub angle: f32,
    /// Icosphere subdivisions.
    pub subdivisions: u32,
    /// Smooth shading around curved shapes.
    pub smooth: bool,
    /// Poly Shape outline (X and Z; Y ignored) or Bezier Shape path points.
    pub points: Vec<Vector3>,
    /// Bezier Shape: the path loops back to its start.
    pub closed: bool,
    /// Poly Shape: faces point inward (rooms seen from inside).
    pub flip_normals: bool,
}

impl Default for ShapeParams {
    fn default() -> Self {
        Self {
            shape: ShapeType::Cube,
            size: Vector3::ONE,
            sides: 16,
            segments: 1,
            thickness: 0.25,
            angle: 180.0,
            subdivisions: 2,
            smooth: true,
            points: Vec::new(),
            closed: false,
            flip_normals: false,
        }
    }
}

impl ShapeParams {
    /// Unity's starting settings for a shape.
    pub fn new(shape: ShapeType) -> Self {
        let mut p = Self {
            shape,
            ..Default::default()
        };
        match shape {
            ShapeType::Plane => {
                p.size = Vector3::new(4.0, 0.0, 4.0);
                p.segments = 2;
            }
            ShapeType::Stairs => {
                p.size = Vector3::new(2.0, 2.5, 4.0);
                p.segments = 6;
            }
            ShapeType::Arch => {
                p.size = Vector3::new(4.0, 2.0, 1.0);
                p.sides = 12;
            }
            ShapeType::Pipe => {
                p.size = Vector3::new(2.0, 2.0, 2.0);
                p.sides = 16;
            }
            ShapeType::Torus => {
                p.size = Vector3::new(2.0, 0.5, 2.0);
                p.sides = 24;
                p.segments = 12;
            }
            ShapeType::Door => {
                p.size = Vector3::new(4.0, 4.0, 1.0);
            }
            ShapeType::Cylinder | ShapeType::Cone => {
                p.size = Vector3::new(1.0, 2.0, 1.0);
            }
            ShapeType::PolyShape => {
                p.size = Vector3::new(1.0, 1.0, 1.0);
                p.points = vec![
                    Vector3::new(-1.0, 0.0, -1.0),
                    Vector3::new(-1.0, 0.0, 1.0),
                    Vector3::new(1.0, 0.0, 1.0),
                    Vector3::new(1.0, 0.0, -1.0),
                ];
            }
            ShapeType::BezierShape => {
                p.size = Vector3::new(0.5, 0.5, 0.5);
                p.sides = 8;
                p.segments = 8;
                p.points = vec![
                    Vector3::new(-2.0, 0.0, 0.0),
                    Vector3::new(-0.7, 0.0, 1.0),
                    Vector3::new(0.7, 0.0, -1.0),
                    Vector3::new(2.0, 0.0, 0.0),
                ];
            }
            _ => {}
        }
        p
    }

    pub fn build(&self) -> PbMesh {
        let s = self.size;
        let sides = self.sides.clamp(3, 128);
        let segs = self.segments.clamp(1, 64);
        match self.shape {
            ShapeType::Cube => cube(s),
            ShapeType::Plane => plane(s.x, s.z, segs, segs),
            ShapeType::Cylinder => cylinder(s, sides, segs, self.smooth),
            ShapeType::Cone => cone(s, sides, self.smooth),
            ShapeType::Prism => prism(s),
            ShapeType::Stairs => stairs(s, segs),
            ShapeType::Arch => arch(s, sides, self.thickness, self.angle),
            ShapeType::Pipe => pipe(s, sides, self.thickness, self.smooth),
            ShapeType::Torus => torus(s, sides, segs.max(3), self.smooth),
            ShapeType::Sphere => sphere(s, self.subdivisions.min(5), self.smooth),
            ShapeType::Door => door(s, self.thickness),
            ShapeType::PolyShape => poly_shape(&self.points, s.y, self.flip_normals),
            ShapeType::BezierShape => bezier_shape(
                &self.points,
                s.x * 0.5,
                sides.min(64),
                segs,
                self.closed,
                self.smooth,
            ),
        }
    }
}

/// Builds faces with the right winding from an outward direction.
struct Builder {
    m: PbMesh,
}

impl Builder {
    fn new() -> Self {
        Self {
            m: PbMesh::default(),
        }
    }

    fn v(&mut self, p: Vector3) -> u32 {
        self.m.positions.push(p);
        self.m.positions.len() as u32 - 1
    }

    /// Adds a face whose normal points along `out` (reversed if needed).
    fn face(&mut self, mut idx: Vec<u32>, out: Vector3, smooth: u32) {
        let p: Vec<Vector3> = idx.iter().map(|i| self.m.positions[*i as usize]).collect();
        let mut n = Vector3::ZERO;
        for k in 1..p.len().saturating_sub(1) {
            n += (p[k] - p[0]).cross(p[k + 1] - p[0]);
        }
        if n.dot(out) < 0.0 {
            idx.reverse();
        }
        let mut f = Face::new(idx);
        f.smoothing_group = smooth;
        self.m.faces.push(f);
    }

    fn done(self) -> PbMesh {
        self.m
    }
}

pub fn cube(s: Vector3) -> PbMesh {
    let mut b = Builder::new();
    let h = s * 0.5;
    let mut c = Vec::new();
    for k in 0..8u32 {
        c.push(b.v(Vector3::new(
            if k & 1 == 0 { -h.x } else { h.x },
            if k & 2 == 0 { -h.y } else { h.y },
            if k & 4 == 0 { -h.z } else { h.z },
        )));
    }
    let quads = [
        ([0, 2, 6, 4], Vector3::NEG_X),
        ([1, 5, 7, 3], Vector3::X),
        ([0, 4, 5, 1], Vector3::NEG_Y),
        ([2, 3, 7, 6], Vector3::Y),
        ([0, 1, 3, 2], Vector3::NEG_Z),
        ([4, 6, 7, 5], Vector3::Z),
    ];
    for (q, n) in quads {
        b.face(q.iter().map(|i| c[*i]).collect(), n, 0);
    }
    b.done()
}

pub fn plane(w: f32, d: f32, ws: u32, ds: u32) -> PbMesh {
    let mut b = Builder::new();
    let mut grid = Vec::new();
    for j in 0..=ds {
        for i in 0..=ws {
            grid.push(b.v(Vector3::new(
                -w * 0.5 + w * i as f32 / ws as f32,
                0.0,
                -d * 0.5 + d * j as f32 / ds as f32,
            )));
        }
    }
    let at = |i: u32, j: u32| grid[(j * (ws + 1) + i) as usize];
    for j in 0..ds {
        for i in 0..ws {
            b.face(
                vec![at(i, j), at(i, j + 1), at(i + 1, j + 1), at(i + 1, j)],
                Vector3::Y,
                0,
            );
        }
    }
    b.done()
}

fn ring(b: &mut Builder, rx: f32, rz: f32, y: f32, sides: u32) -> Vec<u32> {
    (0..sides)
        .map(|k| {
            let a = TAU * k as f32 / sides as f32;
            b.v(Vector3::new(a.cos() * rx, y, a.sin() * rz))
        })
        .collect()
}

pub fn cylinder(s: Vector3, sides: u32, segs: u32, smooth: bool) -> PbMesh {
    let mut b = Builder::new();
    let rings: Vec<Vec<u32>> = (0..=segs)
        .map(|j| {
            ring(
                &mut b,
                s.x * 0.5,
                s.z * 0.5,
                -s.y * 0.5 + s.y * j as f32 / segs as f32,
                sides,
            )
        })
        .collect();
    let g = smooth as u32;
    for j in 0..segs as usize {
        for k in 0..sides as usize {
            let k2 = (k + 1) % sides as usize;
            let mid =
                (b.m.positions[rings[j][k] as usize] + b.m.positions[rings[j][k2] as usize]) * 0.5;
            let out = Vector3::new(mid.x, 0.0, mid.z);
            b.face(
                vec![rings[j][k], rings[j + 1][k], rings[j + 1][k2], rings[j][k2]],
                out,
                g,
            );
        }
    }
    b.face(rings[0].clone(), Vector3::NEG_Y, 0);
    b.face(rings[segs as usize].clone(), Vector3::Y, 0);
    b.done()
}

pub fn cone(s: Vector3, sides: u32, smooth: bool) -> PbMesh {
    let mut b = Builder::new();
    let base = ring(&mut b, s.x * 0.5, s.z * 0.5, -s.y * 0.5, sides);
    let apex = b.v(Vector3::new(0.0, s.y * 0.5, 0.0));
    for k in 0..sides as usize {
        let k2 = (k + 1) % sides as usize;
        let mid = (b.m.positions[base[k] as usize] + b.m.positions[base[k2] as usize]) * 0.5;
        b.face(
            vec![base[k], apex, base[k2]],
            Vector3::new(mid.x, 0.0, mid.z),
            smooth as u32,
        );
    }
    b.face(base, Vector3::NEG_Y, 0);
    b.done()
}

pub fn prism(s: Vector3) -> PbMesh {
    let mut b = Builder::new();
    let h = s * 0.5;
    let tri = |b: &mut Builder, z: f32| {
        [
            b.v(Vector3::new(-h.x, -h.y, z)),
            b.v(Vector3::new(0.0, h.y, z)),
            b.v(Vector3::new(h.x, -h.y, z)),
        ]
    };
    let f = tri(&mut b, -h.z);
    let k = tri(&mut b, h.z);
    b.face(f.to_vec(), Vector3::NEG_Z, 0);
    b.face(k.to_vec(), Vector3::Z, 0);
    for i in 0..3 {
        let j = (i + 1) % 3;
        let mid = (b.m.positions[f[i] as usize] + b.m.positions[f[j] as usize]) * 0.5;
        b.face(
            vec![f[i], f[j], k[j], k[i]],
            Vector3::new(mid.x, mid.y + h.y * 0.33, 0.0),
            0,
        );
    }
    b.done()
}

/// Extrudes a closed profile in the YZ plane (points are (z, y)) across X.
fn extrude_profile(b: &mut Builder, profile: &[Vector2], width: f32) {
    let n = profile.len();
    let area: f32 = (0..n)
        .map(|i| {
            let (a, c) = (profile[i], profile[(i + 1) % n]);
            a.x * c.y - c.x * a.y
        })
        .sum();
    let left: Vec<u32> = profile
        .iter()
        .map(|p| b.v(Vector3::new(-width * 0.5, p.y, p.x)))
        .collect();
    let right: Vec<u32> = profile
        .iter()
        .map(|p| b.v(Vector3::new(width * 0.5, p.y, p.x)))
        .collect();
    b.face(left.clone(), Vector3::NEG_X, 0);
    b.face(right.clone(), Vector3::X, 0);
    for i in 0..n {
        let j = (i + 1) % n;
        let e = profile[j] - profile[i];
        // Outward normal of the edge in (z, y) for the polygon's orientation.
        let nrm = if area > 0.0 {
            Vector2::new(e.y, -e.x)
        } else {
            Vector2::new(-e.y, e.x)
        };
        b.face(
            vec![left[i], left[j], right[j], right[i]],
            Vector3::new(0.0, nrm.y, nrm.x),
            0,
        );
    }
}

pub fn stairs(s: Vector3, steps: u32) -> PbMesh {
    let mut b = Builder::new();
    let (z0, y0) = (-s.z * 0.5, -s.y * 0.5);
    let (sd, sh) = (s.z / steps as f32, s.y / steps as f32);
    let mut p = vec![
        Vector2::new(z0, y0),
        Vector2::new(z0 + s.z, y0),
        Vector2::new(z0 + s.z, y0 + s.y),
    ];
    for i in (0..steps).rev() {
        p.push(Vector2::new(z0 + i as f32 * sd, y0 + (i + 1) as f32 * sh));
        if i > 0 {
            p.push(Vector2::new(z0 + i as f32 * sd, y0 + i as f32 * sh));
        }
    }
    extrude_profile(&mut b, &p, s.x);
    b.done()
}

pub fn arch(s: Vector3, sides: u32, thickness: f32, angle: f32) -> PbMesh {
    let mut b = Builder::new();
    let (rx, ry) = (s.x * 0.5, s.y);
    let t = thickness.clamp(0.01, 0.95);
    let sweep = angle.clamp(1.0, 360.0).to_radians();
    let start = (PI - sweep) * 0.5;
    let n = sides as usize;
    let mut outer = [Vec::new(), Vec::new()];
    let mut inner = [Vec::new(), Vec::new()];
    for (side, z) in [-s.z * 0.5, s.z * 0.5].into_iter().enumerate() {
        for k in 0..=n {
            let a = start + sweep * k as f32 / n as f32;
            let (c, sn) = (a.cos(), a.sin());
            outer[side].push(b.v(Vector3::new(c * rx, sn * ry - s.y * 0.5, z)));
            inner[side].push(b.v(Vector3::new(
                c * rx * (1.0 - t),
                sn * ry * (1.0 - t) - s.y * 0.5,
                z,
            )));
        }
    }
    for k in 0..n {
        let a = start + sweep * (k as f32 + 0.5) / n as f32;
        let radial = Vector3::new(a.cos(), a.sin(), 0.0);
        b.face(
            vec![outer[0][k], outer[0][k + 1], inner[0][k + 1], inner[0][k]],
            Vector3::NEG_Z,
            0,
        );
        b.face(
            vec![outer[1][k], outer[1][k + 1], inner[1][k + 1], inner[1][k]],
            Vector3::Z,
            0,
        );
        b.face(
            vec![outer[0][k], outer[0][k + 1], outer[1][k + 1], outer[1][k]],
            radial,
            1,
        );
        b.face(
            vec![inner[0][k], inner[0][k + 1], inner[1][k + 1], inner[1][k]],
            -radial,
            2,
        );
    }
    if angle < 360.0 {
        for (k, dir) in [(0, -1.0f32), (n, 1.0)] {
            let a = start + sweep * (k as f32 / n as f32);
            let tangent = Vector3::new(-a.sin(), a.cos(), 0.0) * dir;
            b.face(
                vec![outer[0][k], inner[0][k], inner[1][k], outer[1][k]],
                tangent,
                0,
            );
        }
    }
    b.done()
}

pub fn pipe(s: Vector3, sides: u32, thickness: f32, smooth: bool) -> PbMesh {
    let mut b = Builder::new();
    let t = thickness.clamp(0.01, 0.95);
    let (y0, y1) = (-s.y * 0.5, s.y * 0.5);
    let ob = ring(&mut b, s.x * 0.5, s.z * 0.5, y0, sides);
    let ot = ring(&mut b, s.x * 0.5, s.z * 0.5, y1, sides);
    let ib = ring(
        &mut b,
        s.x * 0.5 * (1.0 - t),
        s.z * 0.5 * (1.0 - t),
        y0,
        sides,
    );
    let it = ring(
        &mut b,
        s.x * 0.5 * (1.0 - t),
        s.z * 0.5 * (1.0 - t),
        y1,
        sides,
    );
    let n = sides as usize;
    for k in 0..n {
        let k2 = (k + 1) % n;
        let mid = (b.m.positions[ob[k] as usize] + b.m.positions[ob[k2] as usize]) * 0.5;
        let radial = Vector3::new(mid.x, 0.0, mid.z);
        b.face(vec![ob[k], ot[k], ot[k2], ob[k2]], radial, smooth as u32);
        b.face(
            vec![ib[k], it[k], it[k2], ib[k2]],
            -radial,
            2 * smooth as u32,
        );
        b.face(vec![ot[k], it[k], it[k2], ot[k2]], Vector3::Y, 0);
        b.face(vec![ob[k], ib[k], ib[k2], ob[k2]], Vector3::NEG_Y, 0);
    }
    b.done()
}

pub fn torus(s: Vector3, columns: u32, rows: u32, smooth: bool) -> PbMesh {
    let mut b = Builder::new();
    let tube = s.y * 0.5;
    let (rx, rz) = ((s.x * 0.5 - tube).max(0.01), (s.z * 0.5 - tube).max(0.01));
    let mut grid = vec![Vec::new(); columns as usize];
    for (c, col) in grid.iter_mut().enumerate() {
        let a = TAU * c as f32 / columns as f32;
        let center = Vector3::new(a.cos() * rx, 0.0, a.sin() * rz);
        let dir = Vector3::new(a.cos(), 0.0, a.sin());
        for r in 0..rows {
            let t = TAU * r as f32 / rows as f32;
            col.push(b.v(center + dir * (t.cos() * tube) + Vector3::Y * (t.sin() * tube)));
        }
    }
    for c in 0..columns as usize {
        let c2 = (c + 1) % columns as usize;
        for r in 0..rows as usize {
            let r2 = (r + 1) % rows as usize;
            let q = vec![grid[c][r], grid[c2][r], grid[c2][r2], grid[c][r2]];
            let center: Vector3 = q
                .iter()
                .map(|i| b.m.positions[*i as usize])
                .sum::<Vector3>()
                / 4.0;
            let a = TAU * (c as f32 + 0.5) / columns as f32;
            let ring_center = Vector3::new(a.cos() * rx, 0.0, a.sin() * rz);
            b.face(q, center - ring_center, smooth as u32);
        }
    }
    b.done()
}

pub fn sphere(s: Vector3, subdivisions: u32, smooth: bool) -> PbMesh {
    // Icosahedron.
    let t = (1.0 + 5.0f32.sqrt()) / 2.0;
    let mut pos: Vec<Vector3> = [
        (-1.0, t, 0.0),
        (1.0, t, 0.0),
        (-1.0, -t, 0.0),
        (1.0, -t, 0.0),
        (0.0, -1.0, t),
        (0.0, 1.0, t),
        (0.0, -1.0, -t),
        (0.0, 1.0, -t),
        (t, 0.0, -1.0),
        (t, 0.0, 1.0),
        (-t, 0.0, -1.0),
        (-t, 0.0, 1.0),
    ]
    .iter()
    .map(|(x, y, z)| Vector3::new(*x, *y, *z).normalize())
    .collect();
    let mut tris: Vec<[u32; 3]> = vec![
        [0, 11, 5],
        [0, 5, 1],
        [0, 1, 7],
        [0, 7, 10],
        [0, 10, 11],
        [1, 5, 9],
        [5, 11, 4],
        [11, 10, 2],
        [10, 7, 6],
        [7, 1, 8],
        [3, 9, 4],
        [3, 4, 2],
        [3, 2, 6],
        [3, 6, 8],
        [3, 8, 9],
        [4, 9, 5],
        [2, 4, 11],
        [6, 2, 10],
        [8, 6, 7],
        [9, 8, 1],
    ];
    for _ in 0..subdivisions {
        let mut mid = std::collections::HashMap::new();
        let mut m = |a: u32, b: u32, pos: &mut Vec<Vector3>| -> u32 {
            *mid.entry(crate::pbmesh::edge_key(a, b)).or_insert_with(|| {
                pos.push(((pos[a as usize] + pos[b as usize]) * 0.5).normalize());
                pos.len() as u32 - 1
            })
        };
        let mut next = Vec::with_capacity(tris.len() * 4);
        for [a, b, c] in tris {
            let (ab, bc, ca) = (m(a, b, &mut pos), m(b, c, &mut pos), m(c, a, &mut pos));
            next.extend([[a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]]);
        }
        tris = next;
    }
    let mut b = Builder::new();
    for p in &pos {
        b.v(*p * s * 0.5);
    }
    for [x, y, z] in tris {
        let c = (pos[x as usize] + pos[y as usize] + pos[z as usize]) / 3.0;
        b.face(vec![x, y, z], c, smooth as u32);
    }
    b.done()
}

pub fn door(s: Vector3, thickness: f32) -> PbMesh {
    let mut b = Builder::new();
    let h = s * 0.5;
    let post = (s.x * thickness.clamp(0.05, 0.45)).max(0.01);
    let lintel = (s.y * thickness.clamp(0.05, 0.45)).max(0.01);
    let (xi, yi) = (h.x - post, h.y - lintel);
    // Three boxes: left post, right post, lintel (Unity's door frame).
    let boxes = [
        (Vector3::new(-h.x, -h.y, -h.z), Vector3::new(-xi, yi, h.z)),
        (Vector3::new(xi, -h.y, -h.z), Vector3::new(h.x, yi, h.z)),
        (Vector3::new(-h.x, yi, -h.z), Vector3::new(h.x, h.y, h.z)),
    ];
    for (lo, hi) in boxes {
        let size = hi - lo;
        let mut c = cube(size);
        for p in &mut c.positions {
            *p += (lo + hi) * 0.5;
        }
        let base = b.m.positions.len() as u32;
        b.m.positions.extend(c.positions);
        for mut f in c.faces {
            for i in &mut f.indices {
                *i += base;
            }
            b.m.faces.push(f);
        }
    }
    // Posts and lintel touch: weld their shared corners.
    b.m.weld_vertices(&[], 1e-4);
    b.done()
}

/// Unity's Poly Shape: the outline (X/Z, any winding, may be concave) as a floor face, and
/// with a height, a top and side walls too.
pub fn poly_shape(points: &[Vector3], height: f32, flip: bool) -> PbMesh {
    let pts: Vec<Vector3> = points.iter().map(|p| Vector3::new(p.x, 0.0, p.z)).collect();
    let n = pts.len();
    if n < 3 {
        return PbMesh::default();
    }
    // Counter-clockwise seen from above has positive area.
    let area: f32 = (0..n)
        .map(|i| {
            let (a, c) = (pts[i], pts[(i + 1) % n]);
            a.x * c.z - c.x * a.z
        })
        .sum();
    let mut b = Builder::new();
    let bottom: Vec<u32> = pts.iter().map(|p| b.v(*p)).collect();
    if height.abs() < 1e-5 {
        b.face(bottom, Vector3::Y, 0);
    } else {
        let top: Vec<u32> = pts.iter().map(|p| b.v(*p + Vector3::Y * height)).collect();
        b.face(top.clone(), Vector3::Y * height.signum(), 0);
        b.face(bottom.clone(), -Vector3::Y * height.signum(), 0);
        for i in 0..n {
            let j = (i + 1) % n;
            let edge = pts[j] - pts[i];
            // Outward: right of the edge when the outline runs counter-clockwise from above.
            let mut out = Vector3::new(edge.z, 0.0, -edge.x);
            if area < 0.0 {
                out = -out;
            }
            b.face(vec![bottom[i], bottom[j], top[j], top[i]], out, 0);
        }
    }
    let mut m = b.done();
    if flip {
        for f in &mut m.faces {
            f.indices.reverse();
        }
    }
    m
}

/// Catmull-Rom point on the path segment `i` at `t` (end points repeated, or wrapped when
/// closed).
fn catmull(p: &[Vector3], i: usize, t: f32, closed: bool) -> Vector3 {
    let n = p.len() as isize;
    let at = |k: isize| {
        if closed {
            p[k.rem_euclid(n) as usize]
        } else {
            p[k.clamp(0, n - 1) as usize]
        }
    };
    let i = i as isize;
    let (p0, p1, p2, p3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
    let (t2, t3) = (t * t, t * t * t);
    (p1 * 2.0
        + (p2 - p0) * t
        + (p0 * 2.0 - p1 * 5.0 + p2 * 4.0 - p3) * t2
        + (p1 * 3.0 - p0 - p2 * 3.0 + p3) * t3)
        * 0.5
}

/// Unity's Bezier Shape: a tube of `radius` with `columns` sides along a smooth path
/// through `points`, `rows` rings per segment, capped unless closed.
pub fn bezier_shape(
    points: &[Vector3],
    radius: f32,
    columns: u32,
    rows: u32,
    closed: bool,
    smooth: bool,
) -> PbMesh {
    if points.len() < 2 {
        return PbMesh::default();
    }
    let spans = if closed {
        points.len()
    } else {
        points.len() - 1
    };
    let mut path = Vec::new();
    for s in 0..spans {
        for r in 0..rows {
            path.push(catmull(points, s, r as f32 / rows as f32, closed));
        }
    }
    if !closed {
        path.push(*points.last().expect("2+ points"));
    }
    let count = path.len();
    let tangent = |k: usize| {
        let (a, c) = if closed {
            (path[(k + count - 1) % count], path[(k + 1) % count])
        } else {
            (path[k.saturating_sub(1)], path[(k + 1).min(count - 1)])
        };
        (c - a).normalize_or(Vector3::X)
    };
    // Parallel transport keeps the rings from twisting.
    let t0 = tangent(0);
    let mut normal = t0.cross(if t0.y.abs() < 0.9 {
        Vector3::Y
    } else {
        Vector3::X
    });
    normal = normal.normalize_or(Vector3::Z);
    let mut b = Builder::new();
    let mut rings: Vec<Vec<u32>> = Vec::with_capacity(count);
    let mut prev_t = t0;
    for (k, &center) in path.iter().enumerate() {
        let t = tangent(k);
        normal = (normal - t * normal.dot(t)).normalize_or(normal);
        if prev_t.dot(t) < -0.99 {
            normal = -normal;
        }
        prev_t = t;
        let binormal = t.cross(normal);
        let ring: Vec<u32> = (0..columns)
            .map(|c| {
                let a = TAU * c as f32 / columns as f32;
                b.v(center + (normal * a.cos() + binormal * a.sin()) * radius)
            })
            .collect();
        rings.push(ring);
    }
    let group = u32::from(smooth);
    let links = if closed { count } else { count - 1 };
    for k in 0..links {
        let (r0, r1) = (&rings[k], &rings[(k + 1) % count]);
        let center = (path[k] + path[(k + 1) % count]) * 0.5;
        for c in 0..columns as usize {
            let d = (c + 1) % columns as usize;
            let q = vec![r0[c], r0[d], r1[d], r1[c]];
            let mid = q
                .iter()
                .map(|i| b.m.positions[*i as usize])
                .sum::<Vector3>()
                / 4.0;
            b.face(q, mid - center, group);
        }
    }
    if !closed {
        b.face(rings[0].clone(), -tangent(0), 0);
        b.face(rings[count - 1].clone(), tangent(count - 1), 0);
    }
    b.done()
}
