precision highp float;
uniform float alpha;
uniform vec2 size;
varying vec2 v_coords;

// Liquid Glass — a port of Liquid Glass Studio's material (github.com/iyinchao/liquid-glass-studio)
// to the compositor, tuned for real-time use:
//  * shape: rounded rect with superellipse corners, signed distance + analytic gradient in one go
//    (no extra SDF taps for the normal);
//  * rim: Snell refraction through a convex slab — the backdrop is displaced along the normal by
//    -tan(θt - θi), per channel (dispersion), optionally from the sharp backdrop at the very edge;
//  * lights: a Fresnel edge band and a directional glare computed in LCH, so highlights keep the
//    hue of what is behind them;
//  * body: blurred, saturated, luminance-capped backdrop under the tint; dithered.
// Everything beyond plain sampling + tint only runs inside the rim band.

uniform sampler2D blur_tex;   // blurred backdrop (unit 1)
uniform sampler2D sharp_tex;  // unblurred backdrop (unit 2)
uniform vec4 fb_rect;         // capture rect in framebuffer px
uniform vec4 axes;            // local → framebuffer axes
uniform vec4 shape;           // corner extent px, superellipse exponent, thickness px, 1/ior
uniform vec4 refr;            // refraction px, dispersion, sharp rim (0/1), saturation
uniform vec4 fres;            // fresnel k (1/px), fresnel hardness, fresnel factor, rim gain
uniform vec4 glare;           // glare k (1/px), glare hardness, glare factor, convergence
uniform vec4 glare2;          // opposite factor, light angle (rad), max luma, normal gain
uniform vec4 tint;
uniform vec3 fres_lch;        // LCH of the Fresnel colour (white mixed with half the tint)

const float PI = 3.14159265359;

// --- colour: sRGB <-> CIE LCh (D65) ---------------------------------------------------------
const mat3 RGB_TO_XYZ_M = mat3(0.4124, 0.3576, 0.1805, 0.2126, 0.7152, 0.0722, 0.0193, 0.1192, 0.9505);
const mat3 XYZ_TO_RGB_M = mat3(3.2406255, -1.537208, -0.4986286, -0.9689307, 1.8757561, 0.0415175,
                               0.0557101, -0.2040211, 1.0569959);
const vec3 WHITE = vec3(0.95045592705, 1.0, 1.08905775076);

vec3 srgb_to_lin(vec3 c) {
    return mix(c / 12.92, pow((c + 0.055) / 1.055, vec3(2.4)), step(vec3(0.04045), c));
}
vec3 lin_to_srgb(vec3 c) {
    c = max(c, vec3(0.0));
    return mix(12.92 * c, 1.055 * pow(c, vec3(0.41666666666)) - 0.055, step(vec3(0.0031308), c));
}
vec3 lab_f(vec3 x) {
    return mix(7.78703703704 * x + 0.13793103448, pow(max(x, vec3(0.0)), vec3(0.333333333)),
               step(vec3(0.00885645167), x));
}
vec3 lab_finv(vec3 x) {
    return mix(0.12841854934 * (x - 0.137931034), x * x * x, step(vec3(0.206897), x));
}
vec3 srgb_to_lch(vec3 c) {
    vec3 f = lab_f((srgb_to_lin(c) * RGB_TO_XYZ_M) / WHITE);
    vec3 lab = vec3(116.0 * f.y - 16.0, 500.0 * (f.x - f.y), 200.0 * (f.y - f.z));
    return vec3(lab.x, length(lab.yz), atan(lab.z, lab.y));
}
vec3 lch_to_srgb(vec3 lch) {
    vec2 ab = lch.y * vec2(cos(lch.z), sin(lch.z));
    float w = (lch.x + 16.0) / 116.0;
    vec3 xyz = WHITE * lab_finv(vec3(w + ab.x / 500.0, w, w - ab.y / 200.0));
    return lin_to_srgb(xyz * XYZ_TO_RGB_M);
}

// --- shape ---------------------------------------------------------------------------------
// Signed distance of a rounded rect with superellipse corners (half size hb, corner extent cr,
// exponent n) and its gradient. Like the reference, the corner metric is the superellipse
// norm, so the gradient is shorter than 1 around the diagonal (corners refract a bit less).
vec3 shape_sdf(vec2 p, vec2 hb, float cr, float n) {
    vec2 sg = vec2(p.x < 0.0 ? -1.0 : 1.0, p.y < 0.0 ? -1.0 : 1.0);
    vec2 ap = abs(p);
    vec2 d = ap - hb;
    if (cr > 0.0 && d.x > -cr && d.y > -cr) {
        vec2 q = max(ap - (hb - vec2(cr)), vec2(0.0));
        float m = max(q.x, q.y);
        if (m < 1e-4) {
            return vec3(-cr, 0.0, 0.0);
        }
        vec2 u = q / m;
        float v = m * pow(pow(u.x, n) + pow(u.y, n), 1.0 / n);
        vec2 g = pow(q / v, vec2(n - 1.0)) * sg;
        return vec3(v - cr, g);
    }
    float dist = min(max(d.x, d.y), 0.0) + length(max(d, vec2(0.0)));
    vec2 g = d.x > d.y ? vec2(sg.x, 0.0) : vec2(0.0, sg.y);
    return vec3(dist, g);
}

