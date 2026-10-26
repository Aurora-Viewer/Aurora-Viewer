use super::*;
use crate::ui::chat::previews::{Entry, Previews, card};
use std::time::Duration;

fn shown(ctx: &Context) -> Shown {
    Shown::new(
        ctx,
        "lightbox-test",
        lp::Picture {
            frames: vec![(egui::ColorImage::filled([4, 3], egui::Color32::GRAY), Duration::ZERO)],
            size: [640, 480],
            blurred: egui::ColorImage::filled([1, 1], egui::Color32::GRAY),
        },
    )
}

#[test]
fn fitted_picture_keeps_its_ratio_on_resize_and_zoom_keeps_the_anchor() {
    let canvas = Rect::from_min_size(Pos2::new(20.0, 30.0), Vec2::new(600.0, 300.0));
    let source = Vec2::new(640.0, 480.0);
    let mut view = View::fitted();
    assert_eq!(view.rect(source, canvas).size(), Vec2::new(400.0, 300.0));
    let anchor = Vec2::new(100.0, -50.0);
    let original = (anchor - view.offset) / view.zoom;
    view.zoom_at(2.0, anchor);
    assert_eq!((anchor - view.offset) / view.zoom, original);
    view.offset += Vec2::new(300.0, 90.0);
    assert_eq!(view.rect(source, canvas).center(), canvas.center() + view.offset);
    view.zoom_at(1e6, anchor);
    assert_eq!(view.zoom, 16.0);
    view.zoom_at(1e-6, anchor);
    assert_eq!(view.zoom, 0.1);
    let resized = Rect::from_min_size(Pos2::ZERO, Vec2::new(240.0, 600.0));
    assert_eq!(View::fitted().rect(source, resized).size(), Vec2::new(240.0, 180.0));
}

fn input(events: Vec<egui::Event>) -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 700.0))),
        events,
        ..Default::default()
    }
}

// These headless frames deliberately discard uploads; egui requires the
// renderer's texture delta to be acknowledged even without a GPU backend.
fn run_ui(ctx: &Context, input: egui::RawInput, draw: impl FnMut(&mut egui::Ui)) -> egui::FullOutput {
    let mut output = ctx.run_ui(input, draw);
    output.textures_delta.clear();
    output
}

fn run_pass(ctx: &Context, input: egui::RawInput, draw: impl FnMut(&Context)) -> egui::FullOutput {
    let mut output = ctx.run_pass(input, draw);
    output.textures_delta.clear();
    output
}

fn pointer(at: Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

#[test]
fn card_click_enlarges_locally_and_group_reveal_stays_a_separate_click() {
    let ctx = Context::default();
    let p = crate::theme::Theme::default().palette();
    let url = "https://i.imgur.com/test.png";
    let mut pv = Previews {
        demo: true,
        demo_zoom: None,
        ..Default::default()
    };
    pv.cache.insert(
        url.to_owned(),
        crate::ui::chat::previews::Cached {
            entry: Entry::Picture(shown(&ctx)),
            used: 0,
        },
    );
    let at = Pos2::new(90.0, 120.0);
    let mut frame = |events| {
        run_ui(&ctx, input(events), |ui| {
            card(ui, &p, &mut pv, 7, url, &lp::Target::Picture, lp::Start::Load);
        })
    };
    let _ = frame(vec![]);
    let _ = frame(vec![egui::Event::PointerMoved(at), pointer(at, true)]);
    let out = frame(vec![pointer(at, false)]);
    assert!(
        out.platform_output
            .commands
            .iter()
            .all(|command| !matches!(command, egui::OutputCommand::OpenUrl(_)))
    );
    assert!(pv.enlarged());
    assert!(crate::ui::context::take_requests(&ctx).is_empty());
    pv.close_lightbox(&ctx);
    let browser_at = Pos2::new(209.0, 21.0);
    let mut frame = |events| {
        run_ui(&ctx, input(events), |ui| {
            card(ui, &p, &mut pv, 7, url, &lp::Target::Picture, lp::Start::Load);
        })
    };
    let _ = frame(vec![egui::Event::PointerMoved(browser_at)]);
    let _ = frame(vec![pointer(browser_at, true)]);
    let _ = frame(vec![pointer(browser_at, false)]);
    assert!(
        crate::ui::context::take_requests(&ctx)
            .iter()
            .any(|action| matches!(action, crate::ui::context::CtxAction::OpenUrl(link) if link == url))
    );
    assert!(!pv.enlarged());
    pv.place = lp::Place::Group;
    let mut frame = |events| {
        let _ = run_ui(&ctx, input(events), |ui| {
            card(ui, &p, &mut pv, 7, url, &lp::Target::Picture, lp::Start::Load);
        });
    };
    frame(vec![]);
    frame(vec![pointer(at, true)]);
    frame(vec![pointer(at, false)]);
    assert!(pv.revealed.contains(&7));
    assert!(!pv.enlarged());
    let mut frame = |events| {
        let _ = run_ui(&ctx, input(events), |ui| {
            card(ui, &p, &mut pv, 7, url, &lp::Target::Picture, lp::Start::Load);
        });
    };
    frame(vec![pointer(at, true)]);
    frame(vec![pointer(at, false)]);
    assert!(pv.enlarged());
}

#[test]
fn image_layer_receives_wheel_and_drag_and_closing_restores_focus() {
    let ctx = Context::default();
    let p = crate::theme::Theme::default().palette();
    let bounds = Rect::from_min_size(Pos2::new(50.0, 60.0), Vec2::new(600.0, 420.0));
    let previous = Id::unique("chat-editor");
    ctx.memory_mut(|m| m.request_focus(previous));
    let mut pv = Previews {
        demo_zoom: None,
        ..Default::default()
    };
    pv.lightbox = Some(Lightbox::new(&ctx, "https://i.imgur.com/test.png".to_owned(), shown(&ctx)));
    let mut frame = |events| {
        let _ = run_pass(&ctx, input(events), |ctx| {
            pv.lightbox_keys(ctx);
            pv.lightbox(ctx, &p, Some(bounds));
        });
    };
    frame(vec![]);
    let at = bounds.center();
    frame(vec![egui::Event::PointerMoved(at)]);
    frame(vec![egui::Event::MouseWheel {
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Wheel,
        unit: egui::MouseWheelUnit::Point,
        delta: Vec2::new(0.0, 100.0),
        modifiers: egui::Modifiers::NONE,
    }]);
    assert!(pv.lightbox.as_ref().unwrap().view.zoom > 1.0);
    let before = pv.lightbox.as_ref().unwrap().view.offset;
    let mut frame = |events| {
        let _ = run_pass(&ctx, input(events), |ctx| pv.lightbox(ctx, &p, Some(bounds)));
    };
    frame(vec![pointer(at, true)]);
    frame(vec![egui::Event::PointerMoved(at + Vec2::new(35.0, 20.0))]);
    frame(vec![pointer(at + Vec2::new(35.0, 20.0), false)]);
    assert!((pv.lightbox.as_ref().unwrap().view.offset - before).length() > 10.0);
    let mut escaped = false;
    let _ = run_pass(
        &ctx,
        input(vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]),
        |ctx| {
            pv.lightbox_keys(ctx);
            escaped = ctx.input(|i| i.key_pressed(egui::Key::Escape));
            assert_eq!(ctx.memory(|m| m.focused()), Some(previous));
        },
    );
    assert!(!pv.enlarged());
    assert!(!escaped);
}

