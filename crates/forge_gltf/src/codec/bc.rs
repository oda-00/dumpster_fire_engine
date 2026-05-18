//! Hand-rolled decoders for the Block Compression (BC1-BC7) family that
//! KTX2 surfaces under `VK_FORMAT_BC*_UNORM_BLOCK` / `_SRGB_BLOCK`.
//!
//! Every BC format operates on 4×4 pixel blocks. The decoder iterates
//! the block grid, decodes each block to a 16-pixel RGBA tile in place,
//! and scatters it into the output image with the usual clip-to-edge for
//! non-multiple-of-4 dimensions.
//!
//! For each format the block size and the per-block algorithm differs
//! but the outer scatter loop is identical, so we factor it via the
//! `transcode_blocks` driver.

// (BC decoders are infallible — they never produce error values; only
// the dispatcher in asset.rs needs the GltfError types.)
use thin_vec::ThinVec;

/// Decode a buffer of BC1-format blocks (`VK_FORMAT_BC1_RGB_UNORM_BLOCK`
/// = 131, with alpha-on-black for `_RGBA` variants) into a tightly-packed
/// RGBA8 buffer at `width × height` resolution.
///
/// BC1 layout per 8-byte block:
///   bytes 0..2 = c0 (RGB565, little-endian u16)
///   bytes 2..4 = c1 (RGB565, little-endian u16)
///   bytes 4..8 = 16 × 2-bit selectors (LSB-first; texel order is
///                 row-major within the 4×4 block)
/// The selector picks one of four colours:
///   00 → c0  01 → c1  10 → 2/3·c0 + 1/3·c1   11 → 1/3·c0 + 2/3·c1
/// When c0 ≤ c1 (treated as unsigned u16), selector 11 is "transparent
/// black" and selector 10 is the 1/2 mid-point — the 1-bit-alpha case.
pub fn decode_bc1(blocks: &[u8], width: u32, height: u32) -> ThinVec<u8> {
    transcode_blocks(blocks, width, height, 8, decode_bc1_block)
}

/// BC2 = BC1 colour block + 4-bit alpha per texel in the preceding 8 bytes.
/// Block size: 16 bytes (8 alpha + 8 colour, in that order).
pub fn decode_bc2(blocks: &[u8], width: u32, height: u32) -> ThinVec<u8> {
    transcode_blocks(blocks, width, height, 16, decode_bc2_block)
}

/// BC3 = BC1 colour block + BC4-style 8-bit alpha (interpolated alpha
/// endpoints). Block size: 16 bytes (8 alpha + 8 colour).
pub fn decode_bc3(blocks: &[u8], width: u32, height: u32) -> ThinVec<u8> {
    transcode_blocks(blocks, width, height, 16, decode_bc3_block)
}

/// BC4 = single-channel 8-bit (red). Block size: 8 bytes. Output is
/// expanded into RGBA8 with the BC4 channel in R, zero in GB, 255 in A.
pub fn decode_bc4(blocks: &[u8], width: u32, height: u32) -> ThinVec<u8> {
    transcode_blocks(blocks, width, height, 8, decode_bc4_block)
}

/// BC5 = two BC4 streams (typically used for normal-map XY pairs). Block
/// size: 16 bytes (8 R + 8 G). Output expands as RGBA8 with R/G filled,
/// B = 0, A = 255.
pub fn decode_bc5(blocks: &[u8], width: u32, height: u32) -> ThinVec<u8> {
    transcode_blocks(blocks, width, height, 16, decode_bc5_block)
}

/// BC7 = the high-quality LDR block format. Block size: 16 bytes; eight
/// modes with per-mode endpoint encoding, partition selection, and
/// weight bit-widths. We implement the modes used by glTF assets and
/// fall back to opaque magenta on unsupported modes so the image at
/// least surfaces visually rather than silently giving black.
pub fn decode_bc7(blocks: &[u8], width: u32, height: u32) -> ThinVec<u8> {
    transcode_blocks(blocks, width, height, 16, decode_bc7_block)
}

// ─── Scatter driver ─────────────────────────────────────────────────────────

