//! Oxide text: TextMeshPro's Font Assets, rich text and layout.
//!
//! - [`font`]: `.fontasset` files made from `.ttf`/`.otf` fonts, and their atlases of
//!   signed-distance-field glyphs (sharp at any size, with outlines and shadows).
//! - [`rich`]: TMP rich text tags.
//! - [`layout`]: wrapping, alignment, overflow, spacing and fallback fonts.
//! - [`components`]: TextMeshPro (3D text) and its material settings.
//!
//! Reference: <https://docs.unity3d.com/Packages/com.unity.ugui@2.0/manual/TextMeshPro/index.html>

pub mod assets;
pub mod components;
pub mod creator;
pub mod font;
pub mod layout;
pub mod rich;
pub mod sdf;

pub use assets::{ColorGradient, ColorMode, SpriteAsset, TextMaterialPreset, TmpSprite};
pub use components::{FontStyles, TextInput, TextMaterial, TextMeshPro, TextMeshes, build};
pub use font::{
    AtlasPopulationMode, BUILTIN_FONT, CharacterSet, DEFAULT_FONT_ASSET, Font, FontAsset,
    FontAtlas, FontLibrary, GlyphInfo,
};
pub use layout::{
    GlyphQuad, HorizontalAlignment, LayoutSettings, TextMesh, TextOverflowModes, VerticalAlignment,
    layout,
};
pub use rich::{StyledChar, parse};

/// Registers the text components and asset types; call when a World is created.
pub fn install(world: &mut oxide_scene::World) {
    world
        .registry
        .register::<TextMeshPro>("Mesh/TextMeshPro - Text");
    if let Some(assets) = world.try_resource_mut::<oxide_assets::AssetServer>() {
        register_assets(assets);
    }
}

/// The Font file type and the Font Asset data type.
pub fn register_assets(assets: &mut oxide_assets::AssetServer) {
    use oxide_assets::AssetTypeInfo;
    assets
        .types
        .register(AssetTypeInfo::new("Font", &["ttf", "otf"]).color(200, 200, 230));
    assets.register_data_asset::<FontAsset>(
        AssetTypeInfo::new("FontAsset", &["fontasset"])
            .menu("TextMeshPro/Font Asset", "New Font Asset", 95)
            .color(230, 230, 140),
    );
    assets.register_data_asset::<SpriteAsset>(
        AssetTypeInfo::new("SpriteAsset", &["spriteasset"])
            .menu("TextMeshPro/Sprite Asset", "New Sprite Asset", 96)
            .color(230, 230, 140),
    );
    assets.register_data_asset::<ColorGradient>(
        AssetTypeInfo::new("ColorGradient", &["colorgradient"])
            .menu("TextMeshPro/Color Gradient", "New Color Gradient", 97)
            .color(230, 230, 140),
    );
    assets.register_data_asset::<TextMaterialPreset>(
        AssetTypeInfo::new("TextMaterialPreset", &["textmaterial"])
            .menu("TextMeshPro/Material Preset", "New Material Preset", 98)
            .color(230, 230, 140),
    );
}
