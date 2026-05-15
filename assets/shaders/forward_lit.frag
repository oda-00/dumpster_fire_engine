#version 450

layout(location = 0) in  vec3 inNormal;
layout(location = 1) in  vec2 inUv;
layout(location = 0) out vec4 outColor;

void main() {
    // Fixed directional light from upper-right-front.
    vec3  light = normalize(vec3(1.0, 2.0, 3.0));
    float diff  = max(dot(normalize(inNormal), light), 0.0);
    // Warm base colour + ambient floor so the dark side isn't pitch-black.
    vec3  col   = diff * vec3(0.85, 0.78, 0.65) + vec3(0.08, 0.08, 0.12);
    outColor = vec4(col, 1.0);
}
