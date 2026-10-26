//! Overview: six tiles (processor, graphics card, memory, video memory,
//! network, disk) with their minute, where the time of a frame goes, and
//! the part that costs the most of each resource.

use super::draw::{self, Line, Segment};
use super::model::{Column, RowId};
use super::snapshot::{HISTORY, Point, Snapshot};
use super::{Monitor, Tab, fmt, subsystems};
use crate::theme::Palette;
use crate::ui::widgets;
use aurora_i18n::tr;
use egui::{Color32, Sense, Stroke, Vec2, pos2, vec2};

/// "3,05 Go" → ("3,05", "Go"): a tile's value large, its unit small.
fn split_unit(s: &str) -> (String, String) {
    match s.rsplit_once(' ') {
        Some((v, u)) => (v.to_owned(), u.to_owned()),
        None => (s.to_owned(), String::new()),
    }
}

struct Tile {
    icon: &'static str,
    label: &'static str,
    right: String,
    value: String,
    unit: String,
    sub: String,
    lines: Vec<(Vec<f64>, Color32, bool)>,
    max: f64,
    go: Option<(Column, RowId)>,
    tab: Tab,
    hover: Box<dyn Fn(&Point) -> String>,
}

pub(super) fn show(ui: &mut egui::Ui, p: &Palette, m: &mut Monitor, s: &Snapshot) {
    super::heading(ui, p, tr!("monitor-tab-overview"), tr!("monitor-overview-sub"), |_| {});
    egui::ScrollArea::vertical()
        .id_salt("monitor-overview")
        .auto_shrink([false, false])
        .show(ui, |ui| body(ui, p, m, s));
}

fn body(ui: &mut egui::Ui, p: &Palette, m: &mut Monitor, s: &Snapshot) {
    alerts(ui, p, m, s);
    let tiles = tiles(p, m, s);
    let gap = 10.0;
    let w = (ui.available_width() - gap * 2.0) / 3.0;
    for row in tiles.chunks(3) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for t in row {
                tile(ui, p, m, t, w);
            }
        });
    }
    let room = ui.available_height().max(330.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        let left = (ui.available_width() - gap) * 0.55;
        ui.allocate_ui_with_layout(vec2(left, room), egui::Layout::top_down(egui::Align::Min), |ui| {
            frame_card(ui, p, s)
        });
        ui.allocate_ui_with_layout(vec2(ui.available_width(), room), egui::Layout::top_down(egui::Align::Min), |ui| {
            top_card(ui, p, m, s)
        });
    });
}

fn alerts(ui: &mut egui::Ui, p: &Palette, m: &mut Monitor, s: &Snapshot) {
    let t = &s.texture;
    let mut list: Vec<(String, &'static str, Tab)> = Vec::new();
    if let (true, Some(usage), Some(budget)) = (s.vram.over_budget(), s.vram.usage, s.vram.budget) {
        list.push((
            tr!(
                "monitor-alert-vram-over",
                usage = fmt::bytes(usage as f64),
                budget = fmt::bytes(budget as f64)
            ),
            tr!("monitor-see-memory"),
            Tab::Memory,
        ));
    }
    if t.pressure > 0 {
        list.push((
            tr!(
                "monitor-alert-texture-pressure",
                step = i64::from(t.pressure),
                steps = i64::from(t.max_pressure),
                lowered = t.reduced as i64
            ),
            tr!("monitor-see-memory"),
            Tab::Memory,
        ));
    }
    if s.net.loss >= 2.0 {
        list.push((
            tr!("monitor-alert-losses", loss = fmt::pct(f64::from(s.net.loss))),
            tr!("monitor-see-network"),
            Tab::Network,
        ));
    }
    for (text, action, tab) in list {
        egui::Frame::new()
            .fill(p.warn.gamma_multiply(0.10))
            .stroke(Stroke::new(1.0, p.warn.gamma_multiply(0.35)))
            .corner_radius(ui.visuals().window_corner_radius)
            .inner_margin(egui::Margin::symmetric(10, 5))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    widgets::glyph(ui, "alert-triangle-filled", p.warn, 15.0);
                    // the link first, the text cut in the room left before it
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if draw::link(ui, p, action).clicked() {
                            m.tab = tab;
                        }
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.add(egui::Label::new(egui::RichText::new(text).size(draw::BODY).color(p.ink)).truncate());
                        });
                    });
                });
            });
    }
}

