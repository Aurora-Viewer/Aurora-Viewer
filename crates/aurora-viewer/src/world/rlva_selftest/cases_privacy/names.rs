//! `@shownames` and `@shownearby`: the stand-ins and the text filter, read
//! from the world the interface reads them from (RlvStrings::getAnonym,
//! RlvUtil::filterNames and RlvUtil::filterMentions, rlvcommon.cpp;
//! LLUrlEntryAgentRLVAnonymizedName, llurlentry.cpp).
use super::super::cases::{holds, skip_if};
use super::super::{Case, Step, Who, World};
use super::case;
use crate::world::names::{NameOptions, NameScope};
use crate::world::rlva::privacy::{anonym_in, anonymous_named};

/// The agent's own legacy name, once the name service gave it.
fn own_legacy_name(w: &World) -> Option<String> {
    w.social.avatar_names.raw(&w.agent_id).map(|n| n.legacy_name())
}

/// The agent's display name as RlvUtil::filterNames replaces it, when a
/// `\b` can stand at both of its ends.
fn own_filtered_name(w: &World) -> Option<String> {
    let name = w.social.avatar_names.rlva_filter_display_name(&w.agent_id)?;
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    (name.len() > 2 && word(name.chars().next()) && word(name.chars().next_back())).then_some(name)
}

pub(super) fn cases() -> Vec<Case> {
    vec![
        case(
            "privacy-N1-stand-in-alone",
            "@shownames: a hidden name is the stand-in alone, never \"<stand-in> Resident\"",
            vec![
                Step::Say(Who::Controller, "@shownames=n"),
                Step::Expect(
                    "the made-up resident is a bare stand-in",
                    holds(
                        |w, c| {
                            let options = NameOptions {
                                trim_resident: false,
                                ..NameOptions::default()
                            };
                            w.social.avatar_names.get_in(NameScope::World, &c.other).is_some_and(|n| {
                                n.legacy_first.is_empty()
                                    && n.complete(&options) == n.display_name
                                    && n.tag_lines(&options).0 == n.display_name
                            })
                        },
                        "a last name after the stand-in",
                    ),
                ),
                Step::Say(Who::Controller, "@shownames=y"),
                Step::Expect("the names back", holds(|w, _| !w.rlva.has("shownames"), "still hidden")),
            ],
        ),
        case(
            "privacy-N2-stand-in-of-the-language",
            "@shownames: the stand-in is the Legacy list's, English and the interface language's",
            vec![
                Step::Say(Who::Controller, "@shownames=n"),
                Step::Expect(
                    "the made-up resident's stand-in",
                    holds(
                        |w, c| {
                            w.social.avatar_names.anonymous(&c.other).display_name
                                == anonym_in(aurora_i18n::language(), &c.other.to_string())
                        },
                        "another stand-in",
                    ),
                ),
                Step::Say(Who::Controller, "@shownames=y"),
                Step::Expect("the names back", holds(|w, _| !w.rlva.has("shownames"), "still hidden")),
            ],
        ),
        case(
            "privacy-N4-rlvanonym-link",
            "@shownames: an rlvanonym link shows the stand-in of the legacy name",
            vec![
                skip_if(|w, _| own_legacy_name(w).is_none().then(|| "the avatar's name not known yet".to_owned())),
                Step::Say(Who::Controller, "@shownames=n"),
                Step::Expect(
                    "the link of the avatar shows its stand-in",
                    holds(
                        |w, _| {
                            own_legacy_name(w).is_some_and(|name| w.rlva_anonym_label(w.agent_id) == anonymous_named(&name).display_name)
                        },
                        "another label",
                    ),
                ),
                Step::Say(Who::Controller, "@shownames=y"),
                Step::Expect("the names back", holds(|w, _| !w.rlva.has("shownames"), "still hidden")),
            ],
        ),
        case(
            "privacy-N5-grid-mention",
            "@shownames: a mention written with a grid's host becomes @ and the stand-in",
            vec![
                Step::Say(Who::Controller, "@shownames=n"),
                Step::Expect(
                    "the mention is frozen",
                    holds(
                        |w, c| {
                            let text = format!("x-grid-location-info://grid.example/app/agent/{}/mention", c.other);
                            w.rlva_freeze_mentions(&text) == format!("@{}", w.rlva_anonymous(c.other).display_name)
                        },
                        "the link is left",
                    ),
                ),
                Step::Say(Who::Controller, "@shownames=y"),
                Step::Expect("the names back", holds(|w, _| !w.rlva.has("shownames"), "still hidden")),
            ],
        ),
        case(
            "privacy-M8-own-name-cleared",
            "@shownearby: the hover texts lose the agent's own name too",
            vec![
                skip_if(|w, _| {
                    own_filtered_name(w)
                        .is_none()
                        .then(|| "no name of the avatar the filter can replace".to_owned())
                }),
                Step::Say(Who::Controller, "@shownearby=n"),
                Step::Expect(
                    "the own name is erased",
                    holds(
                        |w, _| {
                            own_filtered_name(w)
                                .is_some_and(|name| !w.rlva_hover_privacy.apply(&format!("hi {name} hi")).contains(name.as_str()))
                        },
                        "the name is left",
                    ),
                ),
                Step::Say(Who::Controller, "@shownearby=y"),
                Step::Expect("the names back", holds(|w, _| !w.rlva.has("shownearby"), "still hidden")),
            ],
        ),
    ]
}
