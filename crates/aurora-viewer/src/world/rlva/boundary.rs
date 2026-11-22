//! The RLVa send boundary: what every outgoing command goes through before
//! it leaves, whoever asks (the interface, a script's force command, the
//! RLVa self-test). `App::send_from` and the self-test's offline host call
//! the same two functions, so that what is tested is what is shipped.
use super::super::World;
use aurora_net::{NetCommand, NetEvent};
use uuid::Uuid;

/// What the boundary decided for one command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Boundary {
    /// The command leaves, possibly reduced (`rlva_prepare_outfit`, a locked
    /// attachment filtered out of a detach request).
    Send,
    /// Nothing is left of it, or it is not to be sent: dropped silently.
    Dropped,
    /// A restriction refuses it: the caller shows `rlva-blocked`.
    Refused,
}

/// The answer the caller gives back for a refused command that someone
/// waits on. A refused inventory change fails as one the server turned down
/// would, so that whoever holds its request lets it go: the inventory takes
/// one change at a time, and a change it still waits for ignores every later
/// one (a move to the Trash refused under @sharedwear blocked the inventory
/// until the next login).
pub fn refused_reply(command: &NetCommand, reason: &str) -> Option<NetEvent> {
    match command {
        NetCommand::EditInventory { request, .. } => Some(NetEvent::InventoryEdited {
            request: *request,
            result: Err(reason.to_owned()),
        }),
        _ => None,
    }
}

/// The name of a command (its variant), for the log: the command is written
/// only up to its first word, not whole (a chat line, a full outfit write).
pub fn command_name(command: &NetCommand) -> String {
    use std::fmt::Write;
    /// Takes the characters up to the end of the name, then refuses the rest:
    /// the formatting stops there.
    struct Name(String);
    impl Write for Name {
        fn write_str(&mut self, text: &str) -> std::fmt::Result {
            for c in text.chars() {
                if matches!(c, '(' | ' ' | '{') {
                    return Err(std::fmt::Error);
                }
                self.0.push(c);
            }
            Ok(())
        }
    }
    let mut name = Name(String::new());
    // (the error is the stop, not a failure)
    let _ = write!(name, "{command:?}");
    name.0
}

/// Whether a refused command is told to the user ("This action is restricted
/// by RLVa."): what they or a script asked for is. What the lock watchdog
/// sends to put a lock back is the viewer's own work, as
/// RlvAttachmentLockWatchdog's (indra/newview/rlvlocks.cpp), which never says
/// anything of it to the user (#3633).
pub fn refusal_shown(actor: Uuid) -> bool {
    actor != super::watchdog::ACTOR
}

impl World {
    /// Reduces `command` to what the restrictions leave of it, then asks
    /// whether it may leave. `actor` is nil for the user.
    pub fn rlva_send_boundary(&mut self, command: &mut NetCommand, actor: Uuid) -> Boundary {
        self.rlva_prepare_outfit(command, actor);
        if !self.rlva_prepare_attachments(command, actor) {
            return Boundary::Dropped;
        }
        if !actor.is_nil() && !self.rlva_normalize_teleport(command) {
            return Boundary::Dropped;
        }
        if let NetCommand::OfferTeleport { to, .. } = command {
            self.rlva_filter_teleport_offer(to);
        }
        if actor.is_nil() && self.rlva_take_decided(command) {
            return Boundary::Send;
        }
        // callback_script_dialog (llviewermessage.cpp): the answer of a
        // dialog on channel 0, shown before @sendchat, is dropped silently
        if actor.is_nil() && matches!(command, NetCommand::ScriptDialogReply { channel: 0, .. }) && self.rlva.has("sendchat") {
            return Boundary::Dropped;
        }
        let im_placeholder = actor.is_nil() && self.rlva_prepare_outgoing_im(command);
        if !im_placeholder && !self.rlva.teleport_already_checked && !self.rlva_allows_command(command, actor) {
            return Boundary::Refused;
        }
        Boundary::Send
    }

    /// What RLVa remembers of a command that passed the boundary: the wear
    /// it now watches, the sit it expects. Under RLVa an attachment is always
    /// added on the wire; the watchdog replaces what a replacing wear may
    /// replace once the object's point is known.
    pub fn rlva_note_sent(&mut self, command: &mut NetCommand, actor: Uuid, now: std::time::Instant) {
        // @showloc compares a pick update to its previous saved position.
        // Only a permitted update can become that position for the next edit.
        if let NetCommand::PickUpdate(pick) = command {
            self.profiles.pick_saved(self.agent_id, (**pick).clone());
        }
        self.rlva_note_attachment_command(command, actor, now);
        self.note_outfit_command(command);
        self.rlva.note_sit_command(command, actor);
        if let NetCommand::OfferTeleport { to, .. } = command {
            self.teleport_invites.request_answers.retain(|id| !to.contains(id));
        }
        // handle_object_sit (llviewermenu.cpp) and RlvForceHandler<RLV_BHVR_SIT>
        // (rlvhandler.cpp), setSitSource: a sit asked while @standtp is held,
        // by hand or by a script, records where the avatar stood. Standing
        // up brings it back there (rlva_sitting_tick). `/siton` and the area
        // search record nothing (`Sitting::no_sit_source`).
        if matches!(command, NetCommand::RequestSit { .. }) && self.rlva.has("standtp") && !self.rlva.sitting.no_sit_source {
            self.rlva.stand_origin = self.main_region.map(|h| (h, self.agent.position));
        }
        if self.rlva.enabled()
            && let NetCommand::RezAttachments(items) = command
        {
            for item in items {
                item.add = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_is_named_by_its_variant_alone() {
        let chat = NetCommand::Chat {
            message: "a long line that must not be formatted".repeat(50),
            channel: 0,
            chat_type: aurora_net::ChatType::Normal,
        };
        assert_eq!(command_name(&chat), "Chat");
        assert_eq!(command_name(&NetCommand::DetachAttachments(vec![Uuid::nil()])), "DetachAttachments");
    }

    #[test]
    fn only_the_watchdogs_own_refusals_are_kept_from_the_user() {
        assert!(refusal_shown(Uuid::nil()));
        assert!(refusal_shown(Uuid::from_u128(7)));
        assert!(!refusal_shown(super::super::watchdog::ACTOR));
    }
}
