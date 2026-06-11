# Editor Design System — "Forge Dark"

A cohesive dark theme for the immediate-mode editor UI, tuned to read like a
modern DCC tool (Blender 4.x / Unreal 5 dark). Everything below is achievable
with **flat axis-aligned rects, 1px lines, and the 8×16 bitmap font** only — no
gradients, no rounded corners, no images beyond the icon atlas.

The core problems being fixed:

- Panels are all near-black `[22,22,28]` → they read as one muddy slab. The new
  palette gives each surface a deliberate luminance step so panels read as
  distinct, stacked surfaces.
- Separators are thin, low-contrast 1.5px lines → replaced with crisp **1px
  bevel borders** (light top/left, dark bottom/right) on every panel.
- Text is dim (`150–205` greys) → **primary text brightened to `~232`**, with a
  clear secondary / disabled hierarchy.
- Hover/active states were invented per-widget → unified into one **accent ramp**
  used everywhere.
- Inconsistent padding → a single **spacing scale** (4px grid).

All colors are sRGB `[u8; 4]` RGBA. The palette uses a very slight cool-blue
neutral (R≈G, B a touch higher) so it never looks dead-grey or muddy-purple.

---

## 1. Color tokens

### 1.1 Neutral surfaces (elevation ladder)

Each step is a deliberate luminance increase. Lower = further back, higher =
closer to the user / more interactive. Note panels are now opaque (`255`), not
the old `240` — the alpha bleed was contributing to the muddy look.

| Token                  | RGBA                  | Luma | Role                                                        |
|------------------------|-----------------------|------|-------------------------------------------------------------|
| `COL_WINDOW_BG`        | `[18, 19, 23, 255]`   | ~19  | App backdrop / behind everything, gaps between panels       |
| `COL_PANEL_BG`         | `[31, 33, 39, 255]`   | ~33  | Default panel body (Outliner, Details, Log content)         |
| `COL_PANEL_BG_ALT`     | `[35, 37, 44, 255]`   | ~37  | Zebra-stripe alt row / nested group bg                      |
| `COL_HEADER_BG`        | `[41, 44, 52, 255]`   | ~44  | Panel title bars, section headers, tab strip                |
| `COL_TOOLBAR_BG`       | `[37, 39, 47, 255]`   | ~39  | Top toolbar / menu bar background                           |
| `COL_RAISED_BG`        | `[52, 55, 64, 255]`   | ~55  | Raised controls: buttons, icon buttons (idle)               |
| `COL_INPUT_BG`         | `[24, 26, 31, 255]`   | ~26  | Recessed inputs: drag fields, text boxes, sliders track     |
| `COL_TOOLTIP_BG`       | `[14, 15, 19, 245]`   | ~15  | Tooltip / popup background (sits above all, slightly trans) |
| `COL_VIEWPORT_TAG_BG`  | `[18, 19, 23, 200]`   | ~19  | Viewport label chip bg (semi-transparent over 3D)          |

**Elevation reads as:** `INPUT (recessed) < WINDOW < PANEL < HEADER/TOOLBAR <
RAISED`. Three clear steps between a recessed input, a flat panel, and a raised
button.

### 1.2 Borders & separators

Every panel gets a **1px bevel**: a lighter line on the top/left edge and a
darker line on the bottom/right edge. This single trick is what makes flat rects
read as raised surfaces.

| Token                    | RGBA                | Role                                                   |
|--------------------------|---------------------|--------------------------------------------------------|
| `COL_BORDER`             | `[12, 13, 16, 255]` | Hard outer border / darkest seam between panels        |
| `COL_BORDER_LIGHT`       | `[58, 61, 71, 255]` | Bevel highlight — top & left inner edge of a panel     |
| `COL_BORDER_DARK`        | `[14, 15, 18, 255]` | Bevel shadow — bottom & right inner edge of a panel    |
| `COL_SEP`                | `[48, 51, 60, 255]` | Generic separator line (section header underline)      |
| `COL_SEP_STRONG`         | `[62, 65, 76, 255]` | Higher-contrast divider (group dividers in toolbar)    |
| `COL_DIVIDER_HOVER`      | `[88, 140, 220, 255]`| Draggable panel divider when hovered/dragging (accent)|

