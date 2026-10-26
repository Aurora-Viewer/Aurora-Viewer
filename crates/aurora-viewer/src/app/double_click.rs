//! The double click on the ground (src/autopilot.rs): walk, run or
//! teleport to the point clicked, as Firestorm's `walk_to` / `teleport_to`
//! double-click bindings do (LLToolPie::walkToClickedLocation and
//! teleportToClickedLocation, indra/newview/lltoolpie.cpp, originally LGPL
//! 2.1), and the walk itself, frame after frame (LLAgent::autoPilot).

use super::*;
use crate::autopilot::{self, AutoPilot, DoubleClick, Finish};

impl App {
    /// What a double click on the ground does in this session.
    pub(super) fn double_click_action(&self) -> DoubleClick {
        match autopilot::demo_action() {
            Some(a) if self.demo => a,
            _ => self.settings.double_click,
        }
    }

    /// Half the height of our avatar: how high above the ground its centre
    /// stands (LLVOAvatar::getPelvisToFoot, read from the size the
    /// simulator gives the avatar object).
    pub(super) fn pelvis_to_foot(&self) -> f32 {
        self.world
            .objects
            .index_of_uuid(&self.world.agent_id)
            .and_then(|i| self.world.objects.get(i))
            .map(|o| o.scale.z * 0.5)
            .filter(|h| h.is_finite() && *h > 0.25)
            .unwrap_or(0.95)
    }

    /// The second press of a double click landed on `point` of the ground
    /// or of an object without a click action (the click actions and the
    /// avatar's own body took theirs before).
    pub(super) fn double_click_ground(&mut self, point: Vec3) {
        let action = self.double_click_action();
        if self.demo && std::env::var_os("AURORA_DEMO_DOUBLECLICK").is_some() {
            log::info!("demo double click: {action:?} at {point:?}");
        }
        match action {
            DoubleClick::Nothing => {}
            DoubleClick::Walk => self.walk_to(point, false),
            DoubleClick::Run => self.walk_to(point, true),
            DoubleClick::Teleport => self.teleport_to_point(point),
        }
    }

    /// walkToClickedLocation: not while flying or manually sitting. Our own
    /// idle anchor releases before the walk and does not cancel its handover.
    fn walk_to(&mut self, point: Vec3, run: bool) {
        let agent = &self.world.agent;
        if agent.flying || self.native_anchor.blocks_walk(agent.seated, agent.ground_sit) || !point.is_finite() {
            return;
        }
        // the target stays its centre's height above the ground
        let target = point + Vec3::Z * self.pelvis_to_foot();
        let was_running = self.autopilot.as_ref().is_some_and(|a| a.run);
        self.world.following_avatar = None;
        self.follow_recovery = None;
        self.follow_ui.open = false;
        self.autopilot = Some(AutoPilot::start(agent.position, target, run, Instant::now()));
        self.autopilot_stop_pending = false;
        self.autopilot_flags = 0;
        self.release_native_movelock();
        if run != was_running {
            self.sync_run();
        }
    }

    /// LLLandSit::handleEvent: approach within 0.7 m, then near_sit_down_point.
    pub(super) fn sit_at(&mut self, point: Vec3) {
        if !point.is_finite()
            || self.world.agent.is_sitting()
            || self.world.agent.flying
            || self.native_anchor.blocks_walk(self.world.agent.seated, self.world.agent.ground_sit)
        {
            return;
        }
        self.walk_to(point, false);
        if let Some(pilot) = &mut self.autopilot {
            *pilot = AutoPilot::start_to(
                self.world.agent.position,
                pilot.target,
                0.7,
                autopilot::Arrival::SitGround,
                Instant::now(),
            );
        }
    }

    /// teleportToClickedLocation: teleportViaLocationLookAt toward where the
    /// camera looks, the point raised by PelvisToFoot.
    fn teleport_to_point(&mut self, point: Vec3) {
        let (Some(origin), Some(main)) = (self.world.main_origin(), self.world.main_region) else {
            return;
        };
        if !point.is_finite() {
            return;
        }
        let size = self.world.main().map_or(256.0, |r| r.heightmap.size_x as f32);
        let (slot, position) = autopilot::teleport_destination(origin, size, point, self.pelvis_to_foot());
        let here = slot == origin;
        let handle = if here { main } else { aurora_net::origin_to_handle(slot.0, slot.1) };
        self.stop_autopilot(true);
        let label = if here {
            format!(
                "{} ({:.0}, {:.0}, {:.0})",
                self.world.region_name(),
                position.x,
                position.y,
                position.z
            )
        } else {
            format!(
                "({:.0}, {:.0}, {:.0})",
                slot.0 as f32 + position.x,
                slot.1 as f32 + position.y,
                position.z
            )
        };
        self.request_user_teleport(
            NetCommand::TeleportTo {
                handle,
                position,
                look_at: self.camera.forward(),
            },
            label,
            !here,
        );
    }

