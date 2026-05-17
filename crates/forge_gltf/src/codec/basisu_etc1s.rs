//! BasisU ETC1S decoder / transcoder.
//!
//! Decodes the ETC1S intermediate format used by BasisLZ (KTX2 supercompression
//! scheme 1).  Produces RGBA8 output with alpha=255 (ETC1S is RGB-only; alpha
//! planes are handled separately at the container layer).
//!
//! # Data-flow
//! ```text
//! KTX2 Supercompression Global Data (SGD)
//!   ├─ SGD header (18 bytes)
//!   ├─ Huffman tables   (tables_size bytes)
//!   ├─ Endpoint records (endpoint_count × 4 bytes)
//!   └─ Selector records (selector_count × 8 bytes)
//!
//! Per-level slice data
//!   └─ Huffman-coded (endpoint_delta, selector_delta) stream
//! ```
//!
//! Reference: Khronos KTX2 specification §4.4, BasisU open format document.
//!
//! # Performance notes
//! * Endpoint RGB5 → RGB8 expansion happens once at SGD parse time; all 16
//!   texels in a block share the same pre-expanded values.
//! * `decode_etc1s_block` is `#[inline(always)]` and purely arithmetic—no
//!   allocation on the hot path.
//! * The Huffman decode table is a flat array indexed by the raw bit peek,
//!   giving O(1) per-symbol decode.

use thin_vec::ThinVec;

use crate::error::{GltfError, GltfResult};

// ---------------------------------------------------------------------------
// ETC1 intensity modifier table (standard, from the ETC1/ETC2 specification)
// ---------------------------------------------------------------------------

#[rustfmt::skip]
const INTENSITY_TABLE: [[i32; 4]; 8] = [
    [  2,   8,   -2,   -8],
    [  5,  17,   -5,  -17],
    [  9,  29,   -9,  -29],
    [ 13,  42,  -13,  -42],
    [ 18,  60,  -18,  -60],
    [ 24,  80,  -24,  -80],
    [ 33, 106,  -33, -106],
    [ 47, 183,  -47, -183],
];

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A decoded ETC1S colour endpoint.
///
/// `r5`, `g5`, `b5` are raw 5-bit values (0..=31).
/// `inten_table` is the intensity modifier table index (0..=7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EndpointEntry {
    pub r5: u8,
    pub g5: u8,
    pub b5: u8,
    pub inten_table: u8,
}

/// A decoded ETC1S selector block.
///
/// `selectors[y][x]` is a 2-bit value (0..=3) for the texel at column `x`,
/// row `y` (row-major, origin top-left).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectorEntry {
    pub selectors: [[u8; 4]; 4],
}

// ---------------------------------------------------------------------------
// 5-bit → 8-bit channel expansion
// ---------------------------------------------------------------------------

/// Expand a 5-bit channel value to 8 bits: `v8 = (v5 << 3) | (v5 >> 2)`.
#[inline(always)]
fn expand5(v: u8) -> u8 {
    (v << 3) | (v >> 2)
}

// ---------------------------------------------------------------------------
// Block decoder
// ---------------------------------------------------------------------------

/// Decode one ETC1S 4×4 block to 64 bytes of packed RGBA8 (alpha = 255).
///
/// Texels are written in row-major order (x varies fastest inside each row).
#[inline(always)]
pub fn decode_etc1s_block(ep: &EndpointEntry, sel: &SelectorEntry) -> [u8; 64] {
    let base_r = expand5(ep.r5) as i32;
    let base_g = expand5(ep.g5) as i32;
    let base_b = expand5(ep.b5) as i32;
    let row = &INTENSITY_TABLE[(ep.inten_table & 7) as usize];

    let mut out = [0u8; 64];
    let mut i = 0usize;

    let mut y = 0usize;
    while y < 4 {
        let mut x = 0usize;
        while x < 4 {
            let modifier = row[(sel.selectors[y][x] & 3) as usize];
            let r = (base_r + modifier).clamp(0, 255) as u8;
            let g = (base_g + modifier).clamp(0, 255) as u8;
            let b = (base_b + modifier).clamp(0, 255) as u8;
            out[i] = r;
            out[i + 1] = g;
            out[i + 2] = b;
            out[i + 3] = 255;
            i += 4;
            x += 1;
        }
        y += 1;
    }
    out
}

// ---------------------------------------------------------------------------
// LSB-first bit-stream reader
// ---------------------------------------------------------------------------

/// A simple LSB-first sliding-window bit reader over a byte slice.
///
/// Maintains a 32-bit window that is refilled from the byte stream as
/// needed.  All reads are at most 16 bits so the window never overflows.
struct BitStream<'a> {
    data: &'a [u8],
    /// Byte offset of the next byte to load into the window.
    byte_pos: usize,
    /// Cached window bits (LSB-aligned, i.e. next bit is in bit 0).
    window: u32,
    /// Number of valid bits currently in `window`.
    bits_in: u32,
}

