use crate::render::ui_core::layout::Rect;
use crate::render::ui_render::vertex::UiVertex;
use thin_vec::ThinVec;

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

    pub fn push_rect(&mut self, rect: Rect, color: [u8; 4]) {
        let base = self.vertices.len() as u32;
        let x0 = rect.x;
        let y0 = rect.y;
        let x1 = rect.x + rect.w;
        let y1 = rect.y + rect.h;

        self.vertices.extend_from_slice(&[
            UiVertex {
                pos: [x0, y0],
                uv: [0., 0.],
                color,
            },
            UiVertex {
                pos: [x1, y0],
                uv: [0., 0.],
                color,
            },
            UiVertex {
                pos: [x1, y1],
                uv: [0., 0.],
                color,
            },
            UiVertex {
                pos: [x0, y1],
                uv: [0., 0.],
                color,
            },
        ]);
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    pub fn push_line(&mut self, from: [f32; 2], to: [f32; 2], thickness: f32, color: [u8; 4]) {
        let dx = to[0] - from[0];
        let dy = to[1] - from[1];
        let len = (dx * dx + dy * dy).sqrt().max(1e-5);
        let px = -dy / len * thickness * 0.5;
        let py = dx / len * thickness * 0.5;

        let base = self.vertices.len() as u32;
        self.vertices.extend_from_slice(&[
            UiVertex {
                pos: [from[0] + px, from[1] + py],
                uv: [0., 0.],
                color,
            },
            UiVertex {
                pos: [to[0] + px, to[1] + py],
                uv: [0., 0.],
                color,
            },
            UiVertex {
                pos: [to[0] - px, to[1] - py],
                uv: [0., 0.],
                color,
            },
            UiVertex {
                pos: [from[0] - px, from[1] - py],
                uv: [0., 0.],
                color,
            },
        ]);
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    pub fn push_text(
        &mut self,
        text: &str,
        font: &super::font::FontAtlas,
        pos: [f32; 2],
        color: [u8; 4],
    ) {
        let mut x = pos[0];
        for ch in text.chars() {
            let g = font.get_glyph(ch, 14);
            let w = g.w;
            let h = g.h;
            let uv = [g.u0, g.v0, g.u1, g.v1];

            let base = self.vertices.len() as u32;
            self.vertices.extend_from_slice(&[
                UiVertex {
                    pos: [x + g.off_x, pos[1] + g.off_y],
                    uv: [uv[0], uv[1]],
                    color,
                },
                UiVertex {
                    pos: [x + g.off_x + w, pos[1] + g.off_y],
                    uv: [uv[2], uv[1]],
                    color,
                },
                UiVertex {
                    pos: [x + g.off_x + w, pos[1] + g.off_y + h],
                    uv: [uv[2], uv[3]],
                    color,
                },
                UiVertex {
                    pos: [x + g.off_x, pos[1] + g.off_y + h],
                    uv: [uv[0], uv[3]],
                    color,
                },
            ]);
            self.indices
                .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
            x += g.advance;
        }
    }
}

impl Default for DrawList {
    fn default() -> Self {
        Self::new()
    }
}
