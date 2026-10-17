//! The Shader Graph and Sub Graph file formats (`.shadergraph`, `.shadersubgraph`).
//!
//! A graph is a list of nodes (each with a Unity node type, a position, inline values for
//! unconnected inputs and option settings) and the edges between their ports. The master
//! stack is two fixed nodes, `Vertex` and `Fragment`, whose inputs are Unity's blocks.
//! Reference: <https://docs.unity3d.com/Packages/com.unity.shadergraph@17.0/manual/index.html>

use oxide_assets::{Handle, Texture2D};
use oxide_core::Reflect;
use oxide_math::{Vector2, Vector4};

/// Unity's Blackboard property types.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PropertyType {
    #[default]
    Float,
    Vector2,
    Vector3,
    Vector4,
    Color,
    Boolean,
    Texture2D,
}

impl PropertyType {
    /// The name materials and the renderer use for this kind of value.
    pub fn kind_name(self) -> &'static str {
        match self {
            Self::Float => "Float",
            Self::Vector2 => "Vector2",
            Self::Vector3 => "Vector3",
            Self::Vector4 => "Vector4",
            Self::Color => "Color",
            Self::Boolean => "Boolean",
            Self::Texture2D => "Texture2D",
        }
    }
}

/// How a Float property shows in the material Inspector.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FloatMode {
    #[default]
    Default,
    Slider,
    Integer,
}

/// A Blackboard property (Unity's `AbstractShaderProperty`).
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct GraphProperty {
    /// Name shown on the Blackboard and in the material Inspector.
    pub name: String,
    /// Reference name materials store values under (`_BaseColor`).
    pub reference: String,
    pub kind: PropertyType,
    pub default: Vector4,
    pub default_texture: Option<Handle<Texture2D>>,
    pub float_mode: FloatMode,
    /// Slider range (min, max).
    pub range: Vector2,
    /// Shown in the material Inspector (Unity "Show In Inspector" / "Exposed").
    pub exposed: bool,
    /// Colour picker allows HDR values.
    pub hdr: bool,
    /// Blackboard category.
    pub category: String,
}

impl Default for GraphProperty {
    fn default() -> Self {
        Self {
            name: "Float".into(),
            reference: "_Float".into(),
            kind: PropertyType::Float,
            default: Vector4::ZERO,
            default_texture: None,
            float_mode: FloatMode::Default,
            range: Vector2::new(0.0, 1.0),
            exposed: true,
            hdr: false,
            category: String::new(),
        }
    }
}

/// A Boolean keyword (Unity's `Keyword` with Boolean type). Oxide branches on a material
/// value instead of compiling variants, so toggling it costs no recompile.
#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct GraphKeyword {
    pub name: String,
    pub reference: String,
    pub default: bool,
    pub exposed: bool,
}

/// One inline value of an unconnected input port.
#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct SlotValue {
    pub slot: String,
    pub value: Vector4,
}

/// One node setting (dropdowns, text fields, the property a Property node reads).
#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct NodeOption {
    pub name: String,
    pub value: String,
}

/// A node on the graph.
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct GraphNode {
    pub id: u32,
    /// Unity node name (`Sample Texture 2D`, `Multiply`, `Fragment`).
    pub kind: String,
    pub position: Vector2,
    pub values: Vec<SlotValue>,
    pub options: Vec<NodeOption>,
    /// Node preview shown.
    pub preview: bool,
    /// Unconnected ports hidden (Unity's collapse arrow).
    pub collapsed: bool,
}

impl Default for GraphNode {
    fn default() -> Self {
        Self {
            id: 0,
            kind: String::new(),
            position: Vector2::ZERO,
            values: Vec::new(),
            options: Vec::new(),
            preview: true,
            collapsed: false,
        }
    }
}

impl GraphNode {
    pub fn new(id: u32, kind: &str, position: Vector2) -> Self {
        Self {
            id,
            kind: kind.into(),
            position,
            ..Default::default()
        }
    }

    pub fn value(&self, slot: &str) -> Option<Vector4> {
        self.values.iter().find(|v| v.slot == slot).map(|v| v.value)
    }

