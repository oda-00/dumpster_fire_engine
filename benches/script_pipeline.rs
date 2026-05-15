//! Pipeline-stage throughput benchmarks for `.lang` compilation.
//!
//!   cargo bench --bench script_pipeline
//!
//! Each stage is benchmarked in isolation so a regression can be attributed
//! to lex / parse / sema / LLVM-codegen / object-load distinctly.  Two corpora
//! are used — a small guard-style script (~40 tokens) and a larger fan-out
//! script (~5 scenes, ~25 transitions) so we see scaling behaviour.
//!
//! Plan §8: "measure per-function compilation time in langcd".  The
//! `full_compile` benches drive the same `codegen::compile_to_object` path
//! that `langc` and `langcd` use, plus the new object-file load step that
//! replaces the former 23 ms link step.

use divan::{black_box, Bencher};
use langc::{codegen, OptimizationLevel};
use lang_frontend::{
    hir::HirScript,
    lexer::Lexer,
    parser::Parser,
    sema,
    ast::LangScript,
};
use thin_vec::ThinVec;
use dumpster_fire_engine::resource_manager::event_manager::object_loader::LoadedObject;

fn main() { divan::main(); }

// ── Corpora ───────────────────────────────────────────────────────────────────

const SMALL: &str = r#"
script "guard" {
    state {
        patrol_index:    i32 = 0
        last_alert_time: f64 = 0.0
    }
    scene patrol {
        on_enter => cue_troupe("walk");
        behavior {
            selector {
                sequence {
                    condition enemy_in_range(10.0),
                    action attack()
                },
                action patrol_path()
            }
        }
        transition alert when after_seconds(1.5);
    }
    scene alert {
        on_enter => cue_troupe("alarm");
        behavior { action attack() }
        transition patrol when after_seconds(2.0);
    }
}
"#;

const LARGE: &str = r#"
script "fanout" {
    state {
        a: i32 = 0
        b: i32 = 0
        c: i32 = 0
        d: i32 = 0
        e: f64 = 0.0
    }
    scene s0 {
        behavior { action patrol_path() }
        transition s1 when after_seconds(0.1);
        transition s2 when after_seconds(0.2);
        transition s3 when after_seconds(0.3);
        transition s4 when after_seconds(0.4);
    }
    scene s1 {
        behavior { action attack() }
        transition s2 when after_seconds(0.1);
        transition s3 when after_seconds(0.2);
        transition s4 when after_seconds(0.3);
        transition s0 when after_seconds(0.4);
    }
    scene s2 {
        behavior {
            selector {
                sequence { condition enemy_in_range(5.0), action attack() },
                sequence { condition enemy_in_range(20.0), action patrol_path() },
                action patrol_path()
            }
        }
        transition s3 when after_seconds(0.1);
        transition s4 when after_seconds(0.2);
        transition s0 when after_seconds(0.3);
        transition s1 when after_seconds(0.4);
    }
    scene s3 {
        behavior { action attack() }
        transition s4 when after_seconds(0.1);
        transition s0 when after_seconds(0.2);
        transition s1 when after_seconds(0.3);
        transition s2 when after_seconds(0.4);
    }
    scene s4 {
        behavior { action patrol_path() }
        transition s0 when after_seconds(0.1);
        transition s1 when after_seconds(0.2);
        transition s2 when after_seconds(0.3);
        transition s3 when after_seconds(0.4);
    }
}
"#;

fn corpus(label: &str) -> &'static str {
    match label {
        "small" => SMALL,
        "large" => LARGE,
        _ => panic!("unknown corpus {label}"),
    }
}
fn tokens_of(src: &str) -> ThinVec<lang_frontend::lexer::Token> {
    Lexer::new(src).tokenise().expect("lex")
}
fn ast_of(src: &str) -> LangScript {
    Parser::new(tokens_of(src)).parse_script().expect("parse")
}
fn hir_of(src: &str) -> HirScript {
    sema::lower(ast_of(src)).expect("sema")
}
fn opt_of(label: &str) -> OptimizationLevel {
    match label {
        "O0" => OptimizationLevel::None,
        "O3" => OptimizationLevel::Aggressive,
        _    => panic!("unknown opt {label}"),
    }
}

// ── Frontend stages ──────────────────────────────────────────────────────────

#[divan::bench(args = ["small", "large"])]
fn lex(bencher: Bencher, label: &str) {
    let src = corpus(label);
    bencher.bench(|| black_box(Lexer::new(black_box(src)).tokenise().expect("lex")));
}

#[divan::bench(args = ["small", "large"])]
fn parse(bencher: Bencher, label: &str) {
    let toks = tokens_of(corpus(label));
    bencher
        .with_inputs(|| toks.clone())
        .bench_local_values(|toks| {
            black_box(Parser::new(toks).parse_script().expect("parse"))
        });
}

#[divan::bench(args = ["small", "large"])]
fn lower(bencher: Bencher, label: &str) {
    let src = corpus(label);
    bencher
        .with_inputs(|| ast_of(src))
        .bench_local_values(|ast| black_box(sema::lower(ast).expect("sema")));
}

// ── Full LLVM codegen (in-process; no subprocess overhead) ───────────────────

#[divan::bench(args = ["small_O0", "small_O3", "large_O0", "large_O3"])]
fn codegen_to_object(bencher: Bencher, label: &str) {
    let (corpus_label, opt_label) = label.split_once('_').unwrap();
    let hir = hir_of(corpus(corpus_label));
    let opt = opt_of(opt_label);
    let dir = tempdir("script_pipeline_codegen");
    bencher.bench(|| {
        let obj = dir.join("out.o");
        codegen::compile_to_object(black_box(&hir), opt, &obj).expect("codegen");
    });
}

/// Object-load bench: mmap + apply relocations.  Replaces the former
/// `link_to_shared` bench — this is the new hot-reload bottleneck.
#[divan::bench(args = ["small", "large"])]
fn load_object(bencher: Bencher, label: &str) {
    let hir = hir_of(corpus(label));
    let dir = tempdir("script_pipeline_load");
    let obj = dir.join("load_input.o");
    codegen::compile_to_object(&hir, OptimizationLevel::Aggressive, &obj).expect("codegen");
    bencher.bench(|| {
        black_box(LoadedObject::from_file(black_box(&obj)).expect("load"));
    });
}

// ── End-to-end: lex → parse → sema → codegen(-O3) → load ────────────────────

#[divan::bench(args = ["small", "large"])]
fn full_compile(bencher: Bencher, label: &str) {
    let src = corpus(label);
    let dir = tempdir("script_pipeline_full");
    bencher.bench(|| {
        let toks = Lexer::new(black_box(src)).tokenise().expect("lex");
        let ast  = Parser::new(toks).parse_script().expect("parse");
        let hir  = sema::lower(ast).expect("sema");
        let obj  = dir.join("full.o");
        codegen::compile_to_object(&hir, OptimizationLevel::Aggressive, &obj).expect("codegen");
        black_box(LoadedObject::from_file(&obj).expect("load"));
    });
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn tempdir(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("dfe_bench_{label}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
