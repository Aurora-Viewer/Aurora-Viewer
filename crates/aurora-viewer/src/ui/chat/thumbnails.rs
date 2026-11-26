//! The thumbnails of the open conversations (#1134): the Legacy Viewer's IM
//! chiclets (LLChicletPanel with its LLIMP2PChiclet, LLIMGroupChiclet and
//! LLAdHocChiclet, llchiclet.cpp; one per session from
//! LLChicletBar::sessionAdded, none with FSDisableIMChiclets). One square
//! per conversation, in the order of the Conversations list: a resident's
//! picture with their status, a group's insignia, the conference glyph. Its
//! unread count wears the one red counter, none while the conversation is
//! shown (LLIMChiclet::setCounter from LLChicletPanel::onMessageCountChanged).
//! A click shows the conversation, or puts it away when it is already shown
//! (FSFloaterIM::toggle); a right click opens the conversation's menu, the
//! list's own, which holds those of menu_imchiclet_p2p / group / adhoc.xml
//! (profile, add friend, show, end the session).
//!
//! Aurora's differences, the human's choices: they stand beside the chat
//! bar by default (the Legacy Viewer keeps them at the top right, the other
//! choice of Preferences › Chat); a message flashes the thumbnail like an
//! entry of the list (`flash.rs`: eight soft rises with the Conversations
//! button's mark, then a faint mark until it is read), where the Legacy
//! Viewer flashes its IM well only. When more conversations are open than
//! fit, left and right scroll arrows step through them in list order when
//! both arrows and one square fit (LLChicletPanel's
//! `chicklet_left_scroll_button` / `chicklet_right_scroll_button`, #3552);
//! otherwise a "+N" slot lists the rest with unread kept in sight (a lone
//! "+N" included). The centred-gap probe alone treats a lone "+N" as no
//! room, so the dock falls back with the strip left of it; without even
//! one square beside the chat bar they go to the top right.

use super::{ChatUi, ConvAction, presence, sidebar};
use crate::settings::ChatThumbnails;
use crate::theme::Palette;
use crate::ui::Panels;
use crate::ui::bars::islands;
use crate::world::World;
use egui::{Align2, Pos2, Rect, Sense, Vec2};
use std::collections::HashMap;
use uuid::Uuid;

/// Room between two thumbnails: the counter of one overhangs its right
/// edge by 2 points.
const GAP: f32 = 4.0;
/// A thumbnail in the Legacy Viewer's bottom bar: the height of its buttons.
pub const BAR_SIZE: f32 = 22.0;

/// The thumbnails' state, kept in `ChatUi`.
#[derive(Default)]
pub struct Thumbnails {
    /// Where they stand and whether they flash (Preferences › Chat), and
    /// Reduce motion, given every frame by the app (`prepare`).
    place: ChatThumbnails,
    flash: bool,
    still: bool,
    /// The conversations and their unread counts, in the list's order.
    items: Vec<(Uuid, usize)>,
    /// Index of the first thumbnail shown when the row scrolls
    /// (LLChicletPanel's chiclet strip).
    scroll: usize,
    /// A conversation just opened: bring it into the scroll window
    /// (`LLChicletPanel::scrollToChiclet` from `createChiclet`).
    scroll_into: Option<Uuid>,
    /// The room the bottom bar left them this frame (`reserve`).
    slot: Option<Slot>,
    flashes: super::flash::Flashes,
    /// What their right-click menus asked, handed to `chat::show`.
    pub(super) actions: Vec<ConvAction>,
}

/// Where the thumbnails are drawn: a row of `size` squares in `rect`, in
/// a glass island of their own (`island`) or in the Legacy Viewer's bar.
#[derive(Debug, Clone, Copy)]
struct Slot {
    rect: Rect,
    size: f32,
    island: bool,
    layer: Option<egui::LayerId>,
    clip: Option<Rect>,
    vertical: bool,
}

impl Slot {
    fn padding(self) -> f32 {
        let height = if self.vertical { islands::HEIGHT } else { self.rect.height() };
        (height - self.size) / 2.0
    }

    fn square(self, index: usize) -> Rect {
        let along = self.padding() + index as f32 * (self.size + GAP);
        let min = if self.vertical {
            Pos2::new(self.rect.center().x - self.size / 2.0, self.rect.top() + along)
        } else {
            Pos2::new(self.rect.left() + along, self.rect.top() + self.padding())
        };
        Rect::from_min_size(min, Vec2::splat(self.size))
    }
}

