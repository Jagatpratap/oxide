//! P0 stand-in for a scene: a camera and a spinning triangle with reflectable settings.
//! Replaced by real GameObjects/Components in P1; the MCP tools keep the same shape.

use oxide_core::reflect_struct;
use oxide_math::{Color, Matrix4x4, Matrix4x4Ext, Quaternion, QuaternionExt, Vector3, Vector3Ext};

/// Mirrors the Unity Camera fields that matter for P0.
/// <https://docs.unity3d.com/Manual/class-Camera.html>
pub struct CameraSettings {
    pub position: Vector3,
    /// Clear color (Unity "Background", Clear Flags = Solid Color). sRGB, like Unity's picker.
    pub background_color: Color,
    /// Half the vertical view height in world units.
    pub orthographic_size: f32,
    pub near_clip_plane: f32,
    pub far_clip_plane: f32,
}

impl Default for CameraSettings {
    fn default() -> Self {
        Self {
            position: Vector3::new(0.0, 0.0, -10.0),
            // Unity's default camera background (49, 77, 121).
            background_color: Color::rgb(49.0 / 255.0, 77.0 / 255.0, 121.0 / 255.0),
            orthographic_size: 1.0,
            near_clip_plane: 0.3,
            far_clip_plane: 1000.0,
        }
    }
}

reflect_struct!(CameraSettings {
    position,
    background_color,
    orthographic_size,
    near_clip_plane,
    far_clip_plane
});

impl CameraSettings {
    /// World → clip for a camera looking down +Z (left-handed, depth 0..1).
    pub fn view_projection(&self, aspect: f32) -> Matrix4x4 {
        let h = self.orthographic_size;
        let w = h * aspect;
        let proj =
            Matrix4x4::orthographic_lh(-w, w, -h, h, self.near_clip_plane, self.far_clip_plane);
        let view = Matrix4x4::look_to_lh(self.position, Vector3::FORWARD, Vector3::UP);
        proj * view
    }
}

pub struct TriangleSettings {
    pub position: Vector3,
    /// Euler angles in degrees (Unity Transform "Rotation").
    pub rotation: Vector3,
    pub scale: Vector3,
    /// Degrees per second around Z, applied in `Update` with `Time.deltaTime`.
    pub spin_speed: f32,
    pub active: bool,
}

impl Default for TriangleSettings {
    fn default() -> Self {
        Self {
            position: Vector3::ZERO,
            rotation: Vector3::ZERO,
            scale: Vector3::ONE,
            spin_speed: 90.0,
            active: true,
        }
    }
}

reflect_struct!(TriangleSettings {
    position,
    rotation,
    scale,
    spin_speed,
    active
});

impl TriangleSettings {
    pub fn local_to_world(&self) -> Matrix4x4 {
        Matrix4x4::trs(
            self.position,
            Quaternion::euler_v(self.rotation),
            self.scale,
        )
    }
}
