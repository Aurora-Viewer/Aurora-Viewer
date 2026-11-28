//! Persistent compact AO controls in one line, after the small mode of
//! FloaterAO (indra/newview/ao.cpp, `animation_overrider_panel_small` in
//! skins/default/xui/en/panel_ao.xml): the set list that switches the set at
//! once (`onSelectSetSmall`), Previous / Next and the Sits checkbox
//! (`onCheckOverrideSitsSmall`).
//!
//! The strip: a grip (the window moves by its background), what plays now
//! (a click opens the set list), then icon buttons with their names in
//! tooltips: AO on / off, Previous, auto cycle, Next, Override sits,
//! Swimming, Water walk and Settings. The toolbar panel keeps its own body
//! (`quick::body`).
//!
//! Differences with the Legacy Viewer:
//! - its small panel has only the set list, Previous / Next, Sits and More;
//!   the AO switch, the auto-cycle button, the "now playing" line and the
//!   Swimming and Water walk switches are Aurora's;
//! - it is one floater that switches between its small and full modes
//!   (`onClickMore` / `onClickLess`, remembered in `UseFullAOInterface`).
//!   Here the mini window is a window of its own, remembered per account
//!   (`Account::mini_open`): its Settings button opens the full AO window and
//!   the mini window stays open, so that both can be shown together.

use super::{Action, Floater, inventory, quick, widgets};
use crate::{
    ao::{Account, State, runtime::Runtime},
    movement::Preferences,
    theme::Palette,
    world::inventory::Inventory,
};
use egui::{RichText, Vec2};

/// The floater id of the mini window.
pub const WINDOW: &str = "ao_mini";
/// One icon button of the strip.
const BUTTON: Vec2 = Vec2::new(30.0, 30.0);
/// What plays now: its width and height, the toolbar dock's buttons'.
const FIELD: Vec2 = Vec2::new(176.0, 34.0);
/// The grip at its left end.
const GRIP: f32 = 12.0;
/// The room between two of its parts, and its margins: 4 above and under,
/// as the dock's, for its height (42).
const GAP: f32 = 3.0;
const MARGIN: f32 = 4.0;
/// The strip's width, from its parts; its height follows its line.
const WIDTH: f32 = 2.0 * MARGIN + GRIP + FIELD.x + 8.0 * BUTTON.x + 11.0 * GAP + 6.0;

