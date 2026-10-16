//! Physical position lock controls, shared by the menu and toolbar.
use super::widgets::{self, ActionKind, Floater};
use crate::{theme::Palette, world::bridge::Preferences};

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
            egui::vec2(430.0, 300.0),
        )
        .show(ctx, p, &mut self.open, |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            ui.label(aurora_i18n::tr!("movelock-help"));
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
        });
        (*prefs != before, repair)
    }
    pub fn indicator(&mut self, ctx: &egui::Context, p: &Palette, status: &str) {
        let room = super::layout::work_area(ctx);
        egui::Area::new(egui::Id::new("movelock_indicator"))
            .fixed_pos(egui::pos2(room.left() + 12.0, room.bottom() - 40.0))
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                widgets::overlay(ui, p, egui::Margin::same(4), |ui| {
                    if widgets::action_button(ui, p, "lock-filled", status, ActionKind::Flat, egui::vec2(0.0, 26.0)).clicked() {
                        self.open = true;
                    }
                });
            });
    }
}

#[cfg(test)]
fn remember(ctx: &egui::Context, index: usize, rect: egui::Rect) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(("movelock_control", index)), rect));
}

#[cfg(test)]
mod tests {
    use super::*;
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
        for index in 0usize..3 {
            let before = prefs;
            let rect = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("movelock_control", index))))
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
                    _ => unreachable!(),
                }
                assert_eq!(prefs, expected);
            }
        }
    }
}
