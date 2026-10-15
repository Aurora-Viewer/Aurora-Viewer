//! The messages of a conversation in Aurora's workspace, as a desktop chat
//! shows them (the human's choice of 2026-10-15, after Discord): left
//! aligned and grouped by author.
//!
//! - Roomy (`Aurora`): the author's picture in a 56 px gutter, their name,
//!   tags and time once at the head of a group, then the lines; the time of
//!   a following line shows in the gutter under the pointer.
//! - Compact: one row per message, `time · name · text`, no pictures.
//!
//! Both: the row under the pointer is tinted and carries a toolbar (copy,
//! quote, the author's profile, the author's menu); a message that names us
//! is marked with an amber bar; an emote is in italics, a shout and a
//! whisper say so with a tag; the conversation's days and its "New" line
//! are separators; notices are discreet rows with an icon; invitation cards
//! stay in the thread. The texts are the classic lines' (`chat_text_marked`:
//! links, places, mentions, emoji, find highlights, selection).

use super::{LineEnv, Mark, chat_text_marked, hhmm, history_tone, line_name_menu, presence, sender_name, view};
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
/// Room kept at the right of a row (its toolbar sits there under the
/// pointer).
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

/// What a row's toolbar asks of the conversation.
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
    if let ChatKind::Invite(invite) = line.kind {
        super::invite_card::draw_card(ui, env, line, invite, if th.compact { TIME_W + 8.0 } else { GUTTER });
        return;
    }
    if line.kind == ChatKind::System {
        notice(ui, env, th, line, mark);
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
        compact_row(ui, env, th, line, &name, &who, mark, width)
    } else {
        roomy_row(ui, env, th, line, head, &name, &who, mark, width)
    };
    let rect = egui::Rect::from_min_max(egui::pos2(left, top), egui::pos2(left + width, rect.bottom() + ROW_GAP));
    ui.add_space(ROW_GAP);
    if !painted {
        return;
    }
    let mentioned = mentions(env, th.me, line);
    let hovered = ui.rect_contains_pointer(rect) && !ui.input(|i| i.pointer.any_down());
    let radius = ui.visuals().window_corner_radius;
    if mentioned {
        ui.painter().set(behind, egui::epaint::RectShape::filled(rect, 0.0, p.amber.gamma_multiply(0.07)));
        ui.painter().rect_filled(
            egui::Rect::from_min_size(rect.min, egui::vec2(2.0, rect.height())),
            0.0,
            p.amber,
        );
    } else if hovered {
        ui.painter().set(behind, egui::epaint::RectShape::filled(rect, radius, p.raised.gamma_multiply(0.35)));
    }
    if hovered {
        if !th.compact && !head {
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
        toolbar(ui, env, th, line, &name, &who, rect);
    }
}

/// Who wrote a line, for its picture, its menu and its profile.
struct Who {
    /// The avatar to look up (none for an object or a history line).
    avatar: uuid::Uuid,
    object: bool,
    logged: bool,
}

impl Who {
    fn of(env: &LineEnv, line: &ChatLine) -> Who {
        let logged = line.origin != Origin::Live;
        let object = matches!(line.kind, ChatKind::Object(_) | ChatKind::ObjectIm);
        let avatar = if logged || object {
            uuid::Uuid::nil()
        } else if line.kind == ChatKind::Own {
            env.world.agent_id
        } else {
            line.source
        };
        Who { avatar, object, logged }
    }
}

