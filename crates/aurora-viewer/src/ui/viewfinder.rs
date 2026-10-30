//! The photo viewfinder (Preferences › Aurora › Photo mode › Viewfinder):
//! the world full screen under a camera's overlay, like an electronic
//! viewfinder. Its three display levels (DISP), the AF point that hunts,
//! turns green and beeps, the distance scale with the zone kept sharp, the
//! exposure bar, the histogram, the level, the dock for the mouse, the
//! quick menu, the help, a drawer with every setting of the photo panel,
//! the isolation panel (`isolate`) and the camera presets (`presets`). The logic is `snapshot::viewfinder`, the frames and the keys
//! `app::viewfinder`; TASKS.md (photo window, part 6) tells what it does.

use super::snapshot::{self as panel, Action, Destination, SnapshotUi, View};
use super::widgets;
use crate::dof::PhotoFocus;
use crate::snapshot::viewfinder::{self as vf, AfArea, Autofocus, Display, Peaking, Phase, Sounds};
use crate::snapshot::{Aspect, FileFormat, Guide, Resolution, Settings};
use crate::theme::Palette;
use egui::{Align2, Color32, FontId, Id, Order, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};

mod controls;
mod isolate;
mod light;
mod looks;
mod presets;
pub use controls::{CamMode, Pad, roll_text};
pub use isolate::{Isolate, IsolateRow};
pub use presets::{Preset, PresetForm, anchor_icon};
#[cfg(test)]
mod message_tests;

/// The value the mouse wheel drives (the underlined one of the bottom bar).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Dial {
    #[default]
    Focal,
    Aperture,
    Exposure,
    Distance,
}

/// The viewfinder's state for the session (its settings are
/// `snapshot::viewfinder::Prefs`).
#[derive(Default)]
pub struct Viewfinder {
    pub menu: bool,
    pub help: bool,
    /// Every setting of the photo panel, in a drawer on the right.
    pub drawer: bool,
    /// The isolation panel (what the photo shows), on the right.
    pub isolate: bool,
    /// The filter panel (`looks`).
    pub looks: bool,
    /// The light panel (`light`): the photo's sky, water and sun.
    pub light: bool,
    /// The camera presets panel (`presets`) and its saving form.
    pub presets: bool,
    pub preset_form: PresetForm,
    pub dial: Dial,
    /// The shutter half-pressed (Enter or the button held).
    pub pressed: bool,
    /// Released while the lens hunts: the photo goes as it locks.
    pub pending_shot: bool,
    /// The shutter button was pressed this frame (the app focuses next).
    pub half_press: bool,
    /// And released in the same frame: the photo follows the focus.
    pub release: bool,
    pub af: Autofocus,
    /// The AF point, in fractions of the screen (None: its centre).
    pub point: Option<Pos2>,
    /// What the focus is on: an avatar (AF area Avatar), or the group of
    /// avatars and points Shift + click gathers, the first the one clicked
    /// first.
    pub subjects: Vec<vf::Subject>,
    /// The point a click alone focused on, the first of a group Shift +
    /// click starts from there.
    pub last_point: Option<glam::Vec3>,
    /// The zone the group keeps sharp (None for one subject).
    pub zone: Option<vf::Zone>,
    /// The depth under the AF point asked of the renderer.
    pub pick: Option<u64>,
    /// That depth was asked by Shift + click: the avatar or the point there
    /// joins the group, or leaves it.
    pub adding: bool,
    /// That depth was asked by the shutter with the lens locked: the same
    /// subject under the point keeps the lock.
    pub recheck: bool,
    /// The first hint was put away (a focus or a photo).
    pub hinted: bool,
    /// When the shutter's blackout started.
    pub flash_at: Option<f64>,
    /// The last photo's card shows until then.
    pub review_until: f64,
    /// The histogram's curve as shown, easing towards the last counts.
    histogram: Vec<f32>,
    /// A line at the top: (text, is an error, until).
    pub message: Option<(String, bool, f64)>,
    /// The shutter button is held by the mouse.
    mouse_down: bool,
    /// AE-L: the automatic exposure held.
    pub ae_lock: bool,
    /// The self-timer runs: when the photo goes (egui's clock).
    pub countdown: Option<f64>,
    /// The camera pad this frame.
    pub pad: Pad,
    /// The lens ring dragged: its value and the pointer's x at the start.
    ring_drag: Option<(f32, f32)>,
    /// The ring's mark last crossed (a click at each).
    ring_mark: Option<i32>,
    /// The pointer was last near the controls (OnScreen::Hover).
    hover_seen: f64,
    /// The display level drawn last frame (the interface just hidden says
    /// how to show it again).
    shown_display: Option<Display>,
}

/// What the app gives the viewfinder each frame.
pub struct Frame<'a> {
    /// egui's clock (seconds).
    pub now: f64,
    /// The photo's size in pixels, and the window's.
    pub photo: (u32, u32),
    pub window: (u32, u32),
    pub fps: f32,
    pub focal_mm: f32,
    /// Stops from Preferences' exposure.
    pub ev: f32,
    /// Preferences' exposure, which the exposure ring counts from.
    pub base_exposure: f32,
    /// The camera's tilt (degrees, positive clockwise).
    pub roll: f32,
    /// The zone the lens keeps sharp (m).
    pub sharp: (f32, f32),
    /// The AF point (points).
    pub point: Pos2,
    /// The head of the avatar under the AF point, or of the subject (its
    /// box in points), whether the focus is on it, and the avatar's name.
    pub face: Option<(Rect, bool, String)>,
    /// The rest of a group focus: its heads (boxes and names) and its
    /// points (points), and the zone kept sharp from the nearest to the
    /// farthest (m).
    pub group: Vec<(Rect, String)>,
    pub points: Vec<Pos2>,
    pub zone: Option<(f32, f32)>,
    /// The camera's pitch (degrees, positive up).
    pub pitch: f32,
    pub histogram: Option<&'a [u32; vf::HISTOGRAM_BINS]>,
    /// The last photo: its thumbnail and what the card says of it.
    pub review: Option<(&'a egui::TextureHandle, String, Review)>,
    pub first_person: bool,
    /// Preferences allow the first person view (EnableMouselook).
    pub first_person_allowed: bool,
    pub free_camera: bool,
    /// The keys: menu, AF mode, first person.
    pub keys: Keys<'a>,
    /// What the photo shows (`snapshot::isolate`), the names of what is
    /// chosen, and of what is under the pointer while choosing (whether it
    /// is chosen).
    pub isolation: &'a crate::snapshot::isolate::Isolation,
    pub isolate_rows: &'a [IsolateRow],
    pub isolate_hover: Option<(String, bool)>,
    /// The camera presets, and the name of the object an object anchor
    /// would hold.
    pub presets: &'a [crate::camera::CameraShot],
    pub preset_object: Option<String>,
    /// The poser's window shows over the viewfinder.
    pub posing: bool,
}

/// The keys the help and the hints name.
pub struct Keys<'a> {
    pub menu: &'a str,
    pub af_mode: &'a str,
    pub first_person: Option<&'a str>,
}

/// What the last photo's card offers.
#[derive(Debug, Clone, PartialEq)]
pub enum Review {
    /// On disk: open it, show its folder.
    Disk(std::path::PathBuf),
    /// In the Photo Album.
    Inventory,
    /// On the profile feed.
    Profile,
    /// Sent by e-mail.
    Email,
}

/// What the viewfinder asks of the app.
#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    /// One of the photo panel's actions (the drawer, the card).
    Panel(Action),
    /// The shutter half-pressed, released (the photo), or let go off the
    /// button (no photo).
    HalfPress,
    Release,
    Cancel,
    /// The view from the avatar's eyes, or back behind it.
    FirstPerson(bool),
    /// The focus mode and area chosen on screen.
    SetAf(PhotoFocus),
    SetArea(AfArea),
    /// What the camera pad drives.
    Camera(CamMode),
    /// The camera's tilt (degrees).
    Roll(f32),
    /// A lens ring passed a mark: its click.
    Detent,
    /// The isolation panel, or choosing in the scene.
    Isolate(Isolate),
    /// The camera presets panel, or choosing its object in the scene.
    Preset(Preset),
    /// The poser's window opened or closed (#1389).
    Poser,
    /// Out of the photo mode.
    Exit,
}

const GAP: f32 = 24.0;
/// The AF point's brackets.
const AF_BOX: f32 = 88.0;

/// The fonts of the overlay.
fn regular(size: f32) -> FontId {
    FontId::proportional(size)
}

fn bold(ctx: &egui::Context, size: f32) -> FontId {
    FontId::new(size, crate::ui::fonts::semibold(ctx))
}

/// A text over the picture, with the shadow that keeps it readable on a
/// bright sky.
fn shadow_text(painter: &egui::Painter, p: &Palette, pos: Pos2, align: Align2, text: &str, font: FontId, color: Color32) -> Rect {
    painter.text(
        pos + Vec2::new(0.0, 1.0),
        align,
        text,
        font.clone(),
        p.navy.gamma_multiply(0.85 * color.a() as f32 / 255.0),
    );
    painter.text(pos, align, text, font, color)
}

/// A band fading from `top` to `bottom`.
fn gradient(painter: &egui::Painter, rect: Rect, top: Color32, bottom: Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 3, 2);
    painter.add(egui::Shape::mesh(mesh));
}

fn icon(painter: &egui::Painter, name: &str, rect: Rect, tint: Color32) {
    if let Some(t) = crate::ui::icons::global(name) {
        painter.image(t.id(), rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), tint);
    }
}

/// A text of the overlay that can be clicked; its colour brightens under
/// the pointer.
fn text_button(ui: &mut egui::Ui, p: &Palette, text: &str, font: FontId, color: Color32, tip: &str) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font.clone(), color);
    let (rect, resp) = ui.allocate_exact_size(galley.size(), Sense::click());
    let color = if resp.hovered() { p.ink } else { color };
    shadow_text(ui.painter(), p, rect.left_top(), Align2::LEFT_TOP, text, font, color);
    keep_keys(&resp);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip)
}

/// A value of the bottom bar: a text clicked, or dragged at once to turn it.
fn value_button(ui: &mut egui::Ui, p: &Palette, text: &str, font: FontId, color: Color32, tip: &str) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font.clone(), color);
    let (rect, resp) = ui.allocate_exact_size(galley.size(), Sense::click_and_drag());
    let color = if resp.hovered() || resp.dragged() { p.ink } else { color };
    shadow_text(ui.painter(), p, rect.left_top(), Align2::LEFT_TOP, text, font, color);
    resp.on_hover_cursor(egui::CursorIcon::ResizeHorizontal).on_hover_text(tip)
}

/// An icon alone of the top bar, clickable, centred on the line.
fn icon_switch(ui: &mut egui::Ui, p: &Palette, name: &str, color: Color32, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::click());
    let color = if resp.hovered() { p.ink } else { color };
    let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(18.0));
    icon(
        ui.painter(),
        name,
        icon_rect.translate(Vec2::new(0.0, 1.0)),
        p.navy.gamma_multiply(0.7),
    );
    icon(ui.painter(), name, icon_rect, color);
    keep_keys(&resp);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip)
}

