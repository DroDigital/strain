//! Audio: a hand-written mixer on top of a cpal output stream. Every sound effect is synthesised
//! into a buffer at load time; music and ambience are generated in real time inside the callback
//! and layer up with combat intensity and Strain.
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use glam::Vec3;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(usize)]
pub enum Sfx {
    Swing,
    SwingHeavy,
    HitFlesh,
    HitHeavy,
    Block,
    Parry,
    Rupture,
    Step,
    Dodge,
    Hurt,
    Death,
    Growl,
    Shriek,
    Spit,
    Cast,
    Fire,
    Explode,
    Rift,
    Heal,
    Pickup,
    LevelUp,
    UiMove,
    UiSelect,
    Gunshot,
    Siren,
    BurnPit,
    BossRoar,
    Thunder,
    StrainWarn,
    Radio,
    Bloom,
    Larva,
    Heli,
    Count,
}

#[derive(Clone, Copy, Default)]
pub struct MusicState {
    pub combat: f32,
    pub strain: f32,
    pub boss: f32,
    /// 0 held, 1 contested, 2 lost, 3 rift
    pub mood: f32,
    pub rain: f32,
    pub wind: f32,
    pub volume: f32,
    pub music_volume: f32,
    pub muffle: f32,
}

enum Cmd {
    Bank(Vec<Arc<Vec<f32>>>),
    Play { id: usize, l: f32, r: f32, rate: f32 },
    Music(MusicState),
}

struct Voice {
    buf: Arc<Vec<f32>>,
    pos: f64,
    rate: f64,
    l: f32,
    r: f32,
}

pub struct Audio {
    queue: Arc<Mutex<Vec<Cmd>>>,
    _stream: Option<cpal::Stream>,
    pub listener: Vec3,
    pub listener_right: Vec3,
    pub ok: bool,
}

const SR: f32 = 44100.0;

struct Lcg(u32);
impl Lcg {
    fn f(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        ((self.0 >> 8) as f32 / 8388608.0) - 1.0
    }
}

fn env(t: f32, a: f32, d: f32) -> f32 {
    if t < a {
        t / a
    } else {
        (-(t - a) / d).exp()
    }
}