fn tiles(p: &Palette, m: &Monitor, s: &Snapshot) -> Vec<Tile> {
    let h = &m.history;
    let a = &s.attribution;
    let mut out = Vec::new();

    // processor
    let (cpu, cores) = s.cpu.as_ref().map_or((None, 0), |c| (Some(c.process), c.cores));
    let cpu_series = h.series(|pt| pt.cpu.map(f64::from));
    out.push(Tile {
        icon: "cpu",
        label: tr!("monitor-tile-cpu"),
        right: cpu.map_or(String::new(), |c| {
            tr!(
                "monitor-cores-used",
                used = aurora_i18n::number(f64::from(c) * cores as f64 / 100.0, 1)
            )
        }),
        value: cpu.map_or("–".into(), |c| aurora_i18n::number(f64::from(c), if c < 10.0 { 1 } else { 0 })),
        unit: tr!("monitor-unit-pct-aurora").to_owned(),
        sub: tr!(
            "monitor-tile-cpu-sub",
            media = a.get(RowId::Media).get(Column::Cpu).map_or("–".into(), fmt::pct),
            machine = s
                .cpu
                .as_ref()
                .and_then(|c| c.machine)
                .map_or("–".into(), |v| fmt::pct(f64::from(v)))
        ),
        max: draw::nice_max(cpu_series.iter().copied().fold(10.0, f64::max) * 1.2),
        lines: vec![(cpu_series, p.violet_light, true)],
        go: Some((Column::Cpu, RowId::Render)),
        tab: Tab::Subsystems,
        hover: Box::new(|pt| pt.cpu.map_or("–".into(), |v| fmt::pct(f64::from(v)))),
    });

    // graphics card
    let gpu = s.parts.gpu.map(|g| g.iter().sum::<f32>());
    let gpu_series = h.series(|pt| pt.gpu_ms.map(f64::from));
    let g = s.parts.gpu.unwrap_or_default();
    out.push(Tile {
        icon: "device-desktop-analytics",
        label: tr!("monitor-tile-gpu"),
        right: tr!("monitor-fps", fps = aurora_i18n::number(f64::from(s.fps), 0)),
        value: gpu.map_or("–".into(), |v| aurora_i18n::number(f64::from(v), 1)),
        unit: tr!("monitor-unit-ms-frame").to_owned(),
        sub: if gpu.is_some() {
            tr!(
                "monitor-tile-gpu-sub",
                shadows = fmt::ms(f64::from(g[0])),
                objects = fmt::ms(f64::from(g[3]))
            )
        } else {
            tr!("ui-perf-per-item-details-unavailable-with-this-graphics-card").to_owned()
        },
        max: draw::nice_max(gpu_series.iter().copied().fold(8.0, f64::max) * 1.2),
        lines: vec![(gpu_series, p.indigo_light, true)],
        go: Some((Column::Gpu, RowId::Render)),
        tab: Tab::Subsystems,
        hover: Box::new(|pt| pt.gpu_ms.map_or("–".into(), |v| fmt::ms(f64::from(v)))),
    });

    // memory
    let ram_series = h.series(|pt| pt.ram);
    let (value, unit) = split_unit(&s.memory.map_or("–".into(), |mem| fmt::bytes(mem.working_set as f64)));
    out.push(Tile {
        icon: "stack-2-filled",
        label: tr!("monitor-tile-ram"),
        right: s.memory.map_or(String::new(), |mem| {
            tr!("monitor-peak", value = fmt::bytes(mem.peak_working_set as f64))
        }),
        value,
        unit,
        sub: tr!(
            "monitor-tile-ram-sub",
            private = s.memory.map_or("–".into(), |mem| fmt::bytes(mem.private as f64)),
            media = a.get(RowId::Media).get(Column::Ram).map_or("–".into(), fmt::bytes)
        ),
        max: ram_series.iter().copied().fold(1.0, f64::max) * 1.25,
        lines: vec![(ram_series, p.teal, true)],
        go: None,
        tab: Tab::Memory,
        hover: Box::new(|pt| pt.ram.map_or("–".into(), fmt::bytes)),
    });

    // video memory
    let vram_series = h.series(|pt| pt.vram);
    let budget = s.vram.budget.map(|b| b as f64);
    let (value, unit) = split_unit(&s.vram.usage.map_or("–".into(), |v| fmt::bytes(v as f64)));
    out.push(Tile {
        icon: "photo-filled",
        label: tr!("monitor-tile-vram"),
        right: if s.texture.pressure > 0 {
            tr!(
                "monitor-reduction",
                step = i64::from(s.texture.pressure),
                steps = i64::from(s.texture.max_pressure)
            )
        } else {
            budget.map_or(String::new(), |b| tr!("monitor-budget", value = fmt::bytes(b)))
        },
        value,
        unit,
        sub: tr!(
            "monitor-tile-vram-sub",
            textures = fmt::bytes(s.vram.pages as f64),
            geometry = fmt::bytes(s.vram.geometry as f64)
        ),
        max: budget.unwrap_or_else(|| vram_series.iter().copied().fold(1.0, f64::max) * 1.25),
        lines: vec![(vram_series, p.violet_pale, true)],
        go: Some((Column::Vram, RowId::Textures)),
        tab: Tab::Subsystems,
        hover: Box::new(|pt| pt.vram.map_or("–".into(), fmt::bytes)),
    });

    // network
    let ins = h.series(|pt| Some(f64::from(pt.net_in)));
    let outs = h.series(|pt| Some(f64::from(pt.net_out)));
    let (value, unit) = split_unit(&fmt::kbps(f64::from(s.net.in_kbps)));
    out.push(Tile {
        icon: "network",
        label: tr!("monitor-tile-net"),
        right: tr!("monitor-ping", ms = i64::from(s.net.ping_ms)),
        value,
        unit: tr!("monitor-incoming-unit", unit = unit),
        sub: tr!(
            "monitor-tile-net-sub",
            out = fmt::kbps(f64::from(s.net.out_kbps)),
            loss = fmt::pct(f64::from(s.net.loss))
        ),
        max: draw::nice_max(ins.iter().chain(&outs).copied().fold(10.0, f64::max) * 1.15),
        lines: vec![(ins, p.teal, true), (outs, p.rose, false)],
        go: Some((Column::Net, RowId::World)),
        tab: Tab::Network,
        hover: Box::new(|pt| {
            tr!(
                "ui-perf-incoming-outgoing",
                value0 = fmt::kbps(f64::from(pt.net_in)),
                value1 = fmt::kbps(f64::from(pt.net_out))
            )
        }),
    });

    // disk
    let reads = h.series(|pt| Some(pt.disk_read));
    let writes = h.series(|pt| Some(pt.disk_write));
    let total: f64 = s.disk.rates.iter().map(|r| r[0] + r[1]).sum();
    let (value, unit) = split_unit(&fmt::disk_rate(total));
    out.push(Tile {
        icon: "database-filled",
        label: tr!("monitor-tile-disk"),
        right: tr!(
            "monitor-cache-of",
            used = fmt::bytes(s.disk.used as f64),
            limit = fmt::bytes(s.disk.limit as f64)
        ),
        value,
        unit,
        sub: tr!(
            "monitor-tile-disk-sub",
            read = fmt::disk_rate(s.disk.rates.iter().map(|r| r[0]).sum()),
            written = fmt::disk_rate(s.disk.rates.iter().map(|r| r[1]).sum())
        ),
        max: draw::nice_max(reads.iter().chain(&writes).copied().fold(1024.0 * 1024.0, f64::max) * 1.15),
        lines: vec![(reads, p.amber, true), (writes, p.rose, false)],
        go: None,
        tab: Tab::Disk,
        hover: Box::new(|pt| {
            tr!(
                "monitor-read-written",
                read = fmt::disk_rate(pt.disk_read),
                written = fmt::disk_rate(pt.disk_write)
            )
        }),
    });
    out
}

