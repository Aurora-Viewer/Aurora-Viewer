//! Local chat window (nearby chat + instant messages).

mod action_marker;
mod app_links;
pub(crate) use app_links::{group_name as app_link_group_name, take_wanted as take_app_link_wants};
mod bubbles;
mod call;
mod completion;
mod compose;
mod console;
mod conv_menu;
pub(crate) mod docking;
mod find;
mod flash;
pub use flash::{ButtonFlash, ButtonMenu, button_menu, nearby_flashes, nearby_out_of_sight};
mod gif_picker;
mod head;
mod header;
pub mod history;
pub(crate) mod input;
mod input_history;
mod invite_card;
mod keys;
mod latest;
mod line_menu;
mod members;
mod object_row;
mod offer_card;
mod payment_card;
mod popout;
pub mod previews;
mod reorder;
pub use reorder::Demo as ReorderDemo;
pub(crate) mod select;
mod sidebar;
pub(crate) use sidebar::group_insignia;
pub mod snooze;
mod stage;
pub mod suggest;
mod thread;
pub mod thumbnails;
mod typing;
mod view;
mod workspace;

#[cfg(test)]
mod action_marker_tests;
#[cfg(test)]
mod buttons_tests;
#[cfg(test)]
mod contact_visual_tests;

use super::context::CtxAction;
use super::presence;
use crate::theme::Palette;
use crate::world::chat_log::format::Slt;
use crate::world::{ChatKind, Origin, World};
use aurora_net::ChatType;
use egui::{Color32, RichText};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub struct ChatUi {
    pub input: String,
    pub focus_request: bool,
    /// `start_gesture` resumes a nonempty draft at its existing caret.
    gesture_keep_cursor: bool,
    /// The slash press reserved for `start_gesture`, until its release.
    gesture_key_down: Option<winit::keyboard::KeyCode>,
    pub has_focus: bool,
    /// The local chat field keeps the keyboard after a message is sent
    /// (Preferences › Chat; the Legacy Viewer's CloseChatOnReturn off).
    pub keep_focus: bool,
    /// The chat bar shows only while it is used (Preferences › Chat; the
    /// Legacy Viewer's AutohideChatBar, #1009): `bar_shown`.
    pub autohide: bool,
    /// The pointer was over the chat bar the last time it was drawn: a
    /// click on it (its emoji, its handle) does not hide it under the
    /// pointer once the field lets the keyboard go.
    pub bar_hovered: bool,
    /// Conversation shown in the floater (None = local chat).
    pub selected: Option<uuid::Uuid>,
    /// Conversation whose entry the list scrolls to: one opened from
    /// elsewhere (a name's menu, a notification) may be out of its view.
    reveal: Option<uuid::Uuid>,
    /// The "Contacts" tab is shown instead (FSFloaterContacts docked as
    /// the first tab of FSFloaterIMContainer).
    pub contacts: bool,
    /// What is being typed in each conversation's field (None = nearby
    /// chat): another tab keeps its own, like each FSFloaterIM's editor.
    drafts: HashMap<Option<uuid::Uuid>, String>,
    /// Sent text per conversation; both nearby chat fields share None.
    input_history: HashMap<Option<uuid::Uuid>, input_history::History>,
    /// The fields' mentions, gestures and names (#980): options given every
    /// frame by the app, residents around, `@Name` written.
    pub suggest: suggest::Shared,
    /// The field of the conversation shown takes the keyboard (its tab was
    /// clicked, a letter key started the nearby chat).
    pub conv_focus_request: bool,
    /// The text selected in the lines of a conversation (`select.rs`), and
    /// which conversation: another tab starts without one.
    selection: (Option<uuid::Uuid>, select::Selection),
    /// `AURORA_DEMO_CONV_SELECT`: a selection to make once lines are shown
    /// ("all", or "part" of three lines).
    pub demo_select: Option<String>,
    /// `AURORA_DEMO_CONV_MENU=local|im|group|conference`: the conversation
    /// of the list whose right-click menu the demo opens.
    pub demo_menu: Option<String>,
    /// Explicit offline pointer fixture for conversation rearrangement
    /// (`AURORA_DEMO_CONV_REORDER`).
    pub demo_reorder: Option<ReorderDemo>,
    /// `AURORA_DEMO_CONV_SCROLL`: this conversation goes to its top once it
    /// is shown with its lines laid out; `demo_scrolled` once it is there.
    pub demo_scroll: Option<uuid::Uuid>,
    pub demo_scrolled: bool,
    /// Presentation order of currently open conversations, shared by all styles.
    reorder: reorder::State,
    /// Avatars whose profile picture the conversation wants (filled while drawing).
    pub wanted_pics: HashSet<uuid::Uuid>,
    /// Textures the conversation wants by id (the groups' insignias), drawn
    /// from the app's UI images.
    pub wanted_images: HashSet<uuid::Uuid>,
    /// Names to resolve (mentions) — taken by the app.
    pub wanted_names: HashSet<uuid::Uuid>,
    /// Participants of the group / conference session shown.
    pub show_profile: bool,
    /// Filter and find bar of each conversation (None = nearby chat).
    searches: HashMap<Option<uuid::Uuid>, find::Search>,
    /// Laid-out rows of each conversation.
    views: HashMap<Option<uuid::Uuid>, view::View>,
    pub emoji_open: bool,
    /// The GIF picker of the conversation's field is open (one picker at a
    /// time: opening it closes the emoji one, and the other way round).
    pub gif_open: bool,
    /// The GIF picker's search and tiles, shared by the windows like the
    /// emoji picker's (`gif_picker.rs`); its settings given every frame by
    /// the app.
    pub gifs: gif_picker::GifPicker,
    /// The emoji picker of the chat island's field is open.
    pub bar_emoji_open: bool,
    /// The lines' style of the settings, given every frame by the app
    /// (`set_style`).
    pub style: crate::settings::ChatStyle,
    /// A demo scenario's style, which the settings do not keep, until the
    /// preferences choose another.
    pub demo_style: Option<crate::settings::ChatStyle>,
    /// `AURORA_DEMO_CONV=aurora:new`: open the menu of the column's "new
    /// conversation" button once it is drawn.
    pub demo_new_menu: bool,
    /// `AURORA_DEMO_CONV=aurora:local:volume`: open the menu of the nearby
    /// chat's volume once it is drawn.
    pub demo_volume_menu: bool,
    /// `AURORA_DEMO_CONV=aurora:local:channel`: the channel selector shown
    /// without changing the preferences.
    pub demo_channel: bool,
    /// The workspace's layout last frame (None before the first): the
    /// column as a rail, room for the members' panel. A threshold crossed
    /// back needs a margin past it (`workspace::rail_now`), so a window
    /// resized around one does not flip at every point.
    layout_rail: Option<bool>,
    layout_members_room: Option<bool>,
    /// The workspace's width last frame, and whether this frame's change of
    /// it comes from a drag of the window's edge: the layout then follows
    /// the edge at once, without easing (`workspace::ease`).
    layout_width: Option<f32>,
    layout_dragged: bool,
    /// `style` was given once: a change after it is the preferences'.
    style_given: bool,
    /// Voice is enabled (calls possible), given every frame by the app.
    pub voice_on: bool,
    pub conversation_voice_indicator: bool,
    /// The microphone in toggle mode (None: hold to talk), for the call's
    /// mute button.
    pub mic: Option<bool>,
    /// Residents' voice volumes, for the call's participants.
    pub volumes: std::collections::BTreeMap<uuid::Uuid, f32>,
    /// The times of the lines are shown (Preferences › Chat), given every
    /// frame by the app.
    pub timestamps: bool,
    /// Objects' lines take one small row (Preferences › Chat, #475, off),
    /// given every frame by the app.
    pub compact_automated: bool,
    /// A conversation's notices too (its sub-option, off), given every
    /// frame by the app.
    pub compact_im_notices: bool,
    /// The nearby chat's field shows its channel selector (Preferences ›
    /// Chat, off by default), given every frame by the app.
    pub show_channel: bool,
    /// Typing notices are sent (Preferences › Chat), given every frame by
    /// the app.
    pub send_typing: bool,
    /// "> " before the "… is typing" of a tab (FSTypingChevronPrefix,
    /// Preferences › Chat), given every frame by the app.
    pub typing_chevron: bool,
    /// The entries of conversations with news flash (Preferences › Chat),
    /// given every frame by the app; and where each flash stands.
    pub flash_unread: bool,
    flashes: flash::Flashes,
    /// The Conversations button's flash and the minimized bar's pulse,
    /// started by the app (Preferences › Chat).
    pub button_flash: flash::ButtonFlash,
    /// Nearby chat flashes the Conversations button too (Preferences ›
    /// Chat, or the button's right-click menu), given every frame by the
    /// app; and what that menu asked for this frame.
    pub flash_nearby: bool,
    pub button_menu: Option<ButtonMenu>,
    /// The thumbnails of the open conversations beside the chat bar or at
    /// the top right (`thumbnails.rs`, #1134).
    pub thumbnails: thumbnails::Thumbnails,
    /// The avatars in chat range and those of them who type, given every
    /// frame by the app (the radar) for the nearby chat.
    pub nearby_count: usize,
    pub nearby_typing: Vec<uuid::Uuid>,
    /// The audio outputs and the one in use, given by the app while a call
    /// is in progress.
    pub output_devices: Vec<String>,
    pub output_device: String,
    /// Aurora's workspace (`workspace.rs`): the members' panel of a group
    /// or a conference is open.
    pub members_open: bool,
    /// The stage of the call is reduced to its strip.
    pub stage_mini: bool,
    /// Where the "New" line of each conversation sits.
    new_lines: workspace::NewLines,
    /// The search field of the column.
    side_filter: String,
    /// The nearby chat's volume: whisper, say or shout.
    volume: compose::Volume,
    /// Our typing notices, and what the field did this frame.
    me_typing: typing::MeTyping,
    typing_field: Option<typing::Field>,
    /// An emote shows us typing in the nearby chat too (FSTypeDuringEmote,
    /// Preferences › Chat), given every frame by the app.
    pub type_during_emote: bool,
    /// ":" stands for "/me" (AllowMUpose), given every frame by the app.
    pub colon_emote: bool,
    /// Our typing in the nearby chat, and what its fields did this frame.
    local_typing: typing::LocalTyping,
    local_field: typing::LocalField,
    /// The resident picker of the column's "+", and what it starts.
    picker: super::avatar_picker::AvatarPicker,
    start: Start,
    /// The pictures and videos under links (`previews.rs`), their settings
    /// given every frame by the app.
    pub previews: previews::Previews,
    /// The duration asked before a group chat's snooze (`snooze.rs`).
    snooze: Option<snooze::Prompt>,
    /// The conversations in their own windows (`popout.rs`).
    popped: popout::Popped,
    /// The conversations asked for open in their own window (Preferences ›
    /// Chat, off), given every frame by the app.
    pub own_windows: bool,
    /// The Conversations window was open last frame.
    main_open: bool,
    main_was_minimized: bool,
    /// The window whose emoji picker is open (`popout::one_picker`).
    picker_window: Option<popout::WindowKey>,
}

/// What the column's resident picker starts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Start {
    #[default]
    Im,
    Conference,
}

/// How a conversation's lines are drawn (`LineEnv::bubbles`): legacy lines,
/// or bubbles with the sender's name opening each group where several
/// people write (a group or conference, the nearby chat).
pub(crate) fn line_style(style: crate::settings::ChatStyle, group: Option<bool>) -> Option<bool> {
    match (style, group) {
        (crate::settings::ChatStyle::Legacy, _) => None,
        (_, None) => Some(true),
        (_, Some(group)) => Some(group),
    }
}

impl ChatUi {
    /// Recreate only the account-independent nearby window. IMs, messages,
    /// drafts and focus never cross sessions. Its stable floater id lets the
    /// existing layout restore position, size and minimized state.
    pub fn from_settings(settings: &crate::settings::Settings) -> Self {
        let mut st = Self::default();
        st.popped.restore_nearby(settings.nearby_chat_window);
        st
    }

    pub fn remember_layout(&self, settings: &mut crate::settings::Settings) {
        settings.nearby_chat_window = self.popped.nearby_state();
    }

    /// `start_gesture` and FSNearbyChat::showDefaultChatBar
    /// (indra/newview/llviewerinput.cpp, fsnearbychathub.cpp): take the local
    /// bar's keyboard, adding `/` only to an empty editor. A conversation's
    /// draft stays separate, even when its nearby-chat tab is in sight.
    pub(crate) fn gesture_key(
        &mut self,
        ctx: &egui::Context,
        key: &winit::keyboard::Key,
        code: winit::keyboard::KeyCode,
        pressed: bool,
        mods: crate::keybinds::Mods,
    ) -> bool {
        let released = !pressed && self.gesture_key_down == Some(code);
        if released {
            self.gesture_key_down = None;
        }
        // A focused button or list is not a text editor. `egui_wants_keyboard_input`
        // includes those too, whereas the Legacy action checks acceptsTextInput.
        if ctx.text_edit_focused() || egui::Popup::is_any_open(ctx) || mods.ctrl || mods.alt {
            return false;
        }
        if pressed && (self.gesture_key_down == Some(code) || crate::keybinds::gesture_chat_key(key, mods)) {
            // Shift may be released before the slash key on AZERTY. Remember
            // its physical key so the changed release character still works.
            self.gesture_key_down = Some(code);
            return true;
        }
        if released {
            self.gesture_keep_cursor = !self.input.is_empty();
            if self.input.is_empty() {
                self.input.push('/');
            }
            self.focus_request = true;
        }
        released
    }

    pub(crate) fn cancel_gesture_key(&mut self) {
        self.gesture_key_down = None;
    }

    /// A demo scene's lines, given while its conversation is in sight,
    /// count as arrived while away: their "New" line shows.
    pub fn demo_lines_unseen(&mut self) {
        self.new_lines.forget();
    }

    /// Show a conversation, its entry brought into the view of the list.
    pub fn open(&mut self, conversation: uuid::Uuid) {
        self.selected = Some(conversation);
        self.reveal = Some(conversation);
        self.show_profile = false;
        self.contacts = false;
    }

    /// Ask how long to snooze a group chat, `minutes` proposed (the
    /// SnoozeDuration notification).
    pub fn ask_snooze(&mut self, group: uuid::Uuid, minutes: u32) {
        self.snooze = Some(snooze::Prompt::new(group, minutes));
    }

    /// A conversation closed: its filter, find bar, rows, draft and recall go with it.
    pub fn forget(&mut self, conversation: uuid::Uuid) {
        self.searches.remove(&Some(conversation));
        self.views.remove(&Some(conversation));
        self.drafts.remove(&Some(conversation));
        self.input_history.remove(&Some(conversation));
        self.popped.remove(Some(conversation));
    }

    /// "Detach": `conversation` in its own window, its field taking the
    /// keyboard, from the next frame (the Conversations window drew it in
    /// this one).
    fn pop_out(&mut self, conversation: Option<uuid::Uuid>) {
        self.popped.pop_out(conversation, self.members_open, self.stage_mini, true);
        self.popped.hold(conversation);
    }

    /// "Attach to Conversations": the window of `conversation` closes, and
    /// the Conversations window, opened if closed, shows it.
    fn dock_back(&mut self, ctx: &egui::Context, conversation: Option<uuid::Uuid>, open: &mut bool) {
        if self.popped.remove(conversation) {
            self.select_tab(conversation);
            self.reveal = conversation;
            super::notifications::open_conversations(ctx, open);
        }
    }

    /// The chat bar is drawn this frame: always, or with `autohide` while it
    /// is used. As the Legacy Viewer's autohidden bar (FSNearbyChat::
    /// showDefaultChatBar, FSNearbyChatControl::autohide), Enter or a letter
    /// key brings it with the keyboard, and Escape or a message sent with
    /// "Keep the chat bar active" off puts it away; a click elsewhere on an
    /// empty bar puts it away too, where the Legacy Viewer waited for
    /// Escape. A draft keeps it.
    pub fn bar_shown(&self) -> bool {
        !self.autohide || self.has_focus || self.focus_request || !self.input.trim().is_empty() || self.bar_emoji_open || self.bar_hovered
    }

    /// The nearby chat is in its own window (#910): the chat island's
    /// button shows and hides that window.
    pub fn nearby_apart(&self) -> bool {
        self.popped.contains(None)
    }

    /// The nearby chat's own window is on screen, expanded.
    pub fn nearby_window_shown(&self, ctx: &egui::Context) -> bool {
        self.popped.shown(None) && !super::widgets::floater_minimized(ctx, &popout::floater_id(None))
    }

    /// The chat island's button with the nearby chat in its own window:
    /// that window shown (expanded, in front, its field taking the
    /// keyboard) or put away.
    pub fn toggle_nearby_window(&mut self, ctx: &egui::Context) {
        if self.nearby_window_shown(ctx) {
            self.popped.set_hidden(None, true);
        } else {
            self.popped.raise(None, true);
        }
    }

    /// `AURORA_DEMO_CONV_POPOUT`: `conversation` in its own window, at `pos`.
    pub fn demo_pop_out(&mut self, ctx: &egui::Context, conversation: Option<uuid::Uuid>, pos: egui::Pos2, size: egui::Vec2) {
        self.popped.pop_out(conversation, self.members_open, self.stage_mini, false);
        demo_place_window(ctx, conversation, pos, size);
    }

    /// What is typed in a conversation's field (None = nearby chat).
    pub fn draft_mut(&mut self, conversation: Option<uuid::Uuid>) -> &mut String {
        self.drafts.entry(conversation).or_default()
    }

    /// Show a tab of the window and give its field the keyboard; a
    /// conversation in its own window raises that window instead.
    fn select_tab(&mut self, conversation: Option<uuid::Uuid>) {
        if self.popped.raise(conversation, true) {
            return;
        }
        self.selected = conversation;
        self.show_profile = false;
        self.contacts = false;
        self.conv_focus_request = true;
    }

    /// The style of the settings, every frame. A change made in the
    /// preferences shows at once, over a demo scenario's style too.
    pub fn set_style(&mut self, style: crate::settings::ChatStyle) {
        if self.style_given && self.style != style {
            self.demo_style = None;
        }
        self.style = style;
        self.style_given = true;
    }

    /// Open the resident picker of the column: one resident for an IM,
    /// several for a conference; a search typed in the column carries over.
    fn start_picker(&mut self, start: Start) {
        self.start = start;
        let query = self.side_filter.trim().to_owned();
        self.picker.open("conversations_start", start == Start::Conference);
        if !query.is_empty() {
            self.picker.set_query(&query);
        }
    }

    /// The names of the avatars in chat range who type.
    fn nearby_typing_names(&self, world: &World) -> Vec<String> {
        self.nearby_typing.iter().map(|id| world.social.display_name_of(id)).collect()
    }

    /// A nearby chat field drawn this frame: its keyboard, its text after
    /// a key, a line sent (FSNearbyChat::handleChatBarKeystroke).
    /// `said`: its text, after this frame's key, says something aloud
    /// (`typing::says_aloud`).
    fn local_field(&mut self, resp: &egui::Response, said: bool, sent: bool) {
        self.local_field.focused |= resp.has_focus();
        if resp.changed() {
            self.local_field.typed = Some(said);
        }
        self.local_field.sent |= sent;
    }

    /// Once per frame, after the fields are drawn: our typing in the nearby
    /// chat starts (Some(true)) or stops (Some(false)).
    pub fn local_typing_update(&mut self, now: std::time::Instant) -> Option<bool> {
        self.local_typing.update(now, std::mem::take(&mut self.local_field))
    }

    /// `AURORA_DEMO_TYPING`: we type without a key, every frame; true when
    /// it starts.
    pub fn hold_local_typing(&mut self, now: std::time::Instant) -> bool {
        self.local_field = typing::LocalField::default();
        self.local_typing.hold(now)
    }

    /// We type in the nearby chat (AGENT_STATE_TYPING).
    pub fn local_typing(&self) -> bool {
        self.local_typing.typing()
    }

    /// Stop typing in the nearby chat (after a teleport); true when we were.
    pub fn stop_local_typing(&mut self) -> bool {
        self.local_typing.stop()
    }
}

pub struct OutgoingChat {
    pub message: String,
    pub channel: i32,
    pub chat_type: ChatType,
}

/// Parse "/5 text" channel prefixes.
/// FSNearbyChat::stripChannelNumber (fsnearbychathub.cpp): a slash, an
/// optional minus sign, then the digits; the text after them, with or
/// without a space ("/5 cmd", "/5cmd", "/-4242 ping"). Used by the chat bar
/// and by gestures' chat steps alike.
pub fn parse_channel(text: &str) -> (i32, String) {
    if let Some(rest) = text.strip_prefix('/') {
        let sign = usize::from(rest.starts_with('-'));
        let digits = rest[sign..].chars().take_while(char::is_ascii_digit).count();
        if digits > 0
            && let Ok(ch) = rest[..sign + digits].parse::<i32>()
        {
            let msg = rest[sign + digits..].trim_start().to_owned();
            return (ch, msg);
        }
    }
    (0, text.to_owned())
}

/// A MU* pose as an emote, when ":" stands for "/me" (AllowMUpose):
/// ":waves" becomes "/me waves" and ":'s here" "/me's here". A line of three
/// bytes or less, or a colon followed by a digit, a punctuation mark or a
/// space (":)", ":3", ": "), stays as it is: the smileys are kept.
/// Port of FSCommon::applyMuPose (indra/newview/fscommon.cpp, originally
/// LGPL 2.1), applied like it on channel 0 and in the conversations.
pub fn colon_emote(text: &mut String) {
    let with = match text.strip_prefix(':').and_then(|rest| rest.chars().next()) {
        _ if text.len() <= 3 => return,
        Some('\'') => "/me",
        Some(c) if !(c.is_ascii_digit() || c.is_ascii_punctuation() || c.is_ascii_whitespace()) => "/me ",
        _ => return,
    };
    text.replace_range(..1, with);
}

fn color_for(kind: ChatKind, p: &Palette) -> Color32 {
    match kind {
        ChatKind::Own(_) => p.violet_pale,
        ChatKind::Local(_) => p.ink,
        // a script error in red in the cards and the islands too (#1263)
        ChatKind::Object(ChatType::Debug) => super::colors::script_error_color(),
        ChatKind::Object(_) | ChatKind::ObjectIm => p.teal.gamma_multiply(0.85),
        ChatKind::System | ChatKind::Invite(_) | ChatKind::InventoryOffer(_) | ChatKind::Payment(_) => p.muted,
        ChatKind::Im => p.violet_light,
    }
}

/// Time of a chat line, in Second Life time as Firestorm shows it and as
/// the transcripts have it ("hour,datetime,slt"). Empty for a line whose
/// time is unknown (history without a timestamp).
pub(crate) fn hhmm(t: std::time::SystemTime) -> String {
    if t == std::time::UNIX_EPOCH {
        return String::new();
    }
    let t = Slt::of(t);
    format!("{:02}:{:02}", t.hour, t.minute)
}

/// Color of the name and text of a history line: Firestorm greys what comes
/// from the transcript (ChatHistoryMessageFromLog) and gives the chat
/// server's replay its own tone (ChatHistoryMessageFromServerLog).
pub(crate) fn history_tone(p: &Palette, origin: Origin) -> Option<Color32> {
    match origin {
        Origin::Live => None,
        Origin::Log => Some(p.muted_dim),
        Origin::ServerLog => Some(p.muted_dim.lerp_to_gamma(p.success, 0.3)),
    }
}

/// The day a group of lines was said, between two lines of different days
/// (a history spans days; the times stay HH:MM).
fn date_marker(ui: &mut egui::Ui, p: &Palette, year: i64, month: i64, day: i64) {
    let month = super::land::months()[(month.clamp(1, 12) - 1) as usize];
    let text = aurora_i18n::tr!(
        "ui-chat-day",
        day = day.to_string(),
        month = month.to_string(),
        year = year.to_string()
    );
    ui.add_space(4.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(text).size(11.0).color(p.muted_dim));
    });
}

/// A link of a chat bubble, clickable like in the conversation window.
#[derive(Debug, Clone, PartialEq)]
enum Link {
    Url(String),
    Place(String, crate::slurl::PlaceLink),
    Agent(uuid::Uuid),
    /// A `secondlife:///app/...` command (`app_links.rs`).
    App(String, crate::slapp::AppLink),
}

impl Link {
    fn action(&self) -> Option<action_marker::Action> {
        Some(match self {
            Self::Url(_) => return None,
            Self::Agent(_) => action_marker::Action::Profile,
            Self::Place(_, place) => action_marker::Action::place(place.teleport),
            Self::App(_, app) => action_marker::Action::app(app),
        })
    }
}

/// A line's text cut into pieces, links replaced by their labels as
/// Firestorm's chat console does (LLConsole::Paragraph with parse_urls,
/// indra/llui/llconsole.cpp): places read "Region (x,y,z)", agent links the
/// avatar's name, mentions "@name" like the conversation window, app
/// commands their label. `label_of` gives the label of an agent link, a
/// mention (without its "@") or an app command.
fn pieces(text: &str, mut label_of: impl FnMut(&Seg) -> String) -> Vec<(String, Option<Link>)> {
    segments(text)
        .into_iter()
        .map(|s| match s {
            Seg::Text(t) | Seg::Slurl(t) => (t.to_owned(), None),
            Seg::Url(u) => (u.to_owned(), Some(Link::Url(u.to_owned()))),
            Seg::Place(u, place) => (place.label.clone(), Some(Link::Place(u.to_owned(), place))),
            Seg::Agent(id) => (label_of(&s), Some(Link::Agent(id))),
            Seg::Mention(id) => (format!("@{}", label_of(&s)), Some(Link::Agent(id))),
            Seg::App(..) => {
                let label = label_of(&s);
                let Seg::App(u, link) = s else { unreachable!() };
                (label, Some(Link::App(u.to_owned(), link)))
            }
            Seg::Labelled(label, inner) => (
                label.into_owned(),
                match *inner {
                    Seg::Url(u) => Some(Link::Url(u.to_owned())),
                    Seg::Agent(id) | Seg::Mention(id) => Some(Link::Agent(id)),
                    Seg::App(u, link) => Some(Link::App(u.to_owned(), link)),
                    _ => None,
                },
            ),
        })
        .collect()
}

