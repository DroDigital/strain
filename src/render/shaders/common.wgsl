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
    fog_par: vec4<f32>,   // height falloff, march steps, max distance, point light scatter
    screen: vec4<f32>,    // internal w, h, vertex snap, affine amount
    grade: vec4<f32>,     // rgb tint, saturation
    fx: vec4<f32>,        // palette levels, dither amount, vignette, strain
    counts: vec4<f32>,    // lights, distortions, fade to black, white flash
    light_pos: array<vec4<f32>, 32>, // xyz, radius
    light_col: array<vec4<f32>, 32>,
    distort: array<vec4<f32>, 8>,    // xyz, radius
    distort2: array<vec4<f32>, 8>,   // strength, swirl
    key_vp: array<mat4x4<f32>, 6>,   // shadow-casting point light faces
    key: vec4<f32>,                  // xyz, enabled
    post: vec4<f32>,                 // retro filter, window w, window h, unused
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
