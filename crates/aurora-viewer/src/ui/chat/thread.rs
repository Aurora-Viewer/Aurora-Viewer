//! The messages of a conversation in Aurora's workspace, as a desktop chat
//! shows them (the human's choice of 2026-10-15): left
//! aligned and grouped by author.
//!
//! - Roomy (`Aurora`): the author's picture in a 56 px gutter, their name,
//!   tags and time once at the head of a group, then the lines; the time of
//!   a following line shows in the gutter under the pointer.
//! - Compact: one row per message, `time · name · text`, no pictures.
//!
//! Both: the row under the pointer is tinted, without buttons (the human's
//! choice of 2026-10-16): a right click on a message copies or quotes it
//! and holds its author's menu (`message_menu`), a click on the picture or
//! the name opens the profile; a message that names us is marked with an
//! amber bar; an emote is in italics, a shout and a
//! whisper say so with a tag; the conversation's days and its "New" line
//! are separators; notices are discreet rows with an icon; invitation cards
//! stay in the thread. The texts are the classic lines' (`chat_text_marked`:
//! links, places, mentions, emoji, find highlights, selection).

use super::{LineEnv, Mark, chat_text_marked, hhmm, history_tone, line_name_menu, sender_name, view};
use crate::theme::Palette;
use crate::world::chat_log::format::Slt;
use crate::world::{ChatKind, ChatLine, Origin};
use aurora_net::ChatType;
use egui::{Color32, RichText};
use std::collections::HashSet;

/// The gutter of the pictures (roomy) and their side.
pub(super) const GUTTER: f32 = 56.0;
const AVATAR: f32 = 36.0;
/// The time column of the compact rows.
const TIME_W: f32 = 40.0;
const TEXT: f32 = 13.5;
/// Room kept at the right of a row.
const RIGHT: f32 = 14.0;
/// Space above the head of a group (roomy) or a row (compact).
const GROUP_GAP: f32 = 12.0;
const ROW_GAP: f32 = 2.0;

/// What drawing the workspace's rows needs besides `LineEnv`.
pub(super) struct Thread<'a> {
    pub compact: bool,
    /// The times are shown (Preferences › Chat).
    pub times: bool,
    /// The moderators of a group or conference: their name says MOD.
    pub moderators: Option<&'a HashSet<uuid::Uuid>>,
    /// Our names, lowercase, whose mention marks a message.
    pub me: &'a [String],
    /// What the rows asked of the conversation.
    pub out: &'a mut Vec<ThreadAction>,
}

/// What a message's menu asks of the conversation.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum ThreadAction {
    /// "> text" for the field.
    Quote(String),
}

