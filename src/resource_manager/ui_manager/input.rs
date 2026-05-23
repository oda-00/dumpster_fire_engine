#[derive(Clone, Debug, Default)]
pub struct UiInputState {
    pub cursor:           [f32; 2],
    pub left_down:        bool,
    pub left_just_pressed: bool,
    pub scroll:           [f32; 2],
}
