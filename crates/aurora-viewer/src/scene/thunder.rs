//! Weather: thunder and lightning in heavy rain, for this viewer only.
//!
//! Strikes come at random, the more often the harder it rains (and the
//! Strikes setting), each at a random distance (300 m to 6 km, the far ones
//! more often) and direction. Its flash is at once: two to four strokes in a
//! fraction of a second (`flash`, which the app turns into light), and for
//! all but the farthest a bolt in the sky, a new one each time (`bolts`:
//! the main channel jagged by midpoint displacement, with a few forks lit by
//! the first stroke only). Its thunder comes as late as
//! sound takes to cover the distance (3 s a kilometre), from its side (3D
//! for headphones with the 3D sound on, the head-related filters of its
//! direction in the sky, followed as the listener turns): only a near
//! strike cracks, its sharpest highs softened, the others roll, quieter and
//! duller the farther (the air takes the highs out first). Indoors the walls
//! take the highs out and much of the level: in a closed room only a muffled
//! rumble comes through, and the room's own reverb carries it; walking in or
//! out while it rolls, it changes with you.

use super::sound_reverb::{SEND_MAX, Sky};
use aurora_audio::{AudioEngine, Channel, SoundClip};
use aurora_render::{ParticleInstance, particle_flags};
use glam::{Vec2, Vec3};
use std::sync::{Once, OnceLock};
use std::time::{Duration, Instant};

const CRACK_1: &[u8] = include_bytes!("../../../../assets/rain/thunder-crack-1.ogg");
const CRACK_2: &[u8] = include_bytes!("../../../../assets/rain/thunder-crack-2.ogg");
const DOUBLE: &[u8] = include_bytes!("../../../../assets/rain/thunder-double.ogg");
const ROLL: &[u8] = include_bytes!("../../../../assets/rain/thunder-roll.ogg");
const FAR: &[u8] = include_bytes!("../../../../assets/rain/thunder-far.ogg");

/// Voice ids, apart from the rain's and the world sounds'.
const VOICE_BASE: u64 = 0x7468_756E_0000_0000;
/// Overall level of the thunder before the Ambient channel's volume.
const LEVEL: f32 = 0.9;
/// Speed of sound (m/s).
const SOUND_SPEED: f32 = 343.0;
/// No thunder below this much rain.
pub const MIN_RAIN: f32 = 0.4;
/// Strikes' distances (m).
pub(crate) const NEAREST: f32 = 300.0;
pub(crate) const FARTHEST: f32 = 6000.0;
/// Beyond this a strike is only a glow of the clouds, no bolt seen (m).
const BOLT_FARTHEST: f32 = 4500.0;
/// The shortest wait between two strikes (s).
const SHORTEST_WAIT: f32 = 2.0;
/// Strikes nearer than this crack (m); farther, the thunder rolls. Cracks
/// were played up to 3 km (always under 1.2 km, half the time to 3 km): too
/// sharp, too often (the team's feedback, 2026-11-16).
const CRACK_FARTHEST: f32 = 700.0;
/// A crack's least muffle: its sharpest highs taken off (a low-pass near
/// 4.7 kHz), its level nearly kept (the occlusion cuts 12 dB at 1, so 3 dB
/// here).
const CRACK_MUFFLE: f32 = 0.25;
/// Indoors (no rain in sight around the listener): the level kept, and the
/// muffle (`AudioEngine::world_occlusion`: 0.5 one wall, 1 two) at least.
/// The muffle cuts 12 dB of its own at 1: in a closed room the thunder is
/// about 16 dB down and only its rumble left (it was 14 dB down, half
/// muffled, its crack still heard).
const INDOOR_LEVEL: f32 = 0.6;
const INDOOR_MUFFLE: f32 = 0.95;
/// A ringing thunder's muffle is sent again when it moved this much.
const MUFFLE_STEP: f32 = 0.02;
/// Its reverb send, when it moved this share of the most.
const SEND_STEP: f32 = 0.05;
/// The sky's measure glides to the last one in about this long (s), as the
/// rain's does: in a doorway it varies from one measure to the next.
const OPEN_GLIDE_SECS: f32 = 0.5;
/// Close enough to the measure to take it (then the reverb send and muffle
/// left short of it by their steps are sent).
const SETTLED: f32 = 1e-3;
/// The binaural rendering's level against `panned`'s: the head-related
/// filters give a sound ahead 1 per ear (the world sounds' panning), this
/// panning 0.707 (`hrtf.rs`); without it the 3D sound was 3 dB louder.
const BINAURAL_LEVEL: f32 = std::f32::consts::FRAC_1_SQRT_2;
/// How high in the sky the thunder comes from (the sine): above the
/// horizon, from the clouds, not from the ground.
const SKY_ELEVATION: f32 = 0.3;
/// A thunder heard binaurally is turned again once the listener turned by
/// this much (rad).
const TURN_STEP: f32 = 0.05;

/// Which recording: the near ones crack, the far ones roll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Clip {
    Crack1,
    Crack2,
    Double,
    Roll,
    Far,
}

struct Clips([SoundClip; 5]);

