//! Runtime occlusion culling: occluder triangles are rasterized on the CPU into a small
//! reversed-Z depth buffer, then each renderer's world bounding box is tested against a
//! min-depth pyramid of that buffer. Needs no bake, so it works for moving occluders and
//! streamed-in scenes.
//!
//! The test is conservative: a box counts as hidden only when every buffer texel it covers
//! holds an occluder nearer than the box's nearest point.
//!
//! Reference: <https://docs.unity3d.com/Manual/OcclusionCulling.html>

use oxide_math::{Matrix4x4, Vector3, Vector4};

/// Buffer width in texels; the height follows the camera's aspect ratio.
pub const BUFFER_WIDTH: u32 = 320;

/// Clip-space distance in front of the near plane below which geometry is clipped.
const NEAR_EPSILON: f32 = 1e-5;

#[derive(Default)]
pub struct OcclusionBuffer {
    width: u32,
    height: u32,
    view_proj: Matrix4x4,
    /// Per texel: nearest occluder depth (reversed-Z, so larger is nearer; 0 = nothing).
    depth: Vec<f32>,
    /// Min-depth pyramid; level 0 is `depth`.
    mips: Vec<(u32, u32, Vec<f32>)>,
    /// Triangles rasterized since [`begin`](Self::begin).
    pub triangles: u32,
}

impl OcclusionBuffer {
    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Clears the buffer for a camera's view-projection (reversed-Z) and aspect ratio.
    pub fn begin(&mut self, view_proj: Matrix4x4, aspect: f32) {
        let w = BUFFER_WIDTH;
        let h = ((w as f32 / aspect.max(0.1)).round() as u32).clamp(16, 1024);
        self.width = w;
        self.height = h;
        self.view_proj = view_proj;
        self.depth.clear();
        self.depth.resize((w * h) as usize, 0.0);
        self.mips.clear();
        self.triangles = 0;
    }

    /// Rasterizes the triangles `indices` (into `vertices`) placed by `model`.
    pub fn rasterize(&mut self, model: &Matrix4x4, vertices: &[Vector3], indices: &[u32]) {
        let mvp = self.view_proj * *model;
        let clip: Vec<Vector4> = vertices.iter().map(|v| mvp * v.extend(1.0)).collect();
        for tri in indices.as_chunks::<3>().0 {
            let (Some(&a), Some(&b), Some(&c)) = (
                clip.get(tri[0] as usize),
                clip.get(tri[1] as usize),
                clip.get(tri[2] as usize),
            ) else {
                continue;
            };
            self.triangles += 1;
            self.clip_and_draw([a, b, c]);
        }
    }

    /// Rasterizes the 12 triangles of a box (a unit cube centered on the origin under `model`).
    pub fn rasterize_box(&mut self, model: &Matrix4x4) {
        let v: Vec<Vector3> = (0..8)
            .map(|i| {
                Vector3::new(
                    if i & 1 == 0 { -0.5 } else { 0.5 },
                    if i & 2 == 0 { -0.5 } else { 0.5 },
                    if i & 4 == 0 { -0.5 } else { 0.5 },
                )
            })
            .collect();
        const I: [u32; 36] = [
            0, 1, 3, 0, 3, 2, 4, 6, 7, 4, 7, 5, 0, 4, 5, 0, 5, 1, 2, 3, 7, 2, 7, 6, 0, 2, 6, 0, 6,
            4, 1, 5, 7, 1, 7, 3,
        ];
        self.rasterize(model, &v, &I);
    }

    /// Clips a clip-space triangle to the near plane (`w - z >= 0` for reversed-Z) and draws it.
    fn clip_and_draw(&mut self, t: [Vector4; 3]) {
        let d = |v: Vector4| v.w - v.z - NEAR_EPSILON * v.w.abs().max(1.0);
        let inside = t.map(|v| d(v) >= 0.0);
        if inside.iter().all(|&i| i) {
            self.draw(t[0], t[1], t[2]);
            return;
        }
        if !inside.iter().any(|&i| i) {
            return;
        }
        // Sutherland–Hodgman against one plane: at most 4 vertices.
        let mut poly: [Vector4; 4] = [Vector4::ZERO; 4];
        let mut n = 0;
        for i in 0..3 {
            let (p, q) = (t[i], t[(i + 1) % 3]);
            let (dp, dq) = (d(p), d(q));
            if dp >= 0.0 {
                poly[n] = p;
                n += 1;
            }
            if (dp >= 0.0) != (dq >= 0.0) {
                poly[n] = p + (q - p) * (dp / (dp - dq));
                n += 1;
            }
        }
        for k in 1..n.saturating_sub(1) {
            self.draw(poly[0], poly[k], poly[k + 1]);
        }
    }

