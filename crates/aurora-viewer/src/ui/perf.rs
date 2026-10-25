//! Performance window. The full view by default, the resource monitor
//! (`ui/monitor`); the compact one shows the frame rate, the frame times and
//! their graph. A title bar button goes from one to the other.

use crate::scene::SceneStats;
use crate::theme::Palette;
use crate::ui::widgets::Floater;
use aurora_net::NetStatsSnapshot;
use aurora_render::{GpuElement, GpuInfo, RenderStats};
use egui::{Color32, Rect, RichText, Sense, Stroke, pos2, vec2};
use std::collections::VecDeque;
use std::time::Instant;

/// Frames kept for the graph and the min / max (4 s at 60 images/s).
const HISTORY: usize = 240;
/// Time constant of the breakdown smoothing (s): calm bars at any frame rate.
const SMOOTHING_S: f32 = 0.6;
/// Time constant of the graph scale easing down (s).
const GRAPH_EASE_S: f32 = 1.5;
/// How often the numbers are refreshed (s): slow enough to be read.
const REFRESH_S: f32 = 0.5;
pub(crate) const COMPACT_W: f32 = 236.0;
/// The two sides of the window around its content: margins and border.
pub(crate) const SIDES: f32 = 22.0;
/// Height of the compact window, as it lays itself out: only to place the
/// other windows of a first launch around it.
pub(crate) const COMPACT_H: f32 = 182.0;
const GRAPH_H: f32 = 64.0;
/// Lowest top of the graph scale (images/s).
const GRAPH_MIN_TOP: f32 = 72.0;

/// Network rate samples kept for the graph (one per second).
const NET_HISTORY: usize = 60;
/// Lowest top of the network graph scale (kb/s).
const NET_MIN_TOP: f32 = 10.0;

/// CPU parts of a frame, in [`cpu_parts`] order.
const CPU_PARTS: usize = 7;

/// Frame times (ms) over the history.
#[derive(Default, Clone, Copy, Debug, PartialEq)]
pub struct FrameStats {
    pub avg: f32,
    pub min: f32,
    pub max: f32,
}

fn frame_stats(frames: &VecDeque<f32>) -> FrameStats {
    if frames.is_empty() {
        return FrameStats::default();
    }
    let (mut min, mut max, mut sum) = (f32::MAX, 0.0f32, 0.0);
    for &f in frames {
        min = min.min(f);
        max = max.max(f);
        sum += f;
    }
    FrameStats {
        avg: sum / frames.len() as f32,
        min,
        max,
    }
}

/// A rate in the unit that reads best: kb/s, Mb/s or Gb/s.
pub(crate) fn fmt_rate(kbps: f32) -> String {
    if kbps >= 1e6 {
        format!("{:.2} Gb/s", kbps / 1e6)
    } else if kbps >= 1e3 {
        format!("{:.1} Mb/s", kbps / 1e3)
    } else {
        format!("{kbps:.0} kb/s")
    }
}

fn fps_of(ms: f32) -> f32 {
    1000.0 / ms.max(0.1)
}

/// Top of the graph scale (images/s) for these frames: above the fast
/// ones, ignoring the 2 % fastest (a lone very short frame after a long
/// one would squash the whole graph).
fn graph_top(frames: &VecDeque<f32>) -> f32 {
    let mut fps: Vec<f32> = frames.iter().map(|&ms| fps_of(ms)).collect();
    if fps.is_empty() {
        return GRAPH_MIN_TOP;
    }
    fps.sort_by(f32::total_cmp);
    let fast = fps[(fps.len() - 1) * 98 / 100];
    (fast * 1.15).max(GRAPH_MIN_TOP)
}

/// Step between the graph marks (images/s, kb/s): the smallest round
/// value (1, 2 or 5 times a power of ten) giving at most three marks under
/// `top`, far enough apart for their labels.
fn grid_step(top: f32) -> f32 {
    let min = (top / 4.0).max(1e-3);
    let mut pow = 10f32.powf(min.log10().floor());
    loop {
        for nice in [1.0, 2.0, 5.0] {
            if nice * pow >= min {
                return nice * pow;
            }
        }
        pow *= 10.0;
    }
}

/// CPU parts of a frame (ms): network → world, scene sync, culling and
/// lists, the rest of the update, interface, GPU encoding, and the time
/// left (vsync, frame limiter, present). Sync and culling run inside the
/// update.
fn cpu_parts(frame: f32, events: f32, update: f32, sync: f32, cull: f32, ui: f32, encode: f32) -> [f32; CPU_PARTS] {
    let other = (update - sync - cull).max(0.0);
    let busy = events + sync + cull + other + ui + encode;
    [events, sync, cull, other, ui, encode, (frame - busy).max(0.0)]
}

