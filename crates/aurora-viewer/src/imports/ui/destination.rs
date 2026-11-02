//! Searchable inventory hierarchy, sharing inventory names, ordering and row widgets.

use super::*;
use crate::ui::inventory::{self as inventory_ui, tree, view};
use crate::world::inventory::{Inventory, actions as rules};

#[cfg(test)]
mod tests;

struct Row {
    id: Uuid,
    parent: Option<usize>,
    depth: usize,
    name: String,
    path: String,
    search: String,
    branch: bool,
}

#[derive(Default)]
pub(in crate::imports) struct Picker {
    order: Option<Arc<view::PickerOrder>>,
    protected: HashSet<Uuid>,
    rows: Vec<Row>,
    expanded: HashSet<Uuid>,
    query: String,
    filter: Option<String>,
    visible: Vec<usize>,
    opened: bool,
    demo_opened: bool,
}

impl Picker {
    pub(super) fn demo(&mut self, ui: &egui::Ui) {
        if !self.demo_opened {
            self.demo_opened = true;
            egui::Popup::open_id(ui.ctx(), ui.make_persistent_id(egui::IdSalt::new("import_folder")).with("popup"));
        }
    }

    fn refresh(&mut self, ctx: &egui::Context, inventory: &Inventory, protected: &HashSet<Uuid>) {
        let order = view::picker_order(ctx, inventory);
        if self.order.as_ref().is_some_and(|old| Arc::ptr_eq(old, &order)) && self.protected == *protected {
            return;
        }
        let root_changed = self.rows.first().is_none_or(|row| row.id != inventory.root);
        let excluded: HashSet<_> = [14, 46, 48, 50, 53]
            .into_iter()
            .filter_map(|kind| rules::system(inventory, kind))
            .chain(protected.iter().copied())
            .collect();
        self.rows.clear();
        let mut seen = HashSet::new();
        let mut stack = vec![(inventory.root, None, 0)];
        while let Some((id, parent, depth)) = stack.pop() {
            let Some(folder) = inventory.folders.get(&id) else { continue };
            if !seen.insert(id) || folder.library || excluded.contains(&id) {
                continue;
            }
            let name = inventory_ui::folder_name(inventory, id);
            let path = parent.map_or_else(|| name.clone(), |index: usize| format!("{} / {name}", self.rows[index].path));
            let index = self.rows.len();
            self.rows.push(Row {
                id,
                parent,
                depth,
                search: name.to_lowercase(),
                name,
                path,
                branch: false,
            });
            // The shared comparator orders siblings; actual parent IDs determine membership.
            for &child in order.children(id).iter().rev() {
                if inventory.folders.get(&child).is_some_and(|folder| folder.info.parent == id) {
                    stack.push((child, Some(index), depth + 1));
                }
            }
        }
        for index in 0..self.rows.len() {
            self.rows[index].branch = self.rows.get(index + 1).is_some_and(|next| next.depth > self.rows[index].depth);
        }
        self.expanded.retain(|id| seen.contains(id));
        if root_changed {
            self.expanded.insert(inventory.root);
        }
        self.order = Some(order);
        self.protected.clone_from(protected);
        self.filter = None;
    }

    fn reveal(&mut self, selected: Uuid) {
        let mut current = self
            .rows
            .iter()
            .position(|row| row.id == selected)
            .and_then(|index| self.rows[index].parent);
        while let Some(index) = current {
            self.expanded.insert(self.rows[index].id);
            current = self.rows[index].parent;
        }
        self.filter = None;
    }

    fn filter(&mut self) {
        let query = self.query.trim().to_lowercase();
        if self.filter.as_ref() == Some(&query) {
            return;
        }
        // Propagate matches upward once, so search retains the hierarchy without
        // repeatedly walking every matching folder's ancestry.
        let mut included: Vec<_> = self.rows.iter().map(|row| row.search.contains(&query)).collect();
        for index in (0..self.rows.len()).rev() {
            if included[index]
                && let Some(parent) = self.rows[index].parent
            {
                included[parent] = true;
            }
        }
        self.visible.clear();
        let mut closed_depth = usize::MAX;
        for (index, row) in self.rows.iter().enumerate() {
            if row.depth > closed_depth || !included[index] {
                continue;
            }
            self.visible.push(index);
            closed_depth = if query.is_empty() && !self.expanded.contains(&row.id) {
                row.depth
            } else {
                usize::MAX
            };
        }
        self.filter = Some(query);
    }

