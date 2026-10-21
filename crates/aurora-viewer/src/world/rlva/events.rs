//! Script permissions, teleport consent and restriction notifications.
//! See RlvUtil::filterScriptQuestions, RlvActions::canAcceptTpOffer and
//! RlvBehaviourNotifyHandler in the Legacy Viewer.
use super::{super::World, Rlva};
use uuid::Uuid;

pub(super) fn notification_option(option: &str) -> Option<(i32, &str)> {
    let (channel, filter) = option.split_once(';').unwrap_or((option, ""));
    let channel = channel.parse::<i32>().ok()?;
    (channel > 0 && channel != i32::MAX).then_some((channel, filter))
}

impl Rlva {
    /// The reference also notifies about rejected and duplicate commands.
    /// Only the behaviour/option part is tested against the subscription.
    pub fn notify(&mut self, text: &str) {
        let subject = text.split_once('=').map_or(text, |(name, _)| name).to_ascii_lowercase();
        let channels: Vec<_> = self
            .rules_for("notify", Uuid::nil())
            .filter_map(|(_, option)| notification_option(option))
            .filter(|(_, filter)| subject.contains(&filter.to_ascii_lowercase()))
            .map(|(channel, _)| channel)
            .collect();
        for channel in channels {
            self.reply(channel, format!("/{text}"));
        }
    }

    pub fn auto_accept(&self, behaviour: &str, sender: Uuid) -> bool {
        self.has(behaviour) || (!sender.is_nil() && self.targets(behaviour, &sender.to_string()))
    }
}

impl World {
    /// The return is delayed until the seat parent has gone, as onSitOrStand
    /// does in the reference. A failed sit must never trigger a teleport.
    pub fn rlva_update_stand(&mut self) {
        if !self.rlva.has("standtp") {
            self.rlva.stand_origin = None;
            self.rlva.stand_return = None;
            self.rlva.stand_seated = false;
        } else if self.rlva.stand_origin.is_some() {
            if self.agent.is_sitting() {
                self.rlva.stand_seated = true;
            } else if self.rlva.stand_seated {
                self.rlva.stand_return = self.rlva.stand_origin.take();
                self.rlva.stand_seated = false;
            }
        }
    }

    pub fn rlva_can_accept_teleport(&self, sender: Uuid, request: bool) -> bool {
        self.rlva.allows(if request { "tprequest" } else { "tplure" }, &sender.to_string())
            && (request || !self.agent.is_sitting() || !self.rlva.has("unsit"))
    }

    /// Returns granted bits and whether the remaining question is eligible
    /// for automatic acceptance. Experiences and caution bits keep their UI.
    pub fn rlva_script_permissions(&self, task: Uuid, requested: i32) -> (i32, bool) {
        const TAKE_CONTROLS: i32 = 1 << 2;
        const ATTACH: i32 = 1 << 5;
        const TELEPORT: i32 = 1 << 12;
        let mut granted = requested;
        if self.rlva.has("addattach") {
            granted &= !ATTACH;
        }
        if self.rlva.has("tploc") {
            granted &= !TELEPORT;
        }
        let mut unanswered = granted;
        if self.rlva.has("acceptpermission")
            && let Some(object) = self.objects.index_of_uuid(&task).and_then(|i| self.objects.get(i))
        {
            unanswered &= !TAKE_CONTROLS;
            if object.owner_id == self.agent_id && self.rlva_attachment_item(task).is_none() {
                unanswered &= !ATTACH;
            }
        }
        (
            granted,
            requested != 0 && unanswered == 0 && !super::super::experiences::caution(requested),
        )
    }
}
