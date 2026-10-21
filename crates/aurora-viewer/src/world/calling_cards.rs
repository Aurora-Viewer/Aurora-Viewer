//! Calling cards between residents: "Give my calling card" of an avatar's
//! menu, the offers we receive and the answers to ours. Port of
//! LLAvatarGiveCard (indra/newview/llviewermenu.cpp) and of
//! process_offer_callingcard, callingcard_offer_callback,
//! process_accept_callingcard and process_decline_callingcard
//! (indra/newview/llviewermessage.cpp), originally LGPL 2.1. The card itself
//! is created by the server in the Calling Cards folder the acceptance names,
//! and reaches the inventory as any other new item.

use super::World;
use super::notifications::{Data, Kind};
use aurora_net::NetCommand;
use uuid::Uuid;

impl World {
    /// LLAvatarGiveCard: an avatar in sight, whose name and region we know,
    /// gets OfferCallingCard through the simulator of its region
    /// ("OfferedCard"); otherwise "CantOfferCallingCard".
    pub fn offer_calling_card(&mut self, dest: Uuid) {
        let target = self
            .objects
            .index_of_uuid(&dest)
            .and_then(|i| self.objects.get(i))
            .filter(|o| o.is_avatar() && o.full_id != self.agent_id)
            .and_then(|o| Some((o.display_name()?, o.key.region)))
            .filter(|(_, region)| self.regions.contains_key(region));
        let title = aurora_i18n::tr!("world-calling-card-title");
        match target {
            Some((name, handle)) => {
                self.groups.out.push(NetCommand::OfferCallingCard {
                    handle,
                    dest,
                    transaction: Uuid::new_v4(),
                });
                let text = aurora_i18n::tr!("world-calling-card-offered", name = name.clone());
                let id = self.notifications.push(Kind::Info, title, text, Data::None);
                self.notifications
                    .link_name(id, name, super::notifications::NameTarget::Avatar(dest));
            }
            None => {
                let text = aurora_i18n::tr!("world-calling-card-cannot-offer");
                self.notifications.push(Kind::Alert, title, text, Data::None);
            }
        }
    }

    /// process_offer_callingcard: from an avatar in sight only (its name
    /// comes from the object); declined at once in do not disturb or when
    /// their text is blocked, else asked.
    pub(super) fn on_calling_card_offer(&mut self, from: Uuid, tx: Uuid, region: aurora_net::RegionHandle) {
        let Some(name) = self
            .objects
            .index_of_uuid(&from)
            .and_then(|i| self.objects.get(i))
            .filter(|o| o.is_avatar())
            .and_then(|o| o.display_name())
        else {
            log::warn!("calling card offer from an unknown source {from}");
            return;
        };
        if self.status.dnd || self.mutes.text_muted(&from, &name) {
            self.groups.out.push(NetCommand::AnswerCallingCard {
                handle: region,
                transaction: tx,
                folder: None,
            });
            self.calling_card_declined(from);
            return;
        }
        let id = self.notifications.push(
            Kind::Friendship,
            aurora_i18n::tr!("world-calling-card-offer", name = name.clone()),
            aurora_i18n::tr!("world-calling-card-offer-body"),
            Data::CallingCard { from, tx, region },
        );
        self.notifications
            .link_name(id, name, super::notifications::NameTarget::Avatar(from));
    }

    /// send_do_not_disturb_message after a declined offer: our do not
    /// disturb response, when we are in do not disturb.
    pub(super) fn calling_card_declined(&mut self, from: Uuid) {
        if self.status.dnd {
            let text = self.status.cfg.dnd_text.clone();
            self.auto_im(from, Uuid::nil(), &text);
        }
    }

    /// CallingCardAccepted / CallingCardDeclined.
    pub(super) fn on_calling_card_answer(&mut self, accepted: bool) {
        let text = if accepted {
            aurora_i18n::tr!("world-calling-card-accepted")
        } else {
            aurora_i18n::tr!("world-calling-card-declined")
        };
        let id = self
            .notifications
            .push(Kind::Info, aurora_i18n::tr!("world-calling-card-title"), text, Data::None);
        let notice = self.notice_of(id, super::attention::Open::Notifications);
        self.ask_attention(super::attention::Cause::Notice, notice);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::avatar::AvatarLibrary;
    use crate::world::notifications::Response;
    use std::sync::Arc;

    fn world() -> World {
        let mut w = World::new(Arc::new(AvatarLibrary::load()));
        for e in crate::demo::events() {
            w.apply(e);
        }
        w.take_social_commands();
        w
    }

    #[test]
    fn our_card_goes_to_an_avatar_in_sight_and_not_to_an_unknown_one() {
        let mut w = world();
        let wolf = crate::demo::DEMO_LOUP;
        w.offer_calling_card(wolf);
        let sent = w.take_social_commands();
        assert!(
            sent.iter()
                .any(|c| matches!(c, NetCommand::OfferCallingCard { dest, transaction, .. } if *dest == wolf && !transaction.is_nil())),
            "{sent:?}"
        );
        assert_eq!(w.notifications.list.last().map(|n| n.kind), Some(Kind::Info));
        // nobody in sight with that id: "CantOfferCallingCard", nothing sent
        w.offer_calling_card(Uuid::from_u128(0xdead));
        assert!(w.take_social_commands().is_empty());
        assert_eq!(w.notifications.list.last().map(|n| n.kind), Some(Kind::Alert));
    }

    #[test]
    fn an_offer_is_asked_then_accepted_into_the_calling_cards_folder() {
        let mut w = world();
        let wolf = crate::demo::DEMO_LOUP;
        // the demo blocks the wolf: his offers are declined without asking
        let tx = Uuid::from_u128(5);
        w.on_calling_card_offer(wolf, tx, 7);
        if w.mutes.text_muted(&wolf, "") {
            let sent = w.take_social_commands();
            assert!(matches!(
                sent.as_slice(),
                [NetCommand::AnswerCallingCard { handle: 7, transaction, folder: None }] if *transaction == tx
            ));
            w.unblock(wolf, "", 0);
            w.take_social_commands();
            w.on_calling_card_offer(wolf, tx, 7);
        }
        let id = w
            .notifications
            .list
            .iter()
            .find(|n| matches!(n.data, Data::CallingCard { from, .. } if from == wolf))
            .map(|n| n.id)
            .expect("offer asked");
        let (out, _) = w.respond_notification(id, Response::Accept);
        let folder = w.calling_card_folder();
        assert!(matches!(
            out.as_slice(),
            [NetCommand::AnswerCallingCard { handle: 7, transaction, folder: Some(f) }] if *transaction == tx && *f == folder
        ));
        // an unknown source is ignored (no avatar in sight)
        w.on_calling_card_offer(Uuid::from_u128(0xdead), tx, 7);
        assert!(!w.notifications.list.iter().any(|n| matches!(n.data, Data::CallingCard { .. })));
        // in do not disturb: declined at once, with our response
        w.status.dnd = true;
        w.on_calling_card_offer(wolf, tx, 7);
        let sent = w.take_social_commands();
        assert!(sent.iter().any(|c| matches!(c, NetCommand::AnswerCallingCard { folder: None, .. })));
        assert!(
            sent.iter()
                .any(|c| matches!(c, NetCommand::SendImDialog { to, dialog: 20, .. } if *to == wolf))
        );
    }
}
