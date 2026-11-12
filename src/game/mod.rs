//! The game: owns the ECS world, the current area, the player, and runs the fixed-timestep
//! simulation with interpolated rendering.
pub mod ai;
pub mod camera;
pub mod combat;
pub mod comp;
pub mod cutscene;
pub mod draw;
pub mod life;
pub mod loot;
pub mod magic;
pub mod player;
pub mod progress;
pub mod save;
pub mod spawn;
pub mod ui;

use crate::anim::{BodyPlan, Rig};
use crate::audio::Audio;
use crate::data::Tables;
use crate::ecs::Entity;
use crate::input::{Action, Input};
use crate::particles::Particles;
use crate::physics::{SpatialHash, Statics};
use crate::render::{Distort, InstList, Light, ParticleInst, Renderer, UiVertex};
use crate::util::{flat, Rng};
use crate::world::{pit_pos, Area};
use camera::Camera;
use comp::*;
use glam::{Vec2, Vec3};
use life::{Heli, Weather};
use player::PlayerState;
use progress::{HuskRec, Profile};
use save::Settings;
use std::collections::{HashMap, HashSet};
use ui::UiState;

pub const DT: f32 = 1.0 / 60.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    None,
    Loading,
    Title,
    Pause,
    Inventory,
    Vendor,
    Stash,
    Pit,
    LevelUp,
    Settings,
    Controls,
    Note,
    Dead,
    Credits,
}
impl Screen {
    /// Screens that freeze the simulation.
    pub fn pauses(self) -> bool {
        !matches!(self, Screen::None | Screen::Dead | Screen::Credits)
    }
}

/// Input distilled for one frame.
#[derive(Clone, Default)]
pub struct InputSnap {
    pub mv: Vec2,
    pub look: Vec2,
    pub pressed: Vec<Action>,
    pub down: Vec<Action>,
    pub released: Vec<Action>,
    pub look_flick: f32,
    pub dodge_held_t: f32,
    pub mouse_moved: bool,
    pub click: bool,
    pub rclick: bool,
    pub text_mode: bool,
}

#[derive(Default, Clone)]
pub struct Labels {
    pub light: String,
    pub heavy: String,
    pub block: String,
    pub dodge: String,
    pub heal: String,
    pub cast: String,
    pub lock: String,
    pub menu: String,
    pub interact: String,
}

