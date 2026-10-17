//! The Shader Graph window: the node canvas with Unity's Blackboard, Graph Inspector
//! (Node Settings / Graph Settings) and Main Preview, live node previews, Save Asset and
//! its own undo. Opens `.shadergraph` and `.shadersubgraph` files.
//! Reference: <https://docs.unity3d.com/Packages/com.unity.shadergraph@17.0/manual/Shader-Graph-Window.html>

use crate::EditorState;
use crate::node_canvas::{self, CanvasModel, CanvasView, Template};
use egui::{Color32, Rect, RichText, pos2, vec2};
use oxide_assets::{AssetServer, Handle, Material, PrimitiveType, Shader, SurfaceShader};
use oxide_core::reflect::Reflect;
use oxide_engine::Engine;
use oxide_math::{Vector2, Vector4};
use oxide_render::{
    CameraView, ClearMode, DrawItem, Environment, PostSettings, ProjectionKind, RenderFrame,
    SceneRenderer, TonemapMode, wgpu,
};
use oxide_shadergraph::nodes::{self, Opt};
use oxide_shadergraph::{
    GraphData, GraphKeyword, GraphNode, GraphProperty, PortCx, PropertyType, ShaderGraph,
    ShaderSubGraph, Slot, SubGraphOutput, Target, Ty,
};
use serde_json::{Value as Json, json};
use std::collections::HashMap;

const MAIN_GRAPH: &str = "runtime:shadergraph-window/main.shadergraph";
const MAIN_MATERIAL: &str = "runtime:shadergraph-window/main.mat";
const PREVIEW_SHADER: &str = "runtime:shadergraph-window/previews.shadergraph";
/// Node preview size in pixels (one cell of the preview atlas).
const CELL: u32 = 96;

/// The open document.
#[derive(Clone, Debug, PartialEq)]
pub enum Doc {
    Graph(ShaderGraph),
    Sub(ShaderSubGraph),
}

impl Doc {
    pub fn data(&self) -> &GraphData {
        match self {
            Doc::Graph(g) => &g.graph,
            Doc::Sub(s) => &s.graph,
        }
    }

    pub fn data_mut(&mut self) -> &mut GraphData {
        match self {
            Doc::Graph(g) => &mut g.graph,
            Doc::Sub(s) => &mut s.graph,
        }
    }

    pub fn properties(&self) -> &[GraphProperty] {
        match self {
            Doc::Graph(g) => &g.properties,
            Doc::Sub(s) => &s.properties,
        }
    }

    pub fn properties_mut(&mut self) -> &mut Vec<GraphProperty> {
        match self {
            Doc::Graph(g) => &mut g.properties,
            Doc::Sub(s) => &mut s.properties,
        }
    }

    pub fn keywords(&self) -> &[GraphKeyword] {
        match self {
            Doc::Graph(g) => &g.keywords,
            Doc::Sub(_) => &[],
        }
    }

    fn outputs(&self) -> &[SubGraphOutput] {
        match self {
            Doc::Graph(_) => &[],
            Doc::Sub(s) => &s.outputs,
        }
    }

    /// The document as a graph the compiler and previews use.
    pub fn as_graph(&self) -> ShaderGraph {
        match self {
            Doc::Graph(g) => g.clone(),
            Doc::Sub(s) => oxide_shadergraph::sub_graph_as_graph(s),
        }
    }

    fn boxed(&self) -> Box<dyn oxide_assets::DataAsset> {
        match self {
            Doc::Graph(g) => Box::new(g.clone()),
            Doc::Sub(s) => Box::new(s.clone()),
        }
    }
}

#[derive(Default)]
pub struct ShaderGraphWindow {
    pub path: Option<String>,
    pub doc: Option<Doc>,
    pub dirty: bool,
    undo: Vec<Doc>,
    redo: Vec<Doc>,
    /// The document at the last undo step.
    before: Option<Doc>,
    pub canvas: CanvasView,
    hide_blackboard: bool,
    hide_inspector: bool,
    hide_preview: bool,
    /// Graph Inspector tab: false = Node Settings, true = Graph Settings.
    graph_settings: bool,
    selected_property: Option<usize>,
    selected_keyword: Option<usize>,
    /// Sub Graphs the document uses, and every Sub Graph in the project (searcher).
    subs: HashMap<String, ShaderSubGraph>,
    subs_stamp: Vec<(String, Option<u64>)>,
    project_subs: Vec<String>,
    project_subs_at: f64,
    // Previews.
    renderer: Option<SceneRenderer>,
    previews_stale: bool,
    uses_time: bool,
    rendered_at: f64,
    node_atlas: Option<egui::TextureHandle>,
    node_uv: HashMap<u32, Rect>,
    main_tex: Option<egui::TextureHandle>,
    pub preview_mesh: Option<PrimitiveType>,
    yaw: f32,
    pitch: f32,
    generation: u64,
    pub errors: Vec<(u32, String)>,
}

impl ShaderGraphWindow {
    /// Opens a graph or Sub Graph file.
    pub fn open(&mut self, engine: &mut Engine, path: &str) -> Result<(), String> {
        let assets = engine.world.resource_mut::<AssetServer>();
        if !assets.data.contains(path) {
            assets.load_data(path)?;
        }
        let doc = if let Some(g) = assets
            .data
            .get(&Handle::<ShaderGraph>::from_path(path.to_owned()))
        {
            Doc::Graph(g.clone())
        } else if let Some(s) = assets
            .data
            .get(&Handle::<ShaderSubGraph>::from_path(path.to_owned()))
        {
            Doc::Sub(s.clone())
        } else {
            return Err(format!("{path} is not a Shader Graph"));
        };
        self.path = Some(path.to_owned());
        self.before = Some(doc.clone());
        self.doc = Some(doc);
        self.dirty = false;
        self.undo.clear();
        self.redo.clear();
        self.canvas.reset();
        self.selected_property = None;
        self.selected_keyword = None;
        self.previews_stale = true;
        self.errors.clear();
        Ok(())
    }

    /// Records an undo step after an edit.
    fn commit(&mut self) {
        let Some(doc) = &self.doc else { return };
        if let Some(b) = self.before.take()
            && b != *doc
        {
            self.undo.push(b);
            self.redo.clear();
            self.dirty = true;
            if self.undo.len() > 200 {
                self.undo.remove(0);
            }
        }
        self.before = self.doc.clone();
        self.previews_stale = true;
    }

    fn undo(&mut self) {
        if let (Some(prev), Some(doc)) = (self.undo.pop(), self.doc.take()) {
            self.redo.push(doc);
            self.before = Some(prev.clone());
            self.doc = Some(prev);
            self.dirty = true;
            self.previews_stale = true;
        }
    }

    fn redo(&mut self) {
        if let (Some(next), Some(doc)) = (self.redo.pop(), self.doc.take()) {
            self.undo.push(doc);
            self.before = Some(next.clone());
            self.doc = Some(next);
            self.dirty = true;
            self.previews_stale = true;
        }
    }

    /// Unity's Save Asset: writes the file (materials using it recompile).
    pub fn save(&mut self, engine: &mut Engine) -> Result<(), String> {
        let (Some(path), Some(doc)) = (&self.path, &self.doc) else {
            return Err("no Shader Graph is open".into());
        };
        engine
            .world
            .resource_mut::<AssetServer>()
            .save_data(path, doc.boxed())?;
        self.dirty = false;
        Ok(())
    }
}

