//! Components stored in the ECS world.
use crate::anim::Rig;
use crate::data::{AffixKind, AiKind, MoveDef, School, Source};
use crate::ecs::{define_world, Entity};
use crate::game::loot::Item;
use glam::Vec3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Team {
    Player,
    Infected,
    Military,
    Civilian,
    Ally,
}

#[derive(Clone, Copy, Debug)]
pub struct Body {
    pub pos: Vec3,
    pub prev: Vec3,
    /// intended planar velocity (from locomotion)
    pub vel: Vec3,
    /// external impulse velocity (knockback, pulls), decays
    pub push: Vec3,
    pub yaw: f32,
    pub prev_yaw: f32,
    pub radius: f32,
    pub height: f32,
    pub mass: f32,
    /// collides with other bodies
    pub solid: bool,
    /// passes through bodies (phase dodge)
    pub ghost: bool,
}
impl Body {
    pub fn new(pos: Vec3, radius: f32, height: f32) -> Body {
        Body { pos, prev: pos, vel: Vec3::ZERO, push: Vec3::ZERO, yaw: 0.0, prev_yaw: 0.0, radius, height, mass: 1.0, solid: true, ghost: false }
    }
    pub fn center(&self) -> Vec3 {
        self.pos + Vec3::Y * self.height * 0.55
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ActKind {
    Idle,
    Attack,
    Dodge,
    Stagger,
    Cast,
    Heal,
    Rupture,
    /// loss of control at full Strain, or other forced paralysis
    Seized,
    /// boss / ai special move; `spell` holds the move index
    Special,
    /// knocked down by a parry, open to a Rupture
    Opened,
}

#[derive(Clone, Debug)]
pub struct Act {
    pub kind: ActKind,
    pub t: f32,
    pub dur: f32,
    pub mv: Option<MoveDef>,
    pub left: bool,
    pub heavy: bool,
    pub charge: f32,
    pub charging: bool,
    pub combo: u8,
    pub dir: Vec3,
    pub spell: usize,
    pub target: Option<Entity>,
    pub hits: Vec<u32>,
    /// attack speed multiplier applied to this action
    pub rate: f32,
    pub fired: bool,
}
impl Default for Act {
    fn default() -> Self {
        Act { kind: ActKind::Idle, t: 0.0, dur: 0.0, mv: None, left: false, heavy: false, charge: 0.0, charging: false, combo: 0, dir: Vec3::Z, spell: 0, target: None, hits: Vec::new(), rate: 1.0, fired: false }
    }
}
impl Act {
    pub fn new(kind: ActKind, dur: f32) -> Act {
        Act { kind, dur, ..Default::default() }
    }
    pub fn attack(mv: MoveDef, left: bool, heavy: bool, combo: u8, rate: f32) -> Act {
        Act { kind: ActKind::Attack, dur: mv.total(), mv: Some(mv), left, heavy, combo, rate, ..Default::default() }
    }
    /// 0..1 windup, 1..2 active, 2..3 recovery (for attacks)
    pub fn phase(&self) -> f32 {
        match self.mv {
            Some(m) => {
                let t = self.t;
                if t < m.wind {
                    t / m.wind.max(1e-3)
                } else if t < m.wind + m.act {
                    1.0 + (t - m.wind) / m.act.max(1e-3)
                } else {
                    (2.0 + (t - m.wind - m.act) / m.rec.max(1e-3)).min(3.0)
                }
            }
            None => (self.t / self.dur.max(1e-3)).min(1.0),
        }
    }
    pub fn active(&self) -> bool {
        let p = self.phase();
        self.kind == ActKind::Attack && (1.0..2.0).contains(&p)
    }
    pub fn busy(&self) -> bool {
        self.kind != ActKind::Idle
    }
}

#[derive(Clone, Debug)]
pub struct Actor {
    pub team: Team,
    pub hp: f32,
    pub max_hp: f32,
    pub poise: f32,
    pub max_poise: f32,
    pub level: u32,
    pub dmg: f32,
    pub speed: f32,
    pub armor: f32,
    pub act: Act,
    pub alive: bool,
    pub dead_t: f32,
    pub flash: f32,
    pub burn: f32,
    pub burn_dps: f32,
    pub rot: f32,
    pub rot_dps: f32,
    pub slow: f32,
    /// time-scale from stasis fields (1 = normal), reset every tick
    pub tscale: f32,
    pub since_hit: f32,
    pub hit_dir: Vec3,
    pub fire_immune: bool,
    pub dissolve: f32,
    pub glow: f32,
    pub armor_buff: f32,
    pub armor_buff_t: f32,
    /// damage reduction from Warding allies (reset every tick)
    pub ward: f32,
    /// damage multiplier from Broadcasting allies (reset every tick)
    pub rally: f32,
    pub kindle: f32,
}
impl Actor {
    pub fn new(team: Team, hp: f32, poise: f32, dmg: f32, speed: f32, level: u32) -> Actor {
        Actor {
            team,
            hp,
            max_hp: hp,
            poise,
            max_poise: poise,
            level,
            dmg,
            speed,
            armor: 0.0,
            act: Act::default(),
            alive: true,
            dead_t: 0.0,
            flash: 0.0,
            burn: 0.0,
            burn_dps: 0.0,
            rot: 0.0,
            rot_dps: 0.0,
            slow: 0.0,
            tscale: 1.0,
            since_hit: 99.0,
            hit_dir: Vec3::Z,
            fire_immune: false,
            dissolve: 0.0,
            glow: 0.0,
            armor_buff: 0.0,
            armor_buff_t: 0.0,
            ward: 0.0,
            rally: 1.0,
            kindle: 0.0,
        }
    }
    pub fn frac(&self) -> f32 {
        (self.hp / self.max_hp).clamp(0.0, 1.0)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AiState {
    Idle,
    Wander,
    Alert,
    Chase,
    Strafe,
    Retreat,
    Hide,
    Flee,
    Cower,
    Follow,
    Turning,
}

#[derive(Clone, Debug)]
pub struct Ai {
    pub kind: AiKind,
    pub state: AiState,
    pub t: f32,
    pub target: Option<Entity>,
    pub home: Vec3,
    pub goal: Vec3,
    pub cd: f32,
    pub special_cd: f32,
    pub strafe_dir: f32,
    pub alert: f32,
    /// soldiers: how angry they are at the player
    pub grudge: f32,
    pub bark_cd: f32,
    pub think: f32,
    pub stuck: f32,
    pub phase: usize,
    pub move_i: usize,
    pub summons: u32,
}
impl Ai {
    pub fn new(kind: AiKind, home: Vec3, seed: u32) -> Ai {
        Ai {
            kind,
            state: AiState::Idle,
            t: 0.0,
            target: None,
            home,
            goal: home,
            cd: 1.0 + (seed % 100) as f32 * 0.01,
            special_cd: 3.0 + (seed % 37) as f32 * 0.1,
            strafe_dir: if seed % 2 == 0 { 1.0 } else { -1.0 },
            alert: 0.0,
            grudge: 0.0,
            bark_cd: 5.0 + (seed % 50) as f32 * 0.2,
            think: (seed % 30) as f32 * 0.01,
            stuck: 0.0,
            phase: 0,
            move_i: 0,
            summons: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Foe {
    pub arch: usize,
    pub name: String,
    /// 1..=4 infection stage (rarity); 0 for humans
    pub stage: u8,
    pub affixes: Vec<AffixKind>,
    pub attacks: Vec<MoveDef>,
    pub biomass: f32,
    pub apex: Option<usize>,
    pub boss: Option<usize>,
    /// risen husk carrying a lost build
    pub husk: bool,
    pub source: Source,
    /// Calcified plates remaining (absorbs damage)
    pub plates: f32,
    pub undying_used: bool,
    pub mimic: bool,
    pub phase_t: f32,
    pub aff_t: f32,
    pub shrieked: bool,
    /// spawn bookkeeping: (chunk key, spawn index)
    pub origin: Option<((i32, i32), usize)>,
    pub ttl: f32,
    pub can_split: bool,
    pub transform_t: f32,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ProjKind {
    Acid,
    Bullet,
    Ember,
    Shard,
    BoneShard,
    BossOrb,
    Fireball,
}
#[derive(Clone, Debug)]
pub struct Proj {
    pub kind: ProjKind,
    pub vel: Vec3,
    pub dmg: f32,
    pub poise: f32,
    pub team: Team,
    pub owner: Option<Entity>,
    pub ttl: f32,
    pub radius: f32,
    pub blast: f32,
    pub homing: Option<Entity>,
    pub reflected: bool,
    pub spell: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ZoneKind {
    /// rot damage, spores
    Spores,
    /// fire damage
    Fire,
    Acid,
    /// gravity well: pulls and damages
    Well,
    /// spatial tear segment
    Tear,
    /// slows everything but the caster's team
    Stasis,
    /// telegraphed delayed blast (meteor, boss rain)
    Blast,
    /// expanding shock ring (slams)
    Ring,
    /// rotating boss beam
    Beam,
    /// boss vortex
    Vortex,
    /// strain-singing hazard in rifts
    Singing,
    /// a line of erupting bone spikes
    Spikes,
}
#[derive(Clone, Debug)]
pub struct Zone {
    pub kind: ZoneKind,
    pub radius: f32,
    pub dps: f32,
    pub t: f32,
    pub dur: f32,
    pub team: Team,
    pub owner: Option<Entity>,
    pub dir: Vec3,
    pub len: f32,
    pub delay: f32,
    pub done: bool,
    /// hazards baked into the area persist forever
    pub permanent: bool,
}

#[derive(Clone, Debug)]
pub enum PickKind {
    Item(Item),
    Biomass(f32),
    Scrip(u32),
}
#[derive(Clone, Debug)]
pub struct Pickup {
    pub kind: PickKind,
    pub t: f32,
}

/// Static interactable placed by world gen (pits, vendor, notes, portals, husk).
#[derive(Clone, Debug)]
pub struct Interact {
    pub kind: crate::world::InteractKind,
}

/// Cast spell data carried by the player's catalysts.
#[derive(Clone, Debug)]
pub struct SpellSlot {
    pub spell: usize,
    pub school: School,
}

define_world! {
    body: Body,
    actor: Actor,
    rig: Rig,
    ai: Ai,
    foe: Foe,
    proj: Proj,
    zone: Zone,
    pickup: Pickup,
    interact: Interact,
}
