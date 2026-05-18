// Morph-blend compute shader.
//
// Applies weighted morph-target deltas to a rest-pose vertex buffer:
//   out_vertex[v] = rest_vertex[v] + Σ_t (weight[t] × delta[t][v])
// After blending, (normal, tangent.xyz) are re-orthonormalised per spec.
//
// Secondary buffer layout (all float32, header fields stored as bit-reinterp uints):
//   [0]          float  target_count   (bits reinterpreted as uint)
//   [1]          float  vertex_count   (bits reinterpreted as uint)
//   [2..2+T-1]   float  weight[T]      (T = target_count)
//   [2+T..]      float  deltas[T×V×9]  (Δpos[3] + Δnorm[3] + Δtan[3] per vertex)
//
// Primary buffer:  vertex_count × ForgeVertex (12 floats = pos[3]+norm[3]+tan[4]+uv[2])
// Result buffer:   same shape and size as primary.
//
// Each invocation handles one vertex. Dispatch ceil(vertex_count / 64) groups.
//
// Compile:
//   glslc -fshader-stage=compute morph_blend.comp.glsl -o morph_blend.comp.glsl.spv

#version 450
layout(local_size_x = 64, local_size_y = 1, local_size_z = 1) in;

layout(set = 0, binding = 0) readonly  buffer Primary   { float data[]; } rest;
layout(set = 0, binding = 1) readonly  buffer Secondary { float data[]; } morph;
layout(set = 0, binding = 2) writeonly buffer Result    { float data[]; } posed;

// ForgeVertex field offsets in floats: pos[0..2], norm[3..5], tan[6..9], uv[10..11].
const uint FVERT = 12u;

void main() {
    uint v = gl_GlobalInvocationID.x;

    // Header: first two floats hold uint bit-patterns for target_count, vertex_count.
    uint target_count = floatBitsToUint(morph.data[0]);
    uint vertex_count = floatBitsToUint(morph.data[1]);
    if (v >= vertex_count) return;

    // Read rest-pose vertex.
    uint rv = v * FVERT;
    vec3  pos  = vec3(rest.data[rv+0], rest.data[rv+1], rest.data[rv+2]);
    vec3  norm = vec3(rest.data[rv+3], rest.data[rv+4], rest.data[rv+5]);
    vec3  tan  = vec3(rest.data[rv+6], rest.data[rv+7], rest.data[rv+8]);
    float tw   = rest.data[rv+9];  // tangent W (handedness), preserved as-is

    // Weights start at float offset 2; deltas follow the weight array.
    uint weights_base = 2u;
    uint deltas_base  = weights_base + target_count;

    for (uint t = 0u; t < target_count; t++) {
        float w = morph.data[weights_base + t];
        if (abs(w) < 1e-7) continue;
        // delta[t][v] at deltas_base + (t * vertex_count + v) * 9
        uint d = deltas_base + (t * vertex_count + v) * 9u;
        pos  += w * vec3(morph.data[d+0], morph.data[d+1], morph.data[d+2]);
        norm += w * vec3(morph.data[d+3], morph.data[d+4], morph.data[d+5]);
        tan  += w * vec3(morph.data[d+6], morph.data[d+7], morph.data[d+8]);
    }

    // Re-orthonormalise per spec §3.7.2.1.
    float nl = length(norm);
    if (nl > 1e-7) norm /= nl;
    tan = tan - dot(tan, norm) * norm;
    float tl = length(tan);
    if (tl > 1e-7) tan /= tl;

    // Write posed vertex (same layout as rest).
    uint wv = v * FVERT;
    posed.data[wv+0]  = pos.x;  posed.data[wv+1]  = pos.y;  posed.data[wv+2]  = pos.z;
    posed.data[wv+3]  = norm.x; posed.data[wv+4]  = norm.y; posed.data[wv+5]  = norm.z;
    posed.data[wv+6]  = tan.x;  posed.data[wv+7]  = tan.y;  posed.data[wv+8]  = tan.z;
    posed.data[wv+9]  = tw;
    posed.data[wv+10] = rest.data[rv+10]; // uv.x
    posed.data[wv+11] = rest.data[rv+11]; // uv.y
}
