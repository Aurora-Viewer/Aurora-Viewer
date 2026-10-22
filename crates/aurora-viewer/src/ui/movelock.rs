//! Physical position lock controls, shared by the menu and toolbar.
use super::widgets::{self, ActionKind, Floater};
use crate::{
    theme::Palette,
    world::bridge::{LockMode, Preferences},
};

#[derive(Default)]
pub struct MoveLockUi {
    pub open: bool,
}
impl MoveLockUi {
    /// (preferences changed, repair requested).
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        prefs: &mut Preferences,
        status: &str,
        error: Option<&str>,
        shortcut: &str,
    ) -> (bool, bool) {
        let before = *prefs;
        let mut repair = false;
        Floater::new(
            "movelock",
            aurora_i18n::tr!("movelock-title"),
            egui::pos2(460.0, 180.0),
            egui::vec2(470.0, 420.0),
        )
        .show(ctx, p, &mut self.open, |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            ui.label(aurora_i18n::tr!("movelock-help"));
            ui.add_space(8.0);
            ui.label(aurora_i18n::tr!("movelock-mode"));
            let response = widgets::radio_value(ui, p, &mut prefs.mode, LockMode::Bridge, aurora_i18n::tr!("movelock-mode-bridge"));
            #[cfg(test)]
            remember(ui.ctx(), 3, response.rect);
            #[cfg(not(test))]
            let _ = response;
            let response = widgets::radio_value(ui, p, &mut prefs.mode, LockMode::NativeSit, aurora_i18n::tr!("movelock-mode-sit"));
            #[cfg(test)]
            remember(ui.ctx(), 4, response.rect);
            #[cfg(not(test))]
            let _ = response;
            if prefs.mode == LockMode::Bridge {
                ui.label(aurora_i18n::tr!("movelock-bridge-help"));
            }
            ui.add_space(8.0);
            let response = widgets::switch_labeled(ui, p, &mut prefs.enabled, aurora_i18n::tr!("movelock-enable"));
            #[cfg(test)]
            remember(ui.ctx(), 0, response.rect);
            #[cfg(not(test))]
            let _ = response;
            ui.label(egui::RichText::new(status).color(p.violet_pale));
            if let Some(error) = error {
                ui.label(error);
            }
            widgets::divider(ui, p);
            let response = widgets::checkbox(ui, p, &mut prefs.indicator, aurora_i18n::tr!("movelock-indicator"));
            #[cfg(test)]
            remember(ui.ctx(), 1, response.rect);
            #[cfg(not(test))]
            let _ = response;
            ui.add_space(8.0);
            ui.label(aurora_i18n::tr!("movelock-shortcut", shortcut = shortcut.to_owned()));
            if prefs.mode == LockMode::Bridge || error.is_some() {
                let response = widgets::action_button(
                    ui,
                    p,
                    "refresh",
                    aurora_i18n::tr!("bridge-repair"),
                    ActionKind::Flat,
                    egui::vec2(0.0, 28.0),
                );
                #[cfg(test)]
                remember(ui.ctx(), 2, response.rect);
                if response.clicked() {
                    repair = true;
                }
            }
        });
        (*prefs != before, repair)
    }
    pub fn indicator(&mut self, ctx: &egui::Context, p: &Palette, label: &str, status: &str) {
        let room = super::layout::work_area(ctx);
        egui::Area::new(egui::Id::unique("movelock_indicator"))
            .default_pos(egui::pos2(room.left() + 12.0, room.bottom() - 40.0))
            .default_size(egui::Vec2::ZERO)
            .movable(true)
            .constrain_to(room)
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                widgets::overlay(ui, p, egui::Margin::same(4), |ui| {
                    if widgets::action_button(ui, p, "lock-filled", label, ActionKind::Flat, egui::vec2(0.0, 26.0))
                        .on_hover_text(status)
                        .clicked()
                    {
                        self.open = true;
                    }
                });
            });
    }
}

