//! Texture layers: CC0 photo textures (assets/textures) combined with procedural overlays
//! (windows, signs, veins, glow lines), 512x512 RGBA, built at load time.
//!
//! Every layer comes as a pair: albedo (sRGB, alpha = cut-out, ambient occlusion folded in) and a
//! linear material layer (R/G tangent-space normal, B roughness, A metalness) for the PBR shader.
use crate::util::{fbm2, hash01, noise2};
use glam::{Vec2, Vec3};
use std::f32::consts::TAU;

pub const TEX: usize = 512;
/// Per-character textures (skin, eyes, hair) live in their own arrays at this size.
pub const CHAR_TEX: usize = 1024;
/// Layer values from here on select a per-character texture (`C:<name>` materials).
pub const CHAR_BASE: f32 = 300.0;

/// Names of the per-character textures (assets/characters/<name>.png), in layer order.
pub fn char_names() -> Vec<String> {
    let mut v: Vec<String> = crate::assets::list("characters/").iter().filter_map(|p| p.strip_prefix("characters/")?.strip_suffix(".png").map(|s| s.to_string())).filter(|s| !s.ends_with("_n")).collect();
    v.sort();
    v
}
pub fn char_layer(name: &str) -> Option<usize> {
    char_names().iter().position(|n| n == name)
}
/// (albedo RGBA8 sRGB, material RGBA8 linear) of one per-character texture.
pub fn char_texture(name: &str) -> (Vec<u8>, Vec<u8>) {
    let get = |p: &str, fill: [u8; 4]| match crate::assets::image(p) {
        Some(im) if im.w == CHAR_TEX && im.h == CHAR_TEX => im.px,
        _ => fill.iter().copied().cycle().take(CHAR_TEX * CHAR_TEX * 4).collect(),
    };
    (get(&format!("characters/{name}.png"), [180, 140, 120, 255]), get(&format!("characters/{name}_n.png"), [128, 128, 160, 0]))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, serde::Serialize, serde::Deserialize)]
#[repr(u32)]
pub enum Tex {
    White,
    Asphalt,
    RoadLine,
    Sidewalk,
    Brick,
    BrickWindow,
    BrickWindowLit,
    Concrete,
    ConcreteWindow,
    ConcreteWindowLit,
    Plaster,
    PlasterWindow,
    PlasterWindowLit,
    Shopfront,
    ShopfrontLit,
    Door,
    Roof,
    Metal,
    Rust,
    Wood,
    Crate,
    Fence,
    Camo,
    Cloth,
    Skin,
    Flesh,
    Vein,
    Bone,
    Chitin,
    Growth,
    GrowthGlow,
    RiftStone,
    RiftGlow,
    Char,
    Ember,
    Tile,
    Graffiti,
    Sign,
    Poster,
    Tarp,
    Sandbag,
    Glass,
    Grass,
    GutWall,
    Lattice,
    /// neutral detail layers for the realistic character models: colour comes from the vertices
    Leather,
    Hair,
    Denim,
    Count,
}

/// A photo texture from assets/textures, as linear-ish 0..1 RGB (sRGB values, as authored).
struct Photo {
    px: Vec<[f32; 3]>,
    mean_lum: f32,
    /// packed material map: normal x/y, roughness, ambient occlusion (0..1)
    mat: Vec<[f32; 4]>,
}
impl Photo {
    fn load(name: &str) -> Photo {
        let im = crate::assets::image(&format!("textures/{name}.png"));
        let px = match im {
            Some(im) if im.w == TEX && im.h == TEX => im.px.chunks(4).map(|c| [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0]).collect(),
            _ => vec![[0.5; 3]; TEX * TEX],
        };
        let mat = match crate::assets::image(&format!("textures/{name}_n.png")) {
            Some(im) if im.w == TEX && im.h == TEX => im.px.chunks(4).map(|c| [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, c[3] as f32 / 255.0]).collect(),
            _ => vec![[0.5, 0.5, 0.7, 1.0]; TEX * TEX],
        };
        let mean_lum = px.iter().map(|c| c[0] * 0.3 + c[1] * 0.55 + c[2] * 0.15).sum::<f32>() / px.len().max(1) as f32;
        Photo { px, mean_lum, mat }
    }
    fn mat_at(&self, x: usize, y: usize) -> [f32; 4] {
        self.mat[(y % TEX) * TEX + (x % TEX)]
    }
    fn at(&self, x: usize, y: usize) -> [f32; 3] {
        self.px[(y % TEX) * TEX + (x % TEX)]
    }
    fn lum(&self, x: usize, y: usize) -> f32 {
        let c = self.at(x, y);
        c[0] * 0.3 + c[1] * 0.55 + c[2] * 0.15
    }
}

