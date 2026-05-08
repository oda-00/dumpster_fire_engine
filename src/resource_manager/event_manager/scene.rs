use std::sync::Mutex;
use std::sync::Arc;
use glam::{Affine3A, Vec3};
use thin_vec::ThinVec;
use crate::resource_manager::component::{Component, ComponentType};
use crate::resource_manager::manager::{
    ActorHandle, ActorId, ActorType, Handle, Id, LevelHandle, StageHandle,
};
use crate::resource_manager::world_manager::stage::StageId;
use crate::resource_manager::world_manager::world::World;

// ── Tags / markers / Ids owned by event_manager ─────────────────────────────

#[derive(Copy, Clone, PartialEq, Eq)] pub struct SceneTag;
pub type SceneHandle = Handle<SceneTag>;

pub struct SceneMarker;
pub type SceneId = Id<SceneMarker>;

pub struct EventMarker;
pub type EventId = Id<EventMarker>;

pub struct TroupeMarker;
pub type TroupeId = Id<TroupeMarker>;

// ── ActiveActor ─────────────────────────────────────────────────────────────
//
// Pointer to a runtime actor + per-scene status flags. Handles are obtained
// upstream (the user already called spawn_actor, which returned the handle)
// and supplied at SceneDef-construction time. ActorId is duplicated for fast
// equality vs Event payloads (which only carry Ids).

#[derive(Clone)]
pub struct ActiveActor {
    pub level_h:    LevelHandle,
    pub stage_h:    StageHandle,
    pub actor_h:    ActorHandle,
    pub actor_id:   ActorId,
    pub entered:    bool,
    pub current_op: i32,
    pub cued:       bool,
}

impl ActiveActor {
    pub fn new(
        level_h: LevelHandle,
        stage_h: StageHandle,
        actor_h: ActorHandle,
        actor_id: ActorId,
    ) -> Self {
        ActiveActor {
            level_h, stage_h, actor_h, actor_id,
            entered: false, current_op: -1, cued: false,
        }
    }
}

// ── Troupe (partitioned cast) ───────────────────────────────────────────────

/// Outer Vec aligns with Scene::troupes — troupes[i] keys actors.0[i].
#[derive(Clone)]
pub struct Troupe(pub Vec<Vec<ActiveActor>>);

impl Troupe {
    pub fn new() -> Self { Troupe(Vec::new()) }

    pub fn group(&self, idx: usize) -> Option<&[ActiveActor]> {
        self.0.get(idx).map(|v| v.as_slice())
    }

    pub fn group_mut(&mut self, idx: usize) -> Option<&mut Vec<ActiveActor>> {
        self.0.get_mut(idx)
    }

    pub fn iter_all(&self) -> impl Iterator<Item = &ActiveActor> {
        self.0.iter().flatten()
    }

    pub fn iter_all_mut(&mut self) -> impl Iterator<Item = &mut ActiveActor> {
        self.0.iter_mut().flatten()
    }
}

impl Default for Troupe { fn default() -> Self { Troupe::new() } }

// ── Payload (for Custom events) ─────────────────────────────────────────────

#[derive(Clone)]
pub enum Payload {
    None,
    Int(i64),
    Float(f32),
    Vec3(Vec3),
    Actor(ActorId),
    Text(Arc<str>),
    Pair(Arc<(Payload, Payload)>),
}

// ── Event ───────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub enum Event {
    SceneEntered(SceneId),
    SceneExited(SceneId),
    ActorEntered(ActorId),
    ActorExited(ActorId),
    ActorMoved { actor: ActorId, from: Vec3, to: Vec3 },
    ComponentAdded   { actor: ActorId, variant_idx: usize, component_type: ComponentType },
    ComponentRemoved { actor: ActorId, variant_idx: usize, component_type: ComponentType },
    Tick { dt: f32 },
    Custom(EventId, Arc<Payload>),
}

// ── EventTarget ─────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
pub enum EventTarget {
    Scene(SceneId),
    CurrentScene,
    Play,
}

// ── Effect ──────────────────────────────────────────────────────────────────
//
// Effects carry resolved handles where they have them (from the SceneDef author)
// and ActorIds for the troupe-cue case. apply_effect fans out to the existing
// World mutators.

