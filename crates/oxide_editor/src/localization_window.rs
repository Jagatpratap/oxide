//! Localization: the Localization Tables window (keys × locales grid for String and Asset
//! Table Collections, CSV import/export, the editor's preview locale) and the Localization
//! page of Project Settings.
//! Reference: <https://docs.unity3d.com/Packages/com.unity.localization@1.5/manual/LocalizationTablesWindow.html>

use crate::EditorState;
use egui::{Color32, RichText};
use oxide_assets::{AssetServer, Handle};
use oxide_engine::Engine;
use oxide_localization::{
    AssetTableCollection, COMMON_LOCALES, LocaleInfo, Localization, LocalizationSettings,
    StringTableCollection,
};

#[derive(Default)]
pub struct LocalizationWindow {
    /// The open table collection.
    pub path: Option<String>,
    search: String,
    new_key: String,
    new_table: String,
    tables_at: f64,
    tables: Vec<String>,
    pub status: String,
}

impl LocalizationWindow {
    pub fn open(&mut self, path: &str) {
        self.path = Some(path.to_owned());
    }
}

fn locales(engine: &Engine) -> Vec<String> {
    engine
        .world
        .try_resource::<LocalizationSettings>()
        .map(|s| s.locales.iter().map(|l| l.code.clone()).collect())
        .unwrap_or_default()
}

