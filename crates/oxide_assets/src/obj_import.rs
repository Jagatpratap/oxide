//! Wavefront OBJ import (`.obj` with its `.mtl` materials), like Unity's OBJ support: one
//! node per `o`/`g` group, one sub-mesh per `usemtl` material, polygons triangulated as fans.
//! OBJ is right-handed; X is negated to Unity's left-handed space (as Unity does).

use crate::handle::Handle;
use crate::material::Material;
use crate::mesh::Mesh;
use crate::model::{Model, ModelNode};
use crate::server::AssetServer;
use crate::texture::Texture2D;
use oxide_math::{Color, Quaternion, Vector2, Vector3};
use std::collections::HashMap;
use std::path::Path;

/// A material from an `.mtl` file.
#[derive(Clone, Debug, Default)]
struct MtlMaterial {
    diffuse: Option<[f32; 3]>,
    emissive: Option<[f32; 3]>,
    specular_exponent: Option<f32>,
    opacity: Option<f32>,
    diffuse_map: Option<String>,
    normal_map: Option<String>,
    emissive_map: Option<String>,
    /// PBR extension: `Pm`, `Pr`.
    metallic: Option<f32>,
    roughness: Option<f32>,
}

fn parse_mtl(text: &str) -> HashMap<String, MtlMaterial> {
    let mut out = HashMap::new();
    let mut cur: Option<(String, MtlMaterial)> = None;
    let f3 = |w: &[&str]| -> Option<[f32; 3]> {
        Some([
            w.first()?.parse().ok()?,
            w.get(1)?.parse().ok()?,
            w.get(2)?.parse().ok()?,
        ])
    };
    for line in text.lines() {
        let line = line.trim();
        let mut w = line.split_whitespace();
        let Some(key) = w.next() else { continue };
        let rest: Vec<&str> = w.collect();
        // Texture maps may have options (`-bm 1.0 file.png`): the file is the last word.
        let file = || rest.last().map(|s| (*s).to_owned());
        match key {
            "newmtl" => {
                if let Some((n, m)) = cur.take() {
                    out.insert(n, m);
                }
                cur = Some((rest.join(" "), MtlMaterial::default()));
            }
            _ => {
                let Some((_, m)) = cur.as_mut() else { continue };
                match key {
                    "Kd" => m.diffuse = f3(&rest),
                    "Ke" => m.emissive = f3(&rest),
                    "Ns" => m.specular_exponent = rest.first().and_then(|s| s.parse().ok()),
                    "d" => m.opacity = rest.first().and_then(|s| s.parse().ok()),
                    "Tr" => {
                        m.opacity = rest
                            .first()
                            .and_then(|s| s.parse::<f32>().ok())
                            .map(|t| 1.0 - t)
                    }
                    "Pm" => m.metallic = rest.first().and_then(|s| s.parse().ok()),
                    "Pr" => m.roughness = rest.first().and_then(|s| s.parse().ok()),
                    "map_Kd" => m.diffuse_map = file(),
                    "map_Bump" | "map_bump" | "bump" | "norm" => m.normal_map = file(),
                    "map_Ke" => m.emissive_map = file(),
                    _ => {}
                }
            }
        }
    }
    if let Some((n, m)) = cur {
        out.insert(n, m);
    }
    out
}

fn conv(v: [f32; 3]) -> Vector3 {
    Vector3::new(-v[0], v[1], v[2])
}

#[derive(Default)]
struct Group {
    name: String,
    /// (material name, triangles as (v, vt, vn) index triples).
    parts: Vec<(String, Vec<[i64; 3]>)>,
}