fn tile(ui: &mut egui::Ui, p: &Palette, m: &mut Monitor, t: &Tile, width: f32) {
    let height = 128.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    let hot = resp.hovered();
    ui.painter().rect(
        rect,
        ui.visuals().window_corner_radius,
        p.bar,
        Stroke::new(1.0, if hot { p.muted_dim } else { p.raised }),
        egui::StrokeKind::Inside,
    );
    let inner = rect.shrink2(vec2(12.0, 10.0));
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.spacing_mut().item_spacing.y = 2.0;
    child.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 7.0;
        widgets::glyph(ui, t.icon, p.muted, 14.0);
        draw::text(ui, t.label, draw::BODY, p.muted);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(egui::Label::new(egui::RichText::new(&t.right).size(draw::BODY).color(p.muted_dim)).truncate());
        });
    });
    child.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        draw::text(ui, &t.value, 22.0, p.ink);
        draw::text(ui, &t.unit, draw::BODY, p.muted);
    });
    child.add(egui::Label::new(egui::RichText::new(&t.sub).size(draw::SMALL).color(p.muted)).truncate());
    let lines: Vec<Line> = t
        .lines
        .iter()
        .map(|(v, c, f)| Line {
            values: v,
            color: *c,
            fill: *f,
        })
        .collect();
    let hover_text = |i: usize| m.history.points.get(i).map_or(String::new(), |pt| (t.hover)(pt));
    let graph_h = (inner.bottom() - child.cursor().top()).max(20.0);
    draw::graph(
        &mut child,
        p,
        vec2(inner.width(), graph_h),
        &lines,
        t.max,
        HISTORY,
        Some(&hover_text),
    );
    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        match t.go {
            Some((col, row)) if t.tab == Tab::Subsystems => m.open_subsystems(col, row),
            _ => m.tab = t.tab,
        }
    }
}

