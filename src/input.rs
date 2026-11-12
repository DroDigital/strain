//! Input: rebindable action map over keyboard, mouse and gamepad.
use glam::Vec2;
use serde::{Deserialize, Serialize};
use winit::event::{ElementState, MouseButton};
use winit::keyboard::KeyCode;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
#[repr(usize)]
pub enum Action {
    MoveF,
    MoveB,
    MoveL,
    MoveR,
    Light,
    Heavy,
    Block,
    Dodge,
    Interact,
    Heal,
    Cast,
    NextSpell,
    LockOn,
    Menu,
    Pause,
    // fixed UI navigation
    UiUp,
    UiDown,
    UiLeft,
    UiRight,
    UiAccept,
    UiBack,
    TabPrev,
    TabNext,
    UiAlt,
    UiDrop,
    Count,
}
pub const REBINDABLE: [(Action, &str); 15] = [
    (Action::MoveF, "MOVE FORWARD"),
    (Action::MoveB, "MOVE BACK"),
    (Action::MoveL, "MOVE LEFT"),
    (Action::MoveR, "MOVE RIGHT"),
    (Action::Light, "LIGHT ATTACK"),
    (Action::Heavy, "HEAVY ATTACK"),
    (Action::Block, "BLOCK / PARRY"),
    (Action::Dodge, "DODGE / SPRINT"),
    (Action::Interact, "INTERACT"),
    (Action::Heal, "HEAL"),
    (Action::Cast, "CAST SPELL"),
    (Action::NextSpell, "NEXT SPELL"),
    (Action::LockOn, "LOCK ON"),
    (Action::Menu, "INVENTORY"),
    (Action::Pause, "PAUSE"),
];

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum Bind {
    Key(String),
    Mouse(u8),
    Pad(String),
    None,
}
impl Bind {
    pub fn label(&self) -> String {
        match self {
            Bind::Key(k) => k.trim_start_matches("Key").trim_start_matches("Digit").to_uppercase(),
            Bind::Mouse(0) => "LMB".into(),
            Bind::Mouse(1) => "RMB".into(),
            Bind::Mouse(2) => "MMB".into(),
            Bind::Mouse(n) => format!("MOUSE{n}"),
            Bind::Pad(p) => format!("PAD {}", p.to_uppercase()),
            Bind::None => "-".into(),
        }
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Bindings {
    /// (action, keyboard/mouse bind, gamepad bind)
    pub map: Vec<(Action, Bind, Bind)>,
    pub mouse_sens: f32,
    pub pad_sens: f32,
    pub invert_y: bool,
}
impl Default for Bindings {
    fn default() -> Self {
        let k = |s: &str| Bind::Key(s.to_string());
        let p = |s: &str| Bind::Pad(s.to_string());
        use Action::*;
        Bindings {
            map: vec![
                (MoveF, k("KeyW"), Bind::None),
                (MoveB, k("KeyS"), Bind::None),
                (MoveL, k("KeyA"), Bind::None),
                (MoveR, k("KeyD"), Bind::None),
                (Light, Bind::Mouse(0), p("RightTrigger")),
                (Heavy, k("KeyF"), p("RightTrigger2")),
                (Block, Bind::Mouse(1), p("LeftTrigger")),
                (Dodge, k("Space"), p("East")),
                (Interact, k("KeyE"), p("South")),
                (Heal, k("KeyR"), p("West")),
                (Cast, k("KeyQ"), p("LeftTrigger2")),
                (NextSpell, k("KeyX"), p("DPadRight")),
                (LockOn, k("Tab"), p("RightThumb")),
                (Menu, k("KeyI"), p("Select")),
                (Pause, k("Escape"), p("Start")),
            ],
            mouse_sens: 1.0,
            pad_sens: 1.0,
            invert_y: false,
        }
    }
}

fn mouse_idx(b: MouseButton) -> u8 {
    match b {
        MouseButton::Left => 0,
        MouseButton::Right => 1,
        MouseButton::Middle => 2,
        MouseButton::Back => 3,
        MouseButton::Forward => 4,
        MouseButton::Other(n) => 5 + n as u8,
    }
}

const N: usize = Action::Count as usize;

/// Buttons of a guest's gamepad (fixed layout, same as the default host pad layout).
pub const PAD_LIGHT: u32 = 1 << 0;
pub const PAD_HEAVY: u32 = 1 << 1;
pub const PAD_BLOCK: u32 = 1 << 2;
pub const PAD_DODGE: u32 = 1 << 3;
pub const PAD_INTERACT: u32 = 1 << 4;
pub const PAD_HEAL: u32 = 1 << 5;
pub const PAD_START: u32 = 1 << 6;
pub const PAD_SELECT: u32 = 1 << 7;

fn pad_bit(b: gilrs::Button) -> u32 {
    use gilrs::Button::*;
    match b {
        RightTrigger => PAD_LIGHT,
        RightTrigger2 => PAD_HEAVY,
        LeftTrigger => PAD_BLOCK,
        East => PAD_DODGE,
        South => PAD_INTERACT,
        West => PAD_HEAL,
        Start => PAD_START,
        Select => PAD_SELECT,
        _ => 0,
    }
}

/// One guest gamepad: sticks and buttons (held, and pressed since the game last consumed them).
#[derive(Clone, Copy, Default, Debug)]
pub struct PadState {
    pub l: Vec2,
    pub r: Vec2,
    pub down: u32,
    pub pressed: u32,
    /// the pad has sent something (a pad that never has leaves the game's state alone)
    pub live: bool,
}

pub struct Input {
    pub bindings: Bindings,
    down: [bool; N],
    pressed: [bool; N],
    released: [bool; N],
    pub look: Vec2,
    pub mouse: Vec2,
    pub mouse_moved: bool,
    pub click: bool,
    pub rclick: bool,
    pub scroll: f32,
    stick_l: Vec2,
    stick_r: Vec2,
    pub using_pad: bool,
    gilrs: Option<gilrs::Gilrs>,
    /// When set, the next key/button press is captured as a new binding for (action, is_pad).
    pub rebinding: Option<(Action, bool)>,
    /// co-op: pads owned by guests (their input never reaches the host), the pad the host last
    /// used, pads asking to join, and a window during which any other pad may join with A
    pub guest_pads: std::collections::HashMap<usize, PadState>,
    pub host_pad: Option<usize>,
    pub joins: Vec<usize>,
    pub join_listen: f32,
}

impl Input {
    pub fn new(bindings: Bindings) -> Self {
        Input {
            bindings,
            down: [false; N],
            pressed: [false; N],
            released: [false; N],
            look: Vec2::ZERO,
            mouse: Vec2::ZERO,
            mouse_moved: false,
            click: false,
            rclick: false,
            scroll: 0.0,
            stick_l: Vec2::ZERO,
            stick_r: Vec2::ZERO,
            using_pad: false,
            gilrs: gilrs::Gilrs::new().ok(),
            rebinding: None,
            guest_pads: std::collections::HashMap::new(),
            host_pad: None,
            joins: Vec::new(),
            join_listen: 0.0,
        }
    }
    /// Inject an action state (used by the scripted test driver).
    pub fn inject(&mut self, a: Action, on: bool) {
        self.set(a, on);
    }
    fn set(&mut self, a: Action, on: bool) {
        let i = a as usize;
        if on && !self.down[i] {
            self.pressed[i] = true;
        }
        if !on && self.down[i] {
            self.released[i] = true;
        }
        self.down[i] = on;
    }
    fn apply(&mut self, b: &Bind, on: bool) {
        let hits: Vec<Action> = self.bindings.map.iter().filter(|(_, k, p)| k == b || p == b).map(|m| m.0).collect();
        for a in hits {
            self.set(a, on);
        }
    }
    fn capture(&mut self, b: Bind) -> bool {
        if let Some((a, pad)) = self.rebinding {
            let is_pad = matches!(b, Bind::Pad(_));
            if is_pad != pad {
                return true;
            }
            if b == Bind::Key("Escape".into()) {
                self.rebinding = None;
                return true;
            }
            for m in &mut self.bindings.map {
                if m.0 == a {
                    if pad {
                        m.2 = b.clone();
                    } else {
                        m.1 = b.clone();
                    }
                }
            }
            self.rebinding = None;
            return true;
        }
        false
    }
    pub fn key(&mut self, code: KeyCode, state: ElementState) {
        let on = state == ElementState::Pressed;
        self.using_pad = false;
        if on && self.capture(Bind::Key(format!("{code:?}"))) {
            return;
        }
        use Action::*;
        match code {
            KeyCode::ArrowUp => self.set(UiUp, on),
            KeyCode::ArrowDown => self.set(UiDown, on),
            KeyCode::ArrowLeft => self.set(UiLeft, on),
            KeyCode::ArrowRight => self.set(UiRight, on),
            KeyCode::Enter | KeyCode::NumpadEnter => self.set(UiAccept, on),
            KeyCode::Escape | KeyCode::Backspace => self.set(UiBack, on),
            KeyCode::PageUp | KeyCode::BracketLeft => self.set(TabPrev, on),
            KeyCode::PageDown | KeyCode::BracketRight => self.set(TabNext, on),
            KeyCode::KeyR => self.set(UiAlt, on),
            KeyCode::KeyG | KeyCode::Delete => self.set(UiDrop, on),
            _ => {}
        }
        self.apply(&Bind::Key(format!("{code:?}")), on);
    }
    pub fn mouse_button(&mut self, b: MouseButton, state: ElementState) {
        let on = state == ElementState::Pressed;
        self.using_pad = false;
        if on && self.capture(Bind::Mouse(mouse_idx(b))) {
            return;
        }
        if on && b == MouseButton::Left {
            self.click = true;
        }
        if on && b == MouseButton::Right {
            self.rclick = true;
        }
        self.apply(&Bind::Mouse(mouse_idx(b)), on);
    }
    pub fn mouse_motion(&mut self, dx: f64, dy: f64) {
        self.look += Vec2::new(dx as f32, dy as f32) * 0.0025 * self.bindings.mouse_sens;
    }
    pub fn cursor(&mut self, x: f32, y: f32) {
        let n = Vec2::new(x, y);
        if (n - self.mouse).length_squared() > 0.5 {
            self.mouse_moved = true;
        }
        self.mouse = n;
    }
    /// Poll gamepads. Call once per frame before the game update.
    pub fn poll_pad(&mut self, dt: f32) {
        let mut evs = Vec::new();
        if let Some(g) = &mut self.gilrs {
            while let Some(ev) = g.next_event() {
                evs.push((usize::from(ev.id), ev.event));
            }
        }
        self.join_listen = (self.join_listen - dt).max(0.0);
        for (pad, ev) in evs {
            use gilrs::{Axis, Button, EventType};
            // guests' pads drive their own players
            if let Some(ps) = self.guest_pads.get_mut(&pad) {
                ps.live = true;
                match ev {
                    EventType::ButtonPressed(b, _) => {
                        ps.down |= pad_bit(b);
                        ps.pressed |= pad_bit(b);
                    }
                    EventType::ButtonReleased(b, _) => ps.down &= !pad_bit(b),
                    EventType::AxisChanged(a, v, _) => {
                        let v = if v.abs() < 0.18 { 0.0 } else { v };
                        match a {
                            Axis::LeftStickX => ps.l.x = v,
                            Axis::LeftStickY => ps.l.y = v,
                            Axis::RightStickX => ps.r.x = v,
                            Axis::RightStickY => ps.r.y = v,
                            _ => {}
                        }
                    }
                    _ => {}
                }
                continue;
            }
            // another pad joins: Start on any pad but the host's, or A while the host is listening
            if let EventType::ButtonPressed(b, _) = ev {
                let other = self.host_pad.is_some_and(|h| h != pad);
                if (b == Button::Start && other) || (self.join_listen > 0.0 && Some(pad) != self.host_pad && matches!(b, Button::South | Button::Start)) {
                    if !self.joins.contains(&pad) {
                        self.joins.push(pad);
                    }
                    continue;
                }
            }
            if matches!(ev, EventType::ButtonPressed(..)) {
                self.host_pad = Some(pad);
            }
            match ev {
                EventType::ButtonPressed(b, _) | EventType::ButtonReleased(b, _) => {
                    let on = matches!(ev, EventType::ButtonPressed(..));
                    self.using_pad = true;
                    if on && self.capture(Bind::Pad(format!("{b:?}"))) {
                        continue;
                    }
                    use Action::*;
                    match b {
                        Button::DPadUp => self.set(UiUp, on),
                        Button::DPadDown => self.set(UiDown, on),
                        Button::DPadLeft => self.set(UiLeft, on),
                        Button::DPadRight => self.set(UiRight, on),
                        Button::South => self.set(UiAccept, on),
                        Button::East => self.set(UiBack, on),
                        Button::LeftTrigger => self.set(TabPrev, on),
                        Button::RightTrigger => self.set(TabNext, on),
                        Button::North => self.set(UiAlt, on),
                        Button::West => self.set(UiDrop, on),
                        _ => {}
                    }
                    self.apply(&Bind::Pad(format!("{b:?}")), on);
                }
                EventType::AxisChanged(a, v, _) => {
                    let v = if v.abs() < 0.18 { 0.0 } else { v };
                    match a {
                        Axis::LeftStickX => self.stick_l.x = v,
                        Axis::LeftStickY => self.stick_l.y = v,
                        Axis::RightStickX => self.stick_r.x = v,
                        Axis::RightStickY => self.stick_r.y = v,
                        _ => {}
                    }
                    if v != 0.0 {
                        self.using_pad = true;
                    }
                }
                _ => {}
            }
        }
        self.look += Vec2::new(self.stick_r.x, -self.stick_r.y) * 2.6 * dt * self.bindings.pad_sens;
    }
    pub fn down(&self, a: Action) -> bool {
        self.down[a as usize]
    }
    pub fn pressed(&self, a: Action) -> bool {
        self.pressed[a as usize]
    }
    pub fn released(&self, a: Action) -> bool {
        self.released[a as usize]
    }
    /// Movement vector: x = right, y = forward, length <= 1.
    pub fn move_axis(&self) -> Vec2 {
        let mut v = self.stick_l;
        if self.down(Action::MoveF) {
            v.y += 1.0;
        }
        if self.down(Action::MoveB) {
            v.y -= 1.0;
        }
        if self.down(Action::MoveR) {
            v.x += 1.0;
        }
        if self.down(Action::MoveL) {
            v.x -= 1.0;
        }
        if v.length_squared() > 1.0 {
            v = v.normalize();
        }
        v
    }
    /// Clear edge-triggered state after the game consumed it.
    pub fn end_frame(&mut self) {
        self.pressed = [false; N];
        self.released = [false; N];
        self.look = Vec2::ZERO;
        self.click = false;
        self.rclick = false;
        self.scroll = 0.0;
        self.mouse_moved = false;
    }
    pub fn bind_label(&self, a: Action) -> String {
        self.bindings.map.iter().find(|m| m.0 == a).map(|m| if self.using_pad { m.2.label() } else { m.1.label() }).unwrap_or_default()
    }
}
