//! Conversations in their own windows (#717): a conversation popped out of
//! the Conversations window and docked back into it, as a tab of the Legacy
//! Viewer's IM container torn off (LLFloater::onClickTearOff, indra/llui/
//! llfloater.cpp; FSFloaterIM torn off, fsfloaterim.cpp), and the option
//! that opens the conversations asked for in their own window, its separate
//! IM windows (FSChatWindow 0, the tabbed window its default).
//!
//! - Popped out, a conversation stays in the column with a mark; its entry
//!   raises its window. The Conversations window shows another one.
//! - The window's cross closes the conversation, as the cross of a torn-off
//!   IM window does; its title button docks it back.
//! - A popped-out window is a conversation in sight like a shown tab
//!   (FSFloaterIM::getVisible: a torn-off floater is visible on its own):
//!   its lines are read while it is drawn, not minimized.
//! - Not restored at the next login. While it lives, the window keeps its
//!   place; a new one opens where the last one was, cascaded from the
//!   windows already open (floater_fs_im_session.xml, save_rect).
//!
//! Each window draws the pane of the Conversations window (the workspace's
//! or the tabs'): what that pane keeps for its window (the conversation
//! shown, the keyboard, the selection, the emoji picker…) is swapped into
//! `ChatUi` while it draws (`View`).

use super::{ChatUi, ConvAction, select, typing, workspace};
use crate::theme::Palette;
use crate::world::World;
use std::collections::HashMap;
use uuid::Uuid;

/// The Legacy Viewer's IM window (floater_fs_im_session.xml): 394 × 332,
/// 250 × 150 at the least.
const DEFAULT_SIZE: egui::Vec2 = egui::vec2(394.0, 332.0);
const MIN_SIZE: egui::Vec2 = egui::vec2(250.0, 150.0);

/// What a window of conversations keeps for itself: the Conversations
/// window in `ChatUi`, each window of its own here, swapped into `ChatUi`
/// while it draws.
#[derive(Default)]
pub(super) struct View {
    selected: Option<Uuid>,
    contacts: bool,
    reveal: Option<Uuid>,
    conv_focus_request: bool,
    selection: (Option<Uuid>, select::Selection),
    emoji_open: bool,
    gif_open: bool,
    show_profile: bool,
    members_open: bool,
    stage_mini: bool,
    layout_rail: Option<bool>,
    layout_members_room: Option<bool>,
    layout_width: Option<f32>,
    layout_dragged: bool,
    typing_field: Option<typing::Field>,
}

impl View {
    fn swap(&mut self, st: &mut ChatUi) {
        std::mem::swap(&mut self.selected, &mut st.selected);
        std::mem::swap(&mut self.contacts, &mut st.contacts);
        std::mem::swap(&mut self.reveal, &mut st.reveal);
        std::mem::swap(&mut self.conv_focus_request, &mut st.conv_focus_request);
        std::mem::swap(&mut self.selection, &mut st.selection);
        std::mem::swap(&mut self.emoji_open, &mut st.emoji_open);
        std::mem::swap(&mut self.gif_open, &mut st.gif_open);
        std::mem::swap(&mut self.show_profile, &mut st.show_profile);
        std::mem::swap(&mut self.members_open, &mut st.members_open);
        std::mem::swap(&mut self.stage_mini, &mut st.stage_mini);
        std::mem::swap(&mut self.layout_rail, &mut st.layout_rail);
        std::mem::swap(&mut self.layout_members_room, &mut st.layout_members_room);
        std::mem::swap(&mut self.layout_width, &mut st.layout_width);
        std::mem::swap(&mut self.layout_dragged, &mut st.layout_dragged);
        std::mem::swap(&mut self.typing_field, &mut st.typing_field);
    }
}

/// A conversation in its own window.
struct Window {
    /// None: the nearby chat.
    conversation: Option<Uuid>,
    /// Its floater's id, `conversation:<uuid>` or `conversation:nearby`.
    floater: String,
    view: View,
    /// Bring it to the front, expanded, on its next frame.
    raise: bool,
    /// Not drawn this frame (`Popped::hold`).
    held: bool,
}

/// The conversations in their own windows, in the order they were popped out.
#[derive(Default)]
pub(crate) struct Popped {
    windows: Vec<Window>,
}

