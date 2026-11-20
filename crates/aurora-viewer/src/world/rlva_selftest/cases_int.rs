//! Phase 1 cases of the interaction with objects: rules issued through the
//! controller, read from the send boundary and the world's own predicates
//! on the worn controller; nothing restricted is sent.
use super::cases::{can, holds};
use super::{Case, Ctx, It, Step, Who, World};
use aurora_net::NetCommand;
use aurora_net::build::{BuildCmd, NewPrim, TaskItem};
use aurora_net::scripts::{CompileTarget, ScriptOptions, ScriptTarget};
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

fn controller(w: &World, c: &Ctx) -> Option<usize> {
    w.objects.index_of_uuid(&c.controller)
}

/// A command about the controller, if it is in view.
fn on_controller(f: fn(&World, &Ctx, usize) -> NetCommand, allowed: bool) -> impl Fn(&World, &Ctx) -> bool + 'static {
    move |w, c| controller(w, c).is_some_and(|i| can(w, &f(w, c, i)) == allowed)
}

/// A save of a script of the controller (LLLiveLSLEditor::saveIfNeeded).
fn save(_: &World, c: &Ctx, _: usize) -> NetCommand {
    NetCommand::SaveScript {
        request: Uuid::nil(),
        target: ScriptTarget {
            object: c.controller,
            item: Uuid::nil(),
        },
        handle: c.region,
        options: ScriptOptions {
            compile: CompileTarget::Mono,
            running: true,
            experience: Uuid::nil(),
        },
        source: String::new(),
    }
}

/// A texture dropped on the controller (dad3dApplyToObject).
fn texture(w: &World, _: &Ctx, i: usize) -> NetCommand {
    let o = w.objects.get(i);
    NetCommand::Build(BuildCmd::SetTextures {
        handle: o.map(|o| o.key.region).unwrap_or_default(),
        local_id: o.map_or(0, |o| o.key.local_id),
        media_url: String::new(),
        texture_entry: Vec::new(),
    })
}

/// One of the test's items dropped into the controller (dad3dRezScript).
fn script_drop(w: &World, c: &Ctx, i: usize) -> NetCommand {
    let o = w.objects.get(i);
    NetCommand::Build(BuildCmd::RezScript {
        handle: o.map(|o| o.key.region).unwrap_or_default(),
        local_id: o.map_or(0, |o| o.key.local_id),
        enabled: true,
        group_id: Uuid::nil(),
        item: Box::new(TaskItem {
            item_id: c.item(It::A),
            parent_id: c.controller,
            ..Default::default()
        }),
    })
}

/// The Reset button of an open script of the controller.
fn reset(w: &World, c: &Ctx, i: usize) -> NetCommand {
    NetCommand::Build(BuildCmd::ScriptReset {
        handle: w.objects.get(i).map(|o| o.key.region).unwrap_or_default(),
        object_id: c.controller,
        item_id: Uuid::nil(),
    })
}

/// A box the chat bar's platform would ask for (cmdline_rezplat).
fn platform(w: &World) -> NetCommand {
    NetCommand::Build(BuildCmd::Add(Box::new(NewPrim {
        handle: w.main_region.unwrap_or_default(),
        group_id: w.groups.active,
        pcode: aurora_prim::params::LL_PCODE_VOLUME,
        material: 3,
        add_flags: 0,
        shape: crate::build::shapes::shapes()[0].raw(),
        ray_start: w.agent.position,
        ray_end: w.agent.position,
        ray_target: Uuid::nil(),
        ray_end_is_intersection: false,
        bypass_raycast: true,
        scale: glam::Vec3::splat(0.5),
        rotation: glam::Quat::IDENTITY,
        state: 0,
    })))
}

