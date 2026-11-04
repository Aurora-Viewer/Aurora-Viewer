//! World and interface sounds, after the Second Life / Firestorm viewer
//! (originally LGPL 2.1, Copyright (C) 2001-2024 Linden Research, Inc. and the
//! Firestorm project): LLViewerObject::setAttachedSound, process_sound_trigger,
//! LLAudioEngine / LLAudioSource (priority by distance) and the FMOD Studio
//! 3D settings LL uses (inverse rolloff, factor 1, 5 under water; minimum
//! distance 1 m), the listener being the camera (MediaSoundsEarLocation 0).
//!
//! Sounds are Ogg Vorbis assets fetched from the ViewerAsset capability,
//! cached on disk and decoded off the main thread.

use super::Scene;
use super::fetch_priority::urgent;
use super::jobs::{JobResult, Jobs};
use crate::world::World;
use aurora_audio::{AudioEngine, Channel, SoundClip};
use aurora_net::fetch::{FetchRequest, FetchResult, Fetcher, Lane};
use aurora_net::objects::AttachedSound;
use glam::Vec3;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

pub mod history;

#[cfg(test)]
mod parcel_tests;

pub const FETCH_KIND_SOUND: u64 = 5 << 60;

/// LL_SOUND_FLAG_*.
const FLAG_LOOP: u8 = 1 << 0;
const FLAG_QUEUE: u8 = 1 << 4;
const FLAG_STOP: u8 = 1 << 5;

/// Voices playing at once (the nearest ones; LL: AudioLevelMaxChannels-ish).
const MAX_VOICES: usize = 32;
/// FMOD minimum distance (full volume closer than this).
const MIN_DISTANCE: f32 = 1.0;
/// AUDIO_LEVEL_ROLLOFF / AUDIO_LEVEL_UNDERWATER_ROLLOFF.
const ROLLOFF: f32 = 1.0;
const UNDERWATER_ROLLOFF: f32 = 5.0;
/// A one-shot sound not loaded by then is dropped (a late sound would be
/// out of place).
const ONE_SHOT_PATIENCE: Duration = Duration::from_secs(10);

/// Collision sounds (LLMaterialTable::isCollisionSound: SL and OpenSim
/// material pairs, llmessage/sound_ids.cpp): EnableCollisionSounds.
pub const COLLISION: [u128; 56] = [
    0xbe7295c0_a158_11e1_b3dd_0804204c9a66,
    0xbe7295c0_a158_11e1_b3dd_0804205c9a66,
    0xbe7295c0_a158_11e1_b3dd_0804206c9a66,
    0xbe7295c0_a158_11e1_b3dd_0802204c9a66,
    0xbe7295c0_a158_11e1_b3dd_0802202c9a66,
    0xbe7295c0_a158_11e1_b3dd_0802205c9a66,
    0xbe7295c0_a158_11e1_b3dd_0802206c9a66,
    0xbe7295c0_a158_11e1_b3dd_0802203c9a66,
    0xbe7295c0_a158_11e1_b3dd_0801204c9a66,
    0xbe7295c0_a158_11e1_b3dd_0801202c9a66,
    0xbe7295c0_a158_11e1_b3dd_0801201c9a66,
    0xbe7295c0_a158_11e1_b3dd_0801205c9a66,
    0xbe7295c0_a158_11e1_b3dd_0801206c9a66,
    0xbe7295c0_a158_11e1_b3dd_0801203c9a66,
    0xbe7295c0_a158_11e1_b3dd_0805205c9a66,
    0xbe7295c0_a158_11e1_b3dd_0805206c9a66,
    0xbe7295c0_a158_11e1_b3dd_0806205c9a66,
    0xbe7295c0_a158_11e1_b3dd_0806206c9a66,
    0xbe7295c0_a158_11e1_b3dd_0800204c9a66,
    0xbe7295c0_a158_11e1_b3dd_0800202c9a66,
    0xbe7295c0_a158_11e1_b3dd_0800201c9a66,
    0xbe7295c0_a158_11e1_b3dd_0800205c9a66,
    0xbe7295c0_a158_11e1_b3dd_0800206c9a66,
    0xbe7295c0_a158_11e1_b3dd_0800200c9a66,
    0xbe7295c0_a158_11e1_b3dd_0800203c9a66,
    0xbe7295c0_a158_11e1_b3dd_0803205c9a66,
    0xbe7295c0_a158_11e1_b3dd_0803206c9a66,
    0xbe7295c0_a158_11e1_b3dd_0803203c9a66,
    0xdce5fdd4_afe4_4ea1_822f_dd52cac46b08,
    0x51011582_fbca_4580_ae9e_1a5593f094ec,
    0x68d62208_e257_4d0c_bbe2_20c9ea9760bb,
    0x75872e8c_bc39_451b_9b0b_042d7ba36cba,
    0x6a45ba0b_5775_4ea8_8513_26008a17f873,
    0x992a6d1b_8c77_40e0_9495_4098ce539694,
    0x2de4da5a_faf8_46be_bac6_c4d74f1e5767,
    0x6e3fb0f7_6d9c_42ca_b86b_1122ff562d7d,
    0x14209133_4961_4acc_9649_53fc38ee1667,
    0xbc4a4348_cfcc_4e5e_908e_8a52a8915fe6,
    0x9e5c1297_6eed_40c0_825a_d9bcd86e3193,
    0xe534761c_1894_4b61_b20c_658a6fb68157,
    0x8761f73f_6cf9_4186_8aaa_0948ed002db1,
    0x874a26fd_142f_4173_8c5b_890cd846c74d,
    0x0e24a717_b97e_4b77_9c94_b59a5a88b2da,
    0x75cf3ade_9a5b_4c4d_bb35_f9799bda7fb2,
    0x153c8bf7_fb89_4d89_b263_47e58b1b4774,
    0x55c3e0ce_275a_46fa_82ff_e0465f5e8703,
    0x24babf58_7156_4841_9a3f_761bdbb8e237,
    0xaca261d8_e145_4610_9e20_9eff990f2c12,
    0x0642fba6_5dcf_4d62_8e7b_94dbb529d117,
    0x25a863e8_dc42_4e8a_a357_e76422ace9b5,
    0x9538f37c_456e_4047_81be_6435045608d4,
    0x8c0f84c3_9afd_4396_b5f5_9bca2c911c20,
    0xbe582e5d_b123_41a2_a150_454c39e961c8,
    0xc70141d4_ba06_41ea_bcbc_35ea81cb8335,
    0x7d1826f4_24c4_4aac_8c2e_eff45df37783,
    0x063c97d3_033a_4e9b_98d8_05c8074922cb,
];

pub fn is_collision(id: &Uuid) -> bool {
    COLLISION.contains(&id.as_u128())
}

#[derive(Default)]
struct Clip {
    clip: Option<Arc<SoundClip>>,
    /// 0 new, 1 reading the cache, 2 downloading, 3 ready, 4 failed.
    state: u8,
    failures: u32,
    retry_at: Option<Instant>,
}

