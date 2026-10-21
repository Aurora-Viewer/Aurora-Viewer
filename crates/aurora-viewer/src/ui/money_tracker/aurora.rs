//! The Aurora look of the money tracker (the mockup's variant A, chosen by
//! the human): the totals received, paid and net, a filter, the newest
//! transaction first with what it was, and an undo after a deletion.

use super::{Filter, MoneyTrackerUi, Party, Payment, TrackerAction, WINDOW, party_name, trans};
use crate::theme::Palette;
use crate::ui::widgets::{self, ActionKind, Floater};
use crate::ui::{fonts, icons, menu};
use crate::world::World;
use egui::{FontId, Pos2, Rect, Sense, Vec2};
use std::collections::HashMap;
use uuid::Uuid;

const ROW_H: f32 = 46.0;
const TIME_W: f32 = 38.0;
const PICTURE: f32 = 26.0;
/// How long "Undo" stays after a deletion.
const UNDO_SECONDS: f64 = 8.0;

/// "+L$ 1,000" / "−L$ 450".
fn signed_money(x: &Payment) -> String {
    let sign = if x.received { '+' } else { '−' };
    format!("{sign}L$ {}", aurora_i18n::integer(i64::from(x.amount)))
}

/// A total of the head: "+1,825", "−630".
fn total(value: i64, plus: bool) -> String {
    let sign = if value < 0 {
        "−"
    } else if plus && value > 0 {
        "+"
    } else {
        ""
    };
    format!("{sign}{}", aurora_i18n::integer(value.abs()))
}

fn paint_icon(painter: &egui::Painter, name: &str, rect: Rect, tint: egui::Color32) {
    if let Some(t) = icons::global(name) {
        painter.image(t.id(), rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), tint);
    }
}