/// Walk the block grid, decode each block to a 16-texel RGBA tile, scatter
/// it into the output. Border tiles that overhang the image dimensions get
/// per-texel clipping; aligned tiles use a 16-byte per-row memcpy.
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
    // Border tiles: pre-fill with zero so partial-coverage tiles leave
    // the unwritten texels at black. Aligned-dimension uploads also
    // benefit from the consistent baseline (the fast path overwrites
    // everything anyway).
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
            // Aligned fast path: 4 × 16-byte rowwise memcpy.
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
            // Slow path: per-texel clip for the right/bottom border.
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

// ─── Per-format block decoders ──────────────────────────────────────────────

#[inline(always)]
fn rgb565_to_rgb8(v: u16) -> [u8; 3] {
    let r5 = ((v >> 11) & 0x1f) as u8;
    let g6 = ((v >>  5) & 0x3f) as u8;
    let b5 = ( v        & 0x1f) as u8;
    // Expand 5/6-bit channels to 8-bit by replicating the top bits into the low
    // bits (standard "bit-shift + or" expansion).
    [ (r5 << 3) | (r5 >> 2),
      (g6 << 2) | (g6 >> 4),
      (b5 << 3) | (b5 >> 2) ]
}

#[inline(always)]
fn lerp_u8(a: u8, b: u8, num: u32, den: u32) -> u8 {
    (((a as u32) * (den - num) + (b as u32) * num + den / 2) / den) as u8
}

fn decode_bc1_block(block: &[u8]) -> [u8; 64] {
    let c0_raw = u16::from_le_bytes([block[0], block[1]]);
    let c1_raw = u16::from_le_bytes([block[2], block[3]]);
    let bits   = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
    let c0 = rgb565_to_rgb8(c0_raw);
    let c1 = rgb565_to_rgb8(c1_raw);
    let (c2, c3, alpha_for_3): ([u8; 3], [u8; 3], u8) = if c0_raw > c1_raw {
        (
            [lerp_u8(c0[0], c1[0], 1, 3), lerp_u8(c0[1], c1[1], 1, 3), lerp_u8(c0[2], c1[2], 1, 3)],
            [lerp_u8(c0[0], c1[0], 2, 3), lerp_u8(c0[1], c1[1], 2, 3), lerp_u8(c0[2], c1[2], 2, 3)],
            255,
        )
    } else {
        (
            [lerp_u8(c0[0], c1[0], 1, 2), lerp_u8(c0[1], c1[1], 1, 2), lerp_u8(c0[2], c1[2], 1, 2)],
            [0, 0, 0],
            0,
        )
    };

    // Pack a 16-byte palette: 4 RGBA entries.
    let palette: [u8; 16] = [
        c0[0], c0[1], c0[2], 255,
        c1[0], c1[1], c1[2], 255,
        c2[0], c2[1], c2[2], 255,
        c3[0], c3[1], c3[2], alpha_for_3,
    ];

    #[cfg(target_arch = "x86_64")]
    unsafe { return decode_bc1_block_pshufb(&palette, bits); }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let mut out = [0u8; 64];
        for i in 0..16 {
            let sel = ((bits >> (i * 2)) & 0x3) as usize;
            let dst = i * 4;
            out[dst]     = palette[sel * 4];
            out[dst + 1] = palette[sel * 4 + 1];
            out[dst + 2] = palette[sel * 4 + 2];
            out[dst + 3] = palette[sel * 4 + 3];
        }
        out
    }
}

/// SSSE3 palette gather for any 4-entry indexed-block decoder (BC1/BC3
/// colour subblocks, ETC1S, ETC2 individual mode). `palette` is 4 RGBA
/// entries packed into 16 bytes; `bits` holds 16 × 2-bit selectors in
/// row-major LSB-first order. One `_mm_shuffle_epi8` per row produces
/// 4 texels' worth of RGBA in 16 bytes.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "ssse3")]
unsafe fn decode_bc1_block_pshufb(palette: &[u8; 16], bits: u32) -> [u8; 64] {
    use std::arch::x86_64::*;
    unsafe {
        let pal_v = _mm_loadu_si128(palette.as_ptr() as *const __m128i);
        let mut out = [0u8; 64];
        for row in 0..4 {
            let sel_byte = (bits >> (row * 8)) as u8;
            let s0 = ((sel_byte     ) & 3) as i8;
            let s1 = ((sel_byte >> 2) & 3) as i8;
            let s2 = ((sel_byte >> 4) & 3) as i8;
            let s3 = ((sel_byte >> 6) & 3) as i8;
            // Build per-byte palette offsets: 4 texels × 4 channel bytes.
            // Each selector contributes [s*4, s*4+1, s*4+2, s*4+3].
            let shuf = _mm_setr_epi8(
                s0 * 4, s0 * 4 + 1, s0 * 4 + 2, s0 * 4 + 3,
                s1 * 4, s1 * 4 + 1, s1 * 4 + 2, s1 * 4 + 3,
                s2 * 4, s2 * 4 + 1, s2 * 4 + 2, s2 * 4 + 3,
                s3 * 4, s3 * 4 + 1, s3 * 4 + 2, s3 * 4 + 3,
            );
            let row_v = _mm_shuffle_epi8(pal_v, shuf);
            _mm_storeu_si128(out.as_mut_ptr().add(row * 16) as *mut __m128i, row_v);
        }
        out
    }
}

