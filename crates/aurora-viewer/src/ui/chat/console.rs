//! The nearby chat lines above the chat bar, Aurora's look: the recent lines
//! grouped by sender in cards like the Conversations window's messages (the
//! sender's picture, their name, the time and the volume on a head row, the
//! lines under it), on the same faded glass as the Legacy Viewer's one-line
//! look (`toasts`, Preferences › Aurora › Nearby chat lines). They let clicks
//! through to the world like the lines did: the name, the picture and the
//! links get their own small click areas (`bubble_sender_link`,
//! `bubble_link`). No status dot on the picture, as in the Conversations
//! thread (the human's choice).

use std::collections::{HashMap, HashSet};

use egui::Color32;

use super::{BubbleJob, JobLook, Link, bubble_link, bubble_sender_link, hhmm, line_extras, line_job, thread};
use crate::theme::Palette;
use crate::world::{ChatKind, ChatLine, World};
use aurora_net::ChatType;

/// The widest a card gets.
const WIDTH: f32 = 470.0;
/// Side of the sender's picture.
const PICTURE: f32 = 26.0;
/// Between the picture and the text.
const GAP: f32 = 8.0;
/// Text size of the lines.
const TEXT: f32 = 13.0;

/// The lines of one sender in a row (a notice is a group of its own).
pub(super) fn groups<'a>(lines: &[&'a ChatLine]) -> Vec<Vec<&'a ChatLine>> {
    let mut out: Vec<Vec<&'a ChatLine>> = Vec::new();
    for &line in lines {
        match out.last_mut() {
            Some(group) if same_sender(group[group.len() - 1], line) => group.push(line),
            _ => out.push(vec![line]),
        }
    }
    out
}

/// Two lines under one head: the same resident or object, said in a row
/// at the same volume (the SHOUTS / WHISPERS tag is the head's, #1323);
/// notices stand alone.
fn same_sender(a: &ChatLine, b: &ChatLine) -> bool {
    !a.kind.is_notice()
        && !b.kind.is_notice()
        && a.source == b.source
        && a.from == b.from
        && is_object(a.kind) == is_object(b.kind)
        && a.kind.is_own() == b.kind.is_own()
        && a.kind.volume() == b.kind.volume()
}

fn is_object(kind: ChatKind) -> bool {
    matches!(kind, ChatKind::Object(_) | ChatKind::ObjectIm)
}

/// How much of a line is still shown: it fades over its last 3 s.
fn alpha(seconds: f32, line: &ChatLine, now: std::time::SystemTime) -> f32 {
    let age = now.duration_since(line.time).map(|d| d.as_secs_f32()).unwrap_or(0.0);
    ((seconds - age) / 3.0).clamp(0.0, 1.0)
}

/// The colour of a head's name: ours in the theme's teal, an object's in
/// its kind's colour, a resident's in ink (the Conversations thread's).
fn name_color(p: &Palette, line: &ChatLine, world: &World) -> Color32 {
    if let Some(color) = crate::ui::colors::contact_sender_color(line, world) {
        return color;
    }
    if is_object(line.kind) {
        return crate::ui::colors::sender_color(line);
    }
    if line.kind.is_own() {
        return p.teal;
    }
    p.ink
}

