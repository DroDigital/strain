//! Mesh building: vertex format, a builder for world geometry, and the unit primitive library
//! used by instanced characters and props.
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Quat, Vec2, Vec3};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    /// u, v, texture array layer
    pub uv: [f32; 3],
    /// rgb tint, a = emissive amount
    pub color: [u8; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct Instance {
    pub m0: [f32; 4],
    pub m1: [f32; 4],
    pub m2: [f32; 4],
    /// rgb tint, a = emissive add
    pub color: [f32; 4],
    /// x = layer override (<0 keeps vertex layer), y = hit flash, z = world warp weight, w = dissolve
    pub params: [f32; 4],
}

impl Instance {
    pub fn new(m: Mat4, color: [f32; 4], layer: f32) -> Self {
        let r = m.transpose().to_cols_array_2d();
        Instance { m0: r[0], m1: r[1], m2: r[2], color, params: [layer, 0.0, 0.0, 0.0] }
    }
    pub fn identity() -> Self {
        let mut i = Instance::new(Mat4::IDENTITY, [1.0, 1.0, 1.0, 0.0], -1.0);
        i.params[2] = 1.0;
        i
    }
}

pub fn rgb(r: f32, g: f32, b: f32) -> [u8; 4] {
    [(r.clamp(0.0, 1.0) * 255.0) as u8, (g.clamp(0.0, 1.0) * 255.0) as u8, (b.clamp(0.0, 1.0) * 255.0) as u8, 0]
}
pub fn rgbe(r: f32, g: f32, b: f32, e: f32) -> [u8; 4] {
    let mut c = rgb(r, g, b);
    c[3] = (e.clamp(0.0, 1.0) * 255.0) as u8;
    c
}
pub const WHITE: [u8; 4] = [255, 255, 255, 0];

#[derive(Default, Clone)]
pub struct MeshBuilder {
    pub verts: Vec<Vertex>,
    pub idx: Vec<u32>,
}

impl MeshBuilder {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn clear(&mut self) {
        self.verts.clear();
        self.idx.clear();
    }
    /// Quad from four corners (counter-clockwise seen from the front), with explicit UVs.
    pub fn quad_uv(&mut self, p: [Vec3; 4], uv: [Vec2; 4], layer: u32, color: [u8; 4]) {
        let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
        let b = self.verts.len() as u32;
        for i in 0..4 {
            self.verts.push(Vertex { pos: p[i].into(), normal: n.into(), uv: [uv[i].x, uv[i].y, layer as f32], color });
        }
        self.idx.extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
    }
    /// Quad with UVs spanning `us` x `vs` tiles.
    pub fn quad(&mut self, p: [Vec3; 4], us: f32, vs: f32, layer: u32, color: [u8; 4]) {
        self.quad_uv(p, [Vec2::new(0.0, vs), Vec2::new(us, vs), Vec2::new(us, 0.0), Vec2::new(0.0, 0.0)], layer, color);
    }
    pub fn tri(&mut self, p: [Vec3; 3], layer: u32, color: [u8; 4]) {
        let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
        let b = self.verts.len() as u32;
        let uv = [[0.0, 1.0], [1.0, 1.0], [0.5, 0.0]];
        for i in 0..3 {
            self.verts.push(Vertex { pos: p[i].into(), normal: n.into(), uv: [uv[i][0], uv[i][1], layer as f32], color });
        }
        self.idx.extend_from_slice(&[b, b + 1, b + 2]);
    }
    /// Axis-aligned box. `tile` is metres per texture repeat (0 = stretch once per face).
    /// `faces` bitmask: 1=+x 2=-x 4=+y 8=-y 16=+z 32=-z.
    pub fn boxx(&mut self, min: Vec3, max: Vec3, layer: u32, top_layer: u32, color: [u8; 4], tile: f32, faces: u32) {
        let s = max - min;
        let t = |a: f32| if tile > 0.0 { a / tile } else { 1.0 };
        let (x0, y0, z0, x1, y1, z1) = (min.x, min.y, min.z, max.x, max.y, max.z);
        let v = Vec3::new;
        if faces & 1 != 0 {
            self.quad([v(x1, y0, z1), v(x1, y0, z0), v(x1, y1, z0), v(x1, y1, z1)], t(s.z), t(s.y), layer, color);
        }
        if faces & 2 != 0 {
            self.quad([v(x0, y0, z0), v(x0, y0, z1), v(x0, y1, z1), v(x0, y1, z0)], t(s.z), t(s.y), layer, color);
        }
        if faces & 4 != 0 {
            self.quad([v(x0, y1, z1), v(x1, y1, z1), v(x1, y1, z0), v(x0, y1, z0)], t(s.x), t(s.z), top_layer, color);
        }
        if faces & 8 != 0 {
            self.quad([v(x0, y0, z0), v(x1, y0, z0), v(x1, y0, z1), v(x0, y0, z1)], t(s.x), t(s.z), top_layer, color);
        }
        if faces & 16 != 0 {
            self.quad([v(x0, y0, z1), v(x1, y0, z1), v(x1, y1, z1), v(x0, y1, z1)], t(s.x), t(s.y), layer, color);
        }
        if faces & 32 != 0 {
            self.quad([v(x1, y0, z0), v(x0, y0, z0), v(x0, y1, z0), v(x1, y1, z0)], t(s.x), t(s.y), layer, color);
        }
    }
    /// Box with arbitrary transform (unit cube -0.5..0.5 mapped through `m`).
    pub fn obox(&mut self, m: Mat4, layer: u32, color: [u8; 4]) {
        let start = self.verts.len();
        self.boxx(Vec3::splat(-0.5), Vec3::splat(0.5), layer, layer, color, 0.0, 63);
        self.xform_from(start, m);
    }
    pub fn xform_from(&mut self, start: usize, m: Mat4) {
        let nm = m.inverse().transpose();
        for v in &mut self.verts[start..] {
            v.pos = m.transform_point3(Vec3::from(v.pos)).into();
            v.normal = nm.transform_vector3(Vec3::from(v.normal)).normalize_or_zero().into();
        }
    }
    /// Tapered prism along +Y from y=0..1, `n` sides, radius r0 at the bottom and r1 at the top.
    pub fn prism(&mut self, n: u32, r0: f32, r1: f32, layer: u32, color: [u8; 4], caps: bool) {
        let ring = |i: u32, r: f32, y: f32| {
            let a = i as f32 / n as f32 * std::f32::consts::TAU + std::f32::consts::PI / n as f32;
            Vec3::new(a.cos() * r, y, a.sin() * r)
        };
        for i in 0..n {
            let (a0, a1, b0, b1) = (ring(i, r0, 0.0), ring(i + 1, r0, 0.0), ring(i, r1, 1.0), ring(i + 1, r1, 1.0));
            let (u0, u1) = (i as f32 / n as f32, (i + 1) as f32 / n as f32);
            if r1 > 1e-4 {
                self.quad_uv([a1, a0, b0, b1], [Vec2::new(u1, 1.0), Vec2::new(u0, 1.0), Vec2::new(u0, 0.0), Vec2::new(u1, 0.0)], layer, color);
            } else {
                self.tri([a1, a0, Vec3::Y], layer, color);
            }
            if caps {
                self.tri([Vec3::ZERO, a0, a1], layer, color);
                if r1 > 1e-4 {
                    self.tri([Vec3::Y, b1, b0], layer, color);
                }
            }
        }
    }
    /// Low-poly sphere, radius 0.5 centred at origin.
    pub fn sphere(&mut self, seg: u32, rings: u32, layer: u32, color: [u8; 4]) {
        let p = |i: u32, j: u32| {
            let th = i as f32 / seg as f32 * std::f32::consts::TAU;
            let ph = j as f32 / rings as f32 * std::f32::consts::PI;
            Vec3::new(ph.sin() * th.cos(), ph.cos(), ph.sin() * th.sin()) * 0.5
        };
        for j in 0..rings {
            for i in 0..seg {
                let (a, b, c, d) = (p(i, j), p(i + 1, j), p(i + 1, j + 1), p(i, j + 1));
                let (u0, u1, v0, v1) = (i as f32 / seg as f32, (i + 1) as f32 / seg as f32, j as f32 / rings as f32, (j + 1) as f32 / rings as f32);
                if j == 0 {
                    self.tri([a, c, d], layer, color);
                } else if j == rings - 1 {
                    self.tri([a, b, d], layer, color);
                } else {
                    self.quad_uv([d, c, b, a], [Vec2::new(u0, v1), Vec2::new(u1, v1), Vec2::new(u1, v0), Vec2::new(u0, v0)], layer, color);
                }
            }
        }
    }
    pub fn append(&mut self, other: &MeshBuilder, m: Mat4) {
        let b = self.verts.len() as u32;
        let start = self.verts.len();
        self.verts.extend_from_slice(&other.verts);
        self.idx.extend(other.idx.iter().map(|i| i + b));
        self.xform_from(start, m);
    }
}

pub fn trs(t: Vec3, r: Quat, s: Vec3) -> Mat4 {
    Mat4::from_scale_rotation_translation(s, r, t)
}

/// Transform that maps a unit Y-up primitive (y in 0..1) onto the segment a->b with the given thickness.
pub fn segment(a: Vec3, b: Vec3, tx: f32, tz: f32, fwd_hint: Vec3) -> Mat4 {
    let d = b - a;
    let len = d.length().max(1e-4);
    let y = d / len;
    let mut x = fwd_hint.cross(y);
    if x.length_squared() < 1e-6 {
        x = Vec3::X.cross(y);
        if x.length_squared() < 1e-6 {
            x = Vec3::Z.cross(y);
        }
    }
    let x = x.normalize();
    let z = x.cross(y);
    Mat4::from_cols((x * tx).extend(0.0), (y * len).extend(0.0), (z * tz).extend(0.0), a.extend(1.0))
}

/// Ids of the shared instanced primitive meshes.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[repr(u32)]
pub enum Prim {
    /// unit cube centred at origin
    Cube,
    /// box from y=0..1, 1x1 footprint, slightly tapered (limbs)
    Limb,
    /// sphere radius 0.5
    Sphere,
    /// cylinder y=0..1 radius 0.5
    Cyl,
    /// cone y=0..1 radius 0.5
    Spike,
    /// flat diamond blade y=0..1
    Blade,
    /// hexagonal plate
    Plate,
    /// upright quad facing +z, y 0..1
    Card,
    /// wedge (triangular prism)
    Wedge,
    Count,
}

#[derive(Clone, Copy, Debug)]
pub struct MeshRange {
    pub first: u32,
    pub count: u32,
    pub base: i32,
}

pub fn build_prims() -> (MeshBuilder, Vec<MeshRange>) {
    let mut all = MeshBuilder::new();
    let mut ranges = Vec::new();
    let mut push = |m: MeshBuilder, all: &mut MeshBuilder| {
        ranges.push(MeshRange { first: all.idx.len() as u32, count: m.idx.len() as u32, base: all.verts.len() as i32 });
        all.verts.extend_from_slice(&m.verts);
        all.idx.extend_from_slice(&m.idx);
    };
    // Cube
    let mut m = MeshBuilder::new();
    m.boxx(Vec3::splat(-0.5), Vec3::splat(0.5), 0, 0, WHITE, 0.0, 63);
    push(m, &mut all);
    // Limb
    let mut m = MeshBuilder::new();
    m.prism(4, 0.62, 0.5, 0, WHITE, true);
    push(m, &mut all);
    let mut m = MeshBuilder::new();
    m.sphere(7, 5, 0, WHITE);
    push(m, &mut all);
    let mut m = MeshBuilder::new();
    m.prism(7, 0.5, 0.5, 0, WHITE, true);
    push(m, &mut all);
    let mut m = MeshBuilder::new();
    m.prism(5, 0.5, 0.0, 0, WHITE, true);
    push(m, &mut all);
    // Blade: diamond cross-section tapering to a point
    let mut m = MeshBuilder::new();
    {
        let v = Vec3::new;
        let (a, b, c, d) = (v(0.5, 0.0, 0.0), v(0.0, 0.0, 0.12), v(-0.5, 0.0, 0.0), v(0.0, 0.0, -0.12));
        let (a1, b1, c1, d1) = (v(0.42, 0.8, 0.0), v(0.0, 0.8, 0.1), v(-0.3, 0.8, 0.0), v(0.0, 0.8, -0.1));
        let tip = v(0.3, 1.0, 0.0);
        m.quad([b, a, a1, b1], 1.0, 1.0, 0, WHITE);
        m.quad([c, b, b1, c1], 1.0, 1.0, 0, WHITE);
        m.quad([d, c, c1, d1], 1.0, 1.0, 0, WHITE);
        m.quad([a, d, d1, a1], 1.0, 1.0, 0, WHITE);
        m.tri([b1, a1, tip], 0, WHITE);
        m.tri([c1, b1, tip], 0, WHITE);
        m.tri([d1, c1, tip], 0, WHITE);
        m.tri([a1, d1, tip], 0, WHITE);
    }
    push(m, &mut all);
    let mut m = MeshBuilder::new();
    m.prism(6, 0.5, 0.42, 0, WHITE, true);
    let nv = m.verts.len();
    for v in &mut m.verts[..nv] {
        v.pos[1] *= 0.2;
    }
    push(m, &mut all);
    let mut m = MeshBuilder::new();
    {
        let v = Vec3::new;
        m.quad([v(-0.5, 0.0, 0.0), v(0.5, 0.0, 0.0), v(0.5, 1.0, 0.0), v(-0.5, 1.0, 0.0)], 1.0, 1.0, 0, WHITE);
        m.quad([v(0.5, 0.0, 0.0), v(-0.5, 0.0, 0.0), v(-0.5, 1.0, 0.0), v(0.5, 1.0, 0.0)], 1.0, 1.0, 0, WHITE);
    }
    push(m, &mut all);
    let mut m = MeshBuilder::new();
    {
        let v = Vec3::new;
        let (a, b, c) = (v(-0.5, 0.0, -0.5), v(0.5, 0.0, -0.5), v(-0.5, 1.0, -0.5));
        let (a2, b2, c2) = (v(-0.5, 0.0, 0.5), v(0.5, 0.0, 0.5), v(-0.5, 1.0, 0.5));
        m.tri([b, a, c], 0, WHITE);
        m.tri([a2, b2, c2], 0, WHITE);
        m.quad([a, a2, c2, c], 1.0, 1.0, 0, WHITE);
        m.quad([b2, b, c, c2], 1.0, 1.0, 0, WHITE);
        m.quad([a, b, b2, a2], 1.0, 1.0, 0, WHITE);
    }
    push(m, &mut all);
    (all, ranges)
}
