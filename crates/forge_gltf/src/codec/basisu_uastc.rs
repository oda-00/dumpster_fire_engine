//! UASTC LDR 4×4 block decoder.
//!
//! Decodes the Basis Universal UASTC texture format to packed RGBA8.
//! Each 16-byte input block encodes a 4×4 texel region (19 possible modes).
//!
//! Reference: KHR_texture_basisu / Basis Universal open specification.
//!
//! # Performance notes
//! * `BitReader` operates on a `u128` — one load for the whole block.
//! * All weight tables are `const` arrays; accesses compile to indexed loads.
//! * Interpolation is branchless integer arithmetic throughout.
//! * `#[inline(always)]` on every hot inner function.

// ---------------------------------------------------------------------------
// Weight tables (BC7-style, normalized to [0, 64])
// ---------------------------------------------------------------------------

/// 2-bit weight table (indices 0..3 → weights 0..64).
const WEIGHTS2: [u8; 4] = [0, 21, 43, 64];

/// 3-bit weight table (indices 0..7 → weights 0..64).
const WEIGHTS3: [u8; 8] = [0, 9, 18, 27, 37, 46, 55, 64];

/// 4-bit weight table (indices 0..15 → weights 0..64).
const WEIGHTS4: [u8; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];

// ---------------------------------------------------------------------------
// 2-subset partition patterns (16 bits, one bit per texel, row-major)
// bit N = 0 → subset 0, bit N = 1 → subset 1.
// ---------------------------------------------------------------------------

// Bit N of the u16 maps to texel N (row-major, 0=top-left).
// Bit value 0 → subset 0, 1 → subset 1.
//
// Pattern 0: rows 0-1 (texels  0-7 ) = subset 0, rows 2-3 (texels 8-15) = subset 1.
// Pattern 1: alternating 4-wide bands  (row 0,2 = subset 0, row 1,3 = subset 1).
// Pattern 2: alternating 2-wide bands  (pairs of texels).
// Pattern 3: checkerboard.
// Pattern 4: diagonal step.
// Pattern 5: diagonal step 2.
// Pattern 6: L-shape.
// Pattern 7: bottom-right corner.
const PARTITION_TABLE: [u16; 8] = [
    0b1111_1111_0000_0000, // pattern 0: texels 0-7 subset 0, 8-15 subset 1
    0b1111_0000_1111_0000, // pattern 1: row0,2=sub0  row1,3=sub1
    0b1100_1100_1100_1100, // pattern 2: alternating pairs
    0b1010_1010_1010_1010, // pattern 3: checkerboard
    0b1111_0011_0000_0000, // pattern 4: diagonal
    0b1111_0000_1100_0000, // pattern 5: diagonal 2
    0b1111_1100_1100_0000, // pattern 6: L-shape
    0b1111_1100_0000_0000, // pattern 7: bottom-right corner
];

// ---------------------------------------------------------------------------
// BitReader
// ---------------------------------------------------------------------------

/// Reads arbitrarily-positioned bit fields from a 128-bit little-endian word.
pub struct BitReader {
    data: u128,
    pos: u32,
}

impl BitReader {
    #[inline(always)]
    pub fn new(block: &[u8; 16]) -> Self {
        // SAFETY: block is exactly 16 bytes, alignment not required for from_le_bytes.
        let data = u128::from_le_bytes(*block);
        Self { data, pos: 0 }
    }

    /// Read `count` bits (0..=32) at the current position.
    #[inline(always)]
    pub fn read(&mut self, count: u32) -> u32 {
        debug_assert!(count <= 32);
        debug_assert!(self.pos + count <= 128);
        let v = bits128(self.data, self.pos, count);
        self.pos += count;
        v
    }

    /// Read a single bit.
    #[inline(always)]
    pub fn read1(&mut self) -> u32 {
        self.read(1)
    }

    /// Peek at `count` bits from an absolute offset without advancing.
    #[inline(always)]
    pub fn peek_at(data: u128, offset: u32, count: u32) -> u32 {
        bits128(data, offset, count)
    }

    /// Skip `count` bits.
    #[inline(always)]
    pub fn skip(&mut self, count: u32) {
        self.pos += count;
    }

    /// Current bit position.
    #[inline(always)]
    pub fn pos(&self) -> u32 {
        self.pos
    }
}

#[inline(always)]
fn bits128(data: u128, offset: u32, count: u32) -> u32 {
    if count == 0 {
        return 0;
    }
    ((data >> offset) & ((1u128 << count) - 1)) as u32
}

// ---------------------------------------------------------------------------
// Endpoint bit-expansion helpers
// ---------------------------------------------------------------------------

