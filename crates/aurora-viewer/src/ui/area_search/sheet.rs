//! The filters of Area Search: the kinds of objects in three states, the
//! mouse click action, permissions, price, where and whose, and the search in
//! one field (the Filter, Advanced and Find tabs of floater_fs_area_search.xml).

use super::AreaSearchUi;
use super::model::{self, CLICK_ICONS, Kind, KindState, Scope};
use super::table::Filters;
use crate::theme::Palette;
use crate::ui::{board, widgets};
use egui::{Rect, RichText, Sense, Stroke, Vec2};

/// A tile for each kind, four a row: its state in words, colour and a mark.
/// `counts`: how many of the results are of each kind, shown on the tile.
pub(super) fn kinds(ui: &mut egui::Ui, p: &Palette, f: &mut Filters, counts: Option<&[u32; Kind::ALL.len()]>, height: f32) {
    let gap = 6.0;
    let width = ((ui.available_width() - 3.0 * gap) / 4.0).floor();
    for line in Kind::ALL.chunks(4) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for kind in line {
                let count = counts.map(|c| c[*kind as usize]);
                let state = kind.state(f);
                if tile(ui, p, *kind, state, count, Vec2::new(width, height)) {
                    kind.set(f, state.next());
                }
            }
        });
        ui.add_space(gap);
    }
}

fn tile(ui: &mut egui::Ui, p: &Palette, kind: Kind, state: KindState, count: Option<u32>, size: Vec2) -> bool {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let hovered = resp.hovered();
    let (face, stroke, ink) = match state {
        KindState::Any if hovered => (p.raised, Stroke::NONE, p.ink),
        KindState::Any => (p.raised.gamma_multiply(0.7), Stroke::NONE, p.ink),
        KindState::Only => (p.violet.gamma_multiply(0.3), Stroke::new(1.0, p.violet_light), p.ink),
        KindState::Hidden => (
            p.danger.gamma_multiply(0.14),
            Stroke::new(1.0, p.danger.gamma_multiply(0.8)),
            p.danger,
        ),
    };
    ui.painter()
        .rect(rect, radius, face, widgets::border_stroke(p, stroke), egui::StrokeKind::Inside);
    let scale = crate::ui::fonts::scale(ui.ctx());
    let word = match (state, count) {
        (KindState::Hidden, Some(_)) => Some(aurora_i18n::tr!("area-search-kind-hidden").to_owned()),
        (_, Some(n)) => Some(aurora_i18n::integer(i64::from(n))),
        _ => None,
    };
    let lines = if word.is_some() { 3.0 } else { 2.0 };
    let pitch = (rect.height() / lines).min(17.0 * scale);
    let mut y = rect.center().y - pitch * (lines - 1.0) / 2.0;
    crate::ui::sheet::icon_at(ui, kind.icon(), egui::pos2(rect.center().x, y), 15.0, ink);
    y += pitch;
    let drawn = centered(ui, kind.label(), rect.center().x, y, rect.width() - 8.0, 11.5, ink);
    if state == KindState::Hidden {
        ui.painter().hline(drawn, y + 0.5, Stroke::new(1.0, ink));
    }
    if let Some(word) = &word {
        let ink = if state == KindState::Hidden { ink } else { p.muted };
        centered(ui, word, rect.center().x, y + pitch, rect.width() - 8.0, 11.0, ink);
    }
    let mark = match state {
        KindState::Any => None,
        KindState::Only => Some(("check", p.violet)),
        KindState::Hidden => Some(("x", p.danger)),
    };
    if let Some((icon, color)) = mark {
        let at = egui::pos2(rect.right() - 11.0, rect.top() + 11.0);
        ui.painter().circle_filled(at, 7.0, color);
        crate::ui::sheet::icon_at(ui, icon, at, 10.0, p.on_accent);
    }
    let state_word = match state {
        KindState::Any => aurora_i18n::tr!("area-search-kind-any"),
        KindState::Only => aurora_i18n::tr!("area-search-kind-only"),
        KindState::Hidden => aurora_i18n::tr!("area-search-kind-hidden"),
    };
    let label = format!("{}: {state_word}", kind.label());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, state != KindState::Any, &label));
    resp.on_hover_text(aurora_i18n::tr!("area-search-kind-tip")).clicked()
}

/// A line of text in the middle of `x`; returns where it is.
fn centered(ui: &egui::Ui, text: &str, x: f32, y: f32, width: f32, size: f32, color: egui::Color32) -> egui::Rangef {
    let font = egui::FontId::proportional(size);
    let w = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font.clone(), color)
        .size()
        .x
        .min(width);
    // (a single figure is narrower than what an ellipsis needs)
    let room = (w + 1.0).max(size + 1.0);
    widgets::paint_elided(ui, text, egui::pos2(x - w / 2.0, y), room, font, color);
    egui::Rangef::new(x - w / 2.0, x + w / 2.0)
}

