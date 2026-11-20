//! A teleport refused for the destination's content rating, answered as the
//! Legacy Viewer answers it (indra/newview/llviewermessage.cpp, originally
//! LGPL 2.1):
//!
//! - handle_teleport_access_blocked picks the prompt from the account's
//!   ceiling and the preference: content for adults only, "Change and
//!   continue" when the refused request can be sent again
//!   (LLAgent::hasRestartableFailedTeleportRequest), "Change preferences"
//!   when it cannot (a lure: the server has already paired and dropped it,
//!   LLTeleportRequestViaLure::canRestartTeleport), or "preferences out of
//!   sync"; for `RegionTPAccessBlocked`, the maturity dialog
//!   (LLFloaterMaturityDialog) when the account may raise its preference.
//! - The answers: handle_prompt_for_maturity_level_change_callback (the
//!   preference only), handle_prompt_for_maturity_level_change_and_reteleport_callback
//!   (the preference, then LLAgent::restartFailedTeleportRequest, and
//!   PreferredMaturityChanged once that teleport ends) and
//!   LLFloaterMaturityDialog::onContinue (the preference and the restart).
//! - The restarted request waits for the server to keep the new preference
//!   (startTeleportRequest's TELEPORT_PENDING, `maturity.rs`).
//!
//! The Legacy Viewer falls back to the `_Notify` tips when a prompt is
//! ignored; Aurora's notifications cannot be ignored, so the prompt is
//! always shown.

use super::{World, notifications};
use aurora_net::NetCommand;
use aurora_net::search::{AgentAccess, SIM_ACCESS_ADULT, SIM_ACCESS_MATURE};
use aurora_net::teleport_access::{AccessBlocked, ENTRY_ACCESS_BLOCKED, REGION_TP_ACCESS_BLOCKED};
use uuid::Uuid;

/// The Legacy Viewer's page on content ratings
/// (TeleportEntryAccessBlocked_AdultsOnlyContent).
const RATINGS_URL: &str = "http://wiki.secondlife.com/wiki/Linden_Lab_Official:Maturity_ratings:_an_overview";

/// What the viewer offers for a refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prompt {
    /// `_AdultsOnlyContent`: the account cannot enter.
    AdultsOnly,
    /// `_ChangeAndReTeleport`: change the preference and teleport again.
    ChangeAndReteleport,
    /// `_Change`: change the preference, the teleport is to ask again.
    Change,
    /// `_PreferencesOutOfSync`: the preference already allows the rating.
    OutOfSync,
    /// RegionTPAccessBlocked: the maturity dialog ("Allow … content").
    Allow,
    /// RegionTPAccessBlocked_NotifyAdultsOnly: not on this account type.
    NotOnAccount,
}

/// A teleport request as it was sent, for a restart
/// (LLAgent::mTeleportRequest).
#[derive(Debug, Clone)]
struct Sent {
    actor: Uuid,
    command: NetCommand,
    /// The teleport screen's destination and whether it showed.
    dest: Option<String>,
    show_progress: bool,
}

impl Sent {
    /// canRestartTeleport: every request but a lure's.
    fn restartable(&self) -> bool {
        !matches!(self.command, NetCommand::AcceptLure { .. })
    }
}

/// The teleport screen a restart opens again (teleportCore).
#[derive(Debug, Clone, PartialEq)]
pub struct Restart {
    pub dest: Option<String>,
    pub show_progress: bool,
}

#[derive(Debug, Default)]
pub struct TeleportAccess {
    /// The last teleport request sent, until it arrives.
    last: Option<Sent>,
    /// The refusal the session announced, for the TeleportFailed that
    /// follows it.
    pending: Option<AccessBlocked>,
    /// The refused request, while its prompt waits for an answer.
    failed: Option<Sent>,
    /// The rating set by "Change and continue"
    /// (setMaturityRatingChangeDuringTeleport): PreferredMaturityChanged
    /// once that teleport ends.
    changed_during_teleport: Option<u8>,
    /// A restart the app opens the teleport screen for.
    restart: Option<Restart>,
}

