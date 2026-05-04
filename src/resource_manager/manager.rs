use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::num::NonZeroU32;
use std::ops::{Index, IndexMut};
use std::sync::Arc;
use glam::Affine3A;
use thin_vec::ThinVec;
use super::component::{Component, ComponentType};

// ── Generational handle ─────────────────────────────────────────────────────
//
// Option<Handle<Tag>> is the same size as Handle<Tag> because generation is NonZeroU32.
// Distinct tag types produce incompatible handle types at compile time.

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Handle<Tag> {
    pub idx: u32,
    pub generation: NonZeroU32,
    _tag: PhantomData<fn() -> Tag>,
}

// Arena tag types — plain ZSTs, no data, no circular imports.
#[derive(Copy, Clone, PartialEq, Eq)] pub struct LevelTag;
#[derive(Copy, Clone, PartialEq, Eq)] pub struct StageTag;
#[derive(Copy, Clone, PartialEq, Eq)] pub struct ActorTag;

pub type LevelHandle = Handle<LevelTag>;
pub type StageHandle = Handle<StageTag>;
pub type ActorHandle = Handle<ActorTag>;

// ── Flat-Vec arena ──────────────────────────────────────────────────────────

struct Slot<T> {
    generation: NonZeroU32,
    val: Option<T>,
}

pub struct Arena<Tag, T> {
    slots: Vec<Slot<T>>,
    free:  ThinVec<u32>,
    _tag:  PhantomData<fn() -> Tag>,
}

impl<Tag, T> Arena<Tag, T> {
    pub fn new() -> Self {
        Self { slots: Vec::new(), free: ThinVec::new(), _tag: PhantomData }
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self { slots: Vec::with_capacity(cap), free: ThinVec::new(), _tag: PhantomData }
    }

    pub fn insert(&mut self, val: T) -> Handle<Tag> {
        if let Some(idx) = self.free.pop() {
            // Reuse a freed slot; generation was already incremented on removal.
            self.slots[idx as usize].val = Some(val);
            Handle { idx, generation: self.slots[idx as usize].generation, _tag: PhantomData }
        } else {
            let idx = self.slots.len() as u32;
            let generation = NonZeroU32::new(1).unwrap();
            self.slots.push(Slot { generation, val: Some(val) });
            Handle { idx, generation, _tag: PhantomData }
        }
    }

    pub fn remove(&mut self, h: Handle<Tag>) -> Option<T> {
        let slot = self.slots.get_mut(h.idx as usize)?;
        if slot.generation != h.generation { return None; }
        let val = slot.val.take()?;
        // Increment generation so any surviving copies of h are now stale.
        slot.generation = NonZeroU32::new(slot.generation.get().wrapping_add(1))
            .unwrap_or(NonZeroU32::new(1).unwrap());
        self.free.push(h.idx);
        Some(val)
    }

    pub fn get(&self, h: Handle<Tag>) -> Option<&T> {
        let slot = self.slots.get(h.idx as usize)?;
        if slot.generation == h.generation { slot.val.as_ref() } else { None }
    }

    pub fn get_mut(&mut self, h: Handle<Tag>) -> Option<&mut T> {
        let slot = self.slots.get_mut(h.idx as usize)?;
        if slot.generation == h.generation { slot.val.as_mut() } else { None }
    }

    pub fn contains(&self, h: Handle<Tag>) -> bool { self.get(h).is_some() }

    pub fn values(&self) -> impl Iterator<Item = &T> {
        self.slots.iter().filter_map(|s| s.val.as_ref())
    }

    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.slots.iter_mut().filter_map(|s| s.val.as_mut())
    }
}

impl<Tag, T> Index<Handle<Tag>> for Arena<Tag, T> {
    type Output = T;
    fn index(&self, h: Handle<Tag>) -> &T {
        self.get(h).expect("handle is stale or was never valid")
    }
}

impl<Tag, T> IndexMut<Handle<Tag>> for Arena<Tag, T> {
    fn index_mut(&mut self, h: Handle<Tag>) -> &mut T {
        self.get_mut(h).expect("handle is stale or was never valid")
    }
}

impl<Tag, T> Default for Arena<Tag, T> {
    fn default() -> Self { Self::new() }
}

// ── Typed database IDs ──────────────────────────────────────────────────────

pub struct Id<T>(pub i64, PhantomData<fn() -> T>);

impl<T> Id<T> {
    pub const fn new(raw: i64) -> Self { Self(raw, PhantomData) }
    pub const fn raw(&self) -> i64 { self.0 }
}

impl<T> Copy for Id<T> {}
impl<T> Clone for Id<T> { fn clone(&self) -> Self { *self } }
impl<T> PartialEq for Id<T> { fn eq(&self, o: &Self) -> bool { self.0 == o.0 } }
impl<T> Eq for Id<T> {}
impl<T> Hash for Id<T> { fn hash<H: Hasher>(&self, h: &mut H) { self.0.hash(h) } }
impl<T> fmt::Debug for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "Id({})", self.0) }
}

