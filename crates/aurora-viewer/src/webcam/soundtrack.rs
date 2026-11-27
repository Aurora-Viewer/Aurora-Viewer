//! Render the source slices that were actually captured, on take time.

use super::take_tools::{AudioPass, audio_command};
use aurora_assets::import_sound::Sound;

pub(super) const RATE: usize = 44_100;
const UPLOAD_SAMPLES: usize = 29 * RATE;

pub(super) fn render(passes: &[AudioPass], [start, end]: [f32; 2]) -> Option<Vec<i16>> {
    let samples = |seconds: f32| (f64::from(seconds) * RATE as f64).round() as usize;
    let frames = samples(end - start);
    if frames == 0 || frames > 60 * RATE {
        return None;
    }
    let mut pcm = vec![0; frames];
    let mut decoded = false;
    for pass in passes {
        let left = start.max(pass.from);
        let right = end.min(pass.end);
        if right <= left {
            continue;
        }
        let offset = pass.offset + (left - pass.from) * pass.rate;
        let dest = samples(left - start).min(frames)..samples(right - start).min(frames);
        let result = audio_command(pass, offset, right - left, RATE as u32, "s16le").output().ok()?;
        if !result.status.success() || result.stdout.len() < 2 {
            return None;
        }
        decoded = true;
        let fade_in = dest.start > 0;
        let fade_out = dest.end < frames;
        let count = dest.len().min(result.stdout.len() / 2);
        let ramp = (RATE / 300).min(count / 2).max(1);
        for (i, (sample, bytes)) in pcm[dest].iter_mut().zip(result.stdout.chunks_exact(2)).enumerate() {
            // A short gain ramp avoids an audible click at a source edit.
            // It keeps both the exact cut time and the number of samples.
            let head = if fade_in { i.min(ramp) } else { ramp };
            let tail = if fade_out { (count - 1 - i).min(ramp) } else { ramp };
            *sample = (i32::from(i16::from_le_bytes([bytes[0], bytes[1]])) * head.min(tail) as i32 / ramp as i32) as i16;
        }
    }
    decoded.then_some(pcm)
}

pub(super) fn to_wav(pcm: &[i16]) -> Vec<u8> {
    let size = (pcm.len() * 2) as u32;
    let mut bytes = Vec::with_capacity(44 + size as usize);
    bytes.extend(b"RIFF");
    bytes.extend((36 + size).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16_u32.to_le_bytes());
    bytes.extend(1_u16.to_le_bytes());
    bytes.extend(1_u16.to_le_bytes());
    bytes.extend((RATE as u32).to_le_bytes());
    bytes.extend((RATE as u32 * 2).to_le_bytes());
    bytes.extend(2_u16.to_le_bytes());
    bytes.extend(16_u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(size.to_le_bytes());
    bytes.extend(pcm.iter().flat_map(|sample| sample.to_le_bytes()));
    bytes
}

pub(super) fn upload_pieces(pcm: &[i16]) -> Vec<(f32, Sound)> {
    pcm.chunks(UPLOAD_SAMPLES)
        .enumerate()
        .map(|(i, samples)| {
            (
                (i * UPLOAD_SAMPLES) as f32 / RATE as f32,
                Sound {
                    source_channels: 1,
                    source_bits: 16,
                    mono: samples.iter().map(|&sample| f32::from(sample) / 32768.0).collect(),
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_and_upload_pieces_share_the_take_clock() {
        let pcm = vec![-1234; 30 * RATE + 110];
        let wav = to_wav(&pcm);
        let decoded = aurora_assets::import_sound::decode_wav(&wav, 60).unwrap();
        assert_eq!(decoded.mono.len(), pcm.len());
        let pieces = upload_pieces(&pcm);
        assert_eq!(pieces.len(), 2);
        assert_eq!(pieces[1].0, 29.0);
        assert_eq!(pieces.iter().map(|(_, p)| p.mono.len()).sum::<usize>(), pcm.len());
        assert_eq!(pieces[0].1.mono, decoded.mono[..29 * RATE]);
    }
}