    fn to_screen(&self, v: Vector4) -> Vector3 {
        let inv = 1.0 / v.w;
        Vector3::new(
            (v.x * inv * 0.5 + 0.5) * self.width as f32,
            (0.5 - v.y * inv * 0.5) * self.height as f32,
            (v.z * inv).clamp(0.0, 1.0),
        )
    }

    fn draw(&mut self, a: Vector4, b: Vector4, c: Vector4) {
        if a.w <= 0.0 || b.w <= 0.0 || c.w <= 0.0 {
            return;
        }
        let (a, mut b, mut c) = (self.to_screen(a), self.to_screen(b), self.to_screen(c));
        let edge = |p: Vector3, q: Vector3, x: f32, y: f32| {
            (q.x - p.x) * (y - p.y) - (q.y - p.y) * (x - p.x)
        };
        let mut area = edge(a, b, c.x, c.y);
        if area.abs() < 1e-8 {
            return;
        }
        // Occluders count from both sides.
        if area < 0.0 {
            std::mem::swap(&mut b, &mut c);
            area = -area;
        }
        let (w, h) = (self.width as i32, self.height as i32);
        let x0 = (a.x.min(b.x).min(c.x).floor() as i32).max(0);
        let x1 = (a.x.max(b.x).max(c.x).ceil() as i32).min(w - 1);
        let y0 = (a.y.min(b.y).min(c.y).floor() as i32).max(0);
        let y1 = (a.y.max(b.y).max(c.y).ceil() as i32).min(h - 1);
        if x0 > x1 || y0 > y1 {
            return;
        }
        let inv_area = 1.0 / area;
        // Depth is affine in screen space: z = z0 + zx·x + zy·y.
        let zx = ((b.y - c.y) * a.z + (c.y - a.y) * b.z + (a.y - b.y) * c.z) * inv_area;
        let zy = ((b.x - c.x) * a.z + (c.x - a.x) * b.z + (a.x - b.x) * c.z) * -inv_area;
        let z_at = |x: f32, y: f32| a.z + zx * (x - a.x) + zy * (y - a.y);
        for y in y0..=y1 {
            let py = y as f32 + 0.5;
            // Each edge function is affine in x along the row; intersect their x ranges.
            let (mut lo, mut hi) = (x0 as f32 + 0.5, x1 as f32 + 0.5);
            for (p, q) in [(b, c), (c, a), (a, b)] {
                let e0 = edge(p, q, 0.0, py);
                let dx = -(q.y - p.y);
                if dx.abs() < 1e-12 {
                    if e0 < 0.0 {
                        lo = f32::MAX;
                    }
                } else if dx > 0.0 {
                    lo = lo.max(-e0 / dx);
                } else {
                    hi = hi.min(-e0 / dx);
                }
            }
            if lo > hi {
                continue;
            }
            // Pixel centers x + 0.5 within [lo, hi].
            let first = ((lo - 0.5).ceil() as i32).max(x0);
            let last = ((hi - 0.5).floor() as i32).min(x1);
            if first > last {
                continue;
            }
            let row = (y * w) as usize;
            let mut z = z_at(first as f32 + 0.5, py);
            for t in &mut self.depth[row + first as usize..=row + last as usize] {
                if z > *t {
                    *t = z;
                }
                z += zx;
            }
        }
    }

    /// Builds the min-depth pyramid. Call after the last occluder, before testing.
    pub fn finish(&mut self) {
        self.mips.clear();
        let (mut w, mut h) = (self.width, self.height);
        let mut prev = self.depth.clone();
        self.mips.push((w, h, prev.clone()));
        while w > 1 || h > 1 {
            let (nw, nh) = (w.div_ceil(2), h.div_ceil(2));
            let mut next = vec![0.0f32; (nw * nh) as usize];
            for y in 0..nh {
                for x in 0..nw {
                    let (sx, sy) = (x * 2, y * 2);
                    let (sx1, sy1) = ((sx + 1).min(w - 1), (sy + 1).min(h - 1));
                    let at = |px: u32, py: u32| prev[(py * w + px) as usize];
                    next[(y * nw + x) as usize] = at(sx, sy)
                        .min(at(sx1, sy))
                        .min(at(sx, sy1))
                        .min(at(sx1, sy1));
                }
            }
            self.mips.push((nw, nh, next.clone()));
            prev = next;
            (w, h) = (nw, nh);
        }
    }

