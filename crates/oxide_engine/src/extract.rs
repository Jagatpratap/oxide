//! Scene → [`RenderFrame`]: cameras, lights, mesh draws, skinning matrices, environment
//! and debug lines, read from the World each frame.

use oxide_assets::{AssetId, AssetServer, DEFAULT_MATERIAL};
use oxide_math::{Color, Matrix4x4, Matrix4x4Ext};
use oxide_render::{
    CameraView, ClearMode, DrawItem, Environment, FogKind, ProbeRefresh, ProjectionKind,
    PunctualLight, ReflectionProbeView, RenderFrame, ShadowSettings, SunLight,
};
use oxide_scene::{
    AntialiasingMode, Camera, CameraClearFlags, DebugDraw, FogMode, LODGroup, Light, LightShadows,
    LightType, MeshFilter, MeshRenderer, MsaaQuality, OcclusionPortal, Projection, QualityLevel,
    QualitySettings, ReflectionProbe, ReflectionProbeClearFlags, ReflectionProbeMode,
    ReflectionProbeRefreshMode, RenderSettings, ShadowCastingMode, ShadowQuality,
    SkinnedMeshRenderer, StaticEditorFlags, World,
};

pub fn camera_view(world: &World, id: oxide_scene::GameObjectId, c: &Camera) -> CameraView {
    let bg = c.background_color.linear();
    CameraView {
        position: world.position(id),
        rotation: world.rotation(id),
        projection: match c.projection {
            Projection::Perspective => ProjectionKind::Perspective {
                fov: c.field_of_view,
            },
            Projection::Orthographic => ProjectionKind::Orthographic {
                size: c.orthographic_size,
            },
        },
        near: c.near_clip_plane.max(0.001),
        far: c.far_clip_plane.max(c.near_clip_plane + 0.01),
        viewport: c.viewport_rect.to_array(),
        clear: match c.clear_flags {
            CameraClearFlags::Skybox => ClearMode::Sky,
            CameraClearFlags::SolidColor => ClearMode::Color(Color::new(bg.r, bg.g, bg.b, 1.0)),
            CameraClearFlags::DepthOnly => ClearMode::DepthOnly,
            CameraClearFlags::Nothing => ClearMode::Nothing,
        },
        culling_mask: c.culling_mask,
        occlusion_culling: c.use_occlusion_culling,
        antialiasing: oxide_render::AntiAliasingSettings {
            mode: match c.antialiasing {
                AntialiasingMode::None => oxide_render::AntiAliasing::None,
                AntialiasingMode::FastApproximateAntialiasing => oxide_render::AntiAliasing::Fxaa,
                AntialiasingMode::TemporalAntiAliasing => oxide_render::AntiAliasing::Taa,
            },
            allow_msaa: c.allow_msaa,
            taa_base_blend_factor: c.taa_base_blend_factor,
            taa_jitter_scale: c.taa_jitter_scale,
            taa_variance_clamp_scale: c.taa_variance_clamp_scale,
            history_id: id.to_bits(),
        },
    }
}

/// Screen cameras sorted by `depth` (Unity draws lower depth first). Cameras with a
/// target texture are in [`texture_cameras`] instead.
/// Which display the game renders (Unity's Game view "Display 1-8"; players show Display 1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ActiveDisplay(pub u32);

/// Cameras that render to the screen on the active display, in depth order.
pub fn cameras(world: &World) -> Vec<CameraView> {
    let display = world.try_resource::<ActiveDisplay>().map_or(0, |d| d.0);
    let mut cams: Vec<_> = world
        .query::<Camera>()
        .filter(|(_, c)| c.target_texture.is_none() && c.target_display == display)
        .map(|(id, c)| (c.depth, camera_view(world, id, c)))
        .collect();
    cams.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    cams.into_iter().map(|(_, v)| v).collect()
}

