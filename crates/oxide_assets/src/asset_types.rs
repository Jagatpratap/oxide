//! The asset type registry and data assets.
//!
//! Every kind of file the Project window knows is described once by an [`AssetTypeInfo`]:
//! its Unity type name, file extensions, Create menu path, default file name and icon
//! colour. The Project window, Create menu, asset picker, loader and MCP tools all read this
//! table, so a new asset type is one registration.
//!
//! **Data assets** are Unity's ScriptableObjects: any reflected Rust struct registered with
//! [`AssetServer::register_data_asset`] becomes a file asset (YAML of its fields), loaded
//! into [`DataAssets`] and edited with the generic Inspector. Physics Materials, Volume
//! Profiles, Input Actions and game-defined data (weapon stats, car setups) all use it.
//! Reference: <https://docs.unity3d.com/Manual/class-ScriptableObject.html>

use crate::handle::{Asset, AssetId, Handle};
use crate::server::AssetServer;
use oxide_core::reflect::{Reflect, Value};
use std::any::Any;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;

/// How the editor creates a new asset of this type from the Create menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreateKind {
    /// Not created from the Create menu (imported files: textures, models, audio).
    None,
    /// A data asset: written from the type's default value.
    Data,
    /// The editor has its own code for it (folders, scripts, materials, scenes, ...).
    Custom,
}

/// One kind of asset file (Unity's asset types as the Project window shows them).
#[derive(Clone, Debug)]
pub struct AssetTypeInfo {
    /// Unity-facing type name (`Material`, `PhysicsMaterial`, `VolumeProfile`).
    pub type_name: &'static str,
    /// File extensions without the dot, lowercase; the first is used for new files.
    pub extensions: &'static [&'static str],
    /// Create menu path (`Physics Material`, `Rendering/Volume Profile`), if creatable.
    pub create_menu: Option<&'static str>,
    pub create: CreateKind,
    /// File name (without extension) for a new asset.
    pub default_name: &'static str,
    /// Icon colour in the Project window.
    pub color: [u8; 3],
    /// Create menu section; items sort by (order, path).
    pub order: i32,
}

impl AssetTypeInfo {
    pub const fn new(type_name: &'static str, extensions: &'static [&'static str]) -> Self {
        Self {
            type_name,
            extensions,
            create_menu: None,
            create: CreateKind::None,
            default_name: "New Asset",
            color: [170, 170, 170],
            order: 100,
        }
    }

    pub const fn color(mut self, r: u8, g: u8, b: u8) -> Self {
        self.color = [r, g, b];
        self
    }

    /// Creatable from the Create menu at `path`, named `default_name`.
    pub const fn menu(
        mut self,
        path: &'static str,
        default_name: &'static str,
        order: i32,
    ) -> Self {
        self.create_menu = Some(path);
        self.default_name = default_name;
        self.order = order;
        if matches!(self.create, CreateKind::None) {
            self.create = CreateKind::Custom;
        }
        self
    }

    const fn data(mut self) -> Self {
        self.create = CreateKind::Data;
        self
    }

    /// Default file name with extension (`New Physics Material.physicMaterial`).
    pub fn default_file_name(&self) -> String {
        format!(
            "{}.{}",
            self.default_name,
            self.extensions.first().copied().unwrap_or("asset")
        )
    }
}

/// A loaded data asset value (any reflected, clonable type).
pub trait DataAsset: Reflect + Any + Send + Sync {
    fn clone_box(&self) -> Box<dyn DataAsset>;
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn as_reflect(&self) -> &dyn Reflect;
    fn as_reflect_mut(&mut self) -> &mut dyn Reflect;
}

impl<T: Reflect + Clone + Any + Send + Sync> DataAsset for T {
    fn clone_box(&self) -> Box<dyn DataAsset> {
        Box::new(self.clone())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn as_reflect(&self) -> &dyn Reflect {
        self
    }
    fn as_reflect_mut(&mut self) -> &mut dyn Reflect {
        self
    }
}

struct DataType {
    info: AssetTypeInfo,
    create: fn() -> Box<dyn DataAsset>,
}

/// Every known asset type, built-in and registered.
#[derive(Default)]
pub struct AssetTypeRegistry {
    types: Vec<AssetTypeInfo>,
    data: HashMap<&'static str, DataType>,
    generation: u64,
}

impl AssetTypeRegistry {
    /// Adds (or replaces, by type name) a type.
    pub fn register(&mut self, info: AssetTypeInfo) {
        self.types.retain(|t| t.type_name != info.type_name);
        self.types.push(info);
        self.generation += 1;
    }

