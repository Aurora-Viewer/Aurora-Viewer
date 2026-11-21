//! RLVa's start and the login: the settings read once and the commands
//! retained until the login completes. Port of RlvSettings::initClass
//! (indra/newview/rlvcommon.cpp), RlvBehaviourDictionary's constructor
//! (rlvhelper.cpp) and RlvHandler::processCommand, onIdleStartup,
//! onLoginComplete and processRetainedCommands (rlvhandler.cpp), originally
//! LGPL 2.1, Kitty Barnett.
use super::{Command, MAX_QUEUED, Param, Rlva, preferences::Preferences};
use crate::world::World;
use aurora_net::ChatMessage;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

/// Where the session stands, as LLStartUp's state does for RLVa.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// The login screen: RLVa has not started, its settings apply as set
    /// (RlvSettings::onChangedSettingMain's "quick toggle").
    Login,
    /// Logging in (RlvHandler::setEnabled ran at STATE_LOGIN_CLEANUP, the
    /// state is before STATE_STARTED): commands are retained.
    Loading,
    /// STATE_STARTED.
    Started,
}

#[derive(Default)]
pub struct Startup {
    /// The settings as RLVa started with them.
    settings: Option<Box<Preferences>>,
    logging_in: bool,
    /// One command each, "@" and its text, in the order received.
    retained: VecDeque<ChatMessage>,
    tick: Option<Instant>,
    /// The retained commands run again: no RestrainedLoveDebug line, as
    /// processRetainedCommands never passes through
    /// process_chat_from_simulator.
    pub(super) replaying: bool,
}

/// onIdleStartup's period for the @version answers during the login.
const VERSION_PERIOD: Duration = Duration::from_secs(2);

impl Rlva {
    /// The application settings, every frame. RlvSettings::initClass reads
    /// RestrainedLoveNoSetEnv, RestrainedLoveCanOOC,
    /// RLVaExperienceMaturityThreshold and RLVaBlockedExperiences once, as
    /// RlvBehaviourDictionary's constructor does RLVaExperimentalCommands and
    /// initCompatibilityMode RLVaCompatibilityModeList: a change applies at
    /// the next start of RLVa (the next login here, or RLVa turned on again).
    /// The others follow the settings live.
    pub fn apply_preferences(&mut self, live: &Preferences, phase: Phase) {
        self.preferences.clone_from(live);
        if phase == Phase::Login || !self.enabled {
            self.startup.settings = None;
            return;
        }
        let start = self.startup.settings.get_or_insert_with(|| Box::new(live.clone()));
        self.preferences.allow_environment_locks = start.allow_environment_locks;
        self.preferences.allow_ooc = start.allow_ooc;
        self.preferences.experimental_commands = start.experimental_commands;
        self.preferences.experience_maturity = start.experience_maturity;
        self.preferences.blocked_experiences.clone_from(&start.blocked_experiences);
        self.preferences.compatibility_list.clone_from(&start.compatibility_list);
    }

    /// RlvHandler::processCommand(idObj, strCommand): before STATE_STARTED
    /// a command is kept (RLV_RET_RETAINED) instead of run.
    pub(super) fn retains(&self) -> bool {
        self.startup.logging_in && !self.startup.replaying
    }

    pub(super) fn retain(&mut self, chat: &ChatMessage, text: &str) {
        if self.startup.retained.len() < MAX_QUEUED {
            self.startup.retained.push_back(ChatMessage {
                message: format!("@{text}"),
                ..chat.clone()
            });
        }
    }
}

/// processRetainedCommands(RLV_BHVR_VERSION…, RLV_TYPE_REPLY): the commands
/// onIdleStartup answers during the login.
fn version_reply(chat: &ChatMessage) -> bool {
    Command::parse(&chat.message[1..])
        .is_some_and(|c| matches!(c.param, Param::Reply(_)) && matches!(c.behaviour.as_str(), "version" | "versionnew" | "versionnum"))
}

impl World {
    /// Every frame: the read-once settings, then the retained commands.
    /// During the login, onIdleStartup answers the retained @version,
    /// @versionnew and @versionnum every 2 seconds; once it completes,
    /// onLoginComplete runs every other one, in order.
    pub fn rlva_startup(&mut self, phase: Phase, live: &Preferences, now: Instant) {
        self.rlva.apply_preferences(live, phase);
        let logging_in = phase == Phase::Loading && self.rlva.enabled();
        self.rlva.startup.logging_in = logging_in;
        if logging_in {
            let since = *self.rlva.startup.tick.get_or_insert(now);
            if now.saturating_duration_since(since) < VERSION_PERIOD {
                return;
            }
            self.rlva.startup.tick = Some(now);
            let (versions, others) = std::mem::take(&mut self.rlva.startup.retained).into_iter().partition(version_reply);
            self.rlva.startup.retained = others;
            self.rlva_replay(versions);
        } else {
            self.rlva.startup.tick = None;
            let retained = std::mem::take(&mut self.rlva.startup.retained);
            self.rlva_replay(retained);
        }
    }

    fn rlva_replay(&mut self, retained: VecDeque<ChatMessage>) {
        if retained.is_empty() || !self.rlva.enabled() {
            return;
        }
        self.rlva.startup.replaying = true;
        for chat in retained {
            self.on_chat(chat);
        }
        self.rlva.startup.replaying = false;
    }
}
