//! Experiment 6 — Glyph quad generation inner loop.
//!
//! Mirrors `DrawList::push_text` (src/render/ui_render/drawlist.rs): per-char
//! atlas lookup + 4-vertex quad + advance. Question: what does the per-glyph
//! work compile to, and does the `chars()` UTF-8 decode dominate?
//!
//! Build: rustc -O --emit asm --crate-type=lib exp6_glyph.rs

#[repr(C)]
#[derive(Copy, Clone)]
pub struct UiVertex { pub pos: [f32; 2], pub uv: [f32; 2], pub color: [u8; 4] }

#[derive(Copy, Clone, Default)]
pub struct GlyphRect {
    pub u0: f32, pub v0: f32, pub u1: f32, pub v1: f32,
    pub w: f32, pub h: f32, pub advance: f32, pub off_x: f32, pub off_y: f32,
}

// Faithful per-char loop, decoding UTF-8 via chars() like the engine.
#[no_mangle]
pub fn push_text(verts: &mut Vec<UiVertex>, text: &str, glyphs: &[GlyphRect; 128], pos: [f32; 2], color: [u8; 4]) {
    let mut x = pos[0];
    for ch in text.chars() {
        let g = glyphs[(ch as usize) & 127];
        let x0 = x + g.off_x;
        let y0 = pos[1] + g.off_y;
        let x1 = x0 + g.w;
        let y1 = y0 + g.h;
        verts.push(UiVertex { pos: [x0, y0], uv: [g.u0, g.v0], color });
        verts.push(UiVertex { pos: [x1, y0], uv: [g.u1, g.v0], color });
        verts.push(UiVertex { pos: [x1, y1], uv: [g.u1, g.v1], color });
        verts.push(UiVertex { pos: [x0, y1], uv: [g.u0, g.v1], color });
        x += g.advance;
    }
}

// ASCII fast path: operate on bytes, skipping UTF-8 decode for the common case.
#[no_mangle]
pub fn push_text_ascii(verts: &mut Vec<UiVertex>, text: &[u8], glyphs: &[GlyphRect; 128], pos: [f32; 2], color: [u8; 4]) {
    let mut x = pos[0];
    for &b in text {
        let g = glyphs[(b as usize) & 127];
        let x0 = x + g.off_x;
        let y0 = pos[1] + g.off_y;
        let x1 = x0 + g.w;
        let y1 = y0 + g.h;
        verts.push(UiVertex { pos: [x0, y0], uv: [g.u0, g.v0], color });
        verts.push(UiVertex { pos: [x1, y0], uv: [g.u1, g.v0], color });
        verts.push(UiVertex { pos: [x1, y1], uv: [g.u1, g.v1], color });
        verts.push(UiVertex { pos: [x0, y1], uv: [g.u0, g.v1], color });
        x += g.advance;
    }
}
