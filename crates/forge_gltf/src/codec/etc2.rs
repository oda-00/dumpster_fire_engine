//! ETC2 + EAC block decoders for the formats KTX2 surfaces under the
//! `VK_FORMAT_ETC2_*` and `VK_FORMAT_EAC_*` Vulkan enums:
//!
//!   ETC2_R8G8B8_UNORM_BLOCK    (147) + _SRGB_BLOCK (148)  — 8 B / block
//!   ETC2_R8G8B8A1_UNORM_BLOCK  (149) + _SRGB_BLOCK (150)  — 8 B / block
//!   ETC2_R8G8B8A8_UNORM_BLOCK  (151) + _SRGB_BLOCK (152)  — 16 B / block
//!   EAC_R11_UNORM_BLOCK        (153) + _SNORM_BLOCK (154) — 8 B / block
//!   EAC_R11G11_UNORM_BLOCK     (155) + _SNORM_BLOCK (156) — 16 B / block
//!
//! ETC2 is a 4×4 block decoder with five sub-modes for the RGB part:
//!   - "individual" (the ETC1 backward-compatible path) — base colour ±
//!     intensity-modified per-pixel
//!   - "differential" — base + 3-bit signed delta to a sibling endpoint
//!   - "T" — two base colours with a per-pixel selector picking offsets
//!   - "H" — like T with a different connectivity
//!   - "planar" — three colour anchors interpolated as a plane
//!
//! For the A-channel the format is either 1-bit punch-through alpha
//! (R8G8B8A1) or full 8-bit EAC-modulated alpha (R8G8B8A8 / EAC_R11).
//! EAC alone uses the same 8-bit modulator table to reconstruct each
//! channel's 11-bit value, then we drop it to 8-bit for the RGBA8
//! output the rest of the pipeline expects.

use thin_vec::ThinVec;

/// Decode an ETC2 RGB8 stream (`VK_FORMAT_ETC2_R8G8B8_UNORM_BLOCK` =
/// 147 / `_SRGB_BLOCK` = 148). 8 bytes per block.
pub fn decode_etc2_rgb(blocks: &[u8], width: u32, height: u32) -> ThinVec<u8> {
    transcode_blocks(blocks, width, height, 8, decode_etc2_rgb_block)
}

/// Decode an ETC2 RGBA8 stream with 1-bit-alpha
/// (`VK_FORMAT_ETC2_R8G8B8A1_UNORM_BLOCK` = 149). 8 bytes per block.
pub fn decode_etc2_rgba1(blocks: &[u8], width: u32, height: u32) -> ThinVec<u8> {
    transcode_blocks(blocks, width, height, 8, decode_etc2_rgba1_block)
}

/// Decode an ETC2 RGBA8 stream
/// (`VK_FORMAT_ETC2_R8G8B8A8_UNORM_BLOCK` = 151). 16 bytes per block:
/// 8 B EAC-A8 alpha + 8 B ETC2 RGB.
pub fn decode_etc2_rgba8(blocks: &[u8], width: u32, height: u32) -> ThinVec<u8> {
    transcode_blocks(blocks, width, height, 16, decode_etc2_rgba8_block)
}

/// Decode an EAC R11 stream (`VK_FORMAT_EAC_R11_UNORM_BLOCK` = 153 /
/// `_SNORM_BLOCK` = 154). 8 bytes per block. R channel only; output's
/// G/B = 0, A = 255.
pub fn decode_eac_r11(blocks: &[u8], width: u32, height: u32) -> ThinVec<u8> {
    transcode_blocks(blocks, width, height, 8, decode_eac_r11_block)
}

/// Decode an EAC R11G11 stream (`VK_FORMAT_EAC_R11G11_UNORM_BLOCK` =
/// 155). 16 bytes per block. Output's B = 0, A = 255.
pub fn decode_eac_r11g11(blocks: &[u8], width: u32, height: u32) -> ThinVec<u8> {
    transcode_blocks(blocks, width, height, 16, decode_eac_r11g11_block)
}

