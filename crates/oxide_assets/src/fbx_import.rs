//! Autodesk FBX import (binary 6.1-7.7 and ASCII), like Unity's Model Importer: the node
//! hierarchy (with pre/post rotation and geometric transforms), meshes (normals, UV sets,
//! vertex colours, one sub-mesh per material), materials with their textures (external or
//! embedded), skinning (clusters → bone weights and bind poses) and animation stacks (baked
//! at 30 fps into clips).
//!
//! Coordinates follow Unity: the file's axis system and unit (GlobalSettings) are converted to
//! Y-up metres, then X is negated into Unity's left-handed space.
//! Reference: <https://docs.unity3d.com/Manual/FBXImporter-Model.html>

use crate::animation::{AnimationClip, Interpolation, Track, TransformCurves};
use crate::handle::Handle;
use crate::material::{Material, SurfaceType};
use crate::mesh::Mesh;
use crate::model::{Model, ModelNode, ModelSkin};
use crate::server::AssetServer;
use crate::texture::Texture2D;
use oxide_math::glam::{DMat3, DMat4, DQuat, DVec3};
use oxide_math::{Color, Matrix4x4, Quaternion, Vector2, Vector3, Vector4};
use std::collections::HashMap;
use std::path::Path;

// ---- document ------------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum Prop {
    Int(i64),
    Float(f64),
    Str(String),
    Raw(Vec<u8>),
    Ints(Vec<i64>),
    Floats(Vec<f64>),
}

impl Prop {
    fn int(&self) -> Option<i64> {
        match self {
            Prop::Int(i) => Some(*i),
            Prop::Float(f) => Some(*f as i64),
            _ => None,
        }
    }
    fn float(&self) -> Option<f64> {
        match self {
            Prop::Float(f) => Some(*f),
            Prop::Int(i) => Some(*i as f64),
            _ => None,
        }
    }
    fn str(&self) -> Option<&str> {
        match self {
            Prop::Str(s) => Some(s),
            _ => None,
        }
    }
    fn ints(&self) -> Option<&[i64]> {
        match self {
            Prop::Ints(v) => Some(v),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct Node {
    name: String,
    props: Vec<Prop>,
    children: Vec<Node>,
}

impl Node {
    fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }
    fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }
    /// The first property of child `name`.
    fn value(&self, name: &str) -> Option<&Prop> {
        self.child(name)?.props.first()
    }
    fn floats(&self, name: &str) -> Vec<f64> {
        match self.value(name) {
            Some(Prop::Floats(v)) => v.clone(),
            // ASCII arrays of whole numbers parse as integers.
            Some(Prop::Ints(v)) => v.iter().map(|&i| i as f64).collect(),
            _ => Vec::new(),
        }
    }
    fn ints(&self, name: &str) -> Vec<i64> {
        match self.value(name) {
            Some(Prop::Ints(v)) => v.clone(),
            Some(Prop::Floats(v)) => v.iter().map(|&f| f as i64).collect(),
            _ => Vec::new(),
        }
    }
    /// A `Properties70` (or 6.x `Properties60`) entry: the values after name/type/label/flags.
    fn prop70(&self, name: &str) -> Option<&[Prop]> {
        let p = self
            .child("Properties70")
            .or_else(|| self.child("Properties60"))?;
        let rec = p
            .children
            .iter()
            .find(|r| r.props.first().and_then(Prop::str) == Some(name))?;
        // P: name, type, label, flags, values... (Properties60 has one fewer header field).
        let skip = if p.name == "Properties60" { 3 } else { 4 };
        rec.props.get(skip..)
    }
    fn p_vec3(&self, name: &str) -> Option<DVec3> {
        let v = self.prop70(name)?;
        Some(DVec3::new(
            v.first()?.float()?,
            v.get(1)?.float()?,
            v.get(2)?.float()?,
        ))
    }
    fn p_f64(&self, name: &str) -> Option<f64> {
        self.prop70(name)?.first()?.float()
    }
    fn p_int(&self, name: &str) -> Option<i64> {
        self.prop70(name)?.first()?.int()
    }
    /// `Name\0\1Class` (binary) or `Class::Name` (ASCII) → `Name`.
    fn object_name(&self) -> String {
        let s = self.props.get(1).and_then(Prop::str).unwrap_or("");
        if let Some((n, _)) = s.split_once("\u{0}\u{1}") {
            n.to_owned()
        } else if let Some((_, n)) = s.split_once("::") {
            n.to_owned()
        } else {
            s.to_owned()
        }
    }
    fn id(&self) -> i64 {
        self.props.first().and_then(Prop::int).unwrap_or(0)
    }
    fn class(&self) -> &str {
        self.props.get(2).and_then(Prop::str).unwrap_or("")
    }
}

// ---- binary parser -------------------------------------------------------------------------

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
    version: u32,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&e| e <= self.b.len())
            .ok_or("unexpected end of file")?;
        let s = &self.b[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("4")))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().expect("8")))
    }
    fn word(&mut self) -> Result<u64, String> {
        if self.version >= 7500 {
            self.u64()
        } else {
            self.u32().map(u64::from)
        }
    }

    fn array<T>(&mut self, size: usize, conv: impl Fn(&[u8]) -> T) -> Result<Vec<T>, String> {
        let len = self.u32()? as usize;
        let encoding = self.u32()?;
        let clen = self.u32()? as usize;
        let data = self.take(clen)?;
        let raw: Vec<u8> = if encoding == 1 {
            use std::io::Read;
            let mut out = Vec::with_capacity(len * size);
            flate2::read::ZlibDecoder::new(data)
                .read_to_end(&mut out)
                .map_err(|e| format!("compressed array: {e}"))?;
            out
        } else {
            data.to_vec()
        };
        if raw.len() < len * size {
            return Err("array shorter than its length".into());
        }
        Ok(raw.chunks_exact(size).take(len).map(conv).collect())
    }

    fn prop(&mut self) -> Result<Prop, String> {
        let t = self.u8()?;
        Ok(match t {
            b'Y' => Prop::Int(i16::from_le_bytes(self.take(2)?.try_into().expect("2")) as i64),
            b'C' => Prop::Int(self.u8()? as i64),
            b'I' => Prop::Int(i32::from_le_bytes(self.take(4)?.try_into().expect("4")) as i64),
            b'L' => Prop::Int(i64::from_le_bytes(self.take(8)?.try_into().expect("8"))),
            b'F' => Prop::Float(f32::from_le_bytes(self.take(4)?.try_into().expect("4")) as f64),
            b'D' => Prop::Float(f64::from_le_bytes(self.take(8)?.try_into().expect("8"))),
            b'f' => Prop::Floats(
                self.array(4, |c| f32::from_le_bytes(c.try_into().expect("4")) as f64)?,
            ),
            b'd' => Prop::Floats(self.array(8, |c| f64::from_le_bytes(c.try_into().expect("8")))?),
            b'i' => {
                Prop::Ints(self.array(4, |c| i32::from_le_bytes(c.try_into().expect("4")) as i64)?)
            }
            b'l' => Prop::Ints(self.array(8, |c| i64::from_le_bytes(c.try_into().expect("8")))?),
            b'b' => Prop::Ints(self.array(1, |c| c[0] as i64)?),
            b'S' => {
                let n = self.u32()? as usize;
                Prop::Str(String::from_utf8_lossy(self.take(n)?).into_owned())
            }
            b'R' => {
                let n = self.u32()? as usize;
                Prop::Raw(self.take(n)?.to_vec())
            }
            other => return Err(format!("unknown property type `{}`", other as char)),
        })
    }

    /// One node, or `None` at a null record (end of a child list).
    fn node(&mut self) -> Result<Option<Node>, String> {
        let end = self.word()? as usize;
        let nprops = self.word()?;
        let _plen = self.word()?;
        let nlen = self.u8()? as usize;
        if end == 0 {
            return Ok(None);
        }
        let name = String::from_utf8_lossy(self.take(nlen)?).into_owned();
        let mut props = Vec::with_capacity(nprops as usize);
        for _ in 0..nprops {
            props.push(self.prop()?);
        }
        let mut children = Vec::new();
        while self.pos < end {
            match self.node()? {
                Some(n) => children.push(n),
                None => break,
            }
        }
        self.pos = end;
        Ok(Some(Node {
            name,
            props,
            children,
        }))
    }
}

