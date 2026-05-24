extern crate anyhow;
extern crate cc;
#[macro_use]
extern crate lazy_static;
extern crate regex_lite;
extern crate semver;

use anyhow::Context as _;
use regex_lite::Regex;
use semver::Version;
use std::env;
use std::ffi::OsStr;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

// Environment variables that can guide compilation
//
// When adding new ones, they should also be added to main() to force a
// rebuild if they are changed.
lazy_static! {
    /// A single path to search for LLVM in (containing bin/llvm-config)
    static ref ENV_LLVM_PREFIX: String =
        format!("LLVM_SYS_{}_PREFIX", env!("CARGO_PKG_VERSION_MAJOR"));

    /// If exactly "YES", ignore the version blocklist
    static ref ENV_IGNORE_BLOCKLIST: String =
        format!("LLVM_SYS_{}_IGNORE_BLOCKLIST", env!("CARGO_PKG_VERSION_MAJOR"));

    /// If set, enforce precise correspondence between crate and binary versions.
    static ref ENV_STRICT_VERSIONING: String =
        format!("LLVM_SYS_{}_STRICT_VERSIONING", env!("CARGO_PKG_VERSION_MAJOR"));

    /// If set, do not attempt to strip irrelevant options for llvm-config --cflags
    static ref ENV_NO_CLEAN_CFLAGS: String =
        format!("LLVM_SYS_{}_NO_CLEAN_CFLAGS", env!("CARGO_PKG_VERSION_MAJOR"));

    /// If set and targeting MSVC, force the debug runtime library
    static ref ENV_USE_DEBUG_MSVCRT: String =
        format!("LLVM_SYS_{}_USE_DEBUG_MSVCRT", env!("CARGO_PKG_VERSION_MAJOR"));

    /// If set, always link against libffi
    static ref ENV_FORCE_FFI: String =
        format!("LLVM_SYS_{}_FFI_WORKAROUND", env!("CARGO_PKG_VERSION_MAJOR"));
}

lazy_static! {
    /// LLVM version used by this version of the crate.
    static ref CRATE_VERSION: Version = {
        let crate_version = Version::parse(env!("CARGO_PKG_VERSION"))
            .expect("Crate version is somehow not valid semver");
        Version {
            major: crate_version.major / 10,
            minor: crate_version.major % 10,
            .. crate_version
        }
    };
}

fn target_env_is(name: &str) -> bool {
    match env::var_os("CARGO_CFG_TARGET_ENV") {
        Some(s) => s == name,
        None => false,
    }
}

fn target_os_is(name: &str) -> bool {
    match env::var_os("CARGO_CFG_TARGET_OS") {
        Some(s) => s == name,
        None => false,
    }
}

/// Try to find a version of llvm-config that is compatible with this crate.
///
/// If $LLVM_SYS_<VERSION>_PREFIX is set, look for llvm-config ONLY in there. The assumption is
/// that the user know best, and they want to link to a specific build or fork of LLVM.
///
/// If $LLVM_SYS_<VERSION>_PREFIX is NOT set, then look for llvm-config in $PATH.
///
/// Returns None on failure.
fn locate_llvm_config() -> Option<PathBuf> {
    let prefix = env::var_os(&*ENV_LLVM_PREFIX)
        .map(|p| PathBuf::from(p).join("bin"))
        .unwrap_or_else(PathBuf::new);
    for binary_name in llvm_config_binary_names() {
        let binary_name = prefix.join(binary_name);
        match llvm_version(&binary_name) {
            Ok(version) => {
                if is_compatible_llvm(&version) {
                    // Compatible version found. Nice.
                    return Some(binary_name);
                }
                // Version mismatch. Will try further searches, but warn that
                // we're not using the system one.
                println!(
                    "found LLVM version {} on PATH, but need {}",
                    version, *CRATE_VERSION
                );
            }
            Err(e) => {
                if e.downcast_ref::<io::Error>()
                    .map_or(false, |e| e.kind() == ErrorKind::NotFound)
                {
                    // Looks like we failed to execute any llvm-config. Keep
                    // searching.
                } else {
                    // Some other error, probably a weird failure. Give up.
                    panic!("Failed to search PATH for llvm-config: {}", e)
                }
            }
        }
    }

    None
}

/// Return an iterator over possible names for the llvm-config binary.
fn llvm_config_binary_names() -> impl Iterator<Item = String> {
    let base_names = [
        "llvm-config".into(),
        format!("llvm-config-{}", CRATE_VERSION.major),
        format!("llvm-config{}", CRATE_VERSION.major),
        format!("llvm{}-config", CRATE_VERSION.major),
        format!(
            "llvm-config-{}.{}",
            CRATE_VERSION.major, CRATE_VERSION.minor
        ),
        format!("llvm-config{}{}", CRATE_VERSION.major, CRATE_VERSION.minor),
    ];

    // On Windows, also search for llvm-config.exe
    if target_os_is("windows") {
        IntoIterator::into_iter(base_names)
            .flat_map(|name| [format!("{}.exe", name), name])
            .collect::<Vec<_>>()
    } else {
        base_names.to_vec()
    }
    .into_iter()
}