/// handle_teleport_access_blocked: the prompt for `blocked`, None when the
/// refusal gets the ordinary failure.
pub fn prompt(blocked: &AccessBlocked, access: AgentAccess, restartable: bool) -> Option<Prompt> {
    let rating = blocked.region_access;
    let change = if restartable { Prompt::ChangeAndReteleport } else { Prompt::Change };
    match blocked.code.as_str() {
        ENTRY_ACCESS_BLOCKED => match rating {
            // isTeen, prefersPG
            SIM_ACCESS_MATURE if access.max < SIM_ACCESS_MATURE => Some(Prompt::AdultsOnly),
            SIM_ACCESS_MATURE if access.preferred < SIM_ACCESS_MATURE => Some(change),
            SIM_ACCESS_MATURE => Some(Prompt::OutOfSync),
            // !isAdult; then prefersPG() || prefersMature(), which every
            // preference meets (prefersMature is `>= MATURE`): the Legacy
            // Viewer never reaches its out-of-sync branch for an adult region
            SIM_ACCESS_ADULT if access.max < SIM_ACCESS_ADULT => Some(Prompt::AdultsOnly),
            SIM_ACCESS_ADULT => Some(change),
            _ => None,
        },
        REGION_TP_ACCESS_BLOCKED => {
            let can_change = if rating == SIM_ACCESS_MATURE {
                access.max >= SIM_ACCESS_MATURE
            } else {
                access.max >= SIM_ACCESS_ADULT
            };
            Some(if can_change { Prompt::Allow } else { Prompt::NotOnAccount })
        }
        _ => None,
    }
}

impl TeleportAccess {
    /// A teleport request leaves (teleportRequest / startTeleportRequest).
    pub fn sent(&mut self, command: &NetCommand, actor: Uuid, dest: Option<String>, show_progress: bool) {
        self.last = Some(Sent {
            actor,
            command: command.clone(),
            dest,
            show_progress,
        });
    }

    /// The session's refusal, read by the TeleportFailed that follows.
    pub fn blocked(&mut self, blocked: AccessBlocked) {
        self.pending = Some(blocked);
    }

    /// The teleport screen of a restart, once.
    pub fn take_restart(&mut self) -> Option<Restart> {
        self.restart.take()
    }

    /// Disabling RLVa cancels delayed scripted actions: a scripted
    /// teleport's restart is not offered any more.
    pub fn cancel_script_teleports(&mut self) {
        if self.failed.as_ref().is_some_and(|s| !s.actor.is_nil()) {
            self.failed = None;
        }
        if self.last.as_ref().is_some_and(|s| !s.actor.is_nil()) {
            self.last = None;
        }
    }
}

impl World {
    /// The TeleportFailed of a refusal for the rating: the prompt instead
    /// of the reason. False for any other failure.
    pub(super) fn teleport_access_failed(&mut self) -> bool {
        let Some(blocked) = self.teleport_access.pending.take() else {
            return false;
        };
        let request = self.teleport_access.last.take();
        let restartable = request.as_ref().is_some_and(Sent::restartable);
        let Some(prompt) = prompt(&blocked, self.agent_access(), restartable) else {
            return false;
        };
        log::info!(
            "teleport: refused for its {} rating, {prompt:?}",
            aurora_net::search::access_short_string(blocked.region_access)
        );
        // clearTeleportRequest, except where the answer may restart it
        self.teleport_access.failed = match prompt {
            Prompt::ChangeAndReteleport | Prompt::Allow => request.filter(Sent::restartable),
            _ => None,
        };
        let rating = blocked.region_access;
        let name = crate::ui::bars::maturity_name(rating);
        let lower = name.to_lowercase();
        let (title, body, data) = match prompt {
            Prompt::AdultsOnly => (
                String::from(aurora_i18n::tr!("world-teleport-access-blocked-title")),
                aurora_i18n::tr!("world-teleport-access-adults-only", rating = lower),
                notifications::Data::Url(RATINGS_URL.into()),
            ),
            Prompt::ChangeAndReteleport => (
                String::from(aurora_i18n::tr!("world-teleport-access-blocked-title")),
                aurora_i18n::tr!("world-teleport-access-change-and-reteleport", rating = lower),
                notifications::Data::MaturityChange { rating, prompt },
            ),
            Prompt::Change => (
                String::from(aurora_i18n::tr!("world-teleport-access-blocked-title")),
                aurora_i18n::tr!("world-teleport-access-change", rating = lower),
                notifications::Data::MaturityChange { rating, prompt },
            ),
            Prompt::OutOfSync => (
                String::from(aurora_i18n::tr!("world-teleport-access-blocked-title")),
                String::from(aurora_i18n::tr!("world-teleport-access-out-of-sync")),
                notifications::Data::MaturityChange { rating, prompt },
            ),
            Prompt::Allow => (
                aurora_i18n::tr!("world-teleport-access-rated", rating = name.to_owned()),
                aurora_i18n::tr!("world-teleport-access-allow", rating = name.to_owned()),
                notifications::Data::MaturityChange { rating, prompt },
            ),
            Prompt::NotOnAccount => (
                String::from(aurora_i18n::tr!("world-teleport-access-blocked-title")),
                aurora_i18n::tr!("world-teleport-access-not-on-account", rating = name.to_owned()),
                notifications::Data::None,
            ),
        };
        self.notifications.push(notifications::Kind::Alert, title, body, data);
        true
    }

