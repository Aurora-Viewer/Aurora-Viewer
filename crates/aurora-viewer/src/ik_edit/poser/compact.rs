//! The poser's compact window, the default one (the title bar's button
//! switches to the full window of `ui.rs`, and the choice is remembered):
//! the subject (its picture opens the subject menu: you, your animesh and
//! the nearby avatars to ask), the others posed, Start / Stop, the joint
//! groups, the transform and the side, the selected joint, then the
//! markers, the depth of field and the files. Posing goes through the
//! markers and gizmos in the world; the joint list, the sliders and the
//! trackball stay in the full window. Drawn with the theme's own pieces
//! (the segmented track, the small icon buttons) so it reads as the other
//! windows do.
//!
//! An Aurora addition: the Legacy Viewer's poser (FSFloaterPoser) has one
//! window, whose advanced panel collapses. Every action here is one of the
//! full window's, with the same rules and the same state.
use super::*;
use crate::ui::menu;
use document::LoadMode;

/// The body's width.
const BODY_W: f32 = 272.0;
/// The window's size when it opens (its height follows the content).
pub const COMPACT_SIZE: egui::Vec2 = egui::vec2(BODY_W + 28.0, 300.0);
/// The subject's picture, and the others posed beside Start / Stop.
const HEAD_DISC: f32 = 34.0;
const OTHER_DISC: f32 = 22.0;
/// The groups' track, and the cells of the two three-way toggles.
const GROUPS_H: f32 = 42.0;
const TOGGLE: egui::Vec2 = egui::vec2(34.0, 26.0);
/// The small square buttons of the last row.
const BUTTON: f32 = 26.0;
/// The track's inset around its cells, as the theme's `segmented`.
const INSET: f32 = 2.0;

/// Where a click picks an avatar posed: its joints' box, a little wider for
/// the body around the bones.
pub(super) fn body_area(points: impl IntoIterator<Item = egui::Pos2>) -> Option<egui::Rect> {
    let area = joints_box(points)?;
    Some(area.expand((area.width().max(area.height()) * 0.06).max(8.0)))
}

fn joints_box(points: impl IntoIterator<Item = egui::Pos2>) -> Option<egui::Rect> {
    let mut points = points.into_iter();
    let first = points.next()?;
    Some(points.fold(egui::Rect::from_min_max(first, first), |r, p| {
        r.union(egui::Rect::from_min_max(p, p))
    }))
}

/// The posed avatar a click lands on: the nearest to the camera of those
/// whose `body_area` holds the pointer (id, area, nearest joint's depth).
pub(super) fn clicked_subject(others: &[(Uuid, egui::Rect, f32)], pointer: egui::Pos2) -> Option<Uuid> {
    others
        .iter()
        .filter(|(_, area, _)| area.contains(pointer))
        .min_by(|a, b| a.2.total_cmp(&b.2))
        .map(|(id, _, _)| *id)
}

/// The margins around the silhouette where the pointer reveals the markers
/// in `markers_hover` (points): to enter, then to leave, wider so that they
/// do not flicker at the edge; and how long they stay once it left (s).
pub(super) const REVEAL_ENTER: f32 = 16.0;
pub(super) const REVEAL_LEAVE: f32 = 36.0;
pub(super) const REVEAL_LINGER: f64 = 0.3;

/// Whether the markers show this frame, before the linger. Always mode
/// shows them; hover mode shows them while the pointer is within the
/// margin of the avatar's silhouette (`depth`: how far inside it, None
/// without a pointer; the margin wider once `shown`) and not over a
/// window, and while a joint or a gizmo is held, so a drag that leaves
/// keeps them.
pub(super) fn markers_shown(hover_mode: bool, depth: Option<f32>, shown: bool, over_ui: bool, held: bool) -> bool {
    if !hover_mode || held {
        return true;
    }
    let margin = if shown { REVEAL_LEAVE } else { REVEAL_ENTER };
    !over_ui && depth.is_some_and(|d| d > -margin)
}

/// The others' stack of pictures in the head: each `PILE_STEP` after the
/// one before, at most `PILE_MAX` slots (the last a "+n" when more).
const PILE_STEP: f32 = 18.0;
const PILE_MAX: usize = 4;
struct Pile {
    /// The pictures shown.
    shown: usize,
    /// A "+n" after them.
    more: bool,
}
impl Pile {
    fn of(count: usize) -> Self {
        if count <= PILE_MAX {
            Self { shown: count, more: false }
        } else {
            Self {
                shown: PILE_MAX - 1,
                more: true,
            }
        }
    }
    fn slots(&self) -> usize {
        self.shown + usize::from(self.more)
    }
    fn width(&self) -> f32 {
        match self.slots() {
            0 => 0.0,
            n => OTHER_DISC + (n - 1) as f32 * PILE_STEP,
        }
    }
}

