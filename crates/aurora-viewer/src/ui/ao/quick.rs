//! Toolbar AO controls, reusing the manager's account and movement preferences.
//! The Legacy Viewer's small AO panel also shares its set selector with the
//! manager and steps the current state's animations with its previous / next
//! buttons (indra/newview/ao.cpp, panel_ao.xml `ao_*_small`); Aurora
//! adds an auto-cycle button between them and the requested water controls.
//! The run / flight boosts stay in Settings › Movement only.
use super::*;
use crate::movement::Preferences;

/// Switching auto cycle on from the panel without an interval arms this one:
/// at 0 s a stand would only change on its next entry, never while standing.
const QUICK_CYCLE_SECONDS: f32 = 30.0;
/// The toolbar panel's width, inside its margins.
const PANEL_WIDTH: f32 = 300.0;

/// The auto-cycle button flips the state's `:CY` option, the manager's
/// "Switch automatically" (AOEngine::setCycle); random order is kept.
pub(super) fn toggled(cycle: &Cycle) -> Cycle {
    let mut cycle = cycle.clone();
    cycle.cycle = !cycle.cycle;
    if cycle.cycle && cycle.seconds <= 0.0 {
        cycle.seconds = QUICK_CYCLE_SECONDS;
    }
    cycle
}

/// The state the buttons act on: the one playing, as AOEngine::cycle uses
/// mLastMotion, or Standing before the first motion arrives.
fn target(runtime: &Runtime) -> State {
    runtime.current.unwrap_or(State::Standing)
}

/// The name of the selected step in the state currently driven by the AO.
pub(super) fn now_playing<'a>(set: Option<&'a Set>, runtime: &Runtime) -> Option<(State, &'a str)> {
    let state = runtime.current?;
    let config = set?.animation_config(state)?;
    let index = runtime.index(state, config.folder);
    config.steps.get(index).map(|step| (state, step.name.as_str()))
}

/// An icon button bare on the glass, a face under the pointer and the
/// accent's face while `on` (the mini strip). Its name in its tooltip.
pub(super) fn icon_toggle(ui: &mut egui::Ui, p: &Palette, icon: &str, tip: &str, on: bool, enabled: bool, size: Vec2) -> egui::Response {
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let hot = response.hovered() && enabled;
    let radius = ui.visuals().widgets.inactive.corner_radius;
    if on {
        ui.painter().rect_filled(rect, radius, if hot { p.violet_light } else { p.violet });
    } else if hot {
        ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.7));
    }
    let ink = if !enabled {
        p.muted_dim
    } else if on {
        p.on_accent
    } else if hot {
        p.ink
    } else {
        p.muted
    };
    widgets::paint_glyph(ui.painter(), icon, rect.center(), 15.0, ink);
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, enabled, on, tip));
    response.on_hover_text(tip)
}

/// How one of the transport's buttons is drawn: its icon, its word (the
/// cycle's time, or None), its tooltip, lit, enabled.
pub(super) type Draw<'a> = dyn FnMut(&mut egui::Ui, &str, Option<&str>, &str, bool, bool) -> egui::Response + 'a;

