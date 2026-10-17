//! Unity's VFX Graph blocks: what each one reads and the WGSL it adds to its context.
//!
//! GPU blocks write statements on `q`, the particle being initialized, updated or drawn.
//! Spawn blocks run on the CPU (Unity's spawn system does too).
//! Reference: <https://docs.unity3d.com/Packages/com.unity.visualeffectgraph@17.0/manual/Block-Library.html>

use oxide_core::curves::{AnimationCurve, Gradient};
use oxide_math::Color;
use oxide_shadergraph::nodes::Opt;
use oxide_shadergraph::{Slot, Ty};
use std::sync::OnceLock;

/// Which context a block can go in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Spawn,
    Initialize,
    Update,
    Output,
}

impl Stage {
    pub fn of(context: &str) -> Option<Stage> {
        Some(match context {
            crate::SPAWN => Stage::Spawn,
            crate::INITIALIZE => Stage::Initialize,
            crate::UPDATE => Stage::Update,
            crate::OUTPUT => Stage::Output,
            _ => return None,
        })
    }
}

/// What a GPU block's code generator gets.
pub struct BlockCx {
    /// Input expressions converted to each input's type.
    pub ins: Vec<String>,
    pub opts: Vec<(&'static str, String)>,
    /// Unique prefix for temporaries.
    pub prefix: String,
    /// Offset of the block's baked gradient or curve in `luts`.
    pub lut: u32,
    salt: u32,
}

impl BlockCx {
    pub fn new(
        ins: Vec<String>,
        opts: Vec<(&'static str, String)>,
        prefix: String,
        lut: u32,
        salt: u32,
    ) -> Self {
        Self {
            ins,
            opts,
            prefix,
            lut,
            salt,
        }
    }

    pub fn opt(&self, name: &str) -> &str {
        self.opts
            .iter()
            .find(|(n, _)| *n == name)
            .map_or("", |(_, v)| v.as_str())
    }

    /// A per-particle random number in [0, 1) unique to this use.
    pub fn rand(&mut self) -> String {
        self.salt += 1;
        format!("vfx_rand(q.seed, {}u)", self.salt)
    }

    /// `attr <op>= value` by the block's Composition setting.
    pub fn compose(&self, attr: &str, value: &str) -> String {
        match self.opt("Composition") {
            "Add" => format!("{attr} += {value};"),
            "Multiply" => format!("{attr} *= {value};"),
            _ => format!("{attr} = {value};"),
        }
    }
}

#[derive(Clone, Copy)]
pub enum BlockCode {
    /// Evaluated on the CPU by the spawn system.
    Spawn,
    Gpu(fn(&mut BlockCx) -> String),
}

#[derive(Clone)]
pub struct BlockDef {
    pub kind: &'static str,
    /// Add Block menu category.
    pub menu: &'static str,
    pub stages: &'static [Stage],
    pub inputs: Vec<Slot>,
    pub options: Vec<Opt>,
    pub gradient: Option<fn() -> Gradient>,
    pub curve: Option<fn() -> AnimationCurve>,
    pub code: BlockCode,
}

pub fn library() -> &'static [BlockDef] {
    static LIB: OnceLock<Vec<BlockDef>> = OnceLock::new();
    LIB.get_or_init(build)
}

pub fn find(kind: &str) -> Option<&'static BlockDef> {
    library().iter().find(|d| d.kind == kind)
}

/// Blocks that can go in a context.
pub fn for_stage(stage: Stage) -> impl Iterator<Item = &'static BlockDef> {
    library().iter().filter(move |d| d.stages.contains(&stage))
}

const COMPOSITION: &[&str] = &["Overwrite", "Add", "Multiply"];
const PARTICLE: &[Stage] = &[Stage::Initialize, Stage::Update, Stage::Output];
const INIT_UPDATE: &[Stage] = &[Stage::Initialize, Stage::Update];
const UPDATE_ONLY: &[Stage] = &[Stage::Update];
const LIFE: &[Stage] = &[Stage::Update, Stage::Output];
const SPAWN: &[Stage] = &[Stage::Spawn];

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

