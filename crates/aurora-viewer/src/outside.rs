//! The interface's windows outside the viewer's window: native windows of
//! their own, on any screen, drawn by the viewer's renderer
//! (aurora-render's outside.rs). Not in the Legacy Viewer, whose windows
//! all stay inside its own.
//!
//! egui calls such a window a viewport. Its pass runs inside the viewer's
//! own (`Context::show_viewport_immediate`), where the code that draws the
//! window is: that code is the same inside and outside, and borrows what
//! it borrows today. egui hands the pass to one function per thread
//! (`Context::set_immediate_viewport_renderer`), which gives it the native
//! window's size and scale and keeps what it drew; [`Outside::hand`] then
//! gives the renderer the windows whose picture changed. A window that
//! shows the same thing costs its pass and a comparison, no drawing.
//!
//! Each window reads its own events of the system, as the viewer's does
//! (`egui_winit::State`): the pointer, the wheel, the keys, the text. A
//! key that no field or menu of the window reads goes on to the viewer's
//! own handling (shortcuts, movement), as under a window inside; so do
//! the releases, the modifiers and the focus that goes, which let go of
//! what was held. What a window copies leaves with the viewer's own
//! output: one clipboard, and the same rules on what may be copied.
//! [`Outside::focused`]: the viewer is in front while any of its windows
//! has the focus (`App::in_front`).
//!
//! A window of the interface goes outside when its title bar's button asks
//! for it (`widgets::Floater::goes_outside`: every window whose title bar
//! the window outside draws, but the Dev Toolbox; none goes by itself).
//! The wish is kept with the interface's layout
//! ([`wanted`]); the window draws its body in its window outside once
//! there is one ([`native`]), asks for it otherwise ([`ask`]), and says at
//! each pass that it drew there ([`seen`]). [`Outside::reconcile`], before
//! the next pass, opens what was asked and lets go of the windows nothing
//! drew in: closed, hidden with the interface, or back inside.
//!
//! egui counts the frames and the passes of each viewport apart. What the
//! interface stamps with them is read across windows (a drop target and
//! the drag that looks for it, a menu and the window that moved), so the
//! stamps are those of the viewer's own window everywhere:
//! `cumulative_frame_nr_for(ViewportId::ROOT)` and its pass twin, which a
//! test of this module asks of every source. A pass outside runs once
//! inside each pass of the viewer's window, and so shares its numbers.
//!
//! The native windows are made and let go between passes
//! ([`Outside::open`], [`Outside::close`]), with the event loop the frame
//! has. A closed window stays until the render thread has destroyed its
//! surface, and all of them go at exit while the event loop still lives
//! (`App::exiting`: on Wayland an object destroyed after the connection is
//! a crash).

pub(crate) mod activation;
pub(crate) mod carry;
pub(crate) mod demo;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use crate::theme::Palette;
use aurora_render::{EguiHandOver, OutsideEvent, OutsideFrame, Renderer};
use egui::epaint::ClippedShape;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowAttributes, WindowId};

/// What a window outside asks with an event of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Asked {
    Nothing,
    /// The system's close button.
    Close(egui::ViewportId),
    /// The viewer's own handling takes the event too, without its
    /// interface: a key the window does not read, a release, the
    /// modifiers, the focus that goes.
    Viewer,
}

/// Whether a key of a window outside goes on to the viewer's own handling:
/// a press only when no field or menu of the window reads the keyboard, a
/// release always (what the press held is let go, wherever it was read).
fn key_goes_on(pressed: bool, reads_keys: bool) -> bool {
    !pressed || !reads_keys
}

/// The viewer is in front: its own window or one of its windows outside
/// has the focus. The frame rate, the textures and the pause of a viewer
/// behind follow this, not the focus of the viewer's own window alone.
pub(crate) fn in_front(own_window: bool, window_outside: bool) -> bool {
    own_window || window_outside
}

/// A window of the interface asks for its window outside.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Ask {
    /// The window's id in the interface.
    pub window: egui::Id,
    pub title: String,
    /// Where it is in the viewer's window, in points: its window outside
    /// opens there, where the system lets a program place its windows.
    pub rect: egui::Rect,
    pub min_size: egui::Vec2,
    /// Its size is its content's (`resize` with `held`): the user does not
    /// resize it.
    pub content_sized: bool,
}

/// The viewport of the window `window` of the interface.
pub(crate) fn viewport(window: egui::Id) -> egui::ViewportId {
    egui::ViewportId(window)
}

fn asks_id() -> egui::Id {
    egui::Id::unique("aurora outside asks")
}

fn seen_id() -> egui::Id {
    egui::Id::unique("aurora outside seen")
}

fn kept_id() -> egui::Id {
    egui::Id::unique("aurora outside kept")
}

fn untold_id() -> egui::Id {
    egui::Id::unique("aurora outside untold")
}

fn notice_id() -> egui::Id {
    egui::Id::unique("aurora outside notice")
}

/// Why the interface's windows are not drawn at a pass (`keep`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kept {
    /// The full-frame photo view.
    Photo,
    /// The Director's playback.
    Director,
    /// "UI off".
    InterfaceOff,
}

impl Kept {
    /// What the notice over a window left as it is says.
    fn text(self) -> String {
        match self {
            Kept::Photo => aurora_i18n::tr!("common-window-paused-photo"),
            Kept::Director => aurora_i18n::tr!("common-window-paused-director"),
            Kept::InterfaceOff => aurora_i18n::tr!("common-window-paused-interface"),
        }
        .to_string()
    }
}

/// The interface's windows are not drawn at this pass (the full-frame
/// photo view, the Director's playback, "UI off"): those outside stay as
/// they are, with the picture they have under a notice that says why
/// (`notice`), until they are drawn again. They take no click and no key
/// meanwhile; the cross of one closes it (`reconcile`). A window this pass drew in
/// (Phototools over the photo view) is none of them. The notice is drawn
/// once, and again when the system resizes or uncovers the window
/// (`untold`): a window left as it is costs nothing at the other frames.
pub(crate) fn keep(ctx: &egui::Context, p: &Palette, why: Kept) {
    let (untold, seen) = ctx.data_mut(|d| {
        d.insert_temp(kept_id(), true);
        (
            d.remove_temp::<Vec<egui::Id>>(untold_id()).unwrap_or_default(),
            d.get_temp::<Vec<egui::Id>>(seen_id()).unwrap_or_default(),
        )
    });
    for window in untold.into_iter().filter(|window| !seen.contains(window)) {
        ctx.show_viewport_immediate(viewport(window), egui::ViewportBuilder::default(), |ui, _| {
            ui.ctx().data_mut(|d| d.insert_temp(notice_id(), true));
            notice(ui, p, &why.text());
        });
    }
}

/// The notice of a window left as it is (`keep`), alone in its pass: a
/// shade over the picture the window has (`under_notice`), and why at its
/// middle.
fn notice(ui: &egui::Ui, p: &Palette, text: &str) {
    const ICON: f32 = 14.0;
    const GAP: f32 = 8.0;
    const PAD: egui::Vec2 = egui::vec2(12.0, 8.0);
    let room = ui.max_rect();
    let painter = ui.painter();
    painter.rect_filled(room, 0.0, p.panel.gamma_multiply(0.6));
    let wrap = (room.width() - ICON - GAP - 4.0 * PAD.x).max(0.0);
    let text = painter.layout(text.to_owned(), egui::FontId::proportional(13.0), p.ink, wrap);
    let size = egui::vec2(ICON + GAP + text.size().x, text.size().y.max(ICON)) + 2.0 * PAD;
    let card = egui::Rect::from_center_size(room.center(), size);
    let radius = ui.visuals().window_corner_radius;
    painter.rect_filled(card, radius, p.panel);
    painter.rect_stroke(card, radius, egui::Stroke::new(1.0, p.violet_light), egui::StrokeKind::Inside);
    let inner = card.shrink2(PAD);
    if let Some(icon) = crate::ui::icons::global("player-pause-filled") {
        painter.image(
            icon.id(),
            egui::Rect::from_center_size(egui::pos2(inner.left() + ICON / 2.0, inner.center().y), egui::Vec2::splat(ICON)),
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            p.ink,
        );
    }
    let at = egui::pos2(inner.left() + ICON + GAP, inner.center().y - text.size().y / 2.0);
    painter.galley(at, text, p.ink);
}

/// The windows outside are kept as they are at this pass (`keep`).
fn kept(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(kept_id())).unwrap_or(false)
}

/// The windows drawn in at this pass.
#[cfg(test)]
pub(crate) fn seen_now(ctx: &egui::Context) -> Vec<egui::Id> {
    ctx.data(|d| d.get_temp(seen_id())).unwrap_or_default()
}

/// The user sent the window `window` outside: kept with the interface's
/// layout, like its place inside.
pub(crate) fn wanted(ctx: &egui::Context, window: egui::Id) -> bool {
    ctx.data_mut(|d| d.get_persisted::<bool>(window.with("outside"))).unwrap_or(false)
}

