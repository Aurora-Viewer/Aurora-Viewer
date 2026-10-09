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
/// Smoothing of the breakdown bars, per frame (exponential moving average).
const SMOOTHING: f32 = 0.08;
/// How often the numbers are refreshed (s): slow enough to be read.
const REFRESH_S: f32 = 0.5;
const COMPACT_W: f32 = 236.0;
const FULL_W: f32 = 300.0;
const GRAPH_H: f32 = 58.0;

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

/// CPU parts of a frame (ms): network → world, scene sync, culling and
/// lists, the rest of the update, interface, GPU encoding, and the time
/// left (vsync, frame limiter, present). Sync and culling run inside the
/// update.
fn cpu_parts(frame: f32, events: f32, update: f32, sync: f32, cull: f32, ui: f32, encode: f32) -> [f32; CPU_PARTS] {
    let other = (update - sync - cull).max(0.0);
    let busy = events + sync + cull + other + ui + encode;
    [events, sync, cull, other, ui, encode, (frame - busy).max(0.0)]
}

fn smooth<const N: usize>(avg: &mut [f32; N], now: &[f32; N]) {
    for (a, n) in avg.iter_mut().zip(now) {
        *a += (n - *a) * SMOOTHING;
    }
}

pub struct PerfData {
    pub frame_ms: VecDeque<f32>,
    pub fps: f32,
    /// Frame times, refreshed with `fps`.
    pub frames: FrameStats,
    last_fps_update: Instant,
    frames_since: u32,
    pub net_prev: NetStatsSnapshot,
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
    /// Top of the graph scale (ms): follows the slowest frame, eases down.
    graph_top: f32,
}

impl Default for PerfData {
    fn default() -> Self {
        PerfData {
            frame_ms: VecDeque::with_capacity(HISTORY),
            fps: 0.0,
            frames: FrameStats::default(),
            last_fps_update: Instant::now(),
            frames_since: 0,
            net_prev: NetStatsSnapshot::default(),
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
            graph_top: 20.0,
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
        smooth(&mut self.cpu, &cpu);
        self.gpu = match (render.gpu_elements, self.gpu) {
            (Some(now), Some(mut avg)) => {
                smooth(&mut avg, &now);
                Some(avg)
            }
            (now, _) => now,
        };
        let slowest = self.frame_ms.iter().copied().fold(0.0f32, f32::max);
        let target = (slowest * 1.25).max(20.0);
        self.graph_top = if target > self.graph_top {
            target
        } else {
            self.graph_top + (target - self.graph_top) * 0.03
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
        }
        let nel = self.last_net.elapsed().as_secs_f32();
        if nel >= 1.0 {
            let p = &self.net_prev;
            self.net_rate = (
                (net.packets_in - p.packets_in) as f32 / nel,
                (net.packets_out - p.packets_out) as f32 / nel,
                (net.bytes_in - p.bytes_in) as f32 * 8.0 / 1000.0 / nel,
                (net.bytes_out - p.bytes_out) as f32 * 8.0 / 1000.0 / nel,
            );
            self.http_rate_kbps = (net.http_bytes - p.http_bytes) as f32 * 8.0 / 1000.0 / nel;
            self.net_prev = net;
            self.last_net = Instant::now();
        }
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

pub fn show(ctx: &egui::Context, p: &Palette, v: &PerfView, open: &mut bool) {
    let full = ctx.data_mut(|d| d.get_persisted::<bool>(full_id())).unwrap_or(false);
    let (icon, tip) = if full {
        ("corners-in", "Version compacte")
    } else {
        ("corners-out", "Afficher le détail")
    };
    let width = if full { FULL_W } else { COMPACT_W };
    let screen = ctx.content_rect();
    let mut toggle = false;
    Floater::new(
        "perf",
        "Performances",
        pos2(screen.right() - FULL_W - 16.0, 300.0),
        vec2(width, 0.0),
    )
    .fixed()
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
            RichText::new(format!("{:.0}", d.fps))
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
                RichText::new(format!("min {:.1} · max {:.1}", d.frames.min, d.frames.max))
                    .size(11.0)
                    .color(p.muted),
            );
        });
    });
}

