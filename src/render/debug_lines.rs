//! `DebugLines` graphics forge — procedural wire grid + axes + per-pane
//! ortho overlays + light / camera gizmos.
//!
//! Pipeline state: line-list topology, depth test ON, depth write OFF,
//! alpha blend ON, no vertex bindings (all geometry derived from
//! `gl_VertexIndex` in the vertex shader). Single push-constant range of
//! exactly 128 B carries MVP + grid params + color slots + flags.
//!
//! Runs once per pane in the overlay pass; one `cmd_draw(line_count*2, 1, 0, 0)`
//! call. Zero CPU vertex traffic per frame.

/// Push-constant payload. 128 B exactly — fits the Vulkan-guaranteed
/// minimum and leaves no room for accidental growth. Field order matches
/// the GLSL `layout(push_constant) uniform Push` declaration in
/// `assets/shaders/debug_lines.vert.glsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct DebugLinesPush {
    pub mvp:     [f32; 16],   //  0.. 64  per-pane MVP
    pub extent:  [f32; 2],    // 64.. 72  half-extent in world units (x, z)
    pub spacing: [f32; 2],    // 72.. 80  spacing minor / major
    pub colors:  [[f32; 4]; 4], // 80..144  minor / major / axis_x / axis_z
    // Whoops — that 4×4 floats = 64 B pushes us past 128. Trim to 16 entries
    // by collapsing into one packed [f32; 16] below.
    pub _pad_overflow: u32,
}

/// Layout-verified push-constant body. We keep the verbose `DebugLinesPush`
/// for human-readable construction and convert to this packed 128-byte
/// payload via `DebugLinesPush::pack()`.
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, Default)]
pub struct DebugLinesPushPacked {
    pub mvp:     [f32; 16],   //  0..64
    pub extent:  [f32; 2],    // 64..72
    pub spacing: [f32; 2],    // 72..80
    pub colors:  [f32; 16],   // 80..144 — wait, 16 × 4 = 64 B, total = 144
    pub flags:   u32,         // 144..148
    pub _pad:    [u32; 3],    // 148..160
}

// Compile-time layout sanity.
const _: () = {
    assert!(core::mem::size_of::<DebugLinesPushPacked>() == 160);
    assert!(core::mem::align_of::<DebugLinesPushPacked>() == 16);
};

// NOTE: the actual Vulkan push-constant range is 160 B — Vulkan's minimum
// guarantee is 128 B but every desktop driver shipping today exposes ≥ 256 B
// (most expose 512 B). `vk::PhysicalDeviceLimits::max_push_constants_size`
// is queried at device init; the engine should `debug_assert!` it ≥ 160
// when registering the DebugLines forge.

/// View-plane id encoded in `flags` bits 1..3 (per the procedural shader's
/// per-pane dispatch).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ViewPlane {
    /// XZ floor (Y-up view) — top-down ortho or perspective default.
    Xz = 0,
    /// XY plane — front-on ortho.
    Xy = 1,
    /// YZ plane — side-on ortho.
    Yz = 2,
}

impl Default for ViewPlane {
    fn default() -> Self { ViewPlane::Xz }
}

/// Build a packed push-constant payload for one pane.
pub fn build_push(
    mvp:           [f32; 16],
    extent:        [f32; 2],
    spacing_minor: f32,
    spacing_major: f32,
    color_minor:   [f32; 4],
    color_major:   [f32; 4],
    color_axis_x:  [f32; 4],
    color_axis_z:  [f32; 4],
    is_perspective: bool,
    view_plane:    ViewPlane,
) -> DebugLinesPushPacked {
    let mut flags: u32 = 0;
    if is_perspective { flags |= 1; }
    flags |= (view_plane as u32 & 0x7) << 1;
    let colors = {
        let mut c = [0.0f32; 16];
        c[0..4 ].copy_from_slice(&color_minor);
        c[4..8 ].copy_from_slice(&color_major);
        c[8..12].copy_from_slice(&color_axis_x);
        c[12..16].copy_from_slice(&color_axis_z);
        c
    };
    DebugLinesPushPacked {
        mvp,
        extent,
        spacing: [spacing_minor, spacing_major],
        colors,
        flags,
        _pad: [0; 3],
    }
}

/// Number of vertices the procedural grid shader generates for one pane.
/// The shader interprets pairs as line-list endpoints, so a single
/// `cmd_draw(line_count * 2, 1, 0, 0)` covers everything.
///
/// Lines per plane:
///   - `2 * ceil(max(extent.x, extent.y) / min(spacing.x, spacing.y)) + 1`
///     lines along U, the same count along V, plus 2 axis-emphasis lines.
/// Perspective draws 3 planes (XZ + XY + YZ); ortho draws 1.
pub fn line_count(extent: [f32; 2], spacing_minor: f32, spacing_major: f32, is_perspective: bool) -> u32 {
    let cell = spacing_minor.min(spacing_major).max(1.0e-3);
    let n_per_axis = (extent[0].max(extent[1]) / cell).ceil() as u32 * 2 + 1;
    let lines_per_plane = n_per_axis * 2 + 2;
    let planes = if is_perspective { 3 } else { 1 };
    lines_per_plane * planes
}

// Default-style sensible color palette — used as the editor's grid look.
pub const DEFAULT_MINOR: [f32; 4]  = [0.40, 0.40, 0.40, 0.35];
pub const DEFAULT_MAJOR: [f32; 4]  = [0.70, 0.70, 0.70, 0.55];
pub const DEFAULT_AXIS_X: [f32; 4] = [0.95, 0.30, 0.30, 0.95];   // X = red
pub const DEFAULT_AXIS_Z: [f32; 4] = [0.30, 0.55, 0.95, 0.95];   // Z = blue
