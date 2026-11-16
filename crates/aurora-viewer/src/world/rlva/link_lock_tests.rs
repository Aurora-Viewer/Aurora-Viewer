//! Folder locks reaching a worn item through its links under #RLV
//! (RlvFolderLocks::getLockedItems, rlvlocks.cpp; issue #2677): every check
//! of a worn item counts them, through `rlva_item_folders_removable_in`.

use super::Command;
use crate::world::World;
use crate::world::inventory::{FetchState, Folder};
use aurora_net::{
    AttachRequest, ChatMessage, ChatSourceType, ChatType, NetCommand,
    inventory::{InvFolder, InvItem},
    outfits::{OutfitLink, OutfitMutation},
};
use std::collections::BTreeSet;
use uuid::Uuid;

/// A second issuer, not worn.
const OTHER_ISSUER: Uuid = Uuid::from_u128(0xB0B);

/// A mesh body kept outside #RLV and worn through its link in
/// `#RLV/Body/Mesh`, the collar that locks it, and an attachment no lock
/// names.
pub(crate) struct Linked {
    pub world: World,
    /// The body's original, in `Clothes` (outside #RLV), worn on the chest.
    pub body: Uuid,
    /// Its link in `#RLV/Body/Mesh`.
    pub link: Uuid,
    /// `#RLV/Body/Mesh`.
    pub mesh: Uuid,
    /// `#RLV/Spare`, empty.
    pub spare: Uuid,
    /// `Clothes`, outside #RLV.
    pub clothes: Uuid,
    /// The collar, in `#RLV/Collar`, worn on the skull.
    pub collar: Uuid,
    /// The collar's object in the world: the issuer of the locks.
    pub issuer: Uuid,
    /// In `Clothes`, with no link, worn on the left shoulder.
    pub free: Uuid,
    /// In `Clothes`, not worn.
    pub new: Uuid,
    pub cof: Uuid,
}

const CHEST: u8 = 1;
const SKULL: u8 = 2;
const LEFT_SHOULDER: u8 = 3;

fn folder(world: &mut World, id: u128, parent: Uuid, name: &str, kind: i32) -> Uuid {
    let id = Uuid::from_u128(id);
    world.inventory.folders.insert(
        id,
        Folder {
            info: InvFolder {
                id,
                parent,
                name: name.into(),
                type_default: kind,
                ..Default::default()
            },
            children: vec![],
            items: vec![],
            state: FetchState::Fetched,
            library: false,
        },
    );
    if let Some(parent) = world.inventory.folders.get_mut(&parent) {
        parent.children.push(id);
    }
    id
}

fn object(id: u128, parent: Uuid) -> InvItem {
    InvItem {
        id: Uuid::from_u128(id),
        parent,
        name: format!("Object {id:x}"),
        asset_type: 6,
        ..Default::default()
    }
}

fn link(id: u128, parent: Uuid, to: Uuid) -> InvItem {
    InvItem {
        id: Uuid::from_u128(id),
        parent,
        name: format!("Link {id:x}"),
        asset_type: 24,
        asset_id: to,
        ..Default::default()
    }
}

/// Wears `item` at `point`; returns its object. As on the grid, the object
/// has an id of its own: only a temporary attachment goes by its object id.
fn wear(world: &mut World, item: Uuid, point: u8) -> Uuid {
    let handle = world.main_region.expect("the demo region");
    let mut object = crate::demo::outfit_attachment(&AttachRequest {
        item_id: item,
        owner_id: world.agent_id,
        point,
        add: true,
        flags: 0,
        group_mask: 0,
        everyone_mask: 0,
        next_owner_mask: 0,
        name: String::new(),
        desc: String::new(),
    });
    object.full_id = Uuid::from_u128(item.as_u128() | 0x0B1E_0000_0000);
    world.objects.upsert(handle, object);
    Uuid::from_u128(item.as_u128() | 0x0B1E_0000_0000)
}

