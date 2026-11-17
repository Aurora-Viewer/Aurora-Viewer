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
//! fit, left and right scroll arrows page through them in list order, as
//! LLChicletPanel's `chicklet_left_scroll_button` /
//! `chicklet_right_scroll_button` (#3552). Without room beside the chat bar
//! (a dock aligned against the chat island, a narrow bar) they go to the
//! top right.

use super::{ChatUi, ConvAction, presence, sidebar};
use crate::settings::ChatThumbnails;
use crate::theme::Palette;
use crate::ui::Panels;
use crate::ui::bars::islands;
use crate::ui::icons::Icons;
use crate::world::World;
use egui::{Pos2, Rect, Sense, Vec2};
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
}

impl Thumbnails {
    /// The thumbnails beside the chat bar: the room they take, a row of
    /// `size` squares with `pad` at each end, at most `room` (0: none, or
    /// no room for one, they go to the top right). When more conversations
    /// are open than fit, the strip fills `room` so the scroll arrows sit
    /// at its ends (LLChicletPanel).
    pub fn bar_width(&self, size: f32, pad: f32, room: f32) -> f32 {
        if self.place != ChatThumbnails::ChatBar || self.items.is_empty() {
            return 0.0;
        }
        let fits = fit(room, size, pad);
        if fits == 0 {
            return 0.0;
        }
        if self.items.len() <= fits {
            return row_width(self.items.len(), size, pad);
        }
        // overflow: need room for both arrows and at least one square
        if scroll_capacity(room, size, pad) == 0 {
            return 0.0;
        }
        room
    }

