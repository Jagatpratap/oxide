//! Unity's built-in primitives with Unity's sizes and orientation.
//! Reference: <https://docs.unity3d.com/Manual/PrimitiveObjects.html>

use crate::mesh::Mesh;
use oxide_math::{Vector2, Vector3};
use std::f32::consts::{PI, TAU};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrimitiveType {
    Sphere,
    Capsule,
    Cylinder,
    Cube,
    Plane,
    Quad,
}

impl PrimitiveType {
    pub const ALL: [PrimitiveType; 6] = [
        PrimitiveType::Sphere,
        PrimitiveType::Capsule,
        PrimitiveType::Cylinder,
        PrimitiveType::Cube,
        PrimitiveType::Plane,
        PrimitiveType::Quad,
    ];

    pub fn name(self) -> &'static str {
        match self {
            PrimitiveType::Sphere => "Sphere",
            PrimitiveType::Capsule => "Capsule",
            PrimitiveType::Cylinder => "Cylinder",
            PrimitiveType::Cube => "Cube",
            PrimitiveType::Plane => "Plane",
            PrimitiveType::Quad => "Quad",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|p| p.name().eq_ignore_ascii_case(s))
    }

    /// Asset path of the shared built-in mesh.
    pub fn mesh_path(self) -> String {
        format!("builtin:{}", self.name())
    }

    pub fn mesh(self) -> Mesh {
        match self {
            PrimitiveType::Sphere => sphere(0.5, 32, 16),
            PrimitiveType::Capsule => capsule(0.5, 2.0, 32, 8),
            PrimitiveType::Cylinder => cylinder(0.5, 2.0, 32),
            PrimitiveType::Cube => cube(),
            PrimitiveType::Plane => plane(10.0, 10),
            PrimitiveType::Quad => quad(),
        }
    }
}

struct Builder {
    v: Vec<Vector3>,
    n: Vec<Vector3>,
    uv: Vec<Vector2>,
    i: Vec<u32>,
}

impl Builder {
    fn new() -> Self {
        Self {
            v: vec![],
            n: vec![],
            uv: vec![],
            i: vec![],
        }
    }
    fn vert(&mut self, p: Vector3, n: Vector3, uv: Vector2) -> u32 {
        self.v.push(p);
        self.n.push(n);
        self.uv.push(uv);
        self.v.len() as u32 - 1
    }
    /// Quad `a b c d` going clockwise as seen from the front.
    fn quad(&mut self, a: u32, b: u32, c: u32, d: u32) {
        self.i.extend_from_slice(&[a, b, c, a, c, d]);
    }
    fn build(self, name: &str) -> Mesh {
        let mut m = Mesh {
            name: name.into(),
            vertices: self.v,
            normals: self.n,
            uv: self.uv,
            ..Default::default()
        };
        m.submeshes = vec![0..self.i.len() as u32];
        m.indices = self.i;
        m.finish();
        m
    }
}

/// Unit cube centered on the origin, 4 vertices per face.
pub fn cube() -> Mesh {
    let mut b = Builder::new();
    // (normal, right, up): looking at the face from outside, `right` points right.
    let faces = [
        (Vector3::NEG_Z, Vector3::X, Vector3::Y),
        (Vector3::Z, Vector3::NEG_X, Vector3::Y),
        (Vector3::X, Vector3::Z, Vector3::Y),
        (Vector3::NEG_X, Vector3::NEG_Z, Vector3::Y),
        (Vector3::Y, Vector3::X, Vector3::Z),
        (Vector3::NEG_Y, Vector3::X, Vector3::NEG_Z),
    ];
    for (n, r, u) in faces {
        let c = n * 0.5;
        let bl = b.vert(c - r * 0.5 - u * 0.5, n, Vector2::new(0.0, 0.0));
        let tl = b.vert(c - r * 0.5 + u * 0.5, n, Vector2::new(0.0, 1.0));
        let tr = b.vert(c + r * 0.5 + u * 0.5, n, Vector2::new(1.0, 1.0));
        let br = b.vert(c + r * 0.5 - u * 0.5, n, Vector2::new(1.0, 0.0));
        b.quad(bl, tl, tr, br);
    }
    b.build("Cube")
}

