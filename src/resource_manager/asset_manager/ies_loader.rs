//! IES photometric profile loader (IESNA LM-63 text format).
//!
//! Parses the angular intensity table from an `.ies` file, resamples it onto
//! a fixed 256×64 (vertical × horizontal) grid, and exposes the result as
//! a single `ThinVec<f32>` ready to upload as a 2D `R32_SFLOAT` GPU texture.
//!
//! The chit / fragment shader samples this texture by the cosine of the
//! angle between the light's negated direction (`-l`) and its forward axis
//! (vertical angle) plus an `atan2` of the radial components (horizontal
//! angle). The looked-up scalar multiplies the light's base intensity.
//!
//! Reference: ANSI/IES LM-63-02 "ANSI Approved Standard File Format for the
//! Electronic Transfer of Photometric Data and Related Information."

use thin_vec::ThinVec;

pub const IES_LUT_W: usize = 256; // horizontal angle resolution
pub const IES_LUT_H: usize = 64;  // vertical angle resolution

/// Parsed IES profile resampled onto the fixed LUT grid.
#[derive(Debug, Clone)]
pub struct IesProfile {
    pub width:  u32,                 // == IES_LUT_W
    pub height: u32,                 // == IES_LUT_H
    pub data:   ThinVec<f32>,        // length width*height; row-major (v * width + h)
    pub max_candela: f32,            // peak intensity (post-multiplier, no normalization)
}

/// Parse error categories — enough to give the user a useful message; we
/// don't lean on `thiserror` to keep the dep graph tight.
#[derive(Debug)]
pub enum IesError {
    UnexpectedEof,
    InvalidHeader,
    InvalidNumber(String),
    UnsupportedTilt,
}

impl std::fmt::Display for IesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IesError::UnexpectedEof          => write!(f, "unexpected EOF while parsing IES file"),
            IesError::InvalidHeader          => write!(f, "invalid IES header (expected IESNA: LM-63-* tag)"),
            IesError::InvalidNumber(s)       => write!(f, "could not parse IES number: {s:?}"),
            IesError::UnsupportedTilt        => write!(f, "IES TILT=INCLUDE not supported in this parser"),
        }
    }
}

impl std::error::Error for IesError {}

/// Parse an IES file from raw bytes.
pub fn parse(bytes: &[u8]) -> Result<IesProfile, IesError> {
    let text = std::str::from_utf8(bytes).map_err(|_| IesError::InvalidHeader)?;
    let mut lines = text.lines().peekable();

    // Skip header comments / TILT line — the actual numeric block begins
    // after "TILT=...". We accept TILT=NONE only; INCLUDE would require a
    // second numeric block we don't support.
    let mut found_tilt = false;
    while let Some(line) = lines.next() {
        let l = line.trim();
        if l.starts_with("TILT=") {
            if l != "TILT=NONE" {
                return Err(IesError::UnsupportedTilt);
            }
            found_tilt = true;
            break;
        }
    }
    if !found_tilt {
        return Err(IesError::InvalidHeader);
    }

    // Numeric block: tokenize the rest of the file by whitespace; every
    // value (header + tables) is whitespace-separated.
    let rest: String = lines.collect::<Vec<_>>().join(" ");
    let mut toks = rest.split_ascii_whitespace();
    let next_f = |s: &str, t: &mut std::str::SplitAsciiWhitespace| -> Result<f32, IesError> {
        let _ = s;
        let tok = t.next().ok_or(IesError::UnexpectedEof)?;
        tok.parse::<f32>().map_err(|_| IesError::InvalidNumber(tok.to_string()))
    };
    let next_u = |s: &str, t: &mut std::str::SplitAsciiWhitespace| -> Result<u32, IesError> {
        let _ = s;
        let tok = t.next().ok_or(IesError::UnexpectedEof)?;
        tok.parse::<u32>().map_err(|_| IesError::InvalidNumber(tok.to_string()))
    };

    // First numeric line: 10 values.
    let _num_lamps        = next_u("lamps",        &mut toks)?;
    let lumens_per_lamp   = next_f("lumens",       &mut toks)?;
    let candela_mult      = next_f("multiplier",   &mut toks)?;
    let n_vert            = next_u("n_vert",       &mut toks)? as usize;
    let n_horiz           = next_u("n_horiz",      &mut toks)? as usize;
    let _photometric_type = next_u("phot_type",    &mut toks)?;
    let _units_type       = next_u("units",        &mut toks)?;
    let _width            = next_f("width",        &mut toks)?;
    let _length           = next_f("length",       &mut toks)?;
    let _height           = next_f("height",       &mut toks)?;
    // Second numeric line: 3 values.
    let _ballast_factor   = next_f("ballast",      &mut toks)?;
    let _future_use       = next_f("future_use",   &mut toks)?;
    let _input_watts      = next_f("input_watts",  &mut toks)?;

    // Vertical angles (degrees, 0..180).
    let mut vert_angles: Vec<f32> = Vec::with_capacity(n_vert);
    for _ in 0..n_vert { vert_angles.push(next_f("v", &mut toks)?); }

    // Horizontal angles (degrees, 0..360).
    let mut horiz_angles: Vec<f32> = Vec::with_capacity(n_horiz);
    for _ in 0..n_horiz { horiz_angles.push(next_f("h", &mut toks)?); }

    // Candela table: n_horiz × n_vert values (per spec).
    let n_total = n_horiz.checked_mul(n_vert).ok_or(IesError::InvalidHeader)?;
    let mut candela: Vec<f32> = Vec::with_capacity(n_total);
    for _ in 0..n_total { candela.push(next_f("cd", &mut toks)?); }

    // ── Resample onto the fixed LUT grid ─────────────────────────────────
    let mut lut: ThinVec<f32> = ThinVec::with_capacity(IES_LUT_W * IES_LUT_H);
    lut.resize(IES_LUT_W * IES_LUT_H, 0.0);
    let mut max_cd = 0.0f32;
    let scale = candela_mult * if lumens_per_lamp > 0.0 { 1.0 } else { 1.0 };

    for v in 0..IES_LUT_H {
        // Vertical angle in degrees, 0..180 mapped to row 0..H-1.
        let theta = (v as f32 / (IES_LUT_H - 1) as f32) * 180.0;
        let (vi, vf) = lerp_index(&vert_angles, theta);

        for h in 0..IES_LUT_W {
            // Horizontal angle in degrees, 0..360 mapped to col 0..W-1.
            let phi = (h as f32 / IES_LUT_W as f32) * 360.0;
            let (hi, hf) = lerp_index(&horiz_angles, phi);

            let i00 = (hi    .min(n_horiz - 1)) * n_vert + (vi    .min(n_vert - 1));
            let i01 = (hi    .min(n_horiz - 1)) * n_vert + ((vi+1).min(n_vert - 1));
            let i10 = ((hi+1).min(n_horiz - 1)) * n_vert + (vi    .min(n_vert - 1));
            let i11 = ((hi+1).min(n_horiz - 1)) * n_vert + ((vi+1).min(n_vert - 1));
            let cd = bilinear(candela[i00], candela[i01], candela[i10], candela[i11], vf, hf) * scale;
            lut[v * IES_LUT_W + h] = cd;
            if cd > max_cd { max_cd = cd; }
        }
    }

    Ok(IesProfile {
        width:       IES_LUT_W as u32,
        height:      IES_LUT_H as u32,
        data:        lut,
        max_candela: max_cd,
    })
}

