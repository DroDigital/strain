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
@fragment
fn fs_blit(in: FsIn) -> @location(0) vec4<f32> {
    let c = textureSampleLevel(t_src, s_src, in.uv, 0.0).rgb;
    return vec4<f32>(mix(c, pow(c, vec3<f32>(2.2)), G.cam_up.w), 1.0);
}
