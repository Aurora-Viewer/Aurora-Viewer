//! Compressed-audio decoding (symphonia): MP3, AAC (ADTS), Ogg Vorbis, WAV.

use std::io::Cursor;
use std::sync::Arc;

use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::well_known::FORMAT_ID_OGG;
use symphonia::core::formats::{FormatOptions, FormatReader, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::{MetadataOptions, StandardTag};

use crate::AudioError;
use crate::mixer::ENGINE_SAMPLE_RATE;
use crate::resample::{Resampler, to_stereo};

/// One decoded block of interleaved samples.
pub(crate) struct Chunk<'a> {
    pub samples: &'a [f32],
    pub channels: usize,
    pub rate: u32,
}

/// A demuxer + decoder pair reading from any (possibly non-seekable) source.
pub(crate) struct DecodeStream {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    /// Interleaved samples of the last decoded packet (reused between packets).
    samples: Vec<f32>,
}

fn dec_err(e: SymError) -> AudioError {
    AudioError::Decode(e.to_string())
}

/// Source sample rates the decoder accepts, in Hz: from the lowest rate of
/// the supported codecs (7.35 kHz for AAC, 8 kHz for MP3; sound assets are
/// uploaded at 44.1 kHz, LLVORBIS_CLIP_SAMPLE_RATE in llvorbisencode.h) to
/// 192 kHz. Keeps the resampler's output within a few times its input.
pub(crate) const SAMPLE_RATES: std::ops::RangeInclusive<u32> = 7_350..=192_000;

fn check_rate(rate: u32) -> Result<(), AudioError> {
    if SAMPLE_RATES.contains(&rate) {
        Ok(())
    } else {
        Err(AudioError::Decode(format!("unsupported sample rate: {rate} Hz")))
    }
}

impl DecodeStream {
    /// `ext` is a file-extension style hint ("mp3", "aac", "ogg", "wav").
    pub(crate) fn open(source: Box<dyn MediaSource>, ext: Option<&str>) -> Result<Self, AudioError> {
        let mss = MediaSourceStream::new(source, Default::default());
        let mut hint = Hint::new();
        if let Some(ext) = ext {
            hint.with_extension(ext);
        }
        let format = symphonia::default::get_probe()
            .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
            .map_err(dec_err)?;
        let (decoder, track_id) = make_decoder(format.as_ref())?;
        Ok(Self {
            format,
            decoder,
            track_id,
            samples: Vec::new(),
        })
    }

    /// Next decoded block, or `Ok(None)` at the end of the stream.
    pub(crate) fn next_chunk(&mut self) -> Result<Option<Chunk<'_>>, AudioError> {
        let Some((channels, rate)) = self.decode_next()? else {
            return Ok(None);
        };
        Ok(Some(Chunk {
            samples: &self.samples,
            channels,
            rate,
        }))
    }

    /// Decodes the next packet into `samples`; returns (channels, rate).
    fn decode_next(&mut self) -> Result<Option<(usize, u32)>, AudioError> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(Some(p)) => p,
                Ok(None) => return Ok(None),
                Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Ok(None);
                }
                Err(SymError::ResetRequired) => {
                    // Chained Ogg stream (new song on Icecast): new decoder.
                    let (decoder, track_id) = make_decoder(self.format.as_ref())?;
                    self.decoder = decoder;
                    self.track_id = track_id;
                    continue;
                }
                Err(e) => return Err(dec_err(e)),
            };
            if packet.track_id != self.track_id {
                continue;
            }
            match self.decoder.decode(&packet) {
                Ok(buf) => {
                    let channels = buf.spec().channels().count();
                    if channels == 0 || buf.frames() == 0 {
                        continue;
                    }
                    let rate = buf.spec().rate();
                    check_rate(rate)?;
                    buf.copy_to_vec_interleaved(&mut self.samples);
                    // Symphonia 0.6 no longer clips its float output; the reference decodes to
                    // 16-bit PCM (ov_read in LLVorbisDecodeState::decodeSection), which clips at
                    // full scale, so overshooting peaks are clamped the same way.
                    for s in &mut self.samples {
                        *s = s.clamp(-1.0, 1.0);
                    }
                    return Ok(Some((channels, rate)));
                }
                Err(SymError::DecodeError(e)) => {
                    log::debug!("skipping undecodable audio packet: {e}");
                }
                Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Ok(None);
                }
                Err(e) => return Err(dec_err(e)),
            }
        }
    }

    /// Source samples (frames x channels) the container announces for the
    /// decoded track, when it says (ov_pcm_total in the Legacy decoder).
    fn declared_samples(&self) -> Option<u64> {
        let track = self.format.tracks().iter().find(|t| t.id == self.track_id)?;
        let frames = track.num_frames?;
        let channels = track
            .codec_params
            .as_ref()
            .and_then(|p| p.audio())
            .and_then(|a| a.channels.as_ref())
            .map_or(1, |c| c.count().max(1));
        Some(frames.saturating_mul(channels as u64))
    }

    /// "Artist - Title" from in-stream tags (Ogg/Vorbis comments), if any.
    pub(crate) fn tag_title(&mut self) -> Option<String> {
        let mut meta = self.format.metadata();
        let rev = meta.skip_to_latest()?;
        let mut title = None;
        let mut artist = None;
        for tag in &rev.media.tags {
            match &tag.std {
                Some(StandardTag::TrackTitle(t)) => title = Some(t.to_string()),
                Some(StandardTag::Artist(a)) => artist = Some(a.to_string()),
                _ => {}
            }
        }
        match (artist, title) {
            (Some(a), Some(t)) => Some(format!("{a} - {t}")),
            (None, Some(t)) => Some(t),
            _ => None,
        }
    }
}

