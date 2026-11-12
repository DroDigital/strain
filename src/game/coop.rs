//! Local co-op: up to three guests join on their own gamepads (press Start on a second controller,
//! or choose CO-OP: ADD PLAYER in the pause menu). Guests share the host's camera, which frames the
//! whole group. Each has a body, a weapon class, stamina and heal charges of their own and fights
//! with the same movesets; a downed guest can be revived by standing next to them, and stragglers
//! are pulled back to the group.
use super::comp::*;
use super::Game;
use crate::anim::{weapon_tint, BodyPlan, Rig, WeaponVis};
use crate::audio::Sfx;
use crate::data::WeaponClass;
use crate::ecs::Entity;
use crate::input::{PAD_BLOCK, PAD_DODGE, PAD_HEAL, PAD_HEAVY, PAD_LIGHT, PAD_SELECT};
use crate::particles::Fx;
use crate::util::{dir_yaw, flat, turn_toward, yaw_dir};
use glam::{Vec2, Vec3};

pub const MAX_GUESTS: usize = 3;
/// A guest's weapon by slot (P2, P3, P4), so the group has a mix of reach and weight.
const CLASSES: [WeaponClass; MAX_GUESTS] = [WeaponClass::FireAxe, WeaponClass::Pipe, WeaponClass::Cleaver];
pub const COLOURS: [[f32; 3]; 4] = [[0.95, 0.85, 0.6], [0.45, 0.75, 1.0], [0.55, 0.95, 0.5], [1.0, 0.55, 0.8]];

#[derive(Clone, Debug)]
pub struct Guest {
    /// 1, 2 or 3 (the host is player 0)
    pub slot: u8,
    pub pad: usize,
    pub e: Entity,
    pub class: WeaponClass,
    pub stamina: f32,
    pub stam_delay: f32,
    pub heals: u8,
    /// seconds spent down, and revive progress from a teammate standing close
    pub down_t: f32,
    pub revive: f32,
    pub combo: u8,
    pub combo_t: f32,
    pub blocking: bool,
    pub leave_t: f32,
    pub dodge_held: f32,
}

impl Guest {
    pub fn down(&self) -> bool {
        self.down_t > 0.0
    }
}

impl Game {
    pub fn is_guest(&self, e: Entity) -> bool {
        self.guests.iter().any(|g| g.e == e)
    }
    pub fn guest_slot(&self, e: Entity) -> Option<u8> {
        self.guests.iter().find(|g| g.e == e).map(|g| g.slot)
    }
    /// The host and every guest that is up.
    pub fn party(&self) -> Vec<Entity> {
        let mut v = vec![self.player];
        v.extend(self.guests.iter().filter(|g| !g.down()).map(|g| g.e));
        v
    }

    /// A gamepad asks to join. Returns the new guest's slot.
    pub fn add_guest(&mut self, pad: usize) -> Option<u8> {
        if self.guests.len() >= MAX_GUESTS || self.guests.iter().any(|g| g.pad == pad) {
            return None;
        }
        let slot = (1..=MAX_GUESTS as u8).find(|s| !self.guests.iter().any(|g| g.slot == *s))?;
        let class = CLASSES[slot as usize - 1];
        let g = Guest { slot, pad, e: self.player, class, stamina: 100.0, stam_delay: 0.0, heals: 3, down_t: 0.0, revive: 0.0, combo: 0, combo_t: 0.0, blocking: false, leave_t: 0.0, dodge_held: 0.0 };
        self.guests.push(g);
        let i = self.guests.len() - 1;
        self.spawn_guest_body(i);
        self.msg(&format!("PLAYER {} JOINED", slot + 1));
        self.audio.play(Sfx::UiSelect, 0.6, 1.2);
        Some(slot)
    }

    pub fn remove_guest(&mut self, slot: u8) {
        if let Some(i) = self.guests.iter().position(|g| g.slot == slot) {
            let e = self.guests[i].e;
            if self.w.alive(e) {
                if let Some(b) = self.w.body.get(e) {
                    self.parts.burst(Fx::Dust, b.pos, Vec3::Y, 12, 1.0);
                }
                self.w.kill(e);
                self.w.flush();
            }
            self.guests.remove(i);
            self.msg(&format!("PLAYER {} LEFT", slot + 1));
        }
    }

