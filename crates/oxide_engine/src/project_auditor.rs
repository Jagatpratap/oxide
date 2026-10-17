//! Unity's Project Auditor: static checks over the project's scripts, assets and settings
//! that report likely performance, memory, build size and quality problems, each with an
//! area, a severity, where it is and what to do about it. Nothing is changed.
//! <https://docs.unity3d.com/Manual/project-auditor/project-auditor.html>

use crate::project_settings::ProjectSettings;
use oxide_assets::{AssetServer, TextureCompression, TextureImporterType};
use oxide_scene::World;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Unity `IssueCategory` (the auditor's views).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum Category {
    Code,
    Assets,
    ProjectSettings,
}

/// Unity `Areas`: what an issue costs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum Area {
    CPU,
    GPU,
    Memory,
    BuildSize,
    LoadTime,
    Quality,
}

/// Unity `Severity`, most severe first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub enum Severity {
    Critical,
    Major,
    Moderate,
    Minor,
    Info,
}

/// One finding (Unity `ReportItem` with its `Descriptor`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Issue {
    /// Rule id, e.g. `code.find_in_update`.
    pub id: &'static str,
    pub category: Category,
    pub area: Area,
    pub severity: Severity,
    /// What was found.
    pub title: String,
    /// Project-relative path (`Assets/...`, `Scripts/src/...`, `ProjectSettings/...`).
    pub path: String,
    /// 1-based line for code issues.
    pub line: Option<u32>,
    pub recommendation: &'static str,
}

/// The result of one analysis.
#[derive(Clone, Debug, Default, Serialize)]
pub struct AuditReport {
    pub issues: Vec<Issue>,
    pub scripts_scanned: usize,
    pub assets_scanned: usize,
    pub seconds: f32,
}

impl AuditReport {
    pub fn count(&self, s: Severity) -> usize {
        self.issues.iter().filter(|i| i.severity == s).count()
    }
}

/// Analyzes the project the World runs (scripts in `Scripts/`, assets in `Assets/`,
/// settings as running). `build_scenes` are the scenes a player includes; with them, assets
/// no build scene uses are reported.
pub fn audit(world: &World, build_scenes: &[String]) -> AuditReport {
    let start = std::time::Instant::now();
    let assets = world.resource::<AssetServer>();
    let root = assets.full_path("");
    let mut report = AuditReport::default();
    audit_code(&root, &mut report);
    audit_assets(assets, &root, build_scenes, &mut report);
    audit_settings(world, &mut report);
    report
        .issues
        .sort_by(|a, b| (a.severity, &a.path, a.line).cmp(&(b.severity, &b.path, b.line)));
    report.seconds = start.elapsed().as_secs_f32();
    report
}

/// The scenes in `ProjectSettings/BuildSettings.yaml` (empty when there's no file).
pub fn build_scenes(project: &Path) -> Vec<String> {
    std::fs::read_to_string(project.join("ProjectSettings/BuildSettings.yaml"))
        .ok()
        .and_then(|t| serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&t).ok())
        .and_then(|v| {
            v.get("scenes")?.as_sequence().map(|s| {
                s.iter()
                    .filter_map(|x| x.as_str().map(str::to_owned))
                    .collect()
            })
        })
        .unwrap_or_default()
}