fn make_decoder(format: &dyn FormatReader) -> Result<(Box<dyn AudioDecoder>, u32), AudioError> {
    let no_track = || AudioError::Decode("no audio track".into());
    let track = format.first_track_known_codec(TrackType::Audio).ok_or_else(no_track)?;
    let params = track.codec_params.as_ref().and_then(|p| p.audio()).ok_or_else(no_track)?;
    if let Some(rate) = params.sample_rate {
        check_rate(rate)?;
    }
    let decoder = symphonia::default::get_codecs()
        .make_audio_decoder(params, &AudioDecoderOptions::default())
        .map_err(dec_err)?;
    Ok((decoder, track.id))
}

/// Maps a stream `Content-Type` to a symphonia extension hint.
pub(crate) fn hint_for_content_type(ct: &str) -> Option<&'static str> {
    match ct {
        "audio/mpeg" | "audio/mp3" | "audio/x-mpeg" | "audio/mpeg3" | "audio/x-mp3" => Some("mp3"),
        "audio/aac" | "audio/aacp" | "audio/x-aac" | "audio/x-aacp" => Some("aac"),
        "application/ogg" | "audio/ogg" | "audio/vorbis" | "audio/x-ogg" | "audio/x-vorbis" => Some("ogg"),
        "audio/wav" | "audio/x-wav" | "audio/wave" => Some("wav"),
        _ => None,
    }
}

/// Maps a URL path's extension to a hint.
pub(crate) fn hint_for_path(path: &str) -> Option<&'static str> {
    let ext = path.rsplit('.').next()?.to_ascii_lowercase();
    match ext.as_str() {
        "mp3" => Some("mp3"),
        "aac" | "aacp" => Some("aac"),
        "ogg" | "oga" => Some("ogg"),
        "wav" => Some("wav"),
        _ => None,
    }
}

/// A fully decoded sound, stored as interleaved stereo at the engine rate,
/// ready for [`crate::AudioEngine::play_clip`]. Cheap to clone.
#[derive(Clone)]
pub struct SoundClip {
    pub(crate) samples: Arc<[f32]>,
}

impl std::fmt::Debug for SoundClip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SoundClip").field("duration", &self.duration()).finish()
    }
}

/// Longest sound asset accepted, in source samples (frames x channels):
/// LLVORBIS_CLIP_REJECT_SIZE / 2 (llaudio/llvorbisencode.h), three times
/// the 30 s stereo upload at 44.1 kHz, the same on every grid. That is 90 s
/// of stereo or 180 s of mono at 44.1 kHz; OpenSim's 60 s uploads fit.
pub(crate) const MAX_CLIP_SOURCE_SAMPLES: usize = 3 * 30 * 44_100 * 2;

/// Longest decoded clip (LLVORBIS_CLIP_REJECT_SAMPLES at 44.1 kHz): a low
/// sample rate does not stretch the source limit into a longer clip once
/// resampled to the engine rate.
pub(crate) const MAX_CLIP_SECONDS: usize = 180;

