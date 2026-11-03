//! The photo reel on screen (`snapshot::reel`): the stack of the session's
//! photos beside the viewfinder's shutter and in the photo window, and the
//! quick view that opens from it, to scroll them, send one again or delete
//! it. Aurora's own: the Legacy Viewer only shows the last photo.

use crate::snapshot::reel::{self, Photo, Place};
use crate::theme::Palette;
use crate::ui::snapshot::Destination;
use crate::ui::widgets::{self, ActionKind, DialogButton};
use egui::{Align2, Color32, Id, Key, Order, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use std::collections::HashMap;

/// What the reel asks of the app.
#[derive(Debug, Clone, PartialEq)]
pub enum ReelAction {
    /// The quick view opened on this photo, or moved to it.
    Show(u64),
    Close,
    /// Sent again: held, then the destination's own flow.
    Send(u64, Destination),
    /// Its file in the file manager, its item in the inventory, the profile.
    Reveal(u64),
    ShowItem(u64),
    ShowProfile,
    /// Deleted, confirmed.
    Delete(u64),
}

/// What the app gives the reel to draw.
#[derive(Clone, Copy)]
pub struct ReelView<'a> {
    /// Newest first.
    pub photos: &'a [Photo],
    pub thumbs: &'a HashMap<u64, egui::TextureHandle>,
    /// The quick view's pictures, at the screen's size, read on a worker.
    pub full: &'a HashMap<u64, egui::TextureHandle>,
    /// The destinations that can take a photo now (logged in, the region's
    /// postcards…), and none while one is sent.
    pub can_send: &'a [Destination],
    /// The time a photo was taken, as the clock writes it.
    pub time: &'a dyn Fn(i64) -> String,
    /// The newest photo is flying from the card to the stack: the stack
    /// shows the one under it meanwhile.
    pub arriving: bool,
}

/// The part of a picture that covers a frame of proportions `want` (width
/// over height), centred.
pub fn cover_uv(size: Vec2, want: f32) -> Rect {
    let r = size.x / size.y.max(1.0);
    if r > want {
        let w = want / r;
        Rect::from_min_max(Pos2::new(0.5 - w / 2.0, 0.0), Pos2::new(0.5 + w / 2.0, 1.0))
    } else {
        let h = r / want.max(f32::EPSILON);
        Rect::from_min_max(Pos2::new(0.0, 0.5 - h / 2.0), Pos2::new(1.0, 0.5 + h / 2.0))
    }
}

/// A thumbnail filling `rect` (the field's colour while it is made).
pub fn picture(painter: &egui::Painter, p: &Palette, rect: Rect, radius: egui::CornerRadius, tex: Option<&egui::TextureHandle>) {
    painter.rect_filled(rect, radius, p.field);
    if let Some(t) = tex {
        let uv = cover_uv(t.size_vec2(), rect.width() / rect.height().max(1.0));
        // the corners of the frame kept: the picture drawn as a textured
        // rounded rectangle
        painter.add(egui::epaint::RectShape::filled(rect, radius, widgets::PICTURE_TINT).with_texture(t.id(), uv));
    }
}

/// The viewfinder's tray: its height, the gap between its places.
pub const TRAY_HEIGHT: f32 = 44.0;
const TRAY_GAP: f32 = 6.0;

/// A photo's place in the tray (3:2, like the card's picture).
fn tray_thumb() -> Vec2 {
    Vec2::new(TRAY_HEIGHT * 1.5, TRAY_HEIGHT)
}