fn def(
    kind: &'static str,
    menu: &'static str,
    stages: &'static [Stage],
    inputs: Vec<Slot>,
    code: BlockCode,
) -> BlockDef {
    BlockDef {
        kind,
        menu,
        stages,
        inputs,
        options: Vec::new(),
        gradient: None,
        curve: None,
        code,
    }
}

fn with_opts(mut d: BlockDef, o: Vec<Opt>) -> BlockDef {
    d.options = o;
    d
}

/// Unity's attributes as WGSL fields of `q`, and their types.
pub const ATTRIBUTES: &[(&str, &str, Ty)] = &[
    ("position", "q.position", Ty::F3),
    ("velocity", "q.velocity", Ty::F3),
    ("age", "q.age", Ty::F1),
    ("lifetime", "q.lifetime", Ty::F1),
    ("color", "q.color", Ty::F3),
    ("alpha", "q.alpha", Ty::F1),
    ("size", "q.size", Ty::F1),
    ("angle", "q.angle", Ty::F1),
    ("texIndex", "q.tex_index", Ty::F1),
    ("mass", "q.mass", Ty::F1),
    ("scaleY", "q.scale_y", Ty::F1),
    ("particleId", "f32(q.id)", Ty::F1),
    ("seed", "f32(q.seed)", Ty::F1),
];