fn decode_bc2_block(block: &[u8]) -> [u8; 64] {
    // Bytes 0..8 = 16 × 4-bit alpha (row-major). Bytes 8..16 = BC1 colour
    // block (always treated as 4-colour mode regardless of c0/c1 order).
    let alpha_bits = u64::from_le_bytes([
        block[0], block[1], block[2], block[3], block[4], block[5], block[6], block[7],
    ]);

    let c0_raw = u16::from_le_bytes([block[8],  block[9]]);
    let c1_raw = u16::from_le_bytes([block[10], block[11]]);
    let bits   = u32::from_le_bytes([block[12], block[13], block[14], block[15]]);
    let c0 = rgb565_to_rgb8(c0_raw);
    let c1 = rgb565_to_rgb8(c1_raw);
    let c2 = [lerp_u8(c0[0], c1[0], 1, 3), lerp_u8(c0[1], c1[1], 1, 3), lerp_u8(c0[2], c1[2], 1, 3)];
    let c3 = [lerp_u8(c0[0], c1[0], 2, 3), lerp_u8(c0[1], c1[1], 2, 3), lerp_u8(c0[2], c1[2], 2, 3)];

    let mut out = [0u8; 64];
    for i in 0..16 {
        let sel = ((bits >> (i * 2)) & 0x3) as u8;
        let rgb = match sel { 0 => c0, 1 => c1, 2 => c2, _ => c3 };
        let a4  = ((alpha_bits >> (i * 4)) & 0xf) as u8;
        let dst = i * 4;
        out[dst]     = rgb[0];
        out[dst + 1] = rgb[1];
        out[dst + 2] = rgb[2];
        out[dst + 3] = (a4 << 4) | a4;
    }
    out
}

fn decode_bc3_block(block: &[u8]) -> [u8; 64] {
    // Bytes 0..8 = BC4-style 8-bit alpha; bytes 8..16 = BC1 colour (4-colour mode).
    let alpha_table = decode_bc4_alpha_table(&block[0..8]);
    let alpha = decode_bc4_selectors(&block[0..8], &alpha_table);

    let c0_raw = u16::from_le_bytes([block[8],  block[9]]);
    let c1_raw = u16::from_le_bytes([block[10], block[11]]);
    let bits   = u32::from_le_bytes([block[12], block[13], block[14], block[15]]);
    let c0 = rgb565_to_rgb8(c0_raw);
    let c1 = rgb565_to_rgb8(c1_raw);
    let c2 = [lerp_u8(c0[0], c1[0], 1, 3), lerp_u8(c0[1], c1[1], 1, 3), lerp_u8(c0[2], c1[2], 1, 3)];
    let c3 = [lerp_u8(c0[0], c1[0], 2, 3), lerp_u8(c0[1], c1[1], 2, 3), lerp_u8(c0[2], c1[2], 2, 3)];

    // Decode RGB via the same SSSE3 palette gather BC1 uses, then overlay the
    // independently-decoded 8-bit alpha channel byte-by-byte. The colour pass
    // writes alpha = 255; the second pass overwrites it with the BC4-derived
    // alpha. Net cost: one SIMD palette gather + 16 byte writes vs 16 full
    // scalar texel writes.
    let palette: [u8; 16] = [
        c0[0], c0[1], c0[2], 255,
        c1[0], c1[1], c1[2], 255,
        c2[0], c2[1], c2[2], 255,
        c3[0], c3[1], c3[2], 255,
    ];
    let mut out = {
        #[cfg(target_arch = "x86_64")]
        unsafe { decode_bc1_block_pshufb(&palette, bits) }
        #[cfg(not(target_arch = "x86_64"))]
        {
            let mut tmp = [0u8; 64];
            for i in 0..16 {
                let sel = ((bits >> (i * 2)) & 0x3) as usize;
                let dst = i * 4;
                tmp[dst]     = palette[sel * 4];
                tmp[dst + 1] = palette[sel * 4 + 1];
                tmp[dst + 2] = palette[sel * 4 + 2];
                tmp[dst + 3] = 255;
            }
            tmp
        }
    };
    for i in 0..16 {
        out[i * 4 + 3] = alpha[i];
    }
    out
}