/// [`audit`] with the project's Build Settings scenes.
pub fn audit_project(world: &World) -> AuditReport {
    let root = world.resource::<AssetServer>().full_path("");
    audit(world, &build_scenes(&root))
}

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        let name = e.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name == "target" {
            continue;
        }
        if p.is_dir() {
            files(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

// ---- code ------------------------------------------------------------------------------

/// A code rule: any of `patterns` on a line inside a per-frame callback.
struct CodeRule {
    id: &'static str,
    patterns: &'static [&'static str],
    area: Area,
    severity: Severity,
    title: &'static str,
    recommendation: &'static str,
}

const CODE_RULES: &[CodeRule] = &[
    CodeRule {
        id: "code.find_in_update",
        patterns: &[
            "find_objects_by_type",
            "find_any_object_by_type",
            "find_game_object_with_tag",
            ".find_with_tag(",
            ".find(\"",
            ".find_path(",
        ],
        area: Area::CPU,
        severity: Severity::Major,
        title: "searches the scene every frame",
        recommendation: "Look the object up once in start() and keep its id, or keep a registry \
                         of the objects you need.",
    },
    CodeRule {
        id: "code.get_component_in_update",
        patterns: &["get_component", "get_components"],
        area: Area::CPU,
        severity: Severity::Minor,
        title: "gets a component every frame",
        recommendation: "Cache the component (or its GameObject id) in start().",
    },
    CodeRule {
        id: "code.alloc_in_update",
        patterns: &[
            "format!(",
            "Vec::new()",
            "vec![",
            "String::from(",
            ".to_string()",
            ".to_owned()",
            "Box::new(",
            ".collect::<Vec",
        ],
        area: Area::Memory,
        severity: Severity::Moderate,
        title: "allocates memory every frame",
        recommendation: "Reuse a buffer kept in the component (clear() it each frame) instead \
                         of allocating; build strings only when they change.",
    },
    CodeRule {
        id: "code.log_in_update",
        patterns: &[
            "debug_log!(",
            "debug_log_warning!(",
            "println!(",
            "eprintln!(",
        ],
        area: Area::CPU,
        severity: Severity::Minor,
        title: "logs every frame",
        recommendation: "Log on changes only; per-frame logging is slow and floods the Console.",
    },
    CodeRule {
        id: "code.instantiate_in_update",
        patterns: &["instantiate(", "instantiate_at(", "instantiate_prefab"],
        area: Area::CPU,
        severity: Severity::Moderate,
        title: "instantiates objects in a per-frame callback",
        recommendation: "Pool objects that are created and destroyed often (bullets, effects) \
                         instead of instantiating them each time.",
    },
    CodeRule {
        id: "code.messages_in_update",
        patterns: &["send_message(", "broadcast_message("],
        area: Area::CPU,
        severity: Severity::Minor,
        title: "sends messages by name every frame",
        recommendation: "Call the component directly; messages look methods up by name.",
    },
    CodeRule {
        id: "code.blocking_in_update",
        patterns: &["std::thread::sleep", "thread::sleep(", ".join()"],
        area: Area::CPU,
        severity: Severity::Critical,
        title: "blocks the main thread in a per-frame callback",
        recommendation: "Never wait on the main thread: use a coroutine, a timer or the job system.",
    },
    CodeRule {
        id: "code.file_io_in_update",
        patterns: &[
            "std::fs::",
            "fs::read",
            "fs::write",
            "File::open(",
            "File::create(",
        ],
        area: Area::LoadTime,
        severity: Severity::Major,
        title: "reads or writes files in a per-frame callback",
        recommendation: "Load data at startup (or on a worker thread) and keep it in memory.",
    },
];

/// Per-frame callbacks (Unity's Update family).
const HOT_FNS: [&str; 4] = [
    "fn update(",
    "fn fixed_update(",
    "fn late_update(",
    "fn on_gui(",
];

/// Code issues in one Rust file's text.
pub fn audit_source(path: &str, text: &str) -> Vec<Issue> {
    let mut out = Vec::new();
    // The brace depth at which the current per-frame callback started.
    let mut hot: Option<(i32, &str)> = None;
    let mut depth = 0i32;
    let mut in_block_comment = false;
    for (n, raw) in text.lines().enumerate() {
        let mut line = raw;
        if in_block_comment {
            match line.find("*/") {
                Some(i) => {
                    in_block_comment = false;
                    line = &line[i + 2..];
                }
                None => continue,
            }
        }
        let code = line.split("//").next().unwrap_or("");
        let code = match code.find("/*") {
            Some(i) => {
                in_block_comment = !code[i..].contains("*/");
                &code[..i]
            }
            None => code,
        };
        if hot.is_none()
            && let Some(f) = HOT_FNS.iter().find(|f| code.contains(**f))
        {
            hot = Some((depth, &f[3..f.len() - 1]));
        }
        if let Some((_, callback)) = hot {
            for r in CODE_RULES {
                if r.patterns.iter().any(|p| code.contains(p)) {
                    out.push(Issue {
                        id: r.id,
                        category: Category::Code,
                        area: r.area,
                        severity: r.severity,
                        title: format!("{callback}() {}", r.title),
                        path: path.to_owned(),
                        line: Some(n as u32 + 1),
                        recommendation: r.recommendation,
                    });
                }
            }
        }
        for c in code.chars() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if hot.is_some_and(|(d, _)| depth <= d) {
                        hot = None;
                    }
                }
                _ => {}
            }
        }
    }
    out
}

