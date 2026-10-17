//! More of Unity ProBuilder's actions: Select Edge Loop / Edge Ring, Insert Edge Loop,
//! rotating and scaling elements, and the UV Editor's manual UV operations.

use crate::pbmesh::{PbMesh, edge_key};
use oxide_math::{Quaternion, Vector2, Vector3};
use std::collections::{HashMap, HashSet};

impl PbMesh {
    /// The edge across a quad from `e` (None if `f` isn't a quad or lacks `e`).
    fn opposite_edge(&self, f: usize, e: (u32, u32)) -> Option<(u32, u32)> {
        let idx = &self.faces[f].indices;
        if idx.len() != 4 {
            return None;
        }
        let k = (0..4).find(|&k| edge_key(idx[k], idx[(k + 1) % 4]) == e)?;
        Some(edge_key(idx[(k + 2) % 4], idx[(k + 3) % 4]))
    }

    /// Unity's Select Edge Ring: `edge` and the edges across each quad from it, both ways,
    /// until the ring closes or reaches a non-quad face or the border.
    pub fn edge_ring(&self, edge: (u32, u32)) -> Vec<(u32, u32)> {
        let ef = self.edge_faces();
        let start = edge_key(edge.0, edge.1);
        if !ef.contains_key(&start) {
            return Vec::new();
        }
        let mut ring = vec![start];
        let mut seen: HashSet<(u32, u32)> = ring.iter().copied().collect();
        for &first_face in ef[&start].iter().take(2) {
            let (mut e, mut f) = (start, first_face);
            while let Some(next) = self.opposite_edge(f, e) {
                if !seen.insert(next) {
                    break;
                }
                ring.push(next);
                let Some(&nf) = ef[&next].iter().find(|&&x| x != f) else {
                    break;
                };
                (e, f) = (next, nf);
            }
        }
        ring
    }

    /// Unity's Select Edge Loop: `edge` continued straight through every vertex where four
    /// quads meet (the edge sharing no face with the incoming one), both ways.
    pub fn edge_loop(&self, edge: (u32, u32)) -> Vec<(u32, u32)> {
        let ef = self.edge_faces();
        let start = edge_key(edge.0, edge.1);
        if !ef.contains_key(&start) {
            return Vec::new();
        }
        let mut vertex_edges: HashMap<u32, Vec<(u32, u32)>> = HashMap::new();
        for e in ef.keys() {
            vertex_edges.entry(e.0).or_default().push(*e);
            vertex_edges.entry(e.1).or_default().push(*e);
        }
        let mut out = vec![start];
        let mut seen: HashSet<(u32, u32)> = out.iter().copied().collect();
        for dir in [start.1, start.0] {
            let (mut e, mut v) = (start, dir);
            loop {
                let around = &vertex_edges[&v];
                let quads = around
                    .iter()
                    .flat_map(|x| &ef[x])
                    .all(|&f| self.faces[f].indices.len() == 4);
                if around.len() != 4 || !quads {
                    break;
                }
                let faces: HashSet<usize> = ef[&e].iter().copied().collect();
                let Some(&next) = around
                    .iter()
                    .find(|x| **x != e && !ef[*x].iter().any(|f| faces.contains(f)))
                else {
                    break;
                };
                if !seen.insert(next) {
                    break;
                }
                out.push(next);
                v = if next.0 == v { next.1 } else { next.0 };
                e = next;
            }
        }
        out
    }

    /// Unity's Insert Edge Loop: splits every edge of `edge`'s ring at its middle and cuts
    /// each quad of the ring in two; other faces touching a split edge get the new vertex.
    /// Returns the new loop's edges.
    pub fn insert_edge_loop(&mut self, edge: (u32, u32)) -> Vec<(u32, u32)> {
        let ring = self.edge_ring(edge);
        if ring.is_empty() {
            return Vec::new();
        }
        let mut mid: HashMap<(u32, u32), u32> = HashMap::new();
        for e in &ring {
            let p = (self.positions[e.0 as usize] + self.positions[e.1 as usize]) * 0.5;
            mid.insert(*e, self.positions.len() as u32);
            self.positions.push(p);
        }
        let mut new_edges = Vec::new();
        let mut faces = Vec::with_capacity(self.faces.len() + ring.len());
        for f in std::mem::take(&mut self.faces) {
            let idx = &f.indices;
            let n = idx.len();
            let cut: Vec<usize> = (0..n)
                .filter(|&k| mid.contains_key(&edge_key(idx[k], idx[(k + 1) % n])))
                .collect();
            if n == 4 && cut.len() == 2 && cut[1] == cut[0] + 2 {
                // Edges k and k+2 are cut: two quads.
                let k = cut[0];
                let at = |j: usize| idx[(k + j) % 4];
                let m0 = mid[&edge_key(at(0), at(1))];
                let m1 = mid[&edge_key(at(2), at(3))];
                let mut a = f.clone();
                a.indices = vec![at(0), m0, m1, at(3)];
                a.manual_uv.clear();
                let mut b = f.clone();
                b.indices = vec![m0, at(1), at(2), m1];
                b.manual_uv.clear();
                new_edges.push(edge_key(m0, m1));
                faces.push(a);
                faces.push(b);
            } else if cut.is_empty() {
                faces.push(f);
            } else {
                // Keep the face whole, with the new vertices on its split edges.
                let mut g = f.clone();
                g.indices = Vec::with_capacity(n + cut.len());
                for k in 0..n {
                    g.indices.push(idx[k]);
                    if let Some(m) = mid.get(&edge_key(idx[k], idx[(k + 1) % n])) {
                        g.indices.push(*m);
                    }
                }
                g.manual_uv.clear();
                faces.push(g);
            }
        }
        self.faces = faces;
        new_edges.sort();
        new_edges.dedup();
        new_edges
    }

