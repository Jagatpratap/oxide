//! glTF 2.0 (`.gltf` / `.glb`) import, converted to Unity's conventions: right-handed glTF
//! space is mirrored on Z into left-handed space (which also turns glTF's counter-clockwise
//! front faces into Unity's clockwise ones), and UVs flip to a bottom-left origin.
//! Reference: <https://docs.unity3d.com/Manual/ImportingModelFiles.html>

use crate::animation::{AnimationClip, Interpolation, Track, TransformCurves};
use crate::handle::Handle;
use crate::material::{Material, SurfaceType};
use crate::mesh::Mesh;
use crate::model::{Model, ModelNode, ModelSkin};
use crate::server::AssetServer;
use crate::texture::{FilterMode, Texture2D, TextureWrapMode};
use oxide_math::{Color, Matrix4x4, Quaternion, Vector2, Vector3, Vector4};
use std::collections::HashMap;

fn pos(v: [f32; 3]) -> Vector3 {
    Vector3::new(v[0], v[1], -v[2])
}

fn rot(q: [f32; 4]) -> Quaternion {
    Quaternion::from_xyzw(-q[0], -q[1], q[2], q[3])
}

fn mirror(m: Matrix4x4) -> Matrix4x4 {
    let s = Matrix4x4::from_scale(Vector3::new(1.0, 1.0, -1.0));
    s * m * s
}

fn to_rgba8(img: &gltf::image::Data) -> Vec<u8> {
    use gltf::image::Format as F;
    let p = &img.pixels;
    let n = (img.width * img.height) as usize;
    let mut out = Vec::with_capacity(n * 4);
    let u16_at = |i: usize| (u16::from_le_bytes([p[i * 2], p[i * 2 + 1]]) >> 8) as u8;
    let f32_at = |i: usize| {
        let v = f32::from_le_bytes([p[i * 4], p[i * 4 + 1], p[i * 4 + 2], p[i * 4 + 3]]);
        (v.clamp(0.0, 1.0) * 255.0) as u8
    };
    for i in 0..n {
        let px: [u8; 4] = match img.format {
            F::R8 => [p[i], p[i], p[i], 255],
            F::R8G8 => [p[i * 2], p[i * 2 + 1], 0, 255],
            F::R8G8B8 => [p[i * 3], p[i * 3 + 1], p[i * 3 + 2], 255],
            F::R8G8B8A8 => [p[i * 4], p[i * 4 + 1], p[i * 4 + 2], p[i * 4 + 3]],
            F::R16 => {
                let v = u16_at(i);
                [v, v, v, 255]
            }
            F::R16G16 => [u16_at(i * 2), u16_at(i * 2 + 1), 0, 255],
            F::R16G16B16 => [u16_at(i * 3), u16_at(i * 3 + 1), u16_at(i * 3 + 2), 255],
            F::R16G16B16A16 => [
                u16_at(i * 4),
                u16_at(i * 4 + 1),
                u16_at(i * 4 + 2),
                u16_at(i * 4 + 3),
            ],
            F::R32G32B32FLOAT => [f32_at(i * 3), f32_at(i * 3 + 1), f32_at(i * 3 + 2), 255],
            F::R32G32B32A32FLOAT => [
                f32_at(i * 4),
                f32_at(i * 4 + 1),
                f32_at(i * 4 + 2),
                f32_at(i * 4 + 3),
            ],
        };
        out.extend_from_slice(&px);
    }
    out
}

struct Importer<'a> {
    server: &'a mut AssetServer,
    path: String,
    doc: gltf::Document,
    buffers: Vec<gltf::buffer::Data>,
    images: Vec<gltf::image::Data>,
    textures: HashMap<(usize, bool), Handle<Texture2D>>,
}

