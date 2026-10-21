//! Being frozen by a parcel owner or an estate manager: the simulator's
//! ViewerFrozenMessage (the state) and its named alerts (who froze us, for
//! how long).
//!
//! - ViewerFrozenMessage carries one BOOL. process_frozen_message
//!   (indra/newview/llviewermessage.cpp, originally LGPL 2.1) only resets the
//!   busy cursor ("TODO: make being frozen change view"); Aurora keeps the
//!   state to show it on the navigation bar while it lasts.
//! - The words come with an AlertMessage whose AlertInfo names the
//!   notification (attempt_standard_notification): `AvatarFrozen`,
//!   `AvatarFrozenDuration` and `AvatarHasUnFrozenYou`, with `AV_FREEZER`
//!   and `AV_FREEZE_TIME` in the ExtraParams (notifications.xml). The viewer
//!   words them itself, in its language.

use crate::region_restart::parse_extra;

/// A named freeze notification of an AlertMessage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Alert {
    /// `[AV_FREEZER] has frozen you`, for `seconds` when the simulator says.
    Frozen { by: String, seconds: Option<i32> },
    /// `[AV_FREEZER] has unfrozen you`.
    Unfrozen { by: String },
}

/// The AlertInfo of an AlertMessage: its notification name and ExtraParams.
pub fn from_alert_info(name: &str, extra: &[u8]) -> Option<Alert> {
    if !matches!(name, "AvatarFrozen" | "AvatarFrozenDuration" | "AvatarHasUnFrozenYou") {
        return None;
    }
    let params = parse_extra(extra);
    let by = params["AV_FREEZER"].to_string_value().trim().to_owned();
    Some(match name {
        "AvatarHasUnFrozenYou" => Alert::Unfrozen { by },
        "AvatarFrozenDuration" => Alert::Frozen {
            by,
            seconds: Some(params["AV_FREEZE_TIME"].as_i32()).filter(|s| *s > 0),
        },
        _ => Alert::Frozen { by, seconds: None },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_freeze_notifications_and_their_parameters() {
        assert_eq!(
            from_alert_info("AvatarFrozen", b"{'AV_FREEZER':'Nova Demo'}\0"),
            Some(Alert::Frozen {
                by: "Nova Demo".into(),
                seconds: None
            })
        );
        assert_eq!(
            from_alert_info("AvatarFrozenDuration", b"{'AV_FREEZER':'Nova Demo','AV_FREEZE_TIME':i30}"),
            Some(Alert::Frozen {
                by: "Nova Demo".into(),
                seconds: Some(30)
            })
        );
        // the time may come as a string; a missing one is no duration
        assert_eq!(
            from_alert_info("AvatarFrozenDuration", b"{'AV_FREEZER':'X','AV_FREEZE_TIME':'45'}"),
            Some(Alert::Frozen {
                by: "X".into(),
                seconds: Some(45)
            })
        );
        assert_eq!(
            from_alert_info("AvatarFrozenDuration", b"{'AV_FREEZER':'X'}"),
            Some(Alert::Frozen {
                by: "X".into(),
                seconds: None
            })
        );
        assert_eq!(
            from_alert_info("AvatarHasUnFrozenYou", b"{'AV_FREEZER':'Nova Demo'}"),
            Some(Alert::Unfrozen { by: "Nova Demo".into() })
        );
    }

    #[test]
    fn other_notifications_are_not_freezes() {
        assert_eq!(from_alert_info("RegionRestartMinutes", b"{'MINUTES':i5}"), None);
        assert_eq!(from_alert_info("YouFrozeAvatar", b""), None);
        assert_eq!(
            from_alert_info("AvatarFrozen", b"not llsd"),
            Some(Alert::Frozen {
                by: String::new(),
                seconds: None
            })
        );
    }
}
