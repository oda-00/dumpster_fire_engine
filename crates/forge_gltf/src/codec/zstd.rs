//! Hand-rolled ZSTD v1 decompressor per RFC 8478.
//!
//! Implements: every frame type that ships in the wild (single-segment +
//! multi-segment), every block type (raw / RLE / compressed), every
//! literals section mode (raw / RLE / 4-stream Huffman / 1-stream Huffman
//! / treeless repeat), every FSE-coded sequence mode (predefined / RLE /
//! compressed / repeat), plus the sequence-application output writer with
//! the spec's three repeat-offset slots and the offset_value-to-actual-
//! offset decoding.
//!
//! Out of scope deliberately: prebuilt-dictionary support
//! (`Frame_Header_Descriptor.Dictionary_ID` ≠ 0 returns an error — that's
//! a separate spec ZSTD_DDict from RFC 8878; KTX2 supercompression uses
//! "raw frames with no dictionary" per the KTX2 spec § 3.10) and the
//! optional XXH64 frame checksum (decode succeeds regardless).
//!
//! The public entry point is `decompress`. For KTX2 levels the input
//! is a single ZSTD frame; this decoder consumes the whole input or
//! returns a clean error.

use crate::error::{GltfError, GltfResult};
use thin_vec::ThinVec;

/// Decompress a single ZSTD frame. Returns the raw bytes.
pub fn decompress(src: &[u8]) -> GltfResult<ThinVec<u8>> {
    let mut r = ByteReader::new(src);

    // Frame magic.
    if r.read_u32_le()? != 0xFD2FB528 {
        // Skippable frame? Magic in [0x184D2A50, 0x184D2A5F] — content
        // is user data; we just skip and return empty.
        let magic = u32::from_le_bytes([src[0], src[1], src[2], src[3]]);
        if (0x184D2A50..=0x184D2A5F).contains(&magic) {
            return Ok(ThinVec::new());
        }
        return Err(GltfError::InvalidAccessor("ZSTD: bad magic"));
    }

    let header = read_frame_header(&mut r)?;
    if header.dictionary_id != 0 {
        return Err(GltfError::UnsupportedFeature(
            format!("ZSTD frame uses dictionary {} (no dict support)", header.dictionary_id)
        ));
    }

    // Pre-size the output when content_size is known. Otherwise allocate
    // optimistically off the window descriptor.
    let initial_capacity = header.frame_content_size
        .map(|s| s as usize)
        .unwrap_or_else(|| header.window_size.min(1 << 24) as usize);
    let mut out: Vec<u8> = Vec::with_capacity(initial_capacity);

    // Per-frame repeat-offset history (the three "Repeat_Offset_1..3" slots
    // each sequence may reference instead of an explicit offset).
    let mut repeats = [1u32, 4, 8];

    // Per-frame previously-built FSE tables (used when a block declares
    // mode = Repeat for one of the sequence streams).
    let mut prev_ll: Option<FseTable> = None;
    let mut prev_ml: Option<FseTable> = None;
    let mut prev_of: Option<FseTable> = None;

    // Per-frame previously-built Huffman tree (literals mode = repeat).
    let mut prev_huff: Option<HuffmanTable> = None;

    loop {
        let bh = read_block_header(&mut r)?;
        match bh.block_type {
            0 => {
                // Raw block — literal bytes.
                let bytes = r.read_bytes(bh.block_size as usize)?;
                out.extend_from_slice(bytes);
            }
            1 => {
                // RLE block — single byte repeated `block_size` times.
                let b = r.read_u8()?;
                out.resize(out.len() + bh.block_size as usize, b);
            }
            2 => {
                // Compressed block: literals + sequences.
                let block_end_pos = r.pos + bh.block_size as usize;
                decompress_compressed_block(
                    &mut r, block_end_pos, &mut out,
                    &mut repeats,
                    &mut prev_ll, &mut prev_ml, &mut prev_of,
                    &mut prev_huff,
                )?;
                if r.pos != block_end_pos {
                    return Err(GltfError::InvalidAccessor(
                        "ZSTD compressed block: trailing bytes after decode"
                    ));
                }
            }
            _ => return Err(GltfError::InvalidAccessor(
                "ZSTD: reserved block type 3"
            )),
        }
        if bh.last_block { break; }
    }

    // Optional 4-byte content checksum (XXH64 low 32 bits) — skipped.
    if header.content_checksum {
        let _ = r.read_u32_le();
    }

    let mut tv: ThinVec<u8> = ThinVec::with_capacity(out.len());
    tv.extend_from_slice(&out);
    Ok(tv)
}

// ─── Frame header ───────────────────────────────────────────────────────────

struct FrameHeader {
    window_size: u64,
    frame_content_size: Option<u64>,
    dictionary_id: u32,
    content_checksum: bool,
}

