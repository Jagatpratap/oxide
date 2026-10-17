//! Lens Flare (SRP) extraction: Lens Flare components become render sources, with their
//! data asset's elements expanded (multiple elements, distributions, gradients) and curves
//! sampled into lookup tables. Components on a Light follow it (directional lights flare
//! from their direction, spot lights only inside their cone, colours follow the light).

use oxide_assets::AssetServer;
use oxide_core::AnimationCurve;
use oxide_math::Vector2;
use oxide_scene::{
    LensFlareBlendMode, LensFlareComponent, LensFlareData, LensFlareDataElement,
    LensFlareDistribution, LensFlareType, Light, LightType, World,
};

/// Loads the data assets of every Lens Flare (call once per frame, before extraction).
pub fn preload(world: &mut World) {
    let handles: Vec<_> = world
        .query::<LensFlareComponent>()
        .filter_map(|(_, c)| c.lens_flare_data.clone())
        .collect();
    if handles.is_empty() {
        return;
    }
    let assets = world.resource_mut::<AssetServer>();
    for h in handles {
        let _ = assets.data_asset(&h);
    }
}

/// Every active Lens Flare with loaded data, as render sources.
pub fn extract(world: &World, out: &mut Vec<oxide_render::LensFlareSource>) {
    let assets = world.resource::<AssetServer>();
    for (id, c) in world.query::<LensFlareComponent>() {
        if !world.get(id).is_some_and(|o| o.active_in_hierarchy()) {
            continue;
        }
        let Some(data) = c.lens_flare_data.as_ref().and_then(|h| assets.data.get(h)) else {
            continue;
        };
        let light = world
            .get_component::<Light>(id)
            .filter(|l| l.intensity > 0.0);
        let (mut position, mut directional, mut color, mut spot) =
            (world.position(id), false, [1.0; 3], None);
        if let Some(l) = light {
            let lc = l.color.linear();
            color = [lc.r, lc.g, lc.b];
            match l.light_type {
                LightType::Directional => {
                    // Toward the light: opposite its forward.
                    position = -world.forward(id);
                    directional = true;
                }
                LightType::Spot => {
                    spot = Some((
                        world.forward(id),
                        (l.spot_angle * 0.5).to_radians().cos(),
                        (l.inner_spot_angle * 0.5).to_radians().cos(),
                    ));
                }
                LightType::Point | LightType::Rectangle | LightType::Disc => {}
            }
        }
        out.push(to_source(c, data, position, directional, color, spot));
    }
}

/// The copies an element draws: `(index fraction 0..1, random 0..1 × 4)` per copy.
fn copies(e: &LensFlareDataElement) -> Vec<(f32, [f32; 4])> {
    let n = if e.allow_multiple_element {
        e.count.max(1)
    } else {
        1
    };
    // Small deterministic hash (Unity reseeds per element too).
    let mut state = e.seed.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    let mut rand = || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        (state >> 8) as f32 / (1u32 << 24) as f32
    };
    (0..n)
        .map(|i| {
            let t = if n > 1 {
                i as f32 / (n - 1) as f32
            } else {
                0.0
            };
            (t, [rand(), rand(), rand(), rand()])
        })
        .collect()
}

fn lut(c: &AnimationCurve) -> oxide_render::FlareLut {
    let mut out = [0.0; oxide_render::FLARE_LUT];
    for (i, v) in out.iter_mut().enumerate() {
        *v = c.evaluate(i as f32 / (oxide_render::FLARE_LUT - 1) as f32);
    }
    out
}

