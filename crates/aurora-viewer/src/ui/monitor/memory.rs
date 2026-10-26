//! Memory: the viewer's memory and its media processes', the video memory
//! against the driver's budget by kind, the texture budget's pressure, the
//! texture streamer's queues and the GPU buffers written each frame.

use super::draw::{self, Segment};
use super::model::{Column, RowId};
use super::snapshot::Snapshot;
use super::{Action, Page, fmt};
use crate::theme::Palette;
use aurora_i18n::tr;
use egui::{Color32, Sense, vec2};

pub(super) fn show(ui: &mut egui::Ui, p: &Palette, s: &Snapshot, out: &mut Vec<Action>) {
    super::heading(ui, p, tr!("monitor-tab-memory"), tr!("monitor-memory-sub"), |_| {});
    egui::ScrollArea::vertical()
        .id_salt("monitor-memory")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            two(ui, |ui| ram_card(ui, p, s), |ui| vram_card(ui, p, s, out));
            two(ui, |ui| textures_card(ui, p, s), |ui| geometry_card(ui, p, s));
        });
}

/// Two cards side by side, as tall as each other's content.
pub(super) fn two(ui: &mut egui::Ui, left: impl FnOnce(&mut egui::Ui), right: impl FnOnce(&mut egui::Ui)) {
    let gap = 10.0;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        let half = (ui.available_width() - gap) / 2.0;
        ui.allocate_ui_with_layout(vec2(half, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_width(half);
            left(ui);
        });
        ui.allocate_ui_with_layout(vec2(half, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_width(half);
            right(ui);
        });
    });
}

fn big(ui: &mut egui::Ui, p: &Palette, value: &str, note: &str) {
    big_noted(ui, p, value, note, p.muted);
}

fn big_noted(ui: &mut egui::Ui, p: &Palette, value: &str, note: &str, note_color: Color32) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        draw::text(ui, value, 22.0, p.ink);
        draw::text(ui, note, draw::BODY, note_color);
    });
}

fn ram_card(ui: &mut egui::Ui, p: &Palette, s: &Snapshot) {
    draw::card(ui, p, |ui| {
        draw::card_title(ui, p, tr!("monitor-tile-ram"), tr!("monitor-aurora-process"));
        let ws = s.memory.map(|m| m.working_set as f64);
        big(ui, p, &ws.map_or("–".into(), fmt::bytes), tr!("monitor-working-set"));
        let colors = draw::part_colors(p);
        let mut parts: Vec<(Color32, String, f64)> = vec![(colors[0], "Aurora".to_owned(), ws.unwrap_or(0.0))];
        for (i, (proc_, usage)) in s.attribution.media.iter().enumerate() {
            parts.push((colors[1 + i % 5], format!("SLPlugin · {}", proc_.label), usage.or_zero(Column::Ram)));
        }
        let total: f64 = parts.iter().map(|x| x.2).sum();
        ui.add_space(4.0);
        draw::segments(
            ui,
            p,
            16.0,
            &parts.iter().map(|x| Segment { value: x.2, color: x.0 }).collect::<Vec<_>>(),
            total,
        );
        ui.add_space(4.0);
        draw::legend(
            ui,
            p,
            2,
            &parts.iter().map(|x| (x.0, x.1.clone(), fmt::bytes(x.2))).collect::<Vec<_>>(),
        );
        ui.add_space(6.0);
        ui.spacing_mut().item_spacing.y = 0.0;
        let mem = s.memory;
        draw::kv_row(
            ui,
            p,
            tr!("monitor-private"),
            &mem.map_or("–".into(), |m| fmt::bytes(m.private as f64)),
        );
        draw::kv_row(
            ui,
            p,
            tr!("monitor-session-peak"),
            &mem.map_or("–".into(), |m| fmt::bytes(m.peak_working_set as f64)),
        );
        draw::kv_row(
            ui,
            p,
            tr!("monitor-page-faults"),
            &s.page_faults_per_s
                .map_or("–".into(), |v| tr!("monitor-per-second", value = fmt::count(v as usize))),
        );
        draw::kv_row(
            ui,
            p,
            &tr!("monitor-media-processes", count = s.attribution.media.len() as i64),
            &s.attribution.get(RowId::Media).get(Column::Ram).map_or("–".into(), fmt::bytes),
        );
    });
}

