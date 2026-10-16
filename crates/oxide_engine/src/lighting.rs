//! Generate Lighting (Unity's Lighting window): gathers static renderers (Contribute GI),
//! Mixed and Baked lights, Light Probe Groups and the sky; bakes them on a background thread
//! with `oxide_lightmapper`; then writes the lightmaps as PNGs into a folder named after the
//! scene and stores each renderer's lightmap index / scale-offset and the probes in the scene.
//! <https://docs.unity3d.com/Manual/lighting-window.html>

use oxide_assets::{AssetId, AssetServer, Material, Texture2D};
use oxide_lightmapper::{
    BakeInput, BakeLight, BakeLightKind, BakeMaterial, BakeObject, BakeOutput, BakeSettings,
    Progress,
};
use oxide_math::{Vector3, Vector4};
use oxide_scene::{
    GameObjectId, Light, LightProbeGroup, LightType, LightmapBakeType, MeshFilter, MeshRenderer,
    ReceiveGI, RenderSettings, SceneHandle, StaticEditorFlags, World,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

struct Job {
    handle: std::thread::JoinHandle<Result<BakeOutput, String>>,
    progress: Arc<Progress>,
    /// The renderer object behind each bake object.
    objects: Vec<GameObjectId>,
    probes: Vec<Vector3>,
    folder: String,
    started: std::time::Instant,
}

/// World resource tracking the background bake.
#[derive(Default)]
pub struct LightingBake {
    job: Option<Job>,
    /// Message of the last finished bake (Ok) or its error.
    pub last: Option<Result<String, String>>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct BakeStatus {
    pub running: bool,
    pub progress: f32,
    pub seconds: f32,
    pub last: Option<String>,
    pub error: Option<String>,
}

/// Folder for a scene's baked data: `Assets/Scenes/City.scene` → `Assets/Scenes/City`.
fn data_folder(world: &World) -> String {
    world
        .scene_info(SceneHandle::MAIN)
        .and_then(|s| s.path)
        .map(|p| p.trim_end_matches(".scene").to_owned())
        .filter(|p| p.starts_with("Assets/"))
        .unwrap_or_else(|| "Assets/Lighting".to_owned())
}

/// Starts baking. Returns how many objects take part.
pub fn start(world: &mut World) -> Result<usize, String> {
    if world
        .try_resource::<LightingBake>()
        .is_some_and(|b| b.job.is_some())
    {
        return Err("a bake is already running".into());
    }
    let rs = world.resource::<RenderSettings>().clone();
    let assets = world.resource::<AssetServer>();
    let mut textures: HashMap<AssetId, Arc<Texture2D>> = HashMap::new();
    let mut material = |m: &Material| {
        let base = m.base_color.linear();
        let em = m.emission_color.linear();
        BakeMaterial {
            albedo: Vector3::new(base.r, base.g, base.b),
            texture: m.base_map.as_ref().and_then(|h| {
                textures
                    .entry(h.id())
                    .or_insert_with(|| {
                        Arc::new(
                            assets
                                .textures
                                .get(h)
                                .cloned()
                                .unwrap_or_else(|| Texture2D::solid("missing", [255; 4], true)),
                        )
                    })
                    .clone()
                    .into()
            }),
            tiling: Vector4::new(m.tiling.x, m.tiling.y, m.offset.x, m.offset.y),
            emission: Vector3::new(em.r, em.g, em.b) * m.emission_intensity,
        }
    };
    let mut objects = Vec::new();
    let mut ids = Vec::new();
    for (id, r) in world.query::<MeshRenderer>() {
        let static_gi = world
            .get(id)
            .is_some_and(|o| o.static_flags.contains(StaticEditorFlags::CONTRIBUTE_GI));
        if !static_gi {
            continue;
        }
        let Some(mesh) = world
            .get_component::<MeshFilter>(id)
            .and_then(|f| f.mesh.as_ref())
            .and_then(|h| assets.meshes.get(h))
        else {
            continue;
        };
        let materials = r
            .materials
            .iter()
            .map(|h| {
                assets
                    .materials
                    .get(h)
                    .map(&mut material)
                    .unwrap_or_default()
            })
            .collect();
        objects.push(BakeObject {
            mesh: Arc::new(mesh.clone()),
            model: world.local_to_world(id),
            materials,
            receive_lightmap: r.receive_gi == ReceiveGI::Lightmaps,
            scale_in_lightmap: r.scale_in_lightmap,
        });
        ids.push(id);
    }
    if objects.is_empty() {
        return Err(
            "nothing to bake: mark objects Static (Contribute GI) to give them lightmaps".into(),
        );
    }
    let mut lights = Vec::new();
    let mut sun: Option<(f32, oxide_render::SunLight)> = None;
    for (id, l) in world.query::<Light>() {
        let c = l.color.linear();
        let color = Vector3::new(c.r, c.g, c.b) * l.intensity;
        if l.light_type == LightType::Directional
            && sun.as_ref().is_none_or(|(i, _)| l.intensity > *i)
        {
            sun = Some((
                l.intensity,
                oxide_render::SunLight {
                    direction: world.forward(id),
                    color: color.to_array(),
                    shadows: None,
                },
            ));
        }
        if l.lightmap_bake_type == LightmapBakeType::Realtime {
            continue;
        }
        let kind = match l.light_type {
            LightType::Directional => BakeLightKind::Directional {
                direction: world.forward(id),
            },
            LightType::Point => BakeLightKind::Point {
                position: world.position(id),
                range: l.range.max(0.01),
            },
            LightType::Spot => {
                let co = (l.spot_angle.to_radians() * 0.5).cos();
                let ci = (l
                    .inner_spot_angle
                    .min(l.spot_angle - 0.1)
                    .max(0.0)
                    .to_radians()
                    * 0.5)
                    .cos();
                BakeLightKind::Spot {
                    position: world.position(id),
                    direction: world.forward(id),
                    range: l.range.max(0.01),
                    cos_outer: co,
                    inv_cone: 1.0 / (ci - co).max(1e-4),
                }
            }
        };
        lights.push(BakeLight {
            kind,
            color,
            baked_direct: l.lightmap_bake_type == LightmapBakeType::Baked,
            cast_shadows: l.shadows != oxide_scene::LightShadows::None,
        });
    }
    let probes: Vec<Vector3> = world
        .query::<LightProbeGroup>()
        .flat_map(|(id, g)| {
            let m = world.local_to_world(id);
            g.probe_positions
                .iter()
                .map(move |p| m.transform_point3(*p))
                .collect::<Vec<_>>()
        })
        .collect();
    let env = crate::extract::environment(&rs);
    let input = BakeInput {
        objects,
        lights,
        sky: oxide_render::sky::SkyModel::new(&env, sun.as_ref().map(|(_, s)| s)),
        ambient_intensity: rs.ambient_intensity,
        probes: probes.clone(),
        settings: BakeSettings {
            texels_per_unit: rs.lightmap_resolution.max(0.1),
            max_lightmap_size: rs.max_lightmap_size.clamp(32, 4096),
            samples: rs.bake_samples.clamp(1, 4096),
            bounces: rs.bake_bounces.min(8),
            padding: rs.lightmap_padding.clamp(1, 16),
            denoise: true,
        },
    };
    let count = input.objects.len();
    let progress = Arc::new(Progress::default());
    let p = progress.clone();
    let handle = std::thread::Builder::new()
        .name("lightmapper".into())
        .spawn(move || oxide_lightmapper::bake(&input, &p))
        .map_err(|e| e.to_string())?;
    let folder = data_folder(world);
    if world.try_resource::<LightingBake>().is_none() {
        world.resources.insert(LightingBake::default());
    }
    world.resource_mut::<LightingBake>().job = Some(Job {
        handle,
        progress,
        objects: ids,
        probes,
        folder,
        started: std::time::Instant::now(),
    });
    Ok(count)
}

pub fn status(world: &World) -> BakeStatus {
    let b = world.try_resource::<LightingBake>();
    let job = b.and_then(|b| b.job.as_ref());
    BakeStatus {
        running: job.is_some(),
        progress: job.map_or(0.0, |j| j.progress.fraction()),
        seconds: job.map_or(0.0, |j| j.started.elapsed().as_secs_f32()),
        last: b.and_then(|b| b.last.clone()).and_then(Result::ok),
        error: b.and_then(|b| b.last.clone()).and_then(Result::err),
    }
}

pub fn cancel(world: &mut World) {
    if let Some(j) = world
        .try_resource::<LightingBake>()
        .and_then(|b| b.job.as_ref())
    {
        j.progress.cancel.store(true, Ordering::Relaxed);
    }
}

/// Removes baked data from the scene (Lighting window "Clear Baked Data").
pub fn clear(world: &mut World) {
    let ids: Vec<GameObjectId> = world.query::<MeshRenderer>().map(|(id, _)| id).collect();
    for id in ids {
        if let Some(r) = world.get_component_mut::<MeshRenderer>(id) {
            r.lightmap_index = -1;
            r.lightmap_scale_offset = Vector4::new(1.0, 1.0, 0.0, 0.0);
        }
    }
    let rs = world.resource_mut::<RenderSettings>();
    rs.lightmaps.clear();
    rs.light_probe_positions.clear();
    rs.light_probe_coefficients.clear();
}

/// Applies a finished bake (call once per frame). Returns the outcome when one finished.
pub fn poll(world: &mut World) -> Option<Result<String, String>> {
    let done = world
        .try_resource::<LightingBake>()
        .and_then(|b| b.job.as_ref())
        .is_some_and(|j| j.handle.is_finished());
    if !done {
        return None;
    }
    let job = world.resource_mut::<LightingBake>().job.take()?;
    let seconds = job.started.elapsed().as_secs_f32();
    let result = match job.handle.join() {
        Ok(Ok(out)) => apply(world, &job.objects, &job.probes, &job.folder, out).map(|n| {
            format!(
                "Baked {n} lightmap(s) in {seconds:.1} s into {}",
                job.folder
            )
        }),
        Ok(Err(e)) => Err(e),
        Err(_) => Err("the lightmapper crashed".into()),
    };
    world.resource_mut::<LightingBake>().last = Some(result.clone());
    Some(result)
}

fn write_png(path: &std::path::Path, size: u32, rgba: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), size, size);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .and_then(|mut w| w.write_image_data(rgba))
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn apply(
    world: &mut World,
    objects: &[GameObjectId],
    probes: &[Vector3],
    folder: &str,
    out: BakeOutput,
) -> Result<usize, String> {
    let mut handles = Vec::new();
    for (i, lm) in out.lightmaps.iter().enumerate() {
        let path = format!("{folder}/Lightmap-{i}_comp_light.png");
        let assets = world.resource_mut::<AssetServer>();
        write_png(&assets.full_path(&path), lm.size, &lm.rgbm)?;
        // RGBM data: linear, uncompressed, no mipmaps, clamped.
        let importer = oxide_assets::TextureImporter {
            srgb: false,
            compression: oxide_assets::TextureCompression::None,
            generate_mip_maps: false,
            max_size: 8192,
            wrap_mode: oxide_assets::TextureWrapMode::Clamp,
            filter_mode: oxide_assets::FilterMode::Bilinear,
            ..Default::default()
        };
        handles.push(assets.set_texture_importer(&path, &importer)?);
    }
    for (id, place) in objects.iter().zip(&out.placements) {
        if let Some(r) = world.get_component_mut::<MeshRenderer>(*id) {
            match place {
                Some((index, st)) => {
                    r.lightmap_index = *index as i32;
                    r.lightmap_scale_offset = *st;
                }
                None => r.lightmap_index = -1,
            }
        }
    }
    let rs = world.resource_mut::<RenderSettings>();
    rs.lightmaps = handles;
    rs.light_probe_positions = probes.to_vec();
    rs.light_probe_coefficients = out
        .probes
        .iter()
        .flat_map(|sh| sh.iter().flat_map(|c| c.to_array()))
        .collect();
    Ok(out.lightmaps.len())
}

/// Light probe SH at `p`: the 4 nearest baked probes, weighted by inverse squared distance.
pub fn probe_sh(rs: &RenderSettings, p: Vector3) -> Option<[Vector3; 9]> {
    let n = rs.light_probe_positions.len();
    if n == 0 || rs.light_probe_coefficients.len() < n * 27 {
        return None;
    }
    let mut nearest: Vec<(f32, usize)> = rs
        .light_probe_positions
        .iter()
        .enumerate()
        .map(|(i, q)| (q.distance_squared(p), i))
        .collect();
    let k = nearest.len().min(4);
    nearest.select_nth_unstable_by(k - 1, |a, b| a.0.total_cmp(&b.0));
    let mut sh = [Vector3::ZERO; 9];
    let mut total = 0.0;
    for &(d2, i) in &nearest[..k] {
        let w = 1.0 / (d2 + 1e-3);
        total += w;
        let c = &rs.light_probe_coefficients[i * 27..i * 27 + 27];
        for (j, s) in sh.iter_mut().enumerate() {
            *s += Vector3::new(c[j * 3], c[j * 3 + 1], c[j * 3 + 2]) * w;
        }
    }
    Some(sh.map(|s| s / total))
}
