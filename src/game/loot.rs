//! ARPG loot: item generation with item level, semi-random rolls, affixes and rule-changing
//! uniques; grid inventory; equipment slots and the valid hand setups.
use crate::data::{Effect, School, Slot, Source, Stat, Tables, WeaponClass};
use crate::util::{chance, rf, Rng};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub enum Rarity {
    Common,
    Magic,
    Rare,
    Epic,
    Legendary,
    Mythic,
}
impl Rarity {
    pub fn name(self) -> &'static str {
        match self {
            Rarity::Common => "COMMON",
            Rarity::Magic => "MAGIC",
            Rarity::Rare => "RARE",
            Rarity::Epic => "EPIC",
            Rarity::Legendary => "LEGENDARY",
            Rarity::Mythic => "MYTHIC",
        }
    }
    pub fn color(self) -> [f32; 3] {
        match self {
            Rarity::Common => [0.8, 0.8, 0.78],
            Rarity::Magic => [0.35, 0.55, 1.0],
            Rarity::Rare => [1.0, 0.85, 0.25],
            Rarity::Epic => [0.75, 0.35, 1.0],
            Rarity::Legendary => [1.0, 0.5, 0.1],
            Rarity::Mythic => [0.2, 1.0, 0.7],
        }
    }
    pub fn affixes(self) -> (usize, usize) {
        match self {
            Rarity::Common => (0, 0),
            Rarity::Magic => (1, 2),
            Rarity::Rare => (3, 4),
            Rarity::Epic => (4, 5),
            Rarity::Legendary => (2, 3),
            Rarity::Mythic => (3, 4),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Item {
    pub base: String,
    pub rarity: Rarity,
    pub ilvl: u32,
    /// rolled base damage (weapons) or armour
    pub power: f32,
    pub affixes: Vec<(Stat, f32)>,
    pub unique: Option<String>,
    /// spells granted when used as a catalyst
    pub spells: Vec<String>,
    /// natural weapon grown by a mutation (cannot be removed or dropped)
    pub grown: bool,
    pub seed: u32,
}

impl Item {
    pub fn base<'a>(&self, t: &'a Tables) -> &'a crate::data::ItemBase {
        &t.bases[t.base(&self.base)]
    }
    pub fn class(&self, t: &Tables) -> Option<WeaponClass> {
        self.base(t).class
    }
    pub fn two_handed(&self, t: &Tables) -> bool {
        self.class(t).map(|c| t.class(c).two_handed).unwrap_or(false)
    }
    pub fn size(&self, t: &Tables) -> (u8, u8) {
        self.base(t).size
    }
    pub fn name(&self, t: &Tables) -> String {
        if let Some(u) = &self.unique {
            if let Some(d) = t.uniques.iter().find(|x| &x.id == u) {
                return d.name.clone();
            }
        }
        let b = self.base(t);
        if self.grown {
            return b.name.clone();
        }
        let mut n = b.name.clone();
        let pre = self.affixes.iter().find_map(|(s, _)| t.item_affixes.iter().find(|a| a.stat == *s && a.prefix));
        let suf = self.affixes.iter().find_map(|(s, _)| t.item_affixes.iter().find(|a| a.stat == *s && !a.prefix));
        if let Some(p) = pre {
            n = format!("{} {}", p.name, n);
        }
        if let Some(s) = suf {
            if self.rarity <= Rarity::Magic || pre.is_none() {
                n = format!("{} {}", n, s.name);
            }
        }
        n
    }
    pub fn effects<'a>(&self, t: &'a Tables) -> &'a [Effect] {
        if let Some(u) = &self.unique {
            if let Some(d) = t.uniques.iter().find(|x| &x.id == u) {
                return &d.fx;
            }
        }
        &[]
    }
    pub fn school(&self, t: &Tables) -> Option<School> {
        self.base(t).school
    }
    pub fn value(&self) -> u32 {
        let r = match self.rarity {
            Rarity::Common => 6,
            Rarity::Magic => 15,
            Rarity::Rare => 40,
            Rarity::Epic => 90,
            Rarity::Legendary => 200,
            Rarity::Mythic => 400,
        };
        r + self.ilvl * 3
    }
}

