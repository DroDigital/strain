//! World: districts, streamed chunks, static collision. The city is a 18x12 grid of 56 m chunks
//! split into six districts; rifts are small self-contained areas built from the same chunk type.
pub mod kit;
pub mod rift;

use crate::physics::{Aabb, Statics};
use crate::render::mesh::MeshBuilder;
use crate::render::{Renderer, StaticMesh};

use glam::{Vec2, Vec3};
use std::collections::HashMap;

pub const CS: f32 = 56.0;
pub const DSIZE: i32 = 6;
pub const CITY_W: i32 = 18;
pub const CITY_H: i32 = 12;
pub const ROAD: f32 = 7.0;
pub const KERB: f32 = 0.12;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DState {
    Held,
    Contested,
    Lost,
}

#[allow(dead_code)]
pub struct District {
    pub name: &'static str,
    pub state: DState,
    pub levels: (u32, u32),
    pub landmark: &'static str,
    /// grid position of the district (columns, rows of DSIZE chunks)
    pub grid: (i32, i32),
    pub fog: [f32; 3],
    pub grade: [f32; 3],
    pub saturation: f32,
    pub palette_levels: f32,
    pub fog_density: f32,
    /// local chunk coordinates of specials
    pub pits: [(i32, i32); 2],
    pub rift: Option<(i32, i32)>,
    pub landmark_at: (i32, i32),
}

pub const DISTRICTS: [District; 6] = [
    District { name: "THE CORDON", state: DState::Held, levels: (1, 5), landmark: "STADIUM FOB", grid: (1, 0), fog: [0.10, 0.11, 0.14], grade: [1.05, 1.0, 0.92], saturation: 0.9, palette_levels: 28.0, fog_density: 0.012, pits: [(2, 2), (4, 4)], rift: None, landmark_at: (3, 3) },
    District { name: "MARKET ROW", state: DState::Contested, levels: (5, 10), landmark: "THE CLOCK", grid: (0, 0), fog: [0.14, 0.10, 0.08], grade: [1.1, 0.95, 0.8], saturation: 0.85, palette_levels: 22.0, fog_density: 0.018, pits: [(4, 1), (1, 2)], rift: Some((1, 4)), landmark_at: (2, 3) },
    District { name: "THE TENEMENTS", state: DState::Contested, levels: (10, 15), landmark: "THE SPIRE", grid: (2, 0), fog: [0.08, 0.11, 0.13], grade: [0.85, 1.0, 1.1], saturation: 0.75, palette_levels: 20.0, fog_density: 0.02, pits: [(1, 1), (4, 2)], rift: Some((4, 4)), landmark_at: (3, 2) },
    District { name: "RAIL YARDS", state: DState::Contested, levels: (15, 20), landmark: "WATER TOWER", grid: (2, 1), fog: [0.13, 0.12, 0.09], grade: [1.05, 1.0, 0.8], saturation: 0.7, palette_levels: 18.0, fog_density: 0.022, pits: [(1, 4), (4, 4)], rift: Some((4, 1)), landmark_at: (2, 2) },
    District { name: "THE BLOOM", state: DState::Lost, levels: (20, 30), landmark: "THE FIRST FLOWER", grid: (0, 1), fog: [0.10, 0.05, 0.12], grade: [1.0, 0.8, 1.15], saturation: 1.1, palette_levels: 14.0, fog_density: 0.03, pits: [(4, 4), (2, 4)], rift: Some((1, 1)), landmark_at: (3, 2) },
    District { name: "GROUND ZERO", state: DState::Lost, levels: (30, 40), landmark: "THE CRATER", grid: (1, 1), fog: [0.04, 0.10, 0.09], grade: [0.8, 1.1, 1.0], saturation: 1.0, palette_levels: 12.0, fog_density: 0.034, pits: [(1, 1), (4, 0)], rift: Some((3, 3)), landmark_at: (3, 3) },
];

