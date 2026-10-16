//! Unity's Sprite Atlas (`.spriteatlas`): packs the sprites of listed folders and
//! textures into one texture, so sprites from different textures draw in one batch. The
//! packed sprites keep their names and sizes; only their texture and rect change.
//! Reference: <https://docs.unity3d.com/Manual/sprite/atlas/atlas-landing.html>

use crate::handle::{Asset, Handle};
use crate::server::AssetServer;
use crate::sprite::SpriteImportMode;
use crate::texture::{FilterMode, Texture2D, TextureImporterType, TextureWrapMode};
use oxide_core::Reflect;
use oxide_math::{Vector2, Vector4};
use std::path::Path;

#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct SpriteAtlas {
    /// Folders and textures whose sprites are packed (`Assets/Sprites/Characters`).
    pub packables: Vec<String>,
    /// Transparent pixels between sprites.
    pub padding: u32,
    /// Largest atlas side.
    pub max_texture_size: u32,
    pub filter_mode: FilterMode,
    /// Pack when the project loads (Unity "Include in Build").
    pub include_in_build: bool,
}

impl Default for SpriteAtlas {
    fn default() -> Self {
        Self {
            packables: Vec::new(),
            padding: 4,
            max_texture_size: 2048,
            filter_mode: FilterMode::Bilinear,
            include_in_build: true,
        }
    }
}

impl Asset for SpriteAtlas {
    const TYPE_NAME: &'static str = "SpriteAtlas";
}

/// Where a packed sprite now lives.
#[derive(Clone, Debug)]
pub struct AtlasSlot {
    pub atlas: String,
    pub texture: Handle<Texture2D>,
    pub rect: Vector4,
    pub texture_size: Vector2,
}

/// Result of packing.
#[derive(Clone, Debug, Default)]
pub struct AtlasReport {
    pub sprites: usize,
    pub width: u32,
    pub height: u32,
}

/// Shelf packing: places `sizes` (w, h) left to right in rows of the tallest first.
/// Returns top-left positions, or `None` if they don't fit in `width`×`max_h`.
pub fn shelf_pack(
    sizes: &[(u32, u32)],
    width: u32,
    max_h: u32,
    padding: u32,
) -> Option<(Vec<(u32, u32)>, u32)> {
    let mut order: Vec<usize> = (0..sizes.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse((sizes[i].1, sizes[i].0)));
    let mut pos = vec![(0, 0); sizes.len()];
    let (mut x, mut y, mut row_h) = (padding, padding, 0);
    for i in order {
        let (w, h) = sizes[i];
        if w + 2 * padding > width {
            return None;
        }
        if x + w + padding > width {
            x = padding;
            y += row_h + padding;
            row_h = 0;
        }
        if y + h + padding > max_h {
            return None;
        }
        pos[i] = (x, y);
        x += w + padding;
        row_h = row_h.max(h);
    }
    Some((pos, y + row_h + padding))
}

impl AssetServer {
    /// Texture files (imported as Sprite) a packable path names: the texture itself, or
    /// every one inside a folder.
    fn atlas_textures(&self, packables: &[String]) -> Vec<String> {
        let files = self.list_files();
        let mut out: Vec<String> = Vec::new();
        for p in packables {
            let p = p.trim_end_matches('/');
            for f in &files {
                let inside = f == p || f.starts_with(&format!("{p}/"));
                let texture = self
                    .types
                    .for_path(f)
                    .is_some_and(|t| t.type_name == "Texture");
                if inside
                    && texture
                    && self.texture_importer(f).texture_type == TextureImporterType::Sprite
                    && !out.contains(f)
                {
                    out.push(f.clone());
                }
            }
        }
        out.sort();
        out
    }