/// The recent nearby chat in cards (see the module).
#[allow(clippy::too_many_arguments)]
pub(super) fn cards(
    ctx: &egui::Context,
    p: &Palette,
    world: &World,
    pics: &HashMap<uuid::Uuid, egui::TextureHandle>,
    want_names: &mut HashSet<uuid::Uuid>,
    wanted_pics: &mut HashSet<uuid::Uuid>,
    bottom: f32,
    seconds: f32,
    times: bool,
    compact_automated: bool,
    residents: bool,
) {
    let now = std::time::SystemTime::now();
    let mut recent: Vec<&ChatLine> = world
        .chat
        .iter()
        .rev()
        .filter(|l| residents || !super::said_by_resident(l))
        .take(6)
        .filter(|l| now.duration_since(l.time).map(|d| d.as_secs_f32() < seconds).unwrap_or(false))
        .collect();
    if recent.is_empty() {
        return;
    }
    recent.reverse();
    let mut links: Vec<(egui::Rect, Link, Color32)> = Vec::new();
    let mut names: Vec<(egui::Rect, &ChatLine)> = Vec::new();
    egui::Area::new(egui::Id::unique("chat_toasts"))
        .anchor(
            egui::Align2::LEFT_BOTTOM,
            egui::vec2(8.0, -(ctx.content_rect().bottom() - bottom) - 6.0),
        )
        .interactable(false)
        .order(egui::Order::Background)
        .show(ctx, |ui| {
            ui.set_max_width(WIDTH);
            ui.spacing_mut().item_spacing.y = 0.0;
            for group in groups(&recent) {
                let card_alpha = group.iter().map(|l| alpha(seconds, l, now)).fold(0.0_f32, f32::max);
                let tint = Color32::from_rgba_unmultiplied(p.bar.r(), p.bar.g(), p.bar.b(), (190.0 * card_alpha) as u8);
                let radius = ui.visuals().window_corner_radius;
                let glass = p.panel_alpha < 255;
                let mut slot = None;
                let small = super::object_row::compact(group[0], compact_automated);
                let card = egui::Frame::new()
                    .fill(if glass { Color32::TRANSPARENT } else { tint })
                    .corner_radius(radius)
                    .inner_margin(if small {
                        egui::Margin::symmetric(9, 3)
                    } else {
                        egui::Margin::symmetric(9, 6)
                    })
                    .show(ui, |ui| {
                        slot = Some(ui.painter().add(egui::Shape::Noop));
                        ui.multiply_opacity(card_alpha);
                        if small {
                            compact_card(
                                ui, p, world, want_names, &group, seconds, now, card_alpha, times, &mut links, &mut names,
                            );
                            return;
                        }
                        card(
                            ui,
                            p,
                            world,
                            pics,
                            want_names,
                            wanted_pics,
                            &group,
                            seconds,
                            now,
                            card_alpha,
                            times,
                            &mut links,
                            &mut names,
                        );
                    });
                if glass && let Some(slot) = slot {
                    ui.painter().set(
                        slot,
                        aurora_render::glass_panel_faded(card.response.rect, f32::from(radius.nw), tint, card_alpha),
                    );
                }
                ui.add_space(4.0);
            }
        });
    for (k, (rect, link, col)) in links.iter().enumerate() {
        bubble_link(ctx, p, world, k, *rect, link, *col);
    }
    for (k, (rect, line)) in names.into_iter().enumerate() {
        bubble_sender_link(ctx, p, world, k, rect, line);
    }
}

/// One card: the head (picture, name, tags, time) and the lines; a notice
/// is its line alone, in italics.
#[allow(clippy::too_many_arguments)]
fn card<'a>(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &World,
    pics: &HashMap<uuid::Uuid, egui::TextureHandle>,
    want_names: &mut HashSet<uuid::Uuid>,
    wanted_pics: &mut HashSet<uuid::Uuid>,
    group: &[&'a ChatLine],
    seconds: f32,
    now: std::time::SystemTime,
    card_alpha: f32,
    times: bool,
    links: &mut Vec<(egui::Rect, Link, Color32)>,
    names: &mut Vec<(egui::Rect, &'a ChatLine)>,
) {
    let first = group[0];
    if first.kind.is_notice() {
        body(
            ui,
            p,
            world,
            want_names,
            first,
            WIDTH - 18.0,
            alpha(seconds, first, now) / card_alpha.max(0.01),
            links,
        );
        return;
    }
    let text_w = WIDTH - 18.0 - PICTURE - GAP;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let (pic, _) = ui.allocate_exact_size(egui::vec2(PICTURE, PICTURE), egui::Sense::hover());
        picture(ui, p, pics, wanted_pics, first, pic);
        names.push((pic, first));
        ui.add_space(GAP);
        // (as wide as its text: a short line keeps a small card)
        ui.vertical(|ui| {
            ui.set_max_width(text_w);
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let font = egui::FontId::new(12.5, crate::ui::fonts::semibold(ui.ctx()));
                let galley = ui
                    .painter()
                    .layout(first.from.clone(), font, name_color(p, first, world), text_w * 0.6);
                let (r, _) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
                ui.painter().galley(r.min, galley, p.ink);
                names.push((r, first));
                // a member of the Aurora team (objects never are)
                let author = if first.kind.is_own() { world.agent_id } else { first.source };
                if !matches!(first.kind, ChatKind::Object(_) | ChatKind::ObjectIm) {
                    crate::ui::staff::after_name(ui, p, crate::world::staff::mark(world, &author), 9.5);
                }
                tags(ui, p, first);
                if times {
                    ui.label(egui::RichText::new(hhmm(first.time)).size(10.5).color(p.muted_dim));
                }
            });
            for &line in group {
                let fade = alpha(seconds, line, now) / card_alpha.max(0.01);
                body(ui, p, world, want_names, line, text_w, fade, links);
            }
        });
    });
}

