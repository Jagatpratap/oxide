//! Project window: browses the project's `Assets/` folder.
//!
//! Like Unity's two-column layout: Favorites and the folder tree on the left, the open
//! folder (or search results) on the right as a thumbnail grid, or a list at the smallest
//! zoom. Search takes names plus `t:Type` and `l:Label` filters. Assets can be renamed,
//! duplicated, deleted, moved by dragging onto a folder, labelled, and dragged into the
//! Scene view, Hierarchy or Inspector fields. Scenes open on double-click.
//! Reference: <https://docs.unity3d.com/Manual/ProjectView.html>

use crate::{Action, EditorState, asset_ops};
use egui::{Color32, Sense, vec2};
use oxide_assets::AssetServer;
use oxide_engine::Engine;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Drag payload: a project-relative asset path (`Assets/Cars/car.glb`).
#[derive(Clone, Debug)]
pub struct AssetPayload(pub String);

/// Unity's Favorites: saved searches in the left column.
pub const FAVORITES: [(&str, &str); 8] = [
    ("All Materials", "t:Material"),
    ("All Models", "t:Model"),
    ("All Prefabs", "t:Prefab"),
    ("All Scenes", "t:Scene"),
    ("All Textures", "t:Texture"),
    ("All Scripts", "t:Script"),
    ("All Audio", "t:AudioClip"),
    ("All Shaders", "t:Shader"),
];

/// Type names for `t:` (Unity's names first) and the Type filter menu.
pub const TYPES: [&str; 11] = [
    "AnimationClip",
    "AudioClip",
    "Folder",
    "Material",
    "Model",
    "Prefab",
    "RenderTexture",
    "Scene",
    "Script",
    "Shader",
    "Texture",
];

pub struct ProjectState {
    /// Open folder, relative to the project root.
    pub folder: String,
    pub selected: Option<String>,
    /// The Inspector shows the selected asset (set by clicking it, cleared when a
    /// GameObject is selected).
    pub inspecting: bool,
    pub search: String,
    /// Search the whole project (Unity "All") instead of only the open folder.
    pub search_all: bool,
    new_script: Option<String>,
    /// Inline rename: asset path and the name being typed.
    renaming: Option<(String, String)>,
    /// Asks before deleting this asset.
    confirm_delete: Option<String>,
    /// Icon size in the grid; at the minimum the folder is shown as a list.
    pub zoom: f32,
    /// The window was clicked last (Delete, Duplicate and Rename act on assets).
    pub focused: bool,
    /// Cached listing of the open folder and when it was read (seconds).
    pub cache: Option<(String, f64, Vec<Entry>)>,
    /// Every asset in the project, for searches, and when it was read.
    index: Option<(f64, Vec<Entry>)>,
    /// Scroll the selected asset into view next frame (after "Show in Project").
    pub reveal: bool,
    /// An asset was clicked this frame (the GameObject selection is cleared).
    clicked: bool,
}

pub const MIN_ZOOM: f32 = 16.0;
const MAX_ZOOM: f32 = 128.0;

impl Default for ProjectState {
    fn default() -> Self {
        Self {
            folder: "Assets".into(),
            selected: None,
            inspecting: false,
            search: String::new(),
            search_all: true,
            new_script: None,
            renaming: None,
            confirm_delete: None,
            zoom: 64.0,
            focused: false,
            cache: None,
            index: None,
            reveal: false,
            clicked: false,
        }
    }
}

impl ProjectState {
    /// Selects an asset and opens its folder (Unity "ping").
    pub fn show(&mut self, path: &str) {
        self.selected = Some(path.to_owned());
        self.inspecting = true;
        if let Some((dir, _)) = path.rsplit_once('/') {
            self.folder = dir.to_owned();
        }
        self.search.clear();
        self.reveal = true;
    }

    /// Starts renaming the selected asset (F2).
    pub fn start_rename(&mut self) -> bool {
        let Some(p) = self.selected.clone() else {
            return false;
        };
        let name = p.rsplit('/').next().unwrap_or(&p);
        let stem = match name.rsplit_once('.') {
            Some((s, _)) if !s.is_empty() => s,
            _ => name,
        };
        self.renaming = Some((p.clone(), stem.to_owned()));
        true
    }

    pub fn ask_delete(&mut self) -> bool {
        self.confirm_delete = self.selected.clone();
        self.confirm_delete.is_some()
    }