/// A row of the thread: its day, its "New" line, then the message.
pub(super) fn draw(
    ui: &mut egui::Ui,
    env: &mut LineEnv,
    th: &mut Thread,
    line: &ChatLine,
    look: view::Look,
    mark: Option<Mark>,
    painted: bool,
) {
    let p = env.p;
    ui.spacing_mut().item_spacing.y = 0.0;
    if let Some(day) = look.marker {
        day_separator(ui, p, &super::bubbles::day_text(day));
    }
    if look.new {
        new_separator(ui, p);
    }
    let indent = if th.compact { TIME_W + 8.0 } else { GUTTER };
    if let ChatKind::Invite(invite) = line.kind {
        super::invite_card::draw_card(ui, env, line, invite, indent);
        return;
    }
    if let ChatKind::InventoryOffer(offer) = line.kind {
        super::offer_card::draw_card(ui, env, line, offer, indent);
        return;
    }
    if let ChatKind::Payment(payment) = line.kind {
        super::payment_card::draw_card(ui, env, line, payment, indent);
        return;
    }
    if line.kind == ChatKind::System {
        notice(ui, env, th, line, mark);
        return;
    }
    // (the notices above are compact rows already)
    if super::object_row::compact_in(line, env) {
        // the cube under the pictures, or after the compact rows' times
        let place = if th.compact {
            super::object_row::Place {
                icon_center: 0.0,
                text_left: 0.0,
                time_column: th.times.then_some(TIME_W),
            }
        } else {
            super::object_row::Place {
                icon_center: GUTTER * 0.5,
                text_left: GUTTER,
                time_column: None,
            }
        };
        if look.first {
            ui.add_space(5.0);
        }
        super::object_row::draw(ui, env, line, mark, place, th.times);
        return;
    }
    let head = look.header;
    if head && !look.first && look.marker.is_none() && !look.new {
        ui.add_space(if th.compact { ROW_GAP } else { GROUP_GAP });
    } else if look.first {
        ui.add_space(6.0);
    }
    let left = ui.max_rect().left();
    let width = ui.available_width();
    let top = ui.cursor().top();
    let behind = ui.painter().add(egui::Shape::Noop);
    let name = sender_name(env.world, env.own_name, line);
    let who = Who::of(env, line);
    let rect = if th.compact {
        compact_row(ui, env, th, line, &name, &who, mark)
    } else {
        roomy_row(ui, env, th, line, head, &name, &who, mark, width)
    };
    // (the current match's marker: this message, not the lines above it)
    super::find::note_message(ui, top, Some(rect.bottom()));
    let rect = egui::Rect::from_min_max(egui::pos2(left, top), egui::pos2(left + width, rect.bottom() + ROW_GAP));
    ui.add_space(ROW_GAP);
    if !painted {
        return;
    }
    let mentioned = mentions(env, th.me, line);
    // (not while a selection is dragged across the rows)
    let hovered = ui.rect_contains_pointer(rect) && !ui.input(|i| i.pointer.is_decidedly_dragging());
    let radius = ui.visuals().window_corner_radius;
    if mentioned {
        ui.painter()
            .set(behind, egui::epaint::RectShape::filled(rect, 0.0, p.amber.gamma_multiply(0.07)));
        ui.painter()
            .rect_filled(egui::Rect::from_min_size(rect.min, egui::vec2(2.0, rect.height())), 0.0, p.amber);
    } else if hovered {
        ui.painter()
            .set(behind, egui::epaint::RectShape::filled(rect, radius, p.raised.gamma_multiply(0.35)));
    }
    if hovered && !th.compact && !head {
        // the time of a following line, in the gutter
        let t = hhmm(line.time);
        if th.times && !t.is_empty() {
            ui.painter().text(
                egui::pos2(left + GUTTER * 0.5, rect.top() + 11.0),
                egui::Align2::CENTER_CENTER,
                t,
                egui::FontId::proportional(10.0),
                p.muted_dim,
            );
        }
    }
}

/// Who wrote a line, for its picture, its menu and its profile.
struct Who {
    /// The avatar to look up (none for an object, nor for a history line
    /// whose sender's name was not cached).
    avatar: uuid::Uuid,
    object: bool,
    logged: bool,
    /// A member of the Aurora team or of Linden Lab (`world::staff`).
    mark: Option<crate::world::staff::Mark>,
}

impl Who {
    fn of(env: &LineEnv, line: &ChatLine) -> Who {
        let logged = line.origin != Origin::Live;
        let object = matches!(line.kind, ChatKind::Object(_) | ChatKind::ObjectIm);
        let avatar = if object {
            uuid::Uuid::nil()
        } else if line.kind.is_own() {
            env.world.agent_id
        } else {
            line.source
        };
        let mark = (!object).then(|| crate::world::staff::mark(env.world, &avatar)).flatten();
        Who {
            avatar,
            object,
            logged,
            mark,
        }
    }
}

/// The colour of an author's name: ours in the theme's teal, a moderator's
/// in light violet, an object's in the colour of objects, a history line's
/// greyed.
fn name_color(env: &LineEnv, th: &Thread, line: &ChatLine, who: &Who) -> Color32 {
    let p = env.p;
    if let Some(color) = crate::ui::colors::contact_sender_color(line, env.world) {
        return color;
    }
    if let Some(tone) = history_tone(p, line.origin) {
        return tone;
    }
    if who.object {
        return crate::ui::colors::sender_color(line);
    }
    if line.kind.is_own() {
        return p.teal;
    }
    if th.moderators.is_some_and(|m| m.contains(&line.source)) {
        return p.violet_light;
    }
    p.ink
}

