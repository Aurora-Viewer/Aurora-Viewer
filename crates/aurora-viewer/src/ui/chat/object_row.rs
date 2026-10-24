//! Everything no person said, on one small row (#475, #741, Preferences ›
//! Chat, off by default): what objects and scripts say (their owner-say,
//! their llRegionSayTo to us and their IMs included) and the viewer's
//! notices. Only people's messages keep their full size.
//!
//! - An object's row: a cube where the notices have their icon (its name
//!   under the pointer, a click inspects it), the object's name in the
//!   colour of objects' names (the same click and its menu: inspect,
//!   ignore), then its text in the notices' size and muted tone, wrapping
//!   under itself.
//! - A notice's row: an icon of what it is about (`notice_topics.rs`: a
//!   resident, a teleport, an item, the region, the music, a refusal, the
//!   "i" otherwise), then its text.
//!
//! The texts keep their links, find highlights and selection; no picture,
//! no head, no link cards. Not in the Legacy Viewer, whose object lines look
//! like residents' ones (its plain text history comes closest). Every style
//! of the Conversations window draws them here, the cards above the chat
//! bar with `console.rs`.

use super::{LineEnv, Mark, chat_text_marked, hhmm, history_tone, object_sender, sender_name};
use crate::world::notice_topics::Topic;
use crate::world::{ChatKind, ChatLine, World};
use egui::RichText;

/// Text size of the rows: the notices'.
pub(super) const TEXT: f32 = 12.0;
/// Side of the cube.
const ICON: f32 = 13.0;
/// Between the cube, the time, the name and the text.
const GAP: f32 = 5.0;
/// Room kept at the right of a row.
const RIGHT: f32 = 14.0;

/// The line takes a compact row: no person sent it (an object's line, said,
/// shouted, whispered, owner-say, said to us or a script error; an
/// object's IM; a notice), while the option is on.
pub(crate) fn compact(line: &ChatLine, on: bool) -> bool {
    on && line.kind.is_automated()
}

/// The icon of a compact row: the cube of objects, or what a notice is
/// about, in the icons the interface already gives these things.
pub(super) fn icon(world: &World, line: &ChatLine) -> &'static str {
    if line.kind != ChatKind::System {
        return "box";
    }
    match world.notice_topics.of(line) {
        Topic::Info => "info-circle-filled",
        Topic::People => "user-filled",
        Topic::Teleport => "plane-departure-filled",
        Topic::Items => "box",
        Topic::Region => "world-filled",
        Topic::Media => "music",
        Topic::Warning => "alert-triangle-filled",
    }
}

/// Where a style puts the parts of a row, from the left of the row.
#[derive(Debug, Clone, Copy)]
pub(super) struct Place {
    /// The middle of the cube: under the pictures of the style.
    pub icon_center: f32,
    /// Where the name starts: the text column of the style.
    pub text_left: f32,
    /// A column of times before the cube (the compact workspace), its
    /// width; elsewhere the time follows the name.
    pub time_column: Option<f32>,
}