fn vram_card(ui: &mut egui::Ui, p: &Palette, s: &Snapshot, out: &mut Vec<Action>) {
    let v = s.vram;
    draw::card(ui, p, |ui| {
        draw::card_title(ui, p, tr!("monitor-tile-vram"), tr!("monitor-driver-budget"));
        // past the budget, Windows moves video memory out to system memory
        let over = v.over_budget();
        big_noted(
            ui,
            p,
            &v.usage.map_or("–".into(), |u| fmt::bytes(u as f64)),
            &v.budget
                .map_or(String::new(), |b| tr!("monitor-of-value", value = fmt::bytes(b as f64))),
            if over { p.warn } else { p.muted },
        );
        let colors = draw::part_colors(p);
        let rest = v.allocated.map(|a| a.saturating_sub(v.pages + v.geometry) as f64);
        let driver = match (v.usage, v.allocated) {
            (Some(u), Some(a)) => Some(u.saturating_sub(a) as f64),
            _ => None,
        };
        let parts = [
            (colors[0], tr!("ui-perf-texture-pages"), Some(v.pages as f64)),
            (colors[1], tr!("ui-perf-geometry"), Some(v.geometry as f64)),
            (colors[2], tr!("monitor-vram-render"), rest),
            (colors[6], tr!("monitor-vram-driver"), driver),
        ];
        ui.add_space(4.0);
        draw::segments(
            ui,
            p,
            16.0,
            &parts
                .iter()
                .map(|x| Segment {
                    value: x.2.unwrap_or(0.0),
                    color: x.0,
                })
                .collect::<Vec<_>>(),
            v.budget.map_or(0.0, |b| b as f64),
        );
        ui.add_space(4.0);
        draw::legend(
            ui,
            p,
            2,
            &parts
                .iter()
                .map(|x| (x.0, x.1.to_owned(), x.2.map_or("–".into(), fmt::bytes)))
                .collect::<Vec<_>>(),
        );
        ui.add_space(6.0);
        ui.spacing_mut().item_spacing.y = 0.0;
        let t = &s.texture;
        // the texture budget's steps, lit up to the pressure
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
        let cy = rect.center().y;
        let label = if t.pressure > 0 {
            tr!("monitor-reduction", step = i64::from(t.pressure), steps = i64::from(t.max_pressure))
        } else {
            tr!("monitor-no-reduction").to_owned()
        };
        let font = egui::FontId::proportional(draw::BODY);
        let g = ui.painter().layout_no_wrap(label, font.clone(), p.ink);
        let gx = rect.right() - g.size().x;
        crate::ui::widgets::paint_row_text(ui.painter(), g, gx, cy, p.ink);
        let step_w = 16.0;
        for i in 0..t.max_pressure {
            let x = gx - 10.0 - (f32::from(t.max_pressure - i)) * (step_w + 3.0);
            let r = egui::Rect::from_min_size(egui::pos2(x, cy - 3.0), vec2(step_w, 6.0));
            let on = i < t.pressure;
            ui.painter().rect_filled(
                r,
                crate::ui::widgets::inner_radius(ui.visuals()),
                if on { p.warn } else { p.raised },
            );
        }
        crate::ui::widgets::paint_elided(ui, tr!("ui-perf-texture-budget"), egui::pos2(rect.left(), cy), 120.0, font, p.muted);
        ui.painter().hline(
            rect.x_range(),
            rect.bottom() - 0.5,
            egui::Stroke::new(1.0, p.raised.gamma_multiply(0.6)),
        );
        draw::kv_row(
            ui,
            p,
            tr!("ui-perf-texture-pages"),
            &tr!(
                "monitor-pages-target",
                pages = fmt::bytes(v.pages as f64),
                target = fmt::bytes(t.target as f64)
            ),
        );
        draw::kv_row(
            ui,
            p,
            tr!("monitor-lowered"),
            &tr!("monitor-lowered-count", count = fmt::count(t.reduced as usize)),
        );
        ui.horizontal(|ui| {
            ui.set_min_height(24.0);
            draw::text(ui, tr!("monitor-budget-mode"), draw::BODY, p.muted);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if draw::link(ui, p, tr!("monitor-adjust")).clicked() {
                    out.push(Action::Preferences(Page::Graphics));
                }
                draw::text(
                    ui,
                    format!(
                        "{} ·",
                        if t.budget_auto {
                            tr!("monitor-automatic")
                        } else {
                            tr!("monitor-manual")
                        }
                    ),
                    draw::BODY,
                    p.ink,
                );
            });
        });
    });
}

