//! The Animator window (Unity's): an Animator Controller's layers and parameters, its state
//! machines as a graph — states, sub-state machines, Entry / Any State / Exit / Up nodes and
//! transitions — blend trees as a graph of their motions, and an inspector for the
//! selection. In Play mode it follows the selected Animator: the playing states and
//! transitions show progress, parameters show live values you can change, and blend trees
//! show their live weights.
//!
//! Edits save to the `.controller` straight away, as in Unity; ⌘Z / ⇧⌘Z over the window
//! undo and redo them.
//! Reference: <https://docs.unity3d.com/Manual/AnimatorWindow.html>

use crate::{EditorState, project::AssetPayload};
use egui::{
    Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2, pos2, vec2,
};
use oxide_animation::{ActiveTransition, StateMachineBehaviours};
use oxide_assets::{
    AnimationClip, AnimatorController, AnimatorControllerLayer, AnimatorLayerBlendingMode,
    AnimatorState, AnimatorStateMachine, AssetServer, BlendTree, BlendTreeType, Condition,
    ConditionMode, Handle, InterruptionSource, Motion, Parameter, ParameterType, StateBehaviour,
    Transition, TransitionSource, TransitionTarget,
};
use oxide_engine::Engine;
use oxide_math::Vector2;
use std::collections::HashMap;

const NODE: Vec2 = Vec2::new(170.0, 40.0);
const SMALL: Vec2 = Vec2::new(120.0, 32.0);
const BLUE: Color32 = Color32::from_rgb(70, 150, 255);
const SEL: Color32 = Color32::from_rgb(90, 170, 255);
const ORANGE: Color32 = Color32::from_rgb(200, 120, 50);
const MUTE: Color32 = Color32::from_rgb(220, 80, 80);
const SOLO: Color32 = Color32::from_rgb(110, 200, 110);
const WARN: Color32 = Color32::from_rgb(230, 190, 90);

/// A node of the current graph.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Node {
    State(usize),
    Machine(usize),
    Entry,
    Any,
    Exit,
    Up,
}

#[derive(Clone, Debug, PartialEq)]
enum Sel {
    /// States and sub-machines of the current machine (several with box / ⌘-click).
    Nodes(Vec<Node>),
    /// A transition: the machine holding it (layer path) and its index there.
    Transition(Vec<usize>, usize),
    /// A blend tree node (child path from the state's motion) in the blend tree view.
    Tree(Vec<usize>),
}

/// A pending target choice: where the transition starts, the options, where to show them.
type Choose = (From, Vec<(String, TransitionTarget)>, Pos2);

/// Transitions drawn as one arrow: (from, to) and the (machine, index) of each.
type ArrowGroup = ((Node, Node), Vec<(Vec<usize>, usize)>);

/// Where "Make Transition" starts.
#[derive(Clone, Debug, PartialEq)]
enum From {
    State(String),
    Machine(String),
    Any,
    Entry,
}

#[derive(Clone, Debug)]
enum Drag {
    /// Moving nodes (graph positions at the start).
    Nodes(Vec<(Node, Vector2)>),
    Pan,
    Box(Pos2),
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum LeftTab {
    #[default]
    Layers,
    Parameters,
}

/// One undo step: the controller and which layer was shown.
#[derive(Clone)]
struct Step {
    c: AnimatorController,
    layer: usize,
}

pub struct AnimatorWindow {
    path: Option<String>,
    layer: usize,
    left_tab: LeftTab,
    /// Sub-state machine being shown (child indices from the layer's root machine).
    machine: Vec<usize>,
    /// Blend tree view: the state (index in the current machine) whose tree is shown.
    tree: Option<usize>,
    sel: Option<Sel>,
    pan: Vec2,
    zoom: f32,
    drag: Option<Drag>,
    making: Option<From>,
    /// A transition target to choose (sub-machine or Up node clicked while making one).
    choose: Option<Choose>,
    menu_at: Pos2,
    renaming_param: Option<(usize, String)>,
    renaming_layer: Option<(usize, String)>,
    param_search: String,
    clipboard: Vec<AnimatorState>,
    undo: Vec<Step>,
    redo: Vec<Step>,
    /// The controller before a drag or a burst of typing (one undo step for all of it).
    pending: Option<(AnimatorController, f64)>,
    gear: Option<usize>,
    /// Follow the selected object's Animator (Unity's Auto Live Link).
    live_link: bool,
    /// Frame the graph's nodes on the next draw (after opening or navigating).
    needs_frame: bool,
    show_left: bool,
    show_left_narrow: bool,
    /// Last drawn nodes (kind, name, screen rect) for the `editor_animator` MCP tool.
    drawn: Vec<(&'static str, String, Rect)>,
    error: Option<String>,
}

impl Default for AnimatorWindow {
    fn default() -> Self {
        Self {
            path: None,
            layer: 0,
            left_tab: LeftTab::Layers,
            machine: vec![],
            tree: None,
            sel: None,
            pan: vec2(20.0, 20.0),
            zoom: 1.0,
            drag: None,
            making: None,
            choose: None,
            menu_at: Pos2::ZERO,
            renaming_param: None,
            renaming_layer: None,
            param_search: String::new(),
            clipboard: vec![],
            undo: vec![],
            redo: vec![],
            pending: None,
            gear: None,
            live_link: true,
            needs_frame: true,
            show_left: true,
            show_left_narrow: false,
            drawn: vec![],
            error: None,
        }
    }
}

impl AnimatorWindow {
    pub fn open(&mut self, path: &str) {
        if self.path.as_deref() != Some(path) {
            *self = Self {
                path: Some(path.to_owned()),
                live_link: self.live_link,
                clipboard: std::mem::take(&mut self.clipboard),
                ..Default::default()
            };
        }
    }

    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    /// MCP control: open a controller, pick a layer, go into a sub-state machine (by
    /// names from the layer root), open a state's blend tree, select a node or transition.
    pub fn control(
        &mut self,
        assets: &AssetServer,
        args: &serde_json::Value,
    ) -> Result<(), String> {
        let st = |k: &str| args.get(k).and_then(serde_json::Value::as_str);
        if let Some(p) = st("open") {
            self.open(p);
        }
        let Some(path) = self.path.clone() else {
            return Err("no controller open (pass `open`)".into());
        };
        let h: Handle<AnimatorController> = Handle::from_path(path);
        let c = assets
            .controllers
            .get(&h)
            .ok_or("controller not loaded yet: show the window first")?;
        if let Some(l) = args.get("layer") {
            let li = match l {
                serde_json::Value::Number(n) => n.as_u64().map(|n| n as usize),
                serde_json::Value::String(n) => c.layer_index(n),
                _ => None,
            }
            .filter(|&i| i < c.layers.len())
            .ok_or("no such layer")?;
            self.layer = li;
            self.navigate(vec![], None);
        }
        let root = &c.layers[source_layer(c, self.layer)].state_machine;
        if let Some(names) = args.get("machine").and_then(serde_json::Value::as_array) {
            let mut path = vec![];
            for n in names.iter().filter_map(serde_json::Value::as_str) {
                let m = root.machine(&path).ok_or("bad machine path")?;
                let k = m
                    .state_machines
                    .iter()
                    .position(|x| x.name == n)
                    .ok_or_else(|| format!("no sub-state machine {n}"))?;
                path.push(k);
            }
            self.navigate(path, None);
        }
        let m = root.machine(&self.machine).ok_or("bad machine")?;
        if let Some(n) = st("open_tree") {
            let i = m
                .state_index(n)
                .ok_or_else(|| format!("no state {n} here"))?;
            if m.states[i].motion.as_tree().is_none() {
                return Err(format!("{n} has no blend tree"));
            }
            self.navigate(self.machine.clone(), Some(i));
            self.sel = Some(Sel::Tree(vec![]));
        }
        if args.get("close_tree").and_then(serde_json::Value::as_bool) == Some(true) {
            self.navigate(self.machine.clone(), None);
        }
        if let Some(n) = st("select") {
            let node = match n {
                "Entry" => Node::Entry,
                "Any State" => Node::Any,
                "Exit" => Node::Exit,
                _ => m
                    .state_index(n)
                    .map(Node::State)
                    .or_else(|| {
                        m.state_machines
                            .iter()
                            .position(|x| x.name == n)
                            .map(Node::Machine)
                    })
                    .ok_or_else(|| format!("no state or sub-state machine {n} here"))?,
            };
            self.sel = Some(Sel::Nodes(vec![node]));
        }
        if let Some(t) = args
            .get("select_transition")
            .and_then(serde_json::Value::as_u64)
        {
            if (t as usize) >= m.transitions.len() {
                return Err("no such transition in this machine".into());
            }
            self.sel = Some(Sel::Transition(self.machine.clone(), t as usize));
        }
        if args.get("frame").and_then(serde_json::Value::as_bool) == Some(true) {
            self.needs_frame = true;
        }
        Ok(())
    }

    /// The window's state for MCP: controller, layer, machine path, blend tree, selection and
    /// the nodes drawn last frame with their screen rectangles (UI points).
    pub fn state_json(&self, assets: &AssetServer) -> serde_json::Value {
        let c = self.path.as_ref().and_then(|p| {
            let h: Handle<AnimatorController> = Handle::from_path(p.clone());
            assets.controllers.get(&h)
        });
        let machine: Vec<String> = c
            .and_then(|c| {
                let root = &c.layers.get(source_layer(c, self.layer))?.state_machine;
                Some(
                    (1..=self.machine.len())
                        .filter_map(|d| root.machine(&self.machine[..d]).map(|m| m.name.clone()))
                        .collect(),
                )
            })
            .unwrap_or_default();
        let sel = match &self.sel {
            Some(Sel::Nodes(n)) => serde_json::json!({ "nodes": format!("{n:?}") }),
            Some(Sel::Transition(h, i)) => serde_json::json!({ "transition": i, "holder": h }),
            Some(Sel::Tree(p)) => serde_json::json!({ "tree_node": p }),
            None => serde_json::Value::Null,
        };
        serde_json::json!({
            "path": self.path,
            "layer": self.layer,
            "layers": c.map(|c| c.layers.iter().map(|l| l.name.clone()).collect::<Vec<_>>()),
            "machine": machine,
            "blend_tree_state": self.tree,
            "selection": sel,
            "zoom": self.zoom,
            "undo_steps": self.undo.len(),
            "nodes": self.drawn.iter().map(|(k, n, r)| serde_json::json!({
                "kind": k, "name": n,
                "rect": [r.min.x, r.min.y, r.max.x, r.max.y],
                "center": [r.center().x, r.center().y],
            })).collect::<Vec<_>>(),
        })
    }

    fn navigate(&mut self, machine: Vec<usize>, tree: Option<usize>) {
        self.machine = machine;
        self.tree = tree;
        self.sel = None;
        self.making = None;
        self.choose = None;
        self.pan = vec2(20.0, 20.0);
        self.needs_frame = true;
    }
}

fn unique(name: &str, taken: &[String]) -> String {
    if !taken.iter().any(|t| t == name) {
        return name.to_owned();
    }
    (1..)
        .map(|i| format!("{name} {i}"))
        .find(|n| !taken.contains(n))
        .unwrap_or_else(|| name.to_owned())
}

// ---- live view ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct LayerLive {
    /// Playing state's name and normalized time.
    current: Option<(String, f32)>,
    /// Incoming state's name and transition progress.
    next: Option<(String, f32)>,
    transition: Option<ActiveTransition>,
}

#[derive(Clone, Default)]
struct Live {
    layers: Vec<LayerLive>,
    params: HashMap<String, f32>,
    weights: Vec<f32>,
}

/// The selected object's Animator in Play mode, if it plays `path`.
fn live_of(st: &EditorState, engine: &Engine, path: &str) -> Option<Live> {
    if st.play == crate::PlayState::Edit {
        return None;
    }
    let id = *st.selection.last()?;
    let a = engine
        .world
        .get_component::<oxide_animation::Animator>(id)?;
    if a.controller.as_ref()?.path() != path {
        return None;
    }
    Some(Live {
        layers: a
            .rt
            .layers
            .iter()
            .map(|l| LayerLive {
                current: l
                    .current
                    .as_ref()
                    .map(|_| (l.info.name.clone(), l.info.normalized_time)),
                next: l.next.as_ref().map(|(_, e, d)| {
                    (
                        l.next_info
                            .as_ref()
                            .map(|i| i.name.clone())
                            .unwrap_or_default(),
                        (e / d.max(1e-4)).clamp(0.0, 1.0),
                    )
                }),
                transition: l.transition.clone(),
            })
            .collect(),
        params: a.rt.params.clone(),
        weights: (0..a.layer_count())
            .map(|i| a.get_layer_weight(i))
            .collect(),
    })
}

// ---- entry point -------------------------------------------------------------------------

pub fn window_ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    // Follow the selection: an object with an Animator shows its controller (Unity).
    if st.animator.live_link
        && let Some(&id) = st.selection.last()
        && let Some(a) = engine.world.get_component::<oxide_animation::Animator>(id)
        && let Some(c) = &a.controller
        && st.animator.path() != Some(c.path())
        && !c.path().starts_with("runtime:")
    {
        let p = c.path().to_owned();
        st.animator.open(&p);
    }
    let Some(path) = st.animator.path.clone() else {
        ui.weak("Select an object with an Animator, or double-click an Animator Controller.");
        return;
    };
    let assets = engine.world.resource_mut::<AssetServer>();
    if let Err(e) = assets.ensure_loaded(&path) {
        ui.colored_label(Color32::from_rgb(230, 120, 100), e);
        return;
    }
    let h: Handle<AnimatorController> = Handle::from_path(path.clone());
    let Some(mut c) = assets.controllers.get(&h).cloned() else {
        return;
    };
    let editable = path.ends_with(".controller");
    if c.layers.is_empty() {
        c.base();
    }
    let before = c.clone();
    let live = live_of(st, engine, &path);
    let w = &mut st.animator;
    w.layer = w.layer.min(c.layers.len() - 1);
    // A machine path from another layer (or a deleted machine) falls back to the root.
    if c.layers[source_layer(&c, w.layer)]
        .state_machine
        .machine(&w.machine)
        .is_none()
    {
        w.navigate(vec![], None);
    }

