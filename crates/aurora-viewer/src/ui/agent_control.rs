//! The agents' control bar: several MCP agents share this logged-in viewer by
//! taking turns with the control lease (aurora_mcp::LeaseManager), and the
//! person at the viewer sees who is working and why, is asked first (Allow /
//! Not now), can take the viewer back, and chooses how it is shared: Ask me,
//! Let agents in (away), or Hold (no agent).
//!
//! Shown only while there is something to show: an agent working, asking or
//! waiting, or Hold on. Chat, IMs and everything else keep working beneath it.

use super::widgets::{self, ActionKind, action_button};
use crate::theme::Palette;
use aurora_mcp::{ControlView, HumanMode};
use egui::{RichText, Vec2};

const WIDTH: f32 = 380.0;
const BUTTON: Vec2 = Vec2::new(104.0, 28.0);

pub enum AgentControlAction {
    Allow(String),
    NotNow(String),
    TakeBack,
    Mode(HumanMode),
}

fn minutes(secs: u64) -> String {
    secs.div_ceil(60).max(1).to_string()
}

pub fn show(ctx: &egui::Context, p: &Palette, view: &ControlView) -> Option<AgentControlAction> {
    let quiet = view.active.is_none() && view.asking.is_none() && view.queue.is_empty() && view.mode != HumanMode::Hold;
    if quiet {
        return None;
    }
    let mut out = None;
    egui::Area::new(egui::Id::unique("agent_control_bar"))
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_TOP, Vec2::new(0.0, 44.0))
        .constrain(true)
        .show(ctx, |ui| {
            widgets::overlay(ui, p, egui::Margin::same(12), |ui| {
                ui.set_width(WIDTH);
                if let Some(ask) = &view.asking {
                    ui.label(
                        RichText::new(aurora_i18n::tr!("agent-control-asks", agent = ask.owner.clone()))
                            .size(14.0)
                            .strong()
                            .color(p.ink),
                    );
                    let purpose = if ask.purpose.is_empty() {
                        aurora_i18n::tr!("agent-control-no-purpose").to_owned()
                    } else {
                        ask.purpose.clone()
                    };
                    ui.label(RichText::new(purpose).size(12.5).color(p.ink));
                    ui.label(
                        RichText::new(aurora_i18n::tr!("agent-control-minutes", minutes = minutes(ask.ttl_seconds)))
                            .size(11.5)
                            .color(p.muted),
                    );
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if action_button(ui, p, "check", aurora_i18n::tr!("agent-control-allow"), ActionKind::Primary, BUTTON).clicked() {
                            out = Some(AgentControlAction::Allow(ask.owner.clone()));
                        }
                        if action_button(ui, p, "clock", aurora_i18n::tr!("agent-control-not-now"), ActionKind::Flat, BUTTON).clicked() {
                            out = Some(AgentControlAction::NotNow(ask.owner.clone()));
                        }
                    });
                    if let Some(secs) = view.asking_auto_in_secs {
                        ui.label(
                            RichText::new(aurora_i18n::tr!("agent-control-auto-in", seconds = secs.to_string()))
                                .size(11.0)
                                .color(p.muted_dim),
                        );
                    }
                    ui.separator();
                } else if let Some(active) = &view.active {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.set_width(WIDTH - BUTTON.x - 12.0);
                            ui.label(
                                RichText::new(aurora_i18n::tr!("agent-control-working", agent = active.owner.clone()))
                                    .size(13.5)
                                    .strong()
                                    .color(p.ink),
                            );
                            if !active.purpose.is_empty() {
                                ui.label(RichText::new(active.purpose.clone()).size(12.0).color(p.ink));
                            }
                            ui.label(
                                RichText::new(aurora_i18n::tr!("agent-control-left", minutes = minutes(active.remaining_seconds)))
                                    .size(11.0)
                                    .color(p.muted),
                            );
                        });
                        if action_button(
                            ui,
                            p,
                            "hand-stop",
                            aurora_i18n::tr!("agent-control-take-back"),
                            ActionKind::Danger,
                            BUTTON,
                        )
                        .on_hover_text(aurora_i18n::tr!("agent-control-take-back-hint"))
                        .clicked()
                        {
                            out = Some(AgentControlAction::TakeBack);
                        }
                    });
                    ui.separator();
                } else if view.mode == HumanMode::Hold {
                    ui.label(
                        RichText::new(aurora_i18n::tr!("agent-control-held"))
                            .size(13.0)
                            .strong()
                            .color(p.ink),
                    );
                }
                let waiting: Vec<&str> = view
                    .queue
                    .iter()
                    .filter(|q| view.asking.as_ref().is_none_or(|a| a.owner != q.owner))
                    .map(|q| q.owner.as_str())
                    .collect();
                if !waiting.is_empty() {
                    ui.label(
                        RichText::new(aurora_i18n::tr!("agent-control-waiting", agents = waiting.join(", ")))
                            .size(11.5)
                            .color(p.muted),
                    );
                }
                ui.horizontal(|ui| {
                    for (mode, label) in [
                        (HumanMode::Ask, aurora_i18n::tr!("agent-control-mode-ask")),
                        (HumanMode::Auto, aurora_i18n::tr!("agent-control-mode-auto")),
                        (HumanMode::Hold, aurora_i18n::tr!("agent-control-mode-hold")),
                    ] {
                        if ui.selectable_label(view.mode == mode, label).clicked() && view.mode != mode {
                            out = Some(AgentControlAction::Mode(mode));
                        }
                    }
                });
            });
        });
    out
}
