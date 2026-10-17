//! What the Region / Estate window knows of the agent's region and its
//! estate (LLFloaterRegionInfo, newview/llfloaterregioninfo.cpp,
//! llregioninfomodel.cpp, llestateinfomodel.cpp, originally LGPL 2.1): the
//! RegionInfo answer, the estate of `estateupdateinfo`, the access lists,
//! the experiences, the restart schedule, the region environment and the
//! simulator's statistics, and the requests that bring them. While the
//! window is open each RegionInfo asks the panels again, as processRegionInfo
//! does with refreshFromRegion.

use super::land::EnvSummary;
use aurora_net::estate::{
    EstateAccess, EstateCommand, EstateEvent, EstateInfo, RegionDetails, RegionExperiences, RegionRules, RegionSun, RestartSchedule,
    SimStats, TerrainSettings, TopObjects,
};
use aurora_net::{NetCommand, RegionHandle};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// Who the agent is to the estate (LLViewerRegion::canManageEstate: the
/// estate owner, a manager, or neither).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Resident,
    Manager,
    Owner,
}

/// The owner is the region's (RegionHandshake SimOwner), replaced by the
/// estate's once `estateupdateinfo` names it (LLEstateInfoModel::update);
/// a manager is told by RegionHandshake IsEstateManager.
pub fn role(agent: Uuid, owner: Uuid, is_estate_manager: bool) -> Role {
    if !owner.is_nil() && agent == owner {
        Role::Owner
    } else if is_estate_manager {
        Role::Manager
    } else {
        Role::Resident
    }
}

/// What the simulator answered for the region the window shows.
#[derive(Debug, Default)]
pub struct Estate {
    /// The region the window shows, while it is open.
    pub open: Option<RegionHandle>,
    pub region: Option<Arc<RegionDetails>>,
    pub estate: Option<Arc<EstateInfo>>,
    /// None until answered; Some(None) when the capability is absent or
    /// refused (a resident).
    pub access: Option<Option<Arc<EstateAccess>>>,
    pub experiences: Option<Option<RegionExperiences>>,
    pub schedule: Option<Result<Option<RestartSchedule>, ()>>,
    pub environment: Option<Option<EnvSummary>>,
    /// The agent's region's statistics, the last second's.
    pub stats: Option<(RegionHandle, Arc<SimStats>)>,
    /// Server versions by region (AgentMovementComplete).
    pub versions: HashMap<RegionHandle, String>,
    /// The last Top scripts / Top colliders report of the agent's region,
    /// and whether one was asked and not answered yet (the buttons wait,
    /// as disableTopButtons / enableTopButtons).
    pub top: Option<(RegionHandle, Arc<TopObjects>)>,
    pub top_waiting: bool,
    /// A change of the lists was sent: the next `setaccess` asks them again
    /// (LLPanelEstateAccess::setPendingUpdate).
    access_pending: bool,
    out: Vec<NetCommand>,
}

impl Estate {
    /// The window opens on `handle` (LLFloaterRegionInfo::onOpen): what it
    /// knew of another region is dropped, then the panels ask. The covenant
    /// goes through About Land's request (`Land::request_covenant`), sent
    /// by the caller between the panels and RequestRegionInfo.
    pub fn open(&mut self, handle: RegionHandle) {
        if self.open != Some(handle) {
            self.clear();
        }
        self.open = Some(handle);
        self.out.push(NetCommand::Estate(EstateCommand::Refresh { handle }));
    }

    /// A request of another module sent in the window's order (the covenant).
    pub fn push(&mut self, c: NetCommand) {
        self.out.push(c);
    }

    /// requestRegionInfo, after the panels.
    pub fn request_region_info(&mut self, handle: RegionHandle) {
        self.out.push(NetCommand::Estate(EstateCommand::RequestRegionInfo { handle }));
    }

    pub fn close(&mut self) {
        self.open = None;
    }

    /// The General panel's "Apply" (LLPanelRegionGeneralInfo::sendUpdate).
    pub fn set_region_rules(&mut self, handle: RegionHandle, rules: RegionRules) {
        self.out.push(NetCommand::Estate(EstateCommand::SetRegionInfo { handle, rules }));
    }

    /// The Estate panel's "Apply": the model takes the new estate first,
    /// then sends it (callbackChangeLindenEstate → sendEstateInfo), and the
    /// panel shows the model.
    pub fn set_estate(&mut self, handle: RegionHandle, info: EstateInfo) {
        self.estate = Some(Arc::new(info.clone()));
        self.out.push(NetCommand::Estate(EstateCommand::SetEstateInfo { handle, info }));
    }

    /// The Terrain panel's "Apply", with the region's sun as RegionInfo
    /// last told it (LLRegionInfoModel::sendRegionTerrain).
    pub fn set_terrain(&mut self, handle: RegionHandle, terrain: TerrainSettings) {
        let sun = self.region.as_deref().map(RegionSun::of).unwrap_or_default();
        self.out
            .push(NetCommand::Estate(EstateCommand::SetTerrain { handle, terrain, sun }));
    }

    /// "Bake Terrain", once confirmed.
    pub fn bake_terrain(&mut self, handle: RegionHandle) {
        self.out.push(NetCommand::Estate(EstateCommand::BakeTerrain { handle }));
    }

    /// A change of the estate's lists (accessCoreConfirm: `batch`, one
    /// answer; importListCallback: each alone). The lists come again once
    /// the simulator answers.
    pub fn change_access(&mut self, handle: RegionHandle, flags: u32, ids: Vec<Uuid>, batch: bool) {
        if ids.is_empty() {
            return;
        }
        self.access_pending = true;
        self.out
            .push(NetCommand::Estate(EstateCommand::AccessDelta { handle, flags, ids, batch }));
    }

