//! Local chat window (nearby chat + instant messages).

mod bubbles;
mod call;
mod completion;
mod compose;
mod find;
mod head;
mod header;
pub mod history;
mod invite_card;
mod members;
pub(crate) mod select;
mod sidebar;
mod stage;
mod thread;
mod typing;
mod view;
mod workspace;

use super::context::CtxAction;
use super::presence;
use crate::theme::Palette;
use crate::world::chat_log::format::Slt;
use crate::world::{ChatKind, Origin, World};
use aurora_net::ChatType;
use egui::{Color32, RichText};
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub struct ChatUi {
    pub input: String,
    pub focus_request: bool,
    pub has_focus: bool,
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
    /// The field of the conversation shown takes the keyboard (its tab was
    /// clicked, a letter key started the nearby chat).
    pub conv_focus_request: bool,
    /// The text selected in the lines of a conversation (`select.rs`), and
    /// which conversation: another tab starts without one.
    selection: (Option<uuid::Uuid>, select::Selection),
    /// `AURORA_DEMO_CONV_SELECT`: a selection to make once lines are shown
    /// ("all", or "part" of three lines).
    pub demo_select: Option<String>,
    /// Avatars whose profile picture the conversation wants (filled while drawing).
    pub wanted_pics: HashSet<uuid::Uuid>,
    /// Names to resolve (mentions) — taken by the app.
    pub wanted_names: HashSet<uuid::Uuid>,
    /// Participants of the group / conference session shown.
    pub show_profile: bool,
    /// Filter and find bar of each conversation (None = nearby chat).
    searches: HashMap<Option<uuid::Uuid>, find::Search>,
    /// Laid-out rows of each conversation.
    views: HashMap<Option<uuid::Uuid>, view::View>,
    pub emoji_open: bool,
    /// The emoji picker of the chat island's field is open.
    pub bar_emoji_open: bool,
    /// The lines' style of the settings, given every frame by the app.
    pub style: crate::settings::ChatStyle,
    /// A demo scenario's style, which the settings do not keep.
    pub demo_style: Option<crate::settings::ChatStyle>,
    /// Voice is enabled (calls possible), given every frame by the app.
    pub voice_on: bool,
    /// The microphone in toggle mode (None: hold to talk), for the call's
    /// mute button.
    pub mic: Option<bool>,
    /// Residents' voice volumes, for the call's participants.
    pub volumes: std::collections::BTreeMap<uuid::Uuid, f32>,
    /// The times of the lines are shown (Preferences › Chat), given every
    /// frame by the app.
    pub timestamps: bool,
    /// Typing notices are sent (Preferences › Chat), given every frame by
    /// the app.
    pub send_typing: bool,
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
    /// Where the "New" line of each conversation sits: its first unread
    /// line when it was opened, until it is left.
    first_unread: HashMap<Option<uuid::Uuid>, u64>,
    /// The conversation shown at the last frame.
    shown: Option<Option<uuid::Uuid>>,
    /// The search field of the column.
    side_filter: String,
    /// The nearby chat's volume: whisper, say or shout.
    volume: compose::Volume,
    /// Our typing notices, and what the field did this frame.
    me_typing: typing::MeTyping,
    typing_field: Option<typing::Field>,
    /// The resident picker of the column's "+", and what it starts.
    picker: super::avatar_picker::AvatarPicker,
    start: Start,
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
    /// Show a conversation, its entry brought into the view of the list.
    pub fn open(&mut self, conversation: uuid::Uuid) {
        self.selected = Some(conversation);
        self.reveal = Some(conversation);
        self.show_profile = false;
        self.contacts = false;
    }

    /// A conversation closed: its filter, find bar, rows and draft go with it.
    pub fn forget(&mut self, conversation: uuid::Uuid) {
        self.searches.remove(&Some(conversation));
        self.views.remove(&Some(conversation));
        self.drafts.remove(&Some(conversation));
    }

    /// What is typed in a conversation's field (None = nearby chat).
    pub fn draft_mut(&mut self, conversation: Option<uuid::Uuid>) -> &mut String {
        self.drafts.entry(conversation).or_default()
    }

    /// Show a tab of the window and give its field the keyboard.
    fn select_tab(&mut self, conversation: Option<uuid::Uuid>) {
        self.selected = conversation;
        self.show_profile = false;
        self.contacts = false;
        self.conv_focus_request = true;
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
}

pub struct OutgoingChat {
    pub message: String,
    pub channel: i32,
    pub chat_type: ChatType,
}

/// Parse "/5 text" channel prefixes.
pub fn parse_channel(text: &str) -> (i32, String) {
    if let Some(rest) = text.strip_prefix('/') {
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit() || *c == '-').collect();
        if !digits.is_empty()
            && let Ok(ch) = digits.parse::<i32>()
        {
            let msg = rest[digits.len()..].trim_start().to_owned();
            return (ch, msg);
        }
    }
    (0, text.to_owned())
}

fn color_for(kind: ChatKind, p: &Palette) -> Color32 {
    match kind {
        ChatKind::Own => p.violet_pale,
        ChatKind::Local(_) => p.ink,
        ChatKind::Object(_) | ChatKind::ObjectIm => p.teal.gamma_multiply(0.85),
        ChatKind::System | ChatKind::Invite(_) => p.muted,
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
}

/// A line's text cut into pieces, links replaced by their labels as
/// Firestorm's chat console does (LLConsole::Paragraph with parse_urls,
/// indra/llui/llconsole.cpp): places read "Region (x,y,z)", agent links the
/// avatar's name, mentions "@name" like the conversation window.
fn pieces(text: &str, mut name_of: impl FnMut(uuid::Uuid) -> String) -> Vec<(String, Option<Link>)> {
    segments(text)
        .into_iter()
        .map(|s| match s {
            Seg::Text(t) | Seg::Slurl(t) => (t.to_owned(), None),
            Seg::Url(u) => (u.to_owned(), Some(Link::Url(u.to_owned()))),
            Seg::Place(u, place) => (place.label.clone(), Some(Link::Place(u.to_owned(), place))),
            Seg::Agent(id) => (name_of(id), Some(Link::Agent(id))),
            Seg::Mention(id) => (format!("@{}", name_of(id)), Some(Link::Agent(id))),
        })
        .collect()
}

/// Check, warning triangle or cross before a web link (`link_trust`), with
/// its explanation.
fn link_badge_look(p: &Palette, url: &str) -> (&'static str, Color32, String) {
    use crate::link_trust::Trust;
    match crate::link_trust::classify(url) {
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
    if super::widgets::region_badge(ui, p, ratings, &place.location.region, size).is_some() {
        ui.add_space(3.0);
    }
    let k = super::colors::get();
    let behind = select::atom_begin(ui);
    let r = ui.add(egui::Link::new(
        RichText::new(&place.label).size(size).color(super::colors::c(k.chat_slurl)),
    ));
    select::atom_end(ui, behind, &place.label, r.rect);
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
                CtxAction::TeleportToPlace(l.region, l.pos)
            } else {
                CtxAction::ShowPlaceInfo(l.region, l.pos)
            },
        );
    }
}

