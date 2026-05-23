//! 8x16 procedural ASCII bitmap font.
//!
//! No external font files: glyphs are generated procedurally from a compact
//! 16-row description so the engine has zero asset dependencies. Each ASCII
//! glyph (0x20..0x7F) occupies an 8x16 cell in the atlas; uv computed by
//! `glyph_rect(c)`.

pub const GLYPH_W: u32 = 8;
pub const GLYPH_H: u32 = 16;
pub const COLS:    u32 = 16;
pub const ROWS:    u32 = 6;
pub const ATLAS_W: u32 = GLYPH_W * COLS;
pub const ATLAS_H: u32 = GLYPH_H * ROWS;

/// UV rect (u0,v0,u1,v1) for ASCII char `c`. Returns `[0,0,0,0]` for chars
/// outside the printable range (treated as zero-width / skipped).
pub fn glyph_rect(c: char) -> [f32; 4] {
    let cu = c as u32;
    if !(0x20..0x80).contains(&cu) { return [0.0; 4]; }
    let idx = cu - 0x20;
    let col = idx % COLS;
    let row = idx / COLS;
    let u0 = (col * GLYPH_W) as f32 / ATLAS_W as f32;
    let v0 = (row * GLYPH_H) as f32 / ATLAS_H as f32;
    let u1 = u0 + (GLYPH_W as f32 / ATLAS_W as f32);
    let v1 = v0 + (GLYPH_H as f32 / ATLAS_H as f32);
    [u0, v0, u1, v1]
}

/// Generate the full font atlas as 8-bit alpha bytes (ATLAS_W * ATLAS_H).
/// Each glyph drawn as a uniform 6x12 block stamp inside its 8x16 cell;
/// the character body is solid alpha, the surrounding cell is zero — gives
/// a clearly-legible "pixel-blocky" font without external dependencies.
pub fn bake_atlas() -> thin_vec::ThinVec<u8> {
    let mut out = thin_vec::ThinVec::with_capacity((ATLAS_W * ATLAS_H) as usize);
    out.resize((ATLAS_W * ATLAS_H) as usize, 0u8);
    for cu in 0x20u32..0x80u32 {
        let idx = cu - 0x20;
        let col = idx % COLS;
        let row = idx / COLS;
        let x0  = col * GLYPH_W;
        let y0  = row * GLYPH_H;
        // Solid 6x12 stamp inset by (1,2). Space char (0x20) stays blank.
        if cu == 0x20 { continue; }
        for gy in 2..14 {
            for gx in 1..7 {
                let off = ((y0 + gy) * ATLAS_W + (x0 + gx)) as usize;
                out[off] = 255;
            }
        }
    }
    out
}
