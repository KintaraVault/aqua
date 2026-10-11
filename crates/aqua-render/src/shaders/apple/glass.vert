#version 300 es
// linuxtosh: vertex stage for Apple's glass_background_sdf (replaces sdf_filter_vert, which
// takes CoreAnimation's own vertex layout). Positions are physical px of the output.
precision highp float;
uniform mat3 u_proj;      // Smithay projection: physical px -> NDC
uniform vec4 u_vp;        // GL viewport
uniform vec4 u_fb_rect;   // backdrop capture rect in framebuffer px
uniform vec4 u_dst;       // element rect, physical px
uniform vec2 u_sdf_size;  // shape size in sdf units (unscaled physical px)
in vec2 a_pos;            // 0..1 within u_dst
out vec2 in_sdf_uv;
out vec2 in_src_uv;
void main() {
    vec2 p = u_dst.xy + a_pos * u_dst.zw;
    vec3 ndc = u_proj * vec3(p, 1.0);
    gl_Position = vec4(ndc.xy, 0.0, 1.0);
    vec2 fb = u_vp.xy + (ndc.xy + 1.0) * 0.5 * u_vp.zw;
    in_src_uv = (fb - u_fb_rect.xy) / u_fb_rect.zw;
    in_sdf_uv = (a_pos - 0.5) * u_sdf_size;
}
