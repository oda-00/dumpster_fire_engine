//! Runtime throughput benchmarks for compiled `.lang` scripts.
//!
//!   cargo bench --bench script_runtime
//!
//! Plan §8: "runtime performance of scripted HSM/BT vs. native Rust."  We
//! compile a representative script once at module-load time, then drive its
//! `ActiveScript::tick` in a tight loop.  A parallel native-Rust BT with
//! identical structure provides the baseline.
//!
//! Each `tick` covers:
//!   * a Selector(Sequence(condition, action), action) leaf walk,
//!   * an intrinsic predicate call through the EngineAPI,
//!   * one push_effect callback into the EffectSink.

use std::sync::Arc;

use divan::{black_box, Bencher};
use langc::{codegen, OptimizationLevel};
use lang_frontend::{hir::HirScript, lexer::Lexer, parser::Parser, sema};

use dumpster_fire_engine::resource_manager::event_manager::script::{
    ActiveScript, ScriptEntryPoints, ScriptManager, tick_batch,
};
use dumpster_fire_engine::resource_manager::event_manager::script_abi::{
    EffectSink, EngineAPI, engine_api_for_sink, effect_kind,
};

fn main() { divan::main(); }

const SRC: &str = r#"
script "bench_runtime" {
    state { ticks: i32 = 0 }
    scene only {
        on_enter => cue_troupe("warmup");
        behavior {
            selector {
                sequence {
                    condition enemy_in_range(10.0),
                    action attack()
                },
                action patrol_path()
            }
        }
    }
}
"#;

