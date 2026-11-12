//! Procedural texture generation. Every layer is 64x64 RGBA, generated at load time.
use crate::util::{fbm2, hash01, noise2};
use glam::Vec2;

pub const TEX: usize = 64;

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
    Count,
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

/// Tiling noise helper (wraps at the texture edge so layers repeat seamlessly).
fn tn(x: usize, y: usize, freq: f32, seed: u32) -> f32 {
    let f = freq as i32;
    let p = Vec2::new(x as f32, y as f32) / TEX as f32 * freq;
    let i = p.floor();
    let fr = p - i;
    let u = fr * fr * (Vec2::splat(3.0) - 2.0 * fr);
    let h = |a: i32, b: i32| hash01(a.rem_euclid(f), b.rem_euclid(f), seed);
    let (xi, yi) = (i.x as i32, i.y as i32);
    let a = h(xi, yi) + (h(xi + 1, yi) - h(xi, yi)) * u.x;
    let b = h(xi, yi + 1) + (h(xi + 1, yi + 1) - h(xi, yi + 1)) * u.x;
    a + (b - a) * u.y
}
fn tfbm(x: usize, y: usize, freq: f32, seed: u32) -> f32 {
    tn(x, y, freq, seed) * 0.5 + tn(x, y, freq * 2.0, seed + 7) * 0.3 + tn(x, y, freq * 4.0, seed + 13) * 0.2
}

fn brick(x: usize, y: usize, seed: u32) -> [f32; 3] {
    let row = y / 8;
    let xo = (x + if row % 2 == 0 { 0 } else { 8 }) % TEX;
    let mortar = y % 8 == 0 || xo % 16 == 0;
    let id = hash01((xo / 16) as i32, row as i32, seed);
    let n = tfbm(x, y, 16.0, seed);
    if mortar {
        mul([0.42, 0.40, 0.37], 0.7 + n * 0.3)
    } else {
        mul([0.45 + id * 0.15, 0.22 + id * 0.06, 0.17], 0.65 + n * 0.5)
    }
}
fn concrete(x: usize, y: usize, seed: u32) -> [f32; 3] {
    let n = tfbm(x, y, 8.0, seed);
    let streak = tn(x, 0, 16.0, seed + 3) * (y as f32 / TEX as f32);
    let seam = if y % 32 == 0 || x % 32 == 0 { 0.8 } else { 1.0 };
    mul([0.52, 0.52, 0.5], (0.6 + n * 0.45 - streak * 0.2) * seam)
}
fn plaster(x: usize, y: usize, seed: u32) -> [f32; 3] {
    let n = tfbm(x, y, 6.0, seed);
    let crack = if tn(x, y, 12.0, seed + 5) > 0.5 && (tn(x, y, 12.0, seed + 5) - 0.52).abs() < 0.015 { 0.6 } else { 1.0 };
    mul([0.62, 0.56, 0.44], (0.65 + n * 0.4) * crack)
}
/// Returns Some(pane brightness) inside the window area of a 64x64 facade bay.
fn window_mask(x: usize, y: usize) -> Option<f32> {
    if (14..50).contains(&x) && (10..46).contains(&y) {
        let frame = x < 16 || x >= 48 || y < 12 || y >= 44 || (31..33).contains(&x) || (27..29).contains(&y);
        Some(if frame { 0.0 } else { 1.0 })
    } else {
        None
    }
}
fn with_window(base: [f32; 3], x: usize, y: usize, lit: bool, seed: u32) -> [f32; 3] {
    match window_mask(x, y) {
        None => {
            // sill
            if (12..52).contains(&x) && (46..49).contains(&y) {
                mul(base, 1.25)
            } else {
                base
            }
        }
        Some(p) if p < 0.5 => [0.13, 0.12, 0.11],
        Some(_) => {
            if lit {
                let blind = if y < 20 { 0.75 } else { 1.0 };
                let sil = tn(x, y, 4.0, seed + 40);
                mul([1.0, 0.82, 0.48], blind * (0.75 + sil * 0.3))
            } else {
                let refl = 0.08 + 0.1 * ((x + y) as f32 / 96.0);
                [refl * 0.6, refl * 0.8, refl]
            }
        }
    }
}

