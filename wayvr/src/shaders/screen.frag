#version 310 es
precision highp float;

layout (location = 0) in vec2 in_uv;
layout (location = 0) out vec4 out_color;

layout (set = 0, binding = 0) uniform sampler2D in_texture;

void main()
{
    out_color = texture(in_texture, in_uv);

    bvec3 cutoff = lessThan(out_color.rgb, vec3(0.04045));
    vec3 higher = pow((out_color.rgb + vec3(0.055)) / vec3(1.055), vec3(2.4));
    vec3 lower = out_color.rgb / vec3(12.92);

    out_color.rgb = mix(higher, lower, cutoff);
    out_color.a = 1.0;
}

