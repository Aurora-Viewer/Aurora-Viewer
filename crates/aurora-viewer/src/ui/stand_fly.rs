//! The floating Stand / Stop flying / Flycam buttons over the world: port of
//! LLPanelStandStopFlying (indra/newview/llmoveview.cpp, llmoveview.h and
//! skins/default/xui/en/panel_stand_stop_flying.xml, originally LGPL 2.1),
//! driven by LLFloaterMove::setSittingMode / setFlyingMode,
//! LLViewerJoystick::toggleFlycam and LLAgent's mouselook changes.
//!
//! Stand shows while our avatar sits (on the ground or on an object), Stop
//! flying while it flies without sitting (again when it stands up still
//! flying, EXT-871), Flycam while the joystick flycam is on. The panel goes
//! with the hidden interface (HideUIControls) and in mouselook, unless the
//! interface stays there (FSShowInterfaceInMouselook). RLVa does not hide
//! Stand but greys it while @unsit forbids standing, as
//! RlvUIEnabler::onToggleUnsit (indra/newview/rlvui.cpp) does to the
//! panel's stand_btn; the click is still refused, as onStandButtonClick does
//! through RlvActions::canStand (`App::stand_fly_action`). A setting
//! (`show_stand_fly_panel`) hides the panel altogether.
//!
//! The Legacy Viewer keeps the panel at a fixed spot of its bottom toolbar
//! area; here it sits centred just above the bottom bar, in the overlay
//! glass of the panels laid over the world.

use super::widgets;
use crate::theme::Palette;
use egui::{Id, Vec2};

/// Height of a button (the Controls window's).
const BUTTON_H: f32 = 26.0;
/// Space between the panel and the bottom bar.
const GAP: f32 = 8.0;

/// What a frame knows of our avatar and the interface.
#[derive(Debug, Clone, Copy, Default)]
pub struct State {
    /// The setting `show_stand_fly_panel`.
    pub enabled: bool,
    /// RLVa forbids standing (@unsit) right now.
    pub stand_locked: bool,
    /// Sitting, on the ground or on an object.
    pub seated: bool,
    pub flying: bool,
    /// The joystick flycam is on.
    pub flycam: bool,
    pub mouselook: bool,
    /// FSShowInterfaceInMouselook.
    pub show_in_mouselook: bool,
    /// The interface is hidden (HideUIControls, immersive mode).
    pub ui_hidden: bool,
}

/// The buttons a frame shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Buttons {
    pub stand: bool,
    pub stop_flying: bool,
    pub flycam: bool,
    /// Stand is shown but greyed: RLVa forbids standing.
    pub stand_disabled: bool,
}

impl Buttons {
    pub fn any(self) -> bool {
        self.stand || self.stop_flying || self.flycam
    }
}

/// A button clicked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// onStandButtonClick: deselect, then AGENT_CONTROL_STAND_UP.
    Stand,
    /// onStopFlyingButtonClick: gAgent.setFlying(false).
    StopFlying,
    /// View.JoystickFlycam: the flycam off.
    Flycam,
}

/// setStandStopFlyingMode / clearStandStopFlyingMode and setVisible: Stand
/// and Stop flying share one slot (sitting wins), Flycam has its own.
pub fn buttons(s: &State) -> Buttons {
    if !s.enabled || s.ui_hidden || (s.mouselook && !s.show_in_mouselook) {
        return Buttons::default();
    }
    Buttons {
        stand: s.seated,
        stand_disabled: s.seated && s.stand_locked,
        stop_flying: s.flying && !s.seated,
        flycam: s.flycam,
    }
}

