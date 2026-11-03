//! The photo reel on screen (`snapshot::reel`): the stack of the session's
//! photos beside the viewfinder's shutter and in the photo window, and the
//! quick view that opens from it, to scroll them, send one again or delete
//! it. Aurora's own: the Legacy Viewer only shows the last photo.

use crate::snapshot::reel::{self, Photo, Place};
use crate::theme::Palette;
use crate::ui::snapshot::Destination;
use crate::ui::viewfinder::{DOCK_SIDE, dock_face, dock_frame, icon as vf_icon};
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
    /// The photos' folder in the file manager.
    OpenFolder,
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

/// The viewfinder's tray, in the dock's tiles: the stack's and the folder's
/// squares (the dock's buttons' side), the dock's gap between them, the
/// room between a photo tile's edge and its picture.
const TRAY_STACK: f32 = DOCK_SIDE;
const TRAY_GAP: f32 = 10.0;
const TRAY_PAD: f32 = 5.0;

/// A photo's tile in the tray: as tall as the stack and the folder's
/// button, its picture 3:2 like the card's.
fn tray_thumb() -> Vec2 {
    let h = TRAY_STACK * 2.0 + TRAY_GAP;
    let picture = h - 2.0 * TRAY_PAD;
    Vec2::new(picture * 1.5 + 2.0 * TRAY_PAD, h)
}

/// The viewfinder's tray: the stack of the older photos with the folder's
/// button under it, then the two newest, the newest on the right; a click
/// opens the quick view on that photo. Each on a tile of the dock.
pub fn tray(ui: &mut egui::Ui, p: &Palette, view: &ReelView, out: &mut Vec<ReelAction>) {
    let photos = &view.photos[usize::from(view.arriving).min(view.photos.len())..];
    let radius = widgets::inner_radius(ui.visuals());
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing = Vec2::splat(TRAY_GAP);
        ui.vertical(|ui| {
            if photos.len() > 2 {
                let older = ReelView {
                    photos: &photos[2..],
                    arriving: false,
                    ..*view
                };
                let (rect, resp) = ui.allocate_exact_size(Vec2::splat(TRAY_STACK), Sense::click());
                dock_face(ui.painter(), p, rect, resp.hovered(), false, 1.0);
                // the front photo down and to the left, the two behind
                // peeking up and to the right inside the tile
                let front = Rect::from_min_size(rect.left_bottom() + Vec2::new(6.0, -36.0), Vec2::splat(30.0));
                paint_stack(ui.painter(), p, &older, front, photos.len());
                let resp = resp.on_hover_text(aurora_i18n::tr!("photo-reel-tip"));
                if resp.clicked() {
                    resp.surrender_focus();
                    out.push(ReelAction::Show(photos[2].id));
                }
            } else {
                // (the button stays under the stack's place)
                ui.add_space(TRAY_STACK);
            }
            let (rect, resp) = ui.allocate_exact_size(Vec2::splat(TRAY_STACK), Sense::click());
            dock_face(ui.painter(), p, rect, resp.hovered(), false, 1.0);
            vf_icon(
                ui.painter(),
                "folder-open-filled",
                Rect::from_center_size(rect.center(), Vec2::splat(20.0)),
                p.ink,
            );
            let resp = resp.on_hover_text(aurora_i18n::tr!("photo-open-folder"));
            if resp.clicked() {
                resp.surrender_focus();
                out.push(ReelAction::OpenFolder);
            }
        });
        for photo in photos.iter().take(2).rev() {
            let (rect, resp) = ui.allocate_exact_size(tray_thumb(), Sense::click());
            dock_face(ui.painter(), p, rect, resp.hovered(), false, 1.0);
            picture(ui.painter(), p, rect.shrink(TRAY_PAD), radius, view.thumbs.get(&photo.id));
            let resp = resp.on_hover_text(places_text(photo));
            if resp.clicked() {
                resp.surrender_focus();
                out.push(ReelAction::Show(photo.id));
            }
        }
    });
}