/// A short label in a frame (a resolution's badge), clickable.
fn badge_button(ui: &mut egui::Ui, p: &Palette, text: &str, color: Color32, tip: &str) -> egui::Response {
    let font = FontId::new(11.5, crate::ui::fonts::semibold(ui.ctx()));
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font.clone(), color);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(galley.size().x + 10.0, 18.0), Sense::click());
    let color = if resp.hovered() { p.ink } else { color };
    let radius = widgets::inner_radius(ui.visuals());
    ui.painter().rect_stroke(
        rect.translate(Vec2::new(0.0, 1.0)),
        radius,
        Stroke::new(1.5, p.navy.gamma_multiply(0.6)),
        StrokeKind::Inside,
    );
    ui.painter().rect_stroke(rect, radius, Stroke::new(1.5, color), StrokeKind::Inside);
    shadow_text(ui.painter(), p, rect.center(), Align2::CENTER_CENTER, text, font, color);
    keep_keys(&resp);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip)
}

/// An icon and a text of the top bar, clickable.
fn icon_text_button(ui: &mut egui::Ui, p: &Palette, name: &str, text: &str, color: Color32, tip: &str) -> egui::Response {
    let font = regular(13.0);
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font.clone(), color);
    let size = Vec2::new(18.0 + 6.0 + galley.size().x, galley.size().y.max(18.0));
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let color = if resp.hovered() { p.ink } else { color };
    let icon_rect = Rect::from_center_size(Pos2::new(rect.left() + 9.0, rect.center().y), Vec2::splat(18.0));
    icon(
        ui.painter(),
        name,
        icon_rect.translate(Vec2::new(0.0, 1.0)),
        p.navy.gamma_multiply(0.7),
    );
    icon(ui.painter(), name, icon_rect, color);
    shadow_text(
        ui.painter(),
        p,
        Pos2::new(rect.left() + 24.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        font,
        color,
    );
    keep_keys(&resp);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip)
}

/// The overlay's controls never keep the keyboard: after a click on one,
/// Enter, I, Tab… are still the viewfinder's.
fn keep_keys(resp: &egui::Response) {
    if resp.clicked() || resp.drag_stopped() {
        resp.surrender_focus();
    }
}

/// The AF mode's name.
pub fn af_label(focus: PhotoFocus) -> &'static str {
    match focus {
        PhotoFocus::Click => "AF-S",
        PhotoFocus::Pointer => "AF-C",
        PhotoFocus::Manual => "MF",
    }
}

fn guide_label(g: Guide) -> &'static str {
    match g {
        Guide::None => aurora_i18n::tr!("photo-vf-guide-none"),
        Guide::Thirds => aurora_i18n::tr!("photo-guide-thirds"),
        Guide::Golden => aurora_i18n::tr!("photo-guide-golden"),
        Guide::Diagonals => aurora_i18n::tr!("photo-guide-diagonals"),
        Guide::Center => aurora_i18n::tr!("photo-guide-center"),
    }
}

pub fn next_guide(g: Guide) -> Guide {
    match g {
        Guide::None => Guide::Thirds,
        Guide::Thirds => Guide::Golden,
        Guide::Golden => Guide::Diagonals,
        Guide::Diagonals => Guide::Center,
        Guide::Center => Guide::None,
    }
}

/// A "·" between two switches.
fn dot(ui: &mut egui::Ui, p: &Palette) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(4.0, 14.0), Sense::hover());
    shadow_text(ui.painter(), p, r.center(), Align2::CENTER_CENTER, "·", regular(13.0), p.ink);
}

fn next_destination(d: Destination) -> Destination {
    match d {
        Destination::Disk => Destination::Inventory,
        Destination::Inventory => Destination::Profile,
        // a postcard needs its e-mail, written in the panel
        Destination::Profile | Destination::Email => Destination::Disk,
    }
}

fn next_time(time: Option<u8>) -> Option<u8> {
    match time {
        None => Some(1),
        Some(n) if n < 4 => Some(n + 1),
        Some(_) => None,
    }
}

/// The next size of the destination: the disk's resolutions (the typed
/// one stays in the drawer), the inventory's textures, the profile's.
fn next_size(s: &mut Settings, destination: Destination) {
    match destination {
        Destination::Disk => {
            let all: Vec<Resolution> = Resolution::ALL.into_iter().filter(|r| *r != Resolution::Custom).collect();
            let i = all.iter().position(|r| *r == s.resolution).map_or(0, |i| i + 1);
            s.resolution = all[i % all.len()];
        }
        Destination::Inventory => {
            let all = crate::snapshot::inventory::TextureSize::ALL;
            let i = all.iter().position(|r| *r == s.inventory_size).unwrap_or(0);
            s.inventory_size = all[(i + 1) % all.len()];
        }
        Destination::Profile => {
            let all = crate::snapshot::profile::ProfileSize::ALL;
            let i = all.iter().position(|r| *r == s.profile_size).unwrap_or(0);
            s.profile_size = all[(i + 1) % all.len()];
        }
        Destination::Email => {
            let all = crate::snapshot::profile::ProfileSize::ALL;
            let i = all.iter().position(|r| *r == s.postcard_size).unwrap_or(0);
            s.postcard_size = all[(i + 1) % all.len()];
        }
    }
}

fn time_label(time: Option<u8>) -> &'static str {
    match time {
        None => aurora_i18n::tr!("photo-vf-sky-current"),
        Some(1) => aurora_i18n::tr!("ui-environment-sunrise"),
        Some(2) => aurora_i18n::tr!("common-midday"),
        Some(3) => aurora_i18n::tr!("ui-environment-lie-down"),
        Some(_) => aurora_i18n::tr!("common-midnight"),
    }
}

fn aspect_label(a: Aspect) -> &'static str {
    a.short().unwrap_or(aurora_i18n::tr!("photo-aspect-window"))
}

fn destination_label(d: Destination) -> &'static str {
    match d {
        Destination::Disk => aurora_i18n::tr!("photo-destination-disk"),
        Destination::Inventory => aurora_i18n::tr!("photo-destination-inventory"),
        Destination::Profile => aurora_i18n::tr!("photo-destination-profile"),
        Destination::Email => aurora_i18n::tr!("photo-destination-email"),
    }
}

fn peaking_label(k: Peaking) -> &'static str {
    match k {
        Peaking::White => aurora_i18n::tr!("photo-vf-peaking-white"),
        Peaking::Red => aurora_i18n::tr!("photo-vf-peaking-red"),
        Peaking::Yellow => aurora_i18n::tr!("photo-vf-peaking-yellow"),
        Peaking::Off => aurora_i18n::tr!("photo-vf-off"),
    }
}

/// The peaking colour of the palette (white, red, yellow).
pub fn peaking_color(p: &Palette, k: Peaking) -> Option<Color32> {
    match k {
        Peaking::White => Some(p.ink),
        Peaking::Red => Some(p.danger),
        Peaking::Yellow => Some(p.amber),
        Peaking::Off => None,
    }
}

fn sounds_label(s: Sounds) -> &'static str {
    match s {
        Sounds::BeepAndShutter => aurora_i18n::tr!("photo-vf-sounds-both"),
        Sounds::Shutter => aurora_i18n::tr!("photo-vf-sounds-shutter"),
        Sounds::None => aurora_i18n::tr!("photo-vf-sounds-none"),
    }
}

fn on_off(on: bool, yes: &'static str, no: &'static str) -> &'static str {
    if on { yes } else { no }
}

impl Viewfinder {
    /// The line at the top for a few seconds.
    pub fn say(&mut self, text: impl Into<String>, error: bool, now: f64) {
        self.message = Some((text.into(), error, now + 2.6));
    }

