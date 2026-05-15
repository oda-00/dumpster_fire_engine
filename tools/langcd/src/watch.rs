//! Linux inotify file watcher — direct rustix, no `notify` crate.
//!
//! Watches a set of file paths and yields `WatchEvent::Modified(path)` when
//! their content changes.  Uses `IN_CLOSE_WRITE | IN_MOVED_TO | IN_MODIFY` so
//! we wake on the same write-completion semantics editors typically produce.

use std::collections::BTreeMap;
use std::mem::MaybeUninit;
use std::os::fd::{AsFd, OwnedFd};
use std::path::{Path, PathBuf};

use rustix::fs::inotify;
use thin_vec::ThinVec;

pub struct Watcher {
    fd:      OwnedFd,
    /// inotify watch descriptor → watched path
    wds:     BTreeMap<i32, PathBuf>,
    /// path → wd (for unwatch)
    by_path: BTreeMap<PathBuf, i32>,
    /// Reusable aligned scratch buffer for inotify reads.  `MaybeUninit<u8>`
    /// to satisfy rustix's `Reader` alignment + initialisation contract.
    buf:     Box<[MaybeUninit<u8>; 8192]>,
}

pub enum WatchEvent {
    Modified(PathBuf),
}

impl Watcher {
    pub fn new() -> std::io::Result<Self> {
        let fd = inotify::init(inotify::CreateFlags::CLOEXEC)?;
        Ok(Watcher {
            fd,
            wds:     BTreeMap::new(),
            by_path: BTreeMap::new(),
            buf:     Box::new([MaybeUninit::uninit(); 8192]),
        })
    }

    pub fn watch(&mut self, path: &Path) -> std::io::Result<()> {
        if self.by_path.contains_key(path) { return Ok(()); }
        let flags = inotify::WatchFlags::CLOSE_WRITE
                  | inotify::WatchFlags::MOVED_TO
                  | inotify::WatchFlags::MODIFY;
        let wd = inotify::add_watch(self.fd.as_fd(), path, flags)? as i32;
        self.wds.insert(wd, path.to_path_buf());
        self.by_path.insert(path.to_path_buf(), wd);
        Ok(())
    }

    pub fn unwatch(&mut self, path: &Path) {
        if let Some(wd) = self.by_path.remove(path) {
            let _ = inotify::remove_watch(self.fd.as_fd(), wd);
            self.wds.remove(&wd);
        }
    }

    /// Blocking read of one or more inotify events.  Pushes one
    /// `WatchEvent::Modified` per recognised record into `out`.
    pub fn read_events(&mut self, out: &mut ThinVec<WatchEvent>) -> std::io::Result<usize> {
        let mut decoded = 0;
        let mut reader = inotify::Reader::new(self.fd.as_fd(), &mut self.buf[..]);
        loop {
            match reader.next() {
                Ok(ev) => {
                    if let Some(path) = self.wds.get(&ev.wd()) {
                        out.push(WatchEvent::Modified(path.clone()));
                        decoded += 1;
                    }
                }
                Err(rustix::io::Errno::AGAIN) => break,
                Err(rustix::io::Errno::INTR) => continue,
                Err(e) => {
                    // The Reader returns errors on every call past the end of
                    // the kernel-supplied batch.  Break and let the next read
                    // refill.  We only return Err for truly fatal cases.
                    if decoded > 0 { return Ok(decoded); }
                    return Err(std::io::Error::from_raw_os_error(e.raw_os_error()));
                }
            }
        }
        Ok(decoded)
    }
}