    // Undo / redo (the window's own history; the scene's ⌘Z is left alone).
    let hovered = ui.rect_contains_pointer(ui.max_rect());
    if hovered && editable {
        crate::shortcuts::claim_keys(ui.ctx());
        let (undo, redo) = ui.input(|i| {
            let mut out = (false, false);
            for e in &i.events {
                if let egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } = e
                    && modifiers.command
                {
                    match key {
                        egui::Key::Z if modifiers.shift => out.1 = true,
                        egui::Key::Z => out.0 = true,
                        egui::Key::Y => out.1 = true,
                        _ => {}
                    }
                }
            }
            out
        });
        if undo && let Some(s) = w.undo.pop() {
            w.redo.push(Step {
                c: c.clone(),
                layer: w.layer,
            });
            c = s.c;
            w.layer = s.layer.min(c.layers.len().saturating_sub(1));
            w.sel = None;
        } else if redo && let Some(s) = w.redo.pop() {
            w.undo.push(Step {
                c: c.clone(),
                layer: w.layer,
            });
            c = s.c;
            w.layer = s.layer.min(c.layers.len().saturating_sub(1));
            w.sel = None;
        }
    }
    let undo_base = c.clone();

    let mut live_params: Vec<(String, f32)> = vec![];
    let assets = engine.world.resource::<AssetServer>();
    let registry = engine.world.try_resource::<StateMachineBehaviours>();
    let world = &engine.world;

    let narrow_ui = ui.available_width() < 820.0;
    // Toolbar: breadcrumb and live link.
    ui.horizontal(|ui| {
        ui.label(RichText::new(path.rsplit('/').next().unwrap_or(&path)).strong());
        ui.separator();
        breadcrumb(ui, w, &c);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let toggle = if narrow_ui {
                &mut w.show_left_narrow
            } else {
                &mut w.show_left
            };
            ui.toggle_value(toggle, "Layers / Parameters")
                .on_hover_text("Show or hide the Layers and Parameters panel");
            ui.toggle_value(&mut w.live_link, "Auto Live Link")
                .on_hover_text("Follow the selected object's Animator");
            if !editable {
                ui.weak("override controller: edit its base controller");
            }
            if let Some(e) = &w.error {
                ui.colored_label(Color32::from_rgb(230, 120, 100), e);
            }
        });
    });
    ui.separator();

    // Narrow docks: the inspector goes under the graph and the Layers / Parameters panel
    // can be hidden (toolbar toggle), so the graph keeps room.
    let total = ui.available_width();
    let narrow = total < 820.0;
    let show_left = if narrow {
        w.show_left_narrow
    } else {
        w.show_left
    };
    ui.add_enabled_ui(editable, |ui| {
        if show_left {
            egui::Panel::left(egui::Id::new(if narrow {
                "animator-left-n"
            } else {
                "animator-left"
            }))
            .resizable(true)
            .default_size(if narrow {
                170.0
            } else {
                (total * 0.2).clamp(190.0, 300.0)
            })
            .size_range(150.0..=420.0)
            .show(ui, |ui| {
                left_panel(ui, w, &mut c, assets, live.as_ref(), &mut live_params);
            });
        }
        let inspector_panel =
            |ui: &mut egui::Ui, w: &mut AnimatorWindow, c: &mut AnimatorController| {
                // Fields fit the panel instead of widening it.
                ui.spacing_mut().text_edit_width = (ui.available_width() * 0.5).max(60.0);
                ui.spacing_mut().combo_width = (ui.available_width() * 0.4).max(60.0);
                // Wide rows scroll sideways instead of growing the panel.
                egui::ScrollArea::both()
                    .id_salt("animator-props")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        inspector(ui, w, c, assets, world, registry, live.as_ref());
                    });
            };
        if narrow {
            egui::Panel::bottom(egui::Id::new("animator-bottom"))
                .resizable(true)
                .default_size(ui.available_height() * 0.42)
                .size_range(80.0..=ui.available_height() * 0.8)
                .show(ui, |ui| inspector_panel(ui, w, &mut c));
        } else {
            egui::Panel::right(egui::Id::new("animator-right"))
                .resizable(true)
                .default_size((total * 0.27).clamp(250.0, 420.0))
                .size_range(240.0..=560.0)
                .show(ui, |ui| inspector_panel(ui, w, &mut c));
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                let size = ui.available_size();
                if w.tree.is_some() {
                    tree_graph(ui, w, &mut c, assets, live.as_ref(), size, &mut live_params);
                } else {
                    graph(ui, w, &mut c, live.as_ref(), size, assets);
                }
            });
    });

    // Undo steps: drags and typing bursts become one step each.
    let now = ui.input(|i| i.time);
    if c != undo_base && editable {
        let typing = ui.ctx().egui_wants_keyboard_input();
        let busy = w.drag.is_some() || typing;
        match &mut w.pending {
            Some((_, t)) => *t = now,
            None => w.pending = Some((undo_base.clone(), now)),
        }
        if !busy {
            flush_undo(w);
        }
    } else if let Some((_, t)) = &w.pending
        && w.drag.is_none()
        && (now - *t > 0.6 || !ui.ctx().egui_wants_keyboard_input())
    {
        flush_undo(w);
    }
    if c != before && editable {
        match engine
            .world
            .resource_mut::<AssetServer>()
            .save_animator_controller(&path, &c)
        {
            Ok(_) => st.animator.error = None,
            Err(e) => st.animator.error = Some(e),
        }
    }
    // Parameter changes in Play mode go to the live Animator, not the asset.
    if !live_params.is_empty()
        && let Some(&id) = st.selection.last()
        && let Some(a) = engine
            .world
            .get_component_mut::<oxide_animation::Animator>(id)
    {
        for (n, v) in live_params {
            a.rt.params.insert(n, v);
        }
    }
}

fn flush_undo(w: &mut AnimatorWindow) {
    if let Some((c, _)) = w.pending.take() {
        w.undo.push(Step { c, layer: w.layer });
        if w.undo.len() > 200 {
            w.undo.remove(0);
        }
        w.redo.clear();
    }
}

/// The layer whose state machine `li` shows (a synced layer shows its source's).
fn source_layer(c: &AnimatorController, li: usize) -> usize {
    match c.layers.get(li).and_then(|l| l.synced_layer_index) {
        Some(s) if s < c.layers.len() && s != li => s,
        _ => li,
    }
}

fn breadcrumb(ui: &mut egui::Ui, w: &mut AnimatorWindow, c: &AnimatorController) {
    let li = source_layer(c, w.layer);
    let root = &c.layers[li].state_machine;
    let mut go: Option<(Vec<usize>, Option<usize>)> = None;
    if ui
        .selectable_label(
            w.machine.is_empty() && w.tree.is_none(),
            &c.layers[w.layer].name,
        )
        .clicked()
    {
        go = Some((vec![], None));
    }
    for d in 1..=w.machine.len() {
        if let Some(m) = root.machine(&w.machine[..d]) {
            ui.label("›");
            if ui
                .selectable_label(d == w.machine.len() && w.tree.is_none(), &m.name)
                .clicked()
            {
                go = Some((w.machine[..d].to_vec(), None));
            }
        }
    }
    if let Some(t) = w.tree
        && let Some(s) = root.machine(&w.machine).and_then(|m| m.states.get(t))
    {
        ui.label("›");
        let _ = ui.selectable_label(true, format!("{} (Blend Tree)", s.name));
    }
    if let Some((m, t)) = go {
        w.navigate(m, t);
    }
}

// ---- left panel: layers and parameters -------------------------------------------------

fn left_panel(
    ui: &mut egui::Ui,
    w: &mut AnimatorWindow,
    c: &mut AnimatorController,
    assets: &AssetServer,
    live: Option<&Live>,
    live_params: &mut Vec<(String, f32)>,
) {
    ui.spacing_mut().text_edit_width = (ui.available_width() * 0.6).max(60.0);
    ui.spacing_mut().combo_width = (ui.available_width() * 0.5).max(60.0);
    ui.horizontal(|ui| {
        ui.selectable_value(&mut w.left_tab, LeftTab::Layers, "Layers");
        ui.selectable_value(&mut w.left_tab, LeftTab::Parameters, "Parameters");
    });
    ui.separator();
    egui::ScrollArea::vertical()
        .id_salt("animator-left-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| match w.left_tab {
            LeftTab::Layers => layers_ui(ui, w, c, assets, live),
            LeftTab::Parameters => params_ui(ui, w, c, live, live_params),
        });
}

fn layers_ui(
    ui: &mut egui::Ui,
    w: &mut AnimatorWindow,
    c: &mut AnimatorController,
    assets: &AssetServer,
    live: Option<&Live>,
) {
    if ui.button("+ Add Layer").clicked() {
        let names: Vec<String> = c.layers.iter().map(|l| l.name.clone()).collect();
        let name = unique("New Layer", &names);
        c.layers.push(AnimatorControllerLayer::new(&name));
        w.layer = c.layers.len() - 1;
        w.navigate(vec![], None);
    }
    ui.add_space(4.0);
    let mut action: Option<(usize, &str)> = None;
    let n = c.layers.len();
    for i in 0..n {
        let selected = w.layer == i;
        let frame = egui::Frame::group(ui.style())
            .fill(if selected {
                Color32::from_rgb(44, 62, 88)
            } else {
                ui.visuals().faint_bg_color
            })
            .inner_margin(6.0);
        let mut name_resp: Option<egui::Response> = None;
        frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                if let Some((ri, text)) = &mut w.renaming_layer
                    && *ri == i
                {
                    let r = ui.add(egui::TextEdit::singleline(text).desired_width(120.0));
                    if !r.has_focus() && !r.lost_focus() {
                        r.request_focus();
                    }
                    if r.lost_focus() {
                        let t = text.trim().to_owned();
                        if !t.is_empty() {
                            c.layers[i].name = t;
                        }
                        w.renaming_layer = None;
                    }
                } else {
                    let r = ui.add(
                        egui::Label::new(RichText::new(&c.layers[i].name).strong())
                            .sense(Sense::click()),
                    );
                    if r.clicked() {
                        w.layer = i;
                        w.navigate(vec![], None);
                    }
                    if r.double_clicked() {
                        w.renaming_layer = Some((i, c.layers[i].name.clone()));
                    }
                    name_resp = Some(r);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let gear = ui
                        .add(egui::Button::new("⚙").small())
                        .on_hover_text("Layer settings");
                    if gear.clicked() {
                        w.gear = if w.gear == Some(i) { None } else { Some(i) };
                    }
                    // Unity's badges: M(ask), S(ync), A(dditive), IK.
                    let l = &c.layers[i];
                    let badge = |ui: &mut egui::Ui, t: &str, tip: &str| {
                        ui.label(RichText::new(t).small().color(Color32::from_gray(170)))
                            .on_hover_text(tip);
                    };
                    if l.ik_pass {
                        badge(ui, "IK", "IK Pass");
                    }
                    if l.blending_mode == AnimatorLayerBlendingMode::Additive {
                        badge(ui, "A", "Additive");
                    }
                    if l.synced_layer_index.is_some() {
                        badge(ui, "S", "Synced");
                    }
                    if l.avatar_mask_asset.is_some() || l.avatar_mask.is_some() {
                        badge(ui, "M", "Avatar Mask");
                    }
                });
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("Weight").small());
                ui.spacing_mut().slider_width = (ui.available_width() - 60.0).max(40.0);
                if i == 0 {
                    // The base layer always plays at full weight (Unity).
                    let mut one = 1.0f32;
                    ui.add_enabled(false, egui::Slider::new(&mut one, 0.0..=1.0));
                } else if let Some(lw) = live.and_then(|l| l.weights.get(i)) {
                    let mut v = *lw;
                    ui.add_enabled(false, egui::Slider::new(&mut v, 0.0..=1.0))
                        .on_hover_text("Live weight (set by scripts in Play mode)");
                } else {
                    ui.add(egui::Slider::new(
                        &mut c.layers[i].default_weight,
                        0.0..=1.0,
                    ));
                }
            });
            if w.gear == Some(i) {
                ui.separator();
                layer_settings(ui, c, i, assets);
            }
        });
        let Some(resp) = name_resp else {
            continue;
        };
        resp.context_menu(|ui| {
            if ui.button("Rename").clicked() {
                w.renaming_layer = Some((i, c.layers[i].name.clone()));
                ui.close();
            }
            if ui
                .add_enabled(i > 1, egui::Button::new("Move Up"))
                .clicked()
            {
                action = Some((i, "up"));
                ui.close();
            }
            if ui
                .add_enabled(i > 0 && i + 1 < n, egui::Button::new("Move Down"))
                .clicked()
            {
                action = Some((i, "down"));
                ui.close();
            }
            if ui.add_enabled(n > 1, egui::Button::new("Delete")).clicked() {
                action = Some((i, "delete"));
                ui.close();
            }
        });
        ui.add_space(3.0);
    }
    if let Some((i, what)) = action {
        match what {
            "up" => swap_layers(c, i, i - 1),
            "down" => swap_layers(c, i, i + 1),
            _ => delete_layer(c, i),
        }
        w.layer = 0;
        w.gear = None;
        w.navigate(vec![], None);
    }
}

fn swap_layers(c: &mut AnimatorController, a: usize, b: usize) {
    c.layers.swap(a, b);
    for l in &mut c.layers {
        l.synced_layer_index = l.synced_layer_index.map(|s| {
            if s == a {
                b
            } else if s == b {
                a
            } else {
                s
            }
        });
    }
}

fn delete_layer(c: &mut AnimatorController, i: usize) {
    c.layers.remove(i);
    for l in &mut c.layers {
        l.synced_layer_index = match l.synced_layer_index {
            Some(s) if s == i => None,
            Some(s) if s > i => Some(s - 1),
            s => s,
        };
    }
}

/// The gear popup's settings (Unity's layer settings).
fn layer_settings(ui: &mut egui::Ui, c: &mut AnimatorController, i: usize, assets: &AssetServer) {
    let names: Vec<String> = c.layers.iter().map(|l| l.name.clone()).collect();
    let n = c.layers.len();
    let l = &mut c.layers[i];
    egui::Grid::new(("layer-settings", i))
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            if i > 0 {
                ui.label("Weight");
                ui.add(egui::Slider::new(&mut l.default_weight, 0.0..=1.0));
                ui.end_row();
            }
            ui.label("Mask");
            let current = l.avatar_mask_asset.as_ref().map(|m| m.path().to_owned());
            egui::ComboBox::from_id_salt(("mask", i))
                .selected_text(
                    current
                        .as_deref()
                        .map_or("None (Avatar Mask)", |p| p.rsplit('/').next().unwrap_or(p)),
                )
                .width((ui.available_width() - 4.0).max(70.0))
                .show_ui(ui, |ui| {
                    if ui.selectable_label(current.is_none(), "None").clicked() {
                        l.avatar_mask_asset = None;
                        l.avatar_mask = None;
                    }
                    let masks = assets.paths_of_type("AvatarMask");
                    if masks.is_empty() {
                        ui.weak("No Avatar Masks. Create one: Create -> Animation -> Avatar Mask.");
                    }
                    for m in masks {
                        let name = m.rsplit('/').next().unwrap_or(&m).to_owned();
                        if ui
                            .selectable_label(current.as_deref() == Some(m.as_str()), name)
                            .on_hover_text(&m)
                            .clicked()
                        {
                            l.avatar_mask = assets
                                .masks
                                .handle_of(&m)
                                .and_then(|h| assets.masks.get(&h))
                                .cloned();
                            l.avatar_mask_asset = Some(Handle::from_path(m));
                        }
                    }
                });
            ui.end_row();
            ui.label("Blending");
            egui::ComboBox::from_id_salt(("blend", i))
                .selected_text(format!("{:?}", l.blending_mode))
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut l.blending_mode,
                        AnimatorLayerBlendingMode::Override,
                        "Override",
                    );
                    ui.selectable_value(
                        &mut l.blending_mode,
                        AnimatorLayerBlendingMode::Additive,
                        "Additive",
                    );
                });
            ui.end_row();
            if i > 0 {
                ui.label("Sync");
                let mut sync = l.synced_layer_index.is_some();
                if ui.checkbox(&mut sync, "").changed() {
                    l.synced_layer_index = sync.then_some(0);
                }
                ui.end_row();
                if let Some(src) = l.synced_layer_index {
                    ui.label("Source Layer");
                    egui::ComboBox::from_id_salt(("sync-src", i))
                        .selected_text(names.get(src).cloned().unwrap_or_default())
                        .show_ui(ui, |ui| {
                            for (k, nm) in names.iter().enumerate().take(n) {
                                if k != i {
                                    ui.selectable_value(
                                        l.synced_layer_index.as_mut().expect("synced"),
                                        k,
                                        nm,
                                    );
                                }
                            }
                        });
                    ui.end_row();
                    ui.label("Timing");
                    ui.checkbox(&mut l.synced_layer_affects_timing, "")
                        .on_hover_text("This layer's motions affect the timing");
                    ui.end_row();
                }
            }
            ui.label("IK Pass");
            ui.checkbox(&mut l.ik_pass, "");
            ui.end_row();
        });
}

