//! Turns downloaded BlenderKit files into project assets under `Assets/BlenderKit/`:
//! models (the server's glTF export) placed in the scene, materials (textures read out of the
//! `.blend` and wired into an Oxide Lit material) applied to the selection, and HDRIs made
//! into a Panoramic skybox. Each asset folder also gets `BlenderKit.json` with its author and
//! license, for attribution.

use crate::api::{Asset, AssetType};
use crate::blend::{BlendFile, PackedImage, TextureRole, guess_role};
use oxide::assets::deps::image;
use oxide::assets::deps::serde_json::json;
use oxide::assets::texture::TextureImporterType;
use oxide::assets::{AssetServer, Material, ShaderKind, TextureImporter};
use oxide::math::Color;
use oxide::scene::{GameObjectId, MeshRenderer, RenderSettings, SkinnedMeshRenderer, World};

/// A file or folder name from an asset name (`Oak Wood #2` → `Oak_Wood_2`).
pub fn safe_name(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
            out.push(c);
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let out = out.trim_matches(['_', '.']).to_owned();
    if out.is_empty() { "Asset".into() } else { out }
}

/// `Assets/BlenderKit/<Kind>/<Name>` (asset-relative) for an asset.
pub fn asset_folder(asset: &Asset, kind: AssetType) -> String {
    format!(
        "Assets/BlenderKit/{}/{}",
        kind.folder(),
        safe_name(&asset.name)
    )
}

/// Writes a project file (asset-relative path).
fn write(assets: &AssetServer, path: &str, bytes: &[u8]) -> Result<(), String> {
    let full = assets.full_path(path);
    if let Some(d) = full.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    std::fs::write(&full, bytes).map_err(|e| format!("{}: {e}", full.display()))
}

/// Records where the asset came from (BlenderKit's licenses ask for nothing more, but it
/// keeps the author and link with the files).
fn write_info(assets: &AssetServer, folder: &str, asset: &Asset, file: &str) -> Result<(), String> {
    let info = json!({
        "name": asset.name,
        "id": asset.id,
        "assetBaseId": asset.base_id,
        "author": asset.author,
        "license": asset.license_label(),
        "free": asset.is_free,
        "url": format!("https://www.blenderkit.com/get-blenderkit/{}/", asset.base_id),
        "file": file,
    });
    let text =
        oxide::assets::deps::serde_json::to_string_pretty(&info).map_err(|e| e.to_string())?;
    write(
        assets,
        &format!("{folder}/BlenderKit.json"),
        text.as_bytes(),
    )
}

/// Saves a model's `.glb` and imports it. Returns its asset path.
pub fn import_model(world: &mut World, asset: &Asset, glb: &[u8]) -> Result<String, String> {
    let folder = asset_folder(asset, AssetType::Model);
    let path = format!("{folder}/{}.glb", safe_name(&asset.name));
    let assets = world.resource_mut::<AssetServer>();
    write(assets, &path, glb)?;
    write_info(assets, &folder, asset, &path)?;
    assets.load_model(&path)?;
    assets.database.refresh();
    Ok(path)
}

/// Instantiates an imported model at `position` and returns its root.
pub fn place_model(
    world: &mut World,
    path: &str,
    position: oxide::math::Vector3,
) -> Result<GameObjectId, String> {
    let assets = world.resource_mut::<AssetServer>();
    let model = match assets.models.handle_of(path) {
        Some(h) => h,
        None => assets.load_model(path)?,
    };
    let root = oxide::objects::instantiate_model(world, &model)
        .ok_or_else(|| format!("{path}: no model"))?;
    if let Some(t) = world.transform_mut(root) {
        t.local_position = position;
    }
    Ok(root)
}

