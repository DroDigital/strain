//! Minimal hand-written entity-component system: generational entities and one sparse
//! `Store<T>` per component type. Systems are plain functions over `World`.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct Entity {
    pub id: u32,
    pub gen: u32,
}

pub struct Store<T> {
    data: Vec<Option<T>>,
}
impl<T> Default for Store<T> {
    fn default() -> Self {
        Store { data: Vec::new() }
    }
}
impl<T> Store<T> {
    #[inline]
    pub fn get(&self, e: Entity) -> Option<&T> {
        self.data.get(e.id as usize).and_then(|o| o.as_ref())
    }
    #[inline]
    pub fn get_mut(&mut self, e: Entity) -> Option<&mut T> {
        self.data.get_mut(e.id as usize).and_then(|o| o.as_mut())
    }
    pub fn insert(&mut self, e: Entity, v: T) {
        let i = e.id as usize;
        if self.data.len() <= i {
            self.data.resize_with(i + 1, || None);
        }
        self.data[i] = Some(v);
    }
    pub fn remove(&mut self, e: Entity) -> Option<T> {
        self.data.get_mut(e.id as usize).and_then(|o| o.take())
    }
    pub fn has(&self, e: Entity) -> bool {
        self.get(e).is_some()
    }
    pub(crate) fn clear_id(&mut self, id: u32) {
        if let Some(o) = self.data.get_mut(id as usize) {
            *o = None;
        }
    }
    pub(crate) fn clear(&mut self) {
        self.data.clear();
    }
    /// Raw ids that currently hold this component.
    pub fn ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.data.iter().enumerate().filter(|(_, o)| o.is_some()).map(|(i, _)| i as u32)
    }
    pub fn iter(&self) -> impl Iterator<Item = (u32, &T)> {
        self.data.iter().enumerate().filter_map(|(i, o)| o.as_ref().map(|v| (i as u32, v)))
    }
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (u32, &mut T)> {
        self.data.iter_mut().enumerate().filter_map(|(i, o)| o.as_mut().map(|v| (i as u32, v)))
    }
}

macro_rules! define_world {
    ($($name:ident : $ty:ty),* $(,)?) => {
        #[derive(Default)]
        pub struct World {
            gens: Vec<u32>,
            alive: Vec<bool>,
            free: Vec<u32>,
            pub doomed: Vec<Entity>,
            $(pub $name: $crate::ecs::Store<$ty>,)*
        }
        impl World {
            pub fn spawn(&mut self) -> Entity {
                if let Some(id) = self.free.pop() {
                    self.alive[id as usize] = true;
                    Entity { id, gen: self.gens[id as usize] }
                } else {
                    self.gens.push(0);
                    self.alive.push(true);
                    Entity { id: self.gens.len() as u32 - 1, gen: 0 }
                }
            }
            pub fn alive(&self, e: Entity) -> bool {
                (e.id as usize) < self.gens.len() && self.alive[e.id as usize] && self.gens[e.id as usize] == e.gen
            }
            /// Entity handle for a raw id (as yielded by `Store::ids`).
            pub fn ent(&self, id: u32) -> Entity {
                Entity { id, gen: self.gens[id as usize] }
            }
            /// Queue for removal at the end of the tick.
            pub fn kill(&mut self, e: Entity) {
                if self.alive(e) && !self.doomed.contains(&e) {
                    self.doomed.push(e);
                }
            }
            pub fn despawn(&mut self, e: Entity) {
                if !self.alive(e) { return; }
                $(self.$name.clear_id(e.id);)*
                self.alive[e.id as usize] = false;
                self.gens[e.id as usize] += 1;
                self.free.push(e.id);
            }
            pub fn flush(&mut self) {
                let d = std::mem::take(&mut self.doomed);
                for e in d { self.despawn(e); }
            }
            pub fn clear(&mut self) {
                $(self.$name.clear();)*
                for (i, a) in self.alive.iter_mut().enumerate() {
                    if *a { *a = false; self.gens[i] += 1; self.free.push(i as u32); }
                }
                self.doomed.clear();
            }
        }
    };
}
pub(crate) use define_world;
