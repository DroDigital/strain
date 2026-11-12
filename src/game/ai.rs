//! AI for every non-player actor: perception (sight + sound), steering with separation and
//! unsticking, archetype behaviours, affix behaviours, soldiers, civilians, allied larvae,
//! risen husks and the phase-driven boss controller.
use super::combat::{hostile, DmgKind};
use super::comp::*;
use super::Game;
use crate::audio::Sfx;
use crate::data::{AffixKind, AiKind, BossMove, Effect, MoveDef, Swing};
use crate::ecs::Entity;
use crate::particles::Fx;
use crate::physics::Statics;
use crate::util::{chance, dir_yaw, flat, rand_dir, rf, turn_toward, yaw_dir};
use glam::Vec3;

impl Game {
    pub fn ai_tick(&mut self, dt: f32) {
        let ids: Vec<u32> = self.w.ai.ids().collect();
        let pp = self.ppos();
        for id in ids {
            let e = self.w.ent(id);
            let alive = self.w.actor.get(e).map(|a| a.alive).unwrap_or(false);
            if !alive {
                continue;
            }
            // dormant far away: freeze to save time
            let pos = self.w.body.get(e).unwrap().pos;
            if (pos - pp).length() > 95.0 && self.boss != Some(e) {
                if let Some(b) = self.w.body.get_mut(e) {
                    b.vel = Vec3::ZERO;
                }
                continue;
            }
            self.affix_tick(e, dt);
            if !self.w.alive(e) || !self.w.actor.get(e).map(|a| a.alive).unwrap_or(false) {
                continue;
            }
            let kind = self.w.ai.get(e).unwrap().kind;
            match kind {
                AiKind::Soldier => self.ai_soldier(e, dt),
                AiKind::Civilian => self.ai_civilian(e, dt),
                AiKind::Ally => self.ai_ally(e, dt),
                AiKind::Boss => self.ai_boss(e, dt),
                _ => self.ai_infected(e, dt),
            }
        }
    }

    /// Choose a target: visible hostiles, things that hit us, and noises.
    fn perceive(&mut self, e: Entity, range: f32) -> Option<Entity> {
        let body = *self.w.body.get(e).unwrap();
        let team = self.w.actor.get(e).unwrap().team;
        let ai = self.w.ai.get(e).unwrap().clone();
        if let Some(t) = ai.target {
            let ok = self.w.actor.get(t).map(|a| a.alive && a.dissolve < 0.9).unwrap_or(false) && self.w.body.get(t).map(|b| (b.pos - body.pos).length() < range * 1.8).unwrap_or(false);
            if ok && ai.alert > 0.0 {
                return Some(t);
            }
        }
        let mut best: Option<(Entity, f32)> = None;
        let mut cands = Vec::new();
        self.hash.query(body.pos, range, |id, p, _| cands.push((id, p)));
        let fwd = yaw_dir(body.yaw);
        for (id, p) in cands {
            let o = self.w.ent(id);
            if o == e || !self.can_hurt(Some(e), team, o) {
                continue;
            }
            if self.w.foe.get(o).map(|f| f.mimic).unwrap_or(false) {
                continue;
            }
            let d = flat(p - body.pos);
            let dist = d.length();
            // forward cone sees far, behind only close
            let seen_range = if d.normalize_or_zero().dot(fwd) > -0.2 { range } else { range * 0.35 };
            if dist > seen_range {
                continue;
            }
            if !self.area.line_of_sight(body.center(), p + Vec3::Y * 1.2) {
                continue;
            }
            // the player is noticed more readily, unless dread
            let mut score = dist;
            if o == self.player {
                score *= 0.7;
            }
            if best.map(|b| score < b.1).unwrap_or(true) {
                best = Some((o, score));
            }
        }
        if best.is_none() && team == Team::Infected {
            // investigate noises
            for (np, r) in &self.noises {
                if (body.pos - *np).length() < *r {
                    if let Some(ai) = self.w.ai.get_mut(e) {
                        ai.goal = *np;
                        ai.state = AiState::Alert;
                        ai.t = 4.0;
                    }
                    break;
                }
            }
        }
        best.map(|b| b.0)
    }

    /// Steer toward `goal` at `speed`, with separation from neighbours and wall sliding.
    fn steer(&mut self, e: Entity, goal: Vec3, speed: f32, face: Option<Vec3>, dt: f32) {
        let body = *self.w.body.get(e).unwrap();
        let mut d = flat(goal - body.pos);
        let dist = d.length();
        if dist > 0.05 {
            d /= dist;
        }
        // separation
        let mut sep = Vec3::ZERO;
        self.hash.query(body.pos, body.radius + 1.0, |id, p, r| {
            if id != e.id {
                let o = flat(body.pos - p);
                let l = o.length();
                if l < body.radius + r + 0.4 && l > 1e-3 {
                    sep += o / l * (1.0 - l / (body.radius + r + 0.4));
                }
            }
        });
        let mut v = (d + sep * 1.5).normalize_or_zero() * speed * if dist < 0.3 { dist / 0.3 } else { 1.0 };
        // unstick: if pushing into a wall, try sliding sideways
        let ai = self.w.ai.get_mut(e).unwrap();
        let moved = (body.pos - body.prev).length() / dt.max(1e-4);
        if speed > 0.5 && moved < speed * 0.2 {
            ai.stuck += dt;
        } else {
            ai.stuck = (ai.stuck - dt).max(0.0);
        }
        if ai.stuck > 0.4 {
            let side = Vec3::new(-d.z, 0.0, d.x) * ai.strafe_dir;
            v = (side * 1.2 + d * 0.3).normalize_or_zero() * speed;
            if ai.stuck > 1.6 {
                ai.strafe_dir = -ai.strafe_dir;
                ai.stuck = 0.0;
            }
        }
        let b = self.w.body.get_mut(e).unwrap();
        b.vel = v;
        let f = face.unwrap_or(if v.length() > 0.2 { v } else { yaw_dir(b.yaw) });
        b.yaw = turn_toward(b.yaw, dir_yaw(f), dt * 7.0);
    }

