//! "Include objects inside objects": the rules, then whole jobs against the
//! demo simulator.
use super::*;
use crate::build::demo_sim::DemoSim;
use crate::build::{BuildSettings, Tool};
use aurora_net::{NetCommand, NetEvent};
use std::sync::Arc;

const ALL: u32 = 0x7fff_ffff;
const AGENT: Uuid = Uuid::from_u128(1);
const GROUP: Uuid = Uuid::from_u128(2);

fn object(owner: Uuid, owner_mask: u32) -> TaskItem {
    TaskItem {
        item_id: Uuid::from_u128(10),
        owner_id: owner,
        asset_type: AT_OBJECT,
        inv_type: AT_OBJECT,
        base_mask: ALL,
        owner_mask,
        name: "Box".into(),
        ..Default::default()
    }
}

#[test]
fn eligibility_follows_the_rez_from_contents_rule_and_modify() {
    // the owner's full-permission object
    assert_eq!(eligibility(&object(AGENT, ALL), AGENT, GROUP, 0), Ok(()));
    // no copy, no transfer: the Legacy Viewer refuses to rez it from contents
    assert_eq!(eligibility(&object(AGENT, ALL & !perm::COPY), AGENT, GROUP, 0), Err(Skip::NoCopy));
    assert_eq!(
        eligibility(&object(AGENT, ALL & !perm::TRANSFER), AGENT, GROUP, 0),
        Err(Skip::NoCopy)
    );
    assert_eq!(
        eligibility(&object(AGENT, ALL & !perm::MODIFY), AGENT, GROUP, 0),
        Err(Skip::NoModify)
    );
    // someone else's (or a Library) item: everyone's copy lets it rez, not modify
    let mut other = object(Uuid::from_u128(3), ALL);
    other.everyone_mask = perm::COPY;
    assert_eq!(eligibility(&other, AGENT, GROUP, 0), Err(Skip::NoModify));
    other.everyone_mask = 0;
    assert_eq!(eligibility(&other, AGENT, GROUP, 0), Err(Skip::NoCopy));
    // group-owned: the active group's copy rezzes it, the agent mask reads everyone's
    let mut group = object(GROUP, ALL);
    group.group_owned = true;
    group.group_id = GROUP;
    group.group_mask = perm::COPY | perm::MODIFY;
    assert_eq!(eligibility(&group, AGENT, GROUP, 0), Err(Skip::NoModify));
    assert_eq!(eligibility(&group, AGENT, Uuid::nil(), 0), Err(Skip::NoCopy));
    // the depth cap
    assert_eq!(eligibility(&object(AGENT, ALL), AGENT, GROUP, MAX_DEPTH - 1), Ok(()));
    assert_eq!(eligibility(&object(AGENT, ALL), AGENT, GROUP, MAX_DEPTH), Err(Skip::TooDeep));
}

#[test]
fn scan_lists_only_object_items() {
    let mut notecard = object(AGENT, ALL);
    notecard.asset_type = 7;
    let mut folder = object(AGENT, ALL);
    folder.is_folder = true;
    let mut heirloom = object(AGENT, ALL & !perm::COPY);
    heirloom.name = "Heirloom".into();
    let found = scan(&[notecard, folder, object(AGENT, ALL), heirloom], AGENT, GROUP, 0);
    assert_eq!(found.eligible.len(), 1);
    assert_eq!(found.skipped, vec![("Heirloom".to_owned(), Skip::NoCopy)]);
}

#[test]
fn rez_point_is_above_the_highest_corner() {
    let p = rez_point(Vec3::new(10.0, 20.0, 30.0), Quat::IDENTITY, Vec3::new(1.0, 1.0, 0.5));
    assert!(p.abs_diff_eq(Vec3::new(10.0, 20.0, 30.25 + REZ_HEIGHT), 1e-5));
    // on its side, a long box reaches higher
    let p = rez_point(
        Vec3::ZERO,
        Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
        Vec3::new(4.0, 1.0, 0.5),
    );
    assert!((p.z - (2.0 + REZ_HEIGHT)).abs() < 1e-4);
}

fn seen(id: u128, flags: u32, position: Vec3) -> Seen {
    Seen {
        id: Uuid::from_u128(id),
        root: true,
        avatar: false,
        flags,
        position,
        scale: Vec3::splat(0.5),
    }
}

#[test]
fn arrival_takes_only_a_new_own_created_selected_root_near_the_point() {
    let ours = crate::build::FLAGS_CREATE_SELECTED | crate::ui::context::flags::OBJECT_YOU_OWNER;
    let at = Vec3::new(100.0, 100.0, 30.0);
    let before: HashSet<Uuid> = [Uuid::from_u128(1)].into();
    assert_eq!(arrival([seen(2, ours, at + Vec3::Z * 0.5)], &before, at), Some(Uuid::from_u128(2)));
    // already there, not created selected, not ours, too far
    assert_eq!(arrival([seen(1, ours, at)], &before, at), None);
    assert_eq!(
        arrival([seen(2, crate::ui::context::flags::OBJECT_YOU_OWNER, at)], &before, at),
        None
    );
    assert_eq!(arrival([seen(2, crate::build::FLAGS_CREATE_SELECTED, at)], &before, at), None);
    assert_eq!(arrival([seen(2, ours, at + Vec3::X * 3.0)], &before, at), None);
    // a child or an avatar is never the rezzed object
    let mut child = seen(2, ours, at);
    child.root = false;
    let mut avatar = seen(3, ours, at);
    avatar.avatar = true;
    assert_eq!(arrival([child, avatar], &before, at), None);
    // the nearest of two
    assert_eq!(
        arrival([seen(2, ours, at + Vec3::X * 1.5), seen(3, ours, at + Vec3::Y * 0.2)], &before, at),
        Some(Uuid::from_u128(3))
    );
}

fn listed(id: u128, name: &str, asset: u128) -> TaskItem {
    TaskItem {
        item_id: Uuid::from_u128(id),
        name: name.into(),
        asset_type: AT_OBJECT,
        asset_id: Uuid::from_u128(asset),
        ..Default::default()
    }
}

