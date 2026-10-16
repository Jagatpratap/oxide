//! TextMeshPro Font Assets: a `.fontasset` file (settings, Unity `TMP_FontAsset`) made
//! from a `.ttf`/`.otf` font file, and its runtime atlas of SDF glyphs. In Dynamic mode
//! (TMP's default) glyphs are added to the atlas the first time text uses them; Static
//! assets hold only their character set. A full atlas gets another page (TMP "Multi Atlas
//! Textures"). Characters a font lacks come from its fallback font assets.
//! Reference: <https://docs.unity3d.com/Packages/com.unity.ugui@2.0/manual/FontAssets.html>

use crate::sdf;
use ab_glyph::{Font as _, FontArc, GlyphId, OutlineCurve};
use oxide_assets::{Asset, AssetServer, FilterMode, Handle, Texture2D, TextureWrapMode};
use oxide_core::Reflect;
use std::collections::HashMap;
use std::sync::Arc;

/// A font file (`.ttf`, `.otf`): Unity's Font asset, the source of Font Assets.
#[derive(Clone, Debug)]
pub struct Font;

impl Asset for Font {
    const TYPE_NAME: &'static str = "Font";
}

/// Path of the font used when a Font Asset has no source file (and of the built-in
/// default Font Asset): Ubuntu Light, bundled with the engine.
pub const BUILTIN_FONT: &str = "builtin:Ubuntu-Light.ttf";
/// The default Font Asset (TMP's "LiberationSans SDF").
pub const DEFAULT_FONT_ASSET: &str = "builtin:Default SDF.fontasset";

/// TMP `AtlasPopulationMode`.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AtlasPopulationMode {
    /// Only the character set, made when the asset loads.
    Static,
    /// Glyphs are added the first time text uses them.
    #[default]
    Dynamic,
}

/// Font Asset Creator "Character Set".
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CharacterSet {
    /// Printable ASCII (32-126).
    #[default]
    Ascii,
    /// ASCII plus Latin-1 (160-255).
    ExtendedAscii,
    /// Only `custom_characters`.
    CustomCharacters,
}

/// Unity `TMP_FontAsset` (`.fontasset`).
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct FontAsset {
    /// The `.ttf`/`.otf` file; empty uses the built-in font.
    pub source_font_file: Option<Handle<Font>>,
    /// Glyph size in the atlas (pixels per em): larger is sharper at big sizes.
    #[reflect(range(16, 256))]
    pub sampling_point_size: u32,
    /// Distance field spread around each glyph (pixels): larger allows thicker outlines.
    #[reflect(range(2, 32))]
    pub padding: u32,
    pub atlas_width: u32,
    pub atlas_height: u32,
    pub atlas_population_mode: AtlasPopulationMode,
    /// Glyphs made up front (Static) or as a start (Dynamic).
    pub character_set: CharacterSet,
    pub custom_characters: String,
    /// Adds atlas pages when one fills up (else missing glyphs show as boxes).
    pub multi_atlas_textures: bool,
    /// Searched in order for characters this font lacks.
    pub fallback_font_assets: Vec<Handle<FontAsset>>,
    /// Extra thickness of bold text (TMP "Bold Style", fraction of the padding).
    #[reflect(range(0, 1))]
    pub bold_style: f32,
    /// Extra spacing of bold text (em / 100).
    pub bold_spacing: f32,
    /// Slant of italic text (horizontal shear × 100).
    pub italic_style: f32,
    /// Tab stops every this many spaces.
    pub tab_multiple: u32,
}

impl Default for FontAsset {
    fn default() -> Self {
        Self {
            source_font_file: None,
            sampling_point_size: 64,
            padding: 8,
            atlas_width: 1024,
            atlas_height: 1024,
            atlas_population_mode: AtlasPopulationMode::Dynamic,
            character_set: CharacterSet::Ascii,
            custom_characters: String::new(),
            multi_atlas_textures: true,
            fallback_font_assets: Vec::new(),
            bold_style: 0.4,
            bold_spacing: 7.0,
            italic_style: 20.0,
            tab_multiple: 4,
        }
    }
}

impl Asset for FontAsset {
    const TYPE_NAME: &'static str = "FontAsset";
}

