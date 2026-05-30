use super::font;
use super::icon::Icon;
use super::vector;
use crate::vfs::{EmbeddedFs, Mount, Vfs};
use thin_vec::ThinVec;

/// Build the asset VFS used to source UI icon SVGs. Starts from the shared
/// [`crate::vfs::engine_default`] (a `Dir` mount on `assets/` when present, the
/// dev/loose-file override) and layers the embedded SVG registry underneath as
/// the guaranteed fallback. So a loose `assets/icons/lucide/*.svg` shadows the
/// embedded copy (hot-reload / modding), and shipped binaries without the
/// directory fall back to the embedded bytes.
pub fn asset_vfs() -> Vfs {
    let mut v = crate::vfs::engine_default();
    v.mount(Mount::Embedded(EmbeddedFs::from_pairs(
        Icon::ALL.iter().map(|ic| (ic.vpath(), ic.svg_bytes())),
    )));
    v
}

/// CPU-side font + icon atlas. GPU upload (ForgeImage) is handled by the
/// UI forge when it registers; this struct holds the baked bitmap and the
/// uv rects.
///
/// The bitmap is one R8 sheet, `ATLAS_FULL_W × ATLAS_FULL_H`: the hand-coded
/// VGA font occupies the top `ATLAS_W × ATLAS_H` block and the Lucide icons are
/// rasterized below it (at runtime, by the handrolled [`vector`] rasterizer).
pub struct Atlas {
    pub pixels: ThinVec<u8>, // ATLAS_FULL_W * ATLAS_FULL_H bytes (R8_UNORM)
    pub width: u32,
    pub height: u32,
    /// Glyph uv rects indexed by ASCII codepoint (0x00..0x80).
    pub glyph_rects: ThinVec<[f32; 4]>,
    /// Icon uv rects, indexed by `Icon as u32` (atlas slot).
    pub icon_rects: ThinVec<[f32; 4]>,
}

impl Atlas {
    pub fn build() -> Self {
        let pixels = bake_combined();

        let mut glyph_rects = ThinVec::with_capacity(0x80);
        for cu in 0..0x80u32 {
            let c = char::from_u32(cu).unwrap_or(' ');
            glyph_rects.push(font::glyph_rect(c));
        }

        let mut icon_rects = ThinVec::with_capacity(Icon::ALL.len());
        for ic in Icon::ALL {
            icon_rects.push(font::icon_rect(ic.slot()));
        }

        Self {
            pixels,
            width: font::ATLAS_FULL_W,
            height: font::ATLAS_FULL_H,
            glyph_rects,
            icon_rects,
        }
    }
}

/// Bake the combined font + icon R8 sheet. The font glyphs are expanded into
/// the top block; each icon SVG is rasterized through the handrolled vector
/// rasterizer into its cell in the icon block below.
pub fn bake_combined() -> ThinVec<u8> {
    let fw = font::ATLAS_W as usize;
    let full_w = font::ATLAS_FULL_W as usize;
    let full_h = font::ATLAS_FULL_H as usize;

    let mut out: ThinVec<u8> = ThinVec::with_capacity(full_w * full_h);
    out.resize(full_w * full_h, 0u8);

    // Copy the font block (same width, top rows) into the combined sheet.
    let font_px = font::bake_atlas();
    for row in 0..font::ATLAS_H as usize {
        let src = &font_px[row * fw..row * fw + fw];
        let dst = &mut out[row * full_w..row * full_w + fw];
        dst.copy_from_slice(src);
    }

    // Rasterize each icon into its cell. Lucide icons are a 24-unit viewBox
    // with a 2-unit stroke; the cell size matches so scale is 1:1. SVG source
    // comes through the VFS, so a loose file under `assets/` overrides the
    // embedded copy (the embedded registry guarantees a value either way).
    let vfs = asset_vfs();
    let cell = font::ICON_CELL;
    for ic in Icon::ALL {
        let svg = vfs
            .read_to_string(ic.vpath())
            .unwrap_or_else(|_| ic.svg().to_string());
        let glyph = vector::rasterize_svg(&svg, cell, 24.0, 2.0);
        let rect = font::icon_rect(ic.slot());
        let x0 = (rect[0] * full_w as f32).round() as usize;
        let y0 = (rect[1] * full_h as f32).round() as usize;
        for gy in 0..cell as usize {
            for gx in 0..cell as usize {
                let v = glyph[gy * cell as usize + gx];
                if v != 0 {
                    let dx = x0 + gx;
                    let dy = y0 + gy;
                    if dx < full_w && dy < full_h {
                        out[dy * full_w + dx] = v;
                    }
                }
            }
        }
    }

    out
}

impl Default for Atlas {
    fn default() -> Self {
        Self::build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combined_atlas_has_expected_dims() {
        let atlas = Atlas::build();
        assert_eq!(atlas.width, font::ATLAS_FULL_W);
        assert_eq!(atlas.height, font::ATLAS_FULL_H);
        assert_eq!(
            atlas.pixels.len(),
            (font::ATLAS_FULL_W * font::ATLAS_FULL_H) as usize
        );
    }

    #[test]
    fn every_icon_has_a_valid_uv_rect() {
        let atlas = Atlas::build();
        assert_eq!(atlas.icon_rects.len(), Icon::ALL.len());
        for r in &atlas.icon_rects {
            assert!(r[0] >= 0.0 && r[2] <= 1.0 && r[0] < r[2], "u range {r:?}");
            assert!(r[1] >= 0.0 && r[3] <= 1.0 && r[1] < r[3], "v range {r:?}");
        }
    }

    #[test]
    fn icons_actually_rasterize_pixels() {
        // The icon block (rows >= ATLAS_H) must contain lit pixels.
        let px = bake_combined();
        let full_w = font::ATLAS_FULL_W as usize;
        let lit = px[font::ATLAS_H as usize * full_w..]
            .iter()
            .filter(|&&v| v > 0)
            .count();
        assert!(lit > 50, "expected icon strokes to mark pixels, got {lit}");
    }
}