impl Thumbnails {
    /// The thumbnails beside the chat bar: the room they take, a row of
    /// `size` squares with `pad` at each end, at most `room` (0: none).
    /// Overflow: arrows when both fit with one square, else as many squares
    /// as fit (the last may be "+N", a lone "+N" included). Overflow width
    /// is never below a plain row, so one more conversation does not shrink
    /// the strip and pull the dock left.
    pub fn bar_width(&self, size: f32, pad: f32, room: f32) -> f32 {
        if self.place != ChatThumbnails::ChatBar || self.items.is_empty() {
            return 0.0;
        }
        let plain = fit(room, size, pad);
        if plain == 0 {
            return 0.0;
        }
        if self.items.len() <= plain {
            return row_width(self.items.len(), size, pad);
        }
        overflow_width(room, size, pad, plain)
    }

    /// Width for the centred-gap probe only (#3552): like `bar_width`, but
    /// a lone "+N" (one square, more conversations, no arrow room) is 0 so
    /// the dock falls back with the strip left of it. Other callers keep
    /// the lone "+N" from `bar_width`.
    pub fn centred_gap_width(&self, size: f32, pad: f32, room: f32) -> f32 {
        if self.place != ChatThumbnails::ChatBar || self.items.is_empty() {
            return 0.0;
        }
        let plain = fit(room, size, pad);
        if plain == 0 {
            return 0.0;
        }
        if self.items.len() <= plain {
            return row_width(self.items.len(), size, pad);
        }
        if scroll_capacity(room, size, pad) == 0 && plain < 2 {
            return 0.0;
        }
        overflow_width(room, size, pad, plain)
    }

    /// The bottom bar left them `rect` (`bar_width` wide).
    pub fn reserve(&mut self, rect: Rect, size: f32, island: bool) {
        if rect.width() > 0.0 {
            self.slot = Some(Slot {
                rect,
                size,
                island,
                layer: None,
                clip: None,
                vertical: false,
            });
        }
    }

    /// How many thumbnails the top bars make room for (at the top right).
    pub fn top_count(&self) -> usize {
        if self.place == ChatThumbnails::TopRight {
            self.items.len()
        } else {
            0
        }
    }
}

/// The width of a row of `n` thumbnails of `size` with `pad` at each end,
/// within `room` (0: none): the top bars' room for them. Overflow matches
/// the bottom strip: arrows when they fit, else as many squares as fit
/// (a lone "+N" included).
pub fn top_width(n: usize, size: f32, pad: f32, room: f32) -> f32 {
    if n == 0 {
        return 0.0;
    }
    let plain = fit(room, size, pad);
    if plain == 0 {
        return 0.0;
    }
    if n <= plain {
        return row_width(n, size, pad);
    }
    overflow_width(room, size, pad, plain)
}

fn top_id() -> egui::Id {
    egui::Id::unique("conversation_thumbnails_top")
}

/// The top bars left the thumbnails `rect` this frame: an island of the
/// top row, or room in the Legacy Viewer's navigation bar.
pub fn reserve_top(ctx: &egui::Context, rect: Rect, size: f32, island: bool) {
    // (the frame read outside: the context is locked within `data_mut`)
    let frame = ctx.cumulative_frame_nr_for(egui::ViewportId::ROOT);
    ctx.data_mut(|d| {
        d.insert_temp(
            top_id(),
            (
                frame,
                Slot {
                    rect,
                    size,
                    island,
                    layer: None,
                    clip: None,
                    vertical: false,
                },
            ),
        )
    });
}

pub fn reserve_toolbar(ctx: &egui::Context, rect: Rect, size: f32, layer: egui::LayerId, clip: Rect, vertical: bool) {
    let frame = ctx.cumulative_frame_nr_for(egui::ViewportId::ROOT);
    ctx.data_mut(|data| {
        data.insert_temp(
            top_id(),
            (
                frame,
                Slot {
                    rect,
                    size,
                    island: false,
                    layer: Some(layer),
                    clip: Some(clip),
                    vertical,
                },
            ),
        )
    });
}

/// What `reserve_top` left this frame.
fn top_slot(ctx: &egui::Context) -> Option<Slot> {
    let frame = ctx.cumulative_frame_nr_for(egui::ViewportId::ROOT);
    ctx.data(|d| d.get_temp::<(u64, Slot)>(top_id()))
        .filter(|(f, _)| *f == frame)
        .map(|(_, s)| s)
}

/// How many `size` squares a row `room` wide holds, with `pad` at each end.
fn fit(room: f32, size: f32, pad: f32) -> usize {
    ((room - pad * 2.0 + GAP) / (size + GAP)).floor().max(0.0) as usize
}

/// The width of a row of `n` squares of `size`, with `pad` at each end.
fn row_width(n: usize, size: f32, pad: f32) -> f32 {
    pad * 2.0 + n as f32 * size + n.saturating_sub(1) as f32 * GAP
}

/// Width of one scroll arrow beside the chiclet strip (narrower than a
/// thumbnail, as LLChicletPanel's scroll buttons).
fn arrow_width(size: f32) -> f32 {
    (size * 0.55).round().max(14.0)
}