    /// Changes whenever a type is added or removed (for caches of this table).
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Removes a type (a script's data asset when its library unloads).
    pub fn unregister(&mut self, type_name: &str) {
        self.types.retain(|t| t.type_name != type_name);
        self.data.remove(type_name);
        self.generation += 1;
    }

    pub fn iter(&self) -> impl Iterator<Item = &AssetTypeInfo> {
        self.types.iter()
    }

    pub fn get(&self, type_name: &str) -> Option<&AssetTypeInfo> {
        self.types.iter().find(|t| t.type_name == type_name)
    }

    /// The type of a file by its extension (case-insensitive).
    pub fn for_path(&self, path: &str) -> Option<&AssetTypeInfo> {
        let file = path.split('#').next().unwrap_or(path);
        let ext = Path::new(file).extension()?.to_str()?.to_ascii_lowercase();
        self.types
            .iter()
            .find(|t| t.extensions.iter().any(|e| e.eq_ignore_ascii_case(&ext)))
    }

    pub fn is_data(&self, type_name: &str) -> bool {
        self.data.contains_key(type_name)
    }

    /// A new default value of a registered data asset type.
    pub fn create_data(&self, type_name: &str) -> Option<Box<dyn DataAsset>> {
        self.data.get(type_name).map(|d| (d.create)())
    }

    /// Creatable types sorted for the Create menu.
    pub fn create_menu(&self) -> Vec<&AssetTypeInfo> {
        let mut v: Vec<&AssetTypeInfo> = self
            .types
            .iter()
            .filter(|t| t.create_menu.is_some() && t.create != CreateKind::None)
            .collect();
        v.sort_by(|a, b| (a.order, a.create_menu).cmp(&(b.order, b.create_menu)));
        v
    }
}

struct DataEntry {
    path: Arc<str>,
    type_name: &'static str,
    value: Box<dyn DataAsset>,
    version: u64,
}

/// Loaded data assets of every registered type, by path.
#[derive(Default)]
pub struct DataAssets {
    entries: HashMap<AssetId, DataEntry>,
}

impl DataAssets {
    pub fn get<T: Any>(&self, h: &Handle<T>) -> Option<&T> {
        self.entries.get(&h.id())?.value.as_any().downcast_ref()
    }

    /// Mutable access; bumps the version.
    pub fn get_mut<T: Any>(&mut self, h: &Handle<T>) -> Option<&mut T> {
        let e = self.entries.get_mut(&h.id())?;
        e.version += 1;
        e.value.as_any_mut().downcast_mut()
    }

    pub fn get_dyn(&self, path: &str) -> Option<&dyn DataAsset> {
        self.entries
            .get(&AssetId::from_path(path))
            .map(|e| &*e.value)
    }

    pub fn get_dyn_mut(&mut self, path: &str) -> Option<&mut dyn DataAsset> {
        let e = self.entries.get_mut(&AssetId::from_path(path))?;
        e.version += 1;
        Some(&mut *e.value)
    }

    pub fn type_of(&self, path: &str) -> Option<&'static str> {
        self.entries
            .get(&AssetId::from_path(path))
            .map(|e| e.type_name)
    }

    pub fn version(&self, path: &str) -> Option<u64> {
        self.entries
            .get(&AssetId::from_path(path))
            .map(|e| e.version)
    }

