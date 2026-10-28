//! Exercise visible actions without a hover, including marker clicks and
//! transcript copying. A custom name cannot hide an IM/payment target.

use super::*;
use action_marker::Action;

fn baselines(shape: &egui::Shape, ys: &mut Vec<f32>) {
    match shape {
        egui::Shape::Text(t) => {
            for row in &t.galley.rows {
                // The transparent marker slot is the only M in this fixture.
                for g in row.glyphs.iter().filter(|g| g.chr != 'M' && !g.chr.is_whitespace()) {
                    ys.push(t.pos.y + row.pos.y + g.pos.y);
                }
            }
        }
        egui::Shape::Vec(v) => v.iter().for_each(|s| baselines(s, ys)),
        _ => {}
    }
}

#[test]
fn pasted_native_links_align_with_the_surrounding_sentence() {
    let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    let id = crate::demo::DEMO_LOUP;
    world.social.names.insert(id, "Resident".into());
    for font in [crate::ui::fonts::INTER, crate::ui::fonts::NOTO, crate::ui::fonts::ROBOTO] {
        for (size, scale, pixels) in [(14.0, 1.0, 1.0), (14.0, 1.4, 1.5), (20.0, 1.0, 1.5)] {
            let ctx = egui::Context::default();
            let theme = crate::theme::Theme::default();
            theme.apply(&ctx, 1.0);
            crate::ui::fonts::apply(&ctx, font);
            crate::ui::fonts::set_scale(&ctx, scale);
            ctx.set_pixels_per_point(pixels);
            let p = theme.palette();
            let mut emoji = crate::ui::emoji::Emoji::load();
            for verb in ["about", "im", "pay", "unknown"] {
                let url = if verb == "unknown" {
                    "secondlife:///app/frobnicate/1".to_owned()
                } else {
                    format!("secondlife:///app/agent/{id}/{verb}")
                };
                let text = format!("Salut tu vas {url} bien ?");
                let mut out = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1800.0, 300.0))),
                        ..Default::default()
                    },
                    |ui| chat_text(ui, &p, &mut emoji, &world, &mut HashSet::new(), &text, size, p.ink, false),
                );
                out.textures_delta.clear();
                let mut ys = Vec::new();
                out.shapes.iter().for_each(|s| baselines(&s.shape, &mut ys));
                assert!(!ys.is_empty());
                let min = ys.iter().copied().fold(f32::INFINITY, f32::min);
                let max = ys.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                assert!(
                    max - min < 0.1,
                    "a pasted link shares its sentence's baseline: font={font}, size={size}, scale={scale}, pixels={pixels}, verb={verb}, ys={ys:?}"
                );
            }
        }
    }
}

#[test]
fn pasted_unknown_uri_shows_a_readable_label_and_keeps_its_target() {
    let world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    let url = "secondlife:///app/frobnicate/1";
    let sentence = format!("Salut tu vas {url} bien ?");
    let parts = pieces(&sentence, |s| match s {
        Seg::App(u, link) => app_links::label(&world, &mut HashSet::new(), u, link),
        _ => unreachable!(),
    });
    assert_eq!(
        parts,
        [
            ("Salut tu vas ".into(), None),
            (
                aurora_i18n::tr!("ui-chat-applink-unsupported-label").into(),
                Some(Link::App(url.into(), crate::slapp::AppLink::Unsupported))
            ),
            (" bien ?".into(), None)
        ]
    );
    let labelled = format!("[{url} Custom name]");
    assert_eq!(pieces(&labelled, |_| unreachable!())[0].0, "Custom name");
}

#[test]
fn pasted_native_links_preserve_console_line_height() {
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    crate::ui::fonts::apply(&ctx, crate::ui::fonts::INTER);
    let p = theme.palette();
    let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    let id = crate::demo::DEMO_LOUP;
    world.social.names.insert(id, "Resident".into());
    let mut line = crate::world::ChatLine {
        time: std::time::SystemTime::now(),
        from: "Resident".into(),
        source: id,
        kind: ChatKind::OWN,
        origin: Origin::Live,
        text: String::new(),
    };
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
        let family = crate::ui::fonts::semibold(ui.ctx());
        for look in [JobLook::Line, JobLook::Pill(&family), JobLook::Body] {
            line.text = "Salut tu vas Resident bien ?".into();
            let plain = line_job(&line, &p, &world, &mut HashSet::new(), 1800.0, 14.0, false, look);
            let plain = ui.fonts_mut(|f| f.layout_job(plain.job));
            for verb in ["about", "im", "pay"] {
                line.text = format!("Salut tu vas secondlife:///app/agent/{id}/{verb} bien ?");
                let linked = line_job(&line, &p, &world, &mut HashSet::new(), 1800.0, 14.0, false, look);
                let linked = ui.fonts_mut(|f| f.layout_job(linked.job));
                assert_eq!(linked.rows.len(), 1);
                assert_eq!(linked.size().y, plain.size().y, "a marker must not raise a console row");
            }
        }
    });
    out.textures_delta.clear();
}

