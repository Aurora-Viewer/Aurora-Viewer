//! The Compass window (toolbar `avatar_align`, Me › Avatar alignment): the
//! Legacy Viewer's FSFloaterAvatarAlign and FSFloaterAvatarAlignMini
//! (indra/newview/fsfloateravataralign.cpp, floater_avatar_align.xml and
//! floater_avatar_align_mini.xml, originally LGPL 2.1). A compass shows the
//! heading; a click on its ring turns to that octant, on its centre to the
//! nearest avatar; buttons turn by steps. "Mini" / "Full" in the compass's
//! corner switches between the two layouts (AvatarAlignMini). The turns
//! themselves are the app's (`app/avatar_align.rs`), their maths
//! `crate::avatar_align`.
//!
//! The compact layout keeps Aurora's title bar and its close button, where
//! the Legacy mini window has neither: every window here is a `Floater`.

use super::widgets::{self, Floater};
use crate::avatar_align::Zone;
use crate::theme::Palette;
use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2, pos2, vec2};

/// The full window's id (floater name `avatar_align`).
pub const WINDOW: &str = "avatar_align";
/// The compact window's id (`avatar_align_mini`).
pub const WINDOW_MINI: &str = "avatar_align_mini";

/// The window of the current layout.
pub fn window(mini: bool) -> &'static str {
    if mini { WINDOW_MINI } else { WINDOW }
}

/// What the window asks the app to do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    /// Turn to this compass bearing (a click on the ring).
    Bearing(f32),
    /// Turn by these degrees, negative to the left (the step buttons).
    Rotate(f32),
    /// "To nearest cardinal".
    NearestCardinal,
    /// "Face nearest avatar" (and a click on the centre).
    FaceNearest,
    /// "Mini" / "Full": the other layout.
    ToggleMode,
}

const FULL_SIZE: Vec2 = vec2(240.0, 420.0);
const MINI_SIZE: Vec2 = vec2(150.0, 210.0);

/// The window; `yaw` is the agent's heading, for the needle and the
/// bearing. Returns what was clicked.
pub fn show(ctx: &egui::Context, p: &Palette, open: &mut bool, mini: bool, yaw: f32) -> Option<Action> {
    let id = window(mini);
    let mut floater = Floater::new(
        id,
        aurora_i18n::tr!("avatar-align-title"),
        pos2(60.0, 160.0),
        if mini { MINI_SIZE } else { FULL_SIZE },
    );
    floater.min_size = if mini { MINI_SIZE } else { FULL_SIZE };
    let action = floater
        .show(ctx, p, open, |ui| if mini { mini_body(ui, p, yaw) } else { full_body(ui, p, yaw) })
        .flatten();
    if action == Some(Action::ToggleMode) {
        swap_layout(ctx, mini);
    }
    action
}

/// repositionOnToggle: the other window takes the place of this one, held
/// by the side of the screen it is nearest to.
fn swap_layout(ctx: &egui::Context, mini: bool) {
    let (from, to) = (window(mini), window(!mini));
    widgets::close_floater(ctx, from);
    let Some(old) = ctx.memory(|m| m.area_rect(egui::Id::unique(from))) else {
        return;
    };
    let size = ctx
        .memory(|m| m.area_rect(egui::Id::unique(to)))
        .map_or(if mini { FULL_SIZE } else { MINI_SIZE }, |r| r.size());
    let screen = ctx.content_rect();
    let x = if old.center().x < screen.center().x {
        old.left()
    } else {
        old.right() - size.x
    };
    let y = if old.center().y > screen.center().y {
        old.bottom() - size.y
    } else {
        old.top()
    };
    widgets::move_floater(ctx, to, pos2(x, y));
}