pub enum Effect {
    SetActorLocal {
        level_h: LevelHandle,
        stage_h: StageHandle,
        actor_h: ActorHandle,
        local:   Affine3A,
    },
    SetSubEntityLocal {
        level_h:     LevelHandle,
        stage_h:     StageHandle,
        actor_h:     ActorHandle,
        variant_idx: usize,
        local:       Affine3A,
    },
    /// Boxed: `Component` is large (~80 B) and AddComponent is rare on the hot
    /// path. Boxing keeps the inline `Effect` discriminant slim so the per-tick
    /// `Vec<Effect>` working set fits more entries per cache line.
    AddComponent(Arc<AddComponentEffect>),
    RemoveComponent {
        level_h:        LevelHandle,
        stage_h:        StageHandle,
        actor_h:        ActorHandle,
        variant_idx:    usize,
        component_type: ComponentType,
    },
    SpawnActor {
        level_h: LevelHandle,
        stage_h: StageHandle,
        id:      ActorId,
        local:   Affine3A,
    },
    /// Boxed: `ActorType` is a large enum + 64 B Affine3A. Same rationale as
    /// AddComponent — rare emit, big payload, no need to bloat the inline size.
    SpawnSubEntity(Arc<SpawnSubEntityEffect>),
    DespawnActor {
        level_h: LevelHandle,
        stage_h: StageHandle,
        actor_h: ActorHandle,
    },
    DespawnSubEntity {
        level_h:     LevelHandle,
        stage_h:     StageHandle,
        actor_h:     ActorHandle,
        variant_idx: usize,
    },
    /// Whole-troupe cue. apply_effect resolves the troupe roster from the active
    /// scene and fans out to per-actor SetActorLocal calls (delta * actor.local).
    CueTroupe {
        level_h: LevelHandle,
        stage_h: StageHandle,
        troupe:  TroupeId,
        delta:   Affine3A,
    },
    /// Emit an event onto a queue chosen by target.
    Emit {
        level_h: LevelHandle,
        stage_h: StageHandle,
        target:  EventTarget,
        event:   Event,
    },
    /// Mealy-style transition record consumed by Play::post_tick_bookkeeping.
    ScheduleTransition {
        level_h: LevelHandle,
        stage_h: StageHandle,
        source:  SceneId,
        target:  SceneId,
        mealy:   ThinVec<Effect>,
    },
}

/// Heap-allocated payload for the rare `AddComponent` effect.
pub struct AddComponentEffect {
    pub level_h:     LevelHandle,
    pub stage_h:     StageHandle,
    pub actor_h:     ActorHandle,
    pub variant_idx: usize,
    pub component:   Component,
}

/// Heap-allocated payload for the rare `SpawnSubEntity` effect.
pub struct SpawnSubEntityEffect {
    pub level_h:    LevelHandle,
    pub stage_h:    StageHandle,
    pub actor_h:    ActorHandle,
    pub actor_type: ActorType,
    pub local:      Affine3A,
}

impl Clone for Effect {
    fn clone(&self) -> Self {
        match self {
            Effect::SetActorLocal { level_h, stage_h, actor_h, local } =>
                Effect::SetActorLocal { level_h: *level_h, stage_h: *stage_h, actor_h: *actor_h, local: *local },
            Effect::SetSubEntityLocal { level_h, stage_h, actor_h, variant_idx, local } =>
                Effect::SetSubEntityLocal { level_h: *level_h, stage_h: *stage_h, actor_h: *actor_h, variant_idx: *variant_idx, local: *local },
            Effect::AddComponent(b) =>
                Effect::AddComponent(Arc::new(AddComponentEffect {
                    level_h: b.level_h, stage_h: b.stage_h, actor_h: b.actor_h,
                    variant_idx: b.variant_idx,
                    component: clone_component(&b.component),
                }))
            Effect::RemoveComponent { level_h, stage_h, actor_h, variant_idx, component_type } =>
                Effect::RemoveComponent { level_h: *level_h, stage_h: *stage_h, actor_h: *actor_h, variant_idx: *variant_idx, component_type: *component_type },
            Effect::SpawnActor { level_h, stage_h, id, local } =>
                Effect::SpawnActor { level_h: *level_h, stage_h: *stage_h, id: *id, local: *local },
            Effect::SpawnSubEntity(_) => panic!(
                "Effect::SpawnSubEntity is not Clone — ActorType holds non-Clone fields. \
                 Author it directly into a Transition or fire it once via on_enter."
            ),
            Effect::DespawnActor { level_h, stage_h, actor_h } =>
                Effect::DespawnActor { level_h: *level_h, stage_h: *stage_h, actor_h: *actor_h },
            Effect::DespawnSubEntity { level_h, stage_h, actor_h, variant_idx } =>
                Effect::DespawnSubEntity { level_h: *level_h, stage_h: *stage_h, actor_h: *actor_h, variant_idx: *variant_idx },
            Effect::CueTroupe { level_h, stage_h, troupe, delta } =>
                Effect::CueTroupe { level_h: *level_h, stage_h: *stage_h, troupe: *troupe, delta: *delta },
            Effect::Emit { level_h, stage_h, target, event } =>
                Effect::Emit { level_h: *level_h, stage_h: *stage_h, target: *target, event: event.clone() },
            Effect::ScheduleTransition { level_h, stage_h, source, target, mealy } =>
                Effect::ScheduleTransition { level_h: *level_h, stage_h: *stage_h, source: *source, target: *target, mealy: mealy.clone() },
        }
    }
}

