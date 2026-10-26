//! NACLFloaterExploreSounds (indra/newview/NACLfloaterexploresounds.cpp):
//! live/paused log, selection, local preview, camera and blacklist actions.
use super::widgets::{self, Floater};
use crate::scene::sounds::history::Entry;
use crate::theme::Palette;
use egui::{RichText, Sense, Vec2};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::Instant;

pub const WINDOW: &str = "sound_explorer";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Filters {
    pub avatars: bool,
    pub objects: bool,
    pub collisions: bool,
    pub repeats: bool,
}

impl Default for Filters {
    fn default() -> Self {
        // floater_NACL_explore_sounds.xml: only objects start checked.
        Self {
            avatars: false,
            objects: true,
            collisions: false,
            repeats: false,
        }
    }
}

#[derive(Clone)]
pub struct Row {
    pub entry: Entry,
    pub owner: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Play,
    StopLocal,
    LookAt(u64),
    Stop,
    Blacklist(u8),
    BlacklistWindow,
}

#[derive(Default)]
pub struct SoundExplorer {
    pub rows: Vec<Row>,
    pub paused: bool,
    pub selected: HashSet<u64>,
    anchor: Option<u64>,
    sort: Option<(usize, bool)>,
    pub next_refresh: Option<Instant>,
}

impl SoundExplorer {
    pub fn selected_rows(&self) -> impl Iterator<Item = &Row> {
        self.rows.iter().filter(|r| self.selected.contains(&r.entry.id))
    }

    fn visible(&self, filters: &Filters) -> Vec<usize> {
        let mut rows: Vec<_> = (0..self.rows.len()).collect();
        rows.sort_by(|a, b| {
            let (a, b) = (&self.rows[*a], &self.rows[*b]);
            let newest = || {
                a.entry
                    .stopped
                    .is_some()
                    .cmp(&b.entry.stopped.is_some())
                    .then_with(|| {
                        b.entry
                            .stopped
                            .unwrap_or(b.entry.started)
                            .cmp(&a.entry.stopped.unwrap_or(a.entry.started))
                    })
                    .then_with(|| b.entry.id.cmp(&a.entry.id))
            };
            match self.sort {
                None | Some((0, false)) => newest(),
                Some((0, true)) => newest().reverse(),
                Some((column, reverse)) => {
                    let order = match column {
                        1 => kind(&a.entry).cmp(kind(&b.entry)),
                        2 => a.owner.to_lowercase().cmp(&b.owner.to_lowercase()),
                        _ => a.entry.asset.cmp(&b.entry.asset),
                    };
                    (if reverse { order.reverse() } else { order }).then_with(newest)
                }
            }
        });
        // Deduplicate in playback order before any user-selected column sort.
        let mut newest: Vec<_> = (0..self.rows.len()).collect();
        newest.sort_by_key(|i| {
            let e = &self.rows[*i].entry;
            (
                e.stopped.is_some(),
                std::cmp::Reverse(e.stopped.unwrap_or(e.started)),
                std::cmp::Reverse(e.id),
            )
        });
        let mut assets = HashSet::new();
        let keep: HashSet<_> = newest
            .into_iter()
            .filter(|i| {
                let e = &self.rows[*i].entry;
                let avatar = e.source == e.owner && !e.owner.is_nil();
                if (avatar && !filters.avatars)
                    || (!avatar && !filters.objects)
                    || (!filters.collisions && crate::scene::sounds::is_collision(&e.asset))
                {
                    return false;
                }
                let first = assets.insert(e.asset);
                filters.repeats || first
            })
            .collect();
        rows.retain(|i| keep.contains(i));
        rows
    }

