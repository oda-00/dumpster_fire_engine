use std::path::Path;
use std::process::Command;

fn main() {
    let compiler = find_compiler();
    let have_compiler = compiler
        .as_ref()
        .map(|c| Command::new(&c.binary).arg(c.version_flag).output().is_ok())
        .unwrap_or(false);
    let cc = compiler.as_ref();
    compile_shader(cc, have_compiler, "assets/shaders/triangle.vert");
    compile_shader(cc, have_compiler, "assets/shaders/triangle.frag");
    compile_shader(cc, have_compiler, "assets/shaders/forward_lit.vert");
    compile_shader(cc, have_compiler, "assets/shaders/forward_lit.frag");
    compile_shader(cc, have_compiler, "assets/shaders/skinned_forward_lit.vert");
    compile_shader(cc, have_compiler, "assets/shaders/skin_palette.comp.glsl");
    compile_shader(cc, have_compiler, "assets/shaders/morph_blend.comp.glsl");
    compile_shader(cc, have_compiler, "assets/shaders/splat_sort.comp.glsl");
    compile_shader(
        cc,
        have_compiler,
        "assets/shaders/splat_billboard.comp.glsl",
    );
    compile_shader(cc, have_compiler, "assets/shaders/gaussian_splat.vert");
    compile_shader(cc, have_compiler, "assets/shaders/gaussian_splat.frag");
}

struct Compiler {
    binary: String,
    version_flag: &'static str,
    kind: CompilerKind,
}

#[derive(Clone, Copy)]
enum CompilerKind {
    Glslc,
    Glslang,
}

fn compile_shader(compiler: Option<&Compiler>, have_compiler: bool, src: &str) {
    let out = format!("{src}.spv");
    println!("cargo::rerun-if-changed={src}");

    if !have_compiler {
        if Path::new(&out).exists() {
            println!("cargo::warning=no SPIR-V compiler found; reusing existing {out}");
            return;
        }
        panic!("no SPIR-V compiler (glslc or glslangValidator) found and no pre-built {out}");
    }
    let compiler = compiler.unwrap();

    let status = match compiler.kind {
        CompilerKind::Glslc => {
            let mut cmd = Command::new(&compiler.binary);

            if src.ends_with(".vert.glsl") {
                cmd.args(["-fshader-stage=vertex"]);
            } else if src.ends_with(".frag.glsl") {
                cmd.args(["-fshader-stage=fragment"]);
            } else if src.ends_with(".comp.glsl") {
                cmd.args(["-fshader-stage=compute"]);
            }

            cmd.args([src, "-o", &out]).status()
        }

        CompilerKind::Glslang => Command::new(&compiler.binary)
            .args(["-V", src, "-o", &out])
            .status(),
    };
    let status = status.unwrap_or_else(|e| panic!("failed to run {}: {e}", compiler.binary));
    if !status.success() {
        panic!("{} failed on {src}", compiler.binary);
    }
}

/// Locate a SPIR-V compiler. Prefers glslc (via VULKAN_SDK or PATH),
/// falls back to glslangValidator on PATH.
fn find_compiler() -> Option<Compiler> {
    if let Ok(sdk) = std::env::var("VULKAN_SDK") {
        for tail in &["Bin/glslc.exe", "bin/glslc"] {
            let c = Path::new(&sdk).join(tail);
            if c.exists() {
                return Some(Compiler {
                    binary: c.to_string_lossy().into_owned(),
                    version_flag: "--version",
                    kind: CompilerKind::Glslc,
                });
            }
        }
    }
    if Command::new("glslc").arg("--version").output().is_ok() {
        return Some(Compiler {
            binary: "glslc".into(),
            version_flag: "--version",
            kind: CompilerKind::Glslc,
        });
    }
    if Command::new("glslangValidator")
        .arg("--version")
        .output()
        .is_ok()
    {
        return Some(Compiler {
            binary: "glslangValidator".into(),
            version_flag: "--version",
            kind: CompilerKind::Glslang,
        });
    }
    None
}
