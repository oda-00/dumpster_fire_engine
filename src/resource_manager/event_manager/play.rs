use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use glam::Affine3A;
use thin_vec::ThinVec;
use crate::resource_manager::manager::{Arena, Handle, Id, LevelHandle, StageHandle};
use crate::resource_manager::world_manager::stage::StageId;
use crate::resource_manager::world_manager::world::World;
use super::scene::{
    BtNode, BtState, EvalCtx, Effect, Event, EventMatcher, Handler, Scene, SceneHandle,
    SceneId, SceneKind, SceneTag, TroupeId,
};
use super::script::{Script, ScriptId};

/// Stack-array inline cap for HSM region-target collection in
/// `descend_to_leaves`. Trees with more concurrent regions than this fall
/// through to a heap path (deliberately not exposed — the limit is a
/// performance hint, not a correctness ceiling).
const REGION_TARGETS_INLINE: usize = 8;

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

// ── PlayStats — workload shape counters ────────────────────────────────────

#[derive(Default)]
pub struct PlayStats {
    /// Total parent_handle steps walked in chain-build loops.
    pub chain_steps:       AtomicU64,
    /// Scenes that passed the dedup check and were fully processed.
    pub scenes_processed:  AtomicU64,
    /// Scenes hit by the early-break dedup (already visited this tick).
    pub dedup_skips:       AtomicU64,
    /// `apply_transition` calls.
    pub transitions_fired: AtomicU64,
    /// `BtNode::tick` invocations on Atomic scenes.
    pub bt_ticks:          AtomicU64,
}

#[derive(Debug, Clone, Copy)]
pub struct PlayStatsSnapshot {
    pub chain_steps:       u64,
    pub scenes_processed:  u64,
    pub dedup_skips:       u64,
    pub transitions_fired: u64,
    pub bt_ticks:          u64,
}

impl PlayStats {
    #[inline(always)]
    pub fn snapshot(&self) -> PlayStatsSnapshot {
        PlayStatsSnapshot {
            chain_steps:       self.chain_steps.load(Ordering::Relaxed),
            scenes_processed:  self.scenes_processed.load(Ordering::Relaxed),
            dedup_skips:       self.dedup_skips.load(Ordering::Relaxed),
            transitions_fired: self.transitions_fired.load(Ordering::Relaxed),
            bt_ticks:          self.bt_ticks.load(Ordering::Relaxed),
        }
    }

    pub fn reset(&self) {
        self.chain_steps.store(0, Ordering::Relaxed);
        self.scenes_processed.store(0, Ordering::Relaxed);
        self.dedup_skips.store(0, Ordering::Relaxed);
        self.transitions_fired.store(0, Ordering::Relaxed);
        self.bt_ticks.store(0, Ordering::Relaxed);
    }
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

    /// Resolution table: SceneId → SceneHandle as a range-compressed direct-index
    /// array. `id_base` is the minimum authored SceneId; `id_lookup[id - id_base]`
    /// is the handle (or None for gaps). O(1) lookup, no branches, no binary
    /// search, no iterator state. Built once at instantiate.
    pub id_base:   i64,
    pub id_lookup: ThinVec<Option<SceneHandle>>,

    /// Hot parallel arrays — dense, indexed by `SceneHandle::idx as usize`.
    /// Pulled out of `Scene` so the per-tick chain-build + dedup loop touches
    /// only ~16 bytes per scene (one Option<SceneHandle> + one AtomicU64)
    /// instead of the full ~200-byte cold Scene struct. At depth ≥ 64 (where
    /// the Scene array spills L1) the working set drops by an order of
    /// magnitude. Set once at instantiate; `parents` is immutable thereafter,
    /// `tick_marks` is the dedup generation field formerly known as
    /// `Scene::last_processed_tick`.
    pub parents:    ThinVec<Option<SceneHandle>>,
    pub tick_marks: ThinVec<AtomicU64>,

    pub handlers: ThinVec<Handler>,
    pub queue:    ThinVec<Event>,

    /// Troupes whose authored cues are entirely IDENTITY (or which never appear
    /// as a CueTroupe target at all). Computed at instantiate time. Stage's
    /// cue_troupe_direct uses this to skip the scratch fill + dirty walk for
    /// cues that won't move anything.
    pub static_troupes: ThinVec<crate::resource_manager::event_manager::TroupeId>,