pub struct Game {
    pub t: Tables,
    pub w: World,
    pub area: Area,
    pub player: Entity,
    pub pl: PlayerState,
    pub prof: Profile,
    pub settings: Settings,
    pub settings_dirty: bool,
    pub rng: Rng,
    pub parts: Particles,
    pub audio: Audio,
    pub cam: Camera,
    pub screen: Screen,
    pub ui: UiState,
    pub input_snapshot: InputSnap,
    pub input_labels: Labels,
    pub hash: SpatialHash,
    pub noises: Vec<(Vec3, f32)>,
    pub consumed: HashSet<((i32, i32), usize)>,
    pub chunk_ents: HashMap<(i32, i32), Vec<Entity>>,
    pub mimic_plans: HashMap<u32, BodyPlan>,
    pub cache_origin: HashMap<u32, ((i32, i32), usize)>,
    pub static_rigs: HashMap<u32, Rig>,
    pub boss: Option<Entity>,
    pub boss_awake: bool,
    pub boss_intro_done: bool,
    pub rush_hit: bool,
    pub mutation_choices: Vec<usize>,
    pub echo_queue: Vec<(usize, f32)>,
    pub note: Option<String>,
    pub return_pos: Option<Vec3>,
    pub risen_husk: Option<(Vec3, HuskRec)>,
    pub cur_district: Option<usize>,
    pub cutscene: Option<cutscene::Cutscene>,
    pub caption: Option<String>,
    pub cut_title: Option<(String, String)>,
    // feedback
    pub hitstop: f32,
    pub slowmo: f32,
    pub flash: f32,
    pub fade: f32,
    pub hurt_flash: f32,
    pub hp_trail: f32,
    pub boss_trail: f32,
    pub dead_t: f32,
    pub tutorial_t: f32,
    pub messages: Vec<(String, [f32; 3], f32)>,
    pub subtitle_line: Option<(String, f32)>,
    pub big_banner: Option<(String, f32)>,
    pub floats: Vec<(Vec3, String, [f32; 3], f32)>,
    pub speech: Vec<(Entity, String, f32)>,
    pub tethers_fx: Vec<(Vec3, Vec3, f32, [f32; 3])>,
    pub spikes_fx: Vec<(Vec3, f32)>,
    pub lights_temp: Vec<(Vec3, Vec3, f32, f32)>,
    pub distorts_temp: Vec<(Vec3, f32, f32, f32)>,
    pub beams: Vec<(Vec3, Vec3)>,
    pub ring_hits: Vec<(u32, u32)>,
    // world life
    pub sim_time: f64,
    pub tod: f32,
    pub weather: Weather,
    pub weather_t: f32,
    pub rain: f32,
    pub spore: f32,
    pub fogw: f32,
    pub wet: f32,
    pub lightning: f32,
    pub thunder_t: f32,
    pub helis: Vec<Heli>,
    pub heli_t: f32,
    pub amb_t: f32,
    pub radio_t: f32,
    // frame scratch
    pub inst: InstList,
    pub part_buf: Vec<ParticleInst>,
    pub light_buf: Vec<Light>,
    pub ui_buf: Vec<UiVertex>,
    pub distorts: Vec<Distort>,
    pub acc: f32,
    pub alpha: f32,
    pub in_game: bool,
    pub has_save: bool,
    pub quit: bool,
    pub adapter: String,
    pub fps: f32,
    pub fps_show: bool,
    pub rebind_wait: Option<(Action, bool)>,
    pub dodge_held: f32,
    pub flick_cd: f32,
    pub title_t: f32,
    pub rebind_armed: bool,
    pub load_progress: f32,
    pub load_label: String,
    pub profile: bool,
    pub spell_test: usize,
    pub pending_ending: f32,
    pub bot_stop_on_death: bool,
    pub bug_steps: u32,
    pub bug_yaw: f32,
    pub build_start: std::time::Instant,
    pub build_ms: f32,
    pub prof_acc: (f32, f32, u32),
}

impl Game {
    pub fn new(t: Tables, settings: Settings, audio: Audio, adapter: String) -> Game {
        let seed = 0x5eed_1234;
        let gear = player::starting_gear(&t, &mut Rng::with_seed(1));
        let mut g = Game {
            w: World::default(),
            area: Area::city(seed),
            player: Entity { id: 0, gen: 0 },
            pl: PlayerState::new(gear),
            prof: Profile::new(seed),
            settings,
            settings_dirty: true,
            rng: Rng::with_seed(0xC0FFEE),
            parts: Particles::new(),
            audio,
            cam: Camera::new(),
            screen: Screen::Title,
            ui: UiState::default(),
            input_snapshot: InputSnap::default(),
            input_labels: Labels::default(),
            hash: SpatialHash::default(),
            noises: Vec::new(),
            consumed: HashSet::new(),
            chunk_ents: HashMap::new(),
            mimic_plans: HashMap::new(),
            cache_origin: HashMap::new(),
            static_rigs: HashMap::new(),
            boss: None,
            boss_awake: false,
            boss_intro_done: false,
            rush_hit: false,
            mutation_choices: Vec::new(),
            echo_queue: Vec::new(),
            note: None,
            return_pos: None,
            risen_husk: None,
            cur_district: None,
            cutscene: None,
            caption: None,
            cut_title: None,
            hitstop: 0.0,
            slowmo: 0.0,
            flash: 0.0,
            fade: 0.0,
            hurt_flash: 0.0,
            hp_trail: 1.0,
            boss_trail: 1.0,
            dead_t: 0.0,
            tutorial_t: 999.0,
            messages: Vec::new(),
            subtitle_line: None,
            big_banner: None,
            floats: Vec::new(),
            speech: Vec::new(),
            tethers_fx: Vec::new(),
            spikes_fx: Vec::new(),
            lights_temp: Vec::new(),
            distorts_temp: Vec::new(),
            beams: Vec::new(),
            ring_hits: Vec::new(),
            sim_time: 0.0,
            tod: 0.86,
            weather: Weather::Rain,
            weather_t: 120.0,
            rain: 0.6,
            spore: 0.0,
            fogw: 0.0,
            wet: 0.7,
            lightning: 0.0,
            thunder_t: 0.0,
            helis: Vec::new(),
            heli_t: 8.0,
            amb_t: 3.0,
            radio_t: 12.0,
            inst: InstList::default(),
            part_buf: Vec::new(),
            light_buf: Vec::new(),
            ui_buf: Vec::new(),
            distorts: Vec::new(),
            acc: 0.0,
            alpha: 0.0,
            in_game: false,
            has_save: save::has_save(),
            quit: false,
            adapter,
            fps: 60.0,
            fps_show: false,
            rebind_wait: None,
            dodge_held: 0.0,
            flick_cd: 0.0,
            title_t: 0.0,
            rebind_armed: false,
            load_progress: 0.0,
            load_label: String::new(),
            profile: std::env::var("STRAIN_PROFILE").is_ok(),
            prof_acc: (0.0, 0.0, 0),
            spell_test: 0,
            pending_ending: 0.0,
            bot_stop_on_death: false,
            bug_steps: 0,
            bug_yaw: 0.0,
            build_start: std::time::Instant::now(),
            build_ms: 0.0,
            t,
        };
        g.make_player();
        g.pl.compute(&g.t);
        g.refresh_player();
        g
    }