/// Check whether the given version of LLVM is blocklisted,
/// returning `Some(reason)` if it is.
fn is_blocklisted_llvm(llvm_version: &Version) -> Option<&'static str> {
    static BLOCKLIST: &'static [(u64, u64, u64, &'static str)] = &[];

    if let Some(x) = env::var_os(&*ENV_IGNORE_BLOCKLIST) {
        if &x == "YES" {
            println!(
                "cargo:warning=ignoring blocklist entry for LLVM {}",
                llvm_version
            );
            return None;
        } else {
            println!(
                "cargo:warning={} is set but not exactly \"YES\"; blocklist is still honored",
                *ENV_IGNORE_BLOCKLIST
            );
        }
    }

    for &(major, minor, patch, reason) in BLOCKLIST.iter() {
        let bad_version = Version {
            major: major,
            minor: minor,
            patch: patch,
            pre: semver::Prerelease::EMPTY,
            build: semver::BuildMetadata::EMPTY,
        };

        if &bad_version == llvm_version {
            return Some(reason);
        }
    }
    None
}

/// Check whether the given LLVM version is compatible with this version of
/// the crate.
fn is_compatible_llvm(llvm_version: &Version) -> bool {
    if let Some(reason) = is_blocklisted_llvm(llvm_version) {
        println!(
            "found LLVM {}, which is blocklisted: {}",
            llvm_version, reason
        );
        return false;
    }

    let strict =
        env::var_os(&*ENV_STRICT_VERSIONING).is_some() || cfg!(feature = "strict-versioning");
    if strict {
        llvm_version.major == CRATE_VERSION.major && llvm_version.minor == CRATE_VERSION.minor
    } else {
        llvm_version.major >= CRATE_VERSION.major
            || (llvm_version.major == CRATE_VERSION.major
                && llvm_version.minor >= CRATE_VERSION.minor)
    }
}

/// Invoke the specified binary as llvm-config.
fn llvm_config<I, S>(binary: &Path, args: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    llvm_config_ex(binary, args).expect("Surprising failure from llvm-config")
}

/// Invoke the specified binary as llvm-config.
///
/// Explicit version of the `llvm_config` function that bubbles errors
/// up.
fn llvm_config_ex<I, S>(binary: &Path, args: I) -> anyhow::Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut cmd = Command::new(binary);
    (|| {
        let Output {
            status,
            stdout,
            stderr,
        } = cmd.args(args).output()?;
        let stdout = String::from_utf8(stdout).context("stdout")?;
        let stderr = String::from_utf8(stderr).context("stderr")?;
        if status.success() {
            Ok(stdout)
        } else {
            Err(anyhow::anyhow!(
                "status={status}\nstdout={}\nstderr={}",
                stdout.trim(),
                stderr.trim()
            ))
        }
    })()
    .with_context(|| format!("{cmd:?}"))
}

/// Get the LLVM version using llvm-config.
fn llvm_version(binary: &Path) -> anyhow::Result<Version> {
    let version_str = llvm_config_ex(binary, ["--version"])?;

    // LLVM isn't really semver and uses version suffixes to build
    // version strings like '3.8.0svn', so limit what we try to parse
    // to only the numeric bits.
    let re = Regex::new(r"^(?P<major>\d+)\.(?P<minor>\d+)(?:\.(?P<patch>\d+))??").unwrap();
    let c = re.captures(&version_str).ok_or_else(|| {
        anyhow::anyhow!(
            "could not determine LLVM version from llvm-config. Version string: {version_str}"
        )
    })?;

    // some systems don't have a patch number but Version wants it so we just append .0 if it isn't
    // there
    let major = c.name("major").unwrap().as_str().parse().context("major")?;
    let minor = c.name("minor").unwrap().as_str().parse().context("minor")?;
    let patch = match c.name("patch") {
        None => 0,
        Some(patch) => patch.as_str().parse().context("patch")?,
    };
    Ok(Version::new(major, minor, patch))
}

