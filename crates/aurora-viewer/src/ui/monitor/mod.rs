//! Resource monitor: the Performance window's full view, a task manager
//! for power users. What the viewer consumes, over the last minute, in
//! eight sections down a rail: an overview, the subsystems (what each part
//! of the viewer costs in processor, frame time, GPU, memory, video memory,
//! network and disk), the threads, the memory, the network, the disk, the
//! costliest avatars and objects around, and the region's simulator. The
//! app gathers a snapshot every half second while the view is open
//! (`app/monitor.rs`); the model attributes the costs (`model.rs`).

mod disk;
mod draw;
mod fmt;
mod memory;
pub mod model;
mod network;
mod overview;
mod region;
mod scene;
pub mod snapshot;
mod subsystems;
mod threads;

use crate::theme::Palette;
use crate::ui::widgets;
use egui::{Rect, Sense, Stroke, Vec2, pos2, vec2};
use model::{Column, RowId};
use snapshot::{History, Snapshot};
use std::collections::HashSet;

pub use model::{MediaProcess, Vram};

/// The sections of the rail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Overview,
    Subsystems,
    Threads,
    Memory,
    Network,
    Disk,
    Scene,
    Region,
}

impl Tab {
    pub const ALL: [Tab; 8] = [
        Tab::Overview,
        Tab::Subsystems,
        Tab::Threads,
        Tab::Memory,
        Tab::Network,
        Tab::Disk,
        Tab::Scene,
        Tab::Region,
    ];

    fn icon(self) -> &'static str {
        match self {
            Tab::Overview => "layout-dashboard-filled",
            Tab::Subsystems => "list-details-filled",
            Tab::Threads => "cpu",
            Tab::Memory => "stack-2-filled",
            Tab::Network => "network",
            Tab::Disk => "database-filled",
            Tab::Scene => "cube",
            Tab::Region => "world-filled",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Tab::Overview => aurora_i18n::tr!("monitor-tab-overview"),
            Tab::Subsystems => aurora_i18n::tr!("monitor-tab-subsystems"),
            Tab::Threads => aurora_i18n::tr!("monitor-tab-threads"),
            Tab::Memory => aurora_i18n::tr!("monitor-tab-memory"),
            Tab::Network => aurora_i18n::tr!("monitor-tab-network"),
            Tab::Disk => aurora_i18n::tr!("monitor-tab-disk"),
            Tab::Scene => aurora_i18n::tr!("monitor-tab-scene"),
            Tab::Region => aurora_i18n::tr!("monitor-tab-region"),
        }
    }

    /// The section a demo or a link names (`AURORA_DEMO_PERF=monitor-…`).
    pub fn from_key(key: &str) -> Option<Tab> {
        Some(match key {
            "overview" => Tab::Overview,
            "subsystems" => Tab::Subsystems,
            "threads" => Tab::Threads,
            "memory" => Tab::Memory,
            "network" => Tab::Network,
            "disk" => Tab::Disk,
            "scene" => Tab::Scene,
            "region" => Tab::Region,
            _ => return None,
        })
    }
}

/// The row of the subsystems' table shown in its detail pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selected {
    Row(RowId),
    /// A media process, by its id.
    Media(u32),
}

/// The preferences page a link of the monitor opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Graphics,
    Interface,
    Chat,
    Network,
    Audio,
}

/// What the monitor asks of the app.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// The compact view of the Performance window.
    Compact,
    /// A report of the snapshot shown, as a CSV file.
    Export,
    Preferences(Page),
    /// The Inspect window on an object (its root prim) or on what an
    /// avatar wears.
    Inspect(uuid::Uuid),
    InspectAvatar(uuid::Uuid),
    RegionWindow,
    TopScripts,
    OpenCacheFolder,
    ClearCache,
}

/// The monitor's state: its history and what the user chose.
pub struct Monitor {
    pub history: History,
    pub tab: Tab,
    /// The history is frozen: nothing is sampled.
    pub paused: bool,
    sort: Column,
    /// The processor as a share of one core rather than of the machine.
    per_core: bool,
    open: HashSet<RowId>,
    selected: Selected,
    filter: String,
    /// The scene's objects rather than its avatars.
    scene_objects: bool,
    /// The scene's sort column (its index among the figures).
    scene_sort: usize,
    /// The last report written, for the status bar.
    pub exported: Option<String>,
}

impl Default for Monitor {
    fn default() -> Self {
        Monitor {
            history: History::default(),
            tab: Tab::Overview,
            paused: false,
            sort: Column::Cpu,
            per_core: false,
            open: [RowId::Render, RowId::Textures].into_iter().collect(),
            selected: Selected::Row(RowId::Textures),
            filter: String::new(),
            scene_objects: false,
            scene_sort: 1,
            exported: None,
        }
    }
}

