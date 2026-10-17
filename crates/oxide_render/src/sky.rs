//! CPU version of the procedural sky (mirrors `sky_color` in shaders/common.wgsl), for the
//! lightmapper: rays that escape the scene pick up this radiance.

use crate::frame::{Environment, SunLight};
use oxide_math::Vector3;

#[derive(Clone, Debug)]
pub struct SkyModel {
    zenith: Vector3,
    horizon: Vector3,
    ground: Vector3,
    /// Direction toward the sun (`None` without a sun).
    to_sun: Option<Vector3>,
    sun_color: Vector3,
    exposure: f32,
    /// A textured skybox replaces the procedural sky.
    texture: Option<std::sync::Arc<SkyTexture>>,
}

/// A skybox's faces as linear colour, with its tint, exposure and rotation.
#[derive(Debug)]
struct SkyTexture {
    size: u32,
    /// Cube face order, row-major texels.
    faces: Vec<Vec<Vector3>>,
    scale: Vector3,
    rotation: f32,
}

impl SkyTexture {
    fn sample(&self, dir: Vector3) -> Vector3 {
        let (s, c) = self.rotation.sin_cos();
        let d = Vector3::new(c * dir.x - s * dir.z, dir.y, s * dir.x + c * dir.z);
        let a = d.abs();
        // Face and (a, b) in -1..1, inverting `CubemapFace::direction`.
        let (face, u, v) = if a.x >= a.y && a.x >= a.z {
            if d.x > 0.0 {
                (0, -d.z / a.x, -d.y / a.x)
            } else {
                (1, d.z / a.x, -d.y / a.x)
            }
        } else if a.y >= a.z {
            if d.y > 0.0 {
                (2, d.x / a.y, d.z / a.y)
            } else {
                (3, d.x / a.y, -d.z / a.y)
            }
        } else if d.z > 0.0 {
            (4, d.x / a.z, -d.y / a.z)
        } else {
            (5, -d.x / a.z, -d.y / a.z)
        };
        let px = |t: f32| (((t + 1.0) * 0.5 * self.size as f32) as u32).min(self.size - 1);
        let texel = self.faces[face][(px(v) * self.size + px(u)) as usize];
        texel * self.scale
    }
}

fn mix(a: Vector3, b: Vector3, t: f32) -> Vector3 {
    a + (b - a) * t
}

impl SkyModel {
    /// The sky the renderer draws for `env` and `sun` (same constants as the renderer).
    pub fn new(env: &Environment, sun: Option<&SunLight>) -> Self {
        let tint = env.sky_tint.linear();
        let ground = env.ground_color.linear();
        // sRGB 0.5 grey (linear 0.214) is the neutral tint.
        let nt = Vector3::new(tint.r, tint.g, tint.b) / 0.214;
        Self {
            zenith: Vector3::new(0.075, 0.17, 0.46) * nt,
            horizon: Vector3::new(0.36, 0.44, 0.56) * nt.powf(0.5),
            ground: Vector3::new(ground.r, ground.g, ground.b),
            to_sun: sun.map(|s| -s.direction.normalize_or(Vector3::NEG_Y)),
            sun_color: sun.map_or(Vector3::ZERO, |s| Vector3::from_array(s.color)),
            exposure: env.sky_exposure,
            texture: None,
        }
    }

    /// With a textured skybox's faces (e.g. from `skybox_faces`), as the renderer draws it.
    pub fn with_skybox(
        mut self,
        faces: &[oxide_assets::Texture2D],
        sb: &crate::frame::Skybox,
    ) -> Self {
        let size = faces.first().map_or(0, |f| f.width);
        if faces.len() != 6
            || size == 0
            || faces
                .iter()
                .any(|f| f.width != size || f.height != size || f.mips.is_empty())
        {
            return self;
        }
        let decode = |v: u8, srgb: bool| {
            let x = v as f32 / 255.0;
            if !srgb {
                x
            } else if x <= 0.04045 {
                x / 12.92
            } else {
                ((x + 0.055) / 1.055).powf(2.4)
            }
        };
        let t = sb.tint.linear();
        self.texture = Some(std::sync::Arc::new(SkyTexture {
            size,
            faces: faces
                .iter()
                .map(|f| {
                    f.mips[0]
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|p| {
                            Vector3::new(
                                decode(p[0], f.srgb),
                                decode(p[1], f.srgb),
                                decode(p[2], f.srgb),
                            )
                        })
                        .collect()
                })
                .collect(),
            // Unity: texel × tint × colour-space double × exposure.
            scale: Vector3::new(t.r, t.g, t.b) * 4.594_794 * sb.exposure,
            rotation: sb.rotation.to_radians(),
        }));
        self
    }

    /// Linear radiance seen looking along `dir` (without the sun disk itself).
    pub fn radiance(&self, dir: Vector3) -> Vector3 {
        let dir = dir.normalize_or(Vector3::Y);
        if let Some(t) = &self.texture {
            return t.sample(dir);
        }
        let sun_h = self.to_sun.map_or(0.6, |l| l.y);
        let day = (sun_h * 3.0 + 0.35).clamp(0.05, 1.0);
        let sunset = (1.0 - sun_h.abs() * 3.0).clamp(0.0, 1.0).powi(2);
        let zenith = self.zenith * day;
        let horizon = mix(self.horizon, Vector3::new(1.0, 0.55, 0.25), sunset * 0.7) * day;
        let ground = self.ground * day * 0.6;
        let y = dir.y;
        let mut c = if y >= 0.0 {
            mix(horizon, zenith, y.powf(0.45))
        } else {
            mix(horizon, ground, (-y * 4.0).powf(0.6).clamp(0.0, 1.0))
        };
        if let Some(l) = self.to_sun {
            let cosang = dir.dot(l).max(0.0);
            let glow = cosang.powf(12.0) * 0.35 + cosang.powf(200.0) * 0.8;
            c += self.sun_color
                * glow
                * mix(Vector3::ONE, Vector3::new(1.0, 0.6, 0.3), sunset)
                * 0.5;
        }
        c * self.exposure
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sky_is_bluer_above_and_darker_below() {
        let sky = SkyModel::new(&Environment::default(), None);
        let up = sky.radiance(Vector3::Y);
        let down = sky.radiance(Vector3::NEG_Y);
        assert!(up.z > up.x, "zenith is blue: {up:?}");
        assert!(down.length() < up.length());
    }
}
