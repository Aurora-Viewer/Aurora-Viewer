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
use super::jobs::{JobResult, Jobs};
use crate::world::World;
use aurora_audio::{AudioEngine, Channel, SoundClip};
use aurora_net::fetch::{FetchRequest, FetchResult, Fetcher};
use aurora_net::objects::AttachedSound;
use glam::Vec3;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

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
}

/// Sound of an object (LLAudioSourceVO).
struct Source {
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
    pub underwater: bool,
}

pub struct SoundManager {
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
}

impl SoundManager {
    pub fn new(cache_dir: PathBuf) -> SoundManager {
        let _ = std::fs::create_dir_all(cache_dir.join("sound"));
        let (tx, rx) = crossbeam_channel::unbounded();
        SoundManager {
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
        let gain = if gain.is_finite() { gain.clamp(0.0, 1.0) } else { 0.0 };
        if sound.is_nil() || gain < f32::EPSILON * 2.0 || self.triggered.len() > 128 {
            return;
        }
        self.want(sound);
        self.triggered.push(Triggered {
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
                    priority: 9e11,
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

    /// (left, right) gains of a sound at `pos` for the listener.
    fn gains(l: &Listener, pos: Vec3, gain: f32) -> (f32, f32) {
        let to = pos - l.pos;
        let d = to.length();
        let rolloff = if l.underwater { UNDERWATER_ROLLOFF } else { ROLLOFF };
        let att = if d <= MIN_DISTANCE {
            1.0
        } else {
            MIN_DISTANCE / (MIN_DISTANCE + rolloff * (d - MIN_DISTANCE))
        };
        let g = gain * att;
        // constant power panning, centered at 0.707 x sqrt 2 = 1 per side
        let pan = if d > 1e-3 { (to / d).dot(l.right).clamp(-1.0, 1.0) } else { 0.0 };
        let a = (pan + 1.0) * std::f32::consts::FRAC_PI_4;
        let s = std::f32::consts::SQRT_2;
        ((a.cos() * s).min(1.0) * g, (a.sin() * s).min(1.0) * g)
    }

    /// Start, place and end the sounds (every frame).
    pub fn update(&mut self, engine: Option<&AudioEngine>, world: &World, l: &Listener, now: Instant) {
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
        let pos_of = |id: &Uuid| -> Option<Vec3> {
            let idx = world.objects.index_of_uuid(id)?;
            Scene::object_transform(world, idx, now, 0).filter(|t| !t.2).map(|t| t.0)
        };
        let mut gone = Vec::new();
        // candidates: (distance, is_source, key)
        let mut active: Vec<(f32, usize, Uuid, Vec3)> = Vec::new();
        for (id, s) in self.sources.iter_mut() {
            let Some(pos) = pos_of(id) else {
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
            active.push((pos.distance(l.pos), 0, *id, pos));
        }
        for id in gone {
            if let Some(s) = self.sources.remove(&id)
                && let (Some(p), Some(e)) = (s.playing, engine)
            {
                e.stop_world(p.voice);
            }
        }
        for (i, t) in self.triggered.iter_mut().enumerate() {
            if let Some(f) = t.follow
                && let Some(p) = pos_of(&f)
            {
                t.pos = p;
            }
            active.push((t.pos.distance(l.pos), 1, Uuid::from_u128(i as u128), t.pos));
        }
        // the nearest within their cut-off radius play
        active.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut voices = 0usize;
        for (d, kind, key, pos) in active {
            let (sound, gain, looped, playing, cutoff) = if kind == 0 {
                let Some(s) = self.sources.get(&key) else {
                    continue;
                };
                (s.sound, s.gain, s.flags & FLAG_LOOP != 0, s.playing, self.radius.get(&key).copied())
            } else {
                let Some(t) = self.triggered.get(key.as_u128() as usize) else {
                    continue;
                };
                (t.sound, t.gain, false, t.playing, None)
            };
            let audible = cutoff.is_none_or(|r| d < r) && voices < MAX_VOICES;
            let g = if audible { Self::gains(l, pos, gain) } else { (0.0, 0.0) };
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
                    if (p.gain.0 - g.0).abs() > 0.004 || (p.gain.1 - g.1).abs() > 0.004 {
                        if let Some(e) = engine {
                            e.world_gain(p.voice, g);
                        }
                        p.gain = g;
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
                        e.play_world(voice, Channel::Sfx, &clip, looped, g);
                    }
                    let p = Playing {
                        voice,
                        started: now,
                        length: (!looped).then(|| clip.duration()),
                        gain: g,
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

    /// Playing voices (performance panel).
    pub fn voices(&self) -> usize {
        self.sources.values().filter(|s| s.playing.is_some()).count() + self.triggered.iter().filter(|t| t.playing.is_some()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_and_panning() {
        let l = Listener {
            pos: Vec3::ZERO,
            right: Vec3::X,
            underwater: false,
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
}
