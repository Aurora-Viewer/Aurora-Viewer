//! Target frame rate: the Legacy Viewer's Autotune (settings/autotune.rs),
//! as its Performance window has it (floater_fs_performance.xml: Target
//! Frame Rate, Start / Stop, Continuous, Tuning Strategy;
//! panel_fs_performance_autotune.xml: distance-based optimization, farthest
//! full avatar, minimum distance). The "preferred distance" is the user's own
//! draw distance here: Autotune never goes above it.

use super::draw;
use super::{Action, Monitor};
use crate::settings::autotune::{
    self, Config, IMPOSTOR_DISTANCE_MAX, IMPOSTOR_DISTANCE_MIN, MIN_DD_MAX, Status, Strategy, TARGET_FPS_MAX, TARGET_FPS_MIN, Tuned,
};
use crate::theme::Palette;
use crate::ui::widgets::{self, ActionKind};
use aurora_i18n::tr;
use egui::vec2;

/// What the window shows of Autotune, filled by the app each frame.
#[derive(Debug, Clone, PartialEq)]
pub struct View {
    pub config: Config,
    pub status: Status,
    /// The median frame time measured (ms).
    pub frame_ms: Option<f32>,
    /// The rate the loop is held to (frame limiter, vsync), 0 for none.
    pub held_to: f32,
    /// The values live, and the user's own.
    pub live: Tuned,
    pub user: Tuned,
}

impl Default for View {
    fn default() -> Self {
        let t = Tuned {
            draw_distance: 0.0,
            max_avatars: 0,
        };
        Self {
            config: Config::default(),
            status: Status::Off,
            frame_ms: None,
            held_to: 0.0,
            live: t,
            user: t,
        }
    }
}

/// What the section asks of the app.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    Start,
    /// Stop, the user's values back.
    Stop,
    /// After a run without "Continuous": the values found become the user's.
    Keep,
    /// A choice changed.
    Config(Config),
}

pub(super) fn status_text(status: Status) -> &'static str {
    match status {
        Status::Off => tr!("autotune-status-off"),
        Status::Waiting => tr!("autotune-status-waiting"),
        Status::Lowering => tr!("autotune-status-lowering"),
        Status::OnTarget => tr!("autotune-status-on-target"),
        Status::CannotReach => tr!("autotune-status-cannot-reach"),
        Status::Finished => tr!("autotune-status-finished"),
    }
}

fn status_color(p: &Palette, status: Status) -> egui::Color32 {
    match status {
        Status::Off => p.muted_dim,
        Status::Waiting => p.muted,
        Status::Lowering => p.warn,
        Status::OnTarget | Status::Finished => p.success,
        Status::CannotReach => p.danger,
    }
}

