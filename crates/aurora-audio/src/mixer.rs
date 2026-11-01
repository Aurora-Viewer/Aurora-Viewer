//! Volume channels and the real-time mixer.
//!
//! The channel model (master gain multiplied by a per-type secondary gain,
//! each forced to zero when muted) follows `audio_update_volume()` in
//! Firestorm's `indra/newview/llvieweraudio.cpp`.
//!
//! Portions derived from the Second Life / Firestorm viewer source code,
//! Copyright (C) Linden Research, Inc. and The Phoenix Firestorm Project,
//! licensed under the GNU Lesser General Public License, version 2.1.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crossbeam_channel::Receiver;

use crate::wind::{SharedWind, WindGen};

/// Internal mixing rate. Every source is converted to this rate (stereo,
/// interleaved f32) before it reaches the mixer; the output stage resamples
/// to the device rate when it differs.
pub const ENGINE_SAMPLE_RATE: u32 = 48_000;
/// The mixer works in interleaved stereo.
pub(crate) const ENGINE_CHANNELS: usize = 2;

/// Volume channels, matching the rows of Firestorm's audio volume floater.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Channel {
    /// "Main" (master). Scales every other channel.
    Master,
    /// "Interface" (UI sounds).
    Ui,
    /// "Ambient" (ambient / wind / looped world sounds).
    Ambient,
    /// "Sounds" (sound effects from objects).
    Sfx,
    /// "Music" (parcel music stream).
    Music,
    /// "Multimedia" (media-on-a-prim / parcel media audio).
    Media,
    /// "Voice" (voice chat).
    Voice,
}

impl Channel {
    /// All channels, master first.
    pub const ALL: [Channel; 7] = [
        Channel::Master,
        Channel::Ui,
        Channel::Ambient,
        Channel::Sfx,
        Channel::Music,
        Channel::Media,
        Channel::Voice,
    ];

    fn index(self) -> usize {
        self as usize
    }

    /// Bus index (everything except `Master`, which is not a bus).
    fn bus(self) -> Option<usize> {
        match self {
            Channel::Master => None,
            other => Some(other.index() - 1),
        }
    }
}

/// Number of mixing buses (every channel except master).
const NUM_BUSES: usize = 6;
const BUS_CHANNELS: [Channel; NUM_BUSES] = [
    Channel::Ui,
    Channel::Ambient,
    Channel::Sfx,
    Channel::Music,
    Channel::Media,
    Channel::Voice,
];

/// User settings of one volume channel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelSettings {
    /// Linear volume, 0.0 ..= 1.0.
    pub volume: f32,
    /// Mute button.
    pub muted: bool,
    /// "Enabled" checkbox. A disabled channel is silent; a disabled
    /// `Music` channel additionally stops the network stream.
    pub enabled: bool,
}

impl Default for ChannelSettings {
    fn default() -> Self {
        Self {
            volume: 1.0,
            muted: false,
            enabled: true,
        }
    }
}

impl ChannelSettings {
    /// Gain contributed by this channel alone (0 when muted or disabled).
    pub fn effective_gain(&self) -> f32 {
        if self.muted || !self.enabled {
            0.0
        } else {
            sanitize_volume(self.volume)
        }
    }
}

pub(crate) fn sanitize_volume(v: f32) -> f32 {
    if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }
}

/// Final linear gain of a bus: master * channel, as in
/// `music_volume = mute_volume * master_volume * AudioLevelMusic`.
pub(crate) fn bus_gain(master: ChannelSettings, channel: ChannelSettings) -> f32 {
    master.effective_gain() * channel.effective_gain()
}

struct ChannelSlot {
    volume: AtomicU32,
    muted: AtomicBool,
    enabled: AtomicBool,
}

impl ChannelSlot {
    fn new() -> Self {
        Self {
            volume: AtomicU32::new(1.0f32.to_bits()),
            muted: AtomicBool::new(false),
            enabled: AtomicBool::new(true),
        }
    }
}

/// Channel settings and wind targets shared between the API thread(s) and
/// the audio callback.
pub(crate) struct SharedParams {
    slots: [ChannelSlot; 7],
    pub(crate) wind: SharedWind,
}

impl SharedParams {
    pub(crate) fn new() -> Self {
        Self {
            slots: std::array::from_fn(|_| ChannelSlot::new()),
            wind: SharedWind::new(),
        }
    }

    pub(crate) fn set_volume(&self, ch: Channel, v: f32) {
        self.slots[ch.index()].volume.store(sanitize_volume(v).to_bits(), Ordering::Relaxed);
    }

    pub(crate) fn set_muted(&self, ch: Channel, m: bool) {
        self.slots[ch.index()].muted.store(m, Ordering::Relaxed);
    }

    pub(crate) fn set_enabled(&self, ch: Channel, e: bool) {
        self.slots[ch.index()].enabled.store(e, Ordering::Relaxed);
    }