/// The colour of an author's name: ours in the theme's teal, a moderator's
/// in light violet, an object's in the colour of objects, a history line's
/// greyed.
fn name_color(env: &LineEnv, th: &Thread, line: &ChatLine, who: &Who) -> Color32 {
    let p = env.p;
    if let Some(tone) = history_tone(p, line.origin) {
        return tone;
    }
    if who.object {
        return crate::ui::colors::sender_color(line);
    }
    if line.kind == ChatKind::Own {
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
    let p = env.p;
    let emote = line.text.starts_with("/me ") || line.text.starts_with("/me'");
    let tone = history_tone(p, line.origin);
    if emote {
        return (format!("{name}{}", &line.text[3..]), tone.unwrap_or(p.violet_pale), true);
    }
    let whisper = matches!(line.kind, ChatKind::Local(ChatType::Whisper));
    let color = tone.unwrap_or_else(|| {
        if whisper {
            p.muted
        } else {
            crate::ui::colors::chat_color(line, env.world)
        }
    });
    (line.text.clone(), color, whisper)
}

/// The tags after a name: MOD, OBJECT, SHOUTS, WHISPERS.
fn tags(ui: &mut egui::Ui, p: &Palette, th: &Thread, line: &ChatLine, who: &Who) {
    if who.object {
        tag(ui, p, aurora_i18n::tr!("ui-chat-tag-object"), p.raised, p.muted);
    } else if th.moderators.is_some_and(|m| m.contains(&line.source)) {
        tag(ui, p, aurora_i18n::tr!("ui-chat-tag-moderator"), p.violet.gamma_multiply(0.22), p.violet_pale);
    }
    match line.kind {
        ChatKind::Local(ChatType::Shout) => tag(ui, p, aurora_i18n::tr!("ui-chat-tag-shouts"), p.warn.gamma_multiply(0.16), p.warn),
        ChatKind::Local(ChatType::Whisper) => tag(ui, p, aurora_i18n::tr!("ui-chat-tag-whispers"), p.raised, p.muted),
        _ => {}
    }
}

fn tag(ui: &mut egui::Ui, _p: &Palette, text: &str, fill: Color32, color: Color32) {
    egui::Frame::new()
        .fill(fill)
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(egui::Margin::symmetric(4, 0))
        .show(ui, |ui| {
            ui.label(RichText::new(text.to_uppercase()).size(9.5).strong().color(color));
        });
}

/// The name of a head or a compact row: clickable to the profile, with the
/// author's menu on a right click.
fn name_label(ui: &mut egui::Ui, env: &LineEnv, line: &ChatLine, name: &str, color: Color32, who: &Who, max: f32) {
    let text = RichText::new(name).size(13.0).strong().color(color);
    let r = ui.add(crate::ui::widgets::elided_label(ui, text, max).sense(egui::Sense::click()));
    if who.avatar.is_nil() {
        return;
    }
    line_name_menu(&r, env.p, env.world, who.avatar, line);
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
    let yesterday = Slt::of(now - std::time::Duration::from_secs(86_400));
    match super::bubbles::day_label((day.year, day.month, day.day), &Slt::of(now), &yesterday) {
        super::bubbles::DayLabel::Today => aurora_i18n::tr!("ui-chat-today-at", time = time.as_str()).to_owned(),
        super::bubbles::DayLabel::Yesterday => aurora_i18n::tr!("ui-chat-yesterday-at", time = time.as_str()).to_owned(),
        super::bubbles::DayLabel::Date(..) => aurora_i18n::tr!(
            "ui-chat-date-at",
            date = super::bubbles::day_text((day.year, day.month, day.day)).as_str(),
            time = time.as_str()
        )
        .to_owned(),
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
            let pic = egui::Rect::from_min_size(egui::pos2(gutter.center().x - AVATAR * 0.5, gutter.top() + 2.0), egui::Vec2::splat(AVATAR));
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
            chat_text_marked(ui, p, env.emoji, env.world, env.want_names, &text, TEXT, color, italics, mark);
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
    width: f32,
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
                egui::FontId::monospace(10.5),
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
        ui.vertical(|ui| {
            ui.set_width((ui.available_width() - RIGHT).max(40.0));
            ui.add_space(1.0);
            chat_text_marked(ui, p, env.emoji, env.world, env.want_names, &text, TEXT, color, italics, mark);
        });
    });
    row.response.rect
}

/// The picture at the head of a group: the avatar's with its status, a box
/// for an object, initials for a history line; clickable to the profile.
fn picture(ui: &mut egui::Ui, env: &mut LineEnv, line: &ChatLine, name: &str, who: &Who, rect: egui::Rect) {
    let p = env.p;
    if who.object {
        ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.raised);
        paint_icon(ui.painter(), "box", rect.shrink(9.0), crate::ui::colors::sender_color(line));
        return;
    }
    if !who.avatar.is_nil() {
        env.wanted.insert(who.avatar);
    }
    let pic = env.pics.get(&who.avatar).filter(|_| !who.logged);
    crate::ui::widgets::avatar_disc(ui, p, rect, pic, name, who.logged);
    if who.avatar.is_nil() {
        return;
    }
    let status = presence::of(env.world, &who.avatar);
    if status != presence::Status::Unknown {
        presence::paint_dot(ui.painter(), p, rect, status, p.field);
    }
    let r = ui.interact(rect, ui.id().with(("thread_pic", line.time, line.source)), egui::Sense::click());
    line_name_menu(&r, p, env.world, who.avatar, line);
    if r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(name).clicked() {
        crate::ui::profile::request_open(ui.ctx(), who.avatar);
    }
}

/// A Tabler icon in a rect, in a colour.
pub(super) fn paint_icon(painter: &egui::Painter, icon: &str, rect: egui::Rect, color: Color32) {
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
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let lead = if th.compact { 4.0 + if th.times { TIME_W + 6.0 } else { 0.0 } } else { GUTTER };
        let (r, _) = ui.allocate_exact_size(egui::vec2(lead, 18.0), egui::Sense::hover());
        let icon = if th.compact {
            egui::Rect::from_min_size(egui::pos2(r.right() - 14.0, r.top() + 2.0), egui::Vec2::splat(14.0))
        } else {
            egui::Rect::from_center_size(egui::pos2(r.center().x, r.top() + 9.0), egui::Vec2::splat(14.0))
        };
        paint_icon(ui.painter(), "info-circle-filled", icon, p.muted_dim);
        ui.vertical(|ui| {
            ui.set_width((ui.available_width() - RIGHT).max(40.0));
            ui.add_space(1.0);
            chat_text_marked(ui, p, env.emoji, env.world, env.want_names, &line.text, 12.0, tone, false, mark);
        });
    });
    ui.add_space(4.0);
}

