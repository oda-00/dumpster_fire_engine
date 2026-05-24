use super::asset::{Asset, AssetArena, AssetHandle, AssetId, AssetKind, AssetType};
pub struct Fetcher {
    asset_arena: AssetArena,
    fetcher_cache: [thin_vec::ThinVec<AssetHandle>; AssetType::COUNT],
    // cache_pos[slot] = position of handle `slot` in fetcher_cache[type], u16::MAX if absent.
    cache_pos: Vec<u16>,
}

impl Fetcher {
    pub fn new(asset_arena: AssetArena) -> Self {
        Self {
            asset_arena,
            fetcher_cache: Default::default(),
            cache_pos: Vec::new(),
        }
    }
    pub fn get(&self, handle: AssetHandle) -> Option<&Asset> {
        self.asset_arena.get(handle)
    }

    /// Register an asset in the arena and update the type cache.
    /// Returns a stable handle. The caller is responsible for deduplication.
    pub fn fetch(&mut self, id: AssetId, kind: AssetKind) -> AssetHandle {
        let ty = kind.asset_type();
        let handle = self.asset_arena.insert(Asset::new(id, kind));
        let idx = handle.idx as usize;
        let pos = self.fetcher_cache[ty.index()].len() as u16;
        self.fetcher_cache[ty.index()].push(handle);
        if idx == self.cache_pos.len() {
            self.cache_pos.push(pos);
        } else {
            self.cache_pos[idx] = pos;
        }
        handle
    }

    /// Remove an asset from the arena and evict it from both type caches.
    pub fn evict(&mut self, handle: AssetHandle) -> Option<Asset> {
        let asset = self.asset_arena.remove(handle)?;
        let ty_idx = asset.asset_type().index();
        let slot = handle.idx as usize;
        let pos = self.cache_pos[slot];
        if pos != u16::MAX {
            self.fetcher_cache[ty_idx].swap_remove(pos as usize);
            self.cache_pos[slot] = u16::MAX;
            if (pos as usize) < self.fetcher_cache[ty_idx].len() {
                let displaced = self.fetcher_cache[ty_idx][pos as usize];
                self.cache_pos[displaced.idx as usize] = pos;
            }
        }
        Some(asset)
    }

    /// Replace an asset payload while preserving the handle and moving it to
    /// the correct type bucket when the discriminant changes.
    pub fn replace_kind(&mut self, handle: AssetHandle, kind: AssetKind) -> Option<AssetKind> {
        let old_ty = self.asset_arena.get(handle)?.asset_type();
        let new_ty = kind.asset_type();
        let old = self.asset_arena.get_mut(handle)?.replace_kind(kind);

        if old_ty != new_ty {
            let slot = handle.idx as usize;
            let old_ty_idx = old_ty.index();
            let new_ty_idx = new_ty.index();
            // O(1) removal from old bucket.
            let pos = self.cache_pos[slot];
            if pos != u16::MAX {
                self.fetcher_cache[old_ty_idx].swap_remove(pos as usize);
                self.cache_pos[slot] = u16::MAX;
                if (pos as usize) < self.fetcher_cache[old_ty_idx].len() {
                    let displaced = self.fetcher_cache[old_ty_idx][pos as usize];
                    self.cache_pos[displaced.idx as usize] = pos;
                }
            }
            // O(1) insertion into new bucket.
            let new_pos = self.fetcher_cache[new_ty_idx].len() as u16;
            self.fetcher_cache[new_ty_idx].push(handle);
            self.cache_pos[slot] = new_pos;
        }

        Some(old)
    }
    pub fn of_type(&self, ty: AssetType) -> &[AssetHandle] {
        &self.fetcher_cache[ty.index()]
    }

    pub fn type_len(&self, ty: AssetType) -> usize {
        self.fetcher_cache[ty.index()].len()
    }
    pub fn contains(&self, handle: AssetHandle) -> bool {
        self.asset_arena.contains(handle)
    }

    pub fn id(&self, handle: AssetHandle) -> Option<AssetId> {
        self.asset_arena.id(handle)
    }
}
