//! Loose street clutter as simple rigid bodies: kicked by anyone walking through it, thrown by
//! explosions and heavy blows, tumbling and bouncing until it settles.
use super::Game;
use crate::physics::Statics;
use crate::render::mesh::Instance;
use glam::{Mat4, Quat, Vec3};

/// Kit models that become dynamic debris instead of static props.
pub const LOOSE: [&str; 7] = ["TrafficCone_1", "TrafficCone_2", "TrashBag_1", "TrashBag_2", "Pallet_Broken", "CinderBlock", "Wheels_Stack"];

#[derive(Clone, Debug)]
pub struct Debris {
    pub id: u32,
    pub key: (i32, i32),
    pub pos: Vec3,
    pub vel: Vec3,
    pub rot: Quat,
    pub spin: Vec3,
    pub scale: f32,
    pub tint: [f32; 4],
    /// rough mass: light things fly further
    pub mass: f32,
    pub asleep: bool,
}

impl Game {
    /// Turn a chunk's loose props into debris bodies.
    pub fn spawn_debris(&mut self, key: (i32, i32)) {
        let Some(c) = self.area.chunks.get(&key) else { return };
        let props: Vec<(&'static str, Mat4, [f32; 4])> = c.props.iter().filter(|p| LOOSE.contains(&p.0)).cloned().collect();
        for (name, m, tint) in props {
            let Some(&id) = self.lib.static_id.get(name) else { continue };
            let (s, r, t) = m.to_scale_rotation_translation();
            let mass = match name {
                "CinderBlock" => 3.0,
                "Wheels_Stack" => 4.0,
                "Pallet_Broken" => 2.0,
                _ => 0.6,
            };
            self.debris.push(Debris { id, key, pos: t, vel: Vec3::ZERO, rot: r, spin: Vec3::ZERO, scale: s.x, tint, mass, asleep: true });
        }
    }

    pub fn despawn_debris(&mut self, key: (i32, i32)) {
        self.debris.retain(|d| d.key != key);
    }

    /// Radial impulse (explosions, slams, heavy hits).
    pub fn debris_impulse(&mut self, p: Vec3, r: f32, force: f32) {
        for d in &mut self.debris {
            let to = d.pos - p;
            let dist = to.length();
            if dist < r {
                let k = (1.0 - dist / r) * force / d.mass;
                d.vel += (to.normalize_or_zero() + Vec3::Y * 0.8) * k;
                d.spin += Vec3::new(to.z, 0.3, -to.x).normalize_or_zero() * k * 2.0;
                d.asleep = false;
            }
        }
    }

    pub fn debris_tick(&mut self, dt: f32) {
        if self.debris.is_empty() {
            return;
        }
        // anyone moving through clutter kicks it along
        let mut kicks: Vec<(usize, Vec3)> = Vec::new();
        for (i, d) in self.debris.iter().enumerate() {
            self.hash.query(d.pos, 0.7, |id, bp, br| {
                let e = self.w.ent(id);
                if let Some(b) = self.w.body.get(e) {
                    let v = b.vel + b.push;
                    let near = Vec3::new(bp.x - d.pos.x, 0.0, bp.z - d.pos.z).length() < br + 0.35;
                    if near && v.length() > 0.8 {
                        kicks.push((i, v));
                    }
                }
            });
        }
        for (i, v) in kicks {
            let d = &mut self.debris[i];
            if d.vel.length() < v.length() * 0.6 {
                d.vel += (v * 1.1 + Vec3::Y * 1.6) / d.mass.sqrt();
                d.spin += Vec3::new(v.z, 0.5, -v.x) * 1.5 / d.mass;
                d.asleep = false;
            }
        }
        for d in &mut self.debris {
            if d.asleep {
                continue;
            }
            d.vel.y -= 14.0 * dt;
            let step = d.vel * dt;
            // walls stop debris
            let flat = Vec3::new(step.x, 0.0, step.z);
            if flat.length() > 1e-4 && self.area.raycast(d.pos + Vec3::Y * 0.2, flat.normalize(), flat.length() + 0.15, crate::physics::SOLID).is_some() {
                d.vel.x *= -0.3;
                d.vel.z *= -0.3;
            } else {
                d.pos += flat;
            }
            d.pos.y += step.y;
            let ground = self.area.ground(d.pos.x, d.pos.z);
            if d.pos.y <= ground {
                d.pos.y = ground;
                if d.vel.y < -1.5 {
                    d.vel.y = -d.vel.y * 0.28;
                } else {
                    d.vel.y = 0.0;
                }
                d.vel.x *= 0.82;
                d.vel.z *= 0.82;
                d.spin *= 0.8;
                // settle upright-ish when slow
                if d.vel.length() < 0.15 && d.spin.length() < 0.2 {
                    d.asleep = true;
                }
            }
            let w = d.spin.length();
            if w > 1e-3 {
                d.rot = (Quat::from_axis_angle(d.spin / w, w * dt) * d.rot).normalize();
            }
        }
    }

    pub fn draw_debris(&mut self, cam: Vec3) {
        for d in &self.debris {
            if (d.pos - cam).length() < 110.0 {
                let m = Mat4::from_scale_rotation_translation(Vec3::splat(d.scale), d.rot, d.pos);
                self.inst.push(d.id, Instance::new(m, d.tint, -1.0));
            }
        }
    }
}