impl FontAsset {
    /// The characters of the character set.
    pub fn characters(&self) -> Vec<char> {
        match self.character_set {
            CharacterSet::Ascii => (32u8..=126).map(char::from).collect(),
            CharacterSet::ExtendedAscii => (32u8..=126).chain(160..=255).map(char::from).collect(),
            CharacterSet::CustomCharacters => {
                let mut v: Vec<char> = self.custom_characters.chars().collect();
                v.sort_unstable();
                v.dedup();
                v
            }
        }
    }
}

/// One glyph in the atlas, metrics in atlas pixels (the sampling size).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphInfo {
    pub page: u32,
    /// Atlas rectangle (x, y, width, height) including the padding.
    pub rect: [u32; 4],
    /// Bitmap top-left relative to the pen position on the baseline (+y up).
    pub offset: [f32; 2],
    pub advance: f32,
    pub glyph_id: u16,
}

/// Shelf packer for one atlas page.
#[derive(Clone, Debug, Default)]
struct Packer {
    x: u32,
    y: u32,
    row_h: u32,
}

impl Packer {
    fn place(&mut self, w: u32, h: u32, aw: u32, ah: u32) -> Option<(u32, u32)> {
        const GAP: u32 = 1;
        if self.x + w > aw {
            self.x = 0;
            self.y += self.row_h + GAP;
            self.row_h = 0;
        }
        if w > aw || self.y + h > ah {
            return None;
        }
        let at = (self.x, self.y);
        self.x += w + GAP;
        self.row_h = self.row_h.max(h);
        Some(at)
    }
}

/// One atlas texture: alpha holds the distance field, colour is white.
#[derive(Clone, Debug)]
pub struct AtlasPage {
    /// RGBA8, shared with draw callbacks (copied on write).
    pub pixels: Arc<Vec<u8>>,
    /// Bumped on every change.
    pub version: u64,
    packer: Packer,
    dirty: bool,
}

/// Size of the solid block at the top-left of page 0 (underlines, strikethroughs, marks).
pub const SOLID_SIZE: u32 = 4;

/// A Font Asset ready to lay out text: its font face, metrics and glyph atlas.
pub struct FontAtlas {
    pub settings: FontAsset,
    face: FontArc,
    /// Pixels per font unit at the sampling size.
    scale: f32,
    /// Ascender, descender (negative) and line height in atlas pixels.
    pub ascender: f32,
    pub descender: f32,
    pub line_height: f32,
    /// Underline position (below the baseline, negative) and thickness, atlas pixels.
    pub underline: (f32, f32),
    glyphs: HashMap<char, Option<GlyphInfo>>,
    pub pages: Vec<AtlasPage>,
    /// Asset path the atlas textures are named after.
    pub key: String,
    /// The data asset version this atlas was made from.
    pub source_version: u64,
}

impl FontAtlas {
    /// Makes an atlas for `settings` with the font file's bytes (`None` = built-in font).
    pub fn new(
        key: &str,
        settings: FontAsset,
        font_bytes: Option<Vec<u8>>,
    ) -> Result<Self, String> {
        let face = match font_bytes {
            Some(b) => FontArc::try_from_vec(b).map_err(|e| format!("bad font file: {e}"))?,
            None => FontArc::try_from_slice(epaint_default_fonts::UBUNTU_LIGHT)
                .map_err(|e| e.to_string())?,
        };
        let upem = face.units_per_em().unwrap_or(1000.0);
        let size = settings.sampling_point_size.clamp(8, 512) as f32;
        let scale = size / upem;
        let ascender = face.ascent_unscaled() * scale;
        let descender = face.descent_unscaled() * scale;
        let line_height =
            (face.ascent_unscaled() - face.descent_unscaled() + face.line_gap_unscaled()) * scale;
        let mut atlas = Self {
            underline: (-0.12 * size, 0.06 * size),
            settings,
            face,
            scale,
            ascender,
            descender,
            line_height,
            glyphs: HashMap::new(),
            pages: Vec::new(),
            key: key.to_owned(),
            source_version: 0,
        };
        atlas.add_page();
        // The solid block for underlines.
        let p = &mut atlas.pages[0];
        let at = p.packer.place(
            SOLID_SIZE,
            SOLID_SIZE,
            atlas.settings.atlas_width,
            atlas.settings.atlas_height,
        );
        debug_assert_eq!(at, Some((0, 0)));
        let aw = atlas.settings.atlas_width as usize;
        let px = Arc::make_mut(&mut p.pixels);
        for y in 0..SOLID_SIZE as usize {
            for x in 0..SOLID_SIZE as usize {
                px[(y * aw + x) * 4 + 3] = 255;
            }
        }
        for c in atlas.settings.characters() {
            atlas.glyph(c);
        }
        Ok(atlas)
    }

