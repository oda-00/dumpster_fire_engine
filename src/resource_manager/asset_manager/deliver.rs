use super::asset::{Asset, AssetArena, AssetHandle, AssetType};

impl AssetArena {
    pub fn get(&self, handle: AssetHandle) -> Option<&Asset> {
        self.assets.get(handle)
    }

    pub fn get_mut(&mut self, handle: AssetHandle) -> Option<&mut Asset> {
        self.assets.get_mut(handle)
    }

    /// All handles of a given type, in insertion order.
    pub fn of_type(&self, ty: AssetType) -> &[AssetHandle] {
        &self.cache[ty.index()]
    }
}
