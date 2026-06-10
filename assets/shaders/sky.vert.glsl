#version 460
//
// Fullscreen sky triangle pinned to the far plane (z = 1 in Vulkan NDC).
// Drawn after opaque geometry with depth-test LESS_OR_EQUAL and write off,
// so it fills exactly the pixels nothing else touched. The fragment shader
// reconstructs the per-pixel view ray from inv_vp — no vertex data.
layout(location = 0) out vec2 v_ndc;
void main() {
    vec2 p = vec2(float((gl_VertexIndex << 1) & 2), float(gl_VertexIndex & 2));
    v_ndc = p * 2.0 - 1.0;
    gl_Position = vec4(v_ndc, 1.0, 1.0);
}
