//! A UI Toolkit panel: the live element tree of one UI Document, styled by its style
//! sheets, laid out with flexbox, painted with egui and driven by pointer and keyboard
//! input (Unity's built-in controls).
//! Reference: <https://docs.unity3d.com/Manual/UIE-Controls.html>

use crate::style::*;
use crate::uss::{self, Matchable, StyleSheet};
use crate::uxml::{UxmlElement, VisualTree};
use egui::{Color32, FontId, Pos2, Rect, Stroke, Vec2, pos2, vec2};
use std::sync::Arc;

/// A control's value.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Value {
    #[default]
    None,
    Text(String),
    Float(f32),
    Int(i64),
    Bool(bool),
}

impl Value {
    pub fn as_f32(&self) -> Option<f32> {
        match self {
            Value::Float(v) => Some(*v),
            Value::Int(v) => Some(*v as f32),
            Value::Bool(b) => Some(*b as u32 as f32),
            Value::Text(t) => t.trim().parse().ok(),
            Value::None => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            Value::Float(v) => Some(*v != 0.0),
            Value::Int(v) => Some(*v != 0),
            _ => None,
        }
    }

    pub fn as_text(&self) -> String {
        match self {
            Value::Text(t) => t.clone(),
            Value::Float(v) => format!("{v}"),
            Value::Int(v) => v.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::None => String::new(),
        }
    }
}

/// One live element.
#[derive(Clone, Debug)]
pub struct Element {
    pub kind: String,
    pub name: String,
    pub classes: Vec<String>,
    pub attrs: Vec<(String, String)>,
    pub inline: Vec<(String, String)>,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    /// Child-index path in the UXML document (the editor selects by it).
    pub path: Vec<usize>,
    pub hover: bool,
    pub active: bool,
    pub focus: bool,
    pub enabled: bool,
    pub value: Value,
    /// Foldout open / Dropdown popup open.
    pub open: bool,
    /// ScrollView offset and content height.
    pub scroll: f32,
    pub content_height: f32,
    pub style: ComputedStyle,
    /// Border box in panel pixels.
    pub rect: Rect,
    /// What the element may draw in.
    pub clip: Rect,
    pub opacity: f32,
}

impl Element {
    pub fn attr(&self, k: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(a, _)| a == k)
            .map(|(_, v)| v.as_str())
    }

    /// The text a text element or a control's label shows.
    pub fn text(&self) -> String {
        match self.kind.as_str() {
            "Label" | "Button" => self.value.as_text(),
            _ => self.attr("text").unwrap_or("").to_owned(),
        }
    }

    pub fn label(&self) -> Option<&str> {
        self.attr("label").filter(|l| !l.is_empty())
    }

    fn checked(&self) -> bool {
        match self.kind.as_str() {
            "Toggle" | "RadioButton" => self.value.as_bool() == Some(true),
            "Foldout" => self.open,
            _ => false,
        }
    }

    pub fn displayed(&self) -> bool {
        self.style.display != Display::None
    }
}

impl Matchable for Element {
    fn kind(&self) -> &str {
        &self.kind
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn has_class(&self, c: &str) -> bool {
        self.classes.iter().any(|x| x == c)
    }

    fn has_pseudo(&self, p: &str) -> bool {
        match p {
            "hover" => self.hover,
            "active" => self.active,
            "focus" => self.focus,
            "checked" => self.checked(),
            "disabled" => !self.enabled,
            "enabled" => self.enabled,
            "root" => self.parent.is_none(),
            _ => false,
        }
    }
}

/// Unity's classes of the built-in elements.
fn default_classes(kind: &str) -> &'static [&'static str] {
    match kind {
        "Button" => &["unity-text-element", "unity-button"],
        "Label" => &["unity-text-element", "unity-label"],
        "Toggle" => &["unity-base-field", "unity-toggle"],
        "RadioButton" => &["unity-base-field", "unity-radio-button"],
        "Slider" | "SliderInt" => &["unity-base-field", "unity-slider"],
        "TextField" => &["unity-base-field", "unity-text-field"],
        "IntegerField" => &["unity-base-field", "unity-integer-field"],
        "FloatField" => &["unity-base-field", "unity-float-field"],
        "DropdownField" => &["unity-base-field", "unity-dropdown-field"],
        "Foldout" => &["unity-foldout"],
        "ScrollView" => &["unity-scroll-view"],
        "ProgressBar" => &["unity-progress-bar"],
        "GroupBox" | "RadioButtonGroup" => &["unity-group-box"],
        "Image" => &["unity-image"],
        _ => &[],
    }
}

/// Every element type UI Toolkit can build (Library of the UI Builder).
pub const CONTROLS: &[(&str, &str)] = &[
    ("Containers", "VisualElement"),
    ("Containers", "ScrollView"),
    ("Containers", "GroupBox"),
    ("Containers", "Foldout"),
    ("Controls", "Label"),
    ("Controls", "Button"),
    ("Controls", "Toggle"),
    ("Controls", "RadioButton"),
    ("Controls", "Slider"),
    ("Controls", "SliderInt"),
    ("Controls", "ProgressBar"),
    ("Controls", "DropdownField"),
    ("Controls", "Image"),
    ("Text Input", "TextField"),
    ("Text Input", "IntegerField"),
    ("Text Input", "FloatField"),
];

/// A control's starting value from its UXML attributes.
fn initial_value(e: &UxmlElement) -> Value {
    let a = |k: &str| e.attr(k);
    match e.kind.as_str() {
        "Label" | "Button" => Value::Text(a("text").unwrap_or("").to_owned()),
        "Toggle" | "RadioButton" => Value::Bool(a("value") == Some("true")),
        "Slider" | "ProgressBar" => Value::Float(
            a("value")
                .and_then(|v| v.parse().ok())
                .or_else(|| a("low-value").and_then(|v| v.parse().ok()))
                .unwrap_or(0.0),
        ),
        "SliderInt" | "IntegerField" => {
            Value::Int(a("value").and_then(|v| v.parse().ok()).unwrap_or(0))
        }
        "FloatField" => Value::Float(a("value").and_then(|v| v.parse().ok()).unwrap_or(0.0)),
        "TextField" => Value::Text(a("value").unwrap_or("").to_owned()),
        "DropdownField" => {
            let choices = choices(e.attr("choices"));
            let idx = a("index")
                .and_then(|v| v.parse::<i64>().ok())
                .or_else(|| {
                    a("value").and_then(|v| choices.iter().position(|c| c == v).map(|i| i as i64))
                })
                .unwrap_or(-1);
            Value::Int(idx)
        }
        _ => Value::None,
    }
}

