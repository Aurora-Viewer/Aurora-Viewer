//! The avatar render info between the viewer and the regions (aurora_net's
//! render_info.rs): the avatars we see handed to the network side every few
//! seconds, which reports them to their regions on its own schedule, and
//! the regions' answers and our own complexity turned into the "Your avatar
//! complexity is N" notice (`world/complexity_notice.rs`).
//!
//! Nothing here runs every frame beyond a timer check: the complexities and
//! silhouettes are the scene's, already computed.

use super::App;
use crate::world::complexity_notice::{Action, Context, Notifier, OwnAvatar};
use aurora_net::RegionHandle;
use aurora_net::render_info::{REQUEST_PERIOD, SCAN_PERIOD, SeenAvatar, Visibility};
use std::time::Instant;

/// The scene refreshes every avatar's complexity every 2 s
/// (`Scene::update_complexity`): ours is read as often.
const OWN_PERIOD: std::time::Duration = std::time::Duration::from_secs(2);

pub(super) struct Tracker {
    notifier: Notifier,
    published_at: Option<Instant>,
    own_at: Option<Instant>,
    /// `AURORA_DEMO_RENDER_INFO`: the made-up answer's last time.
    demo_at: Option<Instant>,
    demo_logged: bool,
}

impl Default for Tracker {
    fn default() -> Self {
        Tracker {
            notifier: Notifier::new(Instant::now()),
            published_at: None,
            own_at: None,
            demo_at: None,
            demo_logged: false,
        }
    }
}

/// `AURORA_DEMO_RENDER_INFO=<overlimit>[/<reporting>]`: the counts the
/// demo's region answers with (8 viewers reporting by default).
fn demo_counts(value: &str) -> Option<Visibility> {
    let (over, reporting) = value.split_once('/').unwrap_or((value, "8"));
    let over_limit = over.trim().parse().ok()?;
    let reporting = reporting.trim().parse().ok()?;
    Some(Visibility { reporting, over_limit })
}

impl App {
    fn complexity_context(&self) -> Context {
        Context {
            seconds: self.settings.complexity_notice_seconds,
            mouselook: self.camera.mouselook(),
        }
    }

    /// Once per frame in the world.
    pub(super) fn update_render_info(&mut self) {
        if !self.in_world() {
            return;
        }
        let now = Instant::now();
        if self.render_info.published_at.is_none_or(|t| now.duration_since(t) >= SCAN_PERIOD) {
            self.render_info.published_at = Some(now);
            let seen = self.seen_avatars();
            if self.demo {
                self.demo_log_render_info(&seen);
            }
            self.net.set_seen_avatars(seen);
        }
        if self.render_info.own_at.is_none_or(|t| now.duration_since(t) >= OWN_PERIOD) {
            self.render_info.own_at = Some(now);
            if let Some(own) = self.own_avatar_state() {
                let cx = self.complexity_context();
                if let Some(action) = self.render_info.notifier.own_avatar(own, now, cx) {
                    self.complexity_notice(action);
                }
            }
        }
        self.demo_render_info(now);
    }

    /// The avatars avatarRenderInfoReportCoro reports: those mostly loaded
    /// (getRezzedStatus() >= 2), ours included, with their complexity and
    /// whether we draw them as silhouettes. (Animated objects are no avatars
    /// here.)
    fn seen_avatars(&self) -> Vec<SeenAvatar> {
        self.world
            .objects
            .avatars()
            .filter(|(idx, _)| self.scene.avatar_rez_status(&self.world, *idx, false) >= 2)
            .map(|(idx, o)| SeenAvatar {
                id: o.full_id,
                region: o.key.region,
                weight: self.scene.avatar_complexity.get(&o.full_id).copied().unwrap_or(0),
                too_complex: self.scene.too_complex.contains(&idx),
            })
            .collect()
    }

    fn own_avatar_state(&self) -> Option<OwnAvatar> {
        let id = self.world.agent_id;
        let idx = self.world.objects.index_of_uuid(&id)?;
        let complexity = *self.scene.avatar_complexity.get(&id)?;
        let rez_status = self.scene.avatar_rez_status(&self.world, idx, true);
        Some(OwnAvatar {
            complexity,
            wearables_loaded: rez_status >= 1,
            outfit_complete: rez_status == 4,
            cof_version: self.world.appearance_cof_version(&id),
            rez_status,
        })
    }

    /// A region's counts (NetEvent::AvatarRenderInfo). An answer from a
    /// region since left is dropped, as the Legacy Viewer's coroutine does.
    pub(super) fn render_info_answer(&mut self, handle: RegionHandle, visibility: Visibility) {
        if !self.world.regions.contains_key(&handle) {
            return;
        }
        let cx = self.complexity_context();
        if let Some(action) = self.render_info.notifier.region_answer(visibility, Instant::now(), cx) {
            self.complexity_notice(action);
        }
    }

    fn complexity_notice(&mut self, action: Action) {
        let notices = &mut self.world.notifications;
        match action {
            Action::Show(n) => {
                // FIRE-19958: the complexity with its digits grouped
                let title = aurora_i18n::tr!(
                    "avatar-complexity-notice",
                    complexity = aurora_i18n::integer(i64::from(n.complexity))
                );
                let body = n.over_limit.map(|o| o.text().to_owned()).unwrap_or_default();
                notices.replace_complexity(Some((title, body)), n.seconds);
            }
            Action::Cancel => {
                notices.replace_complexity(None, 0);
            }
        }
    }

    /// `AURORA_DEMO_RENDER_INFO`: the demo's region answers every 15 s
    /// with the given counts, as a region's GET would.
    fn demo_render_info(&mut self, now: Instant) {
        if !self.demo || self.render_info.demo_at.is_some_and(|t| now.duration_since(t) < REQUEST_PERIOD) {
            return;
        }
        let Some(counts) = std::env::var("AURORA_DEMO_RENDER_INFO").ok().and_then(|v| demo_counts(&v)) else {
            return;
        };
        let Some(handle) = self.world.main_region else {
            return;
        };
        self.render_info.demo_at = Some(now);
        self.render_info_answer(handle, counts);
    }

    /// `AURORA_DEMO_RENDER_INFO`: the report the agent's region would get,
    /// written to the log once our avatar is in it.
    fn demo_log_render_info(&mut self, seen: &[SeenAvatar]) {
        if self.render_info.demo_logged || std::env::var_os("AURORA_DEMO_RENDER_INFO").is_none() {
            return;
        }
        let Some(handle) = self.world.main_region else {
            return;
        };
        if !seen.iter().any(|a| a.id == self.world.agent_id && a.weight > 0) {
            return;
        }
        self.render_info.demo_logged = true;
        let (body, count) = aurora_net::render_info::report(seen, handle);
        log::info!(
            "avatar render info (demo): {count} avatars reported: {}",
            aurora_llsd::to_xml_string(&body)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_counts_read_over_limit_and_reporting() {
        assert_eq!(
            demo_counts("3"),
            Some(Visibility {
                reporting: 8,
                over_limit: 3
            })
        );
        assert_eq!(
            demo_counts("5/12"),
            Some(Visibility {
                reporting: 12,
                over_limit: 5
            })
        );
        assert_eq!(demo_counts("x"), None);
    }
}