/// A notification's or a script dialog's text: plain and selectable, its
/// place links clickable with their region's badge (Firestorm's toasts
/// parse their links too, LLToastNotifyPanel's text editor).
pub(crate) fn text_with_places(
    ui: &mut egui::Ui,
    p: &Palette,
    ratings: &crate::world::worldmap::RegionRatings,
    text: &str,
    size: f32,
    color: Color32,
) {
    let places: Vec<_> = segments(text)
        .into_iter()
        .filter_map(|s| match s {
            Seg::Place(u, place) => Some((u, place)),
            _ => None,
        })
        .collect();
    let plain = |ui: &mut egui::Ui, t: &str| {
        if !t.is_empty() {
            ui.add(egui::Label::new(RichText::new(t).size(size).color(color)).wrap().selectable(true));
        }
    };
    if places.is_empty() {
        plain(ui, text);
        return;
    }
    super::emoji::text_rows(ui, size, |ui| {
        let mut at = 0;
        for (u, place) in places {
            let Some(i) = text[at..].find(u) else {
                continue;
            };
            plain(ui, &text[at..at + i]);
            place_link(ui, p, ratings, u, place, size);
            at += i + u.len();
        }
        plain(ui, &text[at..]);
    });
}

/// Open a web link of a text: trusted ones open at once, unknown ones after
/// a warning (unless turned off), dangerous ones always after a warning
/// (`CtxAction::OpenUrl`).
pub(crate) fn open_web_link(ctx: &egui::Context, url: &str) {
    super::context::request(ctx, CtxAction::OpenUrl(url.to_owned()));
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
}

/// Side of the sender's picture, for a text size.
fn pic_side(size: f32) -> f32 {
    size + 3.0
}

fn line_job(
    line: &crate::world::ChatLine,
    p: &Palette,
    world: &World,
    want_names: &mut HashSet<uuid::Uuid>,
    width: f32,
    size: f32,
    times: bool,
    pill: Option<&egui::FontFamily>,
) -> BubbleJob {
    let col = color_for(line.kind, p);
    let mut job = egui::text::LayoutJob::default();
    let small = egui::TextFormat {
        color: p.muted_dim,
        font_id: egui::FontId::proportional(size - 3.0),
        ..Default::default()
    };
    if times {
        job.append(&format!("[{}] ", hhmm(line.time)), 0.0, small);
    }
    let emote = line.text.starts_with("/me ") || line.text.starts_with("/me'");
    // the pills of the islands (the bars mockup's `.toasts`): a resident's
    // name in semi-bold light violet, no colon, a system line in italics;
    // every other colour stays the kind's
    let resident = matches!(line.kind, ChatKind::Local(_) | ChatKind::Own);
    let name_fmt = match pill {
        Some(family) => super::fonts::semibold_format(family, size, if resident { p.violet_light } else { col }),
        None => egui::TextFormat {
            color: col,
            font_id: egui::FontId::proportional(size),
            ..Default::default()
        },
    };
    let text_fmt = egui::TextFormat {
        color: col,
        font_id: egui::FontId::proportional(size),
        italics: emote || (pill.is_some() && line.kind.is_notice()),
        ..Default::default()
    };
    // (before the name, after it, body)
    let (before, after, body) = match line.kind {
        ChatKind::System | ChatKind::Invite(_) => ("", "", &line.text[..]),
        _ if emote => ("", "", &line.text[3..]),
        ChatKind::Im => ("[IM] ", ": ", &line.text[..]),
        ChatKind::Local(ChatType::Shout) => ("", aurora_i18n::tr!("status-shouts"), &line.text[..]),
        ChatKind::Local(ChatType::Whisper) => ("", aurora_i18n::tr!("status-whispers"), &line.text[..]),
        _ if pill.is_some() => ("", "", &line.text[..]),
        _ => ("", ": ", &line.text[..]),
    };
    // in a pill the body follows the name 4 px away instead of a colon
    // (an emote's body starts with its own space)
    let name_gap = if pill.is_some() && after.is_empty() && !emote && !line.kind.is_notice() {
        4.0
    } else {
        0.0
    };
    let mut sender = None;
    let mut name = None;
    if !line.kind.is_notice() {
        job.append(before, 0.0, name_fmt.clone());
        let who = match line.kind {
            ChatKind::Object(_) | ChatKind::ObjectIm => Some(Sender::Object),
            _ if !line.source.is_nil() => Some(Sender::Avatar(line.source)),
            _ => None,
        };
        // room for the picture, painted over it once laid out (none in a
        // pill)
        let who = who.filter(|_| pill.is_none());
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
    let name_of = |id| {
        want_names.insert(id);
        world.social.name_of(&id)
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
        let badge = match &link {
            Some(Link::Url(u)) => Some(LinkBadge::Web(u.clone())),
            Some(Link::Place(_, place)) => world.map.ratings.get(&place.location.region).map(LinkBadge::Rating),
            _ => None,
        };
        if let Some(badge) = badge {
            job.append("M", std::mem::take(&mut lead), badge_fmt.clone());
            badges.push((at, badge));
            at += 1;
        }
        let n = piece.chars().count();
        // links in the link colors, like in the conversation window
        let k = super::colors::get();
        let fmt = match &link {
            None => text_fmt.clone(),
            Some(Link::Url(_)) => egui::TextFormat {
                color: super::colors::c(k.chat_urls),
                ..text_fmt.clone()
            },
            Some(_) => egui::TextFormat {
                color: super::colors::c(k.chat_slurl),
                ..text_fmt.clone()
            },
        };
        job.append(&piece, std::mem::take(&mut lead), fmt);
        if let Some(link) = link {
            links.push((at..at + n, link));
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
    /// Show the torn-off Contacts window (its entry in the list).
    OpenContacts,
    /// Block / unblock the avatar of a 1:1 conversation.
    ToggleBlock(uuid::Uuid),
    /// Close a conversation (the X of its tab in FSFloaterIMContainer): a
    /// group / conference session is left, a 1:1 conversation leaves the list.
    Close(uuid::Uuid),
    /// Stop receiving a group's chat (and leave its session).
    BlockGroupChat(uuid::Uuid),
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
    Typing { to: uuid::Uuid, typing: bool },
    /// Inventory dropped on a conversation with a resident: give it, once
    /// confirmed (the inventory's Share dialog).
    GiveItems { to: uuid::Uuid, items: Vec<uuid::Uuid> },
    /// Open the inventory, to drag what to give from it.
    OpenInventory,
    /// Play one of our active gestures.
    PlayGesture(uuid::Uuid),
    /// The audio output of the call ("" = the system's default).
    SetOutputDevice(String),
    /// List the audio outputs again (their menu opens).
    ListOutputDevices,
}

use super::widgets::initials;

/// Profile picture (or initials) of an avatar, `size` px square.
fn avatar_pic(ui: &mut egui::Ui, p: &Palette, pic: Option<&egui::TextureHandle>, name: &str, size: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    super::widgets::avatar_picture(ui, p, rect, pic, name, false);
}

/// Tabler icon button of the conversation toolbar.
fn tool(ui: &mut egui::Ui, p: &Palette, icons: &super::icons::Icons, icon: &str, tip: &str, enabled: bool, active: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(26.0, 22.0), egui::Sense::click());
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
}

fn segments(text: &str) -> Vec<Seg<'_>> {
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let next = ["https://", "http://", "secondlife://"].iter().filter_map(|p| rest.find(p)).min();
        let Some(i) = next else {
            out.push(Seg::Text(rest));
            break;
        };
        if i > 0 {
            out.push(Seg::Text(&rest[..i]));
        }
        let tail = &rest[i..];
        let mut end = tail.find(char::is_whitespace).unwrap_or(tail.len());
        // trailing punctuation belongs to the sentence
        while end > 0 && tail[..end].ends_with(['.', ',', '!', '?', ')', ';', ':', '"', '\'']) {
            end -= 1;
        }
        let link = &tail[..end.max(1)];
        let seg = if let Some(rest) = link.strip_prefix("secondlife:///app/agent/") {
            let (id, action) = rest.split_once('/').unwrap_or((rest, ""));
            match (uuid::Uuid::parse_str(id).ok(), action) {
                (Some(id), "mention") => Seg::Mention(id),
                (Some(id), "about" | "inspect" | "completename" | "displayname" | "username") => Seg::Agent(id),
                _ => Seg::Slurl(link),
            }
        } else if let Some(place) = crate::slurl::place_link(link) {
            Seg::Place(link, place)
        } else if link.starts_with("secondlife://") {
            Seg::Slurl(link)
        } else {
            Seg::Url(link)
        };
        out.push(seg);
        rest = &tail[link.len()..];
    }
    out
}