pub(crate) fn linked() -> Linked {
    let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        world.apply(event);
    }
    world.rlva_configure(true);
    world.movement_complete = true;
    let root = folder(&mut world, 0xF000, Uuid::nil(), "My Inventory", 8);
    world.inventory.root = root;
    let rlv = folder(&mut world, 0xF001, root, "#RLV", -1);
    let body_folder = folder(&mut world, 0xF002, rlv, "Body", -1);
    let mesh = folder(&mut world, 0xF003, body_folder, "Mesh", -1);
    let spare = folder(&mut world, 0xF004, rlv, "Spare", -1);
    let collar_folder = folder(&mut world, 0xF005, rlv, "Collar", -1);
    let clothes = folder(&mut world, 0xF006, root, "Clothes", -1);
    let cof = folder(&mut world, 0xF007, root, "Current Outfit", 46);
    let body = Uuid::from_u128(0xC001);
    let collar = Uuid::from_u128(0xC002);
    let free = Uuid::from_u128(0xC003);
    let new = Uuid::from_u128(0xC004);
    let link_id = Uuid::from_u128(0xC011);
    world.inventory.add_items(vec![
        object(0xC001, clothes),
        object(0xC002, collar_folder),
        object(0xC003, clothes),
        object(0xC004, clothes),
        link(0xC011, mesh, body),
        link(0xC021, cof, body),
        link(0xC022, cof, collar),
        link(0xC023, cof, free),
    ]);
    wear(&mut world, body, CHEST);
    let issuer = wear(&mut world, collar, SKULL);
    wear(&mut world, free, LEFT_SHOULDER);
    Linked {
        world,
        body,
        link: link_id,
        mesh,
        spare,
        clothes,
        collar,
        issuer,
        free,
        new,
        cof,
    }
}

impl Linked {
    fn say(&mut self, source: Uuid, text: &str) {
        let chat = ChatMessage {
            from_name: "RLVa test object".to_owned(),
            source_id: source,
            owner_id: Uuid::nil(),
            source_type: ChatSourceType::Object,
            chat_type: ChatType::OwnerSay,
            region: None,
            position: glam::Vec3::ZERO,
            message: text.to_owned(),
        };
        assert!(self.world.rlva.receive(&chat), "{text}");
    }

    fn removable(&self, item: Uuid) -> bool {
        self.world.rlva_can_remove(item, Uuid::nil())
    }

    /// What the watchdog holds on the avatar.
    fn held(&mut self) -> BTreeSet<Uuid> {
        let now = std::time::Instant::now();
        self.world.rlva_watchdog_tick(now);
        self.world.rlva_watchdog_tick(now);
        self.world.rlva.watchdog.checked.as_ref().expect("checked").locked.clone()
    }

    /// The resolution of the folder locks in the cache, by address.
    fn resolution(&self) -> Option<*const super::inventory::LockedFolders> {
        let cache = self.world.rlva.locked_folders.0.lock().expect("cache");
        cache.as_ref().map(|(_, folders)| std::sync::Arc::as_ptr(folders))
    }

    /// What a complete outfit replacement by `new` alone leaves in the
    /// Current Outfit folder, and what its detach request takes off.
    fn replaced(&self) -> (BTreeSet<Uuid>, BTreeSet<Uuid>) {
        let mut outfit = NetCommand::UpdateOutfit {
            request: Uuid::from_u128(0xABC),
            change: OutfitMutation {
                folder: self.cof,
                create: None,
                cof: None,
                links: vec![OutfitLink {
                    target: self.new,
                    name: "new".into(),
                    desc: String::new(),
                    folder: false,
                    inv_type: 17,
                }],
            },
        };
        self.world.rlva_prepare_outfit(&mut outfit, Uuid::nil());
        assert!(
            self.world.rlva_allows_command(&outfit, Uuid::nil()),
            "the prepared change goes through"
        );
        let NetCommand::UpdateOutfit { change, .. } = outfit else {
            unreachable!()
        };
        let mut detach = NetCommand::DetachAttachments(vec![self.body, self.collar, self.free]);
        let detached = if self.world.rlva_prepare_attachments(&mut detach, Uuid::nil()) {
            let NetCommand::DetachAttachments(items) = detach else {
                unreachable!()
            };
            items.into_iter().collect()
        } else {
            BTreeSet::new()
        };
        (change.links.iter().map(|l| l.target).collect(), detached)
    }

    /// The reply to `@getattach:<point>` asked by `source`, with the locked
    /// attachments hidden.
    fn worn_reply(&mut self, source: Uuid, point: &str) -> Option<String> {
        self.world.rlva.preferences.hide_locked_attachments = true;
        let query = Command::parse(&format!("getattach:{point}=123")).expect("a query");
        self.world.rlva_inventory_query(&query, source)
    }
}

