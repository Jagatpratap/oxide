//! Unity ProBuilder's Boolean tool (experimental there too): Union, Subtract and Intersect of
//! two closed meshes with BSP trees (after Evan Wallace's csg.js). Faces keep their material
//! slot, smoothing group and color.
//! <https://docs.unity3d.com/Packages/com.unity.probuilder@6.0/manual/boolean.html>

use crate::pbmesh::{Face, PbMesh};
use oxide_math::{Color, Vector3};
use std::collections::HashMap;

const EPS: f32 = 1e-5;

/// Unity's Boolean operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BooleanOp {
    Union,
    Subtract,
    Intersect,
}

#[derive(Clone, Copy, Debug)]
struct Plane {
    n: Vector3,
    w: f32,
}

impl Plane {
    fn from_points(a: Vector3, b: Vector3, c: Vector3) -> Option<Plane> {
        let n = (b - a).cross(c - a);
        let len = n.length();
        (len > 1e-12).then(|| {
            let n = n / len;
            Plane { n, w: n.dot(a) }
        })
    }

    fn flip(&mut self) {
        self.n = -self.n;
        self.w = -self.w;
    }
}

/// What a polygon came from.
#[derive(Clone, Copy, Debug)]
struct Attr {
    material: u32,
    smoothing_group: u32,
    color: Color,
}

#[derive(Clone, Debug)]
struct Poly {
    v: Vec<Vector3>,
    plane: Plane,
    attr: Attr,
}

impl Poly {
    fn flip(&mut self) {
        self.v.reverse();
        self.plane.flip();
    }
}

const COPLANAR: u8 = 0;
const FRONT: u8 = 1;
const BACK: u8 = 2;
const SPANNING: u8 = 3;

/// Sorts `p` against `plane` into the four lists.
fn split(
    plane: &Plane,
    p: &Poly,
    co_front: &mut Vec<Poly>,
    co_back: &mut Vec<Poly>,
    front: &mut Vec<Poly>,
    back: &mut Vec<Poly>,
) {
    let mut kind = 0u8;
    let types: Vec<u8> =
        p.v.iter()
            .map(|v| {
                let t = plane.n.dot(*v) - plane.w;
                let k = if t < -EPS {
                    BACK
                } else if t > EPS {
                    FRONT
                } else {
                    COPLANAR
                };
                kind |= k;
                k
            })
            .collect();
    match kind {
        COPLANAR => {
            if plane.n.dot(p.plane.n) > 0.0 {
                co_front.push(p.clone())
            } else {
                co_back.push(p.clone())
            }
        }
        FRONT => front.push(p.clone()),
        BACK => back.push(p.clone()),
        _ => {
            let (mut f, mut b) = (Vec::new(), Vec::new());
            let n = p.v.len();
            for i in 0..n {
                let j = (i + 1) % n;
                let (ti, tj) = (types[i], types[j]);
                let (vi, vj) = (p.v[i], p.v[j]);
                if ti != BACK {
                    f.push(vi);
                }
                if ti != FRONT {
                    b.push(vi);
                }
                if (ti | tj) == SPANNING {
                    let t = (plane.w - plane.n.dot(vi)) / plane.n.dot(vj - vi);
                    let v = vi + (vj - vi) * t;
                    f.push(v);
                    b.push(v);
                }
            }
            if f.len() >= 3 {
                front.push(Poly { v: f, ..p.clone() });
            }
            if b.len() >= 3 {
                back.push(Poly { v: b, ..p.clone() });
            }
        }
    }
}

#[derive(Default)]
struct Node {
    plane: Option<Plane>,
    front: Option<Box<Node>>,
    back: Option<Box<Node>>,
    polys: Vec<Poly>,
}

impl Node {
    fn new(polys: Vec<Poly>) -> Node {
        let mut n = Node::default();
        n.build(polys);
        n
    }

    fn invert(&mut self) {
        for p in &mut self.polys {
            p.flip();
        }
        if let Some(pl) = &mut self.plane {
            pl.flip();
        }
        if let Some(f) = &mut self.front {
            f.invert();
        }
        if let Some(b) = &mut self.back {
            b.invert();
        }
        std::mem::swap(&mut self.front, &mut self.back);
    }

    /// The parts of `polys` outside this solid.
    fn clip_polygons(&self, polys: Vec<Poly>) -> Vec<Poly> {
        let Some(plane) = self.plane else {
            return polys;
        };
        let (mut front, mut back) = (Vec::new(), Vec::new());
        for p in &polys {
            let (mut cf, mut cb) = (Vec::new(), Vec::new());
            split(&plane, p, &mut cf, &mut cb, &mut front, &mut back);
            front.extend(cf);
            back.extend(cb);
        }
        let front = match &self.front {
            Some(f) => f.clip_polygons(front),
            None => front,
        };
        let back = match &self.back {
            Some(b) => b.clip_polygons(back),
            None => Vec::new(),
        };
        let mut out = front;
        out.extend(back);
        out
    }

    fn clip_to(&mut self, other: &Node) {
        self.polys = other.clip_polygons(std::mem::take(&mut self.polys));
        if let Some(f) = &mut self.front {
            f.clip_to(other);
        }
        if let Some(b) = &mut self.back {
            b.clip_to(other);
        }
    }

