//! TextMeshPro's Font Asset Creator: picks the characters (sets, decimal / hex ranges,
//! typed characters or a text file), generates the atlas (optionally finding the largest
//! sampling size that fits one atlas), reports characters the font lacks or that didn't fit,
//! and turns the result into a Static Font Asset.
//! Reference: <https://docs.unity3d.com/Packages/com.unity.ugui@2.0/manual/TextMeshPro/FontAssetsCreator.html>

use crate::font::{AtlasPopulationMode, CharacterSet, Font, FontAsset, FontAtlas};
use oxide_assets::Handle;

/// The Creator's Character Set options.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CreatorCharacterSet {
    #[default]
    Ascii,
    ExtendedAscii,
    AsciiLowercase,
    AsciiUppercase,
    NumbersAndSymbols,
    /// Decimal code points and ranges: `32-126,160-255`.
    CustomRange,
    /// Hex code points and ranges: `20-7E,A0-FF`.
    UnicodeRangeHex,
    /// The characters typed in the sequence.
    CustomCharacters,
    /// Every character of a text file.
    CharactersFromFile,
}

impl CreatorCharacterSet {
    pub const ALL: [CreatorCharacterSet; 9] = [
        Self::Ascii,
        Self::ExtendedAscii,
        Self::AsciiLowercase,
        Self::AsciiUppercase,
        Self::NumbersAndSymbols,
        Self::CustomRange,
        Self::UnicodeRangeHex,
        Self::CustomCharacters,
        Self::CharactersFromFile,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Ascii => "ASCII",
            Self::ExtendedAscii => "Extended ASCII",
            Self::AsciiLowercase => "ASCII Lowercase",
            Self::AsciiUppercase => "ASCII Uppercase",
            Self::NumbersAndSymbols => "Numbers + Symbols",
            Self::CustomRange => "Custom Range",
            Self::UnicodeRangeHex => "Unicode Range (Hex)",
            Self::CustomCharacters => "Custom Characters",
            Self::CharactersFromFile => "Characters from File",
        }
    }
}

/// The Creator window's settings.
#[derive(Clone, Debug, PartialEq)]
pub struct CreatorSettings {
    /// `.ttf` / `.otf` asset path (`None` = the built-in font).
    pub source_font_file: Option<String>,
    /// Find the largest sampling size whose glyphs fit one atlas.
    pub auto_sizing: bool,
    pub sampling_point_size: u32,
    pub padding: u32,
    pub atlas_width: u32,
    pub atlas_height: u32,
    pub character_set: CreatorCharacterSet,
    /// Ranges or characters, for the Custom / Unicode / Custom Characters sets.
    pub character_sequence: String,
}

impl Default for CreatorSettings {
    fn default() -> Self {
        Self {
            source_font_file: None,
            auto_sizing: true,
            sampling_point_size: 64,
            padding: 8,
            atlas_width: 512,
            atlas_height: 512,
            character_set: CreatorCharacterSet::Ascii,
            character_sequence: String::new(),
        }
    }
}

fn parse_ranges(seq: &str, hex: bool) -> Result<Vec<char>, String> {
    let num = |s: &str| -> Result<u32, String> {
        let s = s.trim();
        let s = if hex {
            s.trim_start_matches("0x").trim_start_matches("U+")
        } else {
            s
        };
        u32::from_str_radix(s, if hex { 16 } else { 10 })
            .map_err(|_| format!("`{s}` is not a number"))
    };
    let mut out = Vec::new();
    for part in seq.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (a, b) = match part.split_once('-') {
            Some((a, b)) => (num(a)?, num(b)?),
            None => (num(part)?, num(part)?),
        };
        if b < a || b - a > 0x10000 {
            return Err(format!("bad range `{part}`"));
        }
        out.extend((a..=b).filter_map(char::from_u32));
    }
    Ok(out)
}

