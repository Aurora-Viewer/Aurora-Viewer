//! Shared inventory and wear parity with the Legacy Viewer's RLVa
//! (rlvinventory.cpp, rlvlocks.cpp/.h, rlvhelper.cpp, rlvhandler.cpp): each
//! test names the reference function it follows.

use super::Command;
use super::link_lock_tests::{Linked, folder, link, linked, object, wear};
use crate::world::World;
use crate::world::appearance::Action;
use aurora_llsd::llsd_map;
use aurora_net::inventory::InvItem;
use aurora_net::inventory::operations::{Mutation, Operation};
use aurora_net::{ChatMessage, ChatSourceType, ChatType};
use uuid::Uuid;

fn chat(source: Uuid, text: &str) -> ChatMessage {
    ChatMessage {
        from_name: "RLVa test object".to_owned(),
        source_id: source,
        owner_id: Uuid::nil(),
        source_type: ChatSourceType::Object,
        chat_type: ChatType::OwnerSay,
        region: None,
        position: glam::Vec3::ZERO,
        message: text.to_owned(),
    }
}

fn say(world: &mut World, source: Uuid, text: &str) {
    world.rlva.receive(&chat(source, text));
}

/// As the chat handler says it (`World::on_chat`): the folder lock options
/// are checked against the shared folders.
fn say_checked(world: &mut World, source: Uuid, text: &str) {
    let inventory = &world.inventory;
    let shared_folder = |path: &str| super::shared_folder_exists(inventory, path);
    world.rlva.receive_with(&chat(source, text), Some(&shared_folder));
}

fn query(world: &mut World, source: Uuid, text: &str) -> Option<String> {
    let command = Command::parse(text).expect("a query");
    world.rlva_inventory_query(&command, source)
}

fn rename(id: Uuid, name: &str) -> Mutation {
    Mutation {
        operations: vec![Operation::Patch {
            id,
            folder: true,
            body: llsd_map! { "name" => name },
        }],
        ..Default::default()
    }
}

fn delete(id: Uuid) -> Mutation {
    Mutation {
        operations: vec![Operation::Delete { id, folder: false }],
        ..Default::default()
    }
}

/// The `#RLV` folder of the fixture.
fn shared(l: &Linked) -> Uuid {
    l.world.rlva_shared_root().expect("#RLV")
}

/// RlvAttachPtLookup::initLookupTable (rlvlocks.cpp) names the points from
/// avatar_lad.xml, lowercased: points 54 and 55 are "left hind foot" and
/// "right hind foot", in every option and every folder name.
#[test]
fn hind_feet_go_by_their_avatar_lad_names() {
    use super::inventory::{POINTS, point};
    let lib = crate::scene::avatar::AvatarLibrary::load();
    for (id, at) in &lib.attach_points {
        let name = lib.attach_names.get(id).expect("a name").to_lowercase();
        assert_eq!(POINTS.get(usize::from(*id)).copied(), Some(name.as_str()), "point {id} ({at:?})");
    }
    assert_eq!(point("left hind foot"), Some(54));
    assert_eq!(point("Right Hind Foot"), Some(55));
    assert_eq!(point("hind left foot"), None, "not a point name of the reference");
    let mut l = linked();
    let issuer = l.issuer;
    say(&mut l.world, issuer, "@remattach:left hind foot=n");
    assert!(
        l.world
            .rlva
            .restrictions()
            .iter()
            .any(|(_, rule)| rule == "remattach:left hind foot"),
        "the lock is kept"
    );
    say(&mut l.world, issuer, "@remattach:hind left foot=n");
    assert!(
        !l.world
            .rlva
            .restrictions()
            .iter()
            .any(|(_, rule)| rule == "remattach:hind left foot"),
        "the old name is not an option"
    );
    let hind = Uuid::from_u128(0xC0F5);
    l.world.inventory.add_items(vec![object(0xC0F5, l.clothes)]);
    wear(&mut l.world, hind, 54);
    assert!(!l.world.rlva_can_remove(hind, Uuid::nil()));
    assert_eq!(query(&mut l.world, issuer, "getattach:left hind foot=1").as_deref(), Some("1"));
}

/// RlvFolderLocks::canMoveItem and canRemoveItem (rlvlocks.h) count the
/// shared path lock of @sharedwear and @sharedunwear: an item does not leave
/// #RLV, come into it or get deleted from it while one is set. The root lock
/// of the unshared twins is left out for items.
#[test]
fn shared_wear_locks_keep_items_in_place() {
    let mut l = linked();
    let (collar, clothes, spare, free, issuer) = (l.collar, l.clothes, l.spare, l.free, l.issuer);
    let nil = Uuid::nil();
    assert!(l.world.rlva_inventory_move_allowed(collar, clothes, nil), "nothing set");
    for lock in ["@sharedunwear", "@sharedwear"] {
        say(&mut l.world, issuer, &format!("{lock}=n"));
        assert!(!l.world.rlva_inventory_move_allowed(collar, clothes, nil), "{lock}: out of #RLV");
        assert!(!l.world.rlva_inventory_move_allowed(free, spare, nil), "{lock}: into #RLV");
        assert!(l.world.rlva_inventory_move_allowed(collar, spare, nil), "{lock}: within #RLV");
        assert!(!l.world.rlva_inventory_mutation_allowed(&delete(collar), nil), "{lock}: delete");
        assert!(l.world.rlva_inventory_mutation_allowed(&delete(l.new), nil), "{lock}: outside #RLV");
        say(&mut l.world, issuer, &format!("{lock}=y"));
    }
    for lock in ["@unsharedunwear", "@unsharedwear"] {
        say(&mut l.world, issuer, &format!("{lock}=n"));
        assert!(
            l.world.rlva_inventory_move_allowed(free, spare, nil),
            "{lock}: the root lock leaves items"
        );
        assert!(l.world.rlva_inventory_move_allowed(collar, clothes, nil), "{lock}");
        assert!(l.world.rlva_inventory_mutation_allowed(&delete(l.new), nil), "{lock}");
        say(&mut l.world, issuer, &format!("{lock}=y"));
    }
}