    pub fn make_player(&mut self) {
        let e = self.w.spawn();
        let pos = pit_pos(0) + Vec3::new(0.0, 0.0, 3.5);
        self.w.body.insert(e, Body::new(pos, 0.38, 1.8));
        let mut a = Actor::new(Team::Player, 100.0, 20.0, 10.0, 4.2, 1);
        a.max_poise = 20.0;
        self.w.actor.insert(e, a);
        self.w.rig.insert(e, Rig::new(player::player_plan(&self.t, &self.pl)));
        self.player = e;
    }

    // ---- small feedback helpers ------------------------------------------------------------
    pub fn msg(&mut self, s: &str) {
        self.msg_col(s, [0.92, 0.9, 0.85]);
    }
    pub fn msg_col(&mut self, s: &str, c: [f32; 3]) {
        if self.messages.last().map(|m| m.0 == s).unwrap_or(false) {
            self.messages.last_mut().unwrap().2 = 4.0;
            return;
        }
        self.messages.push((s.to_string(), c, 4.0));
        if self.messages.len() > 8 {
            self.messages.remove(0);
        }
    }
    pub fn subtitle(&mut self, s: &str, dur: f32) {
        self.subtitle_line = Some((s.to_uppercase(), dur));
    }
    pub fn say(&mut self, e: Entity, s: &str) {
        self.speech.retain(|x| x.0 != e);
        self.speech.push((e, s.to_uppercase(), 3.5));
    }
    pub fn float_text(&mut self, p: Vec3, s: &str, c: [f32; 3]) {
        if self.floats.len() < 40 {
            self.floats.push((p, s.to_string(), c, 1.0));
        }
    }
    pub fn shake(&mut self, k: f32) {
        self.cam.trauma = (self.cam.trauma + k).min(1.0);
    }
    pub fn screen_allows_control(&self) -> bool {
        self.screen == Screen::None && self.cutscene.is_none()
    }

