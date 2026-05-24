use super::button::{ButtonData, ButtonState};
use super::checkbox::CheckboxData;
use super::draw::{self, DrawList};
use super::font;
use super::input::UiInputState;
use super::layout::Rect;

/// Immediate-mode UI builder. Appends geometry to a `DrawList` each frame
/// and hit-tests against the supplied `UiInputState` so widgets return
/// their interaction state directly (no second pass).
pub struct Ui<'a> {
    pub draw: &'a mut DrawList,
    pub input: UiInputState,
    pub cursor: [f32; 2],
    pub width: f32,
}

impl<'a> Ui<'a> {
    pub fn new(draw: &'a mut DrawList, rect: Rect) -> Self {
        Self {
            draw,
            input: UiInputState::default(),
            cursor: [rect.x, rect.y],
            width: rect.w,
        }
    }

    pub fn with_input(draw: &'a mut DrawList, rect: Rect, input: UiInputState) -> Self {
        Self {
            draw,
            input,
            cursor: [rect.x, rect.y],
            width: rect.w,
        }
    }

    fn hovered(&self, x: f32, y: f32, w: f32, h: f32) -> bool {
        let cx = self.input.cursor[0];
        let cy = self.input.cursor[1];
        cx >= x && cx < x + w && cy >= y && cy < y + h
    }

    // ── Text rendering ────────────────────────────────────────────────────────

    /// Draw text at the current cursor, advancing cursor[1] after.
    pub fn label(&mut self, text: &str) -> &mut Self {
        self.text_at(self.cursor[0], self.cursor[1], text, [210, 210, 220, 255]);
        self.cursor[1] += font::GLYPH_H as f32 + 2.0;
        self
    }

    /// Draw colored text at the current cursor, advancing cursor[1] after.
    pub fn text(&mut self, text: &str, color: [u8; 4]) -> &mut Self {
        self.text_at(self.cursor[0], self.cursor[1], text, color);
        self.cursor[1] += font::GLYPH_H as f32 + 2.0;
        self
    }

    /// Draw colored text at an explicit position without moving cursor.
    pub fn text_at(&mut self, x0: f32, y: f32, text: &str, color: [u8; 4]) {
        let gw = font::GLYPH_W as f32;
        let gh = font::GLYPH_H as f32;
        let max_x = x0 + self.width;
        let mut cx = x0;
        for c in text.chars() {
            if cx + gw > max_x {
                break;
            }
            if c != ' ' {
                let uv = font::glyph_rect(c);
                if uv != [0.0_f32; 4] {
                    self.draw.push_rect(cx, y, gw, gh, uv, color);
                }
            }
            cx += gw;
        }
    }

    // ── Vertical-layout widgets (panels, inspector) ───────────────────────────

    /// Returns true if the button was clicked this frame.
    pub fn button(&mut self, text: &str) -> bool {
        let x = self.cursor[0];
        let y = self.cursor[1];
        let w = self.width;
        let h = 24.0_f32;
        let hovered = self.hovered(x, y, w, h);
        let clicked = hovered && self.input.left_just_pressed;
        let color = if clicked {
            [120, 150, 200, 255]
        } else if hovered {
            [100, 100, 150, 255]
        } else {
            [70, 70, 105, 255]
        };
        self.draw.push_rect(x, y, w, h, draw::SOLID, color);
        self.text_at(x + 4.0, y + 4.0, text, [210, 210, 220, 255]);
        self.cursor[1] += 26.0;
        clicked
    }

    /// Button that reads hover/pressed state from retained `ButtonData`.
    pub fn button_retained(&mut self, text: &str, data: &mut ButtonData) -> bool {
        let x = self.cursor[0];
        let y = self.cursor[1];
        let w = self.width;
        let h = 24.0_f32;
        data.last_rect = Rect { x, y, w, h };
        let hovered = self.hovered(x, y, w, h);
        let clicked = hovered && self.input.left_just_pressed;
        let color = match data.state {
            ButtonState::Pressed => [120, 150, 200, 255],
            ButtonState::Hovered => [100, 100, 155, 255],
            ButtonState::Idle => [70, 70, 105, 255],
        };
        self.draw.push_rect(x, y, w, h, draw::SOLID, color);
        self.text_at(x + 4.0, y + 4.0, text, [210, 210, 220, 255]);
        self.cursor[1] += 26.0;
        clicked
    }

    /// Returns true if the slider value changed this frame.
    pub fn slider(&mut self, label: &str, value: &mut f32, min: f32, max: f32) -> bool {
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
        self.draw
            .push_rect(x, y, w, h, draw::SOLID, [50, 50, 60, 255]);
        self.draw
            .push_rect(x, y, w * t, h, draw::SOLID, [70, 130, 195, 255]);
        // Label to the right of the track (tiny, 8px glyphs)
        self.text_at(x + 2.0, y - 9.0, label, [160, 160, 180, 200]);
        self.cursor[1] += 20.0;
        changed
    }

    /// Returns true if the checkbox toggled this frame.
    pub fn checkbox(&mut self, label: &str, checked: &mut bool) -> bool {
        let x = self.cursor[0];
        let y = self.cursor[1];
        let h = 16.0_f32;
        let hovered = self.hovered(x, y, h, h);
        let clicked = hovered && self.input.left_just_pressed;
        if clicked {
            *checked = !*checked;
        }
        let color = if *checked {
            [80, 200, 80, 255]
        } else {
            [80, 80, 80, 255]
        };
        self.draw.push_rect(x, y, h, h, draw::SOLID, color);
        self.text_at(x + h + 4.0, y, label, [190, 190, 200, 255]);
        self.cursor[1] += 20.0;
        clicked
    }

