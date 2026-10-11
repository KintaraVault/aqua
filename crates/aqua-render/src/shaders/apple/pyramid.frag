#version 300 es
// linuxtosh: one level of the backdrop mip pyramid that Apple's glass samples with
// textureLod(log2(radius)): a 2x downsample with a 13-tap (dual-kawase style) filter.
precision highp float;
uniform sampler2D tex;
uniform vec2 halfpixel;
uniform float lod;
in vec2 uv;
out vec4 color;
void main() {
    vec4 s = textureLod(tex, uv, lod) * 4.0;
    s += textureLod(tex, uv - halfpixel, lod);
    s += textureLod(tex, uv + halfpixel, lod);
    s += textureLod(tex, uv + vec2(halfpixel.x, -halfpixel.y), lod);
    s += textureLod(tex, uv - vec2(halfpixel.x, -halfpixel.y), lod);
    color = vec4((s / 8.0).rgb, 1.0);
}
