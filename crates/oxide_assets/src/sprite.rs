//! Unity `Sprite`: a rectangle of a texture with a pivot and a size in world units
//! (`pixels_per_unit`). Sprites are sub-assets of their texture (`Assets/hero.png#hero`,
//! or one per slice: `Assets/sheet.png#walk_0`), made when a texture is imported with
//! Texture Type "Sprite (2D and UI)".
//! Reference: <https://docs.unity3d.com/Manual/sprite/sprite-landing.html>

use crate::handle::{Asset, Handle};
use crate::texture::Texture2D;
use oxide_core::Reflect;
use oxide_math::{Vector2, Vector4};

/// Unity `SpriteImportMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Reflect)]
pub enum SpriteImportMode {
    /// The whole texture is one sprite.
    #[default]
    Single,
    /// The texture is a sprite sheet sliced into several sprites (Sprite Editor).
    Multiple,
}

/// One slice of a sprite sheet (Unity `SpriteMetaData`), in pixels with the origin at the
/// bottom left of the texture.
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct SpriteRect {
    pub name: String,
    /// x, y, width, height (pixels).
    pub rect: Vector4,
    /// Normalized pivot inside the rect (0.5, 0.5 = centre).
    pub pivot: Vector2,
    /// 9-slice border: left, bottom, right, top (pixels).
    pub border: Vector4,
}

impl Default for SpriteRect {
    fn default() -> Self {
        Self {
            name: "Sprite".into(),
            rect: Vector4::new(0.0, 0.0, 32.0, 32.0),
            pivot: Vector2::new(0.5, 0.5),
            border: Vector4::ZERO,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sprite {
    pub name: String,
    pub texture: Handle<Texture2D>,
    /// x, y, width, height in texture pixels (origin bottom left).
    pub rect: Vector4,
    pub pivot: Vector2,
    pub pixels_per_unit: f32,
    /// Left, bottom, right, top (pixels).
    pub border: Vector4,
    /// Texture size, for UVs.
    pub texture_size: Vector2,
}

impl Asset for Sprite {
    const TYPE_NAME: &'static str = "Sprite";
}

impl Sprite {
    /// Size in world units.
    pub fn size(&self) -> Vector2 {
        Vector2::new(self.rect.z, self.rect.w) / self.pixels_per_unit.max(0.0001)
    }

    /// UV rect (origin top left, as the GPU samples) of a sub-rect given in sprite pixels.
    pub fn uv_rect_of(&self, x: f32, y: f32, w: f32, h: f32) -> [f32; 4] {
        let (tw, th) = (self.texture_size.x.max(1.0), self.texture_size.y.max(1.0));
        let px = self.rect.x + x;
        // Rect y counts from the bottom; texture rows from the top.
        let top = th - (self.rect.y + y + h);
        [px / tw, top / th, w / tw, h / th]
    }

    pub fn uv_rect(&self) -> [f32; 4] {
        self.uv_rect_of(0.0, 0.0, self.rect.z, self.rect.w)
    }
}