    /// Checkbox that reads/writes retained `CheckboxData` and sets `last_rect`.
    pub fn checkbox_retained(&mut self, label: &str, data: &mut CheckboxData) -> bool {
        let x = self.cursor[0];
        let y = self.cursor[1];
        let sz = 16.0_f32;
        data.last_rect = Rect { x, y, w: sz, h: sz };
        let color = if data.checked {
            [80, 200, 80, 255]
        } else {
            [80, 80, 80, 255]
        };
        self.draw.push_rect(x, y, sz, sz, draw::SOLID, color);
        self.text_at(x + sz + 4.0, y, label, [190, 190, 200, 255]);
        self.cursor[1] += 20.0;
        data.checked
    }

    pub fn separator(&mut self) -> &mut Self {
        self.cursor[1] += 4.0;
        self
    }

    /// Section header bar with bottom separator line and white label.
    pub fn section_header(&mut self, text: &str) -> &mut Self {
        let x = self.cursor[0];
        let y = self.cursor[1];
        self.draw
            .push_rect(x, y, self.width, 18.0, draw::SOLID, [42, 42, 54, 255]);
        self.draw.push_line(
            x,
            y + 18.0,
            x + self.width,
            y + 18.0,
            1.0,
            [68, 68, 84, 255],
        );
        self.text_at(x + 4.0, y + 1.0, text, [180, 180, 210, 255]);
        self.cursor[1] += 20.0;
        self
    }

    /// Collapsible header — toggles `*collapsed` on click, returns true when expanded.
    pub fn collapsible_header(&mut self, text: &str, collapsed: &mut bool) -> bool {
        let x = self.cursor[0];
        let y = self.cursor[1];
        let w = self.width;
        let h = 18.0_f32;
        if self.hovered(x, y, w, h) && self.input.left_just_pressed {
            *collapsed = !*collapsed;
        }
        let bg = if self.hovered(x, y, w, h) {
            [52, 52, 66, 255]
        } else {
            [42, 42, 54, 255]
        };
        self.draw.push_rect(x, y, w, h, draw::SOLID, bg);
        // Arrow indicator
        let arrow_col = if *collapsed {
            [110, 110, 130, 255]
        } else {
            [170, 170, 200, 255]
        };
        self.draw
            .push_rect(x + 4.0, y + 5.0, 8.0, 8.0, draw::SOLID, arrow_col);
        self.draw
            .push_line(x, y + h, x + w, y + h, 1.0, [68, 68, 84, 255]);
        self.text_at(x + 16.0, y + 1.0, text, [170, 170, 200, 255]);
        self.cursor[1] += 20.0;
        !*collapsed
    }

    // ── Horizontal-layout widgets (toolbar) ───────────────────────────────────

    /// Fixed-width button that advances `cursor[0]` — for horizontal toolbars.
    pub fn hbutton(&mut self, text: &str, w: f32) -> bool {
        let x = self.cursor[0];
        let y = self.cursor[1];
        let h = 20.0_f32;
        let hovered = self.hovered(x, y, w, h);
        let clicked = hovered && self.input.left_just_pressed;
        let color = if clicked {
            [120, 150, 200, 255]
        } else if hovered {
            [100, 100, 150, 255]
        } else {
            [60, 60, 85, 255]
        };
        self.draw.push_rect(x, y, w, h, draw::SOLID, color);
        // Center text horizontally in the button
        let text_w = text.chars().count() as f32 * font::GLYPH_W as f32;
        let tx = x + ((w - text_w) * 0.5).max(2.0);
        self.text_at(tx, y + 2.0, text, [210, 210, 220, 255]);
        self.cursor[0] += w + 4.0;
        clicked
    }

    /// Fixed-width checkbox that advances `cursor[0]` — for horizontal toolbars.
    pub fn hcheckbox(&mut self, label: &str, checked: &mut bool) -> bool {
        let x = self.cursor[0];
        let y = self.cursor[1];
        let sz = 20.0_f32;
        let hovered = self.hovered(x, y, sz, sz);
        let clicked = hovered && self.input.left_just_pressed;
        if clicked {
            *checked = !*checked;
        }
        let color = if *checked {
            [80, 200, 80, 255]
        } else {
            [60, 60, 80, 255]
        };
        self.draw.push_rect(x, y, sz, sz, draw::SOLID, color);
        // Short label beside the box
        self.text_at(x + sz + 2.0, y + 2.0, label, [190, 190, 200, 255]);
        let label_w = label.chars().count() as f32 * font::GLYPH_W as f32;
        self.cursor[0] += sz + 2.0 + label_w + 4.0;
        clicked
    }

    /// Colored indicator tile — draws a solid rect, advances cursor[0]. No click.
    pub fn htile(&mut self, w: f32, h: f32, color: [u8; 4]) {
        self.draw
            .push_rect(self.cursor[0], self.cursor[1], w, h, draw::SOLID, color);
        self.cursor[0] += w;
    }

    /// Horizontal gap — advances cursor[0] without drawing.
    pub fn hgap(&mut self, px: f32) {
        self.cursor[0] += px;
    }
}
