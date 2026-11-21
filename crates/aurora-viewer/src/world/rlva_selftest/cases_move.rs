//! Phase 1 cases of the movement, sitting and teleport parity fixes: rules
//! set through the relays, then the boundary and the windows' predicates
//! asked. Nothing they restrict is ever sent.
use super::cases::{holds, skip_if};
use super::{Case, Ctx, Step, Who, World};
use aurora_net::NetCommand;
use glam::Vec3;
use uuid::Uuid;

fn case(id: &'static str, title: &'static str, steps: Vec<Step>) -> Case {
    Case {
        id,
        title,
        phase: 1,
        gate: false,
        known: None,
        steps,
    }
}

/// A teleport `distance` metres east in the region the avatar stands in.
fn local_teleport(w: &World, distance: f32) -> Option<NetCommand> {
    Some(NetCommand::TeleportTo {
        handle: w.main_region?,
        position: w.agent.position + Vec3::X * distance,
        look_at: Vec3::X,
    })
}

/// The controller's own @tpto of `distance` metres would pass.
fn controller_may_teleport(w: &World, c: &Ctx, distance: f32) -> bool {
    let actor = c.speaker(w, Who::Controller).unwrap_or(Uuid::nil());
    local_teleport(w, distance).is_some_and(|command| w.rlva_allows_command(&command, actor))
}

fn standing_in_a_region(w: &World, c: &Ctx) -> Option<String> {
    if w.main_region.is_none() {
        Some("the region is not known yet".to_owned())
    } else if w.agent.is_sitting() {
        Some("the avatar is seated".to_owned())
    } else if c.foreign_rules(w) {
        Some("a restriction of another object is held".to_owned())
    } else if c.speaker(w, Who::Second).is_none() {
        Some("the second relay is not worn".to_owned())
    } else {
        None
    }
}

fn places_free(w: &World, _: &Ctx) -> bool {
    w.rlva_can_teleport_location() && w.rlva_landmark_button_enabled() && w.rlva_home_button_enabled()
}

