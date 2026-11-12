//! Short in-engine cutscenes: the opening, each boss's introduction and death, and the ending.
//! They drive the camera, letterbox and captions while the world keeps simulating.
use super::{Game, Screen};
use crate::audio::Sfx;
use crate::util::{ease_out, smooth};
use crate::world::{pit_pos, rift_site, CS};
use glam::Vec3;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum CutKind {
    Opening,
    BossIntro(usize),
    BossDeath(usize),
    Ending,
}

#[derive(Clone, Debug)]
pub struct Cutscene {
    pub kind: CutKind,
    pub t: f32,
    pub dur: f32,
    pub lines: Vec<String>,
    pub line_time: f32,
    pub title: Option<(String, String)>,
}

impl Game {
    pub fn start_cutscene(&mut self, kind: CutKind) {
        let (lines, line_time, title) = match kind {
            CutKind::Opening => (self.t.text.opening.clone(), 3.4, None),
            CutKind::BossIntro(i) => {
                let b = &self.t.bosses[i];
                (b.intro.clone(), 2.6, Some((b.name.clone(), b.title.clone())))
            }
            CutKind::BossDeath(_) => (vec![], 1.0, None),
            CutKind::Ending => (self.t.text.ending.clone(), 3.8, None),
        };
        let dur = match kind {
            CutKind::Opening => lines.len() as f32 * line_time + 3.0,
            CutKind::BossIntro(_) => (lines.len() as f32 * line_time).max(4.0) + 1.5,
            CutKind::BossDeath(_) => 4.5,
            CutKind::Ending => lines.len() as f32 * line_time + 4.0,
        };
        if let CutKind::BossIntro(_) = kind {
            self.audio.play(Sfx::BossRoar, 0.9, 0.7);
        }
        self.cutscene = Some(Cutscene { kind, t: 0.0, dur, lines, line_time, title });
        self.screen = Screen::None;
    }

    /// Advance the active cutscene; returns true while one is running.
    pub fn cutscene_tick(&mut self, dt: f32, skip: bool) -> bool {
        let Some(mut c) = self.cutscene.take() else {
            self.cam.over = None;
            return false;
        };
        c.t += dt;
        let k = (c.t / c.dur).clamp(0.0, 1.0);
        let pp = self.ppos();
        let (eye, target) = match c.kind {
            CutKind::Opening => {
                // sweep from above the burning city down to the compatible at the stadium pit
                let start = pit_pos(0) + Vec3::new(-120.0, 70.0, 260.0);
                let mid = pit_pos(0) + Vec3::new(-40.0, 32.0, 70.0);
                let end = pp + Vec3::new(-3.0, 2.2, 4.5);
                let e = if k < 0.7 { start.lerp(mid, smooth(k / 0.7)) } else { mid.lerp(end, smooth((k - 0.7) / 0.3)) };
                let t0 = pit_pos(0) + Vec3::new(60.0, 10.0, -40.0);
                let t = if k < 0.6 { t0.lerp(pit_pos(0), smooth(k / 0.6)) } else { pit_pos(0).lerp(pp + Vec3::Y * 1.5, smooth((k - 0.6) / 0.4)) };
                (e, t)
            }
            CutKind::BossIntro(_) | CutKind::BossDeath(_) => {
                let bp = self.boss.and_then(|b| self.w.body.get(b)).map(|b| b.pos).unwrap_or(self.area.arena.0);
                let h = self.boss.and_then(|b| self.w.body.get(b)).map(|b| b.height).unwrap_or(4.0);
                let a = c.t * 0.35 + 0.8;
                let (ac, ar) = self.area.arena;
                let mut r = 7.0 + h + ease_out(k) * 2.0;
                if ar > 0.0 {
                    r = r.min((ar - 2.0 - (bp - ac).length()).max(4.0));
                }
                (bp + Vec3::new(a.cos() * r, h * 0.6 + 1.5, a.sin() * r), bp + Vec3::Y * h * 0.6)
            }
            CutKind::Ending => {
                let site = rift_site(4);
                let a = c.t * 0.12;
                let r = 14.0 + k * 30.0;
                (site + Vec3::new(a.cos() * r, 4.0 + k * 40.0, a.sin() * r), site + Vec3::Y * 2.0)
            }
        };
        self.cam.over = Some((eye, target));
        // captions
        let li = (c.t / c.line_time) as usize;
        self.caption = c.lines.get(li).cloned();
        self.cut_title = c.title.clone().filter(|_| c.t > 0.5);
        if let CutKind::Ending = c.kind {
            self.fade = smooth((k - 0.82) / 0.18);
        } else if let CutKind::Opening = c.kind {
            self.fade = 1.0 - smooth(c.t / 2.0);
        }
        let done = c.t >= c.dur || (skip && c.t > 0.6);
        if done {
            self.caption = None;
            self.cut_title = None;
            self.cam.over = None;
            self.cam.snap = true;
            self.fade = 0.0;
            match c.kind {
                CutKind::Opening => {
                    self.prof.intro_seen = true;
                    self.msg("A SOLDIER WAVES YOU TOWARD THE BURN PIT. WASD MOVE, MOUSE LOOK.");
                    self.tutorial_t = 0.0;
                }
                CutKind::BossIntro(_) => self.boss_awake = true,
                CutKind::Ending => {
                    self.screen = Screen::Credits;
                    self.ui.t = 0.0;
                }
                _ => {}
            }
            return false;
        }
        self.cutscene = Some(c);
        true
    }
}
