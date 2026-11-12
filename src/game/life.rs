//! The living world: day-night cycle, weather (rain, fog, spore storms, lightning), helicopters,
//! distant sirens and gunfire, radio chatter, world emitters, and the per-frame environment and
//! light selection handed to the renderer.
use super::Game;
use crate::audio::{MusicState, Sfx};
use crate::particles::{Fx, P};
use crate::render::{Env, Light};
use crate::util::{chance, rand_dir, rf, smooth};
use crate::world::{district_at, AreaKind, DState, EmitKind, DISTRICTS};
use glam::Vec3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Weather {
    Clear,
    Rain,
    Fog,
    SporeStorm,
}

#[derive(Clone, Debug)]
pub struct Heli {
    pub pos: Vec3,
    pub vel: Vec3,
    pub t: f32,
    pub sound_t: f32,
}

impl Game {
    pub fn life_tick(&mut self, dt: f32) {
        // day-night: a full day lasts 16 minutes
        self.tod = (self.tod + dt / 960.0) % 1.0;
        // weather
        self.weather_t -= dt;
        let lost_here = matches!(self.area.kind, AreaKind::City) && district_at(self.ppos()).map(|d| DISTRICTS[d].state == DState::Lost).unwrap_or(false);
        if self.weather_t <= 0.0 {
            self.weather_t = rf(&mut self.rng, 90.0, 220.0);
            let x = self.rng.f32();
            self.weather = if lost_here && x < 0.4 {
                Weather::SporeStorm
            } else if x < 0.4 {
                Weather::Rain
            } else if x < 0.55 {
                Weather::Fog
            } else if x < 0.62 {
                Weather::SporeStorm
            } else {
                Weather::Clear
            };
        }
        let target_rain = if self.weather == Weather::Rain { 1.0 } else { 0.0 };
        self.rain += (target_rain - self.rain) * (dt * 0.2).min(1.0);
        let target_spore = if self.weather == Weather::SporeStorm { 1.0 } else { 0.0 };
        self.spore += (target_spore - self.spore) * (dt * 0.2).min(1.0);
        let target_fog = if self.weather == Weather::Fog { 1.0 } else { 0.0 };
        self.fogw += (target_fog - self.fogw) * (dt * 0.15).min(1.0);
        self.wet = (self.wet + (self.rain - 0.3) * dt * 0.05).clamp(0.25, 1.0);
        self.lightning = (self.lightning - dt * 4.0).max(0.0);
        if self.area.kind == AreaKind::City && self.rain > 0.7 && chance(&mut self.rng, dt * 0.05) {
            self.lightning = 1.0;
            self.thunder_t = rf(&mut self.rng, 0.5, 2.5);
        }
        if self.thunder_t > 0.0 {
            self.thunder_t -= dt;
            if self.thunder_t <= 0.0 {
                self.audio.play(Sfx::Thunder, 0.6, rf(&mut self.rng, 0.7, 1.1));
            }
        }
        let cam = self.cam.eye;
        // rain streaks and storm spores around the camera
        let rain_n = (self.rain * 260.0 * dt * self.parts.density) as u32;
        for _ in 0..rain_n {
            let p = cam + Vec3::new(rf(&mut self.rng, -14.0, 14.0), rf(&mut self.rng, 4.0, 12.0), rf(&mut self.rng, -14.0, 14.0));
            self.parts.push(P { pos: p, vel: Vec3::new(0.6, -16.0, 0.3), life: 0.0, max: 0.8, size: 0.012, size_end: 0.012, col: [0.55, 0.6, 0.7, 0.35], col_end: [0.55, 0.6, 0.7, 0.2], kind: 3.0, grav: 0.0, drag: 0.0, add: 0.0, stretch: 18.0, rot: 0.0, floor: false });
        }
        let spore_n = ((self.spore * 50.0 + if lost_here { 6.0 } else { 1.5 }) * dt * self.parts.density) as u32 + chance(&mut self.rng, 0.2) as u32;
        for _ in 0..spore_n {
            let p = cam + Vec3::new(rf(&mut self.rng, -16.0, 16.0), rf(&mut self.rng, 0.0, 8.0), rf(&mut self.rng, -16.0, 16.0));
            self.parts.burst(Fx::Spore, p, Vec3::new(0.4, 0.0, 0.2) * (1.0 + self.spore * 3.0), 1, 0.2);
        }
        if self.area.kind == AreaKind::City {
            self.city_ambience(dt);
        }
        // world emitters near the player
        let pp = self.ppos();
        let mut ems = Vec::new();
        for c in self.area.chunks.values() {
            for (p, k) in &c.emitters {
                if (*p - pp).length() < 70.0 {
                    ems.push((*p, *k));
                }
            }
        }
        for (p, k) in ems {
            let r = &mut self.rng;
            match k {
                EmitKind::Smoke if chance(r, dt * 6.0) => self.parts.burst(Fx::Smoke, p, Vec3::new(0.5, 0.0, 0.2), 1, 1.0),
                EmitKind::Fire if chance(r, dt * 30.0) => self.parts.burst(Fx::Fire, p, Vec3::ZERO, 1, 1.0),
                EmitKind::Embers if chance(r, dt * 8.0) => self.parts.burst(Fx::Ember, p, Vec3::ZERO, 1, 1.0),
                EmitKind::Spores if chance(r, dt * 6.0) => self.parts.burst(Fx::Spore, p, Vec3::Y * 0.2, 1, 1.0),
                EmitKind::Steam if chance(r, dt * 4.0) => self.parts.burst(Fx::Dust, p, Vec3::Y * 1.5, 1, 1.0),
                EmitKind::Flare if chance(r, dt * 20.0) => {
                    self.parts.burst(Fx::Ember, p, Vec3::Y * 0.5, 1, 0.3);
                    if chance(&mut self.rng, 0.3) {
                        self.parts.burst(Fx::Smoke, p, Vec3::ZERO, 1, 0.4);
                    }
                }
                EmitKind::Rift if chance(r, dt * 12.0) => self.parts.burst(Fx::Rift, p, Vec3::ZERO, 1, 2.0),
                EmitKind::Drip if chance(r, dt * 5.0) => self.parts.burst(Fx::Ichor, p + Vec3::Y * 8.0, Vec3::ZERO, 1, 0.1),
                _ => {}
            }
        }
    }

