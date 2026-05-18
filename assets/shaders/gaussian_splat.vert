#version 450
//
// Pass-through vertex shader for the GaussianSplat raster pipeline.
// Vertices come pre-projected from splat_billboard.comp.glsl — clip
// space already, including the perspective w. We just forward the
// per-vertex ellipse UV + premultiplied colour to the fragment shader
// which evaluates the 2D Gaussian falloff.

layout(location = 0) in vec4 inClipPos;
layout(location = 1) in vec2 inEllipseUv;
layout(location = 2) in vec4 inColour;

layout(location = 0) out vec2 outEllipseUv;
layout(location = 1) out vec4 outColour;

void main() {
    gl_Position  = inClipPos;
    outEllipseUv = inEllipseUv;
    outColour    = inColour;
}
