//! Compiles a Visual Effect Graph into one GPU program per system.
//!
//! A system is an Initialize Particle context with the Spawn contexts flowing into it and
//! the Update and Output contexts it flows to. Its Initialize, Update and Output blocks
//! become the `vfx_init`, `vfx_update` and `vfx_output` hooks (see `oxide_render::vfx`);
//! operators wired into block inputs are generated inline, like Shader Graph nodes. Spawn
//! blocks stay on the CPU.

use crate::asset::*;
use crate::blocks::{self, ATTRIBUTES, BlockCode, BlockCx, Stage};
use oxide_math::Vector4;
use oxide_render::{VfxBlend, VfxOrient, VfxProgram};
use oxide_shadergraph::nodes::{self, Gen, NodeDef, Opt, constant, convert};
use oxide_shadergraph::{GraphNode, PropertyType, Slot, Ty, property_ty};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

/// Samples per baked gradient or curve.
pub const LUT_SIZE: usize = 64;
/// Property slots (vec4).
pub const PROPERTY_SLOTS: usize = 64;

fn opt(name: &'static str, choices: &'static [&'static str], default: &'static str) -> Opt {
    Opt {
        name,
        choices,
        default,
        multiline: false,
    }
}

/// A context's settings (Unity's context Inspector).
pub fn context_options(kind: &str) -> Vec<Opt> {
    match kind {
        INITIALIZE => vec![
            opt("Capacity", &[], "256"),
            opt("Space", &["Local", "World"], "Local"),
        ],
        OUTPUT => vec![
            opt("Blend Mode", &["Alpha", "Additive", "Opaque"], "Alpha"),
            opt(
                "Orient",
                &["Face Camera Plane", "Along Velocity"],
                "Face Camera Plane",
            ),
            opt("Main Texture", &[], ""),
            opt("Flipbook", &[], "None"),
        ],
        _ => Vec::new(),
    }
}

fn special(
    kind: &'static str,
    menu: &'static str,
    ins: Vec<Slot>,
    outs: Vec<Slot>,
    opts: Vec<Opt>,
) -> NodeDef {
    NodeDef {
        kind,
        menu,
        inputs: ins,
        outputs: outs,
        options: opts,
        code: Gen::Special,
        fragment_only: false,
        preview: false,
    }
}

const ATTRIBUTE_NAMES: &[&str] = &[
    "position",
    "velocity",
    "age",
    "lifetime",
    "color",
    "alpha",
    "size",
    "angle",
    "texIndex",
    "mass",
    "scaleY",
    "particleId",
    "seed",
];

/// VFX Graph's own operators (the Shader Graph math library is shared).
pub fn vfx_operators() -> &'static [NodeDef] {
    static OPS: OnceLock<Vec<NodeDef>> = OnceLock::new();
    OPS.get_or_init(|| {
        vec![
            special(
                "Get Attribute",
                "Attribute",
                vec![],
                vec![],
                vec![opt("Attribute", ATTRIBUTE_NAMES, "position")],
            ),
            special(
                "Age Over Lifetime",
                "Attribute",
                vec![],
                vec![Slot::new("t", Ty::F1)],
                vec![],
            ),
            special(
                "Random Number",
                "Math/Random",
                vec![Slot::new("Min", Ty::F1), Slot::new("Max", Ty::F1).f(1.0)],
                vec![Slot::new("r", Ty::F1)],
                vec![],
            ),
            special(
                "Delta Time",
                "Builtin",
                vec![],
                vec![Slot::new("Delta Time", Ty::F1)],
                vec![],
            ),
            special(
                "Total Time",
                "Builtin",
                vec![],
                vec![Slot::new("Total Time", Ty::F1)],
                vec![],
            ),
            special(
                "Property",
                "Property",
                vec![],
                vec![],
                vec![opt("Property", &[], "")],
            ),
        ]
    })
}

/// Shader Graph nodes that work in VFX Graph (pure math: no mesh inputs or textures).
pub fn operator_allowed(def: &NodeDef) -> bool {
    let menu = def.menu;
    if def.fragment_only
        || matches!(
            def.kind,
            "Transform"
                | "Fresnel Effect"
                | "Dither"
                | "Custom Function"
                | "Sub Graph"
                | "Keyword"
                | "Property"
        )
    {
        return false;
    }
    menu.starts_with("Math")
        || menu.starts_with("Channel")
        || menu.starts_with("Procedural")
        || menu.starts_with("Artistic/Adjustment")
        || menu.starts_with("Artistic/Blend")
        || menu.starts_with("Artistic/Utility")
        || menu.starts_with("Utility/Logic")
        || menu == "Input/Basic"
        || menu == "UV"
}

