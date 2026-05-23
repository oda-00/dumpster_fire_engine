use std::path::Path;
use std::process::Command;

fn main() {
    // Prefer the native shaderc Rust binding — zero external tool deps.
    // Falls back to glslc / glslangValidator on PATH, then to pre-built .spv.
    let shaderc_compiler = shaderc::Compiler::new();

    let shaders: &[&str] = &[
        "assets/shaders/triangle.vert",
        "assets/shaders/triangle.frag",
        "assets/shaders/forward_lit.vert",
        "assets/shaders/forward_lit.frag",
        "assets/shaders/skinned_forward_lit.vert",
        "assets/shaders/skin_palette.comp.glsl",
        "assets/shaders/morph_blend.comp.glsl",
        "assets/shaders/splat_sort.comp.glsl",
        "assets/shaders/splat_billboard.comp.glsl",
        "assets/shaders/gaussian_splat.vert",
        "assets/shaders/gaussian_splat.frag",
        "assets/shaders/tonemap.vert.glsl",
        "assets/shaders/tonemap.frag.glsl",
        "assets/shaders/debug_lines.vert.glsl",
        "assets/shaders/debug_lines.frag.glsl",
        "assets/shaders/ui.vert.glsl",
        "assets/shaders/ui.frag.glsl",
        "assets/shaders/raygen.rgen",
        "assets/shaders/primary_miss.rmiss",
        "assets/shaders/shadow_miss.rmiss",
        "assets/shaders/primary_chit.rchit",
    ];

    let ext_compiler = find_ext_compiler();

    for src in shaders {
        println!("cargo::rerun-if-changed={src}");
        let out = format!("{src}.spv");

        // 1. Try native shaderc.
        if let Some(ref sc) = shaderc_compiler {
            if compile_native(sc, src, &out) {
                continue;
            }
        }

        // 2. Fall back to external glslc / glslangValidator.
        if let Some(ref ec) = ext_compiler {
            if compile_external(ec, src, &out) {
                continue;
            }
        }

        // 3. Reuse a pre-built .spv if present.
        if Path::new(&out).exists() {
            println!("cargo::warning=reusing pre-built {out} \
                      (no compiler succeeded for {src})");
            continue;
        }

        // 4. Emit a warning rather than panicking — source is committed and
        //    will compile on the next build that has shaderc / Vulkan SDK.
        println!("cargo::warning=could not compile {src} and no pre-built \
                  {out} found; runtime loading of this shader will fail");
    }
}

// ── Native shaderc path ────────────────────────────────────────────────────

fn shader_kind(path: &str) -> shaderc::ShaderKind {
    if      path.ends_with(".vert") || path.ends_with(".vert.glsl") { shaderc::ShaderKind::Vertex }
    else if path.ends_with(".frag") || path.ends_with(".frag.glsl") { shaderc::ShaderKind::Fragment }
    else if path.ends_with(".comp") || path.ends_with(".comp.glsl") { shaderc::ShaderKind::Compute }
    else if path.ends_with(".rgen")                                  { shaderc::ShaderKind::RayGeneration }
    else if path.ends_with(".rmiss")                                 { shaderc::ShaderKind::Miss }
    else if path.ends_with(".rchit")                                 { shaderc::ShaderKind::ClosestHit }
    else if path.ends_with(".rahit")                                 { shaderc::ShaderKind::AnyHit }
    else if path.ends_with(".rint")                                  { shaderc::ShaderKind::Intersection }
    else if path.ends_with(".rcall")                                 { shaderc::ShaderKind::Callable }
    else                                                             { shaderc::ShaderKind::InferFromSource }
}

fn compile_native(sc: &shaderc::Compiler, src: &str, out: &str) -> bool {
    let source = match std::fs::read_to_string(src) {
        Ok(s)  => s,
        Err(e) => {
            println!("cargo::warning=shaderc: could not read {src}: {e}");
            return false;
        }
    };

    let mut opts = shaderc::CompileOptions::new().expect("shaderc::CompileOptions::new");
    // Target Vulkan 1.3 / SPIR-V 1.6. Ray-tracing shaders require SPIR-V ≥ 1.4.
    opts.set_target_env(shaderc::TargetEnv::Vulkan, shaderc::EnvVersion::Vulkan1_3 as u32);
    opts.set_target_spirv(shaderc::SpirvVersion::V1_6);
    // Enable debug info in non-release builds so renderdoc / NSight can show
    // source-level annotations.
    #[cfg(debug_assertions)]
    opts.set_generate_debug_info();
    opts.set_optimization_level(if cfg!(debug_assertions) {
        shaderc::OptimizationLevel::Zero
    } else {
        shaderc::OptimizationLevel::Performance
    });

    let kind = shader_kind(src);
    let result = sc.compile_into_spirv(&source, kind, src, "main", Some(&opts));

    match result {
        Ok(artifact) => {
            if artifact.get_num_warnings() > 0 {
                println!("cargo::warning=shaderc {src}: {}", artifact.get_warning_messages());
            }
            match std::fs::write(out, artifact.as_binary_u8()) {
                Ok(()) => true,
                Err(e) => {
                    println!("cargo::warning=shaderc: failed to write {out}: {e}");
                    false
                }
            }
        }
        Err(e) => {
            println!("cargo::warning=shaderc failed on {src}: {e}");
            false
        }
    }
}

// ── External-tool fallback path ────────────────────────────────────────────

struct ExtCompiler {
    binary:       String,
    version_flag: &'static str,
    kind:         ExtKind,
}

#[derive(Clone, Copy)]
enum ExtKind { Glslc, Glslang }

fn find_ext_compiler() -> Option<ExtCompiler> {
    if let Ok(sdk) = std::env::var("VULKAN_SDK") {
        for tail in &["Bin/glslc.exe", "bin/glslc"] {
            let c = Path::new(&sdk).join(tail);
            if c.exists() {
                return Some(ExtCompiler {
                    binary: c.to_string_lossy().into_owned(),
                    version_flag: "--version",
                    kind: ExtKind::Glslc,
                });
            }
        }
    }
    if Command::new("glslc").arg("--version").output().is_ok() {
        return Some(ExtCompiler { binary: "glslc".into(), version_flag: "--version", kind: ExtKind::Glslc });
    }
    if Command::new("glslangValidator").arg("--version").output().is_ok() {
        return Some(ExtCompiler { binary: "glslangValidator".into(), version_flag: "--version", kind: ExtKind::Glslang });
    }
    None
}

fn compile_external(ec: &ExtCompiler, src: &str, out: &str) -> bool {
    let status = match ec.kind {
        ExtKind::Glslc => {
            let mut cmd = Command::new(&ec.binary);
            // Explicit stage flags for double-extension files that glslc can't auto-detect.
            if src.ends_with(".vert.glsl") { cmd.args(["-fshader-stage=vertex"]); }
            else if src.ends_with(".frag.glsl") { cmd.args(["-fshader-stage=fragment"]); }
            else if src.ends_with(".comp.glsl") { cmd.args(["-fshader-stage=compute"]); }
            cmd.args(["--target-env=vulkan1.3", src, "-o", out]).status()
        }
        ExtKind::Glslang => Command::new(&ec.binary)
            .args(["--target-env", "vulkan1.3", "-V", src, "-o", out])
            .status(),
    };
    match status {
        Ok(s) if s.success() => true,
        Ok(s) => {
            println!("cargo::warning=external compiler exited {:?} on {src}", s.code());
            false
        }
        Err(e) => {
            println!("cargo::warning=failed to run {}: {e}", ec.binary);
            false
        }
    }
}