#[derive(Debug, Clone, Copy)]
struct Playing {
    voice: u64,
    started: Instant,
    /// None for loops.
    length: Option<Duration>,
    gain: (f32, f32),
    /// Direction last sent in the listener's frame, when rendered
    /// binaurally (3D sound for headphones); None when panned.
    direction: Option<Vec3>,
}

/// Sound of an object (LLAudioSourceVO).
struct Source {
    owner: Uuid,
    sound: Uuid,
    gain: f32,
    flags: u8,
    /// Waiting to start (since).
    pending: Option<Instant>,
    /// llPlaySound queue (LL_SOUND_FLAG_QUEUE): next sound once this one ends.
    queued: Option<Uuid>,
    playing: Option<Playing>,
}

/// Sound triggered at a place (LLAudioEngine::triggerSound).
struct Triggered {
    source: Uuid,
    owner: Uuid,
    sound: Uuid,
    /// Render-space position, or an object to follow.
    pos: Vec3,
    follow: Option<Uuid>,
    gain: f32,
    asked: Instant,
    playing: Option<Playing>,
}

/// What the scene needs from the app each frame.
pub struct Listener {
    pub pos: Vec3,
    /// Camera right axis (stereo panning).
    pub right: Vec3,
    /// Camera look and up axes (with `right`, the frame of the 3D sound).
    pub forward: Vec3,
    pub up: Vec3,
    pub underwater: bool,
    /// World sounds rendered binaurally, for headphones (Aurora's own,
    /// Preferences › Sound; off: panned, as in the Legacy Viewer).
    pub binaural: bool,
}

impl Listener {
    /// A listener at `pos` looking along `forward` (any length), the up of
    /// the world above it.
    pub fn looking(pos: Vec3, forward: Vec3, underwater: bool, binaural: bool) -> Listener {
        let forward = forward.normalize_or(Vec3::X);
        let right = forward.cross(Vec3::Z).normalize_or(Vec3::X);
        Listener {
            pos,
            right,
            forward,
            up: right.cross(forward).normalize_or(Vec3::Z),
            underwater,
            binaural,
        }
    }
}

/// `AURORA_SOUND_HRTF=0|1`: the 3D sound for headphones forced off or on
/// for this launch, without saving it.
pub fn binaural_forced() -> Option<bool> {
    static FORCED: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
    *FORCED.get_or_init(|| match std::env::var("AURORA_SOUND_HRTF").as_deref() {
        Ok("0") => Some(false),
        Ok("1") => Some(true),
        _ => None,
    })
}

/// How a playing world sound is placed for the listener.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Placement {
    /// (left, right) gains.
    Panned((f32, f32)),
    /// Distance gain and direction in the listener's frame (x right, y
    /// ahead, z up), for the head-related filters.
    Binaural(f32, Vec3),
}

impl Placement {
    /// The gains recorded in `Playing` (the distance gain on both sides
    /// when binaural).
    fn gains(self) -> (f32, f32) {
        match self {
            Placement::Panned(g) => g,
            Placement::Binaural(g, _) => (g, g),
        }
    }
}

/// A binaural sound's direction is sent again once it turned by this much
/// (cos 1 degree): the mixer glides between them.
const DIRECTION_RESEND_COS: f32 = 0.999_85;

pub struct Origin {
    pub object: Uuid,
    pub owner: Uuid,
}

/// A world sound playing at a place this frame (not at the listener, as
/// HUD sounds are), for its occlusion (scene/sound_occlusion.rs).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Audible {
    pub voice: u64,
    pub pos: Vec3,
    /// The object that plays it (an attached sound's object, a triggered
    /// sound's source), nil when unknown.
    pub emitter: Uuid,
}

pub struct SoundManager {
    pub history: history::History,
    previews: HashMap<Uuid, (Instant, Option<Playing>)>,
    clips: HashMap<Uuid, Clip>,
    by_key: HashMap<u64, Uuid>,
    next_key: u64,
    cache_dir: PathBuf,
    rx: crossbeam_channel::Receiver<(Uuid, Option<Arc<SoundClip>>, bool)>,
    tx: crossbeam_channel::Sender<(Uuid, Option<Arc<SoundClip>>, bool)>,
    /// Object sounds by object id.
    sources: HashMap<Uuid, Source>,
    triggered: Vec<Triggered>,
    /// Interface sounds waiting for their clip.
    ui_pending: Vec<(Uuid, Instant)>,
    next_voice: u64,
    /// Sound cut-off radius of objects (llSetSoundRadius), from their updates.
    radius: HashMap<Uuid, f32>,
    /// The world sounds playing at a place, from the last update.
    audible: Vec<Audible>,
}

impl SoundManager {
    /// pipeline.cpp walks every audio source, including sources whose
    /// object is invisible and triggered sounds without a drawable.
    pub fn beacon_sources(&self, world: &World, now: Instant) -> Vec<(Option<usize>, Vec3)> {
        let position = |id: &Uuid| {
            let idx = world.objects.index_of_uuid(id)?;
            let (pos, _, hud) = Scene::object_transform(world, idx, now, 0)?;
            (!hud).then_some((Some(idx), pos))
        };
        self.sources
            .iter()
            .filter(|(_, source)| source.playing.is_some() || source.pending.is_some())
            .filter_map(|(id, _)| position(id))
            .chain(
                self.triggered
                    .iter()
                    .map(|s| s.follow.as_ref().and_then(position).unwrap_or((None, s.pos))),
            )
            .collect()
    }

    pub fn new(cache_dir: PathBuf) -> SoundManager {
        let _ = std::fs::create_dir_all(cache_dir.join("sound"));
        let (tx, rx) = crossbeam_channel::unbounded();
        SoundManager {
            history: Default::default(),
            previews: HashMap::new(),
            clips: HashMap::new(),
            by_key: HashMap::new(),
            next_key: 1,
            cache_dir,
            rx,
            tx,
            sources: HashMap::new(),
            triggered: Vec::new(),
            ui_pending: Vec::new(),
            next_voice: 1,
            radius: HashMap::new(),
            audible: Vec::new(),
        }
    }

    /// A sound made locally (offline demo).
    pub fn insert(&mut self, id: Uuid, clip: SoundClip) {
        self.clips.insert(
            id,
            Clip {
                clip: Some(Arc::new(clip)),
                state: 3,
                failures: 0,
                retry_at: None,
            },
        );
    }

    /// Ask for a sound (PreloadSound, or before playing it).
    pub fn want(&mut self, id: Uuid) {
        if !id.is_nil() {
            self.clips.entry(id).or_default();
        }
    }

    pub fn asset_settled(&self, id: &Uuid) -> bool {
        self.clips.get(id).is_some_and(|entry| entry.state >= 3)
    }

    fn ready(&self, id: &Uuid) -> Option<Arc<SoundClip>> {
        self.clips.get(id).and_then(|c| c.clip.clone())
    }

    fn failed(&self, id: &Uuid) -> bool {
        self.clips.get(id).is_some_and(|c| c.state == 4 && c.failures > 3)
    }