/// A node type usable in a VFX Graph (operator or context).
pub fn find_operator(kind: &str) -> Option<&'static NodeDef> {
    vfx_operators()
        .iter()
        .find(|d| d.kind == kind)
        .or_else(|| nodes::find(kind).filter(|d| operator_allowed(d)))
}

fn attribute(name: &str) -> Option<(&'static str, Ty)> {
    ATTRIBUTES
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, e, t)| (*e, *t))
}

fn short(kind: &str) -> String {
    kind.strip_prefix("Set ").unwrap_or(kind).to_owned()
}

/// A node's ports in a VFX Graph: contexts (flow and their blocks' inputs) and operators.
pub fn ports(asset: &VisualEffectAsset, node: &GraphNode) -> (Vec<Slot>, Vec<Slot>) {
    if is_context(&node.kind) {
        let mut ins = Vec::new();
        if node.kind != SPAWN {
            ins.push(Slot::new("Flow In", Ty::Flow).labelled("▸"));
        }
        for b in asset.blocks_of(node.id) {
            let Some(def) = blocks::find(&b.kind) else {
                continue;
            };
            for s in &def.inputs {
                let mut slot = s.clone();
                slot.label = format!("{} › {}", short(&b.kind), s.label);
                slot.id = b.port(&s.id);
                ins.push(slot);
            }
        }
        let outs = if node.kind == OUTPUT {
            Vec::new()
        } else {
            vec![Slot::new("Flow Out", Ty::Flow).labelled("▸")]
        };
        return (ins, outs);
    }
    match node.kind.as_str() {
        "Property" => {
            let r = node.option("Property").unwrap_or("");
            match asset.properties.iter().find(|p| p.reference == r) {
                Some(p) => (
                    vec![],
                    vec![Slot::new("Out", property_ty(p.kind)).labelled(&p.name)],
                ),
                None => (vec![], vec![]),
            }
        }
        "Get Attribute" => {
            let a = node.option("Attribute").unwrap_or("position");
            match attribute(a) {
                Some((_, t)) => (vec![], vec![Slot::new(a, t)]),
                None => (vec![], vec![]),
            }
        }
        _ => match find_operator(&node.kind) {
            Some(d) if node.kind == "Swizzle" => {
                let n = node.option("Mask").unwrap_or("xyzw").len().clamp(1, 4) as u32;
                (d.inputs.clone(), vec![Slot::new("Out", Ty::vector(n))])
            }
            Some(d) => (d.inputs.clone(), d.outputs.clone()),
            None => (vec![], vec![]),
        },
    }
}

