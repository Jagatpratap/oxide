//! The editable mesh of a ProBuilder object: shared positions and polygon faces (any
//! number of sides, clockwise seen from outside like Unity), each with a material slot,
//! smoothing group, auto-UV settings and color. [`PbMesh::build`] turns it into a renderable
//! [`Mesh`]; the operations are Unity ProBuilder's actions on faces, edges and vertices.
//! Reference: <https://docs.unity3d.com/Packages/com.unity.probuilder@6.0/manual/index.html>

use oxide_assets::Mesh;
use oxide_core::Reflect;
use oxide_math::{Color, Vector2, Vector3, Vector4};
use std::collections::{HashMap, HashSet};

/// Unity ProBuilder's Auto UV settings of a face.
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct AutoUv {
    pub scale: Vector2,
    pub offset: Vector2,
    /// Degrees.
    pub rotation: f32,
    pub flip_u: bool,
    pub flip_v: bool,
}

impl Default for AutoUv {
    fn default() -> Self {
        Self {
            scale: Vector2::ONE,
            offset: Vector2::ZERO,
            rotation: 0.0,
            flip_u: false,
            flip_v: false,
        }
    }
}

/// One polygon.
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct Face {
    /// Position indices, clockwise seen from outside.
    pub indices: Vec<u32>,
    /// Material slot (sub-mesh).
    pub material: u32,
    /// Faces in the same non-zero group share smooth normals; 0 is hard.
    pub smoothing_group: u32,
    pub uv: AutoUv,
    pub color: Color,
}

impl Default for Face {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl Face {
    pub fn new(indices: Vec<u32>) -> Self {
        Self {
            indices,
            material: 0,
            smoothing_group: 0,
            uv: AutoUv::default(),
            color: Color::WHITE,
        }
    }

    /// Edges in winding order: (from, to).
    pub fn edges(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        let n = self.indices.len();
        (0..n).map(move |i| (self.indices[i], self.indices[(i + 1) % n]))
    }

    fn with(&self, indices: Vec<u32>) -> Face {
        Face {
            indices,
            ..self.clone()
        }
    }
}

/// An undirected edge (smaller index first).
pub fn edge_key(a: u32, b: u32) -> (u32, u32) {
    if a < b { (a, b) } else { (b, a) }
}

#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct PbMesh {
    pub positions: Vec<Vector3>,
    pub faces: Vec<Face>,
}

/// The texture axes of a face by its dominant normal axis (right, up seen from outside),
/// matching the engine's primitive cube.
fn uv_axes(n: Vector3) -> (Vector3, Vector3) {
    let a = n.abs();
    if a.y >= a.x && a.y >= a.z {
        if n.y >= 0.0 {
            (Vector3::X, Vector3::Z)
        } else {
            (Vector3::X, Vector3::NEG_Z)
        }
    } else if a.x >= a.z {
        if n.x >= 0.0 {
            (Vector3::Z, Vector3::Y)
        } else {
            (Vector3::NEG_Z, Vector3::Y)
        }
    } else if n.z >= 0.0 {
        (Vector3::NEG_X, Vector3::Y)
    } else {
        (Vector3::X, Vector3::Y)
    }
}