/// Where a thunder is heard: through the head-related filters, from a
/// direction in the listener's frame, or panned.
#[derive(Debug, Clone, Copy)]
enum Place {
    Binaural(Vec3),
    Panned(f32),
}

impl Ringing {
    /// Its level, muffle and place for the listener now (`open`: the rain in
    /// sight around them, `Sky::exposure`).
    fn heard(&self, input: &ThunderInput, open: f32) -> (f32, f32, Place) {
        let (level, indoor_muffle) = indoors(open);
        let mut gain = self.gain * level;
        let mut muffle = self.muffle.max(indoor_muffle);
        if input.underwater {
            gain *= 0.5;
            muffle = 1.0;
        }
        let place = if input.binaural {
            gain *= BINAURAL_LEVEL;
            Place::Binaural(heard_from(self.from, input.forward))
        } else {
            Place::Panned(pan_of(self.from, input.forward))
        };
        (gain, muffle, place)
    }

    /// What was sent to the mixer.
    fn mark(&mut self, binaural: bool, gain: f32, muffle: f32, place: Place) {
        self.binaural = binaural;
        self.level = gain;
        self.sent_muffle = muffle;
        self.dir = match place {
            Place::Binaural(d) => d,
            Place::Panned(p) => Vec3::new(p, 0.0, 0.0),
        };
    }
}

impl Clip {
    /// One of the near strikes' recordings (a sharp crack).
    fn cracks(self) -> bool {
        matches!(self, Clip::Crack1 | Clip::Crack2 | Clip::Double)
    }
}

/// The recording for a strike `d` metres away, `r` a draw in 0..1: a crack
/// only near ([`CRACK_FARTHEST`]), a roll farther, the far roll far off.
fn clip_for(d: f32, r: f32) -> Clip {
    if d < CRACK_FARTHEST {
        [Clip::Crack1, Clip::Crack2, Clip::Double][((r * 2.99) as usize).min(2)]
    } else if d < 3000.0 {
        if r < 0.8 { Clip::Roll } else { Clip::Far }
    } else if r < 0.6 {
        Clip::Far
    } else {
        Clip::Roll
    }
}

/// The level kept and the least muffle for a listener with `open` of the
/// falling rain in sight around them (`Sky::exposure`: 1 in the open, 0 in a
/// closed room): indoors only a muffled rumble comes through.
fn indoors(open: f32) -> (f32, f32) {
    let closed = 1.0 - open.clamp(0.0, 1.0);
    (1.0 - (1.0 - INDOOR_LEVEL) * closed, INDOOR_MUFFLE * closed)
}

/// Where a thunder from `from` (world, unit, flat) is heard, in the frame of
/// a listener looking along `forward`: x to the right, y ahead, z up (as
/// `AudioEngine::play_world_binaural`), from above the horizon.
fn heard_from(from: Vec2, forward: Vec3) -> Vec3 {
    let f = forward.normalize_or(Vec3::X);
    let right = f.cross(Vec3::Z).normalize_or(Vec3::Y);
    let up = right.cross(f);
    let flat = from.normalize_or(Vec2::X) * (1.0 - SKY_ELEVATION * SKY_ELEVATION).sqrt();
    let dir = Vec3::new(flat.x, flat.y, SKY_ELEVATION);
    Vec3::new(dir.dot(right), dir.dot(f), dir.dot(up)).normalize_or(Vec3::Y)
}

/// The pan (-1 left .. 1 right) of a thunder from `from` for a listener
/// looking along `forward`, broadly (thunder fills the sky).
fn pan_of(from: Vec2, forward: Vec3) -> f32 {
    let right = forward.cross(Vec3::Z).truncate().normalize_or(Vec2::X);
    (from.dot(right) * 0.6).clamp(-1.0, 1.0)
}

/// (left, right) gains of `gain` at `pan`, constant power.
fn panned(gain: f32, pan: f32) -> (f32, f32) {
    (gain * ((1.0 - pan) * 0.5).sqrt(), gain * ((1.0 + pan) * 0.5).sqrt())
}

impl Clips {
    fn get(&self, c: Clip) -> &SoundClip {
        &self.0[c as usize]
    }
}

fn clips() -> Option<&'static Clips> {
    static CLIPS: OnceLock<Option<Clips>> = OnceLock::new();
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        std::thread::spawn(|| {
            let decode = |name: &str, bytes: &[u8]| SoundClip::decode(bytes).map_err(|e| log::warn!("thunder {name}: {e}")).ok();
            let all = (|| {
                Some(Clips([
                    decode("crack 1", CRACK_1)?,
                    decode("crack 2", CRACK_2)?,
                    decode("double", DOUBLE)?,
                    decode("roll", ROLL)?,
                    decode("far", FAR)?,
                ]))
            })();
            let _ = CLIPS.set(all);
        });
    });
    CLIPS.get().and_then(Option::as_ref)
}