Replaces the old single `SEP=[58,58,74,255]` used for everything. Separators are
now 1px (not 1.5px) and paired with the bevel for depth.

### 1.3 Accent + interaction ramp

One accent hue (a confident DCC blue) drives selection, focus, and active-tool
state. Hover/pressed are derived from the *raised* surface, not the accent, so
non-active buttons stay neutral and only the accent pops.

| Token                  | RGBA                  | Role                                                    |
|------------------------|-----------------------|---------------------------------------------------------|
| `COL_ACCENT`           | `[64, 132, 223, 255]` | Primary accent — selection fill, active tool, focus     |
| `COL_ACCENT_HI`        | `[96, 164, 246, 255]` | Bright accent — accent underline / focus ring / pressed |
| `COL_ACCENT_DIM`       | `[42, 84, 142, 255]`  | Muted accent — selected-row bg, active-tool bg fill     |
| `COL_ACCENT_TEXT`      | `[150, 200, 255, 255]`| Text/icon tint when drawn on an accent or selected bg   |
| `COL_CTRL_HOVER`       | `[66, 70, 82, 255]`   | Button/icon/row hover bg (one step above RAISED)        |
| `COL_CTRL_PRESSED`     | `[78, 132, 200, 255]` | Button/icon pressed bg (accent-tinted press feedback)   |
| `COL_ROW_HOVER`        | `[44, 47, 56, 255]`   | List/tree row hover bg (subtle, over PANEL_BG)          |
| `COL_ROW_SELECTED`     | `[42, 84, 142, 255]`  | List/tree row selected bg (= ACCENT_DIM)                |

### 1.4 Semantic / status

| Token              | RGBA                  | Role                                  |
|--------------------|-----------------------|---------------------------------------|
| `COL_OK`           | `[112, 196, 120, 255]`| Success / "raster ok" / status line   |
| `COL_WARN`         | `[240, 184, 96, 255]` | Warning / EDIT-mode badge             |
| `COL_ERROR`        | `[228, 96, 96, 255]`  | Error log lines                       |
| `COL_CHECK_ON`     | `[108, 190, 116, 255]`| Checkbox enabled fill (de-saturated)  |
| `COL_CHECK_OFF`    | `[52, 55, 64, 255]`   | Checkbox disabled fill (= RAISED)     |

### 1.5 Text

Primary is intentionally bright (`232`) — the single most impactful fix for the
"dim/muddy" complaint. Contrast checks below are vs the surface the text usually
sits on.

| Token              | RGBA                  | Role                          | Contrast vs. bg          |
|--------------------|-----------------------|-------------------------------|--------------------------|
| `COL_TEXT`         | `[232, 234, 240, 255]`| Primary body text / values    | ~12.8:1 on PANEL_BG (AAA)|
| `COL_TEXT_DIM`     | `[164, 168, 180, 255]`| Secondary labels, field names | ~5.9:1 on PANEL_BG (AA)  |
| `COL_TEXT_DISABLED`| `[104, 108, 120, 255]`| Disabled / placeholder        | ~2.7:1 (intentional low) |
| `COL_TEXT_HEADER`  | `[214, 224, 240, 255]`| Title-bar / section labels    | ~8.4:1 on HEADER_BG (AAA)|
| `COL_TEXT_ACCENT`  | `[150, 200, 255, 255]`| Text on selected/accent bg    | ~6.5:1 on ACCENT_DIM (AA)|

All primary/secondary/header text clears WCAG AA (4.5:1) comfortably; primary &
header clear AAA (7:1). Disabled is deliberately below AA to read as inactive.

---

## 2. Spacing system (4px grid)