/// The text of a message and how it reads: an emote is the name in violet
/// italics, a whisper in grey italics; the user's chat colours otherwise.
fn body(env: &LineEnv, line: &ChatLine, name: &str) -> (String, Color32, bool) {
    let emote = line.text.starts_with("/me ") || line.text.starts_with("/me'");
    let text = if emote {
        format!("{name}{}", &line.text[3..])
    } else {
        line.text.clone()
    };
    (text, super::message_color(line, env.p, env.world), emote)
}

/// The tags after a name: MOD and OBJECT on a pill, SHOUTS and WHISPERS
/// as words (the mockup's `.tag` and `.vol`).
fn tags(ui: &mut egui::Ui, p: &Palette, th: &Thread, line: &ChatLine, who: &Who) {
    if !who.object {
        // the size of the pills below
        crate::ui::staff::after_name(ui, p, who.mark, 9.5);
    }
    if who.object {
        tag(ui, aurora_i18n::tr!("ui-chat-tag-object"), Some(p.raised), p.muted);
    } else if th.moderators.is_some_and(|m| m.contains(&line.source)) {
        tag(
            ui,
            aurora_i18n::tr!("ui-chat-tag-moderator"),
            Some(p.violet.gamma_multiply(0.22)),
            p.violet_pale,
        );
    }
    match line.kind.volume() {
        Some(ChatType::Shout) => tag(ui, aurora_i18n::tr!("ui-chat-tag-shouts"), None, p.warn),
        Some(ChatType::Whisper) => tag(ui, aurora_i18n::tr!("ui-chat-tag-whispers"), None, p.muted),
        _ => {}
    }
}

/// A tag: small semi-bold capitals, on a pill of `fill` when given (also
/// the cards of the nearby chat lines, `console.rs`).
pub(super) fn tag(ui: &mut egui::Ui, text: &str, fill: Option<Color32>, color: Color32) {
    let font = egui::FontId::new(if fill.is_some() { 9.5 } else { 10.0 }, crate::ui::fonts::semibold(ui.ctx()));
    let galley = ui.painter().layout_no_wrap(text.to_uppercase(), font, color);
    let pad = if fill.is_some() { 5.0 } else { 0.0 };
    let size = egui::vec2(galley.size().x + 2.0 * pad, 15.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    if let Some(fill) = fill {
        ui.painter().rect_filled(rect, crate::ui::widgets::inner_radius(ui.visuals()), fill);
    }
    crate::ui::widgets::paint_row_text(ui.painter(), galley, rect.left() + pad, rect.center().y, color);
}

/// The name of a head or a compact row: clickable to the profile, with the
/// author's menu on a right click.
fn name_label(ui: &mut egui::Ui, env: &LineEnv, line: &ChatLine, name: &str, color: Color32, who: &Who, max: f32) {
    let text = RichText::new(name)
        .font(egui::FontId::new(13.5, crate::ui::fonts::semibold(ui.ctx())))
        .color(color);
    let r = ui.add(crate::ui::widgets::elided_label(ui, text, max).sense(egui::Sense::click()));
    if who.avatar.is_nil() {
        return;
    }
    line_name_menu(&r, env.p, env.world, who.avatar, line, env.menu);
    if r.on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(aurora_i18n::tr!("common-view-profile"))
        .clicked()
    {
        crate::ui::profile::request_open(ui.ctx(), who.avatar);
    }
}

/// "today at 10:30", "yesterday at 22:41", "3 October 2026 at 09:12".
fn when(line: &ChatLine) -> String {
    let time = hhmm(line.time);
    if time.is_empty() {
        return time;
    }
    let now = std::time::SystemTime::now();
    let day = Slt::of(line.time);
    let date = super::bubbles::day_text((day.year, day.month, day.day));
    let yesterday = Slt::of(now - std::time::Duration::from_secs(86_400));
    match super::bubbles::day_label((day.year, day.month, day.day), &Slt::of(now), &yesterday) {
        super::bubbles::DayLabel::Today => aurora_i18n::tr!("ui-chat-today-at", time = time.as_str()).to_owned(),
        super::bubbles::DayLabel::Yesterday => aurora_i18n::tr!("ui-chat-yesterday-at", time = time.as_str()).to_owned(),
        super::bubbles::DayLabel::Date(..) => aurora_i18n::tr!("ui-chat-date-at", date = date.as_str(), time = time.as_str()).to_owned(),
    }
}

/// A roomy row: the picture and the head for the first line of a group,
/// the text alone for the others. Returns its rect.
#[allow(clippy::too_many_arguments)]
fn roomy_row(
    ui: &mut egui::Ui,
    env: &mut LineEnv,
    th: &Thread,
    line: &ChatLine,
    head: bool,
    name: &str,
    who: &Who,
    mark: Option<Mark>,
    width: f32,
) -> egui::Rect {
    let p = env.p;
    let text_w = (width - GUTTER - RIGHT).max(60.0);
    let row = ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let (gutter, _) = ui.allocate_exact_size(egui::vec2(GUTTER, if head { AVATAR } else { 1.0 }), egui::Sense::hover());
        if head {
            let pic = egui::Rect::from_min_size(
                egui::pos2(gutter.center().x - AVATAR * 0.5, gutter.top() + 2.0),
                egui::Vec2::splat(AVATAR),
            );
            picture(ui, env, line, name, who, pic);
        }
        ui.vertical(|ui| {
            ui.set_width(text_w);
            ui.spacing_mut().item_spacing.y = 1.0;
            if head {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 7.0;
                    let color = name_color(env, th, line, who);
                    name_label(ui, env, line, name, color, who, text_w * 0.6);
                    tags(ui, p, th, line, who);
                    let time = when(line);
                    if th.times && !time.is_empty() {
                        ui.label(RichText::new(time).size(11.0).color(p.muted_dim));
                    }
                });
            }
            let (text, color, italics) = body(env, line, name);
            let visible = super::previews::text(env.previews.as_deref(), line, &text, ui.available_width());
            if !visible.is_empty() {
                let size = env.emoji.message_size(&visible, TEXT);
                chat_text_marked(
                    ui,
                    p,
                    env.emoji,
                    env.world,
                    env.want_names,
                    &visible,
                    size,
                    color,
                    super::message_emphasis(line.kind, italics),
                    mark,
                );
            }
            super::previews::cards(ui, env, line, &text);
        });
    });
    row.response.rect
}

