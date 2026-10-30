use super::*;
use crate::world::inventory::actions as rules;
use aurora_net::inventory::{InvFolder, operations::Operation};

fn fixture(
    mesh: bool,
) -> (
    Imports,
    crate::world::World,
    crate::ui::inventory::InventoryPreferences,
    Preferences,
    Prices,
) {
    let mut world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    world.inventory.root = Uuid::from_u128(1);
    world.inventory.created_folders(vec![InvFolder {
        id: world.inventory.root,
        name: "Inventory".into(),
        type_default: 8,
        ..Default::default()
    }]);
    world.balance = Some(1000);
    let source = if mesh {
        Source::Mesh(Box::new(crate::imports::tests::mesh_fixture()))
    } else {
        Source::Texture(crate::imports::tests::image(16, 16))
    };
    let mut entry = Entry::new("Chair.dae".into(), Ok(source), Default::default());
    entry.source_folders = vec!["Pack".into(), "Chair".into()];
    let mut imports = Imports {
        entries: vec![entry],
        ..Default::default()
    };
    let settings = Preferences {
        use_default_folders: false,
        folder_layout: Some(folders::Layout::Mirror),
        ..Default::default()
    };
    let inventory = Default::default();
    imports.sync_destinations(&settings, &world, &inventory);
    if mesh {
        quote(&mut imports, 20);
    }
    let prices = Prices {
        texture: Some(10),
        large_texture: Some(40),
        sound: Some(10),
        animation: Some(10),
    };
    (imports, world, inventory, settings, prices)
}
fn quote(imports: &mut Imports, cost: i32) {
    let Some(Source::Mesh(draft)) = &mut imports.entries[0].source else {
        panic!("mesh")
    };
    draft.quote = Some(aurora_net::inventory::mesh_upload::Quote {
        token: Uuid::new_v4(),
        cost,
        data: Default::default(),
    });
    draft.pending = None;
}
fn folders(imports: &mut Imports, world: &mut crate::world::World, inventory: &crate::ui::inventory::InventoryPreferences) {
    while let Some(command) = imports.next_import_folder(world, inventory) {
        assert!(
            imports.queue.is_empty() && imports.current.is_none(),
            "no paid transfer before folder acknowledgements"
        );
        let aurora_net::NetCommand::EditInventory { request, mut change } = command else {
            panic!("folder")
        };
        let Operation::Folder { id: original, parent, .. } = change.operations[0] else {
            panic!("folder")
        };
        assert!(world.inventory.folders.contains_key(&parent));
        let remapped = Uuid::new_v4();
        if let Operation::Folder { id, .. } = &mut change.operations[0] {
            *id = remapped;
        }
        let mut reply = rules::demo_mutate(&mut world.inventory, world.agent_id, change).unwrap();
        reply.created.insert(original, remapped);
        imports.import_folder_result(request, Ok(reply), &mut world.inventory);
    }
}

#[test]
fn confirmation_creates_acknowledged_folders_then_starts_uploads_without_another_action() {
    let (mut imports, mut world, inventory, settings, prices) = fixture(false);
    assert!(imports.folder_plan_valid(&world, &inventory));
    assert!(!imports.folder_plan_ready(&world, &inventory));
    assert!(imports.next_import_folder(&world, &inventory).is_none());
    imports.start_confirmed(&world, &inventory, &settings, &prices);
    assert!(imports.busy());
    assert_eq!(
        world.inventory.folders.len(),
        1,
        "confirmation schedules but does not invent acknowledgements"
    );
    folders(&mut imports, &mut world, &inventory);
    assert!(imports.advance_confirmed(&world, &inventory).is_empty());
    assert_eq!(imports.queue.len(), 1);
    assert_eq!(imports.queue[0].folder, imports.entries[0].folder);
    assert_ne!(imports.queue[0].folder, world.inventory.root);
    assert_eq!(imports.spent, 0);
}

#[test]
fn confirmed_mesh_requotes_real_destination_and_stops_if_the_approved_price_increases() {
    for new_price in [20, 21] {
        let (mut imports, mut world, inventory, settings, prices) = fixture(true);
        imports.start_confirmed(&world, &inventory, &settings, &prices);
        folders(&mut imports, &mut world, &inventory);
        let requests = imports.advance_confirmed(&world, &inventory);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].0, imports.entries[0].id);
        quote(&mut imports, new_price);
        imports.advance_confirmed(&world, &inventory);
        assert_eq!(imports.queue.len(), usize::from(new_price == 20));
        if new_price > 20 {
            assert!(!imports.message.is_empty() && !imports.busy());
        }
        assert_eq!(imports.spent, 0);
    }
}

#[test]
fn cancellation_failure_and_invalid_metadata_never_start_an_unconfirmed_transfer() {
    let (mut imports, mut world, inventory, settings, prices) = fixture(false);
    imports.entries[0].name.clear();
    imports.start_confirmed(&world, &inventory, &settings, &prices);
    assert!(!imports.busy() && imports.next_import_folder(&world, &inventory).is_none());
    imports.entries[0].name = "Chair".into();
    imports.start_confirmed(&world, &inventory, &settings, &prices);
    let aurora_net::NetCommand::EditInventory { request, .. } = imports.next_import_folder(&world, &inventory).unwrap() else {
        panic!("folder")
    };
    imports.import_folder_result(request, Err("failure".into()), &mut world.inventory);
    imports.advance_confirmed(&world, &inventory);
    assert!(!imports.busy() && imports.queue.is_empty());
    imports.start_confirmed(&world, &inventory, &settings, &prices);
    imports.cancel_remaining();
    imports.advance_confirmed(&world, &inventory);
    assert!(imports.next_import_folder(&world, &inventory).is_none());
    assert!(imports.queue.is_empty() && imports.spent == 0);
}
