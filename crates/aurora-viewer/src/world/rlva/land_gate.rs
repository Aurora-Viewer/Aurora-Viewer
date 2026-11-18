//! `@showloc`: when About Land and Region / Estate may be shown.
//!
//! Port of `RlvUIEnabler::canViewParcelProperties`,
//! `canViewRegionProperties` and `filterFloaterShowLoc` (indra/newview/
//! rlvui.cpp, originally LGPL 2.1). The Legacy Viewer reads a parcel it
//! already holds, when the floater is asked for. Aurora learns the parcel (or
//! the region) from the simulator a moment after the window is asked for, so
//! a third answer exists here: [`LandVerdict::Unknown`], "not known yet".
//! Treating it as a refusal closed the window of a parcel the agent owns
//! before its data arrived (the window flashed and closed, and a right click
//! on the ground did nothing); [`LandGate`] keeps the window hidden until the
//! answer is known.
//!
//! Differences from the Legacy Viewer, on purpose:
//! - the question is asked again every frame for the parcel the window shows,
//!   not once when it opens: an About Land window open on the agent's land
//!   would turn into the land of someone else when the ground menu's "About
//!   Land" selects another parcel, and the Legacy Viewer lets it show;
//! - `LLParcelSelection::hasOthersSelected` refuses a right click on a
//!   group's land even for a member with the power (the simulator counts the
//!   squares of a group as "other"); the button opens that same parcel, so
//!   Aurora asks only that the selection is one parcel
//!   (`PARCEL_RESULT_MULTIPLE` is refused), which is what that test is for.
use super::super::World;
use aurora_net::land::PARCEL_RESULT_MULTIPLE;
use aurora_net::{GroupMembership, ParcelInfo, RegionInfo};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// `GP_LAND_RETURN` (indra/newview/roles_constants.h): return the objects
/// set to the group, those not set to it and those the group owns.
pub const GP_LAND_RETURN: u64 = (1 << 33) | (1 << 34) | (1 << 48);

/// How long a window waits for the parcel or the region before it gives up
/// and stays closed.
const WAIT: Duration = Duration::from_secs(5);

/// The answer of `canViewParcelProperties` / `canViewRegionProperties`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LandVerdict {
    Allowed,
    Refused,
    /// The parcel or the region has not arrived yet.
    Unknown,
}

/// `RlvUIEnabler::canViewParcelProperties` under `@showloc`: About Land may
/// show a parcel the agent owns, or that a group owns when the agent has a
/// power of that group to return objects (`GP_LAND_RETURN`), and only one
/// parcel at a time. The Legacy Viewer does not use
/// `LLViewerParcelMgr::isParcelOwnedByAgent`, which lets a god in.
pub fn parcel_verdict(agent: Uuid, groups: &[GroupMembership], parcel: Option<&ParcelInfo>) -> LandVerdict {
    let Some(parcel) = parcel else {
        return LandVerdict::Unknown;
    };
    if parcel.request_result == PARCEL_RESULT_MULTIPLE {
        return LandVerdict::Refused;
    }
    if parcel.owner_id == agent {
        return LandVerdict::Allowed;
    }
    match groups.iter().find(|g| g.id == parcel.owner_id) {
        Some(group) if group.powers & GP_LAND_RETURN != 0 => LandVerdict::Allowed,
        _ => LandVerdict::Refused,
    }
}

/// `RlvUIEnabler::canViewRegionProperties` under `@showloc`: Region / Estate
/// opens for the region's owner and for an estate manager, without the god
/// exception of `LLRegion::canManageEstate`.
pub fn region_verdict(agent: Uuid, region: Option<&RegionInfo>) -> LandVerdict {
    match region {
        None => LandVerdict::Unknown,
        Some(r) if r.owner == agent || r.is_estate_manager => LandVerdict::Allowed,
        Some(_) => LandVerdict::Refused,
    }
}

impl World {
    /// The verdict for About Land on the parcel it shows: the selection when
    /// there is one, the agent's parcel otherwise (what the window selects
    /// when it opens).
    pub fn rlva_parcel_verdict(&self) -> LandVerdict {
        let parcel = match &self.land.sel {
            Some(sel) => sel.parcel.as_ref(),
            None => self.parcel.as_ref(),
        };
        parcel_verdict(self.agent_id, &self.groups.groups, parcel.map(|p| &**p))
    }

    pub fn rlva_region_verdict(&self) -> LandVerdict {
        region_verdict(self.agent_id, self.main().and_then(|r| r.info.as_deref()))
    }
}

/// What a gated window does this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateView {
    /// Not asked for.
    Closed,
    /// Asked for, the answer is not known yet: nothing is drawn.
    Hidden,
    /// Allowed: the window is drawn.
    Shown,
    /// Refused: the window stays closed and its request is dropped.
    Refused,
}