    /// Whether any part of the box `center ± extents` (local to `model`) may be visible.
    pub fn is_visible(&self, model: &Matrix4x4, center: Vector3, extents: Vector3) -> bool {
        if self.mips.is_empty() {
            return true;
        }
        let mvp = self.view_proj * *model;
        let (mut min_x, mut min_y) = (f32::MAX, f32::MAX);
        let (mut max_x, mut max_y, mut nearest) = (f32::MIN, f32::MIN, 0.0f32);
        for i in 0..8 {
            let s = Vector3::new(
                if i & 1 == 0 { -1.0 } else { 1.0 },
                if i & 2 == 0 { -1.0 } else { 1.0 },
                if i & 4 == 0 { -1.0 } else { 1.0 },
            );
            let v = mvp * (center + extents * s).extend(1.0);
            // Reaches the near plane: can't be hidden.
            if v.w <= 0.0 || v.w - v.z < 0.0 {
                return true;
            }
            let p = self.to_screen(v);
            min_x = min_x.min(p.x);
            max_x = max_x.max(p.x);
            min_y = min_y.min(p.y);
            max_y = max_y.max(p.y);
            nearest = nearest.max(p.z);
        }
        // One texel of margin covers sampling at texel centers.
        let (w, h) = (self.width as i32, self.height as i32);
        let x0 = (min_x.floor() as i32 - 1).max(0);
        let x1 = (max_x.ceil() as i32).min(w - 1);
        let y0 = (min_y.floor() as i32 - 1).max(0);
        let y1 = (max_y.ceil() as i32).min(h - 1);
        if x0 > x1 || y0 > y1 {
            // Off screen: frustum culling's job.
            return true;
        }
        let span = (x1 - x0).max(y1 - y0) as u32 + 1;
        let level = (span.div_ceil(4).max(1).next_power_of_two().trailing_zeros() as usize)
            .min(self.mips.len() - 1);
        let (mw, mh, texels) = &self.mips[level];
        let (lx0, lx1) = ((x0 >> level) as u32, ((x1 >> level) as u32).min(mw - 1));
        let (ly0, ly1) = ((y0 >> level) as u32, ((y1 >> level) as u32).min(mh - 1));
        let mut farthest = f32::MAX;
        for y in ly0..=ly1 {
            for x in lx0..=lx1 {
                farthest = farthest.min(texels[(y * mw + x) as usize]);
            }
        }
        nearest >= farthest * (1.0 - 1e-5)
    }

