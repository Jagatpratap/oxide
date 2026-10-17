//! Asset thumbnails for the Project window and Inspector (Unity's `AssetPreview`).
//!
//! Textures are decoded and shrunk on a background thread. Materials (on a sphere), models
//! and prefabs are rendered offscreen with their own renderer, one per frame so browsing a
//! big folder doesn't stall the editor. Thumbnails are redone when the file or its `.meta`
//! changes.

use oxide_assets::{AssetServer, PrimitiveType};
use oxide_engine::Engine;
use oxide_math::{Color, Quaternion, QuaternionExt, Vector3};
use oxide_render::{CameraView, ClearMode, ProjectionKind, SceneRenderer, wgpu};
use oxide_scene::World;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::SystemTime;

/// Thumbnail size in pixels (shown at up to this size; the Inspector preview scales it).
pub const SIZE: u32 = 192;

/// Background behind rendered previews (Unity's dark preview grey), sRGB.
const BACKGROUND: [u8; 3] = [49, 49, 49];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewKind {
    Texture,
    Material,
    Model,
    Prefab,
    LensFlare,
    FontAsset,
}

/// What kind of preview an asset gets, if any.
pub fn preview_kind(path: &str) -> Option<PreviewKind> {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "tga" | "bmp" | "hdr" | "exr" | "psd" | "psb" | "tif" | "tiff"
        | "gif" | "webp" => Some(PreviewKind::Texture),
        "mat" => Some(PreviewKind::Material),
        "glb" | "gltf" | "fbx" | "obj" => Some(PreviewKind::Model),
        "prefab" => Some(PreviewKind::Prefab),
        "lensflare" => Some(PreviewKind::LensFlare),
        "fontasset" => Some(PreviewKind::FontAsset),
        _ => None,
    }
}

struct Thumb {
    texture: Option<egui::TextureHandle>,
    /// File (and `.meta`) time the thumbnail was made from.
    stamp: Option<SystemTime>,
    /// When the file's time was last checked (seconds).
    checked: f64,
    /// Queued or being decoded.
    pending: bool,
    failed: bool,
}

type Decoded = (String, Option<SystemTime>, Result<egui::ColorImage, String>);

pub struct Previews {
    thumbs: HashMap<String, Thumb>,
    /// Rendered previews waiting for their turn (newest request first).
    queue: VecDeque<String>,
    renderer: Option<SceneRenderer>,
    tx: mpsc::Sender<Decoded>,
    rx: mpsc::Receiver<Decoded>,
}

impl Default for Previews {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            thumbs: HashMap::new(),
            queue: VecDeque::new(),
            renderer: None,
            tx,
            rx,
        }
    }
}

fn stamp(full: &Path) -> Option<SystemTime> {
    let t = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    let meta = PathBuf::from(format!("{}.meta", full.display()));
    t(full).max(t(&meta))
}

fn decode_texture(full: &Path) -> Result<egui::ColorImage, String> {
    let bytes = std::fs::read(full).map_err(|e| e.to_string())?;
    let img = if bytes.starts_with(b"8BPS") {
        // Photoshop files: the composite image, as the importer reads it.
        let (w, h, px) = oxide_assets::psd::decode(&bytes)?;
        image::DynamicImage::ImageRgba8(
            image::RgbaImage::from_raw(w, h, px).ok_or("bad PSD image size")?,
        )
    } else {
        image::load_from_memory(&bytes).map_err(|e| e.to_string())?
    };
    let img = img.thumbnail(SIZE, SIZE).to_rgba8();
    let (w, h) = img.dimensions();
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        [w as usize, h as usize],
        img.as_raw(),
    ))
}

impl Previews {
    /// The thumbnail of `path`, if there is one yet. Requests it (or a newer one when the
    /// file changed) as a side effect, so call it for the assets that are on screen.
    pub fn get(&mut self, root: &Path, path: &str) -> Option<egui::TextureId> {
        let kind = preview_kind(path)?;
        let now = oxide_core::time::realtime_since_startup();
        let full = root.join(path);
        let t = self.thumbs.entry(path.to_owned()).or_insert(Thumb {
            texture: None,
            stamp: None,
            checked: f64::MIN,
            pending: false,
            failed: false,
        });
        if !t.pending && now - t.checked > 1.0 {
            t.checked = now;
            let s = stamp(&full);
            if s.is_some() && (s != t.stamp || (t.texture.is_none() && !t.failed)) {
                t.pending = true;
                t.failed = false;
                if kind == PreviewKind::Texture {
                    let tx = self.tx.clone();
                    let path = path.to_owned();
                    std::thread::spawn(move || {
                        let _ = tx.send((path, s, decode_texture(&full)));
                    });
                } else {
                    self.queue.push_front(path.to_owned());
                }
            }
        }
        t.texture.as_ref().map(|t| t.id())
    }

