//! RLVa parity of the interaction with objects: what the send boundary and
//! the world's rules refuse and leave, as the Legacy Viewer's RlvActions,
//! LLLiveLSLEditor, LLToolDragAndDrop and rlvCanDeleteOrReturn do.

use super::*;
use crate::world::World;
use aurora_net::build::{BuildCmd, TaskItem};
use aurora_net::scripts::{CompileTarget, ScriptOptions, ScriptTarget};

fn index(world: &World, id: Uuid) -> usize {
    world.objects.index_of_uuid(&id).unwrap()
}

/// The object worn for `item`.
fn worn(world: &World, item: Uuid) -> usize {
    world.worn_attachment(&item).expect("worn")
}

fn save(world: &World, object: Uuid) -> bool {
    world.rlva_allows_command(
        &NetCommand::SaveScript {
            request: Uuid::from_u128(0x7311),
            target: ScriptTarget {
                object,
                item: Uuid::from_u128(0x7312),
            },
            handle: world.main_region.unwrap(),
            options: ScriptOptions {
                compile: CompileTarget::Mono,
                running: true,
                experience: Uuid::nil(),
            },
            source: "default { state_entry() {} }".to_owned(),
        },
        Uuid::nil(),
    )
}

/// LLLiveLSLEditor::saveIfNeeded (llpreviewscript.cpp): nothing is saved
/// into a script of a locked attachment, whatever path sends it; the other
/// objects and the scripts of the inventory are not asked.
#[test]
fn rlva_saving_a_script_into_a_locked_attachment_is_refused() {
    let mut world = parity_demo_world();
    let chest = worn_attachment(&mut world, 0x7301, 1);
    let collar = world.objects.get(worn(&world, chest)).unwrap().full_id;
    let ground = parity_prim(&mut world, 0x7302, 0);
    assert!(save(&world, collar) && save(&world, ground) && save(&world, Uuid::nil()));
    command(&mut world.rlva, A, "@remattach:chest=n");
    assert!(world.rlva_locked_attachment(worn(&world, chest)));
    assert!(!save(&world, collar), "the locked attachment's script stays as it is");
    assert!(!world.rlva_can_save_script(collar));
    assert!(
        save(&world, ground) && save(&world, Uuid::nil()),
        "only the locked attachment is asked"
    );
    command(&mut world.rlva, A, "@remattach:chest=y");
    assert!(save(&world, collar));
    command(&mut world.rlva, A, "@remattach:chest=n");
    world.rlva_configure(false);
    assert!(save(&world, collar), "RLVa off");
}

fn texture(world: &World, i: usize) -> NetCommand {
    let key = world.objects.get(i).unwrap().key;
    NetCommand::Build(BuildCmd::SetTextures {
        handle: key.region,
        local_id: key.local_id,
        media_url: String::new(),
        texture_entry: Vec::new(),
    })
}

fn material(world: &World, i: usize) -> NetCommand {
    let o = world.objects.get(i).unwrap();
    NetCommand::Build(BuildCmd::Cap {
        handle: o.key.region,
        cap: "ModifyMaterialParams".to_owned(),
        put: false,
        body: aurora_llsd::Llsd::Array(vec![aurora_llsd::llsd_map! { "object_id" => o.full_id, "side" => 0 }]),
        tag: 0,
    })
}

/// An item of our inventory (`item`) dropped into `i`'s contents, or one
/// already inside changed (`item` not ours).
fn task_item(world: &World, i: usize, item: Uuid, script: bool) -> NetCommand {
    let o = world.objects.get(i).unwrap();
    let (handle, local_id) = (o.key.region, o.key.local_id);
    let item = Box::new(TaskItem {
        item_id: item,
        parent_id: o.full_id,
        ..Default::default()
    });
    NetCommand::Build(if script {
        BuildCmd::RezScript {
            handle,
            local_id,
            enabled: true,
            group_id: Uuid::nil(),
            item,
        }
    } else {
        BuildCmd::UpdateTaskInventory { handle, local_id, item }
    })
}

fn scripts(world: &World, i: usize) -> [NetCommand; 2] {
    let o = world.objects.get(i).unwrap();
    let (handle, object_id, item_id) = (o.key.region, o.full_id, Uuid::from_u128(0x7313));
    [
        NetCommand::Build(BuildCmd::ScriptReset {
            handle,
            object_id,
            item_id,
        }),
        NetCommand::Build(BuildCmd::SetScriptRunning {
            handle,
            object_id,
            item_id,
            running: false,
        }),
    ]
}