fn read_frame_header(r: &mut ByteReader<'_>) -> GltfResult<FrameHeader> {
    let fhd = r.read_u8()?;
    let dict_id_flag       =  fhd        & 0x3;
    let content_checksum   = (fhd >> 2) & 0x1 != 0;
    // bit 3 reserved (must be 0).
    let unused_bit3        = (fhd >> 3) & 0x1;
    let single_segment     = (fhd >> 5) & 0x1 != 0;
    let fcs_flag           = (fhd >> 6) & 0x3;
    if unused_bit3 != 0 {
        return Err(GltfError::InvalidAccessor("ZSTD: reserved bit set in frame header descriptor"));
    }

    // Window descriptor (omitted iff single_segment).
    let window_size: u64 = if single_segment {
        // The frame content size IS the window size in this case.
        0 // filled in below from FCS
    } else {
        let wd = r.read_u8()? as u64;
        let exponent = wd >> 3;
        let mantissa = wd & 0x7;
        let window_log = 10 + exponent;
        let window_base = 1u64 << window_log;
        let window_add = (window_base >> 3) * mantissa;
        window_base + window_add
    };

    // Dictionary ID (0, 1, 2, or 4 bytes per dict_id_flag).
    let dictionary_id: u32 = match dict_id_flag {
        0 => 0,
        1 => r.read_u8()? as u32,
        2 => r.read_u16_le()? as u32,
        3 => r.read_u32_le()?,
        _ => unreachable!(),
    };

    // Frame content size (0, 1, 2, 4, or 8 bytes).
    let frame_content_size: Option<u64> = match fcs_flag {
        0 if single_segment => Some(r.read_u8()? as u64),
        0                   => None,
        1                   => Some(r.read_u16_le()? as u64 + 256),
        2                   => Some(r.read_u32_le()? as u64),
        3                   => Some(r.read_u64_le()?),
        _                   => unreachable!(),
    };

    let window_size = if single_segment {
        // Per spec: window_size = content_size for single-segment frames.
        frame_content_size.unwrap_or(0)
    } else {
        window_size
    };

    Ok(FrameHeader { window_size, frame_content_size, dictionary_id, content_checksum })
}

// ─── Block header ───────────────────────────────────────────────────────────

struct BlockHeader {
    last_block: bool,
    block_type: u8,
    block_size: u32,
}

fn read_block_header(r: &mut ByteReader<'_>) -> GltfResult<BlockHeader> {
    let b0 = r.read_u8()? as u32;
    let b1 = r.read_u8()? as u32;
    let b2 = r.read_u8()? as u32;
    let raw = b0 | (b1 << 8) | (b2 << 16);
    let last_block = (raw & 0x1) != 0;
    let block_type = ((raw >> 1) & 0x3) as u8;
    let block_size = raw >> 3;
    Ok(BlockHeader { last_block, block_type, block_size })
}

// ─── Compressed block ───────────────────────────────────────────────────────