/// The characters `set` stands for (`file_text`: the Characters from File file).
pub fn characters(
    set: CreatorCharacterSet,
    sequence: &str,
    file_text: Option<&str>,
) -> Result<Vec<char>, String> {
    let mut v: Vec<char> = match set {
        CreatorCharacterSet::Ascii => (32u8..=126).map(char::from).collect(),
        CreatorCharacterSet::ExtendedAscii => {
            (32u8..=126).chain(160..=255).map(char::from).collect()
        }
        CreatorCharacterSet::AsciiLowercase => (b'a'..=b'z').chain(*b" ").map(char::from).collect(),
        CreatorCharacterSet::AsciiUppercase => (b'A'..=b'Z').chain(*b" ").map(char::from).collect(),
        CreatorCharacterSet::NumbersAndSymbols => (32u8..=64)
            .chain(91..=96)
            .chain(123..=126)
            .map(char::from)
            .collect(),
        CreatorCharacterSet::CustomRange => parse_ranges(sequence, false)?,
        CreatorCharacterSet::UnicodeRangeHex => parse_ranges(sequence, true)?,
        CreatorCharacterSet::CustomCharacters => sequence.chars().collect(),
        CreatorCharacterSet::CharactersFromFile => {
            file_text.ok_or("choose a text file")?.chars().collect()
        }
    };
    v.retain(|c| !c.is_control());
    v.sort_unstable();
    v.dedup();
    Ok(v)
}

/// A generated atlas and what went into it.
pub struct CreatorResult {
    pub atlas: FontAtlas,
    pub point_size: u32,
    /// Characters in the atlas.
    pub included: Vec<char>,
    /// Characters the font doesn't have.
    pub missing: Vec<char>,
    /// Characters the font has that didn't fit the atlas.
    pub excluded: Vec<char>,
}

impl CreatorResult {
    /// The Static Font Asset to save.
    pub fn font_asset(&self) -> FontAsset {
        let mut a = self.atlas.settings.clone();
        a.custom_characters = self.included.iter().collect();
        a
    }

