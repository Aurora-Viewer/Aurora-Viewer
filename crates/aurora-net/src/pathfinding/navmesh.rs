//! The region's navmesh status and the agent's right to rebake it, from
//! NavMeshGenerationStatus, AgentState and their event queue messages.
//! Port of LLPathfindingNavMeshStatus (indra/newview/llpathfindingnavmeshstatus.cpp)
//! and the agent state of LLPathfindingManager (llpathfindingmanager.cpp),
//! originally LGPL 2.1.
//!
//! Only the status is read here: the navmesh itself (RetrieveNavMeshSrc) is
//! a Havok blob that Aurora never requests.
use aurora_llsd::Llsd;
use uuid::Uuid;

/// The four states the simulator reports (sStatusPending…).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Changes wait for a rebake.
    Pending,
    Building,
    Complete,
    /// Changed again while building.
    Repending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status {
    /// `region_id`; the GET reply may omit it, the event never does.
    pub region: Uuid,
    pub version: u32,
    pub state: State,
}

impl Status {
    /// parseStatus. An unknown status string reads as complete, like the
    /// Legacy Viewer; a missing or negative version makes the status
    /// invalid (None), as an HTTP error does.
    pub fn parse(body: &Llsd) -> Option<Self> {
        let version = match body["version"] {
            Llsd::Integer(n) if n >= 0 => n as u32,
            _ => return None,
        };
        let state = match body["status"].as_str() {
            "pending" => State::Pending,
            "building" => State::Building,
            "repending" => State::Repending,
            _ => State::Complete,
        };
        Some(Self {
            region: body["region_id"].as_uuid(),
            version,
            state,
        })
    }
}

/// `can_modify_navmesh` of AgentState and AgentStateUpdate; false when the
/// field is missing, as navAgentStateRequestCoro does on an error.
pub fn can_rebake(body: &Llsd) -> bool {
    body["can_modify_navmesh"].as_bool()
}

/// The body of the rebake request (navMeshRebakeCoro).
pub fn rebake_body() -> Llsd {
    let mut body = Llsd::new_map();
    body.insert("command", "rebuild");
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_llsd::llsd_map;

    #[test]
    fn statuses_parse_like_the_legacy_viewer() {
        let region = Uuid::from_u128(3);
        for (text, state) in [
            ("pending", State::Pending),
            ("building", State::Building),
            ("complete", State::Complete),
            ("repending", State::Repending),
            ("something new", State::Complete),
        ] {
            let body = llsd_map! { "region_id" => region, "version" => 7, "status" => text };
            assert_eq!(Status::parse(&body), Some(Status { region, version: 7, state }));
        }
        assert_eq!(Status::parse(&llsd_map! { "status" => "pending" }), None);
        assert_eq!(Status::parse(&llsd_map! { "version" => -1, "status" => "pending" }), None);
        let no_region = Status::parse(&llsd_map! { "version" => 1, "status" => "building" }).expect("valid");
        assert!(no_region.region.is_nil());
    }

    #[test]
    fn agent_state_and_rebake_body() {
        assert!(can_rebake(&llsd_map! { "can_modify_navmesh" => true }));
        assert!(!can_rebake(&llsd_map! { "can_modify_navmesh" => false }));
        assert!(!can_rebake(&Llsd::new_map()));
        assert_eq!(rebake_body()["command"].as_str(), "rebuild");
    }
}
