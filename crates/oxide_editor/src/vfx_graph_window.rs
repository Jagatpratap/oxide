//! The VFX Graph window: contexts with their block stacks, operators and flow on the node
//! canvas, the Blackboard of exposed properties, and a Graph Inspector for context
//! settings and blocks (add, enable, reorder, remove, gradients and curves). Edits apply
//! to Visual Effects in the scene right away (Unity's Auto compile); Save Asset writes the
//! `.vfx` file.
//! Reference: <https://docs.unity3d.com/Packages/com.unity.visualeffectgraph@17.0/manual/VisualEffectGraphWindow.html>

use crate::EditorState;
use crate::node_canvas::{self, CanvasModel, CanvasView, Template};
use egui::{Color32, Rect, RichText, pos2, vec2};
use oxide_assets::{AssetServer, Handle};
use oxide_engine::Engine;
use oxide_math::{Vector2, Vector4};
use oxide_shadergraph::nodes::Opt;
use oxide_shadergraph::{GraphData, GraphNode, GraphProperty, PropertyType, Slot, Ty};
use oxide_vfx::blocks::{self, Stage};
use oxide_vfx::{INITIALIZE, OUTPUT, SPAWN, UPDATE, VisualEffectAsset};
use serde_json::{Value as Json, json};

#[derive(Default)]
pub struct VfxGraphWindow {
    pub path: Option<String>,
    pub asset: Option<VisualEffectAsset>,
    pub dirty: bool,
    undo: Vec<VisualEffectAsset>,
    redo: Vec<VisualEffectAsset>,
    before: Option<VisualEffectAsset>,
    pub canvas: CanvasView,
    hide_blackboard: bool,
    hide_inspector: bool,
    selected_property: Option<usize>,
    /// The block shown in the Graph Inspector.
    selected_block: Option<u32>,
    pub errors: Vec<(u32, String)>,
    /// The compile result is current.
    compiled: bool,
}

impl VfxGraphWindow {
    pub fn open(&mut self, assets: &mut AssetServer, path: &str) -> Result<(), String> {
        if !assets.data.contains(path) {
            assets.load_data(path)?;
        }
        let a = assets
            .data
            .get(&Handle::<VisualEffectAsset>::from_path(path.to_owned()))
            .cloned()
            .ok_or_else(|| format!("{path} is not a Visual Effect Graph"))?;
        self.path = Some(path.to_owned());
        self.before = Some(a.clone());
        self.asset = Some(a);
        self.dirty = false;
        self.undo.clear();
        self.redo.clear();
        self.canvas.reset();
        self.selected_property = None;
        self.selected_block = None;
        self.compiled = false;
        Ok(())
    }

    /// Records an undo step and applies the graph to effects using it.
    fn commit(&mut self, assets: &mut AssetServer) {
        let Some(a) = &mut self.asset else { return };
        // Blocks of deleted contexts go with them.
        let graph = a.graph.clone();
        a.blocks.retain(|b| graph.node(b.context).is_some());
        if let Some(b) = self.before.take()
            && b != *a
        {
            self.undo.push(b);
            self.redo.clear();
            self.dirty = true;
            if self.undo.len() > 200 {
                self.undo.remove(0);
            }
        }
        self.before = self.asset.clone();
        self.apply(assets);
    }

    /// Unity's Auto compile: effects in the scene follow the graph as it is edited.
    fn apply(&mut self, assets: &mut AssetServer) {
        if let (Some(p), Some(a)) = (&self.path, &self.asset) {
            assets
                .data
                .insert(p, "VisualEffectAsset", Box::new(a.clone()));
        }
        self.compiled = false;
    }

    fn undo(&mut self, assets: &mut AssetServer) {
        if let (Some(prev), Some(a)) = (self.undo.pop(), self.asset.take()) {
            self.redo.push(a);
            self.before = Some(prev.clone());
            self.asset = Some(prev);
            self.dirty = true;
            self.apply(assets);
        }
    }

    fn redo(&mut self, assets: &mut AssetServer) {
        if let (Some(next), Some(a)) = (self.redo.pop(), self.asset.take()) {
            self.undo.push(a);
            self.before = Some(next.clone());
            self.asset = Some(next);
            self.dirty = true;
            self.apply(assets);
        }
    }

    pub fn save(&mut self, assets: &mut AssetServer) -> Result<(), String> {
        let (Some(p), Some(a)) = (&self.path, &self.asset) else {
            return Err("no Visual Effect Graph is open".into());
        };
        assets.save_data(p, Box::new(a.clone()))?;
        self.dirty = false;
        Ok(())
    }

