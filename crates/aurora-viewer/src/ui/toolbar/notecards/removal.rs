//! LLItemBridge::removeItem (indra/newview/llinventorybridge.cpp) moves an
//! ordinary item to Trash. Use the shared move/removal checks, not asset rights.
use super::library;
use crate::{
    theme::Palette,
    ui::widgets,
    world::inventory::{Inventory, actions},
};
use aurora_net::{NetCommand, inventory::operations::Reply};
use std::collections::HashSet;
use uuid::Uuid;

struct Pending {
    request: Uuid,
    item: Uuid,
    owner: Uuid,
    epoch: u64,
}

#[derive(Default)]
pub(super) struct State {
    queued: Option<(Uuid, Uuid)>,
    pending: Option<Pending>,
    confirmation: Option<String>,
    approved: bool,
    removed: Option<(Uuid, Uuid)>,
}

fn unavailable() -> String {
    aurora_i18n::tr!("toolbar-interface-delete-unavailable").into()
}

impl State {
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn active(&self) -> bool {
        self.queued.is_some() || self.busy()
    }

    pub fn queue(&mut self, owner: Uuid, item: Uuid) {
        if !self.active() {
            self.queued = Some((owner, item));
        }
    }

    pub fn confirming(&self) -> bool {
        self.confirmation.is_some()
    }

    pub fn approve(&mut self) {
        self.approved = true;
        self.confirmation = None;
    }

    pub fn show(&mut self, ctx: &egui::Context, p: &Palette) {
        let Some(name) = self.confirmation.clone() else { return };
        let mut cancel = false;
        let mut approve = false;
        let modal = widgets::Modal::new(egui::Id::unique("toolbar_interface_delete_confirm")).show(ctx, |ui| {
            ui.set_width(400.0);
            ui.label(aurora_i18n::tr!("toolbar-interface-delete-confirm", name = name.as_str()));
            ui.horizontal(|ui| {
                let button = ui.button(aurora_i18n::tr!("common-cancel"));
                #[cfg(test)]
                ctx.data_mut(|data| data.insert_temp(egui::Id::unique("toolbar_interface_delete_cancel"), button.rect));
                cancel = button.clicked();
                let button = widgets::action_button(
                    ui,
                    p,
                    "trash-filled",
                    aurora_i18n::tr!("common-delete"),
                    widgets::ActionKind::Danger,
                    egui::vec2(130.0, 30.0),
                );
                #[cfg(test)]
                ctx.data_mut(|data| data.insert_temp(egui::Id::unique("toolbar_interface_delete_confirm"), button.rect));
                approve = button.clicked();
            });
        });
        if approve {
            self.approve();
        } else if cancel || modal.should_close() {
            self.queued = None;
            self.confirmation = None;
            self.approved = false;
        }
    }

    pub fn command(
        &mut self,
        inv: &mut Inventory,
        protected: &HashSet<Uuid>,
        epoch: u64,
        confirm: bool,
    ) -> Result<Option<NetCommand>, String> {
        let result = self.prepare(inv, protected, epoch, confirm);
        if result.is_err() {
            self.queued = None;
            self.confirmation = None;
            self.approved = false;
        }
        result
    }

    fn prepare(&mut self, inv: &mut Inventory, protected: &HashSet<Uuid>, epoch: u64, confirm: bool) -> Result<Option<NetCommand>, String> {
        let Some((owner, item)) = self.queued else { return Ok(None) };
        if self.pending.is_some() || inv.owner != owner {
            return Err(unavailable());
        }
        let library::Path::Ready(parent) = library::resolve(inv)? else {
            return Err(unavailable());
        };
        let notecard = inv
            .items
            .get(&item)
            .filter(|item| item.asset_type == 7 && item.parent == parent)
            .ok_or_else(unavailable)?;
        let name = notecard.name.clone();
        let trash = actions::system(inv, 14).ok_or_else(unavailable)?;
        // Only a verified notecard reaches here; it cannot be worn. Preserve
        // the user's protected inventory entries and all shared removal rules.
        let change = actions::move_selection(inv, &[item], trash, protected, &HashSet::new())?;
        if confirm && !self.approved {
            self.confirmation = Some(name);
            return Ok(None);
        }
        self.queued = None;
        self.confirmation = None;
        self.approved = false;
        let request = Uuid::new_v4();
        self.pending = Some(Pending {
            request,
            item,
            owner,
            epoch,
        });
        Ok(Some(NetCommand::EditInventory { request, change }))
    }

    pub fn finished(&mut self, request: Uuid, result: &Result<Reply, String>, epoch: u64) -> bool {
        if !self.pending.as_ref().is_some_and(|pending| pending.request == request) {
            return false;
        }
        let Some(pending) = self.pending.take() else { return false };
        if result.is_ok() && pending.epoch == epoch {
            self.removed = Some((pending.owner, pending.item));
        }
        true
    }

    pub fn take_removed(&mut self) -> Option<(Uuid, Uuid)> {
        self.removed.take()
    }
}
