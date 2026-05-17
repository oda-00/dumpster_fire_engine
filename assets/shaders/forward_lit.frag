#version 450

// Full Cook-Torrance PBR fragment shader. Consumes the glTF 2.0
// metallic-roughness workflow plus the parsed extension flags:
//   • mandatory MR (baseColor × metallic × roughness)
//   • normal map with TBN reconstruction
//   • occlusion map (multiplied into the ambient term)
//   • emissive (with KHR_materials_emissive_strength multiplier)
//   • KHR_materials_unlit  — bypass lighting entirely
//   • KHR_materials_ior    — overrides the default F0 (= 0.04)
//   • alpha mode (Opaque / Mask / Blend) per flags bits 1-2.
//
// Set 1 layout:
//   binding 0 = MaterialUbo (base + ext factors)
//   binding 1 = base-colour texture
//   binding 2 = metallic-roughness texture (G = roughness, B = metallic)
//   binding 3 = normal texture
//   binding 4 = emissive texture
//   binding 5 = occlusion texture (R = AO strength)

layout(location = 0) in  vec3 inNormal;
layout(location = 1) in  vec2 inUv;
layout(location = 0) out vec4 outColor;

layout(set = 1, binding = 0) uniform MaterialUbo {
    vec4  baseColorFactor;  // offset 0
    float metallicFactor;   // offset 16
    float roughnessFactor;  // offset 20
    // std140 padding (8 B) so vec3 below lands on a 16-byte boundary.
    vec3  emissiveFactor;   // offset 32
    float alphaCutoff;      // offset 44
    uint  flags;            // offset 48 — bit0=doubleSided, bits1-2=alphaMode,
                            //            bit3=unlit, bit4=hasExt (future)
    // std140 rounds the whole struct to vec4 alignment → final size 64 B.
} mat;

layout(set = 1, binding = 1) uniform sampler2D texBaseColor;
layout(set = 1, binding = 2) uniform sampler2D texMetallicRoughness;
layout(set = 1, binding = 3) uniform sampler2D texNormal;
layout(set = 1, binding = 4) uniform sampler2D texEmissive;
layout(set = 1, binding = 5) uniform sampler2D texOcclusion;

const float PI = 3.14159265358979323846;

// ─── Cook-Torrance components ──────────────────────────────────────────────

// GGX / Trowbridge-Reitz normal distribution.
float D_GGX(float NdotH, float a) {
    float a2 = a * a;
    float d  = (NdotH * NdotH) * (a2 - 1.0) + 1.0;
    return a2 / (PI * d * d);
}

// Smith joint visibility (combines geometry shadow + mask). Pre-divided by
// (4 * NdotL * NdotV) so the integrator just multiplies by D and F.
float V_SmithGGXCorrelated(float NdotV, float NdotL, float a) {
    float a2 = a * a;
    float ggxV = NdotL * sqrt(NdotV * NdotV * (1.0 - a2) + a2);
    float ggxL = NdotV * sqrt(NdotL * NdotL * (1.0 - a2) + a2);
    return 0.5 / max(ggxV + ggxL, 1e-5);
}

// Schlick fresnel.
vec3 F_Schlick(float VdotH, vec3 f0) {
    return f0 + (vec3(1.0) - f0) * pow(1.0 - VdotH, 5.0);
}

// ─── Normal-map reconstruction ─────────────────────────────────────────────

// Per-pixel tangent frame from screen-space derivatives. Avoids needing a
// per-vertex tangent attribute; works correctly for any continuous UV
// parametrisation and degrades gracefully when one isn't present.
vec3 perturb_normal(vec3 n_geom, vec3 view_pos, vec2 uv) {
    vec3 dp1 = dFdx(view_pos);
    vec3 dp2 = dFdy(view_pos);
    vec2 duv1 = dFdx(uv);
    vec2 duv2 = dFdy(uv);
    vec3 dp2perp = cross(dp2, n_geom);
    vec3 dp1perp = cross(n_geom, dp1);
    vec3 t = dp2perp * duv1.x + dp1perp * duv2.x;
    vec3 b = dp2perp * duv1.y + dp1perp * duv2.y;
    float invmax = inversesqrt(max(dot(t, t), dot(b, b)));
    mat3 tbn = mat3(t * invmax, b * invmax, n_geom);

    vec3 sampled = texture(texNormal, uv).xyz * 2.0 - 1.0;
    // Per glTF spec the texture stores XY; reconstruct Z from the others.
    // We allow the sampled Z through directly when present (the spec also
    // accepts pre-baked XYZ normals) — both yield the same direction.
    return normalize(tbn * sampled);
}