pub(crate) fn want(ctx: &egui::Context, window: egui::Id, outside: bool) {
    ctx.data_mut(|d| d.insert_persisted(window.with("outside"), outside));
}

/// The window `window` has its window outside: it draws there.
pub(crate) fn native(ctx: &egui::Context, window: egui::Id) -> bool {
    ctx.data(|d| d.get_temp::<bool>(window.with("outside native"))).unwrap_or(false)
}

pub(crate) fn set_native(ctx: &egui::Context, window: egui::Id, native: bool) {
    ctx.data_mut(|d| {
        if native {
            d.insert_temp(window.with("outside native"), true);
        } else {
            d.remove::<bool>(window.with("outside native"));
        }
    });
}

fn front_id() -> egui::Id {
    egui::Id::unique("aurora outside front")
}

fn raises_id() -> egui::Id {
    egui::Id::unique("aurora outside raises")
}

fn sizes_id() -> egui::Id {
    egui::Id::unique("aurora outside sizes")
}

/// Asks the system to give the window outside of `window` the size `size`
/// (in the interface's points), at the next frame: what sizes a window
/// inside by its name (`widgets::size_floater`), and the size of its
/// content for a window that follows it (`held`: the user cannot change
/// it). The last size asked wins; a size larger than the screen is brought
/// down to it (`fitting`).
pub(crate) fn resize(ctx: &egui::Context, window: egui::Id, size: egui::Vec2, held: bool) {
    ctx.data_mut(|d| {
        let sizes = d.get_temp_mut_or_default::<Vec<Resize>>(sizes_id());
        sizes.retain(|(asked, ..)| *asked != window);
        sizes.push((window, size, held));
    });
}

/// A size asked of a window outside: the window, the size, held or not.
type Resize = (egui::Id, egui::Vec2, bool);

/// The share of the screen's height a window sized by its content takes at
/// most: the system's bars take the rest, and no system tells their size
/// through winit.
const SCREEN_SHARE: f32 = 0.9;

/// `size` brought down to the screen the window is on, `room` in the
/// interface's points: its content scrolls in what is left.
fn fitting(size: egui::Vec2, room: Option<egui::Vec2>) -> egui::Vec2 {
    room.map_or(size, |room| egui::vec2(size.x.min(room.x), size.y.min(room.y * SCREEN_SHARE)))
}

/// The size `size` (in the interface's points) to ask of the system at the
/// zoom `zoom`: in whole units, and never under the size. A system counts
/// a window in whole units of its own (Wayland) or in whole pixels, and a
/// window given a fraction less than what it shows would scroll it.
fn whole(size: egui::Vec2, zoom: f32) -> winit::dpi::LogicalSize<f32> {
    // (a product a hair over a whole number is that number)
    let up = |points: f32| (points * zoom - 0.01).ceil();
    winit::dpi::LogicalSize::new(up(size.x), up(size.y))
}

/// The window `window` is in its window outside and that one has the
/// system's focus: it is the front window, and the keyboard is its own.
pub(crate) fn in_focus(ctx: &egui::Context, window: egui::Id) -> bool {
    ctx.data(|d| d.get_temp::<egui::Id>(front_id())) == Some(window)
}

/// The window of the viewer that has the system's focus: one of its
/// windows outside, its own, or none when another program has it.
pub(crate) fn focused_window(ctx: &egui::Context) -> Option<egui::ViewportId> {
    ctx.data(|d| d.get_temp::<egui::Id>(front_id())).map(viewport).or_else(|| {
        ctx.input_for(egui::ViewportId::ROOT, |i| i.focused)
            .then_some(egui::ViewportId::ROOT)
    })
}

/// The system's focus went from `was` to `now`, a window of the viewer
/// that is not `mine`: what held the keyboard in `mine` lets go of it, as
/// at a click elsewhere in its own window, which the pointer of `mine`
/// never shows. Another program that takes the focus (`now` None) changes
/// nothing, as with a single window.
pub(crate) fn focus_went_elsewhere(mine: egui::ViewportId, was: Option<egui::ViewportId>, now: Option<egui::ViewportId>) -> bool {
    now != was && now.is_some_and(|now| now != mine)
}

fn left_id() -> egui::Id {
    egui::Id::unique("aurora outside left")
}

/// A window outside that lost the focus this long ago at most lost it to
/// what asks about it now: the click on its toolbar button, in the
/// viewer's window, takes the focus on its way.
const LEFT_FOR: f64 = 1.0;

/// The window `window` is in its window outside and in front, as far as
/// the viewer can tell: it has the system's focus, or had it until a
/// moment ago ([`LEFT_FOR`]). No system tells a program what covers its
/// windows: one that the focus left longer ago may be behind anything.
pub(crate) fn shows_in_front(ctx: &egui::Context, window: egui::Id) -> bool {
    in_focus(ctx, window)
        || ctx
            .data(|d| d.get_temp::<(egui::Id, f64)>(left_id()))
            .is_some_and(|(left, at)| left == window && ctx.input(|i| i.time) - at <= LEFT_FOR)
}

/// The window outside with the focus at this frame, for [`in_focus`], and
/// the one that just lost it with the time, for [`shows_in_front`].
fn publish_front(ctx: &egui::Context, front: Option<egui::Id>) {
    let now = ctx.input(|i| i.time);
    ctx.data_mut(|d| {
        if let Some(was) = d.get_temp::<egui::Id>(front_id())
            && Some(was) != front
        {
            d.insert_temp(left_id(), (was, now));
        }
        match front {
            Some(front) => {
                d.insert_temp(front_id(), front);
            }
            None => {
                d.remove::<egui::Id>(front_id());
            }
        }
    });
}

/// The window outside `front` takes the system's focus, as the host
/// says at a frame.
#[cfg(test)]
pub(crate) fn focus_for_test(ctx: &egui::Context, front: Option<egui::Id>) {
    publish_front(ctx, front);
}

/// Asks the system to bring the window outside of `window` in front, at
/// the next frame: what brings a window inside to the top (a click on a
/// notification, its toolbar button). The system may only signal it where
/// it lets no program take the focus by itself (Wayland).
pub(crate) fn raise(ctx: &egui::Context, window: egui::Id) {
    ctx.data_mut(|d| {
        let raises = d.get_temp_mut_or_default::<Vec<egui::Id>>(raises_id());
        if !raises.contains(&window) {
            raises.push(window);
        }
    });
}

/// The windows asked in front and not raised yet.
#[cfg(test)]
pub(crate) fn raises(ctx: &egui::Context) -> Vec<egui::Id> {
    ctx.data(|d| d.get_temp(raises_id())).unwrap_or_default()
}

/// The window outside with the focus, as the host tells it each frame.
#[cfg(test)]
pub(crate) fn set_front(ctx: &egui::Context, front: Option<egui::Id>) {
    publish_front(ctx, front);
}

/// The sizes asked and not given yet.
#[cfg(test)]
pub(crate) fn resizes(ctx: &egui::Context) -> Vec<Resize> {
    ctx.data(|d| d.get_temp(sizes_id())).unwrap_or_default()
}

/// The sizes asked, taken as the windows' host takes them.
#[cfg(test)]
pub(crate) fn take_resizes(ctx: &egui::Context) -> Vec<Resize> {
    ctx.data_mut(|d| d.remove_temp(sizes_id())).unwrap_or_default()
}

/// Asks for a window outside, opened before the next pass.
pub(crate) fn ask(ctx: &egui::Context, ask: Ask) {
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Vec<Ask>>(asks_id()).push(ask));
}

/// The window `window` drew in its window outside at this pass.
pub(crate) fn seen(ctx: &egui::Context, window: egui::Id) {
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Vec<egui::Id>>(seen_id()).push(window));
}

/// A window of a Wayland session, where no program takes the focus by
/// itself.
fn on_wayland(window: &Window) -> bool {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    window
        .window_handle()
        .is_ok_and(|handle| matches!(handle.as_raw(), RawWindowHandle::Wayland(_)))
}

/// Whether a window of the interface nothing drew in at the last pass is
/// left as it is, the interface's windows not being drawn at all there
/// (`kept`): not one that the pass before, without them too
/// (`kept_before`), drew in (`drawn_before`). Phototools closed over the
/// photo view goes as any window nothing draws in.
fn left_as_it_is(kept: bool, drawn_before: bool, kept_before: bool) -> bool {
    kept && !(drawn_before && kept_before)
}

/// Whether a window left as it is has its notice to draw: none yet at its
/// size, or the system asked for its picture again. Never a minimized one.
fn untold(told: Option<(u32, u32)>, size: (u32, u32), redraw: bool) -> bool {
    size.0 > 0 && size.1 > 0 && (redraw || told != Some(size))
}

/// The picture of a window left as it is: the one it has, under `notice`
/// in place of the `earlier` shapes of the notice it had. Returns how many
/// shapes the notice is.
fn under_notice(shown: &mut Vec<ClippedShape>, earlier: usize, notice: Vec<ClippedShape>) -> usize {
    shown.truncate(shown.len().saturating_sub(earlier));
    let count = notice.len();
    shown.extend(notice);
    count
}