struct Model<'a> {
    doc: &'a mut Doc,
    subs: &'a HashMap<String, ShaderSubGraph>,
    project_subs: &'a [String],
    previews: &'a HashMap<u32, Rect>,
    atlas: Option<egui::TextureId>,
    errors: &'a [(u32, String)],
}

fn stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.split('.').next().unwrap_or(name)
}

impl CanvasModel for Model<'_> {
    fn data(&self) -> &GraphData {
        self.doc.data()
    }

    fn data_mut(&mut self) -> &mut GraphData {
        self.doc.data_mut()
    }

    fn ports(&self, node: &GraphNode) -> (Vec<Slot>, Vec<Slot>) {
        let subs = self.subs;
        let lookup = |p: &str| subs.get(p).cloned();
        oxide_shadergraph::ports(
            node,
            &PortCx {
                properties: self.doc.properties(),
                keywords: self.doc.keywords(),
                sub_outputs: self.doc.outputs(),
                subgraphs: &lookup,
            },
        )
        .unwrap_or_default()
    }

    fn title(&self, node: &GraphNode) -> String {
        match node.kind.as_str() {
            "Property" => {
                let r = node.option("Property").unwrap_or("");
                self.doc
                    .properties()
                    .iter()
                    .find(|p| p.reference == r)
                    .map_or_else(|| format!("{r} (missing)"), |p| p.name.clone())
            }
            "Keyword" => {
                let r = node.option("Keyword").unwrap_or("");
                self.doc
                    .keywords()
                    .iter()
                    .find(|k| k.reference == r)
                    .map_or_else(|| format!("{r} (missing)"), |k| k.name.clone())
            }
            "Sub Graph" => stem(node.option("Sub Graph").unwrap_or("Sub Graph")).to_owned(),
            "SubGraphOutput" => "Output".into(),
            k => k.to_owned(),
        }
    }

    fn options(&self, node: &GraphNode) -> Vec<Opt> {
        nodes::find(&node.kind).map_or_else(Vec::new, |d| d.options.clone())
    }

    fn fixed(&self, node: &GraphNode) -> bool {
        oxide_shadergraph::is_stack(&node.kind) || node.kind == "SubGraphOutput"
    }

    fn width(&self, node: &GraphNode) -> f32 {
        match node.kind.as_str() {
            "Vertex" | "Fragment" => 230.0,
            "Property" | "Keyword" => 160.0,
            "Custom Function" | "Sub Graph" | "SubGraphOutput" => 210.0,
            _ => 190.0,
        }
    }

    fn accent(&self, node: &GraphNode) -> Color32 {
        let menu = nodes::find(&node.kind).map_or("", |d| d.menu);
        match menu.split('/').next().unwrap_or("") {
            _ if oxide_shadergraph::is_stack(&node.kind) => Color32::from_rgb(90, 160, 200),
            "Input" => Color32::from_rgb(110, 190, 110),
            "Math" => Color32::from_rgb(90, 140, 220),
            "UV" => Color32::from_rgb(220, 200, 90),
            "Procedural" => Color32::from_rgb(170, 110, 220),
            "Artistic" => Color32::from_rgb(230, 150, 80),
            "Channel" => Color32::from_rgb(200, 110, 140),
            _ => Color32::from_gray(140),
        }
    }

    fn preview(&self, node: &GraphNode) -> Option<(egui::TextureId, Rect)> {
        Some((self.atlas?, *self.previews.get(&node.id)?))
    }

    fn has_preview(&self, node: &GraphNode) -> bool {
        !matches!(
            node.kind.as_str(),
            "Property" | "Keyword" | "Vertex" | "Fragment" | "SubGraphOutput" | "Texture 2D Asset"
        ) && nodes::find(&node.kind).is_some_and(|d| d.preview)
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
        let mut out: Vec<Template> = nodes::library()
            .iter()
            .filter(|d| {
                !d.menu.is_empty() && !matches!(d.kind, "Property" | "Sub Graph" | "Keyword")
            })
            .map(|d| Template {
                path: format!("{}/{}", d.menu, d.kind),
                kind: d.kind.into(),
                options: Vec::new(),
                preview: d.preview,
            })
            .collect();
        for p in self.doc.properties() {
            out.push(Template {
                path: format!("Properties/{}", p.name),
                kind: "Property".into(),
                options: vec![("Property".into(), p.reference.clone())],
                preview: false,
            });
        }
        for k in self.doc.keywords() {
            out.push(Template {
                path: format!("Keywords/{}", k.name),
                kind: "Keyword".into(),
                options: vec![("Keyword".into(), k.reference.clone())],
                preview: true,
            });
        }
        for s in self.project_subs {
            out.push(Template {
                path: format!("Sub Graph Assets/{}", stem(s)),
                kind: "Sub Graph".into(),
                options: vec![("Sub Graph".into(), s.clone())],
                preview: true,
            });
        }
        out
    }

    fn can_connect(&self, from: Ty, to: Ty) -> bool {
        nodes::convert("x", from, to).is_some()
    }
}

/// The value a new property of a type starts with.
fn new_property(doc: &Doc, kind: PropertyType) -> GraphProperty {
    let name = match kind {
        PropertyType::Float => "Float",
        PropertyType::Vector2 => "Vector2",
        PropertyType::Vector3 => "Vector3",
        PropertyType::Vector4 => "Vector4",
        PropertyType::Color => "Color",
        PropertyType::Boolean => "Boolean",
        PropertyType::Texture2D => "Texture2D",
    };
    let mut n = name.to_owned();
    let mut k = 1;
    while doc.properties().iter().any(|p| p.name == n) {
        n = format!("{name} ({k})");
        k += 1;
    }
    GraphProperty {
        reference: oxide_shadergraph::unique_reference(doc.properties(), doc.keywords(), &n),
        name: n,
        kind,
        default: if kind == PropertyType::Color {
            Vector4::ONE
        } else {
            Vector4::ZERO
        },
        ..Default::default()
    }
}

