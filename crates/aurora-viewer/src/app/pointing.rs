use super::App;
use crate::world::{lookat, pointat};
use aurora_net::NetCommand;
use glam::DVec3;
use std::time::Instant;
use uuid::Uuid;

impl App {
    /// LLSelectMgr::updatePointAt / LLAgentCamera::setPointAt. The remote
    /// viewer starts its editing motion from the point-at effect itself.
    pub(super) fn update_pointing(&mut self) {
        if self.demo
            && let Ok(mode) = std::env::var("AURORA_DEMO_POINTING")
        {
            if self.frame_count == 600 {
                self.settings.ao.accounts.entry(self.world.agent_id).or_default().enabled = false;
                self.settings.bridge.entry(self.world.agent_id).or_default().enabled = false;
                self.ao_ui.prototype = None;
                self.release_native_movelock();
            }
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
                if mode.starts_with("remote")
                    && let Some(idx) = self.world.objects.index_of_uuid(&crate::demo::DEMO_LOUP)
                    && let Some((at, _, _)) = crate::scene::Scene::object_transform(&self.world, idx, Instant::now(), 0)
                {
                    self.camera.set_view(
                        at + glam::Vec3::new(-4.0, -3.0, 1.5),
                        at,
                        &mut self.world.agent,
                        &self.settings.camera,
                    );
                }
                self.panels.people = false;
                self.panels.chat = false;
                self.panels.inventory = false;
                self.panels.ao = false;
            }
            if self.frame_count == 840 && mode == "private" {
                self.settings.lookat.private_point_at = true;
            }
        }
        if self.demo
            && let Ok(mode) = std::env::var("AURORA_DEMO_BEAMS")
        {
            crate::demo::beams::tick(&mut self.world, &mut self.build, &mut self.settings.build, self.frame_count, &mode);
            if self.frame_count == 600 {
                if let Some(style) = crate::demo::beams::style(&mode) {
                    self.settings.beams.style = style;
                }
                self.panels.people = false;
                self.panels.chat = false;
                self.panels.inventory = false;
                self.panels.appearance = false;
                if let Some((avatar, cube)) = crate::demo::beams::view(&self.world, &mode) {
                    // from the side, both in view; in `self`, right of the
                    // Build window
                    let mid = (avatar + cube) * 0.5;
                    let side = (cube - avatar).cross(glam::Vec3::Z).normalize_or_zero();
                    let eye = mid - side * 4.0 + glam::Vec3::Z;
                    let right = (mid - eye).cross(glam::Vec3::Z).normalize_or_zero();
                    let look = if mode == "self" { mid - right * 1.2 } else { mid };
                    self.camera.set_view(eye, look, &mut self.world.agent, &self.settings.camera);
                }
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
        self.update_own_beam(target, now);
        let own = &mut self.world.point_at.own;
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

    /// The one-second beam at what we give to or drop into (LLGiveInventory::
    /// doGiveInventoryItem, LLToolDragAndDrop::dropObject / dropInventory /
    /// dropScript): to the `target` object, or to the global `position` when
    /// it is nil. A give to an avatar not in view has no beam (its object is
    /// not found).
    pub(super) fn fire_beam(&mut self, target: Uuid, position: DVec3) {
        if !target.is_nil() && self.world.objects.index_of_uuid(&target).is_none() {
            return;
        }
        let me = self.world.agent_id;
        let send = self
            .world
            .beams
            .give(me, target, position, self.settings.beams.color, Instant::now());
        self.send(NetCommand::Beam {
            effect: send.effect,
            target: send.target,
            position: send.position.to_array(),
            color: send.color,
            duration: send.duration,
        });
    }

    /// LLVOAvatarSelf::idleUpdateTractorBeam: while we edit or grab (and the
    /// point-at is not private, PrivatePointAtTarget), our beam points where
    /// the point-at does, as a global position, and is sent every 0.25 s.
    fn update_own_beam(&mut self, target: Option<lookat::Target>, now: Instant) {
        let me = self.world.agent_id;
        let aim = target
            .and_then(|t| self.scene.look_target_pos(&self.world, me, &t, now))
            .zip(self.world.main_origin())
            .map(|(p, (mx, my))| DVec3::new(f64::from(p.x) + mx as f64, f64::from(p.y) + my as f64, f64::from(p.z)));
        let color = self.settings.beams.color;
        if let Some(send) = self.world.beams.update_own(me, aim, color, now) {
            self.send(NetCommand::Beam {
                effect: send.effect,
                target: send.target,
                position: send.position.to_array(),
                color: send.color,
                duration: send.duration,
            });
        }
        let objects = &self.world.objects;
        self.world.beams.forget(now, |id| objects.index_of_uuid(id).is_some());
    }
}
