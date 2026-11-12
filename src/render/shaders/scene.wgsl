@group(1) @binding(0) var t_array: texture_2d_array<f32>;
@group(1) @binding(1) var s_array: sampler;
@group(1) @binding(2) var t_shadow: texture_depth_2d;
@group(1) @binding(3) var s_shadow: sampler_comparison;
@group(1) @binding(4) var t_key: texture_depth_2d;
@group(2) @binding(0) var<uniform> KF: mat4x4<f32>;

struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec3<f32>,
    @location(3) color: vec4<f32>,
    @location(4) m0: vec4<f32>,
    @location(5) m1: vec4<f32>,
    @location(6) m2: vec4<f32>,
    @location(7) icol: vec4<f32>,
    @location(8) ipar: vec4<f32>,
};
struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) wpos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uvw: vec3<f32>,
    @location(3) uv: vec2<f32>,
    @location(4) color: vec4<f32>,
    @location(5) @interpolate(flat) par: vec4<f32>,
};

fn world_pos(in: VIn) -> vec3<f32> {
    let p = vec4<f32>(in.pos, 1.0);
    var w = vec3<f32>(dot(in.m0, p), dot(in.m1, p), dot(in.m2, p));
    let warp = G.ambient.w * in.ipar.z;
    if (warp > 0.0) {
        let t = G.cam_pos.w;
        let h = max(w.y, 0.0);
        w.x = w.x + sin(w.y * 0.13 + t * 0.6 + w.z * 0.05) * h * 0.035 * warp;
        w.z = w.z + cos(w.y * 0.11 + t * 0.5 + w.x * 0.05) * h * 0.035 * warp;
    }
    return w;
}

@vertex
fn vs_main(in: VIn) -> VOut {
    var out: VOut;
    let w = world_pos(in);
    var clip = G.view_proj * vec4<f32>(w, 1.0);
    if (G.screen.z > 0.0 && clip.w > 0.01) {
        let grid = G.screen.xy * 0.5 * G.screen.z;
        let ndc = clip.xy / clip.w;
        clip = vec4<f32>(round(ndc * grid) / grid * clip.w, clip.z, clip.w);
    }
    out.clip = clip;
    out.wpos = w;
    let sx = vec3<f32>(in.m0.x, in.m1.x, in.m2.x);
    let sy = vec3<f32>(in.m0.y, in.m1.y, in.m2.y);
    let sz = vec3<f32>(in.m0.z, in.m1.z, in.m2.z);
    let nl = in.normal / max(vec3<f32>(dot(sx, sx), dot(sy, sy), dot(sz, sz)), vec3<f32>(1e-8));
    out.normal = normalize(vec3<f32>(dot(in.m0.xyz, nl), dot(in.m1.xyz, nl), dot(in.m2.xyz, nl)));
    out.uv = in.uv.xy;
    out.uvw = vec3<f32>(in.uv.xy * clip.w, clip.w);
    out.color = vec4<f32>(in.color.rgb * in.icol.rgb, in.color.a + in.icol.a);
    var layer = in.uv.z;
    if (in.ipar.x >= 0.0) { layer = in.ipar.x; }
    out.par = vec4<f32>(layer, in.ipar.y, in.ipar.w, 0.0);
    return out;
}

@vertex
fn vs_shadow(in: VIn) -> @builtin(position) vec4<f32> {
    return G.shadow_vp * vec4<f32>(world_pos(in), 1.0);
}

@vertex
fn vs_key(in: VIn) -> @builtin(position) vec4<f32> {
    return KF * vec4<f32>(world_pos(in), 1.0);
}