    fn click(&mut self, id: u64, order: &[u64], modifiers: egui::Modifiers) {
        if modifiers.shift
            && let Some(anchor) = self.anchor
            && let (Some(a), Some(b)) = (order.iter().position(|i| *i == anchor), order.iter().position(|i| *i == id))
        {
            if !modifiers.command {
                self.selected.clear();
            }
            self.selected.extend(&order[a.min(b)..=a.max(b)]);
            return;
        }
        if modifiers.command {
            if !self.selected.remove(&id) {
                self.selected.insert(id);
            }
        } else {
            self.selected.clear();
            self.selected.insert(id);
        }
        self.anchor = Some(id);
    }

    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, open: &mut bool, filters: &mut Filters, previewing: bool) -> Option<Action> {
        let mut action = None;
        let mut floater = Floater::new(
            WINDOW,
            aurora_i18n::tr!("toolbar-command-sound-explorer"),
            egui::pos2(240.0, 130.0),
            Vec2::new(800.0, 450.0),
        );
        floater.min_size = Vec2::new(580.0, 320.0);
        floater.show(ctx, p, open, |ui| {
            ui.horizontal_wrapped(|ui| {
                widgets::checkbox(ui, p, &mut filters.avatars, aurora_i18n::tr!("sound-explorer-avatars"));
                widgets::checkbox(ui, p, &mut filters.objects, aurora_i18n::tr!("sound-explorer-objects"));
                widgets::checkbox(ui, p, &mut filters.collisions, aurora_i18n::tr!("sound-explorer-collisions"));
                widgets::checkbox(ui, p, &mut filters.repeats, aurora_i18n::tr!("sound-explorer-repeats"));
                widgets::checkbox(ui, p, &mut self.paused, aurora_i18n::tr!("sound-explorer-pause"));
            });
            let shown = self.visible(filters);
            let order: Vec<_> = shown.iter().map(|i| self.rows[*i].entry.id).collect();
            self.selected.retain(|id| order.contains(id));
            let width = widgets::header_width(ui, ui.scope_id().with("sound_rows"));
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                for (column, label) in [
                    aurora_i18n::tr!("sound-explorer-status"),
                    aurora_i18n::tr!("common-type"),
                    aurora_i18n::tr!("inspect-col-owner"),
                    aurora_i18n::tr!("sound-explorer-asset"),
                ]
                .into_iter()
                .enumerate()
                {
                    if ui
                        .add_sized([width * WEIGHTS[column], 25.0], egui::Button::new(label).truncate())
                        .clicked()
                    {
                        self.sort = Some((column, self.sort.is_some_and(|s| s == (column, false))));
                    }
                }
            });
            let height = (ui.available_height() - 110.0).max(50.0);
            let out = egui::ScrollArea::vertical()
                .id_salt("sound_rows")
                .max_height(height)
                .auto_shrink([false, false])
                .show_rows(ui, 26.0, shown.len().max(1), |ui, range| {
                    if shown.is_empty() {
                        ui.label(RichText::new(aurora_i18n::tr!("sound-explorer-empty")).color(p.muted));
                        return;
                    }
                    for i in range {
                        let row = &self.rows[shown[i]];
                        let id = row.entry.id;
                        let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::click());
                        if self.selected.contains(&id) || response.hovered() {
                            let color = if self.selected.contains(&id) {
                                p.violet.gamma_multiply(0.28)
                            } else {
                                p.raised
                            };
                            ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, color);
                        }
                        let status = row.entry.stopped.map_or_else(
                            || aurora_i18n::tr!("sound-explorer-playing").to_owned(),
                            |at| {
                                aurora_i18n::tr!(
                                    "sound-explorer-ago",
                                    minutes = format!("{:.1}", Instant::now().saturating_duration_since(at).as_secs_f32() / 60.0)
                                )
                            },
                        );
                        let texts = [status, kind(&row.entry).to_owned(), row.owner.clone(), row.entry.asset.to_string()];
                        let mut x = rect.left();
                        for (column, text) in texts.iter().enumerate() {
                            let w = rect.width() * WEIGHTS[column];
                            let color = if column == 0 && row.entry.stopped.is_none() {
                                p.success
                            } else {
                                p.ink
                            };
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
                        response.clone().on_hover_text(format!("{}\n{}", row.owner, row.entry.asset));
                        if response.clicked() {
                            self.click(id, &order, ui.input(|i| i.modifiers));
                        }
                        if response.double_clicked() {
                            action = Some(Action::Play);
                        }
                    }
                });
            widgets::remember_scroll_bar(ui, ui.scope_id().with("sound_rows"), &out);
            let selected = !self.selected.is_empty();
            ui.horizontal_wrapped(|ui| {
                for (label, enabled, act) in [
                    (aurora_i18n::tr!("sound-explorer-play"), selected, Action::Play),
                    (aurora_i18n::tr!("sound-explorer-stop-local"), previewing, Action::StopLocal),
                    (
                        aurora_i18n::tr!("sound-explorer-look"),
                        self.selected.len() == 1,
                        Action::LookAt(self.selected.iter().copied().next().unwrap_or_default()),
                    ),
                    (aurora_i18n::tr!("common-stop"), selected, Action::Stop),
                    (aurora_i18n::tr!("sound-explorer-blacklist"), selected, Action::Blacklist(0)),
                ] {
                    if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                        action = Some(act);
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                for (label, flag) in [
                    (aurora_i18n::tr!("sound-explorer-block-worn"), 1),
                    (aurora_i18n::tr!("sound-explorer-block-rezzed"), 2),
                    (aurora_i18n::tr!("sound-explorer-block-gestures"), 4),
                ] {
                    if ui.add_enabled(selected, egui::Button::new(label)).clicked() {
                        action = Some(Action::Blacklist(flag));
                    }
                }
                if ui.button(aurora_i18n::tr!("blacklist-title")).clicked() {
                    action = Some(Action::BlacklistWindow);
                }
            });
        });
        action
    }
}

const WEIGHTS: [f32; 4] = [0.18, 0.22, 0.32, 0.28];

fn kind(e: &Entry) -> &'static str {
    if e.source == e.owner && !e.owner.is_nil() {
        aurora_i18n::tr!("sound-explorer-avatar")
    } else if e.triggered {
        "llTriggerSound"
    } else if e.looped {
        "llLoopSound"
    } else {
        "llPlaySound"
    }
}

#[cfg(test)]
mod tests;