/// An object's lines on small rows, as the Conversations window shows them
/// with the option on (`object_row.rs`): the cube, the name (its click and
/// menu inspect the object) and the text, no picture nor head.
#[allow(clippy::too_many_arguments)]
fn compact_card<'a>(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &World,
    want_names: &mut HashSet<uuid::Uuid>,
    group: &[&'a ChatLine],
    seconds: f32,
    now: std::time::SystemTime,
    card_alpha: f32,
    times: bool,
    links: &mut Vec<(egui::Rect, Link, Color32)>,
    names: &mut Vec<(egui::Rect, &'a ChatLine)>,
) {
    use egui::emath::GuiRounding as _;
    let size = super::object_row::TEXT;
    ui.spacing_mut().item_spacing.y = 1.0;
    for &line in group {
        let fade = alpha(seconds, line, now) / card_alpha.max(0.01);
        // the time before the line, set on its baseline (in the line's job,
        // a smaller section sits on the bottom of the row, lower)
        let time = times.then(|| super::object_row::time_galley(ui.painter(), p, line));
        let time_w = time.as_ref().map_or(0.0, |t| t.size().x + 5.0);
        // a notice's icon of its topic, as in the Conversations window (an
        // object's cube takes the room its job leaves before its name)
        let side = super::pic_side(size);
        let notice = line.kind.is_notice();
        let lead = time_w + if notice { side + 2.0 } else { 0.0 };
        let BubbleJob {
            job,
            links: line_links,
            badges,
            sender,
            name,
        } = line_job(
            ui.ctx(),
            line,
            p,
            world,
            want_names,
            WIDTH - 18.0 - lead,
            size,
            false,
            JobLook::Compact,
        );
        let base = super::bidi_base(&name, &line.from);
        let visual = ui.ctx().fonts_mut(|f| crate::ui::bidi::layout(f, job, base));
        let galley = visual.galley.clone();
        let (rect, _) = ui.allocate_exact_size(egui::vec2(lead + galley.size().x, galley.size().y), egui::Sense::hover());
        let mut painter = ui.painter().clone();
        painter.multiply_opacity(fade);
        let origin = (rect.min + egui::vec2(lead, 0.0)).round_to_pixels(painter.pixels_per_point());
        painter.galley(origin, galley.clone(), p.ink);
        let baseline = origin.y + crate::ui::widgets::galley_baseline(&painter, &galley);
        if let Some(time) = time {
            crate::ui::widgets::paint_on_baseline(&painter, time, rect.left(), baseline, p.muted_dim);
        }
        if notice {
            let middle = super::object_row::caps_middle(&painter, size, baseline);
            let r = egui::Rect::from_center_size(egui::pos2(rect.left() + time_w + side * 0.5, middle), egui::vec2(side, side));
            thread::paint_icon(&painter, super::object_row::icon(world, line), r.shrink(1.0), p.muted_dim);
        }
        // the cube in the room left before the name, on the capitals
        if let Some((at, _)) = sender {
            // at the left of the name, whatever its direction
            let c = visual.left_edge(name.clone().unwrap_or(at..at)).translate(origin.to_vec2());
            let middle = super::object_row::caps_middle(&painter, size, baseline);
            let r = egui::Rect::from_center_size(egui::pos2(c.min.x - 2.0 - side * 0.5, middle), egui::vec2(side, side));
            let (glyph, tint) = super::object_row::object_glyph(line, p, p.muted_dim);
            thread::paint_icon(&painter, glyph, r.shrink(1.0), tint);
        }
        if let Some(range) = name
            && let Some(r) = visual.range_rects(origin, range).first().copied()
        {
            names.push((r, line));
        }
        let col = crate::ui::colors::keyword_color(line)
            .unwrap_or_else(|| super::object_row::text_color(line, p))
            .gamma_multiply(fade);
        line_extras(ui, p, world, &visual, origin, size, fade, badges, line_links, col, links);
    }
}

/// The tags after a name: IM, shouts, whispers (the Conversations thread's).
fn tags(ui: &mut egui::Ui, p: &Palette, line: &ChatLine) {
    match line.kind {
        ChatKind::Im => thread::tag(ui, aurora_i18n::tr!("people-sheet-im"), Some(p.raised), p.muted),
        _ => match line.kind.volume() {
            Some(ChatType::Shout) => thread::tag(ui, aurora_i18n::tr!("ui-chat-tag-shouts"), None, p.warn),
            Some(ChatType::Whisper) => thread::tag(ui, aurora_i18n::tr!("ui-chat-tag-whispers"), None, p.muted),
            _ => {}
        },
    }
}

/// The sender's picture: the resident's profile picture in a disc (initials
/// while it loads), an object's cube on a tile.
fn picture(
    ui: &egui::Ui,
    p: &Palette,
    pics: &HashMap<uuid::Uuid, egui::TextureHandle>,
    wanted_pics: &mut HashSet<uuid::Uuid>,
    line: &ChatLine,
    rect: egui::Rect,
) {
    if is_object(line.kind) {
        ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.raised);
        let (glyph, tint) = super::object_row::object_glyph(line, p, crate::ui::colors::sender_color(line));
        thread::paint_icon(ui.painter(), glyph, rect.shrink(6.0), tint);
        return;
    }
    let id = line.source;
    if !id.is_nil() {
        wanted_pics.insert(id);
    }
    crate::ui::widgets::avatar_disc(ui, p, rect, pics.get(&id), &line.from, false);
}

