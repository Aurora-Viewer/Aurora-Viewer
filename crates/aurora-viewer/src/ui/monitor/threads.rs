//! Threads: the load of each logical core of the machine, the viewer's
//! processes (itself and its media plugins), and every group of threads of
//! the viewer with what it does.

use super::draw::{self, Col, ROW_H};
use super::fmt;
use super::model::Column;
use super::snapshot::Snapshot;
use crate::theme::Palette;
use crate::ui::widgets;
use aurora_i18n::tr;
use egui::{Rect, Sense, pos2, vec2};

/// What a group of threads does.
fn role(group: &str) -> &'static str {
    match group {
        "main" => tr!("monitor-thread-main"),
        "aurora-render" => tr!("monitor-thread-render"),
        "aurora-render-finish" => tr!("monitor-thread-render-finish"),
        "aurora-worker" => tr!("monitor-thread-worker"),
        "aurora-jobs" => tr!("monitor-thread-jobs"),
        "aurora-pages" => tr!("monitor-thread-pages"),
        "aurora-net" => tr!("monitor-thread-net"),
        "aurora-voice" | "aurora-voice-rt" => tr!("monitor-thread-voice"),
        "aurora-audio-out" => tr!("monitor-thread-audio-out"),
        "aurora-audio-in" => tr!("monitor-thread-audio-in"),
        "aurora-audio-stream" => tr!("monitor-thread-audio-stream"),
        "aurora-texcache" => tr!("monitor-thread-texcache"),
        "aurora-cache" => tr!("monitor-thread-cache"),
        "aurora-trash" => tr!("monitor-thread-trash"),
        "aurora-inventory-cache" => tr!("monitor-thread-inventory"),
        "aurora-chatlog" | "aurora-transcript" | "aurora-notification-history" => tr!("monitor-thread-logs"),
        "aurora-monitor" | "aurora-probe" => tr!("monitor-thread-monitor"),
        "unnamed" => tr!("monitor-thread-unnamed"),
        _ => "",
    }
}

/// A Windows thread priority (`THREAD_PRIORITY_*`) in words.
fn priority(p: Option<i32>) -> &'static str {
    match p {
        Some(-15) => tr!("monitor-priority-idle"),
        Some(-2) => tr!("monitor-priority-lowest"),
        Some(-1) => tr!("monitor-priority-below-normal"),
        Some(0) => tr!("monitor-priority-normal"),
        Some(1) => tr!("monitor-priority-above-normal"),
        Some(2) => tr!("monitor-priority-highest"),
        Some(15) => tr!("monitor-priority-time-critical"),
        _ => "–",
    }
}

