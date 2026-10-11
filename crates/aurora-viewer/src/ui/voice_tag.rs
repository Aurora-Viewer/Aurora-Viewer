//! Speaking outline integrated into avatar name tags.
//!
//! Availability and speaking follow LLVOAvatar::idleUpdateVoiceVisualizer and
//! LLWebRTCVoiceClient::predUpdateOwnVolume (originally LGPL 2.1). The requested
//! presentation replaces the dot / waves with a level-driven outline; listening
//! stays neutral and avatars outside voice get a filled red microphone slash.

use crate::theme::Palette;
use egui::{Color32, Rect, Stroke, StrokeKind};
use std::collections::HashMap;
use uuid::Uuid;

/// Time constants in seconds: react quickly, then fade between syllables.
const ATTACK: f32 = 0.035;
const RELEASE: f32 = 0.10;
const LEVEL_MIN: f32 = 0.2;
const LEVEL_MAX: f32 = 0.7;
pub const ICON_SIZE: f32 = 14.0;
pub const ICON_WIDTH: f32 = ICON_SIZE + 4.0;

struct Envelope {
    opacity: f32,
    level: f32,
    seen: f64,
}

impl Envelope {
    fn new(now: f64) -> Self {
        Self {
            opacity: 0.0,
            level: 0.0,
            seen: now,
        }
    }

    fn update(&mut self, level: f32, speaking: bool, now: f64) {
        let dt = (now - self.seen).max(0.0) as f32;
        self.seen = now;
        let target = if speaking { 1.0 } else { 0.0 };
        let tau = if speaking { ATTACK } else { RELEASE };
        self.opacity += (target - self.opacity) * (1.0 - (-dt / tau).exp());
        if speaking {
            let level = if level.is_finite() { level.clamp(0.0, 1.0) } else { 0.0 };
            self.level += (level - self.level) * (1.0 - (-dt / ATTACK).exp());
        }
        // Preserve the last speech color during release.
        if self.opacity < 0.01 {
            self.opacity = 0.0;
            self.level = 0.0;
        }
    }
}

#[derive(Default)]
pub struct VoiceTags {
    envelopes: HashMap<Uuid, Envelope>,
}

/// Preserve the volume colors of LLVoiceVisualizer: green → amber → red.
fn level_color(p: &Palette, level: f32) -> Color32 {
    let lerp = |a: Color32, b: Color32, t: f32| {
        let t = t.clamp(0.0, 1.0);
        let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
        Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
    };
    if level < 0.45 {
        lerp(p.success, p.amber, (level - LEVEL_MIN) / (0.45 - LEVEL_MIN))
    } else {
        lerp(p.amber, p.danger, (level - 0.45) / (LEVEL_MAX - 0.45))
    }
}

impl VoiceTags {
    /// Draw inside the existing tag, preserving its size and click rectangle.
    pub fn paint(&mut self, painter: &egui::Painter, p: &Palette, id: Uuid, rect: Rect, voice: Option<(f32, bool)>, now: f64, fade: f32) {
        let Some((level, speaking)) = voice else {
            self.envelopes.remove(&id);
            return;
        };
        let e = self.envelopes.entry(id).or_insert_with(|| {
            // The first speaking frame must already be visible.
            Envelope::new(now - f64::from(ATTACK))
        });
        e.update(level, speaking, now);
        if e.opacity > 0.0 {
            let volume = ((e.level - LEVEL_MIN) / (LEVEL_MAX - LEVEL_MIN)).clamp(0.0, 1.0);
            let color = level_color(p, e.level).gamma_multiply(e.opacity * fade);
            painter.rect_stroke(rect, 3.0, Stroke::new(1.2 + volume * 1.2, color), StrokeKind::Inside);
        }
    }

    /// Forget avatars that have left the screen.
    pub fn prune(&mut self, now: f64) {
        self.envelopes.retain(|_, e| now - e.seen < 5.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listening_never_activates_outline_even_with_a_stale_level() {
        let mut e = Envelope::new(0.0);
        e.update(0.8, false, 1.0);
        assert_eq!(e.opacity, 0.0);
        assert_eq!(e.level, 0.0);
    }

    #[test]
    fn speech_reacts_quickly_and_fades_with_its_last_level() {
        let mut e = Envelope::new(0.0);
        e.update(0.6, true, 0.08);
        assert!(e.opacity > 0.85);
        assert!(e.level > 0.5);
        let level = e.level;
        e.update(0.0, false, 0.18);
        assert!(e.opacity > 0.0 && e.opacity < 0.4);
        assert_eq!(e.level, level);
        e.update(0.0, false, 0.7);
        assert_eq!(e.opacity, 0.0);
    }

    #[test]
    fn envelope_is_independent_of_frame_rate() {
        let run = |fps: u32| {
            let mut e = Envelope::new(0.0);
            for frame in 1..=fps {
                e.update(0.5, true, frame as f64 / fps as f64);
            }
            e
        };
        let (a, b) = (run(30), run(144));
        assert!((a.opacity - b.opacity).abs() < 0.0001);
        assert!((a.level - b.level).abs() < 0.0001);
    }
}