impl Popped {
    pub(super) fn contains(&self, conversation: Option<Uuid>) -> bool {
        self.windows.iter().any(|w| w.conversation == conversation)
    }

    /// The conversations in their own windows.
    pub(super) fn conversations(&self) -> impl Iterator<Item = Option<Uuid>> + '_ {
        self.windows.iter().map(|w| w.conversation)
    }

    /// Pop `conversation` out into its own window, or raise its window when
    /// it has one. `members_open` and `stage_mini`: how the Conversations
    /// window showed its members and its call, which the new window keeps;
    /// `focus`: its field takes the keyboard.
    pub(super) fn pop_out(&mut self, conversation: Option<Uuid>, members_open: bool, stage_mini: bool, focus: bool) {
        if let Some(w) = self.windows.iter_mut().find(|w| w.conversation == conversation) {
            w.raise = true;
            w.view.conv_focus_request |= focus;
            return;
        }
        self.windows.push(Window {
            conversation,
            floater: floater_id(conversation),
            view: View {
                selected: conversation,
                members_open,
                stage_mini,
                conv_focus_request: focus,
                ..View::default()
            },
            raise: true,
            held: false,
        });
    }

    /// The window of `conversation` waits for the next frame to draw: the
    /// Conversations window drew that conversation in this one (one field,
    /// one selection in a frame).
    pub(super) fn hold(&mut self, conversation: Option<Uuid>) {
        if let Some(w) = self.windows.iter_mut().find(|w| w.conversation == conversation) {
            w.held = true;
        }
    }

    /// Raise the window of `conversation`; false when it has none.
    pub(super) fn raise(&mut self, conversation: Option<Uuid>, focus: bool) -> bool {
        match self.windows.iter_mut().find(|w| w.conversation == conversation) {
            Some(w) => {
                w.raise = true;
                w.view.conv_focus_request |= focus;
                true
            }
            None => false,
        }
    }

    /// Close the window of `conversation`, whether it was docked back or the
    /// conversation closed; false when it had none.
    pub(super) fn remove(&mut self, conversation: Option<Uuid>) -> bool {
        let before = self.windows.len();
        self.windows.retain(|w| w.conversation != conversation);
        self.windows.len() != before
    }
}

thread_local! {
    /// The window drawing (`window`).
    static WINDOW: std::cell::Cell<Option<Option<Uuid>>> = const { std::cell::Cell::new(None) };
}

/// A window of conversations: None for the Conversations window, the
/// conversation of a window of its own.
pub(super) type WindowKey = Option<Option<Uuid>>;

/// The window being drawn: None for the Conversations window, the
/// conversation of a window of its own while it draws. The ids of the
/// pane's controls take it (`sidebar::control_id`, `classic_id`): the same
/// conversation pane drawn in two windows in a frame never uses an id twice.
pub(super) fn window() -> WindowKey {
    WINDOW.with(std::cell::Cell::get)
}

/// One picker open at a time, emoji or GIF: the windows share the
/// pickers' state (`emoji::Emoji`: its search, its tab, its field; the GIF
/// picker's search and tiles). Before the frame, the picker opened last
/// stays and the others close; with none left, the GIF picker frees its
/// tiles.
pub(super) fn one_picker(st: &mut ChatUi) {
    let open: Vec<WindowKey> = (st.emoji_open || st.gif_open)
        .then_some(None)
        .into_iter()
        .chain(
            st.popped
                .windows
                .iter()
                .filter(|w| w.view.emoji_open || w.view.gif_open)
                .map(|w| Some(w.conversation)),
        )
        .collect();
    let keep = open
        .iter()
        .copied()
        .rfind(|w| Some(*w) != st.picker_window)
        .or_else(|| open.first().copied());
    for w in open.into_iter().filter(|w| Some(*w) != keep) {
        match w {
            None => {
                st.emoji_open = false;
                st.gif_open = false;
            }
            Some(c) => {
                if let Some(other) = st.popped.windows.iter_mut().find(|o| o.conversation == c) {
                    other.view.emoji_open = false;
                    other.view.gif_open = false;
                }
            }
        }
    }
    st.picker_window = keep;
    let gif_shown = st.gif_open || st.popped.windows.iter().any(|w| w.view.gif_open);
    if !gif_shown {
        st.gifs.close();
    }
}

