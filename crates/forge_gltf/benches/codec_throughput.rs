//! Throughput benchmarks for the hand-rolled codecs in `forge_gltf::codec`.
//!
//! Each benchmark feeds a self-contained synthetic payload — no external
//! reference files needed — so the bench runs anywhere `cargo bench` does.
//! Use `cargo bench -p forge_gltf --bench codec_throughput` to run.
//!
//! These exist as the "hand-roll first" half of the policy in the design
//! plan. Once the hand-rolled paths land, drop in `meshopt-rs`,
//! `basis-universal`, `image` (webp backend) and `draco-oxide` as extra
//! dev-deps and add side-by-side bench groups to validate that the
//! hand-rolled implementations win (or replace them if they don't).

use std::hint::black_box;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};

use forge_gltf::codec;

// ─── UASTC (basisu_uastc) ───────────────────────────────────────────────────
//
// Each 4×4 block is 16 bytes. Mode 8 (the simplest single-plane mode) over
// a 32×32 grid of blocks = 4 KiB of compressed data → 16384 RGBA8 pixels.
fn bench_uastc(c: &mut Criterion) {
    let mut group = c.benchmark_group("uastc");
    for &(bw, bh, name) in &[
        (32usize, 32usize, "transcode_32x32_blocks_128x128"),
        (128, 128, "transcode_128x128_blocks_512x512"),
    ] {
        let blocks = synth_uastc_blocks(bw, bh);
        // Throughput = decoded RGBA bytes (the useful output).
        let out_bytes = ((bw * 4) * (bh * 4) * 4) as u64;
        group.throughput(Throughput::Bytes(out_bytes));
        group.bench_function(name, |b| {
            b.iter(|| {
                let rgba = codec::basisu_uastc::transcode_to_rgba8(
                    black_box(&blocks), (bw * 4) as u32, (bh * 4) as u32);
                black_box(rgba);
            });
        });
    }
    group.finish();
}

// ─── meshopt vertex codec ──────────────────────────────────────────────────
//
// Synthetic 1024-vertex × 16-byte stream encoded in mode 2 (literal raw
// bytes) with all-zero deltas — the simplest decodable shape, exercises the
// group/header machinery without bias from a specific data distribution.
fn bench_meshopt_vertex(c: &mut Criterion) {
    let mut group = c.benchmark_group("meshopt_vertex");
    for &(count, stride, name) in &[
        (1024usize, 16usize, "decompress_1024x16"),
        (16384, 32, "decompress_16384x32"),
    ] {
        let src = synth_meshopt_vertex_payload(count, stride);
        // Throughput = decompressed output bytes (what the caller cares about).
        let out_bytes = (count * stride) as u64;
        group.throughput(Throughput::Bytes(out_bytes));
        group.bench_function(name, |b| {
            b.iter(|| {
                let out = codec::meshopt::decompress_buffer_view(
                    codec::meshopt::MeshoptMode::Attributes,
                    codec::meshopt::MeshoptFilter::None,
                    count, stride,
                    black_box(&src),
                ).expect("decode");
                black_box(out);
            });
        });
    }
    group.finish();
}

// ─── WebP VP8L ─────────────────────────────────────────────────────────────
//
// 2×2 black image encoded in VP8L lossless — the smallest legal payload.
// Throughput here is decoder-overhead dominated; useful as a regression
// signal more than an absolute MP/s number.
fn bench_webp_vp8l(c: &mut Criterion) {
    let mut group = c.benchmark_group("webp_vp8l");
    let riff = synth_vp8l_2x2_black();
    group.throughput(Throughput::Bytes(riff.len() as u64));
    group.bench_function("decode_to_rgba8_2x2", |b| {
        b.iter(|| {
            let out = codec::webp::decode_to_rgba8(black_box(&riff)).expect("decode");
            black_box(out);
        });
    });
    group.finish();
}

// ─── KTX2 container parse ──────────────────────────────────────────────────
//
// Tiny synthetic header-only KTX2 (no payload, no DFD content) — measures
// parser/loop overhead. Useful sanity check that the parser stays small and
// allocation-free.
fn bench_ktx2_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("ktx2");
    let bytes = synth_ktx2_minimal();
    group.throughput(Throughput::Bytes(bytes.len() as u64));
    group.bench_function("parse_minimal_header", |b| {
        b.iter(|| {
            // Parse may fail on the synthetic blob if the parser cross-checks
            // section pointers; we only care about not crashing here.
            let r = codec::ktx2::Ktx2::parse(black_box(&bytes));
            let _ = black_box(r);
        });
    });
    group.finish();
}

// ─── Draco minimal header ──────────────────────────────────────────────────
//
// Synthetic "DRACO" magic + 0-length payload. The decoder returns early but
// the bench still measures the magic check + header walk, which is the hot
// path for "many small primitives" scenes.
fn bench_draco_header(c: &mut Criterion) {
    let mut group = c.benchmark_group("draco");
    let bytes = synth_draco_header_only();
    group.throughput(Throughput::Bytes(bytes.len() as u64));
    group.bench_function("decode_header_only", |b| {
        b.iter(|| {
            // We only care about consistent dispatch overhead — failure is fine.
            let _ = black_box(codec::draco::decode(black_box(&bytes)));
        });
    });
    group.finish();
}