/// Weight of a new sample in an exponential moving average with time
/// constant `tau_s`, after `dt_ms` (independent of the frame rate).
fn ease(dt_ms: f32, tau_s: f32) -> f32 {
    1.0 - (-dt_ms / 1000.0 / tau_s).exp()
}

fn smooth<const N: usize>(avg: &mut [f32; N], now: &[f32; N], k: f32) {
    for (a, n) in avg.iter_mut().zip(now) {
        *a += (n - *a) * k;
    }
}

pub struct PerfData {
    pub frame_ms: VecDeque<f32>,
    pub fps: f32,
    /// Frame times, refreshed with `fps`.
    pub frames: FrameStats,
    last_fps_update: Instant,
    frames_since: u32,
    /// Counters at the last rate sample; None before the first frame (the
    /// first rate must not count everything received since the start).
    pub net_prev: Option<NetStatsSnapshot>,
    pub net_rate: (f32, f32, f32, f32), // pkts in/s, pkts out/s, kbps in, kbps out
    last_net: Instant,
    pub http_rate_kbps: f32,
    // per-frame CPU timings (ms)
    pub events_ms: f32,
    pub update_ms: f32,
    pub ui_ms: f32,
    /// Smoothed GPU time by element (ms), drawn by the bar; None until the
    /// GPU reports it (or never, without in-pass timestamps).
    gpu: Option<[f32; GpuElement::ALL.len()]>,
    /// The same, refreshed with `fps`, for the numbers.
    gpu_shown: Option<[f32; GpuElement::ALL.len()]>,
    cpu: [f32; CPU_PARTS],
    cpu_shown: [f32; CPU_PARTS],
    /// Time of the render thread on a frame (ms), smoothed, and the value
    /// refreshed with `fps`; 0 without a render thread.
    render_thread: f32,
    pub render_thread_shown: f32,
    /// Top of the graph scale (images/s): follows the fast frames, eases down.
    graph_top: f32,
    /// Network rates (kb/s) per second: (in, UDP + HTTP; out).
    net_hist: VecDeque<(f32, f32)>,
    /// The last one, smoothed for the bar.
    net_live: [f32; 2],
    /// Top of the network graph scale (kb/s), eased like `graph_top`.
    net_top: f32,
}

impl Default for PerfData {
    fn default() -> Self {
        PerfData {
            frame_ms: VecDeque::with_capacity(HISTORY),
            fps: 0.0,
            frames: FrameStats::default(),
            last_fps_update: Instant::now(),
            frames_since: 0,
            net_prev: None,
            net_rate: (0.0, 0.0, 0.0, 0.0),
            last_net: Instant::now(),
            http_rate_kbps: 0.0,
            events_ms: 0.0,
            update_ms: 0.0,
            ui_ms: 0.0,
            gpu: None,
            gpu_shown: None,
            cpu: [0.0; CPU_PARTS],
            cpu_shown: [0.0; CPU_PARTS],
            render_thread: 0.0,
            render_thread_shown: 0.0,
            graph_top: GRAPH_MIN_TOP,
            net_hist: VecDeque::with_capacity(NET_HISTORY),
            net_live: [0.0; 2],
            net_top: NET_MIN_TOP,
        }
    }
}

impl PerfData {
    /// The frame's CPU parts and, when the GPU times them, its GPU
    /// elements (ms), as last refreshed for reading.
    pub fn frame_parts(&self) -> ([f32; CPU_PARTS], Option<[f32; GpuElement::ALL.len()]>) {
        (self.cpu_shown, self.gpu_shown)
    }