pub fn window_ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    let w = &mut st.shader_graph;
    let Some(path) = w.path.clone() else {
        ui.centered_and_justified(|ui| {
            ui.weak("Double-click a Shader Graph or Sub Graph in the Project window to edit it.");
        });
        return;
    };
    if w.doc.is_none() {
        return;
    }
    // Sub Graphs this graph uses (and the project's, for the searcher).
    {
        let assets = engine.world.resource_mut::<AssetServer>();
        let data = w.doc.as_ref().expect("doc").data().clone();
        oxide_shadergraph::load_sub_graphs(assets, &data);
        let assets = engine.world.resource::<AssetServer>();
        w.subs =
            oxide_shadergraph::sub_graph_paths(&data, &|p| oxide_shadergraph::sub_graph(assets, p))
                .into_iter()
                .filter_map(|p| oxide_shadergraph::sub_graph(assets, &p).map(|s| (p, s)))
                .collect();
        // Previews follow Sub Graphs that load late or are saved elsewhere.
        let mut stamp: Vec<(String, Option<u64>)> = w
            .subs
            .keys()
            .map(|p| (p.clone(), assets.data.version(p)))
            .collect();
        stamp.sort();
        if stamp != w.subs_stamp {
            w.subs_stamp = stamp;
            w.previews_stale = true;
        }
        let now = oxide_core::time::realtime_since_startup();
        if now - w.project_subs_at > 2.0 {
            w.project_subs = assets
                .paths_of_type("ShaderSubGraph")
                .into_iter()
                .filter(|p| *p != path)
                .collect();
            w.project_subs_at = now;
        }
    }

    // ---- Toolbar.
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
        let can_convert = matches!(st.shader_graph.doc, Some(Doc::Graph(_)))
            && !st.shader_graph.canvas.selected.is_empty();
        if ui
            .add_enabled(can_convert, egui::Button::new("Convert To Sub Graph"))
            .on_hover_text("Moves the selected nodes into a new Sub Graph asset")
            .clicked()
        {
            st.status = match convert_selection(&mut st.shader_graph, engine, &path) {
                Ok(p) => format!("Converted to {p}"),
                Err(e) => e,
            };
        }
        ui.separator();
        let w = &mut st.shader_graph;
        for (hidden, label) in [
            (&mut w.hide_blackboard, "Blackboard"),
            (&mut w.hide_inspector, "Graph Inspector"),
            (&mut w.hide_preview, "Main Preview"),
        ] {
            let mut shown = !*hidden;
            if ui.toggle_value(&mut shown, label).changed() {
                *hidden = !shown;
            }
        }
        ui.separator();
        let title = format!(
            "{}{}",
            path.rsplit('/').next().unwrap_or(&path),
            if w.dirty { " *" } else { "" }
        );
        ui.label(RichText::new(title).strong());
        let n = w.errors.len();
        if n > 0 {
            ui.colored_label(Color32::from_rgb(240, 110, 100), format!("{n} error(s)"));
        }
    });
    let w = &mut st.shader_graph;

    // ---- Canvas.
    let atlas = w.node_atlas.as_ref().map(|t| t.id());
    let mut doc = w.doc.take().expect("doc");
    let out = {
        let mut model = Model {
            doc: &mut doc,
            subs: &w.subs,
            project_subs: &w.project_subs,
            previews: &w.node_uv,
            atlas,
            errors: &w.errors,
        };
        node_canvas::show(ui, &mut w.canvas, &mut model, "shadergraph")
    };
    w.doc = Some(doc);
    if out.edited {
        w.previews_stale = true;
    }
    if out.commit {
        w.commit();
    }
    if out.undo {
        w.undo();
    }
    if out.redo {
        w.redo();
    }
    if out.save {
        save = true;
    }
    // Dropping a Blackboard property makes a Property node.
    if let Some(resp) = &out.response
        && let Some(payload) = resp.dnd_release_payload::<BlackboardItem>()
        && let Some(p) = ui.input(|i| i.pointer.hover_pos())
    {
        let at = w.canvas.to_graph(out.rect, p);
        let (kind, opt) = match &*payload {
            BlackboardItem::Property(r) => ("Property", ("Property", r.clone())),
            BlackboardItem::Keyword(r) => ("Keyword", ("Keyword", r.clone())),
        };
        if let Some(d) = w.doc.as_mut() {
            let id = d
                .data_mut()
                .add(GraphNode::new(0, kind, at).with_option(opt.0, &opt.1));
            w.canvas.selected = vec![id];
        }
        w.commit();
    }
    // Double-clicking a Sub Graph node opens it.
    if let Some(id) = out.open
        && let Some(sub) = w
            .doc
            .as_ref()
            .and_then(|d| d.data().node(id))
            .filter(|n| n.kind == "Sub Graph")
            .and_then(|n| n.option("Sub Graph").map(str::to_owned))
        && let Err(e) = w.open(engine, &sub)
    {
        st.status = e;
    }
    let w = &mut st.shader_graph;
    let canvas = out.rect;

    if !w.hide_blackboard {
        blackboard(ui, w, canvas, &path);
    }
    if !w.hide_inspector {
        graph_inspector(ui, w, engine, canvas);
    }
    update_previews(w, engine, ui.ctx());
    if !w.hide_preview {
        main_preview(ui, w, canvas);
    }
    if save {
        let r = st.shader_graph.save(engine);
        st.status = match r {
            Ok(()) => format!("Saved {path}"),
            Err(e) => e,
        };
    }
}

#[derive(Clone, Debug)]
enum BlackboardItem {
    Property(String),
    Keyword(String),
}

fn blackboard(ui: &mut egui::Ui, w: &mut ShaderGraphWindow, canvas: Rect, path: &str) {
    let mut changed = false;
    egui::Window::new("Blackboard")
        .id(egui::Id::new("sg-blackboard"))
        .constrain_to(canvas)
        .default_pos(canvas.min + vec2(10.0, 10.0))
        .default_width(220.0)
        .collapsible(true)
        .resizable(true)
        .show(ui.ctx(), |ui| {
            ui.label(RichText::new(stem(path)).strong());
            ui.weak(if matches!(w.doc, Some(Doc::Sub(_))) {
                "Sub Graph"
            } else {
                "Shader Graphs"
            });
            let Some(doc) = w.doc.as_mut() else { return };
            ui.horizontal(|ui| {
                ui.menu_button("+", |ui| {
                    for kind in [
                        PropertyType::Float,
                        PropertyType::Vector2,
                        PropertyType::Vector3,
                        PropertyType::Vector4,
                        PropertyType::Color,
                        PropertyType::Boolean,
                        PropertyType::Texture2D,
                    ] {
                        if ui.button(kind.kind_name()).clicked() {
                            let p = new_property(doc, kind);
                            doc.properties_mut().push(p);
                            w.selected_property = Some(doc.properties().len() - 1);
                            w.selected_keyword = None;
                            w.graph_settings = false;
                            changed = true;
                            ui.close();
                        }
                    }
                    if let Doc::Graph(g) = doc {
                        ui.separator();
                        for (label, kind) in [
                            ("Boolean", oxide_shadergraph::KeywordType::Boolean),
                            ("Enum", oxide_shadergraph::KeywordType::Enum),
                        ] {
                            if !ui.button(format!("Keyword/{label}")).clicked() {
                                continue;
                            }
                            let mut name = format!("{label} Keyword");
                            let mut k = 1;
                            while g.keywords.iter().any(|x| x.name == name) {
                                name = format!("{label} Keyword ({k})");
                                k += 1;
                            }
                            let reference = oxide_shadergraph::unique_reference(
                                &g.properties,
                                &g.keywords,
                                &name.to_uppercase().replace(' ', "_"),
                            );
                            g.keywords.push(GraphKeyword {
                                name,
                                reference,
                                default: false,
                                exposed: true,
                                keyword_type: kind,
                                entries: if kind == oxide_shadergraph::KeywordType::Enum {
                                    vec!["A".into(), "B".into(), "C".into()]
                                } else {
                                    Vec::new()
                                },
                                ..Default::default()
                            });
                            w.selected_keyword = Some(g.keywords.len() - 1);
                            w.selected_property = None;
                            changed = true;
                            ui.close();
                        }
                    }
                });
            });
            ui.separator();
            let mut remove = None;
            let mut move_up = None;
            for (k, p) in doc.properties().iter().enumerate() {
                let sel = w.selected_property == Some(k);
                let r = ui
                    .dnd_drag_source(
                        egui::Id::new(("sg-prop", k)),
                        BlackboardItem::Property(p.reference.clone()),
                        |ui| {
                            ui.horizontal(|ui| {
                                let _ = ui.selectable_label(sel, &p.name);
                                ui.weak(p.kind.kind_name());
                            });
                        },
                    )
                    .response;
                if r.clicked() {
                    w.selected_property = Some(k);
                    w.selected_keyword = None;
                    w.graph_settings = false;
                }
                r.context_menu(|ui| {
                    if ui.button("Move Up").clicked() {
                        move_up = Some(k);
                        ui.close();
                    }
                    if ui.button("Delete").clicked() {
                        remove = Some(k);
                        ui.close();
                    }
                });
            }
            if let Some(k) = move_up
                && k > 0
            {
                doc.properties_mut().swap(k, k - 1);
                changed = true;
            }
            if let Some(k) = remove {
                let r = doc.properties()[k].reference.clone();
                doc.properties_mut().remove(k);
                // Unity deletes the property's nodes too.
                let ids: Vec<u32> = doc
                    .data()
                    .nodes
                    .iter()
                    .filter(|n| n.kind == "Property" && n.option("Property") == Some(r.as_str()))
                    .map(|n| n.id)
                    .collect();
                doc.data_mut().remove(&ids);
                w.selected_property = None;
                changed = true;
            }
            if let Doc::Graph(g) = doc
                && !g.keywords.is_empty()
            {
                ui.separator();
                ui.weak("Keywords");
                let mut remove = None;
                for (k, kw) in g.keywords.iter().enumerate() {
                    let r = ui
                        .dnd_drag_source(
                            egui::Id::new(("sg-kw", k)),
                            BlackboardItem::Keyword(kw.reference.clone()),
                            |ui| {
                                ui.horizontal(|ui| {
                                    let _ = ui
                                        .selectable_label(w.selected_keyword == Some(k), &kw.name);
                                    ui.weak("Boolean");
                                });
                            },
                        )
                        .response;
                    if r.clicked() {
                        w.selected_keyword = Some(k);
                        w.selected_property = None;
                        w.graph_settings = false;
                    }
                    r.context_menu(|ui| {
                        if ui.button("Delete").clicked() {
                            remove = Some(k);
                            ui.close();
                        }
                    });
                }
                if let Some(k) = remove {
                    g.keywords.remove(k);
                    w.selected_keyword = None;
                    changed = true;
                }
            }
        });
    if changed {
        w.commit();
    }
}