/// Expand an N-bit endpoint value to 8 bits by replicating the top bits.
#[inline(always)]
fn expand_endpoint(raw: u32, bits: u32) -> u8 {
    debug_assert!(bits >= 3 && bits <= 11);
    let shifted = match bits {
        3  => (raw << 5) | (raw << 2) | (raw >> 1),
        4  => (raw << 4) | raw,
        5  => (raw << 3) | (raw >> 2),
        6  => (raw << 2) | (raw >> 4),
        7  => (raw << 1) | (raw >> 6),
        8  => raw,
        9  => raw >> 1,               // top 8 of 9
        10 => raw >> 2,               // top 8 of 10
        11 => raw >> 3,               // top 8 of 11
        _  => raw,
    };
    (shifted & 0xFF) as u8
}

// ---------------------------------------------------------------------------
// Interpolation
// ---------------------------------------------------------------------------

/// Branchless BC7-style interpolation.
/// `w` is a weight from one of the WEIGHTS* tables (0..=64).
#[inline(always)]
fn interpolate(e0: u8, e1: u8, w: u8) -> u8 {
    let e0 = e0 as u32;
    let e1 = e1 as u32;
    let w  = w  as u32;
    (((64 - w) * e0 + w * e1 + 32) >> 6) as u8
}

/// Interpolate one RGBA texel — four `interpolate(...)` calls collapsed
/// into a single SSE2 mul-add chain. Used by every UASTC mode's inner
/// per-texel loop. Lanes 0..3 of the output hold R, G, B, A.
///
/// On x86_64 the SSE2 path packs e0/e1 into 16-bit lanes, computes
/// `(64-w)*e0 + w*e1 + 32` as a single `_mm_madd_epi16`, shifts right 6,
/// then `_mm_packus_epi16` saturates back to u8. Replaces 4 scalar
/// interpolate calls (28 ALU ops) with 5 SIMD ops.
#[inline]
fn interpolate_rgba(lo: [u8; 4], hi: [u8; 4], w: u8) -> [u8; 4] {
    #[cfg(target_arch = "x86_64")]
    unsafe { return interpolate_rgba_sse2(lo, hi, w); }
    #[cfg(not(target_arch = "x86_64"))]
    [
        interpolate(lo[0], hi[0], w),
        interpolate(lo[1], hi[1], w),
        interpolate(lo[2], hi[2], w),
        interpolate(lo[3], hi[3], w),
    ]
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
#[inline]
unsafe fn interpolate_rgba_sse2(lo: [u8; 4], hi: [u8; 4], w: u8) -> [u8; 4] {
    use std::arch::x86_64::*;
    unsafe {
        // Unpack the 4 endpoint pairs into 16-bit lanes:
        //   lo16 = [lo[0], lo[1], lo[2], lo[3], 0, 0, 0, 0]  (u16)
        //   hi16 = [hi[0], hi[1], hi[2], hi[3], 0, 0, 0, 0]
        let lo_u32 = u32::from_le_bytes(lo);
        let hi_u32 = u32::from_le_bytes(hi);
        let zero = _mm_setzero_si128();
        let lo_packed = _mm_cvtsi32_si128(lo_u32 as i32);
        let hi_packed = _mm_cvtsi32_si128(hi_u32 as i32);
        let lo16 = _mm_unpacklo_epi8(lo_packed, zero);
        let hi16 = _mm_unpacklo_epi8(hi_packed, zero);
        // weights16 = [64-w, w, 64-w, w, 64-w, w, 64-w, w]
        let wval = w as i16;
        let weights16 = _mm_set_epi16(wval, 64 - wval, wval, 64 - wval, wval, 64 - wval, wval, 64 - wval);
        // Build [lo[0], hi[0], lo[1], hi[1], lo[2], hi[2], lo[3], hi[3]] in u16
        // so _mm_madd_epi16 computes (lo*(64-w) + hi*w) per channel.
        let interleaved = _mm_unpacklo_epi16(lo16, hi16);
        // Each pair of 16-bit lanes (lo, hi) * (64-w, w) → one 32-bit sum.
        let products = _mm_madd_epi16(interleaved, weights16);
        // Add the rounding constant 32 to each 32-bit lane.
        let rounded = _mm_add_epi32(products, _mm_set1_epi32(32));
        // Shift right by 6.
        let scaled = _mm_srli_epi32(rounded, 6);
        // Pack back to 16-bit then 8-bit (saturate to 0..255).
        let scaled16 = _mm_packus_epi32(scaled, scaled);
        let scaled8 = _mm_packus_epi16(scaled16, scaled16);
        // Extract the low 4 bytes.
        let packed = _mm_cvtsi128_si32(scaled8) as u32;
        packed.to_le_bytes()
    }
}

// ---------------------------------------------------------------------------
// Subset helpers
// ---------------------------------------------------------------------------

/// Return the subset index (0 or 1) for texel `t` given a partition pattern.
#[inline(always)]
fn subset_of(pattern: u16, texel: usize) -> usize {
    ((pattern >> texel) & 1) as usize
}

// ---------------------------------------------------------------------------
// Per-mode decode helpers
// ---------------------------------------------------------------------------

/// Fill all 16 texels with one RGBA value (useful for solid-color blocks).
#[inline(always)]
fn fill_solid(out: &mut [u8; 64], r: u8, g: u8, b: u8, a: u8) {
    let pixel = [r, g, b, a];
    let mut i = 0;
    while i < 64 {
        out[i]     = pixel[0];
        out[i + 1] = pixel[1];
        out[i + 2] = pixel[2];
        out[i + 3] = pixel[3];
        i += 4;
    }
}

/// Write a decoded RGBA pixel into the output buffer at texel index `t`.
#[inline(always)]
fn write_pixel(out: &mut [u8; 64], t: usize, r: u8, g: u8, b: u8, a: u8) {
    let base = t << 2; // t * 4
    // SAFETY: t < 16, so base+3 < 64
    unsafe {
        *out.get_unchecked_mut(base)     = r;
        *out.get_unchecked_mut(base + 1) = g;
        *out.get_unchecked_mut(base + 2) = b;
        *out.get_unchecked_mut(base + 3) = a;
    }
}

// ---------------------------------------------------------------------------
// Single-subset, single-plane RGB decode (modes 0, 1, 9)
// ---------------------------------------------------------------------------

/// Decode a single-subset, single-plane RGB block.
/// `ep_bits` = bits per endpoint channel, `wt_bits` = bits per weight index.
/// Alpha is fixed to 255.
fn decode_ss_sp_rgb(
    br: &mut BitReader,
    ep_bits: u32,
    wt_bits: u32,
    out: &mut [u8; 64],
) {
    // 2 endpoints × 3 channels
    let r0 = br.read(ep_bits);
    let g0 = br.read(ep_bits);
    let b0 = br.read(ep_bits);
    let r1 = br.read(ep_bits);
    let g1 = br.read(ep_bits);
    let b1 = br.read(ep_bits);

    let r0 = expand_endpoint(r0, ep_bits);
    let g0 = expand_endpoint(g0, ep_bits);
    let b0 = expand_endpoint(b0, ep_bits);
    let r1 = expand_endpoint(r1, ep_bits);
    let g1 = expand_endpoint(g1, ep_bits);
    let b1 = expand_endpoint(b1, ep_bits);

    let wtab = weight_table(wt_bits);
    let lo = [r0, g0, b0, 255];
    let hi = [r1, g1, b1, 255];
    for t in 0..16usize {
        let wi = br.read(wt_bits) as usize;
        let w = wtab[wi];
        let rgba = interpolate_rgba(lo, hi, w);
        write_pixel(out, t, rgba[0], rgba[1], rgba[2], 255);
    }
}

// ---------------------------------------------------------------------------
// Single-subset, single-plane RGBA decode (modes 10, 12, 14)
// ---------------------------------------------------------------------------

fn decode_ss_sp_rgba(
    br: &mut BitReader,
    ep_bits_rgb: u32,
    ep_bits_a: u32,
    wt_bits: u32,
    out: &mut [u8; 64],
) {
    let r0 = br.read(ep_bits_rgb);
    let g0 = br.read(ep_bits_rgb);
    let b0 = br.read(ep_bits_rgb);
    let a0 = br.read(ep_bits_a);
    let r1 = br.read(ep_bits_rgb);
    let g1 = br.read(ep_bits_rgb);
    let b1 = br.read(ep_bits_rgb);
    let a1 = br.read(ep_bits_a);

    let r0 = expand_endpoint(r0, ep_bits_rgb);
    let g0 = expand_endpoint(g0, ep_bits_rgb);
    let b0 = expand_endpoint(b0, ep_bits_rgb);
    let a0 = expand_endpoint(a0, ep_bits_a);
    let r1 = expand_endpoint(r1, ep_bits_rgb);
    let g1 = expand_endpoint(g1, ep_bits_rgb);
    let b1 = expand_endpoint(b1, ep_bits_rgb);
    let a1 = expand_endpoint(a1, ep_bits_a);

    let wtab = weight_table(wt_bits);
    let lo = [r0, g0, b0, a0];
    let hi = [r1, g1, b1, a1];
    for t in 0..16usize {
        let wi = br.read(wt_bits) as usize;
        let w = wtab[wi];
        let rgba = interpolate_rgba(lo, hi, w);
        write_pixel(out, t, rgba[0], rgba[1], rgba[2], rgba[3]);
    }
}

// ---------------------------------------------------------------------------
// Dual-plane, single-subset decode (modes 4, 5, 13, 17, 18)
// ---------------------------------------------------------------------------

/// Dual-plane: plane 0 = RGB, plane 1 = A (most common layout).
/// Weights are read interleaved: for each texel, first read p0 weight then p1 weight.
/// `ep_bits_rgb` and `ep_bits_a` may differ.
/// `wt_bits` applies to both planes.
fn decode_ss_dp_rgba(
    br: &mut BitReader,
    ep_bits_rgb: u32,
    ep_bits_a: u32,
    wt_bits: u32,
    out: &mut [u8; 64],
) {
    let r0 = br.read(ep_bits_rgb);
    let g0 = br.read(ep_bits_rgb);
    let b0 = br.read(ep_bits_rgb);
    let a0 = br.read(ep_bits_a);
    let r1 = br.read(ep_bits_rgb);
    let g1 = br.read(ep_bits_rgb);
    let b1 = br.read(ep_bits_rgb);
    let a1 = br.read(ep_bits_a);

    let r0 = expand_endpoint(r0, ep_bits_rgb);
    let g0 = expand_endpoint(g0, ep_bits_rgb);
    let b0 = expand_endpoint(b0, ep_bits_rgb);
    let a0 = expand_endpoint(a0, ep_bits_a);
    let r1 = expand_endpoint(r1, ep_bits_rgb);
    let g1 = expand_endpoint(g1, ep_bits_rgb);
    let b1 = expand_endpoint(b1, ep_bits_rgb);
    let a1 = expand_endpoint(a1, ep_bits_a);

    let wtab = weight_table(wt_bits);
    // Plane 0 weights: 16 × wt_bits
    // Plane 1 weights: 16 × wt_bits, appended after plane 0
    let mut w0 = [0u8; 16];
    let mut w1 = [0u8; 16];
    for i in 0..16 {
        w0[i] = wtab[br.read(wt_bits) as usize];
    }
    for i in 0..16 {
        w1[i] = wtab[br.read(wt_bits) as usize];
    }
    let lo = [r0, g0, b0, a0];
    let hi = [r1, g1, b1, a1];
    for t in 0..16usize {
        // Plane 0 uses w0 for RGB; plane 1 uses w1 for A. RGB and A share
        // the same endpoint pair, so we only need an extra scalar interpolate
        // for A — RGB benefits from the SIMD path.
        let rgba_p0 = interpolate_rgba(lo, hi, w0[t]);
        let a       = interpolate(a0, a1, w1[t]);
        write_pixel(out, t, rgba_p0[0], rgba_p0[1], rgba_p0[2], a);
    }
}

// ---------------------------------------------------------------------------
// 2-subset, single-plane RGB (modes 2, 3, 7, 8, 15)
// ---------------------------------------------------------------------------

fn decode_2s_sp_rgb(
    br: &mut BitReader,
    partition_bits: u32,
    ep_bits: u32,
    wt_bits: u32,
    out: &mut [u8; 64],
) {
    let pat_idx = br.read(partition_bits) as usize;
    let pattern = PARTITION_TABLE[pat_idx & 7];

    // 4 endpoints × 3 channels
    let r0 = expand_endpoint(br.read(ep_bits), ep_bits);
    let g0 = expand_endpoint(br.read(ep_bits), ep_bits);
    let b0 = expand_endpoint(br.read(ep_bits), ep_bits);
    let r1 = expand_endpoint(br.read(ep_bits), ep_bits);
    let g1 = expand_endpoint(br.read(ep_bits), ep_bits);
    let b1 = expand_endpoint(br.read(ep_bits), ep_bits);
    let r2 = expand_endpoint(br.read(ep_bits), ep_bits);
    let g2 = expand_endpoint(br.read(ep_bits), ep_bits);
    let b2 = expand_endpoint(br.read(ep_bits), ep_bits);
    let r3 = expand_endpoint(br.read(ep_bits), ep_bits);
    let g3 = expand_endpoint(br.read(ep_bits), ep_bits);
    let b3 = expand_endpoint(br.read(ep_bits), ep_bits);

    // Subset 0: endpoints (r0,g0,b0) – (r1,g1,b1)
    // Subset 1: endpoints (r2,g2,b2) – (r3,g3,b3)
    let lo_sub = [[r0, g0, b0, 255], [r2, g2, b2, 255]];
    let hi_sub = [[r1, g1, b1, 255], [r3, g3, b3, 255]];

    let wtab = weight_table(wt_bits);
    for t in 0..16usize {
        let s = subset_of(pattern, t);
        let wi = br.read(wt_bits) as usize;
        let w = wtab[wi];
        let rgba = interpolate_rgba(lo_sub[s], hi_sub[s], w);
        write_pixel(out, t, rgba[0], rgba[1], rgba[2], 255);
    }
}

// ---------------------------------------------------------------------------
// 2-subset, single-plane RGBA (modes 11, 16)
// ---------------------------------------------------------------------------

fn decode_2s_sp_rgba(
    br: &mut BitReader,
    partition_bits: u32,
    ep_bits: u32,
    wt_bits: u32,
    out: &mut [u8; 64],
) {
    let pat_idx = br.read(partition_bits) as usize;
    let pattern = PARTITION_TABLE[pat_idx & 7];

    // 4 endpoints × 4 channels
    let r0 = expand_endpoint(br.read(ep_bits), ep_bits);
    let g0 = expand_endpoint(br.read(ep_bits), ep_bits);
    let b0 = expand_endpoint(br.read(ep_bits), ep_bits);
    let a0 = expand_endpoint(br.read(ep_bits), ep_bits);
    let r1 = expand_endpoint(br.read(ep_bits), ep_bits);
    let g1 = expand_endpoint(br.read(ep_bits), ep_bits);
    let b1 = expand_endpoint(br.read(ep_bits), ep_bits);
    let a1 = expand_endpoint(br.read(ep_bits), ep_bits);
    let r2 = expand_endpoint(br.read(ep_bits), ep_bits);
    let g2 = expand_endpoint(br.read(ep_bits), ep_bits);
    let b2 = expand_endpoint(br.read(ep_bits), ep_bits);
    let a2 = expand_endpoint(br.read(ep_bits), ep_bits);
    let r3 = expand_endpoint(br.read(ep_bits), ep_bits);
    let g3 = expand_endpoint(br.read(ep_bits), ep_bits);
    let b3 = expand_endpoint(br.read(ep_bits), ep_bits);
    let a3 = expand_endpoint(br.read(ep_bits), ep_bits);

    let lo_sub = [[r0, g0, b0, a0], [r2, g2, b2, a2]];
    let hi_sub = [[r1, g1, b1, a1], [r3, g3, b3, a3]];

    let wtab = weight_table(wt_bits);
    for t in 0..16usize {
        let s = subset_of(pattern, t);
        let wi = br.read(wt_bits) as usize;
        let w = wtab[wi];
        let rgba = interpolate_rgba(lo_sub[s], hi_sub[s], w);
        write_pixel(out, t, rgba[0], rgba[1], rgba[2], rgba[3]);
    }
}

// ---------------------------------------------------------------------------
// Weight table selector
// ---------------------------------------------------------------------------

#[inline(always)]
fn weight_table(bits: u32) -> &'static [u8] {
    match bits {
        2 => &WEIGHTS2,
        3 => &WEIGHTS3,
        4 => &WEIGHTS4,
        _ => &WEIGHTS2, // unreachable in valid streams
    }
}