pub fn choices(s: Option<&str>) -> Vec<String> {
    s.map(|c| {
        c.split(',')
            .map(|x| x.trim().to_owned())
            .filter(|x| !x.is_empty())
            .collect()
    })
    .unwrap_or_default()
}

/// What the user did to a control.
#[derive(Clone, Debug, PartialEq)]
pub enum UiEventKind {
    Click,
    Change,
    Submit,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UiEvent {
    pub kind: UiEventKind,
    /// The element's name (Unity's `Q<T>("name")`).
    pub element: String,
    pub index: usize,
    pub value: Value,
}

/// Input for one frame in panel pixels.
#[derive(Clone, Debug, Default)]
pub struct PanelInput {
    pub pointer: Option<Pos2>,
    pub pressed: bool,
    pub released: bool,
    pub down: bool,
    pub scroll: f32,
    /// Typed text and key presses, in the order they happened.
    pub keyboard: Vec<KeyInput>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum KeyInput {
    Text(String),
    Key(egui::Key),
}

/// A matching declaration: (specificity, order, property, value, base folder).
type Matched<'a> = ((u32, u32, u32), usize, &'a str, &'a str, &'a str);

/// A style sheet with the folder its URLs resolve against.
#[derive(Clone, Debug)]
pub struct SheetRef {
    pub sheet: Arc<StyleSheet>,
    pub base: String,
}

pub struct Panel {
    pub elements: Vec<Element>,
    /// Focused text field (keyboard goes there).
    pub focus: Option<usize>,
    pressed: Option<usize>,
    /// The open dropdown and its popup rows (panel pixels).
    dropdown: Option<(usize, Vec<Rect>)>,
    pub events: Vec<UiEvent>,
    /// The pointer is over a pickable element (game input should ignore it).
    pub wants_pointer: bool,
}

const TEXT_KINDS: &[&str] = &["Label", "Button"];
/// Width of a base field's label column (Unity's `unity-base-field__label` min-width).
const LABEL_WIDTH: f32 = 120.0;

impl Panel {
    /// The live tree of a document (its root is the UI Document's root element).
    pub fn new(tree: &VisualTree) -> Panel {
        let mut elements = Vec::new();
        fn add(
            e: &UxmlElement,
            parent: Option<usize>,
            path: Vec<usize>,
            out: &mut Vec<Element>,
        ) -> usize {
            let mut classes: Vec<String> = default_classes(&e.kind)
                .iter()
                .map(|c| (*c).to_owned())
                .collect();
            classes.extend(e.classes.iter().cloned());
            let enabled = e.attr("enabled") != Some("false");
            if !enabled {
                classes.push("unity-disabled".into());
            }
            let idx = out.len();
            out.push(Element {
                kind: e.kind.clone(),
                name: e.name.clone(),
                classes,
                attrs: e.attrs.clone(),
                inline: e.style.clone(),
                parent,
                children: Vec::new(),
                path: path.clone(),
                hover: false,
                active: false,
                focus: false,
                enabled,
                value: initial_value(e),
                open: e.kind != "Foldout" || e.attr("value") != Some("false"),
                scroll: 0.0,
                content_height: 0.0,
                style: ComputedStyle::default(),
                rect: Rect::NOTHING,
                clip: Rect::EVERYTHING,
                opacity: 1.0,
            });
            for (i, c) in e.children.iter().enumerate() {
                let mut p = path.clone();
                p.push(i);
                let ci = add(c, Some(idx), p, out);
                out[idx].children.push(ci);
            }
            idx
        }
        let mut root = tree.root.clone();
        root.kind = "VisualElement".into();
        root.classes = vec!["unity-ui-document__root".into()];
        add(&root, None, Vec::new(), &mut elements);
        Panel {
            elements,
            focus: None,
            pressed: None,
            dropdown: None,
            events: Vec::new(),
            wants_pointer: false,
        }
    }

    /// Elements matching a selector (`#name`, `.class`, `Button`, `#a > .b`; a bare word
    /// is a name, like Unity's `Q("name")`).
    pub fn query(&self, selector: &str) -> Vec<usize> {
        let s = selector.trim();
        let sel = if s
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
            && !s.contains([' ', '.', '#', ':', '>'])
            && !CONTROLS.iter().any(|(_, k)| *k == s)
        {
            format!("#{s}")
        } else {
            s.to_owned()
        };
        let Ok(sel) = uss::parse_selector(&sel) else {
            return Vec::new();
        };
        (0..self.elements.len())
            .filter(|i| uss::matches(&sel, &self.chain(*i)))
            .collect()
    }

    pub fn find(&self, selector: &str) -> Option<usize> {
        self.query(selector).first().copied()
    }

    fn chain(&self, i: usize) -> Vec<&dyn Matchable> {
        let mut ids = vec![i];
        let mut cur = self.elements[i].parent;
        while let Some(p) = cur {
            ids.push(p);
            cur = self.elements[p].parent;
        }
        ids.iter()
            .rev()
            .map(|k| &self.elements[*k] as &dyn Matchable)
            .collect()
    }