    /// The bottom bar left them `rect` (`bar_width` wide).
    pub fn reserve(&mut self, rect: Rect, size: f32, island: bool) {
        if rect.width() > 0.0 {
            self.slot = Some(Slot { rect, size, island });
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
/// within `room` (0: none): the top bars' room for them.
pub fn top_width(n: usize, size: f32, pad: f32, room: f32) -> f32 {
    match fit(room, size, pad).min(n) {
        0 => 0.0,
        n => row_width(n, size, pad),
    }
}

fn top_id() -> egui::Id {
    egui::Id::unique("conversation_thumbnails_top")
}

/// The top bars left the thumbnails `rect` this frame: an island of the
/// top row, or room in the Legacy Viewer's navigation bar.
pub fn reserve_top(ctx: &egui::Context, rect: Rect, size: f32, island: bool) {
    // (the frame read outside: the context is locked within `data_mut`)
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|d| d.insert_temp(top_id(), (frame, Slot { rect, size, island })));
}

/// What `reserve_top` left this frame.
fn top_slot(ctx: &egui::Context) -> Option<Slot> {
    let frame = ctx.cumulative_frame_nr();
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
/// place (0: not enough room to scroll).
fn scroll_capacity(room: f32, size: f32, pad: f32) -> usize {
    let arrows = 2.0 * arrow_width(size) + 2.0 * GAP;
    fit((room - arrows).max(0.0), size, pad)
}

/// The window of `n` conversations at `scroll`: all of them when
/// `plain_fit` holds them; else `scroll_fit` squares between the arrows
/// (LLChicletPanel). Returns `(start, count, arrows)`.
fn window(n: usize, plain_fit: usize, scroll_fit: usize, scroll: usize) -> (usize, usize, bool) {
    if n <= plain_fit {
        return (0, n, false);
    }
    if scroll_fit == 0 {
        // too narrow for arrows and a square: show what fits from `scroll`
        let start = scroll.min(n.saturating_sub(plain_fit.max(1)));
        return (start, plain_fit.min(n), false);
    }
    let start = scroll.min(n.saturating_sub(scroll_fit));
    (start, scroll_fit, true)
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
    t.items = if place == ChatThumbnails::Off {
        Vec::new()
    } else {
        st.reorder.sessions(world)
    };
    if t.items.is_empty() {
        t.scroll = 0;
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
    icons: &Icons,
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
    if t.items.is_empty() {
        return;
    }
    let slot = match (t.place, t.slot, top_slot(&ctx)) {
        (ChatThumbnails::Off, _, _) => return,
        (ChatThumbnails::ChatBar, Some(slot), _) => slot,
        (ChatThumbnails::TopRight, _, Some(slot)) => slot,
        // the chat bar without room for one: the top right of the windows'
        // area
        _ if !top => return,
        _ => {
            let size = islands::SLOT;
            let height = islands::HEIGHT;
            let pad = (height - size) / 2.0;
            let n = fit(work.width() / 2.0, size, pad).min(t.items.len());
            if n == 0 {
                return;
            }
            let width = row_width(n, size, pad);
            let rect = Rect::from_min_size(Pos2::new(work.right() - width, work.top()), Vec2::new(width, height));
            Slot { rect, size, island: true }
        }
    };
    let now = ctx.input(|i| i.time);
    st.thumbnails.flashes.begin(now);
    let mut moving = false;
    if slot.island {
        islands::area(&ctx, p, "conversation_thumbnails", slot.rect, |ui| {
            moving = row(ui, p, icons, world, st, panels, pics, images, slot, now);
        });
    } else {
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(slot.rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        // the counters overhang the bar's thumbnails
        child.set_clip_rect(slot.rect.expand(8.0));
        moving = row(&mut child, p, icons, world, st, panels, pics, images, slot, now);
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
    icons: &Icons,
    world: &mut World,
    st: &mut ChatUi,
    panels: &mut Panels,
    pics: &HashMap<Uuid, egui::TextureHandle>,
    images: &HashMap<Uuid, egui::TextureHandle>,
    slot: Slot,
    now: f64,
) -> bool {
    let ctx = ui.ctx().clone();
    let pad = (slot.rect.height() - slot.size) / 2.0;
    let plain = fit(slot.rect.width(), slot.size, pad);
    let scrolled = scroll_capacity(slot.rect.width(), slot.size, pad);
    // what is unread in a conversation on screen is being read
    let items: Vec<(Uuid, usize)> = st
        .thumbnails
        .items
        .iter()
        .map(|&(id, n)| (id, if shown(&ctx, st, panels.chat, id) { 0 } else { n }))
        .collect();
    let (start, count, arrows) = window(items.len(), plain, scrolled, st.thumbnails.scroll);
    st.thumbnails.scroll = start;
    let aw = arrow_width(slot.size);
    let content_left = if arrows {
        slot.rect.left() + pad + aw + GAP
    } else {
        slot.rect.left() + pad
    };
    let square = |at: usize| {
        Rect::from_min_size(
            Pos2::new(content_left + at as f32 * (slot.size + GAP), slot.rect.top() + pad),
            Vec2::splat(slot.size),
        )
    };
    let mut moving = false;
    if arrows {
        let left = Rect::from_min_size(Pos2::new(slot.rect.left() + pad, slot.rect.top() + pad), Vec2::new(aw, slot.size));
        let right = Rect::from_min_size(
            Pos2::new(slot.rect.right() - pad - aw, slot.rect.top() + pad),
            Vec2::new(aw, slot.size),
        );
        let can_left = start > 0;
        let can_right = start + count < items.len();
        if scroll_arrow(ui, p, icons, left, "chevron-left", can_left, aurora_i18n::tr!("common-previous")) {
            st.thumbnails.scroll = start.saturating_sub(1);
        }
        if scroll_arrow(ui, p, icons, right, "chevron-right", can_right, aurora_i18n::tr!("common-next")) {
            st.thumbnails.scroll = (start + 1).min(items.len().saturating_sub(count));
        }
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
    // conversations out of the window keep their flash's timing
    for (i, &(id, unread)) in items.iter().enumerate() {
        if i < start || i >= start + count {
            st.thumbnails.flashes.mark(id, unread, now, st.thumbnails.still);
        }
    }
    moving
}

/// One scroll arrow of the chiclet strip (LLChicletPanel).
fn scroll_arrow(ui: &mut egui::Ui, p: &Palette, icons: &Icons, rect: Rect, icon: &str, enabled: bool, tip: impl Into<String>) -> bool {
    let resp = ui.interact(rect, egui::Id::unique(("conversation_thumbnails_scroll", icon)), Sense::click());
    islands::slot_face(ui, p, rect, resp.hovered() && enabled, false);
    if let Some(t) = icons.get(icon) {
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
    enabled && resp.on_hover_text(tip.into()).clicked()
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

    /// Everything fits: no arrows. Too many: scroll_fit squares from
    /// `scroll` between the arrows (LLChicletPanel).
    #[test]
    fn the_row_scrolls_in_list_order_when_full() {
        assert_eq!(window(3, 3, 1, 0), (0, 3, false));
        assert_eq!(window(5, 3, 1, 0), (0, 1, true));
        assert_eq!(window(5, 3, 1, 2), (2, 1, true));
        assert_eq!(window(5, 3, 1, 99), (4, 1, true));
        assert_eq!(window(5, 4, 2, 1), (1, 2, true));
        // too narrow for arrows and a square: show what fits, no arrows
        assert_eq!(window(4, 1, 0, 2), (2, 1, false));
        assert!(scroll_capacity(row_width(3, 22.0, 0.0), 22.0, 0.0) >= 1);
    }

    #[test]
    fn the_bar_reserves_nothing_without_a_conversation_or_off() {
        let mut t = Thumbnails {
            place: ChatThumbnails::ChatBar,
            ..Default::default()
        };
        assert_eq!(t.bar_width(22.0, 0.0, 500.0), 0.0);
        t.items = items(&[0, 1]);
        assert_eq!(t.bar_width(22.0, 0.0, 500.0), row_width(2, 22.0, 0.0));
        // one square of room with many conversations: fill the room for arrows
        t.items = items(&[0; 5]);
        let room = row_width(3, 22.0, 0.0);
        assert_eq!(t.bar_width(22.0, 0.0, room), room);
        assert_eq!(t.bar_width(22.0, 0.0, 10.0), 0.0);
        t.place = ChatThumbnails::TopRight;
        assert_eq!(t.bar_width(22.0, 0.0, 500.0), 0.0);
    }
}