// ─── Synthetic-payload helpers ──────────────────────────────────────────────

/// Build a flat block of UASTC mode-8 blocks (16 bytes each, packing all
/// zeros — decodes to a uniform colour). Output shape: `bw * bh * 16` bytes.
fn synth_uastc_blocks(bw: usize, bh: usize) -> Vec<u8> {
    vec![0u8; bw * bh * 16]
}

/// Build a meshopt mode-2 (literal) vertex stream for `count` vertices of
/// `stride` bytes each. Header byte 0xAA selects mode 2 for all 4 byte
/// positions in a 4-byte stride; for wider strides, repeat the mode byte
/// per 4-byte group as the codec expects.
fn synth_meshopt_vertex_payload(count: usize, stride: usize) -> Vec<u8> {
    let mut src = vec![0xa0u8]; // VERTEX_MAGIC
    // For each group of 16 vertices, write mode bytes for each byte position
    // and a fixed 16-byte literal block per position.
    let groups = count.div_ceil(16);
    for _ in 0..groups {
        // One mode byte per 4 byte-positions (covers stride = 4); for wider
        // strides the codec walks stride bytes total, with 4 positions encoded
        // per mode byte.
        let mode_bytes = (stride + 3) / 4;
        for _ in 0..mode_bytes { src.push(0xAA); } // mode 2 for all positions
        // 16 raw bytes per byte position.
        for _ in 0..stride { src.extend_from_slice(&[0u8; 16]); }
    }
    src
}

/// VP8L 2×2 all-zero pixel payload — adapted from the unit test in
/// `codec::webp::tests`.
fn synth_vp8l_2x2_black() -> Vec<u8> {
    let mut buf: u64 = 0;
    let mut nbits: u32 = 0;
    let mut bits: Vec<u8> = Vec::new();
    let mut put = |val: u64, n: u32, buf: &mut u64, nbits: &mut u32, out: &mut Vec<u8>| {
        *buf |= (val & ((1u64 << n) - 1)) << *nbits;
        *nbits += n;
        while *nbits >= 8 {
            out.push((*buf & 0xff) as u8);
            *buf >>= 8;
            *nbits -= 8;
        }
    };
    put(1, 14, &mut buf, &mut nbits, &mut bits);  // width-1
    put(1, 14, &mut buf, &mut nbits, &mut bits);  // height-1
    put(0, 1,  &mut buf, &mut nbits, &mut bits);  // alpha
    put(0, 3,  &mut buf, &mut nbits, &mut bits);  // version
    put(0, 1,  &mut buf, &mut nbits, &mut bits);  // no transforms
    put(0, 1,  &mut buf, &mut nbits, &mut bits);  // no meta huffman
    for _ in 0..5 {
        put(1, 1, &mut buf, &mut nbits, &mut bits); // simple
        put(0, 1, &mut buf, &mut nbits, &mut bits); // 1 symbol
        put(0, 1, &mut buf, &mut nbits, &mut bits); // sym_bits=0 → 1 bit symbol
        put(0, 1, &mut buf, &mut nbits, &mut bits); // sym = 0
    }
    if nbits > 0 { bits.push((buf & 0xff) as u8); }

    let mut vp8l_data = vec![0x2fu8];
    vp8l_data.extend_from_slice(&bits);

    let vp8l_size = vp8l_data.len() as u32;
    let chunk_total = 8 + vp8l_size;
    let riff_size = 4 + chunk_total;

    let mut riff = Vec::new();
    riff.extend_from_slice(b"RIFF");
    riff.extend_from_slice(&riff_size.to_le_bytes());
    riff.extend_from_slice(b"WEBP");
    riff.extend_from_slice(b"VP8L");
    riff.extend_from_slice(&vp8l_size.to_le_bytes());
    riff.extend_from_slice(&vp8l_data);
    riff
}

/// Minimal KTX2 file — magic + 80-byte header (mostly zeros). The parser
/// validates the magic and reads header fields; deeper sections (level
/// index, DFD, KVD, SGD) come back empty and parsing may legitimately fail
/// on the synthetic file. The bench cares about the dispatch cost only.
fn synth_ktx2_minimal() -> Vec<u8> {
    let magic: [u8; 12] = [0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A];
    let mut bytes = Vec::with_capacity(12 + 17 * 4);
    bytes.extend_from_slice(&magic);
    // 17 little-endian u32 header fields (vkFormat, typeSize, w/h/d, layer/face/level counts,
    // supercompression, then offset/length triples for DFD, KVD, SGD).
    for _ in 0..17 { bytes.extend_from_slice(&0u32.to_le_bytes()); }
    bytes
}

/// Minimal "DRACO" header — version 2.2 (mesh) + sequential encoder.
fn synth_draco_header_only() -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(b"DRACO");
    b.push(2);   // major version
    b.push(2);   // minor version
    b.push(1);   // encoder type (mesh)
    b.push(0);   // encoder method (sequential)
    b.extend_from_slice(&0u16.to_le_bytes()); // flags
    b
}

criterion_group!(
    benches,
    bench_uastc,
    bench_meshopt_vertex,
    bench_webp_vp8l,
    bench_ktx2_parse,
    bench_draco_header,
);
criterion_main!(benches);
