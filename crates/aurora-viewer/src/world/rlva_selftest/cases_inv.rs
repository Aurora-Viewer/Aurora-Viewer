//! Phase 1 cases of the shared inventory and wear parity fixes: rules set
//! through the controller, checked on the predicates of the send boundary
//! and on the replies, nothing restricted being sent.
use super::super::rlva::POINTS;
use super::cases::{holds, skip_if, test};
use super::{Case, Ctx, It, POINT_A, POINT_CONTROLLER, Step, Test, Who, World};
use aurora_llsd::llsd_map;
use aurora_net::inventory::operations::{Mutation, Operation};
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

/// The last reply on the test's channel, checked by `f`.
fn answers(f: impl Fn(&World, &str) -> bool + 'static) -> Test {
    test(move |w, c| {
        let text = c.replies.last().map(String::as_str).ok_or_else(|| "no reply".to_owned())?;
        if f(w, text) { Ok(()) } else { Err(format!("\"{text}\"")) }
    })
}

fn foreign(w: &World, c: &Ctx) -> Option<String> {
    c.foreign_rules(w).then(|| "a restriction of another object is set".to_owned())
}

fn rules(w: &World) -> Vec<String> {
    w.rlva.restrictions().into_iter().map(|(_, rule)| rule).collect()
}

fn has_rule(w: &World, rule: &str) -> bool {
    rules(w).iter().any(|r| r.eq_ignore_ascii_case(rule))
}

/// Renaming `#RLV` to its own name: nothing would change, the boundary is
/// only asked.
fn rename_rlv(c: &Ctx) -> Mutation {
    Mutation {
        operations: vec![Operation::Patch {
            id: c.shared,
            folder: true,
            body: llsd_map! { "name" => "#RLV" },
        }],
        ..Default::default()
    }
}

