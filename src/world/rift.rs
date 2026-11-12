//! Rift dimensions: procedurally arranged room graphs grown by a random walk from the entrance to a
//! fixed boss arena. Each rift has its own palette, rules (hazards, lighting, gravity pulses) and
//! enemy mix; the rules themselves are applied by the game loop from `RiftRules`.
use super::*;
use crate::physics::Aabb;
use crate::render::mesh::{rgb, rgbe, trs};
use crate::render::textures::Tex;
use crate::util::{chance, rf, seed_rng, Rng};
use glam::{Quat, Vec3};
use std::collections::HashSet;

pub const RS: f32 = 36.0;
const WALL_H: f32 = 11.0;
const DOOR: f32 = 7.0;

/// Gameplay rules that make each dimension distinct.
#[derive(Clone, Copy, Debug)]
pub struct RiftRules {
    pub name: &'static str,
    pub rule: &'static str,
    pub wall: Tex,
    pub floor: Tex,
    pub accent: Tex,
    pub fog: [f32; 3],
    pub fog_density: f32,
    pub light: [f32; 3],
    pub sun: [f32; 3],
    pub grade: [f32; 3],
    pub levels: f32,
    pub warp: f32,
    /// archetype ids spawned here
    pub mix: &'static [&'static str],
    /// hazard kind laid in rooms: 0 acid, 1 fire, 2 spores, 3 strain pulse
    pub hazard: u8,
    /// pulls everything toward the room centre every few seconds
    pub gravity_pulse: bool,
    /// healing efficiency
    pub heal_mult: f32,
    /// Strain added per second just by being here
    pub strain_tick: f32,
    /// cinder spells cost no Strain
    pub free_cinder: bool,
    /// enemies are hidden in darkness until near
    pub dark: bool,
    pub level: u32,
}

pub const RULES: [RiftRules; 5] = [
    RiftRules {
        name: "THE GUT",
        rule: "THE FLOOR DIGESTS. HEALING IS HALVED.",
        wall: Tex::GutWall,
        floor: Tex::Flesh,
        accent: Tex::Vein,
        fog: [0.16, 0.04, 0.05],
        fog_density: 0.035,
        light: [2.4, 0.6, 0.5],
        sun: [0.25, 0.08, 0.08],
        grade: [1.15, 0.85, 0.85],
        levels: 14.0,
        warp: 0.6,
        mix: &["rusher", "crawler", "spitter", "brute"],
        hazard: 0,
        gravity_pulse: false,
        heal_mult: 0.5,
        strain_tick: 0.0,
        free_cinder: false,
        dark: false,
        level: 12,
    },
    RiftRules {
        name: "THE LATTICE",
        rule: "SPACE CONTRACTS. EVERYTHING IS PULLED TO THE CENTRE.",
        wall: Tex::Lattice,
        floor: Tex::RiftStone,
        accent: Tex::RiftGlow,
        fog: [0.04, 0.05, 0.14],
        fog_density: 0.025,
        light: [1.0, 1.2, 3.0],
        sun: [0.35, 0.4, 0.8],
        grade: [0.85, 0.95, 1.25],
        levels: 12.0,
        warp: 1.0,
        mix: &["leaper", "ambusher", "shielded", "rusher"],
        hazard: 3,
        gravity_pulse: true,
        heal_mult: 1.0,
        strain_tick: 0.0,
        free_cinder: false,
        dark: false,
        level: 18,
    },
    RiftRules {
        name: "THE KILN",
        rule: "THE AIR BURNS. CINDER COSTS NO STRAIN.",
        wall: Tex::Char,
        floor: Tex::Ember,
        accent: Tex::Rust,
        fog: [0.18, 0.07, 0.03],
        fog_density: 0.03,
        light: [3.4, 1.4, 0.4],
        sun: [0.6, 0.25, 0.1],
        grade: [1.2, 0.95, 0.75],
        levels: 14.0,
        warp: 0.4,
        mix: &["rusher", "brute", "spitter", "summoner"],
        hazard: 1,
        gravity_pulse: false,
        heal_mult: 1.0,
        strain_tick: 0.0,
        free_cinder: true,
        dark: false,
        level: 24,
    },
    RiftRules {
        name: "THE CHOIR",
        rule: "DARKNESS. THE SINGING ADDS STRAIN.",
        wall: Tex::Bone,
        floor: Tex::Chitin,
        accent: Tex::GrowthGlow,
        fog: [0.02, 0.02, 0.03],
        fog_density: 0.05,
        light: [0.6, 1.6, 1.2],
        sun: [0.0, 0.0, 0.0],
        grade: [0.9, 1.0, 1.0],
        levels: 10.0,
        warp: 0.8,
        mix: &["ambusher", "leaper", "summoner", "shielded"],
        hazard: 2,
        gravity_pulse: false,
        heal_mult: 1.0,
        strain_tick: 0.6,
        free_cinder: false,
        dark: true,
        level: 32,
    },
    RiftRules {
        name: "GROUND ZERO",
        rule: "THE SEED IS AWAKE. EVERY RULE AT ONCE.",
        wall: Tex::RiftStone,
        floor: Tex::Growth,
        accent: Tex::GrowthGlow,
        fog: [0.04, 0.1, 0.09],
        fog_density: 0.03,
        light: [0.6, 2.4, 1.6],
        sun: [0.2, 0.4, 0.35],
        grade: [0.85, 1.1, 1.0],
        levels: 12.0,
        warp: 1.2,
        mix: &["rusher", "brute", "leaper", "spitter", "ambusher", "summoner", "shielded"],
        hazard: 3,
        gravity_pulse: true,
        heal_mult: 0.75,
        strain_tick: 0.3,
        free_cinder: false,
        dark: false,
        level: 40,
    },
];