fn px(t: &mut [u8], x: usize, y: usize, c: [f32; 3], a: f32) {
    let i = (y * TEX + x) * 4;
    t[i] = (c[0].clamp(0.0, 1.0) * 255.0) as u8;
    t[i + 1] = (c[1].clamp(0.0, 1.0) * 255.0) as u8;
    t[i + 2] = (c[2].clamp(0.0, 1.0) * 255.0) as u8;
    t[i + 3] = (a.clamp(0.0, 1.0) * 255.0) as u8;
}

fn mul(c: [f32; 3], k: f32) -> [f32; 3] {
    [c[0] * k, c[1] * k, c[2] * k]
}
fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}
fn tint(c: [f32; 3], t: [f32; 3]) -> [f32; 3] {
    [c[0] * t[0], c[1] * t[1], c[2] * t[2]]
}
fn grey(c: [f32; 3]) -> [f32; 3] {
    let l = c[0] * 0.3 + c[1] * 0.55 + c[2] * 0.15;
    [l, l, l]
}

/// Tiling value noise over the texture (wraps at the edge), u/v in 0..1.
fn tn(u: f32, v: f32, freq: f32, seed: u32) -> f32 {
    let f = freq as i32;
    let p = Vec2::new(u, v) * freq;
    let i = p.floor();
    let fr = p - i;
    let s = fr * fr * (Vec2::splat(3.0) - 2.0 * fr);
    let h = |a: i32, b: i32| hash01(a.rem_euclid(f), b.rem_euclid(f), seed);
    let (xi, yi) = (i.x as i32, i.y as i32);
    let a = h(xi, yi) + (h(xi + 1, yi) - h(xi, yi)) * s.x;
    let b = h(xi, yi + 1) + (h(xi + 1, yi + 1) - h(xi, yi + 1)) * s.x;
    a + (b - a) * s.y
}
fn tfbm(u: f32, v: f32, freq: f32, seed: u32) -> f32 {
    tn(u, v, freq, seed) * 0.5 + tn(u, v, freq * 2.0, seed + 7) * 0.3 + tn(u, v, freq * 4.0, seed + 13) * 0.2
}
/// Thin wandering lines (cracks, veins): 1 on the line, 0 away from it.
fn veins(u: f32, v: f32, freq: f32, seed: u32, width: f32) -> f32 {
    let d = (tn(u, v, freq, seed) - 0.5).abs();
    (1.0 - d / width).clamp(0.0, 1.0)
}

/// Window in a facade bay (u, v in 0..1): None outside, Some(true) for frame, Some(false) for glass.
fn window_mask(u: f32, v: f32) -> Option<bool> {
    if (0.22..0.78).contains(&u) && (0.16..0.72).contains(&v) {
        let frame = u < 0.25 || u >= 0.75 || v < 0.19 || v >= 0.69 || (0.485..0.515).contains(&u) || (0.42..0.45).contains(&v);
        Some(frame)
    } else {
        None
    }
}
fn with_window(base: [f32; 3], u: f32, v: f32, lit: bool, seed: u32) -> [f32; 3] {
    match window_mask(u, v) {
        None => {
            // stone sill and lintel, plus a grime streak under the sill
            if (0.19..0.81).contains(&u) && ((0.72..0.76).contains(&v) || (0.12..0.16).contains(&v)) {
                mix(base, [0.62, 0.6, 0.56], 0.7)
            } else if (0.24..0.76).contains(&u) && v > 0.76 {
                mul(base, 1.0 - 0.25 * (1.0 - (v - 0.76) / 0.24) * tn(u, 0.0, 16.0, seed + 3))
            } else {
                base
            }
        }
        Some(true) => [0.11, 0.1, 0.09],
        Some(false) => {
            if lit {
                let blind = if v < 0.3 { 0.72 } else { 1.0 };
                let room = tn(u, v, 3.0, seed + 40);
                mul([1.0, 0.8, 0.45], blind * (0.7 + room * 0.35))
            } else {
                // reflections of the sky and grime on the pane
                let r = 0.07 + 0.12 * (u + v * 0.4) * tn(u, v, 4.0, seed + 41) + 0.05 * tfbm(u, v, 16.0, seed + 42);
                [r * 0.6, r * 0.8, r]
            }
        }
    }
}

