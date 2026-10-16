//! Text layout like TextMeshPro: styled characters become glyph quads, with word wrapping,
//! alignment, overflow modes, spacing options, kerning and fallback fonts.
//!
//! Coordinates are in the text's units with +y up. The layout rectangle spans x from 0 to
//! `width` and y from 0 (top) down to `-height`; a 0 width or height means unlimited, and
//! alignment then happens around 0.

use crate::font::{FontLibrary, GlyphInfo, SOLID_SIZE};
use crate::rich::{Case, Script, SizeSpec, Style, StyledChar};
use oxide_core::Reflect;
use oxide_math::Color;

/// TMP horizontal alignment.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HorizontalAlignment {
    #[default]
    Left,
    Center,
    Right,
    /// Stretches spaces so lines fill the width (not the last line of a paragraph).
    Justified,
    /// Like Justified, including last lines.
    Flush,
}

/// TMP vertical alignment.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VerticalAlignment {
    #[default]
    Top,
    Middle,
    Bottom,
}

/// TMP `TextOverflowModes`.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextOverflowModes {
    /// Text runs past the rectangle.
    #[default]
    Overflow,
    /// Cut at the rectangle, ending in "…".
    Ellipsis,
    /// Lines that don't fit are dropped.
    Truncate,
}

/// Inputs of one layout.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutSettings {
    /// Points.
    pub font_size: f32,
    /// Output units per point (1 for UI pixels, 0.1 for world text).
    pub unit_scale: f32,
    /// Rectangle in output units (0 = unlimited).
    pub width: f32,
    pub height: f32,
    pub wrap: bool,
    pub overflow: TextOverflowModes,
    pub horizontal: HorizontalAlignment,
    pub vertical: VerticalAlignment,
    /// em / 100, like TMP's spacing options.
    pub character_spacing: f32,
    pub word_spacing: f32,
    pub line_spacing: f32,
    pub paragraph_spacing: f32,
    /// Vertex colour (sRGB).
    pub color: Color,
    /// Base style (TMP Font Style buttons), under the rich text tags.
    pub style: Style,
}

impl Default for LayoutSettings {
    fn default() -> Self {
        Self {
            font_size: 36.0,
            unit_scale: 1.0,
            width: 0.0,
            height: 0.0,
            wrap: true,
            overflow: TextOverflowModes::Overflow,
            horizontal: HorizontalAlignment::Left,
            vertical: VerticalAlignment::Top,
            character_spacing: 0.0,
            word_spacing: 0.0,
            line_spacing: 0.0,
            paragraph_spacing: 0.0,
            color: Color::WHITE,
            style: Style::default(),
        }
    }
}

/// One textured quad: corners bottom-left, bottom-right, top-right, top-left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphQuad {
    pub corners: [[f32; 2]; 4],
    /// Normalized atlas rectangle (min x, min y, max x, max y; y down).
    pub uv: [f32; 4],
    /// Index into [`TextMesh::fonts`].
    pub font: u32,
    pub page: u32,
    /// sRGB vertex colour.
    pub color: Color,
    pub bold: bool,
}

/// Laid out text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextMesh {
    pub quads: Vec<GlyphQuad>,
    /// Font Asset keys the quads' `font` index into.
    pub fonts: Vec<String>,
    /// Extent of the text (min x, min y, max x, max y).
    pub bounds: [f32; 4],
    pub line_count: usize,
    /// The text didn't fit its rectangle.
    pub overflowed: bool,
    /// Size used (after auto size), points.
    pub font_size: f32,
    /// Characters drawn (TMP `characterCount`).
    pub character_count: usize,
}

#[derive(Clone, Copy, Debug)]
struct Item {
    c: char,
    style: Style,
    /// Points.
    size: f32,
    font: usize,
    glyph: Option<GlyphInfo>,
    /// Points per atlas pixel.
    k: f32,
    advance: f32,
    /// Baseline shift (sub/superscript), points.
    rise: f32,
    ascender: f32,
    descender: f32,
    line_height: f32,
}

fn apply_case(c: char, case: Case) -> (char, f32) {
    match case {
        Case::Normal => (c, 1.0),
        Case::Upper => (c.to_uppercase().next().unwrap_or(c), 1.0),
        Case::Lower => (c.to_lowercase().next().unwrap_or(c), 1.0),
        Case::SmallCaps if c.is_lowercase() => (c.to_uppercase().next().unwrap_or(c), 0.8),
        Case::SmallCaps => (c, 1.0),
    }
}