impl Importer<'_> {
    fn texture(&mut self, tex: gltf::Texture<'_>, srgb: bool) -> Option<Handle<Texture2D>> {
        let key = (tex.index(), srgb);
        if let Some(h) = self.textures.get(&key) {
            return Some(h.clone());
        }
        let img = self.images.get(tex.source().index())?;
        let mut t = Texture2D::from_rgba8(
            tex.name().unwrap_or("texture"),
            img.width,
            img.height,
            to_rgba8(img),
            srgb,
        );
        use gltf::texture::{MagFilter, WrappingMode};
        let s = tex.sampler();
        t.wrap_mode = match s.wrap_s() {
            WrappingMode::ClampToEdge => TextureWrapMode::Clamp,
            WrappingMode::MirroredRepeat => TextureWrapMode::Mirror,
            WrappingMode::Repeat => TextureWrapMode::Repeat,
        };
        if s.mag_filter() == Some(MagFilter::Nearest) {
            t.filter_mode = FilterMode::Point;
        }
        let suffix = if srgb { "" } else { "_linear" };
        let h = self
            .server
            .textures
            .insert(format!("{}#tex{}{}", self.path, tex.index(), suffix), t);
        self.textures.insert(key, h.clone());
        Some(h)
    }

    fn material(&mut self, m: gltf::Material<'_>) -> Handle<Material> {
        let Some(index) = m.index() else {
            return Handle::from_path("builtin:Default-Material");
        };
        let pbr = m.pbr_metallic_roughness();
        let c = pbr.base_color_factor();
        let mut mat = Material {
            name: m.name().unwrap_or("Material").to_owned(),
            // glTF factors are linear; Unity's _BaseColor is sRGB.
            base_color: Color::new(c[0], c[1], c[2], c[3]).gamma(),
            metallic: pbr.metallic_factor(),
            smoothness: 1.0 - pbr.roughness_factor(),
            double_sided: m.double_sided(),
            alpha_cutoff: m.alpha_cutoff().unwrap_or(0.5),
            surface_type: match m.alpha_mode() {
                gltf::material::AlphaMode::Opaque => SurfaceType::Opaque,
                gltf::material::AlphaMode::Mask => SurfaceType::Cutout,
                gltf::material::AlphaMode::Blend => SurfaceType::Transparent,
            },
            ..Default::default()
        };
        if let Some(info) = pbr.base_color_texture() {
            if let Some(tr) = info.texture_transform() {
                let (s, o) = (tr.scale(), tr.offset());
                mat.tiling = Vector2::new(s[0], s[1]);
                mat.offset = Vector2::new(o[0], o[1]);
            }
            mat.base_map = self.texture(info.texture(), true);
        }
        if let Some(info) = pbr.metallic_roughness_texture() {
            mat.metallic_gloss_map = self.texture(info.texture(), false);
        }
        if let Some(n) = m.normal_texture() {
            mat.normal_scale = n.scale();
            mat.normal_map = self.texture(n.texture(), false);
        }
        if let Some(o) = m.occlusion_texture() {
            mat.occlusion_strength = o.strength();
            mat.occlusion_map = self.texture(o.texture(), false);
        }
        let e = m.emissive_factor();
        mat.emission_color = Color::new(e[0], e[1], e[2], 1.0).gamma();
        mat.emission_intensity = m.emissive_strength().unwrap_or(1.0);
        if let Some(info) = m.emissive_texture() {
            mat.emission_map = self.texture(info.texture(), true);
            if e == [0.0; 3] {
                mat.emission_color = Color::WHITE;
            }
        }
        self.server
            .materials
            .insert(format!("{}#mat{}", self.path, index), mat)
    }

    fn mesh(&mut self, mesh: gltf::Mesh<'_>) -> (Handle<Mesh>, Vec<Handle<Material>>) {
        let mut out = Mesh {
            name: mesh.name().unwrap_or("Mesh").to_owned(),
            ..Default::default()
        };
        let mut materials = Vec::new();
        for prim in mesh.primitives() {
            if prim.mode() != gltf::mesh::Mode::Triangles {
                continue;
            }
            let reader = prim.reader(|b| self.buffers.get(b.index()).map(|d| &d.0[..]));
            let Some(positions) = reader.read_positions() else {
                continue;
            };
            let vertices: Vec<Vector3> = positions.map(pos).collect();
            let n = vertices.len();
            let mut p = Mesh {
                vertices,
                ..Default::default()
            };
            if let Some(normals) = reader.read_normals() {
                p.normals = normals.map(pos).collect();
            }
            if let Some(uv) = reader.read_tex_coords(0) {
                p.uv = uv
                    .into_f32()
                    .map(|t| Vector2::new(t[0], 1.0 - t[1]))
                    .collect();
            }
            if let Some(uv) = reader.read_tex_coords(1) {
                p.uv2 = uv
                    .into_f32()
                    .map(|t| Vector2::new(t[0], 1.0 - t[1]))
                    .collect();
            }
            if let Some(c) = reader.read_colors(0) {
                p.colors = c.into_rgba_f32().map(Vector4::from_array).collect();
            }
            if let (Some(j), Some(w)) = (reader.read_joints(0), reader.read_weights(0)) {
                p.bone_indices = j.into_u16().collect();
                p.bone_weights = w
                    .into_f32()
                    .map(|w| {
                        let v = Vector4::from_array(w);
                        let s = v.x + v.y + v.z + v.w;
                        if s > 0.0 { v / s } else { Vector4::X }
                    })
                    .collect();
            }
            p.indices = match reader.read_indices() {
                Some(i) => i.into_u32().collect(),
                None => (0..n as u32).collect(),
            };
            p.finish();
            out.append(&p);
            materials.push(self.material(prim.material()));
        }
        // Lightmap UVs come from the Model Importer's Generate Lightmap UVs.
        out.finish();
        let h = self
            .server
            .meshes
            .insert(format!("{}#mesh{}", self.path, mesh.index()), out);
        (h, materials)
    }

    fn animation(&mut self, anim: gltf::Animation<'_>, paths: &[String]) -> Handle<AnimationClip> {
        let mut by_node: HashMap<usize, TransformCurves> = HashMap::new();
        for ch in anim.channels() {
            let node = ch.target().node().index();
            let interp = match ch.sampler().interpolation() {
                gltf::animation::Interpolation::Step => Interpolation::Step,
                gltf::animation::Interpolation::Linear => Interpolation::Linear,
                gltf::animation::Interpolation::CubicSpline => Interpolation::CubicSpline,
            };
            let reader = ch.reader(|b| self.buffers.get(b.index()).map(|d| &d.0[..]));
            let (Some(inputs), Some(outputs)) = (reader.read_inputs(), reader.read_outputs())
            else {
                continue;
            };
            let times: Vec<f32> = inputs.collect();
            let curves = by_node.entry(node).or_insert_with(|| TransformCurves {
                path: paths[node].clone(),
                ..Default::default()
            });
            use gltf::animation::util::ReadOutputs;
            match outputs {
                ReadOutputs::Translations(v) => {
                    curves.position = Some(Track {
                        times,
                        values: v.map(pos).collect(),
                        interpolation: interp,
                    })
                }
                ReadOutputs::Rotations(r) => {
                    curves.rotation = Some(Track {
                        times,
                        values: r.into_f32().map(rot).collect(),
                        interpolation: interp,
                    })
                }
                ReadOutputs::Scales(v) => {
                    curves.scale = Some(Track {
                        times,
                        values: v.map(Vector3::from_array).collect(),
                        interpolation: interp,
                    })
                }
                ReadOutputs::MorphTargetWeights(_) => {}
            }
        }
        let mut clip = AnimationClip {
            name: anim
                .name()
                .map_or_else(|| format!("Take {}", anim.index()), str::to_owned),
            looping: true,
            curves: by_node.into_values().collect(),
            ..Default::default()
        };
        clip.curves.sort_by(|a, b| a.path.cmp(&b.path));
        clip.recalculate_length();
        self.server
            .animations
            .insert(format!("{}#anim{}", self.path, anim.index()), clip)
    }
}