/// A complete outfit replacement that leaves out a body worn through a link
/// in a locked shared folder keeps the body and the self-locked collar on,
/// and changes the rest: @detachthis on the link's folder, @detachallthis on
/// a folder above it, their release, and a plain point lock as the control.
#[test]
fn an_outfit_replacement_keeps_what_a_link_in_a_locked_folder_holds() {
    let mut l = linked();
    let (body, collar, issuer, free, new) = (l.body, l.collar, l.issuer, l.free, l.new);
    l.say(issuer, "@detach=n");
    assert_eq!(
        l.replaced(),
        (BTreeSet::from([new, collar]), BTreeSet::from([body, free])),
        "no folder lock: only the self-locked collar stays"
    );
    // @detachthis locks the folder it names, not the folders under it
    l.say(issuer, "@detachthis:Body=n");
    assert!(l.removable(body), "the link is one folder below");
    l.say(issuer, "@detachthis:Body=y");
    for lock in ["@detachthis:Body/Mesh", "@detachallthis:Body"] {
        l.say(issuer, &format!("{lock}=n"));
        assert!(!l.removable(body), "{lock}");
        assert!(l.removable(free), "{lock}: no link in a locked folder");
        assert_eq!(
            l.replaced(),
            (BTreeSet::from([new, body, collar]), BTreeSet::from([free])),
            "{lock}: the body and the collar stay, the unrelated attachment changes"
        );
        assert_eq!(l.held(), BTreeSet::from([body, collar]), "{lock}: the watchdog holds both");
        l.say(issuer, &format!("{lock}=y"));
        assert!(l.removable(body), "{lock} released");
        assert_eq!(
            l.replaced(),
            (BTreeSet::from([new, collar]), BTreeSet::from([body, free])),
            "{lock} released"
        );
        assert_eq!(l.held(), BTreeSet::from([collar]), "{lock} released");
    }
    // the control: a point lock holds the body whatever its links
    l.say(issuer, "@remattach:chest=n");
    assert_eq!(
        l.replaced(),
        (BTreeSet::from([new, body, collar]), BTreeSet::from([free])),
        "a point lock"
    );
}

/// @getattach hides a locked attachment, except from the issuer of its lock;
/// taking it off obeys the lock whoever asks.
#[test]
fn a_presence_query_exempts_the_issuer_of_a_link_lock_but_a_removal_does_not() {
    let mut l = linked();
    let (body, issuer) = (l.body, l.issuer);
    assert_eq!(l.worn_reply(OTHER_ISSUER, "chest").as_deref(), Some("1"), "nothing locked");
    l.say(issuer, "@detachallthis:Body=n");
    assert_eq!(l.worn_reply(issuer, "chest").as_deref(), Some("1"), "its own lock");
    assert_eq!(l.worn_reply(OTHER_ISSUER, "chest").as_deref(), Some("0"), "someone else's lock");
    assert!(!l.world.rlva_can_remove(body, issuer), "the issuer cannot take it off either");
    assert!(!l.world.rlva_can_remove(body, OTHER_ISSUER));
    assert!(!l.removable(body));
    // a second issuer: no one is exempt from the other's lock
    l.say(OTHER_ISSUER, "@detachallthis:Body=n");
    assert_eq!(l.worn_reply(issuer, "chest").as_deref(), Some("0"));
    assert_eq!(l.worn_reply(OTHER_ISSUER, "chest").as_deref(), Some("0"));
    l.say(issuer, "@detachallthis:Body=y");
    assert_eq!(l.worn_reply(OTHER_ISSUER, "chest").as_deref(), Some("1"));
    assert_eq!(l.worn_reply(issuer, "chest").as_deref(), Some("0"));
}

/// The links are indexed once and kept while the inventory, the rules and
/// what is worn stay the same; a link moved out of #RLV, moved back or
/// deleted changes the answers.
#[test]
fn the_link_index_follows_the_inventory_and_is_kept_while_nothing_changes() {
    let mut l = linked();
    let (body, issuer, mesh, clothes, link_id) = (l.body, l.issuer, l.mesh, l.clothes, l.link);
    l.say(issuer, "@detachallthis:Body=n");
    assert!(!l.removable(body));
    assert_eq!(l.held(), BTreeSet::from([body]));
    let first = l.resolution();
    assert!(first.is_some());
    assert!(!l.removable(body));
    assert!(!l.world.rlva_has_locked_hud());
    assert!(l.world.rlva_has_locked_attachment());
    assert_eq!(l.held(), BTreeSet::from([body]));
    assert_eq!(l.resolution(), first, "nothing changed: #RLV is not walked again");
    // the link leaves #RLV
    l.world.inventory.add_items(vec![link(0xC011, clothes, body)]);
    assert!(l.removable(body), "a link outside #RLV carries no lock");
    assert!(l.held().is_empty());
    assert!(!l.world.rlva_has_locked_attachment());
    let moved = l.resolution();
    assert_ne!(moved, first, "an inventory change indexes the links again");
    // and comes back
    l.world.inventory.add_items(vec![link(0xC011, mesh, body)]);
    assert!(!l.removable(body), "back in the locked folder");
    assert_eq!(l.held(), BTreeSet::from([body]));
    // then is deleted
    l.world.inventory.remove(&[link_id]);
    assert!(l.removable(body), "no link left");
    assert!(l.held().is_empty());
    assert!(!l.world.rlva_has_locked_attachment());
}

