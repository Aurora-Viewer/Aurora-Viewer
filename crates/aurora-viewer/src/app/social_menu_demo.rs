//! AURORA_DEMO_SOCIAL_MENU: the steps of "Invite to group", "Add to contact
//! set" and "Block particles" on the demo's resident (demo_social_menu.rs).
use super::*;
use crate::demo::social_menu::{RESIDENT, Step};
use crate::ui::context::CtxAction;

impl App {
    pub(super) fn demo_social_menu_steps(&mut self) {
        if !self.demo {
            return;
        }
        let Some(step) = crate::demo::social_menu::step() else {
            return;
        };
        match (self.frame_count, step) {
            (600, Step::Particles) => {
                self.settings.particle_filters.set(RESIDENT, true, false);
            }
            (600, Step::Menu) | (720, Step::Particles) => self.demo_menu_of(RESIDENT),
            (600, Step::Picker | Step::Invite) => self.on_ctx_action(CtxAction::InviteToGroup(RESIDENT)),
            (640, Step::Invite) => self.group_profile_ui.invite.demo_choose(crate::demo::DEMO_GROUP1),
            (600, Step::NewSet | Step::NewSetDone) => {
                // the new set is not written to the demo account's file
                self.world.contact_sets.keep_in_memory();
                self.on_ctx_action(CtxAction::NewContactSet(vec![RESIDENT]));
            }
            (640, Step::NewSetDone) => {
                let name = aurora_i18n::tr!("demo-social-menu-set");
                self.contacts_ui.demo_answer_new_set(&mut self.world, name);
                self.panels.people = true;
                self.panels.people_tab = ui::people::TAB_SETS;
            }
            (700, Step::Particles) => {
                self.on_ctx_action(CtxAction::ToggleOwnerParticleFilter(RESIDENT));
                log::info!(
                    "demo social menu: particles of {RESIDENT} blocked: {}",
                    self.settings.particle_filters.blocked(RESIDENT, true)
                );
            }
            (600, Step::Card) => self.on_ctx_action(CtxAction::GiveCallingCard(RESIDENT)),
            (600, Step::CardOffer) => {
                self.world.apply(crate::demo::social_menu::card_offer());
            }
            _ => {}
        }
    }

    /// The world menu of a resident, from their name plate.
    pub(super) fn demo_menu_of(&mut self, id: uuid::Uuid) {
        let ppp = self.egui_ctx.pixels_per_point();
        match self.name_tags.iter().find(|t| t.id == id).map(|t| t.rect.center()) {
            Some(at) => {
                self.cursor_pos = (at.x * ppp, at.y * ppp);
                self.open_context_menu();
            }
            None => log::warn!("demo social menu: no name plate of {id} on screen"),
        }
    }
}
