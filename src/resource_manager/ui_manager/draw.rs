use thin_vec::ThinVec;

/// UV passed to `push_rect` for opaque solid-color quads.
/// All four vertices share (0,0) so every fragment triggers the shader's
/// degenerate-UV bypass and uses vertex color directly (no atlas sample).
pub const SOLID: [f32; 4] = [0.0, 0.0, 0.0, 0.0];

// Unified with the canonical UI vertex — one `#[repr(C)]` UiVertex type across
// the engine (GUI_research.md Phase 6 consolidation). Identical memory layout,
// so the GPU vertex-attribute bindings in overlay/backend are unaffected. Path
// kept as a re-export so existing `ui_manager::draw::UiVertex` users are unchanged.
pub use crate::render::ui_render::vertex::UiVertex;

pub struct DrawList {
    pub vertices: ThinVec<UiVertex>,
    pub indices: ThinVec<u32>,
}

impl DrawList {
    pub fn new() -> Self {
        Self {
            vertices: ThinVec::new(),
            indices: ThinVec::new(),
        }
    }

    pub fn clear(&mut self) {
        self.vertices.clear();
        self.indices.clear();
    }

    pub fn push_rect(&mut self, x: f32, y: f32, w: f32, h: f32, uv: [f32; 4], color: [u8; 4]) {
        let base = self.vertices.len() as u32;
        self.vertices.extend_from_slice(&[
            UiVertex {
                pos: [x, y],
                uv: [uv[0], uv[1]],
                color,
            },
            UiVertex {
                pos: [x + w, y],
                uv: [uv[2], uv[1]],
                color,
            },
            UiVertex {
                pos: [x + w, y + h],
                uv: [uv[2], uv[3]],
                color,
            },
            UiVertex {
                pos: [x, y + h],
                uv: [uv[0], uv[3]],
                color,
            },
        ]);
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    pub fn push_panel_bg(&mut self, x: f32, y: f32, w: f32, h: f32, color: [u8; 4]) {
        self.push_rect(x, y, w, h, SOLID, color);
    }

    pub fn push_title_bar(&mut self, x: f32, y: f32, w: f32, h: f32, bg: [u8; 4], sep: [u8; 4]) {
        self.push_rect(x, y, w, h, SOLID, bg);
        self.push_line(x, y + h, x + w, y + h, 1.0, sep);
    }

    pub fn push_vsep(&mut self, x: f32, y: f32, h: f32, color: [u8; 4]) {
        self.push_line(x, y, x, y + h, 1.5, color);
    }

    pub fn push_hsep(&mut self, x: f32, y: f32, w: f32, color: [u8; 4]) {
        self.push_line(x, y, x + w, y, 1.5, color);
    }

    /// Push an arbitrary-angle line segment as a rotated thin quad.
    /// `thickness` is the perpendicular width in pixels.
    pub fn push_line(
        &mut self,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        thickness: f32,
        color: [u8; 4],
    ) {
        let dx = x1 - x0;
        let dy = y1 - y0;
        let len = (dx * dx + dy * dy).sqrt().max(1e-5);
        // Perpendicular unit vector × half thickness.
        let px = -dy / len * thickness * 0.5;
        let py = dx / len * thickness * 0.5;
        let base = self.vertices.len() as u32;
        self.vertices.extend_from_slice(&[
            UiVertex {
                pos: [x0 + px, y0 + py],
                uv: [0.0, 0.0],
                color,
            },
            UiVertex {
                pos: [x1 + px, y1 + py],
                uv: [0.0, 0.0],
                color,
            },
            UiVertex {
                pos: [x1 - px, y1 - py],
                uv: [0.0, 0.0],
                color,
            },
            UiVertex {
                pos: [x0 - px, y0 - py],
                uv: [0.0, 0.0],
                color,
            },
        ]);
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

impl Default for DrawList {
    fn default() -> Self {
        Self::new()
    }
}