    pub fn set_value(&mut self, slot: &str, value: Vector4) {
        match self.values.iter_mut().find(|v| v.slot == slot) {
            Some(v) => v.value = value,
            None => self.values.push(SlotValue {
                slot: slot.into(),
                value,
            }),
        }
    }

    pub fn option(&self, name: &str) -> Option<&str> {
        self.options
            .iter()
            .find(|o| o.name == name)
            .map(|o| o.value.as_str())
    }

    pub fn set_option(&mut self, name: &str, value: &str) {
        match self.options.iter_mut().find(|o| o.name == name) {
            Some(o) => o.value = value.into(),
            None => self.options.push(NodeOption {
                name: name.into(),
                value: value.into(),
            }),
        }
    }

    /// Builder form of [`set_option`](Self::set_option).
    pub fn with_option(mut self, name: &str, value: &str) -> Self {
        self.set_option(name, value);
        self
    }

    pub fn with_value(mut self, slot: &str, value: Vector4) -> Self {
        self.set_value(slot, value);
        self
    }
}

/// A connection from an output port to an input port.
#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct GraphEdge {
    pub from: u32,
    pub output: String,
    pub to: u32,
    pub input: String,
}

/// A titled group around nodes.
#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct GraphGroup {
    pub title: String,
    pub nodes: Vec<u32>,
}

/// A Sticky Note.
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct StickyNote {
    pub title: String,
    pub text: String,
    pub position: Vector2,
    pub size: Vector2,
}

impl Default for StickyNote {
    fn default() -> Self {
        Self {
            title: "Title".into(),
            text: "Write something here".into(),
            position: Vector2::ZERO,
            size: Vector2::new(200.0, 160.0),
        }
    }
}

/// The node network shared by graphs and sub graphs.
#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct GraphData {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub groups: Vec<GraphGroup>,
    pub notes: Vec<StickyNote>,
}

impl GraphData {
    pub fn node(&self, id: u32) -> Option<&GraphNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn node_mut(&mut self, id: u32) -> Option<&mut GraphNode> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    /// The first node of a type (the master stack's `Vertex` / `Fragment`).
    pub fn find(&self, kind: &str) -> Option<&GraphNode> {
        self.nodes.iter().find(|n| n.kind == kind)
    }

    pub fn next_id(&self) -> u32 {
        self.nodes.iter().map(|n| n.id + 1).max().unwrap_or(1)
    }

    /// Adds a node and returns its id.
    pub fn add(&mut self, mut node: GraphNode) -> u32 {
        node.id = self.next_id();
        let id = node.id;
        self.nodes.push(node);
        id
    }

    /// The edge into an input port.
    pub fn input_edge(&self, node: u32, input: &str) -> Option<&GraphEdge> {
        self.edges.iter().find(|e| e.to == node && e.input == input)
    }

    /// Connects `from.output` to `to.input`, replacing the input's old edge (an input takes
    /// one edge; an output may feed many).
    pub fn connect(&mut self, from: u32, output: &str, to: u32, input: &str) {
        self.edges.retain(|e| !(e.to == to && e.input == input));
        self.edges.push(GraphEdge {
            from,
            output: output.into(),
            to,
            input: input.into(),
        });
    }

    pub fn disconnect(&mut self, to: u32, input: &str) {
        self.edges.retain(|e| !(e.to == to && e.input == input));
    }

    /// Removes nodes and their edges (the master stack can't be removed).
    pub fn remove(&mut self, ids: &[u32]) {
        let gone: Vec<u32> = self
            .nodes
            .iter()
            .filter(|n| ids.contains(&n.id) && !is_stack(&n.kind) && n.kind != "SubGraphOutput")
            .map(|n| n.id)
            .collect();
        self.nodes.retain(|n| !gone.contains(&n.id));
        self.edges
            .retain(|e| !gone.contains(&e.from) && !gone.contains(&e.to));
        for g in &mut self.groups {
            g.nodes.retain(|n| !gone.contains(n));
        }
        self.groups.retain(|g| !g.nodes.is_empty());
    }