    /// Packs an atlas: builds its texture (`<atlas>#Texture`) and points its sprites at it.
    pub fn pack_sprite_atlas(&mut self, path: &str) -> Result<AtlasReport, String> {
        let h: Handle<SpriteAtlas> = Handle::from_path(path);
        let atlas = self
            .data_asset(&h)
            .cloned()
            .ok_or_else(|| format!("cannot load {path}"))?;
        // Forget this atlas's old placements first.
        self.atlas_slots.retain(|_, s| s.atlas != path);
        let mut items: Vec<(String, Vec<u8>, u32, u32)> = Vec::new(); // sprite path, pixels, w, h
        for tex in self.atlas_textures(&atlas.packables) {
            let bytes = std::fs::read(self.full_path(&tex)).map_err(|e| format!("{tex}: {e}"))?;
            let img = image::load_from_memory(&bytes)
                .map_err(|e| format!("{tex}: {e}"))?
                .to_rgba8();
            let (iw, ih) = img.dimensions();
            let imp = self.texture_importer(&tex);
            let stem = Path::new(&tex)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Sprite")
                .to_owned();
            let rects: Vec<(String, Vector4)> = match imp.sprite_mode {
                SpriteImportMode::Single => {
                    vec![(stem, Vector4::new(0.0, 0.0, iw as f32, ih as f32))]
                }
                SpriteImportMode::Multiple => imp
                    .sprites
                    .iter()
                    .map(|s| (s.name.clone(), s.rect))
                    .collect(),
            };
            for (name, r) in rects {
                let (x, y, w, h) = (
                    r.x.max(0.0) as u32,
                    r.y.max(0.0) as u32,
                    r.z.max(1.0) as u32,
                    r.w.max(1.0) as u32,
                );
                if x + w > iw || y + h > ih {
                    continue;
                }
                // Rect y counts from the bottom; image rows from the top.
                let top = ih - y - h;
                let mut px = Vec::with_capacity((w * h * 4) as usize);
                for row in top..top + h {
                    let start = ((row * iw + x) * 4) as usize;
                    px.extend_from_slice(&img.as_raw()[start..start + (w * 4) as usize]);
                }
                items.push((format!("{tex}#{name}"), px, w, h));
            }
        }
        let sizes: Vec<(u32, u32)> = items.iter().map(|i| (i.2, i.3)).collect();
        let max = atlas.max_texture_size.clamp(32, 8192);
        let mut width = 64;
        let (placed, height) = loop {
            if let Some(r) = shelf_pack(&sizes, width, max, atlas.padding) {
                break r;
            }
            if width >= max {
                return Err(format!("{path}: the sprites don't fit in {max}×{max}"));
            }
            width *= 2;
        };
        let height = height.next_power_of_two().clamp(1, max);
        let mut rgba = vec![0u8; (width * height * 4) as usize];
        for ((_, px, w, h), (x, y)) in items.iter().zip(&placed) {
            for row in 0..*h {
                let dst = (((y + row) * width + x) * 4) as usize;
                let src = (row * w * 4) as usize;
                rgba[dst..dst + (*w * 4) as usize]
                    .copy_from_slice(&px[src..src + (*w * 4) as usize]);
            }
        }
        let mut tex = Texture2D::from_rgba8("Sprite Atlas", width, height, rgba, true);
        tex.filter_mode = atlas.filter_mode;
        tex.wrap_mode = TextureWrapMode::Clamp;
        if atlas.filter_mode == FilterMode::Point {
            tex.mips.truncate(1);
        }
        let th = self.textures.insert(format!("{path}#Texture"), tex);
        let size = Vector2::new(width as f32, height as f32);
        for ((sprite, _, w, h), (x, y)) in items.iter().zip(&placed) {
            let rect = Vector4::new(*x as f32, (height - y - h) as f32, *w as f32, *h as f32);
            self.atlas_slots.insert(
                sprite.clone(),
                AtlasSlot {
                    atlas: path.to_owned(),
                    texture: th.clone(),
                    rect,
                    texture_size: size,
                },
            );
        }
        // Make sure the sprites exist (their textures loaded), then repoint them.
        let textures: Vec<String> = items
            .iter()
            .filter_map(|i| i.0.split_once('#').map(|(t, _)| t.to_owned()))
            .collect();
        for t in textures {
            let _ = self.ensure_loaded(&t);
        }
        self.apply_atlas_slots();
        Ok(AtlasReport {
            sprites: items.len(),
            width,
            height,
        })
    }

    /// Points loaded sprites that belong to a packed atlas at the atlas texture.
    pub(crate) fn apply_atlas_slots(&mut self) {
        let slots: Vec<(String, AtlasSlot)> = self
            .atlas_slots
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for (sprite, slot) in slots {
            let h = Handle::<crate::sprite::Sprite>::from_path(sprite);
            if let Some(s) = self.sprites.get_mut(&h) {
                s.texture = slot.texture.clone();
                s.rect = slot.rect;
                s.texture_size = slot.texture_size;
            }
        }
    }