/// Port of LLVorbisDecodeState::finishDecode (indra/llaudio/llaudiodecodemgr.cpp):
/// fade the clip's edges to suppress Vorbis ringing and hard start/loop steps.
/// The reference uses 128 mono samples at 44.1 kHz (about 2.9 ms), capped at
/// a quarter of the clip. Here both stereo channels share a frame envelope
/// after resampling, including the fade to zero at the last frame.
fn fade_clip_edges(samples: &mut [f32]) {
    let frames = samples.len() / 2;
    let fade = (128 * ENGINE_SAMPLE_RATE as usize / 44_100).min(frames / 4);
    for i in 0..fade {
        let gain = i as f32 / fade as f32;
        for side in 0..2 {
            samples[2 * i + side] *= gain;
            samples[2 * (frames - 1 - i) + side] *= gain;
        }
    }
}

impl SoundClip {
    /// Decodes an in-memory file (Ogg Vorbis — the Second Life sound asset
    /// format —, WAV, MP3 or ADTS AAC). A sound longer than the Legacy
    /// Viewer accepts is refused, as soon as the container announces it or
    /// the decoding passes the limit.
    pub fn decode(bytes: &[u8]) -> Result<SoundClip, AudioError> {
        Self::decode_limited(bytes, MAX_CLIP_SOURCE_SAMPLES, MAX_CLIP_SECONDS)
    }

    /// Decodes a sound asset of the grid: Ogg Vorbis only, the one format
    /// LLVorbisDecodeState::initDecode opens (ov_open_callbacks,
    /// indra/llaudio/llaudiodecodemgr.cpp), with the limits of
    /// [`SoundClip::decode`].
    pub fn decode_asset(bytes: &[u8]) -> Result<SoundClip, AudioError> {
        let stream = DecodeStream::open(Box::new(Cursor::new(bytes.to_vec())), Some("ogg"))?;
        // The probe reads the content, never the hint (symphonia 0.6).
        if stream.format.format_info().format != FORMAT_ID_OGG {
            return Err(AudioError::Decode("sound asset is not Ogg".into()));
        }
        Self::decode_stream(stream, MAX_CLIP_SOURCE_SAMPLES, MAX_CLIP_SECONDS)
    }

    /// [`SoundClip::decode`] with explicit limits: `max_source` source
    /// samples (frames x channels) and `max_seconds` once decoded.
    fn decode_limited(bytes: &[u8], max_source: usize, max_seconds: usize) -> Result<SoundClip, AudioError> {
        let stream = DecodeStream::open(Box::new(Cursor::new(bytes.to_vec())), None)?;
        Self::decode_stream(stream, max_source, max_seconds)
    }

    fn decode_stream(mut stream: DecodeStream, max_source: usize, max_seconds: usize) -> Result<SoundClip, AudioError> {
        let too_long = || AudioError::Decode("sound is too long".into());
        if stream.declared_samples().is_some_and(|n| n > max_source as u64) {
            return Err(too_long());
        }
        let mut stereo = Vec::new();
        let mut out = Vec::new();
        let mut resampler: Option<(u32, Resampler)> = None;
        let limit = max_seconds * ENGINE_SAMPLE_RATE as usize * 2;
        let mut source = 0usize;
        while let Some(chunk) = stream.next_chunk()? {
            source += chunk.samples.len();
            if source > max_source {
                return Err(too_long());
            }
            stereo.clear();
            to_stereo(chunk.samples, chunk.channels, &mut stereo);
            if chunk.rate == ENGINE_SAMPLE_RATE {
                out.extend_from_slice(&stereo);
            } else {
                if resampler.as_ref().is_none_or(|(r, _)| *r != chunk.rate) {
                    resampler = Some((chunk.rate, Resampler::new(chunk.rate, ENGINE_SAMPLE_RATE, 2)));
                }
                if let Some((_, rs)) = resampler.as_mut() {
                    // One frame past what the limit leaves is enough to
                    // refuse the clip; the rest of the packet is not resampled.
                    let budget = limit.saturating_sub(out.len()) / 2 + 1;
                    rs.push(&stereo);
                    rs.pull(&mut out, budget);
                }
            }
            if out.len() > limit {
                return Err(too_long());
            }
        }
        if out.is_empty() {
            return Err(AudioError::Decode("no audio decoded".into()));
        }
        fade_clip_edges(&mut out);
        Ok(SoundClip { samples: out.into() })
    }