    fn stop(&mut self, e: Entity, face: Option<Vec3>, dt: f32) {
        let b = self.w.body.get_mut(e).unwrap();
        b.vel = Vec3::ZERO;
        if let Some(f) = face {
            b.yaw = turn_toward(b.yaw, dir_yaw(f), dt * 6.0);
        }
    }

    fn enemy_rate(&self, e: Entity) -> f32 {
        let a = self.w.actor.get(e).unwrap();
        let mut r = 1.0;
        if let Some(f) = self.w.foe.get(e) {
            if f.affixes.contains(&AffixKind::Frenzied) {
                r += (1.0 - a.frac()) * 0.8;
            }
            if f.affixes.contains(&AffixKind::Hasted) {
                r += 0.25;
            }
        }
        r * (0.8 + 0.2 * a.rally)
    }

    fn begin_attack(&mut self, e: Entity, mv: MoveDef) {
        let rate = self.enemy_rate(e);
        let a = self.w.actor.get_mut(e).unwrap();
        a.act = Act::attack(mv, chance(&mut self.rng, 0.3), mv.dmg > 1.25, 0, rate);
        a.glow = 1.0;
    }

    /// Generic melee infected (rusher, brute, leaper, spitter, crawler, shielded, ambusher, summoner, husk).
    fn ai_infected(&mut self, e: Entity, dt: f32) {
        let kind = self.w.ai.get(e).unwrap().kind;
        let busy = self.w.actor.get(e).unwrap().act.busy();
        let body = *self.w.body.get(e).unwrap();
        {
            let ai = self.w.ai.get_mut(e).unwrap();
            ai.cd -= dt;
            ai.special_cd -= dt;
            ai.alert -= dt;
            ai.think -= dt;
            ai.t -= dt;
        }
        if self.w.foe.get(e).map(|f| f.mimic).unwrap_or(false) {
            // disguised: idle like a survivor until the player is close
            let d = (self.ppos() - body.pos).length();
            if d < 3.5 {
                self.reveal_mimic(e);
            } else {
                self.stop(e, None, dt);
                return;
            }
        }
        if busy {
            let b = self.w.body.get_mut(e).unwrap();
            let a = self.w.actor.get(e).unwrap();
            // lunge during attacks
            if a.act.kind == ActKind::Attack {
                let mv = a.act.mv.unwrap();
                let ph = a.act.phase();
                if ph < 1.0 {
                    if let Some(t) = self.w.ai.get(e).unwrap().target.and_then(|t| self.w.body.get(t)).map(|tb| tb.pos) {
                        let b = self.w.body.get_mut(e).unwrap();
                        b.yaw = turn_toward(b.yaw, dir_yaw(flat(t - b.pos)), dt * 5.0);
                    }
                    let b = self.w.body.get_mut(e).unwrap();
                    b.vel = Vec3::ZERO;
                } else if ph < 2.0 {
                    let b = self.w.body.get_mut(e).unwrap();
                    b.vel = yaw_dir(b.yaw) * mv.lunge / mv.act.max(0.15) * 0.8;
                } else {
                    let b = self.w.body.get_mut(e).unwrap();
                    b.vel = Vec3::ZERO;
                }
            } else {
                b.vel = Vec3::ZERO;
            }
            return;
        }
        let ai_t = self.w.ai.get(e).unwrap().think;
        if ai_t <= 0.0 {
            let range = if kind == AiKind::Ambusher { 7.0 } else { 18.0 };
            let t = self.perceive(e, range);
            let ai = self.w.ai.get_mut(e).unwrap();
            ai.think = 0.3;
            if t.is_some() {
                if ai.target != t || ai.alert <= 0.0 {
                    ai.alert = 6.0;
                }
                ai.target = t;
            } else if ai.alert <= 0.0 {
                ai.target = None;
            }
        }
        let ai = self.w.ai.get(e).unwrap().clone();
        let speed = self.w.actor.get(e).unwrap().speed * (1.0 - self.w.actor.get(e).unwrap().slow * 0.5) * self.w.actor.get(e).unwrap().tscale;
        let Some(t) = ai.target else {
            // wander / investigate
            if kind == AiKind::Ambusher {
                let a = self.w.actor.get_mut(e).unwrap();
                a.dissolve = (a.dissolve + dt).min(0.85);
                self.stop(e, None, dt);
                return;
            }
            if ai.state == AiState::Alert && ai.t > 0.0 {
                self.steer(e, ai.goal, speed * 0.6, None, dt);
                return;
            }
            let ai = self.w.ai.get_mut(e).unwrap();
            if (ai.goal - body.pos).length() < 1.0 || ai.t <= 0.0 {
                ai.t = rf(&mut self.rng, 3.0, 8.0);
                ai.goal = ai.home + rand_dir(&mut self.rng) * rf(&mut self.rng, 0.0, 9.0);
                ai.state = if chance(&mut self.rng, 0.4) { AiState::Idle } else { AiState::Wander };
            }
            let st = ai.state;
            let g = ai.goal;
            if st == AiState::Wander {
                self.steer(e, g, speed * 0.3, None, dt);
            } else {
                self.stop(e, None, dt);
            }
            return;
        };
        let Some(tb) = self.w.body.get(t).copied() else { return };
        let to = flat(tb.pos - body.pos);
        let dist = to.length() - tb.radius;
        let dirn = to.normalize_or_zero();
        let foe = self.w.foe.get(e).unwrap().clone();
        if !foe.shrieked && foe.affixes.contains(&AffixKind::Shrieking) {
            self.w.foe.get_mut(e).unwrap().shrieked = true;
            self.audio.play_at(Sfx::Shriek, body.pos, 0.9, 1.0);
            self.parts.burst(Fx::Dust, body.pos, Vec3::ZERO, 10, 2.0);
            let mut others = Vec::new();
            self.hash.query(body.pos, 30.0, |id, _, _| others.push(id));
            for id in others {
                let o = self.w.ent(id);
                if self.w.actor.get(o).map(|a| a.team == Team::Infected).unwrap_or(false) {
                    if let Some(oa) = self.w.ai.get_mut(o) {
                        oa.target = Some(t);
                        oa.alert = 12.0;
                    }
                }
            }
        }
        // dread: carriers hesitate before the player
        if t == self.player && foe.stage == 1 && self.pl.stats.has(|x| matches!(x, Effect::Dread)) && ai.cd < 0.2 && chance(&mut self.rng, 0.02) {
            self.w.ai.get_mut(e).unwrap().cd = 1.2;
        }
        let ready = ai.cd <= 0.0;
        let pick_attack = |g: &mut Game, maxd: f32, filter: &dyn Fn(&MoveDef) -> bool| -> Option<MoveDef> {
            let opts: Vec<MoveDef> = foe.attacks.iter().copied().filter(|m| filter(m) && maxd <= m.reach + m.lunge * 0.6).collect();
            if opts.is_empty() {
                None
            } else {
                Some(opts[g.rng.usize(0..opts.len())])
            }
        };
        match kind {
            AiKind::Spitter => {
                if dist < 3.0 && ready {
                    if let Some(mv) = pick_attack(self, dist, &|m| m.swing != Swing::Spit) {
                        self.begin_attack(e, mv);
                        self.w.ai.get_mut(e).unwrap().cd = rf(&mut self.rng, 1.0, 1.8);
                        return;
                    }
                }
                if dist < 7.0 {
                    let away = body.pos - dirn * 4.0;
                    self.steer(e, away, speed, Some(dirn), dt);
                } else if dist > 15.0 {
                    self.steer(e, tb.pos, speed, None, dt);
                } else if ready && self.area.line_of_sight(body.center(), tb.center()) {
                    let mv = foe.attacks.iter().copied().find(|m| m.swing == Swing::Spit).unwrap();
                    self.begin_attack(e, mv);
                    self.w.ai.get_mut(e).unwrap().cd = rf(&mut self.rng, 1.8, 3.0);
                } else {
                    let side = Vec3::new(-dirn.z, 0.0, dirn.x) * ai.strafe_dir;
                    self.steer(e, body.pos + side * 2.0, speed * 0.5, Some(dirn), dt);
                }
            }
            AiKind::Leaper => {
                if (4.0..9.5).contains(&dist) && ready {
                    let mv = foe.attacks.iter().copied().find(|m| m.swing == Swing::Leap).unwrap_or(foe.attacks[0]);
                    self.begin_attack(e, mv);
                    self.w.ai.get_mut(e).unwrap().cd = rf(&mut self.rng, 2.0, 3.2);
                } else if dist < 2.0 && ready {
                    let mv = foe.attacks.iter().copied().find(|m| m.swing != Swing::Leap).unwrap_or(foe.attacks[0]);
                    self.begin_attack(e, mv);
                    self.w.ai.get_mut(e).unwrap().cd = rf(&mut self.rng, 0.8, 1.6);
                } else if dist < 5.0 {
                    let side = Vec3::new(-dirn.z, 0.0, dirn.x) * ai.strafe_dir;
                    self.steer(e, body.pos + side * 3.0 - dirn * 2.0, speed, Some(dirn), dt);
                } else {
                    self.steer(e, tb.pos, speed, None, dt);
                }
            }
            AiKind::Ambusher => {
                let a = self.w.actor.get_mut(e).unwrap();
                if dist > 6.0 && ai.state != AiState::Chase {
                    a.dissolve = (a.dissolve + dt).min(0.85);
                    self.stop(e, Some(dirn), dt);
                    return;
                }
                a.dissolve = (a.dissolve - dt * 3.0).max(0.0);
                self.w.ai.get_mut(e).unwrap().state = AiState::Chase;
                if dist < 3.0 && ready {
                    if let Some(mv) = pick_attack(self, dist, &|_| true) {
                        self.begin_attack(e, mv);
                        let ai = self.w.ai.get_mut(e).unwrap();
                        ai.cd = rf(&mut self.rng, 1.2, 2.0);
                        // retreat and re-hide after striking
                        if chance(&mut self.rng, 0.4) {
                            ai.state = AiState::Hide;
                        }
                    }
                } else {
                    self.steer(e, tb.pos, speed * 1.1, None, dt);
                }
            }
            AiKind::Summoner => {
                if ai.special_cd <= 0.0 && ai.summons < 4 {
                    let ai = self.w.ai.get_mut(e).unwrap();
                    ai.special_cd = rf(&mut self.rng, 5.0, 8.0);
                    ai.summons += 2;
                    let mut a = Act::new(ActKind::Cast, 1.0);
                    a.spell = 0;
                    self.w.actor.get_mut(e).unwrap().act = a;
                    let lvl = self.w.actor.get(e).unwrap().level;
                    for _ in 0..2 {
                        let p = body.pos + rand_dir(&mut self.rng) * 1.5;
                        let c = self.spawn_archetype("crawler", p, 1, lvl, None);
                        if let Some(cai) = self.w.ai.get_mut(c) {
                            cai.target = Some(t);
                            cai.alert = 10.0;
                        }
                    }
                    self.parts.burst(Fx::Ichor, body.center(), Vec3::Y, 30, 1.5);
                    self.audio.play_at(Sfx::Larva, body.pos, 0.6, 0.9);
                } else if dist < 4.0 && ready {
                    let mv = foe.attacks[0];
                    self.begin_attack(e, mv);
                    self.w.ai.get_mut(e).unwrap().cd = 2.0;
                } else if dist < 9.0 {
                    self.steer(e, body.pos - dirn * 3.0, speed, Some(dirn), dt);
                } else if dist > 14.0 {
                    self.steer(e, tb.pos, speed, None, dt);
                } else {
                    self.stop(e, Some(dirn), dt);
                }
            }
            AiKind::Brute => {
                if (5.0..10.0).contains(&dist) && ai.special_cd <= 0.0 {
                    if let Some(mv) = foe.attacks.iter().copied().find(|m| m.swing == Swing::Charge) {
                        self.begin_attack(e, mv);
                        let ai = self.w.ai.get_mut(e).unwrap();
                        ai.special_cd = rf(&mut self.rng, 5.0, 8.0);
                        ai.cd = 1.5;
                        return;
                    }
                }
                if dist < 2.8 && ready {
                    if let Some(mv) = pick_attack(self, dist, &|m| m.swing != Swing::Charge) {
                        self.begin_attack(e, mv);
                        self.w.ai.get_mut(e).unwrap().cd = rf(&mut self.rng, 1.4, 2.4);
                        return;
                    }
                }
                self.steer(e, tb.pos, speed, Some(dirn), dt);
            }
            _ => {
                // rusher, crawler, shielded, husk
                let close = match kind {
                    AiKind::Crawler => 1.1,
                    AiKind::Shielded => 2.2,
                    AiKind::Husk => 2.2,
                    _ => 1.8,
                };
                if dist < close + 0.4 && ready {
                    if let Some(mv) = pick_attack(self, dist, &|_| true) {
                        self.begin_attack(e, mv);
                        let ai = self.w.ai.get_mut(e).unwrap();
                        ai.cd = match kind {
                            AiKind::Crawler => rf(&mut self.rng, 0.6, 1.4),
                            AiKind::Husk => rf(&mut self.rng, 0.4, 1.0),
                            AiKind::Shielded => rf(&mut self.rng, 1.4, 2.4),
                            _ => rf(&mut self.rng, 0.9, 1.9),
                        };
                        return;
                    }
                }
                // husks dodge like the player did
                if kind == AiKind::Husk && dist < 3.0 {
                    if let Some(pa) = self.w.actor.get(self.player) {
                        if pa.act.kind == ActKind::Attack && pa.act.phase() < 1.0 && chance(&mut self.rng, 0.06) {
                            let mut a = Act::new(ActKind::Dodge, 0.45);
                            a.dir = Vec3::new(-dirn.z, 0.0, dirn.x) * ai.strafe_dir;
                            self.w.actor.get_mut(e).unwrap().act = a;
                            return;
                        }
                    }
                }
                if dist < close && !ready {
                    // circle while recovering
                    let side = Vec3::new(-dirn.z, 0.0, dirn.x) * ai.strafe_dir;
                    self.steer(e, body.pos + side * 2.0, speed * 0.45, Some(dirn), dt);
                } else {
                    let sp = if kind == AiKind::Shielded { speed * 0.85 } else { speed };
                    self.steer(e, tb.pos, sp, Some(dirn), dt);
                }
            }
        }
        // dodge movement for husk dodges
        let a = self.w.actor.get(e).unwrap();
        if a.act.kind == ActKind::Dodge {
            let d = a.act.dir;
            self.w.body.get_mut(e).unwrap().vel = d * 7.0;
        }
    }

