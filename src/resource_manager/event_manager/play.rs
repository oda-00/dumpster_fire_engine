use std::sync::Arc;
use glam::Affine3A;
use thin_vec::ThinVec;
use crate::resource_manager::manager::{Arena, Handle, Id, LevelHandle, StageHandle};
use crate::resource_manager::world_manager::stage::StageId;
use crate::resource_manager::world_manager::world::World;
use super::scene::{
    BtNode, BtState, EvalCtx, Effect, Event, Handler, Scene, SceneHandle, SceneId,
    SceneKind, SceneTag, TroupeId,
};
use super::script::{Script, ScriptId};

// ── Tags / markers / Ids owned by play.rs ───────────────────────────────────

#[derive(Copy, Clone, PartialEq, Eq)] pub struct PlayTag;
pub type PlayHandle = Handle<PlayTag>;

pub struct PlayMarker;
pub type PlayId = Id<PlayMarker>;

// ── Pending transition record ───────────────────────────────────────────────

pub struct TransitionRecord {
    pub source: SceneId,
    pub target: SceneId,
    /// Arc-shared slice of Mealy outputs so passing the record around is cheap
    /// (refcount bump, not a Vec copy).
    pub mealy:  Arc<[Effect]>,
}

// ── Play ────────────────────────────────────────────────────────────────────

pub struct Play {
    pub id:        PlayId,
    pub name:      Arc<str>,
    pub script_id: ScriptId,
    pub stage:     StageId,
    pub level_h:   LevelHandle,
    pub stage_h:   StageHandle,

    pub scenes:    Arena<SceneTag, Scene>,

    /// HSM root scene handle. Always valid after instantiate.
    pub root:      SceneHandle,

    /// Leaf scenes currently active. For Atomic-only scripts this has length 1;
    /// for AndParallel scenes at deepest level it has length ≥ k.
    pub active_leaves: ThinVec<SceneHandle>,

    /// History pseudostate cache: (parent_scene_id, last_visited_child_id).
    pub history: ThinVec<(SceneId, SceneId)>,

    /// Resolution table: SceneId → SceneHandle. Linear search is fine at
    /// typical script sizes.
    pub by_id: ThinVec<(SceneId, SceneHandle)>,

    pub handlers: ThinVec<Handler>,
    pub queue:    ThinVec<Event>,

    /// Troupes whose authored cues are entirely IDENTITY (or which never appear
    /// as a CueTroupe target at all). Computed at instantiate time. Stage's
    /// cue_troupe_direct uses this to skip the scratch fill + dirty walk for
    /// cues that won't move anything.
    pub static_troupes: ThinVec<crate::resource_manager::event_manager::TroupeId>,

    /// Filled by World::apply_effect when an Effect::ScheduleTransition fires;
    /// consumed by post_tick_bookkeeping during pass 3.
    pub pending_transition: Option<TransitionRecord>,

    /// Mealy outputs accumulated when a transition is applied; drained by
    /// World::tick at the start of the *next* tick's pass 2 so the one-tick
    /// latency rule holds.
    pub pending_mealy: Vec<Effect>,

    pub paused:   bool,
    pub finished: bool,

    // ── Per-Play scratch buffers ──────────────────────────────────────────
    //
    // Pre-allocated buffers reused every tick to eliminate the transient Vec
    // allocations that appear in apply_transition and active_configuration.
    // Capacity grows to the watermark of the deepest HSM seen at runtime and
    // is never shrunk, so steady-state operation is allocation-free.
    //
    // ancestor_scratch:   source ancestor chain (root → src) in apply_transition
    // transition_scratch: target ancestor chain (root → tgt) in apply_transition;
    //                     also repurposed as the "leaves to keep" buffer after
    //                     the tgt chain is no longer needed.
    // config_scratch:     active configuration set in post_tick_bookkeeping
    ancestor_scratch:   Vec<SceneHandle>,
    transition_scratch: Vec<SceneHandle>,
    config_scratch:     Vec<SceneHandle>,
}

