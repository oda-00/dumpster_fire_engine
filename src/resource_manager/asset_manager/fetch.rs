use super::asset::{Asset, AssetArena, AssetHandle, AssetId, AssetKind};

impl AssetArena {
    /// Register an asset in the arena and update the type cache.
    /// Returns a stable handle. The caller is responsible for deduplication.
    pub fn fetch(&mut self, id: AssetId, kind: AssetKind) -> AssetHandle {
        let ty = kind.asset_type();
        let handle = self.assets.insert(Asset { id, data: kind });
        self.cache[ty.index()].push(handle);
        handle
    }

    /// Remove an asset from the arena and evict it from the type cache.
    pub fn evict(&mut self, handle: AssetHandle) {
        if let Some(asset) = self.assets.remove(handle) {
            let idx = asset.data.asset_type().index();
            self.cache[idx].retain(|&h| h != handle);
        }
    }
}