    /// Per-tick affix behaviours.
    fn affix_tick(&mut self, e: Entity, dt: f32) {
        let Some(f) = self.w.foe.get(e) else { return };
        if f.affixes.is_empty() && f.ttl <= 0.0 {
            return;
        }
        let f = f.clone();
        let body = *self.w.body.get(e).unwrap();
        let target = self.w.ai.get(e).and_then(|a| a.target);
        let engaged = target.is_some();
        {
            let ff = self.w.foe.get_mut(e).unwrap();
            ff.phase_t += dt;
            ff.aff_t -= dt;
        }
        for &af in &f.affixes {
            match af {
                AffixKind::Phasing => {
                    let ph = (f.phase_t % 5.0) > 3.4;
                    let a = self.w.actor.get_mut(e).unwrap();
                    a.dissolve = if ph { (a.dissolve + dt * 4.0).min(0.7) } else { (a.dissolve - dt * 4.0).max(0.0) };
                    if ph && chance(&mut self.rng, 0.3) {
                        self.parts.burst(Fx::Rift, body.center(), Vec3::ZERO, 1, 0.5);
                    }
                }
                AffixKind::Regenerating => {
                    let a = self.w.actor.get_mut(e).unwrap();
                    if a.since_hit > 3.0 && a.hp < a.max_hp {
                        a.hp = (a.hp + a.max_hp * 0.03 * dt).min(a.max_hp);
                        if chance(&mut self.rng, 0.1) {
                            self.parts.burst(Fx::Heal, body.pos, Vec3::ZERO, 1, 0.5);
                        }
                    }
                }
                AffixKind::Magnetised if engaged && target == Some(self.player) => {
                    let pp = self.ppos();
                    let d = flat(body.pos - pp);
                    if d.length() < 9.0 && d.length() > 2.0 && (f.phase_t % 6.0) < 1.2 {
                        if let Some(pb) = self.w.body.get_mut(self.player) {
                            pb.push += d.normalize() * 9.0 * dt;
                        }
                        if chance(&mut self.rng, 0.5) {
                            self.parts.burst(Fx::Rift, pp + Vec3::Y, d.normalize(), 1, 0.5);
                        }
                    }
                }
                AffixKind::Cindered => {
                    if chance(&mut self.rng, 0.4) {
                        self.parts.burst(Fx::Fire, body.center(), Vec3::ZERO, 1, 0.5);
                    }
                    if f.aff_t <= 0.0 && engaged {
                        self.w.foe.get_mut(e).unwrap().aff_t = 1.0;
                        self.aoe(body.pos, 2.2, 6.0 + self.w.actor.get(e).unwrap().level as f32 * 0.5, 0.0, Team::Infected, Some(e), DmgKind::Fire);
                    }
                }
                AffixKind::Sporing => {
                    if f.aff_t <= 0.0 && engaged {
                        self.w.foe.get_mut(e).unwrap().aff_t = 4.0;
                        let dmg = 5.0 + self.w.actor.get(e).unwrap().level as f32 * 0.6;
                        self.spawn_zone(body.pos, ZoneKind::Spores, 2.5, dmg, 4.0, Team::Infected, Some(e));
                    }
                }
                AffixKind::Broadcasting | AffixKind::Warding => {
                    let mut others = Vec::new();
                    self.hash.query(body.pos, 9.0, |id, _, _| others.push(id));
                    for id in others {
                        let o = self.w.ent(id);
                        if o == e {
                            continue;
                        }
                        if let Some(oa) = self.w.actor.get_mut(o) {
                            if oa.team == Team::Infected {
                                if af == AffixKind::Broadcasting {
                                    oa.rally = 1.25;
                                } else {
                                    oa.ward = 0.4;
                                }
                            }
                        }
                    }
                    if chance(&mut self.rng, 0.2) {
                        self.parts.burst(if af == AffixKind::Warding { Fx::Rift } else { Fx::Strain }, body.pos + Vec3::Y * body.height, Vec3::Y, 1, 0.5);
                    }
                }
                AffixKind::Blinking if engaged => {
                    if f.aff_t <= 0.0 {
                        self.w.foe.get_mut(e).unwrap().aff_t = rf(&mut self.rng, 5.0, 8.0);
                        if let Some(tb) = target.and_then(|t| self.w.body.get(t)).copied() {
                            let behind = tb.pos - yaw_dir(tb.yaw) * 2.0;
                            if !self.area.blocked(behind, body.radius) {
                                self.parts.burst(Fx::Rift, body.center(), Vec3::ZERO, 20, 1.0);
                                let b = self.w.body.get_mut(e).unwrap();
                                b.pos = behind;
                                b.prev = behind;
                                b.yaw = dir_yaw(flat(tb.pos - behind));
                                self.parts.burst(Fx::Rift, behind + Vec3::Y, Vec3::ZERO, 20, 1.0);
                                self.audio.play_at(Sfx::Rift, behind, 0.5, 1.4);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        if f.ttl > 0.0 {
            let ff = self.w.foe.get_mut(e).unwrap();
            ff.ttl -= dt;
            if ff.ttl <= 0.0 {
                self.aoe(body.pos, 2.0, 15.0 * self.pl.stats.spell_power, 10.0, Team::Ally, Some(e), DmgKind::Rot);
                self.parts.burst(Fx::Ichor, body.center(), Vec3::Y, 20, 1.5);
                self.w.actor.get_mut(e).unwrap().alive = false;
                self.w.kill(e);
            }
        }
    }

    fn ai_soldier(&mut self, e: Entity, dt: f32) {
        let body = *self.w.body.get(e).unwrap();
        {
            let ai = self.w.ai.get_mut(e).unwrap();
            ai.cd -= dt;
            ai.think -= dt;
            ai.t -= dt;
            ai.bark_cd -= dt;
            ai.grudge = (ai.grudge - dt * 3.0).max(0.0);
        }
        let busy = self.w.actor.get(e).unwrap().act.busy();
        if busy {
            self.stop(e, None, dt);
            return;
        }
        if self.w.ai.get(e).unwrap().think <= 0.0 {
            let t = self.perceive(e, 28.0);
            let ai = self.w.ai.get_mut(e).unwrap();
            ai.think = 0.4;
            ai.target = t;
        }
        let ai = self.w.ai.get(e).unwrap().clone();
        let speed = self.w.actor.get(e).unwrap().speed;
        let pp = self.ppos();
        // barks at the compatible
        if ai.target.is_none() && (pp - body.pos).length() < 4.5 && ai.bark_cd <= 0.0 {
            let mutated = self.pl.life.mutations.len() >= 4;
            let lines = if mutated { &self.t.text.soldier_mutated } else { &self.t.text.soldier_barks };
            let line = lines[self.rng.usize(0..lines.len())].clone();
            self.say(e, &line);
            self.w.ai.get_mut(e).unwrap().bark_cd = rf(&mut self.rng, 18.0, 30.0);
        }
        match ai.target {
            Some(t) => {
                let tb = *self.w.body.get(t).unwrap();
                let to = flat(tb.pos - body.pos);
                let dist = to.length();
                let dirn = to.normalize_or_zero();
                let foe = self.w.foe.get(e).unwrap();
                if dist < 2.0 && ai.cd <= 0.0 {
                    let mv = foe.attacks.iter().copied().find(|m| m.swing == Swing::Bash).unwrap();
                    self.begin_attack(e, mv);
                    self.w.ai.get_mut(e).unwrap().cd = 1.0;
                } else if dist < 6.0 {
                    self.steer(e, body.pos - dirn * 4.0, speed, Some(dirn), dt);
                } else if dist < 24.0 && ai.cd <= 0.0 && self.area.line_of_sight(body.center() + Vec3::Y * 0.3, tb.center()) {
                    let mv = foe.attacks.iter().copied().find(|m| m.swing == Swing::Shoot).unwrap();
                    self.begin_attack(e, mv);
                    self.w.ai.get_mut(e).unwrap().cd = rf(&mut self.rng, 0.9, 1.8);
                } else if dist > 16.0 {
                    self.steer(e, tb.pos, speed, Some(dirn), dt);
                } else {
                    self.stop(e, Some(dirn), dt);
                }
                if chance(&mut self.rng, 0.002) {
                    let lines = ["CONTACT!", "TANGO, TWELVE O'CLOCK!", "KEEP IT OFF THE LINE!", "RELOADING!", "BURN IT!"];
                    let l = lines[self.rng.usize(0..lines.len())];
                    self.say(e, l);
                }
            }
            None => {
                // patrol around home
                if (ai.goal - body.pos).length() < 1.0 || ai.t <= 0.0 {
                    let ai = self.w.ai.get_mut(e).unwrap();
                    ai.t = rf(&mut self.rng, 4.0, 10.0);
                    let r = (ai.summons % 7) as f32;
                    ai.goal = ai.home + Vec3::new((self.sim_time as f32 * 0.05 + r).sin(), 0.0, (self.sim_time as f32 * 0.05 + r).cos()) * 8.0 + rand_dir(&mut self.rng) * 2.0;
                }
                let g = self.w.ai.get(e).unwrap().goal;
                let near_player = (pp - body.pos).length() < 6.0;
                let face = if near_player { Some(flat(pp - body.pos)) } else { None };
                if near_player {
                    self.stop(e, face, dt);
                } else {
                    self.steer(e, g, speed * 0.35, None, dt);
                }
            }
        }
    }

    fn ai_civilian(&mut self, e: Entity, dt: f32) {
        let body = *self.w.body.get(e).unwrap();
        {
            let ai = self.w.ai.get_mut(e).unwrap();
            ai.think -= dt;
            ai.t -= dt;
            ai.bark_cd -= dt;
        }
        // transformation event in the street
        let tr = self.w.foe.get(e).map(|f| f.transform_t).unwrap_or(0.0);
        if tr > 0.0 {
            let f = self.w.foe.get_mut(e).unwrap();
            f.transform_t += dt;
            let a = self.w.actor.get_mut(e).unwrap();
            a.flash = (f.transform_t * 9.0).sin().abs() * 0.6;
            a.act = Act::new(ActKind::Seized, 0.2);
            if chance(&mut self.rng, 0.3) {
                self.parts.burst(Fx::Ichor, body.center(), Vec3::Y, 2, 1.0);
            }
            if f.transform_t > 3.2 {
                let pos = body.pos;
                self.parts.burst(Fx::Ichor, body.center(), Vec3::Y, 50, 2.5);
                self.audio.play_at(Sfx::Bloom, pos, 0.9, 1.0);
                self.w.actor.get_mut(e).unwrap().alive = false;
                self.w.kill(e);
                let lvl = { let mut r = crate::util::Rng::with_seed(self.rng.u64(..)); self.level_at(pos, &mut r) };
                let n = self.spawn_archetype("rusher", pos, 2, lvl, None);
                if let Some(ai) = self.w.ai.get_mut(n) {
                    ai.target = Some(self.player);
                    ai.alert = 8.0;
                }
                self.msg("SOMEONE IS TURNING IN THE STREET");
            }
            self.stop(e, None, dt);
            return;
        }
        let pp = self.ppos();
        let near_player = (pp - body.pos).length();
        if near_player < 25.0 && near_player > 5.0 {
            let lost = matches!(self.area.kind, crate::world::AreaKind::City) && crate::world::district_at(body.pos).map(|d| crate::world::DISTRICTS[d].state != crate::world::DState::Held).unwrap_or(false);
            if lost && chance(&mut self.rng, 0.0006) {
                self.w.foe.get_mut(e).unwrap().transform_t = 0.01;
                self.say(e, "IT'S IN ME - IT'S IN ME -");
                self.audio.play_at(Sfx::Hurt, body.pos, 0.6, 1.3);
                return;
            }
        }
        // flee from infected
        let mut threat: Option<Vec3> = None;
        self.hash.query(body.pos, 10.0, |id, p, _| {
            let o = self.w.ent(id);
            if let Some(a) = self.w.actor.get(o) {
                if a.alive && a.team == Team::Infected && !self.w.foe.get(o).map(|f| f.mimic).unwrap_or(false) {
                    threat = Some(p);
                }
            }
        });
        let speed = self.w.actor.get(e).unwrap().speed;
        let ai = self.w.ai.get(e).unwrap().clone();
        if let Some(tp) = threat {
            let away = flat(body.pos - tp).normalize_or_zero();
            self.steer(e, body.pos + away * 5.0, speed * 1.1, None, dt);
            self.w.ai.get_mut(e).unwrap().state = AiState::Flee;
            if ai.bark_cd <= 0.0 {
                let l = self.t.text.civilian[self.rng.usize(0..self.t.text.civilian.len())].clone();
                self.say(e, &l);
                self.w.ai.get_mut(e).unwrap().bark_cd = 12.0;
            }
        } else {
            // hide: cower near home, glance at the player
            let st = if (ai.home - body.pos).length() > 3.0 { AiState::Wander } else { AiState::Cower };
            self.w.ai.get_mut(e).unwrap().state = st;
            if st == AiState::Wander {
                self.steer(e, ai.home, speed * 0.5, None, dt);
            } else {
                let face = if near_player < 8.0 { Some(flat(pp - body.pos)) } else { None };
                self.stop(e, face, dt);
                if near_player < 3.0 && ai.bark_cd <= 0.0 {
                    let l = self.t.text.civilian[self.rng.usize(0..self.t.text.civilian.len())].clone();
                    self.say(e, &l);
                    self.w.ai.get_mut(e).unwrap().bark_cd = 15.0;
                }
            }
        }
    }

    fn ai_ally(&mut self, e: Entity, dt: f32) {
        let body = *self.w.body.get(e).unwrap();
        {
            let ai = self.w.ai.get_mut(e).unwrap();
            ai.cd -= dt;
            ai.think -= dt;
        }
        if self.w.actor.get(e).unwrap().act.busy() {
            self.stop(e, None, dt);
            return;
        }
        if self.w.ai.get(e).unwrap().think <= 0.0 {
            let t = self.nearest_hostile(body.pos, 9.0, Team::Ally);
            let ai = self.w.ai.get_mut(e).unwrap();
            ai.think = 0.3;
            ai.target = t;
        }
        let speed = self.w.actor.get(e).unwrap().speed;
        let ai = self.w.ai.get(e).unwrap().clone();
        match ai.target.and_then(|t| self.w.body.get(t)).copied() {
            Some(tb) => {
                let d = flat(tb.pos - body.pos).length();
                if d < 1.4 && ai.cd <= 0.0 {
                    let mv = self.w.foe.get(e).unwrap().attacks[0];
                    self.begin_attack(e, mv);
                    self.w.ai.get_mut(e).unwrap().cd = 0.8;
                } else {
                    self.steer(e, tb.pos, speed, None, dt);
                }
            }
            None => {
                let pp = self.ppos();
                if (pp - body.pos).length() > 3.0 {
                    self.steer(e, pp, speed, None, dt);
                } else {
                    self.stop(e, None, dt);
                }
            }
        }
    }

    // ---- bosses -----------------------------------------------------------------------------
    fn ai_boss(&mut self, e: Entity, dt: f32) {
        let Some(bi) = self.w.foe.get(e).and_then(|f| f.boss) else { return };
        let bd = self.t.bosses[bi].clone();
        let body = *self.w.body.get(e).unwrap();
        let frac = self.w.actor.get(e).unwrap().frac();
        {
            let ai = self.w.ai.get_mut(e).unwrap();
            ai.cd -= dt;
            ai.target = Some(self.player);
        }
        if !self.boss_awake {
            self.stop(e, Some(flat(self.ppos() - body.pos)), dt);
            return;
        }
        // phase transitions
        let cur = self.w.ai.get(e).unwrap().phase;
        let next = bd.phases.iter().rposition(|p| frac <= p.at).unwrap_or(0);
        if next > cur {
            self.w.ai.get_mut(e).unwrap().phase = next;
            let line = bd.phases[next].line.clone();
            self.subtitle(&line, 3.5);
            self.audio.play(Sfx::BossRoar, 0.9, 0.8 + next as f32 * 0.1);
            self.shake(0.9);
            self.parts.burst(Fx::Ichor, body.center(), Vec3::Y, 80, 3.0);
            let a = self.w.actor.get_mut(e).unwrap();
            a.act = Act::new(ActKind::Stagger, 1.4);
            a.poise = a.max_poise;
            self.aoe(body.pos, 5.0, 10.0, 60.0, Team::Infected, Some(e), DmgKind::Heavy);
            self.spawn_zone(body.pos, ZoneKind::Ring, 8.0, 0.0, 0.7, Team::Infected, Some(e));
            if let Some(r) = self.w.rig.get_mut(e) {
                r.plan.glow = [1.0, 0.3 + 0.2 * next as f32, 0.3];
            }
            return;
        }
        let a = self.w.actor.get(e).unwrap().act.clone();
        if a.busy() {
            if a.kind == ActKind::Attack {
                let ph = a.phase();
                let mv = a.mv.unwrap();
                let pp = self.ppos();
                let b = self.w.body.get_mut(e).unwrap();
                if ph < 1.0 {
                    b.yaw = turn_toward(b.yaw, dir_yaw(flat(pp - b.pos)), dt * 4.0);
                    b.vel = Vec3::ZERO;
                } else if ph < 2.0 {
                    b.vel = yaw_dir(b.yaw) * mv.lunge / mv.act.max(0.15) * 0.8;
                } else {
                    b.vel = Vec3::ZERO;
                }
            } else if a.kind != ActKind::Special {
                self.w.body.get_mut(e).unwrap().vel = Vec3::ZERO;
            }
            return;
        }
        let phase = &bd.phases[cur];
        let pp = self.ppos();
        let to = flat(pp - body.pos);
        let dist = to.length();
        let speed = 3.2 * phase.speed;
        let ai = self.w.ai.get(e).unwrap().clone();
        if ai.cd > 0.0 {
            if dist > 4.0 {
                self.steer(e, pp, speed, Some(to), dt);
            } else {
                let side = Vec3::new(-to.z, 0.0, to.x).normalize_or_zero() * ai.strafe_dir;
                self.steer(e, body.pos + side * 2.0, speed * 0.4, Some(to), dt);
            }
            return;
        }
        // choose a move: melee when close, otherwise anything that is not melee
        let moves = &phase.moves;
        let mut opts: Vec<usize> = (0..moves.len()).filter(|&i| matches!(moves[i], BossMove::Melee(_)) == (dist < 5.5)).collect();
        if opts.is_empty() {
            opts = (0..moves.len()).collect();
        }
        let mi = opts[self.rng.usize(0..opts.len())];
        let mv = moves[mi];
        self.w.ai.get_mut(e).unwrap().move_i = mi;
        let rate = phase.speed;
        match mv {
            BossMove::Melee(i) => {
                let n = bd.attacks.len();
                let m = bd.attacks[i % n];
                let a = self.w.actor.get_mut(e).unwrap();
                a.act = Act::attack(m, false, m.dmg > 1.2, 0, rate);
                // combos: queue a follow-up by shortening the cooldown
                self.w.ai.get_mut(e).unwrap().cd = if chance(&mut self.rng, 0.5) { 0.1 } else { rf(&mut self.rng, 0.6, 1.2) / rate };
                return;
            }
            _ => {
                let dur = match mv {
                    BossMove::Slam(..) => 1.6,
                    BossMove::Volley(..) => 1.4,
                    BossMove::Rush(_) => 2.2,
                    BossMove::Summon(_) => 1.5,
                    BossMove::Beam(_, d) => d + 1.0,
                    BossMove::Warp => 1.0,
                    BossMove::Pools(..) => 1.2,
                    BossMove::Rain(..) => 1.4,
                    BossMove::Vortex(_, d) => d + 0.8,
                    BossMove::Melee(_) => 1.0,
                };
                let mut act = Act::new(ActKind::Special, dur);
                act.spell = mi;
                act.rate = rate.min(1.3);
                act.dir = to.normalize_or_zero();
                self.w.actor.get_mut(e).unwrap().act = act;
                self.w.actor.get_mut(e).unwrap().glow = 1.0;
                self.audio.play_at(Sfx::Growl, body.pos, 0.8, 0.6);
                self.w.ai.get_mut(e).unwrap().cd = rf(&mut self.rng, 0.8, 1.6) / rate;
            }
        }
    }

    pub fn boss_special_tick(&mut self, e: Entity, prev: f32, a: &Act, dt: f32) {
        let Some(bi) = self.w.foe.get(e).and_then(|f| f.boss) else { return };
        let ai = self.w.ai.get(e).unwrap().clone();
        let bd = &self.t.bosses[bi];
        let Some(mv) = bd.phases[ai.phase].moves.get(a.spell).copied() else { return };
        let body = *self.w.body.get(e).unwrap();
        let pp = self.ppos();
        let dmg_mult = self.w.actor.get(e).unwrap().dmg / bd.dmg.max(1.0);
        let at = |x: f32| prev < x && a.t >= x;
        let summons = bd.summons.clone();
        // tells: glow and particles during windup
        if a.t < 0.8 {
            self.parts.burst(Fx::Strain, body.center(), Vec3::ZERO, 1, 0.8);
        }
        match mv {
            BossMove::Slam(r, d) => {
                if a.t < 0.8 {
                    self.w.body.get_mut(e).unwrap().vel = Vec3::ZERO;
                }
                if at(0.8) {
                    self.spawn_zone(body.pos, ZoneKind::Ring, r, d * dmg_mult, 0.7, Team::Infected, Some(e));
                    self.parts.burst(Fx::Shock, body.pos, Vec3::ZERO, 30, r * 1.5);
                    self.audio.play_at(Sfx::Explode, body.pos, 1.0, 0.6);
                    self.shake(0.9);
                }
            }
            BossMove::Volley(n, speed, d) => {
                if at(0.7) {
                    let dir = flat(pp - body.pos).normalize_or_zero();
                    let spread = 0.9;
                    for i in 0..n {
                        let k = if n > 1 { i as f32 / (n - 1) as f32 - 0.5 } else { 0.0 };
                        let ang = k * spread;
                        let dd = Vec3::new(dir.x * ang.cos() - dir.z * ang.sin(), 0.0, dir.x * ang.sin() + dir.z * ang.cos());
                        let origin = body.center() + Vec3::Y * 0.5;
                        let pe = self.spawn_proj(origin, dd * speed, ProjKind::BossOrb, d * dmg_mult, 15.0, Team::Infected, Some(e), 1.4);
                        if i % 3 == 0 {
                            if let Some(p) = self.w.proj.get_mut(pe) {
                                p.homing = Some(self.player);
                            }
                        }
                    }
                    self.audio.play_at(Sfx::Rift, body.pos, 0.8, 1.2);
                }
            }
            BossMove::Rush(d) => {
                let b = self.w.body.get_mut(e).unwrap();
                if a.t < 0.7 {
                    b.vel = Vec3::ZERO;
                    b.yaw = turn_toward(b.yaw, dir_yaw(flat(pp - b.pos)), dt * 5.0);
                } else if a.t < 1.8 {
                    b.vel = yaw_dir(b.yaw) * 14.0;
                    if (pp - b.pos).length() < b.radius + 1.2 && !self.rush_hit {
                        self.rush_hit = true;
                        let dir = yaw_dir(body.yaw);
                        self.deal(self.player, Some(e), d * dmg_mult, 60.0, dir, DmgKind::Heavy, 8.0);
                    }
                    if chance(&mut self.rng, 0.5) {
                        self.parts.burst(Fx::Dust, body.pos, -yaw_dir(body.yaw), 2, 1.0);
                    }
                } else {
                    b.vel = Vec3::ZERO;
                    self.rush_hit = false;
                }
            }
            BossMove::Summon(n) => {
                if at(0.9) {
                    let lvl = self.w.actor.get(e).unwrap().level.saturating_sub(4);
                    for i in 0..n {
                        let ang = i as f32 / n as f32 * std::f32::consts::TAU;
                        let p = body.pos + Vec3::new(ang.cos(), 0.0, ang.sin()) * 3.0;
                        let c = self.spawn_archetype(&summons, p, 1, lvl.max(1), None);
                        if let Some(cai) = self.w.ai.get_mut(c) {
                            cai.target = Some(self.player);
                            cai.alert = 30.0;
                        }
                        self.parts.burst(Fx::Ichor, p + Vec3::Y, Vec3::Y, 16, 1.5);
                    }
                    self.audio.play_at(Sfx::Larva, body.pos, 0.9, 0.8);
                }
            }
            BossMove::Beam(dps, d) => {
                if at(0.8) {
                    let mut z = self.make_zone(body.pos, ZoneKind::Beam, 30.0, dps * dmg_mult, d, Team::Infected, Some(e));
                    z.dir = flat(pp - body.pos).normalize_or_zero();
                    self.add_zone(body.pos, z);
                    self.audio.play_at(Sfx::Rift, body.pos, 1.0, 0.5);
                }
                self.w.body.get_mut(e).unwrap().vel = Vec3::ZERO;
            }
            BossMove::Warp => {
                if at(0.5) {
                    let pb = self.player_body();
                    let mut dest = pb.pos - yaw_dir(pb.yaw) * 3.5;
                    let (ac, ar) = self.area.arena;
                    if ar > 0.0 && (dest - ac).length() > ar - 2.0 {
                        dest = ac + (dest - ac).normalize_or_zero() * (ar - 3.0);
                    }
                    if self.area.blocked(dest, body.radius) {
                        dest = pb.pos + yaw_dir(pb.yaw) * 4.0;
                    }
                    self.parts.burst(Fx::Rift, body.center(), Vec3::ZERO, 40, 2.0);
                    let b = self.w.body.get_mut(e).unwrap();
                    b.pos = dest;
                    b.prev = dest;
                    b.yaw = dir_yaw(flat(pb.pos - dest));
                    self.parts.burst(Fx::Rift, dest + Vec3::Y, Vec3::ZERO, 40, 2.0);
                    self.audio.play_at(Sfx::Rift, dest, 0.9, 0.8);
                    self.w.ai.get_mut(e).unwrap().cd = 0.2;
                }
            }
            BossMove::Pools(r, dps, d) => {
                if at(0.6) {
                    let kind = if bi == 2 { ZoneKind::Fire } else { ZoneKind::Acid };
                    for k in 0..4 {
                        let p = if k == 0 { pp } else { pp + rand_dir(&mut self.rng) * rf(&mut self.rng, 3.0, 7.0) };
                        self.spawn_zone(p, kind, r, dps * dmg_mult, d, Team::Infected, Some(e));
                    }
                    self.audio.play_at(Sfx::Spit, body.pos, 1.0, 0.6);
                }
            }
            BossMove::Rain(n, d) => {
                if at(0.6) {
                    for k in 0..n {
                        let p = if k == 0 { pp } else { pp + rand_dir(&mut self.rng) * rf(&mut self.rng, 1.5, 9.0) };
                        let mut z = self.make_zone(p, ZoneKind::Blast, 2.4, d * dmg_mult, 2.0, Team::Infected, Some(e));
                        z.delay = 1.1 + k as f32 * 0.12;
                        self.add_zone(p, z);
                    }
                }
            }
            BossMove::Vortex(f, d) => {
                if at(0.5) {
                    let mut z = self.make_zone(body.pos, ZoneKind::Vortex, 13.0, 0.0, d, Team::Infected, Some(e));
                    z.len = f;
                    self.add_zone(body.pos, z);
                    self.audio.play_at(Sfx::Rift, body.pos, 1.0, 0.4);
                }
                self.w.body.get_mut(e).unwrap().vel = Vec3::ZERO;
            }
            BossMove::Melee(_) => {}
        }
        let _ = hostile;
    }
}