| Token              | Value | Role                                                       |
|--------------------|-------|------------------------------------------------------------|
| `PAD_PANEL`        | 8.0   | Inner padding from panel edge to content (left/right/top)  |
| `PAD_TIGHT`        | 4.0   | Tight padding (icon→label, inside chips)                    |
| `PAD_SECTION`      | 6.0   | Vertical gap above a section header / between groups        |
| `ROW_H`            | 24.0  | List/tree row height (Outliner rows, content rows)         |
| `BTN_H`            | 24.0  | Standard button height                                      |
| `HBTN_H`           | 22.0  | Compact horizontal-toolbar button height                   |
| `ICON_BTN`         | 24.0  | Icon button box (toolbar) — bump from 22 for click comfort |
| `BTN_GAP`          | 6.0   | Gap between adjacent buttons/icons                          |
| `FIELD_H`          | 20.0  | Drag-field / input row height                               |
| `FIELD_GAP`        | 4.0   | Vertical gap between stacked fields                         |
| `SECTION_H`        | 22.0  | Section-header bar height                                   |
| `SECTION_GAP`      | 4.0   | Gap below a section header before its content               |
| `TITLEBAR_H`       | 24.0  | Panel title-bar height (was 22; +2 for baseline breathing) |
| `MENUBAR_H`        | 24.0  | Top menu-bar row height                                     |
| `TOOLBAR_H`        | 54.0  | Full toolbar block (menu row + icon row)                    |
| `GROUP_SEP_W`      | 1.0   | Vertical tool-group divider width                           |
| `BORDER_W`         | 1.0   | Panel border / bevel width                                  |
| `ACCENT_BAR_W`     | 2.0   | Accent underline / active-tool bar / focus tab thickness   |

**Text baseline:** the 8×16 font in a bar of height `H` is vertically centered
at `y + (H - 16) / 2`, rounded down. For `TITLEBAR_H = 24` → text at `y + 4`.
For `MENUBAR_H = 24` → `y + 4`. For `SECTION_H = 22` → `y + 3`. For a `ROW_H = 24`
row → glyphs at `row_y + 4`. Use these consistently instead of the current mix
of `+1 / +2 / +3`.

**Glyph advance** stays at the font's 8px width; horizontal text padding from a
container edge is always `PAD_PANEL` (8) for panel content and `PAD_TIGHT` (4)
inside small chips/buttons.

---

## 3. Panel border / bevel recipe

Every bordered panel draws, in order:

1. Body fill: `push_panel_bg(x, y, w, h, COL_PANEL_BG)`.
2. **Top + left bevel** (highlight): two 1px lines in `COL_BORDER_LIGHT`
   along the inside of the top and left edges.
3. **Bottom + right bevel** (shadow): two 1px lines in `COL_BORDER_DARK`
   along the inside of the bottom and right edges.
4. The seam *between* two panels (and the window outer edge) is the darkest
   line, `COL_BORDER`, drawn once on the shared edge.

With flat rects (no `push_rect_border` primitive needed), a border is just four
`push_rect` calls of width/height 1:

```text
// top    (light)
push_rect(x,        y,        w, 1, SOLID, COL_BORDER_LIGHT)
// left   (light)
push_rect(x,        y,        1, h, SOLID, COL_BORDER_LIGHT)
// bottom (dark)
push_rect(x,        y+h-1,    w, 1, SOLID, COL_BORDER_DARK)
// right  (dark)
push_rect(x+w-1,    y,        1, h, SOLID, COL_BORDER_DARK)
```

This produces a subtle inset/raised bevel that reads cleanly at 1px. The
draggable dividers (between Outliner|Viewport, Viewport|Details, Viewport|Bottom)
become a single `COL_BORDER` seam that swaps to `COL_DIVIDER_HOVER` (full
thickness) when `div_hover`/`div_drag` is set.

---

## 4. Title-bar treatment

DCC tools mark a panel header with a subtle **left accent bar** so the eye finds
the section without a loud colored bar.

- Header fill: `push_rect(x, y, w, TITLEBAR_H, SOLID, COL_HEADER_BG)`.
- **Left accent bar:** `push_rect(x, y, ACCENT_BAR_W, TITLEBAR_H, SOLID, COL_ACCENT)`
  (2px). For the *focused* panel use `COL_ACCENT_HI`; for unfocused use
  `COL_ACCENT_DIM` so focus is legible.
