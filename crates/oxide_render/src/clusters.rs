//! Clustered light assignment (URP's Forward+): the view frustum is split into a
//! 16 × 9 screen grid × 24 exponential depth slices, and each cluster lists the lights whose
//! range reaches it. Shaders then loop over one cluster's lights instead of all of them.
//! <https://docs.unity3d.com/Manual/urp/rendering/forward-plus-rendering-path.html>

use oxide_math::{Matrix4x4, Vector3};

pub const CLUSTER_X: u32 = 16;
pub const CLUSTER_Y: u32 = 9;
pub const CLUSTER_Z: u32 = 24;
pub const CLUSTERS: usize = (CLUSTER_X * CLUSTER_Y * CLUSTER_Z) as usize;
/// Lights per cluster beyond this are dropped (the nearest-listed ones are kept).
pub const MAX_PER_CLUSTER: usize = 128;

/// Light lists per cluster: `cells[i] = [offset, count]` into `indices`.
#[derive(Clone, Debug, Default)]
pub struct ClusterGrid {
    pub cells: Vec<[u32; 2]>,
    pub indices: Vec<u32>,
    /// Slice of view depth `z` is `floor(ln(z) * scale + bias)`.
    pub scale: f32,
    pub bias: f32,
}

pub fn cluster_index(x: u32, y: u32, z: u32) -> usize {
    ((z * CLUSTER_Y + y) * CLUSTER_X + x) as usize
}

/// Assigns lights given as world-space bounding spheres `(center, radius)` to the clusters
/// of a camera with `view` and `proj` (unjittered) between `near` and `far`.
pub fn assign(
    view: &Matrix4x4,
    proj: &Matrix4x4,
    near: f32,
    far: f32,
    lights: &[(Vector3, f32)],
) -> ClusterGrid {
    let near = near.max(1e-3);
    let far = far.max(near * 1.01);
    let log_ratio = (far / near).ln();
    let scale = CLUSTER_Z as f32 / log_ratio;
    let bias = -(CLUSTER_Z as f32) * near.ln() / log_ratio;
    let slice = |z: f32| {
        ((z.max(near).ln() * scale + bias).floor() as i32).clamp(0, CLUSTER_Z as i32 - 1) as u32
    };

    let mut lists: Vec<Vec<u32>> = vec![Vec::new(); CLUSTERS];
    for (i, &(center, radius)) in lights.iter().enumerate() {
        let c = view.transform_point3(center);
        if c.z + radius < near || c.z - radius > far {
            continue;
        }
        let z0 = (c.z - radius).max(near);
        let z1 = (c.z + radius).min(far);
        // Screen bounds of the light's view-space box, projected at both depth ends.
        let (mut min, mut max) = ((f32::MAX, f32::MAX), (f32::MIN, f32::MIN));
        for z in [z0, z1] {
            for (dx, dy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let p = proj.project_point3(Vector3::new(c.x + dx * radius, c.y + dy * radius, z));
                min = (min.0.min(p.x), min.1.min(p.y));
                max = (max.0.max(p.x), max.1.max(p.y));
            }
        }
        if max.0 < -1.0 || min.0 > 1.0 || max.1 < -1.0 || min.1 > 1.0 {
            continue;
        }
        let tile = |ndc: f32, n: u32| {
            (((ndc * 0.5 + 0.5) * n as f32).floor() as i32).clamp(0, n as i32 - 1) as u32
        };
        let (x0, x1) = (tile(min.0, CLUSTER_X), tile(max.0, CLUSTER_X));
        // Tile rows count from the top of the screen.
        let (y0, y1) = (tile(-max.1, CLUSTER_Y), tile(-min.1, CLUSTER_Y));
        for z in slice(z0)..=slice(z1) {
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let list = &mut lists[cluster_index(x, y, z)];
                    if list.len() < MAX_PER_CLUSTER {
                        list.push(i as u32);
                    }
                }
            }
        }
    }
    let mut grid = ClusterGrid {
        cells: Vec::with_capacity(CLUSTERS),
        indices: Vec::new(),
        scale,
        bias,
    };
    for list in lists {
        grid.cells
            .push([grid.indices.len() as u32, list.len() as u32]);
        grid.indices.extend(list);
    }
    grid
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxide_math::Matrix4x4Ext;

    fn camera() -> (Matrix4x4, Matrix4x4) {
        let view = Matrix4x4::look_to(Vector3::ZERO, Vector3::Z, Vector3::Y);
        let proj = Matrix4x4::perspective(60f32.to_radians(), 16.0 / 9.0, 100.0, 0.1);
        (view, proj)
    }

    fn lights_at(g: &ClusterGrid, x: u32, y: u32, z: f32) -> Vec<u32> {
        let s = ((z.ln() * g.scale + g.bias).floor() as u32).min(CLUSTER_Z - 1);
        let [o, n] = g.cells[cluster_index(x, y, s)];
        g.indices[o as usize..(o + n) as usize].to_vec()
    }

    #[test]
    fn lights_land_only_in_clusters_they_reach() {
        let (view, proj) = camera();
        // Light 0 straight ahead at 10 m; light 1 far to the right at 10 m; light 2 behind.
        let lights = [
            (Vector3::new(0.0, 0.0, 10.0), 1.0),
            (Vector3::new(8.0, 0.0, 10.0), 1.0),
            (Vector3::new(0.0, 0.0, -5.0), 2.0),
        ];
        let g = assign(&view, &proj, 0.1, 100.0, &lights);
        // Center of the screen at 10 m depth: light 0 only.
        assert_eq!(lights_at(&g, 7, 4, 10.0), vec![0]);
        assert_eq!(lights_at(&g, 8, 4, 10.0), vec![0]);
        // Right edge at 10 m: light 1.
        assert!(lights_at(&g, 15, 4, 10.0).contains(&1));
        // Same tile but much nearer or farther: nothing.
        assert!(lights_at(&g, 7, 4, 2.0).is_empty());
        assert!(lights_at(&g, 7, 4, 60.0).is_empty());
        // The light behind the camera is nowhere.
        assert!(!g.indices.contains(&2));
    }

    #[test]
    fn slices_cover_near_to_far() {
        let (view, proj) = camera();
        let g = assign(&view, &proj, 0.1, 100.0, &[]);
        let s = |z: f32| (z.ln() * g.scale + g.bias).floor();
        assert_eq!(s(0.1), 0.0);
        assert!((s(99.9) - (CLUSTER_Z as f32 - 1.0)).abs() < 1e-3);
        assert_eq!(g.cells.len(), CLUSTERS);
    }
}