fn decompress_compressed_block(
    r:           &mut ByteReader<'_>,
    block_end:   usize,
    out:         &mut Vec<u8>,
    repeats:     &mut [u32; 3],
    prev_ll:     &mut Option<FseTable>,
    prev_ml:     &mut Option<FseTable>,
    prev_of:     &mut Option<FseTable>,
    prev_huff:   &mut Option<HuffmanTable>,
) -> GltfResult<()> {
    // ── Literals section ──
    let lit_header = read_literals_section_header(r)?;
    let literals: Vec<u8> = match lit_header.section_type {
        0 => {
            // Raw literals — `regenerated_size` bytes.
            r.read_bytes(lit_header.regen_size as usize)?.to_vec()
        }
        1 => {
            // RLE literals — single byte repeated.
            let b = r.read_u8()?;
            vec![b; lit_header.regen_size as usize]
        }
        2 | 3 => {
            // Compressed literals (Huffman). `section_type == 2` means a
            // new Huffman table is sent; `== 3` means reuse the previous
            // frame's table ("Treeless").
            let comp_bytes = r.read_bytes(lit_header.comp_size as usize)?;
            if lit_header.section_type == 2 {
                let (table, header_consumed) = HuffmanTable::parse(comp_bytes)?;
                let payload = &comp_bytes[header_consumed..];
                let decoded = huffman_decompress(
                    &table, payload, lit_header.num_streams,
                    lit_header.regen_size as usize,
                )?;
                *prev_huff = Some(table);
                decoded
            } else {
                let table = prev_huff.as_ref().ok_or(GltfError::InvalidAccessor(
                    "ZSTD: treeless literals with no prior Huffman table"
                ))?;
                huffman_decompress(
                    table, comp_bytes, lit_header.num_streams,
                    lit_header.regen_size as usize,
                )?
            }
        }
        _ => unreachable!(),
    };

    // ── Sequences section ──
    let seq_header = read_sequences_section_header(r)?;
    if seq_header.num_sequences == 0 {
        // No sequences — just append the literals verbatim.
        out.extend_from_slice(&literals);
        return Ok(());
    }

    // Build the three FSE tables (literal-length, offset-code, match-length)
    // based on their mode bits.
    let ll_table = build_fse_table(
        r, seq_header.ll_mode, prev_ll, &LL_PREDEFINED_DIST, LL_PREDEFINED_AL,
        "literal_length",
    )?;
    let of_table = build_fse_table(
        r, seq_header.of_mode, prev_of, &OF_PREDEFINED_DIST, OF_PREDEFINED_AL,
        "offset",
    )?;
    let ml_table = build_fse_table(
        r, seq_header.ml_mode, prev_ml, &ML_PREDEFINED_DIST, ML_PREDEFINED_AL,
        "match_length",
    )?;

    // Remember for downstream blocks that pick mode = Repeat.
    *prev_ll = Some(ll_table.clone());
    *prev_of = Some(of_table.clone());
    *prev_ml = Some(ml_table.clone());

    // Decode sequences from the bitstream. The bitstream is BACKWARDS:
    // we read from the end byte (highest address) towards the start, MSB
    // first. The header skip-byte marks the position of the highest bit.
    let bitstream_start = r.pos;
    let bitstream_end = block_end;
    if bitstream_end <= bitstream_start {
        return Err(GltfError::InvalidAccessor("ZSTD sequences: empty bitstream"));
    }
    let bitstream = r.slice_range(bitstream_start, bitstream_end)?;
    r.pos = bitstream_end;
    let mut br = BackwardBitReader::new(bitstream)?;

    // Initial FSE states (read in order: ll, of, ml).
    let mut ll_state = br.read_bits(ll_table.acc_log)? as usize;
    let mut of_state = br.read_bits(of_table.acc_log)? as usize;
    let mut ml_state = br.read_bits(ml_table.acc_log)? as usize;

    // Apply sequences.
    let mut lit_cursor = 0usize;
    for i in 0..seq_header.num_sequences {
        // Decode the three symbols from current states (note: offset is
        // read before its bits, then literal_length, then match_length).
        let ll_sym = ll_table.entries[ll_state].symbol as usize;
        let ml_sym = ml_table.entries[ml_state].symbol as usize;
        let of_sym = of_table.entries[of_state].symbol as usize;

        // Decode their base-value + extra-bits.
        let of_code = of_sym;
        let raw_offset = (1u32 << of_code) + br.read_bits(of_code as u32)? as u32;

        let ll_base = LL_BASE[ll_sym];
        let ll_extra = LL_EXTRA[ll_sym];
        let literal_len = ll_base + br.read_bits(ll_extra as u32)? as u32;

        let ml_base = ML_BASE[ml_sym];
        let ml_extra = ML_EXTRA[ml_sym];
        let match_len = ml_base + br.read_bits(ml_extra as u32)? as u32;

        // Decode the actual match offset using the repeat-offset slots.
        let (actual_offset, new_repeats) = decode_offset(raw_offset, literal_len, *repeats);
        *repeats = new_repeats;

        // Emit `literal_len` bytes of literals then copy `match_len` from
        // (output position - actual_offset).
        if lit_cursor + literal_len as usize > literals.len() {
            return Err(GltfError::InvalidAccessor("ZSTD: literal_length exceeds literals section"));
        }
        out.extend_from_slice(&literals[lit_cursor..lit_cursor + literal_len as usize]);
        lit_cursor += literal_len as usize;

        let copy_from = out.len().checked_sub(actual_offset as usize).ok_or(
            GltfError::InvalidAccessor("ZSTD: match offset before output start")
        )?;
        // Match copy may overlap (offset < match_len) — emit byte-by-byte
        // for correctness in that case. The fast path (no overlap) uses a
        // single copy_within when we know the source range is fully behind
        // the destination.
        if actual_offset as usize >= match_len as usize {
            out.reserve(match_len as usize);
            let src_end = copy_from + match_len as usize;
            let src_slice = out[copy_from..src_end].to_vec();
            out.extend_from_slice(&src_slice);
        } else {
            for j in 0..match_len as usize {
                let b = out[copy_from + j];
                out.push(b);
            }
        }

        // Update FSE states for the next sequence (skip after the last).
        if i + 1 < seq_header.num_sequences {
            let ll_entry = ll_table.entries[ll_state];
            let ml_entry = ml_table.entries[ml_state];
            let of_entry = of_table.entries[of_state];
            let ll_bits = br.read_bits(ll_entry.nb_bits as u32)? as usize;
            let ml_bits = br.read_bits(ml_entry.nb_bits as u32)? as usize;
            let of_bits = br.read_bits(of_entry.nb_bits as u32)? as usize;
            ll_state = ll_entry.base as usize + ll_bits;
            ml_state = ml_entry.base as usize + ml_bits;
            of_state = of_entry.base as usize + of_bits;
        }
    }

    // Trailing literals (after the last sequence's literal_length).
    if lit_cursor < literals.len() {
        out.extend_from_slice(&literals[lit_cursor..]);
    }

    Ok(())
}

// ─── Literals section header ────────────────────────────────────────────────

struct LiteralsSectionHeader {
    section_type:  u8,    // 0=raw, 1=RLE, 2=compressed, 3=treeless
    regen_size:    u32,
    comp_size:     u32,   // ignored for raw/RLE
    num_streams:   u32,   // 1 or 4
}

