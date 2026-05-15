//! Object-to-shared-library linking using `ld.lld`.

use std::path::{Path, PathBuf};
use std::process::Command;

pub fn link_shared(obj: &Path, out_so: &Path) -> Result<(), LinkError> {
    let linker = locate_linker().ok_or(LinkError::LldNotFound)?;
    let status = Command::new(&linker)
        .args([
            "-shared", "-Bsymbolic",
            "-z", "noexecstack",
            "-o",
        ])
        .arg(out_so)
        .arg(obj)
        .status()
        .map_err(LinkError::Io)?;
    if !status.success() {
        return Err(LinkError::LldFailed(status.code().unwrap_or(-1)));
    }
    Ok(())
}

fn locate_linker() -> Option<PathBuf> {
    for cand in ["ld.lld", "ld.lld-18", "lld"] {
        if which(cand).is_some() { return Some(PathBuf::from(cand)); }
    }
    None
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let p = dir.join(name);
        if p.is_file() { return Some(p); }
    }
    None
}

#[derive(Debug)]
pub enum LinkError {
    LldNotFound,
    LldFailed(i32),
    Io(std::io::Error),
}

impl core::fmt::Display for LinkError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            LinkError::LldNotFound    => write!(f, "ld.lld not found in PATH"),
            LinkError::LldFailed(c)   => write!(f, "ld.lld exit {c}"),
            LinkError::Io(e)          => write!(f, "io: {e}"),
        }
    }
}
