//! Oxide Localization: Unity's Localization package.
//!
//! Locales are listed in the project's Localization Settings
//! (`ProjectSettings/LocalizationSettings.yaml`). Strings live in String Table Collections
//! (`.stringtable`, every locale's values per key) and assets in Asset Table Collections
//! (`.assettable`). Scripts ask [`Localization::get_string`]; the Localize String Event and
//! Localize Asset Event components keep text and assets in the selected locale.
//! Reference: <https://docs.unity3d.com/Packages/com.unity.localization@1.5/manual/index.html>

pub mod smart;

pub use smart::{Arg, format_str};

use oxide_assets::{AssetServer, AssetTypeInfo, Handle};
use oxide_core::Reflect;
use oxide_scene::{Component, System, World};
use std::collections::HashMap;

/// One locale of the project (Unity `Locale`).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LocaleInfo {
    /// Locale code (`en`, `fr`, `pt-BR`, `zh-Hans`).
    pub code: String,
    pub name: String,
    /// Code of the locale used for missing entries (empty: the project default).
    #[serde(default)]
    pub fallback: String,
}

/// Unity's Localization Settings (Project Settings → Localization).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct LocalizationSettings {
    pub locales: Vec<LocaleInfo>,
    /// The locale used at startup and for missing entries.
    pub default_locale: String,
    /// Start in the system's language when the project has it (Unity's System Locale
    /// Selector before the Specific Locale Selector).
    pub use_system_language: bool,
}

impl Default for LocalizationSettings {
    fn default() -> Self {
        Self {
            locales: vec![LocaleInfo {
                code: "en".into(),
                name: "English".into(),
                fallback: String::new(),
            }],
            default_locale: "en".into(),
            use_system_language: true,
        }
    }
}

/// Common locales for the Locale Generator (code, English name).
pub const COMMON_LOCALES: &[(&str, &str)] = &[
    ("ar", "Arabic"),
    ("bn", "Bengali"),
    ("zh-Hans", "Chinese (Simplified)"),
    ("zh-Hant", "Chinese (Traditional)"),
    ("cs", "Czech"),
    ("nl", "Dutch"),
    ("en", "English"),
    ("fr", "French"),
    ("de", "German"),
    ("el", "Greek"),
    ("hi", "Hindi"),
    ("id", "Indonesian"),
    ("it", "Italian"),
    ("ja", "Japanese"),
    ("ko", "Korean"),
    ("ms", "Malay"),
    ("pl", "Polish"),
    ("pt", "Portuguese"),
    ("pt-BR", "Portuguese (Brazil)"),
    ("ru", "Russian"),
    ("es", "Spanish"),
    ("es-MX", "Spanish (Mexico)"),
    ("sv", "Swedish"),
    ("ta", "Tamil"),
    ("te", "Telugu"),
    ("th", "Thai"),
    ("tr", "Turkish"),
    ("uk", "Ukrainian"),
    ("vi", "Vietnamese"),
];

/// One locale's value of a string entry.
#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct LocalizedValue {
    pub locale: String,
    pub value: String,
    /// Smart String (formatters like `{0:plural:...}`).
    pub smart: bool,
}

/// One key of a String Table Collection.
#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct StringTableEntry {
    pub key: String,
    pub comment: String,
    pub values: Vec<LocalizedValue>,
}

impl StringTableEntry {
    pub fn get(&self, locale: &str) -> Option<&LocalizedValue> {
        self.values
            .iter()
            .find(|v| v.locale == locale && !v.value.is_empty())
    }

    pub fn set(&mut self, locale: &str, value: &str) {
        match self.values.iter_mut().find(|v| v.locale == locale) {
            Some(v) => v.value = value.into(),
            None => self.values.push(LocalizedValue {
                locale: locale.into(),
                value: value.into(),
                smart: false,
            }),
        }
    }
}

/// Unity `StringTableCollection` (`.stringtable`): keys with a value per locale.
#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct StringTableCollection {
    /// The name scripts and components use (empty: the file name).
    pub table_name: String,
    pub entries: Vec<StringTableEntry>,
}

impl oxide_assets::Asset for StringTableCollection {
    const TYPE_NAME: &'static str = "StringTableCollection";
}