/// Previous, auto cycle, next, after the set selector, each drawn by `draw`.
#[allow(clippy::too_many_arguments)]
pub(super) fn cycle_buttons(
    ui: &mut egui::Ui,
    set: Option<&Set>,
    inv: &Inventory,
    runtime: &mut Runtime,
    enabled: bool,
    busy: bool,
    draw: &mut Draw<'_>,
    out: &mut Vec<Action>,
) {
    let state = target(runtime);
    let label = state.label();
    let steps = set.and_then(|s| s.animation_config(state)).map_or(0, |c| c.steps.len());
    // Like AOEngine::cycle, stepping needs the AO on; it works with or
    // without auto cycle.
    let step = enabled && steps > 1;
    let tip = aurora_i18n::tr!("ao-quick-previous", state = label);
    if draw(ui, "caret-left-filled", None, &tip, false, step).clicked()
        && let Some(set) = set
    {
        runtime.step(set, state, false);
    }
    let config = set.and_then(|s| s.states.get(&state)).filter(|c| !c.folder.is_nil());
    let on = config.is_some_and(|c| c.cycle.cycle);
    let seconds = config.map_or(
        QUICK_CYCLE_SECONDS,
        |c| if on { c.cycle.seconds } else { toggled(&c.cycle).seconds },
    );
    let tip = if seconds > 0.0 {
        let decimals = if seconds.fract() == 0.0 { 0 } else { 1 };
        aurora_i18n::tr!(
            "ao-quick-cycle",
            state = label,
            seconds = aurora_i18n::number(f64::from(seconds), decimals)
        )
    } else {
        aurora_i18n::tr!("ao-quick-cycle-entry", state = label)
    };
    // its word: the time it switches after, Auto at each start, or Off
    let word = if !on {
        aurora_i18n::tr!("ao-cycle-off").to_owned()
    } else {
        time_word(seconds)
    };
    if draw(ui, "repeat", Some(&word), &tip, on, !busy && config.is_some()).clicked()
        && let Some(config) = config
    {
        let mut config = config.clone();
        config.cycle = toggled(&config.cycle);
        match inventory::save_cycle(inv, state, &config) {
            Ok(change) => out.push(Action::Edit(change, None)),
            Err(reason) => log::warn!("AO quick cycle: {reason}"),
        }
    }
    let tip = aurora_i18n::tr!("ao-quick-next", state = label);
    if draw(ui, "caret-right-filled", None, &tip, false, step).clicked()
        && let Some(set) = set
    {
        runtime.step(set, state, true);
    }
}

/// A cycle time in a word: Auto at 0 (the next one at each start), else in
/// seconds or minutes.
pub(super) fn time_word(seconds: f32) -> String {
    if seconds <= 0.0 {
        aurora_i18n::tr!("ao-stop-auto").to_owned()
    } else if seconds >= 60.0 && (seconds % 60.0).abs() < f32::EPSILON {
        aurora_i18n::tr!("ao-stop-minutes", count = (seconds / 60.0) as i64)
    } else {
        aurora_i18n::tr!("ao-stop-seconds", count = seconds.round() as i64)
    }
}

/// The toolbar panel's body, a card after the mockup's (Apple's Now Playing
/// and Control Center): the set and the AO's power, what plays in large
/// type with its countdown, the transport, and the tiles. Differences with
/// the Legacy Viewer's `animation_overrider_panel_small` (FloaterAO,
/// indra/newview/ao.cpp), which has only the set list, Previous / Next, Sits
/// and More: the auto-cycle button, what plays, the power and the Swimming
/// and Water walk tiles are Aurora's.
#[allow(clippy::too_many_arguments)]
pub(super) fn body(
    ui: &mut egui::Ui,
    p: &Palette,
    catalog: &inventory::Catalog,
    inv: &Inventory,
    runtime: &mut Runtime,
    account: &mut Account,
    prefs: &mut Preferences,
    busy: bool,
    combo_id: &'static str,
    list_open: &mut bool,
    out: &mut Vec<Action>,
) {
    // A failed save of Override sits stays shown under the controls, as the
    // manager shows its `state.message`, until a later save succeeds.
    let message_id = egui::Id::unique((combo_id, "message"));
    let width = ui.available_width();
    let gap = 6.0;
    // the set, and the power
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        let picker = Vec2::new(width - CONTROL - gap, CONTROL);
        *list_open = set_picker(ui, p, catalog, account, busy, picker, out);
        let tip = aurora_i18n::tr!("ao-enabled");
        if panel_button(ui, p, "power", None, tip, account.enabled, true, Vec2::splat(CONTROL)).clicked() {
            account.enabled = !account.enabled;
            if account.enabled {
                runtime.stands = true;
                runtime.paused = false;
            }
            out.push(Action::Preferences);
        }
    });
    let set = catalog.selected(account.selected);
    playing_card(ui, p, set, runtime, account);
    // the transport, three alike
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        let size = Vec2::new((width - 2.0 * gap) / 3.0, 34.0);
        let mut draw = |ui: &mut egui::Ui, icon: &str, word: Option<&str>, tip: &str, on: bool, enabled: bool| {
            panel_button(ui, p, icon, word, tip, on, enabled, size)
        };
        cycle_buttons(ui, set, inv, runtime, account.enabled, busy, &mut draw, out);
    });
    // the tiles
    let ready = swimming_guard(catalog, account, prefs);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        let size = Vec2::new((width - 2.0 * gap) / 3.0, 58.0);
        let sits = set.is_some_and(|s| s.sits);
        if tile(
            ui,
            p,
            "armchair",
            aurora_i18n::tr!("ao-sits"),
            aurora_i18n::tr!("ao-sits"),
            sits,
            !busy && set.is_some(),
            false,
            size,
        )
        .clicked()
            && let Some(set) = set
        {
            save_sits(ui, inv, set, !sits, message_id, out);
        }
        let shown = prefs.swimming && ready;
        let tip = if ready {
            aurora_i18n::tr!("movement-swimming")
        } else if catalog.failed {
            aurora_i18n::tr!("ao-fetch-failed")
        } else if catalog.loading {
            aurora_i18n::tr!("ao-loading")
        } else {
            aurora_i18n::tr!("ao-swim-required")
        };
        let warn = !ready && !catalog.loading && !catalog.failed;
        if tile(
            ui,
            p,
            "swimming",
            aurora_i18n::tr!("ao-tile-swimming"),
            tip,
            shown,
            ready,
            warn,
            size,
        )
        .clicked()
        {
            prefs.set_water_mode(false, !shown, ready);
        }
        let walking = prefs.water_walk;
        let label = aurora_i18n::tr!("ao-tile-water-walk");
        if tile(
            ui,
            p,
            "ripple",
            label,
            aurora_i18n::tr!("movement-water-walk"),
            walking,
            true,
            false,
            size,
        )
        .clicked()
        {
            prefs.set_water_mode(true, !walking, ready);
        }
    });
    if let Some(message) = ui.data(|d| d.get_temp::<String>(message_id)) {
        ui.label(RichText::new(message).color(p.warn));
    }
    if !ready {
        if catalog.failed {
            ui.label(RichText::new(aurora_i18n::tr!("ao-fetch-failed")).color(p.warn));
        } else if catalog.loading {
            ui.label(RichText::new(aurora_i18n::tr!("ao-loading")).color(p.muted));
        } else {
            super::swimming_warning(ui, p);
        }
    }
}