/// One input of a spawn block: a value or a property.
#[derive(Clone, Debug, PartialEq)]
pub struct SpawnInput {
    pub value: f32,
    pub property: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpawnBlock {
    /// Unity block name (`Constant Spawn Rate`, `Single Burst`, ...).
    pub kind: String,
    pub id: u32,
    pub inputs: HashMap<String, SpawnInput>,
}

impl SpawnBlock {
    /// An input's value given the effect's property values (by reference).
    pub fn get(&self, name: &str, props: &dyn Fn(&str) -> Option<f32>) -> f32 {
        self.inputs.get(name).map_or(0.0, |i| {
            i.property.as_deref().and_then(props).unwrap_or(i.value)
        })
    }
}

/// One compiled system.
#[derive(Clone, Debug)]
pub struct System {
    pub initialize: u32,
    pub output: Option<u32>,
    pub spawn: Vec<SpawnBlock>,
    pub program: Arc<VfxProgram>,
    /// The output's Main Texture.
    pub texture: Option<String>,
}

/// An exposed property's slot in the program's property uniform.
#[derive(Clone, Debug, PartialEq)]
pub struct PropertySlot {
    pub reference: String,
    pub name: String,
    pub kind: PropertyType,
    pub slot: u32,
    pub default: Vector4,
}

#[derive(Clone, Debug, Default)]
pub struct Compiled {
    pub systems: Vec<System>,
    /// Errors by node id (0: the graph).
    pub errors: Vec<(u32, String)>,
    pub properties: Vec<PropertySlot>,
}

/// WGSL shared by every program's hooks: the Shader Graph helpers (operators call them)
/// and VFX Graph's own (random directions, noise, baked curves).
const VFX_HELPERS: &str = r#"
struct SurfaceInput {
    uv0: vec2f,
    uv1: vec2f,
    world_pos: vec3f,
    object_pos: vec3f,
    world_normal: vec3f,
    object_normal: vec3f,
    world_tangent: vec3f,
    world_bitangent: vec3f,
    view_dir: vec3f,
    vertex_color: vec4f,
    screen_pos: vec4f,
    front: bool,
    in_vertex: bool,
    object_to_world: mat4x4f,
};

fn sg_view_matrix() -> mat4x4f {
    return vfx_view_matrix();
}

fn sg_camera_pos() -> vec3f {
    return vfx_camera_pos();
}

fn sg_time() -> f32 {
    return vfx_time();
}

fn vfx_random_direction(r1: f32, r2: f32) -> vec3f {
    let z = r1 * 2.0 - 1.0;
    let a = r2 * 6.28318530718;
    let s = sqrt(max(0.0, 1.0 - z * z));
    return vec3f(s * cos(a), z, s * sin(a));
}

fn vfx_lut(base: u32, t: f32) -> vec4f {
    let x = clamp(t, 0.0, 1.0) * 63.0;
    let i = u32(floor(x));
    let j = min(i + 1u, 63u);
    return mix(luts[base + i], luts[base + j], fract(x));
}

fn vfx_hash3(p: vec3f) -> vec3f {
    let q = vec3f(
        dot(p, vec3f(127.1, 311.7, 74.7)),
        dot(p, vec3f(269.5, 183.3, 246.1)),
        dot(p, vec3f(113.5, 271.9, 124.6)));
    return fract(sin(q) * 43758.5453) * 2.0 - 1.0;
}

fn vfx_noise3(p: vec3f) -> vec3f {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(vfx_hash3(i), vfx_hash3(i + vec3f(1.0, 0.0, 0.0)), u.x);
    let b = mix(vfx_hash3(i + vec3f(0.0, 1.0, 0.0)), vfx_hash3(i + vec3f(1.0, 1.0, 0.0)), u.x);
    let c = mix(vfx_hash3(i + vec3f(0.0, 0.0, 1.0)), vfx_hash3(i + vec3f(1.0, 0.0, 1.0)), u.x);
    let d = mix(vfx_hash3(i + vec3f(0.0, 1.0, 1.0)), vfx_hash3(i + vec3f(1.0, 1.0, 1.0)), u.x);
    return mix(mix(a, b, u.y), mix(c, d, u.y), u.z);
}

fn vfx_fbm3(p: vec3f, octaves: f32, roughness: f32, lacunarity: f32) -> vec3f {
    var sum = vec3f(0.0);
    var amp = 1.0;
    var freq = 1.0;
    var norm = 0.0;
    for (var k = 0; k < 8; k++) {
        if (f32(k) >= max(octaves, 1.0)) {
            break;
        }
        sum += vfx_noise3(p * freq) * amp;
        norm += amp;
        amp *= roughness;
        freq *= lacunarity;
    }
    return sum / max(norm, 1e-4);
}
"#;

struct Emit<'a> {
    asset: &'a VisualEffectAsset,
    slots: &'a HashMap<String, u32>,
    stage: Stage,
    lines: Vec<String>,
    memo: HashMap<u32, Vec<(String, Ty)>>,
    visiting: HashSet<u32>,
    errors: &'a mut Vec<(u32, String)>,
    salt: u32,
}

