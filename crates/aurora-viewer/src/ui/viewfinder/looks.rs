//! The viewfinder's filter panel (issue #969): the filter in use and "No
//! filter" at the top, the Legacy Viewer's filters, Aurora's looks by
//! section and the user's 3D tables in three tabs, a search over all of
//! them, the strength and the before / after switch always at the bottom.
//! The filter pointed at is tried on the live view; Aurora's looks say
//! what they do in their tooltip. The before / after line over the photo's
//! frame leaves the scene as it is on its left (`snapshot::looks`).

use super::{Out, Viewfinder, bold, keep_keys, regular, shadow_text, widgets};
use crate::snapshot::Settings;
use crate::snapshot::looks::{self, Entry, Tab};
use crate::theme::Palette;
use crate::ui::snapshot::{Action, SnapshotUi, chip};
use egui::{Align2, Id, Order, Pos2, Rect, RichText, Sense, Stroke, Vec2};

const WIDTH: f32 = 340.0;

/// The panel's own state: its tab and search.
#[derive(Default)]
pub struct LooksPanel {
    pub tab: Tab,
    pub query: String,
}

/// A wrapped row of chips; the one pointed at is tried, a click chooses it.
fn chips(ui: &mut egui::Ui, p: &Palette, entries: &[Entry], s: &mut Settings, panel: &mut SnapshotUi) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        for e in entries {
            let mut resp = chip(ui, p, &e.name, None, s.look.as_deref() == Some(e.id.as_str()), true);
            keep_keys(&resp);
            if let Some(tip) = e.tip {
                resp = resp.on_hover_text(tip);
            }
            if resp.hovered() {
                panel.look_hover = Some(Some(e.id.clone()));
            }
            if resp.clicked() {
                s.look = Some(e.id.clone());
            }
        }
    });
}

/// A section's title: small, muted, in capitals.
fn heading(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(text.to_uppercase()).font(regular(10.5)).color(p.muted));
    ui.add_space(2.0);
}

