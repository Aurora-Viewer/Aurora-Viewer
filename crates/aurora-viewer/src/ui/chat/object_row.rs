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
/// Text size of the time.
const TIME: f32 = 10.5;
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

/// The viewer's notices are compacted too: in the nearby chat with the
/// option, in a conversation (a call, an automatic reply) only with its
/// sub-option as well (the human's choice of 2026-10-24).
pub(crate) fn compact_notices(on: bool, im_lines: bool, nearby: bool) -> bool {
    on && (nearby || im_lines)
}

/// The line takes a compact row in a conversation drawn with `env`.
pub(super) fn compact_in(line: &ChatLine, env: &LineEnv) -> bool {
    if line.kind == ChatKind::System {
        env.compact_notices
    } else {
        compact(line, env.compact_automated)
    }
}

/// One of our objects' script errors, shown in the nearby chat (#1263).
pub(crate) fn script_error(line: &ChatLine) -> bool {
    line.kind == ChatKind::Object(aurora_net::ChatType::Debug)
}

/// The warning sign of a script error.
const WARNING: &str = "alert-triangle-filled";

/// The sign an object's line shows for its sender, and its tint: the cube
/// in `cube`, its usual tint, or a script error's warning sign in the
/// interface's red, in every style (#1263).
pub(crate) fn object_glyph(line: &ChatLine, p: &crate::theme::Palette, cube: egui::Color32) -> (&'static str, egui::Color32) {
    if script_error(line) { (WARNING, p.danger) } else { ("box", cube) }
}

/// The text of a compact row: muted, a script error in its own colour.
pub(super) fn text_color(line: &ChatLine, p: &crate::theme::Palette) -> egui::Color32 {
    if script_error(line) {
        crate::ui::colors::script_error_color()
    } else {
        p.muted
    }
}

/// The icon of a compact row: the cube of objects (a script error's warning
/// sign), or what a notice is about, in the icons the interface already
/// gives these things.
pub(super) fn icon(world: &World, line: &ChatLine) -> &'static str {
    if script_error(line) {
        return WARNING;
    }
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
        Topic::Call => "phone-filled",
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

/// Where the letters of a `size` text drawn as a label stand, from the top
/// of its row: the baseline the time, the name and the icon of a row are
/// set on (`widgets::galley_baseline` of a label laid out as egui lays the
/// text's out). Each part set on its own row instead (a smaller time, a
/// semi-bold name) sat up to 2 px off the text, the time lower.
fn label_baseline(ui: &egui::Ui, size: f32) -> f32 {
    let probe = egui::WidgetText::from(RichText::new("H").size(size)).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Body,
    );
    crate::ui::widgets::galley_baseline(ui.painter(), &probe)
}

/// The middle of the capitals of a `size` text standing on `baseline`: where
/// a row's icon is centred.
pub(super) fn caps_middle(painter: &egui::Painter, size: f32, baseline: f32) -> f32 {
    let probe = painter.layout_no_wrap("H".into(), egui::FontId::proportional(size), egui::Color32::PLACEHOLDER);
    baseline - crate::ui::widgets::ink_rect(&probe).height() / 2.0
}

/// The time of a compact row, laid out.
pub(super) fn time_galley(painter: &egui::Painter, p: &crate::theme::Palette, line: &ChatLine) -> std::sync::Arc<egui::Galley> {
    painter.layout_no_wrap(hhmm(line.time), egui::FontId::proportional(TIME), p.muted_dim)
}