    /// Forces the listings to be read again.
    pub fn invalidate(&mut self) {
        self.cache = None;
        self.index = None;
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    /// Project-relative path with `/` separators.
    pub path: String,
    pub dir: bool,
    pub labels: Vec<String>,
}

/// Unity type name of an asset file and its icon color.
/// Extension → (type name, icon colour), from the asset type registry.
static KINDS: std::sync::RwLock<Vec<(&'static str, &'static str, [u8; 3])>> =
    std::sync::RwLock::new(Vec::new());

/// Refreshes the Project window's view of the asset type registry (when it changed).
pub fn sync_asset_types(types: &oxide_assets::AssetTypeRegistry) {
    static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(u64::MAX);
    let g = types.generation();
    if GENERATION.swap(g, std::sync::atomic::Ordering::Relaxed) == g {
        return;
    }
    let mut v = Vec::new();
    for t in types.iter() {
        for e in t.extensions {
            v.push((*e, t.type_name, t.color));
        }
    }
    if let Ok(mut k) = KINDS.write() {
        *k = v;
    }
}

/// Type name and icon colour of a file, by extension.
pub fn kind(name: &str) -> (&'static str, Color32) {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    if let Ok(k) = KINDS.read()
        && let Some((_, t, c)) = k.iter().find(|(e, _, _)| e.eq_ignore_ascii_case(&ext))
    {
        return (t, Color32::from_rgb(c[0], c[1], c[2]));
    }
    match ext.as_str() {
        "scene" => ("Scene", Color32::from_rgb(120, 200, 255)),
        "glb" | "gltf" | "fbx" | "obj" => ("Model", Color32::from_rgb(120, 220, 160)),
        "png" | "jpg" | "jpeg" | "tga" | "bmp" | "hdr" | "exr" | "ktx2" | "psd" | "psb" | "tif"
        | "tiff" | "gif" | "webp" => ("Texture", Color32::from_rgb(230, 190, 110)),
        "wav" | "ogg" | "mp3" | "flac" => ("AudioClip", Color32::from_rgb(230, 140, 200)),
        "rs" => ("Script", Color32::from_rgb(240, 120, 90)),
        "prefab" => ("Prefab", Color32::from_rgb(140, 180, 255)),
        "mat" => ("Material", Color32::from_rgb(200, 160, 255)),
        "rendertexture" => ("RenderTexture", Color32::from_rgb(230, 190, 110)),
        "wgsl" => ("Shader", Color32::from_rgb(160, 210, 230)),
        "anim" => ("AnimationClip", Color32::from_rgb(200, 200, 120)),
        _ => ("File", Color32::from_gray(170)),
    }
}

pub fn entry_kind(e: &Entry) -> (&'static str, Color32) {
    if e.dir {
        ("Folder", Color32::from_rgb(220, 200, 120))
    } else {
        kind(&e.name)
    }
}

fn read_labels(root: &Path, path: &str) -> Vec<String> {
    let meta = root.join(format!("{path}.meta"));
    if !meta.is_file() {
        return Vec::new();
    }
    std::fs::read_to_string(meta)
        .map(|t| oxide_assets::server::labels_in_meta(&t))
        .unwrap_or_default()
}

pub fn list_dir(root: &Path, rel: &str) -> Vec<Entry> {
    let mut v: Vec<Entry> = std::fs::read_dir(root.join(rel))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name.ends_with(".meta") {
                return None;
            }
            let path = format!("{rel}/{name}");
            Some(Entry {
                labels: read_labels(root, &path),
                dir: e.file_type().is_ok_and(|t| t.is_dir()),
                name,
                path,
            })
        })
        .collect();
    v.sort_by(|a, b| {
        b.dir
            .cmp(&a.dir)
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    v
}

/// Every asset and folder under `rel` (depth-first, folders before files).
pub fn list_all(root: &Path, rel: &str) -> Vec<Entry> {
    fn walk(root: &Path, rel: &str, out: &mut Vec<Entry>, depth: usize) {
        if depth > 16 || out.len() > 50_000 {
            return;
        }
        for e in list_dir(root, rel) {
            let dir = e.dir.then(|| e.path.clone());
            out.push(e);
            if let Some(d) = dir {
                walk(root, &d, out, depth + 1);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, rel, &mut out, 0);
    out
}

/// Project-relative paths of files with `ext` under `Assets/`.
pub fn find_files(root: &Path, ext: &str) -> Vec<String> {
    let suffix = format!(".{ext}");
    let mut out: Vec<String> = list_all(root, "Assets")
        .into_iter()
        .filter(|e| !e.dir && e.name.to_ascii_lowercase().ends_with(&suffix))
        .map(|e| e.path)
        .collect();
    out.sort();
    out
}

/// A parsed search: name words plus `t:` and `l:` filters (Unity's search syntax).
#[derive(Debug, Default, PartialEq)]
pub struct Query {
    pub words: Vec<String>,
    pub types: Vec<String>,
    pub labels: Vec<String>,
}

impl Query {
    pub fn parse(text: &str) -> Self {
        let mut q = Query::default();
        for w in text.split_whitespace() {
            let lower = w.to_lowercase();
            if let Some(t) = lower.strip_prefix("t:") {
                q.types.push(t.to_owned());
            } else if let Some(l) = lower.strip_prefix("l:") {
                q.labels.push(l.to_owned());
            } else {
                q.words.push(lower);
            }
        }
        q
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty() && self.types.is_empty() && self.labels.is_empty()
    }

    /// Unity matches: every word in the name, any of the types, and all the labels.
    pub fn matches(&self, e: &Entry) -> bool {
        let name = e.name.to_lowercase();
        if !self.words.iter().all(|w| name.contains(w.as_str())) {
            return false;
        }
        if !self.types.is_empty() {
            let k = entry_kind(e).0.to_lowercase();
            // `t:Texture2D` and `t:GameObject` are Unity's names for textures and prefabs.
            let ok = self.types.iter().any(|t| {
                k == *t
                    || (k == "texture" && t == "texture2d")
                    || (k == "prefab" && t == "gameobject")
                    || (k == "audioclip" && t == "audio")
                    || (k == "script" && t == "monoscript")
            });
            if !ok {
                return false;
            }
        }
        self.labels
            .iter()
            .all(|l| e.labels.iter().any(|x| x.to_lowercase() == *l))
    }
}

/// Labels used anywhere in the project (for the label filter and the Inspector's picker).
pub fn all_labels(st: &mut ProjectState, root: &Path) -> Vec<String> {
    let now = oxide_core::time::realtime_since_startup();
    if st.index.as_ref().is_none_or(|(t, _)| now - t > 2.0) {
        st.index = Some((now, list_all(root, "Assets")));
    }
    let set: BTreeSet<String> = st
        .index
        .iter()
        .flat_map(|(_, v)| v.iter())
        .flat_map(|e| e.labels.iter().cloned())
        .collect();
    set.into_iter().collect()
}

/// What the window asked for this frame (run once the listing isn't borrowed).
enum Op {
    Open(String),
    Move(String, String),
    Rename(String, String),
    Duplicate(String),
    Reveal(String),
    Reimport(String),
    NewFolder(String),
    /// A new asset of a registered type (by type name) in a folder.
    NewAsset(&'static str, String),
    /// A material using a Shader Graph (Unity: select the graph, Create → Material).
    GraphMaterial(String),
    /// A Prefab Variant of a prefab asset, next to it.
    PrefabVariant(String),
    OpenSceneAdditive(String),
    ExportPackage(String),
    /// A Font Asset from a font file, next to it (TMP's Create > TextMeshPro > Font Asset).
    FontAsset(String),
    SpriteAsset(String),
}

/// A Create menu entry: (menu path, asset type name, sort order).
type MenuType = (&'static str, &'static str, i32);

fn folder_tree(
    ui: &mut egui::Ui,
    root: &Path,
    rel: &str,
    ps: &mut ProjectState,
    ops: &mut Vec<Op>,
    depth: usize,
) {
    let name = rel.rsplit('/').next().unwrap_or(rel).to_owned();
    let subdirs: Vec<Entry> = list_dir(root, rel).into_iter().filter(|e| e.dir).collect();
    let selected = ps.folder == rel && ps.search.is_empty();
    // Opening a folder in the grid expands its parents in the tree.
    let open_inside = ps.folder.starts_with(&format!("{rel}/"));
    let mut row = |ui: &mut egui::Ui, text: String| {
        let r = ui.selectable_label(selected, text);
        if r.clicked() {
            ps.folder = rel.to_owned();
            ps.search.clear();
        }
        drop_target(&r, rel, ops);
        if rel != "Assets" {
            r.context_menu(|ui| item_menu(ui, rel, true, ops, ps));
        }
    };
    if subdirs.is_empty() || depth > 10 {
        row(ui, format!("  {name}"));
        return;
    }
    let id = ui.make_persistent_id(("folder", rel));
    let mut state =
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, depth == 0);
    if open_inside && !state.is_open() {
        state.set_open(true);
    }
    state
        .show_header(ui, |ui| row(ui, name.clone()))
        .body(|ui| {
            for d in subdirs {
                folder_tree(ui, root, &d.path, ps, ops, depth + 1);
            }
        });
}

/// Dropping an asset onto a folder moves it there.
fn drop_target(r: &egui::Response, folder: &str, ops: &mut Vec<Op>) {
    if let Some(p) = r.dnd_hover_payload::<AssetPayload>()
        && p.0 != folder
    {
        r.ctx.debug_painter().rect_stroke(
            r.rect,
            2.0,
            egui::Stroke::new(1.5, Color32::from_rgb(120, 170, 230)),
            egui::StrokeKind::Inside,
        );
    }
    if let Some(p) = r.dnd_release_payload::<AssetPayload>() {
        let name = p.0.rsplit('/').next().unwrap_or(&p.0);
        let to = format!("{folder}/{name}");
        if p.0 != folder && to != p.0 {
            ops.push(Op::Move(p.0.clone(), to));
        }
    }
}

/// Create menu (the `+` button and right-click on empty space): new assets go into `folder`.
/// Entries come from the asset type registry; `A/B` paths become submenus, as in Unity.
fn create_menu(
    ui: &mut egui::Ui,
    ps: &mut ProjectState,
    scripts: bool,
    types: &[MenuType],
    ops: &mut Vec<Op>,
) {
    let folder = ps.folder.clone();
    let entries: Vec<(&str, &'static str, i32)> =
        types.iter().map(|(p, t, o)| (*p, *t, *o)).collect();
    create_level(ui, ps, scripts, &entries, &folder, ops);
}

/// One menu level: its own entries first (a separator between order groups of 20), then
/// submenus (nested as deep as the paths go, like Unity's Shader Graph/URP/...).
fn create_level(
    ui: &mut egui::Ui,
    ps: &mut ProjectState,
    scripts: bool,
    entries: &[(&str, &'static str, i32)],
    folder: &str,
    ops: &mut Vec<Op>,
) {
    let mut subs: Vec<&str> = Vec::new();
    let mut group = None;
    for (path, type_name, order) in entries {
        match path.split_once('/') {
            None => {
                if group.is_some_and(|g| g != order / 20) {
                    ui.separator();
                }
                group = Some(order / 20);
                create_item(ui, ps, scripts, path, type_name, folder, ops);
            }
            Some((sub, _)) if !subs.contains(&sub) => subs.push(sub),
            Some(_) => {}
        }
    }
    if !subs.is_empty() && group.is_some() {
        ui.separator();
    }
    for sub in subs {
        ui.menu_button(sub, |ui| {
            let inner: Vec<(&str, &'static str, i32)> = entries
                .iter()
                .filter_map(|(p, t, o)| {
                    let (s, rest) = p.split_once('/')?;
                    (s == sub).then_some((rest, *t, *o))
                })
                .collect();
            create_level(ui, ps, scripts, &inner, folder, ops);
        });
    }
}

fn create_item(
    ui: &mut egui::Ui,
    ps: &mut ProjectState,
    scripts: bool,
    label: &str,
    type_name: &'static str,
    folder: &str,
    ops: &mut Vec<Op>,
) {
    match type_name {
        "Folder" => {
            if ui.button(label).clicked() {
                ops.push(Op::NewFolder(folder.to_owned()));
                ui.close();
            }
        }
        "Script" => {
            if ui
                .add_enabled(scripts, egui::Button::new(label))
                .on_disabled_hover_text("Open a project created with `oxide new` to write scripts")
                .clicked()
            {
                ps.new_script = Some("NewScript".into());
                ui.close();
            }
        }
        _ => {
            if ui.button(label).clicked() {
                ops.push(Op::NewAsset(type_name, folder.to_owned()));
                ui.close();
            }
        }
    }
}

/// Right-click menu of an asset or folder.
fn item_menu(ui: &mut egui::Ui, path: &str, dir: bool, ops: &mut Vec<Op>, ps: &mut ProjectState) {
    ps.selected = Some(path.to_owned());
    let reveal = if cfg!(target_os = "macos") {
        "Reveal in Finder"
    } else {
        "Show in Explorer"
    };
    if ui.button("Open").clicked() {
        ops.push(Op::Open(path.to_owned()));
        ui.close();
    }
    if ui.button(reveal).clicked() {
        ops.push(Op::Reveal(path.to_owned()));
        ui.close();
    }
    ui.separator();
    if ui.button("Rename").clicked() {
        ps.start_rename();
        ui.close();
    }
    if ui.button("Duplicate").clicked() {
        ops.push(Op::Duplicate(path.to_owned()));
        ui.close();
    }
    if ui.button("Delete").clicked() {
        ps.ask_delete();
        ui.close();
    }
    ui.separator();
    if ui.button("Copy Path").clicked() {
        ui.ctx().copy_text(path.to_owned());
        ui.close();
    }
    if !dir && ui.button("Reimport").clicked() {
        ops.push(Op::Reimport(path.to_owned()));
        ui.close();
    }
    if path.ends_with(".shadergraph") && ui.button("Create Material").clicked() {
        ops.push(Op::GraphMaterial(path.to_owned()));
        ui.close();
    }
    if ui.button("Export Package…").clicked() {
        ops.push(Op::ExportPackage(path.to_owned()));
        ui.close();
    }
    if path.ends_with(".scene") && ui.button("Open Scene Additive").clicked() {
        ops.push(Op::OpenSceneAdditive(path.to_owned()));
        ui.close();
    }
    if path.ends_with(".prefab") && ui.button("Create Prefab Variant").clicked() {
        ops.push(Op::PrefabVariant(path.to_owned()));
        ui.close();
    }
    let lower = path.to_ascii_lowercase();
    if (lower.ends_with(".ttf") || lower.ends_with(".otf"))
        && ui.button("Create Font Asset").clicked()
    {
        ops.push(Op::FontAsset(path.to_owned()));
        ui.close();
    }
    if [
        ".png", ".jpg", ".jpeg", ".tga", ".psd", ".bmp", ".gif", ".webp",
    ]
    .iter()
    .any(|e| lower.ends_with(e))
        && ui.button("Create Sprite Asset").clicked()
    {
        ops.push(Op::SpriteAsset(path.to_owned()));
        ui.close();
    }
}

/// An icon for assets without a thumbnail: a folder, or a page with the type's initials.
fn draw_icon(painter: &egui::Painter, rect: egui::Rect, e: &Entry) {
    let (label, color) = entry_kind(e);
    let s = rect.width().min(rect.height());
    let r = egui::Rect::from_center_size(rect.center(), vec2(s, s));
    if e.dir {
        let body = egui::Rect::from_min_max(
            r.min + vec2(s * 0.06, s * 0.24),
            r.max - vec2(s * 0.06, s * 0.14),
        );
        let tab =
            egui::Rect::from_min_size(body.min - vec2(0.0, s * 0.1), vec2(s * 0.38, s * 0.14));
        painter.rect_filled(tab, 2.0, color.gamma_multiply(0.8));
        painter.rect_filled(body, 2.0, color);
        return;
    }
    let page = r.shrink2(vec2(s * 0.16, s * 0.06));
    painter.rect_filled(page, 2.0, Color32::from_gray(78));
    painter.rect_stroke(
        page,
        2.0,
        egui::Stroke::new((s * 0.04).max(1.0), color),
        egui::StrokeKind::Inside,
    );
    if s >= 28.0 {
        let short: String = label.chars().filter(|c| c.is_uppercase()).collect();
        let short = if short.len() >= 2 {
            short
        } else {
            label.chars().take(3).collect::<String>().to_uppercase()
        };
        painter.text(
            page.center(),
            egui::Align2::CENTER_CENTER,
            short,
            egui::FontId::proportional(s * 0.2),
            color,
        );
    }
}

/// The name under a grid tile, cut to fit `width`.
fn fit(painter: &egui::Painter, text: &str, width: f32, font: &egui::FontId) -> String {
    let w = |t: &str| {
        painter
            .layout_no_wrap(t.to_owned(), font.clone(), Color32::WHITE)
            .size()
            .x
    };
    if w(text) <= width {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut n = chars.len();
    while n > 1 {
        n -= 1;
        let t: String = chars[..n].iter().collect::<String>() + "…";
        if w(&t) <= width {
            return t;
        }
    }
    "…".into()
}

pub fn ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    let root: PathBuf = engine.world.resource::<AssetServer>().full_path("");
    let assets_dir = root.join("Assets");
    if !assets_dir.is_dir() {
        ui.label(format!("No Assets folder in {}", root.display()));
        if ui.button("Create Assets folder").clicked() {
            let _ = std::fs::create_dir_all(&assets_dir);
        }
        return;
    }
    if ui.input(|i| i.pointer.any_pressed()) && ui.rect_contains_pointer(ui.max_rect()) {
        st.project.focused = true;
    }
    let scripts = st.scripts_available;
    let mut ops: Vec<Op> = Vec::new();
    let labels = all_labels(&mut st.project, &root);
    let menu: Vec<MenuType> = engine
        .world
        .resource::<AssetServer>()
        .types
        .create_menu()
        .into_iter()
        .filter_map(|t| Some((t.create_menu?, t.type_name, t.order)))
        .flat_map(|e| {
            // Unity's Create → Shader Graph → URP has Lit and Unlit graphs.
            let unlit = (e.1 == "ShaderGraph").then_some((
                "Shader Graph/URP/Unlit Shader Graph",
                "UnlitShaderGraph",
                e.2,
            ));
            std::iter::once(e).chain(unlit)
        })
        .collect();
    let ps = &mut st.project;

    // Toolbar: Create, breadcrumbs / search scope, search with Type and Label filters.
    ui.horizontal(|ui| {
        ui.menu_button("+", |ui| create_menu(ui, ps, scripts, &menu, &mut ops))
            .response
            .on_hover_text("Create");
        if ps.search.trim().is_empty() {
            let parts: Vec<String> = ps.folder.split('/').map(str::to_owned).collect();
            for (i, part) in parts.iter().enumerate() {
                if i > 0 {
                    ui.weak(">");
                }
                let r = ui.link(part);
                if r.clicked() {
                    ps.folder = parts[..=i].join("/");
                }
                drop_target(&r, &parts[..=i].join("/"), &mut ops);
            }
        } else {
            ui.label("Search:");
            ui.selectable_value(&mut ps.search_all, true, "All");
            let folder = ps.folder.rsplit('/').next().unwrap_or("").to_owned();
            ui.selectable_value(&mut ps.search_all, false, format!("'{folder}'"));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.menu_button("Label", |ui| {
                if labels.is_empty() {
                    ui.weak("No labels yet. Add them in the Inspector.");
                }
                for l in &labels {
                    let token = format!("l:{l}");
                    let mut on = ps
                        .search
                        .split_whitespace()
                        .any(|w| w.eq_ignore_ascii_case(&token));
                    if ui.checkbox(&mut on, l).changed() {
                        toggle_token(&mut ps.search, &token, on);
                    }
                }
            })
            .response
            .on_hover_text("Search by Label");
            ui.menu_button("Type", |ui| {
                for t in TYPES {
                    let token = format!("t:{t}");
                    let mut on = ps
                        .search
                        .split_whitespace()
                        .any(|w| w.eq_ignore_ascii_case(&token));
                    if ui.checkbox(&mut on, t).changed() {
                        toggle_token(&mut ps.search, &token, on);
                    }
                }
            })
            .response
            .on_hover_text("Search by Type");
            ui.add(
                egui::TextEdit::singleline(&mut ps.search)
                    .hint_text("Search (t:Type l:Label)")
                    .desired_width(190.0),
            );
        });
    });
    if let Some(name) = ps.new_script.as_mut() {
        let mut done = false;
        ui.horizontal(|ui| {
            ui.label("New script:");
            let r = ui.text_edit_singleline(name);
            r.request_focus();
            if ui.button("Create").clicked()
                || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
            {
                st.actions
                    .push(Action::CreateScript(name.trim().to_owned()));
                done = true;
            }
            if ui.button("Cancel").clicked() {
                done = true;
            }
        });
        if done {
            ps.new_script = None;
        }
    }
    ui.separator();

    let now = oxide_core::time::realtime_since_startup();
    let query = Query::parse(&ps.search);
    let entries: Vec<Entry> = if query.is_empty() {
        let stale = ps
            .cache
            .as_ref()
            .is_none_or(|(f, t, _)| *f != ps.folder || now - t > 1.0);
        if stale {
            ps.cache = Some((ps.folder.clone(), now, list_dir(&root, &ps.folder)));
        }
        ps.cache.as_ref().map(|c| c.2.clone()).unwrap_or_default()
    } else {
        let prefix = format!("{}/", ps.folder);
        ps.index
            .iter()
            .flat_map(|(_, v)| v.iter())
            .filter(|e| ps.search_all || e.path.starts_with(&prefix))
            .filter(|e| query.matches(e))
            .cloned()
            .collect()
    };

    // Bottom bar: the selected asset's path and the zoom slider.
    egui::Panel::bottom(egui::Id::new("project-bottom"))
        .resizable(false)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                match &ps.selected {
                    Some(p) => ui.weak(p),
                    None => ui.weak(format!("{} items", entries.len())),
                };
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add(
                        egui::Slider::new(&mut ps.zoom, MIN_ZOOM..=MAX_ZOOM)
                            .show_value(false)
                            .trailing_fill(true),
                    )
                    .on_hover_text("Icon size (smallest shows a list)");
                });
            });
        });

    egui::Panel::left(egui::Id::new("project-folders"))
        .resizable(true)
        .default_size(170.0)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::CollapsingHeader::new("★ Favorites")
                        .default_open(true)
                        .show(ui, |ui| {
                            for (name, q) in FAVORITES {
                                if ui.selectable_label(ps.search == q, name).clicked() {
                                    ps.search = q.to_owned();
                                    ps.search_all = true;
                                }
                            }
                        });
                    folder_tree(ui, &root, "Assets", ps, &mut ops, 0);
                });
        });

    let folder = ps.folder.clone();
    let mut new_prefab = None;
    let list = ps.zoom <= MIN_ZOOM + 0.5;
    let searching = !query.is_empty();
    let central = egui::CentralPanel::default().show(ui, |ui| {
        // The background is registered first so the assets drawn after it are on top and
        // get their own clicks and right-clicks.
        let bg = ui.interact(
            ui.available_rect_before_wrap(),
            ui.id().with("project-bg"),
            Sense::click(),
        );
        let _scroll = egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if entries.is_empty() {
                    ui.weak(if searching {
                        "No assets match the search."
                    } else {
                        "This folder is empty."
                    });
                }
                if list {
                    for e in &entries {
                        let (rect, resp) = ui.allocate_exact_size(
                            vec2(ui.available_width(), 18.0),
                            Sense::click_and_drag(),
                        );
                        item(
                            ui,
                            &mut st.previews,
                            ps,
                            &root,
                            e,
                            rect,
                            resp,
                            true,
                            searching,
                            &mut ops,
                        );
                    }
                } else {
                    let icon = ps.zoom;
                    let tile = vec2(icon + 20.0, icon + 30.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(4.0, 6.0);
                        for e in &entries {
                            let (rect, resp) =
                                ui.allocate_exact_size(tile, Sense::click_and_drag());
                            item(
                                ui,
                                &mut st.previews,
                                ps,
                                &root,
                                e,
                                rect,
                                resp,
                                false,
                                searching,
                                &mut ops,
                            );
                        }
                    });
                }
            });
        // Clicking empty space clears the asset selection.
        if bg.clicked() {
            ps.selected = None;
        }
        {
            bg.context_menu(|ui| {
                ui.menu_button("Create", |ui| create_menu(ui, ps, scripts, &menu, &mut ops));
                let reveal = if cfg!(target_os = "macos") {
                    "Reveal in Finder"
                } else {
                    "Show in Explorer"
                };
                if ui.button(reveal).clicked() {
                    ops.push(Op::Reveal(folder.clone()));
                    ui.close();
                }
                if ui.button("Refresh").clicked() {
                    st.actions.push(Action::RefreshAssets);
                    ui.close();
                }
            });
        }
    });
    // Drop a Hierarchy object here to make it a prefab (Unity's way of creating prefabs).
    if let Some(payload) = central
        .response
        .dnd_release_payload::<crate::hierarchy::HierarchyDrag>()
        && let Some(&id) = payload.0.first()
        && let Some(o) = engine.world.get(id)
    {
        let name = asset_ops::check_name(&o.name)
            .unwrap_or("Prefab")
            .to_owned();
        let path = asset_ops::unique_path(&root, &format!("{folder}/{name}.prefab"));
        new_prefab = Some((id, path));
    }
    // An asset dropped on empty space moves into the open folder.
    if !searching {
        drop_target(&central.response, &folder, &mut ops);
    }
    if let Some((id, path)) = new_prefab {
        st.select(id);
        st.actions.push(Action::CreatePrefab(path));
    }
    if std::mem::take(&mut st.project.clicked) {
        st.selection.clear();
    }
    delete_dialog(st, &root, ui.ctx());
    for op in ops {
        if let Err(e) = run(st, engine, &root, op) {
            oxide_core::debug_log_warning!("{e}");
            st.status = e;
        }
    }
}

