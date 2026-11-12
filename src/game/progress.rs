//! The roguelike layer: biomass and levels, mutation choices, death and the Husk, Burn Pits
//! (rest, fast travel, re-seeding the city), interactions, rifts and boss progression.
use super::comp::*;
use super::loot::{Grid, Item};
use super::player::Life;
use super::{Game, Screen};
use crate::audio::Sfx;
use crate::data::Effect;
use crate::ecs::Entity;
use crate::particles::Fx;
use crate::util::{rand_dir, Rng};
use crate::world::{pit_pos, rift_site, Area, AreaKind, InteractKind, DISTRICTS};
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HuskRec {
    pub pos: [f32; 3],
    pub life: Life,
    pub hand: Option<Item>,
}

/// Persistent progress (survives death).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Profile {
    pub seed: u32,
    pub epoch: u32,
    pub pits: Vec<bool>,
    pub last_pit: usize,
    pub bosses: [bool; 5],
    pub stash: Grid,
    pub apex_kills: HashSet<usize>,
    pub husk: Option<HuskRec>,
    pub intro_seen: bool,
    pub ended: bool,
    pub districts_seen: Vec<bool>,
    pub deaths: u32,
    pub vendor_stock: Vec<Item>,
}
impl Profile {
    pub fn new(seed: u32) -> Profile {
        let mut pits = vec![false; 12];
        pits[0] = true;
        let mut seen = vec![false; 6];
        seen[0] = true;
        Profile { seed, epoch: 0, pits, last_pit: 0, bosses: [false; 5], stash: Grid::new(12, 8), apex_kills: HashSet::new(), husk: None, intro_seen: false, ended: false, districts_seen: seen, deaths: 0, vendor_stock: Vec::new() }
    }
    /// Burn Pits in district `d` can be lit once the previous tier's rift boss is dead.
    pub fn district_unlocked(&self, d: usize) -> bool {
        d < 2 || self.bosses[d - 2]
    }
}

pub const LEVEL_CAP: u32 = 40;

impl Game {
    pub fn add_biomass(&mut self, amount: f32) {
        let a = amount * self.pl.stats.biomass;
        self.pl.life.biomass += a;
        let glut = self.pl.stats.sum(|e| if let Effect::Glutton(x) = e { Some(*x) } else { None });
        if glut > 0.0 {
            let pe = self.player;
            if let Some(ac) = self.w.actor.get_mut(pe) {
                ac.hp = (ac.hp + glut * a / 10.0).min(ac.max_hp);
            }
        }
        while self.pl.life.biomass >= self.pl.life.next_level_cost() && self.pl.life.level + self.pl.life.pending_levels < LEVEL_CAP {
            self.pl.life.biomass -= self.pl.life.next_level_cost();
            self.pl.life.pending_levels += 1;
            self.pl.life.level += 1;
        }
        if self.pl.life.pending_levels > 0 && self.screen == Screen::None && self.mutation_choices.is_empty() {
            self.open_level_up();
        }
    }

    pub fn open_level_up(&mut self) {
        let mut r = Rng::with_seed(self.rng.u64(..));
        self.mutation_choices = self.roll_mutations(&mut r, 3);
        if self.mutation_choices.is_empty() {
            self.pl.life.pending_levels = 0;
            return;
        }
        self.screen = Screen::LevelUp;
        self.ui.sel = 0;
        self.audio.play(Sfx::LevelUp, 0.8, 1.0);
        let p = self.ppos();
        self.parts.burst(Fx::Strain, p, Vec3::ZERO, 50, 1.0);
    }