fn params_ui(
    ui: &mut egui::Ui,
    w: &mut AnimatorWindow,
    c: &mut AnimatorController,
    live: Option<&Live>,
    live_params: &mut Vec<(String, f32)>,
) {
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut w.param_search)
                .hint_text("Search")
                .desired_width(ui.available_width() - 34.0),
        );
        ui.menu_button("+", |ui| {
            for (label, kind) in [
                ("Float", ParameterType::Float),
                ("Int", ParameterType::Int),
                ("Bool", ParameterType::Bool),
                ("Trigger", ParameterType::Trigger),
            ] {
                if ui.button(label).clicked() {
                    let names: Vec<String> = c.parameters.iter().map(|p| p.name.clone()).collect();
                    let name = unique(&format!("New {label}"), &names);
                    c.parameters.push(Parameter {
                        name: name.clone(),
                        kind,
                        default: 0.0,
                    });
                    w.renaming_param = Some((c.parameters.len() - 1, name));
                    ui.close();
                }
            }
        });
    });
    if live.is_some() {
        ui.weak("Live values of the selected Animator.");
    }
    ui.add_space(4.0);
    let search = w.param_search.to_lowercase();
    let mut remove = None;
    let mut rename: Option<(String, String)> = None;
    let mut dup = None;
    for i in 0..c.parameters.len() {
        if !search.is_empty() && !c.parameters[i].name.to_lowercase().contains(&search) {
            continue;
        }
        let p = &mut c.parameters[i];
        ui.horizontal(|ui| {
            if let Some((ri, text)) = &mut w.renaming_param
                && *ri == i
            {
                let r = ui.add(egui::TextEdit::singleline(text).desired_width(120.0));
                if !r.has_focus() && !r.lost_focus() {
                    r.request_focus();
                }
                if r.lost_focus() {
                    let t = text.trim().to_owned();
                    if !t.is_empty() && t != p.name {
                        rename = Some((p.name.clone(), t));
                    }
                    w.renaming_param = None;
                }
            } else {
                let r = ui.add(egui::Label::new(&p.name).sense(Sense::click()));
                if r.double_clicked() {
                    w.renaming_param = Some((i, p.name.clone()));
                }
                r.context_menu(|ui| {
                    if ui.button("Rename").clicked() {
                        w.renaming_param = Some((i, p.name.clone()));
                        ui.close();
                    }
                    if ui.button("Duplicate").clicked() {
                        dup = Some(i);
                        ui.close();
                    }
                    if ui.button("Delete").clicked() {
                        remove = Some(i);
                        ui.close();
                    }
                });
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Play mode: the live value (edits go to the Animator, not the asset).
                let (mut v, is_live) = match live.and_then(|l| l.params.get(&p.name)) {
                    Some(v) => (*v, true),
                    None => (p.default, false),
                };
                let changed = match p.kind {
                    ParameterType::Float => ui
                        .add(egui::DragValue::new(&mut v).speed(0.01).max_decimals(3))
                        .changed(),
                    ParameterType::Int => {
                        let mut n = v as i64;
                        let ch = ui.add(egui::DragValue::new(&mut n)).changed();
                        v = n as f32;
                        ch
                    }
                    ParameterType::Bool => {
                        let mut on = v != 0.0;
                        let ch = ui.checkbox(&mut on, "").changed();
                        v = f32::from(u8::from(on));
                        ch
                    }
                    ParameterType::Trigger => {
                        // Unity draws triggers as round buttons.
                        let on = v != 0.0;
                        let (r, painter) = ui.allocate_painter(vec2(16.0, 16.0), Sense::click());
                        let col = if on { BLUE } else { Color32::from_gray(70) };
                        painter.circle_filled(r.rect.center(), 6.0, col);
                        painter.circle_stroke(
                            r.rect.center(),
                            6.0,
                            Stroke::new(1.0, Color32::from_gray(140)),
                        );
                        if r.clicked() {
                            v = if on { 0.0 } else { 1.0 };
                        }
                        r.clicked()
                    }
                };
                if changed {
                    if is_live {
                        live_params.push((p.name.clone(), v));
                    } else {
                        p.default = v;
                    }
                }
            });
        });
    }
    if c.parameters.is_empty() {
        ui.weak("No parameters. Click + to add Float, Int, Bool or Trigger parameters.");
    }
    if let Some((old, new)) = rename {
        let names: Vec<String> = c.parameters.iter().map(|p| p.name.clone()).collect();
        let new = unique(&new, &names);
        rename_parameter(c, &old, &new);
    }
    if let Some(i) = dup {
        let mut p = c.parameters[i].clone();
        let names: Vec<String> = c.parameters.iter().map(|p| p.name.clone()).collect();
        p.name = unique(&p.name, &names);
        c.parameters.insert(i + 1, p);
    }
    if let Some(i) = remove {
        c.parameters.remove(i);
    }
}

/// Renames a parameter and every reference to it (conditions, blend trees, state
/// parameters), as Unity does.
fn rename_parameter(c: &mut AnimatorController, old: &str, new: &str) {
    let fix = |s: &mut String| {
        if s == old {
            *s = new.to_owned();
        }
    };
    fn motion(m: &mut Motion, f: &dyn Fn(&mut String)) {
        if let Some(t) = m.as_tree_mut() {
            f(&mut t.blend_parameter);
            f(&mut t.blend_parameter_y);
            for ch in &mut t.children {
                f(&mut ch.direct_blend_parameter);
                motion(&mut ch.motion, f);
            }
        }
    }
    fn machine(m: &mut AnimatorStateMachine, f: &dyn Fn(&mut String)) {
        for t in &mut m.transitions {
            for cd in &mut t.conditions {
                f(&mut cd.parameter);
            }
        }
        for s in &mut m.states {
            f(&mut s.speed_parameter);
            f(&mut s.time_parameter);
            f(&mut s.mirror_parameter);
            f(&mut s.cycle_offset_parameter);
            motion(&mut s.motion, f);
        }
        for sm in &mut m.state_machines {
            machine(sm, f);
        }
    }
    for p in &mut c.parameters {
        fix(&mut p.name);
    }
    for l in &mut c.layers {
        machine(&mut l.state_machine, &fix);
        for m in &mut l.synced_motions {
            motion(&mut m.motion, &fix);
        }
    }
}

// ---- state machine graph ------------------------------------------------------------------

fn arrow(painter: &egui::Painter, a: Pos2, b: Pos2, color: Color32, width: f32, heads: usize) {
    painter.line_segment([a, b], Stroke::new(width, color));
    let m = a + (b - a) * 0.5;
    let dir = (b - a).normalized();
    let n = vec2(-dir.y, dir.x);
    for k in 0..heads.clamp(1, 3) {
        let c = m + dir * (k as f32 * 9.0 - (heads.min(3) - 1) as f32 * 4.5);
        painter.add(egui::Shape::convex_polygon(
            vec![
                c + dir * 7.0,
                c - dir * 5.0 + n * 5.0,
                c - dir * 5.0 - n * 5.0,
            ],
            color,
            Stroke::NONE,
        ));
    }
}

/// Point on `r`'s border toward `to`.
fn border_point(r: Rect, to: Pos2) -> Pos2 {
    let c = r.center();
    let d = to - c;
    if d.length() < 1e-3 {
        return c;
    }
    let sx = if d.x.abs() > 1e-3 {
        (r.width() / 2.0) / d.x.abs()
    } else {
        f32::INFINITY
    };
    let sy = if d.y.abs() > 1e-3 {
        (r.height() / 2.0) / d.y.abs()
    } else {
        f32::INFINITY
    };
    c + d * sx.min(sy)
}

fn dist_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    (a + ab * t).distance(p)
}

/// Graph ↔ screen mapping.
#[derive(Clone, Copy)]
struct View {
    origin: Pos2,
    pan: Vec2,
    zoom: f32,
}

impl View {
    fn to_screen(self, p: Vector2) -> Pos2 {
        self.origin + (vec2(p.x, p.y) + self.pan) * self.zoom
    }
    fn to_graph(self, p: Pos2) -> Vector2 {
        let g = (p - self.origin) / self.zoom - self.pan;
        Vector2::new(g.x, g.y)
    }
    fn rect(self, p: Vector2, size: Vec2) -> Rect {
        Rect::from_min_size(self.to_screen(p), size * self.zoom)
    }
}

fn node_pos(m: &AnimatorStateMachine, n: &Node) -> Vector2 {
    match n {
        Node::State(i) => m.states.get(*i).map_or(Vector2::ZERO, |s| s.position),
        Node::Machine(i) => m
            .state_machines
            .get(*i)
            .map_or(Vector2::ZERO, |s| s.position),
        Node::Entry => m.entry_position,
        Node::Any => m.any_state_position,
        Node::Exit => m.exit_position,
        Node::Up => m.parent_position,
    }
}

fn set_node_pos(m: &mut AnimatorStateMachine, n: &Node, p: Vector2) {
    match n {
        Node::State(i) => {
            if let Some(s) = m.states.get_mut(*i) {
                s.position = p;
            }
        }
        Node::Machine(i) => {
            if let Some(s) = m.state_machines.get_mut(*i) {
                s.position = p;
            }
        }
        Node::Entry => m.entry_position = p,
        Node::Any => m.any_state_position = p,
        Node::Exit => m.exit_position = p,
        Node::Up => m.parent_position = p,
    }
}

fn node_size(n: &Node) -> Vec2 {
    match n {
        Node::State(_) | Node::Machine(_) => NODE,
        _ => SMALL,
    }
}

/// Which node of the current machine shows a state or machine name: itself, the child
/// machine containing it, or the Up node (anywhere else in the layer).
fn node_for(root: &AnimatorStateMachine, here: &[usize], name: &str, machine: bool) -> Node {
    let m = root.machine(here).expect("current machine");
    if !machine && let Some(i) = m.state_index(name) {
        return Node::State(i);
    }
    for (k, sm) in m.state_machines.iter().enumerate() {
        if sm.name == name
            || sm.all_names().iter().any(|n| n == name)
                && (machine || sm.find_state(name).is_some())
        {
            return Node::Machine(k);
        }
    }
    Node::Up
}

/// The arrows of the current view: (from node, to node, holder machine, index).
fn view_transitions(
    root: &AnimatorStateMachine,
    here: &[usize],
) -> Vec<(Node, Node, Vec<usize>, usize)> {
    let mut out = vec![];
    for (holder, t) in root.all_transitions_indexed() {
        let inside = holder.starts_with(here);
        if !inside {
            continue;
        }
        let direct = holder.len() == here.len();
        // Source node.
        let from = if direct {
            if t.is_any_state() {
                Node::Any
            } else if t.is_entry() {
                Node::Entry
            } else if let Some(f) = &t.from {
                node_for(root, here, f, t.source == TransitionSource::StateMachine)
            } else {
                continue;
            }
        } else {
            // Held by a descendant: shown from that child machine, if it leaves it.
            if t.is_any_state() || t.is_entry() {
                continue;
            }
            Node::Machine(holder[here.len()])
        };
        let to = match t.target {
            TransitionTarget::Exit => {
                if direct {
                    Node::Exit
                } else {
                    continue;
                }
            }
            TransitionTarget::State => node_for(root, here, &t.to, false),
            TransitionTarget::StateMachine => node_for(root, here, &t.to, true),
        };
        if !direct && to == from {
            continue;
        }
        out.push((from, to, holder.clone(), t.1));
    }
    out
}

trait Indexed {
    fn all_transitions_indexed(&self) -> Vec<(Vec<usize>, IndexedT<'_>)>;
}

/// A transition with its index in its machine.
struct IndexedT<'a>(&'a Transition, usize);

impl std::ops::Deref for IndexedT<'_> {
    type Target = Transition;
    fn deref(&self) -> &Transition {
        self.0
    }
}

impl Indexed for AnimatorStateMachine {
    fn all_transitions_indexed(&self) -> Vec<(Vec<usize>, IndexedT<'_>)> {
        let mut out = vec![];
        fn walk<'a>(
            m: &'a AnimatorStateMachine,
            path: &mut Vec<usize>,
            out: &mut Vec<(Vec<usize>, IndexedT<'a>)>,
        ) {
            out.extend(
                m.transitions
                    .iter()
                    .enumerate()
                    .map(|(i, t)| (path.clone(), IndexedT(t, i))),
            );
            for (i, c) in m.state_machines.iter().enumerate() {
                path.push(i);
                walk(c, path, out);
                path.pop();
            }
        }
        walk(self, &mut vec![], &mut out);
        out
    }
}

fn grid(painter: &egui::Painter, rect: Rect, v: &View) {
    painter.rect_filled(rect, 0.0, Color32::from_gray(36));
    for (step, col) in [(20.0, 41), (100.0, 47)] {
        let s = step * v.zoom;
        if s < 6.0 {
            continue;
        }
        let off = (v.pan * v.zoom).x.rem_euclid(s);
        let mut x = rect.left() + off;
        while x < rect.right() {
            painter.line_segment(
                [pos2(x, rect.top()), pos2(x, rect.bottom())],
                Stroke::new(1.0, Color32::from_gray(col)),
            );
            x += s;
        }
        let off = (v.pan * v.zoom).y.rem_euclid(s);
        let mut y = rect.top() + off;
        while y < rect.bottom() {
            painter.line_segment(
                [pos2(rect.left(), y), pos2(rect.right(), y)],
                Stroke::new(1.0, Color32::from_gray(col)),
            );
            y += s;
        }
    }
}

/// Pan with the middle button (or Alt / Space + drag) and zoom with the wheel, as in Unity.
fn pan_zoom(ui: &egui::Ui, w: &mut AnimatorWindow, resp: &egui::Response, rect: Rect) {
    if resp.hovered() {
        let (scroll, pointer, zoom_delta) = ui.input(|i| {
            (
                i.smooth_scroll_delta.y,
                i.pointer.hover_pos(),
                i.zoom_delta(),
            )
        });
        let factor = if zoom_delta != 1.0 {
            zoom_delta
        } else if scroll != 0.0 {
            (scroll * 0.0015).exp()
        } else {
            1.0
        };
        if factor != 1.0
            && let Some(p) = pointer
        {
            let old = w.zoom;
            w.zoom = (w.zoom * factor).clamp(0.25, 2.0);
            // Keep the point under the cursor in place.
            let g = (p - rect.min) / old - w.pan;
            w.pan = (p - rect.min) / w.zoom - g;
        }
    }
}

fn node_shape(
    painter: &egui::Painter,
    r: Rect,
    zoom: f32,
    fill: Color32,
    label: &str,
    selected: bool,
    machine: bool,
) {
    let stroke = Stroke::new(
        if selected { 2.5 } else { 1.0 },
        if selected {
            SEL
        } else {
            Color32::from_gray(18)
        },
    );
    if machine {
        // Unity draws sub-state machines as a hexagon.
        let c = r.center();
        let (hw, hh, cut) = (r.width() / 2.0, r.height() / 2.0, 14.0 * zoom);
        let pts = vec![
            pos2(c.x - hw + cut, c.y - hh),
            pos2(c.x + hw - cut, c.y - hh),
            pos2(c.x + hw, c.y),
            pos2(c.x + hw - cut, c.y + hh),
            pos2(c.x - hw + cut, c.y + hh),
            pos2(c.x - hw, c.y),
        ];
        painter.add(egui::Shape::convex_polygon(pts, fill, stroke));
    } else {
        painter.rect_filled(r, 5.0 * zoom, fill);
        painter.rect_stroke(r, 5.0 * zoom, stroke, StrokeKind::Outside);
    }
    painter.text(
        r.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(13.0 * zoom.max(0.6)),
        Color32::WHITE,
    );
}