/// What a cell of a track shows.
enum Face<'a> {
    Icon(&'a str),
    Letter(&'a str),
}

/// How a track shows its chosen cell: `Accent` fills it in the accent (the
/// groups, the window's one strong choice), `Quiet` raises it and lights
/// its glyph in the accent (the modes and sides), so that one violet block
/// leads the eye rather than four.
#[derive(Clone, Copy, PartialEq)]
enum Lit {
    Accent,
    Quiet,
}

/// A choice among cells in one track, painted as the theme's `segmented`;
/// `labels` puts a short word under each icon. The highlight slides to the
/// chosen cell and the hover fades in and out, both with egui's animation
/// time (none under Reduce animations). Returns the cell clicked, if it
/// was not the chosen one.
fn track(
    ui: &mut egui::Ui,
    p: &Palette,
    id: &str,
    cell: egui::Vec2,
    chosen: usize,
    lit: Lit,
    cells: &[(Face, &str, &str)],
) -> Option<usize> {
    let n = cells.len() as f32;
    let size = egui::vec2(cell.x * n + INSET * 2.0, cell.y + INSET * 2.0);
    let (bar, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(bar, ui.visuals().widgets.inactive.corner_radius, p.bar);
    let id = ui.scope_id().with(("poser-track", id));
    let cell_at = |i: f32| egui::Rect::from_min_size(egui::pos2(bar.left() + INSET + cell.x * i, bar.top() + INSET), cell);
    // the highlight, sliding from the cell chosen before
    let time = ui.style().animation_time;
    let slide = ui.ctx().animate_value_with_time(id, chosen as f32, time);
    let (face, chosen_ink) = match lit {
        Lit::Accent => (p.violet, p.on_accent),
        Lit::Quiet => (p.raised, p.violet_light),
    };
    ui.painter().rect_filled(cell_at(slide), widgets::inner_radius(ui.visuals()), face);
    let mut picked = None;
    for (i, (glyph_face, label, tip)) in cells.iter().enumerate() {
        let rect = cell_at(i as f32);
        let response = ui.interact(rect, id.with(i), Sense::click());
        let on = chosen == i;
        let hover = ui.ctx().animate_bool(id.with(("hover", i)), response.hovered() && !on);
        if hover > 0.0 {
            ui.painter()
                .rect_filled(rect, widgets::inner_radius(ui.visuals()), p.raised.gamma_multiply(0.5 * hover));
        }
        let ink = if on { chosen_ink } else { p.muted.lerp_to_gamma(p.ink, hover) };
        let glyph = if label.is_empty() {
            rect.center()
        } else {
            egui::pos2(rect.center().x, rect.top() + 14.0)
        };
        match glyph_face {
            Face::Icon(name) => crate::ui::viewfinder::icon(
                ui.painter(),
                name,
                egui::Rect::from_center_size(glyph, egui::Vec2::splat(16.0)),
                ink,
            ),
            Face::Letter(letter) => {
                ui.painter().text(
                    glyph,
                    egui::Align2::CENTER_CENTER,
                    letter,
                    egui::FontId::new(12.5, crate::ui::fonts::semibold(ui.ctx())),
                    ink,
                );
            }
        }
        if !label.is_empty() {
            let galley = ui
                .painter()
                .layout_no_wrap((*label).to_owned(), egui::FontId::proportional(10.5), ink);
            let left = rect.center().x - galley.size().x.min(rect.width() - 4.0) * 0.5;
            ui.painter().with_clip_rect(rect.shrink(2.0)).galley(
                egui::pos2(left, rect.bottom() - 9.0 - galley.size().y * 0.5),
                galley,
                ink,
            );
        }
        let name = if label.is_empty() { *tip } else { *label };
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, on, name));
        if response.on_hover_text(*tip).clicked() && !on {
            picked = Some(i);
        }
    }
    picked
}

/// A track holding a few small buttons side by side (`square`), so that a
/// set (the view toggles, the files) reads as one cluster.
fn cluster<R>(ui: &mut egui::Ui, p: &Palette, count: usize, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let size = egui::vec2(
        count as f32 * BUTTON + (count.max(1) - 1) as f32 * 2.0 + INSET * 2.0,
        BUTTON + INSET * 2.0,
    );
    let (bar, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(bar, ui.visuals().widgets.inactive.corner_radius, p.bar);
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(bar.shrink(INSET))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    inner.spacing_mut().item_spacing.x = 2.0;
    add(&mut inner)
}

/// A help tag (Apple's): the control's name in bold, then one short line
/// saying what it does.
pub(super) fn help(response: egui::Response, p: &Palette, title: &str, detail: &str) -> egui::Response {
    response.on_hover_ui(|ui| {
        ui.set_max_width(240.0);
        ui.label(
            egui::RichText::new(title)
                .font(egui::FontId::new(12.5, crate::ui::fonts::semibold(ui.ctx())))
                .color(p.ink),
        );
        if !detail.is_empty() {
            ui.label(egui::RichText::new(detail).size(11.5).color(p.muted));
        }
    })
}

/// A small square button of a cluster (`square_sized` at `BUTTON`).
fn square(ui: &mut egui::Ui, p: &Palette, icon: &str, (title, detail): (&str, &str), on: bool, enabled: bool) -> egui::Response {
    square_sized(ui, p, egui::Vec2::splat(BUTTON), icon, (title, detail), on, enabled)
}

/// A button of a cluster: raised under the pointer, and while `on` raised
/// with its icon lit in the accent (a quiet "on", as the modes); its help
/// tag names it (`help`); its response anchors a menu.
fn square_sized(
    ui: &mut egui::Ui,
    p: &Palette,
    size: egui::Vec2,
    icon: &str,
    (title, detail): (&str, &str),
    on: bool,
    enabled: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
    let hover = ui
        .ctx()
        .animate_bool(response.id.with("hover"), enabled && (open || response.hovered()));
    let lift = if on { 1.0 } else { 0.5 * hover };
    if lift > 0.0 {
        ui.painter()
            .rect_filled(rect, widgets::inner_radius(ui.visuals()), p.raised.gamma_multiply(lift));
    }
    let ink = match (enabled, on) {
        (false, _) => p.muted_dim,
        (true, true) => p.violet_light,
        (true, false) => p.muted.lerp_to_gamma(p.ink, hover),
    };
    crate::ui::viewfinder::icon(
        ui.painter(),
        icon,
        egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(15.0)),
        ink,
    );
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, enabled, on, title));
    help(response, p, title, detail)
}

/// A track holding one button as wide as `width` (the depth of field,
/// between the modes and the sides).
fn wide_cluster(ui: &mut egui::Ui, p: &Palette, width: f32, add: impl FnOnce(&mut egui::Ui, egui::Vec2)) {
    let (bar, _) = ui.allocate_exact_size(egui::vec2(width, BUTTON + INSET * 2.0), Sense::hover());
    ui.painter().rect_filled(bar, ui.visuals().widgets.inactive.corner_radius, p.bar);
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(bar.shrink(INSET))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    add(&mut inner, bar.shrink(INSET).size());
}

/// The Fine-tune panel's header rows: smaller than the panel's rows, they
/// introduce the sliders.
const HEADER_H: f32 = 24.0;
/// The least width of a header row's cell: "Change", with room.
const HEADER_CELL: f32 = 68.0;