    /// Draw distinct mutation offers weighted toward the current level's tiers.
    pub fn roll_mutations(&self, r: &mut Rng, n: usize) -> Vec<usize> {
        let lvl = self.pl.life.level;
        let owned: Vec<&str> = self.pl.life.mutations.iter().map(|s| s.as_str()).collect();
        let excl: Vec<&str> = owned.iter().filter_map(|id| self.t.mutations.iter().find(|m| m.id == *id)).filter_map(|m| m.excl.as_deref()).collect();
        let tags: Vec<&str> = owned.iter().filter_map(|id| self.t.mutations.iter().find(|m| m.id == *id)).filter_map(|m| m.tag.as_deref()).collect();
        let mut pool: Vec<(usize, f32)> = Vec::new();
        for (i, m) in self.t.mutations.iter().enumerate() {
            if owned.contains(&m.id.as_str()) {
                continue;
            }
            if let Some(x) = &m.excl {
                if excl.contains(&x.as_str()) {
                    continue;
                }
            }
            let tw = match m.tier {
                1 => (12.0 - lvl as f32 * 0.4).max(1.0),
                2 => if lvl >= 3 { 6.0 } else { 0.5 },
                3 => if lvl >= 7 { 4.0 + lvl as f32 * 0.1 } else { 0.0 },
                _ => if lvl >= 14 { 2.0 + lvl as f32 * 0.08 } else { 0.0 },
            };
            let syn = m.tag.as_deref().map(|t| tags.contains(&t)).unwrap_or(false);
            let w = tw * if syn { 1.6 } else { 1.0 };
            if w > 0.0 {
                pool.push((i, w));
            }
        }
        let mut out = Vec::new();
        while out.len() < n && !pool.is_empty() {
            let total: f32 = pool.iter().map(|p| p.1).sum();
            let mut x = r.f32() * total;
            let mut k = 0;
            for (j, p) in pool.iter().enumerate() {
                x -= p.1;
                if x <= 0.0 {
                    k = j;
                    break;
                }
                k = j;
            }
            out.push(pool.remove(k).0);
        }
        out
    }

    pub fn choose_mutation(&mut self, idx: usize) {
        let m = self.t.mutations[idx].clone();
        self.pl.life.mutations.push(m.id.clone());
        let displaced = self.pl.apply_hand_mutations(&self.t, &m.fx);
        for it in displaced {
            if let Err(it) = self.pl.gear.inv.add(&self.t, it) {
                let p = self.ppos();
                self.drop_item(p + Vec3::Y, it);
                self.msg("NO ROOM - YOUR OLD WEAPON FALLS TO THE GROUND");
            }
        }
        self.pl.life.pending_levels = self.pl.life.pending_levels.saturating_sub(1);
        self.mutation_choices.clear();
        self.refresh_player();
        let p = self.ppos();
        self.parts.burst(Fx::Ichor, p + Vec3::Y, Vec3::Y, 40, 1.5);
        self.audio.play(Sfx::Bloom, 0.7, 1.2);
        self.msg(&format!("MUTATION: {}", m.name.to_uppercase()));
        if self.pl.life.pending_levels > 0 {
            self.open_level_up();
        } else {
            self.screen = Screen::None;
        }
    }

    pub fn pickups_tick(&mut self, dt: f32) {
        let ids: Vec<u32> = self.w.pickup.ids().collect();
        let pp = self.ppos() + Vec3::Y * 0.8;
        let magnet = self.pl.stats.has(|e| matches!(e, Effect::Magnet));
        let alive = self.w.actor.get(self.player).map(|a| a.alive).unwrap_or(false);
        for id in ids {
            let e = self.w.ent(id);
            let mut pk = self.w.pickup.get(e).unwrap().clone();
            pk.t += dt;
            let b = *self.w.body.get(e).unwrap();
            let d = pp - b.pos;
            let dist = d.length();
            let auto = matches!(pk.kind, PickKind::Biomass(_) | PickKind::Scrip(_));
            let pull_r = if auto { if magnet { 10.0 } else { 4.5 } } else if magnet { 3.0 } else { 0.0 };
            let mut np = b.pos + b.push * dt;
            np.y = np.y.max(0.25 + if auto { 0.3 + (pk.t * 3.0).sin() * 0.08 } else { 0.0 });
            if alive && pk.t > 0.6 && dist < pull_r {
                np += d / dist.max(0.1) * dt * (6.0 + 10.0 / dist.max(0.5));
            }
            if let Some(bm) = self.w.body.get_mut(e) {
                bm.prev = bm.pos;
                bm.pos = np;
                bm.push *= 1.0 - (3.0 * dt).min(1.0);
            }
            let take = alive && pk.t > 0.6 && ((auto && dist < 0.9) || (!auto && magnet && dist < 1.2));
            if take {
                match &pk.kind {
                    PickKind::Biomass(a) => {
                        self.parts.burst(Fx::Biomass, np, Vec3::ZERO, 3, 1.0);
                        self.audio.play(Sfx::Pickup, 0.15, 1.6 + crate::util::rf(&mut self.rng, 0.0, 0.3));
                        self.w.kill(e);
                        self.add_biomass(*a);
                        continue;
                    }
                    PickKind::Scrip(s) => {
                        self.pl.gear.scrip += s;
                        self.float_text(np + Vec3::Y * 0.3, &format!("+{s} SCRIP"), [0.9, 0.9, 0.5]);
                        self.audio.play(Sfx::Pickup, 0.3, 1.2);
                        self.w.kill(e);
                        continue;
                    }
                    PickKind::Item(_) => {
                        self.w.pickup.insert(e, pk);
                        self.take_item(e);
                        continue;
                    }
                }
            }
            if !auto && pk.t > 240.0 {
                self.w.kill(e);
                continue;
            }
            self.w.pickup.insert(e, pk);
        }
    }

