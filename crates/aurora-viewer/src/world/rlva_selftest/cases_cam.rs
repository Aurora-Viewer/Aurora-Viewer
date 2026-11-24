//! Phase 1 cases of the camera, visual effects and environment parity
//! fixes: each is checked against the state the viewer keeps, through the
//! real controller, nothing being sent. Environment forces are only made
//! under the case's own `@setenv=n`, so that nothing of the user's local
//! environment changes (an unlocked force would write it, as in the Legacy
//! Viewer).
use super::cases::{holds, test};
use super::{Case, Step, Who};

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

fn reply_is(f: impl Fn(&str) -> bool + 'static) -> super::Test {
    test(move |_, c| match c.replies.last() {
        Some(text) if f(text) => Ok(()),
        Some(text) => Err(format!("\"{text}\"")),
        None => Err("no reply".to_owned()),
    })
}

fn environment() -> Vec<Case> {
    vec![
        case(
            "cam-H2-cloudd",
            "the Legacy colour suffix d answers (rlvGetColorComponentFromCharacter)",
            vec![
                Step::Say(Who::Controller, "@getenv_cloudd={ch}"),
                Step::Expect("a number", reply_is(|text| text.parse::<f32>().is_ok())),
            ],
        ),
        case(
            "cam-H3-daytime-locked",
            "@getenv_daytime answers 2 while ENV_EDIT exists",
            vec![
                Step::Say(Who::Controller, "@setenv=n"),
                Step::Say(Who::Controller, "@getenv_daytime={ch}"),
                Step::Expect("2.000000", reply_is(|text| text == "2.000000")),
                Step::Say(Who::Controller, "@setenv=y"),
            ],
        ),
        case(
            "cam-M6-water-frozen",
            "the lock freezes the water shown with the sky (setEnvironment(fixedEnvironment_t))",
            vec![
                Step::Say(Who::Controller, "@setenv=n"),
                Step::Soon(
                    "sky and water held",
                    holds(
                        |w, _| w.rlva.environment.is_some() && w.rlva.environment_water.is_some(),
                        "the water still follows the day",
                    ),
                ),
                Step::Say(Who::Controller, "@setenv=y"),
                Step::Soon(
                    "released",
                    holds(
                        |w, _| w.rlva.environment.is_none() && w.rlva.environment_water.is_none(),
                        "ENV_EDIT stays",
                    ),
                ),
            ],
        ),
    ]
}

fn camera() -> Vec<Case> {
    vec![case(
        "cam-H1-view-held",
        "@setcam_unlock holds the view: the free cameras give way",
        vec![
            Step::Say(Who::Controller, "@setcam_unlock=n"),
            Step::Expect("held", holds(|w, _| w.rlva.camera_view_held(), "the free cameras stay")),
            Step::Say(Who::Controller, "@setcam_unlock=y"),
            Step::Expect("released", holds(|w, _| !w.rlva.camera_view_held(), "still held")),
        ],
    )]
}

pub(super) fn cases() -> Vec<Case> {
    let mut cases = environment();
    cases.extend(camera());
    cases
}
