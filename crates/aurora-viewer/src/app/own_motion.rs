//! Our avatar's movement each frame: Aurora's prediction (own_motion.rs)
//! and the demo's simulated link (demo_link.rs).

use super::*;
use crate::demo::link::{Delivery, DemoLink, Keys};
use crate::own_motion::Intent;

impl App {
    /// The run state the simulator was told (SetAlwaysRun): "Always run",
    /// a double-tap run, or the run to a double-clicked point.
    pub(super) fn running(&self) -> bool {
        self.always_run || self.temp_run.is_some() || self.autopilot.as_ref().is_some_and(|a| a.run)
    }

    /// Offline with AURORA_DEMO_LINK: the keys go to the stand-in simulator
    /// and its reports come back through the world, as on a grid.
    pub(super) fn drive_demo_link(&mut self, mouselook: bool, dt: f32) {
        let Some(cfg) = self.demo_link_cfg.clone() else {
            return;
        };
        let now = Instant::now();
        let keys = Keys {
            input: self.input,
            yaw: self.world.agent.yaw,
            flying: self.world.agent.flying,
            running: self.running(),
            mouselook,
        };
        let (position, yaw) = (self.world.agent.position, self.world.agent.yaw);
        let half_height = self
            .world
            .objects
            .index_of_uuid(&self.world.agent_id)
            .and_then(|i| self.world.objects.get(i))
            .map_or(0.84, |o| 0.5 * o.scale.z);
        let link = self
            .demo_link
            .get_or_insert_with(|| DemoLink::new(cfg, position, yaw, half_height, now));
        link.probe(now, &keys.input, position);
        for delivery in link.frame(now, keys, dt) {
            match delivery {
                Delivery::Event(ev, received) => {
                    if let Some(e) = self.world.apply_received(*ev, received) {
                        self.on_app_event(e);
                    }
                }
                Delivery::Landed => self.world.agent.flying = false,
            }
        }
        self.world.predict_agent(1.0, 0.0);
    }

    /// Aurora's movement of our avatar for this frame (Preferences › Aurora
    /// › Movement): the lead the keys still on their way give it, kept on
    /// the land, and the walk or stop of the keys over the simulator's
    /// signals. Off (the default), or offline without the simulated link,
    /// or during a teleport: the simulator's motion alone, as in Firestorm.
    pub(super) fn update_own_prediction(&mut self, mouselook: bool, dt: f32) {
        let now = Instant::now();
        let link = self.demo_link.as_ref().map(|l| l.cfg.clone());
        let wanted = link.as_ref().and_then(|c| c.prediction).unwrap_or(self.settings.predict_own_motion);
        let on = wanted && (!self.demo || link.is_some()) && self.world.movement_complete && !self.world.teleporting;
        self.world.agent.set_predicting(on);
        let rtt = match &link {
            Some(c) => c.ping,
            None => self.net.stats.ping_avg_ms.load(std::sync::atomic::Ordering::Relaxed) as f32 / 1000.0,
        };
        let intent = if on {
            let flags = self.world.agent.control_flags(&self.input, mouselook) | self.autopilot_flags;
            Intent::from_flags(flags, self.world.agent.yaw, self.running())
        } else {
            None
        };
        self.world.agent.step_prediction(intent, rtt, dt, now);
        self.world.constrain_agent_lead();
        // a local start or stop waits this long for the simulator's signal
        self.world.update_own_locomotion(intent, rtt + 0.6, now);
        if self.world.agent.prediction.calibration.take_changed()
            && self
                .own_motion_logged
                .is_none_or(|t| now.saturating_duration_since(t) >= Duration::from_secs(5))
        {
            log::info!("own motion learned: {}", self.world.agent.prediction.calibration.learned());
            self.own_motion_logged = Some(now);
        }
    }
}
