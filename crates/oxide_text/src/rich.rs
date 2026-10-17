//! TextMeshPro rich text tags. Unknown or malformed tags are shown as typed, like TMP.
//! Reference: <https://docs.unity3d.com/Packages/com.unity.ugui@2.0/manual/RichText.html>

use oxide_math::Color;

/// Size given by a `<size>` tag.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SizeSpec {
    /// In the text's units (points).
    Absolute(f32),
    /// Relative to the base size.
    Scale(f32),
}

/// Letter case from `<uppercase>`, `<lowercase>` and `<smallcaps>`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Case {
    #[default]
    Normal,
    Upper,
    Lower,
    SmallCaps,
}

/// `<sub>` / `<sup>`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Script {
    #[default]
    Normal,
    Sub,
    Sup,
}

/// The style in effect for one character.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    /// `<color>` (sRGB), multiplied with the text colour.
    pub color: Option<Color>,
    /// `<alpha>`.
    pub alpha: Option<f32>,
    pub size: Option<SizeSpec>,
    /// Extra character spacing, em.
    pub cspace: f32,
    /// Highlight behind the characters.
    pub mark: Option<Color>,
    pub case: Case,
    pub script: Script,
    /// `<link>`: index into [`RichText::links`].
    pub link: Option<u16>,
    /// `<sprite>` (on its placeholder character): index into [`RichText::sprites`].
    pub sprite: Option<u16>,
    /// `<gradient>`: index into [`RichText::gradients`].
    pub gradient: Option<u16>,
}

/// The character a `<sprite>` takes the place of.
pub const SPRITE_CHAR: char = '\u{FFFC}';

/// A `<sprite>` tag: which Sprite Asset (`None` = the text's default) and which sprite.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpriteTag {
    pub asset: Option<String>,
    pub index: Option<u32>,
    pub name: Option<String>,
    /// `tint=1`: multiplied by the text colour.
    pub tint: bool,
    pub color: Option<Color>,
}

/// Parsed rich text: styled characters plus the tables their styles refer to.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RichText {
    pub chars: Vec<StyledChar>,
    /// `<link="id">` ids.
    pub links: Vec<String>,
    pub sprites: Vec<SpriteTag>,
    /// `<gradient="name">` preset names.
    pub gradients: Vec<String>,
}

/// Splits a tag body (`sprite="Emoji" index=3 tint=1`) into its name, its own value and
/// attributes (names lowercased; quotes removed).
fn tag_attributes(tag: &str) -> (String, Option<String>, Vec<(String, String)>) {
    let mut parts: Vec<(String, Option<String>)> = Vec::new();
    let chars: Vec<char> = tag.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        let start = i;
        while i < chars.len() && !chars[i].is_whitespace() && chars[i] != '=' {
            i += 1;
        }
        let name: String = chars[start..i]
            .iter()
            .collect::<String>()
            .to_ascii_lowercase();
        if name.is_empty() {
            i += 1;
            continue;
        }
        let mut value = None;
        if i < chars.len() && chars[i] == '=' {
            i += 1;
            if i < chars.len() && (chars[i] == '"' || chars[i] == '\'') {
                let q = chars[i];
                let vs = i + 1;
                let end = chars[vs..]
                    .iter()
                    .position(|&c| c == q)
                    .map_or(chars.len(), |p| vs + p);
                value = Some(chars[vs..end].iter().collect());
                i = end + 1;
            } else {
                let vs = i;
                while i < chars.len() && !chars[i].is_whitespace() {
                    i += 1;
                }
                value = Some(chars[vs..i].iter().collect());
            }
        }
        parts.push((name, value));
    }
    let mut it = parts.into_iter();
    let (name, value) = it.next().unwrap_or_default();
    (
        name,
        value,
        it.map(|(k, v)| (k, v.unwrap_or_default())).collect(),
    )
}

/// The `<sprite>` tag in `tag`, if it is one.
fn sprite_tag(tag: &str) -> Option<SpriteTag> {
    let (name, value, attrs) = tag_attributes(tag);
    if name != "sprite" {
        return None;
    }
    let mut t = SpriteTag::default();
    // `<sprite=3>` is an index; `<sprite="Asset" ...>` names the asset.
    if let Some(v) = value {
        match v.parse::<u32>() {
            Ok(i) if !tag.contains('"') => t.index = Some(i),
            _ => t.asset = Some(v),
        }
    }
    for (k, v) in attrs {
        match k.as_str() {
            "index" => t.index = Some(v.parse().ok()?),
            "name" => t.name = Some(v),
            "tint" => t.tint = v == "1" || v.eq_ignore_ascii_case("true"),
            "color" => t.color = Some(parse_color(&v)?),
            "anim" => {}
            _ => return None,
        }
    }
    (t.index.is_some() || t.name.is_some()).then_some(t)
}

