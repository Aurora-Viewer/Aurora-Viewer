//! The notice of a new Aurora version found by a periodic check
//! (`updater.rs`): a card of the notifications in the world, the update
//! window on the other screens.

use super::*;
use crate::world::notifications::{Data, Kind, Response};

impl App {
    /// The new version a periodic check found: its notice in the world, the
    /// update window elsewhere (the login screen has no notifications).
    pub(super) fn update_notice(&mut self) {
        let Some(notice) = self.updater.take_notice().or_else(|| self.demo_update_notice()) else {
            return;
        };
        if !matches!(self.screen, Screen::World) {
            self.updater.open = true;
            return;
        }
        // one card per version: a newer one, or the same one downloaded,
        // replaces the card still waiting
        self.world.notifications.list.retain(|n| !matches!(n.data, Data::Update { .. }));
        let (title, body) = if notice.ready {
            (
                aurora_i18n::tr!("update-notice-ready-title", version = notice.version.clone()),
                aurora_i18n::tr!("update-ready").to_owned(),
            )
        } else {
            (
                aurora_i18n::tr!("update-notice-title", version = notice.version.clone()),
                aurora_i18n::tr!(
                    "update-notice-body",
                    current = aurora_version::version().to_owned(),
                    version = notice.version.clone()
                ),
            )
        };
        self.world
            .notifications
            .push(Kind::Info, title, body, Data::Update { ready: notice.ready });
    }

    /// AURORA_DEMO_UPDATE_NOTICE=1|ready: the notice of a made-up version,
    /// once the world is shown (frame 700). Its buttons only close it: the
    /// demo checks for nothing.
    fn demo_update_notice(&self) -> Option<crate::updater::Notice> {
        if !self.demo || self.frame_count != 700 || !matches!(self.screen, Screen::World) {
            return None;
        }
        let mode = std::env::var("AURORA_DEMO_UPDATE_NOTICE").ok()?;
        Some(crate::updater::Notice {
            version: "9.9.9".into(),
            ready: mode == "ready",
        })
    }

    /// The notice's buttons: Update downloads in the update window, Install
    /// (downloaded already) opens it on its install step. Later and the
    /// close button leave it until the next launch. The card itself goes
    /// with the usual answer (`World::respond_notification`).
    pub(super) fn update_notice_answer(&mut self, id: u64, r: &Response) {
        let Some(ready) = self.world.notifications.list.iter().find_map(|n| match n.data {
            Data::Update { ready, .. } if n.id == id => Some(ready),
            _ => None,
        }) else {
            return;
        };
        if !matches!(r, Response::Accept) || !self.updater.enabled {
            return;
        }
        self.notif_ui.open = false;
        if ready || !matches!(self.updater.state, crate::updater::State::Offer) {
            self.updater.open = true;
        } else {
            self.updater.download(self.net.runtime());
        }
    }
}