    pub fn take_item(&mut self, e: Entity) -> bool {
        let Some(PickKind::Item(it)) = self.w.pickup.get(e).map(|p| p.kind.clone()) else { return false };
        let name = it.name(&self.t);
        let col = it.rarity.color();
        match self.pl.gear.inv.add(&self.t, it) {
            Ok(()) => {
                self.msg_col(&format!("PICKED UP {}", name.to_uppercase()), col);
                self.audio.play(Sfx::Pickup, 0.5, 1.0);
                if let Some(o) = self.cache_origin.remove(&e.id) {
                    self.consumed.insert(o);
                }
                self.w.kill(e);
                true
            }
            Err(_) => {
                self.msg("INVENTORY FULL");
                false
            }
        }
    }

    /// Nearest interactable within reach, with a prompt.
    pub fn interact_target(&self) -> Option<(Entity, String)> {
        let pp = self.ppos();
        let mut best: Option<(Entity, f32, String)> = None;
        let consider = |e: Entity, d: f32, label: String, best: &mut Option<(Entity, f32, String)>| {
            if best.as_ref().map(|b| d < b.1).unwrap_or(true) {
                *best = Some((e, d, label));
            }
        };
        for (id, it) in self.w.interact.iter() {
            let e = self.w.ent(id);
            let p = self.w.body.get(e).unwrap().pos;
            let d = (p - pp).length();
            let reach = match it.kind {
                InteractKind::RiftPortal(_) | InteractKind::ExitPortal | InteractKind::BurnPit(_) => 3.6,
                _ => 2.4,
            };
            if d < reach {
                let label = match it.kind {
                    InteractKind::BurnPit(i) => {
                        if self.prof.pits[i] {
                            "REST AT BURN PIT".into()
                        } else if self.prof.district_unlocked(i / 2) {
                            "LIGHT BURN PIT".into()
                        } else {
                            "BURN PIT (COLD - TIER LOCKED)".into()
                        }
                    }
                    InteractKind::Vendor => "TRADE WITH QUARTERMASTER".into(),
                    InteractKind::Stash => "OPEN STASH".into(),
                    InteractKind::RiftPortal(i) => {
                        if i == 4 && !self.prof.bosses[..4].iter().all(|b| *b) {
                            "THE CRATER IS SEALED (FOUR RIFTS REMAIN OPEN)".into()
                        } else if self.prof.bosses[i] {
                            format!("ENTER {} (CLEARED)", crate::world::rift::RULES[i].name)
                        } else {
                            format!("ENTER {}", crate::world::rift::RULES[i].name)
                        }
                    }
                    InteractKind::ExitPortal => "RETURN TO THE CITY".into(),
                    InteractKind::Note(_) => "READ".into(),
                    InteractKind::Husk => "RECLAIM YOUR HUSK".into(),
                };
                consider(e, d, label, &mut best);
            }
        }
        for (id, pk) in self.w.pickup.iter() {
            if let PickKind::Item(it) = &pk.kind {
                let e = self.w.ent(id);
                let d = (self.w.body.get(e).unwrap().pos - pp).length();
                if d < 2.0 {
                    consider(e, d - 0.5, format!("TAKE {}", it.name(&self.t).to_uppercase()), &mut best);
                }
            }
        }
        best.map(|b| (b.0, b.2))
    }

