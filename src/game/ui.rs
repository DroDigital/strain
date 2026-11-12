//! Immediate-mode UI in a fixed 480-unit-tall virtual space: HUD and every menu. Building is
//! read-only (`build_ui`); interaction (`ui_update`) uses the hit rects of the previous frame.
use super::loot::{can_equip, describe, Item, EQ_SLOTS};
use super::{Game, Screen};
use crate::audio::Sfx;
use crate::data::Slot;
use crate::input::{Action, REBINDABLE};
use crate::render::textures::{glyph_cell, GLYPH_H, GLYPH_W, UI_ATLAS_H, UI_ATLAS_W, UI_WHITE};
use crate::render::{UiVertex, WindowMode};
use crate::physics::Statics;
use crate::world::{pit_pos, rift_site, AreaKind, DState, CITY_H, CITY_W, CS, DISTRICTS};
use glam::{Vec2, Vec3, Vec4};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hit {
    Menu(usize),
    Grid(u8, u8, u8),
    Equip(usize),
    Tab(usize),
    Stock(usize),
}

#[derive(Default)]
pub struct UiState {
    pub sel: usize,
    pub cursor: (u8, u8),
    pub focus: u8,
    pub tab: usize,
    pub t: f32,
    pub hits: Vec<(Vec4, Hit)>,
    pub mouse: Vec2,
    pub scroll: usize,
    pub settings_from_title: bool,
    pub pad_col: bool,
    pub confirm_new: bool,
}

pub struct Painter<'a> {
    pub out: &'a mut Vec<UiVertex>,
    pub w: f32,
    pub h: f32,
    pub hits: Vec<(Vec4, Hit)>,
}

pub fn col(c: [f32; 3], a: f32) -> [u8; 4] {
    [(c[0].clamp(0.0, 1.0) * 255.0) as u8, (c[1].clamp(0.0, 1.0) * 255.0) as u8, (c[2].clamp(0.0, 1.0) * 255.0) as u8, (a.clamp(0.0, 1.0) * 255.0) as u8]
}
const WHITE: [f32; 3] = [0.95, 0.93, 0.88];
const DIM: [f32; 3] = [0.55, 0.55, 0.52];
const ACCENT: [f32; 3] = [0.85, 0.25, 0.35];
const PANEL: [f32; 3] = [0.04, 0.035, 0.05];

