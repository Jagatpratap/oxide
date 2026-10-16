//! Data asset types the engine itself defines (Unity's built-in ScriptableObjects), added
//! to the asset type registry when a World is created. Systems in other crates register
//! their own (Physics Material in `oxide_physics`).

use oxide_assets::{AssetServer, AssetTypeInfo};

pub fn register(assets: &mut AssetServer) {
    assets.register_data_asset::<oxide_scene::VolumeProfile>(
        AssetTypeInfo::new("VolumeProfile", &["volumeProfile"])
            .menu("Rendering/Volume Profile", "New Volume Profile", 61)
            .color(150, 190, 240),
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
