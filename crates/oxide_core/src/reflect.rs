//! Runtime reflection: lets the Inspector, scene serializer and MCP tools read and write
//! fields of engine types by name without knowing the concrete Rust type.
//!
//! Reference: <https://docs.unity3d.com/Manual/script-serialization.html>
//!
//! Types opt in with `#[derive(Reflect)]` (crate `oxide_derive`), or the older
//! [`reflect_struct!`](crate::reflect_struct) macro.

use oxide_math::{Color, Quaternion, Vector2, Vector3, Vector4};
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::collections::BTreeMap;

/// A field value of one of the types the Inspector knows how to draw.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Vector2([f32; 2]),
    Vector3([f32; 3]),
    Vector4([f32; 4]),
    /// `[x, y, z, w]`.
    Quaternion([f32; 4]),
    /// `[r, g, b, a]`.
    Color([f32; 4]),
    /// A unit-variant enum, by variant name.
    Enum(String),
    /// An asset reference, by asset path (`builtin:Cube`, `Assets/car.glb#mesh0`, ...).
    Asset(String),
    /// A GameObject reference, by raw id.
    ObjectRef(u64),
    List(Vec<Value>),
    /// A nested serializable struct (Unity `[Serializable]` class field), by field name.
    Object(BTreeMap<String, Value>),
    /// Unity's `null` (an empty `Option`).
    #[default]
    Null,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum ValueKind {
    Bool,
    Int,
    Float,
    String,
    Vector2,
    Vector3,
    Vector4,
    Quaternion,
    Color,
    Enum,
    Asset,
    ObjectRef,
    List,
    Object,
    Null,
}

impl Value {
    pub fn kind(&self) -> ValueKind {
        match self {
            Value::Bool(_) => ValueKind::Bool,
            Value::Int(_) => ValueKind::Int,
            Value::Float(_) => ValueKind::Float,
            Value::String(_) => ValueKind::String,
            Value::Vector2(_) => ValueKind::Vector2,
            Value::Vector3(_) => ValueKind::Vector3,
            Value::Vector4(_) => ValueKind::Vector4,
            Value::Quaternion(_) => ValueKind::Quaternion,
            Value::Color(_) => ValueKind::Color,
            Value::Enum(_) => ValueKind::Enum,
            Value::Asset(_) => ValueKind::Asset,
            Value::ObjectRef(_) => ValueKind::ObjectRef,
            Value::List(_) => ValueKind::List,
            Value::Object(_) => ValueKind::Object,
            Value::Null => ValueKind::Null,
        }
    }
}

/// Conversion between a Rust field type and [`Value`].
pub trait FieldValue: Sized {
    const KIND: ValueKind;
    /// Variant names, for enums.
    const OPTIONS: &'static [&'static str] = &[];
    /// Concrete type behind an `Enum`, `Asset`, `ObjectRef` or `List` field.
    const TYPE_NAME: Option<&'static str> = None;
    /// Kind of each element, for `List` fields.
    const ELEMENT: Option<ValueKind> = None;
    fn to_value(&self) -> Value;
    fn from_value(v: Value) -> Option<Self>;
    /// Updates `self` in place (nested structs merge field by field).
    fn set_from(&mut self, v: Value) -> bool {
        match Self::from_value(v) {
            Some(n) => {
                *self = n;
                true
            }
            None => false,
        }
    }
}

macro_rules! field_value {
    ($t:ty, $kind:ident, |$s:ident| $to:expr, |$v:ident| $from:expr) => {
        impl FieldValue for $t {
            const KIND: ValueKind = ValueKind::$kind;
            fn to_value(&self) -> Value {
                let $s = self;
                Value::$kind($to)
            }
            fn from_value(v: Value) -> Option<Self> {
                match v {
                    Value::$kind($v) => Some($from),
                    _ => None,
                }
            }
        }
    };
}

field_value!(bool, Bool, |s| *s, |v| v);
field_value!(i32, Int, |s| *s as i64, |v| i32::try_from(v).ok()?);
field_value!(i64, Int, |s| *s, |v| v);
field_value!(u32, Int, |s| *s as i64, |v| u32::try_from(v).ok()?);
impl FieldValue for f32 {
    const KIND: ValueKind = ValueKind::Float;
    fn to_value(&self) -> Value {
        Value::Float(*self as f64)
    }
    fn from_value(v: Value) -> Option<Self> {
        match v {
            Value::Float(f) => Some(f as f32),
            Value::Int(i) => Some(i as f32),
            _ => None,
        }
    }
}

