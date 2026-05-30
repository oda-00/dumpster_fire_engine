use crate::forge_master::ore::ForgeImage;
use crate::render::vulkan::VulkanContext;
use ash::vk;
use fontdue::{Font, FontSettings};
use thin_vec::ThinVec;

#[derive(Copy, Clone, Debug)]
pub struct GlyphRect {
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
    pub w: f32,
    pub h: f32,
    pub advance: f32,
    pub off_x: f32,
    pub off_y: f32,
}

pub struct FontAtlas {
    pub texture: ForgeImage,
    pub atlas_size: (u32, u32),
    /// Glyph cache sorted by `(char, size_px)` for O(log N) binary search —
    /// same pattern as the object-loader's sorted symbol table.
    glyph_cache: ThinVec<((char, u16), GlyphRect)>,
    fonts: Vec<Font>,
    next_x: u32,
    next_y: u32,
    row_height: u32,
}

impl FontAtlas {
    pub fn new(vulkan: &VulkanContext) -> Self {
        // DejaVu Sans Mono — a real, freely-licensed monospace TTF. (Replaces the
        // previously-bundled FiraCode-Regular.ttf, which was actually a saved HTML
        // page, not a font, and panicked here at runtime.)
        let default_font_bytes: &[u8] = include_bytes!("../../../assets/fonts/DejaVuSansMono.ttf");
        let font = Font::from_bytes(default_font_bytes, FontSettings::default())
            .expect("bundled DejaVuSansMono.ttf failed to parse");

        let texture = ForgeImage::create_2d(
            &vulkan.device,
            &vulkan.memory_properties,
            1024,
            1024,
            vk::Format::R8G8B8A8_UNORM,
            vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        )
        .unwrap();

        Self {
            texture,
            atlas_size: (1024, 1024),
            glyph_cache: ThinVec::new(),
            fonts: vec![font],
            next_x: 0,
            next_y: 0,
            row_height: 0,
        }
    }

    pub fn get_glyph(&mut self, ch: char, size: u16) -> GlyphRect {
        let key = (ch, size);
        let pos = self.glyph_cache.partition_point(|&(k, _)| k < key);
        if self.glyph_cache.get(pos).map(|&(k, _)| k) == Some(key) {
            return self.glyph_cache[pos].1;
        }

        let (metrics, _bitmap) = self.fonts[0].rasterize(ch, size as f32);
        let w = metrics.width as u32;
        let h = metrics.height as u32;

        if w == 0 || h == 0 {
            let rect = GlyphRect {
                u0: 0.0,
                v0: 0.0,
                u1: 0.0,
                v1: 0.0,
                w: 0.0,
                h: 0.0,
                advance: metrics.advance_width,
                off_x: metrics.xmin as f32,
                off_y: -metrics.ymin as f32,
            };
            self.glyph_cache.insert(pos, (key, rect));
            return rect;
        }

        if self.next_x + w > self.atlas_size.0 {
            self.next_x = 0;
            self.next_y += self.row_height;
            self.row_height = 0;
        }

        if self.next_y + h > self.atlas_size.1 {
            self.grow_atlas();
        }

        let u0 = self.next_x as f32 / self.atlas_size.0 as f32;
        let v0 = self.next_y as f32 / self.atlas_size.1 as f32;
        let u1 = (self.next_x + w) as f32 / self.atlas_size.0 as f32;
        let v1 = (self.next_y + h) as f32 / self.atlas_size.1 as f32;

        let rect = GlyphRect {
            u0,
            v0,
            u1,
            v1,
            w: w as f32,
            h: h as f32,
            advance: metrics.advance_width,
            off_x: metrics.xmin as f32,
            off_y: -metrics.ymin as f32,
        };

        // Insert maintaining sorted order (glyphs are typically added in text-scan
        // order so pos ≈ len — amortised O(1) push, O(N) worst case shift).
        self.glyph_cache.insert(pos, (key, rect));
        self.next_x += w;
        self.row_height = self.row_height.max(h);

        rect
    }

    fn grow_atlas(&mut self) {
        self.atlas_size = (self.atlas_size.0 * 2, self.atlas_size.1 * 2);
        self.next_x = 0;
        self.next_y = 0;
        self.row_height = 0;
        self.glyph_cache.clear();
    }
}