    pub fn find_lock_target(&self, flick: Option<f32>) -> Option<Entity> {
        let pp = self.ppos();
        let fwd = crate::util::yaw_dir(self.cam.yaw);
        let cur = self.pl.lock;
        let cur_ang = cur.and_then(|c| self.w.body.get(c)).map(|b| {
            let d = flat(b.pos - pp);
            d.x.atan2(d.z)
        });
        let mut best: Option<(Entity, f32)> = None;
        let mut nearest: Option<(Entity, f32)> = None;
        self.hash.query(pp, 22.0, |id, p, _| {
            let e = self.w.ent(id);
            if Some(e) == cur || e == self.player {
                return;
            }
            let Some(a) = self.w.actor.get(e) else { return };
            let hostile = a.team == Team::Infected || self.w.ai.get(e).map(|ai| ai.grudge > 40.0).unwrap_or(false);
            if !a.alive || !hostile || self.w.foe.get(e).map(|f| f.mimic).unwrap_or(false) {
                return;
            }
            let d = flat(p - pp);
            let dist = d.length();
            if dist < 0.1 || !self.area.line_of_sight(pp + Vec3::Y * 1.5, p + Vec3::Y) {
                return;
            }
            if flick.is_none() && dist < 12.0 && nearest.map(|n| dist < n.1).unwrap_or(true) {
                nearest = Some((e, dist));
            }
            let score = match (flick, cur_ang) {
                (Some(f), Some(ca)) => {
                    let da = crate::util::wrap_angle(d.x.atan2(d.z) - ca);
                    // flick right (positive) picks targets clockwise
                    if da * -f <= 0.05 {
                        return;
                    }
                    da.abs() * 10.0 + dist * 0.1
                }
                _ => {
                    let facing = d.dot(fwd) / dist;
                    if facing < 0.2 {
                        return;
                    }
                    dist * (2.0 - facing)
                }
            };
            if best.map(|b| score < b.1).unwrap_or(true) {
                best = Some((e, score));
            }
        });
        // nothing ahead of the camera: take the closest threat around you
        best.or(nearest).map(|b| b.0)
    }

    // ---- lifecycle --------------------------------------------------------------------------
    pub fn new_game(&mut self) {
        let seed = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(7) & 0xffff_ffff) as u32;
        let seed = std::env::var("STRAIN_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(seed);
        self.prof = Profile::new(seed);
        self.rng = Rng::with_seed(seed as u64);
        let gear = player::starting_gear(&self.t, &mut Rng::with_seed(seed as u64));
        self.pl = PlayerState::new(gear);
        self.restart_world();
        self.in_game = true;
        self.screen = Screen::None;
        self.has_save = true;
        self.save();
        if std::env::var("STRAIN_SKIP_INTRO").is_err() {
            self.start_cutscene(cutscene::CutKind::Opening);
        } else {
            self.prof.intro_seen = true;
        }
    }

    pub fn continue_game(&mut self) {
        let Some(s) = save::load_game() else {
            self.new_game();
            return;
        };
        self.prof = s.profile;
        self.pl = PlayerState::new(s.gear);
        self.pl.life = s.life;
        self.rng = Rng::with_seed(self.prof.seed as u64 ^ self.prof.epoch as u64);
        self.restart_world();
        self.pl.heal_charges = s.heal_charges.min(self.pl.stats.heal_charges);
        self.in_game = true;
        self.screen = Screen::None;
        self.msg("YOU WAKE BY THE BURN PIT.");
    }

    fn restart_world(&mut self) {
        self.w.clear();
        self.make_player();
        let fx = self.pl.mutation_effects(&self.t);
        let displaced = self.pl.apply_hand_mutations(&self.t, &fx);
        for it in displaced {
            let _ = self.pl.gear.inv.add(&self.t, it);
        }
        self.refresh_player();
        if let Some(a) = self.w.actor.get_mut(self.player) {
            a.hp = a.max_hp;
        }
        self.pl.heal_charges = self.pl.stats.heal_charges;
        self.pl.stamina = self.pl.stats.max_stam;
        self.cutscene = None;
        self.cam.over = None;
        self.messages.clear();
        self.prof.vendor_stock.clear();
        let pit = self.prof.last_pit;
        self.rebuild_city(pit_pos(pit) + Vec3::new(0.0, 0.0, 3.5));
        self.cam.yaw = std::f32::consts::PI;
        self.cur_district = None;
    }

    pub fn to_title(&mut self) {
        self.screen = Screen::Title;
        self.in_game = false;
        self.cutscene = None;
        self.cam.over = None;
        self.has_save = save::has_save();
        self.ui.sel = 0;
    }

    pub fn save(&mut self) {
        if !self.in_game {
            return;
        }
        let s = save::SaveFile { version: save::SAVE_VERSION, profile: self.prof.clone(), gear: self.pl.gear.clone(), life: self.pl.life.clone(), heal_charges: self.pl.heal_charges };
        save::save_game(&s);
    }