    /// The whole viewfinder for this frame; `fov` is the camera's vertical
    /// field of view (the focal length), `panel` the photo panel's state
    /// (its destination, light, free camera, freeze), `view` what its
    /// drawer shows.
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        s: &mut Settings,
        panel: &mut SnapshotUi,
        fov: &mut f32,
        default_fov: f32,
        f: &Frame,
        view: &View,
    ) -> Vec<Out> {
        let mut out = Vec::new();
        panel.looks.hover = None;
        let before = s.clone();
        let light_before = panel.light;
        let screen = ctx.content_rect();
        let display = s.viewfinder.display;
        let full = display == Display::Full;
        let shows = display != Display::Hidden;
        let back = ctx.layer_painter(egui::LayerId::new(Order::Background, Id::unique("vf_back")));
        let front = ctx.layer_painter(egui::LayerId::new(Order::Middle, Id::unique("vf_front")));

        // the photo's proportions: the rest darkened, a fine frame, the guide
        let ratio = f.photo.0 as f32 / f.photo.1.max(1) as f32;
        let frame = panel::fit(screen, ratio);
        for r in panel::outside(screen, frame) {
            back.rect_filled(r, 0.0, p.navy.gamma_multiply(0.66));
        }
        let border = if shows { 0.5 } else { 0.25 };
        back.rect_stroke(frame, 0.0, Stroke::new(1.0, p.ink.gamma_multiply(border)), StrokeKind::Inside);
        if shows && s.guide != Guide::None {
            panel::guide_lines(&back, frame, s.guide, p.ink.gamma_multiply(0.4));
        }
        if !s.looks.is_empty() {
            self.compare_line(ctx, p, frame, panel);
        }

        // the on-screen controls (lens ring, AF choices, camera pad)
        let ctl = shows && !self.menu && self.controls_shown(ctx, s, screen, f.now);
        let pad = ctl && s.viewfinder.pad;
        if full {
            self.top_bar(ctx, p, &back, screen, s, panel, f, &mut out);
        }
        self.af_point(ctx, p, &front, s, f);
        if shows && !self.hinted && !self.menu && !self.help {
            hint(ctx, p, screen, if ctl { 156.0 } else { 128.0 }, ctl && !pad);
        }
        if let Some((text, error, until)) = &self.message {
            if f.now < *until {
                message(ctx, p, screen, text, *error);
            } else {
                self.message = None;
            }
        }
        let manual = s.dof.focus == PhotoFocus::Manual;
        let active = matches!(self.af.phase, Phase::Hunt | Phase::Fail) || (self.af.recent(f.now) && shows);
        if ctl {
            self.ring_deck(ctx, p, screen, s, panel, fov, f, &mut out);
        } else if !self.menu && (manual || active || full) {
            self.scale(ctx, p, &front, screen, s, f);
        }
        if shows {
            self.bottom_bar(ctx, p, &back, screen, s, panel, fov, f, ctl, &mut out);
            self.dock(ctx, p, screen, s, panel, f, &mut out);
        } else if ghost_disp(ctx, p, screen) {
            s.viewfinder.display = display.next();
        }
        // just hidden: say how it comes back, a little longer than a
        // setting's message
        if !shows && self.shown_display != Some(display) {
            self.message = Some((aurora_i18n::tr!("photo-vf-hidden-how").to_owned(), false, f.now + 4.5));
        }
        self.shown_display = Some(display);
        if pad {
            self.camera_pad(ctx, p, screen, f, &mut out);
        } else {
            self.pad = Pad::default();
        }
        if f.now < self.review_until && !self.menu {
            review(ctx, p, screen, f, &mut out, if pad { 300.0 } else { 132.0 });
        }
        self.countdown(ctx, p, screen, f.now);
        if self.menu {
            self.quick_menu(ctx, p, screen, s, panel, f, &mut out);
        }
        if self.help {
            self.help_panel(ctx, p, screen, f);
        }
        if self.drawer {
            self.drawer(ctx, p, screen, s, panel, fov, default_fov, view, &mut out);
        }
        if self.looks {
            self.looks_panel(ctx, p, screen, s, panel, &mut out);
        }
        if self.light {
            self.light_panel(ctx, p, screen, panel, view, &mut out);
        }
        if self.isolate {
            self.isolate_picking(ctx, p, frame, f, &mut out);
            self.isolate_panel(ctx, p, screen, f, &mut out);
        }
        if self.presets {
            self.preset_picking(ctx, p, frame, f, &mut out);
            self.presets_panel(ctx, p, screen, f, &mut out);
        } else {
            self.preset_form.picking = false;
        }
        // the shutter's blackout: black at once, then gone in 0.18 s
        if let Some(t) = self.flash_at {
            let k = ((f.now - t) as f32 / 0.18).clamp(0.0, 1.0);
            let alpha = if f.now - t < 0.09 { 1.0 } else { 1.0 - k };
            if alpha > 0.0 {
                let top = ctx.layer_painter(egui::LayerId::new(Order::Foreground, Id::unique("vf_flash")));
                top.rect_filled(screen, 0.0, p.navy.gamma_multiply(alpha));
                ctx.request_repaint();
            } else {
                self.flash_at = None;
            }
        }

        if *s != before {
            let photo = s.aspect != before.aspect
                || s.resolution != before.resolution
                || s.inventory_size != before.inventory_size
                || s.profile_size != before.profile_size
                || s.dof != before.dof
                || s.huds != before.huds
                || s.looks != before.looks
                || s.look_settings != before.look_settings;
            out.push(Out::Panel(if photo { Action::Changed } else { Action::Saved }));
        }
        if panel.light != light_before && !out.contains(&Out::Panel(Action::Changed)) {
            out.push(Out::Panel(Action::Changed));
        }
        out
    }

    /// The top bar: the focus on the left, what the photo will be on the
    /// right; under it the light of the photo and the histogram, and the
    /// level in the middle.
    #[allow(clippy::too_many_arguments)]
    fn top_bar(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        back: &egui::Painter,
        screen: Rect,
        s: &mut Settings,
        panel: &mut SnapshotUi,
        f: &Frame,
        out: &mut Vec<Out>,
    ) {
        gradient(
            back,
            Rect::from_min_size(screen.min, Vec2::new(screen.width(), 96.0)),
            p.navy.gamma_multiply(0.62),
            Color32::TRANSPARENT,
        );
        // (the AF mode and area are the bottom bar's, once)
        egui::Area::new(Id::unique("vf_top_left"))
            .order(Order::Foreground)
            .fade_in(false)
            .fixed_pos(screen.left_top() + Vec2::new(GAP, 12.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 18.0;
                    if icon_text_button(
                        ui,
                        p,
                        "grid-3x3",
                        guide_label(s.guide),
                        p.ink,
                        aurora_i18n::tr!("photo-vf-guide-tip"),
                    )
                    .clicked()
                    {
                        s.guide = next_guide(s.guide);
                    }
                    if s.viewfinder.timer > 0 {
                        let text = format!("{} s", s.viewfinder.timer);
                        icon_text_button(ui, p, "clock-filled", &text, p.amber, aurora_i18n::tr!("photo-vf-timer"));
                    }
                    if self.ae_lock {
                        text_button(ui, p, "AE-L", bold(ctx, 13.0), p.teal, aurora_i18n::tr!("photo-vf-ael-on"));
                    }
                    if f.free_camera {
                        icon_text_button(ui, p, "video-filled", aurora_i18n::tr!("photo-free-camera"), p.teal, "");
                    }
                    if f.first_person {
                        icon_text_button(ui, p, "user-filled", aurora_i18n::tr!("photo-vf-first-person"), p.teal, "");
                    }
                });
            });

        // what the photo will be: each a switch, a click for the next value
        egui::Area::new(Id::unique("vf_top_right"))
            .order(Order::Foreground)
            .fade_in(false)
            .pivot(Align2::RIGHT_TOP)
            .fixed_pos(screen.right_top() + Vec2::new(-GAP, 14.0))
            .show(ctx, |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 18.0;
                    let fps = format!("{} i/s", f.fps.round() as i32);
                    let (r, _) = ui.allocate_exact_size(
                        ui.painter().layout_no_wrap(fps.clone(), regular(13.0), p.muted).size(),
                        Sense::hover(),
                    );
                    shadow_text(ui.painter(), p, r.left_top(), Align2::LEFT_TOP, &fps, regular(13.0), p.muted);
                    let (eye, hud_color) = if s.huds { ("eye-filled", p.ink) } else { ("eye-off", p.muted) };
                    if icon_text_button(
                        ui,
                        p,
                        eye,
                        aurora_i18n::tr!("photo-huds"),
                        hud_color,
                        aurora_i18n::tr!("photo-vf-hud-tip"),
                    )
                    .clicked()
                    {
                        s.huds = !s.huds;
                    }
                    let quality = if s.quality { p.ink } else { p.muted_dim };
                    if icon_switch(ui, p, "sparkles-filled", quality, aurora_i18n::tr!("photo-vf-quality-tip")).clicked() {
                        s.quality = !s.quality;
                    }
                    if icon_text_button(
                        ui,
                        p,
                        "device-floppy-filled",
                        destination_label(panel.destination),
                        p.ink,
                        aurora_i18n::tr!("photo-vf-destination-tip"),
                    )
                    .clicked()
                    {
                        panel.destination = next_destination(panel.destination);
                        out.push(Out::Panel(Action::Changed));
                    }
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let font = bold(ctx, 13.0);
                    // the file's format: the disk's choice (the profile feed takes a PNG)
                    match panel.destination {
                        Destination::Disk => {
                            if text_button(ui, p, s.format.label(), font.clone(), p.ink, aurora_i18n::tr!("photo-vf-file-tip")).clicked() {
                                let i = FileFormat::ALL.iter().position(|x| *x == s.format).unwrap_or(0);
                                s.format = FileFormat::ALL[(i + 1) % FileFormat::ALL.len()];
                            }
                            dot(ui, p);
                        }
                        Destination::Profile => {
                            text_button(ui, p, FileFormat::Png.label(), font.clone(), p.ink, "");
                            dot(ui, p);
                        }
                        Destination::Email => {
                            text_button(ui, p, FileFormat::Jpeg.label(), font.clone(), p.ink, "");
                            dot(ui, p);
                        }
                        Destination::Inventory => {}
                    }
                    // a common resolution as its badge (HD, QHD, 4K…), the
                    // pixels in its tip; the window's or a typed size in pixels
                    let pixels = format!("{} × {}", f.photo.0, f.photo.1);
                    let preset = (panel.destination == Destination::Disk).then(|| s.resolution.short()).flatten();
                    let clicked = match preset {
                        Some(short) => {
                            let tip = format!("{pixels} · {}", aurora_i18n::tr!("photo-vf-size-tip"));
                            badge_button(ui, p, short, p.ink, &tip).clicked()
                        }
                        None => text_button(ui, p, &pixels, font.clone(), p.ink, aurora_i18n::tr!("photo-vf-size-tip")).clicked(),
                    };
                    if clicked {
                        next_size(s, panel.destination);
                    }
                    dot(ui, p);
                    if text_button(ui, p, aspect_label(s.aspect), font, p.ink, aurora_i18n::tr!("photo-vf-format-tip")).clicked() {
                        let i = Aspect::ALL.iter().position(|a| *a == s.aspect).unwrap_or(0);
                        s.set_aspect(Aspect::ALL[(i + 1) % Aspect::ALL.len()], f.window);
                    }
                });
            });

        // the light of the photo
        let mut y = screen.top() + 66.0;
        let line = |y: &mut f32, name: &str, text: &str, color: Color32| {
            icon(
                back,
                name,
                Rect::from_center_size(Pos2::new(screen.left() + GAP + 8.0, *y), Vec2::splat(16.0)),
                color,
            );
            shadow_text(
                back,
                p,
                Pos2::new(screen.left() + GAP + 23.0, *y),
                Align2::LEFT_CENTER,
                text,
                regular(12.5),
                color,
            );
            *y += 24.0;
        };
        // (the time of day is a switch too)
        egui::Area::new(Id::unique("vf_time"))
            .order(Order::Foreground)
            .fade_in(false)
            .pivot(Align2::LEFT_CENTER)
            .fixed_pos(Pos2::new(screen.left() + GAP, y))
            .show(ctx, |ui| {
                if icon_text_button(
                    ui,
                    p,
                    "sun-filled",
                    time_label(panel.light.env.time),
                    p.ink,
                    aurora_i18n::tr!("photo-vf-time-tip"),
                )
                .clicked()
                {
                    panel.light.env.set_time(next_time(panel.light.env.time));
                }
            });
        y += 24.0;
        if panel.freeze_world {
            line(&mut y, "snowflake", aurora_i18n::tr!("photo-vf-world-frozen"), p.ink);
        }
        if panel.light.clouds_paused {
            line(&mut y, "cloud-filled", aurora_i18n::tr!("photo-vf-clouds-frozen"), p.ink);
        }
        if !panel.light.attached_lights {
            line(&mut y, "bulb-off", aurora_i18n::tr!("photo-vf-lights-off"), p.amber);
        }

        // the controls, under the light
        if s.viewfinder.hints && !self.menu && !self.help && !self.drawer {
            y += 14.0;
            let dial = match self.dial {
                Dial::Focal => aurora_i18n::tr!("photo-vf-dial-focal"),
                Dial::Aperture => aurora_i18n::tr!("photo-vf-dial-aperture"),
                Dial::Exposure => aurora_i18n::tr!("photo-vf-dial-exposure"),
                Dial::Distance => aurora_i18n::tr!("photo-vf-dial-distance"),
            };
            let rows: [(&str, String); 10] = [
                (
                    aurora_i18n::tr!("photo-vf-cheat-click-key"),
                    aurora_i18n::tr!("photo-vf-cheat-click").to_owned(),
                ),
                (
                    aurora_i18n::tr!("photo-vf-cheat-ctrl-click-key"),
                    aurora_i18n::tr!("photo-vf-cheat-ctrl-click").to_owned(),
                ),
                (
                    aurora_i18n::tr!("photo-vf-cheat-shift-click-key"),
                    aurora_i18n::tr!("photo-vf-cheat-shift-click").to_owned(),
                ),
                (
                    aurora_i18n::tr!("photo-vf-cheat-enter-key"),
                    aurora_i18n::tr!("photo-vf-cheat-enter").to_owned(),
                ),
                (
                    aurora_i18n::tr!("photo-vf-cheat-wheel-key"),
                    aurora_i18n::tr!("photo-vf-cheat-wheel").to_owned(),
                ),
                (aurora_i18n::tr!("photo-vf-cheat-alt-wheel-key"), dial.to_owned()),
                (
                    aurora_i18n::tr!("photo-vf-cheat-alt-shift-wheel-key"),
                    aurora_i18n::tr!("photo-vf-dial-aperture").to_owned(),
                ),
                (
                    aurora_i18n::tr!("photo-vf-cheat-alt-ctrl-wheel-key"),
                    aurora_i18n::tr!("photo-vf-dial-exposure").to_owned(),
                ),
                (
                    aurora_i18n::tr!("photo-vf-cheat-ctrl-shift-wheel-key"),
                    aurora_i18n::tr!("photo-vf-cheat-tilt").to_owned(),
                ),
                (
                    aurora_i18n::tr!("photo-vf-cheat-camera-key"),
                    aurora_i18n::tr!("photo-vf-cheat-camera").to_owned(),
                ),
            ];
            let x = screen.left() + GAP;
            for (key, text) in rows {
                let r = shadow_text(back, p, Pos2::new(x, y), Align2::LEFT_CENTER, key, bold(ctx, 12.0), p.ink);
                shadow_text(
                    back,
                    p,
                    Pos2::new(r.right() + 7.0, y),
                    Align2::LEFT_CENTER,
                    &text,
                    regular(12.0),
                    p.muted,
                );
                y += 20.0;
            }
            let keys = aurora_i18n::tr!("photo-vf-cheat-keys", menu = f.keys.menu, af = f.keys.af_mode);
            shadow_text(back, p, Pos2::new(x, y), Align2::LEFT_CENTER, &keys, regular(12.0), p.muted);
            // put away from where it is read (the quick menu brings it back)
            y += 24.0;
            let hide = egui::Area::new(Id::unique("vf_hints_hide"))
                .order(Order::Foreground)
                .fade_in(false)
                .pivot(Align2::LEFT_CENTER)
                .fixed_pos(Pos2::new(x, y))
                .show(ctx, |ui| {
                    icon_text_button(
                        ui,
                        p,
                        "eye-off",
                        aurora_i18n::tr!("photo-vf-hints-hide"),
                        p.muted,
                        aurora_i18n::tr!("photo-vf-hints-hide-tip"),
                    )
                    .clicked()
                })
                .inner;
            if hide {
                s.viewfinder.hints = false;
                self.say(aurora_i18n::tr!("photo-vf-hints-hidden", menu = f.keys.menu), false, f.now);
            }
        }

        // the histogram
        let h = Rect::from_min_size(Pos2::new(screen.right() - GAP - 150.0, screen.top() + 52.0), Vec2::new(150.0, 62.0));
        back.rect_filled(h, ctx.global_style().visuals.window_corner_radius, p.navy.gamma_multiply(0.45));
        back.rect_stroke(
            h,
            ctx.global_style().visuals.window_corner_radius,
            Stroke::new(1.0, p.ink.gamma_multiply(0.12)),
            StrokeKind::Inside,
        );
        for k in [1.0, 2.0] {
            let x = h.left() + h.width() * k / 3.0;
            back.line_segment(
                [Pos2::new(x, h.top()), Pos2::new(x, h.bottom())],
                Stroke::new(1.0, p.ink.gamma_multiply(0.12)),
            );
        }
        if let Some(bins) = f.histogram {
            let total: u32 = bins.iter().sum();
            // a smooth curve, easing between the counts read back
            let curve = vf::histogram_curve(bins);
            if self.histogram.len() != curve.len() {
                self.histogram = curve.clone();
            }
            for (shown, target) in self.histogram.iter_mut().zip(&curve) {
                *shown += (target - *shown) * 0.3;
            }
            let inner = h.shrink2(Vec2::new(0.0, 4.0));
            let points: Vec<Pos2> = self
                .histogram
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    let x = h.left() + i as f32 / (self.histogram.len() - 1) as f32 * h.width();
                    Pos2::new(x, inner.bottom() - v * inner.height())
                })
                .collect();
            let mut mesh = egui::Mesh::default();
            let fill = p.ink.gamma_multiply(0.35);
            for (i, pt) in points.iter().enumerate() {
                mesh.colored_vertex(*pt, fill);
                mesh.colored_vertex(Pos2::new(pt.x, h.bottom()), fill);
                if i > 0 {
                    let k = (i * 2) as u32;
                    mesh.add_triangle(k - 2, k - 1, k);
                    mesh.add_triangle(k - 1, k + 1, k);
                }
            }
            back.add(egui::Shape::mesh(mesh));
            back.add(egui::Shape::line(points, Stroke::new(1.2, p.ink.gamma_multiply(0.85))));
            ctx.request_repaint();
            // the highlights clip: a bar on the right
            let bright = bins[bins.len() - 2..].iter().sum::<u32>();
            if total > 0 && bright as f32 / total as f32 > 0.01 {
                let edge = Rect::from_min_max(Pos2::new(h.right() - 4.0, h.top()), h.right_bottom());
                back.rect_filled(edge, 0.0, p.amber);
            }
        }

        // the level: the horizon, so turned against the camera's tilt (a
        // camera rolled clockwise sees the ground turn the other way), and
        // the tilt, else the pitch (0: verticals stay straight)
        let c = screen.center();
        let level = controls::level_color(p, f.roll, f.pitch);
        let along = Vec2::angled(-f.roll.to_radians());
        for side in [-1.0, 1.0] {
            let a = c + along * side * 35.0;
            let b = c + along * side * 105.0;
            back.line_segment(
                [a + Vec2::new(0.0, 1.0), b + Vec2::new(0.0, 1.0)],
                Stroke::new(1.5, p.navy.gamma_multiply(0.5)),
            );
            back.line_segment([a, b], Stroke::new(1.5, level.gamma_multiply(0.8)));
        }
        let pitch = if f.roll != 0.0 {
            roll_text(f.roll)
        } else {
            format!("{}°", aurora_i18n::number(f64::from(f.pitch.round()), 0).replace('-', "−"))
        };
        shadow_text(
            back,
            p,
            c + Vec2::new(0.0, 12.0),
            Align2::CENTER_TOP,
            &pitch,
            regular(11.0),
            level.gamma_multiply(0.85),
        );
    }

    /// The AF point: brackets, or the face box of an avatar; green when
    /// sharp, red when it failed, the distance under it once locked.
    fn af_point(&self, ctx: &egui::Context, p: &Palette, painter: &egui::Painter, s: &Settings, f: &Frame) {
        let display = s.viewfinder.display;
        let phase = self.af.phase;
        let t = self.af.phase_time(f.now);
        let busy = matches!(phase, Phase::Hunt | Phase::Fail) || self.af.recent(f.now);
        if display == Display::Hidden && !busy {
            return;
        }
        let color = af_color(p, phase, true);
        // the failure blinks five times
        let blink = phase == Phase::Fail && t < 1.1 && ((t / 0.22).fract() >= 0.5);
        let color = if blink { color.gamma_multiply(0.12) } else { color };
        let scale = match phase {
            Phase::Hunt => 1.0 - 0.16 * (0.5 - 0.5 * (t / 0.28 * std::f32::consts::TAU).cos()),
            Phase::Lock if t < 0.24 => {
                let u = t / 0.24;
                if u < 0.55 {
                    1.22 + (0.93 - 1.22) * (u / 0.55)
                } else {
                    0.93 + 0.07 * ((u - 0.55) / 0.45)
                }
            }
            Phase::Track => 0.72,
            _ => 1.0,
        };
        let locked = matches!(phase, Phase::Lock | Phase::Track);
        let width = if locked { 3.0 } else { 2.2 };
        let shadow = Stroke::new(width + 1.5, p.navy.gamma_multiply(0.45));
        let face = f
            .face
            .as_ref()
            .filter(|_| s.viewfinder.area == AfArea::Avatar && s.dof.focus != PhotoFocus::Manual);
        let mut under = f.point + Vec2::new(0.0, AF_BOX / 2.0 + 6.0);
        let radius = ctx.global_style().visuals.window_corner_radius;
        // a face box with whose head the lens takes above it
        let face_box = |rect: Rect, name: &str, stroke: Stroke| {
            painter.rect_stroke(rect, radius, shadow, StrokeKind::Middle);
            painter.rect_stroke(rect, radius, stroke, StrokeKind::Middle);
            shadow_text(
                painter,
                p,
                rect.left_top() - Vec2::new(0.0, 4.0),
                Align2::LEFT_BOTTOM,
                name,
                bold(ctx, 11.0),
                stroke.color,
            );
        };
        match face {
            Some(&(rect, on, ref name)) if on || display != Display::Hidden => {
                let rect = Rect::from_center_size(rect.center(), rect.size() * if on { scale } else { 1.0 });
                let stroke = if on {
                    Stroke::new(2.5, color)
                } else {
                    Stroke::new(1.2, p.ink.gamma_multiply(0.7))
                };
                face_box(rect, name, stroke);
                if on {
                    under = Pos2::new(rect.center().x, rect.bottom() + 8.0);
                } else {
                    brackets(painter, f.point, scale, Stroke::new(width, color), shadow);
                }
            }
            _ => {
                brackets(painter, f.point, scale, Stroke::new(width, color), shadow);
                if s.dof.focus == PhotoFocus::Manual {
                    let c = f.point;
                    painter.line_segment([c - Vec2::new(6.0, 0.0), c + Vec2::new(6.0, 0.0)], Stroke::new(1.5, p.ink));
                    painter.line_segment([c - Vec2::new(0.0, 6.0), c + Vec2::new(0.0, 6.0)], Stroke::new(1.5, p.ink));
                }
            }
        }
        // the rest of a group, each face and point held the same way
        for (r, name) in &f.group {
            face_box(Rect::from_center_size(r.center(), r.size() * scale), name, Stroke::new(2.5, color));
        }
        for &at in &f.points {
            brackets(painter, at, scale, Stroke::new(width, color), shadow);
        }
        if locked && self.af.recent(f.now) && display != Display::Hidden {
            let text = match f.zone {
                // the group: from its nearest subject to its farthest
                Some((near, far)) => {
                    format!("{}–{} m", vf::distance_text(near), vf::distance_text(far))
                }
                _ if self.af.distance() >= vf::INFINITY => "∞".to_owned(),
                _ => format!("{} m", vf::distance_text(self.af.distance())),
            };
            shadow_text(painter, p, under, Align2::CENTER_TOP, &text, bold(ctx, 12.0), p.success);
        }
        if matches!(phase, Phase::Hunt | Phase::Fail) || (phase == Phase::Lock && t < 0.3) {
            ctx.request_repaint();
        }
    }

    /// The distance scale (log, infinity at the end), the zone kept sharp
    /// and the lens's needle.
    fn scale(&self, ctx: &egui::Context, p: &Palette, painter: &egui::Painter, screen: Rect, s: &Settings, f: &Frame) {
        let width = 560.0;
        let left = screen.center().x - 290.0 + 10.0;
        let line_y = screen.bottom() - 74.0 - 40.0 + 12.0;
        let x = |d: f32| left + vf::scale_position(d) * width;
        painter.line_segment(
            [Pos2::new(left, line_y), Pos2::new(left + width, line_y)],
            Stroke::new(1.0, p.ink.gamma_multiply(0.55)),
        );
        if s.dof.enabled {
            let (near, far) = f.sharp;
            let a = x(near);
            let b = x(far).max(a + 3.0);
            painter.rect_filled(
                Rect::from_min_max(Pos2::new(a, line_y - 3.0), Pos2::new(b, line_y + 4.0)),
                widgets::inner_radius(&ctx.global_style().visuals),
                p.success.gamma_multiply(0.55),
            );
        }
        for d in vf::SCALE_MARKS {
            let px = x(d);
            painter.line_segment(
                [Pos2::new(px, line_y - 3.0), Pos2::new(px, line_y + 4.0)],
                Stroke::new(1.0, p.ink.gamma_multiply(0.75)),
            );
            let label = if d >= vf::INFINITY {
                "∞".to_owned()
            } else {
                aurora_i18n::number(f64::from(d), if d < 1.0 { 1 } else { 0 })
            };
            shadow_text(
                painter,
                p,
                Pos2::new(px, line_y + 7.0),
                Align2::CENTER_TOP,
                &label,
                regular(10.5),
                p.ink,
            );
        }
        shadow_text(
            painter,
            p,
            Pos2::new(left + width + 44.0, line_y - 5.0),
            Align2::LEFT_TOP,
            "m",
            regular(10.5),
            p.muted,
        );
        // the needle
        let n = x(self.af.distance());
        let c = af_color(p, self.af.phase, false);
        painter.add(egui::Shape::convex_polygon(
            vec![
                Pos2::new(n - 6.0, line_y - 14.0),
                Pos2::new(n + 6.0, line_y - 14.0),
                Pos2::new(n, line_y - 5.0),
            ],
            c,
            Stroke::NONE,
        ));
    }

    /// The bottom bar: the focus confirmation and the AF mode on the left;
    /// focal length, aperture, exposure and distance in the middle (the
    /// one the wheel drives underlined; a click chooses it); the zone kept
    /// sharp on the right.
    #[allow(clippy::too_many_arguments)]
    fn bottom_bar(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        back: &egui::Painter,
        screen: Rect,
        s: &mut Settings,
        panel: &mut SnapshotUi,
        fov: &mut f32,
        f: &Frame,
        ctl: bool,
        out: &mut Vec<Out>,
    ) {
        gradient(
            back,
            Rect::from_min_max(Pos2::new(screen.left(), screen.bottom() - 110.0), screen.right_bottom()),
            Color32::TRANSPARENT,
            p.navy.gamma_multiply(0.72),
        );
        let y = screen.bottom() - 16.0 - 22.0;
        // the confirmation dot
        let dot = Pos2::new(screen.left() + GAP + 6.0, y);
        let t = self.af.phase_time(f.now);
        match self.af.phase {
            Phase::Lock | Phase::Track => {
                back.circle_filled(dot, 6.0, p.success);
            }
            Phase::Fail => {
                if t > 1.1 || (t / 0.22).fract() < 0.5 {
                    back.circle_filled(dot, 6.0, p.danger);
                }
            }
            Phase::Hunt => {
                let a = 0.25 + 0.75 * (0.5 + 0.5 * (t / 0.3 * std::f32::consts::TAU).cos());
                back.circle_stroke(dot, 5.0, Stroke::new(2.0, p.ink.gamma_multiply(a)));
            }
            _ => {
                back.circle_stroke(dot, 5.25, Stroke::new(1.5, p.muted.gamma_multiply(0.8)));
            }
        }
        if ctl {
            self.af_choices(ctx, p, screen, s, Pos2::new(dot.x + 6.0 + 10.0, y), out);
        }
        let c = af_color(p, self.af.phase, false);
        let r = shadow_text(
            back,
            p,
            Pos2::new(dot.x + 6.0 + 14.0, y),
            Align2::LEFT_CENTER,
            if ctl { "" } else { af_label(s.dof.focus) },
            bold(ctx, 15.0),
            c,
        );
        let area = if ctl {
            ""
        } else if s.dof.focus == PhotoFocus::Manual {
            aurora_i18n::tr!("photo-vf-area-manual")
        } else if s.viewfinder.area == AfArea::Avatar {
            aurora_i18n::tr!("photo-vf-area-avatar")
        } else {
            aurora_i18n::tr!("photo-vf-area-point")
        };
        shadow_text(
            back,
            p,
            Pos2::new(r.right() + 14.0, y),
            Align2::LEFT_CENTER,
            area,
            regular(13.0),
            p.muted,
        );

        // the zone kept sharp
        let right = screen.right() - GAP;
        shadow_text(
            back,
            p,
            Pos2::new(right, y - 4.0),
            Align2::RIGHT_BOTTOM,
            aurora_i18n::tr!("photo-vf-sharp-from"),
            regular(11.0),
            p.muted,
        );
        let range = if s.dof.enabled {
            let (near, far) = f.sharp;
            if far >= vf::INFINITY {
                format!("{} – ∞", vf::distance_text(near))
            } else {
                format!("{} – {} m", vf::distance_text(near), vf::distance_text(far))
            }
        } else {
            aurora_i18n::tr!("photo-vf-all-sharp").to_owned()
        };
        if s.dof.enabled {
            shadow_text(
                back,
                p,
                Pos2::new(right, y - 2.0),
                Align2::RIGHT_TOP,
                &range,
                bold(ctx, 15.0),
                p.ink,
            );
        } else {
            // the depth of field off: a click turns it on
            egui::Area::new(Id::unique("vf_all_sharp"))
                .order(Order::Foreground)
                .fade_in(false)
                .pivot(Align2::RIGHT_TOP)
                .fixed_pos(Pos2::new(right, y - 2.0))
                .show(ctx, |ui| {
                    if text_button(ui, p, &range, bold(ctx, 15.0), p.ink, aurora_i18n::tr!("photo-vf-dof-off-tip")).clicked() {
                        s.dof.enabled = true;
                        self.say(aurora_i18n::tr!("photo-vf-dof-on"), false, f.now);
                    }
                });
        }

        // the dials
        egui::Area::new(Id::unique("vf_dials"))
            .order(Order::Foreground)
            .fade_in(false)
            .pivot(Align2::CENTER_BOTTOM)
            .fixed_pos(Pos2::new(screen.center().x, screen.bottom() - 16.0))
            .show(ctx, |ui| {
                // top-aligned, each block lowered so that its line (the
                // values' text, the scale's ticks, the AE-L frame) sits
                // MIDDLE points down: egui does not move the blocks already
                // laid when a taller one comes
                const MIDDLE: f32 = 22.0;
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 30.0;
                    let font = bold(ctx, 22.0);
                    // each value turns where it is, pressed and dragged or
                    // the wheel over it; a click underlines it for Alt + wheel
                    let dial = |ui: &mut egui::Ui, d: Dial, text: &str, dim: bool, tip: &str, current: Dial| {
                        let color = if dim { p.ink.gamma_multiply(0.45) } else { p.ink };
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 3.0;
                            // the text's middle (22-point figures) on the line
                            ui.add_space(MIDDLE - 13.3);
                            let r = value_button(ui, p, text, font.clone(), color, tip);
                            let (line, _) = ui.allocate_exact_size(Vec2::new(r.rect.width(), 2.0), Sense::hover());
                            if current == d {
                                ui.painter().rect_filled(line, 0.0, p.teal);
                            }
                            r
                        })
                        .inner
                    };
                    let resp = dial(
                        ui,
                        Dial::Focal,
                        &format!("{} mm", f.focal_mm.round() as i32),
                        false,
                        aurora_i18n::tr!("photo-vf-focal-tip"),
                        self.dial,
                    );
                    self.dial_input(ui, &resp, Dial::Focal, s, panel, fov, f, out);
                    let n = s.dof.f_number;
                    let resp = dial(
                        ui,
                        Dial::Aperture,
                        &format!("f/{}", aurora_i18n::number(f64::from(n), if n < 10.0 { 1 } else { 0 })),
                        !s.dof.enabled,
                        aurora_i18n::tr!("photo-vf-aperture-tip"),
                        self.dial,
                    );
                    self.dial_input(ui, &resp, Dial::Aperture, s, panel, fov, f, out);
                    // the exposure scale, by thirds
                    let resp = ui
                        .vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 3.0;
                            // the ticks' middle on the line
                            ui.add_space(MIDDLE - 13.0);
                            let (rect, resp) = ui.allocate_exact_size(Vec2::new(170.0, 36.0), Sense::click_and_drag());
                            let painter = ui.painter();
                            let base = rect.left() + 6.0;
                            let bottom = rect.top() + 20.0;
                            for i in 0..=12 {
                                let x = base + i as f32 * 12.2;
                                let big = i % 3 == 0;
                                let h = if i == 6 {
                                    12.0
                                } else if big {
                                    8.0
                                } else {
                                    4.0
                                };
                                painter.line_segment(
                                    [Pos2::new(x, bottom - h), Pos2::new(x, bottom)],
                                    Stroke::new(if big { 1.5 } else { 1.0 }, p.ink),
                                );
                            }
                            let m = base + (f.ev.clamp(-2.0, 2.0) + 2.0) * 3.0 * 12.2;
                            let mc = if f.ev.abs() < 0.01 { p.ink } else { p.teal };
                            painter.add(egui::Shape::convex_polygon(
                                vec![
                                    Pos2::new(m - 4.5, rect.top()),
                                    Pos2::new(m + 4.5, rect.top()),
                                    Pos2::new(m, rect.top() + 6.0),
                                ],
                                mc,
                                Stroke::NONE,
                            ));
                            for (k, label) in ["−2", "−1", "0", "+1", "+2"].iter().enumerate() {
                                let x = base + k as f32 * 3.0 * 12.2;
                                shadow_text(
                                    painter,
                                    p,
                                    Pos2::new(x, bottom + 2.0),
                                    Align2::CENTER_TOP,
                                    label,
                                    regular(10.5),
                                    p.muted,
                                );
                            }
                            let (line, _) = ui.allocate_exact_size(Vec2::new(170.0, 2.0), Sense::hover());
                            if self.dial == Dial::Exposure {
                                ui.painter().rect_filled(line, 0.0, p.teal);
                            }
                            resp
                        })
                        .inner;
                    let resp = resp
                        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
                        .on_hover_text(aurora_i18n::tr!("photo-vf-exposure-tip"));
                    self.dial_input(ui, &resp, Dial::Exposure, s, panel, fov, f, out);
                    // AE-L: the automatic exposure held where it is, its
                    // frame's middle on the line
                    let (cell, _) = ui.allocate_exact_size(Vec2::new(44.0, MIDDLE + 13.0), Sense::hover());
                    let rect = Rect::from_min_size(Pos2::new(cell.left(), cell.top() + MIDDLE - 13.0), Vec2::new(44.0, 26.0));
                    let resp = ui.interact(rect, Id::unique("vf_ael"), Sense::click());
                    let color = if self.ae_lock {
                        p.teal
                    } else if resp.hovered() {
                        p.ink
                    } else {
                        p.ink.gamma_multiply(0.45)
                    };
                    ui.painter().rect_stroke(
                        rect,
                        widgets::inner_radius(ui.visuals()),
                        Stroke::new(1.5, color),
                        StrokeKind::Inside,
                    );
                    ui.painter()
                        .text(rect.center(), Align2::CENTER_CENTER, "AE-L", bold(ctx, 12.0), color);
                    keep_keys(&resp);
                    if resp
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text(aurora_i18n::tr!("photo-vf-ael-tip"))
                        .clicked()
                    {
                        self.ae_lock = !self.ae_lock;
                        let text = if self.ae_lock {
                            aurora_i18n::tr!("photo-vf-ael-on")
                        } else {
                            aurora_i18n::tr!("photo-vf-ael-off")
                        };
                        self.say(text, false, f.now);
                    }
                    let d = self.af.distance();
                    let text = if d >= vf::INFINITY {
                        "∞".to_owned()
                    } else {
                        format!("{} m", vf::distance_text(d))
                    };
                    let resp = dial(
                        ui,
                        Dial::Distance,
                        &text,
                        false,
                        aurora_i18n::tr!("photo-vf-distance-tip"),
                        self.dial,
                    );
                    self.dial_input(ui, &resp, Dial::Distance, s, panel, fov, f, out);
                });
            });
    }

    /// A value of the bottom bar clicked (underlined: the ring's and Alt +
    /// wheel's), dragged or turned with the wheel.
    #[allow(clippy::too_many_arguments)]
    fn dial_input(
        &mut self,
        ui: &egui::Ui,
        resp: &egui::Response,
        dial: Dial,
        s: &mut Settings,
        panel: &mut SnapshotUi,
        fov: &mut f32,
        f: &Frame,
        out: &mut Vec<Out>,
    ) {
        if resp.clicked() {
            self.dial = dial;
        }
        self.turn_dial(ui, resp, dial, 1.0, s, panel, fov, f, out);
    }

    /// The dock on the right: everything the mouse needs.
    fn dock(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        screen: Rect,
        s: &mut Settings,
        panel: &mut SnapshotUi,
        f: &Frame,
        out: &mut Vec<Out>,
    ) {
        egui::Area::new(Id::unique("vf_dock"))
            .order(Order::Foreground)
            .fade_in(false)
            .pivot(Align2::RIGHT_CENTER)
            .fixed_pos(Pos2::new(screen.right() - 20.0, screen.center().y))
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.spacing_mut().item_spacing.y = 10.0;
                    if dock_button(
                        ui,
                        p,
                        Face::Icon("layout-grid-filled"),
                        &aurora_i18n::tr!("photo-vf-menu-tip", key = f.keys.menu),
                        self.menu,
                    ) {
                        self.menu = !self.menu;
                        self.help = false;
                    }
                    if dock_button(
                        ui,
                        p,
                        Face::Icon("adjustments-horizontal-filled"),
                        aurora_i18n::tr!("photo-vf-drawer-tip"),
                        self.drawer,
                    ) {
                        self.drawer = !self.drawer;
                        self.looks &= !self.drawer;
                        self.presets &= !self.drawer;
                        self.light &= !self.drawer;
                        if self.drawer && self.isolate {
                            self.isolate = false;
                            out.push(Out::Isolate(Isolate::Picking(false)));
                        }
                    }
                    if dock_button(ui, p, Face::Text("DISP"), aurora_i18n::tr!("photo-vf-disp-tip"), false) {
                        s.viewfinder.display = s.viewfinder.display.next();
                    }
                    if dock_button(
                        ui,
                        p,
                        Face::Icon("device-gamepad-filled"),
                        aurora_i18n::tr!("photo-vf-pad-toggle-tip"),
                        s.viewfinder.pad,
                    ) {
                        s.viewfinder.pad = !s.viewfinder.pad;
                    }
                    if dock_button(
                        ui,
                        p,
                        Face::Icon("snowflake"),
                        aurora_i18n::tr!("photo-freeze-world-tip"),
                        panel.freeze_world,
                    ) {
                        panel.freeze_world = !panel.freeze_world;
                    }
                    // the drawer's Light section alone, next to the shutter
                    if dock_button(
                        ui,
                        p,
                        Face::Icon("sun-filled"),
                        aurora_i18n::tr!("photo-vf-light-tip"),
                        self.light,
                    ) {
                        self.light = !self.light;
                        self.drawer &= !self.light;
                        self.looks &= !self.light;
                        self.presets &= !self.light;
                        if self.light && self.isolate {
                            self.isolate = false;
                            out.push(Out::Isolate(Isolate::Picking(false)));
                        }
                    }
                    // six above the shutter, seven under it (the panels of
                    // the shot together)
                    ui.add_space(6.0);
                    self.shutter_button(ui, p, out);
                    ui.add_space(6.0);
                    let isolating = self.isolate || f.isolation.mode != crate::snapshot::isolate::Mode::Off;
                    if dock_button(ui, p, Face::Icon("user-scan"), aurora_i18n::tr!("photo-isolate-tip"), isolating) {
                        self.isolate = !self.isolate;
                        self.drawer &= !self.isolate;
                        self.looks &= !self.isolate;
                        self.presets &= !self.isolate;
                        self.light &= !self.isolate;
                        if !self.isolate {
                            out.push(Out::Isolate(Isolate::Picking(false)));
                        }
                    }
                    if dock_button(
                        ui,
                        p,
                        Face::Icon("color-filter"),
                        aurora_i18n::tr!("photo-looks-tip"),
                        self.looks || !s.looks.is_empty(),
                    ) {
                        self.looks = !self.looks;
                        self.drawer &= !self.looks;
                        self.presets &= !self.looks;
                        self.light &= !self.looks;
                        // opens on the tab of the chosen look
                        if self.looks
                            && let Some(layer) = s.looks.get(panel.looks.selected)
                        {
                            panel.looks.tab = crate::snapshot::looks::tab_of(&layer.id);
                        }
                        if self.looks && self.isolate {
                            self.isolate = false;
                            out.push(Out::Isolate(Isolate::Picking(false)));
                        }
                    }
                    if dock_button(
                        ui,
                        p,
                        Face::Icon("bookmark-filled"),
                        aurora_i18n::tr!("camera-presets-tip"),
                        self.presets,
                    ) {
                        self.presets = !self.presets;
                        self.drawer &= !self.presets;
                        self.looks &= !self.presets;
                        self.light &= !self.presets;
                        if self.presets && self.isolate {
                            self.isolate = false;
                            out.push(Out::Isolate(Isolate::Picking(false)));
                        }
                    }
                    // the toolbar's Poser, the interface being hidden: pose
                    // the avatar without leaving the shot
                    if dock_button(ui, p, Face::Icon("wand"), aurora_i18n::tr!("photo-vf-poser-tip"), f.posing) {
                        out.push(Out::Poser);
                    }
                    let has_photo = f.review.is_some();
                    if dock_button(ui, p, Face::Icon("photo-filled"), aurora_i18n::tr!("photo-vf-last-photo"), false) && has_photo {
                        self.review_until = if f.now < self.review_until { 0.0 } else { f.now + 4.8 };
                    }
                    if dock_button(ui, p, Face::Text("?"), aurora_i18n::tr!("photo-vf-help-tip"), self.help) {
                        self.help = !self.help;
                        self.menu = false;
                    }
                    if dock_button(ui, p, Face::Icon("x"), aurora_i18n::tr!("photo-vf-exit-tip"), false) {
                        out.push(Out::Exit);
                    }
                });
            });
    }

    /// The round shutter: pressed, the focus; released on it, the photo;
    /// let go elsewhere, nothing.
    fn shutter_button(&mut self, ui: &mut egui::Ui, p: &Palette, out: &mut Vec<Out>) {
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(66.0), Sense::click_and_drag());
        let down = resp.is_pointer_button_down_on();
        if down && !self.mouse_down {
            self.mouse_down = true;
            out.push(Out::HalfPress);
        } else if !down && self.mouse_down {
            self.mouse_down = false;
            out.push(if resp.hovered() { Out::Release } else { Out::Cancel });
        }
        let locked = matches!(self.af.phase, Phase::Lock | Phase::Track);
        let c = rect.center();
        ui.painter().circle_filled(c, 33.0, p.navy.gamma_multiply(0.35));
        let ring = if self.pressed && locked { p.success } else { p.ink };
        ui.painter().circle_stroke(c, 31.5, Stroke::new(3.0, ring));
        let fill = if self.pressed {
            if locked { p.success } else { p.ink }
        } else {
            p.ink.gamma_multiply(0.92)
        };
        ui.painter().circle_filled(c, if self.pressed { 25.0 * 0.86 } else { 25.0 }, fill);
        keep_keys(&resp);
        resp.on_hover_text(aurora_i18n::tr!("photo-vf-shutter-tip"));
    }

    /// The quick menu: a tile per setting, a click for the next value.
    #[allow(clippy::too_many_arguments)]
    fn quick_menu(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        screen: Rect,
        s: &mut Settings,
        panel: &mut SnapshotUi,
        f: &Frame,
        out: &mut Vec<Out>,
    ) {
        let cols = 9.0;
        let width = (screen.width() - 2.0 * GAP - 120.0).min(1300.0);
        egui::Area::new(Id::unique("vf_menu"))
            .order(Order::Foreground)
            .fade_in(false)
            .pivot(Align2::CENTER_BOTTOM)
            .fixed_pos(Pos2::new(screen.center().x, screen.bottom() - 128.0))
            .show(ctx, |ui| {
                widgets::overlay(ui, p, egui::Margin::same(14), |ui| {
                    ui.set_width(width - 28.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(aurora_i18n::tr!("photo-vf-menu"))
                                .font(bold(ctx, 14.0))
                                .color(p.ink),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                egui::RichText::new(aurora_i18n::tr!("photo-vf-menu-hint", key = f.keys.menu))
                                    .size(12.0)
                                    .color(p.muted),
                            );
                        });
                    });
                    ui.add_space(6.0);
                    let tile_w = (width - 28.0 - 8.0 * (cols - 1.0)) / cols;
                    let tiles = tiles(s, panel, f);
                    egui::Grid::new("vf_tiles")
                        .num_columns(cols as usize)
                        .spacing(Vec2::splat(8.0))
                        .show(ui, |ui| {
                            for (i, (label, value, step)) in tiles.into_iter().enumerate() {
                                if tile(ui, p, Vec2::new(tile_w, 68.0), label, &value)
                                    && let Some(o) = step(s, panel)
                                {
                                    out.push(o);
                                }
                                if (i + 1) % cols as usize == 0 {
                                    ui.end_row();
                                }
                            }
                        });
                });
            });
    }

    /// The keys and the mouse, in a panel in the middle.
    fn help_panel(&mut self, ctx: &egui::Context, p: &Palette, screen: Rect, f: &Frame) {
        egui::Area::new(Id::unique("vf_help"))
            .order(Order::Foreground)
            .fade_in(false)
            .pivot(Align2::CENTER_CENTER)
            .fixed_pos(screen.center())
            .show(ctx, |ui| {
                widgets::overlay(ui, p, egui::Margin::symmetric(26, 22), |ui| {
                    ui.set_width(548.0);
                    ui.spacing_mut().item_spacing.y = 8.0;
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(aurora_i18n::tr!("photo-vf-help-title"))
                                .font(bold(ctx, 17.0))
                                .color(p.ink),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::icon_button(ui, p, "x", aurora_i18n::tr!("common-close"), true) {
                                self.help = false;
                            }
                        });
                    });
                    let menu = f.keys.menu;
                    let af = f.keys.af_mode;
                    let sections: [(&str, Vec<(String, &str)>); 3] = [
                        (
                            aurora_i18n::tr!("photo-vf-help-aim"),
                            vec![
                                (
                                    aurora_i18n::tr!("photo-vf-help-click").to_owned(),
                                    aurora_i18n::tr!("photo-vf-help-click-text"),
                                ),
                                (
                                    aurora_i18n::tr!("photo-vf-cheat-ctrl-click-key").to_owned(),
                                    aurora_i18n::tr!("photo-vf-help-ctrl-click-text"),
                                ),
                                (
                                    aurora_i18n::tr!("photo-vf-help-shift-click").to_owned(),
                                    aurora_i18n::tr!("photo-vf-help-shift-click-text"),
                                ),
                                (
                                    aurora_i18n::tr!("photo-vf-help-alt-click").to_owned(),
                                    aurora_i18n::tr!("photo-vf-help-alt-click-text"),
                                ),
                                (
                                    aurora_i18n::tr!("photo-vf-help-enter-down").to_owned(),
                                    aurora_i18n::tr!("photo-vf-help-enter-down-text"),
                                ),
                                (
                                    aurora_i18n::tr!("photo-vf-help-enter-up").to_owned(),
                                    aurora_i18n::tr!("photo-vf-help-enter-up-text"),
                                ),
                                (
                                    aurora_i18n::tr!("photo-vf-help-button").to_owned(),
                                    aurora_i18n::tr!("photo-vf-help-button-text"),
                                ),
                            ],
                        ),
                        (
                            aurora_i18n::tr!("photo-vf-help-adjust"),
                            vec![
                                (
                                    aurora_i18n::tr!("photo-vf-help-wheel").to_owned(),
                                    aurora_i18n::tr!("photo-vf-help-wheel-text"),
                                ),
                                (
                                    aurora_i18n::tr!("photo-vf-help-alt-wheel").to_owned(),
                                    aurora_i18n::tr!("photo-vf-help-alt-wheel-text"),
                                ),
                                (
                                    aurora_i18n::tr!("photo-vf-help-alt-shift-wheel").to_owned(),
                                    aurora_i18n::tr!("photo-vf-help-alt-shift-wheel-text"),
                                ),
                                (
                                    aurora_i18n::tr!("photo-vf-help-alt-ctrl-wheel").to_owned(),
                                    aurora_i18n::tr!("photo-vf-help-alt-ctrl-wheel-text"),
                                ),
                                (
                                    aurora_i18n::tr!("photo-vf-help-ctrl-shift-wheel").to_owned(),
                                    aurora_i18n::tr!("photo-vf-help-ctrl-shift-wheel-text"),
                                ),
                                (
                                    aurora_i18n::tr!("photo-vf-help-digits").to_owned(),
                                    aurora_i18n::tr!("photo-vf-help-digits-text"),
                                ),
                            ],
                        ),
                        (aurora_i18n::tr!("photo-vf-help-focus"), {
                            let mut v = vec![
                                (af.to_owned(), aurora_i18n::tr!("photo-vf-help-af-text")),
                                ("V".to_owned(), aurora_i18n::tr!("photo-vf-help-area-text")),
                                ("L".to_owned(), aurora_i18n::tr!("photo-vf-help-ael-text")),
                                ("I".to_owned(), aurora_i18n::tr!("photo-vf-help-disp-text")),
                                ("G".to_owned(), aurora_i18n::tr!("photo-vf-help-guide-text")),
                                (menu.to_owned(), aurora_i18n::tr!("photo-vf-help-menu-text")),
                            ];
                            if let Some(k) = f.keys.first_person {
                                v.push((k.to_owned(), aurora_i18n::tr!("photo-vf-help-first-person-text")));
                            }
                            v.push((
                                aurora_i18n::tr!("photo-vf-help-h").to_owned(),
                                aurora_i18n::tr!("photo-vf-help-h-text"),
                            ));
                            v.push((
                                aurora_i18n::tr!("photo-vf-help-escape").to_owned(),
                                aurora_i18n::tr!("photo-vf-help-escape-text"),
                            ));
                            v
                        }),
                    ];
                    egui::ScrollArea::vertical()
                        .id_salt("vf_help_scroll")
                        .max_height((screen.height() - 112.0).max(80.0))
                        .show(ui, |ui| {
                            for (title, rows) in sections {
                                ui.add_space(6.0);
                                ui.label(egui::RichText::new(title).font(bold(ctx, 12.0)).color(p.teal));
                                egui::Grid::new(("vf_help", title))
                                    .num_columns(2)
                                    .spacing(Vec2::new(14.0, 8.0))
                                    .min_col_width(170.0)
                                    .show(ui, |ui| {
                                        for (key, text) in rows {
                                            ui.label(egui::RichText::new(key).font(bold(ctx, 13.0)).color(p.ink));
                                            ui.add(egui::Label::new(egui::RichText::new(text).size(13.0).color(p.muted)).wrap());
                                            ui.end_row();
                                        }
                                    });
                            }
                            ui.add_space(8.0);
                            widgets::divider(ui, p);
                            ui.label(
                                egui::RichText::new(aurora_i18n::tr!("photo-vf-help-free-camera"))
                                    .size(12.0)
                                    .color(p.muted),
                            );
                        });
                });
            });
    }

    /// Every setting of the photo panel, in a drawer left of the dock.
    #[allow(clippy::too_many_arguments)]
    fn drawer(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        screen: Rect,
        s: &mut Settings,
        panel: &mut SnapshotUi,
        fov: &mut f32,
        default_fov: f32,
        view: &View,
        out: &mut Vec<Out>,
    ) {
        egui::Area::new(Id::unique("vf_drawer"))
            .order(Order::Foreground)
            .fade_in(false)
            .pivot(Align2::RIGHT_TOP)
            .fixed_pos(Pos2::new(screen.right() - 84.0, screen.top() + 128.0))
            .show(ctx, |ui| {
                widgets::overlay(ui, p, egui::Margin::same(12), |ui| {
                    ui.set_width(316.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(aurora_i18n::tr!("photo-vf-drawer"))
                                .font(bold(ctx, 14.0))
                                .color(p.ink),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::icon_button(ui, p, "x", aurora_i18n::tr!("common-close"), true) {
                                self.drawer = false;
                            }
                        });
                    });
                    let mut actions = Vec::new();
                    // above the bottom bar, the shutter and the gallery under it
                    let room = (screen.height() - 128.0 - 120.0 - 170.0).max(160.0);
                    panel.drawer(ui, p, s, fov, default_fov, view, room, &mut actions);
                    out.extend(actions.into_iter().map(Out::Panel));
                });
            });
    }
}

