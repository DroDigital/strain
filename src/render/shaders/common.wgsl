struct Globals {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    shadow_vp: mat4x4<f32>,
    cam_pos: vec4<f32>,   // xyz, time
    cam_right: vec4<f32>, // xyz, shadows enabled
    cam_up: vec4<f32>,
    sun_dir: vec4<f32>,   // xyz toward the light, w = shaft strength
    sun_col: vec4<f32>,   // rgb, w = wetness
    ambient: vec4<f32>,   // rgb, w = world warp amount
    fog_col: vec4<f32>,   // rgb, w = density
    fog_par: vec4<f32>,   // ground-layer height falloff, march steps, max distance, point light scatter
    screen: vec4<f32>,    // internal w, h, frame counter, exposure
    grade: vec4<f32>,     // rgb tint, saturation
    fx: vec4<f32>,        // film grain, unused, vignette, strain
    counts: vec4<f32>,    // lights, distortions, fade to black, white flash
    light_pos: array<vec4<f32>, 32>, // xyz, radius
    light_col: array<vec4<f32>, 32>,
    distort: array<vec4<f32>, 8>,    // xyz, radius
    distort2: array<vec4<f32>, 8>,   // strength, swirl
    key_vp: array<mat4x4<f32>, 6>,   // shadow-casting point light faces
    key: vec4<f32>,                  // xyz, enabled
    fog2: vec4<f32>,                 // elevated layer: start height, full height, density multiplier, noise amount
    wind: vec4<f32>,                 // xz drift of the fog noise (m/s), y rise, w = ground mist
    prev_vp: mat4x4<f32>,            // last frame's view-projection, for motion blur
    lens: vec4<f32>,                 // focus distance, depth-of-field strength, motion blur, unused
};
@group(0) @binding(0) var<uniform> G: Globals;

fn hash21(p: vec2<f32>) -> f32 {
    var q = fract(p * vec2<f32>(123.34, 456.21));
    q = q + dot(q, q + 45.32);
    return fract(q.x * q.y);
}
fn vnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash21(i), hash21(i + vec2<f32>(1.0, 0.0)), u.x),
               mix(hash21(i + vec2<f32>(0.0, 1.0)), hash21(i + vec2<f32>(1.0, 1.0)), u.x), u.y);
}

fn hash31(p: vec3<f32>) -> f32 {
    var q = fract(p * 0.1031);
    q = q + dot(q, q.zyx + 31.32);
    return fract((q.x + q.y) * q.z);
}
fn vnoise3(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(mix(hash31(i), hash31(i + vec3<f32>(1.0, 0.0, 0.0)), u.x), mix(hash31(i + vec3<f32>(0.0, 1.0, 0.0)), hash31(i + vec3<f32>(1.0, 1.0, 0.0)), u.x), u.y);
    let b = mix(mix(hash31(i + vec3<f32>(0.0, 0.0, 1.0)), hash31(i + vec3<f32>(1.0, 0.0, 1.0)), u.x), mix(hash31(i + vec3<f32>(0.0, 1.0, 1.0)), hash31(i + vec3<f32>(1.0, 1.0, 1.0)), u.x), u.y);
    return mix(a, b, u.z);
}
/// Interleaved gradient noise: a stable per-pixel jitter with blue-ish spectrum.
fn ign(p: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}
