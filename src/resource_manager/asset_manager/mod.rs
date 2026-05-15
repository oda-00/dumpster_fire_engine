pub mod asset;
pub mod gltf_loader;
pub mod send;
pub mod fetch;
pub mod pipe;
pub mod pipeline;

pub use asset::*;
pub use gltf_loader::*;
pub use send::*;
pub use fetch::*;
pub use pipe::*;
pub use pipeline::*;

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::resource_manager::asset_manager::fetch::Fetcher;

use super::*;

    fn texture(path: &str) -> AssetKind {
        AssetKind::Texture(Texture {
            path: Arc::<str>::from(path),
        })
    }

    fn mesh(path: &str) -> AssetKind {
        AssetKind::Mesh(Mesh {
            path: Arc::<str>::from(path),
        })
    }

    #[test]
    fn fetch_and_evict_keep_type_cache_consistent() {
        let mut arena = Fetcher::new(AssetArena::new());
        let tex = arena.fetch(AssetId::new(1), texture("albedo.png"));
        let mesh = arena.fetch(AssetId::new(2), mesh("crate.mesh"));

        assert_eq!(arena.type_len(AssetType::Texture), 1);
        assert_eq!(arena.type_len(AssetType::Mesh), 1);
        assert!(arena.of_type(AssetType::Texture).contains(&tex));
        assert!(arena.of_type(AssetType::Mesh).contains(&mesh));

        let removed = arena.evict(tex).expect("texture should be live");
        assert_eq!(removed.id(), AssetId::new(1));
        assert!(!arena.contains(tex));
        assert!(!arena.of_type(AssetType::Texture).contains(&tex));
        assert!(arena.of_type(AssetType::Mesh).contains(&mesh));
    }

    #[test]
    fn replace_kind_moves_handle_between_type_buckets() {
        let mut arena = Fetcher::new(AssetArena::new());
        let handle = arena.fetch(AssetId::new(7), texture("source.png"));

        let old = arena
            .replace_kind(handle, mesh("baked.mesh"))
            .expect("asset should be live");

        assert!(matches!(old, AssetKind::Texture(_)));
        assert_eq!(arena.get(handle).unwrap().asset_type(), AssetType::Mesh);
        assert!(!arena.of_type(AssetType::Texture).contains(&handle));
        assert!(arena.of_type(AssetType::Mesh).contains(&handle));
    }

    #[test]
    fn stale_handles_do_not_resolve_after_evict() {
        let mut arena = Fetcher::new(AssetArena::new());
        let handle = arena.fetch(AssetId::new(3), texture("gone.png"));

        assert!(arena.evict(handle).is_some());
        assert!(arena.evict(handle).is_none());
        assert!(arena.get(handle).is_none());
        assert!(arena.id(handle).is_none());
    }

    #[test]
    fn queue_entries_resolve_against_their_declared_source() {
        let mut a = Fetcher::new(AssetArena::new());
        let h_a = a.fetch(AssetId::new(10), texture("a.png"));

        let mut b = Fetcher::new(AssetArena::new());
        let h_b = b.fetch(AssetId::new(20), texture("b.png"));

        let mut manager = Pipeline::new();
        let a_idx = manager.add_fetcher(a);
        let b_idx = manager.add_fetcher(b);

        let entry_a = QueueEntry::new(1, AssetId::new(10), AssetSource::Fetcher(a_idx), h_a);
        let entry_b = QueueEntry::new(2, AssetId::new(20), AssetSource::Fetcher(b_idx), h_b);

        assert_eq!(manager.resolve(&entry_a).unwrap().id(), AssetId::new(10));
        assert_eq!(manager.resolve(&entry_b).unwrap().id(), AssetId::new(20));
    }
}
