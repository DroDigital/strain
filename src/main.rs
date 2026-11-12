//! STRAIN — entry point: window, event loop, loading screen and settings application.
#![allow(clippy::too_many_arguments, clippy::type_complexity)]
mod anim;
mod audio;
mod data;
mod ecs;
mod game;
mod input;
mod particles;
mod physics;
mod render;
mod util;
mod world;

use game::{Game, Screen};
use input::Input;
use render::mesh::build_prims;
use render::textures::Tex;
use render::{Renderer, WindowMode};
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Fullscreen, Window, WindowId};

mod autotest;

struct Running {
    window: Arc<Window>,
    r: Renderer,
    game: Game,
    input: Input,
    last: Instant,
    load_step: u32,
    grabbed: bool,
    focused: bool,
    test: Option<autotest::AutoTest>,
    applied_mode: Option<WindowMode>,
}

#[derive(Default)]
struct App {
    run: Option<Running>,
}

const LOAD_STEPS: u32 = Tex::Count as u32 + 3;

impl Running {
    fn loading(&mut self) -> bool {
        if self.load_step >= LOAD_STEPS {
            return false;
        }
        let s = self.load_step;
        if s < Tex::Count as u32 {
            // a few layers per frame keeps the bar moving
            for k in 0..4 {
                let i = s + k;
                if i < Tex::Count as u32 {
                    self.r.upload_layer(unsafe { std::mem::transmute::<u32, Tex>(i) });
                }
            }
            self.load_step = (s + 4).min(Tex::Count as u32);
            self.game.load_label = "GROWING TEXTURES".into();
        } else if s == Tex::Count as u32 {
            self.game.audio.build_bank();
            self.load_step += 1;
            self.game.load_label = "SYNTHESISING SOUND".into();
        } else if s == Tex::Count as u32 + 1 {
            self.game.stream_now(true);
            self.load_step += 1;
            self.game.load_label = "RAISING THE CITY".into();
        } else {
            self.load_step += 1;
            self.game.screen = Screen::Title;
            self.game.load_label = "READY".into();
        }
        self.game.load_progress = self.load_step as f32 / LOAD_STEPS as f32;
        true
    }

    fn apply_settings(&mut self) {
        if !self.game.settings_dirty {
            return;
        }
        let gfx = self.game.settings.gfx.clone();
        self.r.apply_settings(&gfx);
        if self.applied_mode != Some(gfx.window_mode) {
            self.applied_mode = Some(gfx.window_mode);
            match gfx.window_mode {
                WindowMode::Windowed => self.window.set_fullscreen(None),
                WindowMode::Borderless => self.window.set_fullscreen(Some(Fullscreen::Borderless(None))),
                WindowMode::Fullscreen => {
                    let mode = self.window.current_monitor().and_then(|m| m.video_modes().max_by_key(|v| (v.size().width * v.size().height, v.refresh_rate_millihertz())));
                    match mode {
                        Some(m) => self.window.set_fullscreen(Some(Fullscreen::Exclusive(m))),
                        None => self.window.set_fullscreen(Some(Fullscreen::Borderless(None))),
                    }
                }
            }
        }
        self.game.settings_dirty = false;
    }

    fn update_grab(&mut self) {
        let want = self.focused && self.game.in_game && self.game.screen == Screen::None && self.test.is_none();
        if want != self.grabbed {
            self.grabbed = want;
            if want {
                let _ = self.window.set_cursor_grab(CursorGrabMode::Locked).or_else(|_| self.window.set_cursor_grab(CursorGrabMode::Confined));
            } else {
                let _ = self.window.set_cursor_grab(CursorGrabMode::None);
            }
            self.window.set_cursor_visible(!want);
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.run.is_some() {
            return;
        }
        let settings = game::save::load_settings();
        let attrs = Window::default_attributes().with_title("STRAIN").with_inner_size(winit::dpi::LogicalSize::new(1280, 720)).with_min_inner_size(winit::dpi::LogicalSize::new(640, 360));
        let window = Arc::new(el.create_window(attrs).expect("create window"));
        let r = Renderer::new(window.clone(), el.owned_display_handle(), settings.gfx.clone(), build_prims());
        let tables = data::Tables::load();
        let audio = audio::Audio::new();
        let adapter = r.adapter_info.clone();
        eprintln!("STRAIN: adapter {adapter}, audio {}", if audio.ok { "ok" } else { "unavailable" });
        let input = Input::new(settings.bindings.clone());
        let mut game = Game::new(tables, settings, audio, adapter);
        game.screen = Screen::Loading;
        let test = autotest::AutoTest::from_env();
        self.run = Some(Running { window, r, game, input, last: Instant::now(), load_step: 0, grabbed: false, focused: true, test, applied_mode: None });
    }

    fn device_event(&mut self, _el: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        if let Some(run) = &mut self.run {
            if let DeviceEvent::MouseMotion { delta } = event {
                if run.grabbed {
                    run.input.mouse_motion(delta.0, delta.1);
                }
            }
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(run) = &mut self.run else { return };
        match event {
            WindowEvent::CloseRequested => {
                run.game.save();
                el.exit();
            }
            WindowEvent::Resized(s) => run.r.resize(s.width, s.height),
            WindowEvent::Focused(f) => run.focused = f,
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    if event.state == ElementState::Pressed && !event.repeat {
                        match code {
                            KeyCode::F3 => run.game.fps_show = !run.game.fps_show,
                            KeyCode::F11 => {
                                let m = &mut run.game.settings.gfx.window_mode;
                                *m = if *m == WindowMode::Windowed { WindowMode::Borderless } else { WindowMode::Windowed };
                                run.game.settings_dirty = true;
                            }
                            KeyCode::F12 => run.r.capture = Some(format!("strain_{}.ppm", run.game.sim_time as u64).into()),
                            _ => {}
                        }
                    }
                    if !event.repeat {
                        run.input.key(code, event.state);
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => run.input.mouse_button(button, state),
            WindowEvent::CursorMoved { position, .. } => run.input.cursor(position.x as f32, position.y as f32),
            WindowEvent::MouseWheel { delta, .. } => {
                run.input.scroll += match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
            }
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let dt = (now - run.last).as_secs_f32();
                run.last = now;
                run.apply_settings();
                if run.loading() {
                    run.game.draw(&mut run.r, 0.0, dt);
                } else {
                    if let Some(t) = &mut run.test {
                        let dt_fixed = 1.0 / 30.0;
                        if t.step(&mut run.game, &mut run.input, &mut run.r) {
                            el.exit();
                        }
                        run.game.frame(&mut run.r, &mut run.input, dt_fixed);
                    } else {
                        run.game.frame(&mut run.r, &mut run.input, dt);
                    }
                    if run.game.quit {
                        el.exit();
                    }
                }
                run.update_grab();
                run.window.request_redraw();
            }
            _ => {}
        }
    }
}

fn main() {
    let el = EventLoop::new().expect("event loop");
    el.set_control_flow(ControlFlow::Poll);
    let mut app = App::default();
    el.run_app(&mut app).expect("run");
}
