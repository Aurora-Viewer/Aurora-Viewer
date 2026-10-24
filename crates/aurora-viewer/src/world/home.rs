//! World › Set Home to Here, as the Legacy Viewer does it (LLAgent::
//! setStartPosition / setStartPositionSuccess / setHomePosRegion,
//! indra/newview/llagent.cpp; LLWorldSetHomeLocation /
//! LLWorldEnableSetHomeLocation, llviewermenu.cpp; originally LGPL 2.1):
//! the agent's feet, kept inside the region and above the land, sent to the
//! simulator, which checks the land and answers with an alert. The home of
//! the world map follows the position the simulator kept.

use super::{World, notifications};
use crate::ui_sound::UiSound;
use aurora_net::home::{Alert, REGION_ALLOW_SET_HOME, Reply};
use aurora_net::{NetCommand, RegionHandle};
use glam::Vec3;

/// How far inside the region's edges the home stays (setStartPosition's
/// INSET, metres).
const INSET: f32 = 0.5;
/// LLWorld::getRegionMaxHeight (MAX_OBJECT_Z).
const MAX_HEIGHT: f32 = 4096.0;

/// The request sent and not yet confirmed: the region and the position asked.
#[derive(Debug, Default)]
pub struct SetHome {
    pending: Option<(RegionHandle, Vec3)>,
}

/// setStartPosition: from the avatar's centre down to its feet, inside the
/// region by `INSET`, between the land and the highest object.
pub fn start_position(centre: Vec3, half_height: f32, region_width: f32, land: impl Fn(Vec3) -> Option<f32>) -> Vec3 {
    let mut p = centre;
    p.z -= half_height;
    p.x = p.x.clamp(INSET, (region_width - INSET).max(INSET));
    p.y = p.y.clamp(INSET, (region_width - INSET).max(INSET));
    let floor = land(p).unwrap_or(0.0).min(MAX_HEIGHT);
    p.z = p.z.clamp(floor, MAX_HEIGHT);
    p
}

impl World {
    /// LLWorldEnableSetHomeLocation: the agent's region allows it
    /// (REGION_FLAGS_ALLOW_SET_HOME). Aurora has no god mode, which the
    /// Legacy Viewer also lets through.
    pub fn can_set_home(&self) -> bool {
        self.main()
            .and_then(|r| r.info.as_ref())
            .is_some_and(|i| i.region_flags & REGION_ALLOW_SET_HOME != 0)
    }

    /// LLWorldSetHomeLocation: the request for the agent's place; the
    /// simulator checks the land and answers with an alert.
    pub fn set_home_here(&mut self) -> Option<NetCommand> {
        let handle = self.main_region?;
        let width = self.main().map_or(256.0, |r| r.heightmap.size_x as f32);
        let half_height = self
            .objects
            .index_of_uuid(&self.agent_id)
            .and_then(|idx| self.objects.get(idx))
            .map_or(0.84, |o| 0.5 * o.scale.z);
        let position = start_position(self.agent.position, half_height, width, |p| self.ground_height(p));
        self.set_home.pending = Some((handle, position));
        Some(NetCommand::SetHomeLocation {
            position,
            look_at: self.agent.forward(),
        })
    }

    /// setHomePosRegion: the home of the world map.
    fn home_at(&mut self, handle: RegionHandle, position: Vec3) {
        let (x, y) = aurora_net::handle_to_origin(handle);
        self.map.home = Some((x as f64 + position.x as f64, y as f64 + position.y as f64, position.z));
        log::info!("home: set in region {handle:x} at {position:.1?}");
    }

    /// The HomeLocation capability's answer (setStartPositionSuccess): the
    /// position the simulator kept, in the region the request was made in.
    pub(super) fn on_home_location(&mut self, reply: Reply) {
        match reply {
            Reply::Set(position) => {
                if let Some((handle, _)) = self.set_home.pending.take() {
                    self.home_at(handle, position);
                }
            }
            // its alert says why
            Reply::Refused => log::warn!("home: error in response to home position set"),
            Reply::Failed => {
                self.set_home.pending = None;
                self.home_message(notifications::Kind::Alert, aurora_i18n::tr!("world-home-set-failed"));
            }
        }
    }

    /// The simulator's alert, in the viewer's words; "Home position set."
    /// confirms the request (the only answer without the capability).
    pub(super) fn on_home_alert(&mut self, alert: Alert) {
        let text = match alert {
            Alert::Set => {
                // the capability's answer, when it comes, has the exact place
                if let Some((handle, position)) = self.set_home.pending {
                    self.home_at(handle, position);
                }
                aurora_i18n::tr!("world-home-position-set")
            }
            Alert::NotInRegion => aurora_i18n::tr!("world-home-cant-set-in-region"),
            Alert::NotHere => aurora_i18n::tr!("world-home-valid-locations"),
            Alert::Reset => aurora_i18n::tr!("world-home-reset-not-legal"),
        };
        if alert != Alert::Set {
            self.set_home.pending = None;
        }
        let kind = if alert == Alert::Set {
            notifications::Kind::Info
        } else {
            notifications::Kind::Alert
        };
        self.home_message(kind, text);
    }