/// How many thumbnails fit in `room` once both scroll arrows take their
/// place (0: not enough room for arrows; use "+N" instead).
fn scroll_capacity(room: f32, size: f32, pad: f32) -> usize {
    let arrows = 2.0 * arrow_width(size) + 2.0 * GAP;
    fit((room - arrows).max(0.0), size, pad)
}

/// Width of a scrolled strip: pads, both arrows, gaps, and `n` squares.
fn scroll_row_width(n: usize, size: f32, pad: f32) -> f32 {
    pad * 2.0 + 2.0 * arrow_width(size) + 2.0 * GAP + n as f32 * size + n.saturating_sub(1) as f32 * GAP
}

/// Reserved width when conversations overflow `plain` squares in `room`.
/// Arrows when they fit; else a plain "+N" row (a lone "+N" included).
/// Never narrower than a plain row, so adding a conversation cannot shrink
/// the strip. The scrolled strip may leave slack on the right; `row` places
/// › after the last square (LLChicletPanel::trimChiclets).
fn overflow_width(room: f32, size: f32, pad: f32, plain: usize) -> f32 {
    match scroll_capacity(room, size, pad) {
        0 => row_width(plain, size, pad),
        n => scroll_row_width(n, size, pad).max(row_width(plain, size, pad)),
    }
}

/// The window of `n` conversations at `scroll` when arrows are used
/// (LLChicletPanel). Returns `(start, count)`.
fn window(n: usize, scroll_fit: usize, scroll: usize) -> (usize, usize) {
    let fit = scroll_fit.max(1).min(n);
    let start = scroll.min(n.saturating_sub(fit));
    (start, fit)
}

/// What a row of `fit` squares shows with a "+N" slot: all of them when
/// they fit; else `fit - 1` and the rest on "+N", unread kept in sight
/// first, in list order.
fn arrange(items: &[(Uuid, usize)], fit: usize) -> (Vec<usize>, Vec<usize>) {
    if items.len() <= fit {
        return ((0..items.len()).collect(), Vec::new());
    }
    let room = fit.saturating_sub(1);
    let unread = items.iter().enumerate().filter(|(_, (_, n))| *n > 0).map(|(i, _)| i);
    let read = items.iter().enumerate().filter(|(_, (_, n))| *n == 0).map(|(i, _)| i);
    let mut shown: Vec<usize> = unread.chain(read).take(room).collect();
    shown.sort_unstable();
    let rest = (0..items.len()).filter(|i| !shown.contains(i)).collect();
    (shown, rest)
}

/// Bring `index` into a window of `count` starting at `scroll`.
fn scroll_to(scroll: &mut usize, index: usize, count: usize) {
    if count == 0 {
        return;
    }
    if index < *scroll {
        *scroll = index;
    } else if index >= *scroll + count {
        *scroll = index + 1 - count;
    }
}

/// AURORA_DEMO_THUMBNAILS=bar|top|off: where the thumbnails stand for the
/// run, the settings left alone.
pub fn demo_place() -> Option<ChatThumbnails> {
    match std::env::var("AURORA_DEMO_THUMBNAILS").ok()?.as_str() {
        "bar" => Some(ChatThumbnails::ChatBar),
        "top" => Some(ChatThumbnails::TopRight),
        "off" => Some(ChatThumbnails::Off),
        _ => None,
    }
}

/// The conversations of this frame and the settings, before the bars
/// (they reserve the thumbnails' room).
pub fn prepare(st: &mut ChatUi, world: &World, place: ChatThumbnails, flash: bool, still: bool) {
    let t = &mut st.thumbnails;
    t.place = place;
    t.flash = flash;
    t.still = still;
    t.slot = None;
    let before: Vec<Uuid> = t.items.iter().map(|(id, _)| *id).collect();
    t.items = if place == ChatThumbnails::Off {
        Vec::new()
    } else {
        st.reorder.sessions(world)
    };
    // LLChicletPanel::createChiclet → scrollToChiclet: a new session is shown
    let mut newest = None;
    for (id, _) in &t.items {
        if !before.contains(id) {
            newest = Some(*id);
        }
    }
    if newest.is_some() {
        t.scroll_into = newest;
    }
    if t.items.is_empty() {
        t.scroll = 0;
        t.scroll_into = None;
    } else {
        t.scroll = t.scroll.min(t.items.len() - 1);
    }
}

/// Is `id` on screen: the conversation shown in the Conversations window,
/// or its own window (`popout.rs`) shown?
fn shown(ctx: &egui::Context, st: &ChatUi, conversations_open: bool, id: Uuid) -> bool {
    if st.popped.contains(Some(id)) {
        super::popout::in_sight(ctx, &st.popped).any(|conversation| conversation == Some(id))
    } else {
        conversations_open && !crate::ui::widgets::floater_minimized(ctx, "conversations") && st.selected == Some(id) && !st.contacts
    }
}