/// The emoji picker may draw in this frame: none drew yet. One opened by
/// a click while another showed waits for the next frame, when
/// `one_picker` has closed the other.
pub(super) fn claim_picker(ctx: &egui::Context) -> bool {
    let id = egui::Id::unique("conversation_emoji_picker_frame");
    let frame = ctx.cumulative_pass_nr();
    let free = ctx.data(|d| d.get_temp::<u64>(id)) != Some(frame);
    if free {
        ctx.data_mut(|d| d.insert_temp(id, frame));
    }
    free
}

/// `conversation:<uuid>`, or `conversation:nearby` for the nearby chat.
pub(super) fn floater_id(conversation: Option<Uuid>) -> String {
    match conversation {
        Some(id) => format!("conversation:{id}"),
        None => "conversation:nearby".to_owned(),
    }
}

/// Where a conversation asked for (`Social::focus_im`: a name's menu, a
/// toast, the bell, a call) shows: in its window when it has one, in a new
/// window of its own with the option on (the nearby chat excepted), in the
/// Conversations window otherwise.
pub(super) fn opens_in_own_window(popped: &Popped, conversation: Option<Uuid>, own_windows: bool) -> bool {
    popped.contains(conversation) || (own_windows && conversation.is_some())
}

/// What the Conversations window shows, given `selected` and the order of
/// its entries: never a conversation in its own window. The next entry
/// that has none; with none left, the nearby chat comes back into it.
pub(super) fn main_selection(popped: &mut Popped, selected: Option<Uuid>, order: &[Option<Uuid>]) -> Option<Uuid> {
    if !popped.contains(selected) {
        return selected;
    }
    let at = order.iter().position(|c| *c == selected).unwrap_or(0);
    let next = order
        .iter()
        .skip(at + 1)
        .chain(order.iter().take(at))
        .copied()
        .find(|c| !popped.contains(*c));
    match next {
        Some(c) => c,
        None => {
            popped.remove(None);
            None
        }
    }
}

/// The popped-out conversations, read by the column's menus while the
/// Conversations window draws (`conv_menu.rs`).
fn shared_id() -> egui::Id {
    egui::Id::unique("conversations_popped_out")
}

pub(super) fn share(ctx: &egui::Context, popped: &Popped) {
    let list: Vec<Option<Uuid>> = popped.conversations().collect();
    ctx.data_mut(|d| d.insert_temp(shared_id(), list));
}

fn shared_contains(ctx: &egui::Context, conversation: Option<Uuid>) -> bool {
    ctx.data(|d| d.get_temp::<Vec<Option<Uuid>>>(shared_id()))
        .is_some_and(|list| list.contains(&conversation))
}

/// The row of a conversation's menu: "Detach" into its own window, or
/// "Attach to Conversations" when it has one.
pub(super) fn menu_row(ui: &mut egui::Ui, p: &Palette, actions: &mut Vec<ConvAction>, conversation: Option<Uuid>) {
    if shared_contains(ui.ctx(), conversation) {
        if crate::ui::menu::item(ui, p, "login-2", aurora_i18n::tr!("ui-chat-attach-to-conversations")) {
            actions.push(ConvAction::DockBack(conversation));
        }
    } else if crate::ui::menu::item(ui, p, "external-link", aurora_i18n::tr!("common-detach")) {
        actions.push(ConvAction::PopOut(conversation));
    }
}

/// The mark of an entry whose conversation is in its own window: the icon
/// of "Detach", in muted.
pub(super) const MARK_ICON: &str = "external-link";

/// That mark in `slot`, the place of the entry's unread count.
pub(super) fn paint_mark(ui: &egui::Ui, p: &Palette, slot: egui::Rect) {
    super::thread::paint_icon(
        ui.painter(),
        MARK_ICON,
        egui::Rect::from_center_size(slot.center(), egui::Vec2::splat(12.0)),
        p.muted,
    );
}

