//! CPU-only `--headless` frame stepping when no GPU renderer (`App::gfx`) is active.
//!
//! Keeps world perception, camera and interest-list updates (`AgentControls` /
//! `AgentUpdate`), FOV updates, autopilot locomotion, loading transitions, and
//! periodic network/world tasks running identically to windowed and
//! `--offscreen-gpu` sessions.

use super::{App, FOV_SEND_EPSILON, Screen};
use aurora_net::NetCommand;
use std::time::{Duration, Instant};

pub(super) fn headless_frame_input(dt: f32, aspect: f32, now: Instant, draw_distance: f32, region_width: f32) -> crate::camera::FrameInput {
    crate::camera::FrameInput {
        dt,
        now,
        steering: false,
        build_mode: false,
        edit_tracking: None,
        eye_offset: None,
        follow_joint_offset: None,
        head_offset: None,
        draw_distance,
        region_width,
        aspect,
    }
}

pub(super) fn headless_loading_ready(movement_complete: bool, loading_elapsed: Duration, terrain: f32, moved_secs: f32) -> bool {
    movement_complete && loading_elapsed > Duration::from_millis(1500) && (terrain > 0.97 || moved_secs > 6.0)
}

pub(super) fn fov_command_if_changed(fov_y: f32, sent_fov: &mut f32) -> Option<NetCommand> {
    if (fov_y - *sent_fov).abs() > FOV_SEND_EPSILON {
        *sent_fov = fov_y;
        Some(NetCommand::SetFov(fov_y))
    } else {
        None
    }
}

impl App {
    /// Steps one non-GPU frame for CPU-only `--headless` mode (`self.gfx` is `None`).
    pub(super) fn step_headless_frame(&mut self, dt: f32, still: bool, frozen: bool, world_now: impl Fn() -> Instant) -> bool {
        // Drain MCP commands first so mutations (camera_set, avatar_walk_to,
        // setting_set) reach the network controls in this same tick.
        self.update_mcp_bridge(None);

        if self.in_world() {
            self.step_draw_distance();
            self.update_autopilot(dt);
            self.update_selection_turn(dt);
            if self.settings.automatic_land {
                self.world.auto_land(&self.input);
            }
            self.update_native_movelock();
            self.update_movement();
            self.update_attention();

            let ml = self.camera.mouselook();
            let build = &self.build;
            let scene = &self.scene;
            self.world.advance_motion(
                world_now(),
                &self.net.stats.simulators.lock(),
                |key| build.is_selected(key),
                |idx| scene.drawable_damping_view(idx),
            );
            if self.demo && self.demo_link_cfg.is_some() {
                self.drive_demo_link(ml, dt);
            } else if self.demo {
                let p = self.world.agent.position;
                let floor = crate::demo::floor_at(p.x, p.y);
                self.world
                    .agent
                    .simulate_locally(&self.input, ml, Some(floor), dt, self.settings.automatic_land);
            } else if !still {
                let motion = *self.net.stats.simulator_motion.lock();
                if motion.handle == self.world.main_region {
                    self.world.predict_agent(motion.time_dilation, motion.packet_age());
                }
            }
            self.update_own_prediction(ml, dt);
            let camera_distance = self.camera.position.distance(self.world.agent.position);
            self.world.agent.update(&self.input, ml, dt, camera_distance);
            if self.world.agent.seated
                && let Some(p) = crate::scene::Scene::seated_agent_position(&self.world, world_now())
            {
                self.world.agent.follow_seat(p);
            }
            self.follow_awaited_zoom();

            let region_width = self.world.main().map_or(256.0, |r| r.heightmap.size_x as f32);
            let frame = headless_frame_input(dt, self.camera_aspect(), world_now(), self.settings.draw_distance, region_width);
            self.camera.update(&mut self.world, &self.settings.camera, &frame);
            if !frozen {
                self.world.agent.body_turn = self.world.update_bodies(
                    world_now(),
                    dt,
                    self.camera.forward(),
                    ml,
                    self.settings.walk_backwards_without_turning,
                    self.settings.avatar_turn_speed,
                );
            }
            self.revoke_on_sit();
            self.trace_own_motion();
            self.update_pointing();
            let w = self.settings.window_width.max(1280) as f32;
            let h = self.settings.window_height.max(720) as f32;
            self.update_look_at(w, h);

            let c = self.compose_agent_controls();
            self.net.set_controls(c);
            self.flush_agent_camera_commands();
        }

        self.update_voice();
        self.update_radar();
        self.update_bridge();
        self.update_region_restart();

        if let Screen::Loading { since } = self.screen {
            let moved = self.move_complete_at.map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0);
            let terrain = self.world.terrain_progress();
            if headless_loading_ready(self.world.movement_complete, since.elapsed(), terrain, moved) {
                self.screen = Screen::World;
                self.world.notifications.started = true;
                self.camera.snap();
                self.was_teleporting = self.world.teleporting;
            }
        }
        if self.world.arrival_overdue(Instant::now(), self.world.own_attachment_count()) {
            let kind = if self.world.teleporting {
                aurora_net::transition::Kind::Teleport
            } else {
                aurora_net::transition::Kind::Crossing
            };
            aurora_net::transition::end(
                Some(kind),
                "failed (the region did not confirm the arrival in time)",
                &self.net.stats,
            );
            self.demo_tp_start = None;
            self.world.give_up_arrival();
            self.net.send(NetCommand::ArrivalGivenUp);
        }

