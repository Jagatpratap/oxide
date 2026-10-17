//! Unity's Shader Graph node library: every node's ports, options and WGSL.
//!
//! Formulas follow Unity's Node Library pages ("Generated Code Example"), so a graph built
//! from the same nodes looks the same.
//! Reference: <https://docs.unity3d.com/Packages/com.unity.shadergraph@17.0/manual/Node-Library.html>

use std::sync::OnceLock;

/// A port's value type. `Dyn` is Unity's Dynamic Vector: it takes the width of what is
/// connected (the narrowest connected vector wider than one; a float broadcasts).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ty {
    F1,
    F2,
    F3,
    F4,
    Bool,
    Tex,
    Dyn,
    /// A VFX Graph flow connection between contexts (connects only to Flow).
    Flow,
}

impl Ty {
    pub fn vector(width: u32) -> Ty {
        match width {
            0 | 1 => Ty::F1,
            2 => Ty::F2,
            3 => Ty::F3,
            _ => Ty::F4,
        }
    }

    /// Vector width (Bool and Tex count as 1).
    pub fn width(self) -> u32 {
        match self {
            Ty::F2 => 2,
            Ty::F3 => 3,
            Ty::F4 => 4,
            _ => 1,
        }
    }

    pub fn wgsl(self) -> &'static str {
        match self {
            Ty::F1 | Ty::Dyn => "f32",
            Ty::F2 => "vec2f",
            Ty::F3 => "vec3f",
            Ty::F4 => "vec4f",
            Ty::Bool => "bool",
            Ty::Tex => "texture_2d<f32>",
            Ty::Flow => "",
        }
    }

    /// Unity's name for the type (port tooltips, Blackboard).
    pub fn label(self) -> &'static str {
        match self {
            Ty::F1 => "Float",
            Ty::F2 => "Vector 2",
            Ty::F3 => "Vector 3",
            Ty::F4 => "Vector 4",
            Ty::Bool => "Boolean",
            Ty::Tex => "Texture 2D",
            Ty::Flow => "Flow",
            Ty::Dyn => "Dynamic Vector",
        }
    }

    pub fn is_vector(self) -> bool {
        matches!(self, Ty::F1 | Ty::F2 | Ty::F3 | Ty::F4 | Ty::Dyn)
    }
}

/// What an unconnected input reads when it has no inline value (Unity's slot bindings:
/// the UV port of Sample Texture 2D reads UV0, Vertex Position reads the mesh position).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bind {
    None,
    Uv0,
    ObjectPos,
    ObjectNormal,
    ObjectTangent,
    WorldPos,
    WorldNormal,
    ViewDirWorld,
    TangentNormal,
}

impl Bind {
    pub fn label(self) -> Option<&'static str> {
        Some(match self {
            Bind::None => return None,
            Bind::Uv0 => "UV0",
            Bind::ObjectPos => "Object Space",
            Bind::ObjectNormal => "Object Space",
            Bind::ObjectTangent => "Object Space",
            Bind::WorldPos => "World Space",
            Bind::WorldNormal => "World Space",
            Bind::ViewDirWorld => "World Space",
            Bind::TangentNormal => "Tangent Space",
        })
    }

    pub fn expr(self) -> Option<&'static str> {
        Some(match self {
            Bind::None => return None,
            Bind::Uv0 => "i.uv0",
            Bind::ObjectPos => "i.object_pos",
            Bind::ObjectNormal => "i.object_normal",
            Bind::ObjectTangent => "sg_object_tangent(i)",
            Bind::WorldPos => "i.world_pos",
            Bind::WorldNormal => "i.world_normal",
            Bind::ViewDirWorld => "i.view_dir",
            Bind::TangentNormal => "vec3f(0.0, 0.0, 1.0)",
        })
    }
}

/// One port.
#[derive(Clone, Debug, PartialEq)]
pub struct Slot {
    /// Identity edges and inline values use.
    pub id: String,
    /// Shown on the node.
    pub label: String,
    pub ty: Ty,
    pub default: [f32; 4],
    pub bind: Bind,
    /// The inline value is an sRGB colour (colour picker; linearised when compiled).
    pub color: bool,
    /// A value field only (no port), like the Color node's colour.
    pub value_only: bool,
}

impl Slot {
    pub fn new(name: &str, ty: Ty) -> Self {
        Self {
            id: name.into(),
            label: name.into(),
            ty,
            default: [0.0; 4],
            bind: Bind::None,
            color: false,
            value_only: false,
        }
    }

    pub fn val(mut self, d: [f32; 4]) -> Self {
        self.default = d;
        self
    }

    pub fn f(mut self, x: f32) -> Self {
        self.default = [x; 4];
        self
    }

    pub fn bind(mut self, b: Bind) -> Self {
        self.bind = b;
        self
    }

    pub fn color(mut self) -> Self {
        self.color = true;
        self
    }

    pub fn value_only(mut self) -> Self {
        self.value_only = true;
        self
    }

    pub fn labelled(mut self, label: &str) -> Self {
        self.label = label.into();
        self
    }
}

/// A node setting: a dropdown (`choices`) or a text field (no choices).
#[derive(Clone, Debug, PartialEq)]
pub struct Opt {
    pub name: &'static str,
    pub choices: &'static [&'static str],
    pub default: &'static str,
    /// Multi-line text (Custom Function body).
    pub multiline: bool,
}

/// How a node writes its WGSL.
#[derive(Clone, Copy)]
pub enum Gen {
    /// One template per output: `{0}`.. are the inputs, `{T}` the resolved dynamic type.
    Tpl(&'static [&'static str]),
    /// Code for the nodes templates can't express.
    Fun(fn(&mut Call) -> Vec<String>),
    /// Compiled by the compiler itself (Property, Sub Graph, master stack, ...).
    Special,
}

/// A node type.
#[derive(Clone)]
pub struct NodeDef {
    pub kind: &'static str,
    /// Create Node menu category (`Math/Basic`).
    pub menu: &'static str,
    pub inputs: Vec<Slot>,
    pub outputs: Vec<Slot>,
    pub options: Vec<Opt>,
    pub code: Gen,
    /// Only valid in the fragment stage (derivatives, filtered sampling is converted).
    pub fragment_only: bool,
    /// Shows a preview by default.
    pub preview: bool,
}

/// What a node's code generator gets.
pub struct Call<'a> {
    /// Input expressions, converted to each input's type.
    pub ins: Vec<String>,
    /// The resolved dynamic vector type.
    pub dyn_ty: Ty,
    pub fragment: bool,
    /// The node's option values (defaults filled in).
    pub opts: Vec<(&'static str, String)>,
    /// Extra statements before the outputs (temporaries).
    pub lines: &'a mut Vec<String>,
    /// Unique prefix for temporaries.
    pub prefix: String,
    temps: u32,
}

impl<'a> Call<'a> {
    pub fn new(
        ins: Vec<String>,
        dyn_ty: Ty,
        fragment: bool,
        opts: Vec<(&'static str, String)>,
        lines: &'a mut Vec<String>,
        prefix: String,
    ) -> Self {
        Self {
            ins,
            dyn_ty,
            fragment,
            opts,
            lines,
            prefix,
            temps: 0,
        }
    }

    pub fn opt(&self, name: &str) -> &str {
        self.opts
            .iter()
            .find(|(n, _)| *n == name)
            .map_or("", |(_, v)| v.as_str())
    }

    /// Stores an expression in a temporary and returns its name.
    pub fn temp(&mut self, ty: &str, expr: String) -> String {
        let name = format!("{}t{}", self.prefix, self.temps);
        self.temps += 1;
        self.lines.push(format!("let {name}: {ty} = {expr};"));
        name
    }

    /// `dpdx`/`dpdy`/`fwidth` in the fragment stage; zero (or a small width) in the vertex
    /// stage, where derivatives don't exist.
    pub fn deriv(&self, f: &str, e: &str, ty: Ty) -> String {
        if self.fragment {
            format!("{f}({e})")
        } else if f == "fwidth" {
            format!("{}(0.001)", ty.wgsl())
        } else {
            format!("{}(0.0)", ty.wgsl())
        }
    }

    /// Samples a texture (`textureSample` in the fragment stage, mip 0 in the vertex stage).
    pub fn sample(&self, tex: &str, uv: &str) -> String {
        if self.fragment {
            format!("textureSample({tex}, mat_sampler, sample_uv({uv}))")
        } else {
            format!("textureSampleLevel({tex}, mat_sampler, sample_uv({uv}), 0.0)")
        }
    }
}

/// Formats a float as a WGSL literal.
pub fn lit(x: f32) -> String {
    if !x.is_finite() {
        return "0.0".into();
    }
    let s = format!("{x:?}");
    if s.contains('.') || s.contains('e') {
        s
    } else {
        format!("{s}.0")
    }
}

/// A constant of type `ty` from four values.
pub fn constant(ty: Ty, v: [f32; 4]) -> String {
    match ty {
        Ty::F1 | Ty::Dyn => lit(v[0]),
        Ty::F2 => format!("vec2f({}, {})", lit(v[0]), lit(v[1])),
        Ty::F3 => format!("vec3f({}, {}, {})", lit(v[0]), lit(v[1]), lit(v[2])),
        Ty::F4 => format!(
            "vec4f({}, {}, {}, {})",
            lit(v[0]),
            lit(v[1]),
            lit(v[2]),
            lit(v[3])
        ),
        Ty::Bool => (v[0] != 0.0).to_string(),
        Ty::Tex => "base_map".into(),
        Ty::Flow => String::new(),
    }
}

/// Converts an expression between port types (Unity's implicit conversions: truncate,
/// broadcast a float, pad with zeros).
pub fn convert(e: &str, from: Ty, to: Ty) -> Option<String> {
    use Ty::*;
    let to = if to == Dyn { F1 } else { to };
    let from = if from == Dyn { F1 } else { from };
    if from == to {
        return Some(e.to_owned());
    }
    Some(match (from, to) {
        (Tex, _) | (_, Tex) | (Flow, _) | (_, Flow) => return None,
        (Bool, t) => format!("{}(select(0.0, 1.0, {e}))", t.wgsl()),
        (F1, Bool) => format!("({e} != 0.0)"),
        (_, Bool) => format!("(({e}).x != 0.0)"),
        (F1, t) => format!("{}({e})", t.wgsl()),
        (f, F1) if f.width() > 1 => format!("({e}).x"),
        (f, t) if f.width() > t.width() => {
            format!("({e}).{}", &"xyzw"[..t.width() as usize])
        }
        (F2, F3) => format!("vec3f({e}, 0.0)"),
        (F2, F4) => format!("vec4f({e}, 0.0, 0.0)"),
        (F3, F4) => format!("vec4f({e}, 0.0)"),
        _ => return None,
    })
}

/// Preview colour of a value (Unity shows floats as grey, vectors as RGB).
pub fn to_color(e: &str, ty: Ty) -> String {
    match ty {
        Ty::F1 | Ty::Dyn => format!("vec4f(vec3f({e}), 1.0)"),
        Ty::F2 => format!("vec4f({e}, 0.0, 1.0)"),
        Ty::F3 => format!("vec4f({e}, 1.0)"),
        Ty::F4 => format!("vec4f(({e}).xyz, 1.0)"),
        Ty::Bool => format!("vec4f(vec3f(select(0.0, 1.0, {e})), 1.0)"),
        Ty::Tex => format!("textureSample({e}, mat_sampler, sample_uv(i.uv0))"),
        Ty::Flow => "vec4f(0.0, 0.0, 0.0, 1.0)".into(),
    }
}

/// Every node type.
pub fn library() -> &'static [NodeDef] {
    static LIB: OnceLock<Vec<NodeDef>> = OnceLock::new();
    LIB.get_or_init(build)
}