/// One character with its style. `'\n'` breaks the line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StyledChar {
    pub c: char,
    pub style: Style,
}

/// Colour names TMP accepts in `<color="name">`.
fn named(name: &str) -> Option<Color> {
    Some(match name {
        "black" => Color::rgb(0.0, 0.0, 0.0),
        "blue" => Color::rgb(0.0, 0.0, 1.0),
        "green" => Color::rgb(0.0, 1.0, 0.0),
        "orange" => Color::rgb(1.0, 0.5, 0.0),
        "purple" => Color::rgb(0.63, 0.13, 0.94),
        "red" => Color::rgb(1.0, 0.0, 0.0),
        "white" => Color::rgb(1.0, 1.0, 1.0),
        "yellow" => Color::rgb(1.0, 1.0, 0.0),
        _ => return None,
    })
}

/// `#RGB`, `#RGBA`, `#RRGGBB`, `#RRGGBBAA` or a colour name.
pub fn parse_color(v: &str) -> Option<Color> {
    let v = v.trim().trim_matches('"');
    let Some(hex) = v.strip_prefix('#') else {
        return named(&v.to_ascii_lowercase());
    };
    let digits: Vec<u8> = hex
        .chars()
        .map(|c| c.to_digit(16).map(|d| d as u8))
        .collect::<Option<_>>()?;
    let (r, g, b, a) = match digits.len() {
        3 | 4 => {
            let e = |i: usize| digits.get(i).map_or(255, |d| d * 17);
            (e(0), e(1), e(2), e(3))
        }
        6 | 8 => {
            let p = |i: usize| digits.get(i).map_or(255, |d| d * 16 + digits[i + 1]);
            (p(0), p(2), p(4), p(6))
        }
        _ => return None,
    };
    Some(Color::new(
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
        a as f32 / 255.0,
    ))
}

/// A `<size>` value: a spec, or `+n` / `-n` points relative to the current size.
enum SizeArg {
    Spec(SizeSpec),
    Relative(f32),
}

fn parse_size(v: &str) -> Option<SizeArg> {
    let v = v.trim().trim_matches('"');
    if let Some(p) = v.strip_suffix('%') {
        return p
            .parse::<f32>()
            .ok()
            .map(|p| SizeArg::Spec(SizeSpec::Scale(p / 100.0)));
    }
    if let Some(e) = v.strip_suffix("em") {
        return e
            .parse::<f32>()
            .ok()
            .map(|e| SizeArg::Spec(SizeSpec::Scale(e)));
    }
    let v = v.strip_suffix("px").unwrap_or(v);
    if v.starts_with('+') || v.starts_with('-') {
        return v.parse::<f32>().ok().map(SizeArg::Relative);
    }
    v.parse::<f32>()
        .ok()
        .map(|a| SizeArg::Spec(SizeSpec::Absolute(a)))
}

/// Spacing in em: `0.1em`, or points relative to `base_size`.
fn parse_em(v: &str, base_size: f32) -> Option<f32> {
    let v = v.trim().trim_matches('"');
    match v.strip_suffix("em") {
        Some(e) => e.parse().ok(),
        None => v
            .strip_suffix("px")
            .unwrap_or(v)
            .parse::<f32>()
            .ok()
            .map(|p| p / base_size.max(1e-3)),
    }
}

/// Splits `text` into styled characters. With `rich` off, tags are plain text.
pub fn parse(text: &str, rich: bool, base_size: f32) -> Vec<StyledChar> {
    parse_rich(text, rich, base_size).chars
}

