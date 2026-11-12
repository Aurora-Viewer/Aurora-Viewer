//! Cancel of a teleport in progress: whether the teleport screen offers it,
//! and what a cancel leaves behind (LLAgent::teleportCancel,
//! restoreCanceledTeleportRequest, indra/newview/llagent.cpp; the Cancel
//! button of LLProgressView, indra/newview/llprogressview.cpp; its
//! visibility in process_teleport_start, process_teleport_progress and
//! process_teleport_finish, indra/newview/llviewermessage.cpp; RLVa's
//! RlvHandler::m_fCanCancelTp, indra/newview/rlvhandler.h).

use super::World;

/// TELEPORT_FLAGS_DISABLE_CANCEL (indra/llmessage/llteleportflags.h): a
/// teleport the simulator will not cancel (llTeleportAgentHome, a god's
/// lure, death).
pub const TELEPORT_FLAGS_DISABLE_CANCEL: u32 = 1 << 11;

/// What the simulator and RLVa say about cancelling the current teleport.
#[derive(Debug, Default)]
pub struct TeleportCancel {
    /// The last TeleportStart / TeleportProgress of this teleport: None
    /// before either (the request has left, the simulator has not answered)
    /// and from TeleportFinish on ("it can't be cancelled now"); Some(true)
    /// when the simulator allows a cancel.
    simulator: Option<bool>,
    /// RLVa forbids it (@tpto, a lure accepted by @accepttp): set before
    /// the request, until the teleport fails or arrives.
    rlva_locked: bool,
    /// A cancel was sent for a teleport that had left (mTeleportCanceled):
    /// a TeleportFinish that comes anyway resumes it.
    canceled: bool,
}

impl TeleportCancel {
    /// Whether the teleport screen shows its Cancel button. `always` is
    /// Preferences' "Always show the teleport Cancel button"
    /// (FSAlwaysShowTPCancel): it overrides the simulator's flag, never RLVa.
    pub fn offered(&self, always: bool) -> bool {
        self.simulator.is_some_and(|allowed| allowed || always) && !self.rlva_locked
    }

    /// TeleportStart or TeleportProgress, with their TeleportFlags.
    pub fn on_simulator_flags(&mut self, flags: u32) {
        self.simulator = Some(flags & TELEPORT_FLAGS_DISABLE_CANCEL == 0);
    }

    /// A new teleport request (LLAgent::startTeleportRequest resets
    /// mTeleportCanceled). The RLVa lock, set just before, stays.
    pub fn on_request(&mut self) {
        self.simulator = None;
        self.canceled = false;
    }

    /// RlvHandler::setCanCancelTp(false).
    pub fn lock_for_rlva(&mut self) {
        self.rlva_locked = true;
    }

    /// TeleportFinish: the simulator has moved us, the button goes. Returns
    /// whether it resumes a cancelled teleport (canRestoreCanceledTeleport).
    pub fn on_finish(&mut self) -> bool {
        self.simulator = None;
        std::mem::take(&mut self.canceled)
    }

    /// The teleport failed or arrived (RlvHandler::onTeleportFailed /
    /// onTeleportFinished, LLAgent::handleTeleportFinished).
    pub fn on_end(&mut self) {
        *self = Self::default();
    }

    /// A TeleportProgress after a cancel, for a teleport we no longer
    /// follow (process_teleport_progress drops it in TELEPORT_NONE).
    pub fn ignores_progress(&self, teleporting: bool) -> bool {
        self.canceled && !teleporting
    }

    fn on_cancel(&mut self, sent: bool) {
        self.simulator = None;
        self.canceled = sent;
    }
}

/// What a cancel asks of the app.
#[derive(Debug, PartialEq, Eq)]
pub enum Cancelled {
    /// The request had left: send TeleportCancel.
    Sent,
    /// The request was still waiting here (for the content rating to be in
    /// sync): dropped, nothing to send.
    Dropped,
}

impl World {
    /// The Cancel button of the teleport screen (LLAgent::teleportCancel):
    /// the teleport stops here and the resident stays where they are. None
    /// when there is nothing to cancel, or the teleport may not be.
    pub fn cancel_teleport(&mut self, always_offered: bool) -> Option<Cancelled> {
        if !self.teleporting || !self.tp_cancel.offered(always_offered) {
            return None;
        }
        // hasPendingTeleportRequest: a request held for the maturity sync
        // never left, so there is nothing to tell the simulator
        let outcome = if self.maturity.holds_teleport() {
            self.maturity.drop_teleport();
            Cancelled::Dropped
        } else {
            Cancelled::Sent
        };
        log::info!(
            "teleport cancelled ({})",
            match outcome {
                Cancelled::Sent => "sending TeleportCancel",
                Cancelled::Dropped => "the request had not left",
            }
        );
        self.tp_cancel.on_cancel(outcome == Cancelled::Sent);
        // clearTeleportRequest + setTeleportState(TELEPORT_NONE). Marked as
        // a teleport that did not happen, for those that wait on its end
        // (follow, friend join, portals, the teleport screen's fade).
        self.cancel_friend_join_flight();
        self.cancel_friend_join();
        self.tp_history.failed();
        self.teleporting = false;
        self.tp_failed = true;
        self.local_teleport = false;
        Some(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::NetEvent;
    use std::sync::Arc;

    fn world() -> World {
        World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()))
    }

