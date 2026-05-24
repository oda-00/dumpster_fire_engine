use thin_vec::ThinVec;

#[derive(Copy, Clone, Debug, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

#[derive(Copy, Clone, Debug)]
pub enum Sizing {
    Fill,
    Fixed(f32),
    Hug,
}

#[derive(Copy, Clone, Debug)]
pub enum Axis {
    Row,
    Column,
}

#[derive(Copy, Clone, Debug)]
pub enum Align {
    Start,
    Center,
    End,
}

#[derive(Copy, Clone, Debug, Default)]
pub struct Padding {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

#[derive(Copy, Clone, Debug)]
pub struct LayoutSpec {
    pub axis: Axis,
    pub gap: f32,
    pub padding: Padding,
    pub align: Align,
}

impl Default for LayoutSpec {
    fn default() -> Self {
        Self {
            axis: Axis::Column,
            gap: 4.0,
            padding: Padding::default(),
            align: Align::Start,
        }
    }
}

/// Flex layout. Distributes `Sizing::Fill` children across remaining space
/// after `Fixed` and `Hug` children claim their share. Returns sized rects
/// in child order.
pub fn measure_and_place(
    rect: Rect,
    spec: LayoutSpec,
    sizes: &[Sizing],
    hugs: &[f32], // intrinsic size used by Hug children (ignored otherwise)
) -> ThinVec<Rect> {
    let mut out = ThinVec::with_capacity(sizes.len());
    let pad = spec.padding;
    let inner_x = rect.x + pad.left;
    let inner_y = rect.y + pad.top;
    let inner_w = (rect.w - pad.left - pad.right).max(0.0);
    let inner_h = (rect.h - pad.top - pad.bottom).max(0.0);

    let main_axis_extent = match spec.axis {
        Axis::Row => inner_w,
        Axis::Column => inner_h,
    };
    let n = sizes.len() as f32;
    let gaps = if n > 1.0 { spec.gap * (n - 1.0) } else { 0.0 };

    let mut fixed_total = 0.0_f32;
    let mut fill_count = 0_usize;
    for (i, s) in sizes.iter().enumerate() {
        match *s {
            Sizing::Fixed(v) => fixed_total += v,
            Sizing::Hug => fixed_total += hugs.get(i).copied().unwrap_or(0.0),
            Sizing::Fill => fill_count += 1,
        }
    }
    let remaining = (main_axis_extent - fixed_total - gaps).max(0.0);
    let fill_each = if fill_count > 0 {
        remaining / fill_count as f32
    } else {
        0.0
    };

    let mut cursor_main = 0.0_f32;
    for (i, s) in sizes.iter().enumerate() {
        let main = match *s {
            Sizing::Fixed(v) => v,
            Sizing::Hug => hugs.get(i).copied().unwrap_or(0.0),
            Sizing::Fill => fill_each,
        };
        let r = match spec.axis {
            Axis::Row => Rect {
                x: inner_x + cursor_main,
                y: inner_y,
                w: main,
                h: inner_h,
            },
            Axis::Column => Rect {
                x: inner_x,
                y: inner_y + cursor_main,
                w: inner_w,
                h: main,
            },
        };
        out.push(r);
        cursor_main += main + spec.gap;
    }
    out
}
