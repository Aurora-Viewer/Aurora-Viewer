//! The removal from this computer (`uninstall.rs`): its confirmation, which
//! lists what will be removed, then the window of what was done and of the
//! folders left for the user to delete, each with its "Open folder".

use super::widgets::{self, ActionKind, Floater, action_button};
use crate::theme::Palette;
use crate::uninstall::{Done, Part, Plan, Report};
use egui::{RichText, Vec2};
use std::path::PathBuf;

/// The result window's id.
pub const WINDOW: &str = "uninstall_result";
const BUTTON: Vec2 = Vec2::new(0.0, 26.0);

#[derive(Default)]
pub struct UninstallUi {
    /// The confirmation, open with what it will remove.
    pub confirm: Option<Plan>,
    /// The result window, and its report once the removal finished.
    pub open: bool,
    pub report: Option<Report>,
    /// The folders paths are shown from (`uninstall::shown`).
    pub bases: Vec<(&'static str, PathBuf)>,
    /// Frames left to scroll the finished report to the folders left (the
    /// demo; a new window's first frame only measures it).
    pub scroll_to_left: u8,
}

#[derive(Debug, PartialEq)]
pub enum Action {
    /// Removal confirmed.
    Run(Plan),
    OpenFolder(PathBuf),
    Quit,
}

pub fn show(ctx: &egui::Context, p: &Palette, st: &mut UninstallUi) -> Vec<Action> {
    let mut actions = Vec::new();
    confirm(ctx, p, st, &mut actions);
    result(ctx, p, st, &mut actions);
    actions
}

fn confirm(ctx: &egui::Context, p: &Palette, st: &mut UninstallUi, actions: &mut Vec<Action>) {
    use aurora_i18n::tr;
    let Some(plan) = &st.confirm else { return };
    let mut close = false;
    let modal = widgets::Modal::new(egui::Id::unique("uninstall_confirm")).show(ctx, |ui| {
        ui.set_width(460.0);
        ui.label(RichText::new(tr!("uninstall-title")).size(15.0).strong().color(p.ink));
        ui.add_space(6.0);
        ui.add(egui::Label::new(RichText::new(tr!("uninstall-text")).size(12.5).color(p.muted)).wrap());
        ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .max_height(280.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for part in &plan.parts {
                    part_row(ui, p, part, "trash-filled", p.muted);
                }
            });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if action_button(ui, p, "trash-filled", tr!("uninstall-confirm"), ActionKind::Danger, BUTTON).clicked() {
                actions.push(Action::Run(plan.clone()));
                close = true;
            }
            if action_button(ui, p, "x", tr!("common-cancel"), ActionKind::Flat, BUTTON).clicked() {
                close = true;
            }
        });
    });
    if close || modal.should_close() {
        st.confirm = None;
    }
}

/// One part: its icon and name, and where it is under it.
fn part_row(ui: &mut egui::Ui, p: &Palette, part: &Part, icon: &str, tint: egui::Color32) {
    ui.horizontal(|ui| {
        if let Some(t) = super::icons::global(icon) {
            ui.add(egui::Image::new(&t).fit_to_exact_size(Vec2::splat(14.0)).tint(tint));
        }
        ui.add(egui::Label::new(RichText::new(crate::uninstall::part_label(part)).size(12.5).color(p.ink)).wrap());
    });
    for place in crate::uninstall::part_places(part) {
        ui.indent(("place", place.as_str()), |ui| {
            ui.add(egui::Label::new(RichText::new(place.as_str()).size(11.5).color(p.muted)).wrap());
        });
    }
}

fn result(ctx: &egui::Context, p: &Palette, st: &mut UninstallUi, actions: &mut Vec<Action>) {
    use aurora_i18n::tr;
    if !st.open {
        return;
    }
    let screen = ctx.content_rect();
    let size = Vec2::new(560.0, 520.0)
        .min(screen.size() - Vec2::new(40.0, 80.0))
        .max(Vec2::new(380.0, 300.0));
    let mut open = st.open;
    let mut close = false;
    let bases = &st.bases;
    let report = st.report.as_ref();
    let scroll_to_left = report.is_some() && st.scroll_to_left > 0;
    if scroll_to_left {
        st.scroll_to_left -= 1;
    }
    Floater::new(
        WINDOW,
        tr!("uninstall-window"),
        egui::pos2(
            screen.center().x - size.x / 2.0,
            (screen.center().y - size.y / 2.0).max(screen.top() + 40.0),
        ),
        size,
    )
    .steady_size()
    .show(ctx, p, &mut open, |ui| {
        let (mut list, footer) = widgets::above_footer(ui);
        let outer = ui;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(&mut list, |ui| match report {
                None => {
                    widgets::busy_button(ui, p, tr!("uninstall-running"));
                }
                Some(report) => report_body(ui, p, report, bases, scroll_to_left, actions),
            });
        footer.show(outer, |ui| {
            ui.horizontal(|ui| {
                let finished = report.is_some();
                if widgets::action_button_if(ui, p, "power", tr!("uninstall-quit"), ActionKind::Primary, BUTTON, finished).clicked() {
                    actions.push(Action::Quit);
                }
                if action_button(ui, p, "x", tr!("common-close"), ActionKind::Flat, BUTTON).clicked() {
                    close = true;
                }
            });
        });
    });
    // closed while it runs: the app opens it again with the result
    st.open = open && !close;
}

