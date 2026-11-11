//! Advanced › Request / Leave Admin Status (Ctrl+Alt+G): the request to the
//! region (handle_god_mode / handle_leave_god_mode, indra/newview/
//! llviewermenu.cpp, originally LGPL 2.1). The region answers with the god
//! level it grants (GrantGodlikePowers, `World::set_god_level`), or not at
//! all when it refuses.
use super::*;
use crate::ui::bars::AdminAction;
use crate::ui::widgets::{self, ActionKind};
use aurora_net::god::{GodCommand, KICK_DEFAULT, KICK_FREEZE};
use uuid::Uuid;

/// A resident's Admin › Kick / Freeze / Unfreeze, asked with the message
/// they will read (the KickUser, FreezeUser and UnFreezeUser notifications).
pub(super) struct KickAsk {
    target: Uuid,
    name: String,
    flags: u32,
    message: String,
}

/// Title, question and default message of each (notifications.xml).
fn kick_texts(flags: u32, name: &str) -> (&'static str, String, &'static str) {
    match flags {
        KICK_DEFAULT => (
            aurora_i18n::tr!("admin-kick"),
            aurora_i18n::tr!("admin-kick-question", name = name),
            aurora_i18n::tr!("admin-kick-message"),
        ),
        KICK_FREEZE => (
            aurora_i18n::tr!("admin-freeze"),
            aurora_i18n::tr!("admin-freeze-question", name = name),
            aurora_i18n::tr!("admin-freeze-message"),
        ),
        _ => (
            aurora_i18n::tr!("admin-unfreeze"),
            aurora_i18n::tr!("admin-unfreeze-question", name = name),
            aurora_i18n::tr!("admin-unfreeze-message"),
        ),
    }
}

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
                    god.push(GodCommand::Request {
                        handle,
                        method: "objectinfo".into(),
                        params: vec!["assetids".into()],
                        objects: local_ids,
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
                        objects: Vec::new(),
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

    /// LLAvatarActions::kick / freeze / unfreeze: the message asked first.
    pub(super) fn ask_god_kick(&mut self, target: Uuid, flags: u32) {
        if self.world.god_level == 0 {
            log::warn!("admin: kick {target} without admin status");
            return;
        }
        let name = self
            .world
            .social
            .avatar_names
            .complete(&target)
            .unwrap_or_else(|| self.world.social.name_of(&target));
        let message = kick_texts(flags, &name).2.to_owned();
        self.god_kick_ask = Some(KickAsk {
            target,
            name,
            flags,
            message,
        });
    }

    /// Each frame: the message box, then GodKickUser to our region, which
    /// finds the resident on the grid (GodsModule.KickUser).
    pub(super) fn god_kick_ui(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        use egui::RichText;
        let Some(ask) = self.god_kick_ask.as_mut() else {
            return;
        };
        let (title, question, _) = kick_texts(ask.flags, &ask.name);
        let (icon, kind) = match ask.flags {
            KICK_DEFAULT => ("logout", ActionKind::Danger),
            KICK_FREEZE => ("snowflake", ActionKind::Danger),
            _ => ("sun-filled", ActionKind::Primary),
        };
        let mut answer = None;
        let modal = widgets::Modal::new(egui::Id::unique("admin_kick_message")).show(ctx, |ui| {
            ui.set_width(400.0);
            ui.label(RichText::new(title).size(15.0).strong().color(p.ink));
            ui.add_space(6.0);
            ui.label(RichText::new(question).size(12.5).color(p.muted));
            ui.add_space(8.0);
            ui.add(
                egui::TextEdit::multiline(&mut ask.message)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY),
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let size = egui::Vec2::new(0.0, 26.0);
                if widgets::action_button(ui, p, icon, title, kind, size).clicked() {
                    answer = Some(true);
                }
                if widgets::action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                    answer = Some(false);
                }
            });
        });
        if modal.should_close() && answer.is_none() {
            answer = Some(false);
        }
        let Some(send) = answer else { return };
        let Some(ask) = self.god_kick_ask.take() else { return };
        let Some(handle) = self.world.main_region.filter(|_| send) else {
            return;
        };
        self.send(NetCommand::God(GodCommand::KickUser {
            handle,
            agent_id: ask.target,
            flags: ask.flags,
            reason: ask.message,
        }));
    }

    /// AURORA_DEMO_GOD=1: admin status asked at frame 400, the crown in the
    /// navigation bar and the notice in the chat.
    pub(super) fn demo_admin_status(&mut self) {
        if self.demo && self.frame_count == 400 && std::env::var_os("AURORA_DEMO_GOD").is_some() {
            self.request_admin_status(true);
        }
    }
}