/// The viewfinder's tray, bottom left: the stack of the older photos, then
/// the two newest, the newest on the right; a click opens the quick view on
/// that photo. Without a photo it is not drawn (false).
pub fn tray(ui: &mut egui::Ui, p: &Palette, view: &ReelView, out: &mut Vec<ReelAction>) -> bool {
    let photos = &view.photos[usize::from(view.arriving).min(view.photos.len())..];
    if photos.is_empty() {
        return false;
    }
    let radius = ui.visuals().window_corner_radius;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = TRAY_GAP;
        if photos.len() > 2 {
            let older = ReelView {
                photos: &photos[2..],
                arriving: false,
                ..*view
            };
            let resp = stack(ui, p, &older, TRAY_HEIGHT, photos.len());
            if resp.clicked() {
                resp.surrender_focus();
                out.push(ReelAction::Show(photos[2].id));
            }
        }
        for photo in photos.iter().take(2).rev() {
            let (rect, resp) = ui.allocate_exact_size(tray_thumb(), Sense::click());
            picture(ui.painter(), p, rect, radius, view.thumbs.get(&photo.id));
            let edge = if resp.hovered() {
                p.teal.gamma_multiply(0.8)
            } else {
                p.ink.gamma_multiply(0.5)
            };
            ui.painter().rect_stroke(rect, radius, Stroke::new(1.5, edge), StrokeKind::Inside);
            let resp = resp.on_hover_text(places_text(photo));
            if resp.clicked() {
                resp.surrender_focus();
                out.push(ReelAction::Show(photo.id));
            }
        }
    });
    true
}

/// Where the tray puts its newest photo once the reel holds `len`, its
/// places starting at `origin` (their top left): the card's photo flies
/// there.
pub fn tray_slot(origin: Pos2, len: usize) -> Rect {
    let mut x = origin.x;
    if len > 2 {
        x += TRAY_HEIGHT + TRAY_GAP;
    }
    if len > 1 {
        x += tray_thumb().x + TRAY_GAP;
    }
    Rect::from_min_size(Pos2::new(x, origin.y), tray_thumb())
}

/// A point of the arc a photo flies along from `from` to `to` (`t` from 0
/// to 1): a quadratic curve whose top rises above both ends by a third of
/// the distance between them.
pub fn arc_point(from: Pos2, to: Pos2, t: f32) -> Pos2 {
    let lift = from.distance(to) / 3.0;
    let control = Pos2::new((from.x + to.x) / 2.0, from.y.min(to.y) - lift);
    let u = 1.0 - t;
    Pos2::new(
        u * u * from.x + 2.0 * u * t * control.x + t * t * to.x,
        u * u * from.y + 2.0 * u * t * control.y + t * t * to.y,
    )
}

/// The stack: the first photo of `view`, the next two peeking out behind
/// it, and `count` on a badge, in a square of `side`. Empty, nothing shows
/// but its room is kept.
pub fn stack(ui: &mut egui::Ui, p: &Palette, view: &ReelView, side: f32, count: usize) -> egui::Response {
    let sense = if view.photos.is_empty() { Sense::hover() } else { Sense::click() };
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(side), sense);
    let shown: Vec<&Photo> = view.photos.iter().skip(usize::from(view.arriving)).take(3).collect();
    if shown.is_empty() {
        return resp;
    }
    let radius = ui.visuals().window_corner_radius;
    let painter = ui.painter();
    // the cards behind, up and to the right, darker
    let step = (side * 0.1).round().max(2.0);
    for (i, photo) in shown.iter().enumerate().skip(1).rev() {
        let r = rect.translate(Vec2::new(step, -step) * i as f32).shrink(step * i as f32 * 0.25);
        picture(painter, p, r, radius, view.thumbs.get(&photo.id));
        painter.rect_filled(r, radius, p.navy.gamma_multiply(0.35 + 0.15 * i as f32));
        painter.rect_stroke(r, radius, Stroke::new(1.0, p.ink.gamma_multiply(0.25)), StrokeKind::Inside);
    }
    picture(painter, p, rect, radius, view.thumbs.get(&shown[0].id));
    let edge = if resp.hovered() {
        p.teal.gamma_multiply(0.8)
    } else {
        p.ink.gamma_multiply(0.5)
    };
    painter.rect_stroke(rect, radius, Stroke::new(1.5, edge), StrokeKind::Inside);
    // the count, on a violet pill at the bottom right
    if count > 1 {
        let font = egui::FontId::proportional(10.5);
        let text = painter.layout_no_wrap(count.to_string(), font, p.on_accent);
        // over the corner, 3 points out: a row of thumbnails leaves room
        let size = Vec2::new((text.size().x + 8.0).max(16.0), 16.0);
        let pill = Rect::from_min_size(rect.right_bottom() + Vec2::splat(3.0) - size, size);
        painter.rect_filled(pill, radius, p.violet);
        painter.galley(pill.center() - text.size() / 2.0, text, p.on_accent);
    }
    resp.on_hover_text(aurora_i18n::tr!("photo-reel-tip"))
}

