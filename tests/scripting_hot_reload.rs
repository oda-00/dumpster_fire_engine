//! Hot-reload through `langcd`.
//!
//! Spawns `langcd` as a subprocess via the engine's `ScriptClient`, asks it
//! to watch a `.lang` file, then mutates the file on disk and verifies that
//! the daemon sends a fresh `CompileOk` whose `state_version` reflects the
//! new layout.  Loads the new `.so` and asserts the live behaviour changed.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use dumpster_fire_engine::resource_manager::event_manager::script::ScriptManager;
use dumpster_fire_engine::resource_manager::event_manager::script_abi::{
    EffectSink, engine_api_for_sink, effect_kind,
};
use dumpster_fire_engine::resource_manager::event_manager::script_client::ScriptClient;
use dumpster_fire_engine::resource_manager::event_manager::script_ipc::DaemonMsg;

const SCRIPT_V1: &str = r#"
script "evolve" {
    state {
        counter: i32 = 0
    }
    scene only {
        on_enter => cue_troupe("v1");
        behavior {
            action patrol_path()
        }
    }
}
"#;

const SCRIPT_V2: &str = r#"
script "evolve" {
    state {
        counter:   i32 = 0
        new_field: f64 = 3.14
    }
    scene only {
        on_enter => cue_troupe("v2");
        behavior {
            action attack()
        }
    }
}
"#;

fn build_release(pkg: &str) -> PathBuf {
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from).unwrap_or_else(|| std::env::current_dir().unwrap());
    let status = Command::new("cargo")
        .args(["build", "-p", pkg, "--release"])
        .current_dir(&manifest).status().unwrap();
    assert!(status.success(), "cargo build -p {pkg} failed");
    manifest.join(format!("target/release/{pkg}"))
}

#[test]
fn daemon_hot_reload_round_trip() {
    let langcd = build_release("langcd");
    let _      = build_release("langc"); // ensures the same compiler is fresh

    let dir = std::env::temp_dir().join(format!("dfe_hr_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("evolve.lang");
    std::fs::write(&src, SCRIPT_V1).unwrap();

    let mut client = ScriptClient::spawn(&langcd).expect("spawn langcd");
    client.watch(1, Arc::from(src.to_string_lossy().as_ref())).expect("watch");

    // ── First compile (v1) ──────────────────────────────────────────────────
    let so1 = wait_compile_ok(&mut client, 1);
    let state_v1 = state_version_of(&so1);
    let size_v1  = state_size_of(&so1);
    assert_eq!(size_v1, 4, "v1 state is just i32");

    // Load + run via ScriptManager: on_enter emits cue_troupe("v1").
    {
        let mut mgr = ScriptManager::new();
        let id = mgr.load_from_file(so1.clone()).expect("load v1");
        let entry = mgr.get_entry_points(id).unwrap();
        let n = entry.state_size() as usize;
        let mut state: thin_vec::ThinVec<u8> = thin_vec::ThinVec::with_capacity(n);
        state.resize(n, 0);
        unsafe { (entry.init_state)(state.as_mut_ptr()); }
        let mut sink = EffectSink::new();
        let api = engine_api_for_sink(&mut sink);
        let scene = entry.scenes.first().unwrap();
        unsafe { (scene.on_enter)(&api, state.as_mut_ptr()); }
        assert_eq!(sink.cues.len(), 1);
        assert_eq!(sink.cues[0] as u64, fnv1a(b"v1"));
        let next = unsafe { (scene.tick)(&api, state.as_mut_ptr()) };
        assert_eq!(next, 0);
        assert_eq!(sink.entries.len(), 1);
        assert_eq!(sink.entries[0].kind, effect_kind::PATROL_PATH);
    }

    // ── Mutate file → triggers hot reload ───────────────────────────────────
    std::fs::write(&src, SCRIPT_V2).unwrap();
    let so2 = wait_compile_ok(&mut client, 1);
    let state_v2 = state_version_of(&so2);
    let size_v2  = state_size_of(&so2);
    assert_ne!(state_v1, state_v2, "layout change → version change");
    assert_eq!(size_v2, 16, "v2 adds f64 → state grows to 16 bytes");

    // Load v2 and verify behaviour changed: now on_enter cues "v2" and tick
    // emits ATTACK instead of PATROL_PATH.
    let mut mgr = ScriptManager::new();
    let id = mgr.load_from_file(so2).expect("load v2");
    let entry = mgr.get_entry_points(id).unwrap();
    let n2 = entry.state_size() as usize;
    let mut state: thin_vec::ThinVec<u8> = thin_vec::ThinVec::with_capacity(n2);
    state.resize(n2, 0);
    unsafe { (entry.init_state)(state.as_mut_ptr()); }
    // Default for `new_field` is 3.14 — verify it landed.
    let f = f64::from_ne_bytes(state[8..16].try_into().unwrap());
    assert!((f - 3.14).abs() < 1e-12);

    let mut sink = EffectSink::new();
    let api = engine_api_for_sink(&mut sink);
    let scene = entry.scenes.first().unwrap();
    unsafe { (scene.on_enter)(&api, state.as_mut_ptr()); }
    assert_eq!(sink.cues[0] as u64, fnv1a(b"v2"));
    let _ = unsafe { (scene.tick)(&api, state.as_mut_ptr()) };
    assert_eq!(sink.entries[0].kind, effect_kind::ATTACK);

    client.unwatch(1).ok();
    client.shutdown().ok();
}

fn wait_compile_ok(client: &mut ScriptClient, expected_id: i64) -> Arc<str> {
    // The daemon may emit several events; spin until we see a CompileOk for us.
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while std::time::Instant::now() < deadline {
        match client.wait_for_event(Duration::from_millis(500)) {
            Some(DaemonMsg::CompileOk { script_id, so_path, .. }) if script_id == expected_id => {
                return so_path;
            }
            Some(DaemonMsg::CompileErr { diagnostics, .. }) => {
                let joined: thin_vec::ThinVec<&str> =
                    diagnostics.iter().map(|s| s.as_ref()).collect();
                panic!("CompileErr from langcd: {:?}", joined);
            }
            _ => continue,
        }
    }
    panic!("timed out waiting for CompileOk")
}

fn state_version_of(so: &Arc<str>) -> u32 {
    let lib = unsafe { libloading::Library::new(so.as_ref()) }.expect("load .so");
    let f: libloading::Symbol<unsafe extern "C" fn() -> u32> =
        unsafe { lib.get(b"df_state_version\0") }.unwrap();
    unsafe { f() }
}
fn state_size_of(so: &Arc<str>) -> u32 {
    let lib = unsafe { libloading::Library::new(so.as_ref()) }.expect("load .so");
    let f: libloading::Symbol<unsafe extern "C" fn() -> u32> =
        unsafe { lib.get(b"df_state_size\0") }.unwrap();
    unsafe { f() }
}

fn fnv1a(data: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME:  u64 = 0x100000001b3;
    let mut h = OFFSET;
    for &b in data { h = (h ^ b as u64).wrapping_mul(PRIME); }
    h
}