    /// The prompt's answer: `accept` is its first button (OK, "Change and
    /// continue", "Change preferences", "Allow … content").
    pub(super) fn answer_teleport_access(&mut self, rating: u8, prompt: Prompt, accept: bool) {
        let failed = self.teleport_access.failed.take();
        if !accept {
            return;
        }
        // gSavedSettings.setU32("PreferredMaturity", _region_access)
        self.set_maturity_preference(rating);
        let restart = match prompt {
            Prompt::ChangeAndReteleport => {
                self.teleport_access.changed_during_teleport = Some(rating);
                failed
            }
            Prompt::Allow => failed,
            _ => None,
        };
        // restartFailedTeleportRequest: sent once the server keeps the new
        // preference (startTeleportRequest)
        if let Some(sent) = restart {
            log::info!("teleport: restarting the refused teleport");
            self.teleport_access.restart = Some(Restart {
                dest: sent.dest,
                show_progress: sent.show_progress,
            });
            self.maturity.restart(sent.command, sent.actor);
        }
    }

    /// handleTeleportFinished / handleTeleportFailed: the request is over,
    /// and a rating changed for it is announced (PreferredMaturityChanged).
    pub(super) fn teleport_access_ended(&mut self, arrived: bool) {
        if arrived {
            self.teleport_access.last = None;
        }
        if let Some(rating) = self.teleport_access.changed_during_teleport.take() {
            let rating = crate::ui::bars::maturity_name(rating).to_lowercase();
            self.notifications.push(
                notifications::Kind::Alert,
                aurora_i18n::tr!("world-teleport-access-preference-changed-title"),
                aurora_i18n::tr!("world-teleport-access-preference-changed", rating = rating),
                notifications::Data::None,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::search::SIM_ACCESS_PG;

    fn blocked(code: &str, rating: u8) -> AccessBlocked {
        AccessBlocked {
            code: code.into(),
            region_access: rating,
        }
    }

    fn acc(max: u8, preferred: u8) -> AgentAccess {
        AgentAccess { max, preferred }
    }

    #[test]
    fn a_moderate_region_follows_the_account_and_the_preference() {
        let m = blocked(ENTRY_ACCESS_BLOCKED, SIM_ACCESS_MATURE);
        assert_eq!(prompt(&m, acc(SIM_ACCESS_PG, SIM_ACCESS_PG), true), Some(Prompt::AdultsOnly));
        assert_eq!(
            prompt(&m, acc(SIM_ACCESS_ADULT, SIM_ACCESS_PG), true),
            Some(Prompt::ChangeAndReteleport)
        );
        assert_eq!(prompt(&m, acc(SIM_ACCESS_ADULT, SIM_ACCESS_PG), false), Some(Prompt::Change));
        assert_eq!(prompt(&m, acc(SIM_ACCESS_ADULT, SIM_ACCESS_MATURE), true), Some(Prompt::OutOfSync));
    }

    #[test]
    fn an_adult_region_always_offers_the_change_to_an_adult_account() {
        let a = blocked(ENTRY_ACCESS_BLOCKED, SIM_ACCESS_ADULT);
        assert_eq!(
            prompt(&a, acc(SIM_ACCESS_MATURE, SIM_ACCESS_MATURE), true),
            Some(Prompt::AdultsOnly)
        );
        for preferred in [SIM_ACCESS_PG, SIM_ACCESS_MATURE, SIM_ACCESS_ADULT] {
            assert_eq!(
                prompt(&a, acc(SIM_ACCESS_ADULT, preferred), true),
                Some(Prompt::ChangeAndReteleport)
            );
            assert_eq!(prompt(&a, acc(SIM_ACCESS_ADULT, preferred), false), Some(Prompt::Change));
        }
        // a general region is no rating refusal
        let pg = blocked(ENTRY_ACCESS_BLOCKED, SIM_ACCESS_PG);
        assert_eq!(prompt(&pg, acc(SIM_ACCESS_ADULT, SIM_ACCESS_PG), true), None);
    }

    #[test]
    fn region_tp_access_blocked_opens_the_dialog_when_the_account_may() {
        let a = blocked(REGION_TP_ACCESS_BLOCKED, SIM_ACCESS_ADULT);
        assert_eq!(prompt(&a, acc(SIM_ACCESS_ADULT, SIM_ACCESS_PG), true), Some(Prompt::Allow));
        assert_eq!(prompt(&a, acc(SIM_ACCESS_MATURE, SIM_ACCESS_PG), true), Some(Prompt::NotOnAccount));
        let m = blocked(REGION_TP_ACCESS_BLOCKED, SIM_ACCESS_MATURE);
        assert_eq!(prompt(&m, acc(SIM_ACCESS_MATURE, SIM_ACCESS_PG), false), Some(Prompt::Allow));
        assert_eq!(prompt(&m, acc(SIM_ACCESS_PG, SIM_ACCESS_PG), false), Some(Prompt::NotOnAccount));
    }

    fn world(max: &str, preferred: u8) -> World {
        use aurora_llsd::llsd_map;
        let mut w = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        w.login = Some(std::sync::Arc::new(aurora_net::LoginResponse {
            agent_id: Uuid::nil(),
            session_id: Uuid::nil(),
            secure_session_id: Uuid::nil(),
            first_name: String::new(),
            last_name: String::new(),
            sim_ip: std::net::Ipv4Addr::LOCALHOST,
            sim_port: 0,
            circuit_code: 0,
            region_x: 0,
            region_y: 0,
            region_size_x: 256,
            region_size_y: 256,
            seed_capability: String::new(),
            look_at: glam::Vec3::X,
            message: String::new(),
            agent_appearance_service: String::new(),
            inventory_root: Uuid::nil(),
            mfa_hash: None,
            raw: llsd_map! { "agent_access_max" => max, "agent_region_access" => max },
        }));
        w.maturity.preferred = Some(preferred);
        w
    }

    fn teleport_to() -> NetCommand {
        NetCommand::TeleportTo {
            handle: 1 << 40,
            position: glam::Vec3::new(128.0, 128.0, 30.0),
            look_at: glam::Vec3::X,
        }
    }

    /// The session's two events for a refusal.
    fn refuse(w: &mut World, code: &str, rating: u8) {
        w.apply(aurora_net::NetEvent::TeleportAccessBlocked(blocked(code, rating)));
        w.apply(aurora_net::NetEvent::TeleportFailed { reason: "blocked".into() });
    }

    fn prompt_id(w: &World) -> (u64, notifications::Data) {
        let n = w.notifications.list.last().expect("a prompt");
        (n.id, n.data.clone())
    }

    #[test]
    fn change_and_continue_restarts_the_refused_teleport_once_the_server_agrees() {
        let mut w = world("A", SIM_ACCESS_PG);
        w.teleport_access.sent(&teleport_to(), Uuid::nil(), Some("Neighbour".into()), true);
        let unread = w.chat_unread;
        refuse(&mut w, ENTRY_ACCESS_BLOCKED, SIM_ACCESS_ADULT);
        // the prompt replaces the reason's nearby chat line
        assert_eq!(w.chat_unread, unread);
        let (id, data) = prompt_id(&w);
        assert!(matches!(
            data,
            notifications::Data::MaturityChange {
                rating: SIM_ACCESS_ADULT,
                prompt: Prompt::ChangeAndReteleport
            }
        ));
        w.respond_notification(id, notifications::Response::Accept);
        assert!(w.notifications.list.iter().all(|n| n.id != id));
        // the preference first; the teleport waits for the server's answer
        assert!(matches!(
            w.maturity.take_commands()[..],
            [(_, NetCommand::SetMaturityPreference(SIM_ACCESS_ADULT))]
        ));
        assert_eq!(
            w.teleport_access.take_restart(),
            Some(Restart {
                dest: Some("Neighbour".into()),
                show_progress: true,
            })
        );
        w.apply(aurora_net::NetEvent::MaturityPreference(Some(SIM_ACCESS_ADULT)));
        assert!(matches!(w.maturity.take_commands()[..], [(_, NetCommand::TeleportTo { .. })]));
        // PreferredMaturityChanged once it arrives
        let before = w.notifications.list.len();
        w.apply(aurora_net::NetEvent::TeleportFinished {
            handle: 1 << 40,
            flying: false,
        });
        assert_eq!(w.notifications.list.len(), before + 1);
        assert!(w.teleport_access.changed_during_teleport.is_none());
    }

    #[test]
    fn a_refused_lure_only_changes_the_preference() {
        let mut w = world("A", SIM_ACCESS_MATURE);
        let lure = NetCommand::AcceptLure {
            lure_id: Uuid::nil(),
            godlike: false,
        };
        w.teleport_access.sent(&lure, Uuid::nil(), None, true);
        refuse(&mut w, ENTRY_ACCESS_BLOCKED, SIM_ACCESS_ADULT);
        let (id, data) = prompt_id(&w);
        assert!(matches!(
            data,
            notifications::Data::MaturityChange {
                prompt: Prompt::Change,
                ..
            }
        ));
        w.respond_notification(id, notifications::Response::Accept);
        assert!(matches!(
            w.maturity.take_commands()[..],
            [(_, NetCommand::SetMaturityPreference(SIM_ACCESS_ADULT))]
        ));
        assert_eq!(w.teleport_access.take_restart(), None);
        w.apply(aurora_net::NetEvent::MaturityPreference(Some(SIM_ACCESS_ADULT)));
        assert!(w.maturity.take_commands().is_empty());
    }

    #[test]
    fn cancel_keeps_the_preference_and_drops_the_request() {
        let mut w = world("A", SIM_ACCESS_PG);
        w.teleport_access.sent(&teleport_to(), Uuid::nil(), None, true);
        refuse(&mut w, REGION_TP_ACCESS_BLOCKED, SIM_ACCESS_MATURE);
        let (id, data) = prompt_id(&w);
        assert!(matches!(data, notifications::Data::MaturityChange { prompt: Prompt::Allow, .. }));
        w.respond_notification(id, notifications::Response::Decline);
        assert!(w.maturity.take_commands().is_empty());
        assert_eq!(w.teleport_access.take_restart(), None);
        assert_eq!(w.agent_access().preferred, SIM_ACCESS_PG);
    }

    #[test]
    fn allow_restarts_without_the_changed_notice() {
        let mut w = world("A", SIM_ACCESS_PG);
        w.teleport_access.sent(&NetCommand::TeleportHome, Uuid::nil(), None, false);
        refuse(&mut w, REGION_TP_ACCESS_BLOCKED, SIM_ACCESS_ADULT);
        let (id, _) = prompt_id(&w);
        w.respond_notification(id, notifications::Response::Accept);
        w.maturity.take_commands();
        w.apply(aurora_net::NetEvent::MaturityPreference(Some(SIM_ACCESS_ADULT)));
        assert!(matches!(w.maturity.take_commands()[..], [(_, NetCommand::TeleportHome)]));
        let before = w.notifications.list.len();
        w.apply(aurora_net::NetEvent::TeleportFinished {
            handle: 1 << 40,
            flying: false,
        });
        assert_eq!(w.notifications.list.len(), before);
    }

    #[test]
    fn an_ordinary_failure_and_an_adults_only_account_get_no_change() {
        // no AlertInfo rating: no prompt
        let mut w = world("A", SIM_ACCESS_PG);
        w.apply(aurora_net::NetEvent::TeleportFailed { reason: "no".into() });
        assert!(w.notifications.list.is_empty());
        // a moderate account and an adult region: the ratings page
        let mut w = world("M", SIM_ACCESS_MATURE);
        w.teleport_access.sent(&teleport_to(), Uuid::nil(), None, true);
        refuse(&mut w, ENTRY_ACCESS_BLOCKED, SIM_ACCESS_ADULT);
        let (_, data) = prompt_id(&w);
        assert!(matches!(data, notifications::Data::Url(_)));
    }

    #[test]
    fn a_lure_is_never_restarted() {
        let lure = Sent {
            actor: Uuid::nil(),
            command: NetCommand::AcceptLure {
                lure_id: Uuid::nil(),
                godlike: false,
            },
            dest: None,
            show_progress: true,
        };
        assert!(!lure.restartable());
        let home = Sent {
            command: NetCommand::TeleportHome,
            ..lure
        };
        assert!(home.restartable());
    }
}
