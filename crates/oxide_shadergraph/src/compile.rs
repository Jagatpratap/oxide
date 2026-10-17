//! Compiles a Shader Graph to the Lit shader's surface hooks (`vertex_mod` and `surface`).
//!
//! Each stage walks back from the master stack's blocks: every node it reaches becomes one
//! `let` per output, in dependency order, so shared nodes are computed once. Sub Graphs are
//! inlined with their inputs bound to the caller's values. Properties become slots of the
//! material's property uniform (`graph_props`) and texture bindings.

use crate::graph::*;
use crate::nodes::{self, Gen, HELPERS, Slot, Ty, constant, convert};
use oxide_assets::{Handle, ShaderProperty, Texture2D};
use std::collections::{HashMap, HashSet};

/// Uniform slots for property values (the last one is the preview index).
pub const VALUE_SLOTS: u32 = 31;
/// The slot node previews read their node index from.
pub const PREVIEW_SLOT: u32 = 31;
pub const PREVIEW_PROPERTY: &str = "__sg_preview";
/// Texture slots: the Lit material's five map bindings.
pub const TEXTURE_BINDINGS: [&str; 5] = [
    "base_map",
    "metal_rough_map",
    "normal_map",
    "occlusion_map",
    "emission_map",
];

/// Looks up a Sub Graph asset by path.
pub type SubGraphs<'a> = &'a dyn Fn(&str) -> Option<ShaderSubGraph>;

/// What ports a node has in its graph (most come from the library; Property, Sub Graph,
/// Custom Function, Swizzle and the outputs depend on the graph).
pub struct PortCx<'a> {
    pub properties: &'a [GraphProperty],
    pub keywords: &'a [GraphKeyword],
    pub sub_outputs: &'a [SubGraphOutput],
    pub subgraphs: SubGraphs<'a>,
}

pub fn property_ty(kind: PropertyType) -> Ty {
    match kind {
        PropertyType::Float => Ty::F1,
        PropertyType::Vector2 => Ty::F2,
        PropertyType::Vector3 => Ty::F3,
        PropertyType::Vector4 | PropertyType::Color => Ty::F4,
        PropertyType::Boolean => Ty::Bool,
        PropertyType::Texture2D => Ty::Tex,
    }
}

/// Parses a Custom Function port list (`A:Float; B:Vector 3`).
pub fn parse_ports(spec: &str) -> Vec<Slot> {
    spec.split([';', ','])
        .filter_map(|p| {
            let (name, ty) = p.split_once(':')?;
            let name = name.trim();
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return None;
            }
            let ty = match ty.trim().replace(' ', "").to_ascii_lowercase().as_str() {
                "float" | "vector1" => Ty::F1,
                "vector2" => Ty::F2,
                "vector3" => Ty::F3,
                "vector4" => Ty::F4,
                "boolean" | "bool" => Ty::Bool,
                "texture2d" => Ty::Tex,
                _ => return None,
            };
            Some(Slot::new(name, ty))
        })
        .collect()
}

/// The option values of a node, defaults filled in.
pub fn options(node: &GraphNode) -> Vec<(&'static str, String)> {
    nodes::find(&node.kind).map_or_else(Vec::new, |d| {
        d.options
            .iter()
            .map(|o| (o.name, node.option(o.name).unwrap_or(o.default).to_owned()))
            .collect()
    })
}

/// A node's input and output ports.
pub fn ports(node: &GraphNode, cx: &PortCx) -> Result<(Vec<Slot>, Vec<Slot>), String> {
    let def = nodes::find(&node.kind).ok_or_else(|| format!("unknown node `{}`", node.kind))?;
    Ok(match node.kind.as_str() {
        "Property" => {
            let r = node.option("Property").unwrap_or("");
            let p = cx
                .properties
                .iter()
                .find(|p| p.reference == r)
                .ok_or_else(|| format!("property `{r}` not found"))?;
            (
                vec![],
                vec![Slot::new("Out", property_ty(p.kind)).labelled(&p.name)],
            )
        }
        "SubGraphOutput" => (
            cx.sub_outputs
                .iter()
                .map(|o| Slot::new(&o.name, property_ty(o.kind)))
                .collect(),
            vec![],
        ),
        "Sub Graph" => {
            let path = node.option("Sub Graph").unwrap_or("");
            let sub =
                (cx.subgraphs)(path).ok_or_else(|| format!("Sub Graph `{path}` not found"))?;
            (
                sub.properties
                    .iter()
                    .map(|p| {
                        let mut s = Slot::new(&p.reference, property_ty(p.kind))
                            .labelled(&p.name)
                            .val(p.default.to_array());
                        s.color = p.kind == PropertyType::Color;
                        s
                    })
                    .collect(),
                sub.outputs
                    .iter()
                    .map(|o| Slot::new(&o.name, property_ty(o.kind)))
                    .collect(),
            )
        }
        "Custom Function" => {
            let opts = options(node);
            let get = |n: &str| {
                opts.iter()
                    .find(|(k, _)| *k == n)
                    .map_or(String::new(), |(_, v)| v.clone())
            };
            (parse_ports(&get("Inputs")), parse_ports(&get("Outputs")))
        }
        "Swizzle" => {
            let mask = node.option("Mask").unwrap_or("xyzw");
            let n = mask.len().clamp(1, 4) as u32;
            (def.inputs.clone(), vec![Slot::new("Out", Ty::vector(n))])
        }
        _ => (def.inputs.clone(), def.outputs.clone()),
    })
}

