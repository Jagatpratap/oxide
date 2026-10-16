//! Conversion between reflected [`Value`]s and the JSON used by MCP tools.

use oxide_core::reflect::{FieldInfo, Reflect, Value, ValueKind};
use oxide_scene::{GameObjectId, World};
use serde_json::{Value as Json, json};

pub fn value_to_json(world: Option<&World>, v: &Value) -> Json {
    match v {
        Value::Bool(b) => json!(b),
        Value::Int(i) => json!(i),
        Value::Float(f) => json!((*f * 1e5).round() / 1e5),
        Value::String(s) | Value::Enum(s) | Value::Asset(s) => json!(s),
        Value::Vector2([x, y]) => json!([x, y]),
        Value::Vector3([x, y, z]) => json!([x, y, z]),
        Value::Vector4(a) | Value::Quaternion(a) | Value::Color(a) => json!(a),
        Value::ObjectRef(bits) => {
            let id = GameObjectId::from_bits(*bits);
            match world.and_then(|w| w.get(id)) {
                Some(o) => json!({ "id": bits, "name": o.name }),
                None => json!({ "id": bits, "missing": true }),
            }
        }
        Value::List(items) => Json::Array(items.iter().map(|i| value_to_json(world, i)).collect()),
        Value::Object(map) => Json::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), value_to_json(world, v)))
                .collect(),
        ),
        Value::Null => Json::Null,
    }
}

pub fn reflect_to_json(world: Option<&World>, obj: &dyn Reflect) -> Json {
    let info = obj.type_info();
    let mut fields = serde_json::Map::new();
    for f in &info.fields {
        if let Some(v) = obj.get_field(f.name) {
            fields.insert(f.name.to_string(), value_to_json(world, &v));
        }
    }
    json!(fields)
}

pub fn schema_to_json(info: &oxide_core::reflect::TypeInfo) -> Json {
    json!(
        info.fields
            .iter()
            .map(|f| {
                let mut o = json!({ "name": f.name, "kind": format!("{:?}", f.kind) });
                if !f.options.is_empty() {
                    o["options"] = json!(f.options);
                }
                if let Some(t) = f.type_name {
                    o["type"] = json!(t);
                }
                if let Some(e) = f.element {
                    o["element"] = json!(format!("{e:?}"));
                }
                if let Some(r) = f.range {
                    o["range"] = json!([r.0, r.1]);
                }
                o
            })
            .collect::<Vec<_>>()
    )
}

fn floats(v: &Json, keys: &[&str], required: usize, default: f32) -> Result<Vec<f32>, String> {
    let nums: Vec<Option<f64>> = match v {
        Json::Array(a) => {
            if a.len() < required || a.len() > keys.len() {
                return Err(format!(
                    "expected {required}..={} numbers, got {}",
                    keys.len(),
                    a.len()
                ));
            }
            a.iter()
                .map(|x| x.as_f64().map(Some))
                .collect::<Option<Vec<_>>>()
                .ok_or("array must contain only numbers")?
        }
        Json::Object(o) => keys
            .iter()
            .map(|k| o.get(*k).and_then(Json::as_f64))
            .collect(),
        Json::Number(n) if required <= 1 => vec![n.as_f64()],
        _ => return Err(format!("expected an array or an object with keys {keys:?}")),
    };
    keys.iter()
        .enumerate()
        .map(|(i, k)| match nums.get(i).copied().flatten() {
            Some(n) => Ok(n as f32),
            None if i >= required => Ok(default),
            None => Err(format!("missing `{k}`")),
        })
        .collect()
}

pub fn parse_hex_color(s: &str) -> Result<[f32; 4], String> {
    let hex = s.trim_start_matches('#');
    let byte = |i: usize| {
        u8::from_str_radix(hex.get(i..i + 2).ok_or("bad hex color")?, 16)
            .map(|b| b as f32 / 255.0)
            .map_err(|_| "bad hex color".to_string())
    };
    match hex.len() {
        6 => Ok([byte(0)?, byte(2)?, byte(4)?, 1.0]),
        8 => Ok([byte(0)?, byte(2)?, byte(4)?, byte(6)?]),
        _ => Err("hex colors are #RRGGBB or #RRGGBBAA".into()),
    }
}

