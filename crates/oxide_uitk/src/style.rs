//! Computed styles: USS properties with Unity's defaults, inheritance and value parsing.
//! Reference: <https://docs.unity3d.com/Manual/UIE-USS-SupportedProperties.html>

use std::collections::HashMap;

/// A length: `auto`, pixels or a percentage of the parent.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Len {
    #[default]
    Auto,
    Px(f32),
    Percent(f32),
}

impl Len {
    pub fn parse(s: &str) -> Option<Len> {
        let s = s.trim();
        if s == "auto" || s == "initial" || s == "none" {
            return Some(Len::Auto);
        }
        if let Some(p) = s.strip_suffix('%') {
            return p.trim().parse().ok().map(Len::Percent);
        }
        s.strip_suffix("px")
            .unwrap_or(s)
            .trim()
            .parse()
            .ok()
            .map(Len::Px)
    }

    pub fn px_or(self, of: f32, auto: f32) -> f32 {
        match self {
            Len::Auto => auto,
            Len::Px(v) => v,
            Len::Percent(p) => of * p / 100.0,
        }
    }
}

/// RGBA, 0..1, sRGB.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba(pub [f32; 4]);

impl Rgba {
    pub const CLEAR: Rgba = Rgba([0.0; 4]);
    pub const WHITE: Rgba = Rgba([1.0; 4]);
    pub const BLACK: Rgba = Rgba([0.0, 0.0, 0.0, 1.0]);

    pub fn parse(s: &str) -> Option<Rgba> {
        let s = s.trim();
        if let Some(h) = s.strip_prefix('#') {
            let hex = |a: &str| u8::from_str_radix(a, 16).ok().map(|v| v as f32 / 255.0);
            let dbl = |c: char| {
                let s: String = [c, c].iter().collect();
                u8::from_str_radix(&s, 16).ok().map(|v| v as f32 / 255.0)
            };
            let cs: Vec<char> = h.chars().collect();
            return Some(Rgba(match cs.len() {
                3 => [dbl(cs[0])?, dbl(cs[1])?, dbl(cs[2])?, 1.0],
                4 => [dbl(cs[0])?, dbl(cs[1])?, dbl(cs[2])?, dbl(cs[3])?],
                6 => [hex(&h[0..2])?, hex(&h[2..4])?, hex(&h[4..6])?, 1.0],
                8 => [
                    hex(&h[0..2])?,
                    hex(&h[2..4])?,
                    hex(&h[4..6])?,
                    hex(&h[6..8])?,
                ],
                _ => return None,
            }));
        }
        if let Some(args) = s
            .strip_prefix("rgba(")
            .or_else(|| s.strip_prefix("rgb("))
            .and_then(|r| r.strip_suffix(')'))
        {
            let v: Vec<f32> = args
                .split(',')
                .filter_map(|x| x.trim().parse::<f32>().ok())
                .collect();
            return match v.len() {
                3 => Some(Rgba([v[0] / 255.0, v[1] / 255.0, v[2] / 255.0, 1.0])),
                4 => Some(Rgba([v[0] / 255.0, v[1] / 255.0, v[2] / 255.0, v[3]])),
                _ => None,
            };
        }
        Some(match s {
            "black" => Rgba::BLACK,
            "white" => Rgba::WHITE,
            "red" => Rgba([1.0, 0.0, 0.0, 1.0]),
            "green" => Rgba([0.0, 1.0, 0.0, 1.0]),
            "blue" => Rgba([0.0, 0.0, 1.0, 1.0]),
            "yellow" => Rgba([1.0, 0.92, 0.016, 1.0]),
            "cyan" => Rgba([0.0, 1.0, 1.0, 1.0]),
            "magenta" => Rgba([1.0, 0.0, 1.0, 1.0]),
            "gray" | "grey" => Rgba([0.5, 0.5, 0.5, 1.0]),
            "orange" => Rgba([1.0, 0.65, 0.0, 1.0]),
            "clear" | "transparent" | "initial" => Rgba::CLEAR,
            _ => return None,
        })
    }

    pub fn to_egui(self, opacity: f32) -> egui::Color32 {
        let c = self.0;
        egui::Color32::from_rgba_unmultiplied(
            (c[0].clamp(0.0, 1.0) * 255.0) as u8,
            (c[1].clamp(0.0, 1.0) * 255.0) as u8,
            (c[2].clamp(0.0, 1.0) * 255.0) as u8,
            (c[3].clamp(0.0, 1.0) * opacity.clamp(0.0, 1.0) * 255.0) as u8,
        )
    }
}