fn read_literals_section_header(r: &mut ByteReader<'_>) -> GltfResult<LiteralsSectionHeader> {
    let b0 = r.read_u8()?;
    let section_type = b0 & 0x3;
    let size_format  = (b0 >> 2) & 0x3;

    match section_type {
        0 | 1 => {
            // Raw or RLE. 3 size formats: 5b, 12b, 20b regenerated_size.
            let (regen_size, _consumed) = match size_format {
                0 | 2 => ((b0 as u32) >> 3, 1),
                1 => {
                    let b1 = r.read_u8()? as u32;
                    (((b0 as u32) >> 4) | (b1 << 4), 2)
                }
                3 => {
                    let b1 = r.read_u8()? as u32;
                    let b2 = r.read_u8()? as u32;
                    (((b0 as u32) >> 4) | (b1 << 4) | (b2 << 12), 3)
                }
                _ => unreachable!(),
            };
            Ok(LiteralsSectionHeader {
                section_type, regen_size, comp_size: 0, num_streams: 1,
            })
        }
        2 | 3 => {
            // Compressed. 4 size formats varying header length (3 / 4 / 5 bytes).
            // The `num_streams` is 1 when size_format == 0, otherwise 4.
            let num_streams = if size_format == 0 { 1 } else { 4 };
            let (regen_size, comp_size) = match size_format {
                0 | 1 => {
                    // 3-byte header: 10-bit regen + 10-bit comp.
                    let b1 = r.read_u8()? as u32;
                    let b2 = r.read_u8()? as u32;
                    let regen = ((b0 as u32) >> 4) | ((b1 & 0x3f) << 4);
                    let comp  = (b1 >> 6) | (b2 << 2);
                    (regen, comp)
                }
                2 => {
                    // 4-byte header: 14-bit regen + 14-bit comp.
                    let b1 = r.read_u8()? as u32;
                    let b2 = r.read_u8()? as u32;
                    let b3 = r.read_u8()? as u32;
                    let regen = ((b0 as u32) >> 4) | (b1 << 4) | ((b2 & 0x3) << 12);
                    let comp  = (b2 >> 2) | (b3 << 6);
                    (regen, comp)
                }
                3 => {
                    // 5-byte header: 18-bit regen + 18-bit comp.
                    let b1 = r.read_u8()? as u32;
                    let b2 = r.read_u8()? as u32;
                    let b3 = r.read_u8()? as u32;
                    let b4 = r.read_u8()? as u32;
                    let regen = ((b0 as u32) >> 4) | (b1 << 4) | (b2 << 12) | ((b3 & 0x3) << 16);
                    let comp  = (b3 >> 2) | (b4 << 6);
                    (regen, comp)
                }
                _ => unreachable!(),
            };
            Ok(LiteralsSectionHeader {
                section_type, regen_size, comp_size, num_streams,
            })
        }
        _ => unreachable!(),
    }
}

// ─── Sequences section header ───────────────────────────────────────────────

struct SequencesSectionHeader {
    num_sequences: u32,
    ll_mode: u8,
    of_mode: u8,
    ml_mode: u8,
}

fn read_sequences_section_header(r: &mut ByteReader<'_>) -> GltfResult<SequencesSectionHeader> {
    let b0 = r.read_u8()? as u32;
    let num_sequences = if b0 < 128 {
        b0
    } else if b0 < 255 {
        let b1 = r.read_u8()? as u32;
        ((b0 - 128) << 8) + b1
    } else {
        let b1 = r.read_u8()? as u32;
        let b2 = r.read_u8()? as u32;
        b1 + (b2 << 8) + 0x7F00
    };
    if num_sequences == 0 {
        return Ok(SequencesSectionHeader {
            num_sequences: 0, ll_mode: 0, of_mode: 0, ml_mode: 0,
        });
    }
    let modes = r.read_u8()?;
    let ll_mode = (modes >> 6) & 0x3;
    let of_mode = (modes >> 4) & 0x3;
    let ml_mode = (modes >> 2) & 0x3;
    // Bits 0-1 reserved.
    Ok(SequencesSectionHeader { num_sequences, ll_mode, of_mode, ml_mode })
}

// ─── FSE table construction ─────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
struct FseEntry {
    symbol:  u8,
    nb_bits: u8,
    base:    u16,
}

#[derive(Debug, Clone)]
struct FseTable {
    entries: Vec<FseEntry>,
    acc_log: u32,
}

fn build_fse_table(
    r:               &mut ByteReader<'_>,
    mode:            u8,
    prev:            &Option<FseTable>,
    predefined_dist: &[i32],
    predefined_al:   u32,
    name:            &'static str,
) -> GltfResult<FseTable> {
    match mode {
        0 => {
            // Predefined distribution.
            Ok(build_fse_from_distribution(predefined_dist, predefined_al))
        }
        1 => {
            // RLE — single symbol repeated.
            let sym = r.read_u8()?;
            let entry = FseEntry { symbol: sym, nb_bits: 0, base: 0 };
            Ok(FseTable { entries: vec![entry; 1], acc_log: 0 })
        }
        2 => {
            // Compressed (FSE-coded distribution).
            // The next bytes encode `accuracy_log` (4 bits) + variable-length
            // normalised counts; we consume bytes until the table is built.
            let bytes = r.remaining();
            let (table, consumed) = parse_compressed_fse_table(bytes, name)?;
            r.pos += consumed;
            Ok(table)
        }
        3 => {
            // Repeat — reuse the previous block's table.
            prev.clone().ok_or(GltfError::InvalidAccessor(
                "ZSTD: FSE mode Repeat with no previous table"
            ))
        }
        _ => unreachable!(),
    }
}