#[allow(clippy::too_many_lines)]
fn graph(
    ui: &mut egui::Ui,
    w: &mut AnimatorWindow,
    c: &mut AnimatorController,
    live: Option<&Live>,
    size: Vec2,
    assets: &AssetServer,
) {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    pan_zoom(ui, w, &resp, rect);
    let view = View {
        origin: rect.min,
        pan: w.pan,
        zoom: w.zoom,
    };
    grid(&painter, rect, &view);
    let synced = source_layer(c, w.layer) != w.layer;
    let li = source_layer(c, w.layer);
    let here = w.machine.clone();
    let layer_live = live.and_then(|l| l.layers.get(w.layer)).cloned();
    let root_snapshot = c.layers[li].state_machine.clone();
    let m_snapshot = root_snapshot.machine(&here).expect("valid machine").clone();
    let in_sub = !here.is_empty();

    let mut nodes: Vec<Node> = (0..m_snapshot.states.len()).map(Node::State).collect();
    nodes.extend((0..m_snapshot.state_machines.len()).map(Node::Machine));
    nodes.extend([Node::Entry, Node::Any, Node::Exit]);
    if in_sub {
        nodes.push(Node::Up);
    }
    if w.needs_frame {
        w.needs_frame = false;
        frame(w, &m_snapshot, &nodes, rect.size());
    }
    let view = View {
        origin: rect.min,
        pan: w.pan,
        zoom: w.zoom,
    };
    let rect_of = |n: &Node| view.rect(node_pos(&m_snapshot, n), node_size(n));
    let pointer = resp.hover_pos().or(resp.interact_pointer_pos());
    let hit_node =
        pointer.and_then(|p| nodes.iter().rev().find(|n| rect_of(n).contains(p)).cloned());

    // Transitions under the nodes. Several between the same pair draw as one arrow with
    // three heads (Unity).
    let arrows = view_transitions(&root_snapshot, &here);
    let mut groups: Vec<ArrowGroup> = vec![];
    for (f, t, h, i) in &arrows {
        match groups.iter_mut().find(|g| g.0 == (f.clone(), t.clone())) {
            Some(g) => g.1.push((h.clone(), *i)),
            None => groups.push(((f.clone(), t.clone()), vec![(h.clone(), *i)])),
        }
    }
    let mut hit_transition: Option<(Vec<usize>, usize)> = None;
    for ((f, t), list) in &groups {
        let (fr, tr) = (rect_of(f), rect_of(t));
        let dir = (tr.center() - fr.center()).normalized();
        let shift = vec2(-dir.y, dir.x) * 5.0 * view.zoom;
        let (a, b) = if f == t {
            // Self transition (Any State -> same state can't happen here): a small loop.
            (fr.right_top(), fr.right_bottom())
        } else {
            (
                border_point(fr, tr.center()) + shift,
                border_point(tr, fr.center()) + shift,
            )
        };
        let trs: Vec<&Transition> = list
            .iter()
            .filter_map(|(h, i)| root_snapshot.machine(h)?.transitions.get(*i))
            .collect();
        let selected = list
            .iter()
            .any(|(h, i)| w.sel == Some(Sel::Transition(h.clone(), *i)));
        let active = layer_live
            .as_ref()
            .and_then(|l| l.transition.as_ref())
            .is_some_and(|at| list.iter().any(|(h, i)| *h == at.machine && *i == at.index));
        let color = if selected || active {
            SEL
        } else if trs.iter().all(|t| t.mute) {
            MUTE
        } else if trs.iter().any(|t| t.solo) {
            SOLO
        } else {
            Color32::from_gray(200)
        };
        arrow(
            &painter,
            a,
            b,
            color,
            if selected || active { 2.5 } else { 1.5 },
            list.len().min(3),
        );
        if active && let Some((_, p)) = layer_live.as_ref().and_then(|l| l.next.as_ref()) {
            // Progress along the arrow.
            painter.circle_filled(a + (b - a) * *p, 4.0, BLUE);
        }
        if let Some(p) = pointer
            && hit_node.is_none()
            && dist_to_segment(p, a, b) < 6.0
        {
            hit_transition = Some(list[0].clone());
        }
    }
    // Entry -> default state.
    if let Some(d) = (!m_snapshot.states.is_empty())
        .then(|| Node::State(m_snapshot.default_state.min(m_snapshot.states.len() - 1)))
        .or_else(|| (!m_snapshot.state_machines.is_empty()).then_some(Node::Machine(0)))
    {
        let (er, dr) = (rect_of(&Node::Entry), rect_of(&d));
        arrow(
            &painter,
            border_point(er, dr.center()),
            border_point(dr, er.center()),
            ORANGE,
            1.5,
            1,
        );
    }

    // Nodes.
    let selected_nodes: Vec<Node> = match &w.sel {
        Some(Sel::Nodes(v)) => v.clone(),
        _ => vec![],
    };
    let live_names: Vec<(String, f32, bool)> = layer_live
        .iter()
        .flat_map(|l| {
            l.current
                .iter()
                .map(|(n, t)| (n.clone(), t.fract(), false))
                .chain(l.next.iter().map(|(n, p)| (n.clone(), *p, true)))
        })
        .collect();
    w.drawn.clear();
    for n in &nodes {
        let r = rect_of(n);
        let sel = selected_nodes.contains(n);
        w.drawn.push(match n {
            Node::State(i) => ("state", m_snapshot.states[*i].name.clone(), r),
            Node::Machine(i) => (
                "state_machine",
                m_snapshot.state_machines[*i].name.clone(),
                r,
            ),
            Node::Entry => ("entry", "Entry".into(), r),
            Node::Any => ("any_state", "Any State".into(), r),
            Node::Exit => ("exit", "Exit".into(), r),
            Node::Up => ("up", "(Up)".into(), r),
        });
        match n {
            Node::State(i) => {
                let s = &m_snapshot.states[*i];
                let default = *i == m_snapshot.default_state;
                let mut fill = if default {
                    ORANGE
                } else {
                    Color32::from_gray(88)
                };
                if synced
                    && c.layers[w.layer]
                        .synced_motion(&s.name)
                        .is_none_or(|m| *m == Motion::Empty)
                {
                    fill = fill.gamma_multiply(0.6);
                }
                let label = if s.motion.as_tree().is_some() {
                    format!("{}  ⏷", s.name)
                } else {
                    s.name.clone()
                };
                node_shape(&painter, r, view.zoom, fill, &label, sel, false);
                for (name, t, incoming) in &live_names {
                    if *name == s.name {
                        let bar = Rect::from_min_size(
                            pos2(r.left() + 4.0, r.bottom() - 7.0 * view.zoom),
                            vec2((r.width() - 8.0) * t.clamp(0.0, 1.0), 4.0 * view.zoom),
                        );
                        painter.rect_filled(
                            bar,
                            1.0,
                            if *incoming {
                                BLUE.gamma_multiply(0.6)
                            } else {
                                BLUE
                            },
                        );
                    }
                }
            }
            Node::Machine(i) => {
                let sm = &m_snapshot.state_machines[*i];
                let fill = Color32::from_gray(98);
                node_shape(&painter, r, view.zoom, fill, &sm.name, sel, true);
                // Live: a state inside is playing.
                if live_names
                    .iter()
                    .any(|(n, _, _)| sm.find_state(n).is_some())
                {
                    painter.rect_stroke(
                        r.shrink(3.0),
                        3.0,
                        Stroke::new(1.5, BLUE),
                        StrokeKind::Inside,
                    );
                }
            }
            Node::Entry => node_shape(
                &painter,
                r,
                view.zoom,
                Color32::from_rgb(90, 170, 90),
                "Entry",
                sel,
                false,
            ),
            Node::Any => node_shape(
                &painter,
                r,
                view.zoom,
                Color32::from_rgb(90, 170, 180),
                "Any State",
                sel,
                false,
            ),
            Node::Exit => node_shape(
                &painter,
                r,
                view.zoom,
                Color32::from_rgb(190, 80, 80),
                "Exit",
                sel,
                false,
            ),
            Node::Up => {
                let parent = if here.len() > 1 {
                    root_snapshot
                        .machine(&here[..here.len() - 1])
                        .map(|m| m.name.clone())
                        .unwrap_or_default()
                } else {
                    c.layers[w.layer].name.clone()
                };
                node_shape(
                    &painter,
                    r,
                    view.zoom,
                    Color32::from_gray(70),
                    &format!("(Up) {parent}"),
                    sel,
                    true,
                );
            }
        }
    }

    // Making a transition: rubber band to the pointer.
    if let Some(from) = &w.making {
        let fnode = match from {
            From::State(n) => m_snapshot.state_index(n).map(Node::State),
            From::Machine(n) => m_snapshot
                .state_machines
                .iter()
                .position(|m| &m.name == n)
                .map(Node::Machine),
            From::Any => Some(Node::Any),
            From::Entry => Some(Node::Entry),
        };
        if let (Some(fnode), Some(p)) = (fnode, pointer) {
            arrow(&painter, rect_of(&fnode).center(), p, WARN, 1.5, 1);
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            w.making = None;
        }
    }
    // Box selection.
    if let Some(Drag::Box(start)) = &w.drag
        && let Some(p) = pointer
    {
        let r = Rect::from_two_pos(*start, p);
        painter.rect_filled(r, 0.0, SEL.gamma_multiply(0.15));
        painter.rect_stroke(r, 0.0, Stroke::new(1.0, SEL), StrokeKind::Inside);
    }

    let editable_structure = !synced;
    let root = &mut c.layers[li].state_machine;
    let m = root.machine_mut(&here).expect("valid machine");
    let cmd = ui.input(|i| i.modifiers.command);

    // Clicks.
    if resp.clicked() {
        if let Some(from) = w.making.take() {
            if let Some(n) = &hit_node {
                finish_transition(
                    w,
                    &root_snapshot,
                    &here,
                    m,
                    from,
                    n,
                    pointer.unwrap_or_default(),
                );
            }
        } else if let Some(n) = &hit_node {
            let mut v = if cmd { selected_nodes.clone() } else { vec![] };
            if let Some(k) = v.iter().position(|x| x == n) {
                v.remove(k);
            } else {
                v.push(n.clone());
            }
            w.sel = Some(Sel::Nodes(v));
        } else if let Some((h, i)) = hit_transition.clone() {
            w.sel = Some(Sel::Transition(h, i));
        } else {
            w.sel = None;
        }
    }
    if resp.double_clicked()
        && let Some(n) = &hit_node
    {
        match n {
            Node::Machine(k) => {
                let mut p = here.clone();
                p.push(*k);
                w.navigate(p, None);
                return;
            }
            Node::Up => {
                let mut p = here.clone();
                p.pop();
                w.navigate(p, None);
                return;
            }
            Node::State(i) if m_snapshot.states[*i].motion.as_tree().is_some() => {
                w.navigate(here.clone(), Some(*i));
                w.sel = Some(Sel::Tree(vec![]));
                return;
            }
            _ => {}
        }
    }

    // Drags: nodes move, empty space box-selects, middle / Alt / Space drag pans.
    if resp.drag_started() {
        let (alt, middle, space) = ui.input(|i| {
            (
                i.modifiers.alt,
                i.pointer.middle_down(),
                i.key_down(egui::Key::Space),
            )
        });
        let at = ui
            .input(|i| i.pointer.press_origin())
            .filter(|p| rect.contains(*p));
        let on = at.and_then(|p| nodes.iter().rev().find(|n| rect_of(n).contains(p)).cloned());
        w.drag = if alt || middle || space {
            Some(Drag::Pan)
        } else if let Some(n) = on {
            let mut group = if selected_nodes.contains(&n) {
                selected_nodes.clone()
            } else {
                vec![n.clone()]
            };
            if !selected_nodes.contains(&n) {
                w.sel = Some(Sel::Nodes(vec![n.clone()]));
            }
            group.retain(|_| editable_structure);
            Some(Drag::Nodes(
                group.iter().map(|g| (g.clone(), node_pos(m, g))).collect(),
            ))
        } else {
            at.map(Drag::Box)
        };
    }
    if resp.dragged() {
        let d = resp.drag_delta() / view.zoom;
        match &mut w.drag {
            Some(Drag::Pan) => w.pan += d,
            Some(Drag::Nodes(list)) => {
                for (n, p) in list.iter_mut() {
                    *p += Vector2::new(d.x, d.y);
                    set_node_pos(m, n, *p);
                }
            }
            _ => {}
        }
    }
    if resp.drag_stopped() {
        match w.drag.take() {
            Some(Drag::Nodes(list)) => {
                // Snap to the grid, as Unity does.
                for (n, p) in list {
                    let s = Vector2::new((p.x / 10.0).round() * 10.0, (p.y / 10.0).round() * 10.0);
                    set_node_pos(m, &n, s);
                }
            }
            Some(Drag::Box(start)) => {
                if let Some(p) = pointer {
                    let r = Rect::from_two_pos(start, p);
                    let picked: Vec<Node> = nodes
                        .iter()
                        .filter(|n| matches!(n, Node::State(_) | Node::Machine(_)))
                        .filter(|n| r.intersects(rect_of(n)))
                        .cloned()
                        .collect();
                    w.sel = Some(Sel::Nodes(picked));
                }
            }
            _ => {}
        }
    }

    // Keys.
    if resp.hovered() && editable_structure {
        let (del, copy, paste, dup, frame_all, frame_sel) = ui.input(|i| {
            let key = |k: egui::Key, cmd: bool| {
                i.events.iter().any(|e| {
                    matches!(e, egui::Event::Key { key, pressed: true, modifiers, .. }
                        if *key == k && modifiers.command == cmd)
                })
            };
            (
                key(egui::Key::Delete, false) || key(egui::Key::Backspace, false),
                key(egui::Key::C, true),
                key(egui::Key::V, true),
                key(egui::Key::D, true),
                key(egui::Key::A, false),
                key(egui::Key::F, false),
            )
        });
        if del && !ui.ctx().egui_wants_keyboard_input() {
            delete_selection(w, root, &here);
        }
        if copy || dup {
            let m = root.machine(&here).expect("valid");
            w.clipboard = selected_nodes
                .iter()
                .filter_map(|n| match n {
                    Node::State(i) => m.states.get(*i).cloned(),
                    _ => None,
                })
                .collect();
        }
        if paste || dup {
            paste_states(w, root, &here);
        }
        if frame_all || frame_sel {
            let m = root.machine(&here).expect("valid");
            let pick: Vec<Node> = if frame_sel && !selected_nodes.is_empty() {
                selected_nodes.clone()
            } else {
                nodes.clone()
            };
            frame(w, m, &pick, rect.size());
        }
    }
    if resp.hovered() {
        crate::shortcuts::claim_keys(ui.ctx());
    }

    // Dropping clips from the Project window creates states (Unity).
    if editable_structure
        && let Some(payload) = resp.dnd_release_payload::<AssetPayload>()
        && let Some(p) = pointer
        && (payload.0.ends_with(".anim")
            || payload.0.contains(".glb#")
            || payload.0.contains(".gltf#"))
    {
        let base = Motion::Clip(Handle::from_path(payload.0.clone())).label();
        let name = unique(&base, &root.all_names());
        let g = view.to_graph(p - NODE * view.zoom / 2.0);
        let m = root.machine_mut(&here).expect("valid");
        let s = m.add_state(&name, Motion::Clip(Handle::from_path(payload.0.clone())));
        s.position = g;
        let i = m.states.len() - 1;
        w.sel = Some(Sel::Nodes(vec![Node::State(i)]));
        let _ = m;
    }

    // Context menus.
    if resp.secondary_clicked()
        && let Some(p) = pointer
    {
        w.menu_at = p;
        if let Some(n) = &hit_node
            && !selected_nodes.contains(n)
        {
            w.sel = Some(Sel::Nodes(vec![n.clone()]));
        }
    }
    let menu_node = nodes
        .iter()
        .rev()
        .find(|n| rect_of(n).contains(w.menu_at))
        .cloned();
    let menu_g = view.to_graph(w.menu_at);
    resp.context_menu(|ui| {
        if !editable_structure {
            ui.weak("Synced layer: edit the states in the source layer.");
            return;
        }
        let m = root.machine(&here).expect("valid").clone();
        match &menu_node {
            Some(Node::State(i)) => {
                let name = m.states[*i].name.clone();
                if ui.button("Make Transition").clicked() {
                    w.making = Some(From::State(name.clone()));
                    ui.close();
                }
                if ui
                    .add_enabled(
                        m.default_state != *i,
                        egui::Button::new("Set as Layer Default State"),
                    )
                    .clicked()
                {
                    root.machine_mut(&here).expect("valid").default_state = *i;
                    ui.close();
                }
                if ui.button("Copy").clicked() {
                    w.clipboard = vec![m.states[*i].clone()];
                    ui.close();
                }
                if m.states[*i].motion.as_tree().is_none()
                    && ui.button("Create new BlendTree in State").clicked()
                {
                    root.machine_mut(&here).expect("valid").states[*i].motion =
                        Motion::tree(BlendTree::default());
                    ui.close();
                }
                if ui.button("Delete").clicked() {
                    delete_selection(w, root, &here);
                    ui.close();
                }
            }
            Some(Node::Machine(k)) => {
                let name = m.state_machines[*k].name.clone();
                if ui.button("Make Transition").clicked() {
                    w.making = Some(From::Machine(name));
                    ui.close();
                }
                if ui.button("Open").clicked() {
                    let mut p = here.clone();
                    p.push(*k);
                    w.navigate(p, None);
                    ui.close();
                }
                if ui.button("Delete").clicked() {
                    delete_selection(w, root, &here);
                    ui.close();
                }
            }
            Some(Node::Any) => {
                if ui.button("Make Transition").clicked() {
                    w.making = Some(From::Any);
                    ui.close();
                }
            }
            Some(Node::Entry) => {
                if ui.button("Make Transition").clicked() {
                    w.making = Some(From::Entry);
                    ui.close();
                }
            }
            _ => {
                ui.menu_button("Create State", |ui| {
                    let mut add = |name: &str, motion: Motion| {
                        let name = unique(name, &root.all_names());
                        let m = root.machine_mut(&here).expect("valid");
                        let s = m.add_state(&name, motion);
                        s.position = menu_g;
                        w.sel = Some(Sel::Nodes(vec![Node::State(m.states.len() - 1)]));
                    };
                    if ui.button("Empty").clicked() {
                        add("New State", Motion::Empty);
                        ui.close();
                    }
                    ui.menu_button("From Clip", |ui| {
                        let clips = clip_paths(assets);
                        if clips.is_empty() {
                            ui.weak("No clips in the project.");
                        }
                        for p in clips {
                            let mo = Motion::Clip(Handle::from_path(p.clone()));
                            if ui.button(mo.label()).on_hover_text(&p).clicked() {
                                add(&mo.label(), mo);
                                ui.close();
                            }
                        }
                    });
                    if ui.button("From New Blend Tree").clicked() {
                        add("Blend Tree", Motion::tree(BlendTree::default()));
                        ui.close();
                    }
                });
                if ui.button("Create Sub-State Machine").clicked() {
                    let name = unique("New StateMachine", &root.all_names());
                    let m = root.machine_mut(&here).expect("valid");
                    m.add_state_machine(&name).position = menu_g;
                    w.sel = Some(Sel::Nodes(vec![Node::Machine(m.state_machines.len() - 1)]));
                    ui.close();
                }
                if ui
                    .add_enabled(!w.clipboard.is_empty(), egui::Button::new("Paste"))
                    .clicked()
                {
                    paste_states(w, root, &here);
                    ui.close();
                }
                if ui.button("Frame All (A)").clicked() {
                    let m = root.machine(&here).expect("valid");
                    frame(w, m, &nodes, rect.size());
                    ui.close();
                }
            }
        }
    });

    // Choosing a target inside a sub-machine or the parent (Unity's popup).
    if let Some((from, options, at)) = w.choose.clone() {
        let mut close = false;
        egui::Area::new(egui::Id::new("animator-choose"))
            .fixed_pos(at)
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.label(RichText::new("Transition to").strong());
                    for (name, target) in &options {
                        let label = match target {
                            TransitionTarget::StateMachine => format!("StateMachine › {name}"),
                            _ => format!("States › {name}"),
                        };
                        if ui.button(label).clicked() {
                            let m = root.machine_mut(&here).expect("valid");
                            add_transition(w, m, &here, from.clone(), name, *target);
                            close = true;
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
            });
        if close || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            w.choose = None;
        }
    }

    if hit_node.is_some() || hit_transition.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if nodes.len() <= 4 && m_snapshot.states.is_empty() {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "Right-click to create a state, or drag clips here from the Project window.",
            FontId::proportional(13.0),
            Color32::from_gray(140),
        );
    }
}