/// The AF point's colour: green when sharp, red when it failed, white
/// otherwise (the confirmation colours of a camera, from the palette).
fn af_color(p: &Palette, phase: Phase, _point: bool) -> Color32 {
    match phase {
        Phase::Lock | Phase::Track => p.success,
        Phase::Fail => p.danger,
        _ => p.ink,
    }
}

/// Four corner brackets around `c`.
fn brackets(painter: &egui::Painter, c: Pos2, scale: f32, stroke: Stroke, shadow: Stroke) {
    let h = AF_BOX / 2.0 * scale - 3.0;
    let l = 19.0 * scale;
    for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        let corner = c + Vec2::new(sx * h, sy * h);
        let pts = vec![corner - Vec2::new(0.0, sy * l), corner, corner - Vec2::new(sx * l, 0.0)];
        painter.add(egui::Shape::line(pts.clone(), shadow));
        painter.add(egui::Shape::line(pts, stroke));
    }
}

/// The first hint: how to focus and take the photo.
fn hint(ctx: &egui::Context, p: &Palette, screen: Rect, bottom: f32, pad_hint: bool) {
    egui::Area::new(Id::unique("vf_hint"))
        .order(Order::Foreground)
        .fade_in(false)
        .interactable(false)
        .pivot(Align2::CENTER_BOTTOM)
        .fixed_pos(Pos2::new(screen.center().x, screen.bottom() - bottom))
        .show(ctx, |ui| {
            widgets::overlay(ui, p, egui::Margin::symmetric(16, 9), |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 18.0;
                    let t = |ui: &mut egui::Ui, s: &str| {
                        ui.label(egui::RichText::new(s).size(13.0).color(p.ink));
                    };
                    t(ui, aurora_i18n::tr!("photo-vf-hint-click"));
                    ui.label(egui::RichText::new("·").color(p.muted_dim));
                    t(ui, aurora_i18n::tr!("photo-vf-hint-enter"));
                    ui.label(egui::RichText::new("·").color(p.muted_dim));
                    t(ui, aurora_i18n::tr!("photo-vf-hint-help"));
                    if pad_hint {
                        ui.label(egui::RichText::new("·").color(p.muted_dim));
                        t(ui, aurora_i18n::tr!("photo-vf-hint-pad"));
                    }
                });
            });
        });
}

