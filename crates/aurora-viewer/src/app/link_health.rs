//! The link with the agent's region as the user feels it: the pill while the
//! region is silent, and "Reconnect" to the same place after a lost link
//! (ui/link_health.rs). The Legacy Viewer only ever shows "connection lost"
//! and sends the user back to the login screen with their usual start
//! location: going back where they were is Aurora's own.

use super::{App, Screen};
use crate::ui::link_health;

/// AURORA_DEMO_LINK: `stall:<frame>:<seconds>` makes the region silent from
/// that frame for that long (the pill and its count), `lost:<frame>` ends the
/// session on a lost link at that frame (the login screen and "Reconnect").
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct LinkHealthDemo {
    stall: Option<(u64, f32)>,
    lost_at: Option<u64>,
}

impl LinkHealthDemo {
    pub(super) fn from_env() -> Option<LinkHealthDemo> {
        std::env::var("AURORA_DEMO_LINK").ok().and_then(|v| LinkHealthDemo::parse(&v))
    }

    fn parse(value: &str) -> Option<LinkHealthDemo> {
        let mut link = LinkHealthDemo::default();
        for part in value.split(',') {
            let mut fields = part.trim().split(':');
            match fields.next()? {
                "stall" => {
                    let frame = fields.next()?.parse().ok()?;
                    let seconds = fields.next().map_or(Some(20.0), |s| s.parse().ok())?;
                    link.stall = Some((frame, seconds));
                }
                "lost" => link.lost_at = Some(fields.next()?.parse().ok()?),
                _ => return None,
            }
        }
        (link != LinkHealthDemo::default()).then_some(link)
    }

    /// Seconds of silence at `frame` (60 frames a second, the demo's pace),
    /// infinite outside the stall like a link that never was.
    fn packet_age(&self, frame: u64) -> f32 {
        match self.stall {
            Some((from, seconds)) if frame >= from => {
                let age = (frame - from) as f32 / 60.0;
                if age <= seconds { age } else { f32::INFINITY }
            }
            _ => f32::INFINITY,
        }
    }
}

impl App {
    /// The session ended: a lost link offers to reconnect where the user was
    /// (a kick does not), a region restart keeps its own message.
    pub(super) fn on_disconnected(&mut self, reason: &str, lost: bool) {
        self.session_ended();
        let restart = self.restart_disconnect();
        let place = if lost && restart.is_none() {
            link_health::reconnect_place(&self.world.region_name(), self.world.agent.position)
        } else {
            None
        };
        let reconnect = restart.is_some() || place.is_some();
        let message = match (restart, &place) {
            (Some(restart), _) => restart,
            (None, Some(_)) => aurora_i18n::tr!("app-disconnected-lost", reason = reason.to_string()),
            (None, None) => aurora_i18n::tr!("app-disconnected", reason = reason.to_string()),
        };
        if let Some(place) = &place {
            log::info!("link lost: \"Reconnect\" goes back to {}", place.to_login_string());
        }
        // YouHaveBeenLoggedOut for the system, while the session still counts
        self.tell_system(crate::world::attention::Notice {
            title: aurora_i18n::tr!("world-attention-logged-out").to_owned(),
            body: message.clone(),
            open: crate::world::attention::Open::Window,
        });
        self.reconnect_place = place.map(|place| (self.settings.login_uri(), self.settings.username.trim().to_lowercase(), place));
        self.back_to_login(Some(message));
        self.login_form.reconnect = reconnect;
    }

    /// AURORA_DEMO_LINK's lost link at its frame, at the top of the frame as
    /// a network event is (never from the interface: the frame holds the
    /// window, and the login screen needs it to give the cursor back).
    pub(super) fn demo_link_lost(&mut self) {
        if matches!(self.screen, Screen::World)
            && let Some(demo) = self.demo_link_health
            && demo.lost_at == Some(self.frame_count)
        {
            self.on_disconnected(aurora_i18n::tr!("session-connection-to-the-region-was-lost"), true);
        }
    }

    /// The pill while the agent's region is silent (and the demo's link).
    pub(super) fn show_link_health(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        if !matches!(self.screen, Screen::World) {
            return;
        }
        let age = match &self.demo_link_health {
            Some(demo) => demo.packet_age(self.frame_count),
            None if self.world.teleporting => f32::INFINITY,
            None => {
                let motion = *self.net.stats.simulator_motion.lock();
                if motion.handle.is_some() && motion.handle == self.world.main_region {
                    motion.packet_age()
                } else {
                    f32::INFINITY
                }
            }
        };
        if let Some(seconds) = link_health::stall_seconds(age) {
            link_health::show(ctx, p, seconds);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_demo_link_reads_a_stall_and_a_lost_link() {
        let link = LinkHealthDemo::parse("stall:600:12, lost:1500").unwrap();
        assert_eq!(link.stall, Some((600, 12.0)));
        assert_eq!(link.lost_at, Some(1500));
        assert!(link.packet_age(599).is_infinite());
        assert_eq!(link.packet_age(900), 5.0);
        assert!(link.packet_age(600 + 13 * 60).is_infinite());
        assert_eq!(LinkHealthDemo::parse("stall:10").unwrap().stall, Some((10, 20.0)));
        assert!(LinkHealthDemo::parse("nonsense:3").is_none());
        assert!(LinkHealthDemo::parse("").is_none());
    }
}