    pub fn frame(&mut self, dt_ms: f32, net: NetStatsSnapshot, render: &RenderStats, scene: &SceneStats) {
        if self.frame_ms.len() >= HISTORY {
            self.frame_ms.pop_front();
        }
        self.frame_ms.push_back(dt_ms);
        let cpu = cpu_parts(
            dt_ms,
            self.events_ms,
            self.update_ms,
            scene.sync_ms,
            scene.cull_ms,
            self.ui_ms,
            render.cpu_encode_ms,
        );
        let k = ease(dt_ms, SMOOTHING_S);
        smooth(&mut self.cpu, &cpu, k);
        // the render thread works beside the main thread: not a part of
        // the bar above, which adds up to the frame time
        self.render_thread += (render.thread_ms - self.render_thread) * k;
        self.gpu = match (render.gpu_elements, self.gpu) {
            (Some(now), Some(mut avg)) => {
                smooth(&mut avg, &now, k);
                Some(avg)
            }
            (now, _) => now,
        };
        let target = graph_top(&self.frame_ms);
        self.graph_top = if target > self.graph_top {
            target
        } else {
            self.graph_top + (target - self.graph_top) * ease(dt_ms, GRAPH_EASE_S)
        };

        self.frames_since += 1;
        let el = self.last_fps_update.elapsed().as_secs_f32();
        if el >= REFRESH_S {
            self.fps = self.frames_since as f32 / el;
            self.frames_since = 0;
            self.last_fps_update = Instant::now();
            self.frames = frame_stats(&self.frame_ms);
            self.gpu_shown = self.gpu;
            self.cpu_shown = self.cpu;
            self.render_thread_shown = self.render_thread;
        }
        let p = *self.net_prev.get_or_insert_with(|| {
            self.last_net = Instant::now();
            net
        });
        let nel = self.last_net.elapsed().as_secs_f32();
        if nel >= 1.0 {
            self.net_rate = (
                net.packets_in.saturating_sub(p.packets_in) as f32 / nel,
                net.packets_out.saturating_sub(p.packets_out) as f32 / nel,
                net.bytes_in.saturating_sub(p.bytes_in) as f32 * 8.0 / 1000.0 / nel,
                net.bytes_out.saturating_sub(p.bytes_out) as f32 * 8.0 / 1000.0 / nel,
            );
            self.http_rate_kbps = net.http_bytes.saturating_sub(p.http_bytes) as f32 * 8.0 / 1000.0 / nel;
            self.net_prev = Some(net);
            self.last_net = Instant::now();
            if self.net_hist.len() >= NET_HISTORY {
                self.net_hist.pop_front();
            }
            self.net_hist.push_back((self.net_rate.2 + self.http_rate_kbps, self.net_rate.3));
        }
        if let Some(&(i, o)) = self.net_hist.back() {
            smooth(&mut self.net_live, &[i, o], k);
        }
        let busiest = self.net_hist.iter().fold(0.0f32, |m, &(i, o)| m.max(i).max(o));
        let target = (busiest * 1.15).max(NET_MIN_TOP);
        self.net_top = if target > self.net_top {
            target
        } else {
            self.net_top + (target - self.net_top) * ease(dt_ms, GRAPH_EASE_S)
        };
    }
}

pub struct PerfView<'a> {
    pub data: &'a PerfData,
    pub gpu: &'a GpuInfo,
}

/// The full view (the resource monitor) rather than the compact one;
/// persisted with the UI layout (ui_layout.ron), the full view by default.
fn full_id() -> egui::Id {
    egui::Id::unique(("perf", "monitor"))
}

pub fn set_full(ctx: &egui::Context, full: bool) {
    ctx.data_mut(|d| d.insert_persisted(full_id(), full));
}

/// The view shown now: the user's choice, or the full view asked for this
/// opening only (`show_network`).
pub fn is_full(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(full_once_id())).unwrap_or(false)
        || ctx.data_mut(|d| d.get_persisted::<bool>(full_id())).unwrap_or(true)
}

/// The full view for this opening only (`show_network`): the saved choice
/// of the window's button stays.
fn full_once_id() -> egui::Id {
    egui::Id::unique(("perf", "full-once"))
}

/// The window on the monitor's Network section, from the bars (the system
/// island's right click, its tooltip, the traffic indicator): the full view
/// for this opening only. The caller opens the window.
pub fn show_network(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(full_once_id(), true));
    super::monitor::demo_tab(ctx, super::monitor::Tab::Network);
}

/// A placement under the system island asked (`place_under`), for the
/// next frame the window is open.
fn place_id() -> egui::Id {
    egui::Id::unique(("perf", "place-under"))
}

/// The system island of this opening (`island`).
fn island_id() -> egui::Id {
    egui::Id::unique(("perf", "island"))
}

/// The window was shown last frame.
fn shown_id() -> egui::Id {
    egui::Id::unique(("perf", "shown"))
}

/// Where the full view stood and its size when last shown (persisted).
fn monitor_rect_id() -> egui::Id {
    egui::Id::unique(("perf", "monitor-rect"))
}

/// Performance opened from the system island of Aurora's bars (its frame
/// rate, its tooltip's network row, the traffic indicator, its right
/// click): the full view 8 px under the island, its right edge on the
/// island's, for as long as it stays open (moved, it stays where it was
/// put); the compact view keeps its own size and place. An open window
/// stays where it is.
pub fn place_under(ctx: &egui::Context, island: Rect) {
    if !ctx.data(|d| d.get_temp::<bool>(shown_id())).unwrap_or(false) {
        ctx.data_mut(|d| d.insert_temp(place_id(), island));
    }
}

/// The system island Performance opened from, for this opening.
fn island(ctx: &egui::Context) -> Option<Rect> {
    ctx.data(|d| d.get_temp::<Rect>(island_id()))
}

