//! Mouselook combat aids: the Legacy Viewer's FSMouselookCombatFeatures
//! (LLViewerWindow::draw, the Exodus block, indra/newview/llviewerwindow.cpp).
//! The name and distance of the first avatar in a narrow box ahead of the
//! camera, under the crosshair; with ExodusMouselookIFF, an arrow at the
//! edge of the screen toward each avatar in range out of view
//! (LLTracker::drawMarker with `is_iff`, indra/newview/lltracker.cpp). Both
//! off by default, as there.

use crate::theme::Palette;
use crate::world::World;
use egui::{Pos2, Rect, Vec2, pos2, vec2};
use glam::Vec3;

/// The box the Legacy Viewer reads in the camera's frame: 0.75 m either
/// side of the view axis, 1.5 m above or below it, in front.
pub fn ahead(eye: Vec3, forward: Vec3, target: Vec3) -> bool {
    let forward = forward.normalize_or(Vec3::X);
    let right = forward.cross(Vec3::Z).normalize_or(Vec3::Y);
    let up = right.cross(forward);
    let v = target - eye;
    let (x, y, z) = (v.dot(right), v.dot(up), v.dot(forward));
    z > 0.0 && x.abs() < 0.75 && y.abs() < 1.5
}

/// One frame of the aids, in mouselook with FSMouselookCombatFeatures on.
pub fn draw(
    ctx: &egui::Context,
    p: &Palette,
    world: &World,
    proj: &super::hud::Projector,
    eye: Vec3,
    forward: Vec3,
    settings: &crate::camera::CameraSettings,
) {
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("mouselook-combat")));
    let screen = ctx.content_rect();
    let inner = screen.shrink(24.0);
    // LLWorld::getAvatars around the agent, ExodusMouselookIFFRange
    let range = settings.mouselook_iff_range;
    // RLV_BHVR_SHOWNAMES hides the target line altogether
    let names_hidden = world.rlva.enabled() && world.rlva.has("shownames");
    let mut target_shown = false;
    for s in crate::world::radar::sightings(world) {
        if !s.altitude_known || s.position.distance(world.agent.position) > range {
            continue;
        }
        let name = world.avatar_name(&s.id);
        let color = crate::ui::colors::map_color(world, &s.id, name.as_deref().unwrap_or_default());
        if settings.mouselook_iff {
            let clip = proj.view_proj * s.position.extend(1.0);
            if let Some((at, false, direction)) = super::minimap::beacon_marker(clip, inner, proj) {
                arrow(&painter, p, at, direction, color);
            }
        }
        if !target_shown && !names_hidden && ahead(eye, forward, s.position) {
            target_shown = true;
            let name = name.unwrap_or_else(|| aurora_i18n::tr!("app-mouselook-unknown-avatar").to_string());
            let text = aurora_i18n::tr!(
                "app-mouselook-target",
                name = name,
                distance = format!("{:.2}", s.position.distance(eye))
            );
            // 150 px under the middle, as ExodusMouselookTextOffsetY
            let at = screen.center() + vec2(0.0, 150.0);
            let font = egui::FontId::proportional(15.0);
            painter.text(
                at + vec2(1.0, 1.0),
                egui::Align2::CENTER_TOP,
                &text,
                font.clone(),
                p.navy.gamma_multiply(0.8),
            );
            painter.text(at, egui::Align2::CENTER_TOP, &text, font, color);
        }
    }
}

/// The edge arrow, pointing toward the avatar.
fn arrow(painter: &egui::Painter, p: &Palette, at: Pos2, direction: Vec2, color: egui::Color32) {
    let Some(texture) = super::icons::global("arrow-right") else {
        return;
    };
    for (size, offset, color) in [(24.0, vec2(0.0, 1.0), p.bar), (20.0, Vec2::ZERO, color)] {
        let mut mesh = egui::Mesh::with_texture(texture.id());
        mesh.add_rect_with_uv(
            Rect::from_center_size(at + offset, vec2(size, size)),
            Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
            color,
        );
        mesh.rotate(egui::emath::Rot2::from_angle(direction.angle()), at);
        painter.add(mesh);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_target_box_is_narrow_and_ahead_only() {
        let eye = Vec3::new(10.0, 10.0, 22.0);
        let forward = Vec3::X;
        assert!(ahead(eye, forward, eye + Vec3::new(40.0, 0.5, -1.2)), "far ahead, a little aside");
        assert!(!ahead(eye, forward, eye + Vec3::new(40.0, 0.8, 0.0)), "0.75 m aside at most");
        assert!(!ahead(eye, forward, eye + Vec3::new(40.0, 0.0, 1.6)), "1.5 m up at most");
        assert!(!ahead(eye, forward, eye - Vec3::new(5.0, 0.0, 0.0)), "never behind");
        // the box turns with the camera
        let diagonal = Vec3::new(1.0, 1.0, 0.0).normalize();
        assert!(ahead(eye, diagonal, eye + diagonal * 30.0));
        assert!(!ahead(eye, diagonal, eye + Vec3::X * 30.0));
    }
}
