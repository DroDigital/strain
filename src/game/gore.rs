//! Gore: dismemberment that differs per creature, gibs that fly and bleed, arterial spray from
//! stumps, blood that soaks into bodies and pools on the ground.
//!
//! A severed part is a set of bone subtrees. On the body those joints fold into the stump; the
//! part itself becomes a gib that renders the creature's own skinned mesh (with whatever growths,
//! plates and tumours were modelled onto that limb), frozen in the pose it was cut in and tumbling
//! as a rigid body.
use super::combat::DmgKind;
use super::comp::{Team, ZoneKind};
use super::Game;
use crate::anim::Cut;
use crate::particles::Fx;
use crate::physics::{Statics, SOLID};
use crate::render::mesh::{segment, Instance, Prim};
use crate::render::textures::Tex;
use crate::util::{rand_dir, rf};
use crate::world::Area;
use glam::{Mat4, Quat, Vec2, Vec3};

/// What happens at a wound beyond blood.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Special {
    None,
    /// stumps leak corrosive bile that pools and burns
    Acid,
    /// stumps and gibs smoulder and spit embers
    Embers,
    /// limbs shatter into glowing shards instead of tearing
    Crystal,
    /// larvae spill out of the wound
    Larvae,
    /// spore puffs from every wound
    Spores,
    /// black mist pours out
    Void,
    /// tumours burst with yellow pus
    Pus,
}