    /// Builds a clip from raw interleaved f32 PCM (`channels` >= 1), made
    /// by the viewer itself (synthesized or imported sounds). A rate outside
    /// [`SAMPLE_RATES`] is taken as its nearest bound; 0 is the engine rate.
    pub fn from_pcm(samples: &[f32], channels: u16, sample_rate: u32) -> SoundClip {
        let mut stereo = Vec::new();
        to_stereo(samples, usize::from(channels), &mut stereo);
        let samples: Arc<[f32]> = if sample_rate == ENGINE_SAMPLE_RATE || sample_rate == 0 {
            stereo.into()
        } else {
            let rate = sample_rate.clamp(*SAMPLE_RATES.start(), *SAMPLE_RATES.end());
            let mut out = Vec::new();
            Resampler::new(rate, ENGINE_SAMPLE_RATE, 2).process(&stereo, &mut out);
            out.into()
        };
        SoundClip { samples }
    }

    /// Length of the clip.
    pub fn duration(&self) -> std::time::Duration {
        std::time::Duration::from_secs_f64(self.samples.len() as f64 / 2.0 / f64::from(ENGINE_SAMPLE_RATE))
    }

    /// Memory held by the decoded samples.
    pub fn memory_bytes(&self) -> usize {
        std::mem::size_of_val(&*self.samples)
    }