    /// Forgets every thumbnail (e.g. after Assets → Refresh).
    pub fn clear(&mut self) {
        self.thumbs.clear();
        self.queue.clear();
    }

    /// Takes finished texture decodes and renders the next queued preview.
    pub fn update(&mut self, engine: &mut Engine, ctx: &egui::Context) {
        while let Ok((path, s, result)) = self.rx.try_recv() {
            self.finish(ctx, &path, s, result);
        }
        // Only previews still on screen (requested again within the last few seconds).
        let now = oxide_core::time::realtime_since_startup();
        while let Some(path) = self.queue.pop_front() {
            let Some(t) = self.thumbs.get(&path) else {
                continue;
            };
            if now - t.checked > 5.0 {
                if let Some(t) = self.thumbs.get_mut(&path) {
                    t.pending = false;
                    t.checked = f64::MIN;
                }
                continue;
            }
            let root = engine.world.resource::<AssetServer>().full_path("");
            let s = stamp(&root.join(&path));
            let result = self.render(engine, &path);
            self.finish(ctx, &path, s, result);
            ctx.request_repaint();
            break;
        }
        if !self.queue.is_empty() {
            ctx.request_repaint();
        }
    }

    fn finish(
        &mut self,
        ctx: &egui::Context,
        path: &str,
        s: Option<SystemTime>,
        result: Result<egui::ColorImage, String>,
    ) {
        let Some(t) = self.thumbs.get_mut(path) else {
            return;
        };
        t.pending = false;
        t.stamp = s;
        match result {
            Ok(img) => {
                let opts = egui::TextureOptions::LINEAR;
                match t.texture.as_mut() {
                    Some(h) => h.set(img, opts),
                    None => {
                        t.texture = Some(ctx.load_texture(format!("preview:{path}"), img, opts))
                    }
                }
            }
            Err(e) => {
                t.failed = true;
                oxide_core::debug_log_warning!("No preview for {path}: {e}");
            }
        }
    }

    /// Renders a material, model or prefab preview.
    fn render(&mut self, engine: &mut Engine, path: &str) -> Result<egui::ColorImage, String> {
        let kind = preview_kind(path).ok_or("no preview for this type")?;
        let frame = if kind == PreviewKind::LensFlare {
            lens_flare_frame(engine, path)?
        } else if kind == PreviewKind::FontAsset {
            font_asset_frame(engine, path)?
        } else {
            self.scene_frame(engine, kind, path)?
        };
        let gpu = &engine.gpu;
        let renderer = self.renderer.get_or_insert_with(|| SceneRenderer::new(gpu));
        let assets = engine.world.resource::<AssetServer>();
        let fmt = wgpu::TextureFormat::Rgba8UnormSrgb;
        let img = oxide_render::capture_frame(gpu, fmt, SIZE, SIZE, |view, _| {
            renderer.render(gpu, assets, &frame, view, fmt, SIZE, SIZE);
        })?;
        Ok(egui::ColorImage::from_rgba_unmultiplied(
            [img.width as usize, img.height as usize],
            &img.rgba,
        ))
    }

    fn scene_frame(
        &mut self,
        engine: &mut Engine,
        kind: PreviewKind,
        path: &str,
    ) -> Result<oxide_render::RenderFrame, String> {
        with_scratch_world(&mut engine.world, |w| {
            build_preview_scene(w, kind, path)?;
            // Unity previews use a key light from the upper left.
            oxide_engine::objects::create_directional_light(w);
            let bg = Color::rgb(
                BACKGROUND[0] as f32 / 255.0,
                BACKGROUND[1] as f32 / 255.0,
                BACKGROUND[2] as f32 / 255.0,
            );
            let cam = CameraView {
                position: Vector3::ZERO,
                rotation: Quaternion::IDENTITY,
                projection: ProjectionKind::Perspective { fov: 30.0 },
                near: 0.01,
                far: 1000.0,
                viewport: [0.0, 0.0, 1.0, 1.0],
                clear: ClearMode::Color(bg.linear()),
                culling_mask: u32::MAX,
                occlusion_culling: false,
                antialiasing: Default::default(),
            };
            let mut frame = oxide_engine::extract::extract(w, Some(vec![cam]));
            // Frame what is drawn (not bones or empty objects).
            let (center, radius) =
                draw_bounds(w.resource::<AssetServer>(), &frame).ok_or("nothing to show")?;
            let radius = radius.max(0.01);
            let distance = radius / 15.0_f32.to_radians().sin() * 1.05;
            // Unity's preview angle: from the front-left, slightly above.
            let cam = &mut frame.cameras[0];
            cam.rotation = Quaternion::euler(20.0, -150.0, 0.0);
            cam.position = center - cam.rotation * Vector3::new(0.0, 0.0, distance);
            cam.near = (distance - radius * 2.0).max(distance * 0.01);
            cam.far = distance + radius * 2.0;
            // Plain colors: no vignette or grain from the defaults.
            frame.environment.post.vignette.intensity = 0.0;
            Ok::<_, String>(frame)
        })
    }
}

