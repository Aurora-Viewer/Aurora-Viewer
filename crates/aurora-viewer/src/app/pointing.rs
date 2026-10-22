use super::App;
use crate::world::{lookat, pointat};
use aurora_net::NetCommand;
use glam::DVec3;
use std::time::Instant;

impl App {
    /// LLSelectMgr::updatePointAt / LLAgentCamera::setPointAt. The remote
    /// viewer starts its editing motion from the point-at effect itself.
    pub(super) fn update_pointing(&mut self) {
        if self.demo
            && let Ok(mode) = std::env::var("AURORA_DEMO_POINTING")
        {
            crate::demo::pointing::tick(
                &mut self.world,
                &mut self.scene,
                &mut self.build,
                &mut self.settings.build,
                self.frame_count,
                &mode,
            );
            if self.frame_count == 600 {
                self.settings.lookat.private_point_at = false;
                self.camera.demo_orbit(2.6, 0.2, Some(5.0), &self.settings.camera);
                self.panels.people = false;
                self.panels.chat = false;
            }
            if self.frame_count == 840 && mode == "private" {
                self.settings.lookat.private_point_at = true;
            }
        }
        let now = Instant::now();
        let selected = self.build.pointing_target(&self.world, now);
        let previous = self.world.point_at.own.target;
        if let Some(t) = selected {
            self.set_look_at(lookat::SELECT, t.object, t.offset);
        } else if previous.kind != 0 || self.world.look_at.own.target.kind == lookat::SELECT {
            self.set_look_at(lookat::CLEAR, None, DVec3::ZERO);
        }
        let target = selected.filter(|_| !self.settings.lookat.private_point_at);
        let points = &mut self.world.point_at;
        // A released grab must yield to a selection's lower priority.
        if previous.kind == pointat::GRAB && target.is_none_or(|t| t.kind != pointat::GRAB) {
            points.set(None, now);
        }
        points.set(target, now);
        let own = &mut points.own;
        if own.needs_send {
            let cmd = NetCommand::PointAt {
                effect: own.effect,
                target: own.target.object.unwrap_or_default(),
                offset: own.target.offset.to_array(),
                kind: own.target.kind,
                duration: own.duration,
            };
            own.sent(now);
            self.send(cmd);
        }
    }
}
