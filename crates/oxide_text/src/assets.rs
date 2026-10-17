//! TextMeshPro's other assets: Sprite Assets (images inline in text with `<sprite>`), Color
//! Gradient presets (per-character four-corner vertex colours, also `<gradient>`) and
//! Material presets (shared outline / underlay settings).
//! Reference: <https://docs.unity3d.com/Packages/com.unity.ugui@2.0/manual/TextMeshPro/Sprites.html>

use crate::components::TextMaterial;
use crate::rich::SpriteTag;
use oxide_assets::{Asset, AssetServer, Handle, Texture2D};
use oxide_core::Reflect;
use oxide_math::{Color, Vector2, Vector4};

/// One sprite of a Sprite Asset (TMP's sprite glyph + character).
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct TmpSprite {
    pub name: String,
    /// The character `<sprite>`-free text can use for it (0 = none).
    pub unicode: u32,
    /// x, y, width, height in texture pixels (origin bottom left, like sprite slices).
    pub rect: Vector4,
    /// Horizontal offset and top edge above the baseline, in sprite pixels.
    pub bearing: Vector2,
    /// Horizontal advance, sprite pixels.
    pub advance: f32,
    /// Size multiplier.
    pub scale: f32,
}

impl Default for TmpSprite {
    fn default() -> Self {
        Self {
            name: "Sprite".into(),
            unicode: 0,
            rect: Vector4::new(0.0, 0.0, 32.0, 32.0),
            bearing: Vector2::new(0.0, 32.0),
            advance: 32.0,
            scale: 1.0,
        }
    }
}

/// TMP Sprite Asset (`.spriteasset`): a texture and the sprites cut from it.
#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct SpriteAsset {
    pub texture: Option<Handle<Texture2D>>,
    pub sprites: Vec<TmpSprite>,
    /// Searched when a sprite isn't found here.
    pub fallback_sprite_assets: Vec<Handle<SpriteAsset>>,
}

impl Asset for SpriteAsset {
    const TYPE_NAME: &'static str = "SpriteAsset";
}

impl SpriteAsset {
    /// One sprite per slice of a sprite sheet (TMP's "Create > Sprite Asset" on a texture
    /// sliced in the Sprite Editor).
    pub fn from_slices(
        texture: Handle<Texture2D>,
        slices: &[oxide_assets::sprite::SpriteRect],
    ) -> Self {
        Self {
            texture: Some(texture),
            sprites: slices
                .iter()
                .map(|s| TmpSprite {
                    name: s.name.clone(),
                    unicode: 0,
                    rect: s.rect,
                    bearing: Vector2::new(0.0, s.rect.w),
                    advance: s.rect.z,
                    scale: 1.0,
                })
                .collect(),
            fallback_sprite_assets: Vec::new(),
        }
    }

    /// `columns` × `rows` equal cells of a `width` × `height` texture, left to right, top
    /// to bottom, named `name_0`, `name_1`, ...
    pub fn from_grid(
        texture: Handle<Texture2D>,
        width: u32,
        height: u32,
        columns: u32,
        rows: u32,
        name: &str,
    ) -> Self {
        let (cw, ch) = (
            width as f32 / columns.max(1) as f32,
            height as f32 / rows.max(1) as f32,
        );
        let slices: Vec<oxide_assets::sprite::SpriteRect> = (0..rows.max(1))
            .flat_map(|r| (0..columns.max(1)).map(move |c| (r, c)))
            .enumerate()
            .map(|(i, (r, c))| oxide_assets::sprite::SpriteRect {
                name: format!("{name}_{i}"),
                rect: Vector4::new(c as f32 * cw, height as f32 - (r + 1) as f32 * ch, cw, ch),
                ..Default::default()
            })
            .collect();
        Self::from_slices(texture, &slices)
    }

    /// The sprite a tag (or a character) asks for.
    pub fn find(&self, tag: &SpriteTag) -> Option<usize> {
        if let Some(n) = &tag.name {
            return self.sprites.iter().position(|s| &s.name == n);
        }
        let i = tag.index? as usize;
        (i < self.sprites.len()).then_some(i)
    }

    pub fn find_unicode(&self, c: char) -> Option<usize> {
        self.sprites.iter().position(|s| s.unicode == c as u32)
    }
}

/// How a Color Gradient's four colours apply.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorMode {
    /// Top left only.
    Single,
    /// Top left → top right.
    HorizontalGradient,
    /// Top left → bottom left.
    VerticalGradient,
    #[default]
    FourCornersGradient,
}

