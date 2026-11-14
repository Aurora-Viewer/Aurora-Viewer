//! "Choose a Resident": pick residents like LLFloaterAvatarPicker
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
    /// Row Shift selects from (LLScrollListCtrl::mLastSelected).
    anchor: Option<Uuid>,
}

impl AvatarPicker {
    pub fn open(&mut self, id: &str, multiple: bool) {
        self.open = true;
        self.id = id.to_owned();
        self.multiple = multiple;
        self.selected.clear();
        self.anchor = None;
    }

    /// Start on the search by name with `query` typed (a search begun
    /// elsewhere, such as the Conversations column).
    pub fn set_query(&mut self, query: &str) {
        self.tab = 0;
        query.clone_into(&mut self.query);
    }

    /// The residents chosen with "Select" (the picker closes).
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, world: &mut World) -> Option<Vec<Uuid>> {
        if !self.open {
            return None;
        }
        let mut chosen = None;
        let mut open = true;
        let screen = ctx.content_rect();
        let size = Vec2::new(380.0, 400.0);
        Floater::new(
            &self.id,
            aurora_i18n::tr!("ui-avatar-picker-choose-a-resident"),
            screen.center() - size * 0.5,
            size,
        )
        .fixed()
        .show(ctx, p, &mut open, |ui| {
            let before = self.tab;
            widgets::tabs(
                ui,
                p,
                &mut self.tab,
                &[
                    (aurora_i18n::tr!("ui-avatar-picker-search"), true),
                    (aurora_i18n::tr!("common-friends"), true),
                    (aurora_i18n::tr!("common-nearby"), true),
                    (aurora_i18n::tr!("ui-avatar-picker-by-uuid"), true),
                ],
            );
            if before != self.tab {
                self.selected.clear();
                self.anchor = None;
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
                    v.sort_by_key(|r| crate::natural_order::key(&r.1));
                    v
                }
                TAB_NEAR => super::people::nearby(world)
                    .into_iter()
                    .filter(|n| n.3 <= NEAR_ME_RANGE)
                    .map(|n| (n.0, n.1, format!("{:.0} m", n.3)))
                    .collect(),
                TAB_UUID => {
                    ui.label(
                        RichText::new(aurora_i18n::tr!("ui-avatar-picker-enter-a-person-s-uuid"))
                            .size(12.0)
                            .color(p.muted),
                    );
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
                        RichText::new(aurora_i18n::tr!("ui-avatar-picker-enter-part-of-a-person-s-name"))
                            .size(12.0)
                            .color(p.muted),
                    );
                    ui.horizontal(|ui| {
                        let r = {
                            let width = ui.available_width();
                            crate::ui::widgets::search_text_edit(ui, p, &mut self.query, |edit| edit.desired_width(width - 60.0))
                        };
                        let go = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        // LLFloaterAvatarPicker::onBtnFind: at least 3 characters
                        ui.add_enabled_ui(self.query.trim().chars().count() >= 3, |ui| {
                            if (flat_button(ui, p, aurora_i18n::tr!("ui-avatar-picker-go")).clicked() || go)
                                && self.query.trim().chars().count() >= 3
                            {
                                world.land.search_avatars(&self.query);
                                self.selected.clear();
                                self.anchor = None;
                            }
                        });
                    });
                    match &world.land.avatar_search {
                        Some((_, None)) => {
                            ui.label(RichText::new(aurora_i18n::tr!("common-search")).size(12.0).color(p.muted));
                            Vec::new()
                        }
                        Some((q, Some(list))) if list.is_empty() => {
                            ui.label(
                                RichText::new(aurora_i18n::tr!("ui-avatar-picker-not-found", q = q.to_string()))
                                    .size(12.0)
                                    .color(p.muted),
                            );
                            Vec::new()
                        }
                        Some((_, Some(list))) => list.iter().map(|a| (a.id, a.display_name.clone(), a.username.clone())).collect(),
                        None => Vec::new(),
                    }
                }
            };
            ui.add_space(4.0);
            // Ctrl+A / Shift+click: LLScrollListCtrl multi-select. Ctrl+A
            // selects every row only while no widget holds the keyboard focus:
            // in the search field it keeps selecting the text.
            let typing = ui.ctx().memory(|m| m.focused().is_some());
            if self.multiple && !typing && !rows.is_empty() && ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::A)) {
                self.selected = rows.iter().map(|(id, ..)| *id).collect();
                self.anchor = rows.last().map(|(id, ..)| *id);
            }
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
                            ui.label(
                                RichText::new(aurora_i18n::tr!("ui-avatar-picker-nobody-nearby"))
                                    .size(12.0)
                                    .color(p.muted),
                            );
                        }
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let w = ui.available_width();
                        for (id, name, extra) in &rows {
                            let sel = self.selected.contains(id);
                            // the whole row is selected, like LLScrollListCtrl
                            let (rect, r) = ui.allocate_exact_size(Vec2::new(w, 20.0), egui::Sense::click());
                            widgets::reveal_search_selection(ui, &r, self.selected.last() == Some(id));
                            if sel {
                                ui.painter().rect_filled(rect, 0.0, p.violet.gamma_multiply(0.35));
                            } else if r.hovered() {
                                ui.painter().rect_filled(rect, 0.0, p.raised.gamma_multiply(0.6));
                            }
                            // the extra column (username, distance) keeps its room
                            let extra_w = if extra.is_empty() {
                                0.0
                            } else {
                                let g = ui
                                    .painter()
                                    .layout_no_wrap(extra.clone(), egui::FontId::proportional(11.0), p.muted);
                                let x = rect.right() - 6.0 - g.size().x;
                                let at = crate::ui::widgets::row_text_origin(ui.painter(), &g, x, rect.center().y);
                                ui.painter().galley(at, g.clone(), p.muted);
                                g.size().x + 12.0
                            };
                            let color = super::colors::resident_name(world, id, p.ink);
                            let g = egui::WidgetText::from(RichText::new(name).size(12.0).color(color)).into_galley(
                                ui,
                                Some(egui::TextWrapMode::Truncate),
                                (w - 12.0 - extra_w).max(20.0),
                                egui::TextStyle::Body,
                            );
                            let at = crate::ui::widgets::row_text_origin(ui.painter(), &g, rect.left() + 6.0, rect.center().y);
                            ui.painter().galley(at, g, p.ink);
                            if r.clicked() {
                                let mods = ui.input(|i| i.modifiers);
                                if self.multiple {
                                    let order: Vec<Uuid> = rows.iter().map(|(row, ..)| *row).collect();
                                    // Ctrl+click toggle, Shift+click range (no Contacts
                                    // MAX_SELECT cap — Share can pick a whole crowd)
                                    click_select(&mut self.selected, &mut self.anchor, &order, *id, mods);
                                } else {
                                    self.selected = vec![*id];
                                    self.anchor = Some(*id);
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
                if flat_button(ui, p, aurora_i18n::tr!("common-close")).clicked() {
                    self.open = false;
                }
                ui.add_enabled_ui(!self.selected.is_empty(), |ui| {
                    if flat_button(ui, p, aurora_i18n::tr!("ui-avatar-picker-select")).clicked() {
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

/// Multi-select like LLScrollListCtrl / Contacts, without the friend-action
/// row cap: Ctrl toggles, Shift fills from the anchor, plain click alone.
fn click_select(sel: &mut Vec<Uuid>, anchor: &mut Option<Uuid>, order: &[Uuid], id: Uuid, mods: egui::Modifiers) {
    if mods.command {
        if let Some(i) = sel.iter().position(|s| *s == id) {
            sel.remove(i);
        } else {
            sel.push(id);
        }
        *anchor = Some(id);
    } else if mods.shift
        && let Some(a) = anchor.and_then(|a| order.iter().position(|x| *x == a))
        && let Some(b) = order.iter().position(|x| *x == id)
    {
        let (lo, hi) = (a.min(b), a.max(b));
        *sel = order[lo..=hi].to_vec();
    } else {
        *sel = vec![id];
        *anchor = Some(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shift_click_selects_the_range_between_anchor_and_row() {
        let order: Vec<Uuid> = (1..=5).map(Uuid::from_u128).collect();
        let mut sel = Vec::new();
        let mut anchor = None;
        let plain = egui::Modifiers::default();
        let shift = egui::Modifiers {
            shift: true,
            ..Default::default()
        };
        click_select(&mut sel, &mut anchor, &order, order[1], plain);
        assert_eq!(sel, vec![order[1]]);
        assert_eq!(anchor, Some(order[1]));
        click_select(&mut sel, &mut anchor, &order, order[4], shift);
        assert_eq!(sel, order[1..=4].to_vec());
        click_select(&mut sel, &mut anchor, &order, order[0], shift);
        assert_eq!(sel, order[0..=1].to_vec());
    }

    #[test]
    fn shift_range_is_not_capped_at_twenty() {
        let order: Vec<Uuid> = (1..=40).map(Uuid::from_u128).collect();
        let mut sel = Vec::new();
        let mut anchor = Some(order[0]);
        let shift = egui::Modifiers {
            shift: true,
            ..Default::default()
        };
        click_select(&mut sel, &mut anchor, &order, order[39], shift);
        assert_eq!(sel.len(), 40);
    }
}