    /// LLViewerObject::setAttachedSound (object updates and AttachedSound).
    pub fn set_attached(&mut self, engine: Option<&AudioEngine>, object: Uuid, sound: Uuid, gain: f32, flags: u8) {
        let gain = if gain.is_finite() { gain.clamp(0.0, 1.0) } else { 0.0 };
        if sound.is_nil() {
            let Some(s) = self.sources.get_mut(&object) else {
                return;
            };
            if s.flags & FLAG_LOOP != 0 {
                // llStopSound: the loop goes away
                if let (Some(p), Some(e)) = (s.playing, engine) {
                    e.stop_world(p.voice);
                }
                self.sources.remove(&object);
            } else if flags & FLAG_STOP != 0 {
                if let (Some(p), Some(e)) = (s.playing.take(), engine) {
                    e.stop_world(p.voice);
                }
                s.pending = None;
            }
            return;
        }
        // already looping this sound: only the volume changes
        if flags & FLAG_LOOP != 0
            && let Some(s) = self.sources.get_mut(&object)
            && s.flags & FLAG_LOOP != 0
            && s.sound == sound
            && (s.playing.is_some() || s.pending.is_some())
        {
            s.gain = gain;
            return;
        }
        self.want(sound);
        let s = self.sources.entry(object).or_insert(Source {
            owner: Uuid::nil(),
            sound,
            gain,
            flags,
            pending: None,
            queued: None,
            playing: None,
        });
        let queue = flags & FLAG_QUEUE != 0;
        s.gain = gain;
        if queue && s.playing.is_some_and(|p| p.length.is_some()) {
            // after the current one
            s.queued = Some(sound);
            s.flags = flags;
            return;
        }
        // not queued: stop the current sound first (SL-1541)
        if let (Some(p), Some(e)) = (s.playing.take(), engine) {
            e.stop_world(p.voice);
        }
        s.sound = sound;
        s.flags = flags;
        s.queued = None;
        s.pending = Some(Instant::now());
    }

    /// AttachedSoundGainChange (llAdjustSoundVolume).
    pub fn set_gain(&mut self, object: Uuid, gain: f32) {
        if let Some(s) = self.sources.get_mut(&object) {
            s.gain = if gain.is_finite() { gain.clamp(0.0, 1.0) } else { 0.0 };
        }
    }

    /// Sound cut-off radius of an object (0 = none).
    pub fn set_radius(&mut self, object: Uuid, radius: f32) {
        if radius >= 0.1 {
            self.radius.insert(object, radius);
        } else {
            self.radius.remove(&object);
        }
    }

    /// A sound played once at a place (or following an object).
    pub fn trigger(&mut self, sound: Uuid, pos: Vec3, follow: Option<Uuid>, gain: f32) {
        let source = follow.unwrap_or_default();
        self.trigger_owned(
            sound,
            pos,
            follow,
            gain,
            Origin {
                object: source,
                owner: source,
            },
        );
    }

    pub fn trigger_owned(&mut self, sound: Uuid, pos: Vec3, follow: Option<Uuid>, gain: f32, origin: Origin) {
        let gain = if gain.is_finite() { gain.clamp(0.0, 1.0) } else { 0.0 };
        if sound.is_nil() || gain < f32::EPSILON * 2.0 || self.triggered.len() > 128 {
            return;
        }
        self.want(sound);
        self.triggered.push(Triggered {
            source: origin.object,
            owner: origin.owner,
            sound,
            pos,
            follow,
            gain,
            asked: Instant::now(),
            playing: None,
        });
    }

    /// An interface sound (Ui channel, not placed).
    pub fn play_ui(&mut self, engine: Option<&AudioEngine>, sound: Uuid) {
        match (self.ready(&sound), engine) {
            (Some(c), Some(e)) => e.play_clip(Channel::Ui, &c, 1.0),
            _ => {
                self.want(sound);
                self.ui_pending.push((sound, Instant::now()));
            }
        }
    }

    /// Stop everything (region change, logout).
    pub fn clear(&mut self, engine: Option<&AudioEngine>) {
        self.stop_previews(engine);
        self.history.sync(std::iter::empty(), Instant::now());
        if let Some(e) = engine {
            for s in self.sources.values() {
                if let Some(p) = s.playing {
                    e.stop_world(p.voice);
                }
            }
            for t in &self.triggered {
                if let Some(p) = t.playing {
                    e.stop_world(p.voice);
                }
            }
        }
        self.sources.clear();
        self.triggered.clear();
        self.radius.clear();
        self.audible.clear();
    }

    // ------------------------------------------------------------ assets

    /// Cache reads and downloads of the wanted sounds.
    pub fn fetch(&mut self, jobs: &Jobs, fetcher: &Fetcher, viewer_asset: Option<&str>) {
        while let Ok((id, clip, from_cache)) = self.rx.try_recv() {
            if let Some(c) = self.clips.get_mut(&id) {
                match clip {
                    Some(clip) => {
                        c.clip = Some(clip);
                        c.state = 3;
                    }
                    None if from_cache => c.state = 4,
                    None => {
                        c.state = 4;
                        c.failures += 1;
                        c.retry_at = Some(Instant::now() + Duration::from_secs(30));
                    }
                }
            }
        }
        let now = Instant::now();
        for (id, c) in self.clips.iter_mut() {
            if c.retry_at.is_some_and(|t| now < t) {
                continue;
            }
            if c.state == 0 {
                c.state = 1;
                let path = self.cache_dir.join("sound").join(format!("{id}.ogg"));
                let tx = self.tx.clone();
                let id = *id;
                jobs.spawn(move || {
                    let clip = crate::cache::read_touch(&path)
                        .ok()
                        .and_then(|d| SoundClip::decode(&d).ok())
                        .map(Arc::new);
                    let _ = tx.send((id, clip, true));
                    JobResult::Done
                });
            } else if c.state == 4 && c.failures <= 3 {
                let Some(base) = viewer_asset else {
                    continue;
                };
                let key = FETCH_KIND_SOUND | self.next_key;
                self.next_key += 1;
                self.by_key.insert(key, *id);
                c.state = 2;
                fetcher.request(FetchRequest {
                    key,
                    url: super::textures::asset_url(base, "sound_id", id),
                    range: None,
                    // sounds are small and late ones are useless
                    priority: urgent::SOUND,
                    lane: Lane::Urgent,
                    accept: "*/*",
                });
            }
        }
    }

    pub fn on_fetch(&mut self, r: FetchResult, jobs: &Jobs) {
        let Some(id) = self.by_key.remove(&r.key) else {
            return;
        };
        match r.data {
            Ok(d) => {
                let path = self.cache_dir.join("sound").join(format!("{id}.ogg"));
                let tx = self.tx.clone();
                jobs.spawn(move || {
                    let clip = match SoundClip::decode(&d) {
                        Ok(c) => {
                            let _ = crate::cache::write(path, &d);
                            Some(Arc::new(c))
                        }
                        Err(e) => {
                            log::debug!("sound {id}: {e}");
                            None
                        }
                    };
                    let _ = tx.send((id, clip, false));
                    JobResult::Done
                });
            }
            Err(_) => {
                if let Some(c) = self.clips.get_mut(&id) {
                    c.state = 4;
                    c.failures += 1;
                    c.retry_at = Some(Instant::now() + Duration::from_secs(if r.status == 404 { 600 } else { 10 }));
                }
            }
        }
    }

    // ------------------------------------------------------------ playback

