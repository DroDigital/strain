//! Procedural enemy assembly and spawning. Infected are built from modular parts selected by
//! infection stage, then altered by affixes, so silhouettes vary enormously. Also spawns soldiers,
//! civilians, apex minibosses, rift bosses, risen husks, allied larvae and loot drops.
use super::comp::*;
use super::loot::{gen_item, roll_rarity, Rarity};
use super::Game;
use crate::anim::{ArmDef, BodyPlan, Rig};
use crate::data::{AffixKind, AiKind, ArmKind, Attach, Growth, GrowthKind, HeadKind, LegKind, Source};
use crate::ecs::Entity;
use crate::render::textures::Tex;
use crate::util::{chance, pick, rf, seed_rng, Rng};
use crate::world::{AreaKind, DState, SpawnKind, DISTRICTS};
use glam::Vec3;

pub const STAGE_NAMES: [&str; 5] = ["", "CARRIER", "SHIFTED", "BLOOMED", "APEX"];

fn stage_roll(r: &mut Rng, state: Option<DState>) -> u8 {
    let w: [f32; 3] = match state {
        Some(DState::Held) => [0.9, 0.1, 0.0],
        Some(DState::Contested) => [0.68, 0.24, 0.08],
        Some(DState::Lost) => [0.46, 0.34, 0.2],
        None => [0.4, 0.38, 0.22],
    };
    let x = r.f32();
    if x < w[0] {
        1
    } else if x < w[0] + w[1] {
        2
    } else {
        3
    }
}

impl Game {
    /// Level for an enemy spawned at `p`.
    pub fn level_at(&self, p: Vec3, r: &mut Rng) -> u32 {
        match self.area.kind {
            AreaKind::Rift(i) => crate::world::rift::RULES[i].level + r.u32(0..3),
            AreaKind::City => match crate::world::district_at(p) {
                Some(d) => {
                    let (a, b) = DISTRICTS[d].levels;
                    r.u32(a..=b)
                }
                None => 1,
            },
        }
    }
    fn state_here(&self, p: Vec3) -> Option<DState> {
        match self.area.kind {
            AreaKind::Rift(_) => None,
            AreaKind::City => crate::world::district_at(p).map(|d| DISTRICTS[d].state),
        }
    }

