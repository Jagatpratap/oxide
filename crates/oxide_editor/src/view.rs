//! Offscreen view textures, the Scene view camera, and world ↔ screen projection.

use oxide_engine::Engine;
use oxide_math::{Matrix4x4, QuaternionExt, Ray, Vector3, Vector4};
use oxide_render::{CameraView, ClearMode, ProjectionKind, wgpu};

pub const VIEW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// A texture the renderer draws a view into and egui shows as an image.
#[derive(Default)]
pub struct ViewTarget {
    view: Option<wgpu::TextureView>,
    id: Option<egui::TextureId>,
    /// Texture size in pixels.
    pub size: [u32; 2],
    /// Where the view is on screen this frame (points); `None` when its tab is hidden.
    pub rect: Option<egui::Rect>,
}

impl ViewTarget {
    /// Makes sure the texture is `w`×`h` pixels and returns its egui id.
    pub fn texture_id(&mut self, engine: &mut Engine, w: u32, h: u32) -> egui::TextureId {
        let (w, h) = (w.max(1), h.max(1));
        if let (Some(id), true) = (self.id, self.size == [w, h]) {
            return id;
        }
        let device = &engine.gpu.device;
        let view = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("editor view"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: VIEW_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let renderer = &mut engine.gui.renderer;
        let id = match self.id {
            Some(id) => {
                renderer.update_egui_texture_from_wgpu_texture(
                    device,
                    &view,
                    wgpu::FilterMode::Linear,
                    id,
                );
                id
            }
            None => renderer.register_native_texture(device, &view, wgpu::FilterMode::Linear),
        };
        self.view = Some(view);
        self.id = Some(id);
        self.size = [w, h];
        id
    }

    pub fn view(&self) -> Option<&wgpu::TextureView> {
        self.view.as_ref()
    }
}

/// Unity's Scene view camera: orbits a pivot at `distance`.
#[derive(Clone, Debug)]
pub struct SceneCamera {
    pub pivot: Vector3,
    /// Degrees.
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub fov: f32,
    pub orthographic: bool,
    /// Fly speed in units per second.
    pub speed: f32,
}

impl Default for SceneCamera {
    fn default() -> Self {
        Self {
            pivot: Vector3::new(0.0, 1.0, 0.0),
            yaw: 30.0,
            pitch: 25.0,
            distance: 14.0,
            fov: 60.0,
            orthographic: false,
            speed: 10.0,
        }
    }
}

impl SceneCamera {
    pub fn rotation(&self) -> oxide_math::Quaternion {
        oxide_math::Quaternion::euler(self.pitch, self.yaw, 0.0)
    }

    pub fn forward(&self) -> Vector3 {
        self.rotation() * Vector3::Z
    }

    pub fn position(&self) -> Vector3 {
        self.pivot - self.forward() * self.distance
    }

    /// Moves the camera to `position` looking at `target` (which becomes the pivot).
    pub fn look_at(&mut self, position: Vector3, target: Vector3) {
        let d = target - position;
        let len = d.length().max(0.01);
        let dir = d / len;
        self.pitch = (-dir.y).clamp(-1.0, 1.0).asin().to_degrees();
        self.yaw = dir.x.atan2(dir.z).to_degrees();
        self.pivot = target;
        self.distance = len;
    }

    /// Keeps the position, changes where the camera looks (fly mode).
    pub fn rotate_in_place(&mut self, dyaw: f32, dpitch: f32) {
        let pos = self.position();
        self.yaw += dyaw;
        self.pitch = (self.pitch + dpitch).clamp(-89.9, 89.9);
        self.pivot = pos + self.forward() * self.distance;
    }

    /// Frames a sphere (Unity's F key).
    pub fn frame(&mut self, center: Vector3, radius: f32) {
        self.pivot = center;
        let half = (self.fov * 0.5).to_radians();
        self.distance = (radius.max(0.25) / half.sin()).max(0.5);
    }

    pub fn camera_view(&self) -> CameraView {
        let near = (self.distance * 0.002).clamp(0.01, 1.0);
        CameraView {
            position: self.position(),
            rotation: self.rotation(),
            projection: if self.orthographic {
                ProjectionKind::Orthographic {
                    size: self.distance * (self.fov * 0.5).to_radians().tan(),
                }
            } else {
                ProjectionKind::Perspective { fov: self.fov }
            },
            near: if self.orthographic {
                -self.distance * 50.0
            } else {
                near
            },
            far: (self.distance * 1000.0).max(5000.0),
            viewport: [0.0, 0.0, 1.0, 1.0],
            clear: ClearMode::Sky,
            culling_mask: u32::MAX,
            occlusion_culling: true,
        }
    }
}

/// Projects between world space and a view rect on screen (egui points).
pub struct Projector {
    pub rect: egui::Rect,
    view_proj: Matrix4x4,
    inv: Matrix4x4,
    pub cam_pos: Vector3,
    pub cam_forward: Vector3,
    /// Vertical half-extent per unit depth (perspective) or the ortho half-height.
    half_h: f32,
    ortho: bool,
}

impl Projector {
    pub fn new(cam: &CameraView, rect: egui::Rect) -> Self {
        let aspect = rect.width() / rect.height().max(1.0);
        let view_proj = cam.projection(aspect) * cam.view();
        let (half_h, ortho) = match cam.projection {
            ProjectionKind::Perspective { fov } => ((fov * 0.5).to_radians().tan(), false),
            ProjectionKind::Orthographic { size } => (size, true),
        };
        Self {
            rect,
            view_proj,
            inv: view_proj.inverse(),
            cam_pos: cam.position,
            cam_forward: cam.rotation * Vector3::Z,
            half_h,
            ortho,
        }
    }

