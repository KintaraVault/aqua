#version 100
#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif
precision highp float;
#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif
uniform float alpha;
varying vec2 v_coords;
#if defined(DEBUG_FLAGS)
uniform float tint;
#endif
uniform vec2 elem_size;
uniform vec4 clip_rect;
uniform float radius;

// QuartzCore's continuous-corner ("squircle") SDF, the same function the Apple glass shader
// uses for the window chrome (CA::OGL::Metal::ShaderUtils::supercircle_sdf, mode 4), so the
// client's clip and the glass frame share one outline. Each axis blends to a circular arc
// when the side is too short for the 1.528 r continuous extent (QuartzCore's own rule).
float sd_box(vec2 p, vec2 b, float r) {
    r = min(r, min(b.x, b.y));
    if (r <= 0.0) {
        vec2 q = abs(p) - b;
        return min(max(q.x, q.y), 0.0) + length(max(q, 0.0));
    }
    vec2 t = clamp((1.528665 - b / r) * 1.8915572, 0.0, 1.0);
    vec2 a = abs(p);
    float e = 1.528665 * r;
    vec2 q = a - b + vec2(mix(e, r, max(t.x, t.y)));
    vec2 u = max(vec2(0.0), (a - b + vec2(e)) / e);
    float len = length(u);
    float mx = max(u.x, u.y);
    float k = mx == 0.0 ? 0.0 : clamp(min(u.x, u.y) / mx, 0.0, 1.0);
    float poly = (((3.15601 - 0.926054 * k) * k - 3.64122) * k + 1.26803) * k + 0.268531;
    float sc = len + 1.0 - 1.0 / (1.0 - k * k * clamp(len, 0.0, 1.0) * poly);
    float circ = length(max(vec2(0.0), u * 1.528665 - 0.528665)) * 0.65416557 + 0.34583443;
    float fx = mix(sc, circ, t.x);
    float fy = mix(sc, circ, t.y);
    float sg = u.y > u.x ? 1.0 : -1.0;
    float f = mix(fx, fy, clamp(0.5 - sg + sg * k, 0.0, 1.0));
    return min(max(q.x, q.y), 0.0) + e * (f - 1.0);
}

void main() {
    vec4 color = texture2D(tex, v_coords);
#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0) * alpha;
#else
    color = color * alpha;
#endif
    vec2 p = v_coords * elem_size;
    float d = sd_box(p - (clip_rect.xy + clip_rect.zw * 0.5), clip_rect.zw * 0.5, radius);
    color *= clamp(0.5 - d, 0.0, 1.0);
#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif
    gl_FragColor = color;
}
