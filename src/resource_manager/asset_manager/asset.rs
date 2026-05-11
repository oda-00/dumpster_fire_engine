use std::sync::Arc;
use thin_vec::ThinVec;

use crate::resource_manager::manager::{Arena, Handle, Id};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct AssetTag;
pub type AssetHandle = Handle<AssetTag>;

pub struct AssetMarker;
pub type AssetId = Id<AssetMarker>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum AssetType {
    Texture = 0,
    TitleText = 1,
    Visual = 2,
    Audio = 3,
    Mesh = 4,
}

impl AssetType {
    pub const ALL: [AssetType; 5] = [
        AssetType::Texture,
        AssetType::TitleText,
        AssetType::Visual,
        AssetType::Audio,
        AssetType::Mesh,
    ];

    pub const COUNT: usize = Self::ALL.len();

    pub const fn index(self) -> usize {
        self as usize
    }
}

#[derive(Debug, Clone)]
pub enum AssetKind {
    Texture(Texture),
    TitleText(TitleText),
    Visual(Visual),
    Audio(Audio),
    Mesh(Mesh),
}

impl AssetKind {
    pub fn asset_type(&self) -> AssetType {
        match self {
            AssetKind::Texture(_) => AssetType::Texture,
            AssetKind::TitleText(_) => AssetType::TitleText,
            AssetKind::Visual(_) => AssetType::Visual,
            AssetKind::Audio(_) => AssetType::Audio,
            AssetKind::Mesh(_) => AssetType::Mesh,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Asset {
    id: AssetId,
    data: AssetKind,
}

impl Asset {
    pub(super) fn new(id: AssetId, data: AssetKind) -> Self {
        Self { id, data }
    }

    pub fn id(&self) -> AssetId {
        self.id
    }

    pub fn data(&self) -> &AssetKind {
        &self.data
    }

    pub fn asset_type(&self) -> AssetType {
        self.data.asset_type()
    }

    pub(super) fn replace_kind(&mut self, kind: AssetKind) -> AssetKind {
        std::mem::replace(&mut self.data, kind)
    }
}

#[derive(Debug, Clone)]
pub struct Texture {
    pub path: Arc<str>,
}

#[derive(Debug, Clone)]
pub struct TitleText {
    pub text: Arc<str>,
}

#[derive(Debug, Clone)]
pub struct Visual {
    pub path: Arc<str>,
}

#[derive(Debug, Clone)]
pub struct Mesh {
    pub path: Arc<str>,
}

#[derive(Debug, Clone)]
pub struct Audio {
    pub path: Arc<str>,
}

pub struct AssetArena {
    pub(super) assets: Arena<AssetTag, Asset>,
    pub(super) cache: [ThinVec<AssetHandle>; AssetType::COUNT],
}

impl AssetArena {
    pub fn new() -> Self {
        Self {
            assets: Arena::new(),
            cache: std::array::from_fn(|_| ThinVec::new()),
        }
    }

    pub fn id(&self, handle: AssetHandle) -> Option<AssetId> {
        self.assets.get(handle).map(Asset::id)
    }
}

impl Default for AssetArena {
    fn default() -> Self {
        Self::new()
    }
}