/// Check, warning triangle or cross before a web link (`link_trust`), with
/// its explanation.
fn link_badge_look(p: &Palette, url: &str) -> (&'static str, Color32, String) {
    use crate::link_trust::Trust;
    match crate::link_trust::classify(&web_target(url)) {
        Trust::Trusted => ("circle-check-filled", p.success, aurora_i18n::tr!("ui-chat-trusted-site").into()),
        Trust::Unknown => (
            "alert-triangle-filled",
            p.amber,
            aurora_i18n::tr!("ui-chat-external-link-check-the-address-before-opening-it").into(),
        ),
        Trust::Dangerous(why) => (
            "circle-x-filled",
            p.danger,
            aurora_i18n::tr!("ui-chat-dangerous-link", why = why.to_string()),
        ),
    }
}

fn paint_link_badge(painter: &egui::Painter, p: &Palette, rect: egui::Rect, url: &str, alpha: f32) {
    let (icon, col, _) = link_badge_look(p, url);
    if let Some(t) = super::icons::global(icon) {
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        painter.image(t.id(), rect, uv, col.gamma_multiply(alpha));
    }
}

/// The badge of a web link in a wrapping text (conversation, profile).
fn link_badge(ui: &mut egui::Ui, p: &Palette, url: &str, size: f32) {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(size + 3.0, size), egui::Sense::hover());
    let icon = egui::Rect::from_min_size(rect.min, egui::vec2(size, size)).shrink(1.0);
    paint_link_badge(ui.painter(), p, icon, url, 1.0);
    resp.on_hover_text(link_badge_look(p, url).2);
}

/// A place link of a text: its region's rating badge (as in the navigation
/// bar, once known), then its label, which teleports or opens the place's
/// details.
fn place_link(
    ui: &mut egui::Ui,
    p: &Palette,
    ratings: &crate::world::worldmap::RegionRatings,
    u: &str,
    place: crate::slurl::PlaceLink,
    size: f32,
) {
    if place.location.grid.is_none() && super::widgets::region_badge_in_text(ui, p, ratings, &place.location.region, size).is_some() {
        ui.add_space(3.0);
    }
    let k = super::colors::get();
    let r = action_marker::link(
        ui,
        action_marker::Action::place(place.teleport),
        &place.label,
        size,
        super::colors::text(k.chat_slurl),
        None,
    );
    place_menu(&r, p, u, &place);
    // TooltipTeleportUrl / TooltipSLURL; a place opens its
    // details (LLURLDispatcherImpl::regionHandleCallback,
    // SLURLTeleportDirectly off: FSFloaterPlaceDetails)
    let tip = if place.teleport {
        aurora_i18n::tr!("ui-chat-click-to-teleport-to-this-place")
    } else {
        aurora_i18n::tr!("ui-chat-click-to-learn-more-about-this-place")
    };
    if r.on_hover_text(format!("{tip}\n{u}")).clicked() {
        let l = place.location;
        super::context::request(
            ui.ctx(),
            if place.teleport {
                CtxAction::TeleportToPlace(l.target(), l.pos)
            } else {
                CtxAction::ShowPlaceInfo(l.target(), l.pos)
            },
        );
    }
}

/// A notification's or a script dialog's text: plain and selectable, its
/// links shown and clickable as in the chat: places with their region's
/// badge, avatars, groups and the other `secondlife:///app` commands by
/// their name (#898), web links. The Legacy Viewer's toasts and script
/// dialogs parse their links the same way (LLToastNotifyPanel's text editor,
/// panel_notification.xml `parse_urls`).
pub(crate) fn text_with_links(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &World,
    text: &str,
    names: &[crate::world::notifications::NameLink],
    size: f32,
    color: Color32,
) {
    let segs = segments(text);
    let plain = |ui: &mut egui::Ui, t: &str| {
        if !t.is_empty() {
            super::notifications::text::show(ui, p, t, names, size, color, false, true);
        }
    };
    if segs.iter().all(|s| matches!(s, Seg::Text(_))) {
        plain(ui, text);
        return;
    }
    let k = super::colors::get();
    let mut wanted = HashSet::new();
    super::emoji::text_rows(ui, size, |ui| {
        for s in segs {
            match s {
                Seg::Text(t) => plain(ui, t),
                Seg::Url(u) => web_link(ui, p, u, u, size),
                Seg::Slurl(u) => {
                    select::label(ui, RichText::new(u).size(size).color(super::colors::text(k.chat_slurl))).on_hover_text(u);
                }
                Seg::Place(u, place) => place_link(ui, p, &world.map.ratings, u, place, size),
                Seg::Labelled(label, inner) => match *inner {
                    Seg::Url(u) => web_link(ui, p, u, &label, size),
                    Seg::Agent(id) | Seg::Mention(id) => agent_link(ui, p, world, id, &label, size),
                    Seg::App(u, link) => app_links::link(ui, p, world, u, &link, &label, size),
                    _ => plain(ui, &label),
                },
                Seg::App(u, link) => {
                    let label = app_links::label(world, &mut wanted, u, &link);
                    app_links::link(ui, p, world, u, &link, &label, size);
                }
                // a dialog has no mention highlight: the name, as a link
                Seg::Agent(id) | Seg::Mention(id) => {
                    wanted.insert(id);
                    agent_link(ui, p, world, id, &world.social.name_of(&id), size);
                }
            }
        }
    });
    app_links::want_agents(wanted);
}

/// Open a web link of a text: trusted ones open at once, unknown ones after
/// a warning (unless turned off), dangerous ones always after a warning
/// (`CtxAction::OpenUrl`).
pub(crate) fn open_web_link(ctx: &egui::Context, url: &str) {
    super::context::request(ctx, CtxAction::OpenUrl(web_target(url).into_owned()));
}

/// Who wrote a bubble line, drawn before the name.
enum Sender {
    Avatar(uuid::Uuid),
    Object,
}

/// A chat bubble laid out: the text, the char range of each link, the char
/// of each link's badge, and the char where the sender's name starts (room
/// for its picture before it).
struct BubbleJob {
    job: egui::text::LayoutJob,
    links: Vec<(std::ops::Range<usize>, Link)>,
    badges: Vec<(usize, LinkBadge)>,
    sender: Option<(usize, Sender)>,
    /// The characters of the sender's name (none for a notice).
    name: Option<std::ops::Range<usize>>,
}

/// The badge before a link: a web link's trust, a place's region rating.
enum LinkBadge {
    Web(String),
    Rating(u8),
    Action(action_marker::Action),
}

/// Side of the sender's picture, for a text size.
fn pic_side(size: f32) -> f32 {
    size + 3.0
}

/// How a chat line is laid out: the full-width bar's "Name: text", the
/// islands' pill (name in semi-bold, no colon), or the body alone under the
/// head of a card (`console.rs`: the name, the time and the volume are in
/// the head; an emote keeps its name, which is part of its sentence).
#[derive(Clone, Copy)]
enum JobLook<'a> {
    Line,
    Pill(&'a egui::FontFamily),
    Body,
    /// An object's line on one small row (`object_row.rs`): the name in the
    /// colour of objects' names, no colon, the text muted.
    Compact,
}

#[allow(clippy::too_many_arguments)]
fn line_job(
    line: &crate::world::ChatLine,
    p: &Palette,
    world: &World,
    want_names: &mut HashSet<uuid::Uuid>,
    width: f32,
    size: f32,
    times: bool,
    look: JobLook<'_>,
) -> BubbleJob {
    let filtered;
    let line = if world.rlva.has("shownames") || world.rlva.has("showloc") {
        let mut copy = line.clone();
        copy.text = world.rlva_filter_text(&copy.text).into_owned();
        copy.from = sender_name(world, &world.own_name(), &copy);
        filtered = copy;
        &filtered
    } else {
        line
    };
    let pill = match look {
        JobLook::Pill(family) => Some(family),
        _ => None,
    };
    let body_only = matches!(look, JobLook::Body);
    let compact = matches!(look, JobLook::Compact);
    // a line that holds a keyword takes its colour, name included
    let col = super::colors::contact_chat_color(line, world)
        .or_else(|| super::colors::keyword_color(line))
        .unwrap_or_else(|| {
            if compact {
                object_row::text_color(line, p)
            } else {
                color_for(line.kind, p)
            }
        });
    let mut job = egui::text::LayoutJob::default();
    let small = egui::TextFormat {
        color: p.muted_dim,
        font_id: egui::FontId::proportional(size - 3.0),
        ..Default::default()
    };
    // (a compact row paints its time on the text's baseline itself: a
    // smaller section of the job sits on the bottom of the row, lower)
    if times && !body_only && !compact {
        job.append(&format!("[{}] ", hhmm(line.time)), 0.0, small);
    }
    let emote = line.text.starts_with("/me ") || line.text.starts_with("/me'");
    // the pills of the islands (the bars mockup's `.toasts`): a resident's
    // name in semi-bold light violet, no colon, a system line in italics;
    // every other colour stays the kind's
    let resident = matches!(line.kind, ChatKind::Local(_) | ChatKind::Own(_));
    let mut name_fmt = match pill {
        Some(family) => super::fonts::semibold_format(family, size, if resident { p.violet_light } else { col }),
        None if compact => egui::TextFormat {
            color: super::colors::sender_color(line),
            font_id: egui::FontId::proportional(size),
            ..Default::default()
        },
        None => egui::TextFormat {
            color: col,
            font_id: egui::FontId::proportional(size),
            ..Default::default()
        },
    };
    if let Some(color) = super::colors::contact_sender_color(line, world) {
        name_fmt.color = color;
    }
    let text_fmt = egui::TextFormat {
        color: col,
        font_id: egui::FontId::proportional(size),
        italics: emote || ((pill.is_some() || body_only) && line.kind.is_notice()),
        ..Default::default()
    };
    // (before the name, after it, body)
    let (before, after, body) = match line.kind {
        ChatKind::System | ChatKind::Invite(_) | ChatKind::InventoryOffer(_) | ChatKind::Payment(_) => ("", "", &line.text[..]),
        _ if emote => ("", "", &line.text[3..]),
        _ if body_only => ("", "", &line.text[..]),
        ChatKind::Im => ("[IM] ", ": ", &line.text[..]),
        ChatKind::Local(ChatType::Shout) | ChatKind::Own(ChatType::Shout) => ("", aurora_i18n::tr!("status-shouts"), &line.text[..]),
        ChatKind::Local(ChatType::Whisper) | ChatKind::Own(ChatType::Whisper) => ("", aurora_i18n::tr!("status-whispers"), &line.text[..]),
        _ if pill.is_some() || compact => ("", "", &line.text[..]),
        _ => ("", ": ", &line.text[..]),
    };
    // in a pill the body follows the name 4 px away instead of a colon, in
    // a compact row 5 px as in the conversations (an emote's body starts
    // with its own space)
    let name_gap = match (pill.is_some(), compact) {
        _ if !after.is_empty() || emote || line.kind.is_notice() => 0.0,
        (true, _) => 4.0,
        (_, true) => 5.0,
        _ => 0.0,
    };
    let mut sender = None;
    let mut name = None;
    // the body of a card: no name, except an emote's, in its own colour
    let name_fmt = if body_only { text_fmt.clone() } else { name_fmt };
    if !line.kind.is_notice() && (!body_only || emote) {
        job.append(before, 0.0, name_fmt.clone());
        let who = match line.kind {
            ChatKind::Object(_) | ChatKind::ObjectIm => Some(Sender::Object),
            _ if !line.source.is_nil() => Some(Sender::Avatar(line.source)),
            _ => None,
        };
        // room for the picture, painted over it once laid out (none in a
        // pill nor in a card, whose head has it)
        let who = who.filter(|_| pill.is_none() && !body_only);
        let lead = if who.is_some() { pic_side(size) + 4.0 } else { 0.0 };
        sender = who.map(|w| (job.text.chars().count(), w));
        let start = job.text.chars().count();
        name = Some(start..start + line.from.chars().count());
        job.append(&line.from, lead, name_fmt.clone());
        job.append(after, 0.0, name_fmt);
    }
    let mut links = Vec::new();
    let mut badges = Vec::new();
    let mut at = job.text.chars().count();
    let name_of = |s: &Seg| match s {
        Seg::Agent(id) | Seg::Mention(id) => {
            want_names.insert(*id);
            world.social.name_of(id)
        }
        Seg::App(u, link) => app_links::label(world, want_names, u, link),
        _ => String::new(),
    };
    // the badge of a web link takes the place of an invisible glyph glued to
    // the link (no space between them), so both wrap to the next row
    // together; a leading space would stay behind on the previous row
    let badge_fmt = egui::TextFormat {
        color: Color32::TRANSPARENT,
        font_id: egui::FontId::proportional(size * 1.15),
        ..Default::default()
    };
    let mut lead = name_gap;
    for (piece, link) in pieces(body, name_of) {
        let piece = if let Some(Link::Agent(id)) = &link
            && !world.rlva_can_name(*id)
        {
            world.social.name_of(id)
        } else {
            world.rlva_filter_text(&piece).into_owned()
        };
        let badge = match &link {
            Some(Link::Url(u)) => Some(LinkBadge::Web(u.clone())),
            Some(Link::Place(_, place)) if place.location.grid.is_none() => {
                world.map.ratings.get(&place.location.region).map(LinkBadge::Rating)
            }
            _ => None,
        };
        if let Some(badge) = badge {
            job.append("M", std::mem::take(&mut lead), badge_fmt.clone());
            badges.push((at, badge));
            at += 1;
        }
        let start = at;
        // links in the link colors, like in the conversation window
        let k = super::colors::get();
        let fmt = match &link {
            None => text_fmt.clone(),
            Some(Link::Url(_)) => egui::TextFormat {
                color: super::colors::text(k.chat_urls),
                ..text_fmt.clone()
            },
            Some(_) => egui::TextFormat {
                color: super::colors::text(k.chat_slurl),
                ..text_fmt.clone()
            },
        };
        if let Some(action) = link.as_ref().and_then(Link::action) {
            badges.push((at, LinkBadge::Action(action)));
            action_marker::reserve_icon(&mut job, size, std::mem::take(&mut lead));
            at += 1;
            lead = 2.0;
        }
        let n = piece.chars().count();
        job.append(&piece, std::mem::take(&mut lead), fmt);
        if let Some(link) = link {
            links.push((start..at + n, link));
        }
        at += n;
    }
    job.wrap.max_width = width;
    BubbleJob {
        job,
        links,
        badges,
        sender,
        name,
    }
}

pub enum ConvAction {
    Local(OutgoingChat),
    Im {
        to: uuid::Uuid,
        text: String,
    },
    OfferTeleport(uuid::Uuid),
    /// Open the People floater (0 = nearby, 1 = friends).
    OpenPeople(u8),
    /// From the "Contacts" tab.
    Contacts(super::contacts::ContactsAction),
    /// From the "Contacts" tab in Aurora's look: People's friends tab.
    People(super::people::PeopleAction),
    /// Block / unblock the avatar of a 1:1 conversation.
    ToggleBlock(uuid::Uuid),
    /// Close a conversation (the X of its tab in FSFloaterIMContainer): a
    /// group / conference session is left, a 1:1 conversation leaves the list.
    Close(uuid::Uuid),
    /// Stop receiving a group's chat (and leave its session).
    BlockGroupChat(uuid::Uuid),
    /// Snooze a group chat: closed without leaving, its messages ignored
    /// for a while (`snooze.rs`).
    Snooze(uuid::Uuid),
    /// Open an avatar's profile window.
    Profile(uuid::Uuid),
    /// Call the conversation's resident, group or conference.
    Call(uuid::Uuid),
    /// Hang up the call in progress.
    HangUp,
    /// Mute / unmute the microphone (toggle mode).
    ToggleMic,
    /// Answer the call invitation of a session from its strip.
    AnswerCall {
        session: uuid::Uuid,
        answer: crate::world::calls::InviteAnswer,
    },
    /// A participant's voice volume (0..1, 0.5 as sent).
    SetVolume(uuid::Uuid, f32),
    /// "Add" of the call strip: pick residents to add to the conversation.
    PickPeople(uuid::Uuid),
    /// Residents dropped on a conversation, to add to it.
    AddPeople(uuid::Uuid, Vec<uuid::Uuid>),
    /// Open a conversation with a resident (the column's search).
    OpenIm(uuid::Uuid),
    /// Open the chat of one of our groups (the column's "+").
    StartGroupChat(uuid::Uuid),
    /// We start or stop typing to a resident (IM_TYPING_START / STOP).
    Typing {
        to: uuid::Uuid,
        typing: bool,
    },
    /// Inventory dropped on a conversation with a resident: give it, once
    /// confirmed (the inventory's Share dialog).
    GiveItems {
        to: uuid::Uuid,
        items: Vec<uuid::Uuid>,
    },
    /// Open the inventory, to drag what to give from it.
    OpenInventory,
    /// The audio output of the call ("" = the system's default).
    SetOutputDevice(String),
    /// List the audio outputs again (their menu opens).
    ListOutputDevices,
    /// Pop a conversation out into its own window, or dock it back (None:
    /// the nearby chat; `popout.rs`). Done by `show` before it returns.
    PopOut(Option<uuid::Uuid>),
    DockBack(Option<uuid::Uuid>),
    /// An entry dragged out of the window (#910): its conversation in its
    /// own window, the window's top left corner where the entry was let go.
    PopOutAt(Option<uuid::Uuid>, egui::Pos2),
}

use super::widgets::initials;

/// Profile picture (or initials) of an avatar, `size` px square.
fn avatar_pic(ui: &mut egui::Ui, p: &Palette, pic: Option<&egui::TextureHandle>, name: &str, size: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    super::widgets::avatar_picture(ui, p, rect, pic, name, false);
}

/// The id of a button of the window of tabs (Bubbles and Lines styles) by
/// its icon ("send" for the send button); each window of its own has its
/// own (`popout::window`).
pub(crate) fn classic_id(icon: &str) -> egui::Id {
    egui::Id::unique(("conv_classic", icon, popout::window()))
}

/// Tabler icon button of the conversation toolbar.
fn tool(ui: &mut egui::Ui, p: &Palette, icons: &super::icons::Icons, icon: &str, tip: &str, enabled: bool, active: bool) -> bool {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(26.0, 22.0), egui::Sense::hover());
    // the same id every frame (the tests reach the buttons by their icon)
    let resp = ui.interact(rect, classic_id(icon), egui::Sense::click());
    let radius = ui.visuals().window_corner_radius;
    if active {
        ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.3));
    } else if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, radius, p.raised);
    }
    if let Some(t) = icons.get(icon) {
        let tint = if !enabled {
            p.muted_dim.gamma_multiply(0.6)
        } else if resp.hovered() || active {
            p.ink
        } else {
            p.muted
        };
        let r = egui::Rect::from_center_size(rect.center(), egui::vec2(16.0, 16.0));
        ui.painter().image(
            t.id(),
            r,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            tint,
        );
    }
    resp.on_hover_text(tip).clicked() && enabled
}

/// Pieces of a chat message: plain text, web links, SL links and mentions.
#[derive(Debug, PartialEq)]
enum Seg<'a> {
    Text(&'a str),
    Url(&'a str),
    Slurl(&'a str),
    /// A place (maps.secondlife.com, secondlife://Region/x/y/z, app/region,
    /// app/teleport): shown as "Region (x,y,z)", clickable.
    Place(&'a str, crate::slurl::PlaceLink),
    /// secondlife:///app/agent/<id>/mention
    Mention(uuid::Uuid),
    /// secondlife:///app/agent/<id>/about (inspect, completename...): shown
    /// as the avatar's name, opens the profile (LLUrlEntryAgent).
    Agent(uuid::Uuid),
    /// A link written with its own text, `[https://example.org Text]` or
    /// `[secondlife:///app/... Text]` (LLUrlEntryHTTPLabel, LLUrlEntrySLLabel):
    /// shown as the text, it acts as the link inside (a web link, an agent
    /// link or another SL link; a place takes the text as its label).
    Labelled(Cow<'a, str>, Box<Seg<'a>>),
    /// Another `secondlife:///app/...` command (`slapp.rs`).
    App(&'a str, crate::slapp::AppLink),
}

/// The text mentions `agent` (LLUrlRegistry::containsAgentMention).
pub(crate) fn mentions(text: &str, agent: &uuid::Uuid) -> bool {
    text.contains("/mention") && segments(text).iter().any(|s| matches!(s, Seg::Mention(id) if id == agent))
}

/// A message as plain text, for a system notification or a short preview:
/// agents by their name (a mention with its "@"), places by their label,
/// app commands by `app` (their label, `app_links::label`), labelled links
/// by their text, web links as written.
pub(crate) fn plain_text(
    text: &str,
    name: impl Fn(&uuid::Uuid) -> Option<String>,
    app: impl Fn(&str, &crate::slapp::AppLink) -> String,
) -> String {
    let mut out = String::with_capacity(text.len());
    for seg in segments(text) {
        match seg {
            Seg::Text(t) | Seg::Url(t) | Seg::Slurl(t) => out.push_str(t),
            Seg::App(u, link) => out.push_str(&app(u, &link)),
            Seg::Place(_, place) => out.push_str(&place.label),
            Seg::Mention(id) => {
                out.push('@');
                out.push_str(&name(&id).unwrap_or_else(|| id.to_string()));
            }
            Seg::Agent(id) => out.push_str(&name(&id).unwrap_or_else(|| id.to_string())),
            Seg::Labelled(label, _) => out.push_str(&label),
        }
    }
    out
}

/// The label of an app command, for the texts drawn outside the chat (a
/// system notification).
pub(crate) fn app_label(world: &World, url: &str, link: &crate::slapp::AppLink) -> String {
    app_links::label(world, &mut HashSet::new(), url, link)
}

/// A message with its links shown as in the conversation, in plain text,
/// for the previews that cut it short (a conversation's last line in the
/// list, a message's card, #947): avatars by their name ("…" until it comes,
/// asked at the end of the frame), places, app commands and labelled links
/// by their label. The preview cuts the result, never a link's address in
/// its middle, as the Legacy Viewer's toasts show the labels of their text
/// (`parse_urls`, LLTextBase::appendTextImpl) before eliding it.
pub(crate) fn resolved_text(world: &World, text: &str) -> String {
    let name = |id: &uuid::Uuid| {
        app_links::want_agents([*id]);
        Some(world.social.name_of(id))
    };
    let text = plain_text(text, name, |u, link| app_label(world, u, link));
    world.rlva_filter_text(&text).into_owned()
}

fn segments(text: &str) -> Vec<Seg<'_>> {
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let Some((i, len, seg)) = next_link(rest) else {
            out.push(Seg::Text(rest));
            break;
        };
        if i > 0 {
            out.push(Seg::Text(&rest[..i]));
        }
        out.push(seg);
        rest = &rest[i + len..];
    }
    out
}

/// The first link of a text: where it starts, how long it is, and what it
/// is. Links are found in the order of LLUrlRegistry::findUrl
/// (indra/llui/llurlregistry.cpp): the one that starts first, so the `[` of
/// a link with its own text wins over the address inside it.
fn next_link(text: &str) -> Option<(usize, usize, Seg<'_>)> {
    let schemes = ["https://", "http://", "secondlife://", "hop://", "x-grid-location-info://"]
        .iter()
        .filter_map(|p| text.find(p));
    let bare = schemes.chain(web_start(text)).min();
    let labelled = text
        .match_indices('[')
        .map(|(i, _)| i)
        .take_while(|&i| bare.is_none_or(|b| i < b))
        .find_map(|i| labelled(&text[i..]).map(|(len, seg)| (i, len, seg)));
    if labelled.is_some() {
        return labelled;
    }
    let i = bare?;
    let tail = &text[i..];
    let mut end = tail.find(char::is_whitespace).unwrap_or(tail.len());
    // trailing punctuation belongs to the sentence
    while end > 0 && tail[..end].ends_with(['.', ',', '!', '?', ')', ']', ';', ':', '"', '\'']) {
        end -= 1;
    }
    let link = &tail[..end.max(1)];
    let link = &link[..verb_end(link)];
    Some((i, link.len(), link_seg(link)))
}

/// Where an agent, group, parcel or experience link ends: after the word of
/// its verb, as in LLUrlEntryAgent, LLUrlEntryGroup, LLUrlEntryParcel and
/// LLUrlEntryExperienceProfile (`/agent/[\da-f-]+/\w+`), which come before
/// the generic LLUrlEntrySL in LLUrlRegistry. So in "…/about's hat" the
/// link is "…/about" and "'s hat" stays text. Unlike the Legacy Viewer, a
/// verb followed by `/` or `?` keeps its arguments (`…/block/Old%20Sign`).
fn verb_end(link: &str) -> usize {
    fn strip<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
        s.get(..prefix.len())
            .filter(|p| p.eq_ignore_ascii_case(prefix))
            .map(|_| &s[prefix.len()..])
    }
    const KINDS: [&str; 5] = ["agent/", "agentself/", "group/", "parcel/", "experience/"];
    let Some(id) = strip(link, "secondlife:///app/").and_then(|rest| KINDS.iter().find_map(|k| strip(rest, k))) else {
        return link.len();
    };
    let id_len = id.find(|c: char| !(c.is_ascii_hexdigit() || c == '-')).unwrap_or(id.len());
    let Some(verb) = id[id_len..].strip_prefix('/').filter(|_| id_len > 0) else {
        return link.len();
    };
    let word = verb.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(verb.len());
    if word == 0 || verb[word..].starts_with(['/', '?']) {
        return link.len();
    }
    link.len() - (verb.len() - word)
}

/// Where an address without its scheme starts, "www.example.org"
/// (LLUrlEntryHTTPNoProtocol: `\bwww\.\S+\.([^\s<]*)?\b`).
fn web_start(text: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(i) = text[from..].find("www.").map(|i| i + from) {
        let word_start = !text[..i].ends_with(|c: char| c.is_alphanumeric() || c == '_');
        let host = text[i + 4..].split(|c: char| c.is_whitespace() || c == '<').next().unwrap_or("");
        // a dot with something on both sides after "www."
        let dotted = host
            .trim_end_matches(|c: char| !c.is_alphanumeric())
            .find('.')
            .is_some_and(|d| d > 0);
        if word_start && dotted {
            return Some(i);
        }
        from = i + 4;
    }
    None
}

/// `[https://example.org Text]` at the start of `text`: its length and its
/// segment. The pattern of LLUrlEntryHTTPLabel and LLUrlEntrySLLabel,
/// `\[(https?|secondlife)://\S+[ \t]+[^\]]+\]`; the text is unescaped like
/// getLabelFromWikiLink, and a text that is itself a link shows the address
/// instead (LLUrlEntryHTTPLabel::getLabel), so it cannot pass for another
/// destination.
fn labelled(text: &str) -> Option<(usize, Seg<'_>)> {
    let body = text.strip_prefix('[')?;
    let known = ["https://", "http://", "secondlife://", "hop://", "x-grid-location-info://"]
        .iter()
        .any(|s| body.get(..s.len()).is_some_and(|p| p.eq_ignore_ascii_case(s)));
    let url_end = body.find(char::is_whitespace)?;
    let url = &body[..url_end];
    if !known || url.ends_with("://") {
        return None;
    }
    let after = &body[url_end..];
    let label_start = after.trim_start_matches([' ', '\t']);
    let gap = after.len() - label_start.len();
    if gap == 0 {
        return None;
    }
    let close = label_start.find(']')?;
    let label = label_start[..close].trim_end();
    if label.trim().is_empty() {
        return None;
    }
    let len = 1 + url_end + gap + close + 1;
    let inner = link_seg(url);
    let label = unescape(label);
    if segments(&label).iter().any(|s| !matches!(s, Seg::Text(_))) {
        return Some((len, inner));
    }
    Some((
        len,
        match inner {
            Seg::Place(u, mut place) => {
                place.label = label.into_owned();
                Seg::Place(u, place)
            }
            inner => Seg::Labelled(label, Box::new(inner)),
        },
    ))
}

/// What a link is, from its address.
fn link_seg(link: &str) -> Seg<'_> {
    if let Some(rest) = link.strip_prefix("secondlife:///app/agent/") {
        let (id, action) = rest.split_once('/').unwrap_or((rest, ""));
        match (uuid::Uuid::parse_str(id).ok(), action) {
            (Some(id), "mention") => Seg::Mention(id),
            (Some(id), "about" | "inspect" | "completename" | "displayname" | "username") => Seg::Agent(id),
            _ => crate::slapp::parse(link).map_or(Seg::Slurl(link), |app| Seg::App(link, app)),
        }
    } else if let Some(place) = crate::slurl::place_link(link) {
        Seg::Place(link, place)
    } else if let Some(app) = crate::slapp::parse(link) {
        Seg::App(link, app)
    } else if ["secondlife://", "hop://", "x-grid-location-info://"]
        .iter()
        .any(|prefix| link.starts_with(prefix))
    {
        Seg::Slurl(link)
    } else {
        Seg::Url(link)
    }
}