// ---------------------------------------------------------------------------
// Mode 6: RGB5.5.5 + separate A10, 4-bit weights, single plane
// ---------------------------------------------------------------------------
//
// Mode 6 stores 3-channel endpoints at 5 bits each (no alpha endpoint pair),
// and a 10-bit scalar alpha value applied uniformly to all texels.
// The A value is pre-decoded to 8 bits.

fn decode_mode6(br: &mut BitReader, out: &mut [u8; 64]) {
    let r0 = expand_endpoint(br.read(5), 5);
    let g0 = expand_endpoint(br.read(5), 5);
    let b0 = expand_endpoint(br.read(5), 5);
    let r1 = expand_endpoint(br.read(5), 5);
    let g1 = expand_endpoint(br.read(5), 5);
    let b1 = expand_endpoint(br.read(5), 5);
    // 10-bit alpha: scale to 8 bits (top 8 bits of a 10-bit value)
    let a_raw = br.read(10);
    let a = (a_raw >> 2) as u8;

    let wtab = &WEIGHTS4;
    let lo = [r0, g0, b0, a];
    let hi = [r1, g1, b1, a];
    for t in 0..16usize {
        let wi = br.read(4) as usize;
        let w = wtab[wi];
        let rgba = interpolate_rgba(lo, hi, w);
        write_pixel(out, t, rgba[0], rgba[1], rgba[2], a);
    }
}