#[test]
fn save_back_is_confirmed_as_added_replaced_or_in_place() {
    let original = listed(10, "Box", 100);
    let note = TaskItem {
        item_id: Uuid::from_u128(11),
        asset_type: 7,
        name: "Note".into(),
        ..Default::default()
    };
    let before = vec![original.clone(), note.clone()];
    // nothing yet
    assert_eq!(save_outcome(&before, &before, &original, "Box", 0, false), Saved::Pending);
    // a new item next to the original: the original is then removed
    let added = listed(20, "Box", 200);
    let after = vec![original.clone(), note.clone(), added.clone()];
    assert_eq!(save_outcome(&before, &after, &original, "Box", 0, false), Saved::Added(added));
    // a numbered duplicate name
    let numbered = listed(21, "Box 1", 200);
    let after = vec![original.clone(), note.clone(), numbered.clone()];
    assert_eq!(save_outcome(&before, &after, &original, "Box", 0, false), Saved::Added(numbered));
    // the original's place taken by a new id: nothing to remove
    let new = listed(22, "Box", 200);
    assert_eq!(
        save_outcome(&before, &[note.clone(), new.clone()], &original, "Box", 0, false),
        Saved::Replaced(new)
    );
    // the same id with a new asset: in place
    assert_eq!(
        save_outcome(&before, &[listed(10, "Box", 300), note.clone()], &original, "Box", 0, false),
        Saved::InPlace
    );
    // the original gone and nothing new: not confirmed (the copy is kept)
    assert_eq!(
        save_outcome(&before, std::slice::from_ref(&note), &original, "Box", 0, false),
        Saved::Pending
    );
    // a new object of another name among others is not ours
    let others = vec![original.clone(), listed(30, "Lamp", 1), listed(31, "Chair", 2)];
    assert_eq!(save_outcome(&before, &others, &original, "Box", 0, false), Saved::Pending);
}

#[test]
fn only_evidence_of_the_save_itself_confirms_it() {
    let original = listed(10, "Box", 100);
    let note = TaskItem {
        item_id: Uuid::from_u128(11),
        asset_type: 7,
        name: "Note".into(),
        asset_id: Uuid::from_u128(110),
        ..Default::default()
    };
    let before = vec![original.clone(), note.clone()];
    // another item edited while the save is on its way (the listing's
    // serial moves, the original's asset does not)
    let edited = TaskItem {
        asset_id: Uuid::from_u128(111),
        ..note.clone()
    };
    assert_eq!(
        save_outcome(&before, &[original.clone(), edited], &original, "Box", 0, false),
        Saved::Pending
    );
    // a new object whose name only starts with the original's
    assert_eq!(
        save_outcome(
            &before,
            &[original.clone(), note.clone(), listed(20, "Boxcar", 200)],
            &original,
            "Box",
            0,
            false
        ),
        Saved::Pending
    );
    assert_eq!(
        save_outcome(
            &before,
            &[original.clone(), note.clone(), listed(20, "Box copy", 200)],
            &original,
            "Box",
            0,
            false
        ),
        Saved::Pending
    );
    // two candidates: neither is taken
    assert_eq!(
        save_outcome(
            &before,
            &[original.clone(), note.clone(), listed(20, "Box", 200), listed(21, "Box 1", 201)],
            &original,
            "Box",
            0,
            false
        ),
        Saved::Pending
    );
    // someone else's object of the same name
    let theirs = TaskItem {
        owner_id: Uuid::from_u128(99),
        ..listed(20, "Box", 200)
    };
    assert_eq!(
        save_outcome(&before, &[original.clone(), note.clone(), theirs], &original, "Box", 0, false),
        Saved::Pending
    );
    assert!(same_name("Box 12", "Box"));
    assert!(!same_name("Box ", "Box"));
    assert!(!same_name("Box 1a", "Box"));
    assert!(!same_name("Boxcar", "Box"));
}

#[test]
fn recovery_deletes_only_copies_whose_original_is_safe_and_stops_on_rez_failures() {
    let stop = |f: Failure| recovery(&f);
    assert_eq!(
        stop(Failure::RezRefused("full".into())),
        Recovery {
            delete_copy: false,
            stop: true
        }
    );
    assert!(stop(Failure::RezTimeout).stop);
    assert!(stop(Failure::Preflight(String::new())).stop);
    // not ours: never deleted
    assert_eq!(
        stop(Failure::NotOurs),
        Recovery {
            delete_copy: false,
            stop: true
        }
    );
    assert_eq!(
        stop(Failure::Step(String::new())),
        Recovery {
            delete_copy: true,
            stop: false
        }
    );
    // the copy may be the only instance left
    assert_eq!(
        stop(Failure::OriginalGone),
        Recovery {
            delete_copy: false,
            stop: false
        }
    );
    assert!(rez_refusal(
        "Can't rez object 'Box' at { 1, 2, 3 } because the parcel is full.",
        "Box"
    ));
    assert!(!rez_refusal("Region restart in 5 minutes", "Box"));
}

#[test]
fn a_refusal_speaks_of_rezzing_and_names_the_object_or_says_no() {
    // Second Life and OpenSim texts
    assert!(rez_refusal(
        "Can't rez object 'Box' at { 1, 2, 3 } on parcel 'X' because the owner of this land does not allow it.",
        "Box"
    ));
    assert!(rez_refusal("Can't rez object because the parcel is full.", "Box"));
    assert!(rez_refusal("Unable to rez the object: not enough land resources", "Lamp"));
    assert!(rez_refusal("You can\u{2019}t rez here", ""));
    assert!(rez_refusal("Rezzing is not permitted on this parcel", "Box"));
    // naming the object is enough, whatever the wording
    assert!(rez_refusal("Rez of outer BOX stopped by the region", "Outer box"));
    // rezzing without a refusal or the name, or a refusal without rezzing
    assert!(!rez_refusal("Your friend rezzed a gift", "Box"));
    assert!(!rez_refusal("Region rezzed objects successfully", "Box"));
    assert!(!rez_refusal("Teleport denied: the region is full", "Box"));
    assert!(!rez_refusal("The prezzie for Box is full of candy", "Box"));
}