#[test]
fn closing_drops_pending_detail_and_hidden_windows_dismiss_the_image() {
    let ctx = Context::default();
    let mut pv = Previews {
        demo_zoom: None,
        ..Default::default()
    };
    let mut view = Lightbox::new(&ctx, String::new(), shown(&ctx));
    let (tx, rx) = mpsc::channel();
    view.pending = Some(rx);
    pv.lightbox = Some(view);
    pv.lightbox(&ctx, &crate::theme::Theme::default().palette(), None);
    assert!(!pv.enlarged());
    assert!(tx.send(Err(lp::Failure::Unavailable)).is_err());
}

#[test]
fn close_accelerator_dismisses_the_image_before_its_conversation() {
    let ctx = Context::default();
    let owner = Id::unique("conversations");
    let p = crate::theme::Theme::default().palette();
    let mut pv = Previews {
        demo_zoom: None,
        ..Default::default()
    };
    pv.lightbox = Some(Lightbox::new(&ctx, String::new(), shown(&ctx)));
    let _ = run_pass(&ctx, input(vec![]), |ctx| {
        crate::ui::widgets::Floater::new("conversations", "Chat", Pos2::ZERO, Vec2::new(500.0, 350.0)).show(ctx, &p, &mut true, |_| {});
        crate::ui::close_key::shown(ctx, owner);
    });
    let _ = run_pass(
        &ctx,
        input(vec![egui::Event::Key {
            key: egui::Key::W,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }]),
        |ctx| {
            crate::ui::close_key::begin(ctx);
            pv.lightbox_keys(ctx);
            assert!(!crate::ui::close_key::take(ctx, owner));
            assert!(!pv.enlarged());
        },
    );
}