impl Monitor {
    /// The scene section on its objects (`AURORA_DEMO_PERF=monitor-scene-objects`).
    pub fn show_scene_objects(&mut self) {
        self.tab = Tab::Scene;
        self.scene_objects = true;
    }

    /// The subsystems' table sorted by a column, a row selected (the
    /// overview's "biggest consumer" rows).
    fn open_subsystems(&mut self, sort: Column, row: RowId) {
        self.tab = Tab::Subsystems;
        self.sort = sort;
        self.selected = Selected::Row(row);
    }
}

/// What the status bar says of the graphics card.
pub struct Device<'a> {
    pub name: &'a str,
    pub backend: &'a str,
    pub vram_mb: Option<u64>,
}

const RAIL_W: f32 = 188.0;
const STATUS_H: f32 = 24.0;

/// The view's body, under the window's title bar: the rail, the section,
/// the status bar.
pub fn body(ui: &mut egui::Ui, p: &Palette, m: &mut Monitor, device: &Device, out: &mut Vec<Action>) {
    let full = ui.available_rect_before_wrap();
    let radius = ui.visuals().window_corner_radius;
    let rail = Rect::from_min_max(full.min, pos2(full.left() + RAIL_W, full.bottom() - STATUS_H));
    let content = Rect::from_min_max(pos2(rail.right(), full.top()), pos2(full.right(), full.bottom() - STATUS_H));
    let status = Rect::from_min_max(pos2(full.left(), full.bottom() - STATUS_H), full.max);
    let line = Stroke::new(1.0, p.raised);
    ui.painter().rect_filled(rail, 0.0, p.bar);
    ui.painter().vline(rail.right() - 0.5, rail.y_range(), line);
    ui.painter().rect_filled(
        status,
        egui::CornerRadius {
            sw: radius.sw,
            se: radius.se,
            ..Default::default()
        },
        p.bar,
    );
    ui.painter().hline(status.x_range(), status.top() + 0.5, line);
    ui.painter().hline(full.x_range(), full.top() + 0.5, line);

    ui.scope_builder(egui::UiBuilder::new().max_rect(rail.shrink2(vec2(6.0, 8.0))), |ui| {
        rail_ui(ui, p, m, out);
    });
    // a child, not a scope: a section taller than its room (a larger text)
    // is cut at the status bar instead of making the window taller at
    // every frame
    {
        let ui = &mut ui.new_child(egui::UiBuilder::new().max_rect(content.shrink2(vec2(16.0, 12.0))));
        ui.set_clip_rect(content.intersect(ui.clip_rect()));
        ui.spacing_mut().item_spacing.y = 10.0;
        match m.history.last.clone() {
            None => {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    draw::text(ui, aurora_i18n::tr!("monitor-measuring"), 13.0, p.muted);
                });
            }
            Some(s) => match m.tab {
                Tab::Overview => overview::show(ui, p, m, &s),
                Tab::Subsystems => subsystems::show(ui, p, m, &s, out),
                Tab::Threads => threads::show(ui, p, &s),
                Tab::Memory => memory::show(ui, p, &s, out),
                Tab::Network => network::show(ui, p, m, &s),
                Tab::Disk => disk::show(ui, p, m, &s, out),
                Tab::Scene => scene::show(ui, p, m, &s, out),
                Tab::Region => region::show(ui, p, m, &s, out),
            },
        }
    }
    ui.scope_builder(egui::UiBuilder::new().max_rect(status.shrink2(vec2(12.0, 0.0))), |ui| {
        status_ui(ui, p, m, device);
    });
    ui.advance_cursor_after_rect(full);
}

/// A section's title, its subtitle, and its controls on the right.
fn heading(ui: &mut egui::Ui, p: &Palette, title: &str, sub: &str, controls: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.set_min_height(28.0);
        ui.label(egui::RichText::new(title).size(15.0).strong().color(p.ink));
        ui.add_space(4.0);
        if !sub.is_empty() {
            ui.add(egui::Label::new(egui::RichText::new(sub).size(draw::BODY).color(p.muted)).truncate());
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), controls);
    });
}