/// A click on the thumbnail of `id`: the conversation shown, with its field
/// taking the keyboard, or put away when it already is (FSFloaterIM::toggle).
fn toggle(ctx: &egui::Context, world: &mut World, st: &mut ChatUi, panels: &mut Panels, id: Uuid, on_screen: bool) {
    let apart = st.popped.contains(Some(id));
    match (on_screen, apart) {
        (true, true) => {
            st.popped.set_hidden(Some(id), true);
        }
        (true, false) => crate::ui::widgets::toggle_floater(ctx, "conversations", &mut panels.chat),
        (false, true) => {
            st.popped.raise(Some(id), true);
        }
        (false, false) => {
            world.focus_conversation(id, true);
            crate::ui::notifications::open_conversations(ctx, &mut panels.chat);
        }
    }
}

/// The thumbnails, after the bars: in the room the bars left them, else at
/// the top right of `work` (the windows' work area) when `top` (the top
/// bars are shown).
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &mut World,
    st: &mut ChatUi,
    panels: &mut Panels,
    pics: &HashMap<Uuid, egui::TextureHandle>,
    images: &HashMap<Uuid, egui::TextureHandle>,
    work: Rect,
    top: bool,
) {
    let ctx = ui.ctx().clone();
    let t = &st.thumbnails;
    let editing = crate::ui::toolbar::ToolbarUi::intercepting(&ctx);
    if editing && top_slot(&ctx).is_none() {
        return;
    }
    if t.place == ChatThumbnails::TopRight
        && crate::ui::toolbar::appearance::settings(&ctx)
            .drawer_of("bar_conversations")
            .is_some()
        && top_slot(&ctx).is_none()
    {
        return;
    }
    if t.items.is_empty() {
        return;
    }
    let slot = match (t.place, t.slot, top_slot(&ctx)) {
        (ChatThumbnails::Off, _, _) => return,
        (ChatThumbnails::ChatBar, Some(slot), _) => slot,
        (ChatThumbnails::TopRight, _, Some(slot)) => slot,
        // the chat bar without room for one: the top right of the windows'
        // area (only while the top bars are shown)
        _ if !top => return,
        _ => {
            let size = islands::SLOT;
            let height = islands::HEIGHT;
            let pad = (height - size) / 2.0;
            let room = work.width() / 2.0;
            let plain = fit(room, size, pad);
            if plain == 0 {
                return;
            }
            let width = if t.items.len() <= plain {
                row_width(t.items.len(), size, pad)
            } else {
                overflow_width(room, size, pad, plain)
            };
            let rect = Rect::from_min_size(Pos2::new(work.right() - width, work.top()), Vec2::new(width, height));
            Slot {
                rect,
                size,
                island: true,
                layer: None,
                clip: None,
                vertical: false,
            }
        }
    };
    let _context = crate::ui::toolbar::native_context::Scope::new(&ctx, "bar_conversations");
    let now = ctx.input(|i| i.time);
    st.thumbnails.flashes.begin(now);
    let mut moving = false;
    if slot.island {
        islands::area(&ctx, p, "conversation_thumbnails", slot.rect, |ui| {
            if editing {
                crate::ui::toolbar::edit_mode::preview(ui);
            }
            moving = row(ui, p, world, st, panels, pics, images, slot, now);
        });
    } else {
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(slot.rect)
                .layer_id(slot.layer.unwrap_or(ui.layer_id()))
                .layout(if slot.vertical {
                    egui::Layout::top_down(egui::Align::Center)
                } else {
                    egui::Layout::left_to_right(egui::Align::Center)
                }),
        );
        // counters and the arrow unread badge lift above the squares
        let lift = (slot.size * 0.4).round().max(8.0) + 2.0;
        child.set_clip_rect(slot.rect.expand(lift).intersect(slot.clip.unwrap_or(Rect::EVERYTHING)));
        if editing {
            crate::ui::toolbar::edit_mode::preview(&mut child);
        }
        moving = row(&mut child, p, world, st, panels, pics, images, slot, now);
    }
    if moving {
        ctx.request_repaint();
    }
}