    /// Resolves every element's style from the sheets (theme first) and inline styles.
    pub fn style(&mut self, sheets: &[SheetRef]) {
        let order = self.order();
        for i in order {
            let chain = self.chain(i);
            let mut matched: Vec<Matched> = Vec::new();
            let mut n = 0;
            for sr in sheets {
                for rule in &sr.sheet.rules {
                    let best = rule
                        .selectors
                        .iter()
                        .filter(|s| uss::matches(s, &chain))
                        .map(|s| s.specificity())
                        .max();
                    if let Some(spec) = best {
                        for (k, v) in &rule.declarations {
                            matched.push((spec, n, k, v, &sr.base));
                            n += 1;
                        }
                    }
                }
            }
            matched.sort_by_key(|m| (m.0, m.1));
            let mut decls: Vec<(&str, &str, &str)> =
                matched.iter().map(|m| (m.2, m.3, m.4)).collect();
            let e = &self.elements[i];
            for (k, v) in &e.inline {
                decls.push((k, v, ""));
            }
            let parent = e
                .parent
                .map(|p| self.elements[p].style.clone())
                .unwrap_or_default();
            let style = ComputedStyle::compute(&parent, &decls);
            drop(chain);
            self.elements[i].style = style;
        }
    }

    /// Element indices parents-first (tree order).
    pub fn order(&self) -> Vec<usize> {
        let mut out = Vec::with_capacity(self.elements.len());
        let mut stack = vec![0];
        while let Some(i) = stack.pop() {
            out.push(i);
            stack.extend(self.elements[i].children.iter().rev());
        }
        out
    }

    /// Foldout header height for its font.
    fn header(e: &Element) -> f32 {
        (e.style.font_size * 1.5).max(18.0)
    }