/// The textures chosen for each role (the first per role; a name that agrees wins).
fn pick_textures(
    file: &BlendFile,
    images: &[PackedImage],
) -> (
    String,
    Vec<(TextureRole, PackedImage)>,
    Option<crate::blend::MaterialInfo>,
) {
    let materials = file.materials();
    // The material with the most textures is the asset's material.
    let main = materials.iter().max_by_key(|m| m.textures.len()).cloned();
    let mut roles: Vec<(TextureRole, String)> = Vec::new();
    if let Some(m) = &main {
        let mut sorted = m.textures.clone();
        // Textures whose names agree with their traced role first.
        sorted.sort_by_key(|t| u8::from(guess_role(&t.image) != t.role));
        for t in sorted {
            if let Some(r) = t.role
                && !roles.iter().any(|(q, _)| *q == r)
            {
                roles.push((r, t.image.clone()));
            }
        }
    }
    // Packed images no node uses: go by name.
    for i in images {
        if let Some(r) = guess_role(&i.name)
            && !roles.iter().any(|(q, _)| *q == r)
            && main.as_ref().is_none_or(|m| m.textures.is_empty())
        {
            roles.push((r, i.name.clone()));
        }
    }
    let picked = roles
        .into_iter()
        .filter_map(|(r, name)| {
            images
                .iter()
                .find(|i| i.name == name)
                .map(|i| (r, i.clone()))
        })
        .collect();
    let name = main
        .as_ref()
        .map(|m| m.name.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_default();
    (name, picked, main)
}

/// The image's file extension from its bytes (falls back to its name's).
fn image_extension(i: &PackedImage) -> String {
    match image::guess_format(&i.data) {
        Ok(image::ImageFormat::Png) => "png".into(),
        Ok(image::ImageFormat::Jpeg) => "jpg".into(),
        Ok(image::ImageFormat::OpenExr) => "exr".into(),
        Ok(image::ImageFormat::Hdr) => "hdr".into(),
        Ok(image::ImageFormat::Tga) => "tga".into(),
        Ok(image::ImageFormat::Tiff) => "tif".into(),
        Ok(image::ImageFormat::Bmp) => "bmp".into(),
        Ok(image::ImageFormat::WebP) => "webp".into(),
        _ => i
            .filepath
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_else(|| "png".into()),
    }
}

/// Packs roughness (G) and metallic (B) maps into one texture, glTF style, as Oxide's Lit
/// shader reads `metallic_gloss_map`. Missing maps use the constants.
#[doc(hidden)]
pub fn pack_metal_rough(
    rough: Option<&PackedImage>,
    smooth: Option<&PackedImage>,
    metal: Option<&PackedImage>,
    roughness: f32,
    metallic: f32,
) -> Result<Vec<u8>, String> {
    let decode = |i: &PackedImage| {
        image::load_from_memory(&i.data)
            .map(|d| d.to_luma8())
            .map_err(|e| format!("{}: {e}", i.name))
    };
    let r = rough.map(decode).transpose()?;
    let s = smooth.map(decode).transpose()?;
    let m = metal.map(decode).transpose()?;
    let (w, h) = [&r, &s, &m]
        .iter()
        .filter_map(|x| x.as_ref().map(|i| i.dimensions()))
        .max_by_key(|(w, h)| w * h)
        .unwrap_or((4, 4));
    let resize = |i: Option<image::GrayImage>| {
        i.map(|i| {
            if i.dimensions() == (w, h) {
                i
            } else {
                image::imageops::resize(&i, w, h, image::imageops::FilterType::Triangle)
            }
        })
    };
    let (r, s, m) = (resize(r), resize(s), resize(m));
    let out = image::RgbaImage::from_fn(w, h, |x, y| {
        let g = match (&r, &s) {
            (Some(r), _) => r.get_pixel(x, y)[0],
            (None, Some(s)) => 255 - s.get_pixel(x, y)[0],
            _ => (roughness.clamp(0.0, 1.0) * 255.0).round() as u8,
        };
        let b = match &m {
            Some(m) => m.get_pixel(x, y)[0],
            None => (metallic.clamp(0.0, 1.0) * 255.0).round() as u8,
        };
        image::Rgba([255, g, b, 255])
    });
    let mut png = Vec::new();
    out.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(png)
}

/// What a material import made.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportedMaterial {
    pub path: String,
    /// (role, texture path) for each texture wired in.
    pub textures: Vec<(String, String)>,
}