/// Right click on the sender of a chat line: the name menu, its report on
/// the line.
fn line_name_menu(r: &egui::Response, p: &Palette, world: &World, id: uuid::Uuid, line: &crate::world::ChatLine) {
    super::menu::context_menu(r, p, |ui| super::context::chat_sender_menu(ui, p, world, id, line));
}

/// Right click on an avatar name in a conversation (menu_url_agent.xml).
fn name_menu(r: &egui::Response, p: &Palette, world: &World, id: uuid::Uuid) {
    super::menu::context_menu(r, p, |ui| {
        super::context::avatar_list_menu(ui, p, world, id, super::context::AvatarList::Name)
    });
}

/// Right click on a place link: menu_url_slurl.xml (place details first)
/// or menu_url_teleport.xml (teleport first), in Firestorm's order.
fn place_menu(r: &egui::Response, p: &Palette, url: &str, place: &crate::slurl::PlaceLink) {
    super::menu::context_menu(r, p, |ui| {
        let l = &place.location;
        let teleport = |ui: &mut egui::Ui| {
            if super::menu::item(ui, p, "location-filled", aurora_i18n::tr!("ui-chat-teleport-to-this-location")) {
                super::context::request(ui.ctx(), CtxAction::TeleportToPlace(l.region.clone(), l.pos));
            }
        };
        let map_label = if place.teleport {
            aurora_i18n::tr!("common-show-on-map")
        } else {
            aurora_i18n::tr!("common-show-on-map-2")
        };
        let map = |ui: &mut egui::Ui| {
            if super::menu::item(ui, p, "map", map_label) {
                super::context::request(ui.ctx(), CtxAction::ShowPlace(l.region.clone(), l.pos));
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
                super::context::request(ui.ctx(), CtxAction::ShowPlaceInfo(l.region.clone(), l.pos));
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
    let segs = segments(text);
    // only a text that has a match is cut into runs
    let mark = mark.filter(|m| !view::find_ci(text, m.needle).is_empty());
    if mark.is_none() && segs.iter().all(|s| matches!(s, Seg::Text(_))) {
        emoji.rich_text(ui, text, size, color, italics);
        return;
    }
    let k = super::colors::get();
    super::emoji::text_rows(ui, size, |ui| {
        for s in segs {
            match s {
                Seg::Text(t) => marked_run(ui, emoji, t, size, color, italics, mark),
                Seg::Url(u) => {
                    link_badge(ui, p, u, size);
                    let behind = select::atom_begin(ui);
                    let r = ui.add(egui::Link::new(RichText::new(u).size(size).color(super::colors::c(k.chat_urls))));
                    select::atom_end(ui, behind, u, r.rect);
                    if r.on_hover_text(u).clicked() {
                        open_web_link(ui.ctx(), u);
                    }
                }
                Seg::Slurl(u) => {
                    select::label(ui, RichText::new(u).size(size).color(super::colors::c(k.chat_slurl))).on_hover_text(u);
                }
                Seg::Place(u, place) => place_link(ui, p, &world.map.ratings, u, place, size),
                Seg::Mention(id) => {
                    want_names.insert(id);
                    let me = id == world.agent_id;
                    let bg = if me { k.mention_me } else { k.mention_residents };
                    let label = format!(" @{} ", world.social.name_of(&id));
                    let behind = select::atom_begin(ui);
                    let r = ui.add(
                        egui::Label::new(
                            RichText::new(&label)
                                .size(size)
                                .color(super::colors::c(k.mention_text))
                                .background_color(super::colors::c(bg)),
                        )
                        .sense(egui::Sense::click()),
                    );
                    select::atom_end(ui, behind, &label, r.rect);
                    name_menu(&r, p, world, id);
                    if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        super::profile::request_open(ui.ctx(), id);
                    }
                }
                Seg::Agent(id) => {
                    want_names.insert(id);
                    let name = world.social.name_of(&id);
                    let behind = select::atom_begin(ui);
                    let r = ui.add(
                        egui::Label::new(RichText::new(&name).size(size).color(super::colors::c(k.chat_slurl))).sense(egui::Sense::click()),
                    );
                    select::atom_end(ui, behind, &name, r.rect);
                    name_menu(&r, p, world, id);
                    if r.on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text(aurora_i18n::tr!("common-view-profile"))
                        .clicked()
                    {
                        super::profile::request_open(ui.ctx(), id);
                    }
                }
            }
        }
    });
}

/// Height of the input row: the emoji button, the field and the send button
/// stand on one line, all as tall.
const INPUT_H: f32 = 28.0;

/// Icon button returning its response (the emoji button before the input).
fn tool_resp(ui: &mut egui::Ui, p: &Palette, icons: &super::icons::Icons, icon: &str, active: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::Vec2::splat(INPUT_H), egui::Sense::click());
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
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(SEND_BTN_W, INPUT_H), egui::Sense::click());
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
    let logged = line.origin != Origin::Live;
    let own = line.kind == ChatKind::Own;
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
        None => line.from.clone(),
    }
}