/// RlvFolderLocks::canRemoveFolder (rlvlocks.h), through
/// get_is_category_removable (llinventoryfunctions.cpp): the shared path lock
/// of @sharedwear and @sharedunwear, held on #RLV, keeps #RLV and its folders
/// from being deleted; the root lock of the unshared twins does not count.
#[test]
fn shared_wear_locks_keep_shared_folders() {
    let mut l = linked();
    let (spare, issuer) = (l.spare, l.issuer);
    let rlv = shared(&l);
    // (Clothes holds the body, whose link in #RLV @sharedunwear keeps on)
    let root = l.world.inventory.root;
    let outside = folder(&mut l.world, 0xF0F0, root, "Boxes", -1);
    // Delete in the inventory moves to the Trash (LLFolderBridge::removeItem)
    let trash = folder(&mut l.world, 0xF0F1, root, "Trash", 14);
    let nil = Uuid::nil();
    let binned = |w: &World, id| w.rlva_inventory_move_allowed(id, trash, nil);
    let gone = |w: &World, id| {
        w.rlva_inventory_mutation_allowed(
            &Mutation {
                operations: vec![Operation::Delete { id, folder: true }],
                ..Default::default()
            },
            nil,
        )
    };
    assert!(gone(&l.world, spare) && gone(&l.world, rlv), "nothing set");
    assert!(binned(&l.world, spare) && binned(&l.world, rlv), "nothing set: to the Trash");
    for lock in ["@sharedunwear", "@sharedwear"] {
        say(&mut l.world, issuer, &format!("{lock}=n"));
        assert!(!gone(&l.world, spare), "{lock}: a shared folder");
        assert!(!gone(&l.world, rlv), "{lock}: #RLV");
        assert!(gone(&l.world, outside), "{lock}: outside #RLV");
        assert!(!binned(&l.world, spare) && !binned(&l.world, rlv), "{lock}: to the Trash");
        assert!(binned(&l.world, outside), "{lock}: outside #RLV to the Trash");
        say(&mut l.world, issuer, &format!("{lock}=y"));
    }
    for lock in ["@unsharedunwear", "@unsharedwear"] {
        say(&mut l.world, issuer, &format!("{lock}=n"));
        assert!(gone(&l.world, spare), "{lock}: the root lock leaves folders");
        say(&mut l.world, issuer, &format!("{lock}=y"));
    }
}

/// RlvFolderLocks::canRenameFolder (rlvlocks.h): #RLV holds the shared path
/// lock of the four placement rules, so it keeps its name under any of them.
#[test]
fn rlv_keeps_its_name_under_the_placement_locks() {
    let mut l = linked();
    let (issuer, spare) = (l.issuer, l.spare);
    let root = shared(&l);
    let nil = Uuid::nil();
    assert!(l.world.rlva_inventory_mutation_allowed(&rename(root, "#RLVx"), nil), "nothing set");
    for lock in ["@sharedwear", "@sharedunwear", "@unsharedwear", "@unsharedunwear"] {
        say(&mut l.world, issuer, &format!("{lock}=n"));
        assert!(!l.world.rlva_inventory_mutation_allowed(&rename(root, "#RLVx"), nil), "{lock}");
        assert!(
            l.world.rlva_inventory_mutation_allowed(&rename(spare, "Other"), nil),
            "{lock}: a subfolder"
        );
        say(&mut l.world, issuer, &format!("{lock}=y"));
    }
}

fn item(id: u128, parent: Uuid, name: &str, kind: i32, flags: u32) -> InvItem {
    InvItem {
        id: Uuid::from_u128(id),
        parent,
        name: name.into(),
        asset_type: kind,
        flags,
        ..Default::default()
    }
}

/// A folder of `#RLV`.
fn outfit(l: &mut Linked, id: u128, name: &str) -> Uuid {
    let rlv = shared(l);
    folder(&mut l.world, id, rlv, name, -1)
}

/// Puts `item` in the Current Outfit folder: worn clothing or a body part.
fn put_on(l: &mut Linked, link_id: u128, item: Uuid) {
    let cof = l.cof;
    l.world.inventory.add_items(vec![link(link_id, cof, item)]);
}

fn force(l: &mut Linked, source: Uuid, text: &str) {
    let c = Command::parse(text).expect("a command");
    assert!(l.world.rlva_inventory_force(source, &c), "{text} runs now");
}