fn audit_code(root: &Path, report: &mut AuditReport) {
    let mut all = Vec::new();
    files(&root.join("Scripts"), &mut all);
    for p in all
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e == "rs"))
    {
        let Ok(text) = std::fs::read_to_string(p) else {
            continue;
        };
        report.scripts_scanned += 1;
        report.issues.extend(audit_source(&rel(root, p), &text));
    }
}

// ---- assets ----------------------------------------------------------------------------

const TEXTURE_EXT: [&str; 12] = [
    "png", "jpg", "jpeg", "tga", "bmp", "psd", "exr", "hdr", "tif", "tiff", "webp", "gif",
];
const AUDIO_EXT: [&str; 4] = ["wav", "ogg", "mp3", "flac"];
const MODEL_EXT: [&str; 4] = ["fbx", "obj", "gltf", "glb"];

fn asset_issue(
    id: &'static str,
    area: Area,
    severity: Severity,
    path: &str,
    title: String,
    recommendation: &'static str,
) -> Issue {
    Issue {
        id,
        category: Category::Assets,
        area,
        severity,
        title,
        path: path.to_owned(),
        line: None,
        recommendation,
    }
}

fn audit_assets(
    assets: &AssetServer,
    root: &Path,
    build_scenes: &[String],
    report: &mut AuditReport,
) {
    let mut all = Vec::new();
    files(&root.join("Assets"), &mut all);
    all.retain(|p| p.extension().is_none_or(|e| e != "meta"));
    // Everything the build scenes use (with the database's dependency tracking).
    let used: Option<std::collections::HashSet<String>> =
        (!build_scenes.is_empty() && assets.database.is_scanned()).then(|| {
            build_scenes
                .iter()
                .flat_map(|s| {
                    let mut d = assets.database.dependencies(s, true);
                    d.push(s.clone());
                    d
                })
                .collect()
        });
    for p in &all {
        report.assets_scanned += 1;
        let path = rel(root, p);
        let ext = p
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let bytes = std::fs::metadata(p).map_or(0, |m| m.len());
        if TEXTURE_EXT.contains(&ext.as_str()) {
            texture_issues(assets, p, &path, report);
        } else if AUDIO_EXT.contains(&ext.as_str()) {
            let imp: oxide_assets::importers::AudioImporter = assets.importer(&path);
            if imp.preload_audio_data && bytes > 4 << 20 {
                report.issues.push(asset_issue(
                    "asset.audio_preload_large",
                    Area::LoadTime,
                    Severity::Moderate,
                    &path,
                    format!(
                        "{} MB audio clip is decoded when the scene loads",
                        bytes >> 20
                    ),
                    "Turn off Preload Audio Data for long clips (music, ambience) so they \
                     decode on first play.",
                ));
            }
            if ext == "wav" && bytes > 1 << 20 {
                report.issues.push(asset_issue(
                    "asset.audio_uncompressed",
                    Area::BuildSize,
                    Severity::Minor,
                    &path,
                    format!("uncompressed WAV ({:.1} MB)", bytes as f64 / 1048576.0),
                    "Use Ogg Vorbis for long clips; WAV is best for short, frequent sounds.",
                ));
            }
        } else if MODEL_EXT.contains(&ext.as_str()) {
            let imp: oxide_assets::importers::ModelImporter = assets.importer(&path);
            if imp.generate_colliders {
                report.issues.push(asset_issue(
                    "asset.model_mesh_colliders",
                    Area::CPU,
                    Severity::Moderate,
                    &path,
                    "Generate Colliders adds a Mesh Collider to every mesh".into(),
                    "Use primitive colliders or a simplified collision mesh; detailed mesh \
                     colliders are slow.",
                ));
            }
            if bytes > 50 << 20 {
                report.issues.push(asset_issue(
                    "asset.model_large",
                    Area::BuildSize,
                    Severity::Info,
                    &path,
                    format!("{} MB model file", bytes >> 20),
                    "Split it, remove unused meshes and animations, or add LOD levels.",
                ));
            }
        }
        // Not used by any build scene (scenes, scripts-only data and Resources excepted).
        if let Some(used) = &used
            && !used.contains(&path)
            && ext != "scene"
            && !path.contains("/Resources/")
            && !path.starts_with("Assets/StreamingAssets/")
            && !path.contains("/Editor/")
        {
            report.issues.push(asset_issue(
                "asset.unused",
                Area::BuildSize,
                Severity::Info,
                &path,
                "not used by any scene in Build Settings".into(),
                "Delete it if it isn't loaded from code; content cooking leaves it out of \
                 builds.",
            ));
        }
    }
}