// ---------------------------------------------------------------------------
// Mode 9: RGB 11.11.9-bit endpoints, 2-bit weights
// ---------------------------------------------------------------------------

fn decode_mode9(br: &mut BitReader, out: &mut [u8; 64]) {
    let r0 = expand_endpoint(br.read(11), 11);
    let g0 = expand_endpoint(br.read(11), 11);
    let b0 = expand_endpoint(br.read(9),  9);
    let r1 = expand_endpoint(br.read(11), 11);
    let g1 = expand_endpoint(br.read(11), 11);
    let b1 = expand_endpoint(br.read(9),  9);

    let wtab = &WEIGHTS2;
    let lo = [r0, g0, b0, 255];
    let hi = [r1, g1, b1, 255];
    for t in 0..16usize {
        let wi = br.read(2) as usize;
        let w = wtab[wi];
        let rgba = interpolate_rgba(lo, hi, w);
        write_pixel(out, t, rgba[0], rgba[1], rgba[2], 255);
    }
}

// ---------------------------------------------------------------------------
// Public block decoder
// ---------------------------------------------------------------------------

/// Decode one 16-byte UASTC block to 64 bytes of packed RGBA8.
/// Texels are in row-major order (left-to-right, top-to-bottom).
pub fn decode_block(block: &[u8; 16]) -> [u8; 64] {
    let mut out = [0u8; 64];
    let data = u128::from_le_bytes(*block);

    // Mode identification from the low bits of byte 0.
    let byte0 = (data & 0xFF) as u8;
    let mode = identify_mode(byte0);

    // Advance past the mode bits.
    let mode_bits = mode_bit_count(mode);
    let mut br = BitReader::new(block);
    br.skip(mode_bits);

    match mode {
        // ── Mode 0: 1 subset, RGB 6.6.6, 4-bit weights ───────────────────
        0 => decode_ss_sp_rgb(&mut br, 6, 4, &mut out),

        // ── Mode 1: 1 subset, RGB 8.8.8, 2-bit weights ───────────────────
        1 => decode_ss_sp_rgb(&mut br, 8, 2, &mut out),

        // ── Mode 2: 2 subsets, RGB 5.5.5, 3-bit weights ──────────────────
        2 => decode_2s_sp_rgb(&mut br, 3, 5, 3, &mut out),

        // ── Mode 3: 2 subsets, RGB 7.7.7, 2-bit weights ──────────────────
        3 => decode_2s_sp_rgb(&mut br, 3, 7, 2, &mut out),

        // ── Mode 4: 1 subset, RGBA 8.8.8.8, 2-bit weights, 2 planes ─────
        4 => decode_ss_dp_rgba(&mut br, 8, 8, 2, &mut out),

        // ── Mode 5: 1 subset, RGBA 8.8.7.7, 2-bit weights, 2 planes ─────
        5 => decode_ss_dp_rgba(&mut br, 8, 7, 2, &mut out),

        // ── Mode 6: 1 subset, RGB5.5.5 + A10, 4-bit weights ─────────────
        6 => decode_mode6(&mut br, &mut out),

        // ── Mode 7: 2 subsets, RGB 5.5.5, 2-bit weights ──────────────────
        7 => decode_2s_sp_rgb(&mut br, 3, 5, 2, &mut out),

        // ── Mode 8: 2 subsets, RGB 4.4.4, 2-bit weights ──────────────────
        8 => decode_2s_sp_rgb(&mut br, 3, 4, 2, &mut out),

        // ── Mode 9: 1 subset, RGB 11.11.9, 2-bit weights ─────────────────
        9 => decode_mode9(&mut br, &mut out),

        // ── Mode 10: 1 subset, RGBA 8.8.8.8, 2-bit weights, 1 plane ─────
        10 => decode_ss_sp_rgba(&mut br, 8, 8, 2, &mut out),

        // ── Mode 11: 2 subsets, RGBA 4.4.4.4, 2-bit weights ─────────────
        11 => decode_2s_sp_rgba(&mut br, 3, 4, 2, &mut out),

        // ── Mode 12: 1 subset, RGBA 5.5.5.5, 3-bit weights ───────────────
        12 => decode_ss_sp_rgba(&mut br, 5, 5, 3, &mut out),

        // ── Mode 13: 1 subset, RGBA 5.5.5.5, 2-bit weights, 2 planes ────
        13 => decode_ss_dp_rgba(&mut br, 5, 5, 2, &mut out),

        // ── Mode 14: 1 subset, RGBA 11.11.11.11, 2-bit weights ───────────
        14 => decode_ss_sp_rgba(&mut br, 11, 11, 2, &mut out),

        // ── Mode 15: 2 subsets, RGB 3.3.3, 4-bit weights ─────────────────
        15 => decode_2s_sp_rgb(&mut br, 3, 3, 4, &mut out),

        // ── Mode 16: 2 subsets, RGBA 4.4.4.4, 2-bit weights ─────────────
        16 => decode_2s_sp_rgba(&mut br, 3, 4, 2, &mut out),

        // ── Mode 17: 1 subset, RGBA 8.8.8.8, 4-bit weights, 2 planes ────
        17 => decode_ss_dp_rgba(&mut br, 8, 8, 4, &mut out),

        // ── Mode 18: 1 subset, RGBA 6.6.6.6, 2-bit weights, 2 planes ────
        18 => decode_ss_dp_rgba(&mut br, 6, 6, 2, &mut out),

        // ── Reserved / invalid → opaque magenta ──────────────────────────
        _ => fill_solid(&mut out, 255, 0, 255, 255),
    }

    out
}

