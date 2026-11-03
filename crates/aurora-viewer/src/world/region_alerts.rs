//! Two alerts about the region we are in, from the Legacy Viewer's
//! Preferences › Alerts (llviewermessage.cpp, originally LGPL 2.1):
//! - the region's total script count jumped or dropped by at least the
//!   threshold (FSReportTotalScriptCountChanges, process_sim_stats), told in
//!   nearby chat;
//! - the region we arrived in runs another simulator version than the last
//!   one (FSShowServerVersionChangeNotice, process_agent_movement_complete),
//!   told in a notice; off by default in Aurora, on in the Legacy Viewer
//!   (it came at nearly every teleport, human choice of 2026-10-30).

use super::World;
use super::notifications::{Data, Kind};
use aurora_net::estate::{SimStats, sim_stat};

/// What the user chose (copied from the settings by the app).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    pub script_count: bool,
    pub script_threshold: u32,
    pub version: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            script_count: false,
            script_threshold: 100,
            version: false,
        }
    }
}

#[derive(Debug, Default)]
pub struct RegionAlerts {
    pub cfg: Config,
    /// The last active script count (kept across regions and while the
    /// alert is off, like the Legacy Viewer's static).
    scripts: Option<i64>,
    /// gLastVersionChannel.
    version: String,
}

impl RegionAlerts {
    /// The change to tell, as (old, new), when it passes the threshold.
    fn script_change(&mut self, stats: &SimStats) -> Option<(i64, i64)> {
        let now = stats.get(sim_stat::NUMSCRIPTSACTIVE)? as i64;
        let before = self.scripts.replace(now)?;
        let threshold = self.cfg.script_threshold as i64;
        (self.cfg.script_count && (now - before).abs() >= threshold && now != before).then_some((before, now))
    }

    /// The previous version when the new one differs (never at login).
    fn version_change(&mut self, version: &str) -> Option<String> {
        if version == self.version {
            return None;
        }
        let before = std::mem::replace(&mut self.version, version.to_owned());
        (!before.is_empty() && self.cfg.version).then_some(before)
    }
}

impl World {
    pub(super) fn on_sim_stats_alert(&mut self, stats: &SimStats) {
        if let Some((old, new)) = self.region_alerts.script_change(stats) {
            let diff = format!("{:+}", new - old);
            let text = if new > old {
                aurora_i18n::tr!("world-region-alerts-scripts-jumped", old = old, new = new, diff = diff)
            } else {
                aurora_i18n::tr!("world-region-alerts-scripts-dropped", old = old, new = new, diff = diff)
            };
            self.system_notice(text, crate::world::notice_topics::Topic::Region);
        }
    }

    pub(super) fn on_sim_version_alert(&mut self, version: &str) {
        if let Some(old) = self.region_alerts.version_change(version) {
            self.notifications.push(
                Kind::Info,
                aurora_i18n::tr!("world-region-alerts-version-title"),
                aurora_i18n::tr!("world-region-alerts-version-body", new = version.to_owned(), old = old),
                Data::None,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(scripts: f32) -> SimStats {
        SimStats {
            values: vec![(sim_stat::NUMSCRIPTSACTIVE, scripts)],
            object_capacity: 0,
        }
    }

    #[test]
    fn the_script_count_is_told_past_the_threshold_only() {
        let mut a = RegionAlerts::default();
        // off: the count is still followed
        assert_eq!(a.script_change(&stats(1000.0)), None);
        a.cfg.script_count = true;
        assert_eq!(a.script_change(&stats(1500.0)), Some((1000, 1500)));
        assert_eq!(a.script_change(&stats(1450.0)), None, "under the threshold");
        assert_eq!(a.script_change(&stats(1300.0)), Some((1450, 1300)));
        assert_eq!(a.script_change(&SimStats::default()), None, "no count in the message");
    }

    #[test]
    fn the_first_count_is_never_told() {
        let mut a = RegionAlerts::default();
        a.cfg.script_count = true;
        assert_eq!(a.script_change(&stats(5000.0)), None);
    }

    #[test]
    fn the_version_notice_reaches_the_notifications_only_when_on() {
        use crate::scene::avatar::AvatarLibrary;
        let mut w = World::new(std::sync::Arc::new(AvatarLibrary::load()));
        let title = aurora_i18n::tr!("world-region-alerts-version-title");
        let told = |w: &World| w.notifications.list.iter().filter(|n| n.title == title).count();
        w.on_sim_version_alert("Server 1");
        w.on_sim_version_alert("Server 2");
        assert_eq!(told(&w), 0, "off by default: no notice");
        w.region_alerts.cfg.version = true;
        w.on_sim_version_alert("Server 3");
        assert_eq!(told(&w), 1, "on: one notice");
    }

    #[test]
    fn a_new_version_is_not_told_by_default() {
        let mut a = RegionAlerts::default();
        assert_eq!(a.version_change("Server 1"), None, "login");
        assert_eq!(a.version_change("Server 2"), None, "off by default");
    }

    #[test]
    fn a_new_version_is_told_after_the_first_region() {
        let mut a = RegionAlerts::default();
        a.cfg.version = true;
        assert_eq!(a.version_change("Server 1"), None, "login");
        assert_eq!(a.version_change("Server 1"), None);
        assert_eq!(a.version_change("Server 2").as_deref(), Some("Server 1"));
        a.cfg.version = false;
        assert_eq!(a.version_change("Server 3"), None);
        a.cfg.version = true;
        assert_eq!(a.version_change("Server 1").as_deref(), Some("Server 3"), "followed while off");
    }
}
