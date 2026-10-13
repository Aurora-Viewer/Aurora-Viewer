//! Performance window. Compact by default: frame rate, frame times and
//! their graph. The full view adds where the GPU and CPU time of a frame
//! goes (segmented bars), then scene, streaming and network statistics.

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
const FULL_W: f32 = 300.0;
const GRAPH_H: f32 = 64.0;
/// Lowest top of the graph scale (images/s).
const GRAPH_MIN_TOP: f32 = 72.0;

/// Network rate samples kept for the graph (one per second).
const NET_HISTORY: usize = 60;
/// Lowest top of the network graph scale (kb/s).
const NET_MIN_TOP: f32 = 10.0;
const NET_GRAPH_H: f32 = 56.0;

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

fn fmt_ms(ms: f32) -> String {
    format!("{ms:.2} ms")
}

/// A rate in the unit that reads best: kb/s, Mb/s or Gb/s.
fn fmt_rate(kbps: f32) -> String {
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
    pub render: &'a RenderStats,
    pub gpu: &'a GpuInfo,
    pub net: &'a NetStatsSnapshot,
    pub scene: &'a SceneStats,
    pub tex: &'a crate::scene::textures::TextureStats,
    pub meshes_fetching: usize,
    pub regions: usize,
    pub draw_distance: f32,
    /// Reflection probes: (known, captured).
    pub probes: (usize, usize),
    /// World sounds playing.
    pub sounds: usize,
    /// Own avatar complexity, avatars shown as silhouettes.
    pub complexity: (Option<u32>, usize),
}

/// Full view shown; persisted with the UI layout (ui_layout.ron).
fn full_id() -> egui::Id {
    egui::Id::new(("perf", "full"))
}

pub fn set_full(ctx: &egui::Context, full: bool) {
    ctx.data_mut(|d| d.insert_persisted(full_id(), full));
}

/// Where the window stands before the user moves it: under the mini-map of
/// a first launch (`minimap::default_rect`), on its right edge.
pub(crate) fn default_pos(work: egui::Rect, width: f32) -> egui::Pos2 {
    let map = super::minimap::default_rect(work);
    pos2(map.right() - width - SIDES, map.bottom() + super::layout::GAP)
}