/// What the thunder depends on, this frame.
#[derive(Debug, Clone, Copy)]
pub struct ThunderInput {
    /// Thunder and lightning chosen in the weather settings.
    pub enabled: bool,
    /// How hard it rains (0..1).
    pub rain: f32,
    /// The Strikes setting, 0 (seldom) to 1 (often).
    pub frequency: f32,
    /// Where the listener looks (for the panning).
    pub forward: Vec3,
    /// World sounds rendered binaurally (Preferences › Sound, 3D sound for
    /// headphones), as `Listener::binaural`.
    pub binaural: bool,
    /// The sky around the listener, when measured (softer under a roof).
    pub sky: Option<Sky>,
    /// Under the water.
    pub underwater: bool,
    /// In the world, not a silent demo.
    pub audible: bool,
}

/// One strike's flash: the stepped leader's time (s: the bolt grows down
/// from the cloud, faint, before the first stroke), its strokes (when after
/// the strike, how bright), and the bolt seen, if any.
#[derive(Debug, Clone)]
struct Flash {
    at: Instant,
    leader: f32,
    strokes: Vec<(f32, f32)>,
    bolt: Option<Bolt>,
    /// Where it struck: the way from the viewer (unit), how far (m).
    from: Vec2,
    distance: f32,
}

/// One segment of a bolt, in the strike's own frame (m): x across the view,
/// y away from the viewer, z up from the ground under the strike.
#[derive(Debug, Clone, Copy)]
struct Segment {
    a: Vec3,
    b: Vec3,
    /// Of the main channel's width and brightness.
    weight: f32,
    /// Lit by the first stroke only (a fork).
    fork: bool,
}

/// A bolt: where it is (direction from the viewer, distance) and its shape.
#[derive(Debug, Clone)]
struct Bolt {
    from: Vec2,
    distance: f32,
    segments: Vec<Segment>,
}

/// One strike's thunder, on its way.
#[derive(Debug, Clone, Copy)]
struct Clap {
    at: Instant,
    clip: Clip,
    gain: f32,
    muffle: f32,
    /// Where it comes from (world, unit).
    from: Vec2,
}

/// A thunder playing: placed, muffled and sent to the room's reverb again
/// as the listener turns or walks in or out, while it lasts.
#[derive(Debug, Clone, Copy)]
struct Ringing {
    id: u64,
    from: Vec2,
    /// Its level and muffle in the open, as it came (distance, crack).
    gain: f32,
    muffle: f32,
    until: Instant,
    /// What was last sent: rendered binaurally or panned, where (in the
    /// listener's frame; panned, its pan in x), at what level and muffle.
    binaural: bool,
    dir: Vec3,
    level: f32,
    sent_muffle: f32,
    sent_send: f32,
}

#[derive(Debug)]
pub struct Thunder {
    seed: u64,
    next: Option<Instant>,
    flashes: Vec<Flash>,
    claps: Vec<Clap>,
    ringing: Vec<Ringing>,
    played: u64,
    /// The rain in sight around the listener (`Sky::exposure`), glided, and
    /// when it last was.
    open: f32,
    open_at: Option<Instant>,
    /// A storm was on the frame before.
    was_active: bool,
    /// How bright the lightning is now (0: none; about 1 for a strike a
    /// kilometre away).
    pub flash: f32,
    /// Where the brightest flash now is: the way to it (unit), and the sine
    /// of its glow's height in the sky (the cloud base seen from here).
    pub flash_at: (Vec2, f32),
}

impl Default for Thunder {
    fn default() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0x9e37_79b9, |d| d.as_nanos() as u64)
            | 1;
        Self {
            seed,
            next: None,
            flashes: Vec::new(),
            claps: Vec::new(),
            ringing: Vec::new(),
            played: 0,
            open: 1.0,
            open_at: None,
            was_active: false,
            flash: 0.0,
            flash_at: (Vec2::X, 0.3),
        }
    }
}

impl Thunder {
    /// 0..1, uniform (xorshift).
    fn rand(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        (self.seed >> 40) as f32 / (1u64 << 24) as f32
    }

    /// -1..1, uniform.
    fn signed(&mut self) -> f32 {
        self.rand() * 2.0 - 1.0
    }

    /// The mean time between strikes for a rain of `rain` (s): a minute
    /// and more in moderate rain, about ten seconds in a downpour, at the
    /// middle Strikes setting; four times rarer to four times more often at
    /// its ends.
    fn mean_interval(rain: f32, frequency: f32) -> f32 {
        let t = ((rain - MIN_RAIN) / (1.0 - MIN_RAIN)).clamp(0.0, 1.0);
        let f = frequency.clamp(0.0, 1.0);
        70.0 * (10.0f32 / 70.0).powf(t) * 4.0f32.powf(1.0 - 2.0 * f)
    }

    /// A jagged path from `a` to `b`: each segment's middle pushed sideways
    /// by a share of its length, `depth` times over (2^depth segments).
    fn jagged(&mut self, a: Vec3, b: Vec3, depth: u32, rough: f32) -> Vec<Vec3> {
        let mut pts = vec![a, b];
        for _ in 0..depth {
            let mut next = Vec::with_capacity(pts.len() * 2);
            for w in pts.windows(2) {
                let (p, q) = (w[0], w[1]);
                let len = p.distance(q);
                // sideways mostly across the view, less in depth and height
                let push = Vec3::new(self.signed(), self.signed() * 0.6, self.signed() * 0.25) * len * rough;
                next.push(p);
                next.push((p + q) * 0.5 + push);
            }
            next.push(*pts.last().expect("never empty"));
            pts = next;
        }
        pts
    }

