//! `langcd` — incremental dev daemon for `.lang` scripts.
//!
//! Reads `EngineMsg`s on stdin, watches the `.lang` files the engine asks it
//! to track via direct Linux inotify, recompiles them through the same
//! frontend + LLVM backend as `langc`, and writes `DaemonMsg::CompileOk`s
//! back to stdout whose `so_path` field points the engine at a fresh `.so`.
//!
//! Transport: stdin/stdout (length-prefixed frames, little-endian).  The
//! `script_client::ScriptClient` on the engine side spawns this binary and
//! pipes the IPC over its stdio.
//!
//! Incremental cache: per file, keyed by the FNV hash of file contents.  When
//! a file's hash matches what we've already compiled, we skip codegen and just
//! re-emit the cached `so_path`.
//!
//! All maps are sorted `ThinVec<(Arc<str>, _)>` with binary-search lookup —
//! no `BTreeMap` / `HashMap`, no `std::Vec`, no `String` field stored.

// Single source of truth for the wire protocol lives in the engine — path-
// import it here so any protocol changes happen in one place.
#[path = "../../../src/resource_manager/event_manager/script_ipc.rs"]
mod ipc;

mod watch;

use std::io::{BufReader, BufWriter, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use inkwell::OptimizationLevel;
use lang_frontend::{lexer::Lexer, parser::Parser, sema};
use thin_vec::ThinVec;

use ipc::{DaemonMsg, EngineMsg};
use watch::WatchEvent;

// Re-use langc's codegen + link modules so there's exactly one LLVM pipeline.
#[path = "../../langc/src/engine_api.rs"] mod engine_api;
#[path = "../../langc/src/codegen.rs"]    mod codegen;
#[path = "../../langc/src/link.rs"]       mod link;

// ── Sorted-array map: path → WatchEntry ──────────────────────────────────────

struct WatchEntry {
    script_id: i64,
    last_hash: u64,
    last_so:   Option<Arc<str>>,
}

/// Sorted ascending by path string; binary-search lookup.
type WatchedMap = ThinVec<(Arc<str>, WatchEntry)>;

fn map_get<'a>(m: &'a WatchedMap, p: &str) -> Option<&'a WatchEntry> {
    let i = m.partition_point(|(k, _)| k.as_ref() < p);
    m.get(i).filter(|(k, _)| k.as_ref() == p).map(|(_, v)| v)
}
fn map_get_mut<'a>(m: &'a mut WatchedMap, p: &str) -> Option<&'a mut WatchEntry> {
    let i = m.partition_point(|(k, _)| k.as_ref() < p);
    let m_at = m.get_mut(i)?;
    if m_at.0.as_ref() != p { return None; }
    Some(&mut m_at.1)
}
fn map_insert(m: &mut WatchedMap, path: Arc<str>, entry: WatchEntry) {
    let i = m.partition_point(|(k, _)| k.as_ref() < path.as_ref());
    if m.get(i).is_some_and(|(k, _)| k.as_ref() == path.as_ref()) {
        m[i].1 = entry;
    } else {
        m.insert(i, (path, entry));
    }
}
fn map_remove(m: &mut WatchedMap, p: &str) {
    let i = m.partition_point(|(k, _)| k.as_ref() < p);
    if m.get(i).is_some_and(|(k, _)| k.as_ref() == p) {
        m.remove(i);
    }
}

// ── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    let stdout = std::io::stdout();
    let writer = std::sync::Mutex::new(BufWriter::new(stdout.lock()));

    let mut watched: WatchedMap = ThinVec::new();

    let (watch_handle, fs_rx) = match watch::spawn() {
        Ok(p) => p,
        Err(e) => { eprintln!("langcd: inotify init: {e}"); return; }
    };

    eprintln!("langcd: ready (pid {})", std::process::id());

    let (tx, rx): (Sender<DaemonEvent>, Receiver<DaemonEvent>) = channel();

    // FS forwarder thread — converts fs_rx into the unified `tx` stream.
    {
        let tx = tx.clone();
        std::thread::spawn(move || {
            while let Ok(ev) = fs_rx.recv() {
                if tx.send(DaemonEvent::Fs(ev)).is_err() { break; }
            }
        });
    }

    // Engine IPC reader thread.
    let tx_io = tx.clone();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut reader = BufReader::new(stdin);
        loop {
            match ipc::read_engine_msg(&mut reader) {
                Ok(m)  => if tx_io.send(DaemonEvent::Engine(m)).is_err() { break; },
                Err(e) => {
                    if matches!(e.kind(), std::io::ErrorKind::UnexpectedEof) { break; }
                    eprintln!("langcd: ipc read: {e}");
                    break;
                }
            }
        }
        let _ = tx_io.send(DaemonEvent::Engine(EngineMsg::Shutdown));
    });

    // Main loop.
    while let Ok(ev) = rx.recv() {
        match ev {
            DaemonEvent::Engine(EngineMsg::Shutdown) => break,
            DaemonEvent::Engine(EngineMsg::Watch { script_id, path }) => {
                let was_watched = map_get(&watched, path.as_ref()).is_some();
                if !was_watched {
                    if let Err(e) = watch_handle.watch(Arc::clone(&path)) {
                        eprintln!("langcd: watch({}) failed: {e}", path);
                        continue;
                    }
                }
                map_insert(&mut watched, Arc::clone(&path), WatchEntry {
                    script_id, last_hash: 0, last_so: None,
                });
                handle_compile(path, script_id, &mut watched, &writer);
            }
            DaemonEvent::Engine(EngineMsg::Unwatch { script_id }) => {
                let mut to_drop: ThinVec<Arc<str>> = ThinVec::new();
                for (p, v) in watched.iter() {
                    if v.script_id == script_id { to_drop.push(Arc::clone(p)); }
                }
                for p in to_drop.iter() {
                    let _ = watch_handle.unwatch(Arc::clone(p));
                    map_remove(&mut watched, p.as_ref());
                }
            }
            DaemonEvent::Fs(WatchEvent::Modified(p)) => {
                // Coalesce by canonical path so editors that rewrite via
                // tmp+rename collapse onto the watched entry.  Both sides go
                // through `canonicalize` and we compare the resulting strs.
                let target_canon = canon_str(p.as_ref());
                let mut matched: Option<Arc<str>> = None;
                for (k, _) in watched.iter() {
                    if canon_str(k.as_ref()).as_deref() == target_canon.as_deref() {
                        matched = Some(Arc::clone(k));
                        break;
                    }
                }
                if let Some(k) = matched {
                    if let Some(id) = map_get(&watched, k.as_ref()).map(|v| v.script_id) {
                        handle_compile(k, id, &mut watched, &writer);
                    }
                }
            }
        }
    }

    drop(watch_handle);
    eprintln!("langcd: exiting");
}

enum DaemonEvent {
    Engine(EngineMsg),
    Fs(WatchEvent),
}