pub(super) fn phase1() -> Vec<Case> {
    vec![
        // RlvBehaviourModifier (rlvmodifiers.h, CompMin): the @tplocal
        // distance is the smallest of every object's, the issuer's own
        // included; only whether the rule is held leaves the issuer out
        // (RlvActions::canTeleportToLocal, rlvactions.cpp).
        case(
            "move-G1-tplocal-own-value",
            "@tpto keeps the issuer's own @tplocal distance",
            vec![
                skip_if(standing_in_a_region),
                Step::Now(
                    "a 1 m teleport allowed before the rules",
                    holds(|w, c| controller_may_teleport(w, c, 1.0), "already refused"),
                ),
                Step::Say(Who::Controller, "@tplocal:0.5=n"),
                Step::Say(Who::Second, "@tplocal:5=n"),
                Step::Expect(
                    "both rules held",
                    holds(|w, _| w.rlva.rules_for("tplocal", Uuid::nil()).count() >= 2, "not held"),
                ),
                Step::Now(
                    "the controller's 1 m @tpto is refused by its own 0.5 m",
                    holds(|w, c| !controller_may_teleport(w, c, 1.0), "the 5 m of the other object was used"),
                ),
                Step::Now(
                    "a 0.3 m one still passes",
                    holds(|w, c| controller_may_teleport(w, c, 0.3), "refused"),
                ),
                Step::Say(Who::Controller, "@tplocal:0.5=y"),
                Step::Say(Who::Second, "@tplocal:5=y"),
                Step::Expect(
                    "allowed again once released",
                    holds(|w, c| controller_may_teleport(w, c, 1.0), "still refused"),
                ),
            ],
        ),
        // LLPanelPlaces::updateVerbs, LLTeleportHistoryPanel and
        // LLPanelPlaceInfo (canTeleportToLocation), LLLandmarksPanel (@tplm),
        // LLFloaterWorldMap and LLNavigationBar's Home (@tplm and @tploc).
        case(
            "move-G9-place-buttons",
            "@tploc and @tplm grey the place buttons as the Legacy windows do",
            vec![
                skip_if(|w, c| {
                    (!places_free(w, c) && c.foreign_rules(w)).then(|| "already refused by a restriction of another object".to_owned())
                }),
                Step::Now("all allowed before the rules", holds(places_free, "already greyed")),
                Step::Say(Who::Controller, "@tploc=n"),
                Step::Expect(
                    "a place's Teleport greyed under @tploc",
                    holds(|w, _| !w.rlva_can_teleport_location(), "still enabled"),
                ),
                Step::Now(
                    "the landmarks and Home left under @tploc alone",
                    holds(|w, _| w.rlva_landmark_button_enabled() && w.rlva_home_button_enabled(), "greyed"),
                ),
                Step::Say(Who::Controller, "@tplm=n"),
                Step::Expect(
                    "the landmarks and Home greyed under both",
                    holds(
                        |w, _| !w.rlva_landmark_button_enabled() && !w.rlva_home_button_enabled(),
                        "still enabled",
                    ),
                ),
                Step::Say(Who::Controller, "@tploc=y"),
                Step::Expect(
                    "Home back under @tplm alone, the landmarks still greyed",
                    holds(
                        |w, _| w.rlva_home_button_enabled() && !w.rlva_landmark_button_enabled(),
                        "not as the Legacy windows",
                    ),
                ),
                Step::Say(Who::Controller, "@tplm=y"),
                Step::Expect("all allowed again once released", holds(places_free, "still greyed")),
            ],
        ),
        // RlvForceHandler<RLV_BHVR_TPTO> (rlvhandler.cpp): a region by name
        // waits for the map, and the teleport it resolves to is checked
        // then; nothing is refused before.
        case(
            "move-G12-region-by-name",
            "a teleport to an unknown region by name waits for the map",
            vec![
                skip_if(|w, c| c.foreign_rules(w).then(|| "a restriction of another object is held".to_owned())),
                Step::Say(Who::Controller, "@tploc=n"),
                Step::Expect("the rule held", holds(|w, _| w.rlva.has("tploc"), "not held")),
                Step::Now(
                    "a region the map does not know is not refused yet",
                    holds(
                        |w, _| {
                            w.rlva_allows_command(
                                &NetCommand::TeleportToRegion {
                                    request: Uuid::nil(),
                                    name: "Aurora Selftest Nowhere".to_owned(),
                                    position: Vec3::new(128.0, 128.0, 30.0),
                                    look_at: Vec3::X,
                                },
                                Uuid::nil(),
                            )
                        },
                        "refused before the map answered",
                    ),
                ),
                Step::Say(Who::Controller, "@tploc=y"),
                Step::Expect("released", holds(|w, _| !w.rlva.has("tploc"), "still held")),
            ],
        ),
        // LLIMProcessing::processNewMessage (llimprocessing.cpp): a god's
        // lure (IM_GODLIKE_LURE_USER) keeps its text under @showloc.
        case(
            "move-G16-godlike-lure",
            "@showloc censors a lure's text, never a god's",
            vec![
                skip_if(|w, c| c.foreign_rules(w).then(|| "a restriction of another object is held".to_owned())),
                Step::Say(Who::Controller, "@showloc=n"),
                Step::Expect(
                    "a resident's lure text hidden",
                    holds(|w, c| w.rlva_teleport_message("Join me", c.other, false) != "Join me", "shown"),
                ),
                Step::Now(
                    "a god's lure text kept",
                    holds(|w, c| w.rlva_teleport_message("Join me", c.other, true) == "Join me", "hidden"),
                ),
                Step::Say(Who::Controller, "@showloc=y"),
                Step::Expect("released", holds(|w, _| !w.rlva.has("showloc"), "still held")),
            ],
        ),
    ]
}