    /// The distance attenuation at `d` metres (FMOD's inverse rolloff).
    fn attenuation(l: &Listener, d: f32) -> f32 {
        let rolloff = if l.underwater { UNDERWATER_ROLLOFF } else { ROLLOFF };
        if d <= MIN_DISTANCE {
            1.0
        } else {
            MIN_DISTANCE / (MIN_DISTANCE + rolloff * (d - MIN_DISTANCE))
        }
    }

    /// (left, right) gains of a sound at `pos` for the listener.
    fn gains(l: &Listener, pos: Vec3, gain: f32) -> (f32, f32) {
        let to = pos - l.pos;
        let d = to.length();
        let g = gain * Self::attenuation(l, d);
        // constant power panning, centered at 0.707 x sqrt 2 = 1 per side
        let pan = if d > 1e-3 { (to / d).dot(l.right).clamp(-1.0, 1.0) } else { 0.0 };
        let a = (pan + 1.0) * std::f32::consts::FRAC_PI_4;
        let s = std::f32::consts::SQRT_2;
        ((a.cos() * s).min(1.0) * g, (a.sin() * s).min(1.0) * g)
    }

    /// The placement of a sound at `pos`: binaural when the listener wants
    /// it, except at the listener's head (HUD sounds, a source at the
    /// camera), which stays centred as panned.
    fn placement(l: &Listener, pos: Vec3, gain: f32, at_listener: bool) -> Placement {
        let to = pos - l.pos;
        if !l.binaural || at_listener || to.length() <= 1e-3 {
            return Placement::Panned(Self::gains(l, pos, gain));
        }
        let d = to.length();
        let dir = to / d;
        Placement::Binaural(
            gain * Self::attenuation(l, d),
            Vec3::new(dir.dot(l.right), dir.dot(l.forward), dir.dot(l.up)),
        )
    }

    /// Start, place and end the sounds (every frame).
    pub fn update(&mut self, engine: Option<&AudioEngine>, world: &World, l: &Listener, now: Instant) {
        self.update_previews(engine, world, now);
        // Blacklist checks cover pending assets and already playing sounds,
        // including AttachedSound packets whose snapshot still contains nil.
        let blocked: Vec<_> = self
            .sources
            .iter()
            .filter_map(|(id, s)| {
                let owner = world
                    .objects
                    .index_of_uuid(id)
                    .and_then(|i| world.objects.get(i))
                    .map_or(s.owner, |o| if s.owner.is_nil() { o.owner_id } else { s.owner });
                (world.sound_blocked(id, &owner) || world.sound_blacklisted(s.sound, *id, owner)).then_some(*id)
            })
            .collect();
        for id in blocked {
            if let Some(s) = self.sources.remove(&id)
                && let (Some(p), Some(e)) = (s.playing, engine)
            {
                e.stop_world(p.voice);
            }
        }
        self.triggered.retain(|t| {
            if !world.sound_blocked(&t.source, &t.owner) && !world.sound_blacklisted(t.sound, t.source, t.owner) {
                return true;
            }
            if let (Some(p), Some(e)) = (t.playing, engine) {
                e.stop_world(p.voice);
            }
            false
        });
        // interface sounds whose clip arrived
        let mut ui_play = Vec::new();
        self.ui_pending.retain(|(id, at)| {
            if self.clips.get(id).is_some_and(|c| c.clip.is_some()) {
                ui_play.push(*id);
                return false;
            }
            now.duration_since(*at) < Duration::from_secs(3)
        });
        if let Some(e) = engine {
            for id in ui_play {
                if let Some(c) = self.ready(&id) {
                    e.play_clip(Channel::Ui, &c, 1.0);
                }
            }
        }

        // positions; objects gone stop their sound
        let pos_of = |id: &Uuid| {
            let idx = world.objects.index_of_uuid(id)?;
            let (pos, _, hud) = Scene::object_transform(world, idx, now, 0)?;
            Some((idx, pos, hud))
        };
        let mut gone = Vec::new();
        // Candidates: (distance, kind, key, audio position, within cut-off,
        // played at the listener).
        let mut active: Vec<(f32, usize, Uuid, Vec3, bool, bool)> = Vec::new();
        for (id, s) in self.sources.iter_mut() {
            let Some((idx, pos, hud)) = pos_of(id) else {
                gone.push(*id);
                continue;
            };
            // one-shot ended: the queued one next
            if let Some(p) = s.playing
                && p.length.is_some_and(|len| now.duration_since(p.started) >= len)
            {
                s.playing = None;
                if let Some(q) = s.queued.take() {
                    s.sound = q;
                    s.pending = Some(now);
                }
            }
            if s.pending
                .is_some_and(|t| s.flags & FLAG_LOOP == 0 && now.duration_since(t) > ONE_SHOT_PATIENCE)
            {
                s.pending = None;
            }
            if s.playing.is_none() && s.pending.is_none() {
                continue;
            }
            // getPosGlobal uses the wearer for every attachment, including
            // linked HUD prims. Parcel isolation follows that world position,
            // never the HUD's screen position or the camera.
            let origin = Scene::wearer_avatar(world, idx)
                .and_then(|(_, wearer)| Scene::object_transform(world, wearer, now, 0))
                .map_or(pos, |t| t.0);
            if !world.can_hear_sound(origin) && !same_seat(world, idx) {
                if let (Some(p), Some(e)) = (s.playing.take(), engine) {
                    e.stop_world(p.voice);
                }
                if s.flags & FLAG_LOOP != 0 {
                    s.pending = Some(now);
                } else {
                    s.pending = None;
                    s.queued = None;
                }
                continue;
            }
            // LLAudioSourceVO::update places HUD audio at the camera, not
            // in HUD coordinates. Its cut-off still uses the wearer's world
            // position (getPosGlobal / updateMute, llaudiosourcevo.cpp).
            let within_radius = self.radius.get(id).is_none_or(|radius| origin.distance(l.pos) < *radius);
            let pos = if hud { l.pos } else { pos };
            active.push((pos.distance(l.pos), 0, *id, pos, within_radius, hud));
        }
        for id in gone {
            if let Some(s) = self.sources.remove(&id)
                && let (Some(p), Some(e)) = (s.playing, engine)
            {
                e.stop_world(p.voice);
            }
        }
        // Recheck delayed and playing triggers as well as their admission:
        // crossing a parcel border must silence an already accepted sound.
        self.triggered.retain_mut(|t| {
            if let Some(f) = t.follow
                && let Some((_, p, false)) = pos_of(&f)
            {
                t.pos = p;
            }
            if world.can_hear_sound(t.pos) {
                return true;
            }
            if let (Some(p), Some(e)) = (t.playing, engine) {
                e.stop_world(p.voice);
            }
            false
        });
        for (i, t) in self.triggered.iter().enumerate() {
            active.push((t.pos.distance(l.pos), 1, Uuid::from_u128(i as u128), t.pos, true, false));
        }
        // the nearest within their cut-off radius play
        active.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut voices = 0usize;
        self.audible.clear();
        for (_, kind, key, pos, within_radius, at_listener) in active {
            let (sound, gain, looped, playing, emitter) = if kind == 0 {
                let Some(s) = self.sources.get(&key) else {
                    continue;
                };
                (s.sound, s.gain, s.flags & FLAG_LOOP != 0, s.playing, key)
            } else {
                let Some(t) = self.triggered.get(key.as_u128() as usize) else {
                    continue;
                };
                (
                    t.sound,
                    t.gain,
                    false,
                    t.playing,
                    if t.source.is_nil() {
                        t.follow.unwrap_or_default()
                    } else {
                        t.source
                    },
                )
            };
            let audible = within_radius && voices < MAX_VOICES;
            let place = if audible {
                Self::placement(l, pos, gain, at_listener)
            } else {
                Placement::Panned((0.0, 0.0))
            };
            match playing {
                Some(mut p) => {
                    if !audible && looped {
                        // out of reach: a loop stops and starts again later
                        if let Some(e) = engine {
                            e.stop_world(p.voice);
                        }
                        self.set_playing(kind, key, None, true);
                        continue;
                    }
                    voices += 1;
                    if !at_listener {
                        self.audible.push(Audible {
                            voice: p.voice,
                            pos,
                            emitter,
                        });
                    }
                    let resend = match (place, p.direction) {
                        (Placement::Panned(g), None) => (p.gain.0 - g.0).abs() > 0.004 || (p.gain.1 - g.1).abs() > 0.004,
                        (Placement::Binaural(g, d), Some(sent)) => (p.gain.0 - g).abs() > 0.004 || d.dot(sent) < DIRECTION_RESEND_COS,
                        // switched between panned and binaural
                        _ => true,
                    };
                    if resend {
                        if let Some(e) = engine {
                            match place {
                                Placement::Panned(g) => e.world_gain(p.voice, g),
                                Placement::Binaural(g, d) => e.world_binaural(p.voice, g, d.to_array()),
                            }
                        }
                        p.gain = place.gains();
                        p.direction = match place {
                            Placement::Binaural(_, d) => Some(d),
                            Placement::Panned(_) => None,
                        };
                        self.set_playing(kind, key, Some(p), false);
                    }
                }
                None if audible => {
                    let Some(clip) = self.ready(&sound) else {
                        if self.failed(&sound) {
                            self.set_playing(kind, key, None, false);
                        }
                        continue;
                    };
                    voices += 1;
                    let voice = self.next_voice;
                    self.next_voice += 1;
                    if let Some(e) = engine {
                        match place {
                            Placement::Panned(g) => e.play_world(voice, Channel::Sfx, &clip, looped, g),
                            Placement::Binaural(g, d) => e.play_world_binaural(voice, Channel::Sfx, &clip, looped, g, d.to_array()),
                        }
                    }
                    if !at_listener {
                        self.audible.push(Audible { voice, pos, emitter });
                    }
                    let p = Playing {
                        voice,
                        started: now,
                        length: (!looped).then(|| clip.duration()),
                        gain: place.gains(),
                        direction: match place {
                            Placement::Binaural(_, d) => Some(d),
                            Placement::Panned(_) => None,
                        },
                    };
                    self.set_playing(kind, key, Some(p), false);
                }
                None => {}
            }
        }
        // triggered sounds: done once played, or too late
        self.triggered.retain(|t| match t.playing {
            Some(p) => p.length.is_none_or(|len| now.duration_since(p.started) < len),
            None => now.duration_since(t.asked) < ONE_SHOT_PATIENCE,
        });
        let (x, y) = world.main_origin().unwrap_or_default();
        let origin = glam::DVec3::new(f64::from(x), f64::from(y), 0.0);
        let attached = self.sources.iter().filter_map(|(id, s)| {
            let p = s.playing?;
            let (idx, mut pos, hud) = pos_of(id)?;
            if hud
                && let Some((_, wearer)) = Scene::wearer_avatar(world, idx)
                && let Some(t) = Scene::object_transform(world, wearer, now, 0)
            {
                pos = t.0;
            }
            let owner = if s.owner.is_nil() {
                world.objects.get(idx)?.owner_id
            } else {
                s.owner
            };
            Some(history::Entry {
                id: p.voice,
                asset: s.sound,
                source: *id,
                owner,
                position: origin + pos.as_dvec3(),
                started: p.started,
                stopped: None,
                triggered: false,
                looped: s.flags & FLAG_LOOP != 0,
            })
        });
        let triggered = self.triggered.iter().filter_map(|t| {
            let p = t.playing?;
            Some(history::Entry {
                id: p.voice,
                asset: t.sound,
                source: t.source,
                owner: t.owner,
                position: origin + t.pos.as_dvec3(),
                started: p.started,
                stopped: None,
                triggered: true,
                looped: false,
            })
        });
        self.history.sync(attached.chain(triggered), now);
    }