    /// The buffer as 8-bit grayscale rows (top row first): brighter is nearer.
    pub fn to_grayscale(&self) -> Vec<u8> {
        let max = self.depth.iter().copied().fold(0.0f32, f32::max).max(1e-9);
        self.depth
            .iter()
            .map(|&d| {
                if d <= 0.0 {
                    0
                } else {
                    (40.0 + 215.0 * (d / max).sqrt()) as u8
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxide_math::{Matrix4x4Ext, Quaternion};

    fn camera() -> Matrix4x4 {
        // Looking down +Z from the origin, 60° fov, reversed-Z like the renderer.
        let view = Matrix4x4::look_to(Vector3::ZERO, Vector3::Z, Vector3::Y);
        Matrix4x4::perspective(60f32.to_radians(), 16.0 / 9.0, 1000.0, 0.3) * view
    }

    /// A buffer with a 10×10 m wall at z = 10.
    fn wall() -> OcclusionBuffer {
        let mut b = OcclusionBuffer::default();
        b.begin(camera(), 16.0 / 9.0);
        b.rasterize_box(&Matrix4x4::trs(
            Vector3::new(0.0, 0.0, 10.0),
            Quaternion::IDENTITY,
            Vector3::new(10.0, 10.0, 0.2),
        ));
        b.finish();
        b
    }

    fn visible(b: &OcclusionBuffer, at: Vector3, half: f32) -> bool {
        b.is_visible(
            &Matrix4x4::from_translation(at),
            Vector3::ZERO,
            Vector3::splat(half),
        )
    }

    #[test]
    fn hidden_behind_wall() {
        let b = wall();
        assert_eq!(b.triangles, 12);
        assert!(!visible(&b, Vector3::new(0.0, 0.0, 20.0), 1.0));
        assert!(!visible(&b, Vector3::new(1.0, -1.0, 200.0), 3.0));
    }

    #[test]
    fn visible_in_front_beside_or_peeking() {
        let b = wall();
        // In front of the wall.
        assert!(visible(&b, Vector3::new(0.0, 0.0, 5.0), 1.0));
        // Beside it.
        assert!(visible(&b, Vector3::new(30.0, 0.0, 40.0), 1.0));
        // Behind, but sticking out past its edge.
        assert!(visible(&b, Vector3::new(9.5, 0.0, 20.0), 1.0));
        // Bigger than the wall's silhouette.
        assert!(visible(&b, Vector3::new(0.0, 0.0, 20.0), 12.0));
        // Touching the near plane.
        assert!(visible(&b, Vector3::new(0.0, 0.0, 0.0), 1.0));
        // Behind the camera is frustum culling's job, but must not report hidden wrongly.
        assert!(visible(&b, Vector3::new(0.0, 0.0, -20.0), 1.0));
    }

    #[test]
    fn slanted_occluder_depth_matches_barycentric() {
        // A floor-like triangle receding from the camera: depth varies along both axes.
        let mut b = OcclusionBuffer::default();
        b.begin(camera(), 16.0 / 9.0);
        let v = [
            Vector3::new(-20.0, -2.0, 2.0),
            Vector3::new(25.0, -1.0, 4.0),
            Vector3::new(3.0, 6.0, 60.0),
        ];
        b.rasterize(&Matrix4x4::IDENTITY, &v, &[0, 1, 2]);
        let s = v.map(|p| b.to_screen(b.view_proj * p.extend(1.0)));
        let edge = |p: Vector3, q: Vector3, x: f32, y: f32| {
            (q.x - p.x) * (y - p.y) - (q.y - p.y) * (x - p.x)
        };
        let area = edge(s[0], s[1], s[2].x, s[2].y);
        let mut checked = 0;
        for y in 0..b.height {
            for x in 0..b.width {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let w0 = edge(s[1], s[2], px, py) / area;
                let w1 = edge(s[2], s[0], px, py) / area;
                let w2 = edge(s[0], s[1], px, py) / area;
                let got = b.depth[(y * b.width + x) as usize];
                if w0 > 0.01 && w1 > 0.01 && w2 > 0.01 {
                    let want = w0 * s[0].z + w1 * s[1].z + w2 * s[2].z;
                    assert!(
                        (got - want).abs() <= want * 1e-3,
                        "({x},{y}): {got} vs {want}"
                    );
                    checked += 1;
                } else if w0 < -0.01 || w1 < -0.01 || w2 < -0.01 {
                    assert_eq!(got, 0.0, "({x},{y}) outside the triangle");
                }
            }
        }
        assert!(checked > 1000, "{checked}");
    }

    #[test]
    fn empty_buffer_hides_nothing() {
        let mut b = OcclusionBuffer::default();
        b.begin(camera(), 16.0 / 9.0);
        b.finish();
        assert!(visible(&b, Vector3::new(0.0, 0.0, 50.0), 1.0));
    }

    #[test]
    fn occluder_crossing_near_plane_is_clipped() {
        // A floor-to-ceiling slab from behind the camera to z = 10 to the left; a box behind
        // it on the left is hidden, one on the right is not.
        let mut b = OcclusionBuffer::default();
        b.begin(camera(), 16.0 / 9.0);
        b.rasterize_box(&Matrix4x4::trs(
            Vector3::new(-5.0, 0.0, 5.0),
            Quaternion::IDENTITY,
            Vector3::new(0.2, 200.0, 20.0),
        ));
        b.finish();
        assert!(!visible(&b, Vector3::new(-8.0, 0.0, 12.0), 1.0));
        assert!(visible(&b, Vector3::new(8.0, 0.0, 12.0), 1.0));
    }
}
