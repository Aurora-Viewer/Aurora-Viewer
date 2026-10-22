//! The Legacy Viewer's look of the money tracker (floater_fs_money_tracker.xml,
//! menu_fs_moneytracker_list.xml): the Time / Name / Amount list, oldest
//! first and sorted by time, the summary line, "Track when closed" and
//! "Clear".

use super::{LEGACY_WINDOW, MoneyTrackerUi, Payment, TrackerAction, hhmmss, party_name};
use crate::theme::Palette;
use crate::ui::menu;
use crate::ui::widgets::{self, ActionKind, Floater};
use crate::world::World;
use egui::{FontId, Rect, Sense, Vec2};

const ROW_H: f32 = 20.0;
/// Widths of the time and amount columns; the name takes the rest.
const TIME_W: f32 = 64.0;
const AMOUNT_W: f32 = 63.0;

/// The columns, and how a click on their heading sorts the list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Column {
    #[default]
    Time,
    Name,
    Amount,
}

fn cells(row: Rect) -> [(Column, Rect); 3] {
    let time = Rect::from_min_max(row.min, egui::pos2(row.left() + TIME_W, row.bottom()));
    let amount = Rect::from_min_max(egui::pos2(row.right() - AMOUNT_W, row.top()), row.max);
    let name = Rect::from_min_max(egui::pos2(time.right(), row.top()), egui::pos2(amount.left(), row.bottom()));
    [(Column::Time, time), (Column::Name, name), (Column::Amount, amount)]
}

