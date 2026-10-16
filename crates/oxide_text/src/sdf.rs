//! Signed distance fields from glyph outlines (TextMeshPro's SDF render mode).
//!
//! Curves are flattened to line segments. Each segment writes exact distances into the
//! pixels within `spread` of it (pixels farther away saturate anyway), and a scanline
//! pass decides inside/outside with the non-zero winding rule. Values are stored as
//! `0.5 + distance / (2 × spread)`: 0.5 on the outline, 1 deep inside, 0 far outside.

/// A point in bitmap pixels, y down.
pub type P = [f32; 2];

/// Flattens quadratic and cubic curves into line segments (pixel space).
#[derive(Default)]
pub struct Segments {
    pub segs: Vec<(P, P)>,
}

impl Segments {
    pub fn line(&mut self, a: P, b: P) {
        if a != b {
            self.segs.push((a, b));
        }
    }

    pub fn quad(&mut self, a: P, c: P, b: P) {
        let n = steps(a, c, b, b);
        let mut prev = a;
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            let p = [
                u * u * a[0] + 2.0 * u * t * c[0] + t * t * b[0],
                u * u * a[1] + 2.0 * u * t * c[1] + t * t * b[1],
            ];
            self.line(prev, p);
            prev = p;
        }
    }

    pub fn cubic(&mut self, a: P, c1: P, c2: P, b: P) {
        let n = steps(a, c1, c2, b);
        let mut prev = a;
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            let (w0, w1, w2, w3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            let p = [
                w0 * a[0] + w1 * c1[0] + w2 * c2[0] + w3 * b[0],
                w0 * a[1] + w1 * c1[1] + w2 * c2[1] + w3 * b[1],
            ];
            self.line(prev, p);
            prev = p;
        }
    }
}

/// Segment count for a curve: about one per 2 px of control polygon length.
fn steps(a: P, b: P, c: P, d: P) -> u32 {
    let len = |p: P, q: P| ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2)).sqrt();
    ((len(a, b) + len(b, c) + len(c, d)) / 2.0)
        .ceil()
        .clamp(2.0, 32.0) as u32
}

fn dist_to_segment(p: P, a: P, b: P) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (x, y) = (a[0] + dx * t - p[0], a[1] + dy * t - p[1]);
    (x * x + y * y).sqrt()
}

/// The SDF of `segs` in a `w`×`h` bitmap (one byte per pixel, rows top to bottom).
pub fn render(segs: &[(P, P)], w: usize, h: usize, spread: f32) -> Vec<u8> {
    let spread = spread.max(0.5);
    // Unsigned distance, limited to the spread.
    let mut dist = vec![spread; w * h];
    for &(a, b) in segs {
        let x0 = (a[0].min(b[0]) - spread).floor().max(0.0) as usize;
        let x1 = ((a[0].max(b[0]) + spread).ceil() as usize).min(w);
        let y0 = (a[1].min(b[1]) - spread).floor().max(0.0) as usize;
        let y1 = ((a[1].max(b[1]) + spread).ceil() as usize).min(h);
        for y in y0..y1 {
            for x in x0..x1 {
                let d = dist_to_segment([x as f32 + 0.5, y as f32 + 0.5], a, b);
                let cell = &mut dist[y * w + x];
                if d < *cell {
                    *cell = d;
                }
            }
        }
    }
    // Inside test per row (non-zero winding along a horizontal ray).
    let mut out = vec![0u8; w * h];
    let mut crossings: Vec<(f32, i32)> = Vec::new();
    for y in 0..h {
        let py = y as f32 + 0.5;
        crossings.clear();
        for &(a, b) in segs {
            let (ay, by) = (a[1], b[1]);
            if (ay <= py && by > py) || (by <= py && ay > py) {
                let t = (py - ay) / (by - ay);
                let x = a[0] + (b[0] - a[0]) * t;
                crossings.push((x, if by > ay { 1 } else { -1 }));
            }
        }
        crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut k = 0;
        let mut winding = 0;
        for x in 0..w {
            let px = x as f32 + 0.5;
            while k < crossings.len() && crossings[k].0 <= px {
                winding += crossings[k].1;
                k += 1;
            }
            let d = dist[y * w + x];
            let signed = if winding != 0 { d } else { -d };
            let v = 0.5 + signed / (2.0 * spread);
            out[y * w + x] = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_is_inside_and_edges_are_half() {
        // A 10×10 square from (5,5) to (15,15) in a 20×20 bitmap, spread 4.
        let mut s = Segments::default();
        s.line([5.0, 5.0], [15.0, 5.0]);
        s.line([15.0, 5.0], [15.0, 15.0]);
        s.line([15.0, 15.0], [5.0, 15.0]);
        s.line([5.0, 15.0], [5.0, 5.0]);
        let img = render(&s.segs, 20, 20, 4.0);
        let at = |x: usize, y: usize| img[y * 20 + x];
        assert_eq!(at(10, 10), 255, "deep inside");
        assert_eq!(at(0, 0), 0, "far outside");
        // Pixel centre 0.5 px inside the left edge: 0.5 + 0.5 / 8.
        assert_eq!(at(5, 10), (255.0 * (0.5 + 0.5 / 8.0) + 0.5) as u8);
        assert_eq!(at(4, 10), (255.0 * (0.5 - 0.5 / 8.0) + 0.5) as u8);
    }

    #[test]
    fn winding_order_does_not_matter_and_holes_are_outside() {
        // Outer square clockwise, inner square counter-clockwise (a ring).
        let mut s = Segments::default();
        let sq = |s: &mut Segments, a: f32, b: f32, rev: bool| {
            let mut pts = [[a, a], [b, a], [b, b], [a, b]];
            if rev {
                pts.reverse();
            }
            for i in 0..4 {
                s.line(pts[i], pts[(i + 1) % 4]);
            }
        };
        sq(&mut s, 2.0, 30.0, false);
        sq(&mut s, 12.0, 20.0, true);
        let img = render(&s.segs, 32, 32, 3.0);
        assert!(img[16 * 32 + 16] < 10, "the hole is outside");
        assert!(img[16 * 32 + 6] > 245, "the ring is inside");
    }

    #[test]
    fn curves_flatten_close_to_the_true_shape() {
        let mut s = Segments::default();
        s.quad([0.0, 0.0], [10.0, 20.0], [20.0, 0.0]);
        // The quad's apex is at (10, 10).
        let apex = s.segs.iter().map(|(a, _)| a[1]).fold(0.0f32, f32::max);
        assert!((apex - 10.0).abs() < 0.2, "{apex}");
    }
}
