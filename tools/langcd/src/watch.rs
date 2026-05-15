//! Linux inotify file watcher — direct rustix, no `notify` crate.
//!
//! **Threading model.**  The `Watcher` itself is single-owner — it is *moved*
//! into a worker thread by `spawn`, which then runs the poll/read loop.
//! Other threads cannot hold a reference to it; instead they push
//! `WatchCmd`s through `WatchHandle::cmd_tx` and signal the worker by
//! writing to `wake_fd` (an eventfd that sits in the worker's poll set
//! alongside the inotify FD).  This eliminates the Mutex<Watcher> deadlock
//! we'd otherwise hit when the worker is parked in `read_events` and the
//! main thread tries to `watch()` a new path.

use std::collections::BTreeMap;
use std::mem::MaybeUninit;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, Receiver, channel};

use rustix::event::{eventfd, EventfdFlags, PollFd, PollFlags};
use rustix::fs::inotify;
use rustix::io;
use thin_vec::ThinVec;

// ── Public types ──────────────────────────────────────────────────────────────

pub enum WatchEvent {
    Modified(PathBuf),
}

pub enum WatchCmd {
    Watch(PathBuf),
    Unwatch(PathBuf),
    Shutdown,
}

/// Owned by the daemon's main thread. Drop sends `Shutdown`.
pub struct WatchHandle {
    pub cmd_tx:  Sender<WatchCmd>,
    pub wake_fd: OwnedFd,
}

impl WatchHandle {
    pub fn watch(&self, p: PathBuf)   -> Result<(), std::io::Error> {
        self.send(WatchCmd::Watch(p))
    }
    pub fn unwatch(&self, p: PathBuf) -> Result<(), std::io::Error> {
        self.send(WatchCmd::Unwatch(p))
    }

    fn send(&self, cmd: WatchCmd) -> Result<(), std::io::Error> {
        self.cmd_tx.send(cmd)
            .map_err(|_| std::io::Error::other("watcher thread gone"))?;
        kick(self.wake_fd.as_fd())
    }
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        let _ = self.cmd_tx.send(WatchCmd::Shutdown);
        let _ = kick(self.wake_fd.as_fd());
    }
}

/// Spawn the watcher worker thread.  Returns the handle the main thread uses
/// to issue watch/unwatch commands, plus the receiver end of the event
/// channel the worker writes events into.
pub fn spawn() -> Result<(WatchHandle, Receiver<WatchEvent>), std::io::Error> {
    // Two fds we need: the inotify fd (owned by the worker) and an eventfd
    // (one copy in the worker's poll set, one copy in the handle for kicks).
    let watcher = Watcher::new()?;
    let wake_a  = eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK)?;
    let wake_b  = dup_fd(wake_a.as_fd())?;

    let (cmd_tx, cmd_rx) = channel::<WatchCmd>();
    let (event_tx, event_rx) = channel::<WatchEvent>();

    std::thread::spawn(move || worker(watcher, wake_a, cmd_rx, event_tx));

    Ok((WatchHandle { cmd_tx, wake_fd: wake_b }, event_rx))
}

// ── Worker ────────────────────────────────────────────────────────────────────

fn worker(
    mut watcher: Watcher,
    wake_fd:     OwnedFd,
    cmd_rx:      Receiver<WatchCmd>,
    event_tx:    Sender<WatchEvent>,
) {
    let mut pending_events: ThinVec<WatchEvent> = ThinVec::new();

    loop {
        // 1. Drain any pending commands.  Cheap try_recv loop — we always
        //    fully drain so the eventfd kick count and the channel depth stay
        //    bounded.
        let mut shutdown = false;
        loop {
            match cmd_rx.try_recv() {
                Ok(WatchCmd::Watch(p))   => { let _ = watcher.watch(&p); }
                Ok(WatchCmd::Unwatch(p)) => { watcher.unwatch(&p); }
                Ok(WatchCmd::Shutdown)   => { shutdown = true; break; }
                Err(_) => break,
            }
        }
        if shutdown { return; }
        drain_eventfd(wake_fd.as_fd());

        // 2. Read any inotify events that are already queued (non-blocking).
        pending_events.clear();
        let _ = watcher.read_events(&mut pending_events);
        for ev in pending_events.drain(..) {
            if event_tx.send(ev).is_err() { return; }
        }

        // 3. Park on poll() until inotify or wake_fd has something.
        //    Negative timeout = block indefinitely.
        let mut pollfds = [
            PollFd::new(&watcher.fd, PollFlags::IN),
            PollFd::new(&wake_fd,    PollFlags::IN),
        ];
        match rustix::event::poll(&mut pollfds, -1) {
            Ok(_) | Err(io::Errno::INTR) => {}
            Err(_) => return,
        }
    }
}

fn kick(fd: BorrowedFd<'_>) -> std::io::Result<()> {
    let buf: [u8; 8] = 1u64.to_ne_bytes();
    match rustix::io::write(fd, &buf) {
        Ok(_) | Err(io::Errno::AGAIN) => Ok(()),
        Err(e) => Err(std::io::Error::from_raw_os_error(e.raw_os_error())),
    }
}

fn drain_eventfd(fd: BorrowedFd<'_>) {
    let mut buf = [0u8; 8];
    loop {
        match rustix::io::read(fd, &mut buf) {
            Ok(_) => continue,
            Err(_) => return,
        }
    }
}

fn dup_fd(fd: BorrowedFd<'_>) -> std::io::Result<OwnedFd> {
    rustix::io::dup(fd).map_err(|e| std::io::Error::from_raw_os_error(e.raw_os_error()))
}

// ── Watcher (single-owner; lives in the worker thread) ────────────────────────

struct Watcher {
    fd:      OwnedFd,
    wds:     BTreeMap<i32, PathBuf>,
    by_path: BTreeMap<PathBuf, i32>,
    buf:     Box<[MaybeUninit<u8>; 8192]>,
}

impl Watcher {
    fn new() -> std::io::Result<Self> {
        let fd = inotify::init(inotify::CreateFlags::CLOEXEC | inotify::CreateFlags::NONBLOCK)?;
        Ok(Watcher {
            fd,
            wds:     BTreeMap::new(),
            by_path: BTreeMap::new(),
            buf:     Box::new([MaybeUninit::uninit(); 8192]),
        })
    }

    fn watch(&mut self, path: &Path) -> std::io::Result<()> {
        if self.by_path.contains_key(path) { return Ok(()); }
        let flags = inotify::WatchFlags::CLOSE_WRITE
                  | inotify::WatchFlags::MOVED_TO
                  | inotify::WatchFlags::MODIFY;
        let wd = inotify::add_watch(self.fd.as_fd(), path, flags)? as i32;
        self.wds.insert(wd, path.to_path_buf());
        self.by_path.insert(path.to_path_buf(), wd);
        Ok(())
    }

    fn unwatch(&mut self, path: &Path) {
        if let Some(wd) = self.by_path.remove(path) {
            let _ = inotify::remove_watch(self.fd.as_fd(), wd);
            self.wds.remove(&wd);
        }
    }

    fn read_events(&mut self, out: &mut ThinVec<WatchEvent>) -> std::io::Result<usize> {
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
                Err(io::Errno::AGAIN) => break,
                Err(io::Errno::INTR)  => continue,
                Err(_) => {
                    if decoded > 0 { return Ok(decoded); }
                    return Ok(0);
                }
            }
        }
        Ok(decoded)
    }
}

// AsRawFd implementation kept for future debugging hooks; not used directly.
impl Watcher {
    #[allow(dead_code)]
    fn raw_fd(&self) -> i32 { self.fd.as_raw_fd() }
}
