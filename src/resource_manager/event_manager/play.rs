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
    pub mealy:  ThinVec<Effect>,
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
        for &leaf in self.active_leaves.iter() {
            for h in self.ancestors(leaf) {
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

    /// Compute the ancestor chain (root → ... → leaf) for a given scene.
    fn ancestors(&self, leaf: SceneHandle) -> Vec<SceneHandle> {
        let mut chain = Vec::new();
        let mut cur = leaf;
        loop {
            chain.push(cur);
            let parent = self.scenes[cur].parent;
            match parent {
                None => break,
                Some(pid) => match self.handle_for(pid) {
                    Some(ph) => cur = ph,
                    None => break,
                },
            }
        }
        chain.reverse();
        chain
    }

    /// Pass 1 — read-only — walk the active configuration, tick BTs, dispatch
    /// handlers, evaluate transitions.
    pub fn collect_effects(&self, _dt: f32, world: &World, out: &mut Vec<Effect>) {
        if self.paused || self.finished { return; }

        let mut play_queue_dispatched = false;

        for &leaf in self.active_leaves.iter() {
            let chain = self.ancestors(leaf);

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

                // 4. Transition evaluation — first match wins.
                for t in scene.transitions.iter() {
                    if t.condition.eval(&ctx) {
                        out.push(Effect::ScheduleTransition {
                            level_h: self.level_h,
                            stage_h: self.stage_h,
                            source:  scene.id,
                            target:  t.target,
                            mealy:   t.effects.clone(),
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

        let active = self.active_configuration();

        for &h in active.iter() {
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

        let src_chain = self.ancestors(src_h); // root → src
        let tgt_chain = self.ancestors(tgt_h); // root → tgt
        let mut lca_idx = 0usize;
        let max = src_chain.len().min(tgt_chain.len());
        while lca_idx < max && src_chain[lca_idx] == tgt_chain[lca_idx] {
            lca_idx += 1;
        }

        // Exit src → LCA (excluding LCA), leaf-first.
        let exit_chain: Vec<SceneHandle> = src_chain[lca_idx..].iter().rev().copied().collect();
        for h in exit_chain.iter() {
            let parent_id = self.scenes[*h].parent;
            let scene_id  = self.scenes[*h].id;
            if let Some(pid) = parent_id {
                self.history.retain(|(p, _)| *p != pid);
                self.history.push((pid, scene_id));
            }
            let scene = &mut self.scenes[*h];
            scene._rendered = true;
            scene.root.reset();
            scene.bt_state = BtState::default();
            scene.entered = false;
            scene.elapsed = 0.0;
            scene.tick_count = 0;
            self.queue.push(Event::SceneExited(scene_id));
        }

        // Drop active leaves descended from src.
        let dropped: Vec<SceneHandle> = self.active_leaves
            .iter()
            .copied()
            .filter(|&l| self.ancestors(l).contains(&src_h))
            .collect();
        self.active_leaves.retain(|l| !dropped.contains(l));

        // Stash Mealy effects for next tick's effect drain.
        self.pending_mealy.extend(mealy.into_iter());

        // Enter LCA → target chain, root-first.
        let enter_chain: Vec<SceneHandle> = tgt_chain[lca_idx..].iter().copied().collect();
        let pre_descent_len = self.active_leaves.len();

        if let Some(&final_h) = enter_chain.last() {
            for &h in enter_chain.iter() {
                let id = self.scenes[h].id;
                self.queue.push(Event::SceneEntered(id));
            }
            self.descend_to_leaves(final_h);
        } else {
            // LCA == src == tgt (self-loop); re-descend from LCA's stored ancestor.
            // For v1 simplicity, descend from src's parent if any.
            if let Some(pid) = self.scenes[src_h].parent {
                if let Some(ph) = self.handle_for(pid) {
                    self.descend_to_leaves(ph);
                }
            }
        }

        // Fire SceneEntered for any leaves added by descent beyond the
        // explicit enter_chain.
        let new_leaves: Vec<SceneHandle> = self.active_leaves[pre_descent_len..].to_vec();
        for &leaf in new_leaves.iter() {
            let id = self.scenes[leaf].id;
            if !enter_chain.iter().any(|&h| self.scenes[h].id == id) {
                self.queue.push(Event::SceneEntered(id));
            }
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