/// Where compiled code goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// The material shader.
    Material,
    /// Node previews: every node's first output, picked by the preview slot.
    Preview,
}

/// A compiled graph.
#[derive(Clone, Debug, Default)]
pub struct Compiled {
    /// `vertex_mod` and `surface` for lit.wgsl (and shadow.wgsl).
    pub hooks: String,
    /// The shadow pass needs the graph's hooks (vertex changes or alpha clipping).
    pub shadow: bool,
    pub properties: Vec<ShaderProperty>,
    /// Errors by top-level node id (0: the graph itself).
    pub errors: Vec<(u32, String)>,
    /// Sub Graphs used (recompile when they change).
    pub dependencies: Vec<String>,
    /// Preview target: the node shown by each preview index.
    pub preview_nodes: Vec<u32>,
    pub unlit: bool,
    pub transparent: bool,
    pub alpha_clip: bool,
    pub double_sided: bool,
    pub cast_shadows: bool,
}

#[derive(Default)]
struct Alloc {
    props: Vec<ShaderProperty>,
    next_value: u32,
    next_texture: u32,
}

impl Alloc {
    fn value(&mut self, p: ShaderProperty) -> Option<u32> {
        if let Some(e) = self.props.iter().find(|e| e.name == p.name) {
            return Some(e.slot);
        }
        if self.next_value >= VALUE_SLOTS {
            return None;
        }
        let slot = self.next_value;
        self.next_value += 1;
        self.props.push(ShaderProperty { slot, ..p });
        Some(slot)
    }

    fn texture(
        &mut self,
        name: &str,
        display: &str,
        default: Option<Handle<Texture2D>>,
    ) -> Option<u32> {
        if let Some(e) = self.props.iter().find(|e| e.name == name) {
            return Some(e.slot);
        }
        if self.next_texture as usize >= TEXTURE_BINDINGS.len() {
            return None;
        }
        let slot = self.next_texture;
        self.next_texture += 1;
        self.props.push(ShaderProperty {
            name: name.into(),
            display_name: display.into(),
            kind: "Texture2D".into(),
            slot,
            default: [1.0; 4],
            default_texture: default,
            range: None,
        });
        Some(slot)
    }
}

/// An inline value as a runtime value: WGSL folds constant expressions when it compiles
/// and rejects ones that overflow (`1.0 / 0.0` from a Reciprocal with nothing connected),
/// which Unity graphs may produce.
fn value(ty: Ty, v: [f32; 4]) -> String {
    match ty {
        Ty::F1 | Ty::Dyn => format!("sg_k1({})", constant(Ty::F1, v)),
        Ty::F2 => format!("sg_k2({})", constant(ty, v)),
        Ty::F3 => format!("sg_k3({})", constant(ty, v)),
        Ty::F4 => format!("sg_k4({})", constant(ty, v)),
        t => constant(t, v),
    }
}

/// Linear value of an sRGB colour (inline colours; Color properties are converted when the
/// material uploads).
fn linear(v: [f32; 4]) -> [f32; 4] {
    let c = oxide_math::Color::new(v[0], v[1], v[2], v[3]).linear();
    [c.r, c.g, c.b, v[3]]
}

struct Scope<'s> {
    data: &'s GraphData,
    properties: &'s [GraphProperty],
    keywords: &'s [GraphKeyword],
    sub_outputs: &'s [SubGraphOutput],
    prefix: String,
    /// A Sub Graph's inputs connected in the calling graph, by property reference.
    bound: HashMap<String, (String, Ty)>,
    /// The top-level node errors inside this Sub Graph are shown on (None at top level).
    owner: Option<u32>,
    /// Sub Graphs being expanded (recursion guard).
    stack: Vec<String>,
}