impl Emit<'_> {
    fn error(&mut self, node: u32, msg: String) {
        if !self.errors.iter().any(|(n, m)| *n == node && *m == msg) {
            self.errors.push((node, msg));
        }
    }

    fn zero(ty: Ty) -> String {
        match ty {
            Ty::Tex | Ty::Flow => String::new(),
            t => format!("{}(0.0)", if t == Ty::Dyn { "f32" } else { t.wgsl() }),
        }
    }

    /// What an unconnected operator input reads (VFX Graph has no mesh: positions read
    /// the particle's).
    fn bind(b: nodes::Bind) -> Option<(String, Ty)> {
        Some(match b {
            nodes::Bind::None => return None,
            nodes::Bind::Uv0 => ("q.position.xy".into(), Ty::F2),
            nodes::Bind::ObjectPos | nodes::Bind::WorldPos => ("q.position".into(), Ty::F3),
            nodes::Bind::ViewDirWorld => {
                ("normalize(vfx_camera_pos() - q.position)".into(), Ty::F3)
            }
            nodes::Bind::TangentNormal => ("vec3f(0.0, 0.0, 1.0)".into(), Ty::F3),
            _ => ("vec3f(0.0, 1.0, 0.0)".into(), Ty::F3),
        })
    }

    /// The value wired into (or typed into) input `slot` of node `to`.
    fn input(&mut self, to: u32, slot: &Slot, inline: Option<Vector4>) -> (String, Ty) {
        if let Some(e) = self.asset.graph.input_edge(to, &slot.id).cloned() {
            let outs = self.node(e.from);
            let names = self
                .asset
                .graph
                .node(e.from)
                .map(|n| ports(self.asset, n).1)
                .unwrap_or_default();
            return match names.iter().position(|s| s.id == e.output) {
                Some(k) if k < outs.len() => outs[k].clone(),
                _ => {
                    self.error(to, format!("missing output `{}`", e.output));
                    (Self::zero(slot.ty), slot.ty)
                }
            };
        }
        if let Some(v) = inline {
            let v = v.to_array();
            let v = if slot.color { linear(v) } else { v };
            let ty = if slot.ty == Ty::Bool {
                Ty::Bool
            } else {
                Ty::F4
            };
            return (value(ty, v), ty);
        }
        if let Some(b) = Self::bind(slot.bind) {
            return b;
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

    fn node(&mut self, id: u32) -> Vec<(String, Ty)> {
        if let Some(v) = self.memo.get(&id) {
            return v.clone();
        }
        let Some(node) = self.asset.graph.node(id).cloned() else {
            return Vec::new();
        };
        let (ins, outs) = ports(self.asset, &node);
        if is_context(&node.kind) {
            self.error(id, "a context can't feed a block input".into());
            return Vec::new();
        }
        if !self.visiting.insert(id) {
            self.error(id, "the graph has a loop".into());
            return outs.iter().map(|s| (Self::zero(s.ty), s.ty)).collect();
        }
        let out = self.node_inner(&node, &ins, &outs);
        self.visiting.remove(&id);
        self.memo.insert(id, out.clone());
        out
    }

    fn node_inner(&mut self, node: &GraphNode, ins: &[Slot], outs: &[Slot]) -> Vec<(String, Ty)> {
        let Some(def) = find_operator(&node.kind) else {
            self.error(
                node.id,
                format!("`{}` can't be used in a VFX Graph", node.kind),
            );
            return outs.iter().map(|s| (Self::zero(s.ty), s.ty)).collect();
        };
        let raw: Vec<(String, Ty)> = ins
            .iter()
            .map(|s| self.input(node.id, s, node.value(&s.id)))
            .collect();
        let width = ins
            .iter()
            .zip(&raw)
            .filter(|(s, _)| {
                s.ty == Ty::Dyn && self.asset.graph.input_edge(node.id, &s.id).is_some()
            })
            .map(|(_, (_, t))| t.width())
            .filter(|w| *w > 1)
            .min()
            .unwrap_or(1);
        let dyn_ty = Ty::vector(width);
        let mut args = Vec::new();
        for (s, (e, t)) in ins.iter().zip(&raw) {
            let want = if s.ty == Ty::Dyn { dyn_ty } else { s.ty };
            args.push(convert(e, *t, want).unwrap_or_else(|| Self::zero(want)));
        }
        let per_particle = self.stage != Stage::Spawn;
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
                    false,
                    oxide_shadergraph::options(node),
                    &mut lines,
                    format!("o{}_", node.id),
                );
                let v = f(&mut call);
                self.lines.extend(lines);
                v
            }
            Gen::Special => match node.kind.as_str() {
                "Property" => {
                    let r = node.option("Property").unwrap_or("");
                    let (Some(p), Some(slot)) = (
                        self.asset.properties.iter().find(|p| p.reference == r),
                        self.slots.get(r),
                    ) else {
                        self.error(node.id, format!("property `{r}` not found"));
                        return outs.iter().map(|s| (Self::zero(s.ty), s.ty)).collect();
                    };
                    vec![match property_ty(p.kind) {
                        Ty::F1 => format!("props[{slot}].x"),
                        Ty::F2 => format!("props[{slot}].xy"),
                        Ty::F3 => format!("props[{slot}].xyz"),
                        Ty::Bool => format!("(props[{slot}].x > 0.5)"),
                        _ => format!("props[{slot}]"),
                    }]
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
                    if mask.is_empty()
                        || mask.len() > 4
                        || !mask.chars().all(|c| "xyzw".contains(c))
                    {
                        self.error(node.id, format!("invalid mask `{mask}`"));
                        return outs.iter().map(|s| (Self::zero(s.ty), s.ty)).collect();
                    }
                    vec![format!("({}).{mask}", args[0])]
                }
                _ if !per_particle => {
                    self.error(
                        node.id,
                        format!("{} needs a particle (not in Spawn)", node.kind),
                    );
                    return outs.iter().map(|s| (Self::zero(s.ty), s.ty)).collect();
                }
                "Get Attribute" => {
                    let a = node.option("Attribute").unwrap_or("position");
                    match attribute(a) {
                        Some((e, _)) => vec![e.into()],
                        None => {
                            self.error(node.id, format!("unknown attribute `{a}`"));
                            return Vec::new();
                        }
                    }
                }
                "Age Over Lifetime" => vec!["(q.age / max(q.lifetime, 1e-6))".into()],
                "Random Number" => {
                    self.salt += 1;
                    vec![format!(
                        "mix({}, {}, vfx_rand(q.seed, {}u))",
                        args[0],
                        args[1],
                        10_000 + node.id * 64 + self.salt
                    )]
                }
                "Delta Time" => vec!["vfx_dt()".into()],
                "Total Time" => vec!["vfx_time()".into()],
                _ => Vec::new(),
            },
        };
        outs.iter()
            .zip(exprs)
            .enumerate()
            .map(|(k, (s, e))| {
                let ty = if s.ty == Ty::Dyn { dyn_ty } else { s.ty };
                let name = format!("o{}_{k}", node.id);
                self.lines.push(format!("let {name}: {} = {e};", ty.wgsl()));
                (name, ty)
            })
            .collect()
    }
}