/// Resolves an object reference given as an id number, an id string, a `path/to/obj` or a name.
pub fn resolve_object(world: &World, v: &Json) -> Option<GameObjectId> {
    match v {
        Json::Number(n) => n
            .as_u64()
            .map(GameObjectId::from_bits)
            .filter(|&id| world.is_alive(id)),
        Json::String(s) => s
            .parse::<u64>()
            .ok()
            .map(GameObjectId::from_bits)
            .filter(|&id| world.is_alive(id))
            .or_else(|| world.find_path(s))
            .or_else(|| world.find_any(s)),
        Json::Object(o) => o.get("id").and_then(|i| resolve_object(world, i)),
        _ => None,
    }
}

fn scalar(kind: ValueKind, options: &[&str], v: &Json, world: &World) -> Result<Value, String> {
    let arr = |f: Vec<f32>| -> [f32; 4] {
        [
            f[0],
            f[1],
            f.get(2).copied().unwrap_or(0.0),
            f.get(3).copied().unwrap_or(0.0),
        ]
    };
    if v.is_null() {
        return Ok(Value::Null);
    }
    Ok(match kind {
        ValueKind::Bool => Value::Bool(v.as_bool().ok_or("expected true/false")?),
        ValueKind::Int => Value::Int(
            v.as_i64()
                .or_else(|| v.as_f64().map(|f| f as i64))
                .ok_or("expected an integer")?,
        ),
        ValueKind::Float => Value::Float(v.as_f64().ok_or("expected a number")?),
        ValueKind::String => Value::String(v.as_str().ok_or("expected a string")?.to_owned()),
        ValueKind::Vector2 => {
            let f = floats(v, &["x", "y"], 2, 0.0)?;
            Value::Vector2([f[0], f[1]])
        }
        ValueKind::Vector3 => {
            let f = floats(v, &["x", "y", "z"], 3, 0.0)?;
            Value::Vector3([f[0], f[1], f[2]])
        }
        ValueKind::Vector4 => Value::Vector4(arr(floats(v, &["x", "y", "z", "w"], 4, 0.0)?)),
        ValueKind::Quaternion => Value::Quaternion(arr(floats(v, &["x", "y", "z", "w"], 4, 0.0)?)),
        ValueKind::Color => Value::Color(match v {
            Json::String(s) => parse_hex_color(s)?,
            _ => arr(floats(v, &["r", "g", "b", "a"], 3, 1.0)?),
        }),
        ValueKind::Enum => {
            let s = match v {
                Json::String(s) => s.clone(),
                Json::Number(n) => options
                    .get(n.as_u64().unwrap_or(u64::MAX) as usize)
                    .ok_or("enum index out of range")?
                    .to_string(),
                _ => return Err(format!("expected one of {options:?}")),
            };
            let found = options
                .iter()
                .find(|o| o.eq_ignore_ascii_case(&s))
                .ok_or_else(|| format!("expected one of {options:?}"))?;
            Value::Enum(found.to_string())
        }
        ValueKind::Asset => Value::Asset(
            v.as_str()
                .ok_or("expected an asset path like \"builtin:Cube\"")?
                .to_owned(),
        ),
        ValueKind::ObjectRef => Value::ObjectRef(
            resolve_object(world, v)
                .ok_or("no such GameObject")?
                .to_bits(),
        ),
        ValueKind::List | ValueKind::Object | ValueKind::Null => {
            return Err("unsupported nested value".into());
        }
    })
}

pub fn json_to_value(f: &FieldInfo, v: &Json, world: &World) -> Result<Value, String> {
    if f.kind == ValueKind::List {
        let elem = f.element.unwrap_or(ValueKind::Float);
        let items = v.as_array().ok_or("expected an array")?;
        return items
            .iter()
            .map(|i| scalar(elem, f.options, i, world))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::List);
    }
    scalar(f.kind, f.options, v, world)
}

