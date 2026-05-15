//! Daemon hot-reload latency benchmark.
//!
//!   cargo bench --bench script_hot_reload
//!
//! Plan §8: "end-to-end hot-reload latency".  Spawns `langcd` once, drives
//! one `Watch` + repeated file mutations through it, and measures the
//! wall-clock from `write(file)` to receiving the corresponding `CompileOk`
//! over the IPC.  Each iteration alternates between two byte-different
//! source versions so the daemon's content-hash cache never short-circuits.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use divan::Bencher;

use dumpster_fire_engine::resource_manager::event_manager::script_client::ScriptClient;
use dumpster_fire_engine::resource_manager::event_manager::script_ipc::DaemonMsg;

fn main() { divan::main(); }

const V_A: &str = r#"
script "hr_bench" {
    state { x: i32 = 0 }
    scene only {
        behavior { action patrol_path() }
    }
}
"#;

const V_B: &str = r#"
script "hr_bench" {
    state { x: i32 = 0 }
    scene only {
        behavior { action attack() }
    }
}
"#;

fn locate_langcd() -> PathBuf {
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap());
    let bin = manifest.join("target/release/langcd");
    if !bin.exists() {
        let status = std::process::Command::new("cargo")
            .args(["build", "-p", "langcd", "--release"])
            .status().unwrap();
        assert!(status.success(), "cargo build -p langcd failed");
    }
    bin
}

fn wait_compile_ok(client: &mut ScriptClient, script_id: i64) -> Arc<str> {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        match client.wait_for_event(Duration::from_millis(500)) {
            Some(DaemonMsg::CompileOk { script_id: id, o_path, .. }) if id == script_id => {
                return o_path;
            }
            Some(DaemonMsg::CompileErr { .. }) => panic!("daemon reported compile error"),
            _ => continue,
        }
    }
    panic!("timed out waiting for CompileOk")
}

struct Fixture {
    client:    ScriptClient,
    src_path:  Arc<str>,
    src_pb:    PathBuf,
    next_is_a: bool,
}

impl Fixture {
    fn build() -> Self {
        let langcd = locate_langcd();
        let dir = std::env::temp_dir().join(format!("dfe_hr_bench_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src_pb = dir.join("bench.lang");
        std::fs::write(&src_pb, V_A).unwrap();
        let src_path: Arc<str> = Arc::from(src_pb.to_string_lossy().as_ref());

        let mut client = ScriptClient::spawn(&langcd).expect("spawn langcd");
        client.watch(1, Arc::clone(&src_path)).expect("watch");
        // Consume the initial compile-on-watch.
        let _ = wait_compile_ok(&mut client, 1);

        Fixture { client, src_path, src_pb, next_is_a: false }
    }
}

#[divan::bench(sample_count = 8, sample_size = 1)]
fn round_trip(bencher: Bencher) {
    bencher.with_inputs(Fixture::build).bench_local_refs(|fx| {
        let body = if fx.next_is_a { V_A } else { V_B };
        fx.next_is_a = !fx.next_is_a;
        let t0 = Instant::now();
        std::fs::write(&fx.src_pb, body).unwrap();
        let _ = wait_compile_ok(&mut fx.client, 1);
        let _ = t0.elapsed(); // captured by the divan harness itself
        let _ = &fx.src_path;
    });
}

// ── Concurrent multi-file reload ─────────────────────────────────────────────
//
// Exercises the rayon-dispatched compile path in langcd: N files are watched,
// then all N are written in immediate succession.  Measures the wall-clock
// from "writes start" to "all N CompileOks received".  Pre-rayon main loop
// this was strictly serial — N × ~6 ms.  With the worker pool it lands closer
// to max(per-file-time) for N ≤ num_cpus.

struct MultiFixture {
    client:    ScriptClient,
    paths:     Vec<PathBuf>,
    next_is_a: bool,
}

fn build_multi(n: usize) -> MultiFixture {
    let langcd = locate_langcd();
    let dir = std::env::temp_dir().join(format!("dfe_hr_multi_{}_{}",
        n, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut paths = Vec::with_capacity(n);
    for i in 0..n {
        let p = dir.join(format!("bench_{i}.lang"));
        std::fs::write(&p, V_A).unwrap();
        paths.push(p);
    }
    let mut client = ScriptClient::spawn(&langcd).expect("spawn langcd");
    for (i, p) in paths.iter().enumerate() {
        let arc: Arc<str> = Arc::from(p.to_string_lossy().as_ref());
        client.watch((i + 1) as i64, arc).expect("watch");
    }
    // Drain the initial-on-watch compiles — order is non-deterministic now
    // that langcd dispatches across rayon, so accept any script_id from 1..=n.
    drain_compile_oks(&mut client, n);
    MultiFixture { client, paths, next_is_a: false }
}

fn drain_compile_oks(client: &mut ScriptClient, n: usize) {
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut got = vec![false; n];
    let mut remaining = n;
    while remaining > 0 && Instant::now() < deadline {
        if let Some(DaemonMsg::CompileOk { script_id, .. }) =
            client.wait_for_event(Duration::from_millis(500))
        {
            let i = script_id as usize - 1;
            if i < n && !got[i] { got[i] = true; remaining -= 1; }
        }
    }
    assert_eq!(remaining, 0, "drain timed out: {}/{} compiled", n - remaining, n);
}

fn run_multi(fx: &mut MultiFixture) {
    let body = if fx.next_is_a { V_A } else { V_B };
    fx.next_is_a = !fx.next_is_a;
    for p in fx.paths.iter() {
        std::fs::write(p, body).unwrap();
    }
    let n = fx.paths.len();
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut got = vec![false; n];
    let mut remaining = n;
    while remaining > 0 && Instant::now() < deadline {
        if let Some(DaemonMsg::CompileOk { script_id, .. }) =
            fx.client.wait_for_event(Duration::from_millis(500))
        {
            let i = script_id as usize - 1;
            if i < n && !got[i] { got[i] = true; remaining -= 1; }
        }
    }
    assert_eq!(remaining, 0, "timed out: only {}/{} compiled", n - remaining, n);
}

#[divan::bench(sample_count = 8, sample_size = 1)]
fn concurrent_round_trip_4(bencher: Bencher) {
    bencher.with_inputs(|| build_multi(4)).bench_local_refs(run_multi);
}

#[divan::bench(sample_count = 8, sample_size = 1)]
fn concurrent_round_trip_8(bencher: Bencher) {
    bencher.with_inputs(|| build_multi(8)).bench_local_refs(run_multi);
}