fn linear(v: [f32; 4]) -> [f32; 4] {
    let c = oxide_math::Color::new(v[0], v[1], v[2], v[3]).linear();
    [c.r, c.g, c.b, v[3]]
}

/// An inline value evaluated at run time (see the Shader Graph compiler).
fn value(ty: Ty, v: [f32; 4]) -> String {
    match ty {
        Ty::F1 | Ty::Dyn => format!("sg_k1({})", constant(Ty::F1, v)),
        Ty::F2 => format!("sg_k2({})", constant(ty, v)),
        Ty::F3 => format!("sg_k3({})", constant(ty, v)),
        Ty::F4 => format!("sg_k4({})", constant(ty, v)),
        t => constant(t, v),
    }
}

fn flow_targets(asset: &VisualEffectAsset, from: u32, kind: &str) -> Vec<u32> {
    asset
        .graph
        .edges
        .iter()
        .filter(|e| e.from == from && e.output == "Flow Out" && e.input == "Flow In")
        .filter(|e| asset.graph.node(e.to).is_some_and(|n| n.kind == kind))
        .map(|e| e.to)
        .collect()
}

/// Bakes a gradient (linear colour, alpha) into LUT_SIZE samples.
fn bake_gradient(g: &oxide_core::curves::Gradient) -> Vec<[f32; 4]> {
    (0..LUT_SIZE)
        .map(|k| {
            let c = g.evaluate(k as f32 / (LUT_SIZE - 1) as f32);
            let l = c.linear();
            [l.r, l.g, l.b, c.a]
        })
        .collect()
}

fn bake_curve(c: &oxide_core::curves::AnimationCurve) -> Vec<[f32; 4]> {
    (0..LUT_SIZE)
        .map(|k| [c.evaluate(k as f32 / (LUT_SIZE - 1) as f32); 4])
        .collect()
}

