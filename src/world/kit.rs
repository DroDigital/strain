//! Modular procedural building kit and city chunk generator: facades, shopfronts, alleys,
//! open interiors, rooftops, subway entrances, street furniture and district dressing.
use super::*;
use crate::physics::Aabb;
use crate::render::mesh::{rgb, rgbe, trs, WHITE};
use crate::render::textures::Tex;
use crate::util::{chance, hash01, noise2, rf, seed_rng, Rng};
use glam::{Mat4, Quat, Vec2, Vec3};

const FLOOR_H: f32 = 3.2;

struct Gen {
    g: ChunkGen,
    r: Rng,
    o: Vec3,
    d: usize,
    state: DState,
    /// 0 = infected ground, 1 = military ground (shifts every rest)
    control: f32,
    cx: i32,
    cz: i32,
}

fn t(x: Tex) -> u32 {
    x as u32
}

impl Gen {
    fn p(&self, x: f32, y: f32, z: f32) -> Vec3 {
        self.o + Vec3::new(x, y, z)
    }
    fn bx(&mut self, min: Vec3, max: Vec3, tex: Tex, top: Tex, col: [u8; 4], tile: f32) {
        let (a, b) = (self.o + min, self.o + max);
        self.g.mesh.boxx(a, b, t(tex), t(top), col, tile, 63 & !8);
    }
    fn solid(&mut self, min: Vec3, max: Vec3, tex: Tex, top: Tex, col: [u8; 4], tile: f32) {
        self.bx(min, max, tex, top, col, tile);
        self.g.cols.push(Aabb::new(self.o + min, self.o + max));
    }
    fn low(&mut self, min: Vec3, max: Vec3, tex: Tex, col: [u8; 4], tile: f32) {
        self.bx(min, max, tex, tex, col, tile);
        self.g.cols.push(Aabb::low(self.o + min, self.o + max));
    }
    fn light(&mut self, x: f32, y: f32, z: f32, c: [f32; 3], radius: f32, flicker: u8) {
        let pos = self.p(x, y, z);
        self.g.lights.push(CLight { pos, color: Vec3::from(c), radius, flicker });
    }
    fn spawn(&mut self, x: f32, z: f32, kind: SpawnKind) {
        let pos = self.p(x, 0.0, z);
        let seed = self.r.u32(..);
        self.g.spawns.push(SpawnPt { pos, kind, seed });
    }

    fn ground(&mut self) {
        let m = &mut self.g.mesh;
        let o = self.o;
        let v = |x: f32, y: f32, z: f32| o + Vec3::new(x, y, z);
        let lost = self.state == DState::Lost;
        let road = if lost && chance(&mut self.r, 0.35) { Tex::Flesh } else { Tex::Asphalt };
        let rc = if road == Tex::Flesh { rgb(0.55, 0.45, 0.5) } else { WHITE };
        m.quad([v(0.0, 0.0, CS), v(CS, 0.0, CS), v(CS, 0.0, 0.0), v(0.0, 0.0, 0.0)], CS / 4.0, CS / 4.0, t(road), rc);
        // lane markings along the west and south borders (road centre lines)
        if road == Tex::Asphalt {
            m.quad([v(-0.6, 0.01, CS - ROAD), v(0.6, 0.01, CS - ROAD), v(0.6, 0.01, ROAD), v(-0.6, 0.01, ROAD)], 1.0, (CS - 2.0 * ROAD) / 4.0, t(Tex::RoadLine), WHITE);
            m.quad_uv(
                [v(ROAD, 0.01, 0.6), v(CS - ROAD, 0.01, 0.6), v(CS - ROAD, 0.01, -0.6), v(ROAD, 0.01, -0.6)],
                [Vec2::new(0.0, 0.0), Vec2::new(0.0, (CS - 2.0 * ROAD) / 4.0), Vec2::new(1.0, (CS - 2.0 * ROAD) / 4.0), Vec2::new(1.0, 0.0)],
                t(Tex::RoadLine),
                WHITE,
            );
        }
        let side = if lost { Tex::Growth } else { Tex::Sidewalk };
        let sc = if lost { rgb(0.6, 0.55, 0.6) } else { WHITE };
        m.boxx(v(ROAD, 0.0, ROAD), v(CS - ROAD, KERB, CS - ROAD), t(Tex::Concrete), t(side), sc, 3.0, 63 & !8);
        // tissue fusing the street in lost districts
        if lost {
            for _ in 0..4 {
                let (x, z, s) = (rf(&mut self.r, 2.0, CS - 2.0), rf(&mut self.r, 2.0, CS - 2.0), rf(&mut self.r, 2.0, 5.0));
                let onroad = x < ROAD || x > CS - ROAD || z < ROAD || z > CS - ROAD;
                let y = if onroad { 0.02 } else { KERB + 0.02 };
                self.g.mesh.quad([v(x - s, y, z + s), v(x + s, y, z + s), v(x + s, y, z - s), v(x - s, y, z - s)], 1.5, 1.5, t(Tex::Vein), rgbe(0.8, 0.5, 0.6, 0.08));
            }
        }
    }

