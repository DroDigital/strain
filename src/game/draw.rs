//! Builds each rendered frame: interpolated actors posed procedurally, pickups with loot beams,
//! projectiles, zone telegraphs, animated world props, helicopters, interactables, and frustum
//! culling of chunks and instances.
use super::comp::*;
use super::Game;
use crate::anim::{BodyPlan, Pose, Rig};
use crate::data::Swing;
use crate::render::mesh::{segment, trs, Instance, Prim};
use crate::render::textures::Tex;
use crate::render::{Distort, Frame, Renderer, StaticMesh};
use crate::util::{angle_lerp, flat, smooth, yaw_dir};
use crate::ecs::Entity;
use crate::world::{AnimKind, InteractKind};
use glam::{Mat4, Quat, Vec3, Vec4};

fn inst(m: Mat4, tex: Tex, c: [f32; 3], emis: f32) -> Instance {
    Instance::new(m, [c[0], c[1], c[2], emis], tex as u32 as f32)
}

/// Conservative frustum test of an AABB in clip space.
fn visible(vp: &Mat4, min: Vec3, max: Vec3) -> bool {
    let mut out = [true; 6];
    for i in 0..8 {
        let p = Vec3::new(if i & 1 == 0 { min.x } else { max.x }, if i & 2 == 0 { min.y } else { max.y }, if i & 4 == 0 { min.z } else { max.z });
        let c = *vp * Vec4::new(p.x, p.y, p.z, 1.0);
        let tests = [c.x < -c.w, c.x > c.w, c.y < -c.w, c.y > c.w, c.z < 0.0, c.z > c.w];
        for k in 0..6 {
            out[k] &= tests[k];
        }
    }
    !out.iter().any(|o| *o)
}

impl Game {
    fn pose_for(&self, e: Entity, a: &Actor) -> Pose {
        let mut p = Pose { flash: a.flash, glow: a.glow, dissolve: a.dissolve, ..Default::default() };
        let act = &a.act;
        match act.kind {
            ActKind::Attack => {
                let mv = act.mv.unwrap();
                p.swing = Some((mv.swing, act.phase(), act.left));
                if act.charging {
                    p.charge = act.charge.max(0.3);
                }
            }
            ActKind::Dodge => p.roll = (act.t / act.dur).clamp(0.0, 0.999),
            ActKind::Stagger => p.stagger = smooth(1.0 - act.t / act.dur) * 0.9,
            ActKind::Opened => {
                p.stagger = 1.0;
                p.cower = 0.6;
                p.glow = 0.5 + 0.5 * (self.sim_time as f32 * 10.0).sin().abs();
            }
            ActKind::Cast => p.cast = (act.t / 0.25).min(1.0),
            ActKind::Heal => p.heal = (act.t / 0.25).min(1.0) * (1.0 - smooth((act.t - act.dur + 0.25) / 0.25)),
            ActKind::Rupture => {
                let ph = (act.t / act.dur * 3.0).min(3.0);
                p.swing = Some((Swing::Thrust, ph, false));
            }
            ActKind::Seized => {
                let t = act.t + e.id as f32;
                p.stagger = 0.5 + 0.5 * (t * 23.0).sin();
                p.flash = p.flash.max(0.3 * (t * 17.0).sin().abs());
            }
            ActKind::Special => {
                let k = (act.t / act.dur).min(1.0);
                p.cast = 1.0;
                p.charge = if k < 0.4 { k * 2.5 } else { 0.0 };
                p.glow = 1.0;
                if act.t < 0.8 {
                    p.swing = Some((Swing::Slam, act.t / 0.8, false));
                } else if act.t < 1.0 {
                    p.swing = Some((Swing::Slam, 1.0 + (act.t - 0.8) / 0.2, false));
                }
            }
            ActKind::Idle => {}
        }
        if !a.alive {
            p.dead = (a.dead_t / 0.7).min(1.0);
            p.swing = None;
            p.dissolve = ((a.dead_t - 5.0) / 2.0).clamp(0.0, 1.0);
        }
        if let Some(ai) = self.w.ai.get(e) {
            match ai.kind {
                crate::data::AiKind::Soldier => p.aim = ai.target.is_some(),
                crate::data::AiKind::Civilian => {
                    if matches!(ai.state, AiState::Cower) {
                        p.cower = 1.0;
                    }
                }
                crate::data::AiKind::Ambusher => p.hide = a.dissolve,
                _ => {}
            }
        }
        if e == self.player {
            p.two_handed = self.pl.two_handing(&self.t);
            p.block = if self.pl.blocking { 1.0 } else { 0.0 };
            p.extra_strike = self.pl.extra_arm_t.max(0.0);
            p.glow = p.glow.max(self.pl.strain_frac() * 0.8);
        }
        p
    }