/// A row of the Fine-tune header: its name on the left, a small two-way
/// choice on the right, as an Apple inspector row; both choices as wide as
/// the wider word. Returns the cell clicked, if it was not the chosen one.
fn labelled_choice(ui: &mut egui::Ui, p: &Palette, id: &str, label: &str, chosen: usize, choices: &[(&str, &str); 2]) -> Option<usize> {
    let mut picked = None;
    ui.horizontal(|ui| {
        ui.set_min_height(HEADER_H);
        ui.label(egui::RichText::new(label).size(12.0).color(p.muted));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let font = egui::FontId::new(12.5, crate::ui::fonts::semibold(ui.ctx()));
            let widest = choices
                .iter()
                .map(|(word, _)| ui.painter().layout_no_wrap((*word).to_owned(), font.clone(), p.ink).size().x)
                .fold(0.0, f32::max);
            // one width for every header row's cells, so that they line up
            let cell = egui::vec2((widest + 18.0).max(HEADER_CELL), HEADER_H - 2.0 * INSET);
            let cells = choices.map(|(word, tip)| (Face::Letter(word), "", tip));
            picked = track(ui, p, id, cell, chosen, Lit::Quiet, &cells);
        });
    });
    picked
}

/// The gap between the sets of a row (the two tracks and the switch
/// between them; the two clusters and Start / Stop between them).
const ROW_GAP: f32 = 8.0;

/// The selected joint's values as a stat line: each axis letter muted, its
/// value bright; degrees for Rotation, metres for Position, times for Scale.
fn readout(p: &Palette, transform: usize, value: glam::Vec3) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    // the same size for both, so that they share a baseline
    let letter = egui::TextFormat::simple(egui::FontId::proportional(11.5), p.muted);
    let number = egui::TextFormat::simple(egui::FontId::proportional(11.5), p.ink);
    let decimals = if transform == 0 { 0 } else { 2 };
    for (i, (axis, v)) in ["X", "Y", "Z"].iter().zip(value.to_array()).enumerate() {
        // rounded first, so that a tiny negative reads 0, never -0
        let scale = 10f64.powi(decimals);
        let v = (f64::from(v) * scale).round() / scale + 0.0;
        let v = if v == 0.0 { 0.0 } else { v };
        let number_text = aurora_i18n::number(v, decimals as usize);
        let text = match transform {
            0 => format!("{number_text}°"),
            1 => number_text,
            _ => format!("{number_text}×"),
        };
        job.append(axis, if i == 0 { 0.0 } else { 8.0 }, letter.clone());
        job.append(&text, 3.0, number.clone());
    }
    job
}