    /// A building from the facade kit. `open` makes the ground floor an enterable interior.
    fn building(&mut self, x0: f32, z0: f32, x1: f32, z1: f32, floors: u32, open: bool) {
        let style = self.r.usize(0..3);
        let (plain, win, lit) = [(Tex::Brick, Tex::BrickWindow, Tex::BrickWindowLit), (Tex::Concrete, Tex::ConcreteWindow, Tex::ConcreteWindowLit), (Tex::Plaster, Tex::PlasterWindow, Tex::PlasterWindowLit)][style];
        let lit_p = match self.state {
            DState::Held => 0.22,
            DState::Contested => 0.07,
            DState::Lost => 0.015,
        };
        let tintv = rf(&mut self.r, 0.75, 1.0);
        let lost = self.state == DState::Lost;
        let wall_col = if lost { rgb(tintv * 0.8, tintv * 0.7, tintv * 0.85) } else { rgb(tintv, tintv, tintv * rf(&mut self.r, 0.9, 1.0)) };
        let shop = chance(&mut self.r, 0.55) || open;
        let h = floors as f32 * FLOOR_H + KERB;
        let o = self.o;
        // sides: (start corner, direction, length, outward normal index)
        let sides = [
            (Vec3::new(x0, 0.0, z0), Vec3::X, x1 - x0, 0),
            (Vec3::new(x1, 0.0, z0), Vec3::Z, z1 - z0, 1),
            (Vec3::new(x1, 0.0, z1), Vec3::NEG_X, x1 - x0, 2),
            (Vec3::new(x0, 0.0, z1), Vec3::NEG_Z, z1 - z0, 3),
        ];
        let door_side = self.r.usize(0..4);
        for (si, (start, dir, len, _)) in sides.iter().enumerate() {
            let bays = (len / 3.2).round().max(1.0) as u32;
            let bw = len / bays as f32;
            let door_bay = bays / 2;
            for f in 0..floors {
                let (y0, y1) = (KERB + f as f32 * FLOOR_H, KERB + (f + 1) as f32 * FLOOR_H);
                let emit = |m: &mut MeshBuilder, b0: u32, b1: u32, tex: Tex, col: [u8; 4]| {
                    let a = o + *start + *dir * (b0 as f32 * bw);
                    let b = o + *start + *dir * (b1 as f32 * bw);
                    // counter-clockwise seen from outside
                    m.quad([Vec3::new(b.x, y0, b.z), Vec3::new(a.x, y0, a.z), Vec3::new(a.x, y1, a.z), Vec3::new(b.x, y1, b.z)], (b1 - b0) as f32, 1.0, t(tex), col);
                };
                if f == 0 {
                    for b in 0..bays {
                        let is_door = si == door_side && (b == door_bay || (open && b + 1 == door_bay));
                        if is_door && open {
                            continue;
                        }
                        let tex = if is_door {
                            Tex::Door
                        } else if shop && si == door_side {
                            if chance(&mut self.r, lit_p * 2.5) { Tex::ShopfrontLit } else { Tex::Shopfront }
                        } else if chance(&mut self.r, 0.12) && !lost {
                            Tex::Graffiti
                        } else {
                            plain
                        };
                        let col = if tex == Tex::ShopfrontLit { [255, 255, 255, 200] } else if tex == Tex::Graffiti || tex == Tex::Door || tex == Tex::Shopfront { WHITE } else { wall_col };
                        emit(&mut self.g.mesh, b, b + 1, tex, col);
                        if tex == Tex::Graffiti {
                            let c = *start + *dir * ((b as f32 + 0.5) * bw);
                            let n = Vec3::new(dir.z, 0.0, -dir.x);
                            let pos = self.p(c.x + n.x * 0.15, 1.7, c.z + n.z * 0.15);
                            let idx = self.r.u32(0..64) as f32;
                            self.g.anims.push(AnimProp { kind: AnimKind::Sign, pos, yaw: idx, size: 1.0 });
                        }
                        if tex == Tex::ShopfrontLit {
                            let c = *start + *dir * ((b as f32 + 0.5) * bw);
                            let n = Vec3::new(dir.z, 0.0, -dir.x);
                            let fl = if chance(&mut self.r, 0.3) { 3 } else { 0 };
                            self.light(c.x + n.x * 1.5, 2.2, c.z + n.z * 1.5, [1.6, 1.5, 1.0], 8.0, fl);
                        }
                    }
                } else {
                    let lits: Vec<bool> = (0..bays).map(|_| chance(&mut self.r, lit_p)).collect();
                    if lits.iter().any(|l| *l) {
                        for b in 0..bays {
                            if lits[b as usize] {
                                let warm = rf(&mut self.r, 0.5, 1.0);
                                emit(&mut self.g.mesh, b, b + 1, lit, [255, (220.0 * warm + 30.0) as u8, (160.0 * warm) as u8, 170]);
                            } else {
                                emit(&mut self.g.mesh, b, b + 1, win, wall_col);
                            }
                        }
                    } else {
                        emit(&mut self.g.mesh, 0, bays, win, wall_col);
                    }
                }
            }
        }
        // roof + parapet
        let m = &mut self.g.mesh;
        let v = |x: f32, y: f32, z: f32| o + Vec3::new(x, y, z);
        m.quad([v(x0, h, z1), v(x1, h, z1), v(x1, h, z0), v(x0, h, z0)], (x1 - x0) / 4.0, (z1 - z0) / 4.0, t(Tex::Roof), WHITE);
        for (a, b) in [(v(x0, h, z0), v(x1, h + 0.7, z0 + 0.3)), (v(x0, h, z1 - 0.3), v(x1, h + 0.7, z1)), (v(x0, h, z0), v(x0 + 0.3, h + 0.7, z1)), (v(x1 - 0.3, h, z0), v(x1, h + 0.7, z1))] {
            m.boxx(a, b, t(plain), t(Tex::Concrete), wall_col, 3.0, 63 & !8);
        }
        // rooftop kit: AC units, water tank, antenna
        for _ in 0..self.r.u32(1..4) {
            let (px, pz) = (rf(&mut self.r, x0 + 2.0, x1 - 3.0), rf(&mut self.r, z0 + 2.0, z1 - 3.0));
            match self.r.u32(0..3) {
                0 => self.bx(Vec3::new(px, h, pz), Vec3::new(px + 1.6, h + 1.1, pz + 1.2), Tex::Metal, Tex::Metal, WHITE, 0.0),
                1 => {
                    self.bx(Vec3::new(px, h, pz), Vec3::new(px + 0.2, h + 1.6, pz + 0.2), Tex::Rust, Tex::Rust, WHITE, 0.0);
                    self.bx(Vec3::new(px + 1.6, h, pz + 1.6), Vec3::new(px + 1.8, h + 1.6, pz + 1.8), Tex::Rust, Tex::Rust, WHITE, 0.0);
                    self.bx(Vec3::new(px - 0.2, h + 1.6, pz - 0.2), Vec3::new(px + 2.0, h + 3.6, pz + 2.0), Tex::Wood, Tex::Rust, WHITE, 0.0);
                }
                _ => {
                    self.bx(Vec3::new(px, h, pz), Vec3::new(px + 0.12, h + 6.0, pz + 0.12), Tex::Metal, Tex::Metal, WHITE, 0.0);
                    if floors > 6 {
                        self.light(px, h + 6.2, pz, [2.0, 0.1, 0.1], 7.0, 4);
                    }
                }
            }
        }
        if lost {
            // growth crown bursting through the roof
            let (px, pz) = ((x0 + x1) * 0.5, (z0 + z1) * 0.5);
            for i in 0..3 {
                let s = rf(&mut self.r, 1.5, 3.5);
                let (ox, oz) = (rf(&mut self.r, -4.0, 4.0), rf(&mut self.r, -4.0, 4.0));
                let pos = self.p(px + ox, h, pz + oz);
                self.g.anims.push(AnimProp { kind: if i == 0 { AnimKind::Pod } else { AnimKind::Tendril }, pos, yaw: i as f32, size: s });
            }
        }
        // collision + interior
        if open {
            let wt = 0.4;
            let top = Aabb::new(o + Vec3::new(x0, KERB + FLOOR_H, z0), o + Vec3::new(x1, h, z1));
            self.g.cols.push(top);
            let (start, dir, len, _) = sides[door_side];
            let bays = (len / 3.2).round().max(1.0);
            let bw = len / bays;
            let door_bay = (bays as u32) / 2;
            let (g0, g1) = ((door_bay as f32 - 1.0).max(0.0) * bw, (door_bay as f32 + 1.0) * bw);
            for (si, (s, d, l, _)) in sides.iter().enumerate() {
                let n = Vec3::new(-d.z, 0.0, d.x);
                let seg = |a: f32, b: f32, cols: &mut Vec<Aabb>| {
                    let p0 = o + *s + *d * a;
                    let p1 = o + *s + *d * b + n * wt;
                    cols.push(Aabb::new(p0.min(p1), p0.max(p1) + Vec3::Y * (KERB + FLOOR_H)));
                };
                if si == door_side {
                    seg(0.0, g0, &mut self.g.cols);
                    seg(g1, *l, &mut self.g.cols);
                } else {
                    seg(0.0, *l, &mut self.g.cols);
                }
            }
            let _ = (start, dir);
            // inside faces: floor, ceiling, walls
            let (ix0, iz0, ix1, iz1) = (x0 + wt, z0 + wt, x1 - wt, z1 - wt);
            let (fy, cy) = (KERB + 0.02, KERB + FLOOR_H - 0.05);
            let m = &mut self.g.mesh;
            let dim = rgb(0.55, 0.55, 0.55);
            m.quad([v(ix0, fy, iz1), v(ix1, fy, iz1), v(ix1, fy, iz0), v(ix0, fy, iz0)], (ix1 - ix0) / 2.0, (iz1 - iz0) / 2.0, t(Tex::Tile), dim);
            m.quad([v(ix0, cy, iz0), v(ix1, cy, iz0), v(ix1, cy, iz1), v(ix0, cy, iz1)], (ix1 - ix0) / 3.0, (iz1 - iz0) / 3.0, t(Tex::Plaster), dim);
            m.quad([v(ix0, fy, iz0), v(ix1, fy, iz0), v(ix1, cy, iz0), v(ix0, cy, iz0)], (ix1 - ix0) / 3.0, 1.0, t(Tex::Plaster), dim);
            m.quad([v(ix1, fy, iz1), v(ix0, fy, iz1), v(ix0, cy, iz1), v(ix1, cy, iz1)], (ix1 - ix0) / 3.0, 1.0, t(Tex::Plaster), dim);
            m.quad([v(ix0, fy, iz1), v(ix0, fy, iz0), v(ix0, cy, iz0), v(ix0, cy, iz1)], (iz1 - iz0) / 3.0, 1.0, t(Tex::Plaster), dim);
            m.quad([v(ix1, fy, iz0), v(ix1, fy, iz1), v(ix1, cy, iz1), v(ix1, cy, iz0)], (iz1 - iz0) / 3.0, 1.0, t(Tex::Plaster), dim);
            // shelves + counter
            let (mx, mz) = ((x0 + x1) * 0.5, (z0 + z1) * 0.5);
            for i in 0..3 {
                let sx = ix0 + 2.0 + i as f32 * ((ix1 - ix0 - 4.0) / 3.0);
                self.low(Vec3::new(sx, KERB, mz - 3.0), Vec3::new(sx + 0.6, KERB + 1.7, mz + 3.0), Tex::Metal, rgb(0.5, 0.5, 0.55), 2.0);
            }
            self.low(Vec3::new(mx - 2.0, KERB, iz0 + 1.2), Vec3::new(mx + 2.0, KERB + 1.0, iz0 + 2.0), Tex::Wood, WHITE, 2.0);
            let fl = if chance(&mut self.r, 0.5) { 3 } else { 1 };
            self.light(mx, KERB + 2.6, mz, [1.2, 1.1, 0.8], 11.0, fl);
            self.spawn(mx + 3.0, mz + 3.5, SpawnKind::Cache);
            if self.state != DState::Held && chance(&mut self.r, 0.6) {
                self.spawn(mx - 3.0, mz, SpawnKind::Infected);
            } else if chance(&mut self.r, 0.5) {
                self.spawn(mx - 3.0, mz, SpawnKind::Civilians);
            }
            if chance(&mut self.r, 0.5) {
                let n = self.r.usize(0..12);
                let pos = self.p(mx, KERB + 1.0, iz0 + 1.6);
                self.g.interact.push((pos, InteractKind::Note(n)));
            }
        } else {
            self.g.cols.push(Aabb::new(o + Vec3::new(x0, 0.0, z0), o + Vec3::new(x1, h, z1)));
        }
    }

