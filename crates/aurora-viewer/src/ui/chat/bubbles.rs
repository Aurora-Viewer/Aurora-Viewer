//! The lines of a conversation as a messenger shows them: bubbles on either
//! side, grouped by sender, the day between days and the notices of the
//! viewer in a pill of their own.
//!
//! - Mine on the right, on the accent (violet to indigo), the others' on the
//!   left on `raised`; corners of the theme's buttons, the one under the
//!   sender square on the last bubble of a group.
//! - The sender's picture (and its status dot) and the time close a group,
//!   under its last bubble; in a group chat or the nearby chat the sender's
//!   name opens it, in the color of the names of the chat.
//! - Same texts as the classic lines (`draw_line`): links, mentions, emoji,
//!   find highlights, history greyed and looked up for nothing.

use super::{LineEnv, Mark, chat_text_marked, hhmm, history_tone, name_menu, presence, view};
use crate::theme::Palette;
use crate::world::chat_log::format::Slt;
use crate::world::{ChatKind, Origin};
use aurora_net::ChatType;
use egui::{Color32, RichText};

/// Side of the sender's picture, and the column it takes on the left.
const PIC: f32 = 26.0;
const PIC_GAP: f32 = 6.0;
/// Room between my bubbles and the right edge of the conversation.
const EDGE: f32 = 8.0;
/// Share of the row a bubble may take.
const MAX_SHARE: f32 = 0.78;
/// Room between the edge of a bubble and its text.
const PAD: egui::Margin = egui::Margin {
    left: 10,
    right: 10,
    top: 6,
    bottom: 6,
};
const TEXT_SIZE: f32 = 13.5;

/// What the day between two days of a conversation says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DayLabel {
    Today,
    Yesterday,
    Date(i64, i64, i64),
}

pub(super) fn day_label(day: (i64, i64, i64), today: &Slt, yesterday: &Slt) -> DayLabel {
    let is = |s: &Slt| (s.year, s.month, s.day) == day;
    if is(today) {
        DayLabel::Today
    } else if is(yesterday) {
        DayLabel::Yesterday
    } else {
        DayLabel::Date(day.0, day.1, day.2)
    }
}

fn day_marker(ui: &mut egui::Ui, p: &Palette, day: (i64, i64, i64)) {
    let now = std::time::SystemTime::now();
    let yesterday = Slt::of(now - std::time::Duration::from_secs(86_400));
    let text = match day_label(day, &Slt::of(now), &yesterday) {
        DayLabel::Today => aurora_i18n::tr!("ui-chat-today").to_owned(),
        DayLabel::Yesterday => aurora_i18n::tr!("ui-chat-yesterday").to_owned(),
        DayLabel::Date(year, month, day) => {
            let month = crate::ui::land::months()[(month.clamp(1, 12) - 1) as usize];
            aurora_i18n::tr!(
                "ui-chat-day",
                day = day.to_string(),
                month = month.to_string(),
                year = year.to_string()
            )
        }
    };
    ui.add_space(6.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(text).size(11.0).color(p.muted_dim));
    });
    ui.add_space(4.0);
}

/// A shape filled with a diagonal gradient, `from` at the top left: the
/// rounded rectangle is tessellated once, its vertices then take the color
/// of where they stand (the feathered edge keeps its coverage in its alpha).
fn gradient_rect(ctx: &egui::Context, rect: egui::Rect, corners: egui::CornerRadius, from: Color32, to: Color32) -> egui::Shape {
    let mut mesh = egui::Mesh::default();
    let mut t = egui::epaint::Tessellator::new(ctx.pixels_per_point(), ctx.tessellation_options(|o| *o), [1, 1], Vec::new());
    t.tessellate_rect(&egui::epaint::RectShape::filled(rect, corners, from), &mut mesh);
    let size = rect.size().max(egui::Vec2::splat(1.0));
    for v in &mut mesh.vertices {
        let at = ((v.pos.x - rect.left()) / size.x + (v.pos.y - rect.top()) / size.y) * 0.5;
        let coverage = f32::from(v.color.a()) / 255.0;
        v.color = from.lerp_to_gamma(to, at.clamp(0.0, 1.0)).gamma_multiply(coverage);
    }
    egui::Shape::mesh(mesh)
}

/// The width a bubble's text takes, at most `max`: the plain text laid out
/// in the bubble's font, with the room the color emoji and the badges of
/// web links take more than their characters. A text that comes out wider
/// than this only wraps sooner.
fn text_width(ui: &egui::Ui, text: &str, max: f32) -> f32 {
    let galley = ui
        .painter()
        .layout(text.to_owned(), egui::FontId::proportional(TEXT_SIZE), Color32::PLACEHOLDER, max);
    let emoji = text
        .chars()
        .filter(|c| matches!(*c as u32, 0x2600..=0x27BF | 0x1F000..=0x1FAFF))
        .count() as f32;
    let links = text.matches("://").count() as f32;
    (galley.size().x + 2.0 + emoji * TEXT_SIZE * 0.4 + links * TEXT_SIZE * 1.6).min(max)
}