fn toggle_token(search: &mut String, token: &str, on: bool) {
    let mut words: Vec<&str> = search
        .split_whitespace()
        .filter(|w| !w.eq_ignore_ascii_case(token))
        .collect();
    if on {
        words.push(token);
    }
    *search = words.join(" ");
}

/// One asset in the grid or list: drawing, selection, rename, drag and its menu.
#[allow(clippy::too_many_arguments)]
fn item(
    ui: &mut egui::Ui,
    previews: &mut crate::previews::Previews,
    ps: &mut ProjectState,
    root: &Path,
    e: &Entry,
    rect: egui::Rect,
    resp: egui::Response,
    list: bool,
    searching: bool,
    ops: &mut Vec<Op>,
) {
    let selected = ps.selected.as_deref() == Some(e.path.as_str());
    if selected && std::mem::take(&mut ps.reveal) {
        ui.scroll_to_rect(rect, Some(egui::Align::Center));
    }
    let painter = ui.painter_at(rect);
    let thumb = (!e.dir).then(|| previews.get(root, &e.path)).flatten();
    let renaming = ps.renaming.as_ref().filter(|(p, _)| *p == e.path).is_some();
    let font = egui::FontId::proportional(if list { 12.5 } else { 11.5 });
    let text_color = Color32::from_gray(215);
    let label_rect;
    if list {
        if selected {
            painter.rect_filled(rect, 0.0, crate::theme::SELECTION_BG);
        } else if resp.hovered() {
            painter.rect_filled(rect, 0.0, Color32::from_gray(69));
        }
        let icon = egui::Rect::from_min_size(rect.min + vec2(4.0, 1.0), vec2(16.0, 16.0));
        match thumb {
            Some(t) => {
                painter.image(
                    t,
                    icon,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            None => draw_icon(&painter, icon, e),
        }
        label_rect = egui::Rect::from_min_max(
            egui::pos2(icon.max.x + 4.0, rect.min.y),
            egui::pos2(rect.max.x - 90.0, rect.max.y),
        );
        if !renaming {
            let name = if searching { &e.path } else { &e.name };
            painter.text(
                label_rect.left_center(),
                egui::Align2::LEFT_CENTER,
                name,
                font.clone(),
                text_color,
            );
        }
        painter.text(
            rect.right_center() - vec2(6.0, 0.0),
            egui::Align2::RIGHT_CENTER,
            entry_kind(e).0,
            egui::FontId::proportional(11.0),
            Color32::from_gray(130),
        );
    } else {
        let s = rect.width() - 20.0;
        let icon = egui::Rect::from_min_size(rect.min + vec2(10.0, 2.0), vec2(s, s));
        if selected {
            painter.rect_filled(
                icon.expand(3.0),
                3.0,
                crate::theme::SELECTION_BG.gamma_multiply(0.6),
            );
        } else if resp.hovered() {
            painter.rect_filled(icon.expand(3.0), 3.0, Color32::from_gray(69));
        }
        match thumb {
            Some(t) => {
                painter.image(
                    t,
                    icon,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            None => draw_icon(&painter, icon, e),
        }
        label_rect = egui::Rect::from_min_max(
            egui::pos2(rect.min.x, icon.max.y + 4.0),
            egui::pos2(rect.max.x, rect.max.y),
        );
        if !renaming {
            let text = fit(&painter, &e.name, label_rect.width() - 4.0, &font);
            let galley = painter.layout_no_wrap(text, font.clone(), text_color);
            let pos = egui::pos2(
                label_rect.center().x - galley.size().x * 0.5,
                label_rect.min.y,
            );
            if selected {
                painter.rect_filled(
                    egui::Rect::from_min_size(pos, galley.size()).expand2(vec2(3.0, 1.0)),
                    2.0,
                    crate::theme::SELECTION_BG,
                );
            }
            painter.galley(pos, galley, text_color);
        }
    }
    if renaming {
        let mut commit = None;
        let mut cancel = false;
        if let Some((path, name)) = ps.renaming.as_mut() {
            let mut child = ui.new_child(
                egui::UiBuilder::new().max_rect(label_rect.with_max_y(label_rect.min.y + 18.0)),
            );
            let out = egui::TextEdit::singleline(name)
                .font(font.clone())
                .desired_width(label_rect.width())
                .show(&mut child);
            let r = out.response;
            if !r.has_focus() && !r.lost_focus() {
                // Start with the whole name selected, so typing replaces it (Unity).
                r.request_focus();
                let mut state = out.state;
                let all = egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(name.chars().count()),
                );
                state.cursor.set_char_range(Some(all));
                state.store(ui.ctx(), r.id);
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                cancel = true;
            } else if r.lost_focus() {
                commit = Some((path.clone(), name.clone()));
            }
        }
        if cancel {
            ps.renaming = None;
        } else if let Some((path, name)) = commit {
            ps.renaming = None;
            ops.push(Op::Rename(path, name));
        }
    }
    if resp.clicked() {
        ps.selected = Some(e.path.clone());
        ps.inspecting = true;
        ps.clicked = true;
    }
    if resp.double_clicked() {
        if e.dir {
            ps.folder = e.path.clone();
            ps.search.clear();
        } else {
            ops.push(Op::Open(e.path.clone()));
        }
    }
    if resp.drag_started() {
        resp.dnd_set_drag_payload(AssetPayload(e.path.clone()));
    }
    if e.dir {
        drop_target(&resp, &e.path, ops);
    }
    let resp = if renaming {
        resp
    } else {
        let mut tip = e.path.clone();
        if !e.labels.is_empty() {
            tip += &format!("\nLabels: {}", e.labels.join(", "));
        }
        resp.on_hover_text(tip)
    };
    resp.context_menu(|ui| item_menu(ui, &e.path, e.dir, ops, ps));
}

/// "Delete selected asset?" (Unity asks too).
fn delete_dialog(st: &mut EditorState, root: &Path, ctx: &egui::Context) {
    let Some(path) = st.project.confirm_delete.clone() else {
        return;
    };
    let mut close = false;
    let mut delete = false;
    egui::Modal::new(egui::Id::new("project-delete")).show(ctx, |ui| {
        ui.set_width(360.0);
        ui.heading("Delete selected asset?");
        ui.add_space(4.0);
        ui.label(&path);
        ui.add_space(4.0);
        ui.weak(if cfg!(target_os = "macos") {
            "It will be moved to the Trash. Scenes and prefabs that use it will show it missing."
        } else {
            "You cannot undo this action. Scenes and prefabs that use it will show it missing."
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Delete").clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                delete = true;
            }
            if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                close = true;
            }
        });
    });
    if delete {
        match asset_ops::delete_asset(root, &path) {
            Ok(()) => {
                st.status = format!("Deleted {path}");
                st.project.selected = None;
                st.project.invalidate();
            }
            Err(e) => st.status = e,
        }
        close = true;
    }
    if close {
        st.project.confirm_delete = None;
    }
}

fn run(st: &mut EditorState, engine: &mut Engine, root: &Path, op: Op) -> Result<(), String> {
    match op {
        Op::Open(path) => open(st, engine, root, &path)?,
        Op::Move(from, to) => {
            st.status = asset_ops::move_asset(st, engine, &from, &to)?;
            st.project.invalidate();
        }
        Op::Rename(path, name) => {
            let to = renamed_path(root, &path, &name)?;
            if to != path {
                st.status = asset_ops::move_asset(st, engine, &path, &to)?;
                st.project.selected = Some(to);
                st.project.invalidate();
            }
        }
        Op::Duplicate(path) => {
            let new = asset_ops::duplicate_asset(root, &path)?;
            st.status = format!("Duplicated {path} as {new}");
            st.project.selected = Some(new);
            st.project.invalidate();
        }
        Op::Reveal(path) => asset_ops::reveal(&root.join(path))?,
        Op::Reimport(path) => {
            let assets = engine.world.resource_mut::<AssetServer>();
            let lower = path.to_lowercase();
            let r = if lower.ends_with(".mat") {
                assets.load_material(&path).map(drop)
            } else if crate::previews::preview_kind(&path)
                == Some(crate::previews::PreviewKind::Texture)
                || lower.ends_with(".ktx2")
            {
                assets.load_texture(&path).map(drop)
            } else if oxide_assets::is_model_path(&lower) {
                assets.load_model(&path).map(drop)
            } else {
                assets.ensure_loaded(&path)
            };
            r?;
            st.previews.clear();
            st.status = format!("Reimported {path}");
        }
        Op::NewFolder(folder) => {
            let path = asset_ops::unique_path(root, &format!("{folder}/New Folder"));
            std::fs::create_dir_all(root.join(&path)).map_err(|e| e.to_string())?;
            st.project.selected = Some(path);
            st.project.start_rename();
            st.project.invalidate();
        }
        Op::ExportPackage(path) => {
            st.actions.push(crate::Action::ExportPackage(vec![path]));
        }
        Op::OpenSceneAdditive(path) => {
            st.actions.push(crate::Action::OpenSceneAdditive(path));
        }
        Op::PrefabVariant(base) => {
            let path = create_prefab_variant(engine, root, &base)?;
            st.project.selected = Some(path);
            st.project.start_rename();
            st.project.invalidate();
        }
        Op::SpriteAsset(texture) => {
            let path = create_sprite_asset(engine, root, &texture)?;
            st.project.selected = Some(path);
            st.project.inspecting = true;
            st.project.start_rename();
            st.project.invalidate();
        }
        Op::FontAsset(font) => {
            let path = create_font_asset(engine, root, &font)?;
            st.project.selected = Some(path);
            st.project.inspecting = true;
            st.project.start_rename();
            st.project.invalidate();
        }
        Op::GraphMaterial(graph) => {
            let stem = graph.trim_end_matches(".shadergraph");
            let path = asset_ops::unique_path(root, &format!("{stem}.mat"));
            let name = path
                .rsplit('/')
                .next()
                .unwrap_or(&path)
                .trim_end_matches(".mat")
                .to_owned();
            let mut m = oxide_assets::Material::lit(&name, oxide_math::Color::WHITE);
            m.shader_graph = Some(oxide_assets::Handle::from_path(graph.clone()));
            let assets = engine.world.resource_mut::<AssetServer>();
            assets.ensure_loaded(&graph)?;
            if let Some(g) = assets.data.get(
                &oxide_assets::Handle::<oxide_shadergraph::ShaderGraph>::from_path(graph.clone()),
            ) {
                g.apply_default_keywords(&mut m);
            }
            assets.save_material(&path, &m)?;
            st.project.selected = Some(path);
            st.project.inspecting = true;
            st.project.invalidate();
        }
        Op::NewAsset(type_name, folder) => {
            let path = new_asset(engine, root, type_name, &folder)?;
            st.project.selected = Some(path);
            st.project.inspecting = true;
            st.project.start_rename();
            st.project.invalidate();
        }
    }
    Ok(())
}

/// Writes a new asset of `type_name` into `folder` and returns its path: data assets from
/// their defaults, the built-in types from Unity-like templates.
pub fn new_asset(
    engine: &mut Engine,
    root: &Path,
    type_name: &str,
    folder: &str,
) -> Result<String, String> {
    if type_name == "UnlitShaderGraph" {
        let path = asset_ops::unique_path(
            root,
            &format!("{folder}/New Unlit Shader Graph.shadergraph"),
        );
        let mut g = oxide_shadergraph::ShaderGraph::default();
        g.settings.material = oxide_shadergraph::GraphMaterial::Unlit;
        engine
            .world
            .resource_mut::<AssetServer>()
            .save_data(&path, Box::new(g))?;
        return Ok(path);
    }
    let info = engine
        .world
        .resource::<AssetServer>()
        .types
        .get(type_name)
        .cloned()
        .ok_or_else(|| format!("unknown asset type `{type_name}`"))?;
    let path = asset_ops::unique_path(root, &format!("{folder}/{}", info.default_file_name()));
    let assets = engine.world.resource_mut::<AssetServer>();
    match type_name {
        "Material" => {
            let m = oxide_assets::Material::lit(info.default_name, oxide_math::Color::WHITE);
            assets.save_material(&path, &m)?;
        }
        "RenderTexture" => {
            assets.save_render_texture(&path, &oxide_assets::RenderTexture::default())?;
        }
        "Shader" => {
            std::fs::write(root.join(&path), FULL_SCREEN_SHADER).map_err(|e| e.to_string())?;
        }
        "VisualTreeAsset" => {
            std::fs::write(root.join(&path), oxide_uitk::uxml::TEMPLATE)
                .map_err(|e| e.to_string())?;
        }
        "StyleSheet" => {
            std::fs::write(root.join(&path), oxide_uitk::USS_TEMPLATE)
                .map_err(|e| e.to_string())?;
        }
        "AssemblyDefinitionAsset" => {
            // Unity names the assembly after the file.
            let name = path
                .rsplit('/')
                .next()
                .unwrap_or(&path)
                .trim_end_matches(".asmdef")
                .to_owned();
            let def = crate::assemblies::AsmDef::new(&name);
            let text = serde_json::to_string_pretty(&def).map_err(|e| e.to_string())?;
            std::fs::write(root.join(&path), text).map_err(|e| e.to_string())?;
        }
        "AssemblyDefinitionReferenceAsset" => {
            std::fs::write(root.join(&path), "{\n    \"reference\": \"\"\n}\n")
                .map_err(|e| e.to_string())?;
        }
        "TextAsset" => {
            std::fs::write(root.join(&path), "").map_err(|e| e.to_string())?;
        }
        "AnimationClip" => {
            let clip = oxide_assets::AnimationClip {
                name: info.default_name.to_owned(),
                looping: true,
                length: 1.0,
                ..Default::default()
            };
            assets.save_animation_clip(&path, &clip)?;
        }
        "AnimatorController" => {
            let mut c = oxide_assets::AnimatorController::default();
            c.base();
            assets.save_animator_controller(&path, &c)?;
        }
        "AnimatorOverrideController" => {
            assets.save_override_controller(&path, &Default::default())?;
        }
        "AvatarMask" => {
            assets.save_avatar_mask(&path, &Default::default())?;
        }
        "Scene" => {
            // Unity's new scene: a Main Camera and a Directional Light.
            let mut w = oxide_scene::World::new();
            oxide_engine::init_world(&mut w, root.to_path_buf());
            oxide_engine::objects::create_default_scene(&mut w);
            oxide_engine::save_scene(&mut w, &path)?;
        }
        _ if assets.types.is_data(type_name) => assets.create_data_asset(type_name, &path)?,
        _ => return Err(format!("{type_name} can't be created from the Create menu")),
    }
    Ok(path)
}

/// TMP's Create → Sprite Asset on a texture: one sprite per Sprite Editor slice (or the
/// whole texture), saved as `<texture>.spriteasset` beside it.
pub fn create_sprite_asset(
    engine: &mut Engine,
    root: &Path,
    texture: &str,
) -> Result<String, String> {
    use oxide_engine::text::SpriteAsset;
    let assets = engine.world.resource_mut::<oxide_assets::AssetServer>();
    assets.ensure_loaded(texture)?;
    let h: oxide_assets::Handle<oxide_assets::Texture2D> =
        oxide_assets::Handle::from_path(texture.to_owned());
    let (w, hgt) = assets
        .textures
        .get(&h)
        .map(|t| (t.width, t.height))
        .ok_or_else(|| format!("{texture} isn't loaded"))?;
    let imp: oxide_assets::TextureImporter = assets.importer(texture);
    let asset = if imp.sprites.is_empty() {
        SpriteAsset::from_grid(h, w, hgt, 1, 1, "sprite")
    } else {
        SpriteAsset::from_slices(h, &imp.sprites)
    };
    let stem = texture.rsplit_once('.').map_or(texture, |(s, _)| s);
    let path = asset_ops::unique_path(root, &format!("{stem}.spriteasset"));
    assets.save_data(&path, Box::new(asset))?;
    Ok(path)
}

/// Unity's Create → Prefab Variant on a prefab asset: `<Name> Variant.prefab` beside it,
/// based on it with no overrides yet.
/// `<font> SDF.fontasset` next to a font file, using it as the source.
pub fn create_font_asset(engine: &mut Engine, root: &Path, font: &str) -> Result<String, String> {
    let stem = font.rsplit_once('.').map_or(font, |(s, _)| s);
    let path = asset_ops::unique_path(root, &format!("{stem} SDF.fontasset"));
    let asset = oxide_engine::text::FontAsset {
        source_font_file: Some(oxide_assets::Handle::from_path(font.to_owned())),
        ..Default::default()
    };
    engine
        .world
        .resource_mut::<oxide_assets::AssetServer>()
        .save_data(&path, Box::new(asset))?;
    Ok(path)
}

pub fn create_prefab_variant(
    engine: &mut Engine,
    root: &Path,
    base: &str,
) -> Result<String, String> {
    let stem = base.trim_end_matches(".prefab");
    let path = asset_ops::unique_path(root, &format!("{stem} Variant.prefab"));
    // Made from a temporary instance in the open world, so every component type is known.
    let w = &mut engine.world;
    let inst = w.instantiate_prefab(base, None)?;
    let r = w.create_prefab_variant(inst, &path);
    // Immediately: Edit mode has no end of frame to flush a deferred Destroy.
    w.destroy_immediate(inst);
    r.map(|()| path)
}

/// Template for Create → Shader → Full Screen Shader (a Full Screen Pass effect).
const FULL_SCREEN_SHADER: &str =
    "// A Full Screen Pass shader: add a Full Screen Pass Renderer Feature to a camera
// and set its Shader to this file. Helpers: sample_source(uv), sample_depth(uv),
// fx.params0 / fx.params1 (the feature's Params), fx.time.x (seconds).
fn fs_main(in: FullscreenIn) -> vec4f {
    let c = sample_source(in.uv);
    // Example: desaturate by params0.x.
    let grey = dot(c.rgb, vec3f(0.2126, 0.7152, 0.0722));
    return vec4f(mix(c.rgb, vec3f(grey), fx.params0.x), c.a);
}
";

/// The path of `path` renamed to `name` (the extension stays, typed or not).
pub fn renamed_path(root: &Path, path: &str, name: &str) -> Result<String, String> {
    let name = asset_ops::check_name(name)?;
    let (dir, file) = path.rsplit_once('/').unwrap_or(("", path));
    let ext = match file.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() && !root.join(path).is_dir() => format!(".{e}"),
        _ => String::new(),
    };
    let new_file = if !ext.is_empty() && name.to_lowercase().ends_with(&ext.to_lowercase()) {
        name.to_owned()
    } else {
        format!("{name}{ext}")
    };
    Ok(format!("{dir}/{new_file}"))
}