/// The height of the set picker and of the power button.
const CONTROL: f32 = 32.0;

/// The set in use as a wide picker: the login set's star, its name, a
/// chevron; a click lists the sets in the order "Your sets" gives them.
/// Returns whether its list is open.
fn set_picker(
    ui: &mut egui::Ui,
    p: &Palette,
    catalog: &inventory::Catalog,
    account: &mut Account,
    busy: bool,
    size: Vec2,
    out: &mut Vec<Action>,
) -> bool {
    let enabled = !busy && !catalog.sets.is_empty();
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let hot = response.hovered() && enabled;
    let radius = ui.visuals().widgets.inactive.corner_radius;
    ui.painter()
        .rect_filled(rect, radius, if hot { p.raised } else { p.raised.gamma_multiply(0.8) });
    let selected = catalog.selected(account.selected);
    let name = selected.map_or_else(|| aurora_i18n::tr!("common-none-2").to_owned(), |s| s.name.clone());
    let y = rect.center().y;
    let mut x = rect.left() + 10.0;
    if selected.is_some_and(|s| s.default) {
        widgets::paint_glyph(ui.painter(), "star-filled", egui::pos2(x + 6.0, y), 13.0, p.amber);
        x += 20.0;
    }
    let ink = if enabled { p.ink } else { p.muted };
    widgets::paint_elided(
        ui,
        &name,
        egui::pos2(x, y),
        rect.right() - x - 26.0,
        egui::FontId::proportional(13.0),
        ink,
    );
    widgets::paint_glyph(ui.painter(), "chevron-down", egui::pos2(rect.right() - 14.0, y), 14.0, p.muted);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, enabled, &name));
    let response = response.on_hover_text(aurora_i18n::tr!("ao-set"));
    let sets = account.ordered(&catalog.sets);
    super::super::menu::dropdown(&response, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
        for set in sets {
            let icon = if set.id == account.selected { "check" } else { "" };
            let tag = if set.default { aurora_i18n::tr!("ao-loads-at-login") } else { "" };
            if super::super::menu::item_tag(ui, p, icon, &set.name, tag) && account.selected != set.id {
                account.selected = set.id;
                out.push(Action::Preferences);
            }
        }
    })
}

