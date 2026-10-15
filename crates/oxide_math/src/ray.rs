//! Unity `Ray` and `Plane`. References: <https://docs.unity3d.com/ScriptReference/Ray.html>,
//! <https://docs.unity3d.com/ScriptReference/Plane.html>

use crate::Vector3;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ray {
    pub origin: Vector3,
    /// Always normalized.
    pub direction: Vector3,
}

impl Ray {
    pub fn new(origin: Vector3, direction: Vector3) -> Self {
        Self {
            origin,
            direction: direction.normalize_or_zero(),
        }
    }

    pub fn get_point(&self, distance: f32) -> Vector3 {
        self.origin + self.direction * distance
    }
}

/// Plane `dot(normal, p) + distance = 0`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Plane {
    pub normal: Vector3,
    pub distance: f32,
}

impl Plane {
    pub fn new(in_normal: Vector3, in_point: Vector3) -> Self {
        let normal = in_normal.normalize_or_zero();
        Self {
            normal,
            distance: -normal.dot(in_point),
        }
    }

    pub fn get_distance_to_point(&self, point: Vector3) -> f32 {
        self.normal.dot(point) + self.distance
    }

    pub fn get_side(&self, point: Vector3) -> bool {
        self.get_distance_to_point(point) > 0.0
    }

    pub fn closest_point_on_plane(&self, point: Vector3) -> Vector3 {
        point - self.normal * self.get_distance_to_point(point)
    }

    /// Distance along the ray where it hits the plane, if it does (Unity `Plane.Raycast`).
    pub fn raycast(&self, ray: &Ray) -> Option<f32> {
        let denom = ray.direction.dot(self.normal);
        if denom.abs() < f32::EPSILON {
            return None;
        }
        let t = -(ray.origin.dot(self.normal) + self.distance) / denom;
        (t > 0.0).then_some(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Vector3Ext;

    #[test]
    fn plane_raycast() {
        let ground = Plane::new(Vector3::UP, Vector3::ZERO);
        let ray = Ray::new(Vector3::new(0.0, 10.0, 0.0), Vector3::DOWN);
        assert_eq!(ground.raycast(&ray), Some(10.0));
        assert_eq!(ray.get_point(10.0), Vector3::ZERO);
    }
}
