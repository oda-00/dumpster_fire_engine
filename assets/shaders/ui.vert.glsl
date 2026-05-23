#version 460

layout(push_constant) uniform Push {
    vec2 screen_size;
    vec2 _pad;
} pc;

layout(location = 0) in vec2 in_pos;
layout(location = 1) in vec2 in_uv;
layout(location = 2) in vec4 in_color;   // R8G8B8A8_UNORM unpacked to vec4

layout(location = 0) out vec2 v_uv;
layout(location = 1) out vec4 v_color;

void main() {
    // Map pixel-space [0,screen] → NDC [-1,1]. Y flipped (Vulkan +Y down).
    vec2 ndc = (in_pos / pc.screen_size) * 2.0 - 1.0;
    gl_Position = vec4(ndc.x, ndc.y, 0.0, 1.0);
    v_uv    = in_uv;
    v_color = in_color;
}