    pub fn stream_now(&mut self, force: bool) {
        let pp = self.ppos();
        let (loaded, unloaded) = self.area.stream(pp, 2, if force { 64 } else { 1 });
        for k in unloaded {
            self.depopulate(k);
        }
        for k in loaded {
            self.populate(k);
        }
    }

    // ---- per frame ------------------------------------------------------------------------------
    pub fn frame(&mut self, r: &mut Renderer, input: &mut Input, real_dt: f32) {
        let dt = real_dt.min(0.1);
        self.fps = self.fps * 0.95 + (1.0 / real_dt.max(1e-4)) * 0.05;
        self.snapshot_input(input, r, dt);
        // rebinding capture
        if let Some(rb) = self.rebind_wait {
            if input.rebinding.is_none() && !input_started(input, rb) {
                input.rebinding = Some(rb);
            }
        }
        if self.rebind_wait.is_some() && input.rebinding.is_none() && self.rebind_armed {
            self.rebind_wait = None;
            self.rebind_armed = false;
            self.settings.bindings = input.bindings.clone();
            self.settings_dirty = true;
            save::save_settings(&self.settings);
        }
        if input.rebinding.is_some() {
            self.rebind_armed = true;
        }
        if self.settings_dirty {
            input.bindings = self.settings.bindings.clone();
            self.parts.density = self.settings.gfx.particle_scale();
            self.cam.fov = self.settings.fov;
        }
        let t0 = std::time::Instant::now();
        self.update(dt);
        let t1 = std::time::Instant::now();
        self.audio.listener = self.cam.eye;
        let f = crate::util::yaw_dir(self.cam.yaw);
        self.audio.listener_right = Vec3::new(-f.z, 0.0, f.x);
        self.audio.set_music(self.music_state());
        let alpha = self.alpha;
        self.draw(r, alpha, dt * self.time_scale());
        let t2 = std::time::Instant::now();
        if self.profile {
            self.prof_acc.0 += (t1 - t0).as_secs_f32();
            self.prof_acc.1 += (t2 - t1).as_secs_f32();
            self.prof_acc.2 += 1;
            if self.prof_acc.2 >= 120 {
                let n = self.prof_acc.2 as f32;
                eprintln!("profile: build {:.2} ms, update {:.2} ms, draw+submit {:.2} ms, entities {}, particles {}, instances {}", self.build_ms, self.prof_acc.0 / n * 1000.0, self.prof_acc.1 / n * 1000.0, self.w.body.ids().count(), self.parts.list.len(), self.inst.buckets.iter().map(|b| b.len()).sum::<usize>());
                self.prof_acc = (0.0, 0.0, 0);
            }
        }
        input.end_frame();
    }

    fn snapshot_input(&mut self, input: &mut Input, r: &Renderer, dt: f32) {
        input.poll_pad(dt);
        let all = [
            Action::MoveF, Action::MoveB, Action::MoveL, Action::MoveR, Action::Light, Action::Heavy, Action::Block, Action::Dodge, Action::Interact, Action::Heal, Action::Cast, Action::NextSpell, Action::LockOn, Action::Menu, Action::Pause, Action::UiUp, Action::UiDown, Action::UiLeft, Action::UiRight, Action::UiAccept, Action::UiBack, Action::TabPrev, Action::TabNext, Action::UiAlt, Action::UiDrop,
        ];
        let mut s = InputSnap { mv: input.move_axis(), look: input.look, mouse_moved: input.mouse_moved, click: input.click, rclick: input.rclick, ..Default::default() };
        for a in all {
            if input.pressed(a) {
                s.pressed.push(a);
            }
            if input.down(a) {
                s.down.push(a);
            }
            if input.released(a) {
                s.released.push(a);
            }
        }
        if input.down(Action::Dodge) {
            self.dodge_held += dt;
        }
        s.dodge_held_t = self.dodge_held;
        if !input.down(Action::Dodge) && !input.released(Action::Dodge) {
            self.dodge_held = 0.0;
        }
        // flicking the look input switches lock targets
        self.flick_cd -= dt;
        if self.pl.lock.is_some() && self.flick_cd <= 0.0 && input.look.x.abs() > 0.06 {
            s.look_flick = input.look.x.signum();
            self.flick_cd = 0.35;
        }
        let (ww, wh) = r.window_size();
        let (uw, uh) = r.ui_size();
        self.ui.mouse = Vec2::new(input.mouse.x / ww.max(1) as f32 * uw, input.mouse.y / wh.max(1) as f32 * uh);
        self.input_snapshot = s;
        let l = |a: Action| input.bind_label(a);
        self.input_labels = Labels { light: l(Action::Light), heavy: l(Action::Heavy), block: l(Action::Block), dodge: l(Action::Dodge), heal: l(Action::Heal), cast: l(Action::Cast), lock: l(Action::LockOn), menu: l(Action::Menu), interact: l(Action::Interact) };
    }

