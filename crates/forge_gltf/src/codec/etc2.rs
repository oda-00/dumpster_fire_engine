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

/// ETC2 "T" mode distance table — Khronos ETC2 spec table 3.17.5.
/// The T mode uses two 4-bit RGB base colours plus a distance picked
/// from this table; each texel's 2-bit selector decodes to one of
/// {c0, c0+d, c1, c1-d}. Same table is reused by H mode.
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
    // overflows the 5-bit range, T/H/Planar take over. The branching
    // order is fixed by the spec — R overflow → T, G overflow → H,
    // B overflow → Planar.
    if r5_b < 0 || r5_b > 31 {
        decode_etc2_t_mode(block, out, default_alpha);
        return;
    }
    if g5_b < 0 || g5_b > 31 {
        decode_etc2_h_mode(block, out, default_alpha);
        return;
    }
    if b5_b < 0 || b5_b > 31 {
        decode_etc2_planar_mode(block, out, default_alpha);
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

    // Build 4-RGBA palettes per sub-block (base ± intensity modifier per
    // selector). Once a block's palettes are known, every texel reduces
    // to a 2-bit lookup → SIMD pshufb gather covers 4 texels per shuffle.
    let pal1 = make_etc1_palette(c1, row1, default_alpha);
    let pal2 = make_etc1_palette(c2, row2, default_alpha);

    #[cfg(target_arch = "x86_64")]
    unsafe {
        decode_etc1_subblocks_pshufb(&pal1, &pal2, flipbit, pixels, out);
        return;
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        // Pixel index format (per spec): bit 16 + i = MSB of selector for
        // texel i; bit i = LSB. Texel ordering is column-major within the
        // 4×4 block: i = x * 4 + y.
        for tx in 0..16usize {
            let lsb = ((pixels >> tx) & 1) as u8;
            let msb = ((pixels >> (tx + 16)) & 1) as u8;
            let sel = (msb << 1) | lsb;
            let x = tx >> 2;
            let y = tx & 3;
            let in_sub1 = if flipbit { (y as u32) < 2 } else { (x as u32) < 2 };
            let pal = if in_sub1 { &pal1 } else { &pal2 };
            let src = (sel as usize) * 4;
            let dst = (y * 4 + x) * 4;
            out[dst]     = pal[src];
            out[dst + 1] = pal[src + 1];
            out[dst + 2] = pal[src + 2];
            out[dst + 3] = pal[src + 3];
        }
    }
}

/// Build a 4-RGBA palette (16 bytes) for one ETC1 sub-block: each of the
/// 4 selector values picks one of four base ± modifier intensities.
fn make_etc1_palette(base: &[u8; 3], table: &[i16; 4], default_alpha: u8) -> [u8; 16] {
    let mut p = [0u8; 16];
    for sel in 0..4 {
        let m = table[sel] as i32;
        p[sel * 4    ] = clamp_u8(base[0] as i32 + m);
        p[sel * 4 + 1] = clamp_u8(base[1] as i32 + m);
        p[sel * 4 + 2] = clamp_u8(base[2] as i32 + m);
        p[sel * 4 + 3] = default_alpha;
    }
    p
}