/// Renderer features from the Full Screen Pass and Render Objects components.
pub fn renderer_features(world: &World) -> Vec<oxide_render::RendererFeature> {
    use oxide_render::{DepthTest, RenderPassEvent as E, RendererFeature};
    let event = |e: oxide_scene::RenderPassEvent| match e {
        oxide_scene::RenderPassEvent::AfterRenderingOpaques => E::AfterRenderingOpaques,
        oxide_scene::RenderPassEvent::AfterRenderingTransparents => E::AfterRenderingTransparents,
        oxide_scene::RenderPassEvent::BeforeRenderingPostProcessing => {
            E::BeforeRenderingPostProcessing
        }
        oxide_scene::RenderPassEvent::AfterRenderingPostProcessing => {
            E::AfterRenderingPostProcessing
        }
    };
    let mut out = Vec::new();
    for (_, f) in world.query::<oxide_scene::FullScreenPassRendererFeature>() {
        let Some(shader) = &f.shader else { continue };
        let e = event(f.injection_point);
        // Full-screen passes need a whole image: before or after post-processing.
        let e = match e {
            E::AfterRenderingOpaques | E::AfterRenderingTransparents => {
                E::BeforeRenderingPostProcessing
            }
            other => other,
        };
        out.push(RendererFeature::FullScreen {
            event: e,
            shader: shader.id(),
            params: [f.params0.to_array(), f.params1.to_array()],
        });
    }
    for (_, f) in world.query::<oxide_scene::RenderObjectsRendererFeature>() {
        out.push(RendererFeature::RenderObjects {
            event: match event(f.event) {
                E::AfterRenderingOpaques => E::AfterRenderingOpaques,
                _ => E::AfterRenderingTransparents,
            },
            layer_mask: f.layer_mask,
            override_material: f.override_material.as_ref().map(|m| m.id()),
            depth_test: match f.depth_test {
                oxide_scene::CompareFunction::LessEqual => DepthTest::LessEqual,
                oxide_scene::CompareFunction::Greater => DepthTest::Greater,
                oxide_scene::CompareFunction::Always => DepthTest::Always,
            },
            write_depth: f.write_depth,
        });
    }
    out
}

/// Cameras rendering into Render Textures, by depth.
pub fn texture_cameras(world: &World) -> Vec<(AssetId, CameraView)> {
    let mut cams: Vec<_> = world
        .query::<Camera>()
        .filter_map(|(id, c)| {
            let rt = c.target_texture.as_ref()?.id();
            let mut v = camera_view(world, id, c);
            // A target texture is the whole viewport.
            v.viewport = [0.0, 0.0, 1.0, 1.0];
            Some((c.depth, (rt, v)))
        })
        .collect();
    cams.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    cams.into_iter().map(|(_, v)| v).collect()
}

/// Scene environment with the active quality level's shadow distance and MSAA.
pub fn environment(rs: &RenderSettings, q: &QualityLevel) -> Environment {
    Environment {
        sky_tint: rs.sky_tint,
        ground_color: rs.ground_color,
        sky_exposure: rs.sky_exposure,
        sun_size: rs.sun_size,
        ambient_intensity: rs.ambient_intensity,
        reflection_intensity: rs.reflection_intensity,
        fog: rs.fog.then_some((
            match rs.fog_mode {
                FogMode::Linear => FogKind::Linear {
                    start: rs.fog_start_distance,
                    end: rs.fog_end_distance,
                },
                FogMode::Exponential => FogKind::Exponential {
                    density: rs.fog_density,
                },
                FogMode::ExponentialSquared => FogKind::ExponentialSquared {
                    density: rs.fog_density,
                },
            },
            rs.fog_color,
        )),
        post: crate::volumes::VolumeStack::base(rs).to_post(rs, true),
        ssao: rs.ssao.then_some(oxide_render::SsaoSettings {
            intensity: rs.ssao_intensity,
            radius: rs.ssao_radius,
            direct_lighting_strength: rs.ssao_direct_lighting_strength,
            falloff_distance: rs.ssao_falloff_distance,
            samples: rs.ssao_samples,
        }),
        shadow_distance: q.shadow_distance,
        msaa_samples: match q.msaa {
            MsaaQuality::Disabled => 1,
            MsaaQuality::Msaa2x => 2,
            MsaaQuality::Msaa4x => 4,
            MsaaQuality::Msaa8x => 8,
        },
    }
}