    pub(crate) fn get(&self, ch: Channel) -> ChannelSettings {
        let s = &self.slots[ch.index()];
        ChannelSettings {
            volume: f32::from_bits(s.volume.load(Ordering::Relaxed)),
            muted: s.muted.load(Ordering::Relaxed),
            enabled: s.enabled.load(Ordering::Relaxed),
        }
    }

    fn bus_target(&self, bus: Channel) -> f32 {
        bus_gain(self.get(Channel::Master), self.get(bus))
    }
}

/// Time constant of the gain smoother. ~15 ms removes zipper noise and clicks
/// while still feeling instantaneous on a slider.
const GAIN_SMOOTHING_SECS: f32 = 0.015;
/// Fade length used when a source starts, restarts after an underrun, or is
/// stopped (10 ms at 48 kHz).
const SOURCE_FADE_FRAMES: f32 = 480.0;

/// One-pole low-pass on the gain value, evaluated per frame.
#[derive(Debug, Clone)]
pub(crate) struct GainSmoother {
    current: f32,
    coef: f32,
}

impl GainSmoother {
    pub(crate) fn new(initial: f32, tau_secs: f32, rate: u32) -> Self {
        let coef = 1.0 - (-1.0 / (tau_secs * rate as f32)).exp();
        Self { current: initial, coef }
    }

    #[inline]
    pub(crate) fn next(&mut self, target: f32) -> f32 {
        let diff = target - self.current;
        // Snap the last -80 dB: f32 cannot represent the tail of the ramp.
        if diff.abs() < 1.0e-4 {
            self.current = target;
        } else {
            self.current += diff * self.coef;
        }
        self.current
    }

    #[cfg(test)]
    pub(crate) fn value(&self) -> f32 {
        self.current
    }
}

/// A queue of interleaved stereo PCM at [`ENGINE_SAMPLE_RATE`], fed by a
/// producer thread (music decoder, voice, media) and drained by the mixer.
///
/// Implements simple jitter buffering: playback starts (or resumes after an
/// underrun) only once `prebuffer` samples are queued, and if more than
/// `max_latency` samples pile up the oldest ones are dropped.
pub(crate) struct PcmSource {
    bus: usize,
    cons: rtrb::Consumer<f32>,
    playing: bool,
    prebuffer: usize,
    max_latency: Option<usize>,
    kill: Option<Arc<AtomicBool>>,
    fade: f32,
}

impl PcmSource {
    /// `prebuffer` / `max_latency` are in frames.
    pub(crate) fn new(
        channel: Channel,
        cons: rtrb::Consumer<f32>,
        prebuffer_frames: usize,
        max_latency_frames: Option<usize>,
        kill: Option<Arc<AtomicBool>>,
    ) -> Self {
        Self {
            // Master is not a bus; route stray master sources to UI.
            bus: channel.bus().unwrap_or(0),
            cons,
            playing: false,
            prebuffer: prebuffer_frames * ENGINE_CHANNELS,
            max_latency: max_latency_frames.map(|f| f * ENGINE_CHANNELS),
            kill,
            fade: 0.0,
        }
    }

    /// Mixes into `bus`. Returns `false` when the source is finished and can
    /// be removed.
    fn mix_into(&mut self, bus: &mut [f32], tmp: &mut Vec<f32>) -> bool {
        let killed = self.kill.as_ref().is_some_and(|k| k.load(Ordering::Relaxed));
        let abandoned = self.cons.is_abandoned();
        let want = bus.len();

        if killed {
            // Fade out what we have this block, then disappear.
            if self.playing {
                let n = self.read(want, tmp);
                let step = 1.0 / SOURCE_FADE_FRAMES;
                let frames = tmp[..n].as_chunks::<ENGINE_CHANNELS>().0;
                for (frame, out) in frames.iter().zip(bus.as_chunks_mut::<ENGINE_CHANNELS>().0) {
                    self.fade = (self.fade - step).max(0.0);
                    for (o, s) in out.iter_mut().zip(frame) {
                        *o += s * self.fade;
                    }
                }
            }
            return false;
        }

        let mut avail = self.cons.slots() & !1;
        if !self.playing {
            if avail >= self.prebuffer.max(ENGINE_CHANNELS) || (abandoned && avail > 0) {
                self.playing = true;
                self.fade = 0.0;
            } else {
                return !(abandoned && avail == 0);
            }
        }

        if let Some(max) = self.max_latency
            && avail > max
        {
            let drop = (avail - self.prebuffer) & !1;
            if let Ok(chunk) = self.cons.read_chunk(drop) {
                chunk.commit_all();
            }
            avail -= drop;
        }

        let n = self.read(want.min(avail), tmp);
        let step = 1.0 / SOURCE_FADE_FRAMES;
        let frames = tmp[..n].as_chunks::<ENGINE_CHANNELS>().0;
        for (frame, out) in frames.iter().zip(bus.as_chunks_mut::<ENGINE_CHANNELS>().0) {
            self.fade = (self.fade + step).min(1.0);
            for (o, s) in out.iter_mut().zip(frame) {
                *o += s * self.fade;
            }
        }
        if n < want {
            // Underrun: wait for the jitter buffer to refill.
            self.playing = false;
        }
        !(abandoned && self.cons.is_empty())
    }

