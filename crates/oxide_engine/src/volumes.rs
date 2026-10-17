//! Volume stack evaluation: RenderSettings give the base values, then every active
//! [`Volume`] blends its override components in ascending priority, weighted by its weight
//! and (for local volumes) by the camera's distance to its colliders.
//! <https://docs.unity3d.com/Manual/urp/Volumes.html>

use oxide_math::{Color, Vector2, Vector3, Vector4};
use oxide_physics::{BoxCollider, CapsuleCollider, SphereCollider};
use oxide_render::{
    BloomSettings, DofMode, DofSettings, LensDistortionSettings, PostSettings,
    ScreenSpaceLensFlareSettings, TonemapMode, VignetteSettings,
};
use oxide_scene::{
    Bloom, ChannelMixer, ChromaticAberration, ColorAdjustments, ColorCurves, ColorLookup,
    Component, DepthOfField, DepthOfFieldMode, FilmGrain, GameObjectId, LensDistortion,
    LiftGammaGain, MotionBlur, PaniniProjection, RenderSettings, ScreenSpaceLensFlare,
    ScreenSpaceLensFlareResolution, ShadowsMidtonesHighlights, SplitToning, Tonemapping,
    TonemappingMode, Vignette, Volume, VolumeProfile, WhiteBalance, World,
};
use std::collections::HashMap;

/// The stack in Unity's parameter types, before conversion for the renderer.
#[derive(Clone, Debug)]
pub struct VolumeStack {
    pub post_exposure: f32,
    pub tonemapping: TonemappingMode,
    pub bloom_intensity: f32,
    pub bloom_threshold: f32,
    pub bloom_scatter: f32,
    pub bloom_tint: Color,
    pub bloom_clamp: f32,
    pub contrast: f32,
    pub color_filter: Color,
    pub hue_shift: f32,
    pub saturation: f32,
    pub temperature: f32,
    pub tint: f32,
    pub lift: Vector4,
    pub gamma: Vector4,
    pub gain: Vector4,
    pub shadows: Vector4,
    pub midtones: Vector4,
    pub highlights: Vector4,
    pub smh_ranges: [f32; 4],
    pub split_shadows: Color,
    pub split_highlights: Color,
    pub split_balance: f32,
    pub mixer: [[f32; 3]; 3],
    pub vignette_color: Color,
    pub vignette_center: Vector2,
    pub vignette_intensity: f32,
    pub vignette_smoothness: f32,
    pub vignette_rounded: bool,
    pub chromatic_aberration: f32,
    pub grain_intensity: f32,
    pub grain_response: f32,
    pub distortion_intensity: f32,
    pub distortion_multiplier: Vector2,
    pub distortion_center: Vector2,
    pub distortion_scale: f32,
    /// Depth of field and motion blur blend here too; the renderer applies them once
    /// their passes exist.
    pub depth_of_field: DepthOfField,
    pub motion_blur: MotionBlur,
    pub screen_space_lens_flare: ScreenSpaceLensFlare,
    pub bloom_dirt: Option<oxide_assets::Handle<oxide_assets::Texture2D>>,
    pub bloom_dirt_intensity: f32,
    /// Color Curves sampled (blended per sample, like Unity's TextureCurve), or None.
    pub curves: Option<oxide_render::ColorCurveSamples>,
    pub lookup: Option<oxide_assets::Handle<oxide_assets::Texture2D>>,
    pub lookup_contribution: f32,
    pub panini_distance: f32,
    pub panini_crop: f32,
}

/// The unchanged Color Curves: identity YRGB, flat 0.5 for the hue/saturation curves.
fn neutral_curves() -> oxide_render::ColorCurveSamples {
    let n = oxide_render::CURVE_SAMPLES;
    std::array::from_fn(|k| {
        std::array::from_fn(|i| {
            if k < 4 {
                i as f32 / (n - 1) as f32
            } else {
                0.5
            }
        })
    })
}

/// A curve sampled over 0..1; an empty one gives curve `k`'s neutral samples.
fn sample_curve(c: &oxide_core::AnimationCurve, k: usize) -> [f32; oxide_render::CURVE_SAMPLES] {
    let n = oxide_render::CURVE_SAMPLES;
    if c.keys.is_empty() {
        return neutral_curves()[k];
    }
    std::array::from_fn(|i| c.evaluate(i as f32 / (n - 1) as f32))
}