/// Frame times of the last seconds: a line over a light area, the 60 and
/// 30 images/s marks, and the time of the frame under the pointer.
fn graph(ui: &mut egui::Ui, p: &Palette, d: &PerfData) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), GRAPH_H), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3.0, p.field);
    let top = d.graph_top.max(1.0);
    let plot = rect.shrink2(vec2(0.0, 4.0));
    let y = |ms: f32| plot.bottom() - (ms / top).min(1.0) * plot.height();

    let mark_font = egui::FontId::proportional(9.0);
    for (ms, label) in [(1000.0 / 60.0, "60"), (1000.0 / 30.0, "30")] {
        if ms >= top {
            continue;
        }
        let ly = y(ms).round() + 0.5;
        painter.extend(egui::Shape::dashed_line(
            &[pos2(rect.left() + 4.0, ly), pos2(rect.right() - 18.0, ly)],
            Stroke::new(1.0, p.raised),
            3.0,
            3.0,
        ));
        painter.text(
            pos2(rect.right() - 4.0, ly),
            egui::Align2::RIGHT_CENTER,
            label,
            mark_font.clone(),
            p.muted_dim,
        );
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
        .map(|(i, &ms)| pos2(x0 + i as f32 * step, y(ms)))
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
        let text = format!("{ms:.1} ms · {:.0} i/s", 1000.0 / ms.max(0.1));
        let galley = painter.layout_no_wrap(text, egui::FontId::proportional(11.0), p.ink);
        let size = galley.size() + vec2(10.0, 4.0);
        // beside the line, on the side with room
        let left = pt.x + 6.0 + size.x > rect.right();
        let x = if left { pt.x - 6.0 - size.x } else { pt.x + 6.0 };
        let r = Rect::from_min_size(pos2(x, rect.top() + 3.0), size);
        painter.rect_filled(r, 3.0, p.panel);
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
/// for colour-blind readers; the last one (neutral) is the « rest ».
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
                GpuElement::ShadowsDepth => ("Ombres et profondeur", "Ombres du soleil, prépasse de profondeur, occlusion"),
                GpuElement::Effects => ("Effets", "Occlusion ambiante, reflets (eau, miroirs, sondes), reflets écran"),
                GpuElement::TerrainSky => ("Terrain et ciel", "Sol des régions et ciel"),
                GpuElement::Objects => ("Objets et avatars", "Prims, meshes et avatars opaques ou masqués, imposteurs"),
                GpuElement::Water => ("Eau", "Surface de l'eau (réfraction, vagues)"),
                GpuElement::Transparent => ("Transparents et particules", "Faces semi-transparentes et particules"),
                GpuElement::Post => ("Post-traitement", "Lueur, anticrénelage (TAA, SMAA), tonalité"),
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
    const NAMES: [(&str, &str); CPU_PARTS] = [
        ("Réseau vers monde", "Messages du simulateur appliqués au monde"),
        ("Scène", "Géométrie, textures et objets envoyés au GPU"),
        ("Culling et listes", "Choix de ce qui est visible, listes de dessin"),
        ("Avatars et caméra", "Animations, agent, caméra, flux des textures"),
        ("Interface", "Fenêtres et barres"),
        ("Encodage GPU", "Préparation des commandes de rendu"),
        ("Attente", "Synchronisation verticale, limite d'images/s, présentation"),
    ];
    let colors = part_colors(p);
    NAMES
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

/// A titled segmented bar (largest part first) and its legend; hovering a
/// segment or a legend row highlights that part.
fn breakdown(ui: &mut egui::Ui, p: &Palette, id: &str, title: &str, mut parts: Vec<Part>) {
    let total_shown: f32 = parts.iter().map(|x| x.shown).sum();
    let total_live: f32 = parts.iter().map(|x| x.live).sum::<f32>().max(1e-3);
    parts.sort_by(|a, b| b.shown.total_cmp(&a.shown));
    let hover_id = ui.id().with(("breakdown_hover", id));
    // by name: the order can change between frames
    let hovered: Option<&'static str> = ui.data(|d| d.get_temp(hover_id)).flatten();
    let mut hover_now = None;

    ui.horizontal(|ui| {
        ui.label(RichText::new(title).size(12.0).strong().color(p.ink));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(format!("{total_shown:.1} ms")).size(12.0).color(p.muted));
        });
    });
    ui.add_space(2.0);

    // the bar: 2 px gaps between segments
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 10.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3.0, p.field);
    let gap = 2.0;
    let visible: Vec<usize> = (0..parts.len())
        .filter(|&i| parts[i].live / total_live * rect.width() >= 1.0)
        .collect();
    let usable = rect.width() - gap * visible.len().saturating_sub(1) as f32;
    let mut x = rect.left();
    for &i in &visible {
        let w = parts[i].live / total_live * usable;
        let seg = Rect::from_min_max(pos2(x, rect.top()), pos2(x + w, rect.bottom()));
        if resp
            .hover_pos()
            .is_some_and(|pos| pos.x >= seg.left() - gap / 2.0 && pos.x < seg.right() + gap / 2.0)
        {
            hover_now = Some(parts[i].name);
        }
        let dim = hovered.is_some_and(|h| h != parts[i].name);
        let col = if dim { parts[i].color.gamma_multiply(0.3) } else { parts[i].color };
        painter.rect_filled(seg, 2.0, col);
        x += w + gap;
    }
    ui.add_space(4.0);

    // legend
    for part in &parts {
        let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 17.0), Sense::hover());
        let resp = resp.on_hover_text(part.help);
        if resp.hovered() {
            hover_now = Some(part.name);
        }
        let on = hovered == Some(part.name);
        let painter = ui.painter();
        if on {
            painter.rect_filled(row, 2.0, p.raised);
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
            format!("{:.2} ms", part.shown),
            font,
            p.ink,
        );
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
    egui::CollapsingHeader::new(RichText::new(title).size(12.0).strong().color(p.violet_light))
        .id_salt(("perf", title))
        .default_open(open)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            body(ui);
        });
}

