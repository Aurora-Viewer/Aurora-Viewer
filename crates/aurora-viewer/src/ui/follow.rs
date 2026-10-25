//! Live controls for the current follow (Aurora addition, issue #967).
use super::widgets::{self, ActionKind, Floater};
use crate::{autopilot::FollowSettings, theme::Palette};

#[derive(Default)]
pub struct FollowUi {
    pub open: bool,
    save_pending: bool,
    #[cfg(test)]
    rects: Vec<egui::Rect>,
}

#[derive(Default)]
pub struct Actions {
    pub changed: bool,
    pub save: bool,
    pub stop: bool,
}

impl FollowUi {
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, name: &str, recovering: bool, settings: &mut FollowSettings) -> Actions {
        let mut actions = Actions::default();
        if !self.open {
            return actions;
        }
        #[cfg(test)]
        self.rects.clear();
        Floater::new(
            "follow",
            aurora_i18n::tr!("follow-title"),
            egui::pos2(40.0, 160.0),
            egui::vec2(340.0, 190.0),
        )
        .fixed()
        .show(ctx, p, &mut self.open, |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            ui.label(egui::RichText::new(name).strong());
            ui.label(aurora_i18n::tr!("follow-distance"));
            settings.distance = settings.bounded_distance();
            ui.spacing_mut().slider_width = (ui.available_width() - 80.0).max(80.0);
            let slider = ui.add(
                egui::Slider::new(&mut settings.distance, FollowSettings::MIN_DISTANCE..=FollowSettings::MAX_DISTANCE)
                    .step_by(0.1)
                    .suffix(" m"),
            );
            actions.changed |= slider.changed();
            let toggle = widgets::switch_labeled(ui, p, &mut settings.teleport_if_stuck, aurora_i18n::tr!("follow-teleport-if-stuck"))
                .on_hover_text(aurora_i18n::tr!("follow-teleport-if-stuck-tip"));
            actions.changed |= toggle.changed();
            if recovering {
                ui.label(egui::RichText::new(aurora_i18n::tr!("follow-recovering")).color(p.muted));
            }
            let stop = widgets::action_button(
                ui,
                p,
                "current-location-filled",
                aurora_i18n::tr!("ui-context-stop-following"),
                ActionKind::Flat,
                egui::vec2(ui.available_width(), 28.0),
            );
            actions.stop |= stop.clicked();
            #[cfg(test)]
            self.rects.extend([slider.rect, toggle.rect, stop.rect]);
        });
        // Closing the controller also stops its follow, including Ctrl+W.
        actions.stop |= !self.open;
        self.save_pending |= actions.changed;
        if actions.stop || !ctx.input(|input| input.pointer.any_down()) {
            actions.save = std::mem::take(&mut self.save_pending);
        }
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ctx: &egui::Context, panel: &mut FollowUi, settings: &mut FollowSettings, events: Vec<egui::Event>) -> Actions {
        let mut actions = Actions::default();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
                events,
                ..Default::default()
            },
            |ui| {
                super::super::close_key::begin(ui.ctx());
                actions = panel.show(
                    ui.ctx(),
                    &crate::theme::Theme::default().palette(),
                    "Demo resident",
                    false,
                    settings,
                );
            },
        );
        output.textures_delta.clear();
        actions
    }

    fn click(ctx: &egui::Context, panel: &mut FollowUi, settings: &mut FollowSettings, pos: egui::Pos2) -> Actions {
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(ctx, panel, settings, vec![egui::Event::PointerMoved(pos)]);
        let down = frame(ctx, panel, settings, vec![button(true)]);
        let mut up = frame(ctx, panel, settings, vec![button(false)]);
        up.changed |= down.changed;
        up
    }

    #[test]
    fn follow_controller_changes_distance_and_recovery_and_stops_through_its_button() {
        let ctx = egui::Context::default();
        let mut panel = FollowUi {
            open: true,
            ..Default::default()
        };
        let mut settings = FollowSettings::default();
        for _ in 0..3 {
            frame(&ctx, &mut panel, &mut settings, Vec::new());
        }
        let slider = panel.rects[0];
        assert!(click(&ctx, &mut panel, &mut settings, slider.left_center() + egui::vec2(100.0, 0.0)).changed);
        assert!(settings.distance > 2.0);
        let toggle = panel.rects[1].center();
        assert!(click(&ctx, &mut panel, &mut settings, toggle).changed);
        assert!(settings.teleport_if_stuck);
        let stop = panel.rects[2].center();
        assert!(click(&ctx, &mut panel, &mut settings, stop).stop);
    }

    #[test]
    fn closing_follow_controller_with_the_window_shortcut_stops_the_follow() {
        let ctx = egui::Context::default();
        let mut panel = FollowUi {
            open: true,
            ..Default::default()
        };
        let mut settings = FollowSettings::default();
        for _ in 0..3 {
            frame(&ctx, &mut panel, &mut settings, Vec::new());
        }
        let actions = frame(
            &ctx,
            &mut panel,
            &mut settings,
            vec![egui::Event::Key {
                key: egui::Key::W,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            }],
        );
        assert!(actions.stop);
        assert!(!panel.open);
    }
}