/// The removals and wears (item, replace, point) of the last forced batch.
fn batch(l: &Linked) -> (Vec<Uuid>, Vec<(Uuid, bool, u8)>) {
    match l.world.rlva.appearance.last() {
        Some((_, Action::Force { remove, wear })) => (remove.clone(), wear.iter().map(|w| (w.item, w.replace, w.point)).collect()),
        _ => (Vec::new(), Vec::new()),
    }
}

fn worn_items(l: &Linked) -> Vec<Uuid> {
    batch(l).1.into_iter().map(|(item, _, _)| item).collect()
}

/// RlvAttachmentLocks::canAttach and RlvWearableLocks::canWear (rlvlocks.h)
/// check the folder of the inventory entry worn: a link in a locked folder
/// is refused though its original sits elsewhere, and a link outside a
/// locked folder is worn though its original sits in it.
#[test]
fn a_link_is_worn_by_its_own_folder() {
    let mut l = linked();
    let (issuer, clothes, spare) = (l.issuer, l.clothes, l.spare);
    let nil = Uuid::nil();
    let dressed = outfit(&mut l, 0xE301, "Dressed");
    let shirt = Uuid::from_u128(0xD301);
    let shirt_link = Uuid::from_u128(0xD302);
    l.world
        .inventory
        .add_items(vec![item(0xD301, clothes, "Shirt", 5, 4), link(0xD302, dressed, shirt)]);
    say(&mut l.world, issuer, "@attachallthis:dressed=n");
    force(&mut l, issuer, "attach:dressed=force");
    assert!(
        !worn_items(&l).contains(&shirt),
        "RlvForceWear::forceFolder checks the link's folder"
    );
    assert!(l.world.rlva_can_add(shirt, 0, nil), "the original's own folder is free");
    l.world.rlva_note_wear_entry(shirt, shirt_link);
    assert!(!l.world.rlva_can_add(shirt, 0, nil), "worn through the link: its folder counts");
    l.world.rlva_note_wear_entry(shirt, shirt);
    assert!(l.world.rlva_can_add(shirt, 0, nil));
    say(&mut l.world, issuer, "@attachallthis:dressed=y");
    // the other way round: the original in #RLV, its link outside
    let kept = Uuid::from_u128(0xD303);
    let kept_link = Uuid::from_u128(0xD304);
    l.world
        .inventory
        .add_items(vec![item(0xD303, spare, "Pants", 5, 5), link(0xD304, clothes, kept)]);
    say(&mut l.world, issuer, "@sharedwear=n");
    assert!(!l.world.rlva_can_add(kept, 0, nil), "the original in #RLV");
    let wear = Action::WearItem {
        item: kept_link,
        replace: true,
        point: 0,
    };
    l.world.rlva_note_wear_request(&wear);
    assert!(l.world.rlva_can_add(kept, 0, nil), "worn through a link outside #RLV");
}

/// RlvWearableItemCollector::getWearActionNormal (rlvinventory.cpp): a
/// folder with no prefix of its own takes its parent's action, so a plain
/// folder under a "+" folder adds.
#[test]
fn plain_subfolders_take_the_wear_action_of_their_parent() {
    let mut l = linked();
    let base = outfit(&mut l, 0xE501, "Outfit");
    let extra = folder(&mut l.world, 0xE502, base, "+Extra", -1);
    let sub = folder(&mut l.world, 0xE503, extra, "Sub", -1);
    let plain = folder(&mut l.world, 0xE504, base, "Plain", -1);
    l.world
        .inventory
        .add_items(vec![item(0xD501, sub, "Shirt", 5, 4), item(0xD502, plain, "Pants", 5, 5)]);
    let collection = super::collection::collect(&l.world.inventory, base, true, true, true, &l.world.rlva.preferences);
    let action = |id: u128| collection.items.iter().find(|e| e.item == Uuid::from_u128(id)).map(|e| e.replace);
    assert_eq!(action(0xD501), Some(false), "+Extra/Sub adds");
    assert_eq!(action(0xD502), Some(true), "Plain keeps the base's replace");
}

/// LLInventoryModel::collectDescendentsIf with folder links followed
/// (llinventorymodel.cpp) and RlvWearableItemCollector::onCollectItem: a
/// folder link anywhere under the base is collected, in a plain or a hidden
/// subfolder too.
#[test]
fn folder_links_anywhere_under_the_base_are_followed() {
    let mut l = linked();
    let root = l.world.inventory.root;
    let base = outfit(&mut l, 0xE601, "Outfit");
    let sub = folder(&mut l.world, 0xE602, base, "Sub", -1);
    let hidden = folder(&mut l.world, 0xE603, base, ".hidden", -1);
    let external = folder(&mut l.world, 0xE604, root, "External", -1);
    let other = folder(&mut l.world, 0xE605, root, "Other", -1);
    let mut to_external = link(0xD601, sub, external);
    to_external.asset_type = 25;
    let mut to_other = link(0xD602, hidden, other);
    to_other.asset_type = 25;
    l.world.inventory.add_items(vec![
        to_external,
        to_other,
        item(0xD603, external, "Shirt", 5, 4),
        item(0xD604, other, "Pants", 5, 5),
    ]);
    for all in [false, true] {
        let collection = super::collection::collect(&l.world.inventory, base, all, true, true, &l.world.rlva.preferences);
        let items: Vec<Uuid> = collection.items.iter().map(|e| e.item).collect();
        assert!(items.contains(&Uuid::from_u128(0xD603)), "all {all}: through Sub");
        assert!(items.contains(&Uuid::from_u128(0xD604)), "all {all}: through .hidden");
    }
}