impl Editor {
    /// The compact window; its title bar's actions are set like the full
    /// window's (`poser_ui` handles them).
    pub(super) fn compact_window(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        world: &World,
        frame: &Frame,
        open: &mut bool,
        actions: Vec<(&str, &str, &mut bool)>,
    ) {
        let mut floater = Floater::new(
            COMPACT_WINDOW,
            aurora_i18n::tr!("poser-title"),
            egui::pos2(1000.0, 90.0),
            COMPACT_SIZE,
        )
        .fixed()
        .auto_height();
        if self.over_viewfinder {
            floater.order = egui::Order::Foreground;
        }
        floater.actions = actions;
        let mut anchor = None;
        // a bone inside the body chosen in the full window's list is posed
        // there alone (`refresh_other_bones`): not here, where it would have
        // no marker nor handle
        self.drop_inner_selection(&world.avatar_lib.rig);
        floater.show(ctx, p, open, |ui| {
            anchor = Some(egui::pos2(ui.max_rect().right(), ui.cursor().top()));
            ui.set_width(BODY_W);
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 8.0);
            let rig = &world.avatar_lib.rig;
            // the head apart; under it every row the same distance apart
            self.compact_head(ui, p, world);
            ui.add_space(6.0);
            self.compact_groups(ui, p, rig, frame);
            self.compact_transform(ui, p, rig, frame);
            self.compact_joint(ui, p, rig, frame);
            self.compact_tools(ui, p, rig, frame);
            if self.poser.storage.busy {
                let width = ui.available_width();
                widgets::progress(
                    ui,
                    p,
                    "poser-compact-busy",
                    1.0,
                    width,
                    aurora_i18n::text("anim-edit-building"),
                    None,
                );
            } else if let Some(message) = &self.poser.storage.message {
                ui.label(egui::RichText::new(message).size(11.5).color(p.muted));
            }
        });
        self.poser.options_anchor = anchor;
    }

    /// Unselect a bone inside the body (`inner_bone`), as a click beside
    /// does: the compact window poses only what it shows.
    pub(super) fn drop_inner_selection(&mut self, rig: &Rig) {
        if self.selected.is_some_and(|b| inner_bone(&rig.canonical_names[b])) {
            self.selected = None;
            self.poser.unselected = true;
        }
    }

    /// The gear's popover: only the options the compact window does not
    /// show itself (the full window's settings panel holds them all, with
    /// the trackball's and the files' that this window has elsewhere: the
    /// depth of field in its row, the handles in Fine-tune, the export
    /// options in Save's form). A menu of the theme's rows under the title
    /// bar, against the window's right edge, as a popover of Apple's
    /// toolbars; it stays open while its switches change and closes on a
    /// click outside. Closing the window's dependent option shows only
    /// while its parent is on, as iOS Settings does.
    pub(super) fn compact_options(&mut self, ctx: &egui::Context, p: &Palette) {
        let Some(corner) = self.poser.options_anchor.filter(|_| self.poser.options_open) else {
            return;
        };
        let id = egui::Id::unique("poser-compact-options");
        let width = ctx.read_response(id).map_or(240.0, |r| r.rect.width());
        let mut open = true;
        menu::popup_at(ctx, id, egui::pos2(corner.x - width, corner.y), &mut open, p, |ui| {
            menu::section(ui, p, aurora_i18n::text("poser-options-closing"));
            menu::toggle(
                ui,
                p,
                "door-exit",
                aurora_i18n::text("poser-options-stop"),
                &mut self.poser.stop_on_close,
            );
            if self.poser.stop_on_close {
                menu::toggle(
                    ui,
                    p,
                    "alert-triangle-filled",
                    aurora_i18n::text("poser-options-confirm-close"),
                    &mut self.poser.confirm_close,
                );
            }
            menu::separator(ui, p);
            menu::section(ui, p, aurora_i18n::text("poser-options-handles"));
            if menu::toggle(
                ui,
                p,
                "rotate-3d",
                aurora_i18n::text("poser-options-natural"),
                &mut self.poser.natural,
            ) {
                // as the full window's switch: the gizmo turns in the
                // joint's own axes, or the world's
                self.space = if self.poser.natural { Space::Local } else { Space::World };
                self.gizmo.cancel();
            }
            menu::separator(ui, p);
            menu::section(ui, p, aurora_i18n::text("poser-options-files"));
            menu::toggle(
                ui,
                p,
                "replace-filled",
                aurora_i18n::text("poser-options-overwrite"),
                &mut self.poser.confirm_overwrite,
            );
        });
        self.poser.options_open = open;
    }

    /// The subject's picture (a click opens the subject menu), its name and
    /// state (a teal dot that breathes while it is posed), the others posed
    /// (a click poses that one) and Start / Stop.
    fn compact_head(&mut self, ui: &mut egui::Ui, p: &Palette, world: &World) {
        let rig = &world.avatar_lib.rig;
        let session = self.poser.sessions.get(&self.poser.subject);
        let enabled = session.is_some_and(|s| s.enabled);
        let count = session.map_or(0, |s| (0..rig.len()).filter(|b| s.edited(*b)).count());
        let initials_of = |id: Uuid| subjects::subject_short_name(world, id);
        let mut others: Vec<(Uuid, String)> = self
            .poseable_others(world)
            .into_iter()
            .map(|id| (id, subjects::subject_short_name(world, id)))
            .collect();
        // you first, then by name
        others.sort_by(|a, b| (a.0 != world.agent_id, &a.1).cmp(&(b.0 != world.agent_id, &b.1)));
        // you are "You"; a resident by their display name alone
        let name = if self.poser.subject == world.agent_id {
            subjects::subject_name(world, self.poser.subject)
        } else {
            subjects::subject_short_name(world, self.poser.subject)
        };
        let now = ui.input(|i| i.time);
        let subject_pic = self.picture(self.poser.subject, now);
        let other_pics: Vec<_> = others.iter().map(|(id, _)| self.picture(*id, now)).collect();
        let mut pick = None;
        ui.horizontal(|ui| {
            ui.set_min_height(HEAD_DISC + 4.0);
            // the picture: the subject menu opens under it
            // the ring's outer edge lines up with the groups' first cell,
            // `INSET` inside the window's edge (the ring reaches 0.5 past
            // the picture's box)
            let nudge = INSET + 0.5;
            let (rect, response) = ui.allocate_exact_size(egui::vec2(HEAD_DISC + 4.0 + nudge, HEAD_DISC + 4.0), Sense::click());
            let disc = egui::Rect::from_center_size(rect.center() + egui::vec2(nudge * 0.5, 0.0), egui::Vec2::splat(HEAD_DISC));
            let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
            widgets::avatar_disc(ui, p, disc, subject_pic.as_ref(), &initials_of(self.poser.subject), !enabled);
            let ring = if open || response.hovered() {
                Some(p.ink.gamma_multiply(0.55))
            } else if enabled {
                Some(p.teal)
            } else {
                None
            };
            if let Some(ring) = ring {
                ui.painter()
                    .circle_stroke(disc.center(), HEAD_DISC * 0.5 + 1.5, Stroke::new(2.0, ring));
            }
            // a small chevron on the picture's corner: it opens a menu
            let badge = egui::pos2(disc.right() - 4.0, disc.bottom() - 4.0);
            ui.painter().circle_filled(badge, 7.5, p.raised.lerp_to_gamma(p.ink, 0.3));
            ui.painter().circle_stroke(badge, 7.5, Stroke::new(1.5, p.panel));
            crate::ui::viewfinder::icon(
                ui.painter(),
                "chevron-down",
                egui::Rect::from_center_size(badge, egui::Vec2::splat(10.0)),
                p.ink,
            );
            let tip = aurora_i18n::text("poser-subject-tip");
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
            let response = response.on_hover_text(tip);
            let menu = egui::Popup::default_response_id(&response);
            // AURORA_DEMO_POSER=compact-invite: the menu opens by itself
            let demo = egui::Id::unique("poser-subjects-open");
            if ui.ctx().data_mut(|d| d.remove_temp::<bool>(demo)).unwrap_or(false) {
                egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&response));
            }
            menu::dropdown(&response, p, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
                self.subject_menu(ui, p, world);
            });
            // the others posed, against the right edge
            let pile = Pile::of(others.len());
            let others_w = pile.width() + if others.is_empty() { 0.0 } else { 6.0 };
            let text_w = (ui.available_width() - others_w - 6.0).max(40.0);
            let (text, text_hover) = ui.allocate_exact_size(egui::vec2(text_w, HEAD_DISC), Sense::hover());
            let title = egui::FontId::new(14.0, crate::ui::fonts::semibold(ui.ctx()));
            let dot = egui::pos2(text.left() + 3.5, text.center().y + 9.0);
            if enabled {
                let t = ui.input(|i| i.time) as f32;
                let breath = 0.5 + 0.5 * (t * 2.4).sin();
                ui.painter()
                    .circle_filled(dot, 3.5 + 3.0 * breath, p.teal.gamma_multiply(0.35 * (1.0 - breath)));
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
            }
            ui.painter().circle_filled(dot, 3.0, if enabled { p.teal } else { p.muted_dim });
            let status = if enabled {
                aurora_i18n::tr!("poser-active-status", count = count)
            } else {
                aurora_i18n::tr!("poser-stopped-status", count = count)
            };
            let name_cut = widgets::paint_elided(ui, &name, egui::pos2(text.left(), text.center().y - 8.0), text_w, title, p.ink);
            let status_cut = widgets::paint_elided(
                ui,
                &status,
                egui::pos2(text.left() + 11.0, text.center().y + 9.0),
                text_w - 11.0,
                egui::FontId::proportional(11.5),
                p.muted,
            );
            // a cut name or state shows whole under the pointer
            if name_cut || status_cut {
                text_hover.on_hover_text(format!("{name}\n{status}"));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                // the others as a stack of pictures, each over the one before
                // it; past `PILE_MAX`, a "+n" opens the subject menu
                let (area, _) = ui.allocate_exact_size(egui::vec2(pile.width(), OTHER_DISC + 4.0), Sense::hover());
                for (k, slot) in (0..pile.slots()).enumerate() {
                    let rect = egui::Rect::from_min_size(
                        egui::pos2(area.left() + k as f32 * PILE_STEP, area.center().y - OTHER_DISC * 0.5),
                        egui::Vec2::splat(OTHER_DISC),
                    );
                    let response = ui.interact(rect, ui.scope_id().with(("poser-pile", slot)), Sense::click());
                    // a ring of the window's colour parts it from the one under
                    ui.painter().circle_filled(rect.center(), OTHER_DISC * 0.5 + 2.0, p.panel);
                    if slot < pile.shown {
                        let (id, other) = &others[slot];
                        let posed = self.poser.sessions.get(id).is_some_and(|s| s.enabled);
                        widgets::avatar_disc(ui, p, rect, other_pics[slot].as_ref(), &initials_of(*id), !posed);
                        if response.hovered() {
                            ui.painter()
                                .circle_stroke(rect.center(), OTHER_DISC * 0.5 + 1.0, Stroke::new(1.5, p.ink.gamma_multiply(0.7)));
                        }
                        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, other));
                        if response
                            .on_hover_text(aurora_i18n::tr!("poser-switch-to", name = other.as_str()))
                            .clicked()
                        {
                            pick = Some(*id);
                        }
                    } else {
                        let more = format!("+{}", others.len() - pile.shown);
                        let face = if response.hovered() {
                            p.raised.lerp_to_gamma(p.ink, 0.2)
                        } else {
                            p.raised
                        };
                        ui.painter().circle_filled(rect.center(), OTHER_DISC * 0.5, face);
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            &more,
                            egui::FontId::new(10.5, crate::ui::fonts::semibold(ui.ctx())),
                            p.ink,
                        );
                        let tip = aurora_i18n::tr!("poser-more-people", count = others.len() - pile.shown);
                        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &tip));
                        if response.on_hover_text(tip.as_str()).clicked() {
                            egui::Popup::open_id(ui.ctx(), menu);
                        }
                    }
                }
            });
        });
        if let Some(id) = pick {
            self.pick_subject(world, id);
        }
    }

    /// Body, Face, Hands, Other and Physics, as the full window's tabs.
    fn compact_groups(&mut self, ui: &mut egui::Ui, p: &Palette, rig: &Rig, frame: &Frame) {
        let cells = [
            (
                Face::Icon("stretching"),
                aurora_i18n::text("poser-body"),
                aurora_i18n::text("poser-body"),
            ),
            (
                Face::Icon("mood-smile-filled"),
                aurora_i18n::text("poser-face"),
                aurora_i18n::text("poser-face"),
            ),
            (
                Face::Icon("hand-stop"),
                aurora_i18n::text("poser-hands"),
                aurora_i18n::text("poser-hands"),
            ),
            (
                Face::Icon("bone-filled"),
                aurora_i18n::text("poser-misc-short"),
                aurora_i18n::text("poser-misc"),
            ),
            (
                Face::Icon("ripple"),
                aurora_i18n::text("poser-physics"),
                aurora_i18n::text("poser-physics"),
            ),
        ];
        let cell = egui::vec2((ui.available_width() - INSET * 2.0) / cells.len() as f32, GROUPS_H - INSET * 2.0);
        if let Some(group) = track(ui, p, "groups", cell, self.poser.group, Lit::Accent, &cells) {
            self.poser.group = group;
            self.poser.library_open = false;
            self.select_pose_group(group, rig, frame);
        }
    }

    /// Rotation, Position and Scale, then Left, Right and Mirror: two
    /// three-way toggles.
    fn compact_transform(&mut self, ui: &mut egui::Ui, p: &Palette, rig: &Rig, frame: &Frame) {
        let modes = [
            (Face::Icon("rotate-360"), "", aurora_i18n::text("poser-mode-rotation")),
            (Face::Icon("arrows-move"), "", aurora_i18n::text("poser-mode-position")),
            (Face::Icon("resize"), "", aurora_i18n::text("poser-mode-scale")),
        ];
        // the sides' initials, in the viewer's language
        let left: String = aurora_i18n::text("poser-side-left").chars().take(1).collect();
        let right: String = aurora_i18n::text("poser-side-right").chars().take(1).collect();
        let sides = [
            (Face::Letter(&left), "", aurora_i18n::text("poser-side-left-tip")),
            (Face::Letter(&right), "", aurora_i18n::text("poser-side-right-tip")),
            (Face::Icon("arrows-horizontal"), "", aurora_i18n::text("poser-side-mirror-tip")),
        ];
        let (mut mode, mut side) = (None, None);
        ui.horizontal(|ui| {
            // the two tracks at the edges, the depth of field filling the
            // room between them, an even gap on each side
            ui.spacing_mut().item_spacing.x = 0.0;
            let track_w = 3.0 * TOGGLE.x + 2.0 * INSET;
            mode = track(ui, p, "modes", TOGGLE, self.poser.transform, Lit::Quiet, &modes);
            ui.add_space(ROW_GAP);
            let width = ui.available_width() - track_w - ROW_GAP;
            wide_cluster(ui, p, width, |ui, size| {
                let dof = self.poser.dof;
                let tag = (aurora_i18n::text("poser-tip-dof"), aurora_i18n::text("poser-tip-dof-detail"));
                if square_sized(ui, p, size, "aperture", tag, dof, true).clicked() {
                    self.poser.dof = !dof;
                }
            });
            ui.add_space(ROW_GAP);
            side = track(ui, p, "sides", TOGGLE, self.poser.side as usize, Lit::Quiet, &sides);
        });
        if let Some(i) = mode {
            self.poser.transform = i;
            self.tool = if i == 0 { Tool::Rotate } else { Tool::Move };
            self.gizmo.cancel();
            self.poser.scale_gizmo.cancel();
        }
        if let Some(i) = side {
            self.set_pose_side([Side::Left, Side::Right, Side::Mirror][i], rig, frame);
        }
    }

    /// The selected joint and what the transform does with it, with its
    /// reset; without one, how to choose it.
    fn compact_joint(&mut self, ui: &mut egui::Ui, p: &Palette, rig: &Rig, frame: &Frame) {
        let (row, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), Sense::hover());
        // open, the row and its sliders are one card: the row keeps its top
        // corners round, the panel under it its bottom ones
        let open = self.poser.joint_open && self.selected.is_some();
        let round = ui.visuals().window_corner_radius;
        let row_corners = if open {
            egui::CornerRadius { sw: 0, se: 0, ..round }
        } else {
            round
        };
        ui.painter().rect_filled(row, row_corners, p.field.gamma_multiply(0.55));
        let Some(bone) = self.selected else {
            let hint = if self.poser.group == OTHER_GROUP && self.poser.other_used.is_empty() {
                // a mesh may use the bones inside the body: they are the
                // full window's (`refresh_other_bones`)
                aurora_i18n::text(if self.poser.other_inner_used.is_empty() {
                    "poser-other-none"
                } else {
                    "poser-other-inner"
                })
            } else {
                aurora_i18n::text("poser-pick-joint")
            };
            widgets::paint_elided(
                ui,
                hint,
                egui::pos2(row.left() + 10.0, row.center().y),
                row.width() - 20.0,
                egui::FontId::proportional(12.0),
                p.muted,
            );
            return;
        };
        let title = self.joint_title(rig, bone);
        // the disclosure on the left, as an Apple list: the chevron and the
        // name are one target that opens the sliders under the row
        let font = egui::FontId::new(13.0, crate::ui::fonts::semibold(ui.ctx()));
        // the joint's values as a stat line (X 12° · Y -4° · Z 0°) against
        // the right edge; the name takes all the room they leave, never
        // less than a third of the row (the values clip first)
        let values = self
            .joint_value(rig, frame, bone)
            .map(|value| ui.painter().layout_job(readout(p, self.poser.transform, value)));
        let values_w = values.as_ref().map_or(0.0, |g| g.size().x + 10.0);
        let edited = self.poser_edited(bone);
        let name_left = row.left() + 26.0 + if edited { 11.0 } else { 0.0 };
        let title_w = ui
            .painter()
            .layout_no_wrap(title.clone(), font.clone(), p.ink)
            .size()
            .x
            .min((row.right() - 10.0 - values_w - name_left).max(row.width() * 0.35));
        let target = egui::Rect::from_min_max(row.min, egui::pos2(name_left + title_w + 8.0, row.max.y));
        let disclosure = ui.interact(target, ui.scope_id().with("poser-joint-disclosure"), Sense::click());
        let hover = ui.ctx().animate_bool(disclosure.id.with("hover"), disclosure.hovered());
        if hover > 0.0 {
            ui.painter().rect_filled(
                target.shrink(2.0),
                widgets::inner_radius(ui.visuals()),
                p.raised.gamma_multiply(0.5 * hover),
            );
        }
        let chevron = if self.poser.joint_open { "chevron-down" } else { "chevron-right" };
        crate::ui::viewfinder::icon(
            ui.painter(),
            chevron,
            egui::Rect::from_center_size(egui::pos2(row.left() + 14.0, row.center().y), egui::Vec2::splat(14.0)),
            p.muted.lerp_to_gamma(p.ink, hover),
        );
        // an edited joint has the violet dot of the markers before its name
        if edited {
            ui.painter()
                .circle_filled(egui::pos2(row.left() + 29.0, row.center().y), 3.0, p.violet_light);
        }
        widgets::paint_elided(ui, &title, egui::pos2(name_left, row.center().y), title_w, font, p.ink);
        let (tag, detail) = if self.poser.joint_open {
            ("poser-tip-sliders-hide", "poser-tip-sliders-hide-detail")
        } else {
            ("poser-tip-sliders-show", "poser-tip-sliders-show-detail")
        };
        if help(disclosure, p, aurora_i18n::text(tag), aurora_i18n::text(detail)).clicked() {
            self.poser.joint_open = !self.poser.joint_open;
        }
        let left = name_left;
        if let Some(galley) = values {
            let readout_left = (row.right() - 10.0 - galley.size().x).max(left + title_w + 10.0);
            let at = widgets::row_text_origin(ui.painter(), &galley, readout_left, row.center().y);
            ui.painter()
                .with_clip_rect(egui::Rect::from_min_max(
                    egui::pos2(left + title_w + 10.0, row.top()),
                    egui::pos2(row.right() - 6.0, row.bottom()),
                ))
                .galley(at, galley, p.ink);
        }
        if !self.poser.joint_open {
            return;
        }
        let width = ui.available_width();
        // flush under the row, the same face: one card
        ui.add_space(-ui.spacing().item_spacing.y);
        egui::Frame::new()
            .fill(p.field.gamma_multiply(0.55))
            .corner_radius(egui::CornerRadius { nw: 0, ne: 0, ..round })
            .inner_margin(egui::Margin {
                left: 10,
                right: 10,
                top: 2,
                bottom: 8,
            })
            .show(ui, |ui| {
                ui.set_width(width - 20.0);
                ui.spacing_mut().item_spacing.y = 2.0;
                // two labelled rows, quieter than the panel's: the handles
                // (out of the sliders' way) and what the values count
                let handles = [
                    (aurora_i18n::text("poser-handles-show"), aurora_i18n::text("poser-tip-handles-show")),
                    (aurora_i18n::text("poser-handles-hide"), aurora_i18n::text("poser-tip-handles-hide")),
                ];
                if let Some(i) = labelled_choice(
                    ui,
                    p,
                    "handles",
                    aurora_i18n::text("poser-handles"),
                    usize::from(!self.poser.gizmo),
                    &handles,
                ) {
                    self.poser.gizmo = i == 0;
                }
                let values = [
                    (aurora_i18n::text("poser-values-total"), aurora_i18n::text("poser-tip-values-total")),
                    (
                        aurora_i18n::text("poser-values-change"),
                        aurora_i18n::text("poser-tip-values-change"),
                    ),
                ];
                if let Some(i) = labelled_choice(
                    ui,
                    p,
                    "values",
                    aurora_i18n::text("poser-values"),
                    usize::from(self.poser.delta),
                    &values,
                ) {
                    self.poser.delta = i == 1;
                }
                ui.add_space(4.0);
                // read after the header: Change counts another way
                let Some(mut value) = self.joint_value(rig, frame, bone) else {
                    return;
                };
                let base_point = self
                    .poser
                    .sessions
                    .get(&self.poser.subject)
                    .filter(|_| self.poser.transform == 1)
                    .map(|s| s.base.matrices(rig, frame.model)[bone].w_axis.truncate());
                if self.axis_sliders(ui, p, &mut value, base_point) {
                    self.apply_joint_value(rig, frame, bone, value);
                }
            });
    }

    /// Three clusters, one kind of control each, as Apple's toolbars group
    /// theirs: picking on the left (switches: markers Always or On hover,
    /// Ctrl + click body parts, the other avatars' hints), going back in the middle
    /// (actions: undo, redo, reset), the files on the right (Save as a small
    /// form, Load the pose library, the upload). Under them Start / Stop
    /// alone across the window, the one main action, as a sheet's bottom
    /// button (Apple) or Lock In (Valorant).
    fn compact_tools(&mut self, ui: &mut egui::Ui, p: &Palette, rig: &Rig, frame: &Frame) {
        let free = !self.poser.storage.busy;
        ui.horizontal(|ui| {
            // the clusters at the edges and in the middle, an even gap between
            ui.spacing_mut().item_spacing.x = 0.0;
            let cluster_w = 3.0 * BUTTON + 2.0 * 2.0 + 2.0 * INSET;
            let gap = ((ui.available_width() - 3.0 * cluster_w) * 0.5).max(0.0);
            // picking: joints, Ctrl + click body parts, switching avatar
            cluster(ui, p, 3, |ui| {
                let hover = self.poser.markers && self.poser.markers_hover;
                // turned off in the full window (its eye): say so, a click
                // brings them back
                let (icon, tag) = if !self.poser.markers {
                    ("eye-off", ("poser-tip-joints-off", "poser-tip-joints-off-detail"))
                } else if hover {
                    ("hand-click", ("poser-tip-joints-hover", "poser-tip-joints-hover-detail"))
                } else {
                    ("eye-filled", ("poser-tip-joints-always", "poser-tip-joints-always-detail"))
                };
                if square(ui, p, icon, (aurora_i18n::text(tag.0), aurora_i18n::text(tag.1)), hover, true).clicked() {
                    // hidden markers come back: the compact window poses with them
                    self.poser.markers_hover = self.poser.markers && !self.poser.markers_hover;
                    self.poser.markers = true;
                }
                let on = self.poser.part_hints;
                let tag = (aurora_i18n::text("poser-tip-parts"), aurora_i18n::text("poser-tip-parts-detail"));
                if square(ui, p, "click-filled", tag, on, true).clicked() {
                    self.poser.part_hints = !on;
                }
                let hints = self.poser.avatar_hints;
                let tag = (aurora_i18n::text("poser-tip-switch"), aurora_i18n::text("poser-tip-switch-detail"));
                if square(ui, p, "users", tag, hints, true).clicked() {
                    self.poser.avatar_hints = !hints;
                }
            });
            ui.add_space(gap);
            // going back: the history, then the reset (Photos' Revert next
            // to its undo); Shift turns the reset to the whole pose, the way
            // the Option key turns a menu item on the Mac
            cluster(ui, p, 3, |ui| {
                let (can_undo, can_redo) = self
                    .poser
                    .sessions
                    .get(&self.poser.subject)
                    .map_or((false, false), |s| (!s.undo.is_empty(), !s.redo.is_empty()));
                let mut history = None;
                if square(ui, p, "arrow-back-up", (aurora_i18n::text("poser-undo"), ""), false, can_undo).clicked() {
                    history = Some(false);
                }
                if square(ui, p, "arrow-forward-up", (aurora_i18n::text("poser-redo"), ""), false, can_redo).clicked() {
                    history = Some(true);
                }
                if let Some(redo) = history
                    && let Some(session) = self.poser.sessions.get_mut(&self.poser.subject)
                {
                    session.history(redo);
                    self.refresh = true;
                }
                let whole = ui.input(|i| i.modifiers.shift);
                let title = if whole {
                    aurora_i18n::text("poser-tip-reset-pose").to_owned()
                } else if let Some(bone) = self.selected {
                    aurora_i18n::tr!("poser-tip-reset-joint", joint = self.joint_title(rig, bone)).to_string()
                } else {
                    aurora_i18n::text("poser-tip-reset").to_owned()
                };
                let detail = aurora_i18n::text(if whole {
                    "poser-tip-reset-pose-detail"
                } else {
                    "poser-tip-reset-detail"
                });
                if square(ui, p, "restore", (&title, detail), false, self.reset_available(whole)).clicked() {
                    self.pose_operation(
                        rig,
                        frame,
                        if whole {
                            controls::Operation::ResetAll
                        } else {
                            controls::Operation::ResetJoint
                        },
                    );
                }
            });
            ui.add_space(gap);
            // the files
            cluster(ui, p, 3, |ui| {
                let tag = (aurora_i18n::text("poser-tip-save"), aurora_i18n::text("poser-tip-save-detail"));
                let save = square(ui, p, "device-floppy-filled", tag, false, free);
                // AURORA_DEMO_POSER=compact-save: the form opens by itself
                let demo = egui::Id::unique("poser-save-open");
                if ui.ctx().data_mut(|d| d.remove_temp::<bool>(demo)).unwrap_or(false) {
                    egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&save));
                }
                self.save_form(&save, p, rig, frame);
                let tag = (aurora_i18n::text("poser-tip-load"), aurora_i18n::text("poser-tip-load-detail"));
                let load = square(ui, p, "folder-open-filled", tag, false, free);
                self.load_list(&load, p);
                let tag = (aurora_i18n::text("poser-upload"), aurora_i18n::text("poser-export-hint"));
                if square(ui, p, "file-upload-filled", tag, false, free).clicked() {
                    self.import_pose(rig, frame);
                }
            });
        });
        if let Some(session) = self.poser.sessions.get_mut(&self.poser.subject) {
            let (icon, key, kind) = if session.enabled {
                ("player-stop-filled", "poser-stop-short", widgets::ActionKind::Flat)
            } else {
                ("player-play-filled", "poser-start-short", widgets::ActionKind::Primary)
            };
            let tip = aurora_i18n::text(if session.enabled { "poser-stop" } else { "poser-start" });
            let width = ui.available_width();
            if widgets::action_button(ui, p, icon, aurora_i18n::text(key), kind, egui::vec2(width, BUTTON + 2.0 * INSET))
                .on_hover_text(tip)
                .clicked()
            {
                session.enabled = !session.enabled;
            }
        }
    }

    /// The selected joint's name as the window shows it: its label, and
    /// its side when it has one.
    fn joint_title(&self, rig: &Rig, bone: usize) -> String {
        let name = &rig.canonical_names[bone];
        if bones::side_of(name).is_some() {
            aurora_i18n::tr!("poser-joint-side", joint = bones::label(name), side = self.side_word()).to_string()
        } else {
            bones::label(name)
        }
    }

    /// Whether the reset would change anything, so that it greys out when
    /// it would not (Apple's rule for unavailable actions): the whole pose
    /// when `whole` (Shift) has any joint edited or locked; otherwise the
    /// selected joint, or in Mirror its counterpart too, as `ResetJoint`
    /// resets both (FSPoserAnimator::resetJoint,
    /// indra/newview/fsposeranimator.cpp).
    pub(super) fn reset_available(&self, whole: bool) -> bool {
        let Some(session) = self.poser.sessions.get(&self.poser.subject).filter(|s| s.enabled) else {
            // stopped: `pose_operation` leaves the pose as it is
            return false;
        };
        let changed = |b: usize| session.edited(b) || session.pose.locks.contains_key(&b);
        if whole {
            return !session.pose.locks.is_empty() || (0..session.pose.rotations.len()).any(|b| session.edited(b));
        }
        let mirrored = self
            .selected
            .filter(|_| self.poser.side == Side::Mirror)
            .and_then(|b| self.mirrors.counterpart(b));
        self.selected.into_iter().chain(mirrored).any(changed)
    }

    /// Save's form: the name, the export options, Save, and Save as….
    fn save_form(&mut self, anchor: &egui::Response, p: &Palette, rig: &Rig, frame: &Frame) {
        menu::dropdown(anchor, p, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
            ui.set_width(260.0);
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(aurora_i18n::text("poser-save-title"))
                        .font(egui::FontId::new(13.5, crate::ui::fonts::semibold(ui.ctx())))
                        .color(p.ink),
                );
            });
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                let field = widgets::restricted_text_field(ui, true, &mut self.poser.name, widgets::TextPolicy::PrintableNoPipe, |b, _| {
                    egui::TextEdit::singleline(b)
                        .char_limit(100)
                        .desired_width(244.0)
                        .hint_text(aurora_i18n::tr!("poser-name"))
                });
                // on opening: not over a field the user moved to
                if self.poser.name.is_empty() && ui.memory(|m| m.focused().is_none()) {
                    field.request_focus();
                }
            });
            menu::toggle(
                ui,
                p,
                "file-upload-filled",
                aurora_i18n::text("poser-also-bvh"),
                &mut self.poser.also_bvh,
            );
            menu::toggle(
                ui,
                p,
                "accessible-filled",
                aurora_i18n::text("poser-unlock-pelvis"),
                &mut self.poser.unlock_pelvis,
            );
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                ui.label(aurora_i18n::tr!("poser-priority"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(8.0);
                    ui.add(egui::DragValue::new(&mut self.poser.priority).range(0..=6));
                });
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                let ready = !self.poser.storage.busy && !self.poser.name.trim().is_empty();
                let enter = ready && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if widgets::action_button_if(
                    ui,
                    p,
                    "device-floppy-filled",
                    aurora_i18n::text("poser-save-library"),
                    widgets::ActionKind::Primary,
                    egui::vec2(244.0, 28.0),
                    ready,
                )
                .clicked()
                    || enter
                {
                    self.save_pose(rig, frame, false);
                    ui.close();
                }
            });
            // the full window's Save as…, the file chosen in the system
            // dialog: the same action, the same icon and the same rule
            let ready = !self.poser.storage.busy && !self.poser.name.trim().is_empty();
            if menu::item_if(ui, p, "device-floppy-filled", aurora_i18n::text("poser-save-as"), ready) {
                self.save_pose(rig, frame, true);
            }
            ui.add_space(4.0);
        });
    }

    /// Load's list: the poses of the library (a click loads one, in the
    /// mode chosen under them), then another file.
    fn load_list(&mut self, anchor: &egui::Response, p: &Palette) {
        menu::dropdown(anchor, p, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
            ui.set_min_width(260.0);
            let mut load = None;
            if self.poser.storage.entries.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(aurora_i18n::text("poser-library-empty"))
                            .size(12.0)
                            .color(p.muted),
                    );
                });
            } else {
                egui::ScrollArea::vertical()
                    .id_salt("poser-compact-library")
                    .max_height(280.0)
                    .show(ui, |ui| {
                        for entry in &self.poser.storage.entries {
                            if menu::item(ui, p, "stretching", &entry.name) {
                                load = Some(Some(entry.path.clone()));
                            }
                        }
                    });
            }
            menu::separator(ui, p);
            menu::submenu(
                ui,
                p,
                "adjustments-horizontal-filled",
                aurora_i18n::text("poser-load-mode"),
                true,
                |ui| {
                    for (mode, key) in [
                        (LoadMode::All, "poser-load-all"),
                        (LoadMode::Rotations, "poser-load-rotations"),
                        (LoadMode::Selective, "poser-load-selective"),
                        (LoadMode::SelectiveRotations, "poser-load-selective-rotations"),
                    ] {
                        if menu::check(
                            ui,
                            p,
                            "folder-open-filled",
                            aurora_i18n::text(key),
                            self.poser.load_mode == mode,
                            true,
                        ) {
                            self.poser.load_mode = mode;
                        }
                    }
                },
            );
            if menu::item(ui, p, "folder-filled", aurora_i18n::text("poser-open-file")) {
                load = Some(None);
            }
            if let Some(path) = load {
                // another file loads whole, as the full window's Open file
                let mode = if path.is_some() { self.poser.load_mode } else { LoadMode::All };
                self.poser.storage.job = Some(storage::Job::Load(path, mode, self.poser.subject));
                ui.close();
            }
        });
    }
}