/// Compiles every system of an asset.
pub fn compile(asset: &VisualEffectAsset, name: &str) -> Compiled {
    let mut out = Compiled::default();
    // Exposed properties take slots in Blackboard order.
    let mut slots = HashMap::new();
    for p in &asset.properties {
        if p.kind == PropertyType::Texture2D {
            out.errors.push((
                0,
                format!(
                    "{}: texture properties go on the output's Main Texture",
                    p.name
                ),
            ));
            continue;
        }
        let slot = slots.len() as u32;
        if slot as usize >= PROPERTY_SLOTS {
            out.errors.push((
                0,
                format!("{}: more than {PROPERTY_SLOTS} properties", p.name),
            ));
            continue;
        }
        slots.insert(p.reference.clone(), slot);
        out.properties.push(PropertySlot {
            reference: p.reference.clone(),
            name: p.name.clone(),
            kind: p.kind,
            slot,
            default: p.default,
        });
    }

    for init in asset.graph.nodes.iter().filter(|n| n.kind == INITIALIZE) {
        let spawns: Vec<u32> = asset
            .graph
            .edges
            .iter()
            .filter(|e| e.to == init.id && e.input == "Flow In")
            .filter(|e| asset.graph.node(e.from).is_some_and(|n| n.kind == SPAWN))
            .map(|e| e.from)
            .collect();
        if spawns.is_empty() {
            out.errors.push((
                init.id,
                "nothing spawns into this system (connect a Spawn context)".into(),
            ));
        }
        let update = flow_targets(asset, init.id, UPDATE).first().copied();
        let outputs = flow_targets(asset, update.unwrap_or(init.id), OUTPUT);
        if outputs.len() > 1 {
            out.errors
                .push((init.id, "only the first Output of a system is drawn".into()));
        }
        let output = outputs.first().copied();
        if output.is_none() {
            out.errors
                .push((init.id, "the system has no Output Particle Quad".into()));
        }

        // Spawn blocks (CPU): values or properties.
        let mut spawn = Vec::new();
        for s in &spawns {
            for b in asset.blocks_of(*s).filter(|b| b.enabled) {
                let Some(def) = blocks::find(&b.kind) else {
                    out.errors.push((*s, format!("unknown block `{}`", b.kind)));
                    continue;
                };
                let mut inputs = HashMap::new();
                for slot in &def.inputs {
                    let port = b.port(&slot.id);
                    let mut input = SpawnInput {
                        value: b.value(&slot.id).map_or(slot.default[0], |v| v.x),
                        property: None,
                    };
                    if let Some(e) = asset.graph.input_edge(*s, &port) {
                        match asset.graph.node(e.from) {
                            Some(n) if n.kind == "Property" => {
                                input.property = n.option("Property").map(str::to_owned);
                            }
                            _ => out.errors.push((
                                *s,
                                format!(
                                    "{} › {}: spawn inputs take values or properties",
                                    b.kind, slot.id
                                ),
                            )),
                        }
                    }
                    inputs.insert(slot.id.clone(), input);
                }
                spawn.push(SpawnBlock {
                    kind: b.kind.clone(),
                    id: b.id,
                    inputs,
                });
            }
        }

        // GPU hooks.
        let mut luts: Vec<[f32; 4]> = Vec::new();
        let mut hook =
            |ctx: Option<u32>, stage: Stage, errors: &mut Vec<(u32, String)>| -> String {
                let Some(ctx) = ctx else {
                    return String::new();
                };
                let mut e = Emit {
                    asset,
                    slots: &slots,
                    stage,
                    lines: Vec::new(),
                    memo: HashMap::new(),
                    visiting: HashSet::new(),
                    errors,
                    salt: 0,
                };
                let mut body = Vec::new();
                for b in asset.blocks_of(ctx).filter(|b| b.enabled) {
                    let Some(def) = blocks::find(&b.kind) else {
                        e.error(ctx, format!("unknown block `{}`", b.kind));
                        continue;
                    };
                    if !def.stages.contains(&stage) {
                        e.error(
                            ctx,
                            format!(
                                "{} can't go in {}",
                                b.kind,
                                asset.graph.node(ctx).map_or("", |n| n.kind.as_str())
                            ),
                        );
                        continue;
                    }
                    let BlockCode::Gpu(code) = def.code else {
                        continue;
                    };
                    let mut ins = Vec::new();
                    for slot in &def.inputs {
                        let port_slot = Slot {
                            id: b.port(&slot.id),
                            ..slot.clone()
                        };
                        let (x, t) = e.input(ctx, &port_slot, b.value(&slot.id));
                        ins.push(convert(&x, t, slot.ty).unwrap_or_else(|| Emit::zero(slot.ty)));
                    }
                    let lut = luts.len() as u32;
                    if def.gradient.is_some() {
                        luts.extend(bake_gradient(&b.gradient));
                    } else if def.curve.is_some() {
                        luts.extend(bake_curve(&b.curve));
                    }
                    let opts: Vec<(&'static str, String)> = def
                        .options
                        .iter()
                        .map(|o| (o.name, b.option(o.name).unwrap_or(o.default).to_owned()))
                        .collect();
                    let mut cx = BlockCx::new(ins, opts, format!("b{}_", b.id), lut, b.id * 1000);
                    let code = code(&mut cx);
                    body.append(&mut e.lines);
                    body.push(format!("// {}\n{code}", b.kind));
                }
                body.iter()
                    .flat_map(|l| l.lines())
                    .map(|l| format!("    {l}\n"))
                    .collect()
            };
        let init_body = hook(Some(init.id), Stage::Initialize, &mut out.errors);
        let update_body = hook(update, Stage::Update, &mut out.errors);
        let output_body = hook(output, Stage::Output, &mut out.errors);
        let hooks = format!(
            "{}{VFX_HELPERS}\nfn vfx_init(p: ptr<function, Particle>) {{\n    var q = *p;\n{init_body}    *p = q;\n}}\n\nfn vfx_update(p: ptr<function, Particle>) {{\n    var q = *p;\n{update_body}    *p = q;\n}}\n\nfn vfx_output(p: ptr<function, Particle>) {{\n    var q = *p;\n{output_body}    *p = q;\n}}\n",
            nodes::HELPERS
        );

        let setting = |id: Option<u32>, name: &str, default: &str| -> String {
            id.and_then(|i| asset.graph.node(i))
                .and_then(|n| n.option(name).map(str::to_owned))
                .unwrap_or_else(|| default.to_owned())
        };
        let capacity = setting(Some(init.id), "Capacity", "256")
            .trim()
            .parse::<u32>()
            .unwrap_or(256)
            .clamp(1, 4_000_000);
        let flipbook = {
            let f = setting(output, "Flipbook", "None");
            f.split_once(['x', 'X'])
                .and_then(|(a, b)| Some([a.trim().parse().ok()?, b.trim().parse().ok()?]))
                .unwrap_or([0, 0])
        };
        let texture = Some(setting(output, "Main Texture", "")).filter(|t| !t.is_empty());
        let mut program = VfxProgram {
            hash: 0,
            name: name.to_owned(),
            hooks,
            capacity,
            blend: match setting(output, "Blend Mode", "Alpha").as_str() {
                "Additive" => VfxBlend::Additive,
                "Opaque" => VfxBlend::Opaque,
                _ => VfxBlend::Alpha,
            },
            orient: if setting(output, "Orient", "") == "Along Velocity" {
                VfxOrient::AlongVelocity
            } else {
                VfxOrient::FaceCamera
            },
            flipbook,
            local_space: setting(Some(init.id), "Space", "Local") != "World",
            soft_dot: texture.is_none(),
            luts,
        };
        let mut h = std::collections::hash_map::DefaultHasher::new();
        program.hooks.hash(&mut h);
        program.capacity.hash(&mut h);
        program.blend.hash(&mut h);
        program.orient.hash(&mut h);
        program.flipbook.hash(&mut h);
        program.local_space.hash(&mut h);
        program.soft_dot.hash(&mut h);
        for l in &program.luts {
            for x in l {
                x.to_bits().hash(&mut h);
            }
        }
        program.hash = h.finish();
        out.systems.push(System {
            initialize: init.id,
            output,
            spawn,
            program: Arc::new(program),
            texture,
        });
    }
    if asset.graph.nodes.iter().all(|n| n.kind != INITIALIZE) {
        out.errors
            .push((0, "the graph has no Initialize Particle context".into()));
    }
    out
}
