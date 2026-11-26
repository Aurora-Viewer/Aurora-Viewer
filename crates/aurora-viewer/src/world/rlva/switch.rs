//! RLVa's global switch: whether RLVa runs in this start of the viewer,
//! apart from the saved preference. Port of RlvHandler::setEnabled and
//! canEnable (indra/newview/rlvhandler.cpp) and
//! RlvSettings::onChangedSettingMain (indra/newview/rlvcommon.cpp),
//! originally LGPL 2.1, Kitty Barnett.
//!
//! RLVa starts with the viewer when the preference is on. Once it runs, it
//! never stops before the viewer quits: turning the preference off only
//! applies at the next start, so the restrictions an object holds stay for
//! the whole session (issue #1866). Turning it on is instant on the login
//! screen, before a login starts; later it too waits for the next start.

/// The alert shown when the preference changes (GenericAlert with
/// RLVaToggleMessageLogin or RLVaToggleMessageRestart).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    /// "RLVa has been enabled (no restart required)".
    EnabledNow,
    /// "RLVa will be enabled after you restart".
    EnabledAfterRestart,
    /// "RLVa will be disabled after you restart".
    DisabledAfterRestart,
}

impl Notice {
    /// The alert's text. The instant change happens on the login screen
    /// only, which shows no alerts: the Preferences page shows the switch on.
    pub fn text(self) -> Option<&'static str> {
        match self {
            Self::EnabledNow => None,
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
}

impl Switch {
    /// The start of the viewer: RLVa runs if the saved preference is on
    /// (LLAppViewer::init calls RlvHandler::setEnabled with RestrainedLove).
    pub fn new(saved: bool) -> Self {
        Self {
            running: saved,
            seen: saved,
        }
    }

    /// RlvHandler::isEnabled.
    pub fn running(&self) -> bool {
        self.running
    }

    /// The preference, every frame. `can_enable` is RlvHandler::canEnable:
    /// the login screen, before a login starts (STATE_LOGIN_CLEANUP or
    /// earlier). A change shows its alert once, like the setting's signal.
    pub fn follow(&mut self, saved: bool, can_enable: bool) -> Option<Notice> {
        if saved == self.seen {
            return None;
        }
        self.seen = saved;
        // onChangedSettingMain's quick toggle: RLVa not running yet and
        // allowed to start; setEnabled(false) never stops it
        if saved && !self.running && can_enable {
            self.running = true;
            return Some(Notice::EnabledNow);
        }
        Some(if saved {
            Notice::EnabledAfterRestart
        } else {
            Notice::DisabledAfterRestart
        })
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
    fn rlva_starts_with_the_saved_preference() {
        assert!(Switch::new(true).running());
        assert!(!Switch::new(false).running());
    }

    #[test]
    fn turning_rlva_off_waits_for_the_next_start() {
        let mut switch = Switch::new(true);
        // on the login screen as in the world: setEnabled(false) is a no-op
        for can_enable in [true, false] {
            let mut s = switch;
            assert_eq!(s.follow(false, can_enable), Some(Notice::DisabledAfterRestart));
            assert!(s.running());
            // once per change
            assert_eq!(s.follow(false, can_enable), None);
        }
        // turned back on: still running
        assert_eq!(switch.follow(false, false), Some(Notice::DisabledAfterRestart));
        assert_eq!(switch.follow(true, false), Some(Notice::EnabledAfterRestart));
        assert!(switch.running());
    }

    #[test]
    fn turning_rlva_on_is_instant_only_before_a_login() {
        let mut login = Switch::new(false);
        assert_eq!(login.follow(true, true), Some(Notice::EnabledNow));
        assert!(login.running());
        // started, it cannot be stopped again before the next start
        assert_eq!(login.follow(false, true), Some(Notice::DisabledAfterRestart));
        assert!(login.running());

        let mut world = Switch::new(false);
        assert_eq!(world.follow(true, false), Some(Notice::EnabledAfterRestart));
        assert!(!world.running());
    }

    #[test]
    fn an_unchanged_preference_shows_nothing() {
        let mut switch = Switch::new(false);
        assert_eq!(switch.follow(false, true), None);
        assert_eq!(switch.follow(false, false), None);
        assert!(!switch.running());
    }

    #[test]
    fn a_demo_sets_the_state_without_an_alert() {
        let mut switch = Switch::new(false);
        switch.demo_set(true);
        assert!(switch.running());
        assert_eq!(switch.follow(true, false), None);
        switch.demo_set(false);
        assert!(!switch.running());
        assert_eq!(switch.follow(false, false), None);
    }
}