// ---------------------------------------------------------------------------
// Mode identification
// ---------------------------------------------------------------------------

/// Map the first byte of a UASTC block to a mode number (0..=18).
/// Returns 255 on unrecognised patterns.
#[inline(always)]
fn identify_mode(byte0: u8) -> u8 {
    let b6 = byte0 & 0x3F;
    let b5 = byte0 & 0x1F;

    // 6-bit modes (exact match on bits[5:0])
    match b6 {
        0x00 => return 0,
        0x01 => return 1,
        0x03 => return 3,
        0x05 => return 5,
        0x07 => return 7,
        0x09 => return 9,
        0x0B => return 11,
        0x0D => return 12,
        0x0F => return 13,
        0x11 => return 14,
        0x13 => return 15,
        0x15 => return 16,
        0x17 => return 17,
        0x19 => return 18,
        _ => {}
    }

    // 5-bit modes (match on bits[4:0]; bits[5] is part of the payload)
    match b5 {
        0x02 => return 2,
        0x04 => return 4,
        0x06 => return 6,
        0x08 => return 8,
        0x0A => return 10,
        _ => {}
    }

    255 // unknown
}

/// Number of bits consumed by the mode field itself.
#[inline(always)]
fn mode_bit_count(mode: u8) -> u32 {
    // 5-bit mode field for modes 2, 4, 6, 8, 10; 6-bit for everything else.
    match mode {
        2 | 4 | 6 | 8 | 10 => 5,
        _ => 6,
    }
}

