#version 310 es
precision highp float;

layout(location = 0) in vec2 in_uv;
layout(location = 0) out vec4 out_color;

const float circle_smoothness = 0.0025;
const float circle_thickness = 0.01;
const float circle_opacity = 0.3;
const float circle_size = 0.1;

float calc_grid(vec2 coord, float m) {
  vec2 grid = fract(coord * m);
  return (step(m, grid.x) * step(m, grid.y));
}

float calc_circle(float dist, float size) {
  float c1 = size;
  float c2 = size - circle_thickness;

  return smoothstep(c1, c1 - circle_smoothness, dist) *
         smoothstep(c2 - circle_smoothness, c2, dist);
}

void main() {
  float dist = length(in_uv.xy + vec2(-0.5, -0.5));
  float fade = max(1.0 - 2.0 * dist, 0.0);

  float mask = 1.0 - calc_grid(in_uv.xy * 1000.0, 0.02);

  mask = max(mask, (calc_circle(dist, circle_size) +
                    calc_circle(dist, circle_size * 2.0) +
                    calc_circle(dist, circle_size * 3.0)) *
                       circle_opacity);

  out_color = vec4(1.0, 1.0, 1.0, mask * fade);
}