    /// Place a kit model at local (x, z) on the ground with a yaw and scale.
    fn prop(&mut self, name: &'static str, x: f32, z: f32, yaw: f32, scale: f32, tint: [f32; 4]) {
        let y = self.g_h(x, z);
        let m = trs(self.p(x, y, z), Quat::from_rotation_y(yaw), Vec3::splat(scale));
        self.g.props.push((name, m, tint));
    }
    /// A prop with a collision box of half extents (hx, hz) and height h (axis aligned after yaw snapping).
    fn prop_solid(&mut self, name: &'static str, x: f32, z: f32, yaw: f32, hx: f32, hz: f32, h: f32, tint: [f32; 4]) {
        self.prop(name, x, z, yaw, 1.0, tint);
        let swap = ((yaw / std::f32::consts::FRAC_PI_2).round() as i32).rem_euclid(2) == 1;
        let (ex, ez) = if swap { (hz, hx) } else { (hx, hz) };
        self.g.cols.push(Aabb::low(self.o + Vec3::new(x - ex, 0.0, z - ez), self.o + Vec3::new(x + ex, h, z + ez)));
    }

    fn lamp(&mut self, x: f32, z: f32, ax: f32, az: f32) {
        // kit street light: pole at the origin, arm reaching 2.6 m along +z at ~6.4 m
        let yaw = ax.atan2(az);
        self.prop("StreetLights", x, z, yaw, 1.0, [0.8, 0.8, 0.82, 0.0]);
        let (hx, hz) = (x + ax * 2.5, z + az * 2.5);
        self.g.cols.push(Aabb::low(self.o + Vec3::new(x - 0.12, 0.0, z - 0.12), self.o + Vec3::new(x + 0.12, 3.0, z + 0.12)));
        let dead = match self.state {
            DState::Held => 0.05,
            DState::Contested => 0.35,
            DState::Lost => 0.9,
        };
        if !chance(&mut self.r, dead) {
            self.bx(Vec3::new(hx - 0.2, 6.2, hz - 0.2), Vec3::new(hx + 0.2, 6.28, hz + 0.2), Tex::White, Tex::White, rgbe(1.0, 0.7, 0.3, 1.0), 0.0);
            let fl = if chance(&mut self.r, 0.2) { 3 } else { 1 };
            self.light(hx, 5.9, hz, [3.0, 1.75, 0.55], 17.0, fl);
        }
    }

    fn car(&mut self, x: f32, z: f32, along_x: bool, burning: bool) {
        let military = self.state != DState::Lost && chance(&mut self.r, if self.control > 0.6 { 0.35 } else { 0.08 });
        let name = if military { *crate::util::pick(&mut self.r, &["Vehicle_Pickup_Armored", "Vehicle_Truck_Armored"]) } else { *crate::util::pick(&mut self.r, &["Vehicle_Pickup", "Vehicle_Sports", "Vehicle_Truck", "Vehicle_Pickup"]) };
        let yaw = if along_x { std::f32::consts::FRAC_PI_2 } else { 0.0 } + if chance(&mut self.r, 0.5) { std::f32::consts::PI } else { 0.0 } + rf(&mut self.r, -0.12, 0.12);
        let tint = if burning { [0.18, 0.15, 0.14, 0.0] } else if self.state == DState::Lost { [0.6, 0.55, 0.6, 0.0] } else { [rf(&mut self.r, 0.65, 1.0), rf(&mut self.r, 0.6, 0.95), rf(&mut self.r, 0.6, 1.0), 0.0] };
        let y = self.g_h(x, z);
        self.prop_solid(name, x, z, (yaw / std::f32::consts::FRAC_PI_2).round() * std::f32::consts::FRAC_PI_2, 1.25, 2.6, 1.6, tint);
        // keep the visible jitter on the model itself
        if let Some(last) = self.g.props.last_mut() {
            last.1 = trs(self.o + Vec3::new(x, y, z), Quat::from_rotation_y(yaw), Vec3::ONE);
        }
        if burning {
            let p = self.p(x, y + 1.2, z);
            self.g.emitters.push((p, EmitKind::Fire));
            self.g.emitters.push((p + Vec3::Y * 0.8, EmitKind::Smoke));
            self.light(x, y + 2.0, z, [3.5, 1.4, 0.3], 13.0, 2);
        }
    }
    fn g_h(&self, x: f32, z: f32) -> f32 {
        if x > ROAD && x < CS - ROAD && z > ROAD && z < CS - ROAD {
            KERB
        } else {
            0.0
        }
    }

    fn barricade(&mut self, x: f32, z: f32, along_x: bool) {
        let n = 3;
        for i in 0..n {
            let off = (i as f32 - 1.0) * 2.6;
            let (px, pz) = if along_x { (x + off, z) } else { (x, z + off) };
            let (hx, hz) = if along_x { (1.2, 0.35) } else { (0.35, 1.2) };
            let yaw = if along_x { 0.0 } else { std::f32::consts::FRAC_PI_2 };
            match self.r.u32(0..4) {
                0 => {
                    self.prop("TrafficBarrier_1", px - if along_x { 0.8 } else { 0.0 }, pz - if along_x { 0.0 } else { 0.8 }, yaw, 1.0, [1.0; 4]);
                    self.prop("TrafficBarrier_1", px + if along_x { 0.8 } else { 0.0 }, pz + if along_x { 0.0 } else { 0.8 }, yaw, 1.0, [1.0; 4]);
                    self.g.cols.push(Aabb::low(self.o + Vec3::new(px - hx * 1.3, 0.0, pz - hz * 1.2), self.o + Vec3::new(px + hx * 1.3, 1.1, pz + hz * 1.2)));
                }
                1 => {
                    for k in 0..2 {
                        let o = (k as f32 - 0.5) * 1.1;
                        self.prop("PlasticBarrier", px + if along_x { o } else { 0.0 }, pz + if along_x { 0.0 } else { o }, yaw, 1.0, [1.0; 4]);
                    }
                    self.g.cols.push(Aabb::low(self.o + Vec3::new(px - hx, 0.0, pz - hz * 0.6), self.o + Vec3::new(px + hx, 0.7, pz + hz * 0.6)));
                }
                _ => {
                    self.low(Vec3::new(px - hx, 0.0, pz - hz * 1.5), Vec3::new(px + hx, 1.1, pz + hz * 1.5), Tex::Sandbag, WHITE, 1.2);
                }
            }
            if chance(&mut self.r, 0.4) {
                let c = *crate::util::pick(&mut self.r, &["TrafficCone_1", "TrafficCone_2"]);
                let y2 = rf(&mut self.r, 0.0, 6.28);
                let (ox, oz) = (rf(&mut self.r, -1.5, 1.5), rf(&mut self.r, 1.0, 1.8));
                self.prop(c, px + ox, pz + oz, y2, 1.0, [1.0; 4]);
            }
        }
    }

