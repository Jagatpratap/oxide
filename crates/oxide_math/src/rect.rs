//! Unity `Rect`. Reference: <https://docs.unity3d.com/ScriptReference/Rect.html>

use crate::Vector2;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn min_max_rect(xmin: f32, ymin: f32, xmax: f32, ymax: f32) -> Self {
        Self::new(xmin, ymin, xmax - xmin, ymax - ymin)
    }

    pub fn position(&self) -> Vector2 {
        Vector2::new(self.x, self.y)
    }

    pub fn size(&self) -> Vector2 {
        Vector2::new(self.width, self.height)
    }

    pub fn center(&self) -> Vector2 {
        self.position() + self.size() * 0.5
    }

    pub fn min(&self) -> Vector2 {
        self.position().min(self.position() + self.size())
    }

    pub fn max(&self) -> Vector2 {
        self.position().max(self.position() + self.size())
    }

    /// Unity semantics: inclusive of min, exclusive of max.
    pub fn contains(&self, p: Vector2) -> bool {
        let (min, max) = (self.min(), self.max());
        p.x >= min.x && p.x < max.x && p.y >= min.y && p.y < max.y
    }

    pub fn overlaps(&self, other: &Rect) -> bool {
        let (a0, a1, b0, b1) = (self.min(), self.max(), other.min(), other.max());
        b1.x > a0.x && b0.x < a1.x && b1.y > a0.y && b0.y < a1.y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_and_overlaps() {
        let r = Rect::new(0.0, 0.0, 10.0, 5.0);
        assert!(r.contains(Vector2::new(0.0, 0.0)));
        assert!(!r.contains(Vector2::new(10.0, 1.0)));
        assert!(r.overlaps(&Rect::new(9.0, 4.0, 3.0, 3.0)));
        assert!(!r.overlaps(&Rect::new(10.0, 0.0, 3.0, 3.0)));
        assert_eq!(r.center(), Vector2::new(5.0, 2.5));
    }
}