impl Play {
    /// Build a runtime Play from a Script. The Script tree is materialized into
    /// the Scene arena; the entry scene becomes the HSM root and its initial
    /// child chain seeds active_leaves.
    pub fn instantiate(
        id:      PlayId,
        name:    impl Into<Arc<str>>,
        script:  &Script,
        stage:   StageId,
        level_h: LevelHandle,
        stage_h: StageHandle,
    ) -> Self {
        let mut scenes: Arena<SceneTag, Scene> = Arena::new();
        let mut by_id: ThinVec<(SceneId, SceneHandle)> = ThinVec::new();

        for def in script.scenes.iter() {
            let h = scenes.insert(Scene::from_def(def));
            by_id.push((def.id, h));
        }

        let root_h = by_id.iter()
            .find(|(id, _)| *id == script.entry)
            .map(|(_, h)| *h)
            .expect("Script::entry must reference a scene that was added via add_scene");

        // Static-troupe analysis: collect every troupe id that appears in the
        // script (either declared by a SceneDef or named by a CueTroupe effect),
        // then subtract the troupes that receive at least one non-identity cue.
        // What's left is the set of "static" troupes whose cues are guaranteed
        // not to move any actor — Stage::cue_troupe_direct can skip them.
        let static_troupes = compute_static_troupes(script);

        let mut play = Play {
            id,
            name: name.into(),
            script_id: script.id,
            stage,
            level_h,
            stage_h,
            scenes,
            root: root_h,
            active_leaves: ThinVec::new(),
            history: ThinVec::new(),
            by_id,
            handlers: script.handlers.iter().cloned().collect(),
            queue: ThinVec::new(),
            static_troupes,
            pending_transition: None,
            pending_mealy: Vec::new(),
            paused: false,
            finished: false,
            ancestor_scratch:   Vec::with_capacity(16),
            transition_scratch: Vec::with_capacity(16),
            config_scratch:     Vec::with_capacity(16),
        };

        play.descend_to_leaves(root_h);
        // Fire SceneEntered for every newly-active scene in the configuration.
        for h in play.active_configuration() {
            let id = play.scenes[h].id;
            play.queue.push(Event::SceneEntered(id));
        }
        play
    }

    pub fn handle_for(&self, id: SceneId) -> Option<SceneHandle> {
        self.by_id.iter().find(|(k, _)| *k == id).map(|(_, h)| *h)
    }

    /// All scenes currently active = each leaf plus its ancestor chain (deduped).
    pub fn active_configuration(&self) -> Vec<SceneHandle> {
        let mut out: Vec<SceneHandle> = Vec::new();
        let mut scratch = Vec::with_capacity(8);
        for &leaf in self.active_leaves.iter() {
            ancestors_into_fields(&self.scenes, &self.by_id, leaf, &mut scratch);
            for &h in scratch.iter() {
                if !out.contains(&h) { out.push(h); }
            }
        }
        out
    }

    /// Walk from `start` through Compound::initial / AndParallel regions,
    /// pushing every reached leaf into active_leaves.
    fn descend_to_leaves(&mut self, start: SceneHandle) {
        let scene = &self.scenes[start];
        match &scene.kind {
            SceneKind::Atomic => {
                self.active_leaves.push(start);
            }
            SceneKind::Compound { initial, history, .. } => {
                let initial = *initial;
                let history = *history;
                let target = self.history_for(self.scenes[start].id)
                    .or(history)
                    .unwrap_or(initial);
                if let Some(child_h) = self.handle_for(target) {
                    self.descend_to_leaves(child_h);
                } else {
                    self.active_leaves.push(start);
                }
            }
            SceneKind::AndParallel { regions } => {
                let regions = regions.clone();
                for region in regions.iter() {
                    let target = region.history.unwrap_or(region.initial);
                    if let Some(child_h) = self.handle_for(target) {
                        self.descend_to_leaves(child_h);
                    }
                }
            }
        }
    }

    fn history_for(&self, parent: SceneId) -> Option<SceneId> {
        self.history.iter().find(|(p, _)| *p == parent).map(|(_, c)| *c)
    }