    pub fn set_owner(&mut self, object: Uuid, owner: Uuid) {
        if let Some(s) = self.sources.get_mut(&object) {
            s.owner = owner;
        }
    }

    /// Stop the selected playback, never a newer sound from the same source.
    pub fn stop_logged(&mut self, engine: Option<&AudioEngine>, ids: &[u64]) {
        for s in self.sources.values_mut() {
            if let Some(p) = s.playing.filter(|p| ids.contains(&p.voice)) {
                if let Some(e) = engine {
                    e.stop_world(p.voice);
                }
                s.playing = None;
                s.pending = None;
                s.queued = None;
            }
        }
        self.triggered.retain(|t| {
            if let Some(p) = t.playing.filter(|p| ids.contains(&p.voice)) {
                if let Some(e) = engine {
                    e.stop_world(p.voice);
                }
                return false;
            }
            true
        });
        for id in ids {
            self.history.stop(*id, Instant::now());
        }
    }

    pub fn preview(&mut self, assets: impl Iterator<Item = Uuid>) {
        for asset in assets {
            if self.previews.len() >= MAX_VOICES {
                break;
            }
            self.want(asset);
            self.previews.entry(asset).or_insert((Instant::now(), None));
        }
    }

    pub fn previewing(&self) -> bool {
        !self.previews.is_empty()
    }

    pub fn stop_previews(&mut self, engine: Option<&AudioEngine>) {
        for (_, (_, p)) in self.previews.drain() {
            if let (Some(p), Some(e)) = (p, engine) {
                e.stop_world(p.voice);
            }
        }
    }

    fn update_previews(&mut self, engine: Option<&AudioEngine>, world: &World, now: Instant) {
        self.previews.retain(|asset, (asked, playing)| {
            if world.derender.sound_blocked(*asset, Uuid::nil(), 0) {
                if let (Some(p), Some(e)) = (*playing, engine) {
                    e.stop_world(p.voice);
                }
                return false;
            }
            if let Some(p) = playing {
                return p.length.is_some_and(|len| now.saturating_duration_since(p.started) < len);
            }
            if now.saturating_duration_since(*asked) > ONE_SHOT_PATIENCE {
                return false;
            }
            if let Some(c) = self.clips.get(asset).and_then(|c| c.clip.as_ref()) {
                let voice = self.next_voice;
                self.next_voice += 1;
                if let Some(e) = engine {
                    e.play_world(voice, Channel::Ui, c, false, (1.0, 1.0));
                }
                *playing = Some(Playing {
                    voice,
                    started: now,
                    length: Some(c.duration()),
                    gain: (1.0, 1.0),
                    direction: None,
                });
            }
            true
        });
    }

