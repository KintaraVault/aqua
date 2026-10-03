#version 100
precision highp float;
uniform sampler2D tex;
uniform vec2 halfpixel;
uniform float offset;
varying vec2 uv;
void main() {
    vec4 sum = texture2D(tex, uv) * 4.0;
    sum += texture2D(tex, uv - halfpixel * offset);
    sum += texture2D(tex, uv + halfpixel * offset);
    sum += texture2D(tex, uv + vec2(halfpixel.x, -halfpixel.y) * offset);
    sum += texture2D(tex, uv - vec2(halfpixel.x, -halfpixel.y) * offset);
    gl_FragColor = sum / 8.0;
}