#[test]
fn in_place_the_serial_counts_only_with_every_other_item_unchanged() {
    // Second Life: the same item id, object asset ids hidden in listings
    let original = listed(10, "Box", 0);
    let note = TaskItem {
        item_id: Uuid::from_u128(11),
        asset_type: 7,
        name: "Note".into(),
        ..Default::default()
    };
    let before = vec![original.clone(), note.clone()];
    // both moves of a save in place, nothing else changed (the original's
    // record follows the copy's: its group)
    let regrouped = TaskItem {
        group_id: Uuid::from_u128(5),
        ..original.clone()
    };
    assert_eq!(
        save_outcome(&before, &[regrouped.clone(), note.clone()], &original, "Box", 2, false),
        Saved::InPlace
    );
    // one move: only once the listing stayed quiet since
    assert_eq!(
        save_outcome(&before, &[regrouped.clone(), note.clone()], &original, "Box", 1, false),
        Saved::Pending
    );
    assert_eq!(
        save_outcome(&before, &[regrouped, note.clone()], &original, "Box", 1, true),
        Saved::InPlace
    );
    // no move: nothing yet
    assert_eq!(save_outcome(&before, &before, &original, "Box", 0, true), Saved::Pending);
    // another item changed, added or removed meanwhile
    let renamed = TaskItem {
        name: "Note 2".into(),
        ..note.clone()
    };
    assert_eq!(
        save_outcome(&before, &[original.clone(), renamed], &original, "Box", 2, true),
        Saved::Pending
    );
    let dropped = TaskItem {
        item_id: Uuid::from_u128(12),
        asset_type: 7,
        name: "Dropped".into(),
        ..Default::default()
    };
    assert_eq!(
        save_outcome(&before, &[original.clone(), note.clone(), dropped], &original, "Box", 2, true),
        Saved::Pending
    );
    assert_eq!(
        save_outcome(std::slice::from_ref(&before[0]), &[], &original, "Box", 2, true),
        Saved::Pending
    );
    assert_eq!(serial_moves(7, 9), 2);
    assert_eq!(serial_moves(i16::MAX, i16::MIN), 1);
}

#[test]
fn a_late_update_of_the_save_before_never_confirms_the_next() {
    // A saved in place, then B's save sent: A's delayed second update moves
    // the serial and changes A's record while B is untouched
    let a = listed(13, "Lamp", 0);
    let b = listed(10, "Box", 0);
    let before = vec![a.clone(), b.clone()];
    let a_settled = TaskItem {
        next_owner_mask: perm::MOVE | perm::COPY,
        group_id: Uuid::from_u128(5),
        ..a
    };
    for moves in [1, 2] {
        assert_eq!(
            save_outcome(&before, &[a_settled.clone(), b.clone()], &b, "Box", moves, true),
            Saved::Pending
        );
    }
}

#[test]
fn a_save_found_only_by_the_copys_name_never_removes_the_original() {
    let original = listed(10, "Box", 100);
    let before = vec![original.clone()];
    let renamed = listed(20, "Glowing thing", 200);
    // a script renamed the copy: found by that name, the original kept next to it
    assert_eq!(
        save_outcome(&before, &[original.clone(), renamed.clone()], &original, "Glowing thing", 0, false),
        Saved::Unnamed(renamed.clone())
    );
    // the original gone: the simulator replaced it
    assert_eq!(
        save_outcome(&before, std::slice::from_ref(&renamed), &original, "Glowing thing", 0, false),
        Saved::Replaced(renamed.clone())
    );
    // a new object of any other name is not the save
    assert_eq!(
        save_outcome(&before, &[original.clone(), renamed.clone()], &original, "Box", 0, false),
        Saved::Pending
    );
    assert_eq!(
        save_outcome(&before, std::slice::from_ref(&renamed), &original, "Box", 0, false),
        Saved::Pending
    );
    // a name match still removes it
    let named = listed(21, "Box", 200);
    assert_eq!(
        save_outcome(&before, &[original.clone(), named.clone()], &original, "Glowing thing", 0, false),
        Saved::Added(named)
    );
}

#[test]
fn object_masks_follow_the_bulk_choices() {
    let options = BulkPerms {
        share: true,
        everyone_copy: true,
        next_modify: true,
        next_copy: false,
        ..Default::default()
    };
    assert_eq!(
        object_masks(&options),
        (
            perm::COPY | perm::MODIFY | perm::MOVE,
            perm::COPY,
            perm::MOVE | perm::MODIFY | perm::TRANSFER
        )
    );
    let changes = permission_changes(&options);
    assert!(changes.contains(&(perm_field::GROUP, true, perm::COPY | perm::MODIFY | perm::MOVE)));
    assert!(changes.contains(&(perm_field::EVERYONE, true, perm::COPY)));
    assert!(changes.contains(&(perm_field::NEXT_OWNER, true, perm::MODIFY | perm::TRANSFER)));
    assert!(changes.contains(&(perm_field::NEXT_OWNER, false, perm::COPY)));
    assert!(!changes.iter().any(|(f, set, _)| *f == perm_field::GROUP && !set));
}

/// The viewer and the demo simulator, commands and events carried between
/// them as the app does.
struct Harness {
    world: World,
    tool: BuildTool,
    sim: DemoSim,
    clock: Instant,
    chest: Uuid,
    /// Commands the "network" loses.
    drop: Box<dyn FnMut(&BuildCmd, &World) -> bool>,
    sent: Vec<BuildCmd>,
    /// A script renames every object on rez.
    rename_on_rez: Option<String>,
    /// Ticks the replies of a RezInventory take to come.
    rez_delay: u32,
    /// Replies on their way: ticks left, then the events.
    held: Vec<(u32, Vec<NetEvent>)>,
    /// A rez's child prims kept back, delivered by the test.
    hold_children: bool,
    held_children: Vec<NetEvent>,
}