    /// TMP's "Font Asset Creation Report".
    pub fn report(&self, font: &str) -> String {
        let list = |v: &[char]| -> String {
            v.iter()
                .map(|c| {
                    if c.is_whitespace() {
                        format!("U+{:04X}", *c as u32)
                    } else {
                        c.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        };
        let mut s = format!(
            "Font: {font}\nSampling Point Size: {}\nPadding: {}\nAtlas: {} x {}\nCharacters included: {}\n",
            self.point_size,
            self.atlas.settings.padding,
            self.atlas.settings.atlas_width,
            self.atlas.settings.atlas_height,
            self.included.len()
        );
        if !self.missing.is_empty() {
            s += &format!(
                "Characters missing from the font ({}): {}\n",
                self.missing.len(),
                list(&self.missing)
            );
        }
        if !self.excluded.is_empty() {
            s += &format!(
                "Characters that didn't fit the atlas ({}): {}\n",
                self.excluded.len(),
                list(&self.excluded)
            );
        }
        s
    }
}

fn build(
    settings: &CreatorSettings,
    size: u32,
    chars: &[char],
    bytes: Option<Vec<u8>>,
) -> Result<CreatorResult, String> {
    let asset = FontAsset {
        source_font_file: settings
            .source_font_file
            .clone()
            .map(Handle::<Font>::from_path),
        sampling_point_size: size,
        padding: settings.padding,
        atlas_width: settings.atlas_width,
        atlas_height: settings.atlas_height,
        atlas_population_mode: AtlasPopulationMode::Static,
        character_set: CharacterSet::CustomCharacters,
        custom_characters: chars.iter().collect(),
        multi_atlas_textures: false,
        ..Default::default()
    };
    let mut atlas = FontAtlas::new("font-asset-creator", asset, bytes)?;
    let (mut included, mut missing, mut excluded) = (Vec::new(), Vec::new(), Vec::new());
    for &c in chars {
        if !atlas.has_char(c) {
            missing.push(c);
        } else if c.is_whitespace() || atlas.glyph(c).is_some() {
            included.push(c);
        } else {
            excluded.push(c);
        }
    }
    Ok(CreatorResult {
        atlas,
        point_size: size,
        included,
        missing,
        excluded,
    })
}

/// Generates the atlas (Unity's "Generate Font Atlas"). With auto sizing, the largest
/// sampling size whose characters all fit.
pub fn generate(
    settings: &CreatorSettings,
    chars: &[char],
    font_bytes: Option<Vec<u8>>,
) -> Result<CreatorResult, String> {
    if chars.is_empty() {
        return Err("no characters to include".into());
    }
    if !settings.auto_sizing {
        return build(
            settings,
            settings.sampling_point_size.clamp(8, 512),
            chars,
            font_bytes,
        );
    }
    let fits = |r: &CreatorResult| r.excluded.is_empty();
    let (mut lo, mut hi) = (
        8u32,
        settings
            .atlas_width
            .min(settings.atlas_height)
            .clamp(8, 512),
    );
    let mut best = build(settings, lo, chars, font_bytes.clone())?;
    if !fits(&best) {
        return Ok(best);
    }
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        let r = build(settings, mid, chars, font_bytes.clone())?;
        if fits(&r) {
            lo = mid;
            best = r;
        } else {
            hi = mid;
        }
    }
    Ok(best)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn character_sets_and_ranges() {
        assert_eq!(
            characters(CreatorCharacterSet::Ascii, "", None)
                .unwrap()
                .len(),
            95
        );
        assert_eq!(
            characters(CreatorCharacterSet::ExtendedAscii, "", None)
                .unwrap()
                .len(),
            95 + 96
        );
        assert_eq!(
            characters(CreatorCharacterSet::AsciiUppercase, "", None)
                .unwrap()
                .len(),
            27
        );
        assert_eq!(
            characters(CreatorCharacterSet::CustomRange, "65-67, 97", None).unwrap(),
            ['A', 'B', 'C', 'a']
        );
        assert_eq!(
            characters(CreatorCharacterSet::UnicodeRangeHex, "41-43,0x20AC", None).unwrap(),
            ['A', 'B', 'C', '€']
        );
        assert_eq!(
            characters(CreatorCharacterSet::CustomCharacters, "hello", None).unwrap(),
            ['e', 'h', 'l', 'o']
        );
        assert_eq!(
            characters(CreatorCharacterSet::CharactersFromFile, "", Some("ba\nab")).unwrap(),
            ['a', 'b']
        );
        assert!(characters(CreatorCharacterSet::CustomRange, "70-60", None).is_err());
        assert!(characters(CreatorCharacterSet::CustomRange, "x", None).is_err());
    }

    #[test]
    fn generates_reports_and_auto_sizes() {
        let chars = characters(CreatorCharacterSet::CustomCharacters, "ABC\u{4E2D}", None).unwrap();
        let s = CreatorSettings {
            auto_sizing: false,
            sampling_point_size: 32,
            atlas_width: 256,
            atlas_height: 256,
            ..Default::default()
        };
        let r = generate(&s, &chars, None).unwrap();
        assert_eq!(r.included, ['A', 'B', 'C']);
        assert_eq!(r.missing, ['\u{4E2D}'], "the built-in font has no CJK");
        let a = r.font_asset();
        assert_eq!(a.atlas_population_mode, AtlasPopulationMode::Static);
        assert_eq!(a.custom_characters, "ABC");
        assert!(
            r.report("built-in")
                .contains("Characters missing from the font (1)")
        );

        // Too big for the atlas: some characters don't fit.
        let all = characters(CreatorCharacterSet::Ascii, "", None).unwrap();
        let big = CreatorSettings {
            sampling_point_size: 200,
            ..s.clone()
        };
        let r = generate(&big, &all, None).unwrap();
        assert!(!r.excluded.is_empty());
        // Auto sizing finds a size where everything fits, and a bit bigger doesn't.
        let auto = CreatorSettings {
            auto_sizing: true,
            padding: 4,
            atlas_width: 512,
            atlas_height: 512,
            ..s
        };
        let r = generate(&auto, &all, None).unwrap();
        assert!(
            r.excluded.is_empty() && r.included.len() == 95,
            "{}",
            r.report("x")
        );
        assert!(r.point_size > 30, "{}", r.point_size);
        let bigger = CreatorSettings {
            auto_sizing: false,
            sampling_point_size: r.point_size + 4,
            ..auto
        };
        assert!(!generate(&bigger, &all, None).unwrap().excluded.is_empty());
    }
}