impl Painter<'_> {
    fn quad(&mut self, x: f32, y: f32, w: f32, h: f32, uv: [f32; 4], c: [u8; 4]) {
        let to = |px: f32, py: f32| [px / self.w * 2.0 - 1.0, 1.0 - py / self.h * 2.0];
        let (a, b, cc, d) = (to(x, y), to(x + w, y), to(x + w, y + h), to(x, y + h));
        let (u0, v0, u1, v1) = (uv[0], uv[1], uv[2], uv[3]);
        for (p, uv) in [(a, [u0, v0]), (b, [u1, v0]), (cc, [u1, v1]), (a, [u0, v0]), (cc, [u1, v1]), (d, [u0, v1])] {
            self.out.push(UiVertex { pos: p, uv, color: c });
        }
    }
    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, c: [f32; 3], a: f32) {
        let u = (UI_WHITE.0 as f32 + 2.0) / UI_ATLAS_W as f32;
        let v = (UI_WHITE.1 as f32 + 2.0) / UI_ATLAS_H as f32;
        self.quad(x, y, w, h, [u, v, u, v], col(c, a));
    }
    pub fn frame(&mut self, x: f32, y: f32, w: f32, h: f32, c: [f32; 3], a: f32) {
        self.rect(x, y, w, 1.0, c, a);
        self.rect(x, y + h - 1.0, w, 1.0, c, a);
        self.rect(x, y, 1.0, h, c, a);
        self.rect(x + w - 1.0, y, 1.0, h, c, a);
    }
    pub fn panel(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.rect(x, y, w, h, PANEL, 0.86);
        self.frame(x, y, w, h, [0.35, 0.3, 0.32], 0.9);
    }
    pub fn text_w(s: &str, scale: f32) -> f32 {
        s.chars().count() as f32 * GLYPH_W as f32 * scale
    }
    pub fn text(&mut self, x: f32, y: f32, s: &str, scale: f32, c: [f32; 3], a: f32) -> f32 {
        let cc = col(c, a);
        let shadow = col([0.0, 0.0, 0.0], a * 0.7);
        let mut cx = x;
        for ch in s.chars() {
            if ch != ' ' {
                let (gx, gy) = glyph_cell(ch);
                let uv = [gx as f32 / UI_ATLAS_W as f32, gy as f32 / UI_ATLAS_H as f32, (gx + GLYPH_W) as f32 / UI_ATLAS_W as f32, (gy + GLYPH_H) as f32 / UI_ATLAS_H as f32];
                let (gw, gh) = (GLYPH_W as f32 * scale, GLYPH_H as f32 * scale);
                self.quad(cx + scale.max(1.0), y + scale.max(1.0), gw, gh, uv, shadow);
                self.quad(cx, y, gw, gh, uv, cc);
            }
            cx += GLYPH_W as f32 * scale;
        }
        cx - x
    }
    pub fn text_c(&mut self, cx: f32, y: f32, s: &str, scale: f32, c: [f32; 3], a: f32) {
        let w = Self::text_w(s, scale);
        self.text(cx - w * 0.5, y, s, scale, c, a);
    }
    /// Word-wrapped text; returns the height used.
    pub fn wrap(&mut self, x: f32, y: f32, w: f32, s: &str, scale: f32, c: [f32; 3], a: f32) -> f32 {
        let max = ((w / (GLYPH_W as f32 * scale)) as usize).max(4);
        let mut line = String::new();
        let mut yy = y;
        for word in s.split_whitespace() {
            if line.len() + word.len() + 1 > max && !line.is_empty() {
                self.text(x, yy, &line, scale, c, a);
                yy += (GLYPH_H as f32 + 2.0) * scale;
                line.clear();
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        if !line.is_empty() {
            self.text(x, yy, &line, scale, c, a);
            yy += (GLYPH_H as f32 + 2.0) * scale;
        }
        yy - y
    }
    pub fn bar(&mut self, x: f32, y: f32, w: f32, h: f32, frac: f32, c: [f32; 3], back: f32) {
        self.rect(x - 1.0, y - 1.0, w + 2.0, h + 2.0, [0.0; 3], 0.8);
        self.rect(x, y, w, h, [0.12, 0.1, 0.1], 0.9);
        if back > frac {
            self.rect(x, y, w * back.clamp(0.0, 1.0), h, [0.9, 0.85, 0.6], 0.5);
        }
        self.rect(x, y, w * frac.clamp(0.0, 1.0), h, c, 1.0);
        self.rect(x, y, w * frac.clamp(0.0, 1.0), 1.0, [1.0; 3], 0.25);
    }
    pub fn hit(&mut self, x: f32, y: f32, w: f32, h: f32, hit: Hit) {
        self.hits.push((Vec4::new(x, y, w, h), hit));
    }
    pub fn button(&mut self, x: f32, y: f32, w: f32, label: &str, selected: bool, i: usize) {
        let h = 14.0;
        if selected {
            self.rect(x, y, w, h, ACCENT, 0.35);
            self.rect(x, y, 2.0, h, ACCENT, 1.0);
        }
        self.text(x + 8.0, y + 3.0, label, 1.0, if selected { WHITE } else { DIM }, 1.0);
        self.hit(x, y, w, h, Hit::Menu(i));
    }
}

pub fn menu_list(p: &mut Painter, x: f32, y: f32, w: f32, items: &[String], sel: usize) {
    for (i, it) in items.iter().enumerate() {
        p.button(x, y + i as f32 * 16.0, w, it, i == sel, i);
    }
}

pub const SETTINGS_ROWS: usize = 13;

impl Game {
    fn project(&self, p: Vec3, uw: f32, uh: f32) -> Option<Vec2> {
        let vp = self.cam.proj(uw / uh) * self.cam.view();
        let c = vp * p.extend(1.0);
        if c.w < 0.1 {
            return None;
        }
        let n = c.truncate() / c.w;
        if n.x.abs() > 1.1 || n.y.abs() > 1.1 {
            return None;
        }
        Some(Vec2::new((n.x * 0.5 + 0.5) * uw, (0.5 - n.y * 0.5) * uh))
    }

    pub fn title_items(&self) -> Vec<String> {
        let mut v = Vec::new();
        if self.has_save {
            v.push("CONTINUE".into());
        }
        v.push("NEW GAME".into());
        v.push("SETTINGS".into());
        v.push("CONTROLS".into());
        v.push("QUIT".into());
        v
    }
    pub fn pause_items(&self) -> Vec<String> {
        let add = if self.guests.len() >= super::coop::MAX_GUESTS { "CO-OP: PARTY FULL".to_string() } else { format!("CO-OP: ADD PLAYER ({}/4)", self.guests.len() + 1) };
        let remove = if self.guests.is_empty() { "CO-OP: (PRESS START ON ANOTHER PAD)".to_string() } else { format!("CO-OP: REMOVE PLAYER {}", self.guests.iter().map(|g| g.slot).max().unwrap_or(1) + 1) };
        vec!["RESUME".into(), "INVENTORY".into(), "SETTINGS".into(), "CONTROLS".into(), add, remove, "SAVE AND QUIT TO TITLE".into(), "QUIT GAME".into()]
    }
    pub fn pit_items(&self) -> Vec<String> {
        let mut v = vec!["REST (HEAL, RE-SEED THE CITY)".to_string()];
        for (i, lit) in self.prof.pits.iter().enumerate() {
            if *lit && i != self.prof.last_pit {
                v.push(format!("TRAVEL: {} PIT {}", DISTRICTS[i / 2].name, i % 2 + 1));
            }
        }
        v.push("LEAVE".into());
        v
    }
    pub fn pit_targets(&self) -> Vec<usize> {
        (0..self.prof.pits.len()).filter(|&i| self.prof.pits[i] && i != self.prof.last_pit).collect()
    }

    pub fn settings_rows(&self) -> Vec<(String, String)> {
        let s = &self.settings;
        let g = &s.gfx;
        let lvl = |v: u8| ["LOW", "MEDIUM", "HIGH"][v.min(2) as usize].to_string();
        vec![
            ("INTERNAL RESOLUTION".into(), if g.scene_height() < g.internal_height { format!("{}P ({}P)", g.internal_height, g.scene_height()) } else { format!("{}P", g.internal_height) }),
            ("SHADOWS".into(), ["OFF", "LOW", "HIGH"][g.shadow_quality.min(2) as usize].into()),
            ("FOG QUALITY".into(), lvl(g.fog_quality)),
            ("PARTICLE DENSITY".into(), lvl(g.particle_density)),
            ("V-SYNC".into(), if g.vsync { "ON" } else { "OFF" }.into()),
            ("WINDOW MODE".into(), format!("{:?}", g.window_mode).to_uppercase()),
            ("RETRO FILTER".into(), crate::render::FILTER_NAMES[g.filter.min(3) as usize].into()),
            ("MASTER VOLUME".into(), format!("{:.0}%", s.master * 100.0)),
            ("MUSIC VOLUME".into(), format!("{:.0}%", s.music * 100.0)),
            ("MOUSE SENSITIVITY".into(), format!("{:.2}", s.bindings.mouse_sens)),
            ("GAMEPAD SENSITIVITY".into(), format!("{:.2}", s.bindings.pad_sens)),
            ("INVERT Y".into(), if s.bindings.invert_y { "ON" } else { "OFF" }.into()),
            ("FIELD OF VIEW".into(), format!("{:.0}", s.fov)),
            ("SCREEN SHAKE".into(), format!("{:.0}%", s.shake * 100.0)),
            ("DAMAGE NUMBERS".into(), if s.damage_numbers { "ON" } else { "OFF" }.into()),
        ]
    }

    pub fn adjust_setting(&mut self, row: usize, dir: i32) {
        let s = &mut self.settings;
        let g = &mut s.gfx;
        let cyc = |v: u8, n: u8| ((v as i32 + dir).rem_euclid(n as i32)) as u8;
        match row {
            0 => {
                let opts = [360u32, 480, 540, 720];
                let i = opts.iter().position(|v| *v == g.internal_height).unwrap_or(1) as i32;
                g.internal_height = opts[(i + dir).clamp(0, 3) as usize];
            }
            1 => g.shadow_quality = cyc(g.shadow_quality, 3),
            2 => g.fog_quality = cyc(g.fog_quality, 3),
            3 => g.particle_density = cyc(g.particle_density, 3),
            4 => g.vsync = !g.vsync,
            5 => {
                let m = [WindowMode::Windowed, WindowMode::Borderless, WindowMode::Fullscreen];
                let i = m.iter().position(|x| *x == g.window_mode).unwrap_or(0) as i32;
                g.window_mode = m[(i + dir).rem_euclid(3) as usize];
            }
            6 => g.filter = cyc(g.filter, 4),
            7 => s.master = (s.master + 0.1 * dir as f32).clamp(0.0, 1.0),
            8 => s.music = (s.music + 0.1 * dir as f32).clamp(0.0, 1.0),
            9 => s.bindings.mouse_sens = (s.bindings.mouse_sens + 0.1 * dir as f32).clamp(0.1, 4.0),
            10 => s.bindings.pad_sens = (s.bindings.pad_sens + 0.1 * dir as f32).clamp(0.1, 4.0),
            11 => s.bindings.invert_y = !s.bindings.invert_y,
            12 => s.fov = (s.fov + 5.0 * dir as f32).clamp(55.0, 100.0),
            13 => s.shake = (s.shake + 0.25 * dir as f32).clamp(0.0, 1.5),
            _ => s.damage_numbers = !s.damage_numbers,
        }
        self.settings_dirty = true;
        self.audio.play(Sfx::UiMove, 0.4, 1.2);
    }

    // =============================================================================================
    pub fn build_ui(&self, out: &mut Vec<UiVertex>, uw: f32, uh: f32) -> Vec<(Vec4, Hit)> {
        let mut p = Painter { out, w: uw, h: uh, hits: Vec::new() };
        match self.screen {
            Screen::Loading => {
                p.rect(0.0, 0.0, uw, uh, [0.0; 3], 1.0);
                p.text_c(uw * 0.5, uh * 0.35, "STRAIN", 6.0, [0.92, 0.88, 0.82], 1.0);
                let w = 300.0;
                p.bar(uw * 0.5 - w * 0.5, uh * 0.6, w, 4.0, self.load_progress, ACCENT, 0.0);
                p.text_c(uw * 0.5, uh * 0.6 + 12.0, &self.load_label, 1.0, DIM, 1.0);
                let tips = &self.t.text.tips;
                p.text_c(uw * 0.5, uh * 0.8, &tips[(self.load_progress * 7.0) as usize % tips.len()], 1.0, WHITE, 0.8);
            }
            Screen::Title => self.ui_title(&mut p),
            _ => {
                if self.cutscene.is_some() {
                    self.ui_cutscene(&mut p);
                } else {
                    self.ui_hud(&mut p);
                }
                match self.screen {
                    Screen::Pause => {
                        p.rect(0.0, 0.0, uw, uh, [0.0; 3], 0.55);
                        p.text_c(uw * 0.5, 120.0, "PAUSED", 3.0, WHITE, 1.0);
                        menu_list(&mut p, uw * 0.5 - 110.0, 180.0, 220.0, &self.pause_items(), self.ui.sel);
                    }
                    Screen::Inventory => self.ui_inventory(&mut p),
                    Screen::Vendor => self.ui_vendor(&mut p),
                    Screen::Stash => self.ui_stash(&mut p),
                    Screen::Pit => {
                        p.rect(0.0, 0.0, uw, uh, [0.05, 0.02, 0.0], 0.5);
                        p.panel(uw * 0.5 - 170.0, 110.0, 340.0, 260.0);
                        p.text_c(uw * 0.5, 122.0, "BURN PIT", 2.0, [1.0, 0.6, 0.25], 1.0);
                        p.text_c(uw * 0.5, 144.0, "THE FIRE KEEPS IT OFF YOU. FOR NOW.", 1.0, DIM, 1.0);
                        menu_list(&mut p, uw * 0.5 - 160.0, 166.0, 320.0, &self.pit_items(), self.ui.sel);
                    }
                    Screen::LevelUp => self.ui_levelup(&mut p),
                    Screen::Settings => self.ui_settings(&mut p),
                    Screen::Controls => self.ui_controls(&mut p),
                    Screen::Note => {
                        p.rect(0.0, 0.0, uw, uh, [0.0; 3], 0.5);
                        let (x, w) = (uw * 0.5 - 160.0, 320.0);
                        p.rect(x, 90.0, w, 280.0, [0.82, 0.78, 0.66], 0.97);
                        p.frame(x, 90.0, w, 280.0, [0.3, 0.25, 0.2], 1.0);
                        if let Some(n) = &self.note {
                            p.wrap(x + 16.0, 110.0, w - 32.0, &n.to_uppercase(), 1.0, [0.15, 0.12, 0.1], 1.0);
                        }
                        p.text_c(uw * 0.5, 350.0, "[BACK] CLOSE", 1.0, [0.3, 0.25, 0.2], 1.0);
                    }
                    Screen::Dead => {
                        let k = (self.dead_t / 1.5).min(1.0);
                        p.rect(0.0, 0.0, uw, uh, [0.1, 0.0, 0.0], 0.6 * k);
                        p.text_c(uw * 0.5, uh * 0.38, "CONSUMED", 4.0, [0.8, 0.1, 0.12], k);
                        p.text_c(uw * 0.5, uh * 0.38 + 46.0, "YOUR HUSK HOLDS YOUR MUTATIONS AND YOUR LEVEL.", 1.0, WHITE, k);
                        p.text_c(uw * 0.5, uh * 0.38 + 60.0, "REACH IT BEFORE YOU DIE AGAIN, OR IT RISES WITH THEM.", 1.0, DIM, k);
                        if self.dead_t > 2.5 {
                            p.text_c(uw * 0.5, uh * 0.38 + 90.0, "PRESS ACCEPT TO WAKE AT THE BURN PIT", 1.0, [1.0, 0.6, 0.3], 0.6 + 0.4 * (self.ui.t * 3.0).sin().abs());
                        }
                    }
                    Screen::Credits => {
                        p.rect(0.0, 0.0, uw, uh, [0.0; 3], 0.92);
                        let lines = ["STRAIN", "", "A CITY, A VIRUS, A COMPATIBLE MAN.", "", "EVERY MESH, TEXTURE, SOUND AND NOTE", "WAS GROWN FROM CODE AT LOAD TIME.", "", "THANK YOU FOR PLAYING.", "", "THE CITY REMAINS OPEN. YOU MAY CONTINUE."];
                        for (i, l) in lines.iter().enumerate() {
                            p.text_c(uw * 0.5, 120.0 + i as f32 * 18.0, l, if i == 0 { 3.0 } else { 1.0 }, if i == 0 { ACCENT } else { WHITE }, (self.ui.t - i as f32 * 0.4).clamp(0.0, 1.0));
                        }
                        if self.ui.t > 4.0 {
                            p.text_c(uw * 0.5, 400.0, "PRESS ACCEPT", 1.0, DIM, 1.0);
                        }
                    }
                    _ => {}
                }
            }
        }
        if self.fps_show {
            p.text(uw - 90.0, 4.0, &format!("{:.0} FPS", self.fps), 1.0, [0.6, 1.0, 0.6], 0.8);
        }
        p.hits
    }

    fn ui_title(&self, p: &mut Painter) {
        let (uw, uh) = (p.w, p.h);
        p.rect(0.0, 0.0, uw, uh, [0.0; 3], 0.35);
        let t = self.ui.t;
        let jit = (t * 13.0).sin() * (t * 7.0).cos() * 1.5;
        p.text_c(uw * 0.5 + jit, 90.0, "STRAIN", 7.0, [0.5, 0.05, 0.1], 0.6);
        p.text_c(uw * 0.5, 88.0, "STRAIN", 7.0, [0.92, 0.88, 0.82], 1.0);
        p.text_c(uw * 0.5, 160.0, "THE VIRUS DID NOT TAKE YOU. IT MOVED IN.", 1.0, DIM, 1.0);
        if self.ui.confirm_new {
            p.panel(uw * 0.5 - 160.0, 210.0, 320.0, 90.0);
            p.text_c(uw * 0.5, 222.0, "START OVER? YOUR SAVE WILL BE REPLACED.", 1.0, WHITE, 1.0);
            menu_list(p, uw * 0.5 - 60.0, 244.0, 120.0, &["YES".into(), "NO".into()], self.ui.sel);
        } else {
            menu_list(p, uw * 0.5 - 70.0, 210.0, 140.0, &self.title_items(), self.ui.sel);
        }
        p.text(8.0, uh - 14.0, &format!("v{}  {}", env!("CARGO_PKG_VERSION"), self.adapter.to_uppercase()), 1.0, [0.4; 3], 0.8);
    }

    fn ui_cutscene(&self, p: &mut Painter) {
        let (uw, uh) = (p.w, p.h);
        p.rect(0.0, 0.0, uw, 48.0, [0.0; 3], 1.0);
        p.rect(0.0, uh - 56.0, uw, 56.0, [0.0; 3], 1.0);
        if let Some(c) = &self.caption {
            p.text_c(uw * 0.5, uh - 36.0, &c.to_uppercase(), 1.0, WHITE, 1.0);
        }
        if let Some((n, t)) = &self.cut_title {
            p.text_c(uw * 0.5, uh * 0.62, n, 3.0, [0.9, 0.85, 0.8], 1.0);
            p.text_c(uw * 0.5, uh * 0.62 + 30.0, &t.to_uppercase(), 1.0, ACCENT, 1.0);
        }
        p.text(uw - 110.0, 20.0, "[ACCEPT] SKIP", 1.0, [0.4; 3], 1.0);
    }

    /// Co-op guests: a panel each in the bottom-right corner and a tag over their heads.
    fn ui_guests(&self, p: &mut Painter) {
        let (uw, uh) = (p.w, p.h);
        for (k, g) in self.guests.iter().enumerate() {
            let col = super::coop::COLOURS[g.slot as usize % 4];
            let x = uw - 150.0;
            let y = uh - 46.0 - k as f32 * 30.0;
            p.rect(x - 4.0, y - 4.0, 142.0, 26.0, [0.0; 3], 0.45);
            p.text(x, y, &format!("P{}", g.slot + 1), 1.0, col, 1.0);
            let frac = self.w.actor.get(g.e).map(|a| a.frac()).unwrap_or(0.0);
            if g.down() {
                p.text(x + 18.0, y, "DOWN", 1.0, [1.0, 0.3, 0.3], 0.6 + 0.4 * (self.ui.t * 6.0).sin().abs());
                p.bar(x + 18.0, y + 10.0, 110.0, 4.0, (g.revive / 2.2).min(1.0), [0.5, 0.95, 0.6], 0.0);
            } else {
                p.bar(x + 18.0, y + 1.0, 110.0, 5.0, frac, [0.75, 0.12, 0.12], 0.0);
                p.bar(x + 18.0, y + 9.0, 90.0, 3.0, g.stamina / 100.0, [0.55, 0.75, 0.35], 0.0);
                for h in 0..g.heals {
                    p.rect(x + 112.0 + h as f32 * 5.0, y + 9.0, 3.0, 3.0, [0.95, 0.55, 0.2], 0.9);
                }
            }
            if let Some(b) = self.w.body.get(g.e) {
                if let Some(s) = self.project(b.pos + Vec3::Y * (b.height + 0.35), uw, uh) {
                    p.text(s.x - 6.0, s.y, &format!("P{}", g.slot + 1), 1.0, col, 0.85);
                }
            }
        }
    }

    fn ui_hud(&self, p: &mut Painter) {
        let (uw, uh) = (p.w, p.h);
        self.ui_guests(p);
        let Some(a) = self.w.actor.get(self.player) else { return };
        let st = &self.pl.stats;
        // bars
        let x = 16.0;
        let hw = 130.0 + st.max_hp.min(500.0) * 0.25;
        p.bar(x, 16.0, hw, 7.0, a.frac(), [0.75, 0.12, 0.12], self.hp_trail);
        let sw = 100.0 + st.max_stam.min(300.0) * 0.4;
        p.bar(x, 27.0, sw, 4.0, (self.pl.stamina / st.max_stam).max(0.0), [0.55, 0.75, 0.35], 0.0);
        let sf = self.pl.strain_frac();
        let scol = if sf > 0.7 { [0.95, 0.25, 0.9] } else if sf > 0.35 { [0.7, 0.35, 0.95] } else { [0.45, 0.4, 0.8] };
        p.bar(x, 35.0, sw, 4.0, sf, scol, 0.0);
        p.rect(x + sw * 0.35, 34.0, 1.0, 6.0, WHITE, 0.6);
        p.rect(x + sw * 0.7, 34.0, 1.0, 6.0, WHITE, 0.6);
        p.text(x + sw + 6.0, 34.0, if sf > 0.7 { "STRAIN!" } else { "STRAIN" }, 1.0, scol, if sf > 0.7 { 0.6 + 0.4 * (self.ui.t * 8.0).sin().abs() } else { 0.8 });
        // heal charges
        for i in 0..st.heal_charges {
            let full = i < self.pl.heal_charges;
            p.rect(x + i as f32 * 9.0, 44.0, 6.0, 9.0, if full { [1.0, 0.55, 0.2] } else { [0.25, 0.2, 0.2] }, 1.0);
        }
        // level and biomass
        let lw = 120.0;
        p.text(x, 58.0, &format!("LV {}", self.pl.life.level), 1.0, WHITE, 1.0);
        p.bar(x + 36.0, 60.0, lw, 3.0, self.pl.life.biomass / self.pl.life.next_level_cost(), [0.9, 0.35, 0.6], 0.0);
        p.text(x + 40.0 + lw, 58.0, &format!("{} SCRIP", self.pl.gear.scrip), 1.0, [0.9, 0.85, 0.5], 0.8);
        if let Some((af, _)) = self.pl.affliction {
            p.text(x, 70.0, af.name(), 1.0, [1.0, 0.4, 0.8], 0.7 + 0.3 * (self.ui.t * 5.0).sin().abs());
        }
        // hands and spell (bottom left)
        let by = uh - 54.0;
        p.panel(x - 4.0, by - 4.0, 210.0, 44.0);
        let hand = |i: usize| self.pl.gear.eq[i].as_ref().map(|it| it.name(&self.t).to_uppercase()).unwrap_or_else(|| "BARE HAND".into());
        let rc = self.pl.gear.eq[0].as_ref().map(|i| i.rarity.color()).unwrap_or(DIM);
        let lc = self.pl.gear.eq[1].as_ref().map(|i| i.rarity.color()).unwrap_or(DIM);
        p.text(x, by, "R", 1.0, DIM, 1.0);
        p.text(x + 10.0, by, &hand(0), 1.0, rc, 1.0);
        p.text(x, by + 11.0, "L", 1.0, DIM, 1.0);
        p.text(x + 10.0, by + 11.0, &hand(1), 1.0, lc, 1.0);
        let spells = self.pl.spells(&self.t);
        if spells.is_empty() {
            p.text(x, by + 24.0, "NO CATALYST", 1.0, [0.4; 3], 1.0);
        } else {
            let s = &self.t.spells[spells[self.pl.spell_idx % spells.len()]];
            let sc = match s.school {
                crate::data::School::Flesh => [1.0, 0.45, 0.5],
                crate::data::School::Rift => [0.6, 0.55, 1.0],
                crate::data::School::Cinder => [1.0, 0.6, 0.25],
            };
            p.text(x, by + 24.0, &format!("{} [{}]  {}/{}", s.name.to_uppercase(), self.input_labels.cast, self.pl.spell_idx % spells.len() + 1, spells.len()), 1.0, sc, 1.0);
        }
        // compass
        let cw = 220.0;
        let cx = uw * 0.5;
        p.rect(cx - cw * 0.5, 8.0, cw, 12.0, [0.0; 3], 0.4);
        let yaw = self.cam.yaw;
        let mark = |p: &mut Painter, ang: f32, label: &str, c: [f32; 3]| {
            let d = crate::util::wrap_angle(ang - yaw);
            if d.abs() < 1.4 {
                let px = cx - d / 1.4 * cw * 0.5;
                p.text_c(px, 10.0, label, 1.0, c, 1.0 - d.abs() / 1.6);
            }
        };
        mark(p, std::f32::consts::PI, "N", WHITE);
        mark(p, 0.0, "S", DIM);
        mark(p, std::f32::consts::FRAC_PI_2, "E", DIM);
        mark(p, -std::f32::consts::FRAC_PI_2, "W", DIM);
        let pp = self.ppos();
        let bearing = |q: Vec3| {
            let d = q - pp;
            d.x.atan2(d.z)
        };
        if self.area.kind == AreaKind::City {
            if let Some(h) = &self.prof.husk {
                mark(p, bearing(Vec3::from(h.pos)), "H", [1.0, 0.3, 0.45]);
            }
            for (i, lit) in self.prof.pits.iter().enumerate() {
                let q = pit_pos(i);
                if (q - pp).length() < 200.0 {
                    mark(p, bearing(q), "^", if *lit { [1.0, 0.6, 0.2] } else { [0.4; 3] });
                }
            }
            for i in 0..5 {
                let q = rift_site(i);
                if (q - pp).length() < 260.0 && !self.prof.bosses[i] {
                    mark(p, bearing(q), "*", [0.7, 0.5, 1.0]);
                }
            }
        }
        // lock-on marker + floating texts
        if let Some(l) = self.pl.lock {
            if let Some(b) = self.w.body.get(l) {
                if let Some(s) = self.project(b.center(), uw, uh) {
                    let k = 5.0 + (self.ui.t * 6.0).sin();
                    p.rect(s.x - k, s.y - 1.0, k * 2.0, 2.0, ACCENT, 0.9);
                    p.rect(s.x - 1.0, s.y - k, 2.0, k * 2.0, ACCENT, 0.9);
                    p.frame(s.x - 4.0, s.y - 4.0, 8.0, 8.0, WHITE, 0.9);
                }
            }
        }
        for (wp, txt, c, life) in &self.floats {
            if let Some(s) = self.project(*wp + Vec3::Y * (1.0 - life) * 0.8, uw, uh) {
                p.text_c(s.x, s.y, txt, 1.0, *c, life.min(1.0) * 1.5);
            }
        }
        // graffiti read from close by
        let lines = &self.t.text.graffiti;
        for c in self.area.chunks.values() {
            for a in &c.anims {
                if a.kind == crate::world::AnimKind::Sign {
                    let d = (a.pos - self.cam.eye).length();
                    if d < 11.0 && self.area.line_of_sight(self.cam.eye, a.pos) {
                        if let Some(s) = self.project(a.pos, uw, uh) {
                            let txt = &lines[a.yaw as usize % lines.len()];
                            let k = (1.0 - d / 11.0).min(0.6) * 1.6;
                            p.text_c(s.x, s.y, txt, (2.0 - d * 0.1).max(1.0), [0.85, 0.15, 0.25], k);
                        }
                    }
                }
            }
        }
        // speech bubbles
        for (e, txt, life) in &self.speech {
            if let Some(b) = self.w.body.get(*e) {
                if (b.pos - self.cam.eye).length() < 22.0 {
                    if let Some(s) = self.project(b.pos + Vec3::Y * (b.height + 0.5), uw, uh) {
                        let w = Painter::text_w(txt, 1.0);
                        p.rect(s.x - w * 0.5 - 3.0, s.y - 2.0, w + 6.0, 12.0, [0.0; 3], 0.55 * life.min(1.0));
                        p.text_c(s.x, s.y, txt, 1.0, WHITE, life.min(1.0));
                    }
                }
            }
        }
        // enemy panel: locked or last hit
        let focus = self.pl.lock.or(self.pl.last_hit_target.map(|x| x.0)).filter(|e| Some(*e) != self.boss);
        if let Some(e) = focus {
            if let (Some(f), Some(ea)) = (self.w.foe.get(e), self.w.actor.get(e)) {
                if ea.alive && !f.mimic {
                    let px = uw - 196.0;
                    p.panel(px - 4.0, 30.0, 190.0, 30.0 + f.affixes.len() as f32 * 10.0);
                    let stage = if f.stage > 0 { super::spawn::STAGE_NAMES[f.stage.min(4) as usize] } else { "HUMAN" };
                    let rc = match f.stage {
                        2 => [0.4, 0.6, 1.0],
                        3 => [1.0, 0.85, 0.3],
                        4 => [1.0, 0.45, 0.15],
                        _ => WHITE,
                    };
                    p.text(px, 34.0, &format!("{} LV{}", f.name.to_uppercase(), ea.level), 1.0, rc, 1.0);
                    p.text(px, 44.0, &format!("STAGE {} {}", ["0", "I", "II", "III", "IV"][f.stage.min(4) as usize], stage), 1.0, DIM, 1.0);
                    p.bar(px, 54.0, 178.0, 3.0, ea.frac(), [0.8, 0.2, 0.2], 0.0);
                    for (i, af) in f.affixes.iter().enumerate() {
                        let d = self.t.affix(*af);
                        p.text(px, 60.0 + i as f32 * 10.0, &format!("{} - {}", d.name.to_uppercase(), d.desc.to_uppercase()), 1.0, [0.75, 0.6, 0.95], 1.0);
                    }
                }
            }
        }
        // boss bar
        if let Some(b) = self.boss {
            if let (Some(f), Some(ba)) = (self.w.foe.get(b), self.w.actor.get(b)) {
                if self.boss_awake && ba.alive {
                    let w = uw * 0.6;
                    let bx = uw * 0.2;
                    p.text_c(uw * 0.5, uh - 60.0, &f.name, 1.5, WHITE, 1.0);
                    p.bar(bx, uh - 44.0, w, 7.0, ba.frac(), [0.7, 0.1, 0.15], self.boss_trail);
                    if let Some(bi) = f.boss {
                        let n = self.t.bosses[bi].phases.len();
                        for (i, ph) in self.t.bosses[bi].phases.iter().enumerate().skip(1) {
                            let _ = n;
                            p.rect(bx + w * ph.at, uh - 46.0, 1.0, 11.0, WHITE, 0.7);
                            let _ = i;
                        }
                    }
                }
            }
        }
        // messages
        for (i, (m, c, life)) in self.messages.iter().rev().take(5).enumerate() {
            p.text(16.0, uh * 0.5 + 40.0 - i as f32 * 11.0, m, 1.0, *c, life.min(1.0));
        }
        if let Some((s, life)) = &self.subtitle_line {
            let w = Painter::text_w(s, 1.0);
            p.rect(uw * 0.5 - w * 0.5 - 4.0, uh - 82.0, w + 8.0, 12.0, [0.0; 3], 0.5 * life.min(1.0));
            p.text_c(uw * 0.5, uh - 80.0, s, 1.0, WHITE, life.min(1.0));
        }
        if let Some((s, life)) = &self.big_banner {
            let a = life.min(1.0);
            p.rect(0.0, uh * 0.3 - 6.0, uw, 32.0, [0.0; 3], 0.5 * a);
            p.text_c(uw * 0.5, uh * 0.3, s, 2.0, [0.95, 0.9, 0.8], a);
        }
        // interaction prompt
        if self.screen == Screen::None {
            if let Some((_, label)) = self.interact_target() {
                p.text_c(uw * 0.5, uh * 0.62, &format!("[{}] {}", self.input_labels.interact, label), 1.0, WHITE, 1.0);
            }
            if self.pl.life.pending_levels > 0 && !self.mutation_choices.is_empty() {
                p.text_c(uw * 0.5, uh * 0.66, "MUTATION READY", 1.0, ACCENT, 1.0);
            }
            if self.tutorial_t < 60.0 && self.prof.deaths == 0 && self.pl.life.level < 3 {
                let tips = [
                    format!("[{}] LIGHT  [{}] HEAVY (HOLD TO CHARGE)  [{}] BLOCK - TAP JUST BEFORE A HIT TO PARRY", self.input_labels.light, self.input_labels.heavy, self.input_labels.block),
                    format!("[{}] DODGE (HOLD TO SPRINT)  [{}] HEAL  [{}] CAST  [{}] LOCK ON  [{}] INVENTORY", self.input_labels.dodge, self.input_labels.heal, self.input_labels.cast, self.input_labels.lock, self.input_labels.menu),
                ];
                let k = ((self.tutorial_t / 12.0) as usize) % tips.len();
                p.text_c(uw * 0.5, uh - 24.0, &tips[k], 1.0, DIM, 0.9);
            }
        }
    }

    fn grid_ui(&self, p: &mut Painter, gx: f32, gy: f32, grid: &super::loot::Grid, which: u8, focus: bool) {
        let cs = 22.0;
        for y in 0..grid.h {
            for x in 0..grid.w {
                let fx = gx + x as f32 * cs;
                let fy = gy + y as f32 * cs;
                p.rect(fx, fy, cs - 1.0, cs - 1.0, [0.1, 0.09, 0.1], 0.9);
                p.hit(fx, fy, cs, cs, Hit::Grid(which, x, y));
            }
        }
        for (x, y, it) in &grid.items {
            let (w, h) = it.size(&self.t);
            let (fx, fy) = (gx + *x as f32 * cs, gy + *y as f32 * cs);
            let c = it.rarity.color();
            p.rect(fx + 1.0, fy + 1.0, w as f32 * cs - 3.0, h as f32 * cs - 3.0, [c[0] * 0.3, c[1] * 0.3, c[2] * 0.3], 1.0);
            p.frame(fx + 1.0, fy + 1.0, w as f32 * cs - 3.0, h as f32 * cs - 3.0, c, 1.0);
            let abbr: String = it.base(&self.t).name.split_whitespace().map(|s| s.chars().next().unwrap_or(' ')).collect();
            p.text(fx + 4.0, fy + 4.0, &abbr, 1.0, c, 1.0);
            if !it.spells.is_empty() {
                p.text(fx + 4.0, fy + h as f32 * cs - 12.0, "*", 1.0, [0.9, 0.6, 1.0], 1.0);
            }
        }
        if focus {
            let (cx, cy) = self.ui.cursor;
            p.frame(gx + cx as f32 * cs - 1.0, gy + cy as f32 * cs - 1.0, cs + 1.0, cs + 1.0, WHITE, 0.6 + 0.4 * (self.ui.t * 6.0).sin().abs());
        }
    }

    fn tooltip(&self, p: &mut Painter, x: f32, y: f32, it: &Item, compare: Option<&Item>, header: &str) -> f32 {
        let lines = describe(&self.t, it);
        let h = 18.0 + lines.len() as f32 * 11.0 + 30.0;
        p.panel(x, y, 250.0, h);
        p.text(x + 6.0, y + 4.0, header, 1.0, DIM, 1.0);
        let mut yy = y + 16.0;
        for (l, c) in &lines {
            yy += p.wrap(x + 6.0, yy, 238.0, l, 1.0, *c, 1.0);
        }
        if let Some(cmp) = compare {
            let b = it.base(&self.t);
            let d = it.power - cmp.power;
            if cmp.base(&self.t).slot == b.slot {
                let label = if b.dmg > 0.0 { "DAMAGE" } else { "ARMOUR" };
                let c = if d >= 0.0 { [0.4, 1.0, 0.4] } else { [1.0, 0.35, 0.35] };
                p.text(x + 6.0, yy + 2.0, &format!("VS EQUIPPED: {}{:.0} {}", if d >= 0.0 { "+" } else { "" }, d, label), 1.0, c, 1.0);
                yy += 12.0;
                let ea = cmp.affixes.len() as i32 - it.affixes.len() as i32;
                if ea != 0 {
                    p.text(x + 6.0, yy + 2.0, &format!("{} AFFIXES THAN EQUIPPED", if ea < 0 { "MORE" } else { "FEWER" }), 1.0, DIM, 1.0);
                    yy += 12.0;
                }
            }
        }
        yy - y
    }

    fn tabs(&self, p: &mut Painter, x: f32, y: f32) {
        let names = ["GEAR", "MUTATIONS", "MAP", "CHARACTER"];
        let mut cx = x;
        for (i, n) in names.iter().enumerate() {
            let w = Painter::text_w(n, 1.0) + 16.0;
            if i == self.ui.tab {
                p.rect(cx, y, w, 14.0, ACCENT, 0.5);
            }
            p.text(cx + 8.0, y + 3.0, n, 1.0, if i == self.ui.tab { WHITE } else { DIM }, 1.0);
            p.hit(cx, y, w, 14.0, Hit::Tab(i));
            cx += w + 4.0;
        }
        p.text(cx + 8.0, y + 3.0, "[ [ ] / LB RB ] TABS", 1.0, [0.4; 3], 1.0);
    }

    fn ui_inventory(&self, p: &mut Painter) {
        let (uw, uh) = (p.w, p.h);
        p.rect(0.0, 0.0, uw, uh, [0.0; 3], 0.7);
        let x0 = (uw - 820.0).max(0.0) * 0.5 + 10.0;
        self.tabs(p, x0, 20.0);
        match self.ui.tab {
            0 => {
                // equipment
                p.panel(x0, 44.0, 230.0, 190.0);
                p.text(x0 + 6.0, 48.0, "EQUIPPED", 1.0, DIM, 1.0);
                for (i, (name, _)) in EQ_SLOTS.iter().enumerate() {
                    let y = 62.0 + i as f32 * 24.0;
                    let sel = self.ui.focus == 1 && self.ui.sel == i;
                    if sel {
                        p.rect(x0 + 2.0, y - 2.0, 226.0, 22.0, ACCENT, 0.3);
                    }
                    p.text(x0 + 6.0, y, name, 1.0, DIM, 1.0);
                    let (t, c) = match &self.pl.gear.eq[i] {
                        Some(it) => (it.name(&self.t).to_uppercase(), it.rarity.color()),
                        None => ("-".into(), [0.35; 3]),
                    };
                    p.text(x0 + 6.0, y + 10.0, &t, 1.0, c, 1.0);
                    p.hit(x0, y - 2.0, 230.0, 22.0, Hit::Equip(i));
                }
                let gx = x0 + 240.0;
                p.text(gx, 48.0, &format!("PACK - {} SCRIP", self.pl.gear.scrip), 1.0, DIM, 1.0);
                self.grid_ui(p, gx, 60.0, &self.pl.gear.inv, 0, self.ui.focus == 0);
                p.text(x0, 248.0, "[ACCEPT] EQUIP / UNEQUIP   [R / Y] EQUIP LEFT HAND OR 2ND TRINKET   [G / X] DROP", 1.0, DIM, 1.0);
                p.text(x0, 260.0, "SETUPS: TWO ONE-HANDERS, ONE-HANDER + SHIELD OR CATALYST, TWO CATALYSTS, ONE TWO-HANDER", 1.0, [0.4; 3], 1.0);
                // tooltip
                let tx = gx + 10.0 * 22.0 + 10.0;
                let tx = tx.min(uw - 255.0);
                let hovered: Option<&Item> = if self.ui.focus == 0 {
                    self.pl.gear.inv.at(&self.t, self.ui.cursor.0, self.ui.cursor.1).map(|i| &self.pl.gear.inv.items[i].2)
                } else {
                    self.pl.gear.eq.get(self.ui.sel).and_then(|x| x.as_ref())
                };
                if let Some(it) = hovered {
                    let slot = it.base(&self.t).slot;
                    let cmp = if self.ui.focus == 0 {
                        match slot {
                            Slot::Hand => self.pl.gear.eq[0].as_ref(),
                            Slot::Head => self.pl.gear.eq[2].as_ref(),
                            Slot::Body => self.pl.gear.eq[3].as_ref(),
                            Slot::Legs => self.pl.gear.eq[4].as_ref(),
                            Slot::Trinket => self.pl.gear.eq[5].as_ref(),
                        }
                    } else {
                        None
                    };
                    let h = self.tooltip(p, tx, 44.0, it, cmp, "SELECTED");
                    if let Some(c) = cmp {
                        if uh > 44.0 + h + 120.0 {
                            self.tooltip(p, tx, 50.0 + h, c, None, "EQUIPPED");
                        }
                    }
                }
            }
            1 => {
                p.panel(x0, 44.0, 800.0, uh - 70.0);
                p.text(x0 + 6.0, 48.0, &format!("MUTATIONS - LEVEL {} - THESE BELONG TO THIS LIFE", self.pl.life.level), 1.0, DIM, 1.0);
                let owned = &self.pl.life.mutations;
                if owned.is_empty() {
                    p.text(x0 + 6.0, 70.0, "NONE YET. KILL. ABSORB. CHANGE.", 1.0, [0.4; 3], 1.0);
                }
                let tags: Vec<&str> = owned.iter().filter_map(|id| self.t.mutations.iter().find(|m| &m.id == id)).filter_map(|m| m.tag.as_deref()).collect();
                let mut y = 64.0;
                for id in owned.iter().skip(self.ui.scroll) {
                    if y > uh - 40.0 {
                        break;
                    }
                    if let Some(m) = self.t.mutations.iter().find(|m| &m.id == id) {
                        let tc = [[0.8, 0.8, 0.8], [0.5, 0.7, 1.0], [1.0, 0.8, 0.3], [1.0, 0.4, 0.2]][(m.tier as usize).clamp(1, 4) - 1];
                        p.text(x0 + 6.0, y, &format!("{} (TIER {})", m.name.to_uppercase(), m.tier), 1.0, tc, 1.0);
                        let h = p.wrap(x0 + 200.0, y, 590.0, &m.desc.to_uppercase(), 1.0, WHITE, 1.0);
                        let mut extra = 0.0;
                        if let Some((tag, _)) = &m.syn {
                            let n = tags.iter().filter(|t| **t == tag.as_str()).count();
                            let own = m.tag.as_deref() == Some(tag.as_str());
                            let active = n > own as usize;
                            p.text(x0 + 200.0, y + h, &format!("SYNERGY ({}) {}", tag.to_uppercase(), if active { "ACTIVE" } else { "INACTIVE" }), 1.0, if active { [0.4, 1.0, 0.6] } else { [0.4; 3] }, 1.0);
                            extra = 11.0;
                        }
                        y += h.max(11.0) + extra + 4.0;
                    }
                }
            }
            2 => self.ui_map(p, x0),
            _ => {
                let st = &self.pl.stats;
                p.panel(x0, 44.0, 420.0, 330.0);
                let rows: Vec<(String, String)> = vec![
                    ("LEVEL".into(), format!("{}", self.pl.life.level)),
                    ("BIOMASS".into(), format!("{:.0} / {:.0}", self.pl.life.biomass, self.pl.life.next_level_cost())),
                    ("HEALTH".into(), format!("{:.0}", st.max_hp)),
                    ("STAMINA".into(), format!("{:.0}", st.max_stam)),
                    ("ARMOUR".into(), format!("{:.0}", st.armor)),
                    ("POISE".into(), format!("{:.0}", st.poise)),
                    ("DAMAGE".into(), format!("{:.0}%", st.dmg * 100.0)),
                    ("ATTACK SPEED".into(), format!("{:.0}%", st.atk_speed * 100.0)),
                    ("CRITICAL".into(), format!("{:.0}%", st.crit * 100.0)),
                    ("SPELL POWER".into(), format!("{:.0}%", st.spell_power * 100.0)),
                    ("STRAIN CAPACITY".into(), format!("{:.0}", st.strain_cap)),
                    ("STRAIN COST".into(), format!("{:.0}%", st.strain_cost * 100.0)),
                    ("STRAIN VENT PER HIT".into(), format!("{:.0}%", st.strain_drain * 100.0)),
                    ("HEAL CHARGES".into(), format!("{}", st.heal_charges)),
                    ("MOVE SPEED".into(), format!("{:.0}%", st.move_speed * 100.0)),
                    ("LOOT FIND".into(), format!("+{:.0}%", st.luck * 100.0)),
                    ("BOSSES DESTROYED".into(), format!("{}/5", self.prof.bosses.iter().filter(|b| **b).count())),
                    ("DEATHS".into(), format!("{}", self.prof.deaths)),
                ];
                for (i, (k, v)) in rows.iter().enumerate() {
                    p.text(x0 + 8.0, 52.0 + i as f32 * 17.0, k, 1.0, DIM, 1.0);
                    p.text(x0 + 220.0, 52.0 + i as f32 * 17.0, v, 1.0, WHITE, 1.0);
                }
            }
        }
    }

    fn ui_map(&self, p: &mut Painter, x0: f32) {
        let cs = 22.0;
        let (mx, my) = (x0 + 10.0, 50.0);
        p.panel(x0, 44.0, CITY_W as f32 * cs + 200.0, CITY_H as f32 * cs + 16.0);
        for cz in 0..CITY_H {
            for cx in 0..CITY_W {
                let d = crate::world::district_of(cx, cz).unwrap();
                let st = DISTRICTS[d].state;
                let base = match st {
                    DState::Held => [0.25, 0.32, 0.4],
                    DState::Contested => [0.4, 0.3, 0.18],
                    DState::Lost => [0.38, 0.12, 0.3],
                };
                let seen = self.prof.districts_seen[d];
                let k = if seen { 1.0 } else { 0.35 };
                p.rect(mx + cx as f32 * cs, my + cz as f32 * cs, cs - 1.0, cs - 1.0, [base[0] * k, base[1] * k, base[2] * k], 1.0);
            }
        }
        for (d, dd) in DISTRICTS.iter().enumerate() {
            let cx = mx + (dd.grid.0 * 6) as f32 * cs + 3.0 * cs;
            let cy = my + (dd.grid.1 * 6) as f32 * cs + 0.4 * cs;
            p.text_c(cx, cy, dd.name, 1.0, if self.prof.district_unlocked(d) { WHITE } else { [0.5; 3] }, 1.0);
            p.text_c(cx, cy + 10.0, &format!("LV {}-{}", dd.levels.0, dd.levels.1), 1.0, DIM, 1.0);
            if self.prof.districts_seen[d] {
                let lm = crate::world::chunk_center(d, dd.landmark_at);
                let s = Vec2::new(mx + lm.x / CS * cs, my + lm.z / CS * cs);
                p.text_c(s.x, s.y + 6.0, dd.landmark, 1.0, [0.75, 0.75, 0.6], 0.8);
            }
        }
        let to_map = |q: Vec3| Vec2::new(mx + q.x / CS * cs, my + q.z / CS * cs);
        for (i, lit) in self.prof.pits.iter().enumerate() {
            let s = to_map(pit_pos(i));
            p.rect(s.x - 3.0, s.y - 3.0, 6.0, 6.0, if *lit { [1.0, 0.55, 0.15] } else { [0.3; 3] }, 1.0);
        }
        for i in 0..5 {
            let s = to_map(rift_site(i));
            p.text_c(s.x, s.y - 4.0, "*", 1.0, if self.prof.bosses[i] { [0.35; 3] } else { [0.8, 0.5, 1.0] }, 1.0);
        }
        if let Some(h) = &self.prof.husk {
            let s = to_map(Vec3::from(h.pos));
            p.text_c(s.x, s.y - 4.0, "H", 1.0, [1.0, 0.3, 0.45], 1.0);
        }
        let pp = match self.area.kind {
            AreaKind::City => self.ppos(),
            AreaKind::Rift(i) => rift_site(i),
        };
        let s = to_map(pp);
        let blink = 0.5 + 0.5 * (self.ui.t * 6.0).sin().abs();
        p.rect(s.x - 3.0, s.y - 3.0, 6.0, 6.0, WHITE, blink);
        let lx = mx + CITY_W as f32 * cs + 12.0;
        let legend = [("^ BURN PIT (LIT)", [1.0, 0.55, 0.15]), ("* RIFT", [0.8, 0.5, 1.0]), ("H YOUR HUSK", [1.0, 0.3, 0.45]), ("BLUE: HELD", [0.4, 0.55, 0.7]), ("AMBER: CONTESTED", [0.7, 0.5, 0.3]), ("PURPLE: LOST", [0.7, 0.3, 0.6])];
        for (i, (l, c)) in legend.iter().enumerate() {
            p.text(lx, my + i as f32 * 14.0, l, 1.0, *c, 1.0);
        }
        p.wrap(lx, my + 100.0, 180.0, "DISTRICTS GATE BY DIFFICULTY. A DISTRICT'S PITS ONLY TAKE ONCE THE PREVIOUS RIFT IS CLOSED.", 1.0, DIM, 1.0);
    }

    fn ui_vendor(&self, p: &mut Painter) {
        let (uw, uh) = (p.w, p.h);
        p.rect(0.0, 0.0, uw, uh, [0.0; 3], 0.7);
        let x0 = (uw - 820.0).max(0.0) * 0.5 + 10.0;
        p.text(x0, 20.0, "QUARTERMASTER - \"DON'T BLEED ON THE STOCK.\"", 1.0, WHITE, 1.0);
        p.text(x0, 32.0, &format!("{} SCRIP", self.pl.gear.scrip), 1.0, [0.9, 0.85, 0.5], 1.0);
        p.panel(x0, 44.0, 300.0, 16.0 + self.prof.vendor_stock.len() as f32 * 14.0);
        for (i, it) in self.prof.vendor_stock.iter().enumerate() {
            let y = 50.0 + i as f32 * 14.0;
            let sel = self.ui.focus == 0 && self.ui.sel == i;
            if sel {
                p.rect(x0 + 2.0, y - 2.0, 296.0, 13.0, ACCENT, 0.3);
            }
            p.text(x0 + 6.0, y, &it.name(&self.t).to_uppercase(), 1.0, it.rarity.color(), 1.0);
            let price = it.value() * 3;
            p.text(x0 + 250.0, y, &format!("{price}"), 1.0, if price <= self.pl.gear.scrip { [0.9, 0.85, 0.5] } else { [0.6, 0.3, 0.3] }, 1.0);
            p.hit(x0, y - 2.0, 300.0, 13.0, Hit::Stock(i));
        }
        let gx = x0 + 310.0;
        p.text(gx, 48.0, "SELL FROM PACK", 1.0, DIM, 1.0);
        self.grid_ui(p, gx, 60.0, &self.pl.gear.inv, 0, self.ui.focus == 1);
        p.text(x0, uh - 24.0, "[ACCEPT] BUY / SELL   [LEFT/RIGHT] SWITCH SIDE   [BACK] CLOSE", 1.0, DIM, 1.0);
        let it = if self.ui.focus == 0 { self.prof.vendor_stock.get(self.ui.sel) } else { self.pl.gear.inv.at(&self.t, self.ui.cursor.0, self.ui.cursor.1).map(|i| &self.pl.gear.inv.items[i].2) };
        if let Some(it) = it {
            let hdr = if self.ui.focus == 0 { format!("BUY FOR {}", it.value() * 3) } else { format!("SELL FOR {}", it.value()) };
            self.tooltip(p, (gx + 230.0).min(uw - 255.0), 200.0, it, self.pl.gear.eq[0].as_ref().filter(|_| it.base(&self.t).slot == Slot::Hand), &hdr);
        }
    }

    fn ui_stash(&self, p: &mut Painter) {
        let (uw, uh) = (p.w, p.h);
        p.rect(0.0, 0.0, uw, uh, [0.0; 3], 0.7);
        let x0 = (uw - 820.0).max(0.0) * 0.5 + 10.0;
        p.text(x0, 30.0, "STASH (SHARED ACROSS LIVES)", 1.0, WHITE, 1.0);
        self.grid_ui(p, x0, 44.0, &self.prof.stash, 1, self.ui.focus == 0);
        let gx = x0 + 12.0 * 22.0 + 20.0;
        p.text(gx, 30.0, "PACK", 1.0, WHITE, 1.0);
        self.grid_ui(p, gx, 44.0, &self.pl.gear.inv, 0, self.ui.focus == 1);
        p.text(x0, uh - 24.0, "[ACCEPT] MOVE ITEM   [LEFT/RIGHT] SWITCH GRID AT THE EDGE   [BACK] CLOSE", 1.0, DIM, 1.0);
        let g = if self.ui.focus == 0 { &self.prof.stash } else { &self.pl.gear.inv };
        if let Some(i) = g.at(&self.t, self.ui.cursor.0, self.ui.cursor.1) {
            self.tooltip(p, (gx + 230.0).min(uw - 255.0), 220.0, &g.items[i].2, None, "SELECTED");
        }
    }

    fn ui_levelup(&self, p: &mut Painter) {
        let (uw, uh) = (p.w, p.h);
        p.rect(0.0, 0.0, uw, uh, [0.06, 0.0, 0.05], 0.75);
        p.text_c(uw * 0.5, 50.0, "THE VIRUS OFFERS", 2.5, [0.9, 0.3, 0.6], 1.0);
        p.text_c(uw * 0.5, 80.0, &format!("LEVEL {} - CHOOSE ONE. IT WILL SHOW.", self.pl.life.level - self.pl.life.pending_levels + 1), 1.0, DIM, 1.0);
        let n = self.mutation_choices.len();
        let cw = 200.0;
        let total = n as f32 * (cw + 12.0) - 12.0;
        for (i, mi) in self.mutation_choices.iter().enumerate() {
            let m = &self.t.mutations[*mi];
            let x = uw * 0.5 - total * 0.5 + i as f32 * (cw + 12.0);
            let y = 110.0;
            let sel = self.ui.sel == i;
            p.panel(x, y, cw, 230.0);
            if sel {
                p.frame(x - 2.0, y - 2.0, cw + 4.0, 234.0, ACCENT, 1.0);
            }
            let tc = [[0.8, 0.8, 0.8], [0.5, 0.7, 1.0], [1.0, 0.8, 0.3], [1.0, 0.4, 0.2]][(m.tier as usize).clamp(1, 4) - 1];
            p.wrap(x + 8.0, y + 10.0, cw - 16.0, &m.name.to_uppercase(), 1.5, tc, 1.0);
            p.text(x + 8.0, y + 40.0, &format!("TIER {}", m.tier), 1.0, DIM, 1.0);
            let h = p.wrap(x + 8.0, y + 58.0, cw - 16.0, &m.desc.to_uppercase(), 1.0, WHITE, 1.0);
            let mut yy = y + 64.0 + h;
            if let Some(ex) = &m.excl {
                p.text(x + 8.0, yy, &format!("EXCLUSIVE: {}", ex.to_uppercase()), 1.0, [1.0, 0.5, 0.4], 1.0);
                yy += 12.0;
            }
            if let Some(t) = &m.tag {
                p.text(x + 8.0, yy, &format!("STRAIN: {}", t.to_uppercase()), 1.0, [0.6, 0.9, 0.7], 1.0);
                yy += 12.0;
            }
            if let Some((tag, _)) = &m.syn {
                p.text(x + 8.0, yy, &format!("SYNERGY WITH {}", tag.to_uppercase()), 1.0, [0.4, 1.0, 0.6], 1.0);
            }
            if m.fx.iter().any(|f| matches!(f, crate::data::Effect::HandWeapon(..) | crate::data::Effect::HandOrgan(..))) {
                p.text(x + 8.0, y + 210.0, "CONVERTS A HAND", 1.0, [1.0, 0.4, 0.4], 1.0);
            }
            p.hit(x, y, cw, 230.0, Hit::Menu(i));
        }
    }

    fn ui_settings(&self, p: &mut Painter) {
        let (uw, uh) = (p.w, p.h);
        p.rect(0.0, 0.0, uw, uh, [0.0; 3], 0.8);
        p.text_c(uw * 0.5, 50.0, "SETTINGS", 2.0, WHITE, 1.0);
        let rows = self.settings_rows();
        let x = uw * 0.5 - 200.0;
        for (i, (k, v)) in rows.iter().enumerate() {
            let y = 90.0 + i as f32 * 18.0;
            let sel = self.ui.sel == i;
            if sel {
                p.rect(x, y - 3.0, 400.0, 15.0, ACCENT, 0.3);
            }
            p.text(x + 8.0, y, k, 1.0, if sel { WHITE } else { DIM }, 1.0);
            p.text(x + 260.0, y, &format!("< {v} >"), 1.0, WHITE, 1.0);
            p.hit(x, y - 3.0, 400.0, 15.0, Hit::Menu(i));
        }
        p.text_c(uw * 0.5, uh - 30.0, "[LEFT/RIGHT] CHANGE   [BACK] SAVE AND RETURN", 1.0, DIM, 1.0);
    }

    fn ui_controls(&self, p: &mut Painter) {
        let (uw, uh) = (p.w, p.h);
        p.rect(0.0, 0.0, uw, uh, [0.0; 3], 0.8);
        p.text_c(uw * 0.5, 40.0, "CONTROLS", 2.0, WHITE, 1.0);
        let x = uw * 0.5 - 230.0;
        p.text(x + 240.0, 64.0, "KEYBOARD/MOUSE", 1.0, DIM, 1.0);
        p.text(x + 360.0, 64.0, "GAMEPAD", 1.0, DIM, 1.0);
        let n = REBINDABLE.len();
        for (i, (a, name)) in REBINDABLE.iter().enumerate() {
            let y = 80.0 + i as f32 * 16.0;
            let sel = self.ui.sel == i;
            if sel {
                p.rect(x, y - 3.0, 460.0, 14.0, ACCENT, 0.25);
                p.rect(x + if self.ui.pad_col { 352.0 } else { 232.0 }, y - 3.0, 110.0, 14.0, ACCENT, 0.45);
            }
            p.text(x + 8.0, y, name, 1.0, if sel { WHITE } else { DIM }, 1.0);
            let m = self.settings.bindings.map.iter().find(|m| m.0 == *a);
            let waiting = |pad| self.rebind_wait == Some((*a, pad));
            let k = if waiting(false) { "PRESS...".into() } else { m.map(|m| m.1.label()).unwrap_or_default() };
            let g = if waiting(true) { "PRESS...".into() } else { m.map(|m| m.2.label()).unwrap_or_default() };
            p.text(x + 240.0, y, &k, 1.0, WHITE, 1.0);
            p.text(x + 360.0, y, &g, 1.0, WHITE, 1.0);
            p.hit(x, y - 3.0, 460.0, 14.0, Hit::Menu(i));
        }
        p.button(x, 80.0 + n as f32 * 16.0 + 6.0, 200.0, "RESET TO DEFAULTS", self.ui.sel == n, n);
        p.text_c(uw * 0.5, uh - 30.0, "[ACCEPT] REBIND   [LEFT/RIGHT] KEYBOARD OR GAMEPAD COLUMN   [BACK] RETURN", 1.0, DIM, 1.0);
    }

    // =============================================================================================
    /// Menu interaction. Called once per frame with fresh input.
    pub fn ui_update(&mut self, dt: f32) {
        self.ui.t += dt;
        let inp = self.input_snapshot.clone();
        let pr = |a: Action| inp.pressed.contains(&a);
        let mouse_hit = |g: &Game| -> Option<Hit> {
            let m = g.ui.mouse;
            g.ui.hits.iter().rev().find(|(r, _)| m.x >= r.x && m.y >= r.y && m.x < r.x + r.z && m.y < r.y + r.w).map(|h| h.1)
        };
        let hover = if inp.mouse_moved || inp.click || inp.rclick { mouse_hit(self) } else { None };
        let up = pr(Action::UiUp) || (pr(Action::MoveF) && !inp.text_mode);
        let down = pr(Action::UiDown) || (pr(Action::MoveB) && !inp.text_mode);
        let left = pr(Action::UiLeft) || pr(Action::MoveL);
        let right = pr(Action::UiRight) || pr(Action::MoveR);
        let accept = pr(Action::UiAccept) || (inp.click && hover.is_some());
        let back = pr(Action::UiBack);
        let alt = pr(Action::UiAlt) || (inp.rclick && hover.is_some());
        let drop = pr(Action::UiDrop);
        let nav = |sel: &mut usize, n: usize| {
            if n == 0 {
                return;
            }
            if up {
                *sel = (*sel + n - 1) % n;
            }
            if down {
                *sel = (*sel + 1) % n;
            }
        };
        if up || down || left || right {
            self.audio.play(Sfx::UiMove, 0.3, 1.0);
        }
        if let Some(Hit::Menu(i)) = hover {
            self.ui.sel = i;
        }
        match self.screen {
            Screen::Title => {
                if self.ui.confirm_new {
                    nav(&mut self.ui.sel, 2);
                    if accept {
                        self.ui.confirm_new = false;
                        if self.ui.sel == 0 {
                            self.new_game();
                        }
                        self.ui.sel = 0;
                    }
                    if back {
                        self.ui.confirm_new = false;
                    }
                    return;
                }
                let items = self.title_items();
                nav(&mut self.ui.sel, items.len());
                if accept {
                    self.audio.play(Sfx::UiSelect, 0.5, 1.0);
                    match items[self.ui.sel.min(items.len() - 1)].as_str() {
                        "CONTINUE" => self.continue_game(),
                        "NEW GAME" => {
                            if self.has_save {
                                self.ui.confirm_new = true;
                                self.ui.sel = 1;
                            } else {
                                self.new_game();
                            }
                        }
                        "SETTINGS" => {
                            self.ui.settings_from_title = true;
                            self.screen = Screen::Settings;
                            self.ui.sel = 0;
                        }
                        "CONTROLS" => {
                            self.ui.settings_from_title = true;
                            self.screen = Screen::Controls;
                            self.ui.sel = 0;
                        }
                        _ => self.quit = true,
                    }
                }
            }
            Screen::Pause => {
                let n = self.pause_items().len();
                nav(&mut self.ui.sel, n);
                if back || pr(Action::Pause) {
                    self.screen = Screen::None;
                }
                if accept {
                    self.audio.play(Sfx::UiSelect, 0.5, 1.0);
                    match self.ui.sel {
                        0 => self.screen = Screen::None,
                        1 => {
                            self.screen = Screen::Inventory;
                            self.ui.focus = 0;
                        }
                        2 => {
                            self.ui.settings_from_title = false;
                            self.screen = Screen::Settings;
                            self.ui.sel = 0;
                        }
                        3 => {
                            self.ui.settings_from_title = false;
                            self.screen = Screen::Controls;
                            self.ui.sel = 0;
                        }
                        4 => {
                            if self.guests.len() < super::coop::MAX_GUESTS {
                                self.join_listen = true;
                                self.msg("PRESS A ON THE NEW CONTROLLER");
                                self.screen = Screen::None;
                            }
                        }
                        5 => {
                            if let Some(s) = self.guests.iter().map(|g| g.slot).max() {
                                self.remove_guest(s);
                            }
                        }
                        6 => {
                            self.save();
                            self.to_title();
                        }
                        _ => {
                            self.save();
                            self.quit = true;
                        }
                    }
                }
            }
            Screen::Settings => {
                nav(&mut self.ui.sel, SETTINGS_ROWS);
                if left {
                    self.adjust_setting(self.ui.sel, -1);
                }
                if right || accept {
                    self.adjust_setting(self.ui.sel, 1);
                }
                if back {
                    super::save::save_settings(&self.settings);
                    self.screen = if self.ui.settings_from_title { Screen::Title } else { Screen::Pause };
                    self.ui.sel = 0;
                }
            }
            Screen::Controls => {
                if self.rebind_wait.is_some() {
                    return;
                }
                nav(&mut self.ui.sel, REBINDABLE.len() + 1);
                if left || right {
                    self.ui.pad_col = !self.ui.pad_col;
                }
                if accept {
                    if self.ui.sel == REBINDABLE.len() {
                        self.settings.bindings = crate::input::Bindings::default();
                        self.settings_dirty = true;
                    } else {
                        self.rebind_wait = Some((REBINDABLE[self.ui.sel].0, self.ui.pad_col));
                    }
                }
                if back {
                    super::save::save_settings(&self.settings);
                    self.screen = if self.ui.settings_from_title { Screen::Title } else { Screen::Pause };
                    self.ui.sel = 0;
                }
            }
            Screen::Inventory => {
                if pr(Action::TabNext) {
                    self.ui.tab = (self.ui.tab + 1) % 4;
                    self.ui.scroll = 0;
                }
                if pr(Action::TabPrev) {
                    self.ui.tab = (self.ui.tab + 3) % 4;
                    self.ui.scroll = 0;
                }
                if let Some(Hit::Tab(i)) = hover {
                    if inp.click {
                        self.ui.tab = i;
                        return;
                    }
                }
                if back || pr(Action::Menu) {
                    self.screen = Screen::None;
                    self.refresh_player();
                    return;
                }
                if self.ui.tab == 1 {
                    if down {
                        self.ui.scroll = (self.ui.scroll + 1).min(self.pl.life.mutations.len().saturating_sub(1));
                    }
                    if up {
                        self.ui.scroll = self.ui.scroll.saturating_sub(1);
                    }
                }
                if self.ui.tab != 0 {
                    return;
                }
                match hover {
                    Some(Hit::Grid(0, x, y)) => {
                        self.ui.focus = 0;
                        self.ui.cursor = (x, y);
                    }
                    Some(Hit::Equip(i)) => {
                        self.ui.focus = 1;
                        self.ui.sel = i;
                    }
                    _ => {}
                }
                let (gw, gh) = (self.pl.gear.inv.w, self.pl.gear.inv.h);
                if self.ui.focus == 0 {
                    self.grid_nav(up, down, left, right, gw, gh, true);
                    if accept || alt {
                        self.equip_from_grid(alt);
                    }
                    if drop {
                        if let Some(i) = self.pl.gear.inv.at(&self.t, self.ui.cursor.0, self.ui.cursor.1) {
                            let it = self.pl.gear.inv.items.remove(i).2;
                            let p = self.ppos() + crate::util::yaw_dir(self.player_body().yaw) + Vec3::Y * 0.5;
                            self.drop_item(p, it);
                            self.audio.play(Sfx::UiSelect, 0.4, 0.7);
                        }
                    }
                } else {
                    nav(&mut self.ui.sel, 7);
                    if right {
                        self.ui.focus = 0;
                    }
                    if accept {
                        self.unequip(self.ui.sel);
                    }
                }
            }
            Screen::Vendor => {
                if back {
                    self.screen = Screen::None;
                    self.refresh_player();
                    return;
                }
                match hover {
                    Some(Hit::Stock(i)) => {
                        self.ui.focus = 0;
                        self.ui.sel = i;
                    }
                    Some(Hit::Grid(0, x, y)) => {
                        self.ui.focus = 1;
                        self.ui.cursor = (x, y);
                    }
                    _ => {}
                }
                if self.ui.focus == 0 {
                    let n = self.prof.vendor_stock.len();
                    nav(&mut self.ui.sel, n);
                    if right {
                        self.ui.focus = 1;
                        self.ui.cursor = (0, 0);
                    }
                    if accept && self.ui.sel < n {
                        let price = self.prof.vendor_stock[self.ui.sel].value() * 3;
                        if self.pl.gear.scrip >= price {
                            let it = self.prof.vendor_stock[self.ui.sel].clone();
                            if self.pl.gear.inv.add(&self.t, it).is_ok() {
                                self.prof.vendor_stock.remove(self.ui.sel);
                                self.pl.gear.scrip -= price;
                                self.audio.play(Sfx::Pickup, 0.6, 0.9);
                                self.ui.sel = self.ui.sel.min(self.prof.vendor_stock.len().saturating_sub(1));
                            } else {
                                self.msg("NO ROOM IN YOUR PACK");
                            }
                        } else {
                            self.msg("NOT ENOUGH SCRIP");
                        }
                    }
                } else {
                    let (gw, gh) = (self.pl.gear.inv.w, self.pl.gear.inv.h);
                    if left && self.ui.cursor.0 == 0 {
                        self.ui.focus = 0;
                    } else {
                        self.grid_nav(up, down, left, right, gw, gh, false);
                    }
                    if accept {
                        if let Some(i) = self.pl.gear.inv.at(&self.t, self.ui.cursor.0, self.ui.cursor.1) {
                            let it = self.pl.gear.inv.items.remove(i).2;
                            self.pl.gear.scrip += it.value();
                            self.prof.vendor_stock.push(it);
                            self.audio.play(Sfx::Pickup, 0.6, 1.3);
                        }
                    }
                }
            }
            Screen::Stash => {
                if back {
                    self.screen = Screen::None;
                    self.save();
                    return;
                }
                match hover {
                    Some(Hit::Grid(1, x, y)) => {
                        self.ui.focus = 0;
                        self.ui.cursor = (x, y);
                    }
                    Some(Hit::Grid(0, x, y)) => {
                        self.ui.focus = 1;
                        self.ui.cursor = (x, y);
                    }
                    _ => {}
                }
                let (gw, gh) = if self.ui.focus == 0 { (self.prof.stash.w, self.prof.stash.h) } else { (self.pl.gear.inv.w, self.pl.gear.inv.h) };
                if self.ui.focus == 0 && right && self.ui.cursor.0 + 1 >= gw {
                    self.ui.focus = 1;
                    self.ui.cursor = (0, self.ui.cursor.1.min(self.pl.gear.inv.h - 1));
                } else if self.ui.focus == 1 && left && self.ui.cursor.0 == 0 {
                    self.ui.focus = 0;
                    self.ui.cursor = (self.prof.stash.w - 1, self.ui.cursor.1);
                } else {
                    self.grid_nav(up, down, left, right, gw, gh, false);
                }
                if accept {
                    let t = &self.t;
                    let (from, to) = if self.ui.focus == 0 { (&mut self.prof.stash, &mut self.pl.gear.inv) } else { (&mut self.pl.gear.inv, &mut self.prof.stash) };
                    if let Some(i) = from.at(t, self.ui.cursor.0, self.ui.cursor.1) {
                        let it = from.items.remove(i);
                        if let Err(back_it) = to.add(t, it.2) {
                            from.items.insert(i, (it.0, it.1, back_it));
                            self.msg("NO ROOM");
                        } else {
                            self.audio.play(Sfx::UiSelect, 0.4, 1.0);
                        }
                    }
                }
            }
            Screen::Pit => {
                let items = self.pit_items();
                nav(&mut self.ui.sel, items.len());
                if back {
                    self.screen = Screen::None;
                }
                if accept {
                    let targets = self.pit_targets();
                    if self.ui.sel == 0 {
                        self.screen = Screen::None;
                        let pit = self.prof.last_pit;
                        self.rest(pit);
                    } else if self.ui.sel <= targets.len() {
                        self.fast_travel(targets[self.ui.sel - 1]);
                    } else {
                        self.screen = Screen::None;
                    }
                }
            }
            Screen::LevelUp => {
                let n = self.mutation_choices.len();
                if left || up {
                    self.ui.sel = (self.ui.sel + n - 1) % n.max(1);
                }
                if right || down {
                    self.ui.sel = (self.ui.sel + 1) % n.max(1);
                }
                if accept && n > 0 {
                    let mi = self.mutation_choices[self.ui.sel.min(n - 1)];
                    self.choose_mutation(mi);
                    self.ui.sel = 0;
                }
            }
            Screen::Note => {
                if back || accept || pr(Action::Interact) {
                    self.screen = Screen::None;
                }
            }
            Screen::Dead => {
                if self.dead_t > 2.5 && (accept || pr(Action::Interact) || pr(Action::Light)) {
                    self.respawn();
                }
            }
            Screen::Credits => {
                if self.ui.t > 4.0 && (accept || back) {
                    self.screen = Screen::None;
                    self.save();
                }
            }
            _ => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn grid_nav(&mut self, up: bool, down: bool, left: bool, right: bool, gw: u8, gh: u8, equip_left: bool) {
        let (x, y) = self.ui.cursor;
        if left {
            if x == 0 && equip_left {
                self.ui.focus = 1;
                self.ui.sel = 0;
                return;
            }
            self.ui.cursor.0 = x.saturating_sub(1);
        }
        if right {
            self.ui.cursor.0 = (x + 1).min(gw - 1);
        }
        if up {
            self.ui.cursor.1 = y.saturating_sub(1);
        }
        if down {
            self.ui.cursor.1 = (y + 1).min(gh - 1);
        }
    }

    fn two_hand_one(&self) -> bool {
        self.pl.stats.has(|e| matches!(e, crate::data::Effect::TwoHandOne))
    }

    /// Equip the item under the cursor. `alt` targets the left hand / second trinket.
    fn equip_from_grid(&mut self, alt: bool) {
        let Some(i) = self.pl.gear.inv.at(&self.t, self.ui.cursor.0, self.ui.cursor.1) else { return };
        let it = self.pl.gear.inv.items[i].2.clone();
        let b = it.base(&self.t);
        let slot = match b.slot {
            Slot::Hand => {
                let class = b.class.unwrap_or(crate::data::WeaponClass::Fist);
                if class == crate::data::WeaponClass::Shield || alt {
                    1
                } else {
                    0
                }
            }
            Slot::Head => 2,
            Slot::Body => 3,
            Slot::Legs => 4,
            Slot::Trinket => {
                if alt || (self.pl.gear.eq[5].is_some() && self.pl.gear.eq[6].is_none()) {
                    6
                } else {
                    5
                }
            }
        };
        let thw = self.two_hand_one();
        if let Err(why) = can_equip(&self.t, &self.pl.gear.eq, slot, &it, thw) {
            self.msg(why);
            self.audio.play(Sfx::UiMove, 0.5, 0.5);
            return;
        }
        let (gx, gy, it) = self.pl.gear.inv.items.remove(i);
        let mut displaced = Vec::new();
        if let Some(old) = self.pl.gear.eq[slot].take() {
            displaced.push(old);
        }
        // a two-hander frees the other hand; equipping into the other hand displaces a two-hander
        if slot <= 1 && !thw {
            let other = 1 - slot;
            let other_2h = self.pl.gear.eq[other].as_ref().map(|o| o.two_handed(&self.t) && !o.grown).unwrap_or(false);
            if (it.two_handed(&self.t) || other_2h) && self.pl.gear.eq[other].as_ref().map(|o| !o.grown).unwrap_or(false) {
                displaced.push(self.pl.gear.eq[other].take().unwrap());
            }
        }
        self.pl.gear.eq[slot] = Some(it);
        let mut first = true;
        for d in displaced {
            let r = if first { self.pl.gear.inv.place(&self.t, gx, gy, d) } else { Err(d) };
            first = false;
            if let Err(d) = r {
                if let Err(d) = self.pl.gear.inv.add(&self.t, d) {
                    let p = self.ppos() + Vec3::Y * 0.5;
                    self.drop_item(p, d);
                    self.msg("NO ROOM - DROPPED");
                }
            }
        }
        self.audio.play(Sfx::Block, 0.3, 1.4);
        self.refresh_player();
    }

    fn unequip(&mut self, slot: usize) {
        if self.pl.gear.eq[slot].as_ref().map(|i| i.grown).unwrap_or(false) {
            self.msg("IT IS PART OF YOU NOW");
            return;
        }
        if let Some(it) = self.pl.gear.eq[slot].take() {
            if let Err(it) = self.pl.gear.inv.add(&self.t, it) {
                self.pl.gear.eq[slot] = Some(it);
                self.msg("NO ROOM IN YOUR PACK");
                return;
            }
            self.audio.play(Sfx::UiSelect, 0.3, 0.8);
            self.refresh_player();
        }
    }
}
