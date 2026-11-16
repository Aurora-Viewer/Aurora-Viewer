//! The navmesh rebake of the agent's region: its state, the Build ›
//! Pathfinding › Rebake region entry and the two pathfinding parcel icons.
//! Port of LLMenuOptionPathfindingRebakeNavmesh (the modes, handleNavMeshStatus,
//! sendRequestRebakeNavmesh, handleRebakeNavMeshResponse,
//! handleRegionBoundaryCrossed), LLLocationInputCtrl's isNavMeshDirty, and the
//! pathfinding icons of LLStatusBar::updateParcelIcons with their blinking and
//! LORebakeStuck (indra/newview, originally LGPL 2.1).
use aurora_net::RegionHandle;
use aurora_net::pathfinding::navmesh::{State, Status};
use std::time::{Duration, Instant};

/// After this long rebaking, the icon stops blinking and stays dim
/// (LORebakeStuck's 30 s).
pub const STUCK_AFTER: Duration = Duration::from_secs(30);
/// The blink's half period (mRebakingTimer > 0.5 s).
pub const BLINK: Duration = Duration::from_millis(500);

/// ERebakeNavMeshMode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Default,
    /// Changes wait: a rebake can be asked.
    Available,
    RequestSent,
    InProgress,
    /// The navmesh is up to date.
    NotAvailable,
}

#[derive(Debug, Clone, Default)]
pub struct Rebake {
    region: Option<RegionHandle>,
    mode: Mode,
    /// isNavMeshDirty: a valid status that is not complete.
    dirty: bool,
    /// can_modify_navmesh (AgentState); the Legacy Viewer only logs a
    /// warning when it is false and sends the rebake anyway.
    pub can_rebake: bool,
    /// When this rebake started (bakingStarted), for the blinking and the
    /// "stuck" dimming.
    rebaking_since: Option<Instant>,
}

impl Rebake {
    /// handleRegionBoundaryCrossed: a new region starts from nothing; its
    /// status and agent state follow from the network.
    pub fn region_changed(&mut self, region: Option<RegionHandle>) {
        if self.region != region {
            *self = Self { region, ..Self::default() };
        }
    }

    /// handleNavMeshStatus, for the agent's region only.
    pub fn on_status(&mut self, region: RegionHandle, status: Option<Status>, now: Instant) {
        if self.region != Some(region) {
            return;
        }
        self.dirty = status.is_some_and(|s| s.state != State::Complete);
        let mode = match status.map(|s| s.state) {
            Some(State::Pending | State::Repending) => Mode::Available,
            Some(State::Building) => Mode::InProgress,
            Some(State::Complete) => Mode::NotAvailable,
            None => Mode::Default,
        };
        self.set_mode(mode, now);
    }

    pub fn on_agent_state(&mut self, region: RegionHandle, can_rebake: bool) {
        if self.region == Some(region) {
            self.can_rebake = can_rebake;
        }
    }

    /// sendRequestRebakeNavmesh: the request is sent whatever the mode (the
    /// Legacy Viewer only logs when it was not expected); returns the
    /// region to send it to.
    pub fn request(&mut self, now: Instant) -> Option<RegionHandle> {
        let region = self.region?;
        if self.mode != Mode::Available || !self.can_rebake {
            log::warn!(
                "pathfinding: rebake asked while mode {:?}, can rebake {}",
                self.mode,
                self.can_rebake
            );
        }
        self.set_mode(Mode::RequestSent, now);
        Some(region)
    }

    /// handleRebakeNavMeshResponse: true when the failure notice
    /// (PathfindingCannotRebakeNavmesh) is due.
    pub fn on_response(&mut self, region: RegionHandle, ok: bool, now: Instant) -> bool {
        if self.region != Some(region) {
            return false;
        }
        if self.mode == Mode::RequestSent {
            self.set_mode(if ok { Mode::InProgress } else { Mode::Default }, now);
        }
        !ok
    }

