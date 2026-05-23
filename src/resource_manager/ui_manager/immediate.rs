use super::draw::DrawList;
use super::layout::Rect;

/// Immediate-mode UI builder. Appends geometry to a `DrawList` each frame.
pub struct Ui<'a> {
    pub draw:   &'a mut DrawList,
    pub cursor: [f32; 2],
    pub width:  f32,
}

impl<'a> Ui<'a> {
    pub fn new(draw: &'a mut DrawList, rect: Rect) -> Self {
        Self { draw, cursor: [rect.x, rect.y], width: rect.w }
    }

    pub fn label(&mut self, _text: &str) -> &mut Self {
        self.draw.push_rect(self.cursor[0], self.cursor[1], self.width, 16.0,
            [0.0, 0.0, 1.0, 1.0], [200, 200, 200, 255]);
        self.cursor[1] += 18.0;
        self
    }

    /// Returns true if the button was clicked this frame.
    pub fn button(&mut self, _text: &str) -> bool {
        self.draw.push_rect(self.cursor[0], self.cursor[1], self.width, 24.0,
            [0.0, 0.0, 1.0, 1.0], [80, 80, 120, 255]);
        self.cursor[1] += 26.0;
        false
    }

    pub fn slider(&mut self, _label: &str, value: &mut f32, min: f32, max: f32) -> &mut Self {
        let t = (*value - min) / (max - min).max(1e-5);
        self.draw.push_rect(self.cursor[0], self.cursor[1], self.width, 6.0,
            [0.0, 0.0, 1.0, 1.0], [60, 60, 60, 255]);
        self.draw.push_rect(self.cursor[0], self.cursor[1], self.width * t, 6.0,
            [0.0, 0.0, 1.0, 1.0], [80, 140, 200, 255]);
        self.cursor[1] += 20.0;
        self
    }

    pub fn checkbox(&mut self, _label: &str, checked: &mut bool) -> &mut Self {
        let color = if *checked { [80, 200, 80, 255] } else { [80, 80, 80, 255] };
        self.draw.push_rect(self.cursor[0], self.cursor[1], 16.0, 16.0,
            [0.0, 0.0, 1.0, 1.0], color);
        self.cursor[1] += 20.0;
        self
    }

    pub fn separator(&mut self) -> &mut Self {
        self.cursor[1] += 4.0;
        self
    }
}