    pub fn try_interact(&mut self) {
        let Some((e, _)) = self.interact_target() else { return };
        if self.w.pickup.has(e) {
            self.take_item(e);
            return;
        }
        let Some(k) = self.w.interact.get(e).map(|i| i.kind) else { return };
        if self.in_combat() && !matches!(k, InteractKind::Note(_) | InteractKind::Husk) {
            self.msg("NOT WITH THEM SO CLOSE");
            return;
        }
        match k {
            InteractKind::BurnPit(i) => {
                if !self.prof.pits[i] {
                    if !self.prof.district_unlocked(i / 2) {
                        self.msg("THE PIT WILL NOT TAKE. CLEAR THE PREVIOUS RIFT FIRST.");
                        return;
                    }
                    self.prof.pits[i] = true;
                    self.msg("BURN PIT LIT");
                    self.audio.play(Sfx::BurnPit, 0.8, 1.0);
                }
                self.prof.last_pit = i;
                self.screen = Screen::Pit;
                self.ui.sel = 0;
            }
            InteractKind::Vendor => {
                if self.prof.vendor_stock.is_empty() {
                    self.restock_vendor();
                }
                self.screen = Screen::Vendor;
                self.ui.cursor = (0, 0);
                self.ui.focus = 0;
            }
            InteractKind::Stash => {
                self.screen = Screen::Stash;
                self.ui.cursor = (0, 0);
                self.ui.focus = 0;
            }
            InteractKind::RiftPortal(i) => {
                if i == 4 && !self.prof.bosses[..4].iter().all(|b| *b) {
                    self.msg("THE CRATER IS SEALED. FOUR RIFTS STILL BLEED INTO THE CITY.");
                    return;
                }
                self.enter_rift(i);
            }
            InteractKind::ExitPortal => self.exit_rift(),
            InteractKind::Note(n) => {
                let txt = self.t.text.notes[n % self.t.text.notes.len()].clone();
                self.note = Some(txt);
                self.screen = Screen::Note;
                self.audio.play(Sfx::UiSelect, 0.4, 0.8);
            }
            InteractKind::Husk => self.reclaim_husk(e),
        }
    }

    pub fn in_combat(&self) -> bool {
        let pp = self.ppos();
        let mut c = false;
        self.hash.query(pp, 9.0, |id, _, _| {
            let e = self.w.ent(id);
            if let (Some(a), Some(ai)) = (self.w.actor.get(e), self.w.ai.get(e)) {
                if a.alive && a.team == Team::Infected && ai.target == Some(self.player) {
                    c = true;
                }
            }
        });
        c
    }

    pub fn restock_vendor(&mut self) {
        let mut r = crate::util::seed_rng(self.prof.seed, self.prof.epoch, 777);
        let lvl = self.pl.life.level.max(1) + 2;
        let mut stock = Vec::new();
        use super::loot::{gen_item, Rarity};
        use crate::data::Source;
        for base in ["rift_shard", "cinder_coal", "flesh_node", "censer"] {
            if let Some(b) = self.t.bases.iter().find(|b| b.id == base) {
                if b.min_level <= lvl {
                    stock.push(gen_item(&self.t, &mut r, lvl, &[Source::Military], Rarity::Magic, Some(base)));
                }
            }
        }
        for _ in 0..10 {
            let rar = if crate::util::chance(&mut r, 0.15) { Rarity::Rare } else if crate::util::chance(&mut r, 0.5) { Rarity::Magic } else { Rarity::Common };
            stock.push(gen_item(&self.t, &mut r, lvl, &[Source::Military, Source::Civilian], rar, None));
        }
        self.prof.vendor_stock = stock;
    }

