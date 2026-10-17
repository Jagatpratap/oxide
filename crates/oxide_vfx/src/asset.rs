//! The Visual Effect Graph file format (`.vfx`).
//!
//! Like Unity's graph, a `.vfx` holds contexts (Spawn, Initialize Particle, Update
//! Particle, Output Particle Quad) joined by flow wires into systems, the blocks stacked in
//! each context, operator nodes wired into block inputs, and Blackboard properties a
//! Visual Effect can override. Contexts and operators share one [`GraphData`] (the node
//! canvas edits it); blocks are listed separately, in stack order.
//! Reference: <https://docs.unity3d.com/Packages/com.unity.visualeffectgraph@17.0/manual/GraphLogicAndPhilosophy.html>

use oxide_core::Reflect;
use oxide_core::curves::{AnimationCurve, Gradient, GradientAlphaKey, GradientColorKey};
use oxide_math::{Color, Vector2, Vector4};
use oxide_shadergraph::{GraphData, GraphNode, GraphProperty, NodeOption, SlotValue};

pub const SPAWN: &str = "Spawn";
pub const INITIALIZE: &str = "Initialize Particle";
pub const UPDATE: &str = "Update Particle";
pub const OUTPUT: &str = "Output Particle Quad";

/// The four context types.
pub const CONTEXTS: [&str; 4] = [SPAWN, INITIALIZE, UPDATE, OUTPUT];

pub fn is_context(kind: &str) -> bool {
    CONTEXTS.contains(&kind)
}

/// A block in a context's stack (Unity's `VFXBlock`).
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct VfxBlock {
    /// Unique in the asset; block input ports are named `b{id}/{input}`.
    pub id: u32,
    /// The context node it belongs to.
    pub context: u32,
    /// Unity block name (`Set Velocity Random`, `Gravity`).
    pub kind: String,
    pub enabled: bool,
    /// Inline values of unconnected inputs.
    pub values: Vec<SlotValue>,
    /// Settings (Composition, Spawn Mode, ...).
    pub options: Vec<NodeOption>,
    /// Used by the "over Life" colour blocks.
    pub gradient: Gradient,
    /// Used by the "over Life" size and alpha blocks.
    pub curve: AnimationCurve,
}

impl Default for VfxBlock {
    fn default() -> Self {
        Self {
            id: 0,
            context: 0,
            kind: String::new(),
            enabled: true,
            values: Vec::new(),
            options: Vec::new(),
            gradient: Gradient::default(),
            curve: AnimationCurve::default(),
        }
    }
}

impl VfxBlock {
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

    /// The port id of one of this block's inputs on its context node.
    pub fn port(&self, input: &str) -> String {
        port_id(self.id, input)
    }
}

pub fn port_id(block: u32, input: &str) -> String {
    format!("b{block}/{input}")
}

/// `(block id, input)` of a context port id.
pub fn parse_port(port: &str) -> Option<(u32, &str)> {
    let rest = port.strip_prefix('b')?;
    let (id, input) = rest.split_once('/')?;
    Some((id.parse().ok()?, input))
}

/// A Visual Effect Graph (`.vfx`).
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct VisualEffectAsset {
    /// Exposed properties (Blackboard): Float, Vector, Color and Boolean.
    pub properties: Vec<GraphProperty>,
    /// Contexts and operators.
    pub graph: GraphData,
    /// Every context's blocks, in stack order.
    pub blocks: Vec<VfxBlock>,
}

impl oxide_assets::Asset for VisualEffectAsset {
    const TYPE_NAME: &'static str = "VisualEffectAsset";
}

impl Default for VisualEffectAsset {
    /// A new Visual Effect Graph starts as Unity's Simple Loop template.
    fn default() -> Self {
        Self::simple_loop()
    }
}

impl VisualEffectAsset {
    /// A graph with nothing in it.
    pub fn empty() -> Self {
        Self {
            properties: Vec::new(),
            graph: GraphData::default(),
            blocks: Vec::new(),
        }
    }

    pub fn next_block_id(&self) -> u32 {
        self.blocks.iter().map(|b| b.id + 1).max().unwrap_or(1)
    }

    /// Adds a block at the end of a context's stack with the block type's defaults.
    pub fn add_block(&mut self, context: u32, kind: &str) -> u32 {
        let id = self.next_block_id();
        let mut b = VfxBlock {
            id,
            context,
            kind: kind.into(),
            ..Default::default()
        };
        if let Some(def) = crate::blocks::find(kind) {
            for o in &def.options {
                b.set_option(o.name, o.default);
            }
            if let Some(g) = &def.gradient {
                b.gradient = g();
            }
            if let Some(c) = &def.curve {
                b.curve = c();
            }
        }
        self.blocks.push(b);
        id
    }