        self.world.objects.refresh_labelled();
        self.update_maps();
        self.poll_key_to_name();
        self.frame_start = Instant::now();
        self.frame_count += 1;
        self.crash_frame();
        if let Some(thread) = crate::crash::thread_died() {
            log::error!("the {thread} thread stopped after a panic: ending the session");
            crate::crash::failure(&format!("The {thread} thread stopped after a panic."));
            self.shutdown_headless();
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::MoveInput;
    use crate::autopilot::AutoPilot;
    use crate::camera::{Camera, CameraSettings};
    use crate::scene::avatar::AvatarLibrary;
    use crate::world::World;
    use aurora_net::{AgentControls, NetEvent, control};
    use glam::Vec3;
    use std::sync::Arc;

    fn skybox_world() -> (World, u64) {
        let mut world = World::new(Arc::new(AvatarLibrary::load()));
        let handle = (1000u64 << 32) | 1000u64;
        world.apply(NetEvent::AgentMovementComplete {
            handle,
            position: Vec3::new(160.0, 48.0, 3500.0),
            look_at: Vec3::X,
        });
        (world, handle)
    }

    #[test]
    fn headless_camera_and_controls_track_skybox_arrival() {
        let (mut world, _handle) = skybox_world();
        let settings = CameraSettings::default();
        let mut camera = Camera::default();
        camera.on_teleport_arrival(&settings, false);

        let frame = headless_frame_input(1.0 / 30.0, 1.0, Instant::now(), 192.0, 256.0);
        camera.update(&mut world, &settings, &frame);
        let controls = world.agent.controls(&MoveInput::default(), &camera, 192.0);

        assert!(
            controls.camera_center.distance(Vec3::new(160.0, 48.0, 3500.0)) < 15.0,
            "camera_center should follow skybox avatar arrival, got {:?}",
            controls.camera_center
        );
        assert_eq!(controls.far, 192.0);
        assert!(controls.update_worthy(&AgentControls::default()));
    }

    #[test]
    fn headless_free_camera_and_fov_propagate_to_controls_and_command() {
        let (mut world, _handle) = skybox_world();
        let mut settings = CameraSettings::default();
        let mut camera = Camera::default();
        let eye = Vec3::new(160.0, 42.0, 3502.5);
        let focus = Vec3::new(160.0, 48.0, 3502.0);
        camera.set_free_view(eye, focus, &mut world.agent, &settings);

        let new_fov = 75.0_f32.to_radians();
        settings.fov = new_fov;
        camera.fov_y = new_fov;

        let frame = headless_frame_input(1.0 / 30.0, 16.0 / 9.0, Instant::now(), 128.0, 256.0);
        camera.update(&mut world, &settings, &frame);
        let controls = world.agent.controls(&MoveInput::default(), &camera, 128.0);

        assert!(controls.camera_center.distance(eye) < 0.05);
        let expected_at = (focus - eye).normalize();
        assert!(controls.camera_at.dot(expected_at) > 0.999);

        let mut sent_fov = CameraSettings::default().fov;
        assert!(matches!(
            fov_command_if_changed(camera.fov_y, &mut sent_fov),
            Some(NetCommand::SetFov(fov)) if (fov - new_fov).abs() < 1e-6
        ));
        assert!(fov_command_if_changed(camera.fov_y, &mut sent_fov).is_none());
    }

    #[test]
    fn headless_loading_transition_matches_windowed_thresholds() {
        assert!(!headless_loading_ready(false, Duration::from_secs(10), 1.0, 10.0));
        assert!(!headless_loading_ready(true, Duration::from_millis(1500), 1.0, 10.0));
        assert!(!headless_loading_ready(true, Duration::from_secs(2), 0.97, 6.0));
        assert!(headless_loading_ready(true, Duration::from_secs(2), 0.98, 0.0));
        assert!(headless_loading_ready(true, Duration::from_secs(2), 0.0, 6.1));
    }

    #[test]
    fn headless_autopilot_emits_movement_controls_when_walking() {
        let (world, _handle) = skybox_world();
        let camera = Camera::default();
        let start = world.agent.position;
        let target = start + Vec3::new(10.0, 0.0, 0.0);
        let mut pilot = AutoPilot::start(start, target, false, Instant::now());
        let step = pilot.step(start, world.agent.yaw, 1.0 / 30.0);

        let mut controls = world.agent.controls(&MoveInput::default(), &camera, 128.0);
        controls.control_flags |= step.flags;

        assert_ne!(controls.control_flags & (control::AT_POS | control::NUDGE_AT_POS), 0);
        assert!(controls.update_worthy(&AgentControls::default()));
    }
}