    fn compile(&mut self) {
        if self.compiled {
            return;
        }
        if let Some(a) = &self.asset {
            let name = self.path.as_deref().unwrap_or("vfx");
            let c = oxide_vfx::compile(a, name);
            self.errors = c.errors.clone();
            for s in &c.systems {
                if let Err(e) = oxide_render::vfx::validate_program(&s.program) {
                    self.errors
                        .push((s.initialize, e.lines().next().unwrap_or("").to_owned()));
                }
            }
        }
        self.compiled = true;
    }
}

struct Model<'a> {
    asset: &'a mut VisualEffectAsset,
    errors: &'a [(u32, String)],
}

impl CanvasModel for Model<'_> {
    fn data(&self) -> &GraphData {
        &self.asset.graph
    }

    fn data_mut(&mut self) -> &mut GraphData {
        &mut self.asset.graph
    }

    fn ports(&self, node: &GraphNode) -> (Vec<Slot>, Vec<Slot>) {
        oxide_vfx::ports(self.asset, node)
    }

    fn title(&self, node: &GraphNode) -> String {
        match node.kind.as_str() {
            "Property" => {
                let r = node.option("Property").unwrap_or("");
                self.asset
                    .properties
                    .iter()
                    .find(|p| p.reference == r)
                    .map_or_else(|| format!("{r} (missing)"), |p| p.name.clone())
            }
            "Get Attribute" => format!(
                "Get Attribute: {}",
                node.option("Attribute").unwrap_or("position")
            ),
            k if oxide_vfx::is_context(k) => {
                let n = self.asset.blocks_of(node.id).count();
                format!("{k}  ({n} block{})", if n == 1 { "" } else { "s" })
            }
            k => k.to_owned(),
        }
    }

    fn options(&self, node: &GraphNode) -> Vec<Opt> {
        if oxide_vfx::is_context(&node.kind) {
            return oxide_vfx::context_options(&node.kind);
        }
        if node.kind == "Get Attribute" {
            return Vec::new();
        }
        oxide_vfx::find_operator(&node.kind).map_or_else(Vec::new, |d| d.options.clone())
    }

    fn fixed(&self, _: &GraphNode) -> bool {
        false
    }

    fn width(&self, node: &GraphNode) -> f32 {
        if oxide_vfx::is_context(&node.kind) {
            300.0
        } else if node.kind == "Get Attribute" {
            190.0
        } else {
            170.0
        }
    }

    fn accent(&self, node: &GraphNode) -> Color32 {
        match node.kind.as_str() {
            SPAWN => Color32::from_rgb(110, 200, 110),
            INITIALIZE => Color32::from_rgb(230, 160, 70),
            UPDATE => Color32::from_rgb(230, 210, 90),
            OUTPUT => Color32::from_rgb(220, 90, 90),
            "Property" => Color32::from_rgb(120, 170, 240),
            "Get Attribute" | "Age Over Lifetime" => Color32::from_rgb(170, 120, 230),
            _ => Color32::from_rgb(90, 140, 220),
        }
    }

    fn preview(&self, _: &GraphNode) -> Option<(egui::TextureId, Rect)> {
        None
    }

    fn has_preview(&self, _: &GraphNode) -> bool {
        false
    }

    fn error(&self, node: u32) -> Option<String> {
        let e: Vec<&str> = self
            .errors
            .iter()
            .filter(|(n, _)| *n == node)
            .map(|(_, e)| e.as_str())
            .collect();
        (!e.is_empty()).then(|| e.join("\n"))
    }

    fn templates(&self) -> Vec<Template> {
        let mut out: Vec<Template> = [SPAWN, INITIALIZE, UPDATE, OUTPUT]
            .iter()
            .map(|k| Template {
                path: format!("Context/{k}"),
                kind: (*k).into(),
                options: Vec::new(),
                preview: false,
            })
            .collect();
        for d in oxide_vfx::vfx_operators()
            .iter()
            .filter(|d| d.kind != "Property")
        {
            out.push(Template {
                path: format!("Operator/{}/{}", d.menu, d.kind),
                kind: d.kind.into(),
                options: Vec::new(),
                preview: false,
            });
        }
        for d in oxide_shadergraph::library() {
            if oxide_vfx::find_operator(d.kind).is_some() && d.kind != "Property" {
                out.push(Template {
                    path: format!("Operator/{}/{}", d.menu, d.kind),
                    kind: d.kind.into(),
                    options: Vec::new(),
                    preview: false,
                });
            }
        }
        for p in &self.asset.properties {
            out.push(Template {
                path: format!("Properties/{}", p.name),
                kind: "Property".into(),
                options: vec![("Property".into(), p.reference.clone())],
                preview: false,
            });
        }
        out
    }

    fn can_connect(&self, from: Ty, to: Ty) -> bool {
        oxide_shadergraph::nodes::convert("x", from, to).is_some()
    }

    fn value(&self, node: &GraphNode, slot: &str) -> Option<Vector4> {
        match oxide_vfx::parse_port(slot) {
            Some((b, input)) if oxide_vfx::is_context(&node.kind) => {
                self.asset.block(b)?.value(input)
            }
            _ => node.value(slot),
        }
    }

    fn set_value(&mut self, node: u32, slot: &str, v: Vector4) {
        let is_context = self
            .asset
            .graph
            .node(node)
            .is_some_and(|n| oxide_vfx::is_context(&n.kind));
        match oxide_vfx::parse_port(slot) {
            Some((b, input)) if is_context => {
                if let Some(blk) = self.asset.block_mut(b) {
                    blk.set_value(input, v);
                }
            }
            _ => {
                if let Some(n) = self.asset.graph.node_mut(node) {
                    n.set_value(slot, v);
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
struct PropertyDrag(String);

pub fn window_ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    let w = &mut st.vfx_graph;
    let Some(path) = w.path.clone() else {
        ui.centered_and_justified(|ui| {
            ui.weak("Double-click a Visual Effect Graph (.vfx) in the Project window to edit it.");
        });
        return;
    };
    let mut save = false;
    ui.horizontal(|ui| {
        if ui.button("Save Asset").clicked() {
            save = true;
        }
        if ui.button("Show In Project").clicked() {
            st.project.selected = Some(path.clone());
            st.project.inspecting = true;
            if let Some((dir, _)) = path.rsplit_once('/') {
                st.project.folder = dir.to_owned();
            }
        }
        ui.separator();
        let w = &mut st.vfx_graph;
        for (hidden, label) in [
            (&mut w.hide_blackboard, "Blackboard"),
            (&mut w.hide_inspector, "Graph Inspector"),
        ] {
            let mut shown = !*hidden;
            if ui.toggle_value(&mut shown, label).changed() {
                *hidden = !shown;
            }
        }
        ui.separator();
        // Playback of the Visual Effects using this graph (Unity's Play Controls).
        for (label, cmd) in [
            ("Play", "Play"),
            ("Pause", "Pause"),
            ("Stop", "Stop"),
            ("Restart", "Restart"),
        ] {
            if ui.button(label).clicked() {
                control_all(&mut engine.world, &path, cmd);
            }
        }
        ui.separator();
        let title = format!(
            "{}{}",
            path.rsplit('/').next().unwrap_or(&path),
            if w.dirty { " *" } else { "" }
        );
        ui.label(RichText::new(title).strong());
        if !w.errors.is_empty() {
            ui.colored_label(
                Color32::from_rgb(240, 110, 100),
                format!("{} error(s)", w.errors.len()),
            );
        }
    });
    let w = &mut st.vfx_graph;
    w.compile();
    let Some(mut asset) = w.asset.take() else {
        return;
    };
    let out = {
        let mut model = Model {
            asset: &mut asset,
            errors: &w.errors,
        };
        node_canvas::show(ui, &mut w.canvas, &mut model, "vfxgraph")
    };
    w.asset = Some(asset);
    let assets = engine.world.resource_mut::<AssetServer>();
    if out.commit {
        w.commit(assets);
    } else if out.edited {
        w.apply(assets);
    }
    if out.undo {
        w.undo(assets);
    }
    if out.redo {
        w.redo(assets);
    }
    if out.save {
        save = true;
    }
    // A Blackboard property dropped on the canvas.
    if let Some(resp) = &out.response
        && let Some(p) = resp.dnd_release_payload::<PropertyDrag>()
        && let Some(pos) = ui.input(|i| i.pointer.hover_pos())
    {
        let at = w.canvas.to_graph(out.rect, pos);
        if let Some(a) = w.asset.as_mut() {
            let id = a
                .graph
                .add(GraphNode::new(0, "Property", at).with_option("Property", &p.0));
            w.canvas.selected = vec![id];
        }
        w.commit(assets);
    }
    // Selecting a context shows its blocks; a different context resets the block.
    if let [id] = w.canvas.selected[..]
        && let Some(b) = w.selected_block
        && w.asset
            .as_ref()
            .and_then(|a| a.block(b))
            .is_some_and(|b| b.context != id)
    {
        w.selected_block = None;
    }
    let canvas = out.rect;
    if !w.hide_blackboard {
        blackboard(ui, w, assets, canvas, &path);
    }
    if !w.hide_inspector {
        inspector(ui, w, engine, canvas);
    }
    if save {
        let r = st
            .vfx_graph
            .save(engine.world.resource_mut::<AssetServer>());
        st.status = match r {
            Ok(()) => format!("Saved {path}"),
            Err(e) => e,
        };
    }
}

/// Play / Pause / Stop / Restart every Visual Effect playing `path`.
pub fn control_all(world: &mut oxide_scene::World, path: &str, cmd: &str) -> usize {
    let ids: Vec<_> = world
        .query::<oxide_vfx::VisualEffect>()
        .filter(|(_, v)| v.asset.as_ref().is_some_and(|h| h.path() == path))
        .map(|(id, _)| id)
        .collect();
    for id in &ids {
        if let Some((v, _)) = oxide_vfx::effect_mut(world, *id) {
            control(v, cmd);
        }
    }
    ids.len()
}

pub fn control(v: &mut oxide_vfx::VisualEffect, cmd: &str) {
    match cmd {
        "Play" => {
            v.pause = false;
            v.play();
        }
        "Pause" => v.pause = !v.pause,
        "Stop" => v.stop(),
        "Restart" => {
            v.pause = false;
            v.reinit();
        }
        _ => {}
    }
}

fn new_property(a: &VisualEffectAsset, kind: PropertyType) -> GraphProperty {
    let base = match kind {
        PropertyType::Float => "Float",
        PropertyType::Vector2 => "Vector2",
        PropertyType::Vector3 => "Vector3",
        PropertyType::Vector4 => "Vector4",
        PropertyType::Color => "Color",
        PropertyType::Boolean => "Bool",
        PropertyType::Texture2D => "Texture2D",
    };
    let mut name = base.to_owned();
    let mut k = 1;
    while a.properties.iter().any(|p| p.name == name) {
        name = format!("{base} ({k})");
        k += 1;
    }
    GraphProperty {
        reference: oxide_shadergraph::unique_reference(&a.properties, &[], &name),
        name,
        kind,
        default: if kind == PropertyType::Color {
            Vector4::ONE
        } else {
            Vector4::ZERO
        },
        ..Default::default()
    }
}

fn blackboard(
    ui: &mut egui::Ui,
    w: &mut VfxGraphWindow,
    assets: &mut AssetServer,
    canvas: Rect,
    path: &str,
) {
    let mut changed = false;
    egui::Window::new("Blackboard")
        .id(egui::Id::new("vfx-blackboard"))
        .constrain_to(canvas)
        .default_pos(canvas.min + vec2(10.0, 10.0))
        .default_width(220.0)
        .collapsible(true)
        .resizable(true)
        .show(ui.ctx(), |ui| {
            let name = path.rsplit('/').next().unwrap_or(path);
            ui.label(RichText::new(name.trim_end_matches(".vfx")).strong());
            ui.weak("Visual Effect Graph");
            let Some(a) = w.asset.as_mut() else { return };
            ui.menu_button("+", |ui| {
                for kind in [
                    PropertyType::Float,
                    PropertyType::Vector2,
                    PropertyType::Vector3,
                    PropertyType::Vector4,
                    PropertyType::Color,
                    PropertyType::Boolean,
                ] {
                    if ui.button(kind.kind_name()).clicked() {
                        let p = new_property(a, kind);
                        a.properties.push(p);
                        w.selected_property = Some(a.properties.len() - 1);
                        w.selected_block = None;
                        changed = true;
                        ui.close();
                    }
                }
            });
            ui.separator();
            let mut remove = None;
            for (k, p) in a.properties.iter().enumerate() {
                let r = ui
                    .dnd_drag_source(
                        egui::Id::new(("vfx-prop", k)),
                        PropertyDrag(p.reference.clone()),
                        |ui| {
                            ui.horizontal(|ui| {
                                let _ =
                                    ui.selectable_label(w.selected_property == Some(k), &p.name);
                                ui.weak(p.kind.kind_name());
                                if p.exposed {
                                    ui.weak("Exposed");
                                }
                            });
                        },
                    )
                    .response;
                if r.clicked() {
                    w.selected_property = Some(k);
                    w.selected_block = None;
                }
                r.context_menu(|ui| {
                    if ui.button("Delete").clicked() {
                        remove = Some(k);
                        ui.close();
                    }
                });
            }
            if let Some(k) = remove {
                let r = a.properties[k].reference.clone();
                a.properties.remove(k);
                let ids: Vec<u32> = a
                    .graph
                    .nodes
                    .iter()
                    .filter(|n| n.kind == "Property" && n.option("Property") == Some(r.as_str()))
                    .map(|n| n.id)
                    .collect();
                a.remove_nodes(&ids);
                w.selected_property = None;
                changed = true;
            }
        });
    if changed {
        w.commit(assets);
    }
}

fn inspector(ui: &mut egui::Ui, w: &mut VfxGraphWindow, engine: &mut Engine, canvas: Rect) {
    let mut commit = false;
    let mut edited = false;
    egui::Window::new("Graph Inspector")
        .id(egui::Id::new("vfx-inspector"))
        .constrain_to(canvas)
        .default_pos(pos2(canvas.right() - 330.0, canvas.top() + 10.0))
        .default_width(310.0)
        .collapsible(true)
        .resizable(true)
        .show(ui.ctx(), |ui| {
            let world = &engine.world;
            let Some(a) = w.asset.as_mut() else { return };
            // A Blackboard property.
            if let Some(k) = w.selected_property
                && k < a.properties.len()
            {
                let mut p = a.properties[k].clone();
                let old = p.reference.clone();
                if crate::inspector::reflect_grid_where(ui, world, &mut p, "vfx-prop", &|f| {
                    !matches!(f, "default_texture" | "hdr" | "category")
                }) {
                    if p.reference != old {
                        for n in &mut a.graph.nodes {
                            if n.kind == "Property" && n.option("Property") == Some(old.as_str()) {
                                n.set_option("Property", &p.reference);
                            }
                        }
                    }
                    a.properties[k] = p;
                    commit = true;
                }
                return;
            }
            let [id] = w.canvas.selected[..] else {
                ui.weak("Select a context, an operator or a Blackboard property.");
                for (n, e) in w.errors.iter().filter(|(n, _)| *n == 0) {
                    ui.colored_label(Color32::from_rgb(240, 110, 100), format!("{n}: {e}"));
                }
                return;
            };
            let Some(node) = a.graph.node(id).cloned() else {
                return;
            };
            ui.label(RichText::new(&node.kind).strong());
            // Settings (contexts) or options (operators).
            let opts = if oxide_vfx::is_context(&node.kind) {
                oxide_vfx::context_options(&node.kind)
            } else {
                oxide_vfx::find_operator(&node.kind).map_or_else(Vec::new, |d| d.options.clone())
            };
            for o in &opts {
                let cur = node.option(o.name).unwrap_or(o.default).to_owned();
                ui.horizontal(|ui| {
                    crate::inspector::prop_label(ui, o.name);
                    let n = a.graph.node_mut(id).expect("node");
                    if o.name == "Main Texture" {
                        let mut v = if cur.is_empty() {
                            oxide_core::reflect::Value::Null
                        } else {
                            oxide_core::reflect::Value::Asset(cur.clone())
                        };
                        if crate::inspector::asset_field(
                            ui,
                            world,
                            Some("Texture"),
                            &mut v,
                            "vfx-tex",
                        ) {
                            let s = match v {
                                oxide_core::reflect::Value::Asset(p) => p,
                                _ => String::new(),
                            };
                            n.set_option(o.name, &s);
                            commit = true;
                        }
                    } else if !o.choices.is_empty() {
                        egui::ComboBox::from_id_salt(("vfx-opt", id, o.name))
                            .selected_text(&cur)
                            .show_ui(ui, |ui| {
                                for c in o.choices {
                                    if ui.selectable_label(*c == cur, *c).clicked() {
                                        n.set_option(o.name, c);
                                        commit = true;
                                    }
                                }
                            });
                    } else {
                        let mut s = cur.clone();
                        let r = ui.text_edit_singleline(&mut s);
                        if r.changed() {
                            n.set_option(o.name, &s);
                            edited = true;
                        }
                        commit |= r.lost_focus();
                    }
                });
            }
            if node.kind == OUTPUT {
                ui.weak("Flipbook: columns x rows (e.g. 4x4), or None.");
            }
            for (_, e) in w.errors.iter().filter(|(n, _)| *n == id) {
                ui.colored_label(Color32::from_rgb(240, 110, 100), e);
            }
            let Some(stage) = Stage::of(&node.kind) else {
                return;
            };
            // The block stack.
            ui.separator();
            ui.label(RichText::new("Blocks").strong());
            let list: Vec<(u32, String, bool)> = a
                .blocks_of(id)
                .map(|b| (b.id, b.kind.clone(), b.enabled))
                .collect();
            let mut action: Option<(u32, &str)> = None;
            for (bid, kind, enabled) in &list {
                ui.horizontal(|ui| {
                    let mut on = *enabled;
                    if ui.checkbox(&mut on, "").on_hover_text("Enabled").changed() {
                        action = Some((*bid, "toggle"));
                    }
                    if ui
                        .selectable_label(w.selected_block == Some(*bid), kind)
                        .clicked()
                    {
                        w.selected_block = Some(*bid);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("×").on_hover_text("Remove").clicked() {
                            action = Some((*bid, "remove"));
                        }
                        if ui.small_button("⏷").on_hover_text("Move down").clicked() {
                            action = Some((*bid, "down"));
                        }
                        if ui.small_button("⏶").on_hover_text("Move up").clicked() {
                            action = Some((*bid, "up"));
                        }
                    });
                });
            }
            if let Some((bid, what)) = action {
                match what {
                    "toggle" => {
                        if let Some(b) = a.block_mut(bid) {
                            b.enabled = !b.enabled;
                        }
                    }
                    "remove" => {
                        a.remove_block(bid);
                        if w.selected_block == Some(bid) {
                            w.selected_block = None;
                        }
                    }
                    "up" => a.move_block(bid, -1),
                    _ => a.move_block(bid, 1),
                }
                commit = true;
            }
            ui.menu_button("Add Block", |ui| {
                let mut menus: Vec<&str> = blocks::for_stage(stage).map(|d| d.menu).collect();
                menus.dedup();
                menus.sort();
                menus.dedup();
                for m in menus {
                    ui.menu_button(m, |ui| {
                        for d in blocks::for_stage(stage).filter(|d| d.menu == m) {
                            if ui.button(d.kind).clicked() {
                                let b = a.add_block(id, d.kind);
                                w.selected_block = Some(b);
                                commit = true;
                                ui.close();
                            }
                        }
                    });
                }
            });
            // The selected block's settings, gradient and curve.
            if let Some(bid) = w.selected_block
                && let Some(def) = a.block(bid).and_then(|b| blocks::find(&b.kind))
                && a.block(bid).is_some_and(|b| b.context == id)
            {
                ui.separator();
                ui.label(RichText::new(def.kind).strong());
                let b = a.block_mut(bid).expect("block");
                for o in def.options.iter().filter(|o| !o.choices.is_empty()) {
                    let cur = b.option(o.name).unwrap_or(o.default).to_owned();
                    ui.horizontal(|ui| {
                        crate::inspector::prop_label(ui, o.name);
                        egui::ComboBox::from_id_salt(("vfx-block-opt", bid, o.name))
                            .selected_text(&cur)
                            .show_ui(ui, |ui| {
                                for c in o.choices {
                                    if ui.selectable_label(*c == cur, *c).clicked() {
                                        b.set_option(o.name, c);
                                        commit = true;
                                    }
                                }
                            });
                    });
                }
                let (g, c) = (def.gradient.is_some(), def.curve.is_some());
                if (g || c)
                    && crate::inspector::reflect_grid_where(
                        ui,
                        world,
                        b,
                        &format!("vfx-block-{bid}"),
                        &|f| (g && f == "gradient") || (c && f == "curve"),
                    )
                {
                    commit = true;
                }
                ui.weak("Inputs are edited on the context's ports in the graph.");
            }
        });
    let assets = engine.world.resource_mut::<AssetServer>();
    if commit {
        w.commit(assets);
    } else if edited {
        w.apply(assets);
    }
}

// ---- MCP: `editor_vfx_graph`.

fn summary(w: &VfxGraphWindow) -> Json {
    let Some(a) = &w.asset else {
        return json!({ "open": null });
    };
    json!({
        "open": w.path,
        "dirty": w.dirty,
        "nodes": a.graph.nodes.iter().map(|n| {
            let (ins, outs) = oxide_vfx::ports(a, n);
            json!({
                "id": n.id, "kind": n.kind, "position": [n.position.x, n.position.y],
                "options": n.options.iter().map(|o| (o.name.clone(), json!(o.value))).collect::<serde_json::Map<_, _>>(),
                "inputs": ins.iter().map(|s| json!({"id": s.id, "type": s.ty.label()})).collect::<Vec<_>>(),
                "outputs": outs.iter().map(|s| json!({"id": s.id, "type": s.ty.label()})).collect::<Vec<_>>(),
                "blocks": a.blocks_of(n.id).map(|b| json!({
                    "id": b.id, "kind": b.kind, "enabled": b.enabled,
                    "values": b.values.iter().map(|v| (v.slot.clone(), json!(v.value.to_array()))).collect::<serde_json::Map<_, _>>(),
                    "options": b.options.iter().map(|o| (o.name.clone(), json!(o.value))).collect::<serde_json::Map<_, _>>(),
                })).collect::<Vec<_>>(),
            })
        }).collect::<Vec<_>>(),
        "edges": a.graph.edges.iter().map(|e| json!({"from": e.from, "output": e.output, "to": e.to, "input": e.input})).collect::<Vec<_>>(),
        "properties": a.properties.iter().map(|p| json!({"name": p.name, "reference": p.reference, "type": p.kind.kind_name(), "default": p.default.to_array(), "exposed": p.exposed})).collect::<Vec<_>>(),
        "errors": w.errors.iter().map(|(n, e)| json!({"node": n, "error": e})).collect::<Vec<_>>(),
    })
}

fn vec4(v: &Json) -> Option<Vector4> {
    match v {
        Json::Number(n) => Some(Vector4::splat(n.as_f64()? as f32)),
        Json::Array(a) => {
            let mut o = [0.0f32; 4];
            for (k, x) in a.iter().take(4).enumerate() {
                o[k] = x.as_f64()? as f32;
            }
            Some(Vector4::from_array(o))
        }
        Json::Bool(b) => Some(Vector4::splat(*b as u32 as f32)),
        _ => None,
    }
}

/// Every block and operator for MCP clients.
pub fn library_json() -> Json {
    let contexts: Vec<Json> = [SPAWN, INITIALIZE, UPDATE, OUTPUT]
        .iter()
        .map(|k| {
            json!({
                "kind": k,
                "settings": oxide_vfx::context_options(k).iter().map(|o| json!({"name": o.name, "choices": o.choices, "default": o.default})).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({
        "contexts": contexts,
        "blocks": blocks::library().iter().map(|d| json!({
            "kind": d.kind,
            "contexts": d.stages.iter().map(|s| format!("{s:?}")).collect::<Vec<_>>(),
            "inputs": d.inputs.iter().map(|s| json!({"id": s.id, "type": s.ty.label(), "default": s.default})).collect::<Vec<_>>(),
            "options": d.options.iter().map(|o| json!({"name": o.name, "choices": o.choices, "default": o.default})).collect::<Vec<_>>(),
            "gradient": d.gradient.is_some(), "curve": d.curve.is_some(),
        })).collect::<Vec<_>>(),
        "operators": oxide_vfx::vfx_operators().iter().map(|d| d.kind).chain(
            oxide_shadergraph::library().iter().filter(|d| oxide_vfx::find_operator(d.kind).is_some()).map(|d| d.kind)
        ).collect::<Vec<_>>(),
    })
}

/// The `editor_vfx_graph` tool.
pub fn mcp(
    st: &mut EditorState,
    engine: &mut Engine,
    args: &serde_json::Map<String, Json>,
) -> Result<Json, String> {
    if let Some(p) = args.get("open").and_then(Json::as_str) {
        st.vfx_graph
            .open(engine.world.resource_mut::<AssetServer>(), p)?;
        st.actions
            .push(crate::Action::OpenTab(crate::Tab::VfxGraph));
    }
    let w = &mut st.vfx_graph;
    let Some(a) = w.asset.as_mut() else {
        return Ok(summary(w));
    };
    let mut created: Vec<u32> = Vec::new();
    let mut edited = false;
    if let Some(p) = args.get("add_property").and_then(Json::as_object) {
        let kind = match p
            .get("type")
            .and_then(Json::as_str)
            .unwrap_or("Float")
            .to_ascii_lowercase()
            .replace(' ', "")
            .as_str()
        {
            "vector2" => PropertyType::Vector2,
            "vector3" => PropertyType::Vector3,
            "vector4" => PropertyType::Vector4,
            "color" => PropertyType::Color,
            "boolean" | "bool" => PropertyType::Boolean,
            _ => PropertyType::Float,
        };
        let mut prop = new_property(a, kind);
        if let Some(n) = p.get("name").and_then(Json::as_str) {
            prop.name = n.to_owned();
            prop.reference = oxide_shadergraph::unique_reference(&a.properties, &[], n);
        }
        if let Some(v) = p.get("default").and_then(vec4) {
            prop.default = v;
        }
        a.properties.push(prop);
        edited = true;
    }
    // Nodes: contexts and operators.
    if let Some(list) = args.get("add_nodes").and_then(Json::as_array) {
        for n in list {
            let kind = n
                .get("kind")
                .and_then(Json::as_str)
                .ok_or("add_nodes[].kind")?;
            if !oxide_vfx::is_context(kind) && oxide_vfx::find_operator(kind).is_none() {
                return Err(format!("unknown context or operator `{kind}`"));
            }
            let pos = n
                .get("position")
                .and_then(Json::as_array)
                .map(|p| {
                    Vector2::new(
                        p[0].as_f64().unwrap_or(0.0) as f32,
                        p.get(1).and_then(Json::as_f64).unwrap_or(0.0) as f32,
                    )
                })
                .unwrap_or(Vector2::new(-320.0, 60.0 * created.len() as f32));
            let mut node = GraphNode::new(0, kind, pos);
            node.preview = false;
            if let Some(o) = n.get("options").and_then(Json::as_object) {
                for (k, v) in o {
                    node.set_option(
                        k,
                        v.as_str()
                            .map_or_else(|| v.to_string(), str::to_owned)
                            .as_str(),
                    );
                }
            }
            if let Some(o) = n.get("values").and_then(Json::as_object) {
                for (k, v) in o {
                    node.set_value(k, vec4(v).ok_or_else(|| format!("values.{k}"))?);
                }
            }
            created.push(a.graph.add(node));
        }
        edited = true;
    }
    let resolve = |a: &VisualEffectAsset, v: &Json| -> Result<u32, String> {
        match v {
            Json::Number(n) => n.as_u64().map(|x| x as u32).ok_or_else(|| "bad id".into()),
            Json::String(s) if s.starts_with('$') => s[1..]
                .parse::<usize>()
                .ok()
                .and_then(|k| created.get(k).copied())
                .ok_or_else(|| format!("no new node {s}")),
            Json::String(s) => a
                .graph
                .find(s)
                .map(|n| n.id)
                .ok_or_else(|| format!("no node `{s}`")),
            _ => Err("node: an id, \"$k\" or a context name".into()),
        }
    };
    if let Some(s) = args.get("settings").and_then(Json::as_object) {
        let id = resolve(a, s.get("node").ok_or("settings.node")?)?;
        let n = a.graph.node_mut(id).ok_or("no such node")?;
        for (k, v) in s.iter().filter(|(k, _)| *k != "node") {
            n.set_option(
                k,
                v.as_str()
                    .map_or_else(|| v.to_string(), str::to_owned)
                    .as_str(),
            );
        }
        edited = true;
    }
    let mut new_blocks = Vec::new();
    if let Some(list) = args.get("add_blocks").and_then(Json::as_array) {
        for b in list {
            let ctx = resolve(a, b.get("context").ok_or("add_blocks[].context")?)?;
            let kind = b
                .get("kind")
                .and_then(Json::as_str)
                .ok_or("add_blocks[].kind")?;
            let def = blocks::find(kind).ok_or_else(|| format!("unknown block `{kind}`"))?;
            let stage = a
                .graph
                .node(ctx)
                .and_then(|n| Stage::of(&n.kind))
                .ok_or("context: not a context")?;
            if !def.stages.contains(&stage) {
                return Err(format!("{kind} can't go in {stage:?}"));
            }
            let id = a.add_block(ctx, kind);
            let blk = a.block_mut(id).expect("block");
            if let Some(o) = b.get("values").and_then(Json::as_object) {
                for (k, v) in o {
                    blk.set_value(k, vec4(v).ok_or_else(|| format!("values.{k}"))?);
                }
            }
            if let Some(o) = b.get("options").and_then(Json::as_object) {
                for (k, v) in o {
                    blk.set_option(k, v.as_str().unwrap_or(""));
                }
            }
            new_blocks.push(id);
        }
        edited = true;
    }
    if let Some(list) = args.get("set_blocks").and_then(Json::as_array) {
        for b in list {
            let id = b
                .get("block")
                .and_then(Json::as_u64)
                .ok_or("set_blocks[].block")? as u32;
            let blk = a.block_mut(id).ok_or_else(|| format!("no block {id}"))?;
            if let Some(o) = b.get("values").and_then(Json::as_object) {
                for (k, v) in o {
                    blk.set_value(k, vec4(v).ok_or_else(|| format!("values.{k}"))?);
                }
            }
            if let Some(o) = b.get("options").and_then(Json::as_object) {
                for (k, v) in o {
                    blk.set_option(k, v.as_str().unwrap_or(""));
                }
            }
            if let Some(e) = b.get("enabled").and_then(Json::as_bool) {
                blk.enabled = e;
            }
        }
        edited = true;
    }
    if let Some(list) = args.get("remove_blocks").and_then(Json::as_array) {
        for b in list.iter().filter_map(Json::as_u64) {
            a.remove_block(b as u32);
        }
        edited = true;
    }
    if let Some(list) = args.get("connect").and_then(Json::as_array) {
        for c in list {
            let from = resolve(a, c.get("from").ok_or("connect[].from")?)?;
            let to = resolve(a, c.get("to").ok_or("connect[].to")?)?;
            let output = c
                .get("output")
                .and_then(Json::as_str)
                .unwrap_or("Out")
                .to_owned();
            // Block inputs: {block, input} or a port id.
            let input = match (
                c.get("block").and_then(Json::as_u64),
                c.get("input").and_then(Json::as_str),
            ) {
                (Some(b), Some(i)) => oxide_vfx::port_id(b as u32, i),
                (None, Some(i)) => i.to_owned(),
                _ => return Err("connect[].input".into()),
            };
            if a.graph.creates_cycle(from, to) {
                return Err(format!("{from} -> {to} would make a loop"));
            }
            a.graph.connect(from, &output, to, &input);
        }
        edited = true;
    }
    if let Some(list) = args.get("delete").and_then(Json::as_array) {
        let ids: Result<Vec<u32>, String> = list.iter().map(|v| resolve(a, v)).collect();
        a.remove_nodes(&ids?);
        edited = true;
    }
    let assets = engine.world.resource_mut::<AssetServer>();
    if edited {
        w.commit(assets);
        w.compile();
    }
    if args.get("frame").and_then(Json::as_bool) == Some(true) {
        w.canvas.frame_all = true;
    }
    let mut out = summary(w);
    if !created.is_empty() {
        out["created"] = json!(created);
    }
    if !new_blocks.is_empty() {
        out["new_blocks"] = json!(new_blocks);
    }
    if args.get("save").and_then(Json::as_bool) == Some(true) {
        w.save(assets)?;
        out["dirty"] = json!(false);
    }
    Ok(out)
}