impl VolumeStack {
    /// Base values from the scene's RenderSettings (Oxide's always-on global defaults).
    pub fn base(rs: &RenderSettings) -> Self {
        let lgg = LiftGammaGain::default();
        let smh = ShadowsMidtonesHighlights::default();
        let st = SplitToning::default();
        let v = Vignette::default();
        let ld = LensDistortion::default();
        Self {
            post_exposure: 0.0,
            tonemapping: rs.tonemapping,
            bloom_intensity: if rs.bloom { rs.bloom_intensity } else { 0.0 },
            bloom_threshold: rs.bloom_threshold,
            bloom_scatter: 0.7,
            bloom_tint: Color::WHITE,
            bloom_clamp: 65472.0,
            contrast: 0.0,
            color_filter: Color::WHITE,
            hue_shift: 0.0,
            saturation: 0.0,
            temperature: 0.0,
            tint: 0.0,
            lift: lgg.lift.value,
            gamma: lgg.gamma.value,
            gain: lgg.gain.value,
            shadows: smh.shadows.value,
            midtones: smh.midtones.value,
            highlights: smh.highlights.value,
            smh_ranges: [
                smh.shadows_start.value,
                smh.shadows_end.value,
                smh.highlights_start.value,
                smh.highlights_end.value,
            ],
            split_shadows: st.shadows.value,
            split_highlights: st.highlights.value,
            split_balance: 0.0,
            mixer: [[100.0, 0.0, 0.0], [0.0, 100.0, 0.0], [0.0, 0.0, 100.0]],
            vignette_color: v.color.value,
            vignette_center: v.center.value,
            vignette_intensity: rs.vignette,
            vignette_smoothness: v.smoothness.value,
            vignette_rounded: false,
            chromatic_aberration: 0.0,
            grain_intensity: 0.0,
            grain_response: 0.8,
            distortion_intensity: 0.0,
            distortion_multiplier: Vector2::ONE,
            distortion_center: ld.center.value,
            distortion_scale: 1.0,
            depth_of_field: DepthOfField::default(),
            motion_blur: MotionBlur::default(),
            screen_space_lens_flare: ScreenSpaceLensFlare::default(),
            bloom_dirt: None,
            bloom_dirt_intensity: 0.0,
            curves: None,
            lookup: None,
            lookup_contribution: 0.0,
            panini_distance: 0.0,
            panini_crop: 1.0,
        }
    }

