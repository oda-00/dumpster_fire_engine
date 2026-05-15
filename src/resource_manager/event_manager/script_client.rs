//! Engine-side client for `langcd`.
//!
//! Spawns the daemon as a subprocess, pipes the IPC over its stdin/stdout,
//! and exposes a synchronous `compile` API plus an async `poll_event` method
//! the engine drains at tick boundaries.
//!
//! Steady-state operation is allocation-free — the client owns a reusable
//! `ThinVec<u8>` framing buffer (inside `script_ipc`'s read/write helpers).

use std::io::{BufReader, BufWriter, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

use thin_vec::ThinVec;

use super::script_ipc::{DaemonMsg, EngineMsg, read_daemon_msg, write_engine_msg};

#[derive(Debug)]
pub enum ScriptClientError {
    Spawn(std::io::Error),
    Io(std::io::Error),
    DaemonDied,
}

impl core::fmt::Display for ScriptClientError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ScriptClientError::Spawn(e)     => write!(f, "spawn langcd: {e}"),
            ScriptClientError::Io(e)        => write!(f, "ipc io: {e}"),
            ScriptClientError::DaemonDied   => write!(f, "langcd exited unexpectedly"),
        }
    }
}

/// Persistent client over a spawned `langcd` process.
pub struct ScriptClient {
    child:  Child,
    stdin:  BufWriter<ChildStdin>,
    /// Inbound queue of decoded daemon messages, populated by a background reader thread.
    rx:     Receiver<DaemonMsg>,
    /// Snapshot of received messages drained out of `rx` by `drain_pending`.
    pending: ThinVec<DaemonMsg>,
}

impl ScriptClient {
    /// Spawn `langcd_path` and bind to its stdin/stdout.
    pub fn spawn(langcd_path: &Path) -> Result<Self, ScriptClientError> {
        let mut child = Command::new(langcd_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(ScriptClientError::Spawn)?;

        let stdin  = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stdin  = BufWriter::new(stdin);

        let (tx, rx) = channel();
        std::thread::spawn(move || reader_loop(stdout, tx));

        Ok(ScriptClient { child, stdin, rx, pending: ThinVec::new() })
    }

    /// Ask the daemon to watch `path` under `script_id`.  The first
    /// compilation result arrives via `poll_event` / `drain_pending`.
    pub fn watch(&mut self, script_id: i64, path: Arc<str>) -> Result<(), ScriptClientError> {
        write_engine_msg(&mut self.stdin, &EngineMsg::Watch { script_id, path })
            .map_err(ScriptClientError::Io)?;
        self.stdin.flush().map_err(ScriptClientError::Io)
    }

    pub fn unwatch(&mut self, script_id: i64) -> Result<(), ScriptClientError> {
        write_engine_msg(&mut self.stdin, &EngineMsg::Unwatch { script_id })
            .map_err(ScriptClientError::Io)?;
        self.stdin.flush().map_err(ScriptClientError::Io)
    }

    pub fn shutdown(&mut self) -> Result<(), ScriptClientError> {
        let _ = write_engine_msg(&mut self.stdin, &EngineMsg::Shutdown);
        let _ = self.stdin.flush();
        let _ = self.child.wait();
        Ok(())
    }

    /// Non-blocking poll — returns the next daemon message, if any.
    pub fn poll_event(&mut self) -> Option<DaemonMsg> {
        if let Some(m) = self.pending.pop() { return Some(m); }
        match self.rx.try_recv() {
            Ok(m) => Some(m),
            Err(_) => None,
        }
    }

    /// Blocking wait up to `timeout` for the next message.
    pub fn wait_for_event(&mut self, timeout: std::time::Duration) -> Option<DaemonMsg> {
        if let Some(m) = self.pending.pop() { return Some(m); }
        self.rx.recv_timeout(timeout).ok()
    }

    /// Drain every queued message into `self.pending` (for batch processing
    /// at a tick boundary).
    pub fn drain_pending(&mut self) -> &mut ThinVec<DaemonMsg> {
        while let Ok(m) = self.rx.try_recv() {
            self.pending.push(m);
        }
        &mut self.pending
    }
}

impl Drop for ScriptClient {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn reader_loop(stdout: ChildStdout, tx: std::sync::mpsc::Sender<DaemonMsg>) {
    let mut r = BufReader::new(stdout);
    loop {
        match read_daemon_msg(&mut r) {
            Ok(m)  => if tx.send(m).is_err() { break; }
            Err(e) => {
                if matches!(e.kind(), std::io::ErrorKind::UnexpectedEof) { break; }
                eprintln!("ScriptClient reader: {e}");
                break;
            }
        }
    }
}