/// Component does not derive Clone in component.rs (and that file is owned by
/// resource_manager — we don't touch it). For the effect-buffer model we need
/// to clone Effect; this helper rebuilds a Component by matching on its variant.
fn clone_component(c: &Component) -> Component {
    use crate::resource_manager::component::*;
    match c {
        Component::Transform(t) => Component::Transform(TransformComponent {
            position: t.position, rotation: t.rotation, scale: t.scale, _transform: t._transform,
        }),
        Component::Audio(a) => Component::Audio(AudioComponent {
            volume: a.volume, pitch: a.pitch, _loop: a._loop, _playing: a._playing,
        }),
        Component::Physics(p) => Component::Physics(PhysicsComponent {
            mass: p.mass, velocity: p.velocity, acceleration: p.acceleration,
        }),
        Component::Collision(c) => Component::Collision(CollisionComponent {
            shape: clone_shape(&c.shape),
            position: c.position, rotation: c.rotation, scale: c.scale, collision: c.collision,
        }),
        Component::Utility(u) => Component::Utility(UtilityComponent {
            name: u.name.clone(), description: u.description.clone(),
        }),
    }
}

fn clone_shape(s: &crate::resource_manager::component::CollisionShape) -> crate::resource_manager::component::CollisionShape {
    use crate::resource_manager::component::CollisionShape;
    match s {
        CollisionShape::Box     => CollisionShape::Box,
        CollisionShape::Sphere  => CollisionShape::Sphere,
        CollisionShape::Capsule => CollisionShape::Capsule,
        CollisionShape::Mesh    => CollisionShape::Mesh,
    }
}

// ── Condition ───────────────────────────────────────────────────────────────

#[derive(Clone)]
pub enum Condition {
    Always,
    Never,
    OnEnter,
    AfterSeconds(f32),
    OnTick(u64),
    ActorNear { actor: ActorId, target: Vec3, radius: f32 },
    ActorMovedThisTick(ActorId),
    ActorHasComponent { actor: ActorId, component_type: ComponentType },
    /// Every actor in the named troupe satisfies the inner predicate.
    TroupeAll { troupe: TroupeId, predicate: Arc<Condition> },
    /// Any actor in the named troupe satisfies the inner predicate.
    TroupeAny { troupe: TroupeId, predicate: Arc<Condition> },
    /// A Custom event with this id was drained this tick.
    EventFired(EventId),
    All(ThinVec<Condition>),
    Any(ThinVec<Condition>),
    Not(Arc<Condition>),
    /// Function-pointer escape hatch — Copy, no heap, no captures.
    Custom(fn(&EvalCtx<'_>) -> bool),
}

// ── EvalCtx ─────────────────────────────────────────────────────────────────

pub struct EvalCtx<'a> {
    pub world:       &'a World,
    pub level_h:     LevelHandle,
    pub stage_h:     StageHandle,
    pub scene_id:    SceneId,
    pub elapsed:     f32,
    pub tick_count:  u64,
    pub events_seen: &'a [Event],
    pub actors:      &'a Troupe,
    pub troupes:     &'a [TroupeId],
}

impl Condition {
    pub fn eval(&self, ctx: &EvalCtx<'_>) -> bool {
        match self {
            Condition::Always => true,
            Condition::Never  => false,
            Condition::OnEnter => ctx.tick_count == 0,
            Condition::AfterSeconds(t) => ctx.elapsed >= *t,
            Condition::OnTick(n) => ctx.tick_count == *n,
            Condition::ActorNear { actor, target, radius } => {
                actor_world_pos(ctx, *actor)
                    .map(|p| p.distance(*target) <= *radius)
                    .unwrap_or(false)
            }
            Condition::ActorMovedThisTick(id) => {
                ctx.actors.iter_all().any(|a| a.actor_id == *id && a.cued)
            }
            Condition::ActorHasComponent { actor, component_type } => {
                actor_has_component(ctx, *actor, *component_type)
            }
            Condition::TroupeAll { troupe, predicate } => {
                let Some(idx) = ctx.troupes.iter().position(|t| t == troupe) else { return false };
                let Some(group) = ctx.actors.group(idx) else { return false };
                group.iter().all(|a| predicate.eval_for_actor(ctx, a.actor_id))
            }
            Condition::TroupeAny { troupe, predicate } => {
                let Some(idx) = ctx.troupes.iter().position(|t| t == troupe) else { return false };
                let Some(group) = ctx.actors.group(idx) else { return false };
                group.iter().any(|a| predicate.eval_for_actor(ctx, a.actor_id))
            }
            Condition::EventFired(id) => {
                ctx.events_seen.iter().any(|e| matches!(e, Event::Custom(eid, _) if eid == id))
            }
            Condition::All(cs) => cs.iter().all(|c| c.eval(ctx)),
            Condition::Any(cs) => cs.iter().any(|c| c.eval(ctx)),
            Condition::Not(c) => !c.eval(ctx),
            Condition::Custom(f) => f(ctx),
        }
    }

