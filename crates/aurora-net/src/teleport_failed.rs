//! What a TeleportFailed says, read as `process_teleport_failed`
//! (indra/newview/llviewermessage.cpp, originally LGPL 2.1) reads it, from the
//! UDP message and from its event-queue form.
//!
//! With an AlertInfo block whose Message is not empty, that Message is the
//! code the viewer looks up (`LLAgent::sTeleportErrorMessages`) and the Info
//! Reason is the text used when the code is unknown. The block's ExtraParams
//! may name the region (`REGION_NAME`). Without it, the Info Reason is the
//! code and the text both, even when empty (OpenSim's EntityTransferModule
//! sends an empty reason): the popup still opens, as it does in the Legacy
//! Viewer. A refusal for the content rating is read first
//! (`teleport_access.rs`).

use crate::NetEvent;

/// The TeleportFailed event for these fields: `reason` is Info.Reason,
/// `alert` the first AlertInfo block's Message and ExtraParams.
pub(crate) fn event(reason: String, alert: Option<(String, &[u8])>) -> NetEvent {
    match alert.filter(|(message, _)| !message.is_empty()) {
        Some((code, extra)) => {
            let region = crate::region_restart::parse_extra(extra)["REGION_NAME"].to_string_value();
            NetEvent::TeleportFailed {
                reason,
                code: Some(code),
                region,
            }
        }
        None => NetEvent::TeleportFailed {
            code: Some(reason.clone()),
            reason,
            region: String::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(event: NetEvent) -> (String, Option<String>, String) {
        match event {
            NetEvent::TeleportFailed { reason, code, region } => (reason, code, region),
            other => panic!("not a TeleportFailed: {other:?}"),
        }
    }

    #[test]
    fn without_alert_info_the_reason_is_the_code() {
        assert_eq!(
            fields(event("Unauthorized".into(), None)),
            ("Unauthorized".into(), Some("Unauthorized".into()), String::new())
        );
    }

    #[test]
    fn an_empty_reason_is_still_the_simulators_failure() {
        // EntityTransferModule: TeleportFailed with an empty Info.Reason and
        // no AlertInfo; the Legacy Viewer shows its popup all the same
        assert_eq!(
            fields(event(String::new(), None)),
            (String::new(), Some(String::new()), String::new())
        );
        assert_eq!(fields(event(String::new(), Some((String::new(), b"")))).1, Some(String::new()));
    }

    #[test]
    fn an_empty_alert_message_counts_as_no_alert_info() {
        // getSizeFast(_PREHASH_AlertInfo, _PREHASH_Message) > 0
        let (_, code, _) = fields(event("noaccess_tport".into(), Some((String::new(), b""))));
        assert_eq!(code.as_deref(), Some("noaccess_tport"));
    }

    #[test]
    fn alert_info_gives_the_code_and_the_region() {
        let extra = br#"<llsd><map><key>REGION_NAME</key><string>Skill Arena</string></map></llsd>"#;
        let (reason, code, region) = fields(event(
            "You cannot enter".into(),
            Some(("RegionTPSpecialUsageBlocked".into(), extra.as_slice())),
        ));
        assert_eq!(reason, "You cannot enter");
        assert_eq!(code.as_deref(), Some("RegionTPSpecialUsageBlocked"));
        assert_eq!(region, "Skill Arena");
    }

    #[test]
    fn missing_or_unreadable_extra_params_leave_no_region() {
        for extra in [b"".as_slice(), b"  \0\0", b"\x01\x02"] {
            let (_, code, region) = fields(event("x".into(), Some(("no_host".into(), extra))));
            assert_eq!((code.as_deref(), region.as_str()), (Some("no_host"), ""));
        }
    }
}