    /// Record what a source / triggered sound plays. `keep_pending`: a loop
    /// out of reach waits to start again.
    fn set_playing(&mut self, kind: usize, key: Uuid, p: Option<Playing>, keep_pending: bool) {
        if kind == 0 {
            if let Some(s) = self.sources.get_mut(&key) {
                let started = p.is_some() && s.playing.is_none();
                s.playing = p;
                if started {
                    s.pending = None;
                } else if p.is_none() && keep_pending {
                    s.pending = Some(Instant::now());
                } else if p.is_none() && !keep_pending {
                    s.pending = None;
                }
            }
        } else if let Some(t) = self.triggered.get_mut(key.as_u128() as usize) {
            t.playing = p;
            if p.is_none() && !keep_pending {
                // failed sound: let it expire
                t.asked = Instant::now() - ONE_SHOT_PATIENCE;
            }
        }
    }

    /// Object sound changes carried by object updates.
    pub fn apply_object_sound(&mut self, engine: Option<&AudioEngine>, object: Uuid, sound: AttachedSound) {
        self.set_radius(object, sound.radius);
        self.set_attached(engine, object, sound.id, sound.gain, sound.flags);
    }

    /// The world sounds playing at a place, nearest first, as the last
    /// update left them.
    pub fn audible(&self) -> &[Audible] {
        &self.audible
    }

    /// Playing voices (performance panel).
    pub fn voices(&self) -> usize {
        self.sources.values().filter(|s| s.playing.is_some()).count() + self.triggered.iter().filter(|t| t.playing.is_some()).count()
    }
}

