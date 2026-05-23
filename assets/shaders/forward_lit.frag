#version 450
#extension GL_EXT_nonuniform_qualifier : enable

// Full Cook-Torrance PBR fragment shader.  Supports the complete 20-variant
// LightKind taxonomy via a UBO loop, with raster approximations for kinds that
// are RT-only in the full path.
//
// Set 0 binding 1 = LightsUBO (FRAGMENT)
// Set 1 layout:
//   binding 0 = MaterialUbo
//   binding 1 = base-colour texture
//   binding 2 = metallic-roughness texture (G = roughness, B = metallic)
//   binding 3 = normal texture
//   binding 4 = emissive texture
//   binding 5 = occlusion texture (R = AO strength)

layout(location = 0) in  vec3 inNormal;
layout(location = 1) in  vec2 inUv;
layout(location = 2) in  vec3 inWorldPos;
layout(location = 0) out vec4 outColor;

// ─── Lights UBO (set 0 binding 1) ─────────────────────────────────────────

struct LightGpu {
    vec4  color_intensity;   // rgb = linear color, a = intensity (cd/lux/lm)
    uint  kind;              // 0..19
    uint  flags;             // bit0 = two_sided, bit1 = hidden
    uint  _p0;
    uint  _p1;
    vec4  d[6];              // per-variant packed data (96 B)
};

layout(set = 0, binding = 1) uniform Lights {
    uint     count;
    uint     env_idx;
    uint     sky_present;
    uint     _pad;
    LightGpu lights[32];
} u_lights;

// ─── Material UBO + textures (set 1) ──────────────────────────────────────

layout(set = 1, binding = 0) uniform MaterialUbo {
    vec4  baseColorFactor;
    float metallicFactor;
    float roughnessFactor;
    vec3  emissiveFactor;
    float alphaCutoff;
    uint  flags;             // bit0=doubleSided, bits1-2=alphaMode, bit3=unlit, bit4=hasIor
} mat;

layout(set = 1, binding = 1) uniform sampler2D texBaseColor;
layout(set = 1, binding = 2) uniform sampler2D texMetallicRoughness;
layout(set = 1, binding = 3) uniform sampler2D texNormal;
layout(set = 1, binding = 4) uniform sampler2D texEmissive;
layout(set = 1, binding = 5) uniform sampler2D texOcclusion;

const float PI = 3.14159265358979323846;

// ─── Cook-Torrance helpers ─────────────────────────────────────────────────

float D_GGX(float NdotH, float a) {
    float a2 = a * a;
    float d  = (NdotH * NdotH) * (a2 - 1.0) + 1.0;
    return a2 / (PI * d * d);
}

float V_Smith(float NdotV, float NdotL, float a) {
    float a2 = a * a;
    float ggxV = NdotL * sqrt(NdotV * NdotV * (1.0 - a2) + a2);
    float ggxL = NdotV * sqrt(NdotL * NdotL * (1.0 - a2) + a2);
    return 0.5 / max(ggxV + ggxL, 1e-5);
}

vec3 F_Schlick(float VdotH, vec3 f0) {
    return f0 + (vec3(1.0) - f0) * pow(clamp(1.0 - VdotH, 0.0, 1.0), 5.0);
}

// glTF KHR_lights_punctual smooth distance cutoff.
// d2 = squared distance to the representative point, inv_r2 = 1/range^2.
// Returns 1.0 when range is unbounded (inv_r2 == 0).
float smooth_cutoff(float d2, float inv_r2) {
    if (inv_r2 == 0.0) return 1.0;
    float x = clamp(1.0 - d2 * d2 * inv_r2 * inv_r2, 0.0, 1.0);
    return x * x;
}

// ─── Normal map reconstruction from screen-space derivatives ───────────────