    pub fn draw(&mut self, r: &mut Renderer, alpha: f32, dt: f32) {
        self.build_start = std::time::Instant::now();
        self.area.upload_pending(r, 4);
        let aspect = r.aspect();
        let view = self.cam.view();
        let proj = self.cam.proj(aspect);
        let vp = proj * view;
        let cam = self.cam.eye;
        let t = self.sim_time as f32;
        self.inst.clear();
        let draw_dist = 105.0;

        // ---- actors
        let ids: Vec<u32> = self.w.rig.ids().collect();
        for id in ids {
            let e = self.w.ent(id);
            let (Some(b), Some(a)) = (self.w.body.get(e).copied(), self.w.actor.get(e).cloned()) else { continue };
            let pos = b.prev.lerp(b.pos, alpha);
            if (pos - cam).length() > draw_dist {
                continue;
            }
            let h = b.height.max(1.0);
            if !visible(&vp, pos - Vec3::new(h, 0.0, h), pos + Vec3::new(h, h * 1.6, h)) {
                continue;
            }
            let yaw = angle_lerp(b.prev_yaw, b.yaw, alpha);
            let pose = self.pose_for(e, &a);
            let vel = b.vel + b.push;
            let lib = self.lib.clone();
            if let Some(rig) = self.w.rig.get_mut(e) {
                if !rig.emit_skinned(&lib, dt, pos, yaw, vel, &pose, &mut self.inst) {
                    rig.update_emit(dt, pos, yaw, vel, &pose, &mut self.inst);
                }
                if e == self.player && rig.stepping() {
                    self.bug_steps += 1;
                }
            }
        }

        // ---- static figures for interactables (quartermaster, husk)
        let inter: Vec<(u32, InteractKind, Vec3)> = self.w.interact.iter().map(|(id, it)| (id, it.kind, self.w.body.get(self.w.ent(id)).unwrap().pos)).collect();
        for (id, kind, p) in inter {
            if (p - cam).length() > draw_dist {
                continue;
            }
            match kind {
                InteractKind::Vendor | InteractKind::Husk => {
                    let rig = self.static_rigs.entry(id).or_insert_with(|| {
                        let mut plan = BodyPlan::human(id);
                        if kind == InteractKind::Vendor {
                            plan.torso_tex = Tex::Camo;
                            plan.cloth = [1.0; 3];
                            plan.head = crate::data::HeadKind::Helmet;
                        } else {
                            plan.skin = [0.35, 0.3, 0.4];
                            plan.cloth = [0.2, 0.18, 0.22];
                            plan.glow = [1.0, 0.3, 0.5];
                            plan.growths.push(crate::data::Growth { kind: crate::data::GrowthKind::Fronds, at: crate::data::Attach::Back, tex: Tex::Vein, size: 1.2 });
                            plan.growths.push(crate::data::Growth { kind: crate::data::GrowthKind::Glow, at: crate::data::Attach::Chest, tex: Tex::GrowthGlow, size: 1.0 });
                        }
                        Rig::new(plan)
                    });
                    let mut pose = Pose::default();
                    let yaw = if kind == InteractKind::Vendor { std::f32::consts::PI } else { 0.0 };
                    if kind == InteractKind::Husk {
                        pose.cower = 1.0;
                        pose.hide = 0.4;
                        pose.glow = 0.6 + 0.4 * (t * 2.0).sin();
                    }
                    rig.update_emit(dt, p, yaw, Vec3::ZERO, &pose, &mut self.inst);
                    if kind == InteractKind::Husk {
                        self.inst.push(Prim::Card as u32, inst(Mat4::from_translation(p) * Mat4::from_rotation_y(-self.cam.yaw) * Mat4::from_scale(Vec3::new(0.15, 30.0, 1.0)), Tex::White, [1.0, 0.25, 0.4], 1.5));
                    }
                }
                InteractKind::RiftPortal(_) | InteractKind::ExitPortal => {
                    let sealed = matches!(kind, InteractKind::RiftPortal(4)) && !self.prof.bosses[..4].iter().all(|b| *b);
                    let k = if sealed { 0.2 } else { 1.0 };
                    let c = if let InteractKind::RiftPortal(i) = kind { crate::world::rift::RULES[i].light } else { [1.4, 0.8, 3.0] };
                    for s in 0..9 {
                        let a = t * (0.6 + s as f32 * 0.05) + s as f32 * 0.7;
                        let rr = 1.3 + 0.2 * (t * 2.0 + s as f32).sin();
                        let q = p + Vec3::new(a.cos() * rr, 1.6 + (a * 1.3).sin() * 0.8, a.sin() * rr);
                        self.inst.push(Prim::Blade as u32, inst(trs(q, Quat::from_rotation_y(-a) * Quat::from_rotation_x(t + s as f32), Vec3::new(0.15, 0.4, 0.15)), Tex::RiftGlow, [c[0].min(1.0), c[1].min(1.0), c[2].min(1.0)], 1.5 * k));
                    }
                    self.inst.push(Prim::Sphere as u32, inst(trs(p + Vec3::Y * 1.6, Quat::from_rotation_y(t), Vec3::new(1.4, 2.4, 0.3) * (0.9 + 0.1 * (t * 3.0).sin())), Tex::RiftGlow, [1.0, 1.0, 1.0], 2.0 * k));
                    if !sealed {
                        self.distorts.push(Distort { pos: p + Vec3::Y * 1.6, radius: 2.4, strength: 0.6, swirl: 1.0 });
                    }
                }
                InteractKind::Note(_) => {
                    self.inst.push(Prim::Cube as u32, inst(trs(p, Quat::from_rotation_y(id as f32), Vec3::new(0.3, 0.02, 0.4)), Tex::Poster, [1.0, 1.0, 0.95], 0.25 + 0.15 * (t * 3.0).sin().abs()));
                }
                _ => {}
            }
        }

        // ---- pickups
        for (id, pk) in self.w.pickup.iter() {
            let e = self.w.ent(id);
            let b = self.w.body.get(e).unwrap();
            let p = b.prev.lerp(b.pos, alpha);
            if (p - cam).length() > draw_dist {
                continue;
            }
            match &pk.kind {
                PickKind::Item(it) => {
                    let c = it.rarity.color();
                    let (w, h) = it.size(&self.t);
                    self.inst.push(Prim::Cube as u32, inst(trs(p + Vec3::Y * 0.12, Quat::from_rotation_y(t * 1.5 + id as f32), Vec3::new(0.18 * w as f32, 0.12, 0.18 * h as f32)), Tex::Metal, c, 0.6));
                    let tall = 2.0 + it.rarity as u8 as f32 * 1.6;
                    let em = 0.6 + it.rarity as u8 as f32 * 0.35;
                    self.inst.push(Prim::Card as u32, inst(Mat4::from_translation(p) * Mat4::from_rotation_y(-self.cam.yaw) * Mat4::from_scale(Vec3::new(0.06 + it.rarity as u8 as f32 * 0.015, tall, 1.0)), Tex::White, c, em));
                }
                PickKind::Biomass(_) => {
                    self.inst.push(Prim::Sphere as u32, inst(trs(p, Quat::IDENTITY, Vec3::splat(0.14 + 0.03 * (t * 6.0 + id as f32).sin())), Tex::GrowthGlow, [1.0, 0.4, 0.7], 1.6));
                }
                PickKind::Scrip(_) => {
                    self.inst.push(Prim::Cube as u32, inst(trs(p, Quat::from_rotation_y(t * 3.0), Vec3::new(0.18, 0.04, 0.1)), Tex::Poster, [1.0, 0.9, 0.5], 0.5));
                }
            }
        }

        // ---- projectiles
        for (id, pr) in self.w.proj.iter() {
            let e = self.w.ent(id);
            let b = self.w.body.get(e).unwrap();
            let p = b.prev.lerp(b.pos, alpha);
            match pr.kind {
                ProjKind::Bullet => {
                    let tail = p - pr.vel.normalize_or_zero() * 1.6;
                    self.inst.push(Prim::Limb as u32, inst(segment(tail, p, 0.03, 0.03, Vec3::Y), Tex::White, [1.0, 0.85, 0.5], 3.0));
                }
                ProjKind::BoneShard => {
                    self.inst.push(Prim::Spike as u32, inst(segment(p - pr.vel.normalize_or_zero() * 0.4, p, 0.08, 0.08, Vec3::Y), Tex::Bone, [1.0; 3], 0.3));
                }
                _ => {
                    let (tex, c, s) = match pr.kind {
                        ProjKind::Acid => (Tex::GrowthGlow, [0.6, 1.0, 0.2], 0.25),
                        ProjKind::Ember => (Tex::Ember, [1.0, 0.6, 0.2], 0.3),
                        ProjKind::BossOrb => (Tex::RiftGlow, [1.0, 0.5, 0.8], 0.55),
                        _ => (Tex::RiftGlow, [0.7, 0.6, 1.0], 0.2),
                    };
                    self.inst.push(Prim::Sphere as u32, inst(trs(p, Quat::from_rotation_y(t * 9.0), Vec3::splat(s * 2.0)), tex, c, 2.5));
                }
            }
        }

        // ---- zones: telegraphs and visuals
        for (id, z) in self.w.zone.iter() {
            let e = self.w.ent(id);
            let p = self.w.body.get(e).unwrap().pos;
            if (p - cam).length() > draw_dist {
                continue;
            }
            match z.kind {
                ZoneKind::Blast => {
                    let k = (z.t / z.delay.max(0.01)).min(1.0);
                    let c = if z.team == Team::Player { [1.0, 0.6, 0.2] } else { [1.0, 0.15, 0.1] };
                    self.inst.push(Prim::Cyl as u32, inst(trs(p + Vec3::Y * 0.03, Quat::IDENTITY, Vec3::new(z.radius * 2.0 * k, 0.03, z.radius * 2.0 * k)), Tex::White, c, 1.0 + k * 1.5));
                    self.inst.push(Prim::Cyl as u32, inst(trs(p + Vec3::Y * 0.02, Quat::IDENTITY, Vec3::new(z.radius * 2.0, 0.02, z.radius * 2.0)), Tex::White, [c[0] * 0.4, c[1] * 0.4, c[2] * 0.4], 0.4));
                    if z.team == Team::Player || z.dur > 1.5 {
                        // falling star
                        let hgt = (1.0 - k) * 30.0;
                        self.inst.push(Prim::Sphere as u32, inst(trs(p + Vec3::Y * hgt, Quat::IDENTITY, Vec3::splat(0.9)), Tex::Ember, [1.0, 0.7, 0.3], 3.0));
                    }
                }
                ZoneKind::Ring => {
                    let r = z.radius * (z.t / z.dur).min(1.0);
                    for i in 0..16 {
                        let a = i as f32 / 16.0 * std::f32::consts::TAU;
                        let q = p + Vec3::new(a.cos() * r, 0.15, a.sin() * r);
                        self.inst.push(Prim::Spike as u32, inst(trs(q, Quat::from_rotation_y(a), Vec3::new(0.3, 0.5 * (1.0 - z.t / z.dur) + 0.1, 0.3)), Tex::Concrete, [0.5, 0.48, 0.45], 0.0));
                    }
                }
                ZoneKind::Spores | ZoneKind::Acid | ZoneKind::Singing => {
                    let c = match z.kind {
                        ZoneKind::Spores => [0.4, 0.9, 0.5],
                        ZoneKind::Acid => [0.6, 1.0, 0.1],
                        _ => [0.8, 0.3, 1.0],
                    };
                    self.inst.push(Prim::Cyl as u32, inst(trs(p + Vec3::Y * 0.02, Quat::IDENTITY, Vec3::new(z.radius * 2.0, 0.015, z.radius * 2.0)), Tex::GrowthGlow, c, 0.5 + 0.2 * (t * 3.0).sin()));
                }
                ZoneKind::Fire => {
                    self.inst.push(Prim::Cyl as u32, inst(trs(p + Vec3::Y * 0.02, Quat::IDENTITY, Vec3::new(z.radius * 2.0, 0.015, z.radius * 2.0)), Tex::Ember, [1.0, 0.6, 0.3], 1.0));
                }
                ZoneKind::Tear => {
                    let a = p - z.dir * z.len * 0.5;
                    let bpt = p + z.dir * z.len * 0.5;
                    self.inst.push(Prim::Blade as u32, inst(segment(a + Vec3::Y * 1.0, bpt + Vec3::Y * 1.0, 0.08, 1.4 + 0.3 * (t * 8.0).sin(), Vec3::Y), Tex::RiftGlow, [1.0, 0.8, 1.0], 2.5));
                }
                ZoneKind::Well | ZoneKind::Vortex => {
                    self.inst.push(Prim::Sphere as u32, inst(trs(p + Vec3::Y, Quat::from_rotation_y(t * 4.0), Vec3::splat(0.6 + 0.1 * (t * 9.0).sin())), Tex::RiftGlow, [0.3, 0.2, 0.6], 2.0));
                    self.distorts.push(Distort { pos: p + Vec3::Y, radius: z.radius * 0.6, strength: 0.8, swirl: 2.0 });
                }
                ZoneKind::Stasis => {
                    self.inst.push(Prim::Cyl as u32, inst(trs(p + Vec3::Y * 0.02, Quat::IDENTITY, Vec3::new(z.radius * 2.0, 0.015, z.radius * 2.0)), Tex::Lattice, [0.6, 0.7, 1.0], 0.6));
                }
                ZoneKind::Beam | ZoneKind::Spikes => {}
            }
        }
        for (q, life) in &self.spikes_fx {
            let k = smooth((1.0 - life) * 5.0) * smooth(*life * 2.0);
            self.inst.push(Prim::Spike as u32, inst(trs(*q, Quat::from_rotation_z((q.x * 7.0).sin() * 0.3), Vec3::new(0.35, 1.8 * k, 0.35)), Tex::Bone, [1.0; 3], 0.0));
        }
        for (a, b, life, c) in &self.tethers_fx {
            let _ = life;
            self.inst.push(Prim::Limb as u32, inst(segment(*a, *b, 0.05, 0.05, Vec3::Y), Tex::Vein, [c[0].min(1.0), c[1].min(1.0), c[2].min(1.0)], 1.5));
        }
        for (a, b) in &self.beams {
            self.inst.push(Prim::Cyl as u32, inst(segment(*a, *b, 0.5 + 0.1 * (t * 30.0).sin(), 0.5, Vec3::Y), Tex::RiftGlow, [1.0, 0.7, 1.0], 3.0));
        }

        // ---- kit models placed by world generation
        for c in self.area.chunks.values() {
            for (name, m, tint) in &c.props {
                let p = m.w_axis.truncate();
                if (p - cam).length() > 120.0 {
                    continue;
                }
                if let Some(&id) = self.lib.static_id.get(*name) {
                    if visible(&vp, p - Vec3::splat(4.0), p + Vec3::new(4.0, 12.0, 4.0)) {
                        self.inst.push(id, Instance::new(*m, *tint, -1.0));
                    }
                }
            }
        }
        // blood pooled where things died
        for (id, m, life) in &self.decals {
            let mut i = Instance::new(*m, [0.55, 0.05, 0.06, 0.0], -1.0);
            // fade out through dithered dissolve in the last seconds
            i.params[3] = if *life < 3.0 { 1.0 - life / 3.0 } else { 0.0 };
            self.inst.push(*id, i);
        }

        // ---- animated world props
        let pp = self.ppos();
        let mut props = Vec::new();
        for c in self.area.chunks.values() {
            for a in &c.anims {
                if (a.pos - cam).length() < 80.0 {
                    props.push(*a);
                }
            }
        }
        for a in props {
            let h = (a.pos.x * 3.1 + a.pos.z * 1.7).sin();
            match a.kind {
                AnimKind::Flag => {
                    for s in 0..5 {
                        let w = (t * 5.0 - s as f32 * 0.9 + h).sin() * 0.25 * s as f32 * 0.3;
                        let q = a.pos + Vec3::new(0.3 + s as f32 * 0.3, -0.35, w);
                        self.inst.push(Prim::Cube as u32, inst(trs(q, Quat::from_rotation_y(w), Vec3::new(0.32, 0.7, 0.02)), Tex::Cloth, if s % 2 == 0 { [0.6, 0.1, 0.1] } else { [0.85, 0.8, 0.7] }, 0.0));
                    }
                }
                AnimKind::Tendril => {
                    let n = 6;
                    let mut prev = a.pos;
                    // tendrils lean toward the player when near
                    let lean = flat(pp - a.pos);
                    let lean = if lean.length() < 8.0 { lean.normalize_or_zero() * (1.0 - lean.length() / 8.0) } else { Vec3::ZERO };
                    for s in 1..=n {
                        let k = s as f32 / n as f32;
                        let sway = Vec3::new((t * 0.9 + h + k * 2.0).sin(), 0.0, (t * 0.7 + h * 2.0 + k * 1.5).cos()) * k * k * 0.9 * a.size * 0.2;
                        let q = a.pos + Vec3::Y * (k * a.size) + sway + lean * k * k * a.size * 0.5;
                        let th = 0.18 * a.size.sqrt() * (1.0 - k * 0.75);
                        self.inst.push(Prim::Limb as u32, inst(segment(prev, q, th, th, Vec3::Z), Tex::Vein, [0.9, 0.6, 0.7], 0.15));
                        prev = q;
                    }
                    self.inst.push(Prim::Sphere as u32, inst(trs(prev, Quat::IDENTITY, Vec3::splat(0.15 * a.size.sqrt())), Tex::GrowthGlow, [0.5, 1.0, 0.7], 1.2));
                }
                AnimKind::Pod => {
                    let s = a.size * (0.5 + 0.06 * (t * 1.8 + h).sin());
                    self.inst.push(Prim::Sphere as u32, inst(trs(a.pos + Vec3::Y * s * 0.45, Quat::from_rotation_y(a.yaw), Vec3::new(s, s * 1.2, s)), Tex::Growth, [1.0, 0.85, 0.95], 0.2));
                    self.inst.push(Prim::Sphere as u32, inst(trs(a.pos + Vec3::Y * s * 0.7, Quat::IDENTITY, Vec3::splat(s * 0.35)), Tex::GrowthGlow, [0.5, 1.0, 0.7], 1.0 + 0.5 * (t * 2.0 + h).sin()));
                }
                AnimKind::Litter => {
                    for k in 0..4 {
                        let ph = t * 0.4 + k as f32 * 1.7 + h;
                        let q = a.pos + Vec3::new((ph * 1.3).sin() * 3.0, 0.05 + ((ph * 3.0).sin() * 0.5 + 0.5) * 0.4, (ph * 0.9).cos() * 3.0);
                        self.inst.push(Prim::Cube as u32, inst(trs(q, Quat::from_rotation_y(ph * 3.0) * Quat::from_rotation_x(ph * 2.0), Vec3::new(0.25, 0.01, 0.3)), Tex::Poster, [0.9, 0.88, 0.8], 0.0));
                    }
                }
                AnimKind::Shard => {
                    let q = a.pos + Vec3::Y * (t * 1.1 + h).sin() * 0.4;
                    self.inst.push(Prim::Blade as u32, inst(trs(q, Quat::from_rotation_y(t * 0.7 + a.yaw) * Quat::from_rotation_z(0.4), Vec3::new(0.6, 1.4, 0.6) * a.size), Tex::RiftGlow, [1.0, 1.0, 1.0], 1.2));
                }
                AnimKind::Sign => {}
            }
        }

        // ---- helicopters
        for hl in &self.helis {
            let p = hl.pos;
            let yaw = crate::util::dir_yaw(hl.vel);
            let q = Quat::from_rotation_y(yaw) * Quat::from_rotation_x(0.1);
            self.inst.push(Prim::Cube as u32, inst(trs(p, q, Vec3::new(1.8, 1.6, 4.0)), Tex::Camo, [0.5, 0.55, 0.5], 0.0));
            self.inst.push(Prim::Limb as u32, inst(segment(p - yaw_dir(yaw) * 1.5, p - yaw_dir(yaw) * 6.0 + Vec3::Y * 0.6, 0.35, 0.35, Vec3::Y), Tex::Camo, [0.5, 0.55, 0.5], 0.0));
            for k in 0..2 {
                self.inst.push(Prim::Cube as u32, inst(trs(p + Vec3::Y * 1.0, Quat::from_rotation_y(t * 25.0 + k as f32 * 1.57), Vec3::new(9.0, 0.05, 0.3)), Tex::Metal, [0.15; 3], 0.0));
            }
            self.inst.push(Prim::Sphere as u32, inst(trs(p - Vec3::Y * 0.9 + yaw_dir(yaw) * 1.6, Quat::IDENTITY, Vec3::splat(0.4)), Tex::White, [1.0, 1.0, 1.0], 3.0));
        }

        // ---- chunks with frustum culling
        let mut statics: Vec<&StaticMesh> = Vec::new();
        let mut shadow: Vec<&StaticMesh> = Vec::new();
        let sc = pp;
        for c in self.area.chunks.values() {
            if let Some(m) = &c.mesh {
                let center = (m.min + m.max) * 0.5;
                let near = flat(center - sc).length() < 220.0;
                if near && visible(&vp, m.min, m.max) {
                    statics.push(m);
                }
                // shadow casters: chunks near the player (shadow map covers ~48 m)
                if flat(center - sc).length() < 80.0 {
                    shadow.push(m);
                }
            }
        }

        for (k, m) in self.area.far.iter() {
            if self.area.chunks.get(k).map(|c| c.mesh.is_some()).unwrap_or(false) {
                continue;
            }
            if let Some(m) = m {
                let center = (m.min + m.max) * 0.5;
                if flat(center - cam).length() < 420.0 && visible(&vp, m.min, m.max) {
                    statics.push(m);
                }
            }
        }

        // ---- particles, lights, environment, ui
        self.part_buf.clear();
        self.parts.fill(&mut self.part_buf);
        let mut lights = std::mem::take(&mut self.light_buf);
        let key = self.gather_lights(&mut lights, t);
        let mut key_statics: Vec<&StaticMesh> = Vec::new();
        if let Some((kp, kr)) = key {
            for c in self.area.chunks.values() {
                if let Some(m) = &c.mesh {
                    let q = kp.clamp(m.min, m.max);
                    if (q - kp).length() < kr {
                        key_statics.push(m);
                    }
                }
            }
        }
        for (p, rr, s, sw) in &self.distorts_temp {
            self.distorts.push(Distort { pos: *p, radius: *rr, strength: *s, swirl: *sw });
        }
        let strain_vis = (self.pl.strain_frac() - 0.3).max(0.0) * 1.2 + if matches!(self.w.actor.get(self.player).map(|a| a.act.kind), Some(ActKind::Seized)) { 0.8 } else { 0.0 };
        let env = self.build_env(strain_vis.min(1.0));
        let (uw, uh) = r.ui_size();
        let mut ui = std::mem::take(&mut self.ui_buf);
        ui.clear();
        let hits = self.build_ui(&mut ui, uw, uh);
        let distorts = std::mem::take(&mut self.distorts);
        self.build_ms = self.build_ms * 0.95 + self.build_start.elapsed().as_secs_f32() * 1000.0 * 0.05;
        r.render(&Frame {
            view,
            proj,
            cam_pos: cam,
            time: t,
            env,
            shadow_center: pp,
            statics,
            shadow_statics: shadow,
            inst: &self.inst,
            lights: &lights,
            distorts: &distorts,
            particles: &self.part_buf,
            ui: &ui,
            key,
            key_statics,
            skinned: &self.inst.skinned,
            joints: &self.inst.joints,
        });
        self.ui_buf = ui;
        self.ui.hits = hits;
        self.light_buf = lights;
        self.distorts = distorts;
        self.distorts.clear();
    }
}