fn clip_paths(assets: &AssetServer) -> Vec<String> {
    let mut paths: Vec<String> = assets.paths_of_type("AnimationClip");
    paths.extend(assets.animations.iter().map(|(p, _)| p.to_owned()));
    paths.retain(|p| !p.starts_with("runtime:"));
    paths.sort();
    paths.dedup();
    paths
}

/// Zooms and pans so `nodes` fit (Unity's Frame All / Frame Selected).
fn frame(w: &mut AnimatorWindow, m: &AnimatorStateMachine, nodes: &[Node], size: Vec2) {
    let mut r = Rect::NOTHING;
    for n in nodes {
        let p = node_pos(m, n);
        r = r.union(Rect::from_min_size(pos2(p.x, p.y), node_size(n)));
    }
    if !r.is_positive() {
        return;
    }
    let r = r.expand(40.0);
    w.zoom = (size.x / r.width())
        .min(size.y / r.height())
        .clamp(0.25, 1.5);
    w.pan = (size / w.zoom - r.size()) / 2.0 - r.min.to_vec2();
}

fn finish_transition(
    w: &mut AnimatorWindow,
    root: &AnimatorStateMachine,
    here: &[usize],
    m: &mut AnimatorStateMachine,
    from: From,
    to: &Node,
    at: Pos2,
) {
    match to {
        Node::State(i) => {
            let name = m.states[*i].name.clone();
            add_transition(w, m, here, from, &name, TransitionTarget::State);
        }
        Node::Exit => {
            if matches!(from, From::State(_) | From::Machine(_)) {
                add_transition(w, m, here, from, "", TransitionTarget::Exit);
            }
        }
        Node::Machine(k) => {
            // A sub-machine: its Entry or one of its states (Unity's popup).
            let sm = &m.state_machines[*k];
            let mut options = vec![(sm.name.clone(), TransitionTarget::StateMachine)];
            options.extend(
                sm.flatten()
                    .into_iter()
                    .map(|(_, s)| (s.name.clone(), TransitionTarget::State)),
            );
            w.choose = Some((from, options, at));
        }
        Node::Up => {
            // The parent machine's states and machines.
            let parent = root.machine(&here[..here.len().saturating_sub(1)]);
            if let Some(pm) = parent {
                let mut options: Vec<(String, TransitionTarget)> = pm
                    .states
                    .iter()
                    .map(|s| (s.name.clone(), TransitionTarget::State))
                    .collect();
                options.extend(
                    pm.state_machines
                        .iter()
                        .map(|s| (s.name.clone(), TransitionTarget::StateMachine)),
                );
                w.choose = Some((from, options, at));
            }
        }
        _ => {}
    }
}

fn add_transition(
    w: &mut AnimatorWindow,
    m: &mut AnimatorStateMachine,
    here: &[usize],
    from: From,
    to: &str,
    target: TransitionTarget,
) {
    let mut t = Transition {
        to: to.to_owned(),
        target,
        ..Default::default()
    };
    match from {
        From::State(n) => {
            // Unity's defaults for state-to-state transitions.
            t.from = Some(n);
            t.has_exit_time = true;
            t.exit_time = 0.75;
        }
        From::Machine(n) => {
            t.from = Some(n);
            t.source = TransitionSource::StateMachine;
            t.duration = 0.0;
        }
        From::Any => {}
        From::Entry => {
            t.source = TransitionSource::Entry;
            t.duration = 0.0;
        }
    }
    m.transitions.push(t);
    w.sel = Some(Sel::Transition(here.to_vec(), m.transitions.len() - 1));
}

fn paste_states(w: &mut AnimatorWindow, root: &mut AnimatorStateMachine, here: &[usize]) {
    let mut added = vec![];
    for s in w.clipboard.clone() {
        let mut s = s;
        s.name = unique(&s.name, &root.all_names());
        s.position += Vector2::new(30.0, 30.0);
        let m = root.machine_mut(here).expect("valid");
        m.states.push(s);
        added.push(Node::State(m.states.len() - 1));
    }
    if !added.is_empty() {
        w.sel = Some(Sel::Nodes(added));
    }
}

fn delete_selection(w: &mut AnimatorWindow, root: &mut AnimatorStateMachine, here: &[usize]) {
    match w.sel.take() {
        Some(Sel::Nodes(nodes)) => {
            let m = root.machine(here).expect("valid");
            let mut states: Vec<usize> = nodes
                .iter()
                .filter_map(|n| match n {
                    Node::State(i) => Some(*i),
                    _ => None,
                })
                .collect();
            let mut machines: Vec<usize> = nodes
                .iter()
                .filter_map(|n| match n {
                    Node::Machine(i) => Some(*i),
                    _ => None,
                })
                .collect();
            let mut gone: Vec<String> = states
                .iter()
                .filter_map(|i| m.states.get(*i).map(|s| s.name.clone()))
                .collect();
            for k in &machines {
                if let Some(sm) = m.state_machines.get(*k) {
                    gone.push(sm.name.clone());
                    gone.extend(sm.all_names());
                }
            }
            states.sort_unstable_by(|a, b| b.cmp(a));
            machines.sort_unstable_by(|a, b| b.cmp(a));
            let m = root.machine_mut(here).expect("valid");
            for i in states {
                if i < m.states.len() {
                    m.states.remove(i);
                    if m.default_state > i {
                        m.default_state -= 1;
                    } else if m.default_state == i {
                        m.default_state = 0;
                    }
                }
            }
            m.default_state = m.default_state.min(m.states.len().saturating_sub(1));
            for k in machines {
                if k < m.state_machines.len() {
                    m.state_machines.remove(k);
                }
            }
            for n in gone {
                root.remove_references(&n);
            }
        }
        Some(Sel::Transition(h, i)) => {
            if let Some(m) = root.machine_mut(&h)
                && i < m.transitions.len()
            {
                m.transitions.remove(i);
            }
        }
        _ => {}
    }
}

// ---- blend tree graph --------------------------------------------------------------------

/// The motion a blend tree view edits: the state's own, or a synced layer's override.
fn view_motion<'a>(
    c: &'a mut AnimatorController,
    layer: usize,
    here: &[usize],
    state: usize,
) -> Option<&'a mut Motion> {
    let li = source_layer(c, layer);
    if li != layer {
        let name = c.layers[li]
            .state_machine
            .machine(here)?
            .states
            .get(state)?
            .name
            .clone();
        return Some(c.layers[layer].synced_motion_mut(&name));
    }
    Some(
        &mut c.layers[li]
            .state_machine
            .machine_mut(here)?
            .states
            .get_mut(state)?
            .motion,
    )
}

/// Lays out a tree left to right: (path, depth, row) for every node, depth first.
fn tree_layout(m: &Motion) -> Vec<(Vec<usize>, usize, f32)> {
    let mut out = vec![];
    let mut row = 0.0f32;
    fn walk(
        m: &Motion,
        path: &mut Vec<usize>,
        out: &mut Vec<(Vec<usize>, usize, f32)>,
        row: &mut f32,
    ) -> f32 {
        let depth = path.len();
        let idx = out.len();
        out.push((path.clone(), depth, 0.0));
        let my_row = match m.as_tree() {
            Some(t) if !t.children.is_empty() => {
                let mut rows = vec![];
                for (i, c) in t.children.iter().enumerate() {
                    path.push(i);
                    rows.push(walk(&c.motion, path, out, row));
                    path.pop();
                }
                (rows[0] + rows[rows.len() - 1]) / 2.0
            }
            _ => {
                let r = *row;
                *row += 1.0;
                r
            }
        };
        out[idx].2 = my_row;
        my_row
    }
    walk(m, &mut vec![], &mut out, &mut row);
    out
}

fn tree_graph(
    ui: &mut egui::Ui,
    w: &mut AnimatorWindow,
    c: &mut AnimatorController,
    assets: &AssetServer,
    live: Option<&Live>,
    size: Vec2,
    live_params: &mut Vec<(String, f32)>,
) {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    pan_zoom(ui, w, &resp, rect);
    let view = View {
        origin: rect.min,
        pan: w.pan,
        zoom: w.zoom,
    };
    grid(&painter, rect, &view);
    let params: HashMap<String, f32> = match live {
        Some(l) => l.params.clone(),
        None => c
            .parameters
            .iter()
            .map(|p| (p.name.clone(), p.default))
            .collect(),
    };
    let here = w.machine.clone();
    let Some(si) = w.tree else { return };
    let Some(motion) = view_motion(c, w.layer, &here, si) else {
        w.navigate(here, None);
        return;
    };
    let snapshot = motion.clone();
    let layout = tree_layout(&snapshot);
    if w.needs_frame {
        w.needs_frame = false;
        let rows = layout.iter().map(|l| l.2).fold(0.0f32, f32::max) + 1.0;
        let depth = layout.iter().map(|l| l.1).max().unwrap_or(0) as f32 + 1.0;
        let content = vec2(depth * 280.0, rows * 70.0);
        w.zoom = (size.x / (content.x + 80.0))
            .min(size.y / (content.y + 80.0))
            .clamp(0.4, 1.2);
        w.pan = (size / w.zoom - content) / 2.0;
    }
    let view = View {
        origin: rect.min,
        pan: w.pan,
        zoom: w.zoom,
    };
    const TW: f32 = 240.0;
    let pos_of = |depth: usize, row: f32| Vector2::new(depth as f32 * 280.0, row * 70.0);
    let size_of = |m: &Motion| match m.as_tree() {
        Some(t) if t.blend_type.is_2d() => vec2(TW, 72.0),
        Some(_) => vec2(TW, 54.0),
        None => vec2(TW, 40.0),
    };
    // Live weight of every node (product down the tree).
    let mut weight: HashMap<Vec<usize>, f32> = HashMap::new();
    weight.insert(vec![], 1.0);
    for (path, _, _) in &layout {
        let Some(m) = snapshot.at(path) else { continue };
        if let Some(t) = m.as_tree() {
            let ws = oxide_animation::tree_weights(t, &params);
            let base = weight.get(path).copied().unwrap_or(0.0);
            for (i, x) in ws.iter().enumerate() {
                let mut p = path.clone();
                p.push(i);
                weight.insert(p, base * x);
            }
        }
    }
    let pointer = resp.hover_pos().or(resp.interact_pointer_pos());
    let mut hit: Option<Vec<usize>> = None;
    // Edges.
    for (path, depth, row) in &layout {
        if path.is_empty() {
            continue;
        }
        let parent = &path[..path.len() - 1];
        let Some((_, pd, pr)) = layout.iter().find(|(p, _, _)| p == parent) else {
            continue;
        };
        let pm = snapshot.at(parent).expect("parent");
        let a = view.rect(pos_of(*pd, *pr), size_of(pm)).right_center();
        let b = view
            .rect(
                pos_of(*depth, *row),
                size_of(snapshot.at(path).expect("node")),
            )
            .left_center();
        let wgt = weight.get(path).copied().unwrap_or(0.0);
        let col = Color32::from_gray(90).lerp_to_gamma(BLUE, wgt.clamp(0.0, 1.0));
        painter.line_segment([a, b], Stroke::new(1.0 + 3.0 * wgt, col));
    }
    // Nodes.
    let sel_path = match &w.sel {
        Some(Sel::Tree(p)) => Some(p.clone()),
        _ => None,
    };
    let mut slider_edit: Option<(String, f32)> = None;
    w.drawn.clear();
    for (path, depth, row) in &layout {
        let m = snapshot.at(path).expect("node");
        let r = view.rect(pos_of(*depth, *row), size_of(m));
        w.drawn.push((
            if m.as_tree().is_some() {
                "blend_tree"
            } else {
                "motion"
            },
            format!("{path:?} {}", motion_name(assets, m)),
            r,
        ));
        let selected = sel_path.as_ref() == Some(path);
        let wgt = weight.get(path).copied().unwrap_or(0.0);
        match m.as_tree() {
            Some(t) => {
                node_shape(
                    &painter,
                    r,
                    view.zoom,
                    Color32::from_gray(80),
                    "",
                    selected,
                    false,
                );
                painter.text(
                    r.left_top() + vec2(8.0, 6.0) * view.zoom,
                    Align2::LEFT_TOP,
                    format!("{}  ({})", t.name, t.blend_type.label()),
                    FontId::proportional(12.5 * view.zoom.max(0.6)),
                    Color32::WHITE,
                );
                // Parameter sliders on the tree node (Unity's): they change the value used
                // to preview (the parameter's default, or the live value in Play mode).
                let names: Vec<&String> = match t.blend_type {
                    BlendTreeType::Direct => vec![],
                    bt if bt.is_2d() => vec![&t.blend_parameter, &t.blend_parameter_y],
                    _ => vec![&t.blend_parameter],
                };
                for (k, n) in names.iter().enumerate() {
                    if n.is_empty() {
                        continue;
                    }
                    let (lo, hi) = param_range(t, k);
                    let mut v = params.get(*n).copied().unwrap_or(0.0);
                    let row = r.left_top() + vec2(8.0, 24.0 + k as f32 * 20.0) * view.zoom;
                    painter.text(
                        row + vec2(0.0, 8.0 * view.zoom),
                        Align2::LEFT_CENTER,
                        n.as_str(),
                        FontId::proportional(11.0 * view.zoom.max(0.6)),
                        Color32::from_gray(200),
                    );
                    let label_w = 64.0 * view.zoom;
                    let sr = Rect::from_min_size(
                        row + vec2(label_w, 0.0),
                        vec2(r.width() - 16.0 - label_w, 16.0 * view.zoom),
                    );
                    let resp = ui.put(sr, egui::Slider::new(&mut v, lo..=hi).show_value(true));
                    if resp.changed() {
                        slider_edit = Some(((*n).clone(), v));
                    }
                }
            }
            None => {
                let fill = Color32::from_gray(60)
                    .lerp_to_gamma(Color32::from_rgb(60, 90, 140), wgt.clamp(0.0, 1.0));
                node_shape(
                    &painter,
                    r,
                    view.zoom,
                    fill,
                    &motion_name(assets, m),
                    selected,
                    false,
                );
                // Weight bar.
                let bar = Rect::from_min_size(
                    pos2(r.left() + 4.0, r.bottom() - 6.0 * view.zoom),
                    vec2((r.width() - 8.0) * wgt.clamp(0.0, 1.0), 3.0 * view.zoom),
                );
                painter.rect_filled(bar, 1.0, BLUE);
            }
        }
        if pointer.is_some_and(|p| r.contains(p)) {
            hit = Some(path.clone());
        }
    }
    if let Some((n, v)) = slider_edit {
        match live {
            Some(_) => live_params.push((n, v)),
            None => {
                if let Some(p) = c.parameters.iter_mut().find(|p| p.name == n) {
                    p.default = v;
                }
            }
        }
    }
    if resp.clicked() {
        w.sel = hit.clone().map(Sel::Tree);
    }
    if resp.drag_started() {
        w.drag = Some(Drag::Pan);
    }
    if resp.dragged() && matches!(w.drag, Some(Drag::Pan)) {
        w.pan += resp.drag_delta() / view.zoom;
    }
    if resp.drag_stopped() {
        w.drag = None;
    }
    if resp.secondary_clicked() {
        w.menu_at = pointer.unwrap_or_default();
        if let Some(h) = &hit {
            w.sel = Some(Sel::Tree(h.clone()));
        }
    }
    let target = match &w.sel {
        Some(Sel::Tree(p)) => p.clone(),
        _ => vec![],
    };
    let Some(motion) = view_motion(c, w.layer, &here, si) else {
        return;
    };
    resp.context_menu(|ui| {
        let is_tree = motion.at(&target).is_some_and(|m| m.as_tree().is_some());
        if is_tree {
            if ui.button("Add Motion").clicked() {
                if let Some(t) = motion.at_mut(&target).and_then(Motion::as_tree_mut) {
                    t.add_child(Motion::Empty);
                }
                ui.close();
            }
            if ui.button("Add Blend Tree").clicked() {
                if let Some(t) = motion.at_mut(&target).and_then(Motion::as_tree_mut) {
                    t.add_child(Motion::tree(BlendTree::default()));
                }
                ui.close();
            }
        }
        if !target.is_empty() && ui.button("Delete").clicked() {
            let (i, parent) = target.split_last().expect("non-empty");
            if let Some(t) = motion.at_mut(parent).and_then(Motion::as_tree_mut)
                && *i < t.children.len()
            {
                t.children.remove(*i);
            }
            w.sel = Some(Sel::Tree(parent.to_vec()));
            ui.close();
        }
        if ui.button("Back to State Machine").clicked() {
            w.navigate(here.clone(), None);
            ui.close();
        }
    });
    if resp.hovered() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        w.navigate(here, None);
    }
    if resp.hovered() {
        crate::shortcuts::claim_keys(ui.ctx());
    }
}

