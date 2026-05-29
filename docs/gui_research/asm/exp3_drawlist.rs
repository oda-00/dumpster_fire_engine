//! Experiment 3 — Hot drawlist quad emission, AoS vs SoA vertex layout.
//!
//! Mirrors `DrawList::push_rect` (src/render/ui_render/drawlist.rs) and its
//! 20-byte `#[repr(C)]` UiVertex { pos:[f32;2], uv:[f32;2], color:[u8;4] }.
//! Question: does the 4-vertex emit auto-vectorize, and what does the odd
//! 20-byte stride cost vs an SoA layout?
//!
//! Build: rustc -O --emit asm --crate-type=lib exp3_drawlist.rs

#[repr(C)]
#[derive(Copy, Clone)]
pub struct UiVertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub color: [u8; 4],
}

#[derive(Copy, Clone)]
pub struct Rect { pub x: f32, pub y: f32, pub w: f32, pub h: f32 }

// AoS push_rect, faithful to the engine.
#[no_mangle]
pub fn push_rect_aos(verts: &mut Vec<UiVertex>, idx: &mut Vec<u32>, rect: Rect, color: [u8; 4]) {
    let base = verts.len() as u32;
    let (x0, y0) = (rect.x, rect.y);
    let (x1, y1) = (rect.x + rect.w, rect.y + rect.h);
    verts.extend_from_slice(&[
        UiVertex { pos: [x0, y0], uv: [0.0, 0.0], color },
        UiVertex { pos: [x1, y0], uv: [0.0, 0.0], color },
        UiVertex { pos: [x1, y1], uv: [0.0, 0.0], color },
        UiVertex { pos: [x0, y1], uv: [0.0, 0.0], color },
    ]);
    idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

// SoA alternative: positions packed separately so a run of quads is a flat
// f32 stream the compiler can vectorize and the GPU can read with a tight stride.
#[derive(Default)]
pub struct SoaVerts {
    pub pos: Vec<f32>,   // x0,y0,x1,y0,...
    pub uv: Vec<f32>,
    pub color: Vec<u32>, // packed RGBA8
}

#[no_mangle]
pub fn push_rect_soa(v: &mut SoaVerts, rect: Rect, color: u32) {
    let (x0, y0) = (rect.x, rect.y);
    let (x1, y1) = (rect.x + rect.w, rect.y + rect.h);
    v.pos.extend_from_slice(&[x0, y0, x1, y0, x1, y1, x0, y1]);
    v.uv.extend_from_slice(&[0.0; 8]);
    v.color.extend_from_slice(&[color; 4]);
}

// Bulk: many quads in a row (a typical panel of widgets). Reveals whether the
// per-quad work hoists out of the loop and vectorizes.
#[no_mangle]
pub fn push_rects_bulk(verts: &mut Vec<UiVertex>, rects: &[Rect], color: [u8; 4]) {
    for &r in rects {
        let (x0, y0) = (r.x, r.y);
        let (x1, y1) = (r.x + r.w, r.y + r.h);
        verts.push(UiVertex { pos: [x0, y0], uv: [0.0, 0.0], color });
        verts.push(UiVertex { pos: [x1, y0], uv: [0.0, 0.0], color });
        verts.push(UiVertex { pos: [x1, y1], uv: [0.0, 0.0], color });
        verts.push(UiVertex { pos: [x0, y1], uv: [0.0, 0.0], color });
    }
}