    /// Pass 1 — read-only — walk the active configuration, tick BTs, dispatch
    /// handlers, evaluate transitions.
    pub fn collect_effects(&self, _dt: f32, world: &World, out: &mut Vec<Effect>) {
        if self.paused || self.finished { return; }

        let mut play_queue_dispatched = false;
        // Single Vec reused across all leaves; avoids one allocation per leaf.
        let mut chain: Vec<SceneHandle> = Vec::with_capacity(8);

        for &leaf in self.active_leaves.iter() {
            ancestors_into_fields(&self.scenes, &self.by_id, leaf, &mut chain);

            // leaf-first walk so innermost transition wins.
            for &handle in chain.iter().rev() {
                let scene = &self.scenes[handle];

                let ctx = EvalCtx {
                    world,
                    level_h:     self.level_h,
                    stage_h:     self.stage_h,
                    scene_id:    scene.id,
                    elapsed:     scene.elapsed,
                    tick_count:  scene.tick_count,
                    events_seen: &scene.queue,
                    actors:      &scene.actors,
                    troupes:     &scene.troupes,
                };

                // 1. Tick BT body (Atomic only).
                if matches!(scene.kind, SceneKind::Atomic) {
                    let _status = scene.root.tick(&ctx, out);
                }

                // 2. Scene-local handler dispatch.
                for ev in scene.queue.iter() {
                    for h in scene.handlers.iter() {
                        if h.matcher.matches(ev) {
                            (h.action)(ev, &ctx, out);
                        }
                    }
                }

                // 3. Play-global handler dispatch (once per tick).
                if !play_queue_dispatched {
                    for ev in self.queue.iter() {
                        for h in self.handlers.iter() {
                            if h.matcher.matches(ev) {
                                (h.action)(ev, &ctx, out);
                            }
                        }
                    }
                }

                // 4. Transition evaluation — first match wins. Arc::clone is a
                //    single atomic refcount bump — no heap allocation.
                for t in scene.transitions.iter() {
                    if t.condition.eval(&ctx) {
                        out.push(Effect::ScheduleTransition {
                            level_h: self.level_h,
                            stage_h: self.stage_h,
                            source:  scene.id,
                            target:  t.target,
                            mealy:   Arc::clone(&t.effects),
                        });
                        break;
                    }
                }
            }

            play_queue_dispatched = true;
        }
    }

    /// Pass 3 — mut — drain queues, advance counters, apply pending transition.
    pub fn post_tick_bookkeeping(&mut self, dt: f32) {
        if self.paused || self.finished { return; }

        // Build active config into config_scratch, reusing its allocation.
        active_configuration_into(
            &self.scenes,
            &self.by_id,
            &self.active_leaves,
            &mut self.config_scratch,
            &mut self.ancestor_scratch,
        );
        // SAFETY: config_scratch is populated above; take a copy of the len
        // so we can iterate by index without holding a borrow on config_scratch.
        let n_active = self.config_scratch.len();

        for i in 0..n_active {
            let h = self.config_scratch[i];
            let scene = &mut self.scenes[h];
            scene.elapsed += dt;
            scene.tick_count = scene.tick_count.saturating_add(1);
            scene.entered = true;
            for a in scene.actors.iter_all_mut() {
                a.cued = false;
            }
            scene.queue.clear();
        }

        self.queue.clear();

        if let Some(rec) = self.pending_transition.take() {
            self.apply_transition(rec);
        }
    }

    /// Drain Mealy effects accumulated by the most recent transition. Called by
    /// World::tick at the start of pass 2 each tick so Mealy outputs are
    /// applied alongside fresh op effects (one-tick latency from the transition).
    pub fn drain_pending_mealy(&mut self, sink: &mut Vec<Effect>) {
        sink.append(&mut self.pending_mealy);
    }