pub fn find(kind: &str) -> Option<&'static NodeDef> {
    library().iter().find(|d| d.kind == kind)
}

const SPACES: &[&str] = &["Object", "World", "View", "Tangent", "Absolute World"];
const DIR_SPACES: &[&str] = &["Object", "World", "View", "Tangent"];
const BOOL: &[&str] = &["False", "True"];

fn s(name: &str, ty: Ty) -> Slot {
    Slot::new(name, ty)
}

fn opt(name: &'static str, choices: &'static [&'static str], default: &'static str) -> Opt {
    Opt {
        name,
        choices,
        default,
        multiline: false,
    }
}

struct B(Vec<NodeDef>, &'static str);

impl B {
    fn node(
        &mut self,
        kind: &'static str,
        ins: Vec<Slot>,
        outs: Vec<Slot>,
        code: Gen,
    ) -> &mut NodeDef {
        self.0.push(NodeDef {
            kind,
            menu: self.1,
            inputs: ins,
            outputs: outs,
            options: Vec::new(),
            code,
            fragment_only: false,
            preview: true,
        });
        self.0.last_mut().expect("node")
    }

    /// A one-input dynamic vector node (`abs(In)`).
    fn unary(&mut self, kind: &'static str, tpl: &'static [&'static str]) {
        self.node(
            kind,
            vec![s("In", Ty::Dyn)],
            vec![s("Out", Ty::Dyn)],
            Gen::Tpl(tpl),
        );
    }

    /// A two-input dynamic vector node (`A + B`).
    fn binary(&mut self, kind: &'static str, b: f32, tpl: &'static [&'static str]) {
        self.node(
            kind,
            vec![s("A", Ty::Dyn), s("B", Ty::Dyn).f(b)],
            vec![s("Out", Ty::Dyn)],
            Gen::Tpl(tpl),
        );
    }
}

impl NodeDef {
    fn opts(&mut self, o: Vec<Opt>) -> &mut Self {
        self.options = o;
        self
    }

    fn frag(&mut self) -> &mut Self {
        self.fragment_only = true;
        self
    }

    fn no_preview(&mut self) -> &mut Self {
        self.preview = false;
        self
    }
}

/// A space conversion of a position or direction in world space to `space`.
fn from_world(e: &str, space: &str, dir: bool) -> String {
    let w = if dir { "0.0" } else { "1.0" };
    match space {
        "Object" => format!("sg_world_to_object(i, {e}, {w})"),
        "View" => format!("(sg_view_matrix() * vec4f({e}, {w})).xyz"),
        "Tangent" => format!("sg_world_to_tangent(i, {e})"),
        _ => e.to_owned(),
    }
}

fn to_world(e: &str, space: &str, dir: bool) -> String {
    let w = if dir { "0.0" } else { "1.0" };
    match space {
        "Object" => format!("(i.object_to_world * vec4f({e}, {w})).xyz"),
        "View" => format!("sg_view_to_world({e}, {w})"),
        "Tangent" => format!("sg_tangent_to_world(i, {e})"),
        _ => e.to_owned(),
    }
}

fn build() -> Vec<NodeDef> {
    use Ty::*;
    let mut b = B(Vec::new(), "");

    // ---- Master stack and graph plumbing (compiled specially).
    b.1 = "";
    b.node(
        "Vertex",
        vec![
            s("Position", F3).bind(Bind::ObjectPos),
            s("Normal", F3).bind(Bind::ObjectNormal),
            s("Tangent", F3).bind(Bind::ObjectTangent),
        ],
        vec![],
        Gen::Special,
    )
    .no_preview();
    b.node(
        "Fragment",
        vec![
            s("Base Color", F3).val([0.5, 0.5, 0.5, 1.0]).color(),
            s("Normal", F3)
                .labelled("Normal (Tangent Space)")
                .bind(Bind::TangentNormal),
            s("Metallic", F1),
            s("Smoothness", F1).f(0.5),
            s("Ambient Occlusion", F1).f(1.0),
            s("Emission", F3).color(),
            s("Alpha", F1).f(1.0),
            s("Alpha Clip Threshold", F1).f(0.5),
        ],
        vec![],
        Gen::Special,
    )
    .no_preview();
    b.node("SubGraphOutput", vec![], vec![], Gen::Special)
        .no_preview();
    b.1 = "Input/Property";
    b.node("Property", vec![], vec![], Gen::Special)
        .opts(vec![opt("Property", &[], "")])
        .no_preview();
    b.1 = "Utility";
    b.node("Sub Graph", vec![], vec![], Gen::Special)
        .opts(vec![opt("Sub Graph", &[], "")]);
    b.node("Custom Function", vec![], vec![], Gen::Special)
        .opts(vec![
            opt("Inputs", &[], "In:Vector 3"),
            opt("Outputs", &[], "Out:Vector 3"),
            Opt {
                name: "Body",
                choices: &[],
                default: "Out = In;",
                multiline: true,
            },
        ]);
    b.1 = "Utility/Keyword";
    b.node(
        "Keyword",
        vec![s("On", Dyn).f(1.0), s("Off", Dyn)],
        vec![s("Out", Dyn)],
        Gen::Special,
    )
    .opts(vec![opt("Keyword", &[], "")]);

    // ---- Input/Basic
    b.1 = "Input/Basic";
    b.node(
        "Float",
        vec![s("X", F1)],
        vec![s("Out", F1)],
        Gen::Tpl(&["{0}"]),
    );
    b.node(
        "Vector 2",
        vec![s("X", F1), s("Y", F1)],
        vec![s("Out", F2)],
        Gen::Tpl(&["vec2f({0}, {1})"]),
    );
    b.node(
        "Vector 3",
        vec![s("X", F1), s("Y", F1), s("Z", F1)],
        vec![s("Out", F3)],
        Gen::Tpl(&["vec3f({0}, {1}, {2})"]),
    );
    b.node(
        "Vector 4",
        vec![s("X", F1), s("Y", F1), s("Z", F1), s("W", F1)],
        vec![s("Out", F4)],
        Gen::Tpl(&["vec4f({0}, {1}, {2}, {3})"]),
    );
    b.node(
        "Color",
        vec![s("Color", F4).val([1.0; 4]).color().value_only()],
        vec![s("Out", F4)],
        Gen::Tpl(&["{0}"]),
    );
    b.node(
        "Integer",
        vec![s("Value", F1).value_only()],
        vec![s("Out", F1)],
        Gen::Tpl(&["floor({0})"]),
    );
    b.node(
        "Slider",
        vec![s("Value", F1).value_only()],
        vec![s("Out", F1)],
        Gen::Tpl(&["{0}"]),
    )
    .opts(vec![opt("Min", &[], "0"), opt("Max", &[], "1")]);
    b.node(
        "Boolean",
        vec![],
        vec![s("Out", Bool)],
        Gen::Fun(|c| vec![(c.opt("Value") == "True").to_string()]),
    )
    .opts(vec![opt("Value", BOOL, "False")]);
    b.node(
        "Constant",
        vec![],
        vec![s("Out", F1)],
        Gen::Fun(|c| {
            vec![
                match c.opt("Constant") {
                    "TAU" => "6.28318530718",
                    "PHI" => "1.618034",
                    "E" => "2.718282",
                    "SQRT2" => "1.414214",
                    _ => "3.1415926",
                }
                .into(),
            ]
        }),
    )
    .opts(vec![opt(
        "Constant",
        &["PI", "TAU", "PHI", "E", "SQRT2"],
        "PI",
    )]);
    b.node(
        "Time",
        vec![],
        vec![
            s("Time", F1),
            s("Sine Time", F1),
            s("Cosine Time", F1),
            s("Delta Time", F1),
            s("Smooth Delta", F1),
        ],
        Gen::Tpl(&[
            "sg_time()",
            "sin(sg_time())",
            "cos(sg_time())",
            "(1.0 / 60.0)",
            "(1.0 / 60.0)",
        ]),
    );

    // ---- Input/Geometry
    b.1 = "Input/Geometry";
    b.node(
        "Position",
        vec![],
        vec![s("Out", F3)],
        Gen::Fun(|c| {
            vec![match c.opt("Space") {
                "Object" => "i.object_pos".into(),
                s => from_world("i.world_pos", s, false),
            }]
        }),
    )
    .opts(vec![opt("Space", SPACES, "World")]);
    b.node(
        "Normal Vector",
        vec![],
        vec![s("Out", F3)],
        Gen::Fun(|c| {
            vec![match c.opt("Space") {
                "Object" => "i.object_normal".into(),
                "Tangent" => "vec3f(0.0, 0.0, 1.0)".into(),
                s => format!("normalize({})", from_world("i.world_normal", s, true)),
            }]
        }),
    )
    .opts(vec![opt("Space", DIR_SPACES, "World")]);
    b.node(
        "Tangent Vector",
        vec![],
        vec![s("Out", F3)],
        Gen::Fun(|c| {
            vec![match c.opt("Space") {
                "Object" => "sg_object_tangent(i)".into(),
                "Tangent" => "vec3f(1.0, 0.0, 0.0)".into(),
                s => from_world("i.world_tangent", s, true),
            }]
        }),
    )
    .opts(vec![opt("Space", DIR_SPACES, "World")]);
    b.node(
        "Bitangent Vector",
        vec![],
        vec![s("Out", F3)],
        Gen::Fun(|c| {
            vec![match c.opt("Space") {
                "Tangent" => "vec3f(0.0, 1.0, 0.0)".into(),
                s => from_world("i.world_bitangent", s, true),
            }]
        }),
    )
    .opts(vec![opt("Space", DIR_SPACES, "World")]);
    b.node(
        "View Direction",
        vec![],
        vec![s("Out", F3)],
        Gen::Fun(|c| {
            vec![format!(
                "normalize({})",
                from_world("i.view_dir", c.opt("Space"), true)
            )]
        }),
    )
    .opts(vec![opt("Space", DIR_SPACES, "World")]);
    b.node(
        "View Vector",
        vec![],
        vec![s("Out", F3)],
        Gen::Fun(|c| {
            vec![from_world(
                "(sg_camera_pos() - i.world_pos)",
                c.opt("Space"),
                true,
            )]
        }),
    )
    .opts(vec![opt("Space", DIR_SPACES, "World")]);
    b.node(
        "UV",
        vec![],
        vec![s("Out", F4)],
        Gen::Fun(|c| {
            vec![match c.opt("Channel") {
                "UV1" => "vec4f(i.uv1, 0.0, 0.0)".into(),
                _ => "vec4f(i.uv0, 0.0, 0.0)".into(),
            }]
        }),
    )
    .opts(vec![opt("Channel", &["UV0", "UV1"], "UV0")]);
    b.node(
        "Vertex Color",
        vec![],
        vec![s("Out", F4)],
        Gen::Tpl(&["i.vertex_color"]),
    );
    b.node(
        "Screen Position",
        vec![],
        vec![s("Out", F4)],
        Gen::Fun(|c| {
            vec![match c.opt("Mode") {
                "Center" => "vec4f(i.screen_pos.xy * 2.0 - 1.0, 0.0, 0.0)".into(),
                "Tiled" => "vec4f(fract(i.screen_pos.xy * 2.0 - 1.0), 0.0, 0.0)".into(),
                _ => "i.screen_pos".into(),
            }]
        }),
    )
    .opts(vec![opt(
        "Mode",
        &["Default", "Raw", "Center", "Tiled"],
        "Default",
    )]);
    b.node("Vertex ID", vec![], vec![s("Out", F1)], Gen::Tpl(&["0.0"]));

    // ---- Input/Scene
    b.1 = "Input/Scene";
    b.node(
        "Camera",
        vec![],
        vec![s("Position", F3), s("Direction", F3)],
        Gen::Tpl(&[
            "sg_camera_pos()",
            "normalize(vec3f(sg_view_matrix()[0].z, sg_view_matrix()[1].z, sg_view_matrix()[2].z))",
        ]),
    );
    b.node(
        "Object",
        vec![],
        vec![s("Position", F3), s("Scale", F3)],
        Gen::Tpl(&[
            "i.object_to_world[3].xyz",
            "vec3f(length(i.object_to_world[0].xyz), length(i.object_to_world[1].xyz), length(i.object_to_world[2].xyz))",
        ]),
    );
    b.node(
        "Is Front Face",
        vec![],
        vec![s("Out", Bool)],
        Gen::Tpl(&["i.front"]),
    )
    .frag();

    // ---- Input/Texture
    b.1 = "Input/Texture";
    let sample_outs = || {
        vec![
            s("RGBA", F4),
            s("R", F1),
            s("G", F1),
            s("B", F1),
            s("A", F1),
        ]
    };
    fn sample_gen(c: &mut Call, lod: Option<String>) -> Vec<String> {
        let (tex, uv) = (c.ins[0].clone(), c.ins[1].clone());
        let raw = match lod {
            Some(l) => format!("textureSampleLevel({tex}, mat_sampler, sample_uv({uv}), {l})"),
            None => c.sample(&tex, &uv),
        };
        let v = c.temp("vec4f", raw);
        let v = if c.opt("Type") == "Normal" {
            c.temp("vec4f", format!("vec4f(sg_unpack_normal({v}), 1.0)"))
        } else {
            v
        };
        vec![
            v.clone(),
            format!("{v}.x"),
            format!("{v}.y"),
            format!("{v}.z"),
            format!("{v}.w"),
        ]
    }
    b.node(
        "Sample Texture 2D",
        vec![s("Texture", Tex), s("UV", F2).bind(Bind::Uv0)],
        sample_outs(),
        Gen::Fun(|c| sample_gen(c, None)),
    )
    .opts(vec![
        opt("Type", &["Default", "Normal"], "Default"),
        opt("Texture", &[], ""),
    ]);
    b.node(
        "Sample Texture 2D LOD",
        vec![s("Texture", Tex), s("UV", F2).bind(Bind::Uv0), s("LOD", F1)],
        sample_outs(),
        Gen::Fun(|c| {
            let l = c.ins[2].clone();
            sample_gen(c, Some(l))
        }),
    )
    .opts(vec![
        opt("Type", &["Default", "Normal"], "Default"),
        opt("Texture", &[], ""),
    ]);
    b.node(
        "Texture 2D Asset",
        vec![],
        vec![s("Out", Tex)],
        Gen::Special,
    )
    .opts(vec![opt("Texture", &[], "")])
    .no_preview();
    b.node(
        "Texel Size",
        vec![s("Texture", Tex)],
        vec![s("Width", F1), s("Height", F1)],
        Gen::Tpl(&[
            "f32(textureDimensions({0}).x)",
            "f32(textureDimensions({0}).y)",
        ]),
    );
    b.node(
        "Triplanar",
        vec![
            s("Texture", Tex),
            s("Position", F3).bind(Bind::WorldPos),
            s("Normal", F3).bind(Bind::WorldNormal),
            s("Tile", F1).f(1.0),
            s("Blend", F1).f(1.0),
        ],
        vec![s("Out", F4)],
        Gen::Fun(|c| {
            let (t, p, n, tile, bl) = (
                c.ins[0].clone(),
                c.ins[1].clone(),
                c.ins[2].clone(),
                c.ins[3].clone(),
                c.ins[4].clone(),
            );
            let w = c.temp("vec3f", format!("pow(abs({n}), vec3f({bl}))"));
            let w = c.temp("vec3f", format!("{w} / max({w}.x + {w}.y + {w}.z, 1e-5)"));
            let x = c.sample(&t, &format!("({p}).zy * {tile}"));
            let y = c.sample(&t, &format!("({p}).xz * {tile}"));
            let z = c.sample(&t, &format!("({p}).xy * {tile}"));
            vec![format!("({x} * {w}.x + {y} * {w}.y + {z} * {w}.z)")]
        }),
    );

    // ---- Math/Basic
    b.1 = "Math/Basic";
    b.binary("Add", 0.0, &["({0} + {1})"]);
    b.binary("Subtract", 1.0, &["({0} - {1})"]);
    b.binary("Multiply", 2.0, &["({0} * {1})"]);
    b.binary("Divide", 2.0, &["({0} / {1})"]);
    b.binary("Power", 2.0, &["pow({0}, {1})"]);
    b.unary("Square Root", &["sqrt({0})"]);

    // ---- Math/Advanced
    b.1 = "Math/Advanced";
    b.unary("Absolute", &["abs({0})"]);
    b.node(
        "Exponential",
        vec![s("In", Dyn)],
        vec![s("Out", Dyn)],
        Gen::Fun(|c| {
            vec![match c.opt("Base") {
                "BaseTwo" => format!("exp2({})", c.ins[0]),
                _ => format!("exp({})", c.ins[0]),
            }]
        }),
    )
    .opts(vec![opt("Base", &["BaseE", "BaseTwo"], "BaseE")]);
    b.node(
        "Length",
        vec![s("In", Dyn)],
        vec![s("Out", F1)],
        Gen::Tpl(&["length({0})"]),
    );
    b.node(
        "Log",
        vec![s("In", Dyn).f(1.0)],
        vec![s("Out", Dyn)],
        Gen::Fun(|c| {
            vec![match c.opt("Base") {
                "BaseTwo" => format!("log2({})", c.ins[0]),
                "BaseTen" => format!("(log({}) / 2.302585)", c.ins[0]),
                _ => format!("log({})", c.ins[0]),
            }]
        }),
    )
    .opts(vec![opt("Base", &["BaseE", "BaseTwo", "BaseTen"], "BaseE")]);
    b.binary("Modulo", 1.0, &["({0} % {1})"]);
    b.unary("Negate", &["(-{0})"]);
    b.node(
        "Normalize",
        vec![s("In", Dyn)],
        vec![s("Out", Dyn)],
        Gen::Fun(|c| {
            vec![if c.dyn_ty.width() == 1 {
                format!("sign({})", c.ins[0])
            } else {
                format!("normalize({})", c.ins[0])
            }]
        }),
    );
    b.node(
        "Posterize",
        vec![s("In", Dyn), s("Steps", Dyn).f(4.0)],
        vec![s("Out", Dyn)],
        Gen::Tpl(&["(floor({0} / (1.0 / {1})) * (1.0 / {1}))"]),
    );
    b.unary("Reciprocal", &["(1.0 / {0})"]);
    b.unary("Reciprocal Square Root", &["inverseSqrt({0})"]);

    // ---- Math/Range
    b.1 = "Math/Range";
    b.node(
        "Clamp",
        vec![s("In", Dyn), s("Min", Dyn), s("Max", Dyn).f(1.0)],
        vec![s("Out", Dyn)],
        Gen::Tpl(&["clamp({0}, {1}, {2})"]),
    );
    b.unary("Fraction", &["fract({0})"]);
    b.binary("Maximum", 0.0, &["max({0}, {1})"]);
    b.binary("Minimum", 0.0, &["min({0}, {1})"]);
    b.unary("One Minus", &["(1.0 - {0})"]);
    b.node(
        "Random Range",
        vec![s("Seed", F2), s("Min", F1), s("Max", F1).f(1.0)],
        vec![s("Out", F1)],
        Gen::Tpl(&["mix({1}, {2}, fract(sin(dot({0}, vec2f(12.9898, 78.233))) * 43758.5453))"]),
    );
    b.node(
        "Remap",
        vec![
            s("In", Dyn),
            s("In Min Max", F2).val([-1.0, 1.0, 0.0, 0.0]),
            s("Out Min Max", F2).val([0.0, 1.0, 0.0, 0.0]),
        ],
        vec![s("Out", Dyn)],
        Gen::Tpl(&["({2}.x + ({0} - {1}.x) * ({2}.y - {2}.x) / ({1}.y - {1}.x))"]),
    );
    b.unary("Saturate", &["saturate({0})"]);

    // ---- Math/Round
    b.1 = "Math/Round";
    b.unary("Ceiling", &["ceil({0})"]);
    b.unary("Floor", &["floor({0})"]);
    b.unary("Round", &["round({0})"]);
    b.unary("Sign", &["sign({0})"]);
    b.node(
        "Step",
        vec![s("Edge", Dyn).f(1.0), s("In", Dyn)],
        vec![s("Out", Dyn)],
        Gen::Tpl(&["step({0}, {1})"]),
    );
    b.unary("Truncate", &["trunc({0})"]);

    // ---- Math/Trigonometry
    b.1 = "Math/Trigonometry";
    b.unary("Sine", &["sin({0})"]);
    b.unary("Cosine", &["cos({0})"]);
    b.unary("Tangent", &["tan({0})"]);
    b.unary("Arcsine", &["asin({0})"]);
    b.unary("Arccosine", &["acos({0})"]);
    b.unary("Arctangent", &["atan({0})"]);
    b.binary("Arctangent2", 0.0, &["atan2({0}, {1})"]);
    b.unary("Hyperbolic Sine", &["sinh({0})"]);
    b.unary("Hyperbolic Cosine", &["cosh({0})"]);
    b.unary("Hyperbolic Tangent", &["tanh({0})"]);
    b.unary("Degrees To Radians", &["radians({0})"]);
    b.unary("Radians To Degrees", &["degrees({0})"]);

    // ---- Math/Vector
    b.1 = "Math/Vector";
    b.node(
        "Cross Product",
        vec![s("A", F3), s("B", F3).val([0.0, 1.0, 0.0, 0.0])],
        vec![s("Out", F3)],
        Gen::Tpl(&["cross({0}, {1})"]),
    );
    b.node(
        "Distance",
        vec![s("A", Dyn), s("B", Dyn)],
        vec![s("Out", F1)],
        Gen::Tpl(&["distance({0}, {1})"]),
    );
    b.node(
        "Dot Product",
        vec![s("A", Dyn), s("B", Dyn).val([0.0, 1.0, 0.0, 0.0])],
        vec![s("Out", F1)],
        Gen::Fun(|c| {
            vec![if c.dyn_ty.width() == 1 {
                format!("({} * {})", c.ins[0], c.ins[1])
            } else {
                format!("dot({}, {})", c.ins[0], c.ins[1])
            }]
        }),
    );
    b.node(
        "Reflection",
        vec![s("In", Dyn), s("Normal", Dyn).val([0.0, 1.0, 0.0, 0.0])],
        vec![s("Out", Dyn)],
        Gen::Fun(|c| {
            let (a, n) = (&c.ins[0], &c.ins[1]);
            vec![format!("({a} - 2.0 * {n} * ({}))", dot(c.dyn_ty, n, a))]
        }),
    );
    b.node(
        "Projection",
        vec![s("A", Dyn), s("B", Dyn).val([0.0, 1.0, 0.0, 0.0])],
        vec![s("Out", Dyn)],
        Gen::Fun(|c| {
            let (a, bb) = (&c.ins[0], &c.ins[1]);
            vec![format!(
                "({bb} * ({}) / max({}, 1e-8))",
                dot(c.dyn_ty, a, bb),
                dot(c.dyn_ty, bb, bb)
            )]
        }),
    );
    b.node(
        "Rejection",
        vec![s("A", Dyn), s("B", Dyn).val([0.0, 1.0, 0.0, 0.0])],
        vec![s("Out", Dyn)],
        Gen::Fun(|c| {
            let (a, bb) = (&c.ins[0], &c.ins[1]);
            vec![format!(
                "({a} - {bb} * ({}) / max({}, 1e-8))",
                dot(c.dyn_ty, a, bb),
                dot(c.dyn_ty, bb, bb)
            )]
        }),
    );
    b.node(
        "Rotate About Axis",
        vec![
            s("In", F3),
            s("Axis", F3).val([0.0, 1.0, 0.0, 0.0]),
            s("Rotation", F1),
        ],
        vec![s("Out", F3)],
        Gen::Fun(|c| {
            let r = if c.opt("Unit") == "Degrees" {
                format!("radians({})", c.ins[2])
            } else {
                c.ins[2].clone()
            };
            vec![format!(
                "sg_rotate_about_axis({}, {}, {r})",
                c.ins[0], c.ins[1]
            )]
        }),
    )
    .opts(vec![opt("Unit", &["Radians", "Degrees"], "Radians")]);
    b.node(
        "Transform",
        vec![s("In", F3)],
        vec![s("Out", F3)],
        Gen::Fun(|c| {
            let dir = c.opt("Type") != "Position";
            let w = to_world(&c.ins[0], c.opt("From"), dir);
            let mut out = from_world(&w, c.opt("To"), dir);
            if c.opt("Type") == "Normal" {
                out = format!("normalize({out})");
            }
            vec![out]
        }),
    )
    .opts(vec![
        opt("From", SPACES, "Object"),
        opt("To", SPACES, "World"),
        opt("Type", &["Position", "Direction", "Normal"], "Position"),
    ]);

    // ---- Math/Interpolation
    b.1 = "Math/Interpolation";
    b.node(
        "Lerp",
        vec![s("A", Dyn), s("B", Dyn).f(1.0), s("T", Dyn)],
        vec![s("Out", Dyn)],
        Gen::Tpl(&["mix({0}, {1}, {2})"]),
    );
    b.node(
        "Inverse Lerp",
        vec![s("A", Dyn), s("B", Dyn).f(1.0), s("T", Dyn)],
        vec![s("Out", Dyn)],
        Gen::Tpl(&["(({2} - {0}) / ({1} - {0}))"]),
    );
    b.node(
        "Smoothstep",
        vec![s("Edge1", Dyn), s("Edge2", Dyn).f(1.0), s("In", Dyn)],
        vec![s("Out", Dyn)],
        Gen::Tpl(&["smoothstep({0}, {1}, {2})"]),
    );

    // ---- Math/Derivative
    b.1 = "Math/Derivative";
    for (kind, f) in [("DDX", "dpdx"), ("DDY", "dpdy"), ("DDXY", "fwidth")] {
        b.node(
            kind,
            vec![s("In", Dyn)],
            vec![s("Out", Dyn)],
            Gen::Fun(match f {
                "dpdx" => |c: &mut Call| vec![c.deriv("dpdx", &c.ins[0], c.dyn_ty)],
                "dpdy" => |c: &mut Call| vec![c.deriv("dpdy", &c.ins[0], c.dyn_ty)],
                _ => |c: &mut Call| vec![c.deriv("fwidth", &c.ins[0], c.dyn_ty)],
            }),
        )
        .frag();
    }

    // ---- Channel
    b.1 = "Channel";
    b.node(
        "Split",
        vec![s("In", Dyn)],
        vec![s("R", F1), s("G", F1), s("B", F1), s("A", F1)],
        Gen::Fun(|c| {
            let w = c.dyn_ty.width() as usize;
            let v = c.ins[0].clone();
            (0..4)
                .map(|k| {
                    if w == 1 {
                        if k == 0 { v.clone() } else { "0.0".into() }
                    } else if k < w {
                        format!("({v}).{}", &"xyzw"[k..k + 1])
                    } else {
                        "0.0".into()
                    }
                })
                .collect()
        }),
    );
    b.node(
        "Combine",
        vec![s("R", F1), s("G", F1), s("B", F1), s("A", F1)],
        vec![s("RGBA", F4), s("RGB", F3), s("RG", F2)],
        Gen::Tpl(&[
            "vec4f({0}, {1}, {2}, {3})",
            "vec3f({0}, {1}, {2})",
            "vec2f({0}, {1})",
        ]),
    );
    b.node(
        "Swizzle",
        vec![s("In", F4)],
        vec![s("Out", Dyn)],
        Gen::Special,
    )
    .opts(vec![opt("Mask", &[], "xyzw")]);
    b.node(
        "Flip",
        vec![s("In", Dyn)],
        vec![s("Out", Dyn)],
        Gen::Fun(|c| {
            let w = c.dyn_ty.width() as usize;
            let f: Vec<String> = ["Red", "Green", "Blue", "Alpha"][..w]
                .iter()
                .map(|n| if c.opt(n) == "True" { "-1.0" } else { "1.0" }.to_string())
                .collect();
            vec![format!(
                "({} * {}({}))",
                c.ins[0],
                c.dyn_ty.wgsl(),
                f.join(", ")
            )]
        }),
    )
    .opts(vec![
        opt("Red", BOOL, "False"),
        opt("Green", BOOL, "False"),
        opt("Blue", BOOL, "False"),
        opt("Alpha", BOOL, "False"),
    ]);

    // ---- UV
    b.1 = "UV";
    b.node(
        "Tiling And Offset",
        vec![
            s("UV", F2).bind(Bind::Uv0),
            s("Tiling", F2).f(1.0),
            s("Offset", F2),
        ],
        vec![s("Out", F2)],
        Gen::Tpl(&["({0} * {1} + {2})"]),
    );
    b.node(
        "Rotate",
        vec![
            s("UV", F2).bind(Bind::Uv0),
            s("Center", F2).f(0.5),
            s("Rotation", F1),
        ],
        vec![s("Out", F2)],
        Gen::Fun(|c| {
            let r = if c.opt("Unit") == "Degrees" {
                format!("radians({})", c.ins[2])
            } else {
                c.ins[2].clone()
            };
            vec![format!("sg_rotate_uv({}, {}, {r})", c.ins[0], c.ins[1])]
        }),
    )
    .opts(vec![opt("Unit", &["Radians", "Degrees"], "Radians")]);
    b.node(
        "Polar Coordinates",
        vec![
            s("UV", F2).bind(Bind::Uv0),
            s("Center", F2).f(0.5),
            s("Radial Scale", F1).f(1.0),
            s("Length Scale", F1).f(1.0),
        ],
        vec![s("Out", F2)],
        Gen::Tpl(&["sg_polar({0}, {1}, {2}, {3})"]),
    );
    b.node(
        "Twirl",
        vec![
            s("UV", F2).bind(Bind::Uv0),
            s("Center", F2).f(0.5),
            s("Strength", F1).f(10.0),
            s("Offset", F2),
        ],
        vec![s("Out", F2)],
        Gen::Tpl(&["sg_twirl({0}, {1}, {2}, {3})"]),
    );
    b.node(
        "Spherize",
        vec![
            s("UV", F2).bind(Bind::Uv0),
            s("Center", F2).f(0.5),
            s("Strength", F2).f(10.0),
            s("Offset", F2),
        ],
        vec![s("Out", F2)],
        Gen::Tpl(&["sg_spherize({0}, {1}, {2}, {3})"]),
    );
    b.node(
        "Radial Shear",
        vec![
            s("UV", F2).bind(Bind::Uv0),
            s("Center", F2).f(0.5),
            s("Strength", F2).f(10.0),
            s("Offset", F2),
        ],
        vec![s("Out", F2)],
        Gen::Tpl(&["sg_radial_shear({0}, {1}, {2}, {3})"]),
    );
    b.node(
        "Flipbook",
        vec![
            s("UV", F2).bind(Bind::Uv0),
            s("Width", F1).f(1.0),
            s("Height", F1).f(1.0),
            s("Tile", F1),
        ],
        vec![s("Out", F2)],
        Gen::Fun(|c| {
            let inv = |n: &str| if c.opt(n) == "True" { "1.0" } else { "0.0" };
            vec![format!(
                "sg_flipbook({}, {}, {}, {}, vec2f({}, {}))",
                c.ins[0],
                c.ins[1],
                c.ins[2],
                c.ins[3],
                inv("Invert X"),
                inv("Invert Y")
            )]
        }),
    )
    .opts(vec![
        opt("Invert X", BOOL, "False"),
        opt("Invert Y", BOOL, "True"),
    ]);

    // ---- Procedural
    b.1 = "Procedural/Noise";
    b.node(
        "Simple Noise",
        vec![s("UV", F2).bind(Bind::Uv0), s("Scale", F1).f(500.0)],
        vec![s("Out", F1)],
        Gen::Tpl(&["sg_simple_noise({0}, {1})"]),
    );
    b.node(
        "Gradient Noise",
        vec![s("UV", F2).bind(Bind::Uv0), s("Scale", F1).f(10.0)],
        vec![s("Out", F1)],
        Gen::Tpl(&["(sg_gradient_noise({0} * {1}) + 0.5)"]),
    );
    b.node(
        "Voronoi",
        vec![
            s("UV", F2).bind(Bind::Uv0),
            s("Angle Offset", F1).f(2.0),
            s("Cell Density", F1).f(5.0),
        ],
        vec![s("Out", F1), s("Cells", F1)],
        Gen::Fun(|c| {
            let v = c.temp(
                "vec2f",
                format!("sg_voronoi({}, {}, {})", c.ins[0], c.ins[1], c.ins[2]),
            );
            vec![format!("{v}.x"), format!("{v}.y")]
        }),
    );
    b.1 = "Procedural";
    b.node(
        "Checkerboard",
        vec![
            s("UV", F2).bind(Bind::Uv0),
            s("Color A", F3).val([0.2, 0.2, 0.2, 1.0]).color(),
            s("Color B", F3).val([0.7, 0.7, 0.7, 1.0]).color(),
            s("Frequency", F2).f(1.0),
        ],
        vec![s("Out", F3)],
        Gen::Fun(|c| {
            let uv = c.temp("vec2f", format!("({} + 0.5) * {}", c.ins[0], c.ins[3]));
            let dx = c.deriv("dpdx", &uv, Ty::F2);
            let dy = c.deriv("dpdy", &uv, Ty::F2);
            vec![format!(
                "sg_checkerboard({uv}, {dx}, {dy}, {}, {})",
                c.ins[1], c.ins[2]
            )]
        }),
    );
    b.1 = "Procedural/Shapes";
    b.node(
        "Ellipse",
        vec![
            s("UV", F2).bind(Bind::Uv0),
            s("Width", F1).f(0.5),
            s("Height", F1).f(0.5),
        ],
        vec![s("Out", F1)],
        Gen::Fun(|c| {
            let d = c.temp(
                "f32",
                format!(
                    "length(({} * 2.0 - 1.0) / vec2f({}, {}))",
                    c.ins[0], c.ins[1], c.ins[2]
                ),
            );
            let fw = c.deriv("fwidth", &d, Ty::F1);
            vec![format!("saturate((1.0 - {d}) / max({fw}, 1e-5))")]
        }),
    );
    b.node(
        "Rectangle",
        vec![
            s("UV", F2).bind(Bind::Uv0),
            s("Width", F1).f(0.5),
            s("Height", F1).f(0.5),
        ],
        vec![s("Out", F1)],
        Gen::Fun(|c| {
            let d = c.temp(
                "vec2f",
                format!(
                    "abs({} * 2.0 - 1.0) - vec2f({}, {})",
                    c.ins[0], c.ins[1], c.ins[2]
                ),
            );
            let fw = c.deriv("fwidth", &d, Ty::F2);
            let e = c.temp("vec2f", format!("1.0 - {d} / max({fw}, vec2f(1e-5))"));
            vec![format!("saturate(min({e}.x, {e}.y))")]
        }),
    );
    b.node(
        "Rounded Rectangle",
        vec![
            s("UV", F2).bind(Bind::Uv0),
            s("Width", F1).f(0.5),
            s("Height", F1).f(0.5),
            s("Radius", F1).f(0.1),
        ],
        vec![s("Out", F1)],
        Gen::Fun(|c| {
            let (uv, w, h, r) = (
                c.ins[0].clone(),
                c.ins[1].clone(),
                c.ins[2].clone(),
                c.ins[3].clone(),
            );
            let r = c.temp(
                "f32",
                format!("max(min(min(abs({r} * 2.0), abs({w})), abs({h})), 1e-5)"),
            );
            let p = c.temp(
                "vec2f",
                format!("abs({uv} * 2.0 - 1.0) - vec2f({w}, {h}) + {r}"),
            );
            let d = c.temp("f32", format!("length(max(vec2f(0.0), {p})) / {r}"));
            let fw = c.deriv("fwidth", &d, Ty::F1);
            vec![format!("saturate((1.0 - {d}) / max({fw}, 1e-5))")]
        }),
    );
    b.node(
        "Polygon",
        vec![
            s("UV", F2).bind(Bind::Uv0),
            s("Sides", F1).f(6.0),
            s("Width", F1).f(0.5),
            s("Height", F1).f(0.5),
        ],
        vec![s("Out", F1)],
        Gen::Fun(|c| {
            let d = c.temp(
                "f32",
                format!(
                    "sg_polygon_distance({}, {}, {}, {})",
                    c.ins[0], c.ins[1], c.ins[2], c.ins[3]
                ),
            );
            let fw = c.deriv("fwidth", &d, Ty::F1);
            vec![format!("saturate((1.0 - {d}) / max({fw}, 1e-5))")]
        }),
    );

    // ---- Artistic
    b.1 = "Artistic/Normal";
    b.node(
        "Normal Strength",
        vec![
            s("In", F3).val([0.0, 0.0, 1.0, 0.0]),
            s("Strength", F1).f(1.0),
        ],
        vec![s("Out", F3)],
        Gen::Tpl(&["vec3f(({0}).xy * {1}, mix(1.0, ({0}).z, saturate({1})))"]),
    );
    b.node(
        "Normal Blend",
        vec![
            s("A", F3).val([0.0, 0.0, 1.0, 0.0]),
            s("B", F3).val([0.0, 0.0, 1.0, 0.0]),
        ],
        vec![s("Out", F3)],
        Gen::Fun(|c| {
            let (a, bb) = (&c.ins[0], &c.ins[1]);
            vec![if c.opt("Mode") == "Reoriented" {
                format!("sg_normal_blend_reoriented({a}, {bb})")
            } else {
                format!("normalize(vec3f(({a}).xy + ({bb}).xy, ({a}).z * ({bb}).z))")
            }]
        }),
    )
    .opts(vec![opt("Mode", &["Default", "Reoriented"], "Default")]);
    b.node(
        "Normal From Height",
        vec![s("In", F1), s("Strength", F1).f(0.01)],
        vec![s("Out", F3)],
        Gen::Fun(|c| {
            let dp = (
                c.deriv("dpdx", "i.world_pos", Ty::F3),
                c.deriv("dpdy", "i.world_pos", Ty::F3),
                c.deriv("dpdx", &c.ins[0], Ty::F1),
                c.deriv("dpdy", &c.ins[0], Ty::F1),
            );
            let n = format!(
                "sg_normal_from_height(i, {}, {}, {}, {}, {})",
                dp.0, dp.1, dp.2, dp.3, c.ins[1]
            );
            vec![if c.opt("Output Space") == "World" {
                n
            } else {
                format!("sg_world_to_tangent(i, {n})")
            }]
        }),
    )
    .opts(vec![opt("Output Space", &["Tangent", "World"], "Tangent")])
    .frag();
    b.node(
        "Normal Unpack",
        vec![s("In", F4).val([0.5, 0.5, 1.0, 1.0])],
        vec![s("Out", F3)],
        Gen::Tpl(&["sg_unpack_normal({0})"]),
    );
    b.node(
        "Normal Reconstruct Z",
        vec![s("In", F2)],
        vec![s("Out", F3)],
        Gen::Tpl(&["normalize(vec3f({0}, sqrt(1.0 - saturate(dot({0}, {0})))))"]),
    );
    b.1 = "Artistic/Adjustment";
    b.node(
        "Contrast",
        vec![s("In", F3), s("Contrast", F1).f(1.0)],
        vec![s("Out", F3)],
        Gen::Tpl(&["(({0} - 0.21763764) * {1} + 0.21763764)"]),
    );
    b.node(
        "Hue",
        vec![s("In", F3), s("Offset", F1)],
        vec![s("Out", F3)],
        Gen::Fun(|c| {
            let off = if c.opt("Range") == "Normalized" {
                c.ins[1].clone()
            } else {
                format!("({} / 360.0)", c.ins[1])
            };
            vec![format!("sg_hue({}, {off})", c.ins[0])]
        }),
    )
    .opts(vec![opt("Range", &["Degrees", "Normalized"], "Degrees")]);
    b.node(
        "Saturation",
        vec![s("In", F3), s("Saturation", F1).f(1.0)],
        vec![s("Out", F3)],
        Gen::Tpl(&["(vec3f(dot({0}, vec3f(0.2126729, 0.7151522, 0.072175))) + {1} * ({0} - vec3f(dot({0}, vec3f(0.2126729, 0.7151522, 0.072175)))))"]),
    );
    b.node(
        "Invert Colors",
        vec![s("In", Dyn)],
        vec![s("Out", Dyn)],
        Gen::Fun(|c| {
            let w = c.dyn_ty.width() as usize;
            let m: Vec<&str> = ["Red", "Green", "Blue", "Alpha"][..w]
                .iter()
                .map(|n| if c.opt(n) == "False" { "0.0" } else { "1.0" })
                .collect();
            let m = format!("{}({})", c.dyn_ty.wgsl(), m.join(", "));
            vec![format!("abs({m} - {})", c.ins[0])]
        }),
    )
    .opts(vec![
        opt("Red", BOOL, "True"),
        opt("Green", BOOL, "True"),
        opt("Blue", BOOL, "True"),
        opt("Alpha", BOOL, "False"),
    ]);
    b.node(
        "Replace Color",
        vec![
            s("In", F3),
            s("From", F3).color(),
            s("To", F3).color(),
            s("Range", F1),
            s("Fuzziness", F1),
        ],
        vec![s("Out", F3)],
        Gen::Tpl(&["mix({2}, {0}, saturate((distance({1}, {0}) - {3}) / max({4}, 1e-5)))"]),
    );
    b.node(
        "White Balance",
        vec![s("In", F3), s("Temperature", F1), s("Tint", F1)],
        vec![s("Out", F3)],
        Gen::Tpl(&["sg_white_balance({0}, {1}, {2})"]),
    );
    b.1 = "Artistic/Blend";
    b.node(
        "Blend",
        vec![
            s("Base", Dyn),
            s("Blend", Dyn),
            s("Opacity", F1).f(1.0),
        ],
        vec![s("Out", Dyn)],
        Gen::Fun(|c| {
            let t = c.dyn_ty.wgsl();
            let (a, bl) = (c.ins[0].clone(), c.ins[1].clone());
            let one = format!("{t}(1.0)");
            let half = format!("{t}(0.5)");
            let r = match c.opt("Mode") {
                "Burn" => format!("({one} - ({one} - {bl}) / {a})"),
                "Darken" => format!("min({bl}, {a})"),
                "Difference" => format!("abs({bl} - {a})"),
                "Dodge" => format!("({a} / ({one} - {bl}))"),
                "Divide" => format!("({a} / ({bl} + {t}(1e-9)))"),
                "Exclusion" => format!("({bl} + {a} - 2.0 * {bl} * {a})"),
                "HardLight" => {
                    let z = c.temp(t, format!("step({bl}, {half})"));
                    format!(
                        "(2.0 * {a} * {bl} * {z} + ({one} - {z}) * ({one} - 2.0 * ({one} - {a}) * ({one} - {bl})))"
                    )
                }
                "HardMix" => format!("step({one} - {a}, {bl})"),
                "Lighten" => format!("max({bl}, {a})"),
                "LinearBurn" => format!("({a} + {bl} - {one})"),
                "LinearDodge" => format!("({a} + {bl})"),
                "LinearLight" => {
                    let z = c.temp(t, format!("step({half}, {bl})"));
                    format!(
                        "(({one} - {z}) * max({a} + 2.0 * {bl} - {one}, {t}(0.0)) + {z} * min({a} + 2.0 * ({bl} - {half}), {one}))"
                    )
                }
                "Multiply" => format!("({a} * {bl})"),
                "Negation" => format!("({one} - abs({one} - {bl} - {a}))"),
                "Screen" => format!("({one} - ({one} - {bl}) * ({one} - {a}))"),
                "Overlay" => {
                    let z = c.temp(t, format!("step({a}, {half})"));
                    format!(
                        "(2.0 * {a} * {bl} * {z} + ({one} - {z}) * ({one} - 2.0 * ({one} - {a}) * ({one} - {bl})))"
                    )
                }
                "PinLight" => {
                    let z = c.temp(t, format!("step({half}, {bl})"));
                    format!(
                        "({z} * max(2.0 * ({a} - {half}), {bl}) + ({one} - {z}) * min(2.0 * {a}, {bl}))"
                    )
                }
                "SoftLight" => {
                    let z = c.temp(t, format!("step({half}, {bl})"));
                    format!(
                        "((sqrt({a}) * (2.0 * {bl} - {one}) + 2.0 * {a} * ({one} - {bl})) * {z} + ({one} - {z}) * (2.0 * {a} * {bl} + {a} * {a} * ({one} - 2.0 * {bl})))"
                    )
                }
                "VividLight" => {
                    let z = c.temp(t, format!("step({half}, {bl})"));
                    format!(
                        "(({one} - {z}) * ({one} - ({one} - {a}) / max(2.0 * {bl}, {t}(1e-5))) + {z} * ({a} / max(2.0 * ({one} - {bl}), {t}(1e-5))))"
                    )
                }
                "Subtract" => format!("({a} - {bl})"),
                "Overwrite" => bl.clone(),
                _ => format!("({a} * {bl})"),
            };
            vec![format!("mix({a}, {r}, {})", c.ins[2])]
        }),
    )
    .opts(vec![opt(
        "Mode",
        &[
            "Burn",
            "Darken",
            "Difference",
            "Dodge",
            "Divide",
            "Exclusion",
            "HardLight",
            "HardMix",
            "Lighten",
            "LinearBurn",
            "LinearDodge",
            "LinearLight",
            "Multiply",
            "Negation",
            "Overlay",
            "PinLight",
            "Screen",
            "SoftLight",
            "Subtract",
            "VividLight",
            "Overwrite",
        ],
        "Overlay",
    )]);
    b.1 = "Artistic/Utility";
    b.node(
        "Colorspace Conversion",
        vec![s("In", F3)],
        vec![s("Out", F3)],
        Gen::Fun(|c| {
            let v = c.ins[0].clone();
            let to_rgb = match c.opt("From") {
                "Linear" => format!("sg_linear_to_srgb({v})"),
                "HSV" => format!("sg_hsv_to_rgb({v})"),
                _ => v,
            };
            vec![match c.opt("To") {
                "Linear" => format!("sg_srgb_to_linear({to_rgb})"),
                "HSV" => format!("sg_rgb_to_hsv({to_rgb})"),
                _ => to_rgb,
            }]
        }),
    )
    .opts(vec![
        opt("From", &["RGB", "Linear", "HSV"], "RGB"),
        opt("To", &["RGB", "Linear", "HSV"], "Linear"),
    ]);
    b.node(
        "Fresnel Effect",
        vec![
            s("Normal", F3).bind(Bind::WorldNormal),
            s("View Dir", F3).bind(Bind::ViewDirWorld),
            s("Power", F1).f(1.0),
        ],
        vec![s("Out", F1)],
        Gen::Tpl(&["pow(1.0 - saturate(dot(normalize({0}), normalize({1}))), {2})"]),
    );
    b.node(
        "Dither",
        vec![s("In", Dyn), s("Screen Position", F4)],
        vec![s("Out", Dyn)],
        Gen::Fun(|c| {
            vec![format!(
                "({} - {}(sg_dither(i.screen_pos.xy * vec2f(1920.0, 1080.0))))",
                c.ins[0],
                c.dyn_ty.wgsl()
            )]
        }),
    );

    // ---- Utility/Logic
    b.1 = "Utility/Logic";
    b.node(
        "Branch",
        vec![s("Predicate", Bool), s("True", Dyn).f(1.0), s("False", Dyn)],
        vec![s("Out", Dyn)],
        Gen::Tpl(&["select({2}, {1}, {0})"]),
    );
    b.node(
        "Comparison",
        vec![s("A", F1), s("B", F1)],
        vec![s("Out", Bool)],
        Gen::Fun(|c| {
            let op = match c.opt("Type") {
                "NotEqual" => "!=",
                "Less" => "<",
                "LessOrEqual" => "<=",
                "Greater" => ">",
                "GreaterOrEqual" => ">=",
                _ => "==",
            };
            vec![format!("({} {op} {})", c.ins[0], c.ins[1])]
        }),
    )
    .opts(vec![opt(
        "Type",
        &[
            "Equal",
            "NotEqual",
            "Less",
            "LessOrEqual",
            "Greater",
            "GreaterOrEqual",
        ],
        "Equal",
    )]);
    b.node(
        "And",
        vec![s("A", Bool), s("B", Bool)],
        vec![s("Out", Bool)],
        Gen::Tpl(&["({0} && {1})"]),
    );
    b.node(
        "Or",
        vec![s("A", Bool), s("B", Bool)],
        vec![s("Out", Bool)],
        Gen::Tpl(&["({0} || {1})"]),
    );
    b.node(
        "Nand",
        vec![s("A", Bool), s("B", Bool)],
        vec![s("Out", Bool)],
        Gen::Tpl(&["!({0} && {1})"]),
    );
    b.node(
        "Not",
        vec![s("In", Bool)],
        vec![s("Out", Bool)],
        Gen::Tpl(&["!({0})"]),
    );
    b.node(
        "All",
        vec![s("In", Dyn)],
        vec![s("Out", Bool)],
        Gen::Tpl(&["all({0} != {T}(0.0))"]),
    );
    b.node(
        "Any",
        vec![s("In", Dyn)],
        vec![s("Out", Bool)],
        Gen::Tpl(&["any({0} != {T}(0.0))"]),
    );
    b.node(
        "Is NaN",
        vec![s("In", F1)],
        vec![s("Out", Bool)],
        Gen::Tpl(&["({0} != {0})"]),
    );
    b.node(
        "Is Infinite",
        vec![s("In", F1)],
        vec![s("Out", Bool)],
        Gen::Tpl(&["(abs({0}) > 3.4e38)"]),
    );
    b.1 = "Utility";
    b.node(
        "Preview",
        vec![s("In", Dyn)],
        vec![s("Out", Dyn)],
        Gen::Tpl(&["{0}"]),
    );

    b.0
}

fn dot(ty: Ty, a: &str, b: &str) -> String {
    if ty.width() == 1 {
        format!("{a} * {b}")
    } else {
        format!("dot({a}, {b})")
    }
}

/// WGSL helpers the generated code calls (Unity's node functions).
pub const HELPERS: &str = r#"
fn sg_k1(v: f32) -> f32 { return v; }
fn sg_k2(v: vec2f) -> vec2f { return v; }
fn sg_k3(v: vec3f) -> vec3f { return v; }
fn sg_k4(v: vec4f) -> vec4f { return v; }

fn sg_safe_normalize(v: vec3f) -> vec3f {
    return v * inverseSqrt(max(dot(v, v), 1e-12));
}

fn sg_object_tangent(i: SurfaceInput) -> vec3f {
    return normalize(sg_world_to_object(i, i.world_tangent, 0.0));
}

fn sg_inverse3(m: mat3x3f) -> mat3x3f {
    let a = m[0];
    let b = m[1];
    let c = m[2];
    let r0 = cross(b, c);
    let r1 = cross(c, a);
    let r2 = cross(a, b);
    let det = dot(a, r0);
    let inv = select(1.0 / det, 0.0, abs(det) < 1e-12);
    return transpose(mat3x3f(r0, r1, r2)) * inv;
}

fn sg_world_to_object(i: SurfaceInput, v: vec3f, w: f32) -> vec3f {
    let m = mat3x3f(i.object_to_world[0].xyz, i.object_to_world[1].xyz, i.object_to_world[2].xyz);
    return sg_inverse3(m) * (v - i.object_to_world[3].xyz * w);
}

fn sg_view_to_world(v: vec3f, w: f32) -> vec3f {
    let m = sg_view_matrix();
    let r = mat3x3f(m[0].xyz, m[1].xyz, m[2].xyz);
    return transpose(r) * (v - m[3].xyz * w);
}

fn sg_world_to_tangent(i: SurfaceInput, v: vec3f) -> vec3f {
    return vec3f(dot(v, i.world_tangent), dot(v, i.world_bitangent), dot(v, i.world_normal));
}

fn sg_tangent_to_world(i: SurfaceInput, v: vec3f) -> vec3f {
    return i.world_tangent * v.x + i.world_bitangent * v.y + i.world_normal * v.z;
}

fn sg_unpack_normal(c: vec4f) -> vec3f {
    let xy = c.xy * 2.0 - 1.0;
    return vec3f(xy, sqrt(saturate(1.0 - dot(xy, xy))));
}

fn sg_rotate_about_axis(v: vec3f, axis: vec3f, r: f32) -> vec3f {
    let a = normalize(axis);
    let s = sin(r);
    let c = cos(r);
    return v * c + cross(a, v) * s + a * dot(a, v) * (1.0 - c);
}

fn sg_rotate_uv(uv: vec2f, center: vec2f, r: f32) -> vec2f {
    let p = uv - center;
    let s = sin(r);
    let c = cos(r);
    return vec2f(p.x * c + p.y * s, -p.x * s + p.y * c) + center;
}

fn sg_polar(uv: vec2f, center: vec2f, radial: f32, len: f32) -> vec2f {
    let d = uv - center;
    return vec2f(length(d) * 2.0 * radial, atan2(d.x, d.y) * (1.0 / 6.28318530718) * len);
}

fn sg_twirl(uv: vec2f, center: vec2f, strength: f32, offset: vec2f) -> vec2f {
    let d = uv - center;
    let a = strength * length(d);
    return vec2f(cos(a) * d.x - sin(a) * d.y, sin(a) * d.x + cos(a) * d.y) + center + offset;
}

fn sg_spherize(uv: vec2f, center: vec2f, strength: vec2f, offset: vec2f) -> vec2f {
    let d = uv - center;
    let d2 = dot(d, d);
    return uv + d * (d2 * d2 * strength) + offset;
}

fn sg_radial_shear(uv: vec2f, center: vec2f, strength: vec2f, offset: vec2f) -> vec2f {
    let d = uv - center;
    let d2 = dot(d, d);
    return uv + vec2f(d.y, -d.x) * (d2 * strength) + offset;
}

fn sg_flipbook(uv: vec2f, width: f32, height: f32, tile_in: f32, invert: vec2f) -> vec2f {
    let tile = floor((tile_in + 0.00001) % (width * height));
    let count = vec2f(1.0) / vec2f(width, height);
    let base = floor((tile + 0.5) * count.x);
    let tx = invert.x * width - ((tile - width * base) + invert.x * 1.0);
    let ty = invert.y * height - (base + invert.y * 1.0);
    return (uv + abs(vec2f(tx, ty))) * count;
}

fn sg_noise_random(uv: vec2f) -> f32 {
    return fract(sin(dot(uv, vec2f(12.9898, 78.233))) * 43758.5453);
}

fn sg_value_noise(uv: vec2f) -> f32 {
    let i = floor(uv);
    var f = fract(uv);
    f = f * f * (3.0 - 2.0 * f);
    let r0 = sg_noise_random(i);
    let r1 = sg_noise_random(i + vec2f(1.0, 0.0));
    let r2 = sg_noise_random(i + vec2f(0.0, 1.0));
    let r3 = sg_noise_random(i + vec2f(1.0, 1.0));
    return mix(mix(r0, r1, f.x), mix(r2, r3, f.x), f.y);
}

fn sg_simple_noise(uv: vec2f, scale: f32) -> f32 {
    var t = 0.0;
    for (var k = 0; k < 3; k++) {
        let freq = pow(2.0, f32(k));
        let amp = pow(0.5, f32(3 - k));
        t += sg_value_noise(uv * scale / freq) * amp;
    }
    return t;
}

fn sg_gradient_dir(p_in: vec2f) -> vec2f {
    let p = p_in % vec2f(289.0);
    var x = (34.0 * p.x + 1.0) * p.x % 289.0 + p.y;
    x = (34.0 * x + 1.0) * x % 289.0;
    x = fract(x / 41.0) * 2.0 - 1.0;
    return normalize(vec2f(x - floor(x + 0.5), abs(x) - 0.5));
}

fn sg_gradient_noise(p: vec2f) -> f32 {
    let ip = floor(p);
    var fp = fract(p);
    let d00 = dot(sg_gradient_dir(ip), fp);
    let d01 = dot(sg_gradient_dir(ip + vec2f(0.0, 1.0)), fp - vec2f(0.0, 1.0));
    let d10 = dot(sg_gradient_dir(ip + vec2f(1.0, 0.0)), fp - vec2f(1.0, 0.0));
    let d11 = dot(sg_gradient_dir(ip + vec2f(1.0, 1.0)), fp - vec2f(1.0, 1.0));
    fp = fp * fp * fp * (fp * (fp * 6.0 - 15.0) + 10.0);
    return mix(mix(d00, d01, fp.y), mix(d10, d11, fp.y), fp.x);
}

fn sg_voronoi_random(uv_in: vec2f, offset: f32) -> vec2f {
    let uv = fract(sin(vec2f(uv_in.x * 15.27 + uv_in.y * 99.41, uv_in.x * 47.63 + uv_in.y * 89.98)) * 46839.32);
    return vec2f(sin(uv.y * offset) * 0.5 + 0.5, cos(uv.x * offset) * 0.5 + 0.5);
}

fn sg_voronoi(uv: vec2f, angle_offset: f32, density: f32) -> vec2f {
    let g = floor(uv * density);
    let f = fract(uv * density);
    var res = vec3f(8.0, 0.0, 0.0);
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let lattice = vec2f(f32(x), f32(y));
            let o = sg_voronoi_random(lattice + g, angle_offset);
            let d = distance(lattice + o, f);
            if (d < res.x) {
                res = vec3f(d, o.x, o.y);
            }
        }
    }
    return res.xy;
}

fn sg_checkerboard(uv: vec2f, dx: vec2f, dy: vec2f, a: vec3f, b: vec3f) -> vec3f {
    let len = sqrt(vec2f(dot(vec2f(dx.x, dy.x), vec2f(dx.x, dy.x)), dot(vec2f(dx.y, dy.y), vec2f(dx.y, dy.y))));
    let dist = 4.0 * abs(fract(uv + 0.25) - 0.5) - 1.0;
    let scale = 0.35 / max(len, vec2f(1e-5));
    let limiter = sqrt(clamp(1.1 - max(len.x, len.y), 0.0, 1.0));
    let va = clamp(dist * scale, vec2f(-1.0), vec2f(1.0));
    let alpha = saturate(0.5 + 0.5 * va.x * va.y * limiter);
    return mix(a, b, vec3f(alpha));
}

fn sg_polygon_distance(uv: vec2f, sides: f32, width: f32, height: f32) -> f32 {
    let pi = 3.14159265359;
    let aw = width * cos(pi / sides);
    let ah = height * cos(pi / sides);
    var p = (uv * 2.0 - 1.0) / vec2f(aw, ah);
    p.y = -p.y;
    let pc = atan2(p.x, p.y);
    let r = 2.0 * pi / sides;
    return cos(floor(0.5 + pc / r) * r - pc) * length(p);
}

fn sg_normal_blend_reoriented(a: vec3f, b: vec3f) -> vec3f {
    let t = a + vec3f(0.0, 0.0, 1.0);
    let u = b * vec3f(-1.0, -1.0, 1.0);
    return normalize(t / t.z * dot(t, u) - u);
}

fn sg_normal_from_height(i: SurfaceInput, dpx: vec3f, dpy: vec3f, dhx: f32, dhy: f32, strength: f32) -> vec3f {
    let n = i.world_normal;
    let cx = cross(n, dpx);
    let cy = cross(dpy, n);
    let d = dot(dpx, cy);
    let sgn = select(1.0, -1.0, d < 0.0);
    let surf = sgn / max(1.192093e-15, abs(d));
    let grad = surf * (dhx * cy + dhy * cx);
    return normalize(n - strength * grad);
}

fn sg_rgb_to_hsv(c: vec3f) -> vec3f {
    let k = vec4f(0.0, -1.0 / 3.0, 2.0 / 3.0, -1.0);
    let p = mix(vec4f(c.bg, k.wz), vec4f(c.gb, k.xy), step(c.b, c.g));
    let q = mix(vec4f(p.xyw, c.r), vec4f(c.r, p.yzx), step(p.x, c.r));
    let d = q.x - min(q.w, q.y);
    let e = 1e-10;
    return vec3f(abs(q.z + (q.w - q.y) / (6.0 * d + e)), d / (q.x + e), q.x);
}

fn sg_hsv_to_rgb(c: vec3f) -> vec3f {
    let k = vec4f(1.0, 2.0 / 3.0, 1.0 / 3.0, 3.0);
    let p = abs(fract(c.xxx + k.xyz) * 6.0 - k.www);
    return c.z * mix(k.xxx, saturate(p - k.xxx), c.y);
}

fn sg_hue(c: vec3f, offset: f32) -> vec3f {
    var hsv = sg_rgb_to_hsv(c);
    let h = hsv.x + offset;
    hsv.x = select(select(h, h - 1.0, h > 1.0), h + 1.0, h < 0.0);
    return sg_hsv_to_rgb(hsv);
}

fn sg_srgb_to_linear(c: vec3f) -> vec3f {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3f(2.4));
    return select(hi, lo, c <= vec3f(0.04045));
}

fn sg_linear_to_srgb(c: vec3f) -> vec3f {
    let lo = c * 12.92;
    let hi = 1.055 * pow(max(c, vec3f(0.0)), vec3f(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3f(0.0031308));
}

fn sg_white_balance(c: vec3f, temperature: f32, tint: f32) -> vec3f {
    let t1 = temperature * 10.0 / 6.0;
    let t2 = tint * 10.0 / 6.0;
    let x = 0.31271 - t1 * select(0.05, 0.1, t1 < 0.0);
    let std_y = 2.87 * x - 3.0 * x * x - 0.27509507;
    let y = std_y + t2 * 0.05;
    let w1 = vec3f(0.949237, 1.03542, 1.08728);
    let yy = 1.0;
    let xx = yy * x / y;
    let zz = yy * (1.0 - x - y) / y;
    let l = 0.7328 * xx + 0.4296 * yy - 0.1624 * zz;
    let m = -0.7036 * xx + 1.6975 * yy + 0.0061 * zz;
    let s = 0.0030 * xx + 0.0136 * yy + 0.9834 * zz;
    let balance = w1 / vec3f(l, m, s);
    let lin_to_lms = mat3x3f(
        vec3f(3.90405e-1, 7.08416e-2, 2.31082e-2),
        vec3f(5.49941e-1, 9.63172e-1, 1.28021e-1),
        vec3f(8.92632e-3, 1.35775e-3, 9.36245e-1));
    let lms_to_lin = mat3x3f(
        vec3f(2.85847e+0, -2.10182e-1, -4.18120e-2),
        vec3f(-1.62879e+0, 1.15820e+0, -1.18169e-1),
        vec3f(-2.48910e-2, 3.24281e-4, 1.06867e+0));
    return lms_to_lin * ((lin_to_lms * c) * balance);
}

fn sg_dither(p: vec2f) -> f32 {
    var m = array<f32, 16>(
        1.0 / 17.0, 9.0 / 17.0, 3.0 / 17.0, 11.0 / 17.0,
        13.0 / 17.0, 5.0 / 17.0, 15.0 / 17.0, 7.0 / 17.0,
        4.0 / 17.0, 12.0 / 17.0, 2.0 / 17.0, 10.0 / 17.0,
        16.0 / 17.0, 8.0 / 17.0, 14.0 / 17.0, 6.0 / 17.0);
    let k = (u32(p.x) % 4u) * 4u + (u32(p.y) % 4u);
    return m[k];
}
"#;
