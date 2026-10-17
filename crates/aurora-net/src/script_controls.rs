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
}
