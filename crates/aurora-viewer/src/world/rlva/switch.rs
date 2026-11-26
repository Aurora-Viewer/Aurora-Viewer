//! RLVa's global switch: whether RLVa runs in this start of the viewer,
//! apart from the saved preference. Port of RlvHandler::setEnabled and
//! canEnable (indra/newview/rlvhandler.cpp), its call at
//! STATE_LOGIN_CLEANUP (LLStartUp::idle_startup, indra/newview/llstartup.cpp)
//! and RlvSettings::onChangedSettingMain (indra/newview/rlvcommon.cpp),
//! originally LGPL 2.1, Kitty Barnett.
//!
//! RLVa starts when a login starts with the preference on. Until then, on
//! the login screen, the preference applies at once both ways (the Legacy
//! quick toggle). Once RLVa runs it never stops before the viewer quits:
//! turning the preference off only applies at the next start, so the
//! restrictions an object holds stay for the whole session (issue #1866).
//! Turning it on during a session waits for the next login too.

/// What a change of the preference does (GenericAlert with
/// RLVaToggleMessageLogin or RLVaToggleMessageRestart).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    /// "RLVa has been enabled (no restart required)".
    EnabledNow,
    /// "RLVa has been disabled (no restart required)".
    DisabledNow,
    /// "RLVa will be enabled after you restart".
    EnabledAfterRestart,
    /// "RLVa will be disabled after you restart".
    DisabledAfterRestart,
}

impl Notice {
    /// The alert's text. The instant changes happen on the login screen
    /// only, which shows no alerts: the Preferences page shows the switch.
    pub fn text(self) -> Option<&'static str> {
        match self {
            Self::EnabledNow | Self::DisabledNow => None,
            Self::EnabledAfterRestart => Some(aurora_i18n::tr!("rlva-toggle-enabled-restart")),
            Self::DisabledAfterRestart => Some(aurora_i18n::tr!("rlva-toggle-disabled-restart")),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Switch {
    running: bool,
    /// The preference as last seen, to show the alert once per change.
    seen: bool,
    /// `can_enable` of the last frame: its end is a login starting.
    could_enable: bool,
}

impl Switch {
    /// The start of the viewer, on the login screen: RLVa does not run yet.
    pub fn new(saved: bool) -> Self {
        Self {
            running: false,
            seen: saved,
            could_enable: true,
        }
    }

    /// RlvHandler::isEnabled.
    pub fn running(&self) -> bool {
        self.running
    }

    /// A login starts (STATE_LOGIN_CLEANUP): RLVa starts if the preference
    /// is on, and runs until the viewer quits.
    pub fn login_starts(&mut self, saved: bool) {
        self.running |= saved;
    }

    /// The preference, every frame. `can_enable` is RlvHandler::canEnable:
    /// the login screen, before a login starts. Leaving it starts RLVa (a
    /// login, or the demo's world); a change shows its alert once, like the
    /// setting's signal.
    pub fn follow(&mut self, saved: bool, can_enable: bool) -> Option<Notice> {
        if self.could_enable && !can_enable {
            self.login_starts(saved);
        }
        self.could_enable = can_enable;
        if saved == self.seen {
            return None;
        }
        self.seen = saved;
        // onChangedSettingMain's quick toggle: RLVa not running yet and
        // allowed to start; setEnabled(false) never stops it
        Some(match (saved, !self.running && can_enable) {
            (true, true) => Notice::EnabledNow,
            (false, true) => Notice::DisabledNow,
            (true, false) => Notice::EnabledAfterRestart,
            (false, false) => Notice::DisabledAfterRestart,
        })
    }

    /// The preference differs from what will run: on the login screen
    /// before RLVa runs, nothing waits; otherwise a change waits for the
    /// next start.
    pub fn waiting(&self, saved: bool, can_enable: bool) -> bool {
        saved != self.running && !(can_enable && !self.running)
    }

    /// A demo scenario starts or stops RLVa in the session, with the
    /// preference set to match: it stands for the next start of the viewer.
    pub fn demo_set(&mut self, on: bool) {
        self.running = on;
        self.seen = on;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rlva_starts_when_a_login_starts_with_the_preference_on() {
        let mut on = Switch::new(true);
        assert!(!on.running(), "the login screen: RLVa has not started");
        assert_eq!(on.follow(true, true), None);
        assert!(!on.running());
        // the login button pressed
        assert_eq!(on.follow(true, false), None);
        assert!(on.running());

        let mut off = Switch::new(false);
        assert_eq!(off.follow(false, false), None);
        assert!(!off.running());
    }

    #[test]
    fn the_login_screen_follows_the_preference_both_ways() {
        let mut switch = Switch::new(true);
        assert_eq!(switch.follow(false, true), Some(Notice::DisabledNow));
        assert!(!switch.waiting(false, true));
        assert_eq!(switch.follow(true, true), Some(Notice::EnabledNow));
        assert_eq!(switch.follow(false, true), Some(Notice::DisabledNow));
        // the login runs without RLVa
        switch.follow(false, false);
        assert!(!switch.running());
    }

    #[test]
    fn turning_rlva_off_waits_for_the_next_start() {
        let mut switch = Switch::new(true);
        switch.follow(true, false);
        assert!(switch.running());
        assert_eq!(switch.follow(false, false), Some(Notice::DisabledAfterRestart));
        assert!(switch.running());
        assert!(switch.waiting(false, false));
        // once per change
        assert_eq!(switch.follow(false, false), None);
        // back on the login screen after a logout: still running
        assert_eq!(switch.follow(false, true), None);
        assert!(switch.running());
        assert!(switch.waiting(false, true));
        // turned back on: still running, nothing waits
        assert_eq!(switch.follow(true, true), Some(Notice::EnabledAfterRestart));
        assert!(switch.running());
        assert!(!switch.waiting(true, false));
    }

    #[test]
    fn turning_rlva_on_in_a_session_waits_for_the_next_login() {
        let mut switch = Switch::new(false);
        switch.follow(false, false);
        assert_eq!(switch.follow(true, false), Some(Notice::EnabledAfterRestart));
        assert!(!switch.running());
        assert!(switch.waiting(true, false));
        // logged out, then in again: the new login starts it
        switch.follow(true, true);
        assert!(!switch.waiting(true, true));
        switch.follow(true, false);
        assert!(switch.running());
    }

    #[test]
    fn a_demo_sets_the_state_without_an_alert() {
        let mut switch = Switch::new(false);
        switch.follow(false, false);
        switch.demo_set(true);
        assert!(switch.running());
        assert_eq!(switch.follow(true, false), None);
        switch.demo_set(false);
        assert!(!switch.running());
        assert_eq!(switch.follow(false, false), None);
    }
}