/// One compact row.
pub(super) fn draw(ui: &mut egui::Ui, env: &mut LineEnv, line: &ChatLine, mark: Option<Mark>, place: Place, times: bool) {
    let p = env.p;
    let world = env.world;
    let tone = if env.plain_history { None } else { history_tone(p, line.origin) };
    ui.add_space(1.0);
    let top = ui.cursor().top();
    let left = ui.max_rect().left();
    // (a notice has no time, as everywhere)
    let notice = line.kind == ChatKind::System;
    let times = times && !notice;
    let row = ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let mut x = 0.0;
        if let Some(w) = place.time_column {
            ui.add_space(4.0);
            let (r, _) = ui.allocate_exact_size(egui::vec2(w, 16.0), egui::Sense::hover());
            if times {
                ui.painter().text(
                    egui::pos2(r.right(), r.top() + 8.5),
                    egui::Align2::RIGHT_CENTER,
                    hhmm(line.time),
                    egui::FontId::proportional(10.5),
                    p.muted_dim,
                );
            }
            x = 4.0 + w;
        }
        let icon_center = place.icon_center.max(x + GAP + ICON * 0.5);
        let text_left = place.text_left.max(icon_center + ICON * 0.5 + GAP);
        let (lead, _) = ui.allocate_exact_size(egui::vec2(text_left - x, 16.0), egui::Sense::hover());
        let icon = egui::Rect::from_center_size(egui::pos2(left + icon_center, lead.top() + 8.0), egui::Vec2::splat(ICON));
        super::thread::paint_icon(ui.painter(), self::icon(world, line), icon, p.muted_dim);
        if !notice {
            let name = sender_name(world, env.own_name, line);
            // the cube names its object under the pointer and inspects it
            // like the name (an object's IM comes from its owner's id: the
            // name alone, no menu)
            let id = ui.scope_id().with(("compact_icon", line.time, line.source));
            let cube = ui.interact(icon.expand(2.0), id, egui::Sense::click()).on_hover_text(&name);
            let color = tone.unwrap_or_else(|| crate::ui::colors::sender_color(line));
            let font = egui::FontId::new(TEXT, crate::ui::fonts::semibold(ui.ctx()));
            let max = (ui.available_width() * 0.35).max(60.0);
            let r =
                ui.add(crate::ui::widgets::elided_label(ui, RichText::new(&name).font(font).color(color), max).sense(egui::Sense::click()));
            if matches!(line.kind, ChatKind::Object(_)) && !line.source.is_nil() {
                for r in [cube, r] {
                    object_sender(
                        &r.on_hover_cursor(egui::CursorIcon::PointingHand),
                        p,
                        world,
                        line.source,
                        &line.from,
                    );
                }
            }
            ui.add_space(GAP);
        }
        if times && place.time_column.is_none() {
            ui.label(RichText::new(hhmm(line.time)).size(10.5).color(p.muted_dim));
            ui.add_space(GAP);
        }
        let (text, italics) = if notice { (&line.text[..], false) } else { body(line) };
        let color = crate::ui::colors::keyword_color(line).or(tone).unwrap_or(p.muted);
        ui.vertical(|ui| {
            ui.set_width((ui.available_width() - RIGHT).max(40.0));
            chat_text_marked(ui, p, env.emoji, world, env.want_names, text, TEXT, color, italics, mark);
        });
    });
    super::find::note_message(ui, top, Some(row.response.rect.bottom()));
    ui.add_space(1.0);
}