/// Slider range for a tree's parameter `k` (0 = x, 1 = y): its children's thresholds or
/// positions, like Unity's.
fn param_range(t: &BlendTree, k: usize) -> (f32, f32) {
    let vals: Vec<f32> = if t.blend_type == BlendTreeType::Simple1D {
        t.effective_thresholds()
    } else {
        t.children
            .iter()
            .map(|c| if k == 0 { c.position.x } else { c.position.y })
            .collect()
    };
    let lo = vals.iter().copied().fold(f32::INFINITY, f32::min);
    let hi = vals.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    if lo.is_finite() && hi > lo {
        (lo, hi)
    } else {
        (0.0, 1.0)
    }
}

// ---- inspector ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn inspector(
    ui: &mut egui::Ui,
    w: &mut AnimatorWindow,
    c: &mut AnimatorController,
    assets: &AssetServer,
    world: &oxide_scene::World,
    registry: Option<&StateMachineBehaviours>,
    live: Option<&Live>,
) {
    let params = c.parameters.clone();
    let li = source_layer(c, w.layer);
    let synced = li != w.layer;
    let here = w.machine.clone();
    match w.sel.clone() {
        Some(Sel::Tree(path)) => {
            let Some(si) = w.tree else { return };
            let pvals: HashMap<String, f32> = match live {
                Some(l) => l.params.clone(),
                None => params.iter().map(|p| (p.name.clone(), p.default)).collect(),
            };
            let Some(motion) = view_motion(c, w.layer, &here, si) else {
                return;
            };
            let Some(m) = motion.at_mut(&path) else {
                w.sel = None;
                return;
            };
            match m.as_tree_mut() {
                Some(t) => tree_inspector(ui, t, &params, assets, &pvals, &path),
                None => {
                    ui.label(RichText::new("Motion").strong());
                    ui.horizontal(|ui| {
                        crate::inspector::prop_label(ui, "Motion");
                        motion_picker(ui, ("leaf", path.clone()), assets, m, true);
                    });
                }
            }
        }
        Some(Sel::Nodes(nodes)) if nodes.len() == 1 => match &nodes[0] {
            Node::State(i) => {
                let root = &mut c.layers[li].state_machine;
                let Some(m) = root.machine(&here) else { return };
                let Some(s) = m.states.get(*i) else {
                    w.sel = None;
                    return;
                };
                let name = s.name.clone();
                if synced {
                    ui.label(RichText::new(format!("{name} (synced layer)")).strong());
                    ui.weak("This layer follows the source layer's states; set this layer's motion for the state:");
                    let layer = &mut c.layers[w.layer];
                    let m = layer.synced_motion_mut(&name);
                    ui.horizontal(|ui| {
                        crate::inspector::prop_label(ui, "Motion");
                        motion_picker(ui, "synced-motion", assets, m, true);
                    });
                    if m.as_tree().is_some() && ui.button("Edit Blend Tree").clicked() {
                        w.navigate(here.clone(), Some(*i));
                        w.sel = Some(Sel::Tree(vec![]));
                    }
                    return;
                }
                state_inspector(ui, w, c, li, &here, *i, assets, world, registry);
            }
            Node::Machine(k) => {
                let root = &mut c.layers[li].state_machine;
                let all = root.all_names();
                let Some(m) = root.machine_mut(&here) else {
                    return;
                };
                let Some(sm) = m.state_machines.get_mut(*k) else {
                    w.sel = None;
                    return;
                };
                let old = sm.name.clone();
                ui.label(RichText::new("Sub-State Machine").strong());
                let mut name = sm.name.clone();
                ui.horizontal(|ui| {
                    crate::inspector::prop_label(ui, "Name");
                    ui.text_edit_singleline(&mut name);
                });
                if name != old && !name.trim().is_empty() && !all.contains(&name) {
                    sm.name = name.clone();
                    root.rename_references(&old, &name);
                }
                let root = &mut c.layers[li].state_machine;
                let sm = root
                    .machine_mut(&here)
                    .and_then(|m| m.state_machines.get_mut(*k))
                    .expect("exists");
                ui.add_space(6.0);
                behaviours_ui(ui, &mut sm.behaviours, world, registry, &format!("smb{k}"));
                ui.add_space(6.0);
                if ui.button("Open").clicked() {
                    let mut p = here.clone();
                    p.push(*k);
                    w.navigate(p, None);
                }
            }
            Node::Entry | Node::Any => {
                let any = nodes[0] == Node::Any;
                ui.label(RichText::new(if any { "Any State" } else { "Entry" }).strong());
                ui.weak(if any {
                    "Transitions from Any State are checked from every state in the layer, before the state's own."
                } else {
                    "Entry transitions pick the first state when the machine is entered; without one that matches, the default state (orange) plays."
                });
                let root = &mut c.layers[li].state_machine;
                let m = root.machine_mut(&here).expect("valid");
                transition_list(ui, w, m, &here, |t| {
                    if any { t.is_any_state() } else { t.is_entry() }
                });
            }
            _ => {}
        },
        Some(Sel::Nodes(nodes)) if nodes.len() > 1 => {
            ui.label(format!("{} items selected", nodes.len()));
            ui.weak(
                "Drag to move them together; Delete removes them; ⌘C / ⌘V copy and paste states.",
            );
        }
        Some(Sel::Transition(h, ti)) => {
            transition_inspector(ui, w, c, li, &h, ti, assets, &params);
        }
        _ => {
            ui.label(RichText::new(&c.layers[w.layer].name).strong());
            ui.weak("Select a state, sub-state machine or transition.\n\n• Right-click the graph to create states and sub-state machines.\n• Right-click a state, choose Make Transition, then click the target.\n• Double-click a blend tree state or a sub-state machine to open it.\n• Drag clips here from the Project window.\n• Scroll to zoom; Alt+drag or middle-drag to pan; A frames all.");
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn param_toggle(
    ui: &mut egui::Ui,
    label: &str,
    active: &mut bool,
    param: &mut String,
    params: &[Parameter],
    kinds: &[ParameterType],
    salt: &str,
    value: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, label);
        if *active {
            param_combo(ui, (salt, "p"), params, param, |p| kinds.contains(&p.kind));
        } else {
            value(ui);
        }
        ui.checkbox(active, "Parameter")
            .on_hover_text("Drive this from a parameter");
    });
}

#[allow(clippy::too_many_arguments)]
fn state_inspector(
    ui: &mut egui::Ui,
    w: &mut AnimatorWindow,
    c: &mut AnimatorController,
    li: usize,
    here: &[usize],
    i: usize,
    assets: &AssetServer,
    world: &oxide_scene::World,
    registry: Option<&StateMachineBehaviours>,
) {
    let params = c.parameters.clone();
    let root = &mut c.layers[li].state_machine;
    let all = root.all_names();
    let m = root.machine_mut(here).expect("valid");
    let s = &mut m.states[i];
    let old = s.name.clone();
    ui.label(RichText::new("Animator State").strong());
    let mut name = s.name.clone();
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "Name");
        ui.text_edit_singleline(&mut name);
    });
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "Tag");
        ui.text_edit_singleline(&mut s.tag);
    });
    ui.separator();
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "Motion");
        motion_picker(ui, "state-motion", assets, &mut s.motion, true);
    });
    if s.motion.as_tree().is_some() && ui.button("Edit Blend Tree").clicked() {
        w.navigate(here.to_vec(), Some(i));
        w.sel = Some(Sel::Tree(vec![]));
        return;
    }
    let floats = [ParameterType::Float, ParameterType::Int];
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "Speed");
        ui.add(egui::DragValue::new(&mut s.speed).speed(0.01));
    });
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "   Multiplier");
        ui.add_enabled_ui(s.speed_parameter_active, |ui| {
            param_combo(ui, "speed-p", &params, &mut s.speed_parameter, |p| {
                floats.contains(&p.kind)
            });
        });
        ui.checkbox(&mut s.speed_parameter_active, "Parameter");
    });
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "Motion Time");
        ui.add_enabled_ui(s.time_parameter_active, |ui| {
            param_combo(ui, "time-p", &params, &mut s.time_parameter, |p| {
                floats.contains(&p.kind)
            });
        });
        ui.checkbox(&mut s.time_parameter_active, "Parameter");
    });
    let (mut mirror, mut mp, mut mpa) = (
        s.mirror,
        s.mirror_parameter.clone(),
        s.mirror_parameter_active,
    );
    param_toggle(
        ui,
        "Mirror",
        &mut mpa,
        &mut mp,
        &params,
        &[ParameterType::Bool],
        "mirror",
        |ui| {
            ui.checkbox(&mut mirror, "");
        },
    );
    (s.mirror, s.mirror_parameter, s.mirror_parameter_active) = (mirror, mp, mpa);
    let (mut co, mut cp, mut cpa) = (
        s.cycle_offset,
        s.cycle_offset_parameter.clone(),
        s.cycle_offset_parameter_active,
    );
    param_toggle(
        ui,
        "Cycle Offset",
        &mut cpa,
        &mut cp,
        &params,
        &floats,
        "cycle",
        |ui| {
            ui.add(egui::DragValue::new(&mut co).speed(0.01).range(0.0..=1.0));
        },
    );
    (
        s.cycle_offset,
        s.cycle_offset_parameter,
        s.cycle_offset_parameter_active,
    ) = (co, cp, cpa);
    ui.checkbox(&mut s.looping, "Loop Time");
    ui.checkbox(&mut s.foot_ik, "Foot IK");
    ui.checkbox(&mut s.write_defaults, "Write Defaults")
        .on_hover_text("Properties this state doesn't animate go back to their default values");
    let mut behaviours = std::mem::take(&mut s.behaviours);
    if name != old && !name.trim().is_empty() && !all.contains(&name) {
        s.name = name.clone();
        root.rename_references(&old, &name);
        // Synced layers keep their motion for the renamed state.
        for l in &mut c.layers {
            for sm in &mut l.synced_motions {
                if sm.state == old {
                    sm.state = name.clone();
                }
            }
        }
    }
    let root = &mut c.layers[li].state_machine;
    let state_name = root.machine(here).expect("valid").states[i].name.clone();
    ui.add_space(8.0);
    ui.label(RichText::new("Transitions").strong());
    let m = root.machine_mut(here).expect("valid");
    transition_list(ui, w, m, here, |t| {
        t.source == TransitionSource::State && t.from.as_deref() == Some(state_name.as_str())
    });
    ui.add_space(8.0);
    behaviours_ui(ui, &mut behaviours, world, registry, &format!("sb{i}"));
    let m = c.layers[li].state_machine.machine_mut(here).expect("valid");
    m.states[i].behaviours = behaviours;
}

/// Unity's transition list: order (priority), Solo and Mute.
fn transition_list(
    ui: &mut egui::Ui,
    w: &mut AnimatorWindow,
    m: &mut AnimatorStateMachine,
    here: &[usize],
    filter: impl Fn(&Transition) -> bool,
) {
    let idx: Vec<usize> = (0..m.transitions.len())
        .filter(|&i| filter(&m.transitions[i]))
        .collect();
    if idx.is_empty() {
        ui.weak("None.");
        return;
    }
    let mut swap = None;
    egui::Grid::new(("tlist", here.to_vec()))
        .num_columns(4)
        .striped(true)
        .show(ui, |ui| {
            ui.weak("Transition");
            ui.weak("Solo");
            ui.weak("Mute");
            ui.weak("");
            ui.end_row();
            for (k, &ti) in idx.iter().enumerate() {
                let t = &mut m.transitions[ti];
                let label = format!(
                    "{} -> {}",
                    if t.is_any_state() {
                        "Any State".to_owned()
                    } else if t.is_entry() {
                        "Entry".to_owned()
                    } else {
                        t.from.clone().unwrap_or_default()
                    },
                    if t.target == TransitionTarget::Exit {
                        "Exit".to_owned()
                    } else {
                        t.to.clone()
                    }
                );
                if ui.selectable_label(false, label).clicked() {
                    w.sel = Some(Sel::Transition(here.to_vec(), ti));
                }
                ui.checkbox(&mut t.solo, "");
                ui.checkbox(&mut t.mute, "");
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(k > 0, egui::Button::new("⏶").small())
                        .clicked()
                    {
                        swap = Some((ti, idx[k - 1]));
                    }
                    if ui
                        .add_enabled(k + 1 < idx.len(), egui::Button::new("⏷").small())
                        .clicked()
                    {
                        swap = Some((ti, idx[k + 1]));
                    }
                });
                ui.end_row();
            }
        });
    if let Some((a, b)) = swap {
        m.transitions.swap(a, b);
    }
}

