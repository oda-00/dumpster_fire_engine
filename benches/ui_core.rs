use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::marker::PhantomData;
use std::num::NonZeroU32;

use dumpster_fire_engine::render::ui_core::{
    id::{path_key, WidgetArena, WidgetId},
    layout::{
        distribute_fill, Alignment, ColumnLayout, Constraint, FillItem, LayoutContext,
        LayoutDispatch, LayoutSolver, Rect, RowLayout, Size,
    },
    manager::UiManager,
    signal::Signal,
    event::{EventBus, UiEvent},
};

fn dummy_id(idx: u32) -> WidgetId {
    WidgetId { idx, generation: NonZeroU32::new(1).unwrap(), _tag: PhantomData }
}

// ── LayoutContext ──────────────────────────────────────────────────────────

fn bench_layout_context_insert(c: &mut Criterion) {
    c.bench_function("LayoutContext::set_size x1000", |b| {
        b.iter(|| {
            let mut ctx = LayoutContext::new();
            for i in 0..1000u32 {
                ctx.set_size(dummy_id(i), black_box(Size { w: i as f32, h: i as f32 }));
            }
        });
    });
}

fn bench_layout_context_lookup(c: &mut Criterion) {
    let mut ctx = LayoutContext::new();
    for i in 0..1000u32 {
        ctx.set_size(dummy_id(i), Size { w: i as f32, h: i as f32 });
    }
    c.bench_function("LayoutContext::get_size (hit)", |b| {
        b.iter(|| ctx.get_size(black_box(dummy_id(500))));
    });
    c.bench_function("LayoutContext::get_size (miss)", |b| {
        b.iter(|| ctx.get_size(black_box(dummy_id(9999))));
    });
}

// ── Signal ─────────────────────────────────────────────────────────────────

fn bench_signal_get(c: &mut Criterion) {
    let s = Signal::new(42u32);
    c.bench_function("Signal::get", |b| b.iter(|| black_box(s.get())));
}

fn bench_signal_set_change(c: &mut Criterion) {
    let mut s = Signal::new(0u32);
    c.bench_function("Signal::set (value changes)", |b| {
        let mut v = 0u32;
        b.iter(|| {
            v = v.wrapping_add(1);
            s.set(black_box(v));
        });
    });
}

fn bench_signal_set_noop(c: &mut Criterion) {
    let mut s = Signal::new(42u32);
    c.bench_function("Signal::set (no-op same value)", |b| {
        b.iter(|| s.set(black_box(42u32)));
    });
}

fn bench_signal_subscribe(c: &mut Criterion) {
    c.bench_function("Signal::subscribe x100 unique", |b| {
        b.iter(|| {
            let mut s = Signal::new(0u32);
            for i in 0..100u32 {
                s.subscribe(dummy_id(i));
            }
            black_box(s.subscribers().len())
        });
    });
}

// ── EventBus ───────────────────────────────────────────────────────────────

fn bench_event_bus_emit_drain(c: &mut Criterion) {
    let mut group = c.benchmark_group("EventBus");
    for n in [10, 100, 1000] {
        group.bench_with_input(BenchmarkId::new("emit+drain", n), &n, |b, &n| {
            b.iter(|| {
                let mut bus = EventBus::new();
                for i in 0..n as u32 {
                    bus.emit(UiEvent::Click(dummy_id(i)));
                }
                bus.drain().count()
            });
        });
    }
    group.finish();
}

// ── Constraint ────────────────────────────────────────────────────────────

fn bench_constraint_clamp(c: &mut Criterion) {
    let constraint = Constraint { min_width: 10.0, max_width: 800.0, min_height: 5.0, max_height: 600.0 };
    c.bench_function("Constraint::clamp", |b| {
        b.iter(|| constraint.clamp(black_box(Size { w: 500.0, h: 300.0 })));
    });
}

// ── Layout dispatch (Phase 0 guard) ─────────────────────────────────────────
// Tracks the enum-dispatch hot path from GUI_research.md §4.1 / exp1_dispatch.rs:
// a homogeneous run of `LayoutDispatch::measure` must stay an inlined, register-only
// loop (no per-element vtable call). Regressions here mean the dispatch win eroded.

fn bench_layout_dispatch(c: &mut Criterion) {
    let arena = WidgetArena::new();
    let items: Vec<LayoutDispatch> = (0..1000u32)
        .map(|i| match i % 3 {
            0 => LayoutDispatch::Row(RowLayout { gap: 4.0, cross_alignment: Alignment::Start }),
            1 => LayoutDispatch::Column(ColumnLayout { gap: 4.0, cross_alignment: Alignment::Start }),
            _ => LayoutDispatch::Null,
        })
        .collect();
    c.bench_function("LayoutDispatch::measure x1000 (enum dispatch)", |b| {
        b.iter(|| {
            let mut ctx = LayoutContext::new();
            let mut acc = 0.0f32;
            for it in &items {
                acc += black_box(it).measure(&[], &arena, &mut ctx).w;
            }
            black_box(acc)
        });
    });
}

// ── Call-site identity (Phase 2) ────────────────────────────────────────────
// The immediate builder now keys widgets by a u64 path hash instead of an
// allocated String path (GUI_research.md §4.2 / exp2_arena.rs). These track the
// hash + sorted-lookup cost that replaced the per-frame String allocation.

fn bench_path_key(c: &mut Criterion) {
    let stack = ["root", "panel", "section"];
    c.bench_function("path_key (3-seg stack + name)", |b| {
        b.iter(|| path_key(black_box(&stack), black_box("button")))
    });
}

fn bench_key_lookup(c: &mut Criterion) {
    let mut m = UiManager::default();
    for i in 0..1000u64 {
        m.register_widget_key(i, dummy_id(i as u32));
    }
    c.bench_function("UiManager::get_widget_by_key (hit)", |b| {
        b.iter(|| m.get_widget_by_key(black_box(500)))
    });
}

// ── Cached / SIMD layout (Phase 3) ──────────────────────────────────────────

fn bench_layout_cache_probe(c: &mut Criterion) {
    let mut ctx = LayoutContext::new();
    let cstr = Constraint { min_width: 0.0, max_width: 800.0, min_height: 0.0, max_height: 600.0 };
    for i in 0..1000u32 {
        ctx.record(dummy_id(i), cstr, Size { w: i as f32, h: i as f32 });
    }
    c.bench_function("LayoutContext::cached (hit, clean subtree reuse)", |b| {
        b.iter(|| ctx.cached(black_box(dummy_id(500)), black_box(&cstr)))
    });
}

fn bench_distribute_fill(c: &mut Criterion) {
    // Flat SoA stream the grow pass sums branchlessly (vaddps, 8x unroll — §4.4).
    let items: Vec<FillItem> = (0..1000u32)
        .map(|i| {
            if i % 4 == 0 {
                FillItem { fixed_px: 0.0, is_fill: 1.0 }
            } else {
                FillItem { fixed_px: 12.0, is_fill: 0.0 }
            }
        })
        .collect();
    c.bench_function("distribute_fill x1000 (branchless SoA)", |b| {
        b.iter(|| distribute_fill(black_box(&items), black_box(8000.0)))
    });
}

criterion_group!(
    benches,
    bench_layout_dispatch,
    bench_path_key,
    bench_key_lookup,
    bench_layout_cache_probe,
    bench_distribute_fill,
    bench_layout_context_insert,
    bench_layout_context_lookup,
    bench_signal_get,
    bench_signal_set_change,
    bench_signal_set_noop,
    bench_signal_subscribe,
    bench_event_bus_emit_drain,
    bench_constraint_clamp,
);
criterion_main!(benches);
