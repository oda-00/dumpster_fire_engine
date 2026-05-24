use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const IAI_VERSION: &str = "0.16.1";
const LLVM_MAJOR: &str = "18";
const LLVM_FULL: &str = "18.1.8";
// Pre-built LLVM is extracted here inside the workspace — gitignored, no sudo needed.
const LLVM_LOCAL: &str = ".llvm/18";

// ── Platform detection ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Distro {
    Debian,
    Fedora,
    Arch,
    Other(String),
}

#[derive(Debug, Clone, PartialEq)]
enum Os {
    Linux(Distro),
    MacOs,
    Windows,
}

#[derive(Debug, Clone)]
struct Platform {
    os: Os,
    is_aarch64: bool,
}

fn detect_platform() -> Platform {
    let is_aarch64 = env::consts::ARCH == "aarch64";
    let os = match env::consts::OS {
        "macos"   => Os::MacOs,
        "windows" => Os::Windows,
        _         => Os::Linux(detect_distro()),
    };
    Platform { os, is_aarch64 }
}

fn detect_distro() -> Distro {
    let content = fs::read_to_string("/etc/os-release").unwrap_or_default();
    let id      = os_release_field(&content, "ID").to_lowercase();
    let like    = os_release_field(&content, "ID_LIKE").to_lowercase();
    if id == "ubuntu" || id == "debian" || like.contains("ubuntu") || like.contains("debian") {
        Distro::Debian
    } else if id == "fedora" || id == "rhel" || id == "centos"
        || id == "rocky" || id == "almalinux"
        || like.contains("fedora") || like.contains("rhel")
    {
        Distro::Fedora
    } else if id == "arch" || id == "manjaro" || like.contains("arch") {
        Distro::Arch
    } else {
        Distro::Other(if id.is_empty() { "unknown".into() } else { id })
    }
}

fn os_release_field<'a>(content: &'a str, key: &str) -> &'a str {
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix(&format!("{key}=")) {
            return rest.trim_matches('"');
        }
    }
    ""
}

// ── Command helpers ───────────────────────────────────────────────────────────

