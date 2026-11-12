//! Magic: the virus acting through the player. No mana — every cast adds Strain; higher Strain
//! makes spells stronger. Cinder burns the caster a little. Three schools, six spells each.
use super::combat::DmgKind;
use super::comp::*;
use super::Game;
use crate::audio::Sfx;
use crate::data::{Effect, School, SpellKind};
use crate::ecs::Entity;
use crate::particles::Fx;
use crate::physics::Statics;
use crate::util::{chance, dir_yaw, flat, rf, yaw_dir};
use glam::Vec3;

impl Game {
    /// Spell power multiplier for the player right now.
    pub fn spell_mult(&self, school: School) -> f32 {
        let st = &self.pl.stats;
        let f = if st.has(|e| matches!(e, Effect::AlwaysFullPower)) { 1.0 } else { self.pl.strain_frac() };
        let mut m = st.spell_power * (1.0 + f * 0.9);
        if self.pl.catalyst_school(&self.t).contains(&school) {
            m *= 1.3;
        }
        m * (1.0 + (self.pl.life.level as f32 - 1.0) * 0.04)
    }

    pub fn cast_spell(&mut self, caster: Entity, si: usize, pay: bool) {
        let d = self.t.spells[si].clone();
        if pay && caster == self.player {
            let st = self.pl.stats.clone();
            let free_rule = matches!(self.area.kind, crate::world::AreaKind::Rift(i) if crate::world::rift::RULES[i].free_cinder) && d.school == School::Cinder;
            let free = free_rule || chance(&mut self.rng, st.sum(|e| if let Effect::SpellFree(x) = e { Some(*x) } else { None }));
            let cost = d.strain * st.strain_cost;
            if !free {
                if st.has(|e| matches!(e, Effect::SpellBlood)) {
                    let a = self.w.actor.get_mut(caster).unwrap();
                    a.hp = (a.hp - cost * 0.6).max(1.0);
                    let p = self.ppos();
                    self.parts.burst(Fx::Blood, p + Vec3::Y, Vec3::Y, 10, 1.0);
                } else {
                    self.pl.strain += cost;
                }
            }
            if d.school == School::Cinder {
                let fh = st.has(|e| matches!(e, Effect::FireHeart));
                let a = self.w.actor.get_mut(caster).unwrap();
                if fh {
                    a.hp = (a.hp + a.max_hp * 0.02).min(a.max_hp);
                } else {
                    a.hp = (a.hp - a.max_hp * 0.035 * (1.0 - st.fire_res)).max(1.0);
                    a.flash = 0.4;
                }
            }
            let echo = st.sum(|e| if let Effect::SpellEcho(x) = e { Some(*x) } else { None });
            if echo > 0.0 && chance(&mut self.rng, echo) {
                self.echo_queue.push((si, 0.35));
            }
        }
        self.spell_effect(caster, si);
    }

    pub fn cast_spell_free(&mut self, caster: Entity, si: usize) {
        self.spell_effect(caster, si);
    }

    /// Aim point/direction for the player: lock target, else facing.
    fn aim(&self, caster: Entity, range: f32) -> (Vec3, Option<Entity>) {
        let b = *self.w.body.get(caster).unwrap();
        if caster == self.player {
            if let Some(l) = self.pl.lock {
                if let Some(lb) = self.w.body.get(l) {
                    return (lb.pos, Some(l));
                }
            }
            // soft aim: nearest hostile in a forward cone
            let fwd = yaw_dir(b.yaw);
            let mut best: Option<(Entity, f32, Vec3)> = None;
            self.hash.query(b.pos, range, |id, p, _| {
                let e = self.w.ent(id);
                if let Some(a) = self.w.actor.get(e) {
                    if a.alive && a.team == Team::Infected && !self.w.foe.get(e).map(|f| f.mimic).unwrap_or(false) {
                        let d = flat(p - b.pos);
                        let dist = d.length();
                        if dist > 0.1 && d.dot(fwd) / dist > 0.6 {
                            let score = dist * (2.0 - d.dot(fwd) / dist);
                            if best.map(|x| score < x.1).unwrap_or(true) {
                                best = Some((e, score, p));
                            }
                        }
                    }
                }
            });
            if let Some((e, _, p)) = best {
                return (p, Some(e));
            }
            return (b.pos + fwd * range.min(8.0), None);
        }
        let t = self.w.ai.get(caster).and_then(|a| a.target);
        (t.and_then(|t| self.w.body.get(t)).map(|tb| tb.pos).unwrap_or(b.pos + yaw_dir(b.yaw) * 6.0), t)
    }