/// RlvForceWear::addAttachment (rlvhelper.cpp): a replacing object clears
/// what is pending on its point, so the last one of the folder stays.
#[test]
fn the_last_replacing_object_of_a_point_is_worn() {
    let mut l = linked();
    let issuer = l.issuer;
    let base = outfit(&mut l, 0xE701, "Outfit");
    l.world.inventory.add_items(vec![
        item(0xD701, base, "Wings A (spine)", 6, 0),
        item(0xD702, base, "Wings B (spine)", 6, 0),
    ]);
    force(&mut l, issuer, "attach:outfit=force");
    assert_eq!(batch(&l).1, vec![(Uuid::from_u128(0xD702), true, 9)]);
    l.world.rlva.appearance.clear();
    force(&mut l, issuer, "attachover:outfit=force");
    assert_eq!(batch(&l).1.len(), 2, "added, both stay");
}

/// RlvForceWear::forceFolder (rlvhelper.cpp) wears in the collector's order,
/// sub-folders first, and the second clothing of a type adds (fSeenWType).
#[test]
fn wears_follow_the_collector_order() {
    let mut l = linked();
    let (issuer, clothes) = (l.issuer, l.clothes);
    let base = outfit(&mut l, 0xE801, "Outfit");
    let extra = folder(&mut l.world, 0xE802, base, "+Extra", -1);
    l.world.inventory.add_items(vec![
        item(0xD801, base, "Shirt B", 5, 4),
        item(0xD802, extra, "Shirt A", 5, 4),
        item(0xD803, clothes, "Shirt C", 5, 4),
    ]);
    put_on(&mut l, 0xD813, Uuid::from_u128(0xD803));
    force(&mut l, issuer, "attachall:outfit=force");
    let (remove, wear) = batch(&l);
    assert!(remove.is_empty(), "the worn shirt stays");
    assert_eq!(wear, vec![(Uuid::from_u128(0xD802), false, 0), (Uuid::from_u128(0xD801), false, 0)]);
}

/// RlvAttachmentLocks::canAttach (rlvlocks.h): a replace needs every object
/// on the point free to come off; RLVaWearReplaceUnlocked does not change
/// that (only the watchdog reads it).
#[test]
fn a_replace_skips_a_point_that_holds_a_locked_object() {
    let mut l = linked();
    let issuer = l.issuer;
    let base = outfit(&mut l, 0xE901, "Outfit");
    let hat = Uuid::from_u128(0xD901);
    l.world.inventory.add_items(vec![item(0xD901, base, "Hat (skull)", 6, 0)]);
    say(&mut l.world, issuer, "@detach=n");
    assert!(l.world.rlva.preferences.replace_unlocked);
    force(&mut l, issuer, "attach:outfit=force");
    assert!(!worn_items(&l).contains(&hat), "the collar holds the skull");
    force(&mut l, issuer, "attachover:outfit=force");
    assert_eq!(batch(&l).1, vec![(hat, false, 2)], "added beside it");
}

/// RlvWearableLocks::canWear (rlvlocks.h) ignores nostrip, and
/// RlvForceWear::addWearable leaves a nostrip item on (forceRemove): the new
/// clothing is worn with a replace.
#[test]
fn nostrip_clothing_does_not_block_a_replace() {
    let mut l = linked();
    let (issuer, clothes) = (l.issuer, l.clothes);
    let base = outfit(&mut l, 0xEA01, "Outfit");
    let new = Uuid::from_u128(0xDA01);
    l.world.inventory.add_items(vec![
        item(0xDA01, base, "Shirt", 5, 4),
        item(0xDA02, clothes, "Old shirt nostrip", 5, 4),
    ]);
    put_on(&mut l, 0xDA12, Uuid::from_u128(0xDA02));
    say(&mut l.world, issuer, "@remattach:tail tip=n");
    force(&mut l, issuer, "attach:outfit=force");
    assert_eq!(batch(&l).1, vec![(new, true, 0)]);
}

/// rlvAttachGroupFromIndex (rlvhelper.cpp): "hud" is avatar_lad group 8
/// (tail, wings, rings, hind feet), and the real HUD points (group 9)
/// belong to no group.
#[test]
fn the_hud_group_is_avatar_lad_group_8() {
    let mut l = linked();
    let issuer = l.issuer;
    let lib = std::sync::Arc::clone(&l.world.avatar_lib);
    let names = |group: u8| {
        let mut names: Vec<&str> = (1..super::inventory::POINTS.len() as u8)
            .filter(|p| lib.attach_points.get(p).is_some_and(|at| at.group == group))
            .filter_map(|p| lib.attach_names.get(&p).map(String::as_str))
            .collect();
        names.sort_unstable();
        names
    };
    assert!(!names(8).is_empty());
    let reply = query(&mut l.world, issuer, "getaddattachnames:hud=1").expect("a reply");
    let mut listed: Vec<&str> = reply.split(',').collect();
    listed.sort_unstable();
    assert_eq!(listed, names(8));
    // @remattach:hud=force takes the tail off, not a HUD
    let clothes = l.clothes;
    l.world
        .inventory
        .add_items(vec![item(0xDB01, clothes, "Tail", 6, 0), item(0xDB02, clothes, "Hud", 6, 0)]);
    let (tail, hud) = (Uuid::from_u128(0xDB01), Uuid::from_u128(0xDB02));
    put_on(&mut l, 0xDB11, tail);
    put_on(&mut l, 0xDB12, hud);
    wear(&mut l.world, tail, 44);
    wear(&mut l.world, hud, 33);
    force(&mut l, issuer, "remattach:hud=force");
    assert_eq!(batch(&l).0, vec![tail]);
}

