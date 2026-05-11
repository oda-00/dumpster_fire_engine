use super::asset::{Asset, AssetArena, AssetHandle, AssetId, AssetKind};

impl AssetArena {
    /// Register an asset in the arena and update the type cache.
    /// Returns a stable handle. The caller is responsible for deduplication.
    pub fn fetch(&mut self, id: AssetId, kind: AssetKind) -> AssetHandle {
        let ty = kind.asset_type();
        let handle = self.assets.insert(Asset::new(id, kind));
        self.cache[ty.index()].push(handle);
        handle
    }

    /// Remove an asset from the arena and evict it from the type cache.
    pub fn evict(&mut self, handle: AssetHandle) -> Option<Asset> {
        let asset = self.assets.remove(handle)?;
        Self::cache_remove(&mut self.cache[asset.asset_type().index()], handle);
        Some(asset)
    }

    /// Replace an asset payload while preserving the handle and moving it to
    /// the correct type bucket when the discriminant changes.
    pub fn replace_kind(&mut self, handle: AssetHandle, kind: AssetKind) -> Option<AssetKind> {
        let old_ty = self.assets.get(handle)?.asset_type();
        let new_ty = kind.asset_type();
        let old = self.assets.get_mut(handle)?.replace_kind(kind);

        if old_ty != new_ty {
            Self::cache_remove(&mut self.cache[old_ty.index()], handle);
            if !self.cache[new_ty.index()].contains(&handle) {
                self.cache[new_ty.index()].push(handle);
            }
        }

        Some(old)
    }

    pub(super) fn cache_remove(
        cache_slot: &mut thin_vec::ThinVec<AssetHandle>,
        handle: AssetHandle,
    ) {
        if let Some(pos) = cache_slot.iter().position(|&h| h == handle) {
            cache_slot.swap_remove(pos);
        }
    }
}