    fn started(world: &mut World, flags: u32) {
        world.begin_teleport(true);
        world.apply(NetEvent::TeleportStarted { flags });
    }

    #[test]
    fn the_button_waits_for_the_simulator_and_goes_at_teleport_finish() {
        let mut c = TeleportCancel::default();
        c.on_request();
        assert!(!c.offered(false), "no button before TeleportStart");
        c.on_simulator_flags(0);
        assert!(c.offered(false));
        c.on_simulator_flags(TELEPORT_FLAGS_DISABLE_CANCEL);
        assert!(!c.offered(false), "TeleportProgress can take it away");
        assert!(c.offered(true), "FSAlwaysShowTPCancel overrides the simulator");
        c.on_simulator_flags(0);
        assert!(!c.on_finish());
        assert!(!c.offered(true), "nothing to cancel after TeleportFinish");
    }

    #[test]
    fn rlva_takes_the_button_away_even_when_always_shown() {
        let mut c = TeleportCancel::default();
        c.lock_for_rlva();
        c.on_request();
        c.on_simulator_flags(0);
        assert!(!c.offered(false) && !c.offered(true));
        c.on_end();
        c.on_simulator_flags(0);
        assert!(c.offered(false), "the lock ends with its teleport");
    }

    #[test]
    fn a_cancel_stops_the_teleport_and_keeps_the_resident_in_place() {
        let mut world = world();
        started(&mut world, 0);
        assert!(world.teleporting && world.tp_show_progress);
        assert_eq!(world.cancel_teleport(false), Some(Cancelled::Sent));
        assert!(!world.teleporting && world.tp_failed);
        assert!(!world.tp_cancel.offered(true));
        // pressed twice: one TeleportCancel only (mCancelBtn->setEnabled(false))
        assert_eq!(world.cancel_teleport(false), None);
        // a late TeleportProgress does not bring the screen back
        world.apply(NetEvent::TeleportProgress {
            message: "sending_dest".into(),
            flags: 0,
        });
        assert!(!world.teleporting);
    }

    #[test]
    fn a_teleport_the_simulator_will_not_cancel_cannot_be() {
        let mut world = world();
        started(&mut world, TELEPORT_FLAGS_DISABLE_CANCEL);
        assert_eq!(world.cancel_teleport(false), None);
        assert!(world.teleporting);
        assert_eq!(world.cancel_teleport(true), Some(Cancelled::Sent));
    }

    #[test]
    fn a_teleport_finish_after_a_cancel_resumes_it() {
        // process_teleport_finish: the simulator ignored the cancel or got
        // it too late; the teleport is complete on its side
        let mut world = world();
        world.main_region = Some(1);
        started(&mut world, 0);
        world.cancel_teleport(false);
        world.apply(NetEvent::TeleportFinished { handle: 2, flying: false });
        assert!(world.teleporting && world.tp_show_progress && !world.tp_failed);
        assert!(!world.tp_cancel.offered(true));
        assert!(!world.tp_cancel.canceled, "restored once");
    }

    #[test]
    fn a_request_held_for_the_rating_is_dropped_without_a_message() {
        let mut world = world();
        world.maturity.request(2, 0);
        world.maturity.hold(aurora_net::NetCommand::TeleportHome, uuid::Uuid::nil());
        started(&mut world, 0);
        assert_eq!(world.cancel_teleport(false), Some(Cancelled::Dropped));
        assert!(!world.maturity.holds_teleport());
        assert!(!world.tp_cancel.canceled, "nothing to resume");
    }

    #[test]
    fn a_failure_or_an_arrival_resets_everything() {
        let mut world = world();
        world.tp_cancel.lock_for_rlva();
        started(&mut world, 0);
        world.apply(NetEvent::viewer_teleport_failed(String::new()));
        assert!(!world.tp_cancel.rlva_locked && !world.tp_cancel.offered(true));
        world.tp_cancel.lock_for_rlva();
        started(&mut world, 0);
        world.apply(NetEvent::AgentMovementComplete {
            handle: 1,
            position: glam::Vec3::ZERO,
            look_at: glam::Vec3::X,
        });
        assert!(!world.tp_cancel.rlva_locked);
    }
}
