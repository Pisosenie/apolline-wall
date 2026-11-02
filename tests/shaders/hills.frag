// Hills in haze, tinted by the shell's accent, drifting slowly.

float hash(vec2 p) {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

float noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 u = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash(i), hash(i + vec2(1.0, 0.0)), u.x),
               mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), u.x), u.y);
}

float ridge(float x, float seed) {
    float v = 0.0;
    float a = 0.5;
    for (int i = 0; i < 5; i++) {
        v += a * noise(vec2(x, seed));
        x *= 2.03;
        a *= 0.5;
    }
    return v;
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = fragCoord / iResolution.xy;
    float aspect = iResolution.x / iResolution.y;
    float t = iTime * 0.02;

    vec3 night = vec3(0.015, 0.02, 0.035);
    vec3 color = mix(night, iAccent * 0.28, smoothstep(0.1, 1.0, uv.y));
    vec2 sun = vec2(0.72, 0.7);
    float glow = exp(-length((uv - sun) * vec2(aspect, 1.0)) * 5.0);
    color += iAccent * glow * 0.55;

    for (int i = 3; i >= 0; i--) {
        float depth = float(i) / 3.0;
        float x = uv.x * aspect * (1.2 + 1.3 * (1.0 - depth)) + t * (0.4 + 2.0 * (1.0 - depth));
        float height = 0.18 + 0.14 * depth + 0.16 * ridge(x, float(i) * 13.0);
        float below = smoothstep(height + 0.002, height - 0.002, uv.y);
        vec3 hill = mix(night * 0.6, mix(iAccent * 0.45, color, 0.55), depth);
        color = mix(color, hill, below);
    }

    fragColor = vec4(color, 1.0);
}