fn build_fse_from_distribution(dist: &[i32], acc_log: u32) -> FseTable {
    let size = 1usize << acc_log;
    let mut entries = vec![FseEntry { symbol: 0, nb_bits: 0, base: 0 }; size];

    // Spread symbols across the table per the spec's gap-pattern:
    //   step = (size >> 1) + (size >> 3) + 3; mask = size - 1.
    let step = (size >> 1) + (size >> 3) + 3;
    let mask = size - 1;
    let mut pos = 0usize;
    for (sym, &n) in dist.iter().enumerate() {
        if n <= 0 { continue; } // negative = "low-prob symbol" — handled below
        for _ in 0..n {
            entries[pos].symbol = sym as u8;
            pos = (pos + step) & mask;
            while pos >= size.wrapping_sub(0) || entries[pos].symbol != 0 && entries[pos].nb_bits != 0 {
                // Skip cells already used by negative-prob symbols (we
                // placed them up-front; this is a stable placeholder).
                if entries[pos].nb_bits != 0 { pos = (pos + step) & mask; }
                else { break; }
            }
        }
    }
    // Negative-prob symbols (each `low_prob`) land in a reserved tail.
    let mut tail = size - 1;
    for (sym, &n) in dist.iter().enumerate() {
        if n < 0 {
            entries[tail].symbol = sym as u8;
            entries[tail].nb_bits = acc_log as u8;
            entries[tail].base = 0;
            if tail > 0 { tail -= 1; }
        }
    }

    // Compute nb_bits + base for each cell per the FSE spec.
    let mut next_state = vec![0u16; dist.len()];
    for sym in 0..dist.len() {
        let n = dist[sym];
        next_state[sym] = if n > 0 { n as u16 } else { 1 };
    }
    for entry in entries.iter_mut() {
        if entry.nb_bits != 0 { continue; } // already filled by low-prob branch
        let sym = entry.symbol as usize;
        if sym >= next_state.len() { continue; }
        let nx = next_state[sym] as u32;
        if nx == 0 { continue; }
        let n_bits = (acc_log) - log2_floor(nx);
        entry.nb_bits = n_bits as u8;
        entry.base = (((nx as u32) << n_bits) - size as u32) as u16;
        next_state[sym] += 1;
    }

    FseTable { entries, acc_log }
}

fn log2_floor(x: u32) -> u32 {
    if x == 0 { 0 } else { 31 - x.leading_zeros() }
}

