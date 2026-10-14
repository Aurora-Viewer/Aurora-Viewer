//! Non-destructive sound edits. PCM preparation remains on import workers;
//! the frame thread draws only the cached amplitude envelope.
use super::*;
use std::{ops::Range, time::Instant};

pub const RATE: u32 = 44100;
pub const MAX_GAIN: f32 = 4.0;
pub const SOURCE_LIMIT: u32 = 600;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    pub gain: f32,
    pub start: f32,
    pub end: Option<f32>,
    pub segment: Option<f32>,
    /// Frozen queue index; the editor and audition use the whole selection.
    pub piece: Option<usize>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            gain: 1.0,
            start: 0.0,
            end: None,
            segment: None,
            piece: None,
        }
    }
}
impl Options {
    fn selection(self, sound: &Sound) -> Result<Range<usize>, String> {
        let end = self.end.unwrap_or(sound.duration());
        if !self.gain.is_finite()
            || !(0.0..=MAX_GAIN).contains(&self.gain)
            || !self.start.is_finite()
            || !end.is_finite()
            || self.start < 0.0
            || end <= self.start
            || end > sound.duration() + 0.0001
        {
            return Err(aurora_i18n::tr!("import-sound-edit-invalid").into());
        }
        let start = (self.start * RATE as f32).round() as usize;
        let end = ((end * RATE as f32).round() as usize).min(sound.mono.len());
        if start >= end {
            return Err(aurora_i18n::tr!("import-sound-edit-invalid").into());
        }
        Ok(start..end)
    }
    fn chunk(self) -> Result<Option<usize>, String> {
        match self.segment {
            Some(seconds) if seconds.is_finite() && (0.1..=60.0).contains(&seconds) => {
                Ok(Some((f64::from(seconds) * f64::from(RATE)).floor() as usize))
            }
            Some(_) => Err(aurora_i18n::tr!("import-sound-edit-invalid").into()),
            None => Ok(None),
        }
    }
    pub fn pieces(self, sound: &Sound) -> Result<usize, String> {
        let range = self.selection(sound)?;
        Ok(self.chunk()?.map_or(1, |chunk| range.len().div_ceil(chunk)))
    }
    pub fn range(self, sound: &Sound) -> Result<Range<usize>, String> {
        let range = self.selection(sound)?;
        let Some(index) = self.piece else { return Ok(range) };
        let chunk = self.chunk()?.unwrap_or(range.len());
        let start = index.checked_mul(chunk).and_then(|offset| range.start.checked_add(offset));
        let Some(start) = start.filter(|start| *start < range.end) else {
            return Err(aurora_i18n::tr!("import-sound-edit-invalid").into());
        };
        Ok(start..start.saturating_add(chunk).min(range.end))
    }
    pub fn importable(self, sound: &Sound, limit: u32) -> bool {
        self.selection(sound).is_ok_and(|range| {
            self.chunk()
                .is_ok_and(|chunk| chunk.map_or(range.len(), |chunk| chunk.min(range.len())) <= limit as usize * RATE as usize)
        })
    }
    pub fn duration(self, sound: &Sound) -> f32 {
        self.range(sound).map_or(0.0, |range| range.len() as f32 / RATE as f32)
    }
    pub fn render(self, sound: &Sound) -> Result<Sound, String> {
        let range = self.range(sound)?;
        Ok(Sound {
            source_channels: sound.source_channels,
            source_bits: sound.source_bits,
            mono: sound.mono[range]
                .iter()
                .map(|sample| (sample * self.gain).clamp(-1.0, 1.0))
                .collect(),
        })
    }
}