impl StringTableCollection {
    pub fn entry(&self, key: &str) -> Option<&StringTableEntry> {
        self.entries.iter().find(|e| e.key == key)
    }

    pub fn entry_mut(&mut self, key: &str) -> &mut StringTableEntry {
        if let Some(i) = self.entries.iter().position(|e| e.key == key) {
            return &mut self.entries[i];
        }
        self.entries.push(StringTableEntry {
            key: key.into(),
            ..Default::default()
        });
        self.entries.last_mut().expect("pushed")
    }

    /// CSV with a `Key` column and one column per locale (Unity's CSV extension).
    pub fn to_csv(&self, locales: &[String]) -> String {
        let q = |s: &str| {
            if s.contains([',', '"', '\n']) {
                format!("\"{}\"", s.replace('"', "\"\""))
            } else {
                s.to_owned()
            }
        };
        let mut out = String::from("Key");
        for l in locales {
            out.push(',');
            out.push_str(&q(l));
        }
        out.push('\n');
        for e in &self.entries {
            out.push_str(&q(&e.key));
            for l in locales {
                out.push(',');
                out.push_str(&q(e
                    .values
                    .iter()
                    .find(|v| v.locale == *l)
                    .map_or("", |v| &v.value)));
            }
            out.push('\n');
        }
        out
    }

    /// Reads CSV written by [`to_csv`](Self::to_csv) (or a spreadsheet): adds and updates
    /// entries. Returns the number of values set.
    pub fn import_csv(&mut self, text: &str) -> Result<usize, String> {
        let rows = parse_csv(text);
        let Some(header) = rows.first() else {
            return Err("the CSV is empty".into());
        };
        if !header
            .first()
            .is_some_and(|h| h.eq_ignore_ascii_case("key"))
        {
            return Err("the first column must be `Key`".into());
        }
        let mut n = 0;
        for row in &rows[1..] {
            let Some(key) = row.first().filter(|k| !k.is_empty()) else {
                continue;
            };
            for (col, locale) in header.iter().enumerate().skip(1) {
                if let Some(v) = row.get(col) {
                    self.entry_mut(key).set(locale, v);
                    n += 1;
                }
            }
        }
        Ok(n)
    }
}

fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                cell.push('"');
                chars.next();
            }
            ('"', _) => quoted = !quoted,
            (',', false) => row.push(std::mem::take(&mut cell)),
            ('\n', false) => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
            }
            ('\r', false) => {}
            (c, _) => cell.push(c),
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        rows.push(row);
    }
    rows
}

/// One locale's asset of an asset entry.
#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct LocalizedAsset {
    pub locale: String,
    /// Asset path (texture, sprite, audio clip, prefab, ...).
    pub asset: String,
}

#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct AssetTableEntry {
    pub key: String,
    pub values: Vec<LocalizedAsset>,
}

/// Unity `AssetTableCollection` (`.assettable`).
#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct AssetTableCollection {
    pub table_name: String,
    pub entries: Vec<AssetTableEntry>,
}

impl oxide_assets::Asset for AssetTableCollection {
    const TYPE_NAME: &'static str = "AssetTableCollection";
}

/// The selected locale and table lookup (Unity `LocalizationSettings` at run time).
#[derive(Default)]
pub struct Localization {
    /// The current locale code (Unity `SelectedLocale`).
    pub selected: String,
    /// Table name → file, refreshed every couple of seconds.
    tables: HashMap<String, String>,
    asset_tables: HashMap<String, String>,
    scanned_at: f64,
    /// Bumped when the locale changes (Unity's `SelectedLocaleChanged`).
    pub generation: u64,
}

fn stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.split('.').next().unwrap_or(name)
}

/// The system language as a locale code (`LANG=fr_FR.UTF-8` → `fr`, `fr-FR`).
pub fn system_language() -> Option<String> {
    let v = std::env::var("LC_ALL")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var("LANG").ok())?;
    let base = v.split('.').next()?.replace('_', "-");
    (!base.is_empty() && base != "C" && base != "POSIX").then_some(base)
}