/// Each view's place and size before the window is laid out, each time it
/// opens or changes view. The full view: under the system island when
/// Performance opened from it (`place_under`), else where it stood when
/// last shown, else in the middle of the work area; the size it had, the
/// monitor's own at first, never larger than the work area. The compact
/// view keeps its own size and place: back from a full view, it returns
/// where it stood.
fn place(ctx: &egui::Context, work: Rect, full: bool, open: bool) {
    let was_shown = ctx.data(|d| d.get_temp::<bool>(shown_id())).unwrap_or(false);
    let was_full = ctx.data(|d| d.get_temp::<bool>(shown_id().with("full"))).unwrap_or(false);
    let compact_pos_id = shown_id().with("compact-pos");
    if !open {
        ctx.data_mut(|d| {
            d.remove_temp::<bool>(shown_id());
            d.remove::<Rect>(island_id());
        });
        return;
    }
    ctx.data_mut(|d| {
        d.insert_temp(shown_id(), true);
        d.insert_temp(shown_id().with("full"), full);
        // asked from the island: for this opening
        if let Some(island) = d.get_temp::<Rect>(place_id()) {
            d.remove::<Rect>(place_id());
            d.insert_temp(island_id(), island);
        }
    });
    let stands = || {
        egui::AreaState::load(ctx, egui::Id::unique("perf"))
            .filter(|s| s.size.is_some())
            .map(|s| s.left_top_pos())
    };
    if !full {
        if !was_full {
            // where the compact view stands, for its return
            if let Some(pos) = stands() {
                ctx.data_mut(|d| d.insert_temp(compact_pos_id, pos));
            }
        } else {
            let pos = ctx
                .data(|d| d.get_temp::<egui::Pos2>(compact_pos_id))
                .or_else(stands)
                .unwrap_or_else(|| default_pos(work, COMPACT_W));
            super::widgets::place_floater(ctx, "perf", pos, vec2(COMPACT_W + SIDES, COMPACT_H));
        }
        return;
    }
    if was_shown && was_full {
        // the full view as it stands, remembered for its next opening
        if let Some(s) = egui::AreaState::load(ctx, egui::Id::unique("perf")).filter(|s| s.size.is_some()) {
            ctx.data_mut(|d| d.insert_persisted(monitor_rect_id(), s.rect()));
        }
        return;
    }
    let last = ctx.data_mut(|d| d.get_persisted::<Rect>(monitor_rect_id()));
    let size = last.map_or(super::monitor::DEFAULT_SIZE, |r| r.size()).min(work.size());
    let top_left = match island(ctx) {
        Some(island) => pos2(island.right() - size.x, island.bottom() + super::bars::islands::MARGIN),
        None => last.map_or_else(|| work.center() - size / 2.0, |r| r.min),
    };
    super::widgets::place_floater(ctx, "perf", top_left, size);
}

/// Where the window stands before the user moves it: under the mini-map of
/// a first launch (`minimap::default_rect`), on its right edge.
pub(crate) fn default_pos(work: egui::Rect, width: f32) -> egui::Pos2 {
    let map = super::minimap::default_rect(work);
    pos2(map.right() - width - SIDES, map.bottom() + super::layout::GAP)
}