/// A line at the top (an error in its colour). One line, never wrapped: the
/// area keeps the size of the last message, and a longer one wrapped into
/// it would stand as a column of syllables.
fn message(ctx: &egui::Context, p: &Palette, screen: Rect, text: &str, error: bool) {
    egui::Area::new(Id::unique("vf_message"))
        .order(Order::Foreground)
        .fade_in(false)
        .interactable(false)
        .pivot(Align2::CENTER_TOP)
        .fixed_pos(Pos2::new(screen.center().x, screen.top() + 64.0))
        .show(ctx, |ui| {
            widgets::overlay(ui, p, egui::Margin::symmetric(16, 8), |ui| {
                let color = if error { p.danger } else { p.ink };
                ui.add(egui::Label::new(egui::RichText::new(text).size(13.0).color(color)).extend());
            });
        });
}

/// With the overlay hidden: a small DISP button to bring it back.
fn ghost_disp(ctx: &egui::Context, p: &Palette, screen: Rect) -> bool {
    let id = Id::unique("vf_ghost");
    // discreet over the photo, plain under the pointer
    let hovered = ctx
        .memory(|m| m.area_rect(id))
        .zip(ctx.pointer_hover_pos())
        .is_some_and(|(r, pos)| r.contains(pos));
    let mut clicked = false;
    egui::Area::new(id)
        .order(Order::Foreground)
        .fade_in(false)
        .pivot(Align2::RIGHT_BOTTOM)
        .fixed_pos(screen.right_bottom() - Vec2::new(20.0, 18.0))
        .show(ctx, |ui| {
            ui.multiply_opacity(if hovered { 1.0 } else { 0.7 });
            widgets::overlay(ui, p, egui::Margin::symmetric(10, 6), |ui| {
                let text = aurora_i18n::tr!("photo-vf-show-ui");
                clicked = icon_text_button(ui, p, "eye-filled", text, p.ink, aurora_i18n::tr!("photo-vf-disp-tip")).clicked();
            });
        });
    clicked
}