fn decode_bc4_block(block: &[u8]) -> [u8; 64] {
    let table = decode_bc4_alpha_table(block);
    let red   = decode_bc4_selectors(block, &table);

    let mut out = [0u8; 64];
    for i in 0..16 {
        let dst = i * 4;
        out[dst]     = red[i];
        out[dst + 1] = 0;
        out[dst + 2] = 0;
        out[dst + 3] = 255;
    }
    out
}

fn decode_bc5_block(block: &[u8]) -> [u8; 64] {
    let r_table = decode_bc4_alpha_table(&block[0..8]);
    let red     = decode_bc4_selectors(&block[0..8], &r_table);
    let g_table = decode_bc4_alpha_table(&block[8..16]);
    let green   = decode_bc4_selectors(&block[8..16], &g_table);

    let mut out = [0u8; 64];
    for i in 0..16 {
        let dst = i * 4;
        out[dst]     = red[i];
        out[dst + 1] = green[i];
        out[dst + 2] = 0;
        out[dst + 3] = 255;
    }
    out
}

/// Decode the 8-entry BC4 endpoint interpolation table from the first two
/// bytes (the endpoint pair `a0`, `a1`).
fn decode_bc4_alpha_table(block: &[u8]) -> [u8; 8] {
    let a0 = block[0];
    let a1 = block[1];
    let mut t = [0u8; 8];
    t[0] = a0;
    t[1] = a1;
    if a0 > a1 {
        // 8-interp mode: a0, a1, then 6 evenly-spaced lerps.
        for i in 1..7 {
            t[i + 1] = (((7 - i) as u32 * a0 as u32 + i as u32 * a1 as u32 + 3) / 7) as u8;
        }
    } else {
        // 6-interp mode: 4 lerps, then 0 and 255 (one-bit alpha cousins).
        for i in 1..5 {
            t[i + 1] = (((5 - i) as u32 * a0 as u32 + i as u32 * a1 as u32 + 2) / 5) as u8;
        }
        t[6] = 0;
        t[7] = 255;
    }
    t
}

/// Decode the 16 × 3-bit selectors from bytes 2..8 of a BC4 block.
fn decode_bc4_selectors(block: &[u8], table: &[u8; 8]) -> [u8; 16] {
    let sel_lo = u32::from_le_bytes([block[2], block[3], block[4], 0]);
    let sel_hi = u32::from_le_bytes([block[5], block[6], block[7], 0]);
    let bits   = (sel_lo as u64) | ((sel_hi as u64) << 24);
    let mut out = [0u8; 16];
    for i in 0..16 {
        let idx = ((bits >> (i * 3)) & 0x7) as usize;
        out[i] = table[idx];
    }
    out
}

// ─── BC7 ────────────────────────────────────────────────────────────────────
//
// BC7 has 8 modes; each picks an endpoint count (1/2/3 subsets), endpoint
// bit-width, weight bit-width, partition table, P-bit count, and rotation.
// The simplest mode (5: 1 subset, RGBA 7.7.7.7 endpoints, 2-bit weights)
// covers a lot of glTF assets. We implement modes 5 and 6 fully and
// fall back to an opaque-magenta block for the others — visually obvious
// without crashing.

fn decode_bc7_block(block: &[u8]) -> [u8; 64] {
    let data = u128::from_le_bytes([
        block[ 0], block[ 1], block[ 2], block[ 3],
        block[ 4], block[ 5], block[ 6], block[ 7],
        block[ 8], block[ 9], block[10], block[11],
        block[12], block[13], block[14], block[15],
    ]);
    // Mode = position of the lowest set bit; 0xff if data is zero.
    let mode = (data & 0xff) as u8;
    if mode == 0 {
        return fill_magenta();
    }
    let mut m = 0u8;
    while (mode >> m) & 1 == 0 && m < 8 { m += 1; }
    match m {
        5 => decode_bc7_mode5(data),
        6 => decode_bc7_mode6(data),
        _ => fill_magenta(),
    }
}