    fn city_ambience(&mut self, dt: f32) {
        let pp = self.ppos();
        let state = district_at(pp).map(|d| DISTRICTS[d].state);
        // helicopters cross the city
        self.heli_t -= dt;
        if self.heli_t <= 0.0 && self.helis.len() < 2 {
            self.heli_t = rf(&mut self.rng, 40.0, 90.0);
            let a = rf(&mut self.rng, 0.0, std::f32::consts::TAU);
            let d = Vec3::new(a.cos(), 0.0, a.sin());
            let start = pp - d * 220.0 + Vec3::new(-d.z, 0.0, d.x) * rf(&mut self.rng, -60.0, 60.0) + Vec3::Y * rf(&mut self.rng, 35.0, 50.0);
            self.helis.push(Heli { pos: start, vel: d * 24.0, t: 0.0, sound_t: 0.0 });
        }
        for h in &mut self.helis {
            h.t += dt;
            h.pos += h.vel * dt;
            h.sound_t -= dt;
            if h.sound_t <= 0.0 {
                h.sound_t = 2.2;
                self.audio.play_at(Sfx::Heli, h.pos, 2.5, 1.0);
            }
        }
        self.helis.retain(|h| h.t < 20.0);
        // distant war
        self.amb_t -= dt;
        if self.amb_t <= 0.0 {
            self.amb_t = rf(&mut self.rng, 4.0, 12.0);
            let far = pp + rand_dir(&mut self.rng) * rf(&mut self.rng, 60.0, 120.0);
            match state {
                Some(DState::Contested) | Some(DState::Held) => {
                    if chance(&mut self.rng, 0.6) {
                        let n = self.rng.u32(1..5);
                        for k in 0..n {
                            let _ = k;
                            self.audio.play_at(Sfx::Gunshot, far, 3.0, rf(&mut self.rng, 0.6, 0.9));
                        }
                    } else {
                        self.audio.play_at(Sfx::Siren, far, 4.0, rf(&mut self.rng, 0.9, 1.1));
                    }
                }
                _ => self.audio.play_at(Sfx::Shriek, far, 2.0, rf(&mut self.rng, 0.4, 0.7)),
            }
        }
        // radio chatter near soldiers
        self.radio_t -= dt;
        if self.radio_t <= 0.0 {
            self.radio_t = rf(&mut self.rng, 20.0, 45.0);
            let mut near = false;
            self.hash.query(pp, 18.0, |id, _, _| {
                let e = self.w.ent(id);
                if self.w.actor.get(e).map(|a| a.team == crate::game::comp::Team::Military && a.alive).unwrap_or(false) {
                    near = true;
                }
            });
            if near {
                let l = self.t.text.radio[self.rng.usize(0..self.t.text.radio.len())].clone();
                self.audio.play(Sfx::Radio, 0.35, 1.0);
                self.subtitle(&format!("RADIO: {l}"), 4.0);
            }
        }
    }

