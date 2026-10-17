//! The UI Builder (Unity's visual UXML/USS editor): StyleSheets and Hierarchy on the left,
//! the Library of controls, the Viewport (the live document, click to select, Preview to
//! try it), and an Inspector for the selected element's attributes and inline styles or
//! a selected USS rule. Save writes the `.uxml` and its style sheets.
//! Reference: <https://docs.unity3d.com/Manual/UIBuilder.html>

use crate::EditorState;
use egui::{Color32, RichText, pos2, vec2};
use oxide_assets::AssetServer;
use oxide_engine::Engine;
use oxide_uitk::uss::{self, Rule, StyleSheet};
use oxide_uitk::uxml::{self, UxmlElement, VisualTree};
use oxide_uitk::{CONTROLS, Panel, PanelInput, SheetRef};
use serde_json::{Value as Json, json};
use std::sync::Arc;

#[derive(Clone, PartialEq)]
struct Snapshot {
    tree: VisualTree,
    sheets: Vec<(String, StyleSheet)>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Selection {
    Element(Vec<usize>),
    /// (sheet index, rule index)
    Rule(usize, usize),
}

pub struct UiBuilderWindow {
    pub path: Option<String>,
    tree: VisualTree,
    /// Attached style sheets (path, parsed), in order.
    sheets: Vec<(String, StyleSheet)>,
    pub selection: Option<Selection>,
    pub dirty: bool,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    panel: Option<Panel>,
    rebuild: bool,
    pub canvas: egui::Vec2,
    pub preview: bool,
    new_selector: String,
    new_property: (String, String),
}

impl Default for UiBuilderWindow {
    fn default() -> Self {
        Self {
            path: None,
            tree: VisualTree::default(),
            sheets: Vec::new(),
            selection: None,
            dirty: false,
            undo: Vec::new(),
            redo: Vec::new(),
            panel: None,
            rebuild: true,
            canvas: vec2(1200.0, 800.0),
            preview: false,
            new_selector: ".new-class".into(),
            new_property: (String::new(), String::new()),
        }
    }
}

fn folder(path: &str) -> String {
    path.rsplit_once('/')
        .map_or(String::new(), |(d, _)| d.to_owned())
}

impl UiBuilderWindow {
    pub fn open(&mut self, assets: &AssetServer, path: &str) -> Result<(), String> {
        let text =
            std::fs::read_to_string(assets.full_path(path)).map_err(|e| format!("{path}: {e}"))?;
        let tree = uxml::parse(&text, &folder(path)).map_err(|e| format!("{path}: {e}"))?;
        let mut sheets = Vec::new();
        for s in &tree.styles {
            let t = std::fs::read_to_string(assets.full_path(s)).unwrap_or_default();
            sheets.push((s.clone(), uss::parse(&t).0));
        }
        *self = Self {
            path: Some(path.to_owned()),
            tree,
            sheets,
            canvas: self.canvas,
            ..Default::default()
        };
        Ok(())
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            tree: self.tree.clone(),
            sheets: self.sheets.clone(),
        }
    }

    /// Call before an edit: records an undo step.
    fn record(&mut self) {
        self.undo.push(self.snapshot());
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.dirty = true;
        self.rebuild = true;
    }

    fn restore(&mut self, s: Snapshot) {
        self.tree = s.tree;
        self.sheets = s.sheets;
        self.dirty = true;
        self.rebuild = true;
    }

    fn undo(&mut self) {
        if let Some(s) = self.undo.pop() {
            self.redo.push(self.snapshot());
            self.restore(s);
        }
    }

    fn redo(&mut self) {
        if let Some(s) = self.redo.pop() {
            self.undo.push(self.snapshot());
            self.restore(s);
        }
    }

    /// Writes the `.uxml` and every attached style sheet.
    pub fn save(&mut self, engine: &mut Engine) -> Result<(), String> {
        let Some(path) = self.path.clone() else {
            return Err("no UI Document is open".into());
        };
        let assets = engine.world.resource::<AssetServer>();
        std::fs::write(
            assets.full_path(&path),
            uxml::write(&self.tree, &folder(&path)),
        )
        .map_err(|e| e.to_string())?;
        for (p, s) in &self.sheets {
            std::fs::write(assets.full_path(p), uss::write(s)).map_err(|e| e.to_string())?;
        }
        if let Some(cache) = engine.world.try_resource_mut::<oxide_uitk::UiAssets>() {
            cache.invalidate(&path);
            for (p, _) in &self.sheets {
                cache.invalidate(p);
            }
        }
        self.dirty = false;
        Ok(())
    }

