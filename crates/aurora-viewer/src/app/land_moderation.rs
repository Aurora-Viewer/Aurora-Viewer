//! "Freeze" and "Eject" of a resident's menus, and the estate's "Kick",
//! "Teleport home" and "Ban": the rights read each frame
//! (world/land_moderation.rs), the parcel asked for when they stand on
//! group land, the question asked first and the message sent to the region
//! they stand in. Port of LLAvatarActions::landFreeze / landEject /
//! estateKick / estateTeleportHome / estateBan and their callbacks
//! (indra/newview/llavataractions.cpp, originally LGPL 2.1), with their
//! FreezeAvatarFullname, EjectAvatarFullname, EjectAvatarFullnameNoBan,
//! EstateKickUser, EstateTeleportHomeUser and EstateBanUser questions
//! (notifications.xml).
use super::*;
use crate::ui::widgets::ActionKind;
use crate::world::land_moderation::Rights;
use aurora_net::estate::{EstateCommand, estate_access as ea};
use aurora_net::land::LandCommand;
use uuid::Uuid;

/// An estate action of a resident's menus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EstateAct {
    /// `kickestate`: sent home, out of the whole estate.
    Kick,
    /// `teleporthomeuser`: sent home.
    TeleportHome,
    /// `estateaccessdelta`: banned from this estate, or from all ours.
    Ban,
}

/// The ban's flags (callbackEstateBan): banned and taken off the allowed
/// list, here only or on every estate we own (all) or manage for this
/// estate's owner.
pub(super) fn ban_flags(all: bool, owner: bool) -> u32 {
    let flags = ea::BANNED_AGENT_ADD | ea::ALLOWED_AGENT_REMOVE;
    match (all, owner) {
        (false, _) => flags,
        (true, true) => flags | ea::APPLY_TO_ALL_ESTATES,
        (true, false) => flags | ea::APPLY_TO_MANAGED_ESTATES,
    }
}

/// The question waiting for its answer.
pub(super) enum Ask {
    Freeze {
        target: Uuid,
        name: String,
    },
    /// `ban`: "Eject and ban" is offered.
    Eject {
        target: Uuid,
        name: String,
        ban: bool,
    },
    /// An estate's kick, teleport home or ban.
    Estate {
        act: EstateAct,
        target: Uuid,
        name: String,
        /// "All estates" covers every estate of the region's owner
        /// (APPLY_TO_ALL_ESTATES): we own the region, or a god.
        owner: bool,
        /// A god's label names that owner ("All estates owned by [OWNER]").
        owner_name: Option<String>,
    },
}

impl App {
    /// Avatar.EnableFreezeEject for `id`, this frame: the rule every menu
    /// reads (World::resident_rights).
    pub(super) fn moderation_rights(&self, id: Uuid) -> Rights {
        self.world.resident_rights(id)
    }

    /// A menu opened on `id` (the world's, a list's or the minimap's): the
    /// group land under them is asked for when the viewer does not know it
    /// (selectParcelAt), and the entries appear once it comes.
    pub(super) fn moderation_menu_opened(&mut self, id: Uuid) {
        if !self.world.resident_needs_parcel(id) {
            return;
        }
        let Some((handle, local)) = self.world.resident_place(id) else {
            return;
        };
        let origin = aurora_net::handle_to_origin(handle);
        if let Some(cmd) = self.hover_tips.parcels.request(handle, local, origin, Instant::now()) {
            self.send(cmd);
        }
    }

    /// estateKick / estateTeleportHome / estateBan: asked first, while we
    /// own the resident's region or manage its estate.
    pub(super) fn ask_estate(&mut self, target: Uuid, act: EstateAct) {
        if !self.world.resident_rights(target).estate {
            log::warn!("estate moderation: not allowed to {act:?} {target}");
            return;
        }
        let Some((owner, owner_name)) = self.estate_scope(target) else {
            return;
        };
        let name = self.resident_full_name(target);
        self.moderation_ask = Some(Ask::Estate {
            act,
            target,
            name,
            owner,
            owner_name,
        });
    }

