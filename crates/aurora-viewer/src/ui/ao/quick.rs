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

/// An icon button lit while `on`: the theme's faces (`bare` false: the
/// toolbar panel), or bare on the glass with a face under the pointer and
/// the accent's face while on (the mini strip). Its name in its tooltip.
#[allow(clippy::too_many_arguments)]
pub(super) fn icon_toggle(
    ui: &mut egui::Ui,
    p: &Palette,
    icon: &str,
    tip: &str,
    on: bool,
    enabled: bool,
    size: Vec2,
    bare: bool,
) -> egui::Response {
    if !bare {
        let kind = if on {
            widgets::ActionKind::Primary
        } else {
            widgets::ActionKind::Flat
        };
        return widgets::action_button_if(ui, p, icon, "", kind, size, enabled).on_hover_text(tip);
    }
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

/// Previous, auto cycle, next, after the set selector, each button of `size`
/// (`bare`: as `icon_toggle` draws them).
#[allow(clippy::too_many_arguments)]
pub(super) fn cycle_buttons(
    ui: &mut egui::Ui,
    p: &Palette,
    set: Option<&Set>,
    inv: &Inventory,
    runtime: &mut Runtime,
    enabled: bool,
    busy: bool,
    size: Vec2,
    bare: bool,
    out: &mut Vec<Action>,
) {
    let state = target(runtime);
    let label = state.label();
    ui.spacing_mut().item_spacing.x = 4.0;
    let steps = set.and_then(|s| s.animation_config(state)).map_or(0, |c| c.steps.len());
    // Like AOEngine::cycle, stepping needs the AO on; it works with or
    // without auto cycle.
    let step = enabled && steps > 1;
    let mut stepped = |ui: &mut egui::Ui, icon: &str, tip: String, forward: bool| {
        if icon_toggle(ui, p, icon, &tip, false, step, size, bare).clicked()
            && let Some(set) = set
        {
            runtime.step(set, state, forward);
        }
    };
    stepped(ui, "caret-left-filled", aurora_i18n::tr!("ao-quick-previous", state = label), false);
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
    if icon_toggle(ui, p, "repeat", &tip, on, !busy && config.is_some(), size, bare).clicked()
        && let Some(config) = config
    {
        let mut config = config.clone();
        config.cycle = toggled(&config.cycle);
        match inventory::save_cycle(inv, state, &config) {
            Ok(change) => out.push(Action::Edit(change, None)),
            Err(reason) => log::warn!("AO quick cycle: {reason}"),
        }
    }
    stepped(ui, "caret-right-filled", aurora_i18n::tr!("ao-quick-next", state = label), true);
}

/// Shared compact AO controls for the popup and persistent mini window.
/// Differences with the Legacy Viewer's `animation_overrider_panel_small`
/// (FloaterAO, indra/newview/ao.cpp), which has only the set list, Previous /
/// Next, Sits and More: the auto-cycle button, the "now playing" line and the
/// Enabled, Swimming and Water walk switches are Aurora's.
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
    // A failed save of the Sits switch stays shown under the controls, as the
    // manager shows its `state.message`, until a later save succeeds.
    let message_id = egui::Id::unique((combo_id, "message"));
    ui.label(aurora_i18n::tr!("ao-set"));
    ui.horizontal(|ui| {
        let buttons = 3.0 * 30.0 + 2.0 * 4.0 + ui.spacing().item_spacing.x;
        ui.add_enabled_ui(!busy && !catalog.sets.is_empty(), |ui| {
            let selected = catalog.selected(account.selected);
            *list_open = egui::ComboBox::from_id_salt(combo_id)
                .width(ui.available_width() - buttons)
                .selected_text(selected.map_or(aurora_i18n::tr!("common-none-2"), |s| &s.name))
                .show_ui(ui, |ui| {
                    for set in &catalog.sets {
                        if ui.selectable_label(selected.is_some_and(|s| s.id == set.id), &set.name).clicked() && account.selected != set.id
                        {
                            account.selected = set.id;
                            out.push(Action::Preferences);
                        }
                    }
                })
                .inner
                .is_some();
        });
        let set = catalog.selected(account.selected);
        let size = Vec2::new(30.0, ui.spacing().interact_size.y);
        cycle_buttons(ui, p, set, inv, runtime, account.enabled, busy, size, false, out);
    });
    if super::enabled(ui, p, runtime, account) {
        out.push(Action::Preferences);
    }
    let set = catalog.selected(account.selected);
    if let Some(set) = set {
        let mut sits = set.sits;
        let changed = ui.add_enabled_ui(!busy, |ui| super::toggle(ui, p, "ao-sits", &mut sits)).inner;
        if changed {
            save_sits(ui, inv, set, sits, message_id, out);
        }
    }
    if let Some(message) = ui.data(|d| d.get_temp::<String>(message_id)) {
        ui.label(RichText::new(message).color(p.warn));
    }
    if let Some((state, name)) = now_playing(set, runtime) {
        ui.label(aurora_i18n::tr!("ao-now-playing", state = state.label(), name = name));
    } else {
        let state = runtime.current.unwrap_or(State::Standing);
        ui.label(aurora_i18n::tr!("ao-now-playing-none", state = state.label()));
    }
    let ready = swimming_guard(catalog, account, prefs);
    let shown = prefs.swimming && ready;
    let mut swimming = shown;
    ui.add_enabled_ui(ready, |ui| {
        widgets::switch_labeled(ui, p, &mut swimming, aurora_i18n::tr!("movement-swimming"));
    });
    if swimming != shown {
        prefs.set_water_mode(false, swimming, ready);
    }
    let mut walking = prefs.water_walk;
    widgets::switch_labeled(ui, p, &mut walking, aurora_i18n::tr!("movement-water-walk"));
    if walking != prefs.water_walk {
        prefs.set_water_mode(true, walking, ready);
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
                    ui.set_width(310.0);
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                    // Tighter rows than the manager: a glance over the world.
                    ui.spacing_mut().item_spacing.y = 3.0;
                    ui.label(RichText::new(aurora_i18n::tr!("ao-title")).strong());
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
                    ui.add_space(3.0);
                    if ui.button(aurora_i18n::tr!("ao-quick-manager")).clicked() {
                        manager_requested = true;
                        widgets::open_floater(ctx, "ao", manager_open);
                        ui.close();
                    }
                    // Hidden while the mini window is already shown; on a
                    // minimized one it expands it (LLFloaterReg::showInstance),
                    // as Settings does for the manager.
                    let mini_shown = account.mini_open && !widgets::floater_minimized(ctx, mini::WINDOW);
                    if !mini_shown && ui.button(aurora_i18n::tr!("ao-quick-pin")).clicked() {
                        widgets::open_floater(ctx, mini::WINDOW, &mut account.mini_open);
                        ui.close();
                    }
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
