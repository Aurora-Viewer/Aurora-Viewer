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
        egui::vec2(300.0, 470.0),
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
        widgets::checkbox(ui, p, &mut s.light_boxes, aurora_i18n::tr!("beacons-light-boxes"));
        widgets::checkbox(ui, p, &mut s.region_corners, aurora_i18n::tr!("beacons-region-corners"));
        widgets::checkbox(ui, p, &mut s.chat_spheres, aurora_i18n::tr!("beacons-chat-spheres"));
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
        marker(ctx, &painter, proj, at, color(p, kind), f32::from(s.width.clamp(1, 16)));
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

/// renderObjectBeacons and the debug beacons of LLViewerObjectList
/// (indra/newview/llviewerobjectlist.cpp, originally LGPL 2.1): faint long
/// axes through the world, then a short cross and a 0.1 m cube compared
/// with scene depth on the GPU.
fn marker(ctx: &egui::Context, painter: &egui::Painter, proj: &super::hud::Projector, at: glam::Vec3, color: egui::Color32, width: f32) {
    for (axis, half) in [(glam::Vec3::X, 2.0), (glam::Vec3::Y, 2.0), (glam::Vec3::Z, 50.0)] {
        let a = proj.view_proj * (at - axis * half).extend(1.0);
        let b = proj.view_proj * (at + axis * half).extend(1.0);
        if let Some(segment) = super::minimap::beacon_segment(a, b, proj) {
            painter.line_segment(segment, egui::Stroke::new(width, color.gamma_multiply(0.125)));
        }
    }
    for axis in [glam::Vec3::X, glam::Vec3::Y, glam::Vec3::Z] {
        depth_line(
            ctx,
            painter,
            proj,
            at - axis * 0.5,
            at + axis * 0.5,
            egui::Stroke::new(width, color.gamma_multiply(0.5)),
        );
    }
    for axis in 0..3 {
        for u in [-0.1, 0.1] {
            for v in [-0.1, 0.1] {
                let mut a = at;
                a[axis] -= 0.1;
                a[(axis + 1) % 3] += u;
                a[(axis + 2) % 3] += v;
                let b = a + [glam::Vec3::X, glam::Vec3::Y, glam::Vec3::Z][axis] * 0.2;
                depth_line(ctx, painter, proj, a, b, egui::Stroke::new(width, color.gamma_multiply(0.5)));
            }
        }
    }
    aurora_render::world_label_at(ctx, None);
}

/// A world segment that the scene in front of it hides.
fn depth_line(
    ctx: &egui::Context,
    painter: &egui::Painter,
    proj: &super::hud::Projector,
    a: glam::Vec3,
    b: glam::Vec3,
    stroke: egui::Stroke,
) {
    if let Some(segment) = super::minimap::beacon_segment(proj.view_proj * a.extend(1.0), proj.view_proj * b.extend(1.0), proj) {
        aurora_render::world_line_at(ctx, a, b);
        painter.line_segment(segment, stroke);
    }
}

/// The guides of the floater's last switches, drawn whatever "Beacons"
/// says, as in the Legacy Viewer: the lights' boxes, the region corners and
/// the chat range spheres.
pub fn draw_guides(
    ctx: &egui::Context,
    p: &Palette,
    world: &crate::world::World,
    proj: &super::hud::Projector,
    lights: &[crate::world::beacons::LightBox],
    s: &crate::world::beacons::Beacons,
) {
    if !(s.light_boxes || s.region_corners || s.chat_spheres) {
        return;
    }
    let painter = super::hud::world_painter(ctx);
    aurora_render::world_label_at(ctx, None);
    // renderLights: the bounds in white over everything (depth test off),
    // the radius in yellow behind the scene
    for l in lights {
        for [a, b] in crate::world::beacons::box_edges(l.center, l.half) {
            if let Some(segment) = super::minimap::beacon_segment(proj.view_proj * a.extend(1.0), proj.view_proj * b.extend(1.0), proj) {
                painter.line_segment(segment, egui::Stroke::new(1.0, p.ink));
            }
        }
        for [a, b] in crate::world::beacons::box_edges(l.center, glam::Vec3::splat(l.radius)) {
            depth_line(ctx, &painter, proj, a, b, egui::Stroke::new(1.0, p.amber));
        }
        aurora_render::world_label_at(ctx, None);
    }
    if s.chat_spheres {
        // the minimap's ring colours (MapWhisperRingColor, MapChatRingColor,
        // MapShoutRingColor), at the region's ranges
        let k = super::colors::get();
        let ranges = crate::world::radar::chat_ranges();
        let at = world.agent.position;
        for (radius, col) in [
            (ranges.whisper, k.map_whisper),
            (ranges.say, k.map_say),
            (ranges.shout, k.map_shout),
        ] {
            let stroke = egui::Stroke::new(1.5, super::colors::c(col).gamma_multiply(0.6));
            for [a, b] in crate::world::beacons::sphere_rings(at, radius) {
                depth_line(ctx, &painter, proj, a, b, stroke);
            }
        }
        aurora_render::world_label_at(ctx, None);
    }
    if s.region_corners
        && let Some(region) = world.main()
    {
        // addDebugBeacon(pos, "SW"…, yellow, white text, DebugBeaconLineWidth)
        let width = f32::from(s.width.clamp(1, 16));
        let names = [
            aurora_i18n::tr!("map-dir-sw"),
            aurora_i18n::tr!("map-dir-se"),
            aurora_i18n::tr!("map-dir-nw"),
            aurora_i18n::tr!("map-dir-ne"),
        ];
        for (at, name) in crate::world::beacons::region_corners(&region.heightmap).into_iter().zip(names) {
            marker(ctx, &painter, proj, at, p.amber, width);
            if let Some((point, _)) = proj.project(at) {
                aurora_render::world_label_at(ctx, Some(at));
                let galley = painter.layout_no_wrap(name.to_owned(), egui::FontId::proportional(12.5), p.ink);
                let rect = egui::Rect::from_center_size(
                    point - egui::vec2(0.0, galley.size().y + 8.0),
                    galley.size() + egui::vec2(12.0, 6.0),
                );
                super::widgets::paint_world_label(&painter, p, rect);
                painter.galley(rect.min + egui::vec2(6.0, 3.0), galley, p.ink);
                aurora_render::world_label_at(ctx, None);
            }
        }
    }
}