/// The windows of the interface that none drew in at the last pass: let
/// go of, unless the interface's windows were not drawn at all (`keep`).
fn unseen(open: impl Iterator<Item = (egui::ViewportId, bool)>, seen: &[egui::Id]) -> Vec<egui::ViewportId> {
    open.filter(|(id, of_interface)| *of_interface && !seen.contains(&id.0))
        .map(|(id, _)| id)
        .collect()
}

/// Frames a window whose renderer was replaced has to ask for its new
/// window: its next pass asks, the frame after opens it. One closed or
/// hidden meanwhile is a new window when it goes outside again, at a new
/// place and with the focus.
const ASKS_WITHIN: u8 = 3;

/// A frame went by for a window that waits for the one that replaces it:
/// whether it still waits.
fn still_waited(frames_left: &mut u8) -> bool {
    *frames_left = frames_left.saturating_sub(1);
    *frames_left > 0
}

/// What a window outside was when the renderer was replaced, for the one
/// that takes its place.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Was {
    /// Where it was on the screen, where the system says so (not Wayland).
    pub at: Option<winit::dpi::PhysicalPosition<i32>>,
    pub focused: bool,
}

pub(crate) struct Outside {
    /// Shared with egui's function for the passes of this thread.
    shared: Rc<RefCell<Shared>>,
    /// The windows of the interface a replaced renderer left without a
    /// surface (`renderer_replaced`), until each asks for its new window,
    /// which it does within `ASKS_WITHIN` frames or not at all.
    replaced: std::collections::HashMap<egui::ViewportId, (Was, u8)>,
    /// Closed windows whose surface the render thread still has, by key.
    closing: Vec<(u64, Arc<Window>)>,
    /// Brings a window in front where winit cannot (Wayland).
    activation: activation::Activation,
    /// Keys given so far: one names a window for the renderer.
    keys: u64,
    /// The pass before the last left the windows as they are (`keep`).
    kept: bool,
    /// Windows of the interface closed by their cross while left as they
    /// are: gone at once, their close waits for their own code.
    closed: Vec<egui::Id>,
}

#[derive(Default)]
struct Shared {
    windows: HashMap<egui::ViewportId, Native>,
    /// The texture changes of this frame's passes outside, older than the
    /// viewer's own pass's: they leave with its interface, before them.
    textures: egui::TexturesDelta,
    /// What this frame's passes outside ask of the system besides their
    /// own window (a text to copy, a link to open): the viewer's output
    /// takes them.
    commands: Vec<egui::OutputCommand>,
    /// What this frame's passes outside read that the viewer follows in
    /// every window (`heard`).
    heard: Vec<egui::Event>,
}

struct Native {
    key: u64,
    window: Arc<Window>,
    /// A window of the interface (`Ask`), let go of when nothing draws in
    /// it; the others are closed by who opened them.
    of_interface: bool,
    /// The title the system shows.
    title: Option<String>,
    /// The window's events of the system, as egui reads them.
    input: egui_winit::State,
    focused: bool,
    /// A field or a menu of the window read the keyboard at its last pass.
    reads_keys: bool,
    /// What its pass left this frame.
    pass: Option<Pass>,
    /// What its pass asks of its own window: the cursor, the text input.
    output: Option<egui::PlatformOutput>,
    /// What was handed to the renderer last, at which size and scale.
    shown: Vec<ClippedShape>,
    shown_size: (u32, u32),
    shown_scale: f32,
    /// How many shapes at the end of `shown` are the notice of a window
    /// left as it is (`keep`), and the size the notice was drawn for.
    notice: usize,
    told: Option<(u32, u32)>,
    /// A window of the interface drew in it at the pass before the last.
    drawn: bool,
    /// Draw it again whatever it shows (a new window, one the system
    /// uncovered or resized).
    redraw: bool,
    /// The system asked for its first picture. Until then nothing is
    /// drawn in it: where the program draws the title bar itself (Wayland),
    /// the window library draws it just before it asks, and a picture
    /// shown earlier has the system count the window without its title
    /// bar, then shrink it by that height.
    asked_for: bool,
}

struct Pass {
    shapes: Vec<ClippedShape>,
    pixels_per_point: f32,
    /// The notice of a window left as it is, not the window's own picture.
    notice: bool,
}

/// What the pass of a window outside starts from: the events `raw` of the
/// window, on a screen of its size at its scale, on the viewer's clock.
fn pass_input(
    mut raw: egui::RawInput,
    ids: egui::ViewportIdPair,
    size: (u32, u32),
    scale: f32,
    zoom: f32,
    focused: bool,
    (time, predicted_dt, max_texture_side): (f64, f32, usize),
) -> egui::RawInput {
    // (the keys read as in the viewer's own window: `App::render`)
    crate::keybinds::adopt_mac_control_keys(&mut raw.events, cfg!(target_os = "macos"));
    crate::keybinds::keep_select_all_exact(&mut raw.events);
    let points = egui::vec2(size.0 as f32, size.1 as f32) / (scale * zoom).max(0.01);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, points);
    let info = egui::ViewportInfo {
        parent: Some(ids.parent),
        native_pixels_per_point: Some(scale),
        inner_rect: Some(screen),
        focused: Some(focused),
        ..Default::default()
    };
    egui::RawInput {
        viewport_id: ids.this,
        viewports: std::iter::once((ids.this, info)).collect(),
        screen_rect: Some(screen),
        max_texture_side: Some(max_texture_side),
        time: Some(time),
        predicted_dt,
        focused,
        ..raw
    }
}

/// What the viewer follows in every window, before the fields read it: an
/// Escape and a paste, which end the Inventory's cut
/// (`InventoryUi::clipboard_input`).
fn heard(events: &[egui::Event]) -> impl Iterator<Item = egui::Event> + '_ {
    events
        .iter()
        .filter(|event| {
            matches!(
                event,
                egui::Event::Key {
                    key: egui::Key::Escape,
                    pressed: true,
                    ..
                } | egui::Event::Paste(_)
            )
        })
        .cloned()
}

/// A field or a menu of the pass under way reads the keyboard.
fn reads_keys(ctx: &egui::Context) -> bool {
    ctx.egui_wants_keyboard_input() || egui::Popup::is_any_open(ctx)
}

/// Whether a window whose pass left `shapes` at `size` is drawn again;
/// never before the system asked for its first picture (`asked_for`).
fn drawn_again(
    asked_for: bool,
    redraw: bool,
    textures_changed: bool,
    size: (u32, u32),
    shown_size: (u32, u32),
    shapes: &[ClippedShape],
    shown: &[ClippedShape],
) -> bool {
    asked_for && (redraw || textures_changed || size != shown_size || !aurora_render::same_picture(shapes, shown))
}

/// Whether a window nothing drew in at this frame shows its last picture
/// again: only one left as it is (`keep`) that the system asked to draw.
fn shown_again(kept: bool, asked_for: bool, redraw: bool, shown: &[ClippedShape]) -> bool {
    kept && asked_for && redraw && !shown.is_empty()
}

/// egui's function for the passes outside: the pass of `viewport`, run at
/// once inside the pass that asked for it.
fn pass(shared: &Rc<RefCell<Shared>>, ctx: &egui::Context, viewport: egui::ImmediateViewport<'_>) {
    let egui::ImmediateViewport {
        ids,
        builder,
        mut viewport_ui_cb,
    } = viewport;
    let clock = ctx.input(|i| (i.time, i.predicted_dt, i.max_texture_side));
    let zoom = ctx.zoom_factor();
    let input = shared.borrow_mut().windows.get_mut(&ids.this).map(|native| {
        if builder.title.is_some() && builder.title != native.title {
            native.window.set_title(builder.title.as_deref().unwrap_or_default());
            native.title = builder.title;
        }
        let size = native.window.inner_size();
        pass_input(
            native.input.take_egui_input(&native.window),
            ids,
            (size.width, size.height),
            native.window.scale_factor() as f32,
            zoom,
            native.focused,
            clock,
        )
    });
    // (egui wants the window's code run whatever happens: `Outside::show`
    // only asks for a window that is open, so this is never the case)
    let input = input.unwrap_or_else(|| pass_input(egui::RawInput::default(), ids, (640, 480), 1.0, zoom, false, clock));
    shared.borrow_mut().heard.extend(heard(&input.events));
    let mut keys = false;
    // One pass, where egui may run two to lay a new content out: the
    // window's code is run once (a `Floater`'s body is `FnOnce`), and what
    // asks for a second pass settles at the next frame instead.
    let passes = ctx.options(|o| o.max_passes);
    ctx.options_mut(|o| o.max_passes = std::num::NonZeroUsize::MIN);
    // (nothing of `shared` is borrowed while the window's code runs)
    let mut out = ctx.run_ui(input, |ui| {
        viewport_ui_cb(ui);
        keys = reads_keys(ui.ctx());
    });
    ctx.options_mut(|o| o.max_passes = passes);
    let notice = ctx.data_mut(|d| d.remove_temp::<bool>(notice_id())).unwrap_or(false);
    let mut shared = shared.borrow_mut();
    shared.textures.append(out.textures_delta);
    shared.commands.append(&mut out.platform_output.commands);
    if let Some(native) = shared.windows.get_mut(&ids.this) {
        native.reads_keys = keys;
        native.output = Some(out.platform_output);
        native.pass = Some(Pass {
            shapes: out.shapes,
            pixels_per_point: out.pixels_per_point,
            notice,
        });
    }
}