    pub fn contains(&self, path: &str) -> bool {
        self.entries.contains_key(&AssetId::from_path(path))
    }

    pub fn insert(&mut self, path: &str, type_name: &'static str, value: Box<dyn DataAsset>) {
        let id = AssetId::from_path(path);
        let version = self.entries.get(&id).map_or(1, |e| e.version + 1);
        self.entries.insert(
            id,
            DataEntry {
                path: path.into(),
                type_name,
                value,
                version,
            },
        );
    }

    pub fn remove(&mut self, path: &str) -> bool {
        self.entries.remove(&AssetId::from_path(path)).is_some()
    }

    /// Drops every asset of a type (its script library is unloading).
    pub fn remove_type(&mut self, type_name: &str) -> Vec<String> {
        let gone: Vec<AssetId> = self
            .entries
            .iter()
            .filter(|(_, e)| e.type_name == type_name)
            .map(|(k, _)| *k)
            .collect();
        gone.into_iter()
            .filter_map(|k| self.entries.remove(&k).map(|e| e.path.to_string()))
            .collect()
    }

    /// `(path, type name)` of every loaded data asset.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &'static str)> {
        self.entries.values().map(|e| (&*e.path, e.type_name))
    }
}

/// On-disk form of a data asset: its type and reflected fields.
#[derive(serde::Serialize, serde::Deserialize)]
struct DataFile {
    #[serde(rename = "type")]
    type_name: String,
    #[serde(default)]
    fields: BTreeMap<String, Value>,
}

/// The built-in asset types (files the engine handles itself).
pub fn builtin_types() -> Vec<AssetTypeInfo> {
    vec![
        AssetTypeInfo::new("Folder", &[])
            .menu("Folder", "New Folder", 0)
            .color(220, 200, 120),
        AssetTypeInfo::new("Script", &["rs"])
            .menu("Rust Script", "NewScript", 10)
            .color(240, 120, 90),
        AssetTypeInfo::new("Scene", &["scene"])
            .menu("Scene", "New Scene", 20)
            .color(120, 200, 255),
        AssetTypeInfo::new("Prefab", &["prefab"]).color(140, 180, 255),
        AssetTypeInfo::new("Material", &["mat"])
            .menu("Material", "New Material", 30)
            .color(200, 160, 255),
        AssetTypeInfo::new("Shader", &["wgsl"])
            .menu("Shader/Full Screen Shader", "NewShader", 32)
            .color(160, 210, 230),
        AssetTypeInfo::new("RenderTexture", &["rendertexture"])
            .menu("Rendering/Render Texture", "New Render Texture", 60)
            .color(230, 190, 110),
        AssetTypeInfo::new(
            "Texture",
            &["png", "jpg", "jpeg", "tga", "bmp", "hdr", "exr", "ktx2"],
        )
        .color(230, 190, 110),
        AssetTypeInfo::new("Model", &["glb", "gltf", "fbx", "obj"]).color(120, 220, 160),
        AssetTypeInfo::new("AudioClip", &["wav", "ogg", "mp3", "flac"]).color(230, 140, 200),
        AssetTypeInfo::new("AnimationClip", &["anim"])
            .menu("Animation/Animation Clip", "New Animation", 70)
            .color(200, 200, 120),
        AssetTypeInfo::new("AnimatorController", &["controller"])
            .menu(
                "Animation/Animator Controller",
                "New Animator Controller",
                71,
            )
            .color(210, 170, 110),
        AssetTypeInfo::new("AnimatorOverrideController", &["overrideController"])
            .menu(
                "Animation/Animator Override Controller",
                "New Animator Override Controller",
                72,
            )
            .color(210, 150, 110),
        AssetTypeInfo::new("AvatarMask", &["mask"])
            .menu("Animation/Avatar Mask", "New Avatar Mask", 73)
            .color(160, 200, 200),
        AssetTypeInfo::new(
            "TextAsset",
            &["txt", "json", "md", "csv", "xml", "yaml", "bytes"],
        )
        .menu("Text Asset", "New Text", 90)
        .color(190, 190, 190),
    ]
}

impl AssetServer {
    /// Registers `T` as a data asset type (Unity's `[CreateAssetMenu]` ScriptableObject):
    /// files with `info`'s extension load as `T`, and the Create menu can make one.
    pub fn register_data_asset<T>(&mut self, info: AssetTypeInfo)
    where
        T: Reflect + Clone + Default + Any + Send + Sync,
    {
        let info = info.data();
        self.types.data.insert(
            info.type_name,
            DataType {
                info: info.clone(),
                create: || Box::new(T::default()),
            },
        );
        self.types.register(info);
    }