/// SSSE3 ETC1 subblock decode. Output rows are processed one at a time;
/// for each row we know which 4 texels land in it and which sub-block
/// each belongs to (determined by flipbit + texel column/row position).
///
/// flipbit=true  → entire row in one sub-block (horizontal split).
/// flipbit=false → row mixes sub1 (cols 0-1) and sub2 (cols 2-3) per
///                 texel (vertical split); two pshufbs + byte-blend.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "ssse3")]
unsafe fn decode_etc1_subblocks_pshufb(
    pal1: &[u8; 16], pal2: &[u8; 16],
    flipbit: bool, pixels: u32, out: &mut [u8; 64],
) {
    use std::arch::x86_64::*;
    unsafe {
        let p1_v = _mm_loadu_si128(pal1.as_ptr() as *const __m128i);
        let p2_v = _mm_loadu_si128(pal2.as_ptr() as *const __m128i);

        // Selector for texel tx = x*4 + y: lsb = bit tx, msb = bit (tx+16).
        // For output row y, we want texels at (x=0..3, y) → tx values
        // 0+y, 4+y, 8+y, 12+y.
        let sel = |tx: usize| -> u8 {
            let lsb = ((pixels >> tx) & 1) as u8;
            let msb = ((pixels >> (tx + 16)) & 1) as u8;
            (msb << 1) | lsb
        };

        for y in 0..4usize {
            let s0 = sel(0 + y) as i8;
            let s1 = sel(4 + y) as i8;
            let s2 = sel(8 + y) as i8;
            let s3 = sel(12 + y) as i8;
            let shuf = _mm_setr_epi8(
                s0 * 4, s0 * 4 + 1, s0 * 4 + 2, s0 * 4 + 3,
                s1 * 4, s1 * 4 + 1, s1 * 4 + 2, s1 * 4 + 3,
                s2 * 4, s2 * 4 + 1, s2 * 4 + 2, s2 * 4 + 3,
                s3 * 4, s3 * 4 + 1, s3 * 4 + 2, s3 * 4 + 3,
            );

            // Pick palette (or blend two) for this row.
            let row_v = if flipbit {
                // Whole row in one sub-block: y < 2 → sub1, else sub2.
                if y < 2 { _mm_shuffle_epi8(p1_v, shuf) }
                else     { _mm_shuffle_epi8(p2_v, shuf) }
            } else {
                // Per-texel split at column 2. Gather from both palettes,
                // mask-blend the four 4-byte pixels: pixels 0,1 from sub1,
                // pixels 2,3 from sub2.
                let from_p1 = _mm_shuffle_epi8(p1_v, shuf);
                let from_p2 = _mm_shuffle_epi8(p2_v, shuf);
                // Mask: 0x00 for sub1 bytes (pixels 0,1 = bytes 0..7),
                //       0xFF for sub2 bytes (pixels 2,3 = bytes 8..15).
                let mask = _mm_setr_epi8(
                    0, 0, 0, 0, 0, 0, 0, 0,
                    -1, -1, -1, -1, -1, -1, -1, -1,
                );
                // result = (from_p1 & ~mask) | (from_p2 & mask) using
                // SSE2 ops (no SSE4.1 _mm_blendv_epi8 dependency).
                let keep_p1 = _mm_andnot_si128(mask, from_p1);
                let keep_p2 = _mm_and_si128(mask, from_p2);
                _mm_or_si128(keep_p1, keep_p2)
            };
            _mm_storeu_si128(out.as_mut_ptr().add(y * 16) as *mut __m128i, row_v);
        }
    }
}

/// Decode the per-texel selector bit pattern that ETC1/ETC2 uses.
/// Returns the 2-bit selector for texel index `tx` (column-major:
/// `tx = x * 4 + y`). `pixels` is the u32 of bytes 4..8.
#[inline]
fn etc2_selector(pixels: u32, tx: usize) -> u8 {
    let lsb = ((pixels >> tx) & 1) as u8;
    let msb = ((pixels >> (tx + 16)) & 1) as u8;
    (msb << 1) | lsb
}

/// Scatter a column-major texel index `tx` to its row-major byte offset
/// in the 16-texel × 4-byte output block.
#[inline]
fn etc2_dst_offset(tx: usize) -> usize {
    let x = tx >> 2;
    let y = tx & 3;
    (y * 4 + x) * 4
}

/// ETC2 T-mode decode. Per Khronos ETC2 spec table 3.17.5: two 4-bit
/// base colours + a 3-bit distance index. Each texel's 2-bit selector
/// picks one of {c0, c0+d, c1, c1-d} (the second variant is c0 plus
/// the distance, the fourth is c1 minus the distance).
///
/// Header bit layout in bytes b0..b3 (bit numbering with bit 63 = MSB
/// of b0, i.e. high bit on disk):
/// - R0[3..2] = b0[4..3]      (bits 60..59 in the 64-bit stream)
/// - R0[1..0] = b0[1..0]      (bits 56 minus the gap layout: byte 0 low 2)
/// - G0[3..0] = b1[7..4]
/// - B0[3..0] = b1[3..0]
/// - R1[3..0] = b2[7..4]
/// - G1[3..0] = b2[3..0]
/// - B1[3..0] = b3[7..4]
/// - da[1..0] = b3[3..2]       (high 2 bits of distance index)
/// - db        = b3[0]          (low bit of distance index)
fn decode_etc2_t_mode(block: &[u8], out: &mut [u8; 64], default_alpha: u8) {
    let r0 = ((block[0] >> 1) & 0x0C) | (block[0] & 0x03);
    let g0 = block[1] >> 4;
    let b0 = block[1] & 0x0F;
    let r1 = block[2] >> 4;
    let g1 = block[2] & 0x0F;
    let b1 = block[3] >> 4;
    let d_idx = (((block[3] >> 1) & 0x6) | (block[3] & 0x1)) as usize;
    let d = T_DISTANCE[d_idx] as i32;

    let c0 = [extend4to8(r0), extend4to8(g0), extend4to8(b0)];
    let c1 = [extend4to8(r1), extend4to8(g1), extend4to8(b1)];

    let pixels = u32::from_be_bytes([block[4], block[5], block[6], block[7]]);
    for tx in 0..16 {
        let sel = etc2_selector(pixels, tx);
        let rgb = match sel {
            0 => c0,
            1 => [clamp_u8(c0[0] as i32 + d), clamp_u8(c0[1] as i32 + d), clamp_u8(c0[2] as i32 + d)],
            2 => c1,
            _ => [clamp_u8(c1[0] as i32 - d), clamp_u8(c1[1] as i32 - d), clamp_u8(c1[2] as i32 - d)],
        };
        let dst = etc2_dst_offset(tx);
        out[dst]     = rgb[0];
        out[dst + 1] = rgb[1];
        out[dst + 2] = rgb[2];
        out[dst + 3] = default_alpha;
    }
}

