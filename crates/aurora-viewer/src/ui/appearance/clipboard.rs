//! Ctrl+C, Ctrl+X and Ctrl+V in Appearance, on the inventory's clipboard.
//! The Legacy Viewer has them only in its inventory panels (LLFolderView::
//! copy/cut/paste, indra/llui/llfolderview.cpp, originally LGPL 2.1), not
//! in its outfit lists; here they act on what the outfit lists select, and
//! a paste follows the inventory's outfit rules (`world::outfit_paste`):
//! what is copied in one window pastes in the other.

use super::{AppearanceUi, WORN_SCOPE, model};
use crate::world::inventory::Inventory;
use uuid::Uuid;

/// What the app does with the inventory's clipboard after the window drew.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    /// The inventory ids to put on the clipboard, cut or copied.
    Store { ids: Vec<Uuid>, cut: bool },
    /// A paste, with the text the system clipboard held: it pastes only
    /// while that is still the inventory clipboard's marker.
    Paste { to: PasteTo, text: String },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PasteTo {
    /// The Current Outfit: the items are worn.
    Worn,
    /// The selected outfit, or My Outfits when none is. The app pastes
    /// folders beside it, in its group, and items into it.
    Outfit(Option<Uuid>),
}

fn focus_id() -> egui::Id {
    egui::Id::unique("appearance_clipboard_focus")
}

/// A click on a row or an outfit gives the lists the keyboard.
pub(super) fn request_focus(ctx: &egui::Context) {
    ctx.memory_mut(|m| m.request_focus(focus_id()));
}

/// Keeps the focus target alive under the list (egui drops a focus no
/// widget claims in a frame), registered before the rows so their clicks
/// take precedence; a click elsewhere, Tab or Escape leaves it.
pub(super) fn list_focus(ui: &mut egui::Ui, rect: egui::Rect) {
    let response = ui.interact(rect, focus_id(), egui::Sense::click());
    let leaving = ui.input(|i| {
        i.pointer.any_pressed() && i.pointer.interact_pos().is_none_or(|pos| !rect.contains(pos))
            || i.key_pressed(egui::Key::Tab)
            || i.key_pressed(egui::Key::Escape)
    });
    if leaving && response.has_focus() {
        ui.memory_mut(|m| m.surrender_focus(focus_id()));
    } else if response.clicked() {
        request_focus(ui.ctx());
    }
}

fn has_focus(ui: &egui::Ui) -> bool {
    ui.is_enabled()
        && ui.memory(|m| m.has_focus(focus_id()) && m.allows_interaction(ui.layer_id()))
        && !ui.ctx().text_edit_focused()
        && !egui::Popup::is_any_open(ui.ctx())
}

/// The link (or the item itself) by which `folder` holds each of `targets`:
/// a copy from an outfit or the Current Outfit carries links, as the same
/// rows copied in the inventory would, and a cut moves the outfit's link,
/// never the original. A worn item with no link comes as itself.
fn holders(inv: &Inventory, folder: Option<Uuid>, targets: &[Uuid]) -> Vec<Uuid> {
    let items = folder
        .and_then(|f| inv.folders.get(&f))
        .map(|f| f.items.as_slice())
        .unwrap_or_default();
    targets
        .iter()
        .map(|target| {
            items
                .iter()
                .rev()
                .copied()
                .find(|id| {
                    *id == *target
                        || inv
                            .items
                            .get(id)
                            .is_some_and(|it| matches!(it.asset_type, 24 | 25) && it.asset_id == *target)
                })
                .unwrap_or(*target)
        })
        .collect()
}

impl AppearanceUi {
    /// The rows Ctrl+C or Ctrl+X take in the view shown: the selected items
    /// of an outfit or of Worn, else the selected outfits.
    fn clipboard_ids(&self, inv: &Inventory, outfits: &[Uuid]) -> Vec<Uuid> {
        let worn_view = self.editing || self.tab == 2;
        if self.editing {
            let cof = model::cof(inv);
            return holders(inv, cof, &self.selected_item.into_iter().collect::<Vec<_>>());
        }
        if let Some(scope) = self.item_scope.filter(|scope| {
            if worn_view {
                *scope == WORN_SCOPE
            } else {
                self.tab == 1 && *scope != WORN_SCOPE && self.outfit_selection.focus == Some(*scope)
            }
        }) {
            let items = self.item_selection.all();
            if !items.is_empty() {
                let folder = if scope == WORN_SCOPE { model::cof(inv) } else { Some(scope) };
                return holders(inv, folder, &items);
            }
        }
        if worn_view {
            return Vec::new();
        }
        self.outfit_selection.ordered(outfits)
    }

