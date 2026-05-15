//! End-to-end scripting integration test.
//!
//! Drives the full pipeline:
//!
//!     .lang  →  langc (LLVM AOT)  →  .so  →  ScriptManager::load_from_file
//!         →  call df_state_size / df_state_version / df_init_state
//!         →  inspect SceneDefArray (df_create_scene_defs result)
//!         →  invoke tick / on_enter / on_exit on a real EngineAPI
//!         →  assert push_effect / cue_troupe callbacks fired with expected data
//!
//! No mocks.  No stubs.  Real native code emitted by LLVM at `-O3` and loaded
//! into the test process via `libloading`.

use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use dumpster_fire_engine::resource_manager::event_manager::script::{
    ActiveScript, ScriptManager,
};
use dumpster_fire_engine::resource_manager::event_manager::script_abi::{
    EffectSink, engine_api_for_sink, effect_kind,
};

fn fnv1a(data: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME:  u64 = 0x100000001b3;
    let mut h = OFFSET;
    for &b in data { h = (h ^ b as u64).wrapping_mul(PRIME); }
    h
}

const GUARD_LANG: &str = r#"
script "guard" {
    state {
        patrol_index:    i32 = 0
        last_alert_time: f64 = 0.0
    }

    scene patrol {
        on_enter => cue_troupe("walk");

        behavior {
            selector {
                sequence {
                    condition enemy_in_range(10.0),
                    action attack()
                },
                action patrol_path()
            }
        }

        transition alert when after_seconds(1.5);
    }

    scene alert {
        on_enter => cue_troupe("alarm");

        behavior {
            action attack()
        }

        transition patrol when after_seconds(2.0);
    }
}
"#;

fn locate_langc() -> std::path::PathBuf {
    // Honour CARGO_BIN_EXE_langc when set by `cargo test`, else fall back to
    // building langc in release mode and using its target path.
    if let Some(p) = std::env::var_os("CARGO_BIN_EXE_langc") {
        return std::path::PathBuf::from(p);
    }
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap());
    let status = Command::new("cargo")
        .args(["build", "-p", "langc", "--release"])
        .current_dir(&manifest)
        .status()
        .expect("invoke cargo build");
    assert!(status.success(), "cargo build -p langc failed");
    manifest.join("target/release/langc")
}

