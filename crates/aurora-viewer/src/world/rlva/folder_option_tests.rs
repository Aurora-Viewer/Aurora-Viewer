//! The option of the folder locks (`@detachallthis:<option>=n` and the
//! seven others) is checked when the command is received, as
//! RlvHandler::onAddRemFolderLock and onAddRemFolderLockException do
//! (rlvhandler.cpp): an option that names nothing fails with
//! RLV_RET_FAILED_OPTION and no rule is stored (issue #4291, rules of folders
//! that no longer exist listed under Preferences > RLVa > Active restrictions).

use super::inventory::{Lookup, lookup};
use super::{Param, Rlva, Verdict, folder_verdict};
use crate::world::inventory::{FetchState, Folder, Inventory};
use aurora_net::{ChatMessage, ChatSourceType, ChatType, inventory::InvFolder};
use std::time::{Duration, Instant};
use uuid::Uuid;

const ROOT: Uuid = Uuid::from_u128(1);
const ISSUER: Uuid = Uuid::from_u128(0xC011);
const OTHER: Uuid = Uuid::from_u128(0xC012);

fn add(inv: &mut Inventory, id: u128, parent: Uuid, name: &str, state: FetchState) -> Uuid {
    let id = Uuid::from_u128(id);
    inv.folders.insert(
        id,
        Folder {
            info: InvFolder {
                id,
                parent,
                name: name.into(),
                ..Default::default()
            },
            children: vec![],
            items: vec![],
            state,
            library: false,
        },
    );
    if let Some(parent) = inv.folders.get_mut(&parent) {
        parent.children.push(id);
    }
    id
}

/// The inventory root with `#RLV` holding `~Body` (with `Mesh` and `Skin
/// Tone`), `Outfit`, `Cuffs` and the hidden `.Secret`, all loaded.
fn shared() -> Inventory {
    let mut inv = Inventory {
        root: ROOT,
        ..Default::default()
    };
    add(&mut inv, 1, Uuid::nil(), "My Inventory", FetchState::Fetched);
    let rlv = add(&mut inv, 2, ROOT, "#RLV", FetchState::Fetched);
    let body = add(&mut inv, 3, rlv, "~Body", FetchState::Fetched);
    add(&mut inv, 4, body, "Mesh", FetchState::Fetched);
    add(&mut inv, 5, body, "Skin Tone", FetchState::Fetched);
    add(&mut inv, 6, rlv, "Outfit", FetchState::Fetched);
    add(&mut inv, 7, rlv, "Cuffs", FetchState::Fetched);
    add(&mut inv, 8, rlv, ".Secret", FetchState::Fetched);
    inv
}

fn chat(source: Uuid, text: &str) -> ChatMessage {
    ChatMessage {
        from_name: "Folder lock test object".to_owned(),
        source_id: source,
        owner_id: Uuid::from_u128(0xA6E),
        source_type: ChatSourceType::Object,
        chat_type: ChatType::OwnerSay,
        region: None,
        position: glam::Vec3::ZERO,
        message: text.to_owned(),
    }
}

fn enabled() -> Rlva {
    let mut rlva = Rlva::default();
    rlva.configure(true);
    rlva
}

/// The rules an issuer holds after `text`, read as the Active restrictions
/// list shows them.
fn send(rlva: &mut Rlva, inv: &Inventory, source: Uuid, text: &str) -> Vec<String> {
    assert!(rlva.receive_in(&chat(source, text), &|path| lookup(inv, path)));
    rlva.restrictions()
        .into_iter()
        .filter(|(id, _)| *id == source)
        .map(|(_, rule)| rule)
        .collect()
}

fn verdict(inv: &Inventory, text: &str) -> Verdict {
    let command = super::Command::parse(text.trim_start_matches('@')).expect("a command");
    assert!(matches!(command.param, Param::Add | Param::Remove));
    folder_verdict(&command, &|path| lookup(inv, path), false)
}