fn parse_binary(bytes: &[u8]) -> Result<Vec<Node>, String> {
    let version = u32::from_le_bytes(bytes.get(23..27).ok_or("too short")?.try_into().expect("4"));
    let mut r = Reader {
        b: bytes,
        pos: 27,
        version,
    };
    let mut top = Vec::new();
    // The file ends with a footer after the last null record.
    while r.pos + 13 < bytes.len() {
        match r.node()? {
            Some(n) => top.push(n),
            None => break,
        }
    }
    Ok(top)
}

// ---- ASCII parser --------------------------------------------------------------------------

fn parse_ascii(text: &str) -> Result<Vec<Node>, String> {
    // Tokens: identifiers followed by ':', strings, numbers, ',', '{', '}', '*N'.
    #[derive(Debug, PartialEq)]
    enum Tok {
        Key(String),
        Str(String),
        Num(String),
        Open,
        Close,
    }
    let mut toks = Vec::new();
    let cs: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c == ';' {
            while i < cs.len() && cs[i] != '\n' {
                i += 1;
            }
        } else if c.is_whitespace() || c == ',' {
            i += 1;
        } else if c == '{' {
            toks.push(Tok::Open);
            i += 1;
        } else if c == '}' {
            toks.push(Tok::Close);
            i += 1;
        } else if c == '"' {
            let start = i + 1;
            i = start;
            while i < cs.len() && cs[i] != '"' {
                i += 1;
            }
            toks.push(Tok::Str(cs[start..i.min(cs.len())].iter().collect()));
            i += 1;
        } else {
            let start = i;
            while i < cs.len()
                && !cs[i].is_whitespace()
                && !matches!(cs[i], ',' | '{' | '}' | ':' | '"')
            {
                i += 1;
            }
            let word: String = cs[start..i].iter().collect();
            if i < cs.len() && cs[i] == ':' {
                toks.push(Tok::Key(word));
                i += 1;
            } else if !word.is_empty() {
                toks.push(Tok::Num(word));
            } else {
                i += 1;
            }
        }
    }
    fn parse_list(toks: &[Tok], pos: &mut usize) -> Vec<Node> {
        let mut out = Vec::new();
        while *pos < toks.len() {
            match &toks[*pos] {
                Tok::Close => {
                    *pos += 1;
                    break;
                }
                Tok::Key(k) => {
                    *pos += 1;
                    let mut node = Node {
                        name: k.clone(),
                        ..Default::default()
                    };
                    while *pos < toks.len() {
                        match &toks[*pos] {
                            Tok::Str(s) => {
                                node.props.push(Prop::Str(s.clone()));
                                *pos += 1;
                            }
                            Tok::Num(n) if n.starts_with('*') => *pos += 1,
                            Tok::Num(n) => {
                                node.props.push(match n.parse::<i64>() {
                                    Ok(i) => Prop::Int(i),
                                    Err(_) => {
                                        n.parse::<f64>().map_or(Prop::Str(n.clone()), Prop::Float)
                                    }
                                });
                                *pos += 1;
                            }
                            Tok::Open => {
                                *pos += 1;
                                node.children = parse_list(toks, pos);
                                break;
                            }
                            _ => break,
                        }
                    }
                    // `Key: *N { a: 1,2,3 }` arrays become one array property.
                    if node.props.is_empty()
                        && let [a] = node.children.as_slice()
                        && a.name == "a"
                    {
                        let all_int = a.props.iter().all(|p| matches!(p, Prop::Int(_)));
                        node.props = vec![if all_int {
                            Prop::Ints(a.props.iter().filter_map(Prop::int).collect())
                        } else {
                            Prop::Floats(a.props.iter().filter_map(Prop::float).collect())
                        }];
                        node.children.clear();
                    }
                    out.push(node);
                }
                _ => *pos += 1,
            }
        }
        out
    }
    let mut pos = 0;
    Ok(parse_list(&toks, &mut pos))
}

// ---- scene ---------------------------------------------------------------------------------

/// FBX ticks per second.
const TICKS: f64 = 46_186_158_000.0;

fn euler(v: DVec3, order: i64) -> DQuat {
    let (x, y, z) = (v.x.to_radians(), v.y.to_radians(), v.z.to_radians());
    let (qx, qy, qz) = (
        DQuat::from_rotation_x(x),
        DQuat::from_rotation_y(y),
        DQuat::from_rotation_z(z),
    );
    // FBX eEulerXYZ applies X first: R = Rz * Ry * Rx.
    match order {
        1 => qy * qz * qx, // XZY
        2 => qx * qz * qy, // YZX
        3 => qz * qx * qy, // YXZ
        4 => qy * qx * qz, // ZXY
        5 => qx * qy * qz, // ZYX
        _ => qz * qy * qx, // XYZ
    }
}

/// A Model's local transform in file space (Unity ignores rotation/scale pivots too).
fn local_matrix(m: &Node, t: DVec3, r: DVec3, s: DVec3) -> DMat4 {
    let order = m.p_int("RotationOrder").unwrap_or(0);
    let pre = euler(m.p_vec3("PreRotation").unwrap_or(DVec3::ZERO), 0);
    let post = euler(m.p_vec3("PostRotation").unwrap_or(DVec3::ZERO), 0);
    let rot_off = m.p_vec3("RotationOffset").unwrap_or(DVec3::ZERO);
    let rot_pivot = m.p_vec3("RotationPivot").unwrap_or(DVec3::ZERO);
    let scale_off = m.p_vec3("ScalingOffset").unwrap_or(DVec3::ZERO);
    let scale_pivot = m.p_vec3("ScalingPivot").unwrap_or(DVec3::ZERO);
    DMat4::from_translation(t + rot_off + rot_pivot)
        * DMat4::from_quat(pre * euler(r, order) * post.inverse())
        * DMat4::from_translation(-rot_pivot + scale_off + scale_pivot)
        * DMat4::from_scale(s)
        * DMat4::from_translation(-scale_pivot)
}

fn lcl(m: &Node) -> (DVec3, DVec3, DVec3) {
    (
        m.p_vec3("Lcl Translation").unwrap_or(DVec3::ZERO),
        m.p_vec3("Lcl Rotation").unwrap_or(DVec3::ZERO),
        m.p_vec3("Lcl Scaling").unwrap_or(DVec3::ONE),
    )
}

fn to_f32(m: DMat4) -> Matrix4x4 {
    m.as_mat4()
}

fn trs(m: DMat4) -> (Vector3, Quaternion, Vector3) {
    let (s, r, t) = m.to_scale_rotation_translation();
    (t.as_vec3(), r.as_quat().normalize(), s.as_vec3())
}

struct Fbx<'a> {
    objects: HashMap<i64, &'a Node>,
    /// (child, parent, property) for every connection.
    links: Vec<(i64, i64, String)>,
    /// File space → Unity space (axes, unit, X flip).
    conv: DMat4,
}