/// The card of the photo just taken.
fn review(ctx: &egui::Context, p: &Palette, screen: Rect, f: &Frame, out: &mut Vec<Out>, bottom: f32) {
    let Some((texture, label, kind)) = &f.review else {
        return;
    };
    egui::Area::new(Id::unique("vf_review"))
        .order(Order::Foreground)
        .fade_in(false)
        .pivot(Align2::LEFT_BOTTOM)
        .fixed_pos(Pos2::new(screen.left() + GAP, screen.bottom() - bottom))
        .show(ctx, |ui| {
            widgets::overlay(ui, p, egui::Margin::same(8), |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(150.0, 100.0), Sense::hover());
                    // the photo covering the card's picture
                    let size = texture.size_vec2();
                    let (r, c) = (size.x / size.y.max(1.0), 1.5);
                    let uv = if r > c {
                        let w = c / r;
                        Rect::from_min_max(Pos2::new(0.5 - w / 2.0, 0.0), Pos2::new(0.5 + w / 2.0, 1.0))
                    } else {
                        let h = r / c;
                        Rect::from_min_max(Pos2::new(0.0, 0.5 - h / 2.0), Pos2::new(1.0, 0.5 + h / 2.0))
                    };
                    ui.painter().image(texture.id(), rect, uv, widgets::PICTURE_TINT);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 6.0;
                        let title = match kind {
                            Review::Disk(_) => aurora_i18n::tr!("photo-vf-saved"),
                            Review::Inventory => aurora_i18n::tr!("photo-uploaded"),
                            Review::Profile => aurora_i18n::tr!("photo-profile-posted"),
                            Review::Email => aurora_i18n::tr!("photo-email-sent"),
                        };
                        // one line each, like the message at the top
                        ui.add(egui::Label::new(egui::RichText::new(title).font(bold(ctx, 13.0)).color(p.ink)).extend());
                        ui.add(egui::Label::new(egui::RichText::new(label).size(12.0).color(p.muted)).extend());
                        ui.horizontal(|ui| match kind {
                            Review::Disk(path) => {
                                if widgets::flat_button(ui, p, aurora_i18n::tr!("photo-open")).clicked() {
                                    out.push(Out::Panel(Action::OpenPhoto(path.clone())));
                                }
                                if widgets::flat_button(ui, p, aurora_i18n::tr!("photo-vf-folder")).clicked() {
                                    out.push(Out::Panel(Action::OpenFolder));
                                }
                            }
                            Review::Inventory => {
                                if widgets::flat_button(ui, p, aurora_i18n::tr!("photo-show-in-inventory")).clicked() {
                                    out.push(Out::Panel(Action::ShowItem));
                                }
                            }
                            Review::Profile => {
                                if widgets::flat_button(ui, p, aurora_i18n::tr!("photo-profile-show")).clicked() {
                                    out.push(Out::Panel(Action::ShowProfile));
                                }
                            }
                            Review::Email => {}
                        });
                    });
                });
            });
        });
    let _ = screen;
}

