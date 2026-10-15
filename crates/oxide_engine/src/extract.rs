//! Scene → [`RenderFrame`]: cameras, lights, mesh draws, skinning matrices, environment
//! and debug lines, read from the World each frame.

use oxide_assets::{AssetId, AssetServer, DEFAULT_MATERIAL};
use oxide_math::{Color, Matrix4x4};
use oxide_render::{
    CameraView, ClearMode, DrawItem, Environment, FogKind, ProjectionKind, PunctualLight,
    RenderFrame, ShadowSettings, SunLight,
};
use oxide_scene::{
    Camera, CameraClearFlags, DebugDraw, FogMode, Light, LightShadows, LightType, MeshFilter,
    MeshRenderer, Projection, RenderSettings, ShadowCastingMode, SkinnedMeshRenderer, World,
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
    }
}

/// Cameras sorted by `depth` (Unity draws lower depth first).
pub fn cameras(world: &World) -> Vec<CameraView> {
    let mut cams: Vec<_> = world
        .query::<Camera>()
        .map(|(id, c)| (c.depth, camera_view(world, id, c)))
        .collect();
    cams.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    cams.into_iter().map(|(_, v)| v).collect()
}

pub fn environment(rs: &RenderSettings) -> Environment {
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
        exposure: rs.exposure,
        bloom: rs.bloom.then_some((rs.bloom_intensity, rs.bloom_threshold)),
        vignette: rs.vignette,
        shadow_distance: rs.shadow_distance,
    }
}

/// Builds the frame. `cameras` overrides the scene cameras (editor Scene view, screenshots).
pub fn extract(world: &World, cameras_override: Option<Vec<CameraView>>) -> RenderFrame {
    let assets = world.resource::<AssetServer>();
    let settings = world
        .try_resource::<RenderSettings>()
        .cloned()
        .unwrap_or_default();
    let matrices = world.all_world_matrices();
    let world_matrix =
        |id: oxide_scene::GameObjectId| matrices.get(id.index() as usize).copied().flatten();

    let mut frame = RenderFrame {
        cameras: cameras_override.unwrap_or_else(|| cameras(world)),
        environment: environment(&settings),
        time: world.resource::<oxide_core::Time>().time(),
        ..Default::default()
    };

    // Lights: the sun is RenderSettings.sun or the brightest directional light.
    let mut sun: Option<(f32, SunLight)> = None;
    for (id, l) in world.query::<Light>() {
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
                            shadows: (l.shadows != LightShadows::None).then_some(ShadowSettings {
                                soft: l.shadows == LightShadows::Soft,
                                strength: l.shadow_strength,
                                bias: l.shadow_bias,
                                normal_bias: l.shadow_normal_bias,
                            }),
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
            }),
        }
    }
    frame.sun = sun.map(|(_, s)| s);

    // Terrains choose chunk LODs for the first camera.
    let cam_pos = frame
        .cameras
        .first()
        .map_or(oxide_math::Vector3::ZERO, |c| c.position);
    oxide_terrain::extract(world, cam_pos, &mut frame.terrain_draws, &mut frame.draws);
    oxide_particles::extract(world, &mut frame.particles);

    let default_mat = AssetId::from_path(DEFAULT_MATERIAL);
    let layer_of = |id| world.get(id).map_or(0, |o| o.layer);
    for (id, r) in world.query::<MeshRenderer>() {
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
        for s in 0..submeshes {
            let material = r
                .materials
                .get(s)
                .or(r.materials.last())
                .map_or(default_mat, |m| m.id());
            frame.draws.push(DrawItem {
                mesh: mesh.id(),
                submesh: s as u32,
                material,
                model,
                cast_shadows: r.cast_shadows != ShadowCastingMode::Off,
                shadows_only: r.cast_shadows == ShadowCastingMode::ShadowsOnly,
                receive_shadows: r.receive_shadows,
                layer: layer_of(id),
                joint_offset: None,
            });
        }
    }

    for (id, r) in world.query::<SkinnedMeshRenderer>() {
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
