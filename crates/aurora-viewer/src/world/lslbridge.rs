//! Firestorm's LSL bridge talks to the viewer through llOwnerSay; Firestorm
//! swallows those messages before they reach nearby chat, after
//! FSLSLBridge::lslToViewer (indra/newview/fslslbridge.cpp, originally LGPL
//! 2.1), called from process_chat_from_simulator for CHAT_TYPE_OWNER.
//!
//! Aurora has no bridge, which is Firestorm with UseLSLBridge off: the
//! handshake (`<bridgeURL>…</bridgeURL><bridgeAuth>…</bridgeAuth>…`, which
//! carries the bridge's capability URL and its key) is hidden and never
//! answered. Firestorm hides the bridge's other replies only once it knows
//! the bridge object (mBridgeUUID, set after the handshake is authorised);
//! we cannot authorise it, so the object that sent the handshake is taken as
//! the bridge, and its later replies are hidden as Firestorm does with the
//! bridge on.

use aurora_net::{ChatMessage, ChatSourceType, ChatType};
use uuid::Uuid;

/// Tags answered by FSLSLBridge::lslToViewer whatever the sender.
const HANDSHAKE_TAGS: [&str; 2] = ["<bridgeURL>", "<bridgeRequestError/>"];
/// Tags answered only when they come from our bridge (FIRE-962 and later).
const BRIDGE_TAGS: [&str; 4] = ["<clientAO ", "<bridgeGetScriptInfo>", "<bridgeMovelock ", "<bridgeError "];

#[derive(Debug, Default)]
pub struct LslBridge {
    /// The object that sent the last handshake (mBridgeUUID).
    bridge: Option<Uuid>,
}

impl LslBridge {
    /// True when `c` is a bridge message that Firestorm keeps out of chat.
    pub fn hides(&mut self, c: &ChatMessage) -> bool {
        if !matches!(c.source_type, ChatSourceType::Object) || c.chat_type != ChatType::OwnerSay {
            return false;
        }
        let Some(tag) = tag_of(&c.message) else {
            return false;
        };
        if HANDSHAKE_TAGS.contains(&tag) {
            if self.bridge != Some(c.source_id) {
                // never the message itself: it holds the capability URL and key
                log::info!("LSL bridge: handshake from object {} hidden (no bridge in Aurora)", c.source_id);
                self.bridge = Some(c.source_id);
            }
            return true;
        }
        self.bridge == Some(c.source_id) && BRIDGE_TAGS.contains(&tag)
    }
}

/// The leading tag of a bridge message: from the opening `<` to the first
/// `>` or space, included (FSLSLBridge::lslToViewer's tagend).
fn tag_of(message: &str) -> Option<&str> {
    if !message.starts_with('<') {
        return None;
    }
    let end = message.find(['>', ' '])?;
    Some(&message[..=end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    const BRIDGE: Uuid = Uuid::from_u128(1);
    const OTHER: Uuid = Uuid::from_u128(2);

    fn owner_say(from: Uuid, text: &str) -> ChatMessage {
        ChatMessage {
            from_name: "#Firestorm LSL Bridge v2.32".into(),
            source_id: from,
            owner_id: Uuid::from_u128(3),
            source_type: ChatSourceType::Object,
            chat_type: ChatType::OwnerSay,
            position: Vec3::ZERO,
            message: text.into(),
        }
    }

    const HANDSHAKE: &str =
        "<bridgeURL>https://simhost.example:12043/cap/x</bridgeURL><bridgeAuth>k</bridgeAuth><bridgeVer>2.32</bridgeVer>";

    #[test]
    fn tags() {
        assert_eq!(tag_of(HANDSHAKE), Some("<bridgeURL>"));
        assert_eq!(tag_of("<clientAO state=on>"), Some("<clientAO "));
        assert_eq!(tag_of("<bridgeRequestError/>"), Some("<bridgeRequestError/>"));
        assert_eq!(tag_of("<nothing"), None);
        assert_eq!(tag_of("hello <bridgeURL>"), None);
        assert_eq!(tag_of(""), None);
    }

    #[test]
    fn handshake_hidden_from_anyone() {
        let mut b = LslBridge::default();
        assert!(b.hides(&owner_say(OTHER, HANDSHAKE)));
        assert!(b.hides(&owner_say(BRIDGE, "<bridgeRequestError/>")));
    }

    #[test]
    fn replies_hidden_only_from_the_bridge() {
        let mut b = LslBridge::default();
        // before the handshake the bridge is unknown, as in Firestorm
        assert!(!b.hides(&owner_say(BRIDGE, "<bridgeError error=wrongvm>")));
        assert!(b.hides(&owner_say(BRIDGE, HANDSHAKE)));
        assert!(b.hides(&owner_say(BRIDGE, "<bridgeError error=wrongvm>")));
        assert!(b.hides(&owner_say(BRIDGE, "<clientAO state=off>")));
        assert!(b.hides(&owner_say(BRIDGE, "<bridgeMovelock state=1>")));
        assert!(!b.hides(&owner_say(BRIDGE, "<unknown tag>")));
        assert!(!b.hides(&owner_say(BRIDGE, "Bridge ready")));
        assert!(!b.hides(&owner_say(OTHER, "<clientAO state=on>")));
    }

    #[test]
    fn only_owner_say_from_objects() {
        let mut b = LslBridge::default();
        let mut said = owner_say(BRIDGE, HANDSHAKE);
        said.chat_type = ChatType::Normal;
        assert!(!b.hides(&said));
        let mut agent = owner_say(BRIDGE, HANDSHAKE);
        agent.source_type = ChatSourceType::Agent;
        assert!(!b.hides(&agent));
    }
}
