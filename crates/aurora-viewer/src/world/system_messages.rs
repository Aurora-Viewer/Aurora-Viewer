//! A message of the grid itself (#1604): an IM with no sender, or from
//! "Second Life" (SYSTEM_FROM), such as a Marketplace purchase delivered.
//! The Legacy Viewer takes it as the system's, not a resident's
//! (CHAT_SOURCE_SYSTEM for `from_id.isNull() || name == SYSTEM_FROM`,
//! LLIMProcessing::processNewMessage, indra/newview/llimprocessing.cpp,
//! originally LGPL 2.1). Aurora drew it as the line of a sender named
//! "System", with its letter: it is now a notice of the nearby chat, like a
//! teleport's.
//!
//! A delivered purchase is about an item: its row shows the box, and
//! "Received Items" in its text opens that folder (the inventory on its
//! Received tab).

use super::chat_log::format::SYSTEM_FROM;
use super::notice_topics::Topic;
use super::{ChatKind, World, attention};
use aurora_net::InstantMessage;
use uuid::Uuid;

/// IM_FROM_TASK: an object's message stays one, even from an object named
/// "Second Life" (the Legacy Viewer also asks it a null region and
/// position, which only the grid's own have).
const FROM_TASK: u8 = 19;

/// FT_INBOX: the Received Items folder.
const FT_INBOX: i32 = 50;

/// The grid's words for a delivered Marketplace purchase, around the item's
/// name (only its English text is known).
const PURCHASE: (&str, &str) = ("Your purchase of ", " has been delivered to your Received Items folder.");

/// The folder's name in `PURCHASE.1`, the words that become its link.
const FOLDER: &str = "Received Items";

/// The message is the grid's: no sender, or "Second Life".
pub fn is_system_im(im: &InstantMessage) -> bool {
    im.dialog != FROM_TASK && (im.from_agent_id.is_nil() || im.from_name.trim().is_empty() || im.from_name == SYSTEM_FROM)
}

/// The grid's text for `item` delivered (the demo's).
pub fn purchase_message(item: &str) -> String {
    format!("{}{item}{}", PURCHASE.0, PURCHASE.1)
}

/// A delivered purchase's text, with "Received Items" as a link to
/// `inbox` when it is known; `None` for any other message.
pub fn purchase_text(text: &str, inbox: Option<Uuid>) -> Option<String> {
    let item = text.strip_prefix(PURCHASE.0)?.strip_suffix(PURCHASE.1)?;
    let Some(inbox) = inbox else {
        return Some(text.to_owned());
    };
    let link = format!("secondlife:///app/inventory/{inbox}/select?name=Received%20Items");
    Some(format!("{}{item}{}", PURCHASE.0, PURCHASE.1.replacen(FOLDER, &link, 1)))
}

