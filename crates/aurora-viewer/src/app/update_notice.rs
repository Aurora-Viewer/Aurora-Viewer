//! The notice of a new Aurora version found by a periodic check
//! (`updater.rs`): a card of the notifications in the world, the update
//! window on the other screens.

use super::*;
use crate::world::notifications::{Data, Kind, Response};

impl App {
    /// The new version a periodic check found: its notice in the world, the
    /// update window elsewhere (the login screen has no notifications).
    pub(super) fn update_notice(&mut self) {
        // a probe that timed out when the card was raised, then a later
        // check that found the folder protected: the card's button says
        // so too, not only its action
        if self.updater.blocked.is_some() {
            mark_cards_blocked(&mut self.world.notifications.list);
        }
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
        self.world.notifications.push(
            Kind::Info,
            title,
            body,
            Data::Update {
                ready: notice.ready,
                blocked: self.updater.blocked.is_some(),
            },
        );
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
        let Some((ready, blocked)) = self.world.notifications.list.iter().find_map(|n| match n.data {
            Data::Update { ready, blocked } if n.id == id => Some((ready, blocked)),
            _ => None,
        }) else {
            return;
        };
        if !matches!(r, Response::Accept) || !self.updater.enabled {
            return;
        }
        self.notif_ui.open = false;
        if notice_opens_window(
            ready,
            blocked,
            self.updater.blocked.is_some(),
            matches!(self.updater.state, crate::updater::State::Offer),
        ) {
            self.updater.open = true;
        } else {
            self.updater.download(self.net.runtime());
        }
    }
}

/// Whether the notice's Update opens the update window instead of starting
/// the download. A card raised while the folder could still be written
/// (`card_blocked` false) checks the updater's own answer too: a slow probe
/// may have found the folder protected since.
fn notice_opens_window(ready: bool, card_blocked: bool, updater_blocked: bool, offer: bool) -> bool {
    ready || card_blocked || updater_blocked || !offer
}

/// Marks the waiting update cards blocked, so that their button reads
/// "How to update…" once the updater knows this installation cannot
/// install the release.
fn mark_cards_blocked(list: &mut [crate::world::notifications::Notification]) {
    for n in list {
        if let Data::Update { ready: false, blocked } = &mut n.data {
            *blocked = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{mark_cards_blocked, notice_opens_window};
    use crate::world::notifications::{Data, Kind, Notifications};

    #[test]
    fn a_card_raised_before_the_folder_was_found_protected_says_how_to_update() {
        let mut notifications = Notifications::default();
        notifications.push(
            Kind::Info,
            "v9.9.9",
            "",
            Data::Update {
                ready: false,
                blocked: false,
            },
        );
        notifications.push(
            Kind::Info,
            "v9.9.9",
            "",
            Data::Update {
                ready: true,
                blocked: false,
            },
        );
        mark_cards_blocked(&mut notifications.list);
        let flags: Vec<_> = notifications
            .list
            .iter()
            .filter_map(|n| match n.data {
                Data::Update { ready, blocked } => Some((ready, blocked)),
                _ => None,
            })
            .collect();
        // the offer's card turns; a downloaded update's Install stays
        assert!(flags.contains(&(false, true)));
        assert!(flags.contains(&(true, false)));
    }

    #[test]
    fn the_notice_downloads_only_an_offer_this_installation_can_install() {
        // an ordinary offer downloads from the card
        assert!(!notice_opens_window(false, false, false, true));
        // a card raised for a protected folder opens the window
        assert!(notice_opens_window(false, true, true, true));
        // a card raised before the probe found the folder protected opens
        // the window too, never the download
        assert!(notice_opens_window(false, false, true, true));
        // a downloaded update, or no offer any more, opens the window
        assert!(notice_opens_window(true, false, false, true));
        assert!(notice_opens_window(false, false, false, false));
    }
}