#[allow(clippy::too_many_arguments)]
fn transition_inspector(
    ui: &mut egui::Ui,
    w: &mut AnimatorWindow,
    c: &mut AnimatorController,
    li: usize,
    holder: &[usize],
    ti: usize,
    assets: &AssetServer,
    params: &[Parameter],
) {
    let root = c.layers[li].state_machine.clone();
    let Some(m) = c.layers[li].state_machine.machine_mut(holder) else {
        w.sel = None;
        return;
    };
    let Some(t) = m.transitions.get_mut(ti) else {
        w.sel = None;
        return;
    };
    let from_label = if t.is_any_state() {
        "Any State".to_owned()
    } else if t.is_entry() {
        "Entry".to_owned()
    } else {
        t.from.clone().unwrap_or_default()
    };
    let to_label = match t.target {
        TransitionTarget::Exit => "Exit".to_owned(),
        TransitionTarget::StateMachine => format!("{} (StateMachine)", t.to),
        TransitionTarget::State => t.to.clone(),
    };
    ui.label(RichText::new(format!("{from_label} -> {to_label}")).strong());
    ui.horizontal(|ui| {
        ui.checkbox(&mut t.solo, "Solo");
        ui.checkbox(&mut t.mute, "Mute");
    });
    let simple = t.is_entry() || t.source == TransitionSource::StateMachine;
    if !simple {
        ui.checkbox(&mut t.has_exit_time, "Has Exit Time");
        egui::CollapsingHeader::new("Settings")
            .default_open(true)
            .show(ui, |ui| {
                ui.add_enabled_ui(t.has_exit_time, |ui| {
                    ui.horizontal(|ui| {
                        crate::inspector::prop_label(ui, "Exit Time");
                        ui.add(
                            egui::DragValue::new(&mut t.exit_time)
                                .speed(0.01)
                                .range(0.0..=100.0),
                        );
                    });
                });
                ui.horizontal(|ui| {
                    crate::inspector::prop_label(ui, "Fixed Duration");
                    ui.checkbox(&mut t.fixed_duration, "");
                });
                ui.horizontal(|ui| {
                    crate::inspector::prop_label(
                        ui,
                        if t.fixed_duration {
                            "Transition Duration (s)"
                        } else {
                            "Transition Duration (%)"
                        },
                    );
                    ui.add(
                        egui::DragValue::new(&mut t.duration)
                            .speed(0.01)
                            .range(0.0..=100.0),
                    );
                });
                ui.horizontal(|ui| {
                    crate::inspector::prop_label(ui, "Transition Offset");
                    ui.add(
                        egui::DragValue::new(&mut t.offset)
                            .speed(0.01)
                            .range(0.0..=1.0),
                    );
                });
                ui.horizontal(|ui| {
                    crate::inspector::prop_label(ui, "Interruption Source");
                    egui::ComboBox::from_id_salt("interrupt")
                        .selected_text(t.interruption_source.label())
                        .show_ui(ui, |ui| {
                            for s in InterruptionSource::ALL {
                                ui.selectable_value(&mut t.interruption_source, s, s.label());
                            }
                        });
                });
                ui.add_enabled_ui(
                    matches!(
                        t.interruption_source,
                        InterruptionSource::Source
                            | InterruptionSource::SourceThenDestination
                            | InterruptionSource::DestinationThenSource
                    ),
                    |ui| {
                        ui.horizontal(|ui| {
                            crate::inspector::prop_label(ui, "Ordered Interruption");
                            ui.checkbox(&mut t.ordered_interruption, "");
                        });
                    },
                );
                if t.is_any_state() {
                    ui.horizontal(|ui| {
                        crate::inspector::prop_label(ui, "Can Transition To Self");
                        ui.checkbox(&mut t.can_transition_to_self, "");
                    });
                }
            });
        timeline(ui, t, &root, assets, params);
    }
    ui.add_space(6.0);
    ui.label(RichText::new("Conditions").strong());
    let mut remove = None;
    for (k, cond) in t.conditions.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            param_combo(ui, ("cond-p", k), params, &mut cond.parameter, |_| true);
            let kind = params
                .iter()
                .find(|p| p.name == cond.parameter)
                .map(|p| p.kind);
            if kind.is_none() {
                ui.colored_label(WARN, "⚠")
                    .on_hover_text("Parameter does not exist");
            }
            condition_mode(ui, k, kind, &mut cond.mode);
            if ui.small_button("×").clicked() {
                remove = Some(k);
            }
        });
    }
    if let Some(k) = remove {
        t.conditions.remove(k);
    }
    if ui.small_button("+ Add Condition").clicked() {
        let first = params.first();
        t.conditions.push(Condition {
            parameter: first.map(|p| p.name.clone()).unwrap_or_default(),
            mode: match first.map(|p| p.kind) {
                Some(ParameterType::Float | ParameterType::Int) => ConditionMode::Greater(0.0),
                _ => ConditionMode::If,
            },
        });
    }
    if t.conditions.is_empty() && !t.has_exit_time && !simple {
        ui.colored_label(WARN, "Transitions need at least one condition or an exit time; this one fires straight away.");
    }
    ui.add_space(6.0);
    if ui.button("Delete Transition").clicked() {
        m.transitions.remove(ti);
        w.sel = None;
    }
}

/// Unity's transition timeline: the source and destination bars, where the destination
/// starts (exit time) and the overlap (duration). Drag the destination to set the exit
/// time and its handle to set the duration.
fn timeline(
    ui: &mut egui::Ui,
    t: &mut Transition,
    root: &AnimatorStateMachine,
    assets: &AssetServer,
    params: &[Parameter],
) {
    let pv: HashMap<String, f32> = params.iter().map(|p| (p.name.clone(), p.default)).collect();
    let len_of = |name: &str| {
        root.find_state(name)
            .and_then(|p| root.state_at(&p))
            .map(|s| oxide_animation::motion_length(assets, &s.motion, &pv))
            .filter(|l| *l > 0.0)
            .unwrap_or(1.0)
    };
    let src = t.from.as_deref().map_or(1.0, len_of);
    let dst = if t.target == TransitionTarget::State {
        len_of(&t.to)
    } else {
        1.0
    };
    let start = if t.has_exit_time {
        t.exit_time * src
    } else {
        0.0
    };
    let dur = if t.fixed_duration {
        t.duration
    } else {
        t.duration * src
    };
    let total = (start + dst - t.offset * dst).max(src).max(start + dur) * 1.1;
    let width = ui.available_width().max(120.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 64.0), Sense::click_and_drag());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 3.0, Color32::from_gray(30));
    let x = |s: f32| rect.left() + 6.0 + (rect.width() - 12.0) * (s / total);
    // Seconds ruler.
    let mut s = 0.0;
    while s <= total {
        p.line_segment(
            [pos2(x(s), rect.top()), pos2(x(s), rect.top() + 5.0)],
            Stroke::new(1.0, Color32::from_gray(90)),
        );
        s += (total / 8.0).max(0.05);
    }
    let src_r = Rect::from_min_max(
        pos2(x(0.0), rect.top() + 10.0),
        pos2(x(src.max(start + dur)), rect.top() + 30.0),
    );
    let dst_r = Rect::from_min_max(
        pos2(x(start), rect.top() + 36.0),
        pos2(x(start + dst * (1.0 - t.offset)), rect.top() + 56.0),
    );
    p.rect_filled(src_r, 2.0, Color32::from_gray(80));
    p.rect_filled(dst_r, 2.0, Color32::from_gray(80));
    let overlap = Rect::from_min_max(
        pos2(x(start), src_r.top()),
        pos2(x(start + dur), dst_r.bottom()),
    );
    p.rect_filled(overlap, 2.0, BLUE.gamma_multiply(0.35));
    p.text(
        src_r.left_center() + vec2(4.0, 0.0),
        Align2::LEFT_CENTER,
        t.from.as_deref().unwrap_or("Any State"),
        FontId::proportional(11.0),
        Color32::WHITE,
    );
    p.text(
        dst_r.left_center() + vec2(4.0, 0.0),
        Align2::LEFT_CENTER,
        &t.to,
        FontId::proportional(11.0),
        Color32::WHITE,
    );
    let handle = Rect::from_center_size(
        pos2(x(start + dur), overlap.center().y),
        vec2(8.0, overlap.height()),
    );
    p.rect_filled(handle, 2.0, SEL);
    if resp.dragged() {
        let ds = resp.drag_delta().x / (rect.width() - 12.0) * total;
        let at = ui.input(|i| i.pointer.press_origin()).unwrap_or_default();
        if handle.expand(4.0).contains(at) {
            let nd = (dur + ds).max(0.0);
            t.duration = if t.fixed_duration {
                nd
            } else {
                nd / src.max(1e-4)
            };
        } else if t.has_exit_time {
            t.exit_time = ((start + ds) / src.max(1e-4)).max(0.0);
        }
    }
    resp.on_hover_text(format!(
        "Exit at {start:.2}s, blend {dur:.2}s. Drag the lower bar to change the exit time, the blue handle to change the duration."
    ));
}

fn tree_inspector(
    ui: &mut egui::Ui,
    t: &mut BlendTree,
    params: &[Parameter],
    assets: &AssetServer,
    live: &HashMap<String, f32>,
    path: &[usize],
) {
    ui.label(RichText::new("Blend Tree").strong());
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "Name");
        ui.text_edit_singleline(&mut t.name);
    });
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "Blend Type");
        egui::ComboBox::from_id_salt(("bt-type", path.to_vec()))
            .selected_text(t.blend_type.label())
            .show_ui(ui, |ui| {
                for k in BlendTreeType::ALL {
                    ui.selectable_value(&mut t.blend_type, k, k.label());
                }
            });
    });
    let floats = |p: &Parameter| matches!(p.kind, ParameterType::Float | ParameterType::Int);
    match t.blend_type {
        BlendTreeType::Direct => {
            ui.checkbox(&mut t.normalized_blend_values, "Normalized Blend Values");
        }
        bt => {
            ui.horizontal(|ui| {
                crate::inspector::prop_label(ui, "Parameter");
                param_combo(
                    ui,
                    ("bt-x", path.to_vec()),
                    params,
                    &mut t.blend_parameter,
                    floats,
                );
                if bt.is_2d() {
                    param_combo(
                        ui,
                        ("bt-y", path.to_vec()),
                        params,
                        &mut t.blend_parameter_y,
                        floats,
                    );
                }
            });
        }
    }
    let weights = oxide_animation::tree_weights(t, live);
    if t.blend_type.is_2d() {
        diagram_2d(ui, t, live, &weights);
    } else if t.blend_type == BlendTreeType::Simple1D {
        diagram_1d(ui, t, live, &weights);
    }
    ui.add_space(4.0);
    ui.label(RichText::new("Motions").strong());
    let auto = t.use_automatic_thresholds && t.blend_type == BlendTreeType::Simple1D;
    let thresholds = t.effective_thresholds();
    let mut remove = None;
    let mut swap = None;
    let n = t.children.len();
    let bt = t.blend_type;
    egui::Grid::new(("bt-children", path.to_vec()))
        .num_columns(6)
        .striped(true)
        .show(ui, |ui| {
            ui.weak("Motion");
            ui.weak(match bt {
                BlendTreeType::Simple1D => "Threshold",
                BlendTreeType::Direct => "Parameter",
                _ => "Pos X / Y",
            });
            ui.weak("Speed");
            ui.weak("Mirror");
            ui.weak("");
            ui.weak("");
            ui.end_row();
            for (k, ch) in t.children.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.set_max_width(130.0);
                    motion_picker(ui, ("bt-m", path.to_vec(), k), assets, &mut ch.motion, true);
                });
                match bt {
                    BlendTreeType::Simple1D => {
                        if auto {
                            ui.label(format!("{:.2}", thresholds[k]));
                        } else {
                            ui.add(
                                egui::DragValue::new(&mut ch.threshold)
                                    .speed(0.01)
                                    .max_decimals(3),
                            );
                        }
                    }
                    BlendTreeType::Direct => {
                        param_combo(
                            ui,
                            ("bt-d", path.to_vec(), k),
                            params,
                            &mut ch.direct_blend_parameter,
                            floats,
                        );
                    }
                    _ => {
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::DragValue::new(&mut ch.position.x)
                                    .speed(0.01)
                                    .max_decimals(2),
                            );
                            ui.add(
                                egui::DragValue::new(&mut ch.position.y)
                                    .speed(0.01)
                                    .max_decimals(2),
                            );
                        });
                    }
                }
                ui.add(
                    egui::DragValue::new(&mut ch.time_scale)
                        .speed(0.01)
                        .max_decimals(2),
                );
                ui.checkbox(&mut ch.mirror, "");
                ui.label(
                    RichText::new(format!(
                        "{:.0}%",
                        weights.get(k).copied().unwrap_or(0.0) * 100.0
                    ))
                    .small()
                    .weak(),
                );
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(k > 0, egui::Button::new("⏶").small())
                        .clicked()
                    {
                        swap = Some((k, k - 1));
                    }
                    if ui
                        .add_enabled(k + 1 < n, egui::Button::new("⏷").small())
                        .clicked()
                    {
                        swap = Some((k, k + 1));
                    }
                    if ui.small_button("×").clicked() {
                        remove = Some(k);
                    }
                });
                ui.end_row();
            }
        });
    if let Some((a, b)) = swap {
        t.children.swap(a, b);
    }
    if let Some(k) = remove {
        t.children.remove(k);
    }
    ui.horizontal(|ui| {
        if ui.button("+ Add Motion Field").clicked() {
            t.add_child(Motion::Empty);
        }
        if ui.button("+ New Blend Tree").clicked() {
            t.add_child(Motion::tree(BlendTree::default()));
        }
    });
    if t.blend_type == BlendTreeType::Simple1D {
        ui.add_space(4.0);
        let mut auto = t.use_automatic_thresholds;
        if ui.checkbox(&mut auto, "Automate Thresholds").changed() {
            if !auto {
                t.bake_thresholds();
            }
            t.use_automatic_thresholds = auto;
        }
        if t.use_automatic_thresholds {
            ui.horizontal(|ui| {
                ui.label("Range");
                ui.add(egui::DragValue::new(&mut t.min_threshold).speed(0.01));
                ui.add(egui::DragValue::new(&mut t.max_threshold).speed(0.01));
            });
        } else {
            ui.menu_button("Compute Thresholds ⏷", |ui| {
                for (label, what) in [
                    ("Speed", RootMeasure::Speed),
                    ("Velocity X", RootMeasure::X),
                    ("Velocity Y", RootMeasure::Y),
                    ("Velocity Z", RootMeasure::Z),
                    ("Angular Speed (Rad)", RootMeasure::AngRad),
                    ("Angular Speed (Deg)", RootMeasure::AngDeg),
                ] {
                    if ui.button(label).clicked() {
                        for ch in &mut t.children {
                            if let Motion::Clip(h) = &ch.motion
                                && let Some(clip) = assets.animations.get(h)
                            {
                                ch.threshold = root_measure(clip, what);
                            }
                        }
                        ui.close();
                    }
                }
            });
        }
    } else if t.blend_type.is_2d() {
        ui.menu_button("Compute Positions ⏷", |ui| {
            for (label, a, b) in [
                ("Velocity XZ", RootMeasure::X, RootMeasure::Z),
                (
                    "Speed and Angular Speed",
                    RootMeasure::Speed,
                    RootMeasure::AngDeg,
                ),
            ] {
                if ui.button(label).clicked() {
                    for ch in &mut t.children {
                        if let Motion::Clip(h) = &ch.motion
                            && let Some(clip) = assets.animations.get(h)
                        {
                            ch.position =
                                Vector2::new(root_measure(clip, a), root_measure(clip, b));
                        }
                    }
                    ui.close();
                }
            }
        });
    }
    if ui
        .button("Adjust Time Scale: Homogeneous Speed")
        .on_hover_text("Scale each motion's speed so they all move at the average root speed")
        .clicked()
    {
        let speeds: Vec<Option<f32>> = t
            .children
            .iter()
            .map(|ch| match &ch.motion {
                Motion::Clip(h) => assets
                    .animations
                    .get(h)
                    .map(|c| root_measure(c, RootMeasure::Speed)),
                _ => None,
            })
            .collect();
        let valid: Vec<f32> = speeds
            .iter()
            .flatten()
            .copied()
            .filter(|s| *s > 1e-3)
            .collect();
        if !valid.is_empty() {
            let avg = valid.iter().sum::<f32>() / valid.len() as f32;
            for (ch, s) in t.children.iter_mut().zip(speeds) {
                if let Some(s) = s.filter(|s| *s > 1e-3) {
                    ch.time_scale = avg / s;
                }
            }
        }
    }
    // Warnings (Unity shows these too).
    if t.blend_type != BlendTreeType::Direct
        && !t.blend_parameter.is_empty()
        && !params.iter().any(|p| p.name == t.blend_parameter)
    {
        ui.colored_label(
            WARN,
            format!("Parameter \"{}\" does not exist.", t.blend_parameter),
        );
    }
    if t.children.iter().any(|c| c.motion == Motion::Empty) {
        ui.colored_label(WARN, "Some motion fields are empty.");
    }
}