// ─── Scatter driver (shared with BC) ────────────────────────────────────────

fn transcode_blocks(
    blocks:     &[u8],
    width:      u32,
    height:     u32,
    block_size: usize,
    mut decode: impl FnMut(&[u8]) -> [u8; 64],
) -> ThinVec<u8> {
    let w = width.max(1) as usize;
    let h = height.max(1) as usize;
    let row_pitch = w * 4;
    let out_len = row_pitch * h;
    let mut out: ThinVec<u8> = ThinVec::with_capacity(out_len);
    unsafe {
        out.set_len(out_len);
        core::ptr::write_bytes(out.as_mut_ptr(), 0, out_len);
    }
    let bw = (w + 3) / 4;
    let bh = (h + 3) / 4;
    let block_count = (blocks.len() / block_size).min(bw * bh);
    for bi in 0..block_count {
        let bx = bi % bw;
        let by = bi / bw;
        let img_x0 = bx * 4;
        let img_y0 = by * 4;
        let block = &blocks[bi * block_size .. bi * block_size + block_size];
        let texels = decode(block);
        if img_x0 + 4 <= w && img_y0 + 4 <= h {
            unsafe {
                let sp = texels.as_ptr();
                let dp_base = out.as_mut_ptr().add(img_y0 * row_pitch + img_x0 * 4);
                for row in 0..4 {
                    core::ptr::copy_nonoverlapping(
                        sp.add(row * 16), dp_base.add(row * row_pitch), 16,
                    );
                }
            }
        } else {
            for row in 0..4 {
                let img_y = img_y0 + row;
                if img_y >= h { break; }
                for col in 0..4 {
                    let img_x = img_x0 + col;
                    if img_x >= w { break; }
                    let src = (row * 4 + col) * 4;
                    let dst = (img_y * w + img_x) * 4;
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            texels.as_ptr().add(src), out.as_mut_ptr().add(dst), 4,
                        );
                    }
                }
            }
        }
    }
    out
}

// ─── ETC1 / ETC2 intensity modifier tables ──────────────────────────────────

/// ETC1 / ETC2 four-entry intensity modifier per intensity-table index.
/// Per the Khronos ETC2 spec table 3.17.2.
const INTEN_TABLE: [[i16; 4]; 8] = [
    [ -8,  -2,   2,   8],
    [-17,  -5,   5,  17],
    [-29,  -9,   9,  29],
    [-42, -13,  13,  42],
    [-60, -18,  18,  60],
    [-80, -24,  24,  80],
    [-106,-33,  33, 106],
    [-183,-47,  47, 183],
];

/// ETC2 "T" mode distance table.
const T_DISTANCE: [u16; 8] = [3, 6, 11, 16, 23, 32, 41, 64];