    /// (Re)create a guest's body next to the host: on joining, after area changes, and after a
    /// long time down.
    fn spawn_guest_body(&mut self, i: usize) {
        let host = self.ppos();
        let host_yaw = self.player_body().yaw;
        let slot = self.guests[i].slot;
        let class = self.guests[i].class;
        let side = [Vec3::new(-1.4, 0.0, -0.8), Vec3::new(1.4, 0.0, -0.8), Vec3::new(0.0, 0.0, -1.6)][(slot as usize - 1).min(2)];
        let off = glam::Quat::from_rotation_y(host_yaw) * side;
        let (pos, _) = crate::physics::Statics::move_cyl(&self.area, host + Vec3::Y * 0.05, off, 0.38, 1.75);
        let e = self.w.spawn();
        let seed = 900 + slot as u32 * 31;
        let mut plan = BodyPlan::human(seed);
        plan.hands[0] = Some(WeaponVis { class, tint: weapon_tint(class, seed), glow: 0.0 });
        let mut b = Body::new(pos, 0.38, plan.height());
        b.yaw = host_yaw;
        b.prev_yaw = host_yaw;
        self.w.body.insert(e, b);
        self.w.rig.insert(e, Rig::new(plan));
        // guests keep pace with the host's build
        let st = &self.pl.stats;
        let hp = st.max_hp * 0.9;
        let (_, power) = self.pl.hand_weapon(&self.t, false);
        let mut a = Actor::new(Team::Player, hp, 60.0, power * st.dmg, 4.2, self.pl.life.level);
        a.armor = st.armor;
        self.w.actor.insert(e, a);
        self.guests[i].e = e;
        self.guests[i].down_t = 0.0;
        self.guests[i].revive = 0.0;
        self.apply_skin(e);
    }

    /// Damage that would kill a guest knocks them down instead. Returns true when handled.
    pub fn guest_downed(&mut self, t: Entity) -> bool {
        let Some(i) = self.guests.iter().position(|g| g.e == t) else { return false };
        if self.guests[i].down() {
            return true;
        }
        self.guests[i].down_t = 0.001;
        self.guests[i].revive = 0.0;
        if let Some(a) = self.w.actor.get_mut(t) {
            a.hp = 0.0;
            a.alive = false;
            a.dead_t = 0.0;
            a.act = Act::default();
        }
        if let Some(b) = self.w.body.get_mut(t) {
            b.vel = Vec3::ZERO;
        }
        let slot = self.guests[i].slot;
        self.msg(&format!("PLAYER {} IS DOWN - STAND CLOSE TO REVIVE", slot + 1));
        self.audio.play(Sfx::Hurt, 0.6, 0.8);
        true
    }

    /// Blocking guests take a fraction of the damage, paid partly in stamina.
    pub fn guest_block(&mut self, t: Entity, amount: f32) -> f32 {
        let Some(g) = self.guests.iter_mut().find(|g| g.e == t) else { return amount };
        if !g.blocking || g.stamina <= 0.0 {
            return amount;
        }
        g.stamina -= amount * 0.6;
        g.stam_delay = 0.8;
        amount * 0.25
    }

