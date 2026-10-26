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

/// Previous, auto cycle, next, after the set selector.
#[allow(clippy::too_many_arguments)]
fn cycle_buttons(
    ui: &mut egui::Ui,
    p: &Palette,
    set: Option<&Set>,
    inv: &Inventory,
    runtime: &mut Runtime,
    enabled: bool,
    busy: bool,
    out: &mut Vec<Action>,
) {
    let state = target(runtime);
    let label = state.label();
    let size = Vec2::new(30.0, ui.spacing().interact_size.y);
    ui.spacing_mut().item_spacing.x = 4.0;
    let steps = set.and_then(|s| s.animation_config(state)).map_or(0, |c| c.steps.len());
    // Like AOEngine::cycle, stepping needs the AO on; it works with or
    // without auto cycle.
    let step = enabled && steps > 1;
    let mut stepped = |ui: &mut egui::Ui, icon: &str, tip: String, forward: bool| {
        if widgets::action_button_if(ui, p, icon, "", widgets::ActionKind::Flat, size, step)
            .on_hover_text(tip)
            .clicked()
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
    let kind = if on {
        widgets::ActionKind::Primary
    } else {
        widgets::ActionKind::Flat
    };
    if widgets::action_button_if(ui, p, "repeat", "", kind, size, !busy && config.is_some())
        .on_hover_text(tip)
        .clicked()
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
        self.anchor = Some((response.clone(), align, response.ctx.cumulative_frame_nr()));
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
        if *frame != ctx.cumulative_frame_nr() {
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
                    ui.label(aurora_i18n::tr!("ao-set"));
                    ui.horizontal(|ui| {
                        let buttons = 3.0 * 30.0 + 2.0 * 4.0 + ui.spacing().item_spacing.x;
                        ui.add_enabled_ui(!busy && !catalog.sets.is_empty(), |ui| {
                            let selected = catalog.selected(account.selected);
                            list_open = egui::ComboBox::from_id_salt("ao_quick_set")
                                .width(ui.available_width() - buttons)
                                .selected_text(selected.map_or(aurora_i18n::tr!("common-none-2"), |s| &s.name))
                                .show_ui(ui, |ui| {
                                    for set in &catalog.sets {
                                        if ui.selectable_label(selected.is_some_and(|s| s.id == set.id), &set.name).clicked()
                                            && account.selected != set.id
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
                        cycle_buttons(ui, p, set, inv, runtime, account.enabled, busy, &mut out);
                    });
                    if super::enabled(ui, p, runtime, account) {
                        out.push(Action::Preferences);
                    }
                    let ready = catalog.swimming_ready(account.selected);
                    if !ready {
                        prefs.swimming = false;
                    }
                    let mut swimming = prefs.swimming;
                    ui.add_enabled_ui(ready, |ui| {
                        widgets::switch_labeled(ui, p, &mut swimming, aurora_i18n::tr!("movement-swimming"));
                    });
                    if swimming != prefs.swimming {
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
                    ui.add_space(3.0);
                    if ui.button(aurora_i18n::tr!("ao-quick-manager")).clicked() {
                        manager_requested = true;
                        widgets::open_floater(ctx, "ao", manager_open);
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