    /// Running while walking there, besides "Always run" and a double-tap
    /// run (SetAlwaysRun is one state for all three).
    fn sync_run(&mut self) {
        let run = self.running();
        self.send(NetCommand::SetAlwaysRun(run));
    }

    pub(super) fn travel_to_avatar(&mut self, id: uuid::Uuid, follow: bool) {
        let agent = &self.world.agent;
        if !self.in_world() || self.native_anchor.blocks_walk(agent.seated, agent.ground_sit) {
            return;
        }
        let Some(target) = autopilot::avatar_target(&self.world, id, self.pelvis_to_foot() * 2.0) else {
            return;
        };
        let mut pilot = AutoPilot::avatar(agent.position, target.position, follow.then_some(id), agent.flying, Instant::now());
        pilot.subject = Some(id);
        if follow {
            pilot.set_follow_distance(self.settings.follow.bounded_distance(), agent.position);
        }
        pilot.observe_avatar(target, agent.position);
        self.stop_autopilot(true);
        self.world.following_avatar = pilot.leader;
        if follow {
            ui::widgets::open_floater(&self.egui_ctx, "follow", &mut self.follow_ui.open);
        }
        self.autopilot = Some(pilot);
        self.autopilot_stop_pending = false;
        self.autopilot_flags = 0;
        self.release_native_movelock();
        if self.autopilot.as_ref().is_some_and(|ap| ap.run) {
            self.sync_run();
        }
    }

    /// The demo feeds the pilot's flags into local physics as keys. Remove
    /// those synthesized movement bits before sending/predicting controls,
    /// retaining the pilot's exact slow/fast/stop request.
    pub(super) fn autopilot_controls(&self, mut flags: u32) -> u32 {
        if self.autopilot.is_some() {
            flags &= !(control::AT_POS
                | control::AT_NEG
                | control::FAST_AT
                | control::NUDGE_AT_POS
                | control::NUDGE_AT_NEG
                | control::UP_POS
                | control::UP_NEG
                | control::FAST_UP);
        }
        flags | self.autopilot_flags
    }

    /// stopAutoPilot: the avatar stops (AGENT_CONTROL_STOP this frame).
    pub(super) fn stop_autopilot(&mut self, user_cancel: bool) {
        if user_cancel {
            self.world.cancel_friend_join();
        }
        self.follow_ui.open = false;
        self.follow_recovery = None;
        self.world.following_avatar = None;
        let Some(ap) = self.autopilot.take() else {
            return;
        };
        self.autopilot_flags = control::STOP;
        self.autopilot_stop_pending = true;
        if ap.run {
            self.sync_run();
        }
        if let Some(arrival) = ap.on_arrival {
            let success = !user_cancel && ap.reached(self.world.agent.position);
            self.autopilot_arrived(arrival, success, user_cancel);
        }
        if self.demo && std::env::var_os("AURORA_DEMO_DOUBLECLICK").is_some() {
            log::info!(
                "demo walk ended ({}): at {:?}, target {:?}",
                if user_cancel { "cancelled" } else { "done" },
                self.world.agent.position,
                ap.target
            );
        }
    }

