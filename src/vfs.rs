//! Handrolled virtual file system.
//!
//! All asset access can be routed through a [`Vfs`]: an ordered set of *mounts*
//! resolved by priority (first match wins). Mounts are a **closed enum** — no
//! `Box<dyn>` — in keeping with the engine's dependency/dispatch philosophy, and
//! the whole module is **std-only** (no `vfs`/`rust-embed`/`include_dir` crate):
//!
//! * [`Mount::Embedded`] — a compile-time registry of `include_bytes!` assets;
//!   the shipping/release source of truth (zero runtime fs dependency).
//! * [`Mount::Dir`] — a directory on the host filesystem, used as a *dev
//!   override*: drop a loose file next to the binary and it shadows the embedded
//!   copy (hot-reload / modding) without recompiling.
//! * [`Mount::Memory`] — an in-memory map for tests and generated assets.
//!
//! Virtual paths are `/`-separated, normalized (`.`/`..`/`//` collapsed), and
//! **sandboxed**: a `..` that would escape a mount root is rejected, so a
//! directory mount can never read outside its root. This is the property that
//! makes the `Dir` override safe to enable in shipped builds.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

/// Bytes returned from the VFS: borrowed `'static` for embedded assets (no
/// copy), owned otherwise.
pub type VfsBytes = Cow<'static, [u8]>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VfsError {
    /// The path could not be normalized, or escaped a mount root via `..`.
    InvalidPath(String),
    /// No mount provided the path.
    NotFound(String),
    /// The bytes were not valid UTF-8 (`read_to_string`).
    NotUtf8(String),
    /// A host filesystem error surfaced from a `Dir` mount.
    Io(String),
}

impl std::fmt::Display for VfsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VfsError::InvalidPath(p) => write!(f, "invalid/escaping vfs path: {p:?}"),
            VfsError::NotFound(p) => write!(f, "asset not found in any mount: {p:?}"),
            VfsError::NotUtf8(p) => write!(f, "asset is not valid UTF-8: {p:?}"),
            VfsError::Io(e) => write!(f, "vfs io error: {e}"),
        }
    }
}

impl std::error::Error for VfsError {}

pub type VfsResult<T> = Result<T, VfsError>;

/// Normalize a virtual path: accept `/` or `\\` separators, drop `.` and empty
/// segments, resolve `..` against earlier segments, and **reject** any `..`
/// that would pop above the root. Returns the canonical `/`-joined path
/// (no leading slash), or `None` if it escapes.
pub fn normalize(path: &str) -> Option<String> {
    let mut out: Vec<&str> = Vec::new();
    for seg in path.split(['/', '\\']) {
        match seg {
            "" | "." => continue,
            ".." => {
                // Escaping above root is not allowed (sandbox).
                out.pop()?;
            }
            s => out.push(s),
        }
    }
    Some(out.join("/"))
}

// ── Mount backends ──────────────────────────────────────────────────────────

/// Compile-time embedded asset registry. Keys are normalized virtual paths;
/// values are `'static` byte slices (typically from `include_bytes!`).
#[derive(Default, Clone)]
pub struct EmbeddedFs {
    entries: BTreeMap<String, &'static [u8]>,
}

impl EmbeddedFs {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build from `(virtual_path, &'static [u8])` pairs. Paths are normalized;
    /// malformed paths are skipped.
    pub fn from_pairs<I>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (&'static str, &'static [u8])>,
    {
        let mut entries = BTreeMap::new();
        for (k, v) in pairs {
            if let Some(n) = normalize(k) {
                entries.insert(n, v);
            }
        }
        Self { entries }
    }

    pub fn insert(&mut self, path: &str, bytes: &'static [u8]) {
        if let Some(n) = normalize(path) {
            self.entries.insert(n, bytes);
        }
    }

    fn read(&self, norm: &str) -> Option<VfsBytes> {
        self.entries.get(norm).map(|b| Cow::Borrowed(*b))
    }

    fn exists(&self, norm: &str) -> bool {
        self.entries.contains_key(norm)
    }

    fn list(&self, prefix: &str, out: &mut Vec<String>) {
        for k in self.entries.keys() {
            if k.starts_with(prefix) {
                out.push(k.clone());
            }
        }
    }
}

/// A host-filesystem directory mount (dev override / hot-reload). Reads are
/// sandboxed to `root` because the VFS normalizes paths before resolving.
#[derive(Clone)]
pub struct DirFs {
    root: PathBuf,
}

