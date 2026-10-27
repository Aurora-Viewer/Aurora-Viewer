//! Provisioning and authenticated owner-chat ingress for the separate IK relay.
use super::*;

pub(super) const NAME: &str = "#Aurora Bridge IK";
pub(super) const SOURCE: &str = include_str!("ik.lsl");
pub(crate) const CONTROL: i32 = -190018106;

#[derive(Default)]
pub(super) struct Service {
    pub script: Uuid,
    pub upload: Option<Uuid>,
    pub restarted: bool,
    candidate: Option<ChatMessage>,
}

impl Service {
    pub(super) fn clear_candidate(&mut self) {
        self.candidate = None;
    }
}

impl Bridge {
    pub(super) fn ik_event(&mut self, world: &mut World, event: &NetEvent, now: Instant) -> bool {
        let NetEvent::Chat(chat) = event else { return false };
        if chat.source_type != ChatSourceType::Object
            || chat.chat_type != ChatType::OwnerSay
            || !(chat.message.starts_with("<auroraBridgeIK>") || chat.message.starts_with("<auroraIKRelay>"))
        {
            return false;
        }
        // Consume even invalid protocol traffic; it must never reach chat/logs.
        if chat.owner_id != world.agent_id || chat.message.len() > 1024 {
            return true;
        }
        if chat.message.starts_with("<auroraBridgeIK>") {
            self.ik.candidate = Some(chat.clone());
            self.accept_ik(world);
        } else if world.hand_contacts.relay.as_ref().is_some_and(|relay| {
            relay.root == chat.source_id
                && Some(relay.region) == world.main_region
                && world
                    .worn_attachment(&self.item)
                    .and_then(|i| world.objects.get(i))
                    .is_some_and(|o| o.full_id == relay.root)
        }) {
            world.receive_ik_relay(chat, now);
        }
        true
    }

    pub(super) fn ik_candidate_relay(&self, world: &World) -> Option<super::super::hand_ik::relay::Relay> {
        let chat = self.ik.candidate.as_ref()?;
        let root = world.worn_attachment(&self.item).and_then(|i| world.objects.get(i))?;
        if root.full_id != chat.source_id || Some(root.key.region) != world.main_region || self.folder.is_nil() {
            return None;
        }
        let body = chat.message.strip_prefix("<auroraBridgeIK>")?.strip_suffix("</auroraBridgeIK>")?;
        let fields: Vec<_> = body.split('|').collect();
        if fields.len() != 3 || fields[0] != VERSION || fields[1].parse::<Uuid>().ok()? != self.folder {
            return None;
        }
        let session = fields[2].parse::<Uuid>().ok().filter(|id| !id.is_nil())?;
        Some(super::super::hand_ik::relay::Relay {
            root: root.full_id,
            region: root.key.region,
            session,
        })
    }

    pub(super) fn accept_ik(&mut self, world: &mut World) {
        if self.phase != Phase::Ready {
            return;
        }
        if let Some(relay) = self.ik_candidate_relay(world)
            && world.hand_contacts.relay.as_ref() != Some(&relay)
        {
            self.commands.push(relay.command("discover"));
            world.hand_contacts.set_relay(relay);
        }
        self.ik.candidate = None;
    }
}