fn fill_magenta() -> [u8; 64] {
    let mut out = [0u8; 64];
    for i in 0..16 {
        let d = i * 4;
        out[d] = 255; out[d + 1] = 0; out[d + 2] = 255; out[d + 3] = 255;
    }
    out
}

/// BC7 mode 5: 1 subset, RGBA 7.7.7.7 endpoints, 2-bit colour weights,
/// 2-bit alpha weights, 2-bit rotation.
fn decode_bc7_mode5(data: u128) -> [u8; 64] {
    let mut bit = 6u32; // skip 6 mode bits (5 zeros + the 1 bit)
    let rot = take_bits(data, &mut bit, 2) as u8;

    // Endpoints: 4 × 7-bit (RGBA) for endpoint 0 and endpoint 1.
    let mut e0 = [0u8; 4];
    let mut e1 = [0u8; 4];
    for c in 0..4 { e0[c] = (take_bits(data, &mut bit, 7) as u8) << 1; }
    for c in 0..4 { e1[c] = (take_bits(data, &mut bit, 7) as u8) << 1; }
    // Add the lsb back as the high bit of the low byte (BC7 stores LSB
    // last, but mode-5 has no P-bits — so we keep the 7-bit value left-
    // shifted by 1 which biases the range correctly per the spec).
    for c in 0..4 {
        e0[c] = e0[c] | (e0[c] >> 7);
        e1[c] = e1[c] | (e1[c] >> 7);
    }

    // Anchor index 0 has a 1-bit weight; the rest have 2-bit weights.
    // Decode colour weights first (16 weights, 31 bits total).
    let mut color_w = [0u8; 16];
    for i in 0..16 {
        let bits = if i == 0 { 1 } else { 2 };
        color_w[i] = take_bits(data, &mut bit, bits) as u8;
    }
    let mut alpha_w = [0u8; 16];
    for i in 0..16 {
        let bits = if i == 0 { 1 } else { 2 };
        alpha_w[i] = take_bits(data, &mut bit, bits) as u8;
    }

    let mut out = [0u8; 64];
    for i in 0..16 {
        // 2-bit weights: BC7 spec weight table is {0, 21, 43, 64} for 2-bit
        // (the anchor's 1-bit form expands to {0, 32} but the same
        // interpolation formula applies via this table).
        let cw = bc7_2bit_weight(color_w[i]);
        let aw = bc7_2bit_weight(alpha_w[i]);
        let r = interpolate(e0[0], e1[0], cw);
        let g = interpolate(e0[1], e1[1], cw);
        let b = interpolate(e0[2], e1[2], cw);
        let a = interpolate(e0[3], e1[3], aw);

        // Rotation: 1 swaps alpha with red; 2 swaps with green; 3 swaps with blue.
        let (r, g, b, a) = match rot {
            1 => (a, g, b, r),
            2 => (r, a, b, g),
            3 => (r, g, a, b),
            _ => (r, g, b, a),
        };

        let d = i * 4;
        out[d]     = r;
        out[d + 1] = g;
        out[d + 2] = b;
        out[d + 3] = a;
    }
    out
}

/// BC7 mode 6: 1 subset, RGBA 7.7.7.7 endpoints with P-bits, 4-bit
/// weights, single endpoint pair.
fn decode_bc7_mode6(data: u128) -> [u8; 64] {
    let mut bit = 7u32; // skip 7 mode bits (6 zeros + the 1 bit)
    let mut e0 = [0u8; 4];
    let mut e1 = [0u8; 4];
    for c in 0..4 { e0[c] = take_bits(data, &mut bit, 7) as u8; }
    for c in 0..4 { e1[c] = take_bits(data, &mut bit, 7) as u8; }
    let p0 = take_bits(data, &mut bit, 1) as u8;
    let p1 = take_bits(data, &mut bit, 1) as u8;
    for c in 0..4 {
        e0[c] = (e0[c] << 1) | p0;
        e1[c] = (e1[c] << 1) | p1;
    }

    // 16 × 4-bit weights (anchor has 3-bit weight).
    let mut weights = [0u8; 16];
    for i in 0..16 {
        let bits = if i == 0 { 3 } else { 4 };
        weights[i] = take_bits(data, &mut bit, bits) as u8;
    }

    let mut out = [0u8; 64];
    for i in 0..16 {
        let w = bc7_4bit_weight(weights[i]);
        let d = i * 4;
        out[d]     = interpolate(e0[0], e1[0], w);
        out[d + 1] = interpolate(e0[1], e1[1], w);
        out[d + 2] = interpolate(e0[2], e1[2], w);
        out[d + 3] = interpolate(e0[3], e1[3], w);
    }
    out
}

