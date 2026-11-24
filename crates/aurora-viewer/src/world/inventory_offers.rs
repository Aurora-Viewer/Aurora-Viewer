//! Inventory offers from other residents (IM_INVENTORY_OFFERED) in the
//! conversation with the giver, as LLOfferHandler::processNotification does
//! for UserGiveItem, a log_to_im notification
//! (indra/newview/llnotificationofferhandler.cpp, originally LGPL 2.1):
//! spawnIMSession and addNotifPanelToIM open the conversation on a card,
//! next to the entry of the notification centre, and logToIMP2P writes the
//! offer to the giver's transcript. Objects' offers
//! (IM_TASK_INVENTORY_OFFERED) stay in the notification centre only.
//!
//! The entry holds the answer (`World::respond_notification`: the reply to
//! the giver, the Trash for a rejected item, the history's outcome); the
//! card answers through it (`World::answer_inventory_offer`) and shows what
//! became of the offer, whichever side answered.
//!
//! With ShowInInventory (Preferences › Inventory, off by default as in the
//! Legacy Viewer), keeping an offer, a resident's or an object's, opens the
//! inventory on what it gave once it is there (`World::take_inventory_reveal`).

use super::dialog_names;
use super::inventory::actions;
use super::names::NameScope;
use super::notification_history::Outcome;
use super::notifications::{AT_CATEGORY, Data, Kind, NameLink, NameTarget, Response};
use super::{ChatKind, World};
use aurora_net::inventory::{InvFolder, InvItem};
use aurora_net::{InstantMessage, NetCommand};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// ObjectGiveItem / OwnObjectGiveItem, inventory_offer_handler in
/// indra/newview/llimprocessing.cpp. The IM sender is the owner, while its
/// FromAgentName is the object name, never an avatar-name cache entry.
#[derive(Debug, Clone)]
pub struct TaskOffer {
    pub object_name: String,
    pub item_name: String,
    pub from_group: bool,
    pub own_object: bool,
}

impl TaskOffer {
    /// `owner`: as [`TaskOffer::shown_owner_name`] gives it.
    fn body(&self, owner: &str) -> String {
        let object = dialog_names::shown(&self.object_name).into_owned();
        let text = if self.own_object {
            aurora_i18n::tr!("world-inventory-offers-own-object", object = object)
        } else {
            aurora_i18n::tr!("world-inventory-offers-object", object = object, owner = owner.to_owned())
        };
        format!("{text}\n{}", item_label(&self.item_name))
    }

    /// The owner's name in the offer's text and its link (dialog_names.rs),
    /// as `names` names them (`giver_names`): the owner RLVa's `@shownames`
    /// hid when the offer arrived is their stand-in. Blocking keeps
    /// [`TaskOffer::owner_name`].
    fn shown_owner_name(&self, world: &World, names: NameScope, owner: Uuid) -> String {
        dialog_names::shown(&self.owner_name_in(world, names, owner)).into_owned()
    }

    fn owner_name(&self, world: &World, owner: Uuid) -> String {
        self.owner_name_in(world, NameScope::World, owner)
    }

    fn owner_name_in(&self, world: &World, names: NameScope, owner: Uuid) -> String {
        if self.from_group {
            world
                .search
                .groups
                .get(&owner)
                .map(|g| g.name.clone())
                .or_else(|| world.groups.groups.iter().find(|g| g.id == owner).map(|g| g.name.clone()))
                .unwrap_or_else(|| owner.to_string())
        } else {
            let name = world.social.name_in(names, &owner);
            if name == "…" { owner.to_string() } else { name }
        }
    }
}

/// The names an offer's notice shows of its giver: everyone's when the
/// offer names them (`World::rlva_names_giver`, `Notifications::names_of`),
/// else the world's, which gives the stand-in of a resident RLVa hides.
fn giver_names(named: bool) -> NameScope {
    if named { NameScope::Roster } else { NameScope::World }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Pending,
    Kept,
    Rejected,
}

#[derive(Debug, Clone)]
pub struct Offer {
    pub id: u64,
    pub from: Uuid,
    /// The item's or folder's name, as the offer gives it.
    pub item: String,
    /// Delivered inventory UUID from the resident's offer bucket, never its transaction.
    pub object: Uuid,
    /// LLAssetType of what is given (AT_CATEGORY for a folder).
    pub asset_type: i8,
    pub answer: Answer,
    /// Its entry in the notification centre.
    pub notification: u64,
    /// The card names the giver: what inventory_offer_handler
    /// (indra/newview/llimprocessing.cpp) decided when the offer arrived
    /// (`World::rlva_names_giver`). The giver RLVa's `@shownames` hid then
    /// stays their stand-in on the card, as the `rlvanonym` link of the
    /// Legacy Viewer's notification does.
    pub named: bool,
}

impl Offer {
    /// LLInventoryHandler's explicit selection, available only after keeping the gift.
    pub(crate) fn inventory_link(&self, label: &str) -> Option<String> {
        (self.answer == Answer::Kept && !self.object.is_nil()).then(|| {
            let name = crate::slurl::encode(label);
            format!("secondlife:///app/inventory/{}/select?name={name}", self.object)
        })
    }
}

/// Offers kept once answered, for the conversation cards.
const MAX_OFFERS: usize = 50;

/// What a kept offer gave, to show in the inventory once it is there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reveal {
    /// A resident's offer names it (inventory_offer_callback, IOR_ACCEPT:
    /// openInventoryPanelAndSetSelection on mObjectID,
    /// indra/newview/llviewermessage.cpp, originally LGPL 2.1).
    Known(Uuid),
    /// An object's offer does not: the first folder or item the server adds
    /// after the answer (LLOpenTaskOffer, an LLInventoryAddedObserver).
    Added,
}

/// How long a kept offer waits for its delivery, and for its move when
/// Received items files it (as long as that filing waits for a name).
const REVEAL_WAIT: Duration = Duration::from_secs(120);

/// LLViewerFolderType's quiet folders (highlight_offered_object): Trash,
/// Lost and Found, Current Outfit, My Outfits.
const QUIET_FOLDERS: [i32; 4] = [14, 16, 46, 48];

pub struct InventoryOffers {
    pub list: Vec<Offer>,
    next: u64,
    /// AutoAcceptNewInventory, off by default.
    pub auto_accept: bool,
    pub auto_accept_chat: bool,
    pub auto_accept_notify: bool,
    /// ShowInInventory, from the preferences.
    pub show_accepted: bool,
    reveal: Option<(Reveal, Instant)>,
}

impl Default for InventoryOffers {
    fn default() -> Self {
        Self {
            list: Vec::new(),
            next: 0,
            auto_accept: false,
            auto_accept_chat: true,
            auto_accept_notify: true,
            show_accepted: false,
            reveal: None,
        }
    }
}

impl InventoryOffers {
    fn push(&mut self, from: Uuid, item: String, object: Uuid, asset_type: i8, notification: u64, named: bool) -> u64 {
        self.next += 1;
        self.list.push(Offer {
            id: self.next,
            from,
            item,
            object,
            asset_type,
            answer: Answer::Pending,
            notification,
            named,
        });
        if self.list.len() > MAX_OFFERS
            && let Some(i) = self.list.iter().position(|v| v.answer != Answer::Pending)
        {
            self.list.remove(i);
        }
        self.next
    }

    pub fn get(&self, id: u64) -> Option<&Offer> {
        self.list.iter().find(|v| v.id == id)
    }

    /// The entries of the pending offers and their givers: one whose
    /// conversation is in sight raises no toast (LLOfferHandler shows one
    /// only when the IM floater is not open).
    pub fn pending_notifications(&self) -> impl Iterator<Item = (u64, Uuid)> + '_ {
        self.list
            .iter()
            .filter(|v| v.answer == Answer::Pending)
            .map(|v| (v.notification, v.from))
    }
}

/// What an offer gives, from its asset type (the bucket has no inventory
/// type nor wearable type): LLTrans's names of LLAssetType, as the
/// inventory's Kind column says them.
pub fn kind_label(asset_type: i8) -> &'static str {
    match asset_type {
        0 => aurora_i18n::tr!("common-texture"),
        1 => aurora_i18n::tr!("common-sound"),
        2 => aurora_i18n::tr!("inventory-kind-calling-card"),
        3 => aurora_i18n::tr!("common-landmark"),
        5 => aurora_i18n::tr!("common-clothing"),
        6 => aurora_i18n::tr!("common-object"),
        7 => aurora_i18n::tr!("inventory-kind-notecard"),
        AT_CATEGORY => aurora_i18n::tr!("inventory-kind-folder"),
        10 => aurora_i18n::tr!("inventory-kind-script"),
        13 => aurora_i18n::tr!("world-inventory-offers-body-part"),
        20 => aurora_i18n::tr!("inventory-kind-animation"),
        21 => aurora_i18n::tr!("inventory-kind-gesture"),
        49 => aurora_i18n::tr!("inventory-kind-mesh"),
        56 => aurora_i18n::tr!("inventory-kind-settings"),
        57 => aurora_i18n::tr!("inventory-kind-material"),
        _ => aurora_i18n::tr!("inventory-kind-other"),
    }
}

