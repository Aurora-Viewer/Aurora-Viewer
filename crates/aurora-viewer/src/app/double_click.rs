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
        self.autopilot = Some(AutoPilot::start(agent.position, target, run, Instant::now()));
        self.release_native_movelock();
        if run != was_running {
            self.sync_run();
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

    /// stopAutoPilot: the avatar stops (AGENT_CONTROL_STOP this frame).
    pub(super) fn stop_autopilot(&mut self, user_cancel: bool) {
        let Some(ap) = self.autopilot.take() else {
            return;
        };
        self.autopilot_flags |= control::STOP;
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

    /// A frame of the walk, after the keys are read: a movement key, flying
    /// or sitting ends it (LLAgent::moveAt & co -> resetView ->
    /// stopAutoPilot(true)); otherwise the body turns and the flags go out
    /// with this frame's AgentUpdate.
    pub(super) fn update_autopilot(&mut self, dt: f32) {
        self.autopilot_flags = 0;
        if self.autopilot.is_none() {
            return;
        }
        let i = &self.input;
        let keys = i.forward || i.back || i.turn_left || i.turn_right || i.strafe_left || i.strafe_right || i.up || i.down;
        let agent = &self.world.agent;
        if keys
            || self.mouse_mode == MouseMode::Steer
            || agent.flying
            || self.native_anchor.blocks_walk(agent.seated, agent.ground_sit)
            || !self.in_world()
        {
            self.stop_autopilot(true);
            return;
        }
        let (position, yaw) = (agent.position, agent.yaw);
        let Some(ap) = self.autopilot.as_mut() else {
            return;
        };
        let step = ap.step(position, yaw, dt.clamp(1e-3, 0.1));
        let run = ap.run;
        self.world.agent.yaw += step.yaw;
        self.autopilot_flags = step.flags;
        if let Some(finish) = step.finish {
            if finish == Finish::NoProgress {
                log::info!("autopilot: no nearer for too long, stopped");
            }
            self.stop_autopilot(false);
        } else if self.demo {
            // no simulator offline: the local stand-in walks on the keys
            self.input.forward |= step.flags & control::AT_POS != 0;
            self.input.back |= step.flags & control::AT_NEG != 0;
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
}
