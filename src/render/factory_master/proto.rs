use std::marker::PhantomData;
use std::sync::Arc;
use thin_vec::ThinVec;

use crate::forge_master::{FramePlan, GraphicsFramePlan};
use crate::resource_manager::manager::Id;

pub struct ProtoMarker;
pub type ProtoId = Id<ProtoMarker>;

// ── Kind tags — same pattern as LevelTag / StageTag in manager.rs ──────────
//
// A ZST marker lives in PhantomData<fn() -> Tag> on Proto<Tag>. The type
// parameter is the only difference between a compute and a graphics proto;
// the impl blocks below expose push_plan / push_call exclusively on the
// correct specialisation, so wrong-kind calls are a compile error.

pub struct ComputeTag;
pub struct GraphicsTag;

pub type ComputeProto  = Proto<ComputeTag>;
pub type GraphicsProto = Proto<GraphicsTag>;

// ── Proto<Tag> ──────────────────────────────────────────────────────────────

pub struct Proto<Tag> {
    pub id:   ProtoId,
    pub name: Arc<str>,
    // Both vecs always present; only one is populated depending on Tag.
    // ThinVec<T> is a single null pointer when empty, so the unused field is
    // free — same trade-off as Option<Box<T>> in the arena slots.
    pub(crate) plans: ThinVec<FramePlan>,
    pub(crate) calls: ThinVec<GraphicsFramePlan>,
    _kind: PhantomData<fn() -> Tag>,
}

// Shared accessors available on any Proto<Tag>.
impl<Tag> Proto<Tag> {
    pub fn is_empty(&self) -> bool {
        self.plans.is_empty() && self.calls.is_empty()
    }

    pub fn len(&self) -> usize {
        self.plans.len() + self.calls.len()
    }
}

// ── Compute ─────────────────────────────────────────────────────────────────

impl Proto<ComputeTag> {
    pub fn new(id: ProtoId, name: impl Into<Arc<str>>) -> Self {
        Self {
            id,
            name: name.into(),
            plans: ThinVec::new(),
            calls: ThinVec::new(),
            _kind: PhantomData,
        }
    }

    pub fn push_plan(&mut self, plan: FramePlan) {
        self.plans.push(plan);
    }
}

// ── Graphics ────────────────────────────────────────────────────────────────

impl Proto<GraphicsTag> {
    pub fn new(id: ProtoId, name: impl Into<Arc<str>>) -> Self {
        Self {
            id,
            name: name.into(),
            plans: ThinVec::new(),
            calls: ThinVec::new(),
            _kind: PhantomData,
        }
    }

    pub fn push_call(&mut self, call: GraphicsFramePlan) {
        self.calls.push(call);
    }
}
