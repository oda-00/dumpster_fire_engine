#[derive(Clone, Debug)]
pub struct SliderData {
    pub value:     f32,
    pub min:       f32,
    pub max:       f32,
    pub step:      f32,
    pub on_change: Option<u32>,
    pub dragging:  bool,
}

impl SliderData {
    pub fn new(value: f32, min: f32, max: f32) -> Self {
        Self { value, min, max, step: 0.0, on_change: None, dragging: false }
    }

    /// Clamp value to [min, max] and quantize to step if step > 0.
    pub fn apply(&mut self, v: f32) {
        let clamped = v.clamp(self.min, self.max);
        self.value = if self.step > 0.0 {
            (clamped / self.step).round() * self.step
        } else {
            clamped
        };
    }
}
