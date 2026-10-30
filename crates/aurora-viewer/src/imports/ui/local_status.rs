//! Persistent scene warning and explicit, confirmed appearance removal (#1375).

use crate::{
    imports::local::AppearanceStatus,
    theme::Palette,
    ui::{
        menu,
        widgets::{self, ActionKind},
    },
};
use std::time::Instant;

#[derive(Default)]
pub struct State {
    pub confirm: bool,
    pub reveal: Option<Instant>,
}

pub enum Action {
    Library,
    Remove,
}

impl State {
    pub fn revealing(&self) -> bool {
        self.reveal.is_some()
    }
    pub fn toggle_reveal(&mut self, now: Instant) {
        self.reveal = if self.revealing() { None } else { Some(now) };
    }
    pub fn pulse(&self, now: Instant, reduce_motion: bool) -> Option<f32> {
        if !self.revealing() {
            return None;
        }
        let elapsed = now.saturating_duration_since(self.reveal?).as_secs_f32();
        // Gentle pulsing while enabled, or a steady highlight with reduced motion.
        Some(if reduce_motion {
            0.3
        } else {
            0.12 + 0.3 * (elapsed * std::f32::consts::PI).sin().powi(2)
        })
    }
}

pub fn show(ctx: &egui::Context, p: &Palette, state: &mut State, status: &AppearanceStatus) -> Option<Action> {
    if status.remembered_objects == 0 || widgets::interface_hidden(ctx) {
        state.confirm = false;
        if status.remembered_objects == 0 {
            state.reveal = None;
        }
        return None;
    }
    let mut action = None;
    let work = crate::ui::layout::work_area(ctx);
    egui::Area::new(egui::Id::unique("local_appearance_warning"))
        .pivot(egui::Align2::CENTER_TOP)
        .fixed_pos(egui::pos2(work.center().x, work.top()))
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            widgets::overlay(ui, p, egui::Margin::same(6), |ui| {
                ui.horizontal(|ui| {
                    let label = if status.faces.is_empty() {
                        aurora_i18n::tr!("local-appearance-pending").to_owned()
                    } else {
                        aurora_i18n::tr!("local-appearance-warning", count = status.faces.len())
                    };
                    let response =
                        widgets::action_button(ui, p, "alert-triangle-filled", &label, ActionKind::Caution, egui::vec2(0.0, 28.0));
                    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("local_appearance_warning_rect"), response.rect));
                    menu::dropdown(&response, p, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
                        ui.set_max_width(360.0);
                        ui.label(aurora_i18n::tr!(
                            "local-appearance-details",
                            textures = status.textures,
                            materials = status.materials
                        ));
                        ui.label(aurora_i18n::tr!("local-appearance-private"));
                        ui.add_space(6.0);
                        if menu::item(ui, p, "eye-filled", aurora_i18n::tr!("local-appearance-library")) {
                            action = Some(Action::Library);
                            ui.close();
                        }
                        let remove = ui.scope(|ui| menu::item(ui, p, "trash-filled", aurora_i18n::tr!("local-appearance-remove")));
                        ctx.data_mut(|d| d.insert_temp(egui::Id::unique("local_appearance_remove_rect"), remove.response.rect));
                        if remove.inner {
                            state.confirm = true;
                            ui.close();
                        }
                    });
                    let on = state.revealing();
                    let tip = if on {
                        aurora_i18n::tr!("local-appearance-stop-reveal")
                    } else {
                        aurora_i18n::tr!("local-appearance-reveal")
                    };
                    let toggle = ui.scope(|ui| {
                        if on || !status.faces.is_empty() {
                            widgets::icon_toggle(ui, p, "focus-centered", tip, on)
                        } else {
                            widgets::icon_button(ui, p, "focus-centered", tip, false)
                        }
                    });
                    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("local_appearance_reveal_rect"), toggle.response.rect));
                    if toggle.inner {
                        state.toggle_reveal(Instant::now());
                    }
                });
            });
        });
    if state.confirm {
        let modal = widgets::Modal::new(egui::Id::unique("local_appearance_confirm")).show(ctx, |ui| {
            ui.set_width(380.0_f32.min(ctx.content_rect().width() - 60.0).max(180.0));
            ui.heading(aurora_i18n::tr!("local-appearance-confirm-title"));
            ui.add_space(8.0);
            ui.label(aurora_i18n::tr!("local-appearance-confirm-body", count = status.remembered_objects));
            ui.add_space(12.0);
            ui.horizontal_wrapped(|ui| {
                let remove = widgets::action_button(
                    ui,
                    p,
                    "trash-filled",
                    aurora_i18n::tr!("common-remove"),
                    ActionKind::Danger,
                    egui::vec2(0.0, 28.0),
                );
                ctx.data_mut(|d| d.insert_temp(egui::Id::unique("local_appearance_confirm_rect"), remove.rect));
                if remove.clicked() {
                    action = Some(Action::Remove);
                    state.confirm = false;
                }
                let cancel = widgets::action_button(
                    ui,
                    p,
                    "x",
                    aurora_i18n::tr!("common-cancel"),
                    ActionKind::Flat,
                    egui::vec2(0.0, 28.0),
                );
                ctx.data_mut(|d| d.insert_temp(egui::Id::unique("local_appearance_cancel_rect"), cancel.rect));
                if cancel.clicked() {
                    state.confirm = false;
                }
            });
        });
        if modal.should_close() {
            state.confirm = false;
        }
    }
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn local_reveal_toggle_stays_active_until_disabled_and_reduced_motion_is_steady() {
        let at = Instant::now();
        let mut state = State::default();
        assert_eq!(state.pulse(at, false), None);
        state.toggle_reveal(at);
        assert_eq!(state.pulse(at, true), Some(0.3));
        assert_eq!(state.pulse(at + Duration::from_secs(1), true), Some(0.3));
        assert!(state.pulse(at + Duration::from_millis(500), false).expect("pulse") > state.pulse(at, false).expect("pulse"));
        assert!(state.pulse(at + Duration::from_secs(10), false).is_some());
        state.toggle_reveal(at + Duration::from_secs(10));
        assert_eq!(state.pulse(at + Duration::from_secs(10), false), None);
    }

    #[test]
    fn warning_has_a_direct_highlight_toggle_and_requires_confirmation_to_remove() {
        for scale in [1.0, 1.5] {
            let ctx = egui::Context::default();
            let theme = crate::theme::Theme::default();
            theme.apply(&ctx, scale);
            let mut state = State::default();
            let status = AppearanceStatus {
                remembered_objects: 2,
                textures: 1,
                materials: 1,
                faces: [(0, 3)].into(),
            };
            let frame = |state: &mut State, events| {
                let mut action = None;
                let _ = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 700.0))),
                        events,
                        ..Default::default()
                    },
                    |ui| action = show(ui.ctx(), &theme.palette(), state, &status),
                );
                action
            };
            let click = |state: &mut State, key: &str| {
                let pos = ctx
                    .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(key)))
                    .expect("visible control")
                    .center();
                let mut result = None;
                for pressed in [true, false] {
                    if let Some(action) = frame(
                        state,
                        vec![
                            egui::Event::PointerMoved(pos),
                            egui::Event::PointerButton {
                                pos,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: Default::default(),
                            },
                        ],
                    ) {
                        result = Some(action);
                    }
                }
                result
            };
            for _ in 0..4 {
                frame(&mut state, vec![]);
            }
            // Highlight is directly accessible without opening the warning menu.
            assert!(click(&mut state, "local_appearance_reveal_rect").is_none());
            assert!(state.revealing());
            assert!(click(&mut state, "local_appearance_reveal_rect").is_none());
            assert!(!state.revealing());
            click(&mut state, "local_appearance_reveal_rect");
            assert!(state.revealing());
            assert!(click(&mut state, "local_appearance_warning_rect").is_none());
            frame(&mut state, vec![]);
            assert!(click(&mut state, "local_appearance_remove_rect").is_none());
            assert!(state.confirm);
            frame(&mut state, vec![]);
            assert!(click(&mut state, "local_appearance_cancel_rect").is_none());
            assert!(!state.confirm);
            assert!(state.revealing(), "cancelling removal keeps highlight enabled");
            state.confirm = true;
            frame(&mut state, vec![]);
            assert!(
                frame(
                    &mut state,
                    vec![egui::Event::Key {
                        key: egui::Key::Escape,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Default::default()
                    }]
                )
                .is_none()
            );
            assert!(!state.confirm);
            state.confirm = true;
            for _ in 0..3 {
                frame(&mut state, vec![]);
            }
            assert!(matches!(click(&mut state, "local_appearance_confirm_rect"), Some(Action::Remove)));
            assert!(!state.confirm);
        }
    }
}