pub(super) fn phase1() -> Vec<Case> {
    let nil = Uuid::nil();
    vec![
        // RlvAttachPtLookup::initLookupTable (rlvlocks.cpp): the avatar_lad
        // names, lowercased, are the point options
        case(
            "inv-G4-hind-foot-names",
            "points 54 and 55 go by their avatar_lad names",
            vec![
                skip_if(|w, c| (c.point(w, It::A) != POINT_A).then(|| "object A is not on Left Hind Foot".to_owned())),
                Step::Now(
                    "the names of the points are the avatar's",
                    holds(
                        |w, _| {
                            [POINT_A, POINT_CONTROLLER].iter().all(|p| {
                                w.avatar_lib
                                    .attach_names
                                    .get(p)
                                    .is_some_and(|name| name.to_lowercase() == POINTS[usize::from(*p)])
                            })
                        },
                        "a point name differs",
                    ),
                ),
                Step::Say(Who::Controller, "@getattach:left hind foot={ch}"),
                Step::Expect("object A counted on left hind foot", answers(|_, text| text == "1")),
                Step::Say(Who::Controller, "@remattach:hind left foot=n"),
                Step::Now(
                    "the old name is no point",
                    holds(|w, _| !has_rule(w, "remattach:hind left foot"), "the rule was kept"),
                ),
                Step::Say(Who::Controller, "@remattach:left hind foot=n"),
                Step::Expect(
                    "taking object A off refused",
                    holds(move |w, c| !w.rlva_can_remove(c.item(It::A), nil), "still allowed"),
                ),
                Step::Say(Who::Controller, "@remattach:left hind foot=y"),
                Step::Expect(
                    "allowed again once released",
                    holds(move |w, c| w.rlva_can_remove(c.item(It::A), nil), "still refused"),
                ),
            ],
        ),
        // rlvAttachGroupFromIndex (rlvhelper.cpp): "hud" is avatar_lad
        // group 8, the real HUD points are in no group
        case(
            "inv-G11-hud-group",
            "@getaddattachnames:hud lists avatar_lad group 8",
            vec![
                skip_if(foreign),
                Step::Say(Who::Controller, "@getaddattachnames:hud={ch}"),
                Step::Expect(
                    "tail, wings, rings and hind feet, no HUD",
                    answers(|w, text| {
                        let mut listed: Vec<&str> = text.split(',').filter(|n| !n.is_empty()).collect();
                        listed.sort_unstable();
                        let mut expected: Vec<&str> = (1..POINTS.len() as u8)
                            .filter(|p| w.avatar_lib.attach_points.get(p).is_some_and(|at| at.group == 8))
                            .filter_map(|p| w.avatar_lib.attach_names.get(&p).map(String::as_str))
                            .collect();
                        expected.sort_unstable();
                        !expected.is_empty() && listed == expected
                    }),
                ),
            ],
        ),
        // RlvHandler::onAddRemFolderLock (rlvhandler.cpp): a UUID, a path
        // that is not there, or an exception on a layer is refused
        case(
            "inv-G14-folder-lock-options",
            "folder locks refuse a UUID, a missing path and a layer exception",
            vec![
                Step::Say(Who::Controller, "@detachallthis:{other}=n"),
                Step::Say(Who::Controller, "@detachallthis:{f}/~nowhere=n"),
                Step::Say(Who::Controller, "@detachallthis_except:{layer}=n"),
                Step::Say(Who::Controller, "@detachallthis:{f}/free=n"),
                Step::Expect(
                    "the lock on an existing folder is kept",
                    holds(|w, _| has_rule(w, "detachallthis:~auroraselftest/free"), "not kept"),
                ),
                Step::Now(
                    "the others are refused",
                    test(|w, c| {
                        let other = format!("detachallthis:{}", c.other);
                        let refused = [other.as_str(), "detachallthis:~auroraselftest/~nowhere"];
                        let kept: Vec<String> = rules(w)
                            .into_iter()
                            .filter(|r| refused.iter().any(|x| r.eq_ignore_ascii_case(x)) || r.starts_with("detachallthis_except"))
                            .collect();
                        if kept.is_empty() { Ok(()) } else { Err(format!("{kept:?}")) }
                    }),
                ),
            ],
        ),
        // RlvFolderLocks::canMoveItem (rlvlocks.h): the shared path lock of
        // @sharedwear keeps the items of #RLV in it
        case(
            "inv-G1-shared-lock-keeps-items",
            "@sharedwear keeps an item in #RLV",
            vec![
                skip_if(foreign),
                Step::Now(
                    "moving a test item out of #RLV allowed before the rule",
                    holds(
                        move |w, c| w.rlva_inventory_move_allowed(c.item(It::Plain), w.inventory.root, nil),
                        "already refused",
                    ),
                ),
                Step::Say(Who::Controller, "@sharedwear=n"),
                Step::Expect(
                    "moving it out of #RLV refused",
                    holds(
                        move |w, c| !w.rlva_inventory_move_allowed(c.item(It::Plain), w.inventory.root, nil),
                        "still allowed",
                    ),
                ),
                Step::Say(Who::Controller, "@sharedwear=y"),
                Step::Expect(
                    "allowed again once released",
                    holds(
                        move |w, c| w.rlva_inventory_move_allowed(c.item(It::Plain), w.inventory.root, nil),
                        "still refused",
                    ),
                ),
            ],
        ),
        // RlvFolderLocks::canRenameFolder (rlvlocks.h): #RLV keeps its name
        // under the placement locks
        case(
            "inv-G2-rlv-keeps-its-name",
            "@sharedunwear keeps the name of #RLV",
            vec![
                skip_if(foreign),
                Step::Now(
                    "renaming #RLV allowed before the rule",
                    holds(
                        move |w, c| w.rlva_inventory_mutation_allowed(&rename_rlv(c), nil),
                        "already refused",
                    ),
                ),
                Step::Say(Who::Controller, "@sharedunwear=n"),
                Step::Expect(
                    "renaming #RLV refused",
                    holds(move |w, c| !w.rlva_inventory_mutation_allowed(&rename_rlv(c), nil), "still allowed"),
                ),
                Step::Say(Who::Controller, "@sharedunwear=y"),
                Step::Expect(
                    "allowed again once released",
                    holds(move |w, c| w.rlva_inventory_mutation_allowed(&rename_rlv(c), nil), "still refused"),
                ),
            ],
        ),
    ]
}