vec3 perturb_normal(vec3 n_geom, vec3 view_pos, vec2 uv) {
    vec3 dp1   = dFdx(view_pos);
    vec3 dp2   = dFdy(view_pos);
    vec2 duv1  = dFdx(uv);
    vec2 duv2  = dFdy(uv);
    vec3 dp2p  = cross(dp2, n_geom);
    vec3 dp1p  = cross(n_geom, dp1);
    vec3 t     = dp2p * duv1.x + dp1p * duv2.x;
    vec3 b     = dp2p * duv1.y + dp1p * duv2.y;
    float imax = inversesqrt(max(dot(t, t), dot(b, b)));
    mat3 tbn   = mat3(t * imax, b * imax, n_geom);
    vec3 samp  = texture(texNormal, uv).xyz * 2.0 - 1.0;
    return normalize(tbn * samp);
}

// ─── Area-light helper: closest point on a rectangle ──────────────────────

// Returns closest point on the parallelogram defined by center c, two
// half-axes a (tangent × half_w) and b (bitangent × half_h), to query point p.
vec3 closest_point_on_rect(vec3 p, vec3 c, vec3 axis_a, vec3 axis_b, float hw, float hh) {
    vec3 d = p - c;
    float u = clamp(dot(d, normalize(axis_a)), -hw, hw);
    float v = clamp(dot(d, normalize(axis_b)), -hh, hh);
    return c + normalize(axis_a) * u + normalize(axis_b) * v;
}

// ─── Polygon helper (≤4 verts inline; ≥5 falls back to AABB centroid) ─────

vec3 closest_point_on_polygon(LightGpu lt, vec3 p) {
    // d[4].xy = v0.xy, d[4].zw = v1.xy, d[5].xy = v2.xy, d[5].zw = v3.xy
    // For simplicity in the raster path, project p onto the polygon plane
    // and find the closest edge midpoint.  RT resolves the real convex polygon.
    vec3 c = lt.d[0].xyz;
    vec3 tangent   = normalize(lt.d[1].xyz);
    vec3 bitangent = normalize(lt.d[2].xyz);
    vec3 d = p - c;
    int  vcnt = int(lt.d[0].w);
    float hw = 0.5, hh = 0.5;
    if (vcnt >= 2) {
        vec2 v0 = lt.d[4].xy, v1 = lt.d[4].zw;
        hw = max(abs(v0.x), abs(v1.x));
        hh = max(abs(v0.y), abs(v1.y));
    }
    float u = clamp(dot(d, tangent),   -hw, hw);
    float v = clamp(dot(d, bitangent), -hh, hh);
    return c + tangent * u + bitangent * v;
}

// ─── SH9 ambient evaluation stubs ─────────────────────────────────────────
// Without a live HDRI upload path the coefficient arrays are not yet bound;
// return a neutral ambient term so the shader compiles and links correctly.
// When the IES/HDRI bindless arrays land (Phase 6) these stubs are replaced.

vec3 sh9_eval(uint env_idx, vec3 n) {
    // Placeholder: neutral grey until HDRI SH9 coefficients are bound.
    return vec3(0.05);
}

vec3 sh9_eval_sky(uint model_id, vec3 sun_dir, float turbidity, vec3 ground, vec3 n) {
    // Placeholder: simple sky gradient projected onto normal hemisphere.
    float t = 0.5 * (n.y + 1.0);
    return mix(ground * 0.1, vec3(0.5, 0.7, 1.0) * 0.3, t);
}

// ─── IES photometric sample stub ──────────────────────────────────────────
// Real implementation reads from set 0 binding 5 (bindless R32_SFLOAT 2D LUT).
// Stub returns 1.0 (uniform intensity) until the bindless array is wired.

float ies_sample(uint ies_idx, vec3 neg_l, vec3 light_dir) {
    return 1.0;
}

// ─── Main ─────────────────────────────────────────────────────────────────

