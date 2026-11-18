//! `@showloc`: when About Land and Region / Estate may open.
//!
//! Port of `RlvUIEnabler::canViewParcelProperties`,
//! `canViewRegionProperties` and `filterFloaterShowLoc` (indra/newview/
//! rlvui.cpp, originally LGPL 2.1). The Legacy Viewer asks the question once,
//! when the floater is about to open (`LLFloaterReg::setValidateCallback`),
//! and keeps what it answered until the floater closes; it also hides both
//! floaters at the moment `@showloc` starts. It never re-asks while they are
//! open.
//!
//! The Legacy Viewer reads a parcel it already holds. Aurora learns the
//! parcel (or the region) from the simulator a moment after the window is
//! asked for, so a third answer exists here: [`LandVerdict::Unknown`], "not
//! known yet". Treating it as a refusal closed the window of a parcel the
//! agent owns before its data arrived (the window flashed and closed, and a
//! right click on the ground did nothing); [`LandGate`] keeps the window
//! hidden and asks again until the answer is known.
use super::super::World;
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
/// open on a parcel the agent owns, or that a group owns when the agent has
/// a power of that group to return objects (`GP_LAND_RETURN`). `selected`:
/// the parcel is an explicit selection (a right click on the ground), not the
/// agent's own parcel; a selection that holds squares of other residents is
/// refused (`LLParcelSelection::hasOthersSelected`).
///
/// The Legacy Viewer does not use `LLViewerParcelMgr::isParcelOwnedByAgent`,
/// which lets a god in.
pub fn parcel_verdict(agent: Uuid, groups: &[GroupMembership], parcel: Option<&ParcelInfo>, selected: bool) -> LandVerdict {
    let Some(parcel) = parcel else {
        return LandVerdict::Unknown;
    };
    if selected && parcel.other_count != 0 {
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
    /// The verdict for About Land on the parcel it would show: the selection
    /// when there is one, the agent's parcel otherwise.
    pub fn rlva_parcel_verdict(&self, selected: bool) -> LandVerdict {
        let parcel = if selected {
            self.land.sel.as_ref().and_then(|s| s.parcel.as_ref())
        } else {
            self.parcel.as_ref()
        };
        parcel_verdict(self.agent_id, &self.groups.groups, parcel.map(|p| &**p), selected)
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

#[derive(Debug, Default, Clone, Copy)]
enum State {
    #[default]
    Closed,
    /// Asked for under `@showloc`; `selected`: the parcel was an explicit
    /// selection when it was asked for.
    Waiting {
        selected: bool,
        since: Instant,
    },
    Open,
}

/// The open-time check of one window: asks the question once when the window
/// is requested, keeps the answer until the window closes.
#[derive(Debug, Default)]
pub struct LandGate {
    state: State,
    was_restricted: bool,
}

impl LandGate {
    /// One call per frame. `requested`: the window's panel flag; `restricted`:
    /// `@showloc` is set; `selected`: read once, when the request starts;
    /// `verdict`: the question, with that `selected`.
    pub fn step(
        &mut self,
        requested: bool,
        restricted: bool,
        selected: impl FnOnce() -> bool,
        verdict: impl FnOnce(bool) -> LandVerdict,
        now: Instant,
    ) -> GateView {
        // onToggleShowLoc: a window that is open when `@showloc` starts closes
        let starts = restricted && !self.was_restricted;
        self.was_restricted = restricted;
        if !requested {
            self.state = State::Closed;
            return GateView::Closed;
        }
        if !restricted {
            self.state = State::Open;
            return GateView::Shown;
        }
        if starts && matches!(self.state, State::Open) {
            self.state = State::Closed;
            return GateView::Refused;
        }
        if matches!(self.state, State::Closed) {
            self.state = State::Waiting {
                selected: selected(),
                since: now,
            };
        }
        match self.state {
            State::Open => GateView::Shown,
            State::Waiting { selected, since } => match verdict(selected) {
                LandVerdict::Allowed => {
                    self.state = State::Open;
                    GateView::Shown
                }
                LandVerdict::Unknown if now.duration_since(since) < WAIT => GateView::Hidden,
                LandVerdict::Unknown | LandVerdict::Refused => {
                    self.state = State::Closed;
                    GateView::Refused
                }
            },
            State::Closed => GateView::Closed,
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
        for selected in [false, true] {
            assert_eq!(parcel_verdict(ME, &[], Some(&parcel(ME)), selected), LandVerdict::Allowed);
        }
    }

    #[test]
    fn another_residents_parcel_is_refused_even_with_groups() {
        let groups = [group(GROUP, u64::MAX)];
        for selected in [false, true] {
            assert_eq!(parcel_verdict(ME, &groups, Some(&parcel(OTHER)), selected), LandVerdict::Refused);
        }
    }

    #[test]
    fn a_group_parcel_opens_with_one_of_the_return_powers_only() {
        let land = parcel(GROUP);
        // GP_LAND_RETURN_GROUP_SET, _NON_GROUP and _GROUP_OWNED, each alone
        for power in [1u64 << 33, 1 << 34, 1 << 48] {
            assert_eq!(parcel_verdict(ME, &[group(GROUP, power)], Some(&land), false), LandVerdict::Allowed);
        }
        // other land powers (manage the allowed and banned lists, eject and
        // freeze, deed, set the landing point, join and subdivide, sell…)
        // and no power at all do not
        for bit in (0..64).filter(|b| ![33, 34, 48].contains(b)) {
            assert_eq!(
                parcel_verdict(ME, &[group(GROUP, 1 << bit)], Some(&land), false),
                LandVerdict::Refused,
                "bit {bit} alone must not unlock About Land"
            );
        }
        assert_eq!(parcel_verdict(ME, &[group(GROUP, 0)], Some(&land), false), LandVerdict::Refused);
    }

    #[test]
    fn a_group_parcel_of_a_group_the_agent_is_not_in_is_refused() {
        assert_eq!(
            parcel_verdict(ME, &[group(OTHER_GROUP, u64::MAX)], Some(&parcel(GROUP)), false),
            LandVerdict::Refused
        );
        assert_eq!(parcel_verdict(ME, &[], Some(&parcel(GROUP)), false), LandVerdict::Refused);
    }

    #[test]
    fn a_selection_that_holds_land_of_others_is_refused() {
        let mut land = parcel(ME);
        land.other_count = 1;
        assert_eq!(parcel_verdict(ME, &[], Some(&land), true), LandVerdict::Refused);
        // the agent's own parcel is not a selection: its counts are not read
        assert_eq!(parcel_verdict(ME, &[], Some(&land), false), LandVerdict::Allowed);
    }

    #[test]
    fn a_parcel_that_has_not_arrived_is_unknown_not_refused() {
        assert_eq!(parcel_verdict(ME, &[], None, false), LandVerdict::Unknown);
        assert_eq!(parcel_verdict(ME, &[], None, true), LandVerdict::Unknown);
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
        gate.step(requested, restricted, || false, |_| v, t0 + Duration::from_secs(t))
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
    fn the_answer_is_kept_until_the_window_closes() {
        let (mut gate, t0) = (LandGate::default(), Instant::now());
        assert_eq!(frame(&mut gate, t0, 0, true, true, LandVerdict::Allowed), GateView::Shown);
        // the agent walks onto land of someone else: the Legacy Viewer asks
        // only when the floater opens, so the window stays
        assert_eq!(frame(&mut gate, t0, 1, true, true, LandVerdict::Refused), GateView::Shown);
        // closed, then asked again: the new parcel decides
        assert_eq!(frame(&mut gate, t0, 2, false, true, LandVerdict::Refused), GateView::Closed);
        assert_eq!(frame(&mut gate, t0, 3, true, true, LandVerdict::Refused), GateView::Refused);
    }

    #[test]
    fn a_refused_window_never_draws_and_drops_its_request() {
        let (mut gate, t0) = (LandGate::default(), Instant::now());
        assert_eq!(frame(&mut gate, t0, 0, true, true, LandVerdict::Refused), GateView::Refused);
    }

    #[test]
    fn a_parcel_that_never_arrives_gives_up_after_the_wait() {
        let (mut gate, t0) = (LandGate::default(), Instant::now());
        assert_eq!(frame(&mut gate, t0, 0, true, true, LandVerdict::Unknown), GateView::Hidden);
        assert_eq!(frame(&mut gate, t0, 4, true, true, LandVerdict::Unknown), GateView::Hidden);
        assert_eq!(frame(&mut gate, t0, 6, true, true, LandVerdict::Unknown), GateView::Refused);
    }

    #[test]
    fn a_window_open_when_showloc_starts_closes() {
        let (mut gate, t0) = (LandGate::default(), Instant::now());
        assert_eq!(frame(&mut gate, t0, 0, true, false, LandVerdict::Allowed), GateView::Shown);
        // onToggleShowLoc hides it, even on the agent's own land
        assert_eq!(frame(&mut gate, t0, 1, true, true, LandVerdict::Allowed), GateView::Refused);
        // and the agent may open it again on land that is theirs
        assert_eq!(frame(&mut gate, t0, 2, true, true, LandVerdict::Allowed), GateView::Shown);
    }

    #[test]
    fn the_selection_is_read_when_the_request_starts() {
        let (mut gate, t0) = (LandGate::default(), Instant::now());
        let mut seen = Vec::new();
        for (t, selected) in [(0, true), (1, false)] {
            gate.step(
                true,
                true,
                || selected,
                |s| {
                    seen.push(s);
                    LandVerdict::Unknown
                },
                t0 + Duration::from_secs(t),
            );
        }
        assert_eq!(seen, [true, true], "a selection made later does not change what was asked");
    }
}