fn textures_card(ui: &mut egui::Ui, p: &Palette, s: &Snapshot) {
    let t = &s.texture;
    draw::card(ui, p, |ui| {
        draw::card_title(ui, p, tr!("common-textures"), "");
        ui.spacing_mut().item_spacing.y = 0.0;
        draw::kv_row(
            ui,
            p,
            tr!("monitor-loaded"),
            &tr!("monitor-x-of-y", x = fmt::count(t.loaded), y = fmt::count(t.total)),
        );
        draw::kv_row(ui, p, tr!("monitor-downloading"), &fmt::count(t.fetching));
        draw::kv_row(ui, p, tr!("ui-perf-decoding"), &fmt::count(t.decoding));
        draw::kv_row(ui, p, tr!("monitor-waiting-upload"), &fmt::count(t.pending_upload));
        draw::kv_row(
            ui,
            p,
            tr!("monitor-uploads-frame"),
            &format!("{} ({})", fmt::count(t.uploads_frame), fmt::bytes(t.upload_bytes_frame as f64)),
        );
        draw::kv_row(ui, p, tr!("monitor-failing"), &fmt::count(t.failing));
        draw::kv_row(ui, p, tr!("ui-perf-background-tasks"), &fmt::count(t.jobs));
        draw::kv_row(ui, p, tr!("ui-perf-network-meshes"), &fmt::count(t.meshes_fetching));
    });
}

fn geometry_card(ui: &mut egui::Ui, p: &Palette, s: &Snapshot) {
    let g = &s.geometry;
    let per_frame = |b: u64| tr!("monitor-per-frame", value = fmt::bytes(b as f64));
    draw::card(ui, p, |ui| {
        draw::card_title(ui, p, tr!("monitor-geometry-title"), "");
        ui.spacing_mut().item_spacing.y = 0.0;
        draw::kv_row(
            ui,
            p,
            tr!("ui-perf-geometry"),
            &tr!(
                "monitor-allocated-used",
                allocated = fmt::bytes(g.allocated as f64),
                used = fmt::bytes(g.used as f64)
            ),
        );
        draw::kv_row(ui, p, tr!("monitor-geometry-pending"), &fmt::count(g.pending));
        draw::kv_row(ui, p, tr!("ui-perf-gpu-records"), &fmt::count(g.records));
        draw::kv_row(ui, p, tr!("monitor-records-sent"), &per_frame(g.records_frame));
        draw::kv_row(ui, p, tr!("monitor-palettes-sent"), &per_frame(g.palettes_frame));
        draw::kv_row(ui, p, tr!("monitor-cull-sent"), &per_frame(g.cull_frame));
        draw::kv_row(ui, p, tr!("monitor-journal"), &per_frame(g.journal_frame));
    });
}
