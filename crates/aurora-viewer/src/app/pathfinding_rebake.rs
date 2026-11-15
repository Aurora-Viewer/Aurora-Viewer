//! The navmesh rebake in the app: the network's answers, Build › Pathfinding
//! › Rebake region, the two pathfinding parcel icons and their clicks.
//! LLMenuOptionPathfindingRebakeNavmesh and the pathfinding parts of
//! LLStatusBar / LLLocationInputCtrl (onParcelIconClick, rebakeRegionCallback),
//! indra/newview, originally LGPL 2.1; the rules are
//! `ui::pathfinding::rebake`.
use super::*;
use crate::ui::parcel_icons::{self, Click, ParcelIcon};
use crate::world::notifications::{Data, Kind as NoticeKind};

impl App {
    /// The region's navmesh status, the agent's right to rebake and the
    /// rebake's answer; other events go on.
    pub(super) fn on_rebake_event(&mut self, ev: NetEvent) -> Option<NetEvent> {
        let now = Instant::now();
        let rebake = &mut self.pathfinding.rebake;
        rebake.region_changed(self.world.main_region);
        match ev {
            NetEvent::NavMeshStatus { region, status } => rebake.on_status(region, status, now),
            NetEvent::PathfindingAgentState { region, can_rebake } => rebake.on_agent_state(region, can_rebake),
            NetEvent::NavMeshRebake { region, ok } => {
                if rebake.on_response(region, ok, now) {
                    self.world.notifications.push(
                        NoticeKind::Alert,
                        aurora_i18n::tr!("pathfinding-menu-rebake"),
                        aurora_i18n::tr!("pathfinding-cannot-rebake"),
                        Data::None,
                    );
                }
            }
            other => return Some(other),
        }
        None
    }

    /// Tools.DoPathfindingRebakeRegion (sendRequestRebakeNavmesh).
    pub(super) fn rebake_navmesh(&mut self) {
        let Some(region) = self.pathfinding.rebake.request(Instant::now()) else {
            return;
        };
        if self.demo {
            self.demo_rebake_answer(region);
        } else {
            self.send(NetCommand::RebakeNavMesh { region });
        }
    }

    /// onParcelIconClick for the pathfinding icons.
    pub(super) fn parcel_icon_click(&mut self, click: Click) {
        match click {
            Click::AboutLand => self.panels.about_land = !self.panels.about_land,
            Click::PathfindingDirty => self.pathfinding.dirty_question = true,
            Click::PathfindingDisabled => {
                self.world.notifications.push(
                    NoticeKind::Info,
                    aurora_i18n::tr!("ui-parcel-icons-name-no-pathfinding"),
                    aurora_i18n::tr!("pathfinding-dynamic-disabled"),
                    Data::None,
                );
            }
        }
    }

    /// The pathfinding icons of the bar, after the parcel's abilities.
    pub(super) fn pathfinding_parcel_icons(&self) -> Vec<ParcelIcon> {
        let now = Instant::now();
        let dynamic = self.world.main().is_some_and(|r| r.dynamic_pathfinding);
        let pathfinding_region = self.world.main_has_cap("NavMeshGenerationStatus") || self.pathfinding_demo.region();
        let rebake = &self.pathfinding.rebake;
        parcel_icons::pathfinding_icons(
            rebake.dirty_icon(dynamic, now),
            rebake.disabled_icon(dynamic, pathfinding_region, now),
        )
    }

    /// Once a frame: follow the agent's region, and the PathfindingDirty
    /// question (rebakeRegionCallback: Rebake rebakes if one is needed).
    pub(super) fn pathfinding_rebake_frame(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        self.pathfinding.rebake.region_changed(self.world.main_region);
        if self.demo {
            self.demo_rebake_frame();
        }
        if self.pathfinding.dirty_dialog(ctx, p) == Some(true) && self.pathfinding.rebake.needed() {
            self.rebake_navmesh();
        }
    }
}