- Bottom border of the header: 1px `COL_BORDER_DARK` (the header sits proud of
  the body).
- Label text: `COL_TEXT_HEADER`, glyphs at `x + PAD_PANEL` (so they clear the
  accent bar), baseline `y + 4`.

Bottom-panel **tabs** (OUTPUT LOG / CONTENT) keep their bottom underline pattern
but standardized: active tab bg `COL_HEADER_BG` + 2px `COL_ACCENT_HI` underline +
`COL_TEXT_HEADER` label; hover tab bg `COL_ROW_HOVER`; inactive label
`COL_TEXT_DIM`.

---

## 5. Widget state mapping (apply these)

| Widget                 | Idle bg          | Hover bg         | Pressed/Active bg     | Text / icon                          |
|------------------------|------------------|------------------|------------------------|--------------------------------------|
| `button` / `hbutton`   | `COL_RAISED_BG`  | `COL_CTRL_HOVER` | `COL_CTRL_PRESSED`     | `COL_TEXT`                           |
| `hicon` (idle)         | `COL_RAISED_BG`  | `COL_CTRL_HOVER` | `COL_CTRL_PRESSED`     | idle `COL_TEXT_DIM`, hover `COL_TEXT`|
| `hicon` (active tool)  | `COL_ACCENT_DIM` | `COL_ACCENT_DIM` | —                      | `COL_TEXT` + 2px `COL_ACCENT_HI` bar |
| Outliner / list row    | `COL_PANEL_BG` / `COL_PANEL_BG_ALT` (zebra) | `COL_ROW_HOVER` | `COL_ROW_SELECTED` | sel `COL_TEXT_ACCENT`, else `COL_TEXT_DIM` |
| `drag_field` value box | `COL_INPUT_BG`   | `COL_CTRL_HOVER` | `COL_ACCENT_DIM` (drag)| value `COL_TEXT`, label `COL_TEXT_DIM`; bottom edge `COL_ACCENT_HI` while dragging |
| `slider` track / fill  | `COL_INPUT_BG` / `COL_ACCENT` | — | — | label `COL_TEXT_DIM`        |
| `checkbox`             | off `COL_CHECK_OFF` / on `COL_CHECK_ON` | — | — | label `COL_TEXT_DIM`     |
| `section_header`       | `COL_HEADER_BG`  | —                | —                      | `COL_TEXT_HEADER`, underline `COL_SEP`|
| `collapsible_header`   | `COL_HEADER_BG`  | `COL_CTRL_HOVER` | —                      | label `COL_TEXT_HEADER`, arrow `COL_TEXT_DIM`/`COL_TEXT` |
| Menu-bar item          | transparent      | `COL_CTRL_HOVER` | `COL_ACCENT_DIM` (open)| `COL_TEXT_HEADER`                    |
| Tooltip                | `COL_TOOLTIP_BG` | —                | —                      | `COL_TEXT`, border `COL_SEP_STRONG`  |

**Rule of thumb:** raised controls hover toward *neutral lightening*
(`COL_CTRL_HOVER`); only *selected / active / pressed* introduce accent. This
keeps the toolbar calm and makes the active tool unmistakable.

---

## 6. Per-panel checklist

**Toolbar (`draw_toolbar`)**
- bg `COL_TOOLBAR_BG`; 1px `COL_BORDER_DARK` under the menu row; 1px
  `COL_BORDER` seam under the whole toolbar (its bottom edge).
- Menu labels `COL_TEXT_HEADER`; hover `COL_CTRL_HOVER`, open `COL_ACCENT_DIM`.
- Icon buttons → `ICON_BTN` (24) boxes, `BTN_GAP` (6) apart; group dividers 1px
  `COL_SEP_STRONG` via `hsep_v`.
- Mode badge: EDIT → `COL_WARN`, OBJ → `COL_TEXT_DIM`. Status line → `COL_OK`.