/// A copy of a `.glb`/`.gltf` whose textures with an `EXT_texture_webp` image use it as
/// their `source` (WebP-only files leave `source` out), with the extension dropped from
/// `extensionsRequired`/`extensionsUsed`. `None` when the file doesn't use the extension.
fn with_webp_sources(bytes: &[u8]) -> Option<Vec<u8>> {
    use serde_json::Value;
    const EXT: &str = "EXT_texture_webp";
    let glb = bytes.starts_with(b"glTF");
    let json = if glb {
        let len = u32::from_le_bytes(bytes.get(12..16)?.try_into().ok()?) as usize;
        (bytes.get(16..20)? == b"JSON").then_some(bytes.get(20..20 + len)?)?
    } else {
        bytes
    };
    if !json.windows(EXT.len()).any(|w| w == EXT.as_bytes()) {
        return None;
    }
    let mut v: Value = serde_json::from_slice(json).ok()?;
    for t in v
        .get_mut("textures")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        let src = t.pointer(&format!("/extensions/{EXT}/source")).cloned();
        if let Some(src) = src {
            if t.get("source").is_none() {
                t["source"] = src;
            }
            if let Some(e) = t.get_mut("extensions").and_then(Value::as_object_mut) {
                e.remove(EXT);
            }
        }
    }
    for k in ["extensionsRequired", "extensionsUsed"] {
        if let Some(a) = v.get_mut(k).and_then(Value::as_array_mut) {
            a.retain(|e| e.as_str() != Some(EXT));
        }
    }
    let mut text = serde_json::to_vec(&v).ok()?;
    if !glb {
        return Some(text);
    }
    // Rebuild the GLB: header, the new JSON chunk (space-padded to 4 bytes), the rest.
    while text.len() % 4 != 0 {
        text.push(b' ');
    }
    let old_len = u32::from_le_bytes(bytes[12..16].try_into().ok()?) as usize;
    let rest = bytes.get(20 + old_len..)?;
    let total = 12 + 8 + text.len() + rest.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&bytes[..8]);
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(text.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&text);
    out.extend_from_slice(rest);
    Some(out)
}

