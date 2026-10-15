//! The asset database: typed stores plus loading from the project's `Assets/` folder.
//! Reference: <https://docs.unity3d.com/Manual/AssetWorkflow.html>

use crate::animation::AnimationClip;
use crate::handle::{Asset, Assets, Handle};
use crate::material::Material;
use crate::mesh::Mesh;
use crate::model::Model;
use crate::primitives::PrimitiveType;
use crate::texture::Texture2D;
use oxide_math::Color;
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

pub struct AssetServer {
    /// Project folder; asset paths like `Assets/x.png` are relative to it.
    pub root: PathBuf,
    pub meshes: Assets<Mesh>,
    pub materials: Assets<Material>,
    pub textures: Assets<Texture2D>,
    pub models: Assets<Model>,
    pub animations: Assets<AnimationClip>,
    pub audio: Assets<AudioClip>,
}

pub const DEFAULT_MATERIAL: &str = "builtin:Default-Material";

impl AssetServer {
    /// Creates the stores and registers the built-in assets (primitive meshes, default
    /// material, white/black/normal textures).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let mut s = Self {
            root: root.into(),
            meshes: Assets::default(),
            materials: Assets::default(),
            textures: Assets::default(),
            models: Assets::default(),
            animations: Assets::default(),
            audio: Assets::default(),
        };
        for p in PrimitiveType::ALL {
            s.meshes.insert(p.mesh_path(), p.mesh());
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

    pub fn full_path(&self, path: &str) -> PathBuf {
        let p = Path::new(path);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.root.join(p)
        }
    }

    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        let full = self.full_path(path);
        std::fs::read(&full).map_err(|e| format!("cannot read {}: {e}", full.display()))
    }

    /// Loads (or reloads) a texture. Data maps (normal, mask, roughness, metallic) are linear.
    pub fn load_texture(&mut self, path: &str) -> Result<Handle<Texture2D>, String> {
        let bytes = self.read(path)?;
        let lower = path.to_ascii_lowercase();
        let linear = ["normal", "_n.", "mask", "rough", "metal", "_orm", "height"]
            .iter()
            .any(|k| lower.contains(k));
        let name = Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("texture");
        let tex = Texture2D::decode(name, &bytes, !linear)?;
        Ok(self.textures.insert(path.to_owned(), tex))
    }

    pub fn load_model(&mut self, path: &str) -> Result<Handle<Model>, String> {
        let bytes = self.read(path)?;
        let full = self.full_path(path);
        let base = full.parent().map(Path::to_path_buf);
        crate::gltf_import::import_gltf(self, path, &bytes, base.as_deref())
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
            "png" | "jpg" | "jpeg" | "tga" | "bmp" | "hdr" => {
                if self.textures.handle_of(file).is_none() {
                    self.load_texture(file)?;
                }
            }
            "glb" | "gltf" => {
                if self.models.handle_of(file).is_none() {
                    self.load_model(file)?;
                }
            }
            "wav" | "ogg" | "mp3" | "flac" => {
                if self.audio.handle_of(file).is_none() {
                    self.load_audio(file)?;
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