/// A compact row: the time, the name and its tags, then the text, which
/// wraps under itself.
#[allow(clippy::too_many_arguments)]
fn compact_row(
    ui: &mut egui::Ui,
    env: &mut LineEnv,
    th: &Thread,
    line: &ChatLine,
    name: &str,
    who: &Who,
    mark: Option<Mark>,
) -> egui::Rect {
    let p = env.p;
    let row = ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.add_space(4.0);
        if th.times {
            let (r, _) = ui.allocate_exact_size(egui::vec2(TIME_W, 20.0), egui::Sense::hover());
            ui.painter().text(
                egui::pos2(r.right(), r.top() + 10.5),
                egui::Align2::RIGHT_CENTER,
                hhmm(line.time),
                egui::FontId::proportional(10.5),
                p.muted_dim,
            );
        }
        let emote = line.text.starts_with("/me ") || line.text.starts_with("/me'");
        if !emote {
            ui.add_space(0.0);
            let color = name_color(env, th, line, who);
            let max = (ui.available_width() * 0.35).max(60.0);
            ui.allocate_ui_with_layout(egui::vec2(max, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                name_label(ui, env, line, name, color, who, max);
                tags(ui, p, th, line, who);
            });
        }
        let (text, color, italics) = body(env, line, name);
        let visible = super::previews::text(env.previews.as_deref(), line, &text, ui.available_width() - RIGHT);
        ui.vertical(|ui| {
            ui.set_width((ui.available_width() - RIGHT).max(40.0));
            ui.add_space(1.0);
            if !visible.is_empty() {
                let size = env.emoji.message_size(&visible, TEXT);
                chat_text_marked(
                    ui,
                    p,
                    env.emoji,
                    env.world,
                    env.want_names,
                    &visible,
                    size,
                    color,
                    super::message_emphasis(line.kind, italics),
                    mark,
                );
            }
            super::previews::cards(ui, env, line, &text);
        });
    });
    row.response.rect
}