/// The row of thumbnails in `slot`; returns whether a flash moves.
#[allow(clippy::too_many_arguments)]
fn row(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &mut World,
    st: &mut ChatUi,
    panels: &mut Panels,
    pics: &HashMap<Uuid, egui::TextureHandle>,
    images: &HashMap<Uuid, egui::TextureHandle>,
    slot: Slot,
    now: f64,
) -> bool {
    let ctx = ui.ctx().clone();
    let pad = slot.padding();
    let along = if slot.vertical { slot.rect.height() } else { slot.rect.width() };
    let plain = fit(along, slot.size, pad);
    let scrolled = if slot.vertical { 0 } else { scroll_capacity(along, slot.size, pad) };
    // what is unread in a conversation on screen is being read
    let items: Vec<(Uuid, usize)> = st
        .thumbnails
        .items
        .iter()
        .map(|&(id, n)| (id, if shown(&ctx, st, panels.chat, id) { 0 } else { n }))
        .collect();
    let arrows = !slot.vertical && items.len() > plain && scrolled > 0;
    let mut moving = false;
    if arrows {
        let (mut start, count) = window(items.len(), scrolled, st.thumbnails.scroll);
        if let Some(id) = st.thumbnails.scroll_into.take()
            && let Some(i) = items.iter().position(|(x, _)| *x == id)
        {
            scroll_to(&mut start, i, count);
        }
        st.thumbnails.scroll = start;
        let aw = arrow_width(slot.size);
        let content_left = slot.rect.left() + pad + aw + GAP;
        let square = |at: usize| {
            Rect::from_min_size(
                Pos2::new(content_left + at as f32 * (slot.size + GAP), slot.rect.top() + pad),
                Vec2::splat(slot.size),
            )
        };
        let left = Rect::from_min_size(Pos2::new(slot.rect.left() + pad, slot.rect.top() + pad), Vec2::new(aw, slot.size));
        // › sits after the last square (LLChicletPanel::trimChiclets); slack
        // from a wider reserved slot stays on the right, not between squares and ›
        let after_last = content_left + count as f32 * (slot.size + GAP);
        let right = Rect::from_min_size(Pos2::new(after_last, slot.rect.top() + pad), Vec2::new(aw, slot.size));
        let can_left = start > 0;
        let can_right = start + count < items.len();
        let unread_left: usize = items.iter().take(start).map(|(_, n)| *n).sum();
        let unread_right: usize = items.iter().skip(start + count).map(|(_, n)| *n).sum();
        let steps_left = scroll_arrow(
            ui,
            p,
            left,
            "caret-left-filled",
            can_left,
            unread_left,
            aurora_i18n::tr!("common-previous"),
        );
        let steps_right = scroll_arrow(
            ui,
            p,
            right,
            "caret-right-filled",
            can_right,
            unread_right,
            aurora_i18n::tr!("common-next"),
        );
        // one notch per step over the whole strip (LLChicletPanel::handleScrollWheel;
        // wheel down / positive Legacy clicks → right). spin_wheel sees the
        // pointer even over the squares' click widgets.
        let wheel = crate::ui::widgets::spin_wheel(ui, slot.rect, egui::Id::unique("conversation_thumbnails_wheel"), true, true);
        let wheel_delta = -(wheel.notches.round() as i32);
        let delta = steps_right as i32 - steps_left as i32 + wheel_delta;
        if delta != 0 {
            let next = (start as i32 + delta).clamp(0, items.len().saturating_sub(count) as i32) as usize;
            st.thumbnails.scroll = next;
        }
        for at in 0..count {
            let i = start + at;
            let (id, unread) = items[i];
            let on_screen = shown(&ctx, st, panels.chat, id);
            let rect = square(at);
            let (wash, flashing) = st.thumbnails.flashes.mark(id, unread, now, st.thumbnails.still);
            let wash = if st.thumbnails.flash { wash } else { 0.0 };
            moving |= flashing && st.thumbnails.flash;
            let resp = thumbnail(ui, p, world, st, pics, images, id, unread, on_screen, wash, rect);
            super::conv_menu::attach(&resp, p, world, &mut st.thumbnails.actions, Some(id));
            if resp.clicked() {
                toggle(&ctx, world, st, panels, id, on_screen);
            }
        }
        for (i, &(id, unread)) in items.iter().enumerate() {
            if i < start || i >= start + count {
                st.thumbnails.flashes.mark(id, unread, now, st.thumbnails.still);
            }
        }
    } else {
        st.thumbnails.scroll_into = None;
        let (in_sight, rest) = arrange(&items, plain.max(1).min(items.len().max(1)));
        let square = |at: usize| slot.square(at);
        for (at, &i) in in_sight.iter().enumerate() {
            let (id, unread) = items[i];
            let on_screen = shown(&ctx, st, panels.chat, id);
            let rect = square(at);
            let (wash, flashing) = st.thumbnails.flashes.mark(id, unread, now, st.thumbnails.still);
            let wash = if st.thumbnails.flash { wash } else { 0.0 };
            moving |= flashing && st.thumbnails.flash;
            let resp = thumbnail(ui, p, world, st, pics, images, id, unread, on_screen, wash, rect);
            super::conv_menu::attach(&resp, p, world, &mut st.thumbnails.actions, Some(id));
            if resp.clicked() {
                toggle(&ctx, world, st, panels, id, on_screen);
            }
        }
        for &i in &rest {
            let (id, unread) = items[i];
            st.thumbnails.flashes.mark(id, unread, now, st.thumbnails.still);
        }
        if !rest.is_empty() {
            let rect = square(in_sight.len());
            if let Some(id) = more(ui, p, world, &items, &rest, rect) {
                toggle(&ctx, world, st, panels, id, false);
            }
        }
    }
    moving
}