/// The photo window's row: the photos of the reel after the first `skip`,
/// newest first, as many as `width` holds (the quick view shows them all);
/// a click opens the quick view on one.
pub fn strip(ui: &mut egui::Ui, p: &Palette, view: &ReelView, skip: usize, width: f32, out: &mut Vec<ReelAction>) {
    const THUMB: Vec2 = Vec2::new(54.0, 34.0);
    let radius = ui.visuals().window_corner_radius;
    let gap = ui.spacing().item_spacing.x;
    let fits = ((width + gap) / (THUMB.x + gap)).floor().max(0.0) as usize;
    for photo in view.photos.iter().skip(skip).take(fits) {
        let (rect, resp) = ui.allocate_exact_size(THUMB, Sense::click());
        picture(ui.painter(), p, rect, radius, view.thumbs.get(&photo.id));
        if resp.hovered() {
            ui.painter()
                .rect_stroke(rect, radius, Stroke::new(1.5, p.teal.gamma_multiply(0.8)), StrokeKind::Inside);
        }
        if resp.on_hover_text(places_text(photo)).clicked() {
            out.push(ReelAction::Show(photo.id));
        }
    }
}

/// Where a photo went, as a line ("Disk · Inventory"), or that it was not
/// sent.
pub fn places_text(photo: &Photo) -> String {
    if photo.places.is_empty() {
        return aurora_i18n::tr!("photo-reel-not-sent").to_owned();
    }
    let mut names: Vec<&str> = Vec::new();
    for place in &photo.places {
        let name = place_name(place);
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names.join(" · ")
}

fn place_name(place: &Place) -> &'static str {
    match place {
        Place::Disk(_) => aurora_i18n::tr!("photo-destination-disk"),
        Place::Inventory(_) => aurora_i18n::tr!("photo-destination-inventory"),
        Place::Profile => aurora_i18n::tr!("photo-destination-profile"),
        Place::Email => aurora_i18n::tr!("photo-destination-email"),
    }
}

/// The photo's destinations offered again: those it has not gone to, the
/// e-mail always (to someone else).
fn destinations(photo: &Photo) -> Vec<Destination> {
    let mut out = Vec::new();
    if photo.on_disk().is_none() {
        out.push(Destination::Disk);
    }
    if photo.item().is_none() {
        out.push(Destination::Inventory);
    }
    if !photo.went(&Place::Profile) {
        out.push(Destination::Profile);
    }
    out.push(Destination::Email);
    out
}

fn send_button(d: Destination) -> (&'static str, &'static str) {
    match d {
        Destination::Disk => ("device-floppy-filled", aurora_i18n::tr!("photo-reel-save")),
        Destination::Inventory => ("briefcase-filled", aurora_i18n::tr!("photo-reel-upload")),
        Destination::Profile => ("user-filled", aurora_i18n::tr!("photo-reel-post")),
        Destination::Email => ("mail-filled", aurora_i18n::tr!("photo-reel-email")),
    }
}

/// The quick view: the photos of the reel one at a time, over the world.
#[derive(Debug, Default)]
pub struct QuickView {
    /// The photo shown; None: closed.
    pub open: Option<u64>,
    /// Its deletion waits for its confirmation.
    confirm: Option<u64>,
    /// The last step: when, and from which side the photo comes in (-1:
    /// from the left).
    slide: Option<(f64, f32)>,
}

/// How long a photo takes to slide in.
const SLIDE: f64 = 0.22;

impl QuickView {
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    pub fn close(&mut self) {
        self.open = None;
        self.confirm = None;
        self.slide = None;
    }

    /// Ask to delete the photo shown (the demo; the button and the Delete
    /// key ask it themselves).
    pub fn ask_delete(&mut self) {
        self.confirm = self.open;
    }