/// Expands a component's data and curves into a render source at `position`.
pub fn to_source(
    c: &LensFlareComponent,
    data: &LensFlareData,
    position: oxide_math::Vector3,
    directional: bool,
    light_color: [f32; 3],
    spot: Option<(oxide_math::Vector3, f32, f32)>,
) -> oxide_render::LensFlareSource {
    use oxide_render::{FlareBlend, FlareDistortion, FlareElement, FlareShape};
    let mut elements = Vec::new();
    for e in data.elements.iter().filter(|e| e.visible) {
        let shape = match e.flare_type {
            LensFlareType::Image => FlareShape::Image,
            LensFlareType::Circle => FlareShape::Circle,
            LensFlareType::Polygon => FlareShape::Polygon,
            LensFlareType::Ring => FlareShape::Ring,
        };
        let blend = match e.blend_mode {
            LensFlareBlendMode::Additive => FlareBlend::Additive,
            LensFlareBlendMode::Screen => FlareBlend::Screen,
            LensFlareBlendMode::Premultiply => FlareBlend::Premultiply,
            LensFlareBlendMode::Lerp => FlareBlend::Lerp,
        };
        let distortion = e.enable_radial_distortion.then(|| FlareDistortion {
            target_size: (e.target_size_distortion * e.uniform_scale).to_array(),
            curve: lut(&e.distortion_curve),
            relative_to_center: e.distortion_relative_to_center,
        });
        let multiple = e.allow_multiple_element && e.count > 1;
        for (t, r) in copies(e) {
            let (mut pos, mut offset, mut scale, mut rot, mut intensity) = (
                e.position,
                e.position_offset,
                e.uniform_scale,
                e.rotation,
                e.intensity,
            );
            let mut tint = e.tint;
            if multiple {
                match e.distribution {
                    LensFlareDistribution::Uniform => {
                        pos += e.length_spread * t;
                        tint = tint * e.color_gradient.evaluate(t);
                    }
                    LensFlareDistribution::Curve => {
                        pos += e.length_spread * e.position_curve.evaluate(t);
                        scale *= e.scale_curve.evaluate(t);
                        tint = tint * e.color_gradient.evaluate(t);
                    }
                    LensFlareDistribution::Random => {
                        // Position Variation: x along the ray, y as a screen offset.
                        pos += e.length_spread * r[0]
                            + (r[3] * 2.0 - 1.0) * e.position_variation.x * 0.25;
                        offset += Vector2::new(r[1] * 2.0 - 1.0, r[2] * 2.0 - 1.0)
                            * e.position_variation.y;
                        scale *= 1.0 - r[2] * e.scale_variation.clamp(0.0, 1.0);
                        rot += (r[3] * 2.0 - 1.0) * e.rotation_variation;
                        intensity *= 1.0 - r[1] * e.intensity_variation.clamp(0.0, 1.0);
                        tint = tint * e.color_gradient.evaluate(r[0]);
                    }
                }
            }
            let col = tint;
            elements.push(FlareElement {
                texture: e.texture.as_ref().map(|h| h.id()),
                shape,
                blend,
                color: [
                    col.r * intensity,
                    col.g * intensity,
                    col.b * intensity,
                    col.a,
                ],
                modulate_by_light_color: e.modulate_by_light_color,
                position: pos,
                position_offset: offset.to_array(),
                angular_offset: e.angular_offset,
                translation_scale: e.translation_scale.to_array(),
                size: (e.size_xy * scale).to_array(),
                preserve_aspect_ratio: e.preserve_aspect_ratio,
                rotation: rot,
                auto_rotate: e.auto_rotate,
                shape_params: [
                    e.fall_off,
                    e.edge_offset,
                    e.sdf_roundness,
                    e.side_count.clamp(3, 32) as f32,
                ],
                inverse: e.inverse_sdf,
                distortion: distortion.clone(),
            });
        }
    }
    oxide_render::LensFlareSource {
        position,
        directional,
        light_color,
        intensity: c.intensity,
        scale: c.scale,
        spot: if c.attenuation_by_light_shape {
            spot
        } else {
            None
        },
        max_attenuation_distance: c.max_attenuation_distance,
        distance_attenuation: lut(&c.distance_attenuation_curve),
        max_attenuation_scale: c.max_attenuation_scale,
        scale_by_distance: lut(&c.scale_by_distance_curve),
        radial_screen_attenuation: lut(&c.radial_screen_attenuation_curve),
        occlusion: c.use_occlusion.then_some(oxide_render::FlareOcclusion {
            radius: c.occlusion_radius,
            samples: c.sample_count.clamp(1, 64),
            offset: c.occlusion_offset,
        }),
        allow_off_screen: c.allow_off_screen,
        elements,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiple_elements_expand_and_hidden_ones_are_skipped() {
        let mut data = LensFlareData {
            elements: vec![LensFlareDataElement {
                allow_multiple_element: true,
                count: 4,
                length_spread: 3.0,
                ..Default::default()
            }],
        };
        data.elements.push(LensFlareDataElement {
            visible: false,
            ..Default::default()
        });
        let c = LensFlareComponent::default();
        let s = to_source(&c, &data, oxide_math::Vector3::ZERO, false, [1.0; 3], None);
        assert_eq!(s.elements.len(), 4);
        let pos: Vec<f32> = s.elements.iter().map(|e| e.position).collect();
        assert_eq!(pos, vec![0.0, 1.0, 2.0, 3.0]);

        // Random distribution is deterministic per seed.
        data.elements[0].distribution = LensFlareDistribution::Random;
        let a = to_source(&c, &data, oxide_math::Vector3::ZERO, false, [1.0; 3], None);
        let b = to_source(&c, &data, oxide_math::Vector3::ZERO, false, [1.0; 3], None);
        assert_eq!(a, b);
        data.elements[0].seed = 7;
        let d = to_source(&c, &data, oxide_math::Vector3::ZERO, false, [1.0; 3], None);
        assert_ne!(a, d);
    }
}