#[cfg(test)]
fn remember(ctx: &egui::Context, index: usize, rect: egui::Rect) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique(("movelock_control", index)), rect));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn indicator_drags_without_opening_options_and_restores_its_position() {
        let ctx = egui::Context::default();
        let mut panel = MoveLockUi::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0));
        let id = egui::Id::unique("movelock_indicator");
        let frame = |ctx: &egui::Context, panel: &mut MoveLockUi, screen, events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    events,
                    ..Default::default()
                },
                |ui| {
                    panel.indicator(
                        ui.ctx(),
                        &crate::theme::Theme::default().palette(),
                        aurora_i18n::tr!("movelock-short-on"),
                        aurora_i18n::tr!("movelock-on"),
                    );
                },
            );
            output.textures_delta.clear();
            egui::AreaState::load(ctx, id).expect("indicator area").rect()
        };
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        let mut before = egui::Rect::NOTHING;
        for _ in 0..4 {
            before = frame(&ctx, &mut panel, screen, Vec::new());
        }
        let room = super::super::layout::work_area(&ctx);
        assert!((before.min - egui::pos2(room.left() + 12.0, room.bottom() - 40.0)).length() < 2.0);
        let from = before.center();
        let delta = egui::vec2(320.0, -180.0);
        frame(&ctx, &mut panel, screen, vec![egui::Event::PointerMoved(from), button(from, true)]);
        frame(&ctx, &mut panel, screen, vec![egui::Event::PointerMoved(from + delta)]);
        frame(&ctx, &mut panel, screen, vec![button(from + delta, false)]);
        let moved = frame(&ctx, &mut panel, screen, Vec::new());
        assert!(
            (moved.min - before.min - delta).length() < 2.0,
            "drag the indicator from its button, not only its margin"
        );
        assert!(!panel.open, "a drag must not activate the options click");
        let saved = ron::to_string(&ctx.memory(|m| m.clone())).expect("save layout");
        let restored = egui::Context::default();
        restored.memory_mut(|m| *m = ron::from_str(&saved).expect("restore layout"));
        for _ in 0..4 {
            frame(&restored, &mut panel, screen, Vec::new());
        }
        let after = frame(&restored, &mut panel, screen, Vec::new());
        assert!(
            (after.min - moved.min).length() < 2.0,
            "the normal saved layout retains the indicator position"
        );
        let small = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640.0, 480.0));
        let clamped = frame(&restored, &mut panel, small, Vec::new());
        assert!(super::super::layout::work_area(&restored).expand(1.0).contains_rect(clamped));
        let click = clamped.center();
        frame(
            &restored,
            &mut panel,
            small,
            vec![egui::Event::PointerMoved(click), button(click, true)],
        );
        frame(&restored, &mut panel, small, vec![button(click, false)]);
        assert!(panel.open, "a simple click still opens options after dragging");
    }
    #[test]
    fn options_clicks_toggle_the_selected_preference_and_repair_keeps_requested_mode() {
        let ctx = egui::Context::default();
        let mut panel = MoveLockUi { open: true };
        let mut prefs = Preferences::default();
        let frame = |events: Vec<egui::Event>, panel: &mut MoveLockUi, prefs: &mut Preferences| {
            let mut action = (false, false);
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    action = panel.show(
                        ui.ctx(),
                        &crate::theme::Theme::default().palette(),
                        prefs,
                        aurora_i18n::tr!("movelock-off"),
                        None,
                        "Ctrl+Alt+P",
                    );
                },
            );
            output.textures_delta.clear();
            action
        };
        for _ in 0..4 {
            assert_eq!(frame(Vec::new(), &mut panel, &mut prefs), (false, false));
        }
        for index in [0usize, 1, 2, 4, 3] {
            let before = prefs;
            let rect = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("movelock_control", index))))
                .expect("control rectangle");
            let pos = rect.center();
            let click = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::default(),
            };
            frame(vec![egui::Event::PointerMoved(pos), click(true)], &mut panel, &mut prefs);
            let action = frame(vec![click(false)], &mut panel, &mut prefs);
            if index == 2 {
                assert_eq!(action, (false, true));
                assert_eq!(prefs, before);
            } else {
                assert_eq!(action, (true, false));
                let mut expected = before;
                match index {
                    0 => expected.enabled = !expected.enabled,
                    1 => expected.indicator = !expected.indicator,
                    3 => expected.mode = LockMode::Bridge,
                    4 => expected.mode = LockMode::NativeSit,
                    _ => unreachable!(),
                }
                assert_eq!(prefs, expected);
            }
        }
    }
}