/// One scroll arrow of the chiclet strip (LLChicletPanel). Returns how many
/// steps this frame (click, or hold after `held_down_delay` every 50 ms).
/// `unread` is the sum of counters behind that arrow (shown as the bar count).
fn scroll_arrow(ui: &mut egui::Ui, p: &Palette, rect: Rect, icon: &str, enabled: bool, unread: usize, tip: impl Into<String>) -> u32 {
    let resp = ui.interact(rect, egui::Id::unique(("conversation_thumbnails_scroll", icon)), Sense::click());
    let hold_id = resp.id.with("thumb_scroll_hold");
    islands::slot_face(ui, p, rect, resp.hovered() && enabled, false);
    if let Some(t) = crate::ui::icons::global(icon) {
        let ink = if !enabled {
            p.muted_dim
        } else if resp.hovered() {
            p.ink
        } else {
            p.muted
        };
        let size = (rect.width() * 0.7).min(rect.height() * 0.7);
        let r = Rect::from_center_size(rect.center(), Vec2::splat(size));
        ui.painter()
            .image(t.id(), r, Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)), ink);
    }
    if unread > 0 {
        // badge above the caret so it does not cover the glyph
        let lift = (rect.height() * 0.4).round().max(6.0);
        let badge = Rect::from_min_max(Pos2::new(rect.left(), rect.top() - lift), rect.right_bottom());
        islands::count(ui, p, badge, unread, 1.0, false);
    }
    let resp = resp.on_hover_text(tip.into());
    if !enabled {
        ui.data_mut(|d| d.remove::<(f64, u32)>(hold_id));
        return 0;
    }
    // LLChicletPanel::onLeftScrollHeldDown / onRightScrollHeldDown
    let now = ui.input(|i| i.time);
    let pointer_here = ui.input(|i| i.pointer.interact_pos()).is_some_and(|at| rect.contains(at));
    let primary_down = ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
    let tracked = ui.data(|d| d.get_temp::<(f64, u32)>(hold_id));
    let holding = primary_down && pointer_here && (resp.is_pointer_button_down_on() || tracked.is_some());
    let mut steps = 0u32;
    if holding {
        let (since, emitted) = tracked.unwrap_or((now, 0));
        let due = crate::ui::widgets::arrow_hold_repeats(now - since);
        steps = due.saturating_sub(emitted).min(20);
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
        ui.data_mut(|d| d.insert_temp(hold_id, (since, due)));
    } else if tracked.is_some() {
        ui.data_mut(|d| d.remove::<(f64, u32)>(hold_id));
        if pointer_here && !primary_down {
            steps = 1;
        }
    } else if resp.clicked() {
        steps = 1;
    }
    steps
}

/// The "+N" square and its menu of conversations left out of the row.
fn more(ui: &mut egui::Ui, p: &Palette, world: &World, items: &[(Uuid, usize)], rest: &[usize], rect: Rect) -> Option<Uuid> {
    let resp = ui.interact(rect, egui::Id::unique("conversation_thumbnails_more"), Sense::click());
    islands::slot_face(ui, p, rect, resp.hovered(), false);
    let unread: usize = rest.iter().map(|&i| items[i].1).sum();
    let (at, align) = if unread > 0 {
        (rect.left_bottom() + Vec2::new(3.0, -2.0), Align2::LEFT_BOTTOM)
    } else {
        (rect.center(), Align2::CENTER_CENTER)
    };
    ui.painter().text(
        at,
        align,
        format!("+{}", rest.len()),
        egui::FontId::new(11.0, crate::ui::fonts::semibold(ui.ctx())),
        if resp.hovered() { p.ink } else { p.muted },
    );
    if unread > 0 {
        islands::count(ui, p, rect, unread, 1.0, false);
    }
    let resp = resp.on_hover_text(aurora_i18n::tr!("ui-chat-thumbnails-more", n = rest.len() as i64));
    let mut picked = None;
    let below = rect.center().y < ui.ctx().content_rect().center().y;
    let opens = if below {
        egui::RectAlign::BOTTOM_END
    } else {
        egui::RectAlign::TOP_END
    };
    crate::ui::menu::dropdown_placed(&resp, p, egui::PopupCloseBehavior::CloseOnClick, Some((opens, 8.0)), |ui| {
        for &i in rest {
            let (id, n) = items[i];
            let kind = sidebar::kind(world, &id);
            let name = sidebar::entry_name(world, &id, kind);
            let icon = match kind {
                sidebar::Kind::Private => "user-filled",
                sidebar::Kind::Group => "users-group",
                sidebar::Kind::Conference => "users",
            };
            let label = if n > 0 {
                aurora_i18n::tr!("ui-bars-unread", value0 = name, unread = n.to_string()).to_string()
            } else {
                name
            };
            if crate::ui::menu::item(ui, p, icon, &label) {
                picked = Some(id);
            }
        }
    });
    picked
}