    pub(super) fn show(&mut self, ui: &mut egui::Ui, p: &Palette, inventory: &Inventory, protected: &HashSet<Uuid>, selected: &mut Uuid) {
        let current = if inventory.folders.contains_key(selected) {
            inventory_ui::folder_name(inventory, *selected)
        } else {
            aurora_i18n::tr!("import-no-folder").to_owned()
        };
        let response = egui::ComboBox::from_id_salt("import_folder")
            .selected_text(current)
            .width(ui.available_width().clamp(160.0, 360.0))
            .height(320.0)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show_ui(ui, |ui| {
                self.refresh(ui.ctx(), inventory, protected);
                let opening = !self.opened;
                if opening {
                    self.reveal(*selected);
                }
                ui.horizontal(|ui| {
                    let search = widgets::search_field(ui, &mut self.query, aurora_i18n::tr!("inventory-move-filter"), 310.0);
                    if opening {
                        search.request_focus();
                    }
                    if widgets::icon_button(ui, p, "x", aurora_i18n::tr!("common-clear"), !self.query.is_empty()) {
                        self.query.clear();
                        search.request_focus();
                    }
                });
                self.filter();
                let searching = !self.query.trim().is_empty();
                let mut toggle = None;
                tree::frame(ui, p, |ui| {
                    tree::spacing(ui);
                    let width = ui.available_width();
                    let row_height = ui.spacing().interact_size.y;
                    let mut scroll = tree::scroll(ui, 240.0);
                    if opening {
                        let index = self.visible.iter().position(|&index| self.rows[index].id == *selected).unwrap_or(0);
                        scroll = scroll.vertical_scroll_offset((index as f32 * row_height - 100.0).max(0.0));
                    }
                    scroll.show_rows(ui, row_height, self.visible.len(), |ui, range| {
                        for &index in &self.visible[range] {
                            let row = &self.rows[index];
                            ui.push_id(row.id, |ui| {
                                ui.horizontal(|ui| {
                                    ui.set_max_width(width);
                                    ui.add_space(row.depth as f32 * 16.0);
                                    let open = searching || self.expanded.contains(&row.id);
                                    if row.branch {
                                        let caret = ui.add_enabled_ui(!searching, |ui| tree::caret(ui, p, open)).inner;
                                        #[cfg(test)]
                                        ui.ctx().data_mut(|data| {
                                            data.insert_temp(egui::Id::unique(("import_destination_caret", row.id)), caret.rect)
                                        });
                                        if caret.clicked() {
                                            toggle = Some(row.id);
                                        }
                                    } else {
                                        ui.add_space(12.0);
                                    }
                                    widgets::glyph(
                                        ui,
                                        if row.branch && open {
                                            "folder-open-filled"
                                        } else {
                                            "folder-filled"
                                        },
                                        p.violet_light,
                                        16.0,
                                    );
                                    let label = tree::label(ui, tree::name(&row.name, p), *selected == row.id, egui::Sense::click(), None);
                                    #[cfg(test)]
                                    ui.ctx().data_mut(|data| {
                                        data.insert_temp(egui::Id::unique(("import_destination_row", row.id)), label.rect)
                                    });
                                    if label.on_hover_text(&row.path).clicked() {
                                        *selected = row.id;
                                        ui.close();
                                    }
                                });
                            });
                        }
                    });
                });
                if let Some(id) = toggle {
                    if !self.expanded.remove(&id) {
                        self.expanded.insert(id);
                    }
                    self.filter = None;
                    ui.ctx().request_repaint();
                }
                if self.visible.is_empty() {
                    ui.label(RichText::new(aurora_i18n::tr!("inventory-move-no-folders")).color(p.muted));
                }
            });
        self.opened = response.inner.is_some();
    }
}