    /// Would `from -> to` close a loop?
    pub fn creates_cycle(&self, from: u32, to: u32) -> bool {
        let mut stack = vec![from];
        let mut seen = Vec::new();
        while let Some(n) = stack.pop() {
            if n == to {
                return true;
            }
            if seen.contains(&n) {
                continue;
            }
            seen.push(n);
            stack.extend(self.edges.iter().filter(|e| e.to == n).map(|e| e.from));
        }
        false
    }
}

/// Is this a master stack node?
pub fn is_stack(kind: &str) -> bool {
    kind == "Vertex" || kind == "Fragment"
}

/// URP's Material setting in the Graph Inspector.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GraphMaterial {
    #[default]
    Lit,
    Unlit,
}

#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GraphSurface {
    #[default]
    Opaque,
    Transparent,
}

#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderFace {
    #[default]
    Front,
    Both,
}

/// The space the Fragment Normal block is in (Unity "Fragment Normal Space").
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NormalSpace {
    #[default]
    Tangent,
    Object,
    World,
}

/// Graph Inspector → Graph Settings (URP target).
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct GraphSettings {
    pub material: GraphMaterial,
    pub surface: GraphSurface,
    pub render_face: RenderFace,
    pub alpha_clipping: bool,
    pub cast_shadows: bool,
    pub fragment_normal_space: NormalSpace,
}

impl Default for GraphSettings {
    fn default() -> Self {
        Self {
            material: GraphMaterial::Lit,
            surface: GraphSurface::Opaque,
            render_face: RenderFace::Front,
            alpha_clipping: false,
            cast_shadows: true,
            fragment_normal_space: NormalSpace::Tangent,
        }
    }
}

/// A Shader Graph (`.shadergraph`): a shader materials can use.
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct ShaderGraph {
    pub settings: GraphSettings,
    pub properties: Vec<GraphProperty>,
    pub keywords: Vec<GraphKeyword>,
    pub graph: GraphData,
}

impl Default for ShaderGraph {
    /// Unity's new URP Lit Shader Graph: the master stack and nothing else.
    fn default() -> Self {
        let mut graph = GraphData::default();
        graph.add(GraphNode::new(0, "Vertex", Vector2::new(0.0, -260.0)));
        graph.add(GraphNode::new(0, "Fragment", Vector2::new(0.0, 0.0)));
        Self {
            settings: GraphSettings::default(),
            properties: Vec::new(),
            keywords: Vec::new(),
            graph,
        }
    }
}

impl oxide_assets::Asset for ShaderGraph {
    const TYPE_NAME: &'static str = "ShaderGraph";
}

/// One output of a Sub Graph.
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct SubGraphOutput {
    pub name: String,
    pub kind: PropertyType,
}

impl Default for SubGraphOutput {
    fn default() -> Self {
        Self {
            name: "Out".into(),
            kind: PropertyType::Vector4,
        }
    }
}

/// A Sub Graph (`.shadersubgraph`): reusable nodes with inputs (its properties) and
/// outputs, used in other graphs through a Sub Graph node.
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct ShaderSubGraph {
    pub properties: Vec<GraphProperty>,
    pub outputs: Vec<SubGraphOutput>,
    pub graph: GraphData,
}

impl Default for ShaderSubGraph {
    fn default() -> Self {
        let mut graph = GraphData::default();
        graph.add(GraphNode::new(
            0,
            "SubGraphOutput",
            Vector2::new(300.0, 0.0),
        ));
        Self {
            properties: Vec::new(),
            outputs: vec![SubGraphOutput::default()],
            graph,
        }
    }
}

impl oxide_assets::Asset for ShaderSubGraph {
    const TYPE_NAME: &'static str = "ShaderSubGraph";
}

/// A unique reference name (`_Name`, `_Name_1`, ...) for a new property.
pub fn unique_reference(
    existing: &[GraphProperty],
    keywords: &[GraphKeyword],
    name: &str,
) -> String {
    let base: String = std::iter::once('_')
        .chain(
            name.chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '_'),
        )
        .collect();
    let taken = |r: &str| {
        existing.iter().any(|p| p.reference == r) || keywords.iter().any(|k| k.reference == r)
    };
    if !taken(&base) {
        return base;
    }
    (1..)
        .map(|i| format!("{base}_{i}"))
        .find(|r| !taken(r))
        .expect("free reference")
}