impl Viewfinder {
    /// The panel, left of the dock.
    pub(super) fn looks_panel(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        screen: Rect,
        s: &mut Settings,
        panel: &mut SnapshotUi,
        out: &mut Vec<Out>,
    ) {
        egui::Area::new(Id::unique("vf_looks"))
            .order(Order::Foreground)
            .fade_in(false)
            .pivot(Align2::RIGHT_CENTER)
            .fixed_pos(Pos2::new(screen.right() - 84.0, screen.center().y))
            .show(ctx, |ui| {
                widgets::overlay(ui, p, egui::Margin::same(12), |ui| {
                    ui.set_width(WIDTH);
                    ui.spacing_mut().item_spacing.y = 6.0;
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(aurora_i18n::tr!("photo-looks"))
                                .font(bold(ctx, 14.0))
                                .color(p.ink),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::icon_button(ui, p, "x", aurora_i18n::tr!("common-close"), true) {
                                self.looks = false;
                            }
                        });
                    });
                    self.looks_current(ui, p, s, panel);
                    let labels: Vec<(&str, bool)> = Tab::ALL.iter().map(|t| (t.label(), true)).collect();
                    let mut tab = Tab::ALL.iter().position(|t| *t == self.looks_state.tab).unwrap_or(0);
                    if widgets::stretched_tabs(ui, p, &mut tab, &labels) {
                        self.looks_state.tab = Tab::ALL[tab];
                        self.looks_state.query.clear();
                    }
                    // in a row: the field centres itself in the height it is given
                    ui.horizontal(|ui| {
                        let search = widgets::clearable_search_box(
                            ui,
                            p,
                            &mut self.looks_state.query,
                            aurora_i18n::tr!("photo-looks-search"),
                            ui.available_width(),
                        );
                        keep_keys(&search);
                    });
                    // above the bottom bar and under the top one
                    let room = (screen.height() - 470.0).max(140.0);
                    egui::ScrollArea::vertical()
                        .id_salt("vf_looks_list")
                        .max_height(room)
                        .auto_shrink([false, true])
                        .show(ui, |ui| self.looks_list(ui, p, s, panel, out));
                    widgets::divider(ui, p);
                    ui.add_enabled_ui(s.look.is_some(), |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(aurora_i18n::tr!("photo-look-strength")).color(p.ink));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.spacing_mut().slider_width = (ui.available_width() - 60.0).max(60.0);
                                let resp = ui.add(egui::Slider::new(&mut s.look_strength, 0..=100).suffix(" %"));
                                keep_keys(&resp);
                            });
                        });
                        let mut compare = panel.look_compare.is_some();
                        let resp = widgets::switch_labeled(ui, p, &mut compare, aurora_i18n::tr!("photo-look-compare"))
                            .on_hover_text(aurora_i18n::tr!("photo-look-compare-tip"));
                        keep_keys(&resp);
                        if resp.changed() {
                            panel.look_compare = compare.then_some(0.5);
                        }
                    });
                });
            });
    }

    /// The filter in use, where it comes from, and "No filter".
    fn looks_current(&mut self, ui: &mut egui::Ui, p: &Palette, s: &mut Settings, panel: &mut SnapshotUi) {
        ui.horizontal(|ui| {
            let (name, from) = match s.look.as_deref() {
                Some(id) => (looks::label(Some(id)), looks::tab_of(id).label()),
                None => (looks::label(None), aurora_i18n::tr!("photo-look-hint")),
            };
            ui.vertical(|ui| {
                ui.label(RichText::new(name.as_ref()).font(bold(ui.ctx(), 13.0)).color(p.ink));
                ui.label(RichText::new(from).font(regular(11.0)).color(p.muted));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let none = chip(ui, p, aurora_i18n::tr!("photo-look-none"), None, s.look.is_none(), true);
                keep_keys(&none);
                if none.hovered() {
                    panel.look_hover = Some(None);
                }
                if none.clicked() {
                    s.look = None;
                }
            });
        });
    }

    /// The tab's filters, or what the search finds in every tab.
    fn looks_list(&mut self, ui: &mut egui::Ui, p: &Palette, s: &mut Settings, panel: &mut SnapshotUi, out: &mut Vec<Out>) {
        let query = self.looks_state.query.trim();
        if !query.is_empty() {
            let found = looks::search(query, &panel.lut_names);
            if found.is_empty() {
                ui.label(RichText::new(aurora_i18n::tr!("photo-looks-no-result")).color(p.muted));
            } else {
                chips(ui, p, &found, s, panel);
            }
            return;
        }
        match self.looks_state.tab {
            Tab::Legacy => chips(ui, p, &looks::legacy_list(), s, panel),
            Tab::Aurora => {
                for (section, entries) in looks::aurora_list() {
                    heading(ui, p, looks::section_name(section));
                    chips(ui, p, &entries, s, panel);
                }
            }
            Tab::Luts => self.lut_rows(ui, p, s, panel, out),
        }
    }

    /// The user's tables: a row each with its bin, then the import.
    fn lut_rows(&mut self, ui: &mut egui::Ui, p: &Palette, s: &mut Settings, panel: &mut SnapshotUi, out: &mut Vec<Out>) {
        let entries = looks::lut_list(&panel.lut_names);
        if entries.is_empty() {
            ui.add(egui::Label::new(RichText::new(aurora_i18n::tr!("photo-lut-empty")).color(p.muted)).wrap());
        }
        for e in &entries {
            ui.horizontal(|ui| {
                let resp = chip(ui, p, &e.name, None, s.look.as_deref() == Some(e.id.as_str()), true);
                keep_keys(&resp);
                if resp.hovered() {
                    panel.look_hover = Some(Some(e.id.clone()));
                }
                if resp.clicked() {
                    s.look = Some(e.id.clone());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::icon_button(ui, p, "trash-filled", aurora_i18n::tr!("photo-lut-remove"), true)
                        && let Some(file) = e.id.strip_prefix(looks::LUT)
                    {
                        out.push(Out::Panel(Action::RemoveLut(file.to_owned())));
                    }
                });
            });
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let import = widgets::action_button(
                ui,
                p,
                "file-import",
                aurora_i18n::tr!("photo-lut-import"),
                widgets::ActionKind::Flat,
                Vec2::new(0.0, 28.0),
            );
            keep_keys(&import);
            if import.clicked() {
                out.push(Out::Panel(Action::ImportLut));
            }
            let folder = widgets::action_button(
                ui,
                p,
                "folder-open",
                aurora_i18n::tr!("photo-lut-folder"),
                widgets::ActionKind::Flat,
                Vec2::new(0.0, 28.0),
            );
            keep_keys(&folder);
            if folder.clicked() {
                out.push(Out::Panel(Action::OpenLutFolder));
            }
        });
        ui.add(egui::Label::new(RichText::new(aurora_i18n::tr!("photo-lut-hint")).font(regular(11.0)).color(p.muted)).wrap());
    }

    /// The before / after line over the photo's `frame`: dragged sideways,
    /// "Before" on its left and "After" on its right.
    pub(super) fn compare_line(&mut self, ctx: &egui::Context, p: &Palette, frame: Rect, panel: &mut SnapshotUi) {
        let Some(at) = panel.look_compare else {
            return;
        };
        let x = frame.left() + at.clamp(0.0, 1.0) * frame.width();
        let grip = Rect::from_x_y_ranges(x - 12.0..=x + 12.0, frame.y_range());
        egui::Area::new(Id::unique("vf_look_compare"))
            .order(Order::Middle)
            .fade_in(false)
            .fixed_pos(grip.min)
            .show(ctx, |ui| {
                let (rect, resp) = ui.allocate_exact_size(grip.size(), Sense::drag());
                keep_keys(&resp);
                if resp.dragged() {
                    let moved = at + resp.drag_delta().x / frame.width().max(1.0);
                    panel.look_compare = Some(moved.clamp(0.0, 1.0));
                }
                let resp = resp.on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
                let painter = ui.painter();
                let strong = resp.hovered() || resp.dragged();
                let line = if strong { p.ink } else { p.ink.gamma_multiply(0.8) };
                painter.line_segment(
                    [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                    Stroke::new(if strong { 2.0 } else { 1.5 }, line),
                );
                // the grip in the middle
                let c = Pos2::new(x, rect.center().y);
                painter.circle_filled(c, 13.0, p.navy.gamma_multiply(0.8));
                painter.circle_stroke(c, 13.0, Stroke::new(1.5, line));
                super::icon(painter, "arrows-horizontal", Rect::from_center_size(c, Vec2::splat(16.0)), p.ink);
            });
        let font = bold(ctx, 12.0);
        let painter = ctx.layer_painter(egui::LayerId::new(Order::Middle, Id::unique("vf_look_compare_labels")));
        let y = frame.top() + 18.0;
        shadow_text(
            &painter,
            p,
            Pos2::new(x - 10.0, y),
            Align2::RIGHT_CENTER,
            aurora_i18n::tr!("photo-look-before"),
            font.clone(),
            p.ink,
        );
        shadow_text(
            &painter,
            p,
            Pos2::new(x + 10.0, y),
            Align2::LEFT_CENTER,
            aurora_i18n::tr!("photo-look-after"),
            font,
            p.ink,
        );
    }
}