impl MoneyTrackerUi {
    pub(super) fn show_legacy(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        world: &World,
        open: &mut bool,
        always: bool,
        actions: &mut Vec<TrackerAction>,
    ) {
        self.undo = None;
        let screen = ctx.content_rect();
        let mut floater = Floater::new(
            LEGACY_WINDOW,
            aurora_i18n::tr!("money-tracker-legacy-title"),
            egui::pos2(screen.right() - 300.0, screen.top() + 90.0),
            Vec2::new(260.0, 300.0),
        );
        floater.min_size = Vec2::new(220.0, 220.0);
        floater.steady_size().show(ctx, p, open, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            let mut rows: Vec<Payment> = self.tracker.payments().to_vec();
            let names: Vec<String> = rows.iter().map(|x| party_name(world, x)).collect();
            let mut order: Vec<usize> = (0..rows.len()).collect();
            let (column, descending) = self.legacy_sort;
            order.sort_by(|&a, &b| {
                let o = match column {
                    Column::Time => rows[a].serial.cmp(&rows[b].serial),
                    Column::Name => names[a].to_lowercase().cmp(&names[b].to_lowercase()),
                    Column::Amount => rows[a].signed().cmp(&rows[b].signed()),
                };
                if descending { o.reverse() } else { o }
            });
            let names: Vec<String> = order.iter().map(|&i| names[i].clone()).collect();
            rows = order.iter().map(|&i| rows[i].clone()).collect();
            let serials: Vec<u64> = rows.iter().map(|x| x.serial).collect();
            // the footer from the bottom up, then the list in exactly what is
            // left: the body never measures more than the window
            let rest = ui.available_rect_before_wrap();
            let mut footer = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(rest)
                    .layout(egui::Layout::bottom_up(egui::Align::Min)),
            );
            self.legacy_footer(&mut footer, p, always, actions);
            let top = (footer.min_rect().top() - ui.spacing().item_spacing.y).max(rest.top());
            let area = Rect::from_min_max(rest.min, egui::pos2(rest.right(), top));
            let mut list = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(area)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            self.legacy_header(&mut list, p);
            egui::ScrollArea::vertical()
                .id_salt("money-tracker-legacy-list")
                .auto_shrink([false, false])
                .max_height((area.height() - 24.0).max(0.0))
                .stick_to_bottom(true)
                .show(&mut list, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (n, (x, name)) in rows.iter().zip(&names).enumerate() {
                        self.legacy_row(ui, p, world, x, name, n, &serials);
                    }
                });
            ui.allocate_rect(rest, Sense::hover());
        });
    }

    /// The summary, "Track when closed" and "Clear", from the bottom up in
    /// a bottom-up `ui`.
    fn legacy_footer(&mut self, ui: &mut egui::Ui, p: &Palette, always: bool, actions: &mut Vec<TrackerAction>) {
        let w = ui.available_width();
        if widgets::action_button(
            ui,
            p,
            "",
            aurora_i18n::tr!("money-tracker-clear"),
            ActionKind::Flat,
            Vec2::new(w, 24.0),
        )
        .clicked()
        {
            self.tracker.clear();
            self.selected.clear();
            self.anchor = None;
        }
        let mut track = always;
        if widgets::checkbox(ui, p, &mut track, aurora_i18n::tr!("money-tracker-legacy-always"))
            .on_hover_text(aurora_i18n::tr!("money-tracker-always-help"))
            .changed()
        {
            actions.push(TrackerAction::AlwaysTrack(track));
        }
        let (received, paid) = self.tracker.totals();
        ui.label(
            egui::RichText::new(aurora_i18n::tr!(
                "money-tracker-legacy-summary",
                paid = aurora_i18n::integer(paid),
                received = aurora_i18n::integer(received)
            ))
            .size(12.0)
            .color(p.ink),
        );
    }

    /// The column headings; a click sorts by the column, a second the
    /// other way.
    fn legacy_header(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 20.0), Sense::hover());
        ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.field);
        let titles = [
            aurora_i18n::tr!("money-tracker-legacy-time"),
            aurora_i18n::tr!("money-tracker-legacy-name"),
            aurora_i18n::tr!("money-tracker-legacy-amount"),
        ];
        for ((column, cell), title) in cells(rect).into_iter().zip(titles) {
            let resp = ui.interact(
                cell,
                ui.scope_id().with(("money-tracker-legacy-head", column as u8)),
                Sense::click(),
            );
            let sorted = self.legacy_sort.0 == column;
            let color = if sorted || resp.hovered() { p.ink } else { p.muted };
            let left = egui::pos2(cell.left() + 4.0, cell.center().y);
            widgets::paint_elided(ui, title, left, cell.width() - 8.0, FontId::proportional(12.0), color);
            if resp.clicked() {
                self.legacy_sort = (column, sorted && !self.legacy_sort.1);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn legacy_row(&mut self, ui: &mut egui::Ui, p: &Palette, world: &World, x: &Payment, name: &str, n: usize, order: &[u64]) {
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
        let selected = self.selected.contains(&x.serial);
        if ui.is_rect_visible(rect) {
            let radius = ui.visuals().window_corner_radius;
            if selected {
                ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.30));
            } else if resp.hovered() {
                ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.6));
            } else if n % 2 == 1 {
                // draw_stripes
                ui.painter().rect_filled(rect, radius, p.field.gamma_multiply(0.6));
            }
            let font = FontId::proportional(12.0);
            let amount = x.signed().to_string();
            let color = if x.received { p.success } else { p.danger };
            for (column, cell) in cells(rect) {
                let left = egui::pos2(cell.left() + 4.0, cell.center().y);
                let (text, color) = match column {
                    Column::Time => (hhmmss(x.time), p.ink),
                    Column::Name => (name.to_owned(), p.ink),
                    Column::Amount => (amount.clone(), color),
                };
                widgets::paint_elided(ui, &text, left, cell.width() - 8.0, font.clone(), color);
            }
        }
        if resp.clicked() {
            let m = ui.input(|i| i.modifiers);
            self.click(x.serial, order, m.command, m.shift);
        }
        if resp.secondary_clicked() && !selected {
            self.selected = std::collections::BTreeSet::from([x.serial]);
            self.anchor = Some(x.serial);
        }
        menu::context_menu(&resp, p, |ui| {
            let any = !self.selected.is_empty();
            if menu::item_if(ui, p, "copy-filled", aurora_i18n::tr!("money-tracker-legacy-copy"), any) {
                let text = super::csv(&self.selection(), |x| party_name(world, x));
                ui.ctx().copy_text(text);
            }
            if menu::item_if(ui, p, "trash-filled", aurora_i18n::tr!("money-tracker-legacy-delete"), any) {
                self.tracker.remove(&self.selected);
                self.selected.clear();
                self.anchor = None;
            }
        });
    }
}