    /// Re-evaluate a sub-condition with `actor` as the implicit subject (used by
    /// TroupeAll / TroupeAny so an inner ActorNear-style predicate can be
    /// retargeted at each member of the troupe).
    fn eval_for_actor(&self, ctx: &EvalCtx<'_>, actor: ActorId) -> bool {
        match self {
            Condition::ActorNear { target, radius, .. } => {
                actor_world_pos(ctx, actor)
                    .map(|p| p.distance(*target) <= *radius)
                    .unwrap_or(false)
            }
            Condition::ActorHasComponent { component_type, .. } =>
                actor_has_component(ctx, actor, *component_type),
            other => other.eval(ctx),
        }
    }
}

fn actor_world_pos(ctx: &EvalCtx<'_>, id: ActorId) -> Option<Vec3> {
    // Find ActiveActor first (cheap — small troupes), then resolve handle.
    let active = ctx.actors.iter_all().find(|a| a.actor_id == id)?;
    let stage  = ctx.world.levels.get(active.level_h)?.stages.get(active.stage_h)?;
    // Generation-gate via the Arena, then index the SoA `worlds` array.
    if !stage.actors.contains(active.actor_h) { return None }
    Some(stage.worlds[active.actor_h.idx as usize].translation.into())
}

fn actor_has_component(ctx: &EvalCtx<'_>, id: ActorId, ct: ComponentType) -> bool {
    let Some(active) = ctx.actors.iter_all().find(|a| a.actor_id == id) else { return false };
    let Some(stage)  = ctx.world.levels.get(active.level_h).and_then(|l| l.stages.get(active.stage_h))
    else { return false };
    let Some(actor)  = stage.actors.get(active.actor_h) else { return false };
    actor.sub_entities.iter().filter_map(|s| s.as_ref()).any(|s| s.has_component(ct))
}

// ── Handler / EventMatcher ──────────────────────────────────────────────────

#[derive(Clone)]
pub enum EventMatcher {
    Any,
    SceneEntered,
    SceneExited,
    ActorEntered,
    ActorExited,
    ActorMoved,
    Tick,
    Custom(EventId),
}

impl EventMatcher {
    pub fn matches(&self, e: &Event) -> bool {
        match (self, e) {
            (EventMatcher::Any, _) => true,
            (EventMatcher::SceneEntered, Event::SceneEntered(_)) => true,
            (EventMatcher::SceneExited,  Event::SceneExited(_))  => true,
            (EventMatcher::ActorEntered, Event::ActorEntered(_)) => true,
            (EventMatcher::ActorExited,  Event::ActorExited(_))  => true,
            (EventMatcher::ActorMoved,   Event::ActorMoved { .. }) => true,
            (EventMatcher::Tick,         Event::Tick { .. }) => true,
            (EventMatcher::Custom(a), Event::Custom(b, _)) => a == b,
            _ => false,
        }
    }
}

#[derive(Clone)]
pub struct Handler {
    pub matcher: EventMatcher,
    pub action:  fn(&Event, &EvalCtx<'_>, &mut Vec<Effect>),
}

// ── SceneOperation ──────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct SceneOperation {
    pub condition: Condition,
    pub effect:    Arc<Effect>,
    /// Cell so pass 1 (read-only against the Scene) can mark a once-op as fired.
    pub fired:     Mutex<bool>,
    pub once:      bool,
}

// ── Behavior Tree ───────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BtStatus { Running, Success, Failure }

#[derive(Clone, Copy)]
pub enum ParallelPolicy { AllSucceed, AnySucceed, AllComplete }

#[derive(Clone)]
pub enum Decorator {
    Inverter,
    Guard(Condition),
    UntilSuccess,
    Cooldown(f32),
}

#[derive(Clone)]
pub enum BtNode {
    Sequence(Vec<BtNode>),
    Selector(Vec<BtNode>),
    Parallel { children: Vec<BtNode>, policy: ParallelPolicy },
    Repeat   { child: Arc<BtNode>, count: u32, current: Cell<u32> },
    Decorator { decorator: Decorator, child: Arc<BtNode> },
    Leaf(SceneOperation),
}

impl BtNode {
    pub fn leaf(condition: Condition, effect: Effect, once: bool) -> Self {
        BtNode::Leaf(SceneOperation {
            condition, effect, fired: Mutex::new(false), once,
        })
    }