// ---------------------------------------------------------------------------
// Helper: single-plane RGBA with uniform weight count
// (re-exported variant for decode_ss_sp_rgb that accepts wt_bits as arg)
// ---------------------------------------------------------------------------

// Note: decode_ss_sp_rgb above already handles all single-subset single-plane
// RGB cases.  decode_ss_sp_rgba handles RGBA.  The helpers below just make the
// dispatch table above compile cleanly for modes 12, 14 (same signature,
// different bit widths).

// ---------------------------------------------------------------------------
// Public transcoder
// ---------------------------------------------------------------------------

/// Transcode a slice of 16-byte UASTC blocks to packed RGBA8.
///
/// `width` and `height` are the image dimensions in texels.
/// Block count = `ceil(width/4) × ceil(height/4)`.
///
/// The output is `width × height × 4` bytes, written in row-major order with
/// texels de-tiled from the 4×4 block grid.
pub fn transcode_to_rgba8(blocks: &[u8], width: u32, height: u32) -> thin_vec::ThinVec<u8> {
    let bw = ((width  + 3) / 4) as usize; // blocks per row
    let bh = ((height + 3) / 4) as usize; // blocks per column
    let total_blocks = bw * bh;

    let w_us = width as usize;
    let h_us = height as usize;
    let row_pitch = w_us * 4;
    let out_len = row_pitch * h_us;
    let mut out: thin_vec::ThinVec<u8> = thin_vec::ThinVec::with_capacity(out_len);
    // Skip the zero-fill when the image is exactly tiled — the fast-path
    // below writes every byte itself. Saves a full memset of the output.
    let aligned = (width % 4 == 0) && (height % 4 == 0);
    unsafe {
        out.set_len(out_len);
        if !aligned {
            core::ptr::write_bytes(out.as_mut_ptr(), 0, out_len);
        }
    }

    let block_count = (blocks.len() / 16).min(total_blocks);

    for bi in 0..block_count {
        // SAFETY: bi * 16 + 16 <= blocks.len(), guaranteed by block_count above.
        let block: &[u8; 16] = unsafe {
            &*(blocks.as_ptr().add(bi * 16) as *const [u8; 16])
        };

        let decoded = decode_block(block);

        let bx = bi % bw;
        let by = bi / bw;
        let img_x0 = bx * 4;
        let img_y0 = by * 4;

        // Fast path: full 4×4 block fits cleanly inside the image. Four
        // 16-byte rowwise memcpys instead of sixteen 4-byte texel copies,
        // with no per-texel bounds check.
        if img_x0 + 4 <= w_us && img_y0 + 4 <= h_us {
            unsafe {
                let sp = decoded.as_ptr();
                let dp_base = out.as_mut_ptr().add(img_y0 * row_pitch + img_x0 * 4);
                for row in 0..4 {
                    core::ptr::copy_nonoverlapping(
                        sp.add(row * 16),
                        dp_base.add(row * row_pitch),
                        16,
                    );
                }
            }
        } else {
            // Slow path: block straddles the right/bottom edge — clip per texel.
            for row in 0..4usize {
                let img_y = img_y0 + row;
                if img_y >= h_us { break; }
                for col in 0..4usize {
                    let img_x = img_x0 + col;
                    if img_x >= w_us { break; }
                    let src = (row * 4 + col) * 4;
                    let dst = (img_y * w_us + img_x) * 4;
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            decoded.as_ptr().add(src),
                            out.as_mut_ptr().add(dst),
                            4,
                        );
                    }
                }
            }
        }
    }

    out
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-computed mode 0 block.
    ///
    /// Mode 0 layout (bits, LSB-first):
    ///   [5:0]   = 0b000000  (mode bits)
    ///   [11:6]  = r0 (6 bits)
    ///   [17:12] = g0 (6 bits)
    ///   [23:18] = b0 (6 bits)
    ///   [29:24] = r1 (6 bits)
    ///   [35:30] = g1 (6 bits)
    ///   [41:36] = b1 (6 bits)
    ///   [45:42] = w[0]  (4 bits)
    ///   ...      w[1..15] follow, 4 bits each
    ///
    /// We set:
    ///   r0=0b111111(63), g0=0, b0=0  → expands to (255,0,0)
    ///   r1=0,            g1=0, b1=0  → expands to (0,0,0)
    ///   all weights = 0b0000 (index 0, weight 0) → result = e0 = (255,0,0)
    #[test]
    fn mode0_solid_red() {
        // Build the 128-bit block value by hand.
        // bits [5:0]   = 0b000000   (mode 0)
        // bits [11:6]  = 0b111111   (r0 = 63)
        // bits [17:12] = 0b000000   (g0 = 0)
        // bits [23:18] = 0b000000   (b0 = 0)
        // bits [29:24] = 0b000000   (r1 = 0)
        // bits [35:30] = 0b000000   (g1 = 0)
        // bits [41:36] = 0b000000   (b1 = 0)
        // bits [105:42]= 0  (all 16 weights = 0b0000)
        let mut val: u128 = 0;
        // r0 = 63 = 0b111111 at bits [11:6]
        val |= 63u128 << 6;
        let block: [u8; 16] = val.to_le_bytes();

        let out = decode_block(&block);

        // Every texel must be (255, 0, 0, 255).
        for t in 0..16 {
            let base = t * 4;
            assert_eq!(out[base],     255, "texel {t} R");
            assert_eq!(out[base + 1],   0, "texel {t} G");
            assert_eq!(out[base + 2],   0, "texel {t} B");
            assert_eq!(out[base + 3], 255, "texel {t} A");
        }
    }

    /// Mode 0 block where all weights = index 15 (weight 64) → output = e1.
    #[test]
    fn mode0_solid_blue_via_e1() {
        // r0=0, g0=0, b0=0, r1=0, g1=0, b1=63
        // All 16 weights = 0b1111 (index 15, weight 64)
        let mut val: u128 = 0;
        // b1 is at bits [41:36]
        val |= 63u128 << 36;
        // weights: 16 × 4 bits starting at bit 42, all = 0b1111 = 15
        for t in 0..16u32 {
            val |= 15u128 << (42 + t * 4);
        }
        let block: [u8; 16] = val.to_le_bytes();
        let out = decode_block(&block);

        for t in 0..16 {
            let base = t * 4;
            // b1=63 expands to: (63<<2)|(63>>4) = 252|3 = 255
            assert_eq!(out[base],       0, "texel {t} R");
            assert_eq!(out[base + 1],   0, "texel {t} G");
            assert_eq!(out[base + 2], 255, "texel {t} B");
            assert_eq!(out[base + 3], 255, "texel {t} A");
        }
    }

    /// Mode 1: RGB 8.8.8, 2-bit weights.
    /// Set e0=(200,100,50), e1=(0,0,0), all weights=0 → output = e0.
    #[test]
    fn mode1_solid_color() {
        let mut val: u128 = 0;
        // Mode bits [5:0] = 0b000001
        val |= 1u128;
        // r0 at bits [13:6]
        val |= 200u128 << 6;
        // g0 at bits [21:14]
        val |= 100u128 << 14;
        // b0 at bits [29:22]
        val |= 50u128 << 22;
        // r1, g1, b1 = 0 → left at 0
        // all 16 weights = 0b00 (2 bits each), starting at bit 54, all zero → already 0
        let block: [u8; 16] = val.to_le_bytes();
        let out = decode_block(&block);

        for t in 0..16 {
            let base = t * 4;
            assert_eq!(out[base],     200, "texel {t} R");
            assert_eq!(out[base + 1], 100, "texel {t} G");
            assert_eq!(out[base + 2],  50, "texel {t} B");
            assert_eq!(out[base + 3], 255, "texel {t} A");
        }
    }

    /// Mode interpolation math sanity check.
    #[test]
    fn interpolate_midpoint() {
        // weight index 2 in WEIGHTS4 = 9/64 … let's use WEIGHTS2[2] = 43
        // e0=0, e1=128, w=43 → (0*21 + 43*128 + 32) / 64 = (5504+32)/64 = 86
        let result = interpolate(0, 128, WEIGHTS2[2]);
        assert_eq!(result, 86);
    }

    /// 4-channel SIMD interpolate must match four scalar calls byte-for-byte.
    #[test]
    fn interpolate_rgba_simd_matches_scalar() {
        let lo = [12u8, 200, 0, 255];
        let hi = [240u8, 5, 128, 0];
        for &w in &[0u8, 16, 32, 43, 64] {
            let scalar = [
                interpolate(lo[0], hi[0], w),
                interpolate(lo[1], hi[1], w),
                interpolate(lo[2], hi[2], w),
                interpolate(lo[3], hi[3], w),
            ];
            let simd = interpolate_rgba(lo, hi, w);
            assert_eq!(scalar, simd, "weight {w}: scalar={:?} simd={:?}", scalar, simd);
        }
    }

    /// Partition table sanity check.
    #[test]
    fn partition_pattern0() {
        // Pattern 0: texels 0-7 (rows 0-1) = subset 0,
        //            texels 8-15 (rows 2-3) = subset 1.
        // Bit N=0 → subset 0, bit N=1 → subset 1.
        let p = PARTITION_TABLE[0];
        for t in 0..8 {
            assert_eq!(subset_of(p, t), 0, "texel {t} should be subset 0");
        }
        for t in 8..16 {
            assert_eq!(subset_of(p, t), 1, "texel {t} should be subset 1");
        }
    }

    /// transcode_to_rgba8 dimensions and output length.
    #[test]
    fn transcode_output_length() {
        // 8×4 image = 2×1 blocks = 2 blocks
        let blocks = vec![0u8; 32]; // 2 × 16 bytes, all-zero (mode 0)
        let out = transcode_to_rgba8(&blocks, 8, 4);
        assert_eq!(out.len(), 8 * 4 * 4);
    }
}