/// A line's text under the head, with its links and their badges; `fade`
/// is its own fading within the card's.
#[allow(clippy::too_many_arguments)]
fn body(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &World,
    want_names: &mut HashSet<uuid::Uuid>,
    line: &ChatLine,
    width: f32,
    fade: f32,
    links: &mut Vec<(egui::Rect, Link, Color32)>,
) {
    let BubbleJob {
        job,
        links: line_links,
        badges,
        name,
        ..
    } = line_job(ui.ctx(), line, p, world, want_names, width, TEXT, false, JobLook::Body);
    let base = super::bidi_base(&name, &line.from);
    let visual = ui.ctx().fonts_mut(|f| crate::ui::bidi::layout(f, job, base));
    let galley = visual.galley.clone();
    let (rect, _) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
    let mut painter = ui.painter().clone();
    painter.multiply_opacity(fade);
    painter.galley(rect.min, galley.clone(), p.ink);
    let col = super::message_color(line, p, world).gamma_multiply(fade);
    line_extras(ui, p, world, &visual, rect.min, TEXT, fade, badges, line_links, col, links);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::Origin;

    fn line(kind: ChatKind, from: &str, source: u128) -> ChatLine {
        ChatLine {
            time: std::time::SystemTime::now(),
            from: from.into(),
            text: "hi".into(),
            kind,
            source: uuid::Uuid::from_u128(source),
            origin: Origin::Live,
        }
    }

    /// Lines of one sender said in a row share a card; another sender, an
    /// object of the same name or a notice starts a new one.
    #[test]
    fn lines_in_a_row_of_one_sender_share_a_card() {
        let said = ChatKind::Local(ChatType::Normal);
        let a1 = line(said, "Nova", 1);
        let a2 = line(said, "Nova", 1);
        let b = line(said, "Orion", 2);
        let notice = line(ChatKind::System, "", 0);
        let object = line(ChatKind::Object(ChatType::Normal), "Nova", 1);
        let a3 = line(said, "Nova", 1);
        let lines = [&a1, &a2, &b, &notice, &object, &a3];
        let sizes: Vec<usize> = groups(&lines).iter().map(Vec::len).collect();
        assert_eq!(sizes, vec![2, 1, 1, 1, 1]);
    }

    /// The human's report of 2026-10-24: above the chat bar, the time of an
    /// object's compact line stood lower than its text (a smaller section
    /// of one job sits on the bottom of the row). The time stands on the
    /// line's baseline and the cube on the middle of its capitals, within
    /// half a pixel, with the four built-in fonts at the scales 1, 1.25 and
    /// 1.5.
    #[test]
    fn a_compact_lines_time_and_cube_stand_on_its_text() {
        let mut said = line(ChatKind::Object(ChatType::Normal), "HELM KIT", 9);
        said.text = "LIFT THE HELM".into();
        // 11:11 in Second Life time (15 January 2026, 19:11 UTC), shown as
        // long as the lines last below: a time of flat-footed digits
        said.time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_768_504_260);
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.chat.push_back(said);
        // and a notice about a teleport, said after it
        let mut notice = line(ChatKind::System, "", 0);
        notice.text = "LIFT OFF".into();
        notice.time = world.chat[0].time;
        world.notice_topics.note(&notice, crate::world::notice_topics::Topic::Teleport);
        world.chat.push_back(notice);
        let time = hhmm(world.chat[0].time);
        for font in [
            crate::ui::fonts::INTER,
            crate::ui::fonts::NOTO,
            crate::ui::fonts::ROBOTO,
            crate::ui::fonts::OPEN_DYSLEXIC,
        ] {
            for pixels in [1.0, 1.25, 1.5] {
                let ctx = egui::Context::default();
                crate::theme::Theme::default().apply(&ctx, 1.0);
                crate::ui::fonts::apply(&ctx, font);
                ctx.set_pixels_per_point(pixels);
                let p = crate::theme::Theme::default().palette();
                let _icons = crate::ui::icons::Icons::load(&ctx, None);
                // (an area shows from its second frame)
                let run = || {
                    ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                            ..Default::default()
                        },
                        |ui| {
                            cards(
                                ui.ctx(),
                                &p,
                                &world,
                                &HashMap::new(),
                                &mut HashSet::new(),
                                &mut HashSet::new(),
                                700.0,
                                1e10,
                                true,
                                true,
                                true,
                            )
                        },
                    )
                };
                let mut first = run();
                first.textures_delta.clear();
                let mut out = run();
                out.textures_delta.clear();
                let case = format!("font={font}, pixels={pixels}");
                let feet = super::super::object_row::tests::feet(&out.shapes, pixels);
                let foot = |text: &str| {
                    feet.iter()
                        .find(|(t, _)| t.contains(text))
                        .map(|(_, y)| *y)
                        .unwrap_or_else(|| panic!("{case}: no {text}: {feet:?}"))
                };
                let text = foot("LIFT THE HELM");
                assert_eq!(time, "11:11");
                let at = foot(&time);
                assert!((at - text).abs() <= 0.5 + 1e-3, "{case}: the time at {at}, the text at {text}");
                let side = super::super::pic_side(super::super::object_row::TEXT) - 2.0;
                // the object's cube, then the notice's icon of its topic
                let icons: Vec<egui::Rect> = out
                    .shapes
                    .iter()
                    .filter_map(|c| match &c.shape {
                        egui::Shape::Mesh(m) if !m.vertices.is_empty() => {
                            let r = egui::Rect::from_points(&m.vertices.iter().map(|v| v.pos).collect::<Vec<_>>());
                            ((r.width() - side).abs() < 0.01 && (r.height() - side).abs() < 0.01).then_some(r)
                        }
                        _ => None,
                    })
                    .collect();
                assert_eq!(icons.len(), 2, "{case}: the cube and the notice's icon, {icons:?}");
                let cube = icons[0];
                let cap = {
                    let probe = ctx.fonts_mut(|f| {
                        f.layout_no_wrap(
                            "H".into(),
                            egui::FontId::proportional(super::super::object_row::TEXT),
                            egui::Color32::PLACEHOLDER,
                        )
                    });
                    crate::ui::widgets::ink_rect(&probe).height()
                };
                assert!(
                    (cube.center().y - (text - cap / 2.0)).abs() <= 0.5 + 1e-3,
                    "{case}: the cube's middle {} against the capitals' {}",
                    cube.center().y,
                    text - cap / 2.0
                );
                let notice = foot("LIFT OFF");
                assert!(
                    (icons[1].center().y - (notice - cap / 2.0)).abs() <= 0.5 + 1e-3,
                    "{case}: the notice's icon {} against its capitals' {}",
                    icons[1].center().y,
                    notice - cap / 2.0
                );
            }
        }
    }

    /// Our whisper then our shout: two heads, each with its tag (#1323).
    #[test]
    fn a_line_at_another_volume_gets_its_own_head() {
        let whisper = line(ChatKind::Own(ChatType::Whisper), "Me", 0);
        let shout = line(ChatKind::Own(ChatType::Shout), "Me", 0);
        let said = line(ChatKind::OWN, "Me", 0);
        assert!(!same_sender(&whisper, &shout));
        assert!(!same_sender(&shout, &said));
        assert!(same_sender(&said, &line(ChatKind::OWN, "Me", 0)));
    }

    /// A card's body leaves the name to the head, except an emote's, whose
    /// name is part of its sentence.
    #[test]
    fn the_body_leaves_the_name_to_the_head_except_an_emotes() {
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
        let p = crate::theme::Theme::default().palette();
        let world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let job = |l: &ChatLine| {
            line_job(&ctx, l, &p, &world, &mut HashSet::new(), 400.0, TEXT, true, JobLook::Body)
                .job
                .text
        };
        let mut said = line(ChatKind::Local(ChatType::Shout), "Nova", 1);
        said.text = "Doors open!".into();
        assert_eq!(job(&said), "Doors open!");
        let mut emote = line(ChatKind::Local(ChatType::Normal), "Orion", 2);
        emote.text = "/me waves".into();
        assert_eq!(job(&emote), "Orion waves");
    }

    /// The picture and the name of a card open the profile, and offer the
    /// sender's menu of the Conversations thread on a right click; its
    /// text, and the room beside a short card, let the click through to
    /// the world.
    #[test]
    fn a_cards_picture_and_name_open_the_profile_or_the_menu_and_the_rest_reaches_the_world() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let mut notice = line(ChatKind::System, "", 0);
        notice.text = "Nova est en ligne.".into();
        let mut said = line(ChatKind::Local(ChatType::Normal), "Orion Exemple", 7);
        said.text = "Bienvenue dans la démo".into();
        world.chat.extend([notice, said]);
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
                    cards(
                        ui.ctx(),
                        &p,
                        &world,
                        &HashMap::new(),
                        &mut HashSet::new(),
                        &mut HashSet::new(),
                        800.0,
                        20.0,
                        false,
                        false,
                        true,
                    )
                },
            );
            out.textures_delta.clear();
            // the texts on screen, to find a menu's items
            out.shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
                    _ => None,
                })
                .collect::<Vec<String>>()
        };
        frame(Vec::new());
        frame(Vec::new());
        let area = |k: usize| ctx.memory(|m| m.area_rect(egui::Id::unique(("chat_toast_sender", k))));
        let picture = area(0).expect("the picture is clickable");
        let name = area(1).expect("the name is clickable");
        // the notice has neither
        assert!(area(2).is_none());
        let click = |at: egui::Pos2, pressed: bool| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        for at in [picture.center(), name.center()] {
            frame(vec![egui::Event::PointerMoved(at), click(at, true)]);
            frame(vec![click(at, false)]);
            assert_eq!(crate::ui::profile::take_open_request(&ctx), Some(uuid::Uuid::from_u128(7)));
        }
        // a right click offers the sender's menu of the Conversations thread
        let right = |at: egui::Pos2, pressed: bool| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: Default::default(),
        };
        let im = aurora_i18n::tr!("ui-context-send-im").to_string();
        for at in [picture.center(), name.center()] {
            frame(vec![egui::Event::PointerMoved(at), right(at, true)]);
            frame(vec![right(at, false)]);
            let texts = frame(Vec::new());
            assert!(egui::Popup::is_any_open(&ctx), "the sender's menu");
            assert!(texts.contains(&im), "its IM item: {texts:?}");
            assert_eq!(crate::ui::profile::take_open_request(&ctx), None, "a right click opens no profile");
            frame(vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }]);
            frame(Vec::new());
            assert!(!egui::Popup::is_any_open(&ctx));
        }
        let text = egui::pos2(name.left() + 20.0, name.bottom() + 8.0);
        frame(vec![egui::Event::PointerMoved(text)]);
        assert!(!ctx.is_pointer_over_egui(), "the message lets the pointer through");
        let beside = egui::pos2(WIDTH + 40.0, name.center().y);
        frame(vec![egui::Event::PointerMoved(beside)]);
        assert!(!ctx.is_pointer_over_egui(), "beside the card");
    }
}
