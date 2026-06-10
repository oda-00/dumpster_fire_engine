#version 460
//
// Debug-lines fragment: premultiplied-alpha output with a view-distance
// fade. The vertex shader encodes the per-line palette in `v_color`; here
// we premultiply so the pipeline's premul blend composites correctly, and
// roll the grid off with linear view depth — a floor seen at a grazing
// angle otherwise bunches its 1-unit lines into solid bands at the horizon.
// `1/gl_FragCoord.w` is linear view-space depth for perspective panes and
// exactly 1.0 for ortho panes (w_clip = 1), so ortho grids never fade.

layout(location = 0) in  vec4 v_color;
layout(location = 0) out vec4 frag;

const float FADE_START = 14.0; // fully visible inside this view distance
const float FADE_END   = 32.0; // fully gone past this

void main() {
    float view_depth = 1.0 / max(gl_FragCoord.w, 1e-6);
    float fade = view_depth <= 1.0
        ? 1.0
        : clamp((FADE_END - view_depth) / (FADE_END - FADE_START), 0.0, 1.0);
    float a = v_color.a * fade;
    frag = vec4(v_color.rgb * a, a);
}