/// 1×1 quad in the XY plane facing -Z (towards a default camera).
pub fn quad() -> Mesh {
    let mut b = Builder::new();
    let n = Vector3::NEG_Z;
    let bl = b.vert(Vector3::new(-0.5, -0.5, 0.0), n, Vector2::new(0.0, 0.0));
    let tl = b.vert(Vector3::new(-0.5, 0.5, 0.0), n, Vector2::new(0.0, 1.0));
    let tr = b.vert(Vector3::new(0.5, 0.5, 0.0), n, Vector2::new(1.0, 1.0));
    let br = b.vert(Vector3::new(0.5, -0.5, 0.0), n, Vector2::new(1.0, 0.0));
    b.quad(bl, tl, tr, br);
    b.build("Quad")
}

/// `size`×`size` plane in XZ facing +Y, `cells`×`cells` quads.
pub fn plane(size: f32, cells: u32) -> Mesh {
    grid("Plane", size, size, cells, cells, |_, _| 0.0)
}

/// A grid in XZ facing +Y with per-vertex height `h(x, z)` (also used for terrain chunks).
pub fn grid(
    name: &str,
    size_x: f32,
    size_z: f32,
    cells_x: u32,
    cells_z: u32,
    h: impl Fn(f32, f32) -> f32,
) -> Mesh {
    let mut b = Builder::new();
    for z in 0..=cells_z {
        for x in 0..=cells_x {
            let u = x as f32 / cells_x as f32;
            let v = z as f32 / cells_z as f32;
            let px = (u - 0.5) * size_x;
            let pz = (v - 0.5) * size_z;
            b.vert(
                Vector3::new(px, h(px, pz), pz),
                Vector3::Y,
                Vector2::new(u, v),
            );
        }
    }
    let row = cells_x + 1;
    for z in 0..cells_z {
        for x in 0..cells_x {
            let bl = z * row + x;
            b.quad(bl, bl + row, bl + row + 1, bl + 1);
        }
    }
    let mut m = b.build(name);
    m.recalculate_normals();
    m.recalculate_tangents();
    m
}

/// UV sphere of `radius`.
pub fn sphere(radius: f32, segments: u32, rings: u32) -> Mesh {
    let mut b = Builder::new();
    for r in 0..=rings {
        let v = r as f32 / rings as f32;
        let phi = v * PI; // 0 at the south pole
        for s in 0..=segments {
            let u = s as f32 / segments as f32;
            let theta = u * TAU;
            let n = Vector3::new(
                -phi.sin() * theta.cos(),
                -phi.cos(),
                -phi.sin() * theta.sin(),
            );
            b.vert(n * radius, n, Vector2::new(u, v));
        }
    }
    let row = segments + 1;
    for r in 0..rings {
        for s in 0..segments {
            let bl = r * row + s;
            b.quad(bl, bl + row, bl + row + 1, bl + 1);
        }
    }
    b.build("Sphere")
}

/// Capped cylinder along Y from `-height/2` to `height/2`.
pub fn cylinder(radius: f32, height: f32, segments: u32) -> Mesh {
    let mut b = Builder::new();
    let h = height * 0.5;
    let ring = |a: f32| Vector3::new(-a.cos(), 0.0, -a.sin());
    for s in 0..=segments {
        let u = s as f32 / segments as f32;
        let d = ring(u * TAU);
        b.vert(d * radius - Vector3::Y * h, d, Vector2::new(u, 0.0));
        b.vert(d * radius + Vector3::Y * h, d, Vector2::new(u, 1.0));
    }
    for s in 0..segments {
        let (bl, tl) = (s * 2, s * 2 + 1);
        b.quad(bl, tl, tl + 2, bl + 2);
    }
    for (y, n) in [(h, Vector3::Y), (-h, Vector3::NEG_Y)] {
        let c = b.vert(Vector3::Y * y, n, Vector2::splat(0.5));
        let first = b.v.len() as u32;
        for s in 0..=segments {
            let d = ring(s as f32 / segments as f32 * TAU);
            b.vert(
                d * radius + Vector3::Y * y,
                n,
                Vector2::new(0.5 + d.x * 0.5, 0.5 + d.z * 0.5),
            );
        }
        for s in 0..segments {
            let (a, bb) = (first + s, first + s + 1);
            if y > 0.0 {
                b.i.extend_from_slice(&[c, bb, a]);
            } else {
                b.i.extend_from_slice(&[c, a, bb]);
            }
        }
    }
    b.build("Cylinder")
}

