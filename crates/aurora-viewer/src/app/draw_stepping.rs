//! Progressive draw distance (settings/draw_stepping.rs): started at the
//! first arrival after the login and when a teleport to another region
//! finishes, stepped each frame.
use super::*;
use crate::settings::draw_stepping::{self, Tick};
use std::time::Duration;

impl App {
    fn draw_stepping_interval(&self) -> Duration {
        Duration::from_secs(u64::from(self.settings.draw_distance_stepping_interval))
    }

    /// The login's first arrival, or a teleport to another region finished
    /// (LLStartUp STATE_STARTED, LLAgent::teleportCore).
    pub(super) fn start_draw_stepping(&mut self, why: &str) {
        if !self.settings.draw_distance_stepping {
            return;
        }
        let (steps, value) = draw_stepping::start(
            self.settings.draw_stepping,
            self.settings.draw_distance,
            self.draw_stepping_interval(),
            Instant::now(),
        );
        log::info!("draw distance stepping ({why}): {value} m, up to {} m", steps.user);
        self.settings.draw_stepping = Some(steps);
        self.settings.draw_distance = value;
    }

    /// An arrival: the next step is a whole interval away
    /// (gTeleportArrivalTimer.reset in process_agent_movement_complete).
    pub(super) fn draw_stepping_arrived(&mut self) {
        let interval = self.draw_stepping_interval();
        if let Some(steps) = &mut self.settings.draw_stepping {
            steps.next = Instant::now() + interval;
        }
    }

    /// Each frame, before the draw distance is read.
    pub(super) fn step_draw_distance(&mut self) {
        let Some(steps) = self.settings.draw_stepping else {
            return;
        };
        match draw_stepping::tick(
            steps,
            self.settings.draw_distance,
            self.settings.draw_distance_stepping,
            self.world.teleporting,
            self.draw_stepping_interval(),
            Instant::now(),
        ) {
            Tick::Wait => {}
            Tick::Step(next, value) => {
                log::info!("draw distance stepping: {value} m");
                self.settings.draw_stepping = Some(next);
                self.settings.draw_distance = value;
            }
            Tick::End(value) => {
                log::info!("draw distance stepping: done at {value} m");
                self.settings.draw_stepping = None;
                self.settings.draw_distance = value;
            }
        }
    }
}