**Outliner (`draw_outliner`)**
- Body `COL_PANEL_BG` + full bevel border. Title bar per §4, label "OUTLINER".
- Rows: zebra `COL_PANEL_BG`/`COL_PANEL_BG_ALT`, `ROW_H` (24), hover
  `COL_ROW_HOVER`, selected `COL_ROW_SELECTED`. Icon at `x + PAD_PANEL`, label at
  `x + PAD_PANEL + 16 + PAD_TIGHT`, baseline `row_y + 4`.
- Left divider seam `COL_BORDER` → `COL_DIVIDER_HOVER` on hover/drag.

**Details / Mesh-edit inspector (`draw_inspector`)**
- Body `COL_PANEL_BG` + bevel. Title "DETAILS" / "MESH EDIT" per §4.
- Section headers per §5; fields use `drag_field` mapping; `PAD_PANEL` inner
  padding on both sides, `FIELD_GAP` between rows.
- Right divider seam `COL_BORDER` → `COL_DIVIDER_HOVER`.

**Bottom panel (`draw_bottom_panel`)**
- Body `COL_PANEL_BG` + bevel; top seam `COL_BORDER` → `COL_DIVIDER_HOVER`.
- Tab strip per §4. Log lines: normal `COL_TEXT_DIM`, errors `COL_ERROR`,
  warnings `COL_WARN`, FPS/stats `COL_OK`.

**Viewport chrome (`draw_viewport_chrome`)**
- Pane borders: focused `COL_ACCENT_HI` (slightly transparent ok), unfocused
  `COL_BORDER` — 1px, all four edges.
- Label chip bg `COL_VIEWPORT_TAG_BG`; focused label `COL_ACCENT_TEXT`,
  unfocused `COL_TEXT_DIM`.

---

## 7. Specific fixes for the named clunky issues

| Complaint                         | Fix                                                                                  |
|-----------------------------------|--------------------------------------------------------------------------------------|
| "Panels are near-black / muddy"   | Elevation ladder §1.1: WINDOW 19 → PANEL 33 → HEADER 44 → RAISED 55, all opaque.     |
| "Panels should have borders"      | 1px bevel on every panel (§3): `COL_BORDER_LIGHT` top/left, `COL_BORDER_DARK` bot/rt.|
| "Separators thin & low-contrast"  | 1px crisp lines, `COL_SEP` / `COL_SEP_STRONG`, paired with bevels and dark seams.    |
| "No crisp panel borders"          | Dark `COL_BORDER` seam between panels + per-panel bevel; dividers brighten on hover. |
| "Padding inconsistent"            | 4px grid (§2): `PAD_PANEL=8` everywhere for panel content, unified baselines.        |
| "Hover/active states ad hoc"      | Single ramp (§1.3): neutral `COL_CTRL_HOVER` for hover, accent only for active/sel.  |
| "Bitmap text is dim"              | Primary text → `[232,234,240]` (~13:1 contrast); clear dim/disabled hierarchy.      |

---

## 8. Rust constants (ready to paste)

Drop these into the const block in `src/bin/editor.rs` (and/or a shared theme
module). Token names match the tables above.

