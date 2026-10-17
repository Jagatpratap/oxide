//! MCP tool for Localization: locales, string tables and the selected locale.

use crate::bus::Bus;
use crate::mcp_tools::{schema, tool};
use oxide_assets::{AssetServer, Handle};
use oxide_localization::{Arg, Localization, LocalizationSettings, StringTableCollection};
use oxide_mcp::{McpServer, ToolDef, ToolResult, json};
use serde_json::Value as Json;
use std::sync::Arc;

pub fn register(s: &mut McpServer, bus: &Arc<Bus>) {
    tool(
        s,
        bus,
        ToolDef::new(
            "localization",
            "Localization",
            "Unity Localization: returns the project's locales, the selected locale and every String Table \
             Collection (name, path, keys). `set_locale` switches the selected locale (Localize String/Asset \
             Events update at once). `get` {table, key, args?: [..]} resolves a string in the selected locale \
             (with fallbacks and Smart String formatting). `set` [{table, key, locale, value, smart?}] writes \
             entries, creating `Assets/Localization/<table>.stringtable` if the table doesn't exist. Locales \
             are edited with project_settings (section `localization`).",
            schema(
                json!({
                    "set_locale": { "type": "string" },
                    "get": { "type": "object" },
                    "set": { "type": "array" }
                }),
                &[],
            ),
        ),
        |args, e| {
            let w = &mut e.world;
            if !w.resources.contains::<Localization>() {
                oxide_localization::apply(w);
            }
            if let Some(code) = args.get("set_locale").and_then(Json::as_str) {
                let known = w
                    .try_resource::<LocalizationSettings>()
                    .is_some_and(|s| s.locales.iter().any(|l| l.code == code));
                if !known {
                    return ToolResult::error(format!(
                        "`{code}` is not one of the project's locales"
                    ));
                }
                Localization::set_locale(w, code);
                oxide_localization::apply(w);
            }
            if let Some(list) = args.get("set").and_then(Json::as_array) {
                for item in list {
                    let (Some(table), Some(key), Some(locale), Some(value)) = (
                        item.get("table").and_then(Json::as_str),
                        item.get("key").and_then(Json::as_str),
                        item.get("locale").and_then(Json::as_str),
                        item.get("value").and_then(Json::as_str),
                    ) else {
                        return ToolResult::error("set[]: {table, key, locale, value}");
                    };
                    let assets = w.resource_mut::<AssetServer>();
                    let path = assets
                        .paths_of_type("StringTableCollection")
                        .into_iter()
                        .find(|p| {
                            let h = Handle::<StringTableCollection>::from_path(p.clone());
                            assets.data_asset(&h).is_some_and(|t| {
                                t.table_name == table
                                    || (t.table_name.is_empty()
                                        && p.rsplit('/')
                                            .next()
                                            .unwrap_or("")
                                            .starts_with(&format!("{table}.")))
                            })
                        })
                        .unwrap_or_else(|| format!("Assets/Localization/{table}.stringtable"));
                    let h = Handle::<StringTableCollection>::from_path(path.clone());
                    let existing = if assets.full_path(&path).exists() {
                        assets.data_asset(&h).cloned()
                    } else {
                        None
                    };
                    let mut t = existing.unwrap_or_else(|| StringTableCollection {
                        table_name: table.to_owned(),
                        entries: Vec::new(),
                    });
                    let entry = t.entry_mut(key);
                    entry.set(locale, value);
                    if let Some(smart) = item.get("smart").and_then(Json::as_bool)
                        && let Some(v) = entry.values.iter_mut().find(|v| v.locale == locale)
                    {
                        v.smart = smart;
                    }
                    if let Err(err) = assets.save_data(&path, Box::new(t)) {
                        return ToolResult::error(err);
                    }
                }
                oxide_localization::apply(w);
            }
            let mut out = json!({});
            if let Some(g) = args.get("get").and_then(Json::as_object) {
                let table = g.get("table").and_then(Json::as_str).unwrap_or("");
                let key = g.get("key").and_then(Json::as_str).unwrap_or("");
                let a: Vec<Arg> = g
                    .get("args")
                    .and_then(Json::as_array)
                    .map(|v| {
                        v.iter()
                            .map(|x| {
                                Arg::new(x.as_str().map_or_else(|| x.to_string(), str::to_owned))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                out["value"] = json!(Localization::get_string(w, table, key, &a));
            }
            let settings = w
                .try_resource::<LocalizationSettings>()
                .cloned()
                .unwrap_or_default();
            out["locales"] = json!(
                settings
                    .locales
                    .iter()
                    .map(|l| json!({"code": l.code, "name": l.name, "fallback": l.fallback}))
                    .collect::<Vec<_>>()
            );
            out["default_locale"] = json!(settings.default_locale);
            out["selected"] = json!(Localization::selected_locale(w));
            let assets = w.resource_mut::<AssetServer>();
            let tables: Vec<Json> = assets
                .paths_of_type("StringTableCollection")
                .into_iter()
                .filter_map(|p| {
                    let t = assets.data_asset(&Handle::<StringTableCollection>::from_path(p.clone()))?.clone();
                    Some(json!({
                        "name": if t.table_name.is_empty() { p.rsplit('/').next().unwrap_or("").split('.').next().unwrap_or("").to_owned() } else { t.table_name.clone() },
                        "path": p,
                        "keys": t.entries.iter().map(|e| e.key.clone()).collect::<Vec<_>>(),
                    }))
                })
                .collect();
            out["string_tables"] = json!(tables);
            ToolResult::json(out)
        },
    );
}
