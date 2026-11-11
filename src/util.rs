//! Small shared helpers: hashing, noise, easing, seeded RNG wrapper, a tiny executor.
use glam::{Vec2, Vec3};

pub type Rng = fastrand::Rng;

pub fn hash_u32(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}

pub fn hash2(x: i32, y: i32, seed: u32) -> u32 {
    hash_u32((x as u32).wrapping_mul(0x9E37_79B1) ^ hash_u32((y as u32).wrapping_add(seed.wrapping_mul(0x85EB_CA6B))))
}

pub fn seed_rng(a: u32, b: u32, c: u32) -> Rng {
    let s = hash_u32(a ^ hash_u32(b ^ hash_u32(c.wrapping_add(0x1234_5678))));
    Rng::with_seed(((s as u64) << 32) | hash_u32(s ^ 0xdead_beef) as u64)
}

pub fn hash01(x: i32, y: i32, seed: u32) -> f32 {
    (hash2(x, y, seed) & 0xffff) as f32 / 65535.0
}

/// Value noise in 2D, range 0..1.
pub fn noise2(p: Vec2, seed: u32) -> f32 {
    let i = p.floor();
    let f = p - i;
    let u = f * f * (Vec2::splat(3.0) - 2.0 * f);
    let (x, y) = (i.x as i32, i.y as i32);
    let a = hash01(x, y, seed);
    let b = hash01(x + 1, y, seed);
    let c = hash01(x, y + 1, seed);
    let d = hash01(x + 1, y + 1, seed);
    lerp(lerp(a, b, u.x), lerp(c, d, u.x), u.y)
}

pub fn fbm2(mut p: Vec2, seed: u32, oct: u32) -> f32 {
    let (mut a, mut s, mut n) = (0.5, 0.0, 0.0);
    for o in 0..oct {
        s += a * noise2(p, seed.wrapping_add(o * 131));
        n += a;
        a *= 0.5;
        p *= 2.03;
    }
    s / n
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
pub fn saturate(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}
pub fn smooth(t: f32) -> f32 {
    let t = saturate(t);
    t * t * (3.0 - 2.0 * t)
}
pub fn ease_out(t: f32) -> f32 {
    let t = 1.0 - saturate(t);
    1.0 - t * t * t
}
pub fn ease_in(t: f32) -> f32 {
    let t = saturate(t);
    t * t * t
}
pub fn approach(cur: f32, target: f32, step: f32) -> f32 {
    if cur < target {
        (cur + step).min(target)
    } else {
        (cur - step).max(target)
    }
}
pub fn wrap_angle(mut a: f32) -> f32 {
    use std::f32::consts::PI;
    while a > PI {
        a -= 2.0 * PI;
    }
    while a < -PI {
        a += 2.0 * PI;
    }
    a
}
pub fn angle_lerp(a: f32, b: f32, t: f32) -> f32 {
    a + wrap_angle(b - a) * t
}
pub fn turn_toward(a: f32, b: f32, max: f32) -> f32 {
    let d = wrap_angle(b - a);
    a + d.clamp(-max, max)
}
/// Forward vector for a yaw (yaw 0 faces +Z).
pub fn yaw_dir(yaw: f32) -> Vec3 {
    Vec3::new(yaw.sin(), 0.0, yaw.cos())
}
pub fn dir_yaw(d: Vec3) -> f32 {
    d.x.atan2(d.z)
}
pub fn flat(v: Vec3) -> Vec3 {
    Vec3::new(v.x, 0.0, v.z)
}
pub fn rf(r: &mut Rng, a: f32, b: f32) -> f32 {
    a + (b - a) * r.f32()
}
pub fn chance(r: &mut Rng, p: f32) -> bool {
    r.f32() < p
}
pub fn pick<'a, T>(r: &mut Rng, s: &'a [T]) -> &'a T {
    &s[r.usize(0..s.len())]
}
pub fn rand_dir(r: &mut Rng) -> Vec3 {
    let a = r.f32() * std::f32::consts::TAU;
    Vec3::new(a.cos(), 0.0, a.sin())
}
pub fn rand_sphere(r: &mut Rng) -> Vec3 {
    loop {
        let v = Vec3::new(r.f32() * 2.0 - 1.0, r.f32() * 2.0 - 1.0, r.f32() * 2.0 - 1.0);
        if v.length_squared() <= 1.0 && v.length_squared() > 1e-4 {
            return v.normalize();
        }
    }
}

/// Minimal executor for the handful of wgpu setup futures (they resolve immediately on native).
pub fn block_on<F: std::future::Future>(f: F) -> F::Output {
    let mut f = std::pin::pin!(f);
    let waker = std::task::Waker::noop();
    let mut cx = std::task::Context::from_waker(waker);
    loop {
        if let std::task::Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::yield_now();
    }
}

/// Critically damped spring toward a target.
#[derive(Clone, Copy, Default, Debug)]
pub struct Spring {
    pub x: Vec3,
    pub v: Vec3,
}
impl Spring {
    pub fn step(&mut self, target: Vec3, stiff: f32, damp: f32, dt: f32) {
        let a = (target - self.x) * stiff - self.v * damp;
        self.v += a * dt;
        self.x += self.v * dt;
    }
}

/// Right-handed view matrix looking from `eye` to `at`.
pub fn look_at(eye: Vec3, at: Vec3, up: Vec3) -> glam::Mat4 {
    let f = (at - eye).normalize();
    let s = f.cross(up).normalize();
    let u = s.cross(f);
    glam::Mat4::from_cols(
        glam::Vec4::new(s.x, u.x, -f.x, 0.0),
        glam::Vec4::new(s.y, u.y, -f.y, 0.0),
        glam::Vec4::new(s.z, u.z, -f.z, 0.0),
        glam::Vec4::new(-s.dot(eye), -u.dot(eye), f.dot(eye), 1.0),
    )
}
/// Right-handed perspective projection with depth 0..1.
pub fn perspective(fovy: f32, aspect: f32, near: f32, far: f32) -> glam::Mat4 {
    let h = 1.0 / (fovy * 0.5).tan();
    let r = far / (near - far);
    glam::Mat4::from_cols(
        glam::Vec4::new(h / aspect, 0.0, 0.0, 0.0),
        glam::Vec4::new(0.0, h, 0.0, 0.0),
        glam::Vec4::new(0.0, 0.0, r, -1.0),
        glam::Vec4::new(0.0, 0.0, r * near, 0.0),
    )
}
/// Right-handed orthographic projection with depth 0..1.
pub fn ortho(l: f32, r: f32, b: f32, t: f32, near: f32, far: f32) -> glam::Mat4 {
    let (rw, rh, rd) = (1.0 / (r - l), 1.0 / (t - b), 1.0 / (near - far));
    glam::Mat4::from_cols(
        glam::Vec4::new(2.0 * rw, 0.0, 0.0, 0.0),
        glam::Vec4::new(0.0, 2.0 * rh, 0.0, 0.0),
        glam::Vec4::new(0.0, 0.0, rd, 0.0),
        glam::Vec4::new(-(l + r) * rw, -(t + b) * rh, rd * near, 1.0),
    )
}