    /// Lays the tree out in a `size` (panel pixels) panel.
    pub fn layout(&mut self, size: Vec2, ctx: &egui::Context) {
        use taffy::prelude::*;
        let mut tree: TaffyTree<usize> = TaffyTree::new();
        let lp = |l: Len| match l {
            Len::Auto => LengthPercentage::length(0.0),
            Len::Px(v) => LengthPercentage::length(v),
            Len::Percent(p) => LengthPercentage::percent(p / 100.0),
        };
        let lpa = |l: Len| match l {
            Len::Auto => LengthPercentageAuto::auto(),
            Len::Px(v) => LengthPercentageAuto::length(v),
            Len::Percent(p) => LengthPercentageAuto::percent(p / 100.0),
        };
        let dim = |l: Len| match l {
            Len::Auto => Dimension::auto(),
            Len::Px(v) => Dimension::length(v),
            Len::Percent(p) => Dimension::percent(p / 100.0),
        };
        let align = |a: Align| -> Option<AlignItems> {
            match a {
                Align::Auto => None,
                Align::FlexStart => Some(AlignItems::FlexStart),
                Align::Center => Some(AlignItems::Center),
                Align::FlexEnd => Some(AlignItems::FlexEnd),
                Align::Stretch => Some(AlignItems::Stretch),
            }
        };
        fn build(
            p: &Panel,
            i: usize,
            tree: &mut TaffyTree<usize>,
            f: &dyn Fn(&Element) -> taffy::Style,
        ) -> NodeId {
            let e = &p.elements[i];
            let style = f(e);
            let shown: Vec<usize> = if e.kind == "Foldout" && !e.open {
                Vec::new()
            } else {
                e.children.clone()
            };
            if shown.is_empty() {
                tree.new_leaf_with_context(style, i).expect("leaf")
            } else {
                let kids: Vec<NodeId> = shown.iter().map(|c| build(p, *c, tree, f)).collect();
                let n = tree.new_with_children(style, &kids).expect("node");
                tree.set_node_context(n, Some(i)).expect("context");
                n
            }
        }
        let to_style = |e: &Element| -> taffy::Style {
            let s = &e.style;
            let mut padding = Rect {
                left: lp(s.padding[0]),
                top: lp(s.padding[1]),
                right: lp(s.padding[2]),
                bottom: lp(s.padding[3]),
            };
            if e.kind == "Foldout" {
                padding.top =
                    LengthPercentage::length(s.padding[1].px_or(0.0, 0.0) + Panel::header(e));
                padding.left = LengthPercentage::length(s.padding[0].px_or(0.0, 0.0) + 14.0);
            }
            taffy::Style {
                display: if s.display == crate::style::Display::None {
                    taffy::Display::None
                } else {
                    taffy::Display::Flex
                },
                position: if s.position == crate::style::Position::Absolute {
                    taffy::Position::Absolute
                } else {
                    taffy::Position::Relative
                },
                inset: Rect {
                    left: lpa(s.left),
                    top: lpa(s.top),
                    right: lpa(s.right),
                    bottom: lpa(s.bottom),
                },
                size: Size {
                    width: dim(s.width),
                    height: dim(s.height),
                },
                min_size: Size {
                    width: dim(s.min_width),
                    height: dim(s.min_height),
                },
                max_size: Size {
                    width: dim(s.max_width),
                    height: dim(s.max_height),
                },
                margin: Rect {
                    left: lpa(s.margin[0]),
                    top: lpa(s.margin[1]),
                    right: lpa(s.margin[2]),
                    bottom: lpa(s.margin[3]),
                },
                padding,
                border: Rect {
                    left: LengthPercentage::length(s.border_width[0]),
                    top: LengthPercentage::length(s.border_width[1]),
                    right: LengthPercentage::length(s.border_width[2]),
                    bottom: LengthPercentage::length(s.border_width[3]),
                },
                flex_direction: match s.flex_direction {
                    crate::style::FlexDirection::Row => taffy::FlexDirection::Row,
                    crate::style::FlexDirection::RowReverse => taffy::FlexDirection::RowReverse,
                    crate::style::FlexDirection::ColumnReverse => {
                        taffy::FlexDirection::ColumnReverse
                    }
                    crate::style::FlexDirection::Column => taffy::FlexDirection::Column,
                },
                flex_wrap: match s.flex_wrap {
                    crate::style::Wrap::Wrap => FlexWrap::Wrap,
                    crate::style::Wrap::WrapReverse => FlexWrap::WrapReverse,
                    crate::style::Wrap::NoWrap => FlexWrap::NoWrap,
                },
                flex_grow: s.flex_grow,
                flex_shrink: s.flex_shrink,
                flex_basis: dim(s.flex_basis),
                align_items: align(s.align_items),
                align_self: align(s.align_self),
                align_content: match s.align_content {
                    Align::Center => Some(AlignContent::Center),
                    Align::FlexEnd => Some(AlignContent::FlexEnd),
                    Align::Stretch => Some(AlignContent::Stretch),
                    _ => Some(AlignContent::FlexStart),
                },
                justify_content: Some(match s.justify_content {
                    Justify::FlexStart => JustifyContent::FlexStart,
                    Justify::Center => JustifyContent::Center,
                    Justify::FlexEnd => JustifyContent::FlexEnd,
                    Justify::SpaceBetween => JustifyContent::SpaceBetween,
                    Justify::SpaceAround => JustifyContent::SpaceAround,
                    Justify::SpaceEvenly => JustifyContent::SpaceEvenly,
                }),
                overflow: if e.kind == "ScrollView" {
                    taffy::Point {
                        x: taffy::Overflow::Hidden,
                        y: taffy::Overflow::Scroll,
                    }
                } else if s.overflow == crate::style::Overflow::Hidden {
                    taffy::Point {
                        x: taffy::Overflow::Hidden,
                        y: taffy::Overflow::Hidden,
                    }
                } else {
                    taffy::Point {
                        x: taffy::Overflow::Visible,
                        y: taffy::Overflow::Visible,
                    }
                },
                scrollbar_width: 0.0,
                ..Default::default()
            }
        };
        let root = build(self, 0, &mut tree, &to_style);
        // The document root fills the panel.
        let mut rs = tree.style(root).expect("root").clone();
        rs.size = Size {
            width: Dimension::length(size.x),
            height: Dimension::length(size.y),
        };
        tree.set_style(root, rs).expect("root style");
        let elements = &self.elements;
        let _ = tree.compute_layout_with_measure(
            root,
            Size {
                width: AvailableSpace::Definite(size.x),
                height: AvailableSpace::Definite(size.y),
            },
            |known, avail, _, ctx_i, _| {
                let Some(i) = ctx_i.copied() else {
                    return Size::ZERO;
                };
                let e = &elements[i];
                let wrap = match (known.width, avail.width) {
                    (Some(w), _) => w,
                    (None, AvailableSpace::Definite(w))
                        if e.style.white_space == WhiteSpace::Normal =>
                    {
                        w
                    }
                    _ => f32::INFINITY,
                };
                let m = measure(ctx, e, wrap);
                Size {
                    width: known.width.unwrap_or(m.x),
                    height: known.height.unwrap_or(m.y),
                }
            },
        );
        // Absolute rects (and ScrollView offsets, translate).
        fn place(
            p: &mut Panel,
            tree: &TaffyTree<usize>,
            node: NodeId,
            origin: Vec2,
            clip: egui::Rect,
            opacity: f32,
        ) {
            let i = *tree.get_node_context(node).expect("context");
            let l = tree.layout(node).expect("layout");
            let e = &p.elements[i];
            let t = vec2(
                e.style.translate[0].px_or(l.size.width, 0.0),
                e.style.translate[1].px_or(l.size.height, 0.0),
            );
            let min = pos2(origin.x + l.location.x, origin.y + l.location.y) + t;
            let rect = egui::Rect::from_min_size(min, vec2(l.size.width, l.size.height));
            let opacity = opacity * e.style.opacity;
            let hidden_overflow =
                e.style.overflow == crate::style::Overflow::Hidden || e.kind == "ScrollView";
            let child_clip = if hidden_overflow {
                clip.intersect(rect)
            } else {
                clip
            };
            let scroll = if e.kind == "ScrollView" {
                e.scroll
            } else {
                0.0
            };
            let kids: Vec<NodeId> = tree.children(node).unwrap_or_default();
            let mut bottom: f32 = 0.0;
            for k in &kids {
                if let Ok(kl) = tree.layout(*k) {
                    bottom = bottom.max(kl.location.y + kl.size.height);
                }
            }
            let el = &mut p.elements[i];
            el.rect = rect;
            el.clip = clip;
            el.opacity = opacity;
            el.content_height = bottom;
            for k in kids {
                place(
                    p,
                    tree,
                    k,
                    min.to_vec2() - vec2(0.0, scroll),
                    child_clip,
                    opacity,
                );
            }
        }
        // Elements not laid out (closed foldouts) have no rect.
        for e in &mut self.elements {
            e.rect = egui::Rect::NOTHING;
        }
        place(self, &tree, root, Vec2::ZERO, egui::Rect::EVERYTHING, 1.0);
        // Keep scroll offsets in range.
        for e in &mut self.elements {
            if e.kind == "ScrollView" {
                let max = (e.content_height - e.rect.height()).max(0.0);
                e.scroll = e.scroll.clamp(0.0, max);
            }
        }
    }

    fn pickable(&self, i: usize, p: Pos2) -> bool {
        let e = &self.elements[i];
        e.rect.contains(p)
            && e.clip.contains(p)
            && e.style.visibility != Visibility::Hidden
            && e.attr("picking-mode") != Some("Ignore")
            && e.displayed()
    }

    /// The deepest element under `p`.
    pub fn hit(&self, p: Pos2) -> Option<usize> {
        let mut best = None;
        for i in self.order() {
            if self.pickable(i, p) {
                best = Some(i);
            }
        }
        best.filter(|i| *i != 0)
    }

    fn ancestor_or_self(&self, i: usize, f: impl Fn(&Element) -> bool) -> Option<usize> {
        let mut cur = Some(i);
        while let Some(c) = cur {
            if f(&self.elements[c]) {
                return Some(c);
            }
            cur = self.elements[c].parent;
        }
        None
    }

    fn event(&mut self, kind: UiEventKind, i: usize) {
        let e = &self.elements[i];
        self.events.push(UiEvent {
            kind,
            element: e.name.clone(),
            index: i,
            value: e.value.clone(),
        });
    }