    /// What the ban's "All estates" covers for `target`'s region, as
    /// LLAvatarActions::estateBanMultiple / callbackEstateBan decide it: a
    /// god, or the region's owner, bans from every estate of that owner
    /// (APPLY_TO_ALL_ESTATES), a god's label naming the owner
    /// (RegionInfoAllEstatesOwnedBy); an estate manager from the estates
    /// they manage (APPLY_TO_MANAGED_ESTATES).
    fn estate_scope(&self, target: Uuid) -> Option<(bool, Option<String>)> {
        let (handle, _) = self.world.resident_place(target)?;
        let info = self.world.regions.get(&handle)?.info.as_ref()?;
        let god = self.world.god_level > 0;
        let ours = info.owner == self.world.agent_id;
        Some((god || ours, god.then(|| self.resident_full_name(info.owner))))
    }

    /// The complete name, anonymous under RLVa's shownames.
    fn resident_full_name(&self, target: Uuid) -> String {
        self.world
            .social
            .avatar_names
            .complete(&target)
            .unwrap_or_else(|| self.world.social.name_of(&target))
    }

    /// landFreeze / landEject: asked first, while the rights hold.
    pub(super) fn ask_moderation(&mut self, target: Uuid, eject: bool) {
        let rights = self.moderation_rights(target);
        if !rights.allowed {
            log::warn!("land moderation: not allowed to freeze or eject {target}");
            return;
        }
        let name = self.resident_full_name(target);
        self.moderation_ask = Some(if eject {
            Ask::Eject {
                target,
                name,
                ban: rights.ban,
            }
        } else {
            Ask::Freeze { target, name }
        });
    }

    /// Each frame: the question, then the message to the resident's region
    /// (where they stand when it is answered).
    pub(super) fn land_moderation_ui(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        let Some(ask) = self.moderation_ask.as_ref() else {
            return;
        };
        let answer = match ask {
            Ask::Freeze { name, .. } => {
                let question = aurora_i18n::tr!("ui-context-freeze-question", name = name.as_str());
                ui::context::choose(
                    ctx,
                    p,
                    "land_freeze_confirm",
                    aurora_i18n::tr!("ui-context-freeze"),
                    &[question.as_str(), aurora_i18n::tr!("ui-context-freeze-detail")],
                    &[
                        ("snowflake", aurora_i18n::tr!("ui-context-freeze"), ActionKind::Primary),
                        ("sun-filled", aurora_i18n::tr!("ui-context-unfreeze"), ActionKind::Flat),
                    ],
                )
            }
            Ask::Eject { name, ban, .. } => {
                let question = aurora_i18n::tr!("ui-context-eject-question", name = name.as_str());
                let eject = ("player-eject-filled", aurora_i18n::tr!("ui-context-eject"), ActionKind::Danger);
                let eject_and_ban = ("ban", aurora_i18n::tr!("ui-context-eject-and-ban"), ActionKind::Danger);
                let choices = if *ban { vec![eject, eject_and_ban] } else { vec![eject] };
                ui::context::choose(
                    ctx,
                    p,
                    "land_eject_confirm",
                    aurora_i18n::tr!("ui-context-eject"),
                    &[question.as_str()],
                    &choices,
                )
            }
            Ask::Estate {
                act,
                name,
                owner,
                owner_name,
                ..
            } => estate_question(ctx, p, *act, name, *owner, owner_name.as_deref()),
        };
        let Some(choice) = answer else {
            return;
        };
        let Some(ask) = self.moderation_ask.take() else {
            return;
        };
        let Some(choice) = choice else {
            return;
        };
        // the second button: "Unfreeze" (Flags 0x1), "Eject and ban" (0x1),
        // the ban's "All estates"
        let second = choice == 1;
        let (Ask::Freeze { target, .. } | Ask::Eject { target, .. } | Ask::Estate { target, .. }) = ask;
        let Some((handle, _)) = self.world.resident_place(target) else {
            log::warn!("land moderation: {target} is no longer in a region we are connected to");
            return;
        };
        if let Ask::Estate { act, owner, .. } = ask {
            // the question was for the estate they stood in: one they moved
            // to since is checked again before anything is sent
            if !self.world.resident_rights(target).estate {
                log::warn!("estate moderation: {target} moved where we may not {act:?} them");
                return;
            }
            if act == EstateAct::Ban && second && self.estate_scope(target).map(|(all, _)| all) != Some(owner) {
                log::warn!("estate moderation: {target} changed estate, the ban on all estates is not sent");
                return;
            }
            self.send_estate_act(handle, target, act, second, owner);
            return;
        }
        let cmd = match ask {
            Ask::Freeze { .. } => LandCommand::FreezeUser {
                handle,
                target,
                unfreeze: second,
            },
            Ask::Eject { .. } => LandCommand::EjectUser {
                handle,
                target,
                ban: second,
            },
            Ask::Estate { .. } => return,
        };
        log::info!("land moderation: {cmd:?}");
        self.send(NetCommand::Land(cmd));
    }