/// Builds the frame. `cameras` overrides the scene cameras (editor Scene view, screenshots).
pub fn extract(world: &World, cameras_override: Option<Vec<CameraView>>) -> RenderFrame {
    let assets = world.resource::<AssetServer>();
    let settings = world
        .try_resource::<RenderSettings>()
        .cloned()
        .unwrap_or_default();
    let quality = QualitySettings::of(world);
    let matrices = world.all_world_matrices();
    let world_matrix =
        |id: oxide_scene::GameObjectId| matrices.get(id.index() as usize).copied().flatten();

    // Post-processing follows the first scene camera's setting (the Scene view always has it).
    let post_enabled = cameras_override.is_some()
        || world
            .query::<Camera>()
            .filter(|(_, c)| c.target_texture.is_none())
            .min_by(|a, b| a.1.depth.total_cmp(&b.1.depth))
            .is_none_or(|(_, c)| c.post_processing);
    let mut frame = RenderFrame {
        cameras: cameras_override.unwrap_or_else(|| cameras(world)),
        environment: environment(&settings, &quality),
        time: world.resource::<oxide_core::Time>().time(),
        frame_index: world.resource::<oxide_core::Time>().frame_count(),
        texture_cameras: texture_cameras(world),
        lightmaps: settings.lightmaps.iter().map(|h| h.id()).collect(),
        renderer_features: renderer_features(world),
        ..Default::default()
    };

    // Lights: the sun is RenderSettings.sun or the brightest directional light.
    let mut sun: Option<(f32, SunLight)> = None;
    for (id, l) in world.query::<Light>() {
        // Baked lights only exist in lightmaps and light probes.
        if l.lightmap_bake_type == oxide_scene::LightmapBakeType::Baked {
            continue;
        }
        let c = l.color.linear();
        let color = [c.r * l.intensity, c.g * l.intensity, c.b * l.intensity];
        match l.light_type {
            LightType::Directional => {
                let preferred = settings.sun == Some(id);
                let score = if preferred {
                    f32::INFINITY
                } else {
                    l.intensity
                };
                if sun.as_ref().is_none_or(|(s, _)| score > *s) {
                    sun = Some((
                        score,
                        SunLight {
                            direction: world.forward(id),
                            color,
                            shadows: shadow_settings(l, &quality),
                        },
                    ));
                }
            }
            LightType::Point | LightType::Spot => frame.lights.push(PunctualLight {
                position: world.position(id),
                range: l.range,
                color,
                spot: (l.light_type == LightType::Spot)
                    .then(|| (world.forward(id), l.spot_angle, l.inner_spot_angle)),
                shadow: shadow_settings(l, &quality),
            }),
        }
    }
    frame.sun = sun.map(|(_, s)| s);
    // The active Render Pipeline Asset (Graphics Settings, or the Quality level's).
    if let Some(p) =
        oxide_scene::GraphicsSettings::current(world).and_then(|h| assets.data.get(&h).cloned())
    {
        use oxide_scene::{PipelineMsaa, RenderingPath as P};
        frame.rendering_path = match p.rendering_path {
            P::Forward => oxide_render::RenderingPath::Forward,
            P::ForwardPlus => oxide_render::RenderingPath::ForwardPlus,
            P::Deferred => oxide_render::RenderingPath::Deferred,
        };
        match p.msaa {
            PipelineMsaa::QualityLevel => {}
            PipelineMsaa::Disabled => frame.environment.msaa_samples = 1,
            PipelineMsaa::Msaa2x => frame.environment.msaa_samples = 2,
            PipelineMsaa::Msaa4x => frame.environment.msaa_samples = 4,
            PipelineMsaa::Msaa8x => frame.environment.msaa_samples = 8,
        }
        frame.opaque_layer_mask = (p.opaque_layer_mask != u32::MAX).then_some(p.opaque_layer_mask);
        frame.transparent_layer_mask =
            (p.transparent_layer_mask != u32::MAX).then_some(p.transparent_layer_mask);
        if !p.post_processing {
            frame.environment.post = oxide_render::PostSettings::neutral();
        }
        // A custom render pipeline from the scripts (Unity SRP).
        if !p.custom_pipeline.is_empty() {
            match world
                .try_resource::<crate::render_pipeline::RenderPipelines>()
                .and_then(|rp| rp.run(&p.custom_pipeline, world, &frame.cameras))
            {
                Some(steps) => frame.custom_steps = Some(steps),
                None => warn_once(&format!(
                    "no render pipeline `{}` is registered (scripts! {{ ...; pipelines: {} }}); \
                     using the Universal Render Pipeline",
                    p.custom_pipeline, p.custom_pipeline
                )),
            }
        }
    }
    // Volumes are evaluated at the first camera (Unity's default volume trigger).
    let trigger = frame
        .cameras
        .first()
        .map_or(oxide_math::Vector3::ZERO, |c| c.position);
    frame.environment.post =
        crate::volumes::evaluate(world, &settings, trigger).to_post(&settings, post_enabled);
    frame.reflection_probes = reflection_probes(world);

    // Terrains choose chunk LODs for the first camera.
    let cam_pos = frame
        .cameras
        .first()
        .map_or(oxide_math::Vector3::ZERO, |c| c.position);
    oxide_terrain::extract(world, cam_pos, &mut frame.terrain_draws, &mut frame.draws);
    oxide_terrain::extract_details(world, cam_pos, &mut frame.grass, &mut frame.draws);
    oxide_particles::extract(world, &mut frame.particles);
    oxide_vfx::extract(world, &mut frame.vfx);
    crate::sprites2d::extract(world, &mut frame.sprites);
    crate::lens_flares::extract(world, &mut frame.lens_flares);
    oxide_text::components::extract(world, &mut frame.texts);

    let default_mat = AssetId::from_path(DEFAULT_MATERIAL);
    let layer_of = |id| world.get(id).map_or(0, |o| o.layer);
    let flags_of = |id| {
        world
            .get(id)
            .map_or(StaticEditorFlags::NOTHING, |o| o.static_flags)
    };
    // Occluder Static objects hide what is behind them; Occludee Static ones, and dynamic
    // ones that allow it, can be hidden (Unity's rules).
    let occludee = |id, allow_dynamic: bool| {
        allow_dynamic || flags_of(id).contains(StaticEditorFlags::OCCLUDEE_STATIC)
    };
    let probe_static = |id| flags_of(id).contains(StaticEditorFlags::REFLECTION_PROBE_STATIC);
    for (id, p) in world.query::<OcclusionPortal>() {
        if !p.open
            && let Some(m) = world_matrix(id)
        {
            frame
                .occluder_boxes
                .push(m * Matrix4x4::trs(p.center, oxide_math::Quaternion::IDENTITY, p.size));
        }
    }
    let hidden = match frame.cameras.first() {
        Some(cam) => lod_hidden(world, cam, quality.lod_bias),
        None => std::collections::HashSet::new(),
    };
    for (id, r) in world.query::<MeshRenderer>() {
        if hidden.contains(&id) {
            continue;
        }
        let Some(mesh) = world
            .get_component::<MeshFilter>(id)
            .and_then(|f| f.mesh.as_ref())
        else {
            continue;
        };
        let Some(model) = world_matrix(id) else {
            continue;
        };
        let submeshes = assets
            .meshes
            .get(mesh)
            .map_or(1, |m| m.submeshes.len().max(1));
        // Baked lighting: the lightmap of a static (Contribute GI) renderer, else probes.
        let lightmap = (flags_of(id).contains(StaticEditorFlags::CONTRIBUTE_GI)
            && r.receive_gi == oxide_scene::ReceiveGI::Lightmaps
            && r.lightmap_index >= 0)
            .then_some((r.lightmap_index as u32, r.lightmap_scale_offset));
        let light_probe = lightmap
            .is_none()
            .then(|| crate::lighting::probe_sh(&settings, model.w_axis.truncate()))
            .flatten();
        for s in 0..submeshes {
            let material = r
                .materials
                .get(s)
                .or(r.materials.last())
                .map_or(default_mat, |m| m.id());
            frame.draws.push(DrawItem {
                id: id.to_bits(),
                mesh: mesh.id(),
                submesh: s as u32,
                material,
                model,
                cast_shadows: r.cast_shadows != ShadowCastingMode::Off,
                shadows_only: r.cast_shadows == ShadowCastingMode::ShadowsOnly,
                receive_shadows: r.receive_shadows,
                layer: layer_of(id),
                joint_offset: None,
                occluder: flags_of(id).contains(StaticEditorFlags::OCCLUDER_STATIC),
                occludee: occludee(id, r.allow_occlusion_when_dynamic),
                reflection_probe_static: probe_static(id),
                lightmap,
                light_probe,
            });
        }
    }

    for (id, r) in world.query::<SkinnedMeshRenderer>() {
        if hidden.contains(&id) {
            continue;
        }
        let light_probe = crate::lighting::probe_sh(&settings, world.position(id));
        let Some(mesh_h) = &r.mesh else { continue };
        let Some(mesh) = assets.meshes.get(mesh_h) else {
            continue;
        };
        let offset = frame.joints.len() as u32;
        if mesh.bindposes.is_empty() || r.bones.is_empty() {
            // Not actually skinned: draw it rigidly at the object.
            let Some(model) = world_matrix(id) else {
                continue;
            };
            for s in 0..mesh.submeshes.len().max(1) {
                frame.draws.push(DrawItem {
                    id: id.to_bits(),
                    mesh: mesh_h.id(),
                    submesh: s as u32,
                    material: r
                        .materials
                        .get(s)
                        .or(r.materials.last())
                        .map_or(default_mat, |m| m.id()),
                    model,
                    cast_shadows: r.cast_shadows != ShadowCastingMode::Off,
                    shadows_only: false,
                    receive_shadows: r.receive_shadows,
                    layer: layer_of(id),
                    joint_offset: None,
                    occluder: false,
                    occludee: occludee(id, r.allow_occlusion_when_dynamic),
                    reflection_probe_static: probe_static(id),
                    lightmap: None,
                    light_probe,
                });
            }
            continue;
        }
        for (i, bp) in mesh.bindposes.iter().enumerate() {
            let bone = r
                .bones
                .get(i)
                .and_then(|&b| world_matrix(b))
                .unwrap_or(Matrix4x4::IDENTITY);
            frame.joints.push(bone * *bp);
        }
        for s in 0..mesh.submeshes.len().max(1) {
            frame.draws.push(DrawItem {
                id: id.to_bits(),
                mesh: mesh_h.id(),
                submesh: s as u32,
                material: r
                    .materials
                    .get(s)
                    .or(r.materials.last())
                    .map_or(default_mat, |m| m.id()),
                model: Matrix4x4::IDENTITY,
                cast_shadows: r.cast_shadows != ShadowCastingMode::Off,
                shadows_only: r.cast_shadows == ShadowCastingMode::ShadowsOnly,
                receive_shadows: r.receive_shadows,
                layer: layer_of(id),
                joint_offset: Some(offset),
                occluder: false,
                occludee: false,
                reflection_probe_static: probe_static(id),
                lightmap: None,
                light_probe,
            });
        }
    }

    if let Some(d) = world.try_resource::<DebugDraw>() {
        for l in &d.lines {
            frame.line(l.start, l.end, l.color);
        }
    }
    frame
}