/// One thumbnail at `rect`, its picture, flash and counter.
#[allow(clippy::too_many_arguments)]
fn thumbnail(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &World,
    st: &mut ChatUi,
    pics: &HashMap<Uuid, egui::TextureHandle>,
    images: &HashMap<Uuid, egui::TextureHandle>,
    id: Uuid,
    unread: usize,
    on_screen: bool,
    wash: f32,
    rect: Rect,
) -> egui::Response {
    let resp = ui.interact(rect, egui::Id::unique(("conversation_thumbnail", id)), Sense::click());
    islands::slot_face(ui, p, rect, resp.hovered(), on_screen);
    if wash > 0.0 {
        crate::ui::widgets::attention_mark(ui.painter(), p, rect, ui.visuals().window_corner_radius, wash);
    }
    let kind = sidebar::kind(world, &id);
    let name = sidebar::entry_name(world, &id, kind);
    let icon = match kind {
        sidebar::Kind::Private => {
            st.wanted_pics.insert(id);
            sidebar::Icon::Avatar(pics.get(&id), presence::of(world, &id), world.social.is_typing(&id))
        }
        sidebar::Kind::Group => {
            let insignia = sidebar::group_insignia(world, &id);
            if let Some(i) = insignia {
                st.wanted_images.insert(i);
            }
            sidebar::Icon::Group(insignia.and_then(|i| images.get(&i)))
        }
        sidebar::Kind::Conference => sidebar::Icon::Glyph("users", p.muted),
    };
    // a call rings there: the picture pulses, as its entry in the list
    if world.calls.invite_for(&id).is_some() {
        let pulse = ((ui.input(|i| i.time) as f32) * 3.0).sin().abs();
        ui.painter().rect_stroke(
            rect.expand(1.0 + pulse * 2.0),
            ui.visuals().window_corner_radius,
            egui::Stroke::new(2.0, p.success.gamma_multiply(1.0 - pulse * 0.7)),
            egui::StrokeKind::Outside,
        );
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
    }
    let inset = slot_inset(rect.width()).max(1.0);
    sidebar::paint_picture(ui, p, &icon, &name, rect.shrink(inset), p.bar);
    if unread > 0 {
        islands::count(ui, p, rect, unread, 1.0, false);
    }
    let tip = if unread > 0 {
        aurora_i18n::tr!("ui-bars-unread", value0 = name.clone(), unread = unread.to_string()).to_string()
    } else {
        name
    };
    resp.on_hover_text(tip)
}

