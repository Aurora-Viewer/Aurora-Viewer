//! The outfit change of a collar's "outfits" menu (issue #3195): the collar
//! locks itself and a hidden `core` folder, strips the rest, then wears the
//! chosen folder. What `core` holds (mesh body, alpha) stays on throughout:
//! RlvWearableItemCollector::onCollectFolder (rlvinventory.cpp) never enters
//! a hidden folder below the one a command names, and a forced removal obeys
//! the folder locks of its own issuer (RlvFolderLocks::canRemoveItem,
//! rlvlocks.cpp), which also hold what `core` only links to, even when the
//! outfit that goes links to it too (RlvFolderLocks::getLockedItems).

use super::link_lock_tests::{folder, link, object, wear};
use crate::world::World;
use crate::world::appearance::Action;
use aurora_net::{ChatMessage, ChatSourceType, ChatType, inventory::InvItem};
use std::collections::BTreeSet;
use uuid::Uuid;

struct Wardrobe {
    world: World,
    /// The collar's object: the issuer of every command.
    issuer: Uuid,
    collar: Uuid,
    /// In `#RLV/.outfits/.core`, worn: a mesh body and an alpha layer.
    body: Uuid,
    alpha: Uuid,
    /// In `#RLV/.outfits/Dress`, worn: an attachment and a shirt.
    dress: Uuid,
    shirt: Uuid,
    /// In `#RLV/.outfits/Gown`, not worn.
    gown: Uuid,
    /// A head kept outside #RLV, worn, with a link in the core folder and
    /// one in each outfit.
    head: Uuid,
}

fn layer(id: u128, parent: Uuid, kind: u32) -> InvItem {
    InvItem {
        id: Uuid::from_u128(id),
        parent,
        name: format!("Layer {id:x}"),
        asset_type: 5,
        flags: kind,
        ..Default::default()
    }
}

fn wardrobe() -> Wardrobe {
    let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        world.apply(event);
    }
    world.rlva_configure(true);
    world.movement_complete = true;
    let root = folder(&mut world, 0xF100, Uuid::nil(), "My Inventory", 8);
    world.inventory.root = root;
    let rlv = folder(&mut world, 0xF101, root, "#RLV", -1);
    let outfits = folder(&mut world, 0xF102, rlv, ".outfits", -1);
    let core = folder(&mut world, 0xF103, outfits, ".core", -1);
    let dress_folder = folder(&mut world, 0xF104, outfits, "Dress", -1);
    let gown_folder = folder(&mut world, 0xF105, outfits, "Gown", -1);
    let collar_folder = folder(&mut world, 0xF106, rlv, "Collar", -1);
    let cof = folder(&mut world, 0xF107, root, "Current Outfit", 46);
    let bodies = folder(&mut world, 0xF108, root, "Bodies", -1);
    let id = Uuid::from_u128;
    let (collar, body, alpha, dress, shirt, gown) = (id(0xD001), id(0xD002), id(0xD003), id(0xD004), id(0xD005), id(0xD006));
    let head = id(0xD007);
    world.inventory.add_items(vec![
        object(0xD001, collar_folder),
        object(0xD002, core),
        layer(0xD003, core, 17),
        object(0xD004, dress_folder),
        layer(0xD005, dress_folder, 4),
        object(0xD006, gown_folder),
        link(0xD011, cof, collar),
        link(0xD012, cof, body),
        link(0xD013, cof, alpha),
        link(0xD014, cof, dress),
        link(0xD015, cof, shirt),
        object(0xD007, bodies),
        link(0xD016, cof, head),
        link(0xD021, core, head),
        link(0xD022, dress_folder, head),
        link(0xD023, gown_folder, head),
    ]);
    let issuer = wear(&mut world, collar, 2);
    wear(&mut world, body, 1);
    wear(&mut world, dress, 3);
    wear(&mut world, head, 4);
    Wardrobe {
        world,
        issuer,
        collar,
        body,
        alpha,
        dress,
        shirt,
        gown,
        head,
    }
}

impl Wardrobe {
    fn say(&mut self, text: &str) {
        let chat = ChatMessage {
            from_name: "Collar".to_owned(),
            source_id: self.issuer,
            owner_id: Uuid::nil(),
            source_type: ChatSourceType::Object,
            chat_type: ChatType::OwnerSay,
            region: None,
            position: glam::Vec3::ZERO,
            message: text.to_owned(),
        };
        assert!(self.world.rlva.receive(&chat), "{text}");
    }

    /// What the forced commands received so far take off and put on. A
    /// folder's removal names what the folder holds, worn or not: only what
    /// is worn counts here.
    fn forced(&mut self) -> (BTreeSet<Uuid>, BTreeSet<Uuid>) {
        self.world.rlva_pending();
        let worn = [self.collar, self.body, self.alpha, self.dress, self.shirt, self.head];
        let mut off = BTreeSet::new();
        let mut on = BTreeSet::new();
        for (_, action) in self.world.rlva.appearance.drain(..) {
            if let Action::Force { remove, wear } = action {
                off.extend(remove.into_iter().filter(|item| worn.contains(item)));
                on.extend(wear.into_iter().map(|w| w.item));
            }
        }
        (off, on)
    }