/// A material on a primitive seen from `pitch`/`yaw` degrees (the Shader Graph window's
/// Main Preview, which you turn by dragging).
pub fn material_frame(
    engine: &mut Engine,
    material: &str,
    mesh: PrimitiveType,
    pitch: f32,
    yaw: f32,
    time: f32,
) -> Result<oxide_render::RenderFrame, String> {
    with_scratch_world(&mut engine.world, |w| {
        let m = w
            .resource::<AssetServer>()
            .materials
            .handle_of(material)
            .ok_or_else(|| format!("{material} is not loaded"))?;
        oxide_engine::objects::create_visual(w, "Preview", mesh, Some(m));
        oxide_engine::objects::create_directional_light(w);
        let bg = Color::rgb(
            BACKGROUND[0] as f32 / 255.0,
            BACKGROUND[1] as f32 / 255.0,
            BACKGROUND[2] as f32 / 255.0,
        );
        let rotation = Quaternion::euler(pitch, yaw, 0.0);
        let distance = 3.2;
        let cam = CameraView {
            position: rotation * Vector3::new(0.0, 0.0, -distance),
            rotation,
            projection: ProjectionKind::Perspective { fov: 30.0 },
            near: 0.05,
            far: 50.0,
            viewport: [0.0, 0.0, 1.0, 1.0],
            clear: ClearMode::Color(bg.linear()),
            culling_mask: u32::MAX,
            occlusion_culling: false,
            antialiasing: Default::default(),
        };
        let mut frame = oxide_engine::extract::extract(w, Some(vec![cam]));
        frame.environment.post.vignette.intensity = 0.0;
        frame.time = time;
        Ok(frame)
    })
}

/// A Font Asset's sample text ("Abg") on a dark background.
fn font_asset_frame(engine: &mut Engine, path: &str) -> Result<oxide_render::RenderFrame, String> {
    with_scratch_world(&mut engine.world, |w| {
        let id = w.create("Preview");
        w.add_component(
            id,
            oxide_engine::text::TextMeshPro {
                text: "Abg".into(),
                font_asset: Some(oxide_assets::Handle::from_path(path.to_owned())),
                font_size: 26.0,
                rect_size: oxide_math::Vector2::ZERO,
                horizontal_alignment: oxide_engine::text::HorizontalAlignment::Center,
                vertical_alignment: oxide_engine::text::VerticalAlignment::Middle,
                wrapping: false,
                ..Default::default()
            },
        );
        oxide_engine::text::components::update(w);
        let bg = Color::rgb(
            BACKGROUND[0] as f32 / 255.0,
            BACKGROUND[1] as f32 / 255.0,
            BACKGROUND[2] as f32 / 255.0,
        );
        let cam = CameraView {
            position: Vector3::new(0.0, 0.0, -10.0),
            rotation: Quaternion::IDENTITY,
            projection: ProjectionKind::Orthographic { size: 3.0 },
            near: 0.1,
            far: 100.0,
            viewport: [0.0, 0.0, 1.0, 1.0],
            clear: ClearMode::Color(bg.linear()),
            culling_mask: u32::MAX,
            occlusion_culling: false,
            antialiasing: Default::default(),
        };
        let mut frame = oxide_engine::extract::extract(w, Some(vec![cam]));
        frame.environment.post.vignette.intensity = 0.0;
        Ok(frame)
    })
}

/// A Lens Flare asset drawn on a dark background, its source up and to the left so the
/// ghosts spread across the preview.
pub fn lens_flare_frame(
    engine: &mut Engine,
    path: &str,
) -> Result<oxide_render::RenderFrame, String> {
    let assets = engine.world.resource_mut::<AssetServer>();
    let h = oxide_assets::Handle::<oxide_scene::LensFlareData>::from_path(path.to_owned());
    let data = assets
        .data_asset(&h)
        .cloned()
        .ok_or("the lens flare can't be loaded")?;
    let comp = oxide_scene::LensFlareComponent {
        use_occlusion: false,
        allow_off_screen: true,
        ..Default::default()
    };
    let source = oxide_engine::lens_flares::to_source(
        &comp,
        &data,
        Vector3::new(-2.0, 2.0, 10.0),
        false,
        [1.0; 3],
        None,
    );
    let bg = Color::rgb(0.02, 0.02, 0.03);
    let mut frame = oxide_render::RenderFrame {
        cameras: vec![CameraView {
            position: Vector3::ZERO,
            rotation: Quaternion::IDENTITY,
            projection: ProjectionKind::Perspective { fov: 40.0 },
            near: 0.1,
            far: 100.0,
            viewport: [0.0, 0.0, 1.0, 1.0],
            clear: ClearMode::Color(bg),
            culling_mask: u32::MAX,
            occlusion_culling: false,
            antialiasing: Default::default(),
        }],
        lens_flares: vec![source],
        ..Default::default()
    };
    frame.environment.post.vignette.intensity = 0.0;
    Ok(frame)
}

