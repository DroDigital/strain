//! Combat: action timing (wind-up / active / recovery), melee hit resolution, the damage pipeline
//! (i-frames, blocking, perfect blocks, armour, poise, stagger, affixes), Ruptures, projectiles,
//! area zones, status effects and deaths.
use super::comp::*;
use super::Game;
use crate::audio::Sfx;
use crate::data::{AffixKind, AiKind, Effect, Swing};
use crate::ecs::Entity;
use crate::particles::Fx;
use crate::physics::{Statics, OPAQUE};
use crate::util::{chance, flat, rand_dir, rf, yaw_dir};
use glam::Vec3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DmgKind {
    Melee,
    Heavy,
    Proj,
    Spell,
    Fire,
    Rot,
    True,
    Rupture,
}

pub fn hostile(a: Team, b: Team) -> bool {
    use Team::*;
    match (a, b) {
        (Infected, x) | (x, Infected) => x != Infected,
        _ => false,
    }
}

impl Game {
    pub fn team(&self, e: Entity) -> Option<Team> {
        self.w.actor.get(e).map(|a| a.team)
    }
    /// Can `att` damage `tgt`? Accounts for soldiers' grudge against the player.
    pub fn can_hurt(&self, att: Option<Entity>, att_team: Team, tgt: Entity) -> bool {
        let Some(ta) = self.w.actor.get(tgt) else { return false };
        if !ta.alive {
            return false;
        }
        if Some(tgt) == att {
            return false;
        }
        if hostile(att_team, ta.team) {
            return true;
        }
        match (att_team, ta.team) {
            (Team::Player, Team::Military) => true,
            (Team::Military, Team::Player) => att.and_then(|a| self.w.ai.get(a)).map(|ai| ai.grudge > 40.0).unwrap_or(false),
            (Team::Ally, Team::Infected) => true,
            _ => false,
        }
    }

    pub fn nearest_hostile(&self, p: Vec3, r: f32, team: Team) -> Option<Entity> {
        let mut best: Option<(Entity, f32)> = None;
        self.hash.query(p, r, |id, bp, _| {
            let e = self.w.ent(id);
            if let Some(a) = self.w.actor.get(e) {
                if a.alive && hostile(team, a.team) && a.dissolve < 0.5 {
                    let d = (bp - p).length();
                    if best.map(|b| d < b.1).unwrap_or(true) {
                        best = Some((e, d));
                    }
                }
            }
        });
        best.map(|b| b.0)
    }

    /// Soldiers who are hit by the player turn on them, and so does the rest of the squad.
    pub fn anger_squad(&mut self, t: Entity) {
        let Some(squad) = self.w.ai.get(t).map(|a| a.summons) else { return };
        let p = self.w.body.get(t).map(|b| b.pos).unwrap_or(Vec3::ZERO);
        let ids: Vec<u32> = self.w.ai.ids().collect();
        for id in ids {
            let e = self.w.ent(id);
            let same = self.w.ai.get(e).map(|a| a.kind == AiKind::Soldier && (a.summons == squad || e == t)).unwrap_or(false);
            let near = self.w.body.get(e).map(|b| (b.pos - p).length() < 30.0).unwrap_or(false);
            if same && near {
                let pe = self.player;
                let ai = self.w.ai.get_mut(e).unwrap();
                ai.grudge = (ai.grudge.max(0.0) + if e == t { 45.0 } else { 35.0 }).max(65.0);
                ai.target = Some(pe);
                ai.think = 0.0;
            }
        }
        if self.pl.soldiers_angered == 0 {
            self.msg("THE SQUAD TURNS ITS RIFLES ON YOU");
        }
        self.pl.soldiers_angered += 1;
    }

    pub fn noise(&mut self, p: Vec3, r: f32) {
        self.noises.push((p, r));
    }

    /// Advance every actor's current action; resolve active attack frames.
    pub fn actions_tick(&mut self, dt: f32) {
        let ids: Vec<u32> = self.w.actor.ids().collect();
        for id in ids {
            let e = self.w.ent(id);
            let (prev, kind, rate, scale) = {
                let a = self.w.actor.get(e).unwrap();
                if !a.alive {
                    continue;
                }
                (a.act.t, a.act.kind, a.act.rate, a.tscale * (1.0 - a.slow * 0.5))
            };
            if kind == ActKind::Idle {
                continue;
            }
            {
                let a = self.w.actor.get_mut(e).unwrap();
                a.act.t += dt * rate.max(0.2) * scale;
            }
            let a = self.w.actor.get(e).unwrap().act.clone();
            if e == self.player {
                self.player_act_events(prev);
            }
            match a.kind {
                ActKind::Attack => {
                    let mv = a.mv.unwrap();
                    let start = mv.wind;
                    // windup tells: sound + glint for enemies
                    if e != self.player && prev < start * 0.55 && a.t >= start * 0.55 {
                        let p = self.w.body.get(e).unwrap().pos;
                        if matches!(mv.swing, Swing::Shoot) {
                            // muzzle handled at fire time
                        } else {
                            self.audio.play_at(Sfx::Growl, p, 0.25, rf(&mut self.rng, 0.9, 1.4));
                        }
                    }
                    if prev < start && a.t >= start {
                        self.on_attack_start(e, &a);
                    }
                    if a.active() {
                        self.resolve_melee(e);
                    }
                }
                ActKind::Rupture => {
                    if prev < 0.45 && a.t >= 0.45 {
                        self.rupture_hit(e, a.target);
                    }
                }
                ActKind::Special => {
                    self.boss_special_tick(e, prev, &a, dt);
                }
                _ => {}
            }
            let a = self.w.actor.get_mut(e).unwrap();
            if a.act.t >= a.act.dur && !(a.act.charging && a.act.kind == ActKind::Attack) {
                let was = a.act.kind;
                a.act = Act::default();
                if was == ActKind::Dodge {
                    if let Some(b) = self.w.body.get_mut(e) {
                        b.ghost = false;
                    }
                }
            }
        }
    }

    /// Fired the instant an attack becomes active.
    fn on_attack_start(&mut self, e: Entity, a: &Act) {
        let mv = a.mv.unwrap();
        let body = *self.w.body.get(e).unwrap();
        let fwd = yaw_dir(body.yaw);
        if e == self.player {
            let p = body.pos;
            if a.heavy {
                let pull = self.pl.stats.sum(|x| if let Effect::HeavyPull(v) = x { Some(*v) } else { None });
                if pull > 0.0 {
                    self.pull_all(p + fwd * 1.0, 6.0, pull, Team::Player);
                    self.parts.burst(Fx::Ichor, p + Vec3::Y, fwd, 10, 2.0);
                }
            }
            return;
        }
        match mv.swing {
            Swing::Spit | Swing::Shoot => {
                let foe = self.w.ai.get(e).and_then(|ai| ai.target);
                let tp = foe.and_then(|t| self.w.body.get(t)).map(|b| b.center()).unwrap_or(body.center() + fwd * 10.0);
                let rig = self.w.rig.get(e);
                // rig anchors are only refreshed while the actor is drawn; fall back when stale
                let origin = rig.map(|r| if mv.swing == Swing::Shoot { r.weapon_tip } else { r.head }).filter(|o| (*o - body.center()).length() < 2.5).unwrap_or(body.center() + fwd * 0.5);
                let dmg = self.w.actor.get(e).unwrap().dmg * mv.dmg;
                let team = self.w.actor.get(e).unwrap().team;
                if mv.swing == Swing::Shoot {
                    for k in 0..3 {
                        let spread = Vec3::new(rf(&mut self.rng, -1.0, 1.0), rf(&mut self.rng, -0.4, 0.6), rf(&mut self.rng, -1.0, 1.0)) * 0.025 * (k as f32 + 1.0);
                        let d = ((tp - origin).normalize_or_zero() + spread).normalize_or_zero();
                        self.spawn_proj(origin, d * 60.0, ProjKind::Bullet, dmg, 4.0, team, Some(e), 0.0);
                    }
                    self.parts.burst(Fx::Flash, origin, fwd, 1, 0.5);
                    self.audio.play_at(Sfx::Gunshot, origin, 0.7, rf(&mut self.rng, 0.9, 1.1));
                    self.noise(origin, 30.0);
                    self.lights_temp.push((origin, Vec3::new(4.0, 3.0, 1.5), 8.0, 0.06));
                } else {
                    // lob acid with an arc
                    let flatd = flat(tp - origin);
                    let dist = flatd.length().max(1.0);
                    let speed = 16.0;
                    let tt = dist / speed;
                    let vy = (tp.y - origin.y) / tt + 0.5 * 9.0 * tt;
                    let v = flatd / dist * speed + Vec3::Y * vy;
                    self.spawn_proj(origin, v, ProjKind::Acid, dmg, 6.0, team, Some(e), 1.2);
                    self.audio.play_at(Sfx::Spit, origin, 0.6, 1.0);
                }
            }
            Swing::Lash if self.w.foe.get(e).map(|f| f.affixes.contains(&AffixKind::Grasping)).unwrap_or(false) => {}
            _ => {
                self.audio.play_at(if mv.dmg > 1.2 { Sfx::SwingHeavy } else { Sfx::Swing }, body.pos, 0.35, 0.8);
            }
        }
    }