fn visible(shape: &egui::Shape, texts: &mut Vec<String>) {
    match shape {
        egui::Shape::Text(t) => texts.push(
            t.galley
                .job
                .sections
                .iter()
                .filter(|s| s.format.color.a() != 0)
                .map(|s| &t.galley.job.text[s.byte_range.start.0..s.byte_range.end.0])
                .collect(),
        ),
        egui::Shape::Vec(v) => v.iter().for_each(|s| visible(s, texts)),
        _ => {}
    }
}

fn icon_count(shape: &egui::Shape) -> usize {
    match shape {
        egui::Shape::Mesh(_) => 1,
        egui::Shape::Vec(v) => v.iter().map(icon_count).sum(),
        _ => 0,
    }
}

fn check_action(ctx: &egui::Context, verb: &str, id: uuid::Uuid) {
    if verb == "about" {
        assert_eq!(crate::ui::profile::take_open_request(ctx), Some(id));
        assert!(crate::ui::context::take_requests(ctx).is_empty());
    } else {
        let requests = crate::ui::context::take_requests(ctx);
        let expected = crate::slapp::parse(&format!("secondlife:///app/agent/{id}/{verb}")).expect("known action");
        assert!(
            matches!(&requests[..], [CtxAction::AppLink(actual)] if *actual == expected),
            "{requests:?}"
        );
        assert_eq!(crate::ui::profile::take_open_request(ctx), None);
    }
}

#[test]
fn native_action_markers_are_visible_without_hover_and_keep_copy_and_click_targets() {
    let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    let id = crate::demo::DEMO_LOUP;
    world.social.names.insert(id, "Resident".into());
    for verb in ["about", "im", "pay", "offerteleport", "requestfriend", "reportAbuse"] {
        // The icon and resident name both activate the same
        // parsed command. Selection records only the original link label.
        for (part, labelled) in [(0.03, false), (0.95, false), (0.03, true), (0.95, true)] {
            let ctx = egui::Context::default();
            let theme = crate::theme::Theme::default();
            theme.apply(&ctx, 1.0);
            let p = theme.palette();
            let _icons = crate::ui::icons::Icons::load(&ctx, None);
            let mut emoji = crate::ui::emoji::Emoji::load();
            let url = format!("secondlife:///app/agent/{id}/{verb}");
            let text = if labelled { format!("[{url} Resident]") } else { url };
            let mut point = egui::Pos2::ZERO;
            let mut shown = Vec::new();
            for frame in 0..6 {
                let events = match frame {
                    3 | 4 => vec![
                        egui::Event::PointerMoved(point),
                        egui::Event::PointerButton {
                            pos: point,
                            button: egui::PointerButton::Primary,
                            pressed: frame == 3,
                            modifiers: Default::default(),
                        },
                    ],
                    _ => Vec::new(),
                };
                let mut out = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 300.0))),
                        time: Some(frame as f64 * 0.1),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        select::row(ui, 1, None, true, |ui| {
                            chat_text(ui, &p, &mut emoji, &world, &mut HashSet::new(), &text, 14.0, p.ink, false);
                        });
                    },
                );
                out.textures_delta.clear();
                let rows = select::take_drawn();
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].text, "Resident", "copy excludes visual markers");
                let mut selection = select::Selection::default();
                selection.select_all(1, 1);
                assert_eq!(selection.copy_text([1], |_| (String::new(), rows[0].text.clone())), "Resident");
                let r = rows[0].rect;
                point = egui::pos2(r.left() + r.width() * part, r.center().y);
                if frame == 2 {
                    out.shapes.iter().for_each(|s| visible(&s.shape, &mut shown));
                    assert!(
                        out.shapes.iter().map(|s| icon_count(&s.shape)).sum::<usize>() > 0,
                        "visible action icon"
                    );
                }
            }
            assert_eq!(shown, ["Resident"], "no added action text: verb={verb}");
            check_action(&ctx, verb, id);
        }
    }
}

