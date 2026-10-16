//! Asset handles and typed asset stores.
//!
//! Every asset has a path: a file under the project (`Assets/Models/car.glb#mesh0`), a
//! built-in (`builtin:Cube`), or a runtime-created one (`runtime:Mesh/7`). A [`Handle`] is
//! that path plus a stable id (hash of the path), so it can be stored in components,
//! serialized into scenes by path, and resolved without locking.

use oxide_core::reflect::{FieldValue, Value, ValueKind};
use std::collections::HashMap;
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::marker::PhantomData;
use std::sync::Arc;

/// Types stored in [`Assets`].
pub trait Asset: Send + Sync + 'static {
    /// Unity-facing type name (`Mesh`, `Material`, `Texture2D`, ...).
    const TYPE_NAME: &'static str;
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct AssetId(pub u64);

impl AssetId {
    pub fn from_path(path: &str) -> Self {
        let mut h = DefaultHasher::new();
        path.hash(&mut h);
        AssetId(h.finish())
    }
}

pub struct Handle<T> {
    id: AssetId,
    path: Arc<str>,
    _t: PhantomData<fn() -> T>,
}

impl<T> Handle<T> {
    pub fn from_path(path: impl Into<Arc<str>>) -> Self {
        let path = path.into();
        Self {
            id: AssetId::from_path(&path),
            path,
            _t: PhantomData,
        }
    }

    pub fn id(&self) -> AssetId {
        self.id
    }

    pub fn path(&self) -> &str {
        &self.path
    }
}

impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            path: self.path.clone(),
            _t: PhantomData,
        }
    }
}

impl<T> PartialEq for Handle<T> {
    fn eq(&self, o: &Self) -> bool {
        self.id == o.id
    }
}
impl<T> Eq for Handle<T> {}

impl<T> Hash for Handle<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state)
    }
}

impl<T> fmt::Debug for Handle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Handle({})", self.path)
    }
}

impl<T: Asset> FieldValue for Handle<T> {
    const KIND: ValueKind = ValueKind::Asset;
    const TYPE_NAME: Option<&'static str> = Some(T::TYPE_NAME);
    fn to_value(&self) -> Value {
        Value::Asset(self.path.to_string())
    }
    fn from_value(v: Value) -> Option<Self> {
        match v {
            Value::Asset(p) | Value::String(p) => Some(Handle::from_path(p)),
            _ => None,
        }
    }
}

/// Serialized as its asset path.
impl<T> serde::Serialize for Handle<T> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.path)
    }
}

impl<'de, T> serde::Deserialize<'de> for Handle<T> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d).map(Handle::from_path)
    }
}

struct Entry<T> {
    path: Arc<str>,
    value: T,
    version: u64,
}

/// All loaded assets of one type.
pub struct Assets<T> {
    entries: HashMap<AssetId, Entry<T>>,
    next_runtime: u64,
}

impl<T> Default for Assets<T> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            next_runtime: 1,
        }
    }
}

impl<T: Asset> Assets<T> {
    /// Stores `value` under `path`, replacing (and bumping the version of) any existing one.
    pub fn insert(&mut self, path: impl Into<Arc<str>>, value: T) -> Handle<T> {
        let handle = Handle::from_path(path);
        let version = self.entries.get(&handle.id).map_or(1, |e| e.version + 1);
        self.entries.insert(
            handle.id,
            Entry {
                path: handle.path.clone(),
                value,
                version,
            },
        );
        handle
    }

    /// Stores a runtime-created asset (Unity `new Mesh()`, `new Material(...)`).
    pub fn add(&mut self, value: T) -> Handle<T> {
        let path = format!("runtime:{}/{}", T::TYPE_NAME, self.next_runtime);
        self.next_runtime += 1;
        self.insert(path, value)
    }

    pub fn get(&self, h: &Handle<T>) -> Option<&T> {
        self.entries.get(&h.id).map(|e| &e.value)
    }

    pub fn get_by_id(&self, id: AssetId) -> Option<&T> {
        self.entries.get(&id).map(|e| &e.value)
    }

    /// Mutable access; bumps the version so GPU copies are refreshed.
    pub fn get_mut(&mut self, h: &Handle<T>) -> Option<&mut T> {
        self.entries.get_mut(&h.id).map(|e| {
            e.version += 1;
            &mut e.value
        })
    }

    pub fn version(&self, id: AssetId) -> Option<u64> {
        self.entries.get(&id).map(|e| e.version)
    }

    pub fn contains(&self, h: &Handle<T>) -> bool {
        self.entries.contains_key(&h.id)
    }

    pub fn remove(&mut self, h: &Handle<T>) -> Option<T> {
        self.entries.remove(&h.id).map(|e| e.value)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// `(path, value)` pairs in no particular order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &T)> {
        self.entries.values().map(|e| (&*e.path, &e.value))
    }

    pub fn handle_of(&self, path: &str) -> Option<Handle<T>> {
        let h = Handle::from_path(path);
        self.entries.contains_key(&h.id).then_some(h)
    }
}
