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

/// The viewfinder's tray: the session's photos as loose prints, shuffled
/// together: the room they lie in, a print's picture, the edge around it, how
/// many of the newest show.
pub const TRAY_TILE: Vec2 = Vec2::new(112.0, 92.0);
const PRINT: Vec2 = Vec2::new(78.0, 52.0);
pub const PRINT_EDGE: f32 = 2.0;
const PRINTS: usize = 4;

/// How a photo lies in the pile: its offset from the tile's middle and its
/// angle. Taken from its id, so a print stays where it fell when the next
/// one lands on it; the golden ratio keeps two photos in a row apart.
fn scatter(id: u64) -> (Vec2, f32) {
    let spread = |ratio: f64| ((id as f64 * ratio).fract() as f32 - 0.5) * 2.0;
    let offset = Vec2::new(spread(0.754_877_666) * 6.0, spread(0.569_840_291) * 5.0);
    (offset, spread(0.618_033_989) * 10f32.to_radians())
}

/// The picture of photo `id` in the tile whose top left is `origin`, and its
/// angle: where the card's photo flies to.
pub fn tray_slot(origin: Pos2, id: u64) -> (Rect, f32) {
    let (offset, angle) = scatter(id);
    (Rect::from_center_size(origin + TRAY_TILE / 2.0 + offset, PRINT), angle)
}

/// A photo as a loose print: its picture on `pic` turned by `angle`, an edge
/// of `edge` around it, its shadow on what lies under it.
pub fn print(painter: &egui::Painter, p: &Palette, pic: Rect, edge: f32, angle: f32, tex: Option<&egui::TextureHandle>) {
    use egui::epaint::RectShape;
    let radius = widgets::inner_radius(&painter.ctx().global_style().visuals);
    let paper = pic.expand(edge);
    painter.add(RectShape::filled(paper.translate(Vec2::new(1.0, 2.0)), radius, p.navy.gamma_multiply(0.45)).with_angle(angle));
    painter.add(RectShape::filled(paper, radius, p.ink).with_angle(angle));
    painter.add(RectShape::filled(pic, radius, p.field).with_angle(angle));
    if let Some(t) = tex {
        let uv = cover_uv(t.size_vec2(), pic.width() / pic.height().max(1.0));
        painter.add(
            RectShape::filled(pic, radius, widgets::PICTURE_TINT)
                .with_texture(t.id(), uv)
                .with_angle(angle),
        );
    }
}

/// What a click on the tray asks: the quick view on the newest photo, or
/// with Shift the folder it was saved to (the photos' folder when it is not
/// on disk).
fn tray_click(photos: &[Photo], shift: bool) -> Option<ReelAction> {
    let newest = photos.first()?;
    Some(match (shift, newest.on_disk()) {
        (false, _) => ReelAction::Show(newest.id),
        (true, Some(_)) => ReelAction::Reveal(newest.id),
        (true, None) => ReelAction::OpenFolder,
    })
}

/// How long the pile answers a photo landing on it (seconds).
pub const PULSE: f32 = 2.0;

/// The pile `t` seconds after a photo landed, a gentle, cheerful answer: its
/// scale, a small bounce settling within a second, and the strength of its
/// light, rising softly and fading out by `PULSE`.
fn pulse_at(t: f32) -> (f32, f32) {
    const RISE: f32 = 0.12;
    const BOUNCE: f32 = 0.55;
    let settle = (1.0 - t).max(0.0) * (-4.0 * t).exp();
    let scale = 1.0 + (t / BOUNCE * std::f32::consts::TAU).sin() * settle * 0.1;
    let fade = (1.0 - (t / PULSE).clamp(0.0, 1.0)).powi(3);
    (scale, (t / RISE).min(1.0) * fade)
}