pub fn show(ctx: &egui::Context, p: &Palette, v: &PerfView, open: &mut bool) {
    let full = ctx.data_mut(|d| d.get_persisted::<bool>(full_id())).unwrap_or(false);
    let (icon, tip) = if full {
        ("arrows-minimize", aurora_i18n::tr!("ui-perf-compact-version"))
    } else {
        ("arrows-maximize", aurora_i18n::tr!("ui-perf-show-details"))
    };
    let width = if full { FULL_W } else { COMPACT_W };
    let work = super::layout::work_area(ctx);
    let mut toggle = false;
    Floater::new(
        "perf",
        aurora_i18n::tr!("common-performance"),
        default_pos(work, width),
        vec2(width, 0.0),
    )
    .fixed()
    .help(aurora_i18n::tr!("ui-perf-the-graph-shows-fps-over-the-last-few-seconds"))
    .action(icon, tip, &mut toggle)
    .show(ctx, p, open, |ui| {
        ui.set_width(width);
        headline(ui, p, v.data);
        ui.add_space(6.0);
        graph(ui, p, v.data);
        if full {
            details(ui, p, v);
        }
    });
    if toggle {
        set_full(ctx, !full);
    }
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

/// Network rates of the last minute: incoming (line over a light area)
/// and outgoing, marks at round rates, both values under the pointer.
fn net_graph(ui: &mut egui::Ui, p: &Palette, d: &PerfData) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), NET_GRAPH_H), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, ui.visuals().window_corner_radius, p.field);
    let top = d.net_top.max(1.0);
    let plot = rect.shrink2(vec2(0.0, 4.0));
    let y = |kbps: f32| plot.bottom() - (kbps / top).min(1.0) * plot.height();

    let mark_font = egui::FontId::proportional(9.0);
    let grid = grid_step(top);
    let mut rate = grid;
    while rate < top * 0.95 {
        let ly = y(rate).round() + 0.5;
        painter.extend(egui::Shape::dashed_line(
            &[pos2(rect.left() + 4.0, ly), pos2(rect.right() - 52.0, ly)],
            Stroke::new(1.0, p.raised),
            3.0,
            3.0,
        ));
        painter.text(
            pos2(rect.right() - 4.0, ly),
            egui::Align2::RIGHT_CENTER,
            fmt_rate(rate),
            mark_font.clone(),
            p.muted_dim,
        );
        rate += grid;
    }

    // zero line over the whole width: the history fills it from the right
    let y0 = y(0.0).round() - 0.5;
    painter.line_segment([pos2(rect.left(), y0), pos2(rect.right(), y0)], Stroke::new(1.0, p.raised));

    let n = d.net_hist.len();
    if n < 2 {
        return;
    }
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

    if let Some(pos) = resp.hover_pos() {
        let i = (((pos.x - x0) / step).round().max(0.0) as usize).min(n - 1);
        let (rin, rout) = d.net_hist[i];
        let px = x(i);
        painter.line_segment([pos2(px, rect.top()), pos2(px, rect.bottom())], Stroke::new(1.0, p.muted_dim));
        painter.circle(outs[i], 3.5, p.rose, Stroke::new(2.0, p.field));
        painter.circle(ins[i], 3.5, p.teal, Stroke::new(2.0, p.field));
        let text = aurora_i18n::tr!(
            "ui-perf-incoming-outgoing",
            value0 = fmt_rate(rin).to_string(),
            value1 = fmt_rate(rout).to_string()
        );
        let galley = painter.layout_no_wrap(text, egui::FontId::proportional(11.0), p.ink);
        let size = galley.size() + vec2(10.0, 4.0);
        let left = px + 6.0 + size.x > rect.right();
        let bx = if left { (px - 6.0 - size.x).max(rect.left()) } else { px + 6.0 };
        let r = Rect::from_min_size(pos2(bx, rect.top() + 3.0), size);
        painter.rect_filled(r, ui.visuals().window_corner_radius, p.panel);
        painter.galley(r.min + vec2(5.0, 2.0), galley, p.ink);
    }
}

/// One part of a breakdown: name, help, color, smoothed and shown time (ms).
struct Part {
    name: &'static str,
    help: &'static str,
    color: Color32,
    live: f32,
    shown: f32,
}

/// Colors of the breakdown parts, in a fixed order: neighbours stay apart
/// for colour-blind readers; the last one (neutral) is the "rest".
fn part_colors(p: &Palette) -> [Color32; 7] {
    [p.indigo_light, p.rose, p.amber, p.violet, p.teal, p.violet_pale, p.muted_dim]
}

fn gpu_parts(p: &Palette, live: &[f32; 7], shown: &[f32; 7]) -> Vec<Part> {
    let colors = part_colors(p);
    GpuElement::ALL
        .iter()
        .enumerate()
        .map(|(i, el)| {
            let (name, help) = match el {
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
            };
            Part {
                name,
                help,
                color: colors[i],
                live: live[i],
                shown: shown[i],
            }
        })
        .collect()
}

