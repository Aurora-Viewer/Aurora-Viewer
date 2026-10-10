//! Firestorm inventory view menu and finder options, presented with Aurora widgets.
use super::view::{Age, Creator, Filters, InventoryPreferences, Links, SearchField, TYPES};
use super::{InventoryUi, Palette};
use crate::ui::widgets::{Floater, flat_button, switch};
use egui::{RichText, Vec2};

fn check(ui: &mut egui::Ui, p: &Palette, label: &str, value: &mut bool) -> bool {
    ui.horizontal(|ui| {
        let changed = switch(ui, p, value).changed();
        ui.label(RichText::new(label).color(p.ink));
        changed
    })
    .inner
}

pub fn preferences(ui: &mut egui::Ui, p: &Palette, prefs: &mut InventoryPreferences) -> bool {
    let mut changed = false;
    ui.label(RichText::new("Ordre d'affichage").color(p.muted));
    ui.horizontal(|ui| {
        changed |= ui.selectable_value(&mut prefs.sort.by_date, true, "Plus récents d'abord").changed();
        changed |= ui.selectable_value(&mut prefs.sort.by_date, false, "Par nom").changed();
    });
    changed |= check(ui, p, "Dossiers toujours par nom", &mut prefs.sort.folders_by_name);
    changed |= check(ui, p, "Dossiers système en premier", &mut prefs.sort.system_first);
    ui.add_space(6.0);
    ui.label(RichText::new("Affichage").color(p.muted));
    changed |= check(ui, p, "Afficher la bibliothèque", &mut prefs.show_library);
    changed |= check(ui, p, "Onglet Récent", &mut prefs.show_recent);
    changed |= check(ui, p, "Onglet Porté", &mut prefs.show_worn);
    changed |= check(ui, p, "Onglet Favoris", &mut prefs.show_favorites);
    changed |= check(ui, p, "Recherche distincte dans chaque onglet", &mut prefs.separate_searches);
    changed |= check(ui, p, "Double-clic : ajouter les objets", &mut prefs.double_click_add_objects);
    changed |= check(ui, p, "Double-clic : ajouter les vêtements", &mut prefs.double_click_add_clothes);
    ui.add_space(6.0);
    ui.label(RichText::new("Rechercher aussi dans…").color(p.muted));
    changed |= check(ui, p, "La bibliothèque", &mut prefs.search_library);
    changed |= check(ui, p, "La corbeille", &mut prefs.search_trash);
    changed |= check(ui, p, "Les dossiers de tenues", &mut prefs.search_outfits);
    changed
}

pub fn toolbar(ui: &mut egui::Ui, p: &Palette, st: &mut InventoryUi, prefs: &mut InventoryPreferences) {
    if !st.filters_initialized {
        st.filters.clone_from(&prefs.filter_defaults);
        st.filters_initialized = true;
    }
    ui.horizontal(|ui| {
        if flat_button(ui, p, if st.filters.active() { "Filtres •" } else { "Filtres" }).clicked() {
            st.filters_open = !st.filters_open;
        }
        if flat_button(ui, p, "Préférences").clicked() {
            st.preferences_open = !st.preferences_open;
        }
        if st.links_filter.is_some() {
            if flat_button(ui, p, "Quitter les liens").clicked() {
                st.links_filter = None;
                if let Some(filters) = st.saved_filters.take() {
                    st.filters = filters;
                }
            }
        } else if (!st.search.is_empty() || st.filters.active()) && flat_button(ui, p, "Effacer").clicked() {
            st.search.clear();
            st.filters = Filters::default();
            st.links_filter = None;
            st.saved_filters = None;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if flat_button(ui, p, "Développer")
                .on_hover_text("Développer les dossiers affichés")
                .clicked()
            {
                st.expand_all = Some(true);
            }
            if flat_button(ui, p, "Réduire")
                .on_hover_text("Réduire les dossiers affichés")
                .clicked()
            {
                st.expand_all = Some(false);
            }
        });
    });
}

pub fn search(ui: &mut egui::Ui, st: &mut InventoryUi, prefs: &mut InventoryPreferences, hint: &str) {
    ui.horizontal(|ui| {
        let selector_width = 112.0_f32.min(ui.available_width() * 0.4);
        let field_width = (ui.available_width() - selector_width - ui.spacing().item_spacing.x).max(40.0);
        ui.add_sized(
            [field_width, ui.spacing().interact_size.y],
            egui::TextEdit::singleline(&mut st.search).hint_text(hint),
        );
        egui::ComboBox::from_id_salt("inventory_search_field")
            .width(selector_width)
            .truncate()
            .selected_text(match prefs.search_field {
                SearchField::Name => "Nom",
                SearchField::Description => "Description",
                SearchField::Creator => "Créateur",
                SearchField::Uuid => "UUID",
                SearchField::All => "Tous les champs",
            })
            .show_ui(ui, |ui| {
                for (field, label) in [
                    (SearchField::Name, "Nom"),
                    (SearchField::Description, "Description"),
                    (SearchField::Creator, "Créateur"),
                    (SearchField::Uuid, "UUID"),
                    (SearchField::All, "Tous les champs"),
                ] {
                    ui.selectable_value(&mut prefs.search_field, field, label);
                }
            })
            .response
            .on_hover_text("Recherche sans distinction de casse. + combine plusieurs termes ; des guillemets recherchent un mot exact.");
    });
}

