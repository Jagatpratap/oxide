//! USS: UI Toolkit style sheets (Unity's CSS subset).
//!
//! Rules are `selector, selector { property: value; }`. Selectors combine a type (`Button`,
//! `*`), `#name`, `.class` and pseudo-classes (`:hover`, `:active`, `:focus`, `:checked`,
//! `:disabled`, `:enabled`, `:root`) with descendant (space) and child (`>`) combinators.
//! `--variables` and `var()` work as in Unity.
//! Reference: <https://docs.unity3d.com/Manual/UIE-USS.html>

use std::fmt::Write;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Combinator {
    Descendant,
    Child,
}

/// One compound selector (`Button.big:hover`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Compound {
    /// Element type, or `None` for any (`*` or omitted).
    pub kind: Option<String>,
    pub name: Option<String>,
    pub classes: Vec<String>,
    pub pseudos: Vec<String>,
}

/// A complex selector: compounds from left to right with the combinator before each
/// (the first's is ignored).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selector {
    pub parts: Vec<(Combinator, Compound)>,
}

impl Selector {
    /// CSS specificity: (ids, classes and pseudo-classes, types).
    pub fn specificity(&self) -> (u32, u32, u32) {
        let mut s = (0, 0, 0);
        for (_, c) in &self.parts {
            s.0 += c.name.is_some() as u32;
            s.1 += (c.classes.len() + c.pseudos.len()) as u32;
            s.2 += c.kind.is_some() as u32;
        }
        s
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    /// The selector list as written (kept for editing).
    pub selector_text: String,
    pub selectors: Vec<Selector>,
    pub declarations: Vec<(String, String)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StyleSheet {
    pub rules: Vec<Rule>,
}

/// What a selector is matched against: one element.
pub trait Matchable {
    fn kind(&self) -> &str;
    fn name(&self) -> &str;
    fn has_class(&self, c: &str) -> bool;
    fn has_pseudo(&self, p: &str) -> bool;
}

fn compound_matches(c: &Compound, e: &dyn Matchable) -> bool {
    if let Some(k) = &c.kind
        && k != e.kind()
    {
        return false;
    }
    if let Some(n) = &c.name
        && n != e.name()
    {
        return false;
    }
    c.classes.iter().all(|x| e.has_class(x)) && c.pseudos.iter().all(|p| e.has_pseudo(p))
}

/// Does `sel` match the last element of `chain` (root first)?
pub fn matches(sel: &Selector, chain: &[&dyn Matchable]) -> bool {
    fn go(parts: &[(Combinator, Compound)], chain: &[&dyn Matchable]) -> bool {
        let Some(((comb, last), rest)) = parts.split_last() else {
            return true;
        };
        let Some((e, ancestors)) = chain.split_last() else {
            return false;
        };
        if !compound_matches(last, *e) {
            return false;
        }
        if rest.is_empty() {
            return true;
        }
        match comb {
            Combinator::Child => go(rest, ancestors),
            Combinator::Descendant => (0..ancestors.len())
                .rev()
                .any(|i| go(rest, &ancestors[..=i])),
        }
    }
    go(&sel.parts, chain)
}

fn strip_comments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("/*") {
        out.push_str(&rest[..i]);
        rest = match rest[i + 2..].find("*/") {
            Some(j) => &rest[i + 2 + j + 2..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

/// Parses one compound (`Button.a#b:hover`).
fn parse_compound(s: &str) -> Result<Compound, String> {
    let mut c = Compound::default();
    let mut chars = s.char_indices().peekable();
    let ident = |start: usize, s: &str| -> usize {
        s[start..]
            .find(|ch: char| !(ch.is_alphanumeric() || ch == '-' || ch == '_'))
            .map_or(s.len(), |k| start + k)
    };
    while let Some(&(i, ch)) = chars.peek() {
        match ch {
            '*' => {
                chars.next();
            }
            '.' | '#' | ':' => {
                let end = ident(i + 1, s);
                let v = s[i + 1..end].to_owned();
                if v.is_empty() {
                    return Err(format!("bad selector `{s}`"));
                }
                match ch {
                    '.' => c.classes.push(v),
                    '#' => c.name = Some(v),
                    _ => c.pseudos.push(v),
                }
                while chars.peek().is_some_and(|(k, _)| *k < end) {
                    chars.next();
                }
            }
            ch if ch.is_alphabetic() || ch == '_' => {
                let end = ident(i, s);
                c.kind = Some(s[i..end].to_owned());
                while chars.peek().is_some_and(|(k, _)| *k < end) {
                    chars.next();
                }
            }
            _ => return Err(format!("bad selector `{s}`")),
        }
    }
    Ok(c)
}

pub fn parse_selector(s: &str) -> Result<Selector, String> {
    let mut sel = Selector::default();
    let spaced = s.replace('>', " > ");
    let mut comb = Combinator::Descendant;
    for tok in spaced.split_whitespace() {
        if tok == ">" {
            comb = Combinator::Child;
            continue;
        }
        sel.parts.push((comb, parse_compound(tok)?));
        comb = Combinator::Descendant;
    }
    if sel.parts.is_empty() {
        return Err("empty selector".into());
    }
    Ok(sel)
}

/// Parses a style sheet. Bad rules are skipped and reported.
pub fn parse(text: &str) -> (StyleSheet, Vec<String>) {
    let text = strip_comments(text);
    let mut sheet = StyleSheet::default();
    let mut errors = Vec::new();
    let mut rest = text.as_str();
    while let Some(open) = rest.find('{') {
        let sel = rest[..open].trim().to_owned();
        let Some(close) = rest[open..].find('}') else {
            errors.push(format!("missing }} after `{sel}`"));
            break;
        };
        let body = &rest[open + 1..open + close];
        rest = &rest[open + close + 1..];
        // `@import` and other at-rules aren't supported.
        let sel = sel.rsplit(';').next().unwrap_or("").trim().to_owned();
        if sel.starts_with('@') {
            continue;
        }
        let mut selectors = Vec::new();
        for s in sel.split(',') {
            match parse_selector(s.trim()) {
                Ok(x) => selectors.push(x),
                Err(e) => errors.push(e),
            }
        }
        let declarations = crate::uxml::parse_inline_style(body);
        if !selectors.is_empty() {
            sheet.rules.push(Rule {
                selector_text: sel,
                selectors,
                declarations,
            });
        }
    }
    (sheet, errors)
}

/// Writes a style sheet back to text.
pub fn write(sheet: &StyleSheet) -> String {
    let mut out = String::new();
    for r in &sheet.rules {
        let _ = writeln!(out, "{} {{", r.selector_text);
        for (k, v) in &r.declarations {
            let _ = writeln!(out, "    {k}: {v};");
        }
        out.push_str("}\n\n");
    }
    out
}

/// Unity's runtime theme for the built-in controls (`unity-` classes), so documents look
/// like Unity's Default Runtime Theme without a theme file.
pub const DEFAULT_THEME: &str = r#"
:root {
    --unity-colors-default-text: #C4C4C4;
    --unity-colors-button-background: #585858;
    --unity-colors-button-background-hover: #676767;
    --unity-colors-button-background-pressed: #464646;
    --unity-colors-button-border: #303030;
    --unity-colors-input_field-background: #2A2A2A;
    --unity-colors-input_field-border: #212121;
    --unity-colors-highlight: #3A79BB;
    color: var(--unity-colors-default-text);
    font-size: 12px;
}
.unity-button {
    background-color: var(--unity-colors-button-background);
    border-width: 1px;
    border-color: var(--unity-colors-button-border);
    border-radius: 3px;
    padding: 1px 6px;
    margin: 1px 3px;
    min-height: 20px;
    -unity-text-align: middle-center;
}
.unity-button:hover {
    background-color: var(--unity-colors-button-background-hover);
}
.unity-button:active {
    background-color: var(--unity-colors-button-background-pressed);
}
.unity-label {
    padding: 2px;
    margin: 1px 3px;
}
.unity-text-field, .unity-dropdown-field, .unity-integer-field, .unity-float-field {
    margin: 1px 3px;
    min-height: 20px;
    flex-direction: row;
}
.unity-toggle, .unity-slider, .unity-progress-bar, .unity-radio-button {
    margin: 1px 3px;
    min-height: 20px;
    flex-direction: row;
}
.unity-foldout {
    margin: 1px 3px;
}
.unity-scroll-view {
    flex-grow: 1;
    overflow: hidden;
}
.unity-group-box {
    margin: 4px;
    padding: 4px;
}
.unity-disabled {
    opacity: 0.5;
}
"#;
