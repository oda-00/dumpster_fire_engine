//! Runtime CPU feature detection — cached once per process.
//!
//! Every `#[target_feature(enable = "...")]` function in this crate is
//! unsafe to call when the host CPU doesn't actually have the feature.
//! Compile-time `#[cfg(target_arch = "x86_64")]` only guarantees that
//! the architecture is x86_64 — NOT that every x86_64 CPU has SSSE3 /
//! SSE4.1 / AVX2 / FMA. SSSE3 is universal on modern desktop x86_64
//! but not on every embedded x86_64 build target.
//!
//! Every public SIMD entry point checks via `cpu_features()` before
//! dispatching to the SIMD path; falls back to the scalar reference
//! when the feature is absent.

use std::sync::OnceLock;

#[derive(Clone, Copy, Debug)]
pub struct CpuFeatures {
    pub sse2:  bool,
    pub ssse3: bool,
    pub sse41: bool,
    pub avx2:  bool,
    pub fma:   bool,
    pub neon:  bool,
}

impl CpuFeatures {
    /// All-features-disabled — forces every dispatcher onto the scalar
    /// fallback. Used by `simd_runtime_dispatch` tests to verify the
    /// scalar path is bit-identical to the SIMD path.
    pub const fn scalar_only() -> Self {
        Self { sse2: false, ssse3: false, sse41: false, avx2: false, fma: false, neon: false }
    }
}

pub fn cpu_features() -> CpuFeatures {
    static CACHE: OnceLock<CpuFeatures> = OnceLock::new();
    *CACHE.get_or_init(|| {
        #[allow(unused_mut)]
        let mut f = CpuFeatures::scalar_only();
        #[cfg(target_arch = "x86_64")]
        {
            f.sse2  = is_x86_feature_detected!("sse2");
            f.ssse3 = is_x86_feature_detected!("ssse3");
            f.sse41 = is_x86_feature_detected!("sse4.1");
            f.avx2  = is_x86_feature_detected!("avx2");
            f.fma   = is_x86_feature_detected!("fma");
        }
        #[cfg(target_arch = "aarch64")]
        {
            f.neon = std::arch::is_aarch64_feature_detected!("neon");
        }
        f
    })
}
