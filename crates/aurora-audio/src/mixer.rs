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

use crate::hrtf::{BLOCK, Binaural, Hrir, Hrtf};
use crate::reverb::{Reverb, ReverbParams};
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

/// The occlusion's filter coefficient and gain over one buffer, ramped from
/// their start values to their end values, one frame at a time.
struct OcclusionRamp {
    filtered: bool,
    c: f32,
    dc: f32,
    og: f32,
    dg: f32,
}

impl OcclusionRamp {
    /// The next mono sample through the two low-passes `y` and the gain cut.
    #[inline(always)]
    fn apply(&mut self, m: f32, y: &mut [f32; 2]) -> f32 {
        let [y1, y2] = y;
        if self.filtered {
            self.c += self.dc;
            self.og += self.dg;
            *y1 += self.c * (m - *y1);
            *y2 += self.c * (*y1 - *y2);
            if y1.abs() < DENORMAL {
                *y1 = 0.0;
            }
            if y2.abs() < DENORMAL {
                *y2 = 0.0;
            }
            *y2 * self.og
        } else {
            // (the states follow the input: filtering resumes without a step)
            *y1 = m;
            *y2 = m;
            m
        }
    }
}

/// Frames of the crossfade between the panned and the binaural rendering
/// when a voice changes from one to the other (the setting turned on or
/// off, a sound reaching the listener's head).
const BINAURAL_SWITCH_FRAMES: f32 = 256.0;

