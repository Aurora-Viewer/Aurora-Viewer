//! Compressed-audio decoding (symphonia): MP3, AAC (ADTS), Ogg Vorbis, WAV.

use std::io::Cursor;
use std::sync::Arc;

use symphonia::core::audio::{SampleBuffer, SignalSpec};
use symphonia::core::codecs::{CODEC_TYPE_NULL, Decoder, DecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::{MetadataOptions, StandardTagKey};
use symphonia::core::probe::Hint;

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
    decoder: Box<dyn Decoder>,
    track_id: u32,
    sample_buf: Option<(SampleBuffer<f32>, SignalSpec, usize)>,
}

fn dec_err(e: SymError) -> AudioError {
    AudioError::Decode(e.to_string())
}

impl DecodeStream {
    /// `ext` is a file-extension style hint ("mp3", "aac", "ogg", "wav").
    pub(crate) fn open(source: Box<dyn MediaSource>, ext: Option<&str>) -> Result<Self, AudioError> {
        let mss = MediaSourceStream::new(source, Default::default());
        let mut hint = Hint::new();
        if let Some(ext) = ext {
            hint.with_extension(ext);
        }
        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
            .map_err(dec_err)?;
        let format = probed.format;
        let (decoder, track_id) = make_decoder(format.as_ref())?;
        Ok(Self {
            format,
            decoder,
            track_id,
            sample_buf: None,
        })
    }

    /// Next decoded block, or `Ok(None)` at the end of the stream.
    pub(crate) fn next_chunk(&mut self) -> Result<Option<Chunk<'_>>, AudioError> {
        let Some((channels, rate)) = self.decode_next()? else {
            return Ok(None);
        };
        let samples = self.sample_buf.as_ref().map(|(sb, _, _)| sb.samples()).unwrap_or(&[]);
        Ok(Some(Chunk { samples, channels, rate }))
    }

    /// Decodes the next packet into `sample_buf`; returns (channels, rate).
    fn decode_next(&mut self) -> Result<Option<(usize, u32)>, AudioError> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(p) => p,
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
            if packet.track_id() != self.track_id {
                continue;
            }
            match self.decoder.decode(&packet) {
                Ok(buf) => {
                    let spec = *buf.spec();
                    let cap = buf.capacity();
                    let channels = spec.channels.count();
                    if channels == 0 || buf.frames() == 0 {
                        continue;
                    }
                    let reuse = matches!(&self.sample_buf, Some((_, s, c)) if *s == spec && *c >= cap);
                    if !reuse {
                        self.sample_buf = Some((SampleBuffer::new(cap as u64, spec), spec, cap));
                    }
                    if let Some((sb, _, _)) = self.sample_buf.as_mut() {
                        sb.copy_interleaved_ref(buf);
                        return Ok(Some((channels, spec.rate)));
                    }
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

    /// "Artist - Title" from in-stream tags (Ogg/Vorbis comments), if any.
    pub(crate) fn tag_title(&mut self) -> Option<String> {
        let mut meta = self.format.metadata();
        let rev = meta.skip_to_latest()?;
        let mut title = None;
        let mut artist = None;
        for tag in rev.tags() {
            match tag.std_key {
                Some(StandardTagKey::TrackTitle) => title = Some(tag.value.to_string()),
                Some(StandardTagKey::Artist) => artist = Some(tag.value.to_string()),
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

fn make_decoder(format: &dyn FormatReader) -> Result<(Box<dyn Decoder>, u32), AudioError> {
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| AudioError::Decode("no audio track".into()))?;
    let decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
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

/// Clips longer than this are refused (protects against hostile assets).
const MAX_CLIP_SECONDS: usize = 600;

impl SoundClip {
    /// Decodes an in-memory file (Ogg Vorbis — the Second Life sound asset
    /// format —, WAV, MP3 or ADTS AAC).
    pub fn decode(bytes: &[u8]) -> Result<SoundClip, AudioError> {
        let mut stream = DecodeStream::open(Box::new(Cursor::new(bytes.to_vec())), None)?;
        let mut stereo = Vec::new();
        let mut out = Vec::new();
        let mut resampler: Option<(u32, Resampler)> = None;
        let limit = MAX_CLIP_SECONDS * ENGINE_SAMPLE_RATE as usize * 2;
        while let Some(chunk) = stream.next_chunk()? {
            stereo.clear();
            to_stereo(chunk.samples, chunk.channels, &mut stereo);
            if chunk.rate == ENGINE_SAMPLE_RATE {
                out.extend_from_slice(&stereo);
            } else {
                if resampler.as_ref().is_none_or(|(r, _)| *r != chunk.rate) {
                    resampler = Some((chunk.rate, Resampler::new(chunk.rate, ENGINE_SAMPLE_RATE, 2)));
                }
                if let Some((_, rs)) = resampler.as_mut() {
                    rs.process(&stereo, &mut out);
                }
            }
            if out.len() > limit {
                return Err(AudioError::Decode("sound is too long".into()));
            }
        }
        if out.is_empty() {
            return Err(AudioError::Decode("no audio decoded".into()));
        }
        Ok(SoundClip { samples: out.into() })
    }

    /// Builds a clip from raw interleaved f32 PCM (`channels` >= 1).
    pub fn from_pcm(samples: &[f32], channels: u16, sample_rate: u32) -> SoundClip {
        let mut stereo = Vec::new();
        to_stereo(samples, usize::from(channels), &mut stereo);
        let samples: Arc<[f32]> = if sample_rate == ENGINE_SAMPLE_RATE || sample_rate == 0 {
            stereo.into()
        } else {
            let mut out = Vec::new();
            Resampler::new(sample_rate, ENGINE_SAMPLE_RATE, 2).process(&stereo, &mut out);
            out.into()
        };
        SoundClip { samples }
    }

    /// Length of the clip.
    pub fn duration(&self) -> std::time::Duration {
        std::time::Duration::from_secs_f64(self.samples.len() as f64 / 2.0 / f64::from(ENGINE_SAMPLE_RATE))
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