impl<'a> BitStream<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            byte_pos: 0,
            window: 0,
            bits_in: 0,
        }
    }

    /// Refill the window until it has at least 24 valid bits (or stream end).
    #[inline(always)]
    fn refill(&mut self) {
        while self.bits_in <= 24 && self.byte_pos < self.data.len() {
            self.window |= (self.data[self.byte_pos] as u32) << self.bits_in;
            self.bits_in += 8;
            self.byte_pos += 1;
        }
    }

    /// Read exactly `n` bits (n ≤ 16) in LSB-first order. The Huffman
    /// decoder inlines peek-and-consume directly against `window` /
    /// `bits_in` to avoid the redundant refill the general `read` would
    /// re-issue, so this method is only reachable from the bit-stream
    /// unit tests in this module.
    #[cfg(test)]
    #[inline(always)]
    fn read(&mut self, n: u32) -> GltfResult<u32> {
        self.refill();
        if self.bits_in < n {
            return Err(GltfError::SpecViolation(
                "ETC1S bitstream underflow".to_string(),
            ));
        }
        let v = self.window & ((1u32 << n) - 1);
        self.window >>= n;
        self.bits_in -= n;
        Ok(v)
    }

    /// True when there are any bits remaining in the stream.
    #[inline(always)]
    fn has_bits(&self) -> bool {
        self.bits_in > 0 || self.byte_pos < self.data.len()
    }
}

// ---------------------------------------------------------------------------
// Huffman decode table
// ---------------------------------------------------------------------------

/// Maximum Huffman code length we support (BasisU uses at most 16 bits).
const MAX_CODE_LEN: u32 = 16;

/// One entry in the flat decode table.
#[derive(Clone, Copy)]
struct HuffEntry {
    /// The decoded symbol.
    symbol: u16,
    /// Length of the code that produced this entry (0 = unused slot).
    len: u8,
}

/// Flat canonical Huffman decode table with 2^MAX_CODE_LEN entries.
///
/// Indexed by the *LSB-first* bit peek of `MAX_CODE_LEN` bits: because
/// canonical Huffman codes are prefix-free, every code occupies a
/// contiguous power-of-two band of table indices.
struct HuffTable {
    entries: Box<[HuffEntry; 1 << MAX_CODE_LEN]>,
}

impl HuffTable {
    /// Build a flat decode table from `(symbol, code_len)` pairs.
    ///
    /// Pairs need not be sorted.  Code lengths must be in [1, MAX_CODE_LEN].
    fn build(pairs: &[(u16, u8)]) -> GltfResult<Self> {
        // Count symbols per length.
        let mut bl_count = [0u32; MAX_CODE_LEN as usize + 1];
        for &(_, len) in pairs {
            if len == 0 || len as u32 > MAX_CODE_LEN {
                return Err(GltfError::UnsupportedFeature(
                    "ETC1S Huffman variant: code length out of range".to_string(),
                ));
            }
            bl_count[len as usize] += 1;
        }

        // Canonical starting code for each length.
        let mut next_code = [0u32; MAX_CODE_LEN as usize + 1];
        let mut code = 0u32;
        for bits in 1..=MAX_CODE_LEN as usize {
            code = (code + bl_count[bits - 1]) << 1;
            next_code[bits] = code;
        }

        // Sort by (len, symbol) to assign canonical codes consistently.
        let mut sorted: Vec<(u16, u8)> = pairs.to_vec();
        sorted.sort_unstable_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));

        // Initialise the flat table with "unused" entries.
        let blank = HuffEntry { symbol: 0, len: 0 };
        let mut entries = Box::new([blank; 1 << MAX_CODE_LEN]);

        for &(sym, len) in &sorted {
            let l = len as u32;
            let c = next_code[l as usize];
            next_code[l as usize] += 1;

            // For LSB-first decoding, the canonical code appears in the low `l`
            // bits of the bit-stream window exactly as-is (no reversal needed).
            // Each code of length `l` fills 2^(MAX_CODE_LEN - l) table slots
            // at stride 2^l so that all possible upper-bit patterns resolve
            // to the correct symbol.
            let base = c;
            let fill_count = 1u32 << (MAX_CODE_LEN - l);
            for k in 0..fill_count {
                let idx = (base | (k << l)) as usize;
                if idx >= (1 << MAX_CODE_LEN) {
                    return Err(GltfError::UnsupportedFeature(
                        "ETC1S Huffman variant: table overflow".to_string(),
                    ));
                }
                entries[idx] = HuffEntry { symbol: sym, len };
            }
        }

        Ok(Self { entries })
    }

    /// Decode one symbol from `bs`.
    #[inline(always)]
    fn decode(&self, bs: &mut BitStream<'_>) -> GltfResult<u16> {
        bs.refill();
        // Peek up to MAX_CODE_LEN bits (or however many are available).
        let avail = bs.bits_in.min(MAX_CODE_LEN);
        if avail == 0 {
            return Err(GltfError::SpecViolation(
                "ETC1S Huffman: stream exhausted".to_string(),
            ));
        }
        let peek_val = bs.window & ((1u32 << avail) - 1);
        let idx = peek_val as usize;
        let entry = self.entries[idx];
        if entry.len == 0 {
            return Err(GltfError::SpecViolation(
                "ETC1S Huffman: invalid code".to_string(),
            ));
        }
        // Consume only the code bits.
        let consume = entry.len as u32;
        bs.window >>= consume;
        bs.bits_in -= consume;
        Ok(entry.symbol)
    }
}

