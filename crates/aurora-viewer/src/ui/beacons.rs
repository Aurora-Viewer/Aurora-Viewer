//! floater_beacons.xml / LLFloaterBeacons::onClickUICheck. Closing the
//! floater leaves the saved switches active, as in pipeline.cpp.

use super::widgets::{self, Floater};
use crate::settings::Settings;
use crate::theme::Palette;
use crate::world::beacons::Kind;

pub const WINDOW: &str = "beacons";

pub fn show(ctx: &egui::Context, p: &Palette, open: &mut bool, settings: &mut Settings) {
    if !*open {
        return;
    }
    let before = (settings.beacons, settings.sun_beacon, settings.moon_beacon);
    let window = Floater::new(
        WINDOW,
        aurora_i18n::tr!("beacons-title"),
        egui::pos2(320.0, 100.0),
        egui::vec2(300.0, 400.0),
    );
    window.show(ctx, p, open, |ui| {
        let s = &mut settings.beacons;
        widgets::checkbox(ui, p, &mut s.lines, aurora_i18n::tr!("beacons-lines"));
        widgets::checkbox(ui, p, &mut s.highlights, aurora_i18n::tr!("beacons-highlights"));
        widgets::checkbox(ui, p, &mut s.info, aurora_i18n::tr!("beacons-info"));
        ui.horizontal(|ui| {
            ui.label(aurora_i18n::tr!("beacons-width"));
            s.width = s.width.clamp(1, 16);
            ui.add(egui::Slider::new(&mut s.width, 1..=16));
        });
        widgets::divider(ui, p);
        for (on, kind, text) in [
            (&mut s.physical, Kind::Physical, aurora_i18n::tr!("beacons-physical")),
            (&mut s.sound, Kind::Sound, aurora_i18n::tr!("beacons-sound")),
            (&mut s.particles, Kind::Particle, aurora_i18n::tr!("beacons-particles")),
            (&mut s.media, Kind::Media, aurora_i18n::tr!("beacons-media")),
        ] {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, widgets::inner_radius(ui.visuals()), color(p, kind));
                widgets::checkbox(ui, p, on, text);
            });
        }
        if widgets::checkbox(ui, p, &mut s.scripted, aurora_i18n::tr!("beacons-scripted")).changed() && s.scripted {
            s.touch = false;
        }
        if widgets::checkbox(ui, p, &mut s.touch, aurora_i18n::tr!("beacons-touch")).changed() && s.touch {
            s.scripted = false;
        }
        widgets::divider(ui, p);
        widgets::checkbox(ui, p, &mut settings.sun_beacon, aurora_i18n::tr!("ui-env-editor-sun-beacon"));
        widgets::checkbox(ui, p, &mut settings.moon_beacon, aurora_i18n::tr!("ui-env-editor-moon-beacon"));
    });
    if before != (settings.beacons, settings.sun_beacon, settings.moon_beacon) {
        settings.save();
    }
}

/// Diagnostic colours, shared by the source legend and the world markers.
pub fn color(p: &Palette, kind: Kind) -> egui::Color32 {
    match kind {
        Kind::Script => p.danger,
        Kind::Physical => p.success,
        Kind::Sound => p.amber,
        Kind::Particle => p.indigo_light,
        Kind::Media => p.on_accent,
    }
}

pub fn draw(
    ctx: &egui::Context,
    p: &Palette,
    proj: &super::hud::Projector,
    markers: &[(glam::Vec3, Kind)],
    s: &crate::world::beacons::Beacons,
) {
    if !s.lines {
        return;
    }
    let painter = super::hud::world_painter(ctx);
    for &(at, kind) in markers {
        let color = color(p, kind);
        for (axis, half) in [(glam::Vec3::X, 2.0), (glam::Vec3::Y, 2.0), (glam::Vec3::Z, 50.0)] {
            let a = proj.view_proj * (at - axis * half).extend(1.0);
            let b = proj.view_proj * (at + axis * half).extend(1.0);
            if let Some(segment) = super::minimap::beacon_segment(a, b, proj) {
                painter.line_segment(
                    segment,
                    egui::Stroke::new(f32::from(s.width.clamp(1, 16)), color.gamma_multiply(0.125)),
                );
            }
        }
        // renderObjectBeacons: faint long axes through the world, then a
        // short cross and a 0.1 m cube compared with scene depth on the GPU.
        let line = |a: glam::Vec3, b: glam::Vec3| {
            if let Some(segment) = super::minimap::beacon_segment(proj.view_proj * a.extend(1.0), proj.view_proj * b.extend(1.0), proj) {
                aurora_render::world_line_at(ctx, a, b);
                painter.line_segment(
                    segment,
                    egui::Stroke::new(f32::from(s.width.clamp(1, 16)), color.gamma_multiply(0.5)),
                );
            }
        };
        for axis in [glam::Vec3::X, glam::Vec3::Y, glam::Vec3::Z] {
            line(at - axis * 0.5, at + axis * 0.5);
        }
        for axis in 0..3 {
            for u in [-0.1, 0.1] {
                for v in [-0.1, 0.1] {
                    let mut a = at;
                    a[axis] -= 0.1;
                    a[(axis + 1) % 3] += u;
                    a[(axis + 2) % 3] += v;
                    line(a, a + [glam::Vec3::X, glam::Vec3::Y, glam::Vec3::Z][axis] * 0.2);
                }
            }
        }
        aurora_render::world_label_at(ctx, None);
    }
    if s.info {
        // Keep the source legend above the nearby-chat console, which
        // occupies the lower left of the world even with its window shut.
        let work = super::layout::work_area(ctx);
        let mut y = work.top() + 12.0;
        for (on, kind, label) in [
            (s.scripted, Kind::Script, aurora_i18n::tr!("beacons-scripted")),
            (s.touch, Kind::Script, aurora_i18n::tr!("beacons-touch")),
            (s.physical, Kind::Physical, aurora_i18n::tr!("beacons-physical")),
            (s.sound, Kind::Sound, aurora_i18n::tr!("beacons-sound")),
            (s.particles, Kind::Particle, aurora_i18n::tr!("beacons-particles")),
            (s.media, Kind::Media, aurora_i18n::tr!("beacons-media")),
        ] {
            if on {
                painter.text(
                    egui::pos2(work.left() + 16.0, y),
                    egui::Align2::LEFT_TOP,
                    label,
                    egui::FontId::proportional(12.0),
                    color(p, kind),
                );
                y += 18.0;
            }
        }
    }
}