/// Converts `{field: value, ...}` for a type into reflected values (resolving object
/// references against `world`). Returns the values and per-field errors.
/// Parses JSON shaped like `template` (the field's current value), for nested structs whose
/// inner field kinds are only known from an existing value.
pub fn json_like(template: &Value, v: &Json, world: &World) -> Result<Value, String> {
    let kind = template.kind();
    match template {
        // A Volume parameter given as a plain value: override it with that value.
        Value::Object(map) if !v.is_object() && map.contains_key("override") && map.len() == 2 => {
            let inner = map.get("value").ok_or("expected an object")?;
            Ok(Value::Object(
                [
                    ("override".to_owned(), Value::Bool(true)),
                    ("value".to_owned(), json_like(inner, v, world)?),
                ]
                .into_iter()
                .collect(),
            ))
        }
        Value::Object(map) => {
            let obj = v.as_object().ok_or("expected an object")?;
            let mut out = std::collections::BTreeMap::new();
            for (k, jv) in obj {
                let t = map.get(k).ok_or_else(|| {
                    format!(
                        "no field `{k}` (fields: {:?})",
                        map.keys().collect::<Vec<_>>()
                    )
                })?;
                out.insert(k.clone(), json_like(t, jv, world)?);
            }
            Ok(Value::Object(out))
        }
        Value::List(items) => {
            let arr = v.as_array().ok_or("expected an array")?;
            match items.first() {
                Some(t) => arr
                    .iter()
                    .map(|i| json_like(t, i, world))
                    .collect::<Result<Vec<_>, _>>()
                    .map(Value::List),
                // Empty list: infer each element from the JSON itself.
                None => arr
                    .iter()
                    .map(|i| infer(i, world))
                    .collect::<Result<Vec<_>, _>>()
                    .map(Value::List),
            }
        }
        Value::Enum(_) => Ok(Value::Enum(
            v.as_str().ok_or("expected a variant name")?.to_owned(),
        )),
        Value::Null => Err(
            "cannot infer the type of a null field; set it with set_component's typed path".into(),
        ),
        _ => scalar(kind, &[], v, world),
    }
}

/// A value from JSON alone: numbers, booleans, `[x, y, z]` vectors, and strings that name
/// a GameObject (references) or anything else (text).
fn infer(v: &Json, world: &World) -> Result<Value, String> {
    Ok(match v {
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) => Value::Float(n.as_f64().unwrap_or(0.0)),
        Json::Array(a) if a.len() == 3 && a.iter().all(Json::is_number) => {
            Value::Vector3([0, 1, 2].map(|i| a[i].as_f64().unwrap_or(0.0) as f32))
        }
        Json::String(_) | Json::Object(_) => match resolve_object(world, v) {
            Some(id) => Value::ObjectRef(id.to_bits()),
            None => match v.as_str() {
                Some(s) => Value::String(s.to_owned()),
                None => return Err(format!("cannot infer a value from {v}")),
            },
        },
        Json::Null => Value::Null,
        other => return Err(format!("cannot infer a value from {other}")),
    })
}

pub fn parse_fields(
    info: &oxide_core::reflect::TypeInfo,
    fields: &serde_json::Map<String, Json>,
    world: &World,
    current: Option<&dyn Reflect>,
) -> (Vec<(&'static str, Value)>, Vec<String>) {
    let mut out = Vec::new();
    let mut errors = Vec::new();
    for (k, v) in fields {
        let Some(f) = info
            .fields
            .iter()
            .find(|f| f.name == k || f.name.eq_ignore_ascii_case(k))
        else {
            let names: Vec<_> = info.fields.iter().map(|f| f.name).collect();
            errors.push(format!("{}: no field `{k}` (fields: {names:?})", info.name));
            continue;
        };
        // Structs (and lists of structs) are parsed like their current value.
        let nested = f.kind == ValueKind::Object
            || (f.kind == ValueKind::List && f.element == Some(ValueKind::Object));
        let parsed = if nested {
            match current.and_then(|c| c.get_field(f.name)) {
                Some(t) => json_like(&t, v, world),
                None => Err("nested value without a template".into()),
            }
        } else {
            json_to_value(f, v, world)
        };
        match parsed {
            Ok(val) => out.push((f.name, val)),
            Err(e) => errors.push(format!("{}.{}: {e}", info.name, f.name)),
        }
    }
    (out, errors)
}

/// Writes parsed values into an object, collecting errors.
pub fn set_fields(
    obj: &mut dyn Reflect,
    values: Vec<(&'static str, Value)>,
    errors: &mut Vec<String>,
) {
    let name = obj.type_info().name;
    for (f, v) in values {
        if let Err(e) = obj.set_field(f, v) {
            errors.push(format!("{name}.{f}: {e}"));
        }
    }
}