    /// Packs every Sprite Atlas in the project that is included in the build (at startup).
    pub fn load_sprite_atlases(&mut self) {
        let atlases: Vec<String> = self
            .list_files()
            .into_iter()
            .filter(|p| {
                self.types
                    .for_path(p)
                    .is_some_and(|t| t.type_name == "SpriteAtlas")
            })
            .collect();
        for a in atlases {
            let h: Handle<SpriteAtlas> = Handle::from_path(a.clone());
            if self.data_asset(&h).is_some_and(|x| x.include_in_build)
                && let Err(e) = self.pack_sprite_atlas(&a)
            {
                oxide_core::debug_log_warning!("{e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sprite::Sprite;

    fn png(path: &Path, w: u32, h: u32, c: [u8; 4]) {
        image::RgbaImage::from_pixel(w, h, image::Rgba(c))
            .save(path)
            .unwrap();
    }

    #[test]
    fn shelf_packing_fits_and_pads() {
        let (pos, h) = shelf_pack(&[(10, 10), (20, 5), (10, 10)], 64, 64, 2).unwrap();
        assert_eq!(pos[0], (2, 2));
        assert_eq!(pos[2], (14, 2));
        assert_eq!(pos[1], (26, 2), "same shelf");
        assert_eq!(h, 14);
        assert!(shelf_pack(&[(70, 10)], 64, 64, 0).is_none());
    }

    #[test]
    fn atlas_repoints_sprites_to_one_texture() {
        let dir = std::env::temp_dir().join(format!("oxide-atlas-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Assets/Sprites")).unwrap();
        png(&dir.join("Assets/Sprites/a.png"), 16, 16, [255, 0, 0, 255]);
        png(&dir.join("Assets/Sprites/b.png"), 8, 24, [0, 0, 255, 255]);
        let mut a = AssetServer::new(&dir);
        for t in ["Assets/Sprites/a.png", "Assets/Sprites/b.png"] {
            let mut imp = a.texture_importer(t);
            imp.texture_type = TextureImporterType::Sprite;
            a.set_texture_importer(t, &imp).unwrap();
        }
        let sa: Handle<Sprite> = Handle::from_path("Assets/Sprites/a.png#a");
        let sb: Handle<Sprite> = Handle::from_path("Assets/Sprites/b.png#b");
        let size_before = a.sprites.get(&sb).unwrap().size();
        assert_ne!(
            a.sprites.get(&sa).unwrap().texture,
            a.sprites.get(&sb).unwrap().texture
        );

        a.save_data(
            "Assets/Main.spriteatlas",
            Box::new(SpriteAtlas {
                packables: vec!["Assets/Sprites".into()],
                ..Default::default()
            }),
        )
        .unwrap();
        let r = a.pack_sprite_atlas("Assets/Main.spriteatlas").unwrap();
        assert_eq!(r.sprites, 2);
        let (pa, pb) = (
            a.sprites.get(&sa).unwrap().clone(),
            a.sprites.get(&sb).unwrap().clone(),
        );
        assert_eq!(pa.texture, pb.texture, "one atlas texture");
        assert_eq!(pb.size(), size_before, "same size in the world");
        // The atlas holds b's blue pixels where its rect says.
        let t = a.textures.get(&pa.texture).unwrap();
        let (x, y_top) = (
            pb.rect.x as u32 + 1,
            t.height - (pb.rect.y + pb.rect.w) as u32 + 1,
        );
        let i = ((y_top * t.width + x) * 4) as usize;
        assert_eq!(&t.mips[0][i..i + 4], &[0, 0, 255, 255]);
        // Re-importing a packed texture keeps its sprite on the atlas.
        let mut imp = a.texture_importer("Assets/Sprites/a.png");
        imp.pixels_per_unit = 50.0;
        a.set_texture_importer("Assets/Sprites/a.png", &imp)
            .unwrap();
        assert_eq!(a.sprites.get(&sa).unwrap().texture, pa.texture);
        let _ = std::fs::remove_dir_all(dir);
    }
}