fn full_body(ui: &mut egui::Ui, p: &Palette, yaw: f32) -> Option<Action> {
    let row = 26.0;
    // floater_avatar_align.xml: the compass, then two rows of four steps and
    // two wide buttons. The buttons are the footer, measured from their
    // content (large text or a taller font cannot grow the window); the
    // compass takes the room that is left, both of buildCompassLayout's
    // limits met.
    let (mut above, footer) = widgets::above_footer(ui);
    let width = above.available_width();
    let area = vec2(width, above.available_height().max(width + 24.0));
    let mut action = compass(&mut above, p, area, yaw, false);
    footer.show(ui, |ui| {
        let step = (width - 3.0 * ui.spacing().item_spacing.x) / 4.0;
        for steps in [[-10.0, -1.0, 1.0, 10.0], [-90.0, -45.0, 45.0, 90.0]] {
            ui.horizontal(|ui| {
                for delta in steps {
                    let label = if delta < 0.0 { format!("{delta}°") } else { format!("+{delta}°") };
                    if ui
                        .add_sized(vec2(step, row), egui::Button::new(label))
                        .on_hover_text(rotate_tip(delta))
                        .clicked()
                    {
                        action = Some(Action::Rotate(delta));
                    }
                }
            });
        }
        let nearest = ui
            .add_sized(
                vec2(width, row),
                egui::Button::new(aurora_i18n::tr!("avatar-align-nearest-cardinal")),
            )
            .on_hover_text(aurora_i18n::tr!("avatar-align-nearest-cardinal-tip"));
        if nearest.clicked() {
            action = Some(Action::NearestCardinal);
        }
        let face = widgets::action_button(
            ui,
            p,
            "user-search",
            aurora_i18n::tr!("toolbar-command-face-nearest"),
            widgets::ActionKind::Flat,
            vec2(width, row),
        )
        .on_hover_text(aurora_i18n::tr!("avatar-align-face-nearest-tip"));
        if face.clicked() {
            action = Some(Action::FaceNearest);
        }
    });
    action
}

fn mini_body(ui: &mut egui::Ui, p: &Palette, yaw: f32) -> Option<Action> {
    let mut action = None;
    // pnl_toolbar: 1° left, 1° right, face the nearest avatar
    ui.horizontal(|ui| {
        if widgets::icon_button(ui, p, "caret-left-filled", &rotate_tip(-1.0), true) {
            action = Some(Action::Rotate(-1.0));
        }
        if widgets::icon_button(ui, p, "caret-right-filled", &rotate_tip(1.0), true) {
            action = Some(Action::Rotate(1.0));
        }
        if widgets::icon_button(ui, p, "user-search", aurora_i18n::tr!("avatar-align-face-nearest-tip"), true) {
            action = Some(Action::FaceNearest);
        }
    });
    let width = ui.available_width();
    let area = vec2(width, ui.available_height().max(width + 13.0));
    compass(ui, p, area, yaw, true).or(action)
}

fn rotate_tip(delta: f32) -> String {
    let degrees = delta.abs().to_string();
    if delta < 0.0 {
        aurora_i18n::tr!("avatar-align-rotate-left", degrees = degrees.as_str())
    } else {
        aurora_i18n::tr!("avatar-align-rotate-right", degrees = degrees.as_str())
    }
}

/// The compass's geometry in `rect` (buildCompassLayout): its radius and
/// centre, and what it has room for.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Layout {
    radius: f32,
    centre: Pos2,
    /// Mini and too small for the bearing and the intercardinal arms.
    compact: bool,
    /// Full and large enough for the N / S / E / W letters.
    letters: bool,
    /// Half the side of the square the toggle sits in the corner of.
    square: f32,
}

impl Layout {
    fn new(rect: Rect, mini: bool) -> Layout {
        // full: 27 above (the N), 80 in all (the S, the bearing); mini: 7
        // and 29; full keeps 28 on the sides for the E and W
        let (top, overhead, side) = if mini { (7.0, 29.0, 8.0) } else { (27.0, 80.0, 28.0) };
        let radius = (rect.width() / 2.0 - side).min((rect.height() - overhead) / 2.0).max(30.0);
        Layout {
            radius,
            centre: pos2(rect.center().x, rect.top() + top + radius),
            compact: mini && radius < 50.0,
            letters: !mini && radius >= 50.0,
            square: if mini { radius } else { radius + 20.0 },
        }
    }

