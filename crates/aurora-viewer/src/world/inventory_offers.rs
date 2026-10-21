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

use super::notification_history::Outcome;
use super::notifications::{AT_CATEGORY, Response};
use super::{ChatKind, World};
use aurora_net::{InstantMessage, NetCommand};
use uuid::Uuid;

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
    /// LLAssetType of what is given (AT_CATEGORY for a folder).
    pub asset_type: i8,
    pub answer: Answer,
    /// Its entry in the notification centre.
    pub notification: u64,
}

/// Offers kept once answered, for the conversation cards.
const MAX_OFFERS: usize = 50;

#[derive(Default)]
pub struct InventoryOffers {
    pub list: Vec<Offer>,
    next: u64,
}

impl InventoryOffers {
    fn push(&mut self, from: Uuid, item: String, asset_type: i8, notification: u64) -> u64 {
        self.next += 1;
        self.list.push(Offer {
            id: self.next,
            from,
            item,
            asset_type,
            answer: Answer::Pending,
            notification,
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
        format!("«\u{a0}{item}\u{a0}»")
    }
}

impl World {
    /// A resident's offer whose entry `notification` was just listed: its
    /// card in the conversation with the giver, opened if needed and
    /// counted as unread, and its text in the giver's transcript. Blocked
    /// givers were answered before (`im_blocked`): no conversation for them.
    pub(super) fn on_inventory_offer_card(&mut self, im: &InstantMessage, from: &str, asset_type: i8, notification: u64) {
        let offer = self
            .inventory_offers
            .push(im.from_agent_id, im.message.clone(), asset_type, notification);
        let title = aurora_i18n::tr!("world-mod-gives-you", from = from.to_string(), what = item_label(&im.message));
        self.add_card(im, ChatKind::InventoryOffer(offer), title);
        let line = self.notice_line(aurora_i18n::tr!(
            "world-inventory-offers-received",
            name = super::teleport_invites::agent_link(im.from_agent_id),
            kind = kind_label(asset_type).to_lowercase(),
            item = im.message.clone()
        ));
        self.transcript_only(im.from_agent_id, &line);
    }

    /// Our offer of an item or a folder went out
    /// (LLGiveInventory::logInventoryOffer, indra/newview/llgiveinventory.cpp):
    /// "Inventory item 'X' offered" in the conversation with the resident
    /// without counting as unread, or only in their transcript when none is
    /// open.
    pub fn inventory_offer_sent(&mut self, to: Uuid, name: &str, folder: bool) {
        let item = item_label(name);
        let text = if folder {
            aurora_i18n::tr!("world-inventory-offers-folder-sent", item = item)
        } else {
            aurora_i18n::tr!("world-inventory-offers-item-sent", item = item)
        };
        self.note_with(to, text);
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
        assert_eq!(w.social.ims[0].lines.len(), 3);
        clean(&w, &root);
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
    fn our_offer_is_noted_in_the_open_conversation() {
        let root = temp();
        let mut w = world(&root);
        let mut hello = offer(0, "hello");
        hello.binary_bucket.clear();
        w.on_im(hello);
        w.social.ims[0].unread = 0;
        w.inventory_offer_sent(JANE, "Welcome note", false);
        w.inventory_offer_sent(JANE, "Outfits", true);
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

    #[test]
    fn our_offer_without_a_conversation_goes_to_the_transcript_only() {
        let root = temp();
        let mut w = world(&root);
        w.inventory_offer_sent(JANE, "Welcome note", false);
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