impl<'a> Fbx<'a> {
    fn parents(&self, id: i64) -> impl Iterator<Item = (i64, &str)> + '_ {
        self.links
            .iter()
            .filter(move |l| l.0 == id)
            .map(|l| (l.1, l.2.as_str()))
    }
    fn children(&self, id: i64) -> impl Iterator<Item = (i64, &str)> + '_ {
        self.links
            .iter()
            .filter(move |l| l.1 == id)
            .map(|l| (l.0, l.2.as_str()))
    }
    fn child_of_kind(&self, id: i64, kind: &str) -> Option<&'a Node> {
        self.children(id)
            .filter_map(|(c, _)| self.objects.get(&c).copied())
            .find(|n| n.name == kind)
    }
    /// A model's world matrix in file space (from its current local transforms).
    fn global(&self, id: i64) -> DMat4 {
        let mut m = DMat4::IDENTITY;
        let mut cur = Some(id);
        let mut guard = 0;
        while let Some(c) = cur {
            let Some(n) = self.objects.get(&c).filter(|n| n.name == "Model") else {
                break;
            };
            let (t, r, s) = lcl(n);
            m = local_matrix(n, t, r, s) * m;
            cur = self
                .parents(c)
                .map(|(p, _)| p)
                .find(|p| self.objects.get(p).is_some_and(|n| n.name == "Model"));
            guard += 1;
            if guard > 256 {
                break;
            }
        }
        m
    }

    /// A model's world matrix from the file's bind pose (`Pose` of type BindPose), if any.
    fn bind_pose(&self, id: i64) -> Option<DMat4> {
        self.objects
            .values()
            .filter(|n| n.name == "Pose" && n.class() == "BindPose")
            .flat_map(|p| p.children_named("PoseNode"))
            .find(|pn| pn.value("Node").and_then(Prop::int) == Some(id))
            .map(|pn| pn.floats("Matrix"))
            .filter(|m| m.len() == 16)
            .map(|m| DMat4::from_cols_slice(&m))
    }

    /// A file-space matrix in Unity space.
    fn convert(&self, m: DMat4) -> DMat4 {
        self.conv * m * self.conv.inverse()
    }
}

/// Matrix from the file's axis system and unit to Y-up metres, then X negated (Unity).
fn conversion(global: Option<&Node>, convert_units: bool) -> DMat4 {
    let g = |n: &str, d: i64| global.and_then(|g| g.p_int(n)).unwrap_or(d);
    let (up, up_s) = (g("UpAxis", 1), g("UpAxisSign", 1));
    let (front, front_s) = (g("FrontAxis", 2), g("FrontAxisSign", 1));
    let (coord, coord_s) = (g("CoordAxis", 0), g("CoordAxisSign", 1));
    let unit = global
        .and_then(|g| g.p_f64("UnitScaleFactor"))
        .unwrap_or(1.0);
    let axis = |i: i64, s: i64| {
        let mut v = DVec3::ZERO;
        v[(i.clamp(0, 2)) as usize] = s.signum() as f64;
        v
    };
    // Rows: the file axes that become right-handed Y-up X, Y and Z (Maya's default is the
    // identity). A left-handed axis declaration gets its front axis negated so the result
    // stays a rotation; the final X flip then makes it Unity's left-handed space.
    let x = axis(coord, coord_s);
    let y = axis(up, up_s);
    let z = axis(front, front_s);
    let mut basis = DMat3::from_cols(x, y, z).transpose();
    if basis.determinant() < 0.0 {
        basis = DMat3::from_cols(x, y, -z).transpose();
    }
    let scale = if convert_units { unit / 100.0 } else { 1.0 };
    DMat4::from_scale(DVec3::new(-1.0, 1.0, 1.0))
        * DMat4::from_mat3(basis)
        * DMat4::from_scale(DVec3::splat(scale))
}

/// Layer element values for each polygon vertex (`k`), control point (`cp`) and polygon (`p`).
fn layer_index(
    layer: &Node,
    data_len: usize,
    index_key: &str,
    k: usize,
    cp: usize,
    p: usize,
) -> Option<usize> {
    let mapping = layer
        .value("MappingInformationType")
        .and_then(Prop::str)
        .unwrap_or("ByPolygonVertex");
    let reference = layer
        .value("ReferenceInformationType")
        .and_then(Prop::str)
        .unwrap_or("Direct");
    let i = match mapping {
        "ByPolygonVertex" => k,
        "ByControlPoint" | "ByVertice" | "ByVertex" => cp,
        "ByPolygon" => p,
        "AllSame" => 0,
        _ => k,
    };
    let i = if reference == "IndexToDirect" || reference == "Index" {
        let idx = layer
            .child(index_key)
            .and_then(|n| n.props.first())
            .and_then(Prop::ints)?;
        usize::try_from(*idx.get(i)?).ok()?
    } else {
        i
    };
    (i < data_len).then_some(i)
}

/// Imports an FBX file as a [`Model`] stored under `path`, with its meshes, materials,
/// textures and animation clips.
pub fn import_fbx(
    server: &mut AssetServer,
    path: &str,
    bytes: &[u8],
) -> Result<Handle<Model>, String> {
    import_fbx_with(server, path, bytes, true)
}

