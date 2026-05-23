#version 460
//
// Vertexless full-screen triangle. Three vertices in NDC cover the screen
// with one large triangle, which is faster than two-tris (one less edge to
// rasterize). Pipeline binds no vertex buffer; topology is TRIANGLE_LIST
// with vertex_count = 3.

layout(location = 0) out vec2 v_uv;

void main() {
    // NDC positions for the big-triangle pattern:
    //   index 0 → (-1, -1)   uv (0, 0)
    //   index 1 → ( 3, -1)   uv (2, 0)
    //   index 2 → (-1,  3)   uv (0, 2)
    // Clipping crops the off-screen halves; UVs interpolate over [0, 1].
    vec2 pos = vec2(
        float((gl_VertexIndex == 1) ? 3 : -1),
        float((gl_VertexIndex == 2) ? 3 : -1)
    );
    v_uv        = (pos + 1.0) * 0.5;
    gl_Position = vec4(pos, 0.0, 1.0);
}