    fn crates(&mut self, x: f32, z: f32) {
        let y = self.g_h(x, z);
        let _ = y;
        for _ in 0..self.r.u32(1..4) {
            let (px, pz) = (x + rf(&mut self.r, -1.2, 1.2), z + rf(&mut self.r, -1.2, 1.2));
            let yaw = rf(&mut self.r, 0.0, 6.28);
            match self.r.u32(0..6) {
                0 => self.prop_solid("Barrel", px, pz, 0.0, 0.35, 0.35, 1.15, [1.0; 4]),
                1 => {
                    for k in 0..self.r.u32(1..4) {
                        let m = trs(self.p(px, self.g_h(px, pz) + k as f32 * 0.14, pz), Quat::from_rotation_y(yaw + k as f32 * 0.3), Vec3::ONE);
                        self.g.props.push(("Pallet", m, [1.0; 4]));
                    }
                }
                2 => self.prop_solid("Chest", px, pz, yaw, 0.32, 0.32, 0.41, [1.0; 4]),
                3 => self.prop("Wheels_Stack", px, pz, yaw, 1.0, [1.0; 4]),
                4 => {
                    for k in 0..4 {
                        self.prop("CinderBlock", px + (k % 2) as f32 * 0.5, pz + (k / 2) as f32 * 0.25, 0.0, 1.0, [1.0; 4]);
                    }
                }
                _ => self.prop("Pallet_Broken", px, pz, yaw, 1.0, [1.0; 4]),
            }
        }
    }
    /// Alley rubbish: a skip made from a container, bags and a barrel.
    fn dumpster(&mut self, x: f32, z: f32) {
        let name = *crate::util::pick(&mut self.r, &["Container_Green", "Container_Red"]);
        let m = trs(self.p(x, KERB, z), Quat::IDENTITY, Vec3::new(0.42, 0.5, 0.55));
        self.g.props.push((name, m, [0.85, 0.85, 0.85, 0.0]));
        self.g.cols.push(Aabb::low(self.o + Vec3::new(x - 1.2, 0.0, z - 0.7), self.o + Vec3::new(x + 1.2, 1.4, z + 0.7)));
        for k in 0..self.r.u32(1..4) {
            let b = *crate::util::pick(&mut self.r, &["TrashBag_1", "TrashBag_2"]);
            let yaw = rf(&mut self.r, 0.0, 6.28);
            let oz = rf(&mut self.r, -0.5, 0.5);
            self.prop(b, x - 1.6 + k as f32 * 0.6, z + oz, yaw, 1.0, [1.0; 4]);
        }
    }
    fn fence(&mut self, x0: f32, z0: f32, x1: f32, z1: f32) {
        let (a, b) = (self.p(x0, KERB, z0), self.p(x1, KERB, z1));
        let up = Vec3::Y * 2.4;
        let len = (b - a).length();
        for (p, q) in [(a, b), (b, a)] {
            self.g.mesh.quad([p, q, q + up, p + up], len / 1.2, 2.0, t(Tex::Fence), WHITE);
        }
        self.g.cols.push(Aabb { min: a.min(b) - Vec3::new(0.08, 0.0, 0.08), max: a.max(b) + Vec3::new(0.08, 2.4, 0.08), flags: crate::physics::SOLID });
    }
    fn growth(&mut self, x: f32, z: f32, s: f32) {
        let y = self.g_h(x, z);
        let pos = self.p(x, y, z);
        self.g.anims.push(AnimProp { kind: AnimKind::Tendril, pos, yaw: x * 3.1, size: s });
        if chance(&mut self.r, 0.5) {
            self.g.anims.push(AnimProp { kind: AnimKind::Pod, pos: pos + Vec3::new(0.8, 0.0, 0.3), yaw: z, size: s * 0.6 });
            self.light(x + 0.8, y + s * 0.5, z + 0.3, [0.3, 1.3, 0.8], 7.0 + s, 4);
            self.g.emitters.push((pos + Vec3::Y * s * 0.5, EmitKind::Spores));
        }
    }
    fn burn_pit(&mut self, x: f32, z: f32, id: usize) {
        for i in 0..8 {
            let a = i as f32 / 8.0 * std::f32::consts::TAU;
            let (px, pz) = (x + a.cos() * 2.6, z + a.sin() * 2.6);
            self.bx(Vec3::new(px - 0.5, KERB, pz - 0.5), Vec3::new(px + 0.5, KERB + 0.45, pz + 0.5), Tex::Sandbag, Tex::Sandbag, WHITE, 1.0);
        }
        self.low(Vec3::new(x - 1.1, KERB, z - 1.1), Vec3::new(x + 1.1, KERB + 0.7, z + 1.1), Tex::Char, WHITE, 1.0);
        self.bx(Vec3::new(x - 0.8, KERB + 0.7, z - 0.8), Vec3::new(x + 0.8, KERB + 0.95, z + 0.8), Tex::Ember, Tex::Ember, rgbe(1.0, 0.7, 0.4, 1.0), 1.0);
        let p = self.p(x, KERB + 1.0, z);
        self.g.emitters.push((p, EmitKind::Fire));
        self.g.emitters.push((p, EmitKind::Embers));
        self.g.emitters.push((p + Vec3::Y * 1.5, EmitKind::Smoke));
        self.light(x, KERB + 2.0, z, [4.0, 1.8, 0.5], 20.0, 2);
        self.g.interact.push((self.p(x, KERB, z), InteractKind::BurnPit(id)));
        // hazard tape poles and a flag
        self.bx(Vec3::new(x + 3.4, KERB, z + 3.4), Vec3::new(x + 3.5, KERB + 4.5, z + 3.5), Tex::Metal, Tex::Metal, WHITE, 0.0);
        let fp = self.p(x + 3.45, KERB + 4.4, z + 3.45);
        self.g.anims.push(AnimProp { kind: AnimKind::Flag, pos: fp, yaw: 0.0, size: 1.0 });
    }
    fn tent(&mut self, x: f32, z: f32) {
        self.solid(Vec3::new(x - 3.0, KERB, z - 2.2), Vec3::new(x + 3.0, KERB + 2.4, z + 2.2), Tex::Tarp, Tex::Tarp, WHITE, 2.0);
        let o = self.o;
        let m = &mut self.g.mesh;
        let (a, b, c, d) = (o + Vec3::new(x - 3.2, KERB + 2.4, z - 2.4), o + Vec3::new(x + 3.2, KERB + 2.4, z - 2.4), o + Vec3::new(x + 3.2, KERB + 2.4, z + 2.4), o + Vec3::new(x - 3.2, KERB + 2.4, z + 2.4));
        let (r0, r1) = (o + Vec3::new(x - 3.2, KERB + 3.4, z), o + Vec3::new(x + 3.2, KERB + 3.4, z));
        m.quad([b, a, r0, r1], 3.0, 1.0, t(Tex::Tarp), WHITE);
        m.quad([d, c, r1, r0], 3.0, 1.0, t(Tex::Tarp), WHITE);
        m.tri([a, d, r0], t(Tex::Tarp), WHITE);
        m.tri([c, b, r1], t(Tex::Tarp), WHITE);
    }
    fn floodlight(&mut self, x: f32, z: f32, tx: f32, tz: f32) {
        self.low(Vec3::new(x - 0.2, KERB, z - 0.2), Vec3::new(x + 0.2, KERB + 7.0, z + 0.2), Tex::Metal, rgb(0.4, 0.4, 0.4), 0.0);
        self.bx(Vec3::new(x - 0.7, KERB + 7.0, z - 0.7), Vec3::new(x + 0.7, KERB + 7.6, z + 0.7), Tex::White, Tex::White, rgbe(0.9, 0.95, 1.0, 1.0), 0.0);
        self.light(x + (tx - x) * 0.35, 5.5, z + (tz - z) * 0.35, [2.6, 2.8, 3.2], 26.0, 0);
    }
    fn subway(&mut self, x: f32, z: f32) {
        // stairwell box, railings and a lit sign
        self.bx(Vec3::new(x - 1.6, KERB - 0.1, z - 3.0), Vec3::new(x + 1.6, KERB + 0.02, z + 3.0), Tex::Char, Tex::Tile, rgb(0.25, 0.25, 0.25), 1.0);
        for sx in [-1.7f32, 1.6] {
            self.low(Vec3::new(x + sx, KERB, z - 3.0), Vec3::new(x + sx + 0.1, KERB + 1.0, z + 3.0), Tex::Metal, rgb(0.3, 0.35, 0.3), 1.0);
        }
        self.low(Vec3::new(x - 1.7, KERB, z + 2.9), Vec3::new(x + 1.7, KERB + 1.0, z + 3.0), Tex::Metal, rgb(0.3, 0.35, 0.3), 1.0);
        for i in 0..5 {
            let zz = z - 2.6 + i as f32 * 1.0;
            self.bx(Vec3::new(x - 1.5, KERB - 0.05 + i as f32 * 0.0, zz), Vec3::new(x + 1.5, KERB + 0.04, zz + 0.5), Tex::Concrete, Tex::Concrete, rgb(0.3 - i as f32 * 0.05, 0.3 - i as f32 * 0.05, 0.3 - i as f32 * 0.05), 1.0);
        }
        self.bx(Vec3::new(x - 1.8, KERB + 2.6, z - 3.2), Vec3::new(x + 1.8, KERB + 3.2, z - 3.0), Tex::Sign, Tex::Metal, rgbe(1.0, 1.0, 1.0, 0.7), 0.0);
        self.low(Vec3::new(x - 1.8, KERB, z - 3.2), Vec3::new(x - 1.65, KERB + 2.6, z - 3.05), Tex::Metal, WHITE, 0.0);
        self.low(Vec3::new(x + 1.65, KERB, z - 3.2), Vec3::new(x + 1.8, KERB + 2.6, z - 3.05), Tex::Metal, WHITE, 0.0);
        let st = self.state;
        self.light(x, KERB + 2.2, z - 2.0, if st == DState::Lost { [0.4, 1.0, 0.6] } else { [0.8, 1.0, 1.3] }, 9.0, 3);
        self.g.emitters.push((self.p(x, KERB, z + 1.0), EmitKind::Steam));
    }

