//! "Forge Dark" — the editor design system (see docs/editor_design.md).
//!
//! A cohesive dark DCC theme expressed entirely as flat RGBA colors + px
//! spacing, so it renders with the immediate UI's rect/line/bitmap-text
//! primitives. An elevation ladder (window → panel → header → raised) gives
//! panels distinct stacked surfaces; a 1px light/dark bevel draws real
//! borders; one accent ramp owns selection/active while neutral controls
//! lighten on hover.

use super::draw::{self, DrawList};

// ── Neutral surfaces (elevation ladder, back → front) ───────────────────────
pub const COL_WINDOW_BG: [u8; 4] = [18, 19, 23, 255];
pub const COL_PANEL_BG: [u8; 4] = [31, 33, 39, 255];
pub const COL_PANEL_BG_ALT: [u8; 4] = [35, 37, 44, 255];
pub const COL_HEADER_BG: [u8; 4] = [41, 44, 52, 255];
pub const COL_TOOLBAR_BG: [u8; 4] = [37, 39, 47, 255];
pub const COL_RAISED_BG: [u8; 4] = [52, 55, 64, 255];
pub const COL_INPUT_BG: [u8; 4] = [24, 26, 31, 255];
pub const COL_TOOLTIP_BG: [u8; 4] = [14, 15, 19, 245];
pub const COL_VIEWPORT_TAG_BG: [u8; 4] = [18, 19, 23, 200];

// ── Borders & separators ────────────────────────────────────────────────────
pub const COL_BORDER: [u8; 4] = [12, 13, 16, 255];
pub const COL_BORDER_LIGHT: [u8; 4] = [58, 61, 71, 255];
pub const COL_BORDER_DARK: [u8; 4] = [14, 15, 18, 255];
pub const COL_SEP: [u8; 4] = [48, 51, 60, 255];
pub const COL_SEP_STRONG: [u8; 4] = [62, 65, 76, 255];
pub const COL_DIVIDER_HOVER: [u8; 4] = [88, 140, 220, 255];

// ── Accent + interaction ramp ───────────────────────────────────────────────
pub const COL_ACCENT: [u8; 4] = [64, 132, 223, 255];
pub const COL_ACCENT_HI: [u8; 4] = [96, 164, 246, 255];
pub const COL_ACCENT_DIM: [u8; 4] = [42, 84, 142, 255];
pub const COL_CTRL_BG: [u8; 4] = [52, 55, 64, 255];
pub const COL_CTRL_HOVER: [u8; 4] = [66, 70, 82, 255];
pub const COL_CTRL_PRESSED: [u8; 4] = [78, 132, 200, 255];
pub const COL_ROW_HOVER: [u8; 4] = [44, 47, 56, 255];
pub const COL_ROW_SELECTED: [u8; 4] = [42, 84, 142, 255];

// ── Semantic / status ───────────────────────────────────────────────────────
pub const COL_OK: [u8; 4] = [112, 196, 120, 255];
pub const COL_WARN: [u8; 4] = [240, 184, 96, 255];
pub const COL_ERROR: [u8; 4] = [228, 96, 96, 255];
pub const COL_CHECK_ON: [u8; 4] = [108, 190, 116, 255];
pub const COL_CHECK_OFF: [u8; 4] = [52, 55, 64, 255];

// ── Text ────────────────────────────────────────────────────────────────────
pub const COL_TEXT: [u8; 4] = [232, 234, 240, 255];
pub const COL_TEXT_DIM: [u8; 4] = [164, 168, 180, 255];
pub const COL_TEXT_DISABLED: [u8; 4] = [104, 108, 120, 255];
pub const COL_TEXT_HEADER: [u8; 4] = [214, 224, 240, 255];
pub const COL_TEXT_ACCENT: [u8; 4] = [150, 200, 255, 255];

// ── Spacing (px, 4px grid) ──────────────────────────────────────────────────
pub const PAD_PANEL: f32 = 8.0;
pub const PAD_SECTION: f32 = 6.0;
pub const ROW_H: f32 = 24.0;
pub const BTN_GAP: f32 = 6.0;
pub const TITLEBAR_H: f32 = 24.0;
pub const BORDER_W: f32 = 1.0;
pub const ACCENT_BAR_W: f32 = 2.0;

/// Draw a 1px bevel border just inside the rect `(x, y, w, h)`: a light
/// top/left edge and a dark bottom/right edge for subtle depth. Use after
/// filling the panel background.
pub fn push_bevel(dl: &mut DrawList, x: f32, y: f32, w: f32, h: f32) {
    // Top + left = light.
    dl.push_rect(x, y, w, BORDER_W, draw::SOLID, COL_BORDER_LIGHT);
    dl.push_rect(x, y, BORDER_W, h, draw::SOLID, COL_BORDER_LIGHT);
    // Bottom + right = dark.
    dl.push_rect(x, y + h - BORDER_W, w, BORDER_W, draw::SOLID, COL_BORDER_DARK);
    dl.push_rect(x + w - BORDER_W, y, BORDER_W, h, draw::SOLID, COL_BORDER_DARK);
}

/// Draw a panel header bar across the top of `(x, y, w, ·)`: header-bg fill,
/// a left accent bar (bright when `focused`), a dark bottom seam, and the
/// title text inset past the bar. Returns nothing; caller draws body below.
#[allow(clippy::too_many_arguments)]
pub fn push_panel_header(dl: &mut DrawList, x: f32, y: f32, w: f32, title: &str, focused: bool) {
    use super::font;
    dl.push_rect(x, y, w, TITLEBAR_H, draw::SOLID, COL_HEADER_BG);
    let bar = if focused { COL_ACCENT_HI } else { COL_ACCENT_DIM };
    dl.push_rect(x, y, ACCENT_BAR_W, TITLEBAR_H, draw::SOLID, bar);
    dl.push_rect(
        x,
        y + TITLEBAR_H - BORDER_W,
        w,
        BORDER_W,
        draw::SOLID,
        COL_BORDER_DARK,
    );
    let mut tx = x + 8.0;
    for c in title.chars() {
        if c != ' ' {
            let uv = font::glyph_rect(c);
            if uv != [0.0_f32; 4] {
                dl.push_rect(tx, y + 4.0, 8.0, 16.0, uv, COL_TEXT_HEADER);
            }
        }
        tx += 8.0;
    }
}
