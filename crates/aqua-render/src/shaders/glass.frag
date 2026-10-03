precision highp float;
uniform float alpha;
uniform vec2 size;
varying vec2 v_coords;

uniform sampler2D blur_tex;
uniform vec4 fb_rect;
uniform vec4 axes;
uniform float radius;
uniform vec4 tint;
uniform float saturation;
uniform float refraction;
uniform float bevel;
uniform float rim;
uniform float max_luma;

float sd_box(vec2 p, vec2 b, float r) {
    vec2 q = abs(p) - b + vec2(r);
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - r;
}

vec3 sample_at(vec2 local_disp) {
    vec2 frag = gl_FragCoord.xy + local_disp.x * axes.xy + local_disp.y * axes.zw;
    vec2 uv = clamp((frag - fb_rect.xy) / fb_rect.zw, vec2(0.002), vec2(0.998));
    return texture2D(blur_tex, uv).rgb;
}

void main() {
    vec2 p = v_coords * size;
    vec2 c = p - size * 0.5;
    vec2 hb = size * 0.5;
    float r = min(radius, min(hb.x, hb.y));
    float d = sd_box(c, hb, r);
    float cov = clamp(0.5 - d, 0.0, 1.0);
    if (cov <= 0.0) { discard; }

    vec2 n = vec2(sd_box(c + vec2(1.0, 0.0), hb, r) - sd_box(c - vec2(1.0, 0.0), hb, r),
                  sd_box(c + vec2(0.0, 1.0), hb, r) - sd_box(c - vec2(0.0, 1.0), hb, r));
    n = n / max(length(n), 1e-4);

    float depth = clamp(-d / max(bevel, 1.0), 0.0, 1.0);
    float k = 1.0 - depth;
    float lens = 1.0 - sqrt(max(1.0 - k * k, 0.0));
    vec2 disp = -n * lens * refraction;

    vec3 col;
    col.r = sample_at(disp * 1.06).r;
    col.g = sample_at(disp).g;
    col.b = sample_at(disp * 0.94).b;

    float l = dot(col, vec3(0.2126, 0.7152, 0.0722));
    col = clamp(mix(vec3(l), col, saturation), 0.0, 1.0);
    if (max_luma < 0.999) {
        float L = dot(col, vec3(0.2126, 0.7152, 0.0722));
        float knee = max_luma * 0.7;
        float span = max(max_luma - knee, 1e-3);
        float Lc = L < knee ? L : knee + span * (1.0 - exp(-(L - knee) / span));
        col *= Lc / max(L, 1e-4);
    }
    col = mix(col, tint.rgb, tint.a);

    vec2 light = normalize(vec2(-0.55, -0.85));
    float facing = max(dot(n, light), 0.0);
    float back = max(dot(n, -light), 0.0);
    float line = 1.0 - smoothstep(0.0, 1.6, -d);
    float glow = (1.0 - smoothstep(0.0, bevel * 0.9, -d));
    col += rim * line * (0.18 + 0.75 * facing + 0.40 * back);
    col += rim * 0.16 * glow * (facing + 0.4 * back);
    col += rim * 0.05 * (1.0 - v_coords.y);

    gl_FragColor = vec4(min(col, vec3(1.0)), 1.0) * cov * alpha;
}
