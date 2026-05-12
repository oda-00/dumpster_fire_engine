
use super::asset::{Asset, AssetArena, AssetHandle, AssetId, AssetKind, AssetType};
pub struct Fetcher {
    asset_arena: AssetArena,
    fetcher_cache: [thin_vec::ThinVec<AssetHandle>; AssetType::COUNT],
}

impl Fetcher {
    pub fn new(asset_arena: AssetArena) -> Self {
        Self { asset_arena, fetcher_cache: Default::default() }
    }
    pub fn get(&self, handle: AssetHandle) -> Option<&Asset> {
    self.asset_arena.get(handle)
}

    /// Register an asset in the arena and update the type cache.
    /// Returns a stable handle. The caller is responsible for deduplication.
    pub fn fetch(&mut self, id: AssetId, kind: AssetKind) -> AssetHandle {
        let ty = kind.asset_type();
        let handle = self.asset_arena.insert(Asset::new(id, kind));
        self.fetcher_cache[ty.index()].push(handle);
        handle
    }

    /// Remove an asset from the arena and evict it from the type cache.
    pub fn evict(&mut self, handle: AssetHandle) -> Option<Asset> {
        let asset = self.asset_arena.remove(handle)?;
        Self::cache_remove(&mut self.fetcher_cache[asset.asset_type().index()], handle);
        Some(asset)
    }

    /// Replace an asset payload while preserving the handle and moving it to
    /// the correct type bucket when the discriminant changes.
    pub fn replace_kind(&mut self, handle: AssetHandle, kind: AssetKind) -> Option<AssetKind> {
        let old_ty = self.asset_arena.get(handle)?.asset_type();
        let new_ty = kind.asset_type();
        let old = self.asset_arena.get_mut(handle)?.replace_kind(kind);

        if old_ty != new_ty {
            Self::cache_remove(&mut self.fetcher_cache[old_ty.index()], handle);
            if !self.fetcher_cache[new_ty.index()].contains(&handle) {
                self.fetcher_cache[new_ty.index()].push(handle);
            }
        }

        Some(old)
    }
    pub fn of_type(&self, ty: AssetType) -> &[AssetHandle] {
        &self.fetcher_cache[ty.index()]
}

    pub fn type_len(&self, ty: AssetType) -> usize {
        self.fetcher_cache[ty.index()].len()
}
    pub(super) fn cache_remove(
        cache_slot: &mut thin_vec::ThinVec<AssetHandle>,
        handle: AssetHandle,
    ) {
        if let Some(pos) = cache_slot.iter().position(|&h| h == handle) {
            cache_slot.swap_remove(pos);
        }
    }
     pub fn contains(&self, handle: AssetHandle) -> bool {
        self.asset_arena.contains(handle)
    }

    pub fn id(&self, handle: AssetHandle) -> Option<AssetId> {
        self.asset_arena.id(handle)
    }

}
