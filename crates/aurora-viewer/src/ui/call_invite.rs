//! Adding residents to a conversation (FSFloaterIM::addSessionParticipants,
//! llfloaterimsession.cpp / fsfloaterim.cpp, originally LGPL 2.1): from the
//! call strip's "Add" (the resident picker, several at once) or a resident
//! dragged onto the conversation. A conference invites them; a
//! conversation with one resident asks first (ConfirmAddingChatParticipants)
//! and becomes a new conference.

use super::avatar_picker::AvatarPicker;
use super::widgets::{self, ActionKind, action_button};
use crate::theme::Palette;
use crate::world::World;
use crate::world::calls::AddOutcome;
use egui::RichText;
use uuid::Uuid;

/// A resident dragged from a list (Contacts, People), dropped on a
/// conversation to add them (the DAD_PERSON drop of LLFloaterIMSession).
#[derive(Debug, Clone, Copy)]
pub struct PersonDrag(pub Uuid);

#[derive(Default)]
pub struct CallInviteUi {
    picker: AvatarPicker,
    /// The conversation the picker adds to.
    target: Option<Uuid>,
    /// A conversation with one resident and the residents to add, waiting
    /// for the confirmation.
    confirm: Option<(Uuid, Vec<Uuid>)>,
}

impl CallInviteUi {
    /// "Add" of a conversation: the resident picker, several at once.
    pub fn pick_for(&mut self, conversation: Uuid) {
        self.target = Some(conversation);
        self.picker.open("call_invite_picker", true);
    }

    /// Residents to add to a conversation (picked or dropped).
    pub fn add(&mut self, world: &mut World, conversation: Uuid, residents: Vec<Uuid>) {
        if let AddOutcome::Confirm(agents) = world.invite_to_conversation(conversation, residents) {
            self.confirm = Some((conversation, agents));
        }
    }

    /// The picker and the confirmation. Returns the conversation with one
    /// resident that the confirmed addition closed (its conference took over).
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, world: &mut World) -> Option<Uuid> {
        if let Some(ids) = self.picker.show(ctx, p, world)
            && let Some(conversation) = self.target.take()
        {
            self.add(world, conversation, ids);
        }
        if !self.picker.open {
            self.target = None;
        }
        let (other, agents) = self.confirm.clone()?;
        let mut answer = None;
        let response = widgets::Modal::new(egui::Id::unique("call_invite_confirm")).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.label(
                RichText::new(aurora_i18n::tr!("ui-call-invite-confirm-title"))
                    .size(17.0)
                    .strong()
                    .color(p.ink),
            );
            ui.add_space(6.0);
            ui.label(aurora_i18n::tr!("ui-call-invite-confirm-text"));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let size = egui::Vec2::new(0.0, 26.0);
                if action_button(ui, p, "user-plus", aurora_i18n::tr!("common-ok"), ActionKind::Primary, size).clicked() {
                    answer = Some(true);
                }
                if action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                    answer = Some(false);
                }
            });
        });
        if response.should_close() {
            answer.get_or_insert(false);
        }
        match answer {
            Some(true) => {
                self.confirm = None;
                world.convert_to_conference(other, agents);
                Some(other)
            }
            Some(false) => {
                self.confirm = None;
                None
            }
            None => None,
        }
    }
}