/// Triangulates a polygon (ear clipping in its plane); returns corner indices into `poly`.
pub fn triangulate(poly: &[Vector3], normal: Vector3) -> Vec<[usize; 3]> {
    let n = poly.len();
    if n < 3 {
        return Vec::new();
    }
    if n == 3 {
        return vec![[0, 1, 2]];
    }
    // 2D coordinates in the polygon's plane.
    let (u, v) = {
        let t = if normal.x.abs() < 0.9 {
            Vector3::X
        } else {
            Vector3::Y
        };
        let u = normal.cross(t).normalize_or(Vector3::X);
        (u, u.cross(normal))
    };
    let pts: Vec<Vector2> = poly
        .iter()
        .map(|p| Vector2::new(p.dot(u), p.dot(v)))
        .collect();
    // Orientation of the 2D polygon.
    let area: f32 = (0..n)
        .map(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            a.x * b.y - b.x * a.y
        })
        .sum();
    let sign = if area >= 0.0 { 1.0 } else { -1.0 };
    let cross = |a: Vector2, b: Vector2, c: Vector2| ((b - a).perp_dot(c - a)) * sign;
    let inside = |p: Vector2, a: Vector2, b: Vector2, c: Vector2| {
        cross(a, b, p) >= 0.0 && cross(b, c, p) >= 0.0 && cross(c, a, p) >= 0.0
    };
    let mut idx: Vec<usize> = (0..n).collect();
    let mut out = Vec::with_capacity(n - 2);
    let mut guard = 0;
    while idx.len() > 3 && guard < n * n {
        guard += 1;
        let m = idx.len();
        let mut clipped = false;
        for i in 0..m {
            let (ia, ib, ic) = (idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]);
            let (a, b, c) = (pts[ia], pts[ib], pts[ic]);
            if cross(a, b, c) <= 1e-9 {
                continue;
            }
            if idx
                .iter()
                .any(|&k| k != ia && k != ib && k != ic && inside(pts[k], a, b, c))
            {
                continue;
            }
            out.push([ia, ib, ic]);
            idx.remove(i);
            clipped = true;
            break;
        }
        if !clipped {
            break;
        }
    }
    // Leftovers (degenerate input): fan.
    if idx.len() >= 3 {
        for k in 1..idx.len() - 1 {
            out.push([idx[0], idx[k], idx[k + 1]]);
        }
    }
    out
}

impl PbMesh {
    /// A face's outward normal.
    pub fn face_normal(&self, f: usize) -> Vector3 {
        let face = &self.faces[f];
        let p: Vec<Vector3> = face
            .indices
            .iter()
            .map(|i| self.positions[*i as usize])
            .collect();
        let mut n = Vector3::ZERO;
        for k in 1..p.len().saturating_sub(1) {
            n += (p[k] - p[0]).cross(p[k + 1] - p[0]);
        }
        n.normalize_or(Vector3::Y)
    }

    pub fn face_center(&self, f: usize) -> Vector3 {
        let face = &self.faces[f];
        face.indices
            .iter()
            .map(|i| self.positions[*i as usize])
            .sum::<Vector3>()
            / face.indices.len().max(1) as f32
    }

    /// Every undirected edge and the faces using it.
    pub fn edge_faces(&self) -> HashMap<(u32, u32), Vec<usize>> {
        let mut m: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
        for (fi, f) in self.faces.iter().enumerate() {
            for (a, b) in f.edges() {
                m.entry(edge_key(a, b)).or_default().push(fi);
            }
        }
        m
    }

    /// All undirected edges, sorted.
    pub fn edges(&self) -> Vec<(u32, u32)> {
        let mut e: Vec<(u32, u32)> = self.edge_faces().into_keys().collect();
        e.sort();
        e
    }

    /// The renderable mesh: one vertex per face corner, smooth normals within smoothing
    /// groups, auto UVs, face colors, one sub-mesh per material slot.
    pub fn build(&self, name: &str, material_slots: usize) -> Mesh {
        let normals: Vec<Vector3> = (0..self.faces.len()).map(|f| self.face_normal(f)).collect();
        // Smooth normals: per (position, group) the sum of the group's face normals.
        let mut smooth: HashMap<(u32, u32), Vector3> = HashMap::new();
        for (fi, f) in self.faces.iter().enumerate() {
            if f.smoothing_group == 0 {
                continue;
            }
            for i in &f.indices {
                *smooth.entry((*i, f.smoothing_group)).or_default() += normals[fi];
            }
        }
        let slots = material_slots.max(1);
        let mut per_slot: Vec<Vec<u32>> = vec![Vec::new(); slots];
        let mut m = Mesh {
            name: name.into(),
            ..Default::default()
        };
        for (fi, f) in self.faces.iter().enumerate() {
            if f.indices.len() < 3 {
                continue;
            }
            let n = normals[fi];
            let (r, u) = uv_axes(n);
            let (s, c) = f.uv.rotation.to_radians().sin_cos();
            let base = m.vertices.len() as u32;
            let poly: Vec<Vector3> = f
                .indices
                .iter()
                .map(|i| self.positions[*i as usize])
                .collect();
            for (k, p) in poly.iter().enumerate() {
                m.vertices.push(*p);
                let nn = if f.smoothing_group != 0 {
                    smooth
                        .get(&(f.indices[k], f.smoothing_group))
                        .copied()
                        .unwrap_or(n)
                        .normalize_or(n)
                } else {
                    n
                };
                m.normals.push(nn);
                let mut uv = Vector2::new(p.dot(r), p.dot(u));
                uv = Vector2::new(uv.x * c - uv.y * s, uv.x * s + uv.y * c);
                uv = uv * f.uv.scale + f.uv.offset;
                if f.uv.flip_u {
                    uv.x = -uv.x;
                }
                if f.uv.flip_v {
                    uv.y = -uv.y;
                }
                m.uv.push(uv);
                m.colors
                    .push(Vector4::new(f.color.r, f.color.g, f.color.b, f.color.a));
            }
            let slot = (f.material as usize).min(slots - 1);
            for [a, b, cc] in triangulate(&poly, n) {
                per_slot[slot].extend([base + a as u32, base + b as u32, base + cc as u32]);
            }
        }
        for s in per_slot {
            let start = m.indices.len() as u32;
            m.indices.extend(s);
            m.submeshes.push(start..m.indices.len() as u32);
        }
        m.recalculate_tangents();
        m.recalculate_bounds();
        m
    }