impl MoneyTrackerUi {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn show_aurora(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        world: &World,
        pics: &HashMap<Uuid, egui::TextureHandle>,
        open: &mut bool,
        always: bool,
        actions: &mut Vec<TrackerAction>,
    ) {
        let now = ctx.input(|i| i.time);
        if self.undo.as_ref().is_some_and(|u| now - u.shown > UNDO_SECONDS) {
            self.undo = None;
        }
        let screen = ctx.content_rect();
        let mut floater = Floater::new(
            WINDOW,
            aurora_i18n::tr!("money-tracker-title"),
            egui::pos2(screen.right() - 420.0, screen.top() + 90.0),
            Vec2::new(380.0, 600.0),
        );
        floater.min_size = Vec2::new(320.0, 380.0);
        floater.steady_size().show(ctx, p, open, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            self.head(ui, p);
            // the footer first, from the bottom up, then the list in exactly
            // what is left: the body never measures more than the window (a
            // guessed footer height let it grow a little every frame)
            let rest = ui.available_rect_before_wrap();
            let mut footer = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(rest)
                    .layout(egui::Layout::bottom_up(egui::Align::Min)),
            );
            self.footer(&mut footer, p, world, always, now, actions);
            let top = footer.min_rect().top() - ui.spacing().item_spacing.y;
            let list = Rect::from_min_max(rest.min, Pos2::new(rest.right(), top.max(rest.top())));
            self.list(ui, p, world, pics, list, now, actions);
            ui.allocate_rect(rest, Sense::hover());
        });
        if self.undo.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }

    /// The three totals, since when, and the filter.
    fn head(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let (received, paid) = self.tracker.totals();
        let tiles = [
            (aurora_i18n::tr!("money-tracker-received"), total(received, true), p.success),
            (aurora_i18n::tr!("money-tracker-paid"), total(-paid, false), p.danger),
            (aurora_i18n::tr!("money-tracker-net"), total(received - paid, true), p.ink),
        ];
        let gap = 6.0;
        let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 50.0), Sense::hover());
        let w = (row.width() - gap * 2.0) / 3.0;
        let radius = ui.visuals().window_corner_radius;
        for (i, (label, value, color)) in tiles.into_iter().enumerate() {
            let tile = Rect::from_min_size(Pos2::new(row.left() + i as f32 * (w + gap), row.top()), Vec2::new(w, row.height()));
            ui.painter().rect_filled(tile, radius, p.field);
            let label = label.to_uppercase();
            widgets::paint_elided(
                ui,
                &label,
                Pos2::new(tile.left() + 10.0, tile.top() + 15.0),
                w - 20.0,
                FontId::new(11.0, fonts::semibold(ui.ctx())),
                p.muted,
            );
            widgets::paint_elided(
                ui,
                &value,
                Pos2::new(tile.left() + 10.0, tile.top() + 34.0),
                w - 20.0,
                FontId::new(17.0, fonts::semibold(ui.ctx())),
                color,
            );
        }
        let count = self.tracker.payments().len();
        let since = self.tracker.since().map(crate::ui::chat::hhmm).unwrap_or_default();
        ui.label(
            egui::RichText::new(aurora_i18n::tr!("money-tracker-since", time = since, count = count))
                .size(11.5)
                .color(p.muted),
        );
        let in_count = self.tracker.payments().iter().filter(|x| x.received).count();
        let tabs = [
            (aurora_i18n::tr!("money-tracker-all"), count),
            (aurora_i18n::tr!("money-tracker-tab-received"), in_count),
            (aurora_i18n::tr!("money-tracker-tab-paid"), count - in_count),
        ];
        let filters = [Filter::All, Filter::Received, Filter::Paid];
        let mut chosen = filters.iter().position(|f| *f == self.filter).unwrap_or(0);
        if widgets::count_tabs(ui, p, "money-tracker-tab", &mut chosen, &tabs) {
            // a hidden row stays out of a copy or a removal
            self.filter = filters[chosen];
            self.selected.clear();
            self.anchor = None;
        }
    }

    /// The transactions shown, newest first, in `area`.
    #[allow(clippy::too_many_arguments)]
    fn list(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        world: &World,
        pics: &HashMap<Uuid, egui::TextureHandle>,
        area: Rect,
        now: f64,
        actions: &mut Vec<TrackerAction>,
    ) {
        let height = area.height();
        let shown: Vec<Payment> = self
            .tracker
            .payments()
            .iter()
            .rev()
            .filter(|x| self.filter.keeps(x))
            .cloned()
            .collect();
        let order: Vec<u64> = shown.iter().map(|x| x.serial).collect();
        let mut inner = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(area)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        if shown.is_empty() {
            inner.add_space(48.0);
            inner.vertical_centered(|ui| {
                ui.label(
                    egui::RichText::new(aurora_i18n::tr!("money-tracker-empty"))
                        .size(12.5)
                        .color(p.muted),
                );
                ui.label(
                    egui::RichText::new(aurora_i18n::tr!("money-tracker-empty-help"))
                        .size(12.5)
                        .color(p.muted),
                );
            });
        } else {
            egui::ScrollArea::vertical()
                .id_salt("money-tracker-list")
                .auto_shrink([false, false])
                .max_height(height)
                .show(&mut inner, |ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    for x in &shown {
                        self.row(ui, p, world, pics, x, &order, now, actions);
                    }
                });
        }
        self.undo_bar(ui, p, area);
    }

    #[allow(clippy::too_many_arguments)]
    fn row(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        world: &World,
        pics: &HashMap<Uuid, egui::TextureHandle>,
        x: &Payment,
        order: &[u64],
        now: f64,
        actions: &mut Vec<TrackerAction>,
    ) {
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
        let selected = self.selected.contains(&x.serial);
        let name = party_name(world, x);
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            let radius = ui.visuals().window_corner_radius;
            if selected {
                painter.rect_filled(rect, radius, p.violet.gamma_multiply(0.28));
            } else if resp.hovered() {
                painter.rect_filled(rect, radius, p.raised.gamma_multiply(0.6));
            }
            let mid = rect.center().y;
            let time = crate::ui::chat::hhmm(x.time);
            widgets::paint_elided(
                ui,
                &time,
                Pos2::new(rect.left() + 6.0, mid),
                TIME_W,
                FontId::proportional(11.5),
                p.muted,
            );
            let pic = Rect::from_center_size(
                Pos2::new(rect.left() + 6.0 + TIME_W + 9.0 + PICTURE / 2.0, mid),
                Vec2::splat(PICTURE),
            );
            match x.party {
                Party::Resident(id) => {
                    self.wanted_pics.insert(id);
                    widgets::avatar_picture(ui, p, pic, pics.get(&id), &name, false);
                }
                Party::Group(_) | Party::System => {
                    painter.rect_filled(pic, radius, p.raised);
                    let icon = match x.party {
                        Party::Group(_) => "users-group",
                        _ if x.kind == trans::UPLOAD_CHARGE => "upload",
                        _ => "coin-filled",
                    };
                    paint_icon(
                        painter,
                        icon,
                        Rect::from_center_size(pic.center(), Vec2::splat(14.0)),
                        p.violet_pale,
                    );
                }
            }
            let amount = signed_money(x);
            let color = if x.received { p.success } else { p.danger };
            let font = FontId::new(13.0, fonts::semibold(ui.ctx()));
            let galley = painter.layout_no_wrap(amount, font, color);
            let amount_left = rect.right() - 8.0 - galley.size().x;
            widgets::paint_row_text(painter, galley, amount_left, mid, color);
            let left = pic.right() + 9.0;
            let width = (amount_left - 10.0 - left).max(20.0);
            widgets::paint_elided(
                ui,
                &name,
                Pos2::new(left, mid - 8.0),
                width,
                FontId::new(13.0, fonts::semibold(ui.ctx())),
                p.ink,
            );
            // a fee: its name is already the type, its item or the system follows
            let detail = match (x.party, x.item.trim()) {
                (Party::System, "") => aurora_i18n::tr!("money-tracker-system").to_owned(),
                (Party::System, item) => item.to_owned(),
                _ => x.detail(),
            };
            widgets::paint_elided(ui, &detail, Pos2::new(left, mid + 9.0), width, FontId::proportional(11.5), p.muted);
        }
        let resp = resp.on_hover_text(format!("{} · {}", super::hhmmss(x.time), x.detail()));
        if resp.clicked() {
            let m = ui.input(|i| i.modifiers);
            self.click(x.serial, order, m.command, m.shift);
        }
        // Firestorm's list: the right click selects the row under it first
        if resp.secondary_clicked() && !selected {
            self.selected = std::collections::BTreeSet::from([x.serial]);
            self.anchor = Some(x.serial);
        }
        menu::context_menu(&resp, p, |ui| {
            match x.party {
                Party::Resident(id) => {
                    if menu::item(ui, p, "user-filled", aurora_i18n::tr!("money-tracker-profile")) {
                        actions.push(TrackerAction::Profile(id));
                    }
                    if menu::item(ui, p, "coin-filled", aurora_i18n::tr!("money-tracker-pay")) {
                        actions.push(TrackerAction::Pay { id, group: false });
                    }
                }
                Party::Group(id) => {
                    if menu::item(ui, p, "users-group", aurora_i18n::tr!("money-tracker-group-profile")) {
                        actions.push(TrackerAction::GroupProfile(id));
                    }
                    if menu::item(ui, p, "coin-filled", aurora_i18n::tr!("money-tracker-pay")) {
                        actions.push(TrackerAction::Pay { id, group: true });
                    }
                }
                Party::System => {}
            }
            if x.party != Party::System {
                menu::separator(ui, p);
            }
            if menu::item(ui, p, "copy-filled", aurora_i18n::tr!("money-tracker-copy-selection")) {
                let text = super::csv(&self.selection(), |x| party_name(world, x));
                ui.ctx().copy_text(text);
            }
            if menu::item(ui, p, "trash-filled", aurora_i18n::tr!("money-tracker-remove")) {
                self.delete_selected(now);
            }
        });
    }

    /// "3 rows removed · Undo", over the bottom of the list for a while.
    fn undo_bar(&mut self, ui: &mut egui::Ui, p: &Palette, area: Rect) {
        let Some(undo) = &self.undo else { return };
        let n = undo.payments.len();
        let bar = Rect::from_min_max(
            Pos2::new(area.left() + 6.0, area.bottom() - 42.0),
            Pos2::new(area.right() - 6.0, area.bottom() - 6.0),
        );
        ui.painter().rect_filled(bar, ui.visuals().window_corner_radius, p.raised);
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(bar.shrink2(Vec2::new(12.0, 0.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        child.label(
            egui::RichText::new(aurora_i18n::tr!("money-tracker-removed", count = n))
                .size(12.5)
                .color(p.ink),
        );
        child.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let undo = egui::RichText::new(aurora_i18n::tr!("money-tracker-undo"))
                .size(12.5)
                .strong()
                .color(p.violet_pale);
            if ui.add(egui::Label::new(undo).sense(Sense::click())).clicked() {
                self.undo();
            }
        });
    }

    /// "Track when closed", Copy and Clear, laid out from the bottom up in a
    /// bottom-up `ui`.
    fn footer(&mut self, ui: &mut egui::Ui, p: &Palette, world: &World, always: bool, now: f64, actions: &mut Vec<TrackerAction>) {
        ui.horizontal(|ui| {
            let selection = self.selection();
            let label = if selection.is_empty() {
                aurora_i18n::tr!("money-tracker-copy-all").to_owned()
            } else {
                aurora_i18n::tr!("money-tracker-copy-some", count = selection.len())
            };
            let all: Vec<&Payment> = self.tracker.payments().iter().collect();
            let rows = if selection.is_empty() { all } else { selection };
            if widgets::action_button_if(
                ui,
                p,
                "copy-filled",
                &label,
                ActionKind::Flat,
                Vec2::new(0.0, 28.0),
                !rows.is_empty(),
            )
            .clicked()
            {
                let text = super::csv(&rows, |x| party_name(world, x));
                ui.ctx().copy_text(text);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let any = !self.tracker.payments().is_empty();
                if widgets::action_button_if(
                    ui,
                    p,
                    "trash-filled",
                    aurora_i18n::tr!("money-tracker-clear"),
                    ActionKind::Flat,
                    Vec2::new(0.0, 28.0),
                    any,
                )
                .clicked()
                {
                    self.clear_all(now);
                }
            });
        });
        let mut track = always;
        if widgets::checkbox(ui, p, &mut track, aurora_i18n::tr!("money-tracker-always"))
            .on_hover_text(aurora_i18n::tr!("money-tracker-always-help"))
            .changed()
        {
            actions.push(TrackerAction::AlwaysTrack(track));
        }
        widgets::divider(ui, p);
    }
}