/// Decodes every image with the `image` crate (PNG, JPEG, WebP, ...), as RGBA8.
fn decode_images(
    doc: &gltf::Document,
    base_dir: Option<&std::path::Path>,
    buffers: &[gltf::buffer::Data],
) -> Result<Vec<gltf::image::Data>, String> {
    doc.images()
        .map(|img| {
            let bytes: Vec<u8> = match img.source() {
                gltf::image::Source::View { view, .. } => {
                    let b = &buffers[view.buffer().index()];
                    b.get(view.offset()..view.offset() + view.length())
                        .ok_or("image view out of range")?
                        .to_vec()
                }
                gltf::image::Source::Uri { uri, .. } => {
                    if let Some(data) = uri.strip_prefix("data:") {
                        use base64::Engine as _;
                        let b64 = data.split_once(";base64,").ok_or("bad data URI")?.1;
                        base64::engine::general_purpose::STANDARD
                            .decode(b64)
                            .map_err(|e| e.to_string())?
                    } else {
                        let dir = base_dir.ok_or("external image without a folder")?;
                        let p = dir.join(uri.replace("%20", " "));
                        std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?
                    }
                }
            };
            let rgba = image::load_from_memory(&bytes)
                .map_err(|e| e.to_string())?
                .to_rgba8();
            Ok(gltf::image::Data {
                width: rgba.width(),
                height: rgba.height(),
                format: gltf::image::Format::R8G8B8A8,
                pixels: rgba.into_raw(),
            })
        })
        .collect()
}