fn run(args: &[&str]) -> bool {
    let args: Vec<&str> = args.iter().copied().filter(|a| !a.is_empty()).collect();
    if args.is_empty() { return false; }
    println!("    > {}", args.join(" "));
    Command::new(args[0])
        .args(&args[1..])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn run_ok(args: &[&str]) -> Result<(), String> {
    if run(args) { Ok(()) } else { Err(format!("command failed: {}", args.join(" "))) }
}

fn cmd_exists(cmd: &str) -> bool {
    Command::new(cmd)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn cmd_output(cmd: &str, args: &[&str]) -> Option<String> {
    Command::new(cmd)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
}

// ── Privilege helpers ─────────────────────────────────────────────────────────

fn is_root() -> bool {
    #[cfg(unix)]
    {
        cmd_output("id", &["-u"])
            .and_then(|s| s.parse::<u32>().ok())
            .map(|uid| uid == 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    { false }
}

fn run_privileged(args: &[&str]) -> Result<(), String> {
    if is_root() {
        run_ok(args)
    } else {
        let mut full = vec!["sudo"];
        full.extend_from_slice(args);
        run_ok(&full)
    }
}

// ── LLVM 18 ───────────────────────────────────────────────────────────────────

// URL of the official pre-built tarball for each platform.
// These are release binaries from the LLVM project — no compilation needed.
fn llvm_tarball_url(platform: &Platform) -> &'static str {
    match (&platform.os, platform.is_aarch64) {
        (Os::Linux(_), false) =>
            concat!("https://github.com/llvm/llvm-project/releases/download/llvmorg-18.1.8",
                    "/clang+llvm-18.1.8-x86_64-linux-gnu-ubuntu-18.04.tar.xz"),
        (Os::Linux(_), true) =>
            concat!("https://github.com/llvm/llvm-project/releases/download/llvmorg-18.1.8",
                    "/clang+llvm-18.1.8-aarch64-linux-gnu.tar.xz"),
        (Os::MacOs, false) =>
            concat!("https://github.com/llvm/llvm-project/releases/download/llvmorg-18.1.8",
                    "/clang+llvm-18.1.8-x86_64-apple-darwin.tar.xz"),
        (Os::MacOs, true) =>
            concat!("https://github.com/llvm/llvm-project/releases/download/llvmorg-18.1.8",
                    "/clang+llvm-18.1.8-arm64-apple-darwin22.0.tar.xz"),
        (Os::Windows, _) =>
            concat!("https://github.com/llvm/llvm-project/releases/download/llvmorg-18.1.8",
                    "/LLVM-18.1.8-win64.exe"),
    }
}

fn local_llvm_dir(root: &Path) -> PathBuf {
    root.join(LLVM_LOCAL)
}

fn local_llvm_config(root: &Path) -> PathBuf {
    #[cfg(windows)]
    { local_llvm_dir(root).join("bin").join("llvm-config.exe") }
    #[cfg(not(windows))]
    { local_llvm_dir(root).join("bin").join("llvm-config") }
}

fn check_llvm18(root: &Path, platform: &Platform) -> bool {
    // Local pre-baked copy takes priority.
    let cfg = local_llvm_config(root);
    if cfg.exists() {
        return cmd_output(cfg.to_str().unwrap(), &["--version"])
            .map(|v| v.starts_with(&format!("{LLVM_MAJOR}.")))
            .unwrap_or(false);
    }
    // Fall back to detecting a system install (Windows installer path).
    if let Os::Windows = &platform.os {
        return resolve_llvm_prefix_windows().is_some();
    }
    false
}

fn resolve_llvm_prefix(root: &Path, platform: &Platform) -> Option<String> {
    // Prefer the local pre-baked directory.
    let local = local_llvm_dir(root);
    if local_llvm_config(root).exists() {
        return Some(local.to_string_lossy().into_owned());
    }
    // Windows installer puts LLVM in a well-known system path.
    if let Os::Windows = &platform.os {
        return resolve_llvm_prefix_windows();
    }
    None
}

fn resolve_llvm_prefix_windows() -> Option<String> {
    let candidates = [
        env::var("LLVM_SYS_180_PREFIX").unwrap_or_default(),
        format!("{}\\LLVM", env::var("PROGRAMFILES").unwrap_or_else(|_| r"C:\Program Files".into())),
        format!("{}\\Programs\\LLVM", env::var("LOCALAPPDATA").unwrap_or_default()),
        r"C:\Program Files\LLVM".to_string(),
        r"C:\LLVM".to_string(),
    ];
    for c in &candidates {
        if c.is_empty() { continue; }
        let cfg = format!("{c}\\bin\\llvm-config.exe");
        if Path::new(&cfg).exists()
            && cmd_output(&cfg, &["--version"])
                .map(|v| v.starts_with(&format!("{LLVM_MAJOR}.")))
                .unwrap_or(false)
        {
            return Some(c.clone());
        }
    }
    None
}

fn install_llvm18(root: &Path, platform: &Platform) -> Result<(), String> {
    if check_llvm18(root, platform) {
        println!("  LLVM {} already present — skipping", LLVM_MAJOR);
        return Ok(());
    }
    match &platform.os {
        Os::Windows => install_llvm18_windows(root, platform),
        _ => install_llvm18_tarball(root, platform),
    }
}

fn install_llvm18_tarball(root: &Path, platform: &Platform) -> Result<(), String> {
    let url = llvm_tarball_url(platform);
    let tmp = env::temp_dir().join(format!("llvm-{LLVM_FULL}.tar.xz"));
    let dest = local_llvm_dir(root);

    println!("  Downloading LLVM {} pre-built release (~400 MB)...", LLVM_FULL);
    println!("  Source: {}", url);
    download_any(url, &tmp)?;

    fs::create_dir_all(&dest)
        .map_err(|e| format!("create {}: {e}", dest.display()))?;

    println!("  Extracting to {}...", dest.display());
    // --strip-components=1 drops the top-level directory from the archive
    run_ok(&[
        "tar", "xJf", tmp.to_str().unwrap(),
        "-C", dest.to_str().unwrap(),
        "--strip-components=1",
    ])?;

    let _ = fs::remove_file(&tmp); // clean up download
    Ok(())
}

fn install_llvm18_windows(root: &Path, platform: &Platform) -> Result<(), String> {
    let url = format!(
        "https://github.com/llvm/llvm-project/releases/download/llvmorg-{LLVM_FULL}/LLVM-{LLVM_FULL}-win64.exe"
    );
    let tmp = env::temp_dir().join(format!("LLVM-{LLVM_FULL}-win64.exe"));

    if cmd_exists("winget") {
        let ok = run(&[
            "winget", "install", "--id", "LLVM.LLVM",
            "--version", LLVM_FULL,
            "--accept-package-agreements", "--accept-source-agreements", "--silent",
        ]);
        if ok && check_llvm18(root, platform) { return Ok(()); }
    }
    if cmd_exists("choco") {
        let ok = run(&["choco", "install", "llvm", "--version", LLVM_FULL, "-y", "--no-progress"]);
        if ok && check_llvm18(root, platform) { return Ok(()); }
    }

    println!("  Downloading LLVM {} installer...", LLVM_FULL);
    download_powershell(&url, &tmp)?;
    run_ok(&[tmp.to_str().unwrap(), "/S"])?;
    std::thread::sleep(std::time::Duration::from_secs(3));

    if check_llvm18(root, platform) { Ok(()) } else {
        Err(format!(
            "LLVM {} installed but not detected — restart your terminal and re-run setup.",
            LLVM_MAJOR
        ))
    }
}

// ── Vulkan ────────────────────────────────────────────────────────────────────

fn check_vulkan(platform: &Platform) -> bool {
    match &platform.os {
        Os::Linux(_) => {
            Path::new("/usr/include/vulkan/vulkan.h").exists()
                || Path::new("/usr/local/include/vulkan/vulkan.h").exists()
        }
        Os::MacOs => cmd_exists("glslangValidator"),
        Os::Windows => {
            env::var("VULKAN_SDK").is_ok() || find_vulkan_sdk_windows().is_some()
        }
    }
}

fn find_vulkan_sdk_windows() -> Option<String> {
    let sdk_root = PathBuf::from(r"C:\VulkanSDK");
    if !sdk_root.exists() { return None; }
    let mut versions: Vec<String> = fs::read_dir(&sdk_root)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    versions.sort();
    Some(format!(r"C:\VulkanSDK\{}", versions.last()?))
}

fn install_vulkan(platform: &Platform) -> Result<(), String> {
    if check_vulkan(platform) {
        println!("  Vulkan already present — skipping");
        return Ok(());
    }
    println!("  Installing Vulkan development libraries...");
    match &platform.os {
        Os::Linux(Distro::Debian) => run_privileged(&[
            "apt-get", "install", "-y", "--no-install-recommends",
            "libvulkan-dev", "vulkan-tools", "glslang-tools",
        ]),
        Os::Linux(Distro::Fedora) => run_privileged(&[
            "dnf", "install", "-y",
            "vulkan-devel", "mesa-vulkan-devel", "glslang",
        ]),
        Os::Linux(Distro::Arch) => run_privileged(&[
            "pacman", "-S", "--noconfirm", "--needed",
            "vulkan-devel", "glslang",
        ]),
        Os::MacOs => run_ok(&[
            "brew", "install", "molten-vk", "vulkan-headers", "vulkan-loader", "glslang",
        ]),
        Os::Windows => install_vulkan_windows(platform),
        Os::Linux(Distro::Other(name)) => Err(format!(
            "Unknown distro '{name}': install libvulkan-dev manually."
        )),
    }
}

fn install_vulkan_windows(platform: &Platform) -> Result<(), String> {
    // Direct download first (universal)
    let url = "https://sdk.lunarg.com/sdk/download/latest/windows/vulkan-sdk.exe";
    let tmp = env::temp_dir().join("vulkan-sdk.exe");

    if cmd_exists("winget") {
        let ok = run(&[
            "winget", "install", "--id", "KhronosGroup.VulkanSDK",
            "--accept-package-agreements", "--accept-source-agreements",
        ]);
        if ok && check_vulkan(platform) { return Ok(()); }
    }

    if cmd_exists("choco") {
        let ok = run(&["choco", "install", "vulkan-sdk", "-y", "--no-progress"]);
        if ok && check_vulkan(platform) { return Ok(()); }
    }

    println!("  Downloading LunarG Vulkan SDK...");
    download_powershell(url, &tmp)?;
    run_ok(&[
        tmp.to_str().unwrap(),
        "--accept-licenses",
        "--default-answer",
        "--confirm-command",
        "install",
    ])
}

// ── CMake ─────────────────────────────────────────────────────────────────────

fn install_cmake(platform: &Platform) -> Result<(), String> {
    if cmd_exists("cmake") {
        println!("  CMake already present — skipping");
        return Ok(());
    }
    println!("  Installing CMake...");
    match &platform.os {
        Os::Linux(Distro::Debian) => run_privileged(&["apt-get", "install", "-y", "cmake"]),
        Os::Linux(Distro::Fedora) => run_privileged(&["dnf", "install", "-y", "cmake"]),
        Os::Linux(Distro::Arch)   => run_privileged(&["pacman", "-S", "--noconfirm", "--needed", "cmake"]),
        Os::MacOs => run_ok(&["brew", "install", "cmake"]),
        Os::Windows => install_cmake_windows(),
        Os::Linux(Distro::Other(name)) => {
            Err(format!("Unknown distro '{name}': install cmake manually."))
        }
    }
}

fn install_cmake_windows() -> Result<(), String> {
    if cmd_exists("winget") {
        let ok = run(&[
            "winget", "install", "--id", "Kitware.CMake",
            "--accept-package-agreements", "--accept-source-agreements",
        ]);
        if ok { return Ok(()); }
    }
    if cmd_exists("choco") {
        let ok = run(&["choco", "install", "cmake", "-y", "--no-progress"]);
        if ok { return Ok(()); }
    }
    // Direct download MSI
    let url = "https://github.com/Kitware/CMake/releases/download/v3.31.0/cmake-3.31.0-windows-x86_64.msi";
    let tmp = env::temp_dir().join("cmake-installer.msi");
    println!("  Downloading CMake installer...");
    download_powershell(url, &tmp)?;
    run_ok(&[
        "msiexec", "/i", tmp.to_str().unwrap(),
        "/quiet", "/norestart", "ALLUSERS=1", "ADD_CMAKE_TO_PATH=System",
    ])
}

// ── Valgrind ──────────────────────────────────────────────────────────────────

fn install_valgrind(platform: &Platform) -> Result<(), String> {
    match &platform.os {
        Os::Windows | Os::MacOs => {
            println!("  [skip] Valgrind not available on this platform — iai-callgrind benches will be skipped at runtime");
            return Ok(());
        }
        Os::Linux(_) => {}
    }
    if cmd_exists("valgrind") {
        println!("  Valgrind already present — skipping");
        return Ok(());
    }
    println!("  Installing Valgrind...");
    match &platform.os {
        Os::Linux(Distro::Debian) => run_privileged(&["apt-get", "install", "-y", "valgrind"]),
        Os::Linux(Distro::Fedora) => run_privileged(&["dnf", "install", "-y", "valgrind"]),
        Os::Linux(Distro::Arch)   => run_privileged(&["pacman", "-S", "--noconfirm", "--needed", "valgrind"]),
        Os::Linux(Distro::Other(n)) => {
            Err(format!("Unknown distro '{n}': install valgrind manually."))
        }
        Os::Windows | Os::MacOs => unreachable!(),
    }
}

// ── iai-callgrind-runner ──────────────────────────────────────────────────────

fn check_iai_runner() -> bool {
    cmd_output("iai-callgrind-runner", &["--version"])
        .map(|v| v.contains(IAI_VERSION))
        .unwrap_or(false)
}

fn install_iai_runner() -> Result<(), String> {
    if check_iai_runner() {
        println!("  iai-callgrind-runner {} already installed — skipping", IAI_VERSION);
        return Ok(());
    }
    println!("  Installing iai-callgrind-runner {}...", IAI_VERSION);
    run_ok(&["cargo", "install", "iai-callgrind-runner", "--version", IAI_VERSION, "--locked"])
}

// ── Environment files ─────────────────────────────────────────────────────────

fn write_env_file(root: &Path, platform: &Platform) {
    let prefix = resolve_llvm_prefix(root, platform);

    // Apply to current process immediately so verify step works without shell restart.
    // SAFETY: single-threaded at this point in setup; no other threads read env.
    if let Some(ref p) = prefix {
        unsafe { env::set_var("LLVM_SYS_180_PREFIX", p); }
        let path = env::var("PATH").unwrap_or_default();
        #[cfg(unix)]
        unsafe { env::set_var("PATH", format!("{p}/bin:{path}")); }
        #[cfg(windows)]
        unsafe { env::set_var("PATH", format!("{p}\\bin;{path}")); }
    }

    match &platform.os {
        Os::Windows => {
            let vk = find_vulkan_sdk_windows();
            if let Some(ref vk) = vk {
                unsafe { env::set_var("VULKAN_SDK", vk); }
            }

            let mut ps = String::from(
                "# Auto-generated by dfe-setup. Re-run tools\\setup to regenerate.\n\
                 # Dot-source in your PowerShell profile:\n\
                 #   . \"$PSScriptRoot\\.env.toolchain.ps1\"\n\n",
            );
            if let Some(ref p) = prefix {
                ps.push_str(&format!("$env:LLVM_SYS_180_PREFIX = \"{p}\"\n"));
                ps.push_str(&format!("$env:PATH = \"{p}\\bin;$env:PATH\"\n"));
            }
            if let Some(ref vk) = vk {
                ps.push_str(&format!("$env:VULKAN_SDK = \"{vk}\"\n"));
                ps.push_str(&format!("$env:PATH = \"{vk}\\Bin;$env:PATH\"\n"));
            }
            let dest = root.join(".env.toolchain.ps1");
            if let Err(e) = fs::write(&dest, &ps) {
                eprintln!("  warning: could not write {}: {e}", dest.display());
            } else {
                println!("  Wrote {}", dest.display());
            }
        }
        _ => {
            let mut sh = String::from(
                "# Auto-generated by dfe-setup. Re-run setup.sh to regenerate.\n\
                 # Add to your shell profile (~/.bashrc or ~/.zshrc):\n\
                 #   source \"$(git rev-parse --show-toplevel)/.env.toolchain\"\n\n",
            );
            if let Some(ref p) = prefix {
                sh.push_str(&format!("export LLVM_SYS_180_PREFIX=\"{p}\"\n"));
                sh.push_str(&format!("export PATH=\"{p}/bin:$PATH\"\n"));
            }
            if let Os::MacOs = &platform.os {
                if let Some(mvk) = cmd_output("brew", &["--prefix", "molten-vk"]) {
                    sh.push_str(&format!(
                        "export DYLD_LIBRARY_PATH=\"{mvk}/lib${{DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}}\"\n"
                    ));
                }
            }
            let dest = root.join(".env.toolchain");
            if let Err(e) = fs::write(&dest, &sh) {
                eprintln!("  warning: could not write {}: {e}", dest.display());
            } else {
                println!("  Wrote {}", dest.display());
            }
        }
    }
}

// ── Download helpers ──────────────────────────────────────────────────────────

fn download_powershell(url: &str, dest: &Path) -> Result<(), String> {
    println!("    Downloading {}...", dest.file_name().unwrap_or_default().to_string_lossy());
    run_ok(&[
        "powershell", "-NoProfile", "-NonInteractive", "-Command",
        &format!(
            "[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; \
             Invoke-WebRequest -Uri '{}' -OutFile '{}'",
            url,
            dest.display()
        ),
    ])
}

// Download using curl, wget, or PowerShell — whichever is available.
fn download_any(url: &str, dest: &Path) -> Result<(), String> {
    if cmd_exists("curl") {
        return run_ok(&["curl", "-fsSL", url, "-o", dest.to_str().unwrap()]);
    }
    if cmd_exists("wget") {
        return run_ok(&["wget", "-q", "-O", dest.to_str().unwrap(), url]);
    }
    download_powershell(url, dest)
}

// ── Workspace root ────────────────────────────────────────────────────────────

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is set to tools/setup when invoked via `cargo run`.
    // Go up two levels: tools/setup → tools → workspace root.
    if let Ok(dir) = env::var("CARGO_MANIFEST_DIR") {
        let p = PathBuf::from(dir);
        if let Some(root) = p.parent().and_then(|t| t.parent()) {
            if root.join("Cargo.toml").exists() {
                return root.to_path_buf();
            }
        }
    }
    env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

// ── Verify ────────────────────────────────────────────────────────────────────

fn verify(root: &Path) -> Result<(), String> {
    println!("  Running cargo check -p langc -p langcd ...");
    let ok = Command::new("cargo")
        .args(["check", "-p", "langc", "-p", "langcd"])
        .current_dir(root)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok {
        Ok(())
    } else {
        Err("cargo check failed — see output above. Ensure LLVM_SYS_180_PREFIX is set correctly.".into())
    }
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() {
    let root = workspace_root();
    let platform = detect_platform();

    let os_label = match &platform.os {
        Os::Linux(Distro::Debian)    => "Linux (Debian/Ubuntu)".to_string(),
        Os::Linux(Distro::Fedora)    => "Linux (Fedora/RHEL)".to_string(),
        Os::Linux(Distro::Arch)      => "Linux (Arch)".to_string(),
        Os::Linux(Distro::Other(n))  => format!("Linux ({})", n),
        Os::MacOs   => (if platform.is_aarch64 { "macOS ARM" } else { "macOS x86_64" }).to_string(),
        Os::Windows => "Windows".to_string(),
    };

    println!();
    println!("dumpster_fire_engine  —  dev environment setup");
    println!("Platform : {}", os_label);
    println!("Root     : {}", root.display());
    println!();

    let results: Vec<(&str, Result<(), String>)> = vec![
        ("LLVM 18",             install_llvm18(&root, &platform)),
        ("Vulkan dev libs",     install_vulkan(&platform)),
        ("CMake",               install_cmake(&platform)),
        ("Valgrind",            install_valgrind(&platform)),
        ("iai-callgrind-runner",install_iai_runner()),
        ("env file",            { write_env_file(&root, &platform); Ok(()) }),
        ("cargo check",         verify(&root)),
    ];

    println!();
    println!("─────────────────────────────────────────");
    println!(" Summary");
    println!("─────────────────────────────────────────");
    let mut failures = 0usize;
    for (name, result) in &results {
        match result {
            Ok(())   => println!("  [  OK  ]  {}", name),
            Err(msg) => { println!("  [ FAIL ]  {}  —  {}", name, msg); failures += 1; }
        }
    }
    println!("─────────────────────────────────────────");
    println!();

    if failures == 0 {
        println!("Setup complete!");
        println!();
        match &platform.os {
            Os::Windows => {
                println!("Add to your PowerShell profile ($PROFILE):");
                println!("  . \"{}\"", root.join(".env.toolchain.ps1").display());
            }
            _ => {
                println!("Add to your shell profile (~/.bashrc or ~/.zshrc):");
                println!("  source \"{}\"", root.join(".env.toolchain").display());
            }
        }
        println!();
        println!("Then: cargo build --workspace");
    } else {
        eprintln!("{} step(s) failed. See messages above.", failures);
        std::process::exit(1);
    }
}