impl FieldValue for f64 {
    const KIND: ValueKind = ValueKind::Float;
    fn to_value(&self) -> Value {
        Value::Float(*self)
    }
    fn from_value(v: Value) -> Option<Self> {
        match v {
            Value::Float(f) => Some(f),
            Value::Int(i) => Some(i as f64),
            _ => None,
        }
    }
}
field_value!(String, String, |s| s.clone(), |v| v);
field_value!(Vector2, Vector2, |s| s.to_array(), |v| Vector2::from_array(
    v
));
field_value!(Vector3, Vector3, |s| s.to_array(), |v| Vector3::from_array(
    v
));
field_value!(Vector4, Vector4, |s| s.to_array(), |v| Vector4::from_array(
    v
));
field_value!(Quaternion, Quaternion, |s| s.to_array(), |v| {
    Quaternion::from_array(v).normalize()
});
field_value!(Color, Color, |s| (*s).into(), |v| Color::new(
    v[0], v[1], v[2], v[3]
));

/// Unity `Option<T>` fields hold `null` as [`Value::Null`].
impl<T: FieldValue> FieldValue for Option<T> {
    const KIND: ValueKind = T::KIND;
    const OPTIONS: &'static [&'static str] = T::OPTIONS;
    const TYPE_NAME: Option<&'static str> = T::TYPE_NAME;
    const ELEMENT: Option<ValueKind> = T::ELEMENT;
    fn to_value(&self) -> Value {
        self.as_ref().map_or(Value::Null, T::to_value)
    }
    fn from_value(v: Value) -> Option<Self> {
        match v {
            Value::Null => Some(None),
            v => T::from_value(v).map(Some),
        }
    }
}

impl<T: FieldValue> FieldValue for Vec<T> {
    const KIND: ValueKind = ValueKind::List;
    const TYPE_NAME: Option<&'static str> = T::TYPE_NAME;
    const ELEMENT: Option<ValueKind> = Some(T::KIND);
    const OPTIONS: &'static [&'static str] = T::OPTIONS;
    fn to_value(&self) -> Value {
        Value::List(self.iter().map(T::to_value).collect())
    }
    fn from_value(v: Value) -> Option<Self> {
        match v {
            Value::List(items) => items.into_iter().map(T::from_value).collect(),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct FieldInfo {
    pub name: &'static str,
    pub kind: ValueKind,
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    pub options: &'static [&'static str],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_name: Option<&'static str>,
    /// Element kind of a `List` field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element: Option<ValueKind>,
    /// Inspector slider bounds (Unity `[Range]`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<(f64, f64)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tooltip: Option<&'static str>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TypeInfo {
    pub name: &'static str,
    pub fields: Vec<FieldInfo>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ReflectError {
    NoSuchField(String),
    WrongType { field: String, expected: ValueKind },
}

impl std::fmt::Display for ReflectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReflectError::NoSuchField(n) => write!(f, "no field named `{n}`"),
            ReflectError::WrongType { field, expected } => {
                write!(f, "field `{field}` expects a {expected:?}")
            }
        }
    }
}

impl std::error::Error for ReflectError {}

/// A type whose fields can be enumerated, read and written by name.
pub trait Reflect: Any + Send + Sync {
    fn type_info(&self) -> TypeInfo;
    fn get_field(&self, name: &str) -> Option<Value>;
    fn set_field(&mut self, name: &str, value: Value) -> Result<(), ReflectError>;

    /// All fields as a name → value map (used for serialization and tool output).
    fn fields(&self) -> BTreeMap<&'static str, Value> {
        self.type_info()
            .fields
            .iter()
            .filter_map(|f| Some((f.name, self.get_field(f.name)?)))
            .collect()
    }
}