pub(super) fn show(ui: &mut egui::Ui, p: &Palette, m: &mut Monitor, out: &mut Vec<Action>) {
    let v = &m.autotune;
    let mut config = v.config.clone();
    let running = matches!(
        v.status,
        Status::Waiting | Status::Lowering | Status::OnTarget | Status::CannotReach
    );
    super::heading(ui, p, tr!("autotune-title"), tr!("autotune-sub"), |ui| {
        draw::pill(ui, p, status_color(p, v.status), status_text(v.status));
    });
    egui::ScrollArea::vertical()
        .id_salt("monitor-autotune")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            // the target and Start / Stop (floater_fs_performance.xml)
            ui.horizontal(|ui| {
                ui.label(tr!("autotune-target"));
                ui.add(
                    egui::Slider::new(&mut config.target_fps, TARGET_FPS_MIN..=TARGET_FPS_MAX)
                        .logarithmic(true)
                        .integer()
                        .suffix(tr!("autotune-fps-suffix")),
                )
                .on_hover_text(tr!("autotune-target-tip"));
                ui.add_space(8.0);
                if running {
                    if widgets::action_button(ui, p, "player-stop-filled", tr!("autotune-stop"), ActionKind::Flat, vec2(0.0, 0.0))
                        .on_hover_text(tr!("autotune-stop-tip"))
                        .clicked()
                    {
                        out.push(Action::Autotune(Request::Stop));
                    }
                } else if widgets::action_button(
                    ui,
                    p,
                    "player-play-filled",
                    tr!("autotune-start"),
                    ActionKind::Primary,
                    vec2(0.0, 0.0),
                )
                .on_hover_text(tr!("autotune-start-tip"))
                .clicked()
                {
                    out.push(Action::Autotune(Request::Start));
                }
            });
            widgets::switch_labeled(ui, p, &mut config.continuous, tr!("autotune-continuous"))
                .on_hover_text(tr!("autotune-continuous-tip"));
            if config.continuous {
                ui.indent("autotune-keep", |ui| {
                    widgets::checkbox(ui, p, &mut config.keep_continuous, tr!("autotune-keep-continuous"));
                });
            }
            let target = autotune::effective_target(&config, v.held_to);
            if target < config.target_fps as f32 {
                draw::text(
                    ui,
                    &tr!("autotune-capped", fps = aurora_i18n::number(f64::from(target), 0)),
                    draw::SMALL,
                    p.muted,
                );
            }

            // where it stands: the frame rate measured, the values live
            // against the user's
            ui.add_space(4.0);
            let fps = v.frame_ms.filter(|ms| *ms > 0.0).map(|ms| 1000.0 / ms);
            let rows = [
                (
                    tr!("autotune-measured"),
                    fps.map_or("–".into(), |f| {
                        tr!("autotune-fps-value", fps = aurora_i18n::number(f64::from(f), 0))
                    }),
                    tr!("autotune-of-target", fps = aurora_i18n::number(f64::from(target), 0)),
                ),
                (
                    tr!("autotune-draw-distance"),
                    tr!("autotune-metres", value = aurora_i18n::number(f64::from(v.live.draw_distance), 0)),
                    tr!(
                        "autotune-yours-metres",
                        value = aurora_i18n::number(f64::from(v.user.draw_distance), 0)
                    ),
                ),
                (
                    tr!("autotune-avatars-full"),
                    aurora_i18n::integer(i64::from(v.live.max_avatars)),
                    tr!("autotune-yours-count", value = aurora_i18n::integer(i64::from(v.user.max_avatars))),
                ),
            ];
            egui::Grid::new("autotune-rows")
                .num_columns(3)
                .spacing(vec2(16.0, 4.0))
                .show(ui, |ui| {
                    for (name, value, sub) in rows {
                        ui.label(egui::RichText::new(name).color(p.muted));
                        ui.label(egui::RichText::new(value).color(p.ink).strong());
                        ui.label(egui::RichText::new(sub).size(draw::SMALL).color(p.muted));
                        ui.end_row();
                    }
                });

            // a run without "Continuous" done: keep or give back
            if v.status == Status::Finished {
                draw::text(ui, tr!("autotune-finished-note"), draw::BODY, p.ink);
                ui.horizontal(|ui| {
                    if widgets::action_button(ui, p, "restore", tr!("autotune-give-back"), ActionKind::Flat, vec2(0.0, 0.0)).clicked() {
                        out.push(Action::Autotune(Request::Stop));
                    }
                    if widgets::action_button(ui, p, "check", tr!("autotune-keep"), ActionKind::Flat, vec2(0.0, 0.0))
                        .on_hover_text(tr!("autotune-keep-tip"))
                        .clicked()
                    {
                        out.push(Action::Autotune(Request::Keep));
                    }
                });
            }

            // what it may change (Tuning Strategy, the autotune panel)
            ui.add_space(4.0);
            draw::text(ui, tr!("autotune-strategy"), draw::BODY, p.ink);
            widgets::radio_value(ui, p, &mut config.strategy, Strategy::AvatarsOnly, tr!("autotune-strategy-avatars"));
            widgets::radio_value(
                ui,
                p,
                &mut config.strategy,
                Strategy::AvatarsAndScene,
                tr!("autotune-strategy-both"),
            );
            widgets::switch_labeled(ui, p, &mut config.by_distance, tr!("autotune-by-distance"))
                .on_hover_text(tr!("autotune-by-distance-tip"));
            ui.add_enabled_ui(config.by_distance, |ui| {
                ui.horizontal(|ui| {
                    ui.label(tr!("autotune-farthest-full"));
                    ui.add(
                        egui::Slider::new(&mut config.impostor_distance, IMPOSTOR_DISTANCE_MIN..=IMPOSTOR_DISTANCE_MAX)
                            .step_by(1.0)
                            .suffix(tr!("autotune-metre-suffix")),
                    );
                });
            });
            ui.add_enabled_ui(config.strategy == Strategy::AvatarsAndScene, |ui| {
                ui.horizontal(|ui| {
                    ui.label(tr!("autotune-min-distance"));
                    ui.add(
                        egui::Slider::new(&mut config.min_draw_distance, autotune::DD_FLOOR..=MIN_DD_MAX)
                            .step_by(1.0)
                            .suffix(tr!("autotune-metre-suffix")),
                    )
                    .on_hover_text(tr!("autotune-min-distance-tip"));
                });
            });
            ui.add_space(4.0);
            draw::text(ui, tr!("autotune-safety-note"), draw::SMALL, p.muted);
            draw::text(ui, tr!("autotune-spared-note"), draw::SMALL, p.muted);
        });
    config.sanitize();
    if config != m.autotune.config {
        out.push(Action::Autotune(Request::Config(config)));
    }
}
