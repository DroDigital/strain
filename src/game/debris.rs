//! Loose street clutter as simple rigid bodies: kicked by anyone walking through it, thrown by
//! explosions and heavy blows, tumbling and bouncing until it settles. Breakable props (pallets,
//! barrels, barriers, couches, hydrants...) shatter into pieces of their own geometry when a blow
//! or blast is strong enough, and the pieces join the debris.
use super::Game;
use crate::physics::Statics;
use crate::render::mesh::Instance;
use crate::particles::Fx;
use crate::util::rf;
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

    /// Break every breakable prop near `p` that `force` is enough for. Returns how many broke.
    pub fn shatter_near(&mut self, p: Vec3, r: f32, force: f32, dir: Vec3) -> usize {
        let keys: Vec<(i32, i32)> = {
            let k = self.area.key(p);
            (-1..=1).flat_map(|dx| (-1..=1).map(move |dz| (k.0 + dx, k.1 + dz))).collect()
        };
        let mut broken: Vec<((i32, i32), &'static str, Mat4, [f32; 4])> = Vec::new();
        for key in keys {
            let Some(c) = self.area.chunks.get_mut(&key) else { continue };
            let mut i = 0;
            while i < c.props.len() {
                let (name, m, tint) = c.props[i];
                let tough = toughness(name);
                let centre = m.w_axis.truncate() + Vec3::Y * 0.5;
                if tough > 0.0 && force >= tough && (centre - p).length() < r + 0.6 {
                    c.props.remove(i);
                    // its collision box goes with it
                    let at = m.w_axis.truncate();
                    if let Some(ci) = c.cols.iter().position(|b| {
                        let bc = (b.min + b.max) * 0.5;
                        (Vec3::new(bc.x, 0.0, bc.z) - Vec3::new(at.x, 0.0, at.z)).length() < 0.35 && b.max.y < 3.0
                    }) {
                        c.cols.remove(ci);
                    }
                    broken.push((key, name, m, tint));
                } else {
                    i += 1;
                }
            }
        }
        let n = broken.len();
        for (key, name, m, tint) in broken {
            let Some(frags) = self.lib.fragments.get(name).cloned() else { continue };
            let (s, rot, t) = m.to_scale_rotation_translation();
            let centre = m.transform_point3(frags.iter().fold(Vec3::ZERO, |a, f| a + f.1) / frags.len().max(1) as f32);
            for (id, c) in frags {
                let pos = m.transform_point3(c);
                let out = (pos - centre).normalize_or_zero();
                let r = &mut self.rng;
                let vel = out * rf(r, 1.0, 3.0) * force.sqrt() + dir * force * rf(r, 0.4, 0.9) + Vec3::Y * rf(r, 1.0, 3.0);
                let spin = crate::util::rand_dir(r) * rf(r, 3.0, 12.0);
                self.debris.push(Debris { id, key, pos, vel, rot, spin, scale: s.x, tint, mass: 0.5, asleep: false });
            }
            let soft = matches!(name, "Couch" | "Pallet" | "Chest");
            self.parts.burst(Fx::Dust, t + Vec3::Y * 0.4, dir, 12, 1.4);
            self.parts.burst(Fx::Bone, t + Vec3::Y * 0.4, dir + Vec3::Y, if soft { 18 } else { 8 }, 1.3);
            if name == "FireHydrant" {
                // a burst main: a white jet that rains back down
                self.parts.burst(Fx::Water, t + Vec3::Y * 0.3, Vec3::Y * 2.0, 60, 2.0);
                self.hydrants.push((t + Vec3::Y * 0.3, 12.0));
            }
            self.audio.play_at(if soft { crate::audio::Sfx::HitHeavy } else { crate::audio::Sfx::Block }, t, 0.8, rf(&mut self.rng, 0.6, 0.85));
        }
        // the debris that was already loose gets thrown too
        self.debris_impulse(p, r + 0.8, force * 1.5);
        n
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
        // burst hydrants keep spraying for a while
        let mut jets = std::mem::take(&mut self.hydrants);
        for j in &mut jets {
            j.1 -= dt;
            let n = if j.1 > 2.0 { 6 } else { 2 };
            self.parts.burst(Fx::Water, j.0, Vec3::Y * 2.2, n, 1.6 * (j.1 / 12.0).max(0.3));
        }
        jets.retain(|j| j.1 > 0.0);
        self.hydrants = jets;
        if self.debris.is_empty() {
            return;
        }
        self.debris.truncate(400);
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

/// Force needed to break a prop (0 = unbreakable).
pub fn toughness(name: &str) -> f32 {
    match name {
        "Pallet" | "Chest" => 1.2,
        "Barrel" | "PlasticBarrier" | "Pipes" => 2.2,
        "TrafficBarrier_1" | "TrafficBarrier_2" | "Couch" => 3.0,
        "FireHydrant" => 4.5,
        _ => 0.0,
    }
}