    fn home_message(&mut self, kind: notifications::Kind, text: &str) {
        if kind == notifications::Kind::Alert {
            self.ui_sounds.push(UiSound::Alert);
        }
        self.system_notice(text, crate::world::notice_topics::Topic::Region);
        self.notifications
            .push(kind, aurora_i18n::tr!("common-home"), text.to_owned(), notifications::Data::None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::NetEvent;
    use std::sync::Arc;

    #[test]
    fn the_home_is_the_feet_inside_the_region_and_above_the_land() {
        let flat = |_: Vec3| Some(20.0);
        assert_eq!(
            start_position(Vec3::new(128.0, 64.0, 31.0), 1.0, 256.0, flat),
            Vec3::new(128.0, 64.0, 30.0)
        );
        // drifted past the edge, sunk under the land
        assert_eq!(
            start_position(Vec3::new(-3.0, 300.0, 10.0), 0.9, 256.0, flat),
            Vec3::new(0.5, 255.5, 20.0)
        );
        // a variable region, and a sky box above the highest object
        assert_eq!(
            start_position(Vec3::new(500.0, 1.0, 5000.0), 1.0, 512.0, |_| None),
            Vec3::new(500.0, 1.0, 4096.0)
        );
    }

    fn world(flags: u32) -> (World, RegionHandle) {
        let mut w = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let mut info = crate::demo::events()
            .into_iter()
            .find_map(|e| match e {
                NetEvent::RegionHandshake(info) => Some((*info).clone()),
                _ => None,
            })
            .expect("demo handshake");
        let handle = aurora_net::origin_to_handle(256_000, 256_512);
        info.handle = handle;
        info.region_flags = flags;
        w.main_region = Some(handle);
        w.ensure_region(handle, (256, 256));
        w.apply(NetEvent::RegionHandshake(Arc::new(info)));
        w.agent.position = Vec3::new(100.0, 40.0, 30.0);
        (w, handle)
    }

    #[test]
    fn the_menu_entry_follows_the_region_flag() {
        assert!(world(REGION_ALLOW_SET_HOME).0.can_set_home());
        assert!(!world(aurora_net::region_flags::ALLOW_VOICE).0.can_set_home());
    }

    #[test]
    fn the_capability_answer_moves_the_map_home_to_the_kept_place() {
        let (mut w, _) = world(REGION_ALLOW_SET_HOME);
        let Some(NetCommand::SetHomeLocation { position, look_at }) = w.set_home_here() else {
            panic!("a request");
        };
        assert!((position.z - (30.0 - 0.84)).abs() < 1e-4, "{position}");
        assert!((look_at.length() - 1.0).abs() < 1e-4);
        w.apply(NetEvent::HomeLocation(Reply::Set(Vec3::new(100.0, 40.0, 29.0))));
        assert_eq!(w.map.home, Some((256_100.0, 256_552.0, 29.0)));
        // then the simulator's alert confirms it, without moving it again
        w.apply(NetEvent::HomeAlert(Alert::Set));
        assert_eq!(w.map.home, Some((256_100.0, 256_552.0, 29.0)));
        assert_eq!(w.notifications.list.len(), 1);
    }

    #[test]
    fn without_the_capability_the_alert_confirms_the_place_asked() {
        let (mut w, _) = world(REGION_ALLOW_SET_HOME);
        let Some(NetCommand::SetHomeLocation { position, .. }) = w.set_home_here() else {
            panic!("a request");
        };
        w.apply(NetEvent::HomeAlert(Alert::Set));
        assert_eq!(w.map.home, Some((256_100.0, 256_552.0, position.z)));
    }

    #[test]
    fn a_refusal_or_a_failure_keeps_the_old_home_and_says_so() {
        for (event, notified) in [
            (NetEvent::HomeAlert(Alert::NotHere), true),
            (NetEvent::HomeAlert(Alert::NotInRegion), true),
            // the alert that follows says why
            (NetEvent::HomeLocation(Reply::Refused), false),
            (NetEvent::HomeLocation(Reply::Failed), true),
        ] {
            let (mut w, _) = world(REGION_ALLOW_SET_HOME);
            w.map.home = Some((1.0, 2.0, 3.0));
            w.set_home_here().expect("a request");
            w.apply(event);
            assert_eq!(w.map.home, Some((1.0, 2.0, 3.0)));
            assert_eq!(!w.notifications.list.is_empty(), notified);
        }
        // a refusal forgets the request: a later "set" alert is not ours
        let (mut w, _) = world(REGION_ALLOW_SET_HOME);
        w.set_home_here().expect("a request");
        w.apply(NetEvent::HomeAlert(Alert::NotHere));
        w.apply(NetEvent::HomeAlert(Alert::Set));
        assert_eq!(w.map.home, None);
    }
}