/// `%XX` escapes decoded (LLURI::unescape).
fn unescape(text: &str) -> Cow<'_, str> {
    if !text.contains('%') {
        return Cow::Borrowed(text);
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(v) = text.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Cow::Owned(String::from_utf8_lossy(&out).into_owned())
}

/// The address a web link opens: "www.example.org" gets its scheme
/// (LLUrlEntryHTTPNoProtocol::getUrl).
fn web_target(url: &str) -> Cow<'_, str> {
    if url.contains("://") {
        Cow::Borrowed(url)
    } else {
        Cow::Owned(format!("http://{url}"))
    }
}

/// Right click on the sender of a chat line: the name menu, its report on
/// the line.
fn line_name_menu(r: &egui::Response, p: &Palette, world: &World, id: uuid::Uuid, line: &crate::world::ChatLine, menu: Option<egui::Id>) {
    if !matches!(line.kind, ChatKind::Object(_) | ChatKind::ObjectIm) {
        super::contacts::drag::person_source(r, id);
    }
    if let Some(menu) = menu {
        line_menu::sender(r, menu, id, line);
    } else {
        super::menu::context_menu(r, p, |ui| super::context::chat_sender_menu(ui, p, world, id, line));
    }
}

/// Right click on an avatar name in a conversation (menu_url_agent.xml).
fn name_menu(r: &egui::Response, p: &Palette, world: &World, id: uuid::Uuid) {
    super::contacts::drag::person_source(r, id);
    super::menu::context_menu(r, p, |ui| {
        super::context::avatar_list_menu(ui, p, world, id, super::context::AvatarList::Name)
    });
    line_menu::inline(r);
}

/// Right click on a place link: menu_url_slurl.xml (place details first)
/// or menu_url_teleport.xml (teleport first), in Firestorm's order.
fn place_menu(r: &egui::Response, p: &Palette, url: &str, place: &crate::slurl::PlaceLink) {
    super::menu::context_menu(r, p, |ui| {
        let l = &place.location;
        let teleport = |ui: &mut egui::Ui| {
            if super::menu::item(ui, p, "location-filled", aurora_i18n::tr!("ui-chat-teleport-to-this-location")) {
                super::context::request(ui.ctx(), CtxAction::TeleportToPlace(l.target(), l.pos));
            }
        };
        let map_label = if place.teleport {
            aurora_i18n::tr!("common-show-on-map")
        } else {
            aurora_i18n::tr!("common-show-on-map-2")
        };
        let map = |ui: &mut egui::Ui| {
            if super::menu::item(ui, p, "map", map_label) {
                super::context::request(ui.ctx(), CtxAction::ShowPlace(l.target(), l.pos));
            }
        };
        if place.teleport {
            teleport(ui);
            super::menu::separator(ui, p);
            map(ui);
        } else {
            if super::menu::item(
                ui,
                p,
                "info-circle-filled",
                aurora_i18n::tr!("ui-chat-show-information-about-this-place"),
            ) {
                super::context::request(ui.ctx(), CtxAction::ShowPlaceInfo(l.target(), l.pos));
            }
            super::menu::separator(ui, p);
            map(ui);
            teleport(ui);
        }
        super::menu::separator(ui, p);
        if super::menu::item(ui, p, "link-filled", aurora_i18n::tr!("common-copy-slurl")) {
            ui.ctx().copy_text(url.to_owned());
        }
    });
    line_menu::inline(r);
}

/// Message text with emoji, clickable links and highlighted mentions
/// (agent links and mentions open the profile).
#[allow(clippy::too_many_arguments)]
pub(crate) fn chat_text(
    ui: &mut egui::Ui,
    p: &Palette,
    emoji: &mut super::emoji::Emoji,
    world: &World,
    want_names: &mut HashSet<uuid::Uuid>,
    text: &str,
    size: f32,
    color: Color32,
    italics: bool,
) {
    chat_text_marked(ui, p, emoji, world, want_names, text, size, color, italics, None);
}

/// A text searched for in the lines, and the wash its matches are drawn on.
#[derive(Clone, Copy)]
pub(crate) struct Mark<'a> {
    /// Lowercase.
    pub needle: &'a str,
    pub wash: Color32,
}

/// A run of plain text, the matches of the searched text on their wash.
fn marked_run(
    ui: &mut egui::Ui,
    emoji: &mut super::emoji::Emoji,
    text: &str,
    size: f32,
    color: Color32,
    italics: bool,
    mark: Option<Mark>,
) {
    let Some(mark) = mark else {
        emoji.inline(ui, text, size, color, italics);
        return;
    };
    let mut at = 0;
    for m in view::find_ci(text, mark.needle) {
        emoji.inline(ui, &text[at..m.start], size, color, italics);
        emoji.inline_marked(ui, &text[m.clone()], size, color, italics, Some(mark.wash));
        at = m.end;
    }
    emoji.inline(ui, &text[at..], size, color, italics);
}

/// `chat_text` with the matches of a searched text highlighted.
#[allow(clippy::too_many_arguments)]
pub(crate) fn chat_text_marked(
    ui: &mut egui::Ui,
    p: &Palette,
    emoji: &mut super::emoji::Emoji,
    world: &World,
    want_names: &mut HashSet<uuid::Uuid>,
    text: &str,
    size: f32,
    color: Color32,
    italics: bool,
    mark: Option<Mark>,
) {
    let filtered = world.rlva_filter_text(text);
    let text = filtered.as_ref();
    let segs = segments(text);
    // only a text that has a match is cut into runs
    let mark = mark.filter(|m| !view::find_ci(text, m.needle).is_empty());
    if mark.is_none() && segs.iter().all(|s| matches!(s, Seg::Text(_))) {
        emoji.rich_text(ui, text, size, color, italics);
        return;
    }
    let k = super::colors::get();
    super::emoji::text_rows_in(ui, size, super::bidi::rtl_paragraph(text), |ui| {
        for s in segs {
            match s {
                Seg::Text(t) => marked_run(ui, emoji, t, size, color, italics, mark),
                Seg::Url(u) => web_link(ui, p, u, u, size),
                Seg::Slurl(u) => {
                    select::label(ui, RichText::new(u).size(size).color(super::colors::text(k.chat_slurl))).on_hover_text(u);
                }
                Seg::Labelled(label, inner) => match *inner {
                    Seg::Url(u) => web_link(ui, p, u, &label, size),
                    Seg::Agent(id) | Seg::Mention(id) => agent_link(ui, p, world, id, &label, size),
                    Seg::Slurl(u) => {
                        select::label(
                            ui,
                            RichText::new(label.as_ref()).size(size).color(super::colors::text(k.chat_slurl)),
                        )
                        .on_hover_text(u);
                    }
                    Seg::App(u, link) => app_links::link(ui, p, world, u, &link, &label, size),
                    _ => marked_run(ui, emoji, &label, size, color, italics, mark),
                },
                Seg::App(u, link) => {
                    let label = app_links::label(world, want_names, u, &link);
                    app_links::link(ui, p, world, u, &link, &label, size);
                }
                Seg::Place(u, place) => place_link(ui, p, &world.map.ratings, u, place, size),
                Seg::Mention(id) => {
                    want_names.insert(id);
                    let me = id == world.agent_id;
                    let bg = if me { k.mention_me } else { k.mention_residents };
                    let label = format!(" @{} ", world.social.name_of(&id));
                    let r = action_marker::link(
                        ui,
                        action_marker::Action::Profile,
                        &label,
                        size,
                        super::colors::c(k.mention_text),
                        Some(super::colors::c(bg)),
                    );
                    name_menu(&r, p, world, id);
                    if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        super::profile::request_open(ui.ctx(), id);
                    }
                }
                Seg::Agent(id) => {
                    want_names.insert(id);
                    agent_link(ui, p, world, id, &world.social.name_of(&id), size);
                }
            }
        }
    });
}

/// A web link of a text, its trust badge before it; `text` is the address
/// or the text it was written with.
fn web_link(ui: &mut egui::Ui, p: &Palette, url: &str, text: &str, size: f32) {
    let k = super::colors::get();
    link_badge(ui, p, url, size);
    let behind = select::atom_begin(ui);
    let r = ui.add(egui::Link::new(
        RichText::new(text).size(size).color(super::colors::text(k.chat_urls)),
    ));
    select::atom_end(ui, behind, text, r.rect);
    if r.on_hover_text(url).clicked() {
        open_web_link(ui.ctx(), url);
    }
}

/// An avatar link of a text (its name, or the text it was written with):
/// opens the profile, the name menu on a right click.
fn agent_link(ui: &mut egui::Ui, p: &Palette, world: &World, id: uuid::Uuid, text: &str, size: f32) {
    let projected = if world.rlva_can_name(id) {
        world.rlva_filter_text(text).into_owned()
    } else {
        world.social.name_of(&id)
    };
    let text = projected.as_str();
    let k = super::colors::get();
    let r = action_marker::link(
        ui,
        action_marker::Action::Profile,
        text,
        size,
        super::colors::text(k.chat_slurl),
        None,
    );
    name_menu(&r, p, world, id);
    if r.on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(aurora_i18n::tr!("common-view-profile"))
        .clicked()
    {
        super::profile::request_open(ui.ctx(), id);
    }
}

/// Height of the input row: the emoji button, the field and the send button
/// stand on one line, all as tall.
const INPUT_H: f32 = 28.0;

/// Icon button returning its response (the emoji button before the input).
fn tool_resp(ui: &mut egui::Ui, p: &Palette, icons: &super::icons::Icons, icon: &str, active: bool) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(INPUT_H), egui::Sense::hover());
    let resp = ui.interact(rect, classic_id(icon), egui::Sense::click());
    let radius = ui.visuals().window_corner_radius;
    if active {
        ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.3));
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, radius, p.raised);
    }
    if let Some(t) = icons.get(icon) {
        let tint = if resp.hovered() || active { p.ink } else { p.muted };
        let r = egui::Rect::from_center_size(rect.center(), egui::vec2(18.0, 18.0));
        ui.painter().image(
            t.id(),
            r,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            tint,
        );
    }
    resp
}

/// Width of the send button at the end of the input.
const SEND_BTN_W: f32 = 32.0;

/// The send button: the accent face of the window's main action, greyed
/// while there is nothing to send (Enter sends too).
fn send_button(ui: &mut egui::Ui, p: &Palette, icons: &super::icons::Icons, enabled: bool) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(SEND_BTN_W, INPUT_H), egui::Sense::hover());
    let resp = ui.interact(rect, classic_id("send"), egui::Sense::click());
    let face = match (enabled, resp.hovered()) {
        (false, _) => p.violet.gamma_multiply(0.35),
        (true, true) => p.violet_light,
        (true, false) => p.violet,
    };
    ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, face);
    if let Some(t) = icons.get("send-filled") {
        ui.painter().image(
            t.id(),
            egui::Rect::from_center_size(rect.center(), egui::vec2(15.0, 15.0)),
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            if enabled { p.on_accent } else { p.on_accent.gamma_multiply(0.6) },
        );
    }
    resp.on_hover_text(aurora_i18n::tr!("common-send"))
}

/// The time at the right of a message header, laid out before the names so
/// that they know the room it takes.
fn header_time(ui: &egui::Ui, p: &Palette, time: String) -> std::sync::Arc<egui::Galley> {
    let text = RichText::new(time).size(11.5).color(p.muted);
    egui::WidgetText::from(text).into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Body)
}

/// Narrower than this, the "- username" of a header is left out (it then
/// shows on hover of the name) rather than reduced to an ellipsis.
const USERNAME_MIN_W: f32 = 36.0;

/// Sender of a message header: the display name, then " - username" in
/// small (FSChatHistoryHeader). Both are cut with an ellipsis so that
/// `reserve` pixels stay free at the right of the row for the time, as the
/// user_name text of panel_chat_header.xml is (use_ellipses="true", the time
/// box keeps its place); what was cut shows on hover. Returns the response
/// of the name, clickable.
fn header_names(ui: &mut egui::Ui, p: &Palette, name: &str, username: Option<&str>, col: Color32, reserve: f32) -> egui::Response {
    let room = ui.available_width() - reserve;
    let label = super::widgets::elided_label(ui, RichText::new(name).size(13.0).strong().color(col), room);
    let r = ui.add(label.sense(egui::Sense::click()));
    let Some(u) = username else {
        return r;
    };
    let room = ui.available_width() - reserve;
    if room < USERNAME_MIN_W {
        return r.on_hover_text(u);
    }
    let text = RichText::new(format!("- {u}")).size(11.5).color(p.indigo_light);
    ui.add(super::widgets::elided_label(ui, text, room));
    r
}

/// The name a line shows for its sender (FSChatHistoryHeader): the display
/// name looked up, mine, else the name the line came with; history lines are
/// looked up for nothing.
pub(crate) fn sender_name(world: &World, own_name: &str, line: &crate::world::ChatLine) -> String {
    if !line.source.is_nil() && !matches!(line.kind, ChatKind::Object(_) | ChatKind::ObjectIm) && !world.rlva_can_name(line.source) {
        return crate::world::rlva::privacy::anonymous(line.source).display_name.clone();
    }
    let logged = line.origin != Origin::Live;
    let own = line.kind.is_own();
    let object = matches!(line.kind, ChatKind::Object(_) | ChatKind::ObjectIm);
    let names = &world.social.avatar_names;
    let cached = if object || logged {
        None
    } else {
        names.get(if own { &world.agent_id } else { &line.source })
    };
    match cached {
        Some(n) => n.display(&names.options),
        None if own && !logged => own_name.to_owned(),
        None => world.rlva_filter_text(&line.from).into_owned(),
    }
}

/// The text of a message and whether it is in italics: an emote is the
/// sender's name in italics, a shout or a whisper says so.
pub(crate) fn line_body(line: &crate::world::ChatLine, name: &str) -> (String, bool) {
    let emote = line.text.starts_with("/me ") || line.text.starts_with("/me'");
    if emote {
        (format!("{}{}", name, &line.text[3..]), true)
    } else {
        let prefix = match line.kind.volume() {
            Some(ChatType::Shout) => aurora_i18n::tr!("status-shouts-2"),
            Some(ChatType::Whisper) => aurora_i18n::tr!("status-whispers-2"),
            _ => "",
        };
        (format!("{prefix}{}", line.text), false)
    }
}

/// The text a line shows, as a copy of it gives it: links as written,
/// places, names and mentions as shown.
fn shown_text(world: &World, line: &crate::world::ChatLine, name: &str) -> String {
    let body = match line.kind {
        ChatKind::System | ChatKind::Invite(_) | ChatKind::InventoryOffer(_) | ChatKind::Payment(_) => line.text.clone(),
        _ => line_body(line, name).0,
    };
    let text: String = segments(&body)
        .into_iter()
        .map(|s| match s {
            Seg::Text(t) | Seg::Url(t) | Seg::Slurl(t) => t.to_owned(),
            Seg::Labelled(label, _) => label.into_owned(),
            Seg::Place(_, place) => place.label,
            Seg::Mention(id) => format!(" @{} ", world.social.name_of(&id)),
            Seg::Agent(id) => world.social.name_of(&id),
            Seg::App(u, link) => app_links::label(world, &mut HashSet::new(), u, &link),
        })
        .collect();
    world.rlva_filter_text(&text).into_owned()
}

/// What a copied line starts with when it is taken from its start, as
/// Firestorm's transcript lines: "[12:34] Name: " ("[12:34] " for an emote
/// or a notice, whose text says who).
fn copy_prefix(line: &crate::world::ChatLine, name: &str) -> String {
    let time = if line.time == std::time::UNIX_EPOCH {
        String::new()
    } else {
        format!("[{}] ", hhmm(line.time))
    };
    let emote = line.text.starts_with("/me ") || line.text.starts_with("/me'");
    if line.kind.is_notice() || emote {
        time
    } else {
        format!("{time}{name}: ")
    }
}

/// What the keyboard given to the lines leaves for the conversation's field.
enum ToField {
    Nothing,
    /// Enter: the field takes the keyboard.
    Focus,
    /// Typed text: it goes on in the field.
    Text(String),
}

/// What a copy of the lines reads.
struct Copied<'a> {
    view: &'a view::View,
    lines: &'a std::collections::VecDeque<crate::world::ChatLine>,
    first: u64,
    world: &'a World,
    own_name: &'a str,
}

impl Copied<'_> {
    fn text(&self, sel: &select::Selection) -> String {
        let seqs = self.view.lines().map(|i| self.first + i as u64);
        sel.copy_text(seqs, |seq| {
            let Some(line) = seq.checked_sub(self.first).and_then(|i| self.lines.get(i as usize)) else {
                return (String::new(), String::new());
            };
            let name = sender_name(self.world, self.own_name, line);
            (copy_prefix(line, &name), shown_text(self.world, line, &name))
        })
    }

    /// The first and last lines shown.
    fn ends(&self) -> Option<(u64, u64)> {
        let first = self.view.lines().next()?;
        let last = self.view.lines().last()?;
        Some((self.first + first as u64, self.first + last as u64))
    }
}

/// The selection of the lines (`select.rs`), as in Firestorm's transcript:
/// a drag selects across them (never moving the window), a double click
/// selects a word, a click without a drag clears it; a press gives them the
/// keyboard, then Ctrl+A selects the
/// whole conversation, Ctrl+C (Ctrl+Insert) copies the selection, Escape
/// clears it, and what is typed goes to the conversation's field.
fn select_lines(
    ui: &mut egui::Ui,
    content: &egui::Response,
    drawn: &[select::Row],
    sel: &mut select::Selection,
    copied: &Copied,
) -> ToField {
    // the lines' own area: egui also hands them a pointer a few points off
    // (interact_radius), on the strip between them and the field, which
    // stays neutral (no selection, no keyboard)
    let area = content.rect.intersect(ui.clip_rect());
    let pressed_in = ui.input(|i| i.pointer.press_origin()).is_some_and(|p| area.contains(p));
    let pressed = ui.input(|i| i.pointer.primary_pressed());
    if pressed && pressed_in && content.is_pointer_button_down_on() {
        content.request_focus();
    }
    if content.drag_started_by(egui::PointerButton::Primary)
        && pressed_in
        && let Some(origin) = ui.input(|i| i.pointer.press_origin())
        && let Some((p, text)) = select::hit(drawn, origin)
    {
        sel.start_at(p, text);
    }
    if content.dragged_by(egui::PointerButton::Primary)
        && pressed_in
        && let Some(pos) = content.interact_pointer_pos()
    {
        if let Some((p, text)) = select::hit(drawn, pos) {
            sel.extend_to(p, text);
        }
        // beyond the top or the bottom, the lines scroll under the pointer
        let clip = ui.clip_rect();
        let beyond = if pos.y < clip.top() {
            clip.top() - pos.y
        } else if pos.y > clip.bottom() {
            clip.bottom() - pos.y
        } else {
            0.0
        };
        if beyond != 0.0 {
            ui.scroll_with_delta(egui::vec2(0.0, (beyond * 0.5).clamp(-20.0, 20.0)));
            ui.ctx().request_repaint();
        }
    }
    if content.clicked() && content.interact_pointer_pos().is_some_and(|p| area.contains(p)) {
        sel.clear();
    }
    // a double click selects the word under the pointer (its second click
    // cleared the selection just above)
    if content.double_clicked()
        && let Some(pos) = content.interact_pointer_pos().filter(|p| area.contains(*p))
        && let Some((seq, range, text)) = select::word_at(drawn, pos)
    {
        sel.select_in_row(seq, range, text);
    }
    // (no text cursor over the lines, though a drag selects them: the
    // arrow, the hand on links and names; the human's choice of 2026-10-16)
    if content.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        sel.clear();
    }
    if !content.has_focus() {
        return ToField::Nothing;
    }
    // the arrows stay here instead of moving the keyboard to another widget
    ui.memory_mut(|m| {
        m.set_focus_lock_filter(
            content.id,
            egui::EventFilter {
                horizontal_arrows: true,
                vertical_arrows: true,
                ..Default::default()
            },
        );
    });
    let mut to_field = ToField::Nothing;
    for event in ui.input(|i| i.events.clone()) {
        match event {
            egui::Event::Key {
                key: egui::Key::A,
                pressed: true,
                modifiers,
                ..
            } if modifiers.command && !modifiers.alt => {
                if let Some((first, last)) = copied.ends() {
                    sel.select_all(first, last);
                }
            }
            egui::Event::Copy if !sel.is_empty() => ui.ctx().copy_text(copied.text(sel)),
            egui::Event::Key {
                key: egui::Key::Enter,
                pressed: true,
                ..
            } => to_field = ToField::Focus,
            egui::Event::Text(t) if !t.chars().any(char::is_control) => match &mut to_field {
                ToField::Text(s) => s.push_str(&t),
                _ => to_field = ToField::Text(t),
            },
            _ => {}
        }
    }
    to_field
}

/// `AURORA_DEMO_CONV_SELECT`: "all" as Ctrl+A, "part" from the middle of the
/// last line drawn but two to the middle of the last one.
fn demo_selection(sel: &mut select::Selection, mode: &str, drawn: &[select::Row], ends: Option<(u64, u64)>) {
    if mode == "part" {
        let middle = |r: &select::Row| select::Point {
            seq: r.seq,
            at: r.text.chars().count() / 2,
        };
        let (Some(a), Some(b)) = (drawn.get(drawn.len().saturating_sub(3)), drawn.last()) else {
            return;
        };
        sel.start_at(middle(a), &a.text);
        sel.extend_to(middle(b), &b.text);
    } else if let Some((first, last)) = ends {
        sel.select_all(first, last);
    }
}

/// The conversation shown is read: its unread count goes, and in the
/// workspace its first unread line is kept for the "New" line.
fn mark_read(world: &mut World, selected: Option<uuid::Uuid>) -> Option<(usize, usize)> {
    match selected {
        None => {
            let unread = world.nearby_read();
            Some((unread, world.chat.len()))
        }
        Some(id) => world
            .social
            .ims
            .iter_mut()
            .find(|s| s.other == id)
            .map(|s| (std::mem::take(&mut s.unread), s.lines.len())),
    }
}

/// What the lines of a conversation are drawn with.
struct Lines<'a> {
    icons: &'a super::icons::Icons,
    emoji: &'a mut super::emoji::Emoji,
    pics: &'a HashMap<uuid::Uuid, egui::TextureHandle>,
    world: &'a World,
    own_name: &'a str,
    /// The height they take.
    height: f32,
}

/// How they are drawn.
enum Rows<'t, 'a> {
    /// The classic lines, or the bubbles (`LineEnv::bubbles`).
    Classic(Option<bool>),
    /// The rows of Aurora's workspace (`thread.rs`).
    Thread(&'t mut thread::Thread<'a>),
}