impl Localization {
    /// The locale to start in: the system language if the project has it, else the
    /// default.
    pub fn startup_locale(settings: &LocalizationSettings) -> String {
        if settings.use_system_language
            && let Some(sys) = system_language()
        {
            let lang = sys.split('-').next().unwrap_or(&sys).to_owned();
            for want in [sys.as_str(), lang.as_str()] {
                if let Some(l) = settings
                    .locales
                    .iter()
                    .find(|l| l.code.eq_ignore_ascii_case(want))
                {
                    return l.code.clone();
                }
            }
        }
        if settings
            .locales
            .iter()
            .any(|l| l.code == settings.default_locale)
        {
            return settings.default_locale.clone();
        }
        settings
            .locales
            .first()
            .map_or_else(|| "en".to_owned(), |l| l.code.clone())
    }

    fn scan(&mut self, assets: &AssetServer, now: f64) {
        if now - self.scanned_at < 2.0 && !self.tables.is_empty() {
            return;
        }
        self.scanned_at = now;
        self.tables.clear();
        self.asset_tables.clear();
        for p in assets.paths_of_type("StringTableCollection") {
            let name = assets
                .data
                .get(&Handle::<StringTableCollection>::from_path(p.clone()))
                .map(|t| t.table_name.clone())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| stem(&p).to_owned());
            self.tables.insert(name, p);
        }
        for p in assets.paths_of_type("AssetTableCollection") {
            let name = assets
                .data
                .get(&Handle::<AssetTableCollection>::from_path(p.clone()))
                .map(|t| t.table_name.clone())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| stem(&p).to_owned());
            self.asset_tables.insert(name, p);
        }
    }

    /// The locales to try for `locale`: itself, its fallbacks, then the default.
    pub fn chain(settings: &LocalizationSettings, locale: &str) -> Vec<String> {
        let mut out = vec![locale.to_owned()];
        let mut cur = locale.to_owned();
        for _ in 0..8 {
            let Some(next) = settings
                .locales
                .iter()
                .find(|l| l.code == cur)
                .map(|l| l.fallback.clone())
                .filter(|f| !f.is_empty() && !out.contains(f))
            else {
                break;
            };
            out.push(next.clone());
            cur = next;
        }
        if !out.contains(&settings.default_locale) {
            out.push(settings.default_locale.clone());
        }
        out
    }

    /// Unity `LocalizationSettings.SelectedLocale = ...`.
    pub fn set_locale(world: &mut World, code: &str) {
        let l = world.resource_mut::<Localization>();
        if l.selected != code {
            l.selected = code.to_owned();
            l.generation += 1;
        }
    }

    pub fn selected_locale(world: &World) -> String {
        world
            .try_resource::<Localization>()
            .map(|l| l.selected.clone())
            .unwrap_or_default()
    }

    /// Unity `LocalizedString.GetLocalizedString(args)`: the entry in the selected locale
    /// (or its fallbacks), formatted. Missing entries give `No translation found for ...`
    /// like Unity.
    pub fn get_string(world: &mut World, table: &str, key: &str, args: &[Arg]) -> String {
        let settings = world
            .try_resource::<LocalizationSettings>()
            .cloned()
            .unwrap_or_default();
        let selected = world
            .try_resource::<Localization>()
            .map(|l| l.selected.clone())
            .unwrap_or_default();
        lookup_string(world, &settings, &selected, table, key, args)
    }

    /// The asset path an asset table entry has in the selected locale (or its fallbacks).
    pub fn get_asset(world: &mut World, table: &str, key: &str) -> Option<String> {
        let settings = world
            .try_resource::<LocalizationSettings>()
            .cloned()
            .unwrap_or_default();
        let selected = Localization::selected_locale(world);
        let now = oxide_core::time::realtime_since_startup();
        let mut loc = world.resources.remove::<Localization>().unwrap_or_default();
        let assets = world.resource_mut::<AssetServer>();
        loc.scan(assets, now);
        let path = loc.asset_tables.get(table).cloned();
        world.resources.insert(loc);
        let path = path?;
        let assets = world.resource_mut::<AssetServer>();
        let t = assets.data_asset(&Handle::<AssetTableCollection>::from_path(path))?;
        let e = t.entries.iter().find(|e| e.key == key)?;
        for l in Localization::chain(&settings, &selected) {
            if let Some(v) = e
                .values
                .iter()
                .find(|v| v.locale == l && !v.asset.is_empty())
            {
                return Some(v.asset.clone());
            }
        }
        None
    }
}