    fn paste_target(&self) -> PasteTo {
        if self.editing || self.tab == 2 {
            PasteTo::Worn
        } else {
            PasteTo::Outfit(self.outfit_selection.focus)
        }
    }

    /// Reads the clipboard keys while the lists have the keyboard. Run
    /// after the lists, whose clicks give them the keyboard first.
    pub(super) fn clipboard_shortcuts(&mut self, ui: &mut egui::Ui, inv: &Inventory, outfits: &[Uuid]) {
        if self.adding || self.pending.is_some() || !has_focus(ui) {
            return;
        }
        // egui-winit turns Ctrl+C, Ctrl+X and Ctrl+V into these events and
        // consumes the key presses (as in the inventory's shortcuts).
        let mut events = Vec::new();
        ui.input_mut(|input| {
            input.events.retain(|event| {
                let ours = matches!(event, egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_));
                if ours {
                    events.push(event.clone());
                }
                !ours
            });
        });
        for event in events {
            self.clipboard = match event {
                egui::Event::Copy | egui::Event::Cut => {
                    let ids = self.clipboard_ids(inv, outfits);
                    if ids.is_empty() {
                        continue;
                    }
                    Some(Request::Store {
                        ids,
                        cut: matches!(event, egui::Event::Cut),
                    })
                }
                egui::Event::Paste(text) => Some(Request::Paste {
                    to: self.paste_target(),
                    text,
                }),
                _ => continue,
            };
        }
    }

    /// The clipboard request of the last frame, for the app.
    pub fn take_clipboard(&mut self) -> Option<Request> {
        self.clipboard.take()
    }
}

/// The frames of `AURORA_DEMO_APPEARANCE=clipboard`: Ctrl+C on the two
/// items selected in the second outfit, then Ctrl+V into the empty one.
const DEMO_COPY_FRAME: u64 = 500;
const DEMO_PASTE_FRAME: u64 = 540;

impl AppearanceUi {
    /// `AURORA_DEMO_APPEARANCE=clipboard`: the copy and the paste as
    /// egui-winit delivers them, `marker` being what the copy left on the
    /// system clipboard.
    pub fn demo_clipboard_events(&mut self, ctx: &egui::Context, frame: u64, marker: Option<Uuid>) -> Vec<egui::Event> {
        static MODE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if !*MODE.get_or_init(|| std::env::var("AURORA_DEMO_APPEARANCE").is_ok_and(|mode| mode == "clipboard")) {
            return Vec::new();
        }
        match frame {
            DEMO_COPY_FRAME => {
                request_focus(ctx);
                vec![egui::Event::Copy]
            }
            DEMO_PASTE_FRAME => {
                self.open_outfit(Uuid::from_u128(702));
                request_focus(ctx);
                marker.map(|id| vec![egui::Event::Paste(id.to_string())]).unwrap_or_default()
            }
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::inventory::InvItem;

    #[test]
    fn holders_carry_the_folders_link_or_the_item_itself() {
        let id = Uuid::from_u128;
        let mut inv = Inventory::default();
        inv.created_folders(vec![aurora_net::inventory::InvFolder {
            id: id(10),
            parent: id(1),
            ..Default::default()
        }]);
        for (n, asset_type, asset) in [(20, 24, 30), (21, 5, 0)] {
            inv.items.insert(
                id(n),
                InvItem {
                    id: id(n),
                    parent: id(10),
                    asset_type,
                    asset_id: id(asset),
                    ..Default::default()
                },
            );
            inv.folders.get_mut(&id(10)).expect("folder").items.push(id(n));
        }
        assert_eq!(holders(&inv, Some(id(10)), &[id(30), id(21), id(40)]), vec![id(20), id(21), id(40)]);
        assert_eq!(holders(&inv, None, &[id(30)]), vec![id(30)]);
    }
}
