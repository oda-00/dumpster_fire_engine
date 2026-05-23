#version 460
//
// Debug-lines fragment: identity passthrough with premultiplied-alpha
// output. The vertex shader already encodes the per-line palette in
// `v_color`; here we just premultiply so the pipeline's premul blend
// math composites correctly against the scene.

layout(location = 0) in  vec4 v_color;
layout(location = 0) out vec4 frag;

void main() {
    frag = vec4(v_color.rgb * v_color.a, v_color.a);
}