struct Emit<'a> {
    fragment: bool,
    lines: Vec<String>,
    memo: HashMap<(String, u32), Vec<(String, Ty)>>,
    visiting: HashSet<(String, u32)>,
    errors: &'a mut Vec<(u32, String)>,
    alloc: &'a mut Alloc,
    functions: &'a mut Vec<(String, String)>,
    subgraphs: SubGraphs<'a>,
    deps: &'a mut Vec<String>,
}

impl Emit<'_> {
    fn error(&mut self, sc: &Scope, node: u32, msg: String) {
        let at = sc.owner.unwrap_or(node);
        if !self.errors.iter().any(|(n, m)| *n == at && *m == msg) {
            self.errors.push((at, msg));
        }
    }

    fn cx<'s>(&self, sc: &'s Scope) -> PortCx<'s>
    where
        Self: 's,
    {
        PortCx {
            properties: sc.properties,
            keywords: sc.keywords,
            sub_outputs: sc.sub_outputs,
            subgraphs: self.subgraphs,
        }
    }

    fn zero(ty: Ty) -> String {
        match ty {
            Ty::Tex => TEXTURE_BINDINGS[0].into(),
            t => value(t, [0.0; 4]),
        }
    }

    /// A texture binding for a texture with no connection (white, or the node's own
    /// texture).
    fn hidden_texture(&mut self, sc: &Scope, node: u32, key: &str, path: &str) -> String {
        let default = (!path.is_empty()).then(|| Handle::from_path(path.to_owned()));
        let name = format!("__tex_{}{key}", sc.prefix);
        match self.alloc.texture(&name, key, default) {
            Some(s) => TEXTURE_BINDINGS[s as usize].into(),
            None => {
                self.error(sc, node, "more than 5 textures".into());
                TEXTURE_BINDINGS[0].into()
            }
        }
    }

    /// The value an input port reads.
    fn input(&mut self, sc: &Scope, node: &GraphNode, slot: &Slot) -> (String, Ty) {
        if let Some(e) = sc.data.input_edge(node.id, &slot.id) {
            let (from, output) = (e.from, e.output.clone());
            let outs = self.node(sc, from);
            let Some(src) = sc.data.node(from) else {
                return (Self::zero(slot.ty), slot.ty);
            };
            let names = ports(src, &self.cx(sc)).map(|p| p.1).unwrap_or_default();
            return match names.iter().position(|s| s.id == output) {
                Some(k) if k < outs.len() => outs[k].clone(),
                _ => {
                    self.error(sc, node.id, format!("missing output `{output}`"));
                    (Self::zero(slot.ty), slot.ty)
                }
            };
        }
        if slot.ty == Ty::Tex {
            let path = node.option("Texture").unwrap_or("").to_owned();
            let key = format!("{}_{}", node.id, slot.id.replace(' ', "_"));
            return (self.hidden_texture(sc, node.id, &key, &path), Ty::Tex);
        }
        if let Some(v) = node.value(&slot.id) {
            let v = v.to_array();
            let v = if slot.color { linear(v) } else { v };
            let ty = if slot.ty == Ty::Bool {
                Ty::Bool
            } else {
                Ty::F4
            };
            return (value(ty, v), ty);
        }
        if let Some(e) = slot.bind.expr() {
            let ty = if slot.bind == nodes::Bind::Uv0 {
                Ty::F2
            } else {
                Ty::F3
            };
            return (e.into(), ty);
        }
        let v = if slot.color {
            linear(slot.default)
        } else {
            slot.default
        };
        match slot.ty {
            Ty::Bool => (constant(Ty::Bool, v), Ty::Bool),
            Ty::Dyn => (value(Ty::F4, v), Ty::F4),
            t => (value(t, v), t),
        }
    }

    /// Evaluates a node's outputs (memoised).
    fn node(&mut self, sc: &Scope, id: u32) -> Vec<(String, Ty)> {
        let key = (sc.prefix.clone(), id);
        if let Some(v) = self.memo.get(&key) {
            return v.clone();
        }
        let Some(node) = sc.data.node(id) else {
            return Vec::new();
        };
        let ports = match ports(node, &self.cx(sc)) {
            Ok(p) => p,
            Err(e) => {
                self.error(sc, id, e);
                return Vec::new();
            }
        };
        if !self.visiting.insert(key.clone()) {
            self.error(sc, id, "the graph has a loop".into());
            return ports.1.iter().map(|s| (Self::zero(s.ty), s.ty)).collect();
        }
        let out = self.node_inner(sc, node, &ports.0, &ports.1);
        self.visiting.remove(&key);
        self.memo.insert(key, out.clone());
        out
    }

    fn node_inner(
        &mut self,
        sc: &Scope,
        node: &GraphNode,
        ins: &[Slot],
        outs: &[Slot],
    ) -> Vec<(String, Ty)> {
        let def = nodes::find(&node.kind).expect("ports checked the node type");
        let zeros = |outs: &[Slot]| outs.iter().map(|s| (Self::zero(s.ty), s.ty)).collect();
        if def.fragment_only && !self.fragment {
            self.error(
                sc,
                node.id,
                format!("{} only works in the Fragment stage", node.kind),
            );
            return zeros(outs);
        }
        // A Sub Graph's unconnected texture inputs use its own defaults (no slot here).
        let raw: Vec<(String, Ty)> = ins
            .iter()
            .map(|s| {
                if node.kind == "Sub Graph"
                    && s.ty == Ty::Tex
                    && sc.data.input_edge(node.id, &s.id).is_none()
                {
                    (Self::zero(Ty::Tex), Ty::Tex)
                } else {
                    self.input(sc, node, s)
                }
            })
            .collect();
        // Dynamic vectors: the narrowest connected vector wider than a float.
        let width = ins
            .iter()
            .zip(&raw)
            .filter(|(s, _)| s.ty == Ty::Dyn && sc.data.input_edge(node.id, &s.id).is_some())
            .map(|(_, (_, t))| t.width())
            .filter(|w| *w > 1)
            .min()
            .unwrap_or(1);
        let dyn_ty = Ty::vector(width);
        let mut args = Vec::new();
        for (s, (e, t)) in ins.iter().zip(&raw) {
            let want = if s.ty == Ty::Dyn { dyn_ty } else { s.ty };
            match convert(e, *t, want) {
                Some(c) => args.push(c),
                None => {
                    self.error(
                        sc,
                        node.id,
                        format!(
                            "{}: can't use a {} as a {}",
                            s.label,
                            t.label(),
                            want.label()
                        ),
                    );
                    args.push(Self::zero(want));
                }
            }
        }
        let prefix = format!("{}n{}_", sc.prefix, node.id);
        let exprs: Vec<String> = match def.code {
            Gen::Tpl(tpls) => tpls
                .iter()
                .map(|t| {
                    let mut s = t.replace("{T}", dyn_ty.wgsl());
                    for (k, a) in args.iter().enumerate() {
                        s = s.replace(&format!("{{{k}}}"), a);
                    }
                    s
                })
                .collect(),
            Gen::Fun(f) => {
                let mut lines = Vec::new();
                let mut call = nodes::Call::new(
                    args.clone(),
                    dyn_ty,
                    self.fragment,
                    options(node),
                    &mut lines,
                    prefix.clone(),
                );
                let v = f(&mut call);
                self.lines.extend(lines);
                v
            }
            Gen::Special => match self.special(sc, node, &args, &raw, ins, outs, dyn_ty) {
                Some(v) => v,
                None => return zeros(outs),
            },
        };
        outs.iter()
            .zip(exprs)
            .enumerate()
            .map(|(k, (s, e))| {
                let ty = if s.ty == Ty::Dyn { dyn_ty } else { s.ty };
                if ty == Ty::Tex {
                    return (e, ty);
                }
                let name = format!("{prefix}{k}");
                self.lines.push(format!("let {name}: {} = {e};", ty.wgsl()));
                (name, ty)
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn special(
        &mut self,
        sc: &Scope,
        node: &GraphNode,
        args: &[String],
        raw: &[(String, Ty)],
        ins: &[Slot],
        outs: &[Slot],
        dyn_ty: Ty,
    ) -> Option<Vec<String>> {
        let _ = dyn_ty;
        match node.kind.as_str() {
            "Property" => {
                let r = node.option("Property").unwrap_or("");
                let p = sc.properties.iter().find(|p| p.reference == r)?;
                let ty = property_ty(p.kind);
                if sc.owner.is_some() || !sc.prefix.is_empty() {
                    // Inside a Sub Graph: the caller's value, else the property default.
                    if let Some((e, t)) = sc.bound.get(r) {
                        return Some(vec![convert(e, *t, ty).unwrap_or_else(|| Self::zero(ty))]);
                    }
                    if ty == Ty::Tex {
                        let path = p
                            .default_texture
                            .as_ref()
                            .map_or(String::new(), |h| h.path().to_owned());
                        return Some(vec![self.hidden_texture(sc, node.id, r, &path)]);
                    }
                    let v = p.default.to_array();
                    let v = if p.kind == PropertyType::Color {
                        linear(v)
                    } else {
                        v
                    };
                    return Some(vec![value(ty, v)]);
                }
                if ty == Ty::Tex {
                    let Some(slot) =
                        self.alloc
                            .texture(&p.reference, &p.name, p.default_texture.clone())
                    else {
                        self.error(sc, node.id, "more than 5 textures".into());
                        return None;
                    };
                    return Some(vec![TEXTURE_BINDINGS[slot as usize].into()]);
                }
                let slot = self
                    .alloc
                    .props
                    .iter()
                    .find(|e| e.name == p.reference)?
                    .slot;
                Some(vec![match ty {
                    Ty::F1 => format!("graph_props[{slot}].x"),
                    Ty::F2 => format!("graph_props[{slot}].xy"),
                    Ty::F3 => format!("graph_props[{slot}].xyz"),
                    Ty::Bool => format!("(graph_props[{slot}].x > 0.5)"),
                    _ => format!("graph_props[{slot}]"),
                }])
            }
            "Keyword" => {
                let r = node.option("Keyword").unwrap_or("");
                let flag = match self.alloc.props.iter().find(|e| e.name == r) {
                    Some(e) if sc.prefix.is_empty() => format!("(graph_props[{}].x > 0.5)", e.slot),
                    _ => match sc.keywords.iter().find(|k| k.reference == r) {
                        Some(k) => k.default.to_string(),
                        None => {
                            self.error(sc, node.id, format!("keyword `{r}` not found"));
                            "false".into()
                        }
                    },
                };
                Some(vec![format!("select({}, {}, {flag})", args[1], args[0])])
            }
            "Texture 2D Asset" => {
                let path = node.option("Texture").unwrap_or("").to_owned();
                Some(vec![self.hidden_texture(
                    sc,
                    node.id,
                    &node.id.to_string(),
                    &path,
                )])
            }
            "Swizzle" => {
                let mask: String = node
                    .option("Mask")
                    .unwrap_or("xyzw")
                    .chars()
                    .map(|c| match c {
                        'r' => 'x',
                        'g' => 'y',
                        'b' => 'z',
                        'a' => 'w',
                        c => c,
                    })
                    .collect();
                if mask.is_empty() || mask.len() > 4 || !mask.chars().all(|c| "xyzw".contains(c)) {
                    self.error(sc, node.id, format!("invalid mask `{mask}`"));
                    return None;
                }
                Some(vec![format!("({}).{mask}", args[0])])
            }
            "Custom Function" => {
                let name = format!("sg_cf_{}n{}", sc.prefix, node.id);
                let body = options(node)
                    .into_iter()
                    .find(|(k, _)| *k == "Body")
                    .map_or(String::new(), |(_, v)| v);
                let params: Vec<String> = ins
                    .iter()
                    .map(|s| format!("{}: {}", s.id, s.ty.wgsl()))
                    .chain(std::iter::once("i: SurfaceInput".into()))
                    .collect();
                let fields: Vec<String> = outs
                    .iter()
                    .map(|s| format!("    {}: {},", s.id, s.ty.wgsl()))
                    .collect();
                let decls: Vec<String> = outs
                    .iter()
                    .map(|s| format!("    var {}: {};", s.id, s.ty.wgsl()))
                    .collect();
                let assigns: Vec<String> = outs
                    .iter()
                    .map(|s| format!("    r.{0} = {0};", s.id))
                    .collect();
                let code = format!(
                    "struct {name}_out {{\n{}\n}}\nfn {name}({}) -> {name}_out {{\n{}\n    {body}\n    var r: {name}_out;\n{}\n    return r;\n}}\n",
                    fields.join("\n"),
                    params.join(", "),
                    decls.join("\n"),
                    assigns.join("\n"),
                );
                if outs.is_empty() {
                    self.error(sc, node.id, "Custom Function has no outputs".into());
                    return None;
                }
                if !self.functions.iter().any(|(n, _)| *n == name) {
                    self.functions.push((name.clone(), code));
                }
                let call_args: Vec<String> = args
                    .iter()
                    .cloned()
                    .chain(std::iter::once("i".into()))
                    .collect();
                let r = format!("{name}_r");
                self.lines
                    .push(format!("let {r} = {name}({});", call_args.join(", ")));
                Some(outs.iter().map(|s| format!("{r}.{}", s.id)).collect())
            }
            "Sub Graph" => {
                let path = node.option("Sub Graph").unwrap_or("").to_owned();
                if sc.stack.contains(&path) {
                    self.error(sc, node.id, format!("{path} uses itself"));
                    return None;
                }
                let sub = (self.subgraphs)(&path)?;
                if !self.deps.contains(&path) {
                    self.deps.push(path.clone());
                }
                let mut bound = HashMap::new();
                for (s, r) in ins.iter().zip(raw) {
                    if sc.data.input_edge(node.id, &s.id).is_some() || node.value(&s.id).is_some() {
                        bound.insert(s.id.clone(), r.clone());
                    }
                }
                let mut stack = sc.stack.clone();
                stack.push(path);
                let child = Scope {
                    data: &sub.graph,
                    properties: &sub.properties,
                    keywords: &[],
                    sub_outputs: &sub.outputs,
                    prefix: format!("{}s{}_", sc.prefix, node.id),
                    bound,
                    owner: Some(sc.owner.unwrap_or(node.id)),
                    stack,
                };
                let Some(out_node) = sub.graph.find("SubGraphOutput") else {
                    self.error(sc, node.id, "the Sub Graph has no Output node".into());
                    return None;
                };
                let out_node = out_node.clone();
                let slots = ports(&out_node, &self.cx(&child)).ok()?.0;
                Some(
                    slots
                        .iter()
                        .map(|s| {
                            let (e, t) = self.input(&child, &out_node, s);
                            convert(&e, t, s.ty).unwrap_or_else(|| Self::zero(s.ty))
                        })
                        .collect(),
                )
            }
            _ => None,
        }
    }

    /// The value of a master stack block, converted to its type.
    fn block(&mut self, sc: &Scope, stack: &GraphNode, id: &str) -> String {
        let cx = self.cx(sc);
        let slot = ports(stack, &cx)
            .ok()
            .and_then(|p| p.0.into_iter().find(|s| s.id == id))
            .expect("master stack block");
        let (e, t) = self.input(sc, stack, &slot);
        convert(&e, t, slot.ty).unwrap_or_else(|| Self::zero(slot.ty))
    }
}

const VERTEX_INPUT: &str = "
fn sg_vertex_si(v: VertexInput) -> SurfaceInput {
    var i: SurfaceInput;
    i.uv0 = v.uv0;
    i.uv1 = v.uv1;
    i.object_pos = v.position;
    i.object_normal = v.normal;
    i.object_to_world = v.object_to_world;
    i.world_pos = (v.object_to_world * vec4f(v.position, 1.0)).xyz;
    let n = sg_safe_normalize((v.object_to_world * vec4f(v.normal, 0.0)).xyz);
    i.world_normal = n;
    var t = (v.object_to_world * vec4f(v.tangent, 0.0)).xyz;
    if (dot(t, t) < 1e-10) {
        t = cross(n, select(vec3f(0.0, 1.0, 0.0), vec3f(1.0, 0.0, 0.0), abs(n.y) > 0.9));
    }
    i.world_tangent = sg_safe_normalize(t);
    i.world_bitangent = cross(n, i.world_tangent);
    i.view_dir = sg_safe_normalize(sg_camera_pos() - i.world_pos);
    i.vertex_color = v.color;
    i.screen_pos = vec4f(0.0, 0.0, 0.0, 1.0);
    i.front = true;
    i.in_vertex = true;
    return i;
}
";

/// Magenta hooks for a graph that can't compile (Unity's error shader).
pub const ERROR_HOOKS: &str = "
fn vertex_mod(v: VertexInput) -> VertexMod {
    var m: VertexMod;
    m.position = v.position;
    m.normal = v.normal;
    m.tangent = v.tangent;
    return m;
}
fn surface(i: SurfaceInput) -> Surface {
    var s: Surface;
    s.base_color = vec3f(0.0);
    s.emission = vec3f(1.0, 0.0, 1.0);
    s.alpha = 1.0;
    s.normal_ts = vec3f(0.0, 0.0, 1.0);
    s.occlusion = 1.0;
    s.alpha_clip = -1.0;
    return s;
}
";

/// Compiles a Shader Graph.
pub fn compile(graph: &ShaderGraph, subgraphs: SubGraphs, target: Target) -> Compiled {
    let mut errors = Vec::new();
    let mut alloc = Alloc::default();
    let mut functions = Vec::new();
    let mut deps = Vec::new();
    // Blackboard properties and keywords keep their slots in Blackboard order.
    for p in &graph.properties {
        if p.kind == PropertyType::Texture2D {
            if alloc
                .texture(&p.reference, &p.name, p.default_texture.clone())
                .is_none()
            {
                errors.push((0, format!("{}: more than 5 textures", p.name)));
            }
            continue;
        }
        let v = alloc.value(ShaderProperty {
            name: p.reference.clone(),
            display_name: p.name.clone(),
            kind: p.kind.kind_name().into(),
            slot: 0,
            default: p.default.to_array(),
            default_texture: None,
            range: (p.float_mode == FloatMode::Slider).then_some((p.range.x, p.range.y)),
        });
        if v.is_none() {
            errors.push((0, format!("{}: more than {VALUE_SLOTS} properties", p.name)));
        }
    }
    for k in &graph.keywords {
        alloc.value(ShaderProperty {
            name: k.reference.clone(),
            display_name: k.name.clone(),
            kind: "Boolean".into(),
            slot: 0,
            default: [k.default as u32 as f32, 0.0, 0.0, 0.0],
            default_texture: None,
            range: None,
        });
    }
    let sc = Scope {
        data: &graph.graph,
        properties: &graph.properties,
        keywords: &graph.keywords,
        sub_outputs: &[],
        prefix: String::new(),
        bound: HashMap::new(),
        owner: None,
        stack: Vec::new(),
    };
    let settings = &graph.settings;
    let mut out = Compiled {
        unlit: settings.material == GraphMaterial::Unlit,
        transparent: settings.surface == GraphSurface::Transparent,
        alpha_clip: settings.alpha_clipping,
        double_sided: settings.render_face == RenderFace::Both,
        cast_shadows: settings.cast_shadows,
        ..Default::default()
    };
    let default_stack = |kind: &str| GraphNode::new(0, kind, Default::default());
    let vstack = graph
        .graph
        .find("Vertex")
        .cloned()
        .unwrap_or_else(|| default_stack("Vertex"));
    let fstack = graph
        .graph
        .find("Fragment")
        .cloned()
        .unwrap_or_else(|| default_stack("Fragment"));

    // ---- Vertex stage.
    let (vertex_lines, position, normal, tangent) = {
        let mut e = Emit {
            fragment: false,
            lines: Vec::new(),
            memo: HashMap::new(),
            visiting: HashSet::new(),
            errors: &mut errors,
            alloc: &mut alloc,
            functions: &mut functions,
            subgraphs,
            deps: &mut deps,
        };
        if target == Target::Material {
            let p = e.block(&sc, &vstack, "Position");
            let n = e.block(&sc, &vstack, "Normal");
            let t = e.block(&sc, &vstack, "Tangent");
            (e.lines, p, n, t)
        } else {
            (
                Vec::new(),
                "v.position".into(),
                "v.normal".into(),
                "v.tangent".into(),
            )
        }
    };
    let vertex_changed =
        target == Target::Material && graph.graph.edges.iter().any(|e| e.to == vstack.id);

    // ---- Fragment stage.
    let mut e = Emit {
        fragment: true,
        lines: Vec::new(),
        memo: HashMap::new(),
        visiting: HashSet::new(),
        errors: &mut errors,
        alloc: &mut alloc,
        functions: &mut functions,
        subgraphs,
        deps: &mut deps,
    };
    let surface_body = match target {
        Target::Material => {
            let base = e.block(&sc, &fstack, "Base Color");
            let normal_in = e.block(&sc, &fstack, "Normal");
            let normal_ts = match settings.fragment_normal_space {
                NormalSpace::Tangent => normal_in,
                NormalSpace::World => format!("sg_world_to_tangent(i, normalize({normal_in}))"),
                NormalSpace::Object => format!(
                    "sg_world_to_tangent(i, normalize((i.object_to_world * vec4f({normal_in}, 0.0)).xyz))"
                ),
            };
            let metallic = e.block(&sc, &fstack, "Metallic");
            let smooth = e.block(&sc, &fstack, "Smoothness");
            let ao = e.block(&sc, &fstack, "Ambient Occlusion");
            let emission = e.block(&sc, &fstack, "Emission");
            let alpha = e.block(&sc, &fstack, "Alpha");
            let clip = if settings.alpha_clipping {
                e.block(&sc, &fstack, "Alpha Clip Threshold")
            } else {
                "-1.0".into()
            };
            format!(
                "    var s: Surface;\n    s.base_color = {base};\n    s.alpha = {alpha};\n    s.normal_ts = {normal_ts};\n    s.metallic = {metallic};\n    s.smoothness = {smooth};\n    s.occlusion = {ao};\n    s.emission = {emission};\n    s.alpha_clip = {clip};\n    return s;"
            )
        }
        Target::Preview => {
            alloc_preview(e.alloc);
            let mut cases = Vec::new();
            let ids: Vec<u32> = graph
                .graph
                .nodes
                .iter()
                .filter(|n| !is_stack(&n.kind) && n.kind != "SubGraphOutput")
                .map(|n| n.id)
                .collect();
            for id in ids {
                let outs = e.node(&sc, id);
                if let Some((x, t)) = outs.first() {
                    cases.push(format!(
                        "        case {}u: {{ p = {}; }}",
                        out.preview_nodes.len(),
                        nodes::to_color(x, *t)
                    ));
                    out.preview_nodes.push(id);
                }
            }
            format!(
                "    var p = vec4f(0.0, 0.0, 0.0, 1.0);\n    switch u32(graph_props[{PREVIEW_SLOT}].x) {{\n{}\n        default: {{}}\n    }}\n    var s: Surface;\n    s.base_color = p.rgb;\n    s.alpha = 1.0;\n    s.normal_ts = vec3f(0.0, 0.0, 1.0);\n    s.metallic = 0.0;\n    s.smoothness = 0.0;\n    s.occlusion = 1.0;\n    s.emission = vec3f(0.0);\n    s.alpha_clip = -1.0;\n    return s;",
                cases.join("\n")
            )
        }
    };
    let fragment_lines = std::mem::take(&mut e.lines);
    drop(e);

    let indent = |lines: &[String]| {
        lines
            .iter()
            .map(|l| format!("    {l}\n"))
            .collect::<String>()
    };
    let mut hooks = String::from(HELPERS);
    for (_, f) in &functions {
        hooks.push_str(f);
    }
    hooks.push_str(VERTEX_INPUT);
    hooks.push_str(&format!(
        "\nfn vertex_mod(v: VertexInput) -> VertexMod {{\n    let i = sg_vertex_si(v);\n{}    var m: VertexMod;\n    m.position = {position};\n    m.normal = {normal};\n    m.tangent = {tangent};\n    return m;\n}}\n",
        indent(&vertex_lines)
    ));
    hooks.push_str(&format!(
        "\nfn surface(i: SurfaceInput) -> Surface {{\n{}{surface_body}\n}}\n",
        indent(&fragment_lines)
    ));
    if target == Target::Preview {
        out.unlit = true;
        out.transparent = false;
        out.alpha_clip = false;
        out.cast_shadows = false;
    }
    out.hooks = hooks;
    out.shadow = target == Target::Material && (vertex_changed || settings.alpha_clipping);
    out.properties = alloc.props;
    out.errors = errors;
    out.dependencies = deps;
    out
}

fn alloc_preview(alloc: &mut Alloc) {
    alloc.props.push(ShaderProperty {
        name: PREVIEW_PROPERTY.into(),
        display_name: "Preview".into(),
        kind: "Float".into(),
        slot: PREVIEW_SLOT,
        default: [0.0; 4],
        default_texture: None,
        range: None,
    });
}

/// Every Sub Graph a graph uses, directly or through other Sub Graphs.
pub fn sub_graph_paths(data: &GraphData, subgraphs: SubGraphs) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut stack: Vec<String> = data
        .nodes
        .iter()
        .filter(|n| n.kind == "Sub Graph")
        .filter_map(|n| n.option("Sub Graph").map(str::to_owned))
        .collect();
    while let Some(p) = stack.pop() {
        if out.contains(&p) {
            continue;
        }
        if let Some(s) = subgraphs(&p) {
            stack.extend(
                s.graph
                    .nodes
                    .iter()
                    .filter(|n| n.kind == "Sub Graph")
                    .filter_map(|n| n.option("Sub Graph").map(str::to_owned)),
            );
        }
        out.push(p);
    }
    out
}

/// Wraps a Sub Graph as a graph whose Fragment shows its first output (the Sub Graph
/// window's main preview and node previews).
pub fn sub_graph_as_graph(sub: &ShaderSubGraph) -> ShaderGraph {
    let mut g = ShaderGraph {
        settings: GraphSettings {
            material: GraphMaterial::Unlit,
            ..Default::default()
        },
        properties: sub.properties.clone(),
        keywords: Vec::new(),
        graph: sub.graph.clone(),
    };
    // The Output node's first input feeds Base Color.
    if let Some(out) = sub.graph.find("SubGraphOutput")
        && let Some(first) = sub.outputs.first()
        && let Some(e) = sub.graph.input_edge(out.id, &first.name)
    {
        let frag = g
            .graph
            .add(GraphNode::new(0, "Fragment", Default::default()));
        g.graph
            .connect(e.from, &e.output.clone(), frag, "Base Color");
    }
    g
}