    /// A frame of the walk, after the keys are read: a movement key, a manual
    /// flight change or sitting ends it (LLAgent::moveAt & co -> resetView ->
    /// stopAutoPilot(true)); otherwise the body turns and the flags go out
    /// with this frame's AgentUpdate.
    pub(super) fn update_autopilot(&mut self, dt: f32) {
        self.autopilot_flags = if std::mem::take(&mut self.autopilot_stop_pending) {
            control::STOP
        } else {
            0
        };
        if self.autopilot.is_none() && self.follow_recovery.is_none() {
            return;
        }
        let i = &self.input;
        let keys = i.forward
            || i.back
            || i.turn_left
            || i.turn_right
            || i.strafe_left
            || i.strafe_right
            || i.up
            || i.down
            || i.joystick_yaw.abs() > 0.001
            || i.joystick_pitch.abs() > 0.001;
        let agent = &self.world.agent;
        let cancelled = keys
            || self.mouse_mode == MouseMode::Steer
            || self
                .autopilot
                .as_ref()
                .is_some_and(|ap| ap.flying != agent.flying && (!ap.landing() || agent.flying))
            || self.native_anchor.blocks_walk(agent.seated, agent.ground_sit)
            || (!self.in_world() && !self.world.teleporting);
        if self.resume_follow_recovery(cancelled) {
            return;
        }
        if cancelled {
            self.stop_autopilot(true);
            return;
        }
        let (position, yaw) = (self.world.agent.position, self.world.agent.yaw);
        let was_running = self.running();
        if let Some(id) = self.autopilot.as_ref().and_then(|ap| ap.subject) {
            let Some(target) = autopilot::avatar_target(&self.world, id, self.pelvis_to_foot() * 2.0) else {
                self.stop_autopilot(true);
                return;
            };
            if let Some(ap) = &mut self.autopilot {
                ap.observe_avatar(target, position);
            }
        }
        if self.running() != was_running {
            self.sync_run();
        }
        if !self.world.agent.flying && self.autopilot.as_ref().is_some_and(|ap| ap.wants_flight()) {
            if !autopilot::can_takeoff(&self.world) {
                log::info!("autopilot: taking off is not permitted, stopped");
                self.stop_autopilot(true);
                return;
            }
            self.release_native_movelock();
            self.movement_runtime.cancel_water_exit();
            self.world.agent.flying = true;
        }
        let Some(ap) = self.autopilot.as_mut() else {
            return;
        };
        ap.set_flying(self.world.agent.flying, position);
        autopilot::land_near_target(&mut self.world, ap);
        // Flight plans its coast from physical reports; flight_response adds
        // their age and the link delay exactly once, without drawable lead.
        let steering = if ap.flying { self.world.agent.server_pos } else { position };
        let rtt = self.demo_link.as_ref().map_or_else(
            || self.net.stats.ping_avg_ms.load(std::sync::atomic::Ordering::Relaxed) as f32 / 1000.0,
            |link| link.cfg.ping + link.cfg.jitter,
        );
        let calibration = &self.world.agent.prediction.calibration;
        let response = autopilot::flight_response(calibration, rtt, self.world.agent.server_age());
        let step = ap.step_moving(steering, self.world.agent.velocity, yaw, dt.clamp(1e-3, 0.1), response);
        let run = ap.run;
        self.world.agent.yaw += step.yaw;
        self.autopilot_flags = step.flags;
        if let Some(finish) = step.finish {
            if finish == Finish::NoProgress {
                if self.recover_blocked_follow() {
                    return;
                }
                log::info!("autopilot: no nearer for too long, stopped");
            }
            self.stop_autopilot(false);
        } else if self.demo {
            if std::env::var("AURORA_DEMO_AVATAR_TRAVEL").is_ok_and(|mode| mode.starts_with("stuck-")) {
                // Offline stand-in for a wall: the pilot asks to walk, but
                // neither local physics nor AgentUpdate advances our body.
                self.autopilot_flags = control::STOP;
                return;
            }
            // no simulator offline: the local stand-in walks on the keys
            self.input.forward |= step.flags & (control::AT_POS | control::NUDGE_AT_POS) != 0;
            self.input.back |= step.flags & control::AT_NEG != 0;
            self.input.nudge_fb = step.flags & control::FAST_AT == 0;
            self.input.up |= step.flags & control::UP_POS != 0;
            self.input.down |= step.flags & control::UP_NEG != 0;
            self.input.run |= run;
        }
    }

    /// The destination of the walk, marked for a few seconds (the HUD blob
    /// of walkToClickedLocation), hidden by what stands in front of it.
    pub(super) fn draw_walk_mark(&self, ctx: &egui::Context, p: &crate::theme::Palette, proj: &ui::hud::Projector) {
        let Some(ap) = &self.autopilot else {
            return;
        };
        let age = ap.started.elapsed();
        if age >= autopilot::MARK_DURATION {
            return;
        }
        let ground = ap.target - Vec3::Z * self.pelvis_to_foot();
        let Some((c, _)) = proj.project(ground) else {
            return;
        };
        let fade = 1.0 - age.as_secs_f32() / autopilot::MARK_DURATION.as_secs_f32();
        let eye = self.camera.position;
        let to = eye - ground;
        let lift = if to.length() > 1e-3 { to.normalize() * 0.15 } else { Vec3::ZERO };
        aurora_render::world_label_at(ctx, Some(ground + lift));
        let painter = ui::hud::world_painter(ctx);
        let color = p.teal.gamma_multiply(fade);
        painter.circle_filled(c, 10.0, color.gamma_multiply(0.12));
        painter.circle_filled(c, 4.5, p.navy.gamma_multiply(0.7 * fade));
        painter.circle_filled(c, 3.5, color);
        aurora_render::world_label_at(ctx, None);
    }