fn t(x: Tex) -> u32 {
    x as u32
}

/// Returns room chunks, the start position and the boss arena (centre, radius).
pub fn gen_rift(idx: usize, seed: u32) -> (Vec<((i32, i32), ChunkGen)>, Vec3, (Vec3, f32)) {
    let rules = &RULES[idx];
    let mut r = seed_rng(seed, idx as u32 * 31 + 7, 99);
    // random walk room graph
    let target = if idx == 4 { 7 } else { 5 + r.usize(0..3) };
    let mut rooms: Vec<(i32, i32)> = vec![(0, 0)];
    let mut links: HashSet<((i32, i32), (i32, i32))> = HashSet::new();
    let mut cur = (0, 0);
    let mut guard = 0;
    while rooms.len() < target && guard < 500 {
        guard += 1;
        let dirs = [(1, 0), (0, 1), (-1, 0), (0, 1), (1, 0)];
        let d = dirs[r.usize(0..dirs.len())];
        let n = (cur.0 + d.0, cur.1 + d.1);
        if n.1 < 0 {
            continue;
        }
        links.insert((cur.min(n), cur.max(n)));
        if !rooms.contains(&n) {
            rooms.push(n);
        }
        cur = n;
    }
    // side branches for loot and extra fights
    for _ in 0..2 {
        let base = rooms[r.usize(1..rooms.len())];
        let d = [(1, 0), (-1, 0), (0, -1), (0, 1)][r.usize(0..4)];
        let n = (base.0 + d.0, base.1 + d.1);
        if n.1 >= 0 && !rooms.contains(&n) {
            rooms.push(n);
            links.insert((base.min(n), base.max(n)));
        }
    }
    // boss room: the room farthest (in steps) from the entrance, through the walk order
    let boss_room = cur;
    let mut out = Vec::new();
    let start = Vec3::new(RS * 0.5, 0.0, RS * 0.5 - 6.0);
    let arena = (Vec3::new(boss_room.0 as f32 * RS + RS * 0.5, 0.0, boss_room.1 as f32 * RS + RS * 0.5), RS * 0.5 - 2.5);
    for &room in &rooms {
        let linked = |d: (i32, i32)| {
            let n = (room.0 + d.0, room.1 + d.1);
            links.contains(&(room.min(n), room.max(n)))
        };
        let open = [linked((0, -1)), linked((1, 0)), linked((0, 1)), linked((-1, 0))];
        let mut rr = seed_rng(seed, (room.0 + 100) as u32, (room.1 + 100) as u32);
        let kind = if room == (0, 0) {
            0
        } else if room == boss_room {
            2
        } else {
            1
        };
        out.push((room, gen_room(rules, idx, room, open, kind, &mut rr)));
    }
    (out, start, arena)
}