/// Active reflection probes as the renderer sees them.
/// A light's realtime shadows, limited by the quality level (Unity `QualitySettings.shadows`).
fn shadow_settings(l: &Light, q: &QualityLevel) -> Option<ShadowSettings> {
    if l.shadows == LightShadows::None || q.shadows == ShadowQuality::Disable {
        return None;
    }
    Some(ShadowSettings {
        soft: l.shadows == LightShadows::Soft && q.shadows == ShadowQuality::All,
        strength: l.shadow_strength,
        bias: l.shadow_bias,
        normal_bias: l.shadow_normal_bias,
    })
}

pub fn reflection_probes(world: &World) -> Vec<ReflectionProbeView> {
    let realtime = QualitySettings::of(world).realtime_reflection_probes;
    world
        .query::<ReflectionProbe>()
        .map(|(id, p)| {
            let position = world.position(id);
            let (box_min, box_max) = p.bounds(position);
            let bg = p.background_color.linear();
            ReflectionProbeView {
                id: id.to_bits(),
                position,
                box_min,
                box_max,
                box_projection: p.box_projection,
                blend_distance: p.blend_distance,
                importance: p.importance,
                intensity: p.intensity,
                near: p.near_clip_plane.max(0.01),
                far: p.far_clip_plane.max(p.near_clip_plane + 0.1),
                culling_mask: p.culling_mask,
                clear: match p.clear_flags {
                    ReflectionProbeClearFlags::Skybox => ClearMode::Sky,
                    ReflectionProbeClearFlags::SolidColor => {
                        ClearMode::Color(Color::new(bg.r, bg.g, bg.b, 1.0))
                    }
                },
                baked: p.mode == ReflectionProbeMode::Baked,
                refresh: match (p.mode, p.refresh_mode) {
                    (ReflectionProbeMode::Baked, _) => ProbeRefresh::OnAwake,
                    (_, ReflectionProbeRefreshMode::OnAwake) => ProbeRefresh::OnAwake,
                    // Quality "Realtime Reflection Probes" off: capture once, never refresh.
                    (_, ReflectionProbeRefreshMode::EveryFrame) if !realtime => {
                        ProbeRefresh::OnAwake
                    }
                    (_, ReflectionProbeRefreshMode::EveryFrame) => ProbeRefresh::EveryFrame,
                    (_, ReflectionProbeRefreshMode::ViaScripting) => ProbeRefresh::ViaScripting,
                },
                render_requests: p.render_requests,
            }
        })
        .collect()
}