    /// Show `id`, sliding in from the side it is on.
    fn go(&mut self, view: &ReelView, id: u64, now: f64, out: &mut Vec<ReelAction>) {
        if self.open == Some(id) {
            return;
        }
        let index = |id| view.photos.iter().position(|p| p.id == id);
        // the newest on the right: an older photo comes in from the left
        let side = match (self.open.and_then(index), index(id)) {
            (Some(from), Some(to)) if to > from => -1.0,
            _ => 1.0,
        };
        self.open = Some(id);
        self.slide = Some((now, side));
        out.push(ReelAction::Show(id));
    }

    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, view: &ReelView, out: &mut Vec<ReelAction>) {
        let Some(id) = self.open else {
            return;
        };
        // deleted, or pushed out of the reel
        let Some(cursor) = view.photos.iter().position(|ph| ph.id == id) else {
            self.close();
            out.push(ReelAction::Close);
            return;
        };
        let photo = &view.photos[cursor];
        let len = view.photos.len();
        let now = ctx.input(|i| i.time);
        let screen = ctx.content_rect();

        // the keys are the quick view's while it shows (the viewfinder's
        // and the camera's wait); not under the confirmation
        if self.confirm.is_none() {
            let (older, newer, close, delete, wheel) = ctx.input_mut(|i| {
                let m = egui::Modifiers::NONE;
                (
                    i.consume_key(m, Key::ArrowLeft),
                    i.consume_key(m, Key::ArrowRight),
                    i.consume_key(m, Key::Escape),
                    i.consume_key(m, Key::Delete),
                    // a notch of the wheel; taken whole, the camera does
                    // not zoom behind
                    {
                        i.smooth_scroll_delta = Vec2::ZERO;
                        i.events
                            .iter()
                            .filter_map(|e| match e {
                                egui::Event::MouseWheel { delta, .. } => Some(delta.y),
                                _ => None,
                            })
                            .sum::<f32>()
                    },
                )
            });
            let mut step = i32::from(older) - i32::from(newer);
            // the wheel down goes back in time, like a feed
            if wheel.abs() > 0.5 && self.slide.is_none_or(|(t, _)| now - t > SLIDE) {
                step += if wheel < 0.0 { 1 } else { -1 };
            }
            if step != 0 {
                let to = reel::step(len, cursor, step);
                self.go(view, view.photos[to].id, now, out);
            }
            if delete {
                self.confirm = Some(id);
            }
            if close {
                self.close();
                out.push(ReelAction::Close);
                return;
            }
        }

        // the world dimmed behind; a click outside the photo and the bars
        // closes
        let back = egui::Area::new(Id::unique("photo_reel_back"))
            .order(Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let (rect, resp) = ui.allocate_exact_size(screen.size(), Sense::click());
                ui.painter().rect_filled(rect, 0.0, p.navy.gamma_multiply(0.92));
                resp
            });
        ctx.move_to_top(back.response.layer_id);

        // the photo, as large as the room between the bars allows
        let room = Rect::from_min_max(
            Pos2::new(screen.left() + 72.0, screen.top() + 64.0),
            Pos2::new(screen.right() - 72.0, screen.bottom() - 168.0),
        );
        let tex = view.full.get(&id).or_else(|| view.thumbs.get(&id));
        // on the backdrop's layer: under the bars and the confirmation
        let painter = ctx.layer_painter(back.response.layer_id);
        let mut photo_rect = Rect::NOTHING;
        if let Some(t) = tex {
            let size = egui::vec2(photo.size.0.max(1) as f32, photo.size.1.max(1) as f32);
            let shown = size * (room.width() / size.x).min(room.height() / size.y);
            let (dx, alpha) = match self.slide {
                Some((t0, side)) if now - t0 < SLIDE => {
                    let k = ((now - t0) / SLIDE) as f32;
                    let ease = 1.0 - (1.0 - k) * (1.0 - k);
                    ctx.request_repaint();
                    (side * 48.0 * (1.0 - ease), ease)
                }
                _ => (0.0, 1.0),
            };
            photo_rect = Rect::from_center_size(room.center() + Vec2::new(dx, 0.0), shown);
            let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            // a cut-out photo over the field, its transparency in sight
            if photo.cutout {
                painter.rect_filled(photo_rect, 0.0, p.field);
            }
            painter.image(t.id(), photo_rect, uv, widgets::PICTURE_TINT.gamma_multiply(alpha));
        }
        let pointer = ctx.input(|i| i.pointer.interact_pos());
        let on_photo = pointer.is_some_and(|pos| photo_rect.contains(pos));

        let mut over_bars = false;
        // the top: which photo, when, where it went; and close
        let top = egui::Area::new(Id::unique("photo_reel_top"))
            .order(Order::Foreground)
            .pivot(Align2::CENTER_TOP)
            .fixed_pos(Pos2::new(screen.center().x, screen.top() + 16.0))
            .show(ctx, |ui| {
                widgets::overlay(ui, p, egui::Margin::symmetric(12, 6), |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        let index = aurora_i18n::tr!("photo-reel-index", index = len - cursor, count = len);
                        ui.label(egui::RichText::new(index).strong().color(p.ink));
                        ui.label(egui::RichText::new((view.time)(photo.taken)).color(p.muted));
                        ui.label(egui::RichText::new(places_text(photo)).color(p.muted));
                        if widgets::icon_button(ui, p, "x", aurora_i18n::tr!("common-close"), true) {
                            self.close();
                            out.push(ReelAction::Close);
                        }
                    });
                });
            });
        ctx.move_to_top(top.response.layer_id);
        over_bars |= top.response.hovered();
        if self.open.is_none() {
            return;
        }

        // the side arrows: older on the left, newer on the right
        for (side, icon, tip, to) in [
            (
                -1.0,
                "chevron-left",
                aurora_i18n::tr!("photo-reel-older"),
                (cursor + 1 < len).then(|| cursor + 1),
            ),
            (1.0, "chevron-right", aurora_i18n::tr!("photo-reel-newer"), cursor.checked_sub(1)),
        ] {
            let Some(to) = to else { continue };
            let x = if side < 0.0 { screen.left() + 20.0 } else { screen.right() - 20.0 };
            let arrow = egui::Area::new(Id::unique(("photo_reel_arrow", icon)))
                .order(Order::Foreground)
                .pivot(if side < 0.0 { Align2::LEFT_CENTER } else { Align2::RIGHT_CENTER })
                .fixed_pos(Pos2::new(x, room.center().y))
                .show(ctx, |ui| {
                    widgets::overlay(ui, p, egui::Margin::same(4), |ui| widgets::icon_button(ui, p, icon, tip, true)).inner
                });
            ctx.move_to_top(arrow.response.layer_id);
            over_bars |= arrow.response.hovered();
            if arrow.inner {
                self.go(view, view.photos[to].id, now, out);
            }
        }

        // the bottom: the strip of the reel, then what the photo can do
        let bottom = egui::Area::new(Id::unique("photo_reel_bottom"))
            .order(Order::Foreground)
            .pivot(Align2::CENTER_BOTTOM)
            .fixed_pos(Pos2::new(screen.center().x, screen.bottom() - 16.0))
            .show(ctx, |ui| {
                widgets::overlay(ui, p, egui::Margin::same(10), |ui| {
                    ui.spacing_mut().item_spacing.y = 10.0;
                    self.filmstrip(ui, p, view, cursor, now, out);
                    self.actions(ui, p, view, photo, out);
                });
            });
        ctx.move_to_top(bottom.response.layer_id);
        over_bars |= bottom.response.hovered();

        if back.inner.clicked() && !on_photo && !over_bars && self.confirm.is_none() {
            self.close();
            out.push(ReelAction::Close);
            return;
        }
        if let Some(target) = self.confirm {
            self.confirm_delete(ctx, p, view, target, out);
        }
    }

    /// The reel in small, oldest on the left like a phone's, the photo
    /// shown framed.
    fn filmstrip(&mut self, ui: &mut egui::Ui, p: &Palette, view: &ReelView, cursor: usize, now: f64, out: &mut Vec<ReelAction>) {
        let radius = ui.visuals().window_corner_radius;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for (i, photo) in view.photos.iter().enumerate().rev() {
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(64.0, 42.0), Sense::click());
                picture(ui.painter(), p, rect, radius, view.thumbs.get(&photo.id));
                let edge = if i == cursor {
                    Stroke::new(2.0, p.violet)
                } else if resp.hovered() {
                    Stroke::new(1.5, p.teal.gamma_multiply(0.8))
                } else {
                    Stroke::new(1.0, p.ink.gamma_multiply(0.2))
                };
                ui.painter().rect_stroke(rect, radius, edge, StrokeKind::Inside);
                if i != cursor {
                    ui.painter().rect_filled(rect, radius, p.navy.gamma_multiply(0.3));
                }
                if resp.clicked() {
                    self.go(view, photo.id, now, out);
                }
            }
        });
    }

    /// Send it again, show where it went, delete it.
    fn actions(&mut self, ui: &mut egui::Ui, p: &Palette, view: &ReelView, photo: &Photo, out: &mut Vec<ReelAction>) {
        let ready = photo.file.is_some();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for d in destinations(photo) {
                let (icon, label) = send_button(d);
                let enabled = ready && view.can_send.contains(&d);
                let resp = widgets::action_button_if(ui, p, icon, label, ActionKind::Flat, Vec2::new(0.0, 30.0), enabled);
                let resp = if ready {
                    resp
                } else {
                    resp.on_disabled_hover_text(aurora_i18n::tr!("photo-reel-preparing"))
                };
                if resp.clicked() && enabled {
                    out.push(ReelAction::Send(photo.id, d));
                }
            }
            ui.add_space(10.0);
            if photo.on_disk().is_some()
                && widgets::action_button(
                    ui,
                    p,
                    "folder-open-filled",
                    aurora_i18n::tr!("photo-reel-in-folder"),
                    ActionKind::Flat,
                    Vec2::new(0.0, 30.0),
                )
                .clicked()
            {
                out.push(ReelAction::Reveal(photo.id));
            }
            if photo.item().is_some()
                && widgets::action_button(
                    ui,
                    p,
                    "briefcase-filled",
                    aurora_i18n::tr!("photo-reel-in-inventory"),
                    ActionKind::Flat,
                    Vec2::new(0.0, 30.0),
                )
                .clicked()
            {
                out.push(ReelAction::ShowItem(photo.id));
            }
            if photo.went(&Place::Profile)
                && widgets::action_button(
                    ui,
                    p,
                    "user-filled",
                    aurora_i18n::tr!("photo-reel-on-profile"),
                    ActionKind::Flat,
                    Vec2::new(0.0, 30.0),
                )
                .clicked()
            {
                out.push(ReelAction::ShowProfile);
            }
            ui.add_space(10.0);
            if widgets::action_button(
                ui,
                p,
                "trash-filled",
                aurora_i18n::tr!("common-delete"),
                ActionKind::FlatDanger,
                Vec2::new(0.0, 30.0),
            )
            .clicked()
            {
                self.confirm = Some(photo.id);
            }
        });
    }

    /// "Delete this photo?", with what goes where and what stays.
    fn confirm_delete(&mut self, ctx: &egui::Context, p: &Palette, view: &ReelView, id: u64, out: &mut Vec<ReelAction>) {
        let Some(photo) = view.photos.iter().find(|ph| ph.id == id) else {
            self.confirm = None;
            return;
        };
        let d = reel::deletion(photo);
        let modal = widgets::Modal::new(Id::unique("photo_reel_delete")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(
                egui::RichText::new(aurora_i18n::tr!("photo-reel-delete-title"))
                    .strong()
                    .size(15.0)
                    .color(p.ink),
            );
            ui.add_space(6.0);
            let line = |ui: &mut egui::Ui, text: &str, color: Color32| {
                ui.add(egui::Label::new(egui::RichText::new(text).color(color)).wrap());
            };
            for path in &d.recycle {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let text = if cfg!(windows) {
                    aurora_i18n::tr!("photo-reel-delete-recycle", name = name.as_str())
                } else {
                    aurora_i18n::tr!("photo-reel-delete-trash-file", name = name.as_str())
                };
                line(ui, &text, p.ink);
            }
            if !d.trash.is_empty() {
                line(ui, aurora_i18n::tr!("photo-reel-delete-item"), p.ink);
            }
            if d.sent {
                line(ui, aurora_i18n::tr!("photo-reel-delete-sent"), p.muted);
            }
            if d.recycle.is_empty() && d.trash.is_empty() {
                line(ui, aurora_i18n::tr!("photo-reel-delete-reel-only"), p.muted);
            }
            ui.add_space(10.0);
            widgets::dialog_buttons(
                ui,
                p,
                &[
                    DialogButton {
                        icon: "",
                        label: aurora_i18n::tr!("common-cancel"),
                        kind: ActionKind::Flat,
                        enabled: true,
                    },
                    DialogButton {
                        icon: "trash-filled",
                        label: aurora_i18n::tr!("common-delete"),
                        kind: ActionKind::Danger,
                        enabled: true,
                    },
                ],
                32.0,
            )
        });
        // above the quick view, which raises its own layers every frame
        // (the backdrop first, the dialog over it)
        ctx.move_to_top(modal.backdrop_response.layer_id);
        ctx.move_to_top(modal.response.layer_id);
        match modal.inner {
            Some(1) => {
                self.confirm = None;
                out.push(ReelAction::Delete(id));
            }
            Some(_) => self.confirm = None,
            None if modal.should_close() => self.confirm = None,
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use uuid::Uuid;

    fn photo(places: Vec<Place>) -> Photo {
        Photo {
            id: 1,
            taken: 0,
            size: (16, 9),
            cutout: false,
            file: Some(PathBuf::from("a.png")),
            own_copy: false,
            places,
        }
    }

    #[test]
    fn offered_where_it_has_not_gone() {
        let all = [Destination::Disk, Destination::Inventory, Destination::Profile, Destination::Email];
        assert_eq!(destinations(&photo(vec![])), all);
        let sent = photo(vec![
            Place::Disk("a.png".into()),
            Place::Inventory(Uuid::nil()),
            Place::Profile,
            Place::Email,
        ]);
        // the e-mail always: to someone else
        assert_eq!(destinations(&sent), [Destination::Email]);
    }

    #[test]
    fn the_flight_arcs_above_both_ends() {
        let (from, to) = (Pos2::new(900.0, 500.0), Pos2::new(0.0, 500.0));
        assert_eq!(arc_point(from, to, 0.0), from);
        assert_eq!(arc_point(from, to, 1.0), to);
        let top = arc_point(from, to, 0.5);
        // halfway across, above both (y grows downwards)
        assert!((top.x - 450.0).abs() < 1e-3);
        assert!(top.y < 500.0 - 100.0);
    }

    #[test]
    fn the_newest_lands_after_the_stack_and_the_older_one() {
        let o = Pos2::new(10.0, 20.0);
        let thumb = TRAY_HEIGHT * 1.5;
        assert_eq!(tray_slot(o, 1).min, o);
        assert_eq!(tray_slot(o, 2).min.x, o.x + thumb + TRAY_GAP);
        assert_eq!(tray_slot(o, 5).min.x, o.x + TRAY_HEIGHT + TRAY_GAP + thumb + TRAY_GAP);
    }

    #[test]
    fn the_cover_keeps_the_middle() {
        let wide = cover_uv(Vec2::new(200.0, 100.0), 1.0);
        assert!((wide.min.x - 0.25).abs() < 1e-6 && (wide.max.x - 0.75).abs() < 1e-6);
        assert_eq!((wide.min.y, wide.max.y), (0.0, 1.0));
        let tall = cover_uv(Vec2::new(100.0, 200.0), 1.0);
        assert!((tall.min.y - 0.25).abs() < 1e-6 && (tall.max.y - 0.75).abs() < 1e-6);
    }
}
