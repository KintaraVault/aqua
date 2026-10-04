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

// Liquid glass: a convex slab over a blurred backdrop.
//  * the rim is a squircle-profiled bevel that bends the backdrop inward (lens), strongest at
//    the very edge, with a hint of dispersion there only;
//  * light comes from the top-left: a crisp specular line on the lit and the opposite edge
//    (light passing through the slab), fading on the edges parallel to the light, plus a soft
//    inner glow and a faint caustic band just inside the bevel;
//  * the body keeps the backdrop's colour (vibrancy) under the tint; dithered against banding.

float sd_box(vec2 p, vec2 b, float r) {
    vec2 q = abs(p) - b + vec2(r);
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - r;
}

vec3 sample_at(vec2 local_disp) {
    vec2 frag = gl_FragCoord.xy + local_disp.x * axes.xy + local_disp.y * axes.zw;
    vec2 uv = clamp((frag - fb_rect.xy) / fb_rect.zw, vec2(0.002), vec2(0.998));
    return texture2D(blur_tex, uv).rgb;
}

float luma(vec3 c) { return dot(c, vec3(0.2126, 0.7152, 0.0722)); }

float hash(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
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

    // Bevel profile: t = 0 at the edge … 1 where the flat top begins. The slope of a squircle
    // cap (1 - (1-t)^4)^(1/4) gives a displacement that is large at the rim and falls off fast.
    float bw = max(min(bevel, min(hb.x, hb.y)), 1.0);
    float t = clamp(-d / bw, 0.0, 1.0);
    float e = 1.0 - t;
    float e4 = e * e * e * e;
    float slope = e * e * e / pow(max(1.0 - e4, 1e-3), 0.75);
    float lens = clamp(slope * 0.42, 0.0, 1.0);
    vec2 disp = -n * lens * refraction;

    vec3 col;
    if (refraction > 0.01) {
        float ca = 0.07 * lens;
        col.r = sample_at(disp * (1.0 + ca)).r;
        col.g = sample_at(disp).g;
        col.b = sample_at(disp * (1.0 - ca)).b;
    } else {
        col = sample_at(vec2(0.0));
    }

    float l = luma(col);
    col = clamp(mix(vec3(l), col, saturation), 0.0, 1.0);
    if (max_luma < 0.999) {
        float L = luma(col);
        float knee = max_luma * 0.7;
        float span = max(max_luma - knee, 1e-3);
        float Lc = L < knee ? L : knee + span * (1.0 - exp(-(L - knee) / span));
        col *= Lc / max(L, 1e-4);
        // Dimming a light backdrop greys it out; give back the chroma it lost.
        col = clamp(mix(vec3(Lc), col, 1.0 + 0.9 * (1.0 - Lc / max(L, 1e-4))), 0.0, 1.0);
    }
    // Tint: a light tint lifts the backdrop toward it while keeping some of its hue
    // (multiply-screen mix) instead of washing everything to one flat milky colour.
    vec3 flat_mix = mix(col, tint.rgb, tint.a);
    vec3 screen = 1.0 - (1.0 - col) * (1.0 - tint.rgb * tint.a);
    float lt = smoothstep(0.55, 0.95, luma(tint.rgb));
    col = mix(flat_mix, screen, lt * 0.55);

    // Lighting.
    vec2 light = normalize(vec2(-0.6, -0.8));
    float ndl = dot(n, light);
    float lit = pow(max(ndl, 0.0), 2.6);
    float through = pow(max(-ndl, 0.0), 3.0);
    float edge_line = 1.0 - smoothstep(0.0, 1.3, -d);
    float edge_soft = 1.0 - smoothstep(0.0, 3.5, -d);
    float inner = 1.0 - smoothstep(0.0, bw * 0.55, -d);
    float band = smoothstep(0.0, 0.35, t) * (1.0 - smoothstep(0.35, 1.0, t));

    float bgl = luma(col);
    float rim_gain = rim * (1.15 - 0.45 * bgl);
    col += rim_gain * edge_line * (0.07 + 0.95 * lit + 0.60 * through);
    col += rim_gain * edge_soft * edge_soft * (0.30 * lit + 0.18 * through);
    col += rim_gain * 0.20 * inner * inner * (lit + 0.5 * through);
    col += rim * 0.035 * band;
    // Faint top sheen and a soft shade toward the bottom edge give the slab some volume.
    col += rim * 0.045 * (1.0 - v_coords.y);
    col *= 1.0 - rim * 0.05 * inner * max(n.y, 0.0);

    // Dither (±0.5/255) against banding in large smooth blurs.
    col += (hash(gl_FragCoord.xy) - 0.5) / 255.0;

    gl_FragColor = vec4(clamp(col, 0.0, 1.0), 1.0) * cov * alpha;
}