    /// The element to add new elements into (the selection, else the root).
    fn insert_parent(&self) -> Vec<usize> {
        match &self.selection {
            Some(Selection::Element(p)) => {
                let e = self.tree.root.at(p);
                // Controls that hold no children add next to themselves.
                match e {
                    Some(e)
                        if matches!(
                            e.kind.as_str(),
                            "VisualElement" | "ScrollView" | "GroupBox" | "Foldout"
                        ) =>
                    {
                        p.clone()
                    }
                    _ => p[..p.len().saturating_sub(1)].to_vec(),
                }
            }
            _ => Vec::new(),
        }
    }

    /// Adds an element of `kind` with Unity's starting attributes; returns its path.
    pub fn add(&mut self, kind: &str) -> Vec<usize> {
        self.record();
        let parent = self.insert_parent();
        let mut e = UxmlElement::new(kind);
        match kind {
            "Label" => e.set_attr("text", "Label"),
            "Button" => e.set_attr("text", "Button"),
            "Toggle" => e.set_attr("label", "Toggle"),
            "RadioButton" => e.set_attr("label", "Radio Button"),
            "Slider" | "SliderInt" => {
                e.set_attr("label", kind);
                e.set_attr("high-value", "100");
            }
            "ProgressBar" => {
                e.set_attr("title", "my-progress");
                e.set_attr("value", "22");
            }
            "DropdownField" => {
                e.set_attr("label", "Dropdown");
                e.set_attr("choices", "Option A,Option B,Option C");
                e.set_attr("index", "0");
            }
            "TextField" | "IntegerField" | "FloatField" => e.set_attr("label", kind),
            "Foldout" => e.set_attr("text", "Foldout"),
            _ => {}
        }
        let p = self.tree.root.at_mut(&parent).expect("parent");
        p.children.push(e);
        let mut path = parent;
        path.push(p.children.len() - 1);
        self.selection = Some(Selection::Element(path.clone()));
        path
    }

    fn delete(&mut self, path: &[usize]) {
        if path.is_empty() {
            return;
        }
        self.record();
        let (last, parent) = path.split_last().expect("not root");
        if let Some(p) = self.tree.root.at_mut(parent)
            && *last < p.children.len()
        {
            p.children.remove(*last);
        }
        self.selection = None;
    }

    fn duplicate(&mut self, path: &[usize]) {
        if path.is_empty() {
            return;
        }
        self.record();
        let (last, parent) = path.split_last().expect("not root");
        if let Some(p) = self.tree.root.at_mut(parent)
            && let Some(e) = p.children.get(*last).cloned()
        {
            p.children.insert(last + 1, e);
            let mut np = parent.to_vec();
            np.push(last + 1);
            self.selection = Some(Selection::Element(np));
        }
    }

    fn move_by(&mut self, path: &[usize], dir: i32) {
        let Some((last, parent)) = path.split_last() else {
            return;
        };
        let Some(p) = self.tree.root.at(parent) else {
            return;
        };
        let to = *last as i32 + dir;
        if to < 0 || to as usize >= p.children.len() {
            return;
        }
        self.record();
        let p = self.tree.root.at_mut(parent).expect("parent");
        p.children.swap(*last, to as usize);
        let mut np = parent.to_vec();
        np.push(to as usize);
        self.selection = Some(Selection::Element(np));
    }

    fn sheet_refs(&self, theme: &Arc<StyleSheet>) -> Vec<SheetRef> {
        std::iter::once(SheetRef {
            sheet: theme.clone(),
            base: String::new(),
        })
        .chain(self.sheets.iter().map(|(p, s)| SheetRef {
            sheet: Arc::new(s.clone()),
            base: folder(p),
        }))
        .collect()
    }
}

