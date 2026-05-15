use std::path::Path;
use std::process::Command;

fn main() {
    let glslc = find_glslc();
    let have_glslc = Command::new(&glslc).arg("--version").output().is_ok();
    compile_shader(&glslc, have_glslc, "assets/shaders/triangle.vert");
    compile_shader(&glslc, have_glslc, "assets/shaders/triangle.frag");
    compile_shader(&glslc, have_glslc, "assets/shaders/forward_lit.vert");
    compile_shader(&glslc, have_glslc, "assets/shaders/forward_lit.frag");
}

fn compile_shader(glslc: &str, have_glslc: bool, src: &str) {
    let out = format!("{src}.spv");
    println!("cargo::rerun-if-changed={src}");

    if !have_glslc {
        if Path::new(&out).exists() {
            println!("cargo::warning=glslc not found; reusing existing {out}");
            return;
        }
        panic!("glslc not found and no pre-built {out}");
    }

    let status = Command::new(glslc)
        .args([src, "-o", &out])
        .status()
        .unwrap_or_else(|e| panic!("failed to run glslc ({glslc}): {e}"));

    if !status.success() {
        panic!("glslc failed on {src}");
    }
}

/// Locate glslc. Prefer VULKAN_SDK env var, fall back to PATH.
fn find_glslc() -> String {
    if let Ok(sdk) = std::env::var("VULKAN_SDK") {
        let candidate = Path::new(&sdk).join("Bin").join("glslc.exe");
        if candidate.exists() {
            return candidate.to_string_lossy().into_owned();
        }
        let candidate = Path::new(&sdk).join("bin").join("glslc");
        if candidate.exists() {
            return candidate.to_string_lossy().into_owned();
        }
    }
    // Fall back to glslc on PATH.
    "glslc".to_owned()
}
