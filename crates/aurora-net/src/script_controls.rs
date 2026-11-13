//! Controls taken by scripts (llTakeControls): port of
//! LLAgent::processScriptControlChange and leftButtonGrabbed
//! (indra/newview/llagent.cpp, originally LGPL 2.1).
//!
//! The simulator says, in ScriptControlChange, which control bits a script
//! takes or releases, and whether it passes them on to the avatar. The
//! viewer keeps one count per bit and per kind: several scripts can take
//! the same control, and a release only lowers the count, never below
//! zero. The counts last as long as the session (the Legacy Viewer never
//! clears them either).
//!
//! Two kinds of question are asked of the counts, as in the Legacy Viewer:
//! `any` and `left_button_grabbed` (anyControlGrabbed, leftButtonGrabbed)
//! count both kinds, while `is_grabbed`, `forward_grabbed`, `backward_grabbed`,
//! `rotate_grabbed`, `up_grabbed` and `down_grabbed` (isControlGrabbed and the
//! others) count only the controls taken outright: a script that passes its
//! controls on to the avatar does not grab them.

use crate::control;

/// The 32 control bits (TOTAL_CONTROLS).
const BITS: usize = 32;

/// How many scripts hold each control bit, taken outright or passed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScriptControls {
    taken: [u16; BITS],
    passed_on: [u16; BITS],
}

impl Default for ScriptControls {
    fn default() -> Self {
        ScriptControls {
            taken: [0; BITS],
            passed_on: [0; BITS],
        }
    }
}

impl ScriptControls {
    /// One block of ScriptControlChange: `take` adds a holder to every bit
    /// of `controls`, a release removes one.
    pub fn apply(&mut self, take: bool, controls: u32, pass_on: bool) {
        let counts = if pass_on { &mut self.passed_on } else { &mut self.taken };
        for (bit, count) in counts.iter_mut().enumerate() {
            if controls & (1 << bit) == 0 {
                continue;
            }
            *count = if take { count.saturating_add(1) } else { count.saturating_sub(1) };
        }
    }

    fn held(&self, flag: u32) -> bool {
        let bit = flag.trailing_zeros() as usize;
        bit < BITS && (self.taken[bit] > 0 || self.passed_on[bit] > 0)
    }

    /// A control bit taken outright (`pass_on` false), ignoring pass-on holds.
    fn taken_only(&self, flag: u32) -> bool {
        let bit = flag.trailing_zeros() as usize;
        bit < BITS && self.taken[bit] > 0
    }

    /// Whether a script wants the left button (leftButtonGrabbed): its
    /// mouselook bit in mouselook, the plain one otherwise.
    pub fn left_button_grabbed(&self, mouselook: bool) -> bool {
        self.held(if mouselook {
            control::ML_LBUTTON_DOWN
        } else {
            control::LBUTTON_DOWN
        })
    }

    /// Whether any script holds a control (anyControlGrabbed).
    pub fn any(&self) -> bool {
        self.taken.iter().chain(self.passed_on.iter()).any(|&n| n > 0)
    }

    /// Whether a script took a specific control bit without passing it on
    /// (isControlGrabbed reads only mControlsTakenCount: a pass-on hold does
    /// not count, unlike in `any` and `left_button_grabbed`).
    pub fn is_grabbed(&self, flag: u32) -> bool {
        self.taken_only(flag)
    }

    /// Whether a script took the forward control without passing it on
    /// (forwardGrabbed, mControlsTakenCount only).
    pub fn forward_grabbed(&self) -> bool {
        self.taken_only(control::AT_POS)
    }

    /// Whether a script took the backward control without passing it on
    /// (backwardGrabbed, mControlsTakenCount only).
    pub fn backward_grabbed(&self) -> bool {
        self.taken_only(control::AT_NEG)
    }

    /// Whether a script took yaw left or right without passing it on
    /// (rotateGrabbed, mControlsTakenCount only).
    pub fn rotate_grabbed(&self) -> bool {
        self.taken_only(control::YAW_POS) || self.taken_only(control::YAW_NEG)
    }

    /// Whether a script took CONTROL_UP without passing it on (upGrabbed,
    /// mControlsTakenCount only).
    pub fn up_grabbed(&self) -> bool {
        self.taken_only(control::UP_POS)
    }

    /// Whether a script took the down/crouch control without passing it on
    /// (downGrabbed, mControlsTakenCount only).
    pub fn down_grabbed(&self) -> bool {
        self.taken_only(control::UP_NEG)
    }
}

