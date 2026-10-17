//! Unity `Mesh`: vertex attribute arrays, sub-meshes (one per material) and bounds.
//!
//! Conventions match Unity: left-handed space, **clockwise** front faces, UV origin at the
//! bottom-left. Reference: <https://docs.unity3d.com/Manual/mesh-introduction.html>

use crate::handle::Asset;
use oxide_math::{Bounds, Vector2, Vector3, Vector4};
use std::ops::Range;

#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub name: String,
    pub vertices: Vec<Vector3>,
    pub normals: Vec<Vector3>,
    /// `xyz` tangent, `w` = bitangent sign (`bitangent = cross(normal, tangent) * w`).
    pub tangents: Vec<Vector4>,
    pub uv: Vec<Vector2>,
    pub uv2: Vec<Vector2>,
    pub colors: Vec<Vector4>,
    /// Skinning: four bone indices and weights per vertex.
    pub bone_indices: Vec<[u16; 4]>,
    pub bone_weights: Vec<Vector4>,
    /// Inverse bind pose per bone (Unity `Mesh.bindposes`).
    pub bindposes: Vec<oxide_math::Matrix4x4>,
    pub indices: Vec<u32>,
    /// Index ranges, one per material slot.
    pub submeshes: Vec<Range<u32>>,
    pub bounds: Bounds,
}

impl Asset for Mesh {
    const TYPE_NAME: &'static str = "Mesh";
}

impl Mesh {
    /// Builds a single-submesh mesh and computes bounds, plus normals/tangents if missing.
    pub fn new(name: &str, vertices: Vec<Vector3>, uv: Vec<Vector2>, indices: Vec<u32>) -> Self {
        let mut m = Mesh {
            name: name.into(),
            vertices,
            uv,
            ..Default::default()
        };
        m.submeshes = vec![0..indices.len() as u32];
        m.indices = indices;
        m.finish();
        m
    }

    /// Fills in whatever derived data is missing: bounds, normals, tangents, sub-mesh list.
    pub fn finish(&mut self) {
        if self.submeshes.is_empty() {
            self.submeshes = vec![0..self.indices.len() as u32];
        }
        if self.normals.len() != self.vertices.len() {
            self.recalculate_normals();
        }
        if self.uv.len() != self.vertices.len() {
            self.uv = vec![Vector2::ZERO; self.vertices.len()];
        }
        if self.tangents.len() != self.vertices.len() {
            self.recalculate_tangents();
        }
        self.recalculate_bounds();
    }

    pub fn vertex_count(&self) -> usize {
        self.vertices.len()
    }

    pub fn is_skinned(&self) -> bool {
        !self.bone_indices.is_empty() && self.bone_indices.len() == self.vertices.len()
    }