/// RlvCommandOptionHelper::parseOption<RlvCommandOptionGeneric>
/// (rlvhelper.cpp): a layer or a point name is never read as a folder, so
/// `@attach:hair=force` does not wear a folder called "Hair"
/// (VERIFY_OPTION(isSharedFolder()) fails).
#[test]
fn a_layer_or_point_name_is_not_a_folder() {
    let mut l = linked();
    let issuer = l.issuer;
    let hair = outfit(&mut l, 0xEC01, "Hair");
    let spine = outfit(&mut l, 0xEC02, "Spine");
    l.world
        .inventory
        .add_items(vec![item(0xDC01, hair, "Hair", 13, 2), item(0xDC02, spine, "Shirt", 5, 4)]);
    force(&mut l, issuer, "attach:hair=force");
    force(&mut l, issuer, "attachall:spine=force");
    assert!(worn_items(&l).is_empty(), "{:?}", batch(&l));
    force(&mut l, issuer, "attach:hai=force");
    assert_eq!(
        worn_items(&l),
        vec![Uuid::from_u128(0xDC01)],
        "a name that is no layer finds the folder"
    );
}

/// RlvFolderLocks::getLockedFolders, ST_ATTACHMENT (rlvlocks.cpp):
/// `@detachthis` with no option locks the folder of the issuer's own item
/// when it is a shared folder other than #RLV, and nothing else: not the
/// folders of its links, not #RLV itself.
#[test]
fn an_issuer_lock_takes_the_folder_of_its_own_item() {
    let mut l = linked();
    let body = l.body;
    let body_object = Uuid::from_u128(body.as_u128() | 0x0B1E_0000_0000);
    let nil = Uuid::nil();
    say(&mut l.world, body_object, "@detachthis=n");
    assert!(
        l.world.rlva_can_remove(body, nil),
        "its own item is in Clothes; its link's folder is not locked"
    );
    say(&mut l.world, body_object, "@clear");
    let rlv = shared(&l);
    l.world
        .inventory
        .add_items(vec![item(0xDD01, rlv, "Ring", 6, 0), item(0xDD02, rlv, "Bangle", 6, 0)]);
    let (ring, bangle) = (Uuid::from_u128(0xDD01), Uuid::from_u128(0xDD02));
    put_on(&mut l, 0xDD11, ring);
    put_on(&mut l, 0xDD12, bangle);
    let ring_object = wear(&mut l.world, ring, 41);
    wear(&mut l.world, bangle, 42);
    say(&mut l.world, ring_object, "@detachthis=n");
    assert!(l.world.rlva_can_remove(bangle, nil), "#RLV is never locked by its items");
    // the collar's own folder is
    let (issuer, collar) = (l.issuer, l.collar);
    let charm = Uuid::from_u128(0xDD03);
    let collar_folder = l.world.inventory.items[&collar].parent;
    l.world.inventory.add_items(vec![item(0xDD03, collar_folder, "Charm", 6, 0)]);
    put_on(&mut l, 0xDD13, charm);
    wear(&mut l.world, charm, 43);
    say(&mut l.world, issuer, "@detachthis=n");
    assert!(!l.world.rlva_can_remove(charm, nil));
}

/// RlvHandler::onAddRemFolderLock (rlvhandler.cpp): a lock takes no
/// option, a layer, a point or a shared folder that exists; a UUID or a
/// path that is not there is refused. An exception takes a shared folder
/// only. The check holds for =y too.
#[test]
fn folder_lock_options_follow_on_add_rem_folder_lock() {
    let mut l = linked();
    let issuer = l.issuer;
    let other = Uuid::from_u128(0xFEED);
    let rules = |l: &Linked| -> Vec<String> { l.world.rlva.restrictions().into_iter().map(|(_, rule)| rule).collect() };
    for text in [
        format!("@detachallthis:{other}=n"),
        "@detachallthis:nowhere=n".to_owned(),
        "@attachthis:body/nowhere=n".to_owned(),
        "@detachallthis_except=n".to_owned(),
        "@detachallthis_except:shirt=n".to_owned(),
        "@detachthis_except:spine=n".to_owned(),
        format!("@detachthis_except:{other}=n"),
    ] {
        say_checked(&mut l.world, issuer, &text);
        assert!(rules(&l).is_empty(), "{text}: {:?}", rules(&l));
    }
    for text in [
        "@detachallthis=n",
        "@detachallthis:shirt=n",
        "@detachallthis:spine=n",
        "@detachallthis:spare=n",
        "@attachthis:body/mesh=n",
        "@detachallthis_except:spare=n",
    ] {
        say_checked(&mut l.world, issuer, text);
        let rule = text.trim_start_matches('@').trim_end_matches("=n");
        assert!(rules(&l).iter().any(|r| r == rule), "{text}: {:?}", rules(&l));
    }
    // a folder renamed away: its =y is refused as well, the lock stays
    let spare = l.spare;
    l.world.inventory.folders.get_mut(&spare).expect("Spare").info.name = "Gone".into();
    say_checked(&mut l.world, issuer, "@detachallthis:spare=y");
    assert!(rules(&l).iter().any(|r| r == "detachallthis:spare"));
}