#[test]
fn rlva_folder_lock_of_an_unknown_folder_is_refused_and_listed_nowhere() {
    let inv = shared();
    let mut rlva = enabled();
    for text in [
        "@detachallthis:rlvunknown=n",
        "@detachthis:rlvunknown=n",
        "@attachthis:rlvunknown=n",
        "@attachallthis:rlvunknown=n",
        "@detachallthis:~body2 - (pe)=n",
        "@detachallthis:~body/nothing=n",
    ] {
        assert_eq!(send(&mut rlva, &inv, ISSUER, text), Vec::<String>::new(), "{text}");
    }
    assert!(rlva.restrictions().is_empty());
    assert!(!rlva.inventory_locked());
    // @getstatus lists nothing for it either
    assert_eq!(rlva.status(Some(ISSUER), ""), "");
}

#[test]
fn rlva_folder_lock_of_an_existing_folder_is_kept_exact_partial_and_case_insensitive() {
    let inv = shared();
    for text in [
        "@detachallthis:outfit=n",
        "@detachallthis:OUTFIT=n",
        "@detachallthis:~body/mesh=n",
        "@detachallthis:~body/skin=n",
        "@detachallthis:cuff=n",
        "@detachthis:uff=n",
        "@attachallthis:/outfit/=n",
        "@attachthis:~BODY/Skin Tone=n",
    ] {
        assert_eq!(verdict(&inv, &text.to_lowercase()), Verdict::Valid, "{text}");
    }
    let mut rlva = enabled();
    assert_eq!(
        send(&mut rlva, &inv, ISSUER, "@detachallthis:Outfit=n"),
        ["detachallthis:outfit".to_owned()]
    );
}

#[test]
fn rlva_folder_lock_path_matching_is_the_legacy_one_exact_first_then_the_first_visible_partial() {
    let mut inv = shared();
    let rlv = Uuid::from_u128(2);
    // "cuffs" is an exact match even though "Cuffs Pair" comes first and contains it
    add(&mut inv, 20, rlv, "Cuffs Pair", FetchState::Fetched);
    inv.folders.get_mut(&rlv).unwrap().children.rotate_right(1);
    assert_eq!(lookup(&inv, "cuffs"), Lookup::Found);
    // a hidden folder is never a partial match, only an exact one
    assert_eq!(lookup(&inv, "secret"), Lookup::Missing);
    assert_eq!(lookup(&inv, ".secret"), Lookup::Found);
    assert_eq!(verdict(&inv, "detachallthis:secret=n"), Verdict::Refuse);
    assert_eq!(verdict(&inv, "detachallthis:.secret=n"), Verdict::Valid);
    // a path of only separators is the shared root itself (getSharedFolder)
    assert_eq!(lookup(&inv, "//"), Lookup::Found);
    assert_eq!(lookup(&inv, "mesh"), Lookup::Missing, "a partial name matches one level only");
}

#[test]
fn rlva_folder_lock_attachment_points_and_wearable_types_are_kept_for_the_lock_forms() {
    let inv = shared();
    for text in [
        "detachallthis:chest=n",
        "detachthis:left hand=n",
        "attachallthis:skull=n",
        "attachthis:root=n",
        "detachallthis:shirt=n",
        "detachthis:pants=n",
        "attachthis:alpha=n",
        "attachallthis:physics=n",
    ] {
        assert_eq!(verdict(&inv, text), Verdict::Valid, "{text}");
    }
    // the option of a lock form may be empty: the object's own folder
    let mut rlva = enabled();
    assert_eq!(send(&mut rlva, &inv, ISSUER, "@detachallthis=n"), ["detachallthis".to_owned()]);
}

