#version 450
//
// KHR_gaussian_splatting billboard projection.
//
// Per dispatched thread: take one splat (in the sorted order produced by
// splat_sort.comp.glsl), project its 3D Gaussian to a 2D screen-space
// ellipse, and emit a 6-vertex billboard quad enclosing the 3σ contour
// into the output vertex buffer the GaussianSplat raster pipeline reads.
//
// Vertex layout per emitted vertex (40 bytes — matches forge.rs's
// `bd_splat` binding):
//   vec4 clip_pos;     // pre-projected clip-space xyzw
//   vec2 ellipse_uv;   // unit-disc UV for the per-fragment Gaussian
//   vec4 colour;       // premultiplied RGBA
//
// Math overview (per the 3D Gaussian Splatting paper, EWA splatting):
//   1.  Build 3D covariance Σ = (R*S) * (R*S)^T  where S = diag(scale),
//       R is the quaternion-derived rotation matrix.
//   2.  Transform to view-space: Σ_view = V3 * Σ * V3^T.
//   3.  Linearise the perspective projection around the splat centre
//       to get a 2x3 Jacobian J. The 2D screen covariance is
//       Σ_2D = J * Σ_view * J^T, plus a (0.3, 0.3) anti-aliasing bias
//       on the diagonal.
//   4.  Eigendecompose Σ_2D to get the major/minor screen axes and
//       extents (3σ corners along each axis).
//   5.  Two triangles, vertices at center ± half_extent_x * v_major ±
//       half_extent_y * v_minor; ellipse_uv is the unit-quad coordinate.

layout(local_size_x = 256, local_size_y = 1, local_size_z = 1) in;

// Sorted (key, index) pairs produced by splat_sort.comp.glsl.
struct SortPair {
    uint key;
    uint index;
};
layout(set = 0, binding = 0, std430) readonly buffer Sorted {
    SortPair pairs[];
} sorted_buf;

// Per-splat raw inputs. Each array is `splat_count` long. Layout chosen
// so the host can blit forge_gltf's `GaussianSplatSet` streams directly
// — vec4 storage for vec3 inputs adds a wasted lane but keeps every
// std430 access naturally aligned.
layout(set = 0, binding = 1, std430) readonly buffer SplatPositions {
    vec4 positions[]; // .xyz used, .w ignored
} pos_buf;
layout(set = 0, binding = 2, std430) readonly buffer SplatScales {
    vec4 scales[];    // .xyz used, .w ignored
} scale_buf;
layout(set = 0, binding = 3, std430) readonly buffer SplatRotations {
    vec4 rotations[]; // quaternion (x, y, z, w)
} rot_buf;
layout(set = 0, binding = 4, std430) readonly buffer SplatColors {
    vec4 colors[];    // RGBA, NOT yet premultiplied
} color_buf;
layout(set = 0, binding = 5, std430) readonly buffer SplatOpacities {
    float opacities[];
} opacity_buf;

// Output vertex stream — 6 vertices per splat.
struct SplatVertex {
    vec4 clip_pos;
    vec2 ellipse_uv;
    vec2 _pad0;       // pad to 32 bytes for vec4 colour alignment
    vec4 colour;
};
layout(set = 0, binding = 6, std430) writeonly buffer OutVerts {
    SplatVertex verts[];
} out_buf;

layout(push_constant) uniform Pc {
    mat4 view;        // camera view matrix
    mat4 projection;  // camera projection matrix
    vec2 viewport;    // (width, height) in pixels
    uint splat_count;
    uint _pad;
} pc;

mat3 quat_to_mat3(vec4 q) {
    float x = q.x, y = q.y, z = q.z, w = q.w;
    return mat3(
        1.0 - 2.0*(y*y + z*z),     2.0*(x*y + w*z),       2.0*(x*z - w*y),
            2.0*(x*y - w*z),   1.0 - 2.0*(x*x + z*z),     2.0*(y*z + w*x),
            2.0*(x*z + w*y),       2.0*(y*z - w*x),   1.0 - 2.0*(x*x + y*y)
    );
}