    /// Track rect of a slider-like control (after its label).
    fn track(e: &Element) -> Rect {
        let r = e.rect;
        let left = if e.label().is_some() {
            LABEL_WIDTH.min(r.width() * 0.5)
        } else {
            0.0
        };
        Rect::from_min_max(
            pos2(r.left() + left + 6.0, r.top()),
            pos2(r.right() - 6.0, r.bottom()),
        )
    }

    fn set_slider(&mut self, i: usize, x: f32) {
        let e = &self.elements[i];
        let lo: f32 = e
            .attr("low-value")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.0);
        let hi: f32 = e
            .attr("high-value")
            .and_then(|v| v.parse().ok())
            .unwrap_or(10.0);
        let t = Panel::track(e);
        let f = ((x - t.left()) / t.width().max(1.0)).clamp(0.0, 1.0);
        let v = lo + (hi - lo) * f;
        let new = if e.kind == "SliderInt" {
            Value::Int(v.round() as i64)
        } else {
            Value::Float(v)
        };
        if new != e.value {
            self.elements[i].value = new;
            self.event(UiEventKind::Change, i);
        }
    }

    /// Hover, press, click and typing for one frame. Returns whether styles may change.
    pub fn input(&mut self, input: &PanelInput) {
        self.events.clear();
        let hit = input.pointer.and_then(|p| {
            // An open dropdown's popup is on top.
            if let Some((_, rows)) = &self.dropdown
                && rows.iter().any(|r| r.contains(p))
            {
                return None;
            }
            self.hit(p)
        });
        self.wants_pointer = hit.is_some()
            || self.dropdown.as_ref().is_some_and(|(_, rows)| {
                input
                    .pointer
                    .is_some_and(|p| rows.iter().any(|r| r.contains(p)))
            });
        // :hover on the element under the pointer and its ancestors.
        for e in &mut self.elements {
            e.hover = false;
        }
        let mut cur = hit;
        while let Some(c) = cur {
            self.elements[c].hover = true;
            cur = self.elements[c].parent;
        }
        let clickable = |e: &Element| {
            e.enabled
                && matches!(
                    e.kind.as_str(),
                    "Button"
                        | "Toggle"
                        | "RadioButton"
                        | "Foldout"
                        | "DropdownField"
                        | "Slider"
                        | "SliderInt"
                        | "TextField"
                        | "IntegerField"
                        | "FloatField"
                )
        };
        if input.pressed {
            // Choosing from an open dropdown.
            if let Some((d, rows)) = self.dropdown.take() {
                if let Some(p) = input.pointer
                    && let Some(k) = rows.iter().position(|r| r.contains(p))
                {
                    self.elements[d].value = Value::Int(k as i64);
                    self.elements[d].open = false;
                    self.event(UiEventKind::Change, d);
                    return;
                }
                self.elements[d].open = false;
            }
            let target = hit.and_then(|h| self.ancestor_or_self(h, clickable));
            // Foldouts toggle from their header only.
            let target = target.filter(|t| {
                let e = &self.elements[*t];
                e.kind != "Foldout"
                    || input
                        .pointer
                        .is_some_and(|p| p.y < e.rect.top() + Panel::header(e))
            });
            self.pressed = target;
            for e in &mut self.elements {
                e.active = false;
                e.focus = false;
            }
            self.focus = None;
            if let Some(t) = target {
                let mut cur = Some(t);
                while let Some(c) = cur {
                    self.elements[c].active = true;
                    cur = self.elements[c].parent;
                }
                self.elements[t].focus = true;
                self.focus = Some(t);
                if matches!(self.elements[t].kind.as_str(), "Slider" | "SliderInt")
                    && let Some(p) = input.pointer
                {
                    self.set_slider(t, p.x);
                }
            }
        }
        if input.down
            && let Some(t) = self.pressed
            && matches!(self.elements[t].kind.as_str(), "Slider" | "SliderInt")
            && let Some(p) = input.pointer
        {
            self.set_slider(t, p.x);
        }
        if input.released
            && let Some(t) = self.pressed.take()
        {
            for e in &mut self.elements {
                e.active = false;
            }
            let over = input
                .pointer
                .is_some_and(|p| self.elements[t].rect.contains(p));
            if over {
                match self.elements[t].kind.as_str() {
                    "Button" => self.event(UiEventKind::Click, t),
                    "Toggle" => {
                        let v = self.elements[t].value.as_bool() != Some(true);
                        self.elements[t].value = Value::Bool(v);
                        self.event(UiEventKind::Change, t);
                    }
                    "RadioButton" => {
                        if self.elements[t].value.as_bool() != Some(true) {
                            // Siblings in the same group switch off.
                            if let Some(p) = self.elements[t].parent {
                                for s in self.elements[p].children.clone() {
                                    if self.elements[s].kind == "RadioButton" && s != t {
                                        self.elements[s].value = Value::Bool(false);
                                    }
                                }
                            }
                            self.elements[t].value = Value::Bool(true);
                            self.event(UiEventKind::Change, t);
                        }
                    }
                    "Foldout" => {
                        self.elements[t].open = !self.elements[t].open;
                        self.elements[t].value = Value::Bool(self.elements[t].open);
                        self.event(UiEventKind::Change, t);
                    }
                    "DropdownField" => {
                        let e = &self.elements[t];
                        let n = choices(e.attr("choices")).len();
                        let r = e.rect;
                        let h = r.height().max(18.0);
                        let rows = (0..n)
                            .map(|k| {
                                Rect::from_min_size(
                                    pos2(r.left(), r.bottom() + k as f32 * h),
                                    vec2(r.width(), h),
                                )
                            })
                            .collect();
                        self.elements[t].open = true;
                        self.dropdown = Some((t, rows));
                    }
                    _ => {}
                }
            }
        }
        // Scrolling the ScrollView under the pointer.
        if input.scroll != 0.0
            && let Some(h) = hit
            && let Some(s) = self.ancestor_or_self(h, |e| e.kind == "ScrollView")
        {
            let e = &mut self.elements[s];
            let max = (e.content_height - e.rect.height()).max(0.0);
            e.scroll = (e.scroll - input.scroll).clamp(0.0, max);
        }
        // Typing into the focused field.
        if let Some(f) = self.focus
            && matches!(
                self.elements[f].kind.as_str(),
                "TextField" | "IntegerField" | "FloatField"
            )
        {
            let mut text = self.elements[f].value.as_text();
            let before = text.clone();
            let numeric = self.elements[f].kind != "TextField";
            let max_len: usize = self.elements[f]
                .attr("max-length")
                .and_then(|v| v.parse::<i64>().ok())
                .filter(|v| *v > 0)
                .map_or(usize::MAX, |v| v as usize);
            let mut submit = false;
            for k in &input.keyboard {
                match k {
                    KeyInput::Text(t) => {
                        for ch in t.chars() {
                            if numeric && !(ch.is_ascii_digit() || ch == '-' || ch == '.') {
                                continue;
                            }
                            if text.chars().count() < max_len {
                                text.push(ch);
                            }
                        }
                    }
                    KeyInput::Key(egui::Key::Backspace) => {
                        text.pop();
                    }
                    KeyInput::Key(egui::Key::Enter) => submit = true,
                    KeyInput::Key(egui::Key::Escape) => {
                        self.elements[f].focus = false;
                        self.focus = None;
                    }
                    _ => {}
                }
            }
            if text != before {
                self.elements[f].value = match self.elements[f].kind.as_str() {
                    "IntegerField" => Value::Int(text.parse().unwrap_or(0)),
                    "FloatField" if text.ends_with('.') || text == "-" || text.is_empty() => {
                        Value::Text(text)
                    }
                    "FloatField" => Value::Float(text.parse().unwrap_or(0.0)),
                    _ => Value::Text(text),
                };
                self.event(UiEventKind::Change, f);
            }
            if submit {
                self.event(UiEventKind::Submit, f);
            }
        }
    }