    /// Fill the block interior according to a layout variant.
    fn block(&mut self) {
        let (a, b) = (ROAD + 3.0, CS - ROAD - 3.0);
        let floors_rng = match self.d {
            0 => (3, 7),
            1 => (2, 6),
            2 => (6, 14),
            3 => (2, 5),
            4 => (3, 10),
            _ => (2, 9),
        };
        let fl = |r: &mut Rng| r.u32(floors_rng.0..=floors_rng.1);
        let open_p = 0.3;
        let layout = self.r.u32(0..10);
        let mid = (a + b) * 0.5;
        let alley = 2.2;
        match layout {
            0 | 1 => {
                let f = fl(&mut self.r);
                let o = chance(&mut self.r, open_p);
                self.building(a + 2.0, a + 2.0, b - 2.0, b - 2.0, f, o);
            }
            2..=4 => {
                let (f0, f1) = (fl(&mut self.r), fl(&mut self.r));
                let (o0, o1) = (chance(&mut self.r, open_p), chance(&mut self.r, open_p));
                if chance(&mut self.r, 0.5) {
                    self.building(a, a, mid - alley, b, f0, o0);
                    self.building(mid + alley, a, b, b, f1, o1);
                    self.alley(mid, a + 3.0, mid, b - 3.0);
                } else {
                    self.building(a, a, b, mid - alley, f0, o0);
                    self.building(a, mid + alley, b, b, f1, o1);
                    self.alley(a + 3.0, mid, b - 3.0, mid);
                }
            }
            5..=7 => {
                for (qx, qz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let (x0, x1) = if qx == 0 { (a, mid - alley) } else { (mid + alley, b) };
                    let (z0, z1) = if qz == 0 { (a, mid - alley) } else { (mid + alley, b) };
                    if chance(&mut self.r, 0.14) {
                        self.yard(x0, z0, x1, z1);
                    } else {
                        let f = fl(&mut self.r);
                        let o = chance(&mut self.r, open_p * 0.7);
                        self.building(x0, z0, x1, z1, f, o);
                    }
                }
                self.alley(mid, a + 3.0, mid, b - 3.0);
                self.alley(a + 3.0, mid, b - 3.0, mid);
            }
            8 => {
                let f = fl(&mut self.r);
                self.building(a, a, mid - alley, b, f, false);
                self.yard(mid + alley, a, b, b);
            }
            _ => self.plaza(a, b),
        }
    }

    fn alley(&mut self, x0: f32, z0: f32, x1: f32, z1: f32) {
        let (mx, mz) = ((x0 + x1) * 0.5, (z0 + z1) * 0.5);
        let n = self.r.u32(1..4);
        for _ in 0..n {
            let k = self.r.f32();
            let (px, pz) = (x0 + (x1 - x0) * k, z0 + (z1 - z0) * k);
            match self.r.u32(0..3) {
                0 => self.dumpster(px, pz + if x0 == x1 { 0.0 } else { 1.0 }),
                1 => self.crates(px, pz),
                _ => {
                    if self.state == DState::Lost {
                        self.growth(px, pz, 2.0)
                    }
                }
            }
        }
        match self.state {
            DState::Held => {
                if chance(&mut self.r, 0.3) {
                    self.spawn(mx, mz, SpawnKind::Civilians)
                }
            }
            _ => {
                if chance(&mut self.r, 0.55) {
                    {
                        let k = if self.control > 0.62 && chance(&mut self.r, 0.4) { SpawnKind::Civilians } else { SpawnKind::Infected };
                        self.spawn(mx, mz, k)
                    }
                }
            }
        }
        if chance(&mut self.r, 0.3) {
            self.light(mx, 3.0, mz, [1.2, 1.0, 0.7], 8.0, 3);
        }
    }

    fn yard(&mut self, x0: f32, z0: f32, x1: f32, z1: f32) {
        // fenced parking / storage yard
        self.fence(x0, z0, x1, z0);
        self.fence(x0, z1, x1, z1);
        self.fence(x0, z0, x0, z1 - 4.0);
        let o = self.o;
        self.g.mesh.quad([o + Vec3::new(x0, KERB + 0.01, z1), o + Vec3::new(x1, KERB + 0.01, z1), o + Vec3::new(x1, KERB + 0.01, z0), o + Vec3::new(x0, KERB + 0.01, z0)], (x1 - x0) / 4.0, (z1 - z0) / 4.0, t(Tex::Asphalt), WHITE);
        let n = self.r.u32(1..4);
        for i in 0..n {
            let burning = self.state == DState::Contested && chance(&mut self.r, 0.25);
            let oz = rf(&mut self.r, -3.0, 3.0);
            self.car(x0 + 3.0 + i as f32 * 3.4, (z0 + z1) * 0.5 + oz, false, burning);
        }
        self.crates(x1 - 2.5, z1 - 2.5);
        if self.state != DState::Held {
            self.spawn((x0 + x1) * 0.5, (z0 + z1) * 0.5, SpawnKind::Infected);
        }
    }

    fn plaza(&mut self, a: f32, b: f32) {
        let mid = (a + b) * 0.5;
        let o = self.o;
        let lost = self.state == DState::Lost;
        self.g.mesh.quad([o + Vec3::new(a, KERB + 0.01, b), o + Vec3::new(b, KERB + 0.01, b), o + Vec3::new(b, KERB + 0.01, a), o + Vec3::new(a, KERB + 0.01, a)], (b - a) / 3.0, (b - a) / 3.0, t(if lost { Tex::Flesh } else { Tex::Grass }), if lost { rgb(0.6, 0.5, 0.55) } else { WHITE });
        // statue / fountain plinth
        self.solid(Vec3::new(mid - 1.5, KERB, mid - 1.5), Vec3::new(mid + 1.5, KERB + 1.2, mid + 1.5), Tex::Concrete, Tex::Concrete, WHITE, 2.0);
        self.bx(Vec3::new(mid - 0.5, KERB + 1.2, mid - 0.5), Vec3::new(mid + 0.5, KERB + 4.5, mid + 0.5), if lost { Tex::Growth } else { Tex::Rust }, Tex::Rust, WHITE, 2.0);
        // dead trees and benches
        for _ in 0..6 {
            let (x, z) = (rf(&mut self.r, a + 2.0, b - 2.0), rf(&mut self.r, a + 2.0, b - 2.0));
            if (x - mid).abs() < 4.0 && (z - mid).abs() < 4.0 {
                continue;
            }
            if lost {
                let s = rf(&mut self.r, 2.0, 5.0);
                self.growth(x, z, s);
            } else {
                let h = rf(&mut self.r, 3.0, 5.5);
                self.low(Vec3::new(x - 0.15, KERB, z - 0.15), Vec3::new(x + 0.15, KERB + h, z + 0.15), Tex::Wood, rgb(0.4, 0.35, 0.3), 1.0);
                let m = trs(self.p(x, KERB + h * 0.75, z), Quat::from_rotation_z(0.7), Vec3::new(0.08, h * 0.5, 0.08));
                self.g.mesh.obox(m, t(Tex::Wood), rgb(0.4, 0.35, 0.3));
                let m = trs(self.p(x, KERB + h * 0.6, z), Quat::from_rotation_x(-0.8), Vec3::new(0.07, h * 0.4, 0.07));
                self.g.mesh.obox(m, t(Tex::Wood), rgb(0.4, 0.35, 0.3));
            }
        }
        self.subway(a + 4.0, a + 6.0);
        self.spawn(mid + 6.0, mid - 6.0, if self.state == DState::Held { SpawnKind::Civilians } else { SpawnKind::Infected });
        if self.state != DState::Held {
            self.spawn(mid - 7.0, mid + 7.0, SpawnKind::Infected);
        }
        if chance(&mut self.r, 0.6) {
            let n = self.r.usize(0..12);
            self.g.interact.push((self.p(mid + 2.2, KERB + 0.6, mid), InteractKind::Note(n)));
        }
    }