pub fn dialogs(ctx: &egui::Context, p: &Palette, st: &mut InventoryUi, prefs: &mut InventoryPreferences) {
    let screen = ctx.content_rect();
    let preferences_size = Vec2::new(410.0, 600.0_f32.min((screen.height() - 90.0).max(180.0)));
    let filters_size = Vec2::new(410.0, 710.0_f32.min((screen.height() - 90.0).max(180.0)));
    Floater::new(
        &format!("inventory_preferences_{}", st.ui_id),
        "Préférences de l'inventaire",
        screen.center() - preferences_size / 2.0,
        preferences_size,
    )
    .show(ctx, p, &mut st.preferences_open, |ui| {
        let height = ui.available_height().min(ctx.content_rect().height() - 90.0).max(180.0);
        egui::ScrollArea::vertical()
            .max_height(height)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                preferences(ui, p, prefs);
                ui.add_space(6.0);
                ui.label(
                    RichText::new("Préférences conservées entre les sessions.")
                        .size(12.0)
                        .color(p.muted),
                );
            });
    });
    Floater::new(
        &format!("inventory_filters_{}", st.ui_id),
        "Filtres de l'inventaire",
        screen.center() - filters_size / 2.0,
        filters_size,
    )
    .show(ctx, p, &mut st.filters_open, |ui| {
        let height = ui.available_height().min(ctx.content_rect().height() - 90.0).max(180.0);
        egui::ScrollArea::vertical()
            .max_height(height)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                let f = &mut st.filters;
                ui.label(RichText::new("Types d'éléments").color(p.muted));
                ui.horizontal(|ui| {
                    if flat_button(ui, p, "Tous").clicked() {
                        f.types = u32::MAX;
                    }
                    if flat_button(ui, p, "Aucun").clicked() {
                        f.types = 0;
                    }
                });
                egui::Grid::new("inventory_types").num_columns(2).show(ui, |ui| {
                    for (n, (kind, label)) in TYPES.iter().enumerate() {
                        let bit = 1_u32 << kind;
                        let mut on = f.types & bit != 0;
                        if check(ui, p, label, &mut on) {
                            if on {
                                f.types |= bit;
                            } else {
                                f.types &= !bit;
                            }
                            // Selecting every known type restores unknown types too.
                            if TYPES.iter().all(|(kind, _)| f.types & (1_u32 << kind) != 0) {
                                f.types = u32::MAX;
                            }
                        }
                        if n % 2 == 1 {
                            ui.end_row();
                        }
                    }
                });
                ui.add_space(5.0);
                ui.label(RichText::new("Permissions requises").color(p.muted));
                ui.horizontal(|ui| {
                    for (bit, label) in [
                        (super::rules::MODIFY, "Modifier"),
                        (super::rules::COPY, "Copier"),
                        (super::rules::TRANSFER, "Transférer"),
                    ] {
                        let mut on = f.permissions & bit != 0;
                        if check(ui, p, label, &mut on) {
                            if on {
                                f.permissions |= bit;
                            } else {
                                f.permissions &= !bit;
                            }
                        }
                    }
                });
                egui::ComboBox::from_id_salt("inventory_links")
                    .selected_text(match f.links {
                        Links::Include => "Inclure les liens",
                        Links::Only => "Liens uniquement",
                        Links::Exclude => "Masquer les liens",
                    })
                    .show_ui(ui, |ui| {
                        for (v, label) in [
                            (Links::Include, "Inclure les liens"),
                            (Links::Only, "Liens uniquement"),
                            (Links::Exclude, "Masquer les liens"),
                        ] {
                            ui.selectable_value(&mut f.links, v, label);
                        }
                    });
                egui::ComboBox::from_id_salt("inventory_creator")
                    .selected_text(match f.creator {
                        Creator::All => "Tous les créateurs",
                        Creator::Me => "Créés par moi",
                        Creator::Others => "Créés par d'autres",
                    })
                    .show_ui(ui, |ui| {
                        for (v, label) in [
                            (Creator::All, "Tous les créateurs"),
                            (Creator::Me, "Créés par moi"),
                            (Creator::Others, "Créés par d'autres"),
                        ] {
                            ui.selectable_value(&mut f.creator, v, label);
                        }
                    });
                check(ui, p, "Afficher aussi les dossiers sans résultat", &mut f.always_folders);
                check(ui, p, "Objets regroupés uniquement", &mut f.coalesced);
                ui.add_space(5.0);
                ui.label(RichText::new("Date de création").color(p.muted));
                egui::ComboBox::from_id_salt("inventory_age")
                    .selected_text(match f.age {
                        Age::Any => "Toutes les dates",
                        Age::SinceLogout => "Depuis la dernière déconnexion",
                        Age::Newer => "Plus récent que…",
                        Age::Older => "Plus ancien que…",
                    })
                    .show_ui(ui, |ui| {
                        for (v, label) in [
                            (Age::Any, "Toutes les dates"),
                            (Age::SinceLogout, "Depuis la dernière déconnexion"),
                            (Age::Newer, "Plus récent que…"),
                            (Age::Older, "Plus ancien que…"),
                        ] {
                            ui.selectable_value(&mut f.age, v, label);
                        }
                    });
                if matches!(f.age, Age::Newer | Age::Older) {
                    let (mut days, mut hours) = (f.hours / 24, f.hours % 24);
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut days).range(0..=36500).suffix(" jours"));
                        ui.add(egui::DragValue::new(&mut hours).range(0..=23).suffix(" heures"));
                    });
                    f.hours = days * 24 + hours;
                }
                ui.add_space(7.0);
                ui.horizontal(|ui| {
                    if flat_button(ui, p, "Réinitialiser").clicked() {
                        *f = Filters::default();
                    }
                    if flat_button(ui, p, "Garder par défaut").clicked() {
                        prefs.filter_defaults.clone_from(f);
                    }
                });
                ui.label(
                    RichText::new("Récent : depuis la dernière déconnexion ; 24 h au premier lancement.")
                        .size(12.0)
                        .color(p.muted),
                );
            });
    });
}
