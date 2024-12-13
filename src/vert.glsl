precision highp float;

attribute vec2 a_position;
attribute vec2 a_tex_coord;
attribute vec4 a_color;

uniform vec2 u_view_size;

varying vec2 v_tex_coord;
varying vec4 v_color;

vec2 view_to_world(vec2 view_size, vec2 world_position) {
  return vec2(
    2.0 * world_position.x / view_size.x - 1.0,
    -2.0 * world_position.y / view_size.y + 1.0
  );
}

void main() {
  v_tex_coord = a_tex_coord;
  // NOTE: div by 255.0 normalizes 0..255 u8 into 0.0..1.0 f32
  v_color = a_color / 255.0;
  gl_Position = vec4(view_to_world(u_view_size, a_position), 0.0, 1.0);
}
