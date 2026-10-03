precision highp float;
uniform float alpha;
uniform vec2 size;
varying vec2 v_coords;

uniform vec4 rect;
uniform float radius;
uniform float sigma;
uniform float strength;
uniform vec4 hole;

float sd_box(vec2 p, vec2 b, float r) {
    vec2 q = abs(p) - b + vec2(r);
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - r;
}

void main() {
    vec2 p = v_coords * size;
    float d = sd_box(p - (rect.xy + rect.zw * 0.5), rect.zw * 0.5, radius);
    float x = clamp(d / max(sigma, 0.5) * 2.3, -15.0, 15.0);
    float e = exp(x);
    float a = 1.0 - e / (e + 1.0);
    float dh = sd_box(p - (hole.xy + hole.zw * 0.5), hole.zw * 0.5, radius);
    a *= clamp(dh + 0.5, 0.0, 1.0);
    gl_FragColor = vec4(0.0, 0.0, 0.0, a * strength * alpha);
}
