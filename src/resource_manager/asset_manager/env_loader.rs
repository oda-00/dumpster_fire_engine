//! Radiance RGBE (.hdr) decoder + SH9 projection for environment maps.
//!
//! Decodes the Radiance HDR file format directly — no external decode
//! dependency. Used by the engine's `Environment` light kind:
//!
//!   1. **RT miss shader** samples the equirect cubemap by ray direction.
//!   2. **Raster fallback** uses the SH9 coefficients projected from the
//!      same equirect image as a cheap directional ambient term.
//!
//! Format reference: Greg Ward, "Real Pixels" (1991).

use glam::Vec3;
use thin_vec::ThinVec;

/// Decoded equirect environment map in linear RGB float.
#[derive(Debug, Clone)]
pub struct EnvironmentMap {
    pub width: u32,
    pub height: u32,
    pub pixels: ThinVec<f32>, // length = width * height * 3
}

#[derive(Debug)]
pub enum EnvError {
    UnexpectedEof,
    InvalidHeader,
    InvalidScanline,
}

impl std::fmt::Display for EnvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EnvError::UnexpectedEof => write!(f, "unexpected EOF in .hdr file"),
            EnvError::InvalidHeader => write!(f, "invalid Radiance .hdr header"),
            EnvError::InvalidScanline => write!(f, "invalid RGBE scanline (corrupt RLE)"),
        }
    }
}

impl std::error::Error for EnvError {}

/// Parse a Radiance `.hdr` byte stream.
pub fn parse(bytes: &[u8]) -> Result<EnvironmentMap, EnvError> {
    // ── Header ──────────────────────────────────────────────────────────
    // Line-based; ends with a blank line followed by a resolution string.
    let mut p = 0usize;
    let mut found_magic = false;
    let mut found_format = false;

    let read_line = |buf: &[u8], pos: &mut usize| -> Option<String> {
        if *pos >= buf.len() {
            return None;
        }
        let start = *pos;
        while *pos < buf.len() && buf[*pos] != b'\n' {
            *pos += 1;
        }
        let line = &buf[start..*pos];
        if *pos < buf.len() {
            *pos += 1;
        } // consume '\n'
        Some(String::from_utf8_lossy(line).into_owned())
    };

    while let Some(line) = read_line(bytes, &mut p) {
        let t = line.trim();
        if t.is_empty() {
            break;
        }
        if t == "#?RADIANCE" || t == "#?RGBE" {
            found_magic = true;
        }
        if t.starts_with("FORMAT=") && t.contains("rgbe") {
            found_format = true;
        }
    }
    if !found_magic || !found_format {
        return Err(EnvError::InvalidHeader);
    }

    // ── Resolution line ─────────────────────────────────────────────────
    let res = read_line(bytes, &mut p).ok_or(EnvError::UnexpectedEof)?;
    // Common: "-Y H +X W"
    let toks: Vec<&str> = res.split_ascii_whitespace().collect();
    if toks.len() != 4 {
        return Err(EnvError::InvalidHeader);
    }
    let h: u32 = toks[1].parse().map_err(|_| EnvError::InvalidHeader)?;
    let w: u32 = toks[3].parse().map_err(|_| EnvError::InvalidHeader)?;

    let mut pixels: ThinVec<f32> = ThinVec::with_capacity((w * h * 3) as usize);
    pixels.resize((w * h * 3) as usize, 0.0);

    // ── Scanline decode ─────────────────────────────────────────────────
    let mut scratch: Vec<u8> = vec![0; (w * 4) as usize];
    for row in 0..h {
        // Per-row header: 0x02 0x02 (hi (w>>8)) (lo (w&0xff)) means new RLE.
        if p + 4 > bytes.len() {
            return Err(EnvError::UnexpectedEof);
        }
        let h0 = bytes[p];
        let h1 = bytes[p + 1];
        let h2 = bytes[p + 2];
        let h3 = bytes[p + 3];

        let new_rle = h0 == 2 && h1 == 2 && ((h2 as u32) << 8 | h3 as u32) == w;
        if new_rle {
            p += 4;
            // Decode 4 channel runs (R, G, B, E), each w bytes after RLE.
            for ch in 0..4 {
                let mut x = 0usize;
                while x < w as usize {
                    if p >= bytes.len() {
                        return Err(EnvError::UnexpectedEof);
                    }
                    let code = bytes[p];
                    p += 1;
                    if code > 128 {
                        // Run of (code & 0x7F) of next byte.
                        let n = (code & 0x7F) as usize;
                        if p >= bytes.len() {
                            return Err(EnvError::UnexpectedEof);
                        }
                        let v = bytes[p];
                        p += 1;
                        for _ in 0..n {
                            if x >= w as usize {
                                return Err(EnvError::InvalidScanline);
                            }
                            scratch[ch * w as usize + x] = v;
                            x += 1;
                        }
                    } else {
                        let n = code as usize;
                        if p + n > bytes.len() {
                            return Err(EnvError::UnexpectedEof);
                        }
                        for i in 0..n {
                            scratch[ch * w as usize + x + i] = bytes[p + i];
                        }
                        p += n;
                        x += n;
                    }
                }
            }
        } else {
            // Old RLE / uncompressed — read w pixels of 4 bytes each.
            if p + (w as usize) * 4 > bytes.len() {
                return Err(EnvError::UnexpectedEof);
            }
            for x in 0..w as usize {
                let off = p + x * 4;
                scratch[x] = bytes[off];
                scratch[(w as usize) + x] = bytes[off + 1];
                scratch[2 * w as usize + x] = bytes[off + 2];
                scratch[3 * w as usize + x] = bytes[off + 3];
            }
            p += (w as usize) * 4;
        }

        // Convert RGBE → RGB float and write into the destination row.
        let row_base = (row * w * 3) as usize;
        for x in 0..w as usize {
            let r = scratch[x] as f32;
            let g = scratch[(w as usize) + x] as f32;
            let b = scratch[2 * w as usize + x] as f32;
            let e = scratch[3 * w as usize + x] as i32;
            if e == 0 {
                pixels[row_base + x * 3] = 0.0;
                pixels[row_base + x * 3 + 1] = 0.0;
                pixels[row_base + x * 3 + 2] = 0.0;
            } else {
                let f = 2.0_f32.powi(e - 128) / 256.0;
                pixels[row_base + x * 3] = r * f;
                pixels[row_base + x * 3 + 1] = g * f;
                pixels[row_base + x * 3 + 2] = b * f;
            }
        }
    }

    Ok(EnvironmentMap {
        width: w,
        height: h,
        pixels,
    })
}