    /// Check the active frames of `e`'s attack against everything in its arc.
    fn resolve_melee(&mut self, e: Entity) {
        let a = self.w.actor.get(e).unwrap().act.clone();
        let mv = a.mv.unwrap();
        if matches!(mv.swing, Swing::Spit | Swing::Shoot) {
            return;
        }
        let body = *self.w.body.get(e).unwrap();
        let team = self.w.actor.get(e).unwrap().team;
        let is_boss = self.w.foe.get(e).map(|f| f.boss.is_some()).unwrap_or(false);
        let scale = if is_boss { 1.0 } else { self.w.rig.get(e).map(|r| r.plan.scale).unwrap_or(1.0) };
        let grasp = self.w.foe.get(e).map(|f| f.affixes.contains(&AffixKind::Grasping)).unwrap_or(false);
        let reach = mv.reach * scale.max(1.0).sqrt() + if grasp { 1.2 } else { 0.0 };
        let fwd = yaw_dir(body.yaw);
        let cos_arc = (mv.arc.to_radians()).cos();
        let mut targets = Vec::new();
        self.hash.query(body.pos, reach + 1.0, |id, p, r| {
            if id == e.id {
                return;
            }
            let d = flat(p - body.pos);
            let dist = d.length();
            if dist - r > reach {
                return;
            }
            let dn = if dist > 1e-3 { d / dist } else { fwd };
            if mv.arc < 179.0 && dn.dot(fwd) < cos_arc && dist > r + 0.3 {
                return;
            }
            targets.push((id, dn));
        });
        for (id, dn) in targets {
            if a.hits.contains(&id) {
                continue;
            }
            let t = self.w.ent(id);
            if !self.can_hurt(Some(e), team, t) {
                continue;
            }
            // walls block swings
            let tp = self.w.body.get(t).unwrap().center();
            if !self.area.line_of_sight(body.center(), tp) {
                continue;
            }
            if let Some(ac) = self.w.actor.get_mut(e) {
                ac.act.hits.push(id);
            }
            if e == self.player {
                self.player_melee_hit(t, &a, dn);
            } else {
                let ac = self.w.actor.get(e).unwrap();
                let dmg = ac.dmg * mv.dmg * ac.rally;
                let kind = if mv.dmg >= 1.3 { DmgKind::Heavy } else { DmgKind::Melee };
                let dealt = self.deal(t, Some(e), dmg, mv.poise, dn, kind, mv.knock);
                if dealt > 0.0 {
                    if let Some(f) = self.w.foe.get(e) {
                        if f.affixes.contains(&AffixKind::Leeching) {
                            if let Some(ac) = self.w.actor.get_mut(e) {
                                ac.hp = (ac.hp + dealt * 0.5).min(ac.max_hp);
                            }
                        }
                        if f.affixes.contains(&AffixKind::Grasping) {
                            if let Some(ta) = self.w.actor.get_mut(t) {
                                ta.slow = ta.slow.max(0.5);
                            }
                        }
                        if f.affixes.contains(&AffixKind::Withering) && t == self.player {
                            self.pl.strain += 7.0;
                        }
                    }
                }
            }
        }
    }

    fn player_melee_hit(&mut self, t: Entity, a: &Act, dn: Vec3) {
        let pe = self.player;
        let st = self.pl.stats.clone();
        let mv = a.mv.unwrap();
        let (_, power) = self.pl.hand_weapon(&self.t, a.left);
        let mut dmg = power * mv.dmg * st.dmg;
        if a.heavy {
            dmg *= st.heavy * (1.0 + a.charge * 0.8);
        }
        let lowhp = st.sum(|e| if let Effect::LowHpPower(x) = e { Some(*x) } else { None });
        let pa = self.w.actor.get(pe).unwrap();
        if lowhp > 0.0 && pa.frac() < 0.35 {
            dmg *= 1.0 + lowhp / 100.0;
        }
        let sp = st.sum(|e| if let Effect::StrainPower(x) = e { Some(*x) } else { None });
        dmg *= 1.0 + sp * self.pl.strain_frac();
        let def = self.t.class(self.pl.hand_weapon(&self.t, a.left).0);
        let last_in_chain = !a.heavy && a.combo as usize == def.light.len() - 1;
        if last_in_chain {
            let f = st.fx.iter().filter_map(|e| if let Effect::ComboFinisher(x) = e { Some(*x) } else { None }).fold(1.0, f32::max);
            dmg *= f;
        }
        // crits: chance, and backstabs
        let tb = *self.w.body.get(t).unwrap();
        let behind = yaw_dir(tb.yaw).dot(dn) > 0.35;
        let mut crit = chance(&mut self.rng, st.crit);
        let bs = st.fx.iter().filter_map(|e| if let Effect::BackstabCrit(x) = e { Some(*x) } else { None }).fold(0.0, f32::max);
        if behind && bs > 0.0 {
            crit = true;
            dmg *= bs;
        }
        if crit {
            dmg *= 1.6;
        }
        let kind = if a.heavy { DmgKind::Heavy } else { DmgKind::Melee };
        let poise = mv.poise * if a.heavy { 1.0 + a.charge } else { 1.0 };
        let dealt = self.deal(t, Some(pe), dmg, poise, dn, kind, mv.knock * if a.heavy { 1.0 + a.charge } else { 1.0 });
        if dealt <= 0.0 {
            return;
        }
        let tp = tb.center();
        if crit {
            self.parts.burst(Fx::Sparks, tp, dn, 14, 1.4);
            self.float_text(tp + Vec3::Y * 0.6, "CRIT", [1.0, 0.85, 0.3]);
        }
        // Strain only falls by landing melee hits
        let melee_add = st.sum(|e| if let Effect::MeleeStrain(x) = e { Some(*x) } else { None });
        if melee_add > 0.0 {
            self.pl.strain += melee_add;
        } else {
            self.pl.strain = (self.pl.strain - 6.0 * st.strain_drain * if a.heavy { 1.6 } else { 1.0 }).max(0.0);
        }
        let pa = self.w.actor.get_mut(pe).unwrap();
        pa.hp = (pa.hp + dealt * st.lifesteal).min(pa.max_hp);
        // on-hit effects
        let burn = st.sum(|e| if let Effect::HitBurn(x) = e { Some(*x) } else { None }) + if self.w.actor.get(pe).unwrap().kindle > 0.0 { self.pl.kindle_dps } else { 0.0 };
        let rot = st.sum(|e| if let Effect::HitRot(x) = e { Some(*x) } else { None });
        let slow = st.sum(|e| if let Effect::HitSlow(x) = e { Some(*x) } else { None });
        if let Some(ta) = self.w.actor.get_mut(t) {
            if burn > 0.0 && !ta.fire_immune {
                ta.burn = 4.0;
                ta.burn_dps = ta.burn_dps.max(burn);
            }
            if rot > 0.0 {
                ta.rot = 5.0;
                ta.rot_dps = ta.rot_dps.max(rot);
            }
            if slow > 0.0 {
                ta.slow = ta.slow.max(0.5);
            }
        }
        let chain = st.sum(|e| if let Effect::HitChain(x) = e { Some(*x) } else { None });
        if chain > 0.0 {
            let mut n = 0;
            let mut others = Vec::new();
            self.hash.query(tb.pos, 5.0, |id, p, _| {
                if id != t.id && id != pe.id {
                    others.push((id, p));
                }
            });
            for (id, p) in others {
                if n >= 2 {
                    break;
                }
                let o = self.w.ent(id);
                if self.can_hurt(Some(pe), Team::Player, o) && self.w.actor.get(o).map(|a| a.team == Team::Infected).unwrap_or(false) {
                    n += 1;
                    self.deal(o, Some(pe), dealt * chain, 4.0, flat(p - tb.pos).normalize_or_zero(), DmgKind::Spell, 0.5);
                    self.tethers_fx.push((tp, p + Vec3::Y, 0.2, [0.6, 0.8, 2.5]));
                }
            }
        }
        let shard = st.sum(|e| if let Effect::HitShard(x) = e { Some(*x) } else { None });
        if shard > 0.0 && !a.heavy {
            let pp = self.ppos() + Vec3::Y * 1.2;
            if let Some(o) = self.nearest_hostile(tp, 12.0, Team::Player).filter(|o| *o != t).or(Some(t)) {
                let op = self.w.body.get(o).unwrap().center();
                self.spawn_proj(pp, (op - pp).normalize_or_zero() * 30.0, ProjKind::BoneShard, shard * st.dmg, 5.0, Team::Player, Some(pe), 0.0);
            }
        }
        if a.heavy && a.charge > 0.95 {
            let shock = st.sum(|e| if let Effect::HeavyShock(x) = e { Some(*x) } else { None });
            if shock > 0.0 {
                let p = self.ppos();
                self.spawn_zone(p, ZoneKind::Ring, 6.0, shock * st.dmg, 0.6, Team::Player, Some(pe));
                self.shake(0.6);
            }
            let spikes = st.sum(|e| if let Effect::HeavySpikes(x) = e { Some(*x) } else { None });
            if spikes > 0.0 {
                let p = self.ppos();
                let mut z = self.make_zone(p, ZoneKind::Spikes, 1.0, spikes * st.dmg, 1.0, Team::Player, Some(pe));
                z.dir = yaw_dir(self.player_body().yaw);
                z.len = 8.0;
                self.add_zone(p, z);
            }
        }
        self.pl.hits_landed += 1;
        if st.has(|e| matches!(e, Effect::Spellblade)) && self.pl.hits_landed % 3 == 0 {
            let sp = self.pl.spells(&self.t);
            if !sp.is_empty() {
                let s = sp[self.pl.spell_idx % sp.len()];
                self.cast_spell_free(pe, s);
            }
        }
        if self.w.actor.get(t).map(|a| a.team == Team::Military).unwrap_or(false) {
            self.anger_squad(t);
        }
        // hit-stop scaled by weight
        self.hitstop = self.hitstop.max(if a.heavy { 0.09 + a.charge * 0.05 } else { 0.05 });
        self.shake(if a.heavy { 0.35 } else { 0.15 });
        self.pl.last_hit_target = Some((t, 4.0));
    }