/// World-space bounding sphere of a LOD group: from `size` if set, else from its first
/// level's renderer bounds.
pub fn lod_group_sphere(
    world: &World,
    id: oxide_scene::GameObjectId,
    g: &LODGroup,
) -> (oxide_math::Vector3, f32) {
    let pos = world.position(id);
    if g.size > 0.0 {
        let s = world.lossy_scale(id).abs().max_element();
        return (pos, g.size * s * 0.5);
    }
    let assets = world.resource::<AssetServer>();
    let mut min = oxide_math::Vector3::splat(f32::MAX);
    let mut max = oxide_math::Vector3::splat(f32::MIN);
    for &r in g.lods.first().map_or(&[][..], |l| &l.renderers[..]) {
        let mesh = world
            .get_component::<MeshFilter>(r)
            .and_then(|f| f.mesh.as_ref())
            .or_else(|| {
                world
                    .get_component::<SkinnedMeshRenderer>(r)
                    .and_then(|s| s.mesh.as_ref())
            })
            .and_then(|h| assets.meshes.get(h));
        let Some(mesh) = mesh else { continue };
        let m = world.local_to_world(r);
        let (lo, hi) = (mesh.bounds.min(), mesh.bounds.max());
        for i in 0..8 {
            let c = oxide_math::Vector3::new(
                if i & 1 == 0 { lo.x } else { hi.x },
                if i & 2 == 0 { lo.y } else { hi.y },
                if i & 4 == 0 { lo.z } else { hi.z },
            );
            let p = m.transform_point3(c);
            min = min.min(p);
            max = max.max(p);
        }
    }
    if min.x > max.x {
        return (pos, 0.5);
    }
    ((min + max) * 0.5, ((max - min) * 0.5).length())
}

