//! Notification names use the same profile/SLapp dispatch as chat links.
//! Typed identity metadata survives history without altering server text.

use crate::theme::Palette;
use crate::world::notifications::{NameLink, NameTarget};
use egui::{Color32, RichText};
use std::ops::Range;

fn spans(text: &str, links: &[NameLink]) -> Vec<(Range<usize>, NameTarget)> {
    let mut candidates: Vec<_> = links
        .iter()
        .filter(|l| !l.name.is_empty())
        .flat_map(|link| {
            text.match_indices(&link.name)
                .filter(move |(start, _)| {
                    let end = start + link.name.len();
                    let word = |c: char| c.is_alphanumeric() || c == '_';
                    !(text[..*start].chars().next_back().is_some_and(word) && link.name.chars().next().is_some_and(word)
                        || text[end..].chars().next().is_some_and(word) && link.name.chars().next_back().is_some_and(word))
                })
                .map(move |(start, _)| (start..start + link.name.len(), link.target))
        })
        .collect();
    candidates.sort_by_key(|(r, _)| (r.start, std::cmp::Reverse(r.len())));
    let mut end = 0;
    candidates.retain(|(r, _)| {
        if r.start < end {
            return false;
        }
        end = r.end;
        true
    });
    candidates
}

pub(crate) fn open(ctx: &egui::Context, target: NameTarget) {
    use crate::ui::context::{CtxAction, request};
    match target {
        NameTarget::Avatar(id) => crate::ui::profile::request_open(ctx, id),
        NameTarget::Group(id) => request(ctx, CtxAction::AppLink(crate::slapp::AppLink::Group(id))),
        NameTarget::Experience(id) => request(ctx, CtxAction::AppLink(crate::slapp::AppLink::Experience(id))),
        NameTarget::Object(id) => request(ctx, CtxAction::Inspect(id)),
    }
}

pub(super) fn person_body(
    ui: &mut egui::Ui,
    p: &Palette,
    emoji: &mut crate::ui::emoji::Emoji,
    text: &str,
    links: &[NameLink],
) -> Option<NameTarget> {
    let spans = spans(text, links);
    if spans.is_empty() {
        emoji.rich_text(ui, text, 12.0, p.muted, false);
        return None;
    }
    let mut hovered = None;
    crate::ui::emoji::text_rows(ui, 12.0, |ui| {
        let mut at = 0;
        for (range, target) in spans {
            emoji.inline(ui, &text[at..range.start], 12.0, p.muted, false);
            let name = &text[range.clone()];
            hovered = hovered.or(show(
                ui,
                p,
                name,
                &[NameLink {
                    name: name.to_owned(),
                    target,
                }],
                12.0,
                p.muted,
                false,
                false,
            ));
            at = range.end;
        }
        emoji.inline(ui, &text[at..], 12.0, p.muted, false);
    });
    hovered
}

