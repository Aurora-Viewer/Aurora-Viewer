//! AnimationExplorer (indra/newview/animationexplorer.cpp,
//! floater_animation_explorer.xml): the animations objects and residents
//! played on our avatar, newest first, with who played them, whether they
//! still play and their priority; the selected one plays on a stand-in
//! avatar, and can be stopped, stopped with the source's permissions
//! revoked, or blacklisted.
use super::widgets::{self, ActionKind, Floater};
use crate::theme::Palette;
use aurora_render::preview::{Model, Pose, View};
use egui::{RichText, Sense, Vec2};
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

pub const WINDOW: &str = "animation_explorer";

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: u64,
    pub animation: Uuid,
    pub source: Uuid,
    /// "Played by": the resident's or the object's name, its UUID until known.
    pub name: String,
    /// Played by our own avatar (AO, gestures, default motions).
    pub own: bool,
    pub playing: bool,
    pub started: Instant,
    /// LLKeyframeMotion::getPriority once the motion has loaded.
    pub priority: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Stop,
    StopAndRevoke,
    Blacklist,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Column {
    PlayedBy,
    Played,
    Priority,
}

#[derive(Default)]
pub struct AnimationExplorer {
    /// Newest first.
    pub rows: Vec<Row>,
    pub selected: Option<u64>,
    /// "No Owned Animations": unchecked when the window opens, like the
    /// Legacy Viewer's check box (it has no saved setting).
    pub no_owned: bool,
    pub view: View,
    sort: Option<(Column, bool)>,
}

const WEIGHTS: [f32; 3] = [0.5, 0.3, 0.2];
const PREVIEW: f32 = 256.0;

/// "Still playing" or how long ago it started (animation_explorer_seconds_ago).
pub fn played_text(row: &Row, now: Instant) -> String {
    if row.playing {
        aurora_i18n::tr!("animation-explorer-still-playing").to_owned()
    } else {
        aurora_i18n::tr!(
            "animation-explorer-seconds-ago",
            seconds = now.saturating_duration_since(row.started).as_secs()
        )
    }
}

fn priority_text(row: &Row) -> String {
    row.priority.map_or_else(
        || aurora_i18n::tr!("animation-explorer-unknown-priority").to_owned(),
        |p| p.to_string(),
    )
}

impl AnimationExplorer {
    pub fn selected_row(&self) -> Option<&Row> {
        self.selected.and_then(|id| self.rows.iter().find(|r| r.id == id))
    }