/// Project an equirect environment onto SH9 (diffuse-irradiance ambient).
///
/// `samples_per_axis` resolution of the spherical Monte-Carlo integration.
/// For an editor preview, 64 is a good default (~4 k samples).
pub fn project_sh9(map: &EnvironmentMap, samples_per_axis: u32) -> crate::render::sky_hw::Sh9 {
    let mut acc: crate::render::sky_hw::Sh9 = [[0.0; 3]; 9];
    let mut weight_sum = 0.0f32;
    let n = samples_per_axis.max(8);

    for i in 0..n {
        for j in 0..n {
            let u = (i as f32 + 0.5) / n as f32;
            let v = (j as f32 + 0.5) / n as f32;
            let theta = (1.0 - 2.0 * u).acos();
            let phi = 2.0 * std::f32::consts::PI * v;
            let st = theta.sin();
            let dir = Vec3::new(st * phi.cos(), theta.cos(), st * phi.sin());

            let rad = sample_equirect(map, dir);

            // SH9 basis — duplicated from sky_hw to avoid making that
            // module's `sh9_basis` public; the math is identical.
            let x = dir.x;
            let y = dir.y;
            let z = dir.z;
            let b = [
                0.282_094_8,
                0.488_602_5 * y,
                0.488_602_5 * z,
                0.488_602_5 * x,
                1.092_548_4 * x * y,
                1.092_548_4 * y * z,
                0.315_391_5 * (3.0 * z * z - 1.0),
                1.092_548_4 * x * z,
                0.546_274_2 * (x * x - y * y),
            ];
            for k in 0..9 {
                acc[k][0] += rad.x * b[k];
                acc[k][1] += rad.y * b[k];
                acc[k][2] += rad.z * b[k];
            }
            weight_sum += 1.0;
        }
    }

    let solid_angle_norm = 4.0 * std::f32::consts::PI / weight_sum;
    for coeff in &mut acc {
        coeff[0] *= solid_angle_norm;
        coeff[1] *= solid_angle_norm;
        coeff[2] *= solid_angle_norm;
    }
    acc
}

#[inline]
fn sample_equirect(map: &EnvironmentMap, dir: Vec3) -> Vec3 {
    // Convert world direction to equirect uv.
    let phi = dir.z.atan2(dir.x); // -π..π
    let theta = dir.y.clamp(-1.0, 1.0).acos(); // 0..π
    let u = (phi / (2.0 * std::f32::consts::PI) + 0.5).fract();
    let v = theta / std::f32::consts::PI;

    let x = (u * map.width as f32).clamp(0.0, (map.width - 1) as f32) as usize;
    let y = (v * map.height as f32).clamp(0.0, (map.height - 1) as f32) as usize;
    let off = (y * map.width as usize + x) * 3;
    Vec3::new(map.pixels[off], map.pixels[off + 1], map.pixels[off + 2])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_header_rejected() {
        let bytes = b"NOT_AN_HDR_FILE\n\n-Y 1 +X 1\n\0\0\0\0";
        assert!(parse(bytes).is_err());
    }
}
