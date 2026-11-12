//! Procedural animation. Characters are modular body plans (torso, head, any number of arms and
//! legs, growths) posed entirely in code: two-bone IK limbs, planted feet that step on their own,
//! verlet tendrils, spring secondary motion and curve-driven attack swings.
use crate::data::{ArmKind, Attach, Growth, GrowthKind, HeadKind, LegKind, Swing, WeaponClass};
use crate::render::mesh::{segment, trs, Instance, Prim};
use crate::render::textures::Tex;
use crate::render::InstList;
use crate::util::{ease_in, ease_out, smooth, Spring};
use glam::{Mat4, Quat, Vec3};
use std::f32::consts::{PI, TAU};

#[derive(Clone, Debug)]
pub struct ArmDef {
    /// +1 right, -1 left
    pub side: f32,
    pub kind: ArmKind,
    pub tex: Tex,
    pub len: f32,
    pub thick: f32,
    /// vertical placement on the torso (1 = shoulder line)
    pub at: f32,
}
#[derive(Clone, Copy, Debug)]
pub struct WeaponVis {
    pub class: WeaponClass,
    pub tint: [f32; 3],
    pub glow: f32,
}
#[derive(Clone, Debug)]
pub struct BodyPlan {
    pub scale: f32,
    pub torso: Vec3,
    pub torso_tex: Tex,
    pub hunch: f32,
    pub head: HeadKind,
    pub head_tex: Tex,
    pub head_size: f32,
    pub arms: Vec<ArmDef>,
    pub leg_kind: LegKind,
    pub leg_tex: Tex,
    pub leg_len: f32,
    pub leg_thick: f32,
    pub leg_count: u8,
    pub growths: Vec<Growth>,
    pub skin: [f32; 3],
    pub cloth: [f32; 3],
    pub legs_tint: [f32; 3],
    pub glow: [f32; 3],
    /// right, left
    pub hands: [Option<WeaponVis>; 2],
    pub seed: u32,
}
impl BodyPlan {
    pub fn human(seed: u32) -> BodyPlan {
        BodyPlan {
            scale: 1.0,
            torso: Vec3::new(0.42, 0.58, 0.24),
            torso_tex: Tex::Cloth,
            hunch: 0.05,
            head: HeadKind::Human,
            head_tex: Tex::Skin,
            head_size: 1.0,
            arms: vec![
                ArmDef { side: 1.0, kind: ArmKind::Human, tex: Tex::Skin, len: 0.62, thick: 0.1, at: 1.0 },
                ArmDef { side: -1.0, kind: ArmKind::Human, tex: Tex::Skin, len: 0.62, thick: 0.1, at: 1.0 },
            ],
            leg_kind: LegKind::Human,
            leg_tex: Tex::Cloth,
            leg_len: 0.88,
            leg_thick: 0.13,
            leg_count: 2,
            growths: Vec::new(),
            skin: [1.0, 1.0, 1.0],
            cloth: [0.35, 0.38, 0.42],
            legs_tint: [0.25, 0.27, 0.32],
            glow: [0.4, 1.0, 0.7],
            hands: [None, None],
            seed,
        }
    }
    pub fn height(&self) -> f32 {
        (self.leg_len * 0.95 + self.torso.y + 0.3 * self.head_size) * self.scale
    }
}

#[derive(Clone, Copy, Default, Debug)]
pub struct Pose {
    /// (swing, phase 0..3 = windup/active/recovery, uses left hand)
    pub swing: Option<(Swing, f32, bool)>,
    pub block: f32,
    pub roll: f32,
    pub stagger: f32,
    pub dead: f32,
    pub cast: f32,
    pub heal: f32,
    pub hide: f32,
    pub charge: f32,
    pub flash: f32,
    pub dissolve: f32,
    pub glow: f32,
    pub two_handed: bool,
    pub aim: bool,
    pub extra_strike: f32,
    pub cower: f32,
}