fn lookup_string(
    world: &mut World,
    settings: &LocalizationSettings,
    locale: &str,
    table: &str,
    key: &str,
    args: &[Arg],
) -> String {
    let now = oxide_core::time::realtime_since_startup();
    let mut loc = world.resources.remove::<Localization>().unwrap_or_default();
    let assets = world.resource_mut::<AssetServer>();
    loc.scan(assets, now);
    let path = loc.tables.get(table).cloned();
    world.resources.insert(loc);
    let missing = || format!("No translation found for '{key}' in {table}");
    let Some(path) = path else {
        return missing();
    };
    let assets = world.resource_mut::<AssetServer>();
    let Some(t) = assets.data_asset(&Handle::<StringTableCollection>::from_path(path)) else {
        return missing();
    };
    let Some(e) = t.entry(key) else {
        return missing();
    };
    for l in Localization::chain(settings, locale) {
        if let Some(v) = e.get(&l) {
            return format_str(&v.value, args, v.smart);
        }
    }
    missing()
}

/// What a Localize String Event writes to.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StringTarget {
    /// The text component on the same object (TextMeshPro, TextMeshPro UGUI or UI Text).
    #[default]
    TextComponent,
    /// An element of the UI Document on the same object (by `ui_element` name).
    UIDocument,
}

/// Unity `LocalizeStringEvent`: keeps a text in the selected locale.
#[derive(Reflect, Clone, Debug, PartialEq)]
pub struct LocalizeStringEvent {
    /// String Table Collection name.
    pub table: String,
    pub entry: String,
    /// `{0}`, `{1}`, ... (Unity's Local Variables / arguments).
    pub arguments: Vec<String>,
    pub target: StringTarget,
    /// The UI Document element to update (Target = UIDocument).
    pub ui_element: String,
}

impl Default for LocalizeStringEvent {
    fn default() -> Self {
        Self {
            table: String::new(),
            entry: String::new(),
            arguments: Vec::new(),
            target: StringTarget::TextComponent,
            ui_element: String::new(),
        }
    }
}

impl Component for LocalizeStringEvent {}

/// Unity `LocalizeTextureEvent` / `LocalizeAudioClipEvent` / `LocalizeSpriteEvent`: keeps
/// the object's texture, sprite or clip in the selected locale.
#[derive(Reflect, Clone, Debug, Default, PartialEq)]
pub struct LocalizeAssetEvent {
    /// Asset Table Collection name.
    pub table: String,
    pub entry: String,
}

impl Component for LocalizeAssetEvent {}

/// Registers `.stringtable` and `.assettable`.
pub fn register_assets(assets: &mut AssetServer) {
    assets.register_data_asset::<StringTableCollection>(
        AssetTypeInfo::new("StringTableCollection", &["stringtable"])
            .menu(
                "Localization/String Table Collection",
                "New String Table",
                280,
            )
            .color(150, 210, 250),
    );
    assets.register_data_asset::<AssetTableCollection>(
        AssetTypeInfo::new("AssetTableCollection", &["assettable"])
            .menu(
                "Localization/Asset Table Collection",
                "New Asset Table",
                281,
            )
            .color(250, 210, 150),
    );
}

/// Applies Localize String and Asset Events every frame (Edit mode too, so the editor's
/// locale preview shows).
#[derive(Default)]
pub struct LocalizationSystem;

impl System for LocalizationSystem {
    fn name(&self) -> &'static str {
        "Localization"
    }

    fn always(&mut self, world: &mut World) {
        apply(world);
    }
}