void main() {
    vec4 baseColor = texture(texBaseColor, inUv) * mat.baseColorFactor;

    uint alpha_mode = mat.flags & 0x6u;
    if (alpha_mode == 0x2u && baseColor.a < mat.alphaCutoff) discard;
    if (alpha_mode == 0x0u) baseColor.a = 1.0;

    // KHR_materials_unlit: bypass all lighting.
    if ((mat.flags & 0x8u) != 0u) {
        outColor = baseColor;
        return;
    }

    vec4  mr        = texture(texMetallicRoughness, inUv);
    float roughness = clamp(mr.g * mat.roughnessFactor, 0.045, 1.0);
    float metallic  = clamp(mr.b * mat.metallicFactor,  0.0,   1.0);
    float ao        = texture(texOcclusion, inUv).r;
    vec3  emissive  = texture(texEmissive,  inUv).rgb * mat.emissiveFactor;

    vec3 n = normalize(inNormal);
    n = perturb_normal(n, inWorldPos, inUv);

    // View vector toward an assumed camera position of (0, 0, 5).
    // Phase 8 will replace this with the real cam_pos from a push constant.
    vec3 v = normalize(vec3(0.0, 0.0, 5.0) - inWorldPos);

    float ior_f0 = ((mat.flags & 0x10u) != 0u)
        ? pow((mat.alphaCutoff - 1.0) / (mat.alphaCutoff + 1.0), 2.0)
        : 0.04;
    vec3 albedo = baseColor.rgb;
    vec3 f0     = mix(vec3(ior_f0), albedo, metallic);
    float rough = roughness;
    float a     = rough * rough;

    // ─── 20-kind direct lighting loop ─────────────────────────────────────

    vec3 direct = vec3(0.0);

    for (uint i = 0u; i < u_lights.count; i++) {
        LightGpu lt = u_lights.lights[i];
        if ((lt.flags & 2u) != 0u) continue;  // hidden

        vec3  l;
        float att;
        bool  skip = false;

        if (lt.kind == 0u) {
            // Point
            vec3  dv  = lt.d[0].xyz - inWorldPos;
            float d2  = dot(dv, dv);
            float dd  = sqrt(d2);
            l   = dv / max(dd, 1e-5);
            att = (1.0 / max(d2, 1e-5)) * smooth_cutoff(d2, lt.d[0].w);

        } else if (lt.kind == 1u) {
            // Spot
            vec3  dv  = lt.d[0].xyz - inWorldPos;
            float d2  = dot(dv, dv);
            float dd  = sqrt(d2);
            l   = dv / max(dd, 1e-5);
            att = (1.0 / max(d2, 1e-5)) * smooth_cutoff(d2, lt.d[0].w);
            float cosT = dot(-l, lt.d[1].xyz);
            att *= smoothstep(lt.d[1].w, lt.d[2].x, cosT);

        } else if (lt.kind == 2u) {
            // Directional
            l   = -lt.d[0].xyz;
            att = 1.0;

        } else if (lt.kind == 3u) {
            // Sun (raster ignores angular_radius)
            l   = -lt.d[0].xyz;
            att = 1.0;

        } else if (lt.kind == 4u) {
            // Sphere (most-representative-point: clamp to surface)
            vec3  dv  = lt.d[0].xyz - inWorldPos;
            float dd  = max(length(dv), lt.d[0].w);
            l   = dv / dd;
            att = (1.0 / (dd * dd)) * smooth_cutoff(dd * dd, lt.d[1].x);

        } else if (lt.kind == 5u) {
            // Disk (one-sided by default; flags bit0 = two_sided)
            vec3  dv  = lt.d[0].xyz - inWorldPos;
            float d2  = dot(dv, dv);
            float dd  = sqrt(d2);
            l = dv / max(dd, 1e-5);
            float face = ((lt.flags & 1u) != 0u)
                ? abs(dot(-l, lt.d[1].xyz))
                : max(dot(-l, lt.d[1].xyz), 0.0);
            att = face / max(d2, 1e-5) * smooth_cutoff(d2, lt.d[1].w);

        } else if (lt.kind == 6u) {
            // Rectangle (MRP)
            vec3 p = closest_point_on_rect(inWorldPos,
                lt.d[0].xyz, lt.d[1].xyz, lt.d[2].xyz, lt.d[1].w, lt.d[0].w);
            vec3  dv = p - inWorldPos;
            float d2 = dot(dv, dv);
            float dd = sqrt(d2);
            l = dv / max(dd, 1e-5);
            float face = ((lt.flags & 1u) != 0u)
                ? abs(dot(-l, lt.d[3].xyz))
                : max(dot(-l, lt.d[3].xyz), 0.0);
            att = face / max(d2, 1e-5) * smooth_cutoff(d2, lt.d[2].w);

        } else if (lt.kind == 7u) {
            // Polygon (LTC placeholder via closest point)
            vec3 p  = closest_point_on_polygon(lt, inWorldPos);
            vec3 dv = p - inWorldPos;
            float d2 = dot(dv, dv);
            float dd = sqrt(d2);
            l = dv / max(dd, 1e-5);
            float face = ((lt.flags & 1u) != 0u)
                ? abs(dot(-l, lt.d[3].xyz))
                : max(dot(-l, lt.d[3].xyz), 0.0);
            att = face / max(d2, 1e-5) * smooth_cutoff(d2, lt.d[2].w);

        } else if (lt.kind == 8u) {
            // Linear: closest point on segment
            vec3  ab  = lt.d[1].xyz - lt.d[0].xyz;
            float t   = clamp(dot(inWorldPos - lt.d[0].xyz, ab) / max(dot(ab, ab), 1e-5), 0.0, 1.0);
            vec3  p   = lt.d[0].xyz + t * ab;
            vec3  dv  = p - inWorldPos;
            float d2  = dot(dv, dv);
            float dd  = sqrt(d2);
            l   = dv / max(dd, 1e-5);
            att = (1.0 / max(d2, 1e-5)) * smooth_cutoff(d2, lt.d[1].w);

        } else if (lt.kind == 9u) {
            // Tube: closest point on cylinder surface
            vec3  ab   = lt.d[1].xyz - lt.d[0].xyz;
            float ab2  = dot(ab, ab);
            float t    = dot(inWorldPos - lt.d[0].xyz, ab) / max(ab2, 1e-5);
            bool capped = lt.d[2].x > 0.5;
            if (capped) t = clamp(t, 0.0, 1.0);
            vec3  axis_pt = lt.d[0].xyz + t * ab;
            vec3  radial  = inWorldPos - axis_pt;
            float rdist   = length(radial);
            vec3  p       = axis_pt + radial * (lt.d[0].w / max(rdist, 1e-5));
            vec3  dv      = p - inWorldPos;
            float d2      = dot(dv, dv);
            float dd      = sqrt(d2);
            l   = dv / max(dd, 1e-5);
            att = (1.0 / max(d2, 1e-5)) * smooth_cutoff(d2, lt.d[1].w);

        } else if (lt.kind == 10u) {
            // Volumetric sphere: raster soft-falloff approximation
            vec3  dv = lt.d[0].xyz - inWorldPos;
            float dd = length(dv);
            float r  = lt.d[0].w;
            l = dv / max(dd, 1e-5);
            float tc = clamp(1.0 - dd / max(r, 1e-5), 0.0, 1.0);
            att = tc * tc;

        } else if (lt.kind == 11u) {
            // VolumeBox: raster AABB soft-falloff
            vec3 dv = lt.d[0].xyz - inWorldPos;
            vec3 he = lt.d[1].xyz;
            vec3 q  = abs(dv) - he;
            float box_d = length(max(q, vec3(0.0))) + min(max(q.x, max(q.y, q.z)), 0.0);
            l   = -dv / max(length(dv), 1e-5);
            att = clamp(1.0 - box_d * 0.1, 0.0, 1.0);

        } else if (lt.kind == 12u) {
            // VolumeCone: raster inside-cone falloff
            vec3  apex = lt.d[0].xyz;
            float ha   = lt.d[0].w;
            vec3  dir  = lt.d[1].xyz;
            float h    = lt.d[1].w;
            vec3  to_p = inWorldPos - apex;
            float along    = dot(to_p, dir);
            float cone_rad = along * tan(ha);
            float perp     = length(to_p - along * dir);
            float inside   = clamp((cone_rad - perp) / max(cone_rad, 1e-5), 0.0, 1.0)
                           * clamp(1.0 - along / max(h, 1e-5), 0.0, 1.0);
            l   = normalize(apex - inWorldPos);
            att = inside;

        } else if (lt.kind == 13u) {
            // VolumeCylinder: raster cylinder inside falloff
            vec3  base = lt.d[0].xyz;
            float r    = lt.d[0].w;
            vec3  dir  = lt.d[1].xyz;
            float h    = lt.d[1].w;
            vec3  to_p = inWorldPos - base;
            float along  = dot(to_p, dir);
            float perp   = length(to_p - along * dir);
            float inside = clamp((r - perp) / max(r, 1e-5), 0.0, 1.0)
                         * clamp(1.0 - abs(along - h * 0.5) / max(h * 0.5, 1e-5), 0.0, 1.0);
            l   = -dir;
            att = inside;

        } else if (lt.kind == 14u) {
            // VolumeMesh: raster AABB of the mesh (RT does real ray-march)
            vec3 mn = lt.d[2].xyz;
            vec3 mx = lt.d[3].xyz;
            vec3 c  = (mn + mx) * 0.5;
            vec3 he = (mx - mn) * 0.5;
            vec3 dv = c - inWorldPos;
            vec3 q  = abs(dv) - he;
            float box_d = length(max(q, vec3(0.0))) + min(max(q.x, max(q.y, q.z)), 0.0);
            l   = -dv / max(length(dv), 1e-5);
            att = clamp(1.0 - box_d * 0.1, 0.0, 1.0);

        } else if (lt.kind == 15u) {
            // IES photometric light
            vec3  dv = lt.d[0].xyz - inWorldPos;
            float d2 = dot(dv, dv);
            float dd = sqrt(d2);
            l = dv / max(dd, 1e-5);
            uint ies_idx = uint(lt.d[0].w);
            att = ies_sample(ies_idx, -l, lt.d[1].xyz) / max(d2, 1e-5);

        } else {
            // Kinds 16..19 (Mesh, Environment, AnalyticSky, Ambient):
            // these contribute outside the direct loop below.
            skip = true;
        }

        if (!skip) {
            float NdotL = max(dot(n, l), 0.0);
            if (NdotL > 0.001 && att > 0.001) {
                vec3  h_    = normalize(v + l);
                float NoH   = max(dot(n,  h_), 0.0);
                float NoV   = max(dot(n,  v),  1e-4);
                float VoH   = max(dot(v,  h_), 0.0);
                vec3  F_    = F_Schlick(VoH, f0);
                float D_    = D_GGX(NoH, a);
                float V_    = V_Smith(NoV, NdotL, a);
                vec3  spec  = F_ * (D_ * V_);
                vec3  diff  = (vec3(1.0) - F_) * (1.0 - metallic) * albedo / PI;
                direct += (diff + spec)
                    * lt.color_intensity.rgb
                    * (lt.color_intensity.a * att * NdotL);
            }
        }
    }

    // ─── Ambient / IBL pass (kinds 17..19) ────────────────────────────────

    vec3 ambient = vec3(0.03) * albedo * ao;

    for (uint i = 0u; i < u_lights.count; i++) {
        LightGpu lt = u_lights.lights[i];
        if ((lt.flags & 2u) != 0u) continue;

        if (lt.kind == 17u) {
            // Environment: HDRI SH9 ambient
            uint env_id = uint(lt.d[0].x);
            ambient += sh9_eval(env_id, n) * lt.color_intensity.a * lt.d[0].z;

        } else if (lt.kind == 18u) {
            // AnalyticSky: precomputed SH9 from Hosek/Preetham/Atmos
            uint model_id = uint(lt.d[1].w);
            ambient += sh9_eval_sky(model_id, lt.d[0].xyz, lt.d[0].w, lt.d[1].xyz, n);

        } else if (lt.kind == 19u) {
            // Ambient: flat color × intensity
            ambient += lt.color_intensity.rgb * lt.color_intensity.a;
        }
        // Kind 16 (Mesh light): emission arrives via primary-ray hits on the
        // emissive mesh itself; RT path adds NEE on top.  No raster contribution.
    }

    vec3 col = direct + ambient + emissive;
    outColor  = vec4(col, baseColor.a);
}
