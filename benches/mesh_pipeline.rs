// Benchmarks for the mesh pipeline:
//   1. glTF parsing throughput  (pure CPU — gltf crate + vertex assembly)
//   2. GpuMesh upload throughput (CPU + GPU DMA — staging → device-local)
//   3. draw-call collection      (what draw_frame pays per frame)
//
//   cargo bench --bench mesh_pipeline

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use thin_vec::ThinVec;

use dumpster_fire_engine::forge_master::ore::{ForgeVertex, GpuMesh, MeshOre};
use dumpster_fire_engine::forge_master::{FrameId, GraphicsFramePlan, GraphicsOreKind};
use dumpster_fire_engine::render::factory_master::{Factory, FactoryId, GraphicsTag, Proto, ProtoId};
use dumpster_fire_engine::render::VulkanContext;
use dumpster_fire_engine::resource_manager::asset_manager::{
    build_test_glb, load_first_mesh_from_slice,
};

// ── Helpers ────────────────────────────────────────────────────────────────

/// Build a `MeshOre` with `tri_count` triangles (no shared vertices).
fn make_mesh(tri_count: usize) -> MeshOre {
    let n = tri_count * 3;
    let vertices: ThinVec<ForgeVertex> = (0..n)
        .map(|i| {
            let f = i as f32;
            ForgeVertex::new([f, f + 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0, 1.0], [0.0, 0.0])
        })
        .collect();
    let indices: ThinVec<u32> = (0..n as u32).collect();
    MeshOre::new(vertices, indices)
}

/// Build a GLB blob from `tri_count` triangles (for the parsing benchmarks).
fn make_glb(tri_count: usize) -> Vec<u8> {
    let n = tri_count * 3;
    let positions: Vec<[f32; 3]> = (0..n).map(|i| {
        let f = i as f32;
        [f, f + 1.0, 0.0]
    }).collect();
    let indices: Vec<u32> = (0..n as u32).collect();
    build_test_glb(&positions, None, None, Some(&indices))
}

// ── 1. glTF parsing throughput ─────────────────────────────────────────────

fn bench_gltf_parse(c: &mut Criterion) {
    let mut g = c.benchmark_group("gltf_parse");

    for &tris in &[1usize, 64, 1_024, 16_384] {
        let glb = make_glb(tris);
        let vertex_count = (tris * 3) as u64;
        g.throughput(Throughput::Elements(vertex_count));
        g.bench_with_input(
            BenchmarkId::new("triangles", tris),
            &glb,
            |b, glb| {
                b.iter(|| {
                    let ore = load_first_mesh_from_slice(black_box(glb)).unwrap();
                    black_box(ore);
                });
            },
        );
    }
    g.finish();
}

// ── 2. MeshOre construction (vertex assembly without I/O) ──────────────────

fn bench_mesh_ore_build(c: &mut Criterion) {
    let mut g = c.benchmark_group("mesh_ore_build");

    for &tris in &[64usize, 1_024, 16_384, 131_072] {
        let vertex_count = (tris * 3) as u64;
        g.throughput(Throughput::Elements(vertex_count));
        g.bench_with_input(
            BenchmarkId::new("triangles", tris),
            &tris,
            |b, &tris| {
                b.iter(|| black_box(make_mesh(tris)));
            },
        );
    }
    g.finish();
}

// ── 3. GpuMesh upload throughput ───────────────────────────────────────────

fn bench_gpu_upload(c: &mut Criterion) {
    // One VulkanContext for the whole group — context creation is expensive.
    let ctx = match VulkanContext::new() {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!("gpu_upload bench skipped: no Vulkan device ({e:?})");
            return;
        }
    };

    let mut g = c.benchmark_group("gpu_mesh_upload");
    // Keep iteration count low — each upload synchronises on a fence.
    g.sample_size(20);

    let upload_ctx = ctx.mesh_upload_ctx();

    for &tris in &[64usize, 1_024, 16_384] {
        let ore = make_mesh(tris);
        let byte_count = (ore.vertices.len() * std::mem::size_of::<ForgeVertex>()
            + ore.indices.len() * 4) as u64;
        g.throughput(Throughput::Bytes(byte_count));
        g.bench_with_input(
            BenchmarkId::new("triangles", tris),
            &ore,
            |b, ore| {
                b.iter(|| {
                    let mut mesh = GpuMesh::upload(&upload_ctx, black_box(ore))
                        .expect("upload");
                    // Destroy immediately so we don't exhaust VRAM over 20 samples.
                    unsafe { mesh.destroy(&ctx.device) };
                });
            },
        );
    }
    g.finish();
}

// ── 4. Draw-call collection (simulates draw_frame inner loop) ─────────────

fn bench_draw_call_collect(c: &mut Criterion) {
    let mut g = c.benchmark_group("draw_call_collect");

    for &factory_count in &[1usize, 8, 64] {
        for &calls_per in &[1usize, 16] {
            let total_calls = factory_count * calls_per;
            g.throughput(Throughput::Elements(total_calls as u64));

            // Build a FactoryMaster-like structure: Vec of Factories, each
            // with `calls_per` GraphicsFrames (no mesh — procedural draws).
            let factories: Vec<_> = (0..factory_count)
                .map(|fi| {
                    let mut proto = Proto::<GraphicsTag>::new(
                        ProtoId::new(fi as i64 + 1),
                        format!("f{fi}"),
                    );
                    for ci in 0..calls_per {
                        proto.push_call(GraphicsFramePlan::new(
                            FrameId::new(ci as i64 + 1),
                            format!("c{ci}"),
                            GraphicsOreKind::Ui,
                            3,
                        ));
                    }
                    Factory::from_graphics_proto(FactoryId::new(fi as i64 + 1), proto)
                })
                .collect();

            g.bench_with_input(
                BenchmarkId::new(format!("factories_{factory_count}"), calls_per),
                &factories,
                |b, factories| {
                    b.iter(|| {
                        let calls: ThinVec<_> = factories
                            .iter()
                            .flat_map(|f| f.graphics_calls().iter().cloned())
                            .collect();
                        black_box(calls);
                    });
                },
            );
        }
    }
    g.finish();
}

// ── Entry point ────────────────────────────────────────────────────────────

criterion_group!(
    benches,
    bench_gltf_parse,
    bench_mesh_ore_build,
    bench_gpu_upload,
    bench_draw_call_collect,
);
criterion_main!(benches);
