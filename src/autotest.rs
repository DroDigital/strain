//! Scripted test driver (STRAIN_AUTOTEST=<scenario>): injects input, teleports and captures
//! screenshots so gameplay can be verified headlessly in CI or on a build machine.
use crate::game::{Game, Screen};
use crate::input::{Action, Input};
use crate::render::Renderer;
use glam::Vec3;

pub enum Step {
    Wait(u32),
    Hold(Action, u32),
    Tap(Action),
    Look(f32, f32),
    Shot(&'static str),
    Do(fn(&mut Game)),
    Log(&'static str),
}

pub struct AutoTest {
    steps: Vec<Step>,
    i: usize,
    t: u32,
    dir: String,
    held: Option<Action>,
    frames: u64,
}

fn teleport(g: &mut Game, p: Vec3) {
    g.rebuild_city(p);
}

impl AutoTest {
    pub fn from_env() -> Option<AutoTest> {
        let name = std::env::var("STRAIN_AUTOTEST").ok()?;
        let dir = std::env::var("STRAIN_SHOT_DIR").unwrap_or_else(|_| ".".into());
        use Step::*;
        let steps: Vec<Step> = match name.as_str() {
            "title" => vec![Wait(40), Shot("title")],
            "opening" => vec![Wait(20), Tap(Action::UiDown), Tap(Action::UiUp), Do(|g| g.new_game()), Wait(60), Shot("opening_a"), Wait(200), Shot("opening_b")],
            _ => vec![
                Wait(10),
                Do(|g| {
                    std::env::set_var("STRAIN_SKIP_INTRO", "1");
                    g.new_game();
                }),
                Wait(30),
                Shot("fob"),
                Hold(Action::MoveF, 60),
                Shot("walk"),
                Tap(Action::Light),
                Wait(8),
                Shot("swing"),
                Wait(30),
                Do(|g| {
                    let p = crate::world::chunk_center(1, (2, 2));
                    teleport(g, p + Vec3::new(-24.0, 0.0, -24.0));
                }),
                Wait(60),
                Shot("market"),
                Do(|g| {
                    // fight whatever is near
                    let p = g.ppos();
                    let e = g.spawn_archetype("rusher", p + Vec3::new(0.0, 0.0, 4.0), 2, 6, None);
                    if let Some(ai) = g.w.ai.get_mut(e) {
                        ai.target = Some(g.player);
                    }
                    g.pl.lock = Some(e);
                }),
                Wait(20),
                Tap(Action::Light),
                Wait(10),
                Tap(Action::Light),
                Wait(5),
                Shot("combat"),
                Wait(10),
                Tap(Action::Cast),
                Wait(14),
                Shot("cast"),
                Do(|g| g.add_biomass(80.0)),
                Wait(5),
                Shot("levelup"),
                Tap(Action::UiAccept),
                Wait(5),
                Tap(Action::Menu),
                Wait(5),
                Shot("inventory"),
                Tap(Action::TabNext),
                Tap(Action::TabNext),
                Wait(3),
                Shot("map"),
                Tap(Action::UiBack),
                Do(|g| {
                    let p = crate::world::chunk_center(4, (3, 3));
                    teleport(g, p + Vec3::new(-24.0, 0.0, -22.0));
                }),
                Wait(60),
                Shot("bloom"),
                Do(|g| {
                    let p = crate::world::chunk_center(2, (2, 2));
                    teleport(g, p + Vec3::new(-24.0, 0.0, -24.0));
                    g.tod = 0.45;
                }),
                Wait(60),
                Shot("tenements_day"),
                Do(|g| g.enter_rift(0)),
                Wait(60),
                Shot("rift_gut"),
                Do(|g| {
                    let (c, _) = g.area.arena;
                    g.place_player(c + Vec3::new(0.0, 0.0, -8.0));
                }),
                Wait(90),
                Shot("boss_intro"),
                Wait(500),
                Shot("boss_fight"),
                Do(|g| g.enter_rift(1)),
                Wait(40),
                Shot("rift_lattice"),
                Do(|g| g.enter_rift(2)),
                Wait(40),
                Shot("rift_kiln"),
                Do(|g| g.enter_rift(3)),
                Wait(40),
                Shot("rift_choir"),
                Do(|g| {
                    g.prof.bosses = [true, true, true, true, false];
                    g.enter_rift(4);
                }),
                Wait(40),
                Shot("rift_final"),
                Do(|g| g.exit_rift()),
                Wait(30),
                Do(|g| {
                    let pe = g.player;
                    g.deal(pe, None, 99999.0, 0.0, Vec3::X, crate::game::combat::DmgKind::True, 0.0);
                }),
                Wait(90),
                Shot("dead"),
                Tap(Action::UiAccept),
                Wait(30),
                Shot("respawn"),
                Log("autotest complete"),
            ],
        };
        Some(AutoTest { steps, i: 0, t: 0, dir, held: None, frames: 0 })
    }

    /// Advance the script by one frame. Returns true when finished.
    pub fn step(&mut self, g: &mut Game, input: &mut Input, r: &mut Renderer) -> bool {
        self.frames += 1;
        if let Some(a) = self.held.take() {
            input.inject(a, false);
        }
        if g.screen == Screen::Loading {
            return false;
        }
        let Some(s) = self.steps.get(self.i) else { return true };
        let mut next = true;
        match s {
            Step::Wait(n) => {
                self.t += 1;
                next = self.t >= *n;
            }
            Step::Hold(a, n) => {
                input.inject(*a, true);
                self.t += 1;
                if self.t >= *n {
                    self.held = Some(*a);
                } else {
                    next = false;
                }
            }
            Step::Tap(a) => {
                input.inject(*a, true);
                self.held = Some(*a);
            }
            Step::Look(x, y) => input.look += glam::Vec2::new(*x, *y),
            Step::Shot(name) => {
                r.capture = Some(format!("{}/{}.ppm", self.dir, name).into());
                eprintln!("autotest: shot {name} (screen {:?}, pos {:?}, hp {:?})", g.screen, g.ppos(), g.w.actor.get(g.player).map(|a| a.hp));
            }
            Step::Do(f) => f(g),
            Step::Log(m) => eprintln!("autotest: {m} after {} frames", self.frames),
        }
        if next {
            self.i += 1;
            self.t = 0;
        }
        false
    }
}
