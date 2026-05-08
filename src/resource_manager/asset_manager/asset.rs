use std::sync::Arc;
use crate::resource_manager::manager::{Arena, Handle, Id};

// ── Arena tag / handles ─────────────────────────────────────────────────────

#[derive(Copy, Clone, PartialEq, Eq)] pub struct AssetTag;
pub type AssetHandle = Handle<AssetTag>;

// ── Database ID ─────────────────────────────────────────────────────────────

pub struct AssetMarker;
pub type AssetId = Id<AssetMarker>;

// ── Discriminant enum (array index) ────────────────────────────────────────

#[repr(u8)]
pub enum AssetType {
    Texture   = 0,
    TitleText = 1,
    Visual    = 2,
    Audio     = 3,
    Mesh      = 4,
}

impl AssetType {
    pub const COUNT: usize = 5;
    pub const fn index(self) -> usize { self as usize }
}

// ── Data-carrying enum ──────────────────────────────────────────────────────

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
            AssetKind::Texture(_)   => AssetType::Texture,
            AssetKind::TitleText(_) => AssetType::TitleText,
            AssetKind::Visual(_)    => AssetType::Visual,
            AssetKind::Audio(_)     => AssetType::Audio,
            AssetKind::Mesh(_)      => AssetType::Mesh,
        }
    }
}

pub struct Asset {
    pub id:   AssetId,
    pub data: AssetKind,
}

// ── Asset subtypes ──────────────────────────────────────────────────────────

pub struct Texture   { pub path: Arc<str> }
pub struct TitleText { pub text: Arc<str> }
// Visual: shader/material description; Mesh: raw geometry.
pub struct Visual    { pub path: Arc<str> }
pub struct Mesh      { pub path: Arc<str> }
pub struct Audio     { pub path: Arc<str> }

// ── Arena ───────────────────────────────────────────────────────────────────

pub struct AssetArena {
    pub assets: Arena<AssetTag, Asset>,
    pub cache:  [Vec<AssetHandle>; AssetType::COUNT],
}

impl AssetArena {
    pub fn new() -> Self {
        Self {
            assets: Arena::new(),
            cache:  std::array::from_fn(|_| Vec::new()),
        }
    }
}