/// Get the names of the dylibs required by LLVM, including the C++ standard
/// library.
fn get_system_libraries(llvm_config_path: &Path, kind: LibraryKind) -> Vec<String> {
    let link_arg = match kind {
        LibraryKind::Static => "--link-static",
        LibraryKind::Dynamic => "--link-shared",
    };

    llvm_config(llvm_config_path, ["--system-libs", link_arg])
        .split(&[' ', '\n'] as &[char])
        .filter(|s| !s.is_empty())
        .map(|flag| {
            if target_env_is("msvc") {
                // Same as --libnames, foo.lib
                flag.strip_suffix(".lib").unwrap_or_else(|| {
                    panic!(
                        "system library '{}' does not appear to be a MSVC library file",
                        flag
                    )
                })
            } else {
                if let Some(flag) = flag.strip_prefix("-l") {
                    // Linker flags style, -lfoo
                    if target_os_is("macos") {
                        // .tdb libraries are "text-based stub" files that provide lists of symbols,
                        // which refer to libraries shipped with a given system and aren't shipped
                        // as part of the corresponding SDK. They're named like the underlying
                        // library object, including the 'lib' prefix that we need to strip.
                        if let Some(flag) = flag
                            .strip_prefix("lib")
                            .and_then(|flag| flag.strip_suffix(".tbd"))
                        {
                            return flag;
                        }
                    }

                    if let Some(i) = flag.find(".so.") {
                        // On some distributions (OpenBSD, perhaps others), we get sonames
                        // like "-lz.so.7.0". Correct those by pruning the file extension
                        // and library version.
                        return &flag[..i];
                    }
                    return flag;
                }

                let maybe_lib = Path::new(flag);
                if maybe_lib.is_file() {
                    // Library on disk, likely an absolute path to a .so. We'll add its location to
                    // the library search path and specify the file as a link target.
                    println!(
                        "cargo:rustc-link-search={}",
                        maybe_lib.parent().unwrap().display()
                    );

                    // Expect a file named something like libfoo.so, or with a version libfoo.so.1.
                    // Trim everything after and including the last .so and remove the leading 'lib'
                    let soname = maybe_lib
                        .file_name()
                        .unwrap()
                        .to_str()
                        .expect("Shared library path must be a valid string");
                    let (stem, _rest) = soname
                        .rsplit_once(target_dylib_extension())
                        .expect("Shared library should be a .so file");

                    stem.strip_prefix("lib").unwrap_or_else(|| {
                        panic!("system library '{}' does not have a 'lib' prefix", soname)
                    })
                } else {
                    panic!(
                        "Unable to parse result of llvm-config --system-libs: {}",
                        flag
                    )
                }
            }
        })
        .chain(get_system_libcpp())
        .map(str::to_owned)
        .collect()
}

/// Return additional linker search paths that should be used but that are not discovered
/// by other means.
///
/// In particular, this should include only directories that are known from platform-specific
/// knowledge that aren't otherwise discovered from either `llvm-config` or a linked library
/// that includes an absolute path.
fn get_system_library_dirs() -> impl IntoIterator<Item=&'static str> {
    if target_os_is("openbsd") {
        Some("/usr/local/lib")
    } else {
        None
    }
}

fn target_dylib_extension() -> &'static str {
    if target_os_is("macos") {
        ".dylib"
    } else {
        ".so"
    }
}

/// Get the library that must be linked for C++, if any.
fn get_system_libcpp() -> Option<&'static str> {
    if target_env_is("msvc") {
        // MSVC doesn't need an explicit one.
        None
    } else if target_os_is("macos") {
        // On OS X 10.9 and later, LLVM's libc++ is the default. On earlier
        // releases GCC's libstdc++ is default. Unfortunately we can't
        // reasonably detect which one we need (on older ones libc++ is
        // available and can be selected with -stdlib=lib++), so assume the
        // latest, at the cost of breaking the build on older OS releases
        // when LLVM was built against libstdc++.
        Some("c++")
    } else if target_os_is("freebsd") || target_os_is("openbsd") {
        Some("c++")
    } else if target_env_is("musl") {
        // The one built with musl.
        Some("c++")
    } else {
        // Otherwise assume GCC's libstdc++.
        // This assumption is probably wrong on some platforms, but would need
        // testing on them.
        Some("stdc++")
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LibraryKind {
    Static,
    Dynamic,
}

impl LibraryKind {
    pub fn string(&self) -> &'static str {
        match self {
            LibraryKind::Static => "static",
            LibraryKind::Dynamic => "dylib",
        }
    }
}

