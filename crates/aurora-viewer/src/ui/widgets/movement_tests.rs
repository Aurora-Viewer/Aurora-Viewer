use super::*;

#[derive(Clone, Copy, Debug)]
enum Kind {
    Resizable,
    Fixed,
    Inspector,
    Minimized,
    Bare,
}

const WINDOW: &str = "movement_fixture";

struct Fixture {
    ctx: egui::Context,
    kind: Kind,
    frame: usize,
    button: egui::Rect,
    clicks: usize,
}

impl Fixture {
    fn new(kind: Kind, scale: f32) -> Self {
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(scale);
        ctx.all_styles_mut(|s| {
            s.animation_time = 0.0;
            s.interaction.selectable_labels = false;
        });
        if matches!(kind, Kind::Minimized) {
            ctx.data_mut(|d| d.insert_persisted(egui::Id::unique((WINDOW, "minimized")), true));
        }
        let mut fixture = Self {
            ctx,
            kind,
            frame: 0,
            button: egui::Rect::NOTHING,
            clicks: 0,
        };
        for _ in 0..8 {
            fixture.show(vec![]);
        }
        fixture
    }

    fn show(&mut self, events: Vec<egui::Event>) -> egui::Rect {
        self.frame += 1;
        let mut output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0))),
                time: Some(self.frame as f64 / 60.0),
                events,
                ..Default::default()
            },
            |ui| {
                let ctx = ui.ctx();
                let p = crate::theme::Theme::default().palette();
                let body = |ui: &mut egui::Ui| {
                    let button = ui.button("Activate");
                    self.button = button.rect;
                    self.clicks += usize::from(button.clicked());
                    ui.set_min_height(160.0);
                };
                let pos = egui::pos2(180.0, 140.0);
                let size = egui::vec2(360.0, 280.0);
                if matches!(self.kind, Kind::Bare) {
                    bare_window(
                        ctx,
                        &p,
                        WINDOW,
                        "Map",
                        egui::Frame::NONE,
                        egui::Rect::from_min_size(pos, size),
                        size,
                        body,
                    );
                } else {
                    let mut floater = Floater::new(WINDOW, "Movement", pos, size);
                    if matches!(self.kind, Kind::Fixed) {
                        floater = floater.fixed();
                    } else if matches!(self.kind, Kind::Inspector) {
                        floater = floater.framed_header("cube").auto_height();
                    }
                    floater.show(ctx, &p, &mut true, body);
                }
            },
        );
        output.textures_delta.clear();
        self.ctx.memory(|m| m.area_rect(egui::Id::unique(WINDOW)).unwrap())
    }

    fn button(&mut self, pos: egui::Pos2, pressed: bool) -> egui::Rect {
        self.show(vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }])
    }

    fn grab(&mut self, pos: egui::Pos2) {
        self.show(vec![egui::Event::PointerMoved(pos)]);
        self.button(pos, true);
    }
}

#[test]
fn slight_motion_during_a_window_click_keeps_its_position_and_activates_the_button() {
    for kind in [Kind::Resizable, Kind::Fixed, Kind::Inspector, Kind::Bare] {
        for scale in [1.0, 1.5, 2.0] {
            let mut f = Fixture::new(kind, scale);
            let before = f.show(vec![]);
            let start = f.button.center();
            f.grab(start);
            let end = start + egui::vec2(3.0, 2.0);
            let held = f.show(vec![egui::Event::PointerMoved(end)]);
            assert_eq!(held, before, "{kind:?} at {scale}: click jitter moved the window");
            f.button(end, false);
            assert_eq!(f.show(vec![]), before);
            assert_eq!(f.clicks, 1, "{kind:?} at {scale}: button lost its click");
        }
    }
}

#[test]
fn window_background_and_title_wait_for_motion_then_follow_the_full_drag() {
    for kind in [Kind::Resizable, Kind::Fixed, Kind::Inspector, Kind::Minimized, Kind::Bare] {
        for scale in [1.0, 1.5, 2.0] {
            let mut f = Fixture::new(kind, scale);
            let before = f.show(vec![]);
            let start = if matches!(kind, Kind::Bare) {
                before.center()
            } else {
                f.ctx
                    .read_response(egui::Id::unique(WINDOW).with("title_double_click"))
                    .unwrap()
                    .rect
                    .center()
            };
            f.grab(start);
            for jitter in [egui::vec2(2.0, 1.0), egui::vec2(-2.0, -1.0), egui::vec2(3.0, 2.0)] {
                assert_eq!(
                    f.show(vec![egui::Event::PointerMoved(start + jitter)]),
                    before,
                    "{kind:?} at {scale}"
                );
            }
            let delta = egui::vec2(42.0, 30.0);
            let end = start + delta;
            let moved = f.show(vec![egui::Event::PointerMoved(end)]);
            assert!(
                (moved.min - before.min - delta).length() < 1.0,
                "{kind:?} at {scale}: {before:?} -> {moved:?}"
            );
            let returned = start + egui::vec2(1.0, 0.0);
            let moved = f.show(vec![egui::Event::PointerMoved(returned)]);
            assert!(
                (moved.min - before.min - egui::vec2(1.0, 0.0)).length() < 1.0,
                "an active drag must keep following inside the tolerance"
            );
            f.button(returned, false);
            let dropped = f.show(vec![]);
            assert_eq!(
                f.show(vec![egui::Event::PointerMoved(end)]),
                dropped,
                "movement must stop on release"
            );
        }
    }
}
