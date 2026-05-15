use std::sync::Arc;
use thin_vec::ThinVec;

use crate::forge_master::{
    Frame, FrameHandle, FrameId, FrameTag, ForgeMaster, ForgeResult, GraphicsFrame,
};
use crate::resource_manager::manager::{Arena, Handle, Id};

use super::proto::{ComputeTag, GraphicsTag, Proto};

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct FactoryTag;
pub type FactoryHandle = Handle<FactoryTag>;

pub struct FactoryMarker;
pub type FactoryId = Id<FactoryMarker>;

// Stable FrameId → live FrameHandle. Frames per factory stay small (one per
// render pass in the proto), so a flat ThinVec wins over a HashMap — same
// rationale as FactoryCacheEntry on FactoryMaster.
#[derive(Clone, Copy)]
pub struct FrameCacheEntry {
    pub id: FrameId,
    pub handle: FrameHandle,
}

// A Factory is a refined Proto: it owns the Frames (sets of forge Ingots)
// that a window's render passes will consume, plus a flat list of
// GraphicsFrame draw calls bound to GraphicsForges. A factory holds one
// flavor or the other depending on the source Proto; the unused side stays
// empty (cheap — `ThinVec::new()` is a single null pointer).
pub struct Factory {
    pub id: FactoryId,
    pub name: Arc<str>,
    frames: Arena<FrameTag, Frame>,
    cache: ThinVec<FrameCacheEntry>,
    graphics_calls: ThinVec<GraphicsFrame>,
}

impl Factory {
    pub fn new(id: FactoryId, name: impl Into<Arc<str>>) -> Self {
        Self {
            id,
            name: name.into(),
            frames: Arena::new(),
            cache: ThinVec::new(),
            graphics_calls: ThinVec::new(),
        }
    }

    /// Drive every compute plan through ForgeMaster (GPU dispatch + readback).
    pub fn from_compute_proto(
        id: FactoryId,
        proto: Proto<ComputeTag>,
        forge: &mut ForgeMaster,
    ) -> ForgeResult<Self> {
        let mut factory = Factory::new(id, proto.name);
        factory.frames = Arena::with_capacity(proto.plans.len());
        factory.cache.reserve(proto.plans.len());
        for plan in proto.plans {
            let frame = plan.refine(forge)?;
            factory.insert_frame(frame);
        }
        Ok(factory)
    }

    /// Convert graphics draw calls — no GPU dispatch, pure type flip.
    pub fn from_graphics_proto(id: FactoryId, proto: Proto<GraphicsTag>) -> Self {
        let mut factory = Factory::new(id, proto.name);
        factory.graphics_calls.reserve(proto.calls.len());
        for call in proto.calls {
            factory.graphics_calls.push(call.refine());
        }
        factory
    }

    pub fn graphics_calls(&self) -> &[GraphicsFrame] {
        &self.graphics_calls
    }

    pub fn push_graphics_call(&mut self, call: GraphicsFrame) {
        self.graphics_calls.push(call);
    }

    pub fn insert_frame(&mut self, frame: Frame) -> FrameHandle {
        let id = frame.id;
        let handle = self.frames.insert(frame);
        self.cache.push(FrameCacheEntry { id, handle });
        handle
    }

    pub fn frame(&self, handle: FrameHandle) -> Option<&Frame> {
        self.frames.get(handle)
    }

    pub fn frame_mut(&mut self, handle: FrameHandle) -> Option<&mut Frame> {
        self.frames.get_mut(handle)
    }

    pub fn frame_by_id(&self, id: FrameId) -> Option<&Frame> {
        self.handle_of(id).and_then(|h| self.frames.get(h))
    }

    pub fn handle_of(&self, id: FrameId) -> Option<FrameHandle> {
        self.cache.iter().find(|e| e.id == id).map(|e| e.handle)
    }

    pub fn frames(&self) -> impl Iterator<Item = &Frame> {
        self.frames.values()
    }

    pub fn frames_mut(&mut self) -> impl Iterator<Item = &mut Frame> {
        self.frames.values_mut()
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.len() == 0
    }

    pub fn remove_frame(&mut self, handle: FrameHandle, device: &ash::Device) -> Option<Frame> {
        let mut frame = self.frames.remove(handle)?;
        unsafe { frame.destroy(device) };
        self.cache.retain(|e| e.handle != handle);
        Some(frame)
    }

    pub unsafe fn destroy(&mut self, device: &ash::Device) {
        for frame in self.frames.values_mut() {
            unsafe { frame.destroy(device) };
        }
        self.cache.clear();
        // Drain graphics calls and free any GpuMesh allocations. The factory
        // is the primary owner; Arc::try_unwrap succeeds unless someone leaked
        // a clone (in which case the Vulkan memory leaks — document this).
        for mut call in self.graphics_calls.drain(..) {
            if let Some(arc) = call.mesh.take() {
                if let Ok(mut mesh) = Arc::try_unwrap(arc) {
                    unsafe { mesh.destroy(device) };
                }
            }
        }
    }
}