#[test]
fn video_hover_actions_copy_the_original_url_and_use_the_browser_confirmation_path() {
    let ctx = Context::default();
    let p = crate::theme::Theme::default().palette();
    let url = "https://youtu.be/jNQXAC9IVRw?t=5";
    let (target, _) = lp::decide(&lp::Settings::default(), lp::Place::Im, url).expect("a supported video");
    let mut pv = Previews {
        demo_zoom: None,
        ..Default::default()
    };
    pv.cache.insert(
        url.into(),
        crate::ui::chat::previews::Cached {
            entry: Entry::Video {
                title: "Demo video".into(),
                channel: "Demo channel".into(),
                thumbnail: None,
                aspect: 16.0 / 9.0,
                clip: None,
            },
            used: 0,
        },
    );
    let mut rect = Rect::NOTHING;
    let _ = run_ui(&ctx, input(vec![]), |ui| {
        card(ui, &p, &mut pv, 7, url, &target, lp::Start::Load);
        rect = ui.min_rect();
    });
    let mut frame = |events| {
        run_ui(&ctx, input(events), |ui| {
            card(ui, &p, &mut pv, 7, url, &target, lp::Start::Load);
        })
    };
    let open = Pos2::new(rect.right() - 18.0, rect.bottom() - 26.0);
    let copy = open - Vec2::new(28.0, 0.0);
    for _ in 0..20 {
        let _ = frame(vec![egui::Event::PointerMoved(copy)]);
    }
    let _ = frame(vec![pointer(copy, true)]);
    let output = frame(vec![pointer(copy, false)]);
    assert!(
        output
            .platform_output
            .commands
            .iter()
            .any(|command| matches!(command, egui::OutputCommand::CopyText(text) if text == url))
    );
    let _ = frame(vec![egui::Event::PointerMoved(open), pointer(open, true)]);
    let _ = frame(vec![pointer(open, false)]);
    assert!(
        crate::ui::context::take_requests(&ctx)
            .iter()
            .any(|action| matches!(action, crate::ui::context::CtxAction::OpenUrl(link) if link == url))
    );
    assert!(pv.playing.is_none(), "URL actions do not start a video");
}

#[test]
fn hover_link_action_copies_the_url_without_enlarging_or_copying_pixels() {
    let ctx = Context::default();
    let p = crate::theme::Theme::default().palette();
    let url = "https://i.imgur.com/test.png";
    let mut pv = Previews {
        demo_zoom: None,
        ..Default::default()
    };
    pv.cache.insert(
        url.into(),
        crate::ui::chat::previews::Cached {
            entry: Entry::Picture(shown(&ctx)),
            used: 0,
        },
    );
    let mut frame = |events| {
        run_ui(&ctx, input(events), |ui| {
            card(ui, &p, &mut pv, 7, url, &lp::Target::Picture, lp::Start::Load);
        })
    };
    let at = Pos2::new(179.0, 21.0);
    let _ = frame(vec![egui::Event::PointerMoved(at)]);
    let _ = frame(vec![pointer(at, true)]);
    let out = frame(vec![pointer(at, false)]);
    assert!(
        out.platform_output
            .commands
            .iter()
            .any(|command| matches!(command, egui::OutputCommand::CopyText(text) if text == url))
    );
    assert!(crate::ui::chat::previews::clipboard::take(&ctx).is_none());
    assert!(crate::ui::context::take_requests(&ctx).is_empty());
    assert!(!pv.enlarged() && !pv.image_window_open());
}

#[test]
fn hover_actions_copy_pixels_and_open_an_independent_image_window() {
    let ctx = Context::default();
    let p = crate::theme::Theme::default().palette();
    let url = "https://i.imgur.com/test.png";
    let image = shown(&ctx);
    let expected = image.copy.clone();
    let mut pv = Previews {
        demo: true,
        demo_zoom: None,
        ..Default::default()
    };
    pv.cache.insert(
        url.into(),
        crate::ui::chat::previews::Cached {
            entry: Entry::Picture(image),
            used: 0,
        },
    );
    let mut frame = |events| {
        let _ = run_ui(&ctx, input(events), |ui| {
            card(ui, &p, &mut pv, 7, url, &lp::Target::Picture, lp::Start::Load)
        });
    };
    frame(vec![]);
    let copy = Pos2::new(269.0, 21.0);
    frame(vec![egui::Event::PointerMoved(copy)]);
    frame(vec![pointer(copy, true)]);
    frame(vec![pointer(copy, false)]);
    let copied = crate::ui::chat::previews::clipboard::take(&ctx).expect("Copy queues the picture, not its link");
    assert_eq!(copied.size, expected.size);
    assert_eq!(copied.pixels, expected.pixels);
    assert!(!pv.enlarged());
    assert!(!pv.image_window_open());
    let window = Pos2::new(239.0, 21.0);
    let mut frame = |events| {
        let _ = run_ui(&ctx, input(events), |ui| {
            card(ui, &p, &mut pv, 7, url, &lp::Target::Picture, lp::Start::Load)
        });
    };
    frame(vec![egui::Event::PointerMoved(window)]);
    frame(vec![pointer(window, true)]);
    frame(vec![pointer(window, false)]);
    assert!(pv.image_window_open());
    assert!(!pv.enlarged(), "The new icon must not darken the conversation");
    let _ = run_pass(&ctx, input(vec![]), |ctx| {
        pv.lightbox(ctx, &p, None); // the chat was closed
        pv.player_window(ctx, &p);
    });
    assert!(pv.image_window_open(), "Closing the chat must keep the detached image");
    let id = Id::unique("link_preview_image");
    assert!(egui::AreaState::load(&ctx, id).is_some());
    assert!(
        ctx.memory(|m| m.areas().parent_layer(egui::LayerId::new(egui::Order::Middle, id)))
            .is_none()
    );
}
