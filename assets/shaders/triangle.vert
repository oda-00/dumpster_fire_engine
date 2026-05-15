#version 450

layout(location = 0) out vec3 outColor;

// Positions and colours baked in — no vertex buffers, no descriptors.
// gl_VertexIndex selects the corner for this invocation.
vec2 positions[3] = vec2[3](
    vec2( 0.0, -0.5),
    vec2( 0.5,  0.5),
    vec2(-0.5,  0.5)
);

vec3 colors[3] = vec3[3](
    vec3(1.0, 0.0, 0.0),
    vec3(0.0, 1.0, 0.0),
    vec3(0.0, 0.0, 1.0)
);

void main() {
    gl_Position = vec4(positions[gl_VertexIndex], 0.0, 1.0);
    outColor    = colors[gl_VertexIndex];
}
