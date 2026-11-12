//! Hand-written wgpu renderer: retro scene pass (vertex snapping, affine wobble, nearest textures),
//! sun shadow map, GPU-instanced meshes and particles, volumetric fog + bloom + distortion +
//! ordered-dither post, fixed internal resolution upscaled to the window, UI overlay.
pub mod mesh;
pub mod textures;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use mesh::{Instance, MeshBuilder, MeshRange, Vertex};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use textures::{Tex, TEX};
use wgpu::util::DeviceExt;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum WindowMode {
    Windowed,
    Borderless,
    Fullscreen,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct GfxSettings {
    pub internal_height: u32,
    pub shadow_quality: u8,
    pub fog_quality: u8,
    pub particle_density: u8,
    pub vsync: bool,
    pub window_mode: WindowMode,
}
impl Default for GfxSettings {
    fn default() -> Self {
        GfxSettings { internal_height: 480, shadow_quality: 2, fog_quality: 1, particle_density: 1, vsync: true, window_mode: WindowMode::Windowed }
    }
}
impl GfxSettings {
    pub fn particle_scale(&self) -> f32 {
        [0.4, 1.0, 1.6][self.particle_density.min(2) as usize]
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [f32; 16],
    inv_view_proj: [f32; 16],
    shadow_vp: [f32; 16],
    cam_pos: [f32; 4],
    cam_right: [f32; 4],
    cam_up: [f32; 4],
    sun_dir: [f32; 4],
    sun_col: [f32; 4],
    ambient: [f32; 4],
    fog_col: [f32; 4],
    fog_par: [f32; 4],
    screen: [f32; 4],
    grade: [f32; 4],
    fx: [f32; 4],
    counts: [f32; 4],
    light_pos: [[f32; 4]; 32],
    light_col: [[f32; 4]; 32],
    distort: [[f32; 4]; 8],
    distort2: [[f32; 4]; 8],
    key_vp: [[f32; 16]; 6],
    key: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct ParticleInst {
    pub pos_size: [f32; 4],
    pub color: [f32; 4],
    /// kind, rotation, stretch, additive
    pub par: [f32; 4],
    pub vel: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct UiVertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub color: [u8; 4],
}

#[derive(Clone, Copy, Debug)]
pub struct Light {
    pub pos: Vec3,
    pub radius: f32,
    pub color: Vec3,
}
#[derive(Clone, Copy, Debug)]
pub struct Distort {
    pub pos: Vec3,
    pub radius: f32,
    pub strength: f32,
    pub swirl: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Env {
    pub sun_dir: Vec3,
    pub sun_col: Vec3,
    pub ambient: Vec3,
    pub fog_col: Vec3,
    pub fog_density: f32,
    pub fog_height: f32,
    pub shaft: f32,
    pub light_scatter: f32,
    pub wetness: f32,
    pub warp: f32,
    pub grade: Vec3,
    pub saturation: f32,
    pub levels: f32,
    pub dither: f32,
    pub vignette: f32,
    pub strain: f32,
    pub fade: f32,
    pub flash: f32,
}
impl Default for Env {
    fn default() -> Self {
        Env {
            sun_dir: Vec3::new(0.3, 0.8, 0.2).normalize(),
            sun_col: Vec3::splat(1.0),
            ambient: Vec3::splat(0.2),
            fog_col: Vec3::new(0.1, 0.12, 0.15),
            fog_density: 0.02,
            fog_height: 0.05,
            shaft: 1.0,
            light_scatter: 1.0,
            wetness: 0.5,
            warp: 0.0,
            grade: Vec3::ONE,
            saturation: 1.0,
            levels: 24.0,
            dither: 1.0,
            vignette: 0.5,
            strain: 0.0,
            fade: 0.0,
            flash: 0.0,
        }
    }
}

pub struct StaticMesh {
    vb: wgpu::Buffer,
    ib: wgpu::Buffer,
    count: u32,
    pub min: Vec3,
    pub max: Vec3,
}

/// Instances bucketed by mesh id.
#[derive(Default)]
pub struct InstList {
    pub buckets: Vec<Vec<Instance>>,
}
impl InstList {
    pub fn clear(&mut self) {
        for b in &mut self.buckets {
            b.clear();
        }
    }
    #[inline]
    pub fn push(&mut self, mesh: u32, inst: Instance) {
        let m = mesh as usize;
        if self.buckets.len() <= m {
            self.buckets.resize_with(m + 1, Vec::new);
        }
        self.buckets[m].push(inst);
    }
}

pub struct Frame<'a> {
    pub view: Mat4,
    pub proj: Mat4,
    pub cam_pos: Vec3,
    pub time: f32,
    pub env: Env,
    pub shadow_center: Vec3,
    pub statics: Vec<&'a StaticMesh>,
    pub shadow_statics: Vec<&'a StaticMesh>,
    pub inst: &'a InstList,
    pub lights: &'a [Light],
    pub distorts: &'a [Distort],
    pub particles: &'a [ParticleInst],
    pub ui: &'a [UiVertex],
    /// shadow-casting point light (position, radius); must be lights[0]
    pub key: Option<(Vec3, f32)>,
    pub key_statics: Vec<&'a StaticMesh>,
}

struct Targets {
    w: u32,
    h: u32,
    scene_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    ldr: wgpu::Texture,
    ldr_view: wgpu::TextureView,
    bloom_views: [wgpu::TextureView; 3],
    post_bg: wgpu::BindGroup,
    bloom_bgs: [wgpu::BindGroup; 3],
    blit_bg: wgpu::BindGroup,
}

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    pub device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pub settings: GfxSettings,
    globals_buf: wgpu::Buffer,
    g0: wgpu::BindGroup,
    g1_scene: wgpu::BindGroup,
    l_scene: wgpu::BindGroupLayout,
    l_post: wgpu::BindGroupLayout,
    l_src: wgpu::BindGroupLayout,
    tex_array: wgpu::Texture,
    tex_view: wgpu::TextureView,
    samp_nearest_rep: wgpu::Sampler,
    samp_nearest: wgpu::Sampler,
    samp_linear: wgpu::Sampler,
    samp_shadow: wgpu::Sampler,
    shadow_view: wgpu::TextureView,
    shadow_size: u32,
    key_view: wgpu::TextureView,
    key_face_buf: wgpu::Buffer,
    key_face_bg: wgpu::BindGroup,
    p_key: wgpu::RenderPipeline,
    targets: Targets,
    p_scene: wgpu::RenderPipeline,
    p_shadow: wgpu::RenderPipeline,
    p_particles: wgpu::RenderPipeline,
    p_post: wgpu::RenderPipeline,
    p_bright: wgpu::RenderPipeline,
    p_blur_h: wgpu::RenderPipeline,
    p_blur_v: wgpu::RenderPipeline,
    p_blit: wgpu::RenderPipeline,
    p_ui: wgpu::RenderPipeline,
    ui_bg: wgpu::BindGroup,
    prim_vb: wgpu::Buffer,
    prim_ib: wgpu::Buffer,
    prim_ranges: Vec<MeshRange>,
    inst_buf: wgpu::Buffer,
    inst_cap: u64,
    part_buf: wgpu::Buffer,
    part_cap: u64,
    ui_buf: wgpu::Buffer,
    ui_cap: u64,
    inst_scratch: Vec<Instance>,
    srgb_surface: bool,
    pub capture: Option<std::path::PathBuf>,
    pub adapter_info: String,
}

const HDR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const LDR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const KEY_FACE: u32 = 512;

fn tex_entry(binding: u32, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::FRAGMENT, ty, count: None }
}
fn bt_tex(dim: wgpu::TextureViewDimension, depth: bool) -> wgpu::BindingType {
    wgpu::BindingType::Texture {
        sample_type: if depth { wgpu::TextureSampleType::Depth } else { wgpu::TextureSampleType::Float { filterable: true } },
        view_dimension: dim,
        multisampled: false,
    }
}
fn bt_samp(cmp: bool) -> wgpu::BindingType {
    wgpu::BindingType::Sampler(if cmp { wgpu::SamplerBindingType::Comparison } else { wgpu::SamplerBindingType::Filtering })
}

fn make_tex(device: &wgpu::Device, w: u32, h: u32, format: wgpu::TextureFormat, label: &str) -> (wgpu::Texture, wgpu::TextureView) {
    let t = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let v = t.create_view(&wgpu::TextureViewDescriptor::default());
    (t, v)
}

impl Renderer {
    pub fn new(window: Arc<winit::window::Window>, display: winit::event_loop::OwnedDisplayHandle, settings: GfxSettings, prims: (MeshBuilder, Vec<MeshRange>)) -> Renderer {
        let mut idesc = wgpu::InstanceDescriptor::new_with_display_handle(Box::new(display));
        idesc.backends = wgpu::Backends::from_env().unwrap_or(wgpu::Backends::all());
        let instance = wgpu::Instance::new(idesc);
        let surface = instance.create_surface(window.clone()).expect("create surface");
        let adapter = crate::util::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            ..Default::default()
        }))
        .or_else(|_| {
            crate::util::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                force_fallback_adapter: true,
                ..Default::default()
            }))
        })
        .expect("no compatible graphics adapter found");
        let info = adapter.get_info();
        let adapter_info = format!("{} ({:?})", info.name, info.backend);
        let limits = wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits());
        let (device, queue) = crate::util::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("strain"),
            required_features: wgpu::Features::empty(),
            required_limits: limits,
            ..Default::default()
        }))
        .expect("request device");
        let size = window.inner_size();
        let mut config = surface.get_default_config(&adapter, size.width.max(1), size.height.max(1)).expect("surface config");
        let caps = surface.get_capabilities(&adapter);
        if let Some(f) = caps.formats.iter().find(|f| !f.is_srgb()) {
            config.format = *f;
        }
        let srgb_surface = config.format.is_srgb();
        config.present_mode = if settings.vsync { wgpu::PresentMode::AutoVsync } else { wgpu::PresentMode::AutoNoVsync };
        surface.configure(&device, &config);

        // ---- shaders
        let common = include_str!("shaders/common.wgsl");
        let full = include_str!("shaders/fullscreen.wgsl");
        let sm = |label: &str, src: String| device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(label), source: wgpu::ShaderSource::Wgsl(src.into()) });
        let sh_scene = sm("scene", format!("{common}\n{}", include_str!("shaders/scene.wgsl")));
        let sh_part = sm("particles", format!("{common}\n{}", include_str!("shaders/particles.wgsl")));
        let sh_post = sm("post", format!("{common}\n{full}\n{}", include_str!("shaders/post.wgsl")));
        let sh_bloom = sm("bloom", format!("{common}\n{full}\n{}", include_str!("shaders/bloom.wgsl")));
        let sh_ui = sm("ui", include_str!("shaders/ui.wgsl").to_string());

        // ---- layouts
        let l_g0 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("g0"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        use wgpu::TextureViewDimension as D;
        let l_scene = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene"),
            entries: &[tex_entry(0, bt_tex(D::D2Array, false)), tex_entry(1, bt_samp(false)), tex_entry(2, bt_tex(D::D2, true)), tex_entry(3, bt_samp(true)), tex_entry(4, bt_tex(D::D2, true))],
        });
        let l_post = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post"),
            entries: &[
                tex_entry(0, bt_tex(D::D2, false)),
                tex_entry(1, bt_tex(D::D2, true)),
                tex_entry(2, bt_tex(D::D2, false)),
                tex_entry(3, bt_samp(false)),
                tex_entry(4, bt_tex(D::D2, true)),
                tex_entry(5, bt_samp(true)),
            ],
        });
        let l_src = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("src"), entries: &[tex_entry(0, bt_tex(D::D2, false)), tex_entry(1, bt_samp(false))] });
        let pl = |a: &[Option<&wgpu::BindGroupLayout>]| device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: a, immediate_size: 0 });
        let pl_scene = pl(&[Some(&l_g0), Some(&l_scene)]);
        let pl_g0 = pl(&[Some(&l_g0)]);
        let pl_post = pl(&[Some(&l_g0), Some(&l_post)]);
        let pl_src = pl(&[Some(&l_g0), Some(&l_src)]);
        let pl_ui = pl(&[Some(&l_src)]);

        // ---- pipelines
        let vattrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3, 3 => Unorm8x4];
        let iattrs = wgpu::vertex_attr_array![4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4, 8 => Float32x4];
        let vbl = wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Vertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &vattrs };
        let ibl = wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Instance>() as u64, step_mode: wgpu::VertexStepMode::Instance, attributes: &iattrs };
        let depth_state = |write: bool, bias: i32| {
            Some(wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: Some(write),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: wgpu::DepthBiasState { constant: bias, slope_scale: if bias != 0 { 2.0 } else { 0.0 }, clamp: 0.0 },
            })
        };
        let prim_state = |cull: bool, strip: bool| wgpu::PrimitiveState {
            topology: if strip { wgpu::PrimitiveTopology::TriangleStrip } else { wgpu::PrimitiveTopology::TriangleList },
            cull_mode: if cull { Some(wgpu::Face::Back) } else { None },
            ..Default::default()
        };
        let p_scene = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("scene"),
            layout: Some(&pl_scene),
            vertex: wgpu::VertexState { module: &sh_scene, entry_point: Some("vs_main"), compilation_options: Default::default(), buffers: &[Some(vbl.clone()), Some(ibl.clone())] },
            primitive: prim_state(true, false),
            depth_stencil: depth_state(true, 0),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &sh_scene, entry_point: Some("fs_main"), compilation_options: Default::default(), targets: &[Some(HDR.into())] }),
            multiview_mask: None,
            cache: None,
        });
        let p_shadow = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow"),
            layout: Some(&pl_g0),
            vertex: wgpu::VertexState { module: &sh_scene, entry_point: Some("vs_shadow"), compilation_options: Default::default(), buffers: &[Some(vbl), Some(ibl)] },
            primitive: prim_state(false, false),
            depth_stencil: depth_state(true, 2),
            multisample: Default::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });
        let l_kf = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("key face"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: wgpu::BufferSize::new(64) },
                count: None,
            }],
        });
        let pl_key = pl(&[Some(&l_g0), None, Some(&l_kf)]);
        let vbl2 = wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Vertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &vattrs };
        let ibl2 = wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Instance>() as u64, step_mode: wgpu::VertexStepMode::Instance, attributes: &iattrs };
        let p_key = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("key shadow"),
            layout: Some(&pl_key),
            vertex: wgpu::VertexState { module: &sh_scene, entry_point: Some("vs_key"), compilation_options: Default::default(), buffers: &[Some(vbl2), Some(ibl2)] },
            primitive: prim_state(false, false),
            depth_stencil: depth_state(true, 2),
            multisample: Default::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });
        let key_face_buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("key faces"), size: 256 * 6, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let key_face_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &l_kf,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &key_face_buf, offset: 0, size: wgpu::BufferSize::new(64) }) }],
        });
        let pattrs = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4];
        let premul = wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
        };
        let p_particles = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("particles"),
            layout: Some(&pl_g0),
            vertex: wgpu::VertexState {
                module: &sh_part,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<ParticleInst>() as u64, step_mode: wgpu::VertexStepMode::Instance, attributes: &pattrs })],
            },
            primitive: prim_state(false, true),
            depth_stencil: depth_state(false, 0),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &sh_part,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: HDR, blend: Some(premul), write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let fsq = |label: &str, layout: &wgpu::PipelineLayout, module: &wgpu::ShaderModule, fs: &str, format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState { module, entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },
                primitive: prim_state(false, false),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState { module, entry_point: Some(fs), compilation_options: Default::default(), targets: &[Some(format.into())] }),
                multiview_mask: None,
                cache: None,
            })
        };
        let p_post = fsq("post", &pl_post, &sh_post, "fs_post", LDR);
        let p_bright = fsq("bright", &pl_src, &sh_bloom, "fs_bright", HDR);
        let p_blur_h = fsq("blur_h", &pl_src, &sh_bloom, "fs_blur_h", HDR);
        let p_blur_v = fsq("blur_v", &pl_src, &sh_bloom, "fs_blur_v", HDR);
        let p_blit = fsq("blit", &pl_src, &sh_bloom, "fs_blit", config.format);
        let uattrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Unorm8x4];
        let p_ui = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui"),
            layout: Some(&pl_ui),
            vertex: wgpu::VertexState {
                module: &sh_ui,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<UiVertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &uattrs })],
            },
            primitive: prim_state(false, false),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &sh_ui,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: config.format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });

        // ---- samplers, textures
        let samp = |filter: wgpu::FilterMode, rep: bool, cmp: bool| {
            let am = if rep { wgpu::AddressMode::Repeat } else { wgpu::AddressMode::ClampToEdge };
            device.create_sampler(&wgpu::SamplerDescriptor {
                address_mode_u: am,
                address_mode_v: am,
                address_mode_w: am,
                mag_filter: filter,
                min_filter: filter,
                compare: if cmp { Some(wgpu::CompareFunction::LessEqual) } else { None },
                ..Default::default()
            })
        };
        let samp_nearest_rep = samp(wgpu::FilterMode::Nearest, true, false);
        let samp_nearest = samp(wgpu::FilterMode::Nearest, false, false);
        let samp_linear = samp(wgpu::FilterMode::Linear, false, false);
        let samp_shadow = samp(wgpu::FilterMode::Linear, false, true);

        let mips = 5;
        let tex_array = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("tex array"),
            size: wgpu::Extent3d { width: TEX as u32, height: TEX as u32, depth_or_array_layers: Tex::Count as u32 },
            mip_level_count: mips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let tex_view = tex_array.create_view(&wgpu::TextureViewDescriptor { dimension: Some(D::D2Array), ..Default::default() });

        let shadow_size = Self::shadow_px(&settings);
        let (_, shadow_view) = make_tex(&device, shadow_size, shadow_size, DEPTH, "shadow");
        let (_, key_view) = make_tex(&device, KEY_FACE * 3, KEY_FACE * 2, DEPTH, "key shadow");

        let globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let g0 = device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &l_g0, entries: &[wgpu::BindGroupEntry { binding: 0, resource: globals_buf.as_entire_binding() }] });
        let g1_scene = Self::scene_bg(&device, &l_scene, &tex_view, &samp_nearest_rep, &shadow_view, &samp_shadow, &key_view);

        // UI atlas
        let ui_tex = device.create_texture_with_data(
            &queue,
            &wgpu::TextureDescriptor {
                label: Some("ui"),
                size: wgpu::Extent3d { width: textures::UI_ATLAS_W as u32, height: textures::UI_ATLAS_H as u32, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &textures::ui_atlas(),
        );
        let ui_view = ui_tex.create_view(&Default::default());
        let ui_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &l_src,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&ui_view) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&samp_nearest) }],
        });

        let prim_vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("prims vb"), contents: bytemuck::cast_slice(&prims.0.verts), usage: wgpu::BufferUsages::VERTEX });
        let prim_ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("prims ib"), contents: bytemuck::cast_slice(&prims.0.idx), usage: wgpu::BufferUsages::INDEX });
        let mkbuf = |size: u64| device.create_buffer(&wgpu::BufferDescriptor { label: None, size, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let inst_cap = 4096 * std::mem::size_of::<Instance>() as u64;
        let part_cap = 8192 * std::mem::size_of::<ParticleInst>() as u64;
        let ui_cap = 16384 * std::mem::size_of::<UiVertex>() as u64;
        let (inst_buf, part_buf, ui_buf) = (mkbuf(inst_cap), mkbuf(part_cap), mkbuf(ui_cap));

        let targets = Self::make_targets(&device, &settings, &config, &l_post, &l_src, &samp_linear, &samp_nearest, &shadow_view, &samp_shadow);
        Renderer {
            surface,
            device,
            queue,
            config,
            settings,
            globals_buf,
            g0,
            g1_scene,
            l_scene,
            l_post,
            l_src,
            tex_array,
            tex_view,
            samp_nearest_rep,
            samp_nearest,
            samp_linear,
            samp_shadow,
            shadow_view,
            shadow_size,
            key_view,
            key_face_buf,
            key_face_bg,
            p_key,
            targets,
            p_scene,
            p_shadow,
            p_particles,
            p_post,
            p_bright,
            p_blur_h,
            p_blur_v,
            p_blit,
            p_ui,
            ui_bg,
            prim_vb,
            prim_ib,
            prim_ranges: prims.1,
            inst_buf,
            inst_cap,
            part_buf,
            part_cap,
            ui_buf,
            ui_cap,
            inst_scratch: Vec::new(),
            srgb_surface,
            capture: None,
            adapter_info,
        }
    }

    fn shadow_px(s: &GfxSettings) -> u32 {
        match s.shadow_quality {
            0 => 16,
            1 => 1024,
            _ => 2048,
        }
    }

    fn scene_bg(device: &wgpu::Device, l: &wgpu::BindGroupLayout, tex: &wgpu::TextureView, s: &wgpu::Sampler, shadow: &wgpu::TextureView, ss: &wgpu::Sampler, key: &wgpu::TextureView) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: l,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(tex) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(s) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(shadow) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(ss) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(key) },
            ],
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn make_targets(
        device: &wgpu::Device,
        s: &GfxSettings,
        config: &wgpu::SurfaceConfiguration,
        l_post: &wgpu::BindGroupLayout,
        l_src: &wgpu::BindGroupLayout,
        lin: &wgpu::Sampler,
        near: &wgpu::Sampler,
        shadow: &wgpu::TextureView,
        ss: &wgpu::Sampler,
    ) -> Targets {
        let h = s.internal_height.clamp(240, 1080);
        let aspect = (config.width as f32 / config.height.max(1) as f32).clamp(1.0, 2.4);
        let w = ((h as f32 * aspect) as u32 + 1) & !1;
        let (_, scene_view) = make_tex(device, w, h, HDR, "scene");
        let (_, depth_view) = make_tex(device, w, h, DEPTH, "depth");
        let (ldr, ldr_view) = make_tex(device, w, h, LDR, "ldr");
        let (_, b0) = make_tex(device, w / 2, h / 2, HDR, "bloom0");
        let (_, b1) = make_tex(device, w / 4, h / 4, HDR, "bloom1");
        let (_, b2) = make_tex(device, w / 4, h / 4, HDR, "bloom2");
        fn tv(v: &wgpu::TextureView) -> wgpu::BindingResource<'_> { wgpu::BindingResource::TextureView(v) }
        let src = |v: &wgpu::TextureView, sm: &wgpu::Sampler| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: l_src,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(v) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sm) }],
            })
        };
        let post_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: l_post,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: tv(&scene_view) },
                wgpu::BindGroupEntry { binding: 1, resource: tv(&depth_view) },
                wgpu::BindGroupEntry { binding: 2, resource: tv(&b2) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(lin) },
                wgpu::BindGroupEntry { binding: 4, resource: tv(shadow) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(ss) },
            ],
        });
        let bloom_bgs = [src(&scene_view, lin), src(&b0, lin), src(&b1, lin)];
        let blit_bg = src(&ldr_view, near);
        Targets { w, h, scene_view, depth_view, ldr, ldr_view, bloom_views: [b0, b1, b2], post_bg, bloom_bgs, blit_bg }
    }

    fn rebuild_targets(&mut self) {
        self.targets = Self::make_targets(&self.device, &self.settings, &self.config, &self.l_post, &self.l_src, &self.samp_linear, &self.samp_nearest, &self.shadow_view, &self.samp_shadow);
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        if w == 0 || h == 0 || (w == self.config.width && h == self.config.height) {
            return;
        }
        self.config.width = w;
        self.config.height = h;
        self.surface.configure(&self.device, &self.config);
        self.rebuild_targets();
    }

    pub fn apply_settings(&mut self, s: &GfxSettings) {
        let old = std::mem::replace(&mut self.settings, s.clone());
        if old.vsync != s.vsync {
            self.config.present_mode = if s.vsync { wgpu::PresentMode::AutoVsync } else { wgpu::PresentMode::AutoNoVsync };
            self.surface.configure(&self.device, &self.config);
        }
        if old.shadow_quality != s.shadow_quality {
            self.shadow_size = Self::shadow_px(s);
            self.shadow_view = make_tex(&self.device, self.shadow_size, self.shadow_size, DEPTH, "shadow").1;
            self.g1_scene = Self::scene_bg(&self.device, &self.l_scene, &self.tex_view, &self.samp_nearest_rep, &self.shadow_view, &self.samp_shadow, &self.key_view);
        }
        self.rebuild_targets();
    }

    /// Virtual UI size: fixed 480 units tall, width follows the window aspect.
    pub fn ui_size(&self) -> (f32, f32) {
        let aspect = self.config.width as f32 / self.config.height.max(1) as f32;
        ((480.0 * aspect).round(), 480.0)
    }
    pub fn window_size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }
    pub fn aspect(&self) -> f32 {
        self.targets.w as f32 / self.targets.h as f32
    }

    /// Generate and upload one texture array layer (with mips). Called incrementally by the loader.
    pub fn upload_layer(&mut self, layer: Tex) {
        let mut data = textures::generate(layer);
        let mut size = TEX;
        for mip in 0..5u32 {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &self.tex_array, mip_level: mip, origin: wgpu::Origin3d { x: 0, y: 0, z: layer as u32 }, aspect: wgpu::TextureAspect::All },
                &data,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(size as u32 * 4), rows_per_image: Some(size as u32) },
                wgpu::Extent3d { width: size as u32, height: size as u32, depth_or_array_layers: 1 },
            );
            data = textures::downsample(&data, size);
            size /= 2;
        }
    }

    pub fn upload_mesh(&self, mb: &MeshBuilder) -> Option<StaticMesh> {
        if mb.idx.is_empty() {
            return None;
        }
        let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for v in &mb.verts {
            min = min.min(Vec3::from(v.pos));
            max = max.max(Vec3::from(v.pos));
        }
        let vb = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::cast_slice(&mb.verts), usage: wgpu::BufferUsages::VERTEX });
        let ib = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::cast_slice(&mb.idx), usage: wgpu::BufferUsages::INDEX });
        Some(StaticMesh { vb, ib, count: mb.idx.len() as u32, min, max })
    }

    fn ensure(device: &wgpu::Device, buf: &mut wgpu::Buffer, cap: &mut u64, need: u64) {
        if need > *cap {
            *cap = need.next_power_of_two();
            *buf = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: *cap, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        }
    }

    pub fn render(&mut self, f: &Frame) {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => return,
            _ => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
        };
        let view = frame.texture.create_view(&Default::default());
        let t = &self.targets;
        let e = &f.env;

        // ---- shadow matrix (texel snapped)
        let ext = 48.0;
        let lview = crate::util::look_at(f.shadow_center + e.sun_dir * 90.0, f.shadow_center, if e.sun_dir.y.abs() > 0.99 { Vec3::X } else { Vec3::Y });
        let lproj = crate::util::ortho(-ext, ext, -ext, ext, 1.0, 220.0);
        let mut svp = lproj * lview;
        let texel = 2.0 / self.shadow_size as f32;
        let o = svp.transform_point3(Vec3::ZERO);
        let snap = Vec3::new((o.x / texel).round() * texel - o.x, (o.y / texel).round() * texel - o.y, 0.0);
        svp = Mat4::from_translation(snap) * svp;
        let shadows_on = self.settings.shadow_quality > 0 && e.sun_col.max_element() > 0.02;

        let vp = f.proj * f.view;
        let inv_view = f.view.inverse();
        let mut g = Globals::zeroed();
        g.view_proj = vp.to_cols_array();
        g.inv_view_proj = vp.inverse().to_cols_array();
        g.shadow_vp = svp.to_cols_array();
        g.cam_pos = f.cam_pos.extend(f.time).into();
        g.cam_right = inv_view.x_axis.truncate().extend(if shadows_on { 1.0 } else { 0.0 }).into();
        g.cam_up = inv_view.y_axis.truncate().extend(if self.srgb_surface { 1.0 } else { 0.0 }).into();
        g.sun_dir = e.sun_dir.extend(e.shaft).into();
        g.sun_col = e.sun_col.extend(e.wetness).into();
        g.ambient = e.ambient.extend(e.warp).into();
        g.fog_col = e.fog_col.extend(e.fog_density).into();
        let steps = [6.0, 12.0, 24.0][self.settings.fog_quality.min(2) as usize];
        g.fog_par = [e.fog_height, steps, 110.0, e.light_scatter];
        g.screen = [t.w as f32, t.h as f32, 1.0, 0.0];
        g.grade = e.grade.extend(e.saturation).into();
        g.fx = [e.levels, e.dither, e.vignette, e.strain];
        let nl = f.lights.len().min(32);
        let nd = f.distorts.len().min(8);
        g.counts = [nl as f32, nd as f32, e.fade, e.flash];
        for (i, l) in f.lights.iter().take(32).enumerate() {
            g.light_pos[i] = l.pos.extend(l.radius).into();
            g.light_col[i] = l.color.extend(0.0).into();
        }
        for (i, d) in f.distorts.iter().take(8).enumerate() {
            g.distort[i] = d.pos.extend(d.radius).into();
            g.distort2[i] = [d.strength, d.swirl, 0.0, 0.0];
        }
        let key_on = self.settings.shadow_quality >= 2 && f.key.is_some() && !f.lights.is_empty();
        let mut faces = [Mat4::IDENTITY; 6];
        if let (true, Some((kp, kr))) = (key_on, f.key) {
            let kproj = crate::util::perspective(std::f32::consts::FRAC_PI_2, 1.0, 0.15, kr.max(1.0));
            let dirs = [(Vec3::X, Vec3::Y), (Vec3::NEG_X, Vec3::Y), (Vec3::Y, Vec3::Z), (Vec3::NEG_Y, Vec3::Z), (Vec3::Z, Vec3::Y), (Vec3::NEG_Z, Vec3::Y)];
            for (i, (d, up)) in dirs.iter().enumerate() {
                faces[i] = kproj * crate::util::look_at(kp, kp + *d, *up);
                g.key_vp[i] = faces[i].to_cols_array();
            }
            g.key = kp.extend(1.0).into();
            let mut fb = vec![0u8; 256 * 6];
            for i in 0..6 {
                fb[i * 256..i * 256 + 64].copy_from_slice(bytemuck::bytes_of(&faces[i].to_cols_array()));
            }
            self.queue.write_buffer(&self.key_face_buf, 0, &fb);
        }
        self.queue.write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(&g));

        // ---- instance upload (slot 0 is the identity instance for static geometry)
        self.inst_scratch.clear();
        self.inst_scratch.push(Instance::identity());
        let mut ranges: Vec<(usize, u32, u32)> = Vec::new();
        for (mesh, b) in f.inst.buckets.iter().enumerate() {
            if !b.is_empty() && mesh < self.prim_ranges.len() {
                let start = self.inst_scratch.len() as u32;
                self.inst_scratch.extend_from_slice(b);
                ranges.push((mesh, start, self.inst_scratch.len() as u32));
            }
        }
        let isz = std::mem::size_of::<Instance>() as u64;
        Self::ensure(&self.device, &mut self.inst_buf, &mut self.inst_cap, self.inst_scratch.len() as u64 * isz);
        self.queue.write_buffer(&self.inst_buf, 0, bytemuck::cast_slice(&self.inst_scratch));
        let np = f.particles.len() as u32;
        if np > 0 {
            Self::ensure(&self.device, &mut self.part_buf, &mut self.part_cap, np as u64 * std::mem::size_of::<ParticleInst>() as u64);
            self.queue.write_buffer(&self.part_buf, 0, bytemuck::cast_slice(f.particles));
        }
        if !f.ui.is_empty() {
            Self::ensure(&self.device, &mut self.ui_buf, &mut self.ui_cap, f.ui.len() as u64 * std::mem::size_of::<UiVertex>() as u64);
            self.queue.write_buffer(&self.ui_buf, 0, bytemuck::cast_slice(f.ui));
        }

        let mut enc = self.device.create_command_encoder(&Default::default());
        let draw_geo = |rp: &mut wgpu::RenderPass, statics: &[&StaticMesh]| {
            rp.set_vertex_buffer(1, self.inst_buf.slice(..));
            for m in statics {
                rp.set_vertex_buffer(0, m.vb.slice(..));
                rp.set_index_buffer(m.ib.slice(..), wgpu::IndexFormat::Uint32);
                rp.draw_indexed(0..m.count, 0, 0..1);
            }
            rp.set_vertex_buffer(0, self.prim_vb.slice(..));
            rp.set_index_buffer(self.prim_ib.slice(..), wgpu::IndexFormat::Uint32);
            for (mesh, a, b) in &ranges {
                let r = self.prim_ranges[*mesh];
                rp.draw_indexed(r.first..r.first + r.count, r.base, *a..*b);
            }
        };
        fn depth_att(v: &wgpu::TextureView) -> Option<wgpu::RenderPassDepthStencilAttachment<'_>> {
            Some(wgpu::RenderPassDepthStencilAttachment { view: v, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }), stencil_ops: None })
        }
        if shadows_on {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some("shadow"), color_attachments: &[], depth_stencil_attachment: depth_att(&self.shadow_view), ..Default::default() });
            rp.set_pipeline(&self.p_shadow);
            rp.set_bind_group(0, &self.g0, &[]);
            draw_geo(&mut rp, &f.shadow_statics);
        }
        if key_on {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some("key shadow"), color_attachments: &[], depth_stencil_attachment: depth_att(&self.key_view), ..Default::default() });
            rp.set_pipeline(&self.p_key);
            rp.set_bind_group(0, &self.g0, &[]);
            for face in 0..6u32 {
                let (x, y) = ((face % 3) * KEY_FACE, (face / 3) * KEY_FACE);
                rp.set_viewport(x as f32, y as f32, KEY_FACE as f32, KEY_FACE as f32, 0.0, 1.0);
                rp.set_scissor_rect(x, y, KEY_FACE, KEY_FACE);
                rp.set_bind_group(2, &self.key_face_bg, &[face * 256]);
                draw_geo(&mut rp, &f.key_statics);
            }
        }
        fn color_att(v: &wgpu::TextureView, clear: Option<Vec3>) -> Option<wgpu::RenderPassColorAttachment<'_>> {
            Some(wgpu::RenderPassColorAttachment {
                view: v,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: match clear {
                        Some(c) => wgpu::LoadOp::Clear(wgpu::Color { r: c.x as f64, g: c.y as f64, b: c.z as f64, a: 1.0 }),
                        None => wgpu::LoadOp::Load,
                    },
                    store: wgpu::StoreOp::Store,
                },
            })
        }
        {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene"),
                color_attachments: &[color_att(&t.scene_view, Some(e.fog_col))],
                depth_stencil_attachment: depth_att(&t.depth_view),
                ..Default::default()
            });
            rp.set_pipeline(&self.p_scene);
            rp.set_bind_group(0, &self.g0, &[]);
            rp.set_bind_group(1, &self.g1_scene, &[]);
            draw_geo(&mut rp, &f.statics);
            if np > 0 {
                rp.set_pipeline(&self.p_particles);
                rp.set_vertex_buffer(0, self.part_buf.slice(..));
                rp.draw(0..4, 0..np);
            }
        }
        let fs_pass = |enc: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, p: &wgpu::RenderPipeline, bg: &wgpu::BindGroup| {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: None, color_attachments: &[color_att(target, Some(Vec3::ZERO))], depth_stencil_attachment: None, ..Default::default() });
            rp.set_pipeline(p);
            rp.set_bind_group(0, &self.g0, &[]);
            rp.set_bind_group(1, bg, &[]);
            rp.draw(0..3, 0..1);
        };
        fs_pass(&mut enc, &t.bloom_views[0], &self.p_bright, &t.bloom_bgs[0]);
        fs_pass(&mut enc, &t.bloom_views[1], &self.p_blur_h, &t.bloom_bgs[1]);
        fs_pass(&mut enc, &t.bloom_views[2], &self.p_blur_v, &t.bloom_bgs[2]);
        fs_pass(&mut enc, &t.ldr_view, &self.p_post, &t.post_bg);

        let final_pass = |enc: &mut wgpu::CommandEncoder, target: &wgpu::TextureView| {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some("present"), color_attachments: &[color_att(target, Some(Vec3::ZERO))], depth_stencil_attachment: None, ..Default::default() });
            rp.set_pipeline(&self.p_blit);
            rp.set_bind_group(0, &self.g0, &[]);
            rp.set_bind_group(1, &t.blit_bg, &[]);
            rp.draw(0..3, 0..1);
            if !f.ui.is_empty() {
                rp.set_pipeline(&self.p_ui);
                rp.set_bind_group(0, &self.ui_bg, &[]);
                rp.set_vertex_buffer(0, self.ui_buf.slice(..));
                rp.draw(0..f.ui.len() as u32, 0..1);
            }
        };
        final_pass(&mut enc, &view);

        // optional screenshot of the composed frame (debug / test harness)
        let mut cap = None;
        if let Some(path) = self.capture.take() {
            let (w, h) = (self.config.width, self.config.height);
            let (ct, cv) = make_tex(&self.device, w, h, self.config.format, "capture");
            final_pass(&mut enc, &cv);
            let bpr = (w * 4).div_ceil(256) * 256;
            let buf = self.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (bpr * h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
            enc.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo { texture: &ct, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bpr), rows_per_image: Some(h) } },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
            cap = Some((path, buf, w, h, bpr));
        }
        let _ = &t.ldr;
        self.queue.submit(Some(enc.finish()));
        self.queue.present(frame);

        if let Some((path, buf, w, h, bpr)) = cap {
            buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
            let _ = self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
            if let Ok(data) = buf.slice(..).get_mapped_range() {
                let bgr = matches!(self.config.format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb);
                let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
                for y in 0..h {
                    for x in 0..w {
                        let i = (y * bpr + x * 4) as usize;
                        let p = &data[i..i + 3];
                        let px = if bgr { [p[2], p[1], p[0]] } else { [p[0], p[1], p[2]] };
                        out.extend_from_slice(&px);
                    }
                }
                let _ = std::fs::write(path, out);
            }
        }
    }
}