/// The lines of the conversation shown, laid out by its view (only those in
/// sight are drawn), with the find bar's highlights and the selection.
fn conversation_lines(ui: &mut egui::Ui, p: &Palette, l: Lines, st: &mut ChatUi, mut rows: Rows) -> ToField {
    let Lines {
        icons,
        emoji,
        pics,
        world: w,
        own_name,
        height,
    } = l;
    let search = st.searches.entry(st.selected).or_default();
    let filter = search.filter.as_deref().unwrap_or("").to_lowercase();
    let needle = search.find.as_ref().map(|f| f.text.to_lowercase()).unwrap_or_default();
    let view = st.views.entry(st.selected).or_default();
    if st.selection.0 != st.selected {
        st.selection = (st.selected, select::Selection::default());
    }
    let layout = match &rows {
        Rows::Classic(_) => view::Layout::Classic,
        Rows::Thread(th) if th.compact => view::Layout::Compact,
        Rows::Thread(_) => view::Layout::Aurora,
    };
    let mut to_field = ToField::Nothing;
    let (lines, clock) = match st.selected {
        None => (Some(&w.chat), w.chat_clock),
        Some(id) => match w.social.ims.iter().find(|s| s.other == id) {
            Some(s) => (Some(&s.lines), s.clock),
            None => (None, w.chat_clock),
        },
    };
    let menu = ui.layer_id().id.with(("chat_line_menu", st.selected));
    let paused = line_menu::is_open(ui.ctx(), menu);
    if paused && !view.is_paused() {
        st.new_lines.pause_at(st.selected, view.next());
    }
    let resume = view.pause(paused, line_menu::selected(ui.ctx(), menu));
    let first_unread = layout.workspace().then(|| st.new_lines.at(st.selected)).flatten();
    st.previews.place = match st.selected {
        None => crate::link_preview::Place::Nearby,
        Some(id) if w.is_group_session(&id) => crate::link_preview::Place::Group,
        Some(_) => crate::link_preview::Place::Im,
    };
    st.previews.update(ui.ctx());
    let mut env = LineEnv {
        p,
        icons,
        emoji,
        pics,
        world: w,
        want_names: &mut st.wanted_names,
        wanted: &mut st.wanted_pics,
        own_name,
        plain_history: false,
        compact_automated: st.compact_automated,
        compact_notices: object_row::compact_notices(st.compact_automated, st.compact_im_notices, st.selected.is_none()),
        menu: Some(menu),
        bubbles: match rows {
            Rows::Classic(style) => style,
            Rows::Thread(_) => None,
        },
        previews: Some(&mut st.previews),
    };
    let wash = find::wash(p);
    let current = search.find.as_ref().and_then(|f| f.current_line(view));
    let area = super::widgets::scroll_offset::show(ui, egui::ScrollArea::vertical(), ("conv", st.selected));
    let out = area
        .stick_to_bottom(!paused)
        .auto_shrink([false, false])
        .max_height(height)
        .min_scrolled_height(height)
        .show_viewport(ui, |ui, viewport| {
            let Some(lines) = lines else {
                return 0.0;
            };
            let width = (ui.available_width() * 2.0).round() / 2.0;
            let changed = view.sync(
                &view::Source {
                    lines,
                    clock,
                    filter: &filter,
                    find: &needle,
                    layout,
                    first_unread,
                },
                width,
            );
            if changed && let Some(f) = search.find.as_mut() {
                f.hits_changed(view);
            }
            // (not in the window's sizing pass, whose scroll area is another)
            if st.demo_scroll.is_some() && st.demo_scroll == st.selected && view.row(0).is_some() && !ui.is_sizing_pass() {
                if view.below() > 0.0 {
                    (st.demo_scroll, st.demo_scrolled) = (None, true);
                } else {
                    view.scroll_to(0);
                }
            }
            let mark = (!needle.is_empty()).then_some(Mark { needle: &needle, wash });
            let selection = &mut st.selection.1;
            let (shift, content) = view.show(ui, viewport, clock.first, &mut |ui, row, painted| {
                let Some(line) = lines.get(row.line) else {
                    return;
                };
                let mark = mark.filter(|_| row.hit);
                find::forget_message(ui.ctx());
                let mut draw = |ui: &mut egui::Ui| match &mut rows {
                    Rows::Classic(_) => draw_line(ui, &mut env, line, row.look, mark),
                    Rows::Thread(th) => thread::draw(ui, &mut env, th, line, row.look, mark, painted),
                };
                if line.kind.is_card() {
                    // a card: its own labels and buttons
                    draw(ui);
                } else {
                    let seq = clock.first + row.line as u64;
                    line_menu::highlight(ui, menu, seq, line, p, |ui| {
                        select::row(ui, seq, selection.of_row(seq), painted, draw);
                    });
                }
                if painted && current == Some(row.line) {
                    find::current_marker(ui, p);
                }
            });
            let drawn = select::take_drawn();
            let copied = Copied {
                view,
                lines,
                first: clock.first,
                world: w,
                own_name,
            };
            if let Some(mode) = st.demo_select.take_if(|_| !drawn.is_empty()) {
                demo_selection(selection, &mode, &drawn, copied.ends());
            }
            to_field = select_lines(ui, &content, &drawn, selection, &copied);
            if matches!(rows, Rows::Thread(_)) {
                line_menu::demo_mark(ui, &drawn);
                let first = clock.first;
                line_menu::message(
                    &content,
                    menu,
                    &drawn,
                    |seq| seq.checked_sub(first).and_then(|i| lines.get(i as usize)),
                    |seq| selection.of_row(seq).map(|_| copied.text(selection)),
                );
            }
            shift
        });
    let mut offset = out.state.unclamped_offset();
    // rows above changed height: what is in sight stays
    offset.y += out.inner;
    let mut store = out.inner != 0.0;
    // what came in since the reader left the end: others' messages, not
    // ours nor the notices
    let arrived = lines.map_or(0, |lines| {
        view.arrived(clock.first, |i| {
            lines.get(i).is_some_and(|l| !l.kind.is_own() && !l.kind.is_notice())
        })
    });
    let latest_id = ui.scope_id().with(("latest", st.selected));
    if resume || (!ui.is_sizing_pass() && latest::show(ui, p, latest_id, out.inner_rect, view.below(), arrived)) {
        // past the end: the scroll area stops there and sticks to it again
        offset.y = out.content_size.y;
        store = true;
    }
    if store {
        super::widgets::scroll_offset::set(ui.ctx(), out.id, offset);
    }
    line_menu::show(ui, menu, p, |ui, target| {
        if let Some(avatar) = target.sender {
            super::context::chat_sender_menu(ui, p, w, avatar, &target.line);
        } else if let Rows::Thread(th) = &mut rows {
            thread::message_menu(ui, &env, th, target);
        }
    });
    to_field
}

/// What drawing a line of a conversation needs.
pub(crate) struct LineEnv<'a> {
    pub p: &'a Palette,
    pub icons: &'a super::icons::Icons,
    pub emoji: &'a mut super::emoji::Emoji,
    pub pics: &'a HashMap<uuid::Uuid, egui::TextureHandle>,
    pub world: &'a World,
    /// Names to resolve (mentions, agent links).
    pub want_names: &'a mut HashSet<uuid::Uuid>,
    /// Profile pictures to load.
    pub wanted: &'a mut HashSet<uuid::Uuid>,
    pub own_name: &'a str,
    /// History lines in the colors of new ones (the History window, where
    /// every line is history; Firestorm's is_conversation_log).
    pub plain_history: bool,
    /// Objects' lines on one small row like the notices (`compact_object`).
    pub compact_automated: bool,
    /// The viewer's notices on that row too, with their topic's icon
    /// (`object_row::compact_notices`).
    pub compact_notices: bool,
    /// A transcript-owned popup; None for nearby-chat toasts and fixtures.
    pub menu: Option<egui::Id>,
    /// The lines as bubbles (`bubbles.rs`), with the sender's name opening
    /// each group when `Some(true)` (several people write); `None`: the
    /// classic lines with their header strips.
    pub bubbles: Option<bool>,
    /// The cards under live links in all conversation styles; None in the
    /// History window and the on-screen chat.
    pub previews: Option<&'a mut previews::Previews>,
}

/// One line of a conversation: the day when it changes, the sender's header
/// strip (picture, name, time) when `look` asks for it, then the text.
pub(crate) fn draw_line(ui: &mut egui::Ui, env: &mut LineEnv, line: &crate::world::ChatLine, look: view::Look, mark: Option<Mark>) {
    if let Some(names) = env.bubbles {
        bubbles::draw(ui, env, line, look, mark, names);
        return;
    }
    let (p, world) = (env.p, env.world);
    ui.spacing_mut().item_spacing.y = 2.0;
    let w = ui.available_width();
    // history lines: greyed, their name as written (message_from_log); their
    // sender, when its name was cached, gives the picture and the profile
    let logged = line.origin != Origin::Live;
    let tone = if env.plain_history { None } else { history_tone(p, line.origin) };
    if let Some((year, month, day)) = look.marker {
        date_marker(ui, p, year, month, day);
    }
    // (the current match's marker starts under the date)
    find::note_message(ui, ui.cursor().top(), None);
    if let ChatKind::Invite(invite) = line.kind {
        invite_card::draw_line(ui, env, line, invite);
        return;
    }
    if let ChatKind::InventoryOffer(offer) = line.kind {
        offer_card::draw_line(ui, env, line, offer);
        return;
    }
    if let ChatKind::Payment(payment) = line.kind {
        payment_card::draw_line(ui, env, line, payment);
        return;
    }
    if object_row::compact_in(line, env) {
        // the icon where the header strips have the picture, the text at
        // the lines' text
        let place = object_row::Place {
            icon_center: 14.0,
            text_left: 26.0,
            time_column: None,
        };
        // (the header strips always show the time)
        object_row::draw(ui, env, line, mark, place, true);
        return;
    }
    if line.kind == ChatKind::System {
        chat_text_marked(
            ui,
            p,
            env.emoji,
            world,
            env.want_names,
            &line.text,
            12.5,
            tone.unwrap_or_else(|| super::colors::chat_color(line, world)),
            true,
            mark,
        );
        return;
    }
    let own = line.kind.is_own();
    let object = matches!(line.kind, ChatKind::Object(_) | ChatKind::ObjectIm);
    // FSChatHistoryHeader: the display name, then " - username" in small
    let names = &world.social.avatar_names;
    let cached = if object || logged {
        None
    } else {
        names.get(if own { &world.agent_id } else { &line.source })
    };
    let name = match cached {
        Some(n) => n.display(&names.options),
        None if own && !logged => env.own_name.to_owned(),
        None => line.from.clone(),
    };
    if look.header {
        ui.add_space(6.0);
        let username = cached.and_then(|n| n.chat_username(&names.options));
        egui::Frame::new()
            .fill(p.raised.gamma_multiply(0.55))
            .corner_radius(ui.visuals().window_corner_radius)
            .inner_margin(egui::Margin::symmetric(4, 3))
            .show(ui, |ui| {
                ui.set_width(w - 8.0);
                ui.horizontal(|ui| {
                    if object {
                        let (rect, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
                        let (glyph, tint) = object_row::object_glyph(line, p, super::colors::sender_color(line));
                        if let Some(t) = env.icons.get(glyph) {
                            ui.painter().image(
                                t.id(),
                                rect.shrink(2.0),
                                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                tint,
                            );
                        }
                    } else {
                        if !line.source.is_nil() {
                            env.wanted.insert(line.source);
                        }
                        avatar_pic(ui, p, env.pics.get(&line.source), &name, 20.0);
                    }
                    let col = super::colors::contact_sender_color(line, world)
                        .or(tone)
                        .unwrap_or_else(|| super::colors::sender_color(line));
                    let time = header_time(ui, p, hhmm(line.time));
                    let reserve = time.size().x + ui.spacing().item_spacing.x;
                    let r = header_names(ui, p, &name, username.as_deref(), col, reserve);
                    // the inspector in the Legacy Viewer; Aurora opens the
                    // profile (a history line's when its sender is known)
                    let avatar = match (logged, own) {
                        (false, true) => world.agent_id,
                        _ => line.source,
                    };
                    if !object && !avatar.is_nil() {
                        line_name_menu(&r, p, world, avatar, line, env.menu);
                    }
                    if !object
                        && !avatar.is_nil()
                        && r.on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text(aurora_i18n::tr!("common-view-profile"))
                            .clicked()
                    {
                        super::profile::request_open(ui.ctx(), avatar);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(time);
                    });
                });
            });
        ui.add_space(2.0);
    }
    let (text, italics) = line_body(line, &name);
    let visible = previews::text(env.previews.as_deref(), line, &text, ui.available_width() - 8.0);
    egui::Frame::new()
        .inner_margin(egui::Margin {
            left: 6,
            right: 2,
            top: 0,
            bottom: 0,
        })
        .show(ui, |ui| {
            if !visible.is_empty() {
                chat_text_marked(
                    ui,
                    p,
                    env.emoji,
                    world,
                    env.want_names,
                    &visible,
                    13.5,
                    tone.unwrap_or_else(|| super::colors::chat_color(line, world)),
                    italics,
                    mark,
                );
            }
            // Aurora also offers its previews in Lines (#976), extending
            // LLChatHistory::appendMessage (indra/newview/llchathistory.cpp).
            previews::cards(ui, env, line, &text);
        });
}

/// Width of the conversation list and of its entries (the vertical tabs of
/// FSFloaterIMContainer have a fixed width too: tab_width="115").
const LIST_W: f32 = 170.0;
const ENTRY_W: f32 = 166.0;
const LIST_ENTRY_H: f32 = 34.0;
/// Side of the picture of a list entry.
const LIST_PIC: f32 = 22.0;
/// Side of the slot every list entry has at its right edge, for its unread
/// dot or its close button: both are centered in it, on one vertical line
/// down the list.
const SLOT_SIDE: f32 = 18.0;

/// What the user did with an entry of the conversation list.
struct ListEntry {
    resp: egui::Response,
    /// Its close button was clicked.
    close: bool,
}

/// The slot at the right of an entry: its unread dot, replaced on an entry
/// that can be closed by the close button when it is selected or under the
/// pointer (a selected conversation is read: it has no dot to show).
fn trailing_slot(entry: egui::Rect) -> egui::Rect {
    let center = egui::pos2(entry.right() - 6.0 - SLOT_SIDE * 0.5, entry.center().y);
    egui::Rect::from_center_size(center, egui::Vec2::splat(SLOT_SIDE))
}

/// Tooltip of the close buttons of a conversation: closing a group or
/// conference session leaves it (LLIMMgr::leaveSession), which is worth
/// saying before the click.
fn close_tip(world: &World, session: &uuid::Uuid) -> &'static str {
    if world.is_group_session(session) {
        aurora_i18n::tr!("ui-chat-close-conversation-leave")
    } else {
        aurora_i18n::tr!("ui-chat-close-conversation")
    }
}

/// The snooze of a group chat, wherever it is offered.
const SNOOZE_ICON: &str = "alarm-snooze-filled";

/// A group chat, which can be snoozed (FSFloaterIM: setCanSnooze for
/// IM_SESSION_GROUP_START and the invitations of our groups).
fn can_snooze(world: &World, session: &uuid::Uuid) -> bool {
    world.groups.sessions.get(session).is_some_and(|s| s.group)
}

/// One entry of the conversation list: picture (or initials), name, unread
/// dot. The name is cut with an ellipsis before the dot and the edge of the
/// entry, as the tab labels of Firestorm's conversations window are
/// (use_ellipses="true" in floater_fs_im_container.xml), and the whole name
/// shows on hover when it was cut (LLTabContainer::addLabelToolTip).
///
/// `close` is the tooltip of the close button of an entry that can be closed
/// (the X of a tab of FSFloaterIMContainer; the nearby chat has none): the
/// button shows on the selected entry and under the pointer, in a slot the
/// name never enters, so nothing moves when it appears.
fn list_entry(
    ui: &mut egui::Ui,
    p: &Palette,
    label: &str,
    pic: Option<(Option<&egui::TextureHandle>, &str)>,
    status: Option<presence::Status>,
    sel: bool,
    unread: usize,
    close: Option<&str>,
    // the violet wash of a conversation with news (`flash.rs`), 0 for none
    flash: f32,
    // it drags out of the window (the nearby chat's, #910); a closable
    // entry drags anyway, to its place too
    drags: bool,
) -> ListEntry {
    // narrower beside the list's scroll bar, when it shows
    let width = ENTRY_W.min(ui.available_width());
    let sense = if close.is_some() || drags {
        egui::Sense::click_and_drag()
    } else {
        egui::Sense::click()
    };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, LIST_ENTRY_H), sense);
    // the close button stands over the entry and takes its hover: the entry
    // keeps its hovered face while the pointer is on the button
    let over = resp.hovered() || (close.is_some() && ui.rect_contains_pointer(rect));
    let fill = if sel {
        p.violet.gamma_multiply(0.22)
    } else if over {
        p.field.gamma_multiply(1.4)
    } else {
        p.field
    };
    // square, like the rows of the Contacts tab next to it: the bar of the
    // selected entry runs flush along its left edge
    ui.painter().rect_filled(rect, 0.0, fill);
    if flash > 0.0 && !sel {
        ui.painter().rect_filled(rect, 0.0, p.violet.gamma_multiply(flash));
    }
    if sel {
        ui.painter()
            .rect_filled(egui::Rect::from_min_size(rect.min, egui::vec2(2.0, rect.height())), 0.0, p.violet);
    }
    let mut x = rect.left() + 8.0;
    if let Some((tex, name)) = pic {
        let r = egui::Rect::from_min_size(egui::pos2(x, rect.center().y - LIST_PIC * 0.5), egui::Vec2::splat(LIST_PIC));
        // a resident's picture is round (it has a status), a group's square
        if status.is_some() {
            super::widgets::avatar_disc(ui, p, r, tex, name, false);
        } else {
            super::widgets::avatar_picture(ui, p, r, tex, name, false);
        }
        if let Some(s) = status {
            // cut in the entry's face as it shows over the window (#744)
            presence::paint_dot(ui.painter(), p, r, s, p.panel.blend(fill));
        }
        x += LIST_PIC + 6.0;
    }
    let slot = trailing_slot(rect);
    let dot = slot.center();
    // the unread count, the interface's one counter ending on the slot's
    // right edge
    let count = (unread > 0).then(|| {
        super::widgets::UnreadCount::new(unread, super::widgets::CountSize::List).place(
            ui.ctx(),
            p,
            egui::pos2(slot.right() - 1.0, dot.y),
            egui::Align2::RIGHT_CENTER,
        )
    });
    // the name stops before the slot of an entry that can be closed (its
    // button comes and goes with the pointer), before the unread count of
    // another, else before the entry's edge
    let text_right = match (&count, close.is_some()) {
        (_, true) => slot.left() - 4.0,
        (Some(count), false) => count.rect.left() - 4.0,
        (None, false) => rect.right() - 6.0,
    };
    let cut = super::widgets::paint_elided(
        ui,
        label,
        egui::pos2(x, rect.center().y),
        text_right - x,
        egui::FontId::proportional(12.0),
        if sel { p.ink } else { p.muted },
    );
    let mut closed = false;
    match close {
        Some(tip) if sel || over => {
            let button = ui.interact(slot, resp.id.with("close"), egui::Sense::click());
            button.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
            if button.hovered() {
                ui.painter()
                    .rect_filled(slot, ui.visuals().widgets.inactive.corner_radius, p.raised);
            }
            if let Some(t) = super::icons::global("x") {
                ui.painter().image(
                    t.id(),
                    egui::Rect::from_center_size(slot.center(), egui::Vec2::splat(12.0)),
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    // the color of the entry's name, lit under the pointer
                    if sel || button.hovered() { p.ink } else { p.muted },
                );
            }
            closed = button.on_hover_text(tip).clicked();
        }
        _ => {
            if let Some(count) = count {
                count.paint(ui.painter());
            }
        }
    }
    ListEntry {
        resp: if cut { resp.on_hover_text(label) } else { resp },
        close: closed,
    }
}

/// The part of the conversation list that scrolls, in the `height` left of
/// its column: with more conversations than fit, the wheel or the bar that
/// shows at its right edge reaches the others. An entry asked into view
/// (`scroll_to_me`) is there at once, the window may just have opened.
fn scrolling_list<R>(ui: &mut egui::Ui, height: f32, entries: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::ScrollArea::vertical()
        .id_salt("conversation_list")
        .max_height(height.max(LIST_ENTRY_H))
        .animated(false)
        .show(ui, entries)
        .inner
}

/// The entry of the tabs' list being dragged, under the pointer, as the
/// tab shown, with the mark of a window of its own while a drop pops it
/// out there (#910).
fn list_ghost(
    ctx: &egui::Context,
    p: &Palette,
    g: reorder::Ghost,
    label: &str,
    pic: Option<Option<&egui::TextureHandle>>,
    status: Option<presence::Status>,
) {
    super::widgets::drag_ghost(ctx, p, "conversation_drag_ghost", g.pos, |ui| {
        ui.set_width(g.size.x);
        // the list's colour under it, which the entry's own face washes
        let face = egui::Rect::from_min_size(ui.cursor().min, g.size);
        ui.painter().rect_filled(face, 0.0, p.bar);
        let entry = list_entry(ui, p, label, pic.map(|t| (t, label)), status, true, 0, None, 0.0, false);
        if g.out {
            popout::paint_mark(ui, p, trailing_slot(entry.resp.rect));
        }
    });
}

/// Size of the window before the user resizes it, where there is room.
const DEFAULT_SIZE: egui::Vec2 = egui::vec2(700.0, 440.0);
/// The least height it opens with, in a small viewer window.
const DEFAULT_MIN_H: f32 = 240.0;

/// The window before the user moves or resizes it: the bottom right corner
/// of the work area (`layout::work_area`), above the bottom bar and on the
/// right edge of the mini-map and Performance windows of a first launch. In
/// a viewer window too low for its whole height under Performance
/// (`perf::default_pos`), it opens lower rather than over it (284 px in a
/// 1280 x 720 window; FSFloaterIMContainer opens at 396 x 390).
pub(crate) fn default_rect(work: egui::Rect) -> egui::Rect {
    let gap = super::layout::GAP;
    let performance_bottom = super::perf::default_pos(work, super::perf::COMPACT_W).y + super::perf::COMPACT_H;
    let room = work.bottom() - gap - (performance_bottom + gap);
    let size = egui::vec2(DEFAULT_SIZE.x, room.clamp(DEFAULT_MIN_H, DEFAULT_SIZE.y));
    egui::Rect::from_min_size(work.right_bottom() - egui::Vec2::splat(gap) - size, size)
}