    fn spell_effect(&mut self, caster: Entity, si: usize) {
        let d = self.t.spells[si].clone();
        let is_player = caster == self.player;
        let m = if is_player { self.spell_mult(d.school) } else { 1.0 };
        let team = self.w.actor.get(caster).map(|a| a.team).unwrap_or(Team::Player);
        let b = *self.w.body.get(caster).unwrap();
        let fwd = yaw_dir(b.yaw);
        let hand = self.w.rig.get(caster).map(|r| r.hand_l).unwrap_or(b.center());
        let fx = match d.school {
            School::Flesh => Fx::Ichor,
            School::Rift => Fx::Rift,
            School::Cinder => Fx::Fire,
        };
        self.parts.burst(fx, hand, fwd, 12, 1.0);
        self.noise(b.pos, 14.0);
        match d.kind {
            SpellKind::Bolt(speed, dmg, blast, count, spread, homing) => {
                let (tp, tgt) = self.aim(caster, 25.0);
                let origin = hand + fwd * 0.3;
                let base = (tp + Vec3::Y * 1.0 - origin).normalize_or_zero();
                for i in 0..count {
                    let k = if count > 1 { i as f32 / (count - 1) as f32 - 0.5 } else { 0.0 };
                    let a = (k * spread).to_radians();
                    let dir = Vec3::new(base.x * a.cos() - base.z * a.sin(), base.y + rf(&mut self.rng, -0.05, 0.1), base.x * a.sin() + base.z * a.cos()).normalize_or_zero();
                    let kind = if d.school == School::Cinder { ProjKind::Ember } else { ProjKind::Shard };
                    let p = self.spawn_proj(origin, dir * speed, kind, dmg * m, 12.0, team, Some(caster), blast);
                    if homing {
                        if let Some(pr) = self.w.proj.get_mut(p) {
                            pr.homing = tgt;
                        }
                    }
                }
                self.audio.play_at(if d.school == School::Cinder { Sfx::Fire } else { Sfx::Rift }, b.pos, 0.6, 1.6);
            }
            SpellKind::Spikes(dmg, len) => {
                let (tp, _) = self.aim(caster, len);
                let dir = flat(tp - b.pos).normalize_or_zero();
                let dir = if dir.length_squared() < 0.1 { fwd } else { dir };
                let mut z = self.make_zone(b.pos, ZoneKind::Spikes, 1.0, dmg * m, 1.0, team, Some(caster));
                z.dir = dir;
                z.len = len;
                self.add_zone(b.pos, z);
            }
            SpellKind::Grab(range, dmg) => {
                let (_, tgt) = self.aim(caster, range);
                if let Some(t) = tgt {
                    let tb = *self.w.body.get(t).unwrap();
                    let boss = self.w.foe.get(t).map(|f| f.boss.is_some() || f.apex.is_some()).unwrap_or(false);
                    self.tethers_fx.push((hand, tb.center(), 0.4, [1.4, 0.3, 0.5]));
                    if boss {
                        if let Some(cb) = self.w.body.get_mut(caster) {
                            cb.push += flat(tb.pos - b.pos).normalize_or_zero() * 14.0;
                        }
                    } else {
                        let dest = b.pos + fwd * 1.6;
                        let (np, _) = self.area.move_cyl(tb.pos, dest - tb.pos, tb.radius, tb.height);
                        if let Some(tbm) = self.w.body.get_mut(t) {
                            tbm.pos = np;
                        }
                        if let Some(ta) = self.w.actor.get_mut(t) {
                            ta.act = Act::new(ActKind::Stagger, 1.0);
                        }
                    }
                    self.deal(t, Some(caster), dmg * m, 20.0, -fwd, DmgKind::Spell, 0.0);
                    self.audio.play_at(Sfx::Spit, tb.pos, 0.7, 0.7);
                }
            }
            SpellKind::Cloud(r, dps, dur) => {
                let (tp, _) = self.aim(caster, 8.0);
                let p = b.pos + flat(tp - b.pos).clamp_length_max(6.0);
                self.spawn_zone(p, ZoneKind::Spores, r, dps * m, dur, team, Some(caster));
                self.parts.burst(Fx::Spore, p + Vec3::Y, Vec3::ZERO, 50, r * 0.6);
                self.audio.play_at(Sfx::Spit, p, 0.6, 0.5);
            }
            SpellKind::Drain(range, dmg, frac) => {
                let (_, tgt) = self.aim(caster, range);
                if let Some(t) = tgt {
                    let tc = self.w.body.get(t).unwrap().center();
                    let dealt = self.deal(t, Some(caster), dmg * m, 10.0, fwd, DmgKind::Spell, 0.0);
                    let a = self.w.actor.get_mut(caster).unwrap();
                    a.hp = (a.hp + dealt * frac).min(a.max_hp);
                    self.tethers_fx.push((tc, hand, 0.6, [2.5, 0.2, 0.3]));
                    for k in 0..12 {
                        let q = tc.lerp(hand, k as f32 / 12.0);
                        self.parts.burst(Fx::Blood, q, (hand - tc).normalize_or_zero(), 1, 1.0);
                    }
                    self.audio.play_at(Sfx::Heal, b.pos, 0.6, 0.6);
                }
            }
            SpellKind::Brood(n, ttl) => {
                for _ in 0..n {
                    let p = b.pos + fwd * 1.0 + crate::util::rand_dir(&mut self.rng) * 0.8;
                    self.spawn_brood(p, ttl);
                }
                self.audio.play_at(Sfx::Larva, b.pos, 0.8, 1.2);
            }
            SpellKind::Carapace(armor, dur) => {
                let a = self.w.actor.get_mut(caster).unwrap();
                a.armor_buff = armor * m.sqrt();
                a.armor_buff_t = dur;
                a.poise = a.max_poise * 2.0;
                self.parts.burst(Fx::Bone, b.center(), Vec3::Y, 30, 1.0);
                self.audio.play_at(Sfx::Block, b.pos, 0.8, 0.6);
            }
            SpellKind::Blink(dist) => {
                let dir = if is_player && self.input_snapshot.mv.length() > 0.2 {
                    let cf = yaw_dir(self.cam.yaw);
                    let cr = Vec3::new(-cf.z, 0.0, cf.x);
                    (cf * self.input_snapshot.mv.y + cr * self.input_snapshot.mv.x).normalize_or_zero()
                } else {
                    fwd
                };
                let (np, _) = self.area.move_cyl(b.pos, dir * dist * m.sqrt().min(1.5), b.radius, b.height);
                self.parts.burst(Fx::Rift, b.center(), Vec3::ZERO, 30, 1.0);
                if let Some(bm) = self.w.body.get_mut(caster) {
                    bm.pos = np;
                    bm.prev = np;
                }
                self.parts.burst(Fx::Rift, np + Vec3::Y, Vec3::ZERO, 30, 1.0);
                self.distorts_temp.push((np + Vec3::Y, 2.0, 1.0, 1.0));
                self.audio.play_at(Sfx::Rift, np, 0.7, 1.5);
            }
            SpellKind::Well(r, force, dps, dur) => {
                let (tp, _) = self.aim(caster, 12.0);
                let p = b.pos + flat(tp - b.pos).clamp_length_max(10.0);
                let mut z = self.make_zone(p, ZoneKind::Well, r, dps * m, dur, team, Some(caster));
                z.len = force;
                self.add_zone(p, z);
                self.audio.play_at(Sfx::Rift, p, 0.8, 0.6);
            }
            SpellKind::Tear(dps, len, dur) => {
                let (tp, _) = self.aim(caster, 7.0);
                let c = b.pos + flat(tp - b.pos).clamp_length_max(5.0);
                let dir = Vec3::new(-fwd.z, 0.0, fwd.x);
                let mut z = self.make_zone(c, ZoneKind::Tear, 1.0, dps * m, dur, team, Some(caster));
                z.dir = dir;
                z.len = len;
                self.add_zone(c, z);
                self.audio.play_at(Sfx::Rift, c, 0.9, 1.0);
            }
            SpellKind::Stasis(r, dur) => {
                self.spawn_zone(b.pos, ZoneKind::Stasis, r * m.sqrt().min(1.4), 0.0, dur, team, Some(caster));
                self.audio.play_at(Sfx::Rift, b.pos, 0.8, 0.4);
            }
            SpellKind::Transpose(range, dmg) => {
                let (_, tgt) = self.aim(caster, range);
                if let Some(t) = tgt {
                    let boss = self.w.foe.get(t).map(|f| f.boss.is_some()).unwrap_or(false);
                    if !boss {
                        let tp = self.w.body.get(t).unwrap().pos;
                        self.parts.burst(Fx::Rift, b.center(), Vec3::ZERO, 20, 1.0);
                        self.parts.burst(Fx::Rift, tp + Vec3::Y, Vec3::ZERO, 20, 1.0);
                        if let Some(bm) = self.w.body.get_mut(caster) {
                            bm.pos = tp;
                            bm.prev = tp;
                            bm.yaw = dir_yaw(flat(b.pos - tp));
                        }
                        if let Some(tm) = self.w.body.get_mut(t) {
                            tm.pos = b.pos;
                            tm.prev = b.pos;
                        }
                        if let Some(ta) = self.w.actor.get_mut(t) {
                            ta.act = Act::new(ActKind::Stagger, 1.2);
                        }
                    }
                    self.deal(t, Some(caster), dmg * m, 30.0, fwd, DmgKind::Spell, 0.0);
                    self.audio.play_at(Sfx::Rift, b.pos, 0.8, 1.8);
                }
            }
            SpellKind::Nova(r, dmg) => {
                self.aoe(b.pos, r, dmg * m, 50.0, team, Some(caster), DmgKind::Fire);
                self.spawn_zone(b.pos, ZoneKind::Ring, r, 0.0, 0.45, team, Some(caster));
                self.parts.burst(Fx::Fire, b.pos + Vec3::Y * 0.5, Vec3::ZERO, 70, r * 0.5);
                self.parts.burst(Fx::Flash, b.center(), Vec3::ZERO, 1, r * 0.4);
                self.lights_temp.push((b.center(), Vec3::new(6.0, 2.5, 0.6), r * 3.0, 0.3));
                self.audio.play_at(Sfx::Explode, b.pos, 0.9, 1.1);
                self.shake(0.4);
            }
            SpellKind::Wall(len, dps, dur) => {
                let c = b.pos + fwd * 3.0;
                let side = Vec3::new(-fwd.z, 0.0, fwd.x);
                let n = (len / 1.6).ceil() as i32;
                for i in 0..n {
                    let k = i as f32 / (n - 1).max(1) as f32 - 0.5;
                    let p = c + side * k * len;
                    if !self.area.blocked(p, 0.3) {
                        self.spawn_zone(p, ZoneKind::Fire, 1.0, dps * m, dur, team, Some(caster));
                    }
                }
                self.audio.play_at(Sfx::Fire, c, 0.9, 1.0);
            }
            SpellKind::Kindle(dps, dur) => {
                self.pl.kindle_dps = dps * m;
                let a = self.w.actor.get_mut(caster).unwrap();
                a.kindle = dur;
                self.parts.burst(Fx::Fire, hand, Vec3::Y, 30, 1.0);
                self.audio.play_at(Sfx::Fire, b.pos, 0.7, 1.4);
            }
            SpellKind::Meteor(r, dmg, delay) => {
                let (tp, _) = self.aim(caster, 18.0);
                let mut z = self.make_zone(tp, ZoneKind::Blast, r, dmg * m, delay + 0.2, team, Some(caster));
                z.delay = delay;
                self.add_zone(tp, z);
                self.audio.play_at(Sfx::Fire, tp, 0.8, 0.5);
            }
            SpellKind::Breath(range, dps, dur) => {
                // instantaneous cone plus lingering flames along it
                let mut hits = Vec::new();
                self.hash.query(b.pos, range, |id, p, _| hits.push((id, p)));
                for (id, p) in hits {
                    let e = self.w.ent(id);
                    let dd = flat(p - b.pos);
                    let l = dd.length();
                    if l > 0.1 && dd.dot(fwd) / l > 0.75 && self.w.actor.get(e).map(|a| a.team == Team::Infected && a.alive).unwrap_or(false) {
                        self.deal(e, Some(caster), dps * m * dur * 0.6, 20.0, dd / l, DmgKind::Fire, 2.0);
                        if let Some(a) = self.w.actor.get_mut(e) {
                            a.burn = 3.0;
                            a.burn_dps = a.burn_dps.max(dps * m * 0.3);
                        }
                    }
                }
                for k in 1..4 {
                    let p = b.pos + fwd * (k as f32 * range / 4.0);
                    self.spawn_zone(p, ZoneKind::Fire, 0.6 + k as f32 * 0.4, dps * m * 0.4, dur, team, Some(caster));
                }
                for _ in 0..50 {
                    let v = (fwd + Vec3::new(rf(&mut self.rng, -0.4, 0.4), rf(&mut self.rng, -0.1, 0.25), rf(&mut self.rng, -0.4, 0.4))).normalize_or_zero() * rf(&mut self.rng, 6.0, 12.0);
                    self.parts.burst(Fx::Fire, hand, v, 1, 1.0);
                }
                self.audio.play_at(Sfx::Fire, b.pos, 1.0, 0.8);
            }
        }
    }

    pub fn echo_tick(&mut self, dt: f32) {
        let mut fire = Vec::new();
        for (s, t) in &mut self.echo_queue {
            *t -= dt;
            if *t <= 0.0 {
                fire.push(*s);
            }
        }
        self.echo_queue.retain(|(_, t)| *t > 0.0);
        for s in fire {
            let pe = self.player;
            if self.w.actor.get(pe).map(|a| a.alive).unwrap_or(false) {
                self.float_text(self.ppos() + Vec3::Y * 2.2, "ECHO", [0.8, 0.6, 1.0]);
                self.spell_effect(pe, s);
            }
        }
    }
}