fn heading(ui: &mut egui::Ui, p: &Palette, text: &str, note: &str) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.set_min_height(18.0);
        let bold = crate::ui::fonts::semibold(ui.ctx());
        ui.label(RichText::new(text).size(11.5).family(bold).color(p.muted));
        if !note.is_empty() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(egui::Label::new(RichText::new(note).size(11.5).color(p.muted)).truncate());
            });
        }
    });
    ui.add_space(3.0);
}

/// Where to look: the parcel, the region, or its neighbours too.
pub(super) fn scope_choice(ui: &mut egui::Ui, p: &Palette, f: &mut Filters, scopes: &[Scope]) {
    let labels: Vec<_> = scopes.iter().map(|s| (s.label(), true)).collect();
    let chosen = Scope::of(f);
    // (one that makes no difference here is the region)
    let region = scopes.iter().position(|s| *s == Scope::Region).unwrap_or(0);
    let mut at = scopes.iter().position(|s| *s == chosen).unwrap_or(region);
    if widgets::segmented(ui, p, &mut at, &labels)
        && let Some(scope) = scopes.get(at)
    {
        scope.set(f);
    }
}

/// Everything but the buttons; `scopes`: where to look too, when there is a
/// choice.
pub(super) fn body(ui: &mut egui::Ui, p: &Palette, f: &mut Filters, scopes: &[Scope]) {
    ui.spacing_mut().item_spacing.y = 0.0;
    if scopes.len() > 1 {
        let reach = super::table::metres(model::REACH);
        heading(
            ui,
            p,
            aurora_i18n::tr!("area-search-scope-heading"),
            &aurora_i18n::tr!("area-search-within-across", distance = reach),
        );
        ui.horizontal(|ui| scope_choice(ui, p, f, scopes));
    }
    heading(
        ui,
        p,
        aurora_i18n::tr!("area-search-kind-heading"),
        aurora_i18n::tr!("area-search-kind-tip"),
    );
    kinds(ui, p, f, None, 50.0 * crate::ui::fonts::scale(ui.ctx()).max(1.0));
    heading(
        ui,
        p,
        aurora_i18n::tr!("area-search-click-action"),
        model::click_label(f.click_action),
    );
    click_row(ui, p, f);
    let wide = ui.available_width() > 460.0;
    if wide {
        ui.columns(2, |cols| {
            permissions(&mut cols[0], p, f);
            price(&mut cols[1], p, f);
        });
    } else {
        permissions(ui, p, f);
        price(ui, p, f);
    }
    heading(ui, p, aurora_i18n::tr!("area-search-objects-heading"), "");
    let mut switches = [
        (aurora_i18n::tr!("area-search-mine"), &mut f.mine),
        (aurora_i18n::tr!("area-search-linked"), &mut f.children),
    ];
    let columns = if wide { 2 } else { 1 };
    for line in switches.chunks_mut(columns) {
        ui.columns(columns, |cols| {
            for (ui, (label, on)) in cols.iter_mut().zip(line.iter_mut()) {
                switch_line(ui, p, label, on);
            }
        });
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.set_min_height(22.0);
        let bold = crate::ui::fonts::semibold(ui.ctx());
        let title = aurora_i18n::tr!("area-search-one-field");
        ui.label(RichText::new(title).size(11.5).family(bold).color(p.muted));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let label = RichText::new(aurora_i18n::tr!("area-search-regex-switch")).size(12.0).color(p.ink);
            widgets::switch_labeled(ui, p, &mut f.regex, label);
        });
    });
    fields(ui, p, f, if wide { 3 } else { 2 });
    f.normalize();
}

/// All, any action, then an icon each, the chosen one named over the row.
fn click_row(ui: &mut egui::Ui, p: &Palette, f: &mut Filters) {
    let (track, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::hover());
    let radius = ui.visuals().widgets.inactive.corner_radius;
    ui.painter().rect_filled(track, radius, p.field);
    let width = (track.width() - 4.0) / 10.0;
    for action in 0..10u8 {
        let min = egui::pos2(track.left() + 2.0 + f32::from(action) * width, track.top() + 2.0);
        let r = Rect::from_min_size(min, Vec2::new(width, track.height() - 4.0));
        let resp = ui.interact(r, ui.scope_id().with(("area_search_click", action)), Sense::click());
        let on = f.click_action == action;
        if on {
            let stroke = Stroke::new(1.0, p.violet_light);
            ui.painter()
                .rect(r, radius, p.violet.gamma_multiply(0.3), stroke, egui::StrokeKind::Inside);
        } else if resp.hovered() {
            ui.painter().rect_filled(r, radius, p.raised.gamma_multiply(0.6));
        }
        let ink = if on || resp.hovered() { p.ink } else { p.muted };
        let label = model::click_label(action);
        match action {
            0 => {
                centered(
                    ui,
                    aurora_i18n::tr!("area-search-click-all"),
                    r.center().x,
                    r.center().y,
                    width,
                    12.0,
                    ink,
                );
            }
            1 => {
                centered(
                    ui,
                    aurora_i18n::tr!("area-search-click-any-short"),
                    r.center().x,
                    r.center().y,
                    width,
                    12.0,
                    ink,
                );
            }
            _ => crate::ui::sheet::icon_at(ui, CLICK_ICONS[usize::from(action)], r.center(), 16.0, ink),
        }
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, on, label));
        if resp.on_hover_text(label).clicked() {
            f.click_action = action;
        }
    }
}

