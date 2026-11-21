//! A teleport refused for the destination's content rating: what the
//! TeleportFailed's AlertInfo block says, read the way process_teleport_failed
//! reads it before handle_teleport_access_blocked
//! (indra/newview/llviewermessage.cpp, originally LGPL 2.1).
//!
//! - The block's Message is the notification the simulator names:
//!   `TeleportEntryAccessBlocked` (the preference excludes the rating) or
//!   `RegionTPAccessBlocked` (the rating is above the preference's ceiling).
//! - Its ExtraParams (LLSD, any serialization) carry `_region_access`, the
//!   destination's rating. Without it, the failure is an ordinary one.
//!
//! The viewer decides what to offer (`world/teleport_access.rs`): the agent's
//! ceiling and preference are its own.

use aurora_llsd::Llsd;

use crate::region_restart::parse_extra;

/// The notification of a rating refusal: the preference excludes the rating.
pub const ENTRY_ACCESS_BLOCKED: &str = "TeleportEntryAccessBlocked";
/// The notification of a rating refusal: the rating is above the preference's
/// ceiling (the maturity dialog when the account may raise it).
pub const REGION_TP_ACCESS_BLOCKED: &str = "RegionTPAccessBlocked";

/// A teleport refused for the destination's content rating.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessBlocked {
    /// `ENTRY_ACCESS_BLOCKED` or `REGION_TP_ACCESS_BLOCKED`.
    pub code: String,
    /// The destination's rating (SIM_ACCESS_*), `_region_access`.
    pub region_access: u8,
}

/// The first AlertInfo block's Message and ExtraParams; None for any other
/// notification, or when the ExtraParams give no rating.
pub fn from_alert_info(code: &str, extra: &[u8]) -> Option<AccessBlocked> {
    if code != ENTRY_ACCESS_BLOCKED && code != REGION_TP_ACCESS_BLOCKED {
        return None;
    }
    let params = parse_extra(extra);
    // llsdBlock.has("_region_access"); the value is cast to U8
    let access = match &params["_region_access"] {
        Llsd::Undef => return None,
        v => v.as_i32(),
    };
    let region_access = u8::try_from(access).ok().filter(|a| *a != 0)?;
    Some(AccessBlocked {
        code: code.to_owned(),
        region_access,
    })
}

/// The event queue's ExtraParams: binary (LLSD's own type) or, from some
/// simulators, a string.
pub(crate) fn extra_bytes(v: &Llsd) -> Vec<u8> {
    match v {
        Llsd::Binary(b) => b.clone(),
        Llsd::String(s) => s.as_bytes().to_vec(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::{SIM_ACCESS_ADULT, SIM_ACCESS_MATURE};

    #[test]
    fn a_rating_refusal_names_the_destination_rating() {
        let xml = b"<llsd><map><key>_region_access</key><integer>42</integer></map></llsd>";
        assert_eq!(
            from_alert_info(ENTRY_ACCESS_BLOCKED, xml),
            Some(AccessBlocked {
                code: ENTRY_ACCESS_BLOCKED.into(),
                region_access: SIM_ACCESS_ADULT,
            })
        );
        let notation = b"{'_region_access':i21}";
        assert_eq!(
            from_alert_info(REGION_TP_ACCESS_BLOCKED, notation).map(|b| b.region_access),
            Some(SIM_ACCESS_MATURE)
        );
    }

    #[test]
    fn other_notifications_and_missing_ratings_are_ordinary_failures() {
        let xml = b"<llsd><map><key>_region_access</key><integer>42</integer></map></llsd>";
        assert_eq!(from_alert_info("RegionEntryAccessBlocked_Notify", xml), None);
        assert_eq!(from_alert_info(ENTRY_ACCESS_BLOCKED, b""), None);
        assert_eq!(from_alert_info(ENTRY_ACCESS_BLOCKED, b"<llsd><map/></llsd>"), None);
        assert_eq!(from_alert_info(ENTRY_ACCESS_BLOCKED, b"not llsd at all"), None);
        // a value no rating can take
        assert_eq!(from_alert_info(ENTRY_ACCESS_BLOCKED, b"{'_region_access':i300}"), None);
    }

    #[test]
    fn event_queue_params_are_binary_or_text() {
        let xml = "<llsd><map><key>_region_access</key><integer>21</integer></map></llsd>";
        let blocked = |v: Llsd| from_alert_info(ENTRY_ACCESS_BLOCKED, &extra_bytes(&v)).map(|b| b.region_access);
        assert_eq!(blocked(Llsd::Binary(xml.as_bytes().to_vec())), Some(SIM_ACCESS_MATURE));
        assert_eq!(blocked(Llsd::String(xml.into())), Some(SIM_ACCESS_MATURE));
        assert_eq!(blocked(Llsd::Undef), None);
    }
}
