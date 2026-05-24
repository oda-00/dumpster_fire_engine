use super::font;
use thin_vec::ThinVec;

/// CPU-side font + icon atlas. GPU upload (ForgeImage) is handled by the
/// UI forge when it registers; this struct holds the baked bitmap and the
/// uv rects.
pub struct Atlas {
    pub pixels: ThinVec<u8>, // ATLAS_W * ATLAS_H bytes (R8_UNORM)
    pub width: u32,
    pub height: u32,
    /// Glyph uv rects indexed by ASCII codepoint (0x00..0x80).
    pub glyph_rects: ThinVec<[f32; 4]>,
    /// Icon uv rects appended after the glyph block, keyed by IconId.
    pub icon_rects: ThinVec<[f32; 4]>,
}

impl Atlas {
    pub fn build() -> Self {
        let mut glyph_rects = ThinVec::with_capacity(0x80);
        for cu in 0..0x80u32 {
            let c = char::from_u32(cu).unwrap_or(' ');
            glyph_rects.push(font::glyph_rect(c));
        }
        Self {
            pixels: font::bake_atlas(),
            width: font::ATLAS_W,
            height: font::ATLAS_H,
            glyph_rects,
            icon_rects: ThinVec::new(),
        }
    }
}

impl Default for Atlas {
    fn default() -> Self {
        Self::build()
    }
}
