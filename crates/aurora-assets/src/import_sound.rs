//! WAV validation and Vorbis mono conversion, following
//! indra/llaudio/llvorbisencode.cpp (Firestorm, originally LGPL 2.1).
//! Decode bounded RIFF chunks without trusting the declared container size.

use crate::AssetError;
use std::num::{NonZeroU8, NonZeroU32};

#[derive(Debug, Clone)]
pub struct Sound {
    pub source_channels: u16,
    pub source_bits: u16,
    pub mono: Vec<f32>,
}
impl Sound {
    pub fn duration(&self) -> f32 {
        self.mono.len() as f32 / 44100.0
    }
    pub fn encode(&self) -> Result<Vec<u8>, AssetError> {
        if self.mono.is_empty() || self.mono.len() > 44100 * 60 || !self.mono.iter().all(|n| n.is_finite()) {
            return Err(AssetError::invalid("invalid PCM samples"));
        }
        let mut output = Vec::new();
        {
            // Constants are nonzero by construction; no external values here.
            let mut builder = vorbis_rs::VorbisEncoderBuilder::new(
                NonZeroU32::new(44100).expect("constant sample rate"),
                NonZeroU8::new(1).expect("mono"),
                &mut output,
            )
            .map_err(|e| AssetError::invalid(e.to_string()))?;
            builder.bitrate_management_strategy(vorbis_rs::VorbisBitrateManagementStrategy::QualityVbr { target_quality: 0.05 });
            let mut encoder = builder.build().map_err(|e| AssetError::invalid(e.to_string()))?;
            for chunk in self.mono.chunks(4096) {
                encoder
                    .encode_audio_block([chunk])
                    .map_err(|e| AssetError::invalid(e.to_string()))?;
            }
            encoder.finish().map_err(|e| AssetError::invalid(e.to_string()))?;
        }
        Ok(output)
    }
}

