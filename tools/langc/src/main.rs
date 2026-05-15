//! `langc` — batch compiler: `.lang` → optimised native `.so`.
//!
//!     langc INPUT.lang -o OUTPUT.so [--opt=0|1|2|3]
//!
//! Pipeline: lex → parse → semantic analysis → LLVM IR (inkwell) → -O3 passes
//! → object file → ld.lld → shared library.

mod codegen;
mod engine_api;
mod link;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use inkwell::OptimizationLevel;
use lang_frontend::{lexer::Lexer, parser::Parser, sema};
use thin_vec::ThinVec;

fn main() -> ExitCode {
    // Stash args as Arc<str> immediately so std::String never lives past the
    // call boundary.  The OS-provided String each arg comes from is dropped
    // at the end of the .map() — only the Arc<str>s survive.
    let args: ThinVec<Arc<str>> = std::env::args()
        .map(|s| Arc::<str>::from(s.as_str()))
        .collect();
    if args.len() < 2 {
        usage();
        return ExitCode::from(1);
    }

    let mut input:  Option<Arc<str>> = None;
    let mut output: Option<Arc<str>> = None;
    let mut opt: OptimizationLevel = OptimizationLevel::Aggressive;
    let mut keep_obj = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_ref() {
            "-o" => {
                if i + 1 >= args.len() { usage(); return ExitCode::from(1); }
                output = Some(Arc::clone(&args[i + 1]));
                i += 2;
            }
            "--opt=0" => { opt = OptimizationLevel::None;        i += 1; }
            "--opt=1" => { opt = OptimizationLevel::Less;        i += 1; }
            "--opt=2" => { opt = OptimizationLevel::Default;     i += 1; }
            "--opt=3" => { opt = OptimizationLevel::Aggressive;  i += 1; }
            "--keep-obj" => { keep_obj = true; i += 1; }
            other if other.starts_with('-') => {
                eprintln!("langc: unknown flag `{other}`");
                usage();
                return ExitCode::from(1);
            }
            _ => {
                if input.is_some() {
                    eprintln!("langc: multiple input files not supported");
                    return ExitCode::from(1);
                }
                input = Some(Arc::clone(&args[i]));
                i += 1;
            }
        }
    }

    let Some(input) = input else { usage(); return ExitCode::from(1); };
    // Default output: replace `.lang` with `.so`.  Avoids constructing a
    // PathBuf::with_extension (which would force conversion through OsString)
    // by stripping the extension at the str level.
    let output = output.unwrap_or_else(|| {
        let stripped: &str = input.strip_suffix(".lang").unwrap_or(input.as_ref());
        Arc::<str>::from(format!("{stripped}.so").as_str())
    });

    match compile(input.as_ref(), output.as_ref(), opt, keep_obj) {
        Ok(()) => {
            eprintln!("langc: wrote {output}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("langc: {e}");
            ExitCode::from(1)
        }
    }
}

fn usage() {
    eprintln!("usage: langc INPUT.lang -o OUTPUT.so [--opt=0|1|2|3] [--keep-obj]");
}

fn compile(input: &str, output: &str, opt: OptimizationLevel, keep_obj: bool) -> Result<(), Arc<str>> {
    let input_p  = Path::new(input);
    let output_p = Path::new(output);

    let src = std::fs::read_to_string(input_p)
        .map_err(|e| Arc::<str>::from(format!("read {input}: {e}").as_str()))?;

    let toks = Lexer::new(&src).tokenise()
        .map_err(|e| Arc::<str>::from(format!("{input}: {e}").as_str()))?;
    let ast = Parser::new(toks).parse_script()
        .map_err(|e| Arc::<str>::from(format!("{input}: {e}").as_str()))?;
    let hir = sema::lower(ast)
        .map_err(|e| Arc::<str>::from(format!("{input}: {e}").as_str()))?;

    // .o sibling of the .so.  PathBuf::with_extension touches OsString
    // internally, but it never leaks past this scope, so it's fine for I/O.
    let obj_path: PathBuf = output_p.with_extension("o");
    codegen::compile_to_object(&hir, opt, &obj_path)
        .map_err(|e| Arc::<str>::from(format!("{e}").as_str()))?;

    link::link_shared(&obj_path, output_p)
        .map_err(|e| Arc::<str>::from(format!("{e}").as_str()))?;

    if !keep_obj { let _ = std::fs::remove_file(&obj_path); }
    Ok(())
}
