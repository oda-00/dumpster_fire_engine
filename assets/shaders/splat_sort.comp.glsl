#version 450
//
// KHR_gaussian_splatting bitonic sort.
//
// One dispatch performs one comparison phase of Batcher's bitonic sort.
// To fully sort N splats, the host re-dispatches this shader
// log2(N) * (log2(N) + 1) / 2 times, varying (k, j) via push constant.
//
// Per-thread: compare splat index `id` against `id XOR j`. If they're in
// the wrong order for this bitonic subsequence, swap them. The "key"
// being sorted is the view-space depth (z) stored alongside the splat
// index in `pairs[]`.
//
// Workgroup size 256 matches the typical splat-count granularity (a few
// thousand splats / 256 ≈ tens of workgroups per dispatch).
//
// Push constant lays out as:
//   k : current bitonic sequence size (power of two)
//   j : current compare distance (power of two, j < k)
//   n : total splat count (so threads with id >= n skip)
//   _ : padding to mat4-aligned 16 bytes

layout(local_size_x = 256, local_size_y = 1, local_size_z = 1) in;

layout(push_constant) uniform Pc {
    uint k;
    uint j;
    uint n;
    uint _pad;
} pc;

// One slot per splat: (view_z bit-packed as u32 by reinterpreting the
// IEEE-754 bits, splat_index). Sorting on the u32 representation of
// view_z works because positive floats compare bit-identical to their
// IEEE encoding; all splats in front of the camera have view_z > 0.
struct SortPair {
    uint key;   // view_z reinterpreted as u32 (positive floats only)
    uint index; // original splat index
};

layout(set = 0, binding = 0, std430) buffer Pairs {
    SortPair pairs[];
} pairs_buf;

void main() {
    uint id  = gl_GlobalInvocationID.x;
    if (id >= pc.n) return;
    uint ixj = id ^ pc.j;
    // Avoid double-swapping: only the lower index of the pair acts.
    if (ixj <= id) return;
    if (ixj >= pc.n) return;

    bool ascending = ((id & pc.k) == 0u);
    SortPair a = pairs_buf.pairs[id];
    SortPair b = pairs_buf.pairs[ixj];
    bool should_swap = (a.key > b.key) == ascending;
    if (should_swap) {
        pairs_buf.pairs[id]  = b;
        pairs_buf.pairs[ixj] = a;
    }
}