fn tree_ui(
    ui: &mut egui::Ui,
    w: &mut UiBuilderWindow,
    e: &UxmlElement,
    path: &mut Vec<usize>,
    actions: &mut Vec<(&'static str, Vec<usize>)>,
) {
    for (i, c) in e.children.iter().enumerate() {
        path.push(i);
        let label = if c.name.is_empty() {
            c.kind.clone()
        } else {
            format!("{}  #{}", c.kind, c.name)
        };
        let label = if c.classes.is_empty() {
            label
        } else {
            format!("{label}  .{}", c.classes.join(" ."))
        };
        let sel = w.selection == Some(Selection::Element(path.clone()));
        let indent = 12.0 * (path.len() - 1) as f32;
        let r = ui
            .horizontal(|ui| {
                ui.add_space(indent);
                ui.selectable_label(sel, label)
            })
            .inner;
        if r.clicked() {
            w.selection = Some(Selection::Element(path.clone()));
        }
        let p = path.clone();
        r.context_menu(|ui| {
            for (label, a) in [
                ("Move Up", "up"),
                ("Move Down", "down"),
                ("Duplicate", "duplicate"),
                ("Delete", "delete"),
            ] {
                if ui.button(label).clicked() {
                    actions.push((a, p.clone()));
                    ui.close();
                }
            }
        });
        tree_ui(ui, w, c, path, actions);
        path.pop();
    }
}

/// Attributes the Inspector offers for a type (besides name and class).
fn attributes_of(kind: &str) -> &'static [&'static str] {
    match kind {
        "Label" | "Button" => &["text", "tooltip"],
        "Toggle" | "RadioButton" => &["label", "text", "value"],
        "Slider" | "SliderInt" => &["label", "low-value", "high-value", "value"],
        "ProgressBar" => &["title", "low-value", "high-value", "value"],
        "DropdownField" => &["label", "choices", "index"],
        "TextField" => &["label", "value", "max-length", "password"],
        "IntegerField" | "FloatField" => &["label", "value"],
        "Foldout" => &["text", "value"],
        _ => &["tooltip"],
    }
}

