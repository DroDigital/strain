@group(0) @binding(0) var t_ui: texture_2d<f32>;
@group(0) @binding(1) var s_ui: sampler;
struct UOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32>, @location(1) color: vec4<f32> };
@vertex
fn vs_main(@location(0) pos: vec2<f32>, @location(1) uv: vec2<f32>, @location(2) color: vec4<f32>) -> UOut {
    var out: UOut;
    out.pos = vec4<f32>(pos, 0.0, 1.0);
    out.uv = uv;
    out.color = color;
    return out;
}
@fragment
fn fs_main(in: UOut) -> @location(0) vec4<f32> {
    let t = textureSampleLevel(t_ui, s_ui, in.uv, 0.0);
    return vec4<f32>(in.color.rgb, in.color.a * t.a);
}