    fn all_polygons(&self) -> Vec<Poly> {
        let mut out = self.polys.clone();
        if let Some(f) = &self.front {
            out.extend(f.all_polygons());
        }
        if let Some(b) = &self.back {
            out.extend(b.all_polygons());
        }
        out
    }

    fn build(&mut self, polys: Vec<Poly>) {
        if polys.is_empty() {
            return;
        }
        let plane = *self.plane.get_or_insert(polys[0].plane);
        let (mut front, mut back) = (Vec::new(), Vec::new());
        for p in &polys {
            let (mut cf, mut cb) = (Vec::new(), Vec::new());
            split(&plane, p, &mut cf, &mut cb, &mut front, &mut back);
            self.polys.extend(cf);
            self.polys.extend(cb);
        }
        if !front.is_empty() {
            self.front.get_or_insert_with(Default::default).build(front);
        }
        if !back.is_empty() {
            self.back.get_or_insert_with(Default::default).build(back);
        }
    }
}

/// Polygons of `m` (positions transformed by `to_world`), material slots offset by
/// `slot_offset`.
fn polygons(m: &PbMesh, to_world: &dyn Fn(Vector3) -> Vector3, slot_offset: u32) -> Vec<Poly> {
    let mut out = Vec::new();
    for f in &m.faces {
        if f.indices.len() < 3 {
            continue;
        }
        // Index order gives the outward normal (right-hand rule), as csg expects.
        let v: Vec<Vector3> = f
            .indices
            .iter()
            .map(|i| to_world(m.positions[*i as usize]))
            .collect();
        let attr = Attr {
            material: f.material + slot_offset,
            smoothing_group: f.smoothing_group,
            color: f.color,
        };
        // Split into a triangle fan so non-planar faces stay valid.
        for k in 1..v.len() - 1 {
            let tri = vec![v[0], v[k], v[k + 1]];
            if let Some(plane) = Plane::from_points(tri[0], tri[1], tri[2]) {
                out.push(Poly {
                    v: tri,
                    plane,
                    attr,
                });
            }
        }
    }
    out
}

/// `a` op `b`, both already in the same space. `b`'s material slots are shifted by
/// `b_slot_offset` so the two objects' materials can be combined.
pub fn boolean(a: &PbMesh, b: &PbMesh, op: BooleanOp, b_slot_offset: u32) -> PbMesh {
    boolean_transformed(a, &|p| p, b, &|p| p, op, b_slot_offset)
}

/// [`boolean`] with each mesh mapped into a shared space first (e.g. world space).
pub fn boolean_transformed(
    a: &PbMesh,
    a_to: &dyn Fn(Vector3) -> Vector3,
    b: &PbMesh,
    b_to: &dyn Fn(Vector3) -> Vector3,
    op: BooleanOp,
    b_slot_offset: u32,
) -> PbMesh {
    let mut na = Node::new(polygons(a, a_to, 0));
    let mut nb = Node::new(polygons(b, b_to, b_slot_offset));
    let polys = match op {
        BooleanOp::Union => {
            na.clip_to(&nb);
            nb.clip_to(&na);
            nb.invert();
            nb.clip_to(&na);
            nb.invert();
            na.build(nb.all_polygons());
            na.all_polygons()
        }
        BooleanOp::Subtract => {
            na.invert();
            na.clip_to(&nb);
            nb.clip_to(&na);
            nb.invert();
            nb.clip_to(&na);
            nb.invert();
            na.build(nb.all_polygons());
            na.invert();
            na.all_polygons()
        }
        BooleanOp::Intersect => {
            na.invert();
            nb.clip_to(&na);
            nb.invert();
            na.clip_to(&nb);
            nb.clip_to(&na);
            na.build(nb.all_polygons());
            na.invert();
            na.all_polygons()
        }
    };
    to_pbmesh(&polys)
}

/// Faces from polygons, with positions welded.
fn to_pbmesh(polys: &[Poly]) -> PbMesh {
    let mut m = PbMesh::default();
    let mut map: HashMap<[i64; 3], u32> = HashMap::new();
    let q = |p: Vector3| {
        [
            (p.x * 1e4).round() as i64,
            (p.y * 1e4).round() as i64,
            (p.z * 1e4).round() as i64,
        ]
    };
    for p in polys {
        let mut idx: Vec<u32> =
            p.v.iter()
                .map(|v| {
                    *map.entry(q(*v)).or_insert_with(|| {
                        m.positions.push(*v);
                        m.positions.len() as u32 - 1
                    })
                })
                .collect();
        idx.dedup();
        if idx.first() == idx.last() && idx.len() > 1 {
            idx.pop();
        }
        if idx.len() < 3 {
            continue;
        }
        let mut f = Face::new(idx);
        f.material = p.attr.material;
        f.smoothing_group = p.attr.smoothing_group;
        f.color = p.attr.color;
        m.faces.push(f);
    }
    m
}

/// The volume enclosed by a closed mesh (for tests and checks).
pub fn volume(m: &PbMesh) -> f32 {
    let mut v = 0.0;
    for f in &m.faces {
        let p: Vec<Vector3> = f.indices.iter().map(|i| m.positions[*i as usize]).collect();
        for k in 1..p.len().saturating_sub(1) {
            // Positive for faces whose index order gives an outward normal.
            v += p[0].dot(p[k].cross(p[k + 1])) / 6.0;
        }
    }
    v
}