/// LLToolDragAndDrop::dad3dApplyToObject, dad3dUpdateInventory and
/// dad3dRezScript ask willObjectAcceptInventory alone: under @edit a
/// texture, a material, an item or a script still goes onto an object the
/// agent may not edit, never onto a locked attachment. LLLiveLSLEditor's
/// Reset and Running ask the attachment lock alone too.
#[test]
fn rlva_edit_restrictions_leave_inventory_drops_and_an_open_editors_reset() {
    let mut world = parity_demo_world();
    let chest = worn_attachment(&mut world, 0x7321, 1);
    let collar = worn(&world, chest);
    let ground = parity_prim(&mut world, 0x7322, 0);
    let ground = index(&world, ground);
    let allowed = |world: &World, command: &NetCommand| world.rlva_allows_command(command, Uuid::nil());
    // (`chest` is an item of our inventory: a drop; 0x7323 is not: a change)
    let drops = |world: &World, i: usize| {
        [
            texture(world, i),
            material(world, i),
            task_item(world, i, chest, false),
            task_item(world, i, chest, true),
        ]
    };
    for restriction in [
        "@edit=n",
        "@editworld=n",
        &format!("@editobj:{}=n", world.objects.get(ground).unwrap().full_id),
    ] {
        command(&mut world.rlva, A, restriction);
        assert!(!world.rlva_can_edit(ground), "{restriction}");
        assert!(
            drops(&world, ground).iter().all(|c| allowed(&world, c)),
            "{restriction}: a drop passes"
        );
        assert!(
            scripts(&world, ground).iter().all(|c| allowed(&world, c)),
            "{restriction}: Reset and Running pass"
        );
        assert!(
            !allowed(&world, &task_item(&world, ground, Uuid::from_u128(0x7323), false)),
            "{restriction}: an item already inside is not changed"
        );
        command(&mut world.rlva, A, "@clear");
    }
    // a locked attachment takes no drop, editable or not, and keeps its scripts
    command(&mut world.rlva, A, "@remattach:chest=n");
    assert!(world.rlva_can_edit(collar));
    assert!(allowed(&world, &texture(&world, collar)), "the build tools still edit it");
    assert!(!allowed(&world, &task_item(&world, collar, chest, true)));
    assert!(scripts(&world, collar).iter().all(|c| !allowed(&world, c)));
    command(&mut world.rlva, A, "@edit=n");
    assert!(drops(&world, collar).iter().all(|c| !allowed(&world, c)));
}

/// RlvActions::canTouch asks neither @interact (it drops the pick,
/// LLViewerWindow::cursorIntersect) nor @showself: the Touch entry of a worn
/// object (handle_attachment_touch) and the chat bar's touch go through.
#[test]
fn rlva_can_touch_leaves_interact_and_showself_to_the_pick() {
    let mut world = parity_demo_world();
    let region = world.main_region.unwrap();
    let chest = worn_attachment(&mut world, 0x7331, 1);
    let collar = worn(&world, chest);
    let collar_local = world.objects.get(collar).unwrap().key.local_id;
    let ground = parity_prim(&mut world, 0x7332, 0);
    let ground_local = parity_local(&world, ground);
    for restriction in ["@interact=n", "@showself=n", "@showselfhead=n"] {
        command(&mut world.rlva, A, restriction);
        assert!(world.rlva_can_touch(region, collar_local), "{restriction}");
        command(&mut world.rlva, A, "@clear");
    }
    command(&mut world.rlva, A, "@interact=n");
    assert!(world.rlva_can_touch(region, ground_local), "canTouch alone");
    assert!(world.rlva_drops_picks() && !world.rlva_can_interact_at(index(&world, ground), None));
    assert!(world.rlva_allows_command(&NetCommand::Touch { local_id: ground_local }, Uuid::nil()));
    command(&mut world.rlva, A, "@touchworld=n");
    assert!(!world.rlva_can_touch(region, ground_local), "@touchworld still refuses");
}

/// cmdline_rezplat (chatbar_as_cmdline.cpp) asks RlvActions::canRez
/// alone: a new prim is refused under @rez, not @interact, which the Create
/// tool asks before it sends (LLToolPlacer::placeObject, `rlva_can_rez`).
#[test]
fn rlva_a_new_prim_is_refused_under_rez_alone() {
    let mut world = parity_demo_world();
    let add = NetCommand::Build(BuildCmd::Add(Box::new(aurora_net::build::NewPrim {
        handle: world.main_region.unwrap(),
        group_id: Uuid::nil(),
        pcode: aurora_prim::params::LL_PCODE_VOLUME,
        material: 3,
        add_flags: 0,
        shape: crate::build::shapes::shapes()[0].raw(),
        ray_start: world.agent.position,
        ray_end: world.agent.position,
        ray_target: Uuid::nil(),
        ray_end_is_intersection: false,
        bypass_raycast: true,
        scale: glam::Vec3::splat(0.5),
        rotation: glam::Quat::IDENTITY,
        state: 0,
    })));
    assert!(world.rlva_allows_command(&add, Uuid::nil()));
    command(&mut world.rlva, A, "@interact=n");
    assert!(world.rlva_allows_command(&add, Uuid::nil()), "the platform of the chat bar");
    assert!(!world.rlva_can_rez(None), "the Create tool");
    command(&mut world.rlva, A, "@interact=y,rez=n");
    assert!(!world.rlva_allows_command(&add, Uuid::nil()));
}