    /// An intervention of the window (message, teleport home, kick,
    /// returned objects, restart, schedule), sent as the panels send it.
    pub fn intervene(&mut self, c: EstateCommand) {
        self.out.push(NetCommand::Estate(c));
    }

    /// "Top scripts" / "Top colliders" (the Debug panel's estate message)
    /// or the window's refresh and filters (LandStatRequest): the list
    /// empties until the report comes.
    pub fn ask_top(&mut self, c: EstateCommand) {
        self.top = None;
        self.top_waiting = true;
        self.out.push(NetCommand::Estate(c));
    }

    fn clear(&mut self) {
        self.region = None;
        self.estate = None;
        self.access = None;
        self.experiences = None;
        self.schedule = None;
        self.environment = None;
        self.access_pending = false;
    }

    pub fn take_commands(&mut self) -> Vec<NetCommand> {
        std::mem::take(&mut self.out)
    }

    /// The window's region's statistics, if they are the last second's.
    pub fn stats_of(&self, handle: RegionHandle) -> Option<&SimStats> {
        self.stats.as_ref().filter(|(h, _)| *h == handle).map(|(_, s)| s.as_ref())
    }

    /// Apply an answer. True when a RegionInfo of the open region came:
    /// the panels are asked again (processRegionInfo → refreshFromRegion).
    pub fn apply(&mut self, ev: EstateEvent) -> bool {
        let shown = |h: RegionHandle| self.open == Some(h);
        match ev {
            EstateEvent::Stats { handle, stats } => self.stats = Some((handle, stats)),
            EstateEvent::SimVersion { handle, version } => {
                self.versions.insert(handle, version);
            }
            EstateEvent::RegionInfo { handle, info } if shown(handle) => {
                self.region = Some(info);
                return true;
            }
            EstateEvent::EstateInfo { handle, info } if shown(handle) => self.estate = Some(info),
            EstateEvent::Access { handle, access } if shown(handle) => self.access = Some(access),
            // LLDispatchSetEstateAccess: only after a change of ours
            EstateEvent::AccessChanged { handle } if shown(handle) && self.access_pending => {
                self.access_pending = false;
                self.out.push(NetCommand::Estate(EstateCommand::RefreshAccess { handle }));
            }
            EstateEvent::Experiences { handle, experiences } if shown(handle) => self.experiences = Some(experiences),
            EstateEvent::Schedule { handle, schedule } if shown(handle) => self.schedule = Some(schedule),
            EstateEvent::TopObjects { handle, report } => {
                self.top = Some((handle, report));
                self.top_waiting = false;
            }
            EstateEvent::Environment { handle, environment } if shown(handle) => {
                self.environment = Some(environment.map(|e| EnvSummary::from_llsd(&e)))
            }
            _ => {}
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_role_follows_the_owner_and_the_manager_flag() {
        let (me, other) = (Uuid::from_u128(1), Uuid::from_u128(2));
        assert_eq!(role(me, me, false), Role::Owner);
        assert_eq!(role(me, me, true), Role::Owner, "the owner is not a mere manager");
        assert_eq!(role(me, other, true), Role::Manager);
        assert_eq!(role(me, other, false), Role::Resident);
        assert_eq!(role(Uuid::nil(), Uuid::nil(), false), Role::Resident, "no owner known yet");
    }

    #[test]
    fn opening_asks_and_region_info_asks_again() {
        let mut e = Estate::default();
        e.open(7);
        e.request_region_info(7);
        assert!(matches!(
            e.take_commands()[..],
            [
                NetCommand::Estate(EstateCommand::Refresh { handle: 7 }),
                NetCommand::Estate(EstateCommand::RequestRegionInfo { handle: 7 }),
            ]
        ));
        let info = Arc::new(RegionDetails::default());
        assert!(e.apply(EstateEvent::RegionInfo {
            handle: 7,
            info: info.clone()
        }));
        assert!(e.region.is_some());
        // another region's answer, or one after closing, is not kept
        assert!(!e.apply(EstateEvent::RegionInfo {
            handle: 8,
            info: info.clone()
        }));
        e.close();
        assert!(!e.apply(EstateEvent::EstateInfo {
            handle: 7,
            info: Arc::new(EstateInfo::default())
        }));
        assert!(e.estate.is_none());
        // reopened on another region, the old answers go
        e.open(8);
        assert!(e.region.is_none());
    }

    /// A change of the lists is sent, and the simulator's `setaccess` asks
    /// them again once; another one (not ours) does not.
    #[test]
    fn a_list_change_asks_the_lists_again_once() {
        use aurora_net::estate::estate_access as ea;
        let mut e = Estate::default();
        e.open(7);
        e.take_commands();
        e.apply(EstateEvent::AccessChanged { handle: 7 });
        assert!(e.take_commands().is_empty(), "no change of ours waiting");
        e.change_access(7, ea::BANNED_AGENT_ADD, Vec::new(), true);
        assert!(e.take_commands().is_empty(), "nobody to change");
        e.change_access(7, ea::BANNED_AGENT_ADD, vec![Uuid::from_u128(3)], true);
        assert!(matches!(
            &e.take_commands()[..],
            [NetCommand::Estate(EstateCommand::AccessDelta { handle: 7, flags, ids, batch: true })]
                if *flags == ea::BANNED_AGENT_ADD && ids.len() == 1
        ));
        e.apply(EstateEvent::AccessChanged { handle: 7 });
        assert!(matches!(
            e.take_commands()[..],
            [NetCommand::Estate(EstateCommand::RefreshAccess { handle: 7 })]
        ));
        e.apply(EstateEvent::AccessChanged { handle: 7 });
        assert!(e.take_commands().is_empty());
    }
}
