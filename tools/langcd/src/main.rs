//! `langcd` — incremental dev daemon for `.lang` scripts.
//!
//! Reads `EngineMsg`s on stdin, watches the `.lang` files the engine asks it
//! to track via direct Linux inotify, recompiles them through the same
//! frontend + LLVM backend as `langc`, and writes `DaemonMsg::CompileOk`s
//! back to stdout whose `o_path` field points the engine at a fresh `.o`.
//!
//! No linker step — the engine's custom object loader maps the `.o` directly.
//! This eliminates the ~23 ms link latency on every hot-reload.
//!
//! Transport: stdin/stdout (length-prefixed frames, little-endian).  The
//! `script_client::ScriptClient` on the engine side spawns this binary and
//! pipes the IPC over its stdio.
//!
//! Incremental cache: per file, keyed by the FNV hash of file contents.  When
//! a file's hash matches what we've already compiled, we skip codegen and just
//! re-emit the cached `o_path`.
//!
//! All maps are sorted `ThinVec<(Arc<str>, _)>` with binary-search lookup —
//! no `BTreeMap` / `HashMap`, no `std::Vec`, no `String` field stored.

// Single source of truth for the wire protocol lives in the engine — path-
// import it here so any protocol changes happen in one place.
#[path = "../../../src/resource_manager/event_manager/script_ipc.rs"]
mod ipc;

mod watch;

use std::io::{BufReader, BufWriter, Write};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use inkwell::OptimizationLevel;
use lang_frontend::{lexer::Lexer, parser::Parser, sema};
use thin_vec::ThinVec;

use ipc::{DaemonMsg, EngineMsg};
use watch::WatchEvent;

// Re-use langc's codegen module so there's exactly one LLVM pipeline.
#[path = "../../langc/src/engine_api.rs"] mod engine_api;
#[path = "../../langc/src/codegen.rs"]    mod codegen;

// ── Sorted-array map: path → WatchEntry ──────────────────────────────────────

struct WatchEntry {
    script_id:          i64,
    last_hash:          u64,
    last_o:             Option<Arc<str>>,
    last_state_size:    u32,
    last_state_version: u32,
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
    //
    // Compile work is dispatched onto rayon's thread pool so multiple file
    // changes (e.g. a save-all in the editor, or a git-branch switch) compile
    // concurrently instead of serializing on a single thread.  The main loop
    // here owns `watched` and `writer` exclusively — worker threads only
    // touch local data and post their results back through the event channel
    // as `DaemonEvent::CompileDone`.
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
                    script_id, last_hash: 0, last_o: None,
                    last_state_size: 0, last_state_version: 0,
                });
                let (lh, lo, lss, lsv) = (0u64, None, 0u32, 0u32);
                dispatch_compile(path, script_id, lh, lo, lss, lsv, tx.clone());
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
                if let Some(k) = matched
                    && let Some(entry) = map_get(&watched, k.as_ref())
                {
                    let (id, lh, lo, lss, lsv) = (
                        entry.script_id, entry.last_hash, entry.last_o.clone(),
                        entry.last_state_size, entry.last_state_version,
                    );
                    dispatch_compile(k, id, lh, lo, lss, lsv, tx.clone());
                }
            }
            DaemonEvent::CompileDone { path, script_id, result } => {
                apply_compile_outcome(path, script_id, result, &mut watched, &writer);
            }
        }
    }

    drop(watch_handle);
    eprintln!("langcd: exiting");
}

enum DaemonEvent {
    Engine(EngineMsg),
    Fs(WatchEvent),
    /// A worker thread finished a compile job.  The main thread folds the
    /// result back into the watched-cache and writes the IPC reply.
    CompileDone {
        path:      Arc<str>,
        script_id: i64,
        result:    CompileOutcome,
    },
}

enum CompileOutcome {
    /// File content hash matched the cache — reuse the prior `.o`.
    Cached {
        o_path:        Arc<str>,
        state_size:    u32,
        state_version: u32,
    },
    /// Fresh compile succeeded.  `hash` is the new content hash to cache.
    Compiled {
        hash:          u64,
        o_path:        Arc<str>,
        state_size:    u32,
        state_version: u32,
    },
    /// Frontend, IO, or codegen failure.  `diagnostics` is the wire-form list.
    Failed { diagnostics: ThinVec<Arc<str>> },
}

/// Dispatch a compile to rayon's pool.  The worker thread does all of the
/// blocking work (file read, hash, lex/parse/sema, LLVM codegen, .o emit)
/// and posts the outcome back through the main event channel so cache
/// updates and stdout writes stay single-threaded.
fn dispatch_compile(
    path:        Arc<str>,
    script_id:   i64,
    last_hash:   u64,
    last_o:      Option<Arc<str>>,
    last_ss:     u32,
    last_sv:     u32,
    result_tx:   Sender<DaemonEvent>,
) {
    rayon::spawn(move || {
        let outcome = compile_worker(&path, script_id, last_hash, last_o, last_ss, last_sv);
        let _ = result_tx.send(DaemonEvent::CompileDone { path, script_id, result: outcome });
    });
}