    fn time_scale(&self) -> f32 {
        if self.slowmo > 0.0 {
            0.3
        } else {
            1.0
        }
    }

    pub fn update(&mut self, dt: f32) {
        let pressed = |g: &Game, a: Action| g.input_snapshot.pressed.contains(&a);
        if self.screen == Screen::Title {
            self.title_t += dt;
            self.ui.t += dt;
            // slow orbit over the city behind the title
            let c = pit_pos(0);
            let a = self.title_t * 0.05;
            self.cam.over = Some((c + Vec3::new(a.cos() * 60.0, 28.0, a.sin() * 60.0), c + Vec3::Y * 4.0));
            self.cam.update(dt, Vec2::ZERO, false, c, None, &self.area, 0.0);
            self.stream_now(false);
            self.life_tick(dt);
            self.parts.update(dt);
            self.ui_update(dt);
            return;
        }
        // global menu keys
        if self.screen == Screen::None && self.cutscene.is_none() {
            if pressed(self, Action::Pause) {
                self.screen = Screen::Pause;
                self.ui.sel = 0;
                self.input_snapshot.pressed.clear();
            } else if pressed(self, Action::Menu) {
                self.screen = Screen::Inventory;
                self.ui.focus = 0;
                self.input_snapshot.pressed.clear();
            } else if self.pl.life.pending_levels > 0 && !self.mutation_choices.is_empty() && pressed(self, Action::UiAccept) {
                self.screen = Screen::LevelUp;
            }
        } else if self.screen != Screen::None {
            self.ui_update(dt);
        }
        if self.screen == Screen::Dead {
            self.dead_t += dt;
        }
        // a level earned while a menu was open is offered as soon as play resumes
        if self.screen == Screen::None && self.cutscene.is_none() && self.pl.life.pending_levels > 0 && self.mutation_choices.is_empty() && self.w.actor.get(self.player).map(|a| a.alive).unwrap_or(false) {
            self.open_level_up();
        }
        let skip = pressed(self, Action::UiAccept) || pressed(self, Action::UiBack) || pressed(self, Action::Pause);
        let in_cut = self.cutscene_tick(dt, skip);
        if in_cut {
            self.input_snapshot.pressed.clear();
        }

        // timers in real time
        self.hitstop = (self.hitstop - dt).max(0.0);
        self.slowmo = (self.slowmo - dt).max(0.0);
        self.flash = (self.flash - dt * 1.5).max(0.0);
        self.hurt_flash = (self.hurt_flash - dt * 0.8).max(0.0);
        self.tutorial_t += dt;
        for m in &mut self.messages {
            m.2 -= dt;
        }
        self.messages.retain(|m| m.2 > 0.0);
        for f in &mut self.floats {
            f.3 -= dt;
        }
        self.floats.retain(|f| f.3 > 0.0);
        for s in &mut self.speech {
            s.2 -= dt;
        }
        self.speech.retain(|s| s.2 > 0.0);
        if let Some((_, t)) = &mut self.subtitle_line {
            *t -= dt;
            if *t <= 0.0 {
                self.subtitle_line = None;
            }
        }
        if let Some((_, t)) = &mut self.big_banner {
            *t -= dt;
            if *t <= 0.0 {
                self.big_banner = None;
            }
        }
        if let Some((_, t)) = &mut self.pl.last_hit_target {
            *t -= dt;
            if *t <= 0.0 {
                self.pl.last_hit_target = None;
            }
        }
        for x in &mut self.tethers_fx {
            x.2 -= dt;
        }
        self.tethers_fx.retain(|x| x.2 > 0.0);
        for x in &mut self.spikes_fx {
            x.1 -= dt;
        }
        self.spikes_fx.retain(|x| x.1 > 0.0);
        for x in &mut self.lights_temp {
            x.3 -= dt;
        }
        self.lights_temp.retain(|x| x.3 > 0.0);
        self.distorts_temp.clear();
        self.beams.clear();
        if let Some(a) = self.w.actor.get(self.player) {
            let f = a.frac();
            self.hp_trail = if self.hp_trail > f { (self.hp_trail - dt * 0.4).max(f) } else { f };
        }
        if let Some(b) = self.boss.and_then(|b| self.w.actor.get(b)) {
            let f = b.frac();
            self.boss_trail = if self.boss_trail > f { (self.boss_trail - dt * 0.3).max(f) } else { f };
        }

        // fixed-step simulation
        let paused = self.screen.pauses();
        if !paused && self.hitstop <= 0.0 {
            self.acc += dt * self.time_scale();
            let mut steps = 0;
            while self.acc >= DT && steps < 5 {
                self.step(DT);
                self.acc -= DT;
                steps += 1;
                // edge-triggered input belongs to the first step of the frame only
                self.input_snapshot.pressed.clear();
                self.input_snapshot.released.clear();
                self.input_snapshot.look_flick = 0.0;
            }
            if steps == 5 {
                self.acc = 0.0;
            }
        }
        self.alpha = (self.acc / DT).clamp(0.0, 1.0);
        if self.pending_ending > 0.0 {
            self.pending_ending -= dt;
            if self.pending_ending <= 0.0 {
                // back to the crater for the ending
                let site = crate::world::rift_site(4);
                self.return_pos = Some(site + Vec3::new(0.0, 0.0, 8.0));
                self.exit_rift();
                self.start_cutscene(cutscene::CutKind::Ending);
            }
        }
        if !paused {
            self.parts.update(dt * self.time_scale());
        }
        self.stream_now(false);

        // camera
        let pp = self.player_render_pos();
        let lock = self.pl.lock.and_then(|l| self.w.body.get(l)).map(|b| b.pos);
        let look = if self.screen_allows_control() { self.input_snapshot.look } else { Vec2::ZERO };
        let shake = self.settings.shake;
        let inv = self.settings.bindings.invert_y;
        self.cam.update(dt, look, inv, pp, lock, &self.area, shake);
    }