/// A sound placed in the world: played mono (both channels averaged) with
/// per-side gains set by the caller (distance and direction), looped or
/// not, stopped by id. Gain changes ramp over one buffer (no clicks).
///
/// Behind walls (the viewer's `sound_occlusion`), the mono signal goes
/// through two one-pole low-passes in a row (12 dB an octave) and a gain
/// cut before the panning; both glide to their new values.
///
/// With the 3D sound for headphones, the caller gives a direction and a
/// distance gain instead of the two gains, and the mono signal goes,
/// after the occlusion, through the head-related filters of that direction
/// (`hrtf.rs`), in a state of the mixer's pool.
///
/// With the reverb of the surroundings, the mono signal after the occlusion
/// also goes to the reverb's send (`reverb.rs`), at its distance gain times
/// its send factor (more for a farther sound); the factor is 0, a dry
/// voice, until the viewer gives one.
pub(crate) struct WorldVoice {
    pub(crate) id: u64,
    bus: usize,
    data: Arc<[f32]>,
    pos: usize,
    looped: bool,
    /// Current and target (left, right) gains of the panned rendering.
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
    /// Rendered binaurally (the last placement was a direction), not panned.
    binaural: bool,
    /// Current and target distance gain of the binaural rendering.
    binaural_gain: f32,
    binaural_target: f32,
    /// Share of the binaural rendering, 0 (panned) to 1, gliding to
    /// `binaural` over [`BINAURAL_SWITCH_FRAMES`].
    blend: f32,
    /// Its binaural state in the mixer's pool, taken the first time it is
    /// placed by direction, given back when it is dropped.
    slot: Option<usize>,
    /// A first placement by direction, applied when the mixer takes the
    /// voice (where the pool is).
    start_binaural: Option<(f32, [f32; 3])>,
    /// Current and target send factors to the reverb.
    send: f32,
    send_target: f32,
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
            binaural: false,
            binaural_gain: 0.0,
            binaural_target: 0.0,
            blend: 0.0,
            slot: None,
            start_binaural: None,
            send: 0.0,
            send_target: 0.0,
        }
    }

    /// New send factor to the reverb: glided to over a buffer, at once
    /// before the voice played.
    fn set_reverb_send(&mut self, factor: f32) {
        let f = if factor.is_finite() { factor.clamp(0.0, MAX_SEND) } else { 0.0 };
        self.send_target = f;
        if self.fresh {
            self.send = f;
        }
    }

    /// A voice placed by direction from its start (3D sound for
    /// headphones): `gain` the distance gain, `direction` in the listener's
    /// frame (x right, y ahead, z up).
    pub(crate) fn new_binaural(id: u64, channel: Channel, data: Arc<[f32]>, looped: bool, gain: f32, direction: [f32; 3]) -> Self {
        let mut v = Self::new(id, channel, data, looped, (0.0, 0.0));
        v.start_binaural = Some((gain, direction));
        v
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

    /// New (left, right) gains: panned from now on (crossfading from the
    /// binaural rendering if it was).
    fn set_gain(&mut self, g: (f32, f32)) {
        self.target = (sanitize_volume(g.0), sanitize_volume(g.1));
        if self.binaural && self.fresh {
            // placed by direction, then panned before it played: panned
            // from the start
            self.gain = self.target;
            self.blend = 0.0;
        }
        self.binaural = false;
    }

    /// New distance gain and direction: rendered binaurally from now on
    /// (crossfading from the panning if it was). A voice without a state
    /// (the pool exhausted, which `MAX_WORLD` prevents) stays panned.
    fn set_binaural(&mut self, gain: f32, direction: [f32; 3], pool: &mut BinauralPool) {
        let g = sanitize_volume(gain);
        if self.slot.is_none() {
            self.slot = pool.free.pop();
        }
        let Some(state) = self.slot.and_then(|s| pool.states.get_mut(s)) else {
            return;
        };
        Hrtf::embedded().lookup(direction, &mut pool.hrir);
        self.binaural_target = g;
        if self.fresh || self.blend == 0.0 {
            // starting: silent histories, the filters of this direction at
            // once (from a panned voice, the crossfade brings it in)
            state.reset(&pool.hrir);
            self.binaural_gain = g;
            if self.fresh {
                self.blend = 1.0;
            }
        } else {
            state.steer(&pool.hrir);
        }
        self.binaural = true;
    }

    /// Mixes one buffer into `bus`, and into the reverb's mono `send` when
    /// it runs. Returns `false` when the voice is done.
    fn mix_into(&mut self, bus: &mut [f32], pool: &mut [Binaural], send: Option<&mut [f32]>) -> bool {
        let frames = bus.len() / 2;
        let len = self.data.len() & !1;
        if len == 0 || frames == 0 {
            return !self.stopping && len > 0;
        }
        self.fresh = false;
        let mut send = send.map(|buf| Send {
            buf,
            factor: self.send,
            step: (self.send_target - self.send) / frames as f32,
        });
        self.send = self.send_target;
        let target = if self.stopping { (0.0, 0.0) } else { self.target };
        let (dl, dr) = ((target.0 - self.gain.0) / frames as f32, (target.1 - self.gain.1) / frames as f32);
        let (mut gl, mut gr) = self.gain;
        // the occlusion's glide over this buffer, its filter coefficient
        // and gain ramped from the start value to the end value
        let from = self.occlusion;
        let diff = self.occlusion_target - from;
        let tau = if self.first_glide {
            OCCLUSION_FIRST_SECS
        } else {
            OCCLUSION_SMOOTHING_SECS
        };
        let to = if diff.abs() < 1.0e-4 {
            self.first_glide = false;
            self.occlusion_target
        } else {
            from + diff * (1.0 - (-(frames as f32) / (tau * ENGINE_SAMPLE_RATE as f32)).exp())
        };
        self.occlusion = to;
        let (c0, c1) = (occlusion_coef(from, ENGINE_SAMPLE_RATE), occlusion_coef(to, ENGINE_SAMPLE_RATE));
        let (o0, o1) = (occlusion_gain(from), occlusion_gain(to));
        let mut ramp = OcclusionRamp {
            filtered: from > 0.0 || to > 0.0,
            c: c0,
            dc: (c1 - c0) / frames as f32,
            og: o0,
            dg: (o1 - o0) / frames as f32,
        };
        let mut lowpass = self.lowpass;
        let state = self.slot.and_then(|s| pool.get_mut(s));
        match state {
            Some(state) if self.binaural || self.blend > 0.0 => {
                self.mix_binaural(bus, state, &mut ramp, &mut lowpass, (gl, gr), (dl, dr), &mut send);
            }
            _ => {
                // panned (the Legacy Viewer's rendering)
                for f in 0..frames {
                    if self.pos + 1 >= len {
                        if !self.looped {
                            break;
                        }
                        self.pos = 0;
                    }
                    let m = ramp.apply((self.data[self.pos] + self.data[self.pos + 1]) * 0.5, &mut lowpass);
                    gl += dl;
                    gr += dr;
                    bus[2 * f] += m * gl;
                    bus[2 * f + 1] += m * gr;
                    if let Some(s) = &mut send {
                        // (the louder side: the distance gain, whatever the
                        // panning)
                        s.add(f, m * gl.max(gr));
                    }
                    self.pos += 2;
                }
                self.blend = 0.0;
            }
        }
        self.lowpass = lowpass;
        self.gain = target;
        !self.stopping && (self.looped || self.pos + 1 < len)
    }

    /// The binaural rendering, by blocks, crossfaded with the panned one
    /// while the voice changes from one to the other.
    fn mix_binaural(
        &mut self,
        bus: &mut [f32],
        state: &mut Binaural,
        ramp: &mut OcclusionRamp,
        lowpass: &mut [f32; 2],
        (mut gl, mut gr): (f32, f32),
        (dl, dr): (f32, f32),
        send: &mut Option<Send>,
    ) {
        let frames = bus.len() / 2;
        let len = self.data.len() & !1;
        let target = if self.stopping { 0.0 } else { self.binaural_target };
        let mut g = self.binaural_gain;
        let dg = (target - g) / frames as f32;
        let mut blend = self.blend;
        let step = (if self.binaural { 1.0 } else { -1.0 }) / BINAURAL_SWITCH_FRAMES;
        let mut ended = false;
        state.begin(frames);
        let mut f = 0;
        while f < frames {
            let n = (frames - f).min(BLOCK);
            let out = &mut bus[2 * f..2 * (f + n)];
            let input = state.input(n);
            for (i, x) in input.iter_mut().enumerate() {
                if !ended && self.pos + 1 >= len {
                    if self.looped {
                        self.pos = 0;
                    } else {
                        ended = true;
                    }
                }
                let m = if ended {
                    0.0
                } else {
                    let m = ramp.apply((self.data[self.pos] + self.data[self.pos + 1]) * 0.5, lowpass);
                    self.pos += 2;
                    m
                };
                gl += dl;
                gr += dr;
                g += dg;
                blend = (blend + step).clamp(0.0, 1.0);
                let panned = 1.0 - blend;
                if panned > 0.0 {
                    out[2 * i] += m * gl * panned;
                    out[2 * i + 1] += m * gr * panned;
                }
                *x = m * g * blend;
                if let Some(s) = send {
                    s.add(f + i, m * (g * blend + gl.max(gr) * panned));
                }
            }
            state.render(n, out);
            f += n;
        }
        self.blend = blend;
        self.binaural_gain = target;
    }
}