/// Lays out `chars` with the fonts in `chain` (main font first, then fallbacks).
pub fn layout(
    chars: &[StyledChar],
    lib: &mut FontLibrary,
    chain: &[String],
    s: &LayoutSettings,
) -> TextMesh {
    let base = s.font_size.max(0.0);
    let Some(main) = chain.first().and_then(|k| lib.get(k)) else {
        return TextMesh::default();
    };
    let (main_settings, main_size) = (main.settings.clone(), main.sampling_size());
    let em = |v: f32, size: f32| v / 100.0 * size;

    // ---- resolve glyphs and advances --------------------------------------------------
    let mut items: Vec<Item> = Vec::with_capacity(chars.len());
    let mut prev: Option<(char, usize)> = None;
    for sc in chars {
        let mut st = sc.style;
        let b = s.style;
        st.bold |= b.bold;
        st.italic |= b.italic;
        st.underline |= b.underline;
        st.strikethrough |= b.strikethrough;
        if st.case == Case::Normal {
            st.case = b.case;
        }
        let (c, case_scale) = apply_case(sc.c, st.case);
        let mut size = match st.size {
            Some(SizeSpec::Absolute(a)) => a,
            Some(SizeSpec::Scale(k)) => base * k,
            None => base,
        } * case_scale;
        let rise = match st.script {
            Script::Normal => 0.0,
            Script::Sub => -0.15 * size,
            Script::Sup => 0.4 * size,
        };
        if st.script != Script::Normal {
            size *= 0.6;
        }
        if c == '\n' {
            let a = lib.get(&chain[0]).expect("main font");
            let k = size / a.sampling_size();
            items.push(Item {
                c,
                style: st,
                size,
                font: 0,
                glyph: None,
                k,
                advance: 0.0,
                rise: 0.0,
                ascender: a.ascender * k,
                descender: a.descender * k,
                line_height: a.line_height * k,
            });
            prev = None;
            continue;
        }
        // The first font of the chain that has the character; else a box, else '?'.
        let mut found = None;
        for want in [c, '\u{25A1}', '?'] {
            for (fi, key) in chain.iter().enumerate() {
                let Some(a) = lib.get_mut(key) else { continue };
                if want != ' ' && want != '\t' && !a.has_char(want) {
                    continue;
                }
                if let Some(g) = a.glyph(if want == '\t' { ' ' } else { want }) {
                    found = Some((fi, g));
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }
        let (font, glyph) = match found {
            Some((f, g)) => (f, Some(g)),
            None => (0, None),
        };
        let a = lib.get(&chain[font]).expect("loaded");
        let k = size / a.sampling_size();
        let mut advance =
            glyph.map_or(0.0, |g| g.advance * k) + em(s.character_spacing, size) + st.cspace * size;
        if st.bold {
            advance += em(a.settings.bold_spacing, size);
        }
        if c == ' ' {
            advance += em(s.word_spacing, size);
        }
        if c == '\t' {
            let space = a.space_advance() * k;
            advance = space * a.settings.tab_multiple.max(1) as f32;
        }
        if let Some((pc, pf)) = prev
            && pf == font
            && let Some(last) = items.last_mut()
        {
            last.advance += a.kerning(pc, c) * last.k;
        }
        prev = Some((c, font));
        items.push(Item {
            c,
            style: st,
            size,
            font,
            glyph: if c == '\t' { None } else { glyph },
            k,
            advance,
            rise,
            ascender: a.ascender * k,
            descender: a.descender * k,
            line_height: a.line_height * k,
        });
    }

    // ---- line breaking ------------------------------------------------------------------
    let u = s.unit_scale.max(1e-6);
    let width = s.width / u; // points
    let height = s.height / u;
    let mut lines: Vec<std::ops::Range<usize>> = Vec::new();
    let mut start = 0;
    let mut x = 0.0f32;
    let mut last_space: Option<usize> = None;
    let mut i = 0;
    while i < items.len() {
        let it = items[i];
        if it.c == '\n' {
            lines.push(start..i + 1);
            start = i + 1;
            x = 0.0;
            last_space = None;
            i += 1;
            continue;
        }
        let right = x + it.advance;
        if s.wrap && width > 0.0 && right > width + 1e-3 && i > start && !it.c.is_whitespace() {
            match last_space {
                Some(sp) if sp >= start => {
                    lines.push(start..sp + 1);
                    start = sp + 1;
                }
                _ => {
                    lines.push(start..i);
                    start = i;
                }
            }
            x = items[start..i].iter().map(|it| it.advance).sum();
            last_space = None;
            continue;
        }
        if it.c.is_whitespace() {
            last_space = Some(i);
        }
        x = right;
        i += 1;
    }
    if start < items.len() || lines.is_empty() {
        lines.push(start..items.len());
    }

    // ---- line metrics -------------------------------------------------------------------
    let base_k = base / main_size;
    let main_atlas = lib.get(&chain[0]).expect("main");
    let (base_asc, base_desc, base_lh) = (
        main_atlas.ascender * base_k,
        main_atlas.descender * base_k,
        main_atlas.line_height * base_k,
    );
    struct Line {
        range: std::ops::Range<usize>,
        baseline: f32,
        descender: f32,
        paragraph_end: bool,
    }
    let mut out_lines: Vec<Line> = Vec::new();
    let mut baseline = 0.0;
    let mut prev_desc = 0.0;
    for (li, r) in lines.iter().enumerate() {
        let visible = items[r.clone()].iter().filter(|it| it.c != '\n');
        let (mut asc, mut desc, mut lh) = (0.0f32, 0.0f32, 0.0f32);
        let mut any = false;
        for it in visible {
            asc = asc.max(it.ascender + it.rise.max(0.0));
            desc = desc.min(it.descender + it.rise.min(0.0));
            lh = lh.max(it.line_height);
            any = true;
        }
        if !any {
            (asc, desc, lh) = (base_asc, base_desc, base_lh);
        }
        let gap = (lh - (asc - desc)).max(0.0);
        baseline = if li == 0 {
            -asc
        } else {
            let para = out_lines.last().is_some_and(|l| l.paragraph_end);
            baseline + prev_desc
                - asc
                - gap
                - em(s.line_spacing, base)
                - if para {
                    em(s.paragraph_spacing, base)
                } else {
                    0.0
                }
        };
        prev_desc = desc;
        let paragraph_end = r.end > r.start && items[r.end - 1].c == '\n';
        out_lines.push(Line {
            range: r.clone(),
            baseline,
            descender: desc,
            paragraph_end,
        });
    }

    // ---- overflow -------------------------------------------------------------------------
    let mut overflowed = false;
    let fits = |l: &Line| height <= 0.0 || -(l.baseline + l.descender) <= height + 1e-3;
    if out_lines.iter().any(|l| !fits(l)) {
        overflowed = true;
        if s.overflow != TextOverflowModes::Overflow {
            let keep = out_lines.iter().take_while(|l| fits(l)).count().max(1);
            out_lines.truncate(keep);
            if s.overflow == TextOverflowModes::Ellipsis {
                let last = out_lines.last_mut().expect("kept one");
                last.range.end = last.range.end.min(items.len());
                ellipsis(&mut items, lib, chain, &mut last.range, width);
            }
        }
    }
    if !s.wrap && width > 0.0 && s.overflow != TextOverflowModes::Overflow {
        for l in &mut out_lines {
            let w: f32 = items[l.range.clone()].iter().map(|it| it.advance).sum();
            if w > width + 1e-3 {
                overflowed = true;
                if s.overflow == TextOverflowModes::Ellipsis {
                    ellipsis(&mut items, lib, chain, &mut l.range, width);
                } else {
                    let mut acc = 0.0;
                    let mut end = l.range.start;
                    while end < l.range.end && acc + items[end].advance <= width + 1e-3 {
                        acc += items[end].advance;
                        end += 1;
                    }
                    l.range.end = end;
                }
            }
        }
    } else if !s.wrap && width > 0.0 {
        overflowed |= out_lines.iter().any(|l| {
            items[l.range.clone()]
                .iter()
                .map(|it| it.advance)
                .sum::<f32>()
                > width + 1e-3
        });
    }

    // ---- placement -------------------------------------------------------------------------
    let total = out_lines
        .last()
        .map_or(0.0, |l| -(l.baseline + l.descender));
    let g = match s.vertical {
        VerticalAlignment::Top => 0.0,
        VerticalAlignment::Middle => 0.5,
        VerticalAlignment::Bottom => 1.0,
    };
    let top = -(height - total) * g;
    let shear = main_settings.italic_style / 100.0;
    let mut marks = Vec::new();
    let mut glyphs = Vec::new();
    let mut decor = Vec::new();
    let mut count = 0;
    let solid_uv = |font: usize, lib: &FontLibrary| {
        let a = lib.get(&chain[font]).expect("loaded");
        let (aw, ah) = (
            a.settings.atlas_width as f32,
            a.settings.atlas_height as f32,
        );
        let h = SOLID_SIZE as f32 * 0.5;
        [
            (h - 0.5) / aw,
            (h - 0.5) / ah,
            (h + 0.5) / aw,
            (h + 0.5) / ah,
        ]
    };
    let rect = |x0: f32, y0: f32, x1: f32, y1: f32| [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
    for (li, l) in out_lines.iter().enumerate() {
        let line = &items[l.range.clone()];
        // Width without trailing whitespace.
        let trimmed = line
            .iter()
            .rposition(|it| !it.c.is_whitespace())
            .map_or(0, |p| p + 1);
        let lw: f32 = line[..trimmed].iter().map(|it| it.advance).sum();
        let last_of_para = l.paragraph_end || li + 1 == out_lines.len();
        let justify = width > 0.0
            && match s.horizontal {
                HorizontalAlignment::Justified => !last_of_para,
                HorizontalAlignment::Flush => true,
                _ => false,
            };
        let spaces = line[..trimmed].iter().filter(|it| it.c == ' ').count();
        let extra = if justify && spaces > 0 {
            (width - lw).max(0.0) / spaces as f32
        } else {
            0.0
        };
        let f = match s.horizontal {
            HorizontalAlignment::Left
            | HorizontalAlignment::Justified
            | HorizontalAlignment::Flush => 0.0,
            HorizontalAlignment::Center => 0.5,
            HorizontalAlignment::Right => 1.0,
        };
        let mut x = (width - lw) * f;
        let base_y = top + l.baseline;
        // Runs of underline / strikethrough: (start x, style item).
        let mut under: Option<(f32, Item)> = None;
        let mut strike: Option<(f32, Item)> = None;
        let flush_run =
            |run: &mut Option<(f32, Item)>, end: f32, decor: &mut Vec<GlyphQuad>, under: bool| {
                if let Some((x0, it)) = run.take() {
                    let a = lib.get(&chain[it.font]).expect("loaded");
                    let (pos, thick) = (a.underline.0 * it.k, a.underline.1 * it.k);
                    let y = if under {
                        base_y + it.rise + pos
                    } else {
                        base_y + it.rise + a.ascender * it.k * 0.3
                    };
                    decor.push(GlyphQuad {
                        corners: rect(x0, y - thick * 0.5, end, y + thick * 0.5),
                        uv: solid_uv(it.font, lib),
                        font: it.font as u32,
                        page: 0,
                        color: item_color(&it.style, s.color),
                        bold: false,
                    });
                }
            };
        for (j, it) in line.iter().enumerate() {
            if it.c == '\n' {
                continue;
            }
            let adv = it.advance
                + if it.c == ' ' && j < trimmed {
                    extra
                } else {
                    0.0
                };
            let color = item_color(&it.style, s.color);
            if let Some(m) = it.style.mark {
                marks.push(GlyphQuad {
                    corners: rect(x, base_y + it.descender, x + adv, base_y + it.ascender),
                    uv: solid_uv(it.font, lib),
                    font: it.font as u32,
                    page: 0,
                    color: m,
                    bold: false,
                });
            }
            for (run, on, is_under) in [
                (&mut under, it.style.underline, true),
                (&mut strike, it.style.strikethrough, false),
            ] {
                let same = run.as_ref().is_some_and(|(_, r)| {
                    r.size == it.size && r.style.color == it.style.color && r.font == it.font
                });
                if run.is_some() && (!on || !same || j >= trimmed) {
                    flush_run(run, x, &mut decor, is_under);
                }
                if on && run.is_none() && j < trimmed {
                    *run = Some((x, *it));
                }
            }
            if let Some(gl) = it.glyph
                && gl.rect[2] > 0
                && !it.c.is_whitespace()
            {
                let a = lib.get(&chain[it.font]).expect("loaded");
                let (aw, ah) = (
                    a.settings.atlas_width as f32,
                    a.settings.atlas_height as f32,
                );
                let x0 = x + gl.offset[0] * it.k;
                let y1 = base_y + it.rise + gl.offset[1] * it.k;
                let x1 = x0 + gl.rect[2] as f32 * it.k;
                let y0 = y1 - gl.rect[3] as f32 * it.k;
                let mut corners = rect(x0, y0, x1, y1);
                if it.style.italic {
                    let b = base_y + it.rise;
                    for c in &mut corners {
                        c[0] += shear * (c[1] - b);
                    }
                }
                glyphs.push(GlyphQuad {
                    corners,
                    uv: [
                        gl.rect[0] as f32 / aw,
                        gl.rect[1] as f32 / ah,
                        (gl.rect[0] + gl.rect[2]) as f32 / aw,
                        (gl.rect[1] + gl.rect[3]) as f32 / ah,
                    ],
                    font: it.font as u32,
                    page: gl.page,
                    color,
                    bold: it.style.bold,
                });
                count += 1;
            } else if !it.c.is_whitespace() {
                count += 1;
            }
            x += adv;
        }
        flush_run(&mut under, x, &mut decor, true);
        flush_run(&mut strike, x, &mut decor, false);
    }
    let mut quads = marks;
    quads.extend(glyphs);
    quads.extend(decor);
    // Points → output units.
    for q in &mut quads {
        for c in &mut q.corners {
            c[0] *= u;
            c[1] *= u;
        }
    }
    let mut bounds = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for q in &quads {
        for c in q.corners {
            bounds[0] = bounds[0].min(c[0]);
            bounds[1] = bounds[1].min(c[1]);
            bounds[2] = bounds[2].max(c[0]);
            bounds[3] = bounds[3].max(c[1]);
        }
    }
    if quads.is_empty() {
        bounds = [0.0; 4];
    }
    TextMesh {
        quads,
        fonts: chain.to_vec(),
        bounds,
        line_count: out_lines.len(),
        overflowed,
        font_size: base,
        character_count: count,
    }
}

fn item_color(st: &Style, base: Color) -> Color {
    let mut c = base;
    if let Some(t) = st.color {
        c = Color::new(c.r * t.r, c.g * t.g, c.b * t.b, c.a * t.a);
    }
    if let Some(a) = st.alpha {
        c.a = a;
    }
    c
}

/// Ends `range` with "…" so it fits `width` (points; 0 = just append).
fn ellipsis(
    items: &mut Vec<Item>,
    lib: &mut FontLibrary,
    chain: &[String],
    range: &mut std::ops::Range<usize>,
    width: f32,
) {
    let Some(last) = items[range.clone()]
        .iter()
        .rev()
        .find(|it| it.c != '\n')
        .copied()
    else {
        return;
    };
    let (font, glyph) = match chain.iter().enumerate().find_map(|(fi, k)| {
        let a = lib.get_mut(k)?;
        a.has_char('…')
            .then(|| a.glyph('…'))
            .flatten()
            .map(|g| (fi, g))
    }) {
        Some(v) => v,
        None => return,
    };
    let a = lib.get(&chain[font]).expect("loaded");
    let k = last.size / a.sampling_size();
    let dots = Item {
        c: '…',
        font,
        glyph: Some(glyph),
        k,
        advance: glyph.advance * k,
        ..last
    };
    // Drop characters (and trailing spaces) until the dots fit.
    let mut end = range.end;
    while end > range.start && items[end - 1].c == '\n' {
        end -= 1;
    }
    loop {
        let w: f32 = items[range.start..end].iter().map(|it| it.advance).sum();
        let trailing_space = end > range.start && items[end - 1].c.is_whitespace();
        if (width <= 0.0 || w + dots.advance <= width + 1e-3 || end == range.start)
            && !trailing_space
        {
            break;
        }
        end -= 1;
    }
    items.insert(end, dots);
    range.end = end + 1;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::{FontAsset, FontLibrary};
    use crate::rich::parse;
    use oxide_assets::AssetServer;

    fn lib() -> (FontLibrary, Vec<String>) {
        let mut assets = AssetServer::new(std::env::temp_dir());
        let mut lib = FontLibrary::default();
        let key = lib.load(&mut assets, None);
        let _ = FontAsset::default();
        (lib, vec![key])
    }

    fn lay(text: &str, s: &LayoutSettings) -> TextMesh {
        let (mut lib, chain) = lib();
        layout(&parse(text, true, s.font_size), &mut lib, &chain, s)
    }

    #[test]
    fn single_line_sizes_with_the_font() {
        let s = LayoutSettings {
            font_size: 20.0,
            ..Default::default()
        };
        let a = lay("Hello", &s);
        let b = lay(
            "Hello",
            &LayoutSettings {
                font_size: 40.0,
                ..s.clone()
            },
        );
        assert_eq!(a.quads.len(), 5);
        assert_eq!(a.line_count, 1);
        let w = |m: &TextMesh| m.bounds[2] - m.bounds[0];
        assert!(
            (w(&b) / w(&a) - 2.0).abs() < 0.05,
            "twice the size, twice as wide"
        );
        // Top alignment: glyphs sit below 0.
        assert!(a.bounds[3] <= 0.01, "{:?}", a.bounds);
    }

    #[test]
    fn wrapping_breaks_at_spaces_and_alignment_moves_lines() {
        let s = LayoutSettings {
            font_size: 20.0,
            width: 120.0,
            ..Default::default()
        };
        let m = lay("one two three four five six", &s);
        assert!(m.line_count >= 3, "{}", m.line_count);
        assert!(
            m.bounds[2] <= 120.0 + 0.5,
            "nothing past the width: {:?}",
            m.bounds
        );
        let right = lay(
            "one",
            &LayoutSettings {
                horizontal: HorizontalAlignment::Right,
                ..s.clone()
            },
        );
        assert!(
            (right.bounds[2] - 120.0).abs() < 3.0,
            "right aligned: {:?}",
            right.bounds
        );
        let center = lay(
            "one",
            &LayoutSettings {
                horizontal: HorizontalAlignment::Center,
                ..s.clone()
            },
        );
        let mid = (center.bounds[0] + center.bounds[2]) / 2.0;
        assert!((mid - 60.0).abs() < 3.0, "centred: {mid}");
        // A word longer than the line breaks inside the word.
        let long = lay("abcdefghijklmnopqrstuvwxyz", &s);
        assert!(long.line_count >= 2);
    }

    #[test]
    fn overflow_modes() {
        let s = LayoutSettings {
            font_size: 20.0,
            width: 100.0,
            height: 30.0,
            ..Default::default()
        };
        let text = "aaa bbb ccc ddd eee fff ggg";
        let over = lay(text, &s);
        assert!(over.overflowed && over.line_count > 1);
        let trunc = lay(
            text,
            &LayoutSettings {
                overflow: TextOverflowModes::Truncate,
                ..s.clone()
            },
        );
        assert_eq!(trunc.line_count, 1);
        assert!(trunc.bounds[1] >= -30.5, "{:?}", trunc.bounds);
        let ell = lay(
            text,
            &LayoutSettings {
                overflow: TextOverflowModes::Ellipsis,
                ..s.clone()
            },
        );
        assert_eq!(ell.line_count, 1);
        assert!(ell.bounds[2] <= 100.5, "{:?}", ell.bounds);
        assert!(ell.quads.len() < over.quads.len());
    }

    #[test]
    fn rich_text_makes_marks_underlines_and_colours() {
        let s = LayoutSettings::default();
        let plain = lay("ab", &s);
        let m = lay(
            "<u>ab</u> <mark=#ffff0080>c</mark> <color=red>d</color>",
            &s,
        );
        // 4 glyphs + 1 underline + 1 mark.
        assert_eq!(m.quads.len(), 6);
        assert_eq!(
            m.quads[0].color.a,
            128.0 / 255.0,
            "mark first, behind the text"
        );
        let red = m
            .quads
            .iter()
            .find(|q| q.color == Color::rgb(1.0, 0.0, 0.0));
        assert!(red.is_some());
        assert_eq!(plain.quads.len(), 2);
        // Italic shears the top of the glyph to the right.
        let it = lay("<i>l</i>", &s);
        let c = it.quads[0].corners;
        assert!(c[3][0] > c[0][0] + 1.0, "{c:?}");
    }

    #[test]
    fn newlines_and_spacing() {
        let s = LayoutSettings::default();
        let one = lay("a", &s);
        let two = lay("a\nb", &s);
        assert_eq!(two.line_count, 2);
        let h = |m: &TextMesh| m.bounds[3] - m.bounds[1];
        assert!(h(&two) > h(&one) * 1.5);
        let spaced = lay(
            "a\nb",
            &LayoutSettings {
                line_spacing: 100.0,
                ..s.clone()
            },
        );
        assert!(h(&spaced) > h(&two) + 30.0, "line spacing 100 adds an em");
        let wide = lay(
            "ab",
            &LayoutSettings {
                character_spacing: 50.0,
                ..s.clone()
            },
        );
        let w = |m: &TextMesh| m.bounds[2] - m.bounds[0];
        assert!(w(&wide) > w(&lay("ab", &s)) + 10.0);
    }
}
