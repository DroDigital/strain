@group(1) @binding(0) var t_scene: texture_2d<f32>;
@group(1) @binding(1) var t_depth: texture_depth_2d;
@group(1) @binding(2) var t_bloom: texture_2d<f32>;
@group(1) @binding(3) var s_lin: sampler;
@group(1) @binding(4) var t_shadow: texture_depth_2d;
@group(1) @binding(5) var s_shadow: sampler_comparison;

fn shadow_vis(p: vec3<f32>) -> f32 {
    let sp = G.shadow_vp * vec4<f32>(p, 1.0);
    let suv = sp.xy * vec2<f32>(0.5, -0.5) + 0.5;
    if (suv.x <= 0.0 || suv.x >= 1.0 || suv.y <= 0.0 || suv.y >= 1.0 || sp.z >= 1.0) { return 1.0; }
    return textureSampleCompareLevel(t_shadow, s_shadow, suv, sp.z - 0.002);
}

fn world_at(px: vec2<i32>) -> vec3<f32> {
    let q = clamp(px, vec2<i32>(0), vec2<i32>(G.screen.xy) - 1);
    let d = textureLoad(t_depth, q, 0);
    let uv = (vec2<f32>(q) + 0.5) / G.screen.xy;
    let w = G.inv_view_proj * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
    return w.xyz / w.w;
}

/// Participating-media density at a world position: a base layer, a mist that hugs the street,
/// and a heavier smog layer that thickens with altitude (rooftops sit inside it). Drifting 3D
/// noise breaks it into banks and wisps.
fn fog_density(p: vec3<f32>, t: f32) -> f32 {
    let y = p.y;
    let ground = G.wind.w * exp(-max(y, 0.0) * G.fog_par.x);
    let elevated = G.fog2.z * smoothstep(G.fog2.x, G.fog2.y, y);
    let drift = vec3<f32>(G.wind.x, -G.wind.y, G.wind.z) * t;
    let q = (p - drift) * 0.055;
    let n = vnoise3(q) * 0.62 + vnoise3(q * 2.9 + vec3<f32>(3.1, 7.7, 1.3)) * 0.38;
    let banks = mix(1.0, 0.2 + 1.6 * n * n * 1.6, G.fog2.w);
    return G.fog_col.w * (1.0 + ground + elevated) * banks;
}

/// Henyey-Greenstein phase function, normalised so isotropic = 1.
fn hg(c: f32, g: f32) -> f32 {
    let g2 = g * g;
    return (1.0 - g2) / pow(max(1.0 + g2 - 2.0 * g * c, 1e-4), 1.5);
}

