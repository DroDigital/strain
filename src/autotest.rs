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
    Shot(&'static str),
    Do(fn(&mut Game)),
    Log(&'static str),
    /// let a simple combat bot play for n frames
    Bot(u32),
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
            "duel" => vec![
                Wait(10),
                Do(|g| {
                    std::env::set_var("STRAIN_SKIP_INTRO", "1");
                    g.new_game();
                    // a level-appropriate character for the first rift: level 12, rare gear
                    g.pl.life.level = 12;
                    let mut r = crate::util::Rng::with_seed(9);
                    use crate::game::loot::{gen_item, Rarity};
                    use crate::data::Source;
                    g.pl.gear.eq[0] = Some(gen_item(&g.t, &mut r, 12, &[Source::Military], Rarity::Rare, Some("trench_cleaver")));
                    g.pl.gear.eq[1] = Some(gen_item(&g.t, &mut r, 12, &[Source::Military], Rarity::Rare, Some("riot_shield")));
                    g.pl.gear.eq[2] = Some(gen_item(&g.t, &mut r, 12, &[Source::Military], Rarity::Rare, Some("riot_helmet")));
                    g.pl.gear.eq[3] = Some(gen_item(&g.t, &mut r, 12, &[Source::Military], Rarity::Rare, Some("flak_vest")));
                    g.pl.gear.eq[4] = Some(gen_item(&g.t, &mut r, 12, &[Source::Military], Rarity::Rare, Some("combat_trousers")));
                    for m in ["keratin_knuckles", "carrion_appetite", "thick_hide", "clot_factor", "nerve_lattice", "bone_spurs"] {
                        g.pl.life.mutations.push(m.into());
                    }
                    g.refresh_player();
                    let pe = g.player;
                    if let Some(a) = g.w.actor.get_mut(pe) { a.hp = a.max_hp; }
                    g.pl.heal_charges = g.pl.stats.heal_charges;
                    let p = crate::world::chunk_center(1, (2, 2));
                    teleport(g, p + Vec3::new(-24.0, 0.0, -24.0));
                    let pp = g.ppos();
                    for k in 0..4 {
                        let e = g.spawn_archetype(["rusher", "brute", "leaper", "spitter"][k], pp + Vec3::new(k as f32 * 2.0 - 3.0, 0.0, 9.0), 2, 7, None);
                        if let Some(ai) = g.w.ai.get_mut(e) { ai.target = Some(pe); ai.alert = 30.0; }
                    }
                    eprintln!("autotest: duel start hp {:.0} armor {:.0} dmg {:.2}", g.pl.stats.max_hp, g.pl.stats.armor, g.pl.stats.dmg);
                }),
                Bot(if std::env::var("STRAIN_BOTLOG").is_ok() { 1 } else { 1800 }),
                Shot("duel_street"),
                Do(|g| {
                    let alive = g.w.foe.iter().filter(|(id, f)| f.stage > 0 && g.w.actor.get(g.w.ent(*id)).map(|a| a.alive && a.team == crate::game::comp::Team::Infected).unwrap_or(false) && (g.w.body.get(g.w.ent(*id)).unwrap().pos - g.ppos()).length() < 30.0).count();
                    eprintln!("autotest: street fight over: player alive {} hp {:.0} deaths {} hostiles left nearby {} level {}", g.w.actor.get(g.player).map(|a| a.alive).unwrap_or(false), g.w.actor.get(g.player).map(|a| a.hp).unwrap_or(0.0), g.prof.deaths, alive, g.pl.life.level);
                    if g.screen == crate::game::Screen::Dead { g.respawn(); }
                    g.pl.life.level = 12;
                    g.pl.life.mutations = ["keratin_knuckles", "carrion_appetite", "thick_hide", "clot_factor", "nerve_lattice", "bone_spurs"].iter().map(|s| s.to_string()).collect();
                    g.refresh_player();
                    g.bot_stop_on_death = true;
                    g.screen = crate::game::Screen::None;
                    g.mutation_choices.clear();
                    g.pl.life.pending_levels = 0;
                    let pe = g.player;
                    if let Some(a) = g.w.actor.get_mut(pe) { a.hp = a.max_hp; }
                    g.pl.heal_charges = g.pl.stats.heal_charges;
                    g.enter_rift(0);
                    let (c, _) = g.area.arena;
                    g.place_player(c + Vec3::new(0.0, 0.0, -7.0));
                }),
                Wait(5),
                Do(|g| { g.progress_tick(); g.cutscene = None; g.boss_awake = true; }),
                Bot(if std::env::var("STRAIN_BOTLOG").is_ok() { 900 } else { 5400 }),
                Shot("duel_boss"),
                Do(|g| {
                    let bf = g.boss.and_then(|b| g.w.actor.get(b)).map(|a| a.frac()).unwrap_or(0.0);
                    eprintln!("autotest: boss fight: boss hp {:.0}% killed {} player alive {} hp {:.0} heals left {}", bf * 100.0, g.prof.bosses[0], g.w.actor.get(g.player).map(|a| a.alive).unwrap_or(false), g.w.actor.get(g.player).map(|a| a.hp).unwrap_or(0.0), g.pl.heal_charges);
                }),
                Log("duel done"),
            ],
            "spells" => {
                let mut v = vec![
                    Wait(10),
                    Do(|g| {
                        std::env::set_var("STRAIN_SKIP_INTRO", "1");
                        g.new_game();
                        let p = crate::world::chunk_center(1, (2, 2));
                        teleport(g, p + Vec3::new(-24.0, 0.0, -24.0));
                    }),
                    Wait(30),
                ];
                for si in 0..18usize {
                    v.push(Do(match si {
                        _ => |g: &mut Game| {
                            let p = g.ppos();
                            for k in 0..3 {
                                let e = g.spawn_archetype("rusher", p + Vec3::new(k as f32 * 2.0 - 2.0, 0.0, 6.0), 1, 5, None);
                                if let Some(a) = g.w.actor.get_mut(e) { a.max_hp = 5000.0; a.hp = 5000.0; }
                            }
                            let n = g.spell_test;
                            let pe = g.player;
                            g.cast_spell(pe, n, true);
                            eprintln!("autotest: cast {} strain {:.0}", g.t.spells[n].name, g.pl.strain);
                            g.spell_test += 1;
                            if let Some(a) = g.w.actor.get_mut(pe) { a.hp = a.max_hp; }
                        },
                    }));
                    v.push(Wait(25));
                    if si % 3 == 2 {
                        v.push(Shot(["sp_a", "sp_b", "sp_c", "sp_d", "sp_e", "sp_f"][si / 3]));
                    }
                }
                v.push(Log("spells done"));
                v
            }
            "stress" => vec![
                Wait(10),
                Do(|g| {
                    std::env::set_var("STRAIN_SKIP_INTRO", "1");
                    g.new_game();
                    // every mutation, one by one, through the real code path
                    for i in 0..g.t.mutations.len() {
                        let m = &g.t.mutations[i];
                        if m.excl.is_some() && g.pl.life.mutations.iter().any(|o| g.t.mutations.iter().find(|x| &x.id == o).and_then(|x| x.excl.clone()) == m.excl) {
                            continue;
                        }
                        g.pl.life.pending_levels = 1;
                        g.choose_mutation(i);
                    }
                    eprintln!("autotest: {} mutations held, hp {:.0}", g.pl.life.mutations.len(), g.pl.stats.max_hp);
                    let p = crate::world::chunk_center(4, (3, 3));
                    teleport(g, p + Vec3::new(-20.0, 0.0, -20.0));
                }),
                Wait(30),
                Shot("all_mutations"),
                Hold(Action::Light, 4),
                Wait(10),
                Tap(Action::Dodge),
                Wait(10),
                Tap(Action::Heavy),
                Wait(40),
                Tap(Action::Block),
                Tap(Action::Heal),
                Wait(60),
                Shot("all_mutations_b"),
                Do(|g| {
                    // loot: every unique, rolled and equipped where possible
                    let mut r = crate::util::Rng::with_seed(5);
                    let ids: Vec<String> = g.t.uniques.iter().map(|u| u.id.clone()).collect();
                    for id in ids {
                        let it = crate::game::loot::make_unique(&g.t, &mut r, 20, &id);
                        let _ = crate::game::loot::describe(&g.t, &it);
                        let p = g.ppos();
                        g.drop_item(p + Vec3::new(0.0, 0.5, 2.0), it);
                    }
                    for _ in 0..300 {
                        let rar = crate::game::loot::roll_rarity(&mut r, 1.0);
                        let it = crate::game::loot::gen_item(&g.t, &mut r, 25, &[crate::data::Source::Military, crate::data::Source::Rift, crate::data::Source::Biological, crate::data::Source::Civilian], rar, None);
                        let _ = it.name(&g.t);
                    }
                    // spawn every archetype at every stage with random affixes
                    let p = g.ppos();
                    let ids: Vec<String> = g.t.archetypes.iter().map(|a| a.id.clone()).collect();
                    for (k, id) in ids.iter().enumerate() {
                        for st in 1..=3u8 {
                            g.spawn_archetype(id, p + Vec3::new(k as f32 * 2.5 - 12.0, 0.0, 8.0 + st as f32 * 3.0), st, 25, None);
                        }
                    }
                    for i in 0..g.t.apex.len() {
                        g.spawn_apex(i, p + Vec3::new(i as f32 * 4.0 - 12.0, 0.0, -10.0), None);
                    }
                }),
                Wait(30),
                Shot("menagerie"),
                Wait(300),
                Shot("menagerie_b"),
                Log("stress done"),
            ],
            "progress" => vec![
                Wait(10),
                Do(|g| {
                    std::env::set_var("STRAIN_SKIP_INTRO", "1");
                    g.new_game();
                    g.add_biomass(400.0);
                }),
                Wait(5),
                Tap(Action::UiAccept),
                Wait(3),
                Tap(Action::UiAccept),
                Wait(3),
                Tap(Action::UiAccept),
                Wait(3),
                Tap(Action::UiAccept),
                Wait(3),
                Do(|g| eprintln!("autotest: level {} mutations {:?}", g.pl.life.level, g.pl.life.mutations)),
                // die: a husk is left with the build
                Do(|g| { let pe = g.player; g.deal(pe, None, 1e6, 0.0, Vec3::X, crate::game::combat::DmgKind::True, 0.0); }),
                Wait(100),
                Tap(Action::UiAccept),
                Wait(20),
                Do(|g| eprintln!("autotest: after death level {} husk {}", g.pl.life.level, g.prof.husk.is_some())),
                Do(|g| {
                    let h = g.prof.husk.as_ref().map(|h| Vec3::from(h.pos)).unwrap();
                    g.place_player(h + Vec3::new(0.5, 0.0, 0.5));
                }),
                Wait(10),
                Shot("husk"),
                Tap(Action::Interact),
                Wait(10),
                Do(|g| eprintln!("autotest: reclaimed level {} mutations {} husk {}", g.pl.life.level, g.pl.life.mutations.len(), g.prof.husk.is_some())),
                // die twice: the old husk rises
                Do(|g| { let pe = g.player; g.deal(pe, None, 1e6, 0.0, Vec3::X, crate::game::combat::DmgKind::True, 0.0); }),
                Wait(100),
                Tap(Action::UiAccept),
                Wait(10),
                Do(|g| { let pe = g.player; g.deal(pe, None, 1e6, 0.0, Vec3::X, crate::game::combat::DmgKind::True, 0.0); }),
                Wait(100),
                Tap(Action::UiAccept),
                Wait(10),
                Do(|g| {
                    let p = match g.risen_husk.clone() {
                        Some((p, _)) => p,
                        None => g.w.foe.iter().find(|(_, f)| f.husk).map(|(id, _)| g.w.body.get(g.w.ent(id)).unwrap().pos).expect("a risen husk"),
                    };
                    g.place_player(p + Vec3::new(6.0, 0.0, 0.0));
                }),
                Wait(30),
                Shot("risen_husk"),
                Do(|g| eprintln!("autotest: risen husks alive {}", g.w.foe.iter().filter(|(_, f)| f.husk).count())),
                // vendor + stash
                Do(|g| { g.restock_vendor(); g.pl.gear.scrip = 5000; g.screen = crate::game::Screen::Vendor; g.ui.focus = 0; g.ui.sel = 0; }),
                Tap(Action::UiAccept),
                Wait(3),
                Shot("vendor"),
                Tap(Action::UiBack),
                Do(|g| { g.screen = crate::game::Screen::Stash; g.ui.focus = 1; g.ui.cursor = (0, 0); }),
                Tap(Action::UiAccept),
                Wait(3),
                Shot("stash"),
                Do(|g| eprintln!("autotest: stash items {}", g.prof.stash.items.len())),
                Tap(Action::UiBack),
                // bosses: kill all four, then the final
                Do(|g| {
                    for i in 0..4 {
                        g.enter_rift(i);
                        let (c, _) = g.area.arena;
                        g.place_player(c + Vec3::new(0.0, 0.0, -6.0));
                        g.progress_tick();
                        let b = g.boss.expect("boss spawned");
                        let pe = g.player;
                        g.cutscene = None;
                        g.deal(b, Some(pe), 1e7, 0.0, Vec3::X, crate::game::combat::DmgKind::True, 0.0);
                        eprintln!("autotest: boss {} dead {:?} pits unlocked {}", i, g.prof.bosses, g.prof.district_unlocked(i + 2));
                        g.cutscene = None;
                    }
                }),
                Wait(10),
                Shot("boss_dead"),
                Do(|g| g.exit_rift()),
                Wait(10),
                Do(|g| {
                    g.enter_rift(4);
                    let (c, _) = g.area.arena;
                    g.place_player(c + Vec3::new(0.0, 0.0, -6.0));
                }),
                Wait(60),
                Shot("final_intro"),
                Wait(300),
                Do(|g| {
                    if let Some(b) = g.boss {
                        let pe = g.player;
                        g.deal(b, Some(pe), 1e7, 0.0, Vec3::X, crate::game::combat::DmgKind::True, 0.0);
                    }
                }),
                Wait(200),
                Shot("ending"),
                Wait(500),
                Shot("credits"),
                Tap(Action::UiAccept),
                Wait(5),
                Do(|g| { eprintln!("autotest: ended {} screen {:?}", g.prof.ended, g.screen); g.save(); g.to_title(); }),
                Wait(10),
                Do(|g| { g.continue_game(); eprintln!("autotest: continued, bosses {:?} pits {:?} level {}", g.prof.bosses, g.prof.pits.iter().filter(|p| **p).count(), g.pl.life.level); }),
                Do(|g| { g.prof.pits = vec![true; 12]; g.fast_travel(11); }),
                Wait(30),
                Shot("ground_zero_pit"),
                Log("progress done"),
            ],
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
        for a in [Action::MoveF, Action::MoveB, Action::MoveL, Action::MoveR, Action::Block, Action::Light, Action::Dodge, Action::Heal, Action::LockOn, Action::UiAccept] {
            if !matches!(s, Step::Bot(_)) {
                break;
            }
            input.inject(a, false);
        }
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
            Step::Shot(name) => {
                r.capture = Some(format!("{}/{}.ppm", self.dir, name).into());
                eprintln!("autotest: shot {name} (screen {:?}, pos {:?}, hp {:?})", g.screen, g.ppos(), g.w.actor.get(g.player).map(|a| a.hp));
            }
            Step::Do(f) => f(g),
            Step::Log(m) => eprintln!("autotest: {m} after {} frames", self.frames),
            Step::Bot(n) => {
                bot(g, input, self.t);
                self.t += 1;
                next = self.t >= *n;
            }
        }
        if next {
            self.i += 1;
            self.t = 0;
        }
        false
    }
}