#[derive(Clone, Copy, Debug)]
pub struct GoreProfile {
    pub blood: Fx,
    pub chunk: Fx,
    /// severable groups: the first bone is the cut, every listed bone takes its subtree along
    pub parts: &'static [&'static [&'static str]],
    /// a crushed head bursts rather than coming off whole
    pub head_pops: bool,
    /// how long a stump keeps pumping, seconds
    pub bleed: f32,
    pub special: Special,
    /// multiplies the chance of a cut (armoured or massive bodies are harder to take apart)
    pub give: f32,
    /// decal tint for pools left by this body
    pub pool: [f32; 3],
}

const HUMAN: &[&[&str]] = &[&["Head"], &["LowerArm.L"], &["LowerArm.R"], &["UpperArm.L"], &["UpperArm.R"], &["LowerLeg.L", "Foot.L"], &["LowerLeg.R", "Foot.R"], &["UpperLeg.L", "Foot.L"], &["UpperLeg.R", "Foot.R"]];
const HALVES: &[&[&str]] = &[&["Head"], &["LowerArm.L"], &["LowerArm.R"], &["UpperArm.L"], &["UpperArm.R"], &["LowerLeg.L", "Foot.L"], &["LowerLeg.R", "Foot.R"], &["Torso"]];
const BRUTE: &[&[&str]] = &[&["Head"], &["UpperArm.L"], &["UpperArm.R"], &["Torso"]];
const PLATED: &[&[&str]] = &[&["Head"], &["LowerArm.L"], &["LowerArm.R"]];
const DOG: &[&[&str]] = &[&["Head"], &["FrontUpperLeg.L", "FF.L"], &["FrontUpperLeg.R", "FF.R"], &["BackUpperLeg.L", "FFB.L"], &["BackUpperLeg.R", "FFB.R"], &["Tail1"]];
const LARVA: &[&[&str]] = &[&["UpperLeg.L", "Foot.L"], &["UpperLeg.R", "Foot.R"], &["Abdomen"]];
const ARMS: &[&[&str]] = &[&["UpperArm.L"], &["UpperArm.R"], &["Head"]];
const CRAWLER: &[&[&str]] = &[&["Head"], &["LowerArm.L"], &["LowerArm.R"], &["UpperArm.L"], &["UpperArm.R"], &["Torso"]];

const RED: [f32; 3] = [0.2, 0.012, 0.012];
const DARK: [f32; 3] = [0.12, 0.02, 0.03];
const ICHOR: [f32; 3] = [0.1, 0.02, 0.09];

/// Every model bleeds and comes apart its own way.
pub fn profile(model: &str) -> GoreProfile {
    let p = |blood, chunk, parts, head_pops, bleed, special, give, pool| GoreProfile { blood, chunk, parts, head_pops, bleed, special, give, pool };
    match model {
        "Characters_GermanShepherd" => p(Fx::Blood, Fx::Bone, DOG, false, 3.0, Special::None, 1.0, RED),
        m if m.starts_with("Characters_") || m.starts_with("Human_") => p(Fx::Blood, Fx::Bone, HUMAN, true, 4.0, Special::None, 1.0, RED),
        "Strain_Carrier" => p(Fx::Blood, Fx::Bone, HALVES, true, 3.5, Special::None, 1.1, DARK),
        "Strain_Shambler" => p(Fx::Ichor, Fx::Bone, HUMAN, true, 3.0, Special::Spores, 1.1, ICHOR),
        "Strain_Bulk" => p(Fx::Ichor, Fx::Bone, BRUTE, true, 5.0, Special::Pus, 0.6, ICHOR),
        "Strain_Skitter" => p(Fx::Ichor, Fx::Bone, DOG, false, 2.5, Special::None, 1.2, ICHOR),
        "Strain_Retch" => p(Fx::Acid, Fx::Bone, ARMS, true, 4.0, Special::Acid, 1.0, [0.08, 0.14, 0.02]),
        // the crawler is a torso dragging itself: arms, head, or the whole upper body
        "Strain_Larva" => p(Fx::Blood, Fx::Bone, CRAWLER, true, 3.0, Special::None, 1.4, DARK),
        "Strain_Bulwark" => p(Fx::Ichor, Fx::Bone, PLATED, false, 2.5, Special::None, 0.5, ICHOR),
        "Strain_Lurker" => p(Fx::Ichor, Fx::Bone, HUMAN, true, 3.0, Special::Void, 1.1, DARK),
        "Strain_Matron" => p(Fx::Ichor, Fx::Bone, BRUTE, false, 4.0, Special::Larvae, 0.8, ICHOR),
        "Strain_Digestor" => p(Fx::Acid, Fx::Bone, BRUTE, false, 5.0, Special::Acid, 0.5, [0.08, 0.14, 0.02]),
        "Strain_Geometer" => p(Fx::Rift, Fx::Rift, ARMS, false, 1.5, Special::Crystal, 0.6, [0.05, 0.04, 0.12]),
        "Strain_MotherCinder" => p(Fx::Blood, Fx::Ember, BRUTE, false, 4.0, Special::Embers, 0.5, [0.06, 0.03, 0.02]),
        "Strain_Cantor" => p(Fx::Ichor, Fx::Bone, ARMS, true, 3.0, Special::Void, 0.6, DARK),
        "Strain_Seed" => p(Fx::Ichor, Fx::Bone, HUMAN, true, 3.0, Special::Spores, 0.7, ICHOR),
        "Zombie_Ribcage" => p(Fx::Blood, Fx::Bone, LARVA, false, 2.5, Special::None, 1.2, DARK),
        m if m.starts_with("Zombie_") => p(Fx::Blood, Fx::Bone, HUMAN, true, 3.0, Special::None, 1.1, DARK),
        _ => p(Fx::Blood, Fx::Bone, &[], false, 2.0, Special::None, 1.0, RED),
    }
}

/// How a blow cuts: `edge` 0 (blunt) .. 1 (blade), `force` 0..1+, and where it landed.
#[derive(Clone, Copy, Debug)]
pub struct CutCtx {
    pub edge: f32,
    pub force: f32,
    pub at: Vec3,
    pub blast: bool,
}

/// A severed part flying free.
#[derive(Clone, Debug)]
pub struct Gib {
    pub skin_id: u32,
    pub model: usize,
    pub joints: Vec<usize>,
    /// world-space joint matrices at the moment of the cut
    pub frozen: Vec<Mat4>,
    /// rigid frame at the cut (origin at the wound)
    pub frame0: Mat4,
    pub pos: Vec3,
    pub rot: Quat,
    pub vel: Vec3,
    pub spin: Vec3,
    pub life: f32,
    pub tint: [f32; 3],
    /// direction (in the cut frame) the wound faces
    pub wound: Vec3,
    pub radius: f32,
    pub prof: GoreProfile,
    pub landed: bool,
    pub asleep: bool,
}

/// A stump that keeps pumping.
#[derive(Clone, Copy, Debug)]
pub struct Bleed {
    pub e: crate::ecs::Entity,
    pub joint: usize,
    pub attach: usize,
    pub t: f32,
    pub dur: f32,
    pub fx: Fx,
    pub special: Special,
}

/// Push a point (with radius) out of the ground and solid world boxes.
pub fn point_collide(area: &Area, p: Vec3, r: f32) -> (Vec3, bool) {
    let mut q = p;
    let mut hit = false;
    let g = area.ground(q.x, q.z);
    if q.y < g + r {
        q.y = g + r;
        hit = true;
    }
    area.each_box(Vec2::new(q.x - r, q.z - r), Vec2::new(q.x + r, q.z + r), &mut |b| {
        if b.flags & SOLID == 0 {
            return;
        }
        let lo = b.min - Vec3::splat(r);
        let hi = b.max + Vec3::splat(r);
        if q.x > lo.x && q.x < hi.x && q.y > lo.y && q.y < hi.y && q.z > lo.z && q.z < hi.z {
            // leave by the shallowest face
            let d = [q.x - lo.x, hi.x - q.x, q.y - lo.y, hi.y - q.y, q.z - lo.z, hi.z - q.z];
            let (i, _) = d.iter().enumerate().fold((0, f32::MAX), |a, (i, v)| if *v < a.1 { (i, *v) } else { a });
            match i {
                0 => q.x = lo.x,
                1 => q.x = hi.x,
                2 => q.y = lo.y,
                3 => q.y = hi.y,
                4 => q.z = lo.z,
                _ => q.z = hi.z,
            }
            hit = true;
        }
    });
    (q, hit)
}

const MAX_GIBS: usize = 32;

impl Game {
    /// Called by `deal` once damage is applied. Decides whether the blow takes something off.
    #[allow(clippy::too_many_arguments)]
    pub fn gore_on_hit(&mut self, t: crate::ecs::Entity, src: Option<crate::ecs::Entity>, amount: f32, kind: DmgKind, dir: Vec3, lethal: bool) {
        let ctx = self.cut_ctx.take();
        if t == self.player || matches!(kind, DmgKind::Rot | DmgKind::True) {
            return;
        }
        let Some(a) = self.w.actor.get(t) else { return };
        let max_hp = a.max_hp;
        let Some(model) = self.w.rig.get(t).and_then(|r| r.skin.as_ref()).map(|s| self.lib.models[s.model].name.clone()) else { return };
        let prof = profile(&model);
        if prof.parts.is_empty() {
            return;
        }
        let boss = self.w.foe.get(t).map(|f| f.boss.is_some()).unwrap_or(false);
        let center = self.w.body.get(t).map(|b| b.center()).unwrap_or_default();
        let ctx = ctx.unwrap_or_else(|| {
            // derive the blow from the attacker's move
            let (edge, force) = match src.and_then(|s| self.w.actor.get(s)).and_then(|s| s.act.mv) {
                Some(mv) => match mv.swing {
                    crate::data::Swing::Claw | crate::data::Swing::Lash | crate::data::Swing::Bite => (0.7, 0.6),
                    crate::data::Swing::Slam | crate::data::Swing::Overhead | crate::data::Swing::Charge | crate::data::Swing::Leap => (0.15, 1.0),
                    crate::data::Swing::Shoot => (0.3, 0.5),
                    _ => (0.4, 0.6),
                },
                None => (0.3, if matches!(kind, DmgKind::Spell | DmgKind::Fire) { 0.9 } else { 0.5 }),
            };
            let blast = matches!(kind, DmgKind::Spell) && amount > max_hp * 0.3;
            CutCtx { edge, force, at: center + dir * -0.3 + Vec3::Y * 0.2, blast }
        });
        let heavy = matches!(kind, DmgKind::Heavy | DmgKind::Rupture) || ctx.force > 0.9;
        let mut r = fastrand::Rng::with_seed(self.rng.u64(..));
        let mut cuts: Vec<usize> = Vec::new();
        if lethal {
            if ctx.blast {
                // explosions take several pieces
                let n = r.usize(1..=3);
                for _ in 0..n {
                    let i = r.usize(..prof.parts.len());
                    if !cuts.contains(&i) {
                        cuts.push(i);
                    }
                }
            } else {
                let p = if self.gore_force { 1.0 } else { (ctx.edge * 0.7 + ctx.force * 0.3 + if heavy { 0.25 } else { 0.0 }) * prof.give };
                if r.f32() < p {
                    if let Some(i) = self.struck_part(t, &prof, ctx.at, true) {
                        cuts.push(i);
                    }
                }
            }
        } else if !boss && heavy && ctx.edge > 0.5 && amount > max_hp * 0.2 && r.f32() < 0.4 * ctx.edge * prof.give {
            // a limb comes off and the thing keeps coming
            if let Some(i) = self.struck_part(t, &prof, ctx.at, false) {
                cuts.push(i);
            }
        }
        for i in cuts {
            let blunt_head = prof.head_pops && ctx.edge < 0.4 && prof.parts[i][0] == "Head";
            self.sever(t, &prof, i, dir, ctx.force.max(0.4) + if ctx.blast { 1.0 } else { 0.0 }, !blunt_head && prof.special != Special::Crystal, lethal);
        }
    }