/// Return the name under the pointer, also usable by cards whose overall
/// click response owns the hit. Text outside those exact spans stays plain.
#[allow(clippy::too_many_arguments)]
pub(crate) fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    text: &str,
    links: &[NameLink],
    size: f32,
    color: Color32,
    truncate: bool,
    activate: bool,
) -> Option<NameTarget> {
    let spans = spans(text, links);
    if spans.is_empty() {
        let rich = RichText::new(text).size(size).color(color);
        if truncate {
            ui.add(egui::Label::new(rich.strong()).truncate()).on_hover_text(text);
        } else {
            ui.add(egui::Label::new(rich).wrap().selectable(true));
        }
        return None;
    }
    let mut job = egui::text::LayoutJob::default();
    let format = egui::TextFormat {
        font_id: egui::FontId::proportional(size),
        color,
        ..Default::default()
    };
    let link_format = egui::TextFormat {
        color: p.violet_light,
        underline: egui::Stroke::new(1.0, p.violet_light),
        ..format.clone()
    };
    let mut at = 0;
    let mut ranges = Vec::new();
    for (range, target) in spans {
        job.append(&text[at..range.start], 0.0, format.clone());
        let first = text[..range.start].chars().count();
        ranges.push((first..first + text[range.clone()].chars().count(), target));
        job.append(&text[range.clone()], 0.0, link_format.clone());
        at = range.end;
    }
    job.append(&text[at..], 0.0, format);
    job.wrap.max_width = ui.available_width().max(1.0);
    if truncate {
        job.wrap = egui::text::TextWrapping::truncate_at_width(job.wrap.max_width);
    }
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let response = ui.add(
        egui::Label::new(galley.clone())
            .selectable(activate && !truncate)
            .sense(if activate { egui::Sense::click() } else { egui::Sense::hover() }),
    );
    let hovered = ui
        .input(|i| i.pointer.hover_pos())
        .filter(|pos| response.rect.intersect(ui.clip_rect()).contains(*pos))
        .and_then(|pos| {
            ranges.into_iter().find_map(|(range, target)| {
                super::super::chat::range_rects(&galley, response.rect.min, range)
                    .iter()
                    .any(|r| r.intersect(response.rect).contains(pos))
                    .then_some(target)
            })
        });
    if let Some(target) = hovered {
        response
            .clone()
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(aurora_i18n::tr!("common-view-profile"));
        if activate && response.clicked() {
            open(ui.ctx(), target);
        }
    }
    hovered
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn names_match_whole_unicode_names_and_prefer_the_longest_identity() {
        let a = NameTarget::Avatar(Uuid::from_u128(2));
        let b = NameTarget::Group(Uuid::from_u128(3));
        let links = [
            NameLink {
                name: "Éloïse".into(),
                target: a,
            },
            NameLink {
                name: "Éloïse Exemple".into(),
                target: b,
            },
        ];
        let text = "Éloïse Exemple, Éloïse et ÉloïseTest";
        let found = spans(text, &links);
        assert_eq!(
            found.iter().map(|(r, t)| (&text[r.clone()], *t)).collect::<Vec<_>>(),
            [("Éloïse Exemple", b), ("Éloïse", a)]
        );
    }

    #[test]
    fn clicking_each_identity_opens_its_native_window_without_an_offer_answer() {
        let id = Uuid::from_u128(2);
        for target in [
            NameTarget::Avatar(id),
            NameTarget::Group(id),
            NameTarget::Experience(id),
            NameTarget::Object(id),
        ] {
            for truncate in [false, true] {
                let ctx = egui::Context::default();
                let theme = crate::theme::Theme::default();
                theme.apply(&ctx, 1.0);
                let p = theme.palette();
                let value = "Un nom : Éloïse Exemple, suite du texte";
                let links = [NameLink {
                    name: "Éloïse Exemple".into(),
                    target,
                }];
                let mut time = 0.0;
                let mut frame = |events| {
                    time += 0.1;
                    let mut out = ctx.run_ui(
                        egui::RawInput {
                            time: Some(time),
                            events,
                            ..Default::default()
                        },
                        |ui| {
                            ui.set_width(190.0);
                            show(ui, &p, value, &links, 12.0, p.ink, truncate, true);
                        },
                    );
                    out.textures_delta.clear();
                    out
                };
                let mut output = frame(vec![]);
                for _ in 0..3 {
                    output = frame(vec![]);
                }
                let pos = output
                    .shapes
                    .iter()
                    .find_map(|s| {
                        let egui::Shape::Text(t) = &s.shape else {
                            return None;
                        };
                        if t.galley.text() != value {
                            return None;
                        }
                        crate::ui::chat::range_rects(&t.galley, t.pos, 9..15).first().map(|r| r.center())
                    })
                    .expect("visible linked name");
                frame(vec![egui::Event::PointerMoved(pos)]);
                for pressed in [true, false] {
                    frame(vec![egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    }]);
                }
                let profile = crate::ui::profile::take_open_request(&ctx);
                let actions = crate::ui::context::take_requests(&ctx);
                match target {
                    NameTarget::Avatar(id) => assert_eq!(profile, Some(id)),
                    NameTarget::Group(id) => assert!(
                        matches!(&actions[..], [crate::ui::context::CtxAction::AppLink(crate::slapp::AppLink::Group(key))] if *key == id)
                    ),
                    NameTarget::Experience(id) => assert!(
                        matches!(&actions[..], [crate::ui::context::CtxAction::AppLink(crate::slapp::AppLink::Experience(key))] if *key == id)
                    ),
                    NameTarget::Object(id) => assert!(matches!(&actions[..], [crate::ui::context::CtxAction::Inspect(key)] if *key == id)),
                }
            }
        }
    }

    #[test]
    fn a_message_name_opens_the_profile_instead_of_the_conversation() {
        use crate::ui::notifications::{ToastEnv, person_card};
        use crate::world::notifications::{NameTarget, Notifications};
        use std::collections::{HashMap, HashSet};
        let id = Uuid::from_u128(2);
        for in_body in [false, true] {
            let ctx = egui::Context::default();
            let theme = crate::theme::Theme::default();
            theme.apply(&ctx, 1.0);
            let p = theme.palette();
            let icons = crate::ui::icons::Icons::default();
            let mut emoji = crate::ui::emoji::Emoji::load();
            let mut list = Notifications::default();
            let notification = list.push_message(
                id,
                id,
                if in_body { "Demo group" } else { "Jane Doe" },
                if in_body { "Jane Doe: hello" } else { "hello" },
                true,
            );
            list.link_name(notification, "Jane Doe", NameTarget::Avatar(id));
            let pics = HashMap::new();
            let mut wanted = HashSet::new();
            let world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
            let mut time = 0.0;
            let mut frame = |events| {
                time += 0.1;
                let mut answer = None;
                let mut out = ctx.run_ui(
                    egui::RawInput {
                        time: Some(time),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        ui.set_width(390.0);
                        let mut env = ToastEnv {
                            pics: &pics,
                            emoji: &mut emoji,
                            hidden: false,
                            message_seconds: 30.0,
                            restart: None,
                            presence_at: Default::default(),
                            classic_bars: false,
                        };
                        answer = person_card(ui, &p, &icons, &world, &mut env, &mut wanted, &list.list[0], false);
                    },
                );
                out.textures_delta.clear();
                (out, answer)
            };
            let mut output = frame(vec![]).0;
            for _ in 0..3 {
                output = frame(vec![]).0;
            }
            let pos = output
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.text() == "Jane Doe" => Some(t.pos + t.galley.size() * 0.5),
                    _ => None,
                })
                .expect("linked name visible");
            frame(vec![egui::Event::PointerMoved(pos)]);
            for pressed in [true, false] {
                let (_, answer) = frame(vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }]);
                assert!(answer.is_none(), "name does not answer/open the notification");
            }
            assert_eq!(crate::ui::profile::take_open_request(&ctx), Some(id));
        }
    }
}