    #[cfg(test)]
    pub(crate) fn queued_samples(&self) -> usize {
        self.cons.slots()
    }

    fn read(&mut self, n: usize, tmp: &mut Vec<f32>) -> usize {
        if tmp.len() < n {
            tmp.resize(n, 0.0);
        }
        let (got, _) = self.cons.pop_partial_slice(&mut tmp[..n]);
        got.len() & !1
    }
}

/// A one-shot in-memory sound (see [`crate::SoundClip`]).
pub(crate) struct ClipVoice {
    bus: usize,
    data: Arc<[f32]>,
    pos: usize,
    gain: f32,
}

impl ClipVoice {
    pub(crate) fn new(channel: Channel, data: Arc<[f32]>, gain: f32) -> Self {
        Self {
            bus: channel.bus().unwrap_or(0),
            data,
            pos: 0,
            gain: sanitize_volume(gain),
        }
    }

    fn mix_into(&mut self, bus: &mut [f32]) -> bool {
        let remaining = &self.data[self.pos.min(self.data.len())..];
        let n = remaining.len().min(bus.len());
        for (o, s) in bus[..n].iter_mut().zip(&remaining[..n]) {
            *o += s * self.gain;
        }
        self.pos += n;
        self.pos < self.data.len()
    }
}

/// Occlusion of a world voice by the time a new value is reached: the
/// filter and the gain glide there with this time constant, so that walking
/// past a doorway opens the sound up over about a tenth of a second. The
/// first value a voice receives before it has played anything applies at
/// once: a sound starting behind a wall starts muffled. The first value of
/// a voice already playing, measured once it started, glides there faster.
const OCCLUSION_SMOOTHING_SECS: f32 = 0.08;
const OCCLUSION_FIRST_SECS: f32 = 0.015;
/// Cut-off of the low-pass for one blocking layer (occlusion 0.5) and for
/// two or more (occlusion 1).
const OCCLUSION_CUTOFF_ONE: f32 = 1_000.0;
const OCCLUSION_CUTOFF_FULL: f32 = 600.0;
/// Where the cut-off starts from: above what the ear hears, so that a slight
/// occlusion (one ray of three grazing an edge) is a slight dulling.
const OCCLUSION_CUTOFF_OPEN: f32 = 22_000.0;
/// Gain cut at occlusion 1 (-6 dB a layer).
const OCCLUSION_GAIN_DB: f32 = -12.0;
/// Below this occlusion the filter fades into a plain pass-through.
const OCCLUSION_FADE_IN: f32 = 0.05;
/// Filter states this small are zero: no denormals through a long silence.
const DENORMAL: f32 = 1.0e-20;

/// Low-pass cut-off for an occlusion amount (0 clear, 0.5 one layer, 1 two
/// or more), interpolated on a log scale.
pub(crate) fn occlusion_cutoff(amount: f32) -> f32 {
    let a = amount.clamp(0.0, 1.0);
    let lerp_log = |from: f32, to: f32, t: f32| (from.ln() + (to.ln() - from.ln()) * t).exp();
    if a <= 0.5 {
        lerp_log(OCCLUSION_CUTOFF_OPEN, OCCLUSION_CUTOFF_ONE, a * 2.0)
    } else {
        lerp_log(OCCLUSION_CUTOFF_ONE, OCCLUSION_CUTOFF_FULL, (a - 0.5) * 2.0)
    }
}

/// Linear gain for an occlusion amount.
pub(crate) fn occlusion_gain(amount: f32) -> f32 {
    10f32.powf(OCCLUSION_GAIN_DB * amount.clamp(0.0, 1.0) / 20.0)
}

/// Coefficient of the one-pole low-pass `y += c (x - y)` for an occlusion
/// amount: exactly 1 (the input unchanged) at 0.
pub(crate) fn occlusion_coef(amount: f32, rate: u32) -> f32 {
    let a = amount.clamp(0.0, 1.0);
    let c = 1.0 - (-std::f32::consts::TAU * occlusion_cutoff(a) / rate as f32).exp();
    1.0 - (1.0 - c) * (a / OCCLUSION_FADE_IN).min(1.0)
}