/// [`import_fbx`] with Unity's "Convert Units" (off: one file unit is one metre).
pub fn import_fbx_with(
    server: &mut AssetServer,
    path: &str,
    bytes: &[u8],
    convert_units: bool,
) -> Result<Handle<Model>, String> {
    let top = if bytes.starts_with(b"Kaydara FBX Binary") {
        parse_binary(bytes).map_err(|e| format!("{path}: {e}"))?
    } else {
        parse_ascii(&String::from_utf8_lossy(bytes)).map_err(|e| format!("{path}: {e}"))?
    };
    let find = |name: &str| top.iter().find(|n| n.name == name);
    let objects_node = find("Objects").ok_or_else(|| format!("{path}: no Objects section"))?;
    let mut fbx = Fbx {
        objects: objects_node.children.iter().map(|n| (n.id(), n)).collect(),
        links: Vec::new(),
        conv: conversion(find("GlobalSettings"), convert_units),
    };
    if let Some(c) = find("Connections") {
        for l in c.children_named("C") {
            let child = l.props.get(1).and_then(Prop::int).unwrap_or(0);
            let parent = l.props.get(2).and_then(Prop::int).unwrap_or(0);
            let prop = l.props.get(3).and_then(Prop::str).unwrap_or("").to_owned();
            fbx.links.push((child, parent, prop));
        }
    }
    let dir = Path::new(path)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();

    // Models (transform nodes), in file order.
    let model_ids: Vec<i64> = objects_node
        .children
        .iter()
        .filter(|n| n.name == "Model")
        .map(Node::id)
        .collect();
    let index_of: HashMap<i64, usize> = model_ids
        .iter()
        .enumerate()
        .map(|(i, &id)| (id, i))
        .collect();
    let parent_of = |id: i64| {
        fbx.parents(id)
            .map(|(p, _)| p)
            .find(|p| index_of.contains_key(p))
    };
    // Unique names among siblings, and animation paths.
    let mut names: Vec<String> = model_ids
        .iter()
        .map(|id| fbx.objects[id].object_name())
        .collect();
    {
        let mut seen: HashMap<(Option<i64>, String), u32> = HashMap::new();
        for (i, &id) in model_ids.iter().enumerate() {
            let k = seen.entry((parent_of(id), names[i].clone())).or_default();
            if *k > 0 {
                names[i] = format!("{} ({k})", names[i]);
            }
            *k += 1;
        }
    }
    let paths: Vec<String> = model_ids
        .iter()
        .map(|&id| {
            let mut parts = vec![names[index_of[&id]].clone()];
            let mut cur = parent_of(id);
            while let Some(p) = cur {
                parts.push(names[index_of[&p]].clone());
                cur = parent_of(p);
            }
            parts.reverse();
            parts.join("/")
        })
        .collect();

    let mut materials_cache: HashMap<i64, Handle<Material>> = HashMap::new();
    let mut textures_cache: HashMap<(i64, bool), Option<Handle<Texture2D>>> = HashMap::new();
    let mut nodes: Vec<ModelNode> = Vec::new();
    let mut skins: Vec<ModelSkin> = Vec::new();
    for (i, &id) in model_ids.iter().enumerate() {
        let m = fbx.objects[&id];
        let (t, r, s) = lcl(m);
        let (position, rotation, scale) = trs(fbx.convert(local_matrix(m, t, r, s)));
        let children = model_ids
            .iter()
            .filter(|&&c| parent_of(c) == Some(id))
            .map(|c| index_of[c])
            .collect();
        let mut node = ModelNode {
            name: names[i].clone(),
            position,
            rotation,
            scale,
            children,
            mesh: None,
            materials: Vec::new(),
            skin: None,
        };
        if let Some(geo) = fbx
            .child_of_kind(id, "Geometry")
            .filter(|g| g.class() == "Mesh" || g.child("Vertices").is_some())
        {
            // Geometric transform: applies to this node's geometry only.
            let g = DMat4::from_scale_rotation_translation(
                m.p_vec3("GeometricScaling").unwrap_or(DVec3::ONE),
                euler(m.p_vec3("GeometricRotation").unwrap_or(DVec3::ZERO), 0),
                m.p_vec3("GeometricTranslation").unwrap_or(DVec3::ZERO),
            );
            let mesh_bind = fbx.bind_pose(id).unwrap_or_else(|| fbx.global(id));
            let (mesh, bones) = build_mesh(&fbx, geo, g, mesh_bind, &node.name);
            // Materials: the ones connected to the model, in connection order (slot order).
            let mats: Vec<Handle<Material>> = fbx
                .children(id)
                .filter_map(|(c, _)| fbx.objects.get(&c).copied())
                .filter(|n| n.name == "Material")
                .map(|mat| {
                    let mid = mat.id();
                    if let Some(h) = materials_cache.get(&mid) {
                        return h.clone();
                    }
                    let h = build_material(server, &fbx, mat, path, &dir, &mut textures_cache);
                    materials_cache.insert(mid, h.clone());
                    h
                })
                .collect();
            let slots = mesh.submeshes.len().max(1);
            node.materials = (0..slots)
                .map(|k| {
                    mats.get(k)
                        .or(mats.first())
                        .cloned()
                        .unwrap_or_else(|| Handle::from_path(crate::server::DEFAULT_MATERIAL))
                })
                .collect();
            if let Some((joints, binds)) = bones {
                let mut mesh = mesh;
                mesh.bindposes = binds;
                node.skin = Some(skins.len());
                skins.push(ModelSkin {
                    joints: joints
                        .iter()
                        .filter_map(|j| index_of.get(j).copied())
                        .collect(),
                });
                node.mesh = Some(server.meshes.insert(format!("{path}#mesh{i}"), mesh));
            } else {
                node.mesh = Some(server.meshes.insert(format!("{path}#mesh{i}"), mesh));
            }
        }
        nodes.push(node);
    }

    // Animation stacks → clips, baked at 30 fps.
    let mut animations = Vec::new();
    for (si, stack) in objects_node
        .children
        .iter()
        .filter(|n| n.name == "AnimationStack")
        .enumerate()
    {
        if let Some(clip) = bake_stack(&fbx, stack, &model_ids, &paths) {
            animations.push(server.animations.insert(format!("{path}#anim{si}"), clip));
        }
    }

    let roots = model_ids
        .iter()
        .filter(|&&id| parent_of(id).is_none())
        .map(|id| index_of[id])
        .collect();
    let name = Path::new(path)
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
    Ok(server.models.insert(path.to_owned(), model))
}

type Bones = Option<(Vec<i64>, Vec<Matrix4x4>)>;