/// RlvFolderLocks::isLockedFolder (rlvlocks.cpp): an issuer's rules are
/// read in the order it set them, and at one level the first one decides:
/// an exception set first leaves the folder free, a lock set first keeps it.
#[test]
fn the_first_rule_of_an_issuer_decides_at_one_level() {
    let nil = Uuid::nil();
    for (first, second, locked) in [
        ("@detachallthis_except:spare=n", "@detachallthis:spare=n", false),
        ("@detachallthis:spare=n", "@detachallthis_except:spare=n", true),
    ] {
        let mut l = linked();
        let (issuer, spare) = (l.issuer, l.spare);
        let pin = Uuid::from_u128(0xDF01);
        l.world.inventory.add_items(vec![item(0xDF01, spare, "Pin", 6, 0)]);
        put_on(&mut l, 0xDF11, pin);
        wear(&mut l.world, pin, 9);
        say(&mut l.world, issuer, first);
        say(&mut l.world, issuer, second);
        assert_eq!(!l.world.rlva_can_remove(pin, nil), locked, "{first} then {second}");
    }
}

/// RlvWearableLocks::canWear(LLWearableType::EType) (rlvlocks.h): a body
/// part type that holds a folder-locked body part cannot be worn on, a
/// clothing layer that holds one still takes more.
#[test]
fn add_outfit_names_count_folder_locked_wearables() {
    let mut l = linked();
    let (issuer, spare) = (l.issuer, l.spare);
    l.world
        .inventory
        .add_items(vec![item(0xE001, spare, "Shape", 13, 0), item(0xE002, spare, "Shirt", 5, 4)]);
    put_on(&mut l, 0xE011, Uuid::from_u128(0xE001));
    put_on(&mut l, 0xE012, Uuid::from_u128(0xE002));
    let names = |l: &mut Linked| -> Vec<String> {
        query(&mut l.world, issuer, "getaddoutfitnames=1")
            .expect("a reply")
            .split(',')
            .map(str::to_owned)
            .collect()
    };
    assert!(names(&mut l).iter().any(|n| n == "shape"));
    say(&mut l.world, issuer, "@detachallthis:spare=n");
    let listed = names(&mut l);
    assert!(!listed.iter().any(|n| n == "shape"), "{listed:?}");
    assert!(listed.iter().any(|n| n == "shirt"), "{listed:?}");
}

/// RlvForceWear::addAttachment (rlvhelper.cpp) never clears the default
/// point: two replacing objects that name no point are both sent.
#[test]
fn replacing_objects_with_no_named_point_are_all_worn() {
    let mut l = linked();
    let issuer = l.issuer;
    let base = outfit(&mut l, 0xE101, "Outfit");
    l.world
        .inventory
        .add_items(vec![item(0xE111, base, "Ring 1", 6, 5), item(0xE112, base, "Ring 2", 6, 5)]);
    force(&mut l, issuer, "attach:outfit=force");
    assert_eq!(
        batch(&l).1,
        vec![(Uuid::from_u128(0xE111), true, 0), (Uuid::from_u128(0xE112), true, 0)]
    );
}

/// RlvForceHandler<RLV_BHVR_DETACHME> (rlvhandler.cpp) fails on any option.
#[test]
fn detachme_takes_no_option() {
    let mut l = linked();
    let (issuer, collar) = (l.issuer, l.collar);
    force(&mut l, issuer, "detachme:skull=force");
    assert!(l.world.rlva.appearance.is_empty());
    force(&mut l, issuer, "detachme=force");
    assert_eq!(batch(&l).0, vec![collar]);
}

/// The Legacy Viewer runs every force at once, with no limit on a batch.
/// Aurora queues them for the outfit: the queue keeps a bound for a hostile
/// object, far above any real use.
#[test]
fn forced_batches_have_no_small_limit() {
    let mut l = linked();
    let issuer = l.issuer;
    let base = outfit(&mut l, 0xE201, "Outfit");
    let many: Vec<InvItem> = (0..300u128).map(|i| item(0x1_0000 + i, base, &format!("Item {i}"), 6, 0)).collect();
    l.world.inventory.add_items(many);
    force(&mut l, issuer, "attachover:outfit=force");
    assert_eq!(batch(&l).1.len(), 300);
    l.world.rlva.appearance.clear();
    let small = outfit(&mut l, 0xE202, "Small");
    l.world.inventory.add_items(vec![item(0xE203, small, "Bead", 6, 0)]);
    for i in 0..300u128 {
        force(&mut l, Uuid::from_u128(0xA000 + (i % 2)), "attachover:small=force");
    }
    assert_eq!(l.world.rlva.appearance.len(), 300);
}