/// Writes every localized text and asset whose value differs from the selected locale's.
pub fn apply(world: &mut World) {
    if !world.resources.contains::<Localization>() {
        let settings = world
            .try_resource::<LocalizationSettings>()
            .cloned()
            .unwrap_or_default();
        world.resources.insert(Localization {
            selected: Localization::startup_locale(&settings),
            ..Default::default()
        });
    }
    let events: Vec<(oxide_scene::GameObjectId, LocalizeStringEvent)> = world
        .query::<LocalizeStringEvent>()
        .filter(|(_, e)| !e.table.is_empty() && !e.entry.is_empty())
        .map(|(id, e)| (id, e.clone()))
        .collect();
    for (id, ev) in events {
        let args: Vec<Arg> = ev.arguments.iter().map(Arg::new).collect();
        let text = Localization::get_string(world, &ev.table, &ev.entry, &args);
        match ev.target {
            StringTarget::TextComponent => {
                if let Some(t) = world.get_component_mut::<oxide_text::TextMeshPro>(id) {
                    if t.text != text {
                        t.text = text;
                    }
                } else if let Some(t) = world.get_component_mut::<oxide_ui::TextMeshProUGUI>(id) {
                    if t.text != text {
                        t.text = text;
                    }
                } else if let Some(t) = world.get_component_mut::<oxide_ui::UIText>(id)
                    && t.text != text
                {
                    t.text = text;
                }
            }
            StringTarget::UIDocument => {
                if let Some(d) = world.get_component_mut::<oxide_uitk::UIDocument>(id)
                    && d.text(&ev.ui_element).as_deref() != Some(text.as_str())
                {
                    d.set_text(&ev.ui_element, &text);
                }
            }
        }
    }
    let assets: Vec<(oxide_scene::GameObjectId, LocalizeAssetEvent)> = world
        .query::<LocalizeAssetEvent>()
        .filter(|(_, e)| !e.table.is_empty() && !e.entry.is_empty())
        .map(|(id, e)| (id, e.clone()))
        .collect();
    for (id, ev) in assets {
        let Some(path) = Localization::get_asset(world, &ev.table, &ev.entry) else {
            continue;
        };
        let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        let is_audio = matches!(ext.as_str(), "wav" | "ogg" | "mp3" | "flac");
        if is_audio {
            if let Some(src) = world.get_component::<oxide_audio::AudioSource>(id)
                && src.clip.as_ref().map(|h| h.path()) != Some(path.as_str())
            {
                if let Err(e) = world.resource_mut::<AssetServer>().ensure_loaded(&path) {
                    oxide_core::debug_log_warning!("{e}");
                    continue;
                }
                let src = world
                    .get_component_mut::<oxide_audio::AudioSource>(id)
                    .expect("checked");
                src.clip = Some(Handle::from_path(path.clone()));
            }
            continue;
        }
        if path.contains('#') || ext == "sprite" {
            if let Some(s) = world.get_component::<oxide_scene::SpriteRenderer>(id)
                && s.sprite.as_ref().map(|h| h.path()) != Some(path.as_str())
            {
                if let Err(e) = world.resource_mut::<AssetServer>().ensure_loaded(&path) {
                    oxide_core::debug_log_warning!("{e}");
                    continue;
                }
                let s = world
                    .get_component_mut::<oxide_scene::SpriteRenderer>(id)
                    .expect("checked");
                s.sprite = Some(Handle::from_path(path.clone()));
            }
            continue;
        }
        if let Some(img) = world.get_component::<oxide_ui::UIImage>(id)
            && img.texture.as_ref().map(|h| h.path()) != Some(path.as_str())
        {
            if let Err(e) = world.resource_mut::<AssetServer>().ensure_loaded(&path) {
                oxide_core::debug_log_warning!("{e}");
                continue;
            }
            let img = world
                .get_component_mut::<oxide_ui::UIImage>(id)
                .expect("checked");
            img.texture = Some(Handle::from_path(path));
        }
    }
}

/// Installs the components and returns the system.
pub fn install(world: &mut World) -> Box<dyn System> {
    world
        .registry
        .register::<LocalizeStringEvent>("Localization/Localize String Event");
    world
        .registry
        .register::<LocalizeAssetEvent>("Localization/Localize Asset Event");
    if !world.resources.contains::<LocalizationSettings>() {
        world.resources.insert(LocalizationSettings::default());
    }
    Box::new(LocalizationSystem)
}

/// Script helpers: `ctx.world.localized("UI", "start", &[])`.
pub trait LocalizationExt {
    fn localized(&mut self, table: &str, key: &str, args: &[Arg]) -> String;
    fn set_locale(&mut self, code: &str);
    fn locale(&self) -> String;
}

impl LocalizationExt for World {
    fn localized(&mut self, table: &str, key: &str, args: &[Arg]) -> String {
        Localization::get_string(self, table, key, args)
    }

    fn set_locale(&mut self, code: &str) {
        Localization::set_locale(self, code);
    }

    fn locale(&self) -> String {
        Localization::selected_locale(self)
    }
}