/// CPU-bound worker function — runs on a rayon thread, no `&mut` engine
/// state, no IO except the file read and `.o` write that the user already
/// paid for in the synchronous version.
fn compile_worker(
    path:      &Arc<str>,
    script_id: i64,
    last_hash: u64,
    last_o:    Option<Arc<str>>,
    last_ss:   u32,
    last_sv:   u32,
) -> CompileOutcome {
    let src = match std::fs::read_to_string(path.as_ref()) {
        Ok(s) => s,
        Err(e) => {
            let mut d: ThinVec<Arc<str>> = ThinVec::new();
            d.push(Arc::<str>::from(format!("{path}: read: {e}").as_str()));
            return CompileOutcome::Failed { diagnostics: d };
        }
    };
    let hash = sema::fnv1a(src.as_bytes());
    if hash == last_hash && last_hash != 0 {
        if let Some(o) = last_o {
            return CompileOutcome::Cached {
                o_path: o, state_size: last_ss, state_version: last_sv,
            };
        }
    }

    let mut diags: ThinVec<Arc<str>> = ThinVec::new();
    let toks = match Lexer::new(&src).tokenise() {
        Ok(t) => t,
        Err(e) => { push_err(&mut diags, e); return CompileOutcome::Failed { diagnostics: diags }; }
    };
    let ast = match Parser::new(toks).parse_script() {
        Ok(a) => a,
        Err(e) => { push_err(&mut diags, e); return CompileOutcome::Failed { diagnostics: diags }; }
    };
    let hir = match sema::lower(ast) {
        Ok(h) => h,
        Err(e) => { push_err(&mut diags, e); return CompileOutcome::Failed { diagnostics: diags }; }
    };

    let dir = std::env::temp_dir()
        .join("dfe_langcd_cache")
        .join(format!("script_{script_id}"));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        diags.push(Arc::<str>::from(format!("mkdir {}: {e}", dir.display()).as_str()));
        return CompileOutcome::Failed { diagnostics: diags };
    }
    let obj = dir.join(format!("{}.o", hir.name));

    if let Err(e) = codegen::compile_to_object(&hir, OptimizationLevel::Less, &obj) {
        diags.push(Arc::<str>::from(format!("{e}").as_str()));
        return CompileOutcome::Failed { diagnostics: diags };
    }

    CompileOutcome::Compiled {
        hash,
        o_path:        Arc::<str>::from(obj.to_string_lossy().as_ref()),
        state_size:    hir.state_size,
        state_version: hir.state_version,
    }
}

/// Apply a worker's outcome to the watched-cache and ship the IPC reply.
/// Stays on the main thread — `watched` is single-owner, `writer` is the
/// shared stdout mutex.
fn apply_compile_outcome(
    path:      Arc<str>,
    script_id: i64,
    outcome:   CompileOutcome,
    watched:   &mut WatchedMap,
    writer:    &std::sync::Mutex<BufWriter<std::io::StdoutLock<'_>>>,
) {
    match outcome {
        CompileOutcome::Cached { o_path, state_size, state_version } => {
            send_ok(writer, script_id, o_path, state_size, state_version);
        }
        CompileOutcome::Compiled { hash, o_path, state_size, state_version } => {
            if let Some(entry) = map_get_mut(watched, path.as_ref()) {
                entry.last_hash          = hash;
                entry.last_o             = Some(Arc::clone(&o_path));
                entry.last_state_size    = state_size;
                entry.last_state_version = state_version;
            }
            send_ok(writer, script_id, o_path, state_size, state_version);
        }
        CompileOutcome::Failed { diagnostics } => {
            send_err(writer, script_id, &diagnostics);
        }
    }
}

fn push_err<E: core::fmt::Display>(diags: &mut ThinVec<Arc<str>>, e: E) {
    diags.push(Arc::<str>::from(format!("{e}").as_str()));
}

fn canon_str(p: &str) -> Option<Arc<str>> {
    std::fs::canonicalize(p).ok()
        .map(|cp| Arc::<str>::from(cp.to_string_lossy().as_ref()))
}

fn send_ok(
    writer: &std::sync::Mutex<BufWriter<std::io::StdoutLock<'_>>>,
    script_id: i64,
    o_path: Arc<str>,
    state_size: u32,
    state_version: u32,
) {
    let msg = DaemonMsg::CompileOk { script_id, o_path, state_size, state_version };
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
