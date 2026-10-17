//! UXML: UI Toolkit's document format (Unity `VisualTreeAsset`).
//!
//! A UXML file is XML whose elements are visual elements (`ui:Button`, `ui:Label`, ...),
//! with `name`, `class` (space-separated), `style` (inline USS) and control attributes
//! (`text`, `value`, `low-value`, ...). `<Style src="..."/>` attaches stylesheets.
//! Reference: <https://docs.unity3d.com/Manual/UIE-UXML.html>

use std::fmt::Write;

/// One element of a UXML document.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct UxmlElement {
    /// Element type without namespace (`Button`, `VisualElement`).
    pub kind: String,
    pub name: String,
    pub classes: Vec<String>,
    /// Inline style (`style="width: 100px; color: red"`), as declarations.
    pub style: Vec<(String, String)>,
    /// Every other attribute (`text`, `value`, `label`, ...).
    pub attrs: Vec<(String, String)>,
    pub children: Vec<UxmlElement>,
}

impl UxmlElement {
    pub fn new(kind: &str) -> Self {
        Self {
            kind: kind.into(),
            ..Default::default()
        }
    }

    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn set_attr(&mut self, name: &str, value: &str) {
        match self.attrs.iter_mut().find(|(k, _)| k == name) {
            Some((_, v)) => *v = value.into(),
            None => self.attrs.push((name.into(), value.into())),
        }
    }

    pub fn remove_attr(&mut self, name: &str) {
        self.attrs.retain(|(k, _)| k != name);
    }

    pub fn style_value(&self, prop: &str) -> Option<&str> {
        self.style
            .iter()
            .find(|(k, _)| k == prop)
            .map(|(_, v)| v.as_str())
    }

    pub fn set_style(&mut self, prop: &str, value: &str) {
        if value.trim().is_empty() {
            self.style.retain(|(k, _)| k != prop);
            return;
        }
        match self.style.iter_mut().find(|(k, _)| k == prop) {
            Some((_, v)) => *v = value.into(),
            None => self.style.push((prop.into(), value.into())),
        }
    }

    /// The element at a child-index path.
    pub fn at(&self, path: &[usize]) -> Option<&UxmlElement> {
        let mut e = self;
        for &i in path {
            e = e.children.get(i)?;
        }
        Some(e)
    }

    pub fn at_mut(&mut self, path: &[usize]) -> Option<&mut UxmlElement> {
        let mut e = self;
        for &i in path {
            e = e.children.get_mut(i)?;
        }
        Some(e)
    }

    /// Visits every element with its index path.
    pub fn walk(&self, f: &mut dyn FnMut(&[usize], &UxmlElement)) {
        fn go(e: &UxmlElement, path: &mut Vec<usize>, f: &mut dyn FnMut(&[usize], &UxmlElement)) {
            f(path, e);
            for (i, c) in e.children.iter().enumerate() {
                path.push(i);
                go(c, path, f);
                path.pop();
            }
        }
        go(self, &mut Vec::new(), f);
    }
}

/// A parsed `.uxml` file: the root's children and attached stylesheets.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct VisualTree {
    /// The document root (`UXML`); its children are the top-level elements.
    pub root: UxmlElement,
    /// `<Style src>` paths, in order.
    pub styles: Vec<String>,
}

/// Inline `style` attribute → declarations.
pub fn parse_inline_style(s: &str) -> Vec<(String, String)> {
    s.split(';')
        .filter_map(|d| {
            let (k, v) = d.split_once(':')?;
            let (k, v) = (k.trim(), v.trim());
            (!k.is_empty() && !v.is_empty()).then(|| (k.to_owned(), v.to_owned()))
        })
        .collect()
}

