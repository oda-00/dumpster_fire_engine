use std::sync::Arc;
use thin_vec::ThinVec;

use crate::forge_master::ore::OreKind;
use crate::resource_manager::manager::{Arena, Handle, Id};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct AssetTag;
pub type AssetHandle = Handle<AssetTag>;

pub struct AssetMarker;
pub type AssetId = Id<AssetMarker>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum AssetType {
    Texture     = 0,
    TitleText   = 1,
    Visual      = 2,
    Audio       = 3,
    Mesh        = 4,
    IngotBuffer = 5,
    IngotImage  = 6,
}

impl AssetType {
    pub const ALL: [AssetType; 7] = [
        AssetType::Texture,
        AssetType::TitleText,
        AssetType::Visual,
        AssetType::Audio,
        AssetType::Mesh,
        AssetType::IngotBuffer,
        AssetType::IngotImage,
    ];

    pub const COUNT: usize = Self::ALL.len();

    pub const fn index(self) -> usize {
        self as usize
    }
}

// ── GPU compute result types ─────────────────────────────────────────────────
//
// Produced by `Pipeline::forge_tick`. Both variants hold the readback bytes as
// a reference-counted slice so cloning an `AssetKind` is a single atomic bump
// rather than a heap copy — important when the same ingot is referenced by
// multiple in-flight pipelines.

/// Compute shader output read back as a raw byte buffer.
#[derive(Debug, Clone)]
pub struct IngotBuffer {
    pub ore_kind: OreKind,
    /// Readback bytes from the `INGOT_BUFFER_BINDING` storage buffer.
    pub data: Arc<[u8]>,
}

/// Compute shader output read back as a 2-D image.
#[derive(Debug, Clone)]
pub struct IngotImage {
    pub ore_kind: OreKind,
    pub width:    u32,
    pub height:   u32,
    /// Readback bytes in the format specified at `Ore` construction time.
    pub data: Arc<[u8]>,
}

// ── AssetKind ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum AssetKind {
    Texture(Texture),
    TitleText(TitleText),
    Visual(Visual),
    Audio(Audio),
    Mesh(Mesh),
    IngotBuffer(IngotBuffer),
    IngotImage(IngotImage),
}

impl AssetKind {
    pub fn asset_type(&self) -> AssetType {
        match self {
            AssetKind::Texture(_)     => AssetType::Texture,
            AssetKind::TitleText(_)   => AssetType::TitleText,
            AssetKind::Visual(_)      => AssetType::Visual,
            AssetKind::Audio(_)       => AssetType::Audio,
            AssetKind::Mesh(_)        => AssetType::Mesh,
            AssetKind::IngotBuffer(_) => AssetType::IngotBuffer,
            AssetKind::IngotImage(_)  => AssetType::IngotImage,
        }
    }
}

// ── Asset ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Asset {
    id:   AssetId,
    data: AssetKind,
}

impl Asset {
    pub(super) fn new(id: AssetId, data: AssetKind) -> Self {
        Self { id, data }
    }

    pub fn id(&self)          -> AssetId    { self.id }
    pub fn data(&self)        -> &AssetKind { &self.data }
    pub fn asset_type(&self)  -> AssetType  { self.data.asset_type() }

    pub(super) fn replace_kind(&mut self, kind: AssetKind) -> AssetKind {
        std::mem::replace(&mut self.data, kind)
    }
}

// ── File-based asset kinds ────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Texture   { pub path: Arc<str> }

#[derive(Debug, Clone)]
pub struct TitleText { pub text: Arc<str> }

#[derive(Debug, Clone)]
pub struct Visual    { pub path: Arc<str> }

#[derive(Debug, Clone)]
pub struct Mesh      { pub path: Arc<str> }

#[derive(Debug, Clone)]
pub struct Audio     { pub path: Arc<str> }

// ── AssetArena ────────────────────────────────────────────────────────────────

pub struct AssetArena {
    pub(super) assets: Arena<AssetTag, Asset>,
    pub(super) cache:  [ThinVec<AssetHandle>; AssetType::COUNT],
}

impl AssetArena {
    pub fn new() -> Self {
        Self {
            assets: Arena::new(),
            cache:  std::array::from_fn(|_| ThinVec::new()),
        }
    }

    pub fn id(&self, handle: AssetHandle) -> Option<AssetId> {
        self.assets.get(handle).map(Asset::id)
    }


    pub fn get(&self, handle: AssetHandle) -> Option<&Asset> {
        self.assets.get(handle)
    }

    pub fn get_mut(&mut self, handle: AssetHandle) -> Option<&mut Asset> {
        self.assets.get_mut(handle)
    }

    pub fn insert(&mut self, asset: Asset) -> AssetHandle {
        let ty = asset.asset_type();
        let h = self.assets.insert(asset);
        self.cache[ty.index()].push(h);
        h
    }

    pub fn remove(&mut self, handle: AssetHandle) -> Option<Asset> {
        let asset = self.assets.remove(handle)?;
        let type_slot = &mut self.cache[asset.asset_type().index()];
        if let Some(pos) = type_slot.iter().position(|&h| h == handle) {
            type_slot.swap_remove(pos);
    }
    Some(asset)
}

    pub fn contains(&self, handle: AssetHandle) -> bool {
        self.assets.contains(handle)
    }

    pub fn len(&self) -> usize {
        self.assets.len()
    }

    /// All handles of a given type, in insertion order.
    pub fn of_type(&self, ty: AssetType) -> &[AssetHandle] {
        &self.cache[ty.index()]
    }

    pub fn type_len(&self, ty: AssetType) -> usize {
        self.cache[ty.index()].len()
    }
}


impl Default for AssetArena {
    fn default() -> Self { Self::new() }
}
