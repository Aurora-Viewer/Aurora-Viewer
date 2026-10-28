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
        link.follow_viewer(self.world.agent.seated, position);
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

    /// Offline, any prim of the demo can be sat on, and stood up from, as on
    /// a grid (the demo's own answers only knew two seats). With the
    /// simulated link the answers come back one round trip later. True when
    /// the command was answered here.
    pub(super) fn demo_sit_command(&mut self, cmd: &NetCommand) -> bool {
        match cmd {
            NetCommand::RequestSit { target, .. } => {
                // the demo's own seats keep their answer without the link
                let own = [970, 986].map(crate::demo::action_id).contains(target);
                if own && self.demo_link.is_none() {
                    return false;
                }
                let Some(seat) = self.demo_seat_root(target) else {
                    return false;
                };
                let Some(o) = self.world.objects.get(seat) else {
                    return false;
                };
                let (local_id, full_id, position, size) = (o.key.local_id, o.full_id, o.position, o.scale);
                log::info!("demo sit: on {local_id} at {position:.1?}");
                self.demo_seat = Some((position, size));
                self.demo_answer(crate::demo::sit_on_prim(local_id, full_id, size));
                true
            }
            NetCommand::OneShotControl(f) if f & control::STAND_UP != 0 => {
                if let Some((position, size)) = self.demo_seat.take() {
                    log::info!("demo sit: standing up");
                    self.demo_answer(crate::demo::stand_from_prim(position, size));
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    /// AURORA_DEMO_SIT_TP=local|region: at frame 300 our avatar sits on the
    /// nearest prim of the plaza, at frame 420 it teleports from the seat,
    /// in the region or to another one; at frame 700 the log says whether
    /// it still sits (the stale seat of a teleport, LLAgent::teleportCore).
    pub(super) fn demo_sit_teleport(&mut self) {
        if !self.demo || !matches!(self.frame_count, 300 | 420 | 700) {
            return;
        }
        let Ok(mode) = std::env::var("AURORA_DEMO_SIT_TP") else {
            return;
        };
        let me = self.world.agent.position;
        match self.frame_count {
            300 => {
                let seat = self
                    .world
                    .objects
                    .iter()
                    .filter(|(_, o)| o.parent_id == 0 && !o.is_avatar() && !o.is_tree() && o.attachment_point() == 0)
                    .filter(|(_, o)| Some(o.key.region) == self.world.main_region && o.scale.max_element() < 4.0)
                    .min_by(|a, b| a.1.position.distance(me).total_cmp(&b.1.position.distance(me)))
                    .map(|(_, o)| o.full_id);
                if let (Some(target), Some(handle)) = (seat, self.world.main_region) {
                    self.send(NetCommand::RequestSit {
                        handle,
                        target,
                        offset: Vec3::ZERO,
                    });
                }
            }
            420 => {
                log::info!("demo sit tp: before the teleport, seated {}", self.world.agent.seated);
                let (x, y) = (me.x - 6.0, me.y - 4.0);
                let to = Vec3::new(x, y, crate::demo::floor_at(x, y) + 0.9);
                let region = if mode.trim() == "region" {
                    aurora_i18n::tr!("app-aurora-elsewhere")
                } else {
                    aurora_i18n::tr!("common-aurora-demo")
                };
                self.teleport_to_location(region.into(), to);
            }
            _ => {
                let parent = self
                    .world
                    .objects
                    .index_of_uuid(&self.world.agent_id)
                    .and_then(|i| self.world.objects.get(i))
                    .map_or(0, |o| o.parent_id);
                log::info!(
                    "demo sit tp: arrived, seated {}, parent {parent}, seat animation {}",
                    self.world.agent.seated,
                    self.world
                        .animations_of(&self.world.agent_id)
                        .iter()
                        .any(|a| a.id == crate::demo::SEAT_ANIM)
                );
            }
        }
    }

    /// The demo simulator's side of a teleport from one of its seats: it
    /// unseats the avatar and stops the sit animation, but sends no update
    /// with the avatar's new parent (as a simulator whose updates after the
    /// teleport are terse ones): the viewer must get off by itself.
    pub(super) fn demo_unseat_on_teleport(&mut self) {
        if self.demo_seat.take().is_some() {
            log::info!("demo sit: the simulator unseats us for the teleport");
            self.demo_answer(vec![NetEvent::AvatarAnimations {
                avatar: crate::demo::DEMO_AGENT,
                anims: vec![(crate::demo::IDLE_ANIM, 1)],
                sources: Vec::new(),
            }]);
        }
    }

    /// The root prim of `target` when it is a prim of the scene (not an
    /// avatar nor something one wears).
    fn demo_seat_root(&self, target: &uuid::Uuid) -> Option<usize> {
        let mut idx = self.world.objects.index_of_uuid(target)?;
        for _ in 0..16 {
            let o = self.world.objects.get(idx)?;
            if o.is_avatar() {
                return None;
            }
            if o.parent_id == 0 {
                return Some(idx);
            }
            idx = self.world.objects.parent_of(o)?;
        }
        None
    }

    /// The demo simulator's answer: through the simulated link when there is
    /// one, at once otherwise.
    fn demo_answer(&mut self, events: Vec<NetEvent>) {
        if let Some(link) = &mut self.demo_link {
            link.reply(Instant::now(), events);
            return;
        }
        for ev in events {
            if let Some(e) = self.world.apply(ev) {
                self.on_app_event(e);
            }
        }
    }

    /// `AURORA_DEMO_LINK=…,trace`, or `AURORA_OWN_MOTION_TRACE=1` on a grid
    /// (only the frames where our avatar moves or a correction is left):
    /// one line per frame with the keys, the drawn position, the
    /// simulator's, the lead, the correction left, the camera and the drawn
    /// animations, then the flight, the simulator's velocity and the drawn
    /// velocity, to read a start, a stop, a take-off or a landing.
    pub(super) fn trace_own_motion(&self) {
        static GRID: std::sync::OnceLock<Option<Instant>> = std::sync::OnceLock::new();
        let grid = *GRID.get_or_init(|| std::env::var_os("AURORA_OWN_MOTION_TRACE").map(|_| Instant::now()));
        let a = &self.world.agent;
        let (offset, lead) = a.drawn_offsets();
        let t = match (self.demo_link.as_ref().filter(|l| l.cfg.trace), grid) {
            (Some(link), _) => link.elapsed(),
            (None, Some(start)) => {
                let still = a.drawn_velocity().length() < 0.05
                    && a.velocity.length() < 0.05
                    && lead == Vec3::ZERO
                    && offset.length() < 0.001
                    && !self.world.agent.prediction.keys_held();
                if !a.predicting() || still {
                    return;
                }
                start.elapsed().as_secs_f64()
            }
            (None, None) => return,
        };
        let anims: Vec<String> = self
            .world
            .animations_of(&self.world.agent_id)
            .iter()
            .map(|p| p.id.to_string()[..8].to_string())
            .collect();
        log::info!(
            "demo link trace: t={:.4} keys={} pos={:.4} {:.4} {:.4} server={:.4} {:.4} lead={:.4} {:.4} offset={:.4} {:.4} {:.4} cam={:.4} {:.4} {:.4} at={:.4} {:.4} {:.4} anims={} fly={} server_z={:.4} lead_z={:.4} svel={:.3} {:.3} {:.3} dvel={:.3} {:.3} {:.3}",
            t,
            self.input.forward,
            a.position.x,
            a.position.y,
            a.position.z,
            a.predicted_position().x,
            a.predicted_position().y,
            lead.x,
            lead.y,
            offset.x,
            offset.y,
            offset.z,
            self.camera.position.x,
            self.camera.position.y,
            self.camera.position.z,
            self.camera.target.x,
            self.camera.target.y,
            self.camera.target.z,
            anims.join(","),
            a.flying,
            a.predicted_position().z,
            lead.z,
            a.velocity.x,
            a.velocity.y,
            a.velocity.z,
            a.drawn_velocity().x,
            a.drawn_velocity().y,
            a.drawn_velocity().z
        );
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
            let flags = self.autopilot_controls(self.world.agent.control_flags(&self.input, mouselook));
            Intent::from_flags(flags, self.world.agent.yaw, self.running())
        } else {
            None
        };
        self.world.agent.step_prediction(intent, rtt, dt, now);
        self.world.constrain_agent_lead();
        // a local start or stop waits this long for the simulator's signal
        self.world
            .update_own_locomotion(intent, rtt + 0.6, now, self.ao_runtime.suppressed_locomotion());
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