#[derive(Default)]
pub struct Editor {
    pub options: Options,
    pub peaks: Arc<[f32]>,
    pub started: Option<Instant>,
    pub playing_duration: f32,
    pub preparing: bool,
    /// Acknowledged pieces of the last confirmed edit, retained across retry.
    pub completed: std::collections::HashSet<usize>,
    pub completed_options: Options,
}
impl Editor {
    pub fn new(source: Option<&Source>) -> Self {
        let peaks = if let Some(Source::Sound(sound)) = source {
            let chunk = sound.mono.len().div_ceil(1024).max(1);
            sound
                .mono
                .chunks(chunk)
                .map(|samples| samples.iter().map(|value| value.abs()).fold(0.0_f32, f32::max))
                .collect()
        } else {
            Vec::new()
        };
        Self {
            peaks: peaks.into(),
            ..Default::default()
        }
    }
    pub fn stop(&mut self) {
        self.started = None;
        self.preparing = false;
    }
    pub fn elapsed(&self) -> Option<f32> {
        self.started
            .map(|started| started.elapsed().as_secs_f32())
            .filter(|elapsed| *elapsed < self.playing_duration)
    }
    pub fn pending(&self, sound: &Sound) -> Result<usize, String> {
        let count = self.options.pieces(sound)?;
        Ok(count
            - if self.completed_options == self.options {
                self.completed.len().min(count)
            } else {
                0
            })
    }
}

pub fn piece_name(name: &str, index: usize) -> String {
    let suffix = format!("_{:02}", index + 1);
    let mut end = name.len().min(63 - suffix.len());
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{suffix}", &name[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sound_edits_preserve_the_source_and_use_exact_sample_bounds() {
        let sound = Sound {
            source_channels: 2,
            source_bits: 16,
            mono: (0..RATE).map(|sample| sample as f32 / RATE as f32).collect(),
        };
        let edit = Options {
            gain: 0.5,
            start: 0.25,
            end: Some(0.75),
            ..Default::default()
        };
        let rendered = edit.render(&sound).expect("valid crop");
        assert_eq!(rendered.mono.len(), 22050);
        assert_eq!(rendered.mono[0], 0.125);
        assert_eq!(edit.duration(&sound), 0.5);
        assert_eq!(sound.mono.len(), RATE as usize);
        assert_eq!(Options::default().render(&sound).expect("original").mono, sound.mono);
        assert!(rendered.encode().expect("edited Vorbis").starts_with(b"OggS"));
        for invalid in [
            Options {
                start: 0.8,
                end: Some(0.2),
                ..edit
            },
            Options { gain: f32::NAN, ..edit },
            Options { end: Some(2.0), ..edit },
        ] {
            assert!(invalid.render(&sound).is_err());
        }
    }
    #[test]
    fn boosted_pieces_are_contiguous_bounded_and_keep_the_short_tail() {
        let source = Sound {
            source_channels: 1,
            source_bits: 16,
            mono: vec![0.4; RATE as usize * 8],
        };
        let options = Options {
            gain: 3.0,
            start: 0.25,
            end: Some(7.5),
            segment: Some(2.0),
            ..Default::default()
        };
        assert_eq!(options.pieces(&source).expect("pieces"), 4);
        assert!(options.importable(&source, 30));
        let ranges: Vec<_> = (0..4)
            .map(|index| {
                Options {
                    piece: Some(index),
                    ..options
                }
                .range(&source)
                .expect("piece")
            })
            .collect();
        for pair in ranges.windows(2) {
            assert_eq!(pair[0].end, pair[1].start, "no gaps or duplicated samples");
        }
        assert_eq!(
            ranges.iter().map(Range::len).sum::<usize>(),
            options.range(&source).expect("selection").len()
        );
        assert_eq!(ranges[3].len(), RATE as usize * 5 / 4);
        for index in 0..4 {
            let piece = Options {
                piece: Some(index),
                ..options
            }
            .render(&source)
            .expect("render");
            assert!(piece.duration() <= 2.0);
            assert_eq!(piece.mono[0], 1.0, "gain is baked and the ceiling is enforced");
            assert!(piece.encode().expect("Vorbis").starts_with(b"OggS"));
            assert_eq!(piece_name("Sound", index), format!("Sound_{:02}", index + 1));
        }
        assert_eq!(source.mono[0], 0.4);
        assert_eq!(
            Options {
                gain: 2.0,
                ..Default::default()
            }
            .render(&source)
            .expect("boost")
            .mono[0],
            0.8
        );
        let long = "é".repeat(32);
        assert!(piece_name(&long, 99).len() <= 63);
        for segment in [f32::NAN, 0.0, 0.01, 61.0] {
            assert!(
                Options {
                    segment: Some(segment),
                    ..options
                }
                .pieces(&source)
                .is_err()
            );
        }
        assert!(Options { piece: Some(4), ..options }.render(&source).is_err());
    }
}