fn texture_issues(assets: &AssetServer, file: &Path, path: &str, report: &mut AuditReport) {
    let imp = assets.texture_importer(path);
    let Ok((w, h)) = image::image_dimensions(file) else {
        return;
    };
    let big = w.max(h);
    let compressed = imp.compression != TextureCompression::None;
    let sprite = imp.texture_type == TextureImporterType::Sprite;
    let mut push = |id, area, severity, title: String, rec| {
        report
            .issues
            .push(asset_issue(id, area, severity, path, title, rec))
    };
    if !compressed && big > 512 {
        push(
            "asset.texture_uncompressed",
            Area::Memory,
            Severity::Major,
            format!(
                "{w}×{h} texture is not compressed ({} MB on the GPU)",
                (w as u64 * h as u64 * 4) >> 20
            ),
            "Set Compression to Normal Quality (BC7 / ASTC): 4–8× less memory.",
        );
    }
    if compressed && (w % 4 != 0 || h % 4 != 0) {
        push(
            "asset.texture_not_block_size",
            Area::Memory,
            Severity::Moderate,
            format!("{w}×{h} isn't a multiple of 4, so it can't be block-compressed"),
            "Resize it to multiples of 4 (powers of two are best).",
        );
    }
    if big > 2048 && imp.max_size > 2048 {
        push(
            "asset.texture_large",
            Area::Memory,
            Severity::Moderate,
            format!("{w}×{h} texture imported at full size"),
            "Lower Max Size unless it fills the screen up close; 2048 is plenty for most \
             surfaces.",
        );
    }
    if !imp.generate_mip_maps && !sprite {
        push(
            "asset.texture_no_mipmaps",
            Area::GPU,
            Severity::Moderate,
            "3D texture without mipmaps (shimmers at a distance and wastes bandwidth)".into(),
            "Turn on Generate Mip Maps for textures used on 3D surfaces.",
        );
    }
    if imp.generate_mip_maps && sprite {
        push(
            "asset.sprite_mipmaps",
            Area::Memory,
            Severity::Minor,
            "sprite has mipmaps (+33% memory)".into(),
            "Turn off Generate Mip Maps for UI and 2D sprites shown at their own size.",
        );
    }
    let stem = file
        .file_stem()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if imp.texture_type == TextureImporterType::Default
        && (stem.ends_with("_n") || stem.ends_with("_normal") || stem.ends_with("_nrm"))
    {
        push(
            "asset.normal_map_type",
            Area::Quality,
            Severity::Moderate,
            "looks like a normal map but is imported as a color texture".into(),
            "Set Texture Type to Normal Map (linear, BC5).",
        );
    }
}

// ---- settings --------------------------------------------------------------------------

fn setting(
    id: &'static str,
    area: Area,
    severity: Severity,
    file: &str,
    title: String,
    recommendation: &'static str,
) -> Issue {
    Issue {
        id,
        category: Category::ProjectSettings,
        area,
        severity,
        title,
        path: format!("ProjectSettings/{file}"),
        line: None,
        recommendation,
    }
}