fn details(ui: &mut egui::Ui, p: &Palette, v: &PerfView) {
    let d = v.data;
    rule(ui, p);
    match (d.gpu, d.gpu_shown) {
        (Some(live), Some(shown)) => breakdown(ui, p, "gpu", "Rendu GPU", gpu_parts(p, &live, &shown)),
        _ => {
            row(
                ui,
                p,
                "Rendu GPU",
                v.render.gpu_ms.map(|g| format!("{g:.1} ms")).unwrap_or_else(|| "n/d".into()),
            );
            ui.label(
                RichText::new("Détail par élément indisponible avec cette carte graphique")
                    .size(11.0)
                    .color(p.muted_dim),
            );
        }
    }
    rule(ui, p);
    breakdown(ui, p, "cpu", "Processeur", cpu_parts_view(p, &d.cpu, &d.cpu_shown));
    rule(ui, p);

    group(ui, p, "Rendu", false, |ui| {
        row(
            ui,
            p,
            "Draw calls",
            format!("{} (+{} ombres)", v.render.draws, v.render.shadow_draws),
        );
        if let Some(n) = v.render.occluded {
            row(ui, p, "Cachés (occlusion)", format!("{n}"));
        }
        row(ui, p, "Triangles", format!("{:.2} M", v.render.triangles as f64 / 1e6));
        row(
            ui,
            p,
            "Objets",
            format!("{} visibles / {}", v.scene.visible_objects, v.scene.objects),
        );
        row(
            ui,
            p,
            "Géométries",
            format!("{} ({} en cours)", v.scene.geometries, v.scene.geom_pending),
        );
        row(ui, p, "Distance", format!("{:.0} m", v.draw_distance));
        row(ui, p, "Particules", format!("{}", v.render.particles));
        if let Some(c) = v.complexity.0 {
            row(ui, p, "Votre complexité", format!("{c}"));
        }
        if v.complexity.1 > 0 {
            row(ui, p, "Avatars en silhouette", format!("{}", v.complexity.1));
        }
        if v.scene.avatars_hidden > 0 {
            row(ui, p, "Avatars en imposteur", format!("{}", v.scene.avatars_hidden));
        }
        let refl = match (v.render.water_reflection, v.render.mirror) {
            (true, true) => "eau + miroir",
            (true, false) => "eau",
            (false, true) => "miroir",
            _ => "aucun",
        };
        row(ui, p, "Reflets plans", refl.to_owned());
        row(ui, p, "Sondes de réflexion", format!("{} prêtes / {}", v.probes.1, v.probes.0 + 1));
        row(ui, p, "Sons en cours", format!("{}", v.sounds));
    });
    group(ui, p, "Mémoire et flux", false, |ui| {
        row(ui, p, "Géométrie", format!("{:.0} Mo", v.render.geometry_bytes as f64 / 1048576.0));
        row(
            ui,
            p,
            "Textures",
            format!(
                "{} / {} · {:.0} Mo",
                v.tex.loaded,
                v.tex.total,
                v.render.texture_bytes as f64 / 1048576.0
            ),
        );
        row(ui, p, "Réseau textures", format!("{}", v.tex.fetching));
        row(ui, p, "Décodage", format!("{} · upload {}", v.tex.decoding, v.tex.pending_upload));
        row(ui, p, "Tâches en fond", format!("{}", v.scene.jobs));
        row(ui, p, "Meshes réseau", format!("{}", v.meshes_fetching));
        row(ui, p, "Records GPU", format!("{}", v.render.records));
    });
    group(ui, p, "Réseau", false, |ui| {
        row(ui, p, "Ping", format!("{} ms", v.net.ping_ms));
        row(ui, p, "Paquets", format!("in {:.0}/s · out {:.0}/s", d.net_rate.0, d.net_rate.1));
        row(ui, p, "Débit UDP", format!("in {:.0} · out {:.0} kb/s", d.net_rate.2, d.net_rate.3));
        row(ui, p, "Renvoyés / perdus", format!("{} / {}", v.net.resent, v.net.dropped_reliable));
        row(ui, p, "Non acquittés", format!("{}", v.net.unacked));
        row(
            ui,
            p,
            "HTTP",
            format!("{} actifs · {} en file", v.net.http_in_flight, v.net.http_queued),
        );
        row(ui, p, "Débit HTTP", format!("{:.0} kb/s", d.http_rate_kbps));
        row(
            ui,
            p,
            "Téléchargé",
            format!("{:.1} Mo ({} échecs)", v.net.http_bytes as f64 / 1048576.0, v.net.http_failed),
        );
        row(ui, p, "Régions", format!("{}", v.regions));
    });
    group(ui, p, "Carte graphique", false, |ui| {
        row(ui, p, "Carte", v.gpu.name.clone());
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
    fn cpu_parts_never_negative() {
        // stale sync / cull larger than the update, a frame shorter than the work
        let parts = cpu_parts(5.0, 1.0, 1.0, 2.0, 1.0, 2.0, 3.0);
        assert!(parts.iter().all(|&x| x >= 0.0));
        assert_eq!(parts[3], 0.0);
        assert_eq!(parts[6], 0.0);
    }
}