    /// AURORA_DEMO_DOUBLECLICK=menu[,y]: the wolf menu opened at frame 540,
    /// Me hovered at 560 and the "Double-click on the ground" entry (`y`
    /// points from the top of the window) at 580: capture frame 620.
    pub(super) fn double_click_menu_demo_input(&mut self, raw: &mut egui::RawInput) {
        let Ok(v) = std::env::var("AURORA_DEMO_DOUBLECLICK") else {
            return;
        };
        let mut parts = v.split(',');
        if parts.next().map(str::trim) != Some("menu") {
            return;
        }
        let y: f32 = parts.next().and_then(|s| s.trim().parse().ok()).unwrap_or(207.0);
        raw.focused = true;
        let mouse = match self.frame_count {
            540 => Some((egui::pos2(14.0, 11.0), Some(true))),
            541 => Some((egui::pos2(14.0, 11.0), Some(false))),
            560 => Some((egui::pos2(100.0, 38.0), None)),
            // across the Me submenu to the entry
            570 => Some((egui::pos2(240.0, 38.0), None)),
            580 => Some((egui::pos2(240.0, y), None)),
            _ => None,
        };
        if let Some((pos, pressed)) = mouse {
            raw.events.push(egui::Event::PointerMoved(pos));
            if let Some(pressed) = pressed {
                raw.events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
            }
        }
    }

    /// AURORA_DEMO_DOUBLECLICK=walk|run|tp[,x,y]: once the world shows, a
    /// double click with the left button at pixel (x, y) of the window (on
    /// the ground ahead and to the left by default), through the same way
    /// in as the mouse.
    pub(super) fn demo_double_click_steps(&mut self) {
        let Ok(v) = std::env::var("AURORA_DEMO_DOUBLECLICK") else {
            return;
        };
        const START: u64 = 700;
        if v.starts_with("menu") || self.frame_count < START || self.frame_count > START + 3 || !matches!(self.screen, Screen::World) {
            return;
        }
        let size = self.gfx.as_ref().map_or((1280, 720), |g| g.renderer.size());
        let c: Vec<f32> = v.split(',').skip(1).filter_map(|s| s.trim().parse().ok()).collect();
        self.cursor_pos = match c[..] {
            [x, y, ..] => (x, y),
            _ => (size.0 as f32 * 0.3, size.1 as f32 * 0.82),
        };
        let pressed = (self.frame_count - START).is_multiple_of(2);
        self.mouse_input(MouseButton::Left, pressed, false);
    }