/// What plays now, in large type on a dark pane: a live dot with the state
/// and its place, the animation's name, and, on a timer, how often it
/// switches and the time left on a track. With the AO off it says so.
fn playing_card(ui: &mut egui::Ui, p: &Palette, set: Option<&Set>, runtime: &Runtime, account: &Account) {
    let playing = now_playing(set, runtime).filter(|_| account.enabled);
    let state = target(runtime);
    let config = set.and_then(|s| s.animation_config(state));
    let timed = config
        .filter(|c| account.enabled && c.cycle.cycle && c.cycle.seconds > 0.0)
        .map(|c| (c.cycle.seconds, runtime.next_switch(state, c.folder)));
    let height = if timed.is_some() { 82.0 } else { 58.0 };
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), egui::Sense::hover());
    let radius = ui.visuals().widgets.inactive.corner_radius;
    ui.painter().rect_filled(rect, radius, p.bar);
    let inner = rect.shrink2(Vec2::new(12.0, 10.0));
    let eyebrow_y = inner.top() + 6.0;
    let mut x = inner.left();
    let live = playing.is_some() && !runtime.paused;
    if live {
        ui.painter().circle_filled(egui::pos2(x + 3.0, eyebrow_y), 3.0, p.teal);
        x += 12.0;
    }
    let eyebrow = if !account.enabled {
        aurora_i18n::tr!("ao-status-off").to_owned()
    } else {
        let place = config
            .filter(|c| c.steps.len() > 1)
            .map(|c| {
                format!(
                    " · {} / {}",
                    runtime.index(state, c.folder).min(c.steps.len() - 1) + 1,
                    c.steps.len()
                )
            })
            .unwrap_or_default();
        format!("{}{place}", state.label())
    };
    let small = egui::FontId::new(11.0, super::super::fonts::semibold(ui.ctx()));
    widgets::paint_elided(
        ui,
        &eyebrow.to_uppercase(),
        egui::pos2(x, eyebrow_y),
        inner.right() - x,
        small,
        p.muted,
    );
    let (name, ink, font) = match playing {
        Some((_, name)) => (
            name.to_owned(),
            p.ink,
            egui::FontId::new(15.0, super::super::fonts::semibold(ui.ctx())),
        ),
        None => (
            aurora_i18n::tr!("ao-mini-default").to_owned(),
            p.muted,
            egui::FontId::proportional(14.0),
        ),
    };
    widgets::paint_elided(ui, &name, egui::pos2(inner.left(), eyebrow_y + 21.0), inner.width(), font, ink);
    if let Some((seconds, next)) = timed {
        let meta_y = inner.bottom() - 13.0;
        let quiet = egui::FontId::proportional(11.5);
        let every = aurora_i18n::tr!("ao-switches-every", time = time_word(seconds));
        widgets::paint_elided(
            ui,
            &every,
            egui::pos2(inner.left(), meta_y),
            inner.width() * 0.6,
            quiet.clone(),
            p.muted,
        );
        let left = next.map(|at| at.saturating_duration_since(std::time::Instant::now()).as_secs_f32());
        if let Some(left) = left.filter(|_| live) {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
            let text = aurora_i18n::tr!("ao-next-in", seconds = left.ceil() as i64);
            let galley = ui.painter().layout_no_wrap(text, quiet, p.muted);
            let w = galley.size().x;
            widgets::paint_row_text(ui.painter(), galley, inner.right() - w, meta_y, p.muted);
        }
        // the time run on a track, from indigo to the accent
        let track = egui::Rect::from_min_max(egui::pos2(inner.left(), inner.bottom() - 3.0), inner.right_bottom());
        ui.painter().rect_filled(track, radius, p.raised);
        let run = left
            .filter(|_| live)
            .map_or(0.0, |left| 1.0 - (left / seconds.max(0.01)).clamp(0.0, 1.0));
        if run > 0.0 {
            let done = egui::Rect::from_min_max(track.min, egui::pos2(track.left() + track.width() * run, track.bottom()));
            let mut mesh = egui::Mesh::default();
            for (pos, color) in [
                (done.left_top(), p.indigo),
                (done.right_top(), p.violet),
                (done.right_bottom(), p.violet),
                (done.left_bottom(), p.indigo),
            ] {
                mesh.colored_vertex(pos, color);
            }
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            ui.painter().add(egui::Shape::mesh(mesh));
        }
    }
}