/// Imports a glTF file (already read into `bytes`; `base_dir` resolves external buffers and
/// images) as a [`Model`] stored under `path`, plus its meshes, materials, textures and clips.
pub fn import_gltf(
    server: &mut AssetServer,
    path: &str,
    bytes: &[u8],
    base_dir: Option<&std::path::Path>,
) -> Result<Handle<Model>, String> {
    // EXT_texture_webp: point the textures at their WebP images so the gltf crate accepts
    // the file, and decode the images here (the crate only decodes PNG and JPEG).
    let webp = with_webp_sources(bytes);
    let gltf::Gltf { document, blob } = gltf::Gltf::from_slice(webp.as_deref().unwrap_or(bytes))
        .map_err(|e| format!("{path}: {e}"))?;
    let buffers = gltf::import_buffers(&document, base_dir, blob)
        .map_err(|e| format!("{path}: buffers: {e}"))?;
    let images = if webp.is_some() {
        decode_images(&document, base_dir, &buffers)
    } else {
        gltf::import_images(&document, base_dir, &buffers).map_err(|e| e.to_string())
    }
    .map_err(|e| format!("{path}: images: {e}"))?;

    let mut imp = Importer {
        server,
        path: path.to_owned(),
        doc: document,
        buffers,
        images,
        textures: HashMap::new(),
    };
    let doc = imp.doc.clone();

    // Unique, Unity-style names and binding paths for every node.
    let mut names: Vec<String> = doc
        .nodes()
        .map(|n| {
            n.name()
                .map_or_else(|| format!("Node{}", n.index()), str::to_owned)
        })
        .collect();
    let mut parent = vec![None; names.len()];
    for n in doc.nodes() {
        for c in n.children() {
            parent[c.index()] = Some(n.index());
        }
    }
    {
        let mut seen: HashMap<(Option<usize>, String), u32> = HashMap::new();
        for i in 0..names.len() {
            let k = seen.entry((parent[i], names[i].clone())).or_default();
            if *k > 0 {
                names[i] = format!("{} ({})", names[i], k);
            }
            *k += 1;
        }
    }
    let paths: Vec<String> = (0..names.len())
        .map(|mut i| {
            let mut parts = vec![names[i].as_str()];
            while let Some(p) = parent[i] {
                parts.push(&names[p]);
                i = p;
            }
            parts.reverse();
            parts.join("/")
        })
        .collect();

    let mut meshes: HashMap<usize, (Handle<Mesh>, Vec<Handle<Material>>)> = HashMap::new();
    let mut nodes = Vec::new();
    for n in doc.nodes() {
        let (t, r, s) = n.transform().decomposed();
        let (mesh, materials) = match n.mesh() {
            Some(m) => {
                let (h, mats) = meshes
                    .entry(m.index())
                    .or_insert_with(|| imp.mesh(m))
                    .clone();
                (Some(h), mats)
            }
            None => (None, Vec::new()),
        };
        nodes.push(ModelNode {
            name: names[n.index()].clone(),
            position: pos(t),
            rotation: rot(r),
            scale: Vector3::from_array(s),
            children: n.children().map(|c| c.index()).collect(),
            mesh,
            materials,
            skin: n.skin().map(|s| s.index()),
        });
    }

    let mut skins = Vec::new();
    for skin in doc.skins() {
        let joints: Vec<usize> = skin.joints().map(|j| j.index()).collect();
        let reader = skin.reader(|b| imp.buffers.get(b.index()).map(|d| &d.0[..]));
        let inverse: Vec<Matrix4x4> = match reader.read_inverse_bind_matrices() {
            Some(m) => m
                .map(|m| mirror(Matrix4x4::from_cols_array_2d(&m)))
                .collect(),
            None => vec![Matrix4x4::IDENTITY; joints.len()],
        };
        // Unity keeps bind poses on the mesh; give each skinned mesh its skin's poses.
        for node in nodes.iter().filter(|n| n.skin == Some(skin.index())) {
            if let Some(h) = &node.mesh
                && let Some(mesh) = imp.server.meshes.get_mut(h)
            {
                mesh.bindposes = inverse.clone();
            }
        }
        skins.push(ModelSkin { joints });
    }

    let animations = doc.animations().map(|a| imp.animation(a, &paths)).collect();
    let scene = doc.default_scene().or_else(|| doc.scenes().next());
    let roots = match scene {
        Some(s) => s.nodes().map(|n| n.index()).collect(),
        None => (0..nodes.len()).filter(|&i| parent[i].is_none()).collect(),
    };
    let name = std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Model")
        .to_owned();
    let model = Model {
        name,
        nodes,
        roots,
        skins,
        animations,
    };
    Ok(imp.server.models.insert(path.to_owned(), model))
}

#[cfg(test)]
mod tests {
    #[test]
    fn webp_textures_get_their_source() {
        let json = br#"{"asset":{"version":"2.0"},"extensionsRequired":["EXT_texture_webp"],"extensionsUsed":["EXT_texture_webp"],"textures":[{"extensions":{"EXT_texture_webp":{"source":3}}}]}"#;
        let mut glb = b"glTF".to_vec();
        glb.extend(2u32.to_le_bytes());
        glb.extend(0u32.to_le_bytes());
        glb.extend((json.len() as u32).to_le_bytes());
        glb.extend(b"JSON");
        glb.extend(json);
        glb.extend(b"BINCHUNK");
        let out = super::with_webp_sources(&glb).unwrap();
        let len = u32::from_le_bytes(out[12..16].try_into().unwrap()) as usize;
        assert_eq!(len % 4, 0);
        assert_eq!(
            u32::from_le_bytes(out[8..12].try_into().unwrap()) as usize,
            out.len()
        );
        let v: serde_json::Value = serde_json::from_slice(&out[20..20 + len]).unwrap();
        assert_eq!(v["textures"][0]["source"], 3);
        assert_eq!(v["extensionsRequired"], serde_json::json!([]));
        assert!(out.ends_with(b"BINCHUNK"));
        assert!(super::with_webp_sources(br#"{"asset":{}}"#).is_none());
    }
}
