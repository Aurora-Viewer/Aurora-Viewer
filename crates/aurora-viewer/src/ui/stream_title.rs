//! World › Stream title: the title the parcel's radio plays now, and the
//! titles it played before (`stream_title.rs`). Port of
//! FSFloaterStreamTitle and FSFloaterStreamTitleHistory
//! (indra/newview/fsfloaterstreamtitle.cpp, floater_fs_streamtitle.xml,
//! floater_fs_streamtitlehistory.xml, originally LGPL 2.1).
//!
//! - The window holds the title, which scrolls when it is too long for it
//!   (the Legacy Viewer moves it one letter at a time; here it slides), and
//!   the History button.
//! - Aurora adds a text-only look (the title bar's button): the title alone
//!   laid over the world, with a shadow, moved anywhere by dragging it; its
//!   right click opens the history, copies the title or gives the window
//!   back.
//! - The history lists the newest title first; several are chosen with Ctrl
//!   and Shift, copied one per line. "Clear" keeps the title playing now,
//!   "Max. entries" is FSStreamtitleHistoryLength.

use super::menu;
use super::widgets::{self, Floater};
use crate::settings::AudioSettings;
use crate::stream_title::StreamTitles;
use crate::theme::Palette;
use egui::{Align2, FontId, Pos2, Rect, RichText, Sense, Vec2};

/// The window's id (fs_streamtitle).
pub const WINDOW: &str = "stream_title";
/// The history's id (fs_streamtitlehistory).
pub const HISTORY: &str = "stream_title_history";
/// The text-only look's id (its place is kept with the windows').
const TEXT: &str = "stream_title_text";

/// The title's slide: still at its start, then sliding until its end shows,
/// still again at its end (SCROLL_END_DELAY, 4 s), then back to its start.
const HOLD_START: f64 = 2.0;
const HOLD_END: f64 = 4.0;
/// Points per second.
const SPEED: f64 = 40.0;

/// The widest the text-only title gets before it scrolls.
const TEXT_MAX_W: f32 = 420.0;

const ROW_H: f32 = 24.0;

#[derive(Default)]
pub struct StreamTitleUi {
    /// The history's titles chosen (`Played::id`).
    selected: Vec<u64>,
    /// The title Shift extends a selection from.
    anchor: Option<u64>,
    /// The title being shown and when it came, for its slide.
    shown: Option<(String, f64)>,
}

/// What the windows changed in the settings.
#[derive(Default)]
pub struct Changed {
    pub settings: bool,
}