    fn player_render_pos(&self) -> Vec3 {
        self.w.body.get(self.player).map(|b| b.prev.lerp(b.pos, self.alpha)).unwrap_or(Vec3::ZERO)
    }

    /// One fixed simulation step.
    fn step(&mut self, dt: f32) {
        self.sim_time += dt as f64;
        let ids: Vec<u32> = self.w.body.ids().collect();
        for id in &ids {
            let e = self.w.ent(*id);
            if let Some(b) = self.w.body.get_mut(e) {
                b.prev = b.pos;
                b.prev_yaw = b.yaw;
            }
        }
        for (_, a) in self.w.actor.iter_mut() {
            a.tscale = 1.0;
            a.ward = 0.0;
            a.rally = 1.0;
            a.glow = (a.glow - dt * 2.0).max(0.0);
        }
        self.sanitize_handles();
        self.rebuild_hash();
        self.player_tick(dt);
        self.ai_tick(dt);
        self.actions_tick(dt);
        self.integrate(dt);
        self.rebuild_hash();
        self.proj_tick(dt);
        self.zones_tick(dt);
        self.status_tick(dt);
        self.pickups_tick(dt);
        self.echo_tick(dt);
        self.progress_tick();
        self.life_tick(dt);
        // corpses fade and are removed
        let dead: Vec<Entity> = self.w.actor.iter().filter(|(id, a)| !a.alive && a.dead_t > 7.5 && *id != self.player.id).map(|(id, _)| self.w.ent(id)).collect();
        for e in dead {
            self.mimic_plans.remove(&e.id);
            self.w.kill(e);
        }
        self.w.flush();
        self.noises.clear();
    }