/// What the windows draw with.
pub(super) struct Env<'a> {
    pub p: &'a Palette,
    pub icons: &'a crate::ui::icons::Icons,
    pub emoji: &'a mut crate::ui::emoji::Emoji,
    pub pics: &'a HashMap<Uuid, egui::TextureHandle>,
    pub images: &'a HashMap<Uuid, egui::TextureHandle>,
    pub gestures: &'a crate::gestures::Manager,
    pub style: crate::settings::ChatStyle,
}

/// Draw the windows of their own. A conversation that ended loses its
/// window; a window closed by its cross closes its conversation (the nearby
/// chat docks back), its title button docks it back.
pub(super) fn show(ctx: &egui::Context, env: Env, world: &mut World, st: &mut ChatUi, actions: &mut Vec<ConvAction>) {
    let Env {
        p,
        icons,
        emoji,
        pics,
        images,
        gestures,
        style,
    } = env;
    let sessions: HashMap<Uuid, usize> = st.reorder.sessions(world).into_iter().collect();
    st.popped
        .windows
        .retain(|w| w.conversation.is_none_or(|id| sessions.contains_key(&id)));
    if st.popped.windows.is_empty() {
        return;
    }
    let own_name = world.own_name();
    let work = crate::ui::layout::work_area(ctx);
    let default = egui::Rect::from_center_size(work.center(), DEFAULT_SIZE);
    // (taken out while they draw: the panes reach `st`)
    let mut windows = std::mem::take(&mut st.popped.windows);
    let mut closed = Vec::new();
    let mut docked = Vec::new();
    for w in &mut windows {
        if std::mem::take(&mut w.held) {
            ctx.request_repaint();
            continue;
        }
        if std::mem::take(&mut w.raise) {
            let mut open = true;
            crate::ui::widgets::open_floater(ctx, &w.floater, &mut open);
            ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::unique(w.floater.as_str())));
        }
        let (title, unread) = match w.conversation {
            None => (aurora_i18n::tr!("common-nearby-chat").to_owned(), world.chat_unread),
            Some(id) => (world.session_title(&id), sessions.get(&id).copied().unwrap_or(0)),
        };
        let mut open = true;
        let mut dock = false;
        let mut floater = crate::ui::widgets::Floater::new(&w.floater, title, default.min, default.size())
            .action("login-2", aurora_i18n::tr!("ui-chat-attach-to-conversations"), &mut dock)
            .attention(unread > 0)
            .steady_size()
            .cascade(crate::ui::widgets::CascadeGroup::Conversation);
        if style.workspace() {
            floater = floater.flush_body();
        }
        floater.min_size = floater.min_size.max(MIN_SIZE);
        w.view.swap(st);
        WINDOW.with(|cell| cell.set(Some(w.conversation)));
        floater.show(ctx, p, &mut open, |ui| {
            if style.workspace() {
                let h = ui.available_height().max(MIN_SIZE.y - 40.0);
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width());
                    ui.set_height(h);
                    ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                    let compact = style == crate::settings::ChatStyle::Compact;
                    workspace::pane(ui, p, icons, emoji, pics, images, gestures, world, st, actions, compact, false, h);
                });
            } else {
                let h = (ui.available_height() - 6.0).max(140.0);
                ui.vertical(|ui| {
                    super::classic_pane(ui, p, icons, emoji, pics, gestures, world, st, actions, h, &own_name);
                });
            }
        });
        WINDOW.with(|cell| cell.set(None));
        // the keyboard of the field drawn in this window
        let field = st.typing_field.take();
        w.view.swap(st);
        if let Some(f) = field.filter(|f| f.to.is_some() || f.typed || f.sent) {
            st.typing_field = Some(f);
        } else if st.typing_field.is_none() {
            st.typing_field = field;
        }
        if dock {
            docked.push(w.conversation);
        } else if !open {
            closed.push(w.conversation);
        }
    }
    // windows popped out while these drew (none so far) come after them
    windows.append(&mut st.popped.windows);
    st.popped.windows = windows;
    for c in docked {
        st.popped.remove(c);
    }
    for c in closed {
        st.popped.remove(c);
        if let Some(id) = c {
            actions.push(ConvAction::Close(id));
        }
    }
}

