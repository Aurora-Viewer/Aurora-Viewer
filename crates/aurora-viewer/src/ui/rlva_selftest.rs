//! The two surfaces of the RLVa self-test (world/rlva_selftest.rs, a Dev
//! Toolbox tool of development builds): the dialog that says what a run
//! will do and asks for it, and the window with its progress and report.

use super::widgets::{self, ActionKind, action_button, action_button_if};
use crate::theme::Palette;
use crate::world::rlva_selftest::View;
use aurora_i18n::tr;
use egui::{FontId, RichText, Vec2};
use std::time::{Duration, Instant};

const COPIED_FOR: Duration = Duration::from_millis(1500);

#[derive(Default)]
pub struct SelfTestUi {
    pub open: bool,
    copied_at: Option<Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// The dialog was confirmed.
    Start,
    Cancel,
    /// "Run again": the dialog opens again, nothing starts.
    Request,
    Stop,
    Copy,
}

/// The confirmation: what the run creates, wears and may take off, and the
/// separate choice of the in-world cases. `rlva`: RLVa is on (no run
/// without it).
pub fn confirm(ctx: &egui::Context, p: &Palette, phase2: &mut bool, rlva: bool) -> Option<Action> {
    let mut answer = None;
    let response = widgets::Modal::new(egui::Id::unique("rlva_selftest_confirm")).show(ctx, |ui| {
        ui.set_width(480.0);
        ui.label(RichText::new(tr!("rlva-selftest-confirm-title")).size(17.0).strong().color(p.ink));
        ui.add_space(6.0);
        for text in [
            tr!("rlva-selftest-confirm-intro"),
            tr!("rlva-selftest-confirm-inventory"),
            tr!("rlva-selftest-confirm-wear"),
            tr!("rlva-selftest-confirm-rules"),
            tr!("rlva-selftest-confirm-end"),
        ] {
            ui.add(egui::Label::new(RichText::new(text).size(12.5).color(p.ink)).wrap());
            ui.add_space(4.0);
        }
        ui.add_space(4.0);
        widgets::checkbox(ui, p, phase2, tr!("rlva-selftest-phase2"));
        ui.add(egui::Label::new(RichText::new(tr!("rlva-selftest-phase2-note")).size(11.0).color(p.muted)).wrap());
        if !rlva {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                widgets::glyph(ui, "alert-triangle-filled", p.warn, 12.0);
                ui.label(RichText::new(tr!("rlva-selftest-needs-rlva")).size(11.0).color(p.warn));
            });
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let size = Vec2::new(0.0, 26.0);
            if action_button_if(ui, p, "flask-filled", tr!("rlva-selftest-start"), ActionKind::Primary, size, rlva).clicked() {
                answer = Some(Action::Start);
            }
            if action_button(ui, p, "x", tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                answer = Some(Action::Cancel);
            }
        });
    });
    if response.should_close() {
        answer.get_or_insert(Action::Cancel);
    }
    answer
}

/// The tool's window: Stop while it runs, then the report and Copy.
pub fn window(ctx: &egui::Context, p: &Palette, st: &mut SelfTestUi, view: &View, report: Option<&str>) -> Option<Action> {
    let mut action = None;
    let mut open = st.open;
    widgets::Floater::new(
        "rlva_selftest",
        tr!("rlva-selftest-title"),
        egui::pos2(120.0, 110.0),
        Vec2::new(820.0, 520.0),
    )
    .show(ctx, p, &mut open, |ui| {
        ui.horizontal(|ui| {
            let size = Vec2::new(0.0, 26.0);
            if view.running {
                if action_button(ui, p, "player-stop-filled", tr!("common-stop"), ActionKind::Flat, size).clicked() {
                    action = Some(Action::Stop);
                }
            } else if action_button(
                ui,
                p,
                "flask-filled",
                if report.is_some() {
                    tr!("rlva-selftest-run-again")
                } else {
                    tr!("rlva-selftest-start")
                },
                ActionKind::Flat,
                size,
            )
            .clicked()
            {
                action = Some(Action::Request);
            }
            let copied = st.copied_at.is_some_and(|t| t.elapsed() < COPIED_FOR);
            let label = if copied {
                tr!("dev-toolbox-copied")
            } else {
                tr!("dev-toolbox-copy")
            };
            if action_button_if(ui, p, "copy-filled", label, ActionKind::Primary, size, report.is_some()).clicked() {
                st.copied_at = Some(Instant::now());
                action = Some(Action::Copy);
            }
        });
        ui.add_space(6.0);
        if view.running {
            let fraction = if view.total == 0 {
                0.0
            } else {
                view.done as f32 / view.total as f32
            };
            let text = tr!("rlva-selftest-progress", done = view.done, total = view.total);
            widgets::progress(ui, p, "rlva_selftest", fraction, ui.available_width(), &text, None);
            ui.label(RichText::new(&view.current).size(11.0).color(p.muted));
        }
        egui::ScrollArea::both()
            .id_salt("rlva_selftest_report")
            .auto_shrink([false, false])
            .show(ui, |ui| match report {
                Some(text) => {
                    let mut lines = text.lines();
                    if let Some(first) = lines.next() {
                        ui.label(RichText::new(first).font(FontId::monospace(11.0)).color(p.violet_light));
                    }
                    let rest: String = lines.collect::<Vec<_>>().join("\n");
                    ui.add(egui::Label::new(RichText::new(rest).font(FontId::monospace(11.0)).color(p.ink)).extend());
                }
                None if !view.running => {
                    ui.label(RichText::new(tr!("rlva-selftest-idle")).size(12.0).color(p.muted));
                }
                None => {}
            });
    });
    st.open = open;
    action
}