    /// The severable group nearest to where the blow landed.
    fn struck_part(&self, t: crate::ecs::Entity, prof: &GoreProfile, at: Vec3, allow_core: bool) -> Option<usize> {
        let sk = self.w.rig.get(t)?.skin.as_ref()?;
        let skel = &self.lib.models[sk.model].skeleton;
        let mut best: Option<(usize, f32)> = None;
        for (i, g) in prof.parts.iter().enumerate() {
            let Some(root) = skel.find(g[0]) else { continue };
            if sk.cuts.iter().any(|c| c.joints.contains(&root)) {
                continue;
            }
            if !allow_core && matches!(g[0], "Head" | "Torso" | "Abdomen") {
                continue;
            }
            let p = sk.last_globals.get(root).map(|m| m.w_axis.truncate()).unwrap_or(at);
            // score the middle of the part, not just its root joint
            let tip = sk.last_globals.get(*children_far(skel, root).last().unwrap_or(&root)).map(|m| m.w_axis.truncate()).unwrap_or(p);
            let d = dist_seg(at, p, tip);
            if best.map(|b| d < b.1).unwrap_or(true) {
                best = Some((i, d));
            }
        }
        best.map(|b| b.0)
    }

    /// Take part `i` off actor `t`.
    #[allow(clippy::too_many_arguments)]
    pub fn sever(&mut self, t: crate::ecs::Entity, prof: &GoreProfile, i: usize, dir: Vec3, force: f32, gib: bool, lethal: bool) {
        let level = self.w.actor.get(t).map(|a| a.level).unwrap_or(1);
        let body_vel = self.w.body.get(t).map(|b| b.vel + b.push).unwrap_or_default();
        let Some(rig) = self.w.rig.get_mut(t) else { return };
        let Some(sk) = rig.skin.as_mut() else { return };
        let model = &self.lib.models[sk.model];
        let skel = &model.skeleton;
        let Some(root) = skel.find(prof.parts[i][0]) else { return };
        if sk.last_globals.len() != skel.joints.len() || sk.cuts.iter().any(|c| c.joints.contains(&root)) {
            return;
        }
        let mut joints = Vec::new();
        for name in prof.parts[i] {
            if let Some(j) = skel.find(name) {
                for k in subtree(skel, j) {
                    if !joints.contains(&k) {
                        joints.push(k);
                    }
                }
            }
        }
        let attach = skel.joints[root].parent.unwrap_or(root);
        let at = sk.last_globals[root].w_axis.truncate();
        let from = sk.last_globals[attach].w_axis.truncate();
        let wound_dir = (at - from).normalize_or_zero();
        sk.cuts.push(Cut { root, joints: joints.clone(), attach, age: 0.0 });
        let frozen = sk.last_globals.clone();
        let tint = sk.tint;
        let (skin_id, model_i) = (sk.skin_id, sk.model);
        let core = matches!(prof.parts[i][0], "Torso" | "Abdomen");
        let leg = prof.parts[i][0].contains("Leg");
        let arm = prof.parts[i][0].contains("Arm");
        let is_head = prof.parts[i][0] == "Head";
        if !lethal && leg {
            sk.crawl = true;
        }
        let ext = joints.iter().map(|&j| (frozen[j].w_axis.truncate() - at).length()).fold(0.0, f32::max);

        // the wound
        let r = &mut self.rng;
        let up = Vec3::Y;
        let spray = (wound_dir + dir * 0.6 + up * 0.4).normalize_or_zero();
        self.parts.burst(prof.blood, at, spray, 40, 1.8);
        self.parts.burst(Fx::Gore, at, spray, 14, 1.4);
        self.parts.burst(Fx::BloodMist, at, dir, 6, 1.0);
        self.parts.burst(prof.chunk, at, spray, 8, 1.3);
        self.audio.play_at(crate::audio::Sfx::Rupture, at, 0.85, rf(r, 0.8, 1.05));
        self.audio.play_at(crate::audio::Sfx::HitHeavy, at, 0.6, rf(r, 0.6, 0.8));
        self.bleeds.push(Bleed { e: t, joint: root, attach, t: 0.0, dur: prof.bleed * if core { 1.6 } else { 1.0 }, fx: prof.blood, special: prof.special });
        let pool = prof.pool;
        let g = self.area.ground(at.x, at.z);
        self.blood_decal_tinted(Vec3::new(at.x, g, at.z) + dir * 0.4, if core { 1.3 } else { 0.7 }, pool);
        if let Some(a) = self.w.actor.get_mut(t) {
            a.blood = (a.blood + 0.4).min(1.0);
            if !lethal && arm {
                a.dmg *= 0.7;
            }
            if !lethal && leg {
                a.speed *= 0.45;
            }
        }
        // what the creature does when opened
        match prof.special {
            Special::Acid => {
                let ground = Vec3::new(at.x, g, at.z);
                self.spawn_zone(ground, ZoneKind::Acid, 1.4, 6.0 + level as f32 * 0.5, 6.0, Team::Infected, None);
                self.parts.burst(Fx::Acid, at, spray, 30, 1.5);
            }
            Special::Embers => {
                self.parts.burst(Fx::Ember, at, spray, 40, 1.6);
                self.parts.burst(Fx::Fire, at, Vec3::Y, 10, 0.8);
                self.lights_temp.push((at, Vec3::new(2.4, 0.9, 0.3), 4.0, 0.5));
            }
            Special::Crystal => {
                self.parts.burst(Fx::Rift, at, spray, 50, 2.2);
                self.parts.burst(Fx::Bone, at, spray, 20, 1.6);
                self.audio.play_at(crate::audio::Sfx::Block, at, 0.7, 1.6);
            }
            Special::Larvae => {
                let n = self.rng.usize(1..=2);
                for _ in 0..n {
                    let p = Vec3::new(at.x, g, at.z) + rand_dir(&mut self.rng) * 0.5;
                    self.spawn_archetype("crawler", p, 1, level.max(1), None);
                }
                self.audio.play_at(crate::audio::Sfx::Larva, at, 0.7, 1.0);
            }
            Special::Spores => self.parts.burst(Fx::Spore, at, Vec3::Y, 40, 1.0),
            Special::Void => self.parts.burst(Fx::Smoke, at, Vec3::Y * 0.5, 10, 0.8),
            Special::Pus => self.parts.burst(Fx::Acid, at, spray, 24, 1.2),
            Special::None => {}
        }
        if !gib {
            // crushed or shattered: no piece survives
            self.parts.burst(Fx::Gore, at, Vec3::Y, 30, 2.0);
            self.parts.burst(prof.chunk, at, Vec3::Y, 24, 1.8);
            return;
        }
        let r = &mut self.rng;
        let frame0 = Mat4::from_translation(at);
        let speed = rf(r, 2.5, 4.5) * force * if is_head { 1.2 } else { 1.0 } / (1.0 + ext * 0.6);
        let vel = body_vel * 0.5 + (dir * 0.8 + wound_dir * 0.6).normalize_or_zero() * speed + up * rf(r, 1.5, 3.2) * force;
        let spin = rand_dir(r) * rf(r, 4.0, 11.0) * force;
        self.gibs.push(Gib { skin_id, model: model_i, joints, frozen, frame0, pos: at, rot: Quat::IDENTITY, vel, spin, life: 0.0, tint, wound: -wound_dir, radius: (ext * 0.25).clamp(0.05, 0.35), prof: *prof, landed: false, asleep: false });
        if self.gibs.len() > MAX_GIBS {
            self.gibs.remove(0);
        }
    }