pub struct ActorMarker;
pub struct CharacterMarker;
pub struct EnvironmentMarker;
pub struct ItemMarker;
pub struct UtilityMarker;

pub type ActorId       = Id<ActorMarker>;
pub type CharacterId   = Id<CharacterMarker>;
pub type EnvironmentId = Id<EnvironmentMarker>;
pub type ItemId        = Id<ItemMarker>;
pub type UtilityId     = Id<UtilityMarker>;

// ── SubtypeId ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SubtypeId {
    Character(CharacterId),
    Environment(EnvironmentId),
    Item(ItemId),
    Utility(UtilityId),
}

impl SubtypeId {
    /// Maps variant to a slot index in Actor::sub_entities (matches ActorType::index).
    pub fn variant_idx(&self) -> usize {
        match self {
            SubtypeId::Character(_)   => 0,
            SubtypeId::Environment(_) => 1,
            SubtypeId::Item(_)        => 2,
            SubtypeId::Utility(_)     => 3,
        }
    }
}

// ── ActorAddress — globally-scoped display address for a SubEntity ──────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ActorAddress {
    pub actor:   ActorId,
    pub subtype: SubtypeId,
}

impl fmt::Display for ActorAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let subtype_raw = match self.subtype {
            SubtypeId::Character(id)   => id.raw(),
            SubtypeId::Environment(id) => id.raw(),
            SubtypeId::Item(id)        => id.raw(),
            SubtypeId::Utility(id)     => id.raw(),
        };
        write!(f, "{}.{}", self.actor.raw(), subtype_raw)
    }
}

// ── Actor sub-type data ─────────────────────────────────────────────────────

pub struct Character {
    pub id:       CharacterId,
    pub name:     Arc<str>,
    pub visible:  bool,
    pub physical: bool,
    pub playable: bool,
}

pub struct Environment {
    pub id:       EnvironmentId,
    pub name:     Arc<str>,
    pub visible:  bool,
    pub physical: bool,
}

pub struct Item {
    pub id:       ItemId,
    pub name:     Arc<str>,
    pub visible:  bool,
    pub physical: bool,
}

pub struct Utility {
    pub id:      UtilityId,
    pub name:    Arc<str>,
    pub visible: bool,
    pub toggle:  bool,
}

pub enum ActorType {
    Character(Character),
    Environment(Environment),
    Item(Item),
    Utility(Utility),
}

impl ActorType {
    pub const COUNT: usize = 4;

    /// Index into Actor::sub_entities for this variant (0–3).
    pub fn index(&self) -> usize {
        match self {
            ActorType::Character(_)   => 0,
            ActorType::Environment(_) => 1,
            ActorType::Item(_)        => 2,
            ActorType::Utility(_)     => 3,
        }
    }

    pub fn subtype_id(&self) -> SubtypeId {
        match self {
            ActorType::Character(c)   => SubtypeId::Character(c.id),
            ActorType::Environment(e) => SubtypeId::Environment(e.id),
            ActorType::Item(i)        => SubtypeId::Item(i.id),
            ActorType::Utility(u)     => SubtypeId::Utility(u.id),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            ActorType::Character(c)   => &c.name,
            ActorType::Environment(e) => &e.name,
            ActorType::Item(i)        => &i.name,
            ActorType::Utility(u)     => &u.name,
        }
    }
}

// ── SubEntity ───────────────────────────────────────────────────────────────
//
// Owned directly by Actor::sub_entities[variant_idx]. No parent pointer needed —
// the owner chain (Actor → Stage → Level → World) carries the context.

pub struct SubEntity {
    pub actor_type: ActorType,
    pub local:      Affine3A,
    pub world:      Affine3A,
    pub dirty:      bool,
    pub components: [Option<Component>; ComponentType::COUNT],
}

impl SubEntity {
    pub fn sub_entity_id(&self) -> SubtypeId { self.actor_type.subtype_id() }
    pub fn name(&self) -> &str { self.actor_type.name() }

    pub fn add_component(&mut self, comp: Component) {
        let idx = comp.component_type().index();
        self.components[idx] = Some(comp);
    }

    pub fn component(&self, ct: ComponentType) -> Option<&Component> {
        self.components[ct.index()].as_ref()
    }

    pub fn component_mut(&mut self, ct: ComponentType) -> Option<&mut Component> {
        self.components[ct.index()].as_mut()
    }

    pub fn remove_component(&mut self, ct: ComponentType) -> Option<Component> {
        self.components[ct.index()].take()
    }

    pub fn has_component(&self, ct: ComponentType) -> bool {
        self.components[ct.index()].is_some()
    }
}

// ── Actor ───────────────────────────────────────────────────────────────────
//
// Owned by Stage::actors arena. No parent pointer needed.
// sub_entities[i] corresponds to the ActorType variant whose index() == i.
// At most one sub-entity per ActorType variant — same invariant as before.

pub struct Actor {
    pub id:           ActorId,
    pub local:        Affine3A,
    pub world:        Affine3A,
    pub dirty:        bool,   // true = already queued in Stage::dirty_actors
    pub sub_entities: [Option<SubEntity>; ActorType::COUNT],
}
