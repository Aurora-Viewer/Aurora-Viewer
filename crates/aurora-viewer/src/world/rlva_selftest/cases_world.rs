//! Phase 2, the human's explicit choice: what needs an object in-world.
//! The run rezzes at most two small plain prims next to the avatar, sits the
//! avatar on one, takes off and lands once, and removes its prims. Every
//! command names those prims only (`guard`), and a command a restriction is
//! expected to refuse is sent only when the boundary's own rule refuses it:
//! nothing destructive is ever "tried" on an object.
use super::cases::{can, holds, make, skip_if, test};
use super::{Case, Cmd, Ctx, Make, Prim, SEAT_NAME, Step, Who, World};
use crate::build::FLAGS_CREATE_SELECTED;
use crate::ui::context::flags::OBJECT_YOU_OWNER;
use crate::world::land::BuildRight;
use aurora_net::build::{BuildCmd, NewPrim, TransformUpdate, derez};
use aurora_net::{NetCommand, TouchSurface};
use glam::{Quat, Vec3};
use std::time::Duration;
use uuid::Uuid;

/// The side of the prims, and how far in front of the avatar they go.
const SIZE: f32 = 0.5;
const AHEAD: f32 = 1.5;

/// A plain default box of 0.5 m, resting on `ground`, as the Create tool
/// asks for one (LLToolPlacer::addObject), created selected so that the
/// simulator's answer names the creating client.
pub(super) fn new_prim(w: &World, ground: Vec3) -> NewPrim {
    NewPrim {
        handle: w.main_region.unwrap_or_default(),
        group_id: w.groups.active,
        pcode: aurora_prim::params::LL_PCODE_VOLUME,
        material: 3,
        add_flags: FLAGS_CREATE_SELECTED,
        shape: crate::build::shapes::shapes()[0].raw(),
        ray_start: w.agent.position,
        ray_end: ground,
        ray_target: Uuid::nil(),
        ray_end_is_intersection: false,
        bypass_raycast: true,
        scale: Vec3::splat(SIZE),
        rotation: Quat::IDENTITY,
        state: 0,
    }
}

/// Where the two prims go: in front of the avatar, at its feet, side by side.
fn spots(w: &World) -> [Vec3; 2] {
    let forward = w.agent.forward();
    let forward = Vec3::new(forward.x, forward.y, 0.0).try_normalize().unwrap_or(Vec3::X);
    let side = Vec3::new(-forward.y, forward.x, 0.0);
    let half_height = w
        .objects
        .index_of_uuid(&w.agent_id)
        .and_then(|i| w.objects.get(i))
        .map_or(0.9, |o| 0.5 * o.scale.z);
    let feet = w.agent.position - Vec3::Z * half_height;
    [feet + forward * AHEAD, feet + forward * AHEAD + side * 0.8]
}

fn flagged(w: &World) -> std::collections::HashSet<Uuid> {
    w.objects
        .iter()
        .filter(|(_, o)| o.update_flags & FLAGS_CREATE_SELECTED != 0)
        .map(|(_, o)| o.full_id)
        .collect()
}

/// The object the simulator created for the run's own request: a root flagged
/// "created selected" (addressed to the creating client, as
/// `BuildTool::expect_created` reads it) that was not there before, owned by
/// the agent, where it was asked for.
fn created(w: &World, c: &Ctx, at: Vec3) -> Option<Prim> {
    w.objects
        .iter()
        .filter(|(_, o)| {
            o.parent_id == 0
                && !o.is_avatar()
                && o.update_flags & FLAGS_CREATE_SELECTED != 0
                && o.update_flags & OBJECT_YOU_OWNER != 0
                && !c.flagged.contains(&o.full_id)
                && !c.prims.iter().any(|p| p.id == o.full_id)
                && o.position.distance(at) < 1.5
        })
        .min_by(|(_, a), (_, b)| a.position.distance(at).total_cmp(&b.position.distance(at)))
        .map(|(_, o)| Prim {
            id: o.full_id,
            handle: o.key.region,
            local_id: o.key.local_id,
        })
}

/// A prim asked for just before the run ended may come back after it: the
/// cleanup counts it among the run's own, to remove it with the others.
pub(super) fn adopt(w: &World, c: &mut Ctx) {
    for spot in c.spots {
        if c.prims.len() >= usize::from(c.adds) {
            break;
        }
        if let Some(found) = created(w, c, spot) {
            c.prims.push(found);
        }
    }
}