fn audit_settings(world: &World, report: &mut AuditReport) {
    let s = ProjectSettings::capture(world);
    let out = &mut report.issues;
    if s.time.fixed_timestep < 0.01 {
        out.push(setting(
            "settings.fixed_timestep",
            Area::CPU,
            Severity::Moderate,
            "TimeManager.yaml",
            format!(
                "Fixed Timestep {} s runs physics {:.0} times a second",
                s.time.fixed_timestep,
                1.0 / s.time.fixed_timestep
            ),
            "Use 0.02 (50 Hz) unless fast vehicles need more; every step costs CPU.",
        ));
    }
    if s.time.maximum_allowed_timestep > 0.34 {
        out.push(setting(
            "settings.max_timestep",
            Area::CPU,
            Severity::Minor,
            "TimeManager.yaml",
            "Maximum Allowed Timestep lets a slow frame run many physics steps".into(),
            "Keep it at 1/3 s or less so a hitch can't snowball into a spiral of physics steps.",
        ));
    }
    let used_layers = s
        .tags_and_layers
        .layers
        .iter()
        .filter(|l| !l.is_empty())
        .count();
    if used_layers > 6
        && s.physics
            .layer_collision_matrix
            .iter()
            .all(|r| *r == u32::MAX)
    {
        out.push(setting(
            "settings.collision_matrix",
            Area::CPU,
            Severity::Minor,
            "DynamicsManager.yaml",
            format!("{used_layers} layers and every layer collides with every other"),
            "Untick layer pairs that never need to collide in the Layer Collision Matrix.",
        ));
    }
    let q = &s.quality;
    for (target, name) in [
        (oxide_assets::BuildTarget::Android, "Android"),
        (oxide_assets::BuildTarget::IOS, "iOS"),
    ] {
        let Some(l) = q.levels.get(q.default_for(target)) else {
            continue;
        };
        if l.msaa == oxide_scene::MsaaQuality::Msaa8x || l.shadow_distance > 150.0 {
            out.push(setting(
                "settings.mobile_quality",
                Area::GPU,
                Severity::Major,
                "QualitySettings.yaml",
                format!(
                    "{name} starts on \"{}\" ({:?}, {} m shadows)",
                    l.name, l.msaa, l.shadow_distance
                ),
                "Give mobile platforms a level with at most 4× MSAA and a short shadow distance.",
            ));
        }
    }
    for l in &q.levels {
        if l.render_scale > 1.0 {
            out.push(setting(
                "settings.render_scale",
                Area::GPU,
                Severity::Moderate,
                "QualitySettings.yaml",
                format!(
                    "\"{}\" renders at {:.0}% scale",
                    l.name,
                    l.render_scale * 100.0
                ),
                "Render scales above 1 supersample every pixel; prefer anti-aliasing.",
            ));
        }
    }
    if !s.adaptive_performance.enabled
        && q.platform_defaults
            .keys()
            .any(|k| k == "Android" || k == "iOS")
    {
        out.push(setting(
            "settings.adaptive_performance_off",
            Area::CPU,
            Severity::Info,
            "AdaptivePerformance.yaml",
            "Adaptive Performance is off".into(),
            "Turn it on so mobile builds lower quality before the device throttles.",
        ));
    }
    if let Some(p) = oxide_scene::GraphicsSettings::current(world)
        .and_then(|h| world.resource::<AssetServer>().data.get(&h).cloned())
        && p.rendering_path == oxide_scene::RenderingPath::Deferred
        && oxide_scene::QualitySettings::of(world).msaa != oxide_scene::MsaaQuality::Disabled
    {
        out.push(setting(
            "settings.deferred_msaa",
            Area::Quality,
            Severity::Info,
            "QualitySettings.yaml",
            "MSAA is set but the Deferred path ignores it".into(),
            "Use FXAA, SMAA or TAA on the camera, or the Forward+ path for MSAA.",
        ));
    }
    if s.player.company_name == "DefaultCompany" {
        out.push(setting(
            "settings.default_company",
            Area::Quality,
            Severity::Info,
            "PlayerSettings.yaml",
            "Company Name is still \"DefaultCompany\"".into(),
            "Set it in Player settings; it names the folder for saves and PlayerPrefs.",
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_rules_only_fire_in_per_frame_callbacks() {
        let src = r#"
impl Component for Gun {
    fn start(&mut self, ctx: &mut Ctx) {
        self.target = ctx.find("Player"); // fine here
        let names = Vec::new();
    }
    fn update(&mut self, ctx: &mut Ctx) {
        let p = ctx.find("Player");
        // ctx.get_component::<Rigidbody>() in a comment
        let label = format!("{}", self.ammo);
        if self.fire {
            ctx.instantiate(self.bullet);
        }
        /* std::thread::sleep(
           blocked */
    }
    fn on_disable(&mut self, _ctx: &mut Ctx) {
        println!("off");
    }
}
"#;
        let issues = audit_source("Scripts/src/gun.rs", src);
        let ids: Vec<(&str, u32)> = issues.iter().map(|i| (i.id, i.line.unwrap())).collect();
        assert_eq!(
            ids,
            vec![
                ("code.find_in_update", 8),
                ("code.alloc_in_update", 10),
                ("code.instantiate_in_update", 12),
            ]
        );
        assert_eq!(issues[0].title, "update() searches the scene every frame");
    }

    #[test]
    fn audits_assets_and_settings() {
        let dir = std::env::temp_dir().join(format!("oxide_auditor_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Assets/Art")).unwrap();
        std::fs::create_dir_all(dir.join("Scripts/src")).unwrap();
        let png = |p: &str, w: u32, h: u32| {
            image::RgbaImage::new(w, h).save(dir.join(p)).unwrap();
        };
        png("Assets/Art/Big.png", 1024, 1024);
        png("Assets/Art/Odd.png", 30, 30);
        png("Assets/Art/Wall_n.png", 64, 64);
        png("Assets/Art/Flat.png", 64, 64);
        std::fs::write(
            dir.join("Scripts/src/lib.rs"),
            "fn update(&mut self, ctx: &mut Ctx) { std::thread::sleep(d); }\n",
        )
        .unwrap();

        let mut w = World::new();
        crate::engine::init_world(&mut w, dir.clone());
        {
            let assets = w.resource_mut::<AssetServer>();
            let mut imp = assets.texture_importer("Assets/Art/Big.png");
            imp.compression = TextureCompression::None;
            assets
                .set_texture_importer("Assets/Art/Big.png", &imp)
                .unwrap();
            let mut imp = assets.texture_importer("Assets/Art/Flat.png");
            imp.generate_mip_maps = false;
            assets
                .set_texture_importer("Assets/Art/Flat.png", &imp)
                .unwrap();
            // Named like a normal map but explicitly set back to a color texture.
            let mut imp = assets.texture_importer("Assets/Art/Wall_n.png");
            imp.texture_type = TextureImporterType::Default;
            imp.srgb = true;
            assets
                .set_texture_importer("Assets/Art/Wall_n.png", &imp)
                .unwrap();
        }
        let mut q = oxide_scene::QualitySettings::default();
        q.set_quality_level(5);
        w.resources.insert(q);
        w.resource_mut::<oxide_core::Time>()
            .set_fixed_delta_time(0.005);

        let r = audit(&w, &[]);
        let has = |id: &str, path: &str| {
            r.issues
                .iter()
                .any(|i| i.id == id && i.path.ends_with(path))
        };
        assert!(
            has("asset.texture_uncompressed", "Big.png"),
            "{:#?}",
            r.issues
        );
        assert!(has("asset.texture_not_block_size", "Odd.png"));
        assert!(has("asset.normal_map_type", "Wall_n.png"));
        assert!(has("asset.texture_no_mipmaps", "Flat.png"));
        assert!(!has("asset.texture_uncompressed", "Flat.png"));
        assert!(has("code.blocking_in_update", "lib.rs"));
        assert!(has("settings.fixed_timestep", "TimeManager.yaml"));
        assert_eq!(r.scripts_scanned, 1);
        // Most severe first.
        assert_eq!(r.issues[0].severity, Severity::Critical);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