    /// Drop stored entity handles whose entity has been despawned (ids are reused).
    fn sanitize_handles(&mut self) {
        let alive = |w: &World, e: Option<Entity>| e.filter(|e| w.alive(*e));
        let ids: Vec<u32> = self.w.ai.ids().collect();
        for id in ids {
            let e = self.w.ent(id);
            let t = self.w.ai.get(e).and_then(|a| a.target);
            if t.is_some() && alive(&self.w, t).is_none() {
                self.w.ai.get_mut(e).unwrap().target = None;
            }
        }
        let ids: Vec<u32> = self.w.proj.ids().collect();
        for id in ids {
            let e = self.w.ent(id);
            let (h, o) = self.w.proj.get(e).map(|p| (p.homing, p.owner)).unwrap();
            let (h, o) = (alive(&self.w, h), alive(&self.w, o));
            let p = self.w.proj.get_mut(e).unwrap();
            p.homing = h;
            p.owner = o;
        }
        let ids: Vec<u32> = self.w.zone.ids().collect();
        for id in ids {
            let e = self.w.ent(id);
            let o = self.w.zone.get(e).and_then(|z| z.owner);
            let o = alive(&self.w, o);
            self.w.zone.get_mut(e).unwrap().owner = o;
        }
        self.pl.lock = alive(&self.w, self.pl.lock);
        self.boss = alive(&self.w, self.boss);
        if let Some((e, _)) = self.pl.last_hit_target {
            if !self.w.alive(e) {
                self.pl.last_hit_target = None;
            }
        }
        let w = &self.w;
        self.speech.retain(|s| w.alive(s.0));
    }

    fn rebuild_hash(&mut self) {
        self.hash.clear();
        for (id, a) in self.w.actor.iter() {
            if a.alive {
                let e = self.w.ent(id);
                if let Some(b) = self.w.body.get(e) {
                    self.hash.insert(id, b.pos, b.radius);
                }
            }
        }
    }

    /// Move bodies with wall sliding, body separation and decaying pushes.
    fn integrate(&mut self, dt: f32) {
        let ids: Vec<u32> = self.w.actor.ids().collect();
        for id in &ids {
            let e = self.w.ent(*id);
            let (alive, ts) = self.w.actor.get(e).map(|a| (a.alive, a.tscale)).unwrap();
            let Some(b) = self.w.body.get(e).copied() else { continue };
            let vel = if alive { b.vel * ts } else { Vec3::ZERO };
            let delta = (vel + b.push) * dt;
            let (mut np, _) = if delta.length_squared() > 1e-10 { self.area.move_cyl(b.pos, delta, b.radius, b.height) } else { (b.pos, false) };
            np.y = self.area.ground(np.x, np.z);
            // keep inside rift arenas during boss fights
            if self.boss_awake && self.boss.is_some() {
                let (c, r) = self.area.arena;
                if r > 0.0 && (b.pos - c).length() < r + 2.0 {
                    let d = flat(np - c);
                    if d.length() > r {
                        np = c + d.normalize() * r + Vec3::Y * np.y;
                    }
                }
            }
            let bm = self.w.body.get_mut(e).unwrap();
            bm.pos = np;
            bm.push *= (1.0 - 7.0 * dt).max(0.0);
        }
        // body separation
        self.rebuild_hash();
        for id in ids {
            let e = self.w.ent(id);
            let Some(b) = self.w.body.get(e).copied() else { continue };
            if !b.solid || b.ghost || !self.w.actor.get(e).map(|a| a.alive).unwrap_or(false) {
                continue;
            }
            let mut push = Vec3::ZERO;
            self.hash.query(b.pos, b.radius + 1.2, |oid, op, or| {
                if oid == id {
                    return;
                }
                let d = flat(b.pos - op);
                let l = d.length();
                let min = b.radius + or;
                if l < min && l > 1e-4 {
                    push += d / l * (min - l) * 0.5;
                }
            });
            if push.length_squared() > 0.0 {
                let ghost_other = false;
                let _ = ghost_other;
                let mass = b.mass.max(0.2);
                let (np, _) = self.area.move_cyl(b.pos, push / mass.sqrt(), b.radius, b.height);
                self.w.body.get_mut(e).unwrap().pos = np;
            }
        }
    }

}

fn input_started(input: &Input, rb: (Action, bool)) -> bool {
    input.rebinding == Some(rb)
}