    /// A new bolt: from the cloud base (1.2 to 2 km) down to the ground,
    /// with two to five forks from its upper part.
    fn bolt(&mut self, from: Vec2, distance: f32) -> Bolt {
        let top = Vec3::new(self.signed() * 150.0, self.signed() * 100.0, 1200.0 + self.rand() * 800.0);
        let ground = Vec3::new(self.signed() * 200.0, 0.0, 0.0);
        let main = self.jagged(top, ground, 6, 0.22);
        let mut segments: Vec<Segment> = main
            .windows(2)
            .map(|w| Segment {
                a: w[0],
                b: w[1],
                weight: 1.0,
                fork: false,
            })
            .collect();
        let forks = 2 + (self.rand() * 3.99) as usize;
        for _ in 0..forks {
            // from somewhere on the upper two thirds, down and outward
            let i = (self.rand() * main.len() as f32 * 0.66) as usize;
            let start = main[i.min(main.len() - 1)];
            let length = top.z * (0.15 + self.rand() * 0.3);
            let out = Vec3::new(self.signed(), self.signed() * 0.5, -(0.8 + self.rand() * 0.8)).normalize_or(-Vec3::Z);
            let end = start + out * length;
            let end = Vec3::new(end.x, end.y, end.z.max(0.0));
            let path = self.jagged(start, end, 4, 0.25);
            let n = path.len() - 1;
            segments.extend(path.windows(2).enumerate().map(|(k, w)| Segment {
                a: w[0],
                b: w[1],
                // thinner toward its tip
                weight: 0.55 * (1.0 - 0.7 * k as f32 / n as f32),
                fork: true,
            }));
        }
        Bolt { from, distance, segments }
    }

    /// A strike now, at `distance` m (random when None).
    pub fn strike(&mut self, now: Instant, distance: Option<f32>) {
        let d = distance.unwrap_or_else(|| NEAREST * (FARTHEST / NEAREST).powf(self.rand()));
        let angle = self.rand() * std::f32::consts::TAU;
        let from = Vec2::from_angle(angle);
        // the flash: brighter the nearer, two to four strokes
        let bright = (1400.0 / (400.0 + d)).clamp(0.15, 1.6);
        let n = 2 + (self.rand() * 2.99) as usize;
        let mut strokes = Vec::with_capacity(n);
        // the leader comes down in 50 to 120 ms, then the strokes
        let leader = 0.05 + self.rand() * 0.07;
        let mut t = leader;
        for i in 0..n {
            let amp = if i == 0 { 1.0 } else { 0.4 + self.rand() * 0.6 };
            strokes.push((t, amp * bright));
            t += 0.04 + self.rand() * 0.14;
        }
        let bolt = (d <= BOLT_FARTHEST && self.rand() < 0.85).then(|| self.bolt(from, d));
        self.flashes.push(Flash {
            at: now,
            leader,
            strokes,
            bolt,
            from,
            distance: d,
        });
        // the thunder: a crack near, a roll farther, the far roll far off;
        // quieter and duller with the distance
        let clip = clip_for(d, self.rand());
        let gain = (1500.0 / (500.0 + d)).clamp(0.12, 1.0);
        let mut muffle = ((d - 800.0) / 5000.0).clamp(0.0, 0.7);
        if clip.cracks() {
            muffle = muffle.max(CRACK_MUFFLE);
        }
        self.claps.push(Clap {
            at: now + Duration::from_secs_f32(d / SOUND_SPEED),
            clip,
            gain,
            muffle,
            from,
        });
    }