/// The face of a dock button.
enum Face<'a> {
    Icon(&'a str),
    Text(&'a str),
}

/// A 44-point button of the dock; true when clicked.
fn dock_button(ui: &mut egui::Ui, p: &Palette, face: Face, tip: &str, on: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(44.0), Sense::click());
    let radius = ui.visuals().window_corner_radius;
    let fill = if resp.hovered() || on {
        p.panel.gamma_multiply(0.85)
    } else {
        p.navy.gamma_multiply(0.55)
    };
    ui.painter().rect_filled(rect, radius, fill);
    let edge = if on {
        p.teal.gamma_multiply(0.7)
    } else {
        p.ink.gamma_multiply(0.16)
    };
    ui.painter().rect_stroke(rect, radius, Stroke::new(1.0, edge), StrokeKind::Inside);
    match face {
        Face::Icon(name) => icon(ui.painter(), name, Rect::from_center_size(rect.center(), Vec2::splat(20.0)), p.ink),
        Face::Text(text) => {
            let size = if text.chars().count() == 1 { 16.0 } else { 11.0 };
            ui.painter()
                .text(rect.center(), Align2::CENTER_CENTER, text, bold(ui.ctx(), size), p.ink);
        }
    }
    keep_keys(&resp);
    resp.on_hover_text(tip).clicked()
}

