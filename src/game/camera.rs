//! Cinematic third-person camera: critically damped follow with velocity look-ahead, an
//! over-the-shoulder offset, distance and lens that breathe with what the player is doing
//! (sprinting pulls out and widens, a charging heavy pushes in, a fight frames the threats),
//! handheld sway, directional impact kicks with a little roll, rotational trauma shake, and a
//! kill cam that eases toward a finishing blow. Wall collision and cutscene override as before.
use crate::physics::{Statics, OPAQUE};
use crate::util::{angle_lerp, look_at, perspective, yaw_dir};
use glam::{Mat4, Quat, Vec3};

/// What the camera should react to this frame.
#[derive(Clone, Copy, Default, Debug)]
pub struct CamCtx {
    pub vel: Vec3,
    pub sprint: bool,
    /// heavy attack charge 0..1
    pub charge: f32,
    /// nearby threats, 0 (none) .. 1 (a crowd)
    pub combat: f32,
    /// centre of the nearby threats, for framing
    pub threat: Option<Vec3>,
}

pub struct Camera {
    pub yaw: f32,
    pub pitch: f32,
    pub dist: f32,
    pub eye: Vec3,
    pub target: Vec3,
    pub trauma: f32,
    pub snap: bool,
    /// base field of view from the settings
    pub fov: f32,
    pub cur_dist: f32,
    pub focus: Vec3,
    /// cutscene override (eye, target)
    pub over: Option<(Vec3, Vec3)>,
    /// distance the lens is focused at (for depth of field)
    pub focus_dist: f32,
    t: f32,
    focus_vel: Vec3,
    look_ahead: Vec3,
    fov_off: f32,
    dist_off: f32,
    /// impact kick: positional offset and roll, both spring back
    kick: Vec3,
    kick_vel: Vec3,
    roll: f32,
    roll_vel: f32,
    /// kill cam: subject and seconds remaining
    cine: Option<(Vec3, f32)>,
    cine_k: f32,
}

/// Critically damped spring toward `target` (smoothing time `t`).
fn spring(x: &mut Vec3, v: &mut Vec3, target: Vec3, t: f32, dt: f32) {
    let w = 2.0 / t.max(1e-3);
    let k = w * dt;
    let e = 1.0 / (1.0 + k + 0.48 * k * k + 0.235 * k * k * k);
    let d = *x - target;
    let tmp = (*v + d * w) * dt;
    *v = (*v - tmp * w) * e;
    *x = target + (d + tmp) * e;
}

fn spring1(x: &mut f32, v: &mut f32, target: f32, t: f32, dt: f32) {
    let (mut a, mut b) = (Vec3::new(*x, 0.0, 0.0), Vec3::new(*v, 0.0, 0.0));
    spring(&mut a, &mut b, Vec3::new(target, 0.0, 0.0), t, dt);
    *x = a.x;
    *v = b.x;
}

fn noise1(t: f32, seed: f32) -> f32 {
    // smooth layered sines: handheld drift without a visible period
    (t * 0.71 + seed).sin() * 0.5 + (t * 1.37 + seed * 2.3).sin() * 0.3 + (t * 2.93 + seed * 4.1).sin() * 0.2
}

impl Camera {
    pub fn new() -> Camera {
        Camera {
            yaw: 0.0,
            pitch: 0.28,
            dist: 4.4,
            eye: Vec3::ZERO,
            target: Vec3::ZERO,
            trauma: 0.0,
            snap: true,
            fov: 70.0,
            cur_dist: 4.4,
            focus: Vec3::ZERO,
            over: None,
            focus_dist: 4.4,
            t: 0.0,
            focus_vel: Vec3::ZERO,
            look_ahead: Vec3::ZERO,
            fov_off: 0.0,
            dist_off: 0.0,
            kick: Vec3::ZERO,
            kick_vel: Vec3::ZERO,
            roll: 0.0,
            roll_vel: 0.0,
            cine: None,
            cine_k: 0.0,
        }
    }

    /// A blow lands: shove the camera along `dir` and roll it a little.
    pub fn impact(&mut self, dir: Vec3, strength: f32) {
        let d = Vec3::new(dir.x, dir.y * 0.3, dir.z).normalize_or_zero();
        self.kick_vel += d * strength * 2.2 + Vec3::NEG_Y * strength * 0.8;
        // roll away from the side the blow comes from
        let right = Vec3::new(yaw_dir(self.yaw).z, 0.0, -yaw_dir(self.yaw).x);
        self.roll_vel += -d.dot(right) * strength * 0.55;
    }

    /// Ease toward a finishing blow for `dur` seconds.
    pub fn kill_cam(&mut self, subject: Vec3, dur: f32) {
        self.cine = Some((subject, dur));
    }