/// Two links in two locked folders, then two issuers locking one folder:
/// releasing one lock leaves the other in force.
#[test]
fn each_link_and_each_issuer_holds_the_item_on_its_own() {
    let mut l = linked();
    let (body, issuer, spare) = (l.body, l.issuer, l.spare);
    l.world.inventory.add_items(vec![link(0xC012, spare, body)]);
    l.say(issuer, "@detachthis:Body/Mesh=n,detachthis:Spare=n");
    assert!(!l.removable(body));
    l.say(issuer, "@detachthis:Body/Mesh=y");
    assert!(!l.removable(body), "the link in Spare still holds it");
    assert_eq!(l.held(), BTreeSet::from([body]));
    l.say(issuer, "@detachthis:Spare=y");
    assert!(l.removable(body));
    assert!(l.held().is_empty());

    l.say(issuer, "@detachallthis:Body=n");
    l.say(OTHER_ISSUER, "@detachallthis:Body=n");
    l.say(issuer, "@detachallthis:Body=y");
    assert!(!l.removable(body), "the other issuer's lock still holds it");
    assert_eq!(l.held(), BTreeSet::from([body]));
    assert!(l.world.rlva_has_locked_attachment());
    l.say(OTHER_ISSUER, "@detachallthis:Body=y");
    assert!(l.removable(body));
    assert!(l.held().is_empty());
    assert!(!l.world.rlva_has_locked_attachment());
}

/// A worn item whose own record is not loaded yet, with a link under #RLV
/// that is: the link's folder still holds it. Deliberately stricter than
/// RlvFolderLocks::getLockedItems, which only walks the worn items the
/// inventory holds and lets such an item go until its folder arrives.
#[test]
fn a_link_holds_a_worn_item_whose_own_record_is_not_loaded() {
    let mut l = linked();
    let (body, issuer, free) = (l.body, l.issuer, l.free);
    l.world.inventory.remove(&[body]);
    assert!(!l.world.inventory.items.contains_key(&body));
    assert!(l.removable(body), "no rule");
    l.say(issuer, "@detachthis:Body/Mesh=n");
    assert!(!l.removable(body));
    assert_eq!(l.held(), BTreeSet::from([body]), "the watchdog holds it");
    assert!(l.world.rlva_has_locked_attachment(), "Release Keys stays greyed out");
    assert_eq!(l.worn_reply(OTHER_ISSUER, "chest").as_deref(), Some("0"), "hidden from the others");
    assert_eq!(l.worn_reply(issuer, "chest").as_deref(), Some("1"), "not from its issuer");
    let mut detach = NetCommand::DetachAttachments(vec![body, free]);
    assert!(l.world.rlva_prepare_attachments(&mut detach, Uuid::nil()));
    assert!(
        matches!(&detach, NetCommand::DetachAttachments(items) if *items == [free]),
        "{detach:?}"
    );
    l.say(issuer, "@detachthis:Body/Mesh=y");
    assert!(l.removable(body), "released with the rule");
    assert!(l.held().is_empty());
    assert!(!l.world.rlva_has_locked_attachment());
    assert_eq!(l.worn_reply(OTHER_ISSUER, "chest").as_deref(), Some("1"));
}

/// A HUD worn through a link in a locked folder is a locked HUD
/// (RlvAttachmentLocks::hasLockedHUD): it cannot be hidden.
#[test]
fn a_hud_held_by_a_link_is_a_locked_hud() {
    let mut l = linked();
    let (issuer, mesh) = (l.issuer, l.mesh);
    let hud_point = l
        .world
        .avatar_lib
        .attach_points
        .iter()
        .filter(|(_, point)| point.hud)
        .map(|(id, _)| *id)
        .min()
        .expect("a HUD point");
    let hud = Uuid::from_u128(0xC005);
    l.world
        .inventory
        .add_items(vec![object(0xC005, l.clothes), link(0xC015, mesh, hud)]);
    wear(&mut l.world, hud, hud_point);
    assert!(!l.world.rlva_has_locked_hud());
    l.say(issuer, "@detachthis:Body/Mesh=n");
    assert!(l.world.rlva_has_locked_hud());
    l.say(issuer, "@detachthis:Body/Mesh=y");
    assert!(!l.world.rlva_has_locked_hud());
}
