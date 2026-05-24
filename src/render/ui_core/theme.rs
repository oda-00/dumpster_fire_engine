/// Deep-pastel-purple / forest-green dual-mode UI palette.
///
/// All colours are `[r, g, b, a]` with `u8` components.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub background:        [u8; 4],
    pub surface:           [u8; 4],
    pub surface_variant:   [u8; 4],
    pub primary:           [u8; 4],
    pub primary_container: [u8; 4],
    pub on_primary:        [u8; 4],
    pub secondary:         [u8; 4],
    pub on_secondary:      [u8; 4],
    pub outline:           [u8; 4],
    pub on_background:     [u8; 4],
    pub on_surface:        [u8; 4],
    pub error:             [u8; 4],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
}

impl Theme {
    /// Deep pastel purple background, forest-green accents — dark variant.
    pub const DARK: Self = Self {
        background:        [0x1E, 0x16, 0x26, 0xFF], // very dark purple
        surface:           [0x2B, 0x1F, 0x38, 0xFF], // dark plum
        surface_variant:   [0x38, 0x2B, 0x4C, 0xFF], // mid plum
        primary:           [0xB3, 0x9D, 0xDB, 0xFF], // deep pastel purple
        primary_container: [0x4A, 0x35, 0x6B, 0xFF], // muted grape
        on_primary:        [0x0F, 0x0B, 0x18, 0xFF], // near-black
        secondary:         [0x4C, 0xAF, 0x50, 0xFF], // forest green
        on_secondary:      [0x05, 0x18, 0x07, 0xFF], // very dark green
        outline:           [0x7B, 0x65, 0x9A, 0xFF], // muted purple border
        on_background:     [0xE8, 0xDF, 0xF5, 0xFF], // lavender-white text
        on_surface:        [0xD5, 0xC9, 0xED, 0xFF], // soft lavender text
        error:             [0xFF, 0x6B, 0x6B, 0xFF], // coral-red
    };

    /// Deep pastel purple / forest-green — light variant.
    pub const LIGHT: Self = Self {
        background:        [0xF5, 0xF0, 0xFF, 0xFF], // pale lavender
        surface:           [0xEE, 0xE5, 0xFF, 0xFF], // off-white purple tint
        surface_variant:   [0xDF, 0xD3, 0xF5, 0xFF], // light plum
        primary:           [0x6A, 0x42, 0xA8, 0xFF], // deep pastel purple
        primary_container: [0xCF, 0xBE, 0xF0, 0xFF], // light lavender fill
        on_primary:        [0xFF, 0xFF, 0xFF, 0xFF],
        secondary:         [0x2E, 0x7D, 0x32, 0xFF], // forest green
        on_secondary:      [0xFF, 0xFF, 0xFF, 0xFF],
        outline:           [0x7B, 0x65, 0x9A, 0xFF], // same purple border
        on_background:     [0x1A, 0x10, 0x2A, 0xFF], // very dark purple text
        on_surface:        [0x2A, 0x1C, 0x3E, 0xFF], // dark plum text
        error:             [0xB0, 0x00, 0x20, 0xFF], // deep red
    };

    pub fn from_mode(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Dark => Self::DARK,
            ThemeMode::Light => Self::LIGHT,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_background_is_opaque() {
        assert_eq!(Theme::DARK.background[3], 0xFF);
    }

    #[test]
    fn light_background_is_opaque() {
        assert_eq!(Theme::LIGHT.background[3], 0xFF);
    }

    #[test]
    fn from_mode_roundtrips() {
        assert_eq!(Theme::from_mode(ThemeMode::Dark), Theme::DARK);
        assert_eq!(Theme::from_mode(ThemeMode::Light), Theme::LIGHT);
    }

    #[test]
    fn primary_colors_distinct_between_modes() {
        assert_ne!(Theme::DARK.primary, Theme::LIGHT.primary);
    }
}