    /// Empty BT — used by Compound / AndParallel scenes whose body lives in their children.
    pub fn empty() -> Self { BtNode::Sequence(Vec::new()) }

    /// Tick this node; push fired effects into `out`. &self because pass 1 is
    /// read-only with respect to World; per-node mutable state uses Cell.
    pub fn tick(&self, ctx: &EvalCtx<'_>, out: &mut Vec<Effect>) -> BtStatus {
        match self {
            BtNode::Sequence(children) => {
                for c in children {
                    match c.tick(ctx, out) {
                        BtStatus::Success => continue,
                        BtStatus::Running => return BtStatus::Running,
                        BtStatus::Failure => return BtStatus::Failure,
                    }
                }
                BtStatus::Success
            }
            BtNode::Selector(children) => {
                for c in children {
                    match c.tick(ctx, out) {
                        BtStatus::Failure => continue,
                        BtStatus::Running => return BtStatus::Running,
                        BtStatus::Success => return BtStatus::Success,
                    }
                }
                BtStatus::Failure
            }
            BtNode::Parallel { children, policy } => {
                let mut succ = 0u32;
                let mut fail = 0u32;
                let mut running = 0u32;
                for c in children {
                    match c.tick(ctx, out) {
                        BtStatus::Success => succ += 1,
                        BtStatus::Failure => fail += 1,
                        BtStatus::Running => running += 1,
                    }
                }
                let total = children.len() as u32;
                match policy {
                    ParallelPolicy::AllSucceed => {
                        if succ == total { BtStatus::Success }
                        else if fail > 0 { BtStatus::Failure }
                        else { BtStatus::Running }
                    }
                    ParallelPolicy::AnySucceed => {
                        if succ > 0 { BtStatus::Success }
                        else if running > 0 { BtStatus::Running }
                        else { BtStatus::Failure }
                    }
                    ParallelPolicy::AllComplete => {
                        if running > 0 { BtStatus::Running }
                        else if fail > 0 { BtStatus::Failure }
                        else { BtStatus::Success }
                    }
                }
            }
            BtNode::Repeat { child, count, current } => {
                let target = *count;
                let mut iters = current.get();
                if target != 0 && iters >= target {
                    return BtStatus::Success;
                }
                match child.tick(ctx, out) {
                    BtStatus::Success => {
                        iters = iters.saturating_add(1);
                        current.set(iters);
                        if target != 0 && iters >= target { BtStatus::Success }
                        else { BtStatus::Running }
                    }
                    BtStatus::Failure => BtStatus::Failure,
                    BtStatus::Running => BtStatus::Running,
                }
            }
            BtNode::Decorator { decorator, child } => match decorator {
                Decorator::Inverter => match child.tick(ctx, out) {
                    BtStatus::Success => BtStatus::Failure,
                    BtStatus::Failure => BtStatus::Success,
                    BtStatus::Running => BtStatus::Running,
                },
                Decorator::Guard(cond) => {
                    if cond.eval(ctx) { child.tick(ctx, out) } else { BtStatus::Success }
                }
                Decorator::UntilSuccess => match child.tick(ctx, out) {
                    BtStatus::Success => BtStatus::Success,
                    _ => BtStatus::Running,
                },
                Decorator::Cooldown(_s) => child.tick(ctx, out),
            },
            BtNode::Leaf(op) => {
                if op.once && op.fired.get() {
                    return BtStatus::Success;
                }
                if op.condition.eval(ctx) {
                    out.push(op.effect.clone());
                    if op.once { op.fired.set(true); }
                    BtStatus::Success
                } else {
                    BtStatus::Running
                }
            }
        }
    }

