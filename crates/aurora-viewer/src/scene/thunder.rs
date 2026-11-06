//! Weather: thunder and lightning in heavy rain, for this viewer only.
//!
//! Strikes come at random, the more often the harder it rains (and the
//! Strikes setting), each at a random distance (300 m to 6 km, the far ones
//! more often) and direction. Its flash is at once: two to four strokes in a
//! fraction of a second (`flash`, which the app turns into light), and for
//! all but the farthest a bolt in the sky, a new one each time (`bolts`:
//! the main channel jagged by midpoint displacement, with a few forks lit by
//! the first stroke only). Its thunder comes as late as
//! sound takes to cover the distance (3 s a kilometre), from its side: a
//! near strike cracks, a far one rolls, quieter and duller (the air takes
//! the highs out first); under a roof all of it is softer and muffled.

use super::sound_reverb::Sky;
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
const MIN_RAIN: f32 = 0.4;
/// Strikes' distances (m).
const NEAREST: f32 = 300.0;
const FARTHEST: f32 = 6000.0;
/// Beyond this a strike is only a glow of the clouds, no bolt seen (m).
const BOLT_FARTHEST: f32 = 4500.0;
/// The shortest wait between two strikes (s).
const SHORTEST_WAIT: f32 = 2.0;

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

#[derive(Debug)]
pub struct Thunder {
    seed: u64,
    next: Option<Instant>,
    flashes: Vec<Flash>,
    claps: Vec<Clap>,
    played: u64,
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
            played: 0,
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
        let r = self.rand();
        let clip = if d < 1200.0 {
            [Clip::Crack1, Clip::Crack2, Clip::Double][((r * 2.99) as usize).min(2)]
        } else if d < 3000.0 {
            if r < 0.5 { Clip::Roll } else { Clip::Crack2 }
        } else if r < 0.6 {
            Clip::Far
        } else {
            Clip::Roll
        };
        let gain = (1500.0 / (500.0 + d)).clamp(0.12, 1.0);
        let muffle = ((d - 800.0) / 5000.0).clamp(0.0, 0.7);
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
        let Some(engine) = engine else {
            return;
        };
        if !input.audible {
            return;
        }
        let Some(clips) = clips() else {
            return;
        };
        let open = input.sky.map_or(1.0, |s| s.exposure.clamp(0.0, 1.0));
        let right = input.forward.cross(Vec3::Z).truncate().normalize_or(Vec2::X);
        for c in due {
            // from its side, but broadly (thunder fills the sky)
            let pan = (c.from.dot(right) * 0.6).clamp(-1.0, 1.0);
            let (l, r) = (((1.0 - pan) * 0.5).sqrt(), ((1.0 + pan) * 0.5).sqrt());
            let mut gain = c.gain * LEVEL * (0.45 + 0.55 * open);
            let mut muffle = c.muffle.max((1.0 - open) * 0.6);
            if input.underwater {
                gain *= 0.5;
                muffle = 1.0;
            }
            let id = VOICE_BASE + self.played;
            self.played += 1;
            engine.play_world(id, Channel::Ambient, clips.get(c.clip), false, (gain * l, gain * r));
            if muffle > 0.01 {
                engine.world_occlusion(id, muffle);
            }
        }
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

    fn input(rain: f32) -> ThunderInput {
        ThunderInput {
            enabled: true,
            rain,
            forward: Vec3::X,
            frequency: 0.5,
            sky: None,
            underwater: false,
            audible: false,
        }
    }

    #[test]
    fn a_strike_flashes_at_once_and_thunders_after_the_sound_has_come() {
        let t0 = Instant::now();
        let mut th = Thunder::default();
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
        let mut th = Thunder::default();
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
        let mut th = Thunder::default();
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
        let mut far = Thunder::default();
        far.strike(t0, Some(5500.0));
        assert!(far.flashes[0].bolt.is_none());
    }
}
