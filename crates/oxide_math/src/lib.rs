//! Unity-style math for Oxide.
//!
//! Coordinate system matches Unity: **left-handed, +Y up, +Z forward, +X right**.
//! Angles in the public API are in **degrees** (as in Unity) unless a name says `_rad`.
//!
//! Vector/quaternion/matrix storage is `glam` (SIMD); Unity-named constants and helpers
//! live in the extension traits [`Vector3Ext`], [`QuaternionExt`] and [`Matrix4x4Ext`].
//!
//! Reference: <https://docs.unity3d.com/Manual/programming-math.html>

mod bounds;
mod color;
pub mod mathf;
mod quaternion;
mod ray;
mod rect;
mod vector;

pub use bounds::Bounds;
pub use color::{Color, Color32};
pub use mathf::Mathf;
pub use quaternion::{Matrix4x4Ext, QuaternionExt};
pub use ray::{Plane, Ray};
pub use rect::Rect;
pub use vector::{Vector2Ext, Vector3Ext};

pub use glam;

/// Unity `Vector2`.
pub type Vector2 = glam::Vec2;
/// Unity `Vector3`.
pub type Vector3 = glam::Vec3;
/// Unity `Vector4`.
pub type Vector4 = glam::Vec4;
/// Unity `Vector2Int`.
pub type Vector2Int = glam::IVec2;
/// Unity `Vector3Int`.
pub type Vector3Int = glam::IVec3;
/// Unity `Quaternion`.
pub type Quaternion = glam::Quat;
/// Unity `Matrix4x4` (column-major, like Unity).
pub type Matrix4x4 = glam::Mat4;

pub mod prelude {
    pub use crate::{
        Bounds, Color, Color32, Mathf, Matrix4x4, Matrix4x4Ext, Plane, Quaternion, QuaternionExt,
        Ray, Rect, Vector2, Vector2Ext, Vector2Int, Vector3, Vector3Ext, Vector3Int, Vector4,
    };
}
