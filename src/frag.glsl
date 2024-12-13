precision highp float;

varying vec2 v_tex_coord;
varying vec4 v_color;

uniform sampler2D u_sampler;

void main() {
  gl_FragColor = v_color * texture2D(u_sampler, v_tex_coord);
} 
