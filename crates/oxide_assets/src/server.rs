//! The asset database: typed stores plus loading from the project's `Assets/` folder.
//! Reference: <https://docs.unity3d.com/Manual/AssetWorkflow.html>

use crate::animation::AnimationClip;
use crate::animator_controller::AnimatorController;
use crate::handle::{Asset, Assets, Handle};
use crate::material::Material;
use crate::mesh::Mesh;
use crate::model::Model;
use crate::primitives::PrimitiveType;
use crate::texture::Texture2D;
use oxide_core::reflect::{Reflect, Value};
use oxide_math::Color;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Encoded audio file contents; decoded by the audio system.
#[derive(Clone, Debug)]
pub struct AudioClip {
    pub name: String,
    pub bytes: Arc<[u8]>,
}

impl Asset for AudioClip {
    const TYPE_NAME: &'static str = "AudioClip";
}

/// The `labels:` of a `.meta` file's text.
pub fn labels_in_meta(text: &str) -> Vec<String> {
    let map: serde_yaml_ng::Mapping = serde_yaml_ng::from_str(text).unwrap_or_default();
    match map.get("labels") {
        Some(serde_yaml_ng::Value::Sequence(v)) => v
            .iter()
            .filter_map(|l| l.as_str().map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    }
}

/// `.meta` fields that belong to every asset rather than to an importer.
const META_KEYS: &[&str] = &["labels", "guid"];

pub struct AssetServer {
    /// Project folder; asset paths like `Assets/x.png` are relative to it.
    pub root: PathBuf,
    pub meshes: Assets<Mesh>,
    pub materials: Assets<Material>,
    pub textures: Assets<Texture2D>,
    pub models: Assets<Model>,
    pub animations: Assets<AnimationClip>,
    pub controllers: Assets<AnimatorController>,
    /// Avatar Mask assets (`.mask`).
    pub masks: Assets<crate::animator_controller::AvatarMask>,
    /// Animator Override Controllers as authored (their resolved controllers are in
    /// `controllers` under the same path).
    pub override_controllers: Assets<crate::animator_controller::AnimatorOverrideController>,
    pub audio: Assets<AudioClip>,
    pub terrains: Assets<crate::terrain::TerrainData>,
    pub render_textures: Assets<crate::render_texture::RenderTexture>,
    pub shaders: Assets<crate::shader::Shader>,
    /// Sprites made from textures imported as Sprite (`Assets/sheet.png#walk_0`).
    pub sprites: Assets<crate::sprite::Sprite>,
    /// Sprites packed into a Sprite Atlas: where they live in it.
    pub atlas_slots: std::collections::HashMap<String, crate::sprite_atlas::AtlasSlot>,
    /// Every asset type the Project window knows (built-in and registered).
    pub types: crate::asset_types::AssetTypeRegistry,
    /// Loaded data assets (Unity ScriptableObjects): Physics Materials, Volume Profiles, ...
    pub data: crate::asset_types::DataAssets,
    /// GUIDs of every asset under `Assets/` (Unity's AssetDatabase). Scanned by the editor
    /// ([`AssetDatabase::refresh`](crate::database::AssetDatabase::refresh)); empty in players.
    pub database: crate::database::AssetDatabase,
    /// Modification times of files read, for [`AssetServer::reload_changed`].
    mtimes: std::sync::Mutex<std::collections::HashMap<String, std::time::SystemTime>>,
}

pub const DEFAULT_MATERIAL: &str = "builtin:Default-Material";

impl AssetServer {
    /// Creates the stores and registers the built-in assets (primitive meshes, default
    /// material, white/black/normal textures).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root: PathBuf = root.into();
        let mut s = Self {
            database: crate::database::AssetDatabase::new(root.clone()),
            root,
            meshes: Assets::default(),
            materials: Assets::default(),
            textures: Assets::default(),
            models: Assets::default(),
            animations: Assets::default(),
            controllers: Assets::default(),
            masks: Assets::default(),
            override_controllers: Assets::default(),
            audio: Assets::default(),
            terrains: Assets::default(),
            render_textures: Assets::default(),
            shaders: Assets::default(),
            sprites: Assets::default(),
            atlas_slots: Default::default(),
            types: Default::default(),
            data: Default::default(),
            mtimes: Default::default(),
        };
        for t in crate::asset_types::builtin_types() {
            s.types.register(t);
        }
        s.register_data_asset::<crate::preset::Preset>(
            crate::AssetTypeInfo::new("Preset", &["preset"]).color(190, 170, 230),
        );
        s.register_data_asset::<crate::sprite_atlas::SpriteAtlas>(
            crate::AssetTypeInfo::new("SpriteAtlas", &["spriteatlas"])
                .menu("2D/Sprite Atlas", "New Sprite Atlas", 80)
                .color(240, 170, 120),
        );
        for p in PrimitiveType::ALL {
            // Built-in primitives have lightmap UVs, like Unity's.
            let mut m = p.mesh();
            m.generate_lightmap_uvs();
            s.meshes.insert(p.mesh_path(), m);
        }
        s.materials.insert(
            DEFAULT_MATERIAL,
            Material::lit("Default-Material", Color::WHITE),
        );
        s.textures
            .insert("builtin:white", Texture2D::solid("white", [255; 4], true));
        s.textures.insert(
            "builtin:black",
            Texture2D::solid("black", [0, 0, 0, 255], true),
        );
        s.textures.insert(
            "builtin:normal",
            Texture2D::solid("normal", [128, 128, 255, 255], false),
        );
        // Unity's built-in 2D sprites (GameObject → 2D Object → Sprites).
        let n = 64u32;
        let mut circle = Vec::with_capacity((n * n * 4) as usize);
        for y in 0..n {
            for x in 0..n {
                let d = ((x as f32 + 0.5 - n as f32 / 2.0).hypot(y as f32 + 0.5 - n as f32 / 2.0))
                    / (n as f32 / 2.0);
                let a = ((1.0 - d) * n as f32 / 2.0).clamp(0.0, 1.0);
                circle.extend_from_slice(&[255, 255, 255, (a * 255.0) as u8]);
            }
        }
        for (name, tex, px) in [
            ("Square", Texture2D::solid("Square", [255; 4], true), 1u32),
            (
                "Circle",
                Texture2D::from_rgba8("Circle", n, n, circle, true),
                n,
            ),
        ] {
            let tpath = format!("builtin:sprite-{}", name.to_lowercase());
            let th = s.textures.insert(tpath, tex);
            s.sprites.insert(
                format!("builtin:Sprites/{name}"),
                crate::sprite::Sprite {
                    name: name.into(),
                    texture: th,
                    rect: oxide_math::Vector4::new(0.0, 0.0, px as f32, px as f32),
                    pivot: oxide_math::Vector2::new(0.5, 0.5),
                    pixels_per_unit: px as f32,
                    border: oxide_math::Vector4::ZERO,
                    texture_size: oxide_math::Vector2::new(px as f32, px as f32),
                },
            );
        }
        s.textures.insert(
            "builtin:grid",
            Texture2D::checker("grid", 256, 8, [200, 200, 200, 255], [150, 150, 150, 255]),
        );
        s
    }

    pub fn default_material(&self) -> Handle<Material> {
        Handle::from_path(DEFAULT_MATERIAL)
    }

    pub fn primitive_mesh(&self, p: PrimitiveType) -> Handle<Mesh> {
        Handle::from_path(p.mesh_path())
    }

    /// Whether a file exists at this project path (`Assets/...`).
    pub fn exists(&self, path: &str) -> bool {
        self.full_path(path).exists()
    }

    pub fn full_path(&self, path: &str) -> PathBuf {
        let p = Path::new(path);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.root.join(p)
        }
    }

    /// Reads an asset file (and remembers its time for [`AssetServer::reload_changed`]).
    pub(crate) fn read_file(&self, path: &str) -> Result<Vec<u8>, String> {
        self.read(path)
    }

    /// Writes an asset file, creating its folder.
    pub(crate) fn write_file(&self, path: &str, text: &str) -> Result<(), String> {
        self.write_bytes(path, text.as_bytes())
    }

    /// Writes a file in the project (and remembers the write so it isn't reloaded).
    pub(crate) fn write_bytes(&self, path: &str, bytes: &[u8]) -> Result<(), String> {
        let full = self.full_path(path);
        if let Some(dir) = full.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        std::fs::write(&full, bytes)
            .map_err(|e| format!("cannot write {}: {e}", full.display()))?;
        // Our own write isn't an outside edit to reload.
        if let Ok(t) = std::fs::metadata(&full).and_then(|m| m.modified()) {
            self.mtimes
                .lock()
                .expect("mtimes lock")
                .insert(path.to_owned(), t);
        }
        Ok(())
    }

    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        let full = self.full_path(path);
        let bytes =
            std::fs::read(&full).map_err(|e| format!("cannot read {}: {e}", full.display()))?;
        let stamp = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        let mut mtimes = self.mtimes.lock().expect("mtimes lock");
        if let Some(t) = stamp(&full) {
            mtimes.insert(path.to_owned(), t);
        }
        // Texture import settings live next to the texture.
        if let Some(t) = stamp(&self.full_path(&format!("{path}.meta"))) {
            mtimes.insert(format!("{path}.meta"), t);
        }
        Ok(bytes)
    }

    /// Re-imports textures, materials, shaders and render textures whose files (or texture
    /// `.meta` files) changed on disk since they were read, like Unity's Asset Database
    /// refresh. Returns the reloaded paths.
    pub fn reload_changed(&mut self) -> Vec<String> {
        let changed: Vec<String> = {
            let mtimes = self.mtimes.lock().expect("mtimes lock");
            mtimes
                .iter()
                .filter(|(p, t)| {
                    std::fs::metadata(self.full_path(p))
                        .and_then(|m| m.modified())
                        .is_ok_and(|now| now > **t)
                })
                .map(|(p, _)| p.trim_end_matches(".meta").to_owned())
                .collect()
        };
        let mut done = Vec::new();
        for path in changed {
            if done.contains(&path) {
                continue;
            }
            let ext = Path::new(&path)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let result = match ext.as_str() {
                "png" | "jpg" | "jpeg" | "tga" | "bmp" | "hdr" | "ktx2" => {
                    self.load_texture(&path).map(|_| ())
                }
                "mat" => self.load_material(&path).map(|_| ()),
                "wgsl" => self.load_shader(&path).map(|_| ()),
                "rendertexture" => self.load_render_texture(&path).map(|_| ()),
                "anim" => self.load_animation_clip(&path).map(|_| ()),
                "controller" => self.load_animator_controller(&path).map(|_| ()),
                "overridecontroller" => self.load_override_controller(&path).map(|_| ()),
                "mask" => self.load_avatar_mask(&path).map(|_| ()),
                "terraindata" => self.load_terrain_data(&path).map(|_| ()),
                _ if self
                    .types
                    .for_path(&path)
                    .is_some_and(|t| self.types.is_data(t.type_name)) =>
                {
                    self.load_data(&path)
                }
                _ => {
                    // Not hot-reloadable yet: remember the new time so it isn't retried.
                    let _ = self.read(&path);
                    continue;
                }
            };
            match result {
                Ok(()) => done.push(path),
                Err(e) => {
                    oxide_core::debug_log_error!("{e}");
                    let _ = self.read(&path);
                }
            }
        }
        done
    }

    /// The fields of `<path>.meta` (empty if there is none or it can't be read).
    fn meta_map(&self, path: &str) -> serde_yaml_ng::Mapping {
        std::fs::read_to_string(self.full_path(&format!("{path}.meta")))
            .ok()
            .and_then(|t| serde_yaml_ng::from_str(&t).ok())
            .unwrap_or_default()
    }

    fn write_meta_map(&self, path: &str, map: &serde_yaml_ng::Mapping) -> Result<(), String> {
        let meta = self.full_path(&format!("{path}.meta"));
        if map.is_empty() {
            let _ = std::fs::remove_file(&meta);
            return Ok(());
        }
        let text = serde_yaml_ng::to_string(map).map_err(|e| e.to_string())?;
        std::fs::write(&meta, text).map_err(|e| format!("cannot write {}: {e}", meta.display()))
    }

    /// Unity Asset Labels of an asset, kept in its `.meta` file (`labels: [...]`).
    pub fn asset_labels(&self, path: &str) -> Vec<String> {
        std::fs::read_to_string(self.full_path(&format!("{path}.meta")))
            .map(|t| labels_in_meta(&t))
            .unwrap_or_default()
    }

    /// Replaces an asset's labels (sorted, without duplicates), keeping its other `.meta`
    /// fields such as texture import settings.
    pub fn set_asset_labels(&self, path: &str, labels: &[String]) -> Result<(), String> {
        let mut labels: Vec<String> = labels
            .iter()
            .map(|l| l.trim().to_owned())
            .filter(|l| !l.is_empty())
            .collect();
        labels.sort_by_key(|l| l.to_lowercase());
        labels.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        let mut map = self.meta_map(path);
        if labels.is_empty() {
            map.remove("labels");
        } else {
            map.insert(
                "labels".into(),
                serde_yaml_ng::Value::Sequence(labels.into_iter().map(Into::into).collect()),
            );
        }
        self.write_meta_map(path, &map)
    }

    /// Import settings of a texture: its `.meta` file, or defaults guessed from the name.
    pub fn texture_importer(&self, path: &str) -> crate::TextureImporter {
        let meta = self.full_path(&format!("{path}.meta"));
        let mut imp = crate::TextureImporter::guess(path);
        if let Ok(text) = std::fs::read_to_string(&meta) {
            match serde_yaml_ng::from_str::<serde_yaml_ng::Mapping>(&text) {
                Ok(fields) => {
                    for (k, v) in fields {
                        let Some(k) = k.as_str() else { continue };
                        if META_KEYS.contains(&k) {
                            continue;
                        }
                        let r = serde_yaml_ng::from_value::<Value>(v)
                            .map_err(|e| e.to_string())
                            .and_then(|v| imp.set_field(k, v).map_err(|e| e.to_string()));
                        if let Err(e) = r {
                            oxide_core::debug_log_warning!("{path}.meta: {k}: {e}");
                        }
                    }
                }
                Err(e) => oxide_core::debug_log_warning!("{path}.meta: {e}"),
            }
        }
        imp
    }

    /// Writes `<path>.meta` and re-imports the texture with the new settings.
    pub fn set_texture_importer(
        &mut self,
        path: &str,
        importer: &crate::TextureImporter,
    ) -> Result<Handle<Texture2D>, String> {
        let fields: BTreeMap<String, Value> = importer
            .fields()
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect();
        // Labels and other non-importer fields stay.
        let mut map = self.meta_map(path);
        for (k, v) in fields {
            let v = serde_yaml_ng::to_value(v).map_err(|e| e.to_string())?;
            map.insert(k.into(), v);
        }
        self.write_meta_map(path, &map)?;
        self.load_texture(path)
    }

    /// Loads (or reloads) a texture with its import settings: size limit, mipmaps, and GPU
    /// compression (cached under `Library/TextureCache`, like Unity's Library folder).
    pub fn load_texture(&mut self, path: &str) -> Result<Handle<Texture2D>, String> {
        let bytes = self.read(path)?;
        let imp = self.texture_importer(path);
        let name = Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("texture");
        if path.to_ascii_lowercase().ends_with(".ktx2") {
            // Already GPU-ready: no resizing or recompression.
            let mut tex = crate::ktx2_import::decode(name, &bytes)?;
            tex.wrap_mode = imp.wrap_mode;
            tex.filter_mode = imp.filter_mode;
            return Ok(self.textures.insert(path.to_owned(), tex));
        }
        let mut tex = Texture2D::decode(name, &bytes, imp.srgb)?;
        // Sprite rects are in the source image's pixels, whatever the import size.
        let source_size = (tex.width, tex.height);
        tex.limit_size(imp.max_size);
        if !imp.generate_mip_maps {
            tex.mips.truncate(1);
        }
        tex.wrap_mode = imp.wrap_mode;
        tex.filter_mode = imp.filter_mode;
        tex.aniso_level = imp.aniso_level.clamp(1, 16) as u8;
        let has_alpha = tex.mips[0].as_chunks::<4>().0.iter().any(|p| p[3] < 255);
        let format =
            crate::texture_compress::choose_format(imp.compression, imp.texture_type, has_alpha);
        if let Some(format) = format
            && tex.width % 4 == 0
            && tex.height % 4 == 0
        {
            tex.compressed = Some(self.compressed_cached(&bytes, &imp, format, &tex));
        }
        let (w, h) = source_size;
        let handle = self.textures.insert(path.to_owned(), tex);
        self.make_sprites(path, &imp, w, h);
        Ok(handle)
    }

    /// Sprite sub-assets of a texture imported as Sprite: the whole texture (Single) or
    /// each slice (Multiple), at `<path>#<name>`.
    fn make_sprites(&mut self, path: &str, imp: &crate::TextureImporter, w: u32, h: u32) {
        let prefix = format!("{path}#");
        let old: Vec<String> = self
            .sprites
            .iter()
            .filter(|(p, _)| p.starts_with(&prefix))
            .map(|(p, _)| p.to_owned())
            .collect();
        for p in old {
            self.sprites.remove(&Handle::from_path(p));
        }
        if imp.texture_type != crate::TextureImporterType::Sprite {
            return;
        }
        let stem = Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Sprite")
            .to_owned();
        let texture: Handle<Texture2D> = Handle::from_path(path);
        let size = oxide_math::Vector2::new(w as f32, h as f32);
        let rects: Vec<crate::sprite::SpriteRect> = match imp.sprite_mode {
            crate::sprite::SpriteImportMode::Single => vec![crate::sprite::SpriteRect {
                name: stem,
                rect: oxide_math::Vector4::new(0.0, 0.0, w as f32, h as f32),
                pivot: imp.sprite_pivot,
                border: imp.sprite_border,
            }],
            crate::sprite::SpriteImportMode::Multiple => imp.sprites.clone(),
        };
        for r in rects {
            let sprite_path = format!("{path}#{}", r.name);
            // Packed into an atlas: drawn from the atlas texture.
            let (texture, rect, texture_size) = match self.atlas_slots.get(&sprite_path) {
                Some(slot) => (slot.texture.clone(), slot.rect, slot.texture_size),
                None => (texture.clone(), r.rect, size),
            };
            self.sprites.insert(
                sprite_path,
                crate::sprite::Sprite {
                    name: r.name.clone(),
                    texture,
                    rect,
                    pivot: r.pivot,
                    pixels_per_unit: imp.pixels_per_unit,
                    border: r.border,
                    texture_size,
                },
            );
        }
    }

    /// Every sprite in the project: loaded ones, and those of texture files imported as
    /// Sprite (read from their `.meta`), for pickers.
    pub fn sprite_paths(&self) -> Vec<String> {
        let mut out: Vec<String> = self.sprites.iter().map(|(p, _)| p.to_owned()).collect();
        for p in self.list_files() {
            if self
                .types
                .for_path(&p)
                .is_none_or(|t| t.type_name != "Texture")
                || !std::fs::metadata(self.full_path(&format!("{p}.meta")))
                    .is_ok_and(|m| m.is_file())
            {
                continue;
            }
            let imp = self.texture_importer(&p);
            if imp.texture_type != crate::TextureImporterType::Sprite {
                continue;
            }
            match imp.sprite_mode {
                crate::sprite::SpriteImportMode::Single => {
                    let stem = Path::new(&p)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("Sprite");
                    out.push(format!("{p}#{stem}"));
                }
                crate::sprite::SpriteImportMode::Multiple => {
                    out.extend(imp.sprites.iter().map(|r| format!("{p}#{}", r.name)));
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }

    fn compressed_cached(
        &self,
        source: &[u8],
        imp: &crate::TextureImporter,
        format: crate::CompressedFormat,
        tex: &Texture2D,
    ) -> crate::CompressedMips {
        use crate::texture_compress::{compress_mips, level_size};
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        source.hash(&mut h);
        format!(
            "{imp:?} {format:?} {}x{} {} v1",
            tex.width,
            tex.height,
            tex.mips.len()
        )
        .hash(&mut h);
        let cache = self
            .full_path("Library/TextureCache")
            .join(format!("{:016x}.bin", h.finish()));
        let sizes: Vec<usize> = (0..tex.mips.len())
            .map(|i| level_size(format, (tex.width >> i).max(1), (tex.height >> i).max(1)))
            .collect();
        if let Ok(data) = std::fs::read(&cache)
            && data.len() == sizes.iter().sum::<usize>()
        {
            let mut mips = Vec::with_capacity(sizes.len());
            let mut at = 0;
            for s in sizes {
                mips.push(data[at..at + s].to_vec());
                at += s;
            }
            return crate::CompressedMips { format, mips };
        }
        let out = compress_mips(format, imp.compression, &tex.mips, tex.width, tex.height);
        if let Some(dir) = cache.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&cache, out.mips.concat());
        out
    }

    pub fn load_model(&mut self, path: &str) -> Result<Handle<Model>, String> {
        let bytes = self.read(path)?;
        let full = self.full_path(path);
        let base = full.parent().map(Path::to_path_buf);
        crate::gltf_import::import_gltf(self, path, &bytes, base.as_deref())
    }

    /// Loads (or reloads) a `.mat` material file (Unity's material asset).
    pub fn load_material(&mut self, path: &str) -> Result<Handle<Material>, String> {
        let bytes = self.read(path)?;
        let text = String::from_utf8(bytes).map_err(|e| format!("{path}: {e}"))?;
        let file: MaterialFile =
            serde_yaml_ng::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
        let mut m = Material {
            name: String::new(),
            ..Default::default()
        };
        for (k, v) in file.fields {
            visit_asset_refs(&v, &mut |p| {
                if let Err(e) = self.ensure_loaded(p) {
                    oxide_core::debug_log_warning!("{path}: {e}");
                }
            });
            if let Err(e) = m.set_field(&k, v) {
                oxide_core::debug_log_warning!("{path}: {k}: {e}");
            }
        }
        if m.name.is_empty() {
            m.name = Path::new(path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Material")
                .to_owned();
        }
        Ok(self.materials.insert(path.to_owned(), m))
    }

    /// Writes `material` to a `.mat` file at `path` and stores it under that path.
    pub fn save_material(
        &mut self,
        path: &str,
        material: &Material,
    ) -> Result<Handle<Material>, String> {
        let file = MaterialFile {
            shader: "Oxide/Lit".into(),
            fields: material
                .fields()
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v))
                .collect(),
        };
        let text = serde_yaml_ng::to_string(&file).map_err(|e| e.to_string())?;
        let full = self.full_path(path);
        if let Some(dir) = full.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        std::fs::write(&full, text).map_err(|e| format!("cannot write {}: {e}", full.display()))?;
        Ok(self.materials.insert(path.to_owned(), material.clone()))
    }

    /// Loads (or reloads) a `.wgsl` shader file.
    pub fn load_shader(&mut self, path: &str) -> Result<Handle<crate::shader::Shader>, String> {
        let source = String::from_utf8(self.read(path)?).map_err(|e| format!("{path}: {e}"))?;
        let name = Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Shader")
            .to_owned();
        Ok(self.shaders.insert(
            path.to_owned(),
            crate::shader::Shader {
                name,
                source,
                surface: None,
            },
        ))
    }

    /// Loads (or reloads) a `.renderTexture` file.
    pub fn load_render_texture(
        &mut self,
        path: &str,
    ) -> Result<Handle<crate::render_texture::RenderTexture>, String> {
        let text = String::from_utf8(self.read(path)?).map_err(|e| format!("{path}: {e}"))?;
        let fields: BTreeMap<String, Value> =
            serde_yaml_ng::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
        let mut rt = crate::render_texture::RenderTexture {
            name: Path::new(path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("RenderTexture")
                .to_owned(),
            ..Default::default()
        };
        for (k, v) in fields {
            if let Err(e) = rt.set_field(&k, v) {
                oxide_core::debug_log_warning!("{path}: {k}: {e}");
            }
        }
        rt.width = rt.width.clamp(1, 8192);
        rt.height = rt.height.clamp(1, 8192);
        Ok(self.render_textures.insert(path.to_owned(), rt))
    }

    /// Writes a `.renderTexture` file at `path` and stores it under that path.
    pub fn save_render_texture(
        &mut self,
        path: &str,
        rt: &crate::render_texture::RenderTexture,
    ) -> Result<Handle<crate::render_texture::RenderTexture>, String> {
        let fields: BTreeMap<String, Value> = rt
            .fields()
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect();
        let text = serde_yaml_ng::to_string(&fields).map_err(|e| e.to_string())?;
        let full = self.full_path(path);
        if let Some(dir) = full.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        std::fs::write(&full, text).map_err(|e| format!("cannot write {}: {e}", full.display()))?;
        Ok(self.render_textures.insert(path.to_owned(), rt.clone()))
    }

    pub fn load_audio(&mut self, path: &str) -> Result<Handle<AudioClip>, String> {
        let bytes = self.read(path)?;
        let name = Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("clip")
            .to_owned();
        Ok(self.audio.insert(
            path.to_owned(),
            AudioClip {
                name,
                bytes: bytes.into(),
            },
        ))
    }

    /// Makes sure the asset behind `path` is loaded (no-op for built-ins/runtime assets
    /// and assets already in memory). Returns what kind of asset it is.
    pub fn ensure_loaded(&mut self, path: &str) -> Result<(), String> {
        let file = path.split('#').next().unwrap_or(path);
        if file.starts_with("builtin:") || file.starts_with("runtime:") {
            return Ok(());
        }
        let ext = Path::new(file)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match ext.as_str() {
            "png" | "jpg" | "jpeg" | "tga" | "bmp" | "hdr" | "ktx2" => {
                if self.textures.handle_of(file).is_none() {
                    self.load_texture(file)?;
                }
            }
            "glb" | "gltf" => {
                if self.models.handle_of(file).is_none() {
                    self.load_model(file)?;
                }
            }
            "mat" => {
                if self.materials.handle_of(file).is_none() {
                    self.load_material(file)?;
                }
            }
            "wgsl" => {
                if self.shaders.handle_of(file).is_none() {
                    self.load_shader(file)?;
                }
            }
            "rendertexture" => {
                if self.render_textures.handle_of(file).is_none() {
                    self.load_render_texture(file)?;
                }
            }
            "wav" | "ogg" | "mp3" | "flac" => {
                if self.audio.handle_of(file).is_none() {
                    self.load_audio(file)?;
                }
            }
            "anim" => {
                if self.animations.handle_of(file).is_none() {
                    self.load_animation_clip(file)?;
                }
            }
            "controller" => {
                if self.controllers.handle_of(file).is_none() {
                    self.load_animator_controller(file)?;
                }
            }
            "terraindata" => {
                if self.terrains.handle_of(file).is_none() {
                    self.load_terrain_data(file)?;
                }
            }
            // Font files are read by the font library when a Font Asset is made.
            "ttf" | "otf" => {
                if !self.full_path(file).exists() {
                    return Err(format!("{file} not found"));
                }
            }
            "overridecontroller" => {
                if self.controllers.handle_of(file).is_none() {
                    self.load_override_controller(file)?;
                }
            }
            "mask" => {
                if self.masks.handle_of(file).is_none() {
                    self.load_avatar_mask(file)?;
                }
            }
            _ if self
                .types
                .for_path(file)
                .is_some_and(|t| self.types.is_data(t.type_name)) =>
            {
                if !self.data.contains(file) {
                    self.load_data(file)?;
                }
            }
            // Text formats read by their own systems (UXML, USS): the file must exist.
            _ if self.types.for_path(file).is_some() => {
                if !self.full_path(file).exists() {
                    return Err(format!("{file} not found"));
                }
            }
            _ => return Err(format!("unsupported asset type: {path}")),
        }
        Ok(())
    }

    /// Asset paths under `Assets/` with a known extension (the Project window's contents).
    pub fn list_files(&self) -> Vec<String> {
        let mut out = Vec::new();
        let root = self.root.join("Assets");
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else {
                continue;
            };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if let Ok(rel) = p.strip_prefix(&self.root) {
                    out.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
        out.sort();
        out
    }
}

/// Calls `f` with every asset path referenced anywhere in `v` (nested objects and lists).
pub(crate) fn visit_asset_refs(v: &Value, f: &mut dyn FnMut(&str)) {
    match v {
        Value::Asset(p) => f(p),
        Value::List(items) => items.iter().for_each(|i| visit_asset_refs(i, f)),
        Value::Object(m) => m.values().for_each(|i| visit_asset_refs(i, f)),
        _ => {}
    }
}

/// On-disk form of a `.mat` file: reflected fields by name.
#[derive(serde::Serialize, serde::Deserialize)]
struct MaterialFile {
    #[serde(default)]
    shader: String,
    fields: BTreeMap<String, Value>,
}

#[cfg(test)]
mod material_file_tests {
    use super::*;

    #[test]
    fn material_round_trips_through_a_file() {
        let dir = std::env::temp_dir().join(format!("oxide-mat-{}", std::process::id()));
        let mut a = AssetServer::new(&dir);
        let mut m = Material::lit("Red Paint", Color::RED);
        m.metallic = 0.7;
        m.base_map = Some(Handle::from_path("builtin:grid"));
        a.save_material("Assets/Red Paint.mat", &m).unwrap();

        let mut b = AssetServer::new(&dir);
        let h = b.load_material("Assets/Red Paint.mat").unwrap();
        let back = b.materials.get(&h).unwrap();
        assert_eq!(back.name, "Red Paint");
        assert_eq!(back.base_color, Color::RED);
        assert_eq!(back.metallic, 0.7);
        assert_eq!(
            back.base_map.as_ref().map(|h| h.path()),
            Some("builtin:grid")
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod reload_tests {
    use super::*;

    #[test]
    fn edited_files_are_reloaded() {
        let dir = std::env::temp_dir().join(format!("oxide-reload-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Assets")).unwrap();
        let file = dir.join("Assets/fx.wgsl");
        std::fs::write(&file, "// v1").unwrap();
        let mut a = AssetServer::new(&dir);
        let h = a.load_shader("Assets/fx.wgsl").unwrap();
        assert!(a.reload_changed().is_empty(), "nothing changed yet");

        std::fs::write(&file, "// v2").unwrap();
        // Make sure the timestamp moves even on coarse file systems.
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(later)
            .unwrap();
        assert_eq!(a.reload_changed(), vec!["Assets/fx.wgsl".to_owned()]);
        assert_eq!(a.shaders.get(&h).unwrap().source, "// v2");
        assert!(a.reload_changed().is_empty(), "reloaded once");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn labels_live_in_meta_beside_import_settings() {
        let dir = std::env::temp_dir().join(format!("oxide-labels-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Assets")).unwrap();
        let png = image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 255]));
        png.save(dir.join("Assets/red.png")).unwrap();
        let mut a = AssetServer::new(&dir);
        assert!(a.asset_labels("Assets/red.png").is_empty());
        a.set_asset_labels(
            "Assets/red.png",
            &[
                "Vehicle".into(),
                "hero".into(),
                "vehicle".into(),
                " ".into(),
            ],
        )
        .unwrap();
        assert_eq!(a.asset_labels("Assets/red.png"), vec!["hero", "Vehicle"]);
        // Changing import settings keeps the labels, and labels don't upset the importer.
        let mut imp = a.texture_importer("Assets/red.png");
        imp.max_size = 2;
        a.set_texture_importer("Assets/red.png", &imp).unwrap();
        assert_eq!(a.asset_labels("Assets/red.png"), vec!["hero", "Vehicle"]);
        assert_eq!(a.texture_importer("Assets/red.png").max_size, 2);
        // Removing every label (and having no other settings) removes the .meta file.
        a.set_asset_labels("Assets/x.mat", &["A".into()]).unwrap();
        assert!(dir.join("Assets/x.mat.meta").is_file());
        a.set_asset_labels("Assets/x.mat", &[]).unwrap();
        assert!(!dir.join("Assets/x.mat.meta").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
