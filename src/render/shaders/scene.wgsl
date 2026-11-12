@group(1) @binding(0) var t_array: texture_2d_array<f32>;
@group(1) @binding(1) var s_array: sampler;
@group(1) @binding(2) var t_shadow: texture_depth_2d;
@group(1) @binding(3) var s_shadow: sampler_comparison;
@group(1) @binding(4) var t_key: texture_depth_2d;
@group(1) @binding(5) var t_atlas: texture_2d<f32>;
@group(1) @binding(6) var t_mat: texture_2d_array<f32>;
@group(1) @binding(7) var t_char: texture_2d_array<f32>;
@group(1) @binding(8) var t_char_mat: texture_2d_array<f32>;
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
    let clip = G.view_proj * vec4<f32>(w, 1.0);
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

/// Sun shadow with a rotated Poisson-disc PCF: soft, contact-hardening-free penumbrae.
fn sun_shadow(wp: vec3<f32>, n: vec3<f32>, jitter: f32) -> f32 {
    let sp = G.shadow_vp * vec4<f32>(wp + n * 0.08, 1.0);
    let suv = sp.xy * vec2<f32>(0.5, -0.5) + 0.5;
    if (suv.x <= 0.0 || suv.x >= 1.0 || suv.y <= 0.0 || suv.y >= 1.0 || sp.z >= 1.0) { return 1.0; }
    let texel = 1.0 / vec2<f32>(textureDimensions(t_shadow));
    var disc = array<vec2<f32>, 8>(
        vec2<f32>(-0.613, 0.617), vec2<f32>(0.170, -0.040), vec2<f32>(-0.299, 0.791), vec2<f32>(0.645, 0.493),
        vec2<f32>(-0.651, 0.717), vec2<f32>(0.421, 0.027), vec2<f32>(-0.817, -0.271), vec2<f32>(-0.705, -0.668));
    let a = jitter * 6.2832;
    let rot = mat2x2<f32>(cos(a), sin(a), -sin(a), cos(a));
    var acc = 0.0;
    for (var i = 0; i < 8; i = i + 1) {
        acc = acc + textureSampleCompareLevel(t_shadow, s_shadow, suv + rot * disc[i] * texel * 2.2, sp.z - 0.0012);
    }
    return acc / 8.0;
}

/// Perturb the geometric normal with a tangent-space normal, using a cotangent frame built from
/// screen-space derivatives (no stored tangents needed).
fn perturb(n: vec3<f32>, dp1: vec3<f32>, dp2: vec3<f32>, duv1: vec2<f32>, duv2: vec2<f32>, tn: vec3<f32>) -> vec3<f32> {
    let dp2perp = cross(dp2, n);
    let dp1perp = cross(n, dp1);
    let t = dp2perp * duv1.x + dp1perp * duv2.x;
    let b = dp2perp * duv1.y + dp1perp * duv2.y;
    let m = max(max(dot(t, t), dot(b, b)), 1e-14);
    let inv = inverseSqrt(m);
    // image rows run down (+v), OpenGL normal maps point +Y up the image
    let r = t * inv * tn.x - b * inv * tn.y + n * tn.z;
    if (dot(r, r) < 1e-8) { return n; }
    return normalize(r);
}

const PI: f32 = 3.14159265;
fn d_ggx(nh: f32, a: f32) -> f32 {
    let a2 = a * a;
    let d = nh * nh * (a2 - 1.0) + 1.0;
    return a2 / (PI * d * d + 1e-6);
}
/// Height-correlated Smith visibility (approximation), includes the 1 / (4 nl nv) term.
fn v_smith(nv: f32, nl: f32, a: f32) -> f32 {
    let gv = nl * (nv * (1.0 - a) + a);
    let gl = nv * (nl * (1.0 - a) + a);
    return 0.5 / max(gv + gl, 1e-5);
}
fn f_schlick(f0: vec3<f32>, vh: f32) -> vec3<f32> {
    return f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - vh, 5.0);
}