/// Synthesise one sound effect (mono, 44.1 kHz).
fn synth(s: Sfx) -> Vec<f32> {
    use std::f32::consts::TAU;
    let mut rng = Lcg(s as u32 * 7919 + 17);
    let dur = match s {
        Sfx::Siren => 4.0,
        Sfx::Thunder | Sfx::BossRoar | Sfx::Heli => 2.5,
        Sfx::Explode | Sfx::Shriek | Sfx::LevelUp | Sfx::Bloom | Sfx::Rupture | Sfx::Radio => 1.4,
        Sfx::Death | Sfx::Growl | Sfx::Rift | Sfx::BurnPit | Sfx::Heal | Sfx::Fire | Sfx::StrainWarn => 0.9,
        Sfx::UiMove | Sfx::UiSelect | Sfx::Step => 0.12,
        _ => 0.45,
    };
    let n = (dur * SR) as usize;
    let mut out = vec![0.0f32; n];
    let (mut lp, mut lp2, mut ph, mut ph2) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    for (i, o) in out.iter_mut().enumerate() {
        let t = i as f32 / SR;
        let u = t / dur;
        let nz = rng.f();
        let v = match s {
            Sfx::Swing | Sfx::SwingHeavy | Sfx::Dodge => {
                let heavy = if s == Sfx::SwingHeavy { 0.6 } else { 1.0 };
                let cut = (0.04 + 0.5 * (1.0 - (u * 2.0 - 0.7).abs()).max(0.0)) * heavy;
                lp += (nz - lp) * cut;
                lp2 += (lp - lp2) * 0.3;
                (lp - lp2) * env(t, 0.06, 0.1) * 2.2
            }
            Sfx::HitFlesh | Sfx::HitHeavy => {
                let heavy = s == Sfx::HitHeavy;
                ph += (if heavy { 70.0 } else { 110.0 }) * (1.0 - u * 0.6) / SR;
                lp += (nz - lp) * 0.25;
                ((ph * TAU).sin() * env(t, 0.002, if heavy { 0.12 } else { 0.07 }) * 1.2 + lp * env(t, 0.001, 0.04) * 1.4).tanh()
            }
            Sfx::Block => {
                ph += 320.0 / SR;
                ph2 += 487.0 / SR;
                lp += (nz - lp) * 0.6;
                ((ph * TAU).sin() + (ph2 * TAU).sin() * 0.7) * env(t, 0.001, 0.07) * 0.5 + lp * env(t, 0.001, 0.03)
            }
            Sfx::Parry => {
                ph += 1320.0 / SR;
                ph2 += 1987.0 / SR;
                ((ph * TAU).sin() + (ph2 * TAU).sin() * 0.6 + ((ph * 2.7) * TAU).sin() * 0.3) * env(t, 0.001, 0.14) * 0.6 + nz * env(t, 0.0, 0.01)
            }
            Sfx::Rupture => {
                ph += (60.0 + 200.0 * (-t * 8.0).exp()) / SR;
                lp += (nz - lp) * (0.02 + 0.3 * (-t * 4.0).exp());
                ((ph * TAU).sin() * env(t, 0.005, 0.35) * 1.5 + lp * env(t, 0.01, 0.4) * 2.5).tanh()
            }
            Sfx::Step => {
                lp += (nz - lp) * 0.12;
                lp * env(t, 0.003, 0.03) * 1.6
            }
            Sfx::Hurt | Sfx::Death | Sfx::Growl | Sfx::BossRoar | Sfx::Larva => {
                let (f0, rough) = match s {
                    Sfx::Hurt => (150.0, 0.3),
                    Sfx::Death => (120.0 - 60.0 * u, 0.5),
                    Sfx::Growl => (75.0 + 15.0 * (t * 9.0).sin(), 0.7),
                    Sfx::Larva => (420.0 + 200.0 * (t * 40.0).sin(), 0.4),
                    _ => (48.0 + 20.0 * (t * 5.0).sin() - 15.0 * u, 0.9),
                };
                lp += (nz - lp) * 0.02;
                ph += f0 * (1.0 + lp * rough * 4.0) / SR;
                let saw = (ph.fract() * 2.0 - 1.0) * 0.6 + ((ph * 1.51).fract() * 2.0 - 1.0) * 0.4;
                lp2 += (saw - lp2) * (0.08 + 0.15 * (1.0 - u));
                (lp2 * 2.5 * (u * std::f32::consts::PI).sin().powf(0.5)).tanh()
            }
            Sfx::Shriek => {
                ph += (900.0 + 600.0 * (t * 23.0).sin() + 400.0 * u) / SR;
                lp += (nz - lp) * 0.3;
                ((ph * TAU).sin() * 0.6 + ((ph * 1.5) * TAU).sin() * 0.3 + lp * 0.3) * (u * std::f32::consts::PI).sin()
            }
            Sfx::Spit => {
                lp += (nz - lp) * (0.3 - 0.25 * u);
                ph += (300.0 - 200.0 * u) / SR;
                (lp * 1.2 + (ph * TAU).sin() * 0.4) * env(t, 0.01, 0.12)
            }
            Sfx::Cast | Sfx::Rift => {
                let k = if s == Sfx::Rift { 0.5 } else { 1.0 };
                ph += (220.0 * k + 660.0 * k * u * u) / SR;
                ph2 += (331.0 * k + 500.0 * k * u) / SR;
                ((ph * TAU).sin() + (ph2 * TAU).sin() * 0.7 + nz * 0.15) * (u * std::f32::consts::PI).sin() * 0.5
            }
            Sfx::Fire | Sfx::BurnPit => {
                lp += (nz - lp) * 0.08;
                lp2 += (nz * if rng.f() > 0.97 { 3.0 } else { 0.0 } - lp2) * 0.4;
                (lp * 2.0 + lp2) * (u * std::f32::consts::PI).sin().powf(0.3)
            }
            Sfx::Explode | Sfx::Thunder => {
                lp += (nz - lp) * (0.25 * (-t * 5.0).exp() + 0.01);
                ph += (55.0 - 25.0 * u) / SR;
                ((lp * 5.0 + (ph * TAU).sin() * 1.2) * env(t, 0.005, if s == Sfx::Thunder { 0.9 } else { 0.4 })).tanh()
            }
            Sfx::Heal => {
                ph += (440.0 + 220.0 * (u * 3.0).floor()) / SR;
                ((ph * TAU).sin() * 0.4 + ((ph * 2.0) * TAU).sin() * 0.2) * (u * std::f32::consts::PI).sin()
            }
            Sfx::Pickup => {
                ph += (if u < 0.4 { 660.0 } else { 990.0 }) / SR;
                (ph * TAU).sin() * env(t, 0.002, 0.12) * 0.5
            }
            Sfx::LevelUp => {
                let notes = [220.0, 277.0, 330.0, 440.0, 554.0];
                let k = ((u * 5.0) as usize).min(4);
                ph += notes[k] / SR;
                ((ph * TAU).sin() + ((ph * 0.5) * TAU).sin() * 0.5 + ((ph * 1.006).fract() * 2.0 - 1.0) * 0.2) * 0.4 * (1.0 - u).powf(0.5)
            }
            Sfx::UiMove => {
                ph += 520.0 / SR;
                (ph * TAU).sin().signum() * env(t, 0.001, 0.02) * 0.25
            }
            Sfx::UiSelect => {
                ph += (700.0 + 500.0 * u) / SR;
                (ph * TAU).sin().signum() * env(t, 0.001, 0.04) * 0.25
            }
            Sfx::Gunshot => {
                lp += (nz - lp) * (0.7 * (-t * 30.0).exp() + 0.03);
                ph += 90.0 / SR;
                ((lp * 4.0 + (ph * TAU).sin()) * env(t, 0.0005, 0.08)).tanh()
            }
            Sfx::Siren => {
                ph += (520.0 + 180.0 * (t * 1.6 * TAU).sin()) / SR;
                lp += (((ph.fract() * 2.0 - 1.0) * 0.5) - lp) * 0.08;
                lp * (u * std::f32::consts::PI).sin()
            }
            Sfx::StrainWarn => {
                ph += (90.0 + 30.0 * (t * 14.0).sin()) / SR;
                ph2 += 1380.0 / SR;
                ((ph.fract() * 2.0 - 1.0) * 0.5 + (ph2 * TAU).sin() * 0.15 * (t * 20.0).sin()) * (u * std::f32::consts::PI).sin()
            }
            Sfx::Radio => {
                lp += (nz - lp) * 0.5;
                ph += (300.0 + 250.0 * ((t * 7.0).sin() * (t * 3.1).sin())) / SR;
                let voice = (ph * TAU).sin().signum() * 0.25 * ((t * 11.0).sin() > -0.2) as i32 as f32;
                (lp * 0.25 + voice) * if u < 0.05 || u > 0.92 { 1.5 } else { 0.7 } * (u * std::f32::consts::PI).sin().powf(0.2)
            }
            Sfx::Bloom => {
                lp += (nz - lp) * (0.02 + 0.2 * u);
                ph += (60.0 + 240.0 * u * u) / SR;
                (lp * 3.0 + (ph.fract() * 2.0 - 1.0) * 0.5) * (u * std::f32::consts::PI).sin()
            }
            Sfx::Heli => {
                lp += (nz - lp) * 0.05;
                let chop = ((t * 13.0 * TAU).sin() * 0.5 + 0.5).powf(3.0);
                lp * 4.0 * chop * (u * std::f32::consts::PI).sin()
            }
            Sfx::Count => 0.0,
        };
        *o = v * 0.8;
    }
    // short fade to avoid clicks
    let f = (0.004 * SR) as usize;
    for i in 0..f.min(n) {
        out[n - 1 - i] *= i as f32 / f as f32;
    }
    out
}