    // ---- Operations (Unity ProBuilder actions).

    /// Moves vertices (Unity: translate selected elements).
    pub fn translate(&mut self, vertices: &[u32], delta: Vector3) {
        let set: HashSet<u32> = vertices.iter().copied().collect();
        for v in set {
            if let Some(p) = self.positions.get_mut(v as usize) {
                *p += delta;
            }
        }
    }

    /// The vertices of some faces.
    pub fn face_vertices(&self, faces: &[usize]) -> Vec<u32> {
        let mut v: Vec<u32> = faces
            .iter()
            .filter_map(|f| self.faces.get(*f))
            .flat_map(|f| f.indices.iter().copied())
            .collect();
        v.sort();
        v.dedup();
        v
    }

    /// Unity's Extrude Faces: the faces move out by `distance` along their normals as a
    /// group, with new side faces along the selection's boundary.
    pub fn extrude_faces(&mut self, faces: &[usize], distance: f32) {
        let sel: HashSet<usize> = faces
            .iter()
            .copied()
            .filter(|f| *f < self.faces.len())
            .collect();
        if sel.is_empty() {
            return;
        }
        // Boundary edges: used by exactly one selected face (directed as in that face).
        let mut count: HashMap<(u32, u32), usize> = HashMap::new();
        for f in &sel {
            for (a, b) in self.faces[*f].edges() {
                *count.entry(edge_key(a, b)).or_default() += 1;
            }
        }
        let mut boundary: Vec<(u32, u32, usize)> = Vec::new();
        for f in &sel {
            for (a, b) in self.faces[*f].edges() {
                if count[&edge_key(a, b)] == 1 {
                    boundary.push((a, b, *f));
                }
            }
        }
        // Each vertex moves along the average normal of the selected faces using it.
        let mut dir: HashMap<u32, Vector3> = HashMap::new();
        for f in &sel {
            let n = self.face_normal(*f);
            for i in &self.faces[*f].indices {
                *dir.entry(*i).or_default() += n;
            }
        }
        let mut new_index: HashMap<u32, u32> = HashMap::new();
        let mut keys: Vec<u32> = dir.keys().copied().collect();
        keys.sort();
        for v in keys {
            let d = dir[&v].normalize_or(Vector3::Y);
            new_index.insert(v, self.positions.len() as u32);
            self.positions
                .push(self.positions[v as usize] + d * distance);
        }
        for f in &sel {
            let face = &mut self.faces[*f];
            for i in &mut face.indices {
                *i = new_index[i];
            }
        }
        for (a, b, owner) in boundary {
            let (a2, b2) = (new_index[&a], new_index[&b]);
            // Clockwise from outside: along the old edge, out, then back along the new one.
            let side = self.faces[owner].with(vec![a, b, b2, a2]);
            self.faces.push(Face {
                smoothing_group: 0,
                ..side
            });
        }
    }