#[test]
fn rlva_folder_lock_exceptions_take_an_existing_folder_only() {
    let inv = shared();
    for text in [
        "detachallthis_except:chest=n",
        "detachthis_except:shirt=n",
        "attachthis_except:skull=n",
        "attachallthis_except:alpha=n",
        "detachallthis_except=n",
        "detachthis_except:rlvunknown=n",
        "attachallthis_except:00000000-0000-0000-0000-000000000009=n",
    ] {
        assert_eq!(verdict(&inv, text), Verdict::Refuse, "{text}");
    }
    for text in [
        "detachallthis_except:outfit=n",
        "detachthis_except:~body/mesh=n",
        "attachthis_except:cuff=n",
        "attachallthis_except:outfit=y",
    ] {
        assert_eq!(verdict(&inv, text), Verdict::Valid, "{text}");
    }
    let mut rlva = enabled();
    assert!(send(&mut rlva, &inv, ISSUER, "@detachallthis_except:chest=n,detachthis_except:nowhere=n").is_empty());
    assert_eq!(
        send(&mut rlva, &inv, ISSUER, "@detachallthis_except:outfit=n"),
        ["detachallthis_except:outfit".to_owned()]
    );
}

#[test]
fn rlva_folder_lock_refuses_a_uuid_and_dot_components_even_when_a_folder_is_named_so() {
    let mut inv = shared();
    let uuid = "12345678-1234-1234-1234-123456789abc";
    add(&mut inv, 30, Uuid::from_u128(2), uuid, FetchState::Fetched);
    // RlvCommandOptionHelper::parseOption reads a UUID before it looks for a folder
    assert_eq!(verdict(&inv, &format!("detachallthis:{uuid}=n")), Verdict::Refuse);
    assert_eq!(verdict(&inv, "detachallthis:../outfit=n"), Verdict::Refuse);
    assert_eq!(verdict(&inv, "detachallthis:./outfit=n"), Verdict::Refuse);
}

#[test]
fn rlva_folder_lock_release_with_an_unknown_folder_is_refused_too() {
    let inv = shared();
    let mut rlva = enabled();
    send(&mut rlva, &inv, ISSUER, "@detachallthis:outfit=n");
    // =y of a folder that is gone fails like =n: the stored rule stays until @clear
    let rules = send(&mut rlva, &inv, ISSUER, "@detachallthis:rlvunknown=y");
    assert_eq!(rules, ["detachallthis:outfit".to_owned()]);
    assert!(send(&mut rlva, &inv, ISSUER, "@detachallthis:outfit=y").is_empty());
}

#[test]
fn rlva_folder_lock_without_a_shared_root_is_refused_once_the_inventory_root_is_loaded() {
    let mut inv = shared();
    inv.folders.remove(&Uuid::from_u128(2));
    inv.folders.get_mut(&ROOT).unwrap().children.clear();
    assert_eq!(lookup(&inv, "outfit"), Lookup::Missing);
    assert_eq!(verdict(&inv, "detachallthis:outfit=n"), Verdict::Refuse);
    // no path to resolve: the object's own folder, a point and a type stay valid
    assert_eq!(verdict(&inv, "detachallthis=n"), Verdict::Valid);
    assert_eq!(verdict(&inv, "detachallthis:chest=n"), Verdict::Valid);
    // the inventory root not loaded yet: wait for it
    inv.folders.get_mut(&ROOT).unwrap().state = FetchState::Fetching;
    assert_eq!(lookup(&inv, "outfit"), Lookup::Loading(ROOT));
    assert_eq!(verdict(&inv, "detachallthis:outfit=n"), Verdict::Wait(ROOT));
    // no inventory at all (before login)
    assert_eq!(lookup(&Inventory::default(), "outfit"), Lookup::Loading(Uuid::nil()));
}