/// Capsule along Y, total `height` including the hemispherical caps.
pub fn capsule(radius: f32, height: f32, segments: u32, cap_rings: u32) -> Mesh {
    let mut b = Builder::new();
    let half = (height * 0.5 - radius).max(0.0);
    let rings = cap_rings * 2 + 1;
    // Ring k from bottom pole (k=0) to top pole; the middle band duplicates the equator.
    let mut ring_params = Vec::new();
    for k in 0..=cap_rings {
        ring_params.push((-PI / 2.0 + k as f32 / cap_rings as f32 * PI / 2.0, -half));
    }
    for k in 0..=cap_rings {
        ring_params.push((k as f32 / cap_rings as f32 * PI / 2.0, half));
    }
    let total = height.max(1e-6);
    for &(lat, yoff) in &ring_params {
        for s in 0..=segments {
            let u = s as f32 / segments as f32;
            let a = u * TAU;
            let n = Vector3::new(-lat.cos() * a.cos(), lat.sin(), -lat.cos() * a.sin());
            let p = n * radius + Vector3::Y * yoff;
            b.vert(p, n, Vector2::new(u, (p.y + height * 0.5) / total));
        }
    }
    let row = segments + 1;
    for r in 0..rings {
        for s in 0..segments {
            let bl = r * row + s;
            b.quad(bl, bl + row, bl + row + 1, bl + 1);
        }
    }
    b.build("Capsule")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every triangle must be clockwise seen from outside: `cross(b-a, c-a)` along the normal.
    fn check_winding(m: &Mesh) {
        let mut bad = 0;
        for [a, b, c] in m.triangles() {
            let (pa, pb, pc) = (
                m.vertices[a as usize],
                m.vertices[b as usize],
                m.vertices[c as usize],
            );
            let face = (pb - pa).cross(pc - pa);
            if face.length_squared() < 1e-12 {
                continue; // degenerate pole triangle
            }
            let n = m.normals[a as usize] + m.normals[b as usize] + m.normals[c as usize];
            if face.dot(n) <= 0.0 {
                bad += 1;
            }
        }
        assert_eq!(bad, 0, "{} has {bad} inward-facing triangles", m.name);
    }

    #[test]
    fn primitives_face_outward() {
        for p in PrimitiveType::ALL {
            check_winding(&p.mesh());
        }
    }

    #[test]
    fn unity_sizes() {
        let size = |p: PrimitiveType| p.mesh().bounds.size();
        assert!((size(PrimitiveType::Cube) - Vector3::ONE).length() < 1e-5);
        assert!((size(PrimitiveType::Sphere) - Vector3::ONE).length() < 1e-3);
        assert!((size(PrimitiveType::Capsule) - Vector3::new(1.0, 2.0, 1.0)).length() < 1e-3);
        assert!((size(PrimitiveType::Cylinder) - Vector3::new(1.0, 2.0, 1.0)).length() < 1e-3);
        assert!((size(PrimitiveType::Plane) - Vector3::new(10.0, 0.0, 10.0)).length() < 1e-5);
        assert_eq!(PrimitiveType::Quad.mesh().normals[0], Vector3::NEG_Z);
    }

    #[test]
    fn tangents_follow_uv() {
        let m = quad();
        // Quad: u grows along +X, v along +Y.
        assert!((m.tangents[0].truncate() - Vector3::X).length() < 1e-5);
        let bit = m.normals[0].cross(m.tangents[0].truncate()) * m.tangents[0].w;
        assert!((bit - Vector3::Y).length() < 1e-5);
    }
}
