//! Engine-wide singletons stored in the [`World`](crate::World): `Time`, `Input`, the asset
//! server, physics, audio, ... Scripts reach them through [`Ctx`](crate::Ctx).

use std::any::{Any, TypeId};
use std::collections::HashMap;

#[derive(Default)]
pub struct Resources {
    map: HashMap<TypeId, Box<dyn Any>>,
    /// Rust type name of each resource (to tell which library defined it).
    names: HashMap<TypeId, &'static str>,
}

impl Resources {
    pub fn insert<T: 'static>(&mut self, value: T) {
        self.map.insert(TypeId::of::<T>(), Box::new(value));
        self.names
            .insert(TypeId::of::<T>(), std::any::type_name::<T>());
    }

    /// Drops every resource whose Rust type is defined in crate `krate` (used before a
    /// script library is reloaded: its old types must not outlive it).
    pub fn remove_crate(&mut self, krate: &str) -> usize {
        let prefix = format!("{krate}::");
        let doomed: Vec<TypeId> = self
            .names
            .iter()
            .filter(|(_, n)| n.starts_with(&prefix))
            .map(|(t, _)| *t)
            .collect();
        for t in &doomed {
            self.map.remove(t);
            self.names.remove(t);
        }
        doomed.len()
    }

    pub fn get<T: 'static>(&self) -> Option<&T> {
        self.map.get(&TypeId::of::<T>())?.downcast_ref()
    }

    pub fn get_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.map.get_mut(&TypeId::of::<T>())?.downcast_mut()
    }

    /// Takes a resource out (so a system can use it alongside `&mut World`); put it back
    /// with [`Resources::insert`].
    pub fn remove<T: 'static>(&mut self) -> Option<T> {
        self.map
            .remove(&TypeId::of::<T>())
            .and_then(|b| b.downcast().ok())
            .map(|b| *b)
    }

    pub fn contains<T: 'static>(&self) -> bool {
        self.map.contains_key(&TypeId::of::<T>())
    }
}