/// A notice of the viewer (offer sent, automatic response…): centered, in
/// a discreet pill.
fn system_pill(ui: &mut egui::Ui, env: &mut LineEnv, line: &crate::world::ChatLine, mark: Option<Mark>) {
    let (p, world) = (env.p, env.world);
    let tone = if env.plain_history { None } else { history_tone(p, line.origin) };
    let width = text_width(ui, &line.text, ui.available_width() * 0.86);
    ui.add_space(3.0);
    ui.vertical_centered(|ui| {
        egui::Frame::new()
            .fill(p.raised.gamma_multiply(0.35))
            .corner_radius(ui.visuals().widgets.inactive.corner_radius)
            .inner_margin(egui::Margin::symmetric(10, 3))
            .show(ui, |ui| {
                ui.set_max_width(width);
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    chat_text_marked(
                        ui,
                        p,
                        env.emoji,
                        world,
                        env.want_names,
                        &line.text,
                        12.0,
                        tone.unwrap_or(p.muted),
                        true,
                        mark,
                    );
                });
            });
    });
    ui.add_space(3.0);
}

/// One line of a conversation as a bubble. `names`: the sender's name opens
/// each group (several people write here); an object's always does.
pub(super) fn draw(ui: &mut egui::Ui, env: &mut LineEnv, line: &crate::world::ChatLine, look: view::Look, mark: Option<Mark>, names: bool) {
    let (p, world) = (env.p, env.world);
    ui.spacing_mut().item_spacing.y = 2.0;
    if let Some(day) = look.marker {
        day_marker(ui, p, day);
    }
    if line.kind == ChatKind::System {
        system_pill(ui, env, line, mark);
        return;
    }
    let logged = line.origin != Origin::Live;
    let tone = if env.plain_history { None } else { history_tone(p, line.origin) };
    let own = line.kind == ChatKind::Own;
    let object = matches!(line.kind, ChatKind::Object(_) | ChatKind::ObjectIm);
    let names_cache = &world.social.avatar_names;
    let cached = if object || logged {
        None
    } else {
        names_cache.get(if own { &world.agent_id } else { &line.source })
    };
    let name = match cached {
        Some(n) => n.display(&names_cache.options),
        None if own && !logged => env.own_name.to_owned(),
        None => line.from.clone(),
    };
    let avatar = if logged || object {
        uuid::Uuid::nil()
    } else if own {
        world.agent_id
    } else {
        line.source
    };
    let w = ui.available_width();
    // mine keep off the scroll bar at the right edge
    let lead = if own { EDGE } else { PIC + PIC_GAP };
    let max_bubble = ((w - lead) * MAX_SHARE).max(80.0);

    if (names || object) && look.header && !own {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(lead + 2.0);
            let col = tone.unwrap_or_else(|| super::super::colors::sender_color(line));
            let r = ui.add(
                super::super::widgets::elided_label(ui, RichText::new(&name).size(11.5).strong().color(col), max_bubble)
                    .sense(egui::Sense::click()),
            );
            if !avatar.is_nil() {
                name_menu(&r, p, world, avatar);
                if r.on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(aurora_i18n::tr!("common-view-profile"))
                    .clicked()
                {
                    super::super::profile::request_open(ui.ctx(), avatar);
                }
            }
        });
    } else if look.header {
        ui.add_space(4.0);
    }

    // the text: an emote is the name in italics, a shout or a whisper says so
    let emote = line.text.starts_with("/me ") || line.text.starts_with("/me'");
    let (text, italics) = if emote {
        (format!("{}{}", name, &line.text[3..]), true)
    } else {
        let prefix = match line.kind {
            ChatKind::Local(ChatType::Shout) => aurora_i18n::tr!("status-shouts-2"),
            ChatKind::Local(ChatType::Whisper) => aurora_i18n::tr!("status-whispers-2"),
            _ => "",
        };
        (format!("{prefix}{}", line.text), false)
    };
    let text_color = match (own, tone) {
        (true, None) => p.on_accent,
        (true, Some(_)) => p.on_accent.gamma_multiply(0.75),
        (false, Some(t)) => t,
        // an object keeps the color of objects; a resident's text reads on
        // the bubble like any text of the interface
        (false, None) if object => super::super::colors::chat_color(line, world),
        (false, None) => p.ink,
    };

    // top-aligned rows: a row aligned to its bottom would take the whole
    // height it is given; the picture is placed against the bubble after it
    let layout = if own {
        egui::Layout::right_to_left(egui::Align::Min)
    } else {
        egui::Layout::left_to_right(egui::Align::Min)
    };
    ui.with_layout(layout, |ui| {
        ui.add_space(lead);
        let behind = ui.painter().add(egui::Shape::Noop);
        let width = text_width(ui, &text, max_bubble);
        let bubble = egui::Frame::new().inner_margin(PAD).show(ui, |ui| {
            // the width of the text, on the bubble's side of the row
            ui.set_max_width(width);
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                chat_text_marked(ui, p, env.emoji, world, env.want_names, &text, TEXT_SIZE, text_color, italics, mark);
            });
        });
        let rect = bubble.response.rect;
        let mut corners = ui.visuals().widgets.inactive.corner_radius;
        if look.tail {
            // square under the sender, like the tail of a speech bubble
            if own {
                corners.se = 0;
            } else {
                corners.sw = 0;
            }
        }
        let shape = if own {
            let (from, to) = if logged {
                (p.violet.gamma_multiply(0.55), p.indigo.gamma_multiply(0.55))
            } else {
                (p.violet, p.indigo)
            };
            gradient_rect(ui.ctx(), rect, corners, from, to)
        } else {
            let fill = if logged { p.raised.gamma_multiply(0.6) } else { p.raised };
            egui::epaint::RectShape::filled(rect, corners, fill).into()
        };
        ui.painter().set(behind, shape);
        if !own && look.tail {
            let pic = egui::Rect::from_min_size(egui::pos2(rect.left() - PIC_GAP - PIC, rect.bottom() - PIC), egui::Vec2::splat(PIC));
            sender_picture(ui, env, line, pic, &name, avatar, object, logged);
            if !avatar.is_nil() {
                let resp = ui.interact(pic, ui.id().with("sender"), egui::Sense::click());
                name_menu(&resp, p, world, avatar);
                if resp
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(name.as_str())
                    .clicked()
                {
                    super::super::profile::request_open(ui.ctx(), avatar);
                }
            }
        }
    });

    if look.tail {
        let time = hhmm(line.time);
        if !time.is_empty() {
            let layout = if own {
                egui::Layout::right_to_left(egui::Align::Min)
            } else {
                egui::Layout::left_to_right(egui::Align::Min)
            };
            ui.with_layout(layout, |ui| {
                ui.add_space(lead + 2.0);
                ui.label(RichText::new(time).size(10.5).color(p.muted_dim));
            });
        }
        ui.add_space(4.0);
    }
}

