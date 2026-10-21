//! Region restart notices: the simulator's AlertMessage read the way
//! Firestorm reads it (attempt_standard_notification and process_alert_core,
//! indra/newview/llviewermessage.cpp, originally LGPL 2.1).
//!
//! - Second Life names the notice in the AlertInfo block:
//!   `RegionRestartMinutes` / `RegionRestartSeconds`, with the region's
//!   `NAME` and `MINUTES` / `SECONDS` in its LLSD ExtraParams. Its English
//!   text (AlertData) is not shown: the viewer words it itself.
//! - OpenSim sends a system message instead: `/RESTART_X_MINUTES n` or
//!   `/RESTART_X_SECONDS n`.
//! - "Region restart cancelled." (a system message, after the slash) closes
//!   the alert.

use aurora_llsd::Llsd;

/// A region restart announced by the simulator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// Seconds left, as the simulator counts them.
    pub seconds: i32,
    /// The region's name (AlertInfo NAME); OpenSim does not give it, the
    /// viewer uses the agent's region.
    pub region: Option<String>,
    /// OpenSim's text notice (UISndRestartOpenSim instead of UISndRestart).
    pub opensim: bool,
}

/// What an AlertMessage says about a region restart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Restart {
    Notice(Notice),
    Cancelled,
}

/// The AlertInfo of an AlertMessage: its notification name and ExtraParams.
pub fn from_alert_info(name: &str, extra: &[u8]) -> Option<Notice> {
    let minutes = match name {
        "RegionRestartMinutes" => true,
        "RegionRestartSeconds" => false,
        _ => return None,
    };
    let params = parse_extra(extra);
    let seconds = if minutes {
        params["MINUTES"].as_i32().saturating_mul(60)
    } else {
        params["SECONDS"].as_i32()
    };
    let region = Some(params["NAME"].to_string_value()).filter(|n| !n.trim().is_empty());
    Some(Notice {
        seconds: seconds.max(0),
        region,
        opensim: false,
    })
}

/// The AlertData text of an AlertMessage (process_alert_core): OpenSim's
/// restart notices and the cancellation, both system messages ("/…").
pub fn from_text(message: &str) -> Option<Restart> {
    let text = message.strip_prefix('/')?;
    // LLStringUtil::convertToS32(text.substr(18)): the number after the space
    let number = |rest: &str| rest.trim().parse::<i32>().ok();
    if let Some(rest) = text.strip_prefix("RESTART_X_MINUTES") {
        return Some(Restart::Notice(Notice {
            seconds: number(rest).unwrap_or(0).max(0).saturating_mul(60),
            region: None,
            opensim: true,
        }));
    }
    if let Some(rest) = text.strip_prefix("RESTART_X_SECONDS") {
        return Some(Restart::Notice(Notice {
            seconds: number(rest).unwrap_or(0).max(0),
            region: None,
            opensim: true,
        }));
    }
    text.starts_with("Region restart cancelled.").then_some(Restart::Cancelled)
}

/// ExtraParams in any LLSD serialization (LLSDSerialize::deserialize detects
/// it); undefined when it cannot be read. Also read for the freeze notices
/// (freeze.rs).
pub(crate) fn parse_extra(extra: &[u8]) -> Llsd {
    const BINARY: &[u8] = b"<? LLSD/Binary ?>";
    let end = extra.iter().rposition(|b| *b != 0).map_or(0, |i| i + 1);
    let start = extra[..end].iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(end);
    let data = &extra[start..end];
    let parsed = if let Some(rest) = data.strip_prefix(BINARY) {
        let skip = rest.iter().take_while(|b| matches!(b, b'\r' | b'\n')).count();
        aurora_llsd::from_binary(&rest[skip..]).map(|(v, _)| v)
    } else if data.starts_with(b"<") {
        aurora_llsd::from_xml(data)
    } else {
        aurora_llsd::from_notation(data)
    };
    match parsed {
        Ok(v) => v,
        Err(e) => {
            log::warn!("alert: its parameters could not be read ({e})");
            Llsd::Undef
        }
    }
}

/// What the "announce on a channel" setting whispers for scripts
/// (fs_report_region_restart_to_channel): the seconds left, or the
/// cancellation.
pub fn announcement(seconds: Option<i32>) -> String {
    match seconds {
        Some(s) => format!("region_restart_in:{s}"),
        None => "region_restart_cancelled".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_life_names_the_notice_and_its_region() {
        let n = from_alert_info("RegionRestartMinutes", b"{'MINUTES':i5,'NAME':'Aurora Demo'}\0").expect("a notice");
        assert_eq!(n.seconds, 300);
        assert_eq!(n.region.as_deref(), Some("Aurora Demo"));
        assert!(!n.opensim);
        let n = from_alert_info(
            "RegionRestartSeconds",
            b"<?xml version=\"1.0\" ?><llsd><map><key>NAME</key><string>Iris</string><key>SECONDS</key><integer>30</integer></map></llsd>",
        )
        .expect("a notice");
        assert_eq!((n.seconds, n.region.as_deref()), (30, Some("Iris")));
        // the values may come as strings
        let n = from_alert_info("RegionRestartSeconds", b"{'SECONDS':'10','NAME':'X'}").expect("a notice");
        assert_eq!(n.seconds, 10);
    }

    #[test]
    fn other_notifications_and_unreadable_parameters() {
        assert_eq!(from_alert_info("HomePositionSet", b"{}"), None);
        let n = from_alert_info("RegionRestartMinutes", b"not llsd").expect("still a notice");
        assert_eq!((n.seconds, n.region), (0, None));
    }

    #[test]
    fn opensim_notices_are_system_messages() {
        let notice = |seconds| {
            Some(Restart::Notice(Notice {
                seconds,
                region: None,
                opensim: true,
            }))
        };
        assert_eq!(from_text("/RESTART_X_MINUTES 2"), notice(120));
        assert_eq!(from_text("/RESTART_X_SECONDS 45"), notice(45));
        assert_eq!(from_text("RESTART_X_SECONDS 45"), None, "a system message starts with a slash");
        assert_eq!(from_text("/Region restart cancelled."), Some(Restart::Cancelled));
        assert_eq!(from_text("/Region restart cancelled. By the owner"), Some(Restart::Cancelled));
        assert_eq!(from_text("Home position set."), None);
    }

    #[test]
    fn the_channel_announcement() {
        assert_eq!(announcement(Some(300)), "region_restart_in:300");
        assert_eq!(announcement(None), "region_restart_cancelled");
    }
}