    /// Filled by World::apply_effect when an Effect::ScheduleTransition fires;
    /// drained by post_tick_bookkeeping during pass 3 by `mem::swap` into
    /// `pending_drain_scratch` (so the apply loop holds &mut self without
    /// borrowing `pending_transitions` itself). Per-tick dedup in
    /// collect_effects guarantees at most one record per source scene.
    pub pending_transitions: ThinVec<TransitionRecord>,

    /// Mealy outputs accumulated when a transition is applied; drained by
    /// World::tick at the start of the *next* tick's pass 2 so the one-tick
    /// latency rule holds.
    pub pending_mealy: Vec<Effect>,

    pub paused:   bool,
    pub finished: bool,

    /// True iff the script has at least one `EventMatcher::Tick` handler
    /// (play-global or scene-local). Computed once at instantiate so
    /// `World::tick`'s pass-0 can skip the per-stage `Event::Tick { dt }`
    /// push when nothing would consume it.
    pub wants_tick: bool,

    /// Monotonic per-Play tick id. `collect_effects` does
    /// `tick_counter.fetch_add(1, Relaxed) + 1` at entry; the result is
    /// stored into each visited `Scene::last_processed_tick`. Comparing the
    /// scene's stored value against the current tick id gives O(1) dedup
    /// for shared ancestors with no scratch buffer.
    tick_counter: AtomicU64,

    /// Cheap workload-shape counters. Bumped via Relaxed atomics from
    /// `collect_effects` (&self) and as plain reads from `apply_transition`
    /// (&mut self). On x86_64 a Relaxed AtomicU64::fetch_add is a plain ADD;
    /// the hot path cost is at noise level. Read via `stats_snapshot()` to
    /// understand workload shape (chain depth, dedup hit rate, transition
    /// frequency) before further structural changes.
    pub stats: PlayStats,

    // ── Per-Play scratch buffers ──────────────────────────────────────────
    //
    // Pre-allocated buffers reused every tick to eliminate the transient Vec
    // allocations that appear in apply_transition and active_configuration.
    // Capacity grows to the watermark of the deepest HSM seen at runtime and
    // is never shrunk, so steady-state operation is allocation-free.
    //
    // ancestor_scratch:        source ancestor chain (root → src) in apply_transition
    // transition_scratch:      target ancestor chain (root → tgt) in apply_transition;
    //                          also repurposed as the "leaves to keep" buffer after
    //                          the tgt chain is no longer needed.
    // config_scratch:          active configuration set in post_tick_bookkeeping
    // pending_drain_scratch:   swap-target for pending_transitions during drain;
    //                          allocation persists across ticks.
    ancestor_scratch:       Vec<SceneHandle>,
    transition_scratch:     Vec<SceneHandle>,
    config_scratch:         Vec<SceneHandle>,
    pending_drain_scratch:  ThinVec<TransitionRecord>,
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

        // Build a range-compressed direct-index lookup. SceneIds in a script are
        // author-assigned i64s; in practice they cluster (e.g. 1..N or 100..104).
        // Allocate `max - min + 1` slots, index by `(id - id_base)`.
        let (id_base, id_span) = if script.scenes.is_empty() {
            (0i64, 0usize)
        } else {
            let mut min_id: i64 = i64::MAX;
            let mut max_id: i64 = i64::MIN;
            for def in script.scenes.iter() {
                let r = def.id.raw();
                if r < min_id { min_id = r; }
                if r > max_id { max_id = r; }
            }
            (min_id, (max_id - min_id + 1) as usize)
        };

        let mut id_lookup: ThinVec<Option<SceneHandle>> = ThinVec::with_capacity(id_span);
        id_lookup.resize(id_span, None);

        for def in script.scenes.iter() {
            let h = scenes.insert(Scene::from_def(def));
            id_lookup[(def.id.raw() - id_base) as usize] = Some(h);
        }

        // Second pass: build the hot parallel arrays. Scenes are never removed
        // from the Arena, so handle indices are dense 0..N and `idx as usize`
        // is a valid array index. `parents[h.idx]` resolves SceneDef::parent
        // to a SceneHandle once; the per-tick chain walk reads it directly.
        let scene_count = script.scenes.len();
        let mut parents:    ThinVec<Option<SceneHandle>> = ThinVec::with_capacity(scene_count);
        let mut tick_marks: ThinVec<AtomicU64>           = ThinVec::with_capacity(scene_count);
        parents.resize(scene_count, None);
        for _ in 0..scene_count { tick_marks.push(AtomicU64::new(0)); }
        for slot in id_lookup.iter() {
            if let Some(h) = *slot {
                let ph = scenes[h].parent.and_then(|pid| {
                    let pidx = (pid.raw() - id_base) as usize;
                    id_lookup.get(pidx).copied().flatten()
                });
                parents[h.idx as usize] = ph;
            }
        }

