//! Collision: static AABBs in a per-chunk grid, cylinder character movement with sliding,
//! ray casts, and a spatial hash for dynamic bodies.
use glam::{Vec2, Vec3};
use std::collections::HashMap;

pub const SOLID: u8 = 1;
/// Blocks sight, projectiles and the camera.
pub const OPAQUE: u8 = 2;

#[derive(Clone, Copy, Debug)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
    pub flags: u8,
}
impl Aabb {
    pub fn new(min: Vec3, max: Vec3) -> Self {
        Aabb { min, max, flags: SOLID | OPAQUE }
    }
    pub fn low(min: Vec3, max: Vec3) -> Self {
        Aabb { min, max, flags: SOLID }
    }
    pub fn ray(&self, o: Vec3, d: Vec3, max_t: f32) -> Option<f32> {
        let inv = Vec3::new(1.0 / d.x, 1.0 / d.y, 1.0 / d.z);
        let t1 = (self.min - o) * inv;
        let t2 = (self.max - o) * inv;
        let tmin = t1.min(t2).max_element().max(0.0);
        let tmax = t1.max(t2).min_element().min(max_t);
        if tmin <= tmax {
            Some(tmin)
        } else {
            None
        }
    }
}

/// Static collider set for one world chunk, bucketed on a coarse grid.
#[derive(Default, Clone)]
pub struct ColSet {
    pub boxes: Vec<Aabb>,
}

/// Anything that can answer static collision queries.
pub trait Statics {
    fn each_box(&self, min: Vec2, max: Vec2, f: &mut dyn FnMut(&Aabb));

    /// Move a vertical cylinder, sliding along walls. Returns the new position and whether it touched something.
    fn move_cyl(&self, pos: Vec3, delta: Vec3, radius: f32, height: f32) -> (Vec3, bool) {
        let mut p = pos + delta;
        let mut hit = false;
        let reach = radius + delta.length() + 0.5;
        let mut near: Vec<Aabb> = Vec::new();
        self.each_box(Vec2::new(p.x - reach, p.z - reach), Vec2::new(p.x + reach, p.z + reach), &mut |b| {
            if b.flags & SOLID != 0 && b.max.y > p.y + 0.3 && b.min.y < p.y + height {
                near.push(*b);
            }
        });
        for _ in 0..3 {
            let mut any = false;
            for b in &near {
                let cx = p.x.clamp(b.min.x, b.max.x);
                let cz = p.z.clamp(b.min.z, b.max.z);
                let (dx, dz) = (p.x - cx, p.z - cz);
                let d2 = dx * dx + dz * dz;
                if d2 < radius * radius {
                    any = true;
                    hit = true;
                    if d2 > 1e-8 {
                        let d = d2.sqrt();
                        p.x += dx / d * (radius - d);
                        p.z += dz / d * (radius - d);
                    } else {
                        // centre is inside the box: push out along the shallowest axis
                        let l = p.x - b.min.x;
                        let r = b.max.x - p.x;
                        let n = p.z - b.min.z;
                        let f = b.max.z - p.z;
                        let m = l.min(r).min(n).min(f);
                        if m == l {
                            p.x = b.min.x - radius;
                        } else if m == r {
                            p.x = b.max.x + radius;
                        } else if m == n {
                            p.z = b.min.z - radius;
                        } else {
                            p.z = b.max.z + radius;
                        }
                    }
                }
            }
            if !any {
                break;
            }
        }
        (p, hit)
    }

    /// Distance to the first box hit by the ray (only boxes with the given flag).
    fn raycast(&self, o: Vec3, d: Vec3, max_t: f32, flag: u8) -> Option<f32> {
        let e = o + d * max_t;
        let mut best: Option<f32> = None;
        self.each_box(Vec2::new(o.x.min(e.x) - 0.1, o.z.min(e.z) - 0.1), Vec2::new(o.x.max(e.x) + 0.1, o.z.max(e.z) + 0.1), &mut |b| {
            if b.flags & flag != 0 {
                if let Some(t) = b.ray(o, d, best.unwrap_or(max_t)) {
                    best = Some(t);
                }
            }
        });
        best
    }

    fn line_of_sight(&self, a: Vec3, b: Vec3) -> bool {
        let d = b - a;
        let l = d.length();
        l < 0.01 || self.raycast(a, d / l, l, OPAQUE).is_none()
    }

    fn blocked(&self, p: Vec3, radius: f32) -> bool {
        let mut hit = false;
        self.each_box(Vec2::new(p.x - radius, p.z - radius), Vec2::new(p.x + radius, p.z + radius), &mut |b| {
            if b.flags & SOLID != 0 && b.max.y > p.y + 0.3 && b.min.y < p.y + 1.6 {
                let cx = p.x.clamp(b.min.x, b.max.x);
                let cz = p.z.clamp(b.min.z, b.max.z);
                if (p.x - cx).powi(2) + (p.z - cz).powi(2) < radius * radius {
                    hit = true;
                }
            }
        });
        hit
    }
}

/// Uniform spatial hash of dynamic bodies (rebuilt every tick).
#[derive(Default)]
pub struct SpatialHash {
    cells: HashMap<(i32, i32), Vec<(u32, Vec3, f32)>>,
}
const CELL: f32 = 4.0;
impl SpatialHash {
    pub fn clear(&mut self) {
        for v in self.cells.values_mut() {
            v.clear();
        }
    }
    pub fn insert(&mut self, id: u32, pos: Vec3, radius: f32) {
        self.cells.entry(((pos.x / CELL).floor() as i32, (pos.z / CELL).floor() as i32)).or_default().push((id, pos, radius));
    }
    /// Calls `f(id, pos, radius)` for every body whose centre is within `r` (+ its own radius) of `p` on the XZ plane.
    pub fn query(&self, p: Vec3, r: f32, mut f: impl FnMut(u32, Vec3, f32)) {
        let reach = r + 3.0;
        let (x0, x1) = (((p.x - reach) / CELL).floor() as i32, ((p.x + reach) / CELL).floor() as i32);
        let (z0, z1) = (((p.z - reach) / CELL).floor() as i32, ((p.z + reach) / CELL).floor() as i32);
        for x in x0..=x1 {
            for z in z0..=z1 {
                if let Some(v) = self.cells.get(&(x, z)) {
                    for &(id, bp, br) in v {
                        let d2 = (bp.x - p.x).powi(2) + (bp.z - p.z).powi(2);
                        if d2 <= (r + br) * (r + br) {
                            f(id, bp, br);
                        }
                    }
                }
            }
        }
    }
}