/// LLAudioSourceVO::updateMute keeps the vehicle we ride audible when its
/// root has crossed a parcel/region border ahead of the seated avatar.
fn same_seat(world: &World, source: usize) -> bool {
    let Some(agent) = world.objects.index_of_uuid(&world.agent_id) else {
        return false;
    };
    if world.objects.get(agent).is_none_or(|o| o.parent_id == 0) {
        return false;
    }
    let root = |mut idx| {
        for _ in 0..64 {
            let object = world.objects.get(idx)?;
            if object.parent_id == 0 {
                return Some(idx);
            }
            idx = world.objects.parent_of(object)?;
        }
        None
    };
    root(source).zip(root(agent)).is_some_and(|(a, b)| a == b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sound_explorer_logs_actual_playback_and_stops_only_that_voice() {
        let world = hud_world();
        let object = object_id(&world, crate::demo::hud::CHILD);
        let asset = Uuid::from_u128(17);
        let mut sounds = SoundManager::new(std::env::temp_dir().join("aurora-sound-explorer-tests"));
        let l = Listener {
            pos: Vec3::ZERO,
            right: Vec3::X,
            forward: Vec3::Y,
            up: Vec3::Z,
            underwater: false,
            binaural: false,
        };
        let now = Instant::now();
        sounds.set_attached(None, object, asset, 1.0, FLAG_LOOP);
        sounds.update(None, &world, &l, now);
        assert_eq!(sounds.history.entries().count(), 0, "pending assets are not playing");
        sounds.insert(asset, SoundClip::from_pcm(&[0.0; 4800], 1, 48_000));
        sounds.update(None, &world, &l, now + Duration::from_millis(10));
        let first = sounds.history.entries().next().unwrap().clone();
        assert!(first.looped && first.stopped.is_none());
        sounds.set_attached(None, object, asset, 1.0, 0);
        sounds.update(None, &world, &l, now + Duration::from_millis(20));
        let second = sounds.sources[&object].playing.unwrap().voice;
        assert_ne!(first.id, second);
        sounds.stop_logged(None, &[first.id]);
        assert_eq!(
            sounds.sources[&object].playing.unwrap().voice,
            second,
            "a frozen row cannot stop its replacement"
        );
        sounds.stop_logged(None, &[second]);
        sounds.update(None, &world, &l, now + Duration::from_millis(30));
        assert!(sounds.sources[&object].playing.is_none());
        assert!(sounds.history.entries().all(|e| e.stopped.is_some()));
        sounds.preview([asset, asset].into_iter());
        sounds.update(None, &world, &l, now + Duration::from_millis(40));
        assert_eq!(sounds.previews.len(), 1, "preview unique assets only");
        assert_eq!(sounds.history.entries().count(), 2, "previews never enter the world log");
        sounds.stop_previews(None);
        assert!(!sounds.previewing());
    }

    #[test]
    fn sound_explorer_blacklisting_stops_current_and_pending_sources_and_previews() {
        use crate::world::derender::{Entry, Kind};
        let mut world = hud_world();
        let object = object_id(&world, crate::demo::hud::CHILD);
        let asset = Uuid::from_u128(21);
        let mut sounds = SoundManager::new(std::env::temp_dir().join("aurora-sound-explorer-tests"));
        let l = Listener {
            pos: Vec3::ZERO,
            right: Vec3::X,
            forward: Vec3::Y,
            up: Vec3::Z,
            underwater: false,
            binaural: false,
        };
        let now = Instant::now();
        sounds.insert(asset, SoundClip::from_pcm(&[0.0; 4800], 1, 48_000));
        sounds.set_attached(None, object, asset, 1.0, FLAG_LOOP);
        sounds.preview(std::iter::once(asset));
        sounds.trigger_owned(
            asset,
            Vec3::ZERO,
            None,
            1.0,
            Origin {
                object: world.agent_id,
                owner: world.agent_id,
            },
        );
        sounds.update(None, &world, &l, now);
        assert_eq!(sounds.voices(), 2);
        world.derender.add_sound(
            asset,
            Entry {
                name: String::new(),
                region: String::new(),
                kind: Kind::Sound(0),
                date: 0,
                permanent: false,
            },
        );
        sounds.update(None, &world, &l, now + Duration::from_millis(10));
        assert_eq!(sounds.voices(), 0);
        assert!(!sounds.previewing());
        assert!(sounds.history.entries().all(|e| e.stopped.is_some()));
        sounds.set_attached(None, object, asset, 1.0, FLAG_LOOP);
        sounds.update(None, &world, &l, now + Duration::from_millis(20));
        assert_eq!(sounds.voices(), 0, "a blocked pending source cannot restart");
        world.derender.remove(&[asset]);
        sounds.set_attached(None, object, asset, 1.0, FLAG_LOOP);
        sounds.update(None, &world, &l, now + Duration::from_millis(30));
        assert_eq!(sounds.voices(), 1, "removal permits future playback");
    }

    #[test]
    fn directly_muted_object_stops_without_muting_unrelated_objects_of_the_same_owner() {
        use crate::world::mutes::{Mute, MuteType};
        let mut world = hud_world();
        let child = object_id(&world, crate::demo::hud::CHILD);
        let owner = world.agent_id;
        let other = Uuid::from_u128(555);
        let asset = Uuid::from_u128(21);
        world.mutes.reset(owner);
        let mut sounds = SoundManager::new(std::env::temp_dir().join("aurora-direct-sound-tests"));
        sounds.insert(asset, SoundClip::from_pcm(&[0.0; 4800], 1, 48_000));
        sounds.set_attached(None, child, asset, 1.0, FLAG_LOOP);
        sounds.set_owner(child, owner);
        sounds.trigger_owned(asset, Vec3::ZERO, None, 1.0, Origin { object: other, owner });
        let listener = Listener {
            pos: Vec3::ZERO,
            right: Vec3::X,
            forward: Vec3::Y,
            up: Vec3::Z,
            underwater: false,
            binaural: false,
        };
        let now = Instant::now();
        sounds.update(None, &world, &listener, now);
        assert_eq!(sounds.voices(), 2);
        world
            .mutes
            .add(
                Mute {
                    id: child,
                    name: "Object".into(),
                    kind: MuteType::Object,
                    flags: 0,
                },
                0,
            )
            .unwrap();
        sounds.update(None, &world, &listener, now + Duration::from_millis(10));
        assert_eq!(sounds.voices(), 1);
        assert!(sounds.history.entries().filter(|e| e.stopped.is_none()).all(|e| e.source == other));
    }

    fn hud_world() -> World {
        let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events().into_iter().chain(crate::demo::hud::events()) {
            world.apply(event);
        }
        world
    }

    fn object_id(world: &World, local_id: u32) -> Uuid {
        world.objects.iter().find(|(_, o)| o.key.local_id == local_id).unwrap().1.full_id
    }

    fn assert_gain(playing: Option<Playing>, gain: (f32, f32)) {
        let actual = playing.expect("sound is playing").gain;
        assert!(
            (actual.0 - gain.0).abs() < 1e-5 && (actual.1 - gain.1).abs() < 1e-5,
            "{actual:?} != {gain:?}"
        );
    }

    #[test]
    fn hud_sounds_play_at_the_listener_on_roots_and_linked_prims() {
        let world = hud_world();
        let mut sounds = SoundManager::new(std::env::temp_dir().join("aurora-hud-sound-tests"));
        let sound = Uuid::from_u128(1);
        sounds.insert(sound, SoundClip::from_pcm(&[0.0; 4800], 1, 48_000));
        let mut ids = Vec::new();
        for point in 31..=38 {
            let id = object_id(&world, 8200 + point);
            sounds.set_attached(None, id, sound, 0.6, if point % 2 == 0 { FLAG_LOOP } else { 0 });
            ids.push(id);
        }
        let child = object_id(&world, crate::demo::hud::CHILD);
        sounds.set_attached(None, child, sound, 0.4, 0);
        let mut listener = Listener {
            pos: Vec3::new(100.0, 200.0, 300.0),
            right: Vec3::X,
            forward: Vec3::Y,
            up: Vec3::Z,
            underwater: false,
            binaural: false,
        };
        let now = Instant::now();
        sounds.update(None, &world, &listener, now);
        assert_eq!(sounds.voices(), 9);
        for id in &ids {
            assert_gain(sounds.sources[id].playing, (0.6, 0.6));
        }
        assert_gain(sounds.sources[&child].playing, (0.4, 0.4));
        listener.pos += Vec3::splat(1000.0);
        listener.right = Vec3::Y;
        listener.underwater = true;
        sounds.update(None, &world, &listener, now + Duration::from_millis(10));
        for id in &ids {
            assert_gain(sounds.sources[id].playing, (0.6, 0.6));
        }
        assert_gain(sounds.sources[&child].playing, (0.4, 0.4));
        assert!(sounds.beacon_sources(&world, now).is_empty());
    }

    #[test]
    fn hud_sounds_wait_for_assets_then_honor_queue_gain_stop_and_detach() {
        let mut world = hud_world();
        let child = object_id(&world, crate::demo::hud::CHILD);
        let root = object_id(&world, crate::demo::hud::FIRST);
        let first = Uuid::from_u128(1);
        let second = Uuid::from_u128(2);
        let mut sounds = SoundManager::new(std::env::temp_dir().join("aurora-hud-sound-tests"));
        let listener = Listener {
            pos: Vec3::splat(100.0),
            right: Vec3::X,
            forward: Vec3::Y,
            up: Vec3::Z,
            underwater: false,
            binaural: false,
        };
        sounds.set_attached(None, child, first, 0.8, 0);
        let now = Instant::now();
        sounds.update(None, &world, &listener, now);
        assert!(sounds.sources.get(&child).is_some_and(|s| s.pending.is_some()));
        for sound in [first, second] {
            sounds.insert(sound, SoundClip::from_pcm(&[0.0; 4800], 1, 48_000));
        }
        sounds.update(None, &world, &listener, now + Duration::from_millis(10));
        let voice = sounds.sources[&child].playing.expect("loaded HUD sound").voice;
        sounds.set_attached(None, child, second, 0.5, FLAG_QUEUE);
        sounds.set_gain(child, 0.3);
        sounds.update(None, &world, &listener, now + Duration::from_millis(20));
        assert_eq!(sounds.sources[&child].playing.unwrap().voice, voice);
        assert_gain(sounds.sources[&child].playing, (0.3, 0.3));
        sounds.update(None, &world, &listener, now + Duration::from_millis(120));
        assert_eq!(sounds.sources[&child].sound, second);
        assert_ne!(sounds.sources[&child].playing.unwrap().voice, voice);
        sounds.set_attached(None, child, Uuid::nil(), 0.0, FLAG_STOP);
        sounds.set_attached(None, root, first, 0.7, FLAG_LOOP);
        sounds.update(None, &world, &listener, now + Duration::from_millis(130));
        assert!(sounds.sources[&child].playing.is_none());
        assert_gain(sounds.sources[&root].playing, (0.7, 0.7));
        sounds.set_attached(None, root, Uuid::nil(), 0.0, FLAG_STOP);
        assert!(!sounds.sources.contains_key(&root));
        sounds.set_attached(None, root, first, 0.7, FLAG_LOOP);
        sounds.update(None, &world, &listener, now + Duration::from_millis(140));
        world.objects.remove_idx(world.objects.index_of_uuid(&root).unwrap());
        sounds.update(None, &world, &listener, now + Duration::from_millis(150));
        assert_eq!(sounds.voices(), 0);
        assert!(!sounds.sources.contains_key(&root));
        assert!(!sounds.sources.contains_key(&child));
    }

    #[test]
    fn hud_sounds_honor_cutoff_at_the_wearer() {
        let world = hud_world();
        let child = object_id(&world, crate::demo::hud::CHILD);
        let now = Instant::now();
        let avatar = world.objects.index_of_uuid(&world.agent_id).unwrap();
        let wearer_pos = Scene::object_transform(&world, avatar, now, 0).unwrap().0;
        let mut sounds = SoundManager::new(std::env::temp_dir().join("aurora-hud-sound-tests"));
        let sound = Uuid::from_u128(1);
        sounds.insert(sound, SoundClip::from_pcm(&[0.0; 4800], 1, 48_000));
        sounds.apply_object_sound(
            None,
            child,
            AttachedSound {
                id: sound,
                gain: 0.6,
                flags: FLAG_LOOP,
                radius: 2.0,
            },
        );
        let mut listener = Listener {
            pos: wearer_pos + Vec3::X * 4.0,
            right: Vec3::X,
            forward: Vec3::Y,
            up: Vec3::Z,
            underwater: false,
            binaural: false,
        };
        sounds.update(None, &world, &listener, now);
        assert_eq!(sounds.voices(), 0, "HUD radius is measured from its wearer, not its audio position");
        listener.pos = wearer_pos + Vec3::X;
        sounds.update(None, &world, &listener, now + Duration::from_millis(10));
        assert_gain(sounds.sources[&child].playing, (0.6, 0.6));
        listener.pos = wearer_pos + Vec3::X * 4.0;
        sounds.update(None, &world, &listener, now + Duration::from_millis(20));
        assert_eq!(sounds.voices(), 0);
        sounds.set_radius(child, 0.0);
        sounds.update(None, &world, &listener, now + Duration::from_millis(30));
        assert_gain(sounds.sources[&child].playing, (0.6, 0.6));
    }

    #[test]
    fn hud_sounds_leave_world_and_triggered_sounds_spatial() {
        let world = hud_world();
        let object = object_id(&world, 101);
        let hud = object_id(&world, crate::demo::hud::CHILD);
        let now = Instant::now();
        let idx = world.objects.index_of_uuid(&object).unwrap();
        let pos = Scene::object_transform(&world, idx, now, 0).unwrap().0;
        let listener = Listener {
            pos: pos + Vec3::X * 4.0,
            right: Vec3::X,
            forward: Vec3::Y,
            up: Vec3::Z,
            underwater: false,
            binaural: false,
        };
        let sound = Uuid::from_u128(1);
        let mut sounds = SoundManager::new(std::env::temp_dir().join("aurora-hud-sound-tests"));
        sounds.insert(sound, SoundClip::from_pcm(&[0.0; 4800], 1, 48_000));
        sounds.set_attached(None, object, sound, 0.6, FLAG_LOOP);
        // SoundTrigger positions remain in the world, even if the emitter is a HUD.
        sounds.trigger(sound, pos, Some(hud), 0.6);
        sounds.update(None, &world, &listener, now);
        assert_gain(sounds.sources[&object].playing, SoundManager::gains(&listener, pos, 0.6));
        assert_gain(sounds.triggered[0].playing, SoundManager::gains(&listener, pos, 0.6));
        assert_eq!(sounds.triggered[0].pos, pos);
        assert!(sounds.sources[&object].playing.unwrap().gain.1 < 1e-5);
        sounds.set_radius(object, 2.0);
        sounds.update(None, &world, &listener, now + Duration::from_millis(10));
        assert!(sounds.sources[&object].playing.is_none());
        assert!(sounds.triggered[0].playing.is_some());
    }

    #[test]
    fn sound_beacons_follow_pending_sources_and_drop_stopped_sounds() {
        let lib = Arc::new(crate::scene::avatar::AvatarLibrary::load());
        let mut world = World::new(lib.clone());
        for event in crate::demo::events() {
            world.apply(event);
        }
        world.apply(crate::demo::beacons::events());
        let object = world.objects.iter().find(|(_, o)| o.key.local_id == 68203).unwrap().1.full_id;
        let mut scene = Scene::new(Default::default(), lib);
        scene.sounds.set_attached(None, object, Uuid::from_u128(1), 0.0, 0);
        assert_eq!(
            scene.sounds.beacon_sources(&world, Instant::now()).len(),
            1,
            "inaudible and waiting for its asset"
        );
        scene.sounds.set_attached(None, object, Uuid::nil(), 0.0, FLAG_STOP);
        assert!(scene.sounds.beacon_sources(&world, Instant::now()).is_empty());
        scene.sounds.trigger(Uuid::from_u128(2), Vec3::new(3.0, 4.0, 5.0), None, 1.0);
        assert_eq!(
            scene.sounds.beacon_sources(&world, Instant::now()),
            [(None, Vec3::new(3.0, 4.0, 5.0))]
        );
    }

    #[test]
    fn distance_and_panning() {
        let l = Listener {
            pos: Vec3::ZERO,
            right: Vec3::X,
            forward: Vec3::Y,
            up: Vec3::Z,
            underwater: false,
            binaural: false,
        };
        // in front, near: full volume on both sides
        let (a, b) = SoundManager::gains(&l, Vec3::new(0.0, 0.5, 0.0), 1.0);
        assert!((a - 1.0).abs() < 1e-3 && (b - 1.0).abs() < 1e-3, "{a} {b}");
        // 11 m ahead: 1 / (1 + 10)
        let (a, _) = SoundManager::gains(&l, Vec3::new(0.0, 11.0, 0.0), 1.0);
        assert!((a - 1.0 / 11.0).abs() < 1e-3, "{a}");
        // on the right: right channel only
        let (a, b) = SoundManager::gains(&l, Vec3::new(3.0, 0.0, 0.0), 1.0);
        assert!(a < 1e-3 && b > 0.3, "{a} {b}");
        // under water it fades much faster
        let w = Listener { underwater: true, ..l };
        let (a, _) = SoundManager::gains(&w, Vec3::new(0.0, 11.0, 0.0), 1.0);
        assert!((a - 1.0 / 51.0).abs() < 1e-3, "{a}");
    }

    #[test]
    fn binaural_placement_is_the_direction_in_the_listeners_frame() {
        // looking east (+x) from (10, 0, 0): north (+y) is on the left
        let off = Listener::looking(Vec3::new(10.0, 0.0, 0.0), Vec3::new(2.0, 0.0, 0.0), false, false);
        assert_eq!((off.right, off.forward, off.up), (Vec3::new(0.0, -1.0, 0.0), Vec3::X, Vec3::Z));
        let on = Listener { binaural: true, ..off };
        let pos = Vec3::new(10.0, 6.0, 8.0);
        // off: the panning, unchanged
        assert_eq!(
            SoundManager::placement(&off, pos, 0.8, false),
            Placement::Panned(SoundManager::gains(&off, pos, 0.8))
        );
        // on: the panning's distance gain, the direction (left, above)
        let Placement::Binaural(g, d) = SoundManager::placement(&on, pos, 0.8, false) else {
            panic!("binaural");
        };
        assert!((g - 0.8 / 10.0).abs() < 1e-6, "{g}");
        assert!((d - Vec3::new(-0.6, 0.0, 0.8)).length() < 1e-6, "{d}");
        // behind, below
        let Placement::Binaural(_, d) = SoundManager::placement(&on, Vec3::new(7.0, 0.0, -4.0), 1.0, false) else {
            panic!("binaural");
        };
        assert!((d - Vec3::new(0.0, -0.6, -0.8)).length() < 1e-6, "{d}");
        // HUD sounds and sounds at the head stay centred
        assert_eq!(
            SoundManager::placement(&on, pos, 1.0, true),
            Placement::Panned(SoundManager::gains(&on, pos, 1.0))
        );
        assert!(matches!(SoundManager::placement(&on, on.pos, 1.0, false), Placement::Panned(_)));
        // looking straight up or down still has a frame
        let up = Listener::looking(Vec3::ZERO, Vec3::Z, false, true);
        assert!(up.right.is_normalized() && up.forward.is_normalized() && up.up.is_normalized());
    }
}
