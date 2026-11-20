//! Phase 1, the command core: how a command is read and in which order its
//! replies leave (RlvCommand, RlvBehaviourDictionary::getCommands and
//! RlvHandler::processCommand, indra/newview/rlvhelper.cpp and
//! rlvhandler.cpp). Each case only sets rules through the controller and
//! reads the replies on the test's channel: nothing is sent in-world.
use super::cases::test;
use super::{Case, Ctx, Step, Test, Who, World};

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

/// The last reply on the test's channel is `expected`.
fn last_is(expected: &'static str) -> Test {
    test(move |_: &World, c: &mut Ctx| match c.replies.last() {
        Some(text) if text == expected => Ok(()),
        Some(text) => Err(format!("\"{text}\"")),
        None => Err("no reply".to_owned()),
    })
}

pub(super) fn cases() -> Vec<Case> {
    vec![
        case(
            "core-G2-reply-channel-trailing",
            "a reply channel followed by other text still answers (LLStringUtil::convertToS32)",
            vec![
                Step::Say(Who::Controller, "@versionnum={ch}abc"),
                Step::Expect("3040300", last_is("3040300")),
            ],
        ),
        case(
            "core-G2-notify-channel-trailing",
            "@notify reads its channel the same way",
            vec![
                Step::Say(Who::Controller, "@notify:{ch}abc=add"),
                Step::Say(Who::Controller, "@showminimap=n"),
                Step::Expect("/showminimap=n", last_is("/showminimap=n")),
            ],
        ),
        case(
            "core-G5-reply-order",
            "replies leave in the order of their commands, a queued query first",
            vec![
                Step::Say(Who::Controller, "@getgroup={ch},versionnum={ch}"),
                Step::Expect(
                    "the group, then 3040300",
                    test(|_, c| {
                        let n = c.replies.len();
                        if n >= 2 && c.replies[n - 1] == "3040300" && c.replies[n - 2] != "3040300" {
                            Ok(())
                        } else {
                            Err(format!("{:?}", c.replies))
                        }
                    }),
                ),
            ],
        ),
        case(
            "core-G5-notify-order",
            "a @notify line leaves after the reply of the queued query before it",
            vec![
                Step::Say(Who::Controller, "@notify:{ch}=add"),
                Step::Say(Who::Controller, "@getgroup={ch},fly=n,versionnum={ch}"),
                Step::Expect(
                    "the group, then /fly=n, then 3040300",
                    test(|_, c| {
                        let n = c.replies.len();
                        if n >= 3 && c.replies[n - 1] == "3040300" && c.replies[n - 2] == "/fly=n" && c.replies[n - 3] != "3040300" {
                            Ok(())
                        } else {
                            Err(format!("{:?}", c.replies))
                        }
                    }),
                ),
            ],
        ),
        case(
            "core-G6-getcommand-duplicates",
            "@getcommand lists a name once per type it has (RlvBehaviourDictionary::getCommands)",
            vec![
                Step::Say(Who::Controller, "@getcommand:fly={ch}"),
                Step::Expect("fly;fly", last_is("fly;fly")),
            ],
        ),
        case(
            "core-G6-getcommand-separator",
            "@getcommand takes an empty separator and keeps the dictionary order",
            vec![
                Step::Say(Who::Controller, "@getcommand:sit;any;;={ch}"),
                Step::Expect(
                    "sitsittpunsitsitsitgroundunsitgetsitid",
                    last_is("sitsittpunsitsitsitgroundunsitgetsitid"),
                ),
            ],
        ),
        case(
            "core-G6-getcommand-no-extensions",
            "@getcommand leaves out the @getdebug_ names",
            vec![
                Step::Say(Who::Controller, "@getcommand:getdebug_={ch}"),
                Step::Expect("an empty reply", last_is("")),
            ],
        ),
        case(
            "core-G9-notify-unknown-name",
            "an unknown command is still told to @notify",
            vec![
                Step::Say(Who::Controller, "@notify:{ch}=add"),
                Step::Say(Who::Controller, "@foo1=n"),
                Step::Expect("/foo1=n", last_is("/foo1=n")),
            ],
        ),
        case(
            "core-G10-uuid-forms",
            "a key without its dashes is not an exception (LLUUID::set)",
            vec![
                Step::Say(Who::Controller, "@recvchatfrom:0123456789abcdef0123456789abcdef=n"),
                Step::Say(Who::Controller, "@getstatus:recvchatfrom={ch}"),
                Step::Expect("an empty status", last_is("")),
            ],
        ),
    ]
}