    /// A point at `bearing` degrees, `distance` from the centre.
    fn at(&self, bearing: f32, distance: f32) -> Pos2 {
        let a = bearing.to_radians();
        self.centre + vec2(a.sin(), -a.cos()) * distance
    }

    /// The zone under `pos` (the screen's y goes down, the compass's north up).
    fn zone(&self, pos: Pos2) -> Option<Zone> {
        let d = pos - self.centre;
        Zone::at(glam::Vec2::new(d.x, -d.y), self.radius)
    }
}

/// The compass (drawCompass) in a space of `size`, with its Mini / Full
/// toggle; a click on it is an action.
fn compass(ui: &mut egui::Ui, p: &Palette, size: Vec2, yaw: f32, mini: bool) -> Option<Action> {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let lo = Layout::new(rect, mini);
    let hover = response.hover_pos().and_then(|pos| lo.zone(pos));
    let mut action = response
        .clicked()
        .then(|| response.interact_pointer_pos())
        .flatten()
        .and_then(|pos| lo.zone(pos))
        .map(|zone| zone.bearing().map_or(Action::FaceNearest, Action::Bearing));
    let painter = ui.painter_at(rect);
    let (c, r) = (lo.centre, lo.radius);
    painter.circle_filled(c, r, p.field);
    painter.circle_stroke(c, r, Stroke::new(1.0, p.muted));
    painter.circle_stroke(c, r * 0.5, Stroke::new(1.0, p.muted_dim));
    for i in 0..8 {
        let b = i as f32 * 45.0;
        painter.line_segment([lo.at(b, r - 7.0), lo.at(b, r)], Stroke::new(1.0, p.muted_dim));
    }
    match hover {
        Some(Zone::Centre) => {
            painter.circle_filled(c, r * 0.25, p.ink.gamma_multiply(0.18));
        }
        Some(Zone::Octant(i)) => {
            painter.add(sector(&lo, f32::from(i) * 45.0, p.ink.gamma_multiply(0.13)));
        }
        None => {}
    }
    // the cardinal arms (north red, south light, east and west grey), then
    // the shorter intercardinals: a kite whose base crosses the centre
    let arm = |bearing: f32, reach: f32, half: f32, color: Color32| {
        let shape = egui::Shape::convex_polygon(
            vec![lo.at(bearing, reach), lo.at(bearing + 90.0, half), lo.at(bearing - 90.0, half)],
            color,
            Stroke::NONE,
        );
        painter.add(shape);
    };
    for (bearing, color) in [(0.0, p.danger), (180.0, p.ink), (90.0, p.muted), (270.0, p.muted)] {
        arm(bearing, r - 4.0, r * 0.245, color);
    }
    if !lo.compact {
        for bearing in [45.0, 135.0, 225.0, 315.0] {
            arm(bearing, r * 0.62, r * 0.10, p.muted_dim);
        }
    }
    painter.circle_filled(c, 5.0, p.raised);
    painter.circle_stroke(c, 5.0, Stroke::new(1.0, p.muted));
    // the heading needle
    let heading = crate::avatar_align::bearing(yaw);
    let reach = (r - 4.0) * 0.88;
    let needle = vec![lo.at(heading, reach), lo.at(heading + 90.0, 4.0), lo.at(heading - 90.0, 4.0)];
    painter.add(egui::Shape::convex_polygon(needle, p.amber, Stroke::NONE));
    let font = egui::FontId::proportional(12.0);
    if lo.letters {
        let d = r + 10.0;
        let letter = |bearing: f32, align: egui::Align2, text: &str, color: Color32| {
            painter.text(lo.at(bearing, d), align, text, font.clone(), color);
        };
        letter(0.0, egui::Align2::CENTER_BOTTOM, "N", p.rose);
        letter(180.0, egui::Align2::CENTER_TOP, "S", p.ink);
        letter(90.0, egui::Align2::LEFT_CENTER, "E", p.ink);
        letter(270.0, egui::Align2::RIGHT_CENTER, "W", p.ink);
    }
    if !lo.compact {
        let below = if mini { 10.0 } else { 25.0 };
        painter.text(
            pos2(c.x, c.y + r + below),
            egui::Align2::CENTER_TOP,
            format!("{heading:03.0}°"),
            font,
            p.ink,
        );
    }
    // the toggle, in the top right corner of the compass's square
    let label = if mini {
        aurora_i18n::tr!("avatar-align-full")
    } else {
        aurora_i18n::tr!("avatar-align-mini")
    };
    let text = egui::RichText::new(label).size(11.0);
    let width = ui.fonts_mut(|f| f.layout_no_wrap(label.to_owned(), egui::FontId::proportional(11.0), p.ink).size().x)
        + 2.0 * ui.spacing().button_padding.x
        + 4.0;
    let corner = pos2(c.x + lo.square, c.y - lo.square);
    let toggle = Rect::from_min_size(pos2(corner.x - width, corner.y), vec2(width, 16.0));
    // a child, not `ui.put`: put moves the cursor back up to the toggle,
    // and the buttons under the compass would cover it
    let mut corner_ui = ui.new_child(egui::UiBuilder::new().max_rect(toggle));
    if corner_ui.add_sized(toggle.size(), egui::Button::new(text)).clicked() {
        action = Some(Action::ToggleMode);
    }
    action
}