    /// `look` is the mouse/stick delta; `lock` the lock-on target position.
    #[allow(clippy::too_many_arguments)]
    pub fn update(&mut self, dt: f32, look: glam::Vec2, invert: bool, player: Vec3, lock: Option<Vec3>, area: &dyn Statics, shake_scale: f32, ctx: CamCtx) {
        self.t += dt;
        if let Some((e, t)) = self.over {
            self.eye = e;
            self.target = t;
            self.focus_dist = (t - e).length();
            self.trauma = (self.trauma - dt * 1.5).max(0.0);
            return;
        }
        // look-ahead: lead the player's motion a little so they move into the frame
        let hv = Vec3::new(ctx.vel.x, 0.0, ctx.vel.z);
        let ahead = hv * if ctx.sprint { 0.28 } else { 0.18 };
        self.look_ahead = self.look_ahead.lerp(ahead, (dt * 2.5).min(1.0));
        let focus = player + Vec3::Y * 1.5 + self.look_ahead;
        if self.snap {
            self.focus = focus;
            self.focus_vel = Vec3::ZERO;
        } else {
            // the horizontal follow is tight, the vertical lazier (stairs, kerbs, landings)
            let mut f = self.focus;
            spring(&mut f, &mut self.focus_vel, focus, 0.09, dt);
            let lazy_y = self.focus.y + (focus.y - self.focus.y) * (dt * 6.0).min(1.0);
            self.focus = Vec3::new(f.x, lazy_y, f.z);
        }
        match lock {
            Some(lp) => {
                let d = lp - player;
                let want = d.x.atan2(d.z);
                self.yaw = angle_lerp(self.yaw, want, (dt * 5.0).min(1.0));
                let dist = Vec3::new(d.x, 0.0, d.z).length();
                let want_p = (0.2 + 0.22 * (1.0 - (dist / 10.0).min(1.0))).clamp(0.12, 0.5) + ((lp.y - player.y) * 0.04);
                self.pitch += (want_p - self.pitch) * (dt * 3.5).min(1.0);
            }
            None => {
                self.yaw -= look.x;
                self.pitch += look.y * if invert { -1.0 } else { 1.0 };
            }
        }
        self.pitch = self.pitch.clamp(-0.5, 1.2);

        // lens and distance breathe with the action
        let want_fov = if ctx.sprint { 7.0 } else { 0.0 } - ctx.charge * 6.0 + ctx.combat * 3.0;
        let want_dist = if ctx.sprint { 0.45 } else { 0.0 } - ctx.charge * 0.45 + ctx.combat * 0.9;
        self.fov_off += (want_fov - self.fov_off) * (dt * 2.2).min(1.0);
        self.dist_off += (want_dist - self.dist_off) * (dt * 1.6).min(1.0);

        // kill cam: push in low and close, toward the subject
        let mut cine_yaw = None;
        if let Some((s, left)) = self.cine {
            let left = left - dt;
            self.cine = if left > 0.0 { Some((s, left)) } else { None };
            let d = s - player;
            cine_yaw = Some(d.x.atan2(d.z) + 0.55);
        }
        let k_want = if self.cine.is_some() { 1.0 } else { 0.0 };
        self.cine_k += (k_want - self.cine_k) * (dt * if k_want > 0.5 { 5.0 } else { 2.0 }).min(1.0);
        let yaw = match cine_yaw {
            Some(cy) => angle_lerp(self.yaw, cy, self.cine_k * 0.6),
            None => self.yaw,
        };
        let pitch = self.pitch + (0.08 - self.pitch) * self.cine_k * 0.7;

        let back = -yaw_dir(yaw);
        let dir = (back * pitch.cos() + Vec3::Y * pitch.sin()).normalize();
        let right = Vec3::new(-back.z, 0.0, back.x);
        let shoulder = right * -(0.55 - 0.15 * self.cine_k);
        let origin = self.focus + shoulder;
        let base = (self.dist + self.dist_off) * (1.0 - 0.35 * self.cine_k);
        // collision: pull in toward the player when walls intervene
        let mut want = base;
        if let Some(hit) = area.raycast(origin, dir, base + 0.3, OPAQUE) {
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
        // frame the fight: aim between the player and the threats (or the lock target)
        let frame = match (lock, ctx.threat) {
            (Some(lp), _) => Some((lp + Vec3::Y * 1.0, 0.3)),
            (None, Some(tp)) => Some((tp + Vec3::Y * 1.0, 0.12 * ctx.combat)),
            _ => None,
        };
        let mut target = match frame {
            Some((p, k)) => origin.lerp(p, k),
            None => origin,
        };
        if let Some((s, _)) = self.cine {
            target = target.lerp(s + Vec3::Y * 0.9, self.cine_k * 0.5);
        }

        // impact kick (springs back), handheld sway, rotational trauma shake
        spring(&mut self.kick, &mut self.kick_vel, Vec3::ZERO, 0.12, dt);
        spring1(&mut self.roll, &mut self.roll_vel, 0.0, 0.16, dt);
        eye += self.kick * 0.18 * shake_scale;
        target += self.kick * 0.1 * shake_scale;
        let sway = 0.012 * (1.0 + ctx.combat * 0.6);
        target += (right * noise1(self.t, 1.3) + Vec3::Y * noise1(self.t, 7.1) * 0.7) * sway * self.cur_dist;
        self.trauma = (self.trauma - dt * 1.4).max(0.0);
        let s = self.trauma * self.trauma * shake_scale;
        if s > 0.0 {
            // low-frequency, rotational: the whole frame lurches instead of buzzing
            let f = 9.0;
            let n = |k: f32| noise1(self.t * f, k * 3.7);
            let ang = Vec3::new(n(1.0), n(2.0), 0.0) * 0.05 * s;
            let to = target - eye;
            let q = Quat::from_axis_angle(Vec3::Y, ang.x) * Quat::from_axis_angle(right, ang.y);
            target = eye + q * to;
            self.roll += n(3.0) * 0.004 * s;
        }
        self.eye = eye;
        self.target = target;
        self.focus_dist = (player + Vec3::Y * 1.2 - eye).length();
        self.snap = false;
    }

    /// Current field of view in degrees.
    pub fn fov_now(&self) -> f32 {
        (self.fov + self.fov_off - self.cine_k * 12.0).clamp(35.0, 110.0)
    }

    pub fn view(&self) -> Mat4 {
        let f = (self.target - self.eye).normalize_or(Vec3::Z);
        let up = Quat::from_axis_angle(f, self.roll) * Vec3::Y;
        look_at(self.eye, self.target, up)
    }
    pub fn proj(&self, aspect: f32) -> Mat4 {
        perspective(self.fov_now().to_radians(), aspect, 0.1, 400.0)
    }
}