/// The item as the titles of the entry and of the card name it ("an
/// object" without a name); non-breaking spaces keep the quotes on its
/// words.
pub fn item_label(item: &str) -> String {
    if item.is_empty() {
        aurora_i18n::tr!("world-mod-an-object").to_owned()
    } else {
        format!("«\u{a0}{}\u{a0}»", dialog_names::shown(item))
    }
}

impl World {
    pub(super) fn on_task_inventory_offer(&mut self, im: &InstantMessage, object_name: &str) -> bool {
        // inventory_offer_handler removes the appended location from object offers.
        let item = [
            " ( http://slurl.com/secondlife/",
            " ( https://slurl.com/secondlife/",
            " ( http://maps.secondlife.com/secondlife/",
            " ( https://maps.secondlife.com/secondlife/",
        ]
        .iter()
        .filter_map(|prefix| im.message.find(prefix))
        .min()
        .map_or(im.message.as_str(), |end| &im.message[..end]);
        let offer = TaskOffer {
            object_name: object_name.to_owned(),
            item_name: item.to_owned(),
            from_group: im.from_group,
            own_object: im.from_agent_id == self.agent_id,
        };
        if !im.from_agent_id.is_nil() && !offer.own_object {
            if offer.from_group {
                if !self.search.groups.contains_key(&im.from_agent_id) {
                    self.search.request_group(im.from_agent_id);
                }
            } else {
                self.social.want_name(im.from_agent_id);
            }
        }
        // "Only filter if the object owner is a nearby agent"
        // (inventory_offer_handler): the owner alone, never the object's name
        let named = self.rlva_names_giver(im.from_agent_id, true);
        let owner = offer.shown_owner_name(self, giver_names(named), im.from_agent_id);
        let target = if offer.from_group {
            NameTarget::Group(im.from_agent_id)
        } else {
            NameTarget::Avatar(im.from_agent_id)
        };
        let own_object = offer.own_object;
        let id = self.notifications.push(
            Kind::Inventory,
            dialog_names::shown(object_name).into_owned(),
            offer.body(&owner),
            Data::Inventory {
                from: im.from_agent_id,
                tx: im.session_id,
                asset_type: im.binary_bucket.first().map(|b| *b as i8).unwrap_or(-1),
                object: Uuid::nil(),
                task: true,
                task_offer: Some(offer),
            },
        );
        // (a hidden owner is plain text: `refresh_task_inventory_offers`)
        if !own_object && named {
            self.notifications.link_name(id, owner, target);
        }
        if self.receive_inventory_offer(im, id) {
            return true;
        }
        self.ui_sounds.push(crate::ui_sound::UiSound::InventoryOffer);
        false
    }

    /// inventory_offer_handler (indra/newview/llimprocessing.cpp, LGPL 2.1):
    /// AutoAcceptNewInventory covers every asset type, but give-to-RLVa
    /// offers retain their explicit confirmation. Use the ordinary answer
    /// so filing, destination folders, history and ShowInInventory agree.
    /// Returns whether the offer was answered without a prompt.
    /// Decides once whether the notice `id` names the resident `from` it is
    /// about (`rlva_names_giver`), and returns it: the notice keeps the name
    /// from then on, or the stand-in it was written with
    /// (inventory_offer_handler, indra/newview/llimprocessing.cpp). The
    /// rest of its text stays as it was given: UserGiveItem and
    /// ObjectGiveItem go through no RlvUtil::filterNames nor filterLocation,
    /// so the name of the item, and of the object that gives it, keep the
    /// names and the places they hold.
    pub(super) fn name_giver_notice(&mut self, id: u64, from: Uuid, from_object: bool) -> bool {
        let named = self.rlva_names_giver(from, from_object);
        if named {
            self.notifications.name_resident(id);
        }
        self.notifications.keep_as_written(id);
        named
    }

    pub(super) fn receive_inventory_offer(&mut self, im: &InstantMessage, id: u64) -> bool {
        // Whether the notice names the giver is decided here, once, before
        // its card opens the conversation (`rlva_names_giver`): the notice
        // keeps the name, or loses it now instead of at the next frame's
        // rewrite (`refresh_rlva_privacy`), which would show it meanwhile.
        let from_object = self
            .notifications
            .list
            .iter()
            .any(|n| n.id == id && matches!(n.data, Data::Inventory { task: true, .. }));
        let named = self.name_giver_notice(id, im.from_agent_id, from_object);
        if !self.rlva_inventory_offer(im, id) {
            self.received_items.offer(im, id);
        }
        if !self.inventory_offers.auto_accept || self.rlva_inventory_offer_needs_confirmation(im) {
            return false;
        }
        let feedback = (self.inventory_offers.auto_accept_chat || self.inventory_offers.auto_accept_notify)
            .then(|| self.auto_accepted_inventory_text(im, id))
            .flatten();
        let (commands, _) = self.respond_notification(id, Response::Accept);
        self.groups.out.extend(commands);
        if let Some(text) = feedback {
            // inventory_offer_handler: local system chat and IMSystemMessageTip
            // are independent; either enables UISndInventoryOffer, once.
            if self.inventory_offers.auto_accept_chat {
                self.system_notice(text.clone(), super::notice_topics::Topic::Items);
            }
            if self.inventory_offers.auto_accept_notify {
                let notice = self
                    .notifications
                    .push(Kind::Inventory, aurora_i18n::tr!("received-auto-accepted-title"), text, Data::None);
                if named {
                    self.notifications.name_resident(notice);
                }
            }
            self.ui_sounds.push(crate::ui_sound::UiSound::InventoryOffer);
        }
        true
    }

    /// InvOfferAutoAcceptUser / InvOfferAutoAcceptObject: use the shared
    /// application links, which retain inspect actions. The giver the offer
    /// does not name (`rlva_names_giver`, the same test again in
    /// inventory_offer_handler) is their stand-in in plain text: the Legacy
    /// Viewer writes the `rlvanonym` link there, which shows the stand-in
    /// and opens nothing (LLUrlEntryAgentRLVAnonymizedName,
    /// indra/llui/llurlentry.cpp).
    fn auto_accepted_inventory_text(&self, im: &InstantMessage, id: u64) -> Option<String> {
        let n = self.notifications.list.iter().find(|n| n.id == id)?;
        let Data::Inventory {
            asset_type, task_offer, ..
        } = &n.data
        else {
            return None;
        };
        let kind = kind_label(*asset_type).to_lowercase();
        Some(if let Some(offer) = task_offer {
            let object = format!(
                "secondlife:///app/objectim/{}?name={}&owner={}&groupowned={}",
                Uuid::nil(),
                crate::slurl::encode(&dialog_names::shown(&offer.object_name)),
                im.from_agent_id,
                u8::from(offer.from_group),
            );
            aurora_i18n::tr!(
                "received-auto-accepted-object",
                object = object,
                kind = kind,
                item = item_label(&offer.item_name)
            )
        } else {
            let name = if self.notifications.names_resident(id) {
                super::teleport_invites::agent_link(im.from_agent_id)
            } else {
                self.rlva_anonymous_from(im.from_agent_id, &im.from_name).display_name.clone()
            };
            aurora_i18n::tr!(
                "received-auto-accepted-resident",
                name = name,
                kind = kind,
                item = item_label(&im.message)
            )
        })
    }

    /// LLPostponedOfferNotification resolves avatar/group names asynchronously.
    pub(super) fn refresh_task_inventory_offers(&mut self) {
        let bodies: Vec<_> = self
            .notifications
            .list
            .iter()
            .enumerate()
            .filter_map(|(index, n)| {
                let Data::Inventory {
                    from,
                    task_offer: Some(offer),
                    ..
                } = &n.data
                else {
                    return None;
                };
                // As the offer named them when it arrived. The owner it
                // hid then stays their stand-in, in plain text, whatever
                // `@shownames` does since: inventory_offer_handler writes
                // the `rlvanonym` link once, which shows the stand-in and
                // opens nothing.
                let names = self.notifications.names_of(n);
                let hidden = self.rlva.enabled() && !offer.from_group && !offer.own_object && !self.notifications.names_resident(n.id);
                let owner = if hidden {
                    self.rlva_anonymous(*from).display_name.clone()
                } else {
                    offer.shown_owner_name(self, names, *from)
                };
                let target = if offer.from_group {
                    NameTarget::Group(*from)
                } else {
                    NameTarget::Avatar(*from)
                };
                Some((
                    index,
                    offer.body(&owner),
                    (!offer.own_object && !hidden).then_some(NameLink { name: owner, target }),
                ))
            })
            .collect();
        for (index, body, link) in bodies {
            self.notifications.list[index].body = body;
            self.notifications.list[index].name_links = link.into_iter().collect();
        }
    }