/// Physical surface response of a layer: (roughness override or -1 to use the photo map,
/// roughness multiplier, metalness, strength of the bump derived from the albedo).
fn surface(layer: Tex) -> (f32, f32, f32, f32) {
    match layer {
        Tex::White => (0.6, 1.0, 0.0, 0.0),
        Tex::Asphalt | Tex::RoadLine => (-1.0, 1.0, 0.0, 0.6),
        Tex::Sidewalk | Tex::Concrete | Tex::Plaster | Tex::Tile => (-1.0, 1.0, 0.0, 0.5),
        Tex::Brick => (-1.0, 1.0, 0.0, 0.4),
        Tex::BrickWindow | Tex::ConcreteWindow | Tex::PlasterWindow | Tex::BrickWindowLit | Tex::ConcreteWindowLit | Tex::PlasterWindowLit => (-1.0, 1.0, 0.0, 1.2),
        Tex::Shopfront | Tex::ShopfrontLit => (-1.0, 0.7, 0.6, 1.0),
        Tex::Door => (-1.0, 1.0, 0.0, 1.4),
        Tex::Roof => (-1.0, 1.0, 0.0, 0.5),
        Tex::Metal => (-1.0, 0.9, 1.0, 0.3),
        Tex::Rust => (-1.0, 1.1, 0.35, 0.6),
        Tex::Wood | Tex::Crate => (-1.0, 1.0, 0.0, 0.7),
        Tex::Fence => (0.45, 1.0, 1.0, 0.3),
        Tex::Camo | Tex::Cloth | Tex::Tarp | Tex::Sandbag => (-1.0, 1.15, 0.0, 0.6),
        Tex::Skin => (0.48, 1.0, 0.0, 0.35),
        Tex::Flesh => (0.3, 1.0, 0.0, 1.0),
        Tex::Vein => (0.28, 1.0, 0.0, 1.2),
        Tex::Bone => (-1.0, 0.8, 0.0, 0.8),
        Tex::Chitin => (0.32, 1.0, 0.0, 0.9),
        Tex::Growth => (0.35, 1.0, 0.0, 1.0),
        Tex::GrowthGlow => (0.25, 1.0, 0.0, 0.8),
        Tex::RiftStone => (-1.0, 0.9, 0.0, 0.7),
        Tex::RiftGlow => (-1.0, 0.7, 0.0, 0.7),
        Tex::Char => (-1.0, 1.2, 0.0, 0.8),
        Tex::Ember => (-1.0, 1.0, 0.0, 0.6),
        Tex::Graffiti => (-1.0, 0.9, 0.0, 0.4),
        Tex::Sign => (0.35, 1.0, 0.2, 0.6),
        Tex::Poster => (-1.0, 1.0, 0.0, 0.3),
        Tex::Glass => (0.06, 1.0, 0.0, 0.1),
        Tex::Grass => (-1.0, 1.1, 0.0, 0.6),
        Tex::GutWall => (0.22, 1.0, 0.0, 1.3),
        Tex::Lattice => (0.2, 1.0, 0.6, 0.8),
        Tex::Leather => (-1.0, 1.3, 0.0, 0.6),
        Tex::Hair => (0.42, 1.0, 0.0, 1.4),
        Tex::Denim => (-1.0, 1.1, 0.0, 0.7),
        Tex::Count => (0.6, 1.0, 0.0, 0.0),
    }
}