    /// Build a body plan for an archetype at a stage.
    fn assemble(&self, arch: usize, stage: u8, r: &mut Rng) -> BodyPlan {
        let t = &self.t;
        let a = &t.archetypes[arch];
        let pt = &t.parts;
        let mut p = BodyPlan::human(r.u32(..));
        let in_stage = |s: (u8, u8)| stage >= s.0 && stage <= s.1;
        let torsos: Vec<&crate::data::TorsoPart> = pt.torsos.iter().filter(|x| a.torsos.contains(&x.id) && in_stage(x.stage)).collect();
        let torso = if torsos.is_empty() { &pt.torsos[0] } else { *pick(r, &torsos) };
        p.torso = Vec3::new(torso.size.0, torso.size.1, torso.size.2) * rf(r, 0.9, 1.12);
        p.torso_tex = torso.tex;
        p.hunch = torso.hunch + (stage as f32 - 1.0) * 0.08;
        let heads: Vec<&crate::data::HeadPart> = pt.heads.iter().filter(|h| in_stage(h.stage)).collect();
        let head = *pick(r, &heads);
        p.head = head.kind;
        p.head_tex = head.tex;
        p.head_size = head.size * rf(r, 0.9, 1.1);
        // arms: archetype decides the kinds, parts decide the look; later stages go asymmetric
        p.arms.clear();
        if !a.arms.is_empty() {
            for side in [1.0f32, -1.0] {
                let kind = if side > 0.0 || stage >= 2 { *pick(r, &a.arms) } else { a.arms[0] };
                let part = pt.arms.iter().find(|x| x.kind == kind).unwrap_or(&pt.arms[0]);
                p.arms.push(ArmDef { side, kind, tex: part.tex, len: part.len * rf(r, 0.9, 1.2), thick: part.thick, at: 1.0 });
            }
            if stage >= 3 && chance(r, 0.4) {
                let kind = *pick(r, &[ArmKind::Claw, ArmKind::Tendril, ArmKind::Blade]);
                let part = pt.arms.iter().find(|x| x.kind == kind).unwrap();
                p.arms.push(ArmDef { side: if chance(r, 0.5) { 1.0 } else { -1.0 }, kind, tex: part.tex, len: part.len, thick: part.thick * 0.8, at: 0.45 });
            }
        }
        let lk = *pick(r, &a.legs);
        let legs: Vec<&crate::data::LegPart> = pt.legs.iter().filter(|l| l.kind == lk && in_stage(l.stage)).collect();
        let leg = if legs.is_empty() { pt.legs.iter().find(|l| l.kind == lk).unwrap() } else { *pick(r, &legs) };
        p.leg_kind = leg.kind;
        p.leg_tex = leg.tex;
        p.leg_len = leg.len * rf(r, 0.92, 1.1);
        p.leg_thick = leg.thick;
        p.leg_count = leg.count;
        // growths by stage
        let n = match stage {
            1 => r.usize(0..2),
            2 => r.usize(1..4),
            3 => r.usize(3..6),
            _ => r.usize(5..8),
        };
        let pool: Vec<&(GrowthKind, Attach, Tex, u8)> = pt.growths.iter().filter(|g| g.3 <= stage).collect();
        for _ in 0..n {
            let g = *pick(r, &pool);
            p.growths.push(Growth { kind: g.0, at: g.1, tex: g.2, size: rf(r, 0.7, 1.3) });
        }
        // colours: stage I keeps clothes; later stages lose them
        let pale = rf(r, 0.6, 0.95);
        p.skin = match stage {
            1 => [pale, pale * 0.82, pale * 0.75],
            2 => [pale * 0.85, pale * 0.7, pale * 0.8],
            _ => [pale * 0.8, pale * 0.6, pale * 0.85],
        };
        p.cloth = [rf(r, 0.15, 0.55), rf(r, 0.15, 0.5), rf(r, 0.15, 0.55)];
        p.legs_tint = if leg.tex == Tex::Cloth { [rf(r, 0.1, 0.35), rf(r, 0.1, 0.35), rf(r, 0.15, 0.45)] } else { p.skin };
        p.glow = match stage {
            1 => [0.6, 1.0, 0.7],
            2 => [0.9, 0.5, 1.0],
            3 => [1.0, 0.4, 0.5],
            _ => [1.0, 0.8, 0.3],
        };
        p.scale = rf(r, a.scale.0, a.scale.1) * (1.0 + (stage as f32 - 1.0) * 0.06);
        p
    }

    fn civilian_plan(r: &mut Rng) -> BodyPlan {
        let mut p = BodyPlan::human(r.u32(..));
        p.cloth = [rf(r, 0.15, 0.7), rf(r, 0.15, 0.6), rf(r, 0.15, 0.6)];
        p.legs_tint = [rf(r, 0.1, 0.35), rf(r, 0.1, 0.35), rf(r, 0.15, 0.45)];
        let s = rf(r, 0.55, 1.0);
        p.skin = [s, s * 0.8, s * 0.68];
        p.head = if chance(r, 0.3) { HeadKind::Hood } else { HeadKind::Human };
        p.scale = rf(r, 0.9, 1.05);
        p
    }

    fn insert_actor(&mut self, pos: Vec3, plan: BodyPlan, actor: Actor, ai: Ai, foe: Option<Foe>) -> Entity {
        let e = self.w.spawn();
        let s = plan.scale;
        let wide = plan.leg_kind == LegKind::Spider;
        let mut b = Body::new(pos, (0.38 * s * if wide { 1.4 } else { 1.0 }).max(0.25), plan.height());
        b.mass = s * s * if actor.max_poise > 70.0 { 2.0 } else { 1.0 };
        b.yaw = rf(&mut self.rng, -3.0, 3.0);
        b.prev_yaw = b.yaw;
        self.w.body.insert(e, b);
        self.w.rig.insert(e, Rig::new(plan));
        self.w.actor.insert(e, actor);
        self.w.ai.insert(e, ai);
        if let Some(f) = foe {
            self.w.foe.insert(e, f);
        }
        e
    }