/// `mesh_bind`: the mesh node's world matrix at bind time (file space).
fn build_mesh(
    fbx: &Fbx,
    geo: &Node,
    geometric: DMat4,
    mesh_bind: DMat4,
    name: &str,
) -> (Mesh, Bones) {
    let cps = geo.floats("Vertices");
    let poly = geo.ints("PolygonVertexIndex");
    let to_unity = fbx.conv * geometric;
    let normal_m = DMat3::from_mat4(to_unity).inverse().transpose();
    let points: Vec<DVec3> = cps
        .as_chunks::<3>()
        .0
        .iter()
        .map(|c| DVec3::new(c[0], c[1], c[2]))
        .collect();

    // Skin weights per control point.
    let mut weights: Vec<Vec<(usize, f32)>> = vec![Vec::new(); points.len()];
    let mut joints: Vec<i64> = Vec::new();
    let mut binds: Vec<Matrix4x4> = Vec::new();
    if let Some(skin) = fbx
        .child_of_kind(geo.id(), "Deformer")
        .filter(|d| d.class() == "Skin")
    {
        for (cid, _) in fbx.children(skin.id()) {
            let Some(cluster) = fbx.objects.get(&cid).filter(|c| c.class() == "Cluster") else {
                continue;
            };
            let Some(bone) = fbx
                .children(cid)
                .filter_map(|(b, _)| fbx.objects.get(&b).map(|n| (b, *n)))
                .find(|(_, n)| n.name == "Model")
                .map(|(b, _)| b)
            else {
                continue;
            };
            let bi = joints.len();
            joints.push(bone);
            let mat = |k: &str| {
                let v = cluster.floats(k);
                if v.len() == 16 {
                    DMat4::from_cols_slice(&v)
                } else {
                    DMat4::IDENTITY
                }
            };
            // Unity's bind pose: bone world⁻¹ × mesh world, at bind time. `TransformLink` is
            // the bone's world matrix at bind; the cluster's `Transform` isn't used for the mesh
            // because exporters disagree on it (Blender writes bone⁻¹ × mesh there).
            let link = match cluster.floats("TransformLink").len() {
                16 => mat("TransformLink"),
                _ => fbx.bind_pose(bone).unwrap_or_else(|| fbx.global(bone)),
            };
            let bind = link.inverse() * mesh_bind;
            binds.push(to_f32(fbx.convert(bind)));
            let idx = cluster.ints("Indexes");
            let w = cluster.floats("Weights");
            for (i, wt) in idx.iter().zip(w) {
                if let Some(v) = weights.get_mut(*i as usize) {
                    v.push((bi, wt as f32));
                }
            }
        }
    }
    let skinned = !joints.is_empty();

    let normals_layer = geo.child("LayerElementNormal");
    let normals = normals_layer
        .map(|l| l.floats("Normals"))
        .unwrap_or_default();
    let uv_layers: Vec<&Node> = geo.children_named("LayerElementUV").collect();
    let uv_data: Vec<Vec<f64>> = uv_layers.iter().map(|l| l.floats("UV")).collect();
    let color_layer = geo.child("LayerElementColor");
    let colors = color_layer.map(|l| l.floats("Colors")).unwrap_or_default();
    let mat_layer = geo.child("LayerElementMaterial");
    let mat_ids = mat_layer.map(|l| l.ints("Materials")).unwrap_or_default();

    let mut mesh = Mesh {
        name: name.to_owned(),
        ..Default::default()
    };
    // Triangles per material slot; vertices deduplicated by all attributes.
    let mut tris: HashMap<usize, Vec<u32>> = HashMap::new();
    let mut dedupe: HashMap<Vec<u64>, u32> = HashMap::new();
    let mut polygon: Vec<(usize, usize)> = Vec::new(); // (k, cp)
    let mut p = 0usize;
    for (k, &raw) in poly.iter().enumerate() {
        let (cp, last) = if raw < 0 {
            ((!raw) as usize, true)
        } else {
            (raw as usize, false)
        };
        polygon.push((k, cp));
        if !last {
            continue;
        }
        let slot = mat_layer
            .and_then(|l| layer_index(l, mat_ids.len(), "", k, cp, p))
            .and_then(|i| mat_ids.get(i))
            .map_or(0, |&m| m.max(0) as usize);
        let mut idx = Vec::with_capacity(polygon.len());
        for &(k, cp) in &polygon {
            let pos = points.get(cp).copied().unwrap_or(DVec3::ZERO);
            let n = normals_layer
                .and_then(|l| layer_index(l, normals.len() / 3, "NormalsIndex", k, cp, p))
                .map(|i| DVec3::new(normals[i * 3], normals[i * 3 + 1], normals[i * 3 + 2]));
            let uvs: Vec<Option<[f64; 2]>> = uv_layers
                .iter()
                .zip(&uv_data)
                .map(|(l, d)| {
                    layer_index(l, d.len() / 2, "UVIndex", k, cp, p)
                        .map(|i| [d[i * 2], d[i * 2 + 1]])
                })
                .collect();
            let col = color_layer
                .and_then(|l| layer_index(l, colors.len() / 4, "ColorIndex", k, cp, p))
                .map(|i| {
                    [
                        colors[i * 4],
                        colors[i * 4 + 1],
                        colors[i * 4 + 2],
                        colors[i * 4 + 3],
                    ]
                });
            let mut key: Vec<u64> = vec![cp as u64];
            if let Some(n) = n {
                key.extend([n.x.to_bits(), n.y.to_bits(), n.z.to_bits()]);
            }
            for uv in uvs.iter().flatten() {
                key.extend([uv[0].to_bits(), uv[1].to_bits()]);
            }
            if let Some(c) = col {
                key.extend(c.map(f64::to_bits));
            }
            let i = *dedupe.entry(key).or_insert_with(|| {
                mesh.vertices.push(to_unity.transform_point3(pos).as_vec3());
                if let Some(n) = n {
                    mesh.normals
                        .push((normal_m * n).normalize_or_zero().as_vec3());
                }
                if let Some(Some(uv)) = uvs.first() {
                    mesh.uv.push(Vector2::new(uv[0] as f32, 1.0 - uv[1] as f32));
                } else if !uv_layers.is_empty() {
                    mesh.uv.push(Vector2::ZERO);
                }
                if let Some(Some(uv)) = uvs.get(1) {
                    mesh.uv2
                        .push(Vector2::new(uv[0] as f32, 1.0 - uv[1] as f32));
                }
                if let Some(c) = col {
                    mesh.colors.push(Vector4::new(
                        c[0] as f32,
                        c[1] as f32,
                        c[2] as f32,
                        c[3] as f32,
                    ));
                }
                if skinned {
                    let mut w = weights.get(cp).cloned().unwrap_or_default();
                    w.sort_by(|a, b| b.1.total_cmp(&a.1));
                    w.truncate(4);
                    let sum: f32 = w.iter().map(|x| x.1).sum();
                    let mut bi = [0u16; 4];
                    let mut bw = [0f32; 4];
                    for (j, (b, x)) in w.iter().enumerate() {
                        bi[j] = *b as u16;
                        bw[j] = if sum > 0.0 { x / sum } else { 0.0 };
                    }
                    if sum <= 0.0 {
                        bw[0] = 1.0;
                    }
                    mesh.bone_indices.push(bi);
                    mesh.bone_weights.push(Vector4::from_array(bw));
                }
                mesh.vertices.len() as u32 - 1
            });
            idx.push(i);
        }
        let out = tris.entry(slot).or_default();
        for j in 1..idx.len().saturating_sub(1) {
            out.extend([idx[0], idx[j], idx[j + 1]]);
        }
        polygon.clear();
        p += 1;
    }
    // Normals and UVs only if every vertex got one.
    if mesh.normals.len() != mesh.vertices.len() {
        mesh.normals.clear();
    }
    if mesh.uv2.len() != mesh.vertices.len() {
        mesh.uv2.clear();
    }
    let max_slot = tris.keys().copied().max().unwrap_or(0);
    for slot in 0..=max_slot {
        let start = mesh.indices.len() as u32;
        if let Some(t) = tris.get(&slot) {
            mesh.indices.extend_from_slice(t);
        }
        mesh.submeshes.push(start..mesh.indices.len() as u32);
    }
    mesh.finish();
    (mesh, skinned.then_some((joints, binds)))
}

fn build_material(
    server: &mut AssetServer,
    fbx: &Fbx,
    mat: &Node,
    path: &str,
    dir: &Path,
    cache: &mut HashMap<(i64, bool), Option<Handle<Texture2D>>>,
) -> Handle<Material> {
    let col = |n: &str| mat.p_vec3(n);
    let diffuse = col("DiffuseColor")
        .or(col("Diffuse"))
        .unwrap_or(DVec3::splat(0.8));
    let diffuse_f = mat.p_f64("DiffuseFactor").unwrap_or(1.0);
    let opacity = mat
        .p_f64("Opacity")
        .or(mat.p_f64("TransparencyFactor").map(|t| 1.0 - t))
        .unwrap_or(1.0)
        .clamp(0.0, 1.0);
    let shininess = mat
        .p_f64("ShininessExponent")
        .or(mat.p_f64("Shininess"))
        .unwrap_or(20.0);
    let d = diffuse * diffuse_f;
    let mut m = Material {
        name: mat.object_name(),
        base_color: Color::new(d.x as f32, d.y as f32, d.z as f32, opacity as f32),
        smoothness: ((shininess / 1000.0).sqrt() as f32).clamp(0.0, 1.0),
        metallic: 0.0,
        ..Default::default()
    };
    if opacity < 0.999 {
        m.surface_type = SurfaceType::Transparent;
    }
    if let Some(e) = col("EmissiveColor").filter(|e| e.max_element() > 0.0) {
        let f = mat.p_f64("EmissiveFactor").unwrap_or(1.0);
        m.emission_color = Color::new((e.x * f) as f32, (e.y * f) as f32, (e.z * f) as f32, 1.0);
    }
    // Textures plugged into the material's properties.
    for (tid, prop) in fbx.children(mat.id()) {
        let Some(tex) = fbx.objects.get(&tid).filter(|t| t.name == "Texture") else {
            continue;
        };
        let srgb = matches!(
            prop,
            "DiffuseColor" | "Diffuse" | "EmissiveColor" | "Maya|baseColor" | "base_color_map"
        );
        let h = cache
            .entry((tid, srgb))
            .or_insert_with(|| load_texture(server, fbx, tex, path, dir, srgb))
            .clone();
        let Some(h) = h else { continue };
        match prop {
            "DiffuseColor" | "Diffuse" | "Maya|baseColor" | "base_color_map" => {
                m.base_map = Some(h);
                if diffuse.max_element() < 0.01 {
                    m.base_color = Color::new(1.0, 1.0, 1.0, opacity as f32);
                }
            }
            "NormalMap" | "Bump" | "Maya|normalCamera" | "normal_map" => m.normal_map = Some(h),
            "EmissiveColor" | "Emissive" => {
                m.emission_map = Some(h);
                if col("EmissiveColor").is_none_or(|e| e.max_element() <= 0.0) {
                    m.emission_color = Color::WHITE;
                }
            }
            "TransparentColor" | "TransparencyFactor" => {
                m.surface_type = SurfaceType::Cutout;
                if m.base_map.is_none() {
                    m.base_map = Some(h);
                }
            }
            _ => {}
        }
    }
    let id = mat.id();
    server.materials.insert(format!("{path}#mat{id}"), m)
}

