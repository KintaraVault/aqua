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
uniform vec2 box;
uniform vec4 win;
uniform vec4 tgt;
uniform vec4 clip_rect;
uniform float radius;
uniform float progress;

float sd_box(vec2 p, vec2 b, float r) {
    vec2 q = abs(p) - b + vec2(r);
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - r;
}

void main() {
    vec2 p = v_coords * box;
    float t = progress;
    float bend = smoothstep(0.0, 0.45, t);
    float slide = smoothstep(0.3, 1.0, t);
    float top = mix(win.y, tgt.y, slide);
    float bot = mix(win.y + win.w, tgt.y + tgt.w, smoothstep(0.0, 0.55, t));
    if (p.y < top || p.y > bot || bot - top < 0.5) {
        gl_FragColor = vec4(0.0);
        return;
    }
    float span = max(tgt.y + tgt.w - win.y, 1.0);
    float ny = clamp((p.y - win.y) / span, 0.0, 1.0);
    float s = bend * smoothstep(0.0, 1.0, ny);
    float l = mix(win.x, tgt.x, s);
    float r = mix(win.x + win.z, tgt.x + tgt.z, s);
    if (p.x < l || p.x > r) {
        gl_FragColor = vec4(0.0);
        return;
    }
    vec2 uv = vec2((p.x - l) / max(r - l, 0.001), (p.y - top) / max(bot - top, 0.001));
    vec4 color = texture2D(tex, uv);
#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0);
#endif
    vec2 q = uv * win.zw;
    float d = sd_box(q - (clip_rect.xy + clip_rect.zw * 0.5), clip_rect.zw * 0.5, radius);
    color *= clamp(0.5 - d, 0.0, 1.0);
    color *= alpha * (1.0 - 0.35 * smoothstep(0.6, 1.0, t));
#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif
    gl_FragColor = color;
}