    /// Converts to the renderer's settings (linear colors, exposure multiplier).
    pub fn to_post(&self, rs: &RenderSettings, enabled: bool) -> PostSettings {
        let lin = |c: Color| {
            let l = c.linear();
            [l.r, l.g, l.b]
        };
        let srgb = |c: Color| [c.r, c.g, c.b];
        let v4 = |v: Vector4| v.to_array();
        PostSettings {
            enabled,
            exposure: rs.exposure * 2f32.powf(self.post_exposure),
            tonemapping: match self.tonemapping {
                TonemappingMode::None => TonemapMode::None,
                TonemappingMode::Neutral => TonemapMode::Neutral,
                TonemappingMode::ACES => TonemapMode::Aces,
            },
            bloom: BloomSettings {
                intensity: self.bloom_intensity.max(0.0),
                threshold: self.bloom_threshold.max(0.0),
                scatter: self.bloom_scatter,
                tint: lin(self.bloom_tint),
                clamp: self.bloom_clamp,
                dirt: self
                    .bloom_dirt
                    .as_ref()
                    .filter(|_| self.bloom_dirt_intensity > 0.0)
                    .map(|h| (h.id(), self.bloom_dirt_intensity)),
            },
            white_balance: [self.temperature, self.tint],
            contrast: self.contrast,
            saturation: self.saturation,
            hue_shift: self.hue_shift,
            color_filter: lin(self.color_filter),
            channel_mixer: self.mixer,
            lift: v4(self.lift),
            gamma: v4(self.gamma),
            gain: v4(self.gain),
            shadows: v4(self.shadows),
            midtones: v4(self.midtones),
            highlights: v4(self.highlights),
            smh_ranges: self.smh_ranges,
            split_shadows: srgb(self.split_shadows),
            split_highlights: srgb(self.split_highlights),
            split_balance: self.split_balance,
            vignette: VignetteSettings {
                color: lin(self.vignette_color),
                center: self.vignette_center.to_array(),
                intensity: self.vignette_intensity,
                smoothness: self.vignette_smoothness.max(0.01),
                rounded: self.vignette_rounded,
            },
            chromatic_aberration: self.chromatic_aberration,
            film_grain: [self.grain_intensity, self.grain_response],
            lens_distortion: LensDistortionSettings {
                intensity: self.distortion_intensity,
                multiplier: self.distortion_multiplier.to_array(),
                center: self.distortion_center.to_array(),
                scale: self.distortion_scale,
            },
            depth_of_field: {
                let d = &self.depth_of_field;
                DofSettings {
                    mode: match d.mode.value {
                        DepthOfFieldMode::Off => DofMode::Off,
                        DepthOfFieldMode::Gaussian => DofMode::Gaussian,
                        DepthOfFieldMode::Bokeh => DofMode::Bokeh,
                    },
                    gaussian_start: d.gaussian_start.value,
                    gaussian_end: d.gaussian_end.value,
                    gaussian_max_radius: d.gaussian_max_radius.value,
                    focus_distance: d.focus_distance.value,
                    focal_length: d.focal_length.value,
                    aperture: d.aperture.value,
                }
            },
            motion_blur: [
                self.motion_blur.intensity.value,
                self.motion_blur.clamp.value,
            ],
            screen_space_lens_flare: {
                let f = &self.screen_space_lens_flare;
                ScreenSpaceLensFlareSettings {
                    intensity: f.intensity.value.max(0.0),
                    tint: lin(f.tint_color.value),
                    bloom_mip: f.bloom_mip.value.min(5),
                    first_flare_intensity: f.first_flare_intensity.value,
                    secondary_flare_intensity: f.secondary_flare_intensity.value,
                    warped_flare_intensity: f.warped_flare_intensity.value,
                    warped_flare_scale: f.warped_flare_scale.value.to_array(),
                    samples: f.samples.value.clamp(1, 3),
                    sample_dimmer: f.sample_dimmer.value,
                    vignette_effect: f.vignette_effect.value,
                    starting_position: f.starting_position.value,
                    scale: f.scale.value,
                    streaks_intensity: f.streaks_intensity.value.max(0.0),
                    streaks_length: f.streaks_length.value,
                    streaks_orientation: f.streaks_orientation.value,
                    streaks_threshold: f.streaks_threshold.value,
                    resolution_mip: match f.resolution.value {
                        ScreenSpaceLensFlareResolution::Full
                        | ScreenSpaceLensFlareResolution::Half => 0,
                        ScreenSpaceLensFlareResolution::Quarter => 1,
                        ScreenSpaceLensFlareResolution::Eighth => 2,
                    },
                    chromatic_aberration: f.chromatic_abberation_intensity.value,
                }
            },
            curves: self.curves.map(std::sync::Arc::new),
            lookup: self
                .lookup
                .as_ref()
                .filter(|_| self.lookup_contribution > 0.0)
                .map(|h| (h.id(), self.lookup_contribution.min(1.0))),
            panini: [
                self.panini_distance.clamp(0.0, 1.0),
                self.panini_crop.clamp(0.0, 1.0),
            ],
        }
    }
}

/// Distance from `p` to the nearest collider on `id` (0 inside), or `None` without
/// colliders.
fn collider_distance(world: &World, id: GameObjectId, p: Vector3) -> Option<f32> {
    let m = world.local_to_world(id);
    let local = m.inverse().transform_point3(p);
    let scale = world.lossy_scale(id).abs().max_element().max(1e-6);
    let mut best: Option<f32> = None;
    let mut take = |d: f32| best = Some(best.map_or(d, |b: f32| b.min(d)));
    if let Some(b) = world.get_component::<BoxCollider>(id) {
        let q = (local - b.center).abs() - b.size.abs() * 0.5;
        take(q.max(Vector3::ZERO).length() * scale);
    }
    if let Some(s) = world.get_component::<SphereCollider>(id) {
        take(((local - s.center).length() - s.radius).max(0.0) * scale);
    }
    if let Some(c) = world.get_component::<CapsuleCollider>(id) {
        // Bounding sphere of the capsule: close enough for volume blending.
        let r = c.radius.max(c.height * 0.5);
        take(((local - c.center).length() - r).max(0.0) * scale);
    }
    best
}

