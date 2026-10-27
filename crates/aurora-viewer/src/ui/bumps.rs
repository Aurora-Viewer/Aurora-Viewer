//! "Bumps, pushes & hits" (Help menu): who bumped us, pushed us with a
//! script or hit us with an object, newest first (`world/collisions.rs`).
//! Port of LLFloaterBump (indra/newview/llfloaterbump.cpp, floater_bumps.xml,
//! menu_fs_bump_list.xml, originally LGPL 2.1): one line per resident and
//! kind with its time in SLT, several chosen with Ctrl / Shift. The
//! right-click menu copies the lines chosen, then holds the avatar lists'
//! menu of the resident clicked (profile, IM, zoom, teleport to, report,
//! block…), a wider one than the Legacy Viewer's.

use super::context::{self, AvatarList};
use super::menu;
use super::widgets::{self, Floater};
use crate::theme::Palette;
use crate::world::World;
use crate::world::collisions::{Collision, Kind};
use egui::{RichText, Sense, Vec2};
use uuid::Uuid;

/// The window's id (Floater.Show "bumps").
pub const WINDOW: &str = "bumps";

type Key = (Uuid, Kind);

const ROW_H: f32 = 24.0;
const TIME_W: f32 = 64.0;

#[derive(Default)]
pub struct BumpsUi {
    selected: Vec<Key>,
    /// The row Shift extends a selection from.
    anchor: Option<Key>,
}

/// "[hour:min:second]" in SLT (floater_bumps.xml timeStr).
fn time_text(time: u32) -> String {
    let (_, _, _, _, h, mi, s) = crate::ui::land::slt_parts(time as i64);
    format!("{h:02}:{mi:02}:{s:02}")
}

/// The line as the list shows it and the copy gives it.
fn line(world: &World, c: &Collision) -> String {
    format!("{}  {}", time_text(c.time), c.kind.sentence(&world.social.name_of(&c.perp)))
}

impl BumpsUi {
    /// A click on a row: alone, Ctrl to add or take it out, Shift for the
    /// rows from the last one clicked.
    fn click(&mut self, key: Key, order: &[Key], modifiers: egui::Modifiers) {
        if modifiers.shift
            && let Some(anchor) = self.anchor
            && let (Some(a), Some(b)) = (order.iter().position(|k| *k == anchor), order.iter().position(|k| *k == key))
        {
            if !modifiers.command {
                self.selected.clear();
            }
            for k in &order[a.min(b)..=a.max(b)] {
                if !self.selected.contains(k) {
                    self.selected.push(*k);
                }
            }
            return;
        }
        if modifiers.command {
            if let Some(i) = self.selected.iter().position(|k| *k == key) {
                self.selected.remove(i);
            } else {
                self.selected.push(key);
            }
        } else {
            self.selected = vec![key];
        }
        self.anchor = Some(key);
    }

    /// Draw the window while `open`.
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, world: &World, open: &mut bool) {
        if !*open {
            return;
        }
        let rows: Vec<Collision> = world.collisions.list().copied().collect();
        let order: Vec<Key> = rows.iter().map(|c| (c.perp, c.kind)).collect();
        self.selected.retain(|k| order.contains(k));
        let mut floater = Floater::new(
            WINDOW,
            aurora_i18n::tr!("bumps-title"),
            egui::pos2(300.0, 160.0),
            Vec2::new(480.0, 220.0),
        );
        floater.min_size = Vec2::new(300.0, 140.0);
        floater.show(ctx, p, open, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                if rows.is_empty() {
                    ui.add_space(8.0);
                    ui.label(RichText::new(aurora_i18n::tr!("bumps-none")).size(12.5).color(p.muted));
                }
                let radius = ui.visuals().window_corner_radius;
                for c in &rows {
                    let key = (c.perp, c.kind);
                    world.social.avatar_names.want(&c.perp);
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
                    if !ui.is_rect_visible(rect) {
                        continue;
                    }
                    let selected = self.selected.contains(&key);
                    if selected {
                        ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.30));
                    } else if resp.hovered() {
                        ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.6));
                    }
                    let font = egui::FontId::proportional(12.5);
                    let time = egui::pos2(rect.left() + 8.0, rect.center().y);
                    widgets::paint_elided(ui, &time_text(c.time), time, TIME_W - 8.0, font.clone(), p.muted);
                    let text = c.kind.sentence(&world.social.name_of(&c.perp));
                    let left = rect.left() + 8.0 + TIME_W;
                    let room = rect.right() - left - 8.0;
                    let cut = widgets::paint_elided(ui, &text, egui::pos2(left, rect.center().y), room, font, p.ink);
                    if resp.clicked() {
                        let modifiers = ui.input(|i| i.modifiers);
                        self.click(key, &order, modifiers);
                    }
                    // menu_fs_bump_list.xml acts on the selection: the row
                    // right-clicked joins it
                    if resp.secondary_clicked() && !selected {
                        self.selected = vec![key];
                        self.anchor = Some(key);
                    }
                    menu::context_menu(&resp, p, |ui| {
                        if menu::item(ui, p, "copy-filled", aurora_i18n::tr!("bumps-copy")) {
                            let text: Vec<String> = rows
                                .iter()
                                .filter(|r| self.selected.contains(&(r.perp, r.kind)))
                                .map(|r| line(world, r))
                                .collect();
                            ui.ctx().copy_text(text.join("\n"));
                            ui.close();
                        }
                        menu::separator(ui, p);
                        context::avatar_list_menu(ui, p, world, c.perp, AvatarList::Name);
                    });
                    if cut && resp.hover_pos().is_some_and(|at| at.x >= left) {
                        resp.on_hover_text(text);
                    }
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_follow_daylight_saving_time() {
        // 2026-10-26 19:00:05 UTC is 12:00:05 PDT, a week before the switch
        assert_eq!(time_text(1_793_041_205), "12:00:05");
        // 2026-11-01 09:00 UTC: 2:00 PDT becomes 1:00 PST
        assert_eq!(time_text(1_793_523_599), "01:59:59");
        assert_eq!(time_text(1_793_523_600), "01:00:00");
    }

    #[test]
    fn ctrl_and_shift_choose_several_rows() {
        let order: Vec<Key> = (1..=4).map(|n| (Uuid::from_u128(n), Kind::Bump)).collect();
        let mut ui = BumpsUi::default();
        let none = egui::Modifiers::default();
        let ctrl = egui::Modifiers {
            command: true,
            ..Default::default()
        };
        let shift = egui::Modifiers {
            shift: true,
            ..Default::default()
        };
        ui.click(order[0], &order, none);
        ui.click(order[2], &order, shift);
        assert_eq!(ui.selected, order[..3]);
        ui.click(order[1], &order, ctrl);
        assert_eq!(ui.selected, [order[0], order[2]]);
        ui.click(order[3], &order, none);
        assert_eq!(ui.selected, [order[3]]);
    }

    #[test]
    fn the_time_is_shown_in_slt() {
        // 2026-01-10 20:04:05 UTC is 12:04:05 PST
        assert_eq!(time_text(1_768_075_445), "12:04:05");
    }
}
