use super::draw::DrawList;
use super::input::UiInputState;
use super::layout::Rect;

/// Immediate-mode UI builder. Appends geometry to a `DrawList` each frame
/// and hit-tests against the supplied `UiInputState` so widgets return
/// their interaction state directly (no second pass).
pub struct Ui<'a> {
    pub draw:   &'a mut DrawList,
    pub input:  UiInputState,
    pub cursor: [f32; 2],
    pub width:  f32,
}

impl<'a> Ui<'a> {
    pub fn new(draw: &'a mut DrawList, rect: Rect) -> Self {
        Self { draw, input: UiInputState::default(), cursor: [rect.x, rect.y], width: rect.w }
    }

    /// Construct with real input state — buttons / sliders / checkboxes
    /// react to the mouse via `input.cursor` + `input.left_just_pressed`.
    pub fn with_input(draw: &'a mut DrawList, rect: Rect, input: UiInputState) -> Self {
        Self { draw, input, cursor: [rect.x, rect.y], width: rect.w }
    }

    fn hovered(&self, x: f32, y: f32, w: f32, h: f32) -> bool {
        let cx = self.input.cursor[0];
        let cy = self.input.cursor[1];
        cx >= x && cx < x + w && cy >= y && cy < y + h
    }

    pub fn label(&mut self, _text: &str) -> &mut Self {
        self.draw.push_rect(self.cursor[0], self.cursor[1], self.width, 16.0,
            [0.0, 0.0, 1.0, 1.0], [200, 200, 200, 255]);
        self.cursor[1] += 18.0;
        self
    }

    /// Returns true if the button was clicked this frame.
    pub fn button(&mut self, _text: &str) -> bool {
        let x = self.cursor[0];
        let y = self.cursor[1];
        let w = self.width;
        let h = 24.0_f32;
        let hovered = self.hovered(x, y, w, h);
        let clicked = hovered && self.input.left_just_pressed;
        let color = if clicked { [120, 150, 200, 255] }
                    else if hovered { [100, 100, 150, 255] }
                    else { [80, 80, 120, 255] };
        self.draw.push_rect(x, y, w, h, [0.0, 0.0, 1.0, 1.0], color);
        self.cursor[1] += 26.0;
        clicked
    }

    /// Returns true if the slider value changed this frame.
    pub fn slider(&mut self, _label: &str, value: &mut f32, min: f32, max: f32) -> bool {
        let x = self.cursor[0];
        let y = self.cursor[1];
        let w = self.width;
        let h = 6.0_f32;
        let hovered = self.hovered(x, y - 4.0, w, h + 8.0);
        let dragging = hovered && self.input.left_down;
        let mut changed = false;
        if dragging {
            let t = ((self.input.cursor[0] - x) / w.max(1e-5)).clamp(0.0, 1.0);
            let new_v = min + t * (max - min);
            if (*value - new_v).abs() > 1e-6 {
                *value = new_v;
                changed = true;
            }
        }
        let t = ((*value - min) / (max - min).max(1e-5)).clamp(0.0, 1.0);
        self.draw.push_rect(x, y, w, h, [0.0, 0.0, 1.0, 1.0], [60, 60, 60, 255]);
        self.draw.push_rect(x, y, w * t, h, [0.0, 0.0, 1.0, 1.0], [80, 140, 200, 255]);
        self.cursor[1] += 20.0;
        changed
    }

    /// Returns true if the checkbox toggled this frame.
    pub fn checkbox(&mut self, _label: &str, checked: &mut bool) -> bool {
        let x = self.cursor[0];
        let y = self.cursor[1];
        let h = 16.0_f32;
        let hovered = self.hovered(x, y, h, h);
        let clicked = hovered && self.input.left_just_pressed;
        if clicked { *checked = !*checked; }
        let color = if *checked { [80, 200, 80, 255] } else { [80, 80, 80, 255] };
        self.draw.push_rect(x, y, h, h, [0.0, 0.0, 1.0, 1.0], color);
        self.cursor[1] += 20.0;
        clicked
    }

    pub fn separator(&mut self) -> &mut Self {
        self.cursor[1] += 4.0;
        self
    }
}