    fn add_page(&mut self) {
        let (w, h) = (
            self.settings.atlas_width.max(64),
            self.settings.atlas_height.max(64),
        );
        self.settings.atlas_width = w;
        self.settings.atlas_height = h;
        let mut pixels = vec![255u8; (w * h * 4) as usize];
        for a in pixels.iter_mut().skip(3).step_by(4) {
            *a = 0;
        }
        self.pages.push(AtlasPage {
            pixels: Arc::new(pixels),
            version: 1,
            packer: Packer::default(),
            dirty: true,
        });
    }

    pub fn has_char(&self, c: char) -> bool {
        self.face.glyph_id(c).0 != 0
    }

    /// Font-unit kerning between two characters, in atlas pixels.
    pub fn kerning(&self, a: char, b: char) -> f32 {
        self.face
            .kern_unscaled(self.face.glyph_id(a), self.face.glyph_id(b))
            * self.scale
    }

    /// The glyph of `c`, adding it to the atlas if needed (Dynamic mode). `None` when the
    /// font lacks it, or a Static atlas doesn't hold it.
    pub fn glyph(&mut self, c: char) -> Option<GlyphInfo> {
        if let Some(g) = self.glyphs.get(&c) {
            return *g;
        }
        if self.settings.atlas_population_mode == AtlasPopulationMode::Static
            && !self.settings.characters().contains(&c)
            && !self.glyphs.is_empty()
        {
            return None;
        }
        let g = self.rasterize(c);
        self.glyphs.insert(c, g);
        g
    }

    /// Number of characters in the atlas.
    pub fn glyph_count(&self) -> usize {
        self.glyphs.values().filter(|g| g.is_some()).count()
    }

    fn rasterize(&mut self, c: char) -> Option<GlyphInfo> {
        let id: GlyphId = self.face.glyph_id(c);
        if id.0 == 0 && c != '\0' {
            return None;
        }
        let advance = self.face.h_advance_unscaled(id) * self.scale;
        let pad = self.settings.padding.clamp(1, 64) as f32;
        let Some(outline) = self.face.outline(id) else {
            // Whitespace: an advance and no bitmap.
            return Some(GlyphInfo {
                page: 0,
                rect: [0, 0, 0, 0],
                offset: [0.0, 0.0],
                advance,
                glyph_id: id.0,
            });
        };
        let s = self.scale;
        // ab_glyph's bounds hold (x min, y max) in `min` and (x max, y min) in `max`.
        let b = outline.bounds;
        let (top, bottom) = (b.min.y.max(b.max.y) * s, b.min.y.min(b.max.y) * s);
        let min_x = b.min.x.min(b.max.x) * s;
        let max_x = b.min.x.max(b.max.x) * s;
        let ox = min_x.floor() - pad;
        let oy = top.ceil() + pad;
        let w = (max_x.ceil() - min_x.floor() + 2.0 * pad) as u32;
        let h = (oy - (bottom.floor() - pad)) as u32;
        // Font units (y up) → bitmap pixels (y down).
        let tp = |p: ab_glyph::Point| [p.x * s - ox, oy - p.y * s];
        let mut segs = sdf::Segments::default();
        for curve in &outline.curves {
            match *curve {
                OutlineCurve::Line(a, b) => segs.line(tp(a), tp(b)),
                OutlineCurve::Quad(a, c1, b) => segs.quad(tp(a), tp(c1), tp(b)),
                OutlineCurve::Cubic(a, c1, c2, b) => segs.cubic(tp(a), tp(c1), tp(c2), tp(b)),
            }
        }
        let bitmap = sdf::render(&segs.segs, w as usize, h as usize, pad);
        let (aw, ah) = (self.settings.atlas_width, self.settings.atlas_height);
        let mut page = self.pages.len() - 1;
        let at = match self.pages[page].packer.place(w, h, aw, ah) {
            Some(at) => at,
            None if self.settings.multi_atlas_textures => {
                self.add_page();
                page += 1;
                self.pages[page].packer.place(w, h, aw, ah)?
            }
            None => return None,
        };
        let p = &mut self.pages[page];
        let px = Arc::make_mut(&mut p.pixels);
        for y in 0..h as usize {
            for x in 0..w as usize {
                let i = ((at.1 as usize + y) * aw as usize + at.0 as usize + x) * 4;
                px[i + 3] = bitmap[y * w as usize + x];
            }
        }
        p.dirty = true;
        p.version += 1;
        Some(GlyphInfo {
            page: page as u32,
            rect: [at.0, at.1, w, h],
            offset: [ox, oy],
            advance,
            glyph_id: id.0,
        })
    }