#[allow(clippy::too_many_arguments)]
pub fn show(
    ctx: &egui::Context,
    p: &Palette,
    catalog: &inventory::Catalog,
    inv: &Inventory,
    runtime: &mut Runtime,
    account: &mut Account,
    prefs: &mut Preferences,
    manager_open: &mut bool,
    busy: bool,
) -> Vec<Action> {
    let mut out = Vec::new();
    if !account.mini_open {
        return out;
    }
    // The strip has no title bar to fold it: a window left minimized by the
    // old mini AO comes back whole.
    if widgets::floater_minimized(ctx, WINDOW) {
        widgets::unminimize_floater(ctx, WINDOW);
    }
    let before = prefs.clone();
    let mut open = account.mini_open;
    let mut close = false;
    // no close button: its right-click menu, its set list and the toolbar
    // panel's pin close it
    let mut floater = Floater::new(
        WINDOW,
        aurora_i18n::tr!("ao-mini-title"),
        egui::pos2(260.0, 420.0),
        Vec2::new(WIDTH, FIELD.y + 2.0 * MARGIN),
    )
    .headless()
    .bar_glass()
    .fixed()
    .margin(egui::Margin::same(MARGIN as i8));
    floater.closable = false;
    floater.show(ctx, p, &mut open, |ui| {
        strip(
            ui,
            p,
            catalog,
            inv,
            runtime,
            account,
            prefs,
            manager_open,
            busy,
            &mut close,
            &mut out,
        );
    });
    account.mini_open = open && !close;
    prefs.sanitize();
    if *prefs != before {
        out.push(Action::MovementPreferences);
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn strip(
    ui: &mut egui::Ui,
    p: &Palette,
    catalog: &inventory::Catalog,
    inv: &Inventory,
    runtime: &mut Runtime,
    account: &mut Account,
    prefs: &mut Preferences,
    manager_open: &mut bool,
    busy: bool,
    close: &mut bool,
    out: &mut Vec<Action>,
) {
    // A failed save of Override sits stays shown under the strip, as the
    // manager shows its `state.message`, until a later save succeeds.
    let message_id = egui::Id::unique(("ao_mini_set", "message"));
    let ready = quick::swimming_guard(catalog, account, prefs);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = GAP;
        // the grip, in the line's middle
        let (grip, _) = ui.allocate_exact_size(Vec2::new(GRIP, FIELD.y), egui::Sense::hover());
        widgets::paint_glyph(ui.painter(), "grip-vertical", grip.center(), 13.0, p.muted_dim);
        let field = now_playing(ui, p, catalog, runtime, account, busy, manager_open, close, out);
        strip_menu(&field, p, manager_open, close);
        let tip = aurora_i18n::text(if account.enabled { "ao-turn-off" } else { "ao-turn-on" });
        if toggle_button(ui, p, "power", tip, account.enabled, true).clicked() {
            account.enabled = !account.enabled;
            if account.enabled {
                runtime.stands = true;
                runtime.paused = false;
            }
            out.push(Action::Preferences);
        }
        let set = catalog.selected(account.selected);
        let mut draw = |ui: &mut egui::Ui, icon: &str, _word: Option<&str>, tip: &str, on: bool, enabled: bool| {
            quick::icon_toggle(ui, p, icon, tip, on, enabled, BUTTON)
        };
        quick::cycle_buttons(ui, set, inv, runtime, account.enabled, busy, &mut draw, out);
        let sits = set.is_some_and(|s| s.sits);
        if toggle_button(ui, p, "armchair", aurora_i18n::tr!("ao-sits"), sits, !busy && set.is_some()).clicked()
            && let Some(set) = set
        {
            quick::save_sits(ui, inv, set, !sits, message_id, out);
        }
        let shown = prefs.swimming && ready;
        // the reason swimming waits, as the toolbar panel says it
        let tip = if ready {
            aurora_i18n::tr!("movement-swimming")
        } else if catalog.failed {
            aurora_i18n::tr!("ao-fetch-failed")
        } else if catalog.loading {
            aurora_i18n::tr!("ao-loading")
        } else {
            aurora_i18n::tr!("ao-swim-required")
        };
        if toggle_button(ui, p, "swimming", tip, shown, ready).clicked() {
            prefs.set_water_mode(false, !shown, ready);
        }
        if toggle_button(ui, p, "walk", aurora_i18n::tr!("movement-water-walk"), prefs.water_walk, true).clicked() {
            prefs.set_water_mode(true, !prefs.water_walk, ready);
        }
        // Opens (or expands) the full AO window; the mini window stays
        // open, unlike FloaterAO::onClickMore (see the module comment).
        if toggle_button(ui, p, "settings-filled", aurora_i18n::tr!("ao-quick-manager"), false, true).clicked() {
            widgets::open_floater(ui.ctx(), "ao", manager_open);
        }
    });
    if let Some(message) = ui.data(|d| d.get_temp::<String>(message_id)) {
        ui.label(RichText::new(message).color(p.warn));
    }
    // a failed fetch is said under the strip, as the toolbar panel does
    if catalog.failed {
        ui.label(RichText::new(aurora_i18n::tr!("ao-fetch-failed")).color(p.warn));
    }
}

/// The strip's right-click menu: the full AO window, and closing the strip.
/// (An id of its own: a left click on the same field lists the sets.)
fn strip_menu(response: &egui::Response, p: &Palette, manager_open: &mut bool, close: &mut bool) {
    super::super::menu::context_menu_of_button(response, p, |ui| {
        if super::super::menu::item(ui, p, "settings-filled", aurora_i18n::tr!("ao-quick-manager")) {
            widgets::open_floater(ui.ctx(), "ao", manager_open);
        }
        if super::super::menu::item(ui, p, "x", aurora_i18n::tr!("ao-mini-close")) {
            *close = true;
        }
    });
}

/// An icon button of the strip: bare on its glass, lit while `on`.
fn toggle_button(ui: &mut egui::Ui, p: &Palette, icon: &str, tip: &str, on: bool, enabled: bool) -> egui::Response {
    quick::icon_toggle(ui, p, icon, tip, on, enabled, BUTTON)
}

