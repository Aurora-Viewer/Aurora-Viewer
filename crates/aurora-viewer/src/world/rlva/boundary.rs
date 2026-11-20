//! The RLVa send boundary: what every outgoing command goes through before
//! it leaves, whoever asks (the interface, a script's force command, the
//! RLVa self-test). `App::send_from` and the self-test's offline host call
//! the same two functions, so that what is tested is what is shipped.
use super::super::World;
use aurora_net::NetCommand;
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
        if !im_placeholder && !self.rlva_allows_command(command, actor) {
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
        // handle_object_sit (llviewermenu.cpp) and RlvForceHandler<RLV_BHVR_SIT>
        // (rlvhandler.cpp), setSitSource: a sit asked while @standtp is held,
        // by hand or by a script, records where the avatar stood. Standing
        // up brings it back there (rlva_update_stand).
        if matches!(command, NetCommand::RequestSit { .. }) && self.rlva.has("standtp") {
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