pub fn window_ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    let w = &mut st.ui_builder;
    let Some(path) = w.path.clone() else {
        ui.centered_and_justified(|ui| {
            ui.weak(
                "Double-click a UI Document (.uxml) in the Project window to open the UI Builder.",
            );
        });
        return;
    };
    // ---- Toolbar.
    let mut save = false;
    ui.horizontal(|ui| {
        save |= ui.button("Save").clicked();
        ui.separator();
        ui.label(
            RichText::new(format!(
                "{}{}",
                path.rsplit('/').next().unwrap_or(&path),
                if w.dirty { " *" } else { "" }
            ))
            .strong(),
        );
        ui.separator();
        egui::ComboBox::from_id_salt("uib-size")
            .selected_text(format!("{} x {}", w.canvas.x, w.canvas.y))
            .show_ui(ui, |ui| {
                for (x, y) in [
                    (1920.0, 1080.0),
                    (1280.0, 720.0),
                    (1200.0, 800.0),
                    (800.0, 600.0),
                    (390.0, 844.0),
                ] {
                    if ui
                        .selectable_label(w.canvas == vec2(x, y), format!("{x} x {y}"))
                        .clicked()
                    {
                        w.canvas = vec2(x, y);
                    }
                }
            });
        ui.toggle_value(&mut w.preview, "Preview")
            .on_hover_text("Try the controls (selection is off)");
        if ui.button("Undo").clicked() {
            w.undo();
        }
        if ui.button("Redo").clicked() {
            w.redo();
        }
    });
    ui.separator();
    // Shortcuts while the window is hovered.
    if ui.rect_contains_pointer(ui.max_rect()) && !ui.ctx().egui_wants_keyboard_input() {
        crate::shortcuts::claim_keys(ui.ctx());
        let keys: Vec<(egui::Key, egui::Modifiers)> = ui.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => Some((*key, *modifiers)),
                    _ => None,
                })
                .collect()
        });
        for (k, m) in keys {
            match (k, m.command) {
                (egui::Key::Z, true) if m.shift => w.redo(),
                (egui::Key::Z, true) => w.undo(),
                (egui::Key::S, true) => save = true,
                (egui::Key::D, true) => {
                    if let Some(Selection::Element(p)) = w.selection.clone() {
                        w.duplicate(&p);
                    }
                }
                (egui::Key::Delete | egui::Key::Backspace, false) => {
                    if let Some(Selection::Element(p)) = w.selection.clone() {
                        w.delete(&p);
                    }
                }
                _ => {}
            }
        }
    }

    // ---- Left: StyleSheets, Hierarchy, Library.
    let mut actions = Vec::new();
    egui::Panel::left(egui::Id::new("uib-left"))
        .resizable(true)
        .default_size(260.0)
        .show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                egui::CollapsingHeader::new("StyleSheets")
                    .default_open(true)
                    .show(ui, |ui| {
                        let mut add_rule = None;
                        for (si, (p, sheet)) in w.sheets.iter().enumerate() {
                            ui.label(RichText::new(p.rsplit('/').next().unwrap_or(p)).strong());
                            for (ri, r) in sheet.rules.iter().enumerate() {
                                let sel = w.selection == Some(Selection::Rule(si, ri));
                                if ui.selectable_label(sel, &r.selector_text).clicked() {
                                    w.selection = Some(Selection::Rule(si, ri));
                                }
                            }
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut w.new_selector)
                                        .desired_width(130.0),
                                );
                                if ui.small_button("Add Selector").clicked() {
                                    add_rule = Some(si);
                                }
                            });
                        }
                        if let Some(si) = add_rule {
                            match uss::parse_selector(&w.new_selector) {
                                Ok(sel) => {
                                    w.record();
                                    let text = w.new_selector.clone();
                                    w.sheets[si].1.rules.push(Rule {
                                        selector_text: text,
                                        selectors: vec![sel],
                                        declarations: Vec::new(),
                                    });
                                    w.selection =
                                        Some(Selection::Rule(si, w.sheets[si].1.rules.len() - 1));
                                }
                                Err(e) => st.status = e,
                            }
                        }
                        if w.sheets.is_empty() && ui.button("Add new USS").clicked() {
                            let uss_path = path.trim_end_matches(".uxml").to_owned() + ".uss";
                            w.record();
                            w.tree.styles.push(uss_path.clone());
                            w.sheets.push((uss_path, StyleSheet::default()));
                        }
                    });
                egui::CollapsingHeader::new("Hierarchy")
                    .default_open(true)
                    .show(ui, |ui| {
                        let root_sel = w.selection == Some(Selection::Element(Vec::new()));
                        if ui
                            .selectable_label(
                                root_sel,
                                RichText::new(path.rsplit('/').next().unwrap_or(&path)).strong(),
                            )
                            .clicked()
                        {
                            w.selection = Some(Selection::Element(Vec::new()));
                        }
                        let root = w.tree.root.clone();
                        tree_ui(ui, w, &root, &mut Vec::new(), &mut actions);
                    });
                egui::CollapsingHeader::new("Library")
                    .default_open(true)
                    .show(ui, |ui| {
                        let mut group = "";
                        for (g, kind) in CONTROLS {
                            if *g != group {
                                ui.weak(*g);
                                group = g;
                            }
                            if ui
                                .button(*kind)
                                .on_hover_text("Add to the selected element")
                                .clicked()
                            {
                                w.add(kind);
                            }
                        }
                    });
            });
        });
    for (a, p) in actions {
        match a {
            "up" => w.move_by(&p, -1),
            "down" => w.move_by(&p, 1),
            "duplicate" => w.duplicate(&p),
            _ => w.delete(&p),
        }
    }

    // ---- Right: Inspector.
    egui::Panel::right(egui::Id::new("uib-inspector"))
        .resizable(true)
        .default_size(300.0)
        .show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| inspector(ui, w, &mut st.status));
        });

    // ---- Center: Viewport.
    let theme = Arc::new(uss::parse(uss::DEFAULT_THEME).0);
    let sheets = w.sheet_refs(&theme);
    let avail = ui.available_rect_before_wrap();
    let scale = (avail.width() / w.canvas.x)
        .min(avail.height() / w.canvas.y)
        .min(1.0)
        * 0.96;
    let canvas = egui::Rect::from_center_size(avail.center(), w.canvas * scale);
    let resp = ui.allocate_rect(avail, egui::Sense::click());
    let painter = ui.painter_at(avail);
    painter.rect_filled(avail, 0.0, Color32::from_gray(40));
    // Unity's checkerboard behind the document.
    let cell = 16.0;
    let mut y = canvas.top();
    let mut row = 0;
    while y < canvas.bottom() {
        let mut x = canvas.left() + if row % 2 == 0 { 0.0 } else { cell };
        while x < canvas.right() {
            let r = egui::Rect::from_min_size(pos2(x, y), vec2(cell, cell)).intersect(canvas);
            painter.rect_filled(r, 0.0, Color32::from_gray(58));
            x += cell * 2.0;
        }
        y += cell;
        row += 1;
    }
    painter.rect_stroke(
        canvas,
        0.0,
        egui::Stroke::new(1.0, Color32::from_gray(90)),
        egui::StrokeKind::Outside,
    );
    if w.rebuild || w.panel.is_none() {
        w.panel = Some(Panel::new(&w.tree));
        w.rebuild = false;
    }
    let panel = w.panel.as_mut().expect("panel");
    let pointer = ui
        .input(|i| i.pointer.latest_pos())
        .filter(|p| canvas.contains(*p));
    let to_panel = |p: egui::Pos2| ((p - canvas.min) / scale).to_pos2();
    let input = if w.preview && resp.hovered() {
        ui.input(|i| PanelInput {
            pointer: pointer.map(to_panel),
            pressed: i.pointer.primary_pressed(),
            released: i.pointer.primary_released(),
            down: i.pointer.primary_down(),
            scroll: i.smooth_scroll_delta.y / scale,
            keyboard: Vec::new(),
        })
    } else {
        PanelInput::default()
    };
    panel.input(&input);
    panel.style(&sheets);
    panel.layout(w.canvas, ui.ctx());
    let mut no_textures = |_: &str| None;
    panel.paint(
        &painter.with_clip_rect(canvas),
        canvas.min,
        scale,
        &mut no_textures,
    );
    // Selection outline and click-to-select.
    if let Some(Selection::Element(sel)) = &w.selection
        && let Some(e) = panel.elements.iter().find(|e| e.path == *sel)
        && e.rect.is_finite()
    {
        let r = egui::Rect::from_min_max(
            canvas.min + e.rect.min.to_vec2() * scale,
            canvas.min + e.rect.max.to_vec2() * scale,
        );
        painter.rect_stroke(
            r,
            0.0,
            egui::Stroke::new(2.0, Color32::from_rgb(68, 192, 255)),
            egui::StrokeKind::Outside,
        );
    }
    if !w.preview && resp.clicked() {
        let hit = pointer.and_then(|p| panel.hit(to_panel(p)));
        w.selection = hit.map(|i| Selection::Element(panel.elements[i].path.clone()));
    }
    if !w.preview
        && let Some(p) = pointer
        && let Some(i) = panel.hit(to_panel(p))
    {
        let e = &panel.elements[i];
        let r = egui::Rect::from_min_max(
            canvas.min + e.rect.min.to_vec2() * scale,
            canvas.min + e.rect.max.to_vec2() * scale,
        );
        painter.rect_stroke(
            r,
            0.0,
            egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(68, 192, 255, 120)),
            egui::StrokeKind::Outside,
        );
    }
    if save {
        st.status = match st.ui_builder.save(engine) {
            Ok(()) => format!("Saved {path}"),
            Err(e) => e,
        };
    }
}

