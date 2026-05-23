#[derive(Copy, Clone, Debug, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Copy, Clone, Debug)]
pub enum Sizing { Fill, Fixed(f32), Hug }

#[derive(Copy, Clone, Debug)]
pub enum Axis { Row, Column }

#[derive(Copy, Clone, Debug)]
pub enum Align { Start, Center, End }