impl StreamTitleUi {
    /// The title window or its text, then the history, while they are
    /// open. `open` is World › Stream title, `history` the history window.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        titles: &mut StreamTitles,
        audio: &mut AudioSettings,
        open: &mut bool,
        history: &mut bool,
    ) -> Changed {
        let mut changed = Changed::default();
        // the history goes with the title (FSFloaterStreamTitle::closeHistory
        // on a change of visibility)
        if !*open {
            *history = false;
        }
        let now = ctx.input(|i| i.time);
        let text = if titles.current().is_empty() {
            aurora_i18n::tr!("stream-title-none").to_owned()
        } else {
            titles.current().to_owned()
        };
        if self.shown.as_ref().is_none_or(|(t, _)| *t != text) {
            self.shown = Some((text.clone(), now));
        }
        let since = self.shown.as_ref().map_or(now, |(_, s)| *s);
        let playing = !titles.current().is_empty();
        if *open {
            let toggled = if audio.stream_title_text_only {
                self.text_only(ctx, p, &text, playing, since, open, history)
            } else {
                self.window(ctx, p, &text, playing, since, open, history)
            };
            if toggled {
                audio.stream_title_text_only = !audio.stream_title_text_only;
                changed.settings = true;
            }
        }
        if *history && self.history(ctx, p, titles, audio, history) {
            changed.settings = true;
        }
        changed
    }

    /// The window: the title and the History button. True when its title
    /// bar asks for the text-only look.
    #[allow(clippy::too_many_arguments)]
    fn window(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        text: &str,
        playing: bool,
        since: f64,
        open: &mut bool,
        history: &mut bool,
    ) -> bool {
        let mut text_only = false;
        let tip = aurora_i18n::tr!("stream-title-text-only");
        let mut floater = Floater::new(
            WINDOW,
            aurora_i18n::tr!("stream-title-window"),
            egui::pos2(360.0, 120.0),
            Vec2::new(400.0, 72.0),
        );
        floater.min_size = Vec2::new(220.0, 72.0);
        floater.actions.push(("typography", tip, &mut text_only));
        floater.show(ctx, p, open, |ui| {
            ui.horizontal(|ui| {
                let button = 20.0 + ui.spacing().item_spacing.x;
                let (rect, resp) = ui.allocate_exact_size(Vec2::new((ui.available_width() - button).max(40.0), ROW_H), Sense::click());
                let color = if playing { p.ink } else { p.muted };
                ticker(ui, p, rect, text, FontId::proportional(13.0), color, false, since);
                let resp = resp.on_hover_text(text);
                menu::context_menu(&resp, p, |ui| {
                    if menu::item_if(ui, p, "copy-filled", aurora_i18n::tr!("stream-title-copy"), playing) {
                        ui.ctx().copy_text(text.to_owned());
                    }
                });
                if widgets::icon_toggle(ui, p, "history", aurora_i18n::tr!("stream-title-history"), *history) {
                    widgets::toggle_floater(ui.ctx(), HISTORY, history);
                }
            });
        });
        text_only
    }

    /// The title alone over the world, dragged anywhere. True when its menu
    /// asks for the window again.
    #[allow(clippy::too_many_arguments)]
    fn text_only(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        text: &str,
        playing: bool,
        since: f64,
        open: &mut bool,
        history: &mut bool,
    ) -> bool {
        let mut window = false;
        let font = FontId::new(15.0, super::fonts::semibold(ctx));
        let width = ctx.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), font.clone(), p.ink).size().x);
        let room = super::layout::work_area(ctx);
        let pos = egui::pos2(room.center().x, room.top() + 56.0);
        widgets::toolstrip(ctx, TEXT, Align2::CENTER_TOP, pos, |ui| {
            let size = Vec2::new(width.min(TEXT_MAX_W) + 12.0, ROW_H);
            let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
            if resp.hovered() {
                let radius = ui.visuals().window_corner_radius;
                ui.painter().rect_filled(rect, radius, p.panel.gamma_multiply(0.35));
            }
            let color = if playing { p.ink } else { p.muted };
            ticker(ui, p, rect.shrink2(Vec2::new(6.0, 0.0)), text, font.clone(), color, true, since);
            let resp = resp.on_hover_text(aurora_i18n::tr!("stream-title-text-tip"));
            menu::context_menu(&resp, p, |ui| {
                if menu::check(ui, p, "history", aurora_i18n::tr!("stream-title-history"), *history, true) {
                    widgets::toggle_floater(ui.ctx(), HISTORY, history);
                }
                if menu::item_if(ui, p, "copy-filled", aurora_i18n::tr!("stream-title-copy"), playing) {
                    ui.ctx().copy_text(text.to_owned());
                }
                menu::separator(ui, p);
                if menu::item(ui, p, "app-window-filled", aurora_i18n::tr!("stream-title-as-window")) {
                    window = true;
                }
                if menu::item(ui, p, "x", aurora_i18n::tr!("common-close")) {
                    *open = false;
                }
            });
        });
        window
    }

    /// The history window. True when its maximum changed.
    fn history(&mut self, ctx: &egui::Context, p: &Palette, titles: &mut StreamTitles, audio: &mut AudioSettings, open: &mut bool) -> bool {
        let mut changed = false;
        // newest first (ADD_TOP)
        let order: Vec<u64> = titles.history().iter().rev().map(|t| t.id).collect();
        self.selected.retain(|id| order.contains(id));
        let mut floater = Floater::new(
            HISTORY,
            aurora_i18n::tr!("stream-title-history-window"),
            egui::pos2(360.0, 236.0),
            Vec2::new(400.0, 240.0),
        );
        floater.min_size = Vec2::new(400.0, 150.0);
        floater.show(ctx, p, open, |ui| {
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                ui.horizontal(|ui| {
                    let size = Vec2::new(0.0, 26.0);
                    let clear = aurora_i18n::tr!("stream-title-clear");
                    if widgets::action_button_if(ui, p, "trash-filled", clear, widgets::ActionKind::Flat, size, !order.is_empty()).clicked()
                    {
                        titles.clear(audio.stream_title_history as usize);
                        self.selected.clear();
                        self.anchor = None;
                    }
                    let copy = aurora_i18n::tr!("stream-title-copy-short");
                    let any = !self.selected.is_empty();
                    if widgets::action_button_if(ui, p, "copy-filled", copy, widgets::ActionKind::Flat, size, any)
                        .on_hover_text(aurora_i18n::tr!("stream-title-copy-selected"))
                        .clicked()
                    {
                        ui.ctx().copy_text(self.selection_text(titles));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let max = ui.add(
                            egui::DragValue::new(&mut audio.stream_title_history)
                                .custom_parser(aurora_i18n::parse_number)
                                .range(0..=10_000)
                                .speed(1.0),
                        );
                        if max.changed() {
                            titles.trim(audio.stream_title_history as usize);
                        }
                        // saved once the drag ends, not at each step
                        changed = (max.changed() && !max.dragged()) || max.drag_stopped();
                        ui.label(RichText::new(aurora_i18n::tr!("stream-title-max-entries")).size(12.5));
                    });
                });
                ui.add_space(6.0);
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    self.history_list(ui, p, titles, &order);
                });
            });
        });
        changed
    }

    fn history_list(&mut self, ui: &mut egui::Ui, p: &Palette, titles: &StreamTitles, order: &[u64]) {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            if order.is_empty() {
                ui.add_space(8.0);
                ui.label(
                    RichText::new(aurora_i18n::tr!("stream-title-history-empty"))
                        .size(12.5)
                        .color(p.muted),
                );
            }
            let radius = ui.visuals().window_corner_radius;
            for played in titles.history().iter().rev() {
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
                if !ui.is_rect_visible(rect) {
                    continue;
                }
                let selected = self.selected.contains(&played.id);
                if selected {
                    ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.30));
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.6));
                }
                let left = egui::pos2(rect.left() + 8.0, rect.center().y);
                let cut = widgets::paint_elided(ui, &played.title, left, rect.width() - 16.0, FontId::proportional(12.5), p.ink);
                let resp = if cut { resp.on_hover_text(&played.title) } else { resp };
                if resp.clicked() {
                    let modifiers = ui.input(|i| i.modifiers);
                    self.click(played.id, order, modifiers);
                }
                if resp.secondary_clicked() && !selected {
                    self.selected = vec![played.id];
                    self.anchor = Some(played.id);
                }
                menu::context_menu(&resp, p, |ui| {
                    if menu::item(ui, p, "copy-filled", aurora_i18n::tr!("stream-title-copy-selected")) {
                        ui.ctx().copy_text(self.selection_text(titles));
                    }
                });
            }
        });
    }

    /// The titles chosen, one per line, in the list's order
    /// (onCopyToClipboard).
    fn selection_text(&self, titles: &StreamTitles) -> String {
        let lines: Vec<&str> = titles
            .history()
            .iter()
            .rev()
            .filter(|t| self.selected.contains(&t.id))
            .map(|t| t.title.as_str())
            .collect();
        lines.join("\n")
    }

    /// A click on a title: alone, Ctrl to add or take it out, Shift for the
    /// titles from the last one clicked.
    fn click(&mut self, id: u64, order: &[u64], modifiers: egui::Modifiers) {
        if modifiers.shift
            && let Some(anchor) = self.anchor
            && let (Some(a), Some(b)) = (order.iter().position(|k| *k == anchor), order.iter().position(|k| *k == id))
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
            if let Some(i) = self.selected.iter().position(|k| *k == id) {
                self.selected.remove(i);
            } else {
                self.selected.push(id);
            }
        } else {
            self.selected = vec![id];
        }
        self.anchor = Some(id);
    }
}