impl Outside {
    /// Once, on the thread of the interface.
    pub fn new(ctx: &egui::Context) -> Outside {
        // (egui draws a viewport inside the window that asks for it until
        // it is told there are native ones)
        ctx.set_embed_viewports(false);
        let shared = Rc::new(RefCell::new(Shared::default()));
        let for_passes = shared.clone();
        egui::Context::set_immediate_viewport_renderer(move |ctx, viewport| pass(&for_passes, ctx, viewport));
        Outside {
            shared,
            replaced: Default::default(),
            activation: Default::default(),
            closing: Vec::new(),
            keys: 0,
            kept: false,
            closed: Vec::new(),
        }
    }

    pub fn is_open(&self, id: egui::ViewportId) -> bool {
        self.shared.borrow().windows.contains_key(&id)
    }

    /// The renderer was replaced (a GPU recovery, the VR runtime closed):
    /// the new one has no surface, and none can be made on a window the
    /// lost device's surface is left on, which is why the viewer's own
    /// window is replaced too. The windows outside go; those of the
    /// interface ask for new ones at their next pass, opened where these
    /// were (`reconcile`), and a scenario opens its own again.
    pub fn renderer_replaced(&mut self, ctx: &egui::Context) {
        self.closing.clear();
        let windows = std::mem::take(&mut self.shared.borrow_mut().windows);
        for (id, native) in windows {
            set_native(ctx, id.0, false);
            if native.of_interface {
                self.replaced.insert(
                    id,
                    (
                        Was {
                            at: native.window.outer_position().ok(),
                            focused: native.focused,
                        },
                        ASKS_WITHIN,
                    ),
                );
            }
        }
    }

    /// Before the interface's pass: opens the windows outside that the
    /// interface's windows asked for at the last pass, with the attributes
    /// `attributes` gives each (and what the window it replaces was, after
    /// `renderer_replaced`), and lets go of those none drew in.
    pub fn reconcile(
        &mut self,
        event_loop: &ActiveEventLoop,
        own: &Window,
        renderer: &mut Renderer,
        ctx: &egui::Context,
        attributes: impl Fn(&Ask, Option<Was>) -> WindowAttributes,
    ) {
        let (asks, seen, raises, sizes, kept) = ctx.data_mut(|d| {
            (
                d.remove_temp::<Vec<Ask>>(asks_id()).unwrap_or_default(),
                d.remove_temp::<Vec<egui::Id>>(seen_id()).unwrap_or_default(),
                d.remove_temp::<Vec<egui::Id>>(raises_id()).unwrap_or_default(),
                d.remove_temp::<Vec<Resize>>(sizes_id()).unwrap_or_default(),
                d.remove_temp::<bool>(kept_id()).unwrap_or(false),
            )
        });
        let zoom = ctx.zoom_factor();
        for (window, size, held) in sizes {
            if let Some(native) = self.shared.borrow().windows.get(&viewport(window)) {
                // A maximized or full-screen window keeps the size the
                // system gave it: Windows takes a size asked of one as the
                // end of that state (winit's `request_inner_size`).
                if native.window.is_maximized() || native.window.fullscreen().is_some() {
                    continue;
                }
                let room = native.window.current_monitor().and_then(|monitor| {
                    let room = crate::screen_info::size(&monitor)?.to_logical::<f32>(monitor.scale_factor());
                    Some(egui::vec2(room.width / zoom, room.height / zoom))
                });
                let size = whole(fitting(size, room), zoom);
                // a held size is the least and the most the system allows:
                // both are lifted for the change, then set to the new size
                if held {
                    native.window.set_min_inner_size(None::<winit::dpi::LogicalSize<f32>>);
                    native.window.set_max_inner_size(None::<winit::dpi::LogicalSize<f32>>);
                }
                // (the system answers with a resize of the window, or not at all)
                let _ = native.window.request_inner_size(size);
                if held {
                    native.window.set_min_inner_size(Some(size));
                    native.window.set_max_inner_size(Some(size));
                }
            }
        }
        // (the tokens the compositor gave for the raises of the last frames)
        for (key, token) in self.activation.pump() {
            if let Some(native) = self.shared.borrow().windows.values().find(|native| native.key == key) {
                self.activation.activate(&token, &native.window);
            }
        }
        for window in raises {
            let shared = self.shared.borrow();
            let Some(native) = shared.windows.get(&viewport(window)) else {
                continue;
            };
            if !on_wayland(&native.window) {
                native.window.set_minimized(false);
                native.window.focus_window();
                continue;
            }
            // winit's `set_minimized(false)` and `focus_window` do nothing
            // there. With the keyboard in one of the viewer's windows, the
            // compositor is asked with the proof of it and brings the
            // window in front (`activation`); without, winit asks and the
            // compositor signals the window
            let open: Vec<&Window> = std::iter::once(own)
                .chain(shared.windows.values().map(|native| native.window.as_ref()))
                .collect();
            if !self.activation.ask(native.key, &open) {
                native
                    .window
                    .request_user_attention(Some(winit::window::UserAttentionType::Informational));
            }
        }
        let front = self
            .shared
            .borrow()
            .windows
            .iter()
            .find(|(_, native)| native.of_interface && native.focused)
            .map(|(id, _)| id.0);
        publish_front(ctx, front);
        let gone = unseen(
            self.shared.borrow().windows.iter().map(|(id, native)| (*id, native.of_interface)),
            &seen,
        );
        let kept_before = std::mem::replace(&mut self.kept, kept);
        // (closed by their cross while left as they are: the close still
        // waits for their code, which runs when the windows are drawn again)
        if kept {
            for window in &self.closed {
                crate::ui::widgets::keep_close_asked(ctx, *window);
            }
        } else {
            self.closed.clear();
        }
        for id in gone {
            let drawn_before = self.shared.borrow().windows.get(&id).is_some_and(|native| native.drawn);
            if !left_as_it_is(kept, drawn_before, kept_before) {
                self.close(renderer, ctx, id);
                continue;
            }
            // left as it is (`keep`): what the system sent it is dropped,
            // not kept for the day its code runs again, and the keys go on
            // to the viewer. Its cross closes it at once; its code, which
            // alone says the window is closed, hears of it when it runs
            // again
            if crate::ui::widgets::keep_close_asked(ctx, id.0) {
                self.close(renderer, ctx, id);
                self.closed.push(id.0);
                continue;
            }
            if let Some(native) = self.shared.borrow_mut().windows.get_mut(&id) {
                let _ = native.input.take_egui_input(&native.window);
                native.reads_keys = false;
            }
        }
        let untold = {
            let mut shared = self.shared.borrow_mut();
            for (id, native) in &mut shared.windows {
                native.drawn = seen.contains(&id.0);
            }
            shared
                .windows
                .iter()
                .filter(|(_, native)| {
                    let size = native.window.inner_size();
                    native.of_interface && untold(native.told, (size.width, size.height), native.redraw)
                })
                .map(|(id, _)| id.0)
                .collect::<Vec<_>>()
        };
        ctx.data_mut(|d| d.insert_temp(untold_id(), untold));
        for ask in asks {
            let id = viewport(ask.window);
            if self.is_open(id) {
                continue;
            }
            let was = self.replaced.remove(&id).map(|(was, _)| was);
            match self.open(event_loop, renderer, ctx, id, attributes(&ask, was), false) {
                Ok(()) => {
                    if let Some(native) = self.shared.borrow_mut().windows.get_mut(&id) {
                        native.of_interface = true;
                        native.title = Some(ask.title);
                    }
                    set_native(ctx, ask.window, true);
                }
                // the window stays inside the viewer's
                Err(e) => {
                    log::warn!("window outside: \"{}\" could not be opened: {e}", ask.title);
                    want(ctx, ask.window, false);
                }
            }
        }
        self.replaced.retain(|_, (_, frames_left)| still_waited(frames_left));
    }