fn parse_compressed_fse_table(bytes: &[u8], _name: &'static str)
    -> GltfResult<(FseTable, usize)>
{
    if bytes.is_empty() {
        return Err(GltfError::InvalidAccessor("ZSTD FSE table: empty"));
    }
    let mut br = ForwardBitReader::new(bytes);
    let acc_log = br.read_bits(4)? as u32 + 5;
    let size = 1u32 << acc_log;
    let mut remaining = size as i32 + 1;
    let mut dist: Vec<i32> = Vec::new();
    while remaining > 1 {
        // Number of bits to read = log2(remaining) + 1.
        let max_n_bits = log2_floor(remaining as u32) + 1;
        let threshold  = (1u32 << max_n_bits) - 1 - remaining as u32;
        let mut value = br.read_bits(max_n_bits - 1)? as u32;
        if value < threshold {
            // Use only max_n_bits-1 bits.
        } else {
            // Need one more bit.
            let extra = br.read_bits(1)? as u32;
            value = (value << 1) | extra;
            if value >= (1u32 << max_n_bits) - 1 {
                value -= threshold;
            }
        }
        let count = value as i32 - 1;
        dist.push(count);
        remaining -= count.abs();
        if count == 0 {
            // Repeat marker — read 2 bits at a time until non-3.
            loop {
                let r2 = br.read_bits(2)? as i32;
                for _ in 0..r2 { dist.push(0); }
                if r2 != 3 { break; }
            }
        }
    }
    let consumed = (br.bits_consumed + 7) / 8;
    Ok((build_fse_from_distribution(&dist, acc_log), consumed))
}

// ─── Offset code → real offset ──────────────────────────────────────────────

/// Decode the raw offset value coming from a sequence's offset_code +
/// extra bits into an actual byte distance into the output stream,
/// updating the rolling 3-slot repeat-offset cache per the spec.
fn decode_offset(raw: u32, literal_length: u32, mut rep: [u32; 3]) -> (u32, [u32; 3]) {
    let actual = if raw > 3 {
        // New offset — value is (raw - 3).
        let off = raw - 3;
        rep = [off, rep[0], rep[1]];
        off
    } else {
        // Repeated offset slot (1, 2, or 3) — with the literal-length-zero
        // special case shifting the indexes.
        let idx = (raw as usize).saturating_sub(1);
        let actual = if literal_length == 0 {
            // L==0: indices map to rep[1], rep[2], rep[0]-1.
            match raw {
                1 => rep[1],
                2 => rep[2],
                3 => rep[0].saturating_sub(1),
                _ => rep[0],
            }
        } else {
            rep[idx.min(2)]
        };
        // Promote the used slot to most-recently-used.
        if raw != 1 {
            // Swap actual into rep[0] and shuffle the rest.
            let mut new = [0u32; 3];
            new[0] = actual;
            let mut w = 1;
            for &v in rep.iter() {
                if v == actual { continue; }
                if w >= 3 { break; }
                new[w] = v;
                w += 1;
            }
            rep = new;
        }
        actual
    };
    (actual.max(1), rep)
}

// ─── Huffman decoder ────────────────────────────────────────────────────────

/// Canonical Huffman table with a lookup tree built for ZSTD literals.
struct HuffmanTable {
    /// max_code_len. Maximum code length in bits (usually ≤ 11 in ZSTD).
    max_len: u8,
    /// Direct-lookup table: index by `max_len`-bit MSB-aligned code; entry
    /// gives the symbol and the actual code length so the reader knows
    /// how many bits to consume.
    table: Vec<(u8, u8)>, // (symbol, length)
}

impl HuffmanTable {
    fn parse(src: &[u8]) -> GltfResult<(Self, usize)> {
        if src.is_empty() {
            return Err(GltfError::InvalidAccessor("ZSTD Huffman: empty header"));
        }
        let header_byte = src[0];
        let (weights, consumed) = if header_byte < 128 {
            // FSE-compressed weights — header_byte bytes of FSE table follow.
            let weights_size = header_byte as usize;
            if 1 + weights_size > src.len() {
                return Err(GltfError::InvalidAccessor("ZSTD Huffman: weights body truncated"));
            }
            // For our use case we only need to handle the direct path; FSE
            // weights are rare in KTX2 supercompression. Decode anyway by
            // falling through to direct interpretation.
            let body = &src[1..1 + weights_size];
            // Build FSE table over weight symbols, then decode the weight
            // sequence. Each weight is 0..=11.
            let (fse, fse_consumed) = parse_compressed_fse_table(body, "huff_weights")?;
            let stream = &body[fse_consumed..];
            let weights = huffman_decode_weight_stream(&fse, stream)?;
            (weights, 1 + weights_size)
        } else {
            // Direct weight encoding: `header_byte - 127` weights packed
            // as 4 bits each in the following bytes.
            let num_weights = (header_byte - 127) as usize;
            let bytes_needed = (num_weights + 1) / 2;
            if 1 + bytes_needed > src.len() {
                return Err(GltfError::InvalidAccessor("ZSTD Huffman: direct weights truncated"));
            }
            let mut weights = Vec::with_capacity(num_weights);
            for i in 0..num_weights {
                let b = src[1 + (i / 2)];
                let w = if i % 2 == 0 { b >> 4 } else { b & 0x0f };
                weights.push(w);
            }
            (weights, 1 + bytes_needed)
        };

        // Per spec: the highest-symbol weight is implicit — the missing
        // weight is whatever makes the total power-of-two complete. Walk
        // the weights to determine the last symbol's weight and the
        // maximum code length.
        let mut sum: u32 = 0;
        for &w in &weights {
            if w > 0 { sum += 1u32 << (w - 1); }
        }
        if sum == 0 {
            return Err(GltfError::InvalidAccessor("ZSTD Huffman: all-zero weights"));
        }
        let max_len = log2_floor(sum) + 1;
        let next_power = 1u32 << max_len;
        let last_weight_pow = next_power - sum;
        let last_weight = log2_floor(last_weight_pow) + 1;
        let mut all_weights = weights;
        all_weights.push(last_weight as u8);

        // Build the canonical decode table.
        let mut symbols_per_len = vec![0u32; max_len as usize + 1];
        for &w in &all_weights {
            if w > 0 {
                let len = (max_len + 1 - w as u32) as usize;
                symbols_per_len[len] += 1;
            }
        }
        let mut rank_offset = vec![0u32; max_len as usize + 2];
        for l in 1..=max_len as usize {
            rank_offset[l + 1] = rank_offset[l] + symbols_per_len[l] * (1 << (max_len as usize - l));
        }
        let mut table = vec![(0u8, 0u8); 1 << max_len];
        let mut rank_cursor = vec![0u32; max_len as usize + 1];
        for (sym, &w) in all_weights.iter().enumerate() {
            if w == 0 { continue; }
            let len = (max_len + 1 - w as u32) as usize;
            let base = rank_offset[len] + rank_cursor[len] * (1 << (max_len as usize - len));
            let span = 1 << (max_len as usize - len);
            for j in 0..span {
                let idx = base as usize + j;
                if idx < table.len() {
                    table[idx] = (sym as u8, len as u8);
                }
            }
            rank_cursor[len] += 1;
        }

        Ok((HuffmanTable { max_len: max_len as u8, table }, consumed))
    }
}

fn huffman_decode_weight_stream(fse: &FseTable, src: &[u8]) -> GltfResult<Vec<u8>> {
    if src.is_empty() {
        return Ok(Vec::new());
    }
    let mut br = BackwardBitReader::new(src)?;
    let mut s1 = br.read_bits(fse.acc_log)? as usize;
    let mut s2 = br.read_bits(fse.acc_log)? as usize;
    let mut out = Vec::new();
    loop {
        let e1 = fse.entries[s1];
        out.push(e1.symbol);
        if br.is_exhausted() { break; }
        let nb1 = br.read_bits(e1.nb_bits as u32)? as usize;
        s1 = e1.base as usize + nb1;

        let e2 = fse.entries[s2];
        out.push(e2.symbol);
        if br.is_exhausted() { break; }
        let nb2 = br.read_bits(e2.nb_bits as u32)? as usize;
        s2 = e2.base as usize + nb2;
    }
    Ok(out)
}

fn huffman_decompress(
    table: &HuffmanTable, src: &[u8], num_streams: u32, regen_size: usize,
) -> GltfResult<Vec<u8>> {
    let mut out = Vec::with_capacity(regen_size);
    if num_streams == 1 {
        huffman_decode_stream(table, src, regen_size, &mut out)?;
    } else {
        // 4-stream layout: 6-byte jump table holds stream 1/2/3 sizes;
        // stream 4 occupies the rest.
        if src.len() < 6 {
            return Err(GltfError::InvalidAccessor("ZSTD 4-stream Huffman: header truncated"));
        }
        let s1 = u16::from_le_bytes([src[0], src[1]]) as usize;
        let s2 = u16::from_le_bytes([src[2], src[3]]) as usize;
        let s3 = u16::from_le_bytes([src[4], src[5]]) as usize;
        let rest = &src[6..];
        if s1 + s2 + s3 > rest.len() {
            return Err(GltfError::InvalidAccessor("ZSTD 4-stream Huffman: sizes overflow"));
        }
        let s4 = rest.len() - s1 - s2 - s3;
        // Each of 4 streams produces ~1/4 of the literals.
        let q  = regen_size / 4;
        let last_q = regen_size - 3 * q;
        let mut buf = Vec::with_capacity(q);
        huffman_decode_stream(table, &rest[0..s1], q, &mut buf)?;
        out.append(&mut buf);
        buf.clear();
        huffman_decode_stream(table, &rest[s1..s1 + s2], q, &mut buf)?;
        out.append(&mut buf);
        buf.clear();
        huffman_decode_stream(table, &rest[s1 + s2..s1 + s2 + s3], q, &mut buf)?;
        out.append(&mut buf);
        buf.clear();
        huffman_decode_stream(table, &rest[s1 + s2 + s3..s1 + s2 + s3 + s4], last_q, &mut buf)?;
        out.append(&mut buf);
    }
    Ok(out)
}

fn huffman_decode_stream(
    table: &HuffmanTable, src: &[u8], n: usize, out: &mut Vec<u8>,
) -> GltfResult<()> {
    let mut br = BackwardBitReader::new(src)?;
    for _ in 0..n {
        // Peek max_len bits MSB-first, look up symbol, consume actual length.
        let peek = br.peek_bits(table.max_len as u32);
        let (sym, len) = table.table[peek as usize];
        if len == 0 {
            return Err(GltfError::InvalidAccessor("ZSTD Huffman: invalid code"));
        }
        br.consume_bits(len as u32);
        out.push(sym);
        if br.is_exhausted() && out.len() < n {
            // Stream ran out before producing all expected symbols.
            return Err(GltfError::InvalidAccessor("ZSTD Huffman: stream underflow"));
        }
    }
    Ok(())
}

// ─── Bit readers ────────────────────────────────────────────────────────────

/// Forward LSB-first bit reader. Used by FSE table parsing where the
/// distribution is encoded LSB-first.
struct ForwardBitReader<'a> {
    bytes: &'a [u8],
    bits_consumed: usize,
}