#[allow(clippy::vec_init_then_push)]
fn build() -> Vec<BlockDef> {
    use Ty::*;
    let mut v = Vec::new();

    // ---- Spawn (CPU).
    v.push(def(
        "Constant Spawn Rate",
        "Spawn",
        SPAWN,
        vec![s("Rate", F1).f(10.0)],
        BlockCode::Spawn,
    ));
    v.push(def(
        "Single Burst",
        "Spawn",
        SPAWN,
        vec![s("Count", F1).f(10.0), s("Delay", F1)],
        BlockCode::Spawn,
    ));
    v.push(def(
        "Periodic Burst",
        "Spawn",
        SPAWN,
        vec![s("Count", F1).f(10.0), s("Delay", F1).f(1.0)],
        BlockCode::Spawn,
    ));
    v.push(def(
        "Variable Spawn Rate",
        "Spawn",
        SPAWN,
        vec![
            s("Rate Min", F1).f(5.0),
            s("Rate Max", F1).f(20.0),
            s("Period Min", F1).f(0.5),
            s("Period Max", F1).f(1.0),
        ],
        BlockCode::Spawn,
    ));

    // ---- Set Attribute (Unity's Set <Attribute> blocks, with Composition).
    macro_rules! set {
        ($kind:literal, $attr:literal, $slot:expr) => {
            v.push(with_opts(
                def(
                    $kind,
                    "Attribute/Set",
                    PARTICLE,
                    vec![$slot],
                    BlockCode::Gpu(|c| {
                        let x = c.ins[0].clone();
                        c.compose($attr, &x)
                    }),
                ),
                vec![opt("Composition", COMPOSITION, "Overwrite")],
            ));
        };
    }
    set!("Set Position", "q.position", s("Position", F3));
    set!(
        "Set Velocity",
        "q.velocity",
        s("Velocity", F3).val([0.0, 1.0, 0.0, 0.0])
    );
    set!("Set Color", "q.color", s("Color", F3).val([1.0; 4]).color());
    set!("Set Alpha", "q.alpha", s("Alpha", F1).f(1.0));
    set!("Set Size", "q.size", s("Size", F1).f(0.1));
    set!("Set Angle", "q.angle", s("Angle", F1));
    set!("Set Lifetime", "q.lifetime", s("Lifetime", F1).f(1.0));
    set!("Set Tex Index", "q.tex_index", s("Tex Index", F1));
    set!("Set Mass", "q.mass", s("Mass", F1).f(1.0));
    set!("Set Scale Y", "q.scale_y", s("Scale Y", F1).f(1.0));

    // ---- Set Attribute Random (Uniform between A and B).
    macro_rules! random {
        ($kind:literal, $attr:literal, $ty:expr, $a:expr, $b:expr) => {
            v.push(with_opts(
                def(
                    $kind,
                    "Attribute/Set Random",
                    PARTICLE,
                    vec![s("Min", $ty).val($a), s("Max", $ty).val($b)],
                    BlockCode::Gpu(|c| {
                        let (a, b) = (c.ins[0].clone(), c.ins[1].clone());
                        let t = match $ty {
                            Ty::F3 => format!("vec3f({}, {}, {})", c.rand(), c.rand(), c.rand()),
                            _ => c.rand(),
                        };
                        c.compose($attr, &format!("mix({a}, {b}, {t})"))
                    }),
                ),
                vec![opt("Composition", COMPOSITION, "Overwrite")],
            ));
        };
    }
    random!(
        "Set Velocity Random",
        "q.velocity",
        F3,
        [-0.5, 1.0, -0.5, 0.0],
        [0.5, 3.0, 0.5, 0.0]
    );
    random!("Set Lifetime Random", "q.lifetime", F1, [1.0; 4], [3.0; 4]);
    random!("Set Size Random", "q.size", F1, [0.05; 4], [0.2; 4]);
    random!("Set Angle Random", "q.angle", F1, [0.0; 4], [360.0; 4]);
    random!("Set Alpha Random", "q.alpha", F1, [0.5; 4], [1.0; 4]);
    random!(
        "Set Tex Index Random",
        "q.tex_index",
        F1,
        [0.0; 4],
        [16.0; 4]
    );
    v.push(with_opts(
        def(
            "Set Color Random",
            "Attribute/Set Random",
            PARTICLE,
            vec![
                s("Color A", F3).val([1.0, 0.4, 0.1, 1.0]).color(),
                s("Color B", F3).val([1.0, 0.9, 0.3, 1.0]).color(),
            ],
            BlockCode::Gpu(|c| {
                let (a, b, t) = (c.ins[0].clone(), c.ins[1].clone(), c.rand());
                c.compose("q.color", &format!("mix({a}, {b}, {t})"))
            }),
        ),
        vec![opt("Composition", COMPOSITION, "Overwrite")],
    ));

    // ---- Velocity.
    v.push(def(
        "Set Velocity Random Direction",
        "Velocity",
        PARTICLE,
        vec![s("Speed Min", F1).f(1.0), s("Speed Max", F1).f(3.0)],
        BlockCode::Gpu(|c| {
            let (a, b, r1, r2, r3) = (
                c.ins[0].clone(),
                c.ins[1].clone(),
                c.rand(),
                c.rand(),
                c.rand(),
            );
            format!("q.velocity = vfx_random_direction({r1}, {r2}) * mix({a}, {b}, {r3});")
        }),
    ));

    // ---- Position (shapes).
    v.push(with_opts(
        def(
            "Set Position (Sphere)",
            "Position",
            INIT_UPDATE,
            vec![s("Center", F3), s("Radius", F1).f(1.0)],
            BlockCode::Gpu(|c| {
                let (center, radius, r1, r2, r3) = (
                    c.ins[0].clone(),
                    c.ins[1].clone(),
                    c.rand(),
                    c.rand(),
                    c.rand(),
                );
                let dist = if c.opt("Spawn Mode") == "Surface" {
                    radius
                } else {
                    format!("{radius} * pow({r3}, 1.0 / 3.0)")
                };
                format!("q.position = {center} + vfx_random_direction({r1}, {r2}) * {dist};")
            }),
        ),
        vec![opt("Spawn Mode", &["Volume", "Surface"], "Volume")],
    ));
    v.push(def(
        "Set Position (Box)",
        "Position",
        INIT_UPDATE,
        vec![s("Center", F3), s("Size", F3).f(1.0)],
        BlockCode::Gpu(|c| {
            let (center, size) = (c.ins[0].clone(), c.ins[1].clone());
            let r = format!("vec3f({}, {}, {})", c.rand(), c.rand(), c.rand());
            format!("q.position = {center} + ({r} - 0.5) * {size};")
        }),
    ));
    v.push(with_opts(
        def(
            "Set Position (Circle)",
            "Position",
            INIT_UPDATE,
            vec![s("Center", F3), s("Radius", F1).f(1.0)],
            BlockCode::Gpu(|c| {
                let (center, radius, r1, r2) =
                    (c.ins[0].clone(), c.ins[1].clone(), c.rand(), c.rand());
                let dist = if c.opt("Spawn Mode") == "Surface" {
                    radius
                } else {
                    format!("{radius} * sqrt({r2})")
                };
                let a = format!("{r1} * 6.28318530718");
                format!("q.position = {center} + vec3f(cos({a}), sin({a}), 0.0) * {dist};")
            }),
        ),
        vec![opt("Spawn Mode", &["Volume", "Surface"], "Volume")],
    ));
    v.push(def(
        "Set Position (Cone)",
        "Position",
        INIT_UPDATE,
        vec![
            s("Base Radius", F1).f(1.0),
            s("Top Radius", F1).f(0.1),
            s("Height", F1).f(1.0),
        ],
        BlockCode::Gpu(|c| {
            let (rb, rt, h, r1, r2, r3) = (c.ins[0].clone(), c.ins[1].clone(), c.ins[2].clone(), c.rand(), c.rand(), c.rand());
            let p = format!("{}h", c.prefix);
            format!(
                "let {p} = {r1};\nlet {p}a = {r2} * 6.28318530718;\nq.position = vec3f(cos({p}a), 0.0, sin({p}a)) * mix({rb}, {rt}, {p}) * sqrt({r3}) + vec3f(0.0, {p} * {h}, 0.0);"
            )
        }),
    ));
    v.push(def(
        "Set Position (Line)",
        "Position",
        INIT_UPDATE,
        vec![s("Start", F3), s("End", F3).val([1.0, 0.0, 0.0, 0.0])],
        BlockCode::Gpu(|c| {
            let (a, b, r) = (c.ins[0].clone(), c.ins[1].clone(), c.rand());
            format!("q.position = mix({a}, {b}, {r});")
        }),
    ));
    v.push(def(
        "Set Position (Torus)",
        "Position",
        INIT_UPDATE,
        vec![s("Center", F3), s("Major Radius", F1).f(1.0), s("Minor Radius", F1).f(0.2)],
        BlockCode::Gpu(|c| {
            let (center, big, small, r1, r2, r3) = (c.ins[0].clone(), c.ins[1].clone(), c.ins[2].clone(), c.rand(), c.rand(), c.rand());
            let p = &c.prefix;
            format!(
                "let {p}u = {r1} * 6.28318530718;\nlet {p}v = {r2} * 6.28318530718;\nlet {p}r = {small} * sqrt({r3});\nq.position = {center} + vec3f(({big} + {p}r * cos({p}v)) * cos({p}u), {p}r * sin({p}v), ({big} + {p}r * cos({p}v)) * sin({p}u));"
            )
        }),
    ));

    // ---- Forces.
    v.push(def(
        "Gravity",
        "Force",
        UPDATE_ONLY,
        vec![s("Force", F3).val([0.0, -9.81, 0.0, 0.0])],
        BlockCode::Gpu(|c| format!("q.velocity += {} * vfx_dt();", c.ins[0])),
    ));
    v.push(with_opts(
        def(
            "Force",
            "Force",
            UPDATE_ONLY,
            vec![
                s("Force", F3).val([1.0, 0.0, 0.0, 0.0]),
                s("Drag", F1).f(0.5),
            ],
            BlockCode::Gpu(|c| {
                let (f, d) = (c.ins[0].clone(), c.ins[1].clone());
                if c.opt("Mode") == "Relative" {
                    format!("q.velocity += ({f} - q.velocity) * min(1.0, {d} * vfx_dt() / q.mass);")
                } else {
                    format!("q.velocity += {f} / q.mass * vfx_dt();")
                }
            }),
        ),
        vec![opt("Mode", &["Absolute", "Relative"], "Absolute")],
    ));
    v.push(def(
        "Linear Drag",
        "Force",
        UPDATE_ONLY,
        vec![s("Drag Coefficient", F1).f(0.5)],
        BlockCode::Gpu(|c| {
            format!(
                "q.velocity *= max(0.0, 1.0 - {} * vfx_dt() / q.mass);",
                c.ins[0]
            )
        }),
    ));
    v.push(def(
        "Turbulence",
        "Force",
        UPDATE_ONLY,
        vec![
            s("Intensity", F1).f(1.0),
            s("Drag", F1).f(1.0),
            s("Frequency", F1).f(1.0),
            s("Octaves", F1).f(1.0),
            s("Roughness", F1).f(0.5),
            s("Lacunarity", F1).f(2.0),
        ],
        BlockCode::Gpu(|c| {
            let [i, d, f, o, r, l] = [0, 1, 2, 3, 4, 5].map(|k| c.ins[k].clone());
            let p = &c.prefix;
            format!(
                "let {p}n = vfx_fbm3(q.position * {f} + vec3f(0.5, 0.5, 0.5 + vfx_time() * 0.25), {o}, {r}, {l}) * {i};\nq.velocity += ({p}n - q.velocity) * min(1.0, vfx_dt() * {d} / q.mass);"
            )
        }),
    ));
    v.push(def(
        "Conform to Sphere",
        "Force",
        UPDATE_ONLY,
        vec![
            s("Center", F3),
            s("Radius", F1).f(1.0),
            s("Attraction Speed", F1).f(5.0),
            s("Attraction Force", F1).f(20.0),
            s("Stick Distance", F1).f(0.1),
            s("Stick Force", F1).f(50.0),
        ],
        BlockCode::Gpu(|c| {
            let [center, radius, speed, force, stick_d, stick_f] = [0, 1, 2, 3, 4, 5].map(|k| c.ins[k].clone());
            let p = &c.prefix;
            format!(
                "var {p}dir = {center} - q.position;\nlet {p}dc = length({p}dir);\nlet {p}ds = {p}dc - {radius};\n{p}dir /= max(1e-6, {p}dc);\nlet {p}sn = dot({p}dir, q.velocity);\nlet {p}ratio = smoothstep(0.0, {stick_d} * 2.0, abs({p}ds));\nlet {p}ts = sign({p}ds) * {speed} * {p}ratio;\nlet {p}dv = {p}ts - {p}sn;\nq.velocity += sign({p}dv) * min(abs({p}dv), vfx_dt() * mix({stick_f}, {force}, {p}ratio)) * {p}dir / q.mass;"
            )
        }),
    ));
    v.push(def(
        "Vortex",
        "Force",
        UPDATE_ONLY,
        vec![
            s("Center", F3),
            s("Axis", F3).val([0.0, 1.0, 0.0, 0.0]),
            s("Speed", F1).f(2.0),
            s("Pull", F1).f(0.5),
        ],
        BlockCode::Gpu(|c| {
            let [center, axis, speed, pull] = [0, 1, 2, 3].map(|k| c.ins[k].clone());
            let p = &c.prefix;
            format!(
                "let {p}a = normalize({axis});\nlet {p}r = q.position - {center};\nlet {p}rad = {p}r - {p}a * dot({p}r, {p}a);\nq.velocity += (cross({p}a, {p}rad) * {speed} - {p}rad * {pull}) * vfx_dt();"
            )
        }),
    ));

    // ---- Collision.
    let collide_opts = || vec![opt("Mode", &["Solid", "Inverted"], "Solid")];
    v.push(with_opts(
        def(
            "Collide with Plane",
            "Collision",
            UPDATE_ONLY,
            vec![
                s("Point", F3),
                s("Normal", F3).val([0.0, 1.0, 0.0, 0.0]),
                s("Bounce", F1).f(0.1),
                s("Friction", F1),
                s("Lifetime Loss", F1),
            ],
            BlockCode::Gpu(|c| {
                let [point, normal, bounce, friction, loss] = [0, 1, 2, 3, 4].map(|k| c.ins[k].clone());
                let p = &c.prefix;
                let flip = if c.opt("Mode") == "Inverted" { "-" } else { "" };
                format!(
                    "let {p}n = {flip}normalize({normal});\nlet {p}d = dot({p}n, q.position - {point});\nif ({p}d < 0.0) {{\n    q.position -= {p}n * {p}d;\n    {}\n}}",
                    response(p, bounce, friction, loss)
                )
            }),
        ),
        collide_opts(),
    ));
    v.push(with_opts(
        def(
            "Collide with Sphere",
            "Collision",
            UPDATE_ONLY,
            vec![
                s("Center", F3),
                s("Radius", F1).f(1.0),
                s("Bounce", F1).f(0.1),
                s("Friction", F1),
                s("Lifetime Loss", F1),
            ],
            BlockCode::Gpu(|c| {
                let [center, radius, bounce, friction, loss] = [0, 1, 2, 3, 4].map(|k| c.ins[k].clone());
                let p = &c.prefix;
                let (cond, sign) = if c.opt("Mode") == "Inverted" {
                    (format!("{p}l > {radius}"), "-")
                } else {
                    (format!("{p}l < {radius}"), "")
                };
                format!(
                    "let {p}r = q.position - {center};\nlet {p}l = length({p}r);\nif ({cond} && {p}l > 1e-6) {{\n    let {p}n = {sign}({p}r / {p}l);\n    q.position = {center} + ({p}r / {p}l) * {radius};\n    {}\n}}",
                    response(p, bounce, friction, loss)
                )
            }),
        ),
        collide_opts(),
    ));
    v.push(with_opts(
        def(
            "Collide with AABox",
            "Collision",
            UPDATE_ONLY,
            vec![
                s("Center", F3),
                s("Size", F3).f(1.0),
                s("Bounce", F1).f(0.1),
                s("Friction", F1),
                s("Lifetime Loss", F1),
            ],
            BlockCode::Gpu(|c| {
                let [center, size, bounce, friction, loss] = [0, 1, 2, 3, 4].map(|k| c.ins[k].clone());
                let p = &c.prefix;
                // Solid pushes particles out of the box; Inverted keeps them inside it.
                let inverted = c.opt("Mode") == "Inverted";
                let test = if inverted {
                    format!("any(abs({p}r) > {p}h)")
                } else {
                    format!("all(abs({p}r) < {p}h)")
                };
                let place = if inverted {
                    format!("{p}n = -{p}n;\n    q.position = {center} + clamp({p}r, -{p}h, {p}h);")
                } else {
                    format!("q.position += {p}n * {p}depth;")
                };
                format!(
                    "let {p}h = {size} * 0.5;\nlet {p}r = q.position - {center};\nif ({test}) {{\n    let {p}o = {p}h - abs({p}r);\n    var {p}n = vec3f(0.0);\n    var {p}depth = 0.0;\n    if ({p}o.x <= {p}o.y && {p}o.x <= {p}o.z) {{ {p}n.x = sign({p}r.x); {p}depth = {p}o.x; }} else if ({p}o.y <= {p}o.z) {{ {p}n.y = sign({p}r.y); {p}depth = {p}o.y; }} else {{ {p}n.z = sign({p}r.z); {p}depth = {p}o.z; }}\n    {place}\n    {}\n}}",
                    response(p, bounce, friction, loss)
                )
            }),
        ),
        collide_opts(),
    ));

    // ---- Kill.
    v.push(with_opts(
        def(
            "Kill (Sphere)",
            "Kill",
            UPDATE_ONLY,
            vec![s("Center", F3), s("Radius", F1).f(1.0)],
            BlockCode::Gpu(|c| {
                let op = if c.opt("Mode") == "Kill Outside" {
                    ">"
                } else {
                    "<"
                };
                format!(
                    "if (distance(q.position, {}) {op} {}) {{ q.age = q.lifetime; }}",
                    c.ins[0], c.ins[1]
                )
            }),
        ),
        vec![opt("Mode", &["Kill Inside", "Kill Outside"], "Kill Inside")],
    ));
    v.push(with_opts(
        def(
            "Kill (AABox)",
            "Kill",
            UPDATE_ONLY,
            vec![s("Center", F3), s("Size", F3).f(1.0)],
            BlockCode::Gpu(|c| {
                let inside = format!("all(abs(q.position - {}) < {} * 0.5)", c.ins[0], c.ins[1]);
                let cond = if c.opt("Mode") == "Kill Outside" {
                    format!("!{inside}")
                } else {
                    inside
                };
                format!("if ({cond}) {{ q.age = q.lifetime; }}")
            }),
        ),
        vec![opt("Mode", &["Kill Inside", "Kill Outside"], "Kill Inside")],
    ));

    // ---- Over life (gradient / curve baked into `luts`).
    let mut color_life = with_opts(
        def(
            "Set Color over Life",
            "Color",
            LIFE,
            vec![],
            BlockCode::Gpu(|c| {
                let v = format!("vfx_lut({}u, q.age / max(q.lifetime, 1e-6))", c.lut);
                let p = &c.prefix;
                let color = match c.opt("Color Composition") {
                    "Multiply" => format!("q.color *= {p}g.rgb;"),
                    "Add" => format!("q.color += {p}g.rgb;"),
                    _ => format!("q.color = {p}g.rgb;"),
                };
                let alpha = match c.opt("Alpha Composition") {
                    "Overwrite" => format!("q.alpha = {p}g.a;"),
                    "Ignore" => String::new(),
                    _ => format!("q.alpha *= {p}g.a;"),
                };
                format!("let {p}g = {v};\n{color}\n{alpha}")
            }),
        ),
        vec![
            opt("Color Composition", COMPOSITION, "Overwrite"),
            opt(
                "Alpha Composition",
                &["Overwrite", "Multiply", "Ignore"],
                "Overwrite",
            ),
        ],
    );
    color_life.gradient = Some(|| Gradient::two(Color::WHITE, Color::new(1.0, 1.0, 1.0, 0.0)));
    v.push(color_life);
    for (kind, attr) in [
        ("Set Size over Life", "q.size"),
        ("Set Alpha over Life", "q.alpha"),
    ] {
        let mut d = with_opts(
            def(
                kind,
                if kind.contains("Size") {
                    "Size"
                } else {
                    "Color"
                },
                LIFE,
                vec![],
                BlockCode::Gpu(match kind {
                    "Set Size over Life" => |c: &mut BlockCx| {
                        let v = format!("vfx_lut({}u, q.age / max(q.lifetime, 1e-6)).x", c.lut);
                        c.compose("q.size", &v)
                    },
                    _ => |c: &mut BlockCx| {
                        let v = format!("vfx_lut({}u, q.age / max(q.lifetime, 1e-6)).x", c.lut);
                        c.compose("q.alpha", &v)
                    },
                }),
            ),
            vec![opt(
                "Composition",
                COMPOSITION,
                if attr == "q.size" {
                    "Multiply"
                } else {
                    "Overwrite"
                },
            )],
        );
        d.curve = Some(|| AnimationCurve::linear(0.0, 1.0, 1.0, 0.0));
        v.push(d);
    }

    // ---- Flipbook.
    v.push(def(
        "Flipbook Player",
        "Flipbook",
        UPDATE_ONLY,
        vec![s("Frame Rate", F1).f(25.0)],
        BlockCode::Gpu(|c| format!("q.tex_index += {} * vfx_dt();", c.ins[0])),
    ));
    v
}

/// Unity's collision response: push out, bounce the normal velocity, slow the tangent one.
fn response(p: &str, bounce: String, friction: String, loss: String) -> String {
    format!(
        "let {p}pv = dot({p}n, q.velocity);\n    let {p}tv = q.velocity - {p}pv * {p}n;\n    if ({p}pv < 0.0) {{ q.velocity -= (1.0 + {bounce}) * {p}pv * {p}n; }}\n    q.velocity -= saturate({friction}) * {p}tv;\n    q.age += {loss} * q.lifetime;"
    )
}