    /// A native window for `id`, drawn by `renderer` from the next frame
    /// on. `sync` synchronizes it with its screen: only to measure
    /// (`Renderer::open_outside`).
    pub fn open(
        &mut self,
        event_loop: &ActiveEventLoop,
        renderer: &mut Renderer,
        ctx: &egui::Context,
        id: egui::ViewportId,
        attributes: WindowAttributes,
        sync: bool,
    ) -> Result<(), String> {
        if self.is_open(id) {
            return Ok(());
        }
        let window = Arc::new(event_loop.create_window(attributes).map_err(|e| e.to_string())?);
        // (from the first window outside on: the keyboard's focus is known
        // by the time one is to come in front)
        self.activation.start(event_loop);
        crate::window_hold::follow(&window);
        // (its first picture waits for the system to ask for one)
        window.request_redraw();
        let size = window.inner_size();
        self.keys += 1;
        let key = self.keys;
        renderer
            .open_outside(key, window.clone(), size.width, size.height, sync)
            .map_err(|e| e.to_string())?;
        let input = egui_winit::State::new(
            ctx.clone(),
            id,
            &window,
            Some(window.scale_factor() as f32),
            None,
            Some(renderer.device.limits().max_texture_dimension_2d as usize),
        );
        let focused = window.has_focus();
        self.shared.borrow_mut().windows.insert(
            id,
            Native {
                key,
                window,
                of_interface: false,
                title: None,
                input,
                focused,
                reads_keys: false,
                pass: None,
                output: None,
                shown: Vec::new(),
                shown_size: (0, 0),
                shown_scale: 1.0,
                notice: 0,
                told: None,
                drawn: false,
                redraw: true,
                asked_for: false,
            },
        );
        Ok(())
    }

    /// The window of `id` goes: hidden at once, gone when the render
    /// thread has destroyed its surface.
    pub fn close(&mut self, renderer: &mut Renderer, ctx: &egui::Context, id: egui::ViewportId) {
        let Some(native) = self.shared.borrow_mut().windows.remove(&id) else {
            return;
        };
        set_native(ctx, id.0, false);
        native.window.set_visible(false);
        renderer.close_outside(native.key);
        self.closing.push((native.key, native.window));
    }

    /// What the render thread answered since the last frame; returns the
    /// windows it cannot draw, which are closed: one of the interface goes
    /// back inside the viewer's.
    pub fn answers(&mut self, renderer: &mut Renderer, ctx: &egui::Context) -> Vec<egui::ViewportId> {
        let mut failed = Vec::new();
        for event in renderer.outside_events() {
            match event {
                OutsideEvent::Closed(key) => self.closing.retain(|(k, _)| *k != key),
                // (its surface is destroyed already)
                OutsideEvent::Failed(key, _) => {
                    let mut shared = self.shared.borrow_mut();
                    let id = shared.windows.iter().find(|(_, native)| native.key == key).map(|(id, _)| *id);
                    if let Some(id) = id
                        && let Some(native) = shared.windows.remove(&id)
                    {
                        set_native(ctx, id.0, false);
                        if native.of_interface {
                            want(ctx, id.0, false);
                        }
                        failed.push(id);
                    }
                }
            }
        }
        failed
    }

    /// Draws `add` in the window outside of `id`, if it has one: its pass,
    /// inside the pass under way.
    pub fn show<R>(&self, ctx: &egui::Context, id: egui::ViewportId, mut add: impl FnMut(&mut egui::Ui) -> R) -> Option<R> {
        if !self.is_open(id) {
            return None;
        }
        Some(ctx.show_viewport_immediate(id, egui::ViewportBuilder::default(), |ui, _| add(ui)))
    }

    /// The inner size of the window outside `id`, in pixels.
    pub fn size_of(&self, id: WindowId) -> Option<winit::dpi::PhysicalSize<u32>> {
        self.shared
            .borrow()
            .windows
            .values()
            .find(|native| native.window.id() == id)
            .map(|native| native.window.inner_size())
    }

    /// A field or a menu of the window outside `id` reads the keyboard, as
    /// its last pass said.
    pub fn reads_keys(&self, id: WindowId) -> bool {
        self.shared
            .borrow()
            .windows
            .values()
            .any(|native| native.window.id() == id && native.reads_keys)
    }

    /// Text for the window outside `id`, read by its next pass as typed in
    /// it: the character of an Alt code (alt_codes.rs).
    pub fn text(&mut self, id: WindowId, text: String) {
        let mut shared = self.shared.borrow_mut();
        if let Some(native) = shared.windows.values_mut().find(|native| native.window.id() == id) {
            native.input.egui_input_mut().events.push(egui::Event::Text(text));
            native.redraw = true;
        }
    }

    /// Where the windows outside are on the screen, `front` telling when
    /// each last took the focus: none where the system does not tell.
    pub fn placed(&self, front: impl Fn(WindowId) -> u64) -> Vec<carry::Placed> {
        self.shared
            .borrow()
            .windows
            .values()
            .filter_map(|native| carry::Placed::of(&native.window, front(native.window.id())))
            .collect()
    }

    /// What the window outside `id` reads at its next pass of a drag held
    /// in another window over it (outside/carry.rs).
    pub fn carried(&mut self, id: WindowId, events: Vec<egui::Event>) {
        let mut shared = self.shared.borrow_mut();
        if let Some(native) = shared.windows.values_mut().find(|native| native.window.id() == id) {
            native.input.egui_input_mut().events.extend(events);
            native.redraw = true;
        }
    }

    /// The scale of the window outside `id`.
    pub fn scale_of(&self, id: WindowId) -> Option<f64> {
        self.shared
            .borrow()
            .windows
            .values()
            .find(|native| native.window.id() == id)
            .map(|native| native.window.scale_factor())
    }

    /// The window outside `id` has the focus.
    pub fn has_focus(&self, id: WindowId) -> bool {
        self.shared
            .borrow()
            .windows
            .values()
            .any(|native| native.window.id() == id && native.focused)
    }

    /// One of the windows outside has the focus.
    pub fn focused(&self) -> bool {
        self.shared.borrow().windows.values().any(|native| native.focused)
    }

    /// An event of the system: `None` when `id` is not a window outside
    /// (the viewer's own), else what the window asks. The window's pass
    /// reads the event at the next frame.
    pub fn window_event(&mut self, id: WindowId, event: &WindowEvent) -> Option<Asked> {
        if self.closing.iter().any(|(_, window)| window.id() == id) {
            return Some(Asked::Nothing);
        }
        let mut shared = self.shared.borrow_mut();
        let (viewport, native) = shared.windows.iter_mut().find(|(_, native)| native.window.id() == id)?;
        // (what egui answers is about the viewer's own window between two
        // frames: `reads_keys`, from the window's last pass, is its own)
        let _ = native.input.on_window_event(&native.window, event);
        if matches!(event, WindowEvent::RedrawRequested) {
            native.asked_for = true;
        }
        Some(match event {
            WindowEvent::CloseRequested => Asked::Close(*viewport),
            WindowEvent::Resized(_)
            | WindowEvent::ScaleFactorChanged { .. }
            | WindowEvent::RedrawRequested
            | WindowEvent::Occluded(false) => {
                native.redraw = true;
                Asked::Nothing
            }
            WindowEvent::Focused(focused) => {
                native.focused = *focused;
                if *focused { Asked::Nothing } else { Asked::Viewer }
            }
            WindowEvent::ModifiersChanged(_) => Asked::Viewer,
            WindowEvent::KeyboardInput { event: key, .. }
                if key_goes_on(key.state == winit::event::ElementState::Pressed, native.reads_keys) =>
            {
                Asked::Viewer
            }
            _ => Asked::Nothing,
        })
    }

    /// The window of `id` and where the point `at` of its pass is in it,
    /// in pixels: for a scripted pointer.
    pub fn pixel(&self, id: egui::ViewportId, at: egui::Pos2, zoom: f32) -> Option<(WindowId, winit::dpi::PhysicalPosition<f64>)> {
        let shared = self.shared.borrow();
        let native = shared.windows.get(&id)?;
        let scale = native.window.scale_factor() * f64::from(zoom);
        Some((
            native.window.id(),
            winit::dpi::PhysicalPosition::new(f64::from(at.x) * scale, f64::from(at.y) * scale),
        ))
    }

    /// What the last frame's passes outside read that the viewer follows in
    /// every window: an Escape, a paste.
    pub fn take_heard(&mut self) -> Vec<egui::Event> {
        std::mem::take(&mut self.shared.borrow_mut().heard)
    }

    /// What this frame's passes outside ask of the system besides their
    /// own window, to put before what the viewer's own pass asks.
    pub fn take_commands(&mut self) -> Vec<egui::OutputCommand> {
        std::mem::take(&mut self.shared.borrow_mut().commands)
    }

    /// After the viewer's pass: each window outside takes what its pass
    /// asks of it (the cursor's shape, the text input's place).
    pub fn platform(&mut self, event_loop: &ActiveEventLoop) {
        for native in self.shared.borrow_mut().windows.values_mut() {
            if let Some(output) = native.output.take() {
                native
                    .input
                    .handle_platform_output_with_event_loop(&native.window, event_loop, output);
            }
        }
    }

    /// What each window outside drew at this frame's pass, before it is
    /// handed to the renderer (`hand`).
    pub fn passes(&self, mut each: impl FnMut(&[ClippedShape])) {
        for native in self.shared.borrow().windows.values() {
            if let Some(pass) = &native.pass {
                each(&pass.shapes);
            }
        }
    }

    /// The texture changes of this frame's passes outside, to put before
    /// those of the viewer's own pass.
    pub fn take_textures(&mut self) -> egui::TexturesDelta {
        std::mem::take(&mut self.shared.borrow_mut().textures)
    }

