use std::path::Path;
use std::process::Command;

fn main() {
    // Auto-install dev tooling that isn't handled by the patched llvm-sys build.rs.
    // Each function is idempotent: it checks whether the tool exists first.
    ensure_iai_callgrind_runner();
    ensure_vulkan_and_glslc();
    ensure_valgrind();

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
        if let Some(ref sc) = shaderc_compiler
            && compile_native(sc, src, &out)
        {
            continue;
        }

        // 2. Fall back to external glslc / glslangValidator.
        if let Some(ref ec) = ext_compiler
            && compile_external(ec, src, &out)
        {
            continue;
        }

        // 3. Reuse a pre-built .spv if present.
        if Path::new(&out).exists() {
            println!(
                "cargo::warning=reusing pre-built {out} \
                      (no compiler succeeded for {src})"
            );
            continue;
        }

        // 4. Emit a warning rather than panicking — source is committed and
        //    will compile on the next build that has shaderc / Vulkan SDK.
        println!(
            "cargo::warning=could not compile {src} and no pre-built \
                  {out} found; runtime loading of this shader will fail"
        );
    }
}

// ── Native shaderc path ────────────────────────────────────────────────────

fn shader_kind(path: &str) -> shaderc::ShaderKind {
    if path.ends_with(".vert") || path.ends_with(".vert.glsl") {
        shaderc::ShaderKind::Vertex
    } else if path.ends_with(".frag") || path.ends_with(".frag.glsl") {
        shaderc::ShaderKind::Fragment
    } else if path.ends_with(".comp") || path.ends_with(".comp.glsl") {
        shaderc::ShaderKind::Compute
    } else if path.ends_with(".rgen") {
        shaderc::ShaderKind::RayGeneration
    } else if path.ends_with(".rmiss") {
        shaderc::ShaderKind::Miss
    } else if path.ends_with(".rchit") {
        shaderc::ShaderKind::ClosestHit
    } else if path.ends_with(".rahit") {
        shaderc::ShaderKind::AnyHit
    } else if path.ends_with(".rint") {
        shaderc::ShaderKind::Intersection
    } else if path.ends_with(".rcall") {
        shaderc::ShaderKind::Callable
    } else {
        shaderc::ShaderKind::InferFromSource
    }
}

fn compile_native(sc: &shaderc::Compiler, src: &str, out: &str) -> bool {
    let source = match std::fs::read_to_string(src) {
        Ok(s) => s,
        Err(e) => {
            println!("cargo::warning=shaderc: could not read {src}: {e}");
            return false;
        }
    };

    let mut opts = shaderc::CompileOptions::new().expect("shaderc::CompileOptions::new");
    // Target Vulkan 1.3 / SPIR-V 1.6. Ray-tracing shaders require SPIR-V ≥ 1.4.
    opts.set_target_env(
        shaderc::TargetEnv::Vulkan,
        shaderc::EnvVersion::Vulkan1_3 as u32,
    );
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
                println!(
                    "cargo::warning=shaderc {src}: {}",
                    artifact.get_warning_messages()
                );
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
    binary: String,
    kind: ExtKind,
}

#[derive(Clone, Copy)]
enum ExtKind {
    Glslc,
    Glslang,
}

fn find_ext_compiler() -> Option<ExtCompiler> {
    if let Ok(sdk) = std::env::var("VULKAN_SDK") {
        for tail in &["Bin/glslc.exe", "bin/glslc"] {
            let c = Path::new(&sdk).join(tail);
            if c.exists() {
                return Some(ExtCompiler {
                    binary: c.to_string_lossy().into_owned(),
                    kind: ExtKind::Glslc,
                });
            }
        }
    }
    if cmd_ok("glslc", &["--version"]) {
        return Some(ExtCompiler {
            binary: "glslc".into(),
            kind: ExtKind::Glslc,
        });
    }
    if cmd_ok("glslangValidator", &["--version"]) {
        return Some(ExtCompiler {
            binary: "glslangValidator".into(),
            kind: ExtKind::Glslang,
        });
    }
    None
}