impl DirFs {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn resolve(&self, norm: &str) -> PathBuf {
        // `norm` is already sandboxed (no `..` escapes), so a plain join stays
        // under `root`.
        self.root.join(norm)
    }

    fn read(&self, norm: &str) -> Option<VfsBytes> {
        std::fs::read(self.resolve(norm)).ok().map(Cow::Owned)
    }

    fn exists(&self, norm: &str) -> bool {
        self.resolve(norm).is_file()
    }

    fn modified(&self, norm: &str) -> Option<SystemTime> {
        std::fs::metadata(self.resolve(norm))
            .and_then(|m| m.modified())
            .ok()
    }

    fn list(&self, prefix: &str, out: &mut Vec<String>) {
        let base = self.resolve(prefix);
        let dir = if base.is_dir() { base } else { self.root.clone() };
        walk_dir(&self.root, &dir, out);
    }
}

fn walk_dir(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            walk_dir(root, &p, out);
        } else if let Ok(rel) = p.strip_prefix(root) {
            if let Some(s) = rel.to_str() {
                out.push(s.replace('\\', "/"));
            }
        }
    }
}

/// In-memory asset map (tests / generated content).
#[derive(Default, Clone)]
pub struct MemoryFs {
    files: BTreeMap<String, Arc<[u8]>>,
}

impl MemoryFs {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, path: &str, bytes: impl Into<Arc<[u8]>>) {
        if let Some(n) = normalize(path) {
            self.files.insert(n, bytes.into());
        }
    }

    fn read(&self, norm: &str) -> Option<VfsBytes> {
        self.files.get(norm).map(|b| Cow::Owned(b.to_vec()))
    }

    fn exists(&self, norm: &str) -> bool {
        self.files.contains_key(norm)
    }

    fn list(&self, prefix: &str, out: &mut Vec<String>) {
        for k in self.files.keys() {
            if k.starts_with(prefix) {
                out.push(k.clone());
            }
        }
    }
}

/// One mount in a [`Vfs`]. Closed enum (no trait objects).
#[derive(Clone)]
pub enum Mount {
    Embedded(EmbeddedFs),
    Dir(DirFs),
    Memory(MemoryFs),
}

impl Mount {
    fn read(&self, norm: &str) -> Option<VfsBytes> {
        match self {
            Mount::Embedded(m) => m.read(norm),
            Mount::Dir(m) => m.read(norm),
            Mount::Memory(m) => m.read(norm),
        }
    }
    fn exists(&self, norm: &str) -> bool {
        match self {
            Mount::Embedded(m) => m.exists(norm),
            Mount::Dir(m) => m.exists(norm),
            Mount::Memory(m) => m.exists(norm),
        }
    }
    fn modified(&self, norm: &str) -> Option<SystemTime> {
        match self {
            Mount::Dir(m) => m.modified(norm),
            // Embedded / memory assets are immutable for the process lifetime.
            _ => None,
        }
    }
    fn list(&self, prefix: &str, out: &mut Vec<String>) {
        match self {
            Mount::Embedded(m) => m.list(prefix, out),
            Mount::Dir(m) => m.list(prefix, out),
            Mount::Memory(m) => m.list(prefix, out),
        }
    }
}

// ── Vfs ─────────────────────────────────────────────────────────────────────

/// An ordered stack of mounts. The mount at the **front** has the highest
/// priority (checked first), so a `Dir` override mounted via [`Vfs::mount_front`]
/// shadows the embedded base layer.
#[derive(Default, Clone)]
pub struct Vfs {
    mounts: Vec<Mount>,
}

impl Vfs {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a base-layer mount (lowest priority — checked last).
    pub fn mount(&mut self, m: Mount) -> &mut Self {
        self.mounts.push(m);
        self
    }

    /// Add an override mount (highest priority — checked first).
    pub fn mount_front(&mut self, m: Mount) -> &mut Self {
        self.mounts.insert(0, m);
        self
    }

    pub fn mount_count(&self) -> usize {
        self.mounts.len()
    }

    /// Read an asset, trying each mount in priority order.
    pub fn read(&self, path: &str) -> VfsResult<VfsBytes> {
        let norm = normalize(path).ok_or_else(|| VfsError::InvalidPath(path.to_string()))?;
        for m in &self.mounts {
            if let Some(b) = m.read(&norm) {
                return Ok(b);
            }
        }
        Err(VfsError::NotFound(norm))
    }

