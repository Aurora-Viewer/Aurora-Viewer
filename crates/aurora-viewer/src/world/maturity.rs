//! The content rating the agent wants to see, changed during the session
//! like Firestorm does (indra/newview/llagent.cpp, originally LGPL 2.1):
//! sendMaturityPreferenceToServer sends it through UpdateAgentInformation,
//! processMaturityPreferenceFromServer / handlePreferredMaturityResult /
//! handlePreferredMaturityError read the answer, sending the request again
//! up to three times while the server keeps another value, then report
//! MaturityChangeError and fall back to the server's value. A teleport asked
//! while the preference is not in sync with the server waits for it
//! (startTeleportRequest, TELEPORT_PENDING), and is dropped on an error
//! (reportPreferredMaturityError).

use aurora_net::NetCommand;
use uuid::Uuid;

/// Requests sent again while the server answers another value.
const MAX_RETRIES: u8 = 3;

#[derive(Debug, Default)]
pub struct MaturityPreference {
    /// The preference chosen during this session (None: the login's).
    pub preferred: Option<u8>,
    request_id: u32,
    response_id: u32,
    /// The last value asked for, and the last one the server answered
    /// (before any answer: the value the change started from).
    last_request: u8,
    last_response: u8,
    retries: u8,
    /// The teleport waiting for the preference to be in sync.
    held: Option<(Uuid, NetCommand)>,
    out: Vec<(Uuid, NetCommand)>,
}

/// What an answer of the server leads to.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// More answers are expected, or the request was sent again.
    Waiting,
    /// In sync with the server.
    Synced,
    /// The server keeps `actual` instead of `preferred`.
    Failed { preferred: u8, actual: u8 },
}

impl MaturityPreference {
    /// Every request has its answer (isMaturityPreferenceSyncedWithServer).
    pub fn synced(&self) -> bool {
        self.request_id == self.response_id
    }

    /// Ask the server for `maturity`, from `current` (the preference shown
    /// until the server answers).
    pub fn request(&mut self, maturity: u8, current: u8) {
        if self.synced() {
            self.last_response = current;
        }
        self.preferred = Some(maturity);
        self.send(maturity);
    }

    fn send(&mut self, maturity: u8) {
        self.request_id += 1;
        self.last_request = maturity;
        self.out.push((Uuid::nil(), NetCommand::SetMaturityPreference(maturity)));
    }

    /// A teleport asked while the preference is not in sync waits for it.
    pub fn hold(&mut self, teleport: NetCommand, actor: Uuid) {
        self.held = Some((actor, teleport));
    }

    /// Disabling RLVa cancels delayed scripted actions, including a teleport
    /// already released by a maturity answer but not yet sent by the app.
    pub fn cancel_script_teleports(&mut self) {
        if self.held.as_ref().is_some_and(|(actor, _)| !actor.is_nil()) {
            self.held = None;
        }
        self.out.retain(|(actor, _)| actor.is_nil());
    }

    pub fn holds_teleport(&self) -> bool {
        self.held.is_some()
    }

    /// The server's answer: the value it kept, or None when the request
    /// failed. The waiting teleport is sent once in sync, dropped on a
    /// failure; on a failure the preference goes back to the server's.
    pub fn answer(&mut self, result: Option<u8>) -> Outcome {
        self.response_id += 1;
        if let Some(server) = result {
            self.last_response = server;
        }
        if !self.synced() {
            return Outcome::Waiting;
        }
        let in_sync = self.last_request == self.last_response;
        let outcome = match result {
            _ if in_sync => {
                self.retries = 0;
                Outcome::Synced
            }
            Some(_) if self.retries < MAX_RETRIES => {
                self.retries += 1;
                log::info!(
                    "maturity: retry #{} for {}",
                    self.retries,
                    aurora_net::search::access_short_string(self.last_request)
                );
                let again = self.last_request;
                self.send(again);
                return Outcome::Waiting;
            }
            _ => {
                self.retries = 0;
                Outcome::Failed {
                    preferred: self.last_request,
                    actual: self.last_response,
                }
            }
        };
        match outcome {
            Outcome::Synced => {
                if let Some(teleport) = self.held.take() {
                    self.out.push(teleport);
                }
            }
            Outcome::Failed { actual, .. } => {
                self.held = None;
                self.preferred = Some(actual);
            }
            Outcome::Waiting => {}
        }
        outcome
    }

