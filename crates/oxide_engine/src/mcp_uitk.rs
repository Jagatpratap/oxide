//! MCP tool for UI Toolkit documents at run time.

use crate::bus::Bus;
use crate::json_values::resolve_object;
use crate::mcp_tools::{schema, tool};
use oxide_mcp::{McpServer, ToolDef, ToolResult, json};
use oxide_uitk::{UIDocument, UiEvent, UiEventKind, Value};
use serde_json::Value as Json;
use std::sync::Arc;

fn value_json(v: &Value) -> Json {
    match v {
        Value::None => Json::Null,
        Value::Text(t) => json!(t),
        Value::Float(f) => json!(f),
        Value::Int(i) => json!(i),
        Value::Bool(b) => json!(b),
    }
}

fn json_value(v: &Json) -> Value {
    match v {
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) if n.is_i64() => Value::Int(n.as_i64().unwrap_or(0)),
        Json::Number(n) => Value::Float(n.as_f64().unwrap_or(0.0) as f32),
        Json::String(s) => Value::Text(s.clone()),
        _ => Value::None,
    }
}

fn doc_json(d: &UIDocument) -> Json {
    let elements: Vec<Json> = d
        .state
        .panel
        .as_ref()
        .map(|p| {
            p.elements
                .iter()
                .skip(1)
                .map(|e| {
                    json!({
                        "type": e.kind, "name": e.name, "classes": e.classes,
                        "text": e.text(), "value": value_json(&e.value),
                        "rect": [e.rect.min.x, e.rect.min.y, e.rect.width(), e.rect.height()],
                        "displayed": e.displayed() && e.rect.is_finite(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    json!({
        "source": d.source_asset.as_ref().map(|h| h.path().to_owned()),
        "panel_settings": d.panel_settings.as_ref().map(|h| h.path().to_owned()),
        "error": d.state.error,
        "scale": d.state.scale,
        "elements": elements,
        "last_events": d.state.events.iter().map(|e| json!({
            "kind": format!("{:?}", e.kind), "element": e.element, "value": value_json(&e.value),
        })).collect::<Vec<_>>(),
    })
}

pub fn register(s: &mut McpServer, bus: &Arc<Bus>) {
    tool(
        s,
        bus,
        ToolDef::new(
            "ui_toolkit",
            "UI Toolkit documents",
            "Lists every UI Document with its elements (type, name, classes, text, value, rect in panel pixels) \
             and last events. With `object` (the UIDocument): `set_text` {name: text}, `set_value` {name: \
             value}, `add_class` / `remove_class` {name: class}, `display` {name: bool}, `style` {name: \
             {property: value}}, `click` (element name: queues a Click event scripts see as \
             `doc.clicked(name)` next frame).",
            schema(
                json!({
                    "object": {}, "set_text": { "type": "object" }, "set_value": { "type": "object" },
                    "add_class": { "type": "object" }, "remove_class": { "type": "object" },
                    "display": { "type": "object" }, "style": { "type": "object" }, "click": { "type": "string" }
                }),
                &[],
            ),
        ),
        |args, e| {
            let w = &mut e.world;
            if let Some(o) = args.get("object") {
                let Some(id) = resolve_object(w, o) else {
                    return ToolResult::error(format!("no GameObject matches {o}"));
                };
                let Some(d) = w.get_component_mut::<UIDocument>(id) else {
                    return ToolResult::error("that object has no UI Document");
                };
                for (k, v) in args
                    .get("set_text")
                    .and_then(Json::as_object)
                    .into_iter()
                    .flatten()
                {
                    d.set_text(k, v.as_str().unwrap_or(&v.to_string()));
                }
                for (k, v) in args
                    .get("set_value")
                    .and_then(Json::as_object)
                    .into_iter()
                    .flatten()
                {
                    d.set_value(k, json_value(v));
                }
                for (k, v) in args
                    .get("add_class")
                    .and_then(Json::as_object)
                    .into_iter()
                    .flatten()
                {
                    d.add_class(k, v.as_str().unwrap_or(""));
                }
                for (k, v) in args
                    .get("remove_class")
                    .and_then(Json::as_object)
                    .into_iter()
                    .flatten()
                {
                    d.remove_class(k, v.as_str().unwrap_or(""));
                }
                for (k, v) in args
                    .get("display")
                    .and_then(Json::as_object)
                    .into_iter()
                    .flatten()
                {
                    d.set_display(k, v.as_bool().unwrap_or(true));
                }
                for (k, v) in args
                    .get("style")
                    .and_then(Json::as_object)
                    .into_iter()
                    .flatten()
                {
                    for (p, x) in v.as_object().into_iter().flatten() {
                        d.set_style(k, p, x.as_str().unwrap_or(&x.to_string()));
                    }
                }
                if let Some(n) = args.get("click").and_then(Json::as_str) {
                    let value = d.value(n).unwrap_or_default();
                    d.state.events.push(UiEvent {
                        kind: UiEventKind::Click,
                        element: n.to_owned(),
                        index: 0,
                        value,
                    });
                }
                let d = w.get_component::<UIDocument>(id).expect("still there");
                return ToolResult::json(doc_json(d));
            }
            let list: Vec<Json> = w
                .query::<UIDocument>()
                .map(|(id, d)| {
                    let mut j = doc_json(d);
                    j["object"] = json!(id.to_bits());
                    j["name"] = json!(w.get(id).map(|o| o.name.clone()));
                    j
                })
                .collect();
            ToolResult::json(json!(list))
        },
    );
}