    pub fn coop_tick(&mut self, dt: f32) {
        if self.guests.is_empty() {
            return;
        }
        let controllable = self.screen_allows_control();
        let host = self.ppos();
        let cam_yaw = self.cam.yaw;
        let fwd = yaw_dir(cam_yaw);
        let right = Vec3::new(-fwd.z, 0.0, fwd.x);
        let mut leave = Vec::new();
        for i in 0..self.guests.len() {
            let g = self.guests[i].clone();
            // bodies are lost on area changes: rebuild next to the host
            if !self.w.alive(g.e) || self.w.body.get(g.e).is_none() {
                self.spawn_guest_body(i);
                continue;
            }
            let inp = self.guest_input.get(&g.pad).cloned().unwrap_or_default();
            let pressed = |b: u32| controllable && inp.pressed & b != 0;
            let held = |b: u32| controllable && inp.down & b != 0;
            // hold SELECT to leave
            if held(PAD_SELECT) {
                self.guests[i].leave_t += dt;
                if self.guests[i].leave_t > 1.5 {
                    leave.push(g.slot);
                    continue;
                }
            } else {
                self.guests[i].leave_t = 0.0;
            }
            let pos = self.w.body.get(g.e).unwrap().pos;
            // stragglers are pulled back to the group
            if (pos - host).length() > 24.0 || (pos.y - host.y).abs() > 6.0 {
                if let Some(b) = self.w.body.get_mut(g.e) {
                    let side = Vec3::new(-1.2 + g.slot as f32 * 0.8, 0.0, -1.0);
                    b.pos = host + side;
                    b.prev = b.pos;
                }
                self.parts.burst(Fx::Dust, host, Vec3::Y, 10, 1.0);
            }

            // ---- down: a teammate standing close revives; after a while they get back up anyway
            if g.down() {
                self.guests[i].down_t += dt;
                if let Some(a) = self.w.actor.get_mut(g.e) {
                    a.dead_t = a.dead_t.min(1.0);
                }
                let helper = self.party().into_iter().filter(|p| *p != g.e).any(|p| self.w.body.get(p).map(|b| (b.pos - pos).length() < 1.8).unwrap_or(false));
                if helper {
                    self.guests[i].revive += dt;
                    if self.rng.f32() < 0.2 {
                        self.parts.burst(Fx::Heal, pos, Vec3::Y, 2, 0.6);
                    }
                } else {
                    self.guests[i].revive = (self.guests[i].revive - dt * 0.5).max(0.0);
                }
                if self.guests[i].revive >= 2.2 || self.guests[i].down_t > 25.0 {
                    if self.guests[i].down_t > 25.0 && !helper {
                        self.spawn_guest_body(i);
                        if let Some(a) = self.w.actor.get_mut(self.guests[i].e) {
                            a.hp = a.max_hp * 0.4;
                        }
                        let old = g.e;
                        if self.w.alive(old) {
                            self.w.kill(old);
                        }
                    } else if let Some(a) = self.w.actor.get_mut(g.e) {
                        a.alive = true;
                        a.hp = a.max_hp * 0.4;
                        a.dead_t = 0.0;
                        a.act = Act::new(ActKind::Stagger, 0.6);
                    }
                    self.guests[i].down_t = 0.0;
                    self.guests[i].revive = 0.0;
                    self.parts.burst(Fx::Heal, pos, Vec3::Y, 24, 1.2);
                    self.audio.play_at(Sfx::Heal, pos, 0.6, 1.1);
                    self.msg(&format!("PLAYER {} IS BACK UP", g.slot + 1));
                }
                continue;
            }

            // ---- stamina
            let busy = self.w.actor.get(g.e).map(|a| a.act.busy()).unwrap_or(false);
            {
                let gs = &mut self.guests[i];
                gs.stam_delay -= dt;
                if gs.stam_delay <= 0.0 && !gs.blocking {
                    gs.stamina = (gs.stamina + 38.0 * dt).min(100.0);
                }
                gs.combo_t -= dt;
                if gs.combo_t <= 0.0 {
                    gs.combo = 0;
                }
                gs.blocking = held(PAD_BLOCK) && !busy && gs.stamina > 0.0;
                if held(PAD_DODGE) {
                    gs.dodge_held += dt;
                }
            }
            let mv = if controllable { inp.l } else { Vec2::ZERO };
            let wish = fwd * mv.y + right * mv.x;
            let act = self.w.actor.get(g.e).unwrap().act.clone();
            let free = !busy || (act.kind == ActKind::Attack && act.phase() >= 2.4);
            // soft aim: swing at the nearest threat in front, or along the stick
            let aim = self.guest_aim(g.e, wish);
            if free {
                let tap_dodge = self.guests[i].dodge_held > 0.0 && !held(PAD_DODGE) && self.guests[i].dodge_held < 0.25;
                if tap_dodge && self.guests[i].stamina >= 18.0 {
                    self.guests[i].stamina -= 18.0;
                    self.guests[i].stam_delay = 0.6;
                    let dir = if wish.length() > 0.1 { wish.normalize() } else { -yaw_dir(self.w.body.get(g.e).unwrap().yaw) };
                    let mut a = Act::new(ActKind::Dodge, 0.5);
                    a.dir = dir;
                    self.w.actor.get_mut(g.e).unwrap().act = a;
                    self.audio.play_at(Sfx::Dodge, pos, 0.4, 1.0);
                } else if pressed(PAD_LIGHT) || pressed(PAD_HEAVY) {
                    let heavy = pressed(PAD_HEAVY);
                    let def = self.t.class(g.class).clone();
                    let mv = if heavy { def.heavy } else { def.light[self.guests[i].combo as usize % def.light.len()] };
                    if self.guests[i].stamina >= mv.stam * 0.5 {
                        self.guests[i].stamina -= mv.stam;
                        self.guests[i].stam_delay = 0.6;
                        let a = Act::attack(mv, false, heavy, self.guests[i].combo, 1.0);
                        self.w.actor.get_mut(g.e).unwrap().act = a;
                        self.guests[i].combo = if heavy { 0 } else { (self.guests[i].combo + 1) % def.light.len() as u8 };
                        self.guests[i].combo_t = mv.total() + 0.45;
                        if let (Some(d), Some(b)) = (aim, self.w.body.get_mut(g.e)) {
                            b.yaw = dir_yaw(d);
                        }
                        self.audio.play_at(if heavy { Sfx::SwingHeavy } else { Sfx::Swing }, pos, 0.4, 1.05);
                    }
                } else if pressed(PAD_HEAL) && !busy && self.guests[i].heals > 0 {
                    self.guests[i].heals -= 1;
                    if let Some(a) = self.w.actor.get_mut(g.e) {
                        a.hp = (a.hp + a.max_hp * 0.45).min(a.max_hp);
                        a.act = Act::new(ActKind::Heal, 0.8);
                    }
                    self.parts.burst(Fx::Heal, pos, Vec3::Y, 20, 1.0);
                    self.audio.play_at(Sfx::Heal, pos, 0.5, 1.0);
                }
            }
            if !held(PAD_DODGE) {
                self.guests[i].dodge_held = 0.0;
            }

            // ---- locomotion (attacks and dodges carry their own motion)
            let act = self.w.actor.get(g.e).unwrap().act.clone();
            let blocking = self.guests[i].blocking;
            let body = self.w.body.get_mut(g.e).unwrap();
            let speed = 4.2 * if blocking { 0.45 } else { 1.0 };
            let (vel, face) = match act.kind {
                ActKind::Idle => (wish.clamp_length_max(1.0) * speed, if wish.length() > 0.1 { Some(wish) } else { None }),
                ActKind::Attack => {
                    let mv = act.mv.unwrap();
                    let ph = act.phase();
                    let v = if (0.7..1.6).contains(&ph) { yaw_dir(body.yaw) * mv.lunge / (mv.act + mv.wind * 0.3).max(0.1) * 0.5 } else { Vec3::ZERO };
                    (v, if ph < 1.0 { aim } else { None })
                }
                ActKind::Dodge => {
                    let k = 1.0 - act.t / act.dur;
                    (act.dir * 8.0 * (0.35 + k), Some(act.dir))
                }
                ActKind::Heal => (wish.clamp_length_max(1.0) * speed * 0.35, None),
                _ => (Vec3::ZERO, None),
            };
            body.vel = vel;
            if let Some(f) = face {
                body.yaw = turn_toward(body.yaw, dir_yaw(f), dt * if act.kind == ActKind::Idle { 14.0 } else { 9.0 });
            }
        }
        for s in leave {
            self.remove_guest(s);
        }
        // presses are consumed by the first step that sees them
        for p in self.guest_input.values_mut() {
            p.pressed = 0;
        }
    }