    /// The rows shown: own motions hidden by the check box, then the
    /// column sort (none: newest first).
    fn visible(&self) -> Vec<usize> {
        let mut rows: Vec<_> = (0..self.rows.len()).filter(|i| !(self.no_owned && self.rows[*i].own)).collect();
        if let Some((column, descending)) = self.sort {
            rows.sort_by(|a, b| {
                let (a, b) = (&self.rows[*a], &self.rows[*b]);
                let order = match column {
                    Column::PlayedBy => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                    Column::Played => b.playing.cmp(&a.playing).then(b.started.cmp(&a.started)),
                    Column::Priority => a.priority.cmp(&b.priority),
                };
                if descending { order.reverse() } else { order }
            });
        }
        rows
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        open: &mut bool,
        model: Option<Arc<Model>>,
        pose: Option<Pose>,
    ) -> Option<Action> {
        let mut action = None;
        let mut floater = Floater::new(
            WINDOW,
            aurora_i18n::tr!("ui-bars-animation-explorer"),
            egui::pos2(260.0, 140.0),
            Vec2::new(760.0, 400.0),
        );
        floater.min_size = Vec2::new(560.0, 360.0);
        let now = Instant::now();
        floater.show(ctx, p, open, |ui| {
            let shown = self.visible();
            if self.selected.is_some_and(|id| !shown.iter().any(|i| self.rows[*i].id == id)) {
                self.selected = None;
            }
            ui.horizontal_top(|ui| {
                let list_width = (ui.available_width() - PREVIEW - ui.spacing().item_spacing.x).max(240.0);
                ui.allocate_ui_with_layout(
                    Vec2::new(list_width, ui.available_height()),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(list_width);
                        self.list(ui, p, &shown, now);
                    },
                );
                ui.vertical(|ui| {
                    ui.set_width(PREVIEW);
                    self.view.background = p.field.to_srgba_unmultiplied().map(|v| f32::from(v) / 255.0);
                    let size = Vec2::splat(PREVIEW);
                    match model {
                        Some(model) => {
                            aurora_render::preview::paint_with_pose(
                                ui,
                                egui::Id::unique("animation_explorer_preview"),
                                model,
                                &mut self.view,
                                size,
                                pose,
                            )
                            .on_hover_text(aurora_i18n::tr!("import-preview-camera-help"));
                        }
                        None => {
                            widgets::picture(ui, p, None, size, "run", "");
                        }
                    }
                    if widgets::action_button(
                        ui,
                        p,
                        "refresh",
                        aurora_i18n::tr!("import-preview-reset-view"),
                        ActionKind::Flat,
                        Vec2::new(0.0, 26.0),
                    )
                    .clicked()
                    {
                        self.view.reset_camera();
                    }
                    let selected = self.selected.is_some();
                    let full = Vec2::new(PREVIEW, 28.0);
                    for (icon, label, act) in [
                        ("player-stop-filled", aurora_i18n::tr!("common-stop"), Action::Stop),
                        (
                            "shield-off",
                            aurora_i18n::tr!("animation-explorer-stop-revoke"),
                            Action::StopAndRevoke,
                        ),
                        ("eye-off", aurora_i18n::tr!("animation-explorer-blacklist"), Action::Blacklist),
                    ] {
                        let response = widgets::action_button_if(ui, p, icon, label, ActionKind::Flat, full, selected);
                        if response.clicked() {
                            action = Some(act);
                        }
                    }
                    widgets::checkbox(ui, p, &mut self.no_owned, aurora_i18n::tr!("animation-explorer-no-owned"))
                        .on_hover_text(aurora_i18n::tr!("animation-explorer-no-owned-help"));
                });
            });
        });
        action
    }

    fn list(&mut self, ui: &mut egui::Ui, p: &Palette, shown: &[usize], now: Instant) {
        let list_id = ui.scope_id().with("animation_rows");
        let width = widgets::header_width(ui, list_id);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for (weight, column, label) in [
                (WEIGHTS[0], Column::PlayedBy, aurora_i18n::tr!("animation-explorer-played-by")),
                (WEIGHTS[1], Column::Played, aurora_i18n::tr!("animation-explorer-played")),
                (WEIGHTS[2], Column::Priority, aurora_i18n::tr!("animation-explorer-priority")),
            ] {
                if ui.add_sized([width * weight, 25.0], egui::Button::new(label).truncate()).clicked() {
                    // A second click on the same column reverses it.
                    self.sort = Some((column, self.sort == Some((column, false))));
                }
            }
        });
        let out = egui::ScrollArea::vertical()
            .id_salt("animation_rows")
            .auto_shrink([false, false])
            .show_rows(ui, 26.0, shown.len().max(1), |ui, range| {
                if shown.is_empty() {
                    ui.label(RichText::new(aurora_i18n::tr!("animation-explorer-empty")).color(p.muted));
                    return;
                }
                for i in range {
                    let row = &self.rows[shown[i]];
                    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::click());
                    let selected = self.selected == Some(row.id);
                    if selected || response.hovered() {
                        let color = if selected { p.violet.gamma_multiply(0.28) } else { p.raised };
                        ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, color);
                    }
                    let texts = [row.name.clone(), played_text(row, now), priority_text(row)];
                    let mut x = rect.left();
                    for (column, text) in texts.iter().enumerate() {
                        let w = rect.width() * WEIGHTS[column];
                        let color = if column == 1 && row.playing { p.success } else { p.ink };
                        widgets::paint_elided(
                            ui,
                            text,
                            egui::pos2(x + 4.0, rect.center().y),
                            w - 8.0,
                            egui::FontId::proportional(12.0),
                            color,
                        );
                        x += w;
                    }
                    let response = response.on_hover_text(format!("{}\n{}", row.name, row.animation));
                    if response.clicked() {
                        self.selected = Some(row.id);
                    }
                }
            });
        widgets::remember_scroll_bar(ui, list_id, &out);
    }
}

#[cfg(test)]
mod tests;
