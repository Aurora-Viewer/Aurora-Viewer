//! The small capabilities of the agent's region (#660): the server release
//! notes, the region console, nearby voice moderation and the LSL keywords
//! (`crate::release_notes`, `crate::sim_console`, `crate::voice_moderation`,
//! `crate::lsl_syntax`).

use super::Session;
use crate::caps;
use crate::types::NetEvent;
use crate::voice_moderation::{self, Target};

impl Session<'_> {
    pub(super) fn fetch_pathfinding(&mut self, request: uuid::Uuid, region: crate::RegionHandle, kind: crate::pathfinding::Kind) {
        use crate::pathfinding::{self, Error};
        let events = self.sh.events.clone();
        let url = self
            .sims
            .values()
            .find(|sim| sim.handle == region)
            .and_then(|sim| sim.caps.get(kind.capability()))
            .cloned();
        let http = self.sh.caps_http.clone();
        tokio::spawn(async move {
            let result = if let Some(url) = url {
                match tokio::time::timeout(std::time::Duration::from_secs(20), caps::get_llsd(&http, &url)).await {
                    Ok(Ok(data)) => pathfinding::parse(&data, kind),
                    _ => Err(Error::Failed),
                }
            } else {
                Err(Error::Unsupported)
            };
            let _ = events.send(NetEvent::Pathfinding {
                request,
                region,
                kind,
                result,
            });
        });
    }

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

    /// LLSyntaxIdLSL::initialize: the cached keywords of `id`, else the
    /// region's; read and turned into keywords off the frame.
    pub(super) fn fetch_lsl_syntax(&mut self, id: uuid::Uuid, cache_dir: std::path::PathBuf) {
        let url = self.main_cap("LSLSyntax");
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            let doc = crate::lsl_syntax::load(&http, url.as_deref(), &cache_dir, id).await;
            let keywords = doc.and_then(|d| crate::lsl_syntax::keywords(&d).ok_or_else(|| "no keyword".to_owned()));
            match &keywords {
                Ok(k) => log::info!("lsl syntax: {id} has {} keywords", k.len()),
                Err(e) => log::warn!("lsl syntax: {id} unavailable ({e})"),
            }
            let _ = events.send(NetEvent::LslSyntax { id, keywords });
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
