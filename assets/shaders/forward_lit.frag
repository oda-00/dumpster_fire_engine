#version 450

layout(location = 0) in  vec3 inNormal;
layout(location = 1) in  vec2 inUv;
layout(location = 0) out vec4 outColor;

// Set 1 — material (optional; if the descriptor set is not bound the
// driver writes white defaults into the UBO and dummy textures).
layout(set = 1, binding = 0) uniform MaterialUbo {
    vec4  baseColorFactor;  // offset 0
    float metallicFactor;   // offset 16
    float roughnessFactor;  // offset 20
    // std140 padding (8 B) so vec3 below lands on a 16-byte boundary.
    vec3  emissiveFactor;   // offset 32
    float alphaCutoff;      // offset 44
    uint  flags;            // offset 48 — bit0=doubleSided, bits1-2=alphaMode
    // std140 rounds the whole struct to vec4 alignment → final size 64 B.
} mat;

layout(set = 1, binding = 1) uniform sampler2D texBaseColor;
layout(set = 1, binding = 2) uniform sampler2D texMetallicRoughness;
layout(set = 1, binding = 3) uniform sampler2D texNormal;
layout(set = 1, binding = 4) uniform sampler2D texEmissive;
layout(set = 1, binding = 5) uniform sampler2D texOcclusion;

void main() {
    vec4 baseColor = texture(texBaseColor, inUv) * mat.baseColorFactor;

    // Alpha cutoff (MASK mode).
    if ((mat.flags & 0x6u) == 0x2u && baseColor.a < mat.alphaCutoff) {
        discard;
    }

    // Fixed directional light from upper-right-front.
    vec3  light = normalize(vec3(1.0, 2.0, 3.0));
    float diff  = max(dot(normalize(inNormal), light), 0.0);
    vec3  col   = baseColor.rgb * (diff * 0.85 + 0.12);

    // Emissive.
    vec3 emissive = texture(texEmissive, inUv).rgb * mat.emissiveFactor;
    col += emissive;

    outColor = vec4(col, baseColor.a);
}
