//! Help › Grid status (and the toolbar's Grid status command): the grid's
//! incidents and maintenance (`grid_status.rs`). LLFloaterGridStatus
//! (indra/newview/llfloatergridstatus.cpp, floater_grid_status.xml,
//! originally LGPL 2.1) shows the status page in a web browser; this window
//! lists the feed's entries in the theme, the most recently updated first,
//! each with its newest update and the earlier ones folded, and opens the
//! status page or an entry's own page in the system browser.

use super::widgets::{self, ActionKind, Floater};
use crate::grid_status::{Entry, GridStatus, Load, Phase};
use crate::theme::Palette;
use egui::{RichText, Vec2};

/// The window's id (Floater.Show "grid_status").
pub const WINDOW: &str = "grid_status";

/// What the window asks of the app.
pub enum Action {
    Refresh,
    /// A web page, in the system browser.
    Open(String),
}

/// Where the grid stands, from the entries: an incident not over first,
/// then a maintenance.
fn summary(entries: &[Entry]) -> Phase {
    let phases: Vec<Phase> = entries.iter().map(Entry::phase).collect();
    if phases.contains(&Phase::Ongoing) {
        Phase::Ongoing
    } else if phases.contains(&Phase::Maintenance) {
        Phase::Maintenance
    } else {
        Phase::Over
    }
}

/// The icon and color of a phase.
fn look(p: &Palette, phase: Phase) -> (&'static str, egui::Color32) {
    match phase {
        Phase::Ongoing => ("alert-triangle-filled", p.warn),
        Phase::Maintenance => ("calendar-event-filled", p.violet_light),
        Phase::Over => ("circle-check-filled", p.success),
        Phase::Unknown => ("info-circle-filled", p.muted),
    }
}

fn phase_label(phase: Phase) -> &'static str {
    match phase {
        Phase::Ongoing => aurora_i18n::tr!("grid-status-ongoing"),
        Phase::Maintenance => aurora_i18n::tr!("grid-status-maintenance"),
        Phase::Over => aurora_i18n::tr!("grid-status-over"),
        Phase::Unknown => aurora_i18n::tr!("grid-status-notice"),
    }
}

/// "2026-10-26 10:45 SLT", in the interface's date order, with the
/// daylight saving time of the bars' clock.
fn date_text(secs: i64) -> String {
    let t = secs + crate::world::chat_log::format::pacific_offset_hours(secs) * 3600;
    let (y, m, d) = aurora_net::profile::civil_from_days(t.div_euclid(86_400));
    let minutes = t.rem_euclid(86_400) / 60;
    aurora_i18n::tr!(
        "grid-status-date",
        year = format!("{y:04}"),
        month = format!("{m:02}"),
        day = format!("{d:02}"),
        hour = format!("{:02}", minutes / 60),
        minute = format!("{:02}", minutes % 60)
    )
}

fn icon(ui: &mut egui::Ui, name: &str, color: egui::Color32, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    if let Some(t) = super::icons::global(name) {
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        ui.painter().image(t.id(), rect, uv, color);
    }
}

/// Draw the window while `open`.
pub fn show(ctx: &egui::Context, p: &Palette, status: &GridStatus, open: &mut bool) -> Option<Action> {
    if !*open {
        return None;
    }
    let mut action = None;
    let mut window = Floater::new(
        WINDOW,
        aurora_i18n::tr!("toolbar-command-grid-status"),
        egui::pos2(360.0, 120.0),
        Vec2::new(540.0, 500.0),
    );
    window.min_size = Vec2::new(380.0, 240.0);
    window.show(ctx, p, open, |ui| {
        ui.horizontal(|ui| {
            if !status.entries.is_empty() {
                let phase = summary(&status.entries);
                let (name, color) = look(p, phase);
                icon(ui, name, color, 18.0);
                let text = match phase {
                    Phase::Ongoing => aurora_i18n::tr!("grid-status-summary-incident"),
                    Phase::Maintenance => aurora_i18n::tr!("grid-status-summary-maintenance"),
                    _ => aurora_i18n::tr!("grid-status-summary-normal"),
                };
                // the buttons keep their room: a long translation is cut
                let room = (ui.available_width() - 260.0).max(60.0);
                ui.scope(|ui| {
                    ui.set_max_width(room);
                    ui.add(egui::Label::new(RichText::new(text).strong().color(color)).truncate());
                });
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(page) = status.page() {
                    let label = aurora_i18n::tr!("grid-status-open-page");
                    if widgets::action_button(ui, p, "external-link-filled", label, ActionKind::Flat, Vec2::new(0.0, 26.0)).clicked() {
                        action = Some(Action::Open(page.to_owned()));
                    }
                }
                let refresh = aurora_i18n::tr!("grid-status-refresh");
                let idle = !matches!(status.load, Load::Loading | Load::NoFeed);
                if widgets::action_button_if(ui, p, "refresh", refresh, ActionKind::Flat, Vec2::new(0.0, 26.0), idle).clicked() {
                    action = Some(Action::Refresh);
                }
            });
        });
        widgets::divider(ui, p);
        if status.entries.is_empty() {
            ui.add_space(8.0);
            let text = match &status.load {
                Load::NoFeed => aurora_i18n::tr!("grid-status-no-feed").to_owned(),
                Load::Loading => aurora_i18n::tr!("common-loading-2").to_owned(),
                Load::Loaded => aurora_i18n::tr!("grid-status-empty").to_owned(),
                Load::Failed(_) => aurora_i18n::tr!("grid-status-failed").to_owned(),
            };
            ui.label(RichText::new(text).color(p.muted));
            if let Load::Failed(e) = &status.load {
                ui.label(RichText::new(e).size(11.5).color(p.muted_dim));
            }
            return;
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for entry in &status.entries {
                if let Some(a) = entry_card(ui, p, entry, status.fresh.contains(&entry.id)) {
                    action = Some(a);
                }
                ui.add_space(8.0);
            }
        });
    });
    action
}