/// kind: 0 entrance, 1 combat room, 2 boss arena
fn gen_room(rules: &RiftRules, idx: usize, room: (i32, i32), open: [bool; 4], kind: u8, r: &mut Rng) -> ChunkGen {
    let mut g = ChunkGen::default();
    let o = Vec3::new(room.0 as f32 * RS, 0.0, room.1 as f32 * RS);
    let p = |x: f32, y: f32, z: f32| o + Vec3::new(x, y, z);
    let wall_c = rgb(0.85, 0.85, 0.85);
    // floor
    g.mesh.quad([p(0.0, 0.0, RS), p(RS, 0.0, RS), p(RS, 0.0, 0.0), p(0.0, 0.0, 0.0)], RS / 3.0, RS / 3.0, t(rules.floor), rgbe(0.8, 0.8, 0.8, if rules.floor == Tex::Ember { 0.15 } else { 0.0 }));
    // walls with doorways: south(-z), east(+x), north(+z), west(-x)
    let th = 1.5;
    let mid = RS * 0.5;
    let wall = |g: &mut ChunkGen, a: Vec3, b: Vec3| {
        g.mesh.boxx(a, b, t(rules.wall), t(rules.accent), wall_c, 3.5, 63 & !8);
        g.cols.push(Aabb::new(a, b));
    };
    let segs = [
        (open[0], p(0.0, 0.0, 0.0), p(RS, WALL_H, th), true),
        (open[1], p(RS - th, 0.0, 0.0), p(RS, WALL_H, RS), false),
        (open[2], p(0.0, 0.0, RS - th), p(RS, WALL_H, RS), true),
        (open[3], p(0.0, 0.0, 0.0), p(th, WALL_H, RS), false),
    ];
    for (is_open, a, b, along_x) in segs {
        if is_open {
            if along_x {
                wall(&mut g, a, Vec3::new(o.x + mid - DOOR * 0.5, b.y, b.z));
                wall(&mut g, Vec3::new(o.x + mid + DOOR * 0.5, a.y, a.z), b);
                // lintel
                g.mesh.boxx(Vec3::new(o.x + mid - DOOR * 0.5, 6.0, a.z), Vec3::new(o.x + mid + DOOR * 0.5, WALL_H, b.z), t(rules.wall), t(rules.accent), wall_c, 3.5, 63 & !8);
            } else {
                wall(&mut g, a, Vec3::new(b.x, b.y, o.z + mid - DOOR * 0.5));
                wall(&mut g, Vec3::new(a.x, a.y, o.z + mid + DOOR * 0.5), b);
                g.mesh.boxx(Vec3::new(a.x, 6.0, o.z + mid - DOOR * 0.5), Vec3::new(b.x, WALL_H, o.z + mid + DOOR * 0.5), t(rules.wall), t(rules.accent), wall_c, 3.5, 63 & !8);
            }
        } else {
            wall(&mut g, a, b);
        }
    }
    // dimension-specific ceiling ribs / floating debris
    for i in 0..4 {
        let z = 4.0 + i as f32 * (RS - 8.0) / 3.0;
        let m = trs(p(mid, WALL_H + 0.5, z), Quat::from_rotation_z(0.0), Vec3::new(RS, 0.8, 0.8));
        g.mesh.obox(m, t(rules.accent), rgbe(1.0, 1.0, 1.0, 0.3));
    }
    let lc = Vec3::from(rules.light);
    match kind {
        0 => {
            g.interact.push((p(mid, 0.0, 4.0), InteractKind::ExitPortal));
            g.emitters.push((p(mid, 1.5, 3.0), EmitKind::Rift));
            g.lights.push(CLight { pos: p(mid, 3.0, 4.0), color: Vec3::new(1.5, 0.8, 3.0), radius: 14.0, flicker: 4 });
            g.lights.push(CLight { pos: p(mid, 5.0, mid + 6.0), color: lc, radius: 22.0, flicker: 4 });
            let m = trs(p(mid, 2.2, 2.6), Quat::from_rotation_y(0.4), Vec3::new(2.4, 4.0, 0.4));
            g.mesh.obox(m, t(Tex::RiftGlow), rgbe(1.0, 1.0, 1.0, 0.9));
            g.spawns.push(SpawnPt { pos: p(mid + 8.0, 0.0, RS - 8.0), kind: SpawnKind::Infected, seed: r.u32(..) });
        }
        1 => {
            // pillars / organs as cover
            for _ in 0..r.usize(3..7) {
                let (x, z) = (rf(r, 6.0, RS - 6.0), rf(r, 6.0, RS - 6.0));
                if (x - mid).abs() < 4.0 && (z - mid).abs() < 4.0 {
                    continue;
                }
                let (w, h) = (rf(r, 1.2, 2.6), rf(r, 3.0, WALL_H));
                let m = trs(p(x, h * 0.5, z), Quat::from_rotation_y(rf(r, 0.0, 3.0)), Vec3::new(w, h, w));
                g.mesh.obox(m, t(rules.wall), wall_c);
                g.cols.push(Aabb::new(p(x - w * 0.5, 0.0, z - w * 0.5), p(x + w * 0.5, h, z + w * 0.5)));
                if chance(r, 0.4) {
                    g.anims.push(AnimProp { kind: if idx == 1 { AnimKind::Shard } else { AnimKind::Tendril }, pos: p(x, h, z), yaw: x, size: 2.0 });
                }
            }
            for _ in 0..r.usize(2..4) {
                let (x, z) = (rf(r, 7.0, RS - 7.0), rf(r, 7.0, RS - 7.0));
                g.hazards.push((p(x, 0.0, z), rf(r, 2.0, 3.5), rules.hazard));
            }
            for _ in 0..r.usize(2..4) {
                g.spawns.push(SpawnPt { pos: p(rf(r, 7.0, RS - 7.0), 0.0, rf(r, 7.0, RS - 7.0)), kind: SpawnKind::Infected, seed: r.u32(..) });
            }
            if chance(r, 0.5) {
                g.spawns.push(SpawnPt { pos: p(mid, 0.0, mid), kind: SpawnKind::Cache, seed: r.u32(..) });
            }
            g.lights.push(CLight { pos: p(mid, 6.0, mid), color: lc, radius: 24.0, flicker: 4 });
            g.lights.push(CLight { pos: p(rf(r, 5.0, RS - 5.0), 2.5, rf(r, 5.0, RS - 5.0)), color: lc * 0.7, radius: 12.0, flicker: if idx == 2 { 2 } else { 4 } });
            let e = match rules.hazard {
                0 => EmitKind::Drip,
                1 => EmitKind::Embers,
                2 => EmitKind::Spores,
                _ => EmitKind::Rift,
            };
            g.emitters.push((p(mid, 2.0, mid), e));
        }
        _ => {
            // boss arena: ring of monoliths, central hazard-free floor, strong lighting
            for i in 0..10 {
                let a = i as f32 / 10.0 * std::f32::consts::TAU;
                let rad = RS * 0.5 - 3.0;
                let (x, z) = (mid + a.cos() * rad, mid + a.sin() * rad);
                let h = 6.0 + (i % 3) as f32 * 2.0;
                let m = trs(p(x, h * 0.5, z), Quat::from_rotation_y(a), Vec3::new(1.4, h, 1.4));
                g.mesh.obox(m, t(rules.accent), rgbe(1.0, 1.0, 1.0, 0.4));
                g.lights.push(CLight { pos: p(x, h, z), color: lc * 0.5, radius: 10.0, flicker: 4 });
            }
            g.mesh.quad([p(mid - 10.0, 0.02, mid + 10.0), p(mid + 10.0, 0.02, mid + 10.0), p(mid + 10.0, 0.02, mid - 10.0), p(mid - 10.0, 0.02, mid - 10.0)], 4.0, 4.0, t(rules.accent), rgbe(0.5, 0.5, 0.5, 0.1));
            g.lights.push(CLight { pos: p(mid, 9.0, mid), color: lc * 1.2, radius: 34.0, flicker: 4 });
            g.spawns.push(SpawnPt { pos: p(mid, 0.0, mid + 4.0), kind: SpawnKind::Boss(idx), seed: r.u32(..) });
            g.emitters.push((p(mid, 8.0, mid), EmitKind::Rift));
        }
    }
    g
}
