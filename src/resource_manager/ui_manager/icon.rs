#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IconId(pub u32);

#[derive(Clone, Debug)]
pub struct IconData {
    pub icon_id: IconId,
    pub tint: [f32; 4],
}

impl IconData {
    pub fn new(icon_id: IconId) -> Self {
        Self {
            icon_id,
            tint: [1.0; 4],
        }
    }
}
