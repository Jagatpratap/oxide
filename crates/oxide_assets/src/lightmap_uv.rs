//! Lightmap UV generation (Unity's model import option "Generate Lightmap UVs"): triangles
//! are grouped into planar charts by their dominant axis and connectivity, each chart is
//! projected onto its plane (keeping real-world proportions), and the charts are packed into
//! the unit square with a margin. Vertices on chart borders are split.
//! <https://docs.unity3d.com/Manual/LightingGiUvs-GeneratingLightmappingUVs.html>

use crate::mesh::Mesh;
use oxide_math::{Vector2, Vector3};
use std::collections::HashMap;

/// Gap between charts as a fraction of the packed size (about 2 texels at 128 px).
const MARGIN: f32 = 1.0 / 64.0;

fn dominant_axis(n: Vector3) -> usize {
    let a = n.abs();
    let (axis, v) = if a.x >= a.y && a.x >= a.z {
        (0, n.x)
    } else if a.y >= a.z {
        (1, n.y)
    } else {
        (2, n.z)
    };
    axis * 2 + (v < 0.0) as usize
}

/// 2D coordinates of `p` on the plane of a dominant axis.
fn project(axis_dir: usize, p: Vector3) -> Vector2 {
    match axis_dir / 2 {
        0 => Vector2::new(p.z, p.y),
        1 => Vector2::new(p.x, p.z),
        _ => Vector2::new(p.x, p.y),
    }
}

/// Union-find for chart grouping.
fn find(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}

impl Mesh {
    /// Whether `uv2` can be used for lightmaps: present and inside [0, 1].
    pub fn has_lightmap_uvs(&self) -> bool {
        self.uv2.len() == self.vertices.len()
            && !self.uv2.is_empty()
            && self
                .uv2
                .iter()
                .all(|u| (0.0..=1.0).contains(&u.x) && (0.0..=1.0).contains(&u.y))
    }

    /// Generates non-overlapping lightmap UVs into `uv2` (splitting vertices on chart
    /// borders). Deterministic, so the same mesh always gets the same UVs.
    pub fn generate_lightmap_uvs(&mut self) {
        let tri_count = self.indices.len() / 3;
        if tri_count == 0 {
            return;
        }
        // Weld positions so charts connect across split vertices (hard edges, UV seams).
        let key = |p: Vector3| {
            let q = |v: f32| (v * 1e4).round() as i64;
            (q(p.x), q(p.y), q(p.z))
        };
        let mut welded: HashMap<(i64, i64, i64), usize> = HashMap::new();
        let weld: Vec<usize> = self
            .vertices
            .iter()
            .map(|&p| {
                let n = welded.len();
                *welded.entry(key(p)).or_insert(n)
            })
            .collect();
        let tri = |t: usize| {
            [
                self.indices[t * 3] as usize,
                self.indices[t * 3 + 1] as usize,
                self.indices[t * 3 + 2] as usize,
            ]
        };
        let axis: Vec<usize> = (0..tri_count)
            .map(|t| {
                let [a, b, c] = tri(t).map(|i| self.vertices[i]);
                dominant_axis((b - a).cross(c - a))
            })
            .collect();
        // Charts: triangles of the same axis sharing an edge.
        let mut parent: Vec<usize> = (0..tri_count).collect();
        let mut edges: HashMap<(usize, usize, usize), usize> = HashMap::new();
        for (t, &tri_axis) in axis.iter().enumerate() {
            let w = tri(t).map(|i| weld[i]);
            for e in 0..3 {
                let (u, v) = (w[e], w[(e + 1) % 3]);
                let k = (u.min(v), u.max(v), tri_axis);
                match edges.get(&k) {
                    Some(&other) => {
                        let (ra, rb) = (find(&mut parent, t), find(&mut parent, other));
                        parent[ra] = rb;
                    }
                    None => {
                        edges.insert(k, t);
                    }
                }
            }
        }
        let mut chart_of: HashMap<usize, usize> = HashMap::new();
        let charts: Vec<usize> = (0..tri_count)
            .map(|t| {
                let r = find(&mut parent, t);
                let n = chart_of.len();
                *chart_of.entry(r).or_insert(n)
            })
            .collect();
        let chart_count = chart_of.len();

        // One output vertex per (original vertex, chart).
        let mut remap: HashMap<(usize, usize), u32> = HashMap::new();
        let mut source: Vec<usize> = Vec::new();
        let mut vert_chart: Vec<usize> = Vec::new();
        let mut indices = Vec::with_capacity(self.indices.len());
        for (t, &chart) in charts.iter().enumerate() {
            for i in tri(t) {
                let n = source.len() as u32;
                let id = *remap.entry((i, chart)).or_insert_with(|| {
                    source.push(i);
                    vert_chart.push(chart);
                    n
                });
                indices.push(id);
            }
        }
        // Chart axis (any triangle's) and projected 2D bounds.
        let mut chart_axis = vec![0; chart_count];
        for t in 0..tri_count {
            chart_axis[charts[t]] = axis[t];
        }
        let mut lo = vec![Vector2::splat(f32::MAX); chart_count];
        let mut hi = vec![Vector2::splat(f32::MIN); chart_count];
        let mut uv2: Vec<Vector2> = source
            .iter()
            .zip(&vert_chart)
            .map(|(&i, &c)| {
                let p = project(chart_axis[c], self.vertices[i]);
                lo[c] = lo[c].min(p);
                hi[c] = hi[c].max(p);
                p
            })
            .collect();

        // Shelf packing, tallest charts first, into a square whose side is chosen from the
        // total area (grown until everything fits).
        let size: Vec<Vector2> = (0..chart_count)
            .map(|c| (hi[c] - lo[c]).max(Vector2::splat(1e-4)))
            .collect();
        let area: f32 = size.iter().map(|s| s.x * s.y).sum();
        let longest = size.iter().map(|s| s.x.max(s.y)).fold(0.0, f32::max);
        let mut order: Vec<usize> = (0..chart_count).collect();
        order.sort_by(|&a, &b| size[b].y.total_cmp(&size[a].y).then(a.cmp(&b)));
        let mut side = area.sqrt().max(longest) * 1.1;
        let (offsets, extent) = loop {
            let margin = side * MARGIN;
            let mut offsets = vec![Vector2::ZERO; chart_count];
            let (mut x, mut y, mut shelf) = (margin, margin, 0.0f32);
            let mut fits = true;
            for &c in &order {
                if x + size[c].x + margin > side {
                    x = margin;
                    y += shelf + margin;
                    shelf = 0.0;
                }
                if x + size[c].x + margin > side || y + size[c].y + margin > side {
                    fits = false;
                    break;
                }
                offsets[c] = Vector2::new(x, y);
                x += size[c].x + margin;
                shelf = shelf.max(size[c].y);
            }
            if fits {
                break (offsets, side);
            }
            side *= 1.15;
        };
        for (v, &c) in uv2.iter_mut().zip(&vert_chart) {
            *v = (*v - lo[c] + offsets[c]) / extent;
        }

        // Split vertices: copy every attribute for the new vertex list.
        fn pick<T: Clone>(v: &[T], source: &[usize]) -> Vec<T> {
            if v.is_empty() {
                return Vec::new();
            }
            source.iter().map(|&i| v[i].clone()).collect()
        }
        self.vertices = pick(&self.vertices, &source);
        self.normals = pick(&self.normals, &source);
        self.tangents = pick(&self.tangents, &source);
        self.uv = pick(&self.uv, &source);
        self.colors = pick(&self.colors, &source);
        self.bone_indices = pick(&self.bone_indices, &source);
        self.bone_weights = pick(&self.bone_weights, &source);
        self.uv2 = uv2;
        self.indices = indices;
    }
}