    /// Paints the panel: `origin` is the panel's top-left on screen, `scale` screen
    /// points per panel pixel. `texture` maps image paths to egui textures.
    pub fn paint(
        &mut self,
        painter: &egui::Painter,
        origin: Pos2,
        scale: f32,
        texture: &mut dyn FnMut(&str) -> Option<egui::TextureId>,
    ) {
        let to = |r: Rect| {
            Rect::from_min_max(
                origin + r.min.to_vec2() * scale,
                origin + r.max.to_vec2() * scale,
            )
        };
        for i in self.order() {
            let e = &self.elements[i];
            if !e.displayed() || e.style.visibility == Visibility::Hidden || !e.rect.is_finite() {
                continue;
            }
            let clip = if e.clip == Rect::EVERYTHING {
                painter.clip_rect()
            } else {
                to(e.clip).intersect(painter.clip_rect())
            };
            let p = painter.with_clip_rect(clip);
            let r = to(e.rect);
            let s = &e.style;
            let op = e.opacity;
            let radius = egui::CornerRadius {
                nw: (s.border_radius[0] * scale).min(255.0) as u8,
                ne: (s.border_radius[1] * scale).min(255.0) as u8,
                se: (s.border_radius[2] * scale).min(255.0) as u8,
                sw: (s.border_radius[3] * scale).min(255.0) as u8,
            };
            if s.background_color.0[3] > 0.0 {
                p.rect_filled(r, radius, s.background_color.to_egui(op));
            }
            if let Some(img) = &s.background_image
                && let Some(t) = texture(img)
            {
                p.image(
                    t,
                    r,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    s.background_tint.to_egui(op),
                );
            }
            let bw = s.border_width;
            if bw.iter().any(|w| *w > 0.0) {
                let uniform = bw.iter().all(|w| *w == bw[0])
                    && s.border_color.iter().all(|c| *c == s.border_color[0]);
                if uniform {
                    p.rect_stroke(
                        r.shrink(bw[0] * scale * 0.5),
                        radius,
                        Stroke::new(bw[0] * scale, s.border_color[0].to_egui(op)),
                        egui::StrokeKind::Middle,
                    );
                } else {
                    let sides = [
                        [r.left_top(), r.left_bottom()],
                        [r.left_top(), r.right_top()],
                        [r.right_top(), r.right_bottom()],
                        [r.left_bottom(), r.right_bottom()],
                    ];
                    for k in 0..4 {
                        if bw[k] > 0.0 {
                            p.line_segment(
                                sides[k],
                                Stroke::new(bw[k] * scale, s.border_color[k].to_egui(op)),
                            );
                        }
                    }
                }
            }
            paint_control(&p, e, r, scale, op);
        }
        // An open dropdown's choices over everything.
        if let Some((d, rows)) = &self.dropdown {
            let e = &self.elements[*d];
            let items = choices(e.attr("choices"));
            for (k, row) in rows.iter().enumerate() {
                let rr = to(*row);
                let hover = painter
                    .ctx()
                    .pointer_hover_pos()
                    .is_some_and(|p| rr.contains(p));
                painter.rect_filled(
                    rr,
                    0.0,
                    if hover {
                        Color32::from_rgb(58, 121, 187)
                    } else {
                        Color32::from_rgb(42, 42, 42)
                    },
                );
                if let Some(t) = items.get(k) {
                    painter.text(
                        rr.left_center() + vec2(6.0 * scale, 0.0),
                        egui::Align2::LEFT_CENTER,
                        t,
                        FontId::proportional(e.style.font_size * scale),
                        Color32::from_gray(210),
                    );
                }
            }
        }
    }
}

fn font(e: &Element, scale: f32) -> FontId {
    FontId::proportional((e.style.font_size * scale).max(1.0))
}

fn text_color(e: &Element, op: f32) -> Color32 {
    e.style.color.to_egui(op)
}

