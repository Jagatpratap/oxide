//! BlenderKit's public REST API (<https://www.blenderkit.com/api/v1/>): search, download
//! links and the signed-in profile. Free assets need no account; an API key (from your
//! BlenderKit profile page) is sent as `Authorization: Bearer <key>` and unlocks Full Plan
//! assets. Requests go through the engine's `UnityWebRequest`, so they run in the background
//! and the window polls them.

use oxide::assets::deps::serde_json::Value;
use oxide::network::web_request::{
    UnityWebRequest, WebRequestOperation, WebRequestResult, url_encode,
};

pub const API: &str = "https://www.blenderkit.com/api/v1";
/// Where users find (or create) their API key.
pub const PROFILE_URL: &str = "https://www.blenderkit.com/profile/";
pub const PLANS_URL: &str = "https://www.blenderkit.com/plans/pricing/";

/// The asset kinds the package imports.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AssetType {
    #[default]
    Model,
    Material,
    Hdr,
}

impl AssetType {
    pub const ALL: [AssetType; 3] = [AssetType::Model, AssetType::Material, AssetType::Hdr];

    /// The API's `asset_type` value.
    pub fn api_name(self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::Material => "material",
            Self::Hdr => "hdr",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Model => "Models",
            Self::Material => "Materials",
            Self::Hdr => "HDRIs",
        }
    }

    /// Project folder for this kind (`Assets/BlenderKit/Models`).
    pub fn folder(self) -> &'static str {
        match self {
            Self::Model => "Models",
            Self::Material => "Materials",
            Self::Hdr => "HDRIs",
        }
    }
}

/// Which assets a search returns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PriceFilter {
    #[default]
    All,
    Free,
    /// Full Plan assets only.
    Full,
}

/// A downloadable file of an asset.
#[derive(Clone, Debug, PartialEq)]
pub struct AssetFile {
    /// `gltf`, `blend`, `resolution_1K`, `thumbnail`, ...
    pub file_type: String,
    /// The API endpoint that hands out a signed download URL.
    pub download_url: String,
    /// The file's extension (`glb`, `blend`, `exr`).
    pub extension: String,
    pub size: u64,
}

/// One search result.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Asset {
    pub id: String,
    pub base_id: String,
    pub name: String,
    pub description: String,
    pub asset_type: String,
    pub author: String,
    pub category: String,
    pub tags: Vec<String>,
    pub is_free: bool,
    /// `royalty_free` or `cc_zero`.
    pub license: String,
    pub thumbnail: String,
    pub thumbnail_large: String,
    pub files: Vec<AssetFile>,
    /// Bookmarks and quality ratings, if any.
    pub quality: Option<f32>,
}

impl Asset {
    pub fn file(&self, file_type: &str) -> Option<&AssetFile> {
        self.files.iter().find(|f| f.file_type == file_type)
    }

    pub fn license_label(&self) -> &str {
        match self.license.as_str() {
            "cc_zero" => "CC0",
            "royalty_free" => "Royalty Free",
            other => other,
        }
    }

    /// Texture resolutions on offer (`0_5K`, `1K`, `2K`, `4K`, `8K`), smallest first.
    pub fn resolutions(&self) -> Vec<String> {
        let mut r: Vec<String> = self
            .files
            .iter()
            .filter_map(|f| f.file_type.strip_prefix("resolution_").map(str::to_owned))
            .collect();
        r.sort_by(|a, b| resolution_pixels(a).total_cmp(&resolution_pixels(b)));
        r
    }

    /// The file to import for `resolution` (`None` = the original): glTF for models, a
    /// resolution `.blend` for materials, the EXR/HDR for HDRIs.
    pub fn import_file(&self, resolution: Option<&str>) -> Option<&AssetFile> {
        match self.asset_type.as_str() {
            // BlenderKit's export for game engines: plain meshes and WebP textures (its
            // `gltf` file is for the web, with Draco-compressed meshes).
            "model" => self.file("gltf_godot"),
            _ => resolution
                .and_then(|r| self.file(&format!("resolution_{r}")))
                .or_else(|| self.file("blend")),
        }
    }
}

/// `0_5K` → 512, `2K` → 2048.
pub fn resolution_pixels(r: &str) -> f32 {
    r.trim_end_matches('K')
        .replace('_', ".")
        .parse::<f32>()
        .unwrap_or(0.0)
        * 1024.0
}

/// A page of search results.
#[derive(Clone, Debug, Default)]
pub struct SearchPage {
    pub count: u64,
    pub next: Option<String>,
    pub results: Vec<Asset>,
}

/// The signed-in user (`/me/`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Profile {
    pub name: String,
    pub email: String,
    /// `Free` or `Full`.
    pub plan: String,
}

impl Profile {
    /// Whether Full Plan assets download.
    pub fn has_full_plan(&self) -> bool {
        !self.plan.is_empty() && !self.plan.eq_ignore_ascii_case("free")
    }
}