/// District index for a chunk coordinate, or None outside the city.
pub fn district_of(cx: i32, cz: i32) -> Option<usize> {
    if cx < 0 || cz < 0 || cx >= CITY_W || cz >= CITY_H {
        return None;
    }
    let g = (cx / DSIZE, cz / DSIZE);
    DISTRICTS.iter().position(|d| d.grid == g)
}
pub fn district_at(p: Vec3) -> Option<usize> {
    district_of((p.x / CS).floor() as i32, (p.z / CS).floor() as i32)
}
pub fn chunk_center(d: usize, local: (i32, i32)) -> Vec3 {
    let g = DISTRICTS[d].grid;
    Vec3::new(((g.0 * DSIZE + local.0) as f32 + 0.5) * CS, 0.0, ((g.1 * DSIZE + local.1) as f32 + 0.5) * CS)
}
pub fn pit_pos(id: usize) -> Vec3 {
    chunk_center(id / 2, DISTRICTS[id / 2].pits[id % 2]) + Vec3::new(0.0, KERB, 0.0)
}
/// World position of the rift portal of rift `i` (0..=3 district rifts, 4 = final).
pub fn rift_site(i: usize) -> Vec3 {
    let d = i + 1;
    chunk_center(d, DISTRICTS[d].rift.unwrap()) + Vec3::new(0.0, KERB, 0.0)
}

#[derive(Clone, Copy, Debug)]
pub struct CLight {
    pub pos: Vec3,
    pub color: Vec3,
    pub radius: f32,
    /// 0 steady, 1 sodium buzz, 2 fire, 3 broken, 4 slow pulse
    pub flicker: u8,
}
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum EmitKind {
    Smoke,
    Fire,
    Spores,
    Steam,
    Flare,
    Rift,
    Embers,
    Drip,
}
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum AnimKind {
    Flag,
    Tendril,
    Sign,
    Pod,
    Litter,
    Shard,
}
#[derive(Clone, Copy, Debug)]
pub struct AnimProp {
    pub kind: AnimKind,
    pub pos: Vec3,
    pub yaw: f32,
    pub size: f32,
}
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SpawnKind {
    Infected,
    Soldiers,
    Civilians,
    Apex(usize),
    Boss(usize),
    Cache,
}
#[derive(Clone, Copy, Debug)]
pub struct SpawnPt {
    pub pos: Vec3,
    pub kind: SpawnKind,
    pub seed: u32,
}
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum InteractKind {
    BurnPit(usize),
    Vendor,
    Stash,
    RiftPortal(usize),
    ExitPortal,
    Note(usize),
    Husk,
}

