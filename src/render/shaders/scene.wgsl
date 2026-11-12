@group(1) @binding(0) var t_array: texture_2d_array<f32>;
@group(1) @binding(1) var s_array: sampler;
@group(1) @binding(2) var t_shadow: texture_depth_2d;
@group(1) @binding(3) var s_shadow: sampler_comparison;
@group(1) @binding(4) var t_key: texture_depth_2d;
@group(1) @binding(5) var t_atlas: texture_2d<f32>;
@group(3) @binding(0) var t_joints: texture_2d<f32>;
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

/// Skinned vertex: the same inputs plus joints and weights.
struct SIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec3<f32>,
    @location(3) color: vec4<f32>,
    @location(4) m0: vec4<f32>,
    @location(5) m1: vec4<f32>,
    @location(6) m2: vec4<f32>,
    @location(7) icol: vec4<f32>,
    @location(8) ipar: vec4<f32>,
    @location(9) joints: vec4<u32>,
    @location(10) weights: vec4<f32>,
};

/// Joint matrices live in a float texture, three texels (rows) per matrix, 256 per row.
fn joint_mat(i: u32) -> mat4x4<f32> {
    let x = i32((i % 256u) * 3u);
    let y = i32(i / 256u);
    let r0 = textureLoad(t_joints, vec2<i32>(x, y), 0);
    let r1 = textureLoad(t_joints, vec2<i32>(x + 1, y), 0);
    let r2 = textureLoad(t_joints, vec2<i32>(x + 2, y), 0);
    return mat4x4<f32>(
        vec4<f32>(r0.x, r1.x, r2.x, 0.0),
        vec4<f32>(r0.y, r1.y, r2.y, 0.0),
        vec4<f32>(r0.z, r1.z, r2.z, 0.0),
        vec4<f32>(r0.w, r1.w, r2.w, 1.0));
}

fn skinned(in: SIn) -> VIn {
    let base = u32(max(in.ipar.x, 0.0));
    let m = joint_mat(base + in.joints.x) * in.weights.x + joint_mat(base + in.joints.y) * in.weights.y
          + joint_mat(base + in.joints.z) * in.weights.z + joint_mat(base + in.joints.w) * in.weights.w;
    var v: VIn;
    v.pos = (m * vec4<f32>(in.pos, 1.0)).xyz;
    v.normal = (m * vec4<f32>(in.normal, 0.0)).xyz;
    v.uv = in.uv;
    v.color = in.color;
    v.m0 = in.m0;
    v.m1 = in.m1;
    v.m2 = in.m2;
    v.icol = in.icol;
    v.ipar = vec4<f32>(-1.0, in.ipar.y, in.ipar.z, in.ipar.w);
    return v;
}

@vertex
fn vs_main(in: VIn) -> VOut {
    return vs_body(in);
}

@vertex
fn vs_skin(in: SIn) -> VOut {
    return vs_body(skinned(in));
}

@vertex
fn vs_skin_shadow(in: SIn) -> @builtin(position) vec4<f32> {
    return G.shadow_vp * vec4<f32>(world_pos(skinned(in)), 1.0);
}

@vertex
fn vs_skin_key(in: SIn) -> @builtin(position) vec4<f32> {
    return KF * vec4<f32>(world_pos(skinned(in)), 1.0);
}

fn vs_body(in: VIn) -> VOut {
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
    let arr = textureSample(t_array, s_array, uv, i32(min(in.par.x, 250.0) + 0.5));
    let atl = textureSample(t_atlas, s_array, in.uv);
    let tex = select(arr, atl, in.par.x > 250.0);
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
    // rim light keeps silhouettes readable in fog and darkness
    let rim = pow(1.0 - max(dot(n, V), 0.0), 3.0);
    var col = albedo * light + emis + spec * (puddle * 0.9 + wet * 0.08) + albedo * rim * (G.fog_col.rgb * 1.5 + G.ambient.rgb * 0.6);
    col = mix(col, vec3<f32>(3.0, 3.0, 3.0), in.par.y);
    return vec4<f32>(col, 1.0);
}
