//! Unity `Bounds` (axis-aligned bounding box). Reference: <https://docs.unity3d.com/ScriptReference/Bounds.html>

use crate::{Ray, Vector3};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    pub center: Vector3,
    /// Half the size.
    pub extents: Vector3,
}

impl Bounds {
    pub fn new(center: Vector3, size: Vector3) -> Self {
        Self {
            center,
            extents: size * 0.5,
        }
    }

    pub fn from_min_max(min: Vector3, max: Vector3) -> Self {
        Self {
            center: (min + max) * 0.5,
            extents: (max - min) * 0.5,
        }
    }

    pub fn size(&self) -> Vector3 {
        self.extents * 2.0
    }

    pub fn min(&self) -> Vector3 {
        self.center - self.extents
    }

    pub fn max(&self) -> Vector3 {
        self.center + self.extents
    }

    pub fn set_min_max(&mut self, min: Vector3, max: Vector3) {
        *self = Self::from_min_max(min, max);
    }

    pub fn contains(&self, point: Vector3) -> bool {
        point.cmpge(self.min()).all() && point.cmple(self.max()).all()
    }

    /// Grows the bounds to include `point`.
    pub fn encapsulate(&mut self, point: Vector3) {
        self.set_min_max(self.min().min(point), self.max().max(point));
    }

    pub fn encapsulate_bounds(&mut self, other: &Bounds) {
        self.set_min_max(self.min().min(other.min()), self.max().max(other.max()));
    }

    /// Grows each side by `amount / 2` (Unity semantics: `amount` is added to the size).
    pub fn expand(&mut self, amount: f32) {
        self.extents += Vector3::splat(amount * 0.5);
    }

    pub fn intersects(&self, other: &Bounds) -> bool {
        self.min().cmple(other.max()).all() && self.max().cmpge(other.min()).all()
    }

    pub fn closest_point(&self, point: Vector3) -> Vector3 {
        point.clamp(self.min(), self.max())
    }

    pub fn sqr_distance(&self, point: Vector3) -> f32 {
        (self.closest_point(point) - point).length_squared()
    }

    /// Distance along the ray to the first hit, if any (slab test).
    pub fn intersect_ray(&self, ray: &Ray) -> Option<f32> {
        let inv = ray.direction.recip();
        let t1 = (self.min() - ray.origin) * inv;
        let t2 = (self.max() - ray.origin) * inv;
        let t_near = t1.min(t2).max_element();
        let t_far = t1.max(t2).min_element();
        if t_far < 0.0 || t_near > t_far {
            None
        } else {
            Some(t_near.max(0.0))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Vector3Ext;

    #[test]
    fn basics() {
        let mut b = Bounds::new(Vector3::ZERO, Vector3::splat(2.0));
        assert!(b.contains(Vector3::splat(1.0)));
        assert!(!b.contains(Vector3::splat(1.1)));
        b.encapsulate(Vector3::new(3.0, 0.0, 0.0));
        assert_eq!(b.max().x, 3.0);
        assert_eq!(b.min().x, -1.0);
    }

    #[test]
    fn ray_hit() {
        let b = Bounds::new(Vector3::new(0.0, 0.0, 5.0), Vector3::splat(2.0));
        let hit = b.intersect_ray(&Ray::new(Vector3::ZERO, Vector3::FORWARD));
        assert_eq!(hit, Some(4.0));
        assert_eq!(
            b.intersect_ray(&Ray::new(Vector3::ZERO, Vector3::BACK)),
            None
        );
    }
}