/// The line between two days, with the day in its middle.
fn day_separator(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.add_space(8.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 18.0), egui::Sense::hover());
    let galley = ui.painter().layout_no_wrap(text.to_owned(), egui::FontId::proportional(11.0), p.muted_dim);
    let half = galley.size().x * 0.5 + 8.0;
    let y = rect.center().y;
    let stroke = egui::Stroke::new(1.0, p.raised.gamma_multiply(0.8));
    let (l, r) = (rect.left() + 16.0, rect.right() - 16.0);
    if rect.center().x - half > l {
        ui.painter().hline(l..=rect.center().x - half, y, stroke);
        ui.painter().hline(rect.center().x + half..=r, y, stroke);
    }
    ui.painter()
        .galley(egui::pos2(rect.center().x - galley.size().x * 0.5, y - galley.size().y * 0.5), galley, p.muted_dim);
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
    ui.painter()
        .hline(rect.left() + 16.0..=text_left - 8.0, y, egui::Stroke::new(1.0, p.danger.gamma_multiply(0.6)));
    ui.painter().galley(egui::pos2(text_left, y - galley.size().y * 0.5), galley, p.danger);
    ui.add_space(2.0);
}

/// The toolbar of the row under the pointer, at its top right: copy the
/// line, quote it in the field, the author's profile, the author's menu.
fn toolbar(ui: &mut egui::Ui, env: &mut LineEnv, th: &mut Thread, line: &ChatLine, name: &str, who: &Who, row: egui::Rect) {
    let p = env.p;
    const B: f32 = 24.0;
    let person = !who.avatar.is_nil();
    let n = if person { 4.0 } else { 2.0 };
    let bar = egui::Rect::from_min_size(egui::pos2(row.right() - RIGHT - B * n - 4.0, row.top() + 2.0), egui::vec2(B * n + 4.0, B + 4.0));
    let radius = ui.visuals().widgets.inactive.corner_radius;
    ui.painter().rect_filled(bar, radius, p.panel);
    let mut x = bar.left() + 2.0;
    let mut button = |ui: &mut egui::Ui, icon: &str, tip: &str, salt: &str| {
        let rect = egui::Rect::from_min_size(egui::pos2(x, bar.top() + 2.0), egui::Vec2::splat(B));
        x += B;
        let r = ui.interact(rect, ui.id().with(("thread_tool", salt, line.time, line.source)), egui::Sense::click());
        if r.hovered() {
            ui.painter().rect_filled(rect, radius, p.raised);
        }
        paint_icon(
            ui.painter(),
            icon,
            egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(15.0)),
            if r.hovered() { p.ink } else { p.muted },
        );
        r.on_hover_text(tip)
    };
    let (text, _, _) = body(env, line, name);
    let shown = super::shown_text(env.world, line, name);
    if button(ui, "copy-filled", aurora_i18n::tr!("ui-chat-copy-message"), "copy").clicked() {
        let prefix = super::copy_prefix(line, name);
        ui.ctx().copy_text(format!("{prefix}{shown}"));
    }
    if button(ui, "quote-filled", aurora_i18n::tr!("ui-chat-quote-message"), "quote").clicked() {
        // the field holds one line: the quote, then the answer after it
        let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
        th.out.push(ThreadAction::Quote(format!("> {one_line} ")));
    }
    if person {
        if button(ui, "user-filled", aurora_i18n::tr!("common-view-profile"), "profile").clicked() {
            crate::ui::profile::request_open(ui.ctx(), who.avatar);
        }
        let more = button(ui, "dots-filled", aurora_i18n::tr!("ui-chat-more"), "more");
        crate::ui::menu::dropdown(&more, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
            crate::ui::context::chat_sender_menu(ui, p, env.world, who.avatar, line);
        });
    }
}

/// The message names us: an explicit mention, or our display or user name
/// as a word of its text.
fn mentions(env: &LineEnv, me: &[String], line: &ChatLine) -> bool {
    if matches!(line.kind, ChatKind::Own | ChatKind::System | ChatKind::Invite(_)) || line.origin != Origin::Live {
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
        let me = vec!["odessa".to_owned(), "odessa.draekavik".to_owned()];
        assert!(names_in("hey Odessa, ready?", &me));
        assert!(names_in("ODESSA!", &me));
        assert!(names_in("ask odessa.draekavik", &me));
        assert!(!names_in("odessan wine", &me));
        assert!(!names_in("nobody here", &me));
        // too short a name never marks
        assert!(!names_in("an ox", &["ox".to_owned()]));
    }
}
