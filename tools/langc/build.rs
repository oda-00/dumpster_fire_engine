//! LLVM 18 statically links zstd via `-lzstd`. Distros sometimes ship
//! only the shared library as `libzstd.so.1` (not the `libzstd.so`
//! symlink the linker needs). When `libzstd.so` is missing, symlink it
//! in OUT_DIR and add that dir to the link search path.

fn main() {
    let probe_paths = [
        "/usr/lib/x86_64-linux-gnu",
        "/usr/lib/aarch64-linux-gnu",
        "/usr/lib",
        "/usr/local/lib",
    ];
    for d in &probe_paths {
        let canon = std::path::Path::new(d).join("libzstd.so");
        if canon.exists() {
            return; // system already has the symlink — nothing to do.
        }
        let real = std::path::Path::new(d).join("libzstd.so.1");
        if real.exists() {
            let out = std::env::var("OUT_DIR").unwrap();
            let dst = std::path::Path::new(&out).join("libzstd.so");
            let _ = std::fs::remove_file(&dst);
            #[cfg(unix)]
            std::os::unix::fs::symlink(&real, &dst).unwrap();
            println!("cargo:rustc-link-search=native={out}");
            return;
        }
    }
    // Last-chance: rely on the linker's default search. If zstd is
    // truly missing, the compile error will surface clearly below.
    println!("cargo:warning=libzstd not found in standard paths; relying on linker defaults");
}
