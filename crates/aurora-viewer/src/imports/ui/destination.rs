//! Searchable import destination picker; cached until inventory or protection changes.

use super::*;
use crate::world::inventory::{Inventory, actions as rules};

#[derive(Default)]
pub(in crate::imports) struct Picker {
    generation: Option<u64>,
    protected: HashSet<Uuid>,
    rows: Vec<(Uuid, String, String)>,
    query: String,
    filter: Option<String>,
    visible: Vec<usize>,
    opened: bool,
}

impl Picker {
    pub(super) fn demo(&mut self, ui: &egui::Ui) {
        self.query = "Textures".into();
        egui::Popup::open_id(ui.ctx(), ui.make_persistent_id(egui::IdSalt::new("import_folder")).with("popup"));
    }

    fn refresh(&mut self, inventory: &Inventory, protected: &HashSet<Uuid>) {
        if self.generation == Some(inventory.generation) && self.protected == *protected {
            return;
        }
        // Resolve system roots once instead of scanning every folder for each candidate.
        let excluded: Vec<_> = [14, 46, 48, 50, 53]
            .into_iter()
            .filter_map(|kind| rules::system(inventory, kind))
            .chain(protected.iter().copied())
            .collect();
        self.rows = inventory
            .folders
            .values()
            .filter(|folder| {
                !rules::library(inventory, folder.info.id) && !excluded.iter().any(|root| rules::under(inventory, folder.info.id, *root))
            })
            .map(|folder| (folder.info.id, folder.info.name.clone(), folder.info.name.to_lowercase()))
            .collect();
        self.rows.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
        self.generation = Some(inventory.generation);
        self.protected.clone_from(protected);
        self.filter = None;
    }

    fn filter(&mut self) {
        let query = self.query.trim().to_lowercase();
        if self.filter.as_ref() != Some(&query) {
            self.visible = self
                .rows
                .iter()
                .enumerate()
                .filter_map(|(index, row)| row.2.contains(&query).then_some(index))
                .collect();
            self.filter = Some(query);
        }
    }