fn rail_ui(ui: &mut egui::Ui, p: &Palette, m: &mut Monitor, out: &mut Vec<Action>) {
    let last = m.history.last.clone();
    ui.spacing_mut().item_spacing.y = 2.0;
    for tab in Tab::ALL {
        let sub = last.as_ref().map(|s| rail_sub(tab, m, s)).unwrap_or_default();
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 42.0), Sense::click());
        let on = m.tab == tab;
        if on {
            ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.raised);
        } else if resp.hovered() {
            ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.panel);
        }
        let icon_color = if on { p.violet_light } else { p.muted };
        widgets::paint_glyph(
            ui.painter(),
            tab.icon(),
            pos2(rect.left() + 18.0, rect.center().y),
            16.0,
            icon_color,
        );
        let left = rect.left() + 36.0;
        let room = rect.right() - left - 6.0;
        widgets::paint_elided(
            ui,
            tab.label(),
            pos2(left, rect.center().y - 8.0),
            room,
            egui::FontId::proportional(13.0),
            if on || resp.hovered() { p.ink } else { p.muted },
        );
        widgets::paint_elided(
            ui,
            &sub,
            pos2(left, rect.center().y + 9.0),
            room,
            egui::FontId::proportional(draw::SMALL),
            if on { p.muted } else { p.muted_dim },
        );
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, tab.label()));
        if resp.clicked() {
            m.tab = tab;
        }
    }
    // the frame rate at the foot, as the compact view shows it
    let foot_h = 104.0;
    let room = ui.available_height();
    if room > foot_h + 8.0 {
        ui.add_space(room - foot_h);
        ui.painter()
            .hline(ui.max_rect().x_range(), ui.cursor().top() - 1.0, Stroke::new(1.0, p.raised));
        ui.add_space(8.0);
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.vertical(|ui| {
                draw::text(ui, aurora_i18n::tr!("monitor-frames-per-second"), draw::SMALL, p.muted_dim);
                let fps = last.as_ref().map_or(0.0, |s| s.fps);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    draw::text(ui, aurora_i18n::number(f64::from(fps), 0), 22.0, fps_color(p, fps));
                    if let Some(s) = &last {
                        draw::text(ui, fmt::ms(f64::from(s.frame_ms)), draw::SMALL, p.muted);
                    }
                });
                let fps_series = m.history.series(|pt| Some(f64::from(pt.fps)));
                let top = draw::nice_max(fps_series.iter().copied().fold(60.0, f64::max) * 1.1);
                draw::graph(
                    ui,
                    p,
                    vec2(ui.available_width() - 8.0, 30.0),
                    &[draw::Line {
                        values: &fps_series,
                        color: p.violet_light,
                        fill: true,
                    }],
                    top,
                    snapshot::HISTORY,
                    None,
                );
                if draw::link(ui, p, aurora_i18n::tr!("ui-perf-compact-version")).clicked() {
                    out.push(Action::Compact);
                }
            });
        });
    }
}

/// What a section's rail entry says under its name.
fn rail_sub(tab: Tab, m: &Monitor, s: &Snapshot) -> String {
    use aurora_i18n::tr;
    let a = &s.attribution;
    match tab {
        Tab::Overview => tr!(
            "monitor-rail-overview",
            fps = aurora_i18n::number(f64::from(s.fps), 0),
            cpu = s.cpu.as_ref().map_or("–".into(), |c| fmt::pct(f64::from(c.process)))
        ),
        Tab::Subsystems => tr!(
            "monitor-rail-subsystems",
            count = RowId::parents().count() as i64,
            column = subsystems::column_title(m.sort, m.per_core).to_lowercase()
        ),
        Tab::Threads => match &s.cpu {
            Some(c) => tr!(
                "monitor-rail-threads",
                used = aurora_i18n::number(f64::from(c.process) * c.cores as f64 / 100.0, 1),
                cores = c.cores as i64
            ),
            None => "–".into(),
        },
        Tab::Memory => tr!(
            "monitor-rail-memory",
            ram = s.memory.map_or("–".into(), |mem| fmt::bytes(mem.working_set as f64)),
            vram = s.vram.usage.map_or("–".into(), |v| fmt::bytes(v as f64))
        ),
        Tab::Network => format!(
            "↓ {} · ↑ {}",
            fmt::kbps(f64::from(s.net.in_kbps)),
            fmt::kbps(f64::from(s.net.out_kbps))
        ),
        Tab::Disk => tr!(
            "monitor-rail-disk",
            rate = fmt::disk_rate(a.total(Column::Disk).unwrap_or(0.0)),
            cache = fmt::bytes(s.disk.used as f64)
        ),
        Tab::Scene => tr!(
            "monitor-rail-scene",
            avatars = s.scene.avatar_count as i64,
            objects = fmt::count(s.scene.object_count)
        ),
        Tab::Region => match s.region.as_ref().and_then(|r| r.dilation) {
            Some(d) => tr!("monitor-rail-region", dilation = aurora_i18n::number(f64::from(d), 2)),
            None => tr!("monitor-no-region-stats").to_owned(),
        },
    }
}