/// A button of the panel: its face (the accent's while on), its icon, and
/// its word after the icon when it has one.
#[allow(clippy::too_many_arguments)]
fn panel_button(
    ui: &mut egui::Ui,
    p: &Palette,
    icon: &str,
    word: Option<&str>,
    tip: &str,
    on: bool,
    enabled: bool,
    size: Vec2,
) -> egui::Response {
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let hot = response.hovered() && enabled;
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let face = match (on, enabled, hot) {
        (_, false, _) => p.raised.gamma_multiply(0.45),
        (true, true, true) => p.violet_light,
        (true, true, false) => p.violet,
        (false, true, true) => p.raised,
        (false, true, false) => p.raised.gamma_multiply(0.8),
    };
    ui.painter().rect_filled(rect, radius, face);
    let ink = if !enabled {
        p.muted_dim
    } else if on {
        p.on_accent
    } else {
        p.ink
    };
    match word {
        Some(word) => {
            let galley = ui.painter().layout_no_wrap(word.to_owned(), egui::FontId::proportional(12.0), ink);
            let total = 16.0 + 6.0 + galley.size().x;
            let x = rect.center().x - total / 2.0;
            widgets::paint_glyph(ui.painter(), icon, egui::pos2(x + 8.0, rect.center().y), 15.0, ink);
            widgets::paint_row_text(ui.painter(), galley, x + 22.0, rect.center().y, ink);
        }
        None => widgets::paint_glyph(ui.painter(), icon, rect.center(), 16.0, ink),
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, enabled, on, tip));
    response.on_hover_text(tip)
}

/// A tile you hit without aiming (Control Center): its icon over its name,
/// lit in the accent while on; one that cannot turn on is dimmed, with the
/// amber triangle when something is missing.
#[allow(clippy::too_many_arguments)]
fn tile(
    ui: &mut egui::Ui,
    p: &Palette,
    icon: &str,
    label: &str,
    tip: &str,
    on: bool,
    enabled: bool,
    warn: bool,
    size: Vec2,
) -> egui::Response {
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let hot = response.hovered() && enabled;
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let face = match (on, enabled, hot) {
        (_, false, _) => p.bar.gamma_multiply(0.8),
        (true, true, true) => p.violet_light,
        (true, true, false) => p.violet,
        (false, true, true) => p.raised,
        (false, true, false) => p.raised.gamma_multiply(0.7),
    };
    ui.painter().rect_filled(rect, radius, face);
    let ink = if !enabled {
        p.muted_dim
    } else if on {
        p.on_accent
    } else {
        p.muted
    };
    widgets::paint_glyph(ui.painter(), icon, egui::pos2(rect.center().x, rect.top() + 19.0), 18.0, ink);
    let mut job = egui::text::LayoutJob::single_section(label.to_owned(), egui::TextFormat::simple(egui::FontId::proportional(11.5), ink));
    job.halign = egui::Align::Center;
    job.wrap = egui::text::TextWrapping {
        max_width: rect.width() - 8.0,
        max_rows: 2,
        break_anywhere: false,
        overflow_character: Some('…'),
    };
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(egui::pos2(rect.center().x, rect.top() + 32.0), galley, ink);
    if warn {
        widgets::paint_glyph(
            ui.painter(),
            "alert-triangle-filled",
            rect.right_top() + Vec2::new(-10.0, 10.0),
            12.0,
            p.warn,
        );
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, enabled, on, label));
    response.on_hover_text(tip)
}

/// Saves the set's Override sits; a failure stays shown in the window under
/// `message_id` until a later save succeeds.
pub(super) fn save_sits(ui: &egui::Ui, inv: &Inventory, set: &Set, sits: bool, message_id: egui::Id, out: &mut Vec<Action>) {
    let mut edit = set.clone();
    edit.sits = sits;
    match inventory::save_set(inv, &edit) {
        Ok(change) => {
            out.push(Action::Edit(change, None));
            ui.data_mut(|d| d.remove::<String>(message_id));
        }
        Err(reason) => {
            log::warn!("AO compact sit override: {reason}");
            ui.data_mut(|d| d.insert_temp(message_id, reason));
        }
    }
}

