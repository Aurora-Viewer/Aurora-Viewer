//! Scripts in the queue: visibility, price, destination, one at a time and
//! the answer of each request.

use super::*;
use aurora_net::{
    NetCommand,
    inventory::{InvFolder, InvItem},
};

const SCRIPT: &str = "default { state_entry() { llSay(0, \"x\"); } }";

fn script(name: &str) -> Entry {
    Entry::new(
        PathBuf::from(format!("{name}.lsl")),
        Ok(Source::Script(Draft::new(SCRIPT.to_owned()))),
        Default::default(),
    )
}

fn texture() -> Entry {
    Entry::new(
        "photo.png".into(),
        Ok(Source::Texture(super::super::tests::image(16, 16))),
        Default::default(),
    )
}

fn prices() -> Prices {
    Prices {
        texture: Some(10),
        large_texture: Some(40),
        sound: Some(10),
        animation: Some(10),
    }
}

fn world(scripts: Uuid) -> crate::world::World {
    let mut world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    world.balance = Some(1000);
    world.inventory.root = Uuid::from_u128(1);
    world.inventory.created_folders(vec![
        InvFolder {
            id: world.inventory.root,
            name: "Inventory".into(),
            type_default: 8,
            ..Default::default()
        },
        InvFolder {
            id: scripts,
            parent: world.inventory.root,
            name: "Scripts".into(),
            type_default: FOLDER_TYPE,
            ..Default::default()
        },
    ]);
    world
}

/// The entries admitted as a file picker's answer does, so that they get their destination.
fn load(imports: &mut Imports, world: &crate::world::World, entries: Vec<Entry>) {
    let request = Uuid::new_v4();
    imports.pending_files = Some(request);
    imports.tx.send(Worker::Loaded(request, entries)).expect("loaded files");
    poll(imports, world);
}

fn poll(imports: &mut Imports, world: &crate::world::World) -> Vec<NetCommand> {
    let (jobs, _results) = crate::scene::jobs::Jobs::new();
    imports.poll(&egui::Context::default(), &jobs, world, &Default::default())
}

/// The (request, parent, name) of each script creation among `commands`.
fn created(commands: &[NetCommand]) -> Vec<(Uuid, Uuid, String)> {
    commands
        .iter()
        .filter_map(|command| match command {
            NetCommand::CreateScriptItem { request, parent, name, .. } => Some((*request, *parent, name.clone())),
            _ => None,
        })
        .collect()
}

fn item(name: &str) -> InvItem {
    InvItem {
        id: Uuid::new_v4(),
        asset_id: Uuid::new_v4(),
        name: name.to_owned(),
        asset_type: 10,
        inv_type: 10,
        ..Default::default()
    }
}

#[test]
fn only_the_unified_queue_lists_a_script() {
    let entry = script("Door");
    for choice in [
        Choice::Texture,
        Choice::Sound,
        Choice::Animation,
        Choice::Material,
        Choice::Mesh,
        Choice::Local,
    ] {
        assert!(!choice.accepts(&entry), "{choice:?}");
    }
    assert!(Choice::Bulk.accepts(&entry));
    assert_eq!(Imports::type_choice(&entry), Choice::Bulk);

    // no type icon filters it, whatever they are set to
    let imports = Imports {
        entries: vec![script("Door"), texture()],
        type_filters: [false; 5],
        ..Default::default()
    };
    assert_eq!(imports.visible_indices(), vec![0]);
}

#[test]
fn a_script_adds_nothing_to_the_quote() {
    let mut imports = Imports {
        entries: vec![script("A"), script("B")],
        ..Default::default()
    };
    assert_eq!(imports.total(&prices()), Some(0));
    assert_eq!(imports.total(&Prices::default()), Some(0), "free even when prices are unknown");

    imports.entries.push(texture());
    let alone = Imports {
        entries: vec![texture()],
        ..Default::default()
    };
    assert_eq!(imports.total(&prices()), Some(10));
    assert_eq!(imports.total(&prices()), alone.total(&prices()));
    assert_eq!(imports.total(&Prices::default()), None, "the texture still needs its price");

    // an unselected script changes nothing; a failed one stops the quote like any file
    imports.entries[0].selected = false;
    assert_eq!(imports.total(&prices()), Some(10));
    imports.entries[1].status = Status::Error("failed".into());
    assert_eq!(imports.total(&prices()), None);
}