    /// Unity's Inset (Extrude with distance 0 then scale): each face gets a smaller copy
    /// inside it joined by a ring of quads.
    pub fn inset_faces(&mut self, faces: &[usize], amount: f32) {
        for &f in faces {
            if f >= self.faces.len() {
                continue;
            }
            let c = self.face_center(f);
            let old = self.faces[f].indices.clone();
            let inner: Vec<u32> = old
                .iter()
                .map(|i| {
                    let p = self.positions[*i as usize];
                    self.positions.push(p + (c - p) * amount.clamp(0.0, 0.99));
                    self.positions.len() as u32 - 1
                })
                .collect();
            let n = old.len();
            for k in 0..n {
                let (a, b) = (old[k], old[(k + 1) % n]);
                let (ia, ib) = (inner[k], inner[(k + 1) % n]);
                let q = self.faces[f].with(vec![a, b, ib, ia]);
                self.faces.push(q);
            }
            self.faces[f].indices = inner;
        }
    }

    /// Unity's Delete Faces (unused vertices removed).
    pub fn delete_faces(&mut self, faces: &[usize]) {
        let set: HashSet<usize> = faces.iter().copied().collect();
        let mut k = 0;
        self.faces.retain(|_| {
            let keep = !set.contains(&k);
            k += 1;
            keep
        });
        self.remove_unused();
    }

    /// Unity's Flip Face Normals.
    pub fn flip_faces(&mut self, faces: &[usize]) {
        for &f in faces {
            if let Some(face) = self.faces.get_mut(f) {
                face.indices.reverse();
            }
        }
    }

    /// Unity's Subdivide Faces: each face becomes quads around its center through its edge
    /// midpoints; neighbours get the midpoints too, so the mesh stays watertight.
    pub fn subdivide_faces(&mut self, faces: &[usize]) {
        let sel: Vec<usize> = faces
            .iter()
            .copied()
            .filter(|f| *f < self.faces.len())
            .collect();
        let mut mid: HashMap<(u32, u32), u32> = HashMap::new();
        for &f in &sel {
            let edges: Vec<(u32, u32)> = self.faces[f].edges().collect();
            for (a, b) in edges {
                mid.entry(edge_key(a, b)).or_insert_with(|| {
                    let p = (self.positions[a as usize] + self.positions[b as usize]) * 0.5;
                    self.positions.push(p);
                    self.positions.len() as u32 - 1
                });
            }
        }
        let selected: HashSet<usize> = sel.iter().copied().collect();
        // Neighbours: insert midpoints into their loops.
        for (fi, face) in self.faces.iter_mut().enumerate() {
            if selected.contains(&fi) {
                continue;
            }
            let n = face.indices.len();
            let mut out = Vec::with_capacity(n * 2);
            for k in 0..n {
                let (a, b) = (face.indices[k], face.indices[(k + 1) % n]);
                out.push(a);
                if let Some(m) = mid.get(&edge_key(a, b)) {
                    out.push(*m);
                }
            }
            face.indices = out;
        }
        let mut new_faces = Vec::new();
        for &f in &sel {
            let c = self.face_center(f);
            self.positions.push(c);
            let ci = self.positions.len() as u32 - 1;
            let idx = self.faces[f].indices.clone();
            let n = idx.len();
            for k in 0..n {
                let prev = idx[(k + n - 1) % n];
                let cur = idx[k];
                let next = idx[(k + 1) % n];
                let m0 = mid[&edge_key(prev, cur)];
                let m1 = mid[&edge_key(cur, next)];
                new_faces.push(self.faces[f].with(vec![m0, cur, m1, ci]));
            }
        }
        let mut k = 0;
        self.faces.retain(|_| {
            let keep = !selected.contains(&k);
            k += 1;
            keep
        });
        self.faces.extend(new_faces);
    }