fn compile_external(ec: &ExtCompiler, src: &str, out: &str) -> bool {
    let status = match ec.kind {
        ExtKind::Glslc => {
            let mut cmd = Command::new(&ec.binary);
            // Explicit stage flags for double-extension files that glslc can't auto-detect.
            if src.ends_with(".vert.glsl") {
                cmd.args(["-fshader-stage=vertex"]);
            } else if src.ends_with(".frag.glsl") {
                cmd.args(["-fshader-stage=fragment"]);
            } else if src.ends_with(".comp.glsl") {
                cmd.args(["-fshader-stage=compute"]);
            }
            cmd.args(["--target-env=vulkan1.3", src, "-o", out])
                .status()
        }
        ExtKind::Glslang => Command::new(&ec.binary)
            .args(["--target-env", "vulkan1.3", "-V", src, "-o", out])
            .status(),
    };
    match status {
        Ok(s) if s.success() => true,
        Ok(s) => {
            println!(
                "cargo::warning=external compiler exited {:?} on {src}",
                s.code()
            );
            false
        }
        Err(e) => {
            println!("cargo::warning=failed to run {}: {e}", ec.binary);
            false
        }
    }
}

// ── Dev-tooling bootstrap ──────────────────────────────────────────────────

fn ensure_iai_callgrind_runner() {
    // iai-callgrind requires valgrind, which is Linux-only.
    if std::env::consts::OS != "linux" {
        return;
    }
    const VERSION: &str = "0.16.1";
    // The runner prints its version to stderr in an error message, not stdout.
    let already_ok = Command::new("iai-callgrind-runner")
        .arg("--version")
        .output()
        .map(|o| {
            let combined = format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr),
            );
            combined.contains(VERSION)
        })
        .unwrap_or(false);
    if already_ok {
        return;
    }
    println!("cargo::warning=Installing iai-callgrind-runner {VERSION} (one-time)...");
    let status = Command::new("cargo")
        .args([
            "install",
            "iai-callgrind-runner",
            "--version",
            VERSION,
            "--locked",
        ])
        .status();
    if !matches!(status, Ok(s) if s.success()) {
        println!(
            "cargo::warning=Could not install iai-callgrind-runner; \
             iai-callgrind benchmarks will fail. Run manually: \
             cargo install iai-callgrind-runner --version {VERSION} --locked"
        );
    }
}

fn ensure_vulkan_and_glslc() {
    let os = std::env::consts::OS;

    // Use proper exit-code checks, not just "did the process launch".
    let has_glslc = cmd_ok("glslc", &["--version"]) || cmd_ok("glslangValidator", &["--version"]);

    let has_vulkan = detect_vulkan(os);

    if has_vulkan && has_glslc {
        return;
    }

    match os {
        "linux" => install_vulkan_linux(has_vulkan, has_glslc),
        "macos" => install_vulkan_macos(has_vulkan, has_glslc),
        "windows" => install_vulkan_windows(has_vulkan, has_glslc),
        _ => {
            if !has_vulkan {
                println!(
                    "cargo::warning=Vulkan runtime not detected. \
                     Install the LunarG Vulkan SDK from https://vulkan.lunarg.com/sdk/home"
                );
            }
        }
    }
}

