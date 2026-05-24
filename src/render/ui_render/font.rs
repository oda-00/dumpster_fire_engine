use fontdue::{Font, FontSettings};
use hashbrown::HashMap;
use ash::vk;
use crate::forge_master::ore::ForgeImage;
use crate::render::vulkan::VulkanContext;

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
    glyph_map: HashMap<(char, u16), GlyphRect>,
    fonts: Vec<Font>,
    next_x: u32,
    next_y: u32,
    row_height: u32,
}

impl FontAtlas {
    pub fn new(vulkan: &VulkanContext) -> Self {
        let default_font_bytes = include_bytes!("../../assets/fonts/FiraCode-Regular.ttf");
        let font = Font::from_bytes(default_font_bytes, FontSettings::default())
            .unwrap_or_else(|_| {
                Font::from_bytes(&[], FontSettings::default()).unwrap()
            });

        let texture = ForgeImage::create_2d(
            &vulkan.device,
            &vulkan.memory_properties,
            1024,
            1024,
            vk::Format::R8G8B8A8_UNORM,
            vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        ).unwrap();

        Self {
            texture,
            atlas_size: (1024, 1024),
            glyph_map: HashMap::new(),
            fonts: vec![font],
            next_x: 0,
            next_y: 0,
            row_height: 0,
        }
    }

    pub fn get_glyph(&mut self, ch: char, size: u16) -> GlyphRect {
        if let Some(rect) = self.glyph_map.get(&(ch, size)) {
            return *rect;
        }

        let (metrics, _bitmap) = self.fonts[0].rasterize(ch, size);
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
            self.glyph_map.insert((ch, size), rect);
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

        self.glyph_map.insert((ch, size), rect);
        self.next_x += w;
        self.row_height = self.row_height.max(h);

        rect
    }

    fn grow_atlas(&mut self) {
        self.atlas_size = (self.atlas_size.0 * 2, self.atlas_size.1 * 2);
        self.next_x = 0;
        self.next_y = 0;
        self.row_height = 0;
        self.glyph_map.clear();
    }
}