/// Room between a thumbnail's face and its picture: the flash's outline
/// shows around it.
fn slot_inset(size: f32) -> f32 {
    (size * 0.1).round()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertical_slots_keep_square_icons_in_order_and_reserve_the_overflow_space() {
        let slot = Slot {
            rect: Rect::from_min_size(Pos2::new(10.0, 40.0), Vec2::new(120.0, row_width(4, 28.0, 4.0))),
            size: 28.0,
            island: false,
            layer: None,
            clip: None,
            vertical: true,
        };
        let (shown, rest) = arrange(&items(&[0, 0, 0, 0, 3, 1]), fit(slot.rect.height(), slot.size, slot.padding()));
        assert_eq!(shown, [0, 4, 5]);
        assert_eq!(rest, [1, 2, 3]);
        for index in 0..4 {
            let rect = slot.square(index);
            assert_eq!(rect.size(), Vec2::splat(28.0));
            assert_eq!(rect.center().x, slot.rect.center().x);
            assert!(slot.rect.contains_rect(rect));
            if index > 0 {
                assert!(rect.top() > slot.square(index - 1).bottom());
            }
        }
        let row = Slot {
            rect: Rect::from_min_size(slot.rect.min, Vec2::new(slot.rect.height(), 36.0)),
            vertical: false,
            ..slot
        };
        assert_eq!(row.square(0).center().y, row.square(1).center().y);
        assert!(row.square(1).left() > row.square(0).right());
    }

    fn items(unread: &[usize]) -> Vec<(Uuid, usize)> {
        unread
            .iter()
            .enumerate()
            .map(|(i, &n)| (Uuid::from_u128(i as u128 + 1), n))
            .collect()
    }

    #[test]
    fn a_row_holds_whole_squares_between_its_pads() {
        assert_eq!(fit(row_width(3, 28.0, 4.0), 28.0, 4.0), 3);
        assert_eq!(fit(row_width(3, 28.0, 4.0) - 1.0, 28.0, 4.0), 2);
        assert_eq!(fit(10.0, 28.0, 4.0), 0);
        assert_eq!(row_width(1, 22.0, 0.0), 22.0);
    }

    /// Everything fits: arrange keeps list order. Overflow without arrow
    /// room: "+N" with unread in sight.
    #[test]
    fn the_unread_conversations_stay_in_sight_when_the_row_is_full() {
        assert_eq!(arrange(&items(&[0, 2, 0]), 3), (vec![0, 1, 2], vec![]));
        assert_eq!(arrange(&items(&[0, 0, 0, 5, 1]), 3), (vec![3, 4], vec![0, 1, 2]));
        assert_eq!(arrange(&items(&[1, 0, 2, 0, 0]), 3), (vec![0, 2], vec![1, 3, 4]));
        assert_eq!(arrange(&items(&[0, 0, 0, 5]), 3), (vec![0, 3], vec![1, 2]));
        assert_eq!(arrange(&items(&[1, 1]), 1), (vec![], vec![0, 1]));
    }

    /// Arrows: scroll_fit squares from `scroll` (LLChicletPanel).
    #[test]
    fn the_row_scrolls_in_list_order_when_full() {
        assert_eq!(window(5, 1, 0), (0, 1));
        assert_eq!(window(5, 1, 2), (2, 1));
        assert_eq!(window(5, 1, 99), (4, 1));
        assert_eq!(window(5, 2, 1), (1, 2));
        assert!(scroll_capacity(scroll_row_width(1, 22.0, 0.0), 22.0, 0.0) >= 1);
    }

    #[test]
    fn the_bar_keeps_the_strip_in_the_gap_or_falls_back_only_without_one_square() {
        let mut t = Thumbnails {
            place: ChatThumbnails::ChatBar,
            ..Default::default()
        };
        assert_eq!(t.bar_width(22.0, 0.0, 500.0), 0.0);
        t.items = items(&[0, 1]);
        assert_eq!(t.bar_width(22.0, 0.0, 500.0), row_width(2, 22.0, 0.0));
        // one conversation in a one-square room: keep it; overflow: lone "+N"
        t.items = items(&[0]);
        assert_eq!(t.bar_width(22.0, 0.0, 30.0), 22.0);
        t.items = items(&[0, 1]);
        assert_eq!(scroll_capacity(30.0, 22.0, 0.0), 0);
        assert_eq!(t.bar_width(22.0, 0.0, 30.0), 22.0);
        // centred-gap probe: that lone "+N" is no room (dock fallback)
        assert_eq!(t.centred_gap_width(22.0, 0.0, 30.0), 0.0);
        // many conversations, room for two squares but not arrows: "+N" width
        t.items = items(&[0; 5]);
        let room = row_width(2, 22.0, 0.0);
        assert_eq!(scroll_capacity(room, 22.0, 0.0), 0);
        assert_eq!(t.bar_width(22.0, 0.0, room), room);
        // room for arrows and one square: at least a plain row (not the whole gap)
        let arrow_room = scroll_row_width(2, 22.0, 0.0) + 40.0;
        assert!(scroll_capacity(arrow_room, 22.0, 0.0) >= 1);
        let plain = fit(arrow_room, 22.0, 0.0);
        let reserved = t.bar_width(22.0, 0.0, arrow_room);
        assert!(reserved > 0.0 && reserved < arrow_room);
        assert_eq!(
            reserved,
            row_width(plain, 22.0, 0.0).max(scroll_row_width(scroll_capacity(arrow_room, 22.0, 0.0), 22.0, 0.0))
        );
        assert_eq!(t.bar_width(22.0, 0.0, 10.0), 0.0);
        assert_eq!(top_width(5, 22.0, 0.0, 30.0), 22.0);
        assert_eq!(top_width(5, 22.0, 0.0, room), room);
        // overflow must not shrink below a plain row (dock would move left)
        let fit_room = row_width(7, 22.0, 0.0);
        assert!(scroll_capacity(fit_room, 22.0, 0.0) >= 1);
        t.items = items(&[0; 7]);
        let seven = t.bar_width(22.0, 0.0, fit_room);
        t.items = items(&[0; 8]);
        let eight = t.bar_width(22.0, 0.0, fit_room);
        assert!(eight + 0.5 >= seven, "7 → {seven}, 8 → {eight}");
        t.place = ChatThumbnails::TopRight;
        assert_eq!(t.bar_width(22.0, 0.0, 500.0), 0.0);
    }

    #[test]
    fn scroll_to_brings_a_new_conversation_into_the_window() {
        let mut scroll = 0usize;
        scroll_to(&mut scroll, 4, 2);
        assert_eq!(scroll, 3);
        scroll_to(&mut scroll, 1, 2);
        assert_eq!(scroll, 1);
    }
}
