//! Experiment 1 — Half-edge one-ring traversal: index-based SoA vs pointer AoS.
//!
//! Blender's BMesh chases pointers through double-linked circular lists; DynTopo
//! spends ~30% of its time just fetching memory once the mesh fragments
//! (code.blender.org). A struct-of-arrays, index-based half-edge keeps the hot
//! arrays dense and lets the loop stay a tight load/compare, with no allocator
//! fragmentation. This isolates the codegen difference.
//!
//! Build: rustc -O -C opt-level=3 -C target-cpu=x86-64-v3 --emit asm --crate-type=lib

// ---- index-based SoA half-edge (what we'd build) -------------------------
// Parallel arrays indexed by half-edge id. `next` walks the face loop; `twin`
// crosses to the adjacent face; `vert` is the origin vertex.
pub struct HalfEdgeSoA {
    pub next: Vec<u32>,
    pub twin: Vec<u32>,
    pub vert: Vec<u32>,
    pub vert_he: Vec<u32>, // one outgoing half-edge per vertex
}

pub const INVALID: u32 = u32::MAX;

// Sum the indices of all vertices in the one-ring around `v` (a stand-in for a
// real per-neighbor op like cotangent weight / normal accumulation). The ring
// walk is the canonical adjacency query.
#[no_mangle]
pub fn one_ring_sum_soa(m: &HalfEdgeSoA, v: u32) -> u64 {
    let start = m.vert_he[v as usize];
    if start == INVALID {
        return 0;
    }
    let mut acc = 0u64;
    let mut he = start;
    loop {
        // neighbor = vertex at the tip of this half-edge = vert[twin]
        let t = m.twin[he as usize];
        acc += m.vert[t as usize] as u64;
        // advance around the vertex: next of twin
        he = m.next[t as usize];
        if he == start || he == INVALID {
            break;
        }
    }
    acc
}

// ---- pointer-based AoS half-edge (BMesh-style) ---------------------------
pub struct HeNode {
    pub next: *const HeNode,
    pub twin: *const HeNode,
    pub vert: u32,
}

/// # Safety: nodes form a valid closed half-edge mesh.
#[no_mangle]
pub unsafe fn one_ring_sum_ptr(start: *const HeNode) -> u64 {
    if start.is_null() {
        return 0;
    }
    let mut acc = 0u64;
    let mut he = start;
    loop {
        let t = (*he).twin;
        acc += (*t).vert as u64; // pointer chase #1 (twin), #2 (vert) — random addrs
        he = (*t).next; // pointer chase #3 (next)
        if he == start || he.is_null() {
            break;
        }
    }
    acc
}
