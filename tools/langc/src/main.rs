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

use inkwell::OptimizationLevel;
use lang_frontend::{lexer::Lexer, parser::Parser, sema};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        usage();
        return ExitCode::from(1);
    }

    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut opt: OptimizationLevel = OptimizationLevel::Aggressive;
    let mut keep_obj = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-o" => {
                if i + 1 >= args.len() { usage(); return ExitCode::from(1); }
                output = Some(PathBuf::from(&args[i + 1]));
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
                input = Some(PathBuf::from(&args[i]));
                i += 1;
            }
        }
    }

    let Some(input) = input else { usage(); return ExitCode::from(1); };
    let output = output.unwrap_or_else(|| input.with_extension("so"));

    match compile(&input, &output, opt, keep_obj) {
        Ok(()) => {
            eprintln!("langc: wrote {}", output.display());
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

fn compile(input: &Path, output: &Path, opt: OptimizationLevel, keep_obj: bool) -> Result<(), String> {
    let src = std::fs::read_to_string(input)
        .map_err(|e| format!("read {}: {e}", input.display()))?;

    let toks = Lexer::new(&src).tokenise()
        .map_err(|e| format!("{}: {e}", input.display()))?;
    let ast = Parser::new(toks).parse_script()
        .map_err(|e| format!("{}: {e}", input.display()))?;
    let hir = sema::lower(ast)
        .map_err(|e| format!("{}: {e}", input.display()))?;

    let obj_path = output.with_extension("o");
    codegen::compile_to_object(&hir, opt, &obj_path)
        .map_err(|e| e.to_string())?;

    link::link_shared(&obj_path, output)
        .map_err(|e| e.to_string())?;

    if !keep_obj { let _ = std::fs::remove_file(&obj_path); }
    Ok(())
}
