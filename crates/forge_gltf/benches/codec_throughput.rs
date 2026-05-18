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
use forge_gltf::{GltfAsset, Pose};

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
    let put = |val: u64, n: u32, buf: &mut u64, nbits: &mut u32, out: &mut Vec<u8>| {
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

// ─── Pose sample (animation evaluator hot path) ─────────────────────────────
//
// Pose::sample drives the CPU side of every animated glTF asset every
// frame. The inner cost is dominated by compose_trs (quat→mat4 expansion)
// per animated joint + a recursive world-matrix composition pass. Bench it
// against the largest local skinned asset (BrainStem, 57 joints) so future
// regressions show up. Throughput is published in joints/sec via
// Throughput::Elements.
fn bench_pose_sample(c: &mut Criterion) {
    let mut group = c.benchmark_group("pose");
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/models/BrainStem.glb");
    if !path.exists() {
        eprintln!("bench skipped: {} not found", path.display());
        return;
    }
    let asset = GltfAsset::load(&path).expect("load BrainStem");
    if asset.animations.is_empty() { return; }
    let anim_idx = 0;
    let total_joints = asset.skins.first().map_or(0, |s| s.joints.len());
    let anim_duration = asset.animations[anim_idx].duration().max(1e-3);

    group.throughput(Throughput::Elements(total_joints as u64));
    group.bench_function("sample_brainstem", |b| {
        let mut pose = Pose::rest(&asset);
        let mut t = 0.0f32;
        b.iter(|| {
            t = (t + 0.016) % anim_duration; // ~60 Hz advance
            pose.sample(black_box(&asset), black_box(&asset.animations[anim_idx]), black_box(t));
            black_box(&pose.world);
        });
    });
    group.finish();
}

// ─── BC family (codec/bc) ───────────────────────────────────────────────────
//
// One bench per BC1/2/3/4/5/6H/7 against a 64×64 fixture (256 blocks ×
// {8 or 16}-byte block size). Throughput measured against the
// uncompressed RGBA8 OUTPUT size — what the user ultimately sees.
fn bench_bc(c: &mut Criterion) {
    let mut group = c.benchmark_group("bc");
    let out_bytes = (64u64 * 64 * 4);
    let bc1_input = vec![0xAAu8; 256 * 8];  // 256 blocks × 8 bytes
    let bc7_input = vec![0xAAu8; 256 * 16]; // 256 blocks × 16 bytes
    group.throughput(Throughput::Bytes(out_bytes));
    group.bench_function("bc1_64x64", |b| {
        b.iter(|| black_box(codec::bc::decode_bc1(black_box(&bc1_input), 64, 64)));
    });
    group.bench_function("bc3_64x64", |b| {
        b.iter(|| black_box(codec::bc::decode_bc3(black_box(&bc7_input), 64, 64)));
    });
    group.bench_function("bc4_64x64", |b| {
        b.iter(|| black_box(codec::bc::decode_bc4(black_box(&bc1_input), 64, 64)));
    });
    group.bench_function("bc5_64x64", |b| {
        b.iter(|| black_box(codec::bc::decode_bc5(black_box(&bc7_input), 64, 64)));
    });
    group.bench_function("bc7_64x64", |b| {
        b.iter(|| black_box(codec::bc::decode_bc7(black_box(&bc7_input), 64, 64)));
    });
    group.finish();
}

// ─── ETC2 + EAC family (codec/etc2) ─────────────────────────────────────────
fn bench_etc2(c: &mut Criterion) {
    let mut group = c.benchmark_group("etc2");
    let out_bytes = (64u64 * 64 * 4);
    let rgb_input    = vec![0xAAu8; 256 * 8];  // 256 blocks × 8 bytes
    let rgba8_input  = vec![0xAAu8; 256 * 16];
    let r11_input    = vec![0xAAu8; 256 * 8];
    let r11g11_input = vec![0xAAu8; 256 * 16];
    group.throughput(Throughput::Bytes(out_bytes));
    group.bench_function("etc2_rgb_64x64", |b| {
        b.iter(|| black_box(codec::etc2::decode_etc2_rgb(black_box(&rgb_input), 64, 64)));
    });
    group.bench_function("etc2_rgba8_64x64", |b| {
        b.iter(|| black_box(codec::etc2::decode_etc2_rgba8(black_box(&rgba8_input), 64, 64)));
    });
    group.bench_function("eac_r11_64x64", |b| {
        b.iter(|| black_box(codec::etc2::decode_eac_r11(black_box(&r11_input), 64, 64)));
    });
    group.bench_function("eac_r11g11_64x64", |b| {
        b.iter(|| black_box(codec::etc2::decode_eac_r11g11(black_box(&r11g11_input), 64, 64)));
    });
    group.finish();
}

// ─── ASTC (codec/astc) — 4x4 / 8x8 / 12x12 block sizes ──────────────────────
fn bench_astc(c: &mut Criterion) {
    let mut group = c.benchmark_group("astc");
    let out_bytes = (64u64 * 64 * 4);
    let input = vec![0xAAu8; 256 * 16]; // 256 blocks × 16 bytes
    group.throughput(Throughput::Bytes(out_bytes));
    group.bench_function("astc_4x4_64x64", |b| {
        b.iter(|| black_box(codec::astc::decode_astc(black_box(&input), 64, 64, 4, 4)));
    });
    // 64×64 image with 8×8 blocks = 8×8 = 64 blocks (1024 B input).
    let input_8x8 = vec![0xAAu8; 64 * 16];
    group.bench_function("astc_8x8_64x64", |b| {
        b.iter(|| black_box(codec::astc::decode_astc(black_box(&input_8x8), 64, 64, 8, 8)));
    });
    // 64×64 with 12×12 blocks: ceil(64/12) = 6, so 6×6 = 36 blocks.
    let input_12x12 = vec![0xAAu8; 36 * 16];
    group.bench_function("astc_12x12_64x64", |b| {
        b.iter(|| black_box(codec::astc::decode_astc(black_box(&input_12x12), 64, 64, 12, 12)));
    });
    group.finish();
}

// ─── ZSTD (codec/zstd) ──────────────────────────────────────────────────────
//
// Constructs a synthetic RLE-block ZSTD frame: header + one block of N
// RLE-encoded zero bytes. Measures the decode-side throughput against
// the uncompressed output size.
fn bench_zstd(c: &mut Criterion) {
    let mut group = c.benchmark_group("zstd");
    // Frame: magic + frame header (1 byte: FCS=u32, content size known) +
    //        u32 frame content size + 1 RLE block (3 byte hdr + 1 byte).
    let mut frame = Vec::with_capacity(13);
    frame.extend_from_slice(&[0x28, 0xB5, 0x2F, 0xFD]); // ZSTD magic
    // Frame header descriptor:
    //   FCS_Flag           = 0b10 (4-byte FCS field)
    //   Single_Segment_flag = 1   (no window descriptor; FCS is mandatory)
    //   Unused              = 0
    //   Reserved            = 0
    //   Content_Checksum_Flag = 0
    //   Dictionary_ID_Flag  = 0b00
    // → FHD = 0b10_1_0_0_0_00 = 0xA0.
    frame.push(0xA0);
    // 4 bytes for content size (1 MiB).
    let cs = 1024u64 * 1024;
    frame.extend_from_slice(&(cs as u32).to_le_bytes());
    // Block header: last=1, block_type=01 (RLE), block_size = cs.
    let block_hdr = (1u32) | (0b01 << 1) | ((cs as u32) << 3);
    frame.extend_from_slice(&block_hdr.to_le_bytes()[..3]);
    frame.push(0x00); // single RLE byte

    group.throughput(Throughput::Bytes(cs));
    group.bench_function("decompress_synthetic_1MB", |b| {
        b.iter(|| black_box(codec::zstd::decompress(black_box(&frame))));
    });
    group.finish();
}

// ─── Draco attribute dequantize (codec/draco) ───────────────────────────────
//
// Direct bench of the SSE2 dequantize path against a 64K-vertex stream
// (3 channels — typical POSITION accessor). The scalar reference lives
// behind the same internal entry point and gets exercised by tests.
fn bench_draco_dequantize(c: &mut Criterion) {
    let npoints = 64 * 1024;
    let nc      = 3;
    let total   = npoints * nc;
    let decoded: Vec<i32> = (0..total).map(|i| (i as i32) & 0xFFF).collect();
    let mut out = vec![0.0f32; total];
    let mut group = c.benchmark_group("draco");
    group.throughput(Throughput::Bytes((total * 4) as u64));
    group.bench_function("attr_dequantize_64K_vertices_nc3", |b| {
        b.iter(|| {
            for f in out.iter_mut() { *f = 0.0; }
            // Use the SIMD path directly; nc=3 → 4-lane SSE2 body.
            #[cfg(target_arch = "x86_64")]
            codec::draco::bench_dequantize_helper(
                black_box(&decoded), black_box(&mut out), npoints, nc,
            );
            black_box(&out);
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_uastc,
    bench_meshopt_vertex,
    bench_webp_vp8l,
    bench_ktx2_parse,
    bench_draco_header,
    bench_pose_sample,
    bench_bc,
    bench_etc2,
    bench_astc,
    bench_zstd,
    bench_draco_dequantize,
);
criterion_main!(benches);