#[derive(Clone, Copy)]
enum RootMeasure {
    Speed,
    X,
    Y,
    Z,
    AngRad,
    AngDeg,
}

/// Average root velocity / angular speed of a clip (Unity's Compute Thresholds).
fn root_measure(clip: &AnimationClip, what: RootMeasure) -> f32 {
    let len = clip.length.max(1e-4);
    let Some(rc) = clip.root_curve().and_then(|i| clip.curves.get(i)) else {
        return 0.0;
    };
    let v = rc
        .position
        .as_ref()
        .and_then(|tr| Some((tr.sample(len)? - tr.sample(0.0)?) / len))
        .unwrap_or_default();
    let ang = rc
        .rotation
        .as_ref()
        .and_then(|tr| {
            let (a, b) = (tr.sample(0.0)?, tr.sample(len)?);
            let d = b * a.inverse();
            Some(2.0 * d.y.atan2(d.w) / len)
        })
        .unwrap_or(0.0);
    match what {
        RootMeasure::Speed => v.length(),
        RootMeasure::X => v.x,
        RootMeasure::Y => v.y,
        RootMeasure::Z => v.z,
        RootMeasure::AngRad => ang,
        RootMeasure::AngDeg => ang.to_degrees(),
    }
}

/// 1D: the thresholds on a line with weight triangles and the parameter's position.
fn diagram_1d(ui: &mut egui::Ui, t: &BlendTree, live: &HashMap<String, f32>, weights: &[f32]) {
    let th = t.effective_thresholds();
    if th.is_empty() {
        return;
    }
    let (lo, hi) = param_range(t, 0);
    let width = ui.available_width().max(120.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, 46.0), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 3.0, Color32::from_gray(30));
    let x = |v: f32| rect.left() + 8.0 + (rect.width() - 16.0) * ((v - lo) / (hi - lo).max(1e-4));
    for (k, v) in th.iter().enumerate() {
        let col =
            Color32::from_gray(110).lerp_to_gamma(BLUE, weights.get(k).copied().unwrap_or(0.0));
        p.add(egui::Shape::convex_polygon(
            vec![
                pos2(x(*v), rect.top() + 6.0),
                pos2(x(*v) + 9.0, rect.bottom() - 6.0),
                pos2(x(*v) - 9.0, rect.bottom() - 6.0),
            ],
            col.gamma_multiply(0.7),
            Stroke::new(1.0, col),
        ));
    }
    let v = live.get(&t.blend_parameter).copied().unwrap_or(0.0);
    p.line_segment(
        [pos2(x(v), rect.top()), pos2(x(v), rect.bottom())],
        Stroke::new(2.0, Color32::from_rgb(230, 80, 80)),
    );
}

/// 2D: child positions as dots (sized by weight) and the parameter point (Unity's diagram).
fn diagram_2d(ui: &mut egui::Ui, t: &BlendTree, live: &HashMap<String, f32>, weights: &[f32]) {
    let side = ui.available_width().clamp(120.0, 240.0);
    let (rect, _) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 3.0, Color32::from_gray(30));
    let ext = t
        .children
        .iter()
        .map(|c| c.position.abs().max_element())
        .fold(1.0f32, f32::max)
        * 1.15;
    let to = |v: Vector2| rect.center() + vec2(v.x, -v.y) * (side / 2.0 / ext);
    p.line_segment(
        [
            pos2(rect.left(), rect.center().y),
            pos2(rect.right(), rect.center().y),
        ],
        Stroke::new(1.0, Color32::from_gray(55)),
    );
    p.line_segment(
        [
            pos2(rect.center().x, rect.top()),
            pos2(rect.center().x, rect.bottom()),
        ],
        Stroke::new(1.0, Color32::from_gray(55)),
    );
    for (k, c) in t.children.iter().enumerate() {
        let wgt = weights.get(k).copied().unwrap_or(0.0);
        p.circle_filled(
            to(c.position),
            3.0 + 10.0 * wgt,
            BLUE.gamma_multiply(0.35 + 0.65 * wgt),
        );
        p.circle_filled(to(c.position), 2.5, Color32::WHITE);
    }
    let v = Vector2::new(
        live.get(&t.blend_parameter).copied().unwrap_or(0.0),
        live.get(&t.blend_parameter_y).copied().unwrap_or(0.0),
    );
    p.circle_filled(to(v), 4.0, Color32::from_rgb(230, 80, 80));
}

/// Unity's StateMachineBehaviour list: Add Behaviour from the registered types; each one's
/// fields are edited like a component's.
fn behaviours_ui(
    ui: &mut egui::Ui,
    list: &mut Vec<StateBehaviour>,
    world: &oxide_scene::World,
    registry: Option<&StateMachineBehaviours>,
    salt: &str,
) {
    ui.label(RichText::new("Behaviours").strong());
    let mut remove = None;
    for (k, b) in list.iter_mut().enumerate() {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(crate::inspector::nicify(&b.type_name)).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("×").on_hover_text("Remove").clicked() {
                        remove = Some(k);
                    }
                });
            });
            match registry.and_then(|r| r.instantiate(b)) {
                Some(mut inst) => {
                    if crate::inspector::reflect_grid(
                        ui,
                        world,
                        inst.as_mut(),
                        &format!("{salt}-{k}"),
                    ) {
                        b.fields = inst
                            .fields()
                            .into_iter()
                            .map(|(k, v)| (k.to_owned(), v))
                            .collect();
                    }
                }
                None => {
                    ui.colored_label(
                        WARN,
                        "Script type not loaded (compile the Scripts project).",
                    );
                }
            }
        });
    }
    if let Some(k) = remove {
        list.remove(k);
    }
    let names = registry.map(|r| r.names()).unwrap_or_default();
    ui.menu_button("Add Behaviour", |ui| {
        if names.is_empty() {
            ui.weak("No StateMachineBehaviour scripts. Implement `StateMachineBehaviour` and list the type in `scripts! { ...; behaviours: Type }`.");
        }
        for n in names {
            if ui.button(&n).clicked() {
                let fields = registry
                    .and_then(|r| r.create(&n))
                    .map(|inst| {
                        inst.fields()
                            .into_iter()
                            .map(|(k, v)| (k.to_owned(), v))
                            .collect()
                    })
                    .unwrap_or_default();
                list.push(StateBehaviour {
                    type_name: n.clone(),
                    fields,
                });
                ui.close();
            }
        }
    });
}

/// A motion's display name: the clip's own name (not its file path) when it has one.
fn motion_name(assets: &AssetServer, m: &Motion) -> String {
    match m {
        Motion::Clip(h) => assets
            .animations
            .get(h)
            .map(|c| c.name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| m.label()),
        _ => m.label(),
    }
}

/// Clip / Blend Tree / None picker for a motion field.
fn motion_picker(
    ui: &mut egui::Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    assets: &AssetServer,
    m: &mut Motion,
    allow_tree: bool,
) -> bool {
    let name = |p: &str| {
        let h: Handle<AnimationClip> = Handle::from_path(p);
        assets
            .animations
            .get(&h)
            .map(|c| c.name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| Motion::Clip(h).label())
    };
    let current = match m {
        Motion::Clip(h) if !h.path().is_empty() => name(h.path()),
        Motion::Clip(_) | Motion::Empty => "None (Motion)".to_owned(),
        _ => m.label(),
    };
    let mut changed = false;
    egui::ComboBox::from_id_salt(salt)
        .selected_text(current)
        .width(crate::inspector::avail(ui).clamp(60.0, 220.0))
        .show_ui(ui, |ui| {
            if ui.selectable_label(*m == Motion::Empty, "None").clicked() {
                *m = Motion::Empty;
                changed = true;
            }
            if allow_tree
                && ui
                    .selectable_label(m.as_tree().is_some(), "Blend Tree")
                    .clicked()
                && m.as_tree().is_none()
            {
                *m = Motion::tree(BlendTree::default());
                changed = true;
            }
            ui.separator();
            for p in clip_paths(assets) {
                let sel = matches!(m, Motion::Clip(h) if h.path() == p);
                if ui
                    .selectable_label(sel, name(&p))
                    .on_hover_text(&p)
                    .clicked()
                {
                    *m = Motion::Clip(Handle::from_path(p));
                    changed = true;
                }
            }
        });
    // Drop a clip from the Project window onto the field.
    let r = ui.response();
    if let Some(payload) = r.dnd_release_payload::<AssetPayload>()
        && (payload.0.ends_with(".anim") || payload.0.contains('#'))
    {
        *m = Motion::Clip(Handle::from_path(payload.0.clone()));
        changed = true;
    }
    changed
}

fn param_combo(
    ui: &mut egui::Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    params: &[Parameter],
    value: &mut String,
    filter: impl Fn(&Parameter) -> bool,
) {
    egui::ComboBox::from_id_salt(salt)
        .selected_text(if value.is_empty() {
            "(parameter)"
        } else {
            value.as_str()
        })
        .show_ui(ui, |ui| {
            for p in params.iter().filter(|p| filter(p)) {
                ui.selectable_value(value, p.name.clone(), &p.name);
            }
        });
}

fn condition_mode(
    ui: &mut egui::Ui,
    k: usize,
    kind: Option<ParameterType>,
    mode: &mut ConditionMode,
) {
    let value = |m: &ConditionMode| match m {
        ConditionMode::Greater(v)
        | ConditionMode::Less(v)
        | ConditionMode::Equals(v)
        | ConditionMode::NotEqual(v) => Some(*v),
        _ => None,
    };
    let label = |m: &ConditionMode| match m {
        ConditionMode::If => "true",
        ConditionMode::IfNot => "false",
        ConditionMode::Greater(_) => "Greater",
        ConditionMode::Less(_) => "Less",
        ConditionMode::Equals(_) => "Equals",
        ConditionMode::NotEqual(_) => "NotEqual",
    };
    let v = value(mode).unwrap_or(0.0);
    let options: Vec<ConditionMode> = match kind {
        Some(ParameterType::Trigger) => vec![ConditionMode::If],
        Some(ParameterType::Bool) => vec![ConditionMode::If, ConditionMode::IfNot],
        Some(ParameterType::Float) => vec![ConditionMode::Greater(v), ConditionMode::Less(v)],
        _ => vec![
            ConditionMode::Greater(v),
            ConditionMode::Less(v),
            ConditionMode::Equals(v),
            ConditionMode::NotEqual(v),
        ],
    };
    if !options
        .iter()
        .any(|o| std::mem::discriminant(o) == std::mem::discriminant(mode))
    {
        *mode = options[0];
    }
    egui::ComboBox::from_id_salt(("cond-m", k))
        .selected_text(label(mode))
        .width(80.0)
        .show_ui(ui, |ui| {
            for o in &options {
                if ui
                    .selectable_label(
                        std::mem::discriminant(o) == std::mem::discriminant(mode),
                        label(o),
                    )
                    .clicked()
                {
                    *mode = *o;
                }
            }
        });
    if let Some(mut x) = value(mode)
        && ui.add(egui::DragValue::new(&mut x).speed(0.01)).changed()
    {
        *mode = match mode {
            ConditionMode::Greater(_) => ConditionMode::Greater(x),
            ConditionMode::Less(_) => ConditionMode::Less(x),
            ConditionMode::Equals(_) => ConditionMode::Equals(x),
            _ => ConditionMode::NotEqual(x),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctrl() -> AnimatorController {
        let mut c = AnimatorController::default();
        c.add_parameter("Speed", ParameterType::Float);
        c.add_state("Idle", Motion::Empty);
        c.add_state("Run", Motion::Empty);
        c.add_state("Jump", Motion::Empty);
        c.add_transition(Some("Idle"), "Run", vec![], 0.1);
        c.add_transition(Some("Run"), "Jump", vec![], 0.1);
        c.add_transition(None, "Idle", vec![], 0.1);
        c
    }

    #[test]
    fn deleting_a_state_removes_its_transitions_and_fixes_default() {
        let mut c = ctrl();
        c.base().default_state = 2;
        let mut w = AnimatorWindow {
            sel: Some(Sel::Nodes(vec![Node::State(1)])),
            ..Default::default()
        };
        delete_selection(&mut w, c.base(), &[]);
        let sm = c.base();
        assert_eq!(sm.states.len(), 2);
        assert_eq!(
            sm.transitions.len(),
            1,
            "transitions to and from Run are gone"
        );
        assert_eq!(sm.default_state, 1, "Jump moved down one");
    }

    #[test]
    fn deleting_a_sub_machine_removes_transitions_into_it() {
        let mut c = ctrl();
        let sub = c.base().add_state_machine("Air");
        sub.add_state("Fall", Motion::Empty);
        c.add_transition(Some("Idle"), "Fall", vec![], 0.1);
        let mut w = AnimatorWindow {
            sel: Some(Sel::Nodes(vec![Node::Machine(0)])),
            ..Default::default()
        };
        delete_selection(&mut w, c.base(), &[]);
        assert!(c.base().state_machines.is_empty());
        assert!(!c.base().transitions.iter().any(|t| t.to == "Fall"));
    }

    #[test]
    fn renaming_a_parameter_updates_references() {
        let mut c = ctrl();
        c.base().transitions[0].conditions.push(Condition {
            parameter: "Speed".into(),
            mode: ConditionMode::Greater(0.1),
        });
        c.add_state(
            "Move",
            Motion::tree(BlendTree {
                blend_parameter: "Speed".into(),
                ..Default::default()
            }),
        );
        rename_parameter(&mut c, "Speed", "Velocity");
        assert_eq!(c.parameters[0].name, "Velocity");
        assert_eq!(c.base().transitions[0].conditions[0].parameter, "Velocity");
        let s = c
            .base()
            .states
            .last()
            .unwrap()
            .motion
            .as_tree()
            .unwrap()
            .blend_parameter
            .clone();
        assert_eq!(s, "Velocity");
    }

    #[test]
    fn sub_machine_views_route_arrows_through_nodes() {
        let mut c = ctrl();
        let sub = c.base().add_state_machine("Air");
        sub.add_state("Fall", Motion::Empty);
        // A state inside Air goes back to Idle: from the parent, it leaves the Air node.
        sub.add_transition(Some("Fall"), "Idle", vec![], 0.1);
        c.add_transition(Some("Jump"), "Fall", vec![], 0.1);
        let root = &c.layers[0].state_machine;
        let top = view_transitions(root, &[]);
        assert!(
            top.iter()
                .any(|(f, t, _, _)| *f == Node::Machine(0) && *t == Node::State(0))
        );
        assert!(
            top.iter()
                .any(|(f, t, _, _)| *f == Node::State(2) && *t == Node::Machine(0))
        );
        let inside = view_transitions(root, &[0]);
        assert!(
            inside
                .iter()
                .any(|(f, t, _, _)| *f == Node::State(0) && *t == Node::Up)
        );
    }

    #[test]
    fn tree_layout_puts_children_right_of_parents() {
        let mut t = BlendTree::default();
        t.add_child(Motion::Empty);
        let mut inner = BlendTree::default();
        inner.add_child(Motion::Empty);
        inner.add_child(Motion::Empty);
        t.add_child(Motion::tree(inner));
        let l = tree_layout(&Motion::tree(t));
        assert_eq!(l.len(), 5);
        assert_eq!(l[0].1, 0);
        assert!(l.iter().any(|(p, d, _)| *p == vec![1, 1] && *d == 2));
    }

    #[test]
    fn border_points_sit_on_the_rect() {
        let r = Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 40.0));
        let p = border_point(r, pos2(500.0, 20.0));
        assert!((p.x - 100.0).abs() < 1e-3 && (p.y - 20.0).abs() < 1e-3);
    }

    #[test]
    fn child_motion_default_is_unit_speed() {
        assert_eq!(oxide_assets::ChildMotion::default().time_scale, 1.0);
    }
}
