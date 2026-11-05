//! The floating Stand / Stop flying / Flycam buttons in the app
//! (`ui/stand_fly.rs`, LLPanelStandStopFlying): what a frame shows and what
//! each click does, with the same commands as the menus and the toolbar.

use super::App;
use crate::ui::stand_fly::{self, Action, Buttons};
use aurora_net::{NetCommand, control};

impl App {
    /// The buttons this frame shows.
    pub(super) fn stand_fly_buttons(&self) -> Buttons {
        stand_fly::buttons(&stand_fly::State {
            seated: self.world.agent.is_sitting(),
            flying: self.world.agent.flying,
            flycam: self.joystick.active,
            mouselook: self.camera.mouselook(),
            show_in_mouselook: self.settings.camera.show_interface_in_mouselook,
            // HideUIControls (LLViewToggleUI), immersive mode included
            ui_hidden: self.interface.hidden,
        })
    }

    pub(super) fn stand_fly_action(&mut self, action: Action) {
        match action {
            // LLPanelStandStopFlying::onStandButtonClick: nothing when RLVa
            // forbids it (RlvActions::canStand), else
            // LLSelectMgr::deselectAllForStandingUp then
            // AGENT_CONTROL_STAND_UP
            Action::Stand => {
                let stand = NetCommand::OneShotControl(control::STAND_UP);
                if !self.world.rlva_allows_command(&stand, uuid::Uuid::nil()) {
                    return;
                }
                self.build.deselect_all(&self.world);
                self.send(stand);
            }
            // onStopFlyingButtonClick: gAgent.setFlying(false)
            Action::StopFlying => {
                if self.world.agent.flying {
                    self.toggle_fly();
                }
            }
            // View.JoystickFlycam while it is on: off
            Action::Flycam => {
                if self.joystick.active {
                    self.toggle_joystick_flycam();
                }
            }
        }
    }

    /// AURORA_DEMO_STAND_FLY=ground|object|fly|stand|land: at frame 240 our
    /// avatar sits on the ground (`ground`, `stand`) or takes off (`fly`,
    /// `land`); at 300 `object` sits on the nearest prim. At 480 `stand`
    /// and `land` press the panel's button. The log gives the buttons shown
    /// at 470 and 560.
    pub(super) fn demo_stand_fly(&mut self) {
        if !self.demo || !matches!(self.frame_count, 240 | 300 | 470 | 480 | 560) {
            return;
        }
        let Ok(mode) = std::env::var("AURORA_DEMO_STAND_FLY") else {
            return;
        };
        let mode = mode.trim();
        match (self.frame_count, mode) {
            (240, "ground" | "stand") if !self.world.agent.is_sitting() => {
                self.send(NetCommand::OneShotControl(control::SIT_ON_GROUND));
            }
            (240, "fly" | "land") if !self.world.agent.flying => self.toggle_fly(),
            (300, "object") => self.demo_sit_on_nearest_prim(),
            (480, "stand") => self.stand_fly_action(Action::Stand),
            (480, "land") => self.stand_fly_action(Action::StopFlying),
            (470 | 560, _) => log::info!(
                "demo stand/fly panel: seated {}, flying {}, buttons {:?}",
                self.world.agent.is_sitting(),
                self.world.agent.flying,
                self.stand_fly_buttons()
            ),
            _ => {}
        }
    }
}