/// The conversations in sight in their own windows: drawn, not minimized.
pub(super) fn in_sight<'a>(ctx: &'a egui::Context, popped: &'a Popped) -> impl Iterator<Item = Option<Uuid>> + 'a {
    popped
        .windows
        .iter()
        .filter(|w| !crate::ui::widgets::floater_minimized(ctx, &w.floater))
        .map(|w| w.conversation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ChatStyle;

    fn id(n: u128) -> Option<Uuid> {
        Some(Uuid::from_u128(n))
    }

    const JANE: Uuid = Uuid::from_u128(0xA);

    /// The Conversations window closed and Jane's conversation in its own.
    struct Kit {
        world: World,
        st: ChatUi,
        icons: Option<crate::ui::icons::Icons>,
        emoji: crate::ui::emoji::Emoji,
        contacts: crate::ui::contacts::ContactsUi,
        contacts_settings: crate::ui::contacts::ContactsSettings,
        gestures: crate::gestures::Manager,
        actions: Vec<ConvAction>,
        /// The Conversations window is open.
        main_open: bool,
    }

    impl Kit {
        fn new(style: ChatStyle) -> Kit {
            let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
            for k in 0..30 {
                world.conversation_line(
                    JANE,
                    crate::world::ChatLine {
                        time: std::time::SystemTime::now(),
                        from: "Jane".into(),
                        text: format!("line {k} {}", "long words ".repeat(k % 7)),
                        kind: crate::world::ChatKind::Im,
                        source: JANE,
                        origin: crate::world::Origin::Live,
                    },
                );
            }
            let mut st = ChatUi {
                style,
                ..ChatUi::default()
            };
            st.popped.pop_out(Some(JANE), false, false, false);
            Kit {
                world,
                st,
                icons: None,
                emoji: crate::ui::emoji::Emoji::load(),
                contacts: Default::default(),
                contacts_settings: Default::default(),
                gestures: Default::default(),
                actions: Vec::new(),
                main_open: false,
            }
        }

        fn show(&mut self, ctx: &egui::Context, p: &Palette) {
            let icons = self.icons.get_or_insert_with(|| crate::ui::icons::Icons::load(ctx, None));
            let pics = HashMap::new();
            crate::ui::close_key::begin(ctx);
            let mut open = self.main_open;
            self.actions.extend(super::super::show(
                ctx,
                p,
                icons,
                &mut self.emoji,
                &pics,
                &pics,
                &mut self.world,
                &self.gestures,
                &mut self.st,
                &mut self.contacts,
                &mut self.contacts_settings,
                false,
                None,
                &mut open,
            ));
        }
    }

    /// A conversation's own window keeps its size, its field inside it, in
    /// the workspace's style and the tabs' (`widgets::steady_size`).
    #[test]
    fn a_window_of_its_own_keeps_its_size() {
        for style in [ChatStyle::Aurora, ChatStyle::Legacy] {
            let name = Kit::new(style).world.session_title(&JANE);
            let bottom = if style.workspace() {
                aurora_i18n::tr!("ui-chat-write-to", name = name.as_str())
            } else {
                aurora_i18n::tr!("ui-chat-at", value0 = name.as_str())
            };
            crate::ui::widgets::steady_tests::assert_window_steady(
                &floater_id(Some(JANE)),
                &[egui::vec2(394.0, 332.0), egui::vec2(260.0, 200.0)],
                &bottom,
                || Kit::new(style),
                |kit, ctx, p| kit.show(ctx, p),
            );
        }
    }

    /// Ctrl+W (the window's cross) closes the conversation of the window in
    /// front; a conversation that ends elsewhere takes its window along.
    #[test]
    fn the_cross_closes_the_conversation_and_an_ended_one_loses_its_window() {
        let mut kit = Kit::new(ChatStyle::Aurora);
        let mut frames = crate::ui::widgets::elide_tests::Frames::new();
        let p = crate::theme::Theme::default().palette();
        for _ in 0..3 {
            frames.run(Vec::new(), |ui| kit.show(ui.ctx(), &p));
        }
        assert!(kit.st.popped.contains(Some(JANE)));
        let ctrl_w = vec![egui::Event::Key {
            key: egui::Key::W,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }];
        frames.run(ctrl_w, |ui| kit.show(ui.ctx(), &p));
        frames.run(Vec::new(), |ui| kit.show(ui.ctx(), &p));
        assert!(!kit.st.popped.contains(Some(JANE)), "the window closed");
        assert!(
            kit.actions.iter().any(|a| matches!(a, ConvAction::Close(id) if *id == JANE)),
            "and its conversation with it"
        );
        // popped out again, then ended from elsewhere (the column's ×)
        kit.st.popped.pop_out(Some(JANE), false, false, false);
        kit.st.forget(JANE);
        assert!(!kit.st.popped.contains(Some(JANE)));
    }

    /// Whether a frame painted egui's warning of a widget id used twice.
    fn clash(texts: &[crate::ui::widgets::elide_tests::Text]) -> bool {
        texts
            .iter()
            .any(|t| t.source.contains("Double use of") || t.source.contains("First use of"))
    }

    /// The Conversations window and a window of its own, each with its find
    /// and filter fields open, then each with the emoji picker: their
    /// fields and controls have their own ids, one picker shows at a time
    /// (the one opened last), and egui warns of no id used twice.
    #[test]
    fn two_windows_never_share_their_find_field_nor_their_emoji_picker() {
        const BOB: Uuid = Uuid::from_u128(0xB);
        let p = crate::theme::Theme::default().palette();
        // the warning this test looks for does show in these frames
        let mut frames = crate::ui::widgets::elide_tests::Frames::new();
        frames.ctx.options_mut(|o| o.warn_on_id_clash = true);
        let mut texts = Vec::new();
        for _ in 0..2 {
            texts = frames.run(Vec::new(), |ui| {
                let mut a = String::new();
                let mut b = String::new();
                ui.add(egui::TextEdit::singleline(&mut a).id(egui::Id::unique("same")));
                ui.add(egui::TextEdit::singleline(&mut b).id(egui::Id::unique("same")));
            });
        }
        assert!(clash(&texts), "a clash is seen");
        for style in [ChatStyle::Aurora, ChatStyle::Legacy] {
            let mut kit = Kit::new(style);
            kit.world.conversation_line(
                BOB,
                crate::world::ChatLine {
                    time: std::time::SystemTime::now(),
                    from: "Bob".into(),
                    text: "hello from Bob".into(),
                    kind: crate::world::ChatKind::Im,
                    source: BOB,
                    origin: crate::world::Origin::Live,
                },
            );
            kit.main_open = true;
            kit.st.selected = Some(BOB);
            for c in [Some(BOB), Some(JANE)] {
                let search = kit.st.searches.entry(c).or_default();
                search.open_find();
                search.filter = Some(String::new());
            }
            let mut frames = crate::ui::widgets::elide_tests::Frames::new();
            frames.ctx.options_mut(|o| o.warn_on_id_clash = true);
            crate::ui::widgets::place_floater(&frames.ctx, "conversations", egui::pos2(500.0, 40.0), egui::vec2(700.0, 500.0));
            let run = |frames: &mut crate::ui::widgets::elide_tests::Frames, kit: &mut Kit, what: &str| {
                for _ in 0..4 {
                    let texts = frames.run(Vec::new(), |ui| kit.show(ui.ctx(), &p));
                    let clashes: Vec<_> = texts.iter().filter(|t| t.source.contains(" use of ")).map(|t| t.rect).collect();
                    assert!(!clash(&texts), "{style:?}, {what}: no id used twice: {clashes:?}");
                }
            };
            run(&mut frames, &mut kit, "find fields");
            let field = |c: Option<Uuid>| frames.ctx.read_response(super::super::find::field_id(c)).map(|r| r.rect);
            let (bob, jane) = (field(Some(BOB)), field(Some(JANE)));
            assert!(
                bob.is_some() && jane.is_some(),
                "{style:?}: both find fields drawn: {bob:?} {jane:?}"
            );
            assert_ne!(bob, jane, "{style:?}: in their own windows");
            // both pickers asked: the one of the window opened last stays
            kit.st.emoji_open = true;
            kit.st.popped.windows[0].view.emoji_open = true;
            run(&mut frames, &mut kit, "both pickers asked");
            assert!(!kit.st.emoji_open && kit.st.popped.windows[0].view.emoji_open, "{style:?}");
            // the Conversations window's picker opened after: the other closes
            kit.st.emoji_open = true;
            run(&mut frames, &mut kit, "picker moved");
            assert!(kit.st.emoji_open && !kit.st.popped.windows[0].view.emoji_open, "{style:?}");
            assert!(kit.st.selected == Some(BOB) && kit.st.popped.contains(Some(JANE)));
        }
    }

    /// "Detach" waits a frame and gives the field the keyboard; "Attach to
    /// Conversations" shows the conversation there, the window opened.
    #[test]
    fn a_conversation_detaches_and_attaches_back() {
        let mut st = ChatUi {
            selected: Some(JANE),
            ..ChatUi::default()
        };
        st.pop_out(Some(JANE));
        let w = &st.popped.windows[0];
        assert!(w.held && w.view.conv_focus_request);
        let ctx = egui::Context::default();
        let mut open = false;
        st.selected = None;
        st.dock_back(&ctx, Some(JANE), &mut open);
        assert!(open, "the Conversations window opens");
        assert!(!st.popped.contains(Some(JANE)));
        assert_eq!(st.selected, Some(JANE));
        assert!(st.conv_focus_request);
        // a conversation without a window: nothing moves
        let mut open = false;
        st.dock_back(&ctx, Some(Uuid::from_u128(0xB)), &mut open);
        assert!(!open && st.selected == Some(JANE));
    }

    #[test]
    fn a_conversation_pops_out_once_and_raises_its_window_after() {
        let mut popped = Popped::default();
        popped.pop_out(id(1), true, false, true);
        assert!(popped.contains(id(1)));
        assert_eq!(popped.windows.len(), 1);
        let w = &popped.windows[0];
        assert_eq!(w.floater, format!("conversation:{}", Uuid::from_u128(1)));
        assert_eq!(w.view.selected, id(1));
        assert!(w.view.members_open && w.view.conv_focus_request && w.raise);
        popped.windows[0].raise = false;
        popped.windows[0].view.conv_focus_request = false;
        // popped out again: the same window, raised
        popped.pop_out(id(1), false, false, false);
        assert_eq!(popped.windows.len(), 1);
        assert!(popped.windows[0].raise);
        assert!(!popped.windows[0].view.conv_focus_request);
        assert!(popped.raise(id(1), true));
        assert!(popped.windows[0].view.conv_focus_request);
        assert!(!popped.raise(id(2), true));
    }

    #[test]
    fn a_window_docks_back_or_closes_with_its_conversation() {
        let mut popped = Popped::default();
        popped.pop_out(id(1), false, false, false);
        popped.pop_out(None, false, false, false);
        assert_eq!(floater_id(None), "conversation:nearby");
        assert!(popped.remove(id(1)));
        assert!(!popped.remove(id(1)));
        assert!(!popped.contains(id(1)));
        assert!(popped.contains(None));
    }

    #[test]
    fn the_option_opens_conversations_in_their_own_window_but_not_the_nearby_chat() {
        let mut popped = Popped::default();
        assert!(!opens_in_own_window(&popped, id(1), false));
        assert!(opens_in_own_window(&popped, id(1), true));
        assert!(!opens_in_own_window(&popped, None, true));
        // a conversation with its window shows there whatever the option
        popped.pop_out(id(2), false, false, false);
        popped.pop_out(None, false, false, false);
        assert!(opens_in_own_window(&popped, id(2), false));
        assert!(opens_in_own_window(&popped, None, false));
    }

    #[test]
    fn the_conversations_window_shows_the_next_conversation_without_a_window() {
        let order = [None, id(1), id(2), id(3)];
        let mut popped = Popped::default();
        assert_eq!(main_selection(&mut popped, id(2), &order), id(2));
        popped.pop_out(id(2), false, false, false);
        assert_eq!(main_selection(&mut popped, id(2), &order), id(3));
        popped.pop_out(id(3), false, false, false);
        // past the end, from the start
        assert_eq!(main_selection(&mut popped, id(3), &order), None);
        popped.pop_out(None, false, false, false);
        assert_eq!(main_selection(&mut popped, None, &order), id(1));
        // every entry in its own window: the nearby chat comes back
        popped.pop_out(id(1), false, false, false);
        assert_eq!(main_selection(&mut popped, id(1), &order), None);
        assert!(!popped.contains(None));
        assert!(popped.contains(id(1)));
    }
}
