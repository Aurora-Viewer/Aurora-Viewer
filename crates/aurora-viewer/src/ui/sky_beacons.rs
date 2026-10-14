//! The sun and moon beacons ("Show Beacon" of the sky editors' Sun & Moon
//! tab, the sunbeacon / moonbeacon settings): LLSky::renderSunMoonBeacons
//! (newview/llsky.cpp, llglsandbox.cpp, originally LGPL 2.1) draws a line
//! from the avatar 50 m towards the shown sun or moon, with a small cross at
//! the avatar and a larger one at the far end. Drawn here over the world
//! like the map's beacon, in the editors' sun and moon colours.

use super::hud::Projector;
use crate::theme::Palette;
use crate::world::World;
use egui::Stroke;
use glam::Vec3;

/// LLSky::renderSunMoonBeacons' length (m) and cross sizes.
const LENGTH: f32 = 50.0;
const NEAR_CROSS: f32 = 0.5;
const FAR_CROSS: f32 = 2.0;

/// Draw the beacons switched on (`[sun, moon]`).
pub fn draw(ctx: &egui::Context, p: &Palette, world: &World, proj: &Projector, beacons: [bool; 2]) {
    if !beacons.contains(&true) {
        return;
    }
    let painter = super::hud::world_painter(ctx);
    let start = world.agent.position;
    let (sun, moon) = world.sky_directions;
    // the sky dome's sun and moon colours of the Sun & Moon tab
    for (on, dir, color) in [(beacons[0], sun, p.amber), (beacons[1], moon, p.violet_light)] {
        if !on || !dir.is_finite() || dir.length_squared() < 1e-6 {
            continue;
        }
        let end = start + dir.normalize() * LENGTH;
        let line = |a: Vec3, b: Vec3, width: f32| {
            if let Some(seg) = super::minimap::beacon_segment(proj.view_proj * a.extend(1.0), proj.view_proj * b.extend(1.0), proj) {
                painter.line_segment(seg, Stroke::new(width + 2.0, p.bar.gamma_multiply(0.5)));
                painter.line_segment(seg, Stroke::new(width, color));
            }
        };
        line(start, end, 2.0);
        for (at, half) in [(start, NEAR_CROSS), (end, FAR_CROSS)] {
            for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
                line(at - axis * half, at + axis * half, 1.5);
            }
        }
    }
}