/// Conversations floater (FSFloaterIMContainer): the tabs on the left
/// (Contacts, nearby chat, sessions); on the right the contacts, or the
/// toolbar, grouped messages and input (with emoji picker).
#[allow(clippy::too_many_arguments)]
pub fn show(
    ctx: &egui::Context,
    p: &Palette,
    icons: &super::icons::Icons,
    emoji: &mut super::emoji::Emoji,
    pics: &HashMap<uuid::Uuid, egui::TextureHandle>,
    images: &HashMap<uuid::Uuid, egui::TextureHandle>,
    world: &mut World,
    gestures: &crate::gestures::Manager,
    st: &mut ChatUi,
    contacts: &mut super::contacts::ContactsUi,
    contacts_settings: &mut super::contacts::ContactsSettings,
    show_contacts_tab: bool,
    // People's friends tab, the workspace's Contacts tab in Aurora's look of
    // People (None: the contact list as it is)
    friends: Option<super::people::FriendsPane<'_, '_>>,
    open: &mut bool,
) -> Vec<ConvAction> {
    let enlarged = st.previews.enlarged();
    st.previews.lightbox_keys(ctx);
    st.reorder.begin(ctx, world);
    // what the thumbnails' menus asked, drawn with the bars before
    let mut actions = std::mem::take(&mut st.thumbnails.actions);
    let mut friends = friends;
    let default = default_rect(super::layout::work_area(ctx));
    let opening_conversation = world.social.focus_im.is_some();
    if let Some(f) = world.social.focus_im.take() {
        let conversation = (!f.is_nil()).then_some(f);
        if popout::opens_in_own_window(&st.popped, conversation, st.own_windows) {
            // the keyboard asked of the Conversations window goes to its own
            let focus = std::mem::take(&mut st.conv_focus_request);
            st.popped.pop_out(conversation, st.members_open, st.stage_mini, focus);
            // and the Conversations window, opened for it, stays closed
            if !st.main_open {
                *open = false;
            }
        } else {
            st.open(f);
        }
    }
    let docked = !contacts_settings.torn_off;
    if !show_contacts_tab && st.contacts {
        st.selected = None;
    }
    if !docked || !show_contacts_tab {
        st.contacts = false;
    }
    let style = st.demo_style.unwrap_or(st.style);
    popout::one_picker(st);
    st.gifs.close_if_hidden(ctx.cumulative_pass_nr());
    // every entry of the window, whatever the column's search: it never
    // shows a conversation that is in its own window (`popout.rs`)
    let entries: Vec<Option<uuid::Uuid>> = std::iter::once(None)
        .chain(st.reorder.sessions(world).into_iter().map(|(id, _)| Some(id)))
        .collect();
    let select_main = |st: &mut ChatUi, open: &mut bool| {
        if st.contacts {
            return;
        }
        match popout::main_selection(&st.popped, st.selected, &entries) {
            Some(conversation) => st.selected = conversation,
            None if show_contacts_tab && docked => st.contacts = true,
            // LLMultiFloater::tabClose (indra/llui/llmultifloater.cpp):
            // close an empty container without reattaching its last tab.
            None => *open = false,
        }
    };
    select_main(st, open);
    let minimized = super::widgets::floater_minimized(ctx, "conversations");
    // Opening a specific session keeps its existing composer/call focus rules.
    if *open && !minimized && !st.contacts && !opening_conversation && (!st.main_open || st.main_was_minimized) {
        st.conv_focus_request = true;
    }
    // Alt + the arrows and Ctrl+W, before the field reads them (`keys.rs`),
    // over the entries without a window of their own
    if *open && !enlarged {
        let mut order = if style.workspace() {
            sidebar::tab_order(world, st)
        } else {
            keys::legacy_order(world, st)
        };
        order.retain(|c| !st.popped.contains(*c));
        keys::handle(ctx, st, &order, &mut actions);
    }
    let title = match st.selected {
        // the workspace's head names the conversation (the mockup's title)
        _ if style.workspace() => aurora_i18n::tr!("common-conversations").to_owned(),
        _ if st.contacts => aurora_i18n::tr!("ui-chat-conversations-contacts").to_owned(),
        None => aurora_i18n::tr!("ui-chat-conversations-nearby-chat").to_owned(),
        Some(id) => aurora_i18n::tr!("ui-chat-conversations", value0 = world.session_title(&id).to_string()),
    };
    let own_name = world.own_name();
    let mut tear_off = false;
    let mut detach = false;
    let mut floater = super::widgets::Floater::new("conversations", title, default.min, default.size())
        .chat()
        .attention(conversations_waiting(world))
        .attention_flash(st.button_flash.bar(ctx.input(|i| i.time)))
        .steady_size();
    if st.contacts {
        // the tear-off button of the multi-floater, for the Contacts tab
        floater = floater.action("external-link", aurora_i18n::tr!("common-detach"), &mut tear_off);
    } else {
        // Nearby chat can detach even before any IM opens (#995), as
        // can_tear_off in floater_fs_nearby_chat.xml does.
        floater = floater.action("external-link", aurora_i18n::tr!("common-detach"), &mut detach);
    }
    popout::share(ctx, &st.popped);
    if style.workspace() {
        floater = floater.flush_body();
    }
    // about the least the window lays out without overflowing (the
    // workspace's rail, head, 60 points of messages and field; the Legacy
    // style's tabs and field): smaller, it grew back to its content during
    // a resize and the messages, laid out for the smaller size first,
    // hopped for a frame
    let least = egui::vec2(if style.workspace() { 300.0 } else { 0.0 }, 240.0);
    floater.min_size = floater.min_size.max(least);
    let image_bounds = floater.show(ctx, p, open, |ui| {
        let bounds = ui.available_rect_before_wrap().intersect(ui.clip_rect());
        if enlarged {
            ui.disable();
        }
        if style.workspace() {
            let env = workspace::Env {
                p,
                icons,
                emoji: &mut *emoji,
                pics,
                images,
                gestures,
                contacts: &mut *contacts,
                contacts_settings: &mut *contacts_settings,
                show_contacts_tab,
                friends: friends.take(),
                docked,
                compact: style == crate::settings::ChatStyle::Compact,
            };
            workspace::show(ui, env, world, st, &mut actions);
            return bounds;
        }
        let h = (ui.available_height() - 6.0).max(140.0);
        ui.horizontal_top(|ui| {
            // ---- contacts column
            ui.vertical(|ui| {
                ui.set_width(LIST_W);
                ui.spacing_mut().item_spacing.y = 2.0;
                let top = ui.cursor().top();
                // Detached tabs leave their host (LLMultiFloater::removeFloater).
                if show_contacts_tab
                    && docked
                    && list_entry(
                        ui,
                        p,
                        aurora_i18n::tr!("common-contacts"),
                        None,
                        None,
                        st.contacts,
                        0,
                        None,
                        0.0,
                        false,
                    )
                    .resp
                    .clicked()
                {
                    st.contacts = true;
                }
                if !st.popped.contains(None) {
                    let local_shown = st.selected.is_none() && !st.contacts;
                    let local = list_entry(
                        ui,
                        p,
                        aurora_i18n::tr!("common-nearby-chat"),
                        None,
                        None,
                        local_shown,
                        if local_shown { 0 } else { world.chat_badge },
                        None,
                        0.0,
                        true,
                    )
                    .resp;
                    conv_menu::demo_mark(ui.ctx(), world, st, None, local.rect);
                    conv_menu::attach(&local, p, world, &mut actions, None);
                    st.reorder.local_row(&local);
                    if st.reorder.dragging(None)
                        && let Some(g) = st.reorder.ghost(ui.ctx())
                    {
                        list_ghost(ui.ctx(), p, g, aurora_i18n::tr!("common-nearby-chat"), None, None);
                    }
                    if local.clicked() {
                        st.select_tab(None);
                    }
                }
                let sessions = st.reorder.sessions(world);
                // The conversations scroll under the pinned entries above, which
                // stay in reach (the vertical tabs of FSFloaterIMContainer
                // scroll with the arrows of LLTabContainer, all together).
                // (kept until its entry is there: the conversation may open
                // a frame before its first message lists it)
                let reveal = st.reveal;
                let left = h - (ui.cursor().top() - top);
                let now = ui.input(|i| i.time);
                st.flashes.begin(now);
                scrolling_list(ui, left, |ui| {
                    for (other, unread) in sessions {
                        if st.popped.contains(Some(other)) {
                            continue;
                        }
                        let shown = st.selected == Some(other) && !st.contacts;
                        let (flash, moving) = st.flashes.wash(other, if shown { 0 } else { unread }, now, st.flash_unread);
                        if moving {
                            ui.ctx().request_repaint();
                        }
                        let name = world.session_title(&other);
                        if !world.is_group_session(&other) {
                            st.wanted_pics.insert(other);
                        }
                        // FSFloaterIM::addTypingIndicator: the tab of a
                        // resident typing to us reads "Nova is typing…", by
                        // their display name alone, so that it shows in a
                        // narrow tab; "> " first with FSTypingChevronPrefix
                        let typing = (sidebar::kind(world, &other) == sidebar::Kind::Private && world.social.is_typing(&other))
                            .then(|| typing::text(&[world.social.display_name_of(&other)]))
                            .flatten()
                            .map(|t| if st.typing_chevron { format!("> {t}") } else { t });
                        st.reorder.space_before(ui, other);
                        let status = (!world.is_group_session(&other)).then(|| presence::of(world, &other));
                        let entry = list_entry(
                            ui,
                            p,
                            typing.as_deref().unwrap_or(&name),
                            Some((pics.get(&other), &name)),
                            status,
                            shown,
                            unread,
                            Some(close_tip(world, &other)),
                            flash,
                            true,
                        );
                        if reveal == Some(other) {
                            entry.resp.scroll_to_me(None);
                            st.reveal = None;
                        }
                        conv_menu::demo_mark(ui.ctx(), world, st, Some(other), entry.resp.rect);
                        conv_menu::attach(&entry.resp, p, world, &mut actions, Some(other));
                        st.reorder.row(world, other, &entry.resp);
                        st.reorder.space_after(ui, other);
                        if st.reorder.dragging(Some(other))
                            && let Some(g) = st.reorder.ghost(ui.ctx())
                        {
                            list_ghost(ui.ctx(), p, g, &name, Some(pics.get(&other)), status);
                        }
                        if entry.close {
                            actions.push(ConvAction::Close(other));
                        } else if entry.resp.clicked() {
                            st.reveal = None;
                            st.select_tab(Some(other));
                        }
                    }
                    if let Some(out) = st.reorder.finish(ui, p, st.demo_reorder) {
                        actions.push(ConvAction::PopOutAt(out.conversation, out.pos));
                    }
                });
            });
            if st.contacts {
                ui.vertical(|ui| {
                    ui.set_min_height(h);
                    let pics = super::contacts::Pics { avatars: pics, images };
                    for a in super::contacts::panel(ui, p, world, contacts, contacts_settings, &pics) {
                        actions.push(ConvAction::Contacts(a));
                    }
                });
                return;
            }
            // ---- conversation pane
            ui.vertical(|ui| classic_pane(ui, p, icons, emoji, pics, gestures, world, st, &mut actions, h, &own_name));
        });
        bounds
    });
    docking::target(ctx, *open && !minimized && image_bounds.is_some());
    st.previews.lightbox(ctx, p, image_bounds.filter(|_| *open));
    if tear_off {
        actions.push(ConvAction::Contacts(super::contacts::ContactsAction::ToggleTornOff));
    }
    if detach {
        actions.push(ConvAction::PopOut(st.selected));
    }
    // popped out or docked back from the title bar and the entries' menus;
    // a window popped out now draws from the next frame, the window above
    // having drawn its conversation in this one
    actions.retain(|a| match a {
        ConvAction::PopOut(c) => {
            st.pop_out(*c);
            false
        }
        ConvAction::DockBack(c) => {
            st.dock_back(ctx, *c, open);
            false
        }
        ConvAction::PopOutAt(c, pos) => {
            st.pop_out(*c);
            super::widgets::move_floater(ctx, &popout::floater_id(*c), *pos);
            false
        }
        _ => true,
    });
    select_main(st, open);
    let env = popout::Env {
        p,
        icons,
        emoji,
        pics,
        images,
        gestures,
        style,
    };
    popout::show(ctx, env, world, st, &mut actions, open);
    st.main_open = *open;
    st.main_was_minimized = minimized;
    // the column's "+" and search: one resident for an IM, several for a
    // conference (LLAvatarActions::startIM / startConference)
    if let Some(residents) = st.picker.show(ctx, p, world) {
        let residents = match st.start {
            Start::Im => residents.into_iter().take(1).collect(),
            Start::Conference => residents,
        };
        world.start_text_conference(residents);
        st.side_filter.clear();
    }
    // our typing notices to the resident written to (FSFloaterIM::setTyping)
    // (the field of the Conversations window or of a window of its own: only
    // a drawn one leaves its frame)
    let field = st.typing_field.take().filter(|_| st.send_typing).unwrap_or(typing::Field {
        to: None,
        typed: false,
        empty: true,
        sent: false,
    });
    for (to, typing) in st.me_typing.update(std::time::Instant::now(), field) {
        actions.push(ConvAction::Typing { to, typing });
    }
    actions
}

/// The conversation shown at the right of the tabs (the Bubbles and
/// Legacy styles), or alone in its own window (`popout.rs`): who it is
/// with and its actions, its call, the participants, its lines and its
/// field (FSFloaterIM, FSFloaterNearbyChat).
#[allow(clippy::too_many_arguments)]
fn classic_pane(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &super::icons::Icons,
    emoji: &mut super::emoji::Emoji,
    pics: &HashMap<uuid::Uuid, egui::TextureHandle>,
    gestures: &crate::gestures::Manager,
    world: &mut World,
    st: &mut ChatUi,
    actions: &mut Vec<ConvAction>,
    h: f32,
    own_name: &str,
) {
    let pane = ui.available_rect_before_wrap();
    let kind = st.selected.map(|id| sidebar::kind(world, &id));
    workspace::drops(ui, p, pane, st, kind, actions);
    // who the conversation is with, and its actions
    header::show(ui, p, icons, pics, world, st, actions);
    ui.add_space(4.0);
    // the call of this conversation, while it is in progress
    if let Some(call) = st.selected.and_then(|s| world.call_of(&s)) {
        call::strip(ui, p, icons, pics, world, call, st, actions);
        ui.add_space(4.0);
    } else if let Some(invite) = st.selected.and_then(|s| world.calls.invite_for(&s)) {
        call::ringing(ui, p, world, invite, actions);
        ui.add_space(4.0);
    }
    // profile card
    if let (true, Some(Some(info))) = (st.show_profile, st.selected.map(|s| world.groups.sessions.get(&s))) {
        // participants of a group / conference session
        egui::Frame::new()
            .fill(p.field)
            .corner_radius(ui.visuals().window_corner_radius)
            .inner_margin(egui::Margin::same(6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let mut names: Vec<(uuid::Uuid, String, bool)> = info
                    .participants
                    .iter()
                    .map(|id| {
                        st.wanted_names.insert(*id);
                        (*id, world.social.name_of(id), info.moderators.contains(id))
                    })
                    .collect();
                names.sort_by_key(|(_, n, _)| n.to_lowercase());
                if names.is_empty() {
                    ui.label(
                        RichText::new(aurora_i18n::tr!("ui-chat-loading-participants"))
                            .size(11.5)
                            .color(p.muted),
                    );
                }
                egui::ScrollArea::vertical().max_height(90.0).show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for (id, n, moderator) in names {
                            let t = RichText::new(n).size(12.0).color(if moderator { p.violet_light } else { p.ink });
                            let r = ui.add(egui::Label::new(t).sense(egui::Sense::click()));
                            name_menu(&r, p, world, id);
                            let tip = if moderator {
                                aurora_i18n::tr!("ui-chat-moderator-click-profile")
                            } else {
                                aurora_i18n::tr!("ui-chat-click-profile")
                            };
                            if r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip).clicked() {
                                actions.push(ConvAction::Profile(id));
                            }
                            ui.add_space(8.0);
                        }
                    });
                });
            });
        ui.add_space(4.0);
    }
    let input_h = input::height(ui.ctx(), egui::Id::unique(("conv_input", st.selected)), INPUT_H);
    // Ctrl+F opens the find field of the conversation shown (in
    // the head's row), when the keyboard is in this window, or
    // nowhere with the pointer over it
    // each conversation's own field (cursor, undo), like its draft
    let conv_input = egui::Id::unique(("conv_input", st.selected));
    let search = st.searches.entry(st.selected).or_default();
    if find::wants_shortcut(ui, &[conv_input, find::field_id(st.selected), find::filter_id(st.selected)]) {
        search.open_find();
    }
    // exactly what the input row takes under them: the spacing after
    // the lines, the 4 points before the row and the row itself (a
    // row that overflows makes the resizable window grow every frame)
    let lines_h = (ui.available_height() - input_h - 4.0 - ui.spacing().item_spacing.y - 1.0).clamp(60.0, h);
    let mut to_field = ToField::Nothing;
    egui::Frame::new()
        .fill(p.field)
        .corner_radius(ui.visuals().window_corner_radius)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            mark_read(world, st.selected);
            let w: &World = world;
            let style = line_style(st.demo_style.unwrap_or(st.style), st.selected.map(|id| w.is_group_session(&id)));
            let lines = Lines {
                icons,
                emoji,
                pics,
                world: w,
                own_name,
                height: lines_h,
            };
            to_field = conversation_lines(ui, p, lines, st, Rows::Classic(style));
        });
    match to_field {
        ToField::Nothing => {}
        ToField::Focus => st.conv_focus_request = true,
        ToField::Text(t) => {
            st.drafts.entry(st.selected).or_default().push_str(&t);
            st.conv_focus_request = true;
        }
    }
    let strip_top = ui.min_rect().bottom();
    ui.add_space(4.0);
    // input + emoji
    let hint = match st.selected {
        None => aurora_i18n::tr!("ui-chat-to-local-chat").to_owned(),
        Some(id) => aurora_i18n::tr!("ui-chat-at", value0 = world.session_title(&id).to_string()),
    };
    let mut emoji_btn = None;
    let mut send_clicked = false;
    let mut draft = std::mem::take(st.draft_mut(st.selected));
    let resp = ui
        .horizontal(|ui| {
            // the edges of the lines above, half the theme's spacing between
            ui.spacing_mut().item_spacing.x *= 0.5;
            let b = tool_resp(ui, p, icons, "mood-smile-filled", st.emoji_open);
            emoji_btn = Some(b.rect);
            if b.on_hover_text(aurora_i18n::tr!("ui-chat-emoji")).clicked() {
                st.emoji_open = !st.emoji_open;
                st.gif_open = false;
            }
            // shown once a key is entered (Preferences › Chat › GIFs)
            if st.gifs.usable() {
                let b = tool_resp(ui, p, icons, "gif", st.gif_open).on_hover_text(aurora_i18n::tr!("ui-chat-gif"));
                gif_picker::button(&b, p, st);
            }
            // leave exactly the send button + spacing (else the row overflows
            // and the resizable window grows every frame)
            let field_w = (ui.available_width() - SEND_BTN_W - ui.spacing().item_spacing.x).floor().max(40.0);
            input::prepare(ui, conv_input, st.selected.is_none());
            let recalled = st.input_history.entry(st.selected).or_default().recall(ui, conv_input, &mut draft);
            let nav = super::emoji::codes::keys(ui, conv_input);
            let suggest_nav = if nav.is_none() { suggest::keys(ui, conv_input) } else { None };
            let mut output = input::show(ui, Some(&mut *emoji), conv_input, &mut draft, field_w, INPUT_H, |edit| {
                edit.id(conv_input).hint_text(hint)
            });
            if recalled {
                output.response.mark_changed();
            }
            super::emoji::codes::after(ui, p, Some(&mut *emoji), &mut output, &mut draft, nav);
            super::spelling::field(ui, p, &mut output, &mut draft, true);
            let (w, selected): (&World, _) = (world, st.selected);
            let people = move || selected.map_or_else(Vec::new, |id| suggest::members(w, id));
            let scope = match selected {
                None => suggest::Scope::Nearby(gestures),
                Some(id) => suggest::Scope::Session(id, &people),
            };
            suggest::after(ui, p, &mut output, &mut draft, suggest_nav, &mut st.suggest, scope);
            let has_text = !draft.trim().is_empty();
            send_clicked = send_button(ui, p, icons, has_text).clicked() && has_text;
            output
                .response
                .response
                .on_hover_text(aurora_i18n::tr!("ui-chat-compose-shortcuts"))
        })
        .inner;
    // the strip between the lines and the field: egui reaches
    // both from it (interact_radius), it keeps the normal cursor
    // unless a drag goes through it
    if let Some(pos) = ui.input(|i| i.pointer.hover_pos())
        && pos.y > strip_top
        && pos.y < resp.rect.top()
        && ui.min_rect().x_range().contains(pos.x)
        && !ui.input(|i| i.pointer.any_down())
    {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
    }
    if st.emoji_open && popout::claim_picker(ui.ctx()) {
        let anchor = emoji_btn.unwrap_or(resp.rect).left_top();
        let mut close = false;
        // one per conversation: one in each window (`popout.rs`)
        egui::Area::new(egui::Id::unique(("emoji_picker", st.selected)))
            .fade_in(false)
            .order(egui::Order::Foreground)
            .pivot(egui::Align2::LEFT_BOTTOM)
            .fixed_pos(anchor - egui::vec2(0.0, 4.0))
            .show(ui.ctx(), |ui| {
                super::widgets::overlay(ui, p, egui::Margin::same(6), |ui| {
                    ui.set_width(300.0);
                    let picked = emoji.picker(ui, p, icons);
                    if let Some(c) = picked.chosen {
                        draft.push(c);
                        close = !ui.input(|i| i.modifiers.shift);
                    }
                    close |= picked.escape;
                });
            });
        if close {
            st.emoji_open = false;
            ui.ctx().memory_mut(|m| m.request_focus(conv_input));
        }
    }
    let anchor = emoji_btn.unwrap_or(resp.rect).left_top();
    gif_picker::popup(ui.ctx(), p, icons, st, anchor, 4.0, conv_input, ChatType::Normal, actions);
    if st.conv_focus_request {
        st.conv_focus_request = false;
        resp.request_focus();
        cursor_to_end(ui.ctx(), conv_input, &draft);
    }
    let mut sent = false;
    if send_clicked || ((resp.has_focus() || resp.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.alt)) {
        let text = st.suggest.expand(st.selected, draft.trim());
        if !text.is_empty() {
            sent = true;
            st.input_history.entry(st.selected).or_default().record(conv_input, &draft);
            match st.selected {
                None => {
                    let (channel, message) = parse_channel(&text);
                    actions.push(ConvAction::Local(OutgoingChat {
                        message,
                        channel,
                        // Ctrl+Enter shouts, Shift+Enter whispers (like the chat bar)
                        chat_type: ui.input(|i| {
                            if i.modifiers.ctrl {
                                ChatType::Shout
                            } else if i.modifiers.shift {
                                ChatType::Whisper
                            } else {
                                ChatType::Normal
                            }
                        }),
                    }));
                }
                Some(to) => actions.push(ConvAction::Im { to, text }),
            }
        }
        draft.clear();
        resp.request_focus();
    }
    st.typing_field = Some(typing::Field {
        to: st.selected.filter(|id| !world.is_group_session(id) && (resp.has_focus() || sent)),
        typed: resp.changed(),
        empty: draft.trim().is_empty(),
        sent,
    });
    if st.selected.is_none() {
        let said = typing::says_aloud(&draft, st.type_during_emote, st.colon_emote);
        st.local_field(&resp, said, sent);
    }
    if !draft.is_empty() {
        *st.draft_mut(st.selected) = draft;
    } else {
        st.drafts.remove(&st.selected);
    }
}

/// The violet outline of the Conversations floater (#293) waits for what the
/// pill of its bottom-bar button counts: IMs, group chats and conferences,
/// never the nearby chat alone (`World::conversations_badge`).
pub fn conversations_waiting(world: &World) -> bool {
    world.conversations_badge() > 0
}

/// The open Conversations floater shows the local chat: its tab is selected
/// and the floater is not minimized. Only then are the bubbles hidden, as
/// Firestorm's nearby chat toasts are while the nearby chat panel is visible
/// (LLFloaterIMNearbyChatHandler::processChat, nearby_chat->getVisible());
/// on the Contacts or an IM tab the bubbles come back.
pub fn local_chat_shown(ctx: &egui::Context, st: &ChatUi) -> bool {
    let minimized = ctx
        .data_mut(|d| d.get_persisted::<bool>(egui::Id::unique(("conversations", "minimized"))))
        .unwrap_or(false);
    (!st.contacts && st.selected.is_none() && !minimized) || popout::in_sight(ctx, &st.popped).any(|c| c.is_none())
}

/// A demo's window of its own for `conversation`, at `pos` (whatever the
/// cascade or a previous run would give it).
pub fn demo_place_window(ctx: &egui::Context, conversation: Option<uuid::Uuid>, pos: egui::Pos2, size: egui::Vec2) {
    super::widgets::place_floater(ctx, &popout::floater_id(conversation), pos, size);
}

/// The conversations in sight (None: the nearby chat): the one the
/// Conversations window shows while `main_open` and not minimized, and
/// those of the windows of their own not minimized (FSFloaterIM::getVisible:
/// a torn-off floater is visible on its own).
pub fn conversations_in_sight(ctx: &egui::Context, st: &ChatUi, main_open: bool) -> Vec<Option<uuid::Uuid>> {
    let minimized = super::widgets::floater_minimized(ctx, "conversations");
    let main = (main_open && !minimized).then_some(st.selected);
    main.into_iter().chain(popout::in_sight(ctx, &st.popped)).collect()
}

/// The sender of a bubble line: the avatar's profile picture (initials while
/// it loads), or the tinted cube of an object, like the conversation window.
#[allow(clippy::too_many_arguments)]
fn bubble_sender(
    painter: &egui::Painter,
    p: &Palette,
    pics: &HashMap<uuid::Uuid, egui::TextureHandle>,
    wanted_pics: &mut HashSet<uuid::Uuid>,
    r: egui::Rect,
    who: &Sender,
    line: &crate::world::ChatLine,
    alpha: f32,
) {
    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    match who {
        Sender::Object => {
            let (glyph, tint) = object_row::object_glyph(line, p, super::colors::sender_color(line));
            if let Some(t) = super::icons::global(glyph) {
                painter.image(t.id(), r.shrink(1.0), uv, tint.gamma_multiply(alpha));
            }
        }
        Sender::Avatar(id) => {
            wanted_pics.insert(*id);
            match pics.get(id) {
                Some(t) => {
                    painter.image(t.id(), r, uv, super::widgets::PICTURE_TINT.gamma_multiply(alpha));
                }
                None => {
                    let radius = painter.ctx().global_style().visuals.window_corner_radius;
                    painter.rect_filled(r, radius, p.raised.gamma_multiply(alpha));
                    painter.text(
                        r.center(),
                        egui::Align2::CENTER_CENTER,
                        initials(&line.from),
                        egui::FontId::proportional(r.height() * 0.45),
                        p.violet_pale.gamma_multiply(alpha),
                    );
                }
            }
        }
    }
}

/// A resident's line, ours included: with the bubbles above the heads it
/// may leave the lines above the chat bar, never the nearby chat
/// (CHAT_SOURCE_AGENT in LLFloaterIMNearbyChatHandler::processChat).
fn said_by_resident(line: &crate::world::ChatLine) -> bool {
    matches!(line.kind, ChatKind::Local(_) | ChatKind::Own(_))
}

/// Recent chat lines floating above the chat bar (fade out after 20 s);
/// names of linked avatars and pictures of the senders are asked through
/// `want_names` / `wanted_pics`. `cards`: Aurora's cards grouped by sender
/// (`console.rs`), else the Legacy Viewer's one-line look (pills over the
/// islands when `pills`). Without `residents`, only the other lines.
#[allow(clippy::too_many_arguments)]
pub fn toasts(
    ctx: &egui::Context,
    p: &Palette,
    world: &World,
    pics: &HashMap<uuid::Uuid, egui::TextureHandle>,
    want_names: &mut HashSet<uuid::Uuid>,
    wanted_pics: &mut HashSet<uuid::Uuid>,
    bottom: f32,
    seconds: f32,
    times: bool,
    pills: bool,
    cards: bool,
    compact_automated: bool,
    residents: bool,
) {
    if cards {
        console::cards(
            ctx,
            p,
            world,
            pics,
            want_names,
            wanted_pics,
            bottom,
            seconds,
            times,
            compact_automated,
            residents,
        );
        return;
    }
    let now = std::time::SystemTime::now();
    let recent: Vec<_> = world
        .chat
        .iter()
        .rev()
        .filter(|l| residents || !said_by_resident(l))
        .take(6)
        .filter(|l| now.duration_since(l.time).map(|d| d.as_secs_f32() < seconds).unwrap_or(false))
        .collect();
    if recent.is_empty() {
        return;
    }
    let mut links: Vec<(egui::Rect, Link, Color32)> = Vec::new();
    let mut names: Vec<(egui::Rect, &crate::world::ChatLine)> = Vec::new();
    egui::Area::new(egui::Id::unique("chat_toasts"))
        .anchor(
            egui::Align2::LEFT_BOTTOM,
            egui::vec2(8.0, -(ctx.content_rect().bottom() - bottom) - 6.0),
        )
        .interactable(false)
        .order(egui::Order::Background)
        .show(ctx, |ui| {
            ui.set_max_width(560.0);
            // the islands' pills: the mockup's 12 px text, 9 × 4 margins and
            // 4 px between two lines
            let (size, margin, gap) = if pills {
                (12.0, egui::Margin::symmetric(9, 4), 4.0)
            } else {
                (13.0, egui::Margin::symmetric(6, 2), 2.0)
            };
            let family = pills.then(|| super::fonts::semibold(ui.ctx()));
            if pills {
                ui.spacing_mut().item_spacing.y = 0.0;
            }
            for line in recent.into_iter().rev() {
                let age = now.duration_since(line.time).map(|d| d.as_secs_f32()).unwrap_or(0.0);
                let alpha = ((seconds - age) / 3.0).clamp(0.0, 1.0);
                let tint = Color32::from_rgba_unmultiplied(p.bar.r(), p.bar.g(), p.bar.b(), (190.0 * alpha) as u8);
                let radius = ui.visuals().window_corner_radius;
                let glass = p.panel_alpha < 255;
                let mut slot = None;
                let bubble = egui::Frame::new()
                    .fill(if glass { Color32::TRANSPARENT } else { tint })
                    .corner_radius(radius)
                    .inner_margin(margin)
                    .show(ui, |ui| {
                        slot = Some(ui.painter().add(egui::Shape::Noop));
                        let BubbleJob {
                            mut job,
                            links: line_links,
                            badges,
                            sender,
                            name,
                        } = line_job(
                            line,
                            p,
                            world,
                            want_names,
                            540.0,
                            size,
                            times,
                            family.as_ref().map_or(JobLook::Line, JobLook::Pill),
                        );
                        for s in job.sections.iter_mut() {
                            s.format.color = s.format.color.gamma_multiply(alpha);
                        }
                        let base = bidi_base(&name, &line.from);
                        let visual = ui.ctx().fonts_mut(|f| super::bidi::layout(f, job, base));
                        let galley = visual.galley.clone();
                        let (rect, _) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
                        let painter = ui.painter();
                        painter.galley(rect.min, galley.clone(), p.ink);
                        // the sender's picture (or the object cube) in the room
                        // left before its name
                        if let Some((at, who)) = sender {
                            let side = pic_side(size);
                            // at the left of the name, whatever its direction
                            let c = visual.left_edge(name.clone().unwrap_or(at..at)).translate(rect.min.to_vec2());
                            let r =
                                egui::Rect::from_center_size(egui::pos2(c.min.x - 2.0 - side * 0.5, c.center().y), egui::vec2(side, side));
                            bubble_sender(painter, p, pics, wanted_pics, r, &who, line, alpha);
                        }
                        // the islands' pills: the sender's name acts like the
                        // conversations' (Firestorm's toast links it)
                        if pills
                            && let Some(range) = name
                            && let Some(r) = visual.range_rects(rect.min, range).first().copied()
                        {
                            names.push((r, line));
                        }
                        let col = color_for(line.kind, p).gamma_multiply(alpha);
                        line_extras(ui, p, world, &visual, rect.min, size, alpha, badges, line_links, col, &mut links);
                    });
                if glass && let Some(slot) = slot {
                    let rect = bubble.response.rect;
                    ui.painter()
                        .set(slot, aurora_render::glass_panel_faded(rect, f32::from(radius.nw), tint, alpha));
                }
                ui.add_space(gap);
            }
        });
    for (k, (rect, link, col)) in links.iter().enumerate() {
        bubble_link(ctx, p, world, k, *rect, link, *col);
    }
    for (k, (rect, line)) in names.into_iter().enumerate() {
        bubble_sender_link(ctx, p, world, k, rect, line);
    }
}

