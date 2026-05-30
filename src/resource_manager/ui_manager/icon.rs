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

/// Named, atlas-baked UI icons. Each variant maps to one bundled open-source
/// Lucide SVG (`assets/icons/lucide/*.svg`, ISC-licensed) that the handrolled
/// [`crate::resource_manager::ui_manager::vector`] rasterizer bakes into the
/// font atlas at startup. `Icon as u32` is the atlas slot index, exposed as
/// [`IconId`] via [`Icon::id`]; the order here defines the packing order in the
/// atlas and must stay stable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum Icon {
    Select = 0,
    Move,
    RotateGizmo,
    Scale,
    EditMode,
    Grid,
    Copy,
    Trash,
    Undo,
    Redo,
    Add,
    Remove,
    Eye,
    EyeOff,
    Layers,
    Axis,
    Grip,
    Pen,
    Ruler,
    Settings,
}

impl Icon {
    /// All icons in atlas-slot order.
    pub const ALL: [Icon; 20] = [
        Icon::Select,
        Icon::Move,
        Icon::RotateGizmo,
        Icon::Scale,
        Icon::EditMode,
        Icon::Grid,
        Icon::Copy,
        Icon::Trash,
        Icon::Undo,
        Icon::Redo,
        Icon::Add,
        Icon::Remove,
        Icon::Eye,
        Icon::EyeOff,
        Icon::Layers,
        Icon::Axis,
        Icon::Grip,
        Icon::Pen,
        Icon::Ruler,
        Icon::Settings,
    ];

    #[inline]
    pub const fn slot(self) -> u32 {
        self as u32
    }

    #[inline]
    pub const fn id(self) -> IconId {
        IconId(self as u32)
    }

    /// Virtual path of this icon's SVG in the asset [`crate::vfs::Vfs`]
    /// (matches the on-disk layout under `assets/`, so a `Dir` mount overrides
    /// the embedded copy for hot-reload / modding).
    pub const fn vpath(self) -> &'static str {
        match self {
            Icon::Select => "icons/lucide/mouse-pointer-2.svg",
            Icon::Move => "icons/lucide/move.svg",
            Icon::RotateGizmo => "icons/lucide/rotate-3d.svg",
            Icon::Scale => "icons/lucide/scale-3d.svg",
            Icon::EditMode => "icons/lucide/box.svg",
            Icon::Grid => "icons/lucide/grid-3x3.svg",
            Icon::Copy => "icons/lucide/copy.svg",
            Icon::Trash => "icons/lucide/trash-2.svg",
            Icon::Undo => "icons/lucide/undo-2.svg",
            Icon::Redo => "icons/lucide/redo-2.svg",
            Icon::Add => "icons/lucide/plus.svg",
            Icon::Remove => "icons/lucide/minus.svg",
            Icon::Eye => "icons/lucide/eye.svg",
            Icon::EyeOff => "icons/lucide/eye-off.svg",
            Icon::Layers => "icons/lucide/layers.svg",
            Icon::Axis => "icons/lucide/axis-3d.svg",
            Icon::Grip => "icons/lucide/grip.svg",
            Icon::Pen => "icons/lucide/pen-tool.svg",
            Icon::Ruler => "icons/lucide/ruler.svg",
            Icon::Settings => "icons/lucide/settings.svg",
        }
    }

    /// The embedded SVG source as raw bytes (for the VFS embedded registry).
    pub const fn svg_bytes(self) -> &'static [u8] {
        self.svg().as_bytes()
    }

    /// The raw SVG source, embedded at compile time.
    pub const fn svg(self) -> &'static str {
        match self {
            Icon::Select => include_str!("../../../assets/icons/lucide/mouse-pointer-2.svg"),
            Icon::Move => include_str!("../../../assets/icons/lucide/move.svg"),
            Icon::RotateGizmo => include_str!("../../../assets/icons/lucide/rotate-3d.svg"),
            Icon::Scale => include_str!("../../../assets/icons/lucide/scale-3d.svg"),
            Icon::EditMode => include_str!("../../../assets/icons/lucide/box.svg"),
            Icon::Grid => include_str!("../../../assets/icons/lucide/grid-3x3.svg"),
            Icon::Copy => include_str!("../../../assets/icons/lucide/copy.svg"),
            Icon::Trash => include_str!("../../../assets/icons/lucide/trash-2.svg"),
            Icon::Undo => include_str!("../../../assets/icons/lucide/undo-2.svg"),
            Icon::Redo => include_str!("../../../assets/icons/lucide/redo-2.svg"),
            Icon::Add => include_str!("../../../assets/icons/lucide/plus.svg"),
            Icon::Remove => include_str!("../../../assets/icons/lucide/minus.svg"),
            Icon::Eye => include_str!("../../../assets/icons/lucide/eye.svg"),
            Icon::EyeOff => include_str!("../../../assets/icons/lucide/eye-off.svg"),
            Icon::Layers => include_str!("../../../assets/icons/lucide/layers.svg"),
            Icon::Axis => include_str!("../../../assets/icons/lucide/axis-3d.svg"),
            Icon::Grip => include_str!("../../../assets/icons/lucide/grip.svg"),
            Icon::Pen => include_str!("../../../assets/icons/lucide/pen-tool.svg"),
            Icon::Ruler => include_str!("../../../assets/icons/lucide/ruler.svg"),
            Icon::Settings => include_str!("../../../assets/icons/lucide/settings.svg"),
        }
    }
}
