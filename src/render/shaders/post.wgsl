@group(1) @binding(0) var t_scene: texture_2d<f32>;
@group(1) @binding(1) var t_depth: texture_depth_2d;
@group(1) @binding(2) var t_bloom: texture_2d<f32>;
@group(1) @binding(3) var s_lin: sampler;
@group(1) @binding(4) var t_shadow: texture_depth_2d;
@group(1) @binding(5) var s_shadow: sampler_comparison;

fn bayer(p: vec2<f32>) -> f32 {
    let x = u32(p.x) & 3u;
    let y = u32(p.y) & 3u;
    var m = array<f32, 16>(0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0, 3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0);
    return (m[y * 4u + x] + 0.5) / 16.0;
}

fn shadow_vis(p: vec3<f32>) -> f32 {
    let sp = G.shadow_vp * vec4<f32>(p, 1.0);
    let suv = sp.xy * vec2<f32>(0.5, -0.5) + 0.5;
    if (suv.x <= 0.0 || suv.x >= 1.0 || suv.y <= 0.0 || suv.y >= 1.0 || sp.z >= 1.0) { return 1.0; }
    return textureSampleCompareLevel(t_shadow, s_shadow, suv, sp.z - 0.002);
}

fn world_at(px: vec2<i32>) -> vec4<f32> {
    let d = textureLoad(t_depth, px, 0);
    let uv = (vec2<f32>(px) + 0.5) / G.screen.xy;
    let w = G.inv_view_proj * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
    return vec4<f32>(w.xyz / w.w, d);
}

// Ambient occlusion from depth alone: samples on a rotated disc that sit in front of this pixel
// (closer to the camera, within half a metre) darken creases, corners and contact with the ground.
fn ssao(px: vec2<i32>, dist: f32, ro: vec3<f32>, frag: vec2<f32>) -> f32 {
    if (dist > 45.0) { return 1.0; }
    let r = clamp(0.55 / max(dist, 0.5) * G.screen.y, 2.0, 26.0);
    let rot = bayer(frag) * 6.2831;
    var occ = 0.0;
    for (var i = 0; i < 8; i = i + 1) {
        let a = rot + f32(i) * 2.3999;
        let rr = r * (0.3 + 0.7 * fract(f32(i) * 0.618 + 0.13));
        let sp = px + vec2<i32>(vec2<f32>(cos(a), sin(a)) * rr);
        if (any(sp < vec2<i32>(0)) || any(sp >= vec2<i32>(G.screen.xy))) { continue; }
        let q = world_at(sp);
        if (q.w >= 0.99999) { continue; }
        let dz = dist - length(q.xyz - ro);
        if (dz > 0.04 && dz < 0.9) { occ = occ + (1.0 - dz / 0.9); }
    }
    return 1.0 - 0.55 * clamp(occ / 8.0, 0.0, 1.0) * smoothstep(45.0, 25.0, dist);
}