/// RlvForceWear::isForceDetachable (rlvhelper.cpp): a folder detach never
/// takes the viewer's bridge off, even through a link in #RLV.
#[test]
fn a_folder_detach_leaves_the_bridge_on() {
    let mut l = linked();
    let (issuer, root) = (l.issuer, l.world.inventory.root);
    let aurora = folder(&mut l.world, 0xE401, root, crate::world::bridge::FOLDER, -1);
    let base = outfit(&mut l, 0xE402, "Outfit");
    let bridge = Uuid::from_u128(0xE403);
    let mut bridge_item = item(0xE403, aurora, crate::world::bridge::NAME, 6, 0);
    bridge_item.owner = l.world.agent_id;
    l.world.inventory.add_items(vec![bridge_item, link(0xE404, base, bridge)]);
    put_on(&mut l, 0xE413, bridge);
    wear(&mut l.world, bridge, crate::world::bridge::POINT);
    assert!(crate::world::bridge::viewer_bridge(&l.world, bridge, crate::world::bridge::POINT));
    force(&mut l, issuer, "detachall:outfit=force");
    assert!(!batch(&l).0.contains(&bridge));
}

/// RlvFindAttachmentsOnPoint (rlvinventory.cpp): an object of the Current
/// Outfit whose name gives the point counts for the point, worn there or not.
#[test]
fn a_point_option_counts_the_objects_that_name_it() {
    let mut l = linked();
    let issuer = l.issuer;
    let hats = outfit(&mut l, 0xE5010, "Hats");
    let hat = Uuid::from_u128(0xE5011);
    l.world.inventory.add_items(vec![item(0xE5011, hats, "Hat (spine)", 6, 0)]);
    put_on(&mut l, 0xE5012, hat);
    assert_eq!(query(&mut l.world, issuer, "getpath:spine=1").as_deref(), Some("Hats"));
}

/// RlvFolderLocks::getLockedFolders, ST_ATTACHMENT (rlvlocks.cpp) reads the
/// issuer's own getAttachmentItemID: a child prim has none (only the root
/// gets AttachItemID, LLVOAvatar::addChild), so its `@detachthis` names no
/// folder.
#[test]
fn a_child_prim_lock_names_no_folder() {
    let mut l = linked();
    let (issuer, collar) = (l.issuer, l.collar);
    let nil = Uuid::nil();
    let charm = Uuid::from_u128(0xDD23);
    let collar_folder = l.world.inventory.items[&collar].parent;
    l.world.inventory.add_items(vec![item(0xDD23, collar_folder, "Charm", 6, 0)]);
    put_on(&mut l, 0xDD33, charm);
    wear(&mut l.world, charm, 43);
    let handle = l.world.main_region.expect("the demo region");
    let root = l.world.objects.index_of_uuid(&issuer).expect("the collar");
    let root_local = l.world.objects.get(root).expect("the collar").key.local_id;
    let mut child = crate::demo::outfit_attachment(&aurora_net::AttachRequest {
        item_id: Uuid::from_u128(0xC0FFEE),
        owner_id: l.world.agent_id,
        point: 5,
        add: true,
        flags: 0,
        group_mask: 0,
        everyone_mask: 0,
        next_owner_mask: 0,
        name: String::new(),
        desc: String::new(),
    });
    child.parent_id = root_local;
    child.full_id = Uuid::from_u128(0xC0FFEE);
    child.name_values.clear();
    l.world.objects.upsert(handle, child);
    say(&mut l.world, Uuid::from_u128(0xC0FFEE), "@detachthis=n");
    assert!(l.world.rlva_can_remove(charm, nil), "the child prim locks nothing");
    say(&mut l.world, issuer, "@detachthis=n");
    assert!(!l.world.rlva_can_remove(charm, nil), "the root locks its folder");
}

/// RlvCriteriaCategoryCollector (rlvinventory.h) and RlvInventory::
/// getSharedFolder (rlvinventory.cpp) fold the case of the ASCII letters
/// only (LLStringUtil::toLower, boost::icontains byte by byte): an accented
/// capital is not its small letter.
#[test]
fn folder_names_fold_the_ascii_case_only() {
    let mut l = linked();
    let issuer = l.issuer;
    let cafe = outfit(&mut l, 0xEF01, "CAFÉ Wear");
    folder(&mut l.world, 0xEF02, cafe, "Sub", -1);
    assert_eq!(query(&mut l.world, issuer, "findfolder:café=1").as_deref(), Some(""));
    assert_eq!(query(&mut l.world, issuer, "findfolder:cafÉ=1").as_deref(), Some("CAFÉ Wear"));
    assert_eq!(query(&mut l.world, issuer, "getinv:é wear=1").as_deref(), Some(""));
    assert_eq!(query(&mut l.world, issuer, "getinv:É wear=1").as_deref(), Some("Sub"));
}