/// Generate one layer: (albedo RGBA8 sRGB, material RGBA8 linear).
pub fn generate(layer: Tex) -> (Vec<u8>, Vec<u8>) {
    let mut t = vec![255u8; TEX * TEX * 4];
    let s = layer as u32 * 97 + 11;
    let photo = |name: &str| Photo::load(name);
    let base = match layer {
        Tex::Asphalt | Tex::RoadLine => Some(photo("asphalt")),
        Tex::Sidewalk => Some(photo("paving")),
        Tex::Brick | Tex::BrickWindow | Tex::BrickWindowLit => Some(photo("brick")),
        Tex::Concrete | Tex::ConcreteWindow | Tex::ConcreteWindowLit | Tex::Graffiti => Some(photo("concrete")),
        Tex::Plaster | Tex::PlasterWindow | Tex::PlasterWindowLit => Some(photo("plaster")),
        Tex::Shopfront | Tex::ShopfrontLit | Tex::Metal => Some(photo("metal")),
        Tex::Door | Tex::Wood => Some(photo("wood")),
        Tex::Crate => Some(photo("crate")),
        Tex::Roof => Some(photo("roof")),
        Tex::Rust => Some(photo("rust")),
        Tex::Camo | Tex::Cloth | Tex::Sandbag => Some(photo("fabric")),
        Tex::Tarp => Some(photo("fabric_coarse")),
        Tex::Skin | Tex::Flesh | Tex::Vein | Tex::GutWall => Some(photo("leather")),
        Tex::Bone => Some(photo("bone")),
        Tex::Chitin => Some(photo("chitin")),
        Tex::Growth | Tex::GrowthGlow => Some(photo("moss_rock")),
        Tex::RiftStone | Tex::RiftGlow => Some(photo("rock")),
        Tex::Char => Some(photo("gravel")),
        Tex::Ember => Some(photo("lava")),
        Tex::Tile => Some(photo("tiles")),
        Tex::Poster | Tex::Sign => Some(photo("paper")),
        Tex::Grass => Some(photo("grass")),
        Tex::Leather => Some(photo("leather")),
        Tex::Denim => Some(photo("fabric_coarse")),
        Tex::Hair => Some(photo("fabric")),
        _ => None,
    };
    // detail layers are normalised to a bright neutral grey so vertex colours set the hue
    let detail = |p: &Option<Photo>, x: usize, y: usize| -> f32 {
        p.as_ref().map(|p| p.lum(x, y) / p.mean_lum.max(0.05) * 0.8).unwrap_or(0.8)
    };
    for y in 0..TEX {
        for x in 0..TEX {
            let (u, v) = ((x as f32 + 0.5) / TEX as f32, (y as f32 + 0.5) / TEX as f32);
            let b = base.as_ref().map(|p| p.at(x, y)).unwrap_or([1.0; 3]);
            let l = base.as_ref().map(|p| p.lum(x, y)).unwrap_or(1.0);
            let n = tfbm(u, v, 6.0, s);
            let mut a = 1.0;
            let c: [f32; 3] = match layer {
                Tex::White | Tex::Count => [1.0, 1.0, 1.0],
                Tex::Asphalt => mul(b, 0.8 * (1.0 - 0.45 * veins(u, v, 5.0, s, 0.02))),
                Tex::RoadLine => {
                    let dash = (0.42..0.58).contains(&u) && v < 0.62 && tn(u, v, 32.0, s) > 0.2;
                    if dash { mix(mul(b, 0.8), [0.85, 0.75, 0.32], 0.85 * (0.7 + 0.3 * l)) } else { mul(b, 0.8) }
                }
                Tex::Sidewalk => mul(b, 0.95),
                Tex::Brick | Tex::Concrete | Tex::Plaster => b,
                Tex::BrickWindow | Tex::ConcreteWindow | Tex::PlasterWindow => with_window(b, u, v, false, s),
                Tex::BrickWindowLit | Tex::ConcreteWindowLit | Tex::PlasterWindowLit => with_window(b, u, v, true, s),
                Tex::Shopfront | Tex::ShopfrontLit => {
                    let lit = layer == Tex::ShopfrontLit;
                    if v < 0.19 {
                        // sign band: weathered paint with lettering blocks
                        let glyph = hash01((u * 14.0) as i32, 0, s) > 0.35 && (0.05..0.14).contains(&v) && (u * 14.0).fract() < 0.7 && (0.1..0.9).contains(&u);
                        let paint = if lit { [0.9, 0.25, 0.2] } else { [0.32, 0.18, 0.16] };
                        if glyph { if lit { [1.0, 0.92, 0.7] } else { [0.55, 0.5, 0.42] } } else { mul(paint, 0.7 + 0.4 * l) }
                    } else if v > 0.88 || u < 0.05 || u > 0.95 || (0.48..0.52).contains(&u) {
                        mul(b, 0.55)
                    } else if lit {
                        mul([0.85, 0.9, 0.7], 0.55 + tn(u, v, 4.0, s) * 0.5)
                    } else {
                        let r = 0.05 + 0.08 * v + 0.05 * tfbm(u, v, 12.0, s);
                        [r * 0.7, r * 0.9, r]
                    }
                }
                Tex::Door => {
                    let panel = (0.12..0.88).contains(&u) && ((0.08..0.44).contains(&v) || (0.53..0.94).contains(&v));
                    let knob = (0.78..0.85).contains(&u) && (0.47..0.53).contains(&v);
                    if knob { [0.7, 0.6, 0.3] } else { mul(tint(b, [0.75, 0.62, 0.55]), if panel { 0.8 } else { 1.0 }) }
                }
                Tex::Roof => mul(b, 0.7),
                Tex::Metal | Tex::Rust => b,
                Tex::Wood => b,
                Tex::Crate => {
                    let edge = u < 0.07 || u > 0.93 || v < 0.07 || v > 0.93 || (u - v).abs() < 0.05;
                    mul(b, if edge { 1.15 } else { 0.85 })
                }
                Tex::Fence => {
                    let (fu, fv) = (u * 16.0, v * 16.0);
                    let d1 = ((fu + fv).fract() - 0.5).abs() < 0.07;
                    let d2 = ((fu - fv).fract() - 0.5).abs() < 0.07;
                    let post = v < 0.04 || v > 0.96;
                    a = if d1 || d2 || post { 1.0 } else { 0.0 };
                    mul([0.5, 0.52, 0.54], 0.8 + 0.4 * tn(u, v, 8.0, s))
                }
                Tex::Camo => {
                    let k = tn(u, v, 5.0, s);
                    let k2 = tn(u, v, 7.0, s + 31);
                    let c = if k > 0.6 { [0.16, 0.2, 0.12] } else if k2 > 0.55 { [0.3, 0.27, 0.17] } else { [0.24, 0.29, 0.17] };
                    mul(c, 0.6 + 0.8 * l)
                }
                Tex::Cloth => mul(grey(b), 1.35),
                Tex::Skin => mul([0.85, 0.66, 0.55], 0.75 + 0.4 * (0.45 + (l - 0.45) * 0.4) + 0.06 * tfbm(u, v, 3.0, s)),
                Tex::Flesh => {
                    let v1 = veins(u, v, 7.0, s + 2, 0.03);
                    mix(tint(mul(b, 1.4), [1.05, 0.38, 0.4]), [0.25, 0.04, 0.08], v1 * 0.7)
                }
                Tex::Vein => {
                    let v1 = veins(u, v, 6.0, s + 2, 0.04);
                    mix(tint(mul(b, 0.7), [0.6, 0.25, 0.35]), [0.95, 0.35, 0.5], v1)
                }
                Tex::Bone => tint(b, [1.0, 0.95, 0.82]),
                Tex::Chitin => tint(mul(b, 0.8), [0.75, 0.6, 1.0]),
                Tex::Growth => {
                    let cell = (tn(u, v, 8.0, s + 4) * 6.0).fract();
                    mix(tint(mul(grey(b), 1.2), [0.55, 0.25, 0.5]), [0.5, 0.18, 0.4], cell * n * 0.6)
                }
                Tex::GrowthGlow => {
                    let v1 = veins(u, v, 6.0, s + 2, 0.05);
                    mix(mul(tint(grey(b), [0.35, 0.25, 0.45]), 0.5), [0.45, 1.0, 0.75], v1)
                }
                Tex::RiftStone => tint(mul(grey(b), 1.1), [0.55, 0.45, 0.85]),
                Tex::RiftGlow => {
                    let line = ((u * 4.0 + tn(u, v, 4.0, s) * 2.0).fract() - 0.5).abs() < 0.06;
                    if line { [0.75, 0.5, 1.0] } else { mul(tint(grey(b), [0.35, 0.25, 0.6]), 0.55) }
                }
                Tex::Char => mul(grey(b), 0.35),
                Tex::Ember => mul(b, 1.1),
                Tex::Tile => b,
                Tex::Graffiti => {
                    let k = fbm2(Vec2::new(u, v) * 3.0 + Vec2::splat(3.7), s, 3);
                    let line = ((u * 2.5 + (v * 9.0).sin() * 0.25).fract() - 0.5).abs() < 0.09 && (0.25..0.8).contains(&v);
                    let spray = 0.75 + 0.25 * tn(u, v, 40.0, s + 9);
                    if line && k > 0.35 { mix(b, [0.85, 0.12, 0.25], 0.85 * spray) } else if k > 0.68 { mix(b, [0.1, 0.55, 0.5], 0.55 * spray) } else { b }
                }
                Tex::Sign => {
                    let border = u < 0.05 || u > 0.95 || v < 0.05 || v > 0.95;
                    let glyph = hash01((u * 11.0) as i32, (v * 6.4) as i32, s) > 0.45 && (u * 11.0).fract() < 0.65 && (v * 6.4).fract() > 0.25 && (v * 6.4).fract() < 0.75 && (0.12..0.88).contains(&v);
                    if border { [0.9, 0.9, 0.85] } else if glyph { [1.0, 0.95, 0.7] } else { [0.1, 0.18, 0.3] }
                }
                Tex::Poster => {
                    let hazard = v < 0.2 && ((u + v) * 10.0) as i32 % 2 == 0;
                    let text = ((v * 5.3) as i32 % 2 == 0) && (0.1..0.9).contains(&u) && hash01((u * 30.0) as i32, (v * 30.0) as i32, s) > 0.3 && v > 0.25;
                    if hazard { [0.9, 0.75, 0.1] } else if text { [0.15, 0.13, 0.12] } else { mul(tint(b, [1.0, 0.97, 0.88]), 1.0) }
                }
                Tex::Tarp => tint(mul(grey(b), 1.2), [0.42, 0.55, 0.38]),
                Tex::Sandbag => {
                    let row = (v * 5.0) as i32;
                    let uo = (u * 2.6 + if row % 2 == 0 { 0.0 } else { 0.5 }).fract();
                    let edge = (v * 5.0).fract() < 0.08 || uo < 0.04;
                    mul(tint(grey(b), [0.95, 0.82, 0.58]), if edge { 0.55 } else { 1.0 })
                }
                Tex::Glass => {
                    let r = 0.2 + 0.25 * noise2(Vec2::new(u * 3.0, v * 3.0), s);
                    [r * 0.6, r * 0.85, r]
                }
                Tex::Grass => mul(b, 0.85),
                Tex::GutWall => {
                    let fold = ((v * 6.0 + tn(u, v, 4.0, s) * 2.0).fract() - 0.5).abs() * 2.0;
                    tint(mul(b, 0.6 + fold * 0.9), [1.1, 0.4, 0.45])
                }
                Tex::Leather => {
                    // creased hide: the grain, plus scuffs that catch the light
                    let k = detail(&base, x, y) * (0.9 + 0.2 * tfbm(u, v, 5.0, s));
                    [k, k * 0.98, k * 0.95]
                }
                Tex::Denim => {
                    let k = detail(&base, x, y) * (0.92 + 0.12 * tn(u, v, 24.0, s));
                    [k, k, k]
                }
                Tex::Hair => {
                    // fine strands running down the texture, clumped by low-frequency noise
                    let clump = tn(u, v, 6.0, s);
                    let strand = 0.5 + 0.5 * ((u * TAU * 96.0 + clump * 9.0 + tn(u, v, 3.0, s + 1) * 5.0).sin());
                    let k = 0.8 * (0.62 + 0.38 * strand * (0.7 + 0.3 * tn(u, v, 32.0, s + 2)));
                    [k, k, k]
                }
                Tex::Lattice => {
                    let g = (u * 4.0).fract() < 0.06 || (v * 4.0).fract() < 0.06;
                    if g { [0.6, 0.75, 1.0] } else { mul([0.05, 0.06, 0.14], 0.6 + n * 0.8) }
                }
            };
            let ao = base.as_ref().map(|p| p.mat_at(x, y)[3]).unwrap_or(1.0);
            px(&mut t, x, y, mul(c, 0.35 + 0.65 * ao), a);
        }
    }
    let m = material(layer, &t, base.as_ref());
    (t, m)
}