/// The viewfinder's tray: the session's photos shuffled on the world with no
/// surface of their own, the newest on top, with their count. `since`: the
/// seconds since a photo landed, while the pile pulses.
pub fn tray(ui: &mut egui::Ui, p: &Palette, view: &ReelView, since: Option<f32>, out: &mut Vec<ReelAction>) {
    use egui::epaint::RectShape;
    let photos = &view.photos[usize::from(view.arriving).min(view.photos.len())..];
    let (rect, resp) = ui.allocate_exact_size(TRAY_TILE, Sense::click());
    let (swell, light) = since.map_or((1.0, 0.0), pulse_at);
    // a little larger under the pointer: no face left to light
    let scale = swell + ui.ctx().animate_bool(resp.id.with("hover"), resp.hovered()) * 0.03;
    let place = |id: u64| {
        let (pic, angle) = tray_slot(rect.min, id);
        let center = rect.center() + (pic.center() - rect.center()) * scale;
        (Rect::from_center_size(center, pic.size() * scale), angle)
    };
    let radius = widgets::inner_radius(ui.visuals());
    let top = photos.first().map(|photo| place(photo.id));
    // the light: a soft teal glow under the pile, a faint veil over the
    // photo on top
    if let Some((pic, angle)) = top
        && light > 0.0
    {
        ui.painter().add(
            RectShape::filled(pic.expand(8.0), radius, p.teal.gamma_multiply(light * 0.55))
                .with_blur_width(24.0)
                .with_angle(angle),
        );
    }
    for photo in photos.iter().take(PRINTS).rev() {
        let (pic, angle) = place(photo.id);
        print(ui.painter(), p, pic, PRINT_EDGE, angle, view.thumbs.get(&photo.id));
    }
    if let Some((pic, angle)) = top
        && light > 0.0
    {
        ui.painter()
            .add(RectShape::filled(pic.expand(PRINT_EDGE), radius, p.on_accent.gamma_multiply(light * 0.3)).with_angle(angle));
    }
    count_pill(ui.painter(), p, rect.right_bottom() - Vec2::splat(5.0), photos.len());
    let resp = resp.on_hover_text(aurora_i18n::tr!("photo-reel-tray-tip"));
    if resp.clicked() {
        resp.surrender_focus();
        out.extend(tray_click(photos, ui.input(|i| i.modifiers.shift)));
    }
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
    // over the corner, 3 points out: a row of thumbnails leaves room
    count_pill(painter, p, front.right_bottom() + Vec2::splat(3.0), count);
    true
}