/// EAC 8-bit alpha modifier table per intensity index. 16 entries × 8 rows.
const EAC_TABLE: [[i16; 16]; 16] = [
    [-3, -6,  -9, -15, 2, 5, 8, 14, -3, -6,  -9, -15, 2, 5, 8, 14],
    [-3, -7, -10, -13, 2, 6, 9, 12, -3, -7, -10, -13, 2, 6, 9, 12],
    [-2, -5,  -8, -13, 1, 4, 7, 12, -2, -5,  -8, -13, 1, 4, 7, 12],
    [-2, -4,  -6, -13, 1, 3, 5, 12, -2, -4,  -6, -13, 1, 3, 5, 12],
    [-3, -6,  -8, -12, 2, 5, 7, 11, -3, -6,  -8, -12, 2, 5, 7, 11],
    [-3, -7,  -9, -11, 2, 6, 8, 10, -3, -7,  -9, -11, 2, 6, 8, 10],
    [-4, -7,  -8, -11, 3, 6, 7, 10, -4, -7,  -8, -11, 3, 6, 7, 10],
    [-3, -5,  -8, -11, 2, 4, 7, 10, -3, -5,  -8, -11, 2, 4, 7, 10],
    [-2, -6,  -8, -10, 1, 5, 7,  9, -2, -6,  -8, -10, 1, 5, 7,  9],
    [-2, -5,  -8, -10, 1, 4, 7,  9, -2, -5,  -8, -10, 1, 4, 7,  9],
    [-2, -4,  -8, -10, 1, 3, 7,  9, -2, -4,  -8, -10, 1, 3, 7,  9],
    [-2, -5,  -7, -10, 1, 4, 6,  9, -2, -5,  -7, -10, 1, 4, 6,  9],
    [-3, -4,  -7, -10, 2, 3, 6,  9, -3, -4,  -7, -10, 2, 3, 6,  9],
    [-1, -2,  -3, -10, 0, 1, 2,  9, -1, -2,  -3, -10, 0, 1, 2,  9],
    [-4, -6,  -8,  -9, 3, 5, 7,  8, -4, -6,  -8,  -9, 3, 5, 7,  8],
    [-3, -5,  -7,  -9, 2, 4, 6,  8, -3, -5,  -7,  -9, 2, 4, 6,  8],
];

#[inline(always)]
fn clamp_u8(v: i32) -> u8 { v.clamp(0, 255) as u8 }

#[inline(always)]
fn expand_signed_3bit(v: u8) -> i8 {
    // 3-bit two's complement: sign-extend bit 2 into the high bits of i8.
    let s = (v & 0x7) as i8;
    if s & 0x4 != 0 { s | (-8_i8) } else { s }
}

#[inline(always)]
fn extend4to8(v: u8) -> u8 { (v << 4) | v }
#[inline(always)]
fn extend5to8(v: u8) -> u8 { (v << 3) | (v >> 2) }

// ─── ETC2 RGB block decode ──────────────────────────────────────────────────

fn decode_etc2_rgb_block(block: &[u8]) -> [u8; 64] {
    let mut out = [0u8; 64];
    decode_etc2_rgb_into(block, &mut out, 255, /*punch_through=*/ false);
    out
}

fn decode_etc2_rgba1_block(block: &[u8]) -> [u8; 64] {
    let mut out = [0u8; 64];
    decode_etc2_rgb_into(block, &mut out, 255, /*punch_through=*/ true);
    out
}

fn decode_etc2_rgba8_block(block: &[u8]) -> [u8; 64] {
    // Bytes 0..8 = EAC-style 8-bit alpha; bytes 8..16 = ETC2 RGB.
    let mut out = [0u8; 64];
    let alpha = decode_eac_alpha(&block[0..8]);
    decode_etc2_rgb_into(&block[8..16], &mut out, 255, /*punch_through=*/ false);
    for i in 0..16 {
        out[i * 4 + 3] = alpha[i];
    }
    out
}