/// Where the slide of a title `overflow` points wider than its room is,
/// `t` seconds after it came: 0 at its start, `overflow` at its end.
fn slide(overflow: f64, t: f64) -> f64 {
    let travel = overflow / SPEED;
    let t = t % (HOLD_START + travel + HOLD_END);
    ((t - HOLD_START) * SPEED).clamp(0.0, overflow)
}

/// `text` in `rect`, sliding to show its end when it is too long for it;
/// `shadow` keeps it readable over the world.
#[allow(clippy::too_many_arguments)]
fn ticker(ui: &egui::Ui, p: &Palette, rect: Rect, text: &str, font: FontId, color: egui::Color32, shadow: bool, since: f64) {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, color);
    let overflow = (galley.size().x - rect.width()).max(0.0);
    let x = if overflow > 0.0 {
        let t = ui.input(|i| i.time) - since;
        ui.ctx().request_repaint();
        slide(f64::from(overflow), t) as f32
    } else {
        0.0
    };
    let pos = Pos2::new(rect.left() - x, rect.center().y - galley.size().y / 2.0);
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    if shadow {
        let shade = p.navy.gamma_multiply(0.85);
        painter.galley_with_override_text_color(pos + Vec2::new(0.0, 1.0), galley.clone(), shade);
        painter.galley_with_override_text_color(pos + Vec2::new(1.0, 0.0), galley.clone(), shade);
    }
    painter.galley(pos, galley, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_title_holds_slides_holds_and_starts_again() {
        let overflow = 80.0; // 2 s of slide
        assert_eq!(slide(overflow, 0.0), 0.0);
        assert_eq!(slide(overflow, HOLD_START), 0.0);
        assert_eq!(slide(overflow, HOLD_START + 1.0), SPEED);
        assert_eq!(slide(overflow, HOLD_START + 2.0), overflow);
        assert_eq!(slide(overflow, HOLD_START + 2.0 + HOLD_END - 0.1), overflow);
        assert_eq!(slide(overflow, HOLD_START + 2.0 + HOLD_END + 0.5), 0.0);
    }

    #[test]
    fn shift_and_ctrl_choose_titles_in_the_list_order() {
        let mut ui = StreamTitleUi::default();
        let order = [5, 4, 3, 2, 1];
        ui.click(4, &order, egui::Modifiers::NONE);
        ui.click(2, &order, egui::Modifiers::SHIFT);
        assert_eq!(ui.selected, [4, 3, 2]);
        ui.click(3, &order, egui::Modifiers::COMMAND);
        assert_eq!(ui.selected, [4, 2]);
        ui.click(1, &order, egui::Modifiers::NONE);
        assert_eq!(ui.selected, [1]);
    }

    #[test]
    fn copies_the_chosen_titles_newest_first() {
        let mut titles = StreamTitles::default();
        for t in ["A", "B", "C"] {
            titles.update(t, 10);
        }
        let ids: Vec<u64> = titles.history().iter().map(|t| t.id).collect();
        let ui = StreamTitleUi {
            selected: vec![ids[0], ids[2]],
            ..Default::default()
        };
        assert_eq!(ui.selection_text(&titles), "C\nA");
    }
}