    /// Read an asset as UTF-8 text.
    pub fn read_to_string(&self, path: &str) -> VfsResult<String> {
        let bytes = self.read(path)?;
        let norm = normalize(path).unwrap_or_else(|| path.to_string());
        match bytes {
            Cow::Borrowed(b) => {
                std::str::from_utf8(b).map(str::to_owned).map_err(|_| VfsError::NotUtf8(norm))
            }
            Cow::Owned(b) => String::from_utf8(b).map_err(|_| VfsError::NotUtf8(norm)),
        }
    }

    /// Whether any mount provides the path.
    pub fn exists(&self, path: &str) -> bool {
        let Some(norm) = normalize(path) else {
            return false;
        };
        self.mounts.iter().any(|m| m.exists(&norm))
    }

    /// Last-modified time of the highest-priority mount that provides the path
    /// (only `Dir` mounts report one — used for hot-reload polling).
    pub fn modified(&self, path: &str) -> Option<SystemTime> {
        let norm = normalize(path)?;
        for m in &self.mounts {
            if m.exists(&norm) {
                return m.modified(&norm);
            }
        }
        None
    }

    /// All asset paths (across mounts) under `prefix`, de-duplicated and sorted.
    pub fn list(&self, prefix: &str) -> Vec<String> {
        let norm = normalize(prefix).unwrap_or_default();
        let mut out = Vec::new();
        for m in &self.mounts {
            m.list(&norm, &mut out);
        }
        out.sort();
        out.dedup();
        out
    }
}

/// The engine's default asset VFS: a host-directory mount on the crate's
/// `assets/` directory when it exists (dev / loose-file assets). Subsystems
/// layer embedded packs or extra directories on top via [`Vfs::mount`] /
/// [`Vfs::mount_front`]. In a shipped binary without that directory the result
/// is an empty VFS, and asset loaders fall back to their direct path I/O.
pub fn engine_default() -> Vfs {
    let mut v = Vfs::new();
    let assets = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
    if assets.is_dir() {
        v.mount(Mount::Dir(DirFs::new(assets)));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_collapses_and_sandboxes() {
        assert_eq!(normalize("a/./b//c").as_deref(), Some("a/b/c"));
        assert_eq!(normalize("a/b/../c").as_deref(), Some("a/c"));
        assert_eq!(normalize("icons\\lucide\\move.svg").as_deref(), Some("icons/lucide/move.svg"));
        assert_eq!(normalize("/leading/slash").as_deref(), Some("leading/slash"));
        // Escapes are rejected.
        assert_eq!(normalize("../secret"), None);
        assert_eq!(normalize("a/../../b"), None);
    }

    #[test]
    fn embedded_read_is_zero_copy() {
        static DATA: &[u8] = b"<svg/>";
        let fs = EmbeddedFs::from_pairs([("icons/x.svg", DATA)]);
        let mut v = Vfs::new();
        v.mount(Mount::Embedded(fs));
        let b = v.read("icons/x.svg").unwrap();
        assert!(matches!(b, Cow::Borrowed(_)), "embedded reads must not copy");
        assert_eq!(&*b, DATA);
        assert!(v.exists("icons/x.svg"));
        assert!(matches!(v.read("nope"), Err(VfsError::NotFound(_))));
    }

    #[test]
    fn front_mount_overrides_base() {
        static EMB: &[u8] = b"embedded";
        let mut v = Vfs::new();
        v.mount(Mount::Embedded(EmbeddedFs::from_pairs([("a.txt", EMB)])));
        let mut mem = MemoryFs::new();
        mem.insert("a.txt", b"override".to_vec());
        v.mount_front(Mount::Memory(mem));
        assert_eq!(v.read_to_string("a.txt").unwrap(), "override");
    }

    #[test]
    fn traversal_is_rejected_at_read() {
        let mut v = Vfs::new();
        v.mount(Mount::Dir(DirFs::new(std::env::temp_dir())));
        assert!(matches!(v.read("../../etc/passwd"), Err(VfsError::InvalidPath(_))));
    }

    #[test]
    fn dir_mount_roundtrip() {
        let dir = std::env::temp_dir().join(format!("dfe_vfs_test_{}", std::process::id()));
        let sub = dir.join("icons");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("a.svg"), b"loose").unwrap();

        let mut v = Vfs::new();
        v.mount(Mount::Dir(DirFs::new(&dir)));
        assert_eq!(v.read("icons/a.svg").unwrap().as_ref(), b"loose");
        assert!(v.exists("icons/a.svg"));
        assert!(v.list("icons").iter().any(|p| p == "icons/a.svg"));
        assert!(v.modified("icons/a.svg").is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