    /// Rest: heal, refill heals, and re-seed every spawn in the city.
    pub fn rest(&mut self, pit: usize) {
        self.prof.last_pit = pit;
        self.prof.epoch += 1;
        self.pl.heal_charges = self.pl.stats.heal_charges;
        // Strain is untouched: it only falls by landing melee hits
        self.pl.affliction = None;
        self.pl.stamina = self.pl.stats.max_stam;
        self.pl.soldiers_angered = 0;
        let pe = self.player;
        if let Some(a) = self.w.actor.get_mut(pe) {
            a.hp = a.max_hp;
            a.burn = 0.0;
            a.rot = 0.0;
        }
        self.restock_vendor();
        self.rebuild_city(pit_pos(pit) + Vec3::new(0.0, 0.0, 3.5));
        self.flash = 0.6;
        self.audio.play(Sfx::BurnPit, 0.9, 0.8);
        self.msg("YOU REST. THE CITY SHIFTS. THE FRONT LINE MOVES.");
        self.save();
    }

    /// Rebuild the city around `pos` (after rest, death, or leaving a rift).
    pub fn rebuild_city(&mut self, pos: Vec3) {
        self.clear_entities();
        let mut a = Area::city(self.prof.seed);
        a.epoch = self.prof.epoch;
        a.bosses_dead = self.prof.bosses;
        self.area = a;
        self.consumed.clear();
        self.chunk_ents.clear();
        self.place_player(pos);
        self.stream_now(true);
        self.spawn_husk_marker();
    }

    pub fn clear_entities(&mut self) {
        let pe = self.player;
        let keep_actor = self.w.actor.get(pe).cloned();
        let keep_body = self.w.body.get(pe).copied();
        let keep_rig = self.w.rig.remove(pe);
        self.w.clear();
        self.player = self.w.spawn();
        if let (Some(a), Some(b), Some(r)) = (keep_actor, keep_body, keep_rig) {
            self.w.actor.insert(self.player, a);
            self.w.body.insert(self.player, b);
            self.w.rig.insert(self.player, r);
        }
        self.boss = None;
        self.boss_awake = false;
        self.pl.lock = None;
        self.mimic_plans.clear();
        self.cache_origin.clear();
        self.parts.clear();
    }

    pub fn place_player(&mut self, pos: Vec3) {
        let pe = self.player;
        if let Some(b) = self.w.body.get_mut(pe) {
            b.pos = pos;
            b.prev = pos;
            b.vel = Vec3::ZERO;
            b.push = Vec3::ZERO;
        }
        self.cam.snap = true;
    }

    fn spawn_husk_marker(&mut self) {
        if self.area.kind != AreaKind::City {
            return;
        }
        if let Some(h) = &self.prof.husk {
            let p = Vec3::from(h.pos);
            let e = self.w.spawn();
            let mut b = Body::new(p, 0.5, 1.0);
            b.solid = false;
            self.w.body.insert(e, b);
            self.w.interact.insert(e, Interact { kind: InteractKind::Husk });
        }
    }

    fn reclaim_husk(&mut self, e: Entity) {
        let Some(h) = self.prof.husk.take() else { return };
        self.w.kill(e);
        // restore the lost life, keeping anything gained since on top
        let gained: Vec<String> = self.pl.life.mutations.clone();
        let extra_bio = self.pl.life.biomass;
        self.pl.life = h.life;
        for m in gained {
            if !self.pl.life.mutations.contains(&m) {
                self.pl.life.mutations.push(m);
            }
        }
        let fx: Vec<Effect> = self.pl.mutation_effects(&self.t);
        let displaced = self.pl.apply_hand_mutations(&self.t, &fx);
        for it in displaced {
            let _ = self.pl.gear.inv.add(&self.t, it);
        }
        self.refresh_player();
        let p = self.ppos();
        self.parts.burst(Fx::Ichor, p + Vec3::Y, Vec3::Y, 60, 2.0);
        self.parts.burst(Fx::Strain, p, Vec3::ZERO, 40, 1.0);
        self.audio.play(Sfx::Bloom, 0.9, 0.9);
        self.msg(&format!("YOU TAKE BACK WHAT YOU WERE. LEVEL {} RESTORED.", self.pl.life.level));
        self.add_biomass(extra_bio);
    }

    pub fn player_died(&mut self, _src: Option<Entity>) {
        let pe = self.player;
        if let Some(a) = self.w.actor.get_mut(pe) {
            if !a.alive {
                return;
            }
            a.alive = false;
            a.hp = 0.0;
            a.dead_t = 0.0;
            a.act = Act::default();
        }
        self.screen = Screen::Dead;
        self.dead_t = 0.0;
        self.slowmo = 1.5;
        self.audio.play(Sfx::Death, 1.0, 0.7);
        self.prof.deaths += 1;
    }

