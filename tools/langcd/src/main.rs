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

// Single source of truth for the wire protocol lives in the engine — path-
// import it here so any protocol changes happen in one place.
#[path = "../../../src/resource_manager/event_manager/script_ipc.rs"]
mod ipc;

mod watch;

use std::collections::BTreeMap;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use inkwell::OptimizationLevel;
use lang_frontend::{lexer::Lexer, parser::Parser, sema};
use thin_vec::ThinVec;

use ipc::{DaemonMsg, EngineMsg};
use watch::{Watcher, WatchEvent};

// Re-use langc's codegen + link modules so there's exactly one LLVM pipeline.
#[path = "../../langc/src/engine_api.rs"] mod engine_api;
#[path = "../../langc/src/codegen.rs"]    mod codegen;
#[path = "../../langc/src/link.rs"]       mod link;

fn main() {
    let stdout  = std::io::stdout();
    let writer  = std::sync::Mutex::new(BufWriter::new(stdout.lock()));

    // path → (script_id, last_content_hash, last_so_path)
    let mut watched: BTreeMap<PathBuf, WatchEntry> = BTreeMap::new();

    let watcher = std::sync::Arc::new(std::sync::Mutex::new(
        match Watcher::new() {
            Ok(w) => w,
            Err(e) => { eprintln!("langcd: inotify init: {e}"); return; }
        }
    ));

    eprintln!("langcd: ready (pid {})", std::process::id());

    // Multiplexed event channel: stdin IPC + inotify events.
    let (tx, rx): (Sender<DaemonEvent>, Receiver<DaemonEvent>) = channel();

    // FS reader thread: blocking read() on inotify FD, decode, forward.
    {
        let watcher = std::sync::Arc::clone(&watcher);
        let tx = tx.clone();
        std::thread::spawn(move || {
            let mut buf: ThinVec<WatchEvent> = ThinVec::new();
            loop {
                buf.clear();
                let r = { watcher.lock().unwrap().read_events(&mut buf) };
                match r {
                    Ok(_) => for ev in buf.drain(..) {
                        if tx.send(DaemonEvent::Fs(ev)).is_err() { return; }
                    }
                    Err(e) => {
                        eprintln!("langcd: inotify read: {e}");
                        return;
                    }
                }
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
                let pp = PathBuf::from(path.as_ref());
                let was_watched = watched.contains_key(&pp);
                if !was_watched {
                    if let Err(e) = watcher.lock().unwrap().watch(&pp) {
                        eprintln!("langcd: watch({}) failed: {e}", pp.display());
                        continue;
                    }
                }
                watched.insert(pp.clone(), WatchEntry {
                    script_id, last_hash: 0, last_so: None,
                });
                handle_compile(&pp, script_id, &mut watched, &writer);
            }
            DaemonEvent::Engine(EngineMsg::Unwatch { script_id }) => {
                let to_drop: Vec<PathBuf> = watched.iter()
                    .filter(|(_, v)| v.script_id == script_id)
                    .map(|(p, _)| p.clone())
                    .collect();
                for p in &to_drop {
                    watcher.lock().unwrap().unwatch(p);
                    watched.remove(p);
                }
            }
            DaemonEvent::Fs(WatchEvent::Modified(p)) => {
                // Coalesce by canonical path for editors that rewrite via tmp+rename.
                let canon = std::fs::canonicalize(&p).unwrap_or(p.clone());
                let key = watched.keys()
                    .find(|k| std::fs::canonicalize(k).unwrap_or_else(|_| k.to_path_buf()) == canon)
                    .cloned();
                if let Some(k) = key {
                    let id = watched[&k].script_id;
                    handle_compile(&k, id, &mut watched, &writer);
                }
            }
        }
    }

    eprintln!("langcd: exiting");
}

enum DaemonEvent {
    Engine(EngineMsg),
    Fs(WatchEvent),
}

struct WatchEntry {
    script_id: i64,
    last_hash: u64,
    last_so:   Option<PathBuf>,
}

fn handle_compile(
    path: &Path,
    script_id: i64,
    watched: &mut BTreeMap<PathBuf, WatchEntry>,
    writer: &std::sync::Mutex<BufWriter<std::io::StdoutLock<'_>>>,
) {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            send_err(writer, script_id, &[format!("{}: read: {e}", path.display())]);
            return;
        }
    };
    let hash = sema::fnv1a(src.as_bytes());

    // If the file content hasn't changed since the last compile, re-emit the
    // existing .so path so the engine can decide whether to reload.
    if let Some(entry) = watched.get_mut(path) {
        if entry.last_hash == hash && entry.last_so.is_some() {
            let so = entry.last_so.clone().unwrap();
            let (ss, sv) = probe_so(&so).unwrap_or((0, 0));
            send_ok(writer, script_id, &so.to_string_lossy(), ss, sv);
            return;
        }
        entry.last_hash = hash;
    }

    let mut diags: ThinVec<Arc<str>> = ThinVec::new();
    let toks = match Lexer::new(&src).tokenise() {
        Ok(t) => t,
        Err(e) => { diags.push(Arc::from(format!("{e}"))); send_err_th(writer, script_id, &diags); return; }
    };
    let ast = match Parser::new(toks).parse_script() {
        Ok(a) => a,
        Err(e) => { diags.push(Arc::from(format!("{e}"))); send_err_th(writer, script_id, &diags); return; }
    };
    let hir = match sema::lower(ast) {
        Ok(h) => h,
        Err(e) => { diags.push(Arc::from(format!("{e}"))); send_err_th(writer, script_id, &diags); return; }
    };

    // Emit to a per-script temp directory so successive recompilations don't
    // race each other's `.o`/`.so` files.
    let dir = std::env::temp_dir()
        .join("dfe_langcd_cache")
        .join(format!("script_{script_id}"));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        diags.push(Arc::from(format!("mkdir {}: {e}", dir.display())));
        send_err_th(writer, script_id, &diags);
        return;
    }
    let obj = dir.join(format!("{}.o", hir.name));
    let so  = dir.join(format!("{}.so", hir.name));

    if let Err(e) = codegen::compile_to_object(&hir, OptimizationLevel::Less, &obj) {
        diags.push(Arc::from(format!("{e}"))); send_err_th(writer, script_id, &diags); return;
    }
    if let Err(e) = link::link_shared(&obj, &so) {
        diags.push(Arc::from(format!("{e}"))); send_err_th(writer, script_id, &diags); return;
    }
    let _ = std::fs::remove_file(&obj);

    let ss = hir.state_size;
    let sv = hir.state_version;
    if let Some(entry) = watched.get_mut(path) {
        entry.last_so = Some(so.clone());
    }
    send_ok(writer, script_id, &so.to_string_lossy(), ss, sv);
}

