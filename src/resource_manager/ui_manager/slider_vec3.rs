#[derive(Clone, Debug)]
pub struct SliderVec3Data {
    pub value: [f32; 3],
    pub min: f32,
    pub max: f32,
    pub on_change: Option<u32>,
    pub dragging_axis: Option<u8>,
}

impl SliderVec3Data {
    pub fn new(value: [f32; 3], min: f32, max: f32) -> Self {
        Self {
            value,
            min,
            max,
            on_change: None,
            dragging_axis: None,
        }
    }

    pub fn apply_axis(&mut self, axis: u8, v: f32) {
        if (axis as usize) < 3 {
            self.value[axis as usize] = v.clamp(self.min, self.max);
        }
    }
}