/// A sound placed in the world: played mono (both channels averaged) with
/// per-side gains set by the caller (distance and direction), looped or
/// not, stopped by id. Gain changes ramp over one buffer (no clicks).
///
/// Behind walls (the viewer's `sound_occlusion`), the mono signal goes
/// through two one-pole low-passes in a row (12 dB an octave) and a gain
/// cut before the panning; both glide to their new values.
pub(crate) struct WorldVoice {
    pub(crate) id: u64,
    bus: usize,
    data: Arc<[f32]>,
    pos: usize,
    looped: bool,
    /// Current and target (left, right) gains.
    gain: (f32, f32),
    target: (f32, f32),
    /// Fading out before removal.
    stopping: bool,
    /// Current and target occlusion (0 clear .. 1).
    occlusion: f32,
    occlusion_target: f32,
    /// States of the two low-passes.
    lowpass: [f32; 2],
    /// Nothing mixed yet.
    fresh: bool,
    /// No occlusion received yet, then gliding to the first one.
    unmeasured: bool,
    first_glide: bool,
}

impl WorldVoice {
    pub(crate) fn new(id: u64, channel: Channel, data: Arc<[f32]>, looped: bool, gain: (f32, f32)) -> Self {
        let g = (sanitize_volume(gain.0), sanitize_volume(gain.1));
        Self {
            id,
            bus: channel.bus().unwrap_or(0),
            data,
            pos: 0,
            looped,
            gain: g,
            target: g,
            stopping: false,
            occlusion: 0.0,
            occlusion_target: 0.0,
            lowpass: [0.0; 2],
            fresh: true,
            unmeasured: true,
            first_glide: false,
        }
    }

    fn set_occlusion(&mut self, amount: f32) {
        let a = if amount.is_finite() { amount.clamp(0.0, 1.0) } else { 0.0 };
        self.occlusion_target = a;
        if self.fresh {
            self.occlusion = a;
        } else if self.unmeasured {
            self.first_glide = true;
        }
        self.unmeasured = false;
    }

    fn mix_into(&mut self, bus: &mut [f32]) -> bool {
        let frames = bus.len() / 2;
        let len = self.data.len() & !1;
        if len == 0 || frames == 0 {
            return !self.stopping && len > 0;
        }
        self.fresh = false;
        let target = if self.stopping { (0.0, 0.0) } else { self.target };
        let (dl, dr) = ((target.0 - self.gain.0) / frames as f32, (target.1 - self.gain.1) / frames as f32);
        let (mut gl, mut gr) = self.gain;
        // the occlusion's glide over this buffer, its filter coefficient
        // and gain ramped from the start value to the end value
        let from = self.occlusion;
        let diff = self.occlusion_target - from;
        let tau = if self.first_glide { OCCLUSION_FIRST_SECS } else { OCCLUSION_SMOOTHING_SECS };
        let to = if diff.abs() < 1.0e-4 {
            self.first_glide = false;
            self.occlusion_target
        } else {
            from + diff * (1.0 - (-(frames as f32) / (tau * ENGINE_SAMPLE_RATE as f32)).exp())
        };
        self.occlusion = to;
        let filtered = from > 0.0 || to > 0.0;
        let (c0, c1) = (occlusion_coef(from, ENGINE_SAMPLE_RATE), occlusion_coef(to, ENGINE_SAMPLE_RATE));
        let (o0, o1) = (occlusion_gain(from), occlusion_gain(to));
        let (dc, dg) = ((c1 - c0) / frames as f32, (o1 - o0) / frames as f32);
        let (mut c, mut og) = (c0, o0);
        let [mut y1, mut y2] = self.lowpass;
        for f in 0..frames {
            if self.pos + 1 >= len {
                if !self.looped {
                    break;
                }
                self.pos = 0;
            }
            let mut m = (self.data[self.pos] + self.data[self.pos + 1]) * 0.5;
            if filtered {
                c += dc;
                og += dg;
                y1 += c * (m - y1);
                y2 += c * (y1 - y2);
                if y1.abs() < DENORMAL {
                    y1 = 0.0;
                }
                if y2.abs() < DENORMAL {
                    y2 = 0.0;
                }
                m = y2 * og;
            } else {
                // (the states follow the input: filtering resumes without a step)
                y1 = m;
                y2 = m;
            }
            gl += dl;
            gr += dr;
            bus[2 * f] += m * gl;
            bus[2 * f + 1] += m * gr;
            self.pos += 2;
        }
        self.lowpass = [y1, y2];
        self.gain = target;
        !self.stopping && (self.looped || self.pos + 1 < len)
    }
}

pub(crate) enum Command {
    AddPcm(Box<PcmSource>),
    PlayClip(ClipVoice),
    PlayWorld(WorldVoice),
    /// New (left, right) gains of a world voice.
    WorldGain(u64, (f32, f32)),
    /// New occlusion of a world voice (0 clear .. 1 behind two walls or more).
    WorldOcclusion(u64, f32),
    /// Fade a world voice out and drop it.
    StopWorld(u64),
}

/// Upper bound on simultaneous world voices (extra requests are dropped).
const MAX_WORLD: usize = 48;

/// Upper bound on simultaneous one-shot clips (extra requests are dropped).
const MAX_CLIPS: usize = 64;