    /// Reset BT runtime state (Repeat counters, once-fired flags) on scene re-entry.
    pub fn reset(&self) {
        match self {
            BtNode::Sequence(cs) | BtNode::Selector(cs) =>
                for c in cs { c.reset() },
            BtNode::Parallel { children, .. } =>
                for c in children { c.reset() },
            BtNode::Repeat { child, current, .. } => {
                current.set(0); child.reset();
            }
            BtNode::Decorator { child, .. } => child.reset(),
            BtNode::Leaf(op) => op.fired.set(false),
        }
    }
}

#[derive(Clone)]
pub struct BtState {
    pub last_status:      BtStatus,
    pub last_run_elapsed: f32,
}

impl Default for BtState {
    fn default() -> Self {
        BtState { last_status: BtStatus::Running, last_run_elapsed: 0.0 }
    }
}

// ── Transition ──────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct Transition {
    pub condition: Condition,
    pub target:    SceneId,
    /// Mealy outputs: applied (at next tick's pass 2) when this edge is taken.
    pub effects:   ThinVec<Effect>,
}

// ── HSM scene shape ─────────────────────────────────────────────────────────

#[derive(Clone)]
pub enum SceneKind {
    Atomic,
    Compound { children: ThinVec<SceneId>, initial: SceneId, history: Option<SceneId> },
    AndParallel { regions: ThinVec<Region> },
}

#[derive(Clone)]
pub struct Region {
    pub children: ThinVec<SceneId>,
    pub initial:  SceneId,
    pub history:  Option<SceneId>,
}

// ── SceneDef (authored, in Script) ──────────────────────────────────────────

#[derive(Clone)]
pub struct SceneDef {
    pub id:             SceneId,
    pub stage:          StageId,
    pub parent:         Option<SceneId>,
    pub kind:           SceneKind,
    pub troupes:        ThinVec<TroupeId>,
    /// Outer Vec parallel to `troupes`. Each ActiveActor is pre-resolved by the
    /// engine user (they have the handles from earlier spawn_actor calls).
    pub initial_actors: ThinVec<ThinVec<ActiveActor>>,
    pub root:           BtNode,
    pub on_enter:       ThinVec<Effect>,
    pub on_exit:        ThinVec<Effect>,
    pub handlers:       ThinVec<Handler>,
    pub transitions:    ThinVec<Transition>,
}

// ── Scene (runtime) ─────────────────────────────────────────────────────────

pub struct Scene {
    pub id:          SceneId,
    pub stage:       StageId,
    pub parent:      Option<SceneId>,
    pub kind:        SceneKind,
    pub troupes:     Vec<TroupeId>,
    pub actors:      Troupe,
    pub root:        BtNode,
    pub bt_state:    BtState,
    pub handlers:    Vec<Handler>,
    pub transitions: Vec<Transition>,
    pub queue:       ThinVec<Event>,
    pub elapsed:     f32,
    pub tick_count:  u64,
    pub entered:     bool,
    pub _rendered:   bool,
}

impl Scene {
    pub fn from_def(def: &SceneDef) -> Self {
        let actors = Troupe(
            def.initial_actors
                .iter()
                .map(|group| group.iter().cloned().collect::<Vec<_>>())
                .collect(),
        );
        Scene {
            id:          def.id,
            stage:       def.stage,
            parent:      def.parent,
            kind:        def.kind.clone(),
            troupes:     def.troupes.iter().copied().collect(),
            actors,
            root:        def.root.clone(),
            bt_state:    BtState::default(),
            handlers:    def.handlers.iter().cloned().collect(),
            transitions: def.transitions.iter().cloned().collect(),
            queue:       ThinVec::new(),
            elapsed:     0.0,
            tick_count:  0,
            entered:     false,
            _rendered:   false,
        }
    }

    pub fn troupe_idx(&self, id: TroupeId) -> Option<usize> {
        self.troupes.iter().position(|t| *t == id)
    }
}