impl Harness {
    fn new() -> Self {
        let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        let handle = world.main_region.expect("demo region");
        for event in demo::events(handle, 25.0) {
            world.apply(event);
        }
        let chest = demo::id(demo::CHEST);
        let mut h = Self {
            world,
            tool: BuildTool::default(),
            sim: DemoSim::default(),
            clock: Instant::now(),
            chest,
            drop: Box::new(|_, _| false),
            sent: Vec::new(),
            rename_on_rez: None,
            rez_delay: 0,
            held: Vec::new(),
            hold_children: false,
            held_children: Vec::new(),
        };
        let idx = h.world.objects.index_of_uuid(&chest).expect("chest");
        h.tool.open_build(Tool::Edit);
        h.tool.click_select(&mut h.world, &BuildSettings::default(), Some(idx), false);
        h.pump();
        h
    }

    fn deliver(&mut self, event: NetEvent) {
        if let Some(event) = self.world.apply(event) {
            match event {
                NetEvent::ObjectProperties(props) => self.tool.on_properties(&self.world, props),
                NetEvent::TaskInventory {
                    object,
                    serial,
                    result: Ok(items),
                } => self.tool.on_task_inventory(&self.world, object, serial, items),
                NetEvent::TaskInventory {
                    object, result: Err(e), ..
                } => self.tool.replacement_task_inventory_error(object, e),
                NetEvent::ScriptRunning {
                    object_id,
                    item_id,
                    running,
                    compile,
                } => {
                    self.tool.contents.running.insert((object_id, item_id), (running, compile));
                }
                _ => {}
            }
        }
    }

    fn pump(&mut self) {
        for _ in 0..8 {
            let commands = std::mem::take(&mut self.tool.out);
            if commands.is_empty() {
                break;
            }
            for command in commands {
                let NetCommand::Build(command) = command else { continue };
                if (self.drop)(&command, &self.world) {
                    continue;
                }
                self.sent.push(command.clone());
                let events = self.sim.reply(&self.world, &command);
                let rezzed: Vec<(u64, u32)> = events
                    .iter()
                    .filter(|_| matches!(command, BuildCmd::RezInventory(_)))
                    .filter_map(|e| match e {
                        NetEvent::ObjectUpdates { handle, objects } => objects.first().map(|o| (*handle, o.local_id)),
                        _ => None,
                    })
                    .collect();
                if self.rez_delay > 0 && matches!(command, BuildCmd::RezInventory(_)) {
                    self.held.push((self.rez_delay, events));
                    continue;
                }
                let events = if self.hold_children && matches!(command, BuildCmd::RezInventory(_)) {
                    let mut roots = Vec::new();
                    for event in events {
                        match event {
                            NetEvent::ObjectUpdates { handle, objects } => {
                                let (r, children): (Vec<_>, Vec<_>) = objects.into_iter().partition(|o| o.parent_id == 0);
                                if !children.is_empty() {
                                    self.held_children.push(NetEvent::ObjectUpdates { handle, objects: children });
                                }
                                roots.push(NetEvent::ObjectUpdates { handle, objects: r });
                            }
                            other => roots.push(other),
                        }
                    }
                    roots
                } else {
                    events
                };
                for event in events {
                    self.deliver(event);
                }
                if let Some(name) = self.rename_on_rez.clone() {
                    for (handle, local_id) in rezzed {
                        let rename = BuildCmd::SetName {
                            handle,
                            local_ids: vec![local_id],
                            name: name.clone(),
                        };
                        for event in self.sim.reply(&self.world, &rename) {
                            self.deliver(event);
                        }
                    }
                }
            }
        }
    }

    fn run(&mut self, options: BulkPerms) -> String {
        self.run_until(options, |_| false)
    }

    /// The replies whose time has come.
    fn release(&mut self) {
        for (ticks, _) in &mut self.held {
            *ticks = ticks.saturating_sub(1);
        }
        let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.held).into_iter().partition(|(t, _)| *t == 0);
        self.held = later;
        for (_, events) in due {
            for event in events {
                self.deliver(event);
            }
        }
    }

    fn start(&mut self, options: BulkPerms) {
        self.tool.ui.bulk.options = options;
        self.tool.ui.bulk.options.nested = true;
        self.tool.start_bulk_permissions(&self.world);
        assert!(self.tool.ui.bulk.deep.is_some());
    }

    /// One tick of the job; false once it is over.
    fn tick(&mut self) -> bool {
        self.clock += Duration::from_millis(250);
        self.tool.advance_deep_permissions(&self.world, self.clock);
        self.pump();
        self.release();
        self.tool.ui.bulk.deep.is_some()
    }

    /// Another object of ours, created selected, at the rez point of the
    /// last RezInventory sent: it looks like the one asked for.
    fn lookalike(&mut self) -> Uuid {
        let rez = self
            .sent
            .iter()
            .rev()
            .find_map(|c| match c {
                BuildCmd::RezInventory(r) => Some(r.clone()),
                _ => None,
            })
            .expect("a rez sent");
        let mut object = crate::demo::inventory_rez(&rez);
        object.position = rez.ray_end;
        let id = object.full_id;
        self.deliver(NetEvent::ObjectUpdates {
            handle: rez.handle,
            objects: vec![object],
        });
        id
    }

    /// Runs the job, pressing Stop once `stop` says so (checked before
    /// each tick).
    fn run_until(&mut self, options: BulkPerms, mut stop: impl FnMut(&Job) -> bool) -> String {
        self.start(options);
        for _ in 0..2000 {
            if let Some(job) = self.tool.ui.bulk.deep.as_mut()
                && !job.stop_requested()
                && stop(job)
            {
                job.request_stop();
            }
            if !self.tick() {
                return self.tool.ui.bulk.status.clone();
            }
        }
        panic!(
            "deep permissions never finished: {:?}",
            self.tool.ui.bulk.deep.as_ref().map(|j| j.stage)
        );
    }

    fn chest_items(&mut self) -> Vec<TaskItem> {
        let handle = self.world.main_region.expect("region");
        let events = self.sim.reply(
            &self.world,
            &BuildCmd::RequestTaskInventory {
                handle,
                local_id: demo::CHEST,
                object_id: self.chest,
            },
        );
        events
            .into_iter()
            .find_map(|e| match e {
                NetEvent::TaskInventory { result: Ok(items), .. } => Some(items.into_iter().filter(|i| !i.is_folder).collect()),
                _ => None,
            })
            .expect("listing")
    }

    fn rezzed_left(&self) -> usize {
        self.world
            .objects
            .iter()
            .filter(|(_, o)| o.update_flags & crate::build::FLAGS_CREATE_SELECTED != 0)
            .count()
    }

    fn count(&self, f: impl Fn(&BuildCmd) -> bool) -> usize {
        self.sent.iter().filter(|c| f(c)).count()
    }
}