pub fn window_ui(st: &mut EditorState, engine: &mut Engine, ui: &mut egui::Ui) {
    let w = &mut st.localization;
    let now = oxide_core::time::realtime_since_startup();
    if now - w.tables_at > 2.0 {
        let assets = engine.world.resource::<AssetServer>();
        w.tables = assets
            .paths_of_type("StringTableCollection")
            .into_iter()
            .chain(assets.paths_of_type("AssetTableCollection"))
            .collect();
        w.tables.sort();
        w.tables.dedup();
        w.tables_at = now;
        if w.path.is_none() {
            w.path = w.tables.first().cloned();
        }
    }
    let codes = locales(engine);
    // ---- Toolbar.
    ui.horizontal(|ui| {
        ui.label("Table Collection");
        egui::ComboBox::from_id_salt("loc-table")
            .selected_text(
                w.path
                    .as_deref()
                    .map_or("None", |p| p.rsplit('/').next().unwrap_or(p)),
            )
            .show_ui(ui, |ui| {
                for t in &w.tables {
                    if ui
                        .selectable_label(
                            w.path.as_deref() == Some(t),
                            t.rsplit('/').next().unwrap_or(t),
                        )
                        .clicked()
                    {
                        w.path = Some(t.clone());
                    }
                }
            });
        ui.separator();
        ui.add(
            egui::TextEdit::singleline(&mut w.new_table)
                .hint_text("New table name")
                .desired_width(120.0),
        );
        if ui.button("New String Table").clicked() && !w.new_table.trim().is_empty() {
            let name = w.new_table.trim().to_owned();
            let path = format!("Assets/Localization/{name}.stringtable");
            let t = StringTableCollection {
                table_name: name,
                entries: Vec::new(),
            };
            match engine
                .world
                .resource_mut::<AssetServer>()
                .save_data(&path, Box::new(t))
            {
                Ok(()) => {
                    w.path = Some(path);
                    w.tables_at = 0.0;
                    w.new_table.clear();
                }
                Err(e) => w.status = e,
            }
        }
        ui.separator();
        // The editor's preview locale (Unity's Game view Locale menu).
        let selected = Localization::selected_locale(&engine.world);
        ui.label("Locale");
        egui::ComboBox::from_id_salt("loc-preview")
            .selected_text(&selected)
            .show_ui(ui, |ui| {
                for c in &codes {
                    if ui.selectable_label(*c == selected, c).clicked() {
                        Localization::set_locale(&mut engine.world, c);
                    }
                }
            });
        ui.separator();
        ui.add(
            egui::TextEdit::singleline(&mut w.search)
                .hint_text("Search")
                .desired_width(120.0),
        );
    });
    if !w.status.is_empty() {
        ui.colored_label(Color32::from_rgb(230, 180, 90), &w.status);
    }
    ui.separator();
    let Some(path) = w.path.clone() else {
        ui.weak(
            "Create a String Table Collection (Assets → Create → Localization) or name one above.",
        );
        return;
    };
    let assets = engine.world.resource_mut::<AssetServer>();
    if !assets.data.contains(&path)
        && let Err(e) = assets.load_data(&path)
    {
        ui.colored_label(Color32::from_rgb(230, 120, 100), e);
        return;
    }
    if path.ends_with(".assettable") {
        asset_table(ui, w, engine, &path, &codes);
        return;
    }
    let Some(mut table) = engine
        .world
        .resource::<AssetServer>()
        .data
        .get(&Handle::<StringTableCollection>::from_path(path.clone()))
        .cloned()
    else {
        return;
    };
    let before = table.clone();
    // ---- CSV.
    ui.horizontal(|ui| {
        let csv_path = path.trim_end_matches(".stringtable").to_owned() + ".csv";
        if ui.button("Export CSV").on_hover_text(&csv_path).clicked() {
            let full = engine.world.resource::<AssetServer>().full_path(&csv_path);
            w.status = match std::fs::write(&full, table.to_csv(&codes)) {
                Ok(()) => format!("Exported {csv_path}"),
                Err(e) => e.to_string(),
            };
        }
        if ui.button("Import CSV").on_hover_text(&csv_path).clicked() {
            let full = engine.world.resource::<AssetServer>().full_path(&csv_path);
            w.status = match std::fs::read_to_string(&full)
                .map_err(|e| format!("{csv_path}: {e}"))
                .and_then(|t| table.import_csv(&t))
            {
                Ok(n) => format!("Imported {n} values from {csv_path}"),
                Err(e) => e,
            };
        }
        ui.weak(format!("{} entries", table.entries.len()));
    });
    // ---- The grid.
    let q = w.search.to_ascii_lowercase();
    let mut remove = None;
    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new("loc-grid")
                .striped(true)
                .min_col_width(120.0)
                .show(ui, |ui| {
                    ui.label(RichText::new("Key").strong());
                    for c in &codes {
                        ui.label(RichText::new(c).strong());
                    }
                    ui.label("");
                    ui.end_row();
                    for (k, e) in table.entries.iter_mut().enumerate() {
                        if !q.is_empty()
                            && !e.key.to_ascii_lowercase().contains(&q)
                            && !e
                                .values
                                .iter()
                                .any(|v| v.value.to_ascii_lowercase().contains(&q))
                        {
                            continue;
                        }
                        ui.add(egui::TextEdit::singleline(&mut e.key).desired_width(140.0));
                        for c in &codes {
                            let mut v = e
                                .values
                                .iter()
                                .find(|x| x.locale == *c)
                                .map(|x| x.value.clone())
                                .unwrap_or_default();
                            let smart = e
                                .values
                                .iter()
                                .find(|x| x.locale == *c)
                                .is_some_and(|x| x.smart);
                            ui.horizontal(|ui| {
                                if ui
                                    .add(
                                        egui::TextEdit::multiline(&mut v)
                                            .desired_rows(1)
                                            .desired_width(180.0),
                                    )
                                    .changed()
                                {
                                    e.set(c, &v);
                                }
                                let mut s = smart;
                                if ui
                                    .checkbox(&mut s, "")
                                    .on_hover_text("Smart String")
                                    .changed()
                                    && let Some(x) = e.values.iter_mut().find(|x| x.locale == *c)
                                {
                                    x.smart = s;
                                }
                            });
                        }
                        if ui.small_button("×").on_hover_text("Remove entry").clicked() {
                            remove = Some(k);
                        }
                        ui.end_row();
                    }
                });
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut w.new_key)
                        .hint_text("New key")
                        .desired_width(140.0),
                );
                if ui.button("Add New Entry").clicked() && !w.new_key.trim().is_empty() {
                    table.entry_mut(w.new_key.trim());
                    w.new_key.clear();
                }
            });
        });
    if let Some(k) = remove {
        table.entries.remove(k);
    }
    if table != before
        && let Err(e) = engine
            .world
            .resource_mut::<AssetServer>()
            .save_data(&path, Box::new(table))
    {
        w.status = e;
    }
}