/// The hovered octant: the 45° slice of the ring between a quarter of the
/// radius and the rim.
fn sector(lo: &Layout, bearing: f32, color: Color32) -> egui::Shape {
    let mut mesh = egui::Mesh::default();
    let (inner, outer) = (lo.radius * 0.25, lo.radius);
    const STEPS: u32 = 8;
    for i in 0..=STEPS {
        let b = bearing - 22.5 + 45.0 * i as f32 / STEPS as f32;
        mesh.colored_vertex(lo.at(b, inner), color);
        mesh.colored_vertex(lo.at(b, outer), color);
    }
    for i in 0..STEPS {
        let k = 2 * i;
        mesh.add_triangle(k, k + 1, k + 3);
        mesh.add_triangle(k, k + 3, k + 2);
    }
    egui::Shape::mesh(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_compass_fits_its_space_like_the_legacy_layout() {
        let full = Layout::new(Rect::from_min_size(Pos2::ZERO, vec2(230.0, 260.0)), false);
        // the width less 28 on each side, below 27 for the N
        assert_eq!(full.radius, 87.0);
        assert_eq!(full.centre, pos2(115.0, 27.0 + 87.0));
        assert!(full.letters && !full.compact);
        // the Legacy mini window's 74 × 98: the 30 floor, too small for
        // the bearing and the intercardinals
        let mini = Layout::new(Rect::from_min_size(Pos2::ZERO, vec2(74.0, 98.0)), true);
        assert_eq!(mini.radius, 30.0);
        assert!(mini.compact && !mini.letters);
        let mini = Layout::new(Rect::from_min_size(Pos2::ZERO, vec2(120.0, 160.0)), true);
        assert_eq!(mini.radius, 52.0);
        assert!(!mini.compact && !mini.letters);
        // never below 30
        assert_eq!(Layout::new(Rect::from_min_size(Pos2::ZERO, vec2(10.0, 10.0)), true).radius, 30.0);
    }

    #[test]
    fn a_click_above_the_centre_is_north_on_screen() {
        let lo = Layout::new(Rect::from_min_size(Pos2::ZERO, vec2(230.0, 260.0)), false);
        assert_eq!(lo.zone(lo.centre), Some(Zone::Centre));
        assert_eq!(lo.zone(lo.centre - vec2(0.0, 60.0)), Some(Zone::Octant(0)));
        assert_eq!(lo.zone(lo.centre + vec2(60.0, 0.0)), Some(Zone::Octant(2)));
        assert_eq!(lo.zone(lo.centre + vec2(0.0, 60.0)), Some(Zone::Octant(4)));
        assert_eq!(lo.zone(lo.centre + vec2(-50.0, -50.0)), Some(Zone::Octant(7)));
        assert!((lo.at(90.0, 10.0) - (lo.centre + vec2(10.0, 0.0))).length() < 1e-4);
    }
}
