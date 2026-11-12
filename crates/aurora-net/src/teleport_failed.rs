//! What a TeleportFailed says, read as `process_teleport_failed`
//! (indra/newview/llviewermessage.cpp, originally LGPL 2.1) reads it, from the
//! UDP message and from its event-queue form.
//!
//! With an AlertInfo block whose Message is not empty, that Message is the
//! code the viewer looks up (`LLAgent::sTeleportErrorMessages`) and the Info
//! Reason is the text used when the code is unknown. The block's ExtraParams
//! may name the region (`REGION_NAME`). Without it, the Info Reason is the
//! code and the text both.

use crate::NetEvent;

/// The TeleportFailed event for these fields: `reason` is Info.Reason,
/// `alert` the first AlertInfo block's Message and ExtraParams.
pub(crate) fn event(reason: String, alert: Option<(String, &[u8])>) -> NetEvent {
    match alert.filter(|(message, _)| !message.is_empty()) {
        Some((code, extra)) => {
            let region = crate::region_restart::parse_extra(extra)["REGION_NAME"].to_string_value();
            NetEvent::TeleportFailed { reason, code, region }
        }
        None => NetEvent::TeleportFailed {
            code: reason.clone(),
            reason,
            region: String::new(),
        },
    }
}

/// The ExtraParams of the event queue's AlertInfo, binary or text.
pub(crate) fn llsd_bytes(value: &aurora_llsd::Llsd) -> Vec<u8> {
    match value {
        aurora_llsd::Llsd::Binary(bytes) => bytes.clone(),
        other => other.to_string_value().into_bytes(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(event: NetEvent) -> (String, String, String) {
        match event {
            NetEvent::TeleportFailed { reason, code, region } => (reason, code, region),
            other => panic!("not a TeleportFailed: {other:?}"),
        }
    }

    #[test]
    fn without_alert_info_the_reason_is_the_code() {
        assert_eq!(
            fields(event("Unauthorized".into(), None)),
            ("Unauthorized".into(), "Unauthorized".into(), String::new())
        );
    }

    #[test]
    fn an_empty_alert_message_counts_as_no_alert_info() {
        // getSizeFast(_PREHASH_AlertInfo, _PREHASH_Message) > 0
        let (_, code, _) = fields(event("noaccess_tport".into(), Some((String::new(), b""))));
        assert_eq!(code, "noaccess_tport");
    }

    #[test]
    fn alert_info_gives_the_code_and_the_region() {
        let extra = br#"<llsd><map><key>REGION_NAME</key><string>Skill Arena</string></map></llsd>"#;
        let (reason, code, region) = fields(event(
            "You cannot enter".into(),
            Some(("RegionTPSpecialUsageBlocked".into(), extra.as_slice())),
        ));
        assert_eq!(reason, "You cannot enter");
        assert_eq!(code, "RegionTPSpecialUsageBlocked");
        assert_eq!(region, "Skill Arena");
    }

    #[test]
    fn unreadable_extra_params_leave_no_region() {
        let (_, code, region) = fields(event("x".into(), Some(("no_host".into(), b"\x01\x02".as_slice()))));
        assert_eq!((code.as_str(), region.as_str()), ("no_host", ""));
    }
}