/// ETC2 H-mode decode. Per Khronos ETC2 spec table 3.17.6: same shape as
/// T-mode but the distance is added/subtracted to both endpoints. Per
/// texel: selector 0 → c0+d, 1 → c0-d, 2 → c1+d, 3 → c1-d. The
/// distance-table index includes one extra bit derived from comparing
/// the colour pair, breaking ties consistently with the encoder.
fn decode_etc2_h_mode(block: &[u8], out: &mut [u8; 64], default_alpha: u8) {
    // Header packing (per spec):
    // R0[3..0] = b0[6..3]
    // G0[3..1] = b0[2..0], G0[0] = b1[7]
    // B0[3]    = b1[6], B0[2..0] = b1[4..2]
    // R1[3..0] = (b1[1..0] << 2) | (b2[7..6])
    // G1[3..0] = b2[5..2]
    // B1[3..0] = (b2[1..0] << 2) | (b3[7..6])
    // da[1..0] = b3[5..4]; db = b3[2]; bit-0 of distance is encoder-dependent
    let r0 = (block[0] >> 3) & 0x0F;
    let g0 = ((block[0] & 0x07) << 1) | (block[1] >> 7);
    let b0 = ((block[1] >> 6) & 0x01) << 3 | ((block[1] >> 2) & 0x07);
    let r1 = ((block[1] & 0x03) << 2) | (block[2] >> 6);
    let g1 = (block[2] >> 2) & 0x0F;
    let b1 = ((block[2] & 0x03) << 2) | (block[3] >> 6);
    let da = (block[3] >> 5) & 0x02 | (block[3] >> 4) & 0x01;
    // The low bit of the distance index is `(c0 >= c1)` per the spec's
    // tiebreak rule: pack colours into 12-bit ints and compare.
    let c0_packed = ((r0 as u32) << 8) | ((g0 as u32) << 4) | (b0 as u32);
    let c1_packed = ((r1 as u32) << 8) | ((g1 as u32) << 4) | (b1 as u32);
    let db = if c0_packed >= c1_packed { 1 } else { 0 };
    let d_idx = ((da << 1) | db) as usize & 0x7;
    let d = T_DISTANCE[d_idx] as i32;

    let c0 = [extend4to8(r0), extend4to8(g0), extend4to8(b0)];
    let c1 = [extend4to8(r1), extend4to8(g1), extend4to8(b1)];

    let pixels = u32::from_be_bytes([block[4], block[5], block[6], block[7]]);
    for tx in 0..16 {
        let sel = etc2_selector(pixels, tx);
        let (base, sign) = match sel {
            0 => (c0,  1),
            1 => (c0, -1),
            2 => (c1,  1),
            _ => (c1, -1),
        };
        let r = clamp_u8(base[0] as i32 + sign * d);
        let g = clamp_u8(base[1] as i32 + sign * d);
        let b = clamp_u8(base[2] as i32 + sign * d);
        let dst = etc2_dst_offset(tx);
        out[dst]     = r;
        out[dst + 1] = g;
        out[dst + 2] = b;
        out[dst + 3] = default_alpha;
    }
}