    /// Loads (or reloads) a data asset file.
    pub fn load_data(&mut self, path: &str) -> Result<(), String> {
        let text = String::from_utf8(self.read_file(path)?).map_err(|e| format!("{path}: {e}"))?;
        let file: DataFile = serde_yaml_ng::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
        // The file's `type` wins; the extension decides for hand-made files without one.
        let type_name = self
            .types
            .data
            .get(file.type_name.as_str())
            .map(|d| d.info.type_name)
            .or_else(|| self.types.for_path(path).map(|t| t.type_name))
            .ok_or_else(|| format!("{path}: unknown asset type `{}`", file.type_name))?;
        let mut value = self
            .types
            .create_data(type_name)
            .ok_or_else(|| format!("{path}: `{type_name}` is not a data asset type"))?;
        for (k, v) in file.fields {
            crate::server::visit_asset_refs(&v, &mut |p| {
                if let Err(e) = self.ensure_loaded(p) {
                    oxide_core::debug_log_warning!("{path}: {e}");
                }
            });
            if let Err(e) = value.as_reflect_mut().set_field(&k, v) {
                oxide_core::debug_log_warning!("{path}: {k}: {e}");
            }
        }
        self.data.insert(path, type_name, value);
        Ok(())
    }

    /// Writes a data asset to `path` and stores it there.
    pub fn save_data(&mut self, path: &str, value: Box<dyn DataAsset>) -> Result<(), String> {
        let type_name = self
            .types
            .iter()
            .find(|t| t.type_name == value.as_reflect().type_info().name)
            .map(|t| t.type_name)
            .or_else(|| self.types.for_path(path).map(|t| t.type_name))
            .ok_or_else(|| format!("{path}: unknown asset type"))?;
        let file = DataFile {
            type_name: type_name.to_owned(),
            fields: value
                .as_reflect()
                .fields()
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v))
                .collect(),
        };
        let text = serde_yaml_ng::to_string(&file).map_err(|e| e.to_string())?;
        self.write_file(path, &text)?;
        self.data.insert(path, type_name, value);
        Ok(())
    }

    /// Saves the in-memory data asset at `path` back to its file.
    pub fn save_data_in_place(&mut self, path: &str) -> Result<(), String> {
        let value = self
            .data
            .get_dyn(path)
            .ok_or_else(|| format!("{path} is not loaded"))?
            .clone_box();
        self.save_data(path, value)
    }

    /// Creates a new data asset of `type_name` with default values at `path`.
    pub fn create_data_asset(&mut self, type_name: &str, path: &str) -> Result<(), String> {
        let value = self
            .types
            .create_data(type_name)
            .ok_or_else(|| format!("`{type_name}` is not a data asset type"))?;
        self.save_data(path, value)
    }

    /// A typed data asset, loading its file on first use.
    pub fn data_asset<T: Asset + Any>(&mut self, h: &Handle<T>) -> Option<&T> {
        if !self.data.contains(h.path())
            && let Err(e) = self.load_data(h.path())
        {
            oxide_core::debug_log_warning!("{e}");
        }
        self.data.get(h)
    }

    /// Every asset path of `type_name`: loaded ones and matching files under `Assets/`
    /// (for asset pickers, like Unity's object picker listing the whole project).
    pub fn paths_of_type(&self, type_name: &str) -> Vec<String> {
        let mut out: Vec<String> = self
            .data
            .iter()
            .filter(|(_, t)| *t == type_name)
            .map(|(p, _)| p.to_owned())
            .collect();
        if let Some(info) = self.types.get(type_name)
            && !info.extensions.is_empty()
        {
            out.extend(self.list_files().into_iter().filter(|p| {
                self.types
                    .for_path(p)
                    .is_some_and(|t| t.type_name == type_name)
            }));
        }
        out.sort();
        out.dedup();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxide_core::Reflect;

    #[derive(Reflect, Clone, Debug, PartialEq)]
    struct WeaponData {
        damage: f32,
        rounds: i32,
        name: String,
        recoil: oxide_core::AnimationCurve,
    }

    impl Default for WeaponData {
        fn default() -> Self {
            Self {
                damage: 25.0,
                rounds: 30,
                name: "Rifle".into(),
                recoil: oxide_core::AnimationCurve::linear(0.0, 0.0, 1.0, 1.0),
            }
        }
    }

    impl Asset for WeaponData {
        const TYPE_NAME: &'static str = "WeaponData";
    }

    fn server(tag: &str) -> (AssetServer, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("oxide-data-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Assets")).unwrap();
        let mut a = AssetServer::new(&dir);
        a.register_data_asset::<WeaponData>(AssetTypeInfo::new("WeaponData", &["weapon"]).menu(
            "Game/Weapon Data",
            "New Weapon",
            200,
        ));
        (a, dir)
    }

    #[test]
    fn create_edit_save_and_reload() {
        let (mut a, dir) = server("rt");
        a.create_data_asset("WeaponData", "Assets/AK.weapon")
            .unwrap();
        let h: Handle<WeaponData> = Handle::from_path("Assets/AK.weapon");
        assert_eq!(a.data.get(&h).unwrap().rounds, 30);
        a.data.get_mut(&h).unwrap().damage = 47.5;
        a.data.get_mut(&h).unwrap().recoil.add_key_at(0.5);
        a.save_data_in_place("Assets/AK.weapon").unwrap();

        let (mut b, _) = (AssetServer::new(&dir), ());
        b.register_data_asset::<WeaponData>(AssetTypeInfo::new("WeaponData", &["weapon"]));
        let w = b.data_asset(&h).expect("loads on first use").clone();
        assert_eq!(w.damage, 47.5);
        assert_eq!(w.recoil.len(), 3, "nested curve keys survive");
        assert_eq!(
            b.paths_of_type("WeaponData"),
            vec!["Assets/AK.weapon".to_owned()]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn registry_finds_types_by_extension_and_menu() {
        let (a, dir) = server("reg");
        assert_eq!(
            a.types.for_path("Assets/x.WEAPON").unwrap().type_name,
            "WeaponData"
        );
        assert_eq!(
            a.types.for_path("Assets/car.glb#mesh0").unwrap().type_name,
            "Model"
        );
        assert!(a.types.is_data("WeaponData"));
        let menu: Vec<_> = a
            .types
            .create_menu()
            .iter()
            .map(|t| t.create_menu.unwrap())
            .collect();
        assert_eq!(menu.first(), Some(&"Folder"));
        assert!(menu.contains(&"Game/Weapon Data"));
        assert_eq!(
            a.types.get("WeaponData").unwrap().default_file_name(),
            "New Weapon.weapon"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn unknown_fields_warn_but_load() {
        let (mut a, dir) = server("bad");
        std::fs::write(
            dir.join("Assets/old.weapon"),
            "type: WeaponData\nfields:\n  damage: !Float 9.0\n  removed_field: !Int 3\n",
        )
        .unwrap();
        a.load_data("Assets/old.weapon").unwrap();
        let h: Handle<WeaponData> = Handle::from_path("Assets/old.weapon");
        assert_eq!(a.data.get(&h).unwrap().damage, 9.0);
        let _ = std::fs::remove_dir_all(dir);
    }
}