/// Get the names of libraries to link against, along with whether it is static or shared library.
fn get_link_libraries(
    llvm_config_path: &Path,
    preferences: &LinkingPreferences,
) -> (LibraryKind, Vec<String>) {
    // Using --libnames in conjunction with --libdir is particularly important
    // for MSVC when LLVM is in a path with spaces, but it is generally less of
    // a hack than parsing linker flags output from --libs and --ldflags.

    fn get_link_libraries_impl(
        llvm_config_path: &Path,
        kind: LibraryKind,
    ) -> anyhow::Result<String> {
        // Windows targets don't get dynamic support.
        // See: https://gitlab.com/taricorp/llvm-sys.rs/-/merge_requests/31#note_1306397918
        if target_env_is("msvc") && kind == LibraryKind::Dynamic {
            anyhow::bail!("Dynamic linking to LLVM is not supported on Windows");
        }

        let link_arg = match kind {
            LibraryKind::Static => "--link-static",
            LibraryKind::Dynamic => "--link-shared",
        };
        llvm_config_ex(llvm_config_path, ["--libnames", link_arg])
    }

    let LinkingPreferences {
        prefer_static,
        force,
    } = preferences;
    let one = [*prefer_static];
    let both = [*prefer_static, !*prefer_static];

    let preferences = if *force { &one[..] } else { &both[..] }
        .iter()
        .map(|is_static| {
            if *is_static {
                LibraryKind::Static
            } else {
                LibraryKind::Dynamic
            }
        });

    for kind in preferences {
        match get_link_libraries_impl(llvm_config_path, kind) {
            Ok(s) => return (kind, extract_library(&s, kind)),
            Err(err) => {
                println!(
                    "failed to get {} libraries from llvm-config: {err:?}",
                    kind.string()
                )
            }
        }
    }

    panic!("failed to get linking libraries from llvm-config",);
}

fn extract_library(s: &str, kind: LibraryKind) -> Vec<String> {
    s.split(&[' ', '\n'] as &[char])
        .filter(|s| !s.is_empty())
        .map(|name| {
            // --libnames gives library filenames. Extract only the name that
            // we need to pass to the linker.
            match kind {
                LibraryKind::Static => {
                    // Match static library
                    if let Some(name) = name
                        .strip_prefix("lib")
                        .and_then(|name| name.strip_suffix(".a"))
                    {
                        // Unix (Linux/Mac)
                        // libLLVMfoo.a
                        name
                    } else if let Some(name) = name.strip_suffix(".lib") {
                        // Windows
                        // LLVMfoo.lib
                        name
                    } else {
                        panic!("'{}' does not look like a static library name", name)
                    }
                }
                LibraryKind::Dynamic => {
                    // Match shared library
                    if let Some(name) = name
                        .strip_prefix("lib")
                        .and_then(|name| name.strip_suffix(".dylib"))
                    {
                        // Mac
                        // libLLVMfoo.dylib
                        name
                    } else if let Some(name) = name
                        .strip_prefix("lib")
                        .and_then(|name| name.strip_suffix(".so"))
                    {
                        // Linux
                        // libLLVMfoo.so
                        name
                    } else if let Some(name) = IntoIterator::into_iter([".dll", ".lib"])
                        .find_map(|suffix| name.strip_suffix(suffix))
                    {
                        // Windows
                        // LLVMfoo.{dll,lib}
                        name
                    } else {
                        panic!("'{}' does not look like a shared library name", name)
                    }
                }
            }
            .to_string()
        })
        .collect::<Vec<String>>()
}

#[derive(Debug, Clone, Copy)]
struct LinkingPreferences {
    /// Prefer static linking over dynamic linking.
    prefer_static: bool,
    /// Force the use of the preferred kind of linking.
    force: bool,
}

impl LinkingPreferences {
    fn init() -> LinkingPreferences {
        let prefer_static = cfg!(feature = "prefer-static");
        let prefer_dynamic = cfg!(feature = "prefer-dynamic");
        let force_static = cfg!(feature = "force-static");
        let force_dynamic = cfg!(feature = "force-dynamic");

        // more than one preference is an error
        if [prefer_static, prefer_dynamic, force_static, force_dynamic]
            .iter()
            .filter(|&&x| x)
            .count()
            > 1
        {
            panic!(
                "Only one of the features `prefer-static`, `prefer-dynamic`, `force-static`, \
                 `force-dynamic` can be enabled at once"
            );
        }

        // if no preference is given, default to force static linking, matching previous behavior
        let force_static = force_static || !(prefer_static || prefer_dynamic || force_dynamic);

        LinkingPreferences {
            prefer_static: force_static || prefer_static,
            force: force_static || force_dynamic,
        }
    }
}