/// Where the tray puts its newest photo's picture once the reel holds
/// `len`, its tiles starting at `origin` (their top left): the card's
/// photo flies there.
pub fn tray_slot(origin: Pos2, len: usize) -> Rect {
    let mut x = origin.x + TRAY_STACK + TRAY_GAP;
    if len > 1 {
        x += tray_thumb().x + TRAY_GAP;
    }
    Rect::from_min_size(Pos2::new(x, origin.y), tray_thumb()).shrink(TRAY_PAD)
}

/// The tray's top left on `screen`: where it was moved (`at`, fractions of
/// the screen) or `default`, kept whole on the screen (a smaller window
/// does not lose it).
pub fn tray_place(at: Option<[f32; 2]>, screen: Rect, size: Vec2, default: Pos2) -> Pos2 {
    let pos = at.map_or(default, |[x, y]| screen.min + Vec2::new(x * screen.width(), y * screen.height()));
    let max = (screen.max - size).max(screen.min);
    pos.clamp(screen.min, max)
}

/// `pos` in fractions of `screen`, to keep it.
pub fn tray_fraction(pos: Pos2, screen: Rect) -> [f32; 2] {
    let rel = pos - screen.min;
    [rel.x / screen.width().max(1.0), rel.y / screen.height().max(1.0)]
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
    if !paint_stack(ui.painter(), p, view, rect, count) {
        return resp;
    }
    if resp.hovered() {
        let radius = ui.visuals().window_corner_radius;
        ui.painter()
            .rect_stroke(rect, radius, Stroke::new(1.5, p.teal.gamma_multiply(0.8)), StrokeKind::Inside);
    }
    resp.on_hover_text(aurora_i18n::tr!("photo-reel-tip"))
}

/// A button of the quick view on a tile of the dock, as tall as its
/// buttons: an icon and its label, red for a deletion, faded when it
/// cannot be used.
fn tile_button(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str, danger: bool, enabled: bool) -> egui::Response {
    const ICON: f32 = 18.0;
    const GAP: f32 = 8.0;
    const PAD: f32 = 14.0;
    let ink = if danger { p.danger } else { p.ink };
    let alpha = if enabled { 1.0 } else { 0.45 };
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), egui::FontId::proportional(13.0), ink.gamma_multiply(alpha));
    let size = Vec2::new(PAD + ICON + GAP + galley.size().x + PAD, DOCK_SIDE);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    dock_face(ui.painter(), p, rect, enabled && resp.hovered(), false, 1.0);
    let left = rect.left() + PAD;
    vf_icon(
        ui.painter(),
        icon,
        Rect::from_center_size(Pos2::new(left + ICON / 2.0, rect.center().y), Vec2::splat(ICON)),
        ink.gamma_multiply(alpha),
    );
    let text_at = Pos2::new(left + ICON + GAP, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(text_at, galley, ink);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    }
}