    /// After the viewer's pass: hands the renderer the windows outside
    /// whose picture changed. `textures_changed`: a texture of the
    /// interface was written this frame (a picture that arrived shows in
    /// the same shapes).
    pub fn hand(
        &mut self,
        ctx: &egui::Context,
        hand_over: &mut EguiHandOver,
        renderer: &mut Renderer,
        clear: egui::Color32,
        textures_changed: bool,
    ) {
        let kept = kept(ctx);
        for native in self.shared.borrow_mut().windows.values_mut() {
            let size = native.window.inner_size();
            let size = (size.width, size.height);
            // (a minimized window has no picture)
            if size.0 == 0 || size.1 == 0 {
                native.pass = None;
                continue;
            }
            let Some(pass) = native.pass.take() else {
                // left as it is (`keep`), and uncovered or resized by the
                // system meanwhile: its last picture again
                if shown_again(kept, native.asked_for, native.redraw, &native.shown) {
                    native.shown_size = size;
                    native.redraw = false;
                    renderer.draw_outside(
                        native.key,
                        OutsideFrame {
                            width: size.0,
                            height: size.1,
                            clear,
                            ui: hand_over.frame(ctx, native.shown.clone(), egui::TexturesDelta::default(), native.shown_scale),
                        },
                    );
                }
                continue;
            };
            if pass.notice {
                native.notice = under_notice(&mut native.shown, native.notice, pass.shapes);
                native.told = Some(size);
                if native.asked_for {
                    native.shown_size = size;
                    native.shown_scale = pass.pixels_per_point;
                    native.redraw = false;
                    renderer.draw_outside(
                        native.key,
                        OutsideFrame {
                            width: size.0,
                            height: size.1,
                            clear,
                            ui: hand_over.frame(ctx, native.shown.clone(), egui::TexturesDelta::default(), pass.pixels_per_point),
                        },
                    );
                }
                continue;
            }
            native.told = None;
            if !drawn_again(
                native.asked_for,
                native.redraw,
                textures_changed,
                size,
                native.shown_size,
                &pass.shapes,
                &native.shown,
            ) {
                continue;
            }
            native.shown.clone_from(&pass.shapes);
            native.notice = 0;
            native.shown_size = size;
            native.shown_scale = pass.pixels_per_point;
            native.redraw = false;
            renderer.draw_outside(
                native.key,
                OutsideFrame {
                    width: size.0,
                    height: size.1,
                    clear,
                    ui: hand_over.frame(ctx, pass.shapes, egui::TexturesDelta::default(), pass.pixels_per_point),
                },
            );
        }
    }