    /// Path of page `i`'s texture in the AssetServer.
    pub fn texture_path(&self, page: u32) -> String {
        format!("{}#atlas{page}", self.key)
    }

    /// Writes changed pages into the AssetServer's textures (the GPU copies follow).
    pub fn upload(&mut self, assets: &mut AssetServer) {
        let (w, h) = (self.settings.atlas_width, self.settings.atlas_height);
        for i in 0..self.pages.len() {
            if !self.pages[i].dirty {
                continue;
            }
            self.pages[i].dirty = false;
            let path = self.texture_path(i as u32);
            assets.textures.insert(
                path.clone(),
                Texture2D {
                    name: path,
                    width: w,
                    height: h,
                    mips: vec![(*self.pages[i].pixels).clone()],
                    srgb: false,
                    wrap_mode: TextureWrapMode::Clamp,
                    filter_mode: FilterMode::Bilinear,
                    aniso_level: 0,
                    compressed: None,
                },
            );
        }
    }

    /// Width of a space in atlas pixels.
    pub fn space_advance(&self) -> f32 {
        let a = self.face.h_advance_unscaled(self.face.glyph_id(' ')) * self.scale;
        if a > 0.0 {
            a
        } else {
            self.sampling_size() * 0.25
        }
    }

    /// The SDF spread in atlas pixels.
    pub fn spread(&self) -> f32 {
        self.settings.padding.clamp(1, 64) as f32
    }

    pub fn sampling_size(&self) -> f32 {
        self.settings.sampling_point_size.clamp(8, 512) as f32
    }
}

/// Every loaded Font Asset's atlas (a World resource).
#[derive(Default)]
pub struct FontLibrary {
    atlases: HashMap<String, FontAtlas>,
    /// Paths that failed to load, with the reason (reported once).
    pub errors: HashMap<String, String>,
}

impl FontLibrary {
    /// Key of a Font Asset handle (`None` = the default Font Asset).
    pub fn key(font: Option<&Handle<FontAsset>>) -> String {
        font.map_or_else(|| DEFAULT_FONT_ASSET.to_owned(), |h| h.path().to_owned())
    }

    /// Loads (or reloads, when its file changed) the atlas of `font`. Falls back to the
    /// default Font Asset when it can't be loaded.
    pub fn load(&mut self, assets: &mut AssetServer, font: Option<&Handle<FontAsset>>) -> String {
        let key = Self::key(font);
        let (settings, version) = match font {
            None => (FontAsset::default(), 0),
            Some(h) => match assets.data_asset(h).cloned() {
                Some(s) => (s, assets.data.version(h.path()).unwrap_or(0)),
                None => {
                    self.errors
                        .entry(key)
                        .or_insert_with(|| "can't load the Font Asset".into());
                    return self.load(assets, None);
                }
            },
        };
        if self
            .atlases
            .get(&key)
            .is_some_and(|a| a.source_version == version)
        {
            return key;
        }
        let bytes = match &settings.source_font_file {
            None => None,
            Some(f) if f.path() == BUILTIN_FONT => None,
            Some(f) => match std::fs::read(assets.full_path(f.path())) {
                Ok(b) => Some(b),
                Err(e) => {
                    self.errors
                        .insert(key.clone(), format!("{}: {e}", f.path()));
                    None
                }
            },
        };
        match FontAtlas::new(&key, settings, bytes) {
            Ok(mut a) => {
                a.source_version = version;
                // Old pages beyond the new count would linger; replace them all.
                a.upload(assets);
                self.atlases.insert(key.clone(), a);
                key
            }
            Err(e) => {
                self.errors.insert(key, e);
                self.load(assets, None)
            }
        }
    }

