//! Face nearest avatar and the Compass, applied: the agent's heading set at
//! once, the drawn bodies snapped and an AgentUpdate forced, as
//! FSAvatarAlignBase::applyRotation does (LLAgent::resetAxes,
//! send_agent_update(true, false), snapAvatarBody, snapRemoteAvatarBody;
//! indra/newview/fsfloateravataralign.cpp, originally LGPL 2.1). The rules
//! are `crate::avatar_align`, the window `ui/avatar_align.rs`.
use super::*;
use crate::avatar_align as align;

impl App {
    /// Each frame: the Compass, while open.
    pub(super) fn avatar_align_window(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        if self.demo {
            self.demo_avatar_align();
        }
        self.panels.avatar_align_mini = self.settings.avatar_align_mini;
        let mini = self.settings.avatar_align_mini;
        let Some(action) = ui::avatar_align::show(ctx, p, &mut self.panels.avatar_align, mini, self.world.agent.yaw) else {
            return;
        };
        self.avatar_align_action(action);
    }

    fn avatar_align_action(&mut self, action: ui::avatar_align::Action) {
        use ui::avatar_align::Action;
        let yaw = self.world.agent.yaw;
        match action {
            Action::Bearing(bearing) => self.turn_agent(align::yaw_of_bearing(bearing), None),
            Action::Rotate(delta) => self.turn_agent(align::rotated(yaw, delta), None),
            Action::NearestCardinal => self.turn_agent(align::nearest_cardinal(yaw), None),
            Action::FaceNearest => self.face_nearest_avatar(),
            Action::ToggleMode => {
                self.settings.avatar_align_mini = !self.settings.avatar_align_mini;
                self.panels.avatar_align_mini = self.settings.avatar_align_mini;
                self.panels.avatar_align = true;
                self.settings.save();
            }
        }
    }

    /// onClickFaceNearestAvatar: turn to the nearest avatar within 20 m,
    /// animesh and ourselves left out; nothing but a log line without one.
    pub(super) fn face_nearest_avatar(&mut self) {
        if !self.in_world() {
            return;
        }
        let world = &self.world;
        let avatars = world
            .objects
            .avatars()
            .filter(|(_, o)| !o.full_id.is_nil() && o.full_id != world.agent_id)
            .filter_map(|(_, o)| crate::world::radar::native_position(world, o).map(|at| (o.full_id, at)));
        let me = world.agent.position;
        let Some((id, at)) = align::nearest(me, avatars) else {
            log::warn!("avatar align: no nearby avatar found to face");
            return;
        };
        let Some(yaw) = align::yaw_towards(me, at) else {
            return;
        };
        self.turn_agent(yaw, Some(id));
    }

    /// applyRotation: the agent faces `yaw` now, our drawn body with it
    /// (no turn animation), `facing`'s body snapped to its own rotation,
    /// and the simulator told at once.
    fn turn_agent(&mut self, yaw: f32, facing: Option<uuid::Uuid>) {
        let before = align::bearing(self.world.agent.yaw);
        self.world.agent.yaw = yaw;
        let agent = self.world.agent_id;
        let forward = self.world.agent.forward();
        if let Some(body) = self.world.bodies.get_mut(&agent) {
            body.snap(forward);
        }
        if let Some(id) = facing {
            let now = Instant::now();
            let heading = self
                .world
                .objects
                .index_of_uuid(&id)
                .and_then(|i| self.world.objects.get(i))
                .map(|o| self.world.objects.predicted(o, now).1 * Vec3::X);
            if let (Some(heading), Some(body)) = (heading, self.world.bodies.get_mut(&id)) {
                body.snap(heading);
            }
        }
        self.force_agent_update = true;
        log::info!(
            "avatar align: bearing {before:.1}° -> {:.1}°{}",
            align::bearing(yaw),
            if facing.is_some() { " (facing the nearest avatar)" } else { "" }
        );
    }

    /// AURORA_DEMO_AVATAR_ALIGN: the commands run one after the other, each
    /// logging the bearing before and after (`avatar align:` lines).
    fn demo_avatar_align(&mut self) {
        let Some(mode) = crate::demo::avatar_align::mode() else {
            return;
        };
        let frame = self.frame_count;
        if frame == crate::demo::avatar_align::OPEN_FRAME && crate::demo::avatar_align::opens(&mode) {
            if mode == "mini" {
                self.settings.avatar_align_mini = true;
            }
            let window = ui::avatar_align::window(self.settings.avatar_align_mini);
            ui::widgets::place_floater(
                &self.egui_ctx,
                window,
                egui::pos2(40.0, 120.0),
                if mode == "mini" {
                    egui::vec2(150.0, 210.0)
                } else {
                    egui::vec2(240.0, 420.0)
                },
            );
            ui::widgets::open_floater(&self.egui_ctx, window, &mut self.panels.avatar_align);
        }
        if let Some(step) = crate::demo::avatar_align::step(&mode, frame) {
            log::info!("demo: avatar align step {step:?} at frame {frame}");
            match step {
                crate::demo::avatar_align::Step::Toolbar(command) => {
                    let action = ui::bars::run_command(&self.egui_ctx, command, &mut self.panels);
                    if matches!(action, ui::bars::BarAction::FaceNearestAvatar) {
                        self.face_nearest_avatar();
                    }
                }
                crate::demo::avatar_align::Step::Window(action) => self.avatar_align_action(action),
                crate::demo::avatar_align::Step::Yaw(yaw) => self.world.agent.yaw = yaw,
            }
        }
    }
}