fn index(w: &World, c: &Ctx, prim: usize) -> Option<usize> {
    c.prims.get(prim).and_then(|p| w.objects.index_of_uuid(&p.id))
}

/// What a predicate says of the seat prim; false when it is gone.
fn seat(f: impl Fn(&World, usize) -> bool + 'static) -> impl Fn(&World, &Ctx) -> bool {
    move |w, c| index(w, c, 0).is_some_and(|i| f(w, i))
}

/// A build command on the test's prims (`both`: the two of them).
fn build(both: bool, f: impl Fn(&World, &Ctx, aurora_net::RegionHandle, Vec<u32>) -> BuildCmd + 'static) -> Make {
    make(move |w, c| {
        let prims: Vec<&Prim> = c.prims.iter().take(if both { 2 } else { 1 }).collect();
        match prims.first() {
            Some(first) if !both || prims.len() == 2 => {
                Cmd::send(NetCommand::Build(f(w, c, first.handle, prims.iter().map(|p| p.local_id).collect())))
            }
            _ => Cmd::Missing("the test's prim"),
        }
    })
}

fn derez_to(destination: u8) -> Make {
    build(false, move |w, _, handle, local_ids| BuildCmd::DeRez {
        handle,
        local_ids,
        destination,
        destination_id: crate::world::inventory::actions::system(&w.inventory, 14).unwrap_or_default(),
        group_id: w.groups.active,
    })
}

fn transform(change: fn(&mut TransformUpdate, Vec3)) -> Make {
    build(false, move |w, c, handle, local_ids| {
        let at = index(w, c, 0).and_then(|i| w.objects.get(i)).map_or(Vec3::ZERO, |o| o.position);
        let mut update = TransformUpdate {
            local_id: local_ids[0],
            position: None,
            rotation: None,
            scale: None,
            linked: true,
            uniform: false,
        };
        change(&mut update, at);
        BuildCmd::Transform {
            handle,
            updates: vec![update],
        }
    })
}

fn move_it() -> Make {
    transform(|u, at| u.position = Some(at + Vec3::X * 0.1))
}

fn rename() -> Make {
    build(false, |_, _, handle, local_ids| BuildCmd::SetName {
        handle,
        local_ids,
        name: SEAT_NAME.to_owned(),
    })
}

fn add_item() -> Make {
    make(|w, c| match c.prims.first() {
        Some(p) => Cmd::send(super::factory::relay_item(w, c, p.handle, p.local_id, p.id)),
        None => Cmd::Missing("the test's prim"),
    })
}

fn sit_request() -> Make {
    make(|_, c| match c.prims.first() {
        Some(p) => Cmd::send(NetCommand::RequestSit {
            handle: p.handle,
            target: p.id,
            offset: Vec3::ZERO,
        }),
        None => Cmd::Missing("the test's prim"),
    })
}

fn stand_up() -> Make {
    make(|_, _| Cmd::send(NetCommand::OneShotControl(aurora_net::control::STAND_UP)))
}

fn grab() -> Make {
    make(|_, c| match c.prims.first() {
        Some(p) => Cmd::send(NetCommand::ObjectGrab {
            handle: p.handle,
            local_id: p.local_id,
            offset: Vec3::ZERO,
            surface: TouchSurface::default(),
        }),
        None => Cmd::Missing("the test's prim"),
    })
}

fn seated_on_seat(w: &World, c: &Ctx) -> bool {
    w.agent.seated && c.prims.first().is_some_and(|p| p.id == w.rlva_sit_id())
}

fn touchable(w: &World, c: &Ctx) -> bool {
    c.prims.first().is_some_and(|p| w.rlva_can_touch(p.handle, p.local_id))
}

fn case(id: &'static str, title: &'static str, steps: Vec<Step>) -> Case {
    Case {
        id,
        title,
        phase: 2,
        gate: false,
        known: None,
        steps,
    }
}

/// The task item of the seat's script, as a contents command names it.
fn script_item(c: &Ctx) -> Uuid {
    c.seat_script.unwrap_or_default()
}

