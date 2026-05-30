//! Experiment 4 — Branchless selection + incremental dirty-range upload.
//!
//! (a) Blender's selection slows to a crawl past ~20M polys (blenderartists);
//!     a branchless mask test over an SoA bit/byte array vectorizes and
//!     parallelizes trivially.
//! (b) Blender rebuilds draw batches on edits; the win is uploading only the
//!     changed vertex range to a persistent GPU buffer (memcpy of a sub-slice),
//!     not rebuilding everything.
//!
//! Build: rustc -O -C opt-level=3 -C target-cpu=x86-64-v3 --emit asm --crate-type=lib

// Count selected vertices: branchless sum of a 0/1 flag stream → vectorizes.
#[no_mangle]
pub fn count_selected(flags: &[u8]) -> u64 {
    let mut n = 0u64;
    for &f in flags {
        n += (f & 1) as u64;
    }
    n
}

// Box-select: set the selection bit for every vertex inside an AABB. Branchless
// per-vertex predicate (compare + mask) so it stays SIMD/parallel friendly.
#[no_mangle]
pub fn box_select_soa(x: &[f32], y: &[f32], z: &[f32], flags: &mut [u8], lo: [f32; 3], hi: [f32; 3]) {
    let n = flags.len();
    for i in 0..n {
        let inside = (x[i] >= lo[0]) & (x[i] <= hi[0])
            & (y[i] >= lo[1]) & (y[i] <= hi[1])
            & (z[i] >= lo[2]) & (z[i] <= hi[2]);
        flags[i] = inside as u8;
    }
}

// Incremental upload: copy only [first,last) vertices into a mapped GPU buffer.
// This is the per-edit cost we want — O(touched) — vs Blender's O(whole mesh)
// batch rebuild.
///
/// # Safety: `dst` points to a mapped buffer with room for `src.len()` verts.
#[no_mangle]
pub unsafe fn upload_dirty_range(dst: *mut f32, src: &[f32], first: usize, count: usize) {
    let s = &src[first..first + count];
    core::ptr::copy_nonoverlapping(s.as_ptr(), dst.add(first), count);
}