    /// The single entry point for all damage. Returns damage actually dealt.
    pub fn deal(&mut self, t: Entity, src: Option<Entity>, amount: f32, poise: f32, dir: Vec3, kind: DmgKind, knock: f32) -> f32 {
        let Some(ta) = self.w.actor.get(t).cloned() else { return 0.0 };
        if !ta.alive || amount <= 0.0 {
            return 0.0;
        }
        let tb = *self.w.body.get(t).unwrap();
        let is_player = t == self.player;
        let mut amount = amount;
        let mut poise = poise;
        let src_body: Option<Body> = src.and_then(|s| self.w.body.get(s).copied());
        if is_player {
            let act = ta.act.clone();
            // dodge invulnerability
            if act.kind == ActKind::Dodge && act.t > 0.03 && act.t < act.dur * 0.72 && kind != DmgKind::True && kind != DmgKind::Rot {
                let r = self.pl.stats.sum(|e| if let Effect::TimeReflex(x) = e { Some(*x) } else { None });
                if r > 0.0 && self.pl.reflex <= 0.0 && matches!(kind, DmgKind::Melee | DmgKind::Heavy | DmgKind::Proj) {
                    self.pl.reflex = 6.0;
                    self.slowmo = r;
                    self.audio.play(Sfx::Rift, 0.4, 0.5);
                }
                return 0.0;
            }
            if act.kind == ActKind::Rupture {
                return 0.0;
            }
            // blocking
            let facing = src_body.map(|sb| flat(sb.pos - tb.pos).normalize_or_zero().dot(yaw_dir(tb.yaw)) > 0.1).unwrap_or(-dir.dot(yaw_dir(tb.yaw)) > 0.1);
            if self.pl.blocking && facing && matches!(kind, DmgKind::Melee | DmgKind::Heavy | DmgKind::Proj) {
                let window = 0.18 * self.pl.stats.parry_window;
                let p = tb.center();
                if self.pl.block_t < window && self.pl.parry_cd <= 0.0 {
                    self.perfect_block(src, p);
                    return 0.0;
                }
                let class = self.pl.guard_class(&self.t);
                let def = self.t.class(class);
                let red = (def.block * self.pl.stats.block_power).clamp(0.0, 1.0);
                let cost = amount * def.stability * 0.9 * if kind == DmgKind::Heavy { 1.5 } else { 1.0 };
                self.pl.stamina -= cost;
                self.pl.stam_delay = 0.8;
                let thorns = self.pl.stats.sum(|e| if let Effect::BlockThorns(x) = e { Some(*x) } else { None });
                if let (Some(s), true) = (src, thorns > 0.0) {
                    self.deal(s, Some(t), thorns, 5.0, -dir, DmgKind::True, 0.0);
                }
                self.parts.burst(Fx::Sparks, p + yaw_dir(tb.yaw) * 0.4, -dir, 10, 1.0);
                self.audio.play_at(Sfx::Block, p, 0.6, rf(&mut self.rng, 0.9, 1.1));
                self.hitstop = self.hitstop.max(0.04);
                if self.pl.stamina < 0.0 {
                    // guard break
                    self.pl.stamina = 0.0;
                    self.pl.blocking = false;
                    let a = self.w.actor.get_mut(t).unwrap();
                    a.act = Act::new(ActKind::Stagger, 0.9);
                    self.msg("GUARD BROKEN");
                    amount *= 1.0 - red * 0.5;
                } else {
                    amount *= 1.0 - red;
                    poise *= 0.3;
                }
                if let Some(b) = self.w.body.get_mut(t) {
                    b.push += dir * knock * 0.3;
                }
                if amount < 0.5 {
                    return 0.0;
                }
            }
            if kind == DmgKind::Fire {
                amount *= 1.0 - self.pl.stats.fire_res;
            }
        } else if let Some(f) = self.w.foe.get(t).cloned() {
            if f.affixes.contains(&AffixKind::Phasing) && ta.dissolve > 0.4 {
                self.parts.burst(Fx::Rift, tb.center(), dir, 4, 0.5);
                return 0.0;
            }
            if kind == DmgKind::Fire && ta.team == Team::Infected {
                if ta.fire_immune {
                    return 0.0;
                }
                amount *= 1.5;
            }
            if f.affixes.contains(&AffixKind::Mirrored) && matches!(kind, DmgKind::Spell | DmgKind::Fire) && src == Some(self.player) {
                let back = amount * 0.4;
                self.deal(self.player, Some(t), back, 0.0, -dir, DmgKind::True, 0.0);
                self.parts.burst(Fx::Rift, tb.center(), -dir, 8, 1.0);
                amount *= 0.6;
            }
            // shielded archetypes guard their front
            if self.w.ai.get(t).map(|ai| ai.kind == AiKind::Shielded).unwrap_or(false) && ta.act.kind == ActKind::Idle && -dir.dot(yaw_dir(tb.yaw)) > 0.35 && matches!(kind, DmgKind::Melee | DmgKind::Proj) {
                amount *= 0.15;
                poise *= 0.4;
                self.parts.burst(Fx::Sparks, tb.center() - dir * 0.5, -dir, 8, 1.0);
                self.audio.play_at(Sfx::Block, tb.pos, 0.5, 0.8);
            }
            if f.plates > 0.0 {
                let absorbed = (amount * 0.7).min(f.plates);
                amount -= absorbed;
                let f = self.w.foe.get_mut(t).unwrap();
                f.plates -= absorbed;
                self.parts.burst(Fx::Bone, tb.center(), dir, 4, 1.0);
                if f.plates <= 0.0 {
                    self.parts.burst(Fx::Bone, tb.center(), dir, 30, 2.0);
                    self.audio.play_at(Sfx::Rupture, tb.pos, 0.5, 1.6);
                    if let Some(r) = self.w.rig.get_mut(t) {
                        r.plan.growths.retain(|g| g.kind != crate::data::GrowthKind::Plates);
                    }
                    poise += 40.0;
                }
            }
            if f.mimic {
                self.reveal_mimic(t);
            }
        }
        let ta = self.w.actor.get(t).unwrap();
        amount *= 1.0 - ta.ward.min(0.6);
        if !matches!(kind, DmgKind::True | DmgKind::Rot) {
            let armor = ta.armor + ta.armor_buff + if is_player { self.strain_armor() } else { 0.0 };
            amount *= 100.0 / (100.0 + armor.max(0.0) * 1.5);
        }
        // execute
        if src == Some(self.player) && !is_player {
            let ex = self.pl.stats.fx.iter().filter_map(|e| if let Effect::Execute(x) = e { Some(*x) } else { None }).fold(0.0, f32::max);
            let boss = self.w.foe.get(t).map(|f| f.boss.is_some()).unwrap_or(false);
            if ex > 0.0 && !boss && ta.hp - amount < ta.max_hp * ex {
                amount = ta.hp + 1.0;
                self.float_text(tb.center() + Vec3::Y * 0.5, "EXECUTED", [1.0, 0.3, 0.3]);
            }
        }
        // tethered infected share wounds
        if !is_player && self.w.foe.get(t).map(|f| f.affixes.contains(&AffixKind::Tethered)).unwrap_or(false) && kind != DmgKind::True {
            let mut mates = Vec::new();
            self.hash.query(tb.pos, 8.0, |id, p, _| {
                if id != t.id {
                    mates.push((id, p));
                }
            });
            let mates: Vec<(Entity, Vec3)> = mates.into_iter().map(|(id, p)| (self.w.ent(id), p)).filter(|(e, _)| self.w.actor.get(*e).map(|a| a.alive && a.team == Team::Infected).unwrap_or(false)).take(3).collect();
            if !mates.is_empty() {
                let share = amount * 0.6 / mates.len() as f32;
                amount *= 0.4;
                for (m, p) in mates {
                    self.tethers_fx.push((tb.center(), p + Vec3::Y, 0.25, [1.5, 0.3, 0.6]));
                    self.deal(m, src, share, 0.0, dir, DmgKind::True, 0.0);
                }
            }
        }
        // apply
        let ta = self.w.actor.get_mut(t).unwrap();
        ta.hp -= amount;
        if !matches!(kind, DmgKind::Rot | DmgKind::Fire | DmgKind::True) || amount > 5.0 {
            ta.flash = 1.0;
        }
        ta.since_hit = 0.0;
        ta.hit_dir = dir;
        let lethal = ta.hp <= 0.0;
        let mut staggered = false;
        if !matches!(kind, DmgKind::Rot | DmgKind::Fire) {
            ta.poise -= poise;
            if ta.poise <= 0.0 && !matches!(ta.act.kind, ActKind::Opened | ActKind::Rupture | ActKind::Seized) {
                ta.poise = ta.max_poise;
                let boss = self.w.foe.get(t).map(|f| f.boss.is_some()).unwrap_or(false);
                let ta = self.w.actor.get_mut(t).unwrap();
                if !(boss && ta.act.kind == ActKind::Special) {
                    ta.act = Act::new(ActKind::Stagger, if is_player { 0.5 } else if boss { 1.2 } else { 0.85 });
                    ta.act.dir = dir;
                    staggered = true;
                }
            }
        }
        let mass = self.w.body.get(t).unwrap().mass;
        if let Some(b) = self.w.body.get_mut(t) {
            b.push += dir * knock * (if staggered { 2.0 } else { 1.0 }) / mass.max(0.3);
        }
        // feedback
        let c = tb.center();
        let blood = match self.w.actor.get(t).unwrap().team {
            Team::Infected => Fx::Ichor,
            _ => Fx::Blood,
        };
        if !matches!(kind, DmgKind::Rot | DmgKind::Fire) || amount > 6.0 {
            self.parts.burst(blood, c, dir, (3.0 + amount * 0.25).min(18.0) as u32, 1.0);
            let heavy = matches!(kind, DmgKind::Heavy | DmgKind::Rupture) || amount > 30.0;
            self.audio.play_at(if heavy { Sfx::HitHeavy } else { Sfx::HitFlesh }, c, 0.7, rf(&mut self.rng, 0.85, 1.15));
            if matches!(kind, DmgKind::Melee | DmgKind::Heavy) {
                self.parts.burst(Fx::Sparks, c, dir, 3, 0.6);
            }
        }
        if amount >= 1.0 && (src == Some(self.player) || is_player) && !matches!(kind, DmgKind::Rot | DmgKind::Fire) {
            self.float_text(c + Vec3::Y * 0.4, &format!("{:.0}", amount), if is_player { [1.0, 0.3, 0.3] } else { [1.0, 1.0, 1.0] });
        }
        if is_player {
            self.hurt_flash = (self.hurt_flash + amount / 40.0).min(0.8);
            self.shake(0.25 + (amount / 60.0).min(0.6));
            self.audio.play(Sfx::Hurt, 0.4, rf(&mut self.rng, 0.9, 1.1));
            self.player_hurt_effects(src, amount, lethal);
        } else if let Some(s) = src {
            // thorns
            if let Some(f) = self.w.foe.get(t) {
                if f.affixes.contains(&AffixKind::Thorned) && matches!(kind, DmgKind::Melee | DmgKind::Heavy) {
                    let back = amount * 0.15;
                    self.deal(s, None, back, 0.0, -dir, DmgKind::True, 0.0);
                }
            }
            if s != self.player {
                if let Some(ai) = self.w.ai.get_mut(t) {
                    if ai.target.is_none() || chance(&mut self.rng, 0.3) {
                        ai.target = Some(s);
                    }
                }
            } else if let Some(ai) = self.w.ai.get_mut(t) {
                ai.target = Some(s);
                ai.alert = 10.0;
            }
        }
        let ta = self.w.actor.get(t).unwrap();
        if ta.hp <= 0.0 {
            if is_player {
                if self.pl.stats.has(|e| matches!(e, Effect::SecondWind)) && !self.pl.life.second_wind_used {
                    self.pl.life.second_wind_used = true;
                    let ta = self.w.actor.get_mut(t).unwrap();
                    ta.hp = ta.max_hp * 0.3;
                    self.msg("SECOND WIND");
                    self.parts.burst(Fx::Heal, tb.pos, Vec3::ZERO, 40, 1.0);
                    self.slowmo = 1.0;
                    return amount;
                }
                self.player_died(src);
            } else {
                self.kill(t, src);
            }
        }
        amount
    }