fn graph_inspector(ui: &mut egui::Ui, w: &mut ShaderGraphWindow, engine: &Engine, canvas: Rect) {
    let mut changed = false;
    let mut commit = false;
    egui::Window::new("Graph Inspector")
        .id(egui::Id::new("sg-inspector"))
        .constrain_to(canvas)
        .default_pos(pos2(canvas.right() - 300.0, canvas.top() + 10.0))
        .default_width(280.0)
        .collapsible(true)
        .resizable(true)
        .show(ui.ctx(), |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut w.graph_settings, false, "Node Settings");
                ui.selectable_value(&mut w.graph_settings, true, "Graph Settings");
            });
            ui.separator();
            let world = &engine.world;
            let Some(doc) = w.doc.as_mut() else { return };
            if w.graph_settings {
                match doc {
                    Doc::Graph(g) => {
                        ui.weak("Target: Universal (URP)");
                        if crate::inspector::reflect_grid(ui, world, &mut g.settings, "sg-settings") {
                            changed = true;
                            commit = true;
                        }
                    }
                    Doc::Sub(s) => {
                        ui.label(RichText::new("Outputs").strong());
                        let mut outputs = s.outputs.clone();
                        let mut remove = None;
                        for (k, o) in outputs.iter_mut().enumerate() {
                            ui.horizontal(|ui| {
                                let r = ui.add(egui::TextEdit::singleline(&mut o.name).desired_width(100.0));
                                if r.lost_focus() {
                                    commit = true;
                                }
                                egui::ComboBox::from_id_salt(("sg-out", k))
                                    .selected_text(o.kind.kind_name())
                                    .show_ui(ui, |ui| {
                                        for kind in [
                                            PropertyType::Float,
                                            PropertyType::Vector2,
                                            PropertyType::Vector3,
                                            PropertyType::Vector4,
                                            PropertyType::Boolean,
                                        ] {
                                            if ui.selectable_value(&mut o.kind, kind, kind.kind_name()).clicked() {
                                                commit = true;
                                            }
                                        }
                                    });
                                if ui.small_button("×").clicked() {
                                    remove = Some(k);
                                }
                            });
                        }
                        if let Some(k) = remove {
                            outputs.remove(k);
                            commit = true;
                        }
                        if ui.button("Add Output").clicked() {
                            let mut name = "Out".to_owned();
                            let mut k = 1;
                            while outputs.iter().any(|o| o.name == name) {
                                name = format!("Out_{k}");
                                k += 1;
                            }
                            outputs.push(SubGraphOutput {
                                name,
                                kind: PropertyType::Vector4,
                            });
                            commit = true;
                        }
                        if outputs != s.outputs {
                            s.outputs = outputs;
                            changed = true;
                        }
                    }
                }
                return;
            }
            // Node Settings: the selected property, keyword, node, group or note.
            if let Some(k) = w.selected_property
                && k < doc.properties().len()
            {
                let mut p = doc.properties()[k].clone();
                let old_ref = p.reference.clone();
                if crate::inspector::reflect_grid(ui, world, &mut p, "sg-prop") {
                    if p.reference != old_ref {
                        // Property nodes follow a renamed reference.
                        for n in &mut doc.data_mut().nodes {
                            if n.kind == "Property" && n.option("Property") == Some(old_ref.as_str()) {
                                n.set_option("Property", &p.reference);
                            }
                        }
                    }
                    doc.properties_mut()[k] = p;
                    changed = true;
                    commit = true;
                }
                return;
            }
            if let (Some(k), Doc::Graph(g)) = (w.selected_keyword, &mut *doc)
                && k < g.keywords.len()
            {
                let mut kw = g.keywords[k].clone();
                let old_ref = kw.reference.clone();
                if crate::inspector::reflect_grid(ui, world, &mut kw, "sg-kw") {
                    for n in &mut g.graph.nodes {
                        if n.kind == "Keyword" && n.option("Keyword") == Some(old_ref.as_str()) {
                            n.set_option("Keyword", &kw.reference);
                        }
                    }
                    g.keywords[k] = kw;
                    changed = true;
                    commit = true;
                }
                return;
            }
            if let Some(k) = w.canvas.selected_group
                && let Some(g) = doc.data_mut().groups.get_mut(k)
            {
                ui.label("Group");
                if ui.text_edit_singleline(&mut g.title).changed() {
                    changed = true;
                }
                commit |= ui.input(|i| i.key_pressed(egui::Key::Enter));
                return;
            }
            if let Some(k) = w.canvas.selected_note
                && let Some(n) = doc.data_mut().notes.get_mut(k)
            {
                ui.label("Sticky Note");
                changed |= ui.text_edit_singleline(&mut n.title).changed();
                changed |= ui.text_edit_multiline(&mut n.text).changed();
                return;
            }
            let [id] = w.canvas.selected[..] else {
                ui.weak(if w.canvas.selected.is_empty() {
                    "Select a node or a Blackboard item."
                } else {
                    "Several nodes selected."
                });
                return;
            };
            let Some(node) = doc.data_mut().node_mut(id) else { return };
            ui.label(RichText::new(&node.kind).strong());
            if let Some(def) = nodes::find(&node.kind) {
                for o in &def.options {
                    let cur = node.option(o.name).unwrap_or(o.default).to_owned();
                    ui.horizontal(|ui| {
                        crate::inspector::prop_label(ui, o.name);
                        if o.name == "Texture" || o.name == "Sub Graph" {
                            let ty = if o.name == "Texture" { "Texture" } else { "ShaderSubGraph" };
                            let mut v = if cur.is_empty() {
                                oxide_core::reflect::Value::Null
                            } else {
                                oxide_core::reflect::Value::Asset(cur.clone())
                            };
                            if crate::inspector::asset_field(ui, world, Some(ty), &mut v, &format!("sg-opt-{id}-{}", o.name)) {
                                let s = match v {
                                    oxide_core::reflect::Value::Asset(p) => p,
                                    _ => String::new(),
                                };
                                node.set_option(o.name, &s);
                                changed = true;
                                commit = true;
                            }
                        } else if !o.choices.is_empty() {
                            egui::ComboBox::from_id_salt(("sg-opt", id, o.name))
                                .selected_text(&cur)
                                .show_ui(ui, |ui| {
                                    for c in o.choices {
                                        if ui.selectable_label(*c == cur, *c).clicked() {
                                            node.set_option(o.name, c);
                                            changed = true;
                                            commit = true;
                                        }
                                    }
                                });
                        } else {
                            let mut s = cur.clone();
                            let r = if o.multiline {
                                ui.add(egui::TextEdit::multiline(&mut s).code_editor().desired_rows(6))
                            } else {
                                ui.text_edit_singleline(&mut s)
                            };
                            if r.changed() {
                                node.set_option(o.name, &s);
                                changed = true;
                            }
                            if r.lost_focus() {
                                commit = true;
                            }
                        }
                    });
                }
                if node.kind == "Custom Function" {
                    ui.weak("Inputs/Outputs: `Name:Type; ...` (Float, Vector 2/3/4, Boolean, Texture 2D). The body is WGSL that assigns the outputs; the inputs and `i` (SurfaceInput) are in scope.");
                }
            }
            ui.horizontal(|ui| {
                if ui.checkbox(&mut node.preview, "Show Preview").changed() {
                    changed = true;
                    commit = true;
                }
            });
            if let Some((_, e)) = w.errors.iter().find(|(n, _)| *n == id) {
                ui.colored_label(Color32::from_rgb(240, 110, 100), e);
            }
        });
    if changed {
        w.previews_stale = true;
    }
    if commit {
        w.commit();
    }
}