    /// Gib physics, stump bleeding and blood landing on the ground.
    pub fn gore_tick(&mut self, dt: f32) {
        // gibs
        let mut splats: Vec<(Vec3, f32, [f32; 3])> = Vec::new();
        for gb in &mut self.gibs {
            gb.life += dt;
            if gb.asleep {
                continue;
            }
            gb.vel.y -= 9.81 * dt;
            gb.vel *= 1.0 - 0.15 * dt;
            gb.pos += gb.vel * dt;
            gb.rot = (Quat::from_scaled_axis(gb.spin * dt) * gb.rot).normalize();
            let (p, hit) = point_collide(&self.area, gb.pos, gb.radius);
            if hit {
                let n = (p - gb.pos).normalize_or(Vec3::Y);
                gb.pos = p;
                let vn = gb.vel.dot(n);
                if vn < 0.0 {
                    gb.vel -= n * vn * 1.3;
                }
                gb.vel *= 0.6;
                gb.spin *= 0.55;
                if !gb.landed && vn < -1.5 {
                    splats.push((p, 0.35 + gb.radius, gb.prof.pool));
                }
                gb.landed = true;
                if gb.vel.length() < 0.25 && gb.spin.length() < 0.6 {
                    gb.asleep = true;
                }
            }
        }
        for (p, s, c) in splats {
            let g = self.area.ground(p.x, p.z);
            self.blood_decal_tinted(Vec3::new(p.x, g, p.z), s, c);
            self.parts.burst(Fx::Gore, p, Vec3::Y, 4, 0.8);
        }
        self.gibs.retain(|g| g.life < 30.0);
        // gib wounds drip and smoulder
        let mut drips: Vec<(Fx, Vec3, Vec3, Special)> = Vec::new();
        for gb in &self.gibs {
            if gb.life < 2.5 && self.rng.f32() < 0.6 {
                let m = Mat4::from_rotation_translation(gb.rot, gb.pos);
                drips.push((gb.prof.blood, m.transform_point3(Vec3::ZERO), gb.rot * gb.wound, gb.prof.special));
            }
        }
        for (fx, p, d, sp) in drips {
            self.parts.burst(fx, p, d * 0.3, 1, 0.4);
            if sp == Special::Embers {
                self.parts.burst(Fx::Ember, p, Vec3::Y, 1, 0.6);
            }
        }
        // stumps: arterial pulses that weaken
        let mut alive_bleeds = Vec::new();
        let bleeds = std::mem::take(&mut self.bleeds);
        for mut b in bleeds {
            b.t += dt;
            if b.t > b.dur {
                continue;
            }
            let Some(sk) = self.w.rig.get(b.e).and_then(|r| r.skin.as_ref()) else { continue };
            let (Some(p), Some(q)) = (sk.last_globals.get(b.joint), sk.last_globals.get(b.attach)) else { continue };
            let at = p.w_axis.truncate();
            let d = (at - q.w_axis.truncate()).normalize_or_zero();
            let k = 1.0 - b.t / b.dur;
            let pulse = ((b.t * 7.5).sin() * 0.5 + 0.5).powf(3.0);
            let n = (pulse * 4.0 * k).round() as u32;
            if n > 0 {
                self.parts.burst(b.fx, at, (d + Vec3::Y * 0.3).normalize_or_zero() * (0.8 + pulse * 1.4), n, 0.6 + k);
            }
            match b.special {
                Special::Embers if self.rng.f32() < 0.5 => self.parts.burst(Fx::Ember, at, d, 1, 0.8),
                Special::Spores if self.rng.f32() < 0.3 => self.parts.burst(Fx::Spore, at, Vec3::Y * 0.2, 1, 0.3),
                Special::Void if self.rng.f32() < 0.2 => self.parts.burst(Fx::Smoke, at, Vec3::Y * 0.2, 1, 0.4),
                _ => {}
            }
            alive_bleeds.push(b);
        }
        self.bleeds = alive_bleeds;
        // blood that reached the ground leaves small spatters
        let landed = std::mem::take(&mut self.parts.landed);
        for (p, size, c) in landed.into_iter().take(6) {
            let g = self.area.ground(p.x, p.z);
            if (p.y - g).abs() < 0.3 {
                self.blood_decal_tinted(Vec3::new(p.x, g, p.z), size, c);
            }
        }
    }