    fn streets(&mut self) {
        // lamps on two opposite corners
        self.lamp(ROAD + 0.8, ROAD + 0.8, -1.0, 0.0);
        self.lamp(CS - ROAD - 0.8, CS - ROAD - 0.8, 1.0, 0.0);
        if chance(&mut self.r, 0.5) {
            self.lamp(CS - ROAD - 0.8, ROAD + 0.8, 0.0, -1.0);
        }
        // kerbside dressing: hydrants, rubbish, a traffic light on the corner
        if chance(&mut self.r, 0.6) {
            let x = rf(&mut self.r, ROAD + 4.0, CS - ROAD - 4.0);
            self.prop_solid("FireHydrant", x, ROAD + 0.6, 0.0, 0.25, 0.2, 0.8, [1.0; 4]);
        }
        for _ in 0..self.r.u32(0..4) {
            let (x, z) = if chance(&mut self.r, 0.5) { (rf(&mut self.r, ROAD + 1.0, CS - ROAD - 1.0), ROAD + 0.5) } else { (ROAD + 0.5, rf(&mut self.r, ROAD + 1.0, CS - ROAD - 1.0)) };
            let b = *crate::util::pick(&mut self.r, &["TrashBag_1", "TrashBag_2", "TrashBag_1"]);
            let yaw = rf(&mut self.r, 0.0, 6.28);
            self.prop(b, x, z, yaw, 1.0, [1.0; 4]);
        }
        if chance(&mut self.r, 0.35) {
            self.prop_solid("TrafficLight_1", ROAD + 0.5, CS - ROAD - 0.5, std::f32::consts::FRAC_PI_2, 0.2, 0.2, 4.6, [0.85, 0.85, 0.85, 0.0]);
        }
        if self.state != DState::Held && chance(&mut self.r, 0.3) {
            for _ in 0..self.r.u32(2..5) {
                let c = *crate::util::pick(&mut self.r, &["TrafficCone_1", "TrafficCone_2"]);
                let (x, z, y) = (rf(&mut self.r, 2.0, ROAD - 1.0), rf(&mut self.r, ROAD, CS - ROAD), rf(&mut self.r, 0.0, 6.28));
                self.prop(c, x, z, y, 1.0, [1.0; 4]);
            }
        }
        // parked / wrecked cars along the south and west roads
        let ncar = self.r.u32(0..4);
        for _ in 0..ncar {
            let burning = self.state == DState::Contested && chance(&mut self.r, 0.3);
            let k = rf(&mut self.r, ROAD + 4.0, CS - ROAD - 4.0);
            if chance(&mut self.r, 0.5) {
                let o = rf(&mut self.r, 3.6, 5.2);
                self.car(k, o, true, burning);
            } else {
                let o = rf(&mut self.r, 3.6, 5.2);
                self.car(o, k, false, burning);
            }
        }
        match self.state {
            DState::Held => {
                if chance(&mut self.r, 0.35) {
                    self.spawn(3.0, CS * 0.5, SpawnKind::Soldiers);
                }
                if chance(&mut self.r, 0.25) {
                    self.spawn(CS * 0.5, CS - 3.0, SpawnKind::Civilians);
                }
                if chance(&mut self.r, 0.3) {
                    self.barricade(CS * 0.5, 1.0, true);
                }
                // stragglers leak in at the district edge
                let (lx, lz) = (self.cx.rem_euclid(DSIZE), self.cz.rem_euclid(DSIZE));
                if (lx == 0 || lx == DSIZE - 1 || lz == DSIZE - 1) && chance(&mut self.r, 0.6) {
                    self.spawn(CS - 4.0, CS * 0.3, SpawnKind::Infected);
                }
            }
            DState::Contested => {
                let mil = self.control > 0.6;
                let front = (self.control - 0.6).abs() < 0.12;
                if mil || front {
                    self.barricade(CS * 0.5, 2.0, true);
                    self.barricade(2.0, CS * 0.5, false);
                    self.spawn(4.0, CS * 0.5 + 5.0, SpawnKind::Soldiers);
                    self.floodlight(ROAD + 1.0, CS - ROAD - 1.0, 0.0, CS);
                    // flare
                    let p = self.p(3.0, 0.1, CS * 0.3);
                    self.g.emitters.push((p, EmitKind::Flare));
                    self.light(3.0, 0.6, CS * 0.3, [3.0, 0.3, 0.2], 10.0, 2);
                }
                if !mil || front {
                    self.spawn(CS * 0.5, 3.0, SpawnKind::Infected);
                    if chance(&mut self.r, 0.6) {
                        self.spawn(3.0, CS * 0.75, SpawnKind::Infected);
                    }
                    if chance(&mut self.r, 0.35) {
                        self.spawn(CS - 3.0, CS * 0.4, SpawnKind::Infected);
                    }
                }
                if chance(&mut self.r, 0.25) {
                    let x = rf(&mut self.r, 8.0, 48.0);
                    let p = self.p(x, 0.0, 2.0);
                    self.g.emitters.push((p, EmitKind::Smoke));
                }
            }
            DState::Lost => {
                for _ in 0..3 {
                    let (x, z) = if chance(&mut self.r, 0.5) { (rf(&mut self.r, 1.0, CS - 1.0), rf(&mut self.r, 0.5, 6.0)) } else { (rf(&mut self.r, 0.5, 6.0), rf(&mut self.r, 1.0, CS - 1.0)) };
                    let s = rf(&mut self.r, 1.5, 4.5);
                    self.growth(x, z, s);
                }
                self.spawn(CS * 0.5, 3.0, SpawnKind::Infected);
                self.spawn(3.0, CS * 0.6, SpawnKind::Infected);
                if chance(&mut self.r, 0.7) {
                    self.spawn(CS - 3.0, CS * 0.3, SpawnKind::Infected);
                }
                if chance(&mut self.r, 0.5) {
                    self.spawn(CS * 0.25, CS - 3.5, SpawnKind::Infected);
                }
                // abandoned checkpoint
                if chance(&mut self.r, 0.2) {
                    self.barricade(CS * 0.5, 2.0, true);
                    self.crates(CS * 0.5 + 5.0, 4.0);
                    let n = self.r.usize(0..12);
                    self.g.interact.push((self.p(CS * 0.5 + 3.0, 0.8, 4.5), InteractKind::Note(n)));
                }
            }
        }
        // litter and a poster board
        if chance(&mut self.r, 0.5) {
            let (x, z) = (rf(&mut self.r, 8.0, 48.0), rf(&mut self.r, 1.0, 6.0));
            let p = self.p(x, 0.1, z);
            self.g.anims.push(AnimProp { kind: AnimKind::Litter, pos: p, yaw: 0.0, size: 1.0 });
        }
        if chance(&mut self.r, 0.4) {
            let x = rf(&mut self.r, ROAD + 6.0, CS - ROAD - 8.0);
            self.low(Vec3::new(x, KERB, ROAD + 0.4), Vec3::new(x + 2.0, KERB + 2.6, ROAD + 0.55), Tex::Poster, WHITE, 0.0);
        }
    }

    // ---- special blocks ---------------------------------------------------------------
    fn fob(&mut self) {
        let mid = CS * 0.5;
        let o = self.o;
        self.g.mesh.quad([o + Vec3::new(ROAD, KERB + 0.01, CS - ROAD), o + Vec3::new(CS - ROAD, KERB + 0.01, CS - ROAD), o + Vec3::new(CS - ROAD, KERB + 0.01, ROAD), o + Vec3::new(ROAD, KERB + 0.01, ROAD)], 10.0, 10.0, t(Tex::Concrete), rgb(0.7, 0.7, 0.7));
        self.burn_pit(mid, mid, 0);
        self.tent(mid - 12.0, mid - 12.0);
        self.tent(mid + 12.0, mid - 12.0);
        self.tent(mid - 12.0, mid + 13.0);
        // vendor stall (quartermaster) and stash lockers
        self.low(Vec3::new(mid + 9.0, KERB, mid + 10.0), Vec3::new(mid + 15.0, KERB + 1.0, mid + 11.0), Tex::Crate, WHITE, 1.0);
        self.solid(Vec3::new(mid + 9.0, KERB, mid + 13.5), Vec3::new(mid + 15.0, KERB + 2.6, mid + 15.0), Tex::Metal, Tex::Metal, rgb(0.5, 0.55, 0.5), 1.5);
        self.g.interact.push((self.p(mid + 12.0, KERB, mid + 9.2), InteractKind::Vendor));
        self.light(mid + 12.0, 3.0, mid + 10.0, [1.6, 1.4, 1.0], 9.0, 1);
        self.solid(Vec3::new(mid - 16.0, KERB, mid + 2.0), Vec3::new(mid - 14.8, KERB + 2.1, mid + 6.0), Tex::Metal, Tex::Metal, rgb(0.35, 0.4, 0.5), 1.0);
        self.g.interact.push((self.p(mid - 14.0, KERB, mid + 4.0), InteractKind::Stash));
        self.light(mid - 13.0, 3.0, mid + 4.0, [0.9, 1.2, 1.6], 8.0, 0);
        for (x, z) in [(ROAD + 2.0, ROAD + 2.0), (CS - ROAD - 2.0, ROAD + 2.0), (ROAD + 2.0, CS - ROAD - 2.0), (CS - ROAD - 2.0, CS - ROAD - 2.0)] {
            self.floodlight(x, z, mid, mid);
        }
        self.fence(ROAD + 0.5, ROAD + 0.5, mid - 5.0, ROAD + 0.5);
        self.fence(mid + 5.0, ROAD + 0.5, CS - ROAD - 0.5, ROAD + 0.5);
        self.fence(ROAD + 0.5, CS - ROAD - 0.5, mid - 5.0, CS - ROAD - 0.5);
        self.fence(mid + 5.0, CS - ROAD - 0.5, CS - ROAD - 0.5, CS - ROAD - 0.5);
        self.crates(mid - 6.0, mid - 9.0);
        self.crates(mid + 4.0, mid + 14.0);
        self.spawn(mid - 7.0, mid + 3.0, SpawnKind::Soldiers);
        self.spawn(mid + 6.0, mid - 6.0, SpawnKind::Soldiers);
        self.spawn(mid - 9.0, mid - 7.0, SpawnKind::Civilians);
        self.g.interact.push((self.p(mid - 10.0, KERB + 0.8, mid - 9.0), InteractKind::Note(1)));
        self.lamp(ROAD + 0.8, mid, -1.0, 0.0);
    }