fn permissions(ui: &mut egui::Ui, p: &Palette, f: &mut Filters) {
    heading(ui, p, aurora_i18n::tr!("area-search-permissions"), "");
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::splat(4.0);
        for (label, on) in [
            (aurora_i18n::tr!("area-search-copy-perm"), &mut f.copy),
            (aurora_i18n::tr!("area-search-modify-perm"), &mut f.modify),
            (aurora_i18n::tr!("area-search-transfer-perm"), &mut f.transfer),
        ] {
            if board::pill(ui, p, label, *on) {
                *on = !*on;
            }
        }
    });
}

fn price(ui: &mut egui::Ui, p: &Palette, f: &mut Filters) {
    heading(ui, p, aurora_i18n::tr!("area-search-price-heading"), "");
    ui.add_enabled_ui(f.sale, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let size = Vec2::new(80.0, 26.0);
            ui.add_sized(size, egui::DragValue::new(&mut f.min_price).range(0..=999_999_999));
            ui.label(RichText::new(aurora_i18n::tr!("area-search-price-to")).size(12.0).color(p.muted));
            ui.add_sized(size, egui::DragValue::new(&mut f.max_price).range(0..=999_999_999));
            ui.label(RichText::new(aurora_i18n::currency()).size(12.0).color(p.muted));
        })
        .response
        .on_disabled_hover_text(aurora_i18n::tr!("area-search-price-tip"));
    });
}

fn switch_line(ui: &mut egui::Ui, p: &Palette, label: &str, on: &mut bool) {
    ui.horizontal(|ui| {
        ui.set_min_height(27.0);
        ui.add_space(6.0);
        let room = (ui.available_width() - 48.0).max(20.0);
        let text = egui::Label::new(RichText::new(label).size(12.5).color(p.ink)).truncate();
        let layout = egui::Layout::left_to_right(egui::Align::Center);
        let clicked = ui
            .allocate_ui_with_layout(Vec2::new(room, 20.0), layout, |ui| {
                ui.set_min_width(room);
                ui.add(text.sense(Sense::CLICK)).clicked()
            })
            .inner;
        let resp = widgets::switch(ui, p, on);
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, *on, label));
        if clicked {
            *on = !*on;
        }
    });
}

fn fields(ui: &mut egui::Ui, p: &Palette, f: &mut Filters, columns: usize) {
    let gap = 6.0;
    let width = ((ui.available_width() - gap * (columns as f32 - 1.0)) / columns as f32).floor();
    let mut list = [
        (aurora_i18n::tr!("inspect-col-item"), &mut f.name),
        (aurora_i18n::tr!("inspect-col-description"), &mut f.description),
        (aurora_i18n::tr!("inspect-col-owner"), &mut f.owner),
        (aurora_i18n::tr!("area-search-group"), &mut f.group),
        (aurora_i18n::tr!("inspect-col-creator"), &mut f.creator),
        (aurora_i18n::tr!("area-search-last-owner"), &mut f.last_owner),
    ];
    let mut at = 0;
    for line in list.chunks_mut(columns) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (label, text) in line {
                ui.vertical(|ui| {
                    ui.set_width(width);
                    ui.label(RichText::new(*label).size(11.5).color(p.muted));
                    ui.add_space(2.0);
                    widgets::filter_field(ui, p, text, egui::Id::unique(("area_search_field", at)), width);
                });
                at += 1;
            }
        });
        ui.add_space(4.0);
    }
}

impl AreaSearchUi {
    /// The buttons of the sheet: reset, save, copy as a link.
    pub(super) fn sheet_buttons(&mut self, ui: &mut egui::Ui, p: &Palette) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(6.0);
            let flat = widgets::ActionKind::Flat;
            let size = Vec2::new(0.0, 28.0);
            if widgets::action_button(ui, p, "", aurora_i18n::tr!("common-reset"), flat, size)
                .on_hover_text(aurora_i18n::tr!("area-search-reset-filters-tip"))
                .clicked()
            {
                self.table.filters.reset_all_but_query();
                self.table.owner = None;
            }
            if widgets::action_button(ui, p, "", aurora_i18n::tr!("area-search-save-defaults"), flat, size)
                .on_hover_text(aurora_i18n::tr!("area-search-save-defaults-tip"))
                .clicked()
            {
                self.save_defaults = true;
            }
            if widgets::action_button(ui, p, "", aurora_i18n::tr!("area-search-copy-link"), flat, size).clicked() {
                ui.ctx().copy_text(self.table.filters.link());
            }
        });
    }
}