/// Real-time generative music + ambience.
struct Music {
    st: MusicState,
    cur: MusicState,
    t: f64,
    ph: [f32; 12],
    lp: [f32; 6],
    rng: Lcg,
    step: i32,
    step_t: f32,
    bass_env: f32,
    kick_env: f32,
    kick_ph: f32,
    hat_env: f32,
    arp_env: f32,
    arp_f: f32,
    bass_f: f32,
}
impl Music {
    fn new() -> Self {
        Music { st: MusicState::default(), cur: MusicState::default(), t: 0.0, ph: [0.0; 12], lp: [0.0; 6], rng: Lcg(4242), step: 0, step_t: 0.0, bass_env: 0.0, kick_env: 0.0, kick_ph: 0.0, hat_env: 0.0, arp_env: 0.0, arp_f: 220.0, bass_f: 55.0 }
    }
    fn next(&mut self, sr: f32) -> (f32, f32) {
        use std::f32::consts::TAU;
        let k = 1.0 / sr;
        let a = 0.00005;
        let c = &mut self.cur;
        c.combat += (self.st.combat - c.combat) * a;
        c.strain += (self.st.strain - c.strain) * a;
        c.boss += (self.st.boss - c.boss) * a;
        c.mood += (self.st.mood - c.mood) * a;
        c.rain += (self.st.rain - c.rain) * a;
        c.wind += (self.st.wind - c.wind) * a;
        c.muffle += (self.st.muffle - c.muffle) * a;
        let c = self.cur;
        self.t += k as f64;
        // root note drifts with the district mood
        let root = 55.0 * [1.0, 0.943_874, 0.840_896, 0.749_154][(c.mood.round() as usize).min(3)];
        let scale = [1.0, 1.0595, 1.1892, 1.3348, 1.4983, 1.5874, 1.7818, 2.0];
        // sequencer
        let bpm = 96.0 + 34.0 * c.combat.max(c.boss);
        self.step_t += k * bpm / 60.0 * 4.0;
        if self.step_t >= 1.0 {
            self.step_t -= 1.0;
            self.step = (self.step + 1) % 32;
            let s = self.step;
            if s % 4 == 0 || (c.boss > 0.5 && s % 8 == 6) {
                self.kick_env = 1.0;
                self.kick_ph = 0.0;
            }
            if s % 2 == 1 {
                self.hat_env = if s % 4 == 3 { 1.0 } else { 0.5 };
            }
            let pat = [0, 0, 3, 0, 0, 5, 0, 3, 0, 0, 6, 0, 1, 0, 3, 0];
            if s % 2 == 0 || c.combat > 0.6 {
                self.bass_env = 1.0;
                self.bass_f = root * scale[pat[(s as usize / 2) % 16] % 8] * if s % 16 >= 12 { 2.0 } else { 1.0 };
            }
            let arp = [0, 2, 4, 7, 4, 2, 5, 3];
            self.arp_env = 1.0;
            self.arp_f = root * 4.0 * scale[arp[(s as usize + (s as usize / 8)) % 8]];
        }
        let nz = self.rng.f();
        let adv = |i: usize, f: f32, ph: &mut [f32; 12]| {
            ph[i] = (ph[i] + f * k).fract();
            ph[i]
        };
        // drone: detuned saws through a slow filter
        let lfo = ((self.t * 0.07) as f32 * TAU).sin();
        let d1 = adv(0, root, &mut self.ph) * 2.0 - 1.0;
        let d2 = adv(1, root * 1.007, &mut self.ph) * 2.0 - 1.0;
        let d3 = adv(2, root * 1.4983 * (1.0 + 0.002 * lfo), &mut self.ph) * 2.0 - 1.0;
        let d4 = (adv(3, root * 0.5, &mut self.ph) * TAU).sin();
        self.lp[0] += ((d1 + d2 + d3 * 0.6) * 0.33 - self.lp[0]) * (0.012 + 0.008 * lfo + 0.02 * c.combat);
        let drone = self.lp[0] * 0.5 + d4 * 0.25;
        // bass pulse
        self.bass_env *= 1.0 - 9.0 * k;
        let b = adv(4, self.bass_f, &mut self.ph);
        self.lp[1] += (((b * 2.0 - 1.0) + if b < 0.5 { 0.6 } else { -0.6 }) * 0.5 - self.lp[1]) * (0.03 + 0.12 * self.bass_env);
        let bass = self.lp[1] * self.bass_env * smooth01(c.combat * 1.6);
        // drums
        self.kick_env *= 1.0 - 14.0 * k;
        self.kick_ph += (45.0 + 120.0 * self.kick_env * self.kick_env) * k;
        let kick = (self.kick_ph * TAU).sin() * self.kick_env * smooth01((c.combat - 0.3) * 2.0).max(c.boss);
        self.hat_env *= 1.0 - 60.0 * k;
        self.lp[2] += (nz - self.lp[2]) * 0.6;
        let hat = (nz - self.lp[2]) * self.hat_env * 0.25 * smooth01((c.combat - 0.5) * 2.5);
        // boss arpeggio
        self.arp_env *= 1.0 - 12.0 * k;
        let ar = adv(5, self.arp_f, &mut self.ph);
        let arp = (if ar < 0.3 { 1.0 } else { -1.0 }) * self.arp_env * 0.12 * c.boss;
        // strain: dissonant cluster with tremolo
        let trem = 0.6 + 0.4 * ((self.t * 5.5) as f32 * TAU).sin();
        let mut cl = 0.0;
        for (i, m) in [7.0, 7.42, 8.9, 9.43, 11.9].iter().enumerate() {
            cl += (adv(6 + i, root * m * (1.0 + 0.01 * c.strain * lfo), &mut self.ph) * TAU).sin();
        }
        let strain = cl * 0.05 * trem * c.strain * c.strain;
        // ambience: wind + rain
        self.lp[3] += (nz - self.lp[3]) * (0.004 + 0.004 * (1.0 + lfo));
        self.lp[4] += (self.rng.f() - self.lp[4]) * 0.35;
        self.lp[5] += (self.lp[4] - self.lp[5]) * 0.6;
        let wind = self.lp[3] * 6.0 * (0.25 + c.wind);
        let rain = (self.lp[4] - self.lp[5] * 0.7) * 0.22 * c.rain;
        let music = (drone * (0.5 + 0.2 * c.mood) + bass * 0.55 + kick * 0.6 + hat + arp + strain) * self.st.music_volume * 0.6;
        let amb = wind * 0.12 + rain;
        let pan = ((self.t * 0.11) as f32 * TAU).sin() * 0.15;
        ((music * (1.0 - pan) + amb), (music * (1.0 + pan) + amb * 0.95 + rain * 0.1))
    }
}
fn smooth01(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

struct Mixer {
    queue: Arc<Mutex<Vec<Cmd>>>,
    bank: Vec<Arc<Vec<f32>>>,
    voices: Vec<Voice>,
    music: Music,
    sr: f32,
    lp: [f32; 2],
}
impl Mixer {
    fn fill(&mut self, out: &mut [f32], channels: usize) {
        if let Ok(mut q) = self.queue.try_lock() {
            for c in q.drain(..) {
                match c {
                    Cmd::Bank(b) => self.bank = b,
                    Cmd::Play { id, l, r, rate } => {
                        if let Some(buf) = self.bank.get(id) {
                            if self.voices.len() < 48 {
                                self.voices.push(Voice { buf: buf.clone(), pos: 0.0, rate: (rate * SR / self.sr) as f64, l, r });
                            }
                        }
                    }
                    Cmd::Music(m) => self.music.st = m,
                }
            }
        }
        let vol = self.music.st.volume;
        let muffle = self.music.cur.muffle;
        for frame in out.chunks_mut(channels) {
            let (mut l, mut r) = self.music.next(self.sr);
            for v in &mut self.voices {
                let i = v.pos as usize;
                if i + 1 < v.buf.len() {
                    let f = (v.pos - i as f64) as f32;
                    let s = v.buf[i] * (1.0 - f) + v.buf[i + 1] * f;
                    l += s * v.l;
                    r += s * v.r;
                }
                v.pos += v.rate;
            }
            // slow-time / low health muffling
            let k = 1.0 - 0.93 * muffle;
            self.lp[0] += (l - self.lp[0]) * k;
            self.lp[1] += (r - self.lp[1]) * k;
            let (l, r) = ((self.lp[0] * vol).tanh(), (self.lp[1] * vol).tanh());
            match frame.len() {
                1 => frame[0] = (l + r) * 0.5,
                _ => {
                    frame[0] = l;
                    frame[1] = r;
                    for x in frame.iter_mut().skip(2) {
                        *x = 0.0;
                    }
                }
            }
        }
        self.voices.retain(|v| (v.pos as usize) + 1 < v.buf.len());
    }
}

impl Audio {
    pub fn new() -> Audio {
        let queue = Arc::new(Mutex::new(Vec::new()));
        let stream = Self::open(queue.clone());
        Audio { queue, ok: stream.is_some(), _stream: stream, listener: Vec3::ZERO, listener_right: Vec3::X }
    }
    fn open(queue: Arc<Mutex<Vec<Cmd>>>) -> Option<cpal::Stream> {
        let host = cpal::default_host();
        let device = host.default_output_device()?;
        let sup = device.default_output_config().ok()?;
        let cfg = sup.config();
        let channels = cfg.channels as usize;
        let mut mixer = Mixer { queue, bank: Vec::new(), voices: Vec::new(), music: Music::new(), sr: cfg.sample_rate as f32, lp: [0.0; 2] };
        let err = |e| eprintln!("audio stream error: {e}");
        let mut scratch: Vec<f32> = Vec::new();
        let stream = match sup.sample_format() {
            cpal::SampleFormat::F32 => device.build_output_stream(cfg, move |d: &mut [f32], _: &cpal::OutputCallbackInfo| mixer.fill(d, channels), err, None),
            cpal::SampleFormat::I16 => device.build_output_stream(
                cfg,
                move |d: &mut [i16], _: &cpal::OutputCallbackInfo| {
                    scratch.resize(d.len(), 0.0);
                    mixer.fill(&mut scratch, channels);
                    for (o, s) in d.iter_mut().zip(&scratch) {
                        *o = (s * 32767.0) as i16;
                    }
                },
                err,
                None,
            ),
            cpal::SampleFormat::U16 => device.build_output_stream(
                cfg,
                move |d: &mut [u16], _: &cpal::OutputCallbackInfo| {
                    scratch.resize(d.len(), 0.0);
                    mixer.fill(&mut scratch, channels);
                    for (o, s) in d.iter_mut().zip(&scratch) {
                        *o = ((s * 0.5 + 0.5) * 65535.0) as u16;
                    }
                },
                err,
                None,
            ),
            _ => return None,
        }
        .ok()?;
        stream.play().ok()?;
        Some(stream)
    }
    fn send(&self, c: Cmd) {
        if let Ok(mut q) = self.queue.lock() {
            if q.len() < 256 {
                q.push(c);
            }
        }
    }
    /// Synthesise the whole sound bank (called from the loading screen).
    pub fn build_bank(&self) {
        let bank: Vec<Arc<Vec<f32>>> = (0..Sfx::Count as usize).map(|i| Arc::new(synth(unsafe { std::mem::transmute::<usize, Sfx>(i) }))).collect();
        self.send(Cmd::Bank(bank));
    }
    /// Non-positional sound.
    pub fn play(&self, s: Sfx, vol: f32, rate: f32) {
        self.send(Cmd::Play { id: s as usize, l: vol, r: vol, rate });
    }
    /// Positional sound with distance attenuation and stereo pan.
    pub fn play_at(&self, s: Sfx, pos: Vec3, vol: f32, rate: f32) {
        let d = pos - self.listener;
        let dist = d.length();
        let att = vol / (1.0 + (dist * 0.16).powi(2));
        if att < 0.004 {
            return;
        }
        let pan = if dist > 0.01 { (d / dist).dot(self.listener_right) } else { 0.0 };
        self.send(Cmd::Play { id: s as usize, l: att * (1.0 - pan * 0.7).min(1.0), r: att * (1.0 + pan * 0.7).min(1.0), rate });
    }
    pub fn set_music(&self, m: MusicState) {
        self.send(Cmd::Music(m));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(b: &[f32]) -> f32 {
        (b.iter().map(|x| x * x).sum::<f32>() / b.len().max(1) as f32).sqrt()
    }

    #[test]
    fn every_sound_is_finite_and_audible() {
        for i in 0..Sfx::Count as usize {
            let s: Sfx = unsafe { std::mem::transmute::<usize, Sfx>(i) };
            let b = synth(s);
            assert!(b.iter().all(|x| x.is_finite()), "{s:?} has NaN");
            let peak = b.iter().fold(0.0f32, |a, x| a.max(x.abs()));
            assert!(peak > 0.02, "{s:?} is silent (peak {peak})");
            assert!(peak < 4.0, "{s:?} is far too loud (peak {peak})");
            eprintln!("{s:?}: {} samples, peak {peak:.2}, rms {:.3}", b.len(), rms(&b));
        }
    }

    #[test]
    fn music_layers_stay_bounded() {
        for (combat, strain, boss) in [(0.0, 0.0, 0.0), (1.0, 0.0, 0.0), (1.0, 1.0, 1.0)] {
            let mut m = Music::new();
            m.st = MusicState { combat, strain, boss, mood: 2.0, rain: 1.0, wind: 1.0, volume: 1.0, music_volume: 1.0, muffle: 0.0 };
            m.cur = m.st;
            let mut buf = Vec::new();
            for _ in 0..44100 * 2 {
                let (l, r) = m.next(44100.0);
                assert!(l.is_finite() && r.is_finite());
                buf.push(l);
            }
            let peak = buf.iter().fold(0.0f32, |a, x| a.max(x.abs()));
            eprintln!("music combat {combat} strain {strain} boss {boss}: peak {peak:.2} rms {:.3}", rms(&buf));
            assert!(rms(&buf) > 0.005 && peak < 3.0);
        }
    }

    #[test]
    fn mixer_fills_stereo_buffers() {
        let q = Arc::new(Mutex::new(Vec::new()));
        let mut mx = Mixer { queue: q.clone(), bank: Vec::new(), voices: Vec::new(), music: Music::new(), sr: 48000.0, lp: [0.0; 2] };
        q.lock().unwrap().push(Cmd::Bank((0..Sfx::Count as usize).map(|i| Arc::new(synth(unsafe { std::mem::transmute::<usize, Sfx>(i) }))).collect()));
        q.lock().unwrap().push(Cmd::Music(MusicState { volume: 1.0, music_volume: 1.0, ..Default::default() }));
        for i in 0..Sfx::Count as usize {
            q.lock().unwrap().push(Cmd::Play { id: i, l: 0.5, r: 0.5, rate: 1.0 });
        }
        let mut out = vec![0.0f32; 4096];
        for _ in 0..50 {
            mx.fill(&mut out, 2);
            assert!(out.iter().all(|x| x.is_finite() && x.abs() <= 1.0));
        }
    }
}