/// The check of one window, stepped once a frame.
#[derive(Debug, Default)]
pub struct LandGate {
    /// Since when the answer has been unknown.
    waiting_since: Option<Instant>,
    /// The last answer, to log it once when it changes.
    last: Option<LandVerdict>,
    /// The window was drawn on the last frame: while its next parcel loads
    /// it keeps drawing (a window with no parcel shows nothing of it), so
    /// that closing it for a frame does not drop the selection.
    shown: bool,
}

impl LandGate {
    /// `requested`: the window's panel flag; `restricted`: `@showloc` is
    /// set; `verdict`: the question, for what the window would show now.
    pub fn step(&mut self, name: &str, requested: bool, restricted: bool, verdict: impl FnOnce() -> LandVerdict, now: Instant) -> GateView {
        let view = self.decide(name, requested, restricted, verdict, now);
        self.shown = view == GateView::Shown;
        view
    }

    fn decide(&mut self, name: &str, requested: bool, restricted: bool, verdict: impl FnOnce() -> LandVerdict, now: Instant) -> GateView {
        if !requested {
            self.waiting_since = None;
            self.last = None;
            return GateView::Closed;
        }
        if !restricted {
            self.waiting_since = None;
            return GateView::Shown;
        }
        let verdict = verdict();
        if self.last != Some(verdict) {
            self.last = Some(verdict);
            log::info!("@showloc {name}: {verdict:?}");
        }
        match verdict {
            LandVerdict::Allowed => {
                self.waiting_since = None;
                GateView::Shown
            }
            LandVerdict::Refused => {
                self.waiting_since = None;
                GateView::Refused
            }
            LandVerdict::Unknown => {
                let since = *self.waiting_since.get_or_insert(now);
                if now.duration_since(since) < WAIT {
                    if self.shown { GateView::Shown } else { GateView::Hidden }
                } else {
                    log::info!("@showloc {name}: nothing arrived in {} s, refused", WAIT.as_secs());
                    self.waiting_since = None;
                    GateView::Refused
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: Uuid = Uuid::from_u128(1);
    const OTHER: Uuid = Uuid::from_u128(2);
    const GROUP: Uuid = Uuid::from_u128(3);
    const OTHER_GROUP: Uuid = Uuid::from_u128(4);

    fn parcel(owner: Uuid) -> ParcelInfo {
        ParcelInfo {
            owner_id: owner,
            ..Default::default()
        }
    }

    fn group(id: Uuid, powers: u64) -> GroupMembership {
        GroupMembership {
            id,
            name: String::new(),
            insignia: Uuid::nil(),
            powers,
            accept_notices: true,
            list_in_profile: true,
            contribution: 0,
        }
    }

    fn region(owner: Uuid, manager: bool) -> RegionInfo {
        RegionInfo {
            handle: 0,
            name: String::new(),
            region_id: Uuid::nil(),
            water_height: 20.0,
            sim_access: 0,
            region_flags: 0,
            terrain_base: [Uuid::nil(); 4],
            terrain_detail: [Uuid::nil(); 4],
            terrain_start_height: [0.0; 4],
            terrain_height_range: [0.0; 4],
            size_x: 256,
            size_y: 256,
            is_main: true,
            owner,
            is_estate_manager: manager,
            product_name: String::new(),
            billable_factor: 1.0,
        }
    }

    #[test]
    fn an_own_parcel_opens_with_no_group_and_no_power() {
        assert_eq!(parcel_verdict(ME, &[], Some(&parcel(ME))), LandVerdict::Allowed);
    }

    #[test]
    fn another_residents_parcel_is_refused_even_with_groups() {
        let groups = [group(GROUP, u64::MAX)];
        assert_eq!(parcel_verdict(ME, &groups, Some(&parcel(OTHER))), LandVerdict::Refused);
    }

    #[test]
    fn a_group_parcel_opens_with_one_of_the_return_powers_only() {
        let land = parcel(GROUP);
        // GP_LAND_RETURN_GROUP_SET, _NON_GROUP and _GROUP_OWNED, each alone
        for power in [1u64 << 33, 1 << 34, 1 << 48] {
            assert_eq!(parcel_verdict(ME, &[group(GROUP, power)], Some(&land)), LandVerdict::Allowed);
        }
        // the other powers (manage the allowed and banned lists, eject and
        // freeze, deed, set the landing point, join and subdivide, sell…) and
        // no power at all do not
        for bit in (0..64).filter(|b| ![33, 34, 48].contains(b)) {
            assert_eq!(
                parcel_verdict(ME, &[group(GROUP, 1 << bit)], Some(&land)),
                LandVerdict::Refused,
                "bit {bit} alone must not unlock About Land"
            );
        }
        assert_eq!(parcel_verdict(ME, &[group(GROUP, 0)], Some(&land)), LandVerdict::Refused);
    }

    #[test]
    fn a_group_parcel_of_a_group_the_agent_is_not_in_is_refused() {
        assert_eq!(
            parcel_verdict(ME, &[group(OTHER_GROUP, u64::MAX)], Some(&parcel(GROUP))),
            LandVerdict::Refused
        );
        assert_eq!(parcel_verdict(ME, &[], Some(&parcel(GROUP))), LandVerdict::Refused);
    }

    #[test]
    fn a_group_parcel_counted_as_others_land_still_opens_for_a_member_with_the_power() {
        // the simulator counts the squares of a group as "other" (other_count
        // 4096, self_count 0 on the grid): the right click must not read it
        let mut land = parcel(GROUP);
        land.other_count = 4096;
        assert_eq!(parcel_verdict(ME, &[group(GROUP, 1 << 33)], Some(&land)), LandVerdict::Allowed);
    }

    #[test]
    fn a_selection_of_several_parcels_is_refused() {
        let mut land = parcel(ME);
        land.request_result = PARCEL_RESULT_MULTIPLE;
        assert_eq!(parcel_verdict(ME, &[], Some(&land)), LandVerdict::Refused);
    }

    #[test]
    fn a_parcel_that_has_not_arrived_is_unknown_not_refused() {
        assert_eq!(parcel_verdict(ME, &[], None), LandVerdict::Unknown);
    }

    #[test]
    fn region_estate_opens_for_the_owner_and_the_estate_managers_only() {
        assert_eq!(region_verdict(ME, Some(&region(ME, false))), LandVerdict::Allowed);
        assert_eq!(region_verdict(ME, Some(&region(OTHER, true))), LandVerdict::Allowed);
        assert_eq!(region_verdict(ME, Some(&region(OTHER, false))), LandVerdict::Refused);
        assert_eq!(region_verdict(ME, None), LandVerdict::Unknown);
    }

    /// One frame at `t` seconds after `t0`.
    fn frame(gate: &mut LandGate, t0: Instant, t: u64, requested: bool, restricted: bool, v: LandVerdict) -> GateView {
        gate.step("test", requested, restricted, || v, t0 + Duration::from_secs(t))
    }

    #[test]
    fn without_showloc_the_window_opens_whatever_the_parcel() {
        let (mut gate, t0) = (LandGate::default(), Instant::now());
        assert_eq!(frame(&mut gate, t0, 0, true, false, LandVerdict::Refused), GateView::Shown);
        assert_eq!(frame(&mut gate, t0, 1, false, false, LandVerdict::Refused), GateView::Closed);
    }

    #[test]
    fn a_window_waits_for_its_parcel_instead_of_closing() {
        let (mut gate, t0) = (LandGate::default(), Instant::now());
        // the first frames: the parcel is not there, nothing is drawn, the
        // request stays
        assert_eq!(frame(&mut gate, t0, 0, true, true, LandVerdict::Unknown), GateView::Hidden);
        assert_eq!(frame(&mut gate, t0, 1, true, true, LandVerdict::Unknown), GateView::Hidden);
        // it arrives and it is the agent's: the window shows
        assert_eq!(frame(&mut gate, t0, 2, true, true, LandVerdict::Allowed), GateView::Shown);
    }

    #[test]
    fn an_open_window_follows_the_parcel_it_shows() {
        let (mut gate, t0) = (LandGate::default(), Instant::now());
        assert_eq!(frame(&mut gate, t0, 0, true, true, LandVerdict::Allowed), GateView::Shown);
        // the ground menu selects land of someone else: the window has no
        // parcel to show while it loads, then closes
        assert_eq!(frame(&mut gate, t0, 1, true, true, LandVerdict::Unknown), GateView::Shown);
        assert_eq!(frame(&mut gate, t0, 2, true, true, LandVerdict::Refused), GateView::Refused);
    }

    #[test]
    fn a_refused_window_drops_its_request_and_opens_again_on_land_that_is_allowed() {
        let (mut gate, t0) = (LandGate::default(), Instant::now());
        assert_eq!(frame(&mut gate, t0, 0, true, true, LandVerdict::Refused), GateView::Refused);
        assert_eq!(frame(&mut gate, t0, 1, false, true, LandVerdict::Allowed), GateView::Closed);
        assert_eq!(frame(&mut gate, t0, 2, true, true, LandVerdict::Allowed), GateView::Shown);
    }

    #[test]
    fn a_parcel_that_never_arrives_gives_up_after_the_wait() {
        let (mut gate, t0) = (LandGate::default(), Instant::now());
        assert_eq!(frame(&mut gate, t0, 0, true, true, LandVerdict::Unknown), GateView::Hidden);
        assert_eq!(frame(&mut gate, t0, 4, true, true, LandVerdict::Unknown), GateView::Hidden);
        assert_eq!(frame(&mut gate, t0, 6, true, true, LandVerdict::Unknown), GateView::Refused);
        // a later request waits again from the start
        assert_eq!(frame(&mut gate, t0, 7, true, true, LandVerdict::Unknown), GateView::Hidden);
    }
}