fn detect_vulkan(os: &str) -> bool {
    match os {
        "linux" => {
            // ldconfig is present on essentially all Linux distributions.
            Command::new("ldconfig")
                .arg("-p")
                .output()
                .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).contains("libvulkan"))
                .unwrap_or(false)
                // Fallback: check well-known library paths directly.
                || Path::new("/usr/lib/x86_64-linux-gnu/libvulkan.so.1").exists()
                || Path::new("/usr/lib/libvulkan.so.1").exists()
                || Path::new("/usr/lib64/libvulkan.so.1").exists()
        }
        "macos" => {
            // MoltenVK exposes itself as libvulkan.dylib in homebrew prefix
            // (both Intel and Apple Silicon) or in the LunarG Vulkan SDK.
            Path::new("/usr/local/lib/libvulkan.dylib").exists()
                || Path::new("/opt/homebrew/lib/libvulkan.dylib").exists()
                // Homebrew keg-only path for molten-vk
                || brew_prefix("molten-vk")
                    .map(|p| Path::new(&p).join("lib/libMoltenVK.dylib").exists())
                    .unwrap_or(false)
                || std::env::var("VULKAN_SDK").is_ok()
        }
        "windows" => {
            // Check exit code, not just process-launch success.
            Command::new("where")
                .arg("vulkan-1.dll")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
                // Also accept the presence of the LunarG SDK directory.
                || std::env::var("VULKAN_SDK").is_ok()
                || Path::new(r"C:\VulkanSDK").exists()
                || Path::new(r"C:\Windows\System32\vulkan-1.dll").exists()
        }
        _ => true,
    }
}

fn install_vulkan_linux(has_vulkan: bool, has_glslc: bool) {
    let mut pkgs: Vec<&str> = Vec::new();
    if !has_vulkan {
        pkgs.extend_from_slice(&["libvulkan1", "libvulkan-dev"]);
    }
    if !has_glslc {
        pkgs.push("glslang-tools");
    }
    if pkgs.is_empty() {
        return;
    }

    // Detect package manager; fall back through apt → dnf → pacman.
    let installed = if cmd_ok("apt-get", &["--version"]) {
        println!("cargo::warning=Installing Vulkan/glslc via apt: {:?}", pkgs);
        try_with_sudo("apt-get", &{
            let mut a = vec!["install", "-y", "--no-install-recommends"];
            a.extend_from_slice(&pkgs);
            a
        })
    } else if cmd_ok("dnf", &["--version"]) {
        // Fedora / RHEL package names differ slightly.
        let fedora_pkgs: Vec<&str> = pkgs
            .iter()
            .map(|&p| match p {
                "libvulkan1" | "libvulkan-dev" => "vulkan-devel",
                "glslang-tools" => "glslang",
                _ => p,
            })
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        println!("cargo::warning=Installing Vulkan/glslc via dnf: {:?}", fedora_pkgs);
        try_with_sudo("dnf", &{
            let mut a = vec!["install", "-y"];
            a.extend_from_slice(&fedora_pkgs);
            a
        })
    } else if cmd_ok("pacman", &["--version"]) {
        let arch_pkgs: Vec<&str> = pkgs
            .iter()
            .map(|&p| match p {
                "libvulkan1" | "libvulkan-dev" => "vulkan-devel",
                "glslang-tools" => "glslang",
                _ => p,
            })
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        println!("cargo::warning=Installing Vulkan/glslc via pacman: {:?}", arch_pkgs);
        try_with_sudo("pacman", &{
            let mut a = vec!["-S", "--noconfirm", "--needed"];
            a.extend_from_slice(&arch_pkgs);
            a
        })
    } else {
        println!(
            "cargo::warning=No known package manager found. \
             Install manually: {}",
            pkgs.join(" ")
        );
        false
    };

    if !installed {
        println!(
            "cargo::warning=Could not auto-install Vulkan packages. \
             Install manually: {}",
            pkgs.join(" ")
        );
    }
}