```rust
// ── Forge Dark — color tokens (RGBA u8x4) ───────────────────────────────────

// Neutral surfaces (elevation ladder, back → front)
pub const COL_WINDOW_BG:       [u8; 4] = [18, 19, 23, 255];
pub const COL_PANEL_BG:        [u8; 4] = [31, 33, 39, 255];
pub const COL_PANEL_BG_ALT:    [u8; 4] = [35, 37, 44, 255];
pub const COL_HEADER_BG:       [u8; 4] = [41, 44, 52, 255];
pub const COL_TOOLBAR_BG:      [u8; 4] = [37, 39, 47, 255];
pub const COL_RAISED_BG:       [u8; 4] = [52, 55, 64, 255];
pub const COL_INPUT_BG:        [u8; 4] = [24, 26, 31, 255];
pub const COL_TOOLTIP_BG:      [u8; 4] = [14, 15, 19, 245];
pub const COL_VIEWPORT_TAG_BG: [u8; 4] = [18, 19, 23, 200];

// Borders & separators
pub const COL_BORDER:          [u8; 4] = [12, 13, 16, 255];
pub const COL_BORDER_LIGHT:    [u8; 4] = [58, 61, 71, 255];
pub const COL_BORDER_DARK:     [u8; 4] = [14, 15, 18, 255];
pub const COL_SEP:             [u8; 4] = [48, 51, 60, 255];
pub const COL_SEP_STRONG:      [u8; 4] = [62, 65, 76, 255];
pub const COL_DIVIDER_HOVER:   [u8; 4] = [88, 140, 220, 255];

// Accent + interaction ramp
pub const COL_ACCENT:          [u8; 4] = [64, 132, 223, 255];
pub const COL_ACCENT_HI:       [u8; 4] = [96, 164, 246, 255];
pub const COL_ACCENT_DIM:      [u8; 4] = [42, 84, 142, 255];
pub const COL_ACCENT_TEXT:     [u8; 4] = [150, 200, 255, 255];
pub const COL_CTRL_HOVER:      [u8; 4] = [66, 70, 82, 255];
pub const COL_CTRL_PRESSED:    [u8; 4] = [78, 132, 200, 255];
pub const COL_ROW_HOVER:       [u8; 4] = [44, 47, 56, 255];
pub const COL_ROW_SELECTED:    [u8; 4] = [42, 84, 142, 255];

// Semantic / status
pub const COL_OK:              [u8; 4] = [112, 196, 120, 255];
pub const COL_WARN:            [u8; 4] = [240, 184, 96, 255];
pub const COL_ERROR:           [u8; 4] = [228, 96, 96, 255];
pub const COL_CHECK_ON:        [u8; 4] = [108, 190, 116, 255];
pub const COL_CHECK_OFF:       [u8; 4] = [52, 55, 64, 255];

// Text
pub const COL_TEXT:            [u8; 4] = [232, 234, 240, 255];
pub const COL_TEXT_DIM:        [u8; 4] = [164, 168, 180, 255];
pub const COL_TEXT_DISABLED:   [u8; 4] = [104, 108, 120, 255];
pub const COL_TEXT_HEADER:     [u8; 4] = [214, 224, 240, 255];
pub const COL_TEXT_ACCENT:     [u8; 4] = [150, 200, 255, 255];

// ── Spacing (px) ────────────────────────────────────────────────────────────
pub const PAD_PANEL:    f32 = 8.0;
pub const PAD_TIGHT:    f32 = 4.0;
pub const PAD_SECTION:  f32 = 6.0;
pub const ROW_H:        f32 = 24.0;
pub const BTN_H:        f32 = 24.0;
pub const HBTN_H:       f32 = 22.0;
pub const ICON_BTN:     f32 = 24.0;
pub const BTN_GAP:      f32 = 6.0;
pub const FIELD_H:      f32 = 20.0;
pub const FIELD_GAP:    f32 = 4.0;
pub const SECTION_H:    f32 = 22.0;
pub const SECTION_GAP:  f32 = 4.0;
pub const TITLEBAR_H:   f32 = 24.0;
pub const MENUBAR_H:    f32 = 24.0;
pub const TOOLBAR_H:    f32 = 54.0;
pub const GROUP_SEP_W:  f32 = 1.0;
pub const BORDER_W:     f32 = 1.0;
pub const ACCENT_BAR_W: f32 = 2.0;
```

### Suggested helper (optional, still flat-rect only)

A tiny bevel helper keeps the border recipe in one place — implement on the
draw list or as a free function; it only uses `push_rect`:

```rust
/// 1px beveled border: light top/left, dark bottom/right.
fn push_bevel(dl: &mut DrawList, x: f32, y: f32, w: f32, h: f32) {
    dl.push_rect(x,         y,         w,   1.0, SOLID, COL_BORDER_LIGHT); // top
    dl.push_rect(x,         y,         1.0, h,   SOLID, COL_BORDER_LIGHT); // left
    dl.push_rect(x,         y + h - 1.0, w, 1.0, SOLID, COL_BORDER_DARK);  // bottom
    dl.push_rect(x + w - 1.0, y,       1.0, h,   SOLID, COL_BORDER_DARK);  // right
}
```