/// Material layer: the photo's normal and roughness maps, plus a bump derived from the final
/// albedo so procedural overlays (window frames, panels, veins, lettering) get relief too.
fn material(layer: Tex, albedo: &[u8], photo: Option<&Photo>) -> Vec<u8> {
    let (rough_fix, rough_mul, metal, bump) = surface(layer);
    let lum = |x: usize, y: usize| {
        let i = ((y % TEX) * TEX + (x % TEX)) * 4;
        (albedo[i] as f32 * 0.3 + albedo[i + 1] as f32 * 0.55 + albedo[i + 2] as f32 * 0.15) / 255.0
    };
    let mut out = vec![0u8; TEX * TEX * 4];
    for y in 0..TEX {
        for x in 0..TEX {
            let (xm, xp, ym, yp) = ((x + TEX - 1) % TEX, (x + 1) % TEX, (y + TEX - 1) % TEX, (y + 1) % TEX);
            // Sobel gradient of the albedo luminance, read as height
            let gx = (lum(xp, ym) + 2.0 * lum(xp, y) + lum(xp, yp)) - (lum(xm, ym) + 2.0 * lum(xm, y) + lum(xm, yp));
            let gy = (lum(xm, yp) + 2.0 * lum(x, yp) + lum(xp, yp)) - (lum(xm, ym) + 2.0 * lum(x, ym) + lum(xp, ym));
            let pm = photo.map(|p| p.mat_at(x, y)).unwrap_or([0.5, 0.5, 0.7, 1.0]);
            // photo normals are OpenGL convention (+Y up the image); image rows run downward
            // body surfaces seen at arm's length: soften relief so a weave or grain never reads as a grid
            let relief = match layer {
                Tex::Skin => 0.3,
                Tex::Cloth | Tex::Camo => 0.45,
                Tex::Denim => 0.55,
                Tex::Leather | Tex::Hair => 0.7,
                _ => 1.0,
            };
            let mut n = Vec3::new((pm[0] * 2.0 - 1.0) * relief, (pm[1] * 2.0 - 1.0) * relief, 1.0);
            n.x -= gx * bump * 0.9 * relief;
            n.y += gy * bump * 0.9 * relief;
            let n = n.normalize();
            let rough = if rough_fix >= 0.0 { (rough_fix * (0.85 + 0.3 * pm[2])).min(1.0) } else { (pm[2] * rough_mul).clamp(0.04, 1.0) };
            let i = (y * TEX + x) * 4;
            out[i] = ((n.x * 0.5 + 0.5) * 255.0).round() as u8;
            out[i + 1] = ((n.y * 0.5 + 0.5) * 255.0).round() as u8;
            out[i + 2] = (rough * 255.0).round() as u8;
            out[i + 3] = (metal * 255.0).round() as u8;
        }
    }
    out
}