    /// callbackEstateKick / callbackEstateTeleportHome / callbackEstateBan,
    /// to the region the resident stands in. The estate's owner is never
    /// banned (OwnerCanNotBeDenied).
    fn send_estate_act(&mut self, handle: aurora_net::RegionHandle, target: Uuid, act: EstateAct, all: bool, owner: bool) {
        let cmd = match act {
            EstateAct::Kick => EstateCommand::KickFromEstate { handle, target },
            EstateAct::TeleportHome => EstateCommand::TeleportHome {
                handle,
                target: Some(target),
            },
            EstateAct::Ban => {
                let estate_owner = self.world.regions.get(&handle).and_then(|r| r.info.as_ref()).map(|i| i.owner);
                if estate_owner == Some(target) {
                    log::warn!("estate moderation: the estate's owner cannot be banned");
                    return;
                }
                EstateCommand::AccessDelta {
                    handle,
                    flags: ban_flags(all, owner),
                    ids: vec![target],
                    batch: false,
                }
            }
        };
        log::info!("estate moderation: {cmd:?}");
        self.world.estate.intervene(cmd);
    }

    /// AURORA_DEMO_LAND_MODERATION=menu|none|freeze|eject|eject-ban: the
    /// wolf stands on our land (the overlay cell under it becomes ours),
    /// then at frame 600 its world menu, or the question of "Freeze" or
    /// "Eject" (`eject-ban` with "Eject and ban", as on a parcel whose ban
    /// list is ours). `none` leaves the land someone else's: the menu
    /// without the two entries. `estate-menu`, `estate-kick`, `estate-home`
    /// and `estate-ban`: we manage the region's estate instead (the land
    /// stays someone else's), then the menu or the estate's question.
    pub(super) fn demo_land_moderation(&mut self) {
        if !self.demo || self.frame_count != 600 {
            return;
        }
        let Ok(mode) = std::env::var("AURORA_DEMO_LAND_MODERATION") else {
            return;
        };
        let wolf = crate::demo::DEMO_LOUP;
        let estate = mode.starts_with("estate-");
        if estate
            && let Some((handle, _)) = self.world.resident_place(wolf)
            && let Some(info) = self.world.regions.get_mut(&handle).and_then(|r| r.info.as_mut())
        {
            std::sync::Arc::make_mut(info).is_estate_manager = true;
        }
        if mode != "none"
            && !estate
            && let Some((handle, local)) = self.world.resident_place(wolf)
            && let Some((cx, _, cells)) = self.world.map.overlays.get_mut(&handle)
        {
            let i = (local.y / 4.0) as usize * *cx as usize + (local.x / 4.0) as usize;
            if let Some(cell) = cells.get_mut(i) {
                *cell = (*cell & !0x07) | 3;
            }
        }
        match mode.as_str() {
            "menu" | "none" | "estate-menu" => self.demo_menu_of(wolf),
            "estate-kick" => self.ask_estate(wolf, EstateAct::Kick),
            "estate-home" => self.ask_estate(wolf, EstateAct::TeleportHome),
            "estate-ban" => self.ask_estate(wolf, EstateAct::Ban),
            "freeze" => self.ask_moderation(wolf, false),
            "eject" | "eject-ban" => {
                self.ask_moderation(wolf, true);
                if let Some(Ask::Eject { ban, .. }) = self.moderation_ask.as_mut() {
                    *ban = mode == "eject-ban";
                }
            }
            _ => log::warn!("AURORA_DEMO_LAND_MODERATION: unknown mode {mode}"),
        }
    }
}

