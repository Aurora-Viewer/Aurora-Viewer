//! Native, offline design study. The existing AO body is reused verbatim;
//! proposed motor settings stay in this transient model, never in the account.
use super::{AoUi, State, widgets};
use crate::{movement::Preferences, theme::Palette, world::PlayingAnimation};
use std::time::Instant;

pub struct Prototype {
    pub layout: usize,
    pub tab: usize,
    pub pose: State,
    pub movement: Preferences,
    signal: Option<PlayingAnimation>,
    passage: bool,
    workbench_open: bool,
}

impl Prototype {
    pub fn open_movement(&mut self) {
        self.layout = 1;
        self.tab = 1;
        self.workbench_open = true;
    }

    pub fn new(mode: &str) -> Self {
        Self {
            layout: if mode == "design-current" {
                0
            } else if mode == "design-section" {
                2
            } else {
                1
            },
            tab: usize::from(mode == "design-movement"),
            pose: if mode == "design-swim" {
                State::SwimmingForward
            } else {
                State::Standing
            },
            movement: Preferences::default(),
            signal: None,
            passage: false,
            workbench_open: true,
        }
    }

    pub fn signal(&mut self, now: Instant) -> PlayingAnimation {
        let id = self.pose.motion();
        let signal = PlayingAnimation::from_signal(id, 0, now, self.signal.as_ref().filter(|old| old.id == id));
        self.signal = Some(signal);
        signal
    }

    pub fn underwater(&self) -> bool {
        self.pose.swimming()
    }
}

pub fn workbench(ctx: &egui::Context, p: &Palette, state: &mut AoUi, ao_open: &mut bool) {
    let Some(prototype) = &mut state.prototype else { return };
    let mut open = prototype.workbench_open;
    widgets::Floater::new(
        "ao_design",
        aurora_i18n::tr!("ao-design-title"),
        egui::pos2(820.0, 120.0),
        egui::vec2(340.0, 360.0),
    )
    .auto_height()
    .show(ctx, p, &mut open, |ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
        ui.label(aurora_i18n::tr!("ao-design-help"));
        widgets::divider(ui, p);
        for (index, label) in ["ao-design-current", "ao-design-tabs", "ao-design-section"].into_iter().enumerate() {
            widgets::radio_value(ui, p, &mut prototype.layout, index, aurora_i18n::text(label));
        }
        widgets::divider(ui, p);
        ui.label(aurora_i18n::tr!("ao-design-avatar"));
        egui::ComboBox::from_id_salt("ao_design_pose")
            .selected_text(prototype.pose.label())
            .show_ui(ui, |ui| {
                for pose in [State::Standing, State::Walking, State::Running].into_iter().chain(State::SWIMMING) {
                    ui.selectable_value(&mut prototype.pose, pose, pose.label());
                }
            });
        if ui.button(aurora_i18n::tr!("ao-design-swim-animations")).clicked() {
            choose_swimming(prototype, &mut state.state);
            *ao_open = true;
        }
        ui.horizontal_wrapped(|ui| {
            if ui.button(aurora_i18n::tr!("ao-design-open")).clicked() {
                *ao_open = true;
            }
            if ui.button(aurora_i18n::tr!("ao-design-reset")).clicked() {
                prototype.movement = Preferences::default();
                prototype.passage = false;
            }
        });
        ui.label(egui::RichText::new(aurora_i18n::tr!("ao-design-session")).color(p.muted));
    });
    prototype.workbench_open = open;
}

fn choose_swimming(prototype: &mut Prototype, state: &mut State) {
    prototype.tab = 0;
    prototype.pose = State::SwimmingForward;
    *state = State::SwimmingForward;
}

pub fn before(ui: &mut egui::Ui, p: &Palette, state: &mut AoUi) -> bool {
    let Some(prototype) = &mut state.prototype else { return false };
    match prototype.layout {
        1 => {
            widgets::tabs(
                ui,
                p,
                &mut prototype.tab,
                &[
                    (aurora_i18n::tr!("ao-design-animations"), true),
                    (aurora_i18n::tr!("ao-design-movement"), true),
                ],
            );
            if prototype.tab == 1 {
                if movements(ui, p, prototype) {
                    choose_swimming(prototype, &mut state.state);
                }
                return true;
            }
        }
        2 => {
            let result = widgets::collapsing(
                ui,
                p,
                "ao_design_movement_section",
                aurora_i18n::tr!("movement-title"),
                true,
                |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("ao_design_movement_scroll")
                        .max_height(210.0)
                        .show(ui, |ui| movements(ui, p, prototype))
                        .inner
                },
            );
            if result.body == Some(true) {
                choose_swimming(prototype, &mut state.state);
            }
        }
        _ => {}
    }
    false
}

fn movements(ui: &mut egui::Ui, p: &Palette, prototype: &mut Prototype) -> bool {
    let prefs = &mut prototype.movement;
    let mut choose = false;
    ui.label(egui::RichText::new(aurora_i18n::tr!("ao-design-movement-help")).color(p.muted));
    widgets::divider(ui, p);
    widgets::switch_labeled(ui, p, &mut prefs.running, aurora_i18n::tr!("movement-running"));
    ui.add_enabled_ui(prefs.running, |ui| {
        ui.add(
            egui::Slider::new(&mut prefs.run_multiplier, 1.0..=5.0)
                .prefix("×")
                .step_by(0.1)
                .text(aurora_i18n::tr!("movement-run-multiplier")),
        );
    });
    ui.label(aurora_i18n::tr!("movement-run-help"));
    widgets::divider(ui, p);
    widgets::switch_labeled(ui, p, &mut prefs.fly_assist, aurora_i18n::tr!("movement-fly-assist"));
    ui.add_enabled_ui(prefs.fly_assist, |ui| {
        ui.add(
            egui::Slider::new(&mut prefs.fly_multiplier, 1.0..=5.0)
                .prefix("×")
                .step_by(0.1)
                .text(aurora_i18n::tr!("movement-fly-multiplier")),
        );
    });
    ui.label(aurora_i18n::tr!("movement-fly-help"));
    widgets::divider(ui, p);
    widgets::switch_labeled(ui, p, &mut prefs.swimming, aurora_i18n::tr!("movement-swimming"));
    ui.add_enabled_ui(prefs.swimming, |ui| {
        ui.add(
            egui::Slider::new(&mut prefs.swim_speed, 0.5..=8.0)
                .suffix(" m/s")
                .text(aurora_i18n::tr!("movement-swim-speed")),
        );
        ui.add(
            egui::Slider::new(&mut prefs.float_height, -0.5..=0.5)
                .suffix(" m")
                .step_by(0.01)
                .text(aurora_i18n::tr!("movement-float-height")),
        );
        widgets::checkbox(ui, p, &mut prefs.jump_obstacles, aurora_i18n::tr!("movement-jump-obstacles"));
    });
    ui.label(aurora_i18n::tr!("ao-design-animation-help"));
    if ui.button(aurora_i18n::tr!("ao-design-swim-animations")).clicked() {
        choose = true;
    }
    widgets::divider(ui, p);
    if widgets::action_button(
        ui,
        p,
        "door-enter",
        aurora_i18n::tr!("movement-pass"),
        widgets::ActionKind::Flat,
        egui::vec2(0.0, 28.0),
    )
    .clicked()
    {
        prototype.passage = true;
    }
    if prototype.passage {
        ui.label(aurora_i18n::tr!("ao-design-passage"));
    }
    prefs.sanitize();
    choose
}