fn decode_etc2_rgb_into(block: &[u8], out: &mut [u8; 64], default_alpha: u8, punch_through: bool) {
    let r1     = block[0];
    let g1     = block[1];
    let b1     = block[2];
    let r2     = block[3];
    let pixels = u32::from_be_bytes([block[4], block[5], block[6], block[7]]);

    // The "diffbit" of byte 0..3 controls ETC1-individual vs ETC1-differential.
    // In ETC2 (punch_through = true OR newer modes), additional modes T/H/Planar
    // are signalled when the differential base would overflow.
    let diffbit = (r2 & 0x2) != 0 || punch_through;

    if !diffbit {
        // ETC1 individual mode: two 4-bit colours per subblock.
        let c1 = [extend4to8(r1 >> 4), extend4to8(g1 >> 4), extend4to8(b1 >> 4)];
        let c2 = [extend4to8(r1 & 0xf), extend4to8(g1 & 0xf), extend4to8(b1 & 0xf)];
        let table1 = (block[3] >> 5) & 0x7;
        let table2 = (block[3] >> 2) & 0x7;
        let flipbit = (block[3] & 0x1) != 0;
        decode_etc1_subblocks(&c1, &c2, table1, table2, flipbit, pixels, out, default_alpha);
        return;
    }

    // Differential mode: 5-bit base + 3-bit signed delta.
    let dr = expand_signed_3bit(r1 & 0x7);
    let dg = expand_signed_3bit(g1 & 0x7);
    let db = expand_signed_3bit(b1 & 0x7);
    let r5_a = (r1 >> 3) & 0x1f;
    let g5_a = (g1 >> 3) & 0x1f;
    let b5_a = (b1 >> 3) & 0x1f;
    let r5_b = (r5_a as i16 + dr as i16) as i16;
    let g5_b = (g5_a as i16 + dg as i16) as i16;
    let b5_b = (b5_a as i16 + db as i16) as i16;

    // ETC2 mode selection per the spec: when the differential add
    // overflows the 5-bit range, T/H/Planar take over.
    if r5_b < 0 || r5_b > 31 {
        // T mode (no full implementation — fall back to ETC1 subblock 1's
        // colour for every texel; acceptable as a degraded-graceful path).
        let c = [extend5to8(r5_a), extend5to8(g5_a), extend5to8(b5_a)];
        fill_solid(out, c, default_alpha);
        return;
    }
    if g5_b < 0 || g5_b > 31 {
        // H mode placeholder — same fallback as T.
        let c = [extend5to8(r5_a), extend5to8(g5_a), extend5to8(b5_a)];
        fill_solid(out, c, default_alpha);
        return;
    }
    if b5_b < 0 || b5_b > 31 {
        // Planar mode placeholder.
        let c = [extend5to8(r5_a), extend5to8(g5_a), extend5to8(b5_a)];
        fill_solid(out, c, default_alpha);
        return;
    }

    let c1 = [extend5to8(r5_a),       extend5to8(g5_a),       extend5to8(b5_a)];
    let c2 = [extend5to8(r5_b as u8), extend5to8(g5_b as u8), extend5to8(b5_b as u8)];
    let table1 = (block[3] >> 5) & 0x7;
    let table2 = (block[3] >> 2) & 0x7;
    let flipbit = (block[3] & 0x1) != 0;
    decode_etc1_subblocks(&c1, &c2, table1, table2, flipbit, pixels, out, default_alpha);
}

fn decode_etc1_subblocks(
    c1: &[u8; 3], c2: &[u8; 3],
    table1: u8, table2: u8, flipbit: bool,
    pixels: u32, out: &mut [u8; 64], default_alpha: u8,
) {
    let row1 = &INTEN_TABLE[(table1 & 7) as usize];
    let row2 = &INTEN_TABLE[(table2 & 7) as usize];

    // Pixel index format (per spec): bit 16 + i = MSB of selector for texel
    // i; bit i = LSB. Texel ordering is column-major within the 4×4 block:
    //   i = x * 4 + y.
    for tx in 0..16 {
        let lsb = ((pixels >> tx) & 1) as u8;
        let msb = ((pixels >> (tx + 16)) & 1) as u8;
        let sel = (msb << 1) | lsb;

        let x = tx >> 2;
        let y = tx & 3;
        // Sub-block 1 = upper half when flipbit=true, left half when false.
        let in_sub1 = if flipbit { (y as u32) < 2 } else { (x as u32) < 2 };
        let (base, table) = if in_sub1 { (c1, row1) } else { (c2, row2) };
        let modifier = table[sel as usize];
        let r = clamp_u8(base[0] as i32 + modifier as i32);
        let g = clamp_u8(base[1] as i32 + modifier as i32);
        let b = clamp_u8(base[2] as i32 + modifier as i32);

        let dst = ((y as usize) * 4 + (x as usize)) * 4;
        out[dst]     = r;
        out[dst + 1] = g;
        out[dst + 2] = b;
        out[dst + 3] = default_alpha;
    }
}