    /// Spawn an infected archetype. `stage` 0 = roll by location.
    pub fn spawn_archetype(&mut self, id: &str, pos: Vec3, stage: u8, level: u32, origin: Option<((i32, i32), usize)>) -> Entity {
        let arch = self.t.archetype(id);
        let mut r = Rng::with_seed(self.rng.u64(..));
        let stage = if stage == 0 { stage_roll(&mut r, self.state_here(pos)) } else { stage };
        self.spawn_infected(arch, pos, stage, level, origin, &mut r, None)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn spawn_infected(&mut self, arch: usize, pos: Vec3, stage: u8, level: u32, origin: Option<((i32, i32), usize)>, r: &mut Rng, fixed_affixes: Option<Vec<AffixKind>>) -> Entity {
        let a = self.t.archetypes[arch].clone();
        let mut plan = self.assemble(arch, stage, r);
        // affixes
        let mut affixes: Vec<AffixKind> = Vec::new();
        if let Some(f) = fixed_affixes {
            affixes = f;
        } else {
            let n = match stage {
                2 => r.usize(1..3),
                3 => r.usize(3..5),
                _ => 0,
            };
            let pool: Vec<AffixKind> = self.t.affixes.iter().filter(|x| x.min_level <= level && !(x.kind == AffixKind::Mimic && a.ai == AiKind::Crawler)).map(|x| x.kind).collect();
            let mut guard = 0;
            while affixes.len() < n && guard < 40 {
                guard += 1;
                let k = *pick(r, &pool);
                if !affixes.contains(&k) {
                    affixes.push(k);
                }
            }
        }
        let lv = level as f32 - 1.0;
        let smult = [1.0, 1.0, 1.7, 2.8, 6.0][stage.min(4) as usize];
        let mut hp = a.hp * (1.0 + lv * 0.13) * smult;
        let mut dmg = a.dmg * (1.0 + lv * 0.085) * (1.0 + (stage as f32 - 1.0) * 0.15);
        let mut speed = a.speed;
        let mut scale = 1.0;
        for k in &affixes {
            let d = self.t.affix(*k);
            hp *= d.hp;
            dmg *= d.dmg;
            speed *= d.speed;
            scale *= d.scale;
            plan.growths.push(Growth { kind: d.vis.0, at: d.vis.1, tex: d.vis.2, size: 1.0 });
        }
        plan.scale *= scale;
        let mut actor = Actor::new(Team::Infected, hp, a.poise * (1.0 + (stage as f32 - 1.0) * 0.4) * scale, dmg, speed, level);
        actor.armor = lv * 1.5;
        if affixes.contains(&AffixKind::Cindered) {
            actor.fire_immune = true;
            actor.burn = 0.0;
        }
        let biomass = a.biomass * [1.0, 1.0, 2.2, 4.5, 12.0][stage.min(4) as usize] * (1.0 + lv * 0.12);
        let mimic = affixes.contains(&AffixKind::Mimic);
        let foe = Foe {
            arch,
            name: a.name.clone(),
            stage,
            affixes: affixes.clone(),
            attacks: a.attacks.clone(),
            biomass,
            apex: None,
            boss: None,
            husk: false,
            source: Source::Biological,
            plates: if affixes.contains(&AffixKind::Calcified) { hp * 0.35 } else { 0.0 },
            undying_used: false,
            mimic,
            phase_t: rf(r, 0.0, 4.0),
            aff_t: rf(r, 0.0, 3.0),
            shrieked: false,
            origin,
            ttl: 0.0,
            can_split: true,
            transform_t: 0.0,
        };
        let ai = Ai::new(a.ai, pos, r.u32(..));
        let shown = if mimic {
            let real = plan.clone();
            let disguise = Self::civilian_plan(r);
            self.mimic_plans.insert(0, real);
            disguise
        } else {
            plan
        };
        let e = self.insert_actor(pos, shown, actor, ai, Some(foe));
        if mimic {
            let real = self.mimic_plans.remove(&0).unwrap();
            self.mimic_plans.insert(e.id, real);
        }
        e
    }

    pub fn reveal_mimic(&mut self, e: Entity) {
        let Some(f) = self.w.foe.get_mut(e) else { return };
        if !f.mimic {
            return;
        }
        f.mimic = false;
        if let Some(plan) = self.mimic_plans.remove(&e.id) {
            if let Some(r) = self.w.rig.get_mut(e) {
                *r = Rig::new(plan);
            }
        }
        let p = self.w.body.get(e).unwrap().center();
        self.parts.burst(crate::particles::Fx::Ichor, p, glam::Vec3::Y, 40, 2.0);
        self.audio.play_at(crate::audio::Sfx::Shriek, p, 0.8, 1.0);
        self.float_text(p + Vec3::Y, "MIMIC", [1.0, 0.4, 0.4]);
        if let Some(ai) = self.w.ai.get_mut(e) {
            ai.target = Some(self.player);
            ai.alert = 10.0;
        }
    }

    pub fn spawn_split(&mut self, parent: Entity, pos: Vec3) {
        let Some(f) = self.w.foe.get(parent).cloned() else { return };
        let level = self.w.actor.get(parent).map(|a| a.level).unwrap_or(1);
        let mut r = Rng::with_seed(self.rng.u64(..));
        let stage = f.stage.saturating_sub(1).max(1);
        let aff: Vec<AffixKind> = f.affixes.iter().copied().filter(|a| *a != AffixKind::Splitting && *a != AffixKind::Colossal).collect();
        let e = self.spawn_infected(f.arch, pos, stage, level, None, &mut r, Some(aff));
        if let Some(rig) = self.w.rig.get_mut(e) {
            rig.plan.scale *= 0.7;
        }
        if let Some(a) = self.w.actor.get_mut(e) {
            a.max_hp *= 0.45;
            a.hp = a.max_hp;
        }
        if let Some(ff) = self.w.foe.get_mut(e) {
            ff.can_split = false;
            ff.biomass *= 0.3;
        }
        if let Some(ai) = self.w.ai.get_mut(e) {
            ai.target = Some(self.player);
            ai.alert = 10.0;
        }
    }

    pub fn spawn_soldier(&mut self, pos: Vec3, squad: u32, level: u32) -> Entity {
        let mut r = Rng::with_seed(self.rng.u64(..));
        let mut p = BodyPlan::human(r.u32(..));
        p.torso_tex = Tex::Camo;
        p.cloth = [1.0, 1.0, 1.0];
        p.leg_tex = Tex::Camo;
        p.legs_tint = [0.9, 0.9, 0.9];
        p.head = HeadKind::Helmet;
        p.arms[0].kind = ArmKind::Rifle;
        p.arms[0].tex = Tex::Camo;
        p.arms[1].tex = Tex::Camo;
        let s = rf(&mut r, 0.55, 0.95);
        p.skin = [s, s * 0.8, s * 0.7];
        p.glow = [1.0, 0.9, 0.6];
        let arch = self.t.archetype("soldier");
        let a = &self.t.archetypes[arch];
        let lv = level as f32;
        let actor = Actor::new(Team::Military, a.hp * (1.0 + lv * 0.12), a.poise, a.dmg * (1.0 + lv * 0.08), a.speed, level);
        let mut ai = Ai::new(AiKind::Soldier, pos, r.u32(..));
        ai.summons = squad;
        let foe = Foe { arch, name: "Soldier".into(), stage: 0, affixes: vec![], attacks: a.attacks.clone(), biomass: 0.0, apex: None, boss: None, husk: false, source: Source::Military, plates: 0.0, undying_used: false, mimic: false, phase_t: 0.0, aff_t: 0.0, shrieked: false, origin: None, ttl: 0.0, can_split: false, transform_t: 0.0 };
        self.insert_actor(pos, p, actor, ai, Some(foe))
    }

    pub fn spawn_civilian(&mut self, pos: Vec3) -> Entity {
        let mut r = Rng::with_seed(self.rng.u64(..));
        let p = Self::civilian_plan(&mut r);
        let arch = self.t.archetype("civilian");
        let a = &self.t.archetypes[arch];
        let actor = Actor::new(Team::Civilian, a.hp, a.poise, a.dmg, a.speed, 1);
        let ai = Ai::new(AiKind::Civilian, pos, r.u32(..));
        let foe = Foe { arch, name: "Civilian".into(), stage: 0, affixes: vec![], attacks: a.attacks.clone(), biomass: 0.0, apex: None, boss: None, husk: false, source: Source::Civilian, plates: 0.0, undying_used: false, mimic: false, phase_t: 0.0, aff_t: 0.0, shrieked: false, origin: None, ttl: 0.0, can_split: false, transform_t: 0.0 };
        self.insert_actor(pos, p, actor, ai, Some(foe))
    }

    pub fn spawn_brood(&mut self, pos: Vec3, ttl: f32) -> Entity {
        let mut r = Rng::with_seed(self.rng.u64(..));
        let arch = self.t.archetype("brood");
        let mut plan = self.assemble(arch, 1, &mut r);
        plan.glow = [0.5, 1.0, 0.6];
        plan.skin = [0.7, 0.9, 0.7];
        let a = self.t.archetypes[arch].clone();
        let lvl = self.pl.life.level;
        let actor = Actor::new(Team::Ally, a.hp * (1.0 + lvl as f32 * 0.1), a.poise, a.dmg * (1.0 + lvl as f32 * 0.1) * self.pl.stats.spell_power, a.speed, lvl);
        let ai = Ai::new(AiKind::Ally, pos, r.u32(..));
        let foe = Foe { arch, name: "Brood".into(), stage: 1, affixes: vec![], attacks: a.attacks.clone(), biomass: 0.0, apex: None, boss: None, husk: false, source: Source::Biological, plates: 0.0, undying_used: false, mimic: false, phase_t: 0.0, aff_t: 0.0, shrieked: false, origin: None, ttl, can_split: false, transform_t: 0.0 };
        let e = self.insert_actor(pos, plan, actor, ai, Some(foe));
        self.parts.burst(crate::particles::Fx::Ichor, pos + Vec3::Y * 0.3, Vec3::Y, 12, 1.0);
        e
    }

    pub fn spawn_apex(&mut self, idx: usize, pos: Vec3, origin: Option<((i32, i32), usize)>) -> Option<Entity> {
        let ap = self.t.apex.get(idx)?.clone();
        let arch = self.t.archetype(&ap.archetype);
        let mut r = seed_rng(idx as u32, self.area.epoch, 4);
        let level = self.level_at(pos, &mut r) + 2;
        let e = self.spawn_infected(arch, pos, 4, level, origin, &mut r, Some(ap.affixes.clone()));
        if let Some(rig) = self.w.rig.get_mut(e) {
            rig.plan.scale = ap.scale;
            rig.plan.head = ap.head;
            rig.plan.skin = [rig.plan.skin[0] * ap.tint.0, rig.plan.skin[1] * ap.tint.1, rig.plan.skin[2] * ap.tint.2];
            for g in &ap.growths {
                rig.plan.growths.push(Growth { kind: g.0, at: g.1, tex: g.2, size: 1.3 });
            }
        }
        if let Some(a) = self.w.actor.get_mut(e) {
            a.max_hp *= ap.hp / 6.0 * 2.0;
            a.hp = a.max_hp;
            a.max_poise *= 1.5;
            a.poise = a.max_poise;
        }
        if let Some(b) = self.w.body.get_mut(e) {
            b.radius *= ap.scale.min(1.5);
            b.height *= ap.scale;
            b.mass = 4.0;
        }
        if let Some(f) = self.w.foe.get_mut(e) {
            f.apex = Some(idx);
            f.name = ap.name.clone();
            f.mimic = false;
        }
        if self.mimic_plans.contains_key(&e.id) {
            self.mimic_plans.remove(&e.id);
        }
        Some(e)
    }

    pub fn spawn_boss(&mut self, idx: usize, pos: Vec3) -> Option<Entity> {
        if self.prof.bosses[idx] && self.area.kind == AreaKind::Rift(idx) {
            return None;
        }
        let bd = self.t.bosses[idx].clone();
        let mut p = BodyPlan::human(idx as u32 * 77 + 5);
        p.scale = bd.scale;
        p.torso = Vec3::new(bd.torso.0, bd.torso.1, bd.torso.2);
        p.torso_tex = bd.tex;
        p.hunch = 0.25;
        p.head = bd.head;
        p.head_tex = bd.tex;
        p.head_size = 1.3;
        p.arms.clear();
        for (i, k) in bd.arms.iter().enumerate() {
            let part = self.t.parts.arms.iter().find(|x| x.kind == *k).unwrap();
            let side = if i % 2 == 0 { 1.0 } else { -1.0 };
            p.arms.push(ArmDef { side, kind: *k, tex: if part.tex == Tex::Skin { bd.tex } else { part.tex }, len: part.len * 1.2, thick: part.thick * 1.2, at: 1.0 - (i / 2) as f32 * 0.4 });
        }
        let lp = self.t.parts.legs.iter().find(|l| l.kind == bd.legs.0).unwrap();
        p.leg_kind = bd.legs.0;
        p.leg_count = bd.legs.1;
        p.leg_tex = lp.tex;
        p.leg_len = lp.len;
        p.leg_thick = lp.thick * 1.3;
        p.skin = [bd.tint.0, bd.tint.1, bd.tint.2];
        p.legs_tint = p.skin;
        p.glow = [bd.tint.0.min(1.0), bd.tint.1.min(1.0), bd.tint.2.min(1.0)];
        for g in &bd.growths {
            p.growths.push(Growth { kind: g.0, at: g.1, tex: g.2, size: 1.2 });
        }
        let level = match self.area.kind {
            AreaKind::Rift(i) => crate::world::rift::RULES[i].level + 3,
            _ => 20,
        };
        let mut actor = Actor::new(Team::Infected, bd.hp * (1.0 + level as f32 * 0.03), 160.0, bd.dmg, 3.4, level);
        actor.armor = 20.0;
        let mut ai = Ai::new(AiKind::Boss, pos, idx as u32);
        ai.cd = 2.0;
        let foe = Foe { arch: self.t.archetype(&bd.summons), name: bd.name.clone(), stage: 4, affixes: vec![], attacks: bd.attacks.clone(), biomass: 300.0 + level as f32 * 20.0, apex: None, boss: Some(idx), husk: false, source: Source::Rift, plates: 0.0, undying_used: false, mimic: false, phase_t: 0.0, aff_t: 0.0, shrieked: true, origin: None, ttl: 0.0, can_split: false, transform_t: 0.0 };
        let e = self.insert_actor(pos, p, actor, ai, Some(foe));
        if let Some(b) = self.w.body.get_mut(e) {
            b.radius = 0.5 * bd.scale;
            b.mass = 12.0;
            b.yaw = std::f32::consts::PI;
        }
        self.boss = Some(e);
        Some(e)
    }

    /// The husk of a lost life rises with that build.
    pub fn spawn_risen_husk(&mut self, pos: Vec3, life: &super::player::Life, gear_r: Option<super::loot::Item>) -> Entity {
        let mut ghost = super::player::PlayerState::new(self.pl.gear.clone());
        ghost.life = life.clone();
        ghost.gear.eq[0] = gear_r.clone();
        let mut plan = super::player::player_plan(&self.t, &ghost);
        plan.skin = [0.45, 0.4, 0.5];
        plan.glow = [1.0, 0.3, 0.4];
        plan.growths.push(Growth { kind: GrowthKind::Cracks, at: Attach::Chest, tex: Tex::Vein, size: 1.2 });
        let class = gear_r.as_ref().and_then(|i| i.class(&self.t)).unwrap_or(crate::data::WeaponClass::Fist);
        let mut attacks = self.t.class(class).light.clone();
        attacks.push(self.t.class(class).heavy);
        let lv = life.level.max(3);
        let actor = Actor::new(Team::Infected, 120.0 + lv as f32 * 30.0, 60.0, 14.0 + lv as f32 * 2.2, 4.4, lv);
        let ai = Ai::new(AiKind::Husk, pos, 99);
        let foe = Foe { arch: self.t.archetype("rusher"), name: "Your Husk".into(), stage: 4, affixes: vec![AffixKind::Frenzied], attacks, biomass: 40.0 + lv as f32 * 25.0, apex: None, boss: None, husk: true, source: Source::Biological, plates: 0.0, undying_used: false, mimic: false, phase_t: 0.0, aff_t: 0.0, shrieked: true, origin: None, ttl: 0.0, can_split: false, transform_t: 0.0 };
        let e = self.insert_actor(pos, plan, actor, ai, Some(foe));
        if let Some(ai) = self.w.ai.get_mut(e) {
            ai.target = Some(self.player);
        }
        e
    }

    /// Populate a chunk's spawn points.
    pub fn populate(&mut self, key: (i32, i32)) {
        let Some(chunk) = self.area.chunks.get(&key) else { return };
        let spawns = chunk.spawns.clone();
        let interacts = chunk.interact.clone();
        let hazards = chunk.hazards.clone();
        let mut ents = Vec::new();
        for (i, sp) in spawns.iter().enumerate() {
            if self.consumed.contains(&(key, i)) {
                continue;
            }
            let mut r = seed_rng(sp.seed, i as u32, self.area.epoch);
            let origin = Some((key, i));
            let pos = sp.pos + Vec3::Y * self.area.ground(sp.pos.x, sp.pos.z);
            match sp.kind {
                SpawnKind::Infected => {
                    let level = self.level_at(pos, &mut r);
                    let state = self.state_here(pos);
                    let mix: Vec<usize> = match self.area.kind {
                        AreaKind::Rift(ri) => crate::world::rift::RULES[ri].mix.iter().map(|id| self.t.archetype(id)).collect(),
                        AreaKind::City => (0..self.t.archetypes.len()).filter(|&k| {
                            let a = &self.t.archetypes[k];
                            a.min_level <= level && !matches!(a.ai, AiKind::Soldier | AiKind::Civilian | AiKind::Ally)
                        }).collect(),
                    };
                    let arch = *pick(&mut r, &mix);
                    let g = self.t.archetypes[arch].group;
                    let n = r.u32(g.0..=g.1);
                    let mut stage = stage_roll(&mut r, state);
                    if self.pl.stats.has(|e| matches!(e, crate::data::Effect::PitHunger)) && stage < 3 && chance(&mut r, 0.5) {
                        stage += 1;
                    }
                    for k in 0..n {
                        let off = Vec3::new((k as f32 * 2.4).cos(), 0.0, (k as f32 * 2.4).sin()) * (k as f32).sqrt() * 1.2;
                        let st = if k == 0 { stage } else { stage_roll(&mut r, state).min(stage) };
                        let e = self.spawn_infected(arch, pos + off, st, level, if k == 0 { origin } else { None }, &mut r, None);
                        ents.push(e);
                    }
                }
                SpawnKind::Soldiers => {
                    let n = r.u32(2..5);
                    let squad = r.u32(..);
                    let level = self.level_at(pos, &mut r);
                    for k in 0..n {
                        let off = Vec3::new(k as f32 * 1.3 - 2.0, 0.0, (k % 2) as f32 * 1.2);
                        ents.push(self.spawn_soldier(pos + off, squad, level));
                    }
                }
                SpawnKind::Civilians => {
                    for k in 0..r.u32(1..3) {
                        ents.push(self.spawn_civilian(pos + Vec3::new(k as f32, 0.0, 0.5)));
                    }
                }
                SpawnKind::Apex(i) => {
                    if let Some(e) = self.spawn_apex(i, pos, origin) {
                        ents.push(e);
                    }
                }
                SpawnKind::Boss(i) => {
                    if let Some(e) = self.spawn_boss(i, pos) {
                        ents.push(e);
                    }
                }
                SpawnKind::Cache => {
                    let level = self.level_at(pos, &mut r);
                    let srcs: Vec<Source> = match (self.area.kind, self.state_here(pos)) {
                        (AreaKind::Rift(_), _) => vec![Source::Rift, Source::Biological],
                        (_, Some(DState::Lost)) => vec![Source::Biological, Source::Rift, Source::Civilian],
                        (_, Some(DState::Held)) => vec![Source::Military, Source::Civilian],
                        _ => vec![Source::Military, Source::Civilian, Source::Biological],
                    };
                    let rar = roll_rarity(&mut r, 0.6 + self.pl.stats.luck);
                    let it = gen_item(&self.t, &mut r, level, &srcs, rar, None);
                    let e = self.drop_item(pos + Vec3::Y * 0.3, it);
                    if let Some(f) = self.w.pickup.get_mut(e) {
                        f.t = 1.0;
                    }
                    self.cache_origin.insert(e.id, (key, i));
                    ents.push(e);
                }
            }
        }
        for (p, k) in interacts {
            let e = self.w.spawn();
            let mut b = Body::new(p, 0.5, 1.0);
            b.solid = false;
            self.w.body.insert(e, b);
            self.w.interact.insert(e, Interact { kind: k });
            ents.push(e);
        }
        for (p, r, k) in hazards {
            let kind = match k {
                0 => ZoneKind::Acid,
                1 => ZoneKind::Fire,
                2 => ZoneKind::Spores,
                _ => ZoneKind::Singing,
            };
            let dps = match kind {
                ZoneKind::Singing => 6.0,
                _ => 8.0 + self.pl.life.level as f32 * 0.6,
            };
            let mut z = self.make_zone(p, kind, r, dps, 1.0, Team::Infected, None);
            z.permanent = true;
            ents.push(self.add_zone(p, z));
        }
        self.chunk_ents.insert(key, ents);
    }

    pub fn depopulate(&mut self, key: (i32, i32)) {
        let pp = self.ppos();
        if let Some(ents) = self.chunk_ents.remove(&key) {
            for e in ents {
                if !self.w.alive(e) || Some(e) == self.boss {
                    continue;
                }
                let far = self.w.body.get(e).map(|b| (b.pos - pp).length() > 70.0).unwrap_or(true);
                let engaged = self.w.ai.get(e).map(|ai| ai.target == Some(self.player)).unwrap_or(false);
                if far || !engaged {
                    if let Some(f) = self.w.foe.get(e) {
                        if f.husk {
                            continue;
                        }
                    }
                    self.mimic_plans.remove(&e.id);
                    self.cache_origin.remove(&e.id);
                    self.w.despawn(e);
                }
            }
        }
    }

    pub fn drop_item(&mut self, pos: Vec3, it: super::loot::Item) -> Entity {
        let e = self.w.spawn();
        let mut b = Body::new(pos, 0.3, 0.4);
        b.solid = false;
        b.push = crate::util::rand_dir(&mut self.rng) * 1.5;
        self.w.body.insert(e, b);
        let r = it.rarity;
        self.w.pickup.insert(e, Pickup { kind: PickKind::Item(it), t: 0.0 });
        if r >= Rarity::Epic {
            self.audio.play_at(crate::audio::Sfx::LevelUp, pos, 0.25, 2.0);
        }
        e
    }

    pub fn drop_loot(&mut self, e: Entity, f: &Foe, by_player: bool) {
        let pos = self.w.body.get(e).unwrap().pos;
        let level = self.w.actor.get(e).map(|a| a.level).unwrap_or(1);
        let team = self.w.actor.get(e).map(|a| a.team).unwrap_or(Team::Infected);
        if team == Team::Civilian || team == Team::Ally {
            return;
        }
        let mut r = Rng::with_seed(self.rng.u64(..));
        let luck = self.pl.stats.luck;
        let srcs: Vec<Source> = if team == Team::Military {
            vec![Source::Military]
        } else {
            match self.area.kind {
                AreaKind::Rift(_) => vec![Source::Rift, Source::Biological],
                _ => match self.state_here(pos) {
                    Some(DState::Lost) => vec![Source::Biological, Source::Rift, Source::Civilian],
                    _ => vec![Source::Biological, Source::Civilian, Source::Military],
                },
            }
        };
        if let Some(bi) = f.boss {
            for d in self.t.bosses[bi].drops.clone() {
                let it = super::loot::make_unique(&self.t, &mut r, level + 2, &d);
                self.drop_item(pos + Vec3::Y, it);
            }
            for _ in 0..2 {
                let it = gen_item(&self.t, &mut r, level + 2, &srcs, Rarity::Epic, None);
                self.drop_item(pos + Vec3::Y, it);
            }
            self.drop_scrip(pos, 300 + level * 10);
            return;
        }
        if !by_player && team != Team::Military {
            return;
        }
        let (chance_drop, bonus, n) = if f.apex.is_some() {
            (1.0, 4.0, 2)
        } else if f.husk {
            (1.0, 2.0, 1)
        } else {
            match f.stage {
                1 => (0.1, 0.0, 1),
                2 => (0.32, 0.8, 1),
                3 => (0.7, 1.8, 1),
                _ => (0.15, 0.0, 1),
            }
        };
        for k in 0..n {
            if chance(&mut r, chance_drop * (1.0 + luck * 0.5)) {
                let mut rar = roll_rarity(&mut r, bonus + luck);
                if f.apex.is_some() && k == 0 {
                    rar = if chance(&mut r, 0.4) { Rarity::Legendary } else { rar.max(Rarity::Epic) };
                }
                let it = gen_item(&self.t, &mut r, level, &srcs, rar, None);
                self.drop_item(pos + Vec3::Y * 0.5, it);
            }
        }
        if chance(&mut r, 0.3 + f.stage as f32 * 0.15) {
            self.drop_scrip(pos, r.u32(2..8) * f.stage.max(1) as u32 + level);
        }
    }

    fn drop_scrip(&mut self, pos: Vec3, amount: u32) {
        let e = self.w.spawn();
        let mut b = Body::new(pos + Vec3::Y * 0.4, 0.2, 0.3);
        b.solid = false;
        b.push = crate::util::rand_dir(&mut self.rng) * 1.2;
        self.w.body.insert(e, b);
        self.w.pickup.insert(e, Pickup { kind: PickKind::Scrip(amount), t: 0.0 });
    }
}

pub fn _unused(_: LegKind) {}