pub fn generate(layer: Tex) -> Vec<u8> {
    let mut t = vec![255u8; TEX * TEX * 4];
    let s = layer as u32 * 97 + 11;
    for y in 0..TEX {
        for x in 0..TEX {
            let n = tfbm(x, y, 8.0, s);
            let fine = hash01(x as i32, y as i32, s);
            let (fx, fy) = (x as f32 / TEX as f32, y as f32 / TEX as f32);
            let mut a = 1.0;
            let c: [f32; 3] = match layer {
                Tex::White | Tex::Count => [1.0, 1.0, 1.0],
                Tex::Asphalt => {
                    let crack = if (tn(x, y, 6.0, s + 9) - 0.5).abs() < 0.02 { 0.55 } else { 1.0 };
                    mul([0.2, 0.2, 0.22], (0.6 + n * 0.4 + fine * 0.2) * crack)
                }
                Tex::RoadLine => {
                    let base = mul([0.2, 0.2, 0.22], 0.6 + n * 0.4 + fine * 0.2);
                    if (28..36).contains(&x) && y % 64 < 40 && fine > 0.15 {
                        mix(base, [0.8, 0.72, 0.35], 0.85)
                    } else {
                        base
                    }
                }
                Tex::Sidewalk => {
                    let seam = if x % 32 == 0 || y % 32 == 0 { 0.6 } else { 1.0 };
                    mul([0.48, 0.47, 0.45], (0.65 + n * 0.35 + fine * 0.1) * seam)
                }
                Tex::Brick => brick(x, y, s),
                Tex::BrickWindow => with_window(brick(x, y, s), x, y, false, s),
                Tex::BrickWindowLit => with_window(brick(x, y, s), x, y, true, s),
                Tex::Concrete => concrete(x, y, s),
                Tex::ConcreteWindow => with_window(concrete(x, y, s), x, y, false, s),
                Tex::ConcreteWindowLit => with_window(concrete(x, y, s), x, y, true, s),
                Tex::Plaster => plaster(x, y, s),
                Tex::PlasterWindow => with_window(plaster(x, y, s), x, y, false, s),
                Tex::PlasterWindowLit => with_window(plaster(x, y, s), x, y, true, s),
                Tex::Shopfront | Tex::ShopfrontLit => {
                    let lit = layer == Tex::ShopfrontLit;
                    if y < 12 {
                        // sign band
                        let stripe = ((x / 4 + y / 4) % 2) as f32;
                        if lit { mix([0.9, 0.25, 0.2], [1.0, 0.9, 0.7], stripe * 0.6) } else { mul([0.35, 0.2, 0.18], 0.7 + stripe * 0.3) }
                    } else if y > 56 || x < 3 || x > 60 || (30..34).contains(&x) {
                        mul([0.2, 0.2, 0.21], 0.7 + n * 0.4)
                    } else if lit {
                        mul([0.85, 0.9, 0.7], 0.6 + tn(x, y, 5.0, s) * 0.5)
                    } else {
                        let r = 0.06 + 0.08 * fy;
                        [r * 0.7, r * 0.9, r]
                    }
                }
                Tex::Door => {
                    let panel = (8..56).contains(&x) && ((6..28).contains(&y) || (34..60).contains(&y));
                    let k = if panel { 0.8 } else { 1.0 };
                    let knob = (50..54).contains(&x) && (30..34).contains(&y);
                    if knob { [0.7, 0.6, 0.3] } else { mul([0.3, 0.2, 0.14], k * (0.7 + tn(x, y * 4, 8.0, s) * 0.4)) }
                }
                Tex::Roof => mul([0.16, 0.15, 0.15], 0.6 + n * 0.5 + fine * 0.15),
                Tex::Metal => {
                    let rivet = (x % 16 == 2 || x % 16 == 13) && (y % 16 == 2 || y % 16 == 13);
                    let seam = if x % 16 == 0 || y % 32 == 0 { 0.6 } else { 1.0 };
                    if rivet { [0.6, 0.6, 0.62] } else { mul([0.4, 0.42, 0.45], (0.7 + n * 0.3) * seam) }
                }
                Tex::Rust => mix([0.35, 0.36, 0.38], [0.45, 0.2, 0.08], crate::util::smooth(n * 1.6 - 0.2)),
                Tex::Wood => {
                    let g = tn(x * 6, y, 8.0, s);
                    let plank = if x % 16 == 0 { 0.5 } else { 1.0 };
                    mul([0.42, 0.29, 0.17], (0.6 + g * 0.5) * plank)
                }
                Tex::Crate => {
                    let edge = x < 5 || x > 58 || y < 5 || y > 58 || (x as i32 - y as i32).abs() < 4;
                    mul([0.45, 0.33, 0.18], (0.6 + tn(x * 4, y, 8.0, s) * 0.4) * if edge { 1.15 } else { 0.85 })
                }
                Tex::Fence => {
                    let d1 = (x + y) % 8 == 0;
                    let d2 = (x + TEX - y) % 8 == 0;
                    let post = y < 3 || y > 60;
                    a = if d1 || d2 || post { 1.0 } else { 0.0 };
                    [0.45, 0.47, 0.48]
                }
                Tex::Camo => {
                    let k = tn(x, y, 5.0, s);
                    let k2 = tn(x, y, 7.0, s + 31);
                    if k > 0.6 { [0.16, 0.2, 0.12] } else if k2 > 0.55 { [0.3, 0.27, 0.17] } else { [0.24, 0.29, 0.17] }
                }
                Tex::Cloth => {
                    let weave = if (x + y) % 2 == 0 { 0.92 } else { 1.0 };
                    mul([0.75, 0.75, 0.75], (0.75 + n * 0.3) * weave)
                }
                Tex::Skin => mul([0.85, 0.66, 0.55], 0.8 + n * 0.25),
                Tex::Flesh => {
                    let v = (tn(x, y, 7.0, s + 2) - 0.5).abs();
                    let vein = if v < 0.03 { 0.55 } else { 1.0 };
                    mul([0.62, 0.2, 0.22], (0.6 + n * 0.6) * vein)
                }
                Tex::Vein => {
                    let v = (tn(x, y, 6.0, s + 2) - 0.5).abs();
                    if v < 0.035 { [0.95, 0.35, 0.5] } else { mul([0.3, 0.1, 0.16], 0.6 + n * 0.5) }
                }
                Tex::Bone => {
                    let pit = if fine > 0.93 { 0.7 } else { 1.0 };
                    mul([0.82, 0.78, 0.66], (0.7 + n * 0.35) * pit)
                }
                Tex::Chitin => {
                    let ridge = ((fy * 8.0 + n * 1.5).fract() - 0.5).abs() * 2.0;
                    mul([0.16, 0.13, 0.2], 0.5 + ridge * 0.9)
                }
                Tex::Growth => {
                    let cell = (tn(x, y, 8.0, s + 4) * 6.0).fract();
                    mix([0.2, 0.08, 0.2], [0.45, 0.16, 0.36], cell * n * 1.6)
                }
                Tex::GrowthGlow => {
                    let v = (tn(x, y, 6.0, s + 2) - 0.5).abs();
                    if v < 0.05 { mix([0.5, 1.0, 0.8], [0.2, 0.8, 0.6], v * 20.0) } else { mul([0.1, 0.07, 0.13], 0.6 + n) }
                }
                Tex::RiftStone => {
                    let facet = (tn(x, y, 4.0, s + 4) * 5.0).floor() / 5.0;
                    mul([0.2, 0.17, 0.3], 0.5 + facet * 0.8 + fine * 0.1)
                }
                Tex::RiftGlow => {
                    let v = ((fx * 4.0 + tn(x, y, 4.0, s) * 2.0).fract() - 0.5).abs();
                    if v < 0.08 { [0.75, 0.5, 1.0] } else { mul([0.1, 0.06, 0.2], 0.6 + n) }
                }
                Tex::Char => mul([0.09, 0.08, 0.08], 0.5 + n * 0.9),
                Tex::Ember => {
                    let v = (tn(x, y, 7.0, s + 2) - 0.5).abs();
                    if v < 0.05 { mix([1.0, 0.75, 0.2], [1.0, 0.3, 0.05], v * 20.0) } else { mul([0.1, 0.06, 0.05], 0.5 + n) }
                }
                Tex::Tile => {
                    let g = x % 16 == 0 || y % 16 == 0;
                    let chk = ((x / 16 + y / 16) % 2) as f32;
                    if g { [0.2, 0.2, 0.2] } else { mul([0.55 + chk * 0.2, 0.58 + chk * 0.2, 0.55 + chk * 0.2], 0.75 + n * 0.25) }
                }
                Tex::Graffiti => {
                    let base = concrete(x, y, s);
                    let p = Vec2::new(fx, fy) * 3.0;
                    let k = fbm2(p + Vec2::splat(3.7), s, 3);
                    let line = ((fx * 2.5 + (fy * 9.0).sin() * 0.25).fract() - 0.5).abs() < 0.09 && (0.25..0.8).contains(&fy);
                    if line && k > 0.35 { mix(base, [0.85, 0.15, 0.3], 0.85) } else if k > 0.7 { mix(base, [0.1, 0.6, 0.5], 0.5) } else { base }
                }
                Tex::Sign => {
                    let border = x < 3 || x > 60 || y < 3 || y > 60;
                    let glyph = hash01((x / 6) as i32, (y / 10) as i32, s) > 0.45 && x % 6 < 4 && y % 10 > 2 && y % 10 < 8 && (8..56).contains(&y);
                    if border { [0.9, 0.9, 0.85] } else if glyph { [1.0, 0.95, 0.7] } else { [0.1, 0.18, 0.3] }
                }
                Tex::Poster => {
                    let band = (y / 12) % 2 == 0;
                    let hazard = y < 14 && ((x + y) / 6) % 2 == 0;
                    if hazard { [0.9, 0.75, 0.1] } else if band && (6..58).contains(&x) && fine > 0.3 { [0.15, 0.13, 0.12] } else { mul([0.8, 0.76, 0.62], 0.8 + n * 0.2) }
                }
                Tex::Tarp => {
                    let fold = ((fx * 5.0 + n).fract() - 0.5).abs();
                    mul([0.25, 0.33, 0.24], 0.6 + fold * 0.9)
                }
                Tex::Sandbag => {
                    let row = y / 12;
                    let xo = (x + if row % 2 == 0 { 0 } else { 12 }) % 24;
                    let edge = y % 12 == 0 || xo == 0;
                    mul([0.5, 0.44, 0.3], if edge { 0.5 } else { 0.75 + n * 0.3 })
                }
                Tex::Glass => {
                    a = 1.0;
                    let r = 0.2 + 0.25 * noise2(Vec2::new(fx * 3.0, fy * 3.0), s);
                    [r * 0.6, r * 0.85, r]
                }
                Tex::Grass => mul([0.2, 0.3, 0.14], 0.5 + n * 0.6 + fine * 0.2),
                Tex::GutWall => {
                    let fold = ((fy * 6.0 + tn(x, y, 4.0, s) * 2.0).fract() - 0.5).abs() * 2.0;
                    mul([0.55, 0.16, 0.2], 0.35 + fold * 0.9)
                }
                Tex::Lattice => {
                    let g = x % 16 < 2 || y % 16 < 2;
                    if g { [0.6, 0.75, 1.0] } else { mul([0.05, 0.06, 0.14], 0.6 + n * 0.8) }
                }
            };
            px(&mut t, x, y, c, a);
        }
    }
    t
}

/// Box-filter downsample one RGBA level.
pub fn downsample(src: &[u8], size: usize) -> Vec<u8> {
    let h = size / 2;
    let mut out = vec![0u8; h * h * 4];
    for y in 0..h {
        for x in 0..h {
            for c in 0..4 {
                let s = |xx: usize, yy: usize| src[(yy * size + xx) * 4 + c] as u32;
                let v = (s(x * 2, y * 2) + s(x * 2 + 1, y * 2) + s(x * 2, y * 2 + 1) + s(x * 2 + 1, y * 2 + 1)) / 4;
                // keep cut-outs crisp
                out[(y * h + x) * 4 + c] = if c == 3 { if v > 96 { 255 } else { 0 } } else { v as u8 };
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
