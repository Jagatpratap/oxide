//! Runtime reflection: lets the Inspector, scene serializer and MCP tools read and write
//! fields of engine types by name without knowing the concrete Rust type.
//!
//! Reference: <https://docs.unity3d.com/Manual/script-serialization.html>
//!
//! Types opt in with the [`reflect_struct!`](crate::reflect_struct) macro for now; a
//! `#[derive(Reflect)]` proc-macro replaces it in P1 when GameObject components arrive.

use oxide_math::{Color, Quaternion, Vector2, Vector3, Vector4};
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::collections::BTreeMap;

/// A field value of one of the types the Inspector knows how to draw.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
        }
    }
}

/// Conversion between a Rust field type and [`Value`].
pub trait FieldValue: Sized {
    const KIND: ValueKind;
    fn to_value(&self) -> Value;
    fn from_value(v: Value) -> Option<Self>;
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
field_value!(f32, Float, |s| *s as f64, |v| v as f32);
field_value!(f64, Float, |s| *s, |v| v);
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

#[derive(Clone, Copy, Debug, Serialize)]
pub struct FieldInfo {
    pub name: &'static str,
    pub kind: ValueKind,
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
                        self.$field = $crate::reflect::FieldValue::from_value(value).ok_or_else(|| {
                            $crate::reflect::ReflectError::WrongType {
                                field: name.to_owned(),
                                expected,
                            }
                        })?;
                        Ok(())
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