/// The text of a message and whether it is in italics: an emote is the
/// sender's name in italics, a shout or a whisper says so.
pub(crate) fn line_body(line: &crate::world::ChatLine, name: &str) -> (String, bool) {
    let emote = line.text.starts_with("/me ") || line.text.starts_with("/me'");
    if emote {
        (format!("{}{}", name, &line.text[3..]), true)
    } else {
        let prefix = match line.kind {
            ChatKind::Local(ChatType::Shout) => aurora_i18n::tr!("status-shouts-2"),
            ChatKind::Local(ChatType::Whisper) => aurora_i18n::tr!("status-whispers-2"),
            _ => "",
        };
        (format!("{prefix}{}", line.text), false)
    }
}

/// The text a line shows, as a copy of it gives it: links as written,
/// places, names and mentions as shown.
fn shown_text(world: &World, line: &crate::world::ChatLine, name: &str) -> String {
    let body = match line.kind {
        ChatKind::System | ChatKind::Invite(_) => line.text.clone(),
        _ => line_body(line, name).0,
    };
    segments(&body)
        .into_iter()
        .map(|s| match s {
            Seg::Text(t) | Seg::Url(t) | Seg::Slurl(t) => t.to_owned(),
            Seg::Place(_, place) => place.label,
            Seg::Mention(id) => format!(" @{} ", world.social.name_of(&id)),
            Seg::Agent(id) => world.social.name_of(&id),
        })
        .collect()
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
/// a drag selects across them (never moving the window), a click without a
/// drag clears it; a press gives them the keyboard, then Ctrl+A selects the
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
    // stays neutral (normal cursor, no selection, no keyboard)
    let area = content.rect.intersect(ui.clip_rect());
    let pointer_in = ui.input(|i| i.pointer.hover_pos()).is_some_and(|p| area.contains(p));
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
    if (content.hovered() && pointer_in) || (content.dragged() && pressed_in) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
    }
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
            let unread = std::mem::take(&mut world.chat_unread);
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
    let (layout, first_unread) = match &rows {
        Rows::Classic(_) => (view::Layout::Classic, None),
        Rows::Thread(th) if th.compact => (view::Layout::Compact, st.first_unread.get(&st.selected).copied()),
        Rows::Thread(_) => (view::Layout::Aurora, st.first_unread.get(&st.selected).copied()),
    };
    let mut to_field = ToField::Nothing;
    let (lines, clock) = match st.selected {
        None => (Some(&w.chat), w.chat_clock),
        Some(id) => match w.social.ims.iter().find(|s| s.other == id) {
            Some(s) => (Some(&s.lines), s.clock),
            None => (None, w.chat_clock),
        },
    };
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
        bubbles: match rows {
            Rows::Classic(style) => style,
            Rows::Thread(_) => None,
        },
    };
    let wash = find::wash(p);
    let current = search.find.as_ref().and_then(|f| f.current_line(view));
    let out = egui::ScrollArea::vertical()
        .id_salt(("conv", st.selected))
        .stick_to_bottom(true)
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
            let mark = (!needle.is_empty()).then_some(Mark { needle: &needle, wash });
            let selection = &mut st.selection.1;
            let (shift, content) = view.show(ui, viewport, clock.first, &mut |ui, row, painted| {
                let Some(line) = lines.get(row.line) else {
                    return;
                };
                let mark = mark.filter(|_| row.hit);
                let mut draw = |ui: &mut egui::Ui| match &mut rows {
                    Rows::Classic(_) => draw_line(ui, &mut env, line, row.look, mark),
                    Rows::Thread(th) => thread::draw(ui, &mut env, th, line, row.look, mark, painted),
                };
                if matches!(line.kind, ChatKind::Invite(_)) {
                    // a card: its own labels and buttons
                    draw(ui);
                } else {
                    let seq = clock.first + row.line as u64;
                    select::row(ui, seq, selection.of_row(seq), painted, draw);
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
            shift
        });
    if out.inner != 0.0 {
        // rows above changed height: what is in sight stays
        let mut state = out.state;
        state.offset.y += out.inner;
        state.store(ui.ctx(), out.id);
    }
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
    /// The lines as bubbles (`bubbles.rs`), with the sender's name opening
    /// each group when `Some(true)` (several people write); `None`: the
    /// classic lines with their header strips.
    pub bubbles: Option<bool>,
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
    // history lines: greyed, and nothing is looked up for them (no name, no
    // picture, no profile), like Firestorm's message_from_log
    let logged = line.origin != Origin::Live;
    let tone = if env.plain_history { None } else { history_tone(p, line.origin) };
    if let Some((year, month, day)) = look.marker {
        date_marker(ui, p, year, month, day);
    }
    if let ChatKind::Invite(invite) = line.kind {
        invite_card::draw_line(ui, env, line, invite);
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
    let own = line.kind == ChatKind::Own;
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
                        if let Some(t) = env.icons.get("box") {
                            ui.painter().image(
                                t.id(),
                                rect.shrink(2.0),
                                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                super::colors::sender_color(line),
                            );
                        }
                    } else {
                        if !line.source.is_nil() && !logged {
                            env.wanted.insert(line.source);
                        }
                        avatar_pic(ui, p, env.pics.get(&line.source), &name, 20.0);
                    }
                    let col = tone.unwrap_or_else(|| super::colors::sender_color(line));
                    let time = header_time(ui, p, hhmm(line.time));
                    let reserve = time.size().x + ui.spacing().item_spacing.x;
                    let r = header_names(ui, p, &name, username.as_deref(), col, reserve);
                    // the inspector in Firestorm; Aurora opens the profile
                    // (history lines name no avatar)
                    let avatar = match (logged, own) {
                        (true, _) => uuid::Uuid::nil(),
                        (false, true) => world.agent_id,
                        (false, false) => line.source,
                    };
                    if !object && !avatar.is_nil() {
                        line_name_menu(&r, p, world, avatar, line);
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
    egui::Frame::new()
        .inner_margin(egui::Margin {
            left: 6,
            right: 2,
            top: 0,
            bottom: 0,
        })
        .show(ui, |ui| {
            chat_text_marked(
                ui,
                p,
                env.emoji,
                world,
                env.want_names,
                &text,
                13.5,
                tone.unwrap_or_else(|| super::colors::chat_color(line, world)),
                italics,
                mark,
            );
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
) -> ListEntry {
    // narrower beside the list's scroll bar, when it shows
    let width = ENTRY_W.min(ui.available_width());
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, LIST_ENTRY_H), egui::Sense::click());
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
            presence::paint_dot(ui.painter(), p, r, s, fill);
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
            button.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tip));
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
    open: &mut bool,
) -> Vec<ConvAction> {
    let mut actions = Vec::new();
    let default = default_rect(super::layout::work_area(ctx));
    if let Some(f) = world.social.focus_im.take() {
        st.open(f);
    }
    let docked = !contacts_settings.torn_off;
    if !show_contacts_tab && st.contacts {
        st.selected = None;
    }
    if !docked || !show_contacts_tab {
        st.contacts = false;
    }
    let title = match st.selected {
        _ if st.contacts => aurora_i18n::tr!("ui-chat-conversations-contacts").to_owned(),
        None => aurora_i18n::tr!("ui-chat-conversations-nearby-chat").to_owned(),
        Some(id) => aurora_i18n::tr!("ui-chat-conversations", value0 = world.session_title(&id).to_string()),
    };
    let own_name = world.own_name();
    let mut tear_off = false;
    let mut floater =
        super::widgets::Floater::new("conversations", title, default.min, default.size()).attention(conversations_waiting(world));
    if st.contacts {
        // the tear-off button of the multi-floater, for the Contacts tab
        floater = floater.action("external-link", aurora_i18n::tr!("common-detach"), &mut tear_off);
    }
    let style = st.demo_style.unwrap_or(st.style);
    floater.show(ctx, p, open, |ui| {
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
                docked,
                compact: style == crate::settings::ChatStyle::Compact,
            };
            workspace::show(ui, env, world, st, &mut actions);
            return;
        }
        let h = (ui.available_height() - 6.0).max(140.0);
        ui.horizontal_top(|ui| {
            // ---- contacts column
            ui.vertical(|ui| {
                ui.set_width(LIST_W);
                ui.spacing_mut().item_spacing.y = 2.0;
                let top = ui.cursor().top();
                // Contacts first (addFloater at START); once torn off the
                // entry brings its own window up (Firestorm has the toolbar
                // button for that)
                if show_contacts_tab
                    && list_entry(ui, p, aurora_i18n::tr!("common-contacts"), None, None, st.contacts, 0, None)
                        .resp
                        .clicked()
                {
                    if docked {
                        st.contacts = true;
                    } else {
                        actions.push(ConvAction::OpenContacts);
                    }
                }
                let local_shown = st.selected.is_none() && !st.contacts;
                if list_entry(
                    ui,
                    p,
                    aurora_i18n::tr!("common-nearby-chat"),
                    None,
                    None,
                    local_shown,
                    if local_shown { 0 } else { world.chat_unread },
                    None,
                )
                .resp
                .clicked()
                {
                    st.select_tab(None);
                }
                let sessions: Vec<(uuid::Uuid, usize)> = world.social.ims.iter().map(|s| (s.other, s.unread)).collect();
                // The conversations scroll under the pinned entries above, which
                // stay in reach (the vertical tabs of FSFloaterIMContainer
                // scroll with the arrows of LLTabContainer, all together).
                // (kept until its entry is there: the conversation may open
                // a frame before its first message lists it)
                let reveal = st.reveal;
                let left = h - (ui.cursor().top() - top);
                scrolling_list(ui, left, |ui| {
                    for (other, unread) in sessions {
                        let name = world.session_title(&other);
                        if !world.is_group_session(&other) {
                            st.wanted_pics.insert(other);
                        }
                        let entry = list_entry(
                            ui,
                            p,
                            &name,
                            Some((pics.get(&other), &name)),
                            (!world.is_group_session(&other)).then(|| presence::of(world, &other)),
                            st.selected == Some(other) && !st.contacts,
                            unread,
                            Some(close_tip(world, &other)),
                        );
                        if reveal == Some(other) {
                            entry.resp.scroll_to_me(None);
                            st.reveal = None;
                        }
                        if entry.close {
                            actions.push(ConvAction::Close(other));
                        } else if entry.resp.clicked() {
                            st.reveal = None;
                            st.select_tab(Some(other));
                        }
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
            ui.vertical(|ui| {
                // a resident dropped on it joins it (LLFloaterIMSession::
                // handleDragAndDrop, DAD_PERSON / DAD_CALLINGCARD)
                if let Some(conversation) = st.selected {
                    let pane = ui.available_rect_before_wrap();
                    let zone = ui.interact(pane, ui.id().with("person_drop"), egui::Sense::hover());
                    if zone.dnd_hover_payload::<super::call_invite::PersonDrag>().is_some() {
                        ui.painter().rect_stroke(
                            pane,
                            ui.visuals().window_corner_radius,
                            egui::Stroke::new(1.5, p.violet),
                            egui::StrokeKind::Inside,
                        );
                    }
                    if let Some(person) = zone.dnd_release_payload::<super::call_invite::PersonDrag>() {
                        actions.push(ConvAction::AddPeople(conversation, vec![person.0]));
                    }
                }
                // who the conversation is with, and its actions
                header::show(ui, p, icons, pics, world, st, &mut actions);
                ui.add_space(4.0);
                // the call of this conversation, while it is in progress
                if let Some(call) = st.selected.and_then(|s| world.call_of(&s)) {
                    call::strip(ui, p, icons, pics, world, call, st, &mut actions);
                    ui.add_space(4.0);
                } else if let Some(invite) = st.selected.and_then(|s| world.calls.invite_for(&s)) {
                    call::ringing(ui, p, world, invite, &mut actions);
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
                let input_h = INPUT_H;
                // Ctrl+F opens the find bar of the conversation shown, when
                // the keyboard is in this window, or nowhere with the
                // pointer over it
                // each conversation's own field (cursor, undo), like its draft
                let conv_input = egui::Id::new(("conv_input", st.selected));
                let search = st.searches.entry(st.selected).or_default();
                if find::wants_shortcut(ui, &[conv_input, find::field_id(), find::filter_id()]) {
                    search.open_find();
                }
                // exactly what the find bar takes under the lines (a row
                // that overflows makes the resizable window grow every frame)
                let find_h = if search.find.is_some() {
                    find::BAR_H + 4.0 + ui.spacing().item_spacing.y
                } else {
                    0.0
                };
                // exactly what the input row takes under them: the spacing after
                // the lines, the 4 points before the row and the row itself (a
                // row that overflows makes the resizable window grow every frame)
                let lines_h = (ui.available_height() - input_h - find_h - 4.0 - ui.spacing().item_spacing.y - 1.0).clamp(60.0, h);
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
                            own_name: &own_name,
                            height: lines_h,
                        };
                        to_field = conversation_lines(ui, p, lines, st, Rows::Classic(style));
                    });
                let search = st.searches.entry(st.selected).or_default();
                let view = st.views.entry(st.selected).or_default();
                match to_field {
                    ToField::Nothing => {}
                    ToField::Focus => st.conv_focus_request = true,
                    ToField::Text(t) => {
                        st.drafts.entry(st.selected).or_default().push_str(&t);
                        st.conv_focus_request = true;
                    }
                }
                if let Some(f) = search.find.as_mut() {
                    ui.add_space(4.0);
                    if find::bar(ui, p, f, view) {
                        search.find = None;
                        ui.ctx().memory_mut(|m| m.request_focus(conv_input));
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
                        }
                        // leave exactly the send button + spacing (else the row overflows
                        // and the resizable window grows every frame)
                        let field_w = (ui.available_width() - SEND_BTN_W - ui.spacing().item_spacing.x).floor().max(40.0);
                        let mut output = emoji.text_field(ui, &mut draft, |edit| {
                            edit.id(conv_input)
                                .hint_text(hint)
                                .desired_width(field_w)
                                .min_size(egui::vec2(field_w, INPUT_H))
                                .vertical_align(egui::Align::Center)
                        });
                        if st.selected.is_none() {
                            completion::complete(ui, &mut output, &mut draft, gestures);
                        }
                        let has_text = !draft.trim().is_empty();
                        send_clicked = send_button(ui, p, icons, has_text).clicked() && has_text;
                        output.response
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
                if st.emoji_open {
                    let anchor = emoji_btn.unwrap_or(resp.rect).left_top();
                    let mut close = false;
                    egui::Area::new(egui::Id::new("emoji_picker"))
                        .fade_in(false)
                        .order(egui::Order::Foreground)
                        .pivot(egui::Align2::LEFT_BOTTOM)
                        .fixed_pos(anchor - egui::vec2(0.0, 4.0))
                        .show(ui.ctx(), |ui| {
                            super::widgets::overlay(ui, p, egui::Margin::same(6), |ui| {
                                ui.set_width(300.0);
                                if let Some(c) = emoji.picker(ui, p, icons) {
                                    draft.push(c);
                                    close = !ui.input(|i| i.modifiers.shift);
                                }
                            });
                        });
                    if close {
                        st.emoji_open = false;
                        ui.ctx().memory_mut(|m| m.request_focus(conv_input));
                    }
                }
                if st.conv_focus_request {
                    st.conv_focus_request = false;
                    resp.request_focus();
                    cursor_to_end(ui.ctx(), conv_input, &draft);
                }
                let mut sent = false;
                if send_clicked
                    || ((resp.has_focus() || resp.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.alt))
                {
                    let text = draft.trim().to_owned();
                    if !text.is_empty() {
                        sent = true;
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
                    to: st
                        .selected
                        .filter(|id| !world.is_group_session(id) && (resp.has_focus() || sent)),
                    typed: resp.changed(),
                    empty: draft.trim().is_empty(),
                    sent,
                });
                if !draft.is_empty() {
                    *st.draft_mut(st.selected) = draft;
                } else {
                    st.drafts.remove(&st.selected);
                }
            });
        });
    });
    if tear_off {
        actions.push(ConvAction::Contacts(super::contacts::ContactsAction::ToggleTornOff));
    }
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
    let field = st.typing_field.take().filter(|_| st.send_typing && *open).unwrap_or(typing::Field {
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
        .data_mut(|d| d.get_persisted::<bool>(egui::Id::new(("conversations", "minimized"))))
        .unwrap_or(false);
    !st.contacts && st.selected.is_none() && !minimized
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
            if let Some(t) = super::icons::global("box") {
                painter.image(t.id(), r.shrink(1.0), uv, super::colors::sender_color(line).gamma_multiply(alpha));
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

/// Recent chat lines floating above the chat bar (fade out after 20 s);
/// names of linked avatars and pictures of the senders are asked through
/// `want_names` / `wanted_pics`.
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
) {
    let now = std::time::SystemTime::now();
    let recent: Vec<_> = world
        .chat
        .iter()
        .rev()
        .take(6)
        .filter(|l| now.duration_since(l.time).map(|d| d.as_secs_f32() < seconds).unwrap_or(false))
        .collect();
    if recent.is_empty() {
        return;
    }
    let mut links: Vec<(egui::Rect, Link, Color32)> = Vec::new();
    let mut names: Vec<(egui::Rect, &crate::world::ChatLine)> = Vec::new();
    egui::Area::new(egui::Id::new("chat_toasts"))
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
                        } = line_job(line, p, world, want_names, 540.0, size, times, family.as_ref());
                        for s in job.sections.iter_mut() {
                            s.format.color = s.format.color.gamma_multiply(alpha);
                        }
                        let galley = ui.ctx().fonts_mut(|f| f.layout_job(job));
                        let (rect, _) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
                        let painter = ui.painter();
                        painter.galley(rect.min, galley.clone(), p.ink);
                        // the sender's picture (or the object cube) in the room
                        // left before its name
                        if let Some((at, who)) = sender {
                            let side = pic_side(size);
                            let c = galley.pos_from_cursor(egui::text::CCursor::new(at)).translate(rect.min.to_vec2());
                            let r =
                                egui::Rect::from_center_size(egui::pos2(c.min.x - 2.0 - side * 0.5, c.center().y), egui::vec2(side, side));
                            bubble_sender(painter, p, pics, wanted_pics, r, &who, line, alpha);
                        }
                        // link badges over their invisible glyph
                        for (at, badge) in badges {
                            let Some(g) = range_rects(&galley, rect.min, at..at + 1).first().copied() else {
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
                            }
                        }
                        // the regions of the places shown, asked if unknown
                        for (_, link) in &line_links {
                            if let Link::Place(_, place) = link {
                                super::widgets::region_rating(ui.ctx(), &world.map.ratings, &place.location.region);
                            }
                        }
                        // the islands' pills: the sender's name acts like the
                        // conversations' (Firestorm's toast links it)
                        if pills
                            && let Some(range) = name
                            && let Some(r) = range_rects(&galley, rect.min, range).first().copied()
                        {
                            names.push((r, line));
                        }
                        let col = color_for(line.kind, p).gamma_multiply(alpha);
                        for (range, link) in line_links {
                            for r in range_rects(&galley, rect.min, range) {
                                links.push((r, link.clone(), col));
                            }
                        }
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

/// `AURORA_DEMO_BARS=chatname`: a click on a resident's name in the pills
/// at frame 700 (its profile opens).
pub fn demo_events(ctx: &egui::Context, frame: u64) -> Vec<egui::Event> {
    if !super::bars::demo::get().click_name || !(700..=701).contains(&frame) {
        return Vec::new();
    }
    let Some(r) = ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new("chat_toast_demo_name"))) else {
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
    let id = if line.kind == ChatKind::Own { world.agent_id } else { line.source };
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
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("chat_toast_demo_name"), rect));
    }
    egui::Area::new(egui::Id::new(("chat_toast_sender", k)))
        .fixed_pos(rect.min)
        .order(egui::Order::Background)
        .show(ctx, |ui| {
            let (_, resp) = ui.allocate_exact_size(rect.size(), egui::Sense::click());
            let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
            match target {
                SenderLink::Resident(id) => {
                    line_name_menu(&resp, p, world, id, line);
                    if resp.clicked() {
                        super::profile::request_open(ui.ctx(), id);
                    }
                }
                SenderLink::Object(id) => {
                    super::menu::context_menu(&resp, p, |ui| {
                        if super::menu::item(ui, p, "zoom-scan-filled", aurora_i18n::tr!("ui-context-inspect-2")) {
                            super::context::request(ui.ctx(), CtxAction::Inspect(id));
                        }
                        let (icon, label) = if world.mutes.is_muted_id(&id) {
                            ("circle-check-filled", aurora_i18n::tr!("common-stop-ignoring"))
                        } else {
                            ("ban", aurora_i18n::tr!("common-ignore"))
                        };
                        if super::menu::item(ui, p, icon, label) {
                            super::context::request(
                                ui.ctx(),
                                CtxAction::ToggleBlockObject {
                                    id,
                                    name: line.from.clone(),
                                },
                            );
                        }
                    });
                    if resp.clicked() {
                        super::context::request(ui.ctx(), CtxAction::Inspect(id));
                    }
                }
            }
        });
}

/// Screen rects of a char range of a laid-out text, one per row.
fn range_rects(galley: &egui::Galley, origin: egui::Pos2, range: std::ops::Range<usize>) -> Vec<egui::Rect> {
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
    egui::Area::new(egui::Id::new(("chat_toast_link", k)))
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
                                CtxAction::TeleportToPlace(l.region.clone(), l.pos)
                            } else {
                                CtxAction::ShowPlaceInfo(l.region.clone(), l.pos)
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
            }
        });
}