    pub fn triangles(&self) -> impl Iterator<Item = [u32; 3]> + '_ {
        self.indices.as_chunks::<3>().0.iter().copied()
    }

    pub fn recalculate_bounds(&mut self) {
        let mut it = self.vertices.iter();
        let Some(&first) = it.next() else {
            self.bounds = Bounds::default();
            return;
        };
        let (mut min, mut max) = (first, first);
        for &v in it {
            min = min.min(v);
            max = max.max(v);
        }
        self.bounds = Bounds::from_min_max(min, max);
    }

    /// Area-weighted smooth normals. Front faces are clockwise, so for triangle `a b c`
    /// the face normal is `cross(b - a, c - a)`.
    pub fn recalculate_normals(&mut self) {
        let mut n = vec![Vector3::ZERO; self.vertices.len()];
        for [a, b, c] in self.triangles() {
            let (pa, pb, pc) = (
                self.vertices[a as usize],
                self.vertices[b as usize],
                self.vertices[c as usize],
            );
            let face = (pb - pa).cross(pc - pa);
            for i in [a, b, c] {
                n[i as usize] += face;
            }
        }
        self.normals = n.into_iter().map(|v| v.normalize_or(Vector3::Y)).collect();
    }

    /// Unity's "Calculate" normals: corners at the same position are smoothed together when
    /// their faces meet at less than `angle` degrees; sharper edges split the vertex so each
    /// side keeps its own normal. Other attributes are kept per vertex.
    pub fn recalculate_normals_with_angle(&mut self, angle: f32) {
        let cos = angle.clamp(0.0, 180.0).to_radians().cos();
        let tris: Vec<[u32; 3]> = self.triangles().collect();
        let key = |v: Vector3| {
            let q = |x: f32| (x * 1e4).round() as i64;
            (q(v.x), q(v.y), q(v.z))
        };
        // Face normals (area-weighted) and the corners at each position.
        let faces: Vec<Vector3> = tris
            .iter()
            .map(|&[a, b, c]| {
                let (pa, pb, pc) = (
                    self.vertices[a as usize],
                    self.vertices[b as usize],
                    self.vertices[c as usize],
                );
                (pb - pa).cross(pc - pa)
            })
            .collect();
        let mut at: std::collections::HashMap<(i64, i64, i64), Vec<usize>> = Default::default();
        for (t, tri) in tris.iter().enumerate() {
            for &v in tri {
                at.entry(key(self.vertices[v as usize]))
                    .or_default()
                    .push(t);
            }
        }
        // New vertices: one per (original vertex, smoothed normal).
        let mut out: Vec<(u32, Vector3)> = Vec::new();
        let mut lookup: std::collections::HashMap<(u32, [i32; 3]), u32> = Default::default();
        let mut indices = Vec::with_capacity(self.indices.len());
        for (t, tri) in tris.iter().enumerate() {
            let fn_t = faces[t].normalize_or_zero();
            for &v in tri {
                let mut n = Vector3::ZERO;
                for &o in &at[&key(self.vertices[v as usize])] {
                    if faces[o].normalize_or_zero().dot(fn_t) >= cos - 1e-6 {
                        n += faces[o];
                    }
                }
                let n = n.normalize_or(fn_t);
                let q = [
                    (n.x * 1000.0) as i32,
                    (n.y * 1000.0) as i32,
                    (n.z * 1000.0) as i32,
                ];
                let i = *lookup.entry((v, q)).or_insert_with(|| {
                    out.push((v, n));
                    out.len() as u32 - 1
                });
                indices.push(i);
            }
        }
        // Rebuild the vertex arrays (sub-mesh ranges are index ranges and stay valid).
        fn pick<T: Copy>(src: &[T], out: &[(u32, Vector3)]) -> Vec<T> {
            if src.is_empty() {
                return Vec::new();
            }
            out.iter().map(|&(v, _)| src[v as usize]).collect()
        }
        self.vertices = pick(&self.vertices, &out);
        self.uv = pick(&self.uv, &out);
        self.uv2 = pick(&self.uv2, &out);
        self.colors = pick(&self.colors, &out);
        self.bone_indices = pick(&self.bone_indices, &out);
        self.bone_weights = pick(&self.bone_weights, &out);
        self.normals = out.iter().map(|&(_, n)| n).collect();
        self.indices = indices;
        self.tangents.clear();
        if !self.uv.is_empty() {
            self.recalculate_tangents();
        }
    }

    /// Per-vertex tangents from UVs (Lengyel's method), orthogonalized against the normal.
    pub fn recalculate_tangents(&mut self) {
        let count = self.vertices.len();
        let mut tan = vec![Vector3::ZERO; count];
        let mut bit = vec![Vector3::ZERO; count];
        if self.uv.len() == count {
            for [a, b, c] in self.triangles() {
                let (a, b, c) = (a as usize, b as usize, c as usize);
                let e1 = self.vertices[b] - self.vertices[a];
                let e2 = self.vertices[c] - self.vertices[a];
                let d1 = self.uv[b] - self.uv[a];
                let d2 = self.uv[c] - self.uv[a];
                let det = d1.x * d2.y - d2.x * d1.y;
                if det.abs() < 1e-12 {
                    continue;
                }
                let r = 1.0 / det;
                let t = (e1 * d2.y - e2 * d1.y) * r;
                let bt = (e2 * d1.x - e1 * d2.x) * r;
                for i in [a, b, c] {
                    tan[i] += t;
                    bit[i] += bt;
                }
            }
        }
        self.tangents = (0..count)
            .map(|i| {
                let n = self.normals.get(i).copied().unwrap_or(Vector3::Y);
                let t = tan[i] - n * n.dot(tan[i]);
                let t = if t.length_squared() > 1e-12 {
                    t.normalize()
                } else {
                    n.any_orthonormal_vector()
                };
                let w = if n.cross(t).dot(bit[i]) < 0.0 {
                    -1.0
                } else {
                    1.0
                };
                t.extend(w)
            })
            .collect();
    }

    /// Appends `other` as new sub-meshes (used to merge glTF primitives into one Mesh).
    pub fn append(&mut self, other: &Mesh) {
        let base = self.vertices.len() as u32;
        let start = self.indices.len() as u32;
        let pad2 = |v: &mut Vec<Vector2>, src: &[Vector2], n: usize, len: usize| {
            if !src.is_empty() || !v.is_empty() {
                v.resize(n, Vector2::ZERO);
                if src.len() == len {
                    v.extend_from_slice(src);
                } else {
                    v.resize(n + len, Vector2::ZERO);
                }
            }
        };
        let n = self.vertices.len();
        let len = other.vertices.len();
        self.vertices.extend_from_slice(&other.vertices);
        self.normals.resize(n, Vector3::Y);
        self.normals.extend_from_slice(&other.normals);
        self.tangents.resize(n, Vector4::X);
        self.tangents.extend_from_slice(&other.tangents);
        pad2(&mut self.uv, &other.uv, n, len);
        pad2(&mut self.uv2, &other.uv2, n, len);
        if !other.colors.is_empty() || !self.colors.is_empty() {
            self.colors.resize(n, Vector4::ONE);
            if other.colors.len() == len {
                self.colors.extend_from_slice(&other.colors);
            } else {
                self.colors.resize(n + len, Vector4::ONE);
            }
        }
        if !other.bone_indices.is_empty() || !self.bone_indices.is_empty() {
            self.bone_indices.resize(n, [0; 4]);
            self.bone_weights.resize(n, Vector4::X);
            if other.bone_indices.len() == len {
                self.bone_indices.extend_from_slice(&other.bone_indices);
                self.bone_weights.extend_from_slice(&other.bone_weights);
            } else {
                self.bone_indices.resize(n + len, [0; 4]);
                self.bone_weights.resize(n + len, Vector4::X);
            }
        }
        self.indices.extend(other.indices.iter().map(|i| i + base));
        for r in &other.submeshes {
            self.submeshes.push(r.start + start..r.end + start);
        }
        self.recalculate_bounds();
    }
}

#[cfg(test)]
mod angle_tests {
    use super::*;

    #[test]
    fn smoothing_angle_splits_hard_edges() {
        // Two quads meeting at 90 degrees along the X axis (an L), sharing two vertices.
        let v = vec![
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(1.0, 0.0, 0.0),
            Vector3::new(1.0, 0.0, 1.0),
            Vector3::new(0.0, 0.0, 1.0),
            Vector3::new(1.0, 1.0, 0.0),
            Vector3::new(0.0, 1.0, 0.0),
        ];
        let idx = vec![0, 2, 1, 0, 3, 2, 0, 1, 4, 0, 4, 5];
        let mut hard = Mesh::new("L", v.clone(), Vec::new(), idx.clone());
        hard.recalculate_normals_with_angle(60.0);
        // The shared edge vertices are split: 4 + 4.
        assert_eq!(hard.vertices.len(), 8);
        let mut soft = Mesh::new("L", v, Vec::new(), idx);
        soft.recalculate_normals_with_angle(120.0);
        assert_eq!(soft.vertices.len(), 6);
        // A shared vertex's smoothed normal points between the two faces.
        let n = soft.normals[0];
        assert!(n.y.abs() > 0.5 && n.z.abs() > 0.5, "{n:?}");
    }
}
