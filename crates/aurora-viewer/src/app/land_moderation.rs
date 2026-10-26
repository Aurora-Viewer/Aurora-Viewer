//! "Freeze" and "Eject" of a resident's menus: the rights read each frame
//! (world/land_moderation.rs), the parcel asked for when they stand on
//! group land, the question asked first and the message sent to the region
//! they stand in. Port of LLAvatarActions::landFreeze / landEject and
//! callbackLandFreeze / callbackLandEject (indra/newview/llavataractions.cpp,
//! originally LGPL 2.1), with their FreezeAvatarFullname,
//! EjectAvatarFullname and EjectAvatarFullnameNoBan questions
//! (notifications.xml).
use super::*;
use crate::ui::widgets::ActionKind;
use crate::world::land_moderation::{self, Rights};
use aurora_net::land::LandCommand;
use uuid::Uuid;

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
}

impl App {
    /// Avatar.EnableFreezeEject for `id`, this frame.
    pub(super) fn moderation_rights(&self, id: Uuid) -> Rights {
        let w = &self.world;
        let Some((handle, local)) = w.resident_place(id) else {
            return Rights::default();
        };
        let hovered = self
            .hover_tips
            .parcels
            .at(handle, local, aurora_net::handle_to_origin(handle))
            .map(|p| p.as_ref());
        w.moderation_place(handle, local, hovered)
            .map_or_else(Rights::default, |place| land_moderation::rights(&place, &w.moderation_agent()))
    }

    /// A menu opened on `id`: the group land under them is asked for when
    /// the viewer does not know it (selectParcelAt), and the entries appear
    /// once it comes.
    pub(super) fn moderation_menu_opened(&mut self, id: Uuid) {
        let Some((handle, local)) = self.world.resident_place(id) else {
            return;
        };
        let origin = aurora_net::handle_to_origin(handle);
        let hovered = self.hover_tips.parcels.at(handle, local, origin).map(|p| p.as_ref());
        let unknown = self
            .world
            .moderation_place(handle, local, hovered)
            .is_some_and(|place| land_moderation::needs_parcel(&place));
        if unknown && let Some(cmd) = self.hover_tips.parcels.request(handle, local, origin, Instant::now()) {
            self.send(cmd);
        }
    }

    /// landFreeze / landEject: asked first, while the rights hold.
    pub(super) fn ask_moderation(&mut self, target: Uuid, eject: bool) {
        let rights = self.moderation_rights(target);
        if !rights.allowed {
            log::warn!("land moderation: not allowed to freeze or eject {target}");
            return;
        }
        // the complete name, anonymous under RLVa's shownames
        let name = self
            .world
            .social
            .avatar_names
            .complete(&target)
            .unwrap_or_else(|| self.world.social.name_of(&target));
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
        // the second button: "Unfreeze" (Flags 0x1), "Eject and ban" (0x1)
        let second = choice == 1;
        let (Ask::Freeze { target, .. } | Ask::Eject { target, .. }) = ask;
        let Some((handle, _)) = self.world.resident_place(target) else {
            log::warn!("land moderation: {target} is no longer in a region we are connected to");
            return;
        };
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
        };
        log::info!("land moderation: {cmd:?}");
        self.send(NetCommand::Land(cmd));
    }

    /// AURORA_DEMO_LAND_MODERATION=menu|none|freeze|eject|eject-ban: the
    /// wolf stands on our land (the overlay cell under it becomes ours),
    /// then at frame 600 its world menu, or the question of "Freeze" or
    /// "Eject" (`eject-ban` with "Eject and ban", as on a parcel whose ban
    /// list is ours). `none` leaves the land someone else's: the menu
    /// without the two entries.
    pub(super) fn demo_land_moderation(&mut self) {
        if !self.demo || self.frame_count != 600 {
            return;
        }
        let Ok(mode) = std::env::var("AURORA_DEMO_LAND_MODERATION") else {
            return;
        };
        let wolf = crate::demo::DEMO_LOUP;
        if mode != "none"
            && let Some((handle, local)) = self.world.resident_place(wolf)
            && let Some((cx, _, cells)) = self.world.map.overlays.get_mut(&handle)
        {
            let i = (local.y / 4.0) as usize * *cx as usize + (local.x / 4.0) as usize;
            if let Some(cell) = cells.get_mut(i) {
                *cell = (*cell & !0x07) | 3;
            }
        }
        match mode.as_str() {
            "menu" | "none" => self.demo_menu_of(wolf),
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