/// A tile of the quick menu; true when clicked.
fn tile(ui: &mut egui::Ui, p: &Palette, size: Vec2, label: &str, value: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let radius = ui.visuals().window_corner_radius;
    ui.painter().rect_filled(rect, radius, p.panel.gamma_multiply(0.85));
    let edge = if resp.hovered() {
        p.teal.gamma_multiply(0.6)
    } else {
        p.ink.gamma_multiply(0.08)
    };
    ui.painter().rect_stroke(rect, radius, Stroke::new(1.0, edge), StrokeKind::Inside);
    let left = rect.left() + 12.0;
    ui.painter().text(
        Pos2::new(left, rect.center().y - 4.0),
        Align2::LEFT_BOTTOM,
        label,
        regular(11.0),
        p.muted,
    );
    let galley = ui
        .painter()
        .layout(value.to_owned(), bold(ui.ctx(), 14.0), p.ink, rect.width() - 24.0);
    let mut job_rect = Rect::from_min_size(Pos2::new(left, rect.center().y), galley.size());
    job_rect.max.x = job_rect.max.x.min(rect.right() - 12.0);
    ui.painter().with_clip_rect(rect.shrink(2.0)).galley(job_rect.min, galley, p.ink);
    keep_keys(&resp);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

type Step = Box<dyn FnOnce(&mut Settings, &mut SnapshotUi) -> Option<Out>>;

/// The quick menu's tiles: (label, value, what a click does).
fn tiles(s: &Settings, panel: &SnapshotUi, f: &Frame) -> Vec<(&'static str, String, Step)> {
    let yes_no = |on: bool| on_off(on, aurora_i18n::tr!("photo-vf-on"), aurora_i18n::tr!("photo-vf-off")).to_owned();
    let mut v: Vec<(&'static str, String, Step)> = Vec::new();
    let window = f.window;
    v.push((
        aurora_i18n::tr!("photo-section-format"),
        aspect_label(s.aspect).to_owned(),
        Box::new(move |s, _| {
            let i = Aspect::ALL.iter().position(|a| *a == s.aspect).unwrap_or(0);
            s.set_aspect(Aspect::ALL[(i + 1) % Aspect::ALL.len()], window);
            None
        }),
    ));
    // the size: the disk's resolution, or the destination's own sizes
    let size_value = match panel.destination {
        Destination::Disk => match s.resolution {
            Resolution::Window => aurora_i18n::tr!("photo-resolution-window").to_owned(),
            Resolution::Custom => aurora_i18n::tr!("photo-resolution-custom").to_owned(),
            r => r.short().unwrap_or("").to_owned(),
        },
        Destination::Inventory => s
            .inventory_size
            .side()
            .map_or_else(|| aurora_i18n::tr!("photo-resolution-window").to_owned(), |v| v.to_string()),
        Destination::Profile | Destination::Email => {
            let (w, h) = f.photo;
            format!("{w} × {h}")
        }
    };
    v.push((
        aurora_i18n::tr!("photo-section-resolution"),
        size_value,
        Box::new(|s, panel| {
            next_size(s, panel.destination);
            None
        }),
    ));
    v.push((
        aurora_i18n::tr!("photo-section-destination"),
        destination_label(panel.destination).to_owned(),
        Box::new(|_, panel| {
            panel.destination = next_destination(panel.destination);
            Some(Out::Panel(Action::Changed))
        }),
    ));
    v.push((
        aurora_i18n::tr!("photo-file-format"),
        s.format.label().to_owned(),
        Box::new(|s, _| {
            let i = FileFormat::ALL.iter().position(|x| *x == s.format).unwrap_or(0);
            s.format = FileFormat::ALL[(i + 1) % FileFormat::ALL.len()];
            None
        }),
    ));
    v.push((
        aurora_i18n::tr!("photo-quality"),
        yes_no(s.quality),
        Box::new(|s, _| {
            s.quality = !s.quality;
            None
        }),
    ));
    v.push((
        aurora_i18n::tr!("photo-huds"),
        on_off(s.huds, aurora_i18n::tr!("photo-vf-visible"), aurora_i18n::tr!("photo-vf-hidden")).to_owned(),
        Box::new(|s, _| {
            s.huds = !s.huds;
            None
        }),
    ));
    v.push((
        aurora_i18n::tr!("photo-section-dof"),
        yes_no(s.dof.enabled),
        Box::new(|s, _| {
            s.dof.enabled = !s.dof.enabled;
            None
        }),
    ));
    v.push((
        aurora_i18n::tr!("photo-vf-time"),
        time_label(panel.light.env.time).replace(
            aurora_i18n::tr!("photo-vf-sky-current"),
            aurora_i18n::tr!("photo-light-time-current"),
        ),
        Box::new(|_, panel| {
            panel.light.env.set_time(next_time(panel.light.env.time));
            None
        }),
    ));
    v.push((
        aurora_i18n::tr!("photo-vf-clouds"),
        on_off(
            panel.light.clouds_paused,
            aurora_i18n::tr!("photo-vf-clouds-still"),
            aurora_i18n::tr!("photo-vf-clouds-moving"),
        )
        .to_owned(),
        Box::new(|_, panel| {
            panel.light.clouds_paused = !panel.light.clouds_paused;
            None
        }),
    ));
    v.push((
        aurora_i18n::tr!("photo-light-attached"),
        on_off(
            panel.light.attached_lights,
            aurora_i18n::tr!("photo-vf-lights-on"),
            aurora_i18n::tr!("photo-vf-lights-out"),
        )
        .to_owned(),
        Box::new(|_, panel| {
            panel.light.attached_lights = !panel.light.attached_lights;
            None
        }),
    ));
    v.push((
        aurora_i18n::tr!("photo-vf-guide"),
        guide_label(s.guide).to_owned(),
        Box::new(|s, _| {
            s.guide = next_guide(s.guide);
            None
        }),
    ));
    v.push((
        aurora_i18n::tr!("photo-vf-peaking"),
        peaking_label(s.viewfinder.peaking).to_owned(),
        Box::new(|s, _| {
            let i = Peaking::ALL.iter().position(|k| *k == s.viewfinder.peaking).unwrap_or(0);
            s.viewfinder.peaking = Peaking::ALL[(i + 1) % Peaking::ALL.len()];
            None
        }),
    ));
    let sounds = Sounds::of(s.viewfinder.beep, s.sound);
    v.push((
        aurora_i18n::tr!("photo-vf-sounds"),
        sounds_label(sounds).to_owned(),
        Box::new(move |s, _| {
            let i = Sounds::ALL.iter().position(|k| *k == sounds).unwrap_or(0);
            let (beep, shutter) = Sounds::ALL[(i + 1) % Sounds::ALL.len()].split();
            s.viewfinder.beep = beep;
            s.sound = shutter;
            None
        }),
    ));
    v.push((
        aurora_i18n::tr!("photo-free-camera"),
        yes_no(panel.free.on),
        Box::new(|_, panel| {
            panel.free.on = !panel.free.on;
            None
        }),
    ));
    v.push((
        aurora_i18n::tr!("photo-freeze"),
        yes_no(panel.freeze),
        Box::new(|_, panel| {
            panel.freeze = !panel.freeze;
            Some(Out::Panel(Action::Freeze(panel.freeze)))
        }),
    ));
    v.push((
        aurora_i18n::tr!("photo-freeze-world"),
        yes_no(panel.freeze_world),
        Box::new(|_, panel| {
            panel.freeze_world = !panel.freeze_world;
            None
        }),
    ));
    v.push((
        aurora_i18n::tr!("photo-vf-hints"),
        on_off(
            s.viewfinder.hints,
            aurora_i18n::tr!("photo-vf-shown"),
            aurora_i18n::tr!("photo-vf-hidden-one"),
        )
        .to_owned(),
        Box::new(|s, _| {
            s.viewfinder.hints = !s.viewfinder.hints;
            None
        }),
    ));
    let timer = s.viewfinder.timer;
    v.push((
        aurora_i18n::tr!("photo-vf-timer"),
        if timer == 0 {
            aurora_i18n::tr!("photo-vf-timer-off").to_owned()
        } else {
            format!("{timer} s")
        },
        Box::new(move |s, _| {
            let i = vf::TIMERS.iter().position(|t| *t == timer).unwrap_or(0);
            s.viewfinder.timer = vf::TIMERS[(i + 1) % vf::TIMERS.len()];
            None
        }),
    ));
    let first = f.first_person;
    if !f.first_person_allowed && !first {
        return v;
    }
    v.push((
        aurora_i18n::tr!("photo-vf-view"),
        on_off(
            first,
            aurora_i18n::tr!("photo-vf-first-person"),
            aurora_i18n::tr!("photo-vf-third-person"),
        )
        .to_owned(),
        Box::new(move |_, _| Some(Out::FirstPerson(!first))),
    ));
    v
}