/// The window: the compact view (frame rate and its graph), or the full
/// one, the resource monitor; a title bar button goes from one to the
/// other. Returns what the monitor asks of the app.
pub fn show(
    ctx: &egui::Context,
    p: &Palette,
    v: &PerfView,
    monitor: &mut super::monitor::Monitor,
    open: &mut bool,
) -> Vec<super::monitor::Action> {
    use super::monitor::Action;
    let full = is_full(ctx);
    let work = super::layout::work_area(ctx);
    place(ctx, work, full, *open);
    let mut actions = Vec::new();
    if full {
        if let Some(tab) = super::monitor::take_asked_tab(ctx) {
            monitor.tab = tab;
        }
        let (mut compact, mut export, mut pause) = (false, false, false);
        let (dot, state) = if monitor.paused {
            (p.muted_dim, aurora_i18n::tr!("monitor-paused"))
        } else {
            (p.success, aurora_i18n::tr!("monitor-live"))
        };
        let (pause_icon, pause_tip) = if monitor.paused {
            ("player-play-filled", aurora_i18n::tr!("monitor-resume"))
        } else {
            ("player-pause-filled", aurora_i18n::tr!("monitor-pause"))
        };
        let device = super::monitor::Device {
            name: &v.gpu.name,
            backend: &v.gpu.backend,
            vram_mb: v.gpu.vram_mb,
        };
        let mut floater = Floater::new(
            "perf",
            aurora_i18n::tr!("monitor-title"),
            work.center() - super::monitor::DEFAULT_SIZE / 2.0,
            super::monitor::DEFAULT_SIZE,
        )
        .flush_body()
        .help(aurora_i18n::tr!("monitor-help"))
        .title_state(dot, state)
        .action("arrows-minimize", aurora_i18n::tr!("ui-perf-compact-version"), &mut compact)
        .action("file-export", aurora_i18n::tr!("monitor-export"), &mut export)
        .action(pause_icon, pause_tip, &mut pause);
        floater.min_size = super::monitor::MIN_SIZE;
        floater.show(ctx, p, open, |ui| {
            super::monitor::body(ui, p, monitor, &device, &mut actions);
        });
        if pause {
            monitor.paused = !monitor.paused;
        }
        if export {
            actions.push(Action::Export);
        }
        if compact {
            actions.push(Action::Compact);
        }
        if actions.contains(&Action::Compact) {
            actions.retain(|a| *a != Action::Compact);
            set_full(ctx, false);
            ctx.data_mut(|d| d.remove_temp::<bool>(full_once_id()));
        }
    } else {
        let mut toggle = false;
        Floater::new(
            "perf",
            aurora_i18n::tr!("common-performance"),
            default_pos(work, COMPACT_W),
            vec2(COMPACT_W, 0.0),
        )
        .fixed()
        .help(aurora_i18n::tr!("ui-perf-the-graph-shows-fps-over-the-last-few-seconds"))
        .action("arrows-maximize", aurora_i18n::tr!("monitor-open-full"), &mut toggle)
        .show(ctx, p, open, |ui| {
            ui.set_width(COMPACT_W);
            headline(ui, p, v.data);
            ui.add_space(6.0);
            graph(ui, p, v.data);
        });
        if toggle {
            set_full(ctx, true);
        }
    }
    // the view of one opening ends with it
    if !*open {
        ctx.data_mut(|d| d.remove_temp::<bool>(full_once_id()));
    }
    actions
}

fn fps_color(p: &Palette, fps: f32) -> Color32 {
    if fps >= 55.0 {
        p.teal
    } else if fps >= 28.0 {
        p.amber
    } else {
        p.danger
    }
}

/// Frame rate on the left, average / min / max frame time on the right.
fn headline(ui: &mut egui::Ui, p: &Palette, d: &PerfData) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        ui.label(
            RichText::new(aurora_i18n::number((d.fps) as f64, 0))
                .size(28.0)
                .strong()
                .color(fps_color(p, d.fps)),
        );
        ui.vertical(|ui| {
            ui.add_space(12.0);
            ui.label(RichText::new("images/s").size(11.0).color(p.muted));
        });
        ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add_space(3.0);
            ui.label(RichText::new(format!("{:.1} ms", d.frames.avg)).size(15.0).color(p.ink));
            ui.label(
                RichText::new(aurora_i18n::tr!(
                    "status-min-max",
                    value0 = aurora_i18n::number_text(&format!("{:.1}", d.frames.min)),
                    value1 = aurora_i18n::number_text(&format!("{:.1}", d.frames.max))
                ))
                .size(11.0)
                .color(p.muted),
            );
        });
    });
}