/// Lays out text with the element's font, style and wrapping.
fn galley(
    ctx: &egui::Context,
    e: &Element,
    text: &str,
    scale: f32,
    wrap: f32,
    op: f32,
) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::default();
    let italic = matches!(
        e.style.font_style,
        FontStyle::Italic | FontStyle::BoldAndItalic
    );
    job.append(
        text,
        0.0,
        egui::TextFormat {
            font_id: font(e, scale),
            color: text_color(e, op),
            italics: italic,
            extra_letter_spacing: e.style.letter_spacing * scale,
            ..Default::default()
        },
    );
    job.wrap.max_width = if e.style.white_space == WhiteSpace::NoWrap {
        f32::INFINITY
    } else {
        wrap
    };
    job.halign = egui::Align::LEFT;
    ctx.fonts_mut(|f| f.layout_job(job))
}

/// Intrinsic content size of a leaf (panel pixels).
fn measure(ctx: &egui::Context, e: &Element, wrap: f32) -> Vec2 {
    let pad_x = e.style.padding[0].px_or(0.0, 0.0)
        + e.style.padding[2].px_or(0.0, 0.0)
        + e.style.border_width[0]
        + e.style.border_width[2];
    let pad_y = e.style.padding[1].px_or(0.0, 0.0)
        + e.style.padding[3].px_or(0.0, 0.0)
        + e.style.border_width[1]
        + e.style.border_width[3];
    let line = e.style.font_size * 1.3;
    let text_size = |t: &str, w: f32| {
        if t.is_empty() {
            return vec2(0.0, line);
        }
        let g = galley(ctx, e, t, 1.0, (w - pad_x).max(1.0), 1.0);
        g.size()
    };
    let label = e
        .label()
        .map_or(0.0, |l| text_size(l, f32::INFINITY).x.max(LABEL_WIDTH));
    let inner = match e.kind.as_str() {
        k if TEXT_KINDS.contains(&k) => text_size(&e.text(), wrap),
        "Toggle" | "RadioButton" => {
            let t = text_size(&e.text(), f32::INFINITY);
            vec2(
                label + 16.0 + if t.x > 0.0 { 4.0 + t.x } else { 0.0 },
                line.max(16.0),
            )
        }
        "Slider" | "SliderInt" => vec2(label + 100.0, line.max(18.0)),
        "ProgressBar" => vec2(100.0, line.max(18.0)),
        "TextField" | "IntegerField" | "FloatField" => {
            let t = text_size(&e.value.as_text(), f32::INFINITY);
            vec2(label + (t.x + 8.0).max(60.0), line + 4.0)
        }
        "DropdownField" => {
            let cs = choices(e.attr("choices"));
            let w = cs
                .iter()
                .map(|c| text_size(c, f32::INFINITY).x)
                .fold(0.0, f32::max);
            vec2(label + (w + 24.0).max(80.0), line + 4.0)
        }
        "Image" => vec2(64.0, 64.0),
        "Foldout" => text_size(&e.text(), f32::INFINITY) + vec2(14.0, 0.0),
        _ => Vec2::ZERO,
    };
    inner + vec2(pad_x, pad_y)
}

