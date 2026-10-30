//! Take destination: the folder the objects were rezzed from, like
//! handle_take (indra/newview/llviewermenu.cpp).

use super::*;
use crate::world::inventory::{FetchState, Folder, Inventory};
use aurora_net::inventory::InvFolder;
use std::sync::Arc;

const TRASH: u128 = 3;

fn folder(inv: &mut Inventory, id: u128, parent: u128, t: i32, library: bool) {
    inv.folders.insert(
        Uuid::from_u128(id),
        Folder {
            info: InvFolder {
                id: Uuid::from_u128(id),
                parent: Uuid::from_u128(parent),
                name: format!("Folder {id}"),
                type_default: t,
                version: 1,
                ..Default::default()
            },
            children: Vec::new(),
            items: Vec::new(),
            state: FetchState::Unknown,
            library,
        },
    );
}

fn inventory() -> Inventory {
    let mut inv = Inventory::default();
    folder(&mut inv, 1, 0, 8, false);
    folder(&mut inv, 2, 1, -1, false); // Builds
    folder(&mut inv, TRASH, 1, 14, false);
    folder(&mut inv, 4, TRASH, -1, false); // trashed
    folder(&mut inv, 5, 1, -1, false); // Other builds
    folder(&mut inv, 6, 0, -1, true); // Library
    inv
}

fn ids(ids: &[u128]) -> Vec<Uuid> {
    ids.iter().map(|&i| Uuid::from_u128(i)).collect()
}

fn take(folders: &[u128]) -> Option<Uuid> {
    take_folder(&inventory(), ids(folders), Uuid::from_u128(TRASH))
}

#[test]
fn objects_rezzed_from_one_folder_go_back_to_it() {
    assert_eq!(take(&[2]), Some(Uuid::from_u128(2)));
    assert_eq!(take(&[2, 2, 2]), Some(Uuid::from_u128(2)));
}

#[test]
fn an_unknown_rez_folder_does_not_count() {
    // handle_take skips a nil FolderID (an object rezzed by a script, or
    // whose properties have not arrived).
    assert_eq!(take(&[0, 2, 0]), Some(Uuid::from_u128(2)));
    assert_eq!(take(&[0, 0]), None);
    assert_eq!(take(&[]), None);
}

#[test]
fn objects_from_different_folders_go_to_objects() {
    assert_eq!(take(&[2, 5]), None);
}

#[test]
fn a_missing_trashed_or_library_folder_is_not_used() {
    assert_eq!(take(&[99]), None);
    assert_eq!(take(&[TRASH]), None);
    assert_eq!(take(&[4]), None);
    assert_eq!(take(&[6]), None);
}

#[test]
fn take_uses_the_rez_folder_and_take_copy_the_objects_folder() {
    let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events().into_iter().chain(crate::demo::selection_cycle_events()) {
        world.apply(event);
    }
    let objects = BuildTool::folder_of_type(&world, 6);
    let rezzed_from = world
        .inventory
        .folders
        .values()
        .find(|f| !f.library && f.info.type_default == -1 && f.info.id != objects)
        .map(|f| f.info.id)
        .expect("the demo inventory has user folders");
    let idx = world.objects.index_of_uuid(&crate::demo::action_id(99614)).unwrap();
    let full_id = world.objects.get(idx).unwrap().full_id;
    let mut tool = BuildTool::default();
    assert_eq!(tool.take_destination(&world, &[idx], false), objects);
    tool.props.insert(
        full_id,
        ObjectProps {
            folder_id: rezzed_from,
            ..Default::default()
        },
    );
    assert_eq!(tool.take_destination(&world, &[idx], false), rezzed_from);
    assert_eq!(tool.take_destination(&world, &[idx], true), objects);
}
