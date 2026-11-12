//! Orbit camera with collision, lock-on framing, trauma shake and cutscene override.
use crate::physics::{Statics, OPAQUE};
use crate::util::{angle_lerp, look_at, perspective, yaw_dir};
use glam::{Mat4, Vec3};

pub struct Camera {
    pub yaw: f32,
    pub pitch: f32,
    pub dist: f32,
    pub eye: Vec3,
    pub target: Vec3,
    pub trauma: f32,
    /// directional recoil from blows, springing back to zero
    pub kick_off: Vec3,
    pub snap: bool,
    pub fov: f32,
    pub cur_dist: f32,
    pub focus: Vec3,
    /// cutscene override (eye, target)
    pub over: Option<(Vec3, Vec3)>,
    t: f32,
}

impl Camera {
    pub fn new() -> Camera {
        Camera { yaw: 0.0, pitch: 0.32, dist: 4.4, eye: Vec3::ZERO, target: Vec3::ZERO, trauma: 0.0, kick_off: Vec3::ZERO, snap: true, fov: 70.0, cur_dist: 4.4, focus: Vec3::ZERO, over: None, t: 0.0 }
    }

    /// `look` is the mouse/stick delta; `lock` the lock-on target position.
    pub fn update(&mut self, dt: f32, look: glam::Vec2, invert: bool, player: Vec3, lock: Option<Vec3>, area: &dyn Statics, shake_scale: f32) {
        self.t += dt;
        if let Some((e, t)) = self.over {
            self.eye = e;
            self.target = t;
            self.trauma = (self.trauma - dt * 1.5).max(0.0);
            return;
        }
        let focus = player + Vec3::Y * 1.55;
        if self.snap {
            self.focus = focus;
        } else {
            self.focus = self.focus.lerp(focus, (dt * 14.0).min(1.0));
        }
        match lock {
            Some(lp) => {
                let d = lp - player;
                let want = d.x.atan2(d.z);
                self.yaw = angle_lerp(self.yaw, want, (dt * 6.0).min(1.0));
                let dist = Vec3::new(d.x, 0.0, d.z).length();
                let want_p = (0.22 + 0.25 * (1.0 - (dist / 10.0).min(1.0))).clamp(0.15, 0.55) + ((lp.y - player.y) * 0.04);
                self.pitch += (want_p - self.pitch) * (dt * 4.0).min(1.0);
            }
            None => {
                self.yaw -= look.x;
                self.pitch += look.y * if invert { -1.0 } else { 1.0 };
            }
        }
        self.pitch = self.pitch.clamp(-0.5, 1.2);
        let back = -yaw_dir(self.yaw);
        let dir = (back * self.pitch.cos() + Vec3::Y * self.pitch.sin()).normalize();
        let shoulder = Vec3::new(-back.z, 0.0, back.x) * -0.45;
        let origin = self.focus + shoulder;
        // collision: pull in toward the player when walls intervene
        let mut want = self.dist;
        if let Some(hit) = area.raycast(origin, dir, self.dist + 0.3, OPAQUE) {
            want = (hit - 0.3).max(0.6);
        }
        if want < self.cur_dist || self.snap {
            self.cur_dist = want;
        } else {
            self.cur_dist += (want - self.cur_dist) * (dt * 3.0).min(1.0);
        }
        let mut eye = origin + dir * self.cur_dist;
        if eye.y < 0.3 {
            eye.y = 0.3;
        }
        self.eye = eye;
        self.target = match lock {
            Some(lp) => origin.lerp(lp + Vec3::Y * 1.0, 0.3),
            None => origin,
        };
        // directional kick from blows: the view is knocked along the blow and springs back
        self.kick_off *= (-dt * 11.0).exp();
        self.eye += self.kick_off * shake_scale.min(1.0);
        self.target += self.kick_off * 0.6 * shake_scale.min(1.0);
        // trauma-based shake
        self.trauma = (self.trauma - dt * 1.6).max(0.0);
        let s = self.trauma * self.trauma * 0.35 * shake_scale;
        if s > 0.0 {
            let n = |k: f32| (self.t * (31.0 + k * 7.0)).sin() * (self.t * (17.0 + k * 3.0)).cos();
            self.eye += Vec3::new(n(1.0), n(2.0), n(3.0)) * s;
            self.target += Vec3::new(n(4.0), n(5.0), n(6.0)) * s * 0.5;
        }
        self.snap = false;
    }

    /// Knock the view along `dir` (world space) by `amount` metres.
    pub fn kick(&mut self, dir: Vec3, amount: f32) {
        self.kick_off = (self.kick_off + dir.normalize_or_zero() * amount).clamp_length_max(0.35);
    }

    pub fn view(&self) -> Mat4 {
        look_at(self.eye, self.target, Vec3::Y)
    }
    pub fn proj(&self, aspect: f32) -> Mat4 {
        perspective(self.fov.to_radians(), aspect, 0.1, 400.0)
    }
}