    /// Environment parameters for this frame.
    pub fn build_env(&self, strain_vis: f32) -> Env {
        let mut e = Env::default();
        match self.area.kind {
            AreaKind::City => {
                let pp = self.ppos();
                // blend district palettes by position
                let d = district_at(pp).unwrap_or(0);
                let dd = &DISTRICTS[d];
                let a = (self.tod - 0.25) * std::f32::consts::TAU;
                let elev = a.sin();
                let day = smooth(elev * 3.0 + 0.3);
                e.sun_dir = Vec3::new(a.cos() * 0.6, elev.abs().max(0.25), 0.45).normalize();
                if elev < 0.0 {
                    // moon
                    e.sun_dir = Vec3::new(-a.cos() * 0.5, (-elev).max(0.35), -0.4).normalize();
                }
                let sun_c = Vec3::new(1.15, 0.95, 0.75) * 1.25;
                let moon_c = Vec3::new(0.18, 0.24, 0.42);
                let dusk = (1.0 - (elev * 4.0).abs()).max(0.0);
                e.sun_col = (moon_c.lerp(sun_c, day) + Vec3::new(0.6, 0.2, -0.1) * dusk * 0.5) * (1.0 - self.rain * 0.6 - self.fogw * 0.3);
                let fog = Vec3::from(dd.fog);
                e.ambient = (Vec3::new(0.06, 0.07, 0.1).lerp(Vec3::new(0.32, 0.32, 0.34), day) + fog * 0.6) * (1.0 - self.rain * 0.2);
                e.fog_col = fog * (0.55 + day * 0.75) + Vec3::new(0.25, 0.1, 0.0) * dusk * 0.4 + Vec3::new(0.03, 0.12, 0.06) * self.spore;
                e.fog_density = dd.fog_density * (1.0 + self.rain * 0.5 + self.fogw * 1.8 + self.spore * 1.2);
                e.grade = Vec3::from(dd.grade);
                e.saturation = dd.saturation;
                e.levels = dd.palette_levels;
                e.wetness = self.wet;
                e.warp = if dd.state == DState::Lost { 1.0 + if d == 5 { 0.6 } else { 0.0 } } else { 0.0 };
                e.shaft = 1.2 + day * 0.4;
                e.sun_col += Vec3::splat(self.lightning * 3.0);
                e.ambient += Vec3::new(0.5, 0.55, 0.7) * self.lightning;
            }
            AreaKind::Rift(i) => {
                let r = &crate::world::rift::RULES[i];
                e.sun_dir = Vec3::new(0.2, 0.9, 0.3).normalize();
                e.sun_col = Vec3::from(r.sun);
                e.ambient = Vec3::from(r.fog) * 1.2 + Vec3::splat(0.04);
                e.fog_col = Vec3::from(r.fog);
                e.fog_density = r.fog_density;
                e.fog_height = 0.02;
                e.grade = Vec3::from(r.grade);
                e.saturation = 1.0;
                e.levels = r.levels;
                e.wetness = if i == 0 { 1.0 } else { 0.3 };
                e.warp = r.warp;
                e.shaft = 0.8;
                e.light_scatter = 1.6;
            }
        }
        e.strain = strain_vis;
        e.vignette = 0.45 + self.hurt_flash * 0.6;
        e.flash = self.flash.min(1.0) * 0.6;
        e.fade = self.fade;
        if self.hurt_flash > 0.0 {
            e.grade *= Vec3::new(1.0 + self.hurt_flash * 0.6, 1.0 - self.hurt_flash * 0.3, 1.0 - self.hurt_flash * 0.3);
        }
        if matches!(self.pl.affliction, Some((crate::game::player::Affliction::Blur, _))) {
            e.strain = (e.strain + 0.6).min(1.0);
        }
        if self.slowmo > 0.0 {
            e.saturation *= 0.6;
        }
        e
    }

