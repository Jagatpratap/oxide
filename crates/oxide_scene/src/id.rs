use oxide_core::reflect::{FieldValue, Value, ValueKind};

/// A GameObject handle. Stale handles (destroyed objects) never resolve, like Unity's
/// "fake null" for destroyed objects.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct GameObjectId {
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

impl GameObjectId {
    pub fn to_bits(self) -> u64 {
        ((self.generation as u64) << 32) | self.index as u64
    }

    pub fn from_bits(bits: u64) -> Self {
        Self {
            index: bits as u32,
            generation: (bits >> 32) as u32,
        }
    }

    pub fn index(self) -> u32 {
        self.index
    }
}

impl std::fmt::Display for GameObjectId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_bits())
    }
}

impl FieldValue for GameObjectId {
    const KIND: ValueKind = ValueKind::ObjectRef;
    const TYPE_NAME: Option<&'static str> = Some("GameObject");
    fn to_value(&self) -> Value {
        Value::ObjectRef(self.to_bits())
    }
    fn from_value(v: Value) -> Option<Self> {
        match v {
            Value::ObjectRef(b) => Some(Self::from_bits(b)),
            Value::Int(b) => Some(Self::from_bits(b as u64)),
            _ => None,
        }
    }
}

/// Unique id of one component instance (stable for its lifetime).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct ComponentId(pub u64);
