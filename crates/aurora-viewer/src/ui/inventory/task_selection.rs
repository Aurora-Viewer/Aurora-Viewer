//! Shared object-inventory selection (LLFolderView, indra/newview/llfolderview.cpp,
//! originally LGPL 2.1): Ctrl toggles, Shift ranges, right click keeps the batch.
use egui::Id;
use std::collections::HashSet;
use uuid::Uuid;

#[derive(Clone, Default)]
pub struct Selection {
    pub object: Option<Uuid>,
    pub items: HashSet<Uuid>,
    pub order: Vec<Uuid>,
    anchor: Option<Uuid>,
    pub layer: Option<egui::LayerId>,
}

impl Selection {
    pub fn sync(&mut self, object: Uuid, order: Vec<Uuid>, layer: egui::LayerId) {
        if self.object != Some(object) {
            *self = Self::default();
        }
        self.object = Some(object);
        if self.order != order {
            let visible: HashSet<_> = order.iter().copied().collect();
            self.items.retain(|id| visible.contains(id));
            self.anchor = self.anchor.filter(|id| visible.contains(id));
            self.order = order;
        }
        self.layer = Some(layer);
    }

    pub fn select(&mut self, ui: &egui::Ui, object: Uuid, item: Uuid, secondary: bool) {
        self.select_with_focus(ui, object, item, secondary, focus_id(object));
    }

    pub fn select_with_focus(&mut self, ui: &egui::Ui, object: Uuid, item: Uuid, secondary: bool, focus: Id) {
        if self.object != Some(object) {
            *self = Self {
                object: Some(object),
                ..Default::default()
            };
        }
        ui.memory_mut(|m| m.request_focus(focus));
        if secondary && self.items.contains(&item) {
            return;
        }
        let mods = ui.input(|i| i.modifiers);
        if mods.shift && !secondary {
            if let Some((a, b)) = self
                .anchor
                .and_then(|id| self.order.iter().position(|row| *row == id))
                .zip(self.order.iter().position(|row| *row == item))
            {
                if !(mods.command || mods.ctrl) {
                    self.items.clear();
                }
                self.items.extend(self.order[a.min(b)..=a.max(b)].iter().copied());
            } else {
                self.items.insert(item);
            }
        } else if (mods.command || mods.ctrl) && !secondary {
            if !self.items.remove(&item) {
                self.items.insert(item);
            }
            self.anchor = Some(item);
        } else {
            self.items.clear();
            self.items.insert(item);
            self.anchor = Some(item);
        }
    }
}

pub(crate) fn focus_id(object: Uuid) -> Id {
    Id::unique(("task_contents_focus", object))
}
