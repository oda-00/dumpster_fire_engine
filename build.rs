use std::path::Path;
use std::process::Command;

fn main() {
    // Auto-install dev tooling that isn't handled by the patched llvm-sys build.rs.
    // Each function is idempotent: it checks whether the tool exists first.
    ensure_llvm_windows();
    ensure_iai_callgrind_runner();
    ensure_vulkan_and_glslc();
    ensure_valgrind();

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

        // 1. Try external glslc / glslangValidator.
        if let Some(ref ec) = ext_compiler {
            if compile_external(ec, src, &out) {
                continue;
            }
        }

        // 2. Reuse a pre-built .spv if present.
        if Path::new(&out).exists() {
            println!(
                "cargo::warning=reusing pre-built {out} \
                      (no compiler succeeded for {src})"
            );
            continue;
        }

        // 3. Emit a warning — source is committed and pre-built .spv is present
        //    for all current shaders, so this path fires only if .spv is missing.
        println!(
            "cargo::warning=could not compile {src} and no pre-built \
                  {out} found; runtime loading of this shader will fail"
        );
    }
}

// ── External-tool compiler path ───────────────────────────────────────────

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
    if Command::new("glslc").arg("--version").output().is_ok() {
        return Some(ExtCompiler {
            binary: "glslc".into(),
            kind: ExtKind::Glslc,
        });
    }
    if Command::new("glslangValidator")
        .arg("--version")
        .output()
        .is_ok()
    {
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
fn ensure_llvm_windows() {
    if std::env::consts::OS != "windows" {
        return;
    }
    let prefix = std::env::var("LLVM_SYS_180_PREFIX").unwrap_or_else(|_| ".llvm/18".to_string());
    let llvm_path = Path::new(&prefix);
    if llvm_path.join("bin/llvm-config.exe").exists() {
        return;
    }
    println!("cargo::warning=Downloading prebuilt LLVM 18 for Windows...");
    let url = "https://github.com/PLC-lang/llvm-package-windows/releases/download/llvm-18.1.8/llvm-18.1.8-msvc19-x86_64.zip";
    let zip_path = llvm_path.with_extension("zip");
    std::fs::create_dir_all(llvm_path).unwrap();

    // Download with curl or PowerShell
    let ok = if Command::new("curl").arg("--version").output().is_ok() {
        Command::new("curl")
            .args(["-L", "-o", zip_path.to_str().unwrap(), url])
            .status()
    } else {
        Command::new("powershell")
            .args([
                "-Command",
                &format!(
                    "Invoke-WebRequest -Uri {} -OutFile {}",
                    url,
                    zip_path.display()
                ),
            ])
            .status()
    }
    .map(|s| s.success())
    .unwrap_or(false);

    if !ok {
        println!("cargo::warning=Failed to download LLVM");
        return;
    }

    // Extract using PowerShell (or fallback to tar)
    let temp = llvm_path.join("temp_extract");
    std::fs::create_dir_all(&temp).unwrap();
    let extract_ok = if Command::new("powershell").output().is_ok() {
        Command::new("powershell")
            .args([
                "-Command",
                &format!(
                    "Expand-Archive -Path {} -DestinationPath {} -Force",
                    zip_path.display(),
                    temp.display()
                ),
            ])
            .status()
    } else {
        Command::new("tar")
            .args([
                "-xf",
                zip_path.to_str().unwrap(),
                "-C",
                temp.to_str().unwrap(),
            ])
            .status()
    }
    .map(|s| s.success())
    .unwrap_or(false);

    if extract_ok {
        // The zip contains one subfolder like "llvm-18.1.8-msvc19-x86_64"
        let entries: Vec<_> = std::fs::read_dir(&temp)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        let source = if entries.len() == 1 && entries[0].path().is_dir() {
            entries[0].path()
        } else {
            temp.clone()
        };
        for entry in std::fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let dest = llvm_path.join(entry.file_name());
            std::fs::rename(entry.path(), &dest).unwrap();
        }
        std::fs::remove_dir_all(temp).ok();
        std::fs::remove_file(zip_path).ok();
        println!("cargo::warning=LLVM installed to {}", llvm_path.display());
    } else {
        println!("cargo::warning=Failed to extract LLVM");
    }
}
fn ensure_iai_callgrind_runner() {
    // iai-callgrind requires valgrind, which doesn't exist on Windows.
    if std::env::consts::OS == "windows" {
        return;
    }
    const VERSION: &str = "0.16.1";
    // The runner prints its version to stderr in an error message, not stdout.
    // Check both streams and also accept a successful (zero) exit code.
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
    let has_glslc = Command::new("glslc").arg("--version").output().is_ok()
        || Command::new("glslangValidator")
            .arg("--version")
            .output()
            .is_ok();

    // Detect whether the Vulkan runtime loader is present by probing for
    // the shared library directly — ash loads it at runtime via dlopen.
    let has_vulkan = match os {
        "linux" => {
            let out = Command::new("ldconfig").arg("-p").output();
            out.map(|o| String::from_utf8_lossy(&o.stdout).contains("libvulkan"))
                .unwrap_or(false)
        }
        "macos" => {
            Path::new("/usr/local/lib/libvulkan.dylib").exists()
                || Path::new("/opt/homebrew/lib/libvulkan.dylib").exists()
                || std::env::var("VULKAN_SDK").is_ok()
        }
        "windows" => Command::new("where").arg("vulkan-1.dll").output().is_ok(),
        _ => true,
    };

    if has_vulkan && has_glslc {
        return;
    }

    match os {
        "linux" => {
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
            println!(
                "cargo::warning=Installing Vulkan/glslc packages: {:?}",
                pkgs
            );
            let mut args = vec!["apt-get", "install", "-y"];
            args.extend_from_slice(&pkgs);
            let ok = try_with_sudo("apt-get", &args[1..]);
            if !ok {
                println!(
                    "cargo::warning=Could not auto-install Vulkan packages. \
                     Run: sudo apt-get install -y {}",
                    pkgs.join(" ")
                );
            }
        }
        "macos" => {
            if !has_vulkan {
                println!("cargo::warning=Installing MoltenVK (Vulkan on macOS)...");
                let _ = Command::new("brew").args(["install", "molten-vk"]).status();
            }
            if !has_glslc {
                println!("cargo::warning=Installing glslang (shader compiler)...");
                let _ = Command::new("brew").args(["install", "glslang"]).status();
            }
        }
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

fn ensure_valgrind() {
    if Command::new("valgrind").arg("--version").output().is_ok() {
        return;
    }
    let os = std::env::consts::OS;
    println!("cargo::warning=valgrind not found; iai-callgrind benchmarks require it on Linux.");
    match os {
        "linux" => {
            let ok = try_with_sudo("apt-get", &["install", "-y", "valgrind"]);
            if !ok {
                println!("cargo::warning=Run: sudo apt-get install -y valgrind");
            }
        }
        _ => {
            println!(
                "cargo::warning=valgrind is not available on {os}; \
                 iai-callgrind benchmarks will be skipped automatically."
            );
        }
    }
}

/// Run a command, prepending `sudo -n` (non-interactive) when not already root.
/// Returns true if the command succeeds.
fn try_with_sudo(cmd: &str, args: &[&str]) -> bool {
    let is_root = std::env::var("EUID").as_deref() == Ok("0")
        || Command::new("id")
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