/// Double-click / Open: scenes open, models and prefabs are placed in the scene, folders
/// open in the grid, and everything else opens in its app (scripts in the script editor).
pub fn open(
    st: &mut EditorState,
    engine: &mut Engine,
    root: &Path,
    path: &str,
) -> Result<(), String> {
    let full = root.join(path);
    if full.is_dir() {
        st.project.folder = path.to_owned();
        st.project.search.clear();
        return Ok(());
    }
    match kind(path).0 {
        "Scene" => st.actions.push(Action::OpenScene(path.to_owned())),
        "Model" => st.actions.push(Action::InstantiateModel(path.to_owned())),
        "Prefab" => st.actions.push(Action::InstantiatePrefab(path.to_owned())),
        "Material" | "RenderTexture" => {
            st.project.selected = Some(path.to_owned());
            st.project.inspecting = true;
        }
        "InputActionAsset" => {
            st.input_actions.open(engine, path);
            st.actions.push(Action::OpenTab(crate::Tab::InputActions));
        }
        "AnimatorController" => {
            st.animator.open(path);
            st.actions.push(Action::OpenTab(crate::Tab::Animator));
        }
        "AnimationClip" => {
            st.animation.open_clip(path);
            st.actions.push(Action::OpenTab(crate::Tab::Animation));
        }
        "TimelineAsset" => {
            st.timeline.open_asset(&engine.world, path);
            st.actions.push(Action::OpenTab(crate::Tab::Timeline));
        }
        "StringTableCollection" | "AssetTableCollection" => {
            st.localization.open(path);
            st.actions
                .push(Action::OpenTab(crate::Tab::LocalizationTables));
        }
        "VisualTreeAsset" => {
            st.ui_builder
                .open(engine.world.resource::<AssetServer>(), path)?;
            st.actions.push(Action::OpenTab(crate::Tab::UiBuilder));
        }
        "VisualEffectAsset" => {
            st.vfx_graph
                .open(engine.world.resource_mut::<AssetServer>(), path)?;
            st.actions.push(Action::OpenTab(crate::Tab::VfxGraph));
        }
        "ShaderGraph" | "ShaderSubGraph" => {
            st.shader_graph.open(engine, path)?;
            st.actions.push(Action::OpenTab(crate::Tab::ShaderGraph));
        }
        "AudioMixer" => {
            st.audio_mixer.open(path);
            st.actions.push(Action::OpenTab(crate::Tab::AudioMixer));
        }
        "AnimatorOverrideController" | "AvatarMask" => {
            st.project.selected = Some(path.to_owned());
            st.project.inspecting = true;
        }
        // Other data assets are edited in the Inspector.
        t if engine.world.resource::<AssetServer>().types.is_data(t) => {
            st.project.selected = Some(path.to_owned());
            st.project.inspecting = true;
        }
        _ => st.prefs.open_file(&full, 1)?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_scenes() {
        let dir = std::env::temp_dir().join(format!("oxide-project-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Assets/Scenes")).unwrap();
        std::fs::write(dir.join("Assets/Scenes/Main.scene"), "x").unwrap();
        std::fs::write(dir.join("Assets/readme.txt"), "x").unwrap();
        assert_eq!(find_files(&dir, "scene"), vec!["Assets/Scenes/Main.scene"]);
        let top = list_dir(&dir, "Assets");
        assert!(top[0].dir, "folders first");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn search_syntax() {
        let e = |name: &str, labels: &[&str]| Entry {
            name: name.into(),
            path: format!("Assets/{name}"),
            dir: false,
            labels: labels.iter().map(|l| l.to_string()).collect(),
        };
        let car = e("Red Car.prefab", &["Vehicle", "Hero"]);
        let paint = e("Car Paint.mat", &["Vehicle"]);
        let tex = e("car_albedo.png", &[]);
        let q = Query::parse("car t:Prefab");
        assert!(q.matches(&car) && !q.matches(&paint) && !q.matches(&tex));
        let q = Query::parse("l:vehicle");
        assert!(q.matches(&car) && q.matches(&paint) && !q.matches(&tex));
        let q = Query::parse("l:Vehicle l:Hero");
        assert!(q.matches(&car) && !q.matches(&paint));
        // Unity's type names and several types at once.
        let q = Query::parse("t:Texture2D t:Material");
        assert!(q.matches(&tex) && q.matches(&paint) && !q.matches(&car));
        let q = Query::parse("RED");
        assert!(q.matches(&car) && !q.matches(&paint));
        assert_eq!(
            Query::parse("a t:X l:Y").types,
            vec!["x".to_string()],
            "filters are lower-cased"
        );
    }
}