    /// One frame: maybe a strike, the thunder due, and the flash now.
    pub fn update(&mut self, engine: Option<&AudioEngine>, input: &ThunderInput, now: Instant) {
        let active = input.enabled && input.rain >= MIN_RAIN;
        if active {
            // the clips start decoding with the first storm
            let _ = clips();
            match self.next {
                None => {
                    let mut wait = -self.rand().max(1e-4).ln() * Self::mean_interval(input.rain, input.frequency);
                    // a storm just begun (Thunder switched on, the rain grown
                    // heavy enough): its first strike within a few seconds
                    if !self.was_active {
                        wait = wait.min(3.0 + self.rand() * 5.0);
                    }
                    self.next = Some(now + Duration::from_secs_f32(wait.clamp(SHORTEST_WAIT, 480.0)));
                }
                Some(t) if now >= t => {
                    self.next = None;
                    self.strike(now, None);
                }
                Some(_) => {}
            }
        } else {
            self.next = None;
        }
        self.was_active = active;

        // the flash: each stroke lit at once, fading in about a tenth of a
        // second, and a dimmer glow of the clouds after
        let mut flash = 0.0;
        let mut brightest = (0.0f32, self.flash_at);
        self.flashes.retain(|f| {
            let t = now.saturating_duration_since(f.at).as_secs_f32();
            if t > 1.5 {
                return false;
            }
            let mut this = 0.0;
            for &(at, amp) in &f.strokes {
                if t >= at {
                    let k = t - at;
                    this += amp * ((-k / 0.06).exp() + 0.25 * (-k / 0.4).exp());
                }
            }
            if this > brightest.0 {
                // the cloud base, about 1.5 km up, seen from here
                let height = (1500.0 / f.distance.max(1.0)).atan().sin();
                brightest = (this, (f.from, height));
            }
            flash += this;
            true
        });
        self.flash = flash;
        self.flash_at = brightest.1;

        // the thunder due
        let (due, later): (Vec<Clap>, Vec<Clap>) = self.claps.iter().partition(|c| now >= c.at);
        self.claps = later;
        self.ringing.retain(|r| now < r.until);
        // the sky around the listener: glided while thunder rolls, taken at
        // once otherwise (nothing to hear step)
        let measured = input.sky.map_or(1.0, |s| s.exposure.clamp(0.0, 1.0));
        let dt = self.open_at.map_or(0.0, |t| now.saturating_duration_since(t).as_secs_f32());
        self.open_at = Some(now);
        self.open = if self.ringing.is_empty() {
            measured
        } else {
            let glided = self.open + (measured - self.open) * (1.0 - (-dt / OPEN_GLIDE_SECS).exp());
            if (measured - glided).abs() < SETTLED { measured } else { glided }
        };
        let open = self.open;
        // settled on the measure: what is still a step short is sent anyway
        let settled = open == measured;
        let Some(engine) = engine else {
            return;
        };
        if !input.audible {
            return;
        }
        let Some(clips) = clips() else {
            return;
        };
        for c in due {
            let id = VOICE_BASE + self.played;
            self.played += 1;
            let clip = clips.get(c.clip);
            let mut r = Ringing {
                id,
                from: c.from,
                gain: c.gain * LEVEL,
                muffle: c.muffle,
                until: now + clip.duration(),
                binaural: input.binaural,
                dir: Vec3::ZERO,
                level: 0.0,
                sent_muffle: 0.0,
                sent_send: 0.0,
            };
            let (gain, muffle, place) = r.heard(input, open);
            match place {
                Place::Binaural(dir) => engine.play_world_binaural(id, Channel::Ambient, clip, false, gain, dir.to_array()),
                Place::Panned(pan) => engine.play_world(id, Channel::Ambient, clip, false, panned(gain, pan)),
            }
            // (a voice not playing yet takes these at once)
            engine.world_occlusion(id, muffle);
            r.sent_send = (1.0 - open) * SEND_MAX;
            engine.world_reverb_send(id, r.sent_send);
            r.mark(input.binaural, gain, muffle, place);
            self.ringing.push(r);
        }
        // the thunder still rolling: from the same side of the sky as the
        // listener turns, muffled or not as they walk in or out, rendered as
        // the 3D sound setting is now
        for r in &mut self.ringing {
            let (gain, muffle, place) = r.heard(input, open);
            let moved = match place {
                Place::Binaural(dir) => !r.binaural || dir.angle_between(r.dir) > TURN_STEP,
                Place::Panned(pan) => r.binaural || (pan - r.dir.x).abs() > TURN_STEP,
            } || (gain - r.level).abs() > r.level.max(1e-3) * 0.05;
            if moved {
                match place {
                    Place::Binaural(dir) => engine.world_binaural(r.id, gain, dir.to_array()),
                    Place::Panned(pan) => engine.world_gain(r.id, panned(gain, pan)),
                }
            }
            // the reverb follows the room even where the muffle stays put
            // (a far roll's own muffle above the indoor one)
            let send = (1.0 - open) * SEND_MAX;
            let off = (send - r.sent_send).abs();
            if off > SEND_STEP * SEND_MAX || (settled && off > 1e-4) {
                engine.world_reverb_send(r.id, send);
                r.sent_send = send;
            }
            let muffle_off = (muffle - r.sent_muffle).abs();
            let remuffle = muffle_off > MUFFLE_STEP || (settled && muffle_off > 1e-4);
            if remuffle {
                engine.world_occlusion(r.id, muffle);
            }
            let sent_muffle = if remuffle { muffle } else { r.sent_muffle };
            if moved {
                r.mark(input.binaural, gain, sent_muffle, place);
            } else {
                r.sent_muffle = sent_muffle;
            }
        }
    }

    /// Whether thunder is on its way or still rolling (the sky around the
    /// listener is measured meanwhile, even once the rain stopped: indoors it
    /// stays muffled to its end).
    pub fn rumbling(&self) -> bool {
        !self.claps.is_empty() || !self.ringing.is_empty()
    }