    /// After the death screen: husk handling and respawn at the last Burn Pit.
    pub fn respawn(&mut self) {
        let death_pos = match self.area.kind {
            AreaKind::City => self.ppos(),
            AreaKind::Rift(i) => rift_site(i) + Vec3::new(4.0, 0.0, 4.0),
        };
        let mut risen: Option<(Vec3, HuskRec)> = None;
        if let Some(old) = self.prof.husk.take() {
            // dying before reaching the old husk destroys its build; it rises with it
            risen = Some((Vec3::from(old.pos), old));
            self.msg("YOUR OLD HUSK RISES WITH EVERYTHING YOU LOST");
        }
        let life = std::mem::replace(&mut self.pl.life, Life::new());
        if life.level > 1 || !life.mutations.is_empty() || life.biomass > 0.0 {
            let hand = self.pl.gear.eq[0].clone();
            self.prof.husk = Some(HuskRec { pos: death_pos.into(), life, hand });
        }
        self.pl.clear_grown();
        self.pl.strain = 0.0;
        self.pl.affliction = None;
        self.pl.frenzy = 0.0;
        self.pl.life.second_wind_used = false;
        let pe = self.player;
        if let Some(a) = self.w.actor.get_mut(pe) {
            a.alive = true;
            a.act = Act::default();
            a.burn = 0.0;
            a.rot = 0.0;
        }
        self.refresh_player();
        let pit = self.prof.last_pit;
        self.screen = Screen::None;
        self.rest(pit);
        if let Some((p, h)) = risen {
            // keep it from rising right at the pit
            let p = if (p - pit_pos(pit)).length() < 12.0 { p + Vec3::new(12.0, 0.0, 0.0) } else { p };
            self.risen_husk = Some((p, h));
        }
    }

    // ---- rifts ------------------------------------------------------------------------------
    pub fn enter_rift(&mut self, i: usize) {
        self.return_pos = Some(rift_site(i) + Vec3::new(0.0, 0.0, if i == 4 { 8.0 } else { 5.5 }));
        self.clear_entities();
        let seed = self.prof.seed ^ (i as u32 * 7919);
        self.area = Area::rift(i, seed, self.prof.epoch);
        self.consumed.clear();
        self.chunk_ents.clear();
        let start = self.area.start;
        self.place_player(start);
        let keys: Vec<(i32, i32)> = self.area.chunks.keys().copied().collect();
        for k in keys {
            self.populate(k);
        }
        let r = &crate::world::rift::RULES[i];
        self.subtitle(&format!("{} - {}", r.name, r.rule), 5.0);
        self.audio.play(Sfx::Rift, 1.0, 0.5);
        self.flash = 0.8;
        self.boss_intro_done = self.prof.bosses[i];
        if self.prof.bosses[i] {
            // a cleared rift keeps an exit portal in the arena
            let (c, _) = self.area.arena;
            self.add_exit_portal(c);
        }
    }

    pub fn exit_rift(&mut self) {
        let pos = self.return_pos.take().unwrap_or(pit_pos(self.prof.last_pit));
        self.rebuild_city(pos);
        self.flash = 0.8;
        self.audio.play(Sfx::Rift, 1.0, 0.7);
        self.save();
    }

    fn add_exit_portal(&mut self, c: Vec3) {
        let e = self.w.spawn();
        let mut b = Body::new(c, 0.5, 1.0);
        b.solid = false;
        self.w.body.insert(e, b);
        self.w.interact.insert(e, Interact { kind: InteractKind::ExitPortal });
    }

