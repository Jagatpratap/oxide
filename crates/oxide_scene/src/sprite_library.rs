//! Unity's 2D Animation sprite swapping: a Sprite Library Asset (`.spriteLib`) maps
//! category + label to a sprite (Category "Head", labels "Helmet", "Cap", ...). A
//! SpriteLibrary component picks the asset for a character; SpriteResolvers below it
//! choose a category and label, which sets their SpriteRenderer's sprite (skins,
//! equipment, flipbook animation by animating the label).
//! Reference: <https://docs.unity3d.com/Packages/com.unity.2d.animation@10.1/manual/SL-Asset.html>

use crate::component::Component;
use crate::id::GameObjectId;
use crate::world::World;
use oxide_assets::{Asset, AssetServer, Handle, Sprite};
use oxide_core::Reflect;

#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct SpriteLibraryEntry {
    pub label: String,
    pub sprite: Option<Handle<Sprite>>,
}

#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct SpriteLibraryCategory {
    pub name: String,
    pub entries: Vec<SpriteLibraryEntry>,
}

/// Unity `SpriteLibraryAsset`.
#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct SpriteLibraryAsset {
    pub categories: Vec<SpriteLibraryCategory>,
}

impl Asset for SpriteLibraryAsset {
    const TYPE_NAME: &'static str = "SpriteLibraryAsset";
}

impl SpriteLibraryAsset {
    /// Unity `GetSprite(category, label)`.
    pub fn get_sprite(&self, category: &str, label: &str) -> Option<&Handle<Sprite>> {
        self.categories
            .iter()
            .find(|c| c.name == category)?
            .entries
            .iter()
            .find(|e| e.label == label)?
            .sprite
            .as_ref()
    }
}

/// Unity `SpriteLibrary`: the Sprite Library Asset for this object and its children.
#[derive(Reflect, Clone, Debug, Default)]
pub struct SpriteLibrary {
    pub sprite_library_asset: Option<Handle<SpriteLibraryAsset>>,
}

impl Component for SpriteLibrary {
    fn allow_multiple(&self) -> bool {
        false
    }
}

/// Unity `SpriteResolver`: shows the sprite at `category`/`label` of the nearest
/// SpriteLibrary on this object or above it.
#[derive(Reflect, Clone, Debug, Default)]
pub struct SpriteResolver {
    pub category: String,
    pub label: String,
}

impl Component for SpriteResolver {
    fn allow_multiple(&self) -> bool {
        false
    }
}

impl SpriteResolver {
    /// Unity `SetCategoryAndLabel`.
    pub fn set_category_and_label(&mut self, category: &str, label: &str) {
        self.category = category.to_owned();
        self.label = label.to_owned();
    }
}

/// The sprite a resolver on `id` points at, if its library and entry exist.
pub fn resolved_sprite(world: &World, id: GameObjectId) -> Option<Handle<Sprite>> {
    let r = world.get_component::<SpriteResolver>(id)?;
    let lib_obj = world.ancestor_with::<SpriteLibrary>(id)?;
    let lib = world
        .get_component::<SpriteLibrary>(lib_obj)?
        .sprite_library_asset
        .clone()?;
    let assets = world.resource::<AssetServer>();
    assets
        .data
        .get(&lib)?
        .get_sprite(&r.category, &r.label)
        .cloned()
}

/// Copies resolved sprites into the SpriteRenderers (so scripts read the current sprite),
/// loading library assets on first use. Returns how many changed.
pub fn resolve_all(world: &mut World) -> usize {
    let libs: Vec<Handle<SpriteLibraryAsset>> = world
        .query::<SpriteLibrary>()
        .filter_map(|(_, l)| l.sprite_library_asset.clone())
        .collect();
    if libs.is_empty() {
        return 0;
    }
    if let Some(a) = world.try_resource_mut::<AssetServer>() {
        for l in &libs {
            let _ = a.data_asset(l);
        }
    }
    let ids: Vec<GameObjectId> = world.query::<SpriteResolver>().map(|(id, _)| id).collect();
    let mut n = 0;
    let mut textures = Vec::new();
    for id in ids {
        let Some(s) = resolved_sprite(world, id) else {
            continue;
        };
        if let Some(r) = world.get_component_mut::<crate::SpriteRenderer>(id)
            && r.sprite.as_ref() != Some(&s)
        {
            textures.extend(s.path().split('#').next().map(str::to_owned));
            r.sprite = Some(s);
            n += 1;
        }
    }
    // The sprites' textures (sprites exist once their texture is imported).
    if let Some(a) = world.try_resource_mut::<AssetServer>() {
        for t in textures.iter().filter(|t| t.starts_with("Assets/")) {
            let _ = a.ensure_loaded(t);
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolver_picks_from_the_nearest_library() {
        let mut w = World::new();
        crate::register_builtin(&mut w.registry);
        w.resources.insert(AssetServer::new("."));
        let lib = SpriteLibraryAsset {
            categories: vec![SpriteLibraryCategory {
                name: "Hat".into(),
                entries: vec![
                    SpriteLibraryEntry {
                        label: "Cap".into(),
                        sprite: Some(Handle::from_path("builtin:Sprites/Square")),
                    },
                    SpriteLibraryEntry {
                        label: "Crown".into(),
                        sprite: Some(Handle::from_path("builtin:Sprites/Circle")),
                    },
                ],
            }],
        };
        w.resource_mut::<AssetServer>().data.insert(
            "Assets/Hero.spriteLib",
            "SpriteLibraryAsset",
            Box::new(lib),
        );
        let hero = w.create("Hero");
        w.add_component(
            hero,
            SpriteLibrary {
                sprite_library_asset: Some(Handle::from_path("Assets/Hero.spriteLib")),
            },
        );
        let hat = w.create("Hat");
        w.set_parent(hat, Some(hero), false).unwrap();
        w.add_component(hat, crate::SpriteRenderer::default());
        w.add_component(
            hat,
            SpriteResolver {
                category: "Hat".into(),
                label: "Cap".into(),
            },
        );
        assert_eq!(resolve_all(&mut w), 1);
        let shown = |w: &World| {
            w.get_component::<crate::SpriteRenderer>(hat)
                .unwrap()
                .sprite
                .as_ref()
                .map(|s| s.path().to_owned())
        };
        assert_eq!(shown(&w).as_deref(), Some("builtin:Sprites/Square"));
        w.get_component_mut::<SpriteResolver>(hat)
            .unwrap()
            .set_category_and_label("Hat", "Crown");
        resolve_all(&mut w);
        assert_eq!(shown(&w).as_deref(), Some("builtin:Sprites/Circle"));
        assert_eq!(resolve_all(&mut w), 0, "nothing changed");
    }
}