/// The count of photos, from 2, on a violet pill whose bottom right is
/// `corner`.
fn count_pill(painter: &egui::Painter, p: &Palette, corner: Pos2, count: usize) {
    if count < 2 {
        return;
    }
    let radius = widgets::inner_radius(&painter.ctx().global_style().visuals);
    let text = painter.layout_no_wrap(count.to_string(), egui::FontId::proportional(10.5), p.on_accent);
    let size = Vec2::new((text.size().x + 8.0).max(16.0), 16.0);
    let pill = Rect::from_min_size(corner - size, size);
    painter.rect_filled(pill, radius, p.violet);
    painter.galley(pill.center() - text.size() / 2.0, text, p.on_accent);
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
/// e-mail always (to someone else). The profile picture comes once the
/// photo is uploaded to the inventory, whose texture it uses, in the
/// upload's place.
fn destinations(photo: &Photo) -> Vec<Destination> {
    let mut out = Vec::new();
    if photo.on_disk().is_none() {
        out.push(Destination::Disk);
    }
    if photo.item().is_none() {
        out.push(Destination::Inventory);
    } else if !photo.went(&Place::Profile) {
        out.push(Destination::Profile);
    }
    out.push(Destination::Email);
    out
}

fn send_button(d: Destination) -> (&'static str, &'static str) {
    match d {
        Destination::Disk => ("device-floppy-filled", aurora_i18n::tr!("photo-reel-save")),
        Destination::Inventory => ("briefcase-filled", aurora_i18n::tr!("photo-reel-upload")),
        Destination::Profile => ("user-filled", aurora_i18n::tr!("photo-reel-apply")),
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
        // colours kept, as a multiply would) and slightly blurred, with the
        // windows' blur; a click outside the photo and the bars closes
        let shade = Color32::from_black_alpha(170); // theme-guard: content — the world dimmed in its own colours, no surface
        let back = egui::Area::new(Id::unique("photo_reel_back"))
            .order(Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let (rect, resp) = ui.allocate_exact_size(screen.size(), Sense::click());
                ui.painter().add(aurora_render::glass_region(rect, 0.0, 1.0, 1.0));
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
        // the dialogs over it see all this through their glass
        widgets::ModalGround {
            shade,
            picture: view.thumbs.get(&id).map(|t| (t.id(), photo_rect)),
        }
        .set(ctx);
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
                // uploaded to the inventory: a green check at its top right,
                // on a dark disc so that any photo keeps it readable
                if photo.item().is_some() {
                    let mark = Rect::from_center_size(rect.right_top() + Vec2::new(-9.0, 9.0), Vec2::splat(14.0));
                    ui.painter().circle_filled(mark.center(), 7.5, p.navy);
                    vf_icon(ui.painter(), "circle-check-filled", mark, p.success);
                }
                let resp = resp.on_hover_text(places_text(photo));
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
        // not uploaded: no texture for the profile picture yet
        let fresh = [Destination::Disk, Destination::Inventory, Destination::Email];
        assert_eq!(destinations(&photo(vec![])), fresh);
        // uploaded: the profile picture in the upload's place
        let uploaded = photo(vec![Place::Disk("a.png".into()), Place::Inventory(Uuid::nil())]);
        assert_eq!(destinations(&uploaded), [Destination::Profile, Destination::Email]);
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
    fn the_prints_stay_in_their_tile_and_lie_differently() {
        let o = Pos2::new(10.0, 20.0);
        let tile = Rect::from_min_size(o, TRAY_TILE);
        for id in 0..200 {
            let (pic, angle) = tray_slot(o, id);
            // 3:2 like the card's picture
            assert!((pic.aspect_ratio() - 1.5).abs() < 1e-4);
            // every corner of the print, turned, inside the tile
            let paper = pic.expand(PRINT_EDGE);
            let turn = egui::emath::Rot2::from_angle(angle);
            for corner in [paper.left_top(), paper.right_top(), paper.left_bottom(), paper.right_bottom()] {
                assert!(tile.contains(pic.center() + turn * (corner - pic.center())), "photo {id}");
            }
            // the next photo does not lie along this one, and the same id
            // falls the same way
            assert!((angle - tray_slot(o, id + 1).1).abs() > 4f32.to_radians(), "photo {id}");
            assert_eq!(tray_slot(o, id), (pic, angle));
        }
    }

    #[test]
    fn a_click_views_the_newest_and_shift_opens_its_folder() {
        let saved = photo(vec![Place::Disk("a.png".into())]);
        assert_eq!(tray_click(std::slice::from_ref(&saved), false), Some(ReelAction::Show(1)));
        assert_eq!(tray_click(std::slice::from_ref(&saved), true), Some(ReelAction::Reveal(1)));
        // not on disk: the photos' folder
        let sent = photo(vec![Place::Email]);
        assert_eq!(tray_click(std::slice::from_ref(&sent), true), Some(ReelAction::OpenFolder));
        assert_eq!(tray_click(&[], true), None);
    }

    /// The tile as it floats in the viewfinder, under real pointer events:
    /// what it asked, and whether its area is being dragged.
    fn tray_frame(ctx: &egui::Context, mut events: Vec<egui::Event>, shift: bool, photos: &[Photo]) -> (Vec<ReelAction>, bool) {
        events.insert(
            0,
            egui::Event::ModifiersChanged(egui::Modifiers {
                shift,
                ..Default::default()
            }),
        );
        let p = crate::theme::Theme::default().palette();
        let thumbs = HashMap::new();
        let time = |_: i64| String::new();
        let view = ReelView {
            photos,
            thumbs: &thumbs,
            full: &thumbs,
            can_send: &[],
            time: &time,
            arriving: false,
        };
        let mut out = Vec::new();
        let mut dragged = false;
        let input = egui::RawInput {
            events,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(600.0))),
            ..Default::default()
        };
        let _ = crate::ui::login::test_output(ctx.run_ui(input, |ui| {
            let area = egui::Area::new(Id::unique("tray"))
                .movable(true)
                .current_pos(Pos2::new(100.0, 100.0))
                .show(ui.ctx(), |ui| tray(ui, &p, &view, None, &mut out));
            dragged = area.response.dragged();
        }));
        (out, dragged)
    }

    fn press(pos: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        }
    }

    #[test]
    fn the_tile_clicks_with_or_without_shift_and_drags() {
        let saved = [photo(vec![Place::Disk("a.png".into())])];
        let ctx = egui::Context::default();
        let at = Pos2::new(150.0, 140.0);
        // two frames for the tile to be laid out and under the pointer
        let _ = tray_frame(&ctx, vec![], false, &saved);
        let _ = tray_frame(&ctx, vec![egui::Event::PointerMoved(at)], false, &saved);
        for (shift, wanted) in [(false, ReelAction::Show(1)), (true, ReelAction::Reveal(1))] {
            let _ = tray_frame(&ctx, vec![press(at, true)], shift, &saved);
            let (out, _) = tray_frame(&ctx, vec![press(at, false)], shift, &saved);
            assert_eq!(out, [wanted]);
        }
        // pressed on the tile and moved: the area follows, no click
        let _ = tray_frame(&ctx, vec![press(at, true)], false, &saved);
        let to = at + Vec2::new(40.0, 0.0);
        let (out, dragged) = tray_frame(&ctx, vec![egui::Event::PointerMoved(to)], false, &saved);
        assert!(dragged && out.is_empty());
        let (out, _) = tray_frame(&ctx, vec![press(to, false)], false, &saved);
        assert!(out.is_empty());
    }

    #[test]
    fn the_pulse_is_gentle_and_over_in_time() {
        // no jump in size or light, the light up within a fifth of a second
        assert_eq!(pulse_at(0.0), (1.0, 0.0));
        assert!(pulse_at(0.2).1 > 0.6);
        // a small bounce, settled long before the light has gone
        for i in 0..200 {
            let t = i as f32 / 100.0;
            assert!((pulse_at(t).0 - 1.0).abs() < 0.06, "at {t}");
        }
        assert!(pulse_at(0.14).0 > 1.03);
        assert_eq!(pulse_at(1.0).0, 1.0);
        assert!(pulse_at(1.0).1 > 0.0 && pulse_at(1.0).1 < pulse_at(0.5).1);
        assert_eq!(pulse_at(PULSE), (1.0, 0.0));
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