    pub fn boss_killed(&mut self, bi: usize, e: Entity) {
        self.prof.bosses[bi] = true;
        self.area.bosses_dead[bi] = true;
        self.boss = None;
        let p = self.w.body.get(e).map(|b| b.pos).unwrap_or(self.ppos());
        self.slowmo = 2.5;
        self.flash = 1.0;
        self.shake(1.0);
        self.audio.play(Sfx::BossRoar, 1.0, 0.5);
        self.parts.burst(Fx::Ichor, p + Vec3::Y * 2.0, Vec3::Y, 120, 4.0);
        self.parts.burst(Fx::Rift, p + Vec3::Y * 2.0, Vec3::ZERO, 80, 3.0);
        let name = self.t.bosses[bi].name.clone();
        self.big_banner = Some((format!("{name} DESTROYED"), 5.0));
        if bi < 4 {
            if bi < 3 {
                self.msg(&format!("THE BURN PITS OF {} WILL NOW TAKE", DISTRICTS[bi + 2].name));
            } else {
                self.msg("THE BURN PITS OF GROUND ZERO WILL NOW TAKE");
            }
            if self.prof.bosses[..4].iter().all(|b| *b) {
                self.msg("ALL FOUR RIFTS ARE CLOSED. THE CRATER OPENS.");
            }
            let (c, _) = self.area.arena;
            self.add_exit_portal(c);
            self.start_cutscene(super::cutscene::CutKind::BossDeath(bi));
        } else {
            // the world cannot be rebuilt mid-damage; the ending starts after this tick
            self.prof.ended = true;
            self.pending_ending = 3.0;
        }
        self.save();
    }

    /// Track district discovery and wake bosses when the player enters the arena.
    pub fn progress_tick(&mut self) {
        let pp = self.ppos();
        match self.area.kind {
            AreaKind::City => {
                if let Some(d) = crate::world::district_at(pp) {
                    if !self.prof.districts_seen[d] {
                        self.prof.districts_seen[d] = true;
                        let dd = &DISTRICTS[d];
                        self.big_banner = Some((format!("{} - LEVELS {}-{}", dd.name, dd.levels.0, dd.levels.1), 4.0));
                    }
                    if self.cur_district != Some(d) {
                        self.cur_district = Some(d);
                        self.subtitle(&format!("{}  ({:?})", DISTRICTS[d].name, DISTRICTS[d].state).to_uppercase(), 3.0);
                    }
                }
                if let Some((p, h)) = self.risen_husk.take() {
                    if (p - pp).length() < 60.0 {
                        self.spawn_risen_husk(p, &h.life, h.hand.clone());
                    } else {
                        self.risen_husk = Some((p, h));
                    }
                }
            }
            AreaKind::Rift(i) => {
                let (c, r) = self.area.arena;
                if !self.boss_intro_done && (pp - c).length() < r - 1.0 && self.boss.is_some() {
                    self.boss_intro_done = true;
                    self.start_cutscene(super::cutscene::CutKind::BossIntro(i));
                }
                if self.boss_intro_done && self.screen == Screen::None && self.cutscene.is_none() {
                    self.boss_awake = true;
                }
                // rift rules
                let rules = &crate::world::rift::RULES[i];
                if rules.gravity_pulse && (self.sim_time % 9.0) < 1.0 {
                    let ids: Vec<u32> = self.w.body.ids().collect();
                    let room = (pp / crate::world::rift::RS).floor() * crate::world::rift::RS + Vec3::splat(crate::world::rift::RS * 0.5);
                    for id in ids {
                        let e = self.w.ent(id);
                        if self.w.actor.get(e).map(|a| a.alive).unwrap_or(false) && self.boss != Some(e) {
                            let b = self.w.body.get_mut(e).unwrap();
                            let d = Vec3::new(room.x - b.pos.x, 0.0, room.z - b.pos.z);
                            if d.length() > 1.0 {
                                b.push += d.normalize() * 0.18;
                            }
                        }
                    }
                    if crate::util::chance(&mut self.rng, 0.1) {
                        self.distorts_temp.push((pp + Vec3::Y, 6.0, 0.4, 2.0));
                    }
                }
                if rules.strain_tick > 0.0 {
                    self.pl.strain += rules.strain_tick / 60.0;
                }
            }
        }
    }

    pub fn rift_heal_mult(&self) -> f32 {
        match self.area.kind {
            AreaKind::Rift(i) => crate::world::rift::RULES[i].heal_mult,
            _ => 1.0,
        }
    }

    pub fn fast_travel(&mut self, pit: usize) {
        self.screen = Screen::None;
        self.rest(pit);
    }

    pub fn _unused_rand(&mut self) -> Vec3 {
        rand_dir(&mut self.rng)
    }
}