#[test]
fn a_script_batch_queues_free_text_steps_for_the_scripts_folder() {
    let scripts = Uuid::from_u128(2);
    let world = world(scripts);
    let mut imports = Imports::default();
    load(&mut imports, &world, vec![script("Door"), script("Door"), texture()]);
    assert!(
        imports.entries.iter().take(2).all(|entry| entry.folder == scripts),
        "the Scripts folder"
    );

    imports.start(&world, &Default::default(), &Default::default(), &prices());
    assert!(imports.message.is_empty(), "{}", imports.message);
    let steps: Vec<_> = imports.queue.iter().collect();
    assert_eq!(steps.len(), 3);
    for step in steps.iter().filter(|step| matches!(step.source, Source::Script(_))) {
        assert_eq!(step.kind, None);
        assert_eq!(step.cost, 0);
        assert_eq!(step.folder, scripts);
        assert_eq!(step.name, "Door");
    }
    assert_eq!(steps[2].kind, Some(Kind::Texture));
    assert_eq!(steps[2].cost, 10);
    assert!(imports.entries.iter().all(|entry| matches!(entry.status, Status::Preparing)));
}

#[test]
fn a_script_only_batch_needs_no_balance() {
    let scripts = Uuid::from_u128(2);
    let mut world = world(scripts);
    world.balance = None;
    let mut imports = Imports::default();
    load(&mut imports, &world, vec![script("A")]);
    assert_eq!(imports.total(&prices()), Some(0));
    imports.start(&world, &Default::default(), &Default::default(), &prices());
    assert_eq!(imports.queue.len(), 1, "{}", imports.message);
}

/// A successful answer: the saved item, None when the request is not the current script's.
fn saved(imports: &mut Imports, request: Uuid, item: InvItem) -> Option<InvItem> {
    match imports.script_saved(request, Uuid::nil(), Ok(item)) {
        Some(Answer::Saved(item)) => Some(*item),
        _ => None,
    }
}

/// The status of a row, None once its upload is acknowledged and the row retired.
fn status(imports: &Imports, id: Uuid) -> Option<Status> {
    imports
        .entries
        .iter()
        .find(|entry| entry.id == id)
        .map(|entry| entry.status.clone())
}

#[test]
fn scripts_with_one_name_are_created_one_at_a_time_and_answered_by_request() {
    let scripts = Uuid::from_u128(2);
    let world = world(scripts);
    let mut imports = Imports::default();
    load(&mut imports, &world, vec![script("Door"), script("Door"), script("Door")]);
    let ids: Vec<_> = imports.entries.iter().map(|entry| entry.id).collect();
    imports.start(&world, &Default::default(), &Default::default(), &prices());

    let first = created(&poll(&mut imports, &world));
    assert_eq!(first.len(), 1, "one creation in flight at a time");
    let (first_request, parent, name) = first[0].clone();
    assert_eq!((parent, name.as_str()), (scripts, "Door"));
    assert!(poll(&mut imports, &world).is_empty(), "the next one waits for the answer");
    assert!(imports.owns_script(first_request));

    // answers for other requests, a recovered script's for instance, are not ours
    let stranger = Uuid::new_v4();
    assert!(!imports.owns_script(stranger));
    imports.script_compile_errors(stranger, vec!["(1, 1) ERROR".into()]);
    assert!(saved(&mut imports, stranger, item("Door")).is_none());
    assert!(imports.receipts.is_empty());
    assert!(imports.owns_script(first_request));

    // ours, with compile errors: kept in the inventory, with its errors
    imports.script_compile_errors(first_request, vec!["(3, 9) : ERROR : Syntax error".into()]);
    let item_saved = saved(&mut imports, first_request, item("Door")).expect("the item");
    assert_eq!(imports.receipts.len(), 1);
    assert_eq!(
        imports.receipts[0].outcome,
        Outcome::CompileErrors(vec!["(3, 9) : ERROR : Syntax error".into()])
    );
    assert!(matches!(status(&imports, ids[0]), Some(Status::Done(id)) if id == item_saved.id));
    assert!(saved(&mut imports, first_request, item("Door")).is_none(), "answered once");

    // the second has its own request, and the first one's errors do not leak into it
    let second = created(&poll(&mut imports, &world));
    assert_eq!(second.len(), 1);
    assert_ne!(second[0].0, first_request);
    assert!(status(&imports, ids[0]).is_none(), "the acknowledged row is retired");
    saved(&mut imports, second[0].0, item("Door")).expect("the item");
    assert_eq!(imports.receipts[1].outcome, Outcome::Saved);
    assert!(matches!(status(&imports, ids[1]), Some(Status::Done(_))));
    assert!(matches!(status(&imports, ids[2]), Some(Status::Preparing)));

    let third = created(&poll(&mut imports, &world));
    assert_eq!(third.len(), 1);
    saved(&mut imports, third[0].0, item("Door")).expect("the item");
    poll(&mut imports, &world);
    assert!(!imports.busy());
    assert_eq!(imports.receipts.len(), 3);
    assert!(imports.entries.is_empty());
}