/// TMP Color Gradient (`.colorgradient` preset, or inline on a text): each character's
/// corners get these colours (multiplied with the vertex colour).
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct ColorGradient {
    pub color_mode: ColorMode,
    pub top_left: Color,
    pub top_right: Color,
    pub bottom_left: Color,
    pub bottom_right: Color,
}

impl Default for ColorGradient {
    fn default() -> Self {
        Self {
            color_mode: ColorMode::FourCornersGradient,
            top_left: Color::WHITE,
            top_right: Color::WHITE,
            bottom_left: Color::WHITE,
            bottom_right: Color::WHITE,
        }
    }
}

impl Asset for ColorGradient {
    const TYPE_NAME: &'static str = "ColorGradient";
}

impl ColorGradient {
    /// Corner colours bottom-left, bottom-right, top-right, top-left (the quad order).
    pub fn corners(&self) -> [Color; 4] {
        let (tl, tr, bl, br) = match self.color_mode {
            ColorMode::Single => (self.top_left, self.top_left, self.top_left, self.top_left),
            ColorMode::HorizontalGradient => {
                (self.top_left, self.top_right, self.top_left, self.top_right)
            }
            ColorMode::VerticalGradient => (
                self.top_left,
                self.top_left,
                self.bottom_left,
                self.bottom_left,
            ),
            ColorMode::FourCornersGradient => (
                self.top_left,
                self.top_right,
                self.bottom_left,
                self.bottom_right,
            ),
        };
        [bl, br, tr, tl]
    }

    /// Finds the preset `<gradient="name">` means: a path, or a `.colorgradient` file named
    /// `name` anywhere under Assets.
    pub fn find_preset(assets: &mut AssetServer, name: &str) -> Option<ColorGradient> {
        let path = if name.ends_with(".colorgradient") {
            name.to_owned()
        } else {
            let file = format!("{name}.colorgradient");
            assets
                .paths_of_type("ColorGradient")
                .into_iter()
                .find(|p| p.rsplit('/').next() == Some(file.as_str()))?
        };
        let h: Handle<ColorGradient> = Handle::from_path(path);
        assets.data_asset(&h).cloned()
    }
}

/// TMP Material preset (`.textmaterial`): face, outline and underlay settings several texts
/// share.
#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct TextMaterialPreset {
    pub material: TextMaterial,
}

impl Asset for TextMaterialPreset {
    const TYPE_NAME: &'static str = "TextMaterialPreset";
}

/// The material a text draws with: its preset's, else its own.
pub fn effective_material(
    assets: &mut AssetServer,
    own: &TextMaterial,
    preset: Option<&Handle<TextMaterialPreset>>,
) -> TextMaterial {
    preset
        .and_then(|h| assets.data_asset(h))
        .map_or_else(|| own.clone(), |p| p.material.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gradients_corners_by_mode() {
        let mut g = ColorGradient {
            top_left: Color::rgb(1.0, 0.0, 0.0),
            top_right: Color::rgb(0.0, 1.0, 0.0),
            bottom_left: Color::rgb(0.0, 0.0, 1.0),
            bottom_right: Color::WHITE,
            ..Default::default()
        };
        let [bl, br, tr, tl] = g.corners();
        assert_eq!((bl.b, br, tr.g, tl.r), (1.0, Color::WHITE, 1.0, 1.0));
        g.color_mode = ColorMode::VerticalGradient;
        let [bl, br, tr, tl] = g.corners();
        assert_eq!(
            (bl, br, tr, tl),
            (g.bottom_left, g.bottom_left, g.top_left, g.top_left)
        );
        g.color_mode = ColorMode::Single;
        assert!(g.corners().iter().all(|c| *c == g.top_left));
    }

    #[test]
    fn sprite_assets_from_grids_and_lookup() {
        let a = SpriteAsset::from_grid(
            Handle::from_path("Assets/Emoji.png"),
            128,
            64,
            4,
            2,
            "emoji",
        );
        assert_eq!(a.sprites.len(), 8);
        // Row 0 is the top of the texture (y from the bottom).
        assert_eq!(a.sprites[0].rect, Vector4::new(0.0, 32.0, 32.0, 32.0));
        assert_eq!(a.sprites[5].rect, Vector4::new(32.0, 0.0, 32.0, 32.0));
        let by_index = SpriteTag {
            index: Some(5),
            ..Default::default()
        };
        assert_eq!(a.find(&by_index), Some(5));
        let by_name = SpriteTag {
            name: Some("emoji_2".into()),
            ..Default::default()
        };
        assert_eq!(a.find(&by_name), Some(2));
        let missing = SpriteTag {
            index: Some(9),
            ..Default::default()
        };
        assert_eq!(a.find(&missing), None);
    }
}
