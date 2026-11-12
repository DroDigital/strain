//! Data tables. All gameplay content (weapons, spells, affixes, mutations, items, enemy parts,
//! bosses) is described in RON files under `data/` that are embedded in the binary and parsed at
//! load time, so content can grow without engine changes.
use crate::render::textures::Tex;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
pub enum WeaponClass {
    Knife,
    Baton,
    Cleaver,
    Pipe,
    FireAxe,
    Sledge,
    Shield,
    BoneBlade,
    Whip,
    Catalyst,
    Fist,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Swing {
    SlashR,
    SlashL,
    Overhead,
    Thrust,
    Spin,
    Upper,
    Slam,
    Bash,
    Lash,
    Bite,
    Spit,
    Shoot,
    Claw,
    Leap,
    Charge,
}

type MoveTuple = (Swing, f32, f32, f32, f32, f32, f32, f32, f32, f32, f32);

/// One attack: wind-up, active and recovery phases plus its hit shape.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(from = "MoveTuple", into = "MoveTuple")]
pub struct MoveDef {
    pub swing: Swing,
    pub wind: f32,
    pub act: f32,
    pub rec: f32,
    /// damage multiplier on the wielder's base damage
    pub dmg: f32,
    pub poise: f32,
    pub stam: f32,
    pub reach: f32,
    /// half-angle of the hit arc in degrees (180 = all around)
    pub arc: f32,
    pub lunge: f32,
    pub knock: f32,
}
impl From<MoveTuple> for MoveDef {
    fn from(t: MoveTuple) -> Self {
        MoveDef { swing: t.0, wind: t.1, act: t.2, rec: t.3, dmg: t.4, poise: t.5, stam: t.6, reach: t.7, arc: t.8, lunge: t.9, knock: t.10 }
    }
}
impl From<MoveDef> for MoveTuple {
    fn from(m: MoveDef) -> Self {
        (m.swing, m.wind, m.act, m.rec, m.dmg, m.poise, m.stam, m.reach, m.arc, m.lunge, m.knock)
    }
}
impl MoveDef {
    pub fn total(&self) -> f32 {
        self.wind + self.act + self.rec
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct ClassDef {
    pub class: WeaponClass,
    pub two_handed: bool,
    /// fraction of damage removed when blocking with it
    pub block: f32,
    /// stamina cost multiplier when blocking
    pub stability: f32,
    pub light: Vec<MoveDef>,
    pub heavy: MoveDef,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
pub enum Source {
    Military,
    Civilian,
    Biological,
    Rift,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
pub enum Slot {
    Hand,
    Head,
    Body,
    Legs,
    Trinket,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
pub enum School {
    Flesh,
    Rift,
    Cinder,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ItemBase {
    pub id: String,
    pub name: String,
    pub slot: Slot,
    #[serde(default)]
    pub class: Option<WeaponClass>,
    pub source: Source,
    pub size: (u8, u8),
    #[serde(default)]
    pub dmg: f32,
    #[serde(default)]
    pub armor: f32,
    #[serde(default)]
    pub school: Option<School>,
    #[serde(default)]
    pub min_level: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
pub enum Stat {
    MaxHp,
    HpPct,
    MaxStam,
    StamRegen,
    Dmg,
    HeavyDmg,
    AtkSpeed,
    Armor,
    Poise,
    SpellPower,
    StrainCap,
    StrainCost,
    StrainDrain,
    MoveSpeed,
    Crit,
    Lifesteal,
    Biomass,
    HealCharges,
    HealPower,
    DodgeCost,
    BlockPower,
    ParryWindow,
    FireRes,
    Thorns,
    Luck,
}
impl Stat {
    pub fn label(self) -> &'static str {
        match self {
            Stat::MaxHp => "MAX HEALTH",
            Stat::HpPct => "% HEALTH",
            Stat::MaxStam => "MAX STAMINA",
            Stat::StamRegen => "% STAMINA REGEN",
            Stat::Dmg => "% DAMAGE",
            Stat::HeavyDmg => "% HEAVY DAMAGE",
            Stat::AtkSpeed => "% ATTACK SPEED",
            Stat::Armor => "ARMOUR",
            Stat::Poise => "POISE",
            Stat::SpellPower => "% SPELL POWER",
            Stat::StrainCap => "STRAIN CAPACITY",
            Stat::StrainCost => "% STRAIN COST",
            Stat::StrainDrain => "% STRAIN VENTED PER HIT",
            Stat::MoveSpeed => "% MOVE SPEED",
            Stat::Crit => "% CRITICAL CHANCE",
            Stat::Lifesteal => "% LIFE ON HIT",
            Stat::Biomass => "% BIOMASS",
            Stat::HealCharges => "HEAL CHARGES",
            Stat::HealPower => "% HEALING",
            Stat::DodgeCost => "% DODGE COST",
            Stat::BlockPower => "% BLOCK",
            Stat::ParryWindow => "% PARRY WINDOW",
            Stat::FireRes => "% FIRE RESISTANCE",
            Stat::Thorns => "THORNS",
            Stat::Luck => "% LOOT FIND",
        }
    }
}

/// Rule-changing behaviours shared by mutations, unique items and affixes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Effect {
    Stat(Stat, f32),
    DodgeSpores(f32),
    DodgeBlink,
    DodgeFire(f32),
    DodgeShock(f32),
    ParryTendril(f32),
    ParryNova(f32),
    ParryStrain(f32),
    HeavyPull(f32),
    HeavyShock(f32),
    HeavySpikes(f32),
    ComboFinisher(f32),
    KillExplode(f32),
    KillHeal(f32),
    KillStamina,
    KillFrenzy(f32),
    LowHpPower(f32),
    StrainPower(f32),
    HitBurn(f32),
    HitRot(f32),
    HitSlow(f32),
    HitChain(f32),
    HitShard(f32),
    BackstabCrit(f32),
    Execute(f32),
    BlockThorns(f32),
    BlockReflect,
    HurtLarvae(f32),
    HurtBurst(f32),
    SecondWind,
    HealBurst(f32),
    HealStrain(f32),
    HealRegen,
    SprintFire(f32),
    SprintCharge(f32),
    SporeAura(f32),
    Regen(f32),
    RuptureNova(f32),
    RuptureHeal(f32),
    RuptureStrain(f32),
    SpellEcho(f32),
    SpellBlood,
    SpellFree(f32),
    StrainCalm,
    StrainLeak(f32),
    StrainArmor(f32),
    MeleeStrain(f32),
    NoHeal,
    NoBlock,
    NoSprint,
    DoubleDodge,
    ExtraArm(f32),
    Eyes,
    Magnet,
    Phase,
    HandWeapon(u8, String),
    HandOrgan(u8, School),
    FireHeart,
    TimeReflex(f32),
    Glutton(f32),
    Unstable(f32),
    DodgeCostsHp,
    HeavyFreeAtStrain,
    AlwaysFullPower,
    PitHunger,
    TwoHandOne,
    Spellblade,
    Dread,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum GrowthKind {
    Spikes,
    Sacs,
    Fronds,
    Plates,
    Eyes,
    Antlers,
    Fins,
    Tumor,
    Mandibles,
    Tail,
    Arm,
    Glow,
    Halo,
    Cracks,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Attach {
    Head,
    Back,
    Chest,
    ShoulderL,
    ShoulderR,
    ArmL,
    ArmR,
    Hip,
    Legs,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Growth {
    pub kind: GrowthKind,
    pub at: Attach,
    pub tex: Tex,
    #[serde(default = "one")]
    pub size: f32,
}
fn one() -> f32 {
    1.0
}

#[derive(Clone, Debug, Deserialize)]
pub struct MutationDef {
    pub id: String,
    pub name: String,
    pub tier: u8,
    pub desc: String,
    pub fx: Vec<Effect>,
    pub vis: (GrowthKind, Attach, Tex),
    /// mutually exclusive branch group
    #[serde(default)]
    pub excl: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    /// (tag of another owned mutation, bonus effect granted while both are held)
    #[serde(default)]
    pub syn: Option<(String, Effect)>,
}

#[derive(Clone, Debug, Deserialize)]
pub enum SpellKind {
    /// speed, damage, blast radius, count, spread degrees, homing
    Bolt(f32, f32, f32, u32, f32, bool),
    /// damage per spike, length
    Spikes(f32, f32),
    /// range, damage
    Grab(f32, f32),
    /// radius, dps, duration
    Cloud(f32, f32, f32),
    /// range, damage, heal fraction
    Drain(f32, f32, f32),
    /// count, duration
    Brood(u32, f32),
    /// armour, duration
    Carapace(f32, f32),
    /// distance
    Blink(f32),
    /// radius, pull force, damage, duration
    Well(f32, f32, f32, f32),
    /// damage per second, length, duration
    Tear(f32, f32, f32),
    /// radius, duration
    Stasis(f32, f32),
    /// range, damage
    Transpose(f32, f32),
    /// radius, damage
    Nova(f32, f32),
    /// length, dps, duration
    Wall(f32, f32, f32),
    /// burn dps, duration
    Kindle(f32, f32),
    /// radius, damage, delay
    Meteor(f32, f32, f32),
    /// range, dps, duration (cone of fire)
    Breath(f32, f32, f32),
}

#[derive(Clone, Debug, Deserialize)]
pub struct SpellDef {
    pub id: String,
    pub name: String,
    pub school: School,
    pub strain: f32,
    pub cast: f32,
    pub kind: SpellKind,
    pub desc: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
pub enum AffixKind {
    Calcified,
    Splitting,
    Tethered,
    Mimic,
    Broadcasting,
    Gravid,
    Volatile,
    Phasing,
    Regenerating,
    Magnetised,
    Frenzied,
    Thorned,
    Leeching,
    Cindered,
    Sporing,
    Shrieking,
    Hasted,
    Colossal,
    Grasping,
    Warding,
    Mirrored,
    Undying,
    Blinking,
    Withering,
}
#[derive(Clone, Debug, Deserialize)]
pub struct AffixDef {
    pub kind: AffixKind,
    pub name: String,
    pub desc: String,
    pub vis: (GrowthKind, Attach, Tex),
    #[serde(default = "one")]
    pub hp: f32,
    #[serde(default = "one")]
    pub dmg: f32,
    #[serde(default = "one")]
    pub speed: f32,
    #[serde(default = "one")]
    pub scale: f32,
    #[serde(default)]
    pub min_level: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
pub enum AiKind {
    Rusher,
    Brute,
    Leaper,
    Spitter,
    Crawler,
    Shielded,
    Ambusher,
    Summoner,
    Soldier,
    Civilian,
    Boss,
    Husk,
    Ally,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum ArmKind {
    Human,
    Claw,
    Blade,
    Club,
    Tendril,
    Cannon,
    Shield,
    Stub,
    Rifle,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum HeadKind {
    Human,
    Hood,
    Helmet,
    Split,
    Eyeless,
    Bloom,
    Maw,
    Cluster,
    Stalk,
    Crown,
    None,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum LegKind {
    Human,
    Digi,
    Spider,
    Stump,
}

#[derive(Clone, Debug, Deserialize)]
pub struct TorsoPart {
    pub id: String,
    /// width, height, depth
    pub size: (f32, f32, f32),
    pub tex: Tex,
    pub hunch: f32,
    /// (min, max) infection stage this part appears at
    pub stage: (u8, u8),
}
#[derive(Clone, Debug, Deserialize)]
pub struct HeadPart {
    pub kind: HeadKind,
    pub tex: Tex,
    pub size: f32,
    pub stage: (u8, u8),
}
#[derive(Clone, Debug, Deserialize)]
pub struct ArmPart {
    pub kind: ArmKind,
    pub tex: Tex,
    pub len: f32,
    pub thick: f32,
    pub stage: (u8, u8),
}
#[derive(Clone, Debug, Deserialize)]
pub struct LegPart {
    pub kind: LegKind,
    pub tex: Tex,
    pub len: f32,
    pub thick: f32,
    pub count: u8,
    pub stage: (u8, u8),
}
#[derive(Clone, Debug, Deserialize)]
pub struct PartTables {
    pub torsos: Vec<TorsoPart>,
    pub heads: Vec<HeadPart>,
    pub arms: Vec<ArmPart>,
    pub legs: Vec<LegPart>,
    pub growths: Vec<(GrowthKind, Attach, Tex, u8)>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Archetype {
    pub id: String,
    pub name: String,
    pub ai: AiKind,
    pub hp: f32,
    pub dmg: f32,
    pub speed: f32,
    pub poise: f32,
    pub scale: (f32, f32),
    pub biomass: f32,
    /// group size range when spawned as a cluster
    pub group: (u32, u32),
    pub min_level: u32,
    pub torsos: Vec<String>,
    pub arms: Vec<ArmKind>,
    pub legs: Vec<LegKind>,
    pub attacks: Vec<MoveDef>,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct ApexDef {
    pub id: String,
    pub name: String,
    pub archetype: String,
    pub district: u8,
    pub affixes: Vec<AffixKind>,
    pub scale: f32,
    pub hp: f32,
    pub tint: (f32, f32, f32),
    pub head: HeadKind,
    pub growths: Vec<(GrowthKind, Attach, Tex)>,
}

#[derive(Clone, Copy, PartialEq, Debug, Deserialize)]
pub enum BossMove {
    /// combo of melee swings from the boss attack list (index range)
    Melee(usize),
    /// radius, damage: telegraphed ground slam with expanding ring
    Slam(f32, f32),
    /// count, speed, damage
    Volley(u32, f32, f32),
    /// damage: long charge across the arena
    Rush(f32),
    /// archetype index into summon list, count
    Summon(u32),
    /// damage per second, duration: rotating beam
    Beam(f32, f32),
    /// teleports next to / away from the target
    Warp,
    /// radius, dps, duration: lingering hazard pools under the player
    Pools(f32, f32, f32),
    /// damage: rain of delayed blasts
    Rain(u32, f32),
    /// pull strength, duration
    Vortex(f32, f32),
}
#[derive(Clone, Debug, Deserialize)]
pub struct BossPhase {
    /// phase starts when hp fraction falls to this
    pub at: f32,
    pub moves: Vec<BossMove>,
    pub speed: f32,
    pub line: String,
}
#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct BossDef {
    pub id: String,
    pub name: String,
    pub title: String,
    pub hp: f32,
    pub dmg: f32,
    pub scale: f32,
    pub tint: (f32, f32, f32),
    pub torso: (f32, f32, f32),
    pub tex: Tex,
    pub head: HeadKind,
    pub arms: Vec<ArmKind>,
    pub legs: (LegKind, u8),
    pub growths: Vec<(GrowthKind, Attach, Tex)>,
    pub attacks: Vec<MoveDef>,
    pub summons: String,
    pub phases: Vec<BossPhase>,
    pub drops: Vec<String>,
    pub intro: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ItemAffix {
    pub name: String,
    pub prefix: bool,
    pub stat: Stat,
    pub range: (f32, f32),
    pub slots: Vec<Slot>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct UniqueDef {
    pub id: String,
    pub name: String,
    pub base: String,
    pub mythic: bool,
    pub fx: Vec<Effect>,
    pub desc: String,
    pub flavor: String,
}

#[derive(Clone, Debug, Deserialize)]
struct WeaponFile {
    classes: Vec<ClassDef>,
}
#[derive(Clone, Debug, Deserialize)]
struct ItemFile {
    bases: Vec<ItemBase>,
    affixes: Vec<ItemAffix>,
    uniques: Vec<UniqueDef>,
}
#[derive(Clone, Debug, Deserialize)]
struct EnemyFile {
    parts: PartTables,
    archetypes: Vec<Archetype>,
    affixes: Vec<AffixDef>,
    apex: Vec<ApexDef>,
    bosses: Vec<BossDef>,
}
#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct TextFile {
    pub notes: Vec<String>,
    pub graffiti: Vec<String>,
    pub soldier_barks: Vec<String>,
    pub soldier_mutated: Vec<String>,
    pub radio: Vec<String>,
    pub civilian: Vec<String>,
    pub opening: Vec<String>,
    pub ending: Vec<String>,
    pub tips: Vec<String>,
}

pub struct Tables {
    pub classes: Vec<ClassDef>,
    pub bases: Vec<ItemBase>,
    pub item_affixes: Vec<ItemAffix>,
    pub uniques: Vec<UniqueDef>,
    pub spells: Vec<SpellDef>,
    pub mutations: Vec<MutationDef>,
    pub parts: PartTables,
    pub archetypes: Vec<Archetype>,
    pub affixes: Vec<AffixDef>,
    pub apex: Vec<ApexDef>,
    pub bosses: Vec<BossDef>,
    pub text: TextFile,
}

fn parse<T: for<'a> Deserialize<'a>>(name: &str, src: &str) -> T {
    match ron::from_str(src) {
        Ok(v) => v,
        Err(e) => panic!("data table {name}: {e}"),
    }
}

impl Tables {
    pub fn load() -> Tables {
        let w: WeaponFile = parse("weapons", include_str!("../data/weapons.ron"));
        let i: ItemFile = parse("items", include_str!("../data/items.ron"));
        let e: EnemyFile = parse("enemies", include_str!("../data/enemies.ron"));
        Tables {
            classes: w.classes,
            bases: i.bases,
            item_affixes: i.affixes,
            uniques: i.uniques,
            spells: parse("spells", include_str!("../data/spells.ron")),
            mutations: parse("mutations", include_str!("../data/mutations.ron")),
            parts: e.parts,
            archetypes: e.archetypes,
            affixes: e.affixes,
            apex: e.apex,
            bosses: e.bosses,
            text: parse("text", include_str!("../data/text.ron")),
        }
    }
    pub fn class(&self, c: WeaponClass) -> &ClassDef {
        self.classes.iter().find(|d| d.class == c).unwrap_or(&self.classes[0])
    }
    pub fn base(&self, id: &str) -> usize {
        self.bases.iter().position(|b| b.id == id).unwrap_or(0)
    }
    pub fn archetype(&self, id: &str) -> usize {
        self.archetypes.iter().position(|b| b.id == id).unwrap_or(0)
    }
    pub fn affix(&self, k: AffixKind) -> &AffixDef {
        self.affixes.iter().find(|a| a.kind == k).unwrap_or(&self.affixes[0])
    }
    pub fn spells_of(&self, s: School) -> Vec<usize> {
        (0..self.spells.len()).filter(|i| self.spells[*i].school == s).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tables_load_and_meet_content_minimums() {
        let t = Tables::load();
        assert!(t.classes.len() >= 11);
        assert!(t.mutations.len() >= 80, "mutations: {}", t.mutations.len());
        assert!(t.uniques.len() >= 30, "uniques: {}", t.uniques.len());
        assert!(t.affixes.len() >= 20);
        assert!(t.archetypes.iter().filter(|a| !matches!(a.ai, AiKind::Soldier | AiKind::Civilian | AiKind::Ally)).count() >= 8);
        for s in [School::Flesh, School::Rift, School::Cinder] {
            assert!(t.spells_of(s).len() >= 6);
        }
        assert_eq!(t.bosses.len(), 5);
        for u in &t.uniques {
            assert!(t.bases.iter().any(|b| b.id == u.base), "unique {} has unknown base {}", u.id, u.base);
        }
        for a in &t.archetypes {
            for to in &a.torsos {
                assert!(t.parts.torsos.iter().any(|p| &p.id == to), "unknown torso {to}");
            }
        }
        for b in &t.bosses {
            for d in &b.drops {
                assert!(t.uniques.iter().any(|u| &u.id == d), "boss drop {d}");
            }
            assert!(t.archetypes.iter().any(|a| a.id == b.summons));
        }
        for a in &t.apex {
            assert!(t.archetypes.iter().any(|x| x.id == a.archetype));
        }
        let mut ids: Vec<&str> = t.mutations.iter().map(|m| m.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), t.mutations.len(), "duplicate mutation ids");
    }
}