macro_rules! keyword_enum {
    ($name:ident { $($variant:ident = $text:literal),* $(,)? } default $def:ident = $deftext:literal) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
        pub enum $name {
            #[default]
            $def,
            $($variant),*
        }
        impl $name {
            pub fn parse(s: &str) -> Option<Self> {
                match s.trim() {
                    $deftext => Some(Self::$def),
                    $($text => Some(Self::$variant),)*
                    _ => None,
                }
            }
        }
    };
}

keyword_enum!(Display { None = "none" } default Flex = "flex");
keyword_enum!(Position { Absolute = "absolute" } default Relative = "relative");
keyword_enum!(FlexDirection { Row = "row", RowReverse = "row-reverse", ColumnReverse = "column-reverse" } default Column = "column");
keyword_enum!(Wrap { Wrap = "wrap", WrapReverse = "wrap-reverse" } default NoWrap = "nowrap");
keyword_enum!(Align { FlexStart = "flex-start", Center = "center", FlexEnd = "flex-end", Stretch = "stretch" } default Auto = "auto");
keyword_enum!(Justify { Center = "center", FlexEnd = "flex-end", SpaceBetween = "space-between", SpaceAround = "space-around", SpaceEvenly = "space-evenly" } default FlexStart = "flex-start");
keyword_enum!(Overflow { Hidden = "hidden" } default Visible = "visible");
keyword_enum!(Visibility { Hidden = "hidden" } default Visible = "visible");
keyword_enum!(FontStyle { Bold = "bold", Italic = "italic", BoldAndItalic = "bold-and-italic" } default Normal = "normal");
keyword_enum!(WhiteSpace { NoWrap = "nowrap" } default Normal = "normal");
keyword_enum!(ScaleMode { ScaleAndCrop = "scale-and-crop", ScaleToFit = "scale-to-fit" } default StretchToFill = "stretch-to-fill");

/// `-unity-text-align`: (horizontal, vertical), each 0 = start, 0.5 = middle, 1 = end.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextAlign(pub f32, pub f32);

impl Default for TextAlign {
    fn default() -> Self {
        TextAlign(0.0, 0.0)
    }
}

impl TextAlign {
    pub fn parse(s: &str) -> Option<TextAlign> {
        let (v, h) = s.trim().split_once('-')?;
        let v = match v {
            "upper" => 0.0,
            "middle" => 0.5,
            "lower" => 1.0,
            _ => return None,
        };
        let h = match h {
            "left" => 0.0,
            "center" => 0.5,
            "right" => 1.0,
            _ => return None,
        };
        Some(TextAlign(h, v))
    }
}

/// Every supported USS property of one element, resolved.
#[derive(Clone, Debug, PartialEq)]
pub struct ComputedStyle {
    pub display: Display,
    pub position: Position,
    pub left: Len,
    pub top: Len,
    pub right: Len,
    pub bottom: Len,
    pub width: Len,
    pub height: Len,
    pub min_width: Len,
    pub min_height: Len,
    pub max_width: Len,
    pub max_height: Len,
    /// left, top, right, bottom.
    pub margin: [Len; 4],
    pub padding: [Len; 4],
    pub border_width: [f32; 4],
    pub border_color: [Rgba; 4],
    /// top-left, top-right, bottom-right, bottom-left.
    pub border_radius: [f32; 4],
    pub flex_direction: FlexDirection,
    pub flex_wrap: Wrap,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    pub flex_basis: Len,
    pub align_items: Align,
    pub align_self: Align,
    pub align_content: Align,
    pub justify_content: Justify,
    pub overflow: Overflow,
    pub background_color: Rgba,
    pub background_image: Option<String>,
    pub background_tint: Rgba,
    pub background_scale_mode: ScaleMode,
    pub opacity: f32,
    pub translate: [Len; 2],
    pub rotate: f32,
    pub scale: [f32; 2],
    // Inherited.
    pub color: Rgba,
    pub font_size: f32,
    pub font_style: FontStyle,
    pub text_align: TextAlign,
    pub white_space: WhiteSpace,
    pub visibility: Visibility,
    pub letter_spacing: f32,
    /// USS custom properties (`--name`), inherited.
    pub vars: HashMap<String, String>,
}