#[test]
fn native_action_markers_wrap_and_are_clickable_in_console_lines_and_bubbles() {
    let world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    let theme = crate::theme::Theme::default();
    let p = theme.palette();
    let id = crate::demo::DEMO_LOUP;
    let line = crate::world::ChatLine {
        time: std::time::SystemTime::now(),
        from: "Resident".into(),
        source: id,
        kind: ChatKind::Local(ChatType::Normal),
        origin: Origin::Live,
        text: format!(
            "[secondlife:///app/agent/{id}/about Resident] · [secondlife:///app/agent/{id}/im Resident] · [secondlife:///app/agent/{id}/pay Resident]"
        ),
    };
    let family = egui::FontFamily::Proportional;
    for look in [JobLook::Line, JobLook::Pill(&family), JobLook::Body] {
        for width in [180.0, 520.0] {
            let ctx = egui::Context::default();
            theme.apply(&ctx, 1.0);
            let mut point = egui::Pos2::ZERO;
            for frame in 0..6 {
                let mut out = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
                        time: Some(frame as f64 * 0.1),
                        events: if matches!(frame, 3 | 4) {
                            vec![
                                egui::Event::PointerMoved(point),
                                egui::Event::PointerButton {
                                    pos: point,
                                    button: egui::PointerButton::Primary,
                                    pressed: frame == 3,
                                    modifiers: Default::default(),
                                },
                            ]
                        } else {
                            Vec::new()
                        },
                        ..Default::default()
                    },
                    |ui| {
                        let rendered = line_job(&line, &p, &world, &mut HashSet::new(), width, 14.0, false, look);
                        assert_eq!(rendered.links.len(), 3);
                        for (range, _) in &rendered.links {
                            let shown: String = rendered.job.text.chars().skip(range.start).take(range.len()).collect();
                            assert_eq!(shown, "MResident", "one invisible icon slot, unchanged destination text");
                        }
                        let icons: Vec<_> = rendered
                            .badges
                            .iter()
                            .filter_map(|(_, b)| match b {
                                LinkBadge::Action(a) => Some(a.icon()),
                                _ => None,
                            })
                            .collect();
                        assert_eq!(icons, ["user-filled", "message-circle-filled", "coin-filled"]);
                        let galley = ui.fonts_mut(|f| f.layout_job(rendered.job));
                        assert!(galley.size().x <= width + 1.0, "wrapping stays bounded: {:?}", galley.size());
                        let origin = egui::pos2(12.0, 20.0);
                        ui.painter().galley(origin, galley.clone(), p.ink);
                        let range = &rendered.links[1].0;
                        let icon = range_rects(&galley, origin, range.start..range.start + 1)[0];
                        point = icon.center();
                        let mut links = Vec::new();
                        line_extras(
                            ui,
                            &p,
                            &world,
                            &crate::ui::bidi::Visual::plain(galley.clone()),
                            origin,
                            14.0,
                            1.0,
                            rendered.badges,
                            rendered.links,
                            p.teal,
                            &mut links,
                        );
                        for (k, (rect, link, color)) in links.into_iter().enumerate() {
                            bubble_link(ui.ctx(), &p, &world, k, rect, &link, color);
                        }
                    },
                );
                out.textures_delta.clear();
            }
            check_action(&ctx, "im", id);
        }
    }
}

#[test]
fn every_native_action_uses_an_available_icon_and_distinct_core_icons() {
    let ctx = egui::Context::default();
    let icons = crate::ui::icons::Icons::load(&ctx, None);
    for action in [
        Action::Profile,
        Action::Im,
        Action::Pay,
        Action::OfferTeleport,
        Action::AddFriend,
        Action::RemoveFriend,
        Action::BlockResident,
        Action::UnblockResident,
        Action::BlockObject,
        Action::UnblockObject,
        Action::Report,
        Action::Group,
        Action::Parcel,
        Action::Object,
        Action::Inventory,
        Action::Experience,
        Action::Classified,
        Action::Event,
        Action::Map,
        Action::Channel,
        Action::Search,
        Action::Blocked,
        Action::Unsupported,
        Action::Place,
        Action::Teleport,
    ] {
        assert!(icons.get(action.icon()).is_some(), "missing icon for {action:?}: {}", action.icon());
    }
    assert_ne!(Action::Profile.icon(), Action::Im.icon());
    assert_ne!(Action::Profile.icon(), Action::Pay.icon());
    assert_ne!(Action::Im.icon(), Action::Pay.icon());
    assert_ne!(Action::Im.icon(), Action::Channel.icon());
    assert_ne!(Action::OfferTeleport.icon(), Action::Teleport.icon());
    assert_ne!(Action::Place.icon(), Action::Teleport.icon());
}
