use super::asset::{Asset, AssetArena, AssetHandle, AssetType};

impl AssetArena {
    pub fn get(&self, handle: AssetHandle) -> Option<&Asset> {
        self.assets.get(handle)
    }

    pub fn contains(&self, handle: AssetHandle) -> bool {
        self.assets.contains(handle)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn len(&self) -> usize {
        self.assets.values().count()
    }

    /// All handles of a given type, in insertion order.
    pub fn of_type(&self, ty: AssetType) -> &[AssetHandle] {
        &self.cache[ty.index()]
    }

    pub fn type_len(&self, ty: AssetType) -> usize {
        self.cache[ty.index()].len()
    }
}
