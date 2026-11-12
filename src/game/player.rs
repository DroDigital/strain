//! The player: persistent gear, the current life (level, biomass, mutations, Strain), derived
//! stats, and the soulslike controller (light chains, chargeable heavies, dodge i-frames,
//! sprint, block, perfect block, Rupture finishers, heals, casting).
use super::comp::*;
use super::loot::{grown_item, Grid, Item};
use super::Game;
use crate::anim::{BodyPlan, WeaponVis};
use crate::audio::Sfx;
use crate::data::{Effect, Growth, School, Stat, Tables, WeaponClass};
use crate::ecs::Entity;
use crate::input::Action;
use crate::particles::Fx;
use crate::util::{dir_yaw, flat, turn_toward, yaw_dir};
use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};

/// Everything that belongs to the current life (lost to the Husk on death).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Life {
    pub level: u32,
    pub biomass: f32,
    pub mutations: Vec<String>,
    pub pending_levels: u32,
    pub second_wind_used: bool,
}
impl Life {
    pub fn new() -> Life {
        Life { level: 1, ..Default::default() }
    }
    pub fn next_level_cost(&self) -> f32 {
        (60.0 * 1.22f32.powi(self.level as i32 - 1)).round()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Gear {
    pub eq: [Option<Item>; 7],
    pub inv: Grid,
    pub scrip: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub max_hp: f32,
    pub max_stam: f32,
    pub stam_regen: f32,
    pub dmg: f32,
    pub heavy: f32,
    pub atk_speed: f32,
    pub armor: f32,
    pub poise: f32,
    pub spell_power: f32,
    pub strain_cap: f32,
    pub strain_cost: f32,
    pub strain_drain: f32,
    pub move_speed: f32,
    pub crit: f32,
    pub lifesteal: f32,
    pub biomass: f32,
    pub heal_charges: u32,
    pub heal_power: f32,
    pub dodge_cost: f32,
    pub block_power: f32,
    pub parry_window: f32,
    pub fire_res: f32,
    pub thorns: f32,
    pub luck: f32,
    pub fx: Vec<Effect>,
}

impl Stats {
    pub fn has(&self, f: impl Fn(&Effect) -> bool) -> bool {
        self.fx.iter().any(f)
    }
    /// Sum the numeric payload of every matching effect.
    pub fn sum(&self, f: impl Fn(&Effect) -> Option<f32>) -> f32 {
        self.fx.iter().filter_map(f).sum()
    }
}

#[derive(Clone, Debug)]
pub struct PlayerState {
    pub life: Life,
    pub gear: Gear,
    pub stats: Stats,
    pub stamina: f32,
    pub strain: f32,
    pub heal_charges: u32,
    pub stam_delay: f32,
    pub sprinting: bool,
    pub blocking: bool,
    pub block_t: f32,
    pub parry_cd: f32,
    pub lock: Option<Entity>,
    pub spell_idx: usize,
    pub combo_t: f32,
    pub next_left: bool,
    pub buffer: Option<(Action, f32)>,
    pub frenzy: f32,
    pub reflex: f32,
    pub tendril: f32,
    pub extra_arm_t: f32,
    pub extra_arm_cd: f32,
    pub hits_landed: u32,
    pub affliction: Option<(Affliction, f32)>,
    pub affl_cd: f32,
    pub heal_regen: f32,
    pub step_t: f32,
    pub last_hit_target: Option<(Entity, f32)>,
    pub spore_t: f32,
    pub sprint_fx_t: f32,
    pub soldiers_angered: u32,
    pub kindle_dps: f32,
    pub combo_index: u8,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Affliction {
    /// stamina drains
    Cramp,
    /// attacks are slower
    Tremor,
    /// health bleeds
    Haemorrhage,
    /// vision swims
    Blur,
    /// body lurches randomly
    Lurch,
    /// guard drops: cannot block
    Spasm,
}
impl Affliction {
    pub fn name(self) -> &'static str {
        match self {
            Affliction::Cramp => "CRAMP: STAMINA DRAINING",
            Affliction::Tremor => "TREMOR: ATTACKS SLOWED",
            Affliction::Haemorrhage => "HAEMORRHAGE: BLEEDING",
            Affliction::Blur => "BLUR: VISION SWIMMING",
            Affliction::Lurch => "LURCH: BODY UNRESPONSIVE",
            Affliction::Spasm => "SPASM: CANNOT GUARD",
        }
    }
}

pub fn starting_gear(t: &Tables, r: &mut crate::util::Rng) -> Gear {
    use super::loot::{gen_item, Rarity};
    use crate::data::Source;
    let mut eq: [Option<Item>; 7] = Default::default();
    eq[0] = Some(gen_item(t, r, 1, &[Source::Civilian], Rarity::Common, Some("lead_pipe")));
    let mut cat = gen_item(t, r, 1, &[Source::Biological], Rarity::Common, Some("flesh_node"));
    cat.spells = vec!["bone_spikes".into(), "tendril_grab".into()];
    eq[1] = Some(cat);
    eq[3] = Some(gen_item(t, r, 1, &[Source::Civilian], Rarity::Common, Some("work_coveralls")));
    let mut inv = Grid::new(10, 6);
    let _ = inv.add(t, gen_item(t, r, 1, &[Source::Civilian], Rarity::Magic, Some("kitchen_knife")));
    Gear { eq, inv, scrip: 40 }
}

impl PlayerState {
    pub fn new(gear: Gear) -> PlayerState {
        PlayerState {
            life: Life::new(),
            gear,
            stats: Stats::default(),
            stamina: 100.0,
            strain: 0.0,
            heal_charges: 4,
            stam_delay: 0.0,
            sprinting: false,
            blocking: false,
            block_t: 0.0,
            parry_cd: 0.0,
            lock: None,
            spell_idx: 0,
            combo_t: 0.0,
            next_left: false,
            buffer: None,
            frenzy: 0.0,
            reflex: 0.0,
            tendril: 0.0,
            extra_arm_t: 0.0,
            extra_arm_cd: 0.0,
            hits_landed: 0,
            affliction: None,
            affl_cd: 3.0,
            heal_regen: 0.0,
            step_t: 0.0,
            last_hit_target: None,
            spore_t: 0.0,
            sprint_fx_t: 0.0,
            soldiers_angered: 0,
            kindle_dps: 0.0,
            combo_index: 0,
        }
    }

    pub fn mutation_effects(&self, t: &Tables) -> Vec<Effect> {
        let mut fx = Vec::new();
        let tags: Vec<&str> = self.life.mutations.iter().filter_map(|m| t.mutations.iter().find(|d| &d.id == m)).filter_map(|d| d.tag.as_deref()).collect();
        for m in &self.life.mutations {
            if let Some(d) = t.mutations.iter().find(|d| &d.id == m) {
                fx.extend(d.fx.iter().cloned());
                if let Some((tag, bonus)) = &d.syn {
                    // synergy: another owned mutation shares the tag
                    let n = tags.iter().filter(|x| **x == tag.as_str()).count();
                    let own = d.tag.as_deref() == Some(tag.as_str());
                    if n > own as usize {
                        fx.push(bonus.clone());
                    }
                }
            }
        }
        fx
    }

    pub fn compute(&mut self, t: &Tables) {
        let mut s = Stats {
            max_hp: 100.0 + (self.life.level as f32 - 1.0) * 7.0,
            max_stam: 100.0,
            stam_regen: 1.0,
            dmg: 1.0 + (self.life.level as f32 - 1.0) * 0.04,
            heavy: 1.0,
            atk_speed: 1.0,
            armor: 0.0,
            poise: 20.0,
            spell_power: 1.0,
            strain_cap: 100.0,
            strain_cost: 1.0,
            strain_drain: 1.0,
            move_speed: 1.0,
            crit: 0.05,
            lifesteal: 0.0,
            biomass: 1.0,
            heal_charges: 4,
            heal_power: 1.0,
            dodge_cost: 1.0,
            block_power: 1.0,
            parry_window: 1.0,
            fire_res: 0.0,
            thorns: 0.0,
            luck: 0.0,
            fx: Vec::new(),
        };
        let mut fx = self.mutation_effects(t);
        let mut hp_pct = 0.0;
        for (i, it) in self.gear.eq.iter().enumerate() {
            if let Some(it) = it {
                let b = it.base(t);
                if b.armor > 0.0 && b.dmg <= 0.0 {
                    s.armor += it.power;
                }
                if b.class == Some(WeaponClass::Shield) && i == 1 {
                    s.armor += b.armor;
                }
                for (st, v) in &it.affixes {
                    fx.push(Effect::Stat(*st, *v));
                }
                fx.extend(it.effects(t).iter().cloned());
            }
        }
        for e in &fx {
            if let Effect::Stat(st, v) = e {
                let p = v / 100.0;
                match st {
                    Stat::MaxHp => s.max_hp += v,
                    Stat::HpPct => hp_pct += p,
                    Stat::MaxStam => s.max_stam += v,
                    Stat::StamRegen => s.stam_regen += p,
                    Stat::Dmg => s.dmg += p,
                    Stat::HeavyDmg => s.heavy += p,
                    Stat::AtkSpeed => s.atk_speed += p,
                    Stat::Armor => s.armor += v,
                    Stat::Poise => s.poise += v,
                    Stat::SpellPower => s.spell_power += p,
                    Stat::StrainCap => s.strain_cap += v,
                    Stat::StrainCost => s.strain_cost += p,
                    Stat::StrainDrain => s.strain_drain += p,
                    Stat::MoveSpeed => s.move_speed += p,
                    Stat::Crit => s.crit += p,
                    Stat::Lifesteal => s.lifesteal += p,
                    Stat::Biomass => s.biomass += p,
                    Stat::HealCharges => s.heal_charges = (s.heal_charges as f32 + v).max(0.0) as u32,
                    Stat::HealPower => s.heal_power += p,
                    Stat::DodgeCost => s.dodge_cost += p,
                    Stat::BlockPower => s.block_power += p,
                    Stat::ParryWindow => s.parry_window += p,
                    Stat::FireRes => s.fire_res += p,
                    Stat::Thorns => s.thorns += v,
                    Stat::Luck => s.luck += p,
                }
            }
        }
        s.max_hp *= (1.0 + hp_pct).max(0.2);
        s.strain_cap = s.strain_cap.max(30.0);
        s.strain_cost = s.strain_cost.max(0.2);
        s.atk_speed = s.atk_speed.clamp(0.5, 2.0);
        s.move_speed = s.move_speed.clamp(0.6, 1.6);
        s.dodge_cost = s.dodge_cost.max(0.2);
        s.block_power = s.block_power.max(0.0);
        s.fire_res = s.fire_res.min(0.8);
        if fx.iter().any(|e| matches!(e, Effect::NoHeal)) {
            s.heal_charges = 0;
        }
        s.fx = fx;
        self.stats = s;
    }

    /// Spells available from catalysts and casting organs.
    pub fn spells(&self, t: &Tables) -> Vec<usize> {
        let mut out = Vec::new();
        for it in self.gear.eq[..2].iter().flatten() {
            if it.class(t) == Some(WeaponClass::Catalyst) {
                for s in &it.spells {
                    if let Some(i) = t.spells.iter().position(|x| &x.id == s) {
                        if !out.contains(&i) {
                            out.push(i);
                        }
                    }
                }
            }
        }
        out
    }
    pub fn catalyst_school(&self, t: &Tables) -> Vec<School> {
        self.gear.eq[..2].iter().flatten().filter_map(|it| if it.class(t) == Some(WeaponClass::Catalyst) { it.school(t) } else { None }).collect()
    }
    pub fn strain_frac(&self) -> f32 {
        (self.strain / self.stats.strain_cap).clamp(0.0, 1.0)
    }

    /// Weapon used for light/heavy attacks from a hand (None = fist).
    pub fn hand_weapon(&self, t: &Tables, left: bool) -> (WeaponClass, f32) {
        let i = if left { 1 } else { 0 };
        match &self.gear.eq[i] {
            Some(it) => (it.class(t).unwrap_or(WeaponClass::Fist), it.power),
            None => (WeaponClass::Fist, 8.0),
        }
    }
    pub fn two_handing(&self, t: &Tables) -> bool {
        self.gear.eq[0].as_ref().map(|i| i.two_handed(t)).unwrap_or(false) && self.gear.eq[1].is_none()
    }
    /// Dual-wielding: both hands hold melee weapons (not shields/catalysts).
    pub fn dual(&self, t: &Tables) -> bool {
        let w = |i: usize| self.gear.eq[i].as_ref().and_then(|it| it.class(t)).map(|c| !matches!(c, WeaponClass::Shield | WeaponClass::Catalyst)).unwrap_or(false);
        w(0) && w(1)
    }
    pub fn guard_class(&self, t: &Tables) -> WeaponClass {
        let l = self.gear.eq[1].as_ref().and_then(|i| i.class(t));
        if l == Some(WeaponClass::Shield) {
            return WeaponClass::Shield;
        }
        self.gear.eq[0].as_ref().and_then(|i| i.class(t)).or(l).unwrap_or(WeaponClass::Fist)
    }

    /// Apply a mutation's permanent hand conversions.
    pub fn apply_hand_mutations(&mut self, t: &Tables, effects: &[Effect]) -> Vec<Item> {
        let mut displaced = Vec::new();
        for e in effects {
            let (slot, base, school) = match e {
                Effect::HandWeapon(s, b) => (*s as usize, b.as_str(), None),
                Effect::HandOrgan(s, sc) => (*s as usize, "mut_organ", Some(*sc)),
                _ => continue,
            };
            let slot = slot.min(1);
            if let Some(old) = self.gear.eq[slot].take() {
                if !old.grown {
                    displaced.push(old);
                }
            }
            // a two-hander in the other hand cannot stay
            if let Some(o) = &self.gear.eq[1 - slot] {
                if o.two_handed(t) && !o.grown {
                    displaced.push(self.gear.eq[1 - slot].take().unwrap());
                }
            }
            self.gear.eq[slot] = Some(grown_item(t, base, self.life.level, school));
        }
        displaced
    }
    /// Remove grown limbs (on death the body is renewed).
    pub fn clear_grown(&mut self) {
        for s in 0..2 {
            if self.gear.eq[s].as_ref().map(|i| i.grown).unwrap_or(false) {
                self.gear.eq[s] = None;
            }
        }
    }
}

/// The player's body plan: an ordinary man, visibly altered by every mutation.
pub fn player_plan(t: &Tables, ps: &PlayerState) -> BodyPlan {
    let mut p = BodyPlan::human(77);
    p.cloth = [0.32, 0.3, 0.27];
    p.legs_tint = [0.2, 0.22, 0.26];
    p.skin = [0.95, 0.85, 0.8];
    p.head = crate::data::HeadKind::Hood;
    if let Some(b) = &ps.gear.eq[3] {
        p.cloth = match b.base.as_str() {
            "flak_vest" => [0.3, 0.35, 0.25],
            "hazmat" => [0.8, 0.7, 0.2],
            "leather_jacket" => [0.2, 0.15, 0.12],
            "carapace" => [0.3, 0.25, 0.35],
            "folded_robe" => [0.35, 0.3, 0.55],
            _ => [0.35, 0.4, 0.45],
        };
        if b.base == "carapace" {
            p.torso_tex = crate::render::textures::Tex::Chitin;
        }
    }
    if let Some(h) = &ps.gear.eq[2] {
        p.head = match h.base.as_str() {
            "riot_helmet" | "hard_hat" | "gas_mask" => crate::data::HeadKind::Helmet,
            "chitin_crown" | "halo_fragment" => crate::data::HeadKind::Crown,
            _ => crate::data::HeadKind::Hood,
        };
        if p.head == crate::data::HeadKind::Crown {
            p.head_tex = crate::render::textures::Tex::Bone;
        }
    }
    let n = ps.life.mutations.len();
    // the infection shows: skin greys and veins glow the further it goes
    let k = (n as f32 / 12.0).min(1.0);
    p.skin = [0.95 - 0.3 * k, 0.85 - 0.25 * k, 0.8 - 0.05 * k];
    p.glow = [0.5 + 0.5 * k, 1.0 - 0.4 * k, 0.8];
    for (i, m) in ps.life.mutations.iter().enumerate() {
        if let Some(d) = t.mutations.iter().find(|d| &d.id == m) {
            let (kind, at, tex) = d.vis;
            p.growths.push(Growth { kind, at, tex, size: 0.8 + d.tier as f32 * 0.12 + (i % 3) as f32 * 0.05 });
            if d.id == "apex_frame" {
                p.scale = 1.18;
            }
            if d.id == "third_arm" {
                p.arms.push(crate::anim::ArmDef { side: 1.0, kind: crate::data::ArmKind::Claw, tex: crate::render::textures::Tex::Flesh, len: 0.7, thick: 0.09, at: 0.4 });
            }
            if d.id == "calcified_legs" {
                p.leg_tex = crate::render::textures::Tex::Bone;
                p.legs_tint = [0.9, 0.85, 0.75];
            }
        }
    }
    for s in 0..2 {
        if let Some(it) = &ps.gear.eq[s] {
            let class = it.class(t).unwrap_or(WeaponClass::Fist);
            if it.grown {
                // the arm itself becomes the weapon
                p.arms[s].kind = match class {
                    WeaponClass::BoneBlade => crate::data::ArmKind::Blade,
                    WeaponClass::Whip => crate::data::ArmKind::Tendril,
                    WeaponClass::Pipe => crate::data::ArmKind::Club,
                    _ => crate::data::ArmKind::Cannon,
                };
                p.arms[s].tex = if class == WeaponClass::Catalyst { crate::render::textures::Tex::Vein } else { crate::render::textures::Tex::Bone };
                p.arms[s].len = 0.75;
                p.arms[s].thick = 0.11;
                continue;
            }
            let glow = if it.rarity >= super::loot::Rarity::Legendary { 0.4 } else { 0.0 };
            p.hands[s] = Some(WeaponVis { class, tint: crate::anim::weapon_tint(class, it.seed), glow });
        }
    }
    p
}

impl Game {
    pub fn player_body(&self) -> Body {
        *self.w.body.get(self.player).unwrap()
    }
    pub fn ppos(&self) -> Vec3 {
        self.w.body.get(self.player).map(|b| b.pos).unwrap_or(Vec3::ZERO)
    }

    pub fn refresh_player(&mut self) {
        self.pl.compute(&self.t);
        let st = &self.pl.stats;
        if let Some(a) = self.w.actor.get_mut(self.player) {
            let frac = a.frac();
            a.max_hp = st.max_hp;
            a.hp = (frac * st.max_hp).max(a.hp.min(st.max_hp));
            a.max_poise = st.poise;
            a.armor = st.armor;
            a.fire_immune = false;
            a.level = self.pl.life.level;
        }
        self.pl.heal_charges = self.pl.heal_charges.min(self.pl.stats.heal_charges);
        let plan = player_plan(&self.t, &self.pl);
        if let Some(r) = self.w.rig.get_mut(self.player) {
            r.plan = plan;
        }
        let sp = self.pl.spells(&self.t);
        if self.pl.spell_idx >= sp.len() {
            self.pl.spell_idx = 0;
        }
    }

    fn spend_stamina(&mut self, amount: f32) -> bool {
        if self.pl.stamina <= 0.0 {
            return false;
        }
        self.pl.stamina -= amount;
        self.pl.stam_delay = 0.7;
        true
    }

    /// Player input → actions. Runs at the fixed timestep.
    pub fn player_tick(&mut self, dt: f32) {
        let pe = self.player;
        let Some(actor) = self.w.actor.get(pe) else { return };
        if !actor.alive {
            return;
        }
        let controllable = self.screen_allows_control();
        let st = self.pl.stats.clone();
        let inp = self.input_snapshot.clone();
        let mv = if controllable { inp.mv } else { Vec2::ZERO };
        let cam_yaw = self.cam.yaw;
        let fwd = yaw_dir(cam_yaw);
        let right = Vec3::new(-fwd.z, 0.0, fwd.x);
        let mut wish = fwd * mv.y + right * mv.x;
        // afflictions
        if let Some((a, _)) = self.pl.affliction {
            if a == Affliction::Lurch {
                let t = self.sim_time as f32;
                wish += Vec3::new((t * 2.3).sin(), 0.0, (t * 1.7).cos()) * 0.6;
            }
        }
        let pressed = |a: Action| controllable && inp.pressed.contains(&a);
        let down = |a: Action| controllable && inp.down.contains(&a);
        let released = |a: Action| controllable && inp.released.contains(&a);

        // input buffer (lets players queue the next action during recovery)
        for a in [Action::Light, Action::Heavy, Action::Dodge, Action::Heal, Action::Cast] {
            if pressed(a) {
                self.pl.buffer = Some((a, 0.35));
            }
        }
        if let Some((_, t)) = &mut self.pl.buffer {
            *t -= dt;
            if *t <= 0.0 {
                self.pl.buffer = None;
            }
        }

        // lock-on
        if pressed(Action::LockOn) {
            if self.pl.lock.is_some() {
                self.pl.lock = None;
            } else {
                self.pl.lock = self.find_lock_target(None);
            }
        }
        if let Some(l) = self.pl.lock {
            let ok = self.w.actor.get(l).map(|a| a.alive).unwrap_or(false) && self.w.body.get(l).map(|b| (b.pos - self.ppos()).length() < 26.0).unwrap_or(false);
            if !ok {
                self.pl.lock = self.find_lock_target(None);
            } else if inp.look_flick != 0.0 {
                self.pl.lock = self.find_lock_target(Some(inp.look_flick)).or(Some(l));
            }
        }

        let a = self.w.actor.get(pe).unwrap().act.clone();
        let busy = a.busy();
        let mut stamina_regen = true;

        // sprint
        let sprint_held = down(Action::Dodge) && !st.has(|e| matches!(e, Effect::NoSprint));
        self.pl.sprinting = sprint_held && wish.length() > 0.3 && !busy && self.pl.stamina > 0.0 && inp.dodge_held_t > 0.25;
        // block
        let can_block = !st.has(|e| matches!(e, Effect::NoBlock)) && !matches!(self.pl.affliction, Some((Affliction::Spasm, _)));
        let blocking_now = down(Action::Block) && can_block && !busy && self.pl.stamina > 0.0;
        if blocking_now && !self.pl.blocking {
            self.pl.block_t = 0.0;
        }
        self.pl.blocking = blocking_now;
        if self.pl.blocking {
            self.pl.block_t += dt;
        }

        // ---- start actions when free (or cancel windows)
        let free = !busy || (a.kind == ActKind::Attack && a.phase() >= 2.4);
        let buf = self.pl.buffer.map(|b| b.0);
        if free && controllable {
            if buf == Some(Action::Dodge) && released(Action::Dodge) && inp.dodge_held_t < 0.25 || (buf == Some(Action::Dodge) && !down(Action::Dodge) && inp.dodge_held_t < 0.25) {
                self.pl.buffer = None;
                self.start_dodge(wish);
            } else if buf == Some(Action::Light) {
                self.pl.buffer = None;
                self.start_light();
            } else if buf == Some(Action::Heavy) {
                self.pl.buffer = None;
                self.start_heavy();
            } else if buf == Some(Action::Heal) && !busy {
                self.pl.buffer = None;
                self.start_heal();
            } else if buf == Some(Action::Cast) && !busy {
                self.pl.buffer = None;
                self.start_cast();
            }
        }
        if pressed(Action::NextSpell) {
            let n = self.pl.spells(&self.t).len().max(1);
            self.pl.spell_idx = (self.pl.spell_idx + 1) % n;
            self.audio.play(Sfx::UiMove, 0.4, 1.0);
        }

        // heavy charge: held heavy keeps the windup going
        {
            let actor = self.w.actor.get_mut(pe).unwrap();
            if actor.act.kind == ActKind::Attack && actor.act.heavy && actor.act.charging {
                if down(Action::Heavy) && actor.act.charge < 1.0 {
                    let mv = actor.act.mv.unwrap();
                    if actor.act.t > mv.wind * 0.8 {
                        actor.act.t = mv.wind * 0.8;
                        actor.act.charge = (actor.act.charge + dt / 0.9).min(1.0);
                        stamina_regen = false;
                    }
                } else {
                    actor.act.charging = false;
                }
            }
        }

        // ---- locomotion
        let actor = self.w.actor.get(pe).unwrap();
        let act = actor.act.clone();
        let slow = 1.0 - actor.slow.min(0.6);
        let mut speed = 4.2 * st.move_speed * slow;
        if self.pl.sprinting {
            speed *= 1.65;
            self.pl.stamina -= 14.0 * dt;
            self.pl.stam_delay = 0.4;
            stamina_regen = false;
        }
        if self.pl.blocking {
            speed *= 0.45;
        }
        let lock_dir = self.pl.lock.and_then(|l| self.w.body.get(l)).map(|b| flat(b.pos - self.ppos()).normalize_or_zero());
        let body = self.w.body.get_mut(pe).unwrap();
        let mut vel = Vec3::ZERO;
        let mut face: Option<Vec3> = None;
        match act.kind {
            ActKind::Idle => {
                vel = wish.clamp_length_max(1.0) * speed;
                face = if lock_dir.is_some() && !self.pl.sprinting { lock_dir } else if wish.length() > 0.1 { Some(wish) } else { None };
            }
            ActKind::Attack => {
                let mv = act.mv.unwrap();
                let ph = act.phase();
                // track the target during windup, commit afterwards
                let aim = lock_dir.or(if wish.length() > 0.1 { Some(wish) } else { None });
                if ph < 1.0 {
                    face = aim;
                }
                if (0.7..1.6).contains(&ph) {
                    let lunge = mv.lunge * if act.heavy { 1.0 + act.charge * 0.5 } else { 1.0 };
                    vel = yaw_dir(body.yaw) * lunge / (mv.act + mv.wind * 0.3).max(0.1) * 0.5;
                }
            }
            ActKind::Dodge => {
                let k = 1.0 - act.t / act.dur;
                vel = act.dir * (if st.has(|e| matches!(e, Effect::DoubleDodge)) { 9.5 } else { 8.0 }) * (0.35 + k);
                face = Some(act.dir);
            }
            ActKind::Heal | ActKind::Cast => {
                vel = wish.clamp_length_max(1.0) * speed * 0.35;
                face = lock_dir.or(if wish.length() > 0.1 { Some(wish) } else { None });
            }
            ActKind::Seized => {
                let t = act.t;
                vel = Vec3::new((t * 17.0).sin(), 0.0, (t * 13.0).cos()) * 1.5;
            }
            _ => {}
        }
        body.vel = vel;
        if let Some(f) = face {
            let target = dir_yaw(f);
            body.yaw = turn_toward(body.yaw, target, dt * if act.kind == ActKind::Idle { 14.0 } else { 9.0 });
        }
        let moving = vel.length() > 0.5;
        let pos = body.pos;

        // footsteps + sprint effects
        if moving && act.kind == ActKind::Idle {
            self.pl.step_t -= dt * vel.length() / 2.0;
            if self.pl.step_t <= 0.0 {
                self.pl.step_t = 0.7;
                self.audio.play_at(Sfx::Step, pos, 0.25, crate::util::rf(&mut self.rng, 0.85, 1.15));
                if self.pl.sprinting {
                    self.noise(pos, 9.0);
                }
            }
        }
        if self.pl.sprinting {
            let fire = st.sum(|e| if let Effect::SprintFire(x) = e { Some(*x) } else { None });
            self.pl.sprint_fx_t -= dt;
            if fire > 0.0 && self.pl.sprint_fx_t <= 0.0 {
                self.pl.sprint_fx_t = 0.25;
                self.spawn_zone(pos, ZoneKind::Fire, 1.2, fire, 2.5, Team::Player, Some(pe));
            }
        }

        // ---- stamina / strain upkeep
        if stamina_regen && !self.pl.blocking && act.kind == ActKind::Idle {
            if self.pl.stam_delay > 0.0 {
                self.pl.stam_delay -= dt;
            } else {
                let cramp = matches!(self.pl.affliction, Some((Affliction::Cramp, _)));
                self.pl.stamina += (if cramp { -4.0 } else { 38.0 * st.stam_regen }) * dt;
            }
        } else if self.pl.blocking {
            self.pl.stamina += 10.0 * st.stam_regen * dt;
        }
        self.pl.stamina = self.pl.stamina.clamp(-20.0, st.max_stam);
        let leak = st.sum(|e| if let Effect::StrainLeak(x) = e { Some(*x) } else { None });
        if leak > 0.0 {
            self.pl.strain = (self.pl.strain - leak * dt).max(0.0);
        }
        self.pl.combo_t -= dt;
        if self.pl.combo_t <= 0.0 {
            self.pl.next_left = false;
        }
        self.pl.parry_cd -= dt;
        self.pl.frenzy -= dt;
        self.pl.reflex -= dt;
        self.pl.tendril -= dt;

        // ---- passive effects: regeneration, spore aura, extra arm, strain afflictions
        let regen = st.sum(|e| if let Effect::Regen(x) = e { Some(*x) } else { None });
        if regen > 0.0 || self.pl.heal_regen > 0.0 {
            let hr = if self.pl.heal_regen > 0.0 { st.max_hp * 0.12 } else { 0.0 };
            self.pl.heal_regen -= dt;
            let a = self.w.actor.get_mut(pe).unwrap();
            a.hp = (a.hp + (regen + hr) * dt).min(a.max_hp);
        }
        let aura = st.sum(|e| if let Effect::SporeAura(x) = e { Some(*x) } else { None });
        if aura > 0.0 {
            self.pl.spore_t -= dt;
            if self.pl.spore_t <= 0.0 {
                self.pl.spore_t = 0.5;
                self.aoe(pos, 3.0, aura * 0.5, 0.0, Team::Player, Some(pe), super::combat::DmgKind::Rot);
                self.parts.burst(Fx::Spore, pos + Vec3::Y, Vec3::ZERO, 4, 1.5);
            }
        }
        let arm = st.sum(|e| if let Effect::ExtraArm(x) = e { Some(*x) } else { None });
        if arm > 0.0 {
            self.pl.extra_arm_cd -= dt;
            if self.pl.extra_arm_t > 0.0 {
                self.pl.extra_arm_t -= dt * 3.0;
            }
            if self.pl.extra_arm_cd <= 0.0 {
                if let Some(t) = self.nearest_hostile(pos, 2.6, Team::Player) {
                    self.pl.extra_arm_cd = 1.4;
                    self.pl.extra_arm_t = 1.0;
                    let tp = self.w.body.get(t).unwrap().pos;
                    self.deal(t, Some(pe), arm * st.dmg, 8.0, flat(tp - pos).normalize_or_zero(), super::combat::DmgKind::Melee, 1.0);
                }
            }
        }
        if self.pl.tendril > 0.0 {
            self.pl.extra_arm_cd -= dt;
            if self.pl.extra_arm_cd <= 0.0 {
                if let Some(t) = self.nearest_hostile(pos, 4.0, Team::Player) {
                    self.pl.extra_arm_cd = 0.6;
                    let tp = self.w.body.get(t).unwrap().pos;
                    let p = st.sum(|e| if let Effect::ParryTendril(x) = e { Some(*x) } else { None });
                    self.deal(t, Some(pe), p * 1.5 * st.dmg, 6.0, flat(tp - pos).normalize_or_zero(), super::combat::DmgKind::Melee, 1.0);
                    self.parts.burst(Fx::Ichor, tp + Vec3::Y, Vec3::ZERO, 6, 1.0);
                }
            }
        }
        self.strain_tick(dt);

        // interaction
        if controllable && pressed(Action::Interact) {
            self.try_interact();
        }
    }

    fn strain_tick(&mut self, dt: f32) {
        let f = self.pl.strain_frac();
        let calm = self.pl.stats.has(|e| matches!(e, Effect::StrainCalm));
        if let Some((a, t)) = &mut self.pl.affliction {
            *t -= dt;
            if *a == Affliction::Haemorrhage {
                let pe = self.player;
                let ac = self.w.actor.get_mut(pe).unwrap();
                ac.hp = (ac.hp - ac.max_hp * 0.02 * dt).max(1.0);
            }
            if *t <= 0.0 {
                self.pl.affliction = None;
            }
        }
        if f > 0.35 && !calm && self.pl.affliction.is_none() {
            self.pl.affl_cd -= dt * (f - 0.3) * 2.0;
            if self.pl.affl_cd <= 0.0 {
                self.pl.affl_cd = crate::util::rf(&mut self.rng, 3.0, 7.0);
                let all = [Affliction::Cramp, Affliction::Tremor, Affliction::Haemorrhage, Affliction::Blur, Affliction::Lurch, Affliction::Spasm];
                let a = all[self.rng.usize(0..all.len())];
                self.pl.affliction = Some((a, crate::util::rf(&mut self.rng, 3.0, 6.0)));
                self.msg(a.name());
                self.audio.play(Sfx::StrainWarn, 0.5, 1.0);
                let p = self.ppos();
                self.parts.burst(Fx::Strain, p, Vec3::ZERO, 16, 1.0);
            }
        }
        if self.pl.strain >= self.pl.stats.strain_cap {
            // overload: brief loss of control and heavy self-damage
            let pe = self.player;
            let p = self.ppos();
            self.pl.strain = self.pl.stats.strain_cap * 0.55;
            let dmg = self.pl.stats.max_hp * 0.3;
            if let Some(a) = self.w.actor.get_mut(pe) {
                a.act = Act::new(ActKind::Seized, 1.8);
            }
            self.deal(pe, None, dmg, 0.0, Vec3::ZERO, super::combat::DmgKind::True, 1.0);
            self.parts.burst(Fx::Strain, p, Vec3::ZERO, 60, 2.0);
            self.parts.burst(Fx::Blood, p + Vec3::Y, Vec3::Y, 20, 1.5);
            self.audio.play(Sfx::Rupture, 0.9, 0.7);
            self.audio.play(Sfx::Shriek, 0.6, 0.6);
            self.shake(0.8);
            self.flash = 0.4;
            self.msg("STRAIN OVERLOAD - THE VIRUS TAKES THE WHEEL");
            self.aoe(p, 3.5, 30.0 * self.pl.stats.spell_power, 40.0, Team::Player, Some(pe), super::combat::DmgKind::Spell);
        }
    }

    fn start_dodge(&mut self, wish: Vec3) {
        let st = &self.pl.stats;
        let cost = 18.0 * st.dodge_cost * if st.has(|e| matches!(e, Effect::DoubleDodge)) { 0.75 } else { 1.0 };
        let pe = self.player;
        let costs_hp = st.has(|e| matches!(e, Effect::DodgeCostsHp));
        if costs_hp {
            let a = self.w.actor.get_mut(pe).unwrap();
            if a.hp <= cost * 0.5 + 1.0 {
                return;
            }
            a.hp -= cost * 0.5;
        } else if !self.spend_stamina(cost) {
            return;
        }
        let body = *self.w.body.get(pe).unwrap();
        let dir = if wish.length() > 0.1 { wish.normalize() } else { -yaw_dir(body.yaw) };
        let blink = self.pl.stats.has(|e| matches!(e, Effect::DodgeBlink));
        let phase = self.pl.stats.has(|e| matches!(e, Effect::Phase));
        let dur = if self.pl.stats.has(|e| matches!(e, Effect::DoubleDodge)) { 0.62 } else { 0.5 };
        let mut act = Act::new(ActKind::Dodge, dur);
        act.dir = dir;
        self.w.actor.get_mut(pe).unwrap().act = act;
        if let Some(b) = self.w.body.get_mut(pe) {
            b.ghost = phase;
        }
        self.audio.play_at(Sfx::Dodge, body.pos, 0.5, 1.0);
        self.parts.burst(Fx::Dust, body.pos, -dir, 5, 1.0);
        if blink {
            // a blink: jump most of the distance instantly
            let to = body.pos + dir * 4.5;
            let (np, _) = crate::physics::Statics::move_cyl(&self.area, body.pos, to - body.pos, body.radius, body.height);
            self.parts.burst(Fx::Rift, body.center(), Vec3::ZERO, 20, 1.0);
            if let Some(b) = self.w.body.get_mut(pe) {
                b.pos = np;
                b.prev = np;
            }
            self.parts.burst(Fx::Rift, np + Vec3::Y, Vec3::ZERO, 20, 1.0);
            self.audio.play_at(Sfx::Rift, np, 0.5, 1.6);
            self.pl.strain += 3.0;
        }
        let pos = body.pos;
        let spores = self.pl.stats.sum(|e| if let Effect::DodgeSpores(x) = e { Some(*x) } else { None });
        if spores > 0.0 {
            self.spawn_zone(pos, ZoneKind::Spores, 2.8, spores, 5.0, Team::Player, Some(pe));
        }
        let fire = self.pl.stats.sum(|e| if let Effect::DodgeFire(x) = e { Some(*x) } else { None });
        if fire > 0.0 {
            self.spawn_zone(pos, ZoneKind::Fire, 2.0, fire, 4.0, Team::Player, Some(pe));
        }
        let shock = self.pl.stats.sum(|e| if let Effect::DodgeShock(x) = e { Some(*x) } else { None });
        if shock > 0.0 {
            self.aoe(pos, 3.0, shock, 25.0, Team::Player, Some(pe), super::combat::DmgKind::Spell);
            self.parts.burst(Fx::Rift, pos + Vec3::Y * 0.5, Vec3::ZERO, 24, 1.5);
        }
    }

    fn start_light(&mut self) {
        if self.try_rupture() {
            return;
        }
        let t = &self.t;
        let dual = self.pl.dual(t);
        let left = dual && self.pl.next_left;
        let (class, _) = self.pl.hand_weapon(t, left);
        let def = t.class(class).clone();
        let combo = if self.pl.combo_t > 0.0 { self.pl.hits_landed_combo() } else { 0 };
        let mv = def.light[combo as usize % def.light.len()];
        let mut cost = mv.stam;
        if self.pl.two_handing(t) {
            cost *= 1.1;
        }
        if !self.spend_stamina(cost) {
            return;
        }
        let rate = self.attack_rate();
        let act = Act::attack(mv, left, false, combo, rate);
        let pe = self.player;
        self.w.actor.get_mut(pe).unwrap().act = act;
        self.pl.combo_t = mv.total() / rate + 0.45;
        self.pl.combo_index = (combo + 1) % def.light.len() as u8;
        if dual {
            self.pl.next_left = !self.pl.next_left;
        }
        let p = self.ppos();
        self.audio.play_at(Sfx::Swing, p, 0.4, 1.1 / rate.sqrt());
    }

    fn start_heavy(&mut self) {
        let t = &self.t;
        let (class, _) = self.pl.hand_weapon(t, false);
        let mv = t.class(class).heavy;
        let free = self.pl.stats.has(|e| matches!(e, Effect::HeavyFreeAtStrain)) && self.pl.strain_frac() > 0.5;
        if !free && !self.spend_stamina(mv.stam) {
            return;
        }
        let rate = self.attack_rate();
        let mut act = Act::attack(mv, false, true, 0, rate);
        act.charging = true;
        let pe = self.player;
        self.w.actor.get_mut(pe).unwrap().act = act;
        self.pl.combo_t = 0.0;
        let p = self.ppos();
        self.audio.play_at(Sfx::SwingHeavy, p, 0.5, 0.9);
    }

    fn start_heal(&mut self) {
        if self.pl.heal_charges == 0 || self.pl.stats.has(|e| matches!(e, Effect::NoHeal)) {
            self.msg("NO HEAL CHARGES");
            return;
        }
        self.pl.heal_charges -= 1;
        let pe = self.player;
        self.w.actor.get_mut(pe).unwrap().act = Act::new(ActKind::Heal, 1.0);
    }

    fn start_cast(&mut self) {
        let spells = self.pl.spells(&self.t);
        if spells.is_empty() {
            self.msg("NO CATALYST - CASTING NEEDS A CATALYST OR A MUTATED HAND");
            return;
        }
        let si = spells[self.pl.spell_idx % spells.len()];
        let d = &self.t.spells[si];
        let mut act = Act::new(ActKind::Cast, d.cast / self.attack_rate().sqrt() + 0.35);
        act.spell = si;
        let pe = self.player;
        self.w.actor.get_mut(pe).unwrap().act = act;
        let p = self.ppos();
        self.audio.play_at(Sfx::Cast, p, 0.5, 1.0);
    }

    pub fn attack_rate(&self) -> f32 {
        let mut r = self.pl.stats.atk_speed;
        if self.pl.frenzy > 0.0 {
            r += self.pl.stats.sum(|e| if let Effect::KillFrenzy(x) = e { Some(*x) } else { None }) / 100.0;
        }
        if matches!(self.pl.affliction, Some((Affliction::Tremor, _))) {
            r *= 0.75;
        }
        r
    }

    /// Advance the player's current action (called from the generic action system).
    pub fn player_act_events(&mut self, prev_t: f32) {
        let pe = self.player;
        let a = self.w.actor.get(pe).unwrap().act.clone();
        match a.kind {
            ActKind::Heal if prev_t < 0.5 && a.t >= 0.5 => {
                let st = &self.pl.stats;
                let mult = self.rift_heal_mult();
                let amount = st.max_hp * 0.45 * st.heal_power * mult;
                let slow = st.has(|e| matches!(e, Effect::HealRegen));
                let p = self.ppos();
                if slow {
                    self.pl.heal_regen = 4.5 * st.heal_power * mult;
                } else {
                    let ac = self.w.actor.get_mut(pe).unwrap();
                    ac.hp = (ac.hp + amount).min(ac.max_hp);
                }
                let vent = self.pl.stats.sum(|e| if let Effect::HealStrain(x) = e { Some(*x) } else { None });
                self.pl.strain = (self.pl.strain - vent).max(0.0);
                let burst = self.pl.stats.sum(|e| if let Effect::HealBurst(x) = e { Some(*x) } else { None });
                if burst > 0.0 {
                    self.aoe(p, 4.0, burst, 30.0, Team::Player, Some(pe), super::combat::DmgKind::Rot);
                    self.parts.burst(Fx::Spore, p + Vec3::Y, Vec3::ZERO, 40, 3.0);
                }
                self.parts.burst(Fx::Heal, p, Vec3::ZERO, 30, 1.0);
                self.audio.play_at(Sfx::Heal, p, 0.6, 1.0);
            }
            ActKind::Cast if !a.fired && a.t >= a.dur - 0.3 => {
                self.w.actor.get_mut(pe).unwrap().act.fired = true;
                self.cast_spell(pe, a.spell, true);
            }
            _ => {}
        }
    }
}

impl PlayerState {
    pub fn hits_landed_combo(&self) -> u8 {
        self.combo_index
    }
}