fn compile_to_o(src: &str) -> Arc<str> {
    let dir = std::env::temp_dir().join(format!("dfe_runbench_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let obj = dir.join("rt.o");
    let toks = Lexer::new(src).tokenise().unwrap();
    let ast  = Parser::new(toks).parse_script().unwrap();
    let hir: HirScript = sema::lower(ast).unwrap();
    codegen::compile_to_object(&hir, OptimizationLevel::Aggressive, &obj).unwrap();
    Arc::<str>::from(obj.to_string_lossy().as_ref())
}

struct Fixture {
    _mgr:    ScriptManager,
    script:  ActiveScript,
    entry:   *const dumpster_fire_engine::resource_manager::event_manager::script::ScriptEntryPoints,
    _sink:   Box<EffectSink>,
    api:     EngineAPI,
}

unsafe impl Send for Fixture {}

fn build_fixture() -> Fixture {
    let o = compile_to_o(SRC);
    let mut mgr = ScriptManager::new();
    let id = mgr.load_from_file(o).unwrap();
    let entry_ref: &dumpster_fire_engine::resource_manager::event_manager::script::ScriptEntryPoints =
        mgr.get_entry_points(id).unwrap();
    // The fixture holds a pointer alongside the manager so the manager owns
    // the .so lifetime while we still hand out the EntryPoints across ticks.
    let entry_ptr: *const _ = entry_ref;
    let script = ActiveScript::from_entry(id, entry_ref);
    let mut sink = Box::new(EffectSink::new());
    let api = engine_api_for_sink(&mut sink);
    Fixture { _mgr: mgr, script, entry: entry_ptr, _sink: sink, api }
}

// ── Benches ───────────────────────────────────────────────────────────────────

/// One tick: condition false (actor_count == 0), Selector falls through to
/// `patrol_path()`, push_effect fires once.  Hot loop touches roughly
/// (1 indirect call + 1 condition + 1 effect emit + EffectAbi memset) per
/// iteration after inlining.
#[divan::bench]
fn tick_single(bencher: Bencher) {
    bencher.with_inputs(build_fixture).bench_local_refs(|fx| {
        let entry = unsafe { &*fx.entry };
        let n = fx.script.tick(black_box(entry), &mut fx.api, 0.016);
        black_box(n);
    });
}

/// 1 000 ticks back-to-back.  Lets the per-iteration overhead of divan amortize.
#[divan::bench]
fn tick_1k(bencher: Bencher) {
    bencher.with_inputs(build_fixture).bench_local_refs(|fx| {
        let entry = unsafe { &*fx.entry };
        for _ in 0..1000 {
            let n = fx.script.tick(entry, &mut fx.api, 0.016);
            black_box(n);
        }
    });
}

/// on_enter alone — a single `cue_troupe` callback, no BT walk.
#[divan::bench]
fn on_enter(bencher: Bencher) {
    bencher.with_inputs(build_fixture).bench_local_refs(|fx| {
        let entry = unsafe { &*fx.entry };
        fx.script.run_initial_on_enter(entry, &mut fx.api);
    });
}

/// init_state cost (raw `df_init_state` indirect call).
#[divan::bench]
fn init_state(bencher: Bencher) {
    bencher.with_inputs(build_fixture).bench_local_refs(|fx| {
        let entry = unsafe { &*fx.entry };
        unsafe { (entry.init_state)(fx.script.state_buffer.as_mut_ptr()); }
    });
}

// ── Native-Rust baseline (no FFI, no LLVM-compiled code) ─────────────────────

/// Hand-coded equivalent of the scripted Selector(Sequence(cond, attack), patrol_path).
/// Used as the apples-to-apples baseline for plan §8's "scripted vs. native".
#[inline(always)]
fn native_bt_tick(actor_count: u32, sink: &mut EffectSink) {
    let enemy_in_range = actor_count > 0; // matches IntrinsicPredicate::EnemyInRange stub
    if enemy_in_range {
        sink.entries.push(dumpster_fire_engine::resource_manager::event_manager::script_abi::EffectAbi {
            kind: effect_kind::ATTACK, _pad: [0;7], arg0: 0, arg1: 0,
        });
    } else {
        sink.entries.push(dumpster_fire_engine::resource_manager::event_manager::script_abi::EffectAbi {
            kind: effect_kind::PATROL_PATH, _pad: [0;7], arg0: 0, arg1: 0,
        });
    }
}

#[divan::bench]
fn native_tick_single(bencher: Bencher) {
    bencher.with_inputs(|| EffectSink::new()).bench_local_refs(|sink| {
        native_bt_tick(black_box(0), sink);
    });
}

#[divan::bench]
fn native_tick_1k(bencher: Bencher) {
    bencher.with_inputs(|| EffectSink::new()).bench_local_refs(|sink| {
        for _ in 0..1000 { native_bt_tick(0, sink); }
    });
}

// ── Batch ticks: 4-way unrolled sequential vs rayon-parallel ─────────────────
//
// Mirror of `world_manager::stage::propagate_transforms`: each script's tick
// is independent, so we group them.  The 16- and 32-script benches exercise
// the sequential 4-way unrolled path; 128 and 1024 exercise the rayon path.

struct BatchFixture {
    _mgr:    ScriptManager,
    scripts: Vec<ActiveScript>,
    apis:    Vec<EngineAPI>,
    /// Pre-materialised entry slice; same `&ScriptEntryPoints` repeated N
    /// times (all scripts share the same compiled object).  Held in the
    /// fixture so the per-iter `run_batch` only measures `tick_batch`
    /// itself — not a Vec allocation that would dominate the parallel path.
    entries: Vec<&'static ScriptEntryPoints>,
    _sinks:  Vec<Box<EffectSink>>,
}

unsafe impl Send for BatchFixture {}

fn build_batch(n: usize) -> BatchFixture {
    let o = compile_to_o(SRC);
    let mut mgr = ScriptManager::new();
    let id = mgr.load_from_file(o).unwrap();
    let entry_ref: &ScriptEntryPoints = mgr.get_entry_points(id).unwrap();
    // SAFETY: the entry lives inside `mgr` which is owned by the fixture;
    // we never expose this `'static` reference past the fixture's lifetime.
    let entry_static: &'static ScriptEntryPoints =
        unsafe { &*(entry_ref as *const ScriptEntryPoints) };

    let mut scripts = Vec::with_capacity(n);
    let mut sinks: Vec<Box<EffectSink>> = Vec::with_capacity(n);
    let mut apis = Vec::with_capacity(n);
    for _ in 0..n {
        let mut sink = Box::new(EffectSink::new());
        sink.entries.reserve(2048);
        sink.cues.reserve(2048);
        let api = engine_api_for_sink(&mut sink);
        scripts.push(ActiveScript::from_entry(id, entry_ref));
        sinks.push(sink);
        apis.push(api);
    }
    let entries: Vec<&'static ScriptEntryPoints> = (0..n).map(|_| entry_static).collect();
    BatchFixture { _mgr: mgr, scripts, apis, entries, _sinks: sinks }
}

fn run_batch(fx: &mut BatchFixture) {
    tick_batch(&mut fx.scripts, &fx.entries, &mut fx.apis, 0.016);
}

#[divan::bench]
fn tick_batch_16(bencher: Bencher) {
    bencher.with_inputs(|| build_batch(16))
        .bench_local_refs(|fx| run_batch(fx));
}

#[divan::bench]
fn tick_batch_32(bencher: Bencher) {
    bencher.with_inputs(|| build_batch(32))
        .bench_local_refs(|fx| run_batch(fx));
}

#[divan::bench]
fn tick_batch_128(bencher: Bencher) {
    bencher.with_inputs(|| build_batch(128))
        .bench_local_refs(|fx| run_batch(fx));
}

#[divan::bench]
fn tick_batch_1024(bencher: Bencher) {
    bencher.with_inputs(|| build_batch(1024))
        .bench_local_refs(|fx| run_batch(fx));
}

#[divan::bench]
fn tick_batch_4096(bencher: Bencher) {
    bencher.with_inputs(|| build_batch(4096))
        .bench_local_refs(|fx| run_batch(fx));
}