    /// Rotates vertices around `pivot` (the rotate handle on elements).
    pub fn rotate_vertices(&mut self, vertices: &[u32], pivot: Vector3, rotation: Quaternion) {
        let set: HashSet<u32> = vertices.iter().copied().collect();
        for v in set {
            if let Some(p) = self.positions.get_mut(v as usize) {
                *p = pivot + rotation * (*p - pivot);
            }
        }
    }

    /// Scales vertices about `pivot` (the scale handle on elements).
    pub fn scale_vertices(&mut self, vertices: &[u32], pivot: Vector3, scale: Vector3) {
        let set: HashSet<u32> = vertices.iter().copied().collect();
        for v in set {
            if let Some(p) = self.positions.get_mut(v as usize) {
                *p = pivot + (*p - pivot) * scale;
            }
        }
    }

    /// The center of some vertices.
    pub fn vertices_center(&self, vertices: &[u32]) -> Vector3 {
        let ps: Vec<Vector3> = vertices
            .iter()
            .filter_map(|v| self.positions.get(*v as usize).copied())
            .collect();
        ps.iter().copied().sum::<Vector3>() / ps.len().max(1) as f32
    }

    // ---- UV Editor --------------------------------------------------------------------

    /// Converts faces to manual UVs, starting from what they show now.
    pub fn uv_to_manual(&mut self, faces: &[usize]) {
        for &f in faces {
            if f < self.faces.len() && !self.faces[f].has_manual_uv() {
                self.faces[f].manual_uv = self.face_uvs(f);
            }
        }
    }

    /// Puts faces back on Auto UVs.
    pub fn uv_reset_auto(&mut self, faces: &[usize]) {
        for &f in faces {
            if let Some(face) = self.faces.get_mut(f) {
                face.manual_uv.clear();
            }
        }
    }

    /// The center of the faces' UVs.
    pub fn uv_center(&self, faces: &[usize]) -> Vector2 {
        let uvs: Vec<Vector2> = faces
            .iter()
            .filter(|f| **f < self.faces.len())
            .flat_map(|f| self.face_uvs(*f))
            .collect();
        uvs.iter().copied().sum::<Vector2>() / uvs.len().max(1) as f32
    }

    /// Moves, rotates (degrees) and scales the faces' UVs about `pivot`, converting them to
    /// manual UVs first (Unity's UV Editor handles).
    pub fn uv_transform(
        &mut self,
        faces: &[usize],
        translate: Vector2,
        rotate: f32,
        scale: Vector2,
        pivot: Vector2,
    ) {
        self.uv_to_manual(faces);
        let (s, c) = rotate.to_radians().sin_cos();
        let mut done = HashSet::new();
        for &f in faces {
            if !done.insert(f) {
                continue;
            }
            let Some(face) = self.faces.get_mut(f) else {
                continue;
            };
            for uv in &mut face.manual_uv {
                let d = (*uv - pivot) * scale;
                *uv = pivot + Vector2::new(d.x * c - d.y * s, d.x * s + d.y * c) + translate;
            }
        }
    }

    /// Unity's Planar Project: the faces' UVs from one projection along their average normal,
    /// fitted to the 0–1 square (one island).
    pub fn uv_planar_project(&mut self, faces: &[usize]) {
        let faces: Vec<usize> = faces
            .iter()
            .copied()
            .filter(|f| *f < self.faces.len())
            .collect();
        if faces.is_empty() {
            return;
        }
        let n = faces
            .iter()
            .map(|f| self.face_normal(*f))
            .sum::<Vector3>()
            .normalize_or(Vector3::Y);
        let t = if n.y.abs() < 0.9 {
            Vector3::Y
        } else {
            Vector3::Z
        };
        let u = t.cross(n).normalize_or(Vector3::X);
        let v = n.cross(u);
        let project = |p: Vector3| Vector2::new(p.dot(u), p.dot(v));
        let all: Vec<Vector2> = faces
            .iter()
            .flat_map(|f| self.faces[*f].indices.iter())
            .map(|i| project(self.positions[*i as usize]))
            .collect();
        let min = all
            .iter()
            .copied()
            .fold(Vector2::splat(f32::MAX), Vector2::min);
        let max = all
            .iter()
            .copied()
            .fold(Vector2::splat(f32::MIN), Vector2::max);
        let size = (max - min).max_element().max(1e-6);
        for f in faces {
            let uvs: Vec<Vector2> = self.faces[f]
                .indices
                .iter()
                .map(|i| (project(self.positions[*i as usize]) - min) / size)
                .collect();
            self.faces[f].manual_uv = uvs;
        }
    }

    /// Unity's Fit UVs: scales and moves the faces' UVs to fill the 0–1 square.
    pub fn uv_fit(&mut self, faces: &[usize]) {
        self.uv_to_manual(faces);
        let all: Vec<Vector2> = faces
            .iter()
            .filter_map(|f| self.faces.get(*f))
            .flat_map(|f| f.manual_uv.iter().copied())
            .collect();
        if all.is_empty() {
            return;
        }
        let min = all
            .iter()
            .copied()
            .fold(Vector2::splat(f32::MAX), Vector2::min);
        let max = all
            .iter()
            .copied()
            .fold(Vector2::splat(f32::MIN), Vector2::max);
        let size = (max - min).max(Vector2::splat(1e-6));
        let mut done = HashSet::new();
        for &f in faces {
            if !done.insert(f) {
                continue;
            }
            if let Some(face) = self.faces.get_mut(f) {
                for uv in &mut face.manual_uv {
                    *uv = (*uv - min) / size;
                }
            }
        }
    }
}