fn inspector(ui: &mut egui::Ui, w: &mut UiBuilderWindow, status: &mut String) {
    match w.selection.clone() {
        None => {
            ui.weak(
                "Select an element in the Hierarchy or the Viewport, or a selector in StyleSheets.",
            );
        }
        Some(Selection::Rule(si, ri)) => {
            let Some(rule) = w.sheets.get(si).and_then(|s| s.1.rules.get(ri)).cloned() else {
                return;
            };
            ui.label(RichText::new("Selector").strong());
            let mut text = rule.selector_text.clone();
            if ui.text_edit_singleline(&mut text).lost_focus() && text != rule.selector_text {
                let parsed: Result<Vec<_>, _> = text
                    .split(',')
                    .map(|s| uss::parse_selector(s.trim()))
                    .collect();
                match parsed {
                    Ok(sels) => {
                        w.record();
                        let r = &mut w.sheets[si].1.rules[ri];
                        r.selector_text = text;
                        r.selectors = sels;
                    }
                    Err(e) => *status = e,
                }
            }
            ui.separator();
            ui.label(RichText::new("Styles").strong());
            let mut decls = rule.declarations.clone();
            if style_grid(ui, &mut decls, &mut w.new_property, "uib-rule") {
                w.record();
                w.sheets[si].1.rules[ri].declarations = decls;
            }
            if ui.button("Delete Selector").clicked() {
                w.record();
                w.sheets[si].1.rules.remove(ri);
                w.selection = None;
            }
        }
        Some(Selection::Element(p)) => {
            let Some(e) = w.tree.root.at(&p).cloned() else {
                return;
            };
            if p.is_empty() {
                ui.label(RichText::new("Document root").strong());
                ui.weak("Add elements from the Library.");
                return;
            }
            ui.label(RichText::new(&e.kind).strong());
            let mut edited = e.clone();
            ui.horizontal(|ui| {
                crate::inspector::prop_label(ui, "Name");
                ui.text_edit_singleline(&mut edited.name);
            });
            let mut classes = edited.classes.join(" ");
            ui.horizontal(|ui| {
                crate::inspector::prop_label(ui, "Class");
                if ui.text_edit_singleline(&mut classes).changed() {
                    edited.classes = classes.split_whitespace().map(str::to_owned).collect();
                }
            });
            ui.separator();
            ui.label(RichText::new("Attributes").strong());
            for a in attributes_of(&e.kind) {
                let mut v = edited.attr(a).unwrap_or("").to_owned();
                ui.horizontal(|ui| {
                    crate::inspector::prop_label(ui, *a);
                    if ui.text_edit_singleline(&mut v).changed() {
                        if v.is_empty() {
                            edited.remove_attr(a);
                        } else {
                            edited.set_attr(a, &v);
                        }
                    }
                });
            }
            let mut picking = edited.attr("picking-mode") != Some("Ignore");
            if ui
                .checkbox(&mut picking, "Picking Mode: Position")
                .changed()
            {
                if picking {
                    edited.remove_attr("picking-mode");
                } else {
                    edited.set_attr("picking-mode", "Ignore");
                }
            }
            let mut enabled = edited.attr("enabled") != Some("false");
            if ui.checkbox(&mut enabled, "Enabled").changed() {
                if enabled {
                    edited.remove_attr("enabled");
                } else {
                    edited.set_attr("enabled", "false");
                }
            }
            ui.separator();
            ui.label(RichText::new("Inline Styles").strong());
            let mut style = edited.style.clone();
            if style_grid(ui, &mut style, &mut w.new_property, "uib-inline") {
                edited.style = style;
            }
            ui.separator();
            ui.label(RichText::new("Matching Selectors").strong());
            for (sp, sheet) in &w.sheets {
                for r in &sheet.rules {
                    let names = (!e.name.is_empty()).then(|| format!("#{}", e.name));
                    let hit = r.selectors.iter().any(|s| {
                        s.parts.last().is_some_and(|(_, c)| {
                            c.kind.as_deref().is_none_or(|k| k == e.kind)
                                && c.name
                                    .as_ref()
                                    .is_none_or(|n| Some(format!("#{n}")) == names)
                                && c.classes.iter().all(|x| e.classes.contains(x))
                        })
                    });
                    if hit {
                        ui.weak(format!(
                            "{}  ({})",
                            r.selector_text,
                            sp.rsplit('/').next().unwrap_or(sp)
                        ));
                    }
                }
            }
            if edited != e {
                w.record();
                *w.tree.root.at_mut(&p).expect("element") = edited;
            }
        }
    }
}