/// LLPanelGroupGeneral::apply and FSFloaterGroupTitles::activateGroupTitle
/// send the title whatever @setgroup holds; only the activation is refused.
#[test]
fn rlva_setgroup_leaves_the_group_title() {
    let mut world = parity_demo_world();
    let (group, role) = (Uuid::from_u128(0x7341), Uuid::from_u128(0x7342));
    world.groups.active = group;
    command(&mut world.rlva, A, "@setgroup=n");
    assert!(world.rlva_allows_command(&NetCommand::GroupTitleUpdate { group, role }, Uuid::nil()));
    assert!(!world.rlva_can_change_group());
}

/// LLFloaterOpenObject::refresh: the contents window closes once
/// RlvActions::canEdit refuses its object.
#[test]
fn rlva_contents_window_follows_can_edit() {
    let mut world = parity_demo_world();
    let ground = parity_prim(&mut world, 0x7351, 0);
    assert!(world.rlva_keeps_contents_open(ground));
    for restriction in ["@edit=n", "@editworld=n", &format!("@editobj:{ground}=n")] {
        command(&mut world.rlva, A, restriction);
        assert!(!world.rlva_keeps_contents_open(ground), "{restriction}");
        command(&mut world.rlva, A, "@clear");
        assert!(world.rlva_keeps_contents_open(ground));
    }
    command(&mut world.rlva, A, "@editattach=n");
    assert!(world.rlva_keeps_contents_open(ground), "a world object");
    assert!(world.rlva_keeps_contents_open(Uuid::from_u128(0x7352)), "an object gone from view");
}

/// rlvCanDeleteOrReturn(pObj), which the area search asks of each row, asks
/// RlvActions::canEdit first: @editworld or @editattach alone hide Delete
/// and Return there, where the selection's rlvCanDeleteOrReturn() is not
/// asked at all.
#[test]
fn rlva_area_search_delete_asks_can_edit_of_each_row() {
    let mut world = parity_demo_world();
    let ground = parity_prim(&mut world, 0x7361, 0);
    let ground = index(&world, ground);
    let chest = worn_attachment(&mut world, 0x7362, 1);
    let collar = worn(&world, chest);
    assert!(world.rlva_can_delete_or_return_object(ground) && world.rlva_can_delete_or_return_object(collar));
    command(&mut world.rlva, A, "@editworld=n");
    assert!(!world.rlva_can_delete_or_return_object(ground) && world.rlva_can_delete_or_return(ground));
    assert!(world.rlva_can_delete_or_return_object(collar));
    command(&mut world.rlva, A, "@editworld=y,editattach=n");
    assert!(world.rlva_can_delete_or_return_object(ground) && !world.rlva_can_delete_or_return_object(collar));
    assert!(world.rlva_can_delete_or_return(collar));
}

/// RLV_BHVR_GETGROUP: gAgent.getGroupName(), the name AgentDataUpdate
/// gave, before the group list arrives too; "none" without an active group
/// or once it is left.
#[test]
fn rlva_getgroup_answers_the_name_of_agent_data_update() {
    let mut world = parity_demo_world();
    let group = Uuid::from_u128(0x7371);
    let ask = |world: &mut World| {
        command(&mut world.rlva, A, "@getgroup=7371");
        assert!(world.rlva_pending().is_empty());
        parity_replies(&mut world.rlva)
    };
    let active = |id: Uuid| aurora_net::NetEvent::ActiveGroup {
        id,
        name: "Tea Party".to_owned(),
        title: String::new(),
    };
    world.apply(active(group));
    assert!(world.groups.groups.iter().all(|g| g.id != group), "no group list yet");
    assert_eq!(ask(&mut world), [parity_reply(7371, "Tea Party")]);
    world.apply(aurora_net::NetEvent::GroupDropped(group));
    assert_eq!(ask(&mut world), [parity_reply(7371, "none")]);
    world.apply(active(Uuid::nil()));
    assert_eq!(ask(&mut world), [parity_reply(7371, "none")]);
}
