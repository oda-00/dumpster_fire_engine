#version 460
//
// Procedural wire grid + axes + per-pane ortho overlays. Geometry is
// generated entirely from gl_VertexIndex — no vertex bindings.
//
// Pairs of vertices form line-list endpoints (Vulkan LINE_LIST topology).
//   `vi = gl_VertexIndex`
//   `line = vi / 2`           — line index
//   `end  = vi & 1`           — 0 or 1 endpoint
//
// Per-pane the host emits `cmd_draw(line_count * 2, 1, 0, 0)` where
// `line_count = host_side_line_count(...)`.
//
// Three world-plane variants are interleaved in perspective (flags bit 0 = 1):
//   plane 0 = XZ (floor)
//   plane 1 = XY (front)
//   plane 2 = YZ (right)
// Ortho panes select a single plane via `flags bits 1..3`.

layout(push_constant) uniform Push {
    mat4 mvp;          //  0..64
    vec2 extent;       // 64..72   half-extents (u, v)
    vec2 spacing;      // 72..80   (minor, major)
    vec4 col_minor;    // 80..96
    vec4 col_major;    // 96..112
    vec4 col_axis_x;   //112..128
    vec4 col_axis_z;   //128..144
    uint flags;        //144..148  bit0=is_perspective, bits1..3=view_plane
    uint _p0;
    uint _p1;
    uint _p2;
} pc;

layout(location = 0) out vec4 v_color;

// Treat ±epsilon as "on the axis line" for color emphasis.
const float AXIS_EPS = 1e-3;

void main() {
    uint vi   = uint(gl_VertexIndex);
    uint line = vi >> 1;
    uint end  = vi & 1u;

    bool persp     = (pc.flags & 1u) != 0u;
    uint plane_in  = (pc.flags >> 1) & 7u;

    // Per-plane vertex layout:
    //   lines_per_axis = 2 * ceil(max(extent.x, extent.y) / min(spacing)) + 1
    //   total_per_plane = lines_per_axis * 2 + 2   (both axes + 2 axis emphasis)
    float cell           = min(pc.spacing.x, pc.spacing.y);
    uint  lines_per_axis = uint(ceil(max(pc.extent.x, pc.extent.y) / max(cell, 1e-3))) * 2u + 1u;
    uint  per_plane      = lines_per_axis * 2u + 2u;

    uint planes_to_draw = persp ? 3u : 1u;
    uint plane_id_local = line / per_plane;
    uint plane_id       = persp ? (plane_id_local % planes_to_draw) : plane_in;
    uint local_line     = line % per_plane;

    // ── Classify the line ────────────────────────────────────────────────
    bool is_axis_x  = (local_line == per_plane - 2u);   // X axis emphasis
    bool is_axis_z  = (local_line == per_plane - 1u);   // Z axis emphasis
    bool is_along_u = local_line < lines_per_axis;      // line constant in V

    // Index within its axis group (0..lines_per_axis-1).
    uint group_idx = is_along_u
        ? local_line
        : (local_line - lines_per_axis);
    int  center_off = int(group_idx) - int(lines_per_axis / 2u);
    float t = float(center_off) * pc.spacing.x;
    if (is_axis_x || is_axis_z) t = 0.0;

    float u_endpoint = (end == 0u) ? -pc.extent.x : pc.extent.x;
    float v_endpoint = (end == 0u) ? -pc.extent.y : pc.extent.y;

    // ── Solve (u, v) in plane-local space ────────────────────────────────
    float u, v;
    if (is_axis_x) {
        // X axis — line lies along U at V = 0.
        u = u_endpoint;
        v = 0.0;
    } else if (is_axis_z) {
        // Z axis — line lies along V at U = 0.
        u = 0.0;
        v = v_endpoint;
    } else if (is_along_u) {
        u = u_endpoint;
        v = t;
    } else {
        u = t;
        v = v_endpoint;
    }

    // ── Map plane-local to world-space ───────────────────────────────────
    vec3 ws;
    if      (plane_id == 0u) ws = vec3(u, 0.0, v);  // XZ floor — Y up
    else if (plane_id == 1u) ws = vec3(u, v, 0.0);  // XY front
    else                     ws = vec3(0.0, u, v);  // YZ right

    // ── Color resolution ─────────────────────────────────────────────────
    vec4 col;
    if      (is_axis_x) col = pc.col_axis_x;
    else if (is_axis_z) col = pc.col_axis_z;
    else {
        // Major lines fall on multiples of spacing.major; minor on multiples
        // of spacing.minor. Use `mod` with float epsilon for safety.
        float at = abs(t);
        bool is_major = mod(at + AXIS_EPS, max(pc.spacing.y, 1e-3)) < 2.0 * AXIS_EPS;
        col = is_major ? pc.col_major : pc.col_minor;
    }

    // Perspective fades non-XZ planes (so the floor reads as primary).
    if (persp && plane_id != 0u) col.a *= 0.35;

    gl_Position = pc.mvp * vec4(ws, 1.0);
    v_color     = col;
}