impl Default for ComputedStyle {
    fn default() -> Self {
        Self {
            display: Display::Flex,
            position: Position::Relative,
            left: Len::Auto,
            top: Len::Auto,
            right: Len::Auto,
            bottom: Len::Auto,
            width: Len::Auto,
            height: Len::Auto,
            min_width: Len::Auto,
            min_height: Len::Auto,
            max_width: Len::Auto,
            max_height: Len::Auto,
            margin: [Len::Px(0.0); 4],
            padding: [Len::Px(0.0); 4],
            border_width: [0.0; 4],
            border_color: [Rgba::BLACK; 4],
            border_radius: [0.0; 4],
            flex_direction: FlexDirection::Column,
            flex_wrap: Wrap::NoWrap,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: Len::Auto,
            align_items: Align::Stretch,
            align_self: Align::Auto,
            align_content: Align::FlexStart,
            justify_content: Justify::FlexStart,
            overflow: Overflow::Visible,
            background_color: Rgba::CLEAR,
            background_image: None,
            background_tint: Rgba::WHITE,
            background_scale_mode: ScaleMode::StretchToFill,
            opacity: 1.0,
            translate: [Len::Px(0.0); 2],
            rotate: 0.0,
            scale: [1.0; 2],
            color: Rgba::BLACK,
            font_size: 12.0,
            font_style: FontStyle::Normal,
            text_align: TextAlign::default(),
            white_space: WhiteSpace::Normal,
            visibility: Visibility::Visible,
            letter_spacing: 0.0,
            vars: HashMap::new(),
        }
    }
}

/// The USS properties this implementation applies (UI Builder lists them).
pub const PROPERTIES: &[&str] = &[
    "display",
    "position",
    "left",
    "top",
    "right",
    "bottom",
    "width",
    "height",
    "min-width",
    "min-height",
    "max-width",
    "max-height",
    "margin",
    "margin-left",
    "margin-top",
    "margin-right",
    "margin-bottom",
    "padding",
    "padding-left",
    "padding-top",
    "padding-right",
    "padding-bottom",
    "border-width",
    "border-left-width",
    "border-top-width",
    "border-right-width",
    "border-bottom-width",
    "border-color",
    "border-left-color",
    "border-top-color",
    "border-right-color",
    "border-bottom-color",
    "border-radius",
    "border-top-left-radius",
    "border-top-right-radius",
    "border-bottom-right-radius",
    "border-bottom-left-radius",
    "flex-direction",
    "flex-wrap",
    "flex-grow",
    "flex-shrink",
    "flex-basis",
    "flex",
    "align-items",
    "align-self",
    "align-content",
    "justify-content",
    "overflow",
    "background-color",
    "background-image",
    "-unity-background-image-tint-color",
    "-unity-background-scale-mode",
    "opacity",
    "translate",
    "rotate",
    "scale",
    "color",
    "font-size",
    "-unity-font-style",
    "-unity-text-align",
    "white-space",
    "visibility",
    "letter-spacing",
];

