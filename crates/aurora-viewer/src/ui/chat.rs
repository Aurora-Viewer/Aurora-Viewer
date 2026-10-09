//! Local chat window (nearby chat + instant messages).

use super::context::CtxAction;
use crate::theme::Palette;
use crate::world::{ChatKind, World};
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
    /// The « Contacts » tab is shown instead (FSFloaterContacts docked as
    /// the first tab of FSFloaterIMContainer).
    pub contacts: bool,
    pub conv_input: String,
    /// Avatars whose profile picture the conversation wants (filled while drawing).
    pub wanted_pics: HashSet<uuid::Uuid>,
    /// Names to resolve (mentions) — taken by the app.
    pub wanted_names: HashSet<uuid::Uuid>,
    /// Participants of the group / conference session shown.
    pub show_profile: bool,
    /// Search filter (None = closed).
    pub search: Option<String>,
    pub emoji_open: bool,
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
        ChatKind::System => p.muted,
        ChatKind::Im => p.violet_light,
    }
}

pub(crate) fn hhmm(t: std::time::SystemTime) -> String {
    let secs = t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    // Local time is not available without a tz database; show UTC.
    format!("{:02}:{:02}", (secs / 3600) % 24, (secs / 60) % 60)
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
        Trust::Trusted => ("check-circle-fill", p.success, "Site de confiance".into()),
        Trust::Unknown => (
            "warning-fill",
            p.amber,
            "Lien externe : vérifiez l'adresse avant de l'ouvrir".into(),
        ),
        Trust::Dangerous(why) => ("x-circle-fill", p.danger, format!("Lien dangereux : {why}")),
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
/// of each web link's badge, and the char where the sender's name starts
/// (room for its picture before it).
struct BubbleJob {
    job: egui::text::LayoutJob,
    links: Vec<(std::ops::Range<usize>, Link)>,
    badges: Vec<(usize, String)>,
    sender: Option<(usize, Sender)>,
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
    let name_fmt = egui::TextFormat {
        color: col,
        font_id: egui::FontId::proportional(size),
        ..Default::default()
    };
    let text_fmt = egui::TextFormat {
        color: col,
        font_id: egui::FontId::proportional(size),
        italics: emote,
        ..Default::default()
    };
    // (before the name, after it, body)
    let (before, after, body) = match line.kind {
        ChatKind::System => ("", "", &line.text[..]),
        _ if emote => ("", "", &line.text[3..]),
        ChatKind::Im => ("[IM] ", ": ", &line.text[..]),
        ChatKind::Local(ChatType::Shout) => ("", " crie : ", &line.text[..]),
        ChatKind::Local(ChatType::Whisper) => ("", " murmure : ", &line.text[..]),
        _ => ("", ": ", &line.text[..]),
    };
    let mut sender = None;
    if line.kind != ChatKind::System {
        job.append(before, 0.0, name_fmt.clone());
        let who = match line.kind {
            ChatKind::Object(_) | ChatKind::ObjectIm => Some(Sender::Object),
            _ if !line.source.is_nil() => Some(Sender::Avatar(line.source)),
            _ => None,
        };
        // room for the picture, painted over it once laid out
        let lead = if who.is_some() { pic_side(size) + 4.0 } else { 0.0 };
        sender = who.map(|w| (job.text.chars().count(), w));
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
    for (piece, link) in pieces(body, name_of) {
        if let Some(Link::Url(u)) = &link {
            job.append("M", 0.0, badge_fmt.clone());
            badges.push((at, u.clone()));
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
        job.append(&piece, 0.0, fmt);
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
    /// From the « Contacts » tab.
    Contacts(super::contacts::ContactsAction),
    /// Show the torn-off Contacts window (its entry in the list).
    OpenContacts,
    /// Block / unblock the avatar of a 1:1 conversation.
    ToggleBlock(uuid::Uuid),
    /// Leave a group / conference session.
    LeaveSession(uuid::Uuid),
    /// Stop receiving a group's chat (and leave its session).
    BlockGroupChat(uuid::Uuid),
    /// Open an avatar's profile window.
    Profile(uuid::Uuid),
}

/// Messages from the same sender within this many seconds share one header.
const GROUP_SECONDS: u64 = 300;

fn initials(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase()
}

/// Profile picture (or initials) of an avatar, `size` px square.
fn avatar_pic(ui: &mut egui::Ui, p: &Palette, pic: Option<&egui::TextureHandle>, name: &str, size: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    match pic {
        Some(t) => {
            ui.painter().image(
                t.id(),
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => {
            ui.painter().rect_filled(rect, 2.0, p.raised);
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                initials(name),
                egui::FontId::proportional(size * 0.45),
                p.violet_pale,
            );
        }
    }
}

/// Phosphor icon button of the conversation toolbar.
fn tool(ui: &mut egui::Ui, p: &Palette, icons: &super::icons::Icons, icon: &str, tip: &str, enabled: bool, active: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(26.0, 22.0), egui::Sense::click());
    if active {
        ui.painter().rect_filled(rect, 2.0, p.violet.gamma_multiply(0.3));
    } else if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, 2.0, p.raised);
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
            if super::menu::item(ui, p, "navigation-arrow", "Se téléporter à cet emplacement") {
                super::context::request(ui.ctx(), CtxAction::TeleportToPlace(l.region.clone(), l.pos));
            }
        };
        let map_label = if place.teleport {
            "Afficher sur la carte"
        } else {
            "Voir sur la carte"
        };
        let map = |ui: &mut egui::Ui| {
            if super::menu::item(ui, p, "map-trifold", map_label) {
                super::context::request(ui.ctx(), CtxAction::ShowPlace(l.region.clone(), l.pos));
            }
        };
        if place.teleport {
            teleport(ui);
            super::menu::separator(ui, p);
            map(ui);
        } else {
            if super::menu::item(ui, p, "info", "Afficher les informations sur ce lieu") {
                super::context::request(ui.ctx(), CtxAction::ShowPlaceInfo(l.region.clone(), l.pos));
            }
            super::menu::separator(ui, p);
            map(ui);
            teleport(ui);
        }
        super::menu::separator(ui, p);
        if super::menu::item(ui, p, "link", "Copier la SLurl") {
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
    let segs = segments(text);
    if segs.iter().all(|s| matches!(s, Seg::Text(_))) {
        emoji.rich_text(ui, text, size, color, italics);
        return;
    }
    let k = super::colors::get();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(0.0, 1.0);
        for s in segs {
            match s {
                Seg::Text(t) => emoji.inline(ui, t, size, color, italics),
                Seg::Url(u) => {
                    link_badge(ui, p, u, size);
                    let r = ui.add(egui::Link::new(RichText::new(u).size(size).color(super::colors::c(k.chat_urls))));
                    if r.on_hover_text(u).clicked() {
                        open_web_link(ui.ctx(), u);
                    }
                }
                Seg::Slurl(u) => {
                    ui.add(egui::Label::new(RichText::new(u).size(size).color(super::colors::c(k.chat_slurl))).selectable(true))
                        .on_hover_text(u);
                }
                Seg::Place(u, place) => {
                    let r = ui.add(egui::Link::new(
                        RichText::new(&place.label).size(size).color(super::colors::c(k.chat_slurl)),
                    ));
                    place_menu(&r, p, u, &place);
                    // TooltipTeleportUrl / TooltipSLURL; a place opens its
                    // details (LLURLDispatcherImpl::regionHandleCallback,
                    // SLURLTeleportDirectly off: FSFloaterPlaceDetails)
                    let tip = if place.teleport {
                        "Cliquez pour vous téléporter à cet endroit"
                    } else {
                        "Cliquez pour en savoir plus sur cet endroit"
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
                Seg::Mention(id) => {
                    want_names.insert(id);
                    let me = id == world.agent_id;
                    let bg = if me { k.mention_me } else { k.mention_residents };
                    let label = format!(" @{} ", world.social.name_of(&id));
                    let r = ui.add(
                        egui::Label::new(
                            RichText::new(label)
                                .size(size)
                                .color(super::colors::c(k.mention_text))
                                .background_color(super::colors::c(bg)),
                        )
                        .sense(egui::Sense::click()),
                    );
                    name_menu(&r, p, world, id);
                    if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        super::profile::request_open(ui.ctx(), id);
                    }
                }
                Seg::Agent(id) => {
                    want_names.insert(id);
                    let r = ui.add(
                        egui::Label::new(
                            RichText::new(world.social.name_of(&id))
                                .size(size)
                                .color(super::colors::c(k.chat_slurl)),
                        )
                        .sense(egui::Sense::click()),
                    );
                    name_menu(&r, p, world, id);
                    if r.on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text("Voir le profil")
                        .clicked()
                    {
                        super::profile::request_open(ui.ctx(), id);
                    }
                }
            }
        }
    });
}

/// Icon button returning its response (emoji button next to the input).
/// Width of the toolbar buttons with a response (emoji).
const EMOJI_BTN_W: f32 = 28.0;

fn tool_resp(ui: &mut egui::Ui, p: &Palette, icons: &super::icons::Icons, icon: &str, active: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(EMOJI_BTN_W, 22.0), egui::Sense::click());
    if active {
        ui.painter().rect_filled(rect, 2.0, p.violet.gamma_multiply(0.3));
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 2.0, p.raised);
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

/// Messages grouped by sender: header strip (picture, name, time) then lines.
#[allow(clippy::too_many_arguments)]
fn conversation(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &super::icons::Icons,
    emoji: &mut super::emoji::Emoji,
    pics: &HashMap<uuid::Uuid, egui::TextureHandle>,
    world: &World,
    want_names: &mut HashSet<uuid::Uuid>,
    lines: &[&crate::world::ChatLine],
    wanted: &mut HashSet<uuid::Uuid>,
    own_name: &str,
) {
    ui.spacing_mut().item_spacing.y = 2.0;
    let w = ui.available_width();
    let mut prev: Option<(uuid::Uuid, std::time::SystemTime, bool)> = None;
    for line in lines {
        if line.kind == ChatKind::System {
            chat_text(
                ui,
                p,
                emoji,
                world,
                want_names,
                &line.text,
                12.5,
                super::colors::chat_color(line, world),
                true,
            );
            prev = None;
            continue;
        }
        let own = line.kind == ChatKind::Own;
        let object = matches!(line.kind, ChatKind::Object(_) | ChatKind::ObjectIm);
        let new_group = match prev {
            Some((src, t, o)) => {
                src != line.source || o != own || line.time.duration_since(t).map(|d| d.as_secs() > GROUP_SECONDS).unwrap_or(true)
            }
            None => true,
        };
        // FSChatHistoryHeader: the display name, then " - username" in small
        let names = &world.social.avatar_names;
        let cached = if object {
            None
        } else {
            names.get(if own { &world.agent_id } else { &line.source })
        };
        let name = match cached {
            Some(n) => n.display(&names.options),
            None if own => own_name.to_owned(),
            None => line.from.clone(),
        };
        if new_group {
            ui.add_space(6.0);
            let username = cached.and_then(|n| n.chat_username(&names.options));
            egui::Frame::new()
                .fill(p.raised.gamma_multiply(0.55))
                .corner_radius(egui::CornerRadius::same(2))
                .inner_margin(egui::Margin::symmetric(4, 3))
                .show(ui, |ui| {
                    ui.set_width(w - 8.0);
                    ui.horizontal(|ui| {
                        if object {
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
                            if let Some(t) = icons.get("cube") {
                                ui.painter().image(
                                    t.id(),
                                    rect.shrink(2.0),
                                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                    super::colors::sender_color(line),
                                );
                            }
                        } else {
                            if !line.source.is_nil() {
                                wanted.insert(line.source);
                            }
                            avatar_pic(ui, p, pics.get(&line.source), &name, 20.0);
                        }
                        let col = super::colors::sender_color(line);
                        let r = ui.add(egui::Label::new(RichText::new(&name).size(13.0).strong().color(col)).sense(egui::Sense::click()));
                        // the inspector in Firestorm; Aurora opens the profile
                        let avatar = if own { world.agent_id } else { line.source };
                        if !object && !avatar.is_nil() {
                            name_menu(&r, p, world, avatar);
                        }
                        if !object
                            && !avatar.is_nil()
                            && r.on_hover_cursor(egui::CursorIcon::PointingHand)
                                .on_hover_text("Voir le profil")
                                .clicked()
                        {
                            super::profile::request_open(ui.ctx(), avatar);
                        }
                        if let Some(u) = &username {
                            ui.label(RichText::new(format!("- {u}")).size(11.5).color(p.indigo_light));
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new(hhmm(line.time)).size(11.5).color(p.muted));
                        });
                    });
                });
            ui.add_space(2.0);
        }
        prev = Some((line.source, line.time, own));
        let emote = line.text.starts_with("/me ") || line.text.starts_with("/me'");
        let (text, italics) = if emote {
            (format!("{}{}", name, &line.text[3..]), true)
        } else {
            let prefix = match line.kind {
                ChatKind::Local(ChatType::Shout) => "(crie) ",
                ChatKind::Local(ChatType::Whisper) => "(murmure) ",
                _ => "",
            };
            (format!("{prefix}{}", line.text), false)
        };
        egui::Frame::new()
            .inner_margin(egui::Margin {
                left: 6,
                right: 2,
                top: 0,
                bottom: 0,
            })
            .show(ui, |ui| {
                chat_text(
                    ui,
                    p,
                    emoji,
                    world,
                    want_names,
                    &text,
                    13.5,
                    super::colors::chat_color(line, world),
                    italics,
                );
            });
    }
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
    st: &mut ChatUi,
    contacts: &mut super::contacts::ContactsUi,
    contacts_settings: &mut super::contacts::ContactsSettings,
    open: &mut bool,
) -> Vec<ConvAction> {
    let mut actions = Vec::new();
    let screen = ctx.content_rect();
    if let Some(f) = world.social.focus_im.take() {
        st.selected = Some(f);
        st.show_profile = false;
        st.contacts = false;
    }
    let docked = !contacts_settings.torn_off;
    if !docked {
        st.contacts = false;
    }
    let title = match st.selected {
        _ if st.contacts => "Conversations - Contacts".to_owned(),
        None => "Conversations - Chat local".to_owned(),
        Some(id) => format!("Conversations - {}", world.session_title(&id)),
    };
    let own_name = world.own_name();
    let mut tear_off = false;
    let mut floater = super::widgets::Floater::new(
        "conversations",
        title,
        egui::pos2(screen.left() + 8.0, screen.bottom() - 400.0),
        egui::vec2(600.0, 360.0),
    );
    if st.contacts {
        // the tear-off button of the multi-floater, for the Contacts tab
        floater = floater.action("arrow-square-out", "Détacher", &mut tear_off);
    }
    floater.show(ctx, p, open, |ui| {
        let h = (ui.available_height() - 6.0).max(140.0);
        ui.horizontal_top(|ui| {
            // ---- contacts column
            ui.vertical(|ui| {
                ui.set_width(140.0);
                ui.spacing_mut().item_spacing.y = 2.0;
                let entry =
                    |ui: &mut egui::Ui, label: &str, pic: Option<(Option<&egui::TextureHandle>, &str)>, sel: bool, unread: usize| -> bool {
                        let (rect, resp) = ui.allocate_exact_size(egui::vec2(136.0, 24.0), egui::Sense::click());
                        let fill = if sel {
                            p.raised
                        } else if resp.hovered() {
                            p.field.gamma_multiply(1.4)
                        } else {
                            p.field
                        };
                        ui.painter().rect_filled(rect, 1.0, fill);
                        if sel {
                            ui.painter()
                                .rect_filled(egui::Rect::from_min_size(rect.min, egui::vec2(2.0, rect.height())), 0.0, p.violet);
                        }
                        let mut x = rect.left() + 8.0;
                        if let Some((tex, name)) = pic {
                            let r = egui::Rect::from_min_size(egui::pos2(x, rect.center().y - 8.0), egui::vec2(16.0, 16.0));
                            match tex {
                                Some(t) => {
                                    ui.painter().image(
                                        t.id(),
                                        r,
                                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                        Color32::WHITE,
                                    );
                                }
                                None => {
                                    ui.painter().rect_filled(r, 2.0, p.raised);
                                    ui.painter().text(
                                        r.center(),
                                        egui::Align2::CENTER_CENTER,
                                        initials(name),
                                        egui::FontId::proportional(7.5),
                                        p.violet_pale,
                                    );
                                }
                            }
                            x += 22.0;
                        }
                        ui.painter().text(
                            egui::pos2(x, rect.center().y),
                            egui::Align2::LEFT_CENTER,
                            label,
                            egui::FontId::proportional(12.0),
                            if sel { p.ink } else { p.muted },
                        );
                        if unread > 0 {
                            ui.painter()
                                .circle_filled(rect.right_center() - egui::vec2(10.0, 0.0), 4.0, p.violet);
                        }
                        resp.clicked()
                    };
                // Contacts first (addFloater at START); once torn off the
                // entry brings its own window up (Firestorm has the toolbar
                // button for that)
                if entry(ui, "Contacts", None, st.contacts, 0) {
                    if docked {
                        st.contacts = true;
                    } else {
                        actions.push(ConvAction::OpenContacts);
                    }
                }
                let local_shown = st.selected.is_none() && !st.contacts;
                if entry(ui, "Chat local", None, local_shown, if local_shown { 0 } else { world.chat_unread }) {
                    st.selected = None;
                    st.show_profile = false;
                    st.contacts = false;
                }
                let sessions: Vec<(uuid::Uuid, usize)> = world.social.ims.iter().map(|s| (s.other, s.unread)).collect();
                for (other, unread) in sessions {
                    let name = world.session_title(&other);
                    if !world.is_group_session(&other) {
                        st.wanted_pics.insert(other);
                    }
                    if entry(
                        ui,
                        &name,
                        Some((pics.get(&other), &name)),
                        st.selected == Some(other) && !st.contacts,
                        unread,
                    ) {
                        st.selected = Some(other);
                        st.show_profile = false;
                        st.contacts = false;
                    }
                }
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
                // toolbar
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    match st.selected {
                        Some(other) if world.is_group_session(&other) => {
                            // group / conference: participants, block the group's chat, leave
                            let n = world.groups.sessions.get(&other).map(|s| s.participants.len()).unwrap_or(0);
                            if tool(ui, p, icons, "users-three", &format!("Participants ({n})"), true, st.show_profile) {
                                st.show_profile = !st.show_profile;
                            }
                            if world.groups.is_member(&other)
                                && tool(
                                    ui,
                                    p,
                                    icons,
                                    "chat-teardrop-slash",
                                    "Ne plus recevoir le chat de ce groupe",
                                    true,
                                    false,
                                )
                            {
                                actions.push(ConvAction::BlockGroupChat(other));
                            }
                            if tool(ui, p, icons, "sign-out", "Quitter la session", true, false) {
                                actions.push(ConvAction::LeaveSession(other));
                            }
                        }
                        Some(other) => {
                            if tool(ui, p, icons, "user-circle", "Profil", true, false) {
                                actions.push(ConvAction::Profile(other));
                            }
                            tool(ui, p, icons, "user-plus", "Ajouter en ami (à venir)", false, false);
                            if tool(ui, p, icons, "airplane-takeoff", "Proposer une téléportation", true, false) {
                                actions.push(ConvAction::OfferTeleport(other));
                            }
                            tool(ui, p, icons, "gift", "Donner un objet (à venir)", false, false);
                            tool(ui, p, icons, "currency-circle-dollar", "Payer (à venir)", false, false);
                            tool(ui, p, icons, "phone", "Appel vocal (à venir)", false, false);
                            let blocked = world.is_avatar_blocked(&other);
                            let tip = if blocked { "Débloquer" } else { "Bloquer" };
                            if tool(ui, p, icons, "prohibit", tip, true, blocked) {
                                actions.push(ConvAction::ToggleBlock(other));
                            }
                        }
                        None => {
                            if tool(ui, p, icons, "users", "Personnes à proximité", true, false) {
                                actions.push(ConvAction::OpenPeople(0));
                            }
                        }
                    }
                    let searching = st.search.is_some();
                    if tool(ui, p, icons, "magnifying-glass", "Rechercher dans la conversation", true, searching) {
                        st.search = if searching { None } else { Some(String::new()) };
                    }
                    if let Some(q) = st.search.as_mut() {
                        ui.add(egui::TextEdit::singleline(q).hint_text("Rechercher…").desired_width(160.0));
                    }
                });
                ui.add_space(2.0);
                // profile card
                if let (true, Some(Some(info))) = (st.show_profile, st.selected.map(|s| world.groups.sessions.get(&s))) {
                    // participants of a group / conference session
                    egui::Frame::new()
                        .fill(p.field)
                        .corner_radius(egui::CornerRadius::same(2))
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
                                ui.label(RichText::new("Participants en cours de chargement…").size(11.5).color(p.muted));
                            }
                            egui::ScrollArea::vertical().max_height(90.0).show(ui, |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    for (id, n, moderator) in names {
                                        let t = RichText::new(n).size(12.0).color(if moderator { p.violet_light } else { p.ink });
                                        let r = ui.add(egui::Label::new(t).sense(egui::Sense::click()));
                                        name_menu(&r, p, world, id);
                                        let tip = if moderator {
                                            "Modérateur · clic : profil"
                                        } else {
                                            "Clic : profil"
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
                let input_h = 30.0;
                let lines_h = (ui.available_height() - input_h - 8.0).clamp(60.0, h);
                let query = st.search.as_ref().map(|q| q.to_lowercase()).filter(|q| !q.is_empty());
                let keep = |l: &&crate::world::ChatLine| {
                    query
                        .as_ref()
                        .is_none_or(|q| l.text.to_lowercase().contains(q) || l.from.to_lowercase().contains(q))
                };
                egui::Frame::new()
                    .fill(p.field)
                    .corner_radius(egui::CornerRadius::same(1))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        egui::ScrollArea::vertical()
                            .id_salt(("conv", st.selected))
                            .stick_to_bottom(true)
                            .auto_shrink([false, false])
                            .max_height(lines_h)
                            .min_scrolled_height(lines_h)
                            .show(ui, |ui| match st.selected {
                                None => {
                                    world.chat_unread = 0;
                                    let w: &World = world;
                                    let lines: Vec<&crate::world::ChatLine> = w.chat.iter().filter(keep).collect();
                                    conversation(
                                        ui,
                                        p,
                                        icons,
                                        emoji,
                                        pics,
                                        w,
                                        &mut st.wanted_names,
                                        &lines,
                                        &mut st.wanted_pics,
                                        &own_name,
                                    );
                                }
                                Some(id) => {
                                    if let Some(s) = world.social.ims.iter_mut().find(|s| s.other == id) {
                                        s.unread = 0;
                                    }
                                    let w: &World = world;
                                    if let Some(s) = w.social.ims.iter().find(|s| s.other == id) {
                                        let lines: Vec<&crate::world::ChatLine> = s.lines.iter().filter(keep).collect();
                                        conversation(
                                            ui,
                                            p,
                                            icons,
                                            emoji,
                                            pics,
                                            w,
                                            &mut st.wanted_names,
                                            &lines,
                                            &mut st.wanted_pics,
                                            &own_name,
                                        );
                                    }
                                }
                            });
                    });
                ui.add_space(4.0);
                // input + emoji
                let hint = match st.selected {
                    None => "Au chat local".to_owned(),
                    Some(id) => format!("À {}", world.session_title(&id)),
                };
                let mut emoji_btn = None;
                let resp = ui
                    .horizontal(|ui| {
                        // leave exactly the emoji button + spacing (else the row overflows
                        // and the resizable window grows every frame)
                        let field_w = (ui.available_width() - EMOJI_BTN_W - ui.spacing().item_spacing.x - 2.0).max(40.0);
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut st.conv_input)
                                .id(egui::Id::new("conv_input"))
                                .hint_text(hint)
                                .desired_width(field_w),
                        );
                        let b = tool_resp(ui, p, icons, "smiley", st.emoji_open);
                        emoji_btn = Some(b.rect);
                        if b.on_hover_text("Emoji").clicked() {
                            st.emoji_open = !st.emoji_open;
                        }
                        r
                    })
                    .inner;
                if st.emoji_open {
                    let anchor = emoji_btn.unwrap_or(resp.rect).right_top();
                    let mut close = false;
                    egui::Area::new(egui::Id::new("emoji_picker"))
                        .fade_in(false)
                        .order(egui::Order::Foreground)
                        .pivot(egui::Align2::RIGHT_BOTTOM)
                        .fixed_pos(anchor - egui::vec2(0.0, 4.0))
                        .show(ui.ctx(), |ui| {
                            egui::Frame::new()
                                .fill(p.panel)
                                .stroke(egui::Stroke::new(1.0, p.raised))
                                .corner_radius(egui::CornerRadius::same(3))
                                .inner_margin(egui::Margin::same(6))
                                .show(ui, |ui| {
                                    ui.set_width(300.0);
                                    if let Some(c) = emoji.picker(ui, p, icons) {
                                        st.conv_input.push(c);
                                        close = !ui.input(|i| i.modifiers.shift);
                                    }
                                });
                        });
                    if close {
                        st.emoji_open = false;
                        ui.ctx().memory_mut(|m| m.request_focus(egui::Id::new("conv_input")));
                    }
                }
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let text = st.conv_input.trim().to_owned();
                    if !text.is_empty() {
                        match st.selected {
                            None => {
                                let (channel, message) = parse_channel(&text);
                                actions.push(ConvAction::Local(OutgoingChat {
                                    message,
                                    channel,
                                    // Ctrl+Entrée crie, Maj+Entrée murmure (comme la barre de chat)
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
                    st.conv_input.clear();
                    resp.request_focus();
                }
            });
        });
    });
    if tear_off {
        actions.push(ConvAction::Contacts(super::contacts::ContactsAction::ToggleTornOff));
    }
    actions
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
            if let Some(t) = super::icons::global("cube") {
                painter.image(t.id(), r.shrink(1.0), uv, super::colors::sender_color(line).gamma_multiply(alpha));
            }
        }
        Sender::Avatar(id) => {
            wanted_pics.insert(*id);
            match pics.get(id) {
                Some(t) => {
                    painter.image(t.id(), r, uv, Color32::WHITE.gamma_multiply(alpha));
                }
                None => {
                    painter.rect_filled(r, 2.0, p.raised.gamma_multiply(alpha));
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
    egui::Area::new(egui::Id::new("chat_toasts"))
        .anchor(
            egui::Align2::LEFT_BOTTOM,
            egui::vec2(8.0, -(ctx.content_rect().bottom() - bottom) - 6.0),
        )
        .interactable(false)
        .order(egui::Order::Background)
        .show(ctx, |ui| {
            ui.set_max_width(560.0);
            for line in recent.into_iter().rev() {
                let age = now.duration_since(line.time).map(|d| d.as_secs_f32()).unwrap_or(0.0);
                let alpha = ((seconds - age) / 3.0).clamp(0.0, 1.0);
                egui::Frame::new()
                    .fill(Color32::from_rgba_unmultiplied(
                        p.bar.r(),
                        p.bar.g(),
                        p.bar.b(),
                        (190.0 * alpha) as u8,
                    ))
                    .corner_radius(egui::CornerRadius::same(3))
                    .inner_margin(egui::Margin::symmetric(6, 2))
                    .show(ui, |ui| {
                        let size = 13.0;
                        let BubbleJob {
                            mut job,
                            links: line_links,
                            badges,
                            sender,
                        } = line_job(line, p, world, want_names, 540.0, size, times);
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
                        // web link badges over their invisible glyph
                        for (at, url) in badges {
                            if let Some(g) = range_rects(&galley, rect.min, at..at + 1).first() {
                                let b = egui::Rect::from_center_size(g.center(), egui::vec2(size, size));
                                paint_link_badge(painter, p, b, &url, alpha);
                            }
                        }
                        let col = color_for(line.kind, p).gamma_multiply(alpha);
                        for (range, link) in line_links {
                            for r in range_rects(&galley, rect.min, range) {
                                links.push((r, link.clone(), col));
                            }
                        }
                    });
                ui.add_space(2.0);
            }
        });
    for (k, (rect, link, col)) in links.iter().enumerate() {
        bubble_link(ctx, p, world, k, *rect, link, *col);
    }
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
                        "Cliquez pour vous téléporter à cet endroit"
                    } else {
                        "Cliquez pour en savoir plus sur cet endroit"
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
                    if resp.on_hover_text("Voir le profil").clicked() {
                        super::profile::request_open(ui.ctx(), *id);
                    }
                }
            }
        });
}

/// The "Chat local" input bar; returns a message to send.
pub fn bar(ui: &mut egui::Ui, p: &Palette, st: &mut ChatUi, width: f32) -> Option<OutgoingChat> {
    let mut out = None;
    ui.label(RichText::new("Chat local").size(12.0).color(p.muted));
    let resp = ui.add(
        egui::TextEdit::singleline(&mut st.input)
            .hint_text("Entrée pour parler · Ctrl+Entrée crier · Maj+Entrée murmurer · /5 canal")
            .desired_width(width),
    );
    if st.focus_request {
        resp.request_focus();
        st.focus_request = false;
    }
    st.has_focus = resp.has_focus();
    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
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
    fn channels() {
        assert_eq!(parse_channel("/5 hello"), (5, "hello".into()));
        assert_eq!(parse_channel("hello"), (0, "hello".into()));
        assert_eq!(parse_channel("/me waves"), (0, "/me waves".into()));
        assert_eq!(parse_channel("/-1 x"), (-1, "x".into()));
    }
}