#[derive(Clone, Debug)]
pub struct Chain {
    pub pts: Vec<Vec3>,
    prev: Vec<Vec3>,
    pub seg: f32,
}
impl Chain {
    pub fn new(n: usize, seg: f32) -> Chain {
        Chain { pts: vec![Vec3::ZERO; n], prev: vec![Vec3::ZERO; n], seg }
    }
    /// Verlet step with the root pinned; `tip` optionally drags the far end toward a point.
    pub fn step(&mut self, root: Vec3, dir: Vec3, dt: f32, gravity: f32, tip: Option<(Vec3, f32)>) {
        if (self.pts[0] - root).length_squared() > 100.0 {
            for (i, p) in self.pts.iter_mut().enumerate() {
                *p = root + dir * self.seg * i as f32;
            }
            self.prev.copy_from_slice(&self.pts);
        }
        let n = self.pts.len();
        for i in 1..n {
            let v = (self.pts[i] - self.prev[i]) * 0.9;
            self.prev[i] = self.pts[i];
            self.pts[i] += v + (Vec3::new(0.0, -gravity, 0.0) + dir * 6.0 / (i as f32)) * dt * dt;
            if self.pts[i].y < 0.03 {
                self.pts[i].y = 0.03;
            }
        }
        if let Some((t, k)) = tip {
            let l = self.pts[n - 1];
            self.pts[n - 1] = l + (t - l) * k;
        }
        self.pts[0] = root;
        for _ in 0..3 {
            for i in 1..n {
                let d = self.pts[i] - self.pts[i - 1];
                let l = d.length().max(1e-5);
                self.pts[i] = self.pts[i - 1] + d / l * self.seg;
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Foot {
    pos: Vec3,
    from: Vec3,
    to: Vec3,
    t: f32,
}

pub struct Rig {
    pub plan: BodyPlan,
    feet: Vec<Foot>,
    chains: Vec<Chain>,
    sway: Spring,
    time: f32,
    bob: f32,
    init: bool,
    /// world-space anchor points refreshed on every update
    pub hand_r: Vec3,
    pub hand_l: Vec3,
    pub head: Vec3,
    pub chest: Vec3,
    pub weapon_tip: Vec3,
}

fn v(r: f32, u: f32, f: f32) -> Vec3 {
    Vec3::new(-r, u, f)
}
fn bez(a: Vec3, b: Vec3, c: Vec3, t: f32) -> Vec3 {
    a.lerp(b, t).lerp(b.lerp(c, t), t)
}

/// Hand path of a swing in chest space (right-handed; mirrored for the left hand).
fn swing_path(s: Swing) -> (Vec3, Vec3, Vec3) {
    match s {
        Swing::SlashR | Swing::Spin => (v(0.75, 0.3, -0.15), v(0.35, 0.2, 0.85), v(-0.55, 0.0, 0.35)),
        Swing::SlashL => (v(-0.5, 0.3, 0.1), v(0.25, 0.2, 0.85), v(0.8, 0.0, 0.15)),
        Swing::Claw => (v(0.6, 0.55, -0.1), v(0.3, 0.4, 0.7), v(-0.3, -0.1, 0.5)),
        Swing::Overhead | Swing::Slam | Swing::Leap => (v(0.2, 0.8, -0.3), v(0.2, 0.75, 0.6), v(0.12, -0.5, 0.65)),
        Swing::Thrust | Swing::Charge => (v(0.32, 0.0, -0.3), v(0.25, 0.05, 0.4), v(0.12, 0.05, 1.0)),
        Swing::Upper => (v(0.35, -0.55, 0.1), v(0.3, -0.15, 0.75), v(0.15, 0.75, 0.35)),
        Swing::Bash => (v(0.1, 0.0, 0.15), v(0.05, 0.05, 0.5), v(0.0, 0.05, 0.9)),
        Swing::Lash => (v(0.5, 0.6, -0.35), v(0.3, 0.5, 0.5), v(0.1, -0.1, 0.9)),
        Swing::Bite => (v(0.5, 0.2, 0.1), v(0.4, 0.15, 0.5), v(0.12, 0.1, 0.75)),
        Swing::Spit | Swing::Shoot => (v(0.22, 0.05, 0.6), v(0.2, 0.08, 0.7), v(0.2, 0.05, 0.75)),
    }
}

/// Two-bone IK: returns the elbow/knee position.
fn ik(root: Vec3, target: &mut Vec3, l1: f32, l2: f32, pole: Vec3) -> Vec3 {
    let mut d = *target - root;
    let mut dist = d.length().max(1e-4);
    let max = (l1 + l2) * 0.999;
    if dist > max {
        d = d / dist * max;
        dist = max;
        *target = root + d;
    }
    let a = ((l1 * l1 - l2 * l2 + dist * dist) / (2.0 * dist)).clamp(-l1, l1);
    let h = (l1 * l1 - a * a).max(0.0).sqrt();
    let dir = d / dist;
    let mut side = pole - dir * pole.dot(dir);
    if side.length_squared() < 1e-6 {
        side = Vec3::Y - dir * dir.y;
    }
    root + dir * a + side.normalize_or_zero() * h
}

struct Emit<'a> {
    out: &'a mut InstList,
    flash: f32,
    dissolve: f32,
}
impl Emit<'_> {
    fn put(&mut self, prim: Prim, m: Mat4, tex: Tex, tint: [f32; 3], emis: f32) {
        let mut i = Instance::new(m, [tint[0], tint[1], tint[2], emis], tex as u32 as f32);
        i.params[1] = self.flash;
        i.params[3] = self.dissolve;
        self.out.push(prim as u32, i);
    }
    fn limb(&mut self, a: Vec3, b: Vec3, t0: f32, tex: Tex, tint: [f32; 3], emis: f32) {
        self.put(Prim::Limb, segment(a, b, t0, t0, Vec3::Z), tex, tint, emis);
    }
    fn chain(&mut self, c: &Chain, thick: f32, tex: Tex, tint: [f32; 3], emis: f32) {
        let n = c.pts.len();
        for i in 0..n - 1 {
            let t = thick * (1.0 - 0.7 * i as f32 / n as f32);
            self.limb(c.pts[i], c.pts[i + 1], t, tex, tint, emis);
        }
    }
}

impl Rig {
    pub fn new(plan: BodyPlan) -> Rig {
        let n = plan.leg_count as usize;
        Rig {
            feet: vec![Foot { pos: Vec3::ZERO, from: Vec3::ZERO, to: Vec3::ZERO, t: 1.0 }; n],
            chains: Vec::new(),
            sway: Spring::default(),
            time: (plan.seed % 1000) as f32 * 0.37,
            bob: 0.0,
            init: false,
            hand_r: Vec3::ZERO,
            hand_l: Vec3::ZERO,
            head: Vec3::ZERO,
            chest: Vec3::ZERO,
            weapon_tip: Vec3::ZERO,
            plan,
        }
    }

    /// True while any foot is mid-step.
    pub fn stepping(&self) -> bool {
        self.feet.iter().any(|f| f.t < 1.0)
    }

    fn chain(&mut self, idx: &mut usize, n: usize, seg: f32) -> usize {
        let i = *idx;
        if self.chains.len() <= i {
            self.chains.push(Chain::new(n, seg));
        }
        *idx += 1;
        i
    }

    /// Advance the rig and append its instances. `pos` is the feet position, `vel` the planar velocity.
    pub fn update_emit(&mut self, dt: f32, pos: Vec3, yaw: f32, vel: Vec3, pose: &Pose, out: &mut InstList) {
        let s = self.plan.scale;
        self.time += dt;
        let time = self.time;
        let speed = Vec3::new(vel.x, 0.0, vel.z).length();
        let rot = Quat::from_rotation_y(yaw);
        let p = &self.plan;
        let spider = p.leg_kind == LegKind::Spider;
        let quad = spider && p.torso.z > p.torso.y;
        let hide = pose.hide.max(pose.cower * 0.5);

        // ---- body root (handles roll, collapse, hide)
        let mut hip_h = p.leg_len * if spider { 0.55 } else { 0.93 } * s;
        hip_h *= 1.0 - 0.75 * hide - 0.35 * pose.roll.min(1.0 - pose.roll).min(0.25) * 4.0;
        self.bob += speed * dt * 2.2 / p.leg_len.max(0.3);
        let bob = (self.bob * 2.0).sin() * 0.03 * s * (speed / 4.0).min(1.0);
        let wind = pose.swing.map(|(_, ph, _)| ph).unwrap_or(-1.0);
        let crouch = match pose.swing {
            Some((Swing::Leap | Swing::Slam | Swing::Charge, ph, _)) if ph < 1.0 => ease_out(ph) * 0.3,
            _ => 0.0,
        } + pose.charge * 0.12;
        hip_h *= 1.0 - crouch;
        let dead_fall = ease_in(pose.dead) * PI * 0.5;
        let roll_ang = pose.roll * TAU;
        let pitch = roll_ang + dead_fall;
        let pivot = Vec3::new(0.0, if pose.dead > 0.0 { 0.15 * s } else { hip_h * 0.8 }, 0.0);
        let spin = match pose.swing {
            Some((Swing::Spin, ph, _)) if (1.0..2.0).contains(&ph) => -(ph - 1.0) * TAU,
            _ => 0.0,
        };
        let root = Mat4::from_translation(pos)
            * Mat4::from_quat(rot * Quat::from_rotation_y(spin))
            * Mat4::from_translation(pivot)
            * Mat4::from_rotation_x(pitch)
            * Mat4::from_translation(-pivot);
        let flat_inv = (Mat4::from_translation(pos) * Mat4::from_quat(rot)).inverse();

        // ---- torso frame
        self.sway.step(vel * 0.03, 60.0, 9.0, dt);
        let lag = flat_inv.transform_vector3(self.sway.x - vel * 0.03);
        let mut lean = p.hunch + (speed * 0.035).min(0.25) + pose.stagger * -0.45 + crouch * 0.8 + pose.cower * 0.7;
        let mut twist = 0.0;
        if let Some((sw, ph, left)) = pose.swing {
            let m = if left { -1.0 } else { 1.0 };
            let (w, a) = (smooth(ph.min(1.0)), smooth((ph - 1.0).clamp(0.0, 1.0)));
            let back = smooth((ph - 2.0).clamp(0.0, 1.0));
            match sw {
                Swing::SlashR | Swing::Claw | Swing::Lash => twist = m * (-0.6 * w + 1.3 * a) * (1.0 - back),
                Swing::SlashL => twist = m * (0.6 * w - 1.3 * a) * (1.0 - back),
                Swing::Overhead | Swing::Slam | Swing::Leap => lean += (-0.35 * w + 0.9 * a) * (1.0 - back),
                Swing::Thrust | Swing::Bash | Swing::Bite | Swing::Charge => lean += (-0.15 * w + 0.6 * a) * (1.0 - back),
                Swing::Upper => lean += (0.5 * w - 0.7 * a) * (1.0 - back),
                Swing::Spit => lean += (-0.4 * w + 0.6 * a) * (1.0 - back),
                _ => {}
            }
        }
        lean += pose.block * 0.12 + pose.heal * -0.2;
        let hip = Vec3::new(lag.x * 0.3, hip_h + bob, lag.z * 0.3);
        let tq = if quad { Quat::from_rotation_y(twist) } else { Quat::from_rotation_y(twist) * Quat::from_rotation_x(lean) };
        let th = p.torso.y * s;
        let up = tq * Vec3::Y;
        let chest_c = hip + up * th * 0.5;
        let neck = hip + up * th;
        let breathe = 1.0 + (time * 1.6).sin() * 0.015;

        let mut e = Emit { out, flash: pose.flash, dissolve: pose.dissolve };
        let skin = p.skin;
        let body_tint = if matches!(p.torso_tex, Tex::Cloth | Tex::Camo) { p.cloth } else { skin };
        let glow_e = pose.glow;

        // torso
        let tm = root * trs(chest_c, tq, Vec3::new(p.torso.x * s * breathe, th, p.torso.z * s * breathe));
        e.put(Prim::Cube, tm, p.torso_tex, body_tint, glow_e * 0.5);
        // pelvis
        if !quad {
            e.put(Prim::Cube, root * trs(hip - Vec3::Y * 0.06 * s, Quat::from_rotation_y(twist * 0.3), Vec3::new(p.torso.x * 0.9, 0.16, p.torso.z * 0.95) * s), p.leg_tex, p.legs_tint, 0.0);
        }

        // ---- head
        let hs = 0.24 * p.head_size * s;
        let look = Quat::from_rotation_x(lean * -0.5 + pose.stagger * -0.3) * Quat::from_rotation_y((time * 0.7 + p.seed as f32).sin() * 0.12);
        let head_c = if quad { hip + Vec3::new(0.0, p.torso.y * s * 0.3, p.torso.z * s * 0.55) } else { neck + up * (hs * 0.55 + 0.03 * s) };
        let hq = tq * look;
        let hm = root * trs(head_c, hq, Vec3::splat(hs));
        let head_tint = if matches!(p.head_tex, Tex::Cloth | Tex::Camo) { p.cloth } else { skin };
        match p.head {
            HeadKind::Human | HeadKind::Eyeless => {
                e.put(Prim::Cube, hm * Mat4::from_scale(Vec3::new(0.85, 1.0, 0.9)), p.head_tex, head_tint, 0.0);
                if p.head == HeadKind::Human {
                    e.put(Prim::Cube, hm * trs(Vec3::new(0.0, 0.38, -0.05), Quat::IDENTITY, Vec3::new(0.9, 0.3, 0.95)), Tex::Cloth, [0.15, 0.12, 0.1], 0.0);
                }
            }
            HeadKind::Hood => {
                e.put(Prim::Cube, hm * Mat4::from_scale(Vec3::new(0.8, 0.95, 0.85)), Tex::Skin, skin, 0.0);
                e.put(Prim::Cube, hm * trs(Vec3::new(0.0, 0.1, -0.12), Quat::IDENTITY, Vec3::new(1.05, 1.15, 1.0)), Tex::Cloth, p.cloth, 0.0);
            }
            HeadKind::Helmet => {
                e.put(Prim::Cube, hm * Mat4::from_scale(Vec3::new(0.85, 1.0, 0.9)), Tex::Rust, [0.25, 0.27, 0.25], 0.0);
                e.put(Prim::Sphere, hm * trs(Vec3::new(0.0, 0.3, 0.0), Quat::IDENTITY, Vec3::new(1.2, 0.9, 1.25)), Tex::Camo, [1.0; 3], 0.0);
                e.put(Prim::Cyl, hm * trs(Vec3::new(0.0, -0.25, 0.45), Quat::from_rotation_x(PI * 0.5), Vec3::new(0.35, 0.3, 0.35)), Tex::Metal, [0.3; 3], 0.0);
            }
            HeadKind::Split => {
                let open = 0.25 + 0.15 * (time * 2.0).sin() + pose.glow * 0.5;
                for sd in [-1.0f32, 1.0] {
                    e.put(Prim::Cube, hm * trs(Vec3::new(sd * 0.28, 0.0, 0.0), Quat::from_rotation_z(-sd * open), Vec3::new(0.42, 1.05, 0.9)), p.head_tex, skin, 0.0);
                }
                e.put(Prim::Sphere, hm * Mat4::from_scale(Vec3::splat(0.5)), Tex::Vein, p.glow, 0.5 + glow_e);
            }
            HeadKind::Maw => {
                let open = 0.3 + 0.2 * (time * 3.0).sin().abs() + pose.glow * 0.6;
                e.put(Prim::Cube, hm * trs(Vec3::new(0.0, 0.2, 0.0), Quat::from_rotation_x(-open), Vec3::new(1.0, 0.5, 1.2)), p.head_tex, skin, 0.0);
                e.put(Prim::Cube, hm * trs(Vec3::new(0.0, -0.3, 0.0), Quat::from_rotation_x(open), Vec3::new(0.95, 0.4, 1.15)), p.head_tex, skin, 0.0);
                for i in 0..4 {
                    let x = -0.36 + i as f32 * 0.24;
                    e.put(Prim::Spike, hm * trs(Vec3::new(x, -0.12, 0.5), Quat::IDENTITY, Vec3::new(0.14, 0.35, 0.14)), Tex::Bone, [1.0; 3], 0.0);
                }
            }
            HeadKind::Stalk => {
                let tip = head_c + hq * Vec3::new(0.0, hs * 1.6, hs * 0.5);
                e.limb(root.transform_point3(neck), root.transform_point3(tip), hs * 0.35, p.head_tex, skin, 0.0);
                e.put(Prim::Sphere, root * trs(tip, hq, Vec3::splat(hs * 0.8)), Tex::GrowthGlow, p.glow, 0.7 + glow_e);
            }
            HeadKind::Bloom => {
                for i in 0..6 {
                    let a = i as f32 / 6.0 * TAU + time * 0.2;
                    let open = 0.7 + 0.25 * (time * 1.3 + i as f32).sin() + pose.glow * 0.4;
                    let q = Quat::from_rotation_y(a) * Quat::from_rotation_x(open);
                    e.put(Prim::Blade, hm * trs(Vec3::ZERO, q, Vec3::new(0.7, 1.5, 0.7)), p.head_tex, skin, 0.0);
                }
                e.put(Prim::Sphere, hm * Mat4::from_scale(Vec3::splat(0.6)), Tex::GrowthGlow, p.glow, 0.9 + glow_e);
            }
            HeadKind::Cluster => {
                for i in 0..5 {
                    let a = i as f32 * 2.4;
                    let o = Vec3::new(a.cos() * 0.4, (i as f32 * 1.7).sin() * 0.35, a.sin() * 0.4);
                    let pulse = 0.55 + 0.12 * (time * 2.0 + i as f32).sin();
                    e.put(Prim::Sphere, hm * trs(o, Quat::IDENTITY, Vec3::splat(pulse)), if i % 2 == 0 { p.head_tex } else { Tex::Flesh }, if i % 2 == 0 { p.glow } else { skin }, if i % 2 == 0 { 0.8 + glow_e } else { 0.0 });
                }
            }
            HeadKind::Crown => {
                e.put(Prim::Cube, hm * Mat4::from_scale(Vec3::new(0.8, 1.0, 0.85)), p.head_tex, skin, 0.0);
                for i in 0..5 {
                    let a = i as f32 / 5.0 * TAU;
                    e.put(Prim::Spike, hm * trs(Vec3::new(a.cos() * 0.35, 0.45, a.sin() * 0.35), Quat::from_rotation_z(-a.cos() * 0.4) * Quat::from_rotation_x(a.sin() * 0.4), Vec3::new(0.2, 0.9, 0.2)), Tex::Bone, [1.0; 3], 0.0);
                }
                e.put(Prim::Cube, hm * trs(Vec3::new(0.0, 0.05, 0.42), Quat::IDENTITY, Vec3::new(0.6, 0.12, 0.1)), Tex::GrowthGlow, p.glow, 1.0 + glow_e);
            }
            HeadKind::None => {
                e.put(Prim::Spike, root * trs(neck, tq, Vec3::new(hs, hs * 0.8, hs)), Tex::Flesh, skin, 0.0);
            }
        }
        self.head = root.transform_point3(head_c);
        self.chest = root.transform_point3(chest_c);

        // ---- arms
        let mut chain_i = 0usize;
        let arms = self.plan.arms.clone();
        let mut hand_pos = [self.chest; 2];
        for (ai, arm) in arms.iter().enumerate() {
            let p = &self.plan;
            let m = arm.side;
            let is_left = m < 0.0;
            let slot = if is_left { 1 } else { 0 };
            let primary = ai < 2;
            let sh = hip + tq * Vec3::new(-m * (p.torso.x * 0.5 + arm.thick * 0.5) * s, th * (0.55 + 0.4 * arm.at), 0.0);
            let len = arm.len * s;
            let reach = len / 0.62;
            let walk = (self.bob + if is_left { 0.0 } else { PI }).sin() * 0.16 * (speed / 4.0).min(1.0);
            let armed = primary && p.hands[slot].is_some();
            // idle / guard
            let mut hand = if armed || arm.kind != ArmKind::Human {
                v(m * 0.3, -0.22, 0.3 + walk * 0.5)
            } else {
                v(m * 0.3, -0.5, 0.05 + walk)
            };
            if !primary {
                hand = v(m * 0.45, 0.1 + 0.1 * (time * 1.3 + ai as f32).sin(), 0.35);
                if pose.extra_strike > 0.0 {
                    let (a, b, c) = swing_path(Swing::Claw);
                    hand = bez(a, b, c, pose.extra_strike);
                    hand.x *= m;
                }
            }
            let mut wdir = (v(m * 0.1, 0.9, 0.45)).normalize();
            let mut swinging = false;
            if let Some((sw, ph, left)) = pose.swing {
                let both = matches!(sw, Swing::Bite | Swing::Leap | Swing::Slam | Swing::Charge) || pose.two_handed;
                if primary && (left == is_left || both) {
                    swinging = true;
                    let (mut a, mut b, mut c) = swing_path(sw);
                    let mm = if left { -1.0 } else { 1.0 };
                    a.x *= mm;
                    b.x *= mm;
                    c.x *= mm;
                    let guard = hand;
                    let h = if ph < 1.0 {
                        guard.lerp(a, ease_out(ph))
                    } else if ph < 2.0 {
                        bez(a, b, c, smooth(ph - 1.0))
                    } else {
                        c.lerp(guard, smooth(ph - 2.0))
                    };
                    let mut off = Vec3::ZERO;
                    if both && left != is_left {
                        off = v(m * 0.22, -0.12, -0.12);
                        if matches!(sw, Swing::Bite) {
                            off = v(m * 0.35, 0.0, 0.0);
                        }
                    }
                    hand = h + off;
                    let t = (h - v(mm * 0.2, -0.1, 0.0)).normalize_or_zero();
                    wdir = if ph < 2.0 { t } else { t.lerp(wdir, smooth(ph - 2.0)).normalize_or_zero() };
                }
            }
            if !swinging && primary {
                if pose.block > 0.0 && (is_left || p.hands[1].is_none()) {
                    hand = hand.lerp(v(m * 0.08, 0.05, 0.5), pose.block);
                    wdir = wdir.lerp(v(-m * 0.5, 0.8, 0.2).normalize(), pose.block).normalize();
                }
                if pose.cast > 0.0 && (p.hands[slot].map(|w| w.class == WeaponClass::Catalyst).unwrap_or(false) || (p.hands[0].is_none() && is_left)) {
                    hand = hand.lerp(v(m * 0.15, 0.25, 0.8), pose.cast);
                }
                if pose.heal > 0.0 && is_left {
                    hand = hand.lerp(v(0.0, 0.1, 0.3), pose.heal);
                }
                if pose.aim || arm.kind == ArmKind::Rifle {
                    hand = v(if is_left { -0.05 } else { 0.18 }, if pose.aim { 0.1 } else { -0.2 }, if is_left { 0.62 } else { 0.35 });
                    wdir = v(0.0, if pose.aim { 0.0 } else { -0.3 }, 1.0).normalize();
                }
                if pose.charge > 0.0 && !is_left {
                    hand = hand.lerp(v(0.45, 0.45, -0.35), pose.charge);
                }
                if pose.stagger > 0.0 {
                    hand = hand.lerp(v(m * 0.7, 0.1, -0.2), pose.stagger);
                }
                if pose.cower > 0.0 {
                    hand = hand.lerp(v(m * 0.15, 0.45, 0.25), pose.cower);
                }
                if pose.two_handed && is_left {
                    hand = v(0.12, -0.3, 0.38);
                }
            }
            let chest_origin = hip + tq * Vec3::new(0.0, th * 0.9, 0.0);
            let mut target = chest_origin + tq * (hand * reach.max(0.9) * s);
            let pole = tq * v(m * 0.6, -0.8, -0.5);
            let tint = if matches!(arm.tex, Tex::Cloth | Tex::Camo) { self.plan.cloth } else { skin };
            let th_a = arm.thick * s;
            let wsh = root.transform_point3(sh);
            match arm.kind {
                ArmKind::Tendril => {
                    let ci = self.chain(&mut chain_i, 7, len / 5.0);
                    let wt = root.transform_point3(target);
                    let tipk = if swinging { 0.6 } else { 0.05 };
                    self.chains[ci].step(wsh, root.transform_vector3(tq * v(m * 0.5, -0.3, 0.6)).normalize(), dt, 6.0, Some((wt + (wt - wsh) * if swinging { 0.9 } else { 0.0 }, tipk)));
                    e.chain(&self.chains[ci], th_a * 1.6, arm.tex, tint, glow_e * 0.6);
                    target = root.inverse().transform_point3(*self.chains[ci].pts.last().unwrap());
                }
                _ => {
                    let (l1, l2) = (len * 0.5, len * 0.5);
                    let elbow = ik(sh, &mut target, l1, l2, pole);
                    let (we, wh) = (root.transform_point3(elbow), root.transform_point3(target));
                    e.limb(wsh, we, th_a * 1.15, arm.tex, tint, 0.0);
                    let fore_tex = if arm.kind == ArmKind::Human || arm.kind == ArmKind::Rifle { Tex::Skin } else { arm.tex };
                    let fore_tint = if fore_tex == Tex::Skin { skin } else { tint };
                    e.limb(we, wh, th_a, fore_tex, fore_tint, 0.0);
                    let fdir = (wh - we).normalize_or_zero();
                    let wd = root.transform_vector3(tq * wdir).normalize_or_zero();
                    match arm.kind {
                        ArmKind::Claw => {
                            for k in -1..=1 {
                                let d = (fdir + root.transform_vector3(tq * v(k as f32 * 0.35, -0.2, 0.2))).normalize();
                                e.put(Prim::Spike, segment(wh, wh + d * 0.28 * s, 0.05 * s, 0.05 * s, Vec3::Z), Tex::Bone, [1.0; 3], glow_e);
                            }
                        }
                        ArmKind::Blade => {
                            let d = if swinging { wd } else { fdir };
                            e.put(Prim::Blade, segment(wh - d * 0.1 * s, wh + d * 0.8 * s, 0.16 * s, 0.5 * s, Vec3::Y), Tex::Bone, [1.0; 3], glow_e);
                        }
                        ArmKind::Club => {
                            e.put(Prim::Sphere, Mat4::from_translation(wh + fdir * 0.12 * s) * Mat4::from_scale(Vec3::splat(th_a * 3.2)), Tex::Bone, [0.9, 0.85, 0.8], glow_e);
                        }
                        ArmKind::Cannon => {
                            e.put(Prim::Cyl, segment(wh - fdir * 0.1 * s, wh + fdir * 0.3 * s, th_a * 2.0, th_a * 2.0, Vec3::Y), Tex::Flesh, skin, 0.0);
                            e.put(Prim::Sphere, Mat4::from_translation(wh + fdir * 0.3 * s) * Mat4::from_scale(Vec3::splat(th_a * 1.6)), Tex::GrowthGlow, self.plan.glow, 0.8 + glow_e * 2.0);
                        }
                        ArmKind::Shield => {
                            let n = root.transform_vector3(tq * Vec3::Z);
                            let c = wh + n * 0.12 * s;
                            let q = Quat::from_rotation_arc(Vec3::Z, n.normalize());
                            e.put(Prim::Cube, trs(c, q, Vec3::new(0.7, 1.1, 0.1) * s), Tex::Bone, [1.0; 3], 0.0);
                            e.put(Prim::Spike, trs(c + n * 0.05 * s, q * Quat::from_rotation_x(PI * 0.5), Vec3::new(0.2, 0.3, 0.2) * s), Tex::Bone, [0.9; 3], 0.0);
                        }
                        ArmKind::Rifle => {
                            if is_left {
                                // drawn with the right hand
                            } else {
                                e.put(Prim::Cube, segment(wh - wd * 0.25, wh + wd * 0.6, 0.05, 0.09, Vec3::Y) * Mat4::from_translation(Vec3::new(0.0, 0.5, 0.0)) * Mat4::from_scale(Vec3::new(1.0, 1.0, 1.0)) * Mat4::from_translation(Vec3::new(0.0, -0.5, 0.0)), Tex::Metal, [0.2, 0.2, 0.22], 0.0);
                                self.weapon_tip = wh + wd * 0.65;
                            }
                        }
                        ArmKind::Human => {
                            e.put(Prim::Cube, trs(wh, Quat::IDENTITY, Vec3::splat(th_a * 1.3)), Tex::Skin, skin, 0.0);
                        }
                        _ => {}
                    }
                    // held weapon
                    if primary {
                        if let Some(w) = self.plan.hands[slot] {
                            let tip = draw_weapon(&mut e, &mut self.chains, &mut chain_i, w, wh, wd, fdir, root.transform_vector3(tq * Vec3::Z).normalize_or_zero(), s, dt, swinging, time, self.plan.glow);
                            if !is_left || self.plan.hands[0].is_none() {
                                self.weapon_tip = tip;
                            }
                        } else if !is_left && arm.kind != ArmKind::Rifle {
                            self.weapon_tip = wh + fdir * 0.2;
                        }
                    }
                }
            }
            if primary {
                hand_pos[slot] = root.transform_point3(target);
            }
        }
        self.hand_r = hand_pos[0];
        self.hand_l = hand_pos[1];
        let _ = wind;

        // ---- legs
        let p = &self.plan;
        let n = p.leg_count as usize;
        let ll = p.leg_len * s;
        let moving = speed > 0.3;
        let airborne = pos.y > 0.25 || pose.roll > 0.0 || pose.dead > 0.0;
        let any_stepping_group = [self.feet.iter().enumerate().any(|(i, f)| i % 2 == 0 && f.t < 1.0), self.feet.iter().enumerate().any(|(i, f)| i % 2 == 1 && f.t < 1.0)];
        for i in 0..n {
            let (sd, row) = if n == 2 { (if i == 0 { 1.0 } else { -1.0 }, 0.0) } else { (if i % 2 == 0 { 1.0 } else { -1.0 }, (i / 2) as f32 / ((n / 2).max(2) - 1) as f32 - 0.5) };
            let spread = if spider { p.torso.x * 0.5 + ll * 0.75 } else { p.torso.x * 0.28 + 0.03 };
            let hip_j = hip + if spider { Vec3::new(-sd * p.torso.x * 0.45 * s, 0.0, row * p.torso.z * s * 0.9) } else { Vec3::new(-sd * p.torso.x * 0.28 * s, -0.05 * s, 0.0) };
            let home_l = Vec3::new(-sd * spread * s, 0.0, row * (p.torso.z * 1.2 + ll * 0.7) * if spider { 1.0 } else { 0.0 });
            let lead = vel * if spider { 0.12 } else { 0.16 };
            let home_w = pos + rot * home_l + Vec3::new(lead.x, 0.0, lead.z);
            let home_w = Vec3::new(home_w.x, 0.0_f32.max(pos.y - 0.0), home_w.z);
            let f = &mut self.feet[i];
            if !self.init || (f.pos - home_w).length_squared() > 16.0 {
                *f = Foot { pos: home_w, from: home_w, to: home_w, t: 1.0 };
            }
            if airborne {
                // tuck
                let tuck = pos + rot * (Vec3::new(-sd * spread * s * 0.8, 0.0, -0.1)) + Vec3::Y * (hip_h * 0.35);
                f.pos = f.pos.lerp(tuck, (dt * 14.0).min(1.0));
                f.t = 1.0;
            } else if f.t < 1.0 {
                f.t = (f.t + dt * (5.0 + speed * 1.4)).min(1.0);
                f.to = home_w;
                let k = smooth(f.t);
                f.pos = f.from.lerp(f.to, k) + Vec3::Y * ((k * PI).sin() * 0.16 * s);
            } else {
                let d = Vec3::new(f.pos.x - home_w.x, 0.0, f.pos.z - home_w.z).length();
                let thresh = if moving { 0.32 * ll } else { 0.12 * ll } * if spider { 0.8 } else { 1.0 };
                let other = any_stepping_group[(i + 1) % 2];
                if d > thresh && (!other || d > thresh * 2.5) {
                    f.from = f.pos;
                    f.to = home_w;
                    f.t = 0.0;
                }
                f.pos.y = if f.pos.y > pos.y { (f.pos.y - dt * 3.0).max(pos.y) } else { pos.y };
            }
            let mut foot_l = flat_inv.transform_point3(self.feet[i].pos);
            let wf = |q: Vec3| root.transform_point3(q);
            let tint = p.legs_tint;
            let tk = p.leg_thick * s;
            match p.leg_kind {
                LegKind::Spider => {
                    let knee = ik(hip_j, &mut foot_l, ll * 0.55, ll * 0.75, Vec3::new(-sd * 0.4, 1.0, 0.0));
                    e.limb(wf(hip_j), wf(knee), tk * 1.2, p.leg_tex, tint, 0.0);
                    e.put(Prim::Spike, segment(wf(knee), wf(foot_l), tk * 1.4, tk * 1.4, Vec3::Z), p.leg_tex, tint, 0.0);
                }
                LegKind::Digi => {
                    let knee = ik(hip_j, &mut foot_l, ll * 0.5, ll * 0.62, Vec3::new(0.0, 0.2, 1.0));
                    let ankle = knee.lerp(foot_l, 0.55) + Vec3::new(0.0, 0.05, -0.2) * s;
                    e.limb(wf(hip_j), wf(knee), tk * 1.3, p.leg_tex, tint, 0.0);
                    e.limb(wf(knee), wf(ankle), tk, p.leg_tex, tint, 0.0);
                    e.limb(wf(ankle), wf(foot_l), tk * 0.8, p.leg_tex, tint, 0.0);
                }
                _ => {
                    let knee = ik(hip_j, &mut foot_l, ll * 0.5, ll * 0.52, Vec3::new(0.0, 0.1, 1.0));
                    e.limb(wf(hip_j), wf(knee), tk * 1.25, p.leg_tex, tint, 0.0);
                    e.limb(wf(knee), wf(foot_l), tk, p.leg_tex, tint, 0.0);
                    e.put(Prim::Cube, root * trs(foot_l + Vec3::new(0.0, 0.04, 0.06) * s, Quat::IDENTITY, Vec3::new(tk * 1.1, 0.08 * s, tk * 2.2)), Tex::Cloth, [0.12, 0.11, 0.1], 0.0);
                }
            }
        }
        self.init = true;

        // ---- growths
        let growths = self.plan.growths.clone();
        let glowc = self.plan.glow;
        for (gi, g) in growths.iter().enumerate() {
            let (at, q): (Vec3, Quat) = match g.at {
                Attach::Head => (head_c + hq * Vec3::Y * hs * 0.4, hq),
                Attach::Back => (chest_c + tq * Vec3::new(0.0, th * 0.15, -self.plan.torso.z * 0.5 * s), tq * Quat::from_rotation_x(-1.1)),
                Attach::Chest => (chest_c + tq * Vec3::new(0.0, th * 0.1, self.plan.torso.z * 0.5 * s), tq * Quat::from_rotation_x(1.2)),
                Attach::ShoulderL => (hip + tq * Vec3::new(self.plan.torso.x * 0.5 * s, th, 0.0), tq * Quat::from_rotation_z(-0.5)),
                Attach::ShoulderR => (hip + tq * Vec3::new(-self.plan.torso.x * 0.5 * s, th, 0.0), tq * Quat::from_rotation_z(0.5)),
                Attach::ArmL => (root.inverse().transform_point3(self.hand_l.lerp(self.chest, 0.3)), tq * Quat::from_rotation_z(-1.2)),
                Attach::ArmR => (root.inverse().transform_point3(self.hand_r.lerp(self.chest, 0.3)), tq * Quat::from_rotation_z(1.2)),
                Attach::Hip => (hip + tq * Vec3::new(0.0, 0.0, -self.plan.torso.z * 0.5 * s), tq * Quat::from_rotation_x(-1.8)),
                Attach::Legs => (hip + Vec3::new(0.0, -hip_h * 0.45, 0.05), Quat::from_rotation_x(1.3)),
            };
            let gs = g.size * s;
            let gm = root * trs(at, q, Vec3::splat(gs));
            let seed = gi as f32 * 1.7 + self.plan.seed as f32 * 0.01;
            let emis = matches!(g.tex, Tex::GrowthGlow | Tex::RiftGlow | Tex::Ember | Tex::Vein);
            let ge = if emis { 0.7 + 0.3 * (time * 2.0 + seed).sin() + glow_e } else { 0.0 };
            let tint = if emis { glowc } else { [1.0; 3] };
            match g.kind {
                GrowthKind::Spikes => {
                    for k in 0..4 {
                        let a = k as f32 * 0.5 - 0.75;
                        e.put(Prim::Spike, gm * trs(Vec3::new(a * 0.12, 0.0, 0.0), Quat::from_rotation_z(-a) * Quat::from_rotation_x((k as f32 + seed).sin() * 0.3), Vec3::new(0.07, 0.3 + 0.1 * (k % 2) as f32, 0.07)), g.tex, tint, ge);
                    }
                }
                GrowthKind::Sacs => {
                    for k in 0..3 {
                        let o = Vec3::new((k as f32 - 1.0) * 0.12, 0.05 + 0.04 * (k % 2) as f32, 0.02 * k as f32);
                        let pulse = 0.17 + 0.03 * (time * 2.5 + k as f32 * 2.0 + seed).sin();
                        e.put(Prim::Sphere, gm * trs(o, Quat::IDENTITY, Vec3::splat(pulse)), g.tex, tint, ge);
                    }
                }
                GrowthKind::Tumor => {
                    e.put(Prim::Sphere, gm * trs(Vec3::new(0.0, 0.05, 0.0), Quat::from_rotation_y(seed), Vec3::new(0.3, 0.24, 0.28)), g.tex, if emis { tint } else { skin }, ge);
                    e.put(Prim::Sphere, gm * trs(Vec3::new(0.12, 0.12, 0.05), Quat::IDENTITY, Vec3::splat(0.16)), g.tex, if emis { tint } else { skin }, ge);
                }
                GrowthKind::Plates => {
                    for k in 0..3 {
                        e.put(Prim::Plate, gm * trs(Vec3::new((k as f32 - 1.0) * 0.16, 0.02 * k as f32, 0.0), Quat::from_rotation_z((k as f32 - 1.0) * -0.35), Vec3::new(0.24, 0.3, 0.3)), g.tex, tint, ge);
                    }
                }
                GrowthKind::Eyes => {
                    for k in 0..4 {
                        let o = Vec3::new(((k as f32 * 2.1 + seed).sin()) * 0.14, 0.02, ((k as f32 * 1.3 + seed).cos()) * 0.12);
                        e.put(Prim::Sphere, gm * trs(o, Quat::IDENTITY, Vec3::splat(0.07)), Tex::GrowthGlow, glowc, 1.2 + glow_e);
                    }
                }
                GrowthKind::Antlers => {
                    for sd in [-1.0f32, 1.0] {
                        let base = gm * trs(Vec3::new(sd * 0.08, 0.0, 0.0), Quat::from_rotation_z(-sd * 0.6), Vec3::ONE);
                        e.put(Prim::Spike, base * Mat4::from_scale(Vec3::new(0.06, 0.5, 0.06)), g.tex, tint, ge);
                        e.put(Prim::Spike, base * trs(Vec3::new(0.0, 0.2, 0.0), Quat::from_rotation_z(sd * 0.9), Vec3::new(0.05, 0.28, 0.05)), g.tex, tint, ge);
                        e.put(Prim::Spike, base * trs(Vec3::new(0.0, 0.32, 0.0), Quat::from_rotation_x(0.8), Vec3::new(0.04, 0.2, 0.04)), g.tex, tint, ge);
                    }
                }
                GrowthKind::Fins => {
                    for k in 0..3 {
                        e.put(Prim::Wedge, gm * trs(Vec3::new(0.0, 0.0, (k as f32 - 1.0) * 0.14), Quat::from_rotation_y(PI * 0.5) * Quat::from_rotation_z((time * 1.5 + k as f32).sin() * 0.1), Vec3::new(0.2, 0.3 - 0.05 * k as f32, 0.02)), g.tex, tint, ge);
                    }
                }
                GrowthKind::Mandibles => {
                    let open = 0.3 + 0.25 * (time * 3.0 + seed).sin().abs();
                    for sd in [-1.0f32, 1.0] {
                        e.put(Prim::Spike, root * trs(head_c + hq * Vec3::new(sd * hs * 0.35, -hs * 0.35, hs * 0.4), hq * Quat::from_rotation_x(PI * 0.5) * Quat::from_rotation_z(sd * open), Vec3::new(0.06, 0.3, 0.06) * gs), g.tex, tint, ge);
                    }
                }
                GrowthKind::Fronds | GrowthKind::Tail => {
                    let count = if g.kind == GrowthKind::Tail { 1 } else { 3 };
                    for k in 0..count {
                        let ci = self.chain(&mut chain_i, if g.kind == GrowthKind::Tail { 7 } else { 5 }, 0.16 * gs);
                        let o = root.transform_point3(at + q * Vec3::new((k as f32 - 1.0) * 0.1 * gs, 0.0, 0.0));
                        let d = root.transform_vector3(q * Vec3::new((k as f32 - 1.0) * 0.5, 1.0, 0.0)).normalize_or_zero();
                        let wave = Vec3::new((time * 2.0 + k as f32 * 2.0).sin(), 0.0, (time * 1.7 + k as f32).cos()) * 0.02;
                        self.chains[ci].step(o, d, dt, if g.kind == GrowthKind::Tail { 8.0 } else { 2.0 }, None);
                        let last = self.chains[ci].pts.len() - 1;
                        self.chains[ci].pts[last] += wave;
                        e.chain(&self.chains[ci], if g.kind == GrowthKind::Tail { 0.11 } else { 0.06 } * gs, g.tex, if emis { tint } else { skin }, ge);
                    }
                }
                GrowthKind::Arm => {
                    let shl = at;
                    let mut tgt = chest_c + tq * (v(0.0, 0.5 + 0.1 * (time * 1.1).sin(), 0.5) * s);
                    if pose.extra_strike > 0.0 {
                        let (a, b, c) = swing_path(Swing::Overhead);
                        tgt = chest_c + tq * (bez(a, b, c, pose.extra_strike) * 1.3 * s);
                    }
                    let el = ik(shl, &mut tgt, 0.45 * gs, 0.45 * gs, tq * Vec3::new(0.0, 1.0, -0.5));
                    e.limb(root.transform_point3(shl), root.transform_point3(el), 0.09 * gs, g.tex, skin, 0.0);
                    e.limb(root.transform_point3(el), root.transform_point3(tgt), 0.07 * gs, g.tex, skin, 0.0);
                    e.put(Prim::Spike, segment(root.transform_point3(tgt), root.transform_point3(tgt + (tgt - el).normalize_or_zero() * 0.3 * gs), 0.07 * gs, 0.07 * gs, Vec3::Z), Tex::Bone, [1.0; 3], 0.0);
                }
                GrowthKind::Glow => {
                    e.put(Prim::Plate, gm * Mat4::from_scale(Vec3::new(0.28, 0.2, 0.28)), g.tex, glowc, 1.0 + 0.4 * (time * 3.0 + seed).sin() + glow_e);
                }
                GrowthKind::Halo => {
                    for k in 0..5 {
                        let a = time * 1.2 + k as f32 / 5.0 * TAU + seed;
                        let o = Vec3::new(a.cos() * 0.38, 0.1 * (a * 2.0).sin(), a.sin() * 0.38);
                        e.put(Prim::Blade, root * trs(at + o * gs, Quat::from_rotation_y(-a) * Quat::from_rotation_x(time + k as f32), Vec3::new(0.08, 0.16, 0.08) * gs), g.tex, tint, if emis { ge } else { 0.0 });
                    }
                }
                GrowthKind::Cracks => {
                    for k in 0..3 {
                        let o = Vec3::new((k as f32 - 1.0) * 0.1, 0.0, (k as f32 * 2.3 + seed).sin() * 0.1);
                        e.put(Prim::Cube, gm * trs(o, Quat::from_rotation_y(k as f32 * 1.1 + seed), Vec3::new(0.03, 0.03, 0.3)), g.tex, glowc, 1.2 + 0.5 * (time * 4.0 + k as f32).sin() + glow_e);
                    }
                }
            }
        }
    }
}

/// Draw a held weapon starting at the hand; returns the world position of its tip.
#[allow(clippy::too_many_arguments)]
fn draw_weapon(e: &mut Emit, chains: &mut Vec<Chain>, chain_i: &mut usize, w: WeaponVis, hand: Vec3, dir: Vec3, fore: Vec3, body_fwd: Vec3, s: f32, dt: f32, swinging: bool, time: f32, glow: [f32; 3]) -> Vec3 {
    let d = if dir.length_squared() > 0.1 { dir } else { Vec3::Y };
    let hint = fore.cross(d).normalize_or_zero();
    let hint = if hint.length_squared() < 0.1 { Vec3::X } else { hint };
    let seg = |a: f32, b: f32, tx: f32, tz: f32| segment(hand + d * a * s, hand + d * b * s, tx * s, tz * s, hint);
    let t = w.tint;
    match w.class {
        WeaponClass::Knife => {
            e.put(Prim::Cube, seg(-0.08, 0.05, 0.035, 0.035) * Mat4::from_translation(Vec3::new(0.0, 0.5, 0.0)), Tex::Wood, [0.3, 0.25, 0.2], 0.0);
            e.put(Prim::Blade, seg(0.05, 0.38, 0.07, 0.2), Tex::Metal, t, w.glow);
            hand + d * 0.38 * s
        }
        WeaponClass::Baton => {
            e.put(Prim::Cyl, seg(-0.1, 0.62, 0.055, 0.055), Tex::Metal, [t[0] * 0.3, t[1] * 0.3, t[2] * 0.3], w.glow);
            hand + d * 0.62 * s
        }
        WeaponClass::Cleaver => {
            e.put(Prim::Cyl, seg(-0.08, 0.1, 0.04, 0.04), Tex::Wood, [0.4, 0.3, 0.2], 0.0);
            e.put(Prim::Cube, seg(0.1, 0.45, 0.2, 0.02) * Mat4::from_translation(Vec3::new(0.25, 0.5, 0.0)), Tex::Metal, t, w.glow);
            hand + d * 0.45 * s
        }
        WeaponClass::Pipe => {
            e.put(Prim::Cyl, seg(-0.12, 0.85, 0.06, 0.06), Tex::Rust, t, w.glow);
            e.put(Prim::Cyl, seg(0.78, 0.9, 0.09, 0.09), Tex::Metal, t, w.glow);
            hand + d * 0.9 * s
        }
        WeaponClass::FireAxe => {
            e.put(Prim::Cyl, seg(-0.35, 0.85, 0.045, 0.045), Tex::Wood, [0.8, 0.6, 0.4], 0.0);
            e.put(Prim::Wedge, seg(0.62, 0.88, 0.3, 0.05) * Mat4::from_translation(Vec3::new(0.3, 0.0, 0.0)), Tex::Metal, t, w.glow);
            hand + d * 0.9 * s
        }
        WeaponClass::Sledge => {
            e.put(Prim::Cyl, seg(-0.4, 0.9, 0.05, 0.05), Tex::Wood, [0.7, 0.55, 0.4], 0.0);
            e.put(Prim::Cube, seg(0.8, 1.02, 0.36, 0.2) * Mat4::from_translation(Vec3::new(0.0, 0.5, 0.0)), Tex::Metal, t, w.glow);
            hand + d * 1.0 * s
        }
        WeaponClass::Shield => {
            let n = if swinging { d } else { body_fwd };
            let c = hand + n * 0.1 * s;
            let q = Quat::from_rotation_arc(Vec3::Z, n);
            e.put(Prim::Cube, trs(c, q, Vec3::new(0.6, 1.0, 0.05) * s), Tex::Metal, t, w.glow);
            e.put(Prim::Cube, trs(c + q * Vec3::new(0.0, 0.2, 0.03) * s, q, Vec3::new(0.5, 0.25, 0.02) * s), Tex::Glass, [1.0; 3], 0.0);
            c + n * 0.3
        }
        WeaponClass::BoneBlade => {
            e.put(Prim::Blade, seg(-0.05, 1.05, 0.13, 0.4), Tex::Bone, t, w.glow);
            hand + d * 1.05 * s
        }
        WeaponClass::Whip => {
            let ci = *chain_i;
            if chains.len() <= ci {
                chains.push(Chain::new(10, 0.3));
            }
            *chain_i += 1;
            let tip = if swinging { Some((hand + d * 4.0 * s, 0.5)) } else { None };
            chains[ci].step(hand, d, dt, 9.0, tip);
            e.chain(&chains[ci], 0.07 * s, Tex::Vein, t, w.glow);
            *chains[ci].pts.last().unwrap()
        }
        WeaponClass::Catalyst => {
            let c = hand + d * 0.12 * s + Vec3::Y * 0.03 * (time * 3.0).sin();
            e.put(Prim::Sphere, Mat4::from_translation(c) * Mat4::from_scale(Vec3::splat(0.15 * s)), Tex::GrowthGlow, t, 1.2 + w.glow);
            for k in 0..3 {
                let a = time * 2.0 + k as f32 * 2.1;
                e.put(Prim::Spike, trs(c, Quat::from_rotation_y(a) * Quat::from_rotation_z(1.2), Vec3::new(0.04, 0.22, 0.04) * s), Tex::Bone, [1.0; 3], 0.0);
            }
            let _ = glow;
            c
        }
        WeaponClass::Fist => hand + fore * 0.15,
    }
}

/// Stand-alone instanced props that are animated procedurally (not characters).
pub fn weapon_tint(class: WeaponClass, seed: u32) -> [f32; 3] {
    let k = 0.8 + (seed % 7) as f32 * 0.04;
    match class {
        WeaponClass::FireAxe => [0.9 * k, 0.2, 0.15],
        WeaponClass::Shield => [0.55 * k, 0.6 * k, 0.65 * k],
        WeaponClass::BoneBlade => [1.0, 0.95, 0.85],
        WeaponClass::Whip => [0.9, 0.5, 0.6],
        _ => [k, k, k * 1.02],
    }
}