fn bayer(p: vec2<f32>) -> f32 {
    let x = u32(p.x) & 3u;
    let y = u32(p.y) & 3u;
    var m = array<f32, 16>(0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0, 3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0);
    return (m[y * 4u + x] + 0.5) / 16.0;
}

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
    let jit = ign(in.pos.xy);
    let res = G.screen.xy;

    // camera motion blur: reproject this pixel into last frame and smear along the difference
    {
        let ndc0 = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0);
        let w4 = G.inv_view_proj * ndc0;
        let pc = G.prev_vp * vec4<f32>(w4.xyz / w4.w, 1.0);
        if (pc.w > 0.01 && G.lens.z > 0.0) {
            let puv = pc.xy / pc.w * vec2<f32>(0.5, -0.5) + 0.5;
            var mv = (uv - puv) * G.lens.z;
            let ml = length(mv * res);
            if (ml > 40.0) { mv = mv * (40.0 / ml); }
            if (length(mv * res) > 1.0) {
                var acc = col;
                for (var i = 1; i < 6; i = i + 1) {
                    let o = (f32(i) + jit - 0.5) / 6.0 - 0.5;
                    let q = vec2<i32>(clamp(uv + mv * o, vec2<f32>(0.0), vec2<f32>(0.999)) * res);
                    acc = acc + textureLoad(t_scene, q, 0).rgb;
                }
                col = acc / 6.0;
            }
        }
    }

    // depth of field: a soft disc gather behind the focus plane (the foreground stays crisp so
    // the player never smears over the background)
    {
        let ndc1 = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0);
        let w5 = G.inv_view_proj * ndc1;
        let d = length(w5.xyz / w5.w - G.cam_pos.xyz);
        let coc = clamp((d - G.lens.x) / max(d, 0.1) * G.lens.y, 0.0, 1.0) * res.y * 0.006;
        if (coc > 0.6) {
            var acc = col;
            var wsum = 1.0;
            let a0 = jit * 6.2832;
            for (var i = 0; i < 12; i = i + 1) {
                let fi = f32(i);
                let ang = a0 + fi * 2.39996;
                let r = coc * sqrt((fi + 0.5) / 12.0);
                let q = vec2<i32>(clamp(uv * res + vec2<f32>(cos(ang), sin(ang)) * r, vec2<f32>(0.0), res - 1.0));
                // do not pull in sharp foreground pixels
                let qd = textureLoad(t_depth, q, 0);
                let ok = select(0.0, 1.0, qd >= depth - 0.0005);
                acc = acc + textureLoad(t_scene, q, 0).rgb * ok;
                wsum = wsum + ok;
            }
            col = acc / wsum;
        }
    }

    // reconstruct world position
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0);
    let wp4 = G.inv_view_proj * ndc;
    let wp = wp4.xyz / wp4.w;
    let ro = G.cam_pos.xyz;
    var rd = wp - ro;
    var dist = length(rd);
    rd = rd / max(dist, 1e-4);
    let sky = depth >= 0.99999;

    if (!sky) {
        // screen-space ambient occlusion: contact shadows in creases, under cars, at wall feet
        let pr = world_at(px + vec2<i32>(1, 0)) - wp;
        let pu = world_at(px + vec2<i32>(0, -1)) - wp;
        let n = normalize(cross(pr, pu));
        let radius = 0.75;
        let rpx = clamp(radius * G.screen.y * 0.71 / max(dist, 0.1), 3.0, 90.0);
        var occ = 0.0;
        let a0 = 0.7;
        for (var i = 0; i < 10; i = i + 1) {
            let fi = f32(i);
            let ang = a0 + fi * 2.39996;
            let r = rpx * sqrt((fi + 0.5) / 10.0);
            let sp = world_at(px + vec2<i32>(vec2<f32>(cos(ang), sin(ang)) * r));
            let v = sp - wp;
            let vv = dot(v, v);
            occ = occ + max(dot(v, n) - 0.002 - 0.0005 * dist, 0.0) / (vv + 0.04) * smoothstep(radius * 2.5, radius * 0.5, sqrt(vv));
        }
        let ao = clamp(1.0 - occ * 0.11, 0.0, 1.0);
        col = col * mix(1.0, ao * ao, 0.9);
    }

    let sd = dot(rd, G.sun_dir.xyz);
    if (sky) {
        let up = clamp(rd.y, -0.2, 1.0);
        col = G.fog_col.rgb * (0.7 + 0.5 * (1.0 - up)) + G.sun_col.rgb * (pow(max(sd, 0.0), 600.0) * 8.0 + pow(max(sd, 0.0), 6.0) * 0.2);
        dist = G.fog_par.z;
    }
    dist = min(dist, G.fog_par.z);

    // volumetric fog: ray-marched density with sun shafts through the shadow map. Samples are
    // spaced quadratically (dense near the camera) and jittered per pixel.
    let steps = i32(G.fog_par.y);
    let fsteps = f32(steps);
    var trans = 1.0;
    var scat = vec3<f32>(0.0);
    let phase = mix(1.0, hg(sd, 0.6), 0.65) * G.sun_dir.w;
    let tnow = G.cam_pos.w;
    var prev = 0.0;
    for (var i = 0; i < steps; i = i + 1) {
        let s1 = (f32(i) + jit) / fsteps;
        let tt = s1 * s1 * dist;
        let dt = max(tt - prev, 0.0) + dist / (fsteps * fsteps);
        prev = tt;
        let p = ro + rd * tt;
        let dens = fog_density(p, tnow);
        var vis = 1.0;
        if (G.cam_right.w > 0.5) { vis = shadow_vis(p); }
        let a = 1.0 - exp(-dens * dt);
        // ambient in-scatter darkens a little inside the dense upper layer
        let amb = G.fog_col.rgb * (0.8 - 0.3 * smoothstep(G.fog2.x, G.fog2.y, p.y));
        scat = scat + trans * a * (amb + G.sun_col.rgb * vis * phase * 0.07);
        trans = trans * (1.0 - a);
        if (trans < 0.004) { break; }
    }
    // analytic in-scattering from point lights (halos around lamps and fires)
    let nl = i32(G.counts.x);
    var pls = vec3<f32>(0.0);
    for (var i = 0; i < nl; i = i + 1) {
        let q = G.light_pos[i].xyz - ro;
        let b = dot(q, rd);
        let h = sqrt(max(dot(q, q) - b * b, 0.09));
        let r = G.light_pos[i].w;
        // a lamp right at the lens would wash the frame out; only lights out in the fog glow
        if (h < r) {
            let fall = (1.0 - h / r) * smoothstep(1.5, 5.0, length(q));
            pls = pls + G.light_col[i].rgb * ((atan((dist - b) / h) - atan(-b / h)) / h) * fall * fall;
        }
    }
    let dens_here = fog_density(ro + rd * min(dist, 6.0), tnow);
    col = col * trans + scat + pls * min(dens_here, 0.06) * G.fog_par.w * 0.4;

    // the CRT filter glows more: halation in the tube's glass
    col = col + textureSampleLevel(t_bloom, s_lin, uv, 0.0).rgb * select(0.45, 1.6, G.post.x > 0.5 && G.post.x < 1.5);

    // exposure, filmic tonemap, grade
    col = col * G.grade.rgb * G.screen.w;
    col = aces(col);
    let l = dot(col, vec3<f32>(0.2126, 0.7152, 0.0722));
    col = mix(vec3<f32>(l), col, G.grade.w);
    let vg = length((in.uv - 0.5) * vec2<f32>(1.1, 1.0));
    col = col * (1.0 - G.fx.z * smoothstep(0.4, 1.0, vg));
    col = mix(col, col * vec3<f32>(1.25, 0.7, 1.3), strain * smoothstep(0.2, 0.75, vg) * (0.7 + 0.3 * sin(G.cam_pos.w * 3.0)));
    col = pow(max(col, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
    if (G.post.x < 0.5) {
        // film grain, strongest in the mid-tones (the retro filters bring their own noise)
        let grain = hash21(in.pos.xy + vec2<f32>(fract(G.cam_pos.w * 7.13) * 431.0, fract(G.cam_pos.w * 3.71) * 977.0)) - 0.5;
        col = col + grain * G.fx.x * (0.3 + 0.7 * (1.0 - abs(l * 2.0 - 1.0)));
    }
    col = mix(col, vec3<f32>(1.0), G.counts.w);
    col = col * (1.0 - G.counts.z);
    if (G.post.x > 2.5) {
        // 32-BIT filter: 15-bit colour with ordered dithering, like the consoles it imitates
        col = floor(col * 31.0 + (bayer(in.pos.xy) - 0.5) * 1.4 + 0.5) / 31.0;
    } else {
        // half-LSB dither against banding in the fog gradients
        col = col + (ign(in.pos.xy) - 0.5) / 255.0;
    }
    return vec4<f32>(col, 1.0);
}