    /// The requests to send, and the teleport released by the sync.
    pub fn take_commands(&mut self) -> Vec<(Uuid, NetCommand)> {
        std::mem::take(&mut self.out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::search::{SIM_ACCESS_ADULT, SIM_ACCESS_MATURE, SIM_ACCESS_PG};

    fn teleport() -> NetCommand {
        NetCommand::TeleportHome
    }

    #[test]
    fn an_accepted_change_releases_the_waiting_teleport() {
        let mut m = MaturityPreference::default();
        assert!(m.synced());
        m.request(SIM_ACCESS_MATURE, SIM_ACCESS_PG);
        assert!(!m.synced());
        assert!(matches!(
            m.take_commands()[..],
            [(_, NetCommand::SetMaturityPreference(SIM_ACCESS_MATURE))]
        ));
        m.hold(teleport(), Uuid::nil());
        assert_eq!(m.answer(Some(SIM_ACCESS_MATURE)), Outcome::Synced);
        assert!(m.synced());
        assert!(matches!(m.take_commands()[..], [(_, NetCommand::TeleportHome)]));
        assert_eq!(m.preferred, Some(SIM_ACCESS_MATURE));
    }

    #[test]
    fn a_server_keeping_another_value_is_asked_three_more_times_then_wins() {
        let mut m = MaturityPreference::default();
        m.request(SIM_ACCESS_ADULT, SIM_ACCESS_PG);
        m.hold(teleport(), Uuid::nil());
        m.take_commands();
        for _ in 0..MAX_RETRIES {
            assert_eq!(m.answer(Some(SIM_ACCESS_PG)), Outcome::Waiting);
            assert!(matches!(
                m.take_commands()[..],
                [(_, NetCommand::SetMaturityPreference(SIM_ACCESS_ADULT))]
            ));
            assert!(m.holds_teleport());
        }
        assert_eq!(
            m.answer(Some(SIM_ACCESS_PG)),
            Outcome::Failed {
                preferred: SIM_ACCESS_ADULT,
                actual: SIM_ACCESS_PG
            }
        );
        // the teleport is dropped, the preference is the server's
        assert!(m.take_commands().is_empty());
        assert!(!m.holds_teleport());
        assert_eq!(m.preferred, Some(SIM_ACCESS_PG));
        assert!(m.synced());
    }

    #[test]
    fn a_failed_request_reports_at_once_unless_already_in_sync() {
        let mut m = MaturityPreference::default();
        m.request(SIM_ACCESS_MATURE, SIM_ACCESS_PG);
        m.hold(teleport(), Uuid::nil());
        assert_eq!(
            m.answer(None),
            Outcome::Failed {
                preferred: SIM_ACCESS_MATURE,
                actual: SIM_ACCESS_PG
            }
        );
        assert!(!m.holds_teleport());
        // asking for what the server already has: an error is no failure
        let mut m = MaturityPreference::default();
        m.request(SIM_ACCESS_PG, SIM_ACCESS_PG);
        assert_eq!(m.answer(None), Outcome::Synced);
    }

    #[test]
    fn the_world_uses_the_new_rating_and_reports_a_failure() {
        use crate::world::World;
        use aurora_llsd::llsd_map;
        let mut w = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let login = |max: &str| aurora_net::LoginResponse {
            agent_id: uuid::Uuid::nil(),
            session_id: uuid::Uuid::nil(),
            secure_session_id: uuid::Uuid::nil(),
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
            inventory_root: uuid::Uuid::nil(),
            mfa_hash: None,
            raw: llsd_map! { "agent_access_max" => max, "agent_region_access" => max },
        };
        w.login = Some(std::sync::Arc::new(login("A")));
        assert_eq!(w.agent_access().preferred, SIM_ACCESS_ADULT);
        // asked: shown at once (as Firestorm's setting), confirmed by the server
        w.set_maturity_preference(SIM_ACCESS_MATURE);
        assert_eq!(w.agent_access().preferred, SIM_ACCESS_MATURE);
        assert!(!w.maturity.synced());
        w.apply(aurora_net::NetEvent::MaturityPreference(Some(SIM_ACCESS_MATURE)));
        assert!(w.maturity.synced());
        assert!(w.notifications.list.is_empty());
        // asked again, the request fails: back to what the server has, and
        // MaturityChangeError
        w.set_maturity_preference(SIM_ACCESS_PG);
        w.apply(aurora_net::NetEvent::MaturityPreference(None));
        assert_eq!(w.agent_access().preferred, SIM_ACCESS_MATURE);
        assert_eq!(w.notifications.list.len(), 1);
        // the ceiling of the account still holds
        w.login = Some(std::sync::Arc::new(login("M")));
        w.maturity.preferred = Some(SIM_ACCESS_ADULT);
        assert_eq!(w.agent_access().preferred, SIM_ACCESS_MATURE);
    }

    #[test]
    fn only_the_last_of_several_requests_counts() {
        let mut m = MaturityPreference::default();
        m.request(SIM_ACCESS_MATURE, SIM_ACCESS_PG);
        m.request(SIM_ACCESS_ADULT, SIM_ACCESS_PG);
        assert_eq!(m.answer(Some(SIM_ACCESS_MATURE)), Outcome::Waiting);
        assert!(!m.synced());
        assert_eq!(m.answer(Some(SIM_ACCESS_ADULT)), Outcome::Synced);
        assert_eq!(m.preferred, Some(SIM_ACCESS_ADULT));
    }
}