    pub fn get(&self, key: &str) -> Option<&FontAtlas> {
        self.atlases.get(key)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut FontAtlas> {
        self.atlases.get_mut(key)
    }

    /// Drops an atlas so it is made again on next use (Inspector "Update Atlas").
    pub fn invalidate(&mut self, key: &str) {
        self.atlases.remove(key);
    }

    /// Uploads every changed atlas page.
    pub fn upload(&mut self, assets: &mut AssetServer) {
        for a in self.atlases.values_mut() {
            a.upload(assets);
        }
    }

    /// The chain `key` then its fallbacks (loaded), without repeats.
    pub fn chain(&mut self, assets: &mut AssetServer, key: &str) -> Vec<String> {
        let mut out = vec![key.to_owned()];
        let mut i = 0;
        while i < out.len() && out.len() < 8 {
            let fallbacks = self
                .atlases
                .get(&out[i])
                .map(|a| a.settings.fallback_font_assets.clone())
                .unwrap_or_default();
            for f in fallbacks {
                let k = self.load(assets, Some(&f));
                if !out.contains(&k) {
                    out.push(k);
                }
            }
            i += 1;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_atlas_has_ascii_and_adds_glyphs_on_demand() {
        let mut a = FontAtlas::new("test", FontAsset::default(), None).unwrap();
        assert_eq!(a.glyph_count(), 95, "printable ASCII up front");
        let g = a.glyph('A').unwrap();
        assert!(g.rect[2] > 16 && g.rect[3] > 16, "{g:?}");
        assert!(a.glyph(' ').unwrap().rect[2] == 0, "space has no bitmap");
        // Dynamic: Latin-1 added on first use.
        assert!(a.glyph('é').is_some());
        assert_eq!(a.glyph_count(), 96);
        // The glyph's bitmap has its outline (0.5) somewhere and a solid inside.
        let p = &a.pages[g.page as usize];
        let aw = a.settings.atlas_width as usize;
        let mut max = 0u8;
        for y in 0..g.rect[3] as usize {
            for x in 0..g.rect[2] as usize {
                max = max.max(
                    p.pixels[((g.rect[1] as usize + y) * aw + g.rect[0] as usize + x) * 4 + 3],
                );
            }
        }
        assert!(max > 160, "inside of 'A' is well inside: {max}");
        // A static atlas doesn't grow.
        let mut s = FontAtlas::new(
            "static",
            FontAsset {
                atlas_population_mode: AtlasPopulationMode::Static,
                ..Default::default()
            },
            None,
        )
        .unwrap();
        assert!(s.glyph('é').is_none());
    }

    #[test]
    fn full_pages_spill_into_new_ones() {
        let mut a = FontAtlas::new(
            "small",
            FontAsset {
                atlas_width: 128,
                atlas_height: 128,
                character_set: CharacterSet::CustomCharacters,
                ..Default::default()
            },
            None,
        )
        .unwrap();
        for c in 'A'..='Z' {
            a.glyph(c).unwrap();
        }
        assert!(a.pages.len() > 1, "26 glyphs don't fit in 128×128");
        let mut assets = AssetServer::new(std::env::temp_dir());
        a.upload(&mut assets);
        for i in 0..a.pages.len() as u32 {
            assert!(assets.textures.handle_of(&a.texture_path(i)).is_some());
        }
    }
}

#[cfg(test)]
#[test]
fn dump_atlas_for_inspection() {
    let Ok(out) = std::env::var("OXIDE_DUMP_ATLAS") else {
        return;
    };
    let a = FontAtlas::new("dump", FontAsset::default(), None).unwrap();
    let alpha: Vec<u8> = a.pages[0]
        .pixels
        .iter()
        .skip(3)
        .step_by(4)
        .copied()
        .collect();
    std::fs::write(out, alpha).unwrap();
}