/// ETC2 Planar mode decode. Per Khronos ETC2 spec table 3.17.7: three
/// 6-bit colours `O`, `H`, `V` (origin, horizontal endpoint, vertical
/// endpoint), each extended via the 6→8 bit replication. Output colour
/// at texel (x, y) is `(x*(H-O) + y*(V-O))/4 + O` per channel, clamped.
fn decode_etc2_planar_mode(block: &[u8], out: &mut [u8; 64], default_alpha: u8) {
    // Header bit layout (per spec):
    // RO[5..1] = b0[6..2]; RO[0]   = b0[0]
    // GO[6..1] = (b0[1] << 6) | (b1[7..3])   ; GO[0] = b1[2]
    // BO[5..0] = (b1[1..0] << 4) | (b2[7..4])
    // (Note: GO is 7-bit; we use top 6 bits for the spec formula.)
    // RH[5..1] = b2[3..0] << 1 | b3[7]      ; RH[0]  = b3[6]
    // GH[6..1] = b3[5..0] << 0               ; GH[0] = b4[7]
    // BH[5..0] = b4[6..1]
    // RV[5..0] = (b4[0] << 5) | (b5[7..3])
    // GV[6..1] = (b5[2..0] << 4) | (b6[7..5]); GV[0] = b6[4]
    // BV[5..0] = b6[3..0] << 2 | b7[7..6]    (then bits 5..0 used)
    let r_o6 = ((block[0] >> 1) & 0x3E) | ((block[0] >> 0) & 0x01);
    let g_o6_full = (((block[0] & 0x01) as u32) << 6) | (((block[1] >> 1) & 0x7F) as u32);
    let g_o6 = (g_o6_full >> 1) as u8 & 0x3F;
    let b_o6 = (((block[1] & 0x03) as u32) << 4) | (((block[2] >> 4) & 0x0F) as u32);
    let b_o6 = b_o6 as u8 & 0x3F;

    let r_h6 = (((block[2] & 0x0F) as u32) << 2) | (((block[3] >> 6) & 0x03) as u32);
    let r_h6 = r_h6 as u8 & 0x3F;
    let g_h6 = (((block[3] & 0x3F) as u32) << 1) | (((block[4] >> 7) & 0x01) as u32);
    let g_h6 = g_h6 as u8 & 0x3F;
    let b_h6 = ((block[4] >> 1) & 0x3F) as u8;

    let r_v6 = (((block[4] & 0x01) as u32) << 5) | (((block[5] >> 3) & 0x1F) as u32);
    let r_v6 = r_v6 as u8 & 0x3F;
    let g_v6_full = (((block[5] & 0x07) as u32) << 4) | (((block[6] >> 4) & 0x0F) as u32);
    let g_v6 = g_v6_full as u8 & 0x3F;
    let b_v6 = (((block[6] & 0x0F) as u32) << 2) | (((block[7] >> 6) & 0x03) as u32);
    let b_v6 = b_v6 as u8 & 0x3F;

    let ro = ((r_o6 as i32) << 2) | ((r_o6 as i32) >> 4);
    let go = ((g_o6 as i32) << 2) | ((g_o6 as i32) >> 4);
    let bo = ((b_o6 as i32) << 2) | ((b_o6 as i32) >> 4);
    let rh = ((r_h6 as i32) << 2) | ((r_h6 as i32) >> 4);
    let gh = ((g_h6 as i32) << 2) | ((g_h6 as i32) >> 4);
    let bh = ((b_h6 as i32) << 2) | ((b_h6 as i32) >> 4);
    let rv = ((r_v6 as i32) << 2) | ((r_v6 as i32) >> 4);
    let gv = ((g_v6 as i32) << 2) | ((g_v6 as i32) >> 4);
    let bv = ((b_v6 as i32) << 2) | ((b_v6 as i32) >> 4);

    for y in 0..4_i32 {
        for x in 0..4_i32 {
            let r = (x * (rh - ro) + y * (rv - ro) + 4 * ro + 2) >> 2;
            let g = (x * (gh - go) + y * (gv - go) + 4 * go + 2) >> 2;
            let b = (x * (bh - bo) + y * (bv - bo) + 4 * bo + 2) >> 2;
            let dst = ((y as usize) * 4 + (x as usize)) * 4;
            out[dst]     = r.clamp(0, 255) as u8;
            out[dst + 1] = g.clamp(0, 255) as u8;
            out[dst + 2] = b.clamp(0, 255) as u8;
            out[dst + 3] = default_alpha;
        }
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