fn install_vulkan_macos(has_vulkan: bool, has_glslc: bool) {
    if !cmd_ok("brew", &["--version"]) {
        println!(
            "cargo::warning=Homebrew not found. \
             Install MoltenVK and glslang manually, or install Homebrew from https://brew.sh"
        );
        return;
    }
    if !has_vulkan {
        println!("cargo::warning=Installing MoltenVK (Vulkan on macOS)...");
        let ok = Command::new("brew")
            .args(["install", "molten-vk", "vulkan-headers", "vulkan-loader"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            println!(
                "cargo::warning=brew install molten-vk failed. \
                 Run manually: brew install molten-vk vulkan-headers vulkan-loader"
            );
        }
    }
    if !has_glslc {
        println!("cargo::warning=Installing glslang (shader compiler)...");
        let ok = Command::new("brew")
            .args(["install", "glslang"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            println!("cargo::warning=brew install glslang failed. Run manually: brew install glslang");
        }
    }
}

fn install_vulkan_windows(has_vulkan: bool, has_glslc: bool) {
    if !has_vulkan {
        // Try winget first (built into Windows 10 1809+ and all Windows 11).
        let ok = cmd_ok("winget", &["--version"])
            && Command::new("winget")
                .args([
                    "install",
                    "--id",
                    "KhronosGroup.VulkanSDK",
                    "--accept-package-agreements",
                    "--accept-source-agreements",
                    "--silent",
                ])
                .status()
                .map(|s| s.success())
                .unwrap_or(false);

        if !ok {
            // Fallback: chocolatey.
            let ok_choco = cmd_ok("choco", &["--version"])
                && Command::new("choco")
                    .args(["install", "vulkan-sdk", "-y", "--no-progress"])
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false);

            if !ok_choco {
                println!(
                    "cargo::warning=Could not auto-install Vulkan SDK on Windows. \
                     Download from https://vulkan.lunarg.com/sdk/home and re-run the build."
                );
            }
        }
    }

    if !has_glslc {
        // glslc ships with the Vulkan SDK; if it still isn't there after installing
        // the SDK, fall back to glslangValidator via winget/choco.
        if !cmd_ok("glslc", &["--version"]) && !cmd_ok("glslangValidator", &["--version"]) {
            let _ = cmd_ok("winget", &["--version"])
                && Command::new("winget")
                    .args([
                        "install",
                        "--id",
                        "Khronos.glslang",
                        "--accept-package-agreements",
                        "--accept-source-agreements",
                    ])
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false);
        }
    }
}

fn ensure_valgrind() {
    // valgrind is Linux-only; skip silently on other platforms.
    if std::env::consts::OS != "linux" {
        return;
    }
    if cmd_ok("valgrind", &["--version"]) {
        return;
    }
    println!("cargo::warning=valgrind not found; installing for iai-callgrind benchmarks...");
    let ok = if cmd_ok("apt-get", &["--version"]) {
        try_with_sudo("apt-get", &["install", "-y", "valgrind"])
    } else if cmd_ok("dnf", &["--version"]) {
        try_with_sudo("dnf", &["install", "-y", "valgrind"])
    } else if cmd_ok("pacman", &["--version"]) {
        try_with_sudo("pacman", &["-S", "--noconfirm", "--needed", "valgrind"])
    } else {
        false
    };
    if !ok {
        println!("cargo::warning=Could not install valgrind. Run: sudo apt-get install -y valgrind");
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

/// Returns true if `cmd args…` runs and exits with status 0.
fn cmd_ok(cmd: &str, args: &[&str]) -> bool {
    Command::new(cmd)
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Returns the brew prefix for a formula, or None if brew/formula not found.
fn brew_prefix(formula: &str) -> Option<String> {
    Command::new("brew")
        .args(["--prefix", formula])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned())
}

/// Run a command, prepending `sudo -n` (non-interactive) when not already root.
/// Returns true if the command succeeds.
fn try_with_sudo(cmd: &str, args: &[&str]) -> bool {
    let is_root = Command::new("id")
        .arg("-u")
        .output()
        .map(|o| o.stdout.starts_with(b"0"))
        .unwrap_or(false);
    let status = if is_root {
        Command::new(cmd).args(args).status()
    } else {
        Command::new("sudo").arg("-n").arg(cmd).args(args).status()
    };
    matches!(status, Ok(s) if s.success())
}