fn get_llvm_cflags(llvm_config_path: &Path) -> String {
    let output = llvm_config(llvm_config_path, ["--cflags"]);

    // llvm-config includes cflags from its own compilation with --cflags that
    // may not be relevant to us. In particularly annoying cases, these might
    // include flags that aren't understood by the default compiler we're
    // using. Unless requested otherwise, clean CFLAGS of options that are
    // known to be possibly-harmful.
    let no_clean = env::var_os(&*ENV_NO_CLEAN_CFLAGS).is_some();
    if no_clean || target_env_is("msvc") {
        // MSVC doesn't accept -W... options, so don't try to strip them and
        // possibly strip something that should be retained. Also do nothing if
        // the user requests it.
        return output;
    }

    output
        .split(&[' ', '\n'][..])
        .filter(|word| !word.starts_with("-W"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_llvm_debug(llvm_config_path: &Path) -> bool {
    // Has to be either Debug or Release
    llvm_config(llvm_config_path, ["--build-mode"]).contains("Debug")
}

fn ensure_llvm_prebuilt() {
    let prefix = match env::var(&*ENV_LLVM_PREFIX) {
        Ok(p) => PathBuf::from(p),
        Err(_) => return,
    };
    let os   = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    let llvm_config = prefix.join(if os == "windows" {
        "bin/llvm-config.exe"
    } else {
        "bin/llvm-config"
    });

    // Also check for the C API header that wrappers/target.c includes.
    // A partial extraction (llvm-config present, include/ absent) would pass
    // the binary check but fail at cc-rs compile time with a confusing error.
    let include_sentinel = prefix.join("include").join("llvm-c").join("Target.h");
    if !llvm_config.exists() || !include_sentinel.exists() {
        download_llvm_prebuilt_coordinated(&prefix, os, arch, &llvm_config);
    }

    // The Ubuntu 18.04 pre-built tarball links against libtinfo.so.5, absent
    // on Ubuntu 22.04+. Compile a minimal stub and inject via LD_LIBRARY_PATH
    // so llvm-config can execute on modern Linux without root/patchelf.
    if os == "linux" {
        let lib_dir = prefix.join("lib");
        ensure_libtinfo_shim(&lib_dir);
        let existing = env::var("LD_LIBRARY_PATH").unwrap_or_default();
        let lib_str  = lib_dir.to_str().unwrap();
        let new_path = if existing.is_empty() {
            lib_str.to_owned()
        } else {
            format!("{lib_str}:{existing}")
        };
        // Safety: build scripts are effectively single-threaded here.
        unsafe { env::set_var("LD_LIBRARY_PATH", &new_path); }
    }
}

// Removes the lock file on drop — ensures cleanup even if the downloader panics.
struct LockGuard(PathBuf);
impl Drop for LockGuard {
    fn drop(&mut self) { let _ = std::fs::remove_file(&self.0); }
}

// Returns true if the lock file was left behind by a process that no longer exists.
fn lock_is_stale(lock_path: &Path) -> bool {
    let content = match std::fs::read_to_string(lock_path) {
        Ok(s) => s,
        Err(_) => return true,
    };
    let pid: u32 = match content.trim().parse() {
        Ok(p) => p,
        // Old lock format with no PID — fall back to mtime: stale after 10 min.
        Err(_) => {
            return std::fs::metadata(lock_path)
                .and_then(|m| m.modified())
                .and_then(|t| std::time::SystemTime::now().duration_since(t).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)))
                .map(|age| age.as_secs() > 600)
                .unwrap_or(true);
        }
    };
    // On Unix check /proc/<pid>; elsewhere fall back to always-live assumption.
    #[cfg(unix)]
    { !std::path::Path::new(&format!("/proc/{pid}")).exists() }
    #[cfg(not(unix))]
    { false }
}

// Coordinates between parallel cargo build-script processes so only one
// actually downloads LLVM — the other polls and waits.
fn download_llvm_prebuilt_coordinated(prefix: &Path, os: &str, arch: &str, llvm_config: &Path) {
    std::fs::create_dir_all(prefix).expect("failed to create LLVM prefix dir");
    let lock_path = prefix.join(".downloading");

    // Purge a stale lock left by a previously-killed build before racing.
    if lock_path.exists() && lock_is_stale(&lock_path) {
        println!("cargo:warning=Stale LLVM 18 download lock detected (owner process is gone). Removing and retrying...");
        let _ = std::fs::remove_file(&lock_path);
    }

    // Try to become the downloader by atomically creating the lock file.
    let got_lock = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true) // fails if the file already exists
        .open(&lock_path)
        .map(|mut f| {
            // Write our PID so waiters can detect if we die.
            let _ = std::io::Write::write_all(&mut f, std::process::id().to_string().as_bytes());
            true
        })
        .unwrap_or(false);

    if got_lock {
        // We won the race. LockGuard removes the sentinel on return OR panic.
        let _guard = LockGuard(lock_path);
        download_llvm_prebuilt(prefix, os, arch);
    } else {
        // Another parallel build script is downloading. Poll until done.
        println!(
            "cargo:warning=Another build process is downloading LLVM 18 — waiting for it to finish..."
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1800);
        loop {
            std::thread::sleep(std::time::Duration::from_secs(5));
            if llvm_config.exists() {
                return;
            }
            // Lock disappeared but llvm-config still absent — the downloader crashed.
            // Take over rather than waiting forever.
            if !lock_path.exists() {
                println!(
                    "cargo:warning=Previous LLVM 18 download appears to have failed. Retrying..."
                );
                download_llvm_prebuilt_coordinated(prefix, os, arch, llvm_config);
                return;
            }
            // Owner process is gone but forgot to clean up — purge and take over.
            if lock_is_stale(&lock_path) {
                println!(
                    "cargo:warning=LLVM 18 download lock is stale (owner process died). Taking over..."
                );
                let _ = std::fs::remove_file(&lock_path);
                download_llvm_prebuilt_coordinated(prefix, os, arch, llvm_config);
                return;
            }
            if std::time::Instant::now() > deadline {
                panic!(
                    "Timed out (30 min) waiting for LLVM 18 download. \
                     Delete {}/.downloading and retry.",
                    prefix.display()
                );
            }
        }
    }
}