    /// Offline avatar approach and follow, through the context action route.
    pub(super) fn demo_avatar_travel_steps(&mut self) {
        let Ok(mode) = std::env::var("AURORA_DEMO_AVATAR_TRAVEL") else {
            return;
        };
        let id = crate::demo::DEMO_LOUP;
        let frame = self.frame_count;
        let flight = match (mode.as_str(), frame) {
            ("flight", 900) | ("go-flight" | "go-flight-late" | "flight-brake", 590) => Some(Some(6.0)),
            ("flight", 1500 | 2700) => Some(None),
            ("flight", 2100) => Some(Some(4.0)),
            _ => None,
        };
        if let Some(height) = flight {
            if let Some(idx) = self.world.objects.index_of_uuid(&id)
                && let Some(o) = self.world.objects.get_mut(idx)
            {
                let floor = crate::demo::floor_at(o.position.x, o.position.y);
                o.position.z = floor + 0.84 + height.unwrap_or(0.0);
                o.velocity = Vec3::ZERO;
                o.acceleration = Vec3::ZERO;
                o.foot_plane = height.is_none().then_some(Vec3::Z.extend(floor));
                if mode == "flight-brake" {
                    o.position.x += 35.0;
                } else if mode == "go-flight-late" {
                    o.position.x = self.world.agent.position.x + 1.0;
                    o.position.y = self.world.agent.position.y;
                }
            }
            self.world.apply(NetEvent::AvatarAnimations {
                avatar: id,
                anims: vec![(
                    if height.is_some() {
                        crate::own_motion::ANIM_HOVER
                    } else {
                        crate::own_motion::ANIM_STAND
                    },
                    frame as i32,
                )],
                sources: Vec::new(),
            });
            if mode == "go-flight-late" {
                self.world.animations.remove(&id);
            }
        }
        if frame == 620 && mode == "go-flight-late" {
            self.world.apply(NetEvent::AvatarAnimations {
                avatar: id,
                anims: vec![(crate::own_motion::ANIM_HOVER, frame as i32)],
                sources: Vec::new(),
            });
        }
        if mode == "run" && matches!(frame, 590 | 1200 | 1500) {
            let current = autopilot::avatar_position(&self.world, id);
            if let Some(o) = self.world.objects.index_of_uuid(&id).and_then(|idx| self.world.objects.get(idx)) {
                let velocity = Vec3::X
                    * match frame {
                        590 => 5.13,
                        1200 => 3.2,
                        _ => 0.0,
                    };
                let update = aurora_net::objects::TerseUpdate {
                    local_id: o.key.local_id,
                    state: o.state,
                    is_avatar: true,
                    foot_plane: o.foot_plane,
                    position: current.unwrap_or(o.position),
                    velocity,
                    acceleration: Vec3::ZERO,
                    rotation: o.rotation,
                    angular_velocity: Vec3::ZERO,
                    texture_entry: None,
                };
                self.world.apply(NetEvent::TerseUpdates {
                    handle: o.key.region,
                    updates: vec![update],
                });
            }
            self.world.apply(NetEvent::AvatarAnimations {
                avatar: id,
                anims: vec![(
                    match frame {
                        590 => crate::own_motion::ANIM_RUN,
                        1200 => crate::own_motion::ANIM_WALK,
                        _ => crate::own_motion::ANIM_STAND,
                    },
                    frame as i32,
                )],
                sources: Vec::new(),
            });
        }
        if frame == 600 && mode != "menu" {
            self.settings.follow = autopilot::FollowSettings {
                distance: if mode == "controls-distance" { 6.0 } else { 2.0 },
                teleport_if_stuck: matches!(mode.as_str(), "stuck-teleport" | "stuck-denied"),
            };
            if mode == "stuck-denied" {
                self.settings.rlva_enabled = true;
                self.world.rlva_configure(true);
                self.rlva_demo_message("@tploc=n,tplocal:0=n".into());
            }
            self.on_ctx_action(if matches!(mode.as_str(), "go" | "go-flight" | "go-flight-late") {
                ui::context::CtxAction::GoToAvatar(id)
            } else {
                ui::context::CtxAction::FollowAvatar(id)
            });
        }
        if ((frame == 600 && mode == "menu") || (frame == 1400 && mode == "follow-menu"))
            && let Some(point) = autopilot::avatar_position(&self.world, id)
        {
            self.context_menu = Some(ui::context::ContextMenu {
                pos: egui::pos2(240.0, 120.0),
                point,
                target: ui::context::Target::Avatar {
                    id,
                    own: false,
                    attachment: None,
                },
                pie_path: Vec::new(),
                pie_input_frame: None,
            });
        }
        if frame == 1500
            && matches!(mode.as_str(), "go" | "follow")
            && let Some(idx) = self.world.objects.index_of_uuid(&id)
            && let Some(o) = self.world.objects.get_mut(idx)
        {
            o.position += Vec3::X * 6.0;
            o.velocity = Vec3::ZERO;
            o.acceleration = Vec3::ZERO;
        }
        if frame == 1500 {
            match mode.as_str() {
                "stop" => self.on_ctx_action(ui::context::CtxAction::StopFollowing),
                "escape" => self.reset_camera_view(),
                "manual" => self.controls_held.push(Action::Forward),
                "lost" => {
                    if let Some(idx) = self.world.objects.index_of_uuid(&id) {
                        self.world.objects.remove_idx(idx);
                    }
                }
                _ => {}
            }
        }
        if frame == 1502 && mode == "manual" {
            self.controls_held.retain(|a| *a != Action::Forward);
        }
        if matches!(
            frame,
            601 | 901 | 1201 | 1400 | 1501 | 2000 | 2101 | 2400 | 2600 | 2701 | 3200 | 4400
        ) {
            let distance = autopilot::avatar_position(&self.world, id).map(|p| (p - self.world.agent.position).truncate().length());
            let height_error = autopilot::avatar_position(&self.world, id).map(|p| p.z - self.world.agent.position.z);
            log::info!(
                "demo avatar travel: mode={mode}, frame={frame}, distance={distance:?}, height_error={height_error:?}, active={}, following={}, flying={}, landing={}, altitude={:.3}, flags={:#x}, running={}, speed={:.3}",
                self.autopilot.is_some(),
                self.world.following_avatar.is_some(),
                self.world.agent.flying,
                self.autopilot.as_ref().is_some_and(|ap| ap.landing()),
                self.world.agent.position.z,
                self.autopilot_flags,
                self.running(),
                self.world.agent.velocity.length(),
            );
        }
    }
}
