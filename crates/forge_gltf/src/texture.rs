//! Texture / image / sampler representation.
//!
//! Images are stored as decoded RGBA8 byte buffers — the engine's
//! `TextureOre` shape takes raw pixels, so we resolve the gltf source data
//! to that single canonical layout. Source format hints (sRGB vs linear)
//! travel with the image so the engine bridge can pick the right vk format
//! without re-parsing.

use thin_vec::ThinVec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ImageFormatHint {
    /// Albedo / base colour / emissive — sRGB encoded.
    Srgb,
    /// Normals / metallic-roughness / occlusion / data — linear.
    Linear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MagFilter { Nearest, Linear }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MinFilter {
    Nearest,
    Linear,
    NearestMipmapNearest,
    LinearMipmapNearest,
    NearestMipmapLinear,
    LinearMipmapLinear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum WrapMode { ClampToEdge, MirroredRepeat, Repeat }

#[derive(Debug, Clone)]
pub struct Sampler {
    pub name:       Option<String>,
    pub mag_filter: MagFilter,
    pub min_filter: MinFilter,
    pub wrap_s:     WrapMode,
    pub wrap_t:     WrapMode,
}

impl Default for Sampler {
    fn default() -> Self {
        Self {
            name: None,
            mag_filter: MagFilter::Linear,
            min_filter: MinFilter::LinearMipmapLinear,
            wrap_s: WrapMode::Repeat,
            wrap_t: WrapMode::Repeat,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Image {
    pub name:   Option<String>,
    pub width:  u32,
    pub height: u32,
    /// 4 bytes per pixel, top-left origin (matches gltf::image::Data conventions).
    pub rgba:   ThinVec<u8>,
    pub format: ImageFormatHint,
}

#[derive(Debug, Clone)]
pub struct Texture {
    pub name:    Option<String>,
    pub image:   u32,
    pub sampler: Option<u32>,
}

pub(crate) fn min_filter_from(f: gltf::texture::MinFilter) -> MinFilter {
    use gltf::texture::MinFilter as G;
    match f {
        G::Nearest              => MinFilter::Nearest,
        G::Linear               => MinFilter::Linear,
        G::NearestMipmapNearest => MinFilter::NearestMipmapNearest,
        G::LinearMipmapNearest  => MinFilter::LinearMipmapNearest,
        G::NearestMipmapLinear  => MinFilter::NearestMipmapLinear,
        G::LinearMipmapLinear   => MinFilter::LinearMipmapLinear,
    }
}

pub(crate) fn mag_filter_from(f: gltf::texture::MagFilter) -> MagFilter {
    use gltf::texture::MagFilter as G;
    match f {
        G::Nearest => MagFilter::Nearest,
        G::Linear  => MagFilter::Linear,
    }
}

pub(crate) fn wrap_from(w: gltf::texture::WrappingMode) -> WrapMode {
    use gltf::texture::WrappingMode as G;
    match w {
        G::ClampToEdge    => WrapMode::ClampToEdge,
        G::MirroredRepeat => WrapMode::MirroredRepeat,
        G::Repeat         => WrapMode::Repeat,
    }
}