/// The mixer, owned by the audio callback.
pub(crate) struct Mixer {
    params: Arc<SharedParams>,
    cmd_rx: Receiver<Command>,
    pcm: Vec<PcmSource>,
    clips: Vec<ClipVoice>,
    world: Vec<WorldVoice>,
    /// The ambient wind noise, always present, silent unless the viewer
    /// sets a gain.
    wind: WindGen,
    buses: [Vec<f32>; NUM_BUSES],
    smoothers: [GainSmoother; NUM_BUSES],
    tmp: Vec<f32>,
}

impl Mixer {
    pub(crate) fn new(params: Arc<SharedParams>, cmd_rx: Receiver<Command>) -> Self {
        let smoothers =
            std::array::from_fn(|i| GainSmoother::new(params.bus_target(BUS_CHANNELS[i]), GAIN_SMOOTHING_SECS, ENGINE_SAMPLE_RATE));
        Self {
            params,
            cmd_rx,
            pcm: Vec::with_capacity(16),
            clips: Vec::with_capacity(MAX_CLIPS),
            world: Vec::with_capacity(MAX_WORLD),
            wind: WindGen::new(ENGINE_SAMPLE_RATE),
            buses: std::array::from_fn(|_| Vec::with_capacity(4096)),
            smoothers,
            tmp: Vec::with_capacity(4096),
        }
    }

    /// Renders interleaved stereo frames at [`ENGINE_SAMPLE_RATE`] into `out`.
    pub(crate) fn render(&mut self, out: &mut [f32]) {
        for _ in 0..64 {
            match self.cmd_rx.try_recv() {
                Ok(Command::AddPcm(src)) => self.pcm.push(*src),
                Ok(Command::PlayClip(clip)) => {
                    if self.clips.len() < MAX_CLIPS {
                        self.clips.push(clip);
                    }
                }
                Ok(Command::PlayWorld(v)) => {
                    // a new sound with the same id replaces the old one
                    self.world.retain(|w| w.id != v.id);
                    if self.world.len() < MAX_WORLD {
                        self.world.push(v);
                    }
                }
                Ok(Command::WorldGain(id, g)) => {
                    if let Some(w) = self.world.iter_mut().find(|w| w.id == id) {
                        w.target = (sanitize_volume(g.0), sanitize_volume(g.1));
                    }
                }
                Ok(Command::WorldOcclusion(id, amount)) => {
                    if let Some(w) = self.world.iter_mut().find(|w| w.id == id) {
                        w.set_occlusion(amount);
                    }
                }
                Ok(Command::StopWorld(id)) => {
                    for w in self.world.iter_mut().filter(|w| w.id == id) {
                        w.stopping = true;
                    }
                }
                Err(_) => break,
            }
        }

        let len = out.len() & !1;
        for bus in &mut self.buses {
            bus.clear();
            bus.resize(len, 0.0);
        }

        let buses = &mut self.buses;
        let tmp = &mut self.tmp;
        self.pcm.retain_mut(|s| s.mix_into(&mut buses[s.bus], tmp));
        self.clips.retain_mut(|c| c.mix_into(&mut buses[c.bus]));
        self.world.retain_mut(|w| w.mix_into(&mut buses[w.bus]));
        // The wind sits on the Ambient channel (master * AudioLevelAmbient,
        // as the Legacy Viewer's mMaxWindGain).
        if let Some(bus) = Channel::Ambient.bus() {
            self.wind.mix_into(self.params.wind.get(), &mut buses[bus]);
        }

        let targets: [f32; NUM_BUSES] = std::array::from_fn(|i| self.params.bus_target(BUS_CHANNELS[i]));

        for (f, frame) in out[..len].as_chunks_mut::<ENGINE_CHANNELS>().0.iter_mut().enumerate() {
            let mut l = 0.0f32;
            let mut r = 0.0f32;
            for ((smoother, bus), &target) in self.smoothers.iter_mut().zip(&self.buses).zip(&targets) {
                let g = smoother.next(target);
                l += bus[2 * f] * g;
                r += bus[2 * f + 1] * g;
            }
            frame[0] = l.clamp(-1.0, 1.0);
            frame[1] = r.clamp(-1.0, 1.0);
        }
        for s in &mut out[len..] {
            *s = 0.0;
        }
    }

    #[cfg(test)]
    pub(crate) fn source_count(&self) -> usize {
        self.pcm.len() + self.clips.len() + self.world.len()
    }