/// Override `T` of each volume: an enabled component on the volume's object, else the
/// one in its Volume Profile (Unity's local overrides take precedence over the profile).
fn overrides<T: Component + Clone>(
    world: &World,
    volumes: &[(GameObjectId, f32)],
    from_profile: fn(&VolumeProfile) -> Option<&T>,
) -> HashMap<GameObjectId, T> {
    let assets = world.resource::<oxide_assets::AssetServer>();
    volumes
        .iter()
        .filter_map(|&(id, _)| {
            let own = world
                .query::<T>()
                .find(|(o, _)| *o == id)
                .map(|(_, c)| c.clone());
            let shared = || {
                let v = world.get_component::<Volume>(id)?;
                let profile = assets.data.get(v.profile.as_ref()?)?;
                from_profile(profile).cloned()
            };
            own.or_else(shared).map(|c| (id, c))
        })
        .collect()
}

/// Evaluates the stack at `position` (the camera: Unity's default volume trigger).
pub fn evaluate(world: &World, rs: &RenderSettings, position: Vector3) -> VolumeStack {
    let mut stack = VolumeStack::base(rs);
    let mut volumes: Vec<(f32, GameObjectId, f32)> = Vec::new();
    for (id, v) in world.query::<Volume>() {
        let w = if v.is_global {
            v.weight
        } else {
            match collider_distance(world, id, position) {
                Some(d) if d <= 0.0 => v.weight,
                Some(d) if v.blend_distance > 0.0 && d < v.blend_distance => {
                    v.weight * (1.0 - d / v.blend_distance)
                }
                _ => 0.0,
            }
        };
        if w > 0.0 {
            volumes.push((v.priority, id, w.clamp(0.0, 1.0)));
        }
    }
    volumes.sort_by(|a, b| a.0.total_cmp(&b.0));
    let order: Vec<(GameObjectId, f32)> = volumes.iter().map(|&(_, id, w)| (id, w)).collect();

    let bloom = overrides::<Bloom>(world, &order, |p| p.bloom.as_ref());
    let tonemap = overrides::<Tonemapping>(world, &order, |p| p.tonemapping.as_ref());
    let adjust = overrides::<ColorAdjustments>(world, &order, |p| p.color_adjustments.as_ref());
    let wb = overrides::<WhiteBalance>(world, &order, |p| p.white_balance.as_ref());
    let lgg = overrides::<LiftGammaGain>(world, &order, |p| p.lift_gamma_gain.as_ref());
    let smh = overrides::<ShadowsMidtonesHighlights>(world, &order, |p| {
        p.shadows_midtones_highlights.as_ref()
    });
    let split = overrides::<SplitToning>(world, &order, |p| p.split_toning.as_ref());
    let mixer = overrides::<ChannelMixer>(world, &order, |p| p.channel_mixer.as_ref());
    let vignette = overrides::<Vignette>(world, &order, |p| p.vignette.as_ref());
    let ca = overrides::<ChromaticAberration>(world, &order, |p| p.chromatic_aberration.as_ref());
    let grain = overrides::<FilmGrain>(world, &order, |p| p.film_grain.as_ref());
    let distortion = overrides::<LensDistortion>(world, &order, |p| p.lens_distortion.as_ref());
    let dof = overrides::<DepthOfField>(world, &order, |p| p.depth_of_field.as_ref());
    let blur = overrides::<MotionBlur>(world, &order, |p| p.motion_blur.as_ref());
    let sslf =
        overrides::<ScreenSpaceLensFlare>(world, &order, |p| p.screen_space_lens_flare.as_ref());
    let curves = overrides::<ColorCurves>(world, &order, |p| p.color_curves.as_ref());
    let lookup = overrides::<ColorLookup>(world, &order, |p| p.color_lookup.as_ref());
    let panini = overrides::<PaniniProjection>(world, &order, |p| p.panini_projection.as_ref());

    let s = &mut stack;
    for (id, w) in order {
        if let Some(b) = bloom.get(&id) {
            b.intensity.blend(&mut s.bloom_intensity, w);
            b.threshold.blend(&mut s.bloom_threshold, w);
            b.scatter.blend(&mut s.bloom_scatter, w);
            b.tint.blend(&mut s.bloom_tint, w);
            b.clamp.blend(&mut s.bloom_clamp, w);
            b.dirt_texture.blend(&mut s.bloom_dirt, w);
            b.dirt_intensity.blend(&mut s.bloom_dirt_intensity, w);
        }
        if let Some(c) = curves.get(&id) {
            let params = [
                &c.master,
                &c.red,
                &c.green,
                &c.blue,
                &c.hue_vs_hue,
                &c.hue_vs_sat,
                &c.sat_vs_sat,
                &c.lum_vs_sat,
            ];
            if params.iter().any(|p| p.override_state) {
                let dst = s.curves.get_or_insert_with(neutral_curves);
                for (k, p) in params.iter().enumerate() {
                    if p.override_state {
                        let samples = sample_curve(&p.value, k);
                        for (d, v) in dst[k].iter_mut().zip(samples) {
                            *d += (v - *d) * w;
                        }
                    }
                }
            }
        }
        if let Some(c) = lookup.get(&id) {
            c.texture.blend(&mut s.lookup, w);
            c.contribution.blend(&mut s.lookup_contribution, w);
        }
        if let Some(c) = panini.get(&id) {
            c.distance.blend(&mut s.panini_distance, w);
            c.crop_to_fit.blend(&mut s.panini_crop, w);
        }
        if let Some(t) = tonemap.get(&id) {
            t.mode.blend(&mut s.tonemapping, w);
        }
        if let Some(c) = adjust.get(&id) {
            c.post_exposure.blend(&mut s.post_exposure, w);
            c.contrast.blend(&mut s.contrast, w);
            c.color_filter.blend(&mut s.color_filter, w);
            c.hue_shift.blend(&mut s.hue_shift, w);
            c.saturation.blend(&mut s.saturation, w);
        }
        if let Some(c) = wb.get(&id) {
            c.temperature.blend(&mut s.temperature, w);
            c.tint.blend(&mut s.tint, w);
        }
        if let Some(c) = lgg.get(&id) {
            c.lift.blend(&mut s.lift, w);
            c.gamma.blend(&mut s.gamma, w);
            c.gain.blend(&mut s.gain, w);
        }
        if let Some(c) = smh.get(&id) {
            c.shadows.blend(&mut s.shadows, w);
            c.midtones.blend(&mut s.midtones, w);
            c.highlights.blend(&mut s.highlights, w);
            c.shadows_start.blend(&mut s.smh_ranges[0], w);
            c.shadows_end.blend(&mut s.smh_ranges[1], w);
            c.highlights_start.blend(&mut s.smh_ranges[2], w);
            c.highlights_end.blend(&mut s.smh_ranges[3], w);
        }
        if let Some(c) = split.get(&id) {
            c.shadows.blend(&mut s.split_shadows, w);
            c.highlights.blend(&mut s.split_highlights, w);
            c.balance.blend(&mut s.split_balance, w);
        }
        if let Some(c) = mixer.get(&id) {
            let rows = [
                [&c.red_out_red_in, &c.red_out_green_in, &c.red_out_blue_in],
                [
                    &c.green_out_red_in,
                    &c.green_out_green_in,
                    &c.green_out_blue_in,
                ],
                [
                    &c.blue_out_red_in,
                    &c.blue_out_green_in,
                    &c.blue_out_blue_in,
                ],
            ];
            for (r, row) in rows.iter().enumerate() {
                for (i, p) in row.iter().enumerate() {
                    p.blend(&mut s.mixer[r][i], w);
                }
            }
        }
        if let Some(c) = vignette.get(&id) {
            c.color.blend(&mut s.vignette_color, w);
            c.center.blend(&mut s.vignette_center, w);
            c.intensity.blend(&mut s.vignette_intensity, w);
            c.smoothness.blend(&mut s.vignette_smoothness, w);
            c.rounded.blend(&mut s.vignette_rounded, w);
        }
        if let Some(c) = ca.get(&id) {
            c.intensity.blend(&mut s.chromatic_aberration, w);
        }
        if let Some(c) = grain.get(&id) {
            c.intensity.blend(&mut s.grain_intensity, w);
            c.response.blend(&mut s.grain_response, w);
        }
        if let Some(c) = distortion.get(&id) {
            c.intensity.blend(&mut s.distortion_intensity, w);
            c.x_multiplier.blend(&mut s.distortion_multiplier.x, w);
            c.y_multiplier.blend(&mut s.distortion_multiplier.y, w);
            c.center.blend(&mut s.distortion_center, w);
            c.scale.blend(&mut s.distortion_scale, w);
        }
        if let Some(c) = dof.get(&id) {
            let d = &mut s.depth_of_field;
            for (src, dst) in [
                (&c.gaussian_start, &mut d.gaussian_start),
                (&c.gaussian_end, &mut d.gaussian_end),
                (&c.gaussian_max_radius, &mut d.gaussian_max_radius),
                (&c.focus_distance, &mut d.focus_distance),
                (&c.focal_length, &mut d.focal_length),
                (&c.aperture, &mut d.aperture),
            ] {
                src.blend(&mut dst.value, w);
            }
            c.mode.blend(&mut d.mode.value, w);
        }
        if let Some(c) = blur.get(&id) {
            c.intensity.blend(&mut s.motion_blur.intensity.value, w);
            c.clamp.blend(&mut s.motion_blur.clamp.value, w);
        }
        if let Some(c) = sslf.get(&id) {
            let d = &mut s.screen_space_lens_flare;
            for (src, dst) in [
                (&c.intensity, &mut d.intensity),
                (&c.first_flare_intensity, &mut d.first_flare_intensity),
                (
                    &c.secondary_flare_intensity,
                    &mut d.secondary_flare_intensity,
                ),
                (&c.warped_flare_intensity, &mut d.warped_flare_intensity),
                (&c.sample_dimmer, &mut d.sample_dimmer),
                (&c.vignette_effect, &mut d.vignette_effect),
                (&c.starting_position, &mut d.starting_position),
                (&c.scale, &mut d.scale),
                (&c.streaks_intensity, &mut d.streaks_intensity),
                (&c.streaks_length, &mut d.streaks_length),
                (&c.streaks_orientation, &mut d.streaks_orientation),
                (&c.streaks_threshold, &mut d.streaks_threshold),
                (
                    &c.chromatic_abberation_intensity,
                    &mut d.chromatic_abberation_intensity,
                ),
            ] {
                src.blend(&mut dst.value, w);
            }
            c.tint_color.blend(&mut d.tint_color.value, w);
            c.bloom_mip.blend(&mut d.bloom_mip.value, w);
            c.warped_flare_scale
                .blend(&mut d.warped_flare_scale.value, w);
            c.samples.blend(&mut d.samples.value, w);
            c.resolution.blend(&mut d.resolution.value, w);
        }
    }
    stack
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world() -> World {
        let mut w = World::new();
        crate::init_world(&mut w, std::env::temp_dir());
        w
    }

    #[test]
    fn volume_profile_assets_apply_and_local_overrides_win() {
        let mut w = world();
        let rs = RenderSettings::default();
        // A Volume Profile asset with Bloom and Color Adjustments, saved and reloaded.
        let mut bloom = Bloom::default();
        bloom.intensity.set(3.0);
        let mut adj = ColorAdjustments::default();
        adj.saturation.set(-80.0);
        let profile = VolumeProfile {
            bloom: Some(bloom),
            color_adjustments: Some(adj),
            ..Default::default()
        };
        let path = format!("Assets/Test{}.volumeProfile", std::process::id());
        let assets = w.resource_mut::<oxide_assets::AssetServer>();
        assets.save_data(&path, Box::new(profile)).unwrap();
        assets.data.remove(&path);
        assets.ensure_loaded(&path).unwrap();
        let v = w.create("Global");
        w.add_component(
            v,
            Volume {
                profile: Some(oxide_assets::Handle::from_path(path.clone())),
                ..Default::default()
            },
        );
        let s = evaluate(&w, &rs, Vector3::ZERO);
        assert_eq!(s.bloom_intensity, 3.0, "bloom from the profile");
        assert_eq!(s.saturation, -80.0, "colour adjustments from the profile");

        // An override component on the Volume's object replaces the profile's.
        let mut local = ColorAdjustments::default();
        local.saturation.set(25.0);
        w.add_component(v, local);
        let s = evaluate(&w, &rs, Vector3::ZERO);
        assert_eq!(s.saturation, 25.0, "local override wins");
        assert_eq!(
            s.bloom_intensity, 3.0,
            "other profile overrides still apply"
        );
        let _ = std::fs::remove_file(std::env::temp_dir().join(&path));
    }

    #[test]
    fn global_volumes_blend_by_priority_and_weight() {
        let mut w = world();
        let rs = RenderSettings::default();
        let a = w.create("Global A");
        w.add_component(a, Volume::default());
        let mut adj = ColorAdjustments::default();
        adj.saturation.set(-100.0);
        adj.contrast.set(20.0);
        w.add_component(a, adj);

        let b = w.create("Global B");
        w.add_component(
            b,
            Volume {
                priority: 1.0,
                weight: 0.5,
                ..Default::default()
            },
        );
        let mut adj = ColorAdjustments::default();
        adj.saturation.set(0.0);
        w.add_component(b, adj);

        let s = evaluate(&w, &rs, Vector3::ZERO);
        // A sets -100; B (higher priority, weight 0.5) pulls halfway back to 0.
        assert_eq!(s.saturation, -50.0);
        // Contrast is only overridden by A.
        assert_eq!(s.contrast, 20.0);
        // Not overridden anywhere: the RenderSettings base.
        assert_eq!(s.bloom_intensity, rs.bloom_intensity);
    }

    #[test]
    fn local_volume_blends_in_by_distance() {
        let mut w = world();
        let rs = RenderSettings::default();
        let v = w.create("Local");
        w.add_component(
            v,
            Volume {
                is_global: false,
                blend_distance: 10.0,
                ..Default::default()
            },
        );
        w.add_component(
            v,
            BoxCollider {
                size: Vector3::splat(4.0),
                ..Default::default()
            },
        );
        let mut ca = ChromaticAberration::default();
        ca.intensity.set(1.0);
        w.add_component(v, ca);

        let at = |x: f32| evaluate(&w, &rs, Vector3::new(x, 0.0, 0.0)).chromatic_aberration;
        assert_eq!(at(0.0), 1.0, "inside the box");
        assert!(
            (at(7.0) - 0.5).abs() < 1e-4,
            "5 m outside, 10 m blend: {}",
            at(7.0)
        );
        assert_eq!(at(20.0), 0.0, "beyond the blend distance");
    }

    #[test]
    fn color_curves_lookup_and_panini_blend_into_post() {
        let mut w = world();
        let rs = RenderSettings::default();
        assert!(
            evaluate(&w, &rs, Vector3::ZERO)
                .to_post(&rs, true)
                .curves
                .is_none()
        );
        let v = w.create("Global Volume");
        w.add_component(
            v,
            Volume {
                weight: 0.5,
                ..Default::default()
            },
        );
        let mut curves = ColorCurves::default();
        curves
            .master
            .set(oxide_core::AnimationCurve::linear(0.0, 1.0, 1.0, 0.0));
        w.add_component(v, curves);
        let mut lookup = ColorLookup::default();
        lookup.texture.set(Some(oxide_assets::Handle::from_path(
            "Assets/lut.png".to_owned(),
        )));
        lookup.contribution.set(1.0);
        w.add_component(v, lookup);
        let mut panini = PaniniProjection::default();
        panini.distance.set(1.0);
        w.add_component(v, panini);
        let post = evaluate(&w, &rs, Vector3::ZERO).to_post(&rs, true);
        let c = post.curves.expect("curves");
        // Halfway (weight 0.5) between identity and inverted: flat 0.5 for Master.
        assert!(
            c[0].iter().all(|v| (v - 0.5).abs() < 1e-3),
            "{:?}",
            &c[0][..4]
        );
        // Unset curves stay neutral.
        assert!((c[1][0]).abs() < 1e-6 && (c[4][7] - 0.5).abs() < 1e-6);
        assert_eq!(post.lookup.map(|l| l.1), Some(0.5));
        assert_eq!(post.panini, [0.5, 1.0]);
    }
}