    fn strain_armor(&self) -> f32 {
        let a = self.pl.stats.sum(|e| if let Effect::StrainArmor(x) = e { Some(*x) } else { None });
        a * self.pl.strain_frac()
    }

    fn player_hurt_effects(&mut self, _src: Option<Entity>, amount: f32, lethal: bool) {
        if lethal {
            return;
        }
        let pe = self.player;
        let p = self.ppos();
        let st = self.pl.stats.clone();
        let larva = st.sum(|e| if let Effect::HurtLarvae(x) = e { Some(*x) } else { None });
        if larva > 0.0 && chance(&mut self.rng, larva) {
            let o = rand_dir(&mut self.rng);
            self.spawn_brood(p + o, 14.0);
        }
        let burst = st.sum(|e| if let Effect::HurtBurst(x) = e { Some(*x) } else { None });
        if burst > 0.0 && amount > st.max_hp * 0.08 {
            self.aoe(p, 3.5, burst * st.dmg, 30.0, Team::Player, Some(pe), DmgKind::Fire);
            self.parts.burst(Fx::Fire, p + Vec3::Y, Vec3::ZERO, 30, 1.5);
            self.audio.play_at(Sfx::Explode, p, 0.6, 1.3);
        }
        let un = st.sum(|e| if let Effect::Unstable(x) = e { Some(*x) } else { None });
        if un > 0.0 && chance(&mut self.rng, 0.25) {
            self.aoe(p, 3.0, un * st.dmg, 30.0, Team::Player, Some(pe), DmgKind::Spell);
            let ac = self.w.actor.get_mut(pe).unwrap();
            ac.hp = (ac.hp - un * 0.25).max(1.0);
            self.parts.burst(Fx::Fire, p + Vec3::Y, Vec3::ZERO, 24, 1.2);
            self.audio.play_at(Sfx::Explode, p, 0.5, 1.4);
        }
    }

