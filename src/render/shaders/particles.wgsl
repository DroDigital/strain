struct PIn {
    @builtin(vertex_index) vi: u32,
    @location(0) pos_size: vec4<f32>,
    @location(1) color: vec4<f32>,
    @location(2) par: vec4<f32>, // kind, rotation, stretch, additive
    @location(3) vel: vec4<f32>,
};
struct POut {
    @builtin(position) clip: vec4<f32>,
    @location(0) c: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) par: vec4<f32>,
};
@vertex
fn vs_main(in: PIn) -> POut {
    var out: POut;
    let c = vec2<f32>(f32(in.vi & 1u) * 2.0 - 1.0, f32((in.vi >> 1u) & 1u) * 2.0 - 1.0);
    var w = in.pos_size.xyz;
    let s = in.pos_size.w;
    if (in.par.z > 0.0) {
        let axis = normalize(in.vel.xyz + vec3<f32>(0.0, 1e-4, 0.0));
        let side = normalize(cross(axis, G.cam_pos.xyz - w) + vec3<f32>(1e-4, 0.0, 0.0));
        w = w + axis * c.y * s * in.par.z + side * c.x * s;
    } else {
        let cr = cos(in.par.y);
        let sr = sin(in.par.y);
        let rc = vec2<f32>(c.x * cr - c.y * sr, c.x * sr + c.y * cr);
        w = w + (G.cam_right.xyz * rc.x + G.cam_up.xyz * rc.y) * s;
    }
    out.clip = G.view_proj * vec4<f32>(w, 1.0);
    out.c = c;
    out.color = in.color;
    out.par = in.par;
    return out;
}
fn pfbm(p: vec2<f32>) -> f32 {
    return vnoise(p) * 0.55 + vnoise(p * 2.1 + 7.3) * 0.3 + vnoise(p * 4.3 + 1.7) * 0.15;
}

@fragment
fn fs_main(in: POut) -> @location(0) vec4<f32> {
    let d = length(in.c);
    var a = 1.0;
    var rgb = in.color.rgb;
    let kind = i32(in.par.x + 0.5);
    let seed = in.par.y * 13.7;
    if (kind == 0) {
        // soft glow with a hot centre
        a = pow(smoothstep(1.0, 0.0, d), 1.6);
        rgb = rgb * (1.0 + 0.8 * smoothstep(0.5, 0.0, d));
    } else if (kind == 1) {
        // droplets and chips: round with a bright edge highlight
        a = smoothstep(1.0, 0.75, d);
        rgb = rgb * (0.8 + 0.4 * smoothstep(0.2, -0.6, in.c.x + in.c.y));
    } else if (kind == 2) {
        // billowing puff: fbm-eroded disc, lit from above, darker underneath
        let n = pfbm(in.c * 1.6 + vec2<f32>(seed, seed * 0.7));
        a = smoothstep(1.0, 0.25, d + (0.45 - n) * 0.9);
        rgb = rgb * (0.65 + 0.55 * clamp(0.5 - in.c.y * 0.5, 0.0, 1.0)) * (0.8 + 0.4 * n);
        // fire puffs (additive) get a white-hot core
        rgb = mix(rgb, rgb * 1.8 + vec3<f32>(0.4, 0.3, 0.1) * in.par.w, smoothstep(0.45, 0.0, d) * in.par.w);
    } else {
        // streaks: bright core fading along the length
        a = (1.0 - abs(in.c.x)) * (1.0 - abs(in.c.y) * abs(in.c.y));
        a = a * a;
    }
    a = a * in.color.a;
    return vec4<f32>(rgb * a, a * (1.0 - in.par.w));
}
