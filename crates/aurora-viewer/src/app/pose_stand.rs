//! The toolbar's pose stand: its window opened and closed in any way (the
//! button, the cross, Ctrl+W) runs FSFloaterPoseStand's onOpen / onClose
//! (rules in `pose_stand.rs`), and its effects go to the simulator here.
use super::*;
use crate::pose_stand::{Effect, Facts};

impl App {
    /// Each frame, with the other windows.
    pub(super) fn pose_stand_window(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        if self.demo {
            self.demo_pose_stand();
        }
        self.sync_pose_stand();
        let action = ui::pose_stand::show(
            ctx,
            p,
            &mut self.panels.pose_stand,
            self.pose_stand.current(),
            self.settings.pose_stand.lock,
        );
        match action {
            Some(ui::pose_stand::Action::Pose(id)) => self.choose_pose(id),
            Some(ui::pose_stand::Action::Lock(on)) => {
                self.settings.pose_stand.lock = on;
                let effects = self.pose_stand.lock_changed(on);
                self.apply_pose_stand(effects);
            }
            None => {}
        }
        self.sync_pose_stand();
        // stood up from outside (the appearance editor, a script, a teleport):
        // the checkbox follows the avatar, as the saved setting does
        if self.pose_stand.lock_lost(self.world.agent.is_sitting()) {
            log::info!("pose stand: the ground lock was lost, lock turned off");
            self.settings.pose_stand.lock = false;
        }
        let effects = self
            .pose_stand
            .late_release(self.world.agent.is_sitting(), std::time::Instant::now());
        self.apply_pose_stand(effects);
    }

    /// onCommitCombo: the pose kept for the next opening, then played.
    fn choose_pose(&mut self, id: uuid::Uuid) {
        self.settings.pose_stand.last_pose = id.to_string();
        let effects = self.pose_stand.select(id);
        self.apply_pose_stand(effects);
    }

    /// The window opened or closed since the stand last looked.
    fn sync_pose_stand(&mut self) {
        let open = self.panels.pose_stand && self.in_world();
        if open && !self.pose_stand.open {
            let facts = Facts {
                ao_on: self.ao_enabled(),
                sitting: self.world.agent.is_sitting(),
                rlv_sit: self.world.rlva.has("sit"),
            };
            let effects = self.pose_stand.open(facts, &self.settings.pose_stand);
            log::info!("pose stand: opened");
            self.apply_pose_stand(effects);
        } else if !open && self.pose_stand.open {
            self.close_pose_stand();
            self.panels.pose_stand = false;
        }
    }

    /// onClose, also when the viewer quits with the stand open (its
    /// app_quitting closing).
    pub(super) fn close_pose_stand(&mut self) {
        if !self.pose_stand.open {
            return;
        }
        let effects = self.pose_stand.close(self.world.agent.is_sitting(), self.ao_enabled());
        log::info!("pose stand: closed");
        self.apply_pose_stand(effects);
    }

    /// UseAO for our account.
    fn ao_enabled(&self) -> bool {
        self.settings.ao.accounts.get(&self.world.agent_id).is_some_and(|a| a.enabled)
    }

    fn apply_pose_stand(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::PauseAo | Effect::ResumeAo => {
                    if let Some(account) = self.settings.ao.accounts.get_mut(&self.world.agent_id) {
                        account.enabled = effect == Effect::ResumeAo;
                    }
                }
                // RLVa's canGroundSit / canStand are checked by the send
                Effect::SitOnGround => self.send(NetCommand::OneShotControl(control::SIT_ON_GROUND)),
                Effect::StandUp => self.send(NetCommand::OneShotControl(control::STAND_UP)),
                Effect::StopAnimations => self.stop_animations(false),
                Effect::Start(anim) | Effect::Stop(anim) => {
                    let start = matches!(effect, Effect::Start(_));
                    log::info!("pose stand: {} {anim}", if start { "start" } else { "stop" });
                    self.send(NetCommand::AgentAnimation { anim, start });
                }
            }
        }
    }

    /// AURORA_DEMO_POSE_STAND (`demo_pose_stand.rs`).
    fn demo_pose_stand(&mut self) {
        use crate::demo::pose_stand::{self as demo, Step};
        let Some(mode) = demo::mode() else { return };
        match demo::step(mode, self.frame_count) {
            Some(Step::Open) => ui::widgets::open_floater(&self.egui_ctx, ui::pose_stand::WINDOW, &mut self.panels.pose_stand),
            Some(Step::Pose(id)) => {
                if self.scene.anims.peek(&id).is_none() {
                    log::info!("pose stand demo: {id} has no stand-in here: the avatar keeps its stand");
                }
                self.choose_pose(id);
            }
            Some(Step::Close) => ui::widgets::close_floater(&self.egui_ctx, ui::pose_stand::WINDOW),
            None => {}
        }
    }
}