    /// Choose the most relevant lights for this frame.
    /// Returns the shadow-casting key light (always placed at index 0).
    pub fn gather_lights(&self, out: &mut Vec<Light>, t: f32) -> Option<(Vec3, f32)> {
        out.clear();
        let cam = self.cam.eye;
        let pp = self.ppos();
        let mut cands: Vec<(f32, Light)> = Vec::new();
        for c in self.area.chunks.values() {
            for l in &c.lights {
                let d = (l.pos - pp).length().min((l.pos - cam).length());
                if d > l.radius + 45.0 {
                    continue;
                }
                let h = (l.pos.x * 13.1 + l.pos.z * 7.7).sin() * 100.0;
                let k = match l.flicker {
                    1 => 0.92 + 0.08 * ((t * 50.0 + h).sin() * (t * 23.0).sin()),
                    2 => 0.75 + 0.25 * ((t * 9.0 + h).sin() * (t * 13.7 + h).cos()).abs(),
                    3 => {
                        let s = ((t * 1.3 + h).sin() * 10.0).sin();
                        if s > 0.6 { 0.05 } else if s > 0.4 { 0.6 } else { 1.0 }
                    }
                    4 => 0.7 + 0.3 * (t * 1.2 + h).sin(),
                    _ => 1.0,
                };
                cands.push((d / l.radius, Light { pos: l.pos, radius: l.radius, color: l.color * k }));
            }
        }
        for (p, c, r, _) in &self.lights_temp {
            cands.push(((*p - pp).length() / r * 0.3, Light { pos: *p, radius: *r, color: *c }));
        }
        for (id, pr) in self.w.proj.iter() {
            let e = self.w.ent(id);
            if let Some(b) = self.w.body.get(e) {
                let col = match pr.kind {
                    crate::game::comp::ProjKind::Ember => Vec3::new(3.0, 1.2, 0.3),
                    crate::game::comp::ProjKind::Acid => Vec3::new(0.6, 1.5, 0.2),
                    crate::game::comp::ProjKind::Bullet => continue,
                    _ => Vec3::new(1.2, 0.7, 2.5),
                };
                cands.push(((b.pos - pp).length() / 6.0 * 0.5, Light { pos: b.pos, radius: 6.0, color: col }));
            }
        }
        for (id, z) in self.w.zone.iter() {
            if matches!(z.kind, crate::game::comp::ZoneKind::Fire) && !z.permanent {
                let e = self.w.ent(id);
                if let Some(b) = self.w.body.get(e) {
                    cands.push(((b.pos - pp).length() / 7.0, Light { pos: b.pos + Vec3::Y, radius: 6.0, color: Vec3::new(2.4, 1.0, 0.25) }));
                }
            }
        }
        for h in &self.helis {
            cands.push(((h.pos - pp).length() / 60.0, Light { pos: Vec3::new(h.pos.x, 8.0, h.pos.z), radius: 22.0, color: Vec3::new(2.2, 2.3, 2.6) }));
        }
        // the player's catalyst and strain glow
        let pf = self.pl.strain_frac();
        if pf > 0.2 || self.pl.spells(&self.t).len() > 0 {
            let c = Vec3::new(0.8, 0.3, 1.2) * (0.3 + pf * 1.4);
            cands.push((0.0, Light { pos: pp + Vec3::Y * 1.3, radius: 4.0 + pf * 3.0, color: c }));
        }
        cands.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        // key light: the closest strong source (fire pits, floodlights, burning wrecks, searchlights)
        let key_i = cands.iter().position(|(_, l)| l.radius >= 12.0 && l.color.max_element() > 1.5 && (l.pos - pp).length() < l.radius * 0.9);
        let mut key = None;
        if let Some(i) = key_i {
            let (_, l) = cands.remove(i);
            key = Some((l.pos, l.radius));
            out.push(l);
        }
        for (_, l) in cands.into_iter().take(32 - out.len()) {
            out.push(l);
        }
        key
    }

    pub fn music_state(&self) -> MusicState {
        let mut combat = 0.0;
        if self.in_combat() {
            combat = 1.0;
        } else if self.pl.last_hit_target.is_some() {
            combat = 0.5;
        }
        let mood = match self.area.kind {
            AreaKind::Rift(_) => 3.0,
            AreaKind::City => match district_at(self.ppos()).map(|d| DISTRICTS[d].state) {
                Some(DState::Held) => 0.0,
                Some(DState::Contested) => 1.0,
                _ => 2.0,
            },
        };
        let pp = self.ppos();
        let space = match self.area.kind {
            AreaKind::Rift(_) => 2.0,
            // a roof overhead means we are inside
            AreaKind::City if crate::physics::Statics::raycast(&self.area, pp + Vec3::Y * 1.7, Vec3::Y, 9.0, crate::physics::SOLID).is_some() => 1.0,
            AreaKind::City => 0.0,
        };
        MusicState {
            space,
            combat,
            strain: self.pl.strain_frac(),
            boss: if self.boss_awake && self.boss.is_some() { 1.0 } else { 0.0 },
            mood,
            rain: self.rain,
            wind: 0.3 + self.spore * 0.7,
            volume: self.settings.master,
            music_volume: self.settings.music,
            muffle: if self.slowmo > 0.0 { 0.6 } else { 0.0 } + if self.screen.pauses() { 0.5 } else { 0.0 },
            fire: {
                let pp = self.ppos();
                let mut best = 0.0f32;
                for c in self.area.chunks.values() {
                    for (p, k) in &c.emitters {
                        if *k == EmitKind::Fire {
                            best = best.max(1.0 - (*p - pp).length() / 14.0);
                        }
                    }
                }
                best.max(0.0)
            },
        }
    }
}