// filmic tone curve (ACES fit): rich mid-tones, highlights that roll off instead of clipping
fn aces(x: vec3<f32>) -> vec3<f32> {
    return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn fs_post(in: FsIn) -> @location(0) vec4<f32> {
    var uv = in.uv;
    let aspect = G.screen.x / G.screen.y;
    // screen-space distortion from rifts and spells
    let nd = i32(G.counts.y);
    for (var i = 0; i < nd; i = i + 1) {
        let c = G.view_proj * vec4<f32>(G.distort[i].xyz, 1.0);
        if (c.w > 0.1) {
            let suv = c.xy / c.w * vec2<f32>(0.5, -0.5) + 0.5;
            let rs = G.distort[i].w * 1.3 / c.w;
            var dl = uv - suv;
            dl.x = dl.x * aspect;
            let dist = length(dl) / max(rs, 1e-4);
            if (dist < 1.0) {
                let f = (1.0 - dist) * (1.0 - dist) * G.distort2[i].x;
                let ang = f * G.distort2[i].y * 3.0;
                let ca = cos(ang);
                let sa = sin(ang);
                var o = vec2<f32>(dl.x * ca - dl.y * sa, dl.x * sa + dl.y * ca) * (1.0 - f * 0.6 * sin(G.cam_pos.w * 5.0 + dist * 14.0) - f * 0.4);
                o.x = o.x / aspect;
                uv = suv + o;
            }
        }
    }
    let strain = G.fx.w;
    let px = vec2<i32>(clamp(uv, vec2<f32>(0.0), vec2<f32>(0.999)) * G.screen.xy);
    var col = textureLoad(t_scene, px, 0).rgb;
    if (G.post.x < 0.5) {
        // a lens's lateral fringing, only towards the edges of the frame
        let e = (uv - 0.5) * dot(uv - 0.5, uv - 0.5) * 0.012;
        let pr = vec2<i32>(clamp(uv + e, vec2<f32>(0.0), vec2<f32>(0.999)) * G.screen.xy);
        let pb = vec2<i32>(clamp(uv - e, vec2<f32>(0.0), vec2<f32>(0.999)) * G.screen.xy);
        col = vec3<f32>(textureLoad(t_scene, pr, 0).r, col.g, textureLoad(t_scene, pb, 0).b);
    }
    if (strain > 0.01) {
        let off = (uv - 0.5) * 0.012 * strain * (1.0 + 0.5 * sin(G.cam_pos.w * 7.0));
        let pr = vec2<i32>(clamp(uv + off, vec2<f32>(0.0), vec2<f32>(0.999)) * G.screen.xy);
        let pb = vec2<i32>(clamp(uv - off, vec2<f32>(0.0), vec2<f32>(0.999)) * G.screen.xy);
        col = vec3<f32>(textureLoad(t_scene, pr, 0).r, col.g, textureLoad(t_scene, pb, 0).b);
    }
    let depth = textureLoad(t_depth, px, 0);

    // reconstruct world position
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0);
    let wp4 = G.inv_view_proj * ndc;
    let wp = wp4.xyz / wp4.w;
    let ro = G.cam_pos.xyz;
    var rd = wp - ro;
    var dist = length(rd);
    rd = rd / max(dist, 1e-4);
    let sky = depth >= 0.99999;
    if (sky) {
        let up = clamp(rd.y, -0.2, 1.0);
        let sd = max(dot(rd, G.sun_dir.xyz), 0.0);
        col = G.fog_col.rgb * (0.55 + 0.6 * (1.0 - up)) + G.sun_col.rgb * (pow(sd, 400.0) * 6.0 + pow(sd, 8.0) * 0.15);
        dist = G.fog_par.z;
    }
    if (!sky) {
        col = col * ssao(px, dist, ro, in.pos.xy);
    }
    dist = min(dist, G.fog_par.z);

    // volumetric fog: marched sun shafts
    let steps = i32(G.fog_par.y);
    let dens0 = G.fog_col.w;
    var trans = 1.0;
    var scat = vec3<f32>(0.0);
    let dth = bayer(in.pos.xy);
    let phase = 0.25 + 0.75 * pow(max(dot(rd, G.sun_dir.xyz), 0.0), 3.0);
    let dt = dist / f32(steps);
    for (var i = 0; i < steps; i = i + 1) {
        let p = ro + rd * (f32(i) + dth) * dt;
        let dens = dens0 * exp(-max(p.y, 0.0) * G.fog_par.x);
        var vis = 1.0;
        if (G.cam_right.w > 0.5) { vis = shadow_vis(p); }
        let a = 1.0 - exp(-dens * dt);
        scat = scat + trans * a * (G.fog_col.rgb + G.sun_col.rgb * vis * phase * G.sun_dir.w);
        trans = trans * (1.0 - a);
    }
    // analytic in-scattering from point lights
    let nl = i32(G.counts.x);
    var pls = vec3<f32>(0.0);
    for (var i = 0; i < nl; i = i + 1) {
        let q = G.light_pos[i].xyz - ro;
        let b = dot(q, rd);
        let h = sqrt(max(dot(q, q) - b * b, 0.09));
        let r = G.light_pos[i].w;
        if (h < r) {
            let fall = 1.0 - h / r;
            pls = pls + G.light_col[i].rgb * ((atan((dist - b) / h) - atan(-b / h)) / h) * fall * fall;
        }
    }
    col = col * trans + scat + pls * dens0 * G.fog_par.w;

    // the CRT filter glows more: halation in the tube's glass
    col = col + textureSampleLevel(t_bloom, s_lin, uv, 0.0).rgb * select(0.9, 1.6, G.post.x > 0.5 && G.post.x < 1.5);

    // tonemap + grade
    col = col * G.grade.rgb;
    col = aces(col * 1.05);
    let l = dot(col, vec3<f32>(0.3, 0.55, 0.15));
    col = mix(vec3<f32>(l), col, G.grade.w);
    let vg = length((in.uv - 0.5) * vec2<f32>(1.1, 1.0));
    col = col * (1.0 - G.fx.z * smoothstep(0.35, 0.9, vg));
    col = mix(col, col * vec3<f32>(1.25, 0.7, 1.3), strain * smoothstep(0.2, 0.75, vg) * (0.7 + 0.3 * sin(G.cam_pos.w * 3.0)));
    col = pow(max(col, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
    if (G.post.x < 0.5) {
        // no retro filter: a fine film grain, stronger in the shadows
        let lum = dot(col, vec3<f32>(0.3, 0.55, 0.15));
        col = col + (hash21(in.pos.xy + vec2<f32>(G.cam_pos.w * 37.0, G.cam_pos.w * 91.0)) - 0.5) * 0.045 * (1.0 - lum * 0.7);
    }
    col = mix(col, vec3<f32>(1.0), G.counts.w);
    col = col * (1.0 - G.counts.z);
    // palette quantisation with ordered dithering
    let levels = G.fx.x;
    col = floor(col * levels + (bayer(in.pos.xy) - 0.5) * G.fx.y + 0.5) / levels;
    return vec4<f32>(col, 1.0);
}

