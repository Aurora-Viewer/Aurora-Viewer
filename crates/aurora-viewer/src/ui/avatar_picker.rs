//! "Choisissez un Résident": pick residents like LLFloaterAvatarPicker
//! (newview/llfloateravatarpicker.cpp, floater_avatar_picker.xml, originally
//! LGPL 2.1): search by name (AvatarPickerSearch), friends, near me
//! (NearMeRange 162 m) and Firestorm's search by UUID.

use super::widgets::{self, Floater, flat_button};
use crate::theme::Palette;
use crate::world::World;
use egui::{RichText, Vec2};
use uuid::Uuid;

/// NearMeRange (settings.xml).
const NEAR_ME_RANGE: f32 = 162.0;
/// Height of the result list (the window is 400 px high, like
/// floater_avatar_picker.xml).
const LIST_H: f32 = 250.0;

// tab 0: search by name
const TAB_FRIENDS: usize = 1;
const TAB_NEAR: usize = 2;
const TAB_UUID: usize = 3;

#[derive(Default)]
pub struct AvatarPicker {
    pub open: bool,
    /// Window id (one picker per floater that uses it).
    id: String,
    multiple: bool,
    tab: usize,
    query: String,
    uuid_text: String,
    selected: Vec<Uuid>,
}

impl AvatarPicker {
    pub fn open(&mut self, id: &str, multiple: bool) {
        self.open = true;
        self.id = id.to_owned();
        self.multiple = multiple;
        self.selected.clear();
    }

    /// The residents chosen with "Sélectionner" (the picker closes).
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, world: &mut World) -> Option<Vec<Uuid>> {
        if !self.open {
            return None;
        }
        let mut chosen = None;
        let mut open = true;
        let screen = ctx.content_rect();
        let size = Vec2::new(380.0, 400.0);
        Floater::new(&self.id, "Choisissez un Résident", screen.center() - size * 0.5, size)
            .fixed()
            .show(ctx, p, &mut open, |ui| {
                let before = self.tab;
                widgets::tabs(
                    ui,
                    p,
                    &mut self.tab,
                    &[("Recherche", true), ("Amis", true), ("À proximité", true), ("Par UUID", true)],
                );
                if before != self.tab {
                    self.selected.clear();
                }
                ui.add_space(6.0);
                let rows: Vec<(Uuid, String, String)> = match self.tab {
                    TAB_FRIENDS => {
                        let mut v: Vec<_> = world
                            .social
                            .friends
                            .iter()
                            .map(|f| (f.id, world.social.name_of(&f.id), String::new()))
                            .collect();
                        v.sort_by_key(|r| r.1.to_lowercase());
                        v
                    }
                    TAB_NEAR => super::people::nearby(world)
                        .into_iter()
                        .filter(|n| n.3 <= NEAR_ME_RANGE)
                        .map(|n| (n.0, n.1, format!("{:.0} m", n.3)))
                        .collect(),
                    TAB_UUID => {
                        ui.label(RichText::new("Saisissez l'UUID d'une personne :").size(12.0).color(p.muted));
                        ui.add(egui::TextEdit::singleline(&mut self.uuid_text).desired_width(f32::INFINITY));
                        match Uuid::parse_str(self.uuid_text.trim()) {
                            Ok(id) if !id.is_nil() => {
                                world.social.want_name(id);
                                vec![(id, world.social.name_of(&id), String::new())]
                            }
                            _ => Vec::new(),
                        }
                    }
                    _ => {
                        ui.label(
                            RichText::new("Saisissez une partie du nom d'une personne :")
                                .size(12.0)
                                .color(p.muted),
                        );
                        ui.horizontal(|ui| {
                            let r = ui.add(egui::TextEdit::singleline(&mut self.query).desired_width(ui.available_width() - 60.0));
                            let go = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            // LLFloaterAvatarPicker::onBtnFind: at least 3 characters
                            ui.add_enabled_ui(self.query.trim().chars().count() >= 3, |ui| {
                                if (flat_button(ui, p, "Aller").clicked() || go) && self.query.trim().chars().count() >= 3 {
                                    world.land.search_avatars(&self.query);
                                    self.selected.clear();
                                }
                            });
                        });
                        match &world.land.avatar_search {
                            Some((_, None)) => {
                                ui.label(RichText::new("Recherche...").size(12.0).color(p.muted));
                                Vec::new()
                            }
                            Some((q, Some(list))) if list.is_empty() => {
                                ui.label(RichText::new(format!("'{q}' pas trouvé")).size(12.0).color(p.muted));
                                Vec::new()
                            }
                            Some((_, Some(list))) => list.iter().map(|a| (a.id, a.display_name.clone(), a.username.clone())).collect(),
                            None => Vec::new(),
                        }
                    }
                };
                ui.add_space(4.0);
                // fixed like the window: the available height of a window
                // sized from its content is the whole screen
                let list_h = LIST_H;
                egui::Frame::new().fill(p.field).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.set_min_height(list_h);
                    egui::ScrollArea::vertical()
                        .id_salt((&self.id, self.tab))
                        .max_height(list_h)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if rows.is_empty() && self.tab == TAB_NEAR {
                                ui.label(RichText::new("Personne à proximité").size(12.0).color(p.muted));
                            }
                            for (id, name, extra) in &rows {
                                let sel = self.selected.contains(id);
                                let r = ui.horizontal(|ui| {
                                    let r = ui.selectable_label(sel, RichText::new(name).size(12.0));
                                    if !extra.is_empty() {
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            ui.label(RichText::new(extra).size(11.0).color(p.muted));
                                        });
                                    }
                                    r
                                });
                                let r = r.inner;
                                if r.clicked() {
                                    let add = ui.input(|i| i.modifiers.ctrl || i.modifiers.command) && self.multiple;
                                    if !add {
                                        self.selected.clear();
                                    }
                                    if sel && add {
                                        self.selected.retain(|x| x != id);
                                    } else {
                                        self.selected.push(*id);
                                    }
                                }
                                if r.double_clicked() {
                                    chosen = Some(vec![*id]);
                                }
                            }
                        });
                });
                ui.add_space(6.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if flat_button(ui, p, "Fermer").clicked() {
                        self.open = false;
                    }
                    ui.add_enabled_ui(!self.selected.is_empty(), |ui| {
                        if flat_button(ui, p, "Sélectionner").clicked() {
                            chosen = Some(self.selected.clone());
                        }
                    });
                });
            });
        if !open || chosen.is_some() {
            self.open = false;
        }
        chosen
    }
}
