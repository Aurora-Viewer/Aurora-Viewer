//! Advanced › Request / Leave Admin Status (Ctrl+Alt+G): the request to the
//! region (handle_god_mode / handle_leave_god_mode, indra/newview/
//! llviewermenu.cpp, originally LGPL 2.1). The region answers with the god
//! level it grants (GrantGodlikePowers, `World::set_god_level`), or not at
//! all when it refuses.
use super::*;
use crate::ui::bars::AdminAction;
use aurora_net::god::GodCommand;

/// The level the demo grants: OpenSim's for a region owner or a grid god
/// (GodController.cs, 200 and more is a god).
const DEMO_GOD_LEVEL: u8 = 200;

impl App {
    pub(super) fn request_admin_status(&mut self, godlike: bool) {
        log::info!("admin status: {} asked", if godlike { "god mode" } else { "leaving god mode" });
        if self.demo {
            // no region to answer: the demo grants it at once
            self.world.set_god_level(if godlike { DEMO_GOD_LEVEL } else { 0 });
            return;
        }
        self.send(NetCommand::RequestGodlikePowers { godlike });
    }

    /// An Admin entry, while the region keeps our god level (each request
    /// is checked by the region again).
    pub(super) fn admin_action(&mut self, action: AdminAction) {
        if self.world.god_level == 0 {
            log::warn!("admin: {action:?} without admin status");
            return;
        }
        log::info!("admin: {action:?}");
        let me = self.world.agent_id;
        let selection = self.build.admin_selection(&self.world);
        let mut god = Vec::new();
        match action {
            AdminAction::TakeCopy => self.build.force_take_copy(&self.world),
            AdminAction::Lock => self.build.lock_selection(&self.world),
            AdminAction::ForceOwnerToMe => {
                for (handle, local_ids) in selection {
                    god.push(GodCommand::ForceOwner {
                        handle,
                        local_ids,
                        owner_id: me,
                        group_id: self.world.groups.active,
                    });
                }
            }
            AdminAction::ForceOwnerPermissive => {
                for (handle, local_ids) in selection {
                    god.push(GodCommand::Permissive { handle, local_ids });
                }
            }
            AdminAction::Delete => {
                for (handle, local_ids) in selection {
                    god.push(GodCommand::ForceDelete { handle, local_ids });
                }
                self.build.deselect_all(&self.world);
            }
            // handle_object_asset_ids: the region answers in chat
            AdminAction::AssetIds => {
                for (handle, local_ids) in selection {
                    let mut params = vec!["assetids".to_owned()];
                    params.extend(local_ids.iter().map(u32::to_string));
                    god.push(GodCommand::Request {
                        handle,
                        method: "objectinfo".into(),
                        params,
                    });
                }
            }
            // the parcel chosen in About Land or by the land tool, else ours
            AdminAction::ParcelForceOwnerToMe => {
                let chosen = self
                    .world
                    .land
                    .sel
                    .as_ref()
                    .and_then(|s| s.parcel.as_ref().map(|p| (s.handle, p.local_id)));
                let ours = self.world.parcel.as_ref().zip(self.world.main_region).map(|(p, h)| (h, p.local_id));
                if let Some((handle, local_id)) = chosen.or(ours) {
                    god.push(GodCommand::ParcelForceOwner {
                        handle,
                        local_id,
                        owner_id: me,
                    });
                }
            }
            // handle_claim_public_land: the land selection, in our region
            AdminAction::ClaimPublicLand => match self.world.land.sel.as_ref() {
                Some(sel) if Some(sel.handle) == self.world.main_region => {
                    let (west, south, east, north) = sel.rect();
                    god.push(GodCommand::Request {
                        handle: sel.handle,
                        method: "claimpublicland".into(),
                        params: [west, south, east, north].iter().map(|v| format!("{v:.6}")).collect(),
                    });
                }
                _ => self.world.system_message(aurora_i18n::tr!("admin-claim-public-land-select")),
            },
            AdminAction::SaveRegionState => {
                if let Some(handle) = self.world.main_region {
                    god.push(GodCommand::SaveState { handle });
                }
            }
        }
        for c in god {
            self.send(NetCommand::God(c));
        }
    }

    /// AURORA_DEMO_GOD=1: admin status asked at frame 400, the crown in the
    /// navigation bar and the notice in the chat.
    pub(super) fn demo_admin_status(&mut self) {
        if self.demo && self.frame_count == 400 && std::env::var_os("AURORA_DEMO_GOD").is_some() {
            self.request_admin_status(true);
        }
    }
}