    fn apply_transition(&mut self, rec: TransitionRecord) {
        let TransitionRecord { source, target, mealy } = rec;
        let Some(src_h) = self.handle_for(source) else { return };
        let Some(tgt_h) = self.handle_for(target) else { return };

        // Reuse scratch buffers for ancestor chains — no heap allocation.
        // ancestors_into_fields only reads scenes + by_id; the mutable borrows
        // on ancestor_scratch / transition_scratch are different fields, so
        // Rust's field-level split-borrow allows this without unsafe.
        ancestors_into_fields(&self.scenes, &self.by_id, src_h, &mut self.ancestor_scratch);
        ancestors_into_fields(&self.scenes, &self.by_id, tgt_h, &mut self.transition_scratch);

        let src_len = self.ancestor_scratch.len();
        let tgt_len = self.transition_scratch.len();
        let mut lca_idx = 0usize;
        {
            let max = src_len.min(tgt_len);
            while lca_idx < max
                && self.ancestor_scratch[lca_idx] == self.transition_scratch[lca_idx]
            {
                lca_idx += 1;
            }
        }

        // Exit src → LCA (excluding LCA), leaf-first — no exit_chain Vec needed.
        for i in (lca_idx..src_len).rev() {
            let h = self.ancestor_scratch[i];
            let scene_id  = self.scenes[h].id;
            let parent_id = self.scenes[h].parent;
            if let Some(pid) = parent_id {
                self.history.retain(|(p, _)| *p != pid);
                self.history.push((pid, scene_id));
            }
            let scene = &mut self.scenes[h];
            scene._rendered = true;
            scene.root.reset();
            scene.bt_state = BtState::default();
            scene.entered  = false;
            scene.elapsed  = 0.0;
            scene.tick_count = 0;
            self.queue.push(Event::SceneExited(scene_id));
        }

        // Drop active leaves descended from src_h. Inline ancestor walk so we
        // avoid allocating a `dropped` Vec; field-level borrows keep this safe.
        let mut i = 0;
        while i < self.active_leaves.len() {
            let leaf = self.active_leaves[i];
            let is_under_src = {
                let scenes = &self.scenes;
                let by_id  = &self.by_id;
                let mut cur = leaf;
                let mut found = false;
                loop {
                    if cur == src_h { found = true; break; }
                    match scenes[cur].parent.and_then(|pid| {
                        by_id.iter().find(|(k, _)| *k == pid).map(|(_, h)| *h)
                    }) {
                        Some(ph) => cur = ph,
                        None     => break,
                    }
                }
                found
            };
            if is_under_src {
                self.active_leaves.swap_remove(i);
            } else {
                i += 1;
            }
        }

        // Stash Mealy effects for next tick's pass-2 drain — Arc slice iter,
        // cloning each Effect (same element cost as before, outer allocation gone).
        self.pending_mealy.extend(mealy.iter().cloned());

        // Enter LCA → target chain root-first — no enter_chain Vec needed.
        let enter_start     = lca_idx;
        let enter_end       = tgt_len;
        let pre_descent_len = self.active_leaves.len();

        if enter_end > enter_start {
            for i in enter_start..enter_end {
                let h  = self.transition_scratch[i];
                let id = self.scenes[h].id;
                self.queue.push(Event::SceneEntered(id));
            }
            let final_h = self.transition_scratch[enter_end - 1];
            self.descend_to_leaves(final_h);
        } else {
            // LCA == src == tgt (self-loop) — re-descend from src's parent.
            if let Some(pid) = self.scenes[src_h].parent
                && let Some(ph) = self.handle_for(pid)
            {
                self.descend_to_leaves(ph);
            }
        }

        // Fire SceneEntered for leaves added by descent beyond the explicit
        // enter chain — use active_leaves[pre_descent_len..] directly,
        // no new_leaves Vec needed.
        let new_end = self.active_leaves.len();
        for i in pre_descent_len..new_end {
            let leaf = self.active_leaves[i];
            let id   = self.scenes[leaf].id;
            let mut already_fired = false;
            for j in enter_start..enter_end {
                if self.scenes[self.transition_scratch[j]].id == id {
                    already_fired = true;
                    break;
                }
            }
            if !already_fired {
                self.queue.push(Event::SceneEntered(id));
            }
        }
    }
}

// ── Allocation-free HSM helpers ─────────────────────────────────────────────
//
// Free functions so callers can hold `&mut` borrows on other Play fields
// (scratch buffers, active_leaves, history, …) simultaneously — Rust's
// field-level split-borrow only works across distinct fields, not across
// `&self` / `&mut self` method boundaries.