    /// The bolts seen now, as glowing line segments for the particle pass:
    /// `eye` the camera, `ground` the height of the ground far off (m),
    /// `reach` how far the frame draws (m). A bolt farther than that is
    /// drawn just inside it at the same size in the view (by its farthest
    /// point, its top), so that it stands in the sky where it struck, behind
    /// everything the frame draws; the farther it struck, the dimmer and
    /// the more it is greyed by the air between.
    pub fn bolts(&self, now: Instant, eye: Vec3, ground: f32, reach: f32, out: &mut Vec<ParticleInstance>) {
        for f in &self.flashes {
            let Some(bolt) = &f.bolt else {
                continue;
            };
            let t = now.saturating_duration_since(f.at).as_secs_f32();
            // the leader: segments lit as it comes down, faint; then each
            // stroke lights the channel at once and it fades in a few
            // hundredths of a second, with a faint afterglow; the forks only
            // with the first
            let top = bolt.segments.first().map_or(1.0, |s| s.a.z.max(1.0));
            let (mut main, mut fork) = (0.0f32, 0.0f32);
            for (i, &(at, amp)) in f.strokes.iter().enumerate() {
                if t >= at {
                    let k = t - at;
                    let lit = (amp / f.strokes[0].1.max(1e-3)) * ((-k / 0.045).exp() + 0.15 * (-k / 0.25).exp());
                    main += lit;
                    if i == 0 {
                        fork += lit;
                    }
                }
            }
            let leading = t < f.leader;
            if !leading && main < 0.02 {
                continue;
            }
            let across = bolt.from.perp();
            let base = Vec3::new(eye.x + bolt.from.x * bolt.distance, eye.y + bolt.from.y * bolt.distance, ground);
            let world = |p: Vec3| base + (across * p.x + bolt.from * p.y).extend(p.z);
            let farthest = bolt
                .segments
                .iter()
                .map(|s| world(s.a).distance(eye).max(world(s.b).distance(eye)))
                .fold(1.0f32, f32::max);
            let scale = ((reach * 0.97) / farthest).min(1.0);
            let place = |p: Vec3| eye + (world(p) - eye) * scale;
            // the air between: dimmer and greyer the farther
            let air = (-bolt.distance / 3500.0).exp();
            let dim = 0.35 + 0.65 * air;
            let tint = |near: [u8; 3], far: [u8; 3]| {
                let m = |a: u8, b: u8| (f32::from(b) + (f32::from(a) - f32::from(b)) * air) as u8;
                [m(near[0], far[0]), m(near[1], far[1]), m(near[2], far[2])]
            };
            let core_rgb = tint([240, 242, 255], [185, 192, 225]);
            let halo_rgb = tint([150, 160, 255], [130, 138, 190]);
            // a few metres wide where it struck, whatever the distance
            let width = (2.0 + bolt.distance * 0.0025) * scale;
            for s in &bolt.segments {
                let lit = if leading {
                    // the leader has reached this far down yet
                    let reached = f.leader * (top - s.a.z.min(s.b.z)) / top;
                    if t < reached { 0.0 } else { 0.25 }
                } else if s.fork {
                    fork
                } else {
                    main
                } * s.weight
                    * dim;
                if lit < 0.02 {
                    continue;
                }
                let (a, b) = (place(s.a), place(s.b));
                // a little longer than the segment, so that neighbours meet
                let axis = (b - a) * 1.08;
                let mid = (a + b) * 0.5;
                let core = ParticleInstance {
                    pos: mid.to_array(),
                    texture: 0,
                    size: [width * s.weight.max(0.35), axis.length()],
                    flags: particle_flags::EMISSIVE | particle_flags::ADDITIVE | particle_flags::AXIS | particle_flags::LINE,
                    color: [core_rgb[0], core_rgb[1], core_rgb[2], (lit.min(1.0) * 255.0) as u8],
                    axis: axis.to_array(),
                    glow: 2.0 * lit,
                };
                // and a wide violet-blue halo around it
                let halo = ParticleInstance {
                    size: [width * 7.0 * s.weight.max(0.35), axis.length()],
                    color: [halo_rgb[0], halo_rgb[1], halo_rgb[2], (lit.min(1.0) * 90.0) as u8],
                    glow: 0.5 * lit,
                    ..core
                };
                out.push(halo);
                out.push(core);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Thunder with a fixed seed: the same strikes at every run (from the
    /// clock's, now and then a fork wandered more than 45° from its bolt's
    /// direction and a test failed at random).
    fn seeded() -> Thunder {
        Thunder {
            seed: 0x9E37_79B9_7F4A_7C15,
            ..Thunder::default()
        }
    }

    fn input(rain: f32) -> ThunderInput {
        ThunderInput {
            enabled: true,
            rain,
            forward: Vec3::X,
            binaural: false,
            frequency: 0.5,
            sky: None,
            underwater: false,
            audible: false,
        }
    }

    #[test]
    fn only_near_strikes_crack_and_the_rest_roll() {
        for k in 0..20 {
            let r = k as f32 / 20.0;
            assert!(clip_for(300.0, r).cracks(), "300 m, draw {r}");
            assert!(clip_for(CRACK_FARTHEST - 1.0, r).cracks());
            for d in [CRACK_FARTHEST, 1000.0, 1200.0, 2500.0, 4000.0, 6000.0] {
                assert!(!clip_for(d, r).cracks(), "{d} m, draw {r}: a crack");
            }
        }
    }

    #[test]
    fn a_near_strike_cracks_softened_and_a_far_one_rolls() {
        let mut t = seeded();
        t.strike(Instant::now(), Some(400.0));
        let c = t.claps[0];
        assert!(c.clip.cracks() && c.muffle >= CRACK_MUFFLE, "{c:?}");
        t.strike(Instant::now(), Some(2000.0));
        assert!(!t.claps[1].clip.cracks());
        assert!(t.rumbling());
    }

    #[test]
    fn a_ringing_thunder_follows_the_listener_in_and_out_and_the_3d_setting() {
        let r = Ringing {
            id: 1,
            from: Vec2::X,
            gain: 1.0,
            muffle: 0.0,
            until: Instant::now(),
            binaural: false,
            dir: Vec3::ZERO,
            level: 0.0,
            sent_muffle: 0.0,
            sent_send: 0.0,
        };
        let mut i = input(1.0);
        let (out_gain, out_muffle, place) = r.heard(&i, 1.0);
        assert!(matches!(place, Place::Panned(_)) && out_muffle == 0.0);
        let (in_gain, in_muffle, _) = r.heard(&i, 0.0);
        assert!(in_gain < out_gain && in_muffle > 0.9);
        i.binaural = true;
        let (b_gain, _, place) = r.heard(&i, 1.0);
        assert!(matches!(place, Place::Binaural(_)));
        // as loud as panned ahead: the filters give 1 per ear, panning 0.707
        assert!((b_gain - out_gain * BINAURAL_LEVEL).abs() < 1e-6);
    }

    #[test]
    fn the_sky_glides_while_thunder_rolls_and_is_taken_at_once_otherwise() {
        let t0 = Instant::now();
        let mut t = Thunder::default();
        let mut i = input(0.0);
        i.sky = Some(Sky {
            exposure: 0.0,
            ..Sky::OPEN
        });
        // nothing rolling: a room is taken at once
        t.update(None, &i, t0);
        assert_eq!(t.open, 0.0);
        t.ringing.push(Ringing {
            id: 1,
            from: Vec2::X,
            gain: 1.0,
            muffle: 0.0,
            until: t0 + Duration::from_secs(10),
            binaural: false,
            dir: Vec3::ZERO,
            level: 0.0,
            sent_muffle: 0.0,
            sent_send: 0.0,
        });
        // rolling, a doorway's measures jumping 0..1: it moves in steps of
        // far less than the jump, toward the new measure
        i.sky = Some(Sky::OPEN);
        t.update(None, &i, t0 + Duration::from_millis(100));
        assert!(t.open > 0.1 && t.open < 0.3, "{}", t.open);
        let mid = t.open;
        i.sky = Some(Sky {
            exposure: 0.0,
            ..Sky::OPEN
        });
        t.update(None, &i, t0 + Duration::from_millis(200));
        assert!(t.open < mid && t.open > 0.1, "{}", t.open);
        // and settles on a steady measure, exactly
        i.sky = Some(Sky::OPEN);
        t.update(None, &i, t0 + Duration::from_secs(5));
        assert_eq!(t.open, 1.0);
        // once it has rung out, taken at once again
        t.update(None, &i, t0 + Duration::from_secs(11));
        i.sky = Some(Sky {
            exposure: 0.0,
            ..Sky::OPEN
        });
        t.update(None, &i, t0 + Duration::from_millis(11_100));
        assert_eq!(t.open, 0.0);
    }

    #[test]
    fn indoors_the_thunder_is_a_muffled_rumble_and_outdoors_it_is_clear() {
        assert_eq!(indoors(1.0), (1.0, 0.0));
        let (level, muffle) = indoors(0.0);
        assert!((level - INDOOR_LEVEL).abs() < 1e-6);
        assert!(muffle > 0.9, "a closed room barely lets the highs through");
        // a window: in between
        let (l, m) = indoors(0.5);
        assert!(l > INDOOR_LEVEL && l < 1.0 && m > 0.0 && m < INDOOR_MUFFLE);
    }

    #[test]
    fn thunder_is_heard_from_its_side_of_the_sky_as_the_listener_turns() {
        // looking north (+y), a strike to the east is on the right, above
        let d = heard_from(Vec2::X, Vec3::Y);
        assert!(d.x > 0.9 && d.y.abs() < 1e-5 && d.z > 0.2, "{d:?}");
        // looking east, the same strike is ahead
        let d = heard_from(Vec2::X, Vec3::X);
        assert!(d.y > 0.9 && d.x.abs() < 1e-5, "{d:?}");
        // looking west, behind
        assert!(heard_from(Vec2::X, -Vec3::X).y < -0.9);
        // a camera looking down still has a frame
        assert!(heard_from(Vec2::X, Vec3::new(0.0, 0.01, -1.0)).is_finite());
        // panned, the same side: right of a listener looking north, left
        // looking south
        assert!(pan_of(Vec2::X, Vec3::Y) > 0.5);
        assert!(pan_of(Vec2::X, -Vec3::Y) < -0.5);
        for k in 0..8 {
            let a = k as f32 * 0.785;
            let from = Vec2::new(a.cos(), a.sin());
            let fwd = Vec3::new(0.3, 1.0, -0.2);
            let (h, p) = (heard_from(from, fwd).x, pan_of(from, fwd));
            assert!(h * p >= -1e-4, "{from:?}: heard {h}, pan {p}");
        }
        let (l, r) = panned(1.0, 0.0);
        assert!((l - r).abs() < 1e-6 && (l * l + r * r - 1.0).abs() < 1e-5);
    }

    #[test]
    fn a_strike_flashes_at_once_and_thunders_after_the_sound_has_come() {
        let t0 = Instant::now();
        let mut th = seeded();
        th.strike(t0, Some(1715.0));
        // the stroke follows the leader (50 to 120 ms)
        let leader = th.flashes[0].leader;
        th.update(None, &input(0.0), t0 + Duration::from_secs_f32(leader + 0.005));
        assert!(th.flash > 0.5, "{}", th.flash);
        assert_eq!(th.claps.len(), 1);
        // 5 s for 1.7 km: still on its way at 4.9 s, gone at 5.1 s
        th.update(None, &input(0.0), t0 + Duration::from_millis(4900));
        assert_eq!(th.claps.len(), 1);
        assert!(th.flash < 0.01);
        th.update(None, &input(0.0), t0 + Duration::from_millis(5100));
        assert!(th.claps.is_empty());
    }

    #[test]
    fn storms_strike_more_often_in_heavier_rain_and_never_when_off() {
        assert!(Thunder::mean_interval(1.0, 0.5) < 15.0);
        assert!(Thunder::mean_interval(MIN_RAIN, 0.5) > 60.0);
        // the Strikes setting: four times either way
        let mid = Thunder::mean_interval(0.7, 0.5);
        assert!((Thunder::mean_interval(0.7, 0.0) / mid - 4.0).abs() < 1e-3);
        assert!((Thunder::mean_interval(0.7, 1.0) / mid - 0.25).abs() < 1e-3);
        let t0 = Instant::now();
        let mut th = seeded();
        let mut off = input(1.0);
        off.enabled = false;
        for s in 0..600 {
            th.update(None, &off, t0 + Duration::from_secs(s));
        }
        assert!(th.flashes.is_empty() && th.claps.is_empty());
        let mut strikes = 0;
        for s in 0..600 {
            th.update(None, &input(1.0), t0 + Duration::from_secs(s));
            if th.flashes.iter().any(|f| f.at == t0 + Duration::from_secs(s)) {
                strikes += 1;
            }
        }
        assert!((20..=120).contains(&strikes), "{strikes} in 10 minutes");
    }

    #[test]
    fn a_near_bolt_reaches_from_the_clouds_to_the_ground_in_its_direction_then_goes_out() {
        let t0 = Instant::now();
        let mut th = seeded();
        // until one comes with a bolt (most do)
        let mut tries = 0;
        while th.flashes.last().is_none_or(|f| f.bolt.is_none()) {
            th.flashes.clear();
            th.strike(t0, Some(1000.0));
            tries += 1;
            assert!(tries < 50);
        }
        let bolt = th.flashes[0].bolt.clone().expect("bolt");
        // every bolt is new: no two the same
        let other = th.bolt(bolt.from, 1000.0);
        assert!(bolt.segments[5].b.distance(other.segments[5].b) > 1e-3);
        // the main channel is joined end to end, from high up to the ground
        let main: Vec<_> = bolt.segments.iter().filter(|s| !s.fork).collect();
        assert_eq!(main.len(), 64);
        assert!(main.windows(2).all(|w| w[0].b == w[1].a));
        assert!(main[0].a.z >= 1200.0 && main.last().expect("main").b.z.abs() < 1e-3);
        assert!(bolt.segments.iter().all(|s| s.a.z >= -50.0 && s.b.z >= -50.0));

        // drawn at once, nearer than the frame's reach, in its direction
        let eye = Vec3::new(128.0, 128.0, 30.0);
        let mut out = Vec::new();
        // the leader first: only the top lit, faint
        th.bolts(t0 + Duration::from_millis(5), eye, 20.0, 256.0, &mut out);
        let leading = out.len();
        out.clear();
        th.bolts(t0 + Duration::from_millis(130), eye, 20.0, 256.0, &mut out);
        assert!(
            out.len() > leading,
            "the whole bolt once the stroke comes: {leading} -> {}",
            out.len()
        );
        for i in &out {
            let p = Vec3::from_array(i.pos);
            assert!(p.distance(eye) < 256.0, "{p}");
            assert!((p - eye).truncate().normalize().dot(bolt.from) > 0.7, "{p}");
            assert_eq!(i.flags & particle_flags::ADDITIVE, particle_flags::ADDITIVE);
        }
        // out in a second, and none from a far strike
        out.clear();
        th.bolts(t0 + Duration::from_millis(1400), eye, 20.0, 256.0, &mut out);
        assert!(out.is_empty());
        let mut far = seeded();
        far.strike(t0, Some(5500.0));
        assert!(far.flashes[0].bolt.is_none());
    }
}