/// Images/s of the last seconds, frame by frame (a dip is a hitch): a line
/// over a light area, the 60 and 30 images/s marks, and the frame under
/// the pointer.
fn graph(ui: &mut egui::Ui, p: &Palette, d: &PerfData) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), GRAPH_H), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, ui.visuals().window_corner_radius, p.field);
    let top = d.graph_top.max(1.0);
    let plot = rect.shrink2(vec2(0.0, 4.0));
    let y = |fps: f32| plot.bottom() - (fps / top).min(1.0) * plot.height();

    // marks at round values that follow the scale (50 / 100 / 150 at
    // 160 images/s, 20 / 40 / 60 at 60)
    let mark_font = egui::FontId::proportional(9.0);
    let grid = grid_step(top);
    let mut fps = grid;
    while fps < top * 0.95 {
        let ly = y(fps).round() + 0.5;
        painter.extend(egui::Shape::dashed_line(
            &[pos2(rect.left() + 4.0, ly), pos2(rect.right() - 42.0, ly)],
            Stroke::new(1.0, p.raised),
            3.0,
            3.0,
        ));
        painter.text(
            pos2(rect.right() - 4.0, ly),
            egui::Align2::RIGHT_CENTER,
            aurora_i18n::tr!("status-fps-fps", fps = aurora_i18n::number_text(&format!("{:.0}", fps))),
            mark_font.clone(),
            p.muted_dim,
        );
        fps += grid;
    }

    let n = d.frame_ms.len();
    if n < 2 {
        return;
    }
    let step = rect.width() / (HISTORY - 1) as f32;
    let x0 = rect.right() - (n - 1) as f32 * step;
    let points: Vec<egui::Pos2> = d
        .frame_ms
        .iter()
        .enumerate()
        .map(|(i, &ms)| pos2(x0 + i as f32 * step, y(fps_of(ms))))
        .collect();
    // area under the line: a strip of quads down to the bottom
    let fill = p.violet.gamma_multiply(0.14);
    let mut mesh = egui::Mesh::default();
    for (i, pt) in points.iter().enumerate() {
        mesh.colored_vertex(*pt, fill);
        mesh.colored_vertex(pos2(pt.x, rect.bottom()), fill);
        if i > 0 {
            let k = i as u32 * 2;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k + 1, k);
        }
    }
    painter.add(mesh);
    painter.add(egui::Shape::line(points.clone(), Stroke::new(1.5, p.violet_light)));

    if let Some(pos) = resp.hover_pos() {
        let i = (((pos.x - x0) / step).round().max(0.0) as usize).min(n - 1);
        let pt = points[i];
        let ms = d.frame_ms[i];
        painter.line_segment([pos2(pt.x, rect.top()), pos2(pt.x, rect.bottom())], Stroke::new(1.0, p.muted_dim));
        painter.circle(pt, 3.5, p.violet_light, Stroke::new(2.0, p.field));
        let text = aurora_i18n::tr!(
            "perf-frame-tooltip",
            fps = aurora_i18n::number(fps_of(ms) as f64, 0),
            ms = aurora_i18n::number(ms as f64, 1)
        );
        let galley = painter.layout_no_wrap(text, egui::FontId::proportional(11.0), p.ink);
        let size = galley.size() + vec2(10.0, 4.0);
        // beside the line, on the side with room
        let left = pt.x + 6.0 + size.x > rect.right();
        let x = if left { pt.x - 6.0 - size.x } else { pt.x + 6.0 };
        let r = Rect::from_min_size(pos2(x, rect.top() + 3.0), size);
        painter.rect_filled(r, ui.visuals().window_corner_radius, p.panel);
        painter.galley(r.min + vec2(5.0, 2.0), galley, p.ink);
    }
}

/// The network rates of the last minute in `rect`: incoming, a line over a
/// light area, and outgoing, on the window's scale, `inset` above and
/// below. Shared by the window's graph and the islands' small one (the
/// system island's tooltip and traffic indicator), on the same data; None
/// before two samples. Returns the two lines' points.
pub fn net_curve(painter: &egui::Painter, p: &Palette, rect: Rect, inset: f32, d: &PerfData) -> Option<(Vec<egui::Pos2>, Vec<egui::Pos2>)> {
    let n = d.net_hist.len();
    if n < 2 {
        return None;
    }
    let top = d.net_top.max(1.0);
    let plot = rect.shrink2(vec2(0.0, inset));
    let y = |kbps: f32| plot.bottom() - (kbps / top).min(1.0) * plot.height();
    let step = rect.width() / (NET_HISTORY - 1) as f32;
    let x0 = rect.right() - (n - 1) as f32 * step;
    let x = |i: usize| x0 + i as f32 * step;
    let ins: Vec<egui::Pos2> = d.net_hist.iter().enumerate().map(|(i, s)| pos2(x(i), y(s.0))).collect();
    let outs: Vec<egui::Pos2> = d.net_hist.iter().enumerate().map(|(i, s)| pos2(x(i), y(s.1))).collect();
    let fill = p.teal.gamma_multiply(0.12);
    let mut mesh = egui::Mesh::default();
    for (i, pt) in ins.iter().enumerate() {
        mesh.colored_vertex(*pt, fill);
        mesh.colored_vertex(pos2(pt.x, rect.bottom()), fill);
        if i > 0 {
            let k = i as u32 * 2;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k + 1, k);
        }
    }
    painter.add(mesh);
    painter.add(egui::Shape::line(outs.clone(), Stroke::new(1.5, p.rose)));
    painter.add(egui::Shape::line(ins.clone(), Stroke::new(1.5, p.teal)));
    Some((ins, outs))
}

/// The incoming rate of the last sample (kb/s, UDP and HTTP), the
/// outgoing one.
pub fn last_rates(d: &PerfData) -> (f32, f32) {
    d.net_hist.back().copied().unwrap_or_default()
}