/// Most a voice sends to the reverb, over its distance gain.
const MAX_SEND: f32 = 4.0;

/// A voice's share of the reverb's mono send over one buffer, its factor
/// ramped from the last value to the new one.
struct Send<'a> {
    buf: &'a mut [f32],
    factor: f32,
    step: f32,
}

impl Send<'_> {
    #[inline(always)]
    fn add(&mut self, frame: usize, v: f32) {
        self.factor += self.step;
        if let Some(o) = self.buf.get_mut(frame) {
            *o += v * self.factor;
        }
    }
}

/// The binaural states of the world voices, allocated with the mixer
/// (one per voice it can hold), so that the audio callback never allocates.
struct BinauralPool {
    states: Vec<Binaural>,
    /// Indices of the states no voice holds.
    free: Vec<usize>,
    /// Scratch for a direction's filters.
    hrir: Hrir,
}

impl BinauralPool {
    fn new() -> Self {
        Self {
            states: (0..MAX_WORLD).map(|_| Binaural::new()).collect(),
            free: (0..MAX_WORLD).rev().collect(),
            hrir: Hrir::default(),
        }
    }

    /// Gives back the state of a voice being dropped.
    fn release(free: &mut Vec<usize>, voice: &mut WorldVoice) {
        if let Some(s) = voice.slot.take() {
            free.push(s);
        }
    }
}

pub(crate) enum Command {
    AddPcm(Box<PcmSource>),
    PlayClip(ClipVoice),
    PlayWorld(WorldVoice),
    /// New (left, right) gains of a world voice: panned.
    WorldGain(u64, (f32, f32)),
    /// New distance gain and direction (listener's frame: x right, y
    /// ahead, z up) of a world voice: rendered binaurally.
    WorldBinaural(u64, f32, [f32; 3]),
    /// New occlusion of a world voice (0 clear .. 1 behind two walls or more).
    WorldOcclusion(u64, f32),
    /// New send factor of a world voice to the reverb.
    WorldReverbSend(u64, f32),
    /// The reverb of the surroundings on with these parameters, or off.
    Reverb(Option<ReverbParams>),
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
    /// The world voices' binaural states.
    binaural: BinauralPool,
    /// The reverb of the surroundings, its mono send per bus (only the
    /// Sounds and Ambient buses feed it) and their sum.
    reverb: Reverb,
    sends: [Vec<f32>; NUM_BUSES],
    send: Vec<f32>,
    /// The reverb's stereo output of a buffer.
    wet: Vec<f32>,
    /// Gains of the reverb's input (the Sounds and Ambient channels alone)
    /// and output (master), so that muting the master silences its tail
    /// at once.
    reverb_gains: [GainSmoother; 3],
}

/// Buses whose world voices feed the reverb.
const REVERB_BUSES: [Channel; 2] = [Channel::Sfx, Channel::Ambient];

fn feeds_reverb(bus: usize) -> bool {
    REVERB_BUSES.iter().any(|c| c.bus() == Some(bus))
}