    fn pit_block(&mut self, id: usize) {
        let mid = CS * 0.5;
        self.burn_pit(mid, mid, id);
        let (a, b) = (ROAD + 3.0, CS - ROAD - 3.0);
        let f = self.r.u32(2..5);
        self.building(a, a, mid - 7.0, mid - 7.0, f, false);
        let f = self.r.u32(2..6);
        self.building(mid + 7.0, mid + 7.0, b, b, f, true);
        self.barricade(mid - 6.0, mid + 6.0, true);
        self.crates(mid + 5.0, mid - 6.0);
        if self.state != DState::Lost {
            self.floodlight(mid + 6.0, mid - 8.0, mid, mid);
            self.spawn(mid - 5.0, mid + 3.0, SpawnKind::Soldiers);
        } else {
            self.growth(mid - 8.0, mid + 8.0, 3.0);
        }
    }

    fn landmark(&mut self) {
        let mid = CS * 0.5;
        let (a, b) = (ROAD + 3.0, CS - ROAD - 3.0);
        match self.d {
            0 => {
                // stadium: ring of stands with floodlight towers
                for (x0, z0, x1, z1) in [(a, a, b, a + 6.0), (a, b - 6.0, b, b), (a, a + 6.0, a + 6.0, b - 6.0), (b - 6.0, a + 6.0, b, mid - 4.0), (b - 6.0, mid + 4.0, b, b - 6.0)] {
                    self.solid(Vec3::new(x0, KERB, z0), Vec3::new(x1, KERB + 11.0, z1), Tex::Concrete, Tex::Metal, WHITE, 3.5);
                }
                let o = self.o;
                self.g.mesh.quad([o + Vec3::new(a + 6.0, KERB + 0.01, b - 6.0), o + Vec3::new(b - 6.0, KERB + 0.01, b - 6.0), o + Vec3::new(b - 6.0, KERB + 0.01, a + 6.0), o + Vec3::new(a + 6.0, KERB + 0.01, a + 6.0)], 6.0, 6.0, t(Tex::Grass), WHITE);
                for (x, z) in [(a + 3.0, a + 3.0), (b - 3.0, a + 3.0), (a + 3.0, b - 3.0), (b - 3.0, b - 3.0)] {
                    self.bx(Vec3::new(x - 0.3, KERB + 11.0, z - 0.3), Vec3::new(x + 0.3, KERB + 22.0, z + 0.3), Tex::Metal, Tex::Metal, WHITE, 0.0);
                    self.bx(Vec3::new(x - 1.5, KERB + 22.0, z - 1.5), Vec3::new(x + 1.5, KERB + 23.5, z + 1.5), Tex::White, Tex::White, rgbe(0.9, 0.95, 1.0, 1.0), 0.0);
                    self.light(x + (mid - x) * 0.5, 12.0, z + (mid - z) * 0.5, [2.4, 2.6, 3.0], 30.0, 0);
                }
                self.tent(mid - 6.0, mid);
                self.tent(mid + 2.0, mid - 8.0);
                self.spawn(mid, mid + 6.0, SpawnKind::Civilians);
                self.spawn(mid + 6.0, mid + 2.0, SpawnKind::Soldiers);
            }
            1 => {
                // clock tower on a market square
                self.solid(Vec3::new(mid - 4.0, KERB, mid - 4.0), Vec3::new(mid + 4.0, KERB + 34.0, mid + 4.0), Tex::Brick, Tex::Roof, WHITE, 3.2);
                self.bx(Vec3::new(mid - 4.6, KERB + 34.0, mid - 4.6), Vec3::new(mid + 4.6, KERB + 36.0, mid + 4.6), Tex::Concrete, Tex::Roof, WHITE, 3.0);
                for (dx, dz) in [(0.0, 4.05f32), (0.0, -4.05), (4.05, 0.0), (-4.05, 0.0)] {
                    let (hx, hz) = if dx == 0.0 { (2.4, 0.05) } else { (0.05, 2.4) };
                    self.bx(Vec3::new(mid + dx - hx, KERB + 26.0, mid + dz - hz), Vec3::new(mid + dx + hx, KERB + 30.8, mid + dz + hz), Tex::White, Tex::White, rgbe(1.0, 0.9, 0.6, 0.8), 0.0);
                }
                self.light(mid, 28.0, mid + 7.0, [2.2, 1.9, 1.2], 26.0, 4);
                for i in 0..6 {
                    let a2 = i as f32 / 6.0 * std::f32::consts::TAU;
                    let (x, z) = (mid + a2.cos() * 14.0, mid + a2.sin() * 14.0);
                    self.low(Vec3::new(x - 1.6, KERB, z - 1.0), Vec3::new(x + 1.6, KERB + 1.0, z + 1.0), Tex::Crate, WHITE, 1.0);
                    self.bx(Vec3::new(x - 1.9, KERB + 2.2, z - 1.3), Vec3::new(x + 1.9, KERB + 2.35, z + 1.3), Tex::Tarp, Tex::Tarp, if i % 2 == 0 { rgb(0.9, 0.4, 0.3) } else { rgb(0.9, 0.85, 0.6) }, 1.0);
                }
                self.spawn(mid + 9.0, mid + 9.0, SpawnKind::Apex(0));
                self.spawn(mid - 9.0, mid - 9.0, SpawnKind::Infected);
            }
            2 => {
                // the spire: a tower block far taller than anything else
                self.building(a + 8.0, a + 8.0, b - 8.0, b - 8.0, 30, false);
                self.building(a, a, a + 7.0, b, 4, false);
                self.light(mid, 99.0, mid, [2.5, 0.2, 0.2], 30.0, 4);
                self.spawn(b - 4.0, mid, SpawnKind::Apex(2));
                self.spawn(mid, b - 4.0, SpawnKind::Infected);
            }
            3 => {
                // rail depot: sheds, tracks and a water tower
                self.solid(Vec3::new(a, KERB, a), Vec3::new(a + 14.0, KERB + 8.0, b), Tex::Metal, Tex::Rust, rgb(0.6, 0.55, 0.5), 3.0);
                for i in 0..3 {
                    let x = mid + i as f32 * 4.0;
                    self.bx(Vec3::new(x, KERB, a), Vec3::new(x + 0.15, KERB + 0.15, b), Tex::Rust, Tex::Metal, WHITE, 2.0);
                    self.bx(Vec3::new(x + 1.4, KERB, a), Vec3::new(x + 1.55, KERB + 0.15, b), Tex::Rust, Tex::Metal, WHITE, 2.0);
                }
                self.solid(Vec3::new(mid - 0.5, KERB, mid + 4.0), Vec3::new(mid + 3.0, KERB + 3.4, mid + 16.0), Tex::Rust, Tex::Rust, rgb(0.5, 0.3, 0.25), 3.0);
                let (wx, wz) = (b - 5.0, a + 5.0);
                let m = trs(self.p(wx, KERB, wz), Quat::IDENTITY, Vec3::splat(2.1));
                self.g.props.push(("WaterTower", m, [1.0; 4]));
                self.g.cols.push(Aabb::low(self.o + Vec3::new(wx - 2.8, 0.0, wz - 2.8), self.o + Vec3::new(wx + 2.8, 19.0, wz + 2.8)));
                for k in 0..4 {
                    let name = if k % 2 == 0 { "Container_Green" } else { "Container_Red" };
                    let m = trs(self.p(a + 18.0, KERB + if k >= 2 { 2.6 } else { 0.0 }, b - 6.0 - (k % 2) as f32 * 2.7), Quat::IDENTITY, Vec3::ONE);
                    self.g.props.push((name, m, [1.0; 4]));
                }
                self.g.cols.push(Aabb::new(self.o + Vec3::new(a + 15.1, 0.0, b - 10.0), self.o + Vec3::new(a + 20.9, 5.3, b - 4.7)));
                self.light(wx, 12.0, wz, [3.0, 1.6, 0.5], 22.0, 1);
                self.spawn(mid + 6.0, mid - 8.0, SpawnKind::Apex(3));
                self.spawn(mid + 8.0, mid + 10.0, SpawnKind::Infected);
            }
            4 => {
                // the first flower: a colossal growth
                self.solid(Vec3::new(mid - 3.5, KERB, mid - 3.5), Vec3::new(mid + 3.5, KERB + 9.0, mid + 3.5), Tex::Growth, Tex::Growth, WHITE, 3.0);
                self.bx(Vec3::new(mid - 2.5, KERB + 9.0, mid - 2.5), Vec3::new(mid + 2.5, KERB + 20.0, mid + 2.5), Tex::Vein, Tex::Vein, rgbe(0.9, 0.7, 0.8, 0.25), 3.0);
                for i in 0..10 {
                    let a2 = i as f32 / 10.0 * std::f32::consts::TAU;
                    let pos = self.p(mid + a2.cos() * 3.0, KERB + 18.0 + (i % 3) as f32 * 2.0, mid + a2.sin() * 3.0);
                    self.g.anims.push(AnimProp { kind: if i % 2 == 0 { AnimKind::Tendril } else { AnimKind::Pod }, pos, yaw: a2, size: 6.0 });
                    let (x, z) = (mid + a2.cos() * 13.0, mid + a2.sin() * 13.0);
                    self.growth(x, z, 4.0);
                }
                self.light(mid, 22.0, mid, [0.8, 2.4, 1.4], 40.0, 4);
                self.light(mid, 6.0, mid + 6.0, [0.5, 1.6, 1.0], 18.0, 4);
                self.spawn(mid + 10.0, mid - 6.0, SpawnKind::Apex(4));
                self.spawn(mid - 10.0, mid + 4.0, SpawnKind::Apex(5));
            }
            _ => {}
        }
    }

