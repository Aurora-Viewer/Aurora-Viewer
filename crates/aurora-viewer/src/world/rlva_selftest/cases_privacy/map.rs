//! Map and tag privacy parity (`@shownametags`, `@showworldmap`,
//! `@shownearby`, `@showhovertext*`).
use super::super::cases::{holds, skip_if};
use super::super::{Case, Step, Who};
use super::case;

pub(super) fn cases() -> Vec<Case> {
    vec![
        case(
            "privacy-M1-plain-cancels-distance",
            "a plain @shownametags cancels a name tag distance",
            vec![
                Step::Say(Who::Controller, "@shownametags:5=n"),
                Step::Expect(
                    "a tag inside the distance shown",
                    holds(|w, c| w.rlva.nametag_visible(c.other, c.agent, 3.0), "hidden at 3 m"),
                ),
                Step::Now(
                    "a tag beyond the distance hidden",
                    holds(|w, c| !w.rlva.nametag_visible(c.other, c.agent, 6.0), "shown at 6 m"),
                ),
                Step::Say(Who::Controller, "@shownametags=n"),
                Step::Expect(
                    "the plain rule wins (distance 0)",
                    holds(|w, c| !w.rlva.nametag_visible(c.other, c.agent, 3.0), "still shown at 3 m"),
                ),
                Step::Say(Who::Controller, "@shownametags=y"),
                Step::Expect(
                    "the distance back",
                    holds(|w, c| w.rlva.nametag_visible(c.other, c.agent, 3.0), "still hidden at 3 m"),
                ),
                Step::Say(Who::Controller, "@shownametags:5=y"),
                Step::Expect(
                    "every tag back",
                    holds(|w, c| w.rlva.nametag_visible(c.other, c.agent, 60.0), "still hidden"),
                ),
            ],
        ),
        case(
            "privacy-M7-near-me-under-shownearby",
            "@shownearby leaves the avatar picker and the name prediction",
            vec![
                skip_if(|w, _| crate::ui::people::around(w).is_empty().then(|| "nobody around".to_owned())),
                Step::Say(Who::Controller, "@shownearby=n"),
                Step::Expect(
                    "the nearby list hidden",
                    holds(|w, _| crate::ui::people::nearby(w).is_empty(), "still listed"),
                ),
                Step::Now(
                    "the avatar picker's Near me tab still lists them",
                    holds(|w, _| !crate::ui::people::around(w).is_empty(), "nobody listed"),
                ),
                Step::Say(Who::Controller, "@shownearby=y"),
                Step::Expect("the nearby list back", holds(|w, _| !w.rlva.has("shownearby"), "still hidden")),
            ],
        ),
    ]
}