#[test]
fn a_script_that_cannot_be_saved_fails_alone() {
    let scripts = Uuid::from_u128(2);
    let world = world(scripts);
    let mut imports = Imports::default();
    load(&mut imports, &world, vec![script("Bad"), script("Good"), texture()]);
    let ids: Vec<_> = imports.entries.iter().map(|entry| entry.id).collect();
    imports.start(&world, &Default::default(), &Default::default(), &prices());

    let request = created(&poll(&mut imports, &world))[0].0;
    assert!(
        imports.script_saved(request, Uuid::nil(), Err("The server did not confirm the save.".into()))
            == Some(Answer::Failed { orphan: None })
    );
    assert!(matches!(status(&imports, ids[0]), Some(Status::Error(reason)) if reason.contains("confirm")));
    assert!(matches!(status(&imports, ids[1]), Some(Status::Preparing)), "the batch goes on");
    assert!(
        matches!(status(&imports, ids[2]), Some(Status::Preparing)),
        "other types are not blocked"
    );
    assert!(matches!(&imports.receipts[0].outcome, Outcome::Failed(reason) if reason.contains("confirm")));
    assert_eq!(imports.queue.len(), 2);

    // the next script is created, and the texture follows it
    let next = created(&poll(&mut imports, &world));
    assert_eq!(next.len(), 1);
    saved(&mut imports, next[0].0, item("Good")).expect("the item");
    assert!(matches!(status(&imports, ids[1]), Some(Status::Done(_))));
    poll(&mut imports, &world);
    assert!(imports.current.as_ref().is_some_and(|(_, step)| step.kind == Some(Kind::Texture)));
}

#[test]
fn a_new_batch_clears_the_last_receipts() {
    let scripts = Uuid::from_u128(2);
    let world = world(scripts);
    let mut imports = Imports::default();
    imports.receipts.push(Receipt {
        name: "Old".into(),
        outcome: Outcome::Saved,
    });
    load(&mut imports, &world, vec![script("New")]);
    imports.start(&world, &Default::default(), &Default::default(), &prices());
    assert!(imports.receipts.is_empty());
}

#[test]
fn cancelling_keeps_the_script_in_flight_and_drops_the_rest() {
    let scripts = Uuid::from_u128(2);
    let world = world(scripts);
    let mut imports = Imports::default();
    load(&mut imports, &world, vec![script("A"), script("B")]);
    imports.start(&world, &Default::default(), &Default::default(), &prices());
    let request = created(&poll(&mut imports, &world))[0].0;
    imports.cancel_remaining();
    assert!(imports.owns_script(request), "an in-flight creation runs to its answer");
    assert!(imports.queue.is_empty());
    assert!(saved(&mut imports, request, item("A")).is_some());
    assert!(matches!(imports.entries[0].status, Status::Done(_)));
    assert!(matches!(imports.entries[1].status, Status::Cancelled));
    assert!(poll(&mut imports, &world).is_empty());
}

#[test]
fn a_script_item_the_grid_made_but_could_not_fill_is_reported_as_an_orphan() {
    let scripts = Uuid::from_u128(2);
    let world = world(scripts);
    let mut imports = Imports::default();
    load(&mut imports, &world, vec![script("Door"), script("Door")]);
    imports.start(&world, &Default::default(), &Default::default(), &prices());
    let first = created(&poll(&mut imports, &world))[0].0;
    let orphan = Uuid::from_u128(500);
    assert_eq!(
        imports.script_saved(first, orphan, Err("Saving failed.".into())),
        Some(Answer::Failed { orphan: Some(orphan) }),
        "the item the grid created is handed back, to be trashed"
    );
    assert!(matches!(&imports.receipts[0].outcome, Outcome::Failed(_)));
    let second = created(&poll(&mut imports, &world))[0].0;
    assert_eq!(
        imports.script_saved(second, Uuid::nil(), Err("No simulator.".into())),
        Some(Answer::Failed { orphan: None }),
        "nothing was created, nothing to trash"
    );
    // not the current request: nothing is reported, nothing is trashed
    assert_eq!(imports.script_saved(first, orphan, Err("late".into())), None);
}