/// Where the time of a frame goes, on the processor and on the graphics
/// card, with the compact view's parts and colours.
fn frame_card(ui: &mut egui::Ui, p: &Palette, s: &Snapshot) {
    draw::card(ui, p, |ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        let colors = draw::part_colors(p);
        let cpu = s.parts.cpu;
        let busy: f32 = cpu[..6].iter().sum();
        let gpu_total = s.parts.gpu.map(|g| g.iter().sum::<f32>());
        let limit = if cpu[6] > s.frame_ms * 0.3 {
            tr!("monitor-limited-waiting")
        } else if gpu_total.is_some_and(|g| g > busy) {
            tr!("monitor-limited-gpu")
        } else {
            tr!("monitor-limited-cpu")
        };
        let thread = s.parts.render_thread.map_or(String::new(), |ms| {
            format!(" · {}", tr!("monitor-render-thread", ms = fmt::ms(f64::from(ms))))
        });
        draw::card_title(
            ui,
            p,
            tr!("monitor-frame-time"),
            &format!(
                "{} · {limit}{thread}",
                tr!("monitor-fps", fps = aurora_i18n::number(f64::from(s.fps), 0))
            ),
        );
        let names = crate::ui::perf::cpu_part_names();
        bar_block(
            ui,
            p,
            tr!("ui-perf-cpu"),
            &tr!(
                "monitor-frame-cpu-total",
                total = fmt::ms(f64::from(s.frame_ms)),
                waiting = fmt::ms(f64::from(cpu[6]))
            ),
            &cpu,
            &names.map(|n| n.0),
            &colors,
            f64::from(s.frame_ms),
        );
        ui.add_space(6.0);
        match s.parts.gpu {
            Some(gpu) => {
                let names = crate::ui::perf::gpu_part_names();
                let total: f32 = gpu.iter().sum();
                bar_block(
                    ui,
                    p,
                    tr!("ui-perf-gpu-rendering"),
                    &fmt::ms(f64::from(total)),
                    &gpu,
                    &names.map(|n| n.0),
                    &colors,
                    f64::from(total.max(s.frame_ms)),
                );
            }
            None => {
                draw::text(ui, tr!("ui-perf-gpu-rendering"), draw::BODY, p.ink);
                draw::text(
                    ui,
                    tr!("ui-perf-per-item-details-unavailable-with-this-graphics-card"),
                    draw::SMALL,
                    p.muted_dim,
                );
            }
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn bar_block(
    ui: &mut egui::Ui,
    p: &Palette,
    title: &str,
    total: &str,
    values: &[f32; 7],
    names: &[&'static str; 7],
    colors: &[Color32],
    scale: f64,
) {
    ui.horizontal(|ui| {
        draw::text(ui, title, draw::BODY, p.ink);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            draw::text(ui, total, draw::BODY, p.muted);
        });
    });
    let segments: Vec<Segment> = values
        .iter()
        .zip(colors)
        .map(|(v, c)| Segment {
            value: f64::from(*v),
            color: *c,
        })
        .collect();
    draw::segments(ui, p, 12.0, &segments, scale);
    let items: Vec<(Color32, String, String)> = names
        .iter()
        .zip(values)
        .zip(colors)
        .map(|((n, v), c)| (*c, (*n).to_owned(), fmt::ms(f64::from(*v))))
        .collect();
    draw::legend(ui, p, 2, &items);
}

/// For each resource, the part that costs the most of it: a click opens
/// the subsystems sorted by that resource, the part selected.
fn top_card(ui: &mut egui::Ui, p: &Palette, m: &mut Monitor, s: &Snapshot) {
    let a = &s.attribution;
    draw::card(ui, p, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        draw::card_title(ui, p, tr!("monitor-top-title"), "");
        let resources = [
            (Column::Cpu, tr!("monitor-tile-cpu")),
            (Column::Gpu, tr!("monitor-tile-gpu")),
            (Column::Ram, tr!("monitor-tile-ram")),
            (Column::Vram, tr!("monitor-tile-vram")),
            (Column::Net, tr!("monitor-tile-net")),
            (Column::Disk, tr!("monitor-tile-disk")),
        ];
        for (col, label) in resources {
            let total = a.total(col).unwrap_or(0.0);
            let best = RowId::parents()
                .filter(|r| *r != RowId::Other)
                .filter_map(|r| a.get(r).get(col).map(|v| (r, v)))
                .filter(|(_, v)| *v > 0.0)
                .max_by(|x, y| x.1.total_cmp(&y.1));
            let Some((row, value)) = best else { continue };
            let share = if total > 0.0 { value / total } else { 0.0 };
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.panel);
            }
            let cy = rect.center().y;
            let font = egui::FontId::proportional(draw::BODY);
            widgets::paint_elided(ui, label, pos2(rect.left() + 6.0, cy), 96.0, font.clone(), p.muted);
            let value_text = subsystems::format(col, value);
            let vg = ui.painter().layout_no_wrap(value_text, font.clone(), p.ink);
            let vx = rect.right() - 6.0 - vg.size().x;
            widgets::paint_row_text(ui.painter(), vg, vx, cy, p.ink);
            let mid_l = rect.left() + 108.0;
            let mid_w = (vx - 10.0 - mid_l).max(40.0);
            let name = format!(
                "{} · {}",
                subsystems::row_name(row),
                tr!("monitor-share-of-total", share = fmt::pct(share * 100.0))
            );
            widgets::paint_elided(ui, &name, pos2(mid_l, cy - 6.0), mid_w, font, p.ink);
            draw::paint_meter(
                ui,
                p,
                egui::Rect::from_min_size(pos2(mid_l, cy + 6.0), Vec2::new(mid_w, 6.0)),
                share as f32,
                p.violet,
            );
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                m.open_subsystems(col, row);
            }
        }
        ui.add_space(6.0);
        draw::text(ui, tr!("monitor-top-hint"), draw::SMALL, p.muted_dim);
    });
}