/// The panel, centred above `bottom_top` (the top of the bottom bar).
///
/// It lies under the windows, like the Legacy Viewer's panel in the toolbars'
/// layer: the background layer, as the chat console's lines, so a click on it
/// never brings it above a window that overlaps it (a click raises an area
/// within its own layer only).
pub fn show(ctx: &egui::Context, p: &Palette, shown: Buttons, bottom_top: f32) -> Option<Action> {
    let screen = ctx.content_rect();
    let mut action = None;
    egui::Area::new(Id::unique("stand_stop_flying"))
        .order(egui::Order::Background)
        .pivot(egui::Align2::CENTER_BOTTOM)
        .fixed_pos(egui::pos2(screen.center().x.round(), (bottom_top - GAP).round()))
        .constrain(false)
        .interactable(shown.any())
        .show(ctx, |ui| {
            if !shown.any() {
                return;
            }
            widgets::overlay(ui, p, egui::Margin::same(4), |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let size = Vec2::new(0.0, BUTTON_H);
                    let kind = widgets::ActionKind::Flat;
                    if shown.stand
                        && widgets::action_button_if(
                            ui,
                            p,
                            "man-filled",
                            aurora_i18n::tr!("common-stand-up"),
                            kind,
                            size,
                            !shown.stand_disabled,
                        )
                        .on_hover_text(aurora_i18n::tr!("stand-panel-stand-tip"))
                        .clicked()
                    {
                        action = Some(Action::Stand);
                    }
                    if shown.stop_flying
                        && widgets::action_button(ui, p, "plane-tilt-filled", aurora_i18n::tr!("ui-bars-stop-flying"), kind, size)
                            .on_hover_text(aurora_i18n::tr!("ui-bars-stop-flying"))
                            .clicked()
                    {
                        action = Some(Action::StopFlying);
                    }
                    if shown.flycam
                        && widgets::action_button(ui, p, "video-filled", aurora_i18n::tr!("stand-panel-flycam"), kind, size)
                            .on_hover_text(aurora_i18n::tr!("joystick-flycam-stop"))
                            .clicked()
                    {
                        action = Some(Action::Flycam);
                    }
                });
            });
        });
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Pos2, Rect};

    fn state() -> State {
        State {
            enabled: true,
            ..State::default()
        }
    }

    #[test]
    fn standing_on_foot_shows_nothing() {
        assert!(!buttons(&state()).any());
    }

    #[test]
    fn sitting_shows_stand_only() {
        let b = buttons(&State { seated: true, ..state() });
        assert_eq!(
            b,
            Buttons {
                stand: true,
                ..Buttons::default()
            }
        );
        // seated while the flight flag lingers: Stand takes the slot
        let b = buttons(&State {
            seated: true,
            flying: true,
            ..state()
        });
        assert!(b.stand && !b.stop_flying);
    }

    #[test]
    fn flying_shows_stop_flying_and_again_after_standing_up() {
        let b = buttons(&State { flying: true, ..state() });
        assert_eq!(
            b,
            Buttons {
                stop_flying: true,
                ..Buttons::default()
            }
        );
        // EXT-871: up from a seat, still flying
        let seated = State {
            seated: true,
            flying: true,
            ..state()
        };
        let up = State { seated: false, ..seated };
        assert!(buttons(&seated).stand);
        assert!(buttons(&up).stop_flying && !buttons(&up).stand);
    }

    #[test]
    fn rlva_greys_stand_while_seated() {
        let b = buttons(&State {
            seated: true,
            stand_locked: true,
            ..state()
        });
        assert!(b.stand && b.stand_disabled);
        // not seated: the lock concerns no button
        let b = buttons(&State {
            flying: true,
            stand_locked: true,
            ..state()
        });
        assert!(b.stop_flying && !b.stand && !b.stand_disabled);
        assert!(!buttons(&State { seated: true, ..state() }).stand_disabled);
    }

    #[test]
    fn the_setting_off_hides_the_panel_in_every_state() {
        let all = State {
            enabled: false,
            seated: true,
            flying: true,
            flycam: true,
            stand_locked: true,
            ..state()
        };
        assert!(!buttons(&all).any());
        assert!(!buttons(&State { seated: false, ..all }).any());
        assert!(
            !buttons(&State {
                flycam: false,
                seated: false,
                ..all
            })
            .any()
        );
    }

    #[test]
    fn flycam_has_its_own_button() {
        let b = buttons(&State { flycam: true, ..state() });
        assert_eq!(
            b,
            Buttons {
                flycam: true,
                ..Buttons::default()
            }
        );
        let b = buttons(&State {
            flycam: true,
            seated: true,
            ..state()
        });
        assert!(b.flycam && b.stand);
    }

    #[test]
    fn mouselook_hides_unless_the_interface_stays() {
        let seated = State {
            seated: true,
            mouselook: true,
            ..state()
        };
        assert!(!buttons(&seated).any());
        assert!(
            buttons(&State {
                show_in_mouselook: true,
                ..seated
            })
            .stand
        );
    }

    #[test]
    fn hidden_interface_hides_everything() {
        let all = State {
            seated: true,
            flycam: true,
            ui_hidden: true,
            ..state()
        };
        assert!(!buttons(&all).any());
        assert!(
            !buttons(&State {
                mouselook: true,
                show_in_mouselook: true,
                ..all
            })
            .any()
        );
    }

    /// The click path: the face drawn for each state answers with its own
    /// action (the app turns Stand into the stand-up, Stop flying into the
    /// landing, Flycam into the flycam's toggle).
    #[test]
    fn a_click_on_each_button_reports_its_action() {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let p = theme.palette();
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 900.0));
        let bottom_top = 840.0;
        for (shown, expected) in [
            (
                Buttons {
                    stand: true,
                    ..Buttons::default()
                },
                Action::Stand,
            ),
            (
                Buttons {
                    stop_flying: true,
                    ..Buttons::default()
                },
                Action::StopFlying,
            ),
            (
                Buttons {
                    flycam: true,
                    ..Buttons::default()
                },
                Action::Flycam,
            ),
        ] {
            let mut time = 0.0;
            let mut frame = |events: Vec<egui::Event>| {
                time += 1.0 / 60.0;
                let mut action = None;
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(screen),
                        time: Some(time),
                        events,
                        ..Default::default()
                    },
                    |ui| action = show(ui.ctx(), &p, shown, bottom_top),
                );
                output.textures_delta.clear();
                action
            };
            for _ in 0..4 {
                frame(vec![]);
            }
            let panel = ctx.memory(|m| m.area_rect(Id::unique("stand_stop_flying"))).expect("the panel");
            assert!(panel.bottom() <= bottom_top, "the panel stays above the bottom bar");
            assert!((panel.center().x - screen.center().x).abs() <= 1.0, "the panel is centred");
            let at = panel.center();
            assert_eq!(frame(vec![egui::Event::PointerMoved(at)]), None);
            assert_eq!(
                frame(vec![egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                }]),
                None
            );
            assert_eq!(
                frame(vec![egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }]),
                Some(expected)
            );
        }
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            assert_eq!(show(ui.ctx(), &p, Buttons::default(), bottom_top), None);
        });
        output.textures_delta.clear();
    }
}