/// One entry: its phase, date and title, its newest update, the earlier
/// ones folded.
fn entry_card(ui: &mut egui::Ui, p: &Palette, entry: &Entry, fresh: bool) -> Option<Action> {
    let mut action = None;
    widgets::surface(ui, p, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let phase = entry.phase();
            let (name, color) = look(p, phase);
            icon(ui, name, color, 14.0);
            ui.label(RichText::new(phase_label(phase)).size(11.5).strong().color(color));
            if fresh {
                ui.label(
                    RichText::new(aurora_i18n::tr!("grid-status-new"))
                        .size(11.5)
                        .strong()
                        .color(p.violet_light),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(link) = &entry.link
                    && widgets::icon_button(ui, p, "external-link-filled", aurora_i18n::tr!("grid-status-open-entry"), true)
                {
                    action = Some(Action::Open(link.clone()));
                }
                if entry.updated > 0 {
                    ui.label(RichText::new(date_text(entry.updated)).size(11.5).color(p.muted));
                }
            });
        });
        ui.add(egui::Label::new(RichText::new(&entry.title).size(14.0).strong().color(p.ink)).wrap());
        let mut updates = entry.updates.iter();
        if let Some(u) = updates.next() {
            update(ui, p, u);
        }
        if entry.updates.len() > 1 {
            widgets::collapsing(
                ui,
                p,
                ("grid_status_earlier", &entry.id),
                aurora_i18n::tr!("grid-status-earlier"),
                false,
                |ui| {
                    for u in updates {
                        update(ui, p, u);
                    }
                },
            );
        }
    });
    action
}

fn update(ui: &mut egui::Ui, p: &Palette, u: &crate::grid_status::Update) {
    ui.add_space(4.0);
    if !u.when.is_empty() {
        ui.label(RichText::new(&u.when).size(11.5).color(p.muted));
    }
    let mut job = egui::text::LayoutJob::default();
    let font = egui::FontId::proportional(12.5);
    if !u.status.is_empty() {
        let family = super::fonts::semibold(ui.ctx());
        job.append(&u.status, 0.0, super::fonts::semibold_format(&family, 12.5, p.ink));
        job.append(" – ", 0.0, egui::TextFormat::simple(font.clone(), p.muted));
    }
    job.append(&u.text, 0.0, egui::TextFormat::simple(font, p.ink));
    job.wrap.max_width = ui.available_width();
    ui.add(egui::Label::new(job).wrap());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid_status::Update;

    fn entry(status: &str) -> Entry {
        Entry {
            updates: vec![Update {
                status: status.into(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn an_incident_not_over_comes_before_a_maintenance() {
        assert_eq!(
            summary(&[entry("Resolved"), entry("Scheduled"), entry("Monitoring")]),
            Phase::Ongoing
        );
        assert_eq!(summary(&[entry("Resolved"), entry("In progress")]), Phase::Maintenance);
        assert_eq!(summary(&[entry("Resolved"), entry("Completed"), entry("")]), Phase::Over);
    }

    #[test]
    fn the_date_is_in_slt() {
        // the date's order follows the interface language: only the parts
        // 2026-10-26 17:45 UTC is 10:45 PDT, a week before the switch
        let summer = date_text(1_793_036_700);
        assert!(
            summer.contains("2026") && summer.contains("10") && summer.contains("26"),
            "{summer}"
        );
        assert!(summer.ends_with(" 10:45 SLT"), "{summer}");
        // 2026-12-01 18:00 UTC is 10:00 PST
        let winter = date_text(1_796_148_000);
        assert!(winter.contains("12") && winter.contains("01"), "{winter}");
        assert!(winter.ends_with(" 10:00 SLT"), "{winter}");
    }
}