    /// inventory_task_offer_callback / inventory_offer_mute_callback in
    /// indra/newview/llviewermessage.cpp: mute the owner (avatar or group),
    /// decline the gift and purge other queued offers from that owner.
    pub(super) fn mute_task_inventory_owner(&mut self, id: u64, owner: Uuid, offer: &TaskOffer) -> Vec<NetCommand> {
        if owner.is_nil() || offer.own_object {
            return Vec::new();
        }
        let name = offer.owner_name(self, owner);
        let kind = if offer.from_group {
            super::mutes::MuteType::Group
        } else {
            super::mutes::MuteType::Agent
        };
        let (out, _) = self.notifications.respond(id, Response::Block, |_| Uuid::nil(), Uuid::nil());
        if let Err(error) = self.block(owner, &name, kind) {
            self.system_notice(error, crate::world::notice_topics::Topic::Items);
        }
        out
    }

    /// A resident's offer whose entry `notification` was just listed: its
    /// card in the conversation with the giver, opened if needed and
    /// counted as unread, and its text in the giver's transcript. Blocked
    /// givers were answered before (`im_blocked`): no conversation for them.
    pub(super) fn on_inventory_offer_card(&mut self, im: &InstantMessage, from: &str, asset_type: i8, notification: u64) {
        let object = super::notifications::offered_object(&im.binary_bucket).map_or(Uuid::nil(), |(_, id)| id);
        // as the entry names the giver (`receive_inventory_offer`, asked
        // before this card opened their conversation): the Legacy Viewer
        // shows and logs the one notification text, with the link of the
        // name or the `rlvanonym` one
        let named = self.notifications.names_resident(notification);
        let offer = self
            .inventory_offers
            .push(im.from_agent_id, im.message.clone(), object, asset_type, notification, named);
        let (giver, logged) = if named {
            (from.to_string(), super::teleport_invites::agent_link(im.from_agent_id))
        } else {
            let stand_in = self.rlva_anonymous_from(im.from_agent_id, &im.from_name).display_name.clone();
            (stand_in.clone(), stand_in)
        };
        let title = aurora_i18n::tr!("world-mod-gives-you", from = giver, what = item_label(&im.message));
        self.add_card(im, ChatKind::InventoryOffer(offer), title);
        let line = self.notice_line(aurora_i18n::tr!(
            "world-inventory-offers-received",
            name = logged,
            kind = kind_label(asset_type).to_lowercase(),
            item = im.message.clone()
        ));
        self.transcript_only(im.from_agent_id, &line);
    }

    /// Our offer of an item or a folder went out
    /// (LLGiveInventory::logInventoryOffer, indra/newview/llgiveinventory.cpp):
    /// "Inventory item 'X' offered" in the conversation with the resident
    /// without counting as unread, or only in their transcript when none is
    /// open. `in_session`: it was dropped on that conversation.
    ///
    /// Under RLVa's `@shownames`, what goes to a resident around us whose
    /// name is hidden, and was dropped neither on their conversation nor
    /// with their profile shown, is only told in the nearby chat, with
    /// their stand-in and without the item ("inventory_item_offered_rlv"):
    /// their conversation and their transcript would name them.
    pub fn inventory_offer_sent(&mut self, to: Uuid, name: &str, folder: bool, in_session: bool) {
        if !in_session
            && self.rlva.enabled()
            && !self.rlva_can_name(to)
            && self.rlva_is_nearby(to)
            && !self.rlva.open_profiles.contains(&to)
        {
            let name = self.rlva_anonymous(to).display_name.clone();
            self.system_message(aurora_i18n::tr!("world-inventory-offers-sent-anonymous", name = name));
            return;
        }
        let item = item_label(name);
        let text = if folder {
            aurora_i18n::tr!("world-inventory-offers-folder-sent", item = item)
        } else {
            aurora_i18n::tr!("world-inventory-offers-item-sent", item = item)
        };
        self.note_with(to, text);
    }

    /// A kept offer (its Accept, never its closing: "we simply won't
    /// highlight it") waits for what it gave, with ShowInInventory. The
    /// last one kept wins.
    pub(super) fn reveal_kept_offer(&mut self, object: Uuid, task: bool) {
        if !self.inventory_offers.show_accepted {
            return;
        }
        let what = match (task, object.is_nil()) {
            (true, _) => Reveal::Added,
            (false, false) => Reveal::Known(object),
            (false, true) => return,
        };
        self.inventory_offers.reveal = Some((what, Instant::now()));
    }

    /// A server update (BulkUpdateInventory, UpdateCreateInventoryItem),
    /// before it is applied: what it adds that the inventory did not know is
    /// what an object's kept offer gave, its top folder (llGiveInventoryList)
    /// or else its last item, as LLOpenTaskOffer::done shows added.back().
    pub(super) fn reveal_added(&mut self, folders: &[InvFolder], items: &[InvItem]) {
        let Some((Reveal::Added, since)) = self.inventory_offers.reveal else {
            return;
        };
        let inv = &self.inventory;
        let new: Vec<&InvFolder> = folders.iter().filter(|f| !inv.folders.contains_key(&f.id)).collect();
        let added = new
            .iter()
            .find(|f| !new.iter().any(|o| o.id == f.parent))
            .map(|f| f.id)
            .or_else(|| {
                items
                    .iter()
                    .rev()
                    .find(|it| !it.id.is_nil() && !inv.items.contains_key(&it.id))
                    .map(|it| it.id)
            });
        if let Some(id) = added {
            self.inventory_offers.reveal = Some((Reveal::Known(id), since));
        }
    }

    /// What the inventory now shows, once (the app opens its window on it):
    /// the kept offer's item or folder once the inventory knows it and
    /// Received items has filed it, unless it is in a quiet folder.
    pub fn take_inventory_reveal(&mut self, now: Instant) -> Option<Uuid> {
        let (what, since) = self.inventory_offers.reveal?;
        if !self.inventory_offers.show_accepted || now.duration_since(since) >= REVEAL_WAIT {
            self.inventory_offers.reveal = None;
            return None;
        }
        let Reveal::Known(id) = what else { return None };
        let inv = &self.inventory;
        if actions::parent(inv, id).is_none() || self.received_items.filing(id) {
            return None;
        }
        self.inventory_offers.reveal = None;
        (!QUIET_FOLDERS.iter().any(|t| actions::in_type(inv, id, *t))).then_some(id)
    }

    /// Answer an offer from its card: through its entry, as its Keep or
    /// Reject button would (the same reply, Trash and history), which
    /// closes the entry and leaves the card with its answer.
    pub fn answer_inventory_offer(&mut self, offer: u64, keep: bool) -> Vec<NetCommand> {
        let Some(v) = self.inventory_offers.get(offer) else {
            return Vec::new();
        };
        if v.answer != Answer::Pending {
            return Vec::new();
        }
        let notification = v.notification;
        let r = if keep { Response::Accept } else { Response::Decline };
        let (out, _) = self.respond_notification(notification, r);
        // an entry that left the list some other way has no answer to give
        self.close_orphan_inventory_offers();
        out
    }

    /// The entry `notification` of a resident's offer was answered with
    /// `outcome` (kept, or rejected): its card says so.
    pub(super) fn inventory_offer_answered(&mut self, notification: u64, outcome: Outcome) {
        if let Some(v) = self
            .inventory_offers
            .list
            .iter_mut()
            .find(|v| v.notification == notification && v.answer == Answer::Pending)
        {
            v.answer = if outcome == Outcome::Kept { Answer::Kept } else { Answer::Rejected };
            if let Some(item) = v.inventory_link(&item_label(&v.item)) {
                let from = v.from;
                // Keep the confirmation in the transcript too, where the live card
                // is no longer available after reopening the conversation.
                self.note_with(from, aurora_i18n::tr!("world-inventory-offers-kept", item = item));
            }
        }
    }