/// One compact row.
pub(super) fn draw(ui: &mut egui::Ui, env: &mut LineEnv, line: &ChatLine, mark: Option<Mark>, place: Place, times: bool) {
    use egui::emath::GuiRounding as _;
    let p = env.p;
    let world = env.world;
    let tone = if env.plain_history { None } else { history_tone(p, line.origin) };
    ui.add_space(1.0);
    let top = ui.cursor().top();
    let left = ui.max_rect().left();
    // (a notice has no time, as everywhere)
    let notice = line.kind == ChatKind::System;
    let times = times && !notice;
    let row_h = ui.fonts_mut(|f| f.row_height(&egui::FontId::proportional(TEXT)));
    // the text's first line stands there: everything beside it too
    let baseline = top.round_to_pixels(ui.pixels_per_point()) + label_baseline(ui, TEXT);
    let row = ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let mut x = 0.0;
        if let Some(w) = place.time_column {
            ui.add_space(4.0);
            let (r, _) = ui.allocate_exact_size(egui::vec2(w, row_h), egui::Sense::hover());
            if times {
                let galley = time_galley(ui.painter(), p, line);
                let at = r.right() - galley.size().x;
                crate::ui::widgets::paint_on_baseline(ui.painter(), galley, at, baseline, p.muted_dim);
            }
            x = 4.0 + w;
        }
        let icon_center = place.icon_center.max(x + GAP + ICON * 0.5);
        let text_left = place.text_left.max(icon_center + ICON * 0.5 + GAP);
        ui.allocate_exact_size(egui::vec2(text_left - x, row_h), egui::Sense::hover());
        let middle = caps_middle(ui.painter(), TEXT, baseline);
        let icon = egui::Rect::from_center_size(egui::pos2(left + icon_center, middle), egui::Vec2::splat(ICON));
        let tint = if script_error(line) { p.danger } else { p.muted_dim };
        super::thread::paint_icon(ui.painter(), self::icon(world, line), icon, tint);
        if !notice {
            let name = sender_name(world, env.own_name, line);
            // the cube names its object under the pointer and inspects it
            // like the name (an object's IM comes from its owner's id: the
            // name alone, no menu)
            let id = ui.scope_id().with(("compact_icon", line.time, line.source));
            let cube = ui.interact(icon.expand(2.0), id, egui::Sense::click()).on_hover_text(&name);
            let color = tone.unwrap_or_else(|| crate::ui::colors::sender_color(line));
            let font = egui::FontId::new(TEXT, crate::ui::fonts::semibold(ui.ctx()));
            let mut job = egui::text::LayoutJob::single_section(name.clone(), egui::TextFormat::simple(font, color));
            job.wrap = egui::text::TextWrapping {
                max_width: (ui.available_width() * 0.35).max(60.0),
                max_rows: 1,
                break_anywhere: true,
                overflow_character: Some('…'),
            };
            let galley = ui.fonts_mut(|f| f.layout_job(job));
            let (r, mut resp) = ui.allocate_exact_size(egui::vec2(galley.size().x, row_h), egui::Sense::click());
            if galley.elided {
                resp = resp.on_hover_text(&name);
            }
            crate::ui::widgets::paint_on_baseline(ui.painter(), galley, r.left(), baseline, color);
            if matches!(line.kind, ChatKind::Object(_)) && !line.source.is_nil() {
                for r in [cube, resp] {
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
            let galley = time_galley(ui.painter(), p, line);
            let (r, _) = ui.allocate_exact_size(egui::vec2(galley.size().x, row_h), egui::Sense::hover());
            crate::ui::widgets::paint_on_baseline(ui.painter(), galley, r.left(), baseline, p.muted_dim);
            ui.add_space(GAP);
        }
        let (text, italics) = if notice { (&line.text[..], false) } else { body(line) };
        let color = crate::ui::colors::keyword_color(line)
            .or(tone)
            .unwrap_or_else(|| text_color(line, p));
        ui.vertical(|ui| {
            ui.set_width((ui.available_width() - RIGHT).max(40.0));
            chat_text_marked(
                ui,
                p,
                env.emoji,
                world,
                env.want_names,
                text,
                TEXT,
                color,
                super::message_emphasis(line.kind, italics),
                mark,
            );
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
pub(super) mod tests {
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
            ChatKind::OWN,
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
                    compact_notices: on,
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

    /// The texts a row paints, with where their flat-footed letters stand
    /// on screen (the feet of "1", "H", "E", "L", "I", "T", "F", "M", "K"
    /// as epaint draws them: round letters and digits overshoot the
    /// baseline, OpenDyslexic's by a pixel).
    pub(in crate::ui::chat) fn feet(shapes: &[egui::epaint::ClippedShape], pixels: f32) -> Vec<(String, f32)> {
        use egui::emath::GuiRounding as _;
        fn texts(shape: &egui::Shape, out: &mut Vec<(egui::Pos2, std::sync::Arc<egui::Galley>)>) {
            match shape {
                egui::Shape::Text(t) => out.push((t.pos, t.galley.clone())),
                egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
                _ => {}
            }
        }
        let mut shown = Vec::new();
        for s in shapes {
            texts(&s.shape, &mut shown);
        }
        let snap = |v: f32| (v * pixels).round() / pixels;
        shown
            .into_iter()
            .filter_map(|(pos, g)| {
                let pos = pos.round_to_pixels(pixels);
                let row = g.rows.first()?;
                let foot = row
                    .row
                    .glyphs
                    .iter()
                    .filter(|gl| "1HELITFMK".contains(gl.chr) && gl.uv_rect.size.y > 0.0)
                    .map(|gl| pos.y + row.pos.y + snap(gl.pos.y + gl.uv_rect.offset.y) + gl.uv_rect.size.y)
                    .fold(f32::MIN, f32::max);
                (foot > f32::MIN).then(|| (g.text().to_owned(), foot))
            })
            .collect()
    }

    /// The human's report of 2026-10-24: the time of a compact row stood
    /// lower than its text. The time, the name and the text stand on one
    /// baseline, and the icon on the middle of the text's capitals, within
    /// half a pixel, with the four built-in fonts at the scales 1, 1.25 and
    /// 1.5, the time after the name or in its column.
    #[test]
    fn a_compact_rows_time_name_and_icon_stand_on_its_text() {
        let world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        // flat-footed letters to measure (`feet`)
        let mut said = line(ChatKind::Object(ChatType::Normal), "LIFT THE HELM");
        said.from = "HELM KIT".into();
        // 11:11 in Second Life time (15 January 2026, 19:11 UTC): a time of
        // flat-footed digits
        said.time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_768_504_260);
        let time = hhmm(said.time);
        assert_eq!(time, "11:11");
        for font in [
            crate::ui::fonts::INTER,
            crate::ui::fonts::NOTO,
            crate::ui::fonts::ROBOTO,
            crate::ui::fonts::OPEN_DYSLEXIC,
        ] {
            for pixels in [1.0, 1.25, 1.5] {
                for column in [None, Some(40.0)] {
                    let ctx = egui::Context::default();
                    let theme = crate::theme::Theme::default();
                    theme.apply(&ctx, 1.0);
                    crate::ui::fonts::apply(&ctx, font);
                    ctx.set_pixels_per_point(pixels);
                    let p = theme.palette();
                    // (the icons drawn, `paint_icon`)
                    let icons = crate::ui::icons::Icons::load(&ctx, None);
                    let mut emoji = crate::ui::emoji::Emoji::load();
                    let pics = std::collections::HashMap::new();
                    let (mut names, mut wanted) = (std::collections::HashSet::new(), std::collections::HashSet::new());
                    let mut out = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 200.0))),
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
                                compact_automated: true,
                                compact_notices: true,
                                menu: None,
                                bubbles: None,
                                previews: None,
                            };
                            let place = Place {
                                icon_center: 14.0,
                                text_left: 26.0,
                                time_column: column,
                            };
                            draw(ui, &mut env, &said, None, place, true);
                        },
                    );
                    out.textures_delta.clear();
                    let feet = feet(&out.shapes, pixels);
                    let case = format!("font={font}, pixels={pixels}, column={column:?}");
                    let foot = |text: &str| {
                        feet.iter()
                            .find(|(t, _)| t == text)
                            .map(|(_, y)| *y)
                            .unwrap_or_else(|| panic!("{case}: no {text}: {feet:?}"))
                    };
                    let body = foot("LIFT THE HELM");
                    for other in ["HELM KIT", time.as_str()] {
                        let y = foot(other);
                        assert!(
                            (y - body).abs() <= 0.5 + 1e-3,
                            "{case}: {other} stands at {y} and the text at {body}"
                        );
                    }
                    // the icon: its square mesh, on the middle of the text's
                    // capitals (an "H" of the text's font, as drawn)
                    let icon = out
                        .shapes
                        .iter()
                        .filter_map(|c| match &c.shape {
                            egui::Shape::Mesh(m) => {
                                let r = egui::Rect::from_points(&m.vertices.iter().map(|v| v.pos).collect::<Vec<_>>());
                                ((r.width() - ICON).abs() < 0.01 && (r.height() - ICON).abs() < 0.01).then_some(r)
                            }
                            _ => None,
                        })
                        .next()
                        .unwrap_or_else(|| panic!("{case}: the icon is drawn"));
                    let cap = {
                        let probe =
                            ctx.fonts_mut(|f| f.layout_no_wrap("H".into(), egui::FontId::proportional(TEXT), egui::Color32::PLACEHOLDER));
                        crate::ui::widgets::ink_rect(&probe).height()
                    };
                    let caps_center = body - cap / 2.0;
                    assert!(
                        (icon.center().y - caps_center).abs() <= 0.5 + 1e-3,
                        "{case}: the icon's middle {} against the capitals' {caps_center}",
                        icon.center().y
                    );
                }
            }
        }
    }

    /// A conversation's notices (calls, automatic replies) stay as they are
    /// unless the sub-option asks for them too; the nearby chat's follow
    /// the option alone.
    #[test]
    fn a_conversations_notices_follow_the_sub_option() {
        // (option, sub-option, nearby chat) → compacted
        let cases = [
            (false, false, true, false),
            (false, true, true, false),
            (false, true, false, false),
            (true, false, true, true),
            (true, false, false, false),
            (true, true, false, true),
            (true, true, true, true),
        ];
        for (on, im, nearby, expected) in cases {
            assert_eq!(compact_notices(on, im, nearby), expected, "{on} {im} {nearby}");
        }
    }

    /// Objects show their cube, notices the icon of what they are about.
    #[test]
    fn each_row_has_the_icon_of_its_kind() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        assert_eq!(icon(&world, &line(ChatKind::Object(ChatType::OwnerSay), "hi")), "box");
        assert_eq!(icon(&world, &line(ChatKind::ObjectIm, "hi")), "box");
        // a script error: the warning sign, in the interface's red (#1263)
        let p = crate::theme::Theme::default().palette();
        let error = line(ChatKind::Object(ChatType::Debug), "Script run-time error");
        assert_eq!(icon(&world, &error), WARNING);
        assert!(crate::ui::icons::embedded(WARNING));
        assert_eq!(object_glyph(&error, &p, p.muted), (WARNING, p.danger));
        assert_eq!(
            object_glyph(&line(ChatKind::Object(ChatType::Normal), "hi"), &p, p.muted),
            ("box", p.muted)
        );
        let topics = [
            (Topic::People, "user-filled"),
            (Topic::Teleport, "plane-departure-filled"),
            (Topic::Items, "box"),
            (Topic::Region, "world-filled"),
            (Topic::Media, "music"),
            (Topic::Call, "phone-filled"),
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