fn cpu_parts_view(p: &Palette, live: &[f32; CPU_PARTS], shown: &[f32; CPU_PARTS]) -> Vec<Part> {
    fn names() -> [(&'static str, &'static str); CPU_PARTS] {
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
    let colors = part_colors(p);
    names()
        .iter()
        .enumerate()
        .map(|(i, &(name, help))| Part {
            name,
            help,
            color: colors[i],
            live: live[i],
            shown: shown[i],
        })
        .collect()
}

/// A titled segmented bar and its legend, both in the fixed order of the
/// parts (nothing jumps around); hovering a segment or a legend row
/// highlights that part.
fn breakdown(ui: &mut egui::Ui, p: &Palette, id: &str, title: &str, parts: Vec<Part>, fmt: fn(f32) -> String) {
    let total_shown: f32 = parts.iter().map(|x| x.shown).sum();
    let total_live: f32 = parts.iter().map(|x| x.live).sum::<f32>().max(1e-3);
    let hover_id = ui.id().with(("breakdown_hover", id));
    let hovered: Option<&'static str> = ui.data(|d| d.get_temp(hover_id)).flatten();
    let mut hover_now = None;

    ui.horizontal(|ui| {
        ui.label(RichText::new(title).size(12.0).strong().color(p.ink));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(fmt(total_shown)).size(12.0).color(p.muted));
        });
    });
    ui.add_space(2.0);

    // the bar: segments laid over the whole width, then inset by 1 px on
    // their inner sides for the 2 px gaps, so a part growing from nothing
    // never shifts the others; rounded at the two ends only
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 10.0), Sense::hover());
    let painter = ui.painter_at(rect);
    // the ends are the first and last parts with some width
    let wide = |x: &Part| x.live / total_live * rect.width() >= 0.5;
    if !parts.iter().any(wide) {
        // nothing measured yet (no traffic): the empty track
        painter.rect_filled(rect, ui.visuals().window_corner_radius, p.field);
    }
    let first = parts.iter().position(wide).unwrap_or(0);
    let last = parts.iter().rposition(wide).unwrap_or(parts.len() - 1);
    let mut x = rect.left();
    for (i, part) in parts.iter().enumerate() {
        let w = part.live / total_live * rect.width();
        let slot = Rect::from_min_max(pos2(x, rect.top()), pos2(x + w, rect.bottom()));
        x += w;
        if resp.hover_pos().is_some_and(|pos| slot.x_range().contains(pos.x)) {
            hover_now = Some(part.name);
        }
        let left = if i <= first { slot.left() } else { slot.left() + 1.0 };
        let right = if i >= last { slot.right() } else { slot.right() - 1.0 };
        if right - left < 0.5 {
            continue;
        }
        let end = ui.visuals().window_corner_radius;
        let corners = egui::CornerRadius {
            nw: if i == first { end.nw } else { 0 },
            sw: if i == first { end.sw } else { 0 },
            ne: if i == last { end.ne } else { 0 },
            se: if i == last { end.se } else { 0 },
        };
        let dim = hovered.is_some_and(|h| h != part.name);
        let col = if dim { part.color.gamma_multiply(0.3) } else { part.color };
        painter.rect_filled(Rect::from_min_max(pos2(left, slot.top()), pos2(right, slot.bottom())), corners, col);
    }
    ui.add_space(4.0);

    // legend: rows touch each other (no gap without a hovered part)
    let spacing = std::mem::replace(&mut ui.spacing_mut().item_spacing.y, 0.0);
    let mut bottom = rect.bottom();
    for part in &parts {
        let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
        bottom = row.bottom();
        let resp = resp.on_hover_text(part.help);
        if resp.hovered() {
            hover_now = Some(part.name);
        }
        let on = hovered == Some(part.name);
        let painter = ui.painter();
        if on {
            painter.rect_filled(row, ui.visuals().window_corner_radius, p.raised);
        }
        let cy = row.center().y;
        painter.circle_filled(pos2(row.left() + 7.0, cy), 3.5, part.color);
        let font = egui::FontId::proportional(11.5);
        painter.text(
            pos2(row.left() + 16.0, cy),
            egui::Align2::LEFT_CENTER,
            part.name,
            font.clone(),
            if on { p.ink } else { p.muted },
        );
        let pct = if total_shown > 0.0 { part.shown / total_shown * 100.0 } else { 0.0 };
        painter.text(
            pos2(row.right() - 4.0, cy),
            egui::Align2::RIGHT_CENTER,
            format!("{pct:.0} %"),
            font.clone(),
            p.muted,
        );
        painter.text(
            pos2(row.right() - 48.0, cy),
            egui::Align2::RIGHT_CENTER,
            fmt(part.shown),
            font,
            p.ink,
        );
    }
    ui.spacing_mut().item_spacing.y = spacing;
    // between the bar and the legend, or a row edge: keep the part shown
    let block = Rect::from_min_max(rect.min, pos2(rect.right(), bottom));
    if hover_now.is_none() && ui.rect_contains_pointer(block) {
        hover_now = hovered;
    }
    ui.data_mut(|d| d.insert_temp(hover_id, hover_now));
}

