#version 460

layout(set = 0, binding = 0) uniform sampler2D u_atlas;

layout(location = 0) in vec2 v_uv;
layout(location = 1) in vec4 v_color;

layout(location = 0) out vec4 out_color;

void main() {
    // Atlas is R8 alpha. Texel sampled into .r; rect quads (uv = degenerate
    // zero rect) bypass the atlas via v_color.a fall-through.
    float a = texture(u_atlas, v_uv).r;
    // If uv collapses to a single point (zero-area rect), treat as solid fill.
    if (v_uv.x <= 0.001 && v_uv.y <= 0.001) {
        out_color = v_color;
    } else {
        out_color = vec4(v_color.rgb, v_color.a * a);
    }
}
