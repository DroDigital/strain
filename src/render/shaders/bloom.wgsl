// ---- bloom chain -------------------------------------------------------------------------
@group(1) @binding(0) var t_src: texture_2d<f32>;
@group(1) @binding(1) var s_src: sampler;

@fragment
fn fs_bright(in: FsIn) -> @location(0) vec4<f32> {
    let ts = 1.0 / vec2<f32>(textureDimensions(t_src));
    var c = vec3<f32>(0.0);
    c = c + textureSampleLevel(t_src, s_src, in.uv + ts * vec2<f32>(-1.0, -1.0), 0.0).rgb;
    c = c + textureSampleLevel(t_src, s_src, in.uv + ts * vec2<f32>(1.0, -1.0), 0.0).rgb;
    c = c + textureSampleLevel(t_src, s_src, in.uv + ts * vec2<f32>(-1.0, 1.0), 0.0).rgb;
    c = c + textureSampleLevel(t_src, s_src, in.uv + ts * vec2<f32>(1.0, 1.0), 0.0).rgb;
    c = c * 0.25;
    let l = max(c.r, max(c.g, c.b));
    return vec4<f32>(min(c * smoothstep(1.0, 2.2, l), vec3<f32>(8.0)), 1.0);
}
fn blur(uv: vec2<f32>, dir: vec2<f32>) -> vec4<f32> {
    let ts = dir / vec2<f32>(textureDimensions(t_src));
    var c = textureSampleLevel(t_src, s_src, uv, 0.0).rgb * 0.227;
    c = c + (textureSampleLevel(t_src, s_src, uv + ts * 1.385, 0.0).rgb + textureSampleLevel(t_src, s_src, uv - ts * 1.385, 0.0).rgb) * 0.316;
    c = c + (textureSampleLevel(t_src, s_src, uv + ts * 3.231, 0.0).rgb + textureSampleLevel(t_src, s_src, uv - ts * 3.231, 0.0).rgb) * 0.07;
    return vec4<f32>(c, 1.0);
}
@fragment
fn fs_blur_h(in: FsIn) -> @location(0) vec4<f32> { return blur(in.uv, vec2<f32>(1.0, 0.0)); }
@fragment
fn fs_blur_v(in: FsIn) -> @location(0) vec4<f32> { return blur(in.uv, vec2<f32>(0.0, 1.0)); }
// ---- present: upscale the internal image to the window, through an optional retro filter -----

fn texel(p: vec2<f32>) -> vec3<f32> {
    let d = vec2<i32>(textureDimensions(t_src));
    return textureLoad(t_src, clamp(vec2<i32>(p), vec2<i32>(0), d - 1), 0).rgb;
}
// gamma 2.0 is close enough for the tube maths and far cheaper than pow per tap
fn to_lin(c: vec3<f32>) -> vec3<f32> { return c * c; }
fn to_gam(c: vec3<f32>) -> vec3<f32> { return sqrt(max(c, vec3<f32>(0.0))); }

// one scanline of the tube, filtered horizontally like an analogue signal (linear light)
fn crt_row(x: f32, row: f32) -> vec3<f32> {
    let fx = x - 0.5;
    let x0 = floor(fx);
    let f = fx - x0;
    // cubic-ish horizontal reconstruction over four texels
    let w0 = (1.0 - f) * (1.0 - f) * (1.0 - f) / 6.0;
    let w1 = (4.0 - 6.0 * f * f + 3.0 * f * f * f) / 6.0;
    let w2 = (1.0 + 3.0 * f + 3.0 * f * f - 3.0 * f * f * f) / 6.0;
    let w3 = f * f * f / 6.0;
    return to_lin(texel(vec2<f32>(x0 - 1.0, row))) * w0 + to_lin(texel(vec2<f32>(x0, row))) * w1
         + to_lin(texel(vec2<f32>(x0 + 1.0, row))) * w2 + to_lin(texel(vec2<f32>(x0 + 2.0, row))) * w3;
}

// TRINITRON: a curved aperture-grille tube. Bright pixels bloom into wider beams, dark ones
// leave visible gaps; the post pass adds a stronger halation glow from the bloom chain.
fn crt(uv0: vec2<f32>, frag: vec2<f32>) -> vec3<f32> {
    let res = G.screen.xy;
    var c = uv0 * 2.0 - 1.0;
    c = c * (1.0 + vec2<f32>(c.y * c.y * 0.045, c.x * c.x * 0.06));
    let uv = c * 0.5 + 0.5;
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) { return vec3<f32>(0.0); }
    let p = uv * res;
    let y = p.y - 0.5;
    let r0 = floor(y);
    let fy = y - r0;
    let a = crt_row(p.x, r0);
    let b = crt_row(p.x, r0 + 1.0);
    let la = max(a.r, max(a.g, a.b));
    let lb = max(b.r, max(b.g, b.b));
    let sa = mix(0.24, 0.42, sqrt(clamp(la, 0.0, 1.0)));
    let sb = mix(0.24, 0.42, sqrt(clamp(lb, 0.0, 1.0)));
    var col = a * exp(-fy * fy / (2.0 * sa * sa)) + b * exp(-(1.0 - fy) * (1.0 - fy) / (2.0 * sb * sb));
    col = col * 1.55;
    // aperture grille: vertical RGB phosphor stripes, three window pixels wide
    let m = u32(frag.x) % 3u;
    var mask = vec3<f32>(0.78);
    if (m == 0u) { mask.r = 1.12; } else if (m == 1u) { mask.g = 1.12; } else { mask.b = 1.12; }
    col = col * mask;
    // the glass: darker, rounded corners
    let e = abs(c);
    let corner = smoothstep(0.0, 0.035, 1.0 - max(e.x, e.y)) * smoothstep(1.08, 0.75, length(e * e));
    return to_gam(col * corner);
}