/// A texture node: an embedded image (its Video's `Content`) or a file found next to the
/// model, in its `Textures` folder, or at the stored relative path (Unity's search).
fn load_texture(
    server: &mut AssetServer,
    fbx: &Fbx,
    tex: &Node,
    path: &str,
    dir: &Path,
    srgb: bool,
) -> Option<Handle<Texture2D>> {
    let file = tex
        .value("RelativeFilename")
        .or(tex.value("FileName"))
        .and_then(Prop::str)
        .unwrap_or("")
        .replace('\\', "/");
    let name = file.rsplit('/').next().unwrap_or(&file).to_owned();
    // Embedded media.
    let video = fbx
        .children(tex.id())
        .filter_map(|(v, _)| fbx.objects.get(&v))
        .find(|v| v.name == "Video");
    if let Some(Prop::Raw(bytes)) = video.and_then(|v| v.value("Content"))
        && !bytes.is_empty()
    {
        match Texture2D::decode(&name, bytes, srgb) {
            Ok(t) => {
                let suffix = if srgb { "" } else { "_linear" };
                return Some(
                    server
                        .textures
                        .insert(format!("{path}#tex{}{suffix}", tex.id()), t),
                );
            }
            Err(e) => oxide_core::debug_log_warning!("{path}: embedded {name}: {e}"),
        }
    }
    if name.is_empty() {
        return None;
    }
    let rel = |p: std::path::PathBuf| p.to_string_lossy().replace('\\', "/");
    let candidates = [
        rel(dir.join(&file)),
        rel(dir.join(&name)),
        rel(dir.join("Textures").join(&name)),
        rel(dir.join("textures").join(&name)),
        rel(dir.join("..").join("Textures").join(&name)),
    ];
    let found = candidates.iter().find(|c| server.full_path(c).is_file())?;
    match server.load_texture(found) {
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
}

/// A curve's value at `t` (ticks), linear between keys, held outside them.
fn sample(times: &[i64], values: &[f64], t: i64) -> Option<f64> {
    if times.is_empty() || values.is_empty() {
        return None;
    }
    let i = times.partition_point(|&k| k <= t);
    if i == 0 {
        return values.first().copied();
    }
    if i >= times.len() {
        return values.get(times.len() - 1).or(values.last()).copied();
    }
    let (t0, t1) = (times[i - 1], times[i]);
    let (v0, v1) = (values[i - 1], *values.get(i)?);
    let f = if t1 > t0 {
        (t - t0) as f64 / (t1 - t0) as f64
    } else {
        0.0
    };
    Some(v0 + (v1 - v0) * f)
}

fn bake_stack(
    fbx: &Fbx,
    stack: &Node,
    model_ids: &[i64],
    paths: &[String],
) -> Option<AnimationClip> {
    // Model id → property ("Lcl Translation", ...) → [x, y, z] curves (times, values) and
    // the curve node's defaults.
    type Curves = [Option<(Vec<i64>, Vec<f64>)>; 3];
    let mut by_model: HashMap<i64, HashMap<String, (Curves, DVec3)>> = HashMap::new();
    let (mut t_min, mut t_max) = (i64::MAX, i64::MIN);
    for (layer, _) in fbx.children(stack.id()) {
        if fbx
            .objects
            .get(&layer)
            .is_none_or(|n| n.name != "AnimationLayer")
        {
            continue;
        }
        for (cn, _) in fbx.children(layer) {
            let Some(curve_node) = fbx
                .objects
                .get(&cn)
                .filter(|n| n.name == "AnimationCurveNode")
            else {
                continue;
            };
            let Some((model, prop)) = fbx
                .parents(cn)
                .find(|(p, prop)| model_ids.contains(p) && prop.starts_with("Lcl "))
            else {
                continue;
            };
            let defaults = DVec3::new(
                curve_node.p_f64("d|X").unwrap_or(0.0),
                curve_node.p_f64("d|Y").unwrap_or(0.0),
                curve_node.p_f64("d|Z").unwrap_or(0.0),
            );
            let mut curves: Curves = [None, None, None];
            for (c, axis) in fbx.children(cn) {
                let Some(curve) = fbx.objects.get(&c).filter(|n| n.name == "AnimationCurve") else {
                    continue;
                };
                let k = match axis {
                    "d|X" => 0,
                    "d|Y" => 1,
                    "d|Z" => 2,
                    _ => continue,
                };
                let times = curve.ints("KeyTime");
                let values = curve.floats("KeyValueFloat");
                if let (Some(a), Some(b)) = (times.first(), times.last()) {
                    t_min = t_min.min(*a);
                    t_max = t_max.max(*b);
                }
                curves[k] = Some((times, values));
            }
            by_model
                .entry(model)
                .or_default()
                .insert(prop.to_owned(), (curves, defaults));
        }
    }
    if by_model.is_empty() || t_min > t_max {
        return None;
    }
    // The stack's own range when it has one.
    if let (Some(a), Some(b)) = (stack.p_int("LocalStart"), stack.p_int("LocalStop"))
        && b > a
    {
        t_min = a;
        t_max = b;
    }
    let fps = 30.0;
    let frames = (((t_max - t_min) as f64 / TICKS) * fps).ceil().max(1.0) as usize;
    let mut curves_out = Vec::new();
    for (&model, props) in &by_model {
        let Some(i) = model_ids.iter().position(|&m| m == model) else {
            continue;
        };
        let node = fbx.objects[&model];
        let (t0, r0, s0) = lcl(node);
        let channel = |name: &str, base: DVec3, tick: i64| -> DVec3 {
            let Some((c, defaults)) = props.get(name) else {
                return base;
            };
            let mut v = *defaults;
            for (k, c) in c.iter().enumerate() {
                if let Some((times, values)) = c
                    && let Some(x) = sample(times, values, tick)
                {
                    v[k] = x;
                }
            }
            v
        };
        let mut times = Vec::with_capacity(frames + 1);
        let (mut ps, mut rs, mut ss) = (Vec::new(), Vec::new(), Vec::new());
        for f in 0..=frames {
            let tick = t_min + ((f as f64 / fps) * TICKS) as i64;
            let tick = tick.min(t_max);
            let t = channel("Lcl Translation", t0, tick);
            let r = channel("Lcl Rotation", r0, tick);
            let s = channel("Lcl Scaling", s0, tick);
            let (p, q, sc) = trs(fbx.convert(local_matrix(node, t, r, s)));
            times.push(f as f32 / fps as f32);
            ps.push(p);
            // Keep quaternions on one hemisphere so interpolation takes the short way.
            let q = match rs.last() {
                Some(prev) if q.dot(*prev) < 0.0 => -q,
                _ => q,
            };
            rs.push(q);
            ss.push(sc);
        }
        curves_out.push(TransformCurves {
            path: paths[i].clone(),
            position: props.contains_key("Lcl Translation").then(|| Track {
                times: times.clone(),
                values: ps,
                interpolation: Interpolation::Linear,
            }),
            rotation: props.contains_key("Lcl Rotation").then(|| Track {
                times: times.clone(),
                values: rs,
                interpolation: Interpolation::Linear,
            }),
            scale: props.contains_key("Lcl Scaling").then_some(Track {
                times,
                values: ss,
                interpolation: Interpolation::Linear,
            }),
        });
    }
    curves_out.sort_by(|a, b| a.path.cmp(&b.path));
    let name = stack.object_name();
    let mut clip = AnimationClip {
        name: name.rsplit('|').next().unwrap_or(&name).to_owned(),
        looping: true,
        curves: curves_out,
        ..Default::default()
    };
    clip.recalculate_length();
    Some(clip)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes a binary FBX 7.4 node (32-bit offsets).
    fn node(out: &mut Vec<u8>, name: &str, props: &[Vec<u8>], children: &dyn Fn(&mut Vec<u8>)) {
        let start = out.len();
        out.extend([0u8; 12]);
        out.push(name.len() as u8);
        out.extend(name.as_bytes());
        let pstart = out.len();
        for p in props {
            out.extend(p);
        }
        let plen = out.len() - pstart;
        let before = out.len();
        children(out);
        if out.len() != before {
            out.extend([0u8; 13]);
        }
        let end = out.len() as u32;
        out[start..start + 4].copy_from_slice(&end.to_le_bytes());
        out[start + 4..start + 8].copy_from_slice(&(props.len() as u32).to_le_bytes());
        out[start + 8..start + 12].copy_from_slice(&(plen as u32).to_le_bytes());
    }
    fn s(v: &str) -> Vec<u8> {
        let mut o = vec![b'S'];
        o.extend((v.len() as u32).to_le_bytes());
        o.extend(v.as_bytes());
        o
    }
    fn l(v: i64) -> Vec<u8> {
        let mut o = vec![b'L'];
        o.extend(v.to_le_bytes());
        o
    }
    fn d(v: f64) -> Vec<u8> {
        let mut o = vec![b'D'];
        o.extend(v.to_le_bytes());
        o
    }
    fn i(v: i32) -> Vec<u8> {
        let mut o = vec![b'I'];
        o.extend(v.to_le_bytes());
        o
    }
    fn da(v: &[f64], compress: bool) -> Vec<u8> {
        let raw: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
        array(b'd', v.len(), raw, compress)
    }
    fn ia(v: &[i32]) -> Vec<u8> {
        let raw: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
        array(b'i', v.len(), raw, false)
    }
    fn array(t: u8, n: usize, raw: Vec<u8>, compress: bool) -> Vec<u8> {
        let data = if compress {
            use std::io::Write;
            let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            e.write_all(&raw).unwrap();
            e.finish().unwrap()
        } else {
            raw
        };
        let mut o = vec![t];
        o.extend((n as u32).to_le_bytes());
        o.extend((compress as u32).to_le_bytes());
        o.extend((data.len() as u32).to_le_bytes());
        o.extend(data);
        o
    }
    fn p70(out: &mut Vec<u8>, entries: &[(&str, &str, Vec<Vec<u8>>)]) {
        node(out, "Properties70", &[], &|out| {
            for (n, t, vals) in entries {
                let mut props = vec![s(n), s(t), s(""), s("A")];
                props.extend(vals.iter().cloned());
                node(out, "P", &props, &|_| {});
            }
        });
    }

    /// A Z-up, centimetre cube-ish quad with a parent node, a material and a 1 s animation.
    fn sample_file() -> Vec<u8> {
        let mut f = b"Kaydara FBX Binary  \x00\x1a\x00".to_vec();
        f.extend(7400u32.to_le_bytes());
        node(&mut f, "GlobalSettings", &[], &|o| {
            p70(
                o,
                &[
                    ("UpAxis", "int", vec![i(2)]),
                    ("UpAxisSign", "int", vec![i(1)]),
                    ("FrontAxis", "int", vec![i(1)]),
                    ("FrontAxisSign", "int", vec![i(-1)]),
                    ("CoordAxis", "int", vec![i(0)]),
                    ("CoordAxisSign", "int", vec![i(1)]),
                    ("UnitScaleFactor", "double", vec![d(1.0)]),
                ],
            )
        });
        node(&mut f, "Objects", &[], &|o| {
            node(
                o,
                "Geometry",
                &[l(10), s("Quad\x00\x01Geometry"), s("Mesh")],
                &|o| {
                    node(
                        o,
                        "Vertices",
                        &[da(
                            &[0., 0., 0., 100., 0., 0., 100., 0., 100., 0., 0., 100.],
                            true,
                        )],
                        &|_| {},
                    );
                    node(o, "PolygonVertexIndex", &[ia(&[0, 1, 2, -4])], &|_| {});
                    node(o, "LayerElementNormal", &[i(0)], &|o| {
                        node(o, "MappingInformationType", &[s("ByControlPoint")], &|_| {});
                        node(o, "ReferenceInformationType", &[s("Direct")], &|_| {});
                        node(
                            o,
                            "Normals",
                            &[da(
                                &[0., -1., 0., 0., -1., 0., 0., -1., 0., 0., -1., 0.],
                                false,
                            )],
                            &|_| {},
                        );
                    });
                },
            );
            node(
                o,
                "Model",
                &[l(20), s("Root\x00\x01Model"), s("Null")],
                &|o| {
                    p70(
                        o,
                        &[(
                            "Lcl Translation",
                            "Lcl Translation",
                            vec![d(0.), d(0.), d(200.)],
                        )],
                    );
                },
            );
            node(
                o,
                "Model",
                &[l(21), s("Quad\x00\x01Model"), s("Mesh")],
                &|o| {
                    p70(
                        o,
                        &[(
                            "Lcl Translation",
                            "Lcl Translation",
                            vec![d(100.), d(0.), d(0.)],
                        )],
                    );
                },
            );
            node(
                o,
                "Material",
                &[l(30), s("Red\x00\x01Material"), s("")],
                &|o| {
                    p70(o, &[("DiffuseColor", "Color", vec![d(1.), d(0.), d(0.)])]);
                },
            );
            node(
                o,
                "AnimationStack",
                &[l(40), s("Take 001\x00\x01AnimStack"), s("")],
                &|_| {},
            );
            node(
                o,
                "AnimationLayer",
                &[l(41), s("Base\x00\x01AnimLayer"), s("")],
                &|_| {},
            );
            node(
                o,
                "AnimationCurveNode",
                &[l(42), s("T\x00\x01AnimCurveNode"), s("")],
                &|o| {
                    p70(
                        o,
                        &[
                            ("d|X", "Number", vec![d(100.)]),
                            ("d|Y", "Number", vec![d(0.)]),
                            ("d|Z", "Number", vec![d(0.)]),
                        ],
                    );
                },
            );
            node(
                o,
                "AnimationCurve",
                &[l(43), s("\x00\x01AnimCurve"), s("")],
                &|o| {
                    let mut t = vec![b'l'];
                    let times: Vec<u8> = [0i64, TICKS as i64]
                        .iter()
                        .flat_map(|x| x.to_le_bytes())
                        .collect();
                    t.extend(2u32.to_le_bytes());
                    t.extend(0u32.to_le_bytes());
                    t.extend((times.len() as u32).to_le_bytes());
                    t.extend(times);
                    node(o, "KeyTime", &[t], &|_| {});
                    let mut v = vec![b'f'];
                    let vals: Vec<u8> = [100f32, 300.0]
                        .iter()
                        .flat_map(|x| x.to_le_bytes())
                        .collect();
                    v.extend(2u32.to_le_bytes());
                    v.extend(0u32.to_le_bytes());
                    v.extend((vals.len() as u32).to_le_bytes());
                    v.extend(vals);
                    node(o, "KeyValueFloat", &[v], &|_| {});
                },
            );
        });
        node(&mut f, "Connections", &[], &|o| {
            let c = |o: &mut Vec<u8>, a: i64, b: i64| node(o, "C", &[s("OO"), l(a), l(b)], &|_| {});
            let cp = |o: &mut Vec<u8>, a: i64, b: i64, p: &str| {
                node(o, "C", &[s("OP"), l(a), l(b), s(p)], &|_| {})
            };
            c(o, 20, 0);
            c(o, 21, 20);
            c(o, 10, 21);
            c(o, 30, 21);
            c(o, 41, 40);
            c(o, 42, 41);
            cp(o, 42, 21, "Lcl Translation");
            cp(o, 43, 42, "d|X");
        });
        f.extend([0u8; 13]);
        f.extend([0u8; 160]); // footer
        f
    }

    #[test]
    fn binary_fbx_imports_hierarchy_mesh_material_and_animation() {
        let dir = std::env::temp_dir().join(format!("oxide_fbx_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = AssetServer::new(&dir);
        let h = import_fbx(&mut s, "Assets/quad.fbx", &sample_file()).unwrap();
        let m = s.models.get(&h).unwrap().clone();
        assert_eq!(m.roots, vec![0]);
        assert_eq!(m.nodes[0].name, "Root");
        assert_eq!(m.nodes[0].children, vec![1]);
        // Z-up centimetres: (0, 0, 200) cm is 2 m up in Unity.
        let near = |a: Vector3, b: Vector3| (a - b).length() < 1e-4;
        assert!(
            near(m.nodes[0].position, Vector3::new(0.0, 2.0, 0.0)),
            "{:?}",
            m.nodes[0].position
        );
        // X is negated (Unity's FBX convention).
        assert!(
            near(m.nodes[1].position, Vector3::new(-1.0, 0.0, 0.0)),
            "{:?}",
            m.nodes[1].position
        );
        let mesh = s.meshes.get(m.nodes[1].mesh.as_ref().unwrap()).unwrap();
        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.indices.len(), 6);
        // The quad lies on the file's XZ plane (Z up): in Unity it stands on XY... no: file
        // Y (depth) → Unity -Z, file Z (up) → Unity Y. The vertex (100, 0, 100) cm →
        // (-1, 1, 0) m.
        assert!(
            mesh.vertices
                .iter()
                .any(|v| near(*v, Vector3::new(-1.0, 1.0, 0.0))),
            "{:?}",
            mesh.vertices
        );
        // File normal -Y (toward the viewer in Z-up) → Unity +Z.
        assert!(
            near(mesh.normals[0], Vector3::new(0.0, 0.0, 1.0)),
            "{:?}",
            mesh.normals[0]
        );
        let mat = s.materials.get(&m.nodes[1].materials[0]).unwrap();
        assert_eq!(mat.name, "Red");
        assert_eq!(mat.base_color, Color::new(1.0, 0.0, 0.0, 1.0));
        // Animation: X 100 → 300 cm over 1 s; Unity X -1 → -3 m.
        let clip = s.animations.get(&m.animations[0]).unwrap();
        assert_eq!(clip.name, "Take 001");
        assert!((clip.length - 1.0).abs() < 1e-3, "{}", clip.length);
        let c = &clip.curves[0];
        assert_eq!(c.path, "Root/Quad");
        let pos = c.position.as_ref().unwrap();
        assert!(near(pos.values[0], Vector3::new(-1.0, 0.0, 0.0)));
        assert!(
            near(*pos.values.last().unwrap(), Vector3::new(-3.0, 0.0, 0.0)),
            "{:?}",
            pos.values.last()
        );
    }

    #[test]
    fn ascii_fbx_parses() {
        let text = r#"; FBX 7.4.0 project file
Objects:  {
    Geometry: 10, "Geometry::Tri", "Mesh" {
        Vertices: *9 {
            a: 0,0,0,100,0,0,0,100,0
        }
        PolygonVertexIndex: *3 {
            a: 0,1,-3
        }
    }
    Model: 20, "Model::Tri", "Mesh" {
        Properties70:  {
            P: "Lcl Translation", "Lcl Translation", "", "A",0,0,0
        }
    }
}
Connections:  {
    C: "OO",20,0
    C: "OO",10,20
}
"#;
        let dir = std::env::temp_dir().join(format!("oxide_fbx_ascii_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = AssetServer::new(&dir);
        let h = import_fbx(&mut s, "Assets/tri.fbx", text.as_bytes()).unwrap();
        let m = s.models.get(&h).unwrap();
        assert_eq!(m.nodes[0].name, "Tri");
        let mesh = s.meshes.get(m.nodes[0].mesh.as_ref().unwrap()).unwrap();
        assert_eq!(mesh.indices.len(), 3);
        assert!(
            (mesh.vertices[1].x + 1.0).abs() < 1e-5,
            "{:?}",
            mesh.vertices
        );
    }
}

#[cfg(test)]
mod real_files {
    /// `OXIDE_FBX_DIR=folder cargo test -p oxide_assets real_fbx -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_fbx() {
        let Ok(dir) = std::env::var("OXIDE_FBX_DIR") else {
            return;
        };
        let dir = std::path::PathBuf::from(dir);
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.to_lowercase().ends_with(".fbx") {
                continue;
            }
            let mut s = crate::server::AssetServer::new(&dir);
            let bytes = std::fs::read(e.path()).unwrap();
            match super::import_fbx(&mut s, &name, &bytes) {
                Ok(h) => {
                    let m = s.models.get(&h).unwrap();
                    let meshes: Vec<String> = m
                        .nodes
                        .iter()
                        .filter_map(|n| n.mesh.as_ref())
                        .filter_map(|h| s.meshes.get(h))
                        .map(|me| {
                            format!(
                                "{}v/{}t/{}sub/{}bones b={:?}",
                                me.vertices.len(),
                                me.indices.len() / 3,
                                me.submeshes.len(),
                                me.bindposes.len(),
                                me.bounds.size()
                            )
                        })
                        .collect();
                    println!(
                        "{name}: {} nodes, roots {:?}, skins {}, anims {:?}, meshes {meshes:?}, mats {:?}",
                        m.nodes.len(),
                        m.roots.len(),
                        m.skins.len(),
                        m.animations
                            .iter()
                            .filter_map(|a| s.animations.get(a))
                            .map(|c| (c.name.clone(), c.length, c.curves.len()))
                            .collect::<Vec<_>>(),
                        m.nodes
                            .iter()
                            .flat_map(|n| &n.materials)
                            .filter_map(|h| s.materials.get(h))
                            .map(|x| (x.name.clone(), x.base_map.is_some()))
                            .collect::<Vec<_>>(),
                    );
                }
                Err(e) => println!("{name}: ERROR {e}"),
            }
        }
    }
}

#[cfg(test)]
mod skin_check {
    use oxide_math::{Matrix4x4, Vector3};
    #[test]
    #[ignore]
    fn huesitos_bind_pose() {
        let Ok(dir) = std::env::var("OXIDE_FBX_DIR") else {
            return;
        };
        let dir = std::path::PathBuf::from(dir);
        let mut s = crate::server::AssetServer::new(&dir);
        let bytes = std::fs::read(dir.join("huesitos.fbx")).unwrap();
        let h = super::import_fbx(&mut s, "huesitos.fbx", &bytes).unwrap();
        let m = s.models.get(&h).unwrap().clone();
        let n = m.nodes.len();
        let mut parent = vec![None; n];
        for (i, nd) in m.nodes.iter().enumerate() {
            for &c in &nd.children {
                parent[c] = Some(i);
            }
        }
        let world = |mut i: usize| {
            let mut w = Matrix4x4::IDENTITY;
            loop {
                let nd = &m.nodes[i];
                w = Matrix4x4::from_scale_rotation_translation(nd.scale, nd.rotation, nd.position)
                    * w;
                match parent[i] {
                    Some(p) => i = p,
                    None => return w,
                }
            }
        };
        let (mi, node) = m
            .nodes
            .iter()
            .enumerate()
            .find(|(_, nd)| nd.skin.is_some())
            .unwrap();
        let mesh = s.meshes.get(node.mesh.as_ref().unwrap()).unwrap();
        let skin = &m.skins[node.skin.unwrap()];
        let wm = world(mi);
        let mut worst = 0.0f32;
        for (k, v) in mesh.vertices.iter().enumerate() {
            let expect = wm.transform_point3(*v);
            let mut got = Vector3::ZERO;
            for j in 0..4 {
                let w = mesh.bone_weights[k][j];
                if w == 0.0 {
                    continue;
                }
                let b = mesh.bone_indices[k][j] as usize;
                got += (world(skin.joints[b]) * mesh.bindposes[b]).transform_point3(*v) * w;
            }
            worst = worst.max((got - expect).length());
            if k < 3 {
                println!("v{k}: expect {expect:?} got {got:?}");
            }
        }
        println!("worst {worst}");
    }
}