    /// Unity's Merge Faces: adjacent faces become one polygon (their outer boundary).
    /// Returns false when the faces don't form one connected region.
    pub fn merge_faces(&mut self, faces: &[usize]) -> bool {
        let sel: HashSet<usize> = faces
            .iter()
            .copied()
            .filter(|f| *f < self.faces.len())
            .collect();
        if sel.len() < 2 {
            return false;
        }
        let mut count: HashMap<(u32, u32), usize> = HashMap::new();
        for f in &sel {
            for (a, b) in self.faces[*f].edges() {
                *count.entry(edge_key(a, b)).or_default() += 1;
            }
        }
        let mut next: HashMap<u32, u32> = HashMap::new();
        for f in &sel {
            for (a, b) in self.faces[*f].edges() {
                if count[&edge_key(a, b)] == 1 && next.insert(a, b).is_some() {
                    return false;
                }
            }
        }
        let Some(&start) = next.keys().min() else {
            return false;
        };
        let mut loop_ = vec![start];
        let mut cur = start;
        for _ in 0..next.len() {
            let Some(&n) = next.get(&cur) else {
                return false;
            };
            if n == start {
                break;
            }
            loop_.push(n);
            cur = n;
        }
        if loop_.len() != next.len() {
            return false;
        }
        let first = *sel.iter().min().expect("selection");
        let merged = self.faces[first].with(loop_);
        let mut k = 0;
        self.faces.retain(|_| {
            let keep = !sel.contains(&k);
            k += 1;
            keep
        });
        self.faces.push(merged);
        true
    }

    /// Unity's Triangulate Faces.
    pub fn triangulate_faces(&mut self, faces: &[usize]) {
        let sel: HashSet<usize> = faces.iter().copied().collect();
        let mut new = Vec::new();
        for (fi, f) in self.faces.iter().enumerate() {
            if !sel.contains(&fi) || f.indices.len() <= 3 {
                new.push(f.clone());
                continue;
            }
            let poly: Vec<Vector3> = f
                .indices
                .iter()
                .map(|i| self.positions[*i as usize])
                .collect();
            for [a, b, c] in triangulate(&poly, self.face_normal(fi)) {
                new.push(f.with(vec![f.indices[a], f.indices[b], f.indices[c]]));
            }
        }
        self.faces = new;
    }

    /// Unity's Detach Faces (to a new object): removes them and returns them as a mesh.
    pub fn detach_faces(&mut self, faces: &[usize]) -> PbMesh {
        let set: HashSet<usize> = faces.iter().copied().collect();
        let mut out = PbMesh::default();
        let mut map: HashMap<u32, u32> = HashMap::new();
        for (fi, f) in self.faces.iter().enumerate() {
            if !set.contains(&fi) {
                continue;
            }
            let idx = f
                .indices
                .iter()
                .map(|i| {
                    *map.entry(*i).or_insert_with(|| {
                        out.positions.push(self.positions[*i as usize]);
                        out.positions.len() as u32 - 1
                    })
                })
                .collect();
            out.faces.push(f.with(idx));
        }
        self.delete_faces(faces);
        out
    }

    /// Unity's Weld Vertices: vertices closer than `distance` become one.
    pub fn weld_vertices(&mut self, vertices: &[u32], distance: f32) {
        let sel: Vec<u32> = if vertices.is_empty() {
            (0..self.positions.len() as u32).collect()
        } else {
            vertices.to_vec()
        };
        let mut remap: HashMap<u32, u32> = HashMap::new();
        for (k, &a) in sel.iter().enumerate() {
            if remap.contains_key(&a) {
                continue;
            }
            for &b in &sel[k + 1..] {
                if !remap.contains_key(&b)
                    && self.positions[a as usize].distance(self.positions[b as usize]) <= distance
                {
                    remap.insert(b, a);
                }
            }
        }
        self.remap(&remap);
    }

    /// Unity's Collapse Vertices: the vertices move to their centroid and merge.
    pub fn collapse_vertices(&mut self, vertices: &[u32]) {
        if vertices.len() < 2 {
            return;
        }
        let c = vertices
            .iter()
            .map(|v| self.positions[*v as usize])
            .sum::<Vector3>()
            / vertices.len() as f32;
        let keep = vertices[0];
        self.positions[keep as usize] = c;
        let remap: HashMap<u32, u32> = vertices[1..].iter().map(|v| (*v, keep)).collect();
        self.remap(&remap);
    }

    fn remap(&mut self, remap: &HashMap<u32, u32>) {
        for f in &mut self.faces {
            for i in &mut f.indices {
                if let Some(t) = remap.get(i) {
                    *i = *t;
                }
            }
            f.indices.dedup();
            while f.indices.len() > 1 && f.indices.first() == f.indices.last() {
                f.indices.pop();
            }
        }
        self.faces.retain(|f| f.indices.len() >= 3);
        self.remove_unused();
    }

