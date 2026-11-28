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
        if let Some(steps) = draw_stepping::begin(&mut self.settings, Instant::now()) {
            log::info!("draw distance stepping ({why}): {} m, up to {} m", steps.step, steps.user);
        }
    }

    /// An arrival: the gaps count from it (gTeleportArrivalTimer.reset in
    /// process_agent_movement_complete).
    pub(super) fn draw_stepping_arrived(&mut self) {
        if let Some(steps) = &mut self.settings.draw_stepping {
            steps.changed = Instant::now();
        }
    }

    /// Each frame, before the draw distance is read.
    pub(super) fn step_draw_distance(&mut self) {
        // (the readback is followed even without steps: the first frame of
        // the steps must not take the last measure from before for a new one)
        let load = self.draw_load();
        let Some(steps) = self.settings.draw_stepping else {
            return;
        };
        match draw_stepping::tick(
            steps,
            self.settings.draw_distance,
            self.settings.draw_distance_stepping,
            self.world.teleporting,
            load,
            self.draw_stepping_interval(),
            Instant::now(),
        ) {
            Tick::Wait => {}
            Tick::Hold(next) => self.settings.draw_stepping = Some(next),
            Tick::Step(next, value) => {
                if value < steps.step {
                    let wait = next.ceiling.map_or(0, |c| c.hold.as_secs());
                    log::info!(
                        "draw distance stepping: back to {value} m, the frame is too heavy ({} m not tried for {wait} s)",
                        steps.step
                    );
                } else {
                    log::info!("draw distance stepping: {value} m");
                }
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

    /// The arrival queue and the last finished frame. Absent GPU times stay
    /// absent: a device without timestamps does not pretend the frame was
    /// slow. Only a new measure counts, and not a photo's
    /// (`draw_stepping::Readback`): only while a photo's own frame is drawn,
    /// not while one is saved or its file dialog is open, when the frames
    /// are the world's and must still be able to step back.
    fn draw_load(&mut self) -> draw_stepping::Load {
        let photo = self.photo_frame_drawn();
        let tex = &self.scene.textures.stats;
        let waiting = tex
            .fetching
            .saturating_add(tex.decoding)
            .saturating_add(tex.pending_upload)
            .saturating_add(self.scene.stats.geom_pending);
        let shadow_ms = self
            .last_render
            .gpu_elements
            .map(|elements| elements[aurora_render::GpuElement::ShadowsDepth as usize]);
        let (gpu_ms, shadow_ms) = self.draw_stepping_readback.fresh(self.last_render.gpu_ms, shadow_ms, photo);
        draw_stepping::Load {
            waiting: waiting.min(u32::MAX as usize) as u32,
            gpu_ms,
            shadow_ms,
        }
    }
}
