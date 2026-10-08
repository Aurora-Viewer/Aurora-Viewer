//! Voice indicator above avatars in voice chat (LLVoiceVisualizer): a dot
//! over the head and, while the person speaks, pairs of waves on both sides
//! that pop out with the speaking level, expand and fade (0.4 s). Waves go
//! from green (quiet) through orange to red (loud).
//!
//! Wave timing and thresholds ported from Firestorm's llvoicevisualizer.cpp
//! (Copyright (C) Linden Research, Inc., GNU originally LGPL 2.1); drawn in screen
//! space at a constant size like the name tags.

use crate::theme::Palette;
use egui::{Color32, Pos2, Stroke};
use std::collections::HashMap;
use uuid::Uuid;

const NUM_WAVES: usize = 7;
/// Seconds for a pair of waves to fade away.
const FADE_OUT_DURATION: f64 = 0.4;
/// Expansion per second (scaled by the speaking level) and maximum.
const EXPANSION_RATE: f32 = 1.0;
const EXPANSION_MAX: f32 = 1.5;
const WAVE_MOTION_RATE: f32 = 1.5;
/// Speaking level range mapped onto 1..6 waves.
const LEVEL_MIN: f32 = 0.2;
const LEVEL_MAX: f32 = 0.7;
/// Screen sizes (points): dot radius, wave half width / height per step.
const DOT_RADIUS: f32 = 3.0;
const WAVE_W: f32 = 2.0;
const WAVE_H: f32 = 1.45;
/// Width reserved in the name tag, left of the name.
pub const TAG_WIDTH: f32 = 30.0;

#[derive(Clone)]
struct Symbol {
    active: [bool; NUM_WAVES],
    fade_start: [f64; NUM_WAVES],
    expansion: [f32; NUM_WAVES],
    last_time: f64,
    seen: f64,
}

impl Symbol {
    fn new(now: f64) -> Symbol {
        Symbol {
            active: [false; NUM_WAVES],
            fade_start: [now; NUM_WAVES],
            expansion: [1.0; NUM_WAVES],
            last_time: now,
            seen: now,
        }
    }
}

#[derive(Default)]
pub struct VoiceDots {
    symbols: HashMap<Uuid, Symbol>,
}

/// Wave color for a speaking level: grey when barely audible, then green →
/// orange → red.
pub fn level_color(p: &Palette, level: f32) -> Color32 {
    let lerp = |a: Color32, b: Color32, t: f32| {
        let t = t.clamp(0.0, 1.0);
        let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
        Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
    };
    let (green, orange, red) = (p.success, p.amber, p.danger);
    if level < LEVEL_MIN {
        Color32::from_gray(180)
    } else if level < 0.45 {
        lerp(green, orange, (level - LEVEL_MIN) / (0.45 - LEVEL_MIN))
    } else {
        lerp(orange, red, (level - 0.45) / (LEVEL_MAX - 0.45))
    }
}

impl VoiceDots {
    /// Draw the indicator of `id` centered on `center`; `fade` dims it with
    /// distance like the name tag.
    #[allow(clippy::too_many_arguments)]
    pub fn paint(&mut self, painter: &egui::Painter, p: &Palette, id: Uuid, center: Pos2, level: f32, speaking: bool, now: f64, fade: f32) {
        let s = self.symbols.entry(id).or_insert_with(|| Symbol::new(now));
        s.seen = now;
        let dt = (now - s.last_time).max(0.0) as f32;
        s.last_time = now;
        if speaking {
            // trigger waves 1..6 according to the level
            let fraction = ((level - LEVEL_MIN) / (LEVEL_MAX - LEVEL_MIN)).clamp(0.0, 1.0);
            let count = 1 + (fraction * (NUM_WAVES - 2) as f32) as usize;
            for i in 0..=count.min(NUM_WAVES - 1) {
                s.active[i] = true;
                s.fade_start[i] = now;
            }
        }
        let col = level_color(p, level);
        // dot (Firestorm: white, 70 % opaque)
        painter.circle_filled(center, DOT_RADIUS + 0.8, Color32::from_black_alpha((90.0 * fade) as u8));
        painter.circle_filled(center, DOT_RADIUS, Color32::from_white_alpha((200.0 * fade) as u8));
        for i in 1..NUM_WAVES {
            if !s.active[i] {
                continue;
            }
            let opacity = 1.0 - ((now - s.fade_start[i]) / FADE_OUT_DURATION) as f32;
            if opacity <= 0.0 {
                s.active[i] = false;
                continue;
            }
            s.expansion[i] *= 1.0 + EXPANSION_RATE * dt * level * WAVE_MOTION_RATE;
            if s.expansion[i] > EXPANSION_MAX {
                s.expansion[i] = 1.0;
            }
            let w = DOT_RADIUS + i as f32 * WAVE_W * s.expansion[i];
            let h = DOT_RADIUS + i as f32 * WAVE_H * s.expansion[i];
            let stroke = Stroke::new(1.1, col.gamma_multiply(opacity * fade));
            // ")" on the right, "(" on the left
            for side in [0.0f32, std::f32::consts::PI] {
                let pts: Vec<Pos2> = (0..=10)
                    .map(|k| {
                        let a = side + (-0.9 + 1.8 * k as f32 / 10.0);
                        center + egui::vec2(a.cos() * w, -a.sin() * h)
                    })
                    .collect();
                painter.add(egui::Shape::line(pts, stroke));
            }
        }
    }

    /// Forget avatars not drawn for a while.
    pub fn prune(&mut self, now: f64) {
        self.symbols.retain(|_, s| now - s.seen < 5.0);
    }
}