/// The picture closing a group: the avatar's (with its status dot), a box
/// for an object, initials for a history line (nothing is looked up then).
#[allow(clippy::too_many_arguments)]
fn sender_picture(
    ui: &mut egui::Ui,
    env: &mut LineEnv,
    line: &crate::world::ChatLine,
    rect: egui::Rect,
    name: &str,
    avatar: uuid::Uuid,
    object: bool,
    logged: bool,
) {
    let p = env.p;
    if object {
        ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.raised);
        if let Some(t) = env.icons.get("box") {
            ui.painter().image(
                t.id(),
                rect.shrink(5.0),
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                super::super::colors::sender_color(line),
            );
        }
        return;
    }
    if !avatar.is_nil() {
        env.wanted.insert(avatar);
    }
    super::super::widgets::avatar_disc(ui, p, rect, env.pics.get(&avatar).filter(|_| !logged), name, logged);
    if !avatar.is_nil() {
        let status = presence::of(env.world, &avatar);
        presence::paint_dot(ui.painter(), p, rect, status, p.field);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_are_named_today_and_yesterday() {
        let slt = |year, month, day| Slt {
            year,
            month,
            day,
            hour: 0,
            minute: 0,
        };
        let (today, yesterday) = (slt(2026, 3, 1), slt(2026, 2, 28));
        assert_eq!(day_label((2026, 3, 1), &today, &yesterday), DayLabel::Today);
        assert_eq!(day_label((2026, 2, 28), &today, &yesterday), DayLabel::Yesterday);
        assert_eq!(day_label((2026, 2, 27), &today, &yesterday), DayLabel::Date(2026, 2, 27));
        assert_eq!(day_label((2025, 3, 1), &today, &yesterday), DayLabel::Date(2025, 3, 1));
    }

    #[test]
    fn the_first_message_of_another_day_says_its_day() {
        use crate::world::{ChatLine, Origin};
        use std::time::{Duration, UNIX_EPOCH};
        let at = |secs: u64, kind, text: &str| ChatLine {
            time: UNIX_EPOCH + Duration::from_secs(1_800_000_000 + secs),
            from: "Nova".into(),
            text: text.into(),
            kind,
            source: uuid::Uuid::from_u128(1),
            origin: Origin::Live,
        };
        let day = 86_400;
        // yesterday's two lines, then today's
        let lines = [
            at(0, ChatKind::Own, "concert?"),
            at(180, ChatKind::Im, "can't"),
            at(day, ChatKind::Im, "sorry"),
        ];
        let today = Slt::of(lines[2].time);
        let yesterday = Slt::of(lines[0].time);
        let mut g = view::Grouping::new(today);
        let looks: Vec<view::Look> = lines.iter().map(|l| g.look(l)).collect();
        // above the very first message: "Yesterday"
        let first = looks[0].marker.expect("a day above the first message");
        assert_eq!(day_label(first, &today, &yesterday), DayLabel::Yesterday);
        assert_eq!(looks[1].marker, None);
        // and "Today" above today's
        let second = looks[2].marker.expect("a day between the two days");
        assert_eq!(day_label(second, &today, &yesterday), DayLabel::Today);
    }
}