    fn set_mode(&mut self, mode: Mode, now: Instant) {
        let rebaking = matches!(mode, Mode::RequestSent | Mode::InProgress);
        if !rebaking {
            self.rebaking_since = None;
        } else if self.rebaking_since.is_none() {
            self.rebaking_since = Some(now);
        }
        self.mode = mode;
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// isRebakeNeeded.
    pub fn needed(&self) -> bool {
        self.mode == Mode::Available
    }

    /// isRebaking.
    pub fn rebaking(&self) -> bool {
        matches!(self.mode, Mode::RequestSent | Mode::InProgress)
    }

    /// Tools.EnablePathfindingRebakeRegion (the location bar's isNavMeshDirty).
    pub fn menu_enabled(&self) -> bool {
        self.dirty
    }

    /// The "rebake needed" icon's opacity, None when hidden: shown while a
    /// rebake is needed or running; while it runs it blinks (only where
    /// dynamic pathfinding is on, as the Legacy timer does) and goes half
    /// dim once stuck.
    pub fn dirty_icon(&self, dynamic: bool, now: Instant) -> Option<f32> {
        if self.needed() {
            return Some(1.0);
        }
        let since = self.rebaking_since.filter(|_| self.rebaking())?;
        let elapsed = now.saturating_duration_since(since);
        if elapsed >= STUCK_AFTER {
            Some(0.5)
        } else if dynamic && (elapsed.as_millis() / BLINK.as_millis()) % 2 == 1 {
            Some(0.25)
        } else {
            Some(1.0)
        }
    }

    /// The "dynamic pathfinding disabled" icon: not while the rebake one
    /// shows, and only in a region that has pathfinding (the Legacy Viewer
    /// tests !is_opensim; a region without the capabilities would show it
    /// for nothing).
    pub fn disabled_icon(&self, dynamic: bool, pathfinding_region: bool, now: Instant) -> bool {
        pathfinding_region && !dynamic && self.dirty_icon(dynamic, now).is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn status(state: State) -> Option<Status> {
        Some(Status {
            region: Uuid::nil(),
            version: 1,
            state,
        })
    }

    fn here() -> (Rebake, Instant) {
        let mut rebake = Rebake::default();
        rebake.region_changed(Some(7));
        (rebake, Instant::now())
    }

    #[test]
    fn statuses_give_the_legacy_modes() {
        let (mut r, now) = here();
        for (state, mode, dirty) in [
            (State::Pending, Mode::Available, true),
            (State::Repending, Mode::Available, true),
            (State::Building, Mode::InProgress, true),
            (State::Complete, Mode::NotAvailable, false),
        ] {
            r.on_status(7, status(state), now);
            assert_eq!((r.mode(), r.menu_enabled()), (mode, dirty), "{state:?}");
        }
        r.on_status(7, None, now);
        assert_eq!((r.mode(), r.menu_enabled()), (Mode::Default, false));
        r.on_status(8, status(State::Pending), now);
        assert_eq!(r.mode(), Mode::Default, "another region's status is ignored");
    }

    #[test]
    fn a_rebake_goes_through_its_states_and_reports_failures() {
        let (mut r, now) = here();
        r.on_status(7, status(State::Pending), now);
        assert!(r.needed());
        assert_eq!(r.request(now), Some(7));
        assert!(r.rebaking() && r.mode() == Mode::RequestSent);
        assert!(!r.on_response(7, true, now));
        assert_eq!(r.mode(), Mode::InProgress);
        r.on_status(7, status(State::Complete), now);
        assert!(!r.rebaking() && !r.needed());

        r.on_status(7, status(State::Pending), now);
        r.request(now);
        assert!(r.on_response(7, false, now), "the failure notice");
        assert_eq!(r.mode(), Mode::Default);
        assert!(!r.on_response(8, false, now), "not our region");
    }

    #[test]
    fn the_icon_blinks_while_rebaking_then_dims_once_stuck() {
        let (mut r, start) = here();
        assert_eq!(r.dirty_icon(true, start), None);
        r.on_status(7, status(State::Pending), start);
        assert_eq!(r.dirty_icon(true, start), Some(1.0));
        r.request(start);
        assert_eq!(r.dirty_icon(true, start), Some(1.0));
        assert_eq!(r.dirty_icon(true, start + Duration::from_millis(600)), Some(0.25));
        assert_eq!(
            r.dirty_icon(false, start + Duration::from_millis(600)),
            Some(1.0),
            "no blinking without dynamic pathfinding"
        );
        assert_eq!(r.dirty_icon(true, start + STUCK_AFTER), Some(0.5));
        // the status that follows the request keeps the same start
        r.on_response(7, true, start + Duration::from_secs(1));
        r.on_status(7, status(State::Building), start + Duration::from_secs(2));
        assert_eq!(r.dirty_icon(true, start + STUCK_AFTER), Some(0.5));
        r.on_status(7, status(State::Complete), start + STUCK_AFTER);
        assert_eq!(r.dirty_icon(true, start + STUCK_AFTER), None);
    }

    #[test]
    fn the_disabled_icon_needs_a_pathfinding_region_and_a_clean_navmesh() {
        let (mut r, now) = here();
        r.on_status(7, status(State::Complete), now);
        assert!(r.disabled_icon(false, true, now));
        assert!(!r.disabled_icon(true, true, now));
        assert!(!r.disabled_icon(false, false, now), "no pathfinding here: no icon");
        r.on_status(7, status(State::Pending), now);
        assert!(!r.disabled_icon(false, true, now), "the rebake icon takes its place");
    }

    #[test]
    fn a_new_region_starts_over() {
        let (mut r, now) = here();
        r.on_status(7, status(State::Pending), now);
        r.on_agent_state(7, true);
        r.region_changed(Some(9));
        assert_eq!((r.mode(), r.can_rebake, r.menu_enabled()), (Mode::Default, false, false));
        r.on_agent_state(7, true);
        assert!(!r.can_rebake, "the old region's answer is ignored");
    }
}