impl<'a> ForwardBitReader<'a> {
    fn new(bytes: &'a [u8]) -> Self { Self { bytes, bits_consumed: 0 } }

    fn read_bits(&mut self, n: u32) -> GltfResult<u64> {
        if n == 0 { return Ok(0); }
        if self.bits_consumed + n as usize > self.bytes.len() * 8 {
            return Err(GltfError::InvalidAccessor("ZSTD ForwardBitReader: underflow"));
        }
        let mut v = 0u64;
        for i in 0..n {
            let pos = self.bits_consumed + i as usize;
            let bit = (self.bytes[pos / 8] >> (pos % 8)) & 1;
            v |= (bit as u64) << i;
        }
        self.bits_consumed += n as usize;
        Ok(v)
    }
}

/// Backward MSB-first bit reader. ZSTD's compressed bitstreams (sequences
/// + Huffman literals) are read from end-to-start, with the highest bit
/// of the last byte being the first bit consumed. A marker bit on the
/// last byte tells us where the actual data starts.
struct BackwardBitReader<'a> {
    bytes:      &'a [u8],
    /// Next bit position (counting from the *end* of the buffer in bits).
    bit_pos: i64,
}

impl<'a> BackwardBitReader<'a> {
    fn new(bytes: &'a [u8]) -> GltfResult<Self> {
        if bytes.is_empty() {
            return Err(GltfError::InvalidAccessor("ZSTD BackwardBitReader: empty"));
        }
        // Find the marker bit — the highest set bit in the last byte
        // marks the position of the stream's most-significant bit.
        let last = *bytes.last().unwrap();
        if last == 0 {
            return Err(GltfError::InvalidAccessor("ZSTD BackwardBitReader: no marker bit"));
        }
        let marker = 7i64 - last.leading_zeros() as i64;
        let total_bits = (bytes.len() as i64) * 8 - (8 - marker - 1);
        Ok(Self { bytes, bit_pos: total_bits - 1 })
    }

    /// Read `n` bits MSB-first.
    fn read_bits(&mut self, n: u32) -> GltfResult<u64> {
        if n == 0 { return Ok(0); }
        if self.bit_pos < (n as i64 - 1) {
            return Err(GltfError::InvalidAccessor("ZSTD BackwardBitReader: underflow"));
        }
        let mut v = 0u64;
        for _ in 0..n {
            let bit_idx = self.bit_pos;
            let byte = self.bytes[(bit_idx / 8) as usize];
            let bit = (byte >> (bit_idx % 8)) & 1;
            v = (v << 1) | bit as u64;
            self.bit_pos -= 1;
        }
        Ok(v)
    }

    fn peek_bits(&self, n: u32) -> u64 {
        if n == 0 { return 0; }
        let mut v = 0u64;
        for i in 0..n {
            let bit_idx = self.bit_pos - i as i64;
            if bit_idx < 0 { v <<= 1; continue; }
            let byte = self.bytes[(bit_idx / 8) as usize];
            let bit = (byte >> (bit_idx % 8)) & 1;
            v = (v << 1) | bit as u64;
        }
        v
    }

    fn consume_bits(&mut self, n: u32) {
        self.bit_pos -= n as i64;
    }

    fn is_exhausted(&self) -> bool {
        self.bit_pos < 0
    }
}

// ─── Byte reader ────────────────────────────────────────────────────────────

struct ByteReader<'a> {
    bytes: &'a [u8],
    pos:   usize,
}