    /// Where a guest's swing should go: the nearest hostile in front within reach, else the stick.
    fn guest_aim(&self, e: Entity, wish: Vec3) -> Option<Vec3> {
        let b = self.w.body.get(e)?;
        let fwd = if wish.length() > 0.1 { wish.normalize() } else { yaw_dir(b.yaw) };
        let mut best: Option<(f32, Vec3)> = None;
        self.hash.query(b.pos, 3.5, |id, p, _| {
            let o = self.w.ent(id);
            if o == e || !self.can_hurt(Some(e), Team::Player, o) {
                return;
            }
            let d = flat(p - b.pos);
            let dist = d.length();
            if dist < 0.1 || d.normalize().dot(fwd) < 0.2 {
                return;
            }
            if best.map(|x| dist < x.0).unwrap_or(true) {
                best = Some((dist, d.normalize()));
            }
        });
        best.map(|x| x.1).or(if wish.length() > 0.1 { Some(wish.normalize()) } else { None })
    }

    /// Where the shared camera looks and how far it pulls back to keep the group in view.
    pub fn party_frame(&self, host: Vec3) -> (Vec3, f32) {
        let pts: Vec<Vec3> = self.guests.iter().filter_map(|g| self.w.body.get(g.e).map(|b| b.prev.lerp(b.pos, self.alpha))).collect();
        if pts.is_empty() {
            return (host, 0.0);
        }
        let mut c = host;
        for p in &pts {
            c += *p;
        }
        c /= (pts.len() + 1) as f32;
        let spread = pts.iter().map(|p| (*p - c).length()).fold((host - c).length(), f32::max);
        (c, (spread * 0.9 - 1.0).clamp(0.0, 9.0))
    }
}
