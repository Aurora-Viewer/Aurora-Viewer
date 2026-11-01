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

use super::{LineEnv, Mark, chat_text_marked, hhmm, history_tone, line_name_menu, presence, view};
use crate::theme::Palette;
use crate::world::chat_log::format::Slt;
use crate::world::{ChatKind, Origin};
use egui::{Color32, RichText};

/// Side of the sender's picture, and the column it takes on the left.
const PIC: f32 = 26.0;
const PIC_GAP: f32 = 6.0;
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

/// What the day between two days says: today, yesterday or the date.
pub(super) fn day_text(day: (i64, i64, i64)) -> String {
    let now = std::time::SystemTime::now();
    let yesterday = Slt::of(now - std::time::Duration::from_secs(86_400));
    match day_label(day, &Slt::of(now), &yesterday) {
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
    }
}

fn day_marker(ui: &mut egui::Ui, p: &Palette, day: (i64, i64, i64)) {
    let text = day_text(day);
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
fn text_width(ui: &egui::Ui, text: &str, size: f32, max: f32) -> f32 {
    let galley = ui
        .painter()
        .layout(text.to_owned(), egui::FontId::proportional(size), Color32::PLACEHOLDER, max);
    let emoji = text
        .chars()
        .filter(|c| matches!(*c as u32, 0x2600..=0x27BF | 0x1F000..=0x1FAFF))
        .count() as f32;
    let links = text.matches("://").count() as f32;
    (galley.size().x + 2.0 + emoji * size * 0.4 + links * size * 1.6).min(max)
}

/// A notice of the viewer (offer sent, automatic response…): centered, in
/// a discreet pill.
fn system_pill(ui: &mut egui::Ui, env: &mut LineEnv, line: &crate::world::ChatLine, mark: Option<Mark>) {
    let (p, world) = (env.p, env.world);
    let tone = if env.plain_history { None } else { history_tone(p, line.origin) };
    let width = text_width(ui, &line.text, TEXT_SIZE, ui.available_width() * 0.86);
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
    // (the current match's marker starts under the day)
    super::find::note_message(ui, ui.cursor().top(), None);
    let compact = super::object_row::compact_in(line, env);
    if line.kind == ChatKind::System && !compact {
        system_pill(ui, env, line, mark);
        return;
    }
    if compact {
        // the cube under the senders' pictures, the name where the bubbles start
        let inset = ui.spacing().item_spacing.x;
        let place = super::object_row::Place {
            icon_center: inset + PIC * 0.5,
            text_left: inset + PIC + PIC_GAP,
            time_column: None,
        };
        ui.add_space(if look.first { inset } else { 2.0 });
        super::object_row::draw(ui, env, line, mark, place, true);
        return;
    }
    if let ChatKind::Invite(invite) = line.kind {
        super::invite_card::draw_card(ui, env, line, invite, PIC + PIC_GAP);
        return;
    }
    if let ChatKind::InventoryOffer(offer) = line.kind {
        super::offer_card::draw_card(ui, env, line, offer, PIC + PIC_GAP);
        return;
    }
    if let ChatKind::Payment(payment) = line.kind {
        super::payment_card::draw_card(ui, env, line, payment, PIC + PIC_GAP);
        return;
    }
    let logged = line.origin != Origin::Live;
    let tone = if env.plain_history { None } else { history_tone(p, line.origin) };
    let own = line.kind.is_own();
    let object = matches!(line.kind, ChatKind::Object(_) | ChatKind::ObjectIm);
    let name = super::sender_name(world, env.own_name, line);
    // a history line's sender is known when its name was cached
    // (world/transcripts.rs): its picture shows, dimmed
    let avatar = if object {
        uuid::Uuid::nil()
    } else if own {
        world.agent_id
    } else {
        line.source
    };
    let w = ui.available_width();
    // the same inset of the theme's spacing on both sides: the pictures off
    // the left edge, my bubbles off the right one (and its scroll bar)
    let inset = ui.spacing().item_spacing.x;
    let lead = if own { inset } else { inset + PIC + PIC_GAP };
    let max_bubble = ((w - lead) * MAX_SHARE).max(80.0);

    if (names || object) && look.header && !own {
        // at the top of the list, the name keeps off its edge by the same
        // inset as the bubbles off the sides (as far as a day's text)
        ui.add_space(if look.first { inset } else { 4.0 });
        // a top-aligned row as tall as the name, like the time's: `horizontal`
        // would make it as tall as a button (`interact_size.y`) and leave the
        // name floating a third of a line above its bubble. Under it, half
        // the room between two bubbles: the line already ends with the
        // font's descent (2 to 3 px of air under the ink at scale 1)
        let spacing = ui.spacing().item_spacing.y;
        ui.spacing_mut().item_spacing.y = spacing * 0.5;
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
            ui.add_space(lead + 2.0);
            let col = super::super::colors::contact_sender_color(line, world)
                .or(tone)
                .unwrap_or_else(|| super::super::colors::sender_color(line));
            let r = ui.add(
                super::super::widgets::elided_label(ui, RichText::new(&name).size(11.5).strong().color(col), max_bubble)
                    .sense(egui::Sense::click()),
            );
            if !avatar.is_nil() {
                line_name_menu(&r, p, world, avatar, line, env.menu);
                if r.on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(aurora_i18n::tr!("common-view-profile"))
                    .clicked()
                {
                    super::super::profile::request_open(ui.ctx(), avatar);
                }
                crate::ui::staff::after_name(ui, p, crate::world::staff::mark(world, &avatar), 9.0);
            }
        });
        ui.spacing_mut().item_spacing.y = spacing;
    } else if look.header {
        ui.add_space(4.0);
    }

    // the text: an emote is the name in italics, a shout or a whisper says so
    let (text, italics) = super::line_body(line, &name);
    let text = world.rlva_filter_text(&text).into_owned();
    let visible = super::previews::text(env.previews.as_deref(), line, &text, max_bubble - 20.0);
    let text_color = match (own, tone) {
        (true, None) => p.on_accent,
        (true, Some(_)) => p.on_accent.gamma_multiply(0.75),
        (false, Some(t)) => t,
        // an object keeps the color of objects; a resident's text reads on
        // the bubble like any text of the interface
        (false, None) if object => super::super::colors::chat_color(line, world),
        // a keyword's colour shows on any bubble (Preferences › Chat)
        (false, None) => super::super::colors::keyword_color(line).unwrap_or(p.ink),
    };

    let text_color = super::super::colors::contact_chat_color(line, world).unwrap_or(text_color);

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
        let size = env.emoji.message_size(&visible, TEXT_SIZE);
        // as wide as its widest card, within the bubble's share of the row
        let cards = env.previews.as_ref().map_or(0.0, |pv| pv.width(line, &text));
        let width = text_width(ui, &visible, size, max_bubble).max(cards.min(max_bubble - 20.0));
        let bubble = egui::Frame::new().inner_margin(PAD).show(ui, |ui| {
            // the width of the text, on the bubble's side of the row
            ui.set_max_width(width);
            if own {
                // the theme's selection is the accent the bubble is on
                super::select::colors(p.on_accent, p.violet);
            }
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                if !visible.is_empty() {
                    chat_text_marked(
                        ui,
                        p,
                        env.emoji,
                        world,
                        env.want_names,
                        &visible,
                        size,
                        text_color,
                        super::message_emphasis(line.kind, italics),
                        mark,
                    );
                }
                super::previews::cards(ui, env, line, &text);
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
                let resp = ui.interact(pic, ui.scope_id().with("sender"), egui::Sense::click());
                line_name_menu(&resp, p, world, avatar, line, env.menu);
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

/// The picture closing a group: the avatar's (with its status dot), dimmed
/// for a history line; a box for an object; initials while no picture is
/// known.
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
        let (glyph, tint) = super::object_row::object_glyph(line, p, super::super::colors::sender_color(line));
        if let Some(t) = env.icons.get(glyph) {
            ui.painter().image(
                t.id(),
                rect.shrink(5.0),
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                tint,
            );
        }
        return;
    }
    if !avatar.is_nil() {
        env.wanted.insert(avatar);
    }
    super::super::widgets::avatar_disc(ui, p, rect, env.pics.get(&avatar), name, logged);
    if !avatar.is_nil() {
        let status = presence::of(env.world, &avatar);
        presence::paint_dot(ui.painter(), p, rect, status, p.field);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::ChatType;

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
            at(0, ChatKind::OWN, "concert?"),
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

    /// Where a line's bubble lands, at scale 1 in `font`: the top of the
    /// list, the ink of each text (text, top, bottom) and the top of the
    /// bubble.
    struct Drawn {
        list_top: f32,
        texts: Vec<(String, f32, f32)>,
        bubble_top: f32,
    }

    fn drawn(font: &str, kind: ChatKind, look: view::Look) -> Drawn {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        crate::ui::fonts::apply(&ctx, font);
        let p = crate::theme::Theme::default().palette();
        let world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let icons = crate::ui::icons::Icons::default();
        let mut emoji = crate::ui::emoji::Emoji::load();
        let pics = std::collections::HashMap::new();
        let (mut names, mut wanted) = (std::collections::HashSet::new(), std::collections::HashSet::new());
        let line = crate::world::ChatLine {
            time: std::time::SystemTime::now(),
            from: "Orion Exemple".into(),
            text: "Bienvenue sur la place".into(),
            kind,
            source: uuid::Uuid::from_u128(7),
            origin: Origin::Live,
        };
        let mut list_top = 0.0;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 400.0))),
                ..Default::default()
            },
            |ui| {
                list_top = ui.cursor().top();
                let mut env = LineEnv {
                    p: &p,
                    icons: &icons,
                    emoji: &mut emoji,
                    pics: &pics,
                    world: &world,
                    want_names: &mut names,
                    wanted: &mut wanted,
                    own_name: "Aurora Demo",
                    plain_history: false,
                    compact_automated: false,
                    compact_notices: false,
                    menu: None,
                    bubbles: Some(true),
                    previews: None,
                };
                draw(ui, &mut env, &line, look, None, true);
            },
        );
        output.textures_delta.clear();
        let shapes: Vec<&egui::Shape> = output.shapes.iter().map(|c| &c.shape).collect();
        // the ink of each text, glyph by glyph as the tessellator places them
        let texts = shapes
            .iter()
            .filter_map(|s| match s {
                egui::Shape::Text(t) => Some(t),
                _ => None,
            })
            .map(|t| {
                let (top, bottom) = t
                    .galley
                    .rows
                    .iter()
                    .flat_map(|row| row.row.glyphs.iter().map(move |g| (row.pos.y, g)))
                    .filter(|(_, g)| !g.uv_rect.is_nothing())
                    .map(|(row_y, g)| {
                        let top = t.pos.y + row_y + (g.pos.y + g.uv_rect.offset.y).round();
                        (top, top + g.uv_rect.size.y)
                    })
                    .fold((f32::MAX, f32::MIN), |(a, b), (top, bottom)| (a.min(top), b.max(bottom)));
                (t.galley.text().to_owned(), top, bottom)
            })
            .collect();
        let bubble_top = shapes
            .iter()
            .filter_map(|s| match s {
                egui::Shape::Rect(r) if r.fill == p.raised && r.rect.top() > list_top => Some(r.rect.top()),
                _ => None,
            })
            .fold(f32::MAX, f32::min);
        assert!(bubble_top < f32::MAX, "the bubble is drawn");
        Drawn {
            list_top,
            texts,
            bubble_top,
        }
    }

    const NAMED: view::Look = view::Look {
        marker: None,
        header: true,
        tail: true,
        first: false,
        new: false,
    };

    /// The sender's name sits just above its bubble, close enough to read as
    /// its title, without touching it (human request of 2026-10-15: it used
    /// to float 9 px above it), in the built-in fonts, for a resident as for
    /// an object: 2 to 3 empty pixels under its lowest ink (the "p").
    #[test]
    fn the_sender_name_sits_just_above_its_bubble() {
        use crate::ui::fonts::{INTER, NOTO, ROBOTO};
        let object = ChatKind::Object(aurora_net::ChatType::Normal);
        for font in [NOTO, INTER, ROBOTO] {
            for kind in [ChatKind::Local(ChatType::Normal), object] {
                let d = drawn(font, kind, NAMED);
                let (_, _, ink_bottom) = d.texts.iter().find(|t| t.0 == "Orion Exemple").expect("the sender's name is drawn");
                let gap = d.bubble_top - ink_bottom;
                assert!(
                    (2.0..=3.0).contains(&gap),
                    "{font} {kind:?}: {gap} px between the name and its bubble"
                );
            }
        }
    }

    /// A list that starts with a sender's name keeps it as far from its top
    /// edge as a list that starts with a day keeps the day's text (human
    /// request of 2026-10-15: the name stood 5 px from the edge).
    #[test]
    fn a_name_at_the_top_of_the_list_keeps_off_its_edge() {
        let kind = ChatKind::Local(ChatType::Normal);
        let named = drawn(crate::ui::fonts::DEFAULT, kind, view::Look { first: true, ..NAMED });
        let name_top = named
            .texts
            .iter()
            .find(|t| t.0 == "Orion Exemple")
            .expect("the sender's name is drawn")
            .1
            - named.list_top;
        let day = drawn(
            crate::ui::fonts::DEFAULT,
            kind,
            view::Look {
                marker: Some((2020, 1, 1)),
                first: true,
                ..NAMED
            },
        );
        // the day comes first, the name under it
        let day_top = day.texts.iter().map(|t| t.1).fold(f32::MAX, f32::min) - day.list_top;
        assert!(
            (name_top - day_top).abs() <= 1.0 && name_top >= 8.0,
            "the name's ink starts {name_top} px under the top, a day's {day_top} px"
        );
        // further down the list, groups stay as close as before
        let below = drawn(crate::ui::fonts::DEFAULT, kind, NAMED);
        let below_top = below
            .texts
            .iter()
            .find(|t| t.0 == "Orion Exemple")
            .expect("the sender's name is drawn")
            .1
            - below.list_top;
        assert!(
            below_top < name_top,
            "{below_top} px under the previous line, {name_top} px under the top"
        );
    }
}