fn report_body(
    ui: &mut egui::Ui,
    p: &Palette,
    report: &Report,
    bases: &[(&'static str, PathBuf)],
    scroll_to_left: bool,
    actions: &mut Vec<Action>,
) {
    use aurora_i18n::tr;
    if report.dry_run {
        ui.add(egui::Label::new(RichText::new(tr!("uninstall-dry-run")).size(12.0).color(p.muted)).wrap());
        ui.add_space(6.0);
    }
    ui.label(RichText::new(tr!("uninstall-done")).size(13.5).strong().color(p.ink));
    ui.add_space(4.0);
    for (part, done) in &report.steps {
        let (icon, tint) = match done {
            Done::Removed => ("circle-check-filled", p.success),
            Done::Absent => ("circle-minus", p.muted),
            Done::Failed(_) => ("alert-triangle-filled", p.danger),
        };
        part_row(ui, p, part, icon, tint);
        let mut notes = vec![crate::uninstall::done_label(done)];
        if *part == Part::BlenderAddon {
            notes.extend(
                report
                    .blender
                    .iter()
                    .flat_map(|b| b.removed.iter().map(|path| path.display().to_string())),
            );
        }
        ui.indent(("done", crate::uninstall::part_label(part)), |ui| {
            for note in notes {
                let color = if matches!(done, Done::Failed(_)) { p.danger } else { p.muted };
                ui.add(egui::Label::new(RichText::new(note).size(11.5).color(color)).wrap());
            }
        });
        ui.add_space(2.0);
    }
    ui.add_space(10.0);
    let left = ui.label(RichText::new(tr!("uninstall-left")).size(13.5).strong().color(p.ink));
    if scroll_to_left {
        left.scroll_to_me(Some(egui::Align::TOP));
    }
    ui.add_space(2.0);
    ui.add(egui::Label::new(RichText::new(tr!("uninstall-left-text")).size(12.0).color(p.muted)).wrap());
    ui.add_space(6.0);
    for place in &report.remaining {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_width((ui.available_width() - 130.0).max(120.0));
                ui.label(RichText::new(crate::uninstall::kinds_label(&place.kinds)).size(12.5).color(p.ink));
                let shown = crate::uninstall::shown(&place.path, bases);
                ui.add(egui::Label::new(RichText::new(shown).size(11.5).color(p.muted)).wrap())
                    .on_hover_text(place.path.display().to_string());
            });
            if action_button(ui, p, "folder-open-filled", tr!("uninstall-open-folder"), ActionKind::Flat, BUTTON).clicked() {
                actions.push(Action::OpenFolder(place.folder(std::path::Path::is_dir)));
            }
        });
        ui.add_space(4.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uninstall::{Kind, Remaining, Secret};

    fn report() -> Report {
        Report {
            steps: vec![
                (
                    Part::Links(vec![r"HKEY_CURRENT_USER\Software\Classes\secondlife".into()]),
                    Done::Removed,
                ),
                (Part::Secret(Secret::CacheKey), Done::Absent),
                (Part::Secret(Secret::CrashReports), Done::Failed("store locked".into())),
                (Part::BlenderAddon, Done::Removed),
            ],
            blender: Vec::new(),
            remaining: (0..12)
                .map(|i| Remaining {
                    kinds: vec![Kind::Settings, Kind::Logs],
                    path: PathBuf::from(format!("folder-{i}/with a rather long name inside it")),
                })
                .collect(),
            dry_run: true,
        }
    }

    #[test]
    fn the_result_window_keeps_its_size_while_running_and_finished() {
        for finished in [false, true] {
            crate::ui::widgets::steady_tests::assert_window_steady(
                WINDOW,
                &[Vec2::new(560.0, 520.0), Vec2::new(380.0, 300.0)],
                aurora_i18n::tr!("common-close"),
                || UninstallUi {
                    open: true,
                    report: finished.then(report),
                    ..Default::default()
                },
                |st, ctx, p| {
                    show(ctx, p, st);
                },
            );
        }
    }

    #[test]
    fn confirming_hands_the_plan_over_once() {
        let ctx = egui::Context::default();
        let p = crate::theme::Theme::default().palette();
        let plan = Plan {
            parts: vec![Part::Secret(Secret::CacheKey)],
        };
        let mut st = UninstallUi {
            confirm: Some(plan),
            ..Default::default()
        };
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            assert!(show(ui.ctx(), &p, &mut st).is_empty(), "nothing runs before the click");
        });
        output.textures_delta.clear();
        assert!(st.confirm.is_some(), "the confirmation stays open");
    }
}