/// Reverse the lowest `bits` bits of `v`, producing a `bits`-wide integer.
/// Kept under `cfg(test)` because the production Huffman decoder uses LSB-
/// first canonical codes (no reversal needed) — the helper is retained for
/// the bit-reversal unit tests so future MSB-first variants can be
/// validated against a known-correct reference.
#[cfg(test)]
#[inline(always)]
fn reverse_bits(v: u32, bits: u32) -> u32 {
    if bits == 0 {
        return 0;
    }
    v.reverse_bits() >> (32 - bits)
}

// ---------------------------------------------------------------------------
// SGD (Supercompression Global Data) parsing
// ---------------------------------------------------------------------------

/// Decoded SGD: pre-built codebooks ready for block reconstruction.
struct Sgd {
    endpoints: Vec<EndpointEntry>,
    selectors: Vec<SelectorEntry>,
    /// Huffman tree for endpoint delta symbols.
    ep_table: HuffTable,
    /// Huffman tree for selector delta symbols.
    sel_table: HuffTable,
}

/// Byte size of the fixed SGD header.
const SGD_HEADER_SIZE: usize = 18;

/// Read a little-endian u16 from `src` at byte offset `off`.
#[inline(always)]
fn read_u16_le(src: &[u8], off: usize) -> GltfResult<u16> {
    src.get(off..off + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or_else(|| GltfError::SpecViolation("ETC1S SGD: truncated (u16 read)".to_string()))
}

/// Read a little-endian u32 from `src` at byte offset `off`.
#[inline(always)]
fn read_u32_le(src: &[u8], off: usize) -> GltfResult<u32> {
    src.get(off..off + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| GltfError::SpecViolation("ETC1S SGD: truncated (u32 read)".to_string()))
}

/// Parse the complete BasisLZ Supercompression Global Data blob.
fn parse_sgd(sgd: &[u8]) -> GltfResult<Sgd> {
    // ---- Header ----
    if sgd.len() < SGD_HEADER_SIZE {
        return Err(GltfError::SpecViolation(
            "ETC1S SGD: blob too small for header".to_string(),
        ));
    }

    let endpoint_count = read_u16_le(sgd, 0)? as usize;
    let selector_count = read_u16_le(sgd, 2)? as usize;
    let tables_size = read_u32_le(sgd, 4)? as usize;
    // Bytes 8..18: extended_filesize(u32), old_header_file_size(u32),
    //              header_size(u16) — all unused by the decoder.

    let tables_start = SGD_HEADER_SIZE;
    let tables_end = tables_start + tables_size;
    let endpoints_start = tables_end;
    let endpoints_end = endpoints_start + endpoint_count * 4;
    let selectors_start = endpoints_end;
    let selectors_end = selectors_start + selector_count * 8;

    if sgd.len() < selectors_end {
        return Err(GltfError::SpecViolation(
            "ETC1S SGD: blob too small for codebooks".to_string(),
        ));
    }

    // ---- Huffman tables ----
    let (ep_table, sel_table) = parse_huffman_tables(&sgd[tables_start..tables_end])?;

    // ---- Endpoints ----
    let endpoints = parse_endpoints(&sgd[endpoints_start..endpoints_end], endpoint_count)?;

    // ---- Selectors ----
    let selectors = parse_selectors(&sgd[selectors_start..selectors_end], selector_count)?;

    Ok(Sgd {
        endpoints,
        selectors,
        ep_table,
        sel_table,
    })
}

// ---------------------------------------------------------------------------
// Huffman table section parsing
// ---------------------------------------------------------------------------

/// Parse the SGD tables section into (endpoint_tree, selector_tree).
///
/// BasisU stores two sequential canonical Huffman trees.  Each tree is encoded as:
/// ```text
/// u16  total_used_syms           // count of (sym, len) pairs
/// [total_used_syms × { u16 sym, u8 len }]
/// ```
fn parse_huffman_tables(data: &[u8]) -> GltfResult<(HuffTable, HuffTable)> {
    let mut pos = 0usize;
    let ep_table = read_one_huffman_tree(data, &mut pos)?;
    let sel_table = read_one_huffman_tree(data, &mut pos)?;
    Ok((ep_table, sel_table))
}

/// Read one Huffman tree from `data` starting at `*pos`, advancing `*pos`.
fn read_one_huffman_tree(data: &[u8], pos: &mut usize) -> GltfResult<HuffTable> {
    if *pos + 2 > data.len() {
        return Err(GltfError::SpecViolation(
            "ETC1S Huffman tree header: truncated".to_string(),
        ));
    }
    let total = u16::from_le_bytes([data[*pos], data[*pos + 1]]) as usize;
    *pos += 2;

    if total == 0 {
        return Err(GltfError::UnsupportedFeature(
            "ETC1S Huffman variant: empty symbol table".to_string(),
        ));
    }

    // Each record: 2-byte sym + 1-byte len = 3 bytes.
    let needed = total * 3;
    if *pos + needed > data.len() {
        return Err(GltfError::SpecViolation(
            "ETC1S Huffman tree: symbol records truncated".to_string(),
        ));
    }

    let mut pairs: Vec<(u16, u8)> = Vec::with_capacity(total);
    for _ in 0..total {
        let sym = u16::from_le_bytes([data[*pos], data[*pos + 1]]);
        let len = data[*pos + 2];
        *pos += 3;
        if len == 0 || len as u32 > MAX_CODE_LEN {
            return Err(GltfError::UnsupportedFeature(
                "ETC1S Huffman variant: code length out of range".to_string(),
            ));
        }
        pairs.push((sym, len));
    }

    HuffTable::build(&pairs)
}

// ---------------------------------------------------------------------------
// Endpoint / selector codebook parsing
// ---------------------------------------------------------------------------

/// Parse `count` endpoint records from `data` (4 bytes each).
///
/// Record layout:
/// ```text
/// u16 color5  bits[4:0]=R5, bits[9:5]=G5, bits[14:10]=B5
/// u8  inten   low 3 bits = intensity table index
/// u8  padding (ignored)
/// ```
fn parse_endpoints(data: &[u8], count: usize) -> GltfResult<Vec<EndpointEntry>> {
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let off = i * 4;
        let color5 = u16::from_le_bytes([data[off], data[off + 1]]);
        let inten = data[off + 2] & 0x07;

        let r5 = (color5 & 0x1F) as u8;
        let g5 = ((color5 >> 5) & 0x1F) as u8;
        let b5 = ((color5 >> 10) & 0x1F) as u8;

        out.push(EndpointEntry {
            r5,
            g5,
            b5,
            inten_table: inten,
        });
    }
    Ok(out)
}

