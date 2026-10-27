//! The left button's clicks for scripts, as the Legacy Viewer sends them:
//! a script that took the button (llTakeControls, script_controls.rs in
//! aurora-net) hears it through the control bits of AgentUpdate.
//!
//! - Mouselook, the mouselook bit taken: only ML_LBUTTON_DOWN / UP
//!   (LLToolCompGun::handleMouseDown / handleMouseUp).
//! - Mouselook, not taken: the object under the crosshair is touched, and
//!   LBUTTON_DOWN / UP go out (LLToolGrabBase::handleMouseDown / Up). With
//!   Alt held the cursor is free (app/cursor_hold.rs): the HUD or the object
//!   under it is touched instead.
//! - Outside mouselook: a press on the world that nothing takes (ground, an
//!   object without action) sends LBUTTON_DOWN / UP
//!   (LLViewerInput::scanMouse, agent_control_lbutton_handle).
//!
//! Each bit is sent once, with the next AgentUpdate (`OneShotControl`);
//! Alt or Ctrl turns the presses into releases when it leaves.

use super::*;
use crate::interaction::Action;
use aurora_net::control;

/// How a mouselook press is handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Gun {
    /// A script took the mouselook button: its bits only.
    Script,
    /// Nobody did: touch at the crosshair, and the plain button's bits.
    Grab,
}

impl Gun {
    pub(super) fn of(grabbed: bool) -> Gun {
        if grabbed { Gun::Script } else { Gun::Grab }
    }

    /// The bits of the press, and of its release.
    pub(super) fn bits(self) -> (u32, u32) {
        match self {
            Gun::Script => (control::ML_LBUTTON_DOWN, control::ML_LBUTTON_UP),
            Gun::Grab => (control::LBUTTON_DOWN, control::LBUTTON_UP),
        }
    }
}

/// A mouselook click acts on what scripts or physics hear, never on the
/// one-click actions (sit, pay, buy…): those are LLToolPie's.
pub(super) fn gun_acts_on(action: Action) -> bool {
    matches!(action, Action::Touch | Action::Grab)
}

/// The release still owed for the press in progress.
#[derive(Debug, Default)]
pub(super) struct ScriptClicks {
    up: Option<u32>,
}

impl App {
    /// Left press in mouselook.
    pub(super) fn mouselook_press(&mut self) {
        let gun = Gun::of(self.world.script_controls.left_button_grabbed(true));
        self.script_click_down(gun.bits());
        if gun == Gun::Grab {
            // Alt held: the grab tool picks under the free cursor
            let at_cursor = self.alt_cursor.free();
            self.click(click_pick::Click::Gun { at_cursor });
        }
    }

    /// A press scripts hear: its bit now, its release on the button's.
    pub(super) fn script_click_down(&mut self, (down, up): (u32, u32)) {
        self.send(NetCommand::OneShotControl(down));
        self.script_clicks.up = Some(up);
    }

    /// The left button released: the release of a press scripts heard.
    pub(super) fn script_click_up(&mut self) {
        if let Some(up) = self.script_clicks.up.take() {
            self.send(NetCommand::OneShotControl(up));
        }
    }

    /// LLToolGrabBase in mouselook: touch (or grab) what is under the
    /// crosshair, or under the cursor Alt freed (a HUD first, as the pick of
    /// the Legacy Viewer); run by `click` again once the depth there is known.
    pub(super) fn gun_press(&mut self, at_cursor: bool) {
        if at_cursor {
            if !self.gun_hud_touch() {
                self.gun_touch();
            }
            return;
        }
        let saved = self.cursor_pos;
        if let Some(g) = &self.gfx {
            let (w, h) = g.renderer.size();
            self.cursor_pos = (w as f32 * 0.5, h as f32 * 0.5);
        }
        self.gun_touch();
        self.cursor_pos = saved;
    }

    /// The HUD under the free cursor takes the press, touched or grabbed like
    /// a prim of the world; true when there is one. A HUD needs no depth: it
    /// is known at the press, never when the press is run again.
    fn gun_hud_touch(&mut self) -> bool {
        if self.clicks.replaying() || !self.huds_shown() {
            return false;
        }
        let Some((idx, point, hud_ray)) = self.scene.hud_pick(&self.world, self.cursor_pos, false) else {
            return false;
        };
        self.left_down = true;
        if let Some(target) = crate::interaction::target(&self.world, idx, &self.interactions.props)
            && gun_acts_on(target.action)
        {
            self.activate_object_action(target, idx, point, Some(hud_ray));
        }
        true
    }

    fn gun_touch(&mut self) {
        let Some(picked) = self.click_pick() else {
            return;
        };
        self.left_down = true;
        let ray = picked.ray;
        let far = self.settings.draw_distance;
        let depth = self.scene.touch_depth(&self.world, &picked.drawn, ray, picked.depth, far);
        let hit = self.scene.action_point(&self.world, ray, depth, false, far);
        let shown = self.designate(crate::scene::drawn::Gesture::Touch, &picked, hit);
        if let Some(d) = shown {
            if let Some(target) = crate::interaction::target(&self.world, d.idx, &self.interactions.props)
                && gun_acts_on(target.action)
            {
                let surface = self.scene.drawn_surface(&self.world, &d);
                self.activate_object_action_at(target, d.idx, d.hit.point, ray, surface);
            }
            return;
        }
        if let (Some(point), Some(pick_ray)) = (hit, ray)
            && let Some(idx) = self.scene.interaction_at_ray(&self.world, point, pick_ray, true)
            && let Some(target) = crate::interaction::target(&self.world, idx, &self.interactions.props)
            && gun_acts_on(target.action)
        {
            self.activate_object_action(target, idx, point, ray);
        }
    }

    /// Whether `idx` is a prim worn by our avatar (or a child of one).
    pub(super) fn own_attachment(&self, idx: usize) -> bool {
        let mut current = idx;
        for _ in 0..64 {
            let Some(o) = self.world.objects.get(current) else {
                return false;
            };
            let Some(parent) = self.world.objects.parent_of(o) else {
                return false;
            };
            match self.world.objects.get(parent) {
                Some(p) if p.is_avatar() => return p.full_id == self.world.agent_id,
                Some(_) => current = parent,
                None => return false,
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_taken_mouselook_button_sends_only_its_own_bits() {
        assert_eq!(Gun::of(true), Gun::Script);
        assert_eq!(Gun::Script.bits(), (control::ML_LBUTTON_DOWN, control::ML_LBUTTON_UP));
        assert_eq!(Gun::of(false), Gun::Grab);
        assert_eq!(Gun::Grab.bits(), (control::LBUTTON_DOWN, control::LBUTTON_UP));
    }

    #[test]
    fn a_mouselook_click_touches_or_grabs_but_runs_no_click_action() {
        assert!(gun_acts_on(Action::Touch));
        assert!(gun_acts_on(Action::Grab));
        for a in [
            Action::Sit,
            Action::Pay,
            Action::Buy,
            Action::Open,
            Action::Play,
            Action::Zoom,
            Action::Disabled,
        ] {
            assert!(!gun_acts_on(a), "{a:?}");
        }
    }
}