    /// Whether another copy of this clip (a voice of the mixer, playing
    /// it) shares its samples.
    pub fn is_shared(&self) -> bool {
        Arc::strong_count(&self.samples) > 1
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// 16-bit PCM WAV file, generated in memory.
    pub(crate) fn wav_bytes(rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
        let data_len = (samples.len() * 2) as u32;
        let mut v = Vec::new();
        v.extend(b"RIFF");
        v.extend((36 + data_len).to_le_bytes());
        v.extend(b"WAVEfmt ");
        v.extend(16u32.to_le_bytes());
        v.extend(1u16.to_le_bytes());
        v.extend(channels.to_le_bytes());
        v.extend(rate.to_le_bytes());
        v.extend((rate * u32::from(channels) * 2).to_le_bytes());
        v.extend((channels * 2).to_le_bytes());
        v.extend(16u16.to_le_bytes());
        v.extend(b"data");
        v.extend(data_len.to_le_bytes());
        for s in samples {
            v.extend(s.to_le_bytes());
        }
        v
    }

    /// Silent MPEG-1 Layer III frames (44.1 kHz, 128 kbit/s, mono): a valid
    /// header, all-zero side information and main data.
    pub(crate) fn mp3_silence(frames: usize) -> Vec<u8> {
        const FRAME_LEN: usize = 417; // 144 * 128000 / 44100
        let mut v = Vec::with_capacity(frames * FRAME_LEN);
        for _ in 0..frames {
            let start = v.len();
            v.extend([0xFF, 0xFB, 0x90, 0xC0]);
            v.resize(start + FRAME_LEN, 0);
        }
        v
    }

    struct BitWriter {
        bytes: Vec<u8>,
        bit: usize,
    }

    impl BitWriter {
        fn put(&mut self, value: u32, bits: usize) {
            for i in (0..bits).rev() {
                if self.bit.is_multiple_of(8) {
                    self.bytes.push(0);
                }
                if (value >> i) & 1 == 1 {
                    let last = self.bytes.len() - 1;
                    self.bytes[last] |= 0x80 >> (self.bit % 8);
                }
                self.bit += 1;
            }
        }
    }

    /// Silent AAC-LC frames in ADTS (44.1 kHz mono). Each raw data block is a
    /// single channel element with max_sfb = 0, followed by ID_END.
    pub(crate) fn adts_silence(frames: usize) -> Vec<u8> {
        let mut payload = BitWriter { bytes: Vec::new(), bit: 0 };
        payload.put(0, 3); // ID_SCE
        payload.put(0, 4); // element_instance_tag
        payload.put(100, 8); // global_gain
        payload.put(0, 1); // ics_reserved_bit
        payload.put(0, 2); // window_sequence = ONLY_LONG
        payload.put(0, 1); // window_shape
        payload.put(0, 6); // max_sfb
        payload.put(0, 1); // predictor_data_present
        payload.put(0, 1); // pulse_data_present
        payload.put(0, 1); // tns_data_present
        payload.put(0, 1); // gain_control_data_present
        payload.put(7, 3); // ID_END
        let raw = payload.bytes;

        let frame_len = 7 + raw.len();
        let mut out = Vec::new();
        for _ in 0..frames {
            let mut h = BitWriter { bytes: Vec::new(), bit: 0 };
            h.put(0xFFF, 12); // syncword
            h.put(0, 1); // MPEG-4
            h.put(0, 2); // layer
            h.put(1, 1); // protection_absent
            h.put(1, 2); // profile: AAC LC
            h.put(4, 4); // 44100 Hz
            h.put(0, 1); // private
            h.put(1, 3); // channel configuration: mono
            h.put(0, 4); // original/home/copyright bits
            h.put(frame_len as u32, 13);
            h.put(0x7FF, 11); // buffer fullness (VBR)
            h.put(0, 2); // one raw data block
            out.extend(h.bytes);
            out.extend(&raw);
        }
        out
    }

    #[test]
    fn decodes_generated_wav_and_resamples() {
        let samples: Vec<i16> = (0..22_050).map(|i| ((i as f32 * 0.05).sin() * 10_000.0) as i16).collect();
        let wav = wav_bytes(22_050, 1, &samples);
        let clip = SoundClip::decode(&wav).unwrap();
        let secs = clip.duration().as_secs_f64();
        assert!((secs - 1.0).abs() < 0.01, "{secs}");
        // Stereo duplication of a mono source.
        assert_eq!(clip.samples[1000], clip.samples[1001]);
        let peak = clip.samples.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((0.25..0.35).contains(&peak), "{peak}");
    }

    #[test]
    fn decoded_clip_edges_do_not_step_from_silence_or_at_a_loop_seam() {
        // A clip whose encoded waveform starts and ends away from zero:
        // silence -> clip, clip -> silence and loop wraps must not pop.
        for rate in [22_050, 44_100, 48_000] {
            for channels in [1, 2] {
                let pcm: Vec<i16> = (0..rate / 10)
                    .flat_map(|_| [16_384, -8_192].into_iter().take(channels as usize))
                    .collect();
                let clip = SoundClip::decode(&wav_bytes(rate, channels, &pcm)).unwrap();
                let frames = clip.samples.as_chunks::<2>().0;
                assert_eq!(frames[0], [0.0, 0.0], "start at {rate} Hz, {channels} channels");
                assert_eq!(frames[frames.len() - 1], [0.0, 0.0], "end at {rate} Hz, {channels} channels");
                for side in 0..2 {
                    let jump = frames.windows(2).map(|w| (w[1][side] - w[0][side]).abs()).fold(0.0f32, f32::max);
                    assert!(jump < 0.01, "step {jump} at {rate} Hz, {channels} channels");
                }
                // The main part of a click keeps its level and stereo balance.
                let middle = frames[frames.len() / 2];
                assert_eq!(middle, if channels == 1 { [0.5, 0.5] } else { [0.5, -0.25] });
            }
        }
    }

    #[test]
    fn decoded_short_clips_keep_their_middle_and_raw_pcm_keeps_its_edges() {
        for frames in [1, 2, 3, 4, 8, 32, 128] {
            let clip = SoundClip::decode(&wav_bytes(48_000, 1, &vec![16_384; frames])).unwrap();
            assert_eq!(clip.samples.len(), frames * 2);
            assert_eq!(clip.samples[frames / 2 * 2], 0.5);
            assert!(clip.samples.iter().all(|s| s.is_finite() && (0.0..=0.5).contains(s)));
            if frames >= 4 {
                assert_eq!(clip.samples[0], 0.0);
                assert_eq!(clip.samples[frames * 2 - 1], 0.0);
            }
        }
        // Locally synthesized sounds already shape their envelopes.
        let clip = SoundClip::from_pcm(&[0.5; 1_000], 1, 48_000);
        assert!(clip.samples.iter().all(|&s| s == 0.5));
    }

    #[test]
    fn decoded_vorbis_clip_has_no_start_stop_or_loop_step() {
        // Own synthetic cosine, deliberately cut away from zero. Vorbis
        // ringing leaves a large first sample, just as a hard-cut click can.
        let clip = SoundClip::decode(include_bytes!("../tests/data/abrupt-tone.ogg")).unwrap();
        let frames = clip.samples.as_chunks::<2>().0;
        let first = frames[0][0];
        let last = frames[frames.len() - 1][0];
        assert_eq!(first, 0.0, "start impulse: {first}");
        assert_eq!(last, 0.0, "stop impulse: {last}");
        assert_eq!(first - last, 0.0, "loop impulse");
        assert!(frames.iter().all(|f| f[0] == f[1]));
        let peak = frames.iter().map(|f| f[0].abs()).fold(0.0f32, f32::max);
        assert!((0.5..0.7).contains(&peak), "body peak: {peak}");
        // A 440 Hz tone has a natural per-sample step below 0.04 at
        // 48 kHz; the short fades must not introduce a sharper transition.
        let step = frames.windows(2).map(|w| (w[1][0] - w[0][0]).abs()).fold(0.0f32, f32::max);
        assert!(step < 0.05, "internal step: {step}");
        // Stream packets (radio/voice paths) must not get a per-packet
        // envelope: those sources already manage playback transitions.
        let mut stream = DecodeStream::open(Box::new(Cursor::new(include_bytes!("../tests/data/abrupt-tone.ogg"))), None).unwrap();
        assert!(stream.next_chunk().unwrap().unwrap().samples[0].abs() > 0.5);
    }

    #[test]
    fn decodes_mp3_frames() {
        let data = mp3_silence(30);
        let mut s = DecodeStream::open(Box::new(Cursor::new(data)), Some("mp3")).unwrap();
        let mut frames = 0;
        let mut rate = 0;
        while let Some(c) = s.next_chunk().unwrap() {
            assert_eq!(c.channels, 1);
            assert!(c.samples.iter().all(|v| v.abs() < 1e-6));
            frames += c.samples.len();
            rate = c.rate;
        }
        assert_eq!(rate, 44_100);
        assert!(frames >= 1152 * 25, "{frames}");
    }

    #[test]
    fn decodes_adts_aac_frames() {
        let data = adts_silence(20);
        let mut s = DecodeStream::open(Box::new(Cursor::new(data)), Some("aac")).unwrap();
        let mut frames = 0;
        let mut rate = 0;
        while let Some(c) = s.next_chunk().unwrap() {
            assert!(c.samples.iter().all(|v| v.abs() < 1e-6));
            frames += c.samples.len() / c.channels;
            rate = c.rate;
        }
        assert_eq!(rate, 44_100);
        assert!(frames >= 1024 * 15, "{frames}");
    }

    #[test]
    fn clip_length_limit_matches_the_legacy_viewer() {
        // 90 s of stereo or 180 s of mono at 44.1 kHz; OpenSim's 60 s stereo uploads fit
        assert_eq!(MAX_CLIP_SOURCE_SAMPLES, 90 * 44_100 * 2);
        assert_eq!(MAX_CLIP_SOURCE_SAMPLES, 180 * 44_100);
        const { assert!(60 * 48_000 * 2 <= MAX_CLIP_SOURCE_SAMPLES) };
        const { assert!(91 * 44_100 * 2 > MAX_CLIP_SOURCE_SAMPLES) };
    }

    #[test]
    fn a_declared_length_over_the_limit_is_refused_before_decoding() {
        let wav = wav_bytes(44_100, 2, &[0; 4_000]);
        let stream = DecodeStream::open(Box::new(Cursor::new(wav.clone())), None).unwrap();
        assert_eq!(stream.declared_samples(), Some(4_000));
        assert!(SoundClip::decode_limited(&wav, 4_000, MAX_CLIP_SECONDS).is_ok());
        assert!(SoundClip::decode_limited(&wav, 3_999, MAX_CLIP_SECONDS).is_err());
    }

    #[test]
    fn decoding_stops_once_past_the_limit() {
        // MP3 frames read without seeking: no length announced, only the decoded samples count
        let open = || {
            let source = symphonia::core::io::ReadOnlySource::new(Cursor::new(mp3_silence(30)));
            DecodeStream::open(Box::new(source), Some("mp3")).unwrap()
        };
        assert_eq!(open().declared_samples(), None);
        assert!(SoundClip::decode_stream(open(), 1_152 * 30, MAX_CLIP_SECONDS).is_ok());
        assert!(SoundClip::decode_stream(open(), 1_152 * 5, MAX_CLIP_SECONDS).is_err());
    }

    #[test]
    fn a_low_sample_rate_does_not_stretch_the_limit() {
        // 2 s at 8 kHz: few source samples, but 2 s once resampled
        let wav = wav_bytes(8_000, 1, &[0; 16_000]);
        assert!(SoundClip::decode_limited(&wav, MAX_CLIP_SOURCE_SAMPLES, 3).is_ok());
        assert!(SoundClip::decode_limited(&wav, MAX_CLIP_SOURCE_SAMPLES, 1).is_err());
    }

    #[test]
    fn only_the_supported_sample_rates_decode() {
        for rate in [1, 100, 7_349, 192_001, 1_000_000] {
            let wav = wav_bytes(rate, 1, &[0; 4_096]);
            assert!(SoundClip::decode(&wav).is_err(), "{rate} Hz");
            assert!(DecodeStream::open(Box::new(Cursor::new(wav)), None).is_err(), "{rate} Hz");
            assert!(check_rate(rate).is_err(), "{rate} Hz");
        }
        for rate in [7_350, 8_000, 11_025, 22_050, 44_100, 96_000, 192_000] {
            let clip = SoundClip::decode(&wav_bytes(rate, 2, &[0; 4_096])).unwrap();
            // The resampler holds back its last two source frames.
            let ratio = f64::from(ENGINE_SAMPLE_RATE) / f64::from(rate);
            let frames = 2_048.0 * ratio;
            let got = clip.samples.len() as f64 / 2.0;
            assert!(
                (got - frames).abs() <= 3.0 * ratio + 2.0,
                "{rate} Hz: {got} frames, {frames} expected"
            );
        }
    }

    #[test]
    fn each_packet_stays_within_the_clip_limit() {
        // 3 s at 8 kHz, six engine frames per source frame: a 2 s limit
        // refuses it, and a pull never goes past the frames it is given.
        let wav = wav_bytes(8_000, 1, &[0; 24_000]);
        assert!(SoundClip::decode_limited(&wav, MAX_CLIP_SOURCE_SAMPLES, 3).is_ok());
        assert!(SoundClip::decode_limited(&wav, MAX_CLIP_SOURCE_SAMPLES, 2).is_err());
        let mut rs = Resampler::new(8_000, ENGINE_SAMPLE_RATE, 2);
        rs.push(&[0.0; 16_000]);
        let mut out = Vec::new();
        assert_eq!(rs.pull(&mut out, 1_001), 1_001);
        assert_eq!(out.len(), 2_002);
    }

    #[test]
    fn sound_assets_are_ogg_only() {
        assert!(SoundClip::decode_asset(include_bytes!("../tests/data/abrupt-tone.ogg")).is_ok());
        assert!(SoundClip::decode_asset(&wav_bytes(44_100, 1, &[0; 4_096])).is_err());
        assert!(SoundClip::decode_asset(&mp3_silence(30)).is_err());
        assert!(SoundClip::decode_asset(b"definitely not audio").is_err());
        // Local files keep every supported format.
        assert!(SoundClip::decode(&wav_bytes(44_100, 1, &[0; 4_096])).is_ok());
        assert!(SoundClip::decode(&mp3_silence(30)).is_ok());
    }

    #[test]
    fn raw_pcm_rates_stay_within_the_supported_range() {
        let clip = SoundClip::from_pcm(&[0.0; 100], 1, 1);
        let bound = SoundClip::from_pcm(&[0.0; 100], 1, *SAMPLE_RATES.start());
        assert_eq!(clip.samples.len(), bound.samples.len());
    }

    #[test]
    fn shared_samples_and_memory() {
        let clip = SoundClip::from_pcm(&[0.0; 4_800], 1, ENGINE_SAMPLE_RATE);
        assert_eq!(clip.memory_bytes(), 4_800 * 2 * 4);
        assert!(!clip.is_shared());
        let voice = clip.samples.clone();
        assert!(clip.is_shared());
        drop(voice);
        assert!(!clip.is_shared());
    }

    #[test]
    fn rejects_garbage() {
        assert!(SoundClip::decode(b"definitely not audio").is_err());
        assert!(SoundClip::decode(&[]).is_err());
    }

    #[test]
    fn content_type_hints() {
        assert_eq!(hint_for_content_type("audio/aacp"), Some("aac"));
        assert_eq!(hint_for_content_type("application/ogg"), Some("ogg"));
        assert_eq!(hint_for_content_type("text/html"), None);
        assert_eq!(hint_for_path("/live/stream.MP3"), Some("mp3"));
        assert_eq!(hint_for_path("/live"), None);
    }
}