/// Implements [`Reflect`] for a struct whose listed fields implement [`FieldValue`].
///
/// ```ignore
/// reflect_struct!(Light { intensity, color, enabled });
/// ```
#[macro_export]
macro_rules! reflect_struct {
    ($ty:ident { $($field:ident),* $(,)? }) => {
        impl $crate::reflect::Reflect for $ty {
            fn type_info(&self) -> $crate::reflect::TypeInfo {
                $crate::reflect::TypeInfo {
                    name: stringify!($ty),
                    fields: vec![$($crate::reflect::FieldInfo {
                        name: stringify!($field),
                        kind: $crate::reflect::field_kind(&self.$field),
                        options: $crate::reflect::field_options(&self.$field),
                        type_name: $crate::reflect::field_type_name(&self.$field),
                        element: $crate::reflect::field_element(&self.$field),
                        range: None,
                        tooltip: None,
                    }),*],
                }
            }

            fn get_field(&self, name: &str) -> Option<$crate::reflect::Value> {
                match name {
                    $(stringify!($field) => Some($crate::reflect::FieldValue::to_value(&self.$field)),)*
                    _ => None,
                }
            }

            fn set_field(
                &mut self,
                name: &str,
                value: $crate::reflect::Value,
            ) -> Result<(), $crate::reflect::ReflectError> {
                match name {
                    $(stringify!($field) => {
                        let expected = $crate::reflect::field_kind(&self.$field);
                        if $crate::reflect::FieldValue::set_from(&mut self.$field, value) {
                            Ok(())
                        } else {
                            Err($crate::reflect::ReflectError::WrongType {
                                field: name.to_owned(),
                                expected,
                            })
                        }
                    })*
                    _ => Err($crate::reflect::ReflectError::NoSuchField(name.to_owned())),
                }
            }
        }
    };
}

#[doc(hidden)]
pub fn field_kind<T: FieldValue>(_: &T) -> ValueKind {
    T::KIND
}

#[doc(hidden)]
pub fn field_options<T: FieldValue>(_: &T) -> &'static [&'static str] {
    T::OPTIONS
}

#[doc(hidden)]
pub fn field_element<T: FieldValue>(_: &T) -> Option<ValueKind> {
    T::ELEMENT
}

#[doc(hidden)]
pub fn field_type_name<T: FieldValue>(_: &T) -> Option<&'static str> {
    T::TYPE_NAME
}

struct Registration {
    info: TypeInfo,
    create: fn() -> Box<dyn Reflect>,
}

/// Registry of reflectable types, by name (the Inspector's "Add Component" list in later phases).
#[derive(Default)]
pub struct TypeRegistry {
    types: BTreeMap<&'static str, Registration>,
}

impl TypeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<T: Reflect + Default>(&mut self) {
        let info = T::default().type_info();
        let create: fn() -> Box<dyn Reflect> = || Box::new(T::default());
        self.types.insert(info.name, Registration { info, create });
    }

    pub fn get(&self, name: &str) -> Option<&TypeInfo> {
        self.types.get(name).map(|r| &r.info)
    }

    /// Creates a default instance of a registered type.
    pub fn create(&self, name: &str) -> Option<Box<dyn Reflect>> {
        self.types.get(name).map(|r| (r.create)())
    }

    pub fn iter(&self) -> impl Iterator<Item = &TypeInfo> {
        self.types.values().map(|r| &r.info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Spinner {
        speed: f32,
        axis: Vector3,
        tint: Color,
        label: String,
    }
    crate::reflect_struct!(Spinner {
        speed,
        axis,
        tint,
        label
    });

    #[test]
    fn get_set_by_name() {
        let mut s = Spinner {
            speed: 90.0,
            ..Default::default()
        };
        assert_eq!(s.get_field("speed"), Some(Value::Float(90.0)));
        s.set_field("axis", Value::Vector3([0.0, 1.0, 0.0]))
            .unwrap();
        assert_eq!(s.axis, Vector3::Y);
        assert_eq!(
            s.set_field("speed", Value::Bool(true)),
            Err(ReflectError::WrongType {
                field: "speed".into(),
                expected: ValueKind::Float
            })
        );
        assert!(matches!(
            s.set_field("nope", Value::Int(1)),
            Err(ReflectError::NoSuchField(_))
        ));
        assert_eq!(s.fields().len(), 4);
    }

    #[test]
    fn registry_creates_instances() {
        let mut reg = TypeRegistry::new();
        reg.register::<Spinner>();
        let info = reg.get("Spinner").unwrap();
        assert_eq!(info.fields[1].kind, ValueKind::Vector3);
        let mut obj = reg.create("Spinner").unwrap();
        obj.set_field("label", Value::String("hi".into())).unwrap();
        assert_eq!(obj.get_field("label"), Some(Value::String("hi".into())));
    }

    #[test]
    fn value_json_shape() {
        let json = serde_json::to_string(&Value::Vector3([1.0, 2.0, 3.0])).unwrap();
        assert_eq!(json, r#"{"Vector3":[1.0,2.0,3.0]}"#);
    }
}