/// Splits a value list on spaces, keeping `rgb(...)` together.
fn tokens(v: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0;
    for ch in v.chars() {
        match ch {
            '(' => {
                depth += 1;
                cur.push(ch);
            }
            ')' => {
                depth -= 1;
                cur.push(ch);
            }
            c if c.is_whitespace() && depth == 0 => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// CSS box shorthand: 1-4 values → left, top, right, bottom.
fn four<T: Copy>(vals: &[T]) -> Option<[T; 4]> {
    // CSS order is top, right, bottom, left.
    let (t, r, b, l) = match vals {
        [a] => (*a, *a, *a, *a),
        [a, b] => (*a, *b, *a, *b),
        [a, b, c] => (*a, *b, *c, *b),
        [a, b, c, d] => (*a, *b, *c, *d),
        _ => return None,
    };
    Some([l, t, r, b])
}

/// Replaces `var(--x)` / `var(--x, fallback)` with the variable's value.
pub fn substitute(v: &str, vars: &HashMap<String, String>) -> String {
    let mut out = v.to_owned();
    for _ in 0..8 {
        let Some(i) = out.find("var(") else { break };
        let Some(j) = out[i..].find(')') else { break };
        let inner = &out[i + 4..i + j];
        let (name, fallback) = match inner.split_once(',') {
            Some((n, f)) => (n.trim(), Some(f.trim())),
            None => (inner.trim(), None),
        };
        let value = vars
            .get(name)
            .cloned()
            .or(fallback.map(str::to_owned))
            .unwrap_or_default();
        out = format!("{}{}{}", &out[..i], value, &out[i + j + 1..]);
    }
    out
}

fn url(v: &str) -> Option<String> {
    let v = v.trim();
    let inner = v
        .strip_prefix("url(")
        .or_else(|| v.strip_prefix("resource("))?
        .strip_suffix(')')?;
    Some(inner.trim().trim_matches(['"', '\'']).to_owned())
}

impl ComputedStyle {
    /// A child's starting style: inherited properties from `parent`, the rest default.
    pub fn inherit(parent: &ComputedStyle) -> ComputedStyle {
        ComputedStyle {
            color: parent.color,
            font_size: parent.font_size,
            font_style: parent.font_style,
            text_align: parent.text_align,
            white_space: parent.white_space,
            visibility: parent.visibility,
            letter_spacing: parent.letter_spacing,
            vars: parent.vars.clone(),
            ..Default::default()
        }
    }

    /// Applies one declaration (`var()` already substituted). Returns false when the
    /// property or value isn't understood.
    pub fn apply(&mut self, prop: &str, v: &str, base: &str) -> bool {
        let v = v.trim();
        let len = || Len::parse(v);
        let px = || Len::parse(v).map(|l| l.px_or(0.0, 0.0));
        macro_rules! set {
            ($field:expr, $val:expr) => {
                match $val {
                    Some(x) => {
                        $field = x;
                        true
                    }
                    None => false,
                }
            };
        }
        let side = |p: &str| match p {
            "left" => Some(0),
            "top" => Some(1),
            "right" => Some(2),
            "bottom" => Some(3),
            _ => None,
        };
        match prop {
            "display" => set!(self.display, Display::parse(v)),
            "position" => set!(self.position, Position::parse(v)),
            "left" => set!(self.left, len()),
            "top" => set!(self.top, len()),
            "right" => set!(self.right, len()),
            "bottom" => set!(self.bottom, len()),
            "width" => set!(self.width, len()),
            "height" => set!(self.height, len()),
            "min-width" => set!(self.min_width, len()),
            "min-height" => set!(self.min_height, len()),
            "max-width" => set!(self.max_width, len()),
            "max-height" => set!(self.max_height, len()),
            "margin" | "padding" => {
                let vals: Option<Vec<Len>> = tokens(v).iter().map(|t| Len::parse(t)).collect();
                let Some(f) = vals.as_deref().and_then(four) else {
                    return false;
                };
                if prop == "margin" {
                    self.margin = f;
                } else {
                    self.padding = f;
                }
                true
            }
            "border-width" => {
                let vals: Option<Vec<f32>> = tokens(v)
                    .iter()
                    .map(|t| Len::parse(t).map(|l| l.px_or(0.0, 0.0)))
                    .collect();
                set!(self.border_width, vals.as_deref().and_then(four))
            }
            "border-color" => {
                let vals: Option<Vec<Rgba>> = tokens(v).iter().map(|t| Rgba::parse(t)).collect();
                set!(self.border_color, vals.as_deref().and_then(four))
            }
            "border-radius" => {
                let vals: Option<Vec<f32>> = tokens(v)
                    .iter()
                    .map(|t| Len::parse(t).map(|l| l.px_or(0.0, 0.0)))
                    .collect();
                // CSS radius order: top-left, top-right, bottom-right, bottom-left.
                let Some(vals) = vals else { return false };
                let r = match vals[..] {
                    [a] => [a; 4],
                    [a, b] => [a, b, a, b],
                    [a, b, c] => [a, b, c, b],
                    [a, b, c, d] => [a, b, c, d],
                    _ => return false,
                };
                self.border_radius = r;
                true
            }
            "border-top-left-radius" => set!(self.border_radius[0], px()),
            "border-top-right-radius" => set!(self.border_radius[1], px()),
            "border-bottom-right-radius" => set!(self.border_radius[2], px()),
            "border-bottom-left-radius" => set!(self.border_radius[3], px()),
            p if p.starts_with("margin-") => match side(&p[7..]) {
                Some(i) => set!(self.margin[i], len()),
                None => false,
            },
            p if p.starts_with("padding-") => match side(&p[8..]) {
                Some(i) => set!(self.padding[i], len()),
                None => false,
            },
            p if p.starts_with("border-") && p.ends_with("-width") => {
                match side(&p[7..p.len() - 6]) {
                    Some(i) => set!(self.border_width[i], px()),
                    None => false,
                }
            }
            p if p.starts_with("border-") && p.ends_with("-color") => {
                match side(&p[7..p.len() - 6]) {
                    Some(i) => set!(self.border_color[i], Rgba::parse(v)),
                    None => false,
                }
            }
            "flex-direction" => set!(self.flex_direction, FlexDirection::parse(v)),
            "flex-wrap" => set!(self.flex_wrap, Wrap::parse(v)),
            "flex-grow" => set!(self.flex_grow, v.parse().ok()),
            "flex-shrink" => set!(self.flex_shrink, v.parse().ok()),
            "flex-basis" => set!(self.flex_basis, len()),
            "flex" => {
                let t = tokens(v);
                if t.len() == 1 && t[0] == "none" {
                    self.flex_grow = 0.0;
                    self.flex_shrink = 0.0;
                    return true;
                }
                let Some(g) = t.first().and_then(|x| x.parse().ok()) else {
                    return false;
                };
                self.flex_grow = g;
                self.flex_shrink = t.get(1).and_then(|x| x.parse().ok()).unwrap_or(1.0);
                self.flex_basis = t.get(2).and_then(|x| Len::parse(x)).unwrap_or(Len::Px(0.0));
                true
            }
            "align-items" => set!(self.align_items, Align::parse(v)),
            "align-self" => set!(self.align_self, Align::parse(v)),
            "align-content" => set!(self.align_content, Align::parse(v)),
            "justify-content" => set!(self.justify_content, Justify::parse(v)),
            "overflow" => set!(self.overflow, Overflow::parse(v)),
            "background-color" => set!(self.background_color, Rgba::parse(v)),
            "background-image" => {
                self.background_image = url(v).map(|u| crate::uxml::resolve(base, &u));
                true
            }
            "-unity-background-image-tint-color" => set!(self.background_tint, Rgba::parse(v)),
            "-unity-background-scale-mode" => set!(self.background_scale_mode, ScaleMode::parse(v)),
            "opacity" => set!(self.opacity, v.parse().ok()),
            "translate" => {
                let t: Option<Vec<Len>> = tokens(v).iter().map(|x| Len::parse(x)).collect();
                match t.as_deref() {
                    Some([x]) => {
                        self.translate = [*x, Len::Px(0.0)];
                        true
                    }
                    Some([x, y, ..]) => {
                        self.translate = [*x, *y];
                        true
                    }
                    _ => false,
                }
            }
            "rotate" => set!(
                self.rotate,
                v.strip_suffix("deg").unwrap_or(v).trim().parse().ok()
            ),
            "scale" => {
                let t: Vec<f32> = tokens(v).iter().filter_map(|x| x.parse().ok()).collect();
                match t[..] {
                    [s] => {
                        self.scale = [s, s];
                        true
                    }
                    [x, y, ..] => {
                        self.scale = [x, y];
                        true
                    }
                    _ => false,
                }
            }
            "color" => set!(self.color, Rgba::parse(v)),
            "font-size" => set!(self.font_size, px()),
            "-unity-font-style" => set!(self.font_style, FontStyle::parse(v)),
            "-unity-text-align" => set!(self.text_align, TextAlign::parse(v)),
            "white-space" => set!(self.white_space, WhiteSpace::parse(v)),
            "visibility" => set!(self.visibility, Visibility::parse(v)),
            "letter-spacing" => set!(self.letter_spacing, px()),
            _ => false,
        }
    }

    /// The style of an element from its parent's and its matching declarations (lowest
    /// priority first). `base` is the folder relative URLs resolve against.
    pub fn compute(parent: &ComputedStyle, decls: &[(&str, &str, &str)]) -> ComputedStyle {
        let mut s = ComputedStyle::inherit(parent);
        for (k, v, _) in decls {
            if k.starts_with("--") {
                let v = substitute(v, &s.vars);
                s.vars.insert((*k).to_owned(), v);
            }
        }
        for (k, v, base) in decls {
            if k.starts_with("--") {
                continue;
            }
            let v = substitute(v, &s.vars);
            s.apply(k, &v, base);
        }
        s
    }
}