pub(super) fn cases() -> Vec<Case> {
    vec![
        case(
            "int-G1-save-locked",
            "@remattach keeps the scripts of a locked attachment from being saved",
            vec![
                Step::Now(
                    "saving a script of the controller is allowed",
                    holds(on_controller(save, true), "refused"),
                ),
                Step::Say(Who::Controller, "@remattach=n"),
                Step::Expect(
                    "saving into the locked controller refused",
                    holds(on_controller(save, false), "still allowed"),
                ),
                Step::Now(
                    "a script of the inventory still saves",
                    holds(|w, _| w.rlva_can_save_script(Uuid::nil()), "refused"),
                ),
                Step::Say(Who::Controller, "@remattach=y"),
                Step::Expect("allowed again once released", holds(on_controller(save, true), "still refused")),
            ],
        ),
        case(
            "int-G4-drop-under-edit",
            "@edit leaves what is dropped onto an object, not onto a locked attachment",
            vec![
                Step::Say(Who::Controller, "@edit=n"),
                Step::Expect(
                    "the controller cannot be edited",
                    holds(|w, c| controller(w, c).is_some_and(|i| !w.rlva_can_edit(i)), "still editable"),
                ),
                Step::Now("a texture dropped onto it passes", holds(on_controller(texture, true), "refused")),
                Step::Now(
                    "a script dropped into it passes",
                    holds(on_controller(script_drop, true), "refused"),
                ),
                Step::Say(Who::Controller, "@remattach=n"),
                Step::Expect(
                    "the locked controller takes no drop",
                    holds(
                        |w, c| on_controller(texture, false)(w, c) && on_controller(script_drop, false)(w, c),
                        "a drop still passes",
                    ),
                ),
                Step::Say(Who::Controller, "@remattach=y,edit=y"),
                Step::Expect("released", holds(on_controller(texture, true), "still refused")),
            ],
        ),
        case(
            "int-G10-reset-under-edit",
            "@edit leaves an open script's Reset; a locked attachment keeps it",
            vec![
                Step::Say(Who::Controller, "@edit=n"),
                Step::Expect(
                    "Reset of the controller's script passes",
                    holds(on_controller(reset, true), "refused"),
                ),
                Step::Say(Who::Controller, "@remattach=n"),
                Step::Expect(
                    "refused on the locked controller",
                    holds(on_controller(reset, false), "still allowed"),
                ),
                Step::Say(Who::Controller, "@remattach=y,edit=y"),
                Step::Expect("released", holds(on_controller(reset, true), "still refused")),
            ],
        ),
        case(
            "int-G12-contents-under-edit",
            "@edit closes an object's contents window",
            vec![
                Step::Now(
                    "the controller's contents may stay open",
                    holds(|w, c| w.rlva_keeps_contents_open(c.controller), "closed"),
                ),
                Step::Say(Who::Controller, "@edit=n"),
                Step::Expect(
                    "closed under @edit",
                    holds(|w, c| !w.rlva_keeps_contents_open(c.controller), "kept open"),
                ),
                Step::Say(Who::Controller, "@edit=y"),
                Step::Expect("open again", holds(|w, c| w.rlva_keeps_contents_open(c.controller), "still closed")),
            ],
        ),
        case(
            "int-G13-delete-under-editattach",
            "@editattach hides an attachment's Delete and Return in the area search",
            vec![
                Step::Say(Who::Controller, "@editattach=n"),
                Step::Expect(
                    "the area search refuses the controller, the selection is not asked",
                    holds(
                        |w, c| controller(w, c).is_some_and(|i| !w.rlva_can_delete_or_return_object(i) && w.rlva_can_delete_or_return(i)),
                        "another answer",
                    ),
                ),
                Step::Say(Who::Controller, "@editattach=y"),
                Step::Expect(
                    "released",
                    holds(
                        |w, c| controller(w, c).is_some_and(|i| w.rlva_can_delete_or_return_object(i)),
                        "still refused",
                    ),
                ),
            ],
        ),
        case(
            "int-G5-touch-worn-under-interact",
            "@interact drops the picks but leaves the Touch of a worn object",
            vec![
                Step::Say(Who::Controller, "@interact=n"),
                Step::Expect("the picks are dropped", holds(|w, _| w.rlva_drops_picks(), "still picked")),
                Step::Now(
                    "touching the controller from its menu is allowed",
                    holds(
                        |w, c| {
                            controller(w, c)
                                .and_then(|i| w.objects.get(i))
                                .is_some_and(|o| w.rlva_can_touch(o.key.region, o.key.local_id))
                        },
                        "refused",
                    ),
                ),
                Step::Say(Who::Controller, "@interact=y"),
                Step::Expect("picked again", holds(|w, _| !w.rlva_drops_picks(), "still dropped")),
            ],
        ),
        case(
            "int-G6-add-under-interact",
            "@interact refuses the Create tool, not the chat bar's platform",
            vec![
                Step::Say(Who::Controller, "@interact=n"),
                Step::Expect("the Create tool refused", holds(|w, _| !w.rlva_can_rez(None), "still allowed")),
                Step::Now("the platform's new prim passes", holds(|w, _| can(w, &platform(w)), "refused")),
                Step::Say(Who::Controller, "@interact=y,rez=n"),
                Step::Expect("@rez refuses it", holds(|w, _| !can(w, &platform(w)), "still allowed")),
                Step::Say(Who::Controller, "@rez=y"),
                Step::Expect(
                    "released",
                    holds(|w, _| can(w, &platform(w)) && w.rlva_can_rez(None), "still refused"),
                ),
            ],
        ),
        case(
            "int-G9-title-under-setgroup",
            "@setgroup leaves the group title",
            vec![
                Step::Say(Who::Controller, "@setgroup=n"),
                Step::Expect(
                    "changing the group refused",
                    holds(|w, _| !w.rlva_can_change_group(), "still allowed"),
                ),
                Step::Now(
                    "the active group's title still changes",
                    holds(
                        |w, _| {
                            can(
                                w,
                                &NetCommand::GroupTitleUpdate {
                                    group: w.groups.active,
                                    role: Uuid::nil(),
                                },
                            )
                        },
                        "refused",
                    ),
                ),
                Step::Say(Who::Controller, "@setgroup=y"),
                Step::Expect("released", holds(|w, _| w.rlva_can_change_group(), "still refused")),
            ],
        ),
    ]
}