    fn perfect_block(&mut self, src: Option<Entity>, p: Vec3) {
        let pe = self.player;
        self.pl.parry_cd = 0.35;
        self.parts.burst(Fx::Sparks, p + yaw_dir(self.player_body().yaw) * 0.5, Vec3::Y, 30, 1.6);
        self.parts.burst(Fx::Flash, p + yaw_dir(self.player_body().yaw) * 0.5, Vec3::ZERO, 1, 1.0);
        self.audio.play_at(Sfx::Parry, p, 0.9, 1.0);
        self.hitstop = self.hitstop.max(0.14);
        self.shake(0.3);
        self.flash = 0.15;
        if let Some(s) = src {
            let boss = self.w.foe.get(s).map(|f| f.boss.is_some()).unwrap_or(false);
            if let Some(sa) = self.w.actor.get_mut(s) {
                if sa.team != Team::Player && self.w.proj.get(s).is_none() {
                    if boss {
                        sa.poise -= 60.0;
                        if sa.poise <= 0.0 {
                            sa.poise = sa.max_poise;
                            sa.act = Act::new(ActKind::Opened, 2.2);
                        } else {
                            sa.act = Act::new(ActKind::Stagger, 0.8);
                        }
                    } else {
                        sa.act = Act::new(ActKind::Opened, 2.0);
                    }
                }
            }
            if let Some(b) = self.w.body.get_mut(s) {
                b.push += yaw_dir(b.yaw) * -3.0;
            }
        }
        self.float_text(p + Vec3::Y * 0.6, "PERFECT BLOCK", [0.9, 0.95, 1.0]);
        let st = self.pl.stats.clone();
        if st.has(|e| matches!(e, Effect::ParryTendril(_))) {
            self.pl.tendril = 6.0;
        }
        let nova = st.sum(|e| if let Effect::ParryNova(x) = e { Some(*x) } else { None });
        if nova > 0.0 {
            let pp = self.ppos();
            self.aoe(pp, 4.0, nova * st.dmg, 40.0, Team::Player, Some(pe), DmgKind::Spell);
            self.spawn_zone(pp, ZoneKind::Ring, 4.5, 0.0, 0.4, Team::Player, Some(pe));
            self.audio.play_at(Sfx::Shriek, pp, 0.5, 1.4);
        }
        let vent = st.sum(|e| if let Effect::ParryStrain(x) = e { Some(*x) } else { None });
        self.pl.strain = (self.pl.strain - vent).max(0.0);
    }

    /// Rupture: finisher on an opened enemy.
    pub fn try_rupture(&mut self) -> bool {
        let pe = self.player;
        let pb = self.player_body();
        let mut best: Option<(Entity, f32)> = None;
        self.hash.query(pb.pos, 3.0, |id, p, _| {
            let e = self.w.ent(id);
            if let Some(a) = self.w.actor.get(e) {
                if a.alive && a.act.kind == ActKind::Opened && a.team != Team::Player {
                    let d = (p - pb.pos).length();
                    if best.map(|b| d < b.1).unwrap_or(true) {
                        best = Some((e, d));
                    }
                }
            }
        });
        let Some((t, _)) = best else { return false };
        let tp = self.w.body.get(t).unwrap().pos;
        let mut act = Act::new(ActKind::Rupture, 1.0);
        act.target = Some(t);
        act.dir = flat(tp - pb.pos).normalize_or_zero();
        self.w.actor.get_mut(pe).unwrap().act = act;
        if let Some(b) = self.w.body.get_mut(pe) {
            b.yaw = crate::util::dir_yaw(flat(tp - pb.pos));
        }
        if let Some(ta) = self.w.actor.get_mut(t) {
            ta.act.t = 0.0;
            ta.act.dur = 1.2;
        }
        self.slowmo = self.slowmo.max(0.4);
        true
    }

    fn rupture_hit(&mut self, e: Entity, target: Option<Entity>) {
        let Some(t) = target else { return };
        let Some(ta) = self.w.actor.get(t) else { return };
        if !ta.alive {
            return;
        }
        let st = self.pl.stats.clone();
        let (_, power) = self.pl.hand_weapon(&self.t, false);
        let boss = self.w.foe.get(t).map(|f| f.boss.is_some()).unwrap_or(false);
        let dmg = power * st.dmg * 5.0 + ta.max_hp * if boss { 0.06 } else { 0.35 };
        let tb = *self.w.body.get(t).unwrap();
        let dir = flat(tb.pos - self.ppos()).normalize_or_zero();
        self.deal(t, Some(e), dmg, 100.0, dir, DmgKind::Rupture, 4.0);
        self.parts.burst(Fx::Ichor, tb.center(), Vec3::Y, 50, 2.5);
        self.parts.burst(Fx::Bone, tb.center(), Vec3::Y, 16, 2.0);
        self.audio.play_at(Sfx::Rupture, tb.pos, 1.0, 1.0);
        self.hitstop = 0.2;
        self.shake(0.8);
        self.float_text(tb.center() + Vec3::Y, "RUPTURE", [1.0, 0.25, 0.4]);
        self.pl.strain = (self.pl.strain - 20.0 * st.strain_drain).max(0.0);
        let nova = st.sum(|e| if let Effect::RuptureNova(x) = e { Some(*x) } else { None });
        if nova > 0.0 {
            self.aoe(tb.pos, 5.0, nova * st.dmg, 50.0, Team::Player, Some(e), DmgKind::Spell);
            self.parts.burst(Fx::Spore, tb.center(), Vec3::ZERO, 50, 3.0);
        }
        let heal = st.sum(|e| if let Effect::RuptureHeal(x) = e { Some(*x) } else { None });
        if heal > 0.0 {
            let a = self.w.actor.get_mut(e).unwrap();
            a.hp = (a.hp + heal).min(a.max_hp);
        }
        let vent = st.sum(|e| if let Effect::RuptureStrain(x) = e { Some(*x) } else { None });
        self.pl.strain = (self.pl.strain - vent).max(0.0);
    }

    /// Damage everything hostile to `team` in a radius.
    pub fn aoe(&mut self, p: Vec3, r: f32, dmg: f32, poise: f32, team: Team, src: Option<Entity>, kind: DmgKind) -> u32 {
        let mut hits = Vec::new();
        self.hash.query(p, r, |id, bp, _| hits.push((id, bp)));
        let mut n = 0;
        for (id, bp) in hits {
            let e = self.w.ent(id);
            let Some(a) = self.w.actor.get(e) else { continue };
            // player area attacks only hurt infected (spares soldiers and civilians)
            let ok = if team == Team::Player || team == Team::Ally { a.team == Team::Infected } else { self.can_hurt(src, team, e) };
            if ok && Some(e) != src {
                let d = flat(bp - p).normalize_or_zero();
                self.deal(e, src, dmg, poise, if d.length_squared() < 0.1 { Vec3::X } else { d }, kind, 2.0);
                n += 1;
            }
        }
        n
    }

    pub fn pull_all(&mut self, p: Vec3, r: f32, force: f32, team: Team) {
        let mut hits = Vec::new();
        self.hash.query(p, r, |id, bp, _| hits.push((id, bp)));
        for (id, bp) in hits {
            let e = self.w.ent(id);
            let ok = self.w.actor.get(e).map(|a| a.alive && hostile(team, a.team)).unwrap_or(false);
            let boss = self.w.foe.get(e).map(|f| f.boss.is_some()).unwrap_or(false);
            if ok && !boss {
                if let Some(b) = self.w.body.get_mut(e) {
                    b.push += flat(p - bp).normalize_or_zero() * force / b.mass.max(0.5);
                }
            }
        }
    }

    // ---- projectiles ----------------------------------------------------------------------
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_proj(&mut self, p: Vec3, vel: Vec3, kind: ProjKind, dmg: f32, poise: f32, team: Team, owner: Option<Entity>, blast: f32) -> Entity {
        let e = self.w.spawn();
        let mut b = Body::new(p, 0.15, 0.3);
        b.solid = false;
        self.w.body.insert(e, b);
        let ttl = if kind == ProjKind::Bullet { 0.6 } else { 3.5 };
        self.w.proj.insert(e, Proj { kind, vel, dmg, poise, team, owner, ttl, radius: if kind == ProjKind::BossOrb { 0.5 } else { 0.3 }, blast, homing: None, reflected: false, spell: matches!(kind, ProjKind::Ember | ProjKind::Shard) });
        e
    }