/// EstateKickUser / EstateTeleportHomeUser (OK, Cancel) and EstateBanUser
/// (This estate, All estates, Cancel).
fn estate_question(
    ctx: &egui::Context,
    p: &crate::theme::Palette,
    act: EstateAct,
    name: &str,
    owner: bool,
    owner_name: Option<&str>,
) -> Option<Option<usize>> {
    match act {
        EstateAct::Kick => {
            let question = aurora_i18n::tr!("ui-context-estate-kick-question", name = name);
            ui::context::choose(
                ctx,
                p,
                "estate_kick_confirm",
                aurora_i18n::tr!("ui-context-estate-kick"),
                &[question.as_str(), aurora_i18n::tr!("ui-context-estate-kick-detail")],
                &[("door-exit", aurora_i18n::tr!("ui-context-estate-kick"), ActionKind::Danger)],
            )
        }
        EstateAct::TeleportHome => {
            let question = aurora_i18n::tr!("ui-context-estate-teleport-home-question", name = name);
            ui::context::choose(
                ctx,
                p,
                "estate_home_confirm",
                aurora_i18n::tr!("ui-context-estate-teleport-home"),
                &[question.as_str(), aurora_i18n::tr!("ui-context-estate-teleport-home-detail")],
                &[(
                    "home-filled",
                    aurora_i18n::tr!("ui-context-estate-teleport-home"),
                    ActionKind::Primary,
                )],
            )
        }
        EstateAct::Ban => {
            let question = aurora_i18n::tr!("ui-context-estate-ban-question", name = name);
            let all: String = if let Some(owner_name) = owner_name {
                aurora_i18n::tr!("ui-context-estate-ban-all-owned-by", owner = owner_name).to_string()
            } else if owner {
                aurora_i18n::tr!("ui-context-estate-ban-all-owned").to_string()
            } else {
                aurora_i18n::tr!("ui-context-estate-ban-all-managed").to_string()
            };
            ui::context::choose(
                ctx,
                p,
                "estate_ban_confirm",
                aurora_i18n::tr!("ui-context-estate-ban"),
                &[question.as_str(), aurora_i18n::tr!("ui-context-estate-ban-detail")],
                &[
                    ("ban", aurora_i18n::tr!("ui-context-estate-ban-this"), ActionKind::Danger),
                    ("ban", all.as_str(), ActionKind::Danger),
                ],
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ban_takes_the_resident_off_the_allowed_list_here_or_on_all_our_estates() {
        let here = ea::BANNED_AGENT_ADD | ea::ALLOWED_AGENT_REMOVE;
        assert_eq!(ban_flags(false, true), here);
        assert_eq!(ban_flags(false, false), here);
        assert_eq!(
            ban_flags(true, true),
            here | ea::APPLY_TO_ALL_ESTATES,
            "the owner: every estate it owns"
        );
        assert_eq!(
            ban_flags(true, false),
            here | ea::APPLY_TO_MANAGED_ESTATES,
            "a manager: the estates it manages for this owner"
        );
    }
}