impl<'a> ByteReader<'a> {
    fn new(bytes: &'a [u8]) -> Self { Self { bytes, pos: 0 } }
    fn read_u8(&mut self) -> GltfResult<u8> {
        let b = *self.bytes.get(self.pos).ok_or(GltfError::InvalidAccessor("ZSTD: read past end"))?;
        self.pos += 1;
        Ok(b)
    }
    fn read_u16_le(&mut self) -> GltfResult<u16> {
        let bs = self.read_bytes(2)?;
        Ok(u16::from_le_bytes([bs[0], bs[1]]))
    }
    fn read_u32_le(&mut self) -> GltfResult<u32> {
        let bs = self.read_bytes(4)?;
        Ok(u32::from_le_bytes([bs[0], bs[1], bs[2], bs[3]]))
    }
    fn read_u64_le(&mut self) -> GltfResult<u64> {
        let bs = self.read_bytes(8)?;
        Ok(u64::from_le_bytes([bs[0], bs[1], bs[2], bs[3], bs[4], bs[5], bs[6], bs[7]]))
    }
    fn read_bytes(&mut self, n: usize) -> GltfResult<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or(GltfError::InvalidAccessor("ZSTD: read overflow"))?;
        if end > self.bytes.len() {
            return Err(GltfError::InvalidAccessor("ZSTD: read past end"));
        }
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }
    fn remaining(&self) -> &'a [u8] { &self.bytes[self.pos..] }
    fn slice_range(&self, lo: usize, hi: usize) -> GltfResult<&'a [u8]> {
        self.bytes.get(lo..hi).ok_or(GltfError::InvalidAccessor("ZSTD: bad slice"))
    }
}

// ─── Spec tables ────────────────────────────────────────────────────────────

// Literal-length base / extra-bits per code.
const LL_BASE: [u32; 36] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
    16, 18, 20, 22, 24, 28, 32, 40, 48, 64, 128, 256, 512, 1024, 2048,
    4096, 8192, 16384, 32768, 65536,
];
const LL_EXTRA: [u8; 36] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    1, 1, 1, 1, 2, 2, 3, 3, 4, 6, 7, 8, 9, 10, 11,
    12, 13, 14, 15, 16,
];

// Match-length base / extra-bits per code.
const ML_BASE: [u32; 53] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18,
    19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34,
    35, 37, 39, 41, 43, 47, 51, 59, 67, 83, 99, 131, 259, 515, 1027, 2051,
    4099, 8195, 16387, 32771, 65539,
];
const ML_EXTRA: [u8; 53] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    1, 1, 1, 1, 2, 2, 3, 3, 4, 4, 5, 7, 8, 9, 10, 11,
    12, 13, 14, 15, 16,
];

// Predefined distributions + accuracy logs (RFC 8478 Appendix A).
const LL_PREDEFINED_AL: u32 = 6;
const LL_PREDEFINED_DIST: [i32; 36] = [
    4, 3, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1,
    2, 2, 2, 2, 2, 2, 2, 2, 2, 3, 2, 1, 1, 1, 1, 1,
    -1, -1, -1, -1,
];

const ML_PREDEFINED_AL: u32 = 6;
const ML_PREDEFINED_DIST: [i32; 53] = [
    1, 4, 3, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    -1, -1, -1, -1, -1,
];

const OF_PREDEFINED_AL: u32 = 5;
const OF_PREDEFINED_DIST: [i32; 29] = [
    1, 1, 1, 1, 1, 1, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, -1, -1, -1, -1, -1,
];

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bad_magic_returns_error() {
        let data = b"NOTZSTD\0\0\0\0\0\0\0\0";
        assert!(decompress(data).is_err());
    }

    #[test]
    fn empty_skippable_frame_decodes_empty() {
        // Skippable frame magic 0x184D2A50, then 4-byte length = 0, then no payload.
        let mut data = vec![0x50, 0x2A, 0x4D, 0x18, 0, 0, 0, 0];
        // Pad to enough bytes for read attempts in decompress.
        data.extend_from_slice(&[0; 16]);
        let out = decompress(&data).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn raw_block_round_trips() {
        // Frame magic.
        let mut data = vec![0x28, 0xB5, 0x2F, 0xFD];
        // Frame header descriptor: single_segment=1, fcs_flag=0
        //   (single_segment + fcs=0 ⇒ 1-byte FCS follows).
        data.push(0b001_00000 | (0 << 6) | (1 << 5));
        // FCS = 5 (we'll emit 5 raw bytes).
        data.push(5);
        // Block header: 3 bytes — last_block=1, type=0 (raw), size=5.
        let bh: u32 = 1 | (0 << 1) | (5 << 3);
        data.push((bh & 0xff) as u8);
        data.push(((bh >> 8) & 0xff) as u8);
        data.push(((bh >> 16) & 0xff) as u8);
        // Raw payload.
        data.extend_from_slice(b"hello");

        let out = decompress(&data).unwrap();
        assert_eq!(&out[..], b"hello");
    }

    #[test]
    fn rle_block_expands_to_n_repeats() {
        // Frame magic.
        let mut data = vec![0x28, 0xB5, 0x2F, 0xFD];
        // single_segment + fcs=0 ⇒ 1-byte FCS follows.
        data.push(0b001_00000 | (0 << 6) | (1 << 5));
        data.push(7); // produce 7 bytes
        // Block header: last_block=1, type=1 (RLE), size=7.
        let bh: u32 = 1 | (1 << 1) | (7 << 3);
        data.push((bh & 0xff) as u8);
        data.push(((bh >> 8) & 0xff) as u8);
        data.push(((bh >> 16) & 0xff) as u8);
        // Single RLE byte.
        data.push(b'x');

        let out = decompress(&data).unwrap();
        assert_eq!(&out[..], b"xxxxxxx");
    }
}
