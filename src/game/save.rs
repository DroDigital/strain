//! Saves and settings in the platform's user data folder (RON).
use super::loot::Grid;
use super::player::{Gear, Life};
use super::progress::Profile;
use crate::input::Bindings;
use crate::render::GfxSettings;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const SAVE_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SaveFile {
    pub version: u32,
    pub profile: Profile,
    pub gear: Gear,
    pub life: Life,
    pub heal_charges: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub gfx: GfxSettings,
    pub bindings: Bindings,
    pub master: f32,
    pub music: f32,
    pub fov: f32,
    pub shake: f32,
    /// floating damage numbers and combat callouts (off: the hit itself is the feedback)
    #[serde(default)]
    pub damage_numbers: bool,
    /// bumped when defaults change in a way old settings files should pick up
    #[serde(default)]
    pub version: u32,
}
pub const SETTINGS_VERSION: u32 = 2;
impl Default for Settings {
    fn default() -> Self {
        Settings { gfx: GfxSettings::default(), bindings: Bindings::default(), master: 0.8, music: 0.6, fov: 70.0, shake: 1.0, damage_numbers: false, version: SETTINGS_VERSION }
    }
}

pub fn data_dir() -> Option<PathBuf> {
    if let Ok(d) = std::env::var("STRAIN_SAVE_DIR") {
        return Some(PathBuf::from(d));
    }
    directories::ProjectDirs::from("com", "Strain", "Strain").map(|d| d.data_dir().to_path_buf())
}

fn write(name: &str, text: String) {
    if let Some(d) = data_dir() {
        let _ = std::fs::create_dir_all(&d);
        let tmp = d.join(format!("{name}.tmp"));
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, d.join(name));
        }
    }
}
fn read(name: &str) -> Option<String> {
    std::fs::read_to_string(data_dir()?.join(name)).ok()
}

pub fn load_settings() -> Settings {
    let mut s: Settings = read("settings.ron").and_then(|s| ron::from_str(&s).ok()).unwrap_or_default();
    if s.version < 2 {
        // the low-resolution retro look is gone: render at the window's resolution
        s.gfx.internal_height = 0;
        s.version = SETTINGS_VERSION;
    }
    s
}
pub fn save_settings(s: &Settings) {
    if let Ok(t) = ron::ser::to_string_pretty(s, Default::default()) {
        write("settings.ron", t);
    }
}
pub fn load_game() -> Option<SaveFile> {
    let s: SaveFile = ron::from_str(&read("save.ron")?).ok()?;
    (s.version == SAVE_VERSION).then_some(s)
}
pub fn save_game(s: &SaveFile) {
    if let Ok(t) = ron::ser::to_string(s) {
        write("save.ron", t);
    }
}
pub fn has_save() -> bool {
    load_game().is_some()
}
pub fn _grid(_: Grid) {}
