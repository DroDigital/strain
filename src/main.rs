mod render;
mod util;
use glam::{Mat4, Vec3};
use render::mesh::{build_prims, Instance, MeshBuilder, Prim, WHITE};
use render::textures::Tex;
use render::*;
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

struct Running {
    window: Arc<Window>,
    r: Renderer,
    mesh: Option<StaticMesh>,
    frame: u32,
}
#[derive(Default)]
struct App {
    run: Option<Running>,
}
impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.run.is_some() {
            return;
        }
        let window = Arc::new(el.create_window(Window::default_attributes().with_title("STRAIN").with_inner_size(winit::dpi::LogicalSize::new(1280, 720))).unwrap());
        let mut r = Renderer::new(window.clone(), el.owned_display_handle(), GfxSettings::default(), build_prims());
        for i in 0..Tex::Count as u32 {
            r.upload_layer(unsafe { std::mem::transmute::<u32, Tex>(i) });
        }
        let mut mb = MeshBuilder::new();
        mb.boxx(Vec3::new(-40.0, -1.0, -40.0), Vec3::new(40.0, 0.0, 40.0), Tex::Asphalt as u32, Tex::Asphalt as u32, WHITE, 4.0, 4);
        mb.boxx(Vec3::new(6.0, 0.0, -20.0), Vec3::new(20.0, 18.0, 20.0), Tex::BrickWindow as u32, Tex::Roof as u32, WHITE, 3.0, 63);
        mb.boxx(Vec3::new(-20.0, 0.0, -20.0), Vec3::new(-6.0, 12.0, 20.0), Tex::ConcreteWindowLit as u32, Tex::Roof as u32, [255, 255, 255, 200], 3.0, 63);
        let mesh = r.upload_mesh(&mb);
        eprintln!("adapter: {}", r.adapter_info);
        self.run = Some(Running { window, r, mesh, frame: 0 });
    }
    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(run) = &mut self.run else { return };
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(s) => run.r.resize(s.width, s.height),
            WindowEvent::RedrawRequested => {
                run.frame += 1;
                let t = run.frame as f32 / 60.0;
                let cam = Vec3::new(0.0, 2.2, -14.0);
                let mut il = InstList::default();
                il.push(Prim::Limb as u32, Instance::new(Mat4::from_translation(Vec3::new(0.0, 0.0, 0.0)) * Mat4::from_scale(Vec3::new(0.5, 1.8, 0.3)), [0.8, 0.3, 0.3, 0.0], Tex::Flesh as u32 as f32));
                il.push(Prim::Sphere as u32, Instance::new(Mat4::from_translation(Vec3::new(1.5, 1.0, 0.0)), [1.0, 1.0, 1.0, 0.0], Tex::Bone as u32 as f32));
                let lights = [Light { pos: Vec3::new(0.0, 4.0, 2.0), radius: 14.0, color: Vec3::new(3.0, 1.8, 0.6) }];
                let parts = [ParticleInst { pos_size: [-1.5, 1.5, 0.0, 0.4], color: [2.0, 1.0, 0.2, 1.0], par: [0.0, 0.0, 0.0, 1.0], vel: [0.0; 4] }];
                let m = run.mesh.as_ref().unwrap();
                if run.frame == 30 {
                    if let Ok(p) = std::env::var("STRAIN_SHOT") {
                        run.r.capture = Some(p.into());
                    }
                }
                let aspect = run.r.aspect();
                run.r.render(&Frame {
                    view: util::look_at(cam, Vec3::new(0.0, 1.5, 0.0), Vec3::Y),
                    proj: util::perspective(1.0, aspect, 0.1, 300.0),
                    cam_pos: cam,
                    time: t,
                    env: Env { sun_col: Vec3::new(0.5, 0.55, 0.8), ..Default::default() },
                    shadow_center: Vec3::ZERO,
                    statics: vec![m],
                    shadow_statics: vec![m],
                    inst: &il,
                    lights: &lights,
                    distorts: &[],
                    particles: &parts,
                    ui: &[],
                });
                if run.frame == 40 && std::env::var("STRAIN_SHOT").is_ok() {
                    el.exit();
                }
                run.window.request_redraw();
            }
            _ => {}
        }
    }
}
fn main() {
    let el = EventLoop::new().unwrap();
    el.set_control_flow(ControlFlow::Poll);
    el.run_app(&mut App::default()).unwrap();
}