/// A crude but honest combat bot: lock on, keep range, dodge telegraphed swings, heal when low.
fn bot(g: &mut Game, input: &mut Input, t: u32) {
    use crate::game::comp::ActKind;
    if g.screen == Screen::Dead {
        if g.dead_t > 2.6 && !g.bot_stop_on_death {
            input.inject(Action::UiAccept, true);
        }
        return;
    }
    if g.screen == Screen::LevelUp {
        input.inject(Action::UiAccept, true);
        return;
    }
    if g.screen != Screen::None {
        return;
    }
    let pe = g.player;
    let Some(pa) = g.w.actor.get(pe).cloned() else { return };
    if !pa.alive {
        return;
    }
    let pp = g.ppos();
    if std::env::var("STRAIN_BOTLOG").is_ok() && t % 30 == 0 {
        let lock = g.pl.lock.and_then(|l| g.w.body.get(l).map(|b| ((b.pos - pp).length(), g.w.foe.get(l).map(|f| f.name.clone()))));
        let boss = g.boss.and_then(|b| g.w.actor.get(b)).map(|a| (a.frac(), a.act.kind, g.w.body.get(g.boss.unwrap()).map(|b| (b.pos - pp).length())));
        eprintln!("bot t{t} hp {:.0}/{:.0} stam {:.0} act {:?} lock {:?} boss {:?} awake {} screen {:?} cut {}", pa.hp, pa.max_hp, g.pl.stamina, pa.act.kind, lock, boss, g.boss_awake, g.screen, g.cutscene.is_some());
    }
    if pa.frac() < 0.4 && pa.act.kind == ActKind::Idle && g.pl.heal_charges > 0 && t % 5 == 0 {
        input.inject(Action::Heal, true);
        return;
    }
    if g.pl.lock.is_none() {
        if t % 10 == 0 {
            input.inject(Action::LockOn, true);
        }
        // no target: walk toward the nearest infected
        let near = g.w.foe.iter().filter(|(id, _)| g.w.actor.get(g.w.ent(*id)).map(|a| a.alive && a.team == crate::game::comp::Team::Infected).unwrap_or(false)).map(|(id, _)| g.w.body.get(g.w.ent(id)).unwrap().pos).min_by(|a, b| (*a - pp).length().partial_cmp(&(*b - pp).length()).unwrap());
        if let Some(np) = near {
            if (np - pp).length() > 3.0 && (np - pp).length() < 40.0 {
                let d = np - pp;
                g.cam.yaw = d.x.atan2(d.z);
                input.inject(Action::MoveF, true);
            }
        }
        return;
    }
    let l = g.pl.lock.unwrap();
    let (Some(lb), Some(la)) = (g.w.body.get(l).copied(), g.w.actor.get(l).cloned()) else { return };
    let dist = crate::util::flat(lb.pos - pp).length() - lb.radius;
    let busy = pa.act.kind != ActKind::Idle;
    let hp = pa.frac();
    // the most threatening attacker in reach
    let mut threat = false;
    g.hash.query(pp, 7.0, |id, p, r| {
        let e = g.w.ent(id);
        if let Some(a) = g.w.actor.get(e) {
            if a.alive && a.team == crate::game::comp::Team::Infected && a.act.kind == ActKind::Attack {
                let mv = a.act.mv.unwrap();
                let ph = a.act.phase();
                let reach = mv.reach + mv.lunge * 0.6 + r + 0.6;
                if ph > 0.6 && ph < 1.15 && (p - pp).length() < reach {
                    threat = true;
                }
            }
        }
    });
    if threat && !busy && g.pl.stamina > 20.0 {
        input.inject(Action::MoveL, true);
        input.inject(Action::MoveB, true);
        input.inject(Action::Dodge, true);
        return;
    }
    if hp < 0.4 && !busy && g.pl.heal_charges > 0 && t % 5 == 0 {
        input.inject(Action::Heal, true);
        input.inject(Action::MoveB, true);
        return;
    }
    let reach = 1.6;
    if dist > reach {
        input.inject(Action::MoveF, true);
    } else if g.pl.stamina < 25.0 {
        input.inject(Action::MoveB, true);
        input.inject(Action::Block, la.act.kind == ActKind::Attack);
    } else if !busy || pa.act.phase() > 2.4 {
        if la.act.kind == ActKind::Opened || t % 9 == 0 {
            input.inject(Action::Light, true);
        }
    }
    let _ = t;
}