/// Fraction of the screen height a sphere covers from a camera.
pub fn screen_relative_height(cam: &CameraView, center: oxide_math::Vector3, radius: f32) -> f32 {
    match cam.projection {
        ProjectionKind::Perspective { fov } => {
            let d = (center - cam.position).length().max(1e-3);
            radius * 2.0 / (2.0 * d * (fov * 0.5).to_radians().tan())
        }
        ProjectionKind::Orthographic { size } => radius / size.max(1e-3),
    }
}

/// The LOD level a group shows from `cam` (`None` = culled).
pub fn lod_level_for(
    world: &World,
    id: oxide_scene::GameObjectId,
    g: &LODGroup,
    cam: &CameraView,
    bias: f32,
) -> Option<usize> {
    let (c, r) = lod_group_sphere(world, id, g);
    let level =
        oxide_scene::lod_level(&g.lods, screen_relative_height(cam, c, r) * bias.max(0.01))?;
    // Quality "Maximum LOD Level": finer levels are skipped.
    let max = QualitySettings::of(world).maximum_lod_level as usize;
    Some(level.max(max.min(g.lods.len().saturating_sub(1))))
}

/// Renderer objects LOD groups hide from `cam` (levels not chosen, or culled groups).
fn lod_hidden(
    world: &World,
    cam: &CameraView,
    bias: f32,
) -> std::collections::HashSet<oxide_scene::GameObjectId> {
    let mut hidden = std::collections::HashSet::new();
    for (id, g) in world.query::<LODGroup>() {
        let level = lod_level_for(world, id, g, cam, bias);
        for (i, lod) in g.lods.iter().enumerate() {
            if Some(i) != level {
                hidden.extend(lod.renderers.iter().copied());
            }
        }
        // A renderer in the chosen level shows even if also listed in another level.
        if let Some(l) = level.and_then(|l| g.lods.get(l)) {
            for r in &l.renderers {
                hidden.remove(r);
            }
        }
    }
    hidden
}