fn main_preview(ui: &mut egui::Ui, w: &mut ShaderGraphWindow, canvas: Rect) {
    egui::Window::new("Main Preview")
        .id(egui::Id::new("sg-main-preview"))
        .constrain_to(canvas)
        .default_pos(pos2(canvas.right() - 230.0, canvas.bottom() - 260.0))
        .default_size(vec2(210.0, 210.0))
        .collapsible(true)
        .resizable(true)
        .show(ui.ctx(), |ui| {
            let side = ui.available_width().clamp(100.0, 600.0);
            let (r, resp) = ui.allocate_exact_size(vec2(side, side), egui::Sense::click_and_drag());
            match &w.main_tex {
                Some(t) => {
                    ui.painter().image(
                        t.id(),
                        r,
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                None => {
                    ui.painter().rect_filled(r, 0.0, Color32::from_gray(49));
                }
            }
            if resp.dragged() {
                let d = resp.drag_delta();
                w.yaw += d.x * 0.6;
                w.pitch = (w.pitch + d.y * 0.6).clamp(-89.0, 89.0);
                w.previews_stale = true;
            }
            resp.context_menu(|ui| {
                for (name, m) in [
                    ("Sphere", PrimitiveType::Sphere),
                    ("Cube", PrimitiveType::Cube),
                    ("Quad", PrimitiveType::Quad),
                    ("Capsule", PrimitiveType::Capsule),
                    ("Cylinder", PrimitiveType::Cylinder),
                ] {
                    if ui
                        .selectable_label(
                            w.preview_mesh.unwrap_or(PrimitiveType::Sphere) == m,
                            name,
                        )
                        .clicked()
                    {
                        w.preview_mesh = Some(m);
                        w.previews_stale = true;
                        ui.close();
                    }
                }
            });
        });
}

/// Nodes whose result depends on the surface's shape preview on a sphere (Unity's 3D
/// preview mode); the rest on a quad.
fn preview_3d(
    data: &GraphData,
    subs: &HashMap<String, ShaderSubGraph>,
) -> std::collections::HashSet<u32> {
    const GEOMETRY: [&str; 12] = [
        "Position",
        "Normal Vector",
        "Tangent Vector",
        "Bitangent Vector",
        "View Direction",
        "View Vector",
        "Fresnel Effect",
        "Transform",
        "Object",
        "Camera",
        "Normal From Height",
        "Triplanar",
    ];
    let sub_3d = |p: &str| {
        subs.get(p).is_some_and(|s| {
            s.graph
                .nodes
                .iter()
                .any(|n| GEOMETRY.contains(&n.kind.as_str()))
        })
    };
    let mut out = std::collections::HashSet::new();
    for n in &data.nodes {
        let mut stack = vec![n.id];
        let mut seen = Vec::new();
        while let Some(id) = stack.pop() {
            if seen.contains(&id) {
                continue;
            }
            seen.push(id);
            let Some(m) = data.node(id) else { continue };
            if GEOMETRY.contains(&m.kind.as_str())
                || (m.kind == "Sub Graph" && sub_3d(m.option("Sub Graph").unwrap_or("")))
            {
                out.insert(n.id);
                break;
            }
            stack.extend(data.edges.iter().filter(|e| e.to == id).map(|e| e.from));
        }
    }
    out
}

/// Recompiles and renders the node previews and Main Preview when the graph changed (and
/// a few times a second for graphs that use Time).
fn update_previews(w: &mut ShaderGraphWindow, engine: &mut Engine, ctx: &egui::Context) {
    let now = oxide_core::time::realtime_since_startup();
    let animate = w.uses_time && now - w.rendered_at > 1.0 / 15.0;
    if !(w.previews_stale || animate) {
        return;
    }
    let Some(doc) = &w.doc else { return };
    w.previews_stale = false;
    w.rendered_at = now;
    let graph = doc.as_graph();
    let subs = w.subs.clone();
    let lookup = move |p: &str| subs.get(p).cloned();
    w.uses_time = graph.graph.nodes.iter().any(|n| n.kind == "Time")
        || w.subs
            .values()
            .any(|s| s.graph.nodes.iter().any(|n| n.kind == "Time"));
    w.generation += 1;
    let time = now as f32;

    // Main preview: the graph as a real material.
    let assets = engine.world.resource_mut::<AssetServer>();
    assets
        .data
        .insert(MAIN_GRAPH, "ShaderGraph", Box::new(graph.clone()));
    oxide_shadergraph::compile_asset(assets, MAIN_GRAPH);
    let errors = assets
        .shaders
        .handle_of(MAIN_GRAPH)
        .and_then(|h| assets.shaders.get(&h))
        .and_then(|s| s.surface.as_ref())
        .map(|s| s.errors.clone())
        .unwrap_or_default();
    let mut mat = Material::lit("Shader Graph Preview", oxide_math::Color::WHITE);
    mat.shader_graph = Some(Handle::from_path(MAIN_GRAPH.to_owned()));
    assets.materials.insert(MAIN_MATERIAL, mat);

    // Node previews: one shader showing the node picked by the preview slot.
    let c = oxide_shadergraph::compile(&graph, &lookup, Target::Preview);
    for p in &c.properties {
        if let Some(t) = &p.default_texture
            && let Err(e) = assets.ensure_loaded(t.path())
        {
            oxide_core::debug_log_warning!("{e}");
        }
    }
    assets.shaders.insert(
        PREVIEW_SHADER,
        Shader {
            name: "Shader Graph node previews".into(),
            source: c.hooks.clone(),
            surface: Some(SurfaceShader {
                unlit: true,
                properties: c.properties.clone(),
                generation: w.generation,
                stamp: w.generation,
                ..Default::default()
            }),
        },
    );
    let sphere = preview_3d(&graph.graph, &w.subs);
    let count = c.preview_nodes.len().max(1) as u32;
    let cols = (count as f32).sqrt().ceil() as u32;
    let rows = count.div_ceil(cols);
    let mut draws = Vec::new();
    for (k, node) in c.preview_nodes.iter().enumerate() {
        let path = format!("runtime:shadergraph-window/preview{k}.mat");
        let mut m = Material::lit("preview", oxide_math::Color::WHITE);
        m.shader_graph = Some(Handle::from_path(PREVIEW_SHADER.to_owned()));
        m.set_float(oxide_shadergraph::PREVIEW_PROPERTY, k as f32);
        let id = assets.materials.insert(path, m).id();
        let (cx, cy) = ((k as u32 % cols) as f32, (k as u32 / cols) as f32);
        draws.push(DrawItem {
            id: k as u64,
            mesh: oxide_assets::AssetId::from_path(
                &if sphere.contains(node) {
                    PrimitiveType::Sphere
                } else {
                    PrimitiveType::Quad
                }
                .mesh_path(),
            ),
            submesh: 0,
            material: id,
            model: oxide_math::Matrix4x4::from_translation(oxide_math::Vector3::new(
                cx - cols as f32 * 0.5 + 0.5,
                rows as f32 * 0.5 - cy - 0.5,
                0.0,
            )) * oxide_math::Matrix4x4::from_scale(oxide_math::Vector3::splat(
                if sphere.contains(node) { 0.98 } else { 1.0 },
            )),
            cast_shadows: false,
            shadows_only: false,
            receive_shadows: false,
            layer: 0,
            joint_offset: None,
            occluder: false,
            occludee: false,
            reflection_probe_static: false,
            lightmap: None,
            light_probe: None,
            sorting: Default::default(),
            rendering_layers: u32::MAX,
        });
    }
    let frame = RenderFrame {
        cameras: vec![CameraView {
            position: oxide_math::Vector3::new(0.0, 0.0, -5.0),
            rotation: oxide_math::Quaternion::IDENTITY,
            projection: ProjectionKind::Orthographic {
                size: rows as f32 * 0.5,
            },
            near: 0.1,
            far: 100.0,
            viewport: [0.0, 0.0, 1.0, 1.0],
            clear: ClearMode::Color(oxide_math::Color::BLACK),
            culling_mask: u32::MAX,
            occlusion_culling: false,
            antialiasing: Default::default(),
        }],
        draws,
        environment: Environment {
            post: PostSettings {
                tonemapping: TonemapMode::None,
                ..PostSettings::neutral()
            },
            ..Default::default()
        },
        time,
        ..Default::default()
    };
    let gpu = &engine.gpu;
    let renderer = w.renderer.get_or_insert_with(|| SceneRenderer::new(gpu));
    let fmt = wgpu::TextureFormat::Rgba8UnormSrgb;
    let (iw, ih) = (cols * CELL, rows * CELL);
    let assets = engine.world.resource::<AssetServer>();
    if !c.preview_nodes.is_empty()
        && let Ok(img) = oxide_render::capture_frame(gpu, fmt, iw, ih, |view, _| {
            renderer.render(gpu, assets, &frame, view, fmt, iw, ih);
        })
    {
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [img.width as usize, img.height as usize],
            &img.rgba,
        );
        match &mut w.node_atlas {
            Some(t) => t.set(image, egui::TextureOptions::LINEAR),
            None => {
                w.node_atlas = Some(ctx.load_texture(
                    "shadergraph-previews",
                    image,
                    egui::TextureOptions::LINEAR,
                ))
            }
        }
        w.node_uv = c
            .preview_nodes
            .iter()
            .enumerate()
            .map(|(k, id)| {
                let (cx, cy) = ((k as u32 % cols) as f32, (k as u32 / cols) as f32);
                let (u, v) = (1.0 / cols as f32, 1.0 / rows as f32);
                (*id, Rect::from_min_size(pos2(cx * u, cy * v), vec2(u, v)))
            })
            .collect();
    }
    w.errors = errors;
    if !w.hide_preview {
        let mesh = w.preview_mesh.unwrap_or(PrimitiveType::Sphere);
        match crate::previews::material_frame(
            engine,
            MAIN_MATERIAL,
            mesh,
            w.pitch + 20.0,
            w.yaw - 150.0,
            time,
        ) {
            Ok(frame) => {
                let size = 256;
                let assets = engine.world.resource::<AssetServer>();
                let gpu = &engine.gpu;
                if let Ok(img) = oxide_render::capture_frame(gpu, fmt, size, size, |view, _| {
                    renderer.render(gpu, assets, &frame, view, fmt, size, size);
                }) {
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [img.width as usize, img.height as usize],
                        &img.rgba,
                    );
                    match &mut w.main_tex {
                        Some(t) => t.set(image, egui::TextureOptions::LINEAR),
                        None => {
                            w.main_tex = Some(ctx.load_texture(
                                "shadergraph-main",
                                image,
                                egui::TextureOptions::LINEAR,
                            ))
                        }
                    }
                }
            }
            Err(e) => oxide_core::debug_log_warning!("Shader Graph preview: {e}"),
        }
    }
    if w.uses_time {
        ctx.request_repaint();
    }
}

