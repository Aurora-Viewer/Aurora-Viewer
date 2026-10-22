//! The state of the link with the agent's region, as the user feels it: a
//! discreet pill while the region has gone quiet for a few seconds (a blip,
//! a lagging simulator, a laptop waking up), and the place to go back to when
//! the link was lost for good.
//!
//! The Legacy Viewer gives no sign before its "connection lost" (it only
//! greys the network bar of the statistics); it calls a circuit blocked after
//! three pings left unanswered (LLCircuitData::pingTimerStart,
//! PING_START_BLOCK, pings every 5 s) and dead after 100 s without one
//! (llstartup.cpp, circuit_timeout). A simulator sends something every
//! second (SimStats, object updates, pings), so a few silent seconds already
//! mean trouble: the pill shows from STALL_AFTER and tells the user the
//! viewer is still waiting for the region, not frozen.

use crate::theme::Palette;
use aurora_net::login::StartLocation;
use egui::{Align2, Id, RichText, Vec2};

/// Silence of the agent's region after which the pill shows (s).
pub const STALL_AFTER: f32 = 5.0;

/// Whole seconds of silence to show, or nothing while the link is fine (or
/// was never up: no packet yet, `packet_age` infinite).
pub fn stall_seconds(packet_age: f32) -> Option<u32> {
    (packet_age.is_finite() && packet_age >= STALL_AFTER).then_some(packet_age as u32)
}

/// Where "Reconnect" takes the user after the link with the region was lost:
/// the region they were in, where they stood (the login's `uri:` start,
/// coordinates in the region's metres, LLSLURL::getLocationString).
pub fn reconnect_place(region: &str, position: glam::Vec3) -> Option<StartLocation> {
    let name = region.trim();
    if name.is_empty() || !position.is_finite() {
        return None;
    }
    let clamp = |v: f32, max: f32| v.clamp(0.0, max).round() as u32;
    Some(StartLocation::Region {
        name: name.to_owned(),
        x: clamp(position.x, 255.0),
        y: clamp(position.y, 255.0),
        z: clamp(position.z, 4096.0),
    })
}

/// The pill under the top bar while the region is silent.
pub fn show(ctx: &egui::Context, p: &Palette, seconds: u32) {
    let text = aurora_i18n::tr!("ui-link-stalled", seconds = seconds as i64);
    egui::Area::new(Id::unique("link_health_pill"))
        .order(egui::Order::Foreground)
        .anchor(Align2::CENTER_TOP, Vec2::new(0.0, 64.0))
        .interactable(false)
        .show(ctx, |ui| {
            super::widgets::overlay(ui, p, egui::Margin::symmetric(12, 6), |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    super::widgets::glyph(ui, "wifi-off", p.amber, 15.0);
                    ui.label(RichText::new(text).size(12.5).color(p.ink));
                });
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pill_shows_only_after_a_few_silent_seconds_of_a_link_that_was_up() {
        assert_eq!(stall_seconds(0.4), None);
        assert_eq!(stall_seconds(STALL_AFTER - 0.1), None);
        assert_eq!(stall_seconds(STALL_AFTER), Some(5));
        assert_eq!(stall_seconds(42.7), Some(42));
        // no packet ever (demo, login): nothing to say
        assert_eq!(stall_seconds(f32::INFINITY), None);
        assert_eq!(stall_seconds(f32::NAN), None);
    }

    #[test]
    fn reconnect_goes_back_to_the_region_and_the_spot() {
        let place = reconnect_place(" Some Region ", glam::Vec3::new(127.6, 3.2, 22.9)).unwrap();
        assert_eq!(place.to_login_string(), "uri:Some Region&128&3&23");
        // a position just across the border (region crossing in flight)
        let edge = reconnect_place("R", glam::Vec3::new(-0.4, 258.0, -3.0)).unwrap();
        assert_eq!(edge.to_login_string(), "uri:R&0&255&0");
        assert!(reconnect_place("", glam::Vec3::ZERO).is_none());
        assert!(reconnect_place("R", glam::Vec3::new(f32::NAN, 0.0, 0.0)).is_none());
    }
}
