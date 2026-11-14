//! CPU-simulated, GPU-instanced particles.
use crate::render::ParticleInst;
use crate::util::{rand_sphere, rf, Rng};
use glam::Vec3;

#[derive(Clone, Copy)]
pub struct P {
    pub pos: Vec3,
    pub vel: Vec3,
    pub life: f32,
    pub max: f32,
    pub size: f32,
    pub size_end: f32,
    pub col: [f32; 4],
    pub col_end: [f32; 4],
    /// 0 soft, 1 square, 2 smoke, 3 streak
    pub kind: f32,
    pub grav: f32,
    pub drag: f32,
    pub add: f32,
    pub stretch: f32,
    pub rot: f32,
    pub floor: bool,
    /// leaves a stain where it lands
    pub stain: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Fx {
    Blood,
    Ichor,
    Sparks,
    Dust,
    Spore,
    Fire,
    Smoke,
    Ember,
    Rift,
    Heal,
    Bone,
    Flash,
    Acid,
    Biomass,
    Strain,
    Shock,
    /// a short-lived streak left behind a swinging weapon
    Trail,
    /// torn flesh and tissue
    Gore,
    /// a fine red haze where a blow lands
    BloodMist,
    /// spray from a burst water main
    Water,
    /// a jet of blood thrown along the blow
    Spurt,
    /// a fine red mist hanging where a blow landed
    Mist,
}

pub struct Particles {
    pub list: Vec<P>,
    pub density: f32,
    rng: Rng,
    /// staining particles that reached the ground this frame: (where, size, colour)
    pub landed: Vec<(Vec3, f32, [f32; 3])>,
}

const MAX: usize = 7000;

impl Particles {
    pub fn new() -> Self {
        Particles { list: Vec::with_capacity(MAX), density: 1.0, rng: Rng::with_seed(99), landed: Vec::new() }
    }
    pub fn clear(&mut self) {
        self.list.clear();
    }
    pub fn push(&mut self, p: P) {
        if self.list.len() < MAX {
            self.list.push(p);
        }
    }
    /// Emit `n` particles of a preset. `dir` biases the initial velocity.
    pub fn burst(&mut self, fx: Fx, pos: Vec3, dir: Vec3, n: u32, power: f32) {
        let n = ((n as f32 * self.density).ceil() as u32).max(1);
        for _ in 0..n {
            let r = &mut self.rng;
            let rs = rand_sphere(r);
            let base = P { pos, vel: Vec3::ZERO, life: 0.0, max: 1.0, size: 0.1, size_end: 0.0, col: [1.0; 4], col_end: [1.0, 1.0, 1.0, 0.0], kind: 0.0, grav: 0.0, drag: 0.0, add: 0.0, stretch: 0.0, rot: rf(r, 0.0, 6.28), floor: false, stain: false };
            let p = match fx {
                Fx::Blood | Fx::Ichor | Fx::Acid => {
                    let c = match fx {
                        Fx::Blood => [0.32, 0.012, 0.015, 1.0],
                        Fx::Ichor => [0.14, 0.025, 0.12, 1.0],
                        _ => [0.35, 0.55, 0.06, 1.0],
                    };
                    // a cone of droplets of every size: fine spray flies fast and far, heavy drops fall
                    let big = r.f32();
                    let size = 0.008 + big * big * 0.05;
                    let cone = (dir * 1.8 + rs * 0.7).normalize_or_zero();
                    let speed = rf(r, 1.5, 6.5) * power * (1.2 - big * 0.6);
                    P { vel: cone * speed + Vec3::Y * rf(r, 0.5, 2.0), max: rf(r, 0.5, 1.2), size, size_end: size * 0.8, col: c, col_end: [c[0] * 0.7, c[1] * 0.7, c[2] * 0.7, 0.9], kind: 1.0, grav: 9.81, drag: 0.4, stretch: 2.2, add: if fx == Fx::Acid { 0.25 } else { 0.0 }, floor: true, stain: r.f32() < 0.12, ..base }
                }
                Fx::Gore => {
                    let c = [rf(r, 0.22, 0.35), rf(r, 0.03, 0.06), rf(r, 0.03, 0.05), 1.0];
                    P { vel: (dir * 1.2 + rs) * rf(r, 1.5, 4.5) * power + Vec3::Y * rf(r, 1.0, 3.0), max: rf(r, 0.9, 1.8), size: rf(r, 0.025, 0.07), size_end: 0.015, col: c, col_end: [c[0] * 0.6, c[1] * 0.6, c[2] * 0.6, 0.0], kind: 1.0, grav: 11.0, drag: 0.2, floor: true, stain: r.f32() < 0.3, ..base }
                }
                Fx::Water => P { pos: pos + rs * 0.05, vel: (dir + rs * 0.25) * rf(r, 2.5, 4.5) * power, max: rf(r, 0.8, 1.6), size: rf(r, 0.015, 0.04), size_end: 0.05, col: [0.75, 0.8, 0.85, 0.55], col_end: [0.7, 0.75, 0.8, 0.0], kind: 1.0, grav: 9.81, drag: 0.3, stretch: 3.0, floor: true, ..base },
                Fx::BloodMist => P { pos: pos + rs * 0.1, vel: (dir * 1.4 + rs * 0.6) * power, max: rf(r, 0.35, 0.7), size: rf(r, 0.12, 0.22), size_end: rf(r, 0.5, 0.8), col: [0.3, 0.02, 0.02, 0.32], col_end: [0.22, 0.02, 0.02, 0.0], kind: 2.0, drag: 4.0, ..base },
                Fx::Sparks => P { vel: (dir + rs * 0.9) * rf(r, 3.0, 9.0) * power, max: rf(r, 0.15, 0.45), size: 0.03, size_end: 0.01, col: [3.0, 2.2, 0.8, 1.0], col_end: [2.0, 0.5, 0.1, 0.0], kind: 3.0, grav: 9.0, add: 1.0, stretch: 4.0, ..base },
                Fx::Bone => P { vel: (dir + rs) * rf(r, 2.0, 5.0) * power + Vec3::Y * 2.0, max: rf(r, 0.5, 1.0), size: rf(r, 0.03, 0.07), size_end: 0.03, col: [0.9, 0.85, 0.7, 1.0], col_end: [0.8, 0.75, 0.6, 0.0], kind: 1.0, grav: 14.0, floor: true, ..base },
                Fx::Dust => P { pos: pos + rs * 0.2, vel: (dir * 0.6 + rs * 0.5) * power + Vec3::Y * 0.4, max: rf(r, 0.5, 1.2), size: rf(r, 0.15, 0.3), size_end: 0.7, col: [0.35, 0.33, 0.3, 0.35], col_end: [0.3, 0.3, 0.3, 0.0], kind: 2.0, drag: 2.0, ..base },
                Fx::Smoke => P { pos: pos + rs * 0.3, vel: Vec3::new(rs.x * 0.4, rf(r, 1.0, 2.2), rs.z * 0.4) * power + dir, max: rf(r, 2.5, 5.0), size: rf(r, 0.4, 0.7), size_end: 2.4, col: [0.08, 0.08, 0.09, 0.5], col_end: [0.12, 0.12, 0.13, 0.0], kind: 2.0, drag: 0.3, ..base },
                Fx::Spore => P { pos: pos + rs * power, vel: rs * 0.25 + Vec3::Y * 0.12 + dir, max: rf(r, 1.5, 4.0), size: rf(r, 0.02, 0.05), size_end: 0.02, col: [0.9, 2.0, 0.9, 0.9], col_end: [0.3, 1.2, 0.5, 0.0], kind: 0.0, drag: 0.5, add: 1.0, ..base },
                Fx::Fire => P { pos: pos + rs * 0.25 * power, vel: Vec3::new(rs.x * 0.5, rf(r, 1.2, 3.0), rs.z * 0.5) + dir, max: rf(r, 0.3, 0.8), size: rf(r, 0.18, 0.4) * power.max(0.6), size_end: 0.05, col: [4.0, 1.8, 0.4, 0.9], col_end: [1.5, 0.15, 0.02, 0.0], kind: 2.0, add: 1.0, ..base },
                Fx::Ember => P { pos: pos + rs * 0.3 * power, vel: Vec3::new(rs.x, rf(r, 1.0, 3.5), rs.z) + dir, max: rf(r, 0.8, 2.2), size: 0.025, size_end: 0.01, col: [4.0, 1.5, 0.3, 1.0], col_end: [1.5, 0.2, 0.0, 0.0], kind: 1.0, grav: -0.5, drag: 0.4, add: 1.0, ..base },
                Fx::Rift => P { pos: pos + rs * 0.4 * power, vel: rs * rf(r, 0.5, 3.0) + dir, max: rf(r, 0.4, 1.1), size: rf(r, 0.04, 0.1), size_end: 0.0, col: [1.6, 1.0, 3.5, 1.0], col_end: [0.4, 0.2, 2.0, 0.0], kind: 1.0, drag: 2.5, add: 1.0, ..base },
                Fx::Heal => P { pos: pos + Vec3::new(rs.x * 0.5, rf(r, 0.0, 1.6), rs.z * 0.5), vel: Vec3::Y * rf(r, 0.5, 1.5), max: rf(r, 0.5, 1.1), size: 0.05, size_end: 0.0, col: [0.6, 3.0, 1.2, 1.0], col_end: [0.2, 1.5, 0.6, 0.0], kind: 0.0, add: 1.0, ..base },
                Fx::Flash => P { max: 0.12 * power.max(0.5), size: 0.6 * power, size_end: 1.6 * power, col: [4.0, 3.6, 3.0, 0.9], col_end: [3.0, 2.0, 1.0, 0.0], add: 1.0, ..base },
                Fx::Biomass => P { pos: pos + rs * 0.3, vel: rs * 1.5 + Vec3::Y * 2.0, max: rf(r, 0.5, 0.9), size: 0.05, size_end: 0.0, col: [2.5, 0.6, 1.2, 1.0], col_end: [1.2, 0.2, 0.8, 0.0], add: 1.0, drag: 2.0, ..base },
                Fx::Strain => P { pos: pos + Vec3::new(rs.x * 0.4, rf(r, 0.2, 1.7), rs.z * 0.4), vel: rs * 0.6 + Vec3::Y, max: rf(r, 0.3, 0.7), size: 0.04, size_end: 0.0, col: [2.6, 0.5, 3.0, 1.0], col_end: [1.0, 0.1, 1.5, 0.0], kind: 1.0, add: 1.0, ..base },
                Fx::Trail => P { pos: pos + rs * 0.02, vel: dir * 0.3, max: 0.12, size: 0.045 * power.max(0.5), size_end: 0.01, col: [0.55, 0.55, 0.55, 0.16], col_end: [0.4, 0.4, 0.4, 0.0], kind: 0.0, add: 1.0, ..base },
                Fx::Spurt => P { pos: pos + rs * 0.04, vel: (dir * rf(r, 2.5, 6.5) + rs * 0.8 + Vec3::Y * rf(r, 0.5, 2.0)) * power, max: rf(r, 0.5, 1.0), size: rf(r, 0.02, 0.05), size_end: 0.015, col: [0.36, 0.01, 0.02, 1.0], col_end: [0.22, 0.0, 0.01, 0.9], kind: 3.0, grav: 13.0, stretch: 2.5, floor: true, ..base },
                Fx::Mist => P { pos: pos + rs * 0.12, vel: dir * rf(r, 0.4, 1.2) + rs * 0.35, max: rf(r, 0.3, 0.7), size: rf(r, 0.06, 0.12) * power.max(0.6), size_end: 0.32 * power.max(0.6), col: [0.3, 0.02, 0.03, 0.22], col_end: [0.2, 0.02, 0.02, 0.0], kind: 2.0, drag: 3.0, grav: 0.4, ..base },
                Fx::Shock => {
                    let a = rf(r, 0.0, 6.283);
                    let d = Vec3::new(a.cos(), 0.0, a.sin());
                    P { pos: pos + d * 0.3 + Vec3::Y * 0.1, vel: d * power * rf(r, 0.85, 1.0) + Vec3::Y * rf(r, 0.0, 0.6), max: 0.5, size: 0.22, size_end: 0.5, col: [0.5, 0.48, 0.42, 0.6], col_end: [0.4, 0.4, 0.4, 0.0], kind: 2.0, drag: 1.5, ..base }
                }
            };
            self.push(p);
        }
    }
    pub fn update(&mut self, dt: f32) {
        for p in &mut self.list {
            p.life += dt;
            p.vel.y -= p.grav * dt;
            p.vel *= 1.0 - (p.drag * dt).min(0.9);
            p.pos += p.vel * dt;
            if p.floor && p.pos.y < 0.03 {
                p.pos.y = 0.03;
                if p.stain && p.vel.y < -0.5 {
                    self.landed.push((p.pos, (p.size * 9.0).clamp(0.12, 0.45), [p.col[0] * 0.6, p.col[1] * 0.6, p.col[2] * 0.6]));
                    p.stain = false;
                }
                p.vel = Vec3::ZERO;
                p.grav = 0.0;
                p.stretch = 0.0;
            }
        }
        self.list.retain(|p| p.life < p.max);
    }
    pub fn fill(&self, out: &mut Vec<ParticleInst>) {
        for p in &self.list {
            let t = p.life / p.max;
            let l = |a: f32, b: f32| a + (b - a) * t;
            out.push(ParticleInst {
                pos_size: [p.pos.x, p.pos.y, p.pos.z, l(p.size, p.size_end)],
                color: [l(p.col[0], p.col_end[0]), l(p.col[1], p.col_end[1]), l(p.col[2], p.col_end[2]), l(p.col[3], p.col_end[3])],
                par: [p.kind, p.rot + t, p.stretch, p.add],
                vel: [p.vel.x, p.vel.y, p.vel.z, 0.0],
            });
        }
    }
}