#[inline]
fn lerp_index(table: &[f32], query: f32) -> (usize, f32) {
    // Find the segment in `table` (sorted ascending) containing `query`.
    // Returns (lower_index, frac in 0..1 toward upper).
    if table.is_empty()    { return (0, 0.0); }
    if query <= table[0]   { return (0, 0.0); }
    let last = table.len() - 1;
    if query >= table[last] { return (last, 0.0); }

    // Linear scan — IES tables are tiny (typically < 200 entries).
    for i in 0..last {
        let lo = table[i];
        let hi = table[i + 1];
        if query >= lo && query <= hi {
            let span = (hi - lo).max(1.0e-6);
            return (i, (query - lo) / span);
        }
    }
    (last, 0.0)
}

#[inline]
fn bilinear(a00: f32, a01: f32, a10: f32, a11: f32, fv: f32, fh: f32) -> f32 {
    let x0 = a00 * (1.0 - fv) + a01 * fv;
    let x1 = a10 * (1.0 - fv) + a11 * fv;
    x0 * (1.0 - fh) + x1 * fh
}

/// Load a baked IES blob (`<name>.ies.f32` produced by `build.rs`). The
/// runtime engine includes the blob via `include_bytes!` and feeds it
/// straight to a `ForgeImage` as `R32_SFLOAT`.
pub fn load_baked(bytes: &'static [u8]) -> Option<IesProfile> {
    // Layout: width (u32 LE) | height (u32 LE) | width*height f32 LE.
    if bytes.len() < 8 { return None; }
    let w = u32::from_le_bytes(bytes[0..4].try_into().ok()?);
    let h = u32::from_le_bytes(bytes[4..8].try_into().ok()?);
    let expected = 8 + (w as usize) * (h as usize) * 4;
    if bytes.len() < expected { return None; }
    let mut data: ThinVec<f32> = ThinVec::with_capacity((w as usize) * (h as usize));
    let mut max_cd = 0.0f32;
    for i in 0..(w as usize) * (h as usize) {
        let off = 8 + i * 4;
        let v = f32::from_le_bytes(bytes[off..off+4].try_into().ok()?);
        if v > max_cd { max_cd = v; }
        data.push(v);
    }
    Some(IesProfile { width: w, height: h, data, max_candela: max_cd })
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const TINY_IES: &str = "\
IESNA:LM-63-2002
TILT=NONE
1 1000 1.0 2 2 1 1 0.0 0.0 0.0
1.0 0.0 100
0.0 90.0
0.0 90.0
500.0 250.0
250.0 100.0
";

    #[test]
    fn parses_minimal_ies() {
        let p = parse(TINY_IES.as_bytes()).unwrap();
        assert_eq!(p.width as usize,  IES_LUT_W);
        assert_eq!(p.height as usize, IES_LUT_H);
        assert!(p.max_candela > 0.0);
        // Top row corresponds to vertical = 0° → in the source the (h=0..1, v=0)
        // entries are 500 and 250 — interpolation should hit 500 near phi=0.
        let v0 = p.data[0];
        assert!((v0 - 500.0).abs() < 1.0e-3, "got {v0}");
    }

    #[test]
    fn rejects_tilt_include() {
        let bad = "IESNA:LM-63-2002\nTILT=INCLUDE\n";
        assert!(matches!(parse(bad.as_bytes()), Err(IesError::UnsupportedTilt)));
    }
}