fn handle_compile(
    path:    Arc<str>,
    script_id: i64,
    watched: &mut WatchedMap,
    writer:  &std::sync::Mutex<BufWriter<std::io::StdoutLock<'_>>>,
) {
    let src = match std::fs::read_to_string(path.as_ref()) {
        Ok(s) => s,
        Err(e) => {
            let mut diags: ThinVec<Arc<str>> = ThinVec::new();
            diags.push(Arc::<str>::from(format!("{path}: read: {e}").as_str()));
            send_err(writer, script_id, &diags);
            return;
        }
    };
    let hash = sema::fnv1a(src.as_bytes());

    // If the file content hasn't changed since the last compile, re-emit the
    // existing .so path so the engine can decide whether to reload.
    if let Some(entry) = map_get_mut(watched, path.as_ref()) {
        if entry.last_hash == hash {
            if let Some(so) = entry.last_so.clone() {
                let (ss, sv) = probe_so(so.as_ref()).unwrap_or((0, 0));
                send_ok(writer, script_id, so, ss, sv);
                return;
            }
        }
        entry.last_hash = hash;
    }

    let mut diags: ThinVec<Arc<str>> = ThinVec::new();
    let toks = match Lexer::new(&src).tokenise() {
        Ok(t) => t,
        Err(e) => { push_err(&mut diags, e); send_err(writer, script_id, &diags); return; }
    };
    let ast = match Parser::new(toks).parse_script() {
        Ok(a) => a,
        Err(e) => { push_err(&mut diags, e); send_err(writer, script_id, &diags); return; }
    };
    let hir = match sema::lower(ast) {
        Ok(h) => h,
        Err(e) => { push_err(&mut diags, e); send_err(writer, script_id, &diags); return; }
    };

    // Per-script temp directory.  PathBuf used as a local convenience for
    // the std::fs API surface — never stored.
    let dir = std::env::temp_dir()
        .join("dfe_langcd_cache")
        .join(format!("script_{script_id}"));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        diags.push(Arc::<str>::from(format!("mkdir {}: {e}", dir.display()).as_str()));
        send_err(writer, script_id, &diags);
        return;
    }
    let obj = dir.join(format!("{}.o", hir.name));
    let so  = dir.join(format!("{}.so", hir.name));

    if let Err(e) = codegen::compile_to_object(&hir, OptimizationLevel::Less, &obj) {
        diags.push(Arc::<str>::from(format!("{e}").as_str()));
        send_err(writer, script_id, &diags);
        return;
    }
    if let Err(e) = link::link_shared(&obj, &so) {
        diags.push(Arc::<str>::from(format!("{e}").as_str()));
        send_err(writer, script_id, &diags);
        return;
    }
    let _ = std::fs::remove_file(&obj);

    let ss = hir.state_size;
    let sv = hir.state_version;
    let so_arc: Arc<str> = Arc::<str>::from(so.to_string_lossy().as_ref());
    if let Some(entry) = map_get_mut(watched, path.as_ref()) {
        entry.last_so = Some(Arc::clone(&so_arc));
    }
    send_ok(writer, script_id, so_arc, ss, sv);
}

fn push_err<E: core::fmt::Display>(diags: &mut ThinVec<Arc<str>>, e: E) {
    diags.push(Arc::<str>::from(format!("{e}").as_str()));
}

fn canon_str(p: &str) -> Option<Arc<str>> {
    std::fs::canonicalize(p).ok()
        .map(|cp| Arc::<str>::from(cp.to_string_lossy().as_ref()))
}

fn probe_so(path: &str) -> Option<(u32, u32)> {
    let lib = unsafe { libloading::Library::new(Path::new(path)).ok()? };
    let ss: libloading::Symbol<unsafe extern "C" fn() -> u32> =
        unsafe { lib.get(b"df_state_size\0") }.ok()?;
    let sv: libloading::Symbol<unsafe extern "C" fn() -> u32> =
        unsafe { lib.get(b"df_state_version\0") }.ok()?;
    Some(( unsafe { ss() }, unsafe { sv() } ))
}

fn send_ok(
    writer: &std::sync::Mutex<BufWriter<std::io::StdoutLock<'_>>>,
    script_id: i64,
    so_path: Arc<str>,
    state_size: u32,
    state_version: u32,
) {
    let msg = DaemonMsg::CompileOk { script_id, so_path, state_size, state_version };
    let mut w = writer.lock().unwrap();
    let _ = ipc::write_daemon_msg(&mut *w, &msg);
    let _ = w.flush();
}

fn send_err(
    writer: &std::sync::Mutex<BufWriter<std::io::StdoutLock<'_>>>,
    script_id: i64,
    diags: &ThinVec<Arc<str>>,
) {
    let mut copy: ThinVec<Arc<str>> = ThinVec::with_capacity(diags.len());
    for d in diags { copy.push(Arc::clone(d)); }
    let msg = DaemonMsg::CompileErr { script_id, diagnostics: copy };
    let mut w = writer.lock().unwrap();
    let _ = ipc::write_daemon_msg(&mut *w, &msg);
    let _ = w.flush();
}