fn fps_color(p: &Palette, fps: f32) -> egui::Color32 {
    if fps >= 55.0 {
        p.teal
    } else if fps >= 28.0 {
        p.amber
    } else {
        p.danger
    }
}

fn status_ui(ui: &mut egui::Ui, p: &Palette, m: &Monitor, device: &Device) {
    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        let left = if m.paused {
            aurora_i18n::tr!("monitor-status-paused")
        } else {
            aurora_i18n::tr!("monitor-status-live")
        };
        draw::text(ui, left, draw::SMALL, p.muted_dim);
        if let Some(s) = &m.history.last {
            let cost = s.attribution.get(RowId::UiMonitor).get(Column::Ms).unwrap_or(0.0);
            draw::text(
                ui,
                aurora_i18n::tr!("monitor-status-cost", cost = fmt::ms(cost)),
                draw::SMALL,
                p.muted_dim,
            );
        }
        if let Some(file) = &m.exported {
            draw::text(
                ui,
                aurora_i18n::tr!("monitor-status-exported", file = file.clone()),
                draw::SMALL,
                p.muted_dim,
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let vram = device
                .vram_mb
                .map(|mb| format!(" · {}", fmt::bytes(mb as f64 * 1024.0 * 1024.0)))
                .unwrap_or_default();
            draw::text(
                ui,
                format!("{}{vram} · {} (wgpu)", device.name, device.backend),
                draw::SMALL,
                p.muted_dim,
            );
        });
    });
}

/// A snapshot as a CSV report (`;` between the fields, numbers with a
/// dot): every part and sub-part with its seven columns, then the media
/// processes, then the groups of threads.
pub fn export_csv(s: &Snapshot) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let num = |v: Option<f64>| v.map_or(String::new(), |v| format!("{v:.3}"));
    let _ = write!(out, "{}", aurora_i18n::tr!("monitor-col-subsystem"));
    for (c, unit) in Column::ALL.iter().zip(["%", "ms", "ms", "B", "B", "kb/s", "B/s"]) {
        let _ = write!(out, ";{} ({unit})", subsystems::column_title(*c, false));
    }
    out.push('\n');
    for id in RowId::ALL {
        let name = subsystems::row_name(*id);
        let indent = if id.parent().is_some() { "  " } else { "" };
        let _ = write!(out, "{indent}{}", name.replace(';', ","));
        let u = s.attribution.get(*id);
        for c in Column::ALL {
            let _ = write!(out, ";{}", num(u.get(c)));
        }
        out.push('\n');
    }
    for (proc_, u) in &s.attribution.media {
        let _ = write!(out, "  SLPlugin {} {}", proc_.pid, proc_.label.replace(';', ","));
        for c in Column::ALL {
            let _ = write!(out, ";{}", num(u.get(c)));
        }
        out.push('\n');
    }
    if let Some(cpu) = &s.cpu {
        let _ = writeln!(
            out,
            "\n{};{};{} (%)",
            aurora_i18n::tr!("monitor-col-thread"),
            aurora_i18n::tr!("monitor-col-count"),
            aurora_i18n::tr!("monitor-col-cpu-core")
        );
        for g in &cpu.groups {
            let _ = writeln!(out, "{};{};{:.2}", g.group, g.threads, g.percent);
        }
    }
    out
}

/// The demo's section (`AURORA_DEMO_PERF=monitor-<section>`) for the next
/// frames the window draws.
pub fn demo_tab(ctx: &egui::Context, tab: Tab) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique(("monitor", "demo-tab")), tab_index(tab)));
}

fn tab_index(tab: Tab) -> usize {
    Tab::ALL.iter().position(|t| *t == tab).unwrap_or(0)
}

/// The section asked by [`demo_tab`] or `perf::show_network`, once.
pub fn take_asked_tab(ctx: &egui::Context) -> Option<Tab> {
    ctx.data_mut(|d| d.remove_temp::<usize>(egui::Id::unique(("monitor", "demo-tab"))))
        .and_then(|i| Tab::ALL.get(i).copied())
}

/// The size the full view opens at, when nothing remembers another.
pub const DEFAULT_SIZE: Vec2 = Vec2::new(1180.0, 760.0);
/// The least size of the full view.
pub const MIN_SIZE: Vec2 = Vec2::new(860.0, 540.0);
