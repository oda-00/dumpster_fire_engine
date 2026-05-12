use std::sync::Arc;
use thin_vec::ThinVec;

use crate::forge_master::FramePlan;
use crate::resource_manager::manager::Id;

pub struct ProtoMarker;
pub type ProtoId = Id<ProtoMarker>;

// A Proto is a recipe for a Factory: a named set of FramePlans whose Ores
// have not yet been refined. FactoryMaster turns a Proto into a Factory by
// driving every plan through ForgeMaster.
pub struct Proto {
    pub id: ProtoId,
    pub name: Arc<str>,
    pub plans: ThinVec<FramePlan>,
}

impl Proto {
    pub fn new(id: ProtoId, name: impl Into<Arc<str>>) -> Self {
        Self {
            id,
            name: name.into(),
            plans: ThinVec::new(),
        }
    }

    pub fn push(&mut self, plan: FramePlan) {
        self.plans.push(plan);
    }
}