/// Whether the report has its "kept next to their originals" part (any
/// count), in the interface's language.
fn says_kept(report: &str) -> bool {
    (1..=20).any(|kept| report.contains(&*aurora_i18n::tr!("content-perms-nested-kept-next", kept = kept)))
}

fn shared() -> BulkPerms {
    BulkPerms {
        share: true,
        everyone_copy: true,
        ..Default::default()
    }
}

fn named<'a>(items: &'a [TaskItem], name: &str) -> Vec<&'a TaskItem> {
    items.iter().filter(|i| i.name == name).collect()
}

#[test]
fn whole_job_opens_every_level_saves_back_and_deletes_the_copies() {
    let mut h = Harness::new();
    let before = h.chest_items();
    let outer_before = named(&before, aurora_i18n::tr!("content-perms-demo-outer"))[0].clone();
    let report = h.run(shared());
    assert!(report.contains(" 6 "), "{report}");
    let items = h.chest_items();
    // one Outer box, a new item: the original was removed after the save
    let outer = named(&items, aurora_i18n::tr!("content-perms-demo-outer"));
    assert_eq!(outer.len(), 1);
    assert_ne!(outer[0].item_id, outer_before.item_id);
    assert_eq!(outer[0].group_mask, perm::COPY | perm::MODIFY | perm::MOVE);
    assert_eq!(outer[0].everyone_mask, perm::COPY);
    // the closed objects keep their items, with the one-level change
    for name in [
        aurora_i18n::tr!("content-perms-demo-heirloom"),
        aurora_i18n::tr!("content-perms-demo-sealed"),
    ] {
        let kept = named(&items, name);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].everyone_mask, perm::COPY);
    }
    // the saved Outer box holds the changed script and notecard and a new Inner box
    let held = h.sim.stored(outer[0].item_id).expect("outer contents").clone();
    assert_eq!(held.len(), 3);
    for item in held.iter().filter(|i| i.asset_type != AT_OBJECT) {
        assert_eq!(item.everyone_mask, perm::COPY, "{}", item.name);
        assert_eq!(item.group_mask, perm::COPY | perm::MODIFY | perm::MOVE, "{}", item.name);
    }
    let inner = held.iter().find(|i| i.asset_type == AT_OBJECT).expect("inner box");
    assert_eq!(inner.name, aurora_i18n::tr!("content-perms-demo-inner"));
    let inner_held = h.sim.stored(inner.item_id).expect("inner contents").clone();
    assert_eq!(inner_held.len(), 2);
    let tiny = inner_held.iter().find(|i| i.asset_type == AT_OBJECT).expect("tiny box");
    let tiny_held = h.sim.stored(tiny.item_id).expect("tiny contents");
    assert_eq!(tiny_held.len(), 1);
    assert_eq!(tiny_held[0].everyone_mask, perm::COPY);
    // six rezzes (Outer, Inner, Tiny, Linked, Pebble, Crate stack), six
    // saves, six copies deleted, nothing left in world
    assert_eq!(h.count(|c| matches!(c, BuildCmd::RezInventory(_))), 6);
    assert_eq!(
        h.count(|c| matches!(c, BuildCmd::DeRez { destination, .. } if *destination == derez::SAVE_INTO_TASK_INVENTORY)),
        6
    );
    assert_eq!(
        h.count(|c| matches!(c, BuildCmd::DeRez { destination, .. } if *destination == derez::TRASH)),
        6
    );
    assert_eq!(h.rezzed_left(), 0);
    // every stopped script was started again before its save
    let stops = h.count(|c| matches!(c, BuildCmd::SetScriptRunning { running: false, .. }));
    let starts = h.count(|c| matches!(c, BuildCmd::SetScriptRunning { running: true, .. }));
    assert_eq!(stops, 4);
    assert_eq!(starts, 4);
    // rezzed from the chest's contents, created selected, never removed from there
    assert!(h.sent.iter().all(|c| match c {
        BuildCmd::RezInventory(r) => r.selected && !r.remove && r.target.is_nil(),
        _ => true,
    }));
}

#[test]
fn every_prim_of_a_nested_linkset_is_opened_and_it_is_saved_once() {
    let mut h = Harness::new();
    let before = h.chest_items();
    let linked_name = aurora_i18n::tr!("content-perms-demo-linked");
    let linked_before = named(&before, linked_name)[0].clone();
    h.run(shared());
    let items = h.chest_items();
    let linked = named(&items, linked_name);
    assert_eq!(linked.len(), 1);
    assert_ne!(linked[0].item_id, linked_before.item_id);
    // the root's notecard
    let root = h.sim.stored(linked[0].item_id).expect("linked root").clone();
    assert_eq!(root.len(), 1);
    assert_eq!(root[0].everyone_mask, perm::COPY);
    // the child prim: its script changed (and stopped, then started again),
    // its object opened and saved back with its notecard changed
    let links = h.sim.stored_links(linked[0].item_id).expect("linked child prims").clone();
    assert_eq!(links.len(), 1);
    let child = &links[0];
    let script = child.iter().find(|i| i.asset_type == 10).expect("child script");
    assert_eq!(script.everyone_mask, perm::COPY);
    let pebbles: Vec<_> = child.iter().filter(|i| i.asset_type == AT_OBJECT).collect();
    assert_eq!(pebbles.len(), 1, "the Pebble box's original was removed");
    assert_eq!(pebbles[0].name, aurora_i18n::tr!("content-perms-demo-pebble"));
    assert_eq!(pebbles[0].everyone_mask, perm::COPY);
    let pebble_held = h.sim.stored(pebbles[0].item_id).expect("pebble contents");
    assert_eq!(pebble_held.len(), 1);
    assert_eq!(pebble_held[0].everyone_mask, perm::COPY);
    // the Pebble box rezzed from the child prim and saved back into it
    let pebble_rez = h
        .sent
        .iter()
        .find_map(|c| match c {
            BuildCmd::RezInventory(r) if r.item.name == aurora_i18n::tr!("content-perms-demo-pebble") => Some(r.from_task),
            _ => None,
        })
        .expect("pebble rezzed");
    assert!(
        h.sent
            .iter()
            .any(|c| matches!(c, BuildCmd::DeRez { destination, destination_id, .. }
        if *destination == derez::SAVE_INTO_TASK_INVENTORY && *destination_id == pebble_rez))
    );
    // the masks of the Linked box went to both prims at once
    assert!(
        h.sent
            .iter()
            .any(|c| matches!(c, BuildCmd::SetPermissions { local_ids, .. } if local_ids.len() == 2))
    );
}