/// The "Nearby chat" input bar; returns a message to send.
pub fn bar(ui: &mut egui::Ui, p: &Palette, st: &mut ChatUi, gestures: &crate::gestures::Manager, width: f32) -> Option<OutgoingChat> {
    ui.label(RichText::new(aurora_i18n::tr!("common-nearby-chat")).size(12.0).color(p.muted));
    bar_input(ui, st, gestures, None, |edit| edit.desired_width(width))
}

/// `build`, then the chat field's id (the app gives it the keyboard).
fn identified(build: impl FnOnce(egui::TextEdit<'_>) -> egui::TextEdit<'_>) -> impl FnOnce(egui::TextEdit<'_>) -> egui::TextEdit<'_> {
    move |edit| build(edit.id(egui::Id::new("nearby_chat_input")))
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
    st: &mut ChatUi,
    gestures: &crate::gestures::Manager,
    emoji: Option<&mut super::emoji::Emoji>,
    build: impl FnOnce(egui::TextEdit<'_>) -> egui::TextEdit<'_>,
) -> Option<OutgoingChat> {
    let mut out = None;
    // the shortcuts (Enter, Ctrl / Shift + Enter, /channel) are a tooltip:
    // written in the empty field they crowded the bar (the human's request)
    let build = identified(build);
    let mut output = match emoji {
        Some(emoji) => emoji.text_field(ui, &mut st.input, build),
        None => build(egui::TextEdit::singleline(&mut st.input)).show(ui),
    };
    completion::complete(ui, &mut output, &mut st.input, gestures);
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
        cursor_to_end(ui.ctx(), resp.id, &st.input);
    }
    st.has_focus = resp.has_focus();
    // Modified Enter does not surrender egui's single-line field focus.
    let enter = !focusing && (resp.has_focus() || resp.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.alt);
    if enter {
        resp.surrender_focus();
    }
    if enter && !st.input.trim().is_empty() {
        let (shout, whisper) = ui.input(|i| (i.modifiers.ctrl, i.modifiers.shift));
        let (channel, message) = parse_channel(st.input.trim());
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
    }
    // Enter on an empty bar returns to movement (like Firestorm).
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One frame of a notification text with place links: the regions
    /// whose rating is not known are the ones asked, each once.
    #[test]
    fn place_links_ask_only_unknown_regions() {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let p = crate::theme::Theme::default().palette();
        let mut ratings = crate::world::worldmap::RegionRatings::default();
        ratings.learn("Aurora Démo", aurora_net::search::SIM_ACCESS_PG);
        let text = "Ici http://maps.secondlife.com/secondlife/Aurora%20D%C3%A9mo/1/2/3, là secondlife:///app/region/Pin%C3%A8de/4/5/6 \
                    et secondlife://Pinède/7/8/9";
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0))),
                ..Default::default()
            },
            |ui| text_with_places(ui, &p, &ratings, text, 12.0, p.ink),
        );
        output.textures_delta.clear();
        assert_eq!(super::super::widgets::take_wanted_regions(&ctx), vec!["Pinède".to_owned()]);
        // taken: nothing left until the next frame draws them again
        assert!(super::super::widgets::take_wanted_regions(&ctx).is_empty());
    }

    #[test]
    fn agent_links() {
        let id = uuid::Uuid::from_u128(5);
        let t = format!("voir secondlife:///app/agent/{id}/about, @ secondlife:///app/agent/{id}/mention");
        let segs = segments(&t);
        assert_eq!(segs[1], Seg::Agent(id));
        assert_eq!(segs[3], Seg::Mention(id));
        let bad = segments("secondlife:///app/agent/nope/about");
        assert!(matches!(bad[0], Seg::Slurl(_)));
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
    fn bubbles_by_default_legacy_lines_on_demand() {
        use crate::settings::ChatStyle;
        assert_eq!(crate::settings::Settings::default().chat_style, ChatStyle::Bubbles);
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
        let job = |l: &crate::world::ChatLine, pill| line_job(l, &p, &world, &mut HashSet::new(), 540.0, 12.0, false, pill).job;
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
        assert_eq!(sender_link(&world, &line(ChatKind::Own, 0)), Some(SenderLink::Resident(id(1))));
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
                    )
                },
            );
            out.textures_delta.clear();
        };
        frame(Vec::new());
        frame(Vec::new());
        let name = ctx
            .memory(|m| m.area_rect(egui::Id::new(("chat_toast_sender", 0usize))))
            .expect("the resident's name is clickable");
        // one name only: the notice has none
        assert!(ctx.memory(|m| m.area_rect(egui::Id::new(("chat_toast_sender", 1usize)))).is_none());
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
    }

    use crate::ui::widgets::elide_tests::{Frames, LONG, SHORT, WIDE, painted, tooltip_shown};

    /// A tab chosen in the list gives its field the keyboard; what is typed
    /// stays with its conversation when another tab is shown, and goes when
    /// the conversation closes.
    #[test]
    fn each_tab_keeps_its_draft_and_takes_the_keyboard() {
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
        let mut st = ChatUi::default();
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
                    &mut open,
                );
            });
        };
        let field = |c: Option<uuid::Uuid>| egui::Id::new(("conv_input", c));
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
        let mut st = ChatUi::default();
        let mut contacts = crate::ui::contacts::ContactsUi::default();
        let mut contacts_settings = crate::ui::contacts::ContactsSettings::default();
        let mut open = true;
        let ctx = frames.ctx.clone();
        let mut frame = |st: &mut ChatUi, world: &mut World| {
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
                    &mut open,
                );
            });
        };
        let field = |c: uuid::Uuid| egui::Id::new(("conv_input", Some(c)));
        // opened by a call: shown, the keyboard stays nowhere
        world.social.focus_im = Some(a);
        frame(&mut st, &mut world);
        frame(&mut st, &mut world);
        assert_eq!(st.selected, Some(a));
        assert!(ctx.memory(|m| m.focused()).is_none());
        // opened by the user: shown and ready to type in
        world.social.focus_im = Some(b);
        st.conv_focus_request = true;
        frame(&mut st, &mut world);
        frame(&mut st, &mut world);
        assert_eq!(st.selected, Some(b));
        assert!(ctx.memory(|m| m.has_focus(field(b))));
    }

    /// A drag across the lines selects their text without moving the
    /// window; with the keyboard on them Ctrl+A selects the conversation,
    /// a copy gives it with the senders, and what is typed goes to the field.
    #[test]
    fn the_lines_select_and_copy_like_a_transcript() {
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
        let mut st = ChatUi::default();
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
            frame(&mut st, vec![egui::Event::PointerMoved(at)]);
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
        assert!(ctx.memory(|m| m.has_focus(egui::Id::new(("conv_input", None::<uuid::Uuid>)))));
        // a click without a drag clears the selection
        let at = egui::pos2(second.center().x, second.center().y);
        frame(&mut st, vec![egui::Event::PointerMoved(at), button(at, true)]);
        frame(&mut st, vec![button(at, false)]);
        assert!(st.selection.1.is_empty());
        // from the empty dark area under a short conversation up to its
        // first line: the window does not move, every line is taken
        let (texts, _) = frame(&mut st, Vec::new());
        let last = painted(&texts, "bye now").rect;
        let empty = egui::pos2(last.center().x, last.bottom() + 80.0);
        frame(&mut st, vec![egui::Event::PointerMoved(empty), button(empty, true)]);
        for k in 1..=4 {
            let at = empty + (from - empty) * (k as f32 / 4.0);
            frame(&mut st, vec![egui::Event::PointerMoved(at)]);
        }
        let (texts, _) = frame(&mut st, vec![button(from, false)]);
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
            .read_response(egui::Id::new(("conv_input", None::<uuid::Uuid>)))
            .expect("the conversation's field")
            .rect;
        let strip = egui::pos2(field.center().x, field.top() - 2.0);
        let (_, out) = frame(&mut st, vec![egui::Event::PointerMoved(strip)]);
        assert_ne!(out.cursor_icon, egui::CursorIcon::Text, "a normal cursor on the strip");
        let (_, out) = frame(&mut st, vec![egui::Event::PointerMoved(from)]);
        assert_eq!(out.cursor_icon, egui::CursorIcon::Text, "the text cursor on the lines");
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
        let mut frames = Frames::new();
        let p = crate::theme::Theme::default().palette();
        let icons = crate::ui::icons::Icons::load(&frames.ctx, None);
        let mut emoji = crate::ui::emoji::Emoji::load();
        let pics = HashMap::new();
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let gestures = crate::gestures::Manager::default();
        let mut st = ChatUi::default();
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
        list_entry(ui, &p, label, pic.then_some((None, label)), None, sel, unread, close)
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
