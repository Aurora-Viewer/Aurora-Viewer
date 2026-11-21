//! `@showloc`: the SLURLs, the teleport offer's region and a classified's
//! place, read from the world the interface reads them from
//! (LLSLURL::getSLURLString, handle_lure, LLPanelProfileClassified).
use super::super::cases::{can, holds, skip_if};
use super::super::{Case, Step, Who, World};
use super::case;
use aurora_net::NetCommand;

/// The agent's SLURL names the region (escaped), with no restriction.
fn slurl_names_region(w: &World) -> bool {
    let region = w.region_name();
    !region.is_empty() && w.back_slurl().is_some_and(|s| s.contains(&crate::slurl::encode(&region)))
}

/// The agent's SLURL is "(Hidden region)", with no position.
fn slurl_hidden(w: &World) -> bool {
    let label = aurora_i18n::tr!("rlva-hidden-region");
    w.back_slurl()
        .is_some_and(|s| s.ends_with(label) && !s.contains(&crate::slurl::encode(&w.region_name())))
        && w.rlva_slurl_hidden_regions()
            .is_some_and(|(_, regions)| regions.contains(&w.region_name()))
}

/// An own classified, moved to where the agent stands.
fn moved_classified(w: &World) -> Option<NetCommand> {
    let mut c = w.profiles.classifieds.values().find(|c| c.creator == w.agent_id)?.clone();
    c.pos_global = w.agent_global()?;
    Some(NetCommand::ClassifiedUpdate(Box::new(c)))
}

pub(super) fn cases() -> Vec<Case> {
    vec![
        case(
            "privacy-L8-slurl-hidden-region",
            "@showloc hides the region of the agent's SLURL",
            vec![
                Step::Now("the SLURL names the region", holds(|w, _| slurl_names_region(w), "no region in it")),
                Step::Say(Who::Controller, "@showloc=n"),
                Step::Expect(
                    "the SLURL is (Hidden region)",
                    holds(|w, _| slurl_hidden(w), "still names the place"),
                ),
                Step::Say(Who::Controller, "@showloc=y"),
                Step::Expect(
                    "the SLURL names the region again",
                    holds(|w, _| slurl_names_region(w), "still hidden"),
                ),
            ],
        ),
        case(
            "privacy-L20-lure-region-generic",
            "@showloc: a teleport offer's region is (Hidden)",
            vec![
                Step::Say(Who::Controller, "@showloc=n"),
                Step::Expect(
                    "the invitation says (Hidden)",
                    holds(
                        |w, _| w.rlva_lure_region() == aurora_i18n::tr!("rlva-hidden-generic"),
                        "another label",
                    ),
                ),
                Step::Say(Who::Controller, "@showloc=y"),
                Step::Expect(
                    "the invitation names the region again",
                    holds(|w, _| w.rlva_lure_region() == w.region_name(), "still hidden"),
                ),
            ],
        ),
        case(
            "privacy-L10-classified-set-location",
            "@showloc: an own classified moved here is published",
            vec![
                skip_if(|w, _| {
                    moved_classified(w)
                        .is_none()
                        .then(|| "no classified of the avatar's loaded".to_owned())
                }),
                Step::Say(Who::Controller, "@showloc=n"),
                Step::Expect(
                    "its new place may be sent",
                    holds(|w, _| moved_classified(w).is_some_and(|c| can(w, &c)), "refused"),
                ),
                Step::Say(Who::Controller, "@showloc=y"),
                Step::Expect("the location shown again", holds(|w, _| !w.rlva.has("showloc"), "still hidden")),
            ],
        ),
    ]
}
