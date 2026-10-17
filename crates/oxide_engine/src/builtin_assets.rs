//! Data asset types the engine itself defines (Unity's built-in ScriptableObjects), added
//! to the asset type registry when a World is created. Systems in other crates register
//! their own (Physics Material in `oxide_physics`).

use oxide_assets::{AssetServer, AssetTypeInfo};
use oxide_input::{Input, InputActionAsset};
use oxide_scene::{PlayerInput, World};

pub fn register(assets: &mut AssetServer) {
    oxide_text::register_assets(assets);
    oxide_timeline::register_assets(assets);
    oxide_shadergraph::register_assets(assets);
    assets.register_data_asset::<oxide_scene::CurvePresetLibrary>(
        AssetTypeInfo::new("CurvePresetLibrary", &["curves"])
            .menu("Presets/Curve Preset Library", "New Curves", 300)
            .color(140, 220, 160),
    );
    assets.register_data_asset::<oxide_scene::GradientPresetLibrary>(
        AssetTypeInfo::new("GradientPresetLibrary", &["gradients"])
            .menu("Presets/Gradient Preset Library", "New Gradients", 301)
            .color(220, 160, 220),
    );
    assets.register_data_asset::<oxide_scene::Tile>(
        AssetTypeInfo::new("Tile", &["tile"])
            .menu("2D/Tiles/Tile", "New Tile", 81)
            .color(140, 200, 120),
    );
    assets.register_data_asset::<oxide_scene::AnimatedTile>(
        AssetTypeInfo::new("AnimatedTile", &["animatedtile"])
            .menu("2D/Tiles/Animated Tile", "New Animated Tile", 82)
            .color(140, 200, 160),
    );
    assets.register_data_asset::<oxide_scene::RuleTile>(
        AssetTypeInfo::new("RuleTile", &["ruletile"])
            .menu("2D/Tiles/Rule Tile", "New Rule Tile", 83)
            .color(120, 190, 140),
    );
    assets.register_data_asset::<oxide_scene::SpriteLibraryAsset>(
        AssetTypeInfo::new("SpriteLibraryAsset", &["spriteLib"])
            .menu("2D/Sprite Library Asset", "New Sprite Library Asset", 85)
            .color(200, 170, 230),
    );
    assets.register_data_asset::<oxide_scene::TilePalette>(
        AssetTypeInfo::new("TilePalette", &["palette"])
            .menu("2D/Tile Palette", "New Palette", 84)
            .color(170, 210, 120),
    );
    assets.register_data_asset::<InputActionAsset>(
        AssetTypeInfo::new("InputActionAsset", &["inputactions"])
            .menu("Input Actions", "New Actions", 50)
            .color(240, 150, 120),
    );
    assets.register_data_asset::<oxide_scene::VolumeProfile>(
        AssetTypeInfo::new("VolumeProfile", &["volumeProfile"])
            .menu("Rendering/Volume Profile", "New Volume Profile", 61)
            .color(150, 190, 240),
    );
    assets.register_data_asset::<oxide_scene::LensFlareData>(
        AssetTypeInfo::new("LensFlareData", &["lensflare"])
            .menu("Rendering/Lens Flare (SRP)", "New Lens Flare", 64)
            .color(250, 200, 110),
    );
    assets.register_data_asset::<oxide_scene::LightingSettings>(
        AssetTypeInfo::new("LightingSettings", &["lighting"])
            .menu("Rendering/Lighting Settings", "New Lighting Settings", 62)
            .color(240, 210, 120),
    );
    assets.register_data_asset::<oxide_scene::LightmapParameters>(
        AssetTypeInfo::new("LightmapParameters", &["giparams"])
            .menu(
                "Rendering/Lightmap Parameters",
                "New Lightmap Parameters",
                63,
            )
            .color(230, 200, 140),
    );
}

/// Evaluates the action maps enabled by `PlayerInput` components into the Input resource
/// (once per frame, before scripts run).
pub fn update_input_actions(world: &mut World, dt: f32) {
    let wanted: Vec<(oxide_assets::Handle<InputActionAsset>, String, String)> = world
        .query::<PlayerInput>()
        .filter(|(id, _)| world.get(*id).is_some_and(|o| o.active_in_hierarchy()))
        .filter_map(|(_, p)| {
            Some((
                p.actions.clone()?,
                p.default_map.clone(),
                p.default_scheme.clone(),
            ))
        })
        .collect();
    let assets = world.resource_mut::<AssetServer>();
    let mut loaded: Vec<(InputActionAsset, String, String)> = Vec::new();
    for (h, map, scheme) in wanted {
        if let Some(a) = assets.data_asset(&h) {
            loaded.push((a.clone(), map, scheme));
        }
    }
    let maps: Vec<_> = loaded
        .iter()
        .flat_map(|(a, map, scheme)| {
            let scheme = a
                .control_schemes
                .iter()
                .find(|s| !scheme.is_empty() && s.name.eq_ignore_ascii_case(scheme));
            a.maps
                .iter()
                .filter(move |m| map.is_empty() || m.name.eq_ignore_ascii_case(map))
                .map(move |m| (m, scheme))
        })
        .collect();
    world.resource_mut::<Input>().update_actions(&maps, dt);
}