pub(super) fn show(ui: &mut egui::Ui, p: &Palette, s: &Snapshot) {
    let Some(cpu) = &s.cpu else {
        super::heading(ui, p, tr!("monitor-tab-threads"), "", |_| {});
        draw::text(ui, tr!("monitor-no-system-counters"), draw::BODY, p.muted);
        return;
    };
    let threads: usize = cpu.groups.iter().map(|g| g.threads).sum();
    super::heading(
        ui,
        p,
        tr!("monitor-tab-threads"),
        &tr!(
            "monitor-threads-sub",
            used = aurora_i18n::number(f64::from(cpu.process) * cpu.cores as f64 / 100.0, 1),
            cores = cpu.cores as i64,
            threads = threads as i64
        ),
        |_| {},
    );
    let gap = 10.0;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        let half = (ui.available_width() - gap) / 2.0;
        ui.allocate_ui_with_layout(vec2(half, 110.0), egui::Layout::top_down(egui::Align::Min), |ui| {
            draw::card(ui, p, |ui| {
                draw::card_title(
                    ui,
                    p,
                    tr!("monitor-cores-title"),
                    &tr!(
                        "monitor-whole-machine",
                        value = cpu.machine.map_or("–".into(), |v| fmt::pct(f64::from(v)))
                    ),
                );
                cores(ui, p, &cpu.per_core);
            });
        });
        ui.allocate_ui_with_layout(vec2(half, 110.0), egui::Layout::top_down(egui::Align::Min), |ui| {
            draw::card(ui, p, |ui| {
                draw::card_title(ui, p, tr!("monitor-processes-title"), tr!("monitor-processes-units"));
                let own = f64::from(cpu.process) * cpu.cores as f64;
                process_row(ui, p, "Aurora", own, s.memory.map(|mem| mem.working_set as f64));
                for (proc_, usage) in &s.attribution.media {
                    let one_core = usage.get(Column::Cpu).map_or(0.0, |v| v * cpu.cores as f64);
                    process_row(ui, p, &format!("SLPlugin · {}", proc_.label), one_core, usage.get(Column::Ram));
                }
                if s.attribution.media.is_empty() {
                    draw::text(ui, tr!("monitor-no-media-process"), draw::SMALL, p.muted_dim);
                }
            });
        });
    });

    let cols = [
        Col::new(tr!("monitor-col-thread"), 0.0, false),
        Col::new(tr!("monitor-col-count"), 60.0, true),
        Col::new(tr!("monitor-col-cpu-core"), 170.0, true),
        Col::new(tr!("monitor-col-busiest"), 130.0, true),
        Col::new(tr!("monitor-col-priority"), 160.0, false),
        Col::new(tr!("monitor-col-role"), 260.0, false),
    ];
    let frame = egui::Frame::new()
        .fill(p.bar)
        .stroke(egui::Stroke::new(1.0, p.raised))
        .corner_radius(ui.visuals().window_corner_radius);
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.set_height(ui.available_height());
        ui.spacing_mut().item_spacing.y = 0.0;
        draw::header(ui, p, &cols);
        egui::ScrollArea::vertical()
            .id_salt("monitor-threads")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let busiest = cpu.groups.iter().map(|g| g.percent).fold(1.0f32, f32::max);
                for g in &cpu.groups {
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::hover());
                    draw::row_surface(ui, p, rect, false, resp.hovered());
                    let r = draw::col_ranges(&cols, rect.left(), rect.width());
                    let cy = rect.center().y;
                    draw::cell_text(ui, r[0], cy, &g.group, false, p.ink, draw::BODY);
                    draw::cell_text(ui, r[1], cy, &fmt::count(g.threads), true, p.ink, draw::BODY);
                    let meter = Rect::from_min_size(pos2(r[2].0 + 10.0, cy - 3.0), vec2(80.0, 6.0));
                    draw::paint_meter(ui, p, meter, g.percent / busiest.max(100.0), p.violet);
                    draw::cell_text(ui, r[2], cy, &fmt::pct(f64::from(g.percent)), true, p.ink, draw::BODY);
                    let busy = if g.threads > 1 {
                        fmt::pct(f64::from(g.busiest))
                    } else {
                        "–".into()
                    };
                    draw::cell_text(ui, r[3], cy, &busy, true, p.muted, draw::BODY);
                    draw::cell_text(ui, r[4], cy, priority(g.priority), false, p.muted, draw::BODY);
                    draw::cell_text(ui, r[5], cy, role(&g.group), false, p.muted, draw::BODY);
                }
            });
    });
}

/// One bar per logical core, its load on hover.
fn cores(ui: &mut egui::Ui, p: &Palette, per_core: &[f32]) {
    if per_core.is_empty() {
        draw::text(ui, tr!("monitor-no-core-counters"), draw::SMALL, p.muted_dim);
        return;
    }
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 54.0), Sense::hover());
    let n = per_core.len() as f32;
    let gap = if n > 32.0 { 1.0 } else { 4.0 };
    let w = ((rect.width() - gap * (n - 1.0)) / n).max(1.0);
    let radius = widgets::inner_radius(ui.visuals());
    for (i, load) in per_core.iter().enumerate() {
        let x = rect.left() + i as f32 * (w + gap);
        let bar = Rect::from_min_size(pos2(x, rect.top()), vec2(w, rect.height()));
        ui.painter().rect_filled(bar, radius, p.field);
        let h = rect.height() * (load / 100.0).clamp(0.0, 1.0);
        if h >= 1.0 {
            ui.painter()
                .rect_filled(Rect::from_min_max(pos2(x, bar.bottom() - h), bar.max), radius, p.violet);
        }
        ui.interact(bar, ui.scope_id().with(("monitor-core", i)), Sense::hover())
            .on_hover_text(tr!("monitor-core-load", core = i as i64, load = fmt::pct(f64::from(*load))));
    }
}

fn process_row(ui: &mut egui::Ui, p: &Palette, name: &str, one_core: f64, ram: Option<f64>) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
    let cy = rect.center().y;
    let font = egui::FontId::proportional(draw::BODY);
    let ram = ram.map_or("–".into(), fmt::bytes);
    let rg = ui.painter().layout_no_wrap(ram, font.clone(), p.muted);
    let rx = rect.right() - rg.size().x;
    widgets::paint_row_text(ui.painter(), rg, rx, cy, p.muted);
    let cg = ui.painter().layout_no_wrap(fmt::pct(one_core), font.clone(), p.ink);
    let cx = rx - 16.0 - cg.size().x;
    widgets::paint_row_text(ui.painter(), cg, cx, cy, p.ink);
    let meter = Rect::from_min_size(pos2(cx - 98.0, cy - 3.0), vec2(90.0, 6.0));
    draw::paint_meter(ui, p, meter, (one_core / 600.0) as f32, p.violet);
    widgets::paint_elided(
        ui,
        name,
        pos2(rect.left(), cy),
        (meter.left() - rect.left() - 8.0).max(20.0),
        font,
        p.ink,
    );
}