/// Thin separator between the sections.
fn rule(ui: &mut egui::Ui, p: &Palette) {
    ui.add_space(8.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, p.raised);
    ui.add_space(8.0);
}

/// Name on the left, value on the right, on one tight line.
fn row(ui: &mut egui::Ui, p: &Palette, k: &str, v: String) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
    let font = egui::FontId::proportional(11.5);
    let painter = ui.painter();
    let cy = rect.center().y;
    painter.text(pos2(rect.left(), cy), egui::Align2::LEFT_CENTER, k, font.clone(), p.muted);
    painter.text(pos2(rect.right() - 4.0, cy), egui::Align2::RIGHT_CENTER, v, font, p.ink);
}

fn group(ui: &mut egui::Ui, p: &Palette, title: &str, open: bool, body: impl FnOnce(&mut egui::Ui)) {
    super::widgets::collapsing(ui, p, ("perf", title), title, open, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        body(ui);
    });
}

fn details(ui: &mut egui::Ui, p: &Palette, v: &PerfView) {
    let d = v.data;
    rule(ui, p);
    match (d.gpu, d.gpu_shown) {
        (Some(live), Some(shown)) => breakdown(
            ui,
            p,
            "gpu",
            aurora_i18n::tr!("ui-perf-gpu-rendering"),
            gpu_parts(p, &live, &shown),
            fmt_ms,
        ),
        _ => {
            row(
                ui,
                p,
                aurora_i18n::tr!("ui-perf-gpu-rendering"),
                v.render.gpu_ms.map(|g| format!("{g:.1} ms")).unwrap_or_else(|| "n/d".into()),
            );
            ui.label(
                RichText::new(aurora_i18n::tr!("ui-perf-per-item-details-unavailable-with-this-graphics-card"))
                    .size(11.0)
                    .color(p.muted_dim),
            );
        }
    }
    rule(ui, p);
    breakdown(
        ui,
        p,
        "cpu",
        aurora_i18n::tr!("ui-perf-cpu"),
        cpu_parts_view(p, &d.cpu, &d.cpu_shown),
        fmt_ms,
    );
    rule(ui, p);

    group(ui, p, aurora_i18n::tr!("common-rendering"), false, |ui| {
        row(
            ui,
            p,
            aurora_i18n::tr!("ui-perf-draw-calls"),
            aurora_i18n::tr!(
                "status-shadows",
                value0 = format!("{}", v.render.draws),
                value1 = format!("{}", v.render.shadow_draws)
            ),
        );
        if let Some(n) = v.render.occluded {
            row(ui, p, aurora_i18n::tr!("ui-perf-hidden-occlusion"), format!("{n}"));
        }
        row(
            ui,
            p,
            aurora_i18n::tr!("common-culling"),
            if v.render.gpu_cull {
                aurora_i18n::tr!("ui-perf-graphics-card-2")
            } else {
                "processeur"
            }
            .to_string(),
        );
        row(
            ui,
            p,
            aurora_i18n::tr!("ui-perf-render-thread"),
            if v.render.render_thread {
                aurora_i18n::tr!("ui-perf-per-frame", value0 = fmt_ms(d.render_thread_shown).to_string())
            } else {
                aurora_i18n::tr!("ui-perf-disabled").to_string()
            },
        );
        row(
            ui,
            p,
            aurora_i18n::tr!("ui-perf-triangles"),
            format!("{:.2} M", v.render.triangles as f64 / 1e6),
        );
        row(
            ui,
            p,
            aurora_i18n::tr!("common-objects"),
            aurora_i18n::tr!(
                "status-visible",
                value0 = format!("{}", v.scene.visible_objects),
                value1 = format!("{}", v.scene.objects)
            ),
        );
        row(
            ui,
            p,
            aurora_i18n::tr!("ui-perf-geometries"),
            aurora_i18n::tr!(
                "ui-perf-in-progress",
                value0 = format!("{}", v.scene.geometries),
                value1 = format!("{}", v.scene.geom_pending)
            ),
        );
        row(ui, p, aurora_i18n::tr!("common-distance"), format!("{:.0} m", v.draw_distance));
        row(ui, p, aurora_i18n::tr!("common-particles"), format!("{}", v.render.particles));
        if let Some(c) = v.complexity.0 {
            row(ui, p, aurora_i18n::tr!("ui-perf-your-complexity"), format!("{c}"));
        }
        if v.complexity.1 > 0 {
            row(ui, p, aurora_i18n::tr!("ui-perf-silhouette-avatars"), format!("{}", v.complexity.1));
        }
        if v.scene.avatars_hidden > 0 {
            row(
                ui,
                p,
                aurora_i18n::tr!("ui-perf-impostor-avatars"),
                format!("{}", v.scene.avatars_hidden),
            );
        }
        let refl = match (v.render.water_reflection, v.render.mirror) {
            (true, true) => aurora_i18n::tr!("status-water-mirror"),
            (true, false) => "eau",
            (false, true) => "miroir",
            _ => aurora_i18n::tr!("extra-none"),
        };
        row(ui, p, aurora_i18n::tr!("ui-perf-planar-reflections"), refl.to_owned());
        row(
            ui,
            p,
            aurora_i18n::tr!("common-reflection-probes"),
            aurora_i18n::tr!(
                "ui-perf-ready",
                value0 = format!("{}", v.probes.1),
                value1 = format!("{}", v.probes.0 + 1)
            ),
        );
        row(ui, p, aurora_i18n::tr!("ui-perf-playing-sounds"), format!("{}", v.sounds));
    });
    group(ui, p, aurora_i18n::tr!("ui-perf-memory-and-streaming"), false, |ui| {
        row(
            ui,
            p,
            aurora_i18n::tr!("ui-perf-geometry"),
            aurora_i18n::tr!(
                "memory-mb",
                value = aurora_i18n::number(v.render.geometry_bytes as f64 / 1048576.0, 0)
            ),
        );
        row(
            ui,
            p,
            aurora_i18n::tr!("common-textures"),
            aurora_i18n::tr!(
                "perf-texture-memory",
                loaded = v.tex.loaded.to_string(),
                total = v.tex.total.to_string(),
                value = aurora_i18n::number(v.render.texture_bytes as f64 / 1048576.0, 0)
            ),
        );
        row(
            ui,
            p,
            aurora_i18n::tr!("ui-perf-texture-pages"),
            aurora_i18n::tr!(
                "perf-page-memory",
                pages = v.render.texture_pages.to_string(),
                value = aurora_i18n::number(v.render.texture_page_bytes as f64 / 1048576.0, 0)
            ),
        );
        // what the pages may hold now, and the pressure step when they
        // hold more (scene/texture_budget.rs)
        let target = aurora_i18n::number(v.tex.target_bytes as f64 / 1048576.0, 0);
        row(
            ui,
            p,
            aurora_i18n::tr!("ui-perf-texture-budget"),
            if v.tex.pressure > 0 {
                aurora_i18n::tr!(
                    "perf-texture-budget-pressure",
                    value = target,
                    step = v.tex.pressure.to_string(),
                    steps = crate::scene::texture_budget::MAX_PRESSURE.to_string()
                )
            } else {
                aurora_i18n::tr!("memory-mb", value = target)
            },
        );
        row(ui, p, aurora_i18n::tr!("ui-perf-texture-network"), format!("{}", v.tex.fetching));
        row(
            ui,
            p,
            aurora_i18n::tr!("ui-perf-decoding"),
            aurora_i18n::tr!(
                "status-upload",
                value0 = format!("{}", v.tex.decoding),
                value1 = format!("{}", v.tex.pending_upload)
            ),
        );
        row(ui, p, aurora_i18n::tr!("ui-perf-background-tasks"), format!("{}", v.scene.jobs));
        row(ui, p, aurora_i18n::tr!("ui-perf-network-meshes"), format!("{}", v.meshes_fetching));
        row(ui, p, aurora_i18n::tr!("ui-perf-gpu-records"), format!("{}", v.render.records));
    });
    group(ui, p, aurora_i18n::tr!("ui-perf-network"), false, |ui| {
        let last = d.net_hist.back().copied().unwrap_or_default();
        let parts = vec![
            Part {
                name: aurora_i18n::tr!("ui-perf-incoming"),
                help: aurora_i18n::tr!("ui-perf-received-from-the-simulator-udp-and-downloaded-http-textures"),
                color: p.teal,
                live: d.net_live[0],
                shown: last.0,
            },
            Part {
                name: aurora_i18n::tr!("ui-perf-outgoing"),
                help: aurora_i18n::tr!("ui-perf-sent-to-simulator-udp"),
                color: p.rose,
                live: d.net_live[1],
                shown: last.1,
            },
        ];
        breakdown(ui, p, "net", aurora_i18n::tr!("ui-perf-throughput"), parts, fmt_rate);
        ui.add_space(6.0);
        net_graph(ui, p, d);
        ui.add_space(6.0);
        row(ui, p, aurora_i18n::tr!("ui-perf-ping"), format!("{} ms", v.net.ping_ms));
        row(
            ui,
            p,
            aurora_i18n::tr!("ui-perf-packets"),
            aurora_i18n::tr!(
                "status-in-s-out-s",
                value0 = aurora_i18n::number_text(&format!("{:.0}", d.net_rate.0)),
                value1 = aurora_i18n::number_text(&format!("{:.0}", d.net_rate.1))
            ),
        );
        row(
            ui,
            p,
            aurora_i18n::tr!("ui-perf-lost-and-found"),
            format!("{} / {}", v.net.resent, v.net.dropped_reliable),
        );
        row(ui, p, aurora_i18n::tr!("ui-perf-unacknowledged"), format!("{}", v.net.unacked));
        row(
            ui,
            p,
            "HTTP",
            aurora_i18n::tr!(
                "ui-perf-active-queued",
                value0 = format!("{}", v.net.http_in_flight),
                value1 = format!("{}", v.net.http_queued)
            ),
        );
        row(ui, p, aurora_i18n::tr!("ui-perf-http-throughput"), fmt_rate(d.http_rate_kbps));
        row(
            ui,
            p,
            aurora_i18n::tr!("ui-perf-downloaded"),
            aurora_i18n::tr!(
                "ui-perf-mb-failures",
                value0 = aurora_i18n::number_text(&format!("{:.1}", v.net.http_bytes as f64 / 1048576.0)),
                value1 = format!("{}", v.net.http_failed)
            ),
        );
        row(ui, p, aurora_i18n::tr!("ui-perf-regions"), format!("{}", v.regions));
    });
    group(ui, p, aurora_i18n::tr!("ui-perf-graphics-card"), false, |ui| {
        row(ui, p, aurora_i18n::tr!("common-map"), v.gpu.name.clone());
        row(ui, p, "API", format!("{} (wgpu)", v.gpu.backend));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

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