#[test]
fn a_save_in_place_removes_nothing() {
    let mut h = Harness::new();
    h.sim.deep_in_place = true;
    let before = h.chest_items();
    let outer_before = named(&before, aurora_i18n::tr!("content-perms-demo-outer"))[0].clone();
    h.run(shared());
    let items = h.chest_items();
    let outer = named(&items, aurora_i18n::tr!("content-perms-demo-outer"));
    assert_eq!(outer.len(), 1);
    assert_eq!(outer[0].item_id, outer_before.item_id);
    assert_eq!(h.count(|c| matches!(c, BuildCmd::RemoveTaskInventory { .. })), 0);
    assert_eq!(h.rezzed_left(), 0);
}

#[test]
fn a_refused_rez_stops_with_nothing_changed() {
    let mut h = Harness::new();
    h.sim.deep_refuse = true;
    let before = h.chest_items();
    let report = h.run(shared());
    assert!(report.contains("Can't rez"), "{report}");
    assert_eq!(h.count(|c| matches!(c, BuildCmd::RezInventory(_))), 1);
    assert_eq!(h.count(|c| matches!(c, BuildCmd::UpdateTaskInventory { .. })), 0);
    assert_eq!(h.count(|c| matches!(c, BuildCmd::DeRez { .. })), 0);
    assert_eq!(h.chest_items(), before);
}

#[test]
fn a_lost_rez_times_out_and_stops() {
    let mut h = Harness::new();
    h.drop = Box::new(|c, _| matches!(c, BuildCmd::RezInventory(_)));
    let before = h.chest_items();
    h.run(shared());
    let job_end = h.tool.ui.bulk.status.clone();
    assert!(!job_end.is_empty());
    assert_eq!(h.count(|c| matches!(c, BuildCmd::UpdateTaskInventory { .. })), 0);
    assert_eq!(h.chest_items(), before);
}

#[test]
fn a_failed_deep_save_deletes_that_copy_and_keeps_its_original() {
    let mut h = Harness::new();
    let tiny = aurora_i18n::tr!("content-perms-demo-tiny");
    // the Tiny box's save back is lost on the way
    h.drop = Box::new(move |c, w| {
        matches!(c, BuildCmd::DeRez { destination, local_ids, handle, .. }
            if *destination == derez::SAVE_INTO_TASK_INVENTORY
                && w.objects
                    .index_of(&ObjKey { region: *handle, local_id: local_ids[0] })
                    .and_then(|i| w.objects.get(i))
                    .is_some_and(|o| o.text == tiny))
    });
    h.run(shared());
    assert_eq!(h.rezzed_left(), 0);
    let items = h.chest_items();
    let outer = named(&items, aurora_i18n::tr!("content-perms-demo-outer"));
    assert_eq!(outer.len(), 1);
    let held = h.sim.stored(outer[0].item_id).expect("outer").clone();
    let inner = held.iter().find(|i| i.asset_type == AT_OBJECT).expect("inner");
    let inner_held = h.sim.stored(inner.item_id).expect("inner").clone();
    // the Tiny box is still the one it was: one item, its script unchanged
    let tinies: Vec<_> = inner_held.iter().filter(|i| i.asset_type == AT_OBJECT).collect();
    assert_eq!(tinies.len(), 1);
    let tiny_held = h.sim.stored(tinies[0].item_id).expect("tiny");
    assert_eq!(tiny_held[0].everyone_mask, 0);
    // the levels above it were still saved
    assert_eq!(
        inner_held.iter().find(|i| i.asset_type == 7).map(|i| i.everyone_mask),
        Some(perm::COPY)
    );
}

#[test]
fn without_objects_ticked_the_copies_keep_their_masks() {
    let mut h = Harness::new();
    let mut options = shared();
    let objects = TYPES_OBJECT_INDEX;
    options.types[objects] = false;
    h.run(options);
    assert_eq!(h.count(|c| matches!(c, BuildCmd::SetPermissions { .. })), 0);
    let items = h.chest_items();
    let outer = named(&items, aurora_i18n::tr!("content-perms-demo-outer"));
    assert_eq!(outer[0].everyone_mask, 0);
    let held = h.sim.stored(outer[0].item_id).expect("outer");
    assert!(
        held.iter()
            .filter(|i| i.asset_type != AT_OBJECT)
            .all(|i| i.everyone_mask == perm::COPY)
    );
}

/// The Objects filter of the window (TYPES order, floater_bulk_perms.xml).
const TYPES_OBJECT_INDEX: usize = 5;