/// Reads a material `.blend`, saves its textures and an Oxide Lit `.mat`. Returns the
/// material's asset path.
pub fn import_material(
    world: &mut World,
    asset: &Asset,
    blend: &[u8],
) -> Result<ImportedMaterial, String> {
    let file = BlendFile::parse(blend)?;
    let images = file.packed_images();
    let (_, picked, info) = pick_textures(&file, &images);
    let folder = asset_folder(asset, AssetType::Material);
    let base = safe_name(&asset.name);
    let assets = world.resource_mut::<AssetServer>();
    let get = |r: TextureRole| picked.iter().find(|(q, _)| *q == r).map(|(_, i)| i);
    let mut textures = Vec::new();
    let mut save = |assets: &mut AssetServer,
                    role: &str,
                    i: &PackedImage,
                    normal: bool|
     -> Result<String, String> {
        let path = format!("{folder}/Textures/{base}_{role}.{}", image_extension(i));
        write(assets, &path, &i.data)?;
        if normal {
            let imp = TextureImporter {
                texture_type: TextureImporterType::NormalMap,
                srgb: false,
                ..assets.texture_importer(&path)
            };
            assets.set_texture_importer(&path, &imp)?;
        } else {
            assets.load_texture(&path)?;
        }
        textures.push((role.to_owned(), path.clone()));
        Ok(path)
    };
    let mut m = Material {
        name: base.clone(),
        shader: ShaderKind::Lit,
        ..Default::default()
    };
    if let Some(i) = &info {
        if let Some([r, g, b, a]) = i.base_color {
            // Blender's values are linear; Oxide's Color is too.
            m.base_color = Color::new(r, g, b, a);
        }
        if let Some(v) = i.metallic {
            m.metallic = v.clamp(0.0, 1.0);
        }
        if let Some(r) = i.roughness {
            m.smoothness = 1.0 - r.clamp(0.0, 1.0);
        }
    }
    if let Some(i) = get(TextureRole::BaseColor) {
        let p = save(assets, "BaseColor", i, false)?;
        m.base_map = assets.textures.handle_of(&p);
        // The texture is the color: Blender multiplies nothing in by default.
        m.base_color = Color::WHITE;
    }
    if let Some(i) = get(TextureRole::Normal) {
        let p = save(assets, "Normal", i, true)?;
        m.normal_map = assets.textures.handle_of(&p);
        m.normal_scale = 1.0;
    }
    if let Some(i) = get(TextureRole::Occlusion) {
        let p = save(assets, "Occlusion", i, false)?;
        m.occlusion_map = assets.textures.handle_of(&p);
        m.occlusion_strength = 1.0;
    }
    if let Some(i) = get(TextureRole::Emission) {
        let p = save(assets, "Emission", i, false)?;
        m.emission_map = assets.textures.handle_of(&p);
        m.emission_color = Color::WHITE;
        m.emission_intensity = 1.0;
    }
    // Height maps have no Lit input; they're kept next to the others.
    if let Some(i) = get(TextureRole::Height) {
        save(assets, "Height", i, false)?;
    }
    let (rough, smooth, metal) = (
        get(TextureRole::Roughness),
        get(TextureRole::Smoothness),
        get(TextureRole::Metallic),
    );
    if rough.is_some() || smooth.is_some() || metal.is_some() {
        let png = pack_metal_rough(rough, smooth, metal, 1.0 - m.smoothness, m.metallic)?;
        let path = format!("{folder}/Textures/{base}_MetallicRoughness.png");
        write(assets, &path, &png)?;
        let imp = TextureImporter {
            srgb: false,
            ..assets.texture_importer(&path)
        };
        assets.set_texture_importer(&path, &imp)?;
        m.metallic_gloss_map = assets.textures.handle_of(&path);
        // The map carries the values (constants were packed in for missing maps), and the
        // shader multiplies: metallic × B, 1 − (1 − smoothness) × G.
        m.metallic = 1.0;
        m.smoothness = 0.0;
        textures.push(("MetallicRoughness".into(), path));
    }
    let path = format!("{folder}/{base}.mat");
    assets.save_material(&path, &m)?;
    write_info(assets, &folder, asset, &path)?;
    assets.database.refresh();
    Ok(ImportedMaterial { path, textures })
}

/// Puts a material on every renderer of the given objects and their children. Returns how
/// many renderers changed.
pub fn apply_material(
    world: &mut World,
    objects: &[GameObjectId],
    path: &str,
) -> Result<usize, String> {
    let assets = world.resource_mut::<AssetServer>();
    let mat = match assets.materials.handle_of(path) {
        Some(h) => h,
        None => assets.load_material(path)?,
    };
    let mut n = 0;
    let mut ids: Vec<GameObjectId> = objects.iter().flat_map(|&o| world.descendants(o)).collect();
    ids.dedup();
    for id in ids {
        if let Some(r) = world.get_component_mut::<MeshRenderer>(id) {
            set_all(&mut r.materials, &mat);
            n += 1;
        }
        if let Some(r) = world.get_component_mut::<SkinnedMeshRenderer>(id) {
            set_all(&mut r.materials, &mat);
            n += 1;
        }
    }
    Ok(n)
}

fn set_all(slots: &mut Vec<oxide::assets::Handle<Material>>, m: &oxide::assets::Handle<Material>) {
    if slots.is_empty() {
        slots.push(m.clone());
    } else {
        slots.iter_mut().for_each(|s| *s = m.clone());
    }
}

/// Saves an HDRI and a Panoramic skybox material using it. Returns the material's path.
pub fn import_hdri(
    world: &mut World,
    asset: &Asset,
    bytes: &[u8],
    extension: &str,
    resolution: &str,
) -> Result<String, String> {
    let folder = asset_folder(asset, AssetType::Hdr);
    let base = safe_name(&asset.name);
    let tex = format!("{folder}/{base}_{resolution}.{extension}");
    let assets = world.resource_mut::<AssetServer>();
    write(assets, &tex, bytes)?;
    let imp = TextureImporter {
        srgb: false,
        ..assets.texture_importer(&tex)
    };
    let handle = assets.set_texture_importer(&tex, &imp)?;
    let m = Material {
        name: format!("{base} Skybox"),
        shader: ShaderKind::SkyboxPanoramic,
        base_map: Some(handle),
        exposure: 1.0,
        ..Default::default()
    };
    let path = format!("{folder}/{base}_{resolution}_Skybox.mat");
    assets.save_material(&path, &m)?;
    write_info(assets, &folder, asset, &tex)?;
    assets.database.refresh();
    Ok(path)
}

/// Makes a skybox material the scene's sky (Lighting window → Skybox Material).
pub fn set_skybox(world: &mut World, path: &str) -> Result<(), String> {
    let assets = world.resource_mut::<AssetServer>();
    let mat = match assets.materials.handle_of(path) {
        Some(h) => h,
        None => assets.load_material(path)?,
    };
    world.resource_mut::<RenderSettings>().skybox_material = Some(mat);
    Ok(())
}