/// Grid inventory with multi-cell items.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Grid {
    pub w: u8,
    pub h: u8,
    pub items: Vec<(u8, u8, Item)>,
}
impl Grid {
    pub fn new(w: u8, h: u8) -> Grid {
        Grid { w, h, items: Vec::new() }
    }
    pub fn at(&self, t: &Tables, x: u8, y: u8) -> Option<usize> {
        self.items.iter().position(|(ix, iy, it)| {
            let (w, h) = it.size(t);
            x >= *ix && x < ix + w && y >= *iy && y < iy + h
        })
    }
    pub fn fits(&self, t: &Tables, x: u8, y: u8, w: u8, h: u8, ignore: Option<usize>) -> bool {
        if x + w > self.w || y + h > self.h {
            return false;
        }
        for (i, (ix, iy, it)) in self.items.iter().enumerate() {
            if Some(i) == ignore {
                continue;
            }
            let (iw, ih) = it.size(t);
            if x < ix + iw && x + w > *ix && y < iy + ih && y + h > *iy {
                return false;
            }
        }
        true
    }
    pub fn add(&mut self, t: &Tables, it: Item) -> Result<(), Item> {
        let (w, h) = it.size(t);
        for y in 0..self.h {
            for x in 0..self.w {
                if self.fits(t, x, y, w, h, None) {
                    self.items.push((x, y, it));
                    return Ok(());
                }
            }
        }
        Err(it)
    }
    pub fn place(&mut self, t: &Tables, x: u8, y: u8, it: Item) -> Result<(), Item> {
        let (w, h) = it.size(t);
        if self.fits(t, x, y, w, h, None) {
            self.items.push((x, y, it));
            Ok(())
        } else {
            Err(it)
        }
    }
}

pub const EQ_SLOTS: [(&str, Slot); 7] = [("RIGHT HAND", Slot::Hand), ("LEFT HAND", Slot::Hand), ("HEAD", Slot::Head), ("BODY", Slot::Body), ("LEGS", Slot::Legs), ("TRINKET", Slot::Trinket), ("TRINKET", Slot::Trinket)];

/// Whether `it` may go into equipment slot `slot` given the other hand. Returns why not.
pub fn can_equip(t: &Tables, eq: &[Option<Item>; 7], slot: usize, it: &Item, two_hand_one: bool) -> Result<(), &'static str> {
    let b = it.base(t);
    if b.slot != EQ_SLOTS[slot].1 {
        return Err("WRONG SLOT");
    }
    if slot <= 1 {
        let other = &eq[1 - slot];
        if other.as_ref().map(|o| o.grown).unwrap_or(false) && it.two_handed(t) && !two_hand_one {
            return Err("THE OTHER HAND IS NOT YOURS ANY MORE");
        }
        let class = b.class.unwrap_or(WeaponClass::Fist);
        if slot == 0 && class == WeaponClass::Shield {
            return Err("SHIELDS GO IN THE LEFT HAND");
        }
        if slot == 1 && it.two_handed(t) && !two_hand_one {
            return Err("TWO-HANDED: EQUIP IN THE RIGHT HAND");
        }
        if eq[slot].as_ref().map(|o| o.grown).unwrap_or(false) {
            return Err("THAT HAND HAS BECOME SOMETHING ELSE");
        }
    }
    Ok(())
}

/// Roll a rarity. `bonus` shifts the curve upward (stage, luck, source).
pub fn roll_rarity(r: &mut Rng, bonus: f32) -> Rarity {
    let x = r.f32() / (1.0 + bonus);
    if x < 0.003 {
        Rarity::Mythic
    } else if x < 0.018 {
        Rarity::Legendary
    } else if x < 0.06 {
        Rarity::Epic
    } else if x < 0.18 {
        Rarity::Rare
    } else if x < 0.45 {
        Rarity::Magic
    } else {
        Rarity::Common
    }
}

/// Generate an item from a source pool.
pub fn gen_item(t: &Tables, r: &mut Rng, ilvl: u32, sources: &[Source], rarity: Rarity, forced_base: Option<&str>) -> Item {
    let ilvl = ilvl.max(1);
    // uniques for legendary/mythic
    if rarity >= Rarity::Legendary && forced_base.is_none() {
        let want_mythic = rarity == Rarity::Mythic;
        let pool: Vec<usize> = (0..t.uniques.len())
            .filter(|&i| {
                let u = &t.uniques[i];
                u.mythic == want_mythic && sources.contains(&t.bases[t.base(&u.base)].source)
            })
            .collect();
        let pool = if pool.is_empty() { (0..t.uniques.len()).filter(|&i| t.uniques[i].mythic == want_mythic).collect() } else { pool };
        let u = &t.uniques[pool[r.usize(0..pool.len())]];
        return make_unique(t, r, ilvl, &u.id);
    }
    let base_i = match forced_base {
        Some(b) => t.base(b),
        None => {
            let pool: Vec<usize> = (0..t.bases.len()).filter(|&i| sources.contains(&t.bases[i].source) && t.bases[i].min_level <= ilvl).collect();
            if pool.is_empty() {
                0
            } else {
                pool[r.usize(0..pool.len())]
            }
        }
    };
    let b = &t.bases[base_i];
    let scale = 1.0 + (ilvl as f32 - 1.0) * 0.07;
    let power = if b.dmg > 0.0 { b.dmg * scale * rf(r, 0.9, 1.12) } else { b.armor * scale * rf(r, 0.85, 1.15) };
    let mut it = Item { base: b.id.clone(), rarity, ilvl, power, affixes: Vec::new(), unique: None, spells: Vec::new(), grown: false, seed: r.u32(..) };
    roll_affixes(t, r, &mut it);
    roll_spells(t, r, &mut it);
    it
}

