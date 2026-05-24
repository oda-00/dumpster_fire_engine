use thin_vec::ThinVec;

#[derive(Copy, Clone, Debug, Default)]
pub struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
}

#[derive(Clone, Debug, Default)]
pub struct UiInputState {
    pub cursor: [f32; 2],
    pub cursor_prev: [f32; 2],
    pub left_down: bool,
    pub left_just_pressed: bool,
    pub left_just_released: bool,
    pub right_down: bool,
    pub mods: Modifiers,
    pub scroll: [f32; 2],
    /// Unicode characters typed this frame (text input).
    pub chars: ThinVec<char>,
}

impl UiInputState {
    /// Per-frame state advance — called by UiManager::tick before any widget walks.
    pub fn end_frame(&mut self) {
        self.cursor_prev = self.cursor;
        self.left_just_pressed = false;
        self.left_just_released = false;
        self.scroll = [0.0, 0.0];
        self.chars.clear();
    }
}