        let root_idx = (script.entry.raw() - id_base) as usize;
        let root_h = id_lookup
            .get(root_idx)
            .copied()
            .flatten()
            .expect("Script::entry must reference a scene that was added via add_scene");

        // Static-troupe analysis: collect every troupe id that appears in the
        // script (either declared by a SceneDef or named by a CueTroupe effect),
        // then subtract the troupes that receive at least one non-identity cue.
        // What's left is the set of "static" troupes whose cues are guaranteed
        // not to move any actor — Stage::cue_troupe_direct can skip them.
        let static_troupes = compute_static_troupes(script);

        // Precompute whether anything actually consumes Event::Tick — World::tick
        // skips the per-tick push if not.
        let wants_tick = script.handlers.iter().any(|h| matches!(h.matcher, EventMatcher::Tick))
            || script.scenes.iter().any(|d| {
                d.handlers.iter().any(|h| matches!(h.matcher, EventMatcher::Tick))
            });

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
            id_base,
            id_lookup,
            parents,
            tick_marks,
            handlers: script.handlers.iter().cloned().collect(),
            queue: ThinVec::new(),
            static_troupes,
            pending_transitions: ThinVec::new(),
            pending_mealy: Vec::new(),
            paused: false,
            finished: false,
            wants_tick,
            tick_counter: AtomicU64::new(0),
            stats: PlayStats::default(),
            ancestor_scratch:      Vec::with_capacity(16),
            transition_scratch:    Vec::with_capacity(16),
            config_scratch:        Vec::with_capacity(16),
            pending_drain_scratch: ThinVec::with_capacity(4),
        };

        play.descend_to_leaves(root_h);
        // Fire SceneEntered for every newly-active scene in the configuration.
        for h in play.active_configuration() {
            let id = play.scenes[h].id;
            play.queue.push(Event::SceneEntered(id));
        }
        play
    }

    /// O(1) range-compressed direct-index lookup. Built at instantiate.
    #[inline(always)]
    pub fn handle_for(&self, id: SceneId) -> Option<SceneHandle> {
        let idx = id.raw().wrapping_sub(self.id_base) as usize;
        self.id_lookup.get(idx).copied().flatten()
    }

    /// All scenes currently active = each leaf plus its ancestor chain (deduped).
    pub fn active_configuration(&self) -> Vec<SceneHandle> {
        let mut out: Vec<SceneHandle> = Vec::new();
        let mut scratch = Vec::with_capacity(8);
        for &leaf in self.active_leaves.iter() {
            ancestors_into_fields(&self.parents, leaf, &mut scratch);
            for &h in scratch.iter() {
                if !out.contains(&h) { out.push(h); }
            }
        }
        out
    }

    /// Walk from `start` through Compound::initial / AndParallel regions,
    /// pushing every reached leaf into active_leaves.
    ///
    /// Snapshots the per-kind targets into a stack array (no SmallVec, no
    /// `regions.clone()` — SceneId is `Id<T>(i64)`, Copy + no-Drop, so the
    /// array can be initialized with a sentinel placeholder cheaply). Trees
    /// with more than `REGION_TARGETS_INLINE` concurrent regions fall through
    /// to a heap `ThinVec` path (extremely unusual for authored HSMs).
    fn descend_to_leaves(&mut self, start: SceneHandle) {
        let mut targets_inline: [SceneId; REGION_TARGETS_INLINE] =
            [SceneId::new(0); REGION_TARGETS_INLINE];
        let mut targets_overflow: ThinVec<SceneId> = ThinVec::new();
        let mut targets_len: usize = 0;
        let is_atomic;
        {
            let scene = &self.scenes[start];
            match &scene.kind {
                SceneKind::Atomic => {
                    is_atomic = true;
                }
                SceneKind::Compound { initial, history, .. } => {
                    is_atomic = false;
                    let chosen = self.history_for(scene.id)
                        .or(*history)
                        .unwrap_or(*initial);
                    targets_inline[0] = chosen;
                    targets_len = 1;
                }
                SceneKind::AndParallel { regions } => {
                    is_atomic = false;
                    for region in regions.iter() {
                        let t = region.history.unwrap_or(region.initial);
                        if targets_len < REGION_TARGETS_INLINE {
                            targets_inline[targets_len] = t;
                            targets_len += 1;
                        } else {
                            targets_overflow.push(t);
                        }
                    }
                }
            }
        }

        if is_atomic {
            self.active_leaves.push(start);
            return;
        }

        // For Compound: a single target. If it doesn't resolve, treat `start`
        // as a degenerate leaf (matches old behavior).
        if targets_len == 1 && targets_overflow.is_empty() {
            if let Some(child_h) = self.handle_for(targets_inline[0]) {
                self.descend_to_leaves(child_h);
            } else {
                self.active_leaves.push(start);
            }
            return;
        }

        // For AndParallel: every region descends; missing targets are skipped.
        for &t in targets_inline.iter().take(targets_len) {
            if let Some(child_h) = self.handle_for(t) {
                self.descend_to_leaves(child_h);
            }
        }
        for &t in targets_overflow.iter() {
            if let Some(child_h) = self.handle_for(t) {
                self.descend_to_leaves(child_h);
            }
        }
    }

    fn history_for(&self, parent: SceneId) -> Option<SceneId> {
        self.history.iter().find(|(p, _)| *p == parent).map(|(_, c)| *c)
    }


    /// Pass 1 — read-only — walk the active configuration, tick BTs, dispatch
    /// handlers, evaluate transitions.
    ///
    /// `chain` is a `&mut Vec<SceneHandle>` scratch buffer threaded down from
    /// `World::tick` (lifted out via `mem::take` so the allocation persists
    /// across ticks). We clear-and-reuse it per active leaf — zero heap
    /// allocation in steady state, no SmallVec inline-vs-heap branch on push.
    ///
    /// Per-tick scene-visit dedupe is via `Scene::last_processed_tick`
    /// (AtomicU64) compared against `Play::tick_counter` — same pattern as
    /// `SceneOperation::fired` and `BtNode::Repeat::current`. The dedup
    /// **breaks** the ancestor walk on first hit (every farther ancestor was
    /// already processed by a prior leaf), strictly faster than `continue`.
    pub fn collect_effects(
        &self,
        _dt:   f32,
        world: &World,
        out:   &mut Vec<Effect>,
        chain: &mut Vec<SceneHandle>,
    ) {
        if self.paused || self.finished { return; }

        // ── Play-global handler dispatch (once per tick, anchored at root) ──
        // Skip entirely when nothing would consume an event — recovers the
        // overhead of the mandatory dispatch path the correctness fix added.
        if !self.handlers.is_empty() && !self.queue.is_empty() {
            let root_scene = &self.scenes[self.root];
            let play_ctx = EvalCtx {
                world,
                level_h:     self.level_h,
                stage_h:     self.stage_h,
                scene_id:    root_scene.id,
                elapsed:     root_scene.elapsed,
                tick_count:  root_scene.tick_count,
                events_seen: &root_scene.queue,
                actors:      &root_scene.actors,
                troupes:     &root_scene.troupes,
            };
            for ev in self.queue.iter() {
                for h in self.handlers.iter() {
                    if h.matcher.matches(ev) {
                        (h.action)(ev, &play_ctx, out);
                    }
                }
            }
        }

        // Bump the per-Play tick id; every visited Scene tags itself with this
        // value so a shared ancestor reached via a second leaf is detected in
        // O(1) without any external dedup buffer.
        let tick_id = self.tick_counter.fetch_add(1, Ordering::Relaxed) + 1;

        // Local stat accumulators: avoid `lock xadd` per inner-loop step.
        // Folded to the atomic fields once at function exit.
        let mut local_chain_steps:      u64 = 0;
        let mut local_scenes_processed: u64 = 0;
        let mut local_dedup_skips:      u64 = 0;
        let mut local_bt_ticks:         u64 = 0;

        for &leaf in self.active_leaves.iter() {
            chain.clear();
            let mut cur = leaf;
            // Build the chain leaf-first by pushing as we walk up. No reverse:
            // leaf-first iteration order is exactly what we need for "innermost
            // transition wins" semantics, and saves one Vec::reverse per leaf.
            // Hot arrays: chain build touches only `parents` (≤ 16 bytes/entry).
            // The cold Scene struct is not loaded until after dedup passes.
            loop {
                chain.push(cur);
                local_chain_steps += 1;
                match self.parents[cur.idx as usize] {
                    Some(ph) => cur = ph,
                    None => break,
                }
            }

            for &handle in chain.iter() {
                // O(1) dedup against the dense AtomicU64 array — one cache line
                // holds 8 entries. Early break: every farther ancestor was
                // reached by a prior leaf.
                let mark = &self.tick_marks[handle.idx as usize];
                if mark.load(Ordering::Relaxed) == tick_id {
                    local_dedup_skips += 1;
                    break;
                }
                mark.store(tick_id, Ordering::Relaxed);
                local_scenes_processed += 1;

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
                    local_bt_ticks += 1;
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

                // 3. Transition evaluation — first match wins. Arc::clone is a
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
        }

        // Fold local accumulators into the atomic fields once. Single Relaxed
        // fetch_add per counter per call — total cost is fixed, not per-step.
        if local_chain_steps      != 0 { self.stats.chain_steps.fetch_add(local_chain_steps, Ordering::Relaxed); }
        if local_scenes_processed != 0 { self.stats.scenes_processed.fetch_add(local_scenes_processed, Ordering::Relaxed); }
        if local_dedup_skips      != 0 { self.stats.dedup_skips.fetch_add(local_dedup_skips, Ordering::Relaxed); }
        if local_bt_ticks         != 0 { self.stats.bt_ticks.fetch_add(local_bt_ticks, Ordering::Relaxed); }
    }

    /// Pass 3 — mut — drain queues, advance counters, apply pending transition.
    pub fn post_tick_bookkeeping(&mut self, dt: f32) {
        if self.paused || self.finished { return; }

        // Build active config into config_scratch, reusing its allocation.
        active_configuration_into(
            &self.parents,
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

        // Drain pending transitions. The dedup added in collect_effects
        // (Scene::last_processed_tick) guarantees at most one record per
        // source scene per tick, so order is irrelevant — we LIFO via pop()
        // and avoid the SmallVec collect / Vec::drain dance entirely.
        // mem::swap into a persistent scratch field keeps the allocation
        // alive across ticks (mirrors the World::tick_effects mem::take pattern).
        if !self.pending_transitions.is_empty() {
            std::mem::swap(&mut self.pending_transitions, &mut self.pending_drain_scratch);
            while let Some(rec) = self.pending_drain_scratch.pop() {
                self.apply_transition(rec);
            }
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
        self.stats.transitions_fired.fetch_add(1, Ordering::Relaxed);

        // Reuse scratch buffers for ancestor chains — no heap allocation.
        // ancestors_into_fields only reads scenes + by_id; the mutable borrows
        // on ancestor_scratch / transition_scratch are different fields, so
        // Rust's field-level split-borrow allows this without unsafe.
        ancestors_into_fields(&self.parents, src_h, &mut self.ancestor_scratch);
        ancestors_into_fields(&self.parents, tgt_h, &mut self.transition_scratch);

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
                let parents = &self.parents;
                let mut cur = leaf;
                let mut found = false;
                loop {
                    if cur == src_h { found = true; break; }
                    match parents[cur.idx as usize] {
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
            if let Some(ph) = self.parents[src_h.idx as usize] {
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
/// Walks the dense `parents` array — single load per step, no Scene access.
fn ancestors_into_fields(
    parents: &[Option<SceneHandle>],
    leaf:    SceneHandle,
    out:     &mut Vec<SceneHandle>,
) {
    out.clear();
    let mut cur = leaf;
    loop {
        out.push(cur);
        match parents[cur.idx as usize] {
            Some(ph) => cur = ph,
            None     => break,
        }
    }
    out.reverse();
}

/// Build the complete active-configuration set into `out` (cleared first),
/// using `scratch` as a per-leaf ancestor work buffer.
fn active_configuration_into(
    parents:       &[Option<SceneHandle>],
    active_leaves: &ThinVec<SceneHandle>,
    out:           &mut Vec<SceneHandle>,
    scratch:       &mut Vec<SceneHandle>,
) {
    out.clear();
    for &leaf in active_leaves.iter() {
        ancestors_into_fields(parents, leaf, scratch);
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