/// The GPU parts of a frame, in [`GpuElement::ALL`] order: name and help
/// (the resource monitor's breakdown).
pub(crate) fn gpu_part_names() -> [(&'static str, &'static str); 7] {
    GpuElement::ALL.map(|el| match el {
        GpuElement::ShadowsDepth => (
            aurora_i18n::tr!("ui-perf-shadows-and-depth"),
            aurora_i18n::tr!("ui-perf-sun-shadows-depth-prepass-occlusion"),
        ),
        GpuElement::Effects => (
            aurora_i18n::tr!("ui-perf-effects"),
            aurora_i18n::tr!("ui-perf-ambient-occlusion-reflections-water-mirrors-probes-screen-space-reflections"),
        ),
        GpuElement::TerrainSky => (
            aurora_i18n::tr!("ui-perf-terrain-and-sky"),
            aurora_i18n::tr!("ui-perf-region-terrain-and-sky"),
        ),
        GpuElement::Objects => (
            aurora_i18n::tr!("ui-perf-objects-and-avatars"),
            aurora_i18n::tr!("ui-perf-opaque-or-masked-prims-meshes-and-avatars-impostors"),
        ),
        GpuElement::Water => (
            aurora_i18n::tr!("common-water"),
            aurora_i18n::tr!("ui-perf-water-surface-refraction-waves"),
        ),
        GpuElement::Transparent => (
            aurora_i18n::tr!("ui-perf-transparent-objects-and-particles"),
            aurora_i18n::tr!("ui-perf-semi-transparent-faces-and-particles"),
        ),
        GpuElement::Post => (
            aurora_i18n::tr!("ui-perf-post-processing"),
            aurora_i18n::tr!("ui-perf-glow-anti-aliasing-taa-smaa-tone-mapping"),
        ),
    })
}

/// The CPU parts of a frame, in [`cpu_parts`] order: name and help.
pub(crate) fn cpu_part_names() -> [(&'static str, &'static str); CPU_PARTS] {
    [
        (
            aurora_i18n::tr!("ui-perf-network-to-world"),
            aurora_i18n::tr!("ui-perf-simulator-messages-applied-to-world"),
        ),
        (
            aurora_i18n::tr!("ui-perf-scene"),
            aurora_i18n::tr!("ui-perf-geometry-textures-and-objects-sent-to-the-gpu"),
        ),
        (
            aurora_i18n::tr!("ui-perf-culling-and-lists"),
            aurora_i18n::tr!("ui-perf-visibility-choices-and-draw-lists"),
        ),
        (
            aurora_i18n::tr!("ui-perf-avatars-and-camera"),
            aurora_i18n::tr!("ui-perf-animations-agent-camera-texture-streaming"),
        ),
        (aurora_i18n::tr!("common-interface"), aurora_i18n::tr!("ui-perf-windows-and-bars")),
        (
            aurora_i18n::tr!("ui-perf-gpu-encoding"),
            aurora_i18n::tr!("ui-perf-preparing-rendering-commands-with-the-render-thread-only-the"),
        ),
        (
            aurora_i18n::tr!("ui-perf-waiting"),
            aurora_i18n::tr!("ui-perf-vertical-synchronization-frame-rate-limit-presentation-render-thread-wait"),
        ),
    ]
}

/// The demo has no circuit: counters that grow like a session's traffic
/// (about 1.2 Mb/s in and 100 kb/s out, in waves), added to the network's,
/// so that the graphs of the window and the bars show a minute of traffic
/// through the same data (`PerfData::frame`).
pub fn demo_traffic(net: &mut NetStatsSnapshot) {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let t = START.get_or_init(Instant::now).elapsed().as_secs_f64();
    net.bytes_in += demo_count(150_000.0, t) as u64;
    net.bytes_out += demo_count(12_000.0, t) as u64;
    net.packets_in += demo_count(120.0, t) as u64;
    net.packets_out += demo_count(40.0, t) as u64;
}

/// What the demo's traffic counts `t` seconds in: the integral of a rate of
/// `base` × (1 + 0.35 sin 0.7t + 0.2 sin 2.3t) a second.
fn demo_count(base: f64, t: f64) -> f64 {
    base * (t + 0.5 * (1.0 - (0.7 * t).cos()) + 0.2 / 2.3 * (1.0 - (2.3 * t).cos()))
}

