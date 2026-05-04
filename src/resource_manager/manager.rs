use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::sync::Arc;
use std::collections::HashMap;
use slotmap::new_key_type;
use super::comps::*;
use super::world_manager::stage::StageKey;

new_key_type! {
    pub struct ActorKey;
    pub struct SubEntityKey;
}


pub struct Handle<Tag>  {
    pub idx: u32,
    pub gen: u32,
    _t: PhantomData<fn() -> Tag>,
}

pub struct LevelTag;
pub struct StageTag;
pub struct ActorTag;
pub struct SubEntityTag;

pub type LevelHandle = Handle<LevelTag>;
pub type StageHandle = Handle<StageTag>;
pub type ActorHandle = Handle<ActorTag>;
pub type SubEntityHandle = Handle<SubEntityTag>;

struct Slot<T>{ gen:u32 val Option<T> }

pub struct Arena <Tag, T> {
    slots: Vec<Slot<T>>,
    free: ThinVec<u32>,
    len: u32,
    _tag: PhantomData<fn() -> Tag>,
}


pub struct Id<T>(pub i64, PhantomData<fn() -> T>);

impl<T> Id<T> {
    pub const fn new(raw: i64) -> Self { Self(raw, PhantomData) }
    pub const fn raw(&self) -> i64 { self.0 }
}

impl<T> Copy for Id<T> {}
impl<T> Clone for Id<T> { fn clone(&self) -> Self { *self } }
impl<T> PartialEq for Id<T> {
    fn eq(&self, o: &Self) -> bool { self.0 == o.0 }
}
impl<T> Eq for Id<T> {}
impl<T> Hash for Id<T> {
    fn hash<H: Hasher>(&self, h: &mut H) { self.0.hash(h) }
}
impl<T> fmt::Debug for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Id({})", self.0)
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SubtypeId {
    Character(CharacterId),
    Environment(EnvironmentId),
    Item(ItemId),
    Utility(UtilityId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ActorAddress {
    pub actor: ActorId,
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

#[derive(Debug, Clone, Copy)]
pub struct Transform {
    pub position: (f32, f32, f32),
    pub rotation: (f32, f32, f32),
    pub scale:    (f32, f32, f32),
}

impl Transform {
    pub const IDENTITY: Self = Self {
        position: (0.0, 0.0, 0.0),
        rotation: (0.0, 0.0, 0.0),
        scale:    (1.0, 1.0, 1.0),
    };

    pub const fn new(
        position: (f32, f32, f32),
        rotation: (f32, f32, f32),
        scale:    (f32, f32, f32),
    ) -> Self {
        Self { position, rotation, scale }
    }

    // Rotation is summed component-wise (Euler) — only correct for axis-aligned rotations.
    // Swap to quaternions before relying on this for arbitrary orientations.
    pub fn compose(parent: &Self, child: &Self) -> Self {
        Self {
            position: (
                parent.position.0 + parent.scale.0 * child.position.0,
                parent.position.1 + parent.scale.1 * child.position.1,
                parent.position.2 + parent.scale.2 * child.position.2,
            ),
            rotation: (
                parent.rotation.0 + child.rotation.0,
                parent.rotation.1 + child.rotation.1,
                parent.rotation.2 + child.rotation.2,
            ),
            scale: (
                parent.scale.0 * child.scale.0,
                parent.scale.1 * child.scale.1,
                parent.scale.2 * child.scale.2,
            ),
        }
    }
}

pub struct Character {
    pub id: CharacterId,
    pub name: Arc<str>,
    pub visible: bool,
    pub physical: bool,
    pub playable: bool,
}

pub struct Environment {
    pub id: EnvironmentId,
    pub name: Arc<str>,
    pub visible: bool,
    pub physical: bool,
}

pub struct Item {
    pub id: ItemId,
    pub name: Arc<str>,
    pub visible: bool,
    pub physical: bool,
}

pub struct Utility {
    pub id: UtilityId,
    pub name: Arc<str>,
    pub visible: bool,
    pub toggle: bool,
}

pub enum ActorType {
    Character(Character) = 0,
    Environment(Environment) = 1,
    Item(Item) = 2,
    Utility(Utility) = 3,
}

impl ActorType {
    pub const COUNT: usize = 4;

    #[inline]
    pub const fn index(self) -> usize { self as usize }

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

pub struct SubEntity {
    pub actor_type: ActorType,
    pub local: Transform,
    pub world: Transform,
    pub dirty: bool,
    pub components: [Vec<Component>; ComponentType::COUNT],
    pub parent: ActorKey,
}

impl SubEntity {
    pub fn sub_entity_id(&self) -> SubtypeId {
        self.actor_type.subtype_id()
    }
    

    pub fn name(&self) -> &str {
        self.actor_type.name()
    }

    pub fn add_component(&mut self, comp: Component) {
        let idx = comp.component_type().index();
        self.components[idx] = Some(comp);
    }

    pub fn component(&self, comp_type: ComponentType) -> Option<&Component> {
        self.components[comp_type.index()].as_ref()
    }

    pub fn component_mut(&mut self, comp_type: ComponentType) -> Option<&mut Component> {
        self.components[comp_type.index()].as_mut()
    }

    pub fn remove_component(&mut self, comp_type: ComponentType) -> Option<Component> {
        self.components[comp_type.index()].take()
    }

    pub fn has_component(&self, comp_type: ComponentType) -> bool {
        self.components[comp_type.index()].is_some()
    }
}

pub struct Actor {
    pub id: ActorId,
    pub local: Transform,
    pub world: Transform,
    pub dirty: bool,
    pub parent: StageKey,
    pub children: [Vec<SubEntityKey>; ActorType::COUNT],
}