    pub fn block(&self, id: u32) -> Option<&VfxBlock> {
        self.blocks.iter().find(|b| b.id == id)
    }

    pub fn block_mut(&mut self, id: u32) -> Option<&mut VfxBlock> {
        self.blocks.iter_mut().find(|b| b.id == id)
    }

    /// A context's blocks in stack order.
    pub fn blocks_of(&self, context: u32) -> impl Iterator<Item = &VfxBlock> {
        self.blocks.iter().filter(move |b| b.context == context)
    }

    /// Removes a block and the wires into its inputs.
    pub fn remove_block(&mut self, id: u32) {
        let prefix = format!("b{id}/");
        self.graph.edges.retain(|e| !e.input.starts_with(&prefix));
        self.blocks.retain(|b| b.id != id);
    }

    /// Moves a block up (`-1`) or down (`1`) within its context.
    pub fn move_block(&mut self, id: u32, dir: i32) {
        let Some(i) = self.blocks.iter().position(|b| b.id == id) else {
            return;
        };
        let ctx = self.blocks[i].context;
        let same: Vec<usize> = (0..self.blocks.len())
            .filter(|k| self.blocks[*k].context == ctx)
            .collect();
        let at = same.iter().position(|k| *k == i).expect("in its context");
        let to = at as i32 + dir;
        if to < 0 || to as usize >= same.len() {
            return;
        }
        self.blocks.swap(i, same[to as usize]);
    }

    /// Removes nodes (contexts take their blocks with them).
    pub fn remove_nodes(&mut self, ids: &[u32]) {
        let blocks: Vec<u32> = self
            .blocks
            .iter()
            .filter(|b| ids.contains(&b.context))
            .map(|b| b.id)
            .collect();
        for b in blocks {
            self.remove_block(b);
        }
        self.graph.remove(ids);
    }

    /// Unity's default new Visual Effect Graph ("Simple Loop"): a fountain of soft dots
    /// that fade and shrink over their life.
    pub fn simple_loop() -> Self {
        let mut a = VisualEffectAsset::empty();
        let spawn = a
            .graph
            .add(GraphNode::new(0, SPAWN, Vector2::new(0.0, -560.0)));
        let init = a.graph.add(
            GraphNode::new(0, INITIALIZE, Vector2::new(0.0, -400.0))
                .with_option("Capacity", "256")
                .with_option("Space", "Local"),
        );
        let update = a
            .graph
            .add(GraphNode::new(0, UPDATE, Vector2::new(0.0, -140.0)));
        let output = a.graph.add(
            GraphNode::new(0, OUTPUT, Vector2::new(0.0, 140.0))
                .with_option("Blend Mode", "Additive")
                .with_option("Orient", "Face Camera Plane"),
        );
        a.graph.connect(spawn, "Flow Out", init, "Flow In");
        a.graph.connect(init, "Flow Out", update, "Flow In");
        a.graph.connect(update, "Flow Out", output, "Flow In");
        let b = a.add_block(spawn, "Constant Spawn Rate");
        a.block_mut(b)
            .expect("block")
            .set_value("Rate", Vector4::splat(32.0));
        a.add_block(init, "Set Velocity Random");
        a.add_block(init, "Set Lifetime Random");
        let b = a.add_block(init, "Set Size");
        a.block_mut(b)
            .expect("block")
            .set_value("Size", Vector4::splat(0.15));
        a.add_block(update, "Turbulence");
        let b = a.add_block(output, "Set Size over Life");
        a.block_mut(b).expect("block").curve = AnimationCurve::linear(0.0, 1.0, 1.0, 0.0);
        let b = a.add_block(output, "Set Color over Life");
        a.block_mut(b).expect("block").gradient = Gradient {
            color_keys: vec![
                GradientColorKey {
                    color: Color::rgb(0.4, 0.8, 1.0),
                    time: 0.0,
                },
                GradientColorKey {
                    color: Color::rgb(0.2, 0.3, 1.0),
                    time: 1.0,
                },
            ],
            alpha_keys: vec![
                GradientAlphaKey {
                    alpha: 1.0,
                    time: 0.0,
                },
                GradientAlphaKey {
                    alpha: 0.0,
                    time: 1.0,
                },
            ],
            mode: Default::default(),
        };
        a
    }
}