/// Write the ancestor chain [root → … → leaf] into `out` (cleared first).
fn ancestors_into_fields(
    scenes: &Arena<SceneTag, Scene>,
    by_id:  &ThinVec<(SceneId, SceneHandle)>,
    leaf:   SceneHandle,
    out:    &mut Vec<SceneHandle>,
) {
    out.clear();
    let mut cur = leaf;
    loop {
        out.push(cur);
        match scenes[cur].parent {
            None => break,
            Some(pid) => {
                match by_id.iter().find(|(k, _)| *k == pid).map(|(_, h)| *h) {
                    Some(ph) => cur = ph,
                    None     => break,
                }
            }
        }
    }
    out.reverse();
}

/// Build the complete active-configuration set into `out` (cleared first),
/// using `scratch` as a per-leaf ancestor work buffer.
fn active_configuration_into(
    scenes:        &Arena<SceneTag, Scene>,
    by_id:         &ThinVec<(SceneId, SceneHandle)>,
    active_leaves: &ThinVec<SceneHandle>,
    out:           &mut Vec<SceneHandle>,
    scratch:       &mut Vec<SceneHandle>,
) {
    out.clear();
    for &leaf in active_leaves.iter() {
        ancestors_into_fields(scenes, by_id, leaf, scratch);
        for &h in scratch.iter() {
            if !out.contains(&h) { out.push(h); }
        }
    }
}

// ── Static-troupe analysis ──────────────────────────────────────────────────
//
// At instantiate time we walk every authored Effect (BT leaves, on_enter,
// on_exit, transition.effects, recursively in Mealy chains) and bucket every
// `Effect::CueTroupe` by its `delta`. Troupes that only ever receive identity
// cues — or never appear at all — are returned as "static". Stage's
// cue_troupe_direct fast-paths these to a no-op without even filling
// cue_scratch.

fn compute_static_troupes(script: &Script) -> ThinVec<TroupeId> {
    // Engine pattern: flat ThinVec with linear-scan dedup, matching how
    // Stage::cache and Level::cache are stored. Troupe counts are tiny (<<100),
    // so .contains() beats HashSet hashing here.
    let mut all_troupes: ThinVec<TroupeId> = ThinVec::new();
    let mut moving:      ThinVec<TroupeId> = ThinVec::new();

    for def in script.scenes.iter() {
        for t in def.troupes.iter() {
            if !all_troupes.contains(t) { all_troupes.push(*t); }
        }
        scan_effects(&def.on_enter, &mut all_troupes, &mut moving);
        scan_effects(&def.on_exit,  &mut all_troupes, &mut moving);
        for tr in def.transitions.iter() {
            scan_effects(&tr.effects, &mut all_troupes, &mut moving);
        }
        scan_bt(&def.root, &mut all_troupes, &mut moving);
    }

    // static = all_troupes \ moving.
    all_troupes.into_iter().filter(|t| !moving.contains(t)).collect()
}

fn scan_effects(
    effects: &[Effect],
    all_troupes: &mut ThinVec<TroupeId>,
    moving:      &mut ThinVec<TroupeId>,
) {
    for e in effects {
        match e {
            Effect::CueTroupe { troupe, delta, .. } => {
                if !all_troupes.contains(troupe) { all_troupes.push(*troupe); }
                if *delta != Affine3A::IDENTITY && !moving.contains(troupe) {
                    moving.push(*troupe);
                }
            }
            Effect::ScheduleTransition { mealy, .. } => scan_effects(mealy, all_troupes, moving),
            _ => {}
        }
    }
}

fn scan_bt(
    node: &BtNode,
    all_troupes: &mut ThinVec<TroupeId>,
    moving:      &mut ThinVec<TroupeId>,
) {
    match node {
        BtNode::Sequence(cs) | BtNode::Selector(cs) => {
            for c in cs { scan_bt(c, all_troupes, moving) }
        }
        BtNode::Parallel { children, .. } => {
            for c in children { scan_bt(c, all_troupes, moving) }
        }
        BtNode::Repeat { child, .. } => scan_bt(child, all_troupes, moving),
        BtNode::Decorator { child, .. } => scan_bt(child, all_troupes, moving),
        BtNode::Leaf(op) => {
            if let Effect::CueTroupe { troupe, delta, .. } = &*op.effect {
                if !all_troupes.contains(troupe) { all_troupes.push(*troupe); }
                if *delta != Affine3A::IDENTITY && !moving.contains(troupe) {
                    moving.push(*troupe);
                }
            }
        }
    }
}
