use thin_vec::ThinVec;

use crate::forge_master::{ForgeMaster, ForgeResult};
use crate::resource_manager::manager::Arena;

use super::factory::{Factory, FactoryHandle, FactoryId, FactoryTag};
use super::proto::{ComputeTag, GraphicsTag, Proto};

// Stable Id → live Handle. Kept as a small ThinVec because factory counts
// stay in the dozens per window; linear scan beats a HashMap for those sizes.
#[derive(Clone, Copy)]
pub struct FactoryCacheEntry {
    pub id: FactoryId,
    pub handle: FactoryHandle,
}

// Typical upper bound for factories per window; bumped automatically if a
// window installs more than this. Sized to keep the initial Arena and cache
// allocations in one cache line each.
pub const DEFAULT_FACTORIES_PER_WINDOW: usize = 16;

// One per Window. Owns the Factories that window renders with, and the
// Id-indexed cache used to look them up after the fact.
pub struct FactoryMaster {
    factories: Arena<FactoryTag, Factory>,
    cache: ThinVec<FactoryCacheEntry>,
}

impl FactoryMaster {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_FACTORIES_PER_WINDOW)
    }

    pub fn with_capacity(cap: usize) -> Self {
        let mut cache = ThinVec::with_capacity(cap);
        cache.reserve(cap);
        Self {
            factories: Arena::with_capacity(cap),
            cache,
        }
    }

    pub fn build_compute_proto(
        &mut self,
        proto: Proto<ComputeTag>,
        forge: &mut ForgeMaster,
        device: &ash::Device,
    ) -> ForgeResult<FactoryHandle> {
        let id = FactoryId::new(proto.id.raw());
        let factory = Factory::from_compute_proto(id, proto, forge)?;
        Ok(self.replace_or_insert(factory, device))
    }

    /// Async compute build. Drives every plan through
    /// `Factory::from_compute_proto_async` (one batched submit + signal
    /// semaphore) and returns the resulting factory handle alongside
    /// the semaphore the downstream graphics submission consumes.
    pub fn build_compute_proto_async(
        &mut self,
        proto: Proto<ComputeTag>,
        forge: &mut ForgeMaster,
        device: &ash::Device,
    ) -> ForgeResult<(FactoryHandle, ash::vk::Semaphore)> {
        let id = FactoryId::new(proto.id.raw());
        let (factory, sem) = Factory::from_compute_proto_async(id, proto, forge)?;
        Ok((self.replace_or_insert(factory, device), sem))
    }

    pub fn build_graphics_proto(
        &mut self,
        proto:  Proto<GraphicsTag>,
        device: &ash::Device,
    ) -> FactoryHandle {
        let id = FactoryId::new(proto.id.raw());
        let factory = Factory::from_graphics_proto(id, proto);
        self.replace_or_insert(factory, device)
    }

    /// Insert `factory`, or — if one with the same `FactoryId` is already
    /// live — destroy the old one and substitute the new. This is the
    /// per-frame entry point: each render frame rebuilds its compute /
    /// graphics protos and they overwrite their predecessors in place
    /// instead of accumulating forever (which would leak GPU memory and
    /// cause every previous frame's draws to keep firing).
    pub fn replace_or_insert(&mut self, factory: Factory, device: &ash::Device) -> FactoryHandle {
        let id = factory.id;
        if let Some(existing) = self.handle_of(id) {
            // Drop the old factory first to free its compute Ingots /
            // GraphicsFrame Arc<GpuMesh> references before the new factory
            // takes ownership of the slot.
            self.remove(existing, device);
        }
        self.insert(factory)
    }

    pub fn insert(&mut self, factory: Factory) -> FactoryHandle {
        let id = factory.id;
        let handle = self.factories.insert(factory);
        self.cache.push(FactoryCacheEntry { id, handle });
        handle
    }

    pub fn remove(&mut self, handle: FactoryHandle, device: &ash::Device) -> Option<Factory> {
        let mut factory = self.factories.remove(handle)?;
        unsafe { factory.destroy(device) };
        self.cache.retain(|entry| entry.handle != handle);
        Some(factory)
    }

    pub fn get(&self, handle: FactoryHandle) -> Option<&Factory> {
        self.factories.get(handle)
    }

    pub fn get_mut(&mut self, handle: FactoryHandle) -> Option<&mut Factory> {
        self.factories.get_mut(handle)
    }

    pub fn handle_of(&self, id: FactoryId) -> Option<FactoryHandle> {
        self.cache.iter().find(|entry| entry.id == id).map(|entry| entry.handle)
    }

    pub fn by_id(&self, id: FactoryId) -> Option<&Factory> {
        self.handle_of(id).and_then(|h| self.factories.get(h))
    }

    pub fn len(&self) -> usize {
        self.factories.len()
    }

    pub fn is_empty(&self) -> bool {
        self.factories.len() == 0
    }

    pub fn iter(&self) -> impl Iterator<Item = &Factory> {
        self.factories.values()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Factory> {
        self.factories.values_mut()
    }

    pub unsafe fn destroy(&mut self, device: &ash::Device) {
        for factory in self.factories.values_mut() {
            unsafe { factory.destroy(device) };
        }
        self.cache.clear();
    }
}

impl Default for FactoryMaster {
    fn default() -> Self {
        Self::new()
    }
}