void main() {
    uint sid = gl_GlobalInvocationID.x;
    if (sid >= pc.splat_count) return;

    uint splat_idx = sorted_buf.pairs[sid].index;

    vec3 p_world = pos_buf.positions[splat_idx].xyz;
    vec3 scale   = scale_buf.scales[splat_idx].xyz;
    vec4 q       = rot_buf.rotations[splat_idx];
    vec4 col     = color_buf.colors[splat_idx];
    float opac   = opacity_buf.opacities[splat_idx];

    // 3D covariance Σ = (R*S) * (R*S)^T
    mat3 R = quat_to_mat3(q);
    mat3 S = mat3(
        scale.x, 0.0, 0.0,
        0.0, scale.y, 0.0,
        0.0, 0.0, scale.z
    );
    mat3 M = R * S;
    mat3 sigma3 = M * transpose(M);

    // View-space covariance
    mat3 V3 = mat3(pc.view);
    mat3 sigma_view = V3 * sigma3 * transpose(V3);

    // Camera-space splat centre
    vec4 t4 = pc.view * vec4(p_world, 1.0);
    vec3 t = t4.xyz;

    // Cull behind-camera and at-clipping-plane splats — write a degenerate
    // quad (all clip_pos w=0 so the rasterizer discards it).
    if (t.z >= -0.001) {
        SplatVertex degen;
        degen.clip_pos   = vec4(0.0);
        degen.ellipse_uv = vec2(0.0);
        degen._pad0      = vec2(0.0);
        degen.colour     = vec4(0.0);
        for (uint i = 0u; i < 6u; ++i) {
            out_buf.verts[sid * 6u + i] = degen;
        }
        return;
    }

    // Jacobian of the projection at t. We use the focal lengths
    // implied by pc.projection: fx = projection[0][0] * 0.5 * viewport.x,
    // fy = projection[1][1] * 0.5 * viewport.y.
    float fx = pc.projection[0][0] * 0.5 * pc.viewport.x;
    float fy = pc.projection[1][1] * 0.5 * pc.viewport.y;
    float inv_tz = 1.0 / -t.z;     // OpenGL/Vulkan view-space looks down -Z
    mat3 J = mat3(
        fx * inv_tz, 0.0, 0.0,
        0.0, fy * inv_tz, 0.0,
        -fx * t.x * inv_tz * inv_tz, -fy * t.y * inv_tz * inv_tz, 0.0
    );

    // 2D screen covariance Σ_2D = J * Σ_view * J^T (2x2 upper-left).
    mat3 cov_full = J * sigma_view * transpose(J);
    float a = cov_full[0][0] + 0.3;
    float b = cov_full[0][1];
    float c = cov_full[1][1] + 0.3;

    // Eigendecompose [[a, b], [b, c]] → eigenvalues l1 >= l2, eigenvectors
    // v1 and v2 (orthonormal).
    float trace = a + c;
    float det   = a * c - b * b;
    float disc  = max(0.0, trace * trace * 0.25 - det);
    float sd    = sqrt(disc);
    float l1    = 0.5 * trace + sd;
    float l2    = 0.5 * trace - sd;

    vec2 v1, v2;
    if (abs(b) < 1e-6) {
        // Diagonal already — axis-aligned ellipse.
        v1 = vec2(1.0, 0.0);
        v2 = vec2(0.0, 1.0);
    } else {
        v1 = normalize(vec2(b, l1 - a));
        v2 = vec2(-v1.y, v1.x);  // perpendicular
    }

    // 3σ half-extents in screen pixels.
    float half_x = 3.0 * sqrt(max(l1, 1e-6));
    float half_y = 3.0 * sqrt(max(l2, 1e-6));

    // Project centre to clip space + viewport-pixel offsets to NDC.
    vec4 center_clip = pc.projection * t4;
    // Convert pixel deltas to NDC deltas. ndc_per_pixel = 2 / viewport.
    vec2 ndc_per_px = vec2(2.0 / pc.viewport.x, 2.0 / pc.viewport.y);

    // 4 corner offsets in (v1, v2) frame, half_extent units.
    const vec2 corners[4] = vec2[4](
        vec2(-1.0, -1.0),
        vec2( 1.0, -1.0),
        vec2( 1.0,  1.0),
        vec2(-1.0,  1.0)
    );
    vec4 corner_clip[4];
    vec2 corner_uv[4];
    for (uint i = 0u; i < 4u; ++i) {
        vec2 c   = corners[i];
        vec2 d   = c.x * v1 * half_x + c.y * v2 * half_y;     // pixel offset
        vec2 dn  = d * ndc_per_px * center_clip.w;             // NDC * w
        corner_clip[i] = vec4(center_clip.xy + dn, center_clip.zw);
        corner_uv[i]   = c; // -1..+1 unit-quad coordinate
    }

    // Pre-multiply alpha into colour. Per the spec, opacity multiplies
    // colour.a; rendering uses premultiplied output (the pipeline blend
    // mode is ONE / ONE_MINUS_SRC_ALPHA).
    float alpha = clamp(col.a * opac, 0.0, 1.0);
    vec4 premul = vec4(col.rgb * alpha, alpha);

    // Two triangles per quad: (0,1,2) and (0,2,3).
    const uint tri_idx[6] = uint[6](0u, 1u, 2u, 0u, 2u, 3u);
    for (uint i = 0u; i < 6u; ++i) {
        uint c = tri_idx[i];
        SplatVertex v;
        v.clip_pos   = corner_clip[c];
        v.ellipse_uv = corner_uv[c];
        v._pad0      = vec2(0.0);
        v.colour     = premul;
        out_buf.verts[sid * 6u + i] = v;
    }
}