impl Mixer {
    pub(crate) fn new(params: Arc<SharedParams>, cmd_rx: Receiver<Command>) -> Self {
        // the filters decoded here, off the audio callback
        Hrtf::embedded();
        let smoothers =
            std::array::from_fn(|i| GainSmoother::new(params.bus_target(BUS_CHANNELS[i]), GAIN_SMOOTHING_SECS, ENGINE_SAMPLE_RATE));
        let reverb_gains = [Channel::Master, REVERB_BUSES[0], REVERB_BUSES[1]]
            .map(|ch| GainSmoother::new(params.get(ch).effective_gain(), GAIN_SMOOTHING_SECS, ENGINE_SAMPLE_RATE));
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
            binaural: BinauralPool::new(),
            reverb: Reverb::new(),
            sends: std::array::from_fn(|_| Vec::with_capacity(4096)),
            send: Vec::with_capacity(4096),
            wet: Vec::with_capacity(4096),
            reverb_gains,
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
                Ok(Command::PlayWorld(mut v)) => {
                    // a new sound with the same id replaces the old one
                    let free = &mut self.binaural.free;
                    self.world.retain_mut(|w| {
                        let keep = w.id != v.id;
                        if !keep {
                            BinauralPool::release(free, w);
                        }
                        keep
                    });
                    if self.world.len() < MAX_WORLD {
                        if let Some((gain, direction)) = v.start_binaural.take() {
                            v.set_binaural(gain, direction, &mut self.binaural);
                        }
                        self.world.push(v);
                    }
                }
                Ok(Command::WorldGain(id, g)) => {
                    if let Some(w) = self.world.iter_mut().find(|w| w.id == id) {
                        w.set_gain(g);
                    }
                }
                Ok(Command::WorldBinaural(id, gain, direction)) => {
                    if let Some(w) = self.world.iter_mut().find(|w| w.id == id) {
                        w.set_binaural(gain, direction, &mut self.binaural);
                    }
                }
                Ok(Command::WorldOcclusion(id, amount)) => {
                    if let Some(w) = self.world.iter_mut().find(|w| w.id == id) {
                        w.set_occlusion(amount);
                    }
                }
                Ok(Command::WorldReverbSend(id, factor)) => {
                    if let Some(w) = self.world.iter_mut().find(|w| w.id == id) {
                        w.set_reverb_send(factor);
                    }
                }
                Ok(Command::Reverb(params)) => self.reverb.set(params),
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
        // the reverb's sends, only while it runs: off, nothing differs from
        // the dry mix
        let reverb = self.reverb.running();
        if reverb {
            for b in REVERB_BUSES.iter().filter_map(|c| c.bus()) {
                self.sends[b].clear();
                self.sends[b].resize(len / 2, 0.0);
            }
        }

        let buses = &mut self.buses;
        let sends = &mut self.sends;
        let tmp = &mut self.tmp;
        self.pcm.retain_mut(|s| s.mix_into(&mut buses[s.bus], tmp));
        self.clips.retain_mut(|c| c.mix_into(&mut buses[c.bus]));
        let (states, free) = (&mut self.binaural.states, &mut self.binaural.free);
        self.world.retain_mut(|w| {
            let send = (reverb && feeds_reverb(w.bus)).then(|| &mut sends[w.bus][..]);
            let keep = w.mix_into(&mut buses[w.bus], states, send);
            if !keep {
                BinauralPool::release(free, w);
            }
            keep
        });
        // The wind sits on the Ambient channel (master * AudioLevelAmbient,
        // as the Legacy Viewer's mMaxWindGain).
        if let Some(bus) = Channel::Ambient.bus() {
            self.wind.mix_into(self.params.wind.get(), &mut buses[bus]);
        }

        let targets: [f32; NUM_BUSES] = std::array::from_fn(|i| self.params.bus_target(BUS_CHANNELS[i]));

        if !reverb {
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
        } else {
            self.mix_with_reverb(&mut out[..len], &targets);
        }
        for s in &mut out[len..] {
            *s = 0.0;
        }
    }