    /// Unity's Bridge Edges: a quad between two open edges.
    pub fn bridge_edges(&mut self, a: (u32, u32), b: (u32, u32)) -> bool {
        let ef = self.edge_faces();
        let (Some(fa), Some(fb)) = (ef.get(&edge_key(a.0, a.1)), ef.get(&edge_key(b.0, b.1)))
        else {
            return false;
        };
        if fa.len() != 1 || fb.len() != 1 {
            return false;
        }
        // Follow the first face's winding on its edge, reversed, so the bridge faces the
        // same way.
        let owner = &self.faces[fa[0]];
        let (x, y) = owner
            .edges()
            .find(|(p, q)| edge_key(*p, *q) == edge_key(a.0, a.1))
            .expect("edge of its face");
        let (p0, p1) = (self.positions[x as usize], self.positions[y as usize]);
        // Pair the far edge's ends with the near edge's by distance.
        let (u, v) = if p0.distance(self.positions[b.0 as usize])
            + p1.distance(self.positions[b.1 as usize])
            <= p0.distance(self.positions[b.1 as usize]) + p1.distance(self.positions[b.0 as usize])
        {
            (b.0, b.1)
        } else {
            (b.1, b.0)
        };
        let face = owner.with(vec![y, x, u, v]);
        self.faces.push(face);
        true
    }

    /// Unity's Center Pivot: moves the positions so their center is the origin. Returns
    /// the offset (the object should move by it to stay put).
    pub fn center_pivot(&mut self) -> Vector3 {
        if self.positions.is_empty() {
            return Vector3::ZERO;
        }
        let (min, max) = self.positions.iter().fold(
            (Vector3::splat(f32::MAX), Vector3::splat(f32::MIN)),
            |(a, b), p| (a.min(*p), b.max(*p)),
        );
        let c = (min + max) * 0.5;
        for p in &mut self.positions {
            *p -= c;
        }
        c
    }

    /// Drops positions no face uses (indices are renumbered).
    pub fn remove_unused(&mut self) {
        let mut used = vec![false; self.positions.len()];
        for f in &self.faces {
            for i in &f.indices {
                if let Some(u) = used.get_mut(*i as usize) {
                    *u = true;
                }
            }
        }
        let mut map = vec![u32::MAX; self.positions.len()];
        let mut out = Vec::new();
        for (k, p) in self.positions.iter().enumerate() {
            if used[k] {
                map[k] = out.len() as u32;
                out.push(*p);
            }
        }
        self.positions = out;
        for f in &mut self.faces {
            for i in &mut f.indices {
                *i = map[*i as usize];
            }
        }
    }

    /// From a Unity mesh (ProBuilderize): triangles become faces sharing welded positions.
    pub fn from_mesh(mesh: &Mesh) -> PbMesh {
        let mut pb = PbMesh::default();
        let mut map: HashMap<[u32; 3], u32> = HashMap::new();
        let key = |p: Vector3| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()];
        let mut index = |p: Vector3, pb: &mut PbMesh| -> u32 {
            *map.entry(key(p)).or_insert_with(|| {
                pb.positions.push(p);
                pb.positions.len() as u32 - 1
            })
        };
        for (slot, range) in mesh.submeshes.iter().enumerate() {
            for tri in mesh.indices[range.start as usize..range.end as usize]
                .as_chunks::<3>()
                .0
            {
                let idx: Vec<u32> = tri
                    .iter()
                    .map(|i| index(mesh.vertices[*i as usize], &mut pb))
                    .collect();
                let mut f = Face::new(idx);
                f.material = slot as u32;
                pb.faces.push(f);
            }
        }
        pb
    }

    /// Wavefront OBJ (Unity ProBuilder's Export → Obj).
    pub fn to_obj(&self, name: &str) -> String {
        let mut s = format!("# Exported from Oxide ProBuilder\no {name}\n");
        for p in &self.positions {
            // OBJ is right-handed: flip Z.
            s.push_str(&format!("v {} {} {}\n", p.x, p.y, -p.z));
        }
        for f in &self.faces {
            s.push('f');
            for i in &f.indices {
                s.push_str(&format!(" {}", i + 1));
            }
            s.push('\n');
        }
        s
    }
}