/// Parse `count` selector records from `data` (8 bytes each).
///
/// Layout: 8 bytes per block = 4 rows × 1 byte (4 × 2-bit selectors per row).
/// Within each byte: bits [1:0] = x=0, bits [3:2] = x=1, bits [5:4] = x=2,
/// bits [7:6] = x=3.  The upper 4 bytes carry no information for RGB
/// transcoding and are ignored.
fn parse_selectors(data: &[u8], count: usize) -> GltfResult<Vec<SelectorEntry>> {
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let off = i * 8;
        let mut e = SelectorEntry {
            selectors: [[0u8; 4]; 4],
        };
        for row in 0..4usize {
            let byte = data[off + row];
            for col in 0..4usize {
                e.selectors[row][col] = (byte >> (col * 2)) & 0x03;
            }
        }
        // Bytes off+4..off+8 are the high-half selectors used in the ETC1S
        // dual-plane alpha path — not needed for RGB-only transcoding.
        out.push(e);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Zigzag decode (for delta-coded codebook indices)
// ---------------------------------------------------------------------------

/// Decode a zigzag-encoded unsigned integer to a signed delta.
///
/// Standard mapping: 0→0, 1→−1, 2→1, 3→−2, 4→2, …
#[inline(always)]
fn zigzag_decode(v: u32) -> i32 {
    ((v >> 1) as i32) ^ -((v & 1) as i32)
}

// ---------------------------------------------------------------------------
// Slice transcoder
// ---------------------------------------------------------------------------

/// Transcode one ETC1S slice to RGBA8, appending to `out`.
///
/// Blocks are processed in row-major order.  The output region starts at
/// `out.len()` before the call and grows by `width × height × 4` bytes.
fn transcode_slice(
    level_data: &[u8],
    width: u32,
    height: u32,
    sgd: &Sgd,
    out: &mut ThinVec<u8>,
) -> GltfResult<()> {
    let bw = ((width + 3) / 4) as usize;
    let bh = ((height + 3) / 4) as usize;
    let total_blocks = bw * bh;

    let out_len = (width as usize) * (height as usize) * 4;
    let base_offset = out.len();

    // Zero-fill the output region (covers non-multiple-of-4 border texels).
    out.resize(base_offset + out_len, 0u8);

    let ep_count = sgd.endpoints.len() as u32;
    let sel_count = sgd.selectors.len() as u32;

    if ep_count == 0 || sel_count == 0 {
        return Ok(());
    }

    let mut bs = BitStream::new(level_data);
    let mut prev_ep_idx = 0u32;
    let mut prev_sel_idx = 0u32;

    for bi in 0..total_blocks {
        if !bs.has_bits() {
            break;
        }

        // Endpoint index: zigzag-decoded delta from the previous index.
        let ep_sym = sgd.ep_table.decode(&mut bs)?;
        let ep_delta = zigzag_decode(ep_sym as u32);
        // Wrap modulo codebook size.
        prev_ep_idx = prev_ep_idx.wrapping_add(ep_delta as u32) % ep_count;

        // Selector index: same scheme.
        let sel_sym = sgd.sel_table.decode(&mut bs)?;
        let sel_delta = zigzag_decode(sel_sym as u32);
        prev_sel_idx = prev_sel_idx.wrapping_add(sel_delta as u32) % sel_count;

        let ep = &sgd.endpoints[prev_ep_idx as usize];
        let sel = &sgd.selectors[prev_sel_idx as usize];
        let texels = decode_etc1s_block(ep, sel);

        // Scatter the 4×4 decoded block into the linear output image.
        let bx = bi % bw;
        let by = bi / bw;

        for row in 0..4usize {
            let img_y = by * 4 + row;
            if img_y >= height as usize {
                break;
            }
            for col in 0..4usize {
                let img_x = bx * 4 + col;
                if img_x >= width as usize {
                    break;
                }
                let src = (row * 4 + col) * 4;
                let dst = base_offset + (img_y * width as usize + img_x) * 4;
                out[dst] = texels[src];
                out[dst + 1] = texels[src + 1];
                out[dst + 2] = texels[src + 2];
                out[dst + 3] = texels[src + 3];
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Transcode ETC1S encoded in a BasisLZ KTX2.
///
/// # Arguments
/// * `sgd`        — the Supercompression Global Data blob from the KTX2 container.
/// * `level_data` — the compressed level bytes for one mip level / slice.
/// * `width`      — image width in texels.
/// * `height`     — image height in texels.
///
/// # Returns
/// A `ThinVec<u8>` of `width × height × 4` bytes of RGBA8 pixel data.
/// Alpha is 255 throughout (ETC1S is an RGB-only format).
pub fn transcode_to_rgba8(
    sgd: &[u8],
    level_data: &[u8],
    width: u32,
    height: u32,
) -> GltfResult<ThinVec<u8>> {
    if width == 0 || height == 0 {
        return Ok(ThinVec::new());
    }

    let decoded_sgd = parse_sgd(sgd)?;

    let out_len = (width as usize) * (height as usize) * 4;
    let mut out: ThinVec<u8> = ThinVec::with_capacity(out_len);

    transcode_slice(level_data, width, height, &decoded_sgd, &mut out)?;

    Ok(out)
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn ep(r5: u8, g5: u8, b5: u8, inten: u8) -> EndpointEntry {
        EndpointEntry {
            r5,
            g5,
            b5,
            inten_table: inten,
        }
    }

    fn sel_uniform(s: u8) -> SelectorEntry {
        SelectorEntry {
            selectors: [[s & 3; 4]; 4],
        }
    }

    // -----------------------------------------------------------------------
    // expand5
    // -----------------------------------------------------------------------

    #[test]
    fn expand5_zero_and_max() {
        assert_eq!(expand5(0), 0u8);
        assert_eq!(expand5(31), 255u8); // (31<<3)|(31>>2) = 248|7 = 255
    }

    #[test]
    fn expand5_midpoint() {
        // v=16=0b10000 → (16<<3)|(16>>2) = 128|4 = 132
        assert_eq!(expand5(16), 132);
    }

    #[test]
    fn expand5_all_distinct() {
        // All 32 five-bit values must map to distinct 8-bit outputs.
        let mut seen = [false; 256];
        for v in 0u8..=31 {
            let e = expand5(v) as usize;
            assert!(!seen[e], "expand5({v}) = {e} is a duplicate");
            seen[e] = true;
        }
    }

    // -----------------------------------------------------------------------
    // decode_etc1s_block: known arithmetic results
    // -----------------------------------------------------------------------

    /// inten_table=0, sel=0 → modifier=+2.
    #[test]
    fn block_inten0_sel0_adds_two() {
        // r5=15 → expand5(15) = (15<<3)|(15>>2) = 120|3 = 123
        // modifier = INTENSITY_TABLE[0][0] = +2 → channel = 125
        let out = decode_etc1s_block(&ep(15, 15, 15, 0), &sel_uniform(0));
        for t in 0..16usize {
            let b = t * 4;
            assert_eq!(out[b], 125, "texel {t} R");
            assert_eq!(out[b + 1], 125, "texel {t} G");
            assert_eq!(out[b + 2], 125, "texel {t} B");
            assert_eq!(out[b + 3], 255, "texel {t} A");
        }
    }

    /// inten_table=0, sel=1 → modifier=+8.
    #[test]
    fn block_inten0_sel1() {
        // r5=0 → r8=0; modifier=+8 → 8
        let out = decode_etc1s_block(&ep(0, 0, 0, 0), &sel_uniform(1));
        for t in 0..16usize {
            let b = t * 4;
            assert_eq!(out[b], 8, "texel {t} R");
            assert_eq!(out[b + 1], 8, "texel {t} G");
            assert_eq!(out[b + 2], 8, "texel {t} B");
            assert_eq!(out[b + 3], 255);
        }
    }

    /// inten_table=0, sel=2 → modifier=-2; clamped to 0 when base=0.
    #[test]
    fn block_inten0_sel2_clamps_zero() {
        let out = decode_etc1s_block(&ep(0, 0, 0, 0), &sel_uniform(2));
        for t in 0..16usize {
            assert_eq!(out[t * 4], 0, "texel {t}");
        }
    }

    /// inten_table=7, sel=1 → modifier=+183; clamps to 255 when base=255.
    #[test]
    fn block_inten7_sel1_clamps_high() {
        // r5=31 → r8=255
        let out = decode_etc1s_block(&ep(31, 31, 31, 7), &sel_uniform(1));
        for t in 0..16usize {
            let b = t * 4;
            assert_eq!(out[b], 255, "texel {t} R");
            assert_eq!(out[b + 1], 255, "texel {t} G");
            assert_eq!(out[b + 2], 255, "texel {t} B");
            assert_eq!(out[b + 3], 255);
        }
    }

    /// inten_table=7, sel=3 → modifier=-183; clamps to 0.
    #[test]
    fn block_inten7_sel3_clamps_low() {
        let out = decode_etc1s_block(&ep(0, 0, 0, 7), &sel_uniform(3));
        for t in 0..16usize {
            let b = t * 4;
            assert_eq!(out[b], 0, "texel {t} R");
            assert_eq!(out[b + 1], 0, "texel {t} G");
            assert_eq!(out[b + 2], 0, "texel {t} B");
            assert_eq!(out[b + 3], 255);
        }
    }

    /// Mixed row-wise selectors with known expected values.
    #[test]
    fn block_mixed_selectors() {
        // r5=16 → r8=132; g5=b5=0; inten_table=1: [5,17,-5,-17]
        // row 0: sel 0 → modifier=+5  → r=137
        // row 1: sel 1 → modifier=+17 → r=149
        // row 2: sel 2 → modifier=-5  → r=127
        // row 3: sel 3 → modifier=-17 → r=115
        let sel = SelectorEntry {
            selectors: [[0, 0, 0, 0], [1, 1, 1, 1], [2, 2, 2, 2], [3, 3, 3, 3]],
        };
        let out = decode_etc1s_block(&ep(16, 0, 0, 1), &sel);

        // g5=b5=0 → base 0; modifier also applied to G and B:
        // sel 0→+5: G=clamp(0+5)=5; sel 1→+17: G=17; sel 2→-5: G=0; sel 3→-17: G=0
        let expected_r = [137u8, 149, 127, 115];
        let expected_g = [5u8, 17, 0, 0];
        for row in 0..4usize {
            for col in 0..4usize {
                let b = (row * 4 + col) * 4;
                assert_eq!(out[b], expected_r[row], "row={row} col={col} R");
                assert_eq!(out[b + 1], expected_g[row], "row={row} col={col} G");
                assert_eq!(out[b + 2], expected_g[row], "row={row} col={col} B");
                assert_eq!(out[b + 3], 255, "row={row} col={col} A");
            }
        }
    }

    // -----------------------------------------------------------------------
    // zigzag_decode
    // -----------------------------------------------------------------------

    #[test]
    fn zigzag_known_values() {
        assert_eq!(zigzag_decode(0), 0);
        assert_eq!(zigzag_decode(1), -1);
        assert_eq!(zigzag_decode(2), 1);
        assert_eq!(zigzag_decode(3), -2);
        assert_eq!(zigzag_decode(4), 2);
    }

    // -----------------------------------------------------------------------
    // BitStream
    // -----------------------------------------------------------------------

    #[test]
    fn bitstream_sequential_reads() {
        // 0b10110011 = 0xB3
        let data = [0xB3u8, 0x4Au8]; // 0x4A = 0b01001010
        let mut bs = BitStream::new(&data);
        assert_eq!(bs.read(4).unwrap(), 0b0011, "low nibble");
        assert_eq!(bs.read(4).unwrap(), 0b1011, "high nibble");
        assert_eq!(bs.read(8).unwrap(), 0b01001010, "second byte");
    }

    #[test]
    fn bitstream_single_bits() {
        let data = [0b10110011u8];
        let mut bs = BitStream::new(&data);
        let bits: Vec<u32> = (0..8).map(|_| bs.read(1).unwrap()).collect();
        // LSB-first: 1, 1, 0, 0, 1, 1, 0, 1
        assert_eq!(bits, [1, 1, 0, 0, 1, 1, 0, 1]);
    }

    // -----------------------------------------------------------------------
    // reverse_bits
    // -----------------------------------------------------------------------

    #[test]
    fn reverse_bits_examples() {
        assert_eq!(reverse_bits(0b001, 3), 0b100);
        assert_eq!(reverse_bits(0b110, 3), 0b011);
        assert_eq!(reverse_bits(0b1000, 4), 0b0001);
        assert_eq!(reverse_bits(0b0000, 4), 0b0000);
    }

    // -----------------------------------------------------------------------
    // HuffTable: two-symbol tree
    // -----------------------------------------------------------------------

    #[test]
    fn hufftable_two_symbols() {
        // sym 0 → len 1, canonical code 0  (reversed: 0b0...0 = index 0..half)
        // sym 1 → len 1, canonical code 1  (reversed: 1 << (MAX-1) = high half)
        let ht = HuffTable::build(&[(0, 1), (1, 1)]).unwrap();

        // Stream byte 0b0000_0001: LSB bit0=1 → sym1, bit1=0 → sym0
        let mut bs = BitStream::new(&[0b0000_0001u8]);
        assert_eq!(ht.decode(&mut bs).unwrap(), 1, "first code (1) → sym 1");
        assert_eq!(ht.decode(&mut bs).unwrap(), 0, "second code (0) → sym 0");
    }

    #[test]
    fn hufftable_three_symbols_mixed_lengths() {
        // Canonical assignment (sorted by len then sym):
        //   sym 0 → len 1 → code 0
        //   sym 1 → len 2 → code 10
        //   sym 2 → len 2 → code 11
        //
        // LSB-first encoding of stream [sym0, sym1, sym2]:
        //   code(sym0) = 0   (1 bit)  : bit 0 = 0
        //   code(sym1) = 10  (2 bits) : bits 1-2 = 0,1  (LSB first: bit1=0 bit2=1)
        //   code(sym2) = 11  (2 bits) : bits 3-4 = 1,1
        //   raw byte = 0b_0001_1100 = 0x1C (bits 7..0)
        let ht = HuffTable::build(&[(0, 1), (1, 2), (2, 2)]).unwrap();
        let mut bs = BitStream::new(&[0x1Cu8]);

        assert_eq!(ht.decode(&mut bs).unwrap(), 0, "sym0");
        assert_eq!(ht.decode(&mut bs).unwrap(), 1, "sym1");
        assert_eq!(ht.decode(&mut bs).unwrap(), 2, "sym2");
    }

    // -----------------------------------------------------------------------
    // parse_endpoints / parse_selectors: round-trip verification
    // -----------------------------------------------------------------------

    #[test]
    fn parse_endpoint_roundtrip() {
        // Encode R5=10, G5=20, B5=5, inten=3.
        let color5: u16 = 10u16 | (20u16 << 5) | (5u16 << 10);
        let bytes: [u8; 4] = [(color5 & 0xFF) as u8, (color5 >> 8) as u8, 3u8, 0u8];
        let eps = parse_endpoints(&bytes, 1).unwrap();
        assert_eq!(eps[0].r5, 10);
        assert_eq!(eps[0].g5, 20);
        assert_eq!(eps[0].b5, 5);
        assert_eq!(eps[0].inten_table, 3);
    }

    #[test]
    fn parse_selector_first_row() {
        // Row 0: sel[0][x] = x → packed: (3<<6)|(2<<4)|(1<<2)|0 = 0b11_10_01_00 = 0xE4
        let mut bytes = [0u8; 8];
        bytes[0] = 0b11_10_01_00u8;
        let sels = parse_selectors(&bytes, 1).unwrap();
        assert_eq!(sels[0].selectors[0], [0, 1, 2, 3]);
        // Remaining rows zeroed.
        for row in 1..4 {
            assert_eq!(sels[0].selectors[row], [0, 0, 0, 0], "row {row}");
        }
    }

    // -----------------------------------------------------------------------
    // transcode_to_rgba8: integration smoke tests
    // -----------------------------------------------------------------------

    /// Helper: build a minimal well-formed SGD + slice for a single solid-colour
    /// block with known R/G/B values.
    ///
    /// Uses a 1-symbol Huffman tree for both tables (symbol 0, len 1).
    /// The slice encodes two symbols: ep_delta=0, sel_delta=0.
    fn make_minimal_sgd_and_slice(r5: u8, g5: u8, b5: u8, inten: u8) -> (Vec<u8>, Vec<u8>) {
        // ---- Endpoint bytes ----
        let color5: u16 = (r5 as u16) | ((g5 as u16) << 5) | ((b5 as u16) << 10);
        let ep_bytes: Vec<u8> = vec![(color5 & 0xFF) as u8, (color5 >> 8) as u8, inten & 0x07, 0];

        // ---- Selector bytes (all-zero = sel 0 for every texel) ----
        let sel_bytes = vec![0u8; 8];

        // ---- Huffman tree: 1 symbol (sym=0, len=1) ----
        // Format: u16 total=1, then {u16 sym=0, u8 len=1}
        let huff_tree: Vec<u8> = vec![1, 0, 0, 0, 1]; // total=1 (LE), sym=0 (LE), len=1

        // Two identical trees (ep + sel).
        let mut tables_bytes: Vec<u8> = huff_tree.clone();
        tables_bytes.extend_from_slice(&huff_tree);

        // ---- SGD header (18 bytes) ----
        let tables_size = tables_bytes.len() as u32;
        let mut sgd: Vec<u8> = Vec::new();
        sgd.extend_from_slice(&1u16.to_le_bytes()); // endpoint_count = 1
        sgd.extend_from_slice(&1u16.to_le_bytes()); // selector_count = 1
        sgd.extend_from_slice(&tables_size.to_le_bytes());
        sgd.extend_from_slice(&0u32.to_le_bytes()); // extended_filesize
        sgd.extend_from_slice(&0u32.to_le_bytes()); // old_header_file_size
        sgd.extend_from_slice(&18u16.to_le_bytes()); // header_size
        sgd.extend_from_slice(&tables_bytes);
        sgd.extend_from_slice(&ep_bytes);
        sgd.extend_from_slice(&sel_bytes);

        // ---- Slice: two symbols (ep_delta=0, sel_delta=0). ----
        // In our 1-symbol tree, sym=0 has code=0 (1 bit, LSB-first).
        // Two bits → 0b00 = byte 0x00.
        let level_data: Vec<u8> = vec![0x00];

        (sgd, level_data)
    }

    #[test]
    fn transcode_1x1_known_color() {
        // R5=31 → R8=255; G5=B5=0 → G8=B8=0; inten=0, sel=0 → modifier=+2.
        // Modifier applies to all three channels: R=clamp(255+2)=255, G=clamp(0+2)=2, B=2.
        let (sgd, level) = make_minimal_sgd_and_slice(31, 0, 0, 0);
        let px = transcode_to_rgba8(&sgd, &level, 1, 1).unwrap();
        assert_eq!(px.len(), 4);
        assert_eq!(px[0], 255, "R");
        assert_eq!(px[1], 2, "G");
        assert_eq!(px[2], 2, "B");
        assert_eq!(px[3], 255, "A");
    }

    #[test]
    fn transcode_zero_dimensions_ok() {
        let result = transcode_to_rgba8(&[0u8; 32], &[], 0, 0);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 0);
    }

    #[test]
    fn transcode_output_length_4x4() {
        // A 4×4 image = exactly 1 block.
        let (sgd, level) = make_minimal_sgd_and_slice(0, 0, 16, 3);
        let px = transcode_to_rgba8(&sgd, &level, 4, 4).unwrap();
        assert_eq!(px.len(), 4 * 4 * 4, "expected 64 bytes for 4×4 RGBA8");
    }

    #[test]
    fn transcode_output_length_non_power_of_two() {
        // 5×3 image: ceil(5/4)=2 blocks wide, ceil(3/4)=1 block tall = 2 blocks.
        // But our minimal SGD only has 1 endpoint+selector; the second block
        // will decode the same colour (ep_delta=0 wraps to same ep).
        let (sgd, mut level) = make_minimal_sgd_and_slice(15, 15, 0, 2);
        // Append another pair of 0-bits for the second block.
        // The existing 0x00 byte already holds 8 bits; that's enough for 4
        // one-bit symbols (2 per block × 2 blocks).
        level.push(0x00);
        let px = transcode_to_rgba8(&sgd, &level, 5, 3).unwrap();
        assert_eq!(px.len(), 5 * 3 * 4, "expected 60 bytes for 5×3 RGBA8");
    }
}
