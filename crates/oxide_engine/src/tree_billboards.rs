//! Bakes terrain tree billboards (Unity makes them from each tree prototype): the
//! prototype mesh rendered from eight directions around it into one atlas, its background
//! made transparent by rendering on black and on white and comparing. Far trees then draw
//! as these images (see `oxide_terrain::extract_tree_billboards`).

use crate::Engine;
use oxide_assets::{AssetId, AssetServer, Handle, Material, Mesh, Texture2D};
use oxide_math::{Color, Matrix4x4, Quaternion, QuaternionExt, Vector3};
use oxide_render::*;
use oxide_terrain::{Terrain, TreeBillboard, TreeBillboards};

/// Views around each tree.
pub const VIEWS: u32 = 8;
/// Width of one view in the atlas (pixels).
const TILE: u32 = 128;

/// Bakes the billboard of one tree prototype that has none yet (one per frame).
pub fn update(engine: &mut Engine) {
    let w = &engine.world;
    if w.try_resource::<TreeBillboards>().is_none() {
        engine.world.resources.insert(TreeBillboards::default());
    }
    let w = &engine.world;
    let done = w.resource::<TreeBillboards>();
    let assets = w.resource::<AssetServer>();
    let mut next: Option<(Handle<Mesh>, Vec<Handle<Material>>)> = None;
    'find: for (_, t) in w.query::<Terrain>() {
        if !t.draw_trees {
            continue;
        }
        let Some(data) = t.data.as_ref().and_then(|h| assets.terrains.get(h)) else {
            continue;
        };
        for p in &data.tree_prototypes {
            let id = p.mesh.id();
            if !done.by_mesh.contains_key(&id)
                && !done.failed.contains(&id)
                && assets.meshes.get(&p.mesh).is_some()
            {
                next = Some((p.mesh.clone(), p.materials.clone()));
                break 'find;
            }
        }
    }
    let Some((mesh, materials)) = next else {
        return;
    };
    let result = bake(
        &engine.gpu,
        &mut engine.renderer,
        engine.world.resource::<AssetServer>(),
        &mesh,
        &materials,
    );
    let id = mesh.id();
    match result {
        Some((texture, mut billboard)) => {
            let path = format!("runtime:tree-billboard/{}", mesh.path());
            let h = engine
                .world
                .resource_mut::<AssetServer>()
                .textures
                .insert(path, texture);
            billboard.texture = h.id();
            engine
                .world
                .resource_mut::<TreeBillboards>()
                .by_mesh
                .insert(id, billboard);
        }
        None => {
            // Not bakeable (no GPU capture): draw meshes only, don't retry every frame.
            oxide_core::debug_log_warning!("could not bake a billboard for {}", mesh.path());
            engine
                .world
                .resource_mut::<TreeBillboards>()
                .failed
                .insert(id);
        }
    }
}

/// Renders `mesh` from [`VIEWS`] directions into an atlas.
pub fn bake(
    gpu: &Gpu,
    renderer: &mut SceneRenderer,
    assets: &AssetServer,
    mesh: &Handle<Mesh>,
    materials: &[Handle<Material>],
) -> Option<(Texture2D, TreeBillboard)> {
    let m = assets.meshes.get(mesh)?;
    let b = m.bounds;
    let (min, max) = (b.min(), b.max());
    let radius = (max.x.abs().max(min.x.abs()))
        .max(max.z.abs().max(min.z.abs()))
        .max(1e-3);
    let width = radius * 2.0;
    let height = (max.y - min.y).max(1e-3);
    let th = ((TILE as f32 * height / width).round() as u32).clamp(32, 512);
    let center = Vector3::new(0.0, (min.y + max.y) * 0.5, 0.0);
    let subs = m.submeshes.len().max(1);
    let draws: Vec<DrawItem> = (0..subs)
        .map(|s| DrawItem {
            id: 0,
            mesh: mesh.id(),
            submesh: s as u32,
            material: materials
                .get(s)
                .or(materials.last())
                .map_or(AssetId::from_path(oxide_assets::DEFAULT_MATERIAL), |h| {
                    h.id()
                }),
            model: Matrix4x4::IDENTITY,
            cast_shadows: false,
            shadows_only: false,
            receive_shadows: false,
            layer: 0,
            joint_offset: None,
            occluder: false,
            occludee: false,
            reflection_probe_static: false,
            lightmap: None,
            light_probe: None,
            sorting: Default::default(),
            rendering_layers: u32::MAX,
        })
        .collect();
    let fmt = wgpu::TextureFormat::Rgba8UnormSrgb;
    let atlas_w = TILE * VIEWS;
    let mut atlas = vec![0u8; (atlas_w * th * 4) as usize];
    for v in 0..VIEWS {
        let a = v as f32 / VIEWS as f32 * std::f32::consts::TAU;
        let dir = Vector3::new(a.sin(), 0.0, a.cos());
        let eye = center + dir * (radius + height) * 2.0;
        let render = |renderer: &mut SceneRenderer, bg: Color| -> Option<CapturedImage> {
            let frame = RenderFrame {
                cameras: vec![CameraView {
                    position: eye,
                    rotation: Quaternion::look_rotation(-dir, Vector3::Y),
                    projection: ProjectionKind::Orthographic { size: height * 0.5 },
                    near: 0.01,
                    far: (radius + height) * 6.0,
                    viewport: [0.0, 0.0, 1.0, 1.0],
                    clear: ClearMode::Color(bg),
                    culling_mask: u32::MAX,
                    occlusion_culling: false,
                    antialiasing: Default::default(),
                }],
                draws: draws.clone(),
                environment: Environment {
                    post: PostSettings {
                        tonemapping: TonemapMode::None,
                        ..PostSettings::neutral()
                    },
                    ..Default::default()
                },
                ..Default::default()
            };
            capture_frame(gpu, fmt, TILE, th, |view, _| {
                renderer.render(gpu, assets, &frame, view, fmt, TILE, th);
            })
            .ok()
        };
        let black = render(renderer, Color::BLACK)?;
        let white = render(renderer, Color::WHITE)?;
        for y in 0..th {
            for x in 0..TILE {
                let (b, w) = (black.pixel(x, y), white.pixel(x, y));
                // Coverage from how much the background shows through.
                let diff = (0..3).map(|i| w[i] as f32 - b[i] as f32).sum::<f32>() / 3.0;
                let alpha = (1.0 - diff / 255.0).clamp(0.0, 1.0);
                let o = (((y * atlas_w) + v * TILE + x) * 4) as usize;
                if alpha > 0.01 {
                    for i in 0..3 {
                        atlas[o + i] = (b[i] as f32 / alpha).min(255.0) as u8;
                    }
                }
                atlas[o + 3] = (alpha * 255.0) as u8;
            }
        }
    }
    let images = (0..VIEWS)
        .map(|v| [v as f32 / VIEWS as f32, 0.0, 1.0 / VIEWS as f32, 1.0])
        .collect();
    Some((
        Texture2D::from_rgba8("Tree Billboard", atlas_w, th, atlas, true),
        TreeBillboard {
            texture: AssetId::from_path(""),
            width,
            height,
            bottom: -min.y,
            images,
        },
    ))
}