/// [`parse`] with the link, sprite and gradient tables.
pub fn parse_rich(text: &str, rich: bool, base_size: f32) -> RichText {
    let chars: Vec<char> = text.chars().collect();
    let mut rt = RichText::default();
    let mut out = Vec::with_capacity(chars.len());
    let mut links: Vec<Option<u16>> = Vec::new();
    let mut gradients: Vec<Option<u16>> = Vec::new();
    let mut st = Style::default();
    // Stacks so nested tags restore the outer value.
    let mut colors: Vec<Option<Color>> = Vec::new();
    let mut alphas: Vec<Option<f32>> = Vec::new();
    let mut sizes: Vec<Option<SizeSpec>> = Vec::new();
    let mut cspaces: Vec<f32> = Vec::new();
    let mut marks: Vec<Option<Color>> = Vec::new();
    let mut cases: Vec<Case> = Vec::new();
    let mut scripts: Vec<Script> = Vec::new();
    let mut noparse = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if rich && c == '<' {
            let end = chars[i + 1..].iter().position(|&c| c == '>' || c == '<');
            if let Some(n) = end.filter(|&n| chars[i + 1 + n] == '>') {
                let tag: String = chars[i + 1..i + 1 + n].iter().collect();
                let lower = tag.to_ascii_lowercase();
                let (name, value) = match lower.split_once('=') {
                    Some((k, _)) => (k.trim().to_owned(), Some(tag[k.len() + 1..].trim())),
                    None => (lower.trim().to_owned(), None),
                };
                let attr_value = || {
                    let (_, v, attrs) = tag_attributes(&tag);
                    v.filter(|_| attrs.is_empty())
                };
                let handled = if noparse {
                    if name == "/noparse" {
                        noparse = false;
                        true
                    } else {
                        false
                    }
                } else {
                    match (name.as_str(), value) {
                        ("noparse", None) => {
                            noparse = true;
                            true
                        }
                        ("b", None) => {
                            st.bold = true;
                            true
                        }
                        ("/b", None) => {
                            st.bold = false;
                            true
                        }
                        ("i", None) => {
                            st.italic = true;
                            true
                        }
                        ("/i", None) => {
                            st.italic = false;
                            true
                        }
                        ("u", None) => {
                            st.underline = true;
                            true
                        }
                        ("/u", None) => {
                            st.underline = false;
                            true
                        }
                        ("s", None) => {
                            st.strikethrough = true;
                            true
                        }
                        ("/s", None) => {
                            st.strikethrough = false;
                            true
                        }
                        ("br", None) => {
                            out.push(StyledChar { c: '\n', style: st });
                            true
                        }
                        ("color", Some(v)) => match parse_color(v) {
                            Some(col) => {
                                colors.push(st.color);
                                st.color = Some(col);
                                true
                            }
                            None => false,
                        },
                        ("/color", None) => {
                            st.color = colors.pop().flatten();
                            true
                        }
                        ("alpha", Some(v)) => {
                            match parse_color(&format!("#ffffff{}", v.trim_start_matches('#'))) {
                                Some(col) => {
                                    alphas.push(st.alpha);
                                    st.alpha = Some(col.a);
                                    true
                                }
                                None => false,
                            }
                        }
                        ("/alpha", None) => {
                            st.alpha = alphas.pop().flatten();
                            true
                        }
                        ("size", Some(v)) => match parse_size(v) {
                            Some(s) => {
                                sizes.push(st.size);
                                st.size = Some(match s {
                                    SizeArg::Relative(d) => {
                                        let cur = match st.size {
                                            Some(SizeSpec::Absolute(a)) => a,
                                            Some(SizeSpec::Scale(k)) => base_size * k,
                                            None => base_size,
                                        };
                                        SizeSpec::Absolute((cur + d).max(0.0))
                                    }
                                    SizeArg::Spec(s) => s,
                                });
                                true
                            }
                            None => false,
                        },
                        ("/size", None) => {
                            st.size = sizes.pop().flatten();
                            true
                        }
                        ("cspace", Some(v)) => match parse_em(v, base_size) {
                            Some(s) => {
                                cspaces.push(st.cspace);
                                st.cspace = s;
                                true
                            }
                            None => false,
                        },
                        ("/cspace", None) => {
                            st.cspace = cspaces.pop().unwrap_or(0.0);
                            true
                        }
                        ("mark", Some(v)) => match parse_color(v) {
                            Some(col) => {
                                marks.push(st.mark);
                                st.mark = Some(col);
                                true
                            }
                            None => false,
                        },
                        ("/mark", None) => {
                            st.mark = marks.pop().flatten();
                            true
                        }
                        ("uppercase" | "allcaps", None) => {
                            cases.push(st.case);
                            st.case = Case::Upper;
                            true
                        }
                        ("lowercase", None) => {
                            cases.push(st.case);
                            st.case = Case::Lower;
                            true
                        }
                        ("smallcaps", None) => {
                            cases.push(st.case);
                            st.case = Case::SmallCaps;
                            true
                        }
                        ("/uppercase" | "/allcaps" | "/lowercase" | "/smallcaps", None) => {
                            st.case = cases.pop().unwrap_or_default();
                            true
                        }
                        ("link", Some(_)) => match attr_value() {
                            Some(id) => {
                                links.push(st.link);
                                st.link = Some(rt.links.len() as u16);
                                rt.links.push(id);
                                true
                            }
                            None => false,
                        },
                        ("/link", None) => {
                            st.link = links.pop().flatten();
                            true
                        }
                        ("gradient", Some(_)) => match attr_value() {
                            Some(name) => {
                                gradients.push(st.gradient);
                                st.gradient = Some(rt.gradients.len() as u16);
                                rt.gradients.push(name);
                                true
                            }
                            None => false,
                        },
                        ("/gradient", None) => {
                            st.gradient = gradients.pop().flatten();
                            true
                        }
                        _ if name.starts_with("sprite") => match sprite_tag(&tag) {
                            Some(t) => {
                                let mut sst = st;
                                sst.sprite = Some(rt.sprites.len() as u16);
                                rt.sprites.push(t);
                                out.push(StyledChar {
                                    c: SPRITE_CHAR,
                                    style: sst,
                                });
                                true
                            }
                            None => false,
                        },
                        ("sub", None) => {
                            scripts.push(st.script);
                            st.script = Script::Sub;
                            true
                        }
                        ("sup", None) => {
                            scripts.push(st.script);
                            st.script = Script::Sup;
                            true
                        }
                        ("/sub" | "/sup", None) => {
                            st.script = scripts.pop().unwrap_or_default();
                            true
                        }
                        _ => false,
                    }
                };
                if handled {
                    i += n + 2;
                    continue;
                }
            }
        }
        out.push(StyledChar { c, style: st });
        i += 1;
    }
    rt.chars = out;
    rt
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(v: &[StyledChar]) -> String {
        v.iter().map(|c| c.c).collect()
    }

    #[test]
    fn tags_style_characters_and_nest() {
        let v = parse(
            "a<b>b<color=#ff0000>c<color=blue>d</color>e</color></b>f",
            true,
            36.0,
        );
        assert_eq!(text(&v), "abcdef");
        assert!(!v[0].style.bold && v[1].style.bold && !v[5].style.bold);
        assert_eq!(v[2].style.color, Some(Color::rgb(1.0, 0.0, 0.0)));
        assert_eq!(v[3].style.color, Some(Color::rgb(0.0, 0.0, 1.0)));
        assert_eq!(
            v[4].style.color,
            Some(Color::rgb(1.0, 0.0, 0.0)),
            "outer colour restored"
        );
        assert_eq!(v[5].style.color, None);
    }

    #[test]
    fn sizes_spacing_and_breaks() {
        let v = parse(
            "<size=50%>a</size><size=20>b<size=+4>c</size></size><br><cspace=0.5em>d",
            true,
            40.0,
        );
        assert_eq!(text(&v), "abc\nd");
        assert_eq!(v[0].style.size, Some(SizeSpec::Scale(0.5)));
        assert_eq!(v[1].style.size, Some(SizeSpec::Absolute(20.0)));
        assert_eq!(v[2].style.size, Some(SizeSpec::Absolute(24.0)));
        assert_eq!(v[4].style.cspace, 0.5);
    }

    #[test]
    fn unknown_tags_noparse_and_plain_mode_show_text() {
        assert_eq!(text(&parse("<foo>x", true, 10.0)), "<foo>x");
        assert_eq!(text(&parse("<noparse><b></noparse>x", true, 10.0)), "<b>x");
        assert_eq!(text(&parse("<b>x", false, 10.0)), "<b>x");
        assert_eq!(text(&parse("a < b", true, 10.0)), "a < b");
        assert_eq!(text(&parse("<color=nope>x", true, 10.0)), "<color=nope>x");
    }

    #[test]
    fn links_sprites_and_gradients() {
        let r = parse_rich(
            "Go <link=\"shop\">to <b>shop</b></link> <sprite=2> <sprite=\"Emoji\" name=\"smile\" tint=1> <gradient=\"Fire\">hot</gradient>",
            true,
            10.0,
        );
        assert_eq!(r.links, ["shop"]);
        assert_eq!(r.gradients, ["Fire"]);
        let t: String = r.chars.iter().map(|c| c.c).collect();
        assert_eq!(t, "Go to shop \u{FFFC} \u{FFFC} hot");
        assert_eq!(r.chars[3].style.link, Some(0));
        assert_eq!(r.chars[9].style.link, Some(0));
        assert!(r.chars[9].style.bold);
        assert_eq!(r.chars[10].style.link, None);
        assert_eq!(r.sprites[0].index, Some(2));
        assert_eq!(r.chars[11].style.sprite, Some(0));
        assert_eq!(r.sprites[1].asset.as_deref(), Some("Emoji"));
        assert_eq!(r.sprites[1].name.as_deref(), Some("smile"));
        assert!(r.sprites[1].tint);
        assert_eq!(r.chars[15].style.gradient, Some(0));
        assert_eq!(r.chars[18 - 1].style.gradient, Some(0));
        // Malformed tags stay as text.
        assert_eq!(text(&parse("<sprite>", true, 10.0)), "<sprite>");
        assert_eq!(
            text(&parse("<sprite bogus=1>", true, 10.0)),
            "<sprite bogus=1>"
        );
    }

    #[test]
    fn colours() {
        assert_eq!(parse_color("#f00"), Some(Color::rgb(1.0, 0.0, 0.0)));
        assert_eq!(
            parse_color("#00FF0080").map(|c| (c.g, (c.a * 255.0).round())),
            Some((1.0, 128.0))
        );
        assert_eq!(parse_color("\"red\""), Some(Color::rgb(1.0, 0.0, 0.0)));
        assert!(parse_color("#12345").is_none());
    }
}