fn asset_table(
    ui: &mut egui::Ui,
    w: &mut LocalizationWindow,
    engine: &mut Engine,
    path: &str,
    codes: &[String],
) {
    let Some(mut table) = engine
        .world
        .resource::<AssetServer>()
        .data
        .get(&Handle::<AssetTableCollection>::from_path(path.to_owned()))
        .cloned()
    else {
        return;
    };
    let before = table.clone();
    let mut remove = None;
    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new("loc-asset-grid")
                .striped(true)
                .min_col_width(140.0)
                .show(ui, |ui| {
                    ui.label(RichText::new("Key").strong());
                    for c in codes {
                        ui.label(RichText::new(c).strong());
                    }
                    ui.label("");
                    ui.end_row();
                    for (k, e) in table.entries.iter_mut().enumerate() {
                        ui.add(egui::TextEdit::singleline(&mut e.key).desired_width(140.0));
                        for c in codes {
                            let cur = e
                                .values
                                .iter()
                                .find(|x| x.locale == *c)
                                .map(|x| x.asset.clone())
                                .unwrap_or_default();
                            let mut v = if cur.is_empty() {
                                oxide_core::reflect::Value::Null
                            } else {
                                oxide_core::reflect::Value::Asset(cur)
                            };
                            if crate::inspector::asset_field(
                                ui,
                                &engine.world,
                                None,
                                &mut v,
                                &format!("loc-asset-{k}-{c}"),
                            ) {
                                let p = match v {
                                    oxide_core::reflect::Value::Asset(p) => p,
                                    _ => String::new(),
                                };
                                match e.values.iter_mut().find(|x| x.locale == *c) {
                                    Some(x) => x.asset = p,
                                    None => e.values.push(oxide_localization::LocalizedAsset {
                                        locale: c.clone(),
                                        asset: p,
                                    }),
                                }
                            }
                        }
                        if ui.small_button("×").clicked() {
                            remove = Some(k);
                        }
                        ui.end_row();
                    }
                });
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut w.new_key)
                        .hint_text("New key")
                        .desired_width(140.0),
                );
                if ui.button("Add New Entry").clicked() && !w.new_key.trim().is_empty() {
                    table.entries.push(oxide_localization::AssetTableEntry {
                        key: w.new_key.trim().to_owned(),
                        values: Vec::new(),
                    });
                    w.new_key.clear();
                }
            });
        });
    if let Some(k) = remove {
        table.entries.remove(k);
    }
    if table != before
        && let Err(e) = engine
            .world
            .resource_mut::<AssetServer>()
            .save_data(path, Box::new(table))
    {
        w.status = e;
    }
}

/// Project Settings → Localization: the project's locales and startup locale.
pub fn settings_page(ui: &mut egui::Ui, s: &mut LocalizationSettings) {
    ui.label(RichText::new("Available Locales").strong());
    let mut remove = None;
    let codes: Vec<String> = s.locales.iter().map(|l| l.code.clone()).collect();
    egui::Grid::new("loc-locales").striped(true).show(ui, |ui| {
        ui.weak("Code");
        ui.weak("Name");
        ui.weak("Fallback");
        ui.end_row();
        for (k, l) in s.locales.iter_mut().enumerate() {
            ui.add(egui::TextEdit::singleline(&mut l.code).desired_width(70.0));
            ui.add(egui::TextEdit::singleline(&mut l.name).desired_width(150.0));
            egui::ComboBox::from_id_salt(("loc-fallback", k))
                .selected_text(if l.fallback.is_empty() {
                    "None"
                } else {
                    &l.fallback
                })
                .show_ui(ui, |ui| {
                    if ui.selectable_label(l.fallback.is_empty(), "None").clicked() {
                        l.fallback.clear();
                    }
                    for c in codes.iter().filter(|c| **c != l.code) {
                        if ui.selectable_label(l.fallback == *c, c).clicked() {
                            l.fallback = c.clone();
                        }
                    }
                });
            if ui.small_button("×").clicked() {
                remove = Some(k);
            }
            ui.end_row();
        }
    });
    if let Some(k) = remove
        && s.locales.len() > 1
    {
        s.locales.remove(k);
    }
    // Unity's Locale Generator.
    ui.menu_button("Add Locale", |ui| {
        egui::ScrollArea::vertical()
            .max_height(300.0)
            .show(ui, |ui| {
                for (code, name) in COMMON_LOCALES {
                    let has = s.locales.iter().any(|l| l.code == *code);
                    if ui
                        .add_enabled(!has, egui::Button::new(format!("{name} ({code})")))
                        .clicked()
                    {
                        s.locales.push(LocaleInfo {
                            code: (*code).into(),
                            name: (*name).into(),
                            fallback: String::new(),
                        });
                        ui.close();
                    }
                }
            });
    });
    ui.separator();
    ui.label(RichText::new("Locale Selectors").strong());
    ui.checkbox(
        &mut s.use_system_language,
        "System Locale Selector (start in the system's language)",
    );
    ui.horizontal(|ui| {
        crate::inspector::prop_label(ui, "Specific Locale / Default");
        egui::ComboBox::from_id_salt("loc-default")
            .selected_text(&s.default_locale)
            .show_ui(ui, |ui| {
                for c in &codes {
                    if ui.selectable_label(s.default_locale == *c, c).clicked() {
                        s.default_locale = c.clone();
                    }
                }
            });
    });
}