/// The left-button bits as `send_agent_update` sends them: while Alt or
/// Ctrl is held a press reads as a release, so that moving the camera with
/// Alt never fires a script.
pub fn mask_left_button(flags: u32, alt_or_ctrl: bool) -> u32 {
    if !alt_or_ctrl {
        return flags;
    }
    (flags & !(control::LBUTTON_DOWN | control::ML_LBUTTON_DOWN)) | control::LBUTTON_UP | control::ML_LBUTTON_UP
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_follow_takes_and_releases_per_kind() {
        let mut c = ScriptControls::default();
        assert!(!c.left_button_grabbed(true));
        // a weapon takes the mouselook button, passed on to the avatar
        c.apply(true, control::ML_LBUTTON_DOWN | control::ML_LBUTTON_UP, true);
        assert!(c.left_button_grabbed(true));
        assert!(!c.left_button_grabbed(false));
        // a second script takes it outright
        c.apply(true, control::ML_LBUTTON_DOWN, false);
        // the first one releases: the second still holds it
        c.apply(false, control::ML_LBUTTON_DOWN | control::ML_LBUTTON_UP, true);
        assert!(c.left_button_grabbed(true));
        c.apply(false, control::ML_LBUTTON_DOWN, false);
        assert!(!c.left_button_grabbed(true));
        assert!(!c.any());
    }

    #[test]
    fn a_release_never_goes_below_zero() {
        let mut c = ScriptControls::default();
        c.apply(false, control::LBUTTON_DOWN, false);
        c.apply(false, control::LBUTTON_DOWN, false);
        c.apply(true, control::LBUTTON_DOWN, false);
        assert!(c.left_button_grabbed(false));
        assert!(!c.left_button_grabbed(true));
    }

    #[test]
    fn the_button_bits_are_the_legacy_viewers() {
        assert_eq!(control::LBUTTON_DOWN, 0x1000_0000);
        assert_eq!(control::LBUTTON_UP, 0x2000_0000);
        assert_eq!(control::ML_LBUTTON_DOWN, 0x4000_0000);
        assert_eq!(control::ML_LBUTTON_UP, 0x8000_0000);
    }

    #[test]
    fn alt_or_ctrl_turns_presses_into_releases() {
        let pressed = control::AT_POS | control::ML_LBUTTON_DOWN;
        assert_eq!(mask_left_button(pressed, false), pressed);
        assert_eq!(
            mask_left_button(pressed, true),
            control::AT_POS | control::LBUTTON_UP | control::ML_LBUTTON_UP
        );
        // held without a click: the release bits go out all the same
        assert_eq!(
            mask_left_button(control::AT_POS, true),
            control::AT_POS | control::LBUTTON_UP | control::ML_LBUTTON_UP
        );
    }

    /// Every movement query with the control it reads: the Legacy Viewer's
    /// forwardGrabbed & co read mControlsTakenCount alone.
    fn movement_queries() -> [(&'static str, u32, fn(&ScriptControls) -> bool); 6] {
        [
            ("forward", control::AT_POS, ScriptControls::forward_grabbed),
            ("backward", control::AT_NEG, ScriptControls::backward_grabbed),
            ("rotate (yaw left)", control::YAW_POS, ScriptControls::rotate_grabbed),
            ("rotate (yaw right)", control::YAW_NEG, ScriptControls::rotate_grabbed),
            ("up", control::UP_POS, ScriptControls::up_grabbed),
            ("down", control::UP_NEG, ScriptControls::down_grabbed),
        ]
    }

    #[test]
    fn movement_controls_grabbed_detection() {
        let mut c = ScriptControls::default();
        for (name, flag, grabbed) in movement_queries() {
            assert!(!grabbed(&c), "{name}: nothing taken");
            assert!(!c.is_grabbed(flag), "{name}: nothing taken (is_grabbed)");
        }

        // A vehicle script takes forward and yaw controls outright
        c.apply(true, control::AT_POS | control::YAW_POS, false);
        assert!(c.forward_grabbed());
        assert!(c.rotate_grabbed());
        assert!(!c.backward_grabbed());
        assert!(!c.up_grabbed());
        assert!(!c.down_grabbed());

        // Release forward: yaw stays grabbed
        c.apply(false, control::AT_POS, false);
        assert!(!c.forward_grabbed());
        assert!(c.rotate_grabbed());
    }

    #[test]
    fn a_pass_on_hold_does_not_grab_a_movement_control() {
        for (name, flag, grabbed) in movement_queries() {
            let mut c = ScriptControls::default();

            // taken with pass_on TRUE (a HUD that only listens): not grabbed,
            // so the camera keys keep their meaning
            c.apply(true, flag, true);
            assert!(!grabbed(&c), "{name}: passed on only");
            assert!(!c.is_grabbed(flag), "{name}: passed on only (is_grabbed)");

            // the two kinds are still held for the questions that count both
            assert!(c.any(), "{name}: a pass-on hold still counts for any");

            // taken outright as well: grabbed
            c.apply(true, flag, false);
            assert!(grabbed(&c), "{name}: taken outright");
            assert!(c.is_grabbed(flag), "{name}: taken outright (is_grabbed)");

            // released outright: the pass-on hold alone does not grab it
            c.apply(false, flag, false);
            assert!(!grabbed(&c), "{name}: outright hold released, pass-on hold left");
            assert!(!c.is_grabbed(flag), "{name}: outright hold released (is_grabbed)");

            c.apply(false, flag, true);
            assert!(!grabbed(&c), "{name}: everything released");
            assert!(!c.any(), "{name}: everything released (any)");
        }
    }

    #[test]
    fn a_pass_on_hold_still_counts_for_the_left_button_and_any() {
        // leftButtonGrabbed and anyControlGrabbed count both kinds
        let mut c = ScriptControls::default();
        c.apply(true, control::LBUTTON_DOWN, true);
        assert!(c.left_button_grabbed(false));
        assert!(c.any());
        // ... while the movement queries and isControlGrabbed do not
        assert!(!c.is_grabbed(control::LBUTTON_DOWN));
    }

    #[test]
    fn one_grabbed_movement_control_leaves_the_others_free() {
        for (name, flag, _) in movement_queries() {
            let mut c = ScriptControls::default();
            c.apply(true, flag, false);
            for (other, other_flag, grabbed) in movement_queries() {
                let same_query = other_flag == flag || (name.starts_with("rotate") && other.starts_with("rotate"));
                assert_eq!(grabbed(&c), same_query, "{name} taken, {other} asked");
            }
        }
    }
}