    #[cfg(test)]
    fn bus_gain_now(&self, ch: Channel) -> f32 {
        ch.bus().map(|b| self.smoothers[b].value()).unwrap_or(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::{Sender, unbounded};

    fn mixer() -> (Mixer, Arc<SharedParams>, Sender<Command>) {
        let params = Arc::new(SharedParams::new());
        let (tx, rx) = unbounded();
        (Mixer::new(params.clone(), rx), params, tx)
    }

    fn settle(m: &mut Mixer) {
        let mut buf = vec![0.0; 48_000];
        m.render(&mut buf);
    }

    #[test]
    fn world_voices_loop_pan_and_stop() {
        let (mut m, _p, tx) = mixer();
        settle(&mut m);
        // 0.1 s of a constant signal, looped, fully left
        let data: Arc<[f32]> = vec![0.5f32; 9_600].into();
        tx.send(Command::PlayWorld(WorldVoice::new(7, Channel::Sfx, data.clone(), true, (1.0, 0.0))))
            .unwrap();
        let mut buf = vec![0.0; 48_000];
        m.render(&mut buf);
        assert_eq!(m.source_count(), 1, "looped voice still playing after 0.5 s");
        let (l, r) = (buf[47_998], buf[47_999]);
        assert!(l > 0.1 && r.abs() < 1e-4, "left only: {l} {r}");
        // moved to the right
        tx.send(Command::WorldGain(7, (0.0, 1.0))).unwrap();
        m.render(&mut buf);
        assert!(buf[47_998].abs() < 1e-4 && buf[47_999] > 0.1);
        // stopped: fades out and goes away
        tx.send(Command::StopWorld(7)).unwrap();
        m.render(&mut buf);
        assert_eq!(m.source_count(), 0);
        // a one-shot world voice ends on its own
        tx.send(Command::PlayWorld(WorldVoice::new(8, Channel::Sfx, data, false, (0.7, 0.7))))
            .unwrap();
        m.render(&mut buf);
        assert_eq!(m.source_count(), 0);
    }

    #[test]
    fn occlusion_curve_goes_from_clear_to_muffled() {
        assert_eq!(occlusion_coef(0.0, ENGINE_SAMPLE_RATE), 1.0, "clear: the input unchanged");
        assert!((occlusion_cutoff(0.5) - 1_000.0).abs() < 1.0);
        assert!((occlusion_cutoff(1.0) - 600.0).abs() < 1.0);
        assert!((20.0 * occlusion_gain(0.5).log10() + 6.0).abs() < 0.01, "one layer: -6 dB");
        assert!((20.0 * occlusion_gain(1.0).log10() + 12.0).abs() < 0.01);
        let (mut cut, mut coef, mut gain) = (f32::MAX, 1.0, 1.0);
        for i in 1..=100 {
            let a = i as f32 / 100.0;
            let (k, c, g) = (occlusion_cutoff(a), occlusion_coef(a, ENGINE_SAMPLE_RATE), occlusion_gain(a));
            assert!(k < cut && c < coef && g < gain, "monotonic at {a}");
            assert!(c > 0.0 && c <= 1.0);
            (cut, coef, gain) = (k, c, g);
        }
        // out of range and garbage stay in range
        assert_eq!(occlusion_coef(-1.0, ENGINE_SAMPLE_RATE), 1.0);
        assert_eq!(occlusion_gain(7.0), occlusion_gain(1.0));
    }

    /// RMS of the left channel of `buf` from frame `from`.
    fn rms(buf: &[f32], from: usize) -> f32 {
        let left: Vec<f32> = buf.as_chunks::<2>().0[from..].iter().map(|f| f[0]).collect();
        (left.iter().map(|v| v * v).sum::<f32>() / left.len() as f32).sqrt()
    }

    fn occluded_render(data: &Arc<[f32]>, occlusion: Option<f32>) -> Vec<f32> {
        let (mut m, _p, tx) = mixer();
        tx.send(Command::PlayWorld(WorldVoice::new(1, Channel::Sfx, data.clone(), true, (1.0, 1.0))))
            .unwrap();
        if let Some(a) = occlusion {
            tx.send(Command::WorldOcclusion(1, a)).unwrap();
        }
        let mut buf = vec![0.0; 9_600];
        m.render(&mut buf);
        buf
    }

    #[test]
    fn occluded_voice_loses_its_highs_and_some_level() {
        // a tone at the Nyquist frequency and one at 100 Hz
        let high: Arc<[f32]> = (0..9_600).map(|i| if (i / 2) % 2 == 0 { 0.5 } else { -0.5 }).collect();
        let low: Arc<[f32]> = (0..96_000)
            .map(|i| 0.5 * ((i / 2) as f32 * 100.0 * std::f32::consts::TAU / 48_000.0).sin())
            .collect();
        let clear = occluded_render(&low, None);
        assert_eq!(clear, occluded_render(&low, Some(0.0)), "occlusion 0 is the plain path");
        let open = rms(&clear, 2_400);
        let muffled = rms(&occluded_render(&low, Some(1.0)), 2_400);
        // -12 dB, the low tone itself barely filtered
        assert!((muffled / open - occlusion_gain(1.0)).abs() < 0.02, "{muffled} / {open}");
        let high_open = rms(&occluded_render(&high, None), 2_400);
        let high_muffled = rms(&occluded_render(&high, Some(0.5)), 2_400);
        assert!(high_muffled < high_open * 0.01, "highs gone: {high_muffled} / {high_open}");
        // the first value of a voice that has not played applies at once
        let first = occluded_render(&high, Some(1.0));
        assert!(rms(&first, 0) < high_open * 0.01);
    }

    #[test]
    fn occlusion_changes_glide_without_clicks() {
        let (mut m, _p, tx) = mixer();
        let data: Arc<[f32]> = vec![0.5f32; 9_600].into();
        tx.send(Command::PlayWorld(WorldVoice::new(3, Channel::Sfx, data, true, (1.0, 1.0))))
            .unwrap();
        let mut buf = vec![0.0; 960];
        m.render(&mut buf);
        tx.send(Command::WorldOcclusion(3, 1.0)).unwrap();
        let mut out = Vec::new();
        for _ in 0..100 {
            m.render(&mut buf);
            out.extend(buf.as_chunks::<2>().0.iter().map(|f| f[0]));
        }
        let max_step = out.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
        assert!(max_step < 0.001, "max step {max_step}");
        assert!(out[0] > 0.45, "glides from where it was: {}", out[0]);
        let end = *out.last().unwrap();
        assert!((end - 0.5 * occlusion_gain(1.0)).abs() < 1e-3, "settles: {end}");
        // and back open, to the exact input
        tx.send(Command::WorldOcclusion(3, 0.0)).unwrap();
        for _ in 0..200 {
            m.render(&mut buf);
        }
        assert_eq!(buf[0], 0.5);
        assert!(m.world[0].occlusion == 0.0);
    }

    #[test]
    fn occlusion_filter_never_leaves_denormals_in_silence() {
        let (mut m, _p, tx) = mixer();
        // one click, then two seconds of silence, looped
        let mut pcm = vec![0.0f32; 192_000];
        pcm[0] = 1.0;
        pcm[1] = 1.0;
        tx.send(Command::PlayWorld(WorldVoice::new(4, Channel::Sfx, pcm.into(), true, (1.0, 1.0))))
            .unwrap();
        tx.send(Command::WorldOcclusion(4, 1.0)).unwrap();
        let mut buf = vec![0.0; 48_000];
        for _ in 0..3 {
            m.render(&mut buf);
            for s in m.world[0].lowpass {
                assert!(s == 0.0 || s.is_normal(), "state {s:e}");
            }
            assert!(buf.iter().all(|s| *s == 0.0 || s.is_normal()));
        }
        assert_eq!(m.world[0].lowpass, [0.0; 2]);
    }

    #[test]
    fn effective_gain_rules() {
        let s = ChannelSettings {
            volume: 0.5,
            muted: false,
            enabled: true,
        };
        assert_eq!(s.effective_gain(), 0.5);
        assert_eq!(ChannelSettings { muted: true, ..s }.effective_gain(), 0.0);
        assert_eq!(ChannelSettings { enabled: false, ..s }.effective_gain(), 0.0);
        assert_eq!(ChannelSettings { volume: 3.0, ..s }.effective_gain(), 1.0);
        assert_eq!(ChannelSettings { volume: f32::NAN, ..s }.effective_gain(), 0.0);
    }

    #[test]
    fn master_scales_every_bus() {
        let master = ChannelSettings {
            volume: 0.5,
            ..Default::default()
        };
        let music = ChannelSettings {
            volume: 0.4,
            ..Default::default()
        };
        assert!((bus_gain(master, music) - 0.2).abs() < 1e-6);
        let muted_master = ChannelSettings { muted: true, ..master };
        assert_eq!(bus_gain(muted_master, music), 0.0);
    }

    #[test]
    fn smoother_is_click_free_and_converges() {
        let mut s = GainSmoother::new(0.0, GAIN_SMOOTHING_SECS, ENGINE_SAMPLE_RATE);
        let mut prev = 0.0;
        let mut max_step = 0.0f32;
        for _ in 0..48_000 {
            let v = s.next(1.0);
            max_step = max_step.max((v - prev).abs());
            prev = v;
        }
        // A hard 0 -> 1 jump becomes a ramp: no single-frame step above 1/500.
        assert!(max_step < 0.002, "max step {max_step}");
        assert!((prev - 1.0).abs() < 1e-5);
    }

    #[test]
    fn clip_follows_channel_and_master_volume() {
        let (mut m, params, tx) = mixer();
        params.set_volume(Channel::Master, 0.5);
        params.set_volume(Channel::Ui, 0.5);
        settle(&mut m);
        let data: Arc<[f32]> = vec![1.0f32; 2000].into();
        tx.send(Command::PlayClip(ClipVoice::new(Channel::Ui, data, 1.0))).unwrap();
        let mut out = vec![0.0; 1000];
        m.render(&mut out);
        assert!(out.iter().all(|&v| (v - 0.25).abs() < 1e-3), "{:?}", &out[..4]);
        assert!((m.bus_gain_now(Channel::Ui) - 0.25).abs() < 1e-4);
        // Clip ends after 2000 samples.
        m.render(&mut out);
        assert_eq!(m.source_count(), 0);
    }

    #[test]
    fn mute_and_disable_silence_a_bus() {
        let (mut m, params, tx) = mixer();
        let data: Arc<[f32]> = vec![0.5f32; 96_000].into();
        tx.send(Command::PlayClip(ClipVoice::new(Channel::Sfx, data, 1.0))).unwrap();
        params.set_muted(Channel::Sfx, true);
        settle(&mut m);
        let mut out = vec![0.0; 256];
        m.render(&mut out);
        assert!(out.iter().all(|v| v.abs() < 1e-4));
        params.set_muted(Channel::Sfx, false);
        params.set_enabled(Channel::Master, false);
        settle(&mut m);
        m.render(&mut out);
        assert!(out.iter().all(|v| v.abs() < 1e-4));
    }

    #[test]
    fn wind_plays_on_the_ambient_channel() {
        let (mut m, params, _tx) = mixer();
        params.wind.set(crate::WindTarget::from_wind([-20.0, 0.0, 0.0], 1.0));
        let mut out = vec![0.0; 48_000];
        m.render(&mut out);
        assert!(out[24_000..].iter().any(|v| v.abs() > 1e-3), "wind heard");
        params.set_muted(Channel::Ambient, true);
        settle(&mut m);
        m.render(&mut out);
        assert!(out.iter().all(|v| v.abs() < 1e-4), "muted with Ambient");
        assert_eq!(m.source_count(), 0, "the wind is not a source");
    }

    #[test]
    fn volume_change_is_ramped() {
        let (mut m, params, tx) = mixer();
        let data: Arc<[f32]> = vec![1.0f32; 200_000].into();
        tx.send(Command::PlayClip(ClipVoice::new(Channel::Music, data, 1.0))).unwrap();
        let mut out = vec![0.0; 2000];
        m.render(&mut out);
        params.set_volume(Channel::Music, 0.0);
        m.render(&mut out);
        // First frame after the change is still near full scale; it decays smoothly.
        assert!(out[0] > 0.9);
        let mut prev = out[0];
        for f in out.as_chunks::<2>().0 {
            assert!(prev - f[0] < 0.01);
            prev = f[0];
        }
    }

    #[test]
    fn jitter_buffer_waits_for_prebuffer_then_plays() {
        let (mut m, _params, tx) = mixer();
        let (mut prod, cons) = rtrb::RingBuffer::<f32>::new(48_000);
        tx.send(Command::AddPcm(Box::new(PcmSource::new(Channel::Voice, cons, 100, None, None))))
            .unwrap();
        // 50 frames queued: below the 100-frame prebuffer, so silence.
        for _ in 0..100 {
            prod.push(0.5).unwrap();
        }
        let mut out = vec![0.0; 64];
        m.render(&mut out);
        assert!(out.iter().all(|&v| v == 0.0));
        for _ in 0..1000 {
            prod.push(0.5).unwrap();
        }
        let mut out = vec![0.0; 1100];
        m.render(&mut out);
        // Fade-in reaches full level after SOURCE_FADE_FRAMES frames.
        assert!((out[1000] - 0.5).abs() < 1e-3, "{}", out[1000]);
        assert!(out[0] < 0.01);
    }

    #[test]
    fn jitter_buffer_caps_latency() {
        let (mut m, _params, tx) = mixer();
        let (mut prod, cons) = rtrb::RingBuffer::<f32>::new(48_000);
        tx.send(Command::AddPcm(Box::new(PcmSource::new(Channel::Voice, cons, 10, Some(100), None))))
            .unwrap();
        // 1000 frames queued, cap is 100: older audio must be dropped.
        for i in 0..2000 {
            prod.push(i as f32).unwrap();
        }
        let mut out = vec![0.0; 2];
        m.render(&mut out);
        assert!(prod.slots() > 48_000 - 40, "queue was not trimmed");
    }

    #[test]
    fn abandoned_and_killed_sources_are_removed() {
        let (mut m, _params, tx) = mixer();
        let (prod, cons) = rtrb::RingBuffer::<f32>::new(1024);
        tx.send(Command::AddPcm(Box::new(PcmSource::new(Channel::Media, cons, 10, None, None))))
            .unwrap();
        let kill = Arc::new(AtomicBool::new(false));
        let (_prod2, cons2) = rtrb::RingBuffer::<f32>::new(1024);
        tx.send(Command::AddPcm(Box::new(PcmSource::new(
            Channel::Music,
            cons2,
            10,
            None,
            Some(kill.clone()),
        ))))
        .unwrap();
        let mut out = vec![0.0; 64];
        m.render(&mut out);
        assert_eq!(m.source_count(), 2);
        drop(prod);
        kill.store(true, Ordering::Relaxed);
        m.render(&mut out);
        assert_eq!(m.source_count(), 0);
    }
}
