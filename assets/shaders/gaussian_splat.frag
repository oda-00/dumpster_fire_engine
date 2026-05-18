#version 450
//
// GaussianSplat fragment shader. Per the 3D Gaussian Splatting paper:
// each splat's per-pixel coverage is exp(-0.5 * r²) where r is the
// distance from the splat centre in unit-Gaussian space. The vertex
// shader hands us r as `inEllipseUv` in [-1, +1] × [-1, +1] (the 3σ
// quad), so r² = dot(uv, uv) * 9.
//
// The colour is premultiplied; multiplying the per-pixel coverage in
// composites the splat correctly with the ONE / ONE_MINUS_SRC_ALPHA
// blend mode the pipeline sets up.

layout(location = 0) in vec2 inEllipseUv;
layout(location = 1) in vec4 inColour;

layout(location = 0) out vec4 outColor;

void main() {
    // Distance² from splat centre in unit-Gaussian space.
    float r2 = dot(inEllipseUv, inEllipseUv) * 9.0;
    // Cull fragments outside the 3σ contour — they'd contribute
    // < exp(-4.5) ≈ 0.011 which is below the typical 8-bit threshold.
    if (r2 > 9.0) discard;
    float coverage = exp(-0.5 * r2);
    outColor = inColour * coverage;
}
