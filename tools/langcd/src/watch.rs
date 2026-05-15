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
//!
//! Path bookkeeping uses sorted `ThinVec<(Arc<str>, _)>` with binary-search
//! lookup — no `BTreeMap`, no `HashMap`, matching the engine-wide invariant.

use std::mem::MaybeUninit;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Sender, Receiver, channel};

use rustix::event::{eventfd, EventfdFlags, PollFd, PollFlags};
use rustix::fs::inotify;
use rustix::io;
use thin_vec::ThinVec;

// ── Public types ──────────────────────────────────────────────────────────────

pub enum WatchEvent {
    Modified(Arc<str>),
}

pub enum WatchCmd {
    Watch(Arc<str>),
    Unwatch(Arc<str>),
    Shutdown,
}

/// Owned by the daemon's main thread. Drop sends `Shutdown`.
pub struct WatchHandle {
    pub cmd_tx:  Sender<WatchCmd>,
    pub wake_fd: OwnedFd,
}

impl WatchHandle {
    pub fn watch(&self, p: Arc<str>)   -> Result<(), std::io::Error> {
        self.send(WatchCmd::Watch(p))
    }
    pub fn unwatch(&self, p: Arc<str>) -> Result<(), std::io::Error> {
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
                Ok(WatchCmd::Watch(p))   => { let _ = watcher.watch(p); }
                Ok(WatchCmd::Unwatch(p)) => { watcher.unwatch(p.as_ref()); }
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
    /// Sorted ascending by `wd`; binary-search lookup on inotify-event decode.
    wds:     ThinVec<(i32, Arc<str>)>,
    /// Sorted ascending by path; binary-search lookup on watch / unwatch.
    by_path: ThinVec<(Arc<str>, i32)>,
    buf:     Box<[MaybeUninit<u8>; 8192]>,
}

impl Watcher {
    fn new() -> std::io::Result<Self> {
        let fd = inotify::init(inotify::CreateFlags::CLOEXEC | inotify::CreateFlags::NONBLOCK)?;
        Ok(Watcher {
            fd,
            wds:     ThinVec::new(),
            by_path: ThinVec::new(),
            buf:     Box::new([MaybeUninit::uninit(); 8192]),
        })
    }

    fn lookup_by_path(&self, p: &str) -> Option<i32> {
        let i = self.by_path.partition_point(|(k, _)| k.as_ref() < p);
        self.by_path.get(i)
            .filter(|(k, _)| k.as_ref() == p)
            .map(|(_, wd)| *wd)
    }

    fn lookup_by_wd(&self, wd: i32) -> Option<&Arc<str>> {
        let i = self.wds.partition_point(|(k, _)| *k < wd);
        self.wds.get(i)
            .filter(|(k, _)| *k == wd)
            .map(|(_, p)| p)
    }

    fn watch(&mut self, path: Arc<str>) -> std::io::Result<()> {
        if self.lookup_by_path(path.as_ref()).is_some() { return Ok(()); }
        let flags = inotify::WatchFlags::CLOSE_WRITE
                  | inotify::WatchFlags::MOVED_TO
                  | inotify::WatchFlags::MODIFY;
        let wd = inotify::add_watch(self.fd.as_fd(), Path::new(path.as_ref()), flags)? as i32;

        let pp = self.wds.partition_point(|(k, _)| *k < wd);
        self.wds.insert(pp, (wd, path.clone()));
        let qp = self.by_path.partition_point(|(k, _)| k.as_ref() < path.as_ref());
        self.by_path.insert(qp, (path, wd));
        Ok(())
    }

    fn unwatch(&mut self, path: &str) {
        let qp = self.by_path.partition_point(|(k, _)| k.as_ref() < path);
        let Some(entry) = self.by_path.get(qp) else { return };
        if entry.0.as_ref() != path { return; }
        let wd = entry.1;
        self.by_path.remove(qp);
        let _ = inotify::remove_watch(self.fd.as_fd(), wd);
        let pp = self.wds.partition_point(|(k, _)| *k < wd);
        if self.wds.get(pp).is_some_and(|(k, _)| *k == wd) {
            self.wds.remove(pp);
        }
    }

    fn read_events(&mut self, out: &mut ThinVec<WatchEvent>) -> std::io::Result<usize> {
        // Two passes: pass 1 collects wds (Reader holds &mut self.buf, so we
        // can't touch the watcher's tables during it).  Pass 2 resolves wds
        // to paths via binary search once the Reader is dropped.
        let mut wd_buf: ThinVec<i32> = ThinVec::new();
        {
            let mut reader = inotify::Reader::new(self.fd.as_fd(), &mut self.buf[..]);
            loop {
                match reader.next() {
                    Ok(ev) => wd_buf.push(ev.wd()),
                    Err(io::Errno::AGAIN) => break,
                    Err(io::Errno::INTR)  => continue,
                    Err(_) => {
                        if !wd_buf.is_empty() { break; }
                        return Ok(0);
                    }
                }
            }
        }
        let mut decoded = 0;
        for wd in wd_buf.iter() {
            if let Some(path) = self.lookup_by_wd(*wd) {
                out.push(WatchEvent::Modified(Arc::clone(path)));
                decoded += 1;
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