/// Parses UXML. `base` is the folder of the file (for relative `<Style src>`).
pub fn parse(text: &str, base: &str) -> Result<VisualTree, String> {
    let doc = roxmltree::Document::parse(text).map_err(|e| e.to_string())?;
    let root = doc.root_element();
    if root.tag_name().name() != "UXML" {
        return Err(format!(
            "the root element is <{}>, expected <UXML>",
            root.tag_name().name()
        ));
    }
    let mut styles = Vec::new();
    let mut tree = VisualTree {
        root: UxmlElement::new("UXML"),
        styles: Vec::new(),
    };
    fn convert(n: roxmltree::Node, base: &str, styles: &mut Vec<String>) -> Option<UxmlElement> {
        let kind = n.tag_name().name();
        if kind == "Style" {
            if let Some(src) = n.attribute("src") {
                styles.push(resolve(base, src));
            }
            return None;
        }
        let mut e = UxmlElement::new(kind);
        for a in n.attributes() {
            match a.name() {
                "name" => e.name = a.value().into(),
                "class" => {
                    e.classes = a.value().split_whitespace().map(str::to_owned).collect();
                }
                "style" => e.style = parse_inline_style(a.value()),
                k => e.attrs.push((k.into(), a.value().into())),
            }
        }
        for c in n.children().filter(|c| c.is_element()) {
            if let Some(ce) = convert(c, base, styles) {
                e.children.push(ce);
            }
        }
        Some(e)
    }
    for c in root.children().filter(|c| c.is_element()) {
        if let Some(e) = convert(c, base, &mut styles) {
            tree.root.children.push(e);
        }
    }
    tree.styles = styles;
    Ok(tree)
}

/// `src` relative to `base` (`project://database/Assets/...` and `/Assets/...` too).
pub fn resolve(base: &str, src: &str) -> String {
    let src = src
        .strip_prefix("project://database/")
        .unwrap_or(src)
        .split(['?', '#'])
        .next()
        .unwrap_or(src);
    if let Some(s) = src.strip_prefix('/') {
        return s.to_owned();
    }
    if src.starts_with("Assets/") {
        return src.to_owned();
    }
    let mut parts: Vec<&str> = base.split('/').filter(|p| !p.is_empty()).collect();
    for p in src.split('/') {
        match p {
            "." | "" => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Writes UXML in Unity's layout (`ui:` namespace, two-space indent).
pub fn write(tree: &VisualTree, base: &str) -> String {
    let mut out = String::from(
        "<ui:UXML xmlns:ui=\"UnityEngine.UIElements\" xmlns:uie=\"UnityEditor.UIElements\" editor-extension-mode=\"False\">\n",
    );
    for s in &tree.styles {
        let rel = relative(base, s);
        let _ = writeln!(out, "  <Style src=\"{}\" />", esc(&rel));
    }
    fn go(e: &UxmlElement, depth: usize, out: &mut String) {
        let pad = "  ".repeat(depth);
        let _ = write!(out, "{pad}<ui:{}", e.kind);
        if !e.name.is_empty() {
            let _ = write!(out, " name=\"{}\"", esc(&e.name));
        }
        for (k, v) in &e.attrs {
            let _ = write!(out, " {k}=\"{}\"", esc(v));
        }
        if !e.classes.is_empty() {
            let _ = write!(out, " class=\"{}\"", esc(&e.classes.join(" ")));
        }
        if !e.style.is_empty() {
            let s: Vec<String> = e.style.iter().map(|(k, v)| format!("{k}: {v};")).collect();
            let _ = write!(out, " style=\"{}\"", esc(&s.join(" ")));
        }
        if e.children.is_empty() {
            out.push_str(" />\n");
        } else {
            out.push_str(">\n");
            for c in &e.children {
                go(c, depth + 1, out);
            }
            let _ = writeln!(out, "{pad}</ui:{}>", e.kind);
        }
    }
    for c in &tree.root.children {
        go(c, 1, &mut out);
    }
    out.push_str("</ui:UXML>\n");
    out
}

/// `path` relative to the folder `base` (for `<Style src>`).
pub fn relative(base: &str, path: &str) -> String {
    let b: Vec<&str> = base.split('/').filter(|p| !p.is_empty()).collect();
    let p: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let common = b.iter().zip(&p).take_while(|(x, y)| x == y).count();
    if common == 0 {
        return format!("/{path}");
    }
    let mut parts: Vec<&str> = std::iter::repeat_n("..", b.len() - common).collect();
    parts.extend(&p[common..]);
    parts.join("/")
}

/// Unity's template for Create → UI Toolkit → UI Document.
pub const TEMPLATE: &str = "<ui:UXML xmlns:ui=\"UnityEngine.UIElements\" xmlns:uie=\"UnityEditor.UIElements\" editor-extension-mode=\"False\">\n</ui:UXML>\n";