/// Editable USS declarations: every property in a grid, plus custom ones.
fn style_grid(
    ui: &mut egui::Ui,
    decls: &mut Vec<(String, String)>,
    new: &mut (String, String),
    salt: &str,
) -> bool {
    let mut changed = false;
    egui::CollapsingHeader::new("Properties")
        .id_salt(salt)
        .default_open(true)
        .show(ui, |ui| {
            for prop in oxide_uitk::style::PROPERTIES {
                let cur = decls
                    .iter()
                    .find(|(k, _)| k == prop)
                    .map(|(_, v)| v.clone());
                let mut v = cur.clone().unwrap_or_default();
                ui.horizontal(|ui| {
                    let label = RichText::new(*prop);
                    crate::inspector::prop_label(
                        ui,
                        if cur.is_some() {
                            label.strong()
                        } else {
                            label.weak()
                        },
                    );
                    let r = ui.add(egui::TextEdit::singleline(&mut v).desired_width(110.0));
                    if r.lost_focus()
                        && Some(&v) != cur.as_ref()
                        && !(cur.is_none() && v.is_empty())
                    {
                        decls.retain(|(k, _)| k != prop);
                        if !v.trim().is_empty() {
                            decls.push(((*prop).to_owned(), v.trim().to_owned()));
                        }
                        changed = true;
                    }
                });
            }
            // Variables and anything else.
            for (k, v) in decls
                .clone()
                .iter()
                .filter(|(k, _)| !oxide_uitk::style::PROPERTIES.contains(&k.as_str()))
            {
                ui.horizontal(|ui| {
                    crate::inspector::prop_label(ui, RichText::new(k).strong());
                    let mut x = v.clone();
                    if ui
                        .add(egui::TextEdit::singleline(&mut x).desired_width(110.0))
                        .lost_focus()
                        && x != *v
                    {
                        if let Some(d) = decls.iter_mut().find(|(n, _)| n == k) {
                            d.1 = x;
                        }
                        changed = true;
                    }
                });
            }
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut new.0)
                        .hint_text("--variable")
                        .desired_width(100.0),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut new.1)
                        .hint_text("value")
                        .desired_width(80.0),
                );
                if ui.small_button("Add").clicked() && !new.0.trim().is_empty() {
                    decls.push((new.0.trim().to_owned(), new.1.trim().to_owned()));
                    new.0.clear();
                    new.1.clear();
                    changed = true;
                }
            });
        });
    changed
}