#[test]
fn rlva_folder_lock_waits_for_the_folders_it_names_and_decides_when_they_load() {
    let mut inv = shared();
    let rlv = Uuid::from_u128(2);
    let body = Uuid::from_u128(3);
    // the contents of ~Body have not arrived: "mesh" may be inside, "outfit" is settled above it
    inv.folders.get_mut(&body).unwrap().children.clear();
    inv.folders.get_mut(&body).unwrap().state = FetchState::Unknown;
    assert_eq!(lookup(&inv, "~body"), Lookup::Found);
    assert_eq!(lookup(&inv, "~body/mesh"), Lookup::Loading(body));
    // a partial name at an unloaded level waits too: an exact one may still come
    let mut partial = shared();
    partial.folders.get_mut(&rlv).unwrap().state = FetchState::Fetching;
    assert_eq!(lookup(&partial, "cuff"), Lookup::Loading(rlv));
    assert_eq!(lookup(&partial, "cuffs"), Lookup::Found);
    // a folder that failed to load cannot be told: the command is kept as before
    inv.folders.get_mut(&body).unwrap().state = FetchState::Failed;
    assert_eq!(lookup(&inv, "~body/mesh"), Lookup::Unverified);
    assert_eq!(verdict(&inv, "detachallthis:~body/mesh=n"), Verdict::Valid);

    let mut rlva = enabled();
    inv.folders.get_mut(&body).unwrap().state = FetchState::Unknown;
    // received while ~Body is unloaded: held, not stored, not refused
    assert!(rlva.receive_in(&chat(ISSUER, "@detachallthis:~body/mesh=n"), &|path| lookup(&inv, path)));
    assert!(rlva.restrictions().is_empty());
    assert_eq!(rlva.deferred.len(), 1);
    // nothing changed: it keeps waiting, and names the folder to fetch
    let (wanted, ran) = rlva.release_deferred(Instant::now(), &|path| lookup(&inv, path));
    assert_eq!((wanted, ran), (vec![body], false));
    // ~Body arrives with its folders: the held command runs
    let mesh = Uuid::from_u128(4);
    inv.folders.get_mut(&body).unwrap().children.push(mesh);
    inv.folders.get_mut(&body).unwrap().state = FetchState::Fetched;
    let (wanted, ran) = rlva.release_deferred(Instant::now(), &|path| lookup(&inv, path));
    assert!(wanted.is_empty() && ran);
    assert!(rlva.deferred.is_empty());
    assert_eq!(
        rlva.restrictions().into_iter().map(|(_, rule)| rule).collect::<Vec<_>>(),
        ["detachallthis:~body/mesh".to_owned()]
    );
}

#[test]
fn rlva_folder_lock_held_command_refuses_when_the_loaded_folders_do_not_have_it() {
    let mut inv = shared();
    let body = Uuid::from_u128(3);
    inv.folders.get_mut(&body).unwrap().children.clear();
    inv.folders.get_mut(&body).unwrap().state = FetchState::Fetching;
    let mut rlva = enabled();
    rlva.receive_in(&chat(ISSUER, "@detachallthis:~body/gone=n"), &|path| lookup(&inv, path));
    assert_eq!(rlva.deferred.len(), 1);
    inv.folders.get_mut(&body).unwrap().state = FetchState::Fetched;
    let (_, ran) = rlva.release_deferred(Instant::now(), &|path| lookup(&inv, path));
    assert!(ran);
    assert!(rlva.restrictions().is_empty(), "the folder never existed: nothing is stored");
}