/// Logs a warning once per message (extraction runs every frame).
fn warn_once(msg: &str) {
    static SEEN: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if !seen.iter().any(|m| m == msg) {
        seen.push(msg.to_owned());
        oxide_core::debug_log_warning!("{msg}");
    }
}

#[cfg(test)]
mod lod_tests {
    use super::*;
    use oxide_assets::PrimitiveType;
    use oxide_math::Vector3;
    use oxide_scene::Lod;

    fn camera_at(z: f32) -> CameraView {
        CameraView {
            position: Vector3::new(0.0, 0.0, z),
            rotation: oxide_math::Quaternion::IDENTITY,
            projection: ProjectionKind::Perspective { fov: 60.0 },
            near: 0.1,
            far: 5000.0,
            viewport: [0.0, 0.0, 1.0, 1.0],
            clear: ClearMode::Sky,
            culling_mask: u32::MAX,
            occlusion_culling: true,
            antialiasing: Default::default(),
        }
    }

    #[test]
    fn lod_switches_with_distance_and_culls() {
        let mut w = World::new();
        crate::init_world(&mut w, std::env::temp_dir());
        let root = w.create("Tree");
        let hi = crate::objects::create_primitive(&mut w, PrimitiveType::Sphere);
        let lo = crate::objects::create_primitive(&mut w, PrimitiveType::Cube);
        w.set_parent(hi, Some(root), false).unwrap();
        w.set_parent(lo, Some(root), false).unwrap();
        w.add_component(
            root,
            LODGroup {
                lods: vec![
                    Lod {
                        screen_relative_height: 0.5,
                        renderers: vec![hi],
                    },
                    Lod {
                        screen_relative_height: 0.05,
                        renderers: vec![lo],
                    },
                ],
                size: 1.0,
            },
        );
        let drawn = |z: f32| -> Vec<AssetId> {
            let mut f = extract(&w, Some(vec![camera_at(z)]));
            f.draws
                .retain(|d| d.mesh != AssetId::from_path("builtin:Plane"));
            f.draws.iter().map(|d| d.mesh).collect()
        };
        // 1 m object: at 1.5 m it fills ~58% of the screen height -> LOD0.
        assert_eq!(
            drawn(-1.5),
            vec![AssetId::from_path(&PrimitiveType::Sphere.mesh_path())]
        );
        // At 5 m: ~17% -> LOD1.
        assert_eq!(
            drawn(-5.0),
            vec![AssetId::from_path(&PrimitiveType::Cube.mesh_path())]
        );
        // At 50 m: ~1.7% -> culled.
        assert!(drawn(-50.0).is_empty());
    }
}