/// Bounding sphere of everything drawn in `frame`.
fn draw_bounds(assets: &AssetServer, frame: &oxide_render::RenderFrame) -> Option<(Vector3, f32)> {
    let mut min = Vector3::splat(f32::MAX);
    let mut max = Vector3::splat(f32::MIN);
    for d in &frame.draws {
        let Some(mesh) = assets.meshes.get_by_id(d.mesh) else {
            continue;
        };
        // Skinned meshes are posed by their joints: skin a sample of the vertices.
        if let Some(off) = d.joint_offset {
            let step = (mesh.vertices.len() / 2000).max(1);
            for (i, v) in mesh.vertices.iter().enumerate().step_by(step) {
                let (Some(bi), Some(bw)) = (mesh.bone_indices.get(i), mesh.bone_weights.get(i))
                else {
                    continue;
                };
                let mut p = Vector3::ZERO;
                for (k, w) in [bw.x, bw.y, bw.z, bw.w].into_iter().enumerate() {
                    if let Some(j) = frame.joints.get(off as usize + bi[k] as usize) {
                        p += j.transform_point3(*v) * w;
                    }
                }
                min = min.min(p);
                max = max.max(p);
            }
            continue;
        }
        let (a, b) = (mesh.bounds.min(), mesh.bounds.max());
        for i in 0..8 {
            let c = Vector3::new(
                if i & 1 == 0 { a.x } else { b.x },
                if i & 2 == 0 { a.y } else { b.y },
                if i & 4 == 0 { a.z } else { b.z },
            );
            let p = d.model.transform_point3(c);
            min = min.min(p);
            max = max.max(p);
        }
    }
    (min.x <= max.x).then(|| ((min + max) * 0.5, ((max - min) * 0.5).length()))
}

/// The objects a preview shows, in an empty world.
fn build_preview_scene(w: &mut World, kind: PreviewKind, path: &str) -> Result<(), String> {
    match kind {
        PreviewKind::Material => {
            let m = {
                let assets = w.resource_mut::<AssetServer>();
                match assets.materials.handle_of(path) {
                    Some(h) => h,
                    None => assets.load_material(path)?,
                }
            };
            oxide_engine::objects::create_visual(w, "Preview", PrimitiveType::Sphere, Some(m));
        }
        PreviewKind::Model => {
            let h = {
                let assets = w.resource_mut::<AssetServer>();
                match assets.models.handle_of(path) {
                    Some(h) => h,
                    None => assets.load_model(path)?,
                }
            };
            oxide_engine::objects::instantiate_model(w, &h).ok_or("the model is empty")?;
        }
        PreviewKind::Prefab => {
            w.instantiate_prefab(path, None)?;
        }
        PreviewKind::Texture => return Err("textures are decoded, not rendered".into()),
        PreviewKind::LensFlare => return Err("lens flares are drawn without a scene".into()),
        PreviewKind::FontAsset => return Err("font assets are drawn as text".into()),
    }
    Ok(())
}

/// Runs `f` on an empty world that borrows the editor world's assets and component types
/// (so prefabs resolve and loaded assets are shared), then gives them back, even if `f`
/// panics.
fn with_scratch_world<R>(main: &mut World, f: impl FnOnce(&mut World) -> R) -> R {
    let mut scratch = World::new();
    scratch.run_scripts = false;
    std::mem::swap(&mut main.registry, &mut scratch.registry);
    if let Some(a) = main.resources.remove::<AssetServer>() {
        scratch.resources.insert(a);
    }
    // Font atlases too: they write textures into the shared AssetServer.
    if let Some(f) = main.resources.remove::<oxide_engine::text::FontLibrary>() {
        scratch.resources.insert(f);
    }
    scratch.resources.insert(oxide_core::Time::default());
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(&mut scratch)));
    std::mem::swap(&mut main.registry, &mut scratch.registry);
    if let Some(a) = scratch.resources.remove::<AssetServer>() {
        main.resources.insert(a);
    }
    if let Some(f) = scratch
        .resources
        .remove::<oxide_engine::text::FontLibrary>()
    {
        main.resources.insert(f);
    }
    match result {
        Ok(r) => r,
        Err(p) => std::panic::resume_unwind(p),
    }
}
