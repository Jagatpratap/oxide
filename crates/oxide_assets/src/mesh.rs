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
        self.indices.chunks_exact(3).map(|t| [t[0], t[1], t[2]])
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
