//! Lights, sourced from KHR_lights_punctual. The 48-byte `LightBlock` is the
//! flat per-light record the `LightClustering` pipeline consumes.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LightKind {
    Directional,
    Point,
    Spot,
}

#[derive(Debug, Clone)]
pub struct Light {
    pub name:        Option<String>,
    pub kind:        LightKind,
    pub color:       [f32; 3],
    /// Luminous intensity (candela) for point/spot, illuminance (lux) for directional.
    pub intensity:   f32,
    /// Distance attenuation cutoff (0 = unbounded).
    pub range:       f32,
    /// Spot only: cone half-angles in radians.
    pub inner_cone:  f32,
    pub outer_cone:  f32,
}

/// Std140-friendly per-light record. 3 × vec4 = 48 bytes.
///
/// Layout:
///   vec4 position_kind   = xyz | kind as f32
///   vec4 direction_range = xyz | range
///   vec4 color_intensity = rgb | intensity
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct LightBlock {
    pub position_kind:   [f32; 4],
    pub direction_range: [f32; 4],
    pub color_intensity: [f32; 4],
}

impl LightBlock {
    pub const BYTES: usize = core::mem::size_of::<Self>();

    /// Build a per-light record. Position/direction come from the host's
    /// world transform of the node carrying the light; for directional lights
    /// `position` is unused, for point lights `direction` is unused.
    pub fn from_light(
        light:    &Light,
        position: [f32; 3],
        direction:[f32; 3],
    ) -> Self {
        let kind_f = light.kind as u32 as f32;
        Self {
            position_kind:   [position[0], position[1], position[2], kind_f],
            direction_range: [direction[0], direction[1], direction[2], light.range],
            color_intensity: [light.color[0], light.color[1], light.color[2], light.intensity],
        }
    }
}