    /// Pending cards whose entry left the list unanswered (its giver
    /// blocked, a full list): what they gave stays in the inventory, as
    /// when the offer is closed (inventory_offer_callback, "close button
    /// probably").
    pub(super) fn close_orphan_inventory_offers(&mut self) {
        let listed = &self.notifications.list;
        for v in self.inventory_offers.list.iter_mut() {
            if v.answer == Answer::Pending && !listed.iter().any(|n| n.id == v.notification) {
                v.answer = Answer::Kept;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::chat_log;
    use super::super::notifications::{Data, Kind, im};
    use super::*;
    use crate::scene::avatar::AvatarLibrary;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::Duration;

    const ME: Uuid = Uuid::from_u128(1);
    const JANE: Uuid = Uuid::from_u128(2);
    const TX: Uuid = Uuid::from_u128(0x10);
    const ITEM: Uuid = Uuid::from_u128(0x20);

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aurora-inv-offers-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn world(root: &Path) -> World {
        let mut w = World::new(Arc::new(AvatarLibrary::load()));
        w.chat_log = chat_log::ChatLog::in_folder(root.to_path_buf());
        w.chat_log.set_grid("agni".into());
        w.agent_id = ME;
        w.chat_log.login(ME);
        w.social.names.insert(JANE, "Jane Doe".into());
        w
    }

    /// An offer of a notecard (asset type 7) already in our inventory, or
    /// an object's offer (only the type in the bucket).
    fn offer(dialog: u8, name: &str) -> aurora_net::InstantMessage {
        let mut bucket = vec![7u8];
        if dialog == im::INVENTORY_OFFERED {
            bucket.extend_from_slice(ITEM.as_bytes());
        }
        aurora_net::InstantMessage {
            from_agent_id: JANE,
            from_name: "Jane Doe".into(),
            to_agent_id: ME,
            dialog,
            session_id: TX,
            message: name.into(),
            offline: false,
            from_group: false,
            binary_bucket: bucket,
            timestamp: 0,
        }
    }

    fn transcript(w: &mut World) -> Vec<String> {
        assert!(w.chat_log.flush(Duration::from_secs(10)));
        let Some(path) = w.chat_log.file_of("Jane Doe") else {
            return Vec::new();
        };
        chat_log::store::read_tail(&path)
            .unwrap_or_default()
            .into_iter()
            .map(|e| e.text)
            .collect()
    }

    fn clean(w: &World, root: &Path) {
        assert!(w.chat_log.flush(Duration::from_secs(10)));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The answers sent to Jane about the offer.
    fn answers(cmds: &[NetCommand]) -> Vec<u8> {
        cmds.iter()
            .filter_map(|c| match c {
                NetCommand::SendImDialog {
                    to: JANE, dialog, id: TX, ..
                } => Some(*dialog),
                _ => None,
            })
            .collect()
    }

    /// A world where RLVa's `@shownames` hides Jane.
    fn hiding_names(root: &Path) -> World {
        let mut w = world(root);
        w.rlva.configure(true);
        w.rlva_console_input("@shownames=n");
        w.refresh_rlva_privacy();
        assert!(!w.rlva_can_name(JANE));
        w
    }

    /// Jane stands in a region around.
    fn around(w: &mut World) {
        w.coarse.entry(1).or_default().push((JANE, glam::Vec3::ONE));
    }

    /// inventory_offer_handler under `@shownames`: the giver is hidden only
    /// while they stand around and no conversation with them is open; the
    /// notice, its card and the entry of the offer agree, and the next
    /// frame's rewrite keeps what was decided.
    #[test]
    fn shownames_hides_a_giver_around_us_unless_we_already_talk() {
        for (near, talking, named) in [(false, false, true), (true, false, false), (true, true, true)] {
            let root = temp();
            let mut w = hiding_names(&root);
            if near {
                around(&mut w);
            }
            if talking {
                w.social.session_mut(JANE);
            }
            assert_eq!(w.rlva_names_giver(JANE, false), named, "{near}/{talking}");
            w.on_im(offer(im::INVENTORY_OFFERED, "Welcome note"));
            for frame in 0..2 {
                let case = format!("{near}/{talking}, frame {frame}");
                let n = &w.notifications.list[0];
                assert_eq!(n.title.contains("Jane"), named, "{case}: {}", n.title);
                // the stand-in is plain text: no link at all
                assert_eq!(n.name_links.iter().any(|l| l.name.contains("Jane")), named, "{case}");
                assert_eq!(n.name_links.is_empty(), !named, "{case}");
                assert_eq!(w.notifications.names_of(n), giver_names(named), "{case}");
                let v = &w.inventory_offers.list[0];
                assert_eq!(v.named, named, "{case}");
                let card = w.social.ims[0].lines.back().unwrap();
                assert!(matches!(card.kind, ChatKind::InventoryOffer(_)), "{case}");
                assert_eq!(card.text.contains("Jane"), named, "{case}: {}", card.text);
                w.refresh_rlva_privacy();
            }
            clean(&w, &root);
        }
    }

    /// IM_INVENTORY_ACCEPTED and IM_INVENTORY_DECLINED: who answered what we
    /// gave is hidden like a giver, only while they stand around and no
    /// conversation with them is open, and the next frames keep it.
    #[test]
    fn shownames_hides_who_answered_what_we_gave_only_around_us() {
        for dialog in [im::INVENTORY_ACCEPTED, im::INVENTORY_DECLINED] {
            for (near, talking, named) in [(false, false, true), (true, false, false), (true, true, true)] {
                let root = temp();
                let mut w = hiding_names(&root);
                if near {
                    around(&mut w);
                }
                if talking {
                    w.social.session_mut(JANE);
                }
                w.on_im(offer(dialog, ""));
                for frame in 0..2 {
                    let n = &w.notifications.list[0];
                    assert_eq!(
                        n.title.contains("Jane"),
                        named,
                        "{dialog} {near}/{talking}, frame {frame}: {}",
                        n.title
                    );
                    assert_eq!(w.notifications.names_of(n), giver_names(named));
                    w.refresh_rlva_privacy();
                }
                clean(&w, &root);
            }
        }
    }

    /// UserGiveItem and ObjectGiveItem are written once, with the name of
    /// who gives or their stand-in, and go through no RlvUtil::filterNames
    /// nor filterLocation (inventory_offer_handler,
    /// indra/newview/llimprocessing.cpp): under `@shownames` and `@showloc`
    /// the name of the item, and of the object that gives it, keep the
    /// names and the places they hold.
    #[test]
    fn an_offer_notice_hides_who_gives_and_rewrites_nothing_else() {
        const ITEM_NAME: &str = "Jane Doe and the map of Secret Parcel";
        const BOX_NAME: &str = "Jane Doe and the box at Secret Parcel";
        for task in [false, true] {
            let root = temp();
            let mut w = hiding_names(&root);
            around(&mut w);
            w.parcel_name = "Secret Parcel".into();
            w.rlva_console_input("@showloc=n");
            w.refresh_rlva_privacy();
            let hidden = w.rlva_filter_text("the map of Secret Parcel").into_owned();
            assert!(!hidden.contains("Secret Parcel"), "a text loses the place: {hidden}");
            let dialog = if task { im::TASK_INVENTORY_OFFERED } else { im::INVENTORY_OFFERED };
            let mut gift = offer(dialog, ITEM_NAME);
            if task {
                gift.from_name = BOX_NAME.into();
            }
            w.on_im(gift);
            for frame in 0..2 {
                let case = format!("task {task}, frame {frame}");
                let n = &w.notifications.list[0];
                let text = format!("{} / {}", n.title, n.body);
                assert!(text.contains(ITEM_NAME), "{case}: {text}");
                if task {
                    assert_eq!(n.title, BOX_NAME, "{case}");
                    assert!(n.body.contains(BOX_NAME), "{case}: {}", n.body);
                }
                // who gives is their stand-in, in plain text
                let rest = text.replace(ITEM_NAME, "").replace(BOX_NAME, "");
                assert!(!rest.contains("Jane"), "{case}: {text}");
                assert!(n.name_links.is_empty(), "{case}");
                w.refresh_task_inventory_offers();
                w.refresh_rlva_privacy();
            }
            clean(&w, &root);
        }
    }

    /// RlvUIEnabler::hasOpenProfile (indra/newview/rlvui.cpp): a giver
    /// around us whose profile is open is named; the owner of an object
    /// that gives is not.
    #[test]
    fn shownames_names_a_giver_around_us_whose_profile_is_open() {
        let root = temp();
        let mut w = hiding_names(&root);
        around(&mut w);
        assert!(!w.rlva_names_giver(JANE, false));
        w.rlva.open_profiles.insert(JANE);
        assert!(w.rlva_names_giver(JANE, false));
        assert!(!w.rlva_names_giver(JANE, true));
        clean(&w, &root);
    }

    /// An object's offer hides its owner whenever they stand around, the
    /// conversation with them open or not, and never the object's name.
    #[test]
    fn shownames_hides_the_owner_of_an_object_that_gives_only_around_us() {
        for (near, named) in [(false, true), (true, false)] {
            let root = temp();
            let mut w = hiding_names(&root);
            w.social.session_mut(JANE);
            if near {
                around(&mut w);
            }
            assert_eq!(w.rlva_names_giver(JANE, true), named);
            let mut gift = offer(im::TASK_INVENTORY_OFFERED, "Welcome note");
            gift.from_name = "Gift box".into();
            w.on_im(gift);
            for frame in 0..2 {
                let n = &w.notifications.list[0];
                assert_eq!(n.title, "Gift box", "{near}, frame {frame}");
                assert_eq!(n.body.contains("Jane"), named, "{near}, frame {frame}: {}", n.body);
                assert_eq!(n.name_links.iter().any(|l| l.name.contains("Jane")), named, "{near}, frame {frame}");
                assert_eq!(n.name_links.is_empty(), !named, "{near}, frame {frame}");
                // a name that arrives rebuilds the text the same way
                w.refresh_task_inventory_offers();
                w.refresh_rlva_privacy();
            }
            // and the restriction lifted leaves the notice as it came
            w.rlva_console_input("@shownames=y");
            w.refresh_task_inventory_offers();
            w.refresh_rlva_privacy();
            let n = &w.notifications.list[0];
            assert_eq!(n.body.contains("Jane"), named, "{near}, lifted: {}", n.body);
            assert_eq!(n.name_links.is_empty(), !named, "{near}, lifted");
            clean(&w, &root);
        }
    }

    /// InvOfferAutoAcceptUser: the link of the giver's name, or the
    /// stand-in of the giver the offer hides, with no link to their profile.
    #[test]
    fn shownames_auto_accept_feedback_hides_a_giver_around_us() {
        for (near, named) in [(false, true), (true, false)] {
            let root = temp();
            let mut w = hiding_names(&root);
            w.inventory_offers.auto_accept = true;
            if near {
                around(&mut w);
            }
            w.on_im(offer(im::INVENTORY_OFFERED, "Welcome note"));
            let text = w.chat.back().unwrap().text.clone();
            assert!(text.contains("Welcome note"), "{text}");
            assert_eq!(text.contains(&super::super::teleport_invites::agent_link(JANE)), named, "{text}");
            assert!(named || (!text.contains("Jane") && !text.contains("/app/agent/")), "{text}");
            let n = &w.notifications.list[0];
            assert_eq!(n.body, text);
            assert_eq!(w.notifications.names_of(n), giver_names(named));
            clean(&w, &root);
        }
    }

    #[test]
    fn auto_accept_inventory_answers_all_item_types_and_folders_without_prompts() {
        for dialog in [im::INVENTORY_OFFERED, im::TASK_INVENTORY_OFFERED] {
            let root = temp();
            let mut w = world(&root);
            w.inventory_offers.auto_accept = true;
            w.inventory_offers.auto_accept_chat = false;
            w.inventory_offers.auto_accept_notify = false;
            for asset_type in [0, 1, 3, 5, 6, 7, 8, 10, 13, 20, 21, 49, 56, 57] {
                for offline in [false, true] {
                    let mut gift = offer(dialog, "Gift");
                    gift.binary_bucket[0] = asset_type;
                    gift.offline = offline;
                    w.on_im(gift);
                    let commands = w.take_social_commands();
                    let expected = if dialog == im::INVENTORY_OFFERED {
                        im::INVENTORY_ACCEPTED
                    } else {
                        im::TASK_INVENTORY_ACCEPTED
                    };
                    assert_eq!(answers(&commands), [expected], "{dialog}/{asset_type}/{offline}");
                    assert!(commands.iter().any(|c| matches!(c,
                        NetCommand::SendImDialog { bucket, .. } if bucket == w.inventory.root.as_bytes()
                    )));
                    assert!(w.notifications.list.is_empty());
                    assert!(w.inventory_offers.list.is_empty());
                    assert!(w.social.ims.is_empty());
                    assert!(w.ui_sounds.is_empty());
                    assert!(w.attention.is_empty());
                }
            }
            w.record_notification_history();
            assert_eq!(w.notification_history.entries().count(), 28);
            assert!(w.notification_history.entries().all(|entry| entry.outcome == Outcome::Kept));
            // Disabling affects the next offer immediately.
            w.inventory_offers.auto_accept = false;
            w.on_im(offer(dialog, "Manual gift"));
            assert_eq!(w.notifications.list.len(), 1);
            assert!(answers(&w.take_social_commands()).is_empty());
            clean(&w, &root);
        }
    }

    #[test]
    fn auto_accept_inventory_feedback_has_independent_chat_notification_and_one_sound() {
        let defaults = InventoryOffers::default();
        assert!(!defaults.auto_accept);
        assert!(defaults.auto_accept_chat && defaults.auto_accept_notify);
        for task in [false, true] {
            for chat in [false, true] {
                for notify in [false, true] {
                    let root = temp();
                    let mut w = world(&root);
                    w.inventory_offers.auto_accept = true;
                    w.inventory_offers.auto_accept_chat = chat;
                    w.inventory_offers.auto_accept_notify = notify;
                    let mut gift = offer(
                        if task { im::TASK_INVENTORY_OFFERED } else { im::INVENTORY_OFFERED },
                        "Welcome note",
                    );
                    if task {
                        gift.from_name = "Gift & café + 50%?".into();
                        gift.message.push_str(" ( https://maps.secondlife.com/secondlife/Demo/1/2/3 )");
                    }
                    w.on_im(gift);
                    let commands = w.take_social_commands();
                    assert_eq!(
                        answers(&commands),
                        [if task {
                            im::TASK_INVENTORY_ACCEPTED
                        } else {
                            im::INVENTORY_ACCEPTED
                        }]
                    );
                    assert_eq!(w.chat.len(), usize::from(chat));
                    assert_eq!(w.notifications.list.len(), usize::from(notify));
                    assert_eq!(
                        w.ui_sounds,
                        if chat || notify {
                            vec![crate::ui_sound::UiSound::InventoryOffer]
                        } else {
                            vec![]
                        }
                    );
                    assert!(w.social.ims.is_empty() && w.inventory_offers.list.is_empty());
                    let text = w
                        .chat
                        .back()
                        .map(|line| &line.text)
                        .or_else(|| w.notifications.list.first().map(|n| &n.body));
                    if let Some(text) = text {
                        assert!(text.contains("Welcome note") && text.contains(&kind_label(7).to_lowercase()));
                        if task {
                            assert!(text.contains("/objectim/") && text.contains(&crate::slurl::encode("Gift & café + 50%?")));
                            assert!(!text.contains("maps.secondlife.com"));
                        } else {
                            assert!(text.contains(&super::super::teleport_invites::agent_link(JANE)));
                        }
                    }
                    if chat {
                        let line = w.chat.back().unwrap();
                        assert_eq!(line.kind, ChatKind::System);
                        assert!(line.source.is_nil());
                        assert_eq!(w.notice_topics.of(line), super::super::notice_topics::Topic::Items);
                    }
                    if notify {
                        let n = &w.notifications.list[0];
                        assert!(matches!(n.data, Data::None));
                        assert!(!n.interactive() && !n.close_declines() && !n.stays_until_answered());
                        if chat {
                            assert_eq!(n.body, w.chat.back().unwrap().text);
                        }
                        let id = n.id;
                        assert!(w.respond_notification(id, Response::Dismiss).0.is_empty());
                        assert!(w.take_social_commands().is_empty());
                    }
                    clean(&w, &root);
                }
            }
        }
    }

    #[test]
    fn auto_accept_inventory_feedback_never_reports_blocked_or_pending_rlva_gifts_as_received() {
        for blocked in [false, true] {
            let root = temp();
            let mut w = world(&root);
            w.inventory_offers.auto_accept = true;
            if blocked {
                w.block(JANE, "Jane Doe", super::super::mutes::MuteType::Agent).unwrap();
            } else {
                // a gift for #RLV waits for its answer while #RLV exists
                // (RlvInventory::isGiveToRLVOffer)
                w.rlva_configure(true);
                w.inventory.created_folders(vec![InvFolder {
                    id: Uuid::from_u128(0x31),
                    parent: w.inventory.root,
                    type_default: -1,
                    name: "#RLV".into(),
                    ..Default::default()
                }]);
            }
            w.chat.clear();
            w.ui_sounds.clear();
            for task in [false, true] {
                let mut gift = offer(
                    if task { im::TASK_INVENTORY_OFFERED } else { im::INVENTORY_OFFERED },
                    if task { "'#RLV/~Wardrobe/Cuffs'" } else { "#RLV/~Wardrobe/Cuffs" },
                );
                gift.binary_bucket[0] = AT_CATEGORY as u8;
                w.on_im(gift);
                assert!(w.chat.is_empty());
                assert!(w.notifications.list.iter().all(|n| matches!(n.data, Data::Inventory { .. })));
            }
            clean(&w, &root);
        }
    }

    #[test]
    fn auto_accept_inventory_respects_mutes_and_shared_rlva_offer_consent() {
        let root = temp();
        let mut w = world(&root);
        w.inventory_offers.auto_accept = true;
        w.inventory_offers.auto_accept_chat = false;
        w.inventory_offers.auto_accept_notify = false;
        w.block(JANE, "Jane Doe", super::super::mutes::MuteType::Agent).unwrap();
        for dialog in [im::INVENTORY_OFFERED, im::TASK_INVENTORY_OFFERED] {
            w.on_im(offer(dialog, "Blocked gift"));
            let expected = if dialog == im::INVENTORY_OFFERED {
                im::INVENTORY_DECLINED
            } else {
                im::TASK_INVENTORY_DECLINED
            };
            assert_eq!(answers(&w.take_social_commands()), [expected]);
            assert!(w.notifications.list.is_empty());
        }
        clean(&w, &root);

        // RlvInventory::isGiveToRLVOffer (rlvinventory.cpp): only while
        // giving to #RLV is allowed and #RLV exists; otherwise the offer is
        // accepted like any other.
        for enabled in [false, true] {
            for allow_filing in [false, true] {
                for shared in [false, true] {
                    let root = temp();
                    let mut w = world(&root);
                    w.inventory_offers.auto_accept = true;
                    w.inventory_offers.auto_accept_chat = false;
                    w.inventory_offers.auto_accept_notify = false;
                    w.rlva_configure(enabled);
                    w.rlva.preferences.allow_shared_offers = allow_filing;
                    if shared {
                        w.inventory.created_folders(vec![InvFolder {
                            id: Uuid::from_u128(0x31),
                            parent: w.inventory.root,
                            type_default: -1,
                            name: "#RLV".into(),
                            ..Default::default()
                        }]);
                    }
                    let asks = enabled && allow_filing && shared;
                    for dialog in [im::INVENTORY_OFFERED, im::TASK_INVENTORY_OFFERED] {
                        let task = dialog == im::TASK_INVENTORY_OFFERED;
                        let mut gift = offer(dialog, if task { "'#RLV/~Wardrobe/Cuffs'" } else { "#RLV/~Wardrobe/Cuffs" });
                        gift.binary_bucket[0] = AT_CATEGORY as u8;
                        w.on_im(gift);
                        assert_eq!(
                            answers(&w.take_social_commands()).is_empty(),
                            asks,
                            "rlva {enabled}, filing {allow_filing}, #RLV {shared}, task {task}"
                        );
                        if asks {
                            let id = w.notifications.list.last().expect("shared offer confirmation").id;
                            let commands = w.respond_notification(id, Response::Accept).0;
                            assert_eq!(answers(&commands).len(), 1);
                        }
                        assert!(w.notifications.list.is_empty());
                    }
                    clean(&w, &root);
                }
            }
        }
    }

    #[test]
    fn auto_accept_inventory_uses_the_normal_destination_and_optional_reveal() {
        for task in [false, true] {
            for show in [false, true] {
                let root = temp();
                let mut w = world(&root);
                w.inventory_offers.auto_accept = true;
                w.inventory_offers.auto_accept_chat = false;
                w.inventory_offers.auto_accept_notify = false;
                w.inventory_offers.show_accepted = show;
                let folder = Uuid::from_u128(0x30);
                w.inventory.created_folders(vec![InvFolder {
                    id: folder,
                    parent: w.inventory.root,
                    type_default: 7,
                    name: "Notecards".into(),
                    ..Default::default()
                }]);
                w.on_im(offer(if task { im::TASK_INVENTORY_OFFERED } else { im::INVENTORY_OFFERED }, "Gift"));
                let commands = w.take_social_commands();
                assert!(commands.iter().any(|c| matches!(c,
                    NetCommand::SendImDialog { bucket, .. } if bucket == folder.as_bytes()
                )));
                assert!(w.take_inventory_reveal(Instant::now()).is_none());
                let items = vec![item(ITEM, folder)];
                w.reveal_added(&[], &items);
                w.inventory.add_items(items);
                assert_eq!(w.take_inventory_reveal(Instant::now()), show.then_some(ITEM));
                assert!(w.take_inventory_reveal(Instant::now()).is_none());
                clean(&w, &root);
            }
        }
    }

    #[test]
    fn an_offer_opens_the_conversation_on_a_card_and_goes_to_the_transcript() {
        let root = temp();
        let mut w = world(&root);
        w.on_im(offer(im::INVENTORY_OFFERED, "Welcome note"));
        // the entry stays (title, prefetch of the item)
        assert_eq!(w.notifications.list.len(), 1);
        let entry = &w.notifications.list[0];
        assert!(entry.kind == Kind::Inventory && matches!(entry.data, Data::Inventory { object: ITEM, .. }));
        // the conversation with Jane opens on the card, one unread line
        assert_eq!(w.social.ims.len(), 1);
        let s = &w.social.ims[0];
        assert_eq!((s.other, s.unread), (JANE, 1));
        let v = w.inventory_offers.list[0].clone();
        assert!(matches!(&s.lines.iter().map(|l| l.kind).collect::<Vec<_>>()[..], [ChatKind::InventoryOffer(o)] if *o == v.id));
        assert_eq!((v.item.as_str(), v.asset_type, v.notification), ("Welcome note", 7, entry.id));
        assert!(w.chat.is_empty());
        // the entry is known with its giver, for the toast to stay quiet
        // while the conversation is in sight
        assert_eq!(w.inventory_offers.pending_notifications().collect::<Vec<_>>(), [(entry.id, JANE)]);
        // the card is not written: the transcript has the offer once
        let log = transcript(&mut w);
        assert_eq!(log.len(), 1, "{log:?}");
        let kind = kind_label(7).to_lowercase();
        assert!(log[0].contains("Welcome note") && log[0].contains(&kind), "{log:?}");
        clean(&w, &root);
    }

    #[test]
    fn keeping_from_the_card_answers_like_the_entry_and_closes_it() {
        let root = temp();
        let mut w = world(&root);
        w.on_im(offer(im::INVENTORY_OFFERED, "Welcome note"));
        let id = w.inventory_offers.list[0].id;
        let cmds = w.answer_inventory_offer(id, true);
        assert_eq!(answers(&cmds), [im::INVENTORY_ACCEPTED]);
        assert!(w.notifications.list.is_empty());
        assert_eq!(w.inventory_offers.get(id).unwrap().answer, Answer::Kept);
        assert_eq!(w.inventory_offers.pending_notifications().count(), 0);
        // answered once only
        assert!(w.answer_inventory_offer(id, false).is_empty());
        // the history has it kept, with the way to the item
        let gone = w.notifications.take_departures();
        assert!(gone.is_empty() || gone.iter().all(|e| e.outcome == Outcome::Kept));
        clean(&w, &root);
    }

    #[test]
    fn rejecting_from_the_card_sends_the_item_to_the_trash_like_the_entry() {
        let root = temp();
        let mut w = world(&root);
        w.on_im(offer(im::INVENTORY_OFFERED, "Welcome note"));
        let id = w.inventory_offers.list[0].id;
        let cmds = w.answer_inventory_offer(id, false);
        assert_eq!(answers(&cmds), [im::INVENTORY_DECLINED]);
        assert!(w.notifications.list.is_empty());
        assert_eq!(w.inventory_offers.get(id).unwrap().answer, Answer::Rejected);
        clean(&w, &root);
    }

    #[test]
    fn answering_the_entry_marks_the_card() {
        let root = temp();
        let mut w = world(&root);
        for (r, answer) in [
            (Response::Accept, Answer::Kept),
            (Response::Decline, Answer::Rejected),
            // closing a resident's offer keeps what it gave
            (Response::Dismiss, Answer::Kept),
        ] {
            w.on_im(offer(im::INVENTORY_OFFERED, "Welcome note"));
            let v = w.inventory_offers.list.last().unwrap().clone();
            let (cmds, _) = w.respond_notification(v.notification, r.clone());
            assert_eq!(answers(&cmds).len(), 1, "{r:?}");
            assert_eq!(w.inventory_offers.get(v.id).unwrap().answer, answer, "{r:?}");
            // the card's button does nothing more
            assert!(w.answer_inventory_offer(v.id, true).is_empty());
        }
        // one conversation, three cards
        assert_eq!(w.social.ims.len(), 1);
        assert_eq!(
            w.social.ims[0]
                .lines
                .iter()
                .filter(|l| matches!(l.kind, ChatKind::InventoryOffer(_)))
                .count(),
            3
        );
        clean(&w, &root);
    }

    #[test]
    fn kept_gifts_link_the_delivered_item_or_folder_in_the_card_and_transcript() {
        for kind in [6, AT_CATEGORY] {
            let root = temp();
            let mut w = world(&root);
            let name = "Gift & café + 50%? #1\nsecondlife:///app/chat/5/hello";
            let mut gift = offer(im::INVENTORY_OFFERED, name);
            gift.binary_bucket[0] = kind as u8;
            w.on_im(gift);
            let id = w.inventory_offers.list[0].id;
            assert!(w.inventory_offers.get(id).unwrap().inventory_link(name).is_none());
            w.answer_inventory_offer(id, true);
            let v = w.inventory_offers.get(id).unwrap();
            let link = v.inventory_link(&item_label(name)).expect("kept gift link");
            assert_eq!(
                crate::slapp::parse(&link),
                Some(crate::slapp::AppLink::Inventory {
                    id: ITEM,
                    name: Some(item_label(name)),
                })
            );
            assert!(!link.contains('\n') && !link.contains(' '));
            assert!(w.social.ims[0].lines.back().unwrap().text.contains(&link));
            // Repeated answers do not duplicate the confirmation; reopening chat
            // can recover the same native selection from its stored text.
            w.answer_inventory_offer(id, true);
            let log = transcript(&mut w);
            assert_eq!(log.len(), 2, "{log:?}");
            assert!(log[1].contains(&link), "{log:?}");
            clean(&w, &root);
        }
    }

    #[test]
    fn rejected_or_unidentified_gifts_never_offer_an_inventory_link() {
        for (keep, identified) in [(false, true), (true, false)] {
            let root = temp();
            let mut w = world(&root);
            let mut gift = offer(im::INVENTORY_OFFERED, "Welcome note");
            if !identified {
                gift.binary_bucket.truncate(1);
            }
            w.on_im(gift);
            let id = w.inventory_offers.list[0].id;
            w.answer_inventory_offer(id, keep);
            assert!(w.inventory_offers.get(id).unwrap().inventory_link("Gift").is_none());
            assert!(transcript(&mut w).iter().all(|text| !text.contains("/app/inventory/")));
            clean(&w, &root);
        }
    }

    fn item(id: Uuid, parent: Uuid) -> InvItem {
        InvItem {
            id,
            parent,
            name: "Welcome note".into(),
            asset_type: 7,
            ..Default::default()
        }
    }

    #[test]
    fn a_kept_offer_shows_its_item_once_it_is_known_only_with_the_option() {
        for show in [false, true] {
            let root = temp();
            let mut w = world(&root);
            w.inventory_offers.show_accepted = show;
            w.on_im(offer(im::INVENTORY_OFFERED, "Welcome note"));
            let id = w.inventory_offers.list[0].id;
            w.answer_inventory_offer(id, true);
            // not fetched yet: nothing to select
            assert_eq!(w.take_inventory_reveal(Instant::now()), None);
            let folder = w.inventory.root;
            w.inventory.add_items(vec![item(ITEM, folder)]);
            assert_eq!(w.take_inventory_reveal(Instant::now()), show.then_some(ITEM), "{show}");
            // once
            assert_eq!(w.take_inventory_reveal(Instant::now()), None);
            clean(&w, &root);
        }
    }

    #[test]
    fn a_rejected_or_closed_offer_shows_nothing() {
        for r in [Response::Decline, Response::Dismiss] {
            let root = temp();
            let mut w = world(&root);
            w.inventory_offers.show_accepted = true;
            let folder = w.inventory.root;
            w.inventory.add_items(vec![item(ITEM, folder)]);
            w.on_im(offer(im::INVENTORY_OFFERED, "Welcome note"));
            let n = w.notifications.list[0].id;
            w.respond_notification(n, r.clone());
            assert_eq!(w.take_inventory_reveal(Instant::now()), None, "{r:?}");
            clean(&w, &root);
        }
    }

    #[test]
    fn an_object_s_kept_offer_shows_what_the_server_adds_next() {
        let root = temp();
        let mut w = world(&root);
        w.inventory_offers.show_accepted = true;
        let folder = w.inventory.root;
        let old = Uuid::from_u128(0x30);
        w.inventory.add_items(vec![item(old, folder)]);
        w.on_im(offer(im::TASK_INVENTORY_OFFERED, "Free gift"));
        // an update before the answer is not the gift
        w.apply(aurora_net::NetEvent::InventoryUpdated {
            bulk: false,
            folders: Vec::new(),
            items: vec![item(Uuid::from_u128(0x31), folder)],
        });
        let n = w.notifications.list[0].id;
        w.respond_notification(n, Response::Accept);
        assert_eq!(w.take_inventory_reveal(Instant::now()), None);
        // what the inventory already knew is not the gift
        let gift = Uuid::from_u128(0x32);
        w.apply(aurora_net::NetEvent::InventoryUpdated {
            bulk: false,
            folders: Vec::new(),
            items: vec![item(gift, folder), item(old, folder)],
        });
        assert_eq!(w.take_inventory_reveal(Instant::now()), Some(gift));
        clean(&w, &root);
    }

    #[test]
    fn an_object_s_folder_shows_the_folder_not_its_contents() {
        let root = temp();
        let mut w = world(&root);
        w.inventory_offers.show_accepted = true;
        w.on_im(offer(im::TASK_INVENTORY_OFFERED, "Free outfit"));
        let n = w.notifications.list[0].id;
        w.respond_notification(n, Response::Accept);
        let (top, inner) = (Uuid::from_u128(0x40), Uuid::from_u128(0x41));
        let folder = |id, parent| InvFolder {
            id,
            parent,
            name: "Free outfit".into(),
            type_default: -1,
            ..Default::default()
        };
        let inv_root = w.inventory.root;
        w.apply(aurora_net::NetEvent::InventoryUpdated {
            bulk: false,
            folders: vec![folder(inner, top), folder(top, inv_root)],
            items: vec![item(Uuid::from_u128(0x42), inner)],
        });
        assert_eq!(w.take_inventory_reveal(Instant::now()), Some(top));
        clean(&w, &root);
    }

    #[test]
    fn a_gift_in_a_quiet_folder_or_too_late_shows_nothing() {
        for late in [false, true] {
            let root = temp();
            let mut w = world(&root);
            w.inventory_offers.show_accepted = true;
            let trash = Uuid::from_u128(0x50);
            let inv_root = w.inventory.root;
            w.inventory.created_folders(vec![InvFolder {
                id: trash,
                parent: inv_root,
                name: "Trash".into(),
                type_default: 14,
                ..Default::default()
            }]);
            w.on_im(offer(im::INVENTORY_OFFERED, "Welcome note"));
            let id = w.inventory_offers.list[0].id;
            w.answer_inventory_offer(id, true);
            let parent = if late { inv_root } else { trash };
            w.inventory.add_items(vec![item(ITEM, parent)]);
            let now = Instant::now() + if late { REVEAL_WAIT } else { Duration::ZERO };
            assert_eq!(w.take_inventory_reveal(now), None, "{late}");
            clean(&w, &root);
        }
    }

    #[test]
    fn a_card_joins_the_open_conversation() {
        let root = temp();
        let mut w = world(&root);
        let mut hello = offer(0, "hello");
        hello.binary_bucket.clear();
        w.on_im(hello);
        w.on_im(offer(im::INVENTORY_OFFERED, "Welcome note"));
        assert_eq!(w.social.ims.len(), 1);
        let kinds: Vec<ChatKind> = w.social.ims[0].lines.iter().map(|l| l.kind).collect();
        assert!(matches!(&kinds[..], [ChatKind::Im, ChatKind::InventoryOffer(_)]), "{kinds:?}");
        clean(&w, &root);
    }

    #[test]
    fn a_blocked_giver_gets_no_conversation_nor_card() {
        let root = temp();
        let mut w = world(&root);
        w.block(JANE, "Jane Doe", super::super::mutes::MuteType::Agent).unwrap();
        w.on_im(offer(im::INVENTORY_OFFERED, "Welcome note"));
        assert!(w.notifications.list.is_empty());
        assert!(w.social.ims.is_empty());
        assert!(w.inventory_offers.list.is_empty());
        clean(&w, &root);
    }

    #[test]
    fn blocking_the_giver_afterwards_closes_the_card() {
        let root = temp();
        let mut w = world(&root);
        w.on_im(offer(im::INVENTORY_OFFERED, "Welcome note"));
        let id = w.inventory_offers.list[0].id;
        w.block(JANE, "Jane Doe", super::super::mutes::MuteType::Agent).unwrap();
        assert!(w.notifications.list.is_empty());
        assert_eq!(w.inventory_offers.get(id).unwrap().answer, Answer::Kept);
        assert!(w.answer_inventory_offer(id, false).is_empty());
        clean(&w, &root);
    }

    #[test]
    fn an_object_s_offer_stays_in_the_notification_centre() {
        let root = temp();
        let mut w = world(&root);
        w.on_im(offer(im::TASK_INVENTORY_OFFERED, "Free gift"));
        assert_eq!(w.notifications.list.len(), 1);
        assert!(w.inventory_offers.list.is_empty());
        assert!(w.social.ims.is_empty());
        assert!(transcript(&mut w).is_empty());
        clean(&w, &root);
    }

    #[test]
    fn object_offer_resolves_owner_without_caching_the_object_as_a_resident() {
        let root = temp();
        let mut w = world(&root);
        w.social.names.remove(&JANE);
        let mut gift = offer(
            im::TASK_INVENTORY_OFFERED,
            "Free gift ( https://maps.secondlife.com/secondlife/Demo/1/2/3 )",
        );
        gift.from_name = "Gift dispenser".into();
        w.on_im(gift);
        assert!(!w.social.names.contains_key(&JANE));
        assert!(w.notifications.list[0].body.contains(&JANE.to_string()));
        w.apply(aurora_net::NetEvent::Names(vec![(JANE, "Jane".into(), "Doe".into())]));
        let n = &w.notifications.list[0];
        assert_eq!(n.title, "Gift dispenser");
        assert!(n.body.contains("Gift dispenser") && n.body.contains("Jane Doe") && n.body.contains("Free gift"));
        assert!(!n.body.contains("https://") && !n.body.contains(&JANE.to_string()));
        assert_eq!(
            n.name_links,
            [NameLink {
                name: "Jane Doe".into(),
                target: NameTarget::Avatar(JANE)
            }]
        );
        clean(&w, &root);
    }

    #[test]
    fn object_offer_accept_discard_and_mute_reply_to_the_owner_with_the_transaction() {
        for response in [Response::Accept, Response::Decline, Response::Block] {
            for group in [false, true] {
                let root = temp();
                let mut w = world(&root);
                let mut gift = offer(im::TASK_INVENTORY_OFFERED, "Welcome note");
                gift.from_name = "Gift dispenser".into();
                gift.from_group = group;
                w.on_im(gift.clone());
                if group {
                    assert!(
                        w.search
                            .take_commands()
                            .iter()
                            .any(|c| matches!(c, NetCommand::GroupProfileRequest(JANE)))
                    );
                    w.apply(aurora_net::NetEvent::GroupProfile(Box::new(aurora_net::search::GroupProfile {
                        id: JANE,
                        name: "Demo group".into(),
                        ..Default::default()
                    })));
                    assert!(w.notifications.list[0].body.contains("Demo group"));
                    assert_eq!(w.notifications.list[0].name_links[0].target, NameTarget::Group(JANE));
                }
                let id = w.notifications.list[0].id;
                if matches!(response, Response::Block) {
                    gift.session_id = Uuid::from_u128(0x11);
                    w.on_im(gift.clone());
                }
                let (cmds, _) = w.respond_notification(id, response.clone());
                let keep = matches!(response, Response::Accept);
                let expected_dialog = if keep {
                    im::TASK_INVENTORY_ACCEPTED
                } else {
                    im::TASK_INVENTORY_DECLINED
                };
                assert!(
                    matches!(&cmds[..], [NetCommand::SendImDialog { to: JANE, id: TX, dialog, bucket, .. }]
                    if *dialog == expected_dialog
                    && (if keep { bucket.len() == 16 } else { bucket.is_empty() }))
                );
                assert!(w.notifications.list.is_empty());
                assert!(w.take_inventory_edits().is_empty());
                assert!(w.social.ims.is_empty());
                if matches!(response, Response::Block) {
                    let mute = w.mutes.get(&JANE).expect("owner blocked");
                    assert_eq!(
                        mute.kind,
                        if group {
                            super::super::mutes::MuteType::Group
                        } else {
                            super::super::mutes::MuteType::Agent
                        }
                    );
                    assert_eq!(mute.name, if group { "Demo group" } else { "Jane Doe" });
                    assert!(w.take_social_commands().iter().any(|c| matches!(c, NetCommand::UpdateMute { .. })));
                    w.on_im(gift);
                    assert!(w.notifications.list.is_empty(), "future gifts from that owner are filtered");
                }
                clean(&w, &root);
            }
        }
    }

    #[test]
    fn our_object_offer_cannot_mute_ourselves() {
        let root = temp();
        let mut w = world(&root);
        let mut gift = offer(im::TASK_INVENTORY_OFFERED, "Welcome note");
        gift.from_agent_id = ME;
        gift.from_name = "My dispenser".into();
        w.on_im(gift);
        let n = &w.notifications.list[0];
        assert!(n.body.contains("My dispenser") && n.body.contains("Welcome note"));
        assert!(matches!(
            &n.data,
            Data::Inventory {
                task_offer: Some(TaskOffer { own_object: true, .. }),
                ..
            }
        ));
        let (cmds, _) = w.respond_notification(n.id, Response::Block);
        assert!(cmds.is_empty() && !w.mutes.is_muted_id(&ME));
        assert_eq!(w.notifications.list.len(), 1);
        clean(&w, &root);
    }

    #[test]
    fn our_offer_is_noted_in_the_open_conversation() {
        let root = temp();
        let mut w = world(&root);
        let mut hello = offer(0, "hello");
        hello.binary_bucket.clear();
        w.on_im(hello);
        w.social.ims[0].unread = 0;
        w.inventory_offer_sent(JANE, "Welcome note", false, false);
        w.inventory_offer_sent(JANE, "Outfits", true, false);
        let s = &w.social.ims[0];
        assert_eq!(s.unread, 0);
        let notes: Vec<_> = s
            .lines
            .iter()
            .filter(|l| l.kind == ChatKind::System)
            .map(|l| l.text.clone())
            .collect();
        assert_eq!(
            notes,
            [
                aurora_i18n::tr!("world-inventory-offers-item-sent", item = item_label("Welcome note")),
                aurora_i18n::tr!("world-inventory-offers-folder-sent", item = item_label("Outfits")),
            ]
        );
        assert!(w.chat.is_empty());
        clean(&w, &root);
    }

    /// LLGiveInventory::logInventoryOffer under `@shownames`: what goes to a
    /// hidden resident around us is told in the nearby chat alone, by their
    /// stand-in, unless it was dropped on their conversation or their
    /// profile is shown.
    #[test]
    fn shownames_tells_a_gift_to_a_hidden_resident_around_us_in_the_nearby_chat_alone() {
        let root = temp();
        let mut w = hiding_names(&root);
        around(&mut w);
        w.social.session_mut(JANE);
        let stand_in = w.rlva_anonymous(JANE).display_name.clone();
        w.inventory_offer_sent(JANE, "Welcome note", false, false);
        let said: Vec<String> = w.chat.iter().map(|l| l.text.clone()).collect();
        assert_eq!(said, [aurora_i18n::tr!("world-inventory-offers-sent-anonymous", name = stand_in)]);
        assert!(!said[0].contains("Welcome note"), "the item is not told either");
        assert!(w.social.ims[0].lines.is_empty(), "nothing in their conversation");
        // dropped on their conversation, then with their profile shown: as
        // without the restriction
        w.inventory_offer_sent(JANE, "Welcome note", false, true);
        assert_eq!(w.social.ims[0].lines.len(), 1);
        w.rlva.open_profiles.insert(JANE);
        w.inventory_offer_sent(JANE, "Outfits", true, false);
        assert_eq!(w.social.ims[0].lines.len(), 2);
        assert_eq!(w.chat.len(), 1);
        clean(&w, &root);
        // someone who is not around: their conversation, as ever
        let root = temp();
        let mut w = hiding_names(&root);
        w.social.session_mut(JANE);
        w.inventory_offer_sent(JANE, "Welcome note", false, false);
        assert!(w.chat.is_empty());
        assert_eq!(w.social.ims[0].lines.len(), 1);
        clean(&w, &root);
    }

    #[test]
    fn our_offer_without_a_conversation_goes_to_the_transcript_only() {
        let root = temp();
        let mut w = world(&root);
        w.inventory_offer_sent(JANE, "Welcome note", false, false);
        assert!(w.social.ims.is_empty());
        assert!(w.chat.is_empty());
        let log = transcript(&mut w);
        assert_eq!(log.len(), 1, "{log:?}");
        assert!(log[0].contains("Welcome note"), "{log:?}");
        clean(&w, &root);
    }

    #[test]
    fn the_kind_comes_from_the_asset_type() {
        assert_eq!(kind_label(7), aurora_i18n::tr!("inventory-kind-notecard"));
        assert_eq!(kind_label(AT_CATEGORY), aurora_i18n::tr!("inventory-kind-folder"));
        assert_eq!(kind_label(-1), aurora_i18n::tr!("inventory-kind-other"));
        assert_eq!(item_label(""), aurora_i18n::tr!("world-mod-an-object"));
    }
}