#[test]
fn rlva_folder_lock_keeps_the_order_of_an_issuer_behind_its_held_command_only() {
    let mut inv = shared();
    let body = Uuid::from_u128(3);
    inv.folders.get_mut(&body).unwrap().children.clear();
    inv.folders.get_mut(&body).unwrap().state = FetchState::Fetching;
    let mut rlva = enabled();
    rlva.receive_in(
        &chat(ISSUER, "@detachallthis:~body/mesh=n,detachallthis:outfit=n,sendchat=n"),
        &|path| lookup(&inv, path),
    );
    // another issuer is not held up
    let other = send(&mut rlva, &inv, OTHER, "@detachallthis:outfit=n");
    assert_eq!(other, ["detachallthis:outfit".to_owned()]);
    assert!(
        rlva.restrictions().iter().all(|(id, _)| *id == OTHER),
        "nothing of the first issuer ran"
    );
    assert_eq!(rlva.deferred.len(), 3);
    let mesh = Uuid::from_u128(4);
    inv.folders.get_mut(&body).unwrap().children.push(mesh);
    inv.folders.get_mut(&body).unwrap().state = FetchState::Fetched;
    rlva.release_deferred(Instant::now(), &|path| lookup(&inv, path));
    let issued: Vec<_> = rlva
        .restrictions()
        .into_iter()
        .filter(|(id, _)| *id == ISSUER)
        .map(|(_, rule)| rule)
        .collect();
    assert_eq!(issued.len(), 3);
    assert!(issued.iter().any(|rule| rule == "detachallthis:~body/mesh"));
    // a source that goes away loses what it held
    rlva.receive_in(&chat(Uuid::from_u128(0xC013), "@detachallthis:~nowhere/x=n"), &|_| {
        Lookup::Loading(body)
    });
    assert_eq!(rlva.deferred.len(), 1);
    rlva.forget(Uuid::from_u128(0xC013));
    assert!(rlva.deferred.is_empty());
}

#[test]
fn rlva_folder_lock_stops_waiting_after_a_minute_and_keeps_the_rule_as_before() {
    let inv = Inventory::default();
    let mut rlva = enabled();
    rlva.receive_in(&chat(ISSUER, "@detachallthis:outfit=n"), &|path| lookup(&inv, path));
    assert_eq!(rlva.deferred.len(), 1);
    let (_, ran) = rlva.release_deferred(Instant::now() + Duration::from_secs(30), &|path| lookup(&inv, path));
    assert!(!ran);
    let (_, ran) = rlva.release_deferred(Instant::now() + Duration::from_secs(61), &|path| lookup(&inv, path));
    assert!(ran);
    assert_eq!(
        rlva.restrictions().into_iter().map(|(_, rule)| rule).collect::<Vec<_>>(),
        ["detachallthis:outfit".to_owned()]
    );
}

#[test]
fn rlva_folder_lock_global_disable_discards_the_held_commands() {
    let inv = Inventory::default();
    let mut rlva = enabled();
    rlva.receive_in(&chat(ISSUER, "@detachallthis:outfit=n"), &|path| lookup(&inv, path));
    assert_eq!(rlva.deferred.len(), 1);
    rlva.configure(false);
    assert!(rlva.deferred.is_empty());
}

#[test]
fn rlva_folder_lock_commands_of_the_world_are_checked_against_its_inventory() {
    let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        world.apply(event);
    }
    world.rlva_configure(true);
    world.inventory = shared();
    assert!(world.rlva_receive(&chat(ISSUER, "@detachallthis:outfit=n,detachallthis:rlvunknown=n")));
    let rules: Vec<_> = world.rlva.restrictions().into_iter().map(|(_, rule)| rule).collect();
    assert_eq!(rules, ["detachallthis:outfit".to_owned()]);
    // an inventory that is still loading holds the command, and the tick asks for the folder
    let rlv = Uuid::from_u128(2);
    world.inventory.folders.get_mut(&rlv).unwrap().state = FetchState::Unknown;
    assert!(world.rlva_receive(&chat(OTHER, "@detachallthis:cuff=n")));
    assert!(world.rlva.restrictions().iter().all(|(id, _)| *id != OTHER));
    assert!(world.inventory.queue.iter().any(|(id, _)| *id == rlv));
    world.inventory.folders.get_mut(&rlv).unwrap().state = FetchState::Fetched;
    world.rlva_deferred_tick(Instant::now());
    assert!(
        world
            .rlva
            .restrictions()
            .iter()
            .any(|(id, rule)| *id == OTHER && rule == "detachallthis:cuff")
    );
}