/// The picture at the head of a group: the avatar's, a box for an object,
/// initials for a history line; clickable to the profile. No status dot:
/// the head and the column already show it (the human's choice of
/// 2026-10-16), and a dot on every group of a busy chat said it again.
fn picture(ui: &mut egui::Ui, env: &mut LineEnv, line: &ChatLine, name: &str, who: &Who, rect: egui::Rect) {
    let p = env.p;
    if who.object {
        ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.raised);
        let (glyph, tint) = super::object_row::object_glyph(line, p, crate::ui::colors::sender_color(line));
        paint_icon(ui.painter(), glyph, rect.shrink(9.0), tint);
        return;
    }
    if !who.avatar.is_nil() {
        env.wanted.insert(who.avatar);
    }
    crate::ui::widgets::avatar_disc(ui, p, rect, env.pics.get(&who.avatar), name, who.logged);
    if who.avatar.is_nil() {
        return;
    }
    let r = ui.interact(
        rect,
        ui.scope_id().with(("thread_pic", line.time, line.source)),
        egui::Sense::click(),
    );
    line_name_menu(&r, p, env.world, who.avatar, line, env.menu);
    if r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(name).clicked() {
        crate::ui::profile::request_open(ui.ctx(), who.avatar);
    }
}

/// A Tabler icon in a rect, in a colour.
pub(super) fn paint_icon(painter: &egui::Painter, icon: &str, rect: egui::Rect, color: Color32) {
    // a name that is not built in would paint nothing (a blank button)
    debug_assert!(crate::ui::icons::embedded(icon), "icon {icon:?} is not embedded");
    if let Some(t) = crate::ui::icons::global(icon) {
        painter.image(
            t.id(),
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            color,
        );
    }
}

/// A notice of the viewer (someone online, a call, an automatic response):
/// a discreet row with an icon.
fn notice(ui: &mut egui::Ui, env: &mut LineEnv, th: &Thread, line: &ChatLine, mark: Option<Mark>) {
    let p = env.p;
    let tone = history_tone(p, line.origin).unwrap_or(p.muted);
    ui.add_space(4.0);
    let top = ui.cursor().top();
    let row = ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let lead = if th.compact {
            4.0 + if th.times { TIME_W + 6.0 } else { 0.0 }
        } else {
            GUTTER
        };
        let (r, _) = ui.allocate_exact_size(egui::vec2(lead, 18.0), egui::Sense::hover());
        let icon = if th.compact {
            egui::Rect::from_min_size(egui::pos2(r.right() - 14.0, r.top() + 2.0), egui::Vec2::splat(14.0))
        } else {
            egui::Rect::from_center_size(egui::pos2(r.center().x, r.top() + 9.0), egui::Vec2::splat(14.0))
        };
        // what it is about, with #741's option (`object_row::icon`)
        let name = if env.compact_notices {
            super::object_row::icon(env.world, line)
        } else {
            "info-circle-filled"
        };
        paint_icon(ui.painter(), name, icon, p.muted_dim);
        ui.vertical(|ui| {
            ui.set_width((ui.available_width() - RIGHT).max(40.0));
            ui.add_space(1.0);
            chat_text_marked(ui, p, env.emoji, env.world, env.want_names, &line.text, 12.0, tone, false, mark);
        });
    });
    super::find::note_message(ui, top, Some(row.response.rect.bottom()));
    ui.add_space(4.0);
}

/// The line between two days, with the day in its middle.
fn day_separator(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.add_space(8.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 18.0), egui::Sense::hover());
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), egui::FontId::proportional(11.0), p.muted_dim);
    let half = galley.size().x * 0.5 + 8.0;
    let y = rect.center().y;
    let stroke = egui::Stroke::new(1.0, p.raised.gamma_multiply(0.8));
    let (l, r) = (rect.left() + 16.0, rect.right() - 16.0);
    if rect.center().x - half > l {
        ui.painter().hline(l..=rect.center().x - half, y, stroke);
        ui.painter().hline(rect.center().x + half..=r, y, stroke);
    }
    let left = rect.center().x - galley.size().x * 0.5;
    crate::ui::widgets::paint_row_text(ui.painter(), galley, left, y, p.muted_dim);
    ui.add_space(4.0);
}