// ---- MCP: `editor_ui_builder`.

fn summary(w: &UiBuilderWindow) -> Json {
    let mut elements = Vec::new();
    w.tree.root.walk(&mut |p, e| {
        if !p.is_empty() {
            elements.push(json!({
                "path": p, "type": e.kind, "name": e.name, "classes": e.classes,
                "attributes": e.attrs.iter().map(|(k, v)| (k.clone(), json!(v))).collect::<serde_json::Map<_, _>>(),
                "style": e.style.iter().map(|(k, v)| (k.clone(), json!(v))).collect::<serde_json::Map<_, _>>(),
            }));
        }
    });
    json!({
        "open": w.path,
        "dirty": w.dirty,
        "elements": elements,
        "style_sheets": w.sheets.iter().map(|(p, s)| json!({
            "path": p,
            "rules": s.rules.iter().map(|r| json!({
                "selector": r.selector_text,
                "styles": r.declarations.iter().map(|(k, v)| (k.clone(), json!(v))).collect::<serde_json::Map<_, _>>(),
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "selection": match &w.selection {
            Some(Selection::Element(p)) => json!({"element": p}),
            Some(Selection::Rule(s, r)) => json!({"sheet": s, "rule": r}),
            None => Json::Null,
        },
    })
}

fn path_of(v: &Json, w: &UiBuilderWindow) -> Result<Vec<usize>, String> {
    match v {
        Json::Array(a) => a
            .iter()
            .map(|x| {
                x.as_u64()
                    .map(|n| n as usize)
                    .ok_or_else(|| "path: numbers".to_string())
            })
            .collect(),
        Json::String(name) => {
            let mut found = None;
            w.tree.root.walk(&mut |p, e| {
                if found.is_none() && !p.is_empty() && e.name == *name {
                    found = Some(p.to_vec());
                }
            });
            found.ok_or_else(|| format!("no element named `{name}`"))
        }
        _ => Err("element: a path [i, j, ...] or a name".into()),
    }
}

/// The `editor_ui_builder` tool.
pub fn mcp(
    st: &mut EditorState,
    engine: &mut Engine,
    args: &serde_json::Map<String, Json>,
) -> Result<Json, String> {
    if let Some(p) = args.get("open").and_then(Json::as_str) {
        st.ui_builder
            .open(engine.world.resource::<AssetServer>(), p)?;
        st.actions
            .push(crate::Action::OpenTab(crate::Tab::UiBuilder));
    }
    let w = &mut st.ui_builder;
    if w.path.is_none() {
        return Ok(summary(w));
    }
    if let Some(sel) = args.get("select") {
        w.selection = Some(Selection::Element(path_of(sel, w)?));
    }
    if let Some(list) = args.get("add").and_then(Json::as_array) {
        for a in list {
            let kind = a.get("type").and_then(Json::as_str).ok_or("add[].type")?;
            if !CONTROLS.iter().any(|(_, k)| *k == kind) {
                return Err(format!("unknown element type `{kind}`"));
            }
            if let Some(parent) = a.get("parent") {
                w.selection = Some(Selection::Element(path_of(parent, w)?));
            } else if a.get("root").and_then(Json::as_bool) == Some(true) {
                w.selection = None;
            }
            let p = w.add(kind);
            let e = w.tree.root.at_mut(&p).expect("added");
            if let Some(n) = a.get("name").and_then(Json::as_str) {
                e.name = n.to_owned();
            }
            if let Some(c) = a.get("classes").and_then(Json::as_str) {
                e.classes = c.split_whitespace().map(str::to_owned).collect();
            }
            if let Some(o) = a.get("attributes").and_then(Json::as_object) {
                for (k, v) in o {
                    e.set_attr(
                        k,
                        v.as_str()
                            .map_or_else(|| v.to_string(), str::to_owned)
                            .as_str(),
                    );
                }
            }
            if let Some(o) = a.get("style").and_then(Json::as_object) {
                for (k, v) in o {
                    e.set_style(k, v.as_str().unwrap_or(""));
                }
            }
        }
    }
    if let Some(s) = args.get("set").and_then(Json::as_object) {
        let p = path_of(s.get("element").ok_or("set.element")?, w)?;
        w.record();
        let e = w.tree.root.at_mut(&p).ok_or("no such element")?;
        if let Some(n) = s.get("name").and_then(Json::as_str) {
            e.name = n.to_owned();
        }
        if let Some(c) = s.get("classes").and_then(Json::as_str) {
            e.classes = c.split_whitespace().map(str::to_owned).collect();
        }
        if let Some(o) = s.get("attributes").and_then(Json::as_object) {
            for (k, v) in o {
                match v {
                    Json::Null => e.remove_attr(k),
                    v => e.set_attr(
                        k,
                        v.as_str()
                            .map_or_else(|| v.to_string(), str::to_owned)
                            .as_str(),
                    ),
                }
            }
        }
        if let Some(o) = s.get("style").and_then(Json::as_object) {
            for (k, v) in o {
                e.set_style(k, v.as_str().unwrap_or(""));
            }
        }
    }
    if let Some(p) = args.get("delete") {
        let p = path_of(p, w)?;
        w.delete(&p);
    }
    if let Some(r) = args.get("rule").and_then(Json::as_object) {
        let sel = r
            .get("selector")
            .and_then(Json::as_str)
            .ok_or("rule.selector")?
            .to_owned();
        let parsed: Result<Vec<_>, _> = sel
            .split(',')
            .map(|s| uss::parse_selector(s.trim()))
            .collect();
        let parsed = parsed?;
        if w.sheets.is_empty() {
            let path = w.path.clone().expect("open");
            let uss_path = path.trim_end_matches(".uxml").to_owned() + ".uss";
            w.tree.styles.push(uss_path.clone());
            w.sheets.push((uss_path, StyleSheet::default()));
        }
        w.record();
        let sheet = &mut w.sheets[0].1;
        let rule = match sheet.rules.iter().position(|x| x.selector_text == sel) {
            Some(i) => &mut sheet.rules[i],
            None => {
                sheet.rules.push(Rule {
                    selector_text: sel.clone(),
                    selectors: parsed,
                    declarations: Vec::new(),
                });
                sheet.rules.last_mut().expect("pushed")
            }
        };
        if let Some(o) = r.get("styles").and_then(Json::as_object) {
            for (k, v) in o {
                rule.declarations.retain(|(n, _)| n != k);
                if let Some(v) = v.as_str().filter(|v| !v.is_empty()) {
                    rule.declarations.push((k.clone(), v.to_owned()));
                }
            }
        }
    }
    if args.get("undo").and_then(Json::as_bool) == Some(true) {
        w.undo();
    }
    if args.get("save").and_then(Json::as_bool) == Some(true) {
        w.save(engine)?;
    }
    Ok(summary(&st.ui_builder))
}
