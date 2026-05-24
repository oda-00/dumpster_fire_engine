//! UI pipeline descriptor — captures the contract the overlay renderpass
//! consumes. Engine forge code reads these constants to build the actual
//! `vk::Pipeline`.
//!
//! Layout:
//!   - Vertex format: pos(vec2) + uv(vec2) + color(u8x4 unorm), stride 20 B.
//!   - Premultiplied-alpha blend.
//!   - Depth test/write OFF (UI always on top).
//!   - Dynamic scissor (per-panel clipping).
//!   - Single descriptor set (set 0 binding 0) sampling the font atlas.

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct UiPushConstants {
    pub screen_w: f32,
    pub screen_h: f32,
    pub _pad: [f32; 2],
}

pub struct UiPipeline {
    pub screen_w: f32,
    pub screen_h: f32,
}

impl UiPipeline {
    pub fn new(screen_w: f32, screen_h: f32) -> Self {
        Self { screen_w, screen_h }
    }

    pub fn push(&self) -> UiPushConstants {
        UiPushConstants {
            screen_w: self.screen_w,
            screen_h: self.screen_h,
            _pad: [0.0; 2],
        }
    }
}