/// Imports `bytes` (an `.obj` at `path`) as a [`Model`] with its meshes and materials.
pub fn import_obj(
    server: &mut AssetServer,
    path: &str,
    bytes: &[u8],
) -> Result<Handle<Model>, String> {
    let text = String::from_utf8_lossy(bytes);
    let dir = Path::new(path)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let rel = |f: &str| {
        let p = dir.join(f.replace('\\', "/"));
        p.to_string_lossy().replace('\\', "/")
    };
    let (mut v, mut vt, mut vn) = (
        Vec::<[f32; 3]>::new(),
        Vec::<[f32; 2]>::new(),
        Vec::<[f32; 3]>::new(),
    );
    let mut colors = Vec::<[f32; 3]>::new();
    let mut mtl = HashMap::new();
    let mut groups: Vec<Group> = vec![Group {
        name: "default".into(),
        parts: Vec::new(),
    }];
    let mut material = String::new();
    // OBJ indices are 1-based; negative ones count back from the end.
    let index = |s: &str, len: usize| -> i64 {
        match s.parse::<i64>() {
            Ok(i) if i < 0 => len as i64 + i,
            Ok(i) => i - 1,
            Err(_) => -1,
        }
    };
    for line in text.lines() {
        let line = line.trim();
        let mut w = line.split_whitespace();
        let Some(key) = w.next() else { continue };
        let rest: Vec<&str> = w.collect();
        let num = |i: usize| {
            rest.get(i)
                .and_then(|s| s.parse::<f32>().ok())
                .unwrap_or(0.0)
        };
        match key {
            "v" => {
                v.push([num(0), num(1), num(2)]);
                // Vertex colours (`v x y z r g b`), a common extension.
                if rest.len() >= 6 {
                    colors.push([num(3), num(4), num(5)]);
                }
            }
            "vt" => vt.push([num(0), num(1)]),
            "vn" => vn.push([num(0), num(1), num(2)]),
            "mtllib" => {
                let f = rel(&rest.join(" "));
                match std::fs::read_to_string(server.full_path(&f)) {
                    Ok(t) => mtl.extend(parse_mtl(&t)),
                    Err(e) => oxide_core::debug_log_warning!("{path}: material library {f}: {e}"),
                }
            }
            "usemtl" => material = rest.join(" "),
            "o" | "g" => {
                let name = if rest.is_empty() {
                    "default".to_owned()
                } else {
                    rest.join(" ")
                };
                if groups.last().is_some_and(|g| g.parts.is_empty()) {
                    groups.last_mut().expect("group").name = name;
                } else {
                    groups.push(Group {
                        name,
                        parts: Vec::new(),
                    });
                }
            }
            "f" => {
                let corners: Vec<[i64; 3]> = rest
                    .iter()
                    .map(|c| {
                        let mut p = c.split('/');
                        let vi = p.next().map_or(-1, |s| index(s, v.len()));
                        let ti = p
                            .next()
                            .map_or(-1, |s| if s.is_empty() { -1 } else { index(s, vt.len()) });
                        let ni = p
                            .next()
                            .map_or(-1, |s| if s.is_empty() { -1 } else { index(s, vn.len()) });
                        [vi, ti, ni]
                    })
                    .collect();
                if corners.len() < 3 {
                    continue;
                }
                let g = groups.last_mut().expect("group");
                if g.parts.last().is_none_or(|(m, _)| *m != material) {
                    g.parts.push((material.clone(), Vec::new()));
                }
                let tris = &mut g.parts.last_mut().expect("part").1;
                for i in 1..corners.len() - 1 {
                    tris.extend([corners[0], corners[i], corners[i + 1]]);
                }
            }
            _ => {}
        }
    }
    groups.retain(|g| !g.parts.is_empty());
    if groups.is_empty() {
        return Err(format!("{path}: no faces"));
    }

    // Materials (shared by every group), textures loaded from the project.
    let mut mats: HashMap<String, Handle<Material>> = HashMap::new();
    let mut material_for = |server: &mut AssetServer, name: &str| -> Handle<Material> {
        if let Some(h) = mats.get(name) {
            return h.clone();
        }
        let Some(m) = mtl.get(name) else {
            return Handle::from_path(crate::server::DEFAULT_MATERIAL);
        };
        let mut tex = |f: &Option<String>, srgb: bool| -> Option<Handle<Texture2D>> {
            let p = rel(f.as_deref()?);
            match server.load_texture(&p) {
                Ok(h) => {
                    if !srgb && let Some(t) = server.textures.get_mut(&h) {
                        t.srgb = false;
                    }
                    Some(h)
                }
                Err(e) => {
                    oxide_core::debug_log_warning!("{path}: {e}");
                    None
                }
            }
        };
        let d = m.diffuse.unwrap_or([1.0; 3]);
        let mut mat = Material {
            name: name.to_owned(),
            base_color: Color::new(d[0], d[1], d[2], m.opacity.unwrap_or(1.0)),
            metallic: m.metallic.unwrap_or(0.0),
            smoothness: m
                .roughness
                .map(|r| 1.0 - r)
                .or(m
                    .specular_exponent
                    .map(|n| (n / 1000.0).sqrt().clamp(0.0, 1.0)))
                .unwrap_or(0.5),
            ..Default::default()
        };
        if m.opacity.is_some_and(|o| o < 0.999) {
            mat.surface_type = crate::material::SurfaceType::Transparent;
        }
        mat.base_map = tex(&m.diffuse_map, true);
        mat.normal_map = tex(&m.normal_map, false);
        if let Some(e) = m.emissive.filter(|e| e.iter().any(|&c| c > 0.0)) {
            mat.emission_color = Color::new(e[0], e[1], e[2], 1.0);
        }
        mat.emission_map = tex(&m.emissive_map, true);
        if mat.emission_map.is_some() && m.emissive.is_none() {
            mat.emission_color = Color::WHITE;
        }
        let h = server.materials.insert(format!("{path}#mat_{name}"), mat);
        mats.insert(name.to_owned(), h.clone());
        h
    };

    let mut nodes = Vec::new();
    for (gi, g) in groups.iter().enumerate() {
        let mut mesh = Mesh {
            name: g.name.clone(),
            ..Default::default()
        };
        let mut materials = Vec::new();
        let mut remap: HashMap<[i64; 3], u32> = HashMap::new();
        let has_normals = g.parts.iter().flat_map(|p| &p.1).all(|c| c[2] >= 0);
        for (mat_name, tris) in &g.parts {
            let start = mesh.indices.len() as u32;
            for c in tris {
                let i = *remap.entry(*c).or_insert_with(|| {
                    let p = v.get(c[0] as usize).copied().unwrap_or([0.0; 3]);
                    mesh.vertices.push(conv(p));
                    if !vt.is_empty() {
                        let t = vt.get(c[1] as usize).copied().unwrap_or([0.0; 2]);
                        mesh.uv.push(Vector2::new(t[0], 1.0 - t[1]));
                    }
                    if has_normals {
                        let n = vn.get(c[2] as usize).copied().unwrap_or([0.0, 1.0, 0.0]);
                        mesh.normals.push(conv(n));
                    }
                    if colors.len() == v.len() {
                        let k = colors[c[0] as usize];
                        mesh.colors
                            .push(oxide_math::Vector4::new(k[0], k[1], k[2], 1.0));
                    }
                    mesh.vertices.len() as u32 - 1
                });
                mesh.indices.push(i);
            }
            mesh.submeshes.push(start..mesh.indices.len() as u32);
            materials.push(material_for(server, mat_name));
        }
        mesh.finish();
        let h = server.meshes.insert(format!("{path}#mesh{gi}"), mesh);
        nodes.push(ModelNode {
            name: g.name.clone(),
            position: Vector3::ZERO,
            rotation: Quaternion::IDENTITY,
            scale: Vector3::ONE,
            children: Vec::new(),
            mesh: Some(h),
            materials,
            skin: None,
        });
    }
    let name = Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Model")
        .to_owned();
    let model = Model {
        name,
        roots: (0..nodes.len()).collect(),
        nodes,
        skins: Vec::new(),
        animations: Vec::new(),
    };
    Ok(server.models.insert(path.to_owned(), model))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_groups_materials_and_quads() {
        let dir = std::env::temp_dir().join(format!("oxide_obj_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Assets")).unwrap();
        std::fs::write(
            dir.join("Assets/box.mtl"),
            "newmtl Red\nKd 1 0 0\nNs 250\nnewmtl Glass\nKd 0.5 0.5 1\nd 0.4\n",
        )
        .unwrap();
        let obj = "mtllib box.mtl\n\
                   v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nv 0 0 1\n\
                   vt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\n\
                   vn 0 0 1\n\
                   o Front\nusemtl Red\nf 1/1/1 2/2/1 3/3/1 4/4/1\n\
                   o Side\nusemtl Glass\nf 1 2 5\nusemtl Red\nf -1 -2 -3\n";
        let mut s = AssetServer::new(&dir);
        let h = import_obj(&mut s, "Assets/box.obj", obj.as_bytes()).unwrap();
        let m = s.models.get(&h).unwrap();
        assert_eq!(m.nodes.len(), 2);
        assert_eq!(m.nodes[0].name, "Front");
        let front = s.meshes.get(m.nodes[0].mesh.as_ref().unwrap()).unwrap();
        // A quad: 4 vertices, 2 triangles, X negated, V flipped.
        assert_eq!(front.vertices.len(), 4);
        assert_eq!(front.indices.len(), 6);
        assert_eq!(front.vertices[1], Vector3::new(-1.0, 0.0, 0.0));
        assert_eq!(front.uv[2], Vector2::new(1.0, 0.0));
        assert_eq!(front.normals[0], Vector3::new(0.0, 0.0, 1.0));
        // Two materials on the side group, in order.
        let side = s.meshes.get(m.nodes[1].mesh.as_ref().unwrap()).unwrap();
        assert_eq!(side.submeshes.len(), 2);
        let glass = s.materials.get(&m.nodes[1].materials[0]).unwrap();
        assert_eq!(
            glass.surface_type,
            crate::material::SurfaceType::Transparent
        );
        let red = s.materials.get(&m.nodes[0].materials[0]).unwrap();
        assert_eq!(red.base_color, Color::new(1.0, 0.0, 0.0, 1.0));
        assert!(red.smoothness > 0.4);
    }
}