    /// The final mix with the reverb: the buses as without it, the sends
    /// summed at their channels' volumes into the reverb, its output at the
    /// master volume, then the clamp.
    fn mix_with_reverb(&mut self, out: &mut [f32], targets: &[f32; NUM_BUSES]) {
        let frames = out.len() / 2;
        let gain = |ch: Channel| self.params.get(ch).effective_gain();
        let reverb_targets = [gain(Channel::Master), gain(REVERB_BUSES[0]), gain(REVERB_BUSES[1])];
        let reverb_buses = REVERB_BUSES.map(|c| c.bus().unwrap_or(0));
        self.send.clear();
        self.send.resize(frames, 0.0);
        for (f, frame) in out.as_chunks_mut::<ENGINE_CHANNELS>().0.iter_mut().enumerate() {
            let mut l = 0.0f32;
            let mut r = 0.0f32;
            for ((smoother, bus), &target) in self.smoothers.iter_mut().zip(&self.buses).zip(targets) {
                let g = smoother.next(target);
                l += bus[2 * f] * g;
                r += bus[2 * f + 1] * g;
            }
            frame[0] = l;
            frame[1] = r;
            let [_, sfx, ambient] = &mut self.reverb_gains;
            self.send[f] = self.sends[reverb_buses[0]][f] * sfx.next(reverb_targets[1])
                + self.sends[reverb_buses[1]][f] * ambient.next(reverb_targets[2]);
        }
        self.wet.clear();
        self.wet.resize(out.len(), 0.0);
        self.reverb.render(&self.send, &mut self.wet);
        let master = &mut self.reverb_gains[0];
        for (frame, wet) in out
            .as_chunks_mut::<ENGINE_CHANNELS>()
            .0
            .iter_mut()
            .zip(self.wet.as_chunks::<ENGINE_CHANNELS>().0)
        {
            let g = master.next(reverb_targets[0]);
            frame[0] = (frame[0] + wet[0] * g).clamp(-1.0, 1.0);
            frame[1] = (frame[1] + wet[1] * g).clamp(-1.0, 1.0);
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

    /// The world voice's panned rendering as it was before the binaural
    /// one, kept to prove that the panned path still renders the same
    /// samples, bit for bit.
    struct PannedReference {
        data: Arc<[f32]>,
        pos: usize,
        looped: bool,
        gain: (f32, f32),
        target: (f32, f32),
        occlusion: f32,
        occlusion_target: f32,
        lowpass: [f32; 2],
        first_glide: bool,
    }

    impl PannedReference {
        fn mix_into(&mut self, bus: &mut [f32]) {
            let frames = bus.len() / 2;
            let len = self.data.len() & !1;
            let target = self.target;
            let (dl, dr) = ((target.0 - self.gain.0) / frames as f32, (target.1 - self.gain.1) / frames as f32);
            let (mut gl, mut gr) = self.gain;
            let from = self.occlusion;
            let diff = self.occlusion_target - from;
            let tau = if self.first_glide {
                OCCLUSION_FIRST_SECS
            } else {
                OCCLUSION_SMOOTHING_SECS
            };
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
        }
    }

    /// A second of stereo noise with a tone.
    fn noise(seed: u32, frames: usize) -> Arc<[f32]> {
        let mut n = seed;
        (0..frames * 2)
            .map(|i| {
                n = n.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let white = (n >> 8) as f32 / (1u32 << 24) as f32 - 0.5;
                white * 0.6 + 0.3 * ((i / 2) as f32 * 0.05).sin()
            })
            .collect()
    }

    #[test]
    fn panned_voices_render_bit_for_bit_as_before() {
        let data = noise(5, 30_000);
        let mut voice = WorldVoice::new(9, Channel::Sfx, data.clone(), true, (0.8, 0.3));
        let mut reference = PannedReference {
            data,
            pos: 0,
            looped: true,
            gain: (0.8, 0.3),
            target: (0.8, 0.3),
            occlusion: 0.0,
            occlusion_target: 0.0,
            lowpass: [0.0; 2],
            first_glide: false,
        };
        let mut pool = BinauralPool::new();
        let steps: [(usize, Option<(f32, f32)>, Option<f32>); 8] = [
            (480, None, None),
            (333, Some((0.2, 0.9)), None),
            (1024, None, Some(0.7)),
            (7, Some((1.0, 1.0)), None),
            (480, None, Some(0.2)),
            (4096, Some((0.0, 0.5)), Some(0.0)),
            (480, None, None),
            (961, Some((0.6, 0.6)), Some(1.0)),
        ];
        for (k, (frames, gain, occlusion)) in steps.into_iter().enumerate() {
            if let Some(g) = gain {
                voice.set_gain(g);
                reference.target = g;
            }
            if let Some(a) = occlusion {
                voice.set_occlusion(a);
                reference.occlusion_target = a;
                if k == 2 {
                    reference.first_glide = true;
                }
            }
            let (mut a, mut b) = (vec![0.0f32; frames * 2], vec![0.0f32; frames * 2]);
            // (feeding the reverb or not changes nothing in the dry mix)
            let mut send = vec![0.0f32; frames];
            voice.mix_into(&mut a, &mut pool.states, (k % 2 == 1).then_some(&mut send[..]));
            reference.mix_into(&mut b);
            assert!(a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()), "buffer {k} differs");
        }
        assert!(voice.slot.is_none(), "a panned voice never takes a binaural state");
    }

    /// Renders voice 1 by buffers of `frames`, calling `between` before
    /// each one, the left and right channels interleaved.
    fn run(m: &mut Mixer, tx: &Sender<Command>, buffers: usize, frames: usize, between: impl Fn(usize, &Sender<Command>)) -> Vec<f32> {
        let mut out = Vec::new();
        let mut buf = vec![0.0; frames * 2];
        for k in 0..buffers {
            between(k, tx);
            m.render(&mut buf);
            out.extend_from_slice(&buf);
        }
        out
    }

    fn largest_step(out: &[f32], from: usize) -> f32 {
        out.as_chunks::<2>().0[from..]
            .windows(2)
            .map(|w| (w[1][0] - w[0][0]).abs().max((w[1][1] - w[0][1]).abs()))
            .fold(0.0, f32::max)
    }

    #[test]
    fn binaural_voices_turn_and_switch_without_clicks() {
        let sine: Arc<[f32]> = (0..96_000)
            .map(|i| 0.4 * ((i / 2) as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin())
            .collect();
        let start = |tx: &Sender<Command>, binaural: bool| {
            let v = if binaural {
                WorldVoice::new_binaural(1, Channel::Sfx, sine.clone(), true, 1.0, [0.0, 1.0, 0.0])
            } else {
                WorldVoice::new(1, Channel::Sfx, sine.clone(), true, (1.0, 1.0))
            };
            tx.send(Command::PlayWorld(v)).unwrap();
        };
        // still, panned or binaural from every side: the steepest a sine
        // makes there
        let mut still = 0.0f32;
        for k in 0..8 {
            let (mut m, _p, tx) = mixer();
            settle(&mut m);
            start(&tx, k > 0);
            let a = (k as f32 * 45.0).to_radians();
            let out = run(&mut m, &tx, 40, 480, |_, tx| {
                if k > 0 {
                    tx.send(Command::WorldBinaural(1, 1.0, [a.sin(), a.cos(), 0.0])).unwrap();
                }
            });
            still = still.max(largest_step(&out, 4_800));
        }
        // turning around fast, and switching between panned and binaural
        // every 50 ms
        let (mut m, _p, tx) = mixer();
        settle(&mut m);
        start(&tx, true);
        let out = run(&mut m, &tx, 200, 480, |k, tx| {
            if (k / 5) % 2 == 1 {
                tx.send(Command::WorldGain(1, (1.0, 1.0))).unwrap();
            } else {
                let a = (k as f32 * 23.0).to_radians();
                tx.send(Command::WorldBinaural(1, 1.0, [a.sin(), a.cos(), 0.3])).unwrap();
            }
        });
        let step = largest_step(&out, 4_800);
        assert!(step < still * 1.3, "largest step {step}, still sources {still}");
        assert!(out.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn binaural_voices_are_occluded_before_their_filters() {
        // a 6 kHz tone (the filters fade out above 20 kHz)
        let high: Arc<[f32]> = (0..9_600)
            .map(|i| 0.5 * ((i / 2) as f32 * 6_000.0 * std::f32::consts::TAU / 48_000.0).sin())
            .collect();
        let level = |occlusion: f32| {
            let (mut m, _p, tx) = mixer();
            tx.send(Command::PlayWorld(WorldVoice::new_binaural(
                1,
                Channel::Sfx,
                high.clone(),
                true,
                1.0,
                [0.3, 1.0, 0.0],
            )))
            .unwrap();
            tx.send(Command::WorldOcclusion(1, occlusion)).unwrap();
            let mut buf = vec![0.0; 19_200];
            m.render(&mut buf);
            rms(&buf, 2_400)
        };
        let (open, muffled) = (level(0.0), level(1.0));
        assert!(open > 0.01, "heard: {open}");
        assert!(muffled < open * 0.01, "highs gone: {muffled} / {open}");
    }

    #[test]
    fn binaural_states_come_back_to_the_pool() {
        let (mut m, _p, tx) = mixer();
        let data: Arc<[f32]> = vec![0.1f32; 960].into();
        let mut buf = vec![0.0; 960];
        for round in 0..3 {
            for id in 0..MAX_WORLD as u64 {
                let id = id + round * 100;
                tx.send(Command::PlayWorld(WorldVoice::new_binaural(
                    id,
                    Channel::Sfx,
                    data.clone(),
                    true,
                    1.0,
                    [1.0, 0.0, 0.0],
                )))
                .unwrap();
                if id % 64 == 63 {
                    m.render(&mut buf);
                }
            }
            m.render(&mut buf);
            assert_eq!(m.world.len(), MAX_WORLD);
            assert!(m.world.iter().all(|w| w.slot.is_some()), "round {round}");
            // the same ids again replace them
            for id in 0..4 {
                tx.send(Command::PlayWorld(WorldVoice::new_binaural(
                    id + round * 100,
                    Channel::Sfx,
                    data.clone(),
                    true,
                    1.0,
                    [0.0, 1.0, 0.0],
                )))
                .unwrap();
            }
            m.render(&mut buf);
            assert!(m.world.iter().all(|w| w.slot.is_some()));
            for id in 0..MAX_WORLD as u64 {
                tx.send(Command::StopWorld(id + round * 100)).unwrap();
                if id % 64 == 63 {
                    m.render(&mut buf);
                }
            }
            m.render(&mut buf);
            assert_eq!(m.world.len(), 0);
            assert_eq!(m.binaural.free.len(), MAX_WORLD);
        }
    }

    #[test]
    fn binaural_silence_leaves_no_denormals() {
        let (mut m, _p, tx) = mixer();
        let mut pcm = vec![0.0f32; 192_000];
        pcm[0] = 1.0;
        pcm[1] = 1.0;
        tx.send(Command::PlayWorld(WorldVoice::new_binaural(
            4,
            Channel::Sfx,
            pcm.into(),
            true,
            1.0,
            [1.0, 0.2, 0.4],
        )))
        .unwrap();
        tx.send(Command::WorldOcclusion(4, 1.0)).unwrap();
        let mut buf = vec![0.0; 48_000];
        for k in 0..3 {
            tx.send(Command::WorldBinaural(4, 0.7, [k as f32, 1.0, 0.0])).unwrap();
            m.render(&mut buf);
            assert!(buf.iter().all(|s| *s == 0.0 || s.is_normal()));
        }
        let slot = m.world[0].slot.expect("binaural");
        assert!(m.binaural.states[slot].states().all(|s| s == 0.0 || s.is_normal()));
    }

    /// The mixer's cost with 30 world sounds: printed (`cargo test
    /// --workspace binaural_render_cost -- --nocapture`), and bounded in
    /// optimised builds.
    #[test]
    fn binaural_render_cost() {
        const VOICES: u64 = 30;
        const FRAMES: usize = 480;
        let measure = |binaural: bool| {
            let (mut m, _p, tx) = mixer();
            for id in 0..VOICES {
                let data = noise(id as u32, 24_000);
                let v = if binaural {
                    WorldVoice::new_binaural(id, Channel::Sfx, data, true, 0.2, [1.0, 1.0, 0.0])
                } else {
                    WorldVoice::new(id, Channel::Sfx, data, true, (0.2, 0.2))
                };
                tx.send(Command::PlayWorld(v)).unwrap();
                tx.send(Command::WorldOcclusion(id, 0.3)).unwrap();
            }
            let mut buf = vec![0.0; FRAMES * 2];
            let mut times = Vec::with_capacity(300);
            for k in 0..300 {
                // every voice moves every buffer (the camera turning)
                for id in 0..VOICES {
                    let a = (k as f32 * 3.0 + id as f32 * 12.0).to_radians();
                    if binaural {
                        tx.send(Command::WorldBinaural(id, 0.2, [a.sin(), a.cos(), 0.2])).unwrap();
                    } else {
                        tx.send(Command::WorldGain(id, (0.2 + 0.1 * a.sin(), 0.2 - 0.1 * a.sin()))).unwrap();
                    }
                }
                let t = std::time::Instant::now();
                m.render(&mut buf);
                times.push(t.elapsed().as_secs_f64() * 1e6);
            }
            times.sort_by(f64::total_cmp);
            (times[0], times[times.len() / 2], times[times.len() * 99 / 100])
        };
        let period = FRAMES as f64 / ENGINE_SAMPLE_RATE as f64 * 1e6;
        let panned = measure(false);
        let binaural = measure(true);
        eprintln!(
            "mixer render, {VOICES} world sounds, {FRAMES} frames ({period:.0} us): panned min {:.0} / median {:.0} / p99 {:.0} us, \
             binaural min {:.0} / median {:.0} / p99 {:.0} us",
            panned.0, panned.1, panned.2, binaural.0, binaural.1, binaural.2
        );
        if !cfg!(debug_assertions) {
            // a small share of the buffer's time, even on a loaded PC
            assert!(binaural.0 < period * 0.2, "binaural render {:.0} us of {period:.0}", binaural.0);
        }
    }

    fn small_room() -> ReverbParams {
        ReverbParams {
            level: 1.0,
            decay: 0.6,
            predelay: 0.008,
            damping: 0.7,
            path: 4.0,
        }
    }

    /// Renders `buffers` buffers of 480 frames, `between(k, tx)` sent
    /// before each.
    fn render_all(m: &mut Mixer, tx: &Sender<Command>, buffers: usize, between: impl Fn(usize, &Sender<Command>)) -> Vec<f32> {
        run(m, tx, buffers, 480, between)
    }

    #[test]
    fn reverb_off_is_bit_identical_to_the_dry_mix() {
        let data = noise(11, 24_000);
        let voices = |tx: &Sender<Command>| {
            tx.send(Command::PlayWorld(WorldVoice::new(1, Channel::Sfx, data.clone(), true, (0.6, 0.3))))
                .unwrap();
            tx.send(Command::PlayWorld(WorldVoice::new_binaural(
                2,
                Channel::Ambient,
                data.clone(),
                true,
                0.4,
                [0.5, 0.5, 0.2],
            )))
            .unwrap();
            tx.send(Command::WorldOcclusion(1, 0.4)).unwrap();
        };
        let moves = |k: usize, tx: &Sender<Command>| {
            tx.send(Command::WorldGain(1, (0.6 - 0.001 * k as f32, 0.3))).unwrap();
        };
        // never told about the reverb
        let (mut dry, _p, tx) = mixer();
        voices(&tx);
        let dry_out = render_all(&mut dry, &tx, 300, moves);
        // sends given, the reverb never on: the same samples
        let (mut sent, _p, tx) = mixer();
        voices(&tx);
        tx.send(Command::WorldReverbSend(1, 1.5)).unwrap();
        tx.send(Command::WorldReverbSend(2, 2.0)).unwrap();
        let sent_out = render_all(&mut sent, &tx, 300, moves);
        assert!(
            dry_out.iter().zip(&sent_out).all(|(a, b)| a.to_bits() == b.to_bits()),
            "sends alone change nothing"
        );
        // on for a second, then off: once faded, the same samples again
        let (mut wet, _p, tx) = mixer();
        voices(&tx);
        tx.send(Command::WorldReverbSend(1, 1.5)).unwrap();
        tx.send(Command::WorldReverbSend(2, 2.0)).unwrap();
        tx.send(Command::Reverb(Some(small_room()))).unwrap();
        let wet_out = render_all(&mut wet, &tx, 300, |k, tx| {
            moves(k, tx);
            if k == 100 {
                tx.send(Command::Reverb(None)).unwrap();
            }
        });
        let on = &wet_out[..100 * 960];
        assert!(
            on.iter().zip(&dry_out).any(|(a, b)| (a - b).abs() > 1e-3),
            "the reverb is heard while on"
        );
        let after = 110 * 960;
        assert!(
            wet_out[after..]
                .iter()
                .zip(&dry_out[after..])
                .all(|(a, b)| a.to_bits() == b.to_bits()),
            "off: the dry mix, bit for bit"
        );
        assert!(!wet.reverb.running());
    }

    #[test]
    fn only_world_sounds_on_sounds_and_ambient_are_reverberated() {
        let data = noise(3, 9_600);
        let render = |reverb: bool| {
            let (mut m, _p, tx) = mixer();
            if reverb {
                tx.send(Command::Reverb(Some(small_room()))).unwrap();
            }
            // an interface sound played as a world voice (previews), a clip
            tx.send(Command::PlayWorld(WorldVoice::new(1, Channel::Ui, data.clone(), true, (0.5, 0.5))))
                .unwrap();
            tx.send(Command::WorldReverbSend(1, 2.0)).unwrap();
            tx.send(Command::PlayClip(ClipVoice::new(Channel::Sfx, data.clone(), 0.5))).unwrap();
            // voice chat: a PCM source
            let (mut prod, cons) = rtrb::RingBuffer::<f32>::new(96_000);
            tx.send(Command::AddPcm(Box::new(PcmSource::new(Channel::Voice, cons, 10, None, None))))
                .unwrap();
            for v in data.iter() {
                let _ = prod.push(*v * 0.3);
            }
            render_all(&mut m, &tx, 100, |_, _| {})
        };
        let (off, on) = (render(false), render(true));
        assert!(off.iter().any(|v| v.abs() > 1e-3), "heard");
        assert!(off.iter().zip(&on).all(|(a, b)| a.to_bits() == b.to_bits()));
    }

    #[test]
    fn a_world_sound_rings_after_it_stops_and_master_mute_silences_it() {
        let click: Arc<[f32]> = vec![0.8f32; 96].into();
        let tail = |mute_at: Option<usize>| {
            let (mut m, params, tx) = mixer();
            tx.send(Command::Reverb(Some(small_room()))).unwrap();
            tx.send(Command::PlayWorld(WorldVoice::new(
                1,
                Channel::Sfx,
                click.clone(),
                false,
                (1.0, 1.0),
            )))
            .unwrap();
            tx.send(Command::WorldReverbSend(1, 1.0)).unwrap();
            let mut out = Vec::new();
            let mut buf = vec![0.0; 960];
            for k in 0..60 {
                if Some(k) == mute_at {
                    params.set_muted(Channel::Master, true);
                }
                m.render(&mut buf);
                out.extend_from_slice(&buf);
            }
            out
        };
        let ringing = tail(None);
        // from 100 ms after the click: only the reverb
        let late = rms(&ringing[..20 * 960], 4_800);
        assert!(late > 1e-4, "rings: {late}");
        let muted = tail(Some(10));
        let after = &muted[10 * 960 + 2 * 4_800..];
        assert!(after.iter().all(|v| v.abs() < 1e-4), "muted at once");
    }

    #[test]
    fn panned_and_binaural_voices_feed_the_same_reverb() {
        let data = noise(21, 24_000);
        let wet = |binaural: bool| {
            let out = |reverb: bool| {
                let (mut m, _p, tx) = mixer();
                if reverb {
                    tx.send(Command::Reverb(Some(small_room()))).unwrap();
                }
                let v = if binaural {
                    WorldVoice::new_binaural(1, Channel::Sfx, data.clone(), true, 0.5, [0.7, 0.7, 0.0])
                } else {
                    WorldVoice::new(1, Channel::Sfx, data.clone(), true, (0.5, 0.2))
                };
                tx.send(Command::PlayWorld(v)).unwrap();
                tx.send(Command::WorldReverbSend(1, 1.5)).unwrap();
                tx.send(Command::WorldOcclusion(1, 0.3)).unwrap();
                render_all(&mut m, &tx, 50, |_, _| {})
            };
            let (on, off) = (out(true), out(false));
            on.iter().zip(&off).map(|(a, b)| a - b).collect::<Vec<f32>>()
        };
        let (p, b) = (wet(false), wet(true));
        let level = rms(&p, 4_800);
        assert!(level > 1e-3, "heard: {level}");
        let diff = p.iter().zip(&b).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
        assert!(diff < 1e-5, "the same tail either way: {diff}");
    }

    /// The mixer's cost with 30 world sounds and the reverb: printed
    /// (`cargo test --workspace reverb_render_cost -- --nocapture`), and
    /// bounded in optimised builds.
    #[test]
    fn reverb_render_cost() {
        const VOICES: u64 = 30;
        const FRAMES: usize = 480;
        let measure = |reverb: bool| {
            let (mut m, _p, tx) = mixer();
            if reverb {
                tx.send(Command::Reverb(Some(ReverbParams {
                    level: 1.0,
                    decay: 2.5,
                    predelay: 0.03,
                    damping: 0.5,
                    path: 18.0,
                })))
                .unwrap();
            }
            for id in 0..VOICES {
                let v = WorldVoice::new(id, Channel::Sfx, noise(id as u32, 24_000), true, (0.2, 0.2));
                tx.send(Command::PlayWorld(v)).unwrap();
                tx.send(Command::WorldOcclusion(id, 0.3)).unwrap();
                tx.send(Command::WorldReverbSend(id, 1.5)).unwrap();
            }
            let mut buf = vec![0.0; FRAMES * 2];
            let mut times = Vec::with_capacity(300);
            for k in 0..300 {
                for id in 0..VOICES {
                    let a = (k as f32 * 3.0 + id as f32 * 12.0).to_radians();
                    tx.send(Command::WorldGain(id, (0.2 + 0.1 * a.sin(), 0.2 - 0.1 * a.sin()))).unwrap();
                }
                let t = std::time::Instant::now();
                m.render(&mut buf);
                times.push(t.elapsed().as_secs_f64() * 1e6);
            }
            times.sort_by(f64::total_cmp);
            (times[0], times[times.len() / 2], times[times.len() * 99 / 100])
        };
        let period = FRAMES as f64 / ENGINE_SAMPLE_RATE as f64 * 1e6;
        let dry = measure(false);
        let wet = measure(true);
        eprintln!(
            "mixer render, {VOICES} world sounds, {FRAMES} frames ({period:.0} us): dry min {:.0} / median {:.0} / p99 {:.0} us, \
             reverb min {:.0} / median {:.0} / p99 {:.0} us",
            dry.0, dry.1, dry.2, wet.0, wet.1, wet.2
        );
        if !cfg!(debug_assertions) {
            assert!(wet.0 - dry.0 < period * 0.05, "reverb {:.0} us of {period:.0}", wet.0 - dry.0);
        }
    }
}
