//! Nearby voice moderation by estate managers and parcel owners: mute one
//! resident, or everyone, in the region's spatial voice, through the
//! SpatialVoiceModerationRequest capability.
//!
//! Port of LLNearbyVoiceModeration::requestMuteIndividual / requestMuteAll
//! (indra/newview/llnearbyvoicemoderation.cpp, originally LGPL 2.1). The
//! state comes back on the WebRTC data channel (`"m"`, aurora-voice).

use aurora_llsd::{Llsd, llsd_map};
use uuid::Uuid;

/// Whom a moderation request is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Resident(Uuid),
    Everyone,
}

/// The POST body: `{operand: "mute" | "unmute", agent_id}` for one
/// resident, `{operand: "mute_all" | "unmute_all"}` for everyone.
pub fn body(target: Target, mute: bool) -> Llsd {
    match target {
        Target::Resident(agent_id) => llsd_map! {
            "operand" => if mute { "mute" } else { "unmute" },
            "agent_id" => agent_id,
        },
        Target::Everyone => llsd_map! { "operand" => if mute { "mute_all" } else { "unmute_all" } },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decoded(target: Target, mute: bool) -> Llsd {
        aurora_llsd::from_xml(&aurora_llsd::to_xml(&body(target, mute))).unwrap()
    }

    #[test]
    fn one_resident_carries_its_id() {
        let id = Uuid::from_u128(7);
        let b = decoded(Target::Resident(id), true);
        assert_eq!(b["operand"].as_str(), "mute");
        assert_eq!(b["agent_id"].as_uuid(), id);
        assert_eq!(decoded(Target::Resident(id), false)["operand"].as_str(), "unmute");
    }

    #[test]
    fn everyone_has_no_id() {
        let b = decoded(Target::Everyone, true);
        assert_eq!(b["operand"].as_str(), "mute_all");
        assert_eq!(b.as_map().unwrap().len(), 1);
        assert_eq!(decoded(Target::Everyone, false)["operand"].as_str(), "unmute_all");
    }
}
