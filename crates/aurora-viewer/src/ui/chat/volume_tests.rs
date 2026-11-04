//! Inspect the actual painted text in every chat renderer (#1323).

use super::*;
use crate::ui::widgets::elide_tests::Frames;
use crate::world::{ChatLine, Origin};

#[test]
fn volume_emphasis_reaches_conversations_toasts_links_emoji_and_search() {
    let mut frames = Frames::new();
    crate::theme::Theme::default().apply(&frames.ctx, 1.0);
    crate::ui::fonts::apply(&frames.ctx, crate::ui::fonts::INTER);
    let p = crate::theme::Theme::default().palette();
    let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    let icons = crate::ui::icons::Icons::default();
    let mut emoji = crate::ui::emoji::Emoji::load();
    let pics = HashMap::new();
    let (mut names, mut wanted) = (HashSet::new(), HashSet::new());
    let link = format!(
        "Volume 😀 [WebVolume https://example.com/volume] [AgentVolume secondlife:///app/agent/{}/about]",
        uuid::Uuid::from_u128(42)
    );
    for (renderer, label) in [
        "Aurora",
        "Compact",
        "Bubbles",
        "Lines",
        "compact objects",
        "cards",
        "pills",
        "console",
        "compact cards",
    ]
    .into_iter()
    .enumerate()
    {
        for volume in [ChatType::Normal, ChatType::Whisper, ChatType::Shout] {
            for kind in [ChatKind::Own(volume), ChatKind::Local(volume), ChatKind::Object(volume)] {
                for text in ["Volume text", link.as_str(), "/me Volume text"] {
                    let line = ChatLine {
                        time: std::time::SystemTime::now(),
                        from: "Speaker".into(),
                        text: text.into(),
                        kind,
                        source: uuid::Uuid::from_u128(7),
                        origin: Origin::Live,
                    };
                    world.chat.clear();
                    world.chat.push_back(line.clone());
                    let mut draw = |ui: &mut egui::Ui| {
                        if renderer >= 5 {
                            toasts(
                                ui.ctx(),
                                &p,
                                &world,
                                &pics,
                                &mut names,
                                &mut wanted,
                                850.0,
                                60.0,
                                false,
                                renderer == 6,
                                matches!(renderer, 5 | 8),
                                renderer == 8,
                                true,
                            );
                            return;
                        }
                        let mut env = LineEnv {
                            p: &p,
                            icons: &icons,
                            emoji: &mut emoji,
                            pics: &pics,
                            world: &world,
                            want_names: &mut names,
                            wanted: &mut wanted,
                            own_name: "Speaker",
                            plain_history: false,
                            compact_automated: renderer == 4,
                            compact_notices: false,
                            menu: None,
                            bubbles: (renderer == 2).then_some(true),
                            previews: None,
                        };
                        let look = view::Look {
                            header: true,
                            first: true,
                            tail: true,
                            ..Default::default()
                        };
                        let mark = (text == link).then_some(Mark {
                            needle: "volume",
                            wash: p.raised,
                        });
                        if matches!(renderer, 0 | 1 | 4) {
                            let mut out = Vec::new();
                            let mut thread = thread::Thread {
                                compact: renderer == 1,
                                times: false,
                                moderators: None,
                                me: &[],
                                out: &mut out,
                            };
                            thread::draw(ui, &mut env, &mut thread, &line, look, mark, true);
                        } else {
                            draw_line(ui, &mut env, &line, look, mark);
                        }
                    };
                    frames.run(vec![], &mut draw);
                    let shapes = frames.shapes(vec![], &mut draw);
                    let mut checked = 0;
                    for shape in shapes {
                        let egui::Shape::Text(t) = shape else { continue };
                        for section in &t.galley.job.sections {
                            let section_text = &t.galley.job.text[section.byte_range.start.0..section.byte_range.end.0];
                            if !section_text.contains("Volume") {
                                continue;
                            }
                            checked += 1;
                            let case = format!("renderer={label}, kind={kind:?}, text={section_text}");
                            assert_eq!(
                                section.format.italics,
                                volume == ChatType::Whisper || text.starts_with("/me "),
                                "{case}"
                            );
                            let family = if volume == ChatType::Shout {
                                crate::ui::fonts::semibold(&frames.ctx)
                            } else {
                                egui::FontFamily::Proportional
                            };
                            assert_eq!(section.format.font_id.family, family, "{case}");
                            if text == "Volume text" && matches!(renderer, 0 | 5) {
                                let color = match volume {
                                    ChatType::Whisper => p.muted,
                                    ChatType::Shout => p.ink,
                                    _ => crate::ui::colors::chat_color(&line, &world),
                                };
                                assert_eq!(section.format.color, color, "{case}");
                            }
                        }
                    }
                    assert!(checked > 0, "renderer {renderer} painted no message for {kind:?}");
                }
            }
        }
    }
}

#[test]
fn object_volume_changes_start_a_new_console_card() {
    let line = |volume| ChatLine {
        time: std::time::SystemTime::now(),
        from: "Speaker".into(),
        text: "Volume text".into(),
        kind: ChatKind::Object(volume),
        source: uuid::Uuid::from_u128(7),
        origin: Origin::Live,
    };
    let lines = [line(ChatType::Normal), line(ChatType::Whisper), line(ChatType::Shout)];
    assert_eq!(console::groups(&lines.iter().collect::<Vec<_>>()).len(), 3);
}