    /// Draw flying parts: each is its creature's own skinned mesh with only the part left.
    pub fn draw_gibs(&mut self) {
        for gb in &self.gibs {
            let model = &self.lib.models[gb.model];
            let skel = &model.skeleton;
            let now = Mat4::from_rotation_translation(gb.rot, gb.pos) * gb.frame0.inverse();
            let wound = now.transform_point3(gb.frame0.w_axis.truncate());
            let collapsed = Mat4::from_translation(wound) * Mat4::from_scale(Vec3::splat(1e-4));
            let base = self.inst.joints.len();
            if base + skel.joints.len() > crate::render::MAX_JOINTS {
                break;
            }
            for (j, jt) in skel.joints.iter().enumerate() {
                let g = if gb.joints.contains(&j) { now * gb.frozen[j] } else { collapsed };
                let m = (g * jt.inverse_bind).transpose().to_cols_array();
                self.inst.joints.push([m[0], m[1], m[2], m[3], m[4], m[5], m[6], m[7], m[8], m[9], m[10], m[11]]);
            }
            let mut inst = Instance::new(Mat4::IDENTITY, [gb.tint[0], gb.tint[1], gb.tint[2], 0.0], base as f32);
            inst.params[1] = 12.0;
            self.inst.skinned.push((gb.skin_id, inst));
            // the torn end
            let d = (gb.rot * gb.wound).normalize_or(Vec3::Y);
            let r = gb.radius * 0.45;
            self.inst.push(Prim::Sphere as u32, Instance::new(segment(wound - d * r * 0.5, wound + d * r * 0.4, r, r, Vec3::Y), [0.5, 0.1, 0.08, 0.0], Tex::Flesh as u32 as f32));
            self.inst.push(Prim::Spike as u32, Instance::new(segment(wound, wound + d * r * 1.2, r * 0.3, r * 0.3, Vec3::Y), [0.95, 0.88, 0.78, 0.0], Tex::Bone as u32 as f32));
        }
    }
}

/// All joints in the subtree rooted at `j` (including `j`).
pub fn subtree(skel: &crate::render::model::Skeleton, j: usize) -> Vec<usize> {
    let mut out = vec![j];
    let mut i = 0;
    while i < out.len() {
        let p = out[i];
        for (k, jt) in skel.joints.iter().enumerate() {
            if jt.parent == Some(p) && !out.contains(&k) {
                out.push(k);
            }
        }
        i += 1;
    }
    out
}

/// The joint chain from `j` down its longest branch.
fn children_far(skel: &crate::render::model::Skeleton, j: usize) -> Vec<usize> {
    let mut out = vec![j];
    let mut cur = j;
    loop {
        let kids: Vec<usize> = (0..skel.joints.len()).filter(|&k| skel.joints[k].parent == Some(cur)).collect();
        let Some(&next) = kids.iter().max_by_key(|&&k| subtree(skel, k).len()) else { break };
        out.push(next);
        cur = next;
        if out.len() > 4 {
            break;
        }
    }
    out
}

fn dist_seg(p: Vec3, a: Vec3, b: Vec3) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
    (a + ab * t - p).length()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::model::load_all;

    /// Every severable part named in a profile exists on that model's skeleton.
    #[test]
    fn gore_profiles_match_skeletons() {
        let all = load_all();
        for m in all.iter().filter(|m| m.skinned()) {
            let p = profile(&m.name);
            for g in p.parts {
                assert!(m.skeleton.find(g[0]).is_some(), "{}: no bone {}", m.name, g[0]);
            }
            if m.name.starts_with("Strain_") || m.name.starts_with("Characters_") || m.name.starts_with("Human_") {
                assert!(!p.parts.is_empty(), "{} cannot be dismembered", m.name);
            }
        }
    }
}