#[inline(always)]
fn take_bits(data: u128, bit: &mut u32, n: u32) -> u32 {
    let v = ((data >> *bit) as u32) & ((1u32 << n) - 1);
    *bit += n;
    v
}

#[inline(always)]
fn bc7_2bit_weight(w: u8) -> u32 {
    // 2-bit weight table from the BC7 spec.
    match w & 0x3 { 0 => 0, 1 => 21, 2 => 43, _ => 64 }
}

#[inline(always)]
fn bc7_4bit_weight(w: u8) -> u32 {
    // 4-bit weight table from the BC7 spec.
    const T: [u32; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];
    T[(w & 0xf) as usize]
}

#[inline(always)]
fn interpolate(a: u8, b: u8, w: u32) -> u8 {
    (((64 - w) * a as u32 + w * b as u32 + 32) / 64) as u8
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bc1_solid_color_block_decodes_to_uniform_pixel() {
        // c0 = c1 = pure red (RGB565 = 0xF800). Any selector picks c0 or
        // c1 (or their lerps, which are also red). Selectors all-zero.
        let red = 0xF800u16.to_le_bytes();
        let mut block = [0u8; 8];
        block[0] = red[0]; block[1] = red[1];
        block[2] = red[0]; block[3] = red[1];
        // selectors all 00 → c0 for every texel
        let out = decode_bc1_block(&block);
        // RGB565 0xF800 → R5 = 0x1f → 0xFF (5-bit-to-8-bit replication).
        for i in 0..16 {
            assert_eq!(out[i * 4],     0xFF, "R should be red at texel {i}");
            assert_eq!(out[i * 4 + 1], 0,    "G should be 0 at texel {i}");
            assert_eq!(out[i * 4 + 2], 0,    "B should be 0 at texel {i}");
            assert_eq!(out[i * 4 + 3], 255,  "A should be 255 at texel {i}");
        }
    }

    #[test]
    fn bc4_all_zero_block_decodes_zero_red() {
        let block = [0u8; 8];
        let out = decode_bc4_block(&block);
        for i in 0..16 {
            assert_eq!(out[i * 4],     0);
            assert_eq!(out[i * 4 + 1], 0);
            assert_eq!(out[i * 4 + 2], 0);
            assert_eq!(out[i * 4 + 3], 255);
        }
    }

    #[test]
    fn bc4_endpoint_pair_lerps_correctly() {
        // a0 = 0, a1 = 255 with selectors all 1 should produce a1=255.
        let mut block = [0u8; 8];
        block[0] = 0;  block[1] = 255;
        // All selectors = 1 (3 bits each, 16 entries = 48 bits = 6 bytes).
        // Pack 1 every 3 bits: 0b001001001001001001001001001001001001001001001001 = 0x249249249249.
        let pat: u64 = 0x249249249249;
        block[2] = (pat & 0xff) as u8;
        block[3] = ((pat >> 8) & 0xff) as u8;
        block[4] = ((pat >> 16) & 0xff) as u8;
        block[5] = ((pat >> 24) & 0xff) as u8;
        block[6] = ((pat >> 32) & 0xff) as u8;
        block[7] = ((pat >> 40) & 0xff) as u8;
        let out = decode_bc4_block(&block);
        for i in 0..16 {
            assert_eq!(out[i * 4], 255, "selector 1 should land on a1 at texel {i}");
        }
    }

    #[test]
    fn decode_bc1_outputs_correct_pixel_count() {
        // 8×8 = four 4×4 blocks. With 8 bytes per block that's 32 bytes input.
        let blocks = vec![0u8; 32];
        let out = decode_bc1(&blocks, 8, 8);
        assert_eq!(out.len(), 8 * 8 * 4);
    }
}