/// The text after the name: an emote's action in italics (its name is
/// already shown), the message otherwise.
fn body(line: &ChatLine) -> (&str, bool) {
    match line.text.strip_prefix("/me") {
        Some(rest) if rest.starts_with(' ') || rest.starts_with('\'') => (rest.trim_start(), true),
        _ => (&line.text, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::Origin;
    use aurora_net::ChatType;

    fn line(kind: ChatKind, text: &str) -> ChatLine {
        ChatLine {
            time: std::time::SystemTime::now(),
            from: "Demo Vendor".into(),
            text: text.into(),
            kind,
            source: uuid::Uuid::from_u128(9),
            origin: Origin::Live,
        }
    }

    /// Everything no person sent is compacted while the option is on (every
    /// way an object talks, its IM, the viewer's notices); residents,
    /// ourselves, IMs and residents' offers never are, and nothing is with
    /// the option off.
    #[test]
    fn only_what_no_person_sent_is_compacted_and_only_with_the_option() {
        let objects = [
            ChatType::Normal,
            ChatType::Shout,
            ChatType::Whisper,
            ChatType::OwnerSay,
            ChatType::Other(9),
            ChatType::Debug,
        ];
        let automated = objects
            .into_iter()
            .map(ChatKind::Object)
            .chain([ChatKind::ObjectIm, ChatKind::System]);
        for kind in automated {
            assert!(compact(&line(kind, "hi"), true), "{kind:?}");
            assert!(!compact(&line(kind, "hi"), false), "{kind:?} with the option off");
        }
        for kind in [
            ChatKind::Local(ChatType::Normal),
            ChatKind::Local(ChatType::Shout),
            ChatKind::Own,
            ChatKind::Im,
            ChatKind::Invite(1),
            ChatKind::InventoryOffer(1),
        ] {
            assert!(!compact(&line(kind, "hi"), true), "{kind:?}");
        }
    }

    /// The texts an object's line paints in a style, and whether a bubble
    /// (or a header strip) holds it.
    fn painted(style: Option<bool>, on: bool) -> (Vec<(String, f32)>, bool) {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let p = crate::theme::Theme::default().palette();
        let world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let icons = crate::ui::icons::Icons::default();
        let mut emoji = crate::ui::emoji::Emoji::load();
        let pics = std::collections::HashMap::new();
        let (mut names, mut wanted) = (std::collections::HashSet::new(), std::collections::HashSet::new());
        let said = line(ChatKind::Object(ChatType::Normal), "Touch me for a notecard");
        let look = super::super::view::Look {
            header: true,
            tail: true,
            ..Default::default()
        };
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 400.0))),
                ..Default::default()
            },
            |ui| {
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
                    compact_automated: on,
                    menu: None,
                    bubbles: style,
                    previews: None,
                };
                super::super::draw_line(ui, &mut env, &said, look, None);
            },
        );
        output.textures_delta.clear();
        let shapes: Vec<&egui::Shape> = output.shapes.iter().map(|c| &c.shape).collect();
        let texts = shapes
            .iter()
            .filter_map(|s| match s {
                egui::Shape::Text(t) => Some((
                    t.galley.text().to_owned(),
                    t.galley.job.sections.first().map_or(0.0, |s| s.format.font_id.size),
                )),
                _ => None,
            })
            .collect();
        let held = shapes
            .iter()
            .any(|s| matches!(s, egui::Shape::Rect(r) if r.fill != egui::Color32::TRANSPARENT));
        (texts, held)
    }

    /// Off, an object's line keeps the style's look (its bubble or its
    /// header strip, the text at the style's size); on, it is one row of
    /// the notices' size, the name then the text, nothing around them.
    #[test]
    fn the_option_off_keeps_each_styles_look_and_on_makes_one_small_row() {
        for style in [Some(true), None] {
            let (texts, held) = painted(style, false);
            assert!(held, "{style:?}: the bubble or the strip");
            let body = texts.iter().find(|(t, _)| t == "Touch me for a notecard").expect("the text");
            assert!(body.1 > TEXT, "{style:?}: the style's size {}", body.1);
            let (texts, held) = painted(style, true);
            assert!(!held, "{style:?}: nothing around the row");
            assert_eq!(
                texts.iter().find(|(t, _)| t == "Touch me for a notecard").map(|t| t.1),
                Some(TEXT),
                "{style:?}"
            );
            assert!(texts.iter().any(|(t, _)| t == "Demo Vendor"), "{style:?}: the name");
        }
    }

    /// Objects show their cube, notices the icon of what they are about.
    #[test]
    fn each_row_has_the_icon_of_its_kind() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        assert_eq!(icon(&world, &line(ChatKind::Object(ChatType::OwnerSay), "hi")), "box");
        assert_eq!(icon(&world, &line(ChatKind::ObjectIm, "hi")), "box");
        let topics = [
            (Topic::People, "user-filled"),
            (Topic::Teleport, "plane-departure-filled"),
            (Topic::Items, "box"),
            (Topic::Region, "world-filled"),
            (Topic::Media, "music"),
            (Topic::Warning, "alert-triangle-filled"),
            (Topic::Info, "info-circle-filled"),
        ];
        for (topic, expected) in topics {
            world.system_notice(format!("{topic:?}"), topic);
            let notice = world.chat.back().expect("the notice").clone();
            assert_eq!(icon(&world, &notice), expected, "{topic:?}");
            assert!(crate::ui::icons::embedded(expected), "{expected} is built in");
        }
    }

    #[test]
    fn an_objects_emote_keeps_its_action_without_its_name() {
        let said = line(ChatKind::Object(ChatType::Normal), "Touch me for a notecard");
        assert_eq!(body(&said), ("Touch me for a notecard", false));
        let emote = line(ChatKind::Object(ChatType::Normal), "/me hums softly");
        assert_eq!(body(&emote), ("hums softly", true));
        let possessive = line(ChatKind::Object(ChatType::Normal), "/me's light blinks");
        assert_eq!(body(&possessive), ("'s light blinks", true));
        // "/meow" is no emote
        let word = line(ChatKind::Object(ChatType::Normal), "/meow");
        assert_eq!(body(&word), ("/meow", false));
    }
}