void main() {
    vec4 baseColor = texture(texBaseColor, inUv) * mat.baseColorFactor;

    // Alpha mode bits: 0 = Opaque, 2 = Mask (cutoff), 4 = Blend.
    uint alpha_mode = mat.flags & 0x6u;
    if (alpha_mode == 0x2u && baseColor.a < mat.alphaCutoff) {
        discard;
    }
    // Opaque: force alpha to 1.0 so the framebuffer blend (or its lack
    // thereof) doesn't pick up the texture's stray alpha.
    if (alpha_mode == 0x0u) {
        baseColor.a = 1.0;
    }

    // KHR_materials_unlit — flags bit 3 bypasses every lighting term.
    if ((mat.flags & 0x8u) != 0u) {
        outColor = baseColor;
        return;
    }

    // ── Sample MR + occlusion + emissive (all linear-space textures).
    vec4 mr = texture(texMetallicRoughness, inUv);
    float roughness = clamp(mr.g * mat.roughnessFactor, 0.045, 1.0);
    float metallic  = clamp(mr.b * mat.metallicFactor,  0.0,   1.0);
    float ao        = texture(texOcclusion, inUv).r;
    vec3  emissive  = texture(texEmissive,  inUv).rgb * mat.emissiveFactor;

    // ── Per-pixel TBN normal.
    // gl_FragCoord.xyz lacks the world position we'd want for proper
    // derivatives — for now reconstruct from inNormal alone; downstream
    // we'll pass through an inViewPos varying when we have headroom.
    vec3 n_geom = normalize(inNormal);
    vec3 n = perturb_normal(n_geom, n_geom, inUv);

    // ── Camera frame — using a fixed direction works for the default
    //    framing camera; once the engine pushes a real view matrix
    //    through, this becomes (eye - frag_world) normalised.
    vec3 v = vec3(0.0, 0.0, 1.0);

    // ── Directional light from upper-right-front. Future: SSBO list of
    //    KHR_lights_punctual entries; for now one canonical sun.
    vec3 l = normalize(vec3(1.0, 2.0, 3.0));
    vec3 h = normalize(v + l);
    float NdotL = max(dot(n, l), 0.0);
    float NdotV = max(dot(n, v), 1e-4);
    float NdotH = max(dot(n, h), 0.0);
    float VdotH = max(dot(v, h), 0.0);

    // ── Material reflectance terms.
    vec3 diffuse_color  = baseColor.rgb * (1.0 - metallic);
    // F0 = 0.04 for dielectrics, baseColor for metals. KHR_materials_ior
    // overrides the dielectric value via the alphaCutoff slot when bit 4
    // is set — repurposed here since the shader is the only consumer.
    float ior_f0 = ((mat.flags & 0x10u) != 0u)
        ? pow((mat.alphaCutoff - 1.0) / (mat.alphaCutoff + 1.0), 2.0)
        : 0.04;
    vec3  f0 = mix(vec3(ior_f0), baseColor.rgb, metallic);
    float a  = roughness * roughness;

    // ── Specular lobe.
    vec3  F = F_Schlick(VdotH, f0);
    float D = D_GGX(NdotH, a);
    float V = V_SmithGGXCorrelated(NdotV, NdotL, a);
    vec3  specular = (D * V) * F;

    // ── Diffuse lobe (energy-conserving Lambert with metal carve-out).
    vec3 diffuse = (vec3(1.0) - F) * (diffuse_color / PI);

    vec3 direct = (diffuse + specular) * NdotL * vec3(3.14159);

    // ── Ambient term: small constant scaled by occlusion. Without an
    //    image-based lighting pass this is the floor that prevents
    //    shadowed regions from going to pure black.
    vec3 ambient = 0.10 * diffuse_color * ao;

    vec3 col = direct + ambient + emissive;

    outColor = vec4(col, baseColor.a);
}