pub fn make_unique(t: &Tables, r: &mut Rng, ilvl: u32, id: &str) -> Item {
    let u = t.uniques.iter().find(|u| u.id == id).expect("unique");
    let mut it = gen_item(t, r, ilvl, &[], Rarity::Rare, Some(&u.base));
    it.rarity = if u.mythic { Rarity::Mythic } else { Rarity::Legendary };
    it.unique = Some(u.id.clone());
    it.power *= 1.15;
    it.affixes.clear();
    roll_affixes(t, r, &mut it);
    it
}

fn roll_affixes(t: &Tables, r: &mut Rng, it: &mut Item) {
    let slot = it.base(t).slot;
    let (lo, hi) = it.rarity.affixes();
    let n = if hi == 0 { 0 } else { r.usize(lo..=hi) };
    let pool: Vec<usize> = (0..t.item_affixes.len()).filter(|&i| t.item_affixes[i].slots.contains(&slot)).collect();
    let lvl = 1.0 + it.ilvl as f32 * 0.03;
    for _ in 0..n {
        if pool.is_empty() {
            break;
        }
        let a = &t.item_affixes[pool[r.usize(0..pool.len())]];
        if it.affixes.iter().any(|(s, _)| *s == a.stat) {
            continue;
        }
        let v = rf(r, a.range.0, a.range.1) * lvl;
        let v = if v.abs() >= 3.0 { v.round() } else { (v * 10.0).round() / 10.0 };
        it.affixes.push((a.stat, v));
    }
}

fn roll_spells(t: &Tables, r: &mut Rng, it: &mut Item) {
    if let Some(s) = it.school(t) {
        let pool = t.spells_of(s);
        let n = if it.rarity >= Rarity::Rare { 3 } else { 2 };
        let mut picks: Vec<usize> = Vec::new();
        while picks.len() < n.min(pool.len()) {
            let c = pool[r.usize(0..pool.len())];
            if !picks.contains(&c) {
                picks.push(c);
            }
        }
        it.spells = picks.into_iter().map(|i| t.spells[i].id.clone()).collect();
    }
}

/// Natural weapon or casting organ grown by a mutation.
pub fn grown_item(t: &Tables, base: &str, level: u32, school: Option<School>) -> Item {
    let mut r = Rng::with_seed(level as u64 * 31 + base.len() as u64);
    let b = &t.bases[t.base(base)];
    let mut it = Item { base: b.id.clone(), rarity: Rarity::Epic, ilvl: level, power: b.dmg * (1.0 + level as f32 * 0.08), affixes: Vec::new(), unique: None, spells: Vec::new(), grown: true, seed: 7 };
    if let Some(s) = school {
        let pool = t.spells_of(s);
        it.spells = pool.iter().take(4).map(|&i| t.spells[i].id.clone()).collect();
        let _ = chance(&mut r, 0.5);
    }
    it
}

/// Lines describing an item for tooltips.
pub fn describe(t: &Tables, it: &Item) -> Vec<(String, [f32; 3])> {
    let mut out = Vec::new();
    let b = it.base(t);
    let grey = [0.65, 0.65, 0.62];
    out.push((it.name(t), it.rarity.color()));
    let kind = match b.class {
        Some(c) => format!("{:?}{}", c, if it.two_handed(t) { " - TWO-HANDED" } else { "" }).to_uppercase(),
        None => format!("{:?}", b.slot).to_uppercase(),
    };
    out.push((format!("{} {} - ILVL {}", it.rarity.name(), kind, it.ilvl), grey));
    out.push((format!("SOURCE: {:?}", b.source).to_uppercase(), grey));
    if b.dmg > 0.0 {
        out.push((format!("DAMAGE {:.0}", it.power), [1.0, 1.0, 1.0]));
    } else if b.armor > 0.0 {
        out.push((format!("ARMOUR {:.0}", it.power), [1.0, 1.0, 1.0]));
    }
    if b.class == Some(WeaponClass::Shield) {
        out.push((format!("ARMOUR {:.0}", b.armor), [1.0, 1.0, 1.0]));
    }
    for (s, v) in &it.affixes {
        let sign = if *v >= 0.0 { "+" } else { "" };
        out.push((format!("{sign}{v} {}", s.label()), [0.55, 0.7, 1.0]));
    }
    for s in &it.spells {
        if let Some(d) = t.spells.iter().find(|x| &x.id == s) {
            out.push((format!("SPELL: {} ({:?}, {:.0} STRAIN)", d.name.to_uppercase(), d.school, d.strain).to_uppercase(), [0.9, 0.6, 1.0]));
            out.push((format!("  {}", d.desc.to_uppercase()), [0.6, 0.5, 0.7]));
        }
    }
    if let Some(u) = &it.unique {
        if let Some(d) = t.uniques.iter().find(|x| &x.id == u) {
            out.push((d.desc.to_uppercase(), [1.0, 0.65, 0.3]));
            out.push((format!("\"{}\"", d.flavor.to_uppercase()), grey));
        }
    }
    if it.grown {
        out.push(("PART OF YOU FOR THIS LIFE".into(), [1.0, 0.4, 0.5]));
    }
    out
}