/// The parts of a control drawn inside its box.
fn paint_control(p: &egui::Painter, e: &Element, r: Rect, scale: f32, op: f32) {
    let ctx = p.ctx();
    let s = &e.style;
    let content = Rect::from_min_max(
        r.min
            + vec2(
                (s.padding[0].px_or(0.0, 0.0) + s.border_width[0]) * scale,
                (s.padding[1].px_or(0.0, 0.0) + s.border_width[1]) * scale,
            ),
        r.max
            - vec2(
                (s.padding[2].px_or(0.0, 0.0) + s.border_width[2]) * scale,
                (s.padding[3].px_or(0.0, 0.0) + s.border_width[3]) * scale,
            ),
    );
    let accent = Color32::from_rgb(58, 121, 187);
    let field_bg = Color32::from_rgb(42, 42, 42).gamma_multiply(op);
    let field_border = Color32::from_rgb(33, 33, 33).gamma_multiply(op);
    let text_at = |text: &str, rect: Rect, align: TextAlign| {
        if text.is_empty() {
            return;
        }
        let g = galley(ctx, e, text, scale, rect.width().max(1.0), op);
        let pos = pos2(
            rect.left() + (rect.width() - g.size().x) * align.0,
            rect.top() + (rect.height() - g.size().y) * align.1,
        );
        let bold = matches!(s.font_style, FontStyle::Bold | FontStyle::BoldAndItalic);
        if bold {
            p.galley(pos + vec2(0.6 * scale, 0.0), g.clone(), text_color(e, op));
        }
        p.galley(pos, g, text_color(e, op));
    };
    // A base field's label column.
    let (label_rect, field) = match e.label() {
        Some(_) => {
            let w = (LABEL_WIDTH * scale).min(content.width() * 0.5);
            (
                Some(Rect::from_min_max(
                    content.min,
                    pos2(content.left() + w, content.bottom()),
                )),
                Rect::from_min_max(pos2(content.left() + w, content.top()), content.max),
            )
        }
        None => (None, content),
    };
    if let (Some(lr), Some(l)) = (label_rect, e.label()) {
        text_at(l, lr, TextAlign(0.0, 0.5));
    }
    match e.kind.as_str() {
        "Label" | "Button" => text_at(&e.text(), content, s.text_align),
        "Toggle" | "RadioButton" => {
            let b = 14.0 * scale;
            let boxr =
                Rect::from_min_size(pos2(field.left(), field.center().y - b * 0.5), vec2(b, b));
            let on = e.value.as_bool() == Some(true);
            if e.kind == "RadioButton" {
                p.circle(
                    boxr.center(),
                    b * 0.5,
                    field_bg,
                    Stroke::new(1.0, field_border),
                );
                if on {
                    p.circle_filled(
                        boxr.center(),
                        b * 0.25,
                        Color32::from_gray(220).gamma_multiply(op),
                    );
                }
            } else {
                p.rect(
                    boxr,
                    2.0 * scale,
                    if on {
                        accent.gamma_multiply(op)
                    } else {
                        field_bg
                    },
                    Stroke::new(1.0, field_border),
                    egui::StrokeKind::Inside,
                );
                if on {
                    let c = boxr.center();
                    let k = b * 0.28;
                    p.line_segment(
                        [c + vec2(-k, 0.0), c + vec2(-k * 0.3, k * 0.7)],
                        Stroke::new(2.0 * scale, Color32::WHITE.gamma_multiply(op)),
                    );
                    p.line_segment(
                        [c + vec2(-k * 0.3, k * 0.7), c + vec2(k, -k * 0.7)],
                        Stroke::new(2.0 * scale, Color32::WHITE.gamma_multiply(op)),
                    );
                }
            }
            let tr = Rect::from_min_max(pos2(boxr.right() + 4.0 * scale, field.top()), field.max);
            text_at(&e.text(), tr, TextAlign(0.0, 0.5));
        }
        "Slider" | "SliderInt" | "ProgressBar" => {
            let lo: f32 = e
                .attr("low-value")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.0);
            let hi: f32 = e
                .attr("high-value")
                .and_then(|v| v.parse().ok())
                .unwrap_or(if e.kind == "ProgressBar" { 100.0 } else { 10.0 });
            let f = ((e.value.as_f32().unwrap_or(lo) - lo) / (hi - lo).max(1e-6)).clamp(0.0, 1.0);
            if e.kind == "ProgressBar" {
                p.rect_filled(field, 2.0 * scale, field_bg);
                let mut fill = field.shrink(1.0 * scale);
                fill.max.x = fill.left() + fill.width() * f;
                p.rect_filled(fill, 2.0 * scale, accent.gamma_multiply(op));
                text_at(e.attr("title").unwrap_or(""), field, TextAlign(0.5, 0.5));
            } else {
                let track = Rect::from_min_max(
                    pos2(field.left() + 6.0 * scale, field.center().y - 1.5 * scale),
                    pos2(field.right() - 6.0 * scale, field.center().y + 1.5 * scale),
                );
                p.rect_filled(track, 1.0, Color32::from_rgb(94, 94, 94).gamma_multiply(op));
                let x = track.left() + track.width() * f;
                p.rect(
                    Rect::from_center_size(pos2(x, field.center().y), vec2(10.0, 14.0) * scale),
                    2.0 * scale,
                    Color32::from_rgb(153, 153, 153).gamma_multiply(op),
                    Stroke::new(1.0, field_border),
                    egui::StrokeKind::Inside,
                );
            }
        }
        "TextField" | "IntegerField" | "FloatField" => {
            p.rect(
                field,
                2.0 * scale,
                field_bg,
                Stroke::new(1.0, if e.focus { accent } else { field_border }),
                egui::StrokeKind::Inside,
            );
            let mut t = e.value.as_text();
            if e.attr("password") == Some("true") {
                t = "•".repeat(t.chars().count());
            }
            let inner = field.shrink2(vec2(4.0 * scale, 0.0));
            text_at(&t, inner, TextAlign(0.0, 0.5));
            if e.focus {
                let g = galley(ctx, e, &t, scale, f32::INFINITY, op);
                let x = inner.left() + g.size().x + 1.0;
                let blink = (ctx.input(|i| i.time) * 2.0) as i64 % 2 == 0;
                if blink {
                    p.line_segment(
                        [
                            pos2(x, inner.top() + 3.0 * scale),
                            pos2(x, inner.bottom() - 3.0 * scale),
                        ],
                        Stroke::new(1.0, text_color(e, op)),
                    );
                }
                ctx.request_repaint();
            }
        }
        "DropdownField" => {
            p.rect(
                field,
                2.0 * scale,
                Color32::from_rgb(81, 81, 81).gamma_multiply(op),
                Stroke::new(1.0, field_border),
                egui::StrokeKind::Inside,
            );
            let cs = choices(e.attr("choices"));
            let cur = match &e.value {
                Value::Int(k) if *k >= 0 => cs.get(*k as usize).cloned().unwrap_or_default(),
                _ => String::new(),
            };
            text_at(
                &cur,
                field.shrink2(vec2(6.0 * scale, 0.0)),
                TextAlign(0.0, 0.5),
            );
            let c = pos2(field.right() - 9.0 * scale, field.center().y);
            let k = 3.5 * scale;
            p.add(egui::Shape::convex_polygon(
                vec![
                    c + vec2(-k, -k * 0.5),
                    c + vec2(k, -k * 0.5),
                    c + vec2(0.0, k * 0.6),
                ],
                text_color(e, op),
                Stroke::NONE,
            ));
        }
        "Foldout" => {
            let h = Panel::header(e) * scale;
            let hr = Rect::from_min_size(r.min, vec2(r.width(), h));
            let c = pos2(hr.left() + 7.0 * scale, hr.center().y);
            let k = 4.0 * scale;
            let tri = if e.open {
                vec![
                    c + vec2(-k, -k * 0.5),
                    c + vec2(k, -k * 0.5),
                    c + vec2(0.0, k * 0.7),
                ]
            } else {
                vec![
                    c + vec2(-k * 0.5, -k),
                    c + vec2(k * 0.7, 0.0),
                    c + vec2(-k * 0.5, k),
                ]
            };
            p.add(egui::Shape::convex_polygon(
                tri,
                text_color(e, op),
                Stroke::NONE,
            ));
            text_at(
                &e.text(),
                Rect::from_min_max(pos2(hr.left() + 16.0 * scale, hr.top()), hr.max),
                TextAlign(0.0, 0.5),
            );
        }
        "ScrollView" if e.content_height > e.rect.height() + 0.5 => {
            let frac = e.rect.height() / e.content_height;
            let bar_h = (r.height() * frac).max(16.0 * scale);
            let max = (e.content_height - e.rect.height()).max(1.0);
            let y = r.top() + (r.height() - bar_h) * (e.scroll / max);
            p.rect_filled(
                Rect::from_min_size(pos2(r.right() - 6.0 * scale, y), vec2(4.0 * scale, bar_h)),
                2.0 * scale,
                Color32::from_white_alpha(80),
            );
        }
        _ => {}
    }
}