/// The direction of a chat line: from the left when it starts with its
/// sender's name `from` (the picture before it), the display name isolated
/// so that an Arabic message does not carry it to the right end, and the
/// username of a complete name ("ذئب بنفسجي (glyph1)") left after it; else
/// that of its first letter.
fn bidi_base(name: &Option<std::ops::Range<usize>>, from: &str) -> super::bidi::Base {
    let Some(range) = name else {
        return super::bidi::Base::Auto;
    };
    // a username is Latin: " (glyph1)" at the end is not the display name's
    let display = match from.strip_suffix(')').and_then(|f| f.rfind(" (").map(|i| (i, &f[i + 2..]))) {
        Some((i, username)) if username.is_ascii() => from[..i].chars().count(),
        _ => from.chars().count(),
    };
    super::bidi::Base::Sender(range.start..range.start + display.min(range.len()))
}

/// What a laid-out chat line carries besides its text: the badge of each
/// link painted over its invisible glyph, the regions of the places shown
/// asked when unknown, and each link's rects (`links`), for their click
/// areas (`bubble_link`). Shared by the pills and the cards (`console.rs`).
#[allow(clippy::too_many_arguments)]
fn line_extras(
    ui: &egui::Ui,
    p: &Palette,
    world: &World,
    visual: &super::bidi::Visual,
    origin: egui::Pos2,
    size: f32,
    alpha: f32,
    badges: Vec<(usize, LinkBadge)>,
    line_links: Vec<(std::ops::Range<usize>, Link)>,
    col: Color32,
    links: &mut Vec<(egui::Rect, Link, Color32)>,
) {
    let painter = ui.painter();
    for (at, badge) in badges {
        let Some(g) = visual.range_rects(origin, at..at + 1).first().copied() else {
            continue;
        };
        let b = egui::Rect::from_center_size(g.center(), egui::vec2(size, size));
        match badge {
            LinkBadge::Web(url) => paint_link_badge(painter, p, b, &url, alpha),
            LinkBadge::Rating(access) => {
                let mut faded = painter.clone();
                faded.multiply_opacity(alpha);
                let name = super::bars::maturity_name(access);
                let radius = super::widgets::inner_radius(ui.visuals());
                super::widgets::paint_maturity_badge(&faded, p, name, b.shrink(1.0), radius);
            }
            LinkBadge::Action(action) => {
                let k = super::colors::get();
                action.paint(painter, b, super::colors::text(k.chat_slurl).gamma_multiply(alpha));
            }
        }
    }
    for (_, link) in &line_links {
        if let Link::Place(_, place) = link
            && place.location.grid.is_none()
        {
            super::widgets::region_rating(ui.ctx(), &world.map.ratings, &place.location.region);
        }
    }
    for (range, link) in line_links {
        for r in visual.range_rects(origin, range) {
            links.push((r, link.clone(), col));
        }
    }
}

/// `AURORA_DEMO_BARS=chatname`: a click on a resident's name in the pills
/// at frame 700 (its profile opens); `AURORA_DEMO_CONV_MENU`'s right click
/// on a conversation of the list (`conv_menu`); `AURORA_DEMO_CONV_GIVE`'s
/// inventory carried to an IM (`workspace`); `AURORA_DEMO_GIFS=menu`'s
/// right click on the GIF button (`gif_picker`).
pub fn demo_events(ctx: &egui::Context, frame: u64) -> Vec<egui::Event> {
    let docking = docking::demo_events(ctx, frame);
    if !docking.is_empty() {
        return docking;
    }
    let line_menu = line_menu::demo_events(ctx, frame);
    if !line_menu.is_empty() {
        return line_menu;
    }
    let preview = previews::demo_events(ctx, frame);
    if !preview.is_empty() {
        return preview;
    }
    let reorder = reorder::demo_events(ctx, frame);
    if !reorder.is_empty() {
        return reorder;
    }
    let give = workspace::give_demo_events(ctx, frame);
    if !give.is_empty() {
        return give;
    }
    let menu = conv_menu::demo_events(ctx, frame);
    if !menu.is_empty() {
        return menu;
    }
    let gif_menu = gif_picker::demo_events(ctx, frame);
    if !gif_menu.is_empty() {
        return gif_menu;
    }
    if !super::bars::demo::get().click_name || !(700..=701).contains(&frame) {
        return Vec::new();
    }
    let Some(r) = ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::unique("chat_toast_demo_name"))) else {
        return Vec::new();
    };
    let pos = r.center();
    let mut events = vec![egui::Event::PointerMoved(pos)];
    events.push(egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: frame == 700,
        modifiers: Default::default(),
    });
    events
}

/// Who a chat line's sender name leads to: a resident, an object, or no
/// one (a notice, a line without its source).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SenderLink {
    Resident(uuid::Uuid),
    Object(uuid::Uuid),
}

fn sender_link(world: &World, line: &crate::world::ChatLine) -> Option<SenderLink> {
    let id = if line.kind.is_own() { world.agent_id } else { line.source };
    match line.kind {
        _ if id.is_nil() || line.kind.is_notice() => None,
        ChatKind::Object(_) | ChatKind::ObjectIm => Some(SenderLink::Object(id)),
        _ => Some(SenderLink::Resident(id)),
    }
}

/// The sender's name of a pill: a click shows the resident's profile (the
/// one chosen in the preferences) or inspects the object, a right click
/// offers the conversations' sender menu, or the object's. Firestorm's
/// toast links the name the same way (LLViewerChat::getSenderSLURL: the
/// agent's `about`, an object's `objectim`). The hand over it says so,
/// nothing else changes (the human's choice).
fn bubble_sender_link(ctx: &egui::Context, p: &Palette, world: &World, k: usize, rect: egui::Rect, line: &crate::world::ChatLine) {
    let Some(target) = sender_link(world, line) else { return };
    // AURORA_DEMO_BARS=chatname: the newest resident's name is clicked
    if matches!(target, SenderLink::Resident(_)) && super::bars::demo::get().click_name {
        ctx.data_mut(|d| d.insert_temp(egui::Id::unique("chat_toast_demo_name"), rect));
    }
    egui::Area::new(egui::Id::unique(("chat_toast_sender", k)))
        .fixed_pos(rect.min)
        .order(egui::Order::Background)
        .show(ctx, |ui| {
            let (_, resp) = ui.allocate_exact_size(rect.size(), egui::Sense::click());
            let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
            match target {
                SenderLink::Resident(id) => {
                    line_name_menu(&resp, p, world, id, line, None);
                    if resp.clicked() {
                        super::profile::request_open(ui.ctx(), id);
                    }
                }
                SenderLink::Object(id) => object_sender(&resp, p, world, id, &line.from),
            }
        });
}

/// An object's name in a line: a click inspects it, a right click offers
/// to inspect or to ignore it (Firestorm's objectim link of the name,
/// LLViewerChat::getSenderSLURL).
fn object_sender(resp: &egui::Response, p: &Palette, world: &World, id: uuid::Uuid, name: &str) {
    super::menu::context_menu(resp, p, |ui| {
        if super::menu::item(ui, p, "zoom-scan-filled", aurora_i18n::tr!("ui-context-inspect-2")) {
            super::context::request(ui.ctx(), CtxAction::Inspect(id));
        }
        let (icon, label) = if world.mutes.is_muted_id(&id) {
            ("circle-check-filled", aurora_i18n::tr!("common-stop-ignoring"))
        } else {
            ("ban", aurora_i18n::tr!("common-ignore"))
        };
        if super::menu::item(ui, p, icon, label) {
            super::context::request(ui.ctx(), CtxAction::ToggleBlockObject { id, name: name.to_owned() });
        }
    });
    if resp.clicked() {
        super::context::request(&resp.ctx, CtxAction::Inspect(id));
    }
}

/// Screen rects of a char range of a laid-out text, one per row.
pub(crate) fn range_rects(galley: &egui::Galley, origin: egui::Pos2, range: std::ops::Range<usize>) -> Vec<egui::Rect> {
    use egui::text::CCursor;
    let mut out: Vec<egui::Rect> = Vec::new();
    for i in range {
        let mut a = galley.pos_from_cursor(CCursor::new(i));
        let b = galley.pos_from_cursor(CCursor::new(i + 1));
        // the first char of a wrapped row: a cursor at a row break reads as
        // the end of the previous row, the char starts its own row
        if (a.min.y - b.min.y).abs() > 0.5 {
            let Some(row) = galley.rows.iter().find(|r| r.rect().y_range().contains(b.center().y)) else {
                continue;
            };
            a = egui::Rect::from_min_max(egui::pos2(row.pos.x, b.min.y), b.max);
        }
        let r = egui::Rect::from_min_max(a.min, egui::pos2(b.min.x, a.max.y)).translate(origin.to_vec2());
        match out.last_mut() {
            Some(last) if (last.min.y - r.min.y).abs() < 0.5 => *last = last.union(r),
            _ => out.push(r),
        }
    }
    out
}

/// A link of a chat bubble. The bubbles let clicks through to the world
/// (Firestorm's console is read-only), so each link gets its own small
/// clickable area, as the links of Firestorm's chat toasts are
/// (LLFloaterIMNearbyChatToastPanel::handleMouseUp): same actions and
/// right-click menus as in the conversation window.
fn bubble_link(ctx: &egui::Context, p: &Palette, world: &World, k: usize, rect: egui::Rect, link: &Link, col: Color32) {
    egui::Area::new(egui::Id::unique(("chat_toast_link", k)))
        .fixed_pos(rect.min)
        .order(egui::Order::Background)
        .show(ctx, |ui| {
            let (r, resp) = ui.allocate_exact_size(rect.size(), egui::Sense::click());
            if resp.hovered() {
                ui.painter().hline(r.x_range(), r.bottom() - 1.0, egui::Stroke::new(1.0, col));
            }
            let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
            match link {
                Link::Url(u) => {
                    if resp.on_hover_text(format!("{}\n{u}", link_badge_look(p, u).2)).clicked() {
                        open_web_link(ui.ctx(), u);
                    }
                }
                Link::Place(u, place) => {
                    place_menu(&resp, p, u, place);
                    let tip = if place.teleport {
                        aurora_i18n::tr!("ui-chat-click-to-teleport-to-this-place")
                    } else {
                        aurora_i18n::tr!("ui-chat-click-to-learn-more-about-this-place")
                    };
                    if resp.on_hover_text(format!("{tip}\n{u}")).clicked() {
                        let l = &place.location;
                        super::context::request(
                            ui.ctx(),
                            if place.teleport {
                                CtxAction::TeleportToPlace(l.target(), l.pos)
                            } else {
                                CtxAction::ShowPlaceInfo(l.target(), l.pos)
                            },
                        );
                    }
                }
                Link::Agent(id) => {
                    name_menu(&resp, p, world, *id);
                    if resp.on_hover_text(aurora_i18n::tr!("common-view-profile")).clicked() {
                        super::profile::request_open(ui.ctx(), *id);
                    }
                }
                Link::App(u, app) => {
                    // the bubble's label is the menu's "copy name"
                    let label = app_links::label(world, &mut HashSet::new(), u, app);
                    app_links::menu(&resp, p, world, u, app, &label);
                    if resp.on_hover_text(app_links::tooltip(app)).clicked() {
                        super::context::request(ui.ctx(), CtxAction::AppLink(app.clone()));
                    }
                }
            }
        });
}

/// The "Nearby chat" input bar; returns a message to send.
pub fn bar(ui: &mut egui::Ui, p: &Palette, st: &mut ChatUi, gestures: &crate::gestures::Manager, width: f32) -> Option<OutgoingChat> {
    ui.label(RichText::new(aurora_i18n::tr!("common-nearby-chat")).size(12.0).color(p.muted));
    bar_input(ui, p, st, gestures, None, width, |edit| edit.desired_width(width))
}

