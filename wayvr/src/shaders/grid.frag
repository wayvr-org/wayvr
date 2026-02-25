#version 310 es
precision highp float;

layout (location = 0) in vec2 in_uv;
layout (location = 0) out vec4 out_color;

void main()
{
    float fade = max(1.0 - 2.0 * length(in_uv.xy + vec2(-0.5, -0.5)), 0.0);
    float grid;

    if (fract(in_uv.x / 0.0005) < 0.01 || fract(in_uv.y / 0.0005) < 0.01) {
        grid = 1.0;
    } else {
        grid = 0.0;
    }
    out_color = vec4(1.0, 1.0, 1.0, grid * fade);
}