// ---- MCP: `editor_shader_graph`.

fn node_json(doc: &Doc, n: &GraphNode, subs: &HashMap<String, ShaderSubGraph>) -> Json {
    let lookup = |p: &str| subs.get(p).cloned();
    let (ins, outs) = oxide_shadergraph::ports(
        n,
        &PortCx {
            properties: doc.properties(),
            keywords: doc.keywords(),
            sub_outputs: doc.outputs(),
            subgraphs: &lookup,
        },
    )
    .unwrap_or_default();
    json!({
        "id": n.id,
        "kind": n.kind,
        "position": [n.position.x, n.position.y],
        "inputs": ins.iter().map(|s| json!({"id": s.id, "type": s.ty.label()})).collect::<Vec<_>>(),
        "outputs": outs.iter().map(|s| json!({"id": s.id, "type": s.ty.label()})).collect::<Vec<_>>(),
        "values": n.values.iter().map(|v| (v.slot.clone(), json!(v.value.to_array()))).collect::<serde_json::Map<_, _>>(),
        "options": n.options.iter().map(|o| (o.name.clone(), json!(o.value))).collect::<serde_json::Map<_, _>>(),
    })
}

fn summary(w: &ShaderGraphWindow) -> Json {
    let Some(doc) = &w.doc else {
        return json!({ "open": null });
    };
    json!({
        "open": w.path,
        "sub_graph": matches!(doc, Doc::Sub(_)),
        "dirty": w.dirty,
        "nodes": doc.data().nodes.iter().map(|n| node_json(doc, n, &w.subs)).collect::<Vec<_>>(),
        "edges": doc.data().edges.iter().map(|e| json!({"from": e.from, "output": e.output, "to": e.to, "input": e.input})).collect::<Vec<_>>(),
        "properties": doc.properties().iter().map(|p| json!({"name": p.name, "reference": p.reference, "type": p.kind.kind_name(), "default": p.default.to_array(), "exposed": p.exposed})).collect::<Vec<_>>(),
        "keywords": doc.keywords().iter().map(|k| json!({"name": k.name, "reference": k.reference, "default": k.default, "type": format!("{:?}", k.keyword_type), "entries": k.entries, "definition": format!("{:?}", k.definition), "scope": format!("{:?}", k.scope)})).collect::<Vec<_>>(),
        "errors": w.errors.iter().map(|(n, e)| json!({"node": n, "error": e})).collect::<Vec<_>>(),
        "selected": w.canvas.selected,
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

fn property_type(s: &str) -> Option<PropertyType> {
    Some(match s.replace(' ', "").to_ascii_lowercase().as_str() {
        "float" => PropertyType::Float,
        "vector2" => PropertyType::Vector2,
        "vector3" => PropertyType::Vector3,
        "vector4" => PropertyType::Vector4,
        "color" => PropertyType::Color,
        "boolean" | "bool" => PropertyType::Boolean,
        "texture2d" | "texture" => PropertyType::Texture2D,
        _ => return None,
    })
}

/// The `editor_shader_graph` tool: open, inspect and edit the graph in the window.
pub fn mcp(
    st: &mut EditorState,
    engine: &mut Engine,
    args: &serde_json::Map<String, Json>,
) -> Result<Json, String> {
    let w = &mut st.shader_graph;
    if let Some(p) = args.get("open").and_then(Json::as_str) {
        w.open(engine, p)?;
        st.actions
            .push(crate::Action::OpenTab(crate::Tab::ShaderGraph));
    }
    let w = &mut st.shader_graph;
    // Convert To Sub Graph (with `select` naming the nodes).
    if args.get("convert_to_sub_graph").and_then(Json::as_bool) == Some(true) {
        if let Some(sel) = args.get("select").and_then(Json::as_array) {
            w.canvas.selected = sel
                .iter()
                .filter_map(|v| v.as_u64().map(|x| x as u32))
                .collect();
        }
        let path = w.path.clone().ok_or("open a Shader Graph")?;
        convert_selection(w, engine, &path)?;
    }
    let mut edited = false;
    if let Some(doc) = w.doc.as_mut() {
        // Blackboard.
        if let Some(props) = args.get("add_property").and_then(Json::as_object) {
            let kind = props
                .get("type")
                .and_then(Json::as_str)
                .and_then(property_type)
                .ok_or("add_property.type: Float, Vector2, Vector3, Vector4, Color, Boolean or Texture2D")?;
            let mut p = new_property(doc, kind);
            if let Some(n) = props.get("name").and_then(Json::as_str) {
                p.name = n.to_owned();
                p.reference = props
                    .get("reference")
                    .and_then(Json::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        oxide_shadergraph::unique_reference(doc.properties(), doc.keywords(), n)
                    });
            }
            if let Some(v) = props.get("default").and_then(vec4) {
                p.default = v;
            }
            if let Some(t) = props.get("texture").and_then(Json::as_str) {
                p.default_texture = Some(Handle::from_path(t.to_owned()));
            }
            doc.properties_mut().push(p);
            edited = true;
        }
        if let Some(p) = args.get("set_property").and_then(Json::as_object) {
            let r = p
                .get("reference")
                .and_then(Json::as_str)
                .ok_or("set_property.reference")?;
            let prop = doc
                .properties_mut()
                .iter_mut()
                .find(|x| x.reference == r)
                .ok_or_else(|| format!("no property `{r}`"))?;
            if let Some(v) = p.get("default").and_then(vec4) {
                prop.default = v;
            }
            if let Some(n) = p.get("name").and_then(Json::as_str) {
                prop.name = n.to_owned();
            }
            if let Some(e) = p.get("exposed").and_then(Json::as_bool) {
                prop.exposed = e;
            }
            if let Some(t) = p.get("texture").and_then(Json::as_str) {
                prop.default_texture = (!t.is_empty()).then(|| Handle::from_path(t.to_owned()));
            }
            if let Some(r) = p.get("range").and_then(Json::as_array)
                && r.len() == 2
            {
                prop.float_mode = oxide_shadergraph::FloatMode::Slider;
                prop.range = Vector2::new(
                    r[0].as_f64().unwrap_or(0.0) as f32,
                    r[1].as_f64().unwrap_or(1.0) as f32,
                );
            }
            edited = true;
        }
        if let Some(k) = args.get("add_keyword").and_then(Json::as_str)
            && let Doc::Graph(g) = doc
        {
            let reference = oxide_shadergraph::unique_reference(
                &g.properties,
                &g.keywords,
                &k.to_uppercase().replace(' ', "_"),
            );
            let entries: Vec<String> = args
                .get("entries")
                .and_then(Json::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|e| e.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            g.keywords.push(GraphKeyword {
                name: k.to_owned(),
                reference,
                default: false,
                exposed: true,
                keyword_type: if entries.is_empty() {
                    oxide_shadergraph::KeywordType::Boolean
                } else {
                    oxide_shadergraph::KeywordType::Enum
                },
                entries,
                scope: if args.get("global").and_then(Json::as_bool) == Some(true) {
                    oxide_shadergraph::KeywordScope::Global
                } else {
                    oxide_shadergraph::KeywordScope::Local
                },
                ..Default::default()
            });
            edited = true;
        }
        if let Some(s) = args.get("settings").and_then(Json::as_object)
            && let Doc::Graph(g) = doc
        {
            for (k, v) in s {
                let info = g.settings.type_info();
                let f = info
                    .fields
                    .iter()
                    .find(|f| f.name == k)
                    .ok_or_else(|| format!("settings: no field `{k}`"))?;
                let v = oxide_engine::json_values::json_to_value(f, v, &engine.world)?;
                g.settings
                    .set_field(k, v)
                    .map_err(|e| format!("settings.{k}: {e}"))?;
            }
            edited = true;
        }
        // Nodes.
        let mut created = Vec::new();
        if let Some(list) = args.get("add_nodes").and_then(Json::as_array) {
            for n in list {
                let kind = n
                    .get("kind")
                    .and_then(Json::as_str)
                    .ok_or("add_nodes[].kind")?;
                if nodes::find(kind).is_none() {
                    return Err(format!("unknown node `{kind}`"));
                }
                let pos = n
                    .get("position")
                    .and_then(Json::as_array)
                    .map(|a| {
                        Vector2::new(
                            a[0].as_f64().unwrap_or(0.0) as f32,
                            a.get(1).and_then(Json::as_f64).unwrap_or(0.0) as f32,
                        )
                    })
                    .unwrap_or(Vector2::new(
                        -300.0 - 40.0 * created.len() as f32,
                        40.0 * created.len() as f32,
                    ));
                let mut node = GraphNode::new(0, kind, pos);
                node.preview = nodes::find(kind).is_some_and(|d| d.preview);
                if let Some(o) = n.get("options").and_then(Json::as_object) {
                    for (k, v) in o {
                        node.set_option(k, v.as_str().unwrap_or(&v.to_string()));
                    }
                }
                if let Some(o) = n.get("values").and_then(Json::as_object) {
                    for (k, v) in o {
                        node.set_value(
                            k,
                            vec4(v)
                                .ok_or_else(|| format!("values.{k}: a number or [x, y, z, w]"))?,
                        );
                    }
                }
                created.push(doc.data_mut().add(node));
            }
            edited = true;
        }
        // Node references: an id, or "$k" for the k-th node added by this call, or a stack name.
        let resolve = |doc: &Doc, v: &Json| -> Result<u32, String> {
            match v {
                Json::Number(n) => n
                    .as_u64()
                    .map(|x| x as u32)
                    .ok_or_else(|| "bad node id".into()),
                Json::String(s) if s.starts_with('$') => s[1..]
                    .parse::<usize>()
                    .ok()
                    .and_then(|k| created.get(k).copied())
                    .ok_or_else(|| format!("no new node {s}")),
                Json::String(s) => doc
                    .data()
                    .find(if s == "Output" { "SubGraphOutput" } else { s })
                    .map(|n| n.id)
                    .ok_or_else(|| format!("no node `{s}`")),
                _ => Err("node: an id, \"$k\" or \"Fragment\"/\"Vertex\"/\"Output\"".into()),
            }
        };
        if let Some(list) = args.get("set_nodes").and_then(Json::as_array) {
            for n in list {
                let id = resolve(doc, n.get("node").ok_or("set_nodes[].node")?)?;
                let node = doc
                    .data_mut()
                    .node_mut(id)
                    .ok_or_else(|| format!("no node {id}"))?;
                if let Some(o) = n.get("options").and_then(Json::as_object) {
                    for (k, v) in o {
                        node.set_option(k, v.as_str().unwrap_or(&v.to_string()));
                    }
                }
                if let Some(o) = n.get("values").and_then(Json::as_object) {
                    for (k, v) in o {
                        node.set_value(k, vec4(v).ok_or_else(|| format!("values.{k}"))?);
                    }
                }
                if let Some(p) = n.get("position").and_then(Json::as_array) {
                    node.position = Vector2::new(
                        p[0].as_f64().unwrap_or(0.0) as f32,
                        p.get(1).and_then(Json::as_f64).unwrap_or(0.0) as f32,
                    );
                }
            }
            edited = true;
        }
        if let Some(list) = args.get("connect").and_then(Json::as_array) {
            for c in list {
                let from = resolve(doc, c.get("from").ok_or("connect[].from")?)?;
                let to = resolve(doc, c.get("to").ok_or("connect[].to")?)?;
                let output = c.get("output").and_then(Json::as_str).unwrap_or("Out");
                let input = c
                    .get("input")
                    .and_then(Json::as_str)
                    .ok_or("connect[].input")?;
                if doc.data().creates_cycle(from, to) {
                    return Err(format!("{from} -> {to} would make a loop"));
                }
                doc.data_mut().connect(from, output, to, input);
            }
            edited = true;
        }
        if let Some(list) = args.get("disconnect").and_then(Json::as_array) {
            for c in list {
                let to = resolve(doc, c.get("to").ok_or("disconnect[].to")?)?;
                let input = c
                    .get("input")
                    .and_then(Json::as_str)
                    .ok_or("disconnect[].input")?;
                doc.data_mut().disconnect(to, input);
            }
            edited = true;
        }
        if let Some(list) = args.get("delete").and_then(Json::as_array) {
            let ids: Result<Vec<u32>, String> = list.iter().map(|v| resolve(doc, v)).collect();
            doc.data_mut().remove(&ids?);
            edited = true;
        }
        if let Some(sel) = args.get("select").and_then(Json::as_array) {
            w.canvas.selected = sel.iter().filter_map(|v| resolve(doc, v).ok()).collect();
        }
        if edited {
            w.commit();
            // Errors for this edit right away.
            let assets = engine.world.resource_mut::<AssetServer>();
            let graph = w.doc.as_ref().expect("doc").as_graph();
            oxide_shadergraph::load_sub_graphs(assets, &graph.graph);
            let assets = engine.world.resource::<AssetServer>();
            let c = oxide_shadergraph::compile(
                &graph,
                &|p| oxide_shadergraph::sub_graph(assets, p),
                Target::Material,
            );
            w.errors = c.errors;
            w.subs = oxide_shadergraph::sub_graph_paths(&graph.graph, &|p| {
                oxide_shadergraph::sub_graph(assets, p)
            })
            .into_iter()
            .filter_map(|p| oxide_shadergraph::sub_graph(assets, &p).map(|s| (p, s)))
            .collect();
        }
        if args.get("frame").and_then(Json::as_bool) == Some(true) {
            w.canvas.frame_all = true;
        }
        if args.get("undo").and_then(Json::as_bool) == Some(true) {
            w.undo();
        }
        if args.get("redo").and_then(Json::as_bool) == Some(true) {
            w.redo();
        }
        let mut out = summary(w);
        if !created.is_empty() {
            out["created"] = json!(created);
        }
        if args.get("save").and_then(Json::as_bool) == Some(true) {
            w.save(engine)?;
            out["dirty"] = json!(false);
        }
        if args.get("wgsl").and_then(Json::as_bool) == Some(true)
            && let Some(path) = &w.path
        {
            let assets = engine.world.resource::<AssetServer>();
            out["wgsl"] = json!(
                assets
                    .shaders
                    .handle_of(path)
                    .and_then(|h| assets.shaders.get(&h))
                    .map(|s| s.source.clone())
            );
        }
        return Ok(out);
    }
    Ok(summary(w))
}

/// Every node type for MCP clients (`editor_shader_graph` with `library: true`).
pub fn library_json() -> Json {
    json!(nodes::library()
        .iter()
        .filter(|d| !d.menu.is_empty())
        .map(|d| json!({
            "kind": d.kind,
            "category": d.menu,
            "inputs": d.inputs.iter().map(|s| json!({"id": s.id, "type": s.ty.label()})).collect::<Vec<_>>(),
            "outputs": d.outputs.iter().map(|s| json!({"id": s.id, "type": s.ty.label()})).collect::<Vec<_>>(),
            "options": d.options.iter().map(|o| json!({"name": o.name, "choices": o.choices, "default": o.default})).collect::<Vec<_>>(),
        }))
        .collect::<Vec<_>>())
}

/// Unity's Convert To Sub Graph: saves the selection as `<graph> Sub Graph.shadersubgraph`
/// next to the graph and puts a Sub Graph node in its place (one undo step).
fn convert_selection(
    w: &mut ShaderGraphWindow,
    engine: &mut Engine,
    graph_path: &str,
) -> Result<String, String> {
    let Some(Doc::Graph(g)) = &mut w.doc else {
        return Err("open a Shader Graph".into());
    };
    let dir = graph_path.rsplit_once('/').map_or("Assets", |(d, _)| d);
    let assets = engine.world.resource_mut::<AssetServer>();
    let mut sub_path = format!("{dir}/{} Sub Graph.shadersubgraph", stem(graph_path));
    let mut k = 1;
    while assets.full_path(&sub_path).exists() {
        k += 1;
        sub_path = format!("{dir}/{} Sub Graph {k}.shadersubgraph", stem(graph_path));
    }
    let subs = &w.subs;
    let lookup = |p: &str| subs.get(p).cloned();
    let selection = w.canvas.selected.clone();
    let (sub, node) = oxide_shadergraph::convert_to_sub_graph(g, &selection, &lookup, &sub_path)?;
    assets.save_data(&sub_path, Box::new(sub.clone()))?;
    w.subs.insert(sub_path.clone(), sub);
    w.canvas.selected = vec![node];
    w.commit();
    Ok(sub_path)
}
