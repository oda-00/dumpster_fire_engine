//! Object-to-shared-library linking.
//!
//! On Unix the emitted object is ELF and is linked into a `.so` with `ld.lld`.
//! On Windows the object is COFF/PE, so it is linked into a DLL with `clang` as
//! the driver (it locates the MSVC CRT for the `memset`/`memcpy` libcalls LLVM
//! emits and invokes the COFF linker); the exported `df_*` symbols come from the
//! `dllexport` storage class set in codegen. Either way the result is loaded via
//! the cross-platform `libloading` (so the `.so` filename the caller chooses is
//! just a name — its contents are a native module for the host platform).

use std::path::{Path, PathBuf};
use std::process::Command;

pub fn link_shared(obj: &Path, out_so: &Path) -> Result<(), LinkError> {
    if cfg!(windows) {
        let clang = locate_tool(&["clang", "clang-cl"]).ok_or(LinkError::LinkerNotFound)?;
        let status = Command::new(&clang)
            .arg("-shared")
            .arg(obj)
            .arg("-o")
            .arg(out_so)
            .status()
            .map_err(LinkError::Io)?;
        if !status.success() {
            return Err(LinkError::LinkFailed(status.code().unwrap_or(-1)));
        }
        Ok(())
    } else {
        let linker = locate_tool(&["ld.lld", "lld"]).ok_or(LinkError::LinkerNotFound)?;
        let status = Command::new(&linker)
            .args(["-shared", "-Bsymbolic", "-z", "noexecstack", "-o"])
            .arg(out_so)
            .arg(obj)
            .status()
            .map_err(LinkError::Io)?;
        if !status.success() {
            return Err(LinkError::LinkFailed(status.code().unwrap_or(-1)));
        }
        Ok(())
    }
}

/// Find an LLVM tool by stem name, trying the platform exe suffix. Searches
/// PATH first, then the LLVM tree `langc` was built against (`.cargo/config.toml`
/// sets `LLVM_SYS_211_PREFIX` and Cargo propagates it here) and common install
/// locations — the tools sit next to `llvm-config` in `<prefix>/bin`.
fn locate_tool(stems: &[&str]) -> Option<PathBuf> {
    let exe = std::env::consts::EXE_SUFFIX;

    for stem in stems {
        if let Some(p) = which(&format!("{stem}{exe}")) {
            return Some(p);
        }
    }

    let mut roots: Vec<PathBuf> = ["LLVM_SYS_211_PREFIX", "LLVM_SYS_181_PREFIX", "LLVM_PREFIX"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .collect();
    if cfg!(windows) {
        roots.push(PathBuf::from(r"C:\Program Files\LLVM"));
    }
    for root in roots {
        for stem in stems {
            let p = root.join("bin").join(format!("{stem}{exe}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let p = dir.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

#[derive(Debug)]
pub enum LinkError {
    LinkerNotFound,
    LinkFailed(i32),
    Io(std::io::Error),
}

impl core::fmt::Display for LinkError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            LinkError::LinkerNotFound => write!(
                f,
                "linker not found ({}); ensure LLVM's bin is on PATH or LLVM_SYS_211_PREFIX is set",
                if cfg!(windows) { "clang" } else { "ld.lld" }
            ),
            LinkError::LinkFailed(c) => write!(f, "linker exit {c}"),
            LinkError::Io(e) => write!(f, "io: {e}"),
        }
    }
}