    fn clip(&self, p: Vector3) -> Vector4 {
        self.view_proj * p.extend(1.0)
    }

    fn to_screen(&self, c: Vector4) -> egui::Pos2 {
        let n = c.truncate() / c.w;
        egui::pos2(
            self.rect.left() + (n.x * 0.5 + 0.5) * self.rect.width(),
            self.rect.top() + (0.5 - n.y * 0.5) * self.rect.height(),
        )
    }

    /// Screen position of a world point, or `None` behind the camera.
    pub fn project(&self, p: Vector3) -> Option<egui::Pos2> {
        let c = self.clip(p);
        (c.w > 1e-4).then(|| self.to_screen(c))
    }

    /// A world-space segment clipped to the front of the camera.
    pub fn segment(&self, a: Vector3, b: Vector3) -> Option<[egui::Pos2; 2]> {
        let (mut ca, mut cb) = (self.clip(a), self.clip(b));
        const EPS: f32 = 1e-3;
        if ca.w < EPS && cb.w < EPS {
            return None;
        }
        if ca.w < EPS {
            let t = (EPS - ca.w) / (cb.w - ca.w);
            ca = ca.lerp(cb, t);
        } else if cb.w < EPS {
            let t = (EPS - cb.w) / (ca.w - cb.w);
            cb = cb.lerp(ca, t);
        }
        Some([self.to_screen(ca), self.to_screen(cb)])
    }

    /// The world-space ray under a screen position.
    pub fn ray(&self, p: egui::Pos2) -> Ray {
        let x = (p.x - self.rect.left()) / self.rect.width() * 2.0 - 1.0;
        let y = 1.0 - (p.y - self.rect.top()) / self.rect.height() * 2.0;
        // Reversed Z: depth 1 is the near plane.
        let near = self.inv * Vector4::new(x, y, 1.0, 1.0);
        let far = self.inv * Vector4::new(x, y, 0.5, 1.0);
        let (near, far) = (near.truncate() / near.w, far.truncate() / far.w);
        Ray::new(near, (far - near).normalize_or(self.cam_forward))
    }

    /// World size that covers `pixels` on screen at `p` (for constant-size handles).
    pub fn world_per_pixels(&self, p: Vector3, pixels: f32) -> f32 {
        let per_pixel = if self.ortho {
            self.half_h * 2.0 / self.rect.height().max(1.0)
        } else {
            let depth = (p - self.cam_pos).dot(self.cam_forward).max(0.01);
            depth * self.half_h * 2.0 / self.rect.height().max(1.0)
        };
        per_pixel * pixels
    }

    /// Direction from `p` toward the camera (for front/back facing tests).
    pub fn to_camera(&self, p: Vector3) -> Vector3 {
        if self.ortho {
            -self.cam_forward
        } else {
            (self.cam_pos - p).normalize_or(-self.cam_forward)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_and_ray_agree() {
        let mut cam = SceneCamera::default();
        cam.look_at(Vector3::new(3.0, 4.0, -8.0), Vector3::new(0.0, 1.0, 0.0));
        let rect = egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(800.0, 450.0));
        let p = Projector::new(&cam.camera_view(), rect);
        // The pivot is at the center of the view.
        let c = p.project(cam.pivot).unwrap();
        assert!((c - rect.center()).length() < 0.5, "{c:?}");
        // A ray through a projected point passes through the point.
        let target = Vector3::new(1.5, 0.5, 2.0);
        let s = p.project(target).unwrap();
        let r = p.ray(s);
        let t = (target - r.origin).dot(r.direction);
        let closest = r.origin + r.direction * t;
        assert!((closest - target).length() < 1e-2, "{closest:?}");
        // Points behind the camera don't project.
        assert!(p.project(cam.position() - cam.forward() * 5.0).is_none());
    }

    #[test]
    fn look_at_points_forward() {
        let mut cam = SceneCamera::default();
        cam.look_at(Vector3::new(0.0, 0.0, -10.0), Vector3::ZERO);
        assert!((cam.forward() - Vector3::Z).length() < 1e-4);
        assert!((cam.position() - Vector3::new(0.0, 0.0, -10.0)).length() < 1e-3);
    }
}