    pub fn proj_tick(&mut self, dt: f32) {
        let ids: Vec<u32> = self.w.proj.ids().collect();
        for id in ids {
            let e = self.w.ent(id);
            let mut pr = self.w.proj.get(e).unwrap().clone();
            let p0 = self.w.body.get(e).unwrap().pos;
            pr.ttl -= dt;
            if pr.kind == ProjKind::Acid {
                pr.vel.y -= 9.0 * dt;
            }
            if let Some(h) = pr.homing {
                if let Some(hb) = self.w.body.get(h) {
                    let want = (hb.center() - p0).normalize_or_zero() * pr.vel.length();
                    pr.vel = pr.vel.lerp(want, (dt * 4.0).min(1.0));
                }
            }
            let step = pr.vel * dt;
            let len = step.length();
            let mut hit_pos: Option<Vec3> = None;
            let mut hit_ent: Option<Entity> = None;
            if len > 0.0 {
                if let Some(tw) = self.area.raycast(p0, step / len, len, OPAQUE) {
                    hit_pos = Some(p0 + step / len * tw);
                }
            }
            if p0.y + step.y < 0.05 {
                hit_pos = Some(Vec3::new(p0.x + step.x, 0.05, p0.z + step.z));
            }
            // body hits along the segment
            let mid = p0 + step * 0.5;
            let mut cands = Vec::new();
            self.hash.query(mid, len * 0.5 + pr.radius + 0.6, |bid, bp, br| cands.push((bid, bp, br)));
            let mut best = f32::MAX;
            for (bid, bp, br) in cands {
                let be = self.w.ent(bid);
                if Some(be) == pr.owner {
                    continue;
                }
                if !self.can_hurt(pr.owner, pr.team, be) {
                    continue;
                }
                let bb = self.w.body.get(be).unwrap();
                // closest approach on the XZ plane, then height check
                let a = Vec3::new(p0.x, 0.0, p0.z);
                let s = Vec3::new(step.x, 0.0, step.z);
                let c = Vec3::new(bp.x, 0.0, bp.z);
                let tt = if s.length_squared() > 1e-6 { ((c - a).dot(s) / s.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
                let q = p0 + step * tt;
                let d = (Vec3::new(q.x, 0.0, q.z) - c).length();
                if d < br + pr.radius && q.y > bb.pos.y - 0.2 && q.y < bb.pos.y + bb.height + 0.3 && tt < best {
                    best = tt;
                    hit_ent = Some(be);
                    hit_pos = Some(q);
                }
            }
            let mut dead = pr.ttl <= 0.0;
            if let Some(he) = hit_ent {
                // player can reflect with a perfect block or BlockReflect
                let refl = he == self.player && self.pl.blocking && (self.pl.block_t < 0.18 * self.pl.stats.parry_window || self.pl.stats.has(|e| matches!(e, Effect::BlockReflect))) && !pr.reflected;
                let mirrored = self.w.foe.get(he).map(|f| f.affixes.contains(&AffixKind::Mirrored)).unwrap_or(false) && pr.team == Team::Player && pr.spell && !pr.reflected;
                if refl || mirrored {
                    pr.vel = -pr.vel * 1.2;
                    pr.team = if refl { Team::Player } else { Team::Infected };
                    pr.owner = Some(he);
                    pr.reflected = true;
                    pr.homing = None;
                    pr.ttl = 2.0;
                    self.audio.play_at(Sfx::Parry, p0, 0.6, 1.3);
                    self.parts.burst(Fx::Sparks, p0, -pr.vel.normalize_or_zero(), 12, 1.0);
                    self.w.proj.insert(e, pr);
                    continue;
                }
                let dir = flat(pr.vel).normalize_or_zero();
                let kind = match pr.kind {
                    ProjKind::Ember => DmgKind::Fire,
                    ProjKind::Shard => DmgKind::Spell,
                    _ => DmgKind::Proj,
                };
                self.deal(he, pr.owner, pr.dmg, pr.poise, dir, kind, 1.0);
                if pr.kind == ProjKind::Acid {
                    if let Some(a) = self.w.actor.get_mut(he) {
                        a.rot = 3.0;
                        a.rot_dps = a.rot_dps.max(pr.dmg * 0.15);
                    }
                }
                if pr.kind == ProjKind::Bullet && he != self.player {
                    if let Some(ai) = self.w.ai.get_mut(he) {
                        if ai.target.is_none() {
                            ai.target = pr.owner;
                        }
                    }
                }
                dead = true;
            } else if hit_pos.is_some() {
                dead = true;
            }
            let np = hit_pos.unwrap_or(p0 + step);
            if let Some(b) = self.w.body.get_mut(e) {
                b.prev = p0;
                b.pos = np;
            }
            // trails
            match pr.kind {
                ProjKind::Acid => self.parts.burst(Fx::Acid, np, Vec3::ZERO, 1, 0.2),
                ProjKind::Ember => self.parts.burst(Fx::Fire, np, Vec3::ZERO, 2, 0.6),
                ProjKind::Shard | ProjKind::BossOrb => self.parts.burst(Fx::Rift, np, Vec3::ZERO, 1, 0.3),
                ProjKind::BoneShard => self.parts.burst(Fx::Bone, np, Vec3::ZERO, 1, 0.2),
                ProjKind::Bullet => {}
            }
            if dead {
                if pr.blast > 0.0 && pr.ttl > 0.0 || matches!(pr.kind, ProjKind::Acid | ProjKind::BossOrb) {
                    let r = pr.blast.max(1.2);
                    let kind = if pr.kind == ProjKind::Ember { DmgKind::Fire } else { DmgKind::Spell };
                    if pr.blast > 0.0 {
                        self.aoe(np, r, pr.dmg * 0.6, pr.poise, pr.team, pr.owner, kind);
                    }
                    match pr.kind {
                        ProjKind::Acid => {
                            self.parts.burst(Fx::Acid, np, Vec3::Y, 16, 1.0);
                            self.spawn_zone(np, ZoneKind::Acid, 1.6, pr.dmg * 0.35, 3.0, pr.team, pr.owner);
                        }
                        ProjKind::Ember => {
                            self.parts.burst(Fx::Fire, np, Vec3::Y, 24, 1.4);
                            self.parts.burst(Fx::Flash, np, Vec3::ZERO, 1, 1.0);
                            self.audio.play_at(Sfx::Explode, np, 0.6, 1.5);
                            self.lights_temp.push((np, Vec3::new(5.0, 2.0, 0.5), 10.0, 0.15));
                        }
                        _ => self.parts.burst(Fx::Rift, np, Vec3::Y, 16, 1.0),
                    }
                } else if pr.kind == ProjKind::Bullet {
                    self.parts.burst(Fx::Sparks, np, -pr.vel.normalize_or_zero(), 3, 0.5);
                    self.parts.burst(Fx::Dust, np, Vec3::Y, 1, 0.3);
                }
                self.w.kill(e);
            } else {
                self.w.proj.insert(e, pr);
            }
        }
    }

    // ---- zones -------------------------------------------------------------------------------
    #[allow(clippy::too_many_arguments)]
    pub fn make_zone(&self, _p: Vec3, kind: ZoneKind, radius: f32, dps: f32, dur: f32, team: Team, owner: Option<Entity>) -> Zone {
        Zone { kind, radius, dps, t: 0.0, dur, team, owner, dir: Vec3::Z, len: 0.0, delay: 0.0, done: false, permanent: false }
    }
    pub fn add_zone(&mut self, p: Vec3, z: Zone) -> Entity {
        let e = self.w.spawn();
        let mut b = Body::new(p, 0.1, 0.1);
        b.solid = false;
        self.w.body.insert(e, b);
        self.w.zone.insert(e, z);
        e
    }
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_zone(&mut self, p: Vec3, kind: ZoneKind, radius: f32, dps: f32, dur: f32, team: Team, owner: Option<Entity>) -> Entity {
        let z = self.make_zone(p, kind, radius, dps, dur, team, owner);
        self.add_zone(p, z)
    }

    pub fn zones_tick(&mut self, dt: f32) {
        let ids: Vec<u32> = self.w.zone.ids().collect();
        let pe = self.player;
        for id in ids {
            let e = self.w.ent(id);
            let mut z = self.w.zone.get(e).unwrap().clone();
            let p = self.w.body.get(e).unwrap().pos;
            let prev_t = z.t;
            z.t += dt;
            let affects = |g: &Game, t: Entity| -> bool {
                let Some(a) = g.w.actor.get(t) else { return false };
                if !a.alive {
                    return false;
                }
                match z.team {
                    Team::Player | Team::Ally => a.team == Team::Infected,
                    _ => hostile(z.team, a.team) || (z.permanent && a.team == Team::Player),
                }
            };
            let mut inside: Vec<(Entity, Vec3)> = Vec::new();
            let qr = z.radius.max(z.len);
            self.hash.query(p, qr + 0.5, |bid, bp, _| inside.push((self.w.ent(bid), bp)));
            let in_r = |bp: Vec3| flat(bp - p).length() < z.radius;
            match z.kind {
                ZoneKind::Spores | ZoneKind::Fire | ZoneKind::Acid | ZoneKind::Singing => {
                    let kind = match z.kind {
                        ZoneKind::Fire => DmgKind::Fire,
                        ZoneKind::Spores | ZoneKind::Acid => DmgKind::Rot,
                        _ => DmgKind::True,
                    };
                    for (t, bp) in &inside {
                        if in_r(*bp) && affects(self, *t) {
                            if z.kind == ZoneKind::Singing {
                                if *t == pe {
                                    self.pl.strain += z.dps * dt;
                                }
                                continue;
                            }
                            if *t == pe && z.kind == ZoneKind::Fire && self.pl.stats.has(|e| matches!(e, Effect::FireHeart)) {
                                let a = self.w.actor.get_mut(pe).unwrap();
                                a.hp = (a.hp + z.dps * 0.5 * dt).min(a.max_hp);
                                continue;
                            }
                            self.deal(*t, z.owner, z.dps * dt, 0.0, Vec3::X, kind, 0.0);
                            if z.kind == ZoneKind::Fire {
                                if let Some(a) = self.w.actor.get_mut(*t) {
                                    if !a.fire_immune {
                                        a.burn = a.burn.max(1.5);
                                        a.burn_dps = a.burn_dps.max(z.dps * 0.3);
                                    }
                                }
                            }
                        }
                    }
                    let n = (z.radius * z.radius * 0.5 * dt * 10.0) as u32 + 1;
                    for _ in 0..n.min(6) {
                        let off = rand_dir(&mut self.rng) * rf(&mut self.rng, 0.0, z.radius);
                        let fx = match z.kind {
                            ZoneKind::Fire => Fx::Fire,
                            ZoneKind::Acid => Fx::Acid,
                            ZoneKind::Singing => Fx::Strain,
                            _ => Fx::Spore,
                        };
                        if chance(&mut self.rng, 0.4) {
                            self.parts.burst(fx, p + off + Vec3::Y * 0.1, Vec3::ZERO, 1, if z.kind == ZoneKind::Spores { 0.3 } else { 0.5 });
                        }
                    }
                }
                ZoneKind::Well | ZoneKind::Vortex => {
                    for (t, bp) in &inside {
                        if in_r(*bp) && affects(self, *t) {
                            let d = flat(p - *bp);
                            let boss = self.w.foe.get(*t).map(|f| f.boss.is_some()).unwrap_or(false);
                            if !boss {
                                if let Some(b) = self.w.body.get_mut(*t) {
                                    b.push += d.normalize_or_zero() * z.len.max(6.0) * dt * 3.0;
                                }
                            }
                            if z.dps > 0.0 {
                                self.deal(*t, z.owner, z.dps * dt, 0.0, Vec3::X, DmgKind::Spell, 0.0);
                            }
                        }
                    }
                    self.parts.burst(Fx::Rift, p + rand_dir(&mut self.rng) * z.radius + Vec3::Y * 0.5, Vec3::ZERO, 2, 0.3);
                    self.distorts_temp.push((p + Vec3::Y, z.radius * 0.5, 0.6, 1.0));
                }
                ZoneKind::Tear => {
                    let a = p - z.dir * z.len * 0.5;
                    for (t, bp) in &inside {
                        let s = z.dir;
                        let tt = (*bp - a).dot(s).clamp(0.0, z.len);
                        let d = flat(*bp - (a + s * tt)).length();
                        if d < 0.9 && affects(self, *t) {
                            self.deal(*t, z.owner, z.dps * dt, 6.0 * dt, s, DmgKind::Spell, 0.0);
                        }
                    }
                    for _ in 0..3 {
                        let q = a + z.dir * rf(&mut self.rng, 0.0, z.len) + Vec3::Y * rf(&mut self.rng, 0.2, 2.0);
                        self.parts.burst(Fx::Rift, q, Vec3::ZERO, 1, 0.3);
                    }
                    self.distorts_temp.push((p + Vec3::Y, z.len * 0.4, 0.5, 0.2));
                }
                ZoneKind::Stasis => {
                    for (t, bp) in &inside {
                        if in_r(*bp) && affects(self, *t) {
                            if let Some(a) = self.w.actor.get_mut(*t) {
                                a.tscale = a.tscale.min(0.3);
                            }
                        }
                    }
                    self.distorts_temp.push((p + Vec3::Y, z.radius, 0.25, -0.5));
                    if chance(&mut self.rng, 0.5) {
                        self.parts.burst(Fx::Rift, p + rand_dir(&mut self.rng) * z.radius + Vec3::Y * rf(&mut self.rng, 0.0, 2.0), Vec3::ZERO, 1, 0.2);
                    }
                }
                ZoneKind::Blast => {
                    if prev_t < z.delay && z.t >= z.delay {
                        let kind = if z.team == Team::Player { DmgKind::Fire } else { DmgKind::Heavy };
                        if z.team == Team::Player {
                            self.aoe(p, z.radius, z.dps, 60.0, z.team, z.owner, kind);
                        } else {
                            for (t, bp) in &inside {
                                if in_r(*bp) && affects(self, *t) {
                                    self.deal(*t, z.owner, z.dps, 40.0, flat(*bp - p).normalize_or_zero(), DmgKind::Heavy, 4.0);
                                }
                            }
                        }
                        self.parts.burst(Fx::Fire, p + Vec3::Y * 0.3, Vec3::Y, 40, z.radius * 0.5);
                        self.parts.burst(Fx::Flash, p + Vec3::Y, Vec3::ZERO, 1, z.radius * 0.6);
                        self.parts.burst(Fx::Smoke, p, Vec3::ZERO, 8, 1.0);
                        self.audio.play_at(Sfx::Explode, p, 0.9, 0.9);
                        self.lights_temp.push((p + Vec3::Y * 2.0, Vec3::new(6.0, 3.0, 1.0), z.radius * 3.0, 0.25));
                        self.shake(0.5);
                        z.done = true;
                    }
                }
                ZoneKind::Ring => {
                    let r = z.radius * (z.t / z.dur).min(1.0);
                    let r0 = z.radius * (prev_t / z.dur).min(1.0);
                    for (t, bp) in &inside {
                        let d = flat(*bp - p).length();
                        if d >= r0 - 0.6 && d < r + 0.4 && affects(self, *t) && z.dps > 0.0 {
                            let hit_key = t.id;
                            if !z.done || hit_key != u32::MAX {
                                let already = self.ring_hits.contains(&(e.id, t.id));
                                if !already {
                                    self.ring_hits.push((e.id, t.id));
                                    self.deal(*t, z.owner, z.dps, 50.0, flat(*bp - p).normalize_or_zero(), DmgKind::Heavy, 5.0);
                                }
                            }
                        }
                    }
                    let n = (r * 3.0) as u32 + 4;
                    for i in 0..n.min(24) {
                        let a = i as f32 / n as f32 * std::f32::consts::TAU + z.t;
                        if chance(&mut self.rng, 0.5) {
                            self.parts.burst(Fx::Dust, p + Vec3::new(a.cos() * r, 0.1, a.sin() * r), Vec3::new(a.cos(), 0.3, a.sin()), 1, 1.0);
                        }
                    }
                }
                ZoneKind::Beam => {
                    // rotating beam from the owner
                    let ang = z.t * 0.9 + z.dir.x.atan2(z.dir.z);
                    let d = Vec3::new(ang.sin(), 0.0, ang.cos());
                    let origin = z.owner.and_then(|o| self.w.body.get(o)).map(|b| b.pos).unwrap_or(p);
                    if let Some(b) = self.w.body.get_mut(e) {
                        b.pos = origin;
                    }
                    let len = self.area.raycast(origin + Vec3::Y, d, 30.0, OPAQUE).unwrap_or(30.0);
                    for (t, bp) in &inside {
                        let tt = (*bp - origin).dot(d).clamp(0.0, len);
                        let dd = flat(*bp - (origin + d * tt)).length();
                        if dd < 0.9 && affects(self, *t) {
                            self.deal(*t, z.owner, z.dps * dt, 30.0 * dt, Vec3::new(-d.z, 0.0, d.x), DmgKind::Spell, 0.0);
                        }
                    }
                    for _ in 0..6 {
                        let q = origin + Vec3::Y * 1.2 + d * rf(&mut self.rng, 0.5, len);
                        self.parts.burst(Fx::Rift, q, Vec3::Y * 0.5, 1, 0.3);
                    }
                    self.beams.push((origin + Vec3::Y * 1.2, origin + Vec3::Y * 1.2 + d * len));
                }
                ZoneKind::Spikes => {
                    let n = 8;
                    for i in 0..n {
                        let k0 = i as f32 / n as f32;
                        let tt = k0 * 0.5;
                        if prev_t < tt && z.t >= tt {
                            let q = p + z.dir * (k0 * z.len + 1.0);
                            self.parts.burst(Fx::Bone, q + Vec3::Y * 0.3, Vec3::Y, 6, 1.5);
                            self.audio.play_at(Sfx::HitHeavy, q, 0.3, 1.6);
                            for (t, bp) in &inside {
                                if flat(*bp - q).length() < 1.3 && affects(self, *t) {
                                    self.deal(*t, z.owner, z.dps, 30.0, Vec3::Y, DmgKind::Spell, 2.0);
                                }
                            }
                            self.spikes_fx.push((q, 1.0));
                        }
                    }
                }
            }
            if (z.t >= z.dur && !z.permanent) || (z.done && z.kind == ZoneKind::Blast) {
                self.ring_hits.retain(|(zid, _)| *zid != e.id);
                self.w.kill(e);
            } else {
                self.w.zone.insert(e, z);
            }
        }
    }

    /// Status effects, regeneration and per-tick affix auras.
    pub fn status_tick(&mut self, dt: f32) {
        let ids: Vec<u32> = self.w.actor.ids().collect();
        for id in ids {
            let e = self.w.ent(id);
            let Some(a) = self.w.actor.get_mut(e) else { continue };
            a.flash = (a.flash - dt * 6.0).max(0.0);
            a.since_hit += dt;
            a.slow = (a.slow - dt * 0.25).max(0.0);
            a.armor_buff_t -= dt;
            if a.armor_buff_t <= 0.0 {
                a.armor_buff = 0.0;
            }
            a.kindle -= dt;
            if !a.alive {
                a.dead_t += dt;
                continue;
            }
            // poise regenerates when not under pressure
            if a.since_hit > 2.5 {
                a.poise = (a.poise + a.max_poise * 0.4 * dt).min(a.max_poise);
            }
            let mut dot = 0.0;
            let mut fire = 0.0;
            if a.burn > 0.0 {
                a.burn -= dt;
                fire = a.burn_dps * dt;
                if a.burn <= 0.0 {
                    a.burn_dps = 0.0;
                }
            }
            if a.rot > 0.0 {
                a.rot -= dt;
                dot = a.rot_dps * dt;
                if a.rot <= 0.0 {
                    a.rot_dps = 0.0;
                }
            }
            let burning = a.burn > 0.0;
            if fire > 0.0 {
                self.deal(e, None, fire, 0.0, Vec3::X, DmgKind::Fire, 0.0);
            }
            if dot > 0.0 {
                self.deal(e, None, dot, 0.0, Vec3::X, DmgKind::Rot, 0.0);
            }
            if burning && chance(&mut self.rng, 0.5) {
                if let Some(b) = self.w.body.get(e) {
                    let c = b.center();
                    self.parts.burst(Fx::Fire, c, Vec3::ZERO, 1, 0.5);
                }
            }
        }
    }

    /// Death of a non-player actor.
    pub fn kill(&mut self, e: Entity, killer: Option<Entity>) {
        let Some(a) = self.w.actor.get_mut(e) else { return };
        if !a.alive {
            return;
        }
        // Undying: rises once
        if let Some(f) = self.w.foe.get_mut(e) {
            if f.affixes.contains(&AffixKind::Undying) && !f.undying_used {
                f.undying_used = true;
                let a = self.w.actor.get_mut(e).unwrap();
                a.hp = a.max_hp * 0.4;
                a.act = Act::new(ActKind::Stagger, 1.6);
                let p = self.w.body.get(e).unwrap().center();
                self.parts.burst(Fx::Ichor, p, Vec3::Y, 30, 2.0);
                self.audio.play_at(Sfx::Shriek, p, 0.6, 0.7);
                self.float_text(p + Vec3::Y, "IT GETS UP", [1.0, 0.5, 0.5]);
                return;
            }
        }
        let a = self.w.actor.get_mut(e).unwrap();
        a.alive = false;
        a.hp = 0.0;
        a.dead_t = 0.0;
        a.act = Act::default();
        let team = a.team;
        if let Some(b) = self.w.body.get_mut(e) {
            b.solid = false;
            b.vel = Vec3::ZERO;
        }
        let b = *self.w.body.get(e).unwrap();
        self.audio.play_at(Sfx::Death, b.pos, 0.6, rf(&mut self.rng, 0.8, 1.3));
        self.parts.burst(if team == Team::Infected { Fx::Ichor } else { Fx::Blood }, b.center(), Vec3::Y, 24, 1.6);
        let sz = self.w.rig.get(e).map(|r| r.plan.scale).unwrap_or(1.0) * crate::util::rf(&mut self.rng, 0.5, 0.9);
        self.blood_decal(b.pos, sz, team == Team::Infected);
        if self.pl.lock == Some(e) {
            self.pl.lock = None;
        }
        let by_player = killer.map(|k| k == self.player || self.team(k) == Some(Team::Ally)).unwrap_or(false) || (b.pos - self.ppos()).length() < 9.0;
        if let Some(f) = self.w.foe.get(e).cloned() {
            if let Some(o) = f.origin {
                self.consumed.insert(o);
            }
            // biomass
            if team == Team::Infected && f.biomass > 0.0 {
                let mult = if by_player { 1.0 } else { 0.35 };
                let n = (f.biomass * mult / 10.0).ceil() as u32;
                for _ in 0..n.min(10) {
                    let pe = self.w.spawn();
                    let mut pb = Body::new(b.pos + Vec3::Y * 0.6 + rand_dir(&mut self.rng) * 0.5, 0.2, 0.3);
                    pb.solid = false;
                    pb.push = rand_dir(&mut self.rng) * rf(&mut self.rng, 1.0, 3.0);
                    self.w.body.insert(pe, pb);
                    self.w.pickup.insert(pe, Pickup { kind: PickKind::Biomass(f.biomass * mult / n as f32), t: 0.0 });
                }
            }
            self.drop_loot(e, &f, by_player);
            if by_player && team == Team::Infected {
                self.on_player_kill(b.pos);
            }
            // affix death triggers
            for af in &f.affixes {
                match af {
                    AffixKind::Splitting if f.can_split => {
                        for k in 0..2 {
                            let off = Vec3::new(if k == 0 { 0.7 } else { -0.7 }, 0.0, 0.3);
                            self.spawn_split(e, b.pos + off);
                        }
                    }
                    AffixKind::Gravid => {
                        let n = self.rng.u32(4..7);
                        for _ in 0..n {
                            let p = b.pos + rand_dir(&mut self.rng) * 0.8;
                            self.spawn_archetype("crawler", p, 1, (self.w.actor.get(e).unwrap().level).max(1), None);
                        }
                        self.parts.burst(Fx::Ichor, b.center(), Vec3::Y, 40, 2.5);
                        self.audio.play_at(Sfx::Larva, b.pos, 0.7, 1.0);
                    }
                    AffixKind::Volatile => {
                        let mut z = self.make_zone(b.pos, ZoneKind::Blast, 3.5, self.w.actor.get(e).unwrap().dmg * 2.0, 1.2, Team::Infected, None);
                        z.delay = 0.9;
                        self.add_zone(b.pos, z);
                    }
                    _ => {}
                }
            }
            if let Some(bi) = f.boss {
                self.boss_killed(bi, e);
            }
            if f.husk {
                self.msg("THE HUSK IS STILL. WHAT IT HELD IS GONE.");
            }
            if let Some(ap) = f.apex {
                self.prof.apex_kills.insert(ap);
                self.msg(&format!("APEX SLAIN: {}", f.name.to_uppercase()));
            }
        }
    }

    fn on_player_kill(&mut self, p: Vec3) {
        let st = self.pl.stats.clone();
        let pe = self.player;
        let heal = st.sum(|e| if let Effect::KillHeal(x) = e { Some(*x) } else { None });
        if heal > 0.0 {
            let a = self.w.actor.get_mut(pe).unwrap();
            a.hp = (a.hp + heal).min(a.max_hp);
        }
        if st.has(|e| matches!(e, Effect::KillFrenzy(_))) {
            self.pl.frenzy = 5.0;
        }
        if st.has(|e| matches!(e, Effect::KillStamina)) {
            self.pl.stamina = st.max_stam;
        }
        let ex = st.sum(|e| if let Effect::KillExplode(x) = e { Some(*x) } else { None });
        if ex > 0.0 {
            let mut z = self.make_zone(p, ZoneKind::Blast, 3.2, ex * st.dmg, 0.6, Team::Player, Some(pe));
            z.delay = 0.25;
            self.add_zone(p, z);
        }
    }
}