    fn rift_block(&mut self, idx: usize, sealed: bool) {
        let mid = CS * 0.5;
        let o = self.o;
        // scorched ground and the fragment
        self.g.mesh.quad([o + Vec3::new(ROAD, KERB + 0.02, CS - ROAD), o + Vec3::new(CS - ROAD, KERB + 0.02, CS - ROAD), o + Vec3::new(CS - ROAD, KERB + 0.02, ROAD), o + Vec3::new(ROAD, KERB + 0.02, ROAD)], 8.0, 8.0, t(Tex::Char), WHITE);
        let big = idx == 4;
        let n = if big { 14 } else { 8 };
        for i in 0..n {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            let rad = if big { 9.0 } else { 5.0 } + (i % 3) as f32;
            let (x, z) = (mid + a.cos() * rad, mid + a.sin() * rad);
            let h = 2.0 + ((i * 7) % 5) as f32 * if big { 2.0 } else { 0.9 };
            let m = trs(self.p(x, KERB + h * 0.4, z), Quat::from_rotation_y(a) * Quat::from_rotation_z(0.35), Vec3::new(1.0, h, 1.2));
            self.g.mesh.obox(m, t(Tex::RiftStone), WHITE);
            self.g.cols.push(Aabb::new(self.p(x - 0.7, 0.0, z - 0.7), self.p(x + 0.7, h, z + 0.7)));
            let pos = self.p(x, KERB + h + 1.0, z);
            self.g.anims.push(AnimProp { kind: AnimKind::Shard, pos, yaw: a, size: 0.6 });
        }
        let m = trs(self.p(mid, KERB + 0.8, mid), Quat::from_rotation_y(0.6) * Quat::from_rotation_x(0.3), Vec3::splat(if big { 5.0 } else { 2.4 }));
        self.g.mesh.obox(m, t(Tex::RiftGlow), rgbe(1.0, 1.0, 1.0, if sealed { 0.1 } else { 0.8 }));
        let c = if sealed { [0.4, 0.2, 0.6] } else { [1.6, 0.8, 3.0] };
        self.light(mid, 4.0, mid, c, 24.0, 4);
        self.g.emitters.push((self.p(mid, KERB + 2.5, mid), EmitKind::Rift));
        self.g.interact.push((self.p(mid, KERB, mid + if big { 6.5 } else { 3.6 }), InteractKind::RiftPortal(idx)));
        // abandoned survey camp
        self.tent(ROAD + 7.0, ROAD + 6.0);
        self.crates(ROAD + 12.0, ROAD + 5.0);
        self.g.interact.push((self.p(ROAD + 11.0, KERB + 0.8, ROAD + 8.5), InteractKind::Note(8)));
        self.spawn(mid + 10.0, mid + 10.0, SpawnKind::Infected);
        self.spawn(mid - 10.0, mid + 8.0, SpawnKind::Infected);
        if idx == 4 {
            self.spawn(mid + 14.0, mid - 12.0, SpawnKind::Apex(6));
        } else if idx == 0 {
            self.spawn(mid - 14.0, mid - 12.0, SpawnKind::Apex(1));
        }
    }
}

/// Deterministically generate one city chunk.
pub fn gen_chunk(seed: u32, epoch: u32, bosses_dead: &[bool; 5], cx: i32, cz: i32) -> ChunkGen {
    let o = Vec3::new(cx as f32 * CS, 0.0, cz as f32 * CS);
    let Some(d) = district_of(cx, cz) else {
        // outside the quarantine: a blank wall of concrete
        let mut g = ChunkGen::default();
        g.mesh.boxx(o, o + Vec3::new(CS, 16.0, CS), Tex::Concrete as u32, Tex::Roof as u32, rgb(0.6, 0.6, 0.62), 4.0, 63 & !8);
        g.cols.push(Aabb::new(o, o + Vec3::new(CS, 16.0, CS)));
        return g;
    };
    let dist = &DISTRICTS[d];
    let control = noise2(Vec2::new(cx as f32 * 0.45 + epoch as f32 * 0.37, cz as f32 * 0.45 - epoch as f32 * 0.21), seed ^ 0x51ed) * 0.6 + hash01(cx + epoch as i32 * 13, cz, seed) * 0.4;
    let mut gen = Gen { g: ChunkGen::default(), r: seed_rng(seed, cx as u32, cz as u32), o, d, state: dist.state, control, cx, cz };
    gen.ground();
    let local = (cx - dist.grid.0 * DSIZE, cz - dist.grid.1 * DSIZE);
    let pit = dist.pits.iter().position(|p| *p == local);
    if d == 0 && local == (2, 2) {
        gen.fob();
    } else if let Some(pi) = pit {
        gen.pit_block(d * 2 + pi);
    } else if dist.rift == Some(local) {
        let idx = d - 1;
        let sealed = idx == 4 && !bosses_dead[..4].iter().all(|b| *b);
        gen.rift_block(idx, sealed);
    } else if dist.landmark_at == local {
        gen.landmark();
    } else {
        gen.block();
    }
    gen.streets();
    // spawn seeds change every rest so rarities and affixes are re-rolled
    for s in &mut gen.g.spawns {
        s.seed = crate::util::hash_u32(s.seed ^ epoch.wrapping_mul(0x9E37_79B1));
    }
    gen.g
}

pub fn _unused(_: Mat4) {}

/// Low-detail silhouette of a city chunk, drawn when the full chunk is not streamed in.
pub fn far_chunk(seed: u32, cx: i32, cz: i32) -> MeshBuilder {
    let mut m = MeshBuilder::new();
    let o = Vec3::new(cx as f32 * CS, 0.0, cz as f32 * CS);
    let Some(d) = district_of(cx, cz) else {
        m.boxx(o, o + Vec3::new(CS, 16.0, CS), Tex::Concrete as u32, Tex::Roof as u32, rgb(0.6, 0.6, 0.62), 4.0, 63 & !8);
        return m;
    };
    let mut r = seed_rng(seed ^ 0xfa7, cx as u32, cz as u32);
    let state = DISTRICTS[d].state;
    let (lo, hi) = match d {
        2 => (6, 14),
        3 => (2, 5),
        4 => (3, 10),
        _ => (3, 8),
    };
    let (a, b) = (ROAD + 3.0, CS - ROAD - 3.0);
    let mid = (a + b) * 0.5;
    let lit = match state {
        DState::Held => 0.25,
        DState::Contested => 0.08,
        DState::Lost => 0.02,
    };
    for (x0, z0, x1, z1) in [(a, a, mid - 2.0, mid - 2.0), (mid + 2.0, a, b, mid - 2.0), (a, mid + 2.0, mid - 2.0, b), (mid + 2.0, mid + 2.0, b, b)] {
        if chance(&mut r, 0.15) {
            continue;
        }
        let h = r.u32(lo..=hi) as f32 * FLOOR_H;
        let tex = if chance(&mut r, lit) { Tex::ConcreteWindowLit } else { Tex::ConcreteWindow };
        let c = if tex == Tex::ConcreteWindowLit { [255, 220, 170, 160] } else { rgb(0.7, 0.7, 0.72) };
        m.boxx(o + Vec3::new(x0, 0.0, z0), o + Vec3::new(x1, h, z1), tex as u32, Tex::Roof as u32, c, 3.2, 63 & !8);
        if state == DState::Lost && chance(&mut r, 0.5) {
            // growth crowns bending out of the roofs
            let s = rf(&mut r, 3.0, 8.0);
            let mt = trs(o + Vec3::new((x0 + x1) * 0.5, h + s * 0.5, (z0 + z1) * 0.5), Quat::from_rotation_z(rf(&mut r, -0.4, 0.4)), Vec3::new(1.5, s, 1.5));
            m.obox(mt, Tex::Growth as u32, rgbe(0.8, 0.6, 0.8, 0.1));
        }
    }
    m
}