    pub(super) fn show(&mut self, ui: &mut egui::Ui, p: &Palette, inventory: &Inventory, protected: &HashSet<Uuid>, selected: &mut Uuid) {
        let current = inventory
            .folders
            .get(selected)
            .map(|folder| folder.info.name.as_str())
            .unwrap_or(aurora_i18n::tr!("import-no-folder"));
        let response = egui::ComboBox::from_id_salt("import_folder")
            .selected_text(current)
            .width(ui.available_width().clamp(160.0, 360.0))
            .height(300.0)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show_ui(ui, |ui| {
                self.refresh(inventory, protected);
                ui.horizontal(|ui| {
                    let search = widgets::search_field(ui, &mut self.query, aurora_i18n::tr!("inventory-move-filter"), 310.0);
                    if !self.opened {
                        search.request_focus();
                    }
                    if widgets::icon_button(ui, p, "x", aurora_i18n::tr!("common-clear"), !self.query.is_empty()) {
                        self.query.clear();
                        search.request_focus();
                    }
                });
                widgets::divider(ui, p);
                self.filter();
                egui::ScrollArea::vertical()
                    .id_salt("import_folder_results")
                    .max_height(220.0)
                    .show_rows(ui, ui.spacing().interact_size.y, self.visible.len(), |ui, range| {
                        for index in range {
                            let (id, name, _) = &self.rows[self.visible[index]];
                            ui.push_id(id, |ui| {
                                if selection(ui, selected, *id, name).clicked() {
                                    ui.close();
                                }
                            });
                        }
                    });
                if self.visible.is_empty() {
                    ui.label(RichText::new(aurora_i18n::tr!("inventory-move-no-folders")).color(p.muted));
                }
            });
        self.opened = response.inner.is_some();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::inventory::{FetchState, Folder};
    use aurora_net::inventory::InvFolder;

    fn fixture() -> Inventory {
        let mut inventory = Inventory {
            root: Uuid::from_u128(1),
            ..Default::default()
        };
        for (id, parent, name, kind, library) in [
            (1, 0, "Inventory", 8, false),
            (2, 1, "Textures", 0, false),
            (3, 1, "Booth assets", -1, false),
            (4, 1, "Trash", 14, false),
            (5, 4, "Discarded", -1, false),
            (6, 1, "Protected", -1, false),
            (7, 6, "Child", -1, false),
            (8, 0, "Library", 8, true),
        ] {
            let id = Uuid::from_u128(id);
            inventory.folders.insert(
                id,
                Folder {
                    info: InvFolder {
                        id,
                        parent: Uuid::from_u128(parent),
                        name: name.into(),
                        type_default: kind,
                        ..Default::default()
                    },
                    children: Vec::new(),
                    items: Vec::new(),
                    state: FetchState::Fetched,
                    library,
                },
            );
        }
        inventory
    }

    #[test]
    fn cached_destinations_keep_rules_and_refresh_for_inventory_and_protection() {
        let mut inventory = fixture();
        let mut picker = Picker::default();
        for protected in [HashSet::new(), HashSet::from([Uuid::from_u128(6)])] {
            picker.refresh(&inventory, &protected);
            let expected: HashSet<_> = inventory
                .folders
                .keys()
                .copied()
                .filter(|id| rules::destination(&inventory, *id, &protected))
                .collect();
            assert_eq!(picker.rows.iter().map(|row| row.0).collect::<HashSet<_>>(), expected);
        }
        picker.query = "  BOOTH  ".into();
        picker.filter();
        assert_eq!(picker.visible.len(), 1);
        assert_eq!(picker.rows[picker.visible[0]].0, Uuid::from_u128(3));
        inventory.folders.get_mut(&Uuid::from_u128(3)).unwrap().info.name = "Renamed".into();
        inventory.generation += 1;
        picker.refresh(&inventory, &HashSet::new());
        picker.filter();
        assert!(picker.visible.is_empty());
    }

    #[test]
    fn typing_in_popup_filters_without_closing_or_changing_destination() {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let mut picker = Picker::default();
        let inventory = fixture();
        let mut selected = Uuid::from_u128(2);
        let mut result_position = None;
        for step in 0..4 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(600.0, 500.0))),
                    events: if step == 2 {
                        vec![egui::Event::Text("Booth".into())]
                    } else {
                        Vec::new()
                    },
                    ..Default::default()
                },
                |ui| {
                    if step == 0 {
                        egui::Popup::open_id(ui.ctx(), ui.make_persistent_id(egui::IdSalt::new("import_folder")).with("popup"));
                    }
                    picker.show(ui, &theme.palette(), &inventory, &HashSet::new(), &mut selected);
                },
            );
            output.textures_delta.clear();
            result_position = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::epaint::Shape::Text(text) if text.galley.text() == "Booth assets" => Some(text.pos + text.galley.size() * 0.5),
                    _ => None,
                })
                .or(result_position);
            drop(output);
        }
        assert!(picker.opened);
        assert_eq!(picker.query, "Booth");
        assert_eq!(selected, Uuid::from_u128(2));
        assert_eq!(picker.visible.len(), 1);
        let position = result_position.expect("filtered result visible");
        for pressed in [Some(true), Some(false), None] {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(600.0, 500.0))),
                    events: pressed.map_or_else(Vec::new, |pressed| {
                        vec![
                            egui::Event::PointerMoved(position),
                            egui::Event::PointerButton {
                                pos: position,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ]
                    }),
                    ..Default::default()
                },
                |ui| {
                    picker.show(ui, &theme.palette(), &inventory, &HashSet::new(), &mut selected);
                },
            );
            output.textures_delta.clear();
            drop(output);
        }
        assert_eq!(selected, Uuid::from_u128(3));
        assert!(!picker.opened, "choosing a result closes the list");
    }
}