pub fn decode_wav(data: &[u8], max_seconds: u32) -> Result<Sound, AssetError> {
    // Aurora's editor can split a bounded long source into valid upload assets.
    if data.len() > 16 * 1024 * 1024 || !(1..=600).contains(&max_seconds) {
        return Err(AssetError::TooLarge("sound file"));
    }
    if data.get(..4) != Some(b"RIFF") || data.get(8..12) != Some(b"WAVE") {
        return Err(AssetError::invalid("expected RIFF WAV"));
    }
    let u16_at = |slice: &[u8], offset| {
        slice
            .get(offset..offset + 2)
            .map(|v| u16::from_le_bytes([v[0], v[1]]))
            .ok_or(AssetError::Truncated("WAV field"))
    };
    let u32_at = |slice: &[u8], offset| {
        slice
            .get(offset..offset + 4)
            .map(|v| u32::from_le_bytes([v[0], v[1], v[2], v[3]]))
            .ok_or(AssetError::Truncated("WAV field"))
    };
    let container_end = (u32_at(data, 4)? as usize)
        .checked_add(8)
        .filter(|end| *end <= data.len())
        .ok_or(AssetError::Truncated("RIFF"))?;
    let mut pos: usize = 12;
    let mut format = None;
    let mut samples = None;
    while pos + 8 <= container_end {
        let kind = &data[pos..pos + 4];
        let size = u32_at(data, pos + 4)? as usize;
        let end = (pos + 8)
            .checked_add(size)
            .filter(|end| *end <= container_end)
            .ok_or(AssetError::Truncated("WAV chunk"))?;
        let chunk = &data[pos + 8..end];
        match kind {
            b"fmt " => {
                if format.is_some() || size < 16 {
                    return Err(AssetError::invalid("invalid WAV format chunk"));
                }
                let encoding = u16_at(chunk, 0)?;
                let channels = u16_at(chunk, 2)?;
                let rate = u32_at(chunk, 4)?;
                let byte_rate = u32_at(chunk, 8)?;
                let align = u16_at(chunk, 12)?;
                let bits = u16_at(chunk, 14)?;
                if encoding != 1 || !matches!(channels, 1 | 2) || rate != 44100 || !matches!(bits, 8 | 16) {
                    return Err(AssetError::invalid("WAV must be PCM, mono/stereo, 44100 Hz, 8/16-bit"));
                }
                if align != channels * (bits / 8) || byte_rate != rate * u32::from(align) {
                    return Err(AssetError::invalid("WAV block alignment or byte rate"));
                }
                format = Some((channels, bits, align));
            }
            b"data" => {
                if samples.is_some() {
                    return Err(AssetError::invalid("multiple WAV data chunks"));
                }
                samples = Some(chunk);
            }
            _ => {}
        }
        pos = end + (size & 1);
    }
    let (channels, bits, align) = format.ok_or_else(|| AssetError::invalid("WAV has no format"))?;
    let samples = samples.ok_or_else(|| AssetError::invalid("WAV has no sample data"))?;
    if samples.is_empty() || samples.len() % usize::from(align) != 0 {
        return Err(AssetError::invalid("incomplete PCM frame"));
    }
    let frames = samples.len() / usize::from(align);
    if frames > 44100 * max_seconds as usize {
        return Err(AssetError::TooLarge("sound duration"));
    }
    let mono = samples
        .chunks_exact(usize::from(align))
        .map(|frame| {
            let sample = |channel: usize| {
                if bits == 8 {
                    (frame[channel] as f32 - 128.0) / 128.0
                } else {
                    let i = channel * 2;
                    i16::from_le_bytes([frame[i], frame[i + 1]]) as f32 / 32768.0
                }
            };
            if channels == 1 { sample(0) } else { (sample(0) + sample(1)) * 0.5 }
        })
        .collect();
    Ok(Sound {
        source_channels: channels,
        source_bits: bits,
        mono,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wav(channels: u16, bits: u16, samples: &[u8]) -> Vec<u8> {
        let align = channels * bits / 8;
        let mut v = Vec::from(*b"RIFF");
        v.extend_from_slice(&(36u32 + samples.len() as u32).to_le_bytes());
        v.extend_from_slice(b"WAVEfmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&channels.to_le_bytes());
        v.extend_from_slice(&44100u32.to_le_bytes());
        v.extend_from_slice(&(44100u32 * u32::from(align)).to_le_bytes());
        v.extend_from_slice(&align.to_le_bytes());
        v.extend_from_slice(&bits.to_le_bytes());
        v.extend_from_slice(b"data");
        v.extend_from_slice(&(samples.len() as u32).to_le_bytes());
        v.extend_from_slice(samples);
        v
    }
    #[test]
    fn stereo_is_averaged_and_pcm_unsigned_silence_stays_silent() {
        let sound = decode_wav(&wav(2, 16, &[0xff, 0x7f, 1, 0x80]), 30).expect("stereo");
        assert_eq!(sound.mono, vec![0.0]);
        let sound = decode_wav(&wav(1, 8, &[128; 4410]), 30).expect("mono");
        assert_eq!(sound.duration(), 0.1);
        assert!(sound.mono.iter().all(|n| *n == 0.0));
        assert!(sound.encode().expect("vorbis").starts_with(b"OggS"));
    }
    #[test]
    fn rejects_truncation_bad_alignment_and_long_sound() {
        assert!(decode_wav(&wav(2, 16, &[0; 3]), 30).is_err());
        let mut bad = wav(1, 8, &[128; 10]);
        bad[24..28].copy_from_slice(&48000u32.to_le_bytes());
        assert!(decode_wav(&bad, 30).is_err());
        assert!(decode_wav(&wav(1, 8, &vec![128; 44100 * 30 + 2]), 30).is_err());
        let mut bad = wav(1, 8, &[128; 10]);
        bad.truncate(43);
        assert!(decode_wav(&bad, 30).is_err());
    }
}