fn download_llvm_prebuilt(prefix: &Path, os: &str, arch: &str) {
    let tarball = match (os, arch) {
        ("linux",   "x86_64")  => "clang+llvm-18.1.8-x86_64-linux-gnu-ubuntu-18.04.tar.xz",
        ("linux",   "aarch64") => "clang+llvm-18.1.8-aarch64-linux-gnu.tar.xz",
        ("macos",   "aarch64") => "clang+llvm-18.1.8-arm64-apple-darwin22.0.tar.xz",
        ("macos",   "x86_64")  => "clang+llvm-18.1.8-x86_64-apple-darwin.tar.xz",
        ("windows", "x86_64")  => "LLVM-18.1.8-win64.exe",
        _ => {
            println!(
                "cargo:warning=No pre-built LLVM 18 for {os}/{arch}. \
                 Set LLVM_SYS_180_PREFIX to an existing LLVM 18 installation."
            );
            return;
        }
    };

    let url = format!(
        "https://github.com/llvm/llvm-project/releases/download/llvmorg-18.1.8/{tarball}"
    );
    std::fs::create_dir_all(prefix).expect("failed to create LLVM prefix dir");
    println!(
        "cargo:warning=Downloading pre-built LLVM 18 for {os}/{arch} \
         (one-time ~400 MB cache) — this may take a few minutes..."
    );

    if os == "windows" {
        // PID-unique temp name avoids collision when two build scripts run in parallel.
        let tmp = std::env::temp_dir()
            .join(format!("dfe_llvm18_{}.exe", std::process::id()));
        let tmp_str = tmp.to_str().unwrap();

        // ── Download ──────────────────────────────────────────────────────────
        // curl.exe (Windows 10 1803+) is much faster than Invoke-WebRequest;
        // win_download tries curl first, falls back to PowerShell automatically.
        assert!(win_download(&url, &tmp), "LLVM 18 download failed");

        // ── Extract ───────────────────────────────────────────────────────────
        // The LLVM NSIS installer has a UAC manifest requiring elevation regardless
        // of the target path — running it is not an option without admin.
        // NSIS installers are 7-Zip archives internally; `7z x` extracts them
        // without any installer scripts, no admin, no registry writes.
        //
        // Priority: system 7z → bootstrap 7za.exe from 7-zip.org (no admin needed).
        let prefix_str = prefix.to_str().unwrap();
        let sz = find_7z_windows()
            .or_else(|| bootstrap_7za_windows());

        match sz {
            Some(seven_z) => {
                let ok = Command::new(&seven_z)
                    .args(["x", tmp_str, &format!("-o{prefix_str}"), "-y"])
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false);
                let _ = std::fs::remove_file(&tmp);
                assert!(ok, "7-Zip extraction of LLVM installer failed");
            }
            None => {
                let _ = std::fs::remove_file(&tmp);
                panic!(
                    "Cannot extract LLVM 18: 7-Zip is unavailable and bootstrap failed.\n\
                     Install 7-Zip from https://www.7-zip.org and retry,\n\
                     or set LLVM_SYS_180_PREFIX to an existing LLVM 18 installation."
                );
            }
        }
    } else {
        let cmd = format!(
            "curl -fsSL '{url}' | tar xJf - -C '{}' --strip-components=1",
            prefix.display()
        );
        let status = Command::new("sh").args(["-c", &cmd]).status().expect("curl|tar failed");
        assert!(status.success(), "LLVM download/extract failed");
    }
    println!("cargo:warning=LLVM 18 cached at {}", prefix.display());
}

fn find_7z_windows() -> Option<PathBuf> {
    for candidate in &[
        "7z.exe",
        r"C:\Program Files\7-Zip\7z.exe",
        r"C:\Program Files (x86)\7-Zip\7z.exe",
    ] {
        if Command::new(candidate)
            .arg("i") // info command, exits 0
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return Some(PathBuf::from(candidate));
        }
    }
    None
}