#[cfg(test)]
mod tests {
    use crate::PrimitiveType;

    /// Rasterizes every triangle's lightmap UVs into a grid; a cell hit by two charts means
    /// overlap.
    fn overlapping_cells(m: &crate::Mesh, res: usize) -> usize {
        let mut owner: Vec<Option<usize>> = vec![None; res * res];
        let mut overlaps = 0;
        for (t, tri) in m.indices.chunks(3).enumerate() {
            let p = [0, 1, 2].map(|k| m.uv2[tri[k] as usize] * res as f32);
            let edge = |a: oxide_math::Vector2, b: oxide_math::Vector2, c: oxide_math::Vector2| {
                (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
            };
            let area = edge(p[0], p[1], p[2]);
            if area.abs() < 1e-9 {
                continue;
            }
            for y in 0..res {
                for x in 0..res {
                    let c = oxide_math::Vector2::new(x as f32 + 0.5, y as f32 + 0.5);
                    let w = [
                        edge(p[1], p[2], c),
                        edge(p[2], p[0], c),
                        edge(p[0], p[1], c),
                    ];
                    let inside = w.iter().all(|&v| v * area > 1e-6 * area.abs());
                    if inside {
                        // Triangles of one chart share cells along their edges only via
                        // the strict test, so any second owner is another chart.
                        match owner[y * res + x] {
                            Some(o) if o != t => overlaps += 1,
                            _ => owner[y * res + x] = Some(t),
                        }
                    }
                }
            }
        }
        overlaps
    }

    #[test]
    fn primitives_get_valid_non_overlapping_lightmap_uvs() {
        for p in PrimitiveType::ALL {
            let mut m = p.mesh();
            m.generate_lightmap_uvs();
            assert!(m.has_lightmap_uvs(), "{p:?}");
            assert_eq!(m.uv2.len(), m.vertices.len());
            assert_eq!(m.normals.len(), m.vertices.len());
            assert_eq!(overlapping_cells(&m, 128), 0, "{p:?} charts overlap");
            // Deterministic.
            let mut again = p.mesh();
            again.generate_lightmap_uvs();
            assert_eq!(again.uv2, m.uv2);
        }
    }
}