/// The line above the first message that was unread when the conversation
/// opened: "NEW" at its right end, in the colour of the unread counts.
fn new_separator(ui: &mut egui::Ui, p: &Palette) {
    ui.add_space(6.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 14.0), egui::Sense::hover());
    let galley = ui.painter().layout_no_wrap(
        aurora_i18n::tr!("ui-chat-new-messages").to_uppercase(),
        egui::FontId::proportional(10.0),
        p.danger,
    );
    let y = rect.center().y;
    let text_left = rect.right() - 16.0 - galley.size().x;
    ui.painter().hline(
        rect.left() + 16.0..=text_left - 8.0,
        y,
        egui::Stroke::new(1.0, p.danger.gamma_multiply(0.6)),
    );
    // capitals only: their middle on the line
    let font = egui::FontId::proportional(10.0);
    let baseline = crate::ui::widgets::centered_baseline(ui.painter(), font, y);
    crate::ui::widgets::paint_on_baseline(ui.painter(), galley, text_left, baseline, p.danger);
    ui.add_space(2.0);
}

/// Actions for the menu's retained message, even after its row leaves the
/// viewport or its line is evicted: copy, quote, then its author's menu.
pub(super) fn message_menu(ui: &mut egui::Ui, env: &LineEnv, th: &mut Thread, target: &super::line_menu::Target) {
    let line = &target.line;
    let p = env.p;
    let name = sender_name(env.world, env.own_name, line);
    let who = Who::of(env, line);
    if crate::ui::menu::item(ui, p, "copy-filled", aurora_i18n::tr!("ui-chat-copy-message")) {
        // LLTextEditor::copy (indra/llui/lltexteditor.cpp): a transcript's
        // selected text takes priority over the message under the menu.
        let text = target.selected_text.clone().unwrap_or_else(|| {
            let prefix = super::copy_prefix(line, &name);
            format!("{prefix}{}", super::shown_text(env.world, line, &name))
        });
        ui.ctx().copy_text(text);
    }
    if crate::ui::menu::item(ui, p, "quote-filled", aurora_i18n::tr!("ui-chat-quote-message")) {
        let (text, _, _) = body(env, line, &name);
        // the field holds one line: the quote, then the answer after it
        let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
        th.out.push(ThreadAction::Quote(format!("> {one_line} ")));
    }
    if !who.avatar.is_nil() {
        crate::ui::menu::separator(ui, p);
        crate::ui::context::chat_sender_menu(ui, p, env.world, who.avatar, line);
    }
}

/// The message names us: an explicit mention, or our display or user name
/// as a word of its text.
fn mentions(env: &LineEnv, me: &[String], line: &ChatLine) -> bool {
    if matches!(line.kind, ChatKind::Own(_) | ChatKind::System) || line.kind.is_card() || line.origin != Origin::Live {
        return false;
    }
    let agent = env.world.agent_id;
    if super::segments(&line.text)
        .iter()
        .any(|s| matches!(s, super::Seg::Mention(id) if *id == agent))
    {
        return true;
    }
    names_in(&line.text, me)
}

/// One of `names` (lowercase) stands as a word of `text`.
pub(super) fn names_in(text: &str, names: &[String]) -> bool {
    names.iter().filter(|n| n.chars().count() >= 3).any(|name| {
        view::find_ci(text, name).into_iter().any(|m| {
            let before = text[..m.start].chars().next_back();
            let after = text[m.end..].chars().next();
            !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn our_name_counts_as_a_word_only() {
        let me = vec!["nova".to_owned(), "nova.exemple".to_owned()];
        assert!(names_in("hey Nova, ready?", &me));
        assert!(names_in("NOVA!", &me));
        assert!(names_in("ask nova.exemple", &me));
        assert!(!names_in("novan wine", &me));
        assert!(!names_in("nobody here", &me));
        // too short a name never marks
        assert!(!names_in("an ox", &["ox".to_owned()]));
    }
}
