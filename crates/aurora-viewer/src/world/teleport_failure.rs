//! What a teleport failure tells the resident.
//!
//! Port of `process_teleport_failed` (indra/newview/llviewermessage.cpp,
//! originally LGPL 2.1): the simulator's code is looked up in
//! `LLAgent::sTeleportErrorMessages` (skins/default/xui/en/teleport_strings.xml,
//! "errors"), `[REGION_NAME]` filled from the AlertInfo, else the
//! simulator's own text is shown; the `CouldNotTeleportReason` popup
//! (notifications.xml) says "Teleport failed." with it.
//!
//! Difference, beyond the Legacy Viewer: OpenSim's Hypergrid services refuse
//! with short English texts ("Unauthorized", "Could not resolve Landmark grid
//! gatekeeper"…) that do not say what happened. On an OpenSim grid, the popup
//! adds one line that explains the usual cause.

/// A failure the simulator sent, kept until its popup is closed. A new one
/// replaces it (the notification is `<unique/>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub code: String,
    pub reason: String,
    pub region: String,
}

/// Where the agent is, for the Hypergrid explanations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grid {
    SecondLife,
    /// An OpenSim grid, the one we logged in to.
    OpenSimHome,
    /// An OpenSim grid reached by Hypergrid.
    OpenSimAway,
}

/// The reason as the Legacy Viewer words it: its own text for a code it
/// knows, else the simulator's text, else the code.
pub fn text(code: &str, reason: &str, region: &str) -> String {
    match legacy(code, region) {
        Some(text) => text,
        None if !reason.is_empty() => reason.to_owned(),
        None => code.to_owned(),
    }
}

/// teleport_strings.xml, "errors".
fn legacy(code: &str, region: &str) -> Option<String> {
    use aurora_i18n::tr;
    let text: &str = match code {
        "invalid_tport" => tr!("teleport-error-invalid-tport"),
        "invalid_region_handoff" => tr!("teleport-error-invalid-region-handoff"),
        "blocked_tport" => tr!("teleport-error-blocked-tport"),
        "nolandmark_tport" => tr!("teleport-error-nolandmark-tport"),
        "timeout_tport" => tr!("teleport-error-timeout-tport"),
        "NoHelpIslandTP" => tr!("teleport-error-no-help-island"),
        "noaccess_tport" => tr!("teleport-error-noaccess-tport"),
        "missing_attach_tport" => tr!("teleport-error-missing-attach-tport"),
        "too_many_uploads_tport" => tr!("teleport-error-too-many-uploads-tport"),
        "expired_tport" => tr!("teleport-error-expired-tport"),
        "expired_region_handoff" => tr!("teleport-error-expired-region-handoff"),
        "preexisting_tport" => tr!("teleport-error-preexisting-tport"),
        "no_host" => tr!("teleport-error-no-host"),
        "no_inventory_host" => tr!("teleport-error-no-inventory-host"),
        "MustGetAgeRegion" => tr!("teleport-error-must-get-age-region"),
        "RegionTPSpecialUsageBlocked" => {
            return Some(tr!("teleport-error-special-usage-blocked", region = region.to_owned()).into());
        }
        _ => return None,
    };
    Some(text.to_owned())
}

/// The line under the reason that explains an OpenSim Hypergrid refusal, by
/// the texts of OpenSim's gatekeeper and entity transfer services. None on
/// Second Life and for any other reason.
pub fn hint(reason: &str, grid: Grid) -> Option<&'static str> {
    use aurora_i18n::tr;
    if grid == Grid::SecondLife {
        return None;
    }
    let said = reason.trim().trim_end_matches('.').to_ascii_lowercase();
    match said.as_str() {
        // GatekeeperService.LoginAgent: IsAgentComingHome refused a local
        // account coming back (its travel record does not name this grid's
        // gatekeeper address)
        "unauthorized" if grid == Grid::OpenSimAway => Some(tr!("teleport-hint-hypergrid-home-refused")),
        "error contacting grid" | "could not resolve landmark grid gatekeeper" => Some(tr!("teleport-hint-hypergrid-unreachable")),
        "destination does not allow visitors from your world" => Some(tr!("teleport-hint-hypergrid-visitors-refused")),
        "landmark region not found" | "the teleport destination could not be found" => {
            Some(tr!("teleport-hint-hypergrid-destination-gone"))
        }
        _ if said.contains("gatekeeper") => Some(tr!("teleport-hint-hypergrid-unreachable")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_known_code_takes_the_legacy_text() {
        assert_eq!(text("noaccess_tport", "", ""), aurora_i18n::tr!("teleport-error-noaccess-tport"));
        // the AlertInfo's code wins over the simulator's text
        assert_eq!(
            text("no_host", "whatever the region said", ""),
            aurora_i18n::tr!("teleport-error-no-host")
        );
    }

    #[test]
    fn an_unknown_code_shows_what_the_simulator_said() {
        assert_eq!(text("Unauthorized", "Unauthorized", ""), "Unauthorized");
        assert_eq!(text("SomeNewTag", "Readable text", ""), "Readable text");
        assert_eq!(text("SomeNewTag", "", ""), "SomeNewTag");
    }

    #[test]
    fn the_region_name_is_filled_in() {
        let said = text("RegionTPSpecialUsageBlocked", "", "Skill Arena");
        assert!(said.contains("Skill Arena"), "{said}");
    }

    #[test]
    fn unauthorized_away_from_home_explains_the_refused_return() {
        assert_eq!(
            hint("Unauthorized", Grid::OpenSimAway),
            Some(aurora_i18n::tr!("teleport-hint-hypergrid-home-refused"))
        );
        // at home it is not a Hypergrid return, and Second Life never gets one
        assert_eq!(hint("Unauthorized", Grid::OpenSimHome), None);
        assert_eq!(hint("Unauthorized", Grid::SecondLife), None);
    }

    #[test]
    fn opensim_hypergrid_texts_get_their_explanation() {
        let unreachable = Some(aurora_i18n::tr!("teleport-hint-hypergrid-unreachable"));
        assert_eq!(hint("Could not resolve Landmark grid gatekeeper", Grid::OpenSimHome), unreachable);
        assert_eq!(hint("Error contacting grid.", Grid::OpenSimAway), unreachable);
        assert_eq!(
            hint("Destination does not allow visitors from your world", Grid::OpenSimHome),
            Some(aurora_i18n::tr!("teleport-hint-hypergrid-visitors-refused"))
        );
        let gone = Some(aurora_i18n::tr!("teleport-hint-hypergrid-destination-gone"));
        assert_eq!(hint("landmark region not found", Grid::OpenSimAway), gone);
        assert_eq!(hint("The teleport destination could not be found.", Grid::OpenSimHome), gone);
        assert_eq!(hint("Region is full", Grid::OpenSimHome), None);
        assert_eq!(hint("Could not resolve Landmark grid gatekeeper", Grid::SecondLife), None);
    }
}