impl PerfData {
    /// The demo's minute of traffic before it started (`demo_traffic`
    /// goes on from there), so that its graphs are full from the start.
    pub fn demo_history(&mut self) {
        self.net_hist.clear();
        for s in -(NET_HISTORY as i32 - 1)..0 {
            let t = f64::from(s);
            let rate = |base: f64| ((demo_count(base, t + 1.0) - demo_count(base, t)) * 8.0 / 1000.0) as f32;
            self.net_hist.push_back((rate(150_000.0), rate(12_000.0)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The resource monitor keeps its size in each of its sections.
    #[test]
    fn the_monitor_keeps_its_size_in_every_section() {
        use super::super::monitor::snapshot::*;
        let gpu = GpuInfo {
            name: "Test GPU".into(),
            backend: "Vulkan".into(),
            driver: String::new(),
            bindless: true,
            timestamps: true,
            max_textures: 4096,
            vram_mb: Some(8192),
            integrated: false,
            vendor_id: 0,
            max_texture_size: 0,
            adapters: vec!["Test GPU".into()],
            notice: None,
        };
        let data = PerfData::default();
        for tab in super::super::monitor::Tab::ALL {
            crate::ui::widgets::steady_tests::assert_window_steady(
                "perf",
                &[super::super::monitor::DEFAULT_SIZE, super::super::monitor::MIN_SIZE],
                aurora_i18n::tr!("monitor-status-live"),
                || {
                    let mut m = super::super::monitor::Monitor::default();
                    m.tab = tab;
                    for fps in [58.0, 60.0, 61.0] {
                        m.history.push(Snapshot {
                            attribution: Default::default(),
                            cpu: None,
                            probes: None,
                            fps,
                            frame_ms: 1000.0 / fps,
                            parts: FrameParts::default(),
                            memory: None,
                            page_faults_per_s: None,
                            vram: Default::default(),
                            texture: TextureInfo::default(),
                            geometry: GeometryInfo::default(),
                            net: NetInfo::default(),
                            regions: Vec::new(),
                            disk: DiskInfo::default(),
                            scene: SceneInfo::default(),
                            region: None,
                        });
                    }
                    m
                },
                |m, ctx, p| {
                    show(ctx, p, &PerfView { data: &data, gpu: &gpu }, m, &mut true);
                },
            );
        }
    }

    #[test]
    fn frame_stats_over_history() {
        let frames: VecDeque<f32> = [10.0, 20.0, 30.0].into_iter().collect();
        assert_eq!(
            frame_stats(&frames),
            FrameStats {
                avg: 20.0,
                min: 10.0,
                max: 30.0
            }
        );
        assert_eq!(frame_stats(&VecDeque::new()), FrameStats::default());
    }

    #[test]
    fn cpu_parts_fill_the_frame() {
        // update 6 ms holds sync 2 + cull 1: 3 ms of other updates
        let parts = cpu_parts(16.0, 1.0, 6.0, 2.0, 1.0, 2.0, 3.0);
        assert_eq!(parts, [1.0, 2.0, 1.0, 3.0, 2.0, 3.0, 4.0]);
        assert_eq!(parts.iter().sum::<f32>(), 16.0);
    }

    #[test]
    fn smoothing_follows_time_not_frames() {
        // one 16 ms frame or two 8 ms frames move the average the same way
        let mut one = [0.0];
        smooth(&mut one, &[1.0], ease(16.0, SMOOTHING_S));
        let mut two = [0.0];
        smooth(&mut two, &[1.0], ease(8.0, SMOOTHING_S));
        smooth(&mut two, &[1.0], ease(8.0, SMOOTHING_S));
        assert!((one[0] - two[0]).abs() < 1e-6);
        // after one time constant: 63 % of the way
        assert!((ease(SMOOTHING_S * 1000.0, SMOOTHING_S) - 0.632).abs() < 1e-3);
    }

    #[test]
    fn graph_top_ignores_a_lone_fast_frame() {
        // 160 images/s, one 1 ms frame: the scale stays near 160
        let mut frames: VecDeque<f32> = std::iter::repeat_n(6.25, 239).collect();
        frames.push_back(1.0);
        assert!((graph_top(&frames) - 160.0 * 1.15).abs() < 0.5);
        // slow frames: the 60 mark stays in view
        let slow: VecDeque<f32> = std::iter::repeat_n(50.0, 240).collect();
        assert_eq!(graph_top(&slow), GRAPH_MIN_TOP);
    }

    #[test]
    fn graph_marks_follow_the_scale() {
        assert_eq!(grid_step(160.0 * 1.15), 50.0); // 50, 100, 150
        assert_eq!(grid_step(GRAPH_MIN_TOP), 20.0); // 20, 40, 60
        assert_eq!(grid_step(400.0), 100.0);
        assert_eq!(grid_step(1200.0), 500.0); // 500, 1000
        assert_eq!(grid_step(2400.0), 1000.0); // 1 Mb/s, 2 Mb/s
    }

    #[test]
    fn rates_in_the_unit_that_reads_best() {
        assert_eq!(fmt_rate(0.0), "0 kb/s");
        assert_eq!(fmt_rate(850.4), "850 kb/s");
        assert_eq!(fmt_rate(1250.0), "1.2 Mb/s");
        assert_eq!(fmt_rate(2_500_000.0), "2.50 Gb/s");
    }

    #[test]
    fn cpu_parts_never_negative() {
        // stale sync / cull larger than the update, a frame shorter than the work
        let parts = cpu_parts(5.0, 1.0, 1.0, 2.0, 1.0, 2.0, 3.0);
        assert!(parts.iter().all(|&x| x >= 0.0));
        assert_eq!(parts[3], 0.0);
        assert_eq!(parts[6], 0.0);
    }
}
