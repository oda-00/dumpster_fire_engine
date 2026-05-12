
use super::asset::{Asset, AssetArena, AssetHandle, AssetId, AssetType};

pub struct Sender {
    asset_arena: AssetArena,
    sender_cache: [thin_vec::ThinVec<AssetHandle>; AssetType::COUNT],   
}

impl Sender {
    pub fn new(asset_arena: AssetArena) -> Self {
        Self { asset_arena, sender_cache: Default::default() }
    }

    pub fn get(&self, handle: AssetHandle) -> Option<&Asset> {
    self.asset_arena.get(handle)
}
    pub fn contains(&self, handle: AssetHandle) -> bool {
        self.asset_arena.contains(handle)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn len(&self) -> usize {
        self.asset_arena.len()
    }

    /// All handles of a given type, in insertion order.
    pub fn of_type(&self, ty: AssetType) -> &[AssetHandle] {
        &self.sender_cache[ty.index()]
    }

    pub fn type_len(&self, ty: AssetType) -> usize {
        self.sender_cache[ty.index()].len()
    }
      pub fn id(&self, handle: AssetHandle) -> Option<AssetId> {
        self.asset_arena.id(handle)
    }
}