fn rez(prim: usize, label: &'static str) -> [Step; 3] {
    [
        Step::Now(
            "the place is free",
            test(|w, c| {
                c.flagged = flagged(w);
                Ok(())
            }),
        ),
        Step::Sent(
            label,
            make(move |w, c| Cmd::send(NetCommand::Build(BuildCmd::Add(Box::new(new_prim(w, c.spots[prim])))))),
        ),
        Step::Expect(
            "the simulator answers with the new prim",
            test(move |w, c| match created(w, c, c.spots[prim]) {
                Some(found) => {
                    c.prims.push(found);
                    Ok(())
                }
                None => Err("the rez did not come back".to_owned()),
            }),
        ),
    ]
}

fn setup() -> Case {
    let mut steps = vec![skip_if(|w, c| {
        let handle = w.main_region.unwrap_or_default();
        let here = w.parcel.as_deref();
        if w.agent.seated {
            Some("the avatar is sitting".to_owned())
        } else if w.agent.flying {
            Some("the avatar is flying".to_owned())
        } else if c.speaker(w, Who::Controller).is_none() {
            Some("the controller is not worn".to_owned())
        } else if w.build_right_here() != BuildRight::Allowed {
            Some("the parcel does not let the agent build".to_owned())
        } else if spots(w).iter().any(|spot| {
            !w.known_parcel_at(handle, *spot, None)
                .is_some_and(|p| here.is_some_and(|h| std::ptr::eq(p, h)))
        }) {
            Some("the place in front of the avatar is on another parcel".to_owned())
        } else {
            None
        }
    })];
    steps.push(Step::Now(
        "a place in front of the avatar",
        test(|w, c| {
            c.spots = spots(w);
            Ok(())
        }),
    ));
    steps.extend(rez(0, "a first prim is created"));
    steps.extend(rez(1, "a second prim is created"));
    steps.extend([
        Step::Sent(
            "they are deselected",
            build(true, |_, _, handle, local_ids| BuildCmd::Deselect { handle, local_ids }),
        ),
        Step::Sent(
            "they are named",
            build(true, |_, _, handle, local_ids| BuildCmd::SetName {
                handle,
                local_ids,
                name: SEAT_NAME.to_owned(),
            }),
        ),
        Step::InstallSeat,
        Step::Hello(Who::Seat),
    ]);
    Case {
        gate: true,
        ..case("W00", "two small prims next to the avatar", steps)
    }
}