/// Whether the selected set can swim. app/movement.rs's rule: only an AO
/// loaded without swimming animations turns swimming off. While it loads (at
/// login, when the mini window is drawn before the AO folder arrives) or
/// after a failed fetch, the setting stays: it was switched off and saved at
/// every login.
pub(super) fn swimming_guard(catalog: &inventory::Catalog, account: &Account, prefs: &mut Preferences) -> bool {
    let ready = catalog.swimming_ready(account.selected);
    if !ready && !catalog.loading && !catalog.failed {
        prefs.swimming = false;
    }
    ready
}

#[derive(Default)]
pub struct Quick {
    pub open: bool,
    anchor: Option<(egui::Response, egui::RectAlign, u64)>,
    list_was_open: bool,
}

impl Quick {
    pub fn anchor(&mut self, response: &egui::Response, align: egui::RectAlign) {
        if response.clicked() {
            self.open = !self.open;
        }
        self.anchor = Some((
            response.clone(),
            align,
            response.ctx.cumulative_frame_nr_for(egui::ViewportId::ROOT),
        ));
    }

    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
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
        let Some((anchor, align, frame)) = &self.anchor else { return out };
        if *frame != ctx.cumulative_frame_nr_for(egui::ViewportId::ROOT) {
            self.open = false;
            self.list_was_open = false;
            return out;
        }
        let before = prefs.clone();
        let was_open = self.open;
        let mut list_open = false;
        let mut manager_requested = false;
        let shown = egui::Popup::from_response(anchor)
            .open_bool(&mut self.open)
            .align(if *align == egui::RectAlign::TOP {
                egui::RectAlign::TOP_END
            } else {
                *align
            })
            .gap(7.0)
            .frame(egui::Frame::NONE)
            .close_behavior(egui::PopupCloseBehavior::IgnoreClicks)
            .show(|ui| {
                widgets::overlay(ui, p, egui::Margin::same(10), |ui| {
                    ui.set_width(PANEL_WIDTH);
                    ui.spacing_mut().item_spacing.y = 8.0;
                    // its title, the mini window's pin and Settings in its
                    // header (the mockup's toolbar panel)
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        ui.label(RichText::new(aurora_i18n::tr!("ao-title")).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let size = Vec2::splat(26.0);
                            if icon_toggle(ui, p, "settings-filled", aurora_i18n::tr!("ao-quick-manager"), false, true, size).clicked() {
                                manager_requested = true;
                                widgets::open_floater(ctx, "ao", manager_open);
                                ui.close();
                            }
                            // lit while the mini window is shown: a click
                            // then closes it; on a minimized one it expands it
                            // (LLFloaterReg::showInstance)
                            let mini_shown = account.mini_open && !widgets::floater_minimized(ctx, mini::WINDOW);
                            let tip = aurora_i18n::text(if mini_shown { "ao-mini-close" } else { "ao-quick-pin" });
                            if icon_toggle(ui, p, "pin-filled", tip, mini_shown, true, size).clicked() {
                                if mini_shown {
                                    account.mini_open = false;
                                } else {
                                    widgets::open_floater(ctx, mini::WINDOW, &mut account.mini_open);
                                    ui.close();
                                }
                            }
                        });
                    });
                    let mut set_list_open = false;
                    body(
                        ui,
                        p,
                        catalog,
                        inv,
                        runtime,
                        account,
                        prefs,
                        busy,
                        "ao_quick_set",
                        &mut set_list_open,
                        &mut out,
                    );
                    list_open = set_list_open;
                });
            });
        // A set combo opens a child popup. Keep the parent through selecting
        // a set or dismissing that child, as for the toolbar microphone panel.
        let in_list = list_open || self.list_was_open;
        if let Some(shown) = shown {
            if was_open && shown.response.clicked_elsewhere() && anchor.clicked_elsewhere() && !in_list {
                self.open = false;
            } else if !self.open && in_list && !manager_requested {
                self.open = true;
            }
        }
        self.list_was_open = self.open && list_open;
        prefs.sanitize();
        if *prefs != before {
            out.push(Action::MovementPreferences);
        }
        out
    }
}
