//! The small capabilities of the agent's region (#660): the server release
//! notes, the region console and nearby voice moderation
//! (`crate::release_notes`, `crate::sim_console`, `crate::voice_moderation`).

use super::Session;
use crate::caps;
use crate::types::NetEvent;
use crate::voice_moderation::{self, Target};

impl Session<'_> {
    /// LLFloaterAbout::startFetchServerReleaseNotes: the page behind the
    /// capability, or ErrorFetchingServerReleaseNotesURL.
    pub(super) fn fetch_server_release_notes(&mut self) {
        let events = self.sh.events.clone();
        let Some(url) = self.main_cap("ServerReleaseNotes") else {
            let _ = events.send(NetEvent::ServerReleaseNotes(Err("no ServerReleaseNotes capability".into())));
            return;
        };
        tokio::spawn(async move {
            let page = crate::release_notes::fetch(&url).await;
            match &page {
                Ok(page) => log::info!("release notes: {page}"),
                Err(e) => log::warn!("release notes: no page ({e})"),
            }
            let _ = events.send(NetEvent::ServerReleaseNotes(page));
        });
    }

    /// LLFloaterRegionDebugConsole::onInput: the answer comes on the event
    /// queue; only a failed POST answers here (UNABLE_TO_SEND_COMMAND).
    pub(super) fn send_sim_console(&mut self, command: String) {
        let events = self.sh.events.clone();
        let Some(url) = self.main_cap("SimConsoleAsync") else {
            let _ = events.send(NetEvent::SimConsoleReply(Err("no SimConsoleAsync capability".into())));
            return;
        };
        let http = self.sh.caps_http.clone();
        tokio::spawn(async move {
            if let Err(e) = caps::post_llsd(&http, &url, &crate::sim_console::command_body(&command)).await {
                log::warn!("region console: the command was not received ({e})");
                let _ = events.send(NetEvent::SimConsoleReply(Err(e.to_string())));
            }
        });
    }

    /// LLNearbyVoiceModeration::requestMuteIndividual / requestMuteAll. The
    /// Legacy Viewer asks the region of the resident's avatar; the spatial
    /// voice we moderate is our region's, so its capability is used for both.
    pub(super) fn moderate_voice(&mut self, target: Target, mute: bool) {
        let events = self.sh.events.clone();
        let everyone = target == Target::Everyone;
        let Some(url) = self.main_cap("SpatialVoiceModerationRequest") else {
            log::warn!("voice moderation: no SpatialVoiceModerationRequest capability here");
            let _ = events.send(NetEvent::VoiceModerationFailed { everyone });
            return;
        };
        log::info!("voice moderation: {target:?} mute={mute}");
        let http = self.sh.caps_http.clone();
        tokio::spawn(async move {
            match caps::post_llsd(&http, &url, &voice_moderation::body(target, mute)).await {
                Ok(_) => log::info!("voice moderation: {target:?} set to mute={mute}"),
                Err(e) => {
                    log::warn!("voice moderation: unable to change {target:?} ({e})");
                    let _ = events.send(NetEvent::VoiceModerationFailed { everyone });
                }
            }
        });
    }
}