fn rgb2yiq(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(c, vec3<f32>(0.299, 0.587, 0.114)), dot(c, vec3<f32>(0.596, -0.274, -0.322)), dot(c, vec3<f32>(0.211, -0.523, 0.312)));
}
fn yiq2rgb(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(c.x + 0.956 * c.y + 0.621 * c.z, c.x - 0.272 * c.y - 0.647 * c.z, c.x - 1.106 * c.y + 1.703 * c.z);
}

// CAMCORDER: a consumer tape. Colour smears sideways behind a sharper picture, lines wobble,
// a tracking band rolls up the screen and the head switch tears the bottom edge.
fn vhs(uv0: vec2<f32>, frag: vec2<f32>) -> vec3<f32> {
    let res = G.screen.xy;
    let t = G.cam_pos.w;
    var uv = uv0;
    let line = floor(uv.y * res.y);
    uv.x = uv.x + (vnoise(vec2<f32>(line * 0.31, t * 9.0)) - 0.5) * 0.0018;
    let band = 1.0 - fract(t * 0.045);
    let inband = smoothstep(0.035, 0.0, abs(uv.y - band));
    uv.x = uv.x + inband * 0.012 * (hash21(vec2<f32>(line, floor(t * 30.0))) - 0.5);
    let hs = smoothstep(0.952, 0.985, uv.y);
    uv.x = uv.x + hs * 0.05 * (vnoise(vec2<f32>(line * 1.7, t * 25.0)) - 0.25);
    let p = uv * res;
    // luma: slightly soft; chroma: wide and late
    var yv = 0.0;
    yv = yv + rgb2yiq(texel(p + vec2<f32>(-0.8, 0.0))).x * 0.25;
    yv = yv + rgb2yiq(texel(p)).x * 0.5;
    yv = yv + rgb2yiq(texel(p + vec2<f32>(0.8, 0.0))).x * 0.25;
    var iq = vec2<f32>(0.0);
    for (var k = 0; k < 8; k = k + 1) {
        iq = iq + rgb2yiq(texel(p + vec2<f32>(f32(k) * 1.1 - 2.5, 0.0))).yz;
    }
    iq = iq / 8.0;
    // sharpening ring on luma edges, as tape decks do
    let ring = yv - rgb2yiq(texel(p + vec2<f32>(-2.5, 0.0))).x;
    yv = yv + ring * 0.18;
    var col = yiq2rgb(vec3<f32>(yv, iq * 1.2));
    // lifted blacks, rolled-off highlights, a warm cast
    col = col * vec3<f32>(0.93, 0.9, 0.86) + vec3<f32>(0.04, 0.035, 0.045);
    col = col / (1.0 + max(col - vec3<f32>(0.8), vec3<f32>(0.0)) * 1.5);
    // noise: fine grain, sparse dropouts, the tracking band and the head-switch hash
    col = col + (hash21(frag * 0.5 + vec2<f32>(t * 61.0, t * 17.0)) - 0.5) * 0.05;
    let drop = step(0.9975, hash21(vec2<f32>(line, floor(t * 20.0)))) * step(hash21(vec2<f32>(floor(uv.x * 9.0), line + t)), 0.35);
    col = mix(col, vec3<f32>(0.9), drop * 0.7);
    col = col + inband * (hash21(frag + t) - 0.3) * 0.12;
    col = mix(col, vec3<f32>(hash21(frag * 0.7 + t) * 0.6), hs * 0.55);
    // interlaced fields
    col = col * (0.94 + 0.06 * sin(uv0.y * res.y * 3.14159 * 2.0));
    let vg = length((uv0 - 0.5) * vec2<f32>(1.0, 1.15));
    return col * (1.0 - 0.35 * smoothstep(0.45, 0.85, vg));
}

// 32-BIT: crisp, unfiltered pixels (the colour depth and dithering happen in the post pass)
fn psx(uv: vec2<f32>) -> vec3<f32> {
    let col = texel(uv * G.screen.xy);
    let vg = length((uv - 0.5) * vec2<f32>(1.0, 1.1));
    return col * (1.0 - 0.18 * smoothstep(0.5, 0.9, vg));
}

@fragment
fn fs_blit(in: FsIn) -> @location(0) vec4<f32> {
    let mode = i32(G.post.x + 0.5);
    var c: vec3<f32>;
    if (mode == 1) {
        c = crt(in.uv, in.pos.xy);
    } else if (mode == 2) {
        c = vhs(in.uv, in.pos.xy);
    } else if (mode == 3) {
        c = psx(in.uv);
    } else {
        c = textureSampleLevel(t_src, s_src, in.uv, 0.0).rgb;
    }
    c = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(mix(c, pow(c, vec3<f32>(2.2)), G.cam_up.w), 1.0);
}