/// Shadow from the key point light, looked up in a 3x2 atlas of 90-degree faces.
fn key_shadow(wp: vec3<f32>) -> f32 {
    let d = wp - G.key.xyz;
    let ad = abs(d);
    var f = 0;
    if (ad.x >= ad.y && ad.x >= ad.z) {
        f = select(1, 0, d.x > 0.0);
    } else if (ad.y >= ad.z) {
        f = select(3, 2, d.y > 0.0);
    } else {
        f = select(5, 4, d.z > 0.0);
    }
    let c = G.key_vp[f] * vec4<f32>(wp, 1.0);
    let ndc = c.xyz / c.w;
    if (ndc.z >= 1.0 || c.w <= 0.0) { return 1.0; }
    let uv = clamp(ndc.xy * vec2<f32>(0.5, -0.5) + 0.5, vec2<f32>(0.004), vec2<f32>(0.996));
    let tile = vec2<f32>(f32(f % 3), f32(f / 3));
    return textureSampleCompareLevel(t_key, s_shadow, (tile + uv) / vec2<f32>(3.0, 2.0), ndc.z - 0.0015);
}

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    let uv = mix(in.uv, in.uvw.xy / in.uvw.z, G.screen.w);
    let tex = textureSample(t_array, s_array, uv, i32(in.par.x + 0.5));
    if (tex.a < 0.5) { discard; }
    if (in.par.z > 0.0 && hash21(floor(uv * 24.0) + floor(in.wpos.xz * 3.0)) < in.par.z) { discard; }
    var albedo = tex.rgb * in.color.rgb;
    let n = normalize(in.normal);
    let V = normalize(G.cam_pos.xyz - in.wpos);

    // sun / moon with shadow map
    var sh = 1.0;
    if (G.cam_right.w > 0.5) {
        let sp = G.shadow_vp * vec4<f32>(in.wpos + n * 0.12, 1.0);
        let suv = sp.xy * vec2<f32>(0.5, -0.5) + 0.5;
        if (suv.x > 0.0 && suv.x < 1.0 && suv.y > 0.0 && suv.y < 1.0 && sp.z < 1.0) {
            sh = textureSampleCompareLevel(t_shadow, s_shadow, suv, sp.z - 0.0015);
        }
    }
    let ndl = max(dot(n, G.sun_dir.xyz), 0.0);
    var light = G.ambient.rgb * (0.65 + 0.35 * n.y) + G.sun_col.rgb * ndl * sh;

    // wet ground
    let wet = G.sun_col.w * smoothstep(0.7, 0.95, n.y);
    let puddle = wet * smoothstep(0.42, 0.62, vnoise(in.wpos.xz * 0.45));
    albedo = albedo * (1.0 - 0.35 * wet);
    let R = reflect(-V, normalize(n + vec3<f32>(vnoise(in.wpos.xz * 9.0) - 0.5, 0.0, vnoise(in.wpos.zx * 9.0) - 0.5) * 0.08));
    var spec = G.sun_col.rgb * pow(max(dot(R, G.sun_dir.xyz), 0.0), 48.0) * sh;

    let nl = i32(G.counts.x);
    for (var i = 0; i < nl; i = i + 1) {
        let lp = G.light_pos[i];
        let L = lp.xyz - in.wpos;
        let d2 = dot(L, L);
        let r2 = lp.w * lp.w;
        if (d2 < r2) {
            var att = 1.0 - d2 / r2;
            att = att * att;
            if (i == 0 && G.key.w > 0.5) {
                att = att * key_shadow(in.wpos + n * 0.06);
            }
            let Ld = L * inverseSqrt(max(d2, 1e-4));
            let c = G.light_col[i].rgb * att;
            light = light + c * (max(dot(n, Ld), 0.0) * 0.85 + 0.15);
            spec = spec + c * pow(max(dot(R, Ld), 0.0), 24.0);
        }
    }
    let luma = dot(tex.rgb, vec3<f32>(0.3, 0.5, 0.2));
    let emis = albedo * in.color.a * 4.0 * smoothstep(0.25, 0.7, luma + in.color.a * 0.2);
    var col = albedo * light + emis + spec * (puddle * 0.9 + wet * 0.08);
    col = mix(col, vec3<f32>(3.0, 3.0, 3.0), in.par.y);
    return vec4<f32>(col, 1.0);
}