/// The stack's photos, the front one on `front`, with the count; false
/// when there is none to show.
fn paint_stack(painter: &egui::Painter, p: &Palette, view: &ReelView, front: Rect, count: usize) -> bool {
    let shown: Vec<&Photo> = view.photos.iter().skip(usize::from(view.arriving)).take(3).collect();
    if shown.is_empty() {
        return false;
    }
    let radius = widgets::inner_radius(&painter.ctx().global_style().visuals);
    // the cards behind, up and to the right, darker
    let step = (front.width() * 0.1).round().max(2.0);
    for (i, photo) in shown.iter().enumerate().skip(1).rev() {
        let r = front.translate(Vec2::new(step, -step) * i as f32).shrink(step * i as f32 * 0.25);
        picture(painter, p, r, radius, view.thumbs.get(&photo.id));
        painter.rect_filled(r, radius, p.navy.gamma_multiply(0.35 + 0.15 * i as f32));
        painter.rect_stroke(r, radius, Stroke::new(1.0, p.ink.gamma_multiply(0.25)), StrokeKind::Inside);
    }
    picture(painter, p, front, radius, view.thumbs.get(&shown[0].id));
    painter.rect_stroke(front, radius, Stroke::new(1.0, p.ink.gamma_multiply(0.5)), StrokeKind::Inside);
    // the count, on a violet pill at the bottom right
    if count > 1 {
        let font = egui::FontId::proportional(10.5);
        let text = painter.layout_no_wrap(count.to_string(), font, p.on_accent);
        // over the corner, 3 points out: a row of thumbnails leaves room
        let size = Vec2::new((text.size().x + 8.0).max(16.0), 16.0);
        let pill = Rect::from_min_size(front.right_bottom() + Vec2::splat(3.0) - size, size);
        painter.rect_filled(pill, radius, p.violet);
        painter.galley(pill.center() - text.size() / 2.0, text, p.on_accent);
    }
    true
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
    /// A dialog of the app over it (the price, the e-mail): the keys and
    /// the clicks outside are the dialog's. Set before each `show`.
    pub blocked: bool,
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
        if self.confirm.is_none() && !self.blocked {
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

        // the world darkened behind in its own light (the sky and its
        // colours kept, as a multiply would); a click outside the photo
        // and the bars closes
        let back = egui::Area::new(Id::unique("photo_reel_back"))
            .order(Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let (rect, resp) = ui.allocate_exact_size(screen.size(), Sense::click());
                let shade = Color32::from_black_alpha(170); // theme-guard: content — the world dimmed in its own colours, no surface
                ui.painter().rect_filled(rect, 0.0, shade);
                resp
            });
        // on top of photo mode, not over a dialog of its own (the deletion,
        // the price, the e-mail): egui keeps the order of layers raised in
        // the same frame, an older dialog would stay under it
        let layer = back.response.layer_id;
        if self.confirm.is_none() && !self.blocked {
            ctx.move_to_top(layer);
        }

        // the photo in its frame, as large as the room between the bars
        // allows
        const MAT: f32 = 10.0;
        let room = Rect::from_min_max(
            Pos2::new(screen.left() + 72.0, screen.top() + 64.0),
            Pos2::new(screen.right() - 72.0, screen.bottom() - 196.0),
        )
        .shrink(MAT);
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
            // the frame: a tile of the dock, a mat around the photo
            dock_face(&painter, p, photo_rect.expand(MAT), false, false, alpha);
            let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            // a cut-out photo over the field, its transparency in sight
            if photo.cutout {
                painter.rect_filled(photo_rect, 0.0, p.field);
            }
            painter.image(t.id(), photo_rect, uv, widgets::PICTURE_TINT.gamma_multiply(alpha));
        }
        let pointer = ctx.input(|i| i.pointer.interact_pos());
        let on_photo = pointer.is_some_and(|pos| photo_rect.expand(MAT).contains(pos));

        let mut over_bars = false;
        // the top: which photo, when, where it went; and close
        let top = egui::Area::new(Id::unique("photo_reel_top"))
            .order(Order::Foreground)
            .pivot(Align2::CENTER_TOP)
            .fixed_pos(Pos2::new(screen.center().x, screen.top() + 16.0))
            .show(ctx, |ui| {
                dock_frame(p, ui.visuals(), 0)
                    .inner_margin(egui::Margin::symmetric(14, 8))
                    .show(ui, |ui| {
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
        // the bars and the buttons right above the backdrop, always (a
        // click on the backdrop raises it alone)
        ctx.set_sublayer(layer, top.response.layer_id);
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
                    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(DOCK_SIDE), Sense::click());
                    dock_face(ui.painter(), p, rect, resp.hovered(), false, 1.0);
                    vf_icon(ui.painter(), icon, Rect::from_center_size(rect.center(), Vec2::splat(20.0)), p.ink);
                    resp.on_hover_text(tip).clicked()
                });
            ctx.set_sublayer(layer, arrow.response.layer_id);
            over_bars |= arrow.response.hovered();
            if arrow.inner {
                self.go(view, view.photos[to].id, now, out);
            }
        }

        // the bottom: what the photo can do, each button on its own under
        // the frame, then the strip of the reel
        let strip = egui::Area::new(Id::unique("photo_reel_strip"))
            .order(Order::Foreground)
            .pivot(Align2::CENTER_BOTTOM)
            .fixed_pos(Pos2::new(screen.center().x, screen.bottom() - 16.0))
            .show(ctx, |ui| {
                dock_frame(p, ui.visuals(), 8).show(ui, |ui| self.filmstrip(ui, p, view, cursor, now, out));
            });
        let buttons = egui::Area::new(Id::unique("photo_reel_actions"))
            .order(Order::Foreground)
            .pivot(Align2::CENTER_BOTTOM)
            .fixed_pos(Pos2::new(screen.center().x, strip.response.rect.top() - 16.0))
            .show(ctx, |ui| self.actions(ui, p, view, photo, out));
        for area in [&strip, &buttons] {
            ctx.set_sublayer(layer, area.response.layer_id);
            over_bars |= area.response.hovered();
        }

        if back.inner.clicked() && !on_photo && !over_bars && self.confirm.is_none() && !self.blocked {
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

    /// Send it again, show where it went, delete it: each button a tile of
    /// the dock, apart from the frame and from one another.
    fn actions(&mut self, ui: &mut egui::Ui, p: &Palette, view: &ReelView, photo: &Photo, out: &mut Vec<ReelAction>) {
        let ready = photo.file.is_some();
        let button = |ui: &mut egui::Ui, icon: &str, label: &str, kind: ActionKind, enabled: bool| {
            tile_button(ui, p, icon, label, matches!(kind, ActionKind::FlatDanger), enabled)
        };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            for d in destinations(photo) {
                let (icon, label) = send_button(d);
                let enabled = ready && view.can_send.contains(&d);
                let resp = button(ui, icon, label, ActionKind::Flat, enabled);
                let resp = if ready {
                    resp
                } else {
                    resp.on_hover_text(aurora_i18n::tr!("photo-reel-preparing"))
                };
                if resp.clicked() && enabled {
                    out.push(ReelAction::Send(photo.id, d));
                }
            }
            if photo.on_disk().is_some()
                && button(
                    ui,
                    "folder-open-filled",
                    aurora_i18n::tr!("photo-reel-in-folder"),
                    ActionKind::Flat,
                    true,
                )
                .clicked()
            {
                out.push(ReelAction::Reveal(photo.id));
            }
            if photo.item().is_some()
                && button(
                    ui,
                    "briefcase-filled",
                    aurora_i18n::tr!("photo-reel-in-inventory"),
                    ActionKind::Flat,
                    true,
                )
                .clicked()
            {
                out.push(ReelAction::ShowItem(photo.id));
            }
            if photo.went(&Place::Profile)
                && button(ui, "user-filled", aurora_i18n::tr!("photo-reel-on-profile"), ActionKind::Flat, true).clicked()
            {
                out.push(ReelAction::ShowProfile);
            }
            if button(ui, "trash-filled", aurora_i18n::tr!("common-delete"), ActionKind::FlatDanger, true).clicked() {
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
    fn the_tray_stays_where_it_was_moved_and_on_screen() {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 500.0));
        let size = Vec2::new(200.0, 100.0);
        let default = Pos2::new(20.0, 380.0);
        assert_eq!(tray_place(None, screen, size, default), default);
        let at = tray_fraction(Pos2::new(500.0, 100.0), screen);
        assert_eq!(tray_place(Some(at), screen, size, default), Pos2::new(500.0, 100.0));
        // a smaller window: pushed back inside, never off it
        let small = Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 300.0));
        assert_eq!(tray_place(Some([0.9, 0.9]), small, size, default), Pos2::new(400.0, 200.0));
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
        let column = TRAY_STACK + TRAY_GAP;
        let thumb = tray_thumb();
        // the stack's column first, kept for the folder's button; the
        // picture inside its tile
        assert_eq!(tray_slot(o, 1).min, Pos2::new(o.x + column + TRAY_PAD, o.y + TRAY_PAD));
        assert_eq!(tray_slot(o, 2).min.x, o.x + column + thumb.x + TRAY_GAP + TRAY_PAD);
        assert_eq!(tray_slot(o, 5).min.x, tray_slot(o, 2).min.x);
        assert_eq!(tray_slot(o, 5).size(), thumb - Vec2::splat(2.0 * TRAY_PAD));
        // 3:2 like the card's picture
        assert!((tray_slot(o, 5).aspect_ratio() - 1.5).abs() < 1e-4);
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