// Two-stage bootstrap to get a modern 7za.exe without any pre-installed tools:
//
//   Stage 1 — 7za920.zip (7-zip.org, ~374 KB) is a plain ZIP so PowerShell's
//              built-in Expand-Archive opens it without any tools. Gives 7za 9.20.
//
//   Stage 2 — 7za 9.20 supports LZMA2 so it can open modern .7z archives.
//              We download 7z2601-extra.7z from GitHub releases (ip7z/7zip),
//              which contains 7za 26.01. That version understands NSIS 3.x
//              (LLVM 18 uses NSIS 3.x; 7za 9.20 only handles NSIS 2.x).
//
//   NOTE: the old stage-2 URL https://www.7-zip.org/a/7z2408-extra.7z now
//   returns 404. GitHub releases are used instead — they are stable permanent
//   URLs and don't get purged.
fn bootstrap_7za_windows() -> Option<PathBuf> {
    println!(
        "cargo:warning=7-Zip not found — bootstrapping 7za 26.01 (~1.2 MB, one-time)..."
    );

    let tmp = std::env::temp_dir().join(format!("dfe_7za_bootstrap_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp);

    // ── Stage 1: 7za920.zip → 7za 9.20 via PowerShell Expand-Archive ─────────
    let zip920 = tmp.join("7za920.zip");
    if !win_download("https://www.7-zip.org/a/7za920.zip", &zip920) {
        println!("cargo:warning=7za bootstrap: stage-1 download of 7za920.zip failed.");
        return None;
    }
    let s1 = tmp.join("s1");
    let _ = std::fs::create_dir_all(&s1);
    let ok = Command::new("powershell")
        .args([
            "-NoProfile", "-Command",
            &format!(
                "Expand-Archive -LiteralPath '{}' -DestinationPath '{}' -Force",
                zip920.display(), s1.display()
            ),
        ])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        println!("cargo:warning=7za bootstrap: Expand-Archive failed.");
        return None;
    }
    let old_7za = s1.join("7za.exe");
    if !old_7za.exists() {
        println!("cargo:warning=7za bootstrap: 7za.exe not found in 7za920.zip.");
        return None;
    }

    // ── Stage 2: 7za 9.20 extracts 7z2601-extra.7z → 7za 26.01 ──────────────
    // Primary: GitHub releases (stable permanent URLs, not purged).
    // Fallback: 7-zip.org direct (may drift over time).
    let extra = tmp.join("7z_extra.7z");
    let extra_urls = [
        "https://github.com/ip7z/7zip/releases/download/26.01/7z2601-extra.7z",
        "https://www.7-zip.org/a/7z2601-extra.7z",
    ];
    let downloaded = extra_urls.iter().any(|url| win_download(url, &extra));
    if !downloaded {
        println!("cargo:warning=7za bootstrap: stage-2 download of 7z2601-extra.7z failed from all URLs.");
        return None;
    }
    let s2 = tmp.join("s2");
    let _ = std::fs::create_dir_all(&s2);
    let ok = Command::new(&old_7za)
        .args(["x", extra.to_str().unwrap(), &format!("-o{}", s2.display()), "-y"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        println!("cargo:warning=7za bootstrap: stage-2 extraction of 7z2601-extra.7z failed.");
        return None;
    }

    // 7za.exe from the extra package sits at the archive root.
    let new_7za = s2.join("7za.exe");
    if new_7za.exists() {
        return Some(new_7za);
    }
    // Search one level of subdirectories as a fallback.
    std::fs::read_dir(&s2).ok()?.flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.path().join("7za.exe"))
        .find(|p| p.exists())
}

/// Download `url` to `dest` using curl.exe (Windows 10 built-in), falling back
/// to PowerShell's Invoke-WebRequest. Returns true on success.
fn win_download(url: &str, dest: &Path) -> bool {
    let s = dest.to_str().unwrap();
    Command::new("curl.exe")
        .args(["-fsSL", "--retry", "3", "-o", s, url])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
        || Command::new("powershell")
            .args(["-NoProfile", "-Command",
                   &format!("Invoke-WebRequest -Uri '{url}' -OutFile '{s}'")])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
}

fn ensure_libtinfo_shim(lib_dir: &Path) {
    let shim = lib_dir.join("libtinfo.so.5");
    if shim.exists() {
        return;
    }
    std::fs::create_dir_all(lib_dir).ok();

    // Four versioned symbols imported by Ubuntu 18.04-built LLVM binaries.
    // Stubs return safe no-op values; llvm-config ignores terminal-capability
    // failures when querying --version/--libs/--cflags.
    let c_src = b"
void setupterm(char *t, int f, int *e) { if (e) *e = -1; }
void *set_curterm(void *t) { return t; }
void del_curterm(void *t) {}
int tigetnum(char *c) { return -1; }
";
    let ver_script = b"
NCURSES_TINFO_5.0.19991023 {
    global: del_curterm; set_curterm; setupterm; tigetnum;
    local: *;
};
";
    let tmp      = std::env::temp_dir();
    let src_path = tmp.join("dfe_tinfo5_shim.c");
    let map_path = tmp.join("dfe_tinfo5.map");
    std::fs::write(&src_path, c_src).expect("write tinfo5 shim");
    std::fs::write(&map_path, ver_script).expect("write tinfo5 map");

    let ok = Command::new("cc")
        .args([
            "-shared", "-fPIC",
            "-o", shim.to_str().unwrap(),
            src_path.to_str().unwrap(),
            &format!("-Wl,--version-script={}", map_path.display()),
            "-Wl,-soname,libtinfo.so.5",
        ])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !ok {
        println!(
            "cargo:warning=Could not compile libtinfo.so.5 compatibility shim. \
             If the build fails, install libtinfo5 or point LLVM_SYS_180_PREFIX \
             at a system LLVM 18 that works on your distro."
        );
    }
}

fn main() {
    ensure_llvm_prebuilt();

    // Behavior can be significantly affected by these vars.
    println!("cargo:rerun-if-env-changed={}", &*ENV_LLVM_PREFIX);
    if let Ok(path) = env::var(&*ENV_LLVM_PREFIX) {
        println!("cargo:rerun-if-changed={}", path);
    }

    println!("cargo:rerun-if-env-changed={}", &*ENV_IGNORE_BLOCKLIST);
    println!("cargo:rerun-if-env-changed={}", &*ENV_STRICT_VERSIONING);
    println!("cargo:rerun-if-env-changed={}", &*ENV_NO_CLEAN_CFLAGS);
    println!("cargo:rerun-if-env-changed={}", &*ENV_USE_DEBUG_MSVCRT);
    println!("cargo:rerun-if-env-changed={}", &*ENV_FORCE_FFI);

    if cfg!(feature = "no-llvm-linking") && cfg!(feature = "disable-alltargets-init") {
        // exit early as we don't need to do anything and llvm-config isn't needed at all
        return;
    }

    let llvm_config_path = match locate_llvm_config() {
        None => {
            println!("cargo:rustc-cfg=LLVM_SYS_NOT_FOUND");
            return;
        }
        Some(llvm_config_path) => llvm_config_path,
    };

    // Build the extra wrapper functions.
    if !cfg!(feature = "disable-alltargets-init") {
        std::env::set_var("CFLAGS", get_llvm_cflags(&llvm_config_path));
        cc::Build::new()
            .file("wrappers/target.c")
            .compile("targetwrappers");
    }

    if cfg!(feature = "no-llvm-linking") {
        return;
    }

    let libdir = llvm_config(&llvm_config_path, ["--libdir"]);

    // Export information to other crates
    println!("cargo:config_path={}", llvm_config_path.display()); // will be DEP_LLVM_CONFIG_PATH
    println!("cargo:libdir={}", libdir); // DEP_LLVM_LIBDIR

    let preferences = LinkingPreferences::init();

    // Link LLVM libraries
    println!("cargo:rustc-link-search=native={}", libdir);
    for link_search_dir in get_system_library_dirs() {
        println!("cargo:rustc-link-search=native={}", link_search_dir);
    }
    // We need to take note of what kind of libraries we linked to, so that
    // we can link to the same kind of system libraries
    let (kind, libs) = get_link_libraries(&llvm_config_path, &preferences);
    for name in libs {
        println!("cargo:rustc-link-lib={}={}", kind.string(), name);
    }

    // Link system libraries
    // We get the system libraries based on the kind of LLVM libraries we link to, but we link to
    // system libs based on the target environment.
    let sys_lib_kind = if target_env_is("musl") {
        LibraryKind::Static
    } else {
        LibraryKind::Dynamic
    };
    for name in get_system_libraries(&llvm_config_path, kind) {
        println!("cargo:rustc-link-lib={}={}", sys_lib_kind.string(), name);
    }

    let use_debug_msvcrt = env::var_os(&*ENV_USE_DEBUG_MSVCRT).is_some();
    if target_env_is("msvc") && (use_debug_msvcrt || is_llvm_debug(&llvm_config_path)) {
        println!("cargo:rustc-link-lib={}", "msvcrtd");
    }

    // Link libffi if the user requested this workaround.
    // See https://bitbucket.org/tari/llvm-sys.rs/issues/12/
    let force_ffi = env::var_os(&*ENV_FORCE_FFI).is_some();
    if force_ffi {
        println!("cargo:rustc-link-lib=dylib={}", "ffi");
    }
}