/// `build`, then the chat field's id (the app gives it the keyboard).
fn identified(build: impl FnOnce(egui::TextEdit<'_>) -> egui::TextEdit<'_>) -> impl FnOnce(egui::TextEdit<'_>) -> egui::TextEdit<'_> {
    move |edit| build(edit.id(bar_input_id()))
}

/// The id of the local chat's field (`bar_input`).
pub(crate) fn bar_input_id() -> egui::Id {
    egui::Id::unique("nearby_chat_input")
}

/// The text cursor of a field after the last character: the keyboard given
/// to a field that holds a draft, or the letter that started the chat.
pub(crate) fn cursor_to_end(ctx: &egui::Context, id: egui::Id, text: &str) {
    let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    let end = egui::text::CCursor::new(text.chars().count());
    state.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
    egui::TextEdit::store_state(ctx, id, state);
}

/// The local chat's field, Firestorm's chat bar (LLFloaterIMNearbyChat):
/// Enter says, Ctrl+Enter shouts, Shift+Enter whispers, `/channel` and the
/// gestures' completion; Enter on an empty field returns to movement. Shared
/// by the full-width bar and the chat island (`bars/dock.rs`), which draws
/// its own frame (`build` dresses the field) and shows emoji pictures.
pub(crate) fn bar_input(
    ui: &mut egui::Ui,
    p: &Palette,
    st: &mut ChatUi,
    gestures: &crate::gestures::Manager,
    emoji: Option<&mut super::emoji::Emoji>,
    width: f32,
    build: impl FnOnce(egui::TextEdit<'_>) -> egui::TextEdit<'_>,
) -> Option<OutgoingChat> {
    let mut out = None;
    // the shortcuts (Enter, Ctrl / Shift + Enter, /channel) are a tooltip:
    // written in the empty field they crowded the bar (the human's request)
    let build = identified(build);
    input::prepare(ui, bar_input_id(), true);
    let recalled = st.input_history.entry(None).or_default().recall(ui, bar_input_id(), &mut st.input);
    let nav = super::emoji::codes::keys(ui, bar_input_id());
    let suggest_nav = if nav.is_none() { suggest::keys(ui, bar_input_id()) } else { None };
    let mut emoji = emoji;
    let minimum = ui.spacing().interact_size.y;
    let mut output = input::show(ui, emoji.as_deref_mut(), bar_input_id(), &mut st.input, width, minimum, build);
    if recalled {
        output.response.mark_changed();
    }
    super::emoji::codes::after(ui, p, emoji, &mut output, &mut st.input, nav);
    super::spelling::field(ui, p, &mut output, &mut st.input, true);
    suggest::after(
        ui,
        p,
        &mut output,
        &mut st.input,
        suggest_nav,
        &mut st.suggest,
        suggest::Scope::Nearby(gestures),
    );
    let resp = output
        .response
        .response
        .on_hover_text(aurora_i18n::tr!("ui-chat-enter-to-talk-ctrl-enter-to-shout-shift-enter"));
    // the Enter that asked for the keyboard (start_chat) is not one typed
    // in the field: taken as Enter on an empty bar, it handed the keyboard
    // straight back to the world
    let focusing = std::mem::take(&mut st.focus_request);
    if focusing {
        resp.request_focus();
        // FSNearbyChat::showDefaultChatBar only moves the caret after
        // inserting `/` into an empty field; a draft keeps its position.
        if !std::mem::take(&mut st.gesture_keep_cursor) {
            cursor_to_end(ui.ctx(), resp.id, &st.input);
        }
    }
    st.has_focus = resp.has_focus();
    // The editor leaves ordinary Enter to this send path.
    let enter = !focusing && (resp.has_focus() || resp.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.alt);
    // Enter, even on an empty bar, ends the typing (sendChat → stopTyping)
    let said = typing::says_aloud(&st.input, st.type_during_emote, st.colon_emote);
    st.local_field(&resp, said, enter);
    if enter {
        resp.surrender_focus();
    }
    if enter && !st.input.trim().is_empty() {
        st.input_history.entry(None).or_default().record(bar_input_id(), &st.input);
        let (shout, whisper) = ui.input(|i| (i.modifiers.ctrl, i.modifiers.shift));
        let (channel, message) = parse_channel(&st.suggest.expand(None, st.input.trim()));
        out = Some(OutgoingChat {
            message,
            channel,
            chat_type: if shout {
                ChatType::Shout
            } else if whisper {
                ChatType::Whisper
            } else {
                ChatType::Normal
            },
        });
        st.input.clear();
        if st.keep_focus {
            // ready for the next message; only Enter on the empty bar or a
            // click elsewhere gives the keyboard back to the world
            st.focus_request = true;
        }
    }
    // Enter on an empty bar returns to movement (like Firestorm).
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chat_line_isolates_its_sender_s_display_name() {
        use crate::ui::bidi::Base;
        // the username of a complete name stays after the display name
        assert_eq!(bidi_base(&Some(4..23), "ذئب بنفسجي (glyph1)"), Base::Sender(4..14));
        assert_eq!(bidi_base(&Some(0..10), "ذئب بنفسجي"), Base::Sender(0..10));
        // brackets of the display name itself are part of it
        assert_eq!(bidi_base(&Some(0..8), "ليلى (٢)"), Base::Sender(0..8));
        assert_eq!(bidi_base(&None, "ذئب"), Base::Auto);
    }
    #[test]
    fn gesture_chat_remembers_a_shifted_slash_until_its_own_release() {
        let ctx = egui::Context::default();
        let mut st = ChatUi::default();
        let slash = winit::keyboard::Key::Character("/".into());
        let period = winit::keyboard::Key::Character(".".into());
        let code = winit::keyboard::KeyCode::Period;
        let plain = crate::keybinds::Mods::default();
        let shifted = crate::keybinds::Mods { shift: true, ..plain };
        for _ in 0..3 {
            assert!(st.gesture_key(&ctx, &slash, code, true, shifted));
            assert!(st.input.is_empty());
            assert!(!st.focus_request);
        }
        assert!(!st.gesture_key(&ctx, &period, winit::keyboard::KeyCode::ShiftLeft, false, plain));
        assert!(st.gesture_key(&ctx, &period, code, false, plain));
        assert_eq!(st.input, "/");
        assert!(st.focus_request);

        st.input.clear();
        st.focus_request = false;
        assert!(st.gesture_key(&ctx, &slash, code, true, shifted));
        st.cancel_gesture_key();
        assert!(!st.gesture_key(&ctx, &period, code, false, plain));
        assert!(st.input.is_empty());
        assert!(!st.focus_request);
    }

    #[test]
    fn gesture_chat_respects_text_editors_and_popups_but_accepts_button_focus() {
        let mut frames = Frames::new();
        let mut st = ChatUi::default();
        let slash = winit::keyboard::Key::Character("/".into());
        let code = winit::keyboard::KeyCode::Slash;
        let mods = crate::keybinds::Mods::default();
        let mut search = "search".to_owned();
        frames.run(vec![], |ui| {
            egui::TextEdit::singleline(&mut search)
                .id_salt("search")
                .show(ui)
                .response
                .request_focus();
        });
        assert!(frames.ctx.text_edit_focused());
        for pressed in [true, false] {
            assert!(!st.gesture_key(&frames.ctx, &slash, code, pressed, mods));
        }
        frames.run(vec![egui::Event::Text("/".into())], |ui| {
            egui::TextEdit::singleline(&mut search).id_salt("search").show(ui);
        });
        assert_eq!(search, "search/");
        assert!(st.input.is_empty());
        assert!(!st.focus_request);

        frames.run(vec![], |ui| {
            ui.button("A button").request_focus();
        });
        assert!(frames.ctx.egui_wants_keyboard_input());
        assert!(!frames.ctx.text_edit_focused());
        assert!(st.gesture_key(&frames.ctx, &slash, code, true, mods));
        let popup = egui::Id::unique("gesture_popup");
        egui::Popup::open_id(&frames.ctx, popup);
        assert!(!st.gesture_key(&frames.ctx, &slash, code, false, mods));
        assert!(!st.focus_request);
        egui::Popup::close_all(&frames.ctx);
        assert!(!st.gesture_key(&frames.ctx, &slash, code, false, mods));
        assert!(st.gesture_key(&frames.ctx, &slash, code, true, mods));
        assert!(st.gesture_key(&frames.ctx, &slash, code, false, mods));
        assert_eq!(st.input, "/");
        assert!(st.focus_request);
    }

    /// One frame of a notification text with place links: the regions
    /// whose rating is not known are the ones asked, each once.
    #[test]
    fn place_links_ask_only_unknown_regions() {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let p = crate::theme::Theme::default().palette();
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.map.ratings.learn("Aurora Démo", aurora_net::search::SIM_ACCESS_PG);
        let text = "Ici http://maps.secondlife.com/secondlife/Aurora%20D%C3%A9mo/1/2/3, là secondlife:///app/region/Pin%C3%A8de/4/5/6 \
                    et secondlife://Pinède/7/8/9";
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0))),
                ..Default::default()
            },
            |ui| text_with_links(ui, &p, &world, text, &[], 12.0, p.ink),
        );
        output.textures_delta.clear();
        assert_eq!(super::super::widgets::take_wanted_regions(&ctx), vec!["Pinède".to_owned()]);
        // taken: nothing left until the next frame draws them again
        assert!(super::super::widgets::take_wanted_regions(&ctx).is_empty());
    }

    /// #898 follow-up: a place link's maturity square stands on the
    /// capitals of the text beside it (and its letter in its middle), within
    /// half a pixel, for every rating, built-in font and scale, in a dialog's
    /// text and in a chat line.
    #[test]
    fn place_link_badge_is_centred_on_the_text_beside_it() {
        use egui::emath::GuiRounding as _;
        fn texts(shape: &egui::Shape, out: &mut Vec<(egui::Pos2, std::sync::Arc<egui::Galley>)>) {
            match shape {
                egui::Shape::Text(t) => out.push((t.pos, t.galley.clone())),
                egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
                _ => {}
            }
        }
        fn rects(shape: &egui::Shape, size: f32, out: &mut Vec<egui::Rect>) {
            match shape {
                egui::Shape::Rect(r) if (r.rect.width() - size).abs() < 1.01 && (r.rect.height() - size).abs() < 1.01 => out.push(r.rect),
                egui::Shape::Vec(v) => v.iter().for_each(|s| rects(s, size, out)),
                _ => {}
            }
        }
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.map.ratings.learn("Pinède", aurora_net::search::SIM_ACCESS_PG);
        world.map.ratings.learn("Lagune", aurora_net::search::SIM_ACCESS_MATURE);
        world.map.ratings.learn("Falaise", aurora_net::search::SIM_ACCESS_ADULT);
        for font in [
            crate::ui::fonts::INTER,
            crate::ui::fonts::NOTO,
            crate::ui::fonts::ROBOTO,
            crate::ui::fonts::OPEN_DYSLEXIC,
        ] {
            for pixels in [1.0, 1.25, 1.5] {
                let ctx = egui::Context::default();
                let theme = crate::theme::Theme::default();
                theme.apply(&ctx, 1.0);
                crate::ui::fonts::apply(&ctx, font);
                ctx.set_pixels_per_point(pixels);
                let p = theme.palette();
                let mut emoji = crate::ui::emoji::Emoji::load();
                for (region, letter) in [("Pin%C3%A8de", "G"), ("Lagune", "M"), ("Falaise", "A")] {
                    for chat in [false, true] {
                        let text = format!("AB secondlife:///app/region/{region}/1/2/3");
                        let size = 12.0;
                        let mut out = ctx.run_ui(
                            egui::RawInput {
                                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 200.0))),
                                ..Default::default()
                            },
                            |ui| {
                                if chat {
                                    chat_text(ui, &p, &mut emoji, &world, &mut HashSet::new(), &text, size, p.ink, false);
                                } else {
                                    text_with_links(ui, &p, &world, &text, &[], size, p.ink);
                                }
                            },
                        );
                        out.textures_delta.clear();
                        let mut shown = Vec::new();
                        let mut squares = Vec::new();
                        for s in &out.shapes {
                            texts(&s.shape, &mut shown);
                            rects(&s.shape, size, &mut squares);
                        }
                        let case = format!("font={font}, pixels={pixels}, letter={letter}, chat={chat}");
                        assert_eq!(squares.len(), 1, "{case}: one badge square, {squares:?}");
                        let square = squares[0];
                        // the neighbour's capitals and the badge's letter, as drawn
                        // (the link's label starts with its icon's slot, an
                        // "M": the letter is the text of its own alone)
                        let ink = |text: &str, whole: bool| {
                            let (pos, galley) = shown
                                .iter()
                                .find(|(_, g)| if whole { g.text() == text } else { g.text().starts_with(text) })
                                .unwrap_or_else(|| panic!("{case}: no {text}"));
                            let ink = crate::ui::widgets::ink_rect(galley);
                            ink.translate(pos.round_to_pixels(pixels).to_vec2())
                        };
                        let caps = ink("AB", false);
                        assert!(
                            (square.center().y - caps.center().y).abs() <= 0.5 + 1e-3,
                            "{case}: square {square:?} against the capitals {caps:?}"
                        );
                        let mark = ink(letter, true);
                        assert!(
                            (mark.center() - square.center()).abs().max_elem() <= 0.5 + 1e-3,
                            "{case}: letter {mark:?} in its square {square:?}"
                        );
                    }
                }
            }
        }
    }

    /// #898: a script dialog's agent and group links show the names, not
    /// the addresses; an unknown avatar shows the placeholder until its
    /// name comes.
    #[test]
    fn dialog_text_shows_agent_and_group_names() {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let p = crate::theme::Theme::default().palette();
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let (known, unknown, group) = (
            uuid::Uuid::from_u128(0x8981),
            uuid::Uuid::from_u128(0x8982),
            uuid::Uuid::from_u128(0x8983),
        );
        world.social.names.insert(known, "Demo Resident".into());
        world.land.group_names.insert(group, "Demo Builders".into());
        let text = format!(
            "#1 secondlife:///app/agent/{known}/about\n#2 secondlife:///app/agent/{unknown}/about\n#3 secondlife:///app/group/{group}/about"
        );
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0))),
                ..Default::default()
            },
            |ui| text_with_links(ui, &p, &world, &text, &[], 12.0, p.ink),
        );
        output.textures_delta.clear();
        let mut shown = String::new();
        for clipped in &output.shapes {
            if let egui::Shape::Text(t) = &clipped.shape {
                shown.push_str(t.galley.text());
                shown.push('|');
            }
        }
        assert!(shown.contains("Demo Resident"), "{shown}");
        assert!(shown.contains("Demo Builders"), "{shown}");
        assert!(shown.contains('…'), "{shown}");
        assert!(!shown.contains("secondlife:///app"), "{shown}");
    }

    #[test]
    fn hypergrid_links_are_native_places_in_chat_and_labelled_text() {
        for text in [
            "See hop://grid.example:8002/Region/12/34/56 here",
            "See [x-grid-location-info://grid.example:8002/region/Region/12/34/56 My place] here",
        ] {
            let parts = segments(text);
            let place = parts
                .iter()
                .find_map(|part| match part {
                    Seg::Place(_, place) => Some(place),
                    _ => None,
                })
                .expect("native place");
            assert_eq!(place.location.grid.as_deref(), Some("grid.example:8002"));
            assert_eq!(place.location.region, "Region");
            assert_eq!(place.location.pos, glam::Vec3::new(12.0, 34.0, 56.0));
            assert_eq!(place.location.target(), "hop://grid.example:8002/Region");
            assert!(
                !parts.iter().any(|part| matches!(part, Seg::Url(_))),
                "no external browser dispatch"
            );
        }
    }

    #[test]
    fn agent_links() {
        let id = uuid::Uuid::from_u128(5);
        let t = format!("voir secondlife:///app/agent/{id}/about, @ secondlife:///app/agent/{id}/mention");
        let segs = segments(&t);
        assert_eq!(segs[1], Seg::Agent(id));
        assert_eq!(segs[3], Seg::Mention(id));
        // not an agent link: the generic SLapp (LLUrlEntrySL), whose click
        // says the command is not supported
        let bad = segments("secondlife:///app/agent/nope/about");
        assert!(matches!(bad[0], Seg::App(_, crate::slapp::AppLink::Unsupported)));
        // another verb acts (LLAgentHandler) and shows the name
        let im = format!("secondlife:///app/agent/{id}/im");
        assert!(matches!(segments(&im)[0], Seg::App(_, crate::slapp::AppLink::Agent { .. })));
    }

    #[test]
    fn a_link_verb_ends_at_its_word() {
        // an object's "secondlife:///app/agent/<id>/about's follower" (LLUrlEntryAgent)
        let id = uuid::Uuid::from_u128(5);
        let t = format!("secondlife:///app/agent/{id}/about's follower");
        assert_eq!(segments(&t), vec![Seg::Agent(id), Seg::Text("'s follower")]);
        let t = format!("secondlife:///app/group/{id}/about’s notice");
        assert!(matches!(&segments(&t)[..], [Seg::App(_, crate::slapp::AppLink::Group(g)), Seg::Text("’s notice")] if *g == id));
        // arguments and queries stay in the link
        let t = format!("secondlife:///app/agent/{id}/block/Old%20Sign!");
        assert!(matches!(&segments(&t)[..], [Seg::App(l, _), Seg::Text("!")] if l.ends_with("Sign")));
        // other commands keep the whole address (LLUrlEntryChat: `\d+/\S+`)
        let t = "secondlife:///app/chat/42/it's";
        assert!(matches!(&segments(t)[..], [Seg::App(l, _)] if *l == t));
    }

    #[test]
    fn a_mention_of_the_agent_is_found_and_written_as_a_name() {
        let (me, jane) = (uuid::Uuid::from_u128(5), uuid::Uuid::from_u128(6));
        let t = format!("secondlife:///app/agent/{me}/mention see secondlife:///app/agent/{jane}/about at https://example.org");
        assert!(mentions(&t, &me));
        assert!(!mentions(&t, &jane), "an about link is not a mention");
        assert!(!mentions(&format!("secondlife:///app/agent/{me}/about"), &me));
        let name = |id: &uuid::Uuid| (*id == me).then(|| "Aurora Demo".to_owned());
        let app = |_: &str, _: &crate::slapp::AppLink| "Demo Group".to_owned();
        assert_eq!(
            plain_text(&t, name, app),
            format!("@Aurora Demo see {jane} at https://example.org"),
            "an unknown name keeps its id, a web link its address"
        );
        assert_eq!(plain_text("[https://example.org the site] now", name, app), "the site now");
        let g = uuid::Uuid::from_u128(7);
        assert_eq!(
            plain_text(&format!("join secondlife:///app/group/{g}/about!"), name, app),
            "join Demo Group!",
            "an app command shows its label (#947)"
        );
    }

    /// The previews read a message as the conversation shows it: names,
    /// groups and places by their labels, "…" while a name is asked (#947).
    #[test]
    fn a_preview_shows_the_labels_of_the_links() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let (lune, unknown, group) = (
            uuid::Uuid::from_u128(0x61),
            uuid::Uuid::from_u128(0x62),
            uuid::Uuid::from_u128(0x63),
        );
        world.social.names.insert(lune, "Lune Étoilée".into());
        world.land.group_names.insert(group, "Cercle des Brumes".into());
        let text = format!(
            "secondlife:///app/agent/{lune}/about, secondlife:///app/agent/{lune}/mention et \
             secondlife:///app/agent/{unknown}/about à secondlife:///app/group/{group}/about \
             http://maps.secondlife.com/secondlife/Pinède/1/2/3 [https://example.org le site] https://example.org/x"
        );
        assert_eq!(
            resolved_text(&world, &text),
            "Lune Étoilée, @Lune Étoilée et … à Cercle des Brumes Pinède (1,2,3) le site https://example.org/x"
        );
        assert_eq!(resolved_text(&world, "plain words"), "plain words");
    }

    #[test]
    fn links_written_with_their_own_text_show_the_text() {
        let segs = segments("My channel: [https://www.youtube.com/@me YouTube], come!");
        assert_eq!(segs[0], Seg::Text("My channel: "));
        assert_eq!(
            segs[1],
            Seg::Labelled("YouTube".into(), Box::new(Seg::Url("https://www.youtube.com/@me")))
        );
        assert_eq!(segs[2], Seg::Text(", come!"));
        // several words, a tab, an escape (getLabelFromWikiLink)
        let segs = segments("[http://example.org\tMy%20shop  is open]");
        assert_eq!(
            segs,
            vec![Seg::Labelled("My shop  is open".into(), Box::new(Seg::Url("http://example.org")))]
        );
        let id = uuid::Uuid::from_u128(5);
        let t = format!("[secondlife:///app/agent/{id}/about my friend]");
        assert_eq!(segments(&t), vec![Seg::Labelled("my friend".into(), Box::new(Seg::Agent(id)))]);
        let place = segments("[secondlife:///app/teleport/Ahern/1/2/3 the beach]");
        assert!(matches!(&place[0], Seg::Place(_, p) if p.label == "the beach"));
    }

    #[test]
    fn a_text_that_is_a_link_shows_the_real_address() {
        // LLUrlEntryHTTPLabel::getLabel: no "[https://evil.example https://bank.example]"
        let segs = segments("[https://evil.example https://bank.example]");
        assert_eq!(segs, vec![Seg::Url("https://evil.example")]);
        let segs = segments("[https://evil.example www.bank.example]");
        assert_eq!(segs, vec![Seg::Url("https://evil.example")]);
    }

    #[test]
    fn brackets_that_are_not_a_labelled_link_stay_text() {
        for t in [
            "[https://example.org]",
            "[https://example.org ]",
            "[note] text",
            "[https://example.org\nText]",
            "[ftp.example.org x]",
        ] {
            assert!(!segments(t).iter().any(|s| matches!(s, Seg::Labelled(..))), "{t}");
        }
        // the address alone is still a link, without the bracket
        assert_eq!(segments("[https://example.org]")[1], Seg::Url("https://example.org"));
    }

    #[test]
    fn addresses_without_scheme_are_links() {
        let segs = segments("see www.youtube.com/watch, or [www.youtube.com YouTube]");
        assert_eq!(segs[1], Seg::Url("www.youtube.com/watch"));
        assert_eq!(segs[3], Seg::Url("www.youtube.com"));
        assert_eq!(web_target("www.youtube.com"), "http://www.youtube.com");
        for t in ["awww.example.com", "www.", "www.example", "www .example.com"] {
            assert!(segments(t).iter().all(|s| matches!(s, Seg::Text(_))), "{t}");
        }
    }

    #[test]
    fn toast_text_shows_link_labels() {
        let id = uuid::Uuid::from_u128(5);
        let t = format!(
            "Viens secondlife:///app/agent/{id}/about à http://maps.secondlife.com/secondlife/Ahern/1/2/3, merci secondlife:///app/agent/{id}/mention ! https://example.com/x"
        );
        let ps = pieces(&t, |_| "Loup Violet".into());
        let text: String = ps.iter().map(|(s, _)| s.as_str()).collect();
        assert_eq!(
            text,
            "Viens Loup Violet à Ahern (1,2,3), merci @Loup Violet ! https://example.com/x"
        );
        let links: Vec<_> = ps.iter().filter_map(|(s, l)| l.as_ref().map(|l| (s.as_str(), l))).collect();
        assert_eq!(links.len(), 4);
        assert_eq!(links[0], ("Loup Violet", &Link::Agent(id)));
        assert!(matches!(links[1], ("Ahern (1,2,3)", Link::Place(..))));
        assert_eq!(links[2], ("@Loup Violet", &Link::Agent(id)));
        assert_eq!(links[3].1, &Link::Url("https://example.com/x".into()));
    }

    #[test]
    fn an_emoji_does_not_make_its_line_taller() {
        let mut emoji = crate::ui::emoji::Emoji::load();
        assert!(emoji.available());
        let mut frames = Frames::new();
        let (mut plain, mut with) = (0.0, 0.0);
        let p = crate::theme::Theme::default().palette();
        frames.run(Vec::new(), |ui| {
            plain = ui
                .scope(|ui| emoji.rich_text(ui, "On my way!", 13.5, p.ink, false))
                .response
                .rect
                .height();
            with = ui
                .scope(|ui| emoji.rich_text(ui, "On my way 😄", 13.5, p.ink, false))
                .response
                .rect
                .height();
        });
        assert!(plain > 0.0);
        assert!((with - plain).abs() < 0.5, "a line of text is {plain} tall, with an emoji {with}");
    }

    #[test]
    fn the_workspace_by_default_bubbles_and_legacy_lines_on_demand() {
        use crate::settings::ChatStyle;
        assert_eq!(crate::settings::Settings::default().chat_style, ChatStyle::Aurora);
        assert!(ChatStyle::Aurora.workspace() && ChatStyle::Compact.workspace());
        assert!(!ChatStyle::Bubbles.workspace() && !ChatStyle::Legacy.workspace());
        // the nearby chat and the groups name their senders, a 1:1 does not
        assert_eq!(line_style(ChatStyle::Bubbles, None), Some(true));
        assert_eq!(line_style(ChatStyle::Bubbles, Some(true)), Some(true));
        assert_eq!(line_style(ChatStyle::Bubbles, Some(false)), Some(false));
        for group in [None, Some(true), Some(false)] {
            assert_eq!(line_style(ChatStyle::Legacy, group), None);
        }
        // kept by the settings file
        let legacy: ChatStyle = serde_json::from_str("\"Legacy\"").unwrap();
        assert_eq!(legacy, ChatStyle::Legacy);
    }

    /// The islands' chat pills (the bars mockup): a resident's name in
    /// semi-bold light violet followed by the text without a colon, an
    /// object's name in its kind's colour, a system line in italics; the
    /// full-width bar's lines keep "Name: text" and their colours.
    #[test]
    fn chat_pills_name_residents_in_violet_without_a_colon() {
        let p = crate::theme::Theme::default().palette();
        let world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let family = egui::FontFamily::Proportional;
        let line = |kind, from: &str, text: &str| crate::world::ChatLine {
            time: std::time::SystemTime::now(),
            from: from.into(),
            text: text.into(),
            kind,
            source: uuid::Uuid::from_u128(7),
            origin: Origin::Live,
        };
        let job = |l: &crate::world::ChatLine, pill: Option<&egui::FontFamily>| {
            line_job(
                l,
                &p,
                &world,
                &mut HashSet::new(),
                540.0,
                12.0,
                false,
                pill.map_or(JobLook::Line, JobLook::Pill),
            )
            .job
        };
        let said = line(ChatKind::Local(ChatType::Normal), "Orion Exemple", "Bienvenue");
        let pill = job(&said, Some(&family));
        assert_eq!(pill.text, "Orion ExempleBienvenue");
        let name = &pill.sections[0];
        assert_eq!(name.format.color, p.violet_light);
        assert_eq!(pill.sections.last().map(|s| s.leading_space), Some(4.0));
        assert_eq!(pill.sections.last().map(|s| s.format.color), Some(p.ink));
        let bar = job(&said, None);
        assert_eq!(bar.text, "Orion Exemple: Bienvenue");
        let object = line(ChatKind::Object(ChatType::Normal), "Panneau", "Bonjour");
        let pill = job(&object, Some(&family));
        assert_eq!(pill.sections[0].format.color, color_for(object.kind, &p));
        let online = line(ChatKind::System, "", "Nova est en ligne.");
        let pill = job(&online, Some(&family));
        assert!(pill.sections.iter().all(|s| s.format.italics));
        assert!(!job(&online, None).sections.iter().any(|s| s.format.italics));
    }

    /// The pills' sender names: a resident's profile (ours for our own
    /// lines), an object's inspection, nothing for a notice or a line
    /// without its source.
    #[test]
    fn a_pill_name_leads_to_its_resident_or_object() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.agent_id = uuid::Uuid::from_u128(1);
        let line = |kind, source: u128| crate::world::ChatLine {
            time: std::time::SystemTime::now(),
            from: "Someone".into(),
            text: "Hello".into(),
            kind,
            source: uuid::Uuid::from_u128(source),
            origin: Origin::Live,
        };
        let id = uuid::Uuid::from_u128;
        assert_eq!(
            sender_link(&world, &line(ChatKind::Local(ChatType::Normal), 7)),
            Some(SenderLink::Resident(id(7)))
        );
        assert_eq!(sender_link(&world, &line(ChatKind::OWN, 0)), Some(SenderLink::Resident(id(1))));
        assert_eq!(
            sender_link(&world, &line(ChatKind::Object(ChatType::Normal), 9)),
            Some(SenderLink::Object(id(9)))
        );
        assert_eq!(sender_link(&world, &line(ChatKind::ObjectIm, 9)), Some(SenderLink::Object(id(9))));
        assert_eq!(sender_link(&world, &line(ChatKind::System, 7)), None);
        assert_eq!(sender_link(&world, &line(ChatKind::Local(ChatType::Normal), 0)), None);
    }

    /// A click on a resident's name in the pills opens the profile; the
    /// rest of the pill, and the room between the pills, let the click
    /// through to the world.
    #[test]
    fn a_click_on_a_pill_name_opens_the_profile_and_the_rest_reaches_the_world() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let orion = uuid::Uuid::from_u128(7);
        for (kind, from, source, text) in [
            (ChatKind::System, "", 0, "Nova est en ligne."),
            (ChatKind::Local(ChatType::Normal), "Orion Exemple", 7, "Bienvenue dans la démo"),
        ] {
            world.chat.push_back(crate::world::ChatLine {
                time: std::time::SystemTime::now(),
                from: from.into(),
                text: text.into(),
                kind,
                source: uuid::Uuid::from_u128(source),
                origin: Origin::Live,
            });
        }
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let p = crate::theme::Theme::default().palette();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1536.0, 864.0));
        let frame = |events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    events,
                    ..Default::default()
                },
                |ui| {
                    toasts(
                        ui.ctx(),
                        &p,
                        &world,
                        &HashMap::new(),
                        &mut HashSet::new(),
                        &mut HashSet::new(),
                        800.0,
                        20.0,
                        false,
                        true,
                        false,
                        false,
                        true,
                    )
                },
            );
            out.textures_delta.clear();
        };
        frame(Vec::new());
        frame(Vec::new());
        let name = ctx
            .memory(|m| m.area_rect(egui::Id::unique(("chat_toast_sender", 0usize))))
            .expect("the resident's name is clickable");
        // one name only: the notice has none
        assert!(
            ctx.memory(|m| m.area_rect(egui::Id::unique(("chat_toast_sender", 1usize))))
                .is_none()
        );
        let click = |at: egui::Pos2, pressed: bool| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        let at = name.center();
        frame(vec![egui::Event::PointerMoved(at), click(at, true)]);
        frame(vec![click(at, false)]);
        assert_eq!(crate::ui::profile::take_open_request(&ctx), Some(orion));
        // the message after the name and the gap between pills: the world's
        let after = egui::pos2(name.right() + 40.0, name.center().y);
        frame(vec![egui::Event::PointerMoved(after)]);
        assert!(!ctx.is_pointer_over_egui(), "the message lets the pointer through");
        let between = egui::pos2(name.left() + 2.0, name.top() - 3.0);
        frame(vec![egui::Event::PointerMoved(between)]);
        assert!(!ctx.is_pointer_over_egui(), "between two pills");
    }

    #[test]
    fn channels() {
        assert_eq!(parse_channel("/5 hello"), (5, "hello".into()));
        assert_eq!(parse_channel("hello"), (0, "hello".into()));
        assert_eq!(parse_channel("/me waves"), (0, "/me waves".into()));
        assert_eq!(parse_channel("/-1 x"), (-1, "x".into()));
        // a gesture's commands to worn scripts (#952)
        assert_eq!(parse_channel("/-4242 ping"), (-4242, "ping".into()));
        assert_eq!(parse_channel("/5test"), (5, "test".into()));
        assert_eq!(parse_channel("/5-3"), (5, "-3".into()), "the sign only before the digits");
        assert_eq!(parse_channel("/-"), (0, "/-".into()));
        assert_eq!(parse_channel("/--5 x"), (0, "/--5 x".into()));
    }

    /// ":" as "/me" (FSCommon::applyMuPose): poses and possessives become
    /// emotes, smileys and short lines stay as they are.
    #[test]
    fn colon_emote_follows_the_mu_pose_rule() {
        let pose = |text: &str| {
            let mut text = text.to_owned();
            colon_emote(&mut text);
            text
        };
        assert_eq!(pose(":waves"), "/me waves");
        assert_eq!(pose(":'s here"), "/me's here");
        assert_eq!(pose(":été là"), "/me été là", "a letter beyond ASCII is a pose");
        for kept in [":)", ":-) hi", ":3 cute", ": spaced", ":D!", ":ab", "hello :waves", "/me waves", ""] {
            assert_eq!(pose(kept), kept);
        }
        // more than three bytes, as the Legacy Viewer counts them
        assert_eq!(pose(":abc"), "/me abc");
        assert_eq!(pose(":'s"), ":'s");
        assert_eq!(parse_channel("/hudping"), (0, "/hudping".into()));
    }

    use crate::ui::widgets::elide_tests::{Frames, LONG, SHORT, WIDE, painted, tooltip_shown};

    #[test]
    fn opening_conversations_focuses_the_selected_chat_without_taking_it_back() {
        for style in [
            crate::settings::ChatStyle::Aurora,
            crate::settings::ChatStyle::Compact,
            crate::settings::ChatStyle::Bubbles,
            crate::settings::ChatStyle::Legacy,
        ] {
            let mut frames = Frames::new();
            let ctx = frames.ctx.clone();
            let theme = crate::theme::Theme::default();
            theme.apply(&ctx, 1.0);
            let p = theme.palette();
            let icons = crate::ui::icons::Icons::load(&ctx, None);
            let mut emoji = crate::ui::emoji::Emoji::load();
            let pics = HashMap::new();
            let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
            let (a, b) = (uuid::Uuid::from_u128(0xA), uuid::Uuid::from_u128(0xB));
            world.social.session_mut(a);
            world.social.session_mut(b);
            let gestures = crate::gestures::Manager::default();
            let mut st = ChatUi {
                style,
                selected: Some(a),
                ..Default::default()
            };
            st.draft_mut(Some(a)).push_str("draft ");
            st.draft_mut(Some(b)).push_str("other draft");
            let mut contacts = super::super::contacts::ContactsUi::default();
            let mut contacts_settings = super::super::contacts::ContactsSettings::default();
            let other_id = egui::Id::unique("other_conversation_field");
            let mut other = String::new();
            let mut frame = |st: &mut ChatUi, mut open, events| {
                frames.run(events, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut other).id(other_id));
                    show(
                        ui.ctx(),
                        &p,
                        &icons,
                        &mut emoji,
                        &pics,
                        &pics,
                        &mut world,
                        &gestures,
                        st,
                        &mut contacts,
                        &mut contacts_settings,
                        false,
                        None,
                        &mut open,
                    );
                });
            };
            super::super::widgets::minimize_floater(&ctx, "conversations");
            frame(&mut st, true, Vec::new());
            assert!(!st.conv_focus_request, "a minimized window does not queue keyboard focus");
            super::super::widgets::expand_floater(&ctx, "conversations");
            for _ in 0..3 {
                frame(&mut st, true, Vec::new());
            }
            let field = |chat| egui::Id::unique(("conv_input", chat));
            assert!(ctx.memory(|m| m.has_focus(field(Some(a)))), "{style:?}");
            frame(&mut st, true, vec![egui::Event::Text("hello".into())]);
            assert_eq!(st.draft_mut(Some(a)), "draft hello");
            assert_eq!(st.draft_mut(Some(b)), "other draft");
            assert!(st.side_filter.is_empty());
            let other_field = if style.workspace() { sidebar::search_id() } else { other_id };
            ctx.memory_mut(|m| m.request_focus(other_field));
            frame(&mut st, true, Vec::new());
            frame(&mut st, true, vec![egui::Event::Text("filter".into())]);
            assert!(ctx.memory(|m| m.has_focus(other_field)));
            assert_eq!(st.draft_mut(Some(a)), "draft hello");
            assert_eq!(st.side_filter, if style.workspace() { "filter" } else { "" });
            frame(&mut st, false, Vec::new());
            frame(&mut st, true, Vec::new());
            assert!(ctx.memory(|m| m.has_focus(field(Some(a)))));
            frame(&mut st, true, vec![egui::Event::Text("!".into())]);
            assert_eq!(st.draft_mut(Some(a)), "draft hello!");
            super::super::widgets::minimize_floater(&ctx, "conversations");
            frame(&mut st, true, Vec::new());
            frame(&mut st, true, Vec::new());
            super::super::widgets::expand_floater(&ctx, "conversations");
            for _ in 0..3 {
                frame(&mut st, true, Vec::new());
            }
            assert!(
                ctx.memory(|m| m.has_focus(field(Some(a)))),
                "expanding restores the selected chat's focus"
            );
            frame(&mut st, false, Vec::new());
            st.selected = Some(b);
            frame(&mut st, true, Vec::new());
            frame(&mut st, true, vec![egui::Event::Text("!".into())]);
            assert_eq!(st.draft_mut(Some(b)), "other draft!");
            assert_eq!(st.draft_mut(Some(a)), "draft hello!");
            frame(&mut st, false, Vec::new());
            st.selected = None;
            frame(&mut st, true, Vec::new());
            frame(&mut st, true, vec![egui::Event::Text("nearby".into())]);
            assert_eq!(st.draft_mut(None), "nearby");
            assert_eq!(other, if style.workspace() { "" } else { "filter" });
        }
    }

    /// A tab chosen in the list gives its field the keyboard; what is typed
    /// stays with its conversation when another tab is shown, and goes when
    /// the conversation closes.
    #[test]
    fn each_tab_keeps_its_draft_and_takes_the_keyboard() {
        for style in [
            crate::settings::ChatStyle::Aurora,
            crate::settings::ChatStyle::Compact,
            crate::settings::ChatStyle::Bubbles,
            crate::settings::ChatStyle::Legacy,
        ] {
            each_tab_keeps_its_draft_and_takes_the_keyboard_in(style);
        }
    }

    fn each_tab_keeps_its_draft_and_takes_the_keyboard_in(style: crate::settings::ChatStyle) {
        let mut frames = Frames::new();
        let p = crate::theme::Theme::default().palette();
        let icons = crate::ui::icons::Icons::load(&frames.ctx, None);
        let mut emoji = crate::ui::emoji::Emoji::load();
        let pics = HashMap::new();
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let (a, b) = (uuid::Uuid::from_u128(0xA), uuid::Uuid::from_u128(0xB));
        world.social.session_mut(a);
        world.social.session_mut(b);
        let gestures = crate::gestures::Manager::default();
        let mut st = ChatUi {
            style,
            ..ChatUi::default()
        };
        let mut contacts = crate::ui::contacts::ContactsUi::default();
        let mut contacts_settings = crate::ui::contacts::ContactsSettings::default();
        let mut open = true;
        let ctx = frames.ctx.clone();
        let mut frame = |st: &mut ChatUi, events: Vec<egui::Event>| {
            frames.run(events, |ui| {
                show(
                    ui.ctx(),
                    &p,
                    &icons,
                    &mut emoji,
                    &pics,
                    &pics,
                    &mut world,
                    &gestures,
                    st,
                    &mut contacts,
                    &mut contacts_settings,
                    false,
                    None,
                    &mut open,
                );
            });
        };
        let field = |c: Option<uuid::Uuid>| egui::Id::unique(("conv_input", c));
        for (tab, text) in [(Some(a), "to A"), (Some(b), "to B"), (None, "nearby")] {
            st.select_tab(tab);
            frame(&mut st, Vec::new());
            frame(&mut st, Vec::new());
            assert!(ctx.memory(|m| m.has_focus(field(tab))), "{tab:?} takes the keyboard");
            frame(&mut st, vec![egui::Event::Text(text.into())]);
        }
        assert_eq!(st.draft_mut(Some(a)), "to A");
        assert_eq!(st.draft_mut(Some(b)), "to B");
        assert_eq!(st.draft_mut(None), "nearby");
        // back on A: its own text, typed on after the cursor at its end
        st.select_tab(Some(a));
        frame(&mut st, Vec::new());
        frame(&mut st, Vec::new());
        frame(&mut st, vec![egui::Event::Text("!".into())]);
        assert_eq!(st.draft_mut(Some(a)), "to A!");
        assert_eq!(st.draft_mut(Some(b)), "to B");
        st.forget(b);
        assert_eq!(st.draft_mut(Some(b)), "");
    }

    /// A conversation opened by the user (a contact's double click, #405)
    /// shows up with its field taking the keyboard; one opened with
    /// `focus_im` alone (a call) leaves the keyboard where it was.
    #[test]
    fn a_conversation_opened_to_type_takes_the_keyboard() {
        for style in [
            crate::settings::ChatStyle::Aurora,
            crate::settings::ChatStyle::Compact,
            crate::settings::ChatStyle::Bubbles,
            crate::settings::ChatStyle::Legacy,
        ] {
            a_conversation_opened_to_type_takes_the_keyboard_in(style);
        }
    }

    fn a_conversation_opened_to_type_takes_the_keyboard_in(style: crate::settings::ChatStyle) {
        let mut frames = Frames::new();
        let p = crate::theme::Theme::default().palette();
        let icons = crate::ui::icons::Icons::load(&frames.ctx, None);
        let mut emoji = crate::ui::emoji::Emoji::load();
        let pics = HashMap::new();
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let (a, b) = (uuid::Uuid::from_u128(0xA), uuid::Uuid::from_u128(0xB));
        world.social.session_mut(a);
        world.social.session_mut(b);
        let gestures = crate::gestures::Manager::default();
        let mut st = ChatUi {
            style,
            ..ChatUi::default()
        };
        let mut contacts = crate::ui::contacts::ContactsUi::default();
        let mut contacts_settings = crate::ui::contacts::ContactsSettings::default();
        let mut open = true;
        let ctx = frames.ctx.clone();
        let mut frame = |st: &mut ChatUi, world: &mut World, open: &mut bool| {
            frames.run(Vec::new(), |ui| {
                show(
                    ui.ctx(),
                    &p,
                    &icons,
                    &mut emoji,
                    &pics,
                    &pics,
                    world,
                    &gestures,
                    st,
                    &mut contacts,
                    &mut contacts_settings,
                    false,
                    None,
                    open,
                );
            });
        };
        let field = |c: uuid::Uuid| egui::Id::unique(("conv_input", Some(c)));
        // opened by a call: shown, the keyboard stays nowhere
        world.social.focus_im = Some(a);
        frame(&mut st, &mut world, &mut open);
        frame(&mut st, &mut world, &mut open);
        assert_eq!(st.selected, Some(a));
        assert!(ctx.memory(|m| m.focused()).is_none());
        // opened by the user: shown and ready to type in
        world.social.focus_im = Some(b);
        st.conv_focus_request = true;
        frame(&mut st, &mut world, &mut open);
        frame(&mut st, &mut world, &mut open);
        assert_eq!(st.selected, Some(b));
        assert!(ctx.memory(|m| m.has_focus(field(b))));
        // Send IM with the window minimized (#704, App::open_conversation_to_type):
        // it expands on the asked conversation, ready to type in
        crate::ui::widgets::minimize_floater(&ctx, "conversations");
        frame(&mut st, &mut world, &mut open);
        frame(&mut st, &mut world, &mut open);
        assert!(crate::ui::widgets::floater_minimized(&ctx, "conversations"));
        world.social.focus_im = Some(a);
        st.conv_focus_request = true;
        crate::ui::notifications::open_conversations(&ctx, &mut open);
        for _ in 0..4 {
            frame(&mut st, &mut world, &mut open);
        }
        assert!(open && !crate::ui::widgets::floater_minimized(&ctx, "conversations"));
        assert_eq!(st.selected, Some(a));
        assert!(ctx.memory(|m| m.has_focus(field(a))));
    }

    /// A drag across the lines selects their text without moving the
    /// window; with the keyboard on them Ctrl+A selects the conversation,
    /// a copy gives it with the senders, and what is typed goes to the field.
    #[test]
    fn the_lines_select_and_copy_like_a_transcript() {
        select_and_copy_like_a_transcript(crate::settings::ChatStyle::Bubbles);
    }

    /// The same in Aurora's workspace, roomy and compact.
    #[test]
    fn the_workspace_lines_select_and_copy_like_a_transcript() {
        select_and_copy_like_a_transcript(crate::settings::ChatStyle::Aurora);
        select_and_copy_like_a_transcript(crate::settings::ChatStyle::Compact);
    }

    fn select_and_copy_like_a_transcript(style: crate::settings::ChatStyle) {
        let mut frames = Frames::new();
        let p = crate::theme::Theme::default().palette();
        let icons = crate::ui::icons::Icons::load(&frames.ctx, None);
        let mut emoji = crate::ui::emoji::Emoji::load();
        let pics = HashMap::new();
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for (from, source, text) in [("Jane", 0xA, "hello there"), ("Bob", 0xB, "hi Jane"), ("Jane", 0xA, "bye now")] {
            world.chat.push_back(crate::world::ChatLine {
                time: std::time::SystemTime::now(),
                from: from.into(),
                text: text.into(),
                kind: ChatKind::Local(ChatType::Normal),
                source: uuid::Uuid::from_u128(source),
                origin: Origin::Live,
            });
            world.chat_clock.pushed_back();
        }
        let gestures = crate::gestures::Manager::default();
        let mut st = ChatUi {
            style,
            ..ChatUi::default()
        };
        let mut contacts = crate::ui::contacts::ContactsUi::default();
        let mut contacts_settings = crate::ui::contacts::ContactsSettings::default();
        let mut open = true;
        let ctx = frames.ctx.clone();
        let mut frame = |st: &mut ChatUi, events: Vec<egui::Event>| {
            frames.run_with_output(events, |ui| {
                show(
                    ui.ctx(),
                    &p,
                    &icons,
                    &mut emoji,
                    &pics,
                    &pics,
                    &mut world,
                    &gestures,
                    st,
                    &mut contacts,
                    &mut contacts_settings,
                    false,
                    None,
                    &mut open,
                );
            })
        };
        let copied = |out: &egui::PlatformOutput| {
            out.commands.iter().find_map(|c| match c {
                egui::OutputCommand::CopyText(t) => Some(t.clone()),
                _ => None,
            })
        };
        let mut texts = Vec::new();
        for _ in 0..3 {
            texts = frame(&mut st, Vec::new()).0;
        }
        let first = painted(&texts, "hello there").rect;
        let second = painted(&texts, "hi Jane").rect;
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        // from the start of the first message to the end of the second
        let from = egui::pos2(first.left() + 1.0, first.center().y);
        let to = egui::pos2(second.right() + 2.0, second.center().y);
        frame(&mut st, vec![egui::Event::PointerMoved(from), button(from, true)]);
        for k in 1..=4 {
            let at = from + (to - from) * (k as f32 / 4.0);
            let (_, out) = frame(&mut st, vec![egui::Event::PointerMoved(at)]);
            assert_eq!(out.cursor_icon, egui::CursorIcon::Default, "the arrow while selecting");
        }
        let (texts, _) = frame(&mut st, vec![button(to, false)]);
        assert_eq!(painted(&texts, "hello there").rect, first, "the window stayed in place");
        let (_, out) = frame(&mut st, vec![egui::Event::Copy]);
        let text = copied(&out).expect("the selection is copied");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "{text:?}");
        assert!(lines[0].starts_with('[') && lines[0].ends_with("] Jane: hello there"), "{text:?}");
        assert!(lines[1].ends_with("] Bob: hi Jane"), "{text:?}");
        // Ctrl+A: the whole conversation
        let ctrl_a = egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        };
        frame(&mut st, vec![ctrl_a]);
        let (_, out) = frame(&mut st, vec![egui::Event::Copy]);
        let text = copied(&out).expect("everything is copied");
        assert_eq!(text.lines().count(), 3, "{text:?}");
        assert!(text.ends_with("] Jane: bye now"), "{text:?}");
        // typing goes on in the conversation's field
        frame(&mut st, vec![egui::Event::Text("ok".into())]);
        frame(&mut st, Vec::new());
        frame(&mut st, Vec::new());
        assert_eq!(st.draft_mut(None), "ok");
        assert!(ctx.memory(|m| m.has_focus(egui::Id::unique(("conv_input", None::<uuid::Uuid>)))));
        // a click without a drag clears the selection
        let at = egui::pos2(second.center().x, second.center().y);
        frame(&mut st, vec![egui::Event::PointerMoved(at), button(at, true)]);
        frame(&mut st, vec![button(at, false)]);
        assert!(st.selection.1.is_empty());
        // from the empty dark area of a short conversation (under it; above
        // it in the workspace, whose lines stand at the bottom) across its
        // lines: the window does not move, every line is taken
        let (texts, _) = frame(&mut st, Vec::new());
        let last = painted(&texts, "bye now").rect;
        let (empty, end) = if style.workspace() {
            (
                egui::pos2(first.center().x, first.top() - 60.0),
                egui::pos2(last.right() + 2.0, last.center().y),
            )
        } else {
            (egui::pos2(last.center().x, last.bottom() + 80.0), from)
        };
        frame(&mut st, vec![egui::Event::PointerMoved(empty), button(empty, true)]);
        for k in 1..=4 {
            let at = empty + (end - empty) * (k as f32 / 4.0);
            frame(&mut st, vec![egui::Event::PointerMoved(at)]);
        }
        let (texts, _) = frame(&mut st, vec![button(end, false)]);
        assert_eq!(painted(&texts, "hello there").rect, first, "the window stayed in place");
        let (_, out) = frame(&mut st, vec![egui::Event::Copy]);
        let text = copied(&out).expect("the selection from the empty area is copied");
        assert_eq!(text.lines().count(), 3, "{text:?}");
        // the strip between the lines and the field: a normal cursor, and a
        // drag from it selects nothing
        frame(&mut st, vec![egui::Event::PointerMoved(from), button(from, true)]);
        frame(&mut st, vec![button(from, false)]);
        assert!(st.selection.1.is_empty());
        let field = ctx
            .read_response(egui::Id::unique(("conv_input", None::<uuid::Uuid>)))
            .expect("the conversation's field")
            .rect;
        // (in the workspace: the line of who is typing, above the box)
        let strip = egui::pos2(field.center().x, field.top() - if style.workspace() { 10.0 } else { 2.0 });
        let (_, out) = frame(&mut st, vec![egui::Event::PointerMoved(strip)]);
        assert_eq!(out.cursor_icon, egui::CursorIcon::Default, "a normal cursor on the strip");
        let (_, out) = frame(&mut st, vec![egui::Event::PointerMoved(from)]);
        assert_eq!(out.cursor_icon, egui::CursorIcon::Default, "the arrow on the lines, no text cursor");
        frame(&mut st, vec![egui::Event::PointerMoved(strip), button(strip, true)]);
        for k in 1..=4 {
            let at = strip + (from - strip) * (k as f32 / 4.0);
            frame(&mut st, vec![egui::Event::PointerMoved(at)]);
        }
        frame(&mut st, vec![button(from, false)]);
        assert!(st.selection.1.is_empty(), "a drag from the strip selects nothing");
    }

    #[test]
    fn contacts_tab_visibility_updates_the_list_and_selected_pane() {
        for style in [
            crate::settings::ChatStyle::Aurora,
            crate::settings::ChatStyle::Compact,
            crate::settings::ChatStyle::Bubbles,
            crate::settings::ChatStyle::Legacy,
        ] {
            contacts_tab_visibility_updates_the_list_and_selected_pane_in(style);
        }
    }

    fn contacts_tab_visibility_updates_the_list_and_selected_pane_in(style: crate::settings::ChatStyle) {
        let mut frames = Frames::new();
        let p = crate::theme::Theme::default().palette();
        let icons = crate::ui::icons::Icons::load(&frames.ctx, None);
        let mut emoji = crate::ui::emoji::Emoji::load();
        let pics = HashMap::new();
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let gestures = crate::gestures::Manager::default();
        let mut st = ChatUi {
            style,
            ..ChatUi::default()
        };
        let mut contacts = crate::ui::contacts::ContactsUi::default();
        let mut contacts_settings = crate::ui::contacts::ContactsSettings::default();
        let mut open = true;
        for visible in [false, true, false, true] {
            st.contacts = true;
            st.selected = Some(uuid::Uuid::from_u128(42));
            let mut texts = Vec::new();
            for _ in 0..2 {
                texts = frames.run(Vec::new(), |ui| {
                    show(
                        ui.ctx(),
                        &p,
                        &icons,
                        &mut emoji,
                        &pics,
                        &pics,
                        &mut world,
                        &gestures,
                        &mut st,
                        &mut contacts,
                        &mut contacts_settings,
                        visible,
                        None,
                        &mut open,
                    );
                });
            }
            assert_eq!(texts.iter().any(|t| t.source == aurora_i18n::tr!("common-contacts")), visible);
            assert_eq!(st.contacts, visible);
            if !visible {
                assert!(st.selected.is_none());
                assert!(local_chat_shown(&frames.ctx, &st));
            }
        }
    }

    const LONG_GROUP: &str = "The Honourable Society of Extremely Long Conference Names (Northern Chapter)";
    const USERNAME: &str = "maximilienne.wolfeschlegelsteinhausen";

    const CLOSE_TIP: &str = "Close this conversation";

    fn entry(ui: &mut egui::Ui, label: &str, pic: bool, sel: bool, unread: usize, closable: bool) -> ListEntry {
        let p = crate::theme::Theme::default().palette();
        let close = closable.then_some(CLOSE_TIP);
        list_entry(ui, &p, label, pic.then_some((None, label)), None, sel, unread, close, 0.0, false)
    }

    #[test]
    fn list_entry_keeps_long_names_inside() {
        let mut frames = Frames::new();
        for name in [LONG, WIDE, LONG_GROUP] {
            for (pic, sel, unread, closable) in [
                (true, false, 0, false),
                (true, true, 0, false),
                (true, false, 3, false),
                (false, false, 1, false),
                (true, false, 0, true),
                (true, true, 0, true),
                (true, false, 3, true),
            ] {
                let mut rect = egui::Rect::NOTHING;
                let texts = frames.run(Vec::new(), |ui| rect = entry(ui, name, pic, sel, unread, closable).resp.rect);
                assert_eq!(rect.width(), ENTRY_W);
                let t = painted(&texts, name);
                assert!(t.cut && t.shown.ends_with('…'), "{name:?} shown as {:?}", t.shown);
                // before the close button of an entry that has one, before
                // the unread count when there is one, else inside the entry
                let limit = if closable {
                    trailing_slot(rect).left()
                } else if unread > 0 {
                    trailing_slot(rect).right() - SLOT_SIDE + 1.0
                } else {
                    rect.right()
                };
                assert!(t.rect.right() < limit, "{name:?} reaches {} (limit {limit})", t.rect.right());
                // after the picture
                assert!(t.rect.left() >= rect.left() + if pic { 24.0 } else { 8.0 });
                // and it still says something
                assert!(t.shown.chars().count() > 4, "{name:?} shown as {:?}", t.shown);
            }
        }
    }

    #[test]
    fn list_entry_shows_the_whole_name_on_hover_only_when_cut() {
        for (name, cut) in [(LONG, true), (LONG_GROUP, true), (SHORT, false)] {
            let mut frames = Frames::new();
            let mut rect = egui::Rect::NOTHING;
            let count = |texts: &[crate::ui::widgets::elide_tests::Text]| texts.iter().filter(|t| t.source == name).count();
            let texts = frames.run(Vec::new(), |ui| rect = entry(ui, name, false, false, 2, false).resp.rect);
            assert_eq!(painted(&texts, name).cut, cut);
            assert_eq!(count(&texts), 1);
            let texts = frames.hover(rect.center(), |ui| {
                entry(ui, name, false, false, 2, false);
            });
            assert_eq!(count(&texts), if cut { 2 } else { 1 }, "{name:?}");
            assert!(!cut || tooltip_shown(&texts, name), "{name:?}");
        }
    }

    /// More conversations than the list is high: the ones below are out of
    /// its view, the list scrolls to the one that is opened from elsewhere,
    /// and the wheel brings the first ones back.
    #[test]
    fn conversations_beyond_the_height_of_the_list_are_reached_by_scrolling() {
        const HEIGHT: f32 = 120.0;
        let names: Vec<String> = (1..=24).map(|i| format!("Resident{i:02}")).collect();
        let mut frames = Frames::new();
        let mut draw = |reveal: Option<usize>, events: Vec<egui::Event>| {
            let (mut view, mut rects) = (egui::Rect::NOTHING, Vec::new());
            frames.run(events, |ui| {
                scrolling_list(ui, HEIGHT, |ui| {
                    view = ui.clip_rect();
                    for (i, name) in names.iter().enumerate() {
                        let e = entry(ui, name, true, false, 0, true);
                        if reveal == Some(i) {
                            e.resp.scroll_to_me(None);
                        }
                        rects.push(e.resp.rect);
                    }
                });
            });
            (view, rects)
        };
        let (view, rects) = draw(None, Vec::new());
        assert!(view.height() <= HEIGHT + 1.0, "{view:?}");
        assert!(view.contains_rect(rects[0]) && !view.intersects(rects[23]));
        // opened from a notification: its entry comes into view
        draw(Some(23), Vec::new());
        let (view, rects) = draw(None, Vec::new());
        assert!(
            view.contains_rect(rects[23]) && !view.intersects(rects[0]),
            "{view:?} {:?}",
            rects[23]
        );
        // the wheel, pointer over the list, goes back to the first ones
        let over = egui::pos2(rects[23].center().x, view.center().y);
        draw(None, vec![egui::Event::PointerMoved(over)]);
        let wheel = egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, 2000.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: egui::Modifiers::NONE,
        };
        draw(None, vec![wheel]);
        // (the wheel's travel is spread over a few frames)
        for _ in 0..30 {
            draw(None, Vec::new());
        }
        let (view, rects) = draw(None, Vec::new());
        assert!(
            view.contains_rect(rects[0]) && !view.intersects(rects[23]),
            "{view:?} {:?}",
            rects[0]
        );
    }

    /// One click at `at` on an entry drawn the same way each frame:
    /// (the entry was clicked, its close button was clicked).
    fn click_entry(at: impl Fn(egui::Rect) -> egui::Pos2, sel: bool, closable: bool) -> (bool, bool) {
        let mut frames = Frames::new();
        let mut rect = egui::Rect::NOTHING;
        frames.run(Vec::new(), |ui| rect = entry(ui, SHORT, true, sel, 2, closable).resp.rect);
        let pos = at(rect);
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let mut out = (false, false);
        for events in [vec![egui::Event::PointerMoved(pos)], vec![button(true)], vec![button(false)]] {
            frames.run(events, |ui| {
                let e = entry(ui, SHORT, true, sel, 2, closable);
                out = (e.resp.clicked(), e.close);
            });
        }
        out
    }

    #[test]
    fn list_entry_close_button_closes_without_selecting() {
        let on_button = |r: egui::Rect| trailing_slot(r).center();
        let on_name = |r: egui::Rect| r.left_center() + egui::vec2(40.0, 0.0);
        for sel in [false, true] {
            assert_eq!(click_entry(on_button, sel, true), (false, true), "selected: {sel}");
            assert_eq!(click_entry(on_name, sel, true), (true, false), "selected: {sel}");
            // the nearby chat and the contacts have no button: the same
            // spot selects them
            assert_eq!(click_entry(on_button, sel, false), (true, false), "selected: {sel}");
        }
    }

    /// The close button of a closable entry: always on the selected entry,
    /// under the pointer on the others, which show their unread dot the
    /// rest of the time. Its tooltip tells it from the entry.
    #[test]
    fn list_entry_shows_its_close_button_when_selected_or_hovered() {
        let tip = |texts: &[crate::ui::widgets::elide_tests::Text]| texts.iter().any(|t| t.source == CLOSE_TIP);
        for sel in [false, true] {
            let mut frames = Frames::new();
            let mut rect = egui::Rect::NOTHING;
            frames.run(Vec::new(), |ui| rect = entry(ui, SHORT, true, sel, 0, true).resp.rect);
            let texts = frames.hover(trailing_slot(rect).center(), |ui| {
                entry(ui, SHORT, true, sel, 0, true);
            });
            assert!(tip(&texts), "selected: {sel}");
            // away from the entry, only the selected one keeps its button
            let away = rect.center_bottom() + egui::vec2(0.0, 200.0);
            frames.hover(away, |ui| {
                entry(ui, SHORT, true, sel, 0, true);
            });
            let mut shown = false;
            frames.run(Vec::new(), |ui| {
                let e = entry(ui, SHORT, true, sel, 0, true);
                shown = ui.ctx().read_response(e.resp.id.with("close")).is_some();
            });
            assert_eq!(shown, sel);
        }
    }

    /// The unread count of an entry ends where the close button of another
    /// does, whether the entry can be closed or not.
    #[test]
    fn unread_counts_and_close_buttons_are_aligned() {
        let p = crate::theme::Theme::default().palette();
        let mut frames = Frames::new();
        // (nearby chat with unread lines, unread IM, IM under the pointer)
        let hovered = std::cell::Cell::new(egui::Rect::NOTHING);
        let draw = |ui: &mut egui::Ui| {
            entry(ui, "Nearby chat", false, false, 2, false);
            entry(ui, SHORT, true, false, 2, true);
            hovered.set(entry(ui, LONG, true, false, 2, true).resp.rect);
        };
        frames.shapes(Vec::new(), draw);
        let on_button = trailing_slot(hovered.get()).center();
        frames.shapes(vec![egui::Event::PointerMoved(on_button)], draw);
        let shapes = frames.shapes(Vec::new(), draw);
        let counts: Vec<egui::Rect> = shapes
            .iter()
            .filter_map(|s| match s {
                egui::Shape::Rect(r) if r.fill == p.danger_dim => Some(r.rect),
                _ => None,
            })
            .collect();
        // the face of the hovered close button, the only square of the list
        let faces: Vec<egui::Rect> = shapes
            .iter()
            .filter_map(|s| match s {
                egui::Shape::Rect(r) if r.fill == p.raised && r.rect.width() == SLOT_SIDE => Some(r.rect),
                _ => None,
            })
            .collect();
        assert_eq!(counts.len(), 2, "the hovered entry shows its button instead of its count");
        assert_eq!(faces.len(), 1);
        assert_eq!(counts[0].right(), counts[1].right());
        assert_eq!(counts[0].right(), faces[0].right() - 1.0);
        assert_eq!(faces[0].center().y, hovered.get().center().y);
    }

    /// A message header `width` wide: (row, name, time) rects.
    fn header(ui: &mut egui::Ui, width: f32, name: &str, username: Option<&str>) -> (egui::Rect, egui::Rect, egui::Rect) {
        let p = crate::theme::Theme::default().palette();
        let mut rects = (egui::Rect::NOTHING, egui::Rect::NOTHING);
        let row = ui
            .allocate_ui_with_layout(egui::vec2(width, 22.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let time = header_time(ui, &p, "23:59".into());
                let reserve = time.size().x + ui.spacing().item_spacing.x;
                rects.0 = header_names(ui, &p, name, username, p.ink, reserve).rect;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    rects.1 = ui.label(time).rect;
                });
            })
            .response
            .rect;
        (row, rects.0, rects.1)
    }

    #[test]
    fn header_names_stop_before_the_time() {
        let mut frames = Frames::new();
        let small = format!("- {USERNAME}");
        for name in [LONG, WIDE, SHORT] {
            for width in (90..=1200).step_by(11).map(|w| w as f32) {
                let mut rects = None;
                let texts = frames.run(Vec::new(), |ui| rects = Some(header(ui, width, name, Some(USERNAME))));
                let (row, name_rect, time) = rects.expect("the header is drawn");
                assert!(
                    row.width() <= width + 0.01,
                    "{name:?} widens the header to {} at {width} px",
                    row.width()
                );
                assert!(name_rect.right() <= time.left(), "{name:?} runs under the time at {width} px");
                // the username, when there is room for it, stops before the time too
                if let Some(u) = texts.iter().find(|t| t.source == small) {
                    assert!(u.rect.left() >= name_rect.right());
                    assert!(u.rect.right() <= time.left() + 0.01, "username under the time at {width} px");
                    assert!(u.rect.width() >= USERNAME_MIN_W || !u.cut);
                }
            }
        }
    }

    #[test]
    fn header_names_are_whole_when_there_is_room() {
        let mut frames = Frames::new();
        let small = format!("- {USERNAME}");
        let texts = frames.run(Vec::new(), |ui| {
            header(ui, 1200.0, LONG, Some(USERNAME));
        });
        assert!(!painted(&texts, LONG).cut);
        assert!(!painted(&texts, &small).cut);
        // no room left after the name: the username waits on hover of the name
        let mut name_rect = egui::Rect::NOTHING;
        let texts = frames.run(Vec::new(), |ui| name_rect = header(ui, 200.0, LONG, Some(USERNAME)).1);
        assert!(painted(&texts, LONG).cut);
        assert!(texts.iter().all(|t| t.source != small));
        let texts = frames.hover(name_rect.center(), |ui| {
            header(ui, 200.0, LONG, Some(USERNAME));
        });
        assert!(tooltip_shown(&texts, LONG));
        assert!(texts.iter().any(|t| t.source == USERNAME));
    }
}