/// A rule set, what it refuses on the prim, then released.
fn refusing(set: &'static str, clear: &'static str, allowed: fn(&World, &Ctx) -> bool, probes: Vec<(&'static str, Make)>) -> Vec<Step> {
    let mut steps = vec![
        Step::Say(Who::Controller, set),
        Step::Expect("refused under the rule", holds(move |w, c| !allowed(w, c), "still allowed")),
    ];
    steps.extend(probes.into_iter().map(|(label, command)| Step::Refused(label, command)));
    steps.push(Step::Say(Who::Controller, clear));
    steps.push(Step::Expect("allowed again once released", holds(allowed, "still refused")));
    steps
}

pub(super) fn phase2() -> Vec<Case> {
    let editable: fn(&World, &Ctx) -> bool = |w, c| index(w, c, 0).is_some_and(|i| w.rlva_can_edit(i));
    let deletable = seat(|w, i| w.rlva_can_delete_or_return(i));
    let mut edit = Vec::new();
    for (set, clear) in [
        ("@edit=n", "@edit=y"),
        ("@editobj:{seat}=n", "@editobj:{seat}=y"),
        ("@editworld=n", "@editworld=y"),
    ] {
        edit.extend(refusing(
            set,
            clear,
            editable,
            vec![
                ("moving its prim is refused", move_it()),
                ("renaming its prim is refused", rename()),
            ],
        ));
    }
    edit.extend([
        Step::Say(Who::Controller, "@edit=n,rez=n"),
        Step::Expect("the build tools closed", holds(|w, _| !w.rlva_can_build(), "still open")),
        Step::Say(Who::Controller, "@edit=y,rez=y"),
        Step::Expect("the build tools open again", holds(|w, _| w.rlva_can_build(), "still closed")),
        Step::Sent("renaming its prim to the same name leaves again", rename()),
    ]);
    let mut touch = Vec::new();
    for (set, clear) in [
        ("@touchall=n", "@touchall=y"),
        ("@touchworld=n", "@touchworld=y"),
        ("@touchthis:{seat}=n", "@touchthis:{seat}=y"),
        ("@fartouch:0.1=n", "@fartouch:0.1=y"),
    ] {
        touch.extend(refusing(set, clear, touchable, vec![("touching its prim is refused", grab())]));
    }
    let seat_lock = vec![
        Step::Sent(
            "its two prims are linked",
            build(true, |_, _, handle, local_ids| BuildCmd::Link { handle, local_ids }),
        ),
        Step::Expect(
            "one linkset",
            holds(
                |w, c| index(w, c, 1).is_some_and(|i| Some(w.objects.linkset_root(i)) == index(w, c, 0)),
                "not linked",
            ),
        ),
        // the one sit of the run, on its own prim and nothing else
        Step::Now(
            "its prim is still there and its own",
            holds(
                |w, c| c.prims.first().is_some_and(|p| super::guard::still_own_prim(w, c, p)),
                "its prim is gone",
            ),
        ),
        // RlvHandler::setSitSource (rlvhandler.cpp): a sit asked under
        // @standtp keeps where the avatar stood. The rule is released before
        // the avatar stands, so that the run never teleports it back: the
        // place stays kept (RlvHandler::onSitOrStand uses it only while
        // @standtp is held, and clears it then).
        Step::Say(Who::Controller, "@standtp=n"),
        Step::Expect(
            "@standtp is held",
            test(|w, c| {
                if !w.rlva.has("standtp") {
                    return Err("not held".to_owned());
                }
                c.stood = w.main_region.map(|handle| (handle, w.agent.position));
                Ok(())
            }),
        ),
        Step::Say(Who::Controller, "@sit:{seat}=force"),
        Step::Expect(
            "the avatar sits on its prim",
            test(|w, c| {
                if seated_on_seat(w, c) {
                    Ok(())
                } else if w.agent.seated {
                    Err("seated elsewhere".to_owned())
                } else {
                    Err("not seated".to_owned())
                }
            }),
        ),
        Step::Now(
            "@standtp kept the place the avatar stood (#2738)",
            test(|w, c| match (w.rlva.stand_origin, c.stood) {
                (Some((region, at)), Some((stood_region, stood))) if region == stood_region && at.distance(stood) <= 1.0 => Ok(()),
                (Some(_), _) => Err("another place is kept".to_owned()),
                (None, _) => Err("no place is kept".to_owned()),
            }),
        ),
        Step::Say(Who::Controller, "@standtp=y"),
        Step::Expect(
            "the place stays kept once @standtp is released, but no rule asks for it",
            holds(
                |w, c| w.rlva.stand_origin.is_some() && (!w.rlva.has("standtp") || c.foreign_rules(w)),
                "the place is lost, or @standtp is still held",
            ),
        ),
        Step::Say(Who::Controller, "@unsit=n"),
        Step::Expect("the seat is locked", holds(seat(|w, i| w.rlva_locked_seat(i)), "not locked")),
        Step::Refused("standing up is refused", stand_up()),
        Step::Pause(Duration::from_millis(500)),
        Step::Now("still seated", holds(seated_on_seat, "the avatar stood up")),
        Step::Now(
            "delete, unlink, contents and derender refuse the seat",
            holds(
                seat(|w, i| {
                    !w.rlva_can_delete_or_return(i)
                        && !w.rlva_can_unlink(i)
                        && !w.rlva_can_take_copy(i)
                        && w.rlva_restrained_seat(i)
                        && w.rlva_contents_locked(i)
                        && w.rlva_locked_seat(i)
                }),
                "a predicate lets the seat go",
            ),
        ),
        Step::Refused("deleting the seat is refused", derez_to(derez::TRASH)),
        Step::Refused("taking the seat is refused", derez_to(derez::TAKE_INTO_AGENT_INVENTORY)),
        Step::Refused("taking a copy of the seat is refused", derez_to(derez::ACQUIRE_TO_AGENT_INVENTORY)),
        Step::Refused("returning the seat is refused", derez_to(derez::RETURN_TO_OWNER)),
        Step::Refused(
            "duplicating the seat is refused",
            build(false, |w, _, handle, local_ids| BuildCmd::Duplicate {
                handle,
                local_ids,
                offset: Vec3::X * 0.5,
                flags: FLAGS_CREATE_SELECTED,
                group_id: w.groups.active,
            }),
        ),
        Step::Refused("moving the seat is refused", move_it()),
        Step::Refused(
            "rotating the seat is refused",
            transform(|u, _| u.rotation = Some(Quat::from_rotation_z(0.3))),
        ),
        Step::Refused("stretching the seat is refused", transform(|u, _| u.scale = Some(Vec3::splat(0.4)))),
        Step::Refused(
            "unlinking the seat is refused",
            build(true, |_, _, handle, local_ids| BuildCmd::Delink { handle, local_ids }),
        ),
        Step::Refused("adding to the seat's contents is refused", add_item()),
        Step::Refused(
            "removing from the seat's contents is refused",
            build(false, |_, c, handle, local_ids| BuildCmd::RemoveTaskInventory {
                handle,
                local_id: local_ids[0],
                item_id: script_item(c),
            }),
        ),
        // LLTaskInvFVBridge::isItemRenameable (llpanelobjectinventory.cpp)
        // keeps the names in a locked attachment only: an item of the seat
        // is renamed. Asked of the rule, never sent.
        Step::Now(
            "renaming in the seat's contents stays allowed, as in the Legacy Viewer",
            holds(
                |w, c| {
                    c.prims.first().is_some_and(|p| {
                        can(
                            w,
                            &NetCommand::Build(BuildCmd::UpdateTaskInventory {
                                handle: p.handle,
                                local_id: p.local_id,
                                item: Box::new(aurora_net::build::TaskItem {
                                    item_id: script_item(c),
                                    parent_id: p.id,
                                    owner_id: c.agent,
                                    asset_type: 10,
                                    inv_type: 10,
                                    name: "renamed".to_owned(),
                                    ..Default::default()
                                }),
                            }),
                        )
                    })
                },
                "refused",
            ),
        ),
        // handle_selected_script_action (llviewermenu.cpp) only keeps the
        // scripts of a locked attachment: those of the seat can be reset and
        // stopped. Asked of the rule, never sent.
        Step::Now(
            "resetting and stopping the seat's scripts stay allowed, as in the Legacy Viewer",
            holds(
                |w, c| {
                    c.prims.first().is_some_and(|p| {
                        can(
                            w,
                            &NetCommand::Build(BuildCmd::ScriptReset {
                                handle: p.handle,
                                object_id: p.id,
                                item_id: script_item(c),
                            }),
                        ) && can(
                            w,
                            &NetCommand::Build(BuildCmd::SetScriptRunning {
                                handle: p.handle,
                                object_id: p.id,
                                item_id: script_item(c),
                                running: false,
                            }),
                        )
                    })
                },
                "refused",
            ),
        ),
        Step::Say(Who::Controller, "@unsit=y"),
        Step::Expect("the seat is released", holds(seat(|w, i| !w.rlva_locked_seat(i)), "still locked")),
        Step::Say(Who::Controller, "@sittp=n"),
        Step::Expect(
            "@sittp restrains the seat too",
            holds(
                seat(|w, i| w.rlva_restrained_seat(i) && w.rlva_contents_locked(i)),
                "not restrained",
            ),
        ),
        Step::Refused("moving the seat is refused under @sittp", move_it()),
        Step::Refused("adding to the seat's contents is refused under @sittp", add_item()),
        Step::Say(Who::Controller, "@sittp=y"),
        Step::Expect(
            "the seat is free",
            holds(seat(|w, i| !w.rlva_restrained_seat(i)), "still restrained"),
        ),
        Step::Now(
            "standing up is allowed again",
            holds(
                |w, _| can(w, &NetCommand::OneShotControl(aurora_net::control::STAND_UP)),
                "still refused",
            ),
        ),
        Step::Say(Who::Controller, "@unsit=force"),
        Step::Expect("the avatar stands", holds(|w, _| !w.agent.seated, "still seated")),
    ];
    vec![
        setup(),
        case(
            "W03",
            "@rez and @interact refuse a new prim; @rez keeps its own prim",
            vec![
                Step::Say(Who::Controller, "@rez=n"),
                Step::Expect("rezzing refused", holds(|w, _| !w.rlva_can_rez(None), "still allowed")),
                Step::Now(
                    "deleting its own prim is refused (it could not be rezzed again)",
                    holds(move |w, c| !deletable(w, c), "allowed"),
                ),
                Step::Now(
                    "the place is watched",
                    test(|w, c| {
                        c.flagged = flagged(w);
                        Ok(())
                    }),
                ),
                Step::Refused(
                    "creating a prim is refused",
                    make(|w, c| Cmd::send(NetCommand::Build(BuildCmd::Add(Box::new(new_prim(w, c.spots[0] + Vec3::Z)))))),
                ),
                Step::Pause(Duration::from_millis(1500)),
                Step::Now(
                    "no prim appeared",
                    holds(|w, c| created(w, c, c.spots[0] + Vec3::Z).is_none(), "a prim was created"),
                ),
                Step::Say(Who::Controller, "@rez=y"),
                Step::Expect(
                    "rezzing and deleting allowed again",
                    holds(
                        |w, c| w.rlva_can_rez(None) && index(w, c, 0).is_some_and(|i| w.rlva_can_delete_or_return(i)),
                        "still refused",
                    ),
                ),
                Step::Say(Who::Controller, "@interact=n"),
                Step::Expect(
                    "rezzing refused under @interact",
                    holds(|w, _| !w.rlva_can_rez(None), "still allowed"),
                ),
                Step::Refused(
                    "creating a prim is refused under @interact",
                    make(|w, c| Cmd::send(NetCommand::Build(BuildCmd::Add(Box::new(new_prim(w, c.spots[0] + Vec3::Z)))))),
                ),
                Step::Say(Who::Controller, "@interact=y"),
                Step::Expect("allowed again once released", holds(|w, _| w.rlva_can_rez(None), "still refused")),
            ],
        ),
        case("W04", "@edit, @editobj and @editworld keep its prim as it is", edit),
        case("W05", "@touchall, @touchworld, @touchthis and @fartouch refuse a touch", touch),
        case(
            "W05b",
            "@touchme from the prim itself",
            vec![
                skip_if(|w, c| {
                    c.speaker(w, Who::Seat)
                        .is_none()
                        .then(|| "no relay script could be put in the prim".to_owned())
                }),
                Step::Say(Who::Controller, "@touchall=n"),
                Step::Expect("touching its prim refused", holds(|w, c| !touchable(w, c), "still allowed")),
                Step::Say(Who::Seat, "@touchme=add"),
                Step::Expect("the prim that asked can be touched", holds(touchable, "still refused")),
                Step::Say(Who::Seat, "@touchme=rem"),
                Step::Expect("refused again", holds(|w, c| !touchable(w, c), "still allowed")),
            ],
        ),
        case(
            "W07",
            "@fartouch limits the build selection",
            vec![
                Step::Now(
                    "its prim can be selected",
                    holds(seat(|w, i| w.rlva_within_fartouch(i)), "out of reach"),
                ),
                Step::Say(Who::Controller, "@fartouch:0.1=n"),
                Step::Expect(
                    "too far to select",
                    holds(seat(|w, i| !w.rlva_within_fartouch(i)), "still in reach"),
                ),
                Step::Say(Who::Controller, "@fartouch:0.1=y"),
                Step::Expect(
                    "in reach again",
                    holds(seat(|w, i| w.rlva_within_fartouch(i)), "still out of reach"),
                ),
            ],
        ),
        case(
            "W06",
            "@fly lands a flight and refuses a take-off",
            vec![
                skip_if(|w, _| (!crate::autopilot::can_takeoff(w)).then(|| "flying is not allowed here".to_owned())),
                Step::Fly(true),
                Step::Expect("the avatar took off", holds(|w, _| w.agent.flying, "it did not take off")),
                Step::Say(Who::Controller, "@fly=n"),
                Step::Expect("the flight is landed", holds(|w, _| !w.agent.flying, "still flying")),
                Step::Now(
                    "taking off is refused",
                    holds(|w, _| !crate::autopilot::can_takeoff(w), "taking off is allowed"),
                ),
                Step::Say(Who::Controller, "@fly=y"),
                Step::Expect(
                    "taking off allowed again",
                    holds(|w, _| crate::autopilot::can_takeoff(w), "still refused"),
                ),
            ],
        ),
        case(
            "W02",
            "@sit refuses a sit while standing",
            vec![
                Step::Say(Who::Controller, "@sit=n"),
                Step::Refused("a sit on its prim is refused", sit_request()),
                Step::Pause(Duration::from_millis(500)),
                Step::Now("the avatar stands", holds(|w, _| !w.agent.seated, "it sat down")),
            ],
        ),
        case(
            "W01",
            "@sit by force, the seat under @unsit and @sittp (#3532), @unsit by force",
            seat_lock,
        ),
    ]
}