#[test]
fn a_renamed_copy_takes_its_item_name_back_before_the_save() {
    let mut h = Harness::new();
    h.rename_on_rez = Some("Renamed by a script".into());
    let before = h.chest_items();
    let outer_before = named(&before, aurora_i18n::tr!("content-perms-demo-outer"))[0].clone();
    let report = h.run(shared());
    assert!(!says_kept(&report), "{report}");
    assert_eq!(
        h.count(|c| matches!(c, BuildCmd::SetName { name, .. } if *name == outer_before.name)),
        1
    );
    let items = h.chest_items();
    let outer = named(&items, aurora_i18n::tr!("content-perms-demo-outer"));
    assert_eq!(outer.len(), 1);
    assert_ne!(outer[0].item_id, outer_before.item_id);
    assert!(named(&items, "Renamed by a script").is_empty());
    assert_eq!(h.rezzed_left(), 0);
}

#[test]
fn a_name_that_stays_different_keeps_the_original_next_to_the_save() {
    let mut h = Harness::new();
    h.rename_on_rez = Some("Renamed by a script".into());
    // the name cannot be set back
    h.drop = Box::new(|c, _| matches!(c, BuildCmd::SetName { .. }));
    let before = h.chest_items();
    let outer_before = named(&before, aurora_i18n::tr!("content-perms-demo-outer"))[0].clone();
    let report = h.run(shared());
    assert!(says_kept(&report), "{report}");
    // only the Crate stack's original is removed: a coalesced item keeps its
    // own name, whatever its parts are called
    assert_eq!(h.count(|c| matches!(c, BuildCmd::RemoveTaskInventory { .. })), 1);
    let items = h.chest_items();
    assert_eq!(named(&items, aurora_i18n::tr!("content-perms-demo-stack")).len(), 1);
    // the originals and the saves side by side (Outer box, Linked box)
    assert!(items.iter().any(|i| i.item_id == outer_before.item_id));
    assert_eq!(named(&items, "Renamed by a script").len(), 2);
    assert_eq!(h.rezzed_left(), 0);
}

#[test]
fn stop_at_the_second_level_trashes_every_copy_and_keeps_the_originals() {
    let mut h = Harness::new();
    let before = h.chest_items();
    let report = h.run_until(shared(), |job| job.level() == 2 && job.stage == Stage::Scripts);
    assert!(report.contains(aurora_i18n::tr!("content-perms-nested-stopped")), "{report}");
    assert_eq!(h.rezzed_left(), 0);
    // two copies rezzed (Outer box, Inner box), both trashed, nothing saved
    assert_eq!(h.count(|c| matches!(c, BuildCmd::RezInventory(_))), 2);
    assert_eq!(
        h.count(|c| matches!(c, BuildCmd::DeRez { destination, .. } if *destination == derez::TRASH)),
        2
    );
    assert_eq!(
        h.count(|c| matches!(c, BuildCmd::DeRez { destination, .. } if *destination == derez::SAVE_INTO_TASK_INVENTORY)),
        0
    );
    assert_eq!(h.count(|c| matches!(c, BuildCmd::RemoveTaskInventory { .. })), 0);
    assert_eq!(h.chest_items(), before);
}

#[test]
fn stop_while_a_rez_is_on_its_way_trashes_it_when_it_arrives() {
    let mut h = Harness::new();
    let report = h.run_until(shared(), |job| job.level() == 1 && job.rez_in_flight());
    assert!(report.contains(aurora_i18n::tr!("content-perms-nested-stopped")), "{report}");
    assert_eq!(h.count(|c| matches!(c, BuildCmd::RezInventory(_))), 2);
    assert_eq!(h.rezzed_left(), 0);
    assert_eq!(h.count(|c| matches!(c, BuildCmd::RemoveTaskInventory { .. })), 0);
}

#[test]
fn stop_with_another_object_arriving_first_deletes_only_the_verified_copy() {
    let mut h = Harness::new();
    let before = h.chest_items();
    // the Outer box comes two seconds after its rez
    h.rez_delay = 8;
    h.start(shared());
    let mut lookalike = None;
    for _ in 0..2000 {
        if lookalike.is_none()
            && let Some(job) = h.tool.ui.bulk.deep.as_mut()
            && job.rez_in_flight()
        {
            job.request_stop();
            lookalike = Some(h.lookalike());
        }
        if !h.tick() {
            break;
        }
    }
    assert!(h.tool.ui.bulk.deep.is_none(), "the job ended");
    let lookalike = lookalike.expect("stopped during the rez");
    // the other object is left alone, the real copy went to the trash
    let idx = h.world.objects.index_of_uuid(&lookalike).expect("still in world");
    let local = h.world.objects.get(idx).expect("object").key.local_id;
    assert!(
        !h.sent
            .iter()
            .any(|c| matches!(c, BuildCmd::DeRez { local_ids, .. } if local_ids.contains(&local)))
    );
    assert_eq!(
        h.count(|c| matches!(c, BuildCmd::DeRez { destination, .. } if *destination == derez::TRASH)),
        1
    );
    assert_eq!(h.count(|c| matches!(c, BuildCmd::RezInventory(_))), 1, "the rez is not sent again");
    assert_eq!(h.rezzed_left(), 1);
    assert_eq!(h.count(|c| matches!(c, BuildCmd::RemoveTaskInventory { .. })), 0);
    assert_eq!(h.chest_items(), before);
}

#[test]
fn an_arrival_whose_origin_never_comes_is_never_deleted() {
    let mut h = Harness::new();
    let before = h.chest_items();
    // the properties of the new object never come back
    h.drop = Box::new(|c, _| matches!(c, BuildCmd::Select { .. }));
    let report = h.run(shared());
    assert!(
        report.contains(&*aurora_i18n::tr!(
            "content-perms-nested-unverified",
            name = aurora_i18n::tr!("content-perms-demo-outer")
        )),
        "{report}"
    );
    assert_eq!(h.count(|c| matches!(c, BuildCmd::DeRez { .. })), 0);
    assert_eq!(h.rezzed_left(), 1);
    assert_eq!(h.chest_items(), before);
}