fn write_temp(name: &str, body: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("dfe_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(name);
    std::fs::write(&p, body).unwrap();
    p
}

#[test]
fn compile_load_and_tick_guard_script() {
    let langc = locate_langc();
    let src   = write_temp("guard.lang", GUARD_LANG);
    let out   = src.with_extension("so");

    let status = Command::new(&langc)
        .arg(&src)
        .arg("-o").arg(&out)
        .status()
        .expect("spawn langc");
    assert!(status.success(), "langc failed");
    assert!(out.exists(), "no .so at {}", out.display());

    // ── Load via libloading ──────────────────────────────────────────────────
    let mut mgr = ScriptManager::new();
    let path: Arc<str> = Arc::from(out.to_string_lossy().as_ref());
    let id = mgr.load_from_file(path).expect("load");

    let entry = mgr.get_entry_points(id).expect("entry points");

    // State size = i32(4) + 4 pad + f64(8) = 16 bytes (alignment 8).
    assert_eq!(entry.state_size(), 16, "state_size from compiled .so");
    let v = entry.state_version();
    assert_ne!(v, 0, "state_version is hashed, non-zero for non-empty layout");

    // df_init_state should zero+default-init.  Patrol_index = 0, alert_time = 0.0.
    let mut state: thin_vec::ThinVec<u8> = thin_vec::ThinVec::with_capacity(16);
    state.resize(16, 0xAAu8);
    unsafe { (entry.init_state)(state.as_mut_ptr()); }
    // patrol_index at offset 0 → i32 zero
    assert_eq!(i32::from_ne_bytes(state[0..4].try_into().unwrap()), 0);
    // last_alert_time at offset 8 → f64 zero
    assert_eq!(f64::from_ne_bytes(state[8..16].try_into().unwrap()), 0.0);

    // Verify the SceneDefArray has both scenes.
    assert_eq!(entry.scenes.len(), 2, "two scenes");

    // ── Drive on_enter / tick for patrol scene ───────────────────────────────
    let patrol = &entry.scenes[0];
    let mut sink = EffectSink::new();
    let api = engine_api_for_sink(&mut sink);

    // on_enter fires "cue_troupe(\"walk\")" → cb_cue_troupe with the FNV hash.
    unsafe { (patrol.on_enter)(&api, state.as_mut_ptr()); }
    assert_eq!(sink.cues.len(), 1, "cue_troupe should fire once on_enter");
    // FNV-1a 64-bit of "walk":
    assert_eq!(sink.cues[0] as u64, fnv1a(b"walk"));

    sink.clear();

    // ── Tick patrol with elapsed = 0.0 (before transition fires) ─────────────
    // actor_count = 0 so enemy_in_range returns false; selector falls to
    // patrol_path → push_effect with kind = PATROL_PATH.
    // (api built above already has actor_count = 0)
    let next = unsafe { (patrol.tick)(&api, state.as_mut_ptr()) };
    assert_eq!(next, 0, "transition (after_seconds 1.5) shouldn't fire at elapsed=0");
    assert_eq!(sink.entries.len(), 1, "patrol_path effect emitted");
    assert_eq!(sink.entries[0].kind, effect_kind::PATROL_PATH);

    sink.clear();

    // ── Tick patrol with elapsed = 2.0 → transition fires ────────────────────
    let mut api2 = engine_api_for_sink(&mut sink);
    api2.elapsed = 2.0;
    let next = unsafe { (patrol.tick)(&api2, state.as_mut_ptr()) };
    assert_ne!(next, 0, "transition (after_seconds 1.5) must fire at elapsed=2.0");
    // raw_id derived as FNV-1a of "guard::alert".
    let alert_raw = {
        let mut h = 0xcbf29ce484222325u64;
        const P: u64 = 0x100000001b3;
        for &b in b"guard"   { h = (h ^ b as u64).wrapping_mul(P); }
        for &b in b"::"      { h = (h ^ b as u64).wrapping_mul(P); }
        for &b in b"alert"   { h = (h ^ b as u64).wrapping_mul(P); }
        h as i64
    };
    assert_eq!(next, alert_raw, "transition target must be the alert scene's raw_id");

    let _ = sink.tick.fetch_add(1, Ordering::Relaxed);
    mgr.unload(id);
}

const MIGRATE_V1: &str = r#"
script "migrate" {
    state {
        x: i32 = 5
    }
    scene only {
        behavior { action attack() }
    }
}
"#;

/// Template — `{V1_VERSION}` is replaced with v1's runtime state_version so the
/// migration block matches the exact `old_version` the engine will pass.
const MIGRATE_V2: &str = r#"
script "migrate" {
    state {
        x:  i32 = 99
        y:  f64 = 0.0
    }
    migrate from {V1_VERSION} {
        // Carry x over from the old layout, then add y = 1.5 for the new field.
        x = old.x
        y = 1.5
    }
    scene only {
        behavior { action attack() }
    }
}
"#;

#[test]
fn active_script_ticks_through_transitions_and_migrates_state() {
    let langc = locate_langc();

    // ── v1: state is just `x: i32 = 5` ──────────────────────────────────────
    let v1_src = write_temp("migrate_v1.lang", MIGRATE_V1);
    let v1_so  = v1_src.with_extension("so");
    let status = Command::new(&langc).arg(&v1_src).arg("-o").arg(&v1_so).status().unwrap();
    assert!(status.success());

    let mut mgr = ScriptManager::new();
    let v1_id = mgr.load_from_file(Arc::from(v1_so.to_string_lossy().as_ref())).unwrap();
    let v1_entry = mgr.get_entry_points(v1_id).unwrap();
    assert_eq!(v1_entry.state_size(), 4); // i32 only

    // Build an ActiveScript and tick it twice via the helper.
    let mut script = ActiveScript::from_entry(v1_id, v1_entry);
    // Default value `x = 5` lives at offset 0:
    assert_eq!(i32::from_ne_bytes(script.state_buffer[0..4].try_into().unwrap()), 5);

    let mut sink = EffectSink::new();
    let mut api  = engine_api_for_sink(&mut sink);
    let _ = script.tick(v1_entry, &mut api, 0.016);
    assert_eq!(sink.entries.len(), 1);
    assert_eq!(sink.entries[0].kind, effect_kind::ATTACK);
    let _ = script.tick(v1_entry, &mut api, 0.016);
    assert_eq!(sink.entries.len(), 2);

    // ── v2: layout grows by 8 bytes, `migrate from {v1_version} { ... }` runs ──
    let v1_version = v1_entry.state_version();
    let v2_body = MIGRATE_V2.replace("{V1_VERSION}", &v1_version.to_string());
    let v2_src = write_temp("migrate_v2.lang", &v2_body);
    let v2_so  = v2_src.with_extension("so");
    let status = Command::new(&langc).arg(&v2_src).arg("-o").arg(&v2_so).status().unwrap();
    assert!(status.success());

    let v2_id = mgr.load_from_file(Arc::from(v2_so.to_string_lossy().as_ref())).unwrap();
    let v2_entry = mgr.get_entry_points(v2_id).unwrap();
    assert_eq!(v2_entry.state_size(), 16);

    // Migrate the running script in place.  init_state runs first (defaults:
    // x=99, y=0.0), then the migrate body fires: `x = old.x` (5) and
    // `y = 1.5`.  Result: x=5 carried over from the previous layout, y=1.5
    // from the explicit assignment.
    script.migrate_into(v2_entry);
    assert_eq!(script.state_buffer.len(), 16);
    assert_eq!(script.state_version, v2_entry.state_version());

    let x = i32::from_ne_bytes(script.state_buffer[0..4].try_into().unwrap());
    let y = f64::from_ne_bytes(script.state_buffer[8..16].try_into().unwrap());
    assert_eq!(x, 5, "x must be carried over from old layout via `old.x`");
    assert!((y - 1.5).abs() < 1e-12, "y must come from the explicit migration assignment");

    mgr.unload(v1_id);
    mgr.unload(v2_id);
}