    /// The collar's locks before it strips: itself, then the core folder.
    fn lock_core(&mut self) {
        self.say("@detach=n");
        self.say("@detachallthis:.outfits/.core=y");
        self.say("@detachallthis:.outfits/.core=n");
    }
}

/// The default change: everything under the outfits folder comes off but the
/// hidden core folder, then the chosen folder is worn over what is left.
#[test]
fn a_collar_outfit_change_leaves_the_hidden_core_folder_on() {
    let mut w = wardrobe();
    let (collar, body, alpha, dress, shirt, gown) = (w.collar, w.body, w.alpha, w.dress, w.shirt, w.gown);
    w.lock_core();
    w.say("@detachall:.outfits=force");
    let (off, on) = w.forced();
    assert_eq!(off, BTreeSet::from([dress, shirt]), "the outfit goes, the core and the collar stay");
    assert!(on.is_empty());
    for kept in [collar, body, alpha, w.head] {
        assert!(!w.world.rlva_can_remove(kept, w.issuer), "held against its own issuer");
    }
    w.say("@detach=y");
    w.say("@detachallthis:.outfits/.core=y");
    w.say("@attachallover:.outfits/Gown=force");
    let (off, on) = w.forced();
    assert!(off.is_empty(), "wearing over takes nothing off");
    assert!(on.contains(&gown) && !on.contains(&dress), "{on:?}");
}

/// "Strip all": the collar detaches every attachment and every layer; its
/// own locks still hold the core folder and itself.
#[test]
fn a_collar_strip_all_keeps_the_locked_core_folder_and_the_collar() {
    let mut w = wardrobe();
    let (dress, shirt) = (w.dress, w.shirt);
    w.lock_core();
    w.say("@detach=force");
    w.say("@remoutfit=force");
    let (off, on) = w.forced();
    assert_eq!(off, BTreeSet::from([dress, shirt]));
    assert!(on.is_empty());
    // without the core lock the same strip takes the core off too
    w.say("@detachallthis:.outfits/.core=y");
    w.say("@detach=force");
    w.say("@remoutfit=force");
    let (off, _) = w.forced();
    assert_eq!(
        off,
        BTreeSet::from([w.body, w.alpha, dress, shirt, w.head]),
        "only the self-locked collar stays"
    );
}

/// The collar releases its locks while its strip still waits for an outfit
/// change to end: RlvForceWear::forceDetach and forceRemove (rlvhelper.cpp)
/// read the locks when the command is processed, so the strip keeps the ones
/// it was received under.
#[test]
fn a_collar_strip_that_waited_keeps_the_locks_it_was_received_under() {
    let mut w = wardrobe();
    let (body, dress, shirt) = (w.body, w.dress, w.shirt);
    w.lock_core();
    w.world.rlva.appearance_busy = true;
    w.say("@detach=force");
    w.say("@remoutfit=force");
    w.world.rlva_pending();
    assert!(w.world.rlva.appearance.is_empty(), "the strip waits for the outfit change");
    w.say("@detach=y");
    w.say("@detachallthis:.outfits/.core=y");
    w.world.rlva.appearance_busy = false;
    let (off, on) = w.forced();
    assert_eq!(off, BTreeSet::from([dress, shirt]), "the core and the collar stay on");
    assert!(on.is_empty());
    assert!(
        w.world.rlva_can_remove(body, Uuid::nil()),
        "the locks are released for what comes after"
    );
}

/// Lock, strip and release in one message: each command reads the rules of
/// its place in the message, not those the message ends with.
#[test]
fn a_strip_between_a_lock_and_its_release_in_one_message_obeys_the_lock() {
    let mut w = wardrobe();
    let (dress, shirt) = (w.dress, w.shirt);
    w.say("@detach=n,detachallthis:.outfits/.core=n,detach=force,remoutfit=force,detachallthis:.outfits/.core=y,detach=y");
    let (off, _) = w.forced();
    assert_eq!(off, BTreeSet::from([dress, shirt]));
    assert!(w.world.rlva.restrictions().is_empty());
}

/// The other way round, release, strip and lock again: the strip names what
/// was free when it was received. (The removal is then sent under the
/// current rules, which may hold more: stricter than the Legacy Viewer,
/// never looser.)
#[test]
fn a_strip_between_a_release_and_a_new_lock_in_one_message_names_what_was_free() {
    let mut w = wardrobe();
    let alpha = w.alpha;
    w.lock_core();
    w.say("@detachallthis:.outfits/.core=y,remoutfit=force,detachallthis:.outfits/.core=n");
    let (off, _) = w.forced();
    assert!(off.contains(&alpha), "{off:?}");
    assert!(!w.world.rlva_can_remove(alpha, Uuid::nil()), "locked again for what comes after");
}