/// The set and the state on a small line, the animation playing below; a
/// click lists the sets, to switch at once (FloaterAO::onSelectSetSmall).
#[allow(clippy::too_many_arguments)]
fn now_playing(
    ui: &mut egui::Ui,
    p: &Palette,
    catalog: &inventory::Catalog,
    runtime: &Runtime,
    account: &mut Account,
    busy: bool,
    manager_open: &mut bool,
    close: &mut bool,
    out: &mut Vec<Action>,
) -> egui::Response {
    let set = catalog.selected(account.selected);
    let (rect, response) = ui.allocate_exact_size(FIELD, egui::Sense::click());
    let radius = ui.visuals().widgets.inactive.corner_radius;
    // a pane of the glass, lit under the pointer
    let pane = if response.hovered() {
        p.raised.gamma_multiply(0.75)
    } else {
        p.field.gamma_multiply(0.45)
    };
    ui.painter().rect_filled(rect, radius, pane);
    let inner = rect.shrink2(Vec2::new(10.0, 0.0));
    let playing = quick::now_playing(set, runtime);
    let state = playing.map_or(runtime.current.unwrap_or(State::Standing), |(s, _)| s);
    let live = account.enabled && !runtime.paused && playing.is_some();
    let top = inner.top() + 9.5;
    ui.painter()
        .circle_filled(egui::pos2(inner.left() + 3.0, top), 3.0, if live { p.teal } else { p.muted_dim });
    let small = egui::FontId::new(9.5, super::super::fonts::semibold(ui.ctx()));
    let set_name = set.map_or_else(|| aurora_i18n::tr!("ao-no-set").to_owned(), |s| s.name.clone());
    let set_galley = widgets::elided_galley(
        ui,
        RichText::new(set_name.to_uppercase()).font(small.clone()).color(p.violet_light),
        inner.width() * 0.5,
    );
    let set_w = set_galley.size().x;
    widgets::paint_row_text(ui.painter(), set_galley, inner.left() + 12.0, top, p.violet_light);
    let detail = if !account.enabled {
        aurora_i18n::tr!("ao-status-off").to_owned()
    } else {
        let place = set
            .and_then(|s| s.animation_config(state))
            .filter(|c| c.steps.len() > 1)
            .map(|c| format!(" {} / {}", runtime.index(state, c.folder).min(c.steps.len() - 1) + 1, c.steps.len()))
            .unwrap_or_default();
        format!("{}{place}", state.label())
    };
    widgets::paint_elided(
        ui,
        &format!("· {}", detail.to_uppercase()),
        egui::pos2(inner.left() + 18.0 + set_w, top),
        inner.width() - set_w - 18.0,
        small,
        p.muted,
    );
    let name = match playing {
        Some((_, name)) if account.enabled => name.to_owned(),
        _ => aurora_i18n::tr!("ao-mini-default").to_owned(),
    };
    widgets::paint_elided(
        ui,
        &name,
        egui::pos2(inner.left(), inner.bottom() - 10.0),
        inner.width(),
        egui::FontId::new(12.5, super::super::fonts::semibold(ui.ctx())),
        p.ink,
    );
    let response = response.on_hover_text(aurora_i18n::tr!("ao-mini-sets-tip"));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, !busy, &name));
    if busy || catalog.sets.is_empty() {
        return response;
    }
    // in the order "Your sets" gives them
    let sets = account.ordered(&catalog.sets);
    super::super::menu::dropdown(&response, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
        for set in sets {
            let tag = if set.default {
                aurora_i18n::tr!("ao-loads-at-login")
            } else if set.swimming_ready() {
                aurora_i18n::tr!("ao-swim-ready-tag")
            } else {
                ""
            };
            let icon = if set.id == account.selected { "check" } else { "" };
            if super::super::menu::item_tag(ui, p, icon, &set.name, tag) && account.selected != set.id {
                account.selected = set.id;
                out.push(Action::Preferences);
            }
        }
        super::super::menu::separator(ui, p);
        if super::super::menu::item(ui, p, "settings-filled", aurora_i18n::tr!("ao-manage-sets")) {
            widgets::open_floater(ui.ctx(), "ao", manager_open);
        }
        if super::super::menu::item(ui, p, "x", aurora_i18n::tr!("ao-mini-close")) {
            *close = true;
        }
    });
    response
}