// --- backdrop ------------------------------------------------------------------------------
vec2 fb_uv(vec2 local_disp) {
    vec2 frag = gl_FragCoord.xy + local_disp.x * axes.xy + local_disp.y * axes.zw;
    return clamp((frag - fb_rect.xy) / fb_rect.zw, vec2(0.0005), vec2(0.9995));
}

float luma(vec3 c) { return dot(c, vec3(0.2126, 0.7152, 0.0722)); }

// Vibrancy: saturation boost and a soft luminance cap for legibility.
vec3 grade(vec3 col) {
    float l = luma(col);
    col = clamp(mix(vec3(l), col, refr.w), 0.0, 1.0);
    float max_luma = glare2.z;
    if (max_luma < 0.999) {
        float L = luma(col);
        float knee = max_luma * 0.7;
        float span = max(max_luma - knee, 1e-3);
        float Lc = L < knee ? L : knee + span * (1.0 - exp(-(L - knee) / span));
        col *= Lc / max(L, 1e-4);
        col = clamp(mix(vec3(Lc), col, 1.0 + 0.9 * (1.0 - Lc / max(L, 1e-4))), 0.0, 1.0);
    }
    return col;
}

float hash(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

void main() {
    vec2 hb = size * 0.5;
    vec2 c = v_coords * size - hb;
    vec3 sd = shape_sdf(c, hb, shape.x, shape.y);
    float d = sd.x;
    float cov = clamp(0.5 - d, 0.0, 1.0);
    if (cov <= 0.0) {
        discard;
    }
    float depth = max(-d, 0.0);
    float thick = shape.z;
    vec3 col;

    if (depth >= thick) {
        // Flat top of the slab: the plain (blurred) backdrop under the tint.
        col = grade(texture2D(blur_tex, fb_uv(vec2(0.0))).rgb);
        col = mix(col, tint.rgb, tint.a);
    } else {
        vec2 grad = sd.yz;
        float glen = length(grad);
        // Snell: the slab's height profile turns the incidence angle from grazing (rim) to
        // straight on (flat top); the backdrop shifts by the refracted ray's deviation.
        float x = 1.0 - depth / thick;
        float ti = asin(clamp(x * x, 0.0, 1.0));
        float tt = asin(clamp(shape.w * sin(ti), -1.0, 1.0));
        float edge = -tan(tt - ti);
        vec2 disp = -grad * edge * refr.x;
        float k = 0.02 * refr.y;
        vec2 uvr = fb_uv(disp * (1.0 + k));
        vec2 uvg = fb_uv(disp);
        vec2 uvb = fb_uv(disp * (1.0 - k));
        vec3 bl = vec3(texture2D(blur_tex, uvr).r, texture2D(blur_tex, uvg).g, texture2D(blur_tex, uvb).b);
        vec3 back = bl;
        if (refr.z > 0.5) {
            // sharp backdrop at the very rim, blurring in toward the flat top
            vec3 sh = vec3(texture2D(sharp_tex, uvr).r, texture2D(sharp_tex, uvg).g, texture2D(sharp_tex, uvb).b);
            back = mix(sh, bl, depth / thick);
        }
        back = grade(back);
        col = mix(back, tint.rgb, tint.a);

        float gain = fres.w * glen * glare2.w;
        if (gain > 0.0) {
            // Fresnel: a thin light band hugging the rim, tinted like the glass.
            float f = clamp(pow(max(1.0 - depth * fres.x + fres.y, 0.0), 5.0), 0.0, 1.0);
            if (f * fres.z > 0.002) {
                vec3 lch = fres_lch;
                lch.x = clamp(lch.x + 20.0 * f * fres.z, 0.0, 100.0);
                col = mix(col, lch_to_srgb(lch), clamp(f * fres.z * 0.7 * gain, 0.0, 1.0));
            }
            // Glare: light from `angle` catches the near rim, and through the slab the far one.
            float geo = clamp(pow(max(1.0 - depth * glare.x + glare.y, 0.0), 5.0), 0.0, 1.0);
            if (geo * glare.z > 0.002 && glen > 1e-4) {
                vec2 nn = grad / glen;
                float th = atan(-nn.y, nn.x);
                if (th < 0.0) th += 2.0 * PI;
                float ga = (th - PI * 0.25 + glare2.y) * 2.0;
                bool far = (ga > PI * 1.5 && ga < PI * 3.5) || ga < -PI * 0.5;
                float a = (0.5 + 0.5 * sin(ga)) * (far ? 1.2 * glare2.x : 1.2) * glare.z;
                a = clamp(pow(max(a, 0.0), 0.1 + glare.w * 2.0), 0.0, 1.0);
                float amt = a * geo;
                if (amt > 0.002) {
                    vec3 gl = srgb_to_lch(mix(bl, tint.rgb, tint.a * 0.5));
                    gl.x = clamp(gl.x + 150.0 * amt, 0.0, 120.0);
                    gl.y += 30.0 * amt;
                    col = mix(col, lch_to_srgb(gl), clamp(amt * gain, 0.0, 1.0));
                }
            }
        }
    }

    // Dither (±0.5/255) against banding in large smooth blurs.
    col += (hash(gl_FragCoord.xy) - 0.5) / 255.0;
    gl_FragColor = vec4(clamp(col, 0.0, 1.0), 1.0) * cov * alpha;
}