struct Surf {
    n: vec3<f32>,
    v: vec3<f32>,
    diff: vec3<f32>,
    f0: vec3<f32>,
    a: f32,
    sss: f32,
};

/// Radiance reflected toward the eye from a light of colour `c` in direction `l`.
fn shade(s: Surf, l: vec3<f32>, c: vec3<f32>) -> vec3<f32> {
    let nl_raw = dot(s.n, l);
    let nl = max(nl_raw, 0.0);
    let h = normalize(l + s.v);
    let nv = max(dot(s.n, s.v), 1e-3);
    let nh = max(dot(s.n, h), 0.0);
    let vh = max(dot(s.v, h), 0.0);
    let f = f_schlick(s.f0, vh);
    let spec = f * d_ggx(nh, s.a) * v_smith(nv, nl, s.a);
    // skin and wet flesh: wrapped diffuse with a blood-red scatter in the terminator
    let wrap = max((nl_raw + 0.45) / 1.45, 0.0);
    let scatter = vec3<f32>(1.0, 0.32, 0.22) * (wrap - nl) * s.sss;
    let diffuse = s.diff * (vec3<f32>(nl) * (1.0 - s.sss * 0.35) + scatter) * (vec3<f32>(1.0) - f) / PI;
    return (diffuse + spec * nl) * c * PI;
}

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let li = i32(min(in.par.x, 250.0) + 0.5);
    let ci = i32(max(in.par.x - 300.0, 0.0) + 0.5);
    let is_char = in.par.x > 299.5;
    let arr0 = textureSample(t_array, s_array, uv, li);
    let mat0 = textureSample(t_mat, s_array, uv, li);
    let carr = textureSample(t_char, s_array, uv, ci);
    let cmat = textureSample(t_char_mat, s_array, uv, ci);
    let arr = select(arr0, carr, is_char);
    // character materials carry subsurface amount where photo layers carry metalness
    let mat_s = select(mat0, vec4<f32>(cmat.xyz, 0.0), is_char);
    let atl = textureSample(t_atlas, s_array, in.uv);
    let dp1 = dpdx(in.wpos);
    let dp2 = dpdy(in.wpos);
    let duv1 = dpdx(uv);
    let duv2 = dpdy(uv);
    let is_atlas = in.par.x > 250.0 && in.par.x < 299.5;
    let tex = select(arr, atl, is_atlas);
    if (tex.a < 0.5) { discard; }
    if (in.par.z > 0.0 && hash21(floor(uv * 48.0) + floor(in.wpos.xz * 3.0)) < in.par.z) { discard; }
    var albedo = tex.rgb * in.color.rgb;
    var mat = select(mat_s, vec4<f32>(0.5, 0.5, 0.62, 0.0), is_atlas);
    let ng = normalize(in.normal);
    let V = normalize(G.cam_pos.xyz - in.wpos);
    let txy = mat.xy * 2.0 - 1.0;
    let tn = vec3<f32>(txy, sqrt(max(1.0 - dot(txy, txy), 0.04)));
    var n = perturb(ng, dp1, dp2, duv1, duv2, tn);
    let jit = ign(in.clip.xy);

    // wet ground: darker, glossier, with puddles that flatten the normal into a mirror
    let wet = G.sun_col.w * smoothstep(0.7, 0.95, ng.y);
    let puddle = wet * smoothstep(0.45, 0.62, vnoise(in.wpos.xz * 0.45) + vnoise(in.wpos.xz * 1.7) * 0.12);
    var rough = mat.z;
    rough = mix(rough, rough * 0.45, wet);
    rough = mix(rough, 0.03, puddle);
    n = normalize(mix(n, ng, puddle * 0.9));
    albedo = albedo * (1.0 - 0.3 * wet) * (1.0 - 0.25 * puddle);

    // blood coverage (skinned bodies pack it into the integer part of the flash parameter):
    // splashes and runs that grow with damage taken, glossy while wet
    var flash = clamp(in.par.y, 0.0, 1.0);
    if (in.par.y > 1.0) {
        flash = fract(in.par.y);
        let blood = floor(in.par.y) / 15.0;
        let bn = vnoise(uv * 23.0) * 0.55 + vnoise(uv * 71.0 + 3.7) * 0.3 + vnoise(vec2<f32>(uv.x * 9.0, uv.y * 41.0)) * 0.15;
        let m = smoothstep(1.02 - blood, 1.1 - blood, bn);
        let clot = vnoise(uv * 140.0);
        albedo = mix(albedo, vec3<f32>(0.16, 0.012, 0.01) * (0.6 + 0.6 * clot), m * 0.92);
        rough = mix(rough, 0.18 + 0.3 * clot, m);
    }

    let li_u = u32(max(in.par.x, 0.0) + 0.5);
    // Skin, Flesh, Vein, GutWall (see textures::Tex) scatter light under the surface
    var sss = select(0.0, 1.0, li_u == 24u || li_u == 25u || li_u == 26u || li_u == 43u);
    if (is_char) { sss = cmat.w; }
    let metal = mat.w;
    var s: Surf;
    s.n = n;
    s.v = V;
    s.a = max(rough * rough, 0.002);
    s.diff = albedo * (1.0 - metal);
    s.f0 = mix(vec3<f32>(0.04), albedo, metal);
    s.sss = sss;

    // sun / moon with a soft shadow map
    var sh = 1.0;
    if (G.cam_right.w > 0.5) {
        // a fixed kernel: with no temporal accumulation, per-pixel rotation reads as halftone noise
        sh = sun_shadow(in.wpos, ng, 0.13);
    }
    var col = shade(s, G.sun_dir.xyz, G.sun_col.rgb * sh);

    let nl = i32(G.counts.x);
    for (var i = 0; i < nl; i = i + 1) {
        let lp = G.light_pos[i];
        let L = lp.xyz - in.wpos;
        let d2 = dot(L, L);
        let r2 = lp.w * lp.w;
        if (d2 < r2) {
            // inverse-square falloff windowed to the light radius
            let win = 1.0 - (d2 / r2);
            var att = win * win * (1.0 + 4.0 / (1.0 + d2));
            if (i == 0 && G.key.w > 0.5) {
                att = att * key_shadow(in.wpos + ng * 0.06);
            }
            col = col + shade(s, L * inverseSqrt(max(d2, 1e-4)), G.light_col[i].rgb * att);
        }
    }

    // image-based ambient: sky above, bounce from the ground below, and a fogged environment
    // reflection with roughness-aware Fresnel
    let sky = G.ambient.rgb * 1.15 + G.fog_col.rgb * 0.35;
    let ground = G.ambient.rgb * 0.35 + G.sun_col.rgb * 0.04;
    let hemi = mix(ground, sky, n.y * 0.5 + 0.5);
    let nv = max(dot(n, V), 1e-3);
    let fr = s.f0 + (max(vec3<f32>(1.0 - rough), s.f0) - s.f0) * pow(1.0 - nv, 5.0);
    let R = reflect(-V, n);
    let env = mix(ground, sky * 1.3 + G.sun_col.rgb * 0.15 * pow(max(dot(R, G.sun_dir.xyz), 0.0), 8.0), smoothstep(-0.2, 0.3, R.y));
    col = col + s.diff * hemi * (vec3<f32>(1.0) - fr) * (1.0 + sss * 0.15) + env * fr * (1.0 - rough * 0.7);

    // emission (vertex alpha): glowing growths, embers, lit windows
    let luma = dot(tex.rgb, vec3<f32>(0.3, 0.5, 0.2));
    col = col + albedo * in.color.a * 4.0 * smoothstep(0.25, 0.7, luma + in.color.a * 0.2);

    // hit / seizure response: a dark, wet flush across the body instead of a white flash
    let f = flash;
    col = mix(col, col * vec3<f32>(0.55, 0.32, 0.3) + albedo * vec3<f32>(0.08, 0.0, 0.0), f * 0.7);
    return vec4<f32>(col, 1.0);
}