    /// At exit, once the renderer is gone and its surfaces with it: the
    /// native windows go while the event loop still lives.
    pub fn exit(&mut self) {
        self.closing.clear();
        self.shared.borrow_mut().windows.clear();
        self.activation = Default::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> egui::ViewportIdPair {
        egui::ViewportIdPair {
            this: egui::ViewportId::from_hash_of("window outside"),
            parent: egui::ViewportId::ROOT,
        }
    }

    #[test]
    fn the_pass_of_a_window_outside_has_its_size_in_its_own_points() {
        // a 1200 x 800 window on a screen at 200 %, the interface at 125 %
        let events = egui::RawInput {
            events: vec![egui::Event::PointerMoved(egui::pos2(10.0, 20.0))],
            ..Default::default()
        };
        let input = pass_input(events, ids(), (1200, 800), 2.0, 1.25, true, (12.5, 1.0 / 144.0, 8192));
        assert_eq!(input.viewport_id, ids().this);
        assert_eq!(
            input.screen_rect,
            Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(480.0, 320.0)))
        );
        let info = &input.viewports[&ids().this];
        assert_eq!(info.native_pixels_per_point, Some(2.0));
        assert_eq!(info.parent, Some(egui::ViewportId::ROOT));
        assert_eq!(info.focused, Some(true));
        // the viewer's clock: an animation runs at one pace in every window
        assert_eq!(input.time, Some(12.5));
        assert_eq!(input.max_texture_side, Some(8192));
        // the window's own events, and no other
        assert_eq!(input.events, [egui::Event::PointerMoved(egui::pos2(10.0, 20.0))]);
    }

    #[test]
    fn a_window_replaced_with_the_renderer_is_awaited_a_few_frames_only() {
        let mut left = ASKS_WITHIN;
        // the pass that asks, then the frame that opens it
        assert!(still_waited(&mut left));
        assert!(still_waited(&mut left));
        assert!(!still_waited(&mut left));
        assert!(!still_waited(&mut left));
    }

    fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    /// Ctrl+Shift+A opens People from a field of the viewer's window: from
    /// one of a window outside too, where egui would select all.
    #[test]
    fn the_keys_of_a_window_outside_read_as_in_the_viewer_s() {
        let events = egui::RawInput {
            events: vec![key(egui::Key::A, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT)],
            ..Default::default()
        };
        let input = pass_input(events, ids(), (1200, 800), 1.0, 1.0, true, (0.0, 1.0 / 60.0, 8192));
        let [egui::Event::Key { modifiers, .. }] = input.events.as_slice() else {
            panic!("the key is still there");
        };
        assert!(!modifiers.command && modifiers.shift);
    }

    #[test]
    fn an_escape_and_a_paste_outside_are_heard_by_the_viewer() {
        let events = [
            egui::Event::PointerMoved(egui::pos2(10.0, 20.0)),
            key(egui::Key::Escape, egui::Modifiers::NONE),
            key(egui::Key::A, egui::Modifiers::NONE),
            egui::Event::Paste("text".to_owned()),
        ];
        let heard: Vec<egui::Event> = heard(&events).collect();
        assert_eq!(heard, [events[1].clone(), events[3].clone()]);
    }

    #[test]
    fn a_window_outside_nothing_drew_in_is_let_go() {
        let window = |name: &str| egui::Id::unique(name);
        let open = [
            (viewport(window("conversations")), true),
            (viewport(window("people")), true),
            // (opened by a scenario, which closes it itself)
            (viewport(window("test picture")), false),
        ];
        // Conversations drew in its window; People was closed, hidden with
        // the interface or brought back inside
        let seen = [window("conversations")];
        assert_eq!(unseen(open.into_iter(), &seen), [viewport(window("people"))]);
        assert_eq!(unseen(open.into_iter(), &[]).len(), 2);
    }

    /// The full-frame photo view, the Director's playback and "UI off"
    /// draw none of the interface's windows: those outside are left as
    /// they are for that pass, and only then show their last picture again
    /// when the system asks.
    #[test]
    fn the_windows_outside_are_kept_for_the_pass_that_asked() {
        let ctx = egui::Context::default();
        assert!(!kept(&ctx));
        let p = crate::theme::Theme::default().palette();
        keep(&ctx, &p, Kept::Photo);
        assert!(kept(&ctx));
        // (taken by the host with what the pass left: `reconcile`)
        assert_eq!(ctx.data_mut(|d| d.remove_temp::<bool>(kept_id())), Some(true));
        assert!(!kept(&ctx));
        let picture = vec![ClippedShape {
            clip_rect: egui::Rect::EVERYTHING,
            shape: egui::Shape::Noop,
            transform_after_tessellation: Default::default(),
        }];
        assert!(shown_again(true, true, true, &picture));
        // not one closed or brought inside, which goes at the next frame
        assert!(!shown_again(false, true, true, &picture));
        // nor before the system asked, without a reason, or with nothing shown
        assert!(!shown_again(true, false, true, &picture));
        assert!(!shown_again(true, true, false, &picture));
        assert!(!shown_again(true, true, true, &[]));
    }

    fn shape(at: f32) -> ClippedShape {
        ClippedShape {
            clip_rect: egui::Rect::from_min_size(egui::pos2(at, 0.0), egui::Vec2::splat(1.0)),
            shape: egui::Shape::Noop,
            transform_after_tessellation: Default::default(),
        }
    }

    #[test]
    fn a_window_left_as_it_is_has_its_notice_over_its_picture() {
        let mut shown = vec![shape(1.0), shape(2.0)];
        let notice = under_notice(&mut shown, 0, vec![shape(8.0), shape(9.0)]);
        assert_eq!(notice, 2);
        let at = |shown: &[ClippedShape]| shown.iter().map(|s| s.clip_rect.left()).collect::<Vec<_>>();
        assert_eq!(at(&shown), [1.0, 2.0, 8.0, 9.0]);
        // drawn again for another size: in place of the older notice
        let notice = under_notice(&mut shown, notice, vec![shape(7.0)]);
        assert_eq!(notice, 1);
        assert_eq!(at(&shown), [1.0, 2.0, 7.0]);
        // a window that had no picture yet
        let mut shown = Vec::new();
        assert_eq!(under_notice(&mut shown, 0, vec![shape(7.0)]), 1);
        assert_eq!(at(&shown), [7.0]);
    }

    #[test]
    fn the_notice_is_drawn_once_and_again_for_the_system() {
        // none yet
        assert!(untold(None, (640, 480), false));
        // drawn at this size: nothing more at the frames that follow
        assert!(!untold(Some((640, 480)), (640, 480), false));
        // resized, or uncovered
        assert!(untold(Some((640, 480)), (800, 480), false));
        assert!(untold(Some((640, 480)), (640, 480), true));
        // a minimized window has no picture
        assert!(!untold(None, (0, 0), true));
    }

    #[test]
    fn a_window_closed_where_the_windows_are_not_drawn_is_not_left() {
        // the windows are not drawn any more (the photo view opens): left
        assert!(left_as_it_is(true, true, false));
        // and at the passes that follow
        assert!(left_as_it_is(true, false, true));
        // one that was drawn in there (Phototools), closed: it goes
        assert!(!left_as_it_is(true, true, true));
        // the windows are drawn: one nothing draws in goes
        assert!(!left_as_it_is(false, true, false));
        assert!(!left_as_it_is(false, false, true));
    }

    #[test]
    fn the_notice_goes_to_the_windows_nothing_drew_in() {
        let ctx = egui::Context::default();
        let p = crate::theme::Theme::default().palette();
        let (left, live) = (egui::Id::unique("left"), egui::Id::unique("live"));
        let told = |ctx: &egui::Context| ctx.data_mut(|d| d.remove_temp::<bool>(notice_id())).unwrap_or(false);
        // (no host here: egui runs a window outside inside the viewer's pass)
        crate::ui::login::test_output(ctx.run_ui(Default::default(), |ui| {
            let ctx = ui.ctx();
            ctx.data_mut(|d| d.insert_temp(untold_id(), vec![live]));
            seen(ctx, live);
            keep(ctx, &p, Kept::Photo);
            assert!(!told(ctx), "a window this pass drew in is not left as it is");
            ctx.data_mut(|d| d.insert_temp(untold_id(), vec![left, live]));
            keep(ctx, &p, Kept::Director);
            assert!(told(ctx));
            // told once: the host names it again when its notice is to draw
            keep(ctx, &p, Kept::Director);
            assert!(!told(ctx));
        }));
    }

    #[test]
    fn a_window_asks_to_go_outside_and_the_wish_is_kept() {
        let ctx = egui::Context::default();
        let window = egui::Id::unique("conversations");
        assert!(!wanted(&ctx, window));
        assert!(!native(&ctx, window));
        want(&ctx, window, true);
        assert!(wanted(&ctx, window));
        // (the wish alone opens nothing: the window asks, then draws)
        assert!(!native(&ctx, window));
        set_native(&ctx, window, true);
        assert!(native(&ctx, window));
        set_native(&ctx, window, false);
        assert!(!native(&ctx, window));
        want(&ctx, window, false);
        assert!(!wanted(&ctx, window));
    }

    #[test]
    fn a_window_outside_is_raised_once_however_often_it_is_asked() {
        let ctx = egui::Context::default();
        let (chat, people) = (egui::Id::unique("conversations"), egui::Id::unique("people"));
        raise(&ctx, chat);
        raise(&ctx, people);
        raise(&ctx, chat);
        let raises = ctx.data(|d| d.get_temp::<Vec<egui::Id>>(raises_id())).unwrap_or_default();
        assert_eq!(raises, [chat, people]);
        // (no window outside has the focus until the host says so)
        assert!(!in_focus(&ctx, chat));
        ctx.data_mut(|d| d.insert_temp(front_id(), chat));
        assert!(in_focus(&ctx, chat));
        assert!(!in_focus(&ctx, people));
    }

    /// What has the keyboard in a window (a web page) lets go of it when
    /// the focus arrives in another window of the viewer, not when another
    /// program takes it nor when it comes back to its own window.
    #[test]
    fn the_keyboard_held_in_a_window_goes_with_the_focus_to_another_window_of_the_viewer() {
        let own = egui::ViewportId::ROOT;
        let (chat, search) = (viewport(egui::Id::unique("conversations")), viewport(egui::Id::unique("search")));
        // a page outside, the viewer's own window clicked; a page inside,
        // a window outside clicked; a page outside, another one clicked
        assert!(focus_went_elsewhere(search, Some(search), Some(own)));
        assert!(focus_went_elsewhere(own, Some(own), Some(chat)));
        assert!(focus_went_elsewhere(search, Some(search), Some(chat)));
        // another program in front, then the viewer's other window
        assert!(!focus_went_elsewhere(search, Some(search), None));
        assert!(focus_went_elsewhere(search, None, Some(own)));
        // back to its own window, or nothing new: the page clicked keeps it
        assert!(!focus_went_elsewhere(search, None, Some(search)));
        assert!(!focus_went_elsewhere(search, Some(own), Some(search)));
        assert!(!focus_went_elsewhere(search, Some(own), Some(own)));
    }

    #[test]
    fn the_focused_window_is_the_one_outside_then_the_viewers_own() {
        let ctx = egui::Context::default();
        let chat = egui::Id::unique("conversations");
        crate::ui::login::test_output(ctx.run_ui(egui::RawInput::default(), |_| {}));
        assert_eq!(focused_window(&ctx), Some(egui::ViewportId::ROOT));
        publish_front(&ctx, Some(chat));
        assert_eq!(focused_window(&ctx), Some(viewport(chat)));
        publish_front(&ctx, None);
        crate::ui::login::test_output(ctx.run_ui(
            egui::RawInput {
                focused: false,
                ..Default::default()
            },
            |_| {},
        ));
        assert_eq!(focused_window(&ctx), None);
    }

    #[test]
    fn a_window_outside_stays_in_front_for_a_moment_after_the_focus_left() {
        let ctx = egui::Context::default();
        let (chat, people) = (egui::Id::unique("conversations"), egui::Id::unique("people"));
        let at = |time: f64| {
            crate::ui::login::test_output(ctx.run_ui(
                egui::RawInput {
                    time: Some(time),
                    ..Default::default()
                },
                |_| {},
            ));
        };
        at(10.0);
        publish_front(&ctx, Some(chat));
        assert!(shows_in_front(&ctx, chat));
        assert!(!shows_in_front(&ctx, people));
        // the click in the viewer's window takes the focus: still in front
        at(10.2);
        publish_front(&ctx, None);
        assert!(!in_focus(&ctx, chat));
        assert!(shows_in_front(&ctx, chat));
        at(10.9);
        publish_front(&ctx, None);
        assert!(shows_in_front(&ctx, chat));
        // longer ago: anything may cover it by now
        at(11.5);
        publish_front(&ctx, None);
        assert!(!shows_in_front(&ctx, chat));
        // the focus that goes to another window outside leaves the first
        at(20.0);
        publish_front(&ctx, Some(chat));
        at(20.1);
        publish_front(&ctx, Some(people));
        assert!(shows_in_front(&ctx, people));
        assert!(shows_in_front(&ctx, chat));
        at(22.0);
        assert!(shows_in_front(&ctx, people));
        assert!(!shows_in_front(&ctx, chat));
    }

    #[test]
    fn the_last_size_asked_of_a_window_outside_wins() {
        let ctx = egui::Context::default();
        let (inventory, people) = (egui::Id::unique("inventory"), egui::Id::unique("people"));
        resize(&ctx, inventory, egui::vec2(600.0, 400.0), false);
        resize(&ctx, people, egui::vec2(300.0, 500.0), true);
        resize(&ctx, inventory, egui::vec2(1180.0, 400.0), false);
        assert_eq!(
            resizes(&ctx),
            [
                (people, egui::vec2(300.0, 500.0), true),
                (inventory, egui::vec2(1180.0, 400.0), false)
            ]
        );
    }

    #[test]
    fn a_size_larger_than_the_screen_is_brought_down_to_it() {
        let room = Some(egui::vec2(1280.0, 800.0));
        assert_eq!(fitting(egui::vec2(400.0, 600.0), room), egui::vec2(400.0, 600.0));
        // taller than the screen less the system's bars: its content scrolls
        assert_eq!(fitting(egui::vec2(400.0, 1100.0), room), egui::vec2(400.0, 720.0));
        assert_eq!(fitting(egui::vec2(1600.0, 300.0), room), egui::vec2(1280.0, 300.0));
        // (a system that does not tell the screen: as asked)
        assert_eq!(fitting(egui::vec2(400.0, 1100.0), None), egui::vec2(400.0, 1100.0));
        // asked of the system in whole units, never under the size: 359.4
        // points at an interface scale of 0.8333 are 299.47 units, and a
        // window of 299 would be shorter than what it shows
        assert_eq!(whole(egui::vec2(300.0, 359.4), 0.8333), winit::dpi::LogicalSize::new(250.0, 300.0));
        assert_eq!(whole(egui::vec2(300.0, 360.0), 1.1), winit::dpi::LogicalSize::new(330.0, 396.0));
        assert_eq!(whole(egui::vec2(256.0, 203.5), 1.0), winit::dpi::LogicalSize::new(256.0, 204.0));
    }

    /// A restart: the layout is written at exit and read at the next start.
    #[test]
    fn a_window_sent_outside_is_outside_again_after_a_restart() {
        let ctx = egui::Context::default();
        let window = egui::Id::unique("conversations");
        want(&ctx, window, true);
        set_native(&ctx, window, true);
        let saved = crate::ui::layout::text(&ctx).expect("the layout is written");
        let next = egui::Context::default();
        crate::ui::layout::apply(&next, &saved).expect("the layout is read");
        assert!(wanted(&next, window));
        // (its window of the last session is gone: it asks for a new one)
        assert!(!native(&next, window));
        assert!(!wanted(&next, egui::Id::unique("people")));
    }

    #[test]
    fn a_reset_of_the_windows_is_left_for_the_window_to_take() {
        let ctx = egui::Context::default();
        let window = egui::Id::unique("conversations");
        assert!(!crate::ui::layout::reset_pending(&ctx, window));
        crate::ui::layout::reset_windows(&ctx);
        // asked twice, still there: the window outside comes back inside,
        // then takes the reset as a window inside does
        assert!(crate::ui::layout::reset_pending(&ctx, window));
        assert!(crate::ui::layout::reset_pending(&ctx, window));
        assert!(crate::ui::layout::take_reset(&ctx, window));
        assert!(!crate::ui::layout::reset_pending(&ctx, window));
    }

    #[test]
    fn a_key_goes_on_to_the_viewer_unless_the_window_outside_reads_it() {
        // a list, a picture: the key is a shortcut or a movement, as under
        // a window inside
        assert!(key_goes_on(true, false));
        // a text field or an open menu keeps the press
        assert!(!key_goes_on(true, true));
        // what a press held is let go wherever the release arrives
        assert!(key_goes_on(false, true));
        assert!(key_goes_on(false, false));
    }

    #[test]
    fn the_viewer_is_in_front_with_the_focus_in_any_of_its_windows() {
        assert!(in_front(true, false));
        assert!(in_front(false, true));
        assert!(in_front(true, true));
        // another program has the focus
        assert!(!in_front(false, false));
    }

    #[test]
    fn a_focused_field_or_an_open_menu_reads_the_keys() {
        let ctx = egui::Context::default();
        let mut text = String::new();
        let mut read = Vec::new();
        for focus in [false, true, true] {
            crate::ui::login::test_output(ctx.run_ui(egui::RawInput::default(), |ui| {
                let field = ui.text_edit_singleline(&mut text);
                if focus {
                    field.request_focus();
                }
                read.push(reads_keys(ui.ctx()));
            }));
        }
        assert_eq!(read, [false, true, true]);
    }

    #[test]
    fn a_window_outside_is_drawn_again_only_when_its_picture_changed() {
        let ctx = egui::Context::default();
        let shapes = |text: &str| {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.label(text);
            });
            out.textures_delta.clear();
            out.shapes
        };
        let (first, same, other) = (shapes("one"), shapes("one"), shapes("two"));
        let size = (640, 480);
        assert!(!drawn_again(true, false, false, size, size, &same, &first));
        assert!(drawn_again(true, false, false, size, size, &other, &first));
        // resized, uncovered by the system, or a picture that arrived
        assert!(drawn_again(true, false, false, (800, 480), size, &same, &first));
        assert!(drawn_again(true, true, false, size, size, &same, &first));
        assert!(drawn_again(true, false, true, size, size, &same, &first));
    }

    /// A new window's first picture waits for the system to ask for one:
    /// shown before its title bar is drawn, it made the system count the
    /// window without the title bar and shrink it by that height at each
    /// opening.
    #[test]
    fn nothing_is_drawn_in_a_window_outside_before_the_system_asks() {
        let size = (640, 480);
        // a new window: nothing shown yet, everything to draw
        assert!(!drawn_again(false, true, true, size, (0, 0), &[], &[]));
        assert!(drawn_again(true, true, true, size, (0, 0), &[], &[]));
    }

    /// The pass of a window outside, as egui runs it inside the viewer's:
    /// both leave their own shapes, at their own scale.
    #[test]
    fn a_pass_outside_runs_inside_the_viewer_s_pass() {
        let ctx = egui::Context::default();
        let outside = Outside::new(&ctx);
        let id = ids().this;
        // (no native window in a test: the pass is run by hand, as egui's
        // function does)
        let kept: Rc<RefCell<Option<egui::FullOutput>>> = Rc::default();
        let for_pass = kept.clone();
        egui::Context::set_immediate_viewport_renderer(move |ctx, viewport| {
            let egui::ImmediateViewport {
                ids, mut viewport_ui_cb, ..
            } = viewport;
            let clock = ctx.input(|i| (i.time, i.predicted_dt, i.max_texture_side));
            let input = pass_input(egui::RawInput::default(), ids, (600, 400), 2.0, ctx.zoom_factor(), false, clock);
            *for_pass.borrow_mut() = Some(ctx.run_ui(input, |ui| viewport_ui_cb(ui)));
        });
        let mut root = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.label("inside");
            let answer = ui.ctx().show_viewport_immediate(id, egui::ViewportBuilder::default(), |ui, class| {
                assert!(class == egui::ViewportClass::Immediate);
                assert_eq!(ui.ctx().viewport_id(), id);
                assert_eq!(ui.ctx().content_rect().size(), egui::vec2(300.0, 200.0));
                ui.label("outside");
                7
            });
            assert_eq!(answer, 7);
            assert_eq!(ui.ctx().viewport_id(), egui::ViewportId::ROOT);
        });
        let mut out = kept.borrow_mut().take().expect("the pass outside ran");
        assert_eq!(out.pixels_per_point, 2.0);
        assert_eq!(root.pixels_per_point, 1.0);
        let texts = |shapes: &[ClippedShape]| -> Vec<String> {
            shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Text(text) => Some(text.galley.job.text.clone()),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(texts(&out.shapes), ["outside"]);
        assert_eq!(texts(&root.shapes), ["inside"]);
        root.textures_delta.clear();
        out.textures_delta.clear();
        // (without a native window nothing is asked of it)
        assert!(!outside.is_open(id));
        assert_eq!(outside.show(&ctx, id, |_| ()), None);
    }

    fn sources(path: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        if path.is_dir() {
            for entry in std::fs::read_dir(path).expect("a source directory can be read") {
                sources(&entry.expect("a directory entry").path(), out);
            }
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path.to_owned());
        }
    }

    /// The frame and pass numbers of a pass outside are its viewport's own:
    /// a stamp read in another window is the viewer's own window's.
    #[test]
    fn frames_and_passes_are_counted_by_the_viewers_own_window() {
        let mut files = Vec::new();
        sources(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
        assert!(files.len() > 100, "the sources were found");
        let own = [concat!(".cumulative_frame", "_nr()"), concat!(".cumulative_pass", "_nr()")];
        let mut found = Vec::new();
        for file in files {
            let source = std::fs::read_to_string(&file).expect("a source file can be read");
            for (n, line) in source.lines().enumerate() {
                if own.iter().any(|call| line.contains(call)) && !line.trim_start().starts_with("//") {
                    found.push(format!("{}:{}", file.display(), n + 1));
                }
            }
        }
        assert!(
            found.is_empty(),
            "a window drawn outside counts its own frames and passes: use cumulative_frame_nr_for(egui::ViewportId::ROOT) \
             or cumulative_pass_nr_for(egui::ViewportId::ROOT) at {found:?}"
        );
    }

    #[test]
    fn a_pass_outside_reads_the_frame_of_the_viewers_own_window() {
        let ctx = egui::Context::default();
        let _outside = Outside::new(&ctx);
        let id = egui::ViewportId::from_hash_of("counted apart");
        let kept: Rc<RefCell<Option<(u64, u64)>>> = Rc::default();
        let for_pass = kept.clone();
        egui::Context::set_immediate_viewport_renderer(move |ctx, viewport| {
            let egui::ImmediateViewport {
                ids, mut viewport_ui_cb, ..
            } = viewport;
            let clock = ctx.input(|i| (i.time, i.predicted_dt, i.max_texture_side));
            let input = pass_input(egui::RawInput::default(), ids, (600, 400), 1.0, 1.0, false, clock);
            crate::ui::login::test_output(ctx.run_ui(input, |ui| {
                *for_pass.borrow_mut() = Some((
                    ui.ctx().cumulative_frame_nr_for(ui.ctx().viewport_id()),
                    ui.ctx().cumulative_frame_nr_for(egui::ViewportId::ROOT),
                ));
                viewport_ui_cb(ui);
            }));
        });
        // (the window goes outside after a few frames of the viewer)
        let mut root = 0;
        for frame in 0..5 {
            crate::ui::login::test_output(ctx.run_ui(egui::RawInput::default(), |ui| {
                root = ui.ctx().cumulative_frame_nr_for(egui::ViewportId::ROOT);
                if frame >= 3 {
                    ui.ctx().show_viewport_immediate(id, egui::ViewportBuilder::default(), |_, _| ());
                }
            }));
        }
        let (own, of_root) = kept.borrow_mut().take().expect("the pass outside ran");
        assert_eq!(root, 4);
        assert_eq!(of_root, root);
        assert_eq!(own, 1, "egui counts a viewport's frames apart");
    }
}