#[test]
fn a_coalesced_item_is_opened_part_by_part_and_saved_back_together() {
    let mut h = Harness::new();
    let name = aurora_i18n::tr!("content-perms-demo-stack");
    let before = h.chest_items();
    let stack_before = named(&before, name)[0].clone();
    h.run(shared());
    let items = h.chest_items();
    let stack = named(&items, name);
    assert_eq!(stack.len(), 1, "one item, the original removed after the save");
    assert_ne!(stack[0].item_id, stack_before.item_id);
    assert!(is_group(stack[0]), "still several objects saved as one");
    let parts = h.sim.stored_group(stack[0].item_id).expect("the parts").clone();
    assert_eq!(parts.len(), 2);
    for part in &parts {
        assert_eq!(part.len(), 1);
        assert_eq!(part[0].everyone_mask, perm::COPY, "{}", part[0].name);
    }
    // both parts saved in one DeRezObject, and deleted together
    assert!(h.sent.iter().any(|c| matches!(c, BuildCmd::DeRez { destination, local_ids, .. }
        if *destination == derez::SAVE_INTO_TASK_INVENTORY && local_ids.len() == 2)));
    assert!(
        h.sent
            .iter()
            .any(|c| matches!(c, BuildCmd::DeRez { destination, local_ids, .. } if *destination == derez::TRASH && local_ids.len() == 2))
    );
    assert_eq!(h.rezzed_left(), 0);
}

#[test]
fn a_coalesced_item_saved_as_one_object_keeps_its_parts_in_world() {
    let mut h = Harness::new();
    h.sim.deep_split = true;
    let report = h.run(shared());
    assert!(
        report.contains(&*aurora_i18n::tr!(
            "content-perms-nested-group-split",
            name = aurora_i18n::tr!("content-perms-demo-stack")
        )),
        "{report}"
    );
    // the two parts left for the user to take together again
    assert_eq!(h.rezzed_left(), 2);
}

#[test]
fn every_part_of_a_coalesced_rez_is_taken_wherever_it_lands() {
    let before: HashSet<Uuid> = [Uuid::from_u128(1)].into();
    let flags = crate::build::FLAGS_CREATE_SELECTED | crate::ui::context::flags::OBJECT_YOU_OWNER;
    let seen = [
        seen(1, flags, Vec3::ZERO),
        seen(2, flags, Vec3::new(40.0, 0.0, 0.0)),
        seen(3, flags, Vec3::new(1.0, 0.0, 0.0)),
        seen(4, crate::ui::context::flags::OBJECT_YOU_OWNER, Vec3::ZERO),
    ];
    assert_eq!(group_arrivals(seen, &before), vec![Uuid::from_u128(2), Uuid::from_u128(3)]);
}

#[test]
fn an_unverified_arrival_is_never_deleted_even_when_a_part_disappears() {
    let mut h = Harness::new();
    let stack = aurora_i18n::tr!("content-perms-demo-stack");
    h.start(shared());
    let mut lookalike = None;
    let mut killed = None;
    for _ in 0..4000 {
        let Some(job) = h.tool.ui.bulk.deep.as_ref() else { break };
        let (stage, name, in_flight) = (job.stage, job.name(), job.rez_in_flight());
        if lookalike.is_none() && in_flight && name == stack {
            // another object of ours among the stack's parts; the
            // properties of what came never answered from then on
            lookalike = Some(h.lookalike());
            h.drop = Box::new(|c, _| matches!(c, BuildCmd::Select { .. }));
        }
        if killed.is_none()
            && stage == Stage::Props
            && let Some(other) = lookalike
        {
            // one real part leaves the world before the check, Stop pressed
            let part = h
                .world
                .objects
                .iter()
                .find(|(_, o)| o.update_flags & crate::build::FLAGS_CREATE_SELECTED != 0 && o.parent_id == 0 && o.full_id != other)
                .map(|(_, o)| o.key)
                .expect("a part");
            h.deliver(NetEvent::ObjectsKilled {
                handle: part.region,
                local_ids: vec![part.local_id],
            });
            killed = Some(part);
            if let Some(job) = h.tool.ui.bulk.deep.as_mut() {
                job.request_stop();
            }
        }
        if !h.tick() {
            break;
        }
    }
    assert!(h.tool.ui.bulk.deep.is_none(), "the job ended");
    assert!(killed.is_some(), "the stack's arrival was checked");
    let other = lookalike.expect("lookalike");
    let idx = h.world.objects.index_of_uuid(&other).expect("the other object stays in world");
    let local = h.world.objects.get(idx).expect("object").key.local_id;
    assert!(!h.sent.iter().any(|c| matches!(c, BuildCmd::DeRez { destination, local_ids, .. }
        if *destination == derez::TRASH && local_ids.contains(&local))));
    // the stack's part left and the other object: nothing unchecked deleted
    assert_eq!(h.rezzed_left(), 2);
}

#[test]
fn a_child_prim_that_comes_late_is_still_opened_before_the_save() {
    let mut h = Harness::new();
    let linked = aurora_i18n::tr!("content-perms-demo-linked");
    h.hold_children = true;
    h.start(shared());
    let mut released = false;
    for _ in 0..4000 {
        let Some(job) = h.tool.ui.bulk.deep.as_ref() else { break };
        if !released && !h.held_children.is_empty() && job.stage == Stage::Scripts {
            // the Linked box's child prim arrives once its root reached Scripts
            released = true;
            h.hold_children = false;
            for event in std::mem::take(&mut h.held_children) {
                h.deliver(event);
            }
        }
        if !h.tick() {
            break;
        }
    }
    assert!(released, "the child prim was held");
    let items = h.chest_items();
    let saved = named(&items, linked);
    assert_eq!(saved.len(), 1);
    let links = h.sim.stored_links(saved[0].item_id).expect("linked child prims").clone();
    let child = &links[0];
    let script = child.iter().find(|i| i.asset_type == 10).expect("child script");
    assert_eq!(script.everyone_mask, perm::COPY, "the late child's script was changed");
    let pebble = child.iter().find(|i| i.asset_type == AT_OBJECT).expect("pebble");
    assert_eq!(pebble.everyone_mask, perm::COPY, "the late child's object was opened and saved");
    assert_eq!(h.rezzed_left(), 0);
}