#[derive(Default)]
pub struct ChunkGen {
    pub mesh: MeshBuilder,
    pub cols: Vec<Aabb>,
    pub lights: Vec<CLight>,
    pub emitters: Vec<(Vec3, EmitKind)>,
    pub anims: Vec<AnimProp>,
    pub spawns: Vec<SpawnPt>,
    pub interact: Vec<(Vec3, InteractKind)>,
    /// hazard zones baked into the area (pos, radius, kind index)
    pub hazards: Vec<(Vec3, f32, u8)>,
    /// kit models placed in the chunk: (model name, transform, tint rgb + emissive)
    pub props: Vec<(&'static str, glam::Mat4, [f32; 4])>,
}

pub struct Chunk {
    pub mesh: Option<StaticMesh>,
    /// geometry waiting to be uploaded by the renderer
    pub pending: Option<MeshBuilder>,
    pub cols: Vec<Aabb>,
    pub lights: Vec<CLight>,
    pub emitters: Vec<(Vec3, EmitKind)>,
    pub anims: Vec<AnimProp>,
    pub spawns: Vec<SpawnPt>,
    pub interact: Vec<(Vec3, InteractKind)>,
    pub hazards: Vec<(Vec3, f32, u8)>,
    pub props: Vec<(&'static str, glam::Mat4, [f32; 4])>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AreaKind {
    City,
    /// 0..=3 district rifts, 4 = the final arena
    Rift(usize),
}

pub struct Area {
    pub kind: AreaKind,
    pub cell: f32,
    pub chunks: HashMap<(i32, i32), Chunk>,
    pub seed: u32,
    /// rest counter: changes the front line and spawn seeds
    pub epoch: u32,
    pub bosses_dead: [bool; 5],
    /// rift only: start position, boss arena centre and radius
    pub start: Vec3,
    pub arena: (Vec3, f32),
    /// low-detail silhouettes of every city chunk (drawn where the full chunk is not loaded)
    pub far: HashMap<(i32, i32), Option<StaticMesh>>,
}

impl Statics for Area {
    fn each_box(&self, min: Vec2, max: Vec2, f: &mut dyn FnMut(&Aabb)) {
        let (x0, x1) = ((min.x / self.cell).floor() as i32, (max.x / self.cell).floor() as i32);
        let (z0, z1) = ((min.y / self.cell).floor() as i32, (max.y / self.cell).floor() as i32);
        for cx in x0..=x1 {
            for cz in z0..=z1 {
                if let Some(c) = self.chunks.get(&(cx, cz)) {
                    for b in &c.cols {
                        if b.max.x >= min.x && b.min.x <= max.x && b.max.z >= min.y && b.min.z <= max.y {
                            f(b);
                        }
                    }
                }
            }
        }
    }
}

impl Area {
    pub fn city(seed: u32) -> Area {
        Area { kind: AreaKind::City, cell: CS, chunks: HashMap::new(), seed, epoch: 0, bosses_dead: [false; 5], start: Vec3::ZERO, arena: (Vec3::ZERO, 0.0), far: HashMap::new() }
    }
    pub fn key(&self, p: Vec3) -> (i32, i32) {
        ((p.x / self.cell).floor() as i32, (p.z / self.cell).floor() as i32)
    }
    /// Floor height (kerbs in the city).
    pub fn ground(&self, x: f32, z: f32) -> f32 {
        if self.kind != AreaKind::City {
            return 0.0;
        }
        let (lx, lz) = (x.rem_euclid(CS), z.rem_euclid(CS));
        let d = (lx - ROAD).min(CS - ROAD - lx).min(lz - ROAD).min(CS - ROAD - lz);
        (d * 0.5).clamp(0.0, 1.0) * KERB
    }
    fn make_chunk(g: ChunkGen) -> Chunk {
        Chunk { mesh: None, pending: Some(g.mesh), cols: g.cols, lights: g.lights, emitters: g.emitters, anims: g.anims, spawns: g.spawns, interact: g.interact, hazards: g.hazards, props: g.props }
    }
    /// Load/unload city chunks around `center`. Generates at most `budget` chunks per call.
    /// Returns (newly loaded keys, unloaded keys).
    pub fn stream(&mut self, center: Vec3, radius: i32, budget: usize) -> (Vec<(i32, i32)>, Vec<(i32, i32)>) {
        let mut loaded = Vec::new();
        let mut unloaded = Vec::new();
        if self.kind != AreaKind::City {
            return (loaded, unloaded);
        }
        let (ccx, ccz) = self.key(center);
        let mut want: Vec<(i32, i32)> = Vec::new();
        for dz in -radius..=radius {
            for dx in -radius..=radius {
                want.push((ccx + dx, ccz + dz));
            }
        }
        want.sort_by_key(|k| (k.0 - ccx).abs() + (k.1 - ccz).abs());
        for k in want {
            if loaded.len() >= budget {
                break;
            }
            if !self.chunks.contains_key(&k) {
                let g = kit::gen_chunk(self.seed, self.epoch, &self.bosses_dead, k.0, k.1);
                self.chunks.insert(k, Self::make_chunk(g));
                loaded.push(k);
            }
        }
        let far: Vec<(i32, i32)> = self.chunks.keys().filter(|k| (k.0 - ccx).abs() > radius + 1 || (k.1 - ccz).abs() > radius + 1).copied().collect();
        for k in far {
            self.chunks.remove(&k);
            unloaded.push(k);
        }
        (loaded, unloaded)
    }
    pub fn rift(idx: usize, seed: u32, epoch: u32) -> Area {
        let (gens, start, arena) = rift::gen_rift(idx, seed ^ epoch.wrapping_mul(7919));
        let mut a = Area { kind: AreaKind::Rift(idx), cell: rift::RS, chunks: HashMap::new(), seed, epoch, bosses_dead: [false; 5], start, arena, far: HashMap::new() };
        for (k, g) in gens {
            a.chunks.insert(k, Self::make_chunk(g));
        }
        a
    }

    /// Upload chunk geometry generated since the last frame (at most `budget` chunks).
    pub fn upload_pending(&mut self, r: &Renderer, budget: usize) {
        let mut n = 0;
        for c in self.chunks.values_mut() {
            if n >= budget {
                break;
            }
            if let Some(mb) = c.pending.take() {
                c.mesh = r.upload_mesh(&mb);
                n += 1;
            }
        }
        if self.kind == AreaKind::City && self.far.len() < ((CITY_W + 2) * (CITY_H + 2)) as usize {
            for cz in -1..=CITY_H {
                for cx in -1..=CITY_W {
                    if !self.far.contains_key(&(cx, cz)) {
                        let m = kit::far_chunk(self.seed, cx, cz);
                        self.far.insert((cx, cz), r.upload_mesh(&m));
                    }
                }
            }
        }
    }
}