fn probe_so(path: &Path) -> Option<(u32, u32)> {
    let lib = unsafe { libloading::Library::new(path).ok()? };
    let ss: libloading::Symbol<unsafe extern "C" fn() -> u32> =
        unsafe { lib.get(b"df_state_size\0") }.ok()?;
    let sv: libloading::Symbol<unsafe extern "C" fn() -> u32> =
        unsafe { lib.get(b"df_state_version\0") }.ok()?;
    Some(( unsafe { ss() }, unsafe { sv() } ))
}

fn send_ok(
    writer: &std::sync::Mutex<BufWriter<std::io::StdoutLock<'_>>>,
    script_id: i64,
    so_path: &str,
    state_size: u32,
    state_version: u32,
) {
    let msg = DaemonMsg::CompileOk {
        script_id,
        so_path: Arc::from(so_path),
        state_size, state_version,
    };
    let mut w = writer.lock().unwrap();
    let _ = ipc::write_daemon_msg(&mut *w, &msg);
    let _ = w.flush();
}

fn send_err(
    writer: &std::sync::Mutex<BufWriter<std::io::StdoutLock<'_>>>,
    script_id: i64,
    diags: &[String],
) {
    let mut tv: ThinVec<Arc<str>> = ThinVec::new();
    for d in diags { tv.push(Arc::from(d.as_str())); }
    send_err_th(writer, script_id, &tv);
}

fn send_err_th(
    writer: &std::sync::Mutex<BufWriter<std::io::StdoutLock<'_>>>,
    script_id: i64,
    diags: &ThinVec<Arc<str>>,
) {
    let mut copy: ThinVec<Arc<str>> = ThinVec::with_capacity(diags.len());
    for d in diags { copy.push(d.clone()); }
    let msg = DaemonMsg::CompileErr { script_id, diagnostics: copy };
    let mut w = writer.lock().unwrap();
    let _ = ipc::write_daemon_msg(&mut *w, &msg);
    let _ = w.flush();
}