/// RlvHandler::processCommand (rlvhandler.cpp) retains commands only until
/// the login completes; after it every query answers at once with what
/// gInventory holds, while RlvInventory::fetchSharedInventory goes on.
#[test]
fn queries_answer_at_once_while_the_shared_folders_load() {
    use crate::world::inventory::FetchState;
    let mut l = linked();
    let issuer = l.issuer;
    let cuffs = outfit(&mut l, 0xE201, "Cuffs");
    folder(&mut l.world, 0xE202, cuffs, "Left", -1);
    l.world.inventory.folders.get_mut(&cuffs).expect("Cuffs").state = FetchState::Unknown;
    let rlv = shared(&l);
    l.world.inventory.folders.get_mut(&rlv).expect("#RLV").state = FetchState::Unknown;
    assert_eq!(query(&mut l.world, issuer, "getinv:Cuffs=1").as_deref(), Some("Left"));
    assert_eq!(query(&mut l.world, issuer, "findfolder:left=1").as_deref(), Some("Cuffs/Left"));
    assert_eq!(query(&mut l.world, issuer, "getpath=1").as_deref(), Some("Collar"));
}

/// RlvHandler::onForceWear runs RlvForceWear::forceFolder at once
/// (rlvhandler.cpp, rlvhelper.cpp): a folder force wears what is loaded and
/// does not wait for a subfolder still being fetched.
#[test]
fn a_folder_force_runs_at_once_with_what_is_loaded() {
    use crate::world::inventory::FetchState;
    let mut l = linked();
    let issuer = l.issuer;
    let base = outfit(&mut l, 0xE211, "Mittens");
    let glove = Uuid::from_u128(0xE212);
    l.world.inventory.add_items(vec![item(0xE212, base, "Glove", 5, 9)]);
    let later = folder(&mut l.world, 0xE213, base, "Later", -1);
    l.world.inventory.folders.get_mut(&later).expect("Later").state = FetchState::Unknown;
    force(&mut l, issuer, "attachall:Mittens=force");
    assert_eq!(worn_items(&l), vec![glove]);
}

/// RlvCommandOptionHelper::parseOption (rlvhelper.cpp) looks for a shared
/// folder in what is loaded, then for a group: a group force does not wait
/// for #RLV.
#[test]
fn a_group_force_does_not_wait_for_the_shared_folders() {
    use crate::world::inventory::FetchState;
    let mut l = linked();
    let (issuer, collar) = (l.issuer, l.collar);
    let rlv = shared(&l);
    l.world.inventory.folders.get_mut(&rlv).expect("#RLV").state = FetchState::Unknown;
    force(&mut l, issuer, "remattach:head=force");
    assert_eq!(batch(&l).0, vec![collar]);
}

/// rez_attachment (llinventorybridge.cpp): a Wear with no point chosen goes
/// to the point the item names while a point is locked, and to the default
/// point with RLVaDebugDeprecateExplicitPoint on (off by default).
#[test]
fn a_wear_takes_the_named_point_while_a_point_is_locked() {
    let mut l = linked();
    let (issuer, clothes) = (l.issuer, l.clothes);
    let ring = Uuid::from_u128(0xE301);
    l.world.inventory.add_items(vec![item(0xE301, clothes, "Ring (left hand)", 6, 0)]);
    assert!(!l.world.rlva.preferences.debug_deprecate_explicit_point, "off by default");
    assert_eq!(l.world.rlva_rez_point(ring, 0), 0, "no point locked");
    say(&mut l.world, issuer, "@remattach:chest=n");
    assert_eq!(l.world.rlva_rez_point(ring, 0), 5, "the named point");
    assert_eq!(l.world.rlva_rez_point(ring, 2), 2, "a chosen point stays");
    l.world.rlva.preferences.debug_deprecate_explicit_point = true;
    assert_eq!(l.world.rlva_rez_point(ring, 0), 0, "the lookup is skipped");
    l.world.rlva.preferences.debug_deprecate_explicit_point = false;
    say(&mut l.world, issuer, "@remattach:chest=y");
    say(&mut l.world, issuer, "@addattach:skull=n");
    assert_eq!(l.world.rlva_rez_point(ring, 0), 5, "an add lock counts too");
}

/// RlvAttachmentLocks::canAttach(const LLInventoryItem*) (rlvlocks.h): an
/// object whose names give no point is RLV_WEAR, whatever point is saved on
/// it or where it was last worn; a named point is checked.
#[test]
fn an_object_with_no_named_point_can_be_worn() {
    let mut l = linked();
    let (issuer, clothes) = (l.issuer, l.clothes);
    let saved = Uuid::from_u128(0xE311);
    let named = Uuid::from_u128(0xE312);
    l.world.inventory.add_items(vec![
        item(0xE311, clothes, "Bracelet", 6, 5),
        item(0xE312, clothes, "Ring (left hand)", 6, 0),
    ]);
    say(&mut l.world, issuer, "@addattach:left hand=n");
    let nil = Uuid::nil();
    assert!(l.world.rlva_can_add(saved, 0, nil), "the saved left hand is no named point");
    assert!(!l.world.rlva_can_add(named, 0, nil), "the named left hand is locked");
    assert!(!l.world.rlva_can_add(saved, 5, nil), "Attach To the left hand");
    let locks = l.world.rlva_wear_locks();
    assert!(locks.wear(&l.world.inventory, saved, 0).replace);
    assert!(!locks.wear(&l.world.inventory, named, 0).add);
}