/// The search URL for a query.
pub fn search_url(kind: AssetType, text: &str, price: PriceFilter, page_size: u32) -> String {
    let mut q = String::new();
    let text = text.trim();
    if !text.is_empty() {
        q.push_str(&url_encode(text));
    }
    q.push_str(&format!("+asset_type:{}", kind.api_name()));
    // Only models with the server's glTF export for game engines import into Oxide.
    if kind == AssetType::Model {
        q.push_str("+last_gltf_godot_upload_gte:2000-01-01");
    }
    match price {
        PriceFilter::All => {}
        PriceFilter::Free => q.push_str("+is_free:true"),
        PriceFilter::Full => q.push_str("+is_free:false"),
    }
    // Relevance for keyword searches, newest first otherwise (what the Blender add-on does).
    q.push_str(if text.is_empty() {
        "+order:-last_blend_upload"
    } else {
        "+order:_score"
    });
    format!(
        "{API}/search/?query={q}&dict_parameters=1&page_size={page_size}",
        q = q.trim_start_matches('+')
    )
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_owned()
}

/// Parses a search response.
pub fn parse_search(v: &Value) -> SearchPage {
    let results = v
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(parse_asset)
        .collect();
    SearchPage {
        count: v.get("count").and_then(Value::as_u64).unwrap_or(0),
        next: v
            .get("next")
            .and_then(Value::as_str)
            .map(|n| n.replace("http://", "https://")),
        results,
    }
}

fn parse_asset(a: &Value) -> Asset {
    let files = a
        .get("files")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|f| AssetFile {
            file_type: s(f, "fileType"),
            download_url: s(f, "downloadUrl"),
            extension: s(f, "filename")
                .rsplit_once('.')
                .map(|(_, e)| e.to_ascii_lowercase())
                .unwrap_or_default(),
            size: f.get("fileUploadSize").and_then(Value::as_u64).unwrap_or(0),
        })
        .collect();
    let author = a
        .get("author")
        .map(|au| s(au, "fullName"))
        .unwrap_or_default();
    Asset {
        id: s(a, "id"),
        base_id: s(a, "assetBaseId"),
        name: {
            let d = s(a, "displayName");
            if d.is_empty() { s(a, "name") } else { d }
        },
        description: s(a, "description"),
        asset_type: s(a, "assetType"),
        author,
        category: s(a, "category"),
        tags: a
            .get("tags")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|t| t.as_str().map(str::to_owned))
            .collect(),
        is_free: a.get("isFree").and_then(Value::as_bool).unwrap_or(false),
        license: s(a, "license"),
        thumbnail: s(a, "thumbnailSmallUrl"),
        thumbnail_large: s(a, "thumbnailMiddleUrl"),
        files,
        quality: a
            .pointer("/ratingsAverage/quality")
            .and_then(Value::as_f64)
            .map(|q| q as f32),
    }
}

/// Parses `/me/`.
pub fn parse_profile(v: &Value) -> Profile {
    let name = {
        let f = s(v, "fullName");
        if f.trim().is_empty() {
            s(v, "username")
        } else {
            f
        }
    };
    Profile {
        name,
        email: s(v, "email"),
        plan: s(v, "currentPlanName"),
    }
}

/// A GET with the API key (when there is one).
pub fn get(url: &str, api_key: &str) -> WebRequestOperation {
    let mut r = UnityWebRequest::get(url);
    r.timeout = 60;
    r.set_request_header("Accept", "application/json");
    if !api_key.trim().is_empty() {
        r.set_request_header("Authorization", &format!("Bearer {}", api_key.trim()));
    }
    r.send_web_request()
}

/// Starts a search.
pub fn search(url: &str, api_key: &str) -> WebRequestOperation {
    get(url, api_key)
}

/// Starts a `/me/` request (checks the API key).
pub fn me(api_key: &str) -> WebRequestOperation {
    get(&format!("{API}/me/"), api_key)
}

/// Asks for a signed download URL for a file (`scene_uuid` identifies the project, as the
/// Blender add-on does per scene).
pub fn download_link(file: &AssetFile, api_key: &str, scene_uuid: &str) -> WebRequestOperation {
    get(
        &format!("{}?scene_uuid={scene_uuid}", file.download_url),
        api_key,
    )
}

/// The signed URL from a download-link response.
pub fn parse_download_link(v: &Value) -> Option<String> {
    v.get("filePath").and_then(Value::as_str).map(str::to_owned)
}

/// A readable error for a finished request (BlenderKit's `detail` message when it sent one).
pub fn error_of(op: &WebRequestOperation) -> Option<String> {
    if op.result() == WebRequestResult::Success {
        return None;
    }
    let detail = op
        .json()
        .ok()
        .and_then(|v| v.get("detail").and_then(Value::as_str).map(str::to_owned));
    Some(match (op.response_code(), detail) {
        (401, _) => "This asset needs a BlenderKit Full Plan: add your API key in Account.".into(),
        (403, Some(d)) => d,
        (_, Some(d)) => d,
        (0, None) => op.error(),
        (code, None) => format!("HTTP {code}: {}", op.error()),
    })
}