/// Box-filter downsample one RGBA level. `cutout` thresholds alpha so cut-outs stay crisp.
pub fn downsample(src: &[u8], size: usize, cutout: bool) -> Vec<u8> {
    let h = size / 2;
    let mut out = vec![0u8; h * h * 4];
    for y in 0..h {
        for x in 0..h {
            for c in 0..4 {
                let s = |xx: usize, yy: usize| src[(yy * size + xx) * 4 + c] as u32;
                let v = (s(x * 2, y * 2) + s(x * 2 + 1, y * 2) + s(x * 2, y * 2 + 1) + s(x * 2 + 1, y * 2 + 1)) / 4;
                // keep cut-outs crisp
                out[(y * h + x) * 4 + c] = if c == 3 && cutout { if v > 96 { 255 } else { 0 } } else { v as u8 };
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// UI font: 5x7 glyphs for ASCII 32..=96 (lowercase maps to uppercase).
#[rustfmt::skip]
const FONT: [[u8; 7]; 65] = [
    [0x00,0x00,0x00,0x00,0x00,0x00,0x00],[0x04,0x04,0x04,0x04,0x00,0x00,0x04],[0x0A,0x0A,0x0A,0x00,0x00,0x00,0x00],
    [0x0A,0x0A,0x1F,0x0A,0x1F,0x0A,0x0A],[0x04,0x0F,0x14,0x0E,0x05,0x1E,0x04],[0x18,0x19,0x02,0x04,0x08,0x13,0x03],
    [0x0C,0x12,0x14,0x08,0x15,0x12,0x0D],[0x0C,0x04,0x08,0x00,0x00,0x00,0x00],[0x02,0x04,0x08,0x08,0x08,0x04,0x02],
    [0x08,0x04,0x02,0x02,0x02,0x04,0x08],[0x00,0x04,0x15,0x0E,0x15,0x04,0x00],[0x00,0x04,0x04,0x1F,0x04,0x04,0x00],
    [0x00,0x00,0x00,0x00,0x0C,0x04,0x08],[0x00,0x00,0x00,0x1F,0x00,0x00,0x00],[0x00,0x00,0x00,0x00,0x00,0x0C,0x0C],
    [0x00,0x01,0x02,0x04,0x08,0x10,0x00],[0x0E,0x11,0x13,0x15,0x19,0x11,0x0E],[0x04,0x0C,0x04,0x04,0x04,0x04,0x0E],
    [0x0E,0x11,0x01,0x02,0x04,0x08,0x1F],[0x1F,0x02,0x04,0x02,0x01,0x11,0x0E],[0x02,0x06,0x0A,0x12,0x1F,0x02,0x02],
    [0x1F,0x10,0x1E,0x01,0x01,0x11,0x0E],[0x06,0x08,0x10,0x1E,0x11,0x11,0x0E],[0x1F,0x01,0x02,0x04,0x08,0x08,0x08],
    [0x0E,0x11,0x11,0x0E,0x11,0x11,0x0E],[0x0E,0x11,0x11,0x0F,0x01,0x02,0x0C],[0x00,0x0C,0x0C,0x00,0x0C,0x0C,0x00],
    [0x00,0x0C,0x0C,0x00,0x0C,0x04,0x08],[0x02,0x04,0x08,0x10,0x08,0x04,0x02],[0x00,0x00,0x1F,0x00,0x1F,0x00,0x00],
    [0x08,0x04,0x02,0x01,0x02,0x04,0x08],[0x0E,0x11,0x01,0x02,0x04,0x00,0x04],[0x0E,0x11,0x01,0x0D,0x15,0x15,0x0E],
    [0x0E,0x11,0x11,0x11,0x1F,0x11,0x11],[0x1E,0x11,0x11,0x1E,0x11,0x11,0x1E],[0x0E,0x11,0x10,0x10,0x10,0x11,0x0E],
    [0x1C,0x12,0x11,0x11,0x11,0x12,0x1C],[0x1F,0x10,0x10,0x1E,0x10,0x10,0x1F],[0x1F,0x10,0x10,0x1E,0x10,0x10,0x10],
    [0x0E,0x11,0x10,0x17,0x11,0x11,0x0F],[0x11,0x11,0x11,0x1F,0x11,0x11,0x11],[0x0E,0x04,0x04,0x04,0x04,0x04,0x0E],
    [0x07,0x02,0x02,0x02,0x02,0x12,0x0C],[0x11,0x12,0x14,0x18,0x14,0x12,0x11],[0x10,0x10,0x10,0x10,0x10,0x10,0x1F],
    [0x11,0x1B,0x15,0x15,0x11,0x11,0x11],[0x11,0x11,0x19,0x15,0x13,0x11,0x11],[0x0E,0x11,0x11,0x11,0x11,0x11,0x0E],
    [0x1E,0x11,0x11,0x1E,0x10,0x10,0x10],[0x0E,0x11,0x11,0x11,0x15,0x12,0x0D],[0x1E,0x11,0x11,0x1E,0x14,0x12,0x11],
    [0x0F,0x10,0x10,0x0E,0x01,0x01,0x1E],[0x1F,0x04,0x04,0x04,0x04,0x04,0x04],[0x11,0x11,0x11,0x11,0x11,0x11,0x0E],
    [0x11,0x11,0x11,0x11,0x11,0x0A,0x04],[0x11,0x11,0x11,0x15,0x15,0x15,0x0A],[0x11,0x11,0x0A,0x04,0x0A,0x11,0x11],
    [0x11,0x11,0x11,0x0A,0x04,0x04,0x04],[0x1F,0x01,0x02,0x04,0x08,0x10,0x1F],[0x0E,0x08,0x08,0x08,0x08,0x08,0x0E],
    [0x00,0x10,0x08,0x04,0x02,0x01,0x00],[0x0E,0x02,0x02,0x02,0x02,0x02,0x0E],[0x04,0x0A,0x11,0x00,0x00,0x00,0x00],
    [0x00,0x00,0x00,0x00,0x00,0x00,0x1F],[0x08,0x04,0x02,0x00,0x00,0x00,0x00],
];

pub const UI_ATLAS_W: usize = 128;
pub const UI_ATLAS_H: usize = 64;
pub const GLYPH_W: usize = 6;
pub const GLYPH_H: usize = 8;

/// UV rect (pixels) of a glyph in the UI atlas.
pub fn glyph_cell(ch: char) -> (usize, usize) {
    let mut c = ch as u32;
    if (97..=122).contains(&c) {
        c -= 32;
    }
    let idx = if (32..=96).contains(&c) { c - 32 } else { 31 } as usize;
    ((idx % 16) * GLYPH_W, (idx / 16) * GLYPH_H)
}
/// A solid white texel block lives here.
pub const UI_WHITE: (usize, usize) = (120, 56);

pub fn ui_atlas() -> Vec<u8> {
    let mut t = vec![0u8; UI_ATLAS_W * UI_ATLAS_H * 4];
    for (i, g) in FONT.iter().enumerate() {
        let (ox, oy) = ((i % 16) * GLYPH_W, (i / 16) * GLYPH_H);
        for (r, row) in g.iter().enumerate() {
            for b in 0..5 {
                if row & (0x10 >> b) != 0 {
                    let p = ((oy + r) * UI_ATLAS_W + ox + b) * 4;
                    t[p..p + 4].copy_from_slice(&[255, 255, 255, 255]);
                }
            }
        }
    }
    for y in 52..64 {
        for x in 116..128 {
            let p = (y * UI_ATLAS_W + x) * 4;
            t[p..p + 4].copy_from_slice(&[255, 255, 255, 255]);
        }
    }
    t
}