impl World {
    /// Its notice in the nearby chat; it still asks for attention as the
    /// "System" line did (a SystemMessage).
    pub(super) fn on_system_im(&mut self, message: &str) {
        let text = message.trim();
        if text.is_empty() {
            return;
        }
        let inbox = super::inventory::actions::system(&self.inventory, FT_INBOX);
        let (line, topic) = match purchase_text(text, inbox) {
            Some(line) => (line, Topic::Items),
            None => (text.to_owned(), Topic::Info),
        };
        if self.alerts(ChatKind::Im) {
            let notice = attention::Notice {
                title: aurora_i18n::tr!("common-system").to_owned(),
                body: text.to_owned(),
                open: attention::Open::NearbyChat,
            };
            self.ask_attention(attention::Cause::Message, Some(notice));
        }
        self.system_notice(line, topic);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::avatar::AvatarLibrary;
    use aurora_net::NetEvent;
    use aurora_net::inventory::InvFolder;
    use std::sync::Arc;

    const ME: Uuid = Uuid::from_u128(1);
    const INBOX: Uuid = Uuid::from_u128(0x1b0c);
    const PURCHASE_TEXT: &str = "Your purchase of Underwater Lighting - Buttonjar has been delivered to your Received Items folder.";

    fn world(inbox: bool) -> World {
        let mut world = World::new(Arc::new(AvatarLibrary::load()));
        world.agent_id = ME;
        world.inventory.root = Uuid::from_u128(90_000);
        let mut folders = vec![InvFolder {
            id: world.inventory.root,
            name: "Inventory".into(),
            type_default: 8,
            ..Default::default()
        }];
        if inbox {
            folders.push(InvFolder {
                id: INBOX,
                parent: world.inventory.root,
                name: "Received Items".into(),
                type_default: FT_INBOX,
                ..Default::default()
            });
        }
        world.inventory.created_folders(folders);
        world
    }

    fn im(from: Uuid, name: &str, dialog: u8, text: &str) -> InstantMessage {
        InstantMessage {
            from_agent_id: from,
            from_name: name.into(),
            to_agent_id: ME,
            dialog,
            session_id: Uuid::from_u128(3),
            message: text.into(),
            offline: false,
            from_group: false,
            binary_bucket: Vec::new(),
            timestamp: 0,
        }
    }

    #[test]
    fn the_grid_s_messages_are_its_own_and_an_object_s_stay_an_object_s() {
        assert!(is_system_im(&im(Uuid::nil(), "", 0, "x")));
        assert!(is_system_im(&im(Uuid::nil(), SYSTEM_FROM, 0, "x")));
        assert!(is_system_im(&im(Uuid::from_u128(7), "", 2, "x")));
        assert!(!is_system_im(&im(Uuid::from_u128(7), "Jane Doe", 2, "x")));
        // an object called "Second Life"
        assert!(!is_system_im(&im(Uuid::from_u128(7), SYSTEM_FROM, FROM_TASK, "x")));
    }

    #[test]
    fn a_purchase_links_its_folder_and_nothing_else_is_a_purchase() {
        let text = purchase_text(PURCHASE_TEXT, Some(INBOX)).expect("a purchase");
        assert_eq!(
            text,
            format!(
                "Your purchase of Underwater Lighting - Buttonjar has been delivered to your \
                 secondlife:///app/inventory/{INBOX}/select?name=Received%20Items folder."
            )
        );
        // the link is the inventory's, named after the folder
        let url = text.split_whitespace().find(|w| w.starts_with("secondlife:")).expect("a link");
        assert_eq!(
            crate::slapp::parse(url),
            Some(crate::slapp::AppLink::Inventory {
                id: INBOX,
                name: Some("Received Items".into()),
            })
        );
        // no folder known yet: the words as they are
        assert_eq!(purchase_text(PURCHASE_TEXT, None).as_deref(), Some(PURCHASE_TEXT));
        assert_eq!(purchase_text("The region will restart.", Some(INBOX)), None);
    }

    /// The purchase is a notice about an item, not the line of a "System"
    /// sender, and opens no conversation.
    #[test]
    fn a_delivered_purchase_is_a_notice_of_the_nearby_chat() {
        let mut w = world(true);
        w.apply(NetEvent::InstantMessage(im(Uuid::nil(), "", 0, PURCHASE_TEXT)));
        let line = w.chat.back().expect("a nearby line").clone();
        assert_eq!(line.kind, ChatKind::System);
        assert!(line.from.is_empty());
        assert!(line.text.contains(&format!("secondlife:///app/inventory/{INBOX}/select")));
        assert_eq!(w.notice_topics.of(&line), Topic::Items);
        assert!(!w.social.has_session(&Uuid::nil()));

        // another message of the grid: a plain notice
        w.apply(NetEvent::InstantMessage(im(Uuid::nil(), SYSTEM_FROM, 0, "Maintenance tonight. ")));
        let line = w.chat.back().expect("a nearby line").clone();
        assert_eq!(line.kind, ChatKind::System);
        assert_eq!(line.text, "Maintenance tonight.");
        assert_eq!(w.notice_topics.of(&line), Topic::Info);
    }

    #[test]
    fn an_object_named_second_life_still_speaks_as_an_object() {
        let mut w = world(false);
        w.apply(NetEvent::InstantMessage(im(Uuid::from_u128(7), SYSTEM_FROM, FROM_TASK, "hello")));
        let line = w.chat.back().expect("a nearby line");
        assert_eq!(line.kind, ChatKind::ObjectIm);
        assert_eq!(line.from, SYSTEM_FROM);
    }
}