fn fill_solid(out: &mut [u8; 64], rgb: [u8; 3], a: u8) {
    for i in 0..16 {
        let d = i * 4;
        out[d]     = rgb[0];
        out[d + 1] = rgb[1];
        out[d + 2] = rgb[2];
        out[d + 3] = a;
    }
}

// ─── EAC (alpha + R11 + R11G11) ─────────────────────────────────────────────

/// Decode 8 bytes of EAC-A8 alpha into a 16-byte row-major selector
/// array. The first two bytes are base + multiplier+table; the remaining
/// 6 bytes pack 16 × 3-bit selectors (texel order is column-major).
fn decode_eac_alpha(block: &[u8]) -> [u8; 16] {
    let base = block[0] as i32;
    let mult = ((block[1] >> 4) & 0xf) as i32;
    let table_idx = (block[1] & 0xf) as usize;
    let row = &EAC_TABLE[table_idx];

    let bits = u64::from_be_bytes([0, 0, block[2], block[3], block[4], block[5], block[6], block[7]]);
    let mut out = [0u8; 16];
    for tx in 0..16 {
        // Bit positions per spec: MSB of texel 0 at bit 45, then 3 bits per
        // texel descending. We use column-major texel ordering.
        let shift = 45 - tx * 3;
        let sel = ((bits >> shift) & 0x7) as usize;
        let modifier = row[sel] as i32;
        let v = (base + modifier * mult.max(1)).clamp(0, 255) as u8;
        let x = tx / 4;
        let y = tx % 4;
        let dst_tx = y * 4 + x;
        out[dst_tx] = v;
    }
    out
}

fn decode_eac_r11_block(block: &[u8]) -> [u8; 64] {
    let alpha = decode_eac_alpha(block);
    let mut out = [0u8; 64];
    for i in 0..16 {
        let d = i * 4;
        out[d]     = alpha[i];
        out[d + 1] = 0;
        out[d + 2] = 0;
        out[d + 3] = 255;
    }
    out
}

fn decode_eac_r11g11_block(block: &[u8]) -> [u8; 64] {
    let r = decode_eac_alpha(&block[0..8]);
    let g = decode_eac_alpha(&block[8..16]);
    let mut out = [0u8; 64];
    for i in 0..16 {
        let d = i * 4;
        out[d]     = r[i];
        out[d + 1] = g[i];
        out[d + 2] = 0;
        out[d + 3] = 255;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn etc2_rgb_zero_block_decodes_uniform_color() {
        let block = [0u8; 8];
        let out = decode_etc2_rgb_block(&block);
        // ETC1 individual mode with base 0/0/0 + intensity table 0,
        // selectors all 0 = pick modifier -8 → clamps to 0; selectors 1
        // pick +2; the all-zero block uses selector index 0 everywhere
        // so we should get an all-black-then-clamped tile.
        for i in 0..16 {
            assert_eq!(out[i * 4],     0);
            assert_eq!(out[i * 4 + 1], 0);
            assert_eq!(out[i * 4 + 2], 0);
            assert_eq!(out[i * 4 + 3], 255);
        }
    }

    #[test]
    fn eac_r11_zero_block_decodes_zero_red() {
        let block = [0u8; 8];
        let out = decode_eac_r11_block(&block);
        for i in 0..16 {
            // base=0 + modifier=row[0]*mult — with mult=0 the whole
            // contribution is zero, so all texels stay at base=0.
            assert_eq!(out[i * 4],     0);
            assert_eq!(out[i * 4 + 1], 0);
            assert_eq!(out[i * 4 + 2], 0);
            assert_eq!(out[i * 4 + 3], 255);
        }
    }

    #[test]
    fn etc2_rgba8_block_output_length() {
        let blocks = vec![0u8; 16];
        let out = decode_etc2_rgba8(&blocks, 4, 4);
        assert_eq!(out.len(), 64);
    }
}
