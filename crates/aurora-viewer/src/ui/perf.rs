//! Performance overlay: frame timing graph, CPU/GPU breakdown, scene,
//! streaming and network statistics.

use crate::theme::Palette;
use aurora_net::NetStatsSnapshot;
use aurora_render::{GpuInfo, RenderStats};
use egui::{Color32, RichText, Vec2};
use std::collections::VecDeque;
use std::time::Instant;

pub struct PerfData {
    pub frame_ms: VecDeque<f32>,
    pub fps: f32,
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
}

impl Default for PerfData {
    fn default() -> Self {
        PerfData {
            frame_ms: VecDeque::with_capacity(300),
            fps: 0.0,
            last_fps_update: Instant::now(),
            frames_since: 0,
            net_prev: NetStatsSnapshot::default(),
            net_rate: (0.0, 0.0, 0.0, 0.0),
            last_net: Instant::now(),
            http_rate_kbps: 0.0,
            events_ms: 0.0,
            update_ms: 0.0,
            ui_ms: 0.0,
        }
    }
}

impl PerfData {
    pub fn frame(&mut self, dt_ms: f32, net: NetStatsSnapshot) {
        if self.frame_ms.len() >= 240 {
            self.frame_ms.pop_front();
        }
        self.frame_ms.push_back(dt_ms);
        self.frames_since += 1;
        let el = self.last_fps_update.elapsed().as_secs_f32();
        if el >= 0.5 {
            self.fps = self.frames_since as f32 / el;
            self.frames_since = 0;
            self.last_fps_update = Instant::now();
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
    pub scene: &'a crate::scene::SceneStats,
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

fn row(ui: &mut egui::Ui, p: &Palette, k: &str, v: String) {
    ui.label(RichText::new(k).size(11.5).color(p.muted));
    ui.label(RichText::new(v).size(11.5).color(p.ink));
    ui.end_row();
}

fn group(ui: &mut egui::Ui, p: &Palette, title: &str, open: bool, body: impl FnOnce(&mut egui::Ui)) {
    egui::CollapsingHeader::new(RichText::new(title).size(12.0).strong().color(p.violet_light))
        .default_open(open)
        .show(ui, |ui| {
            egui::Grid::new(title).num_columns(2).spacing([12.0, 1.0]).show(ui, body);
        });
}

pub fn show(ctx: &egui::Context, p: &Palette, v: &PerfView, open: &mut bool) {
    let screen = ctx.content_rect();
    egui::Window::new(RichText::new("Performances").size(13.0).color(p.ink))
        .id(egui::Id::new("perf"))
        .fade_in(false)
        .fade_out(false)
        .open(open)
        .default_pos(egui::pos2(screen.right() - 300.0, 300.0))
        .default_width(280.0)
        .resizable(false)
        .collapsible(true)
        .show(ctx, |ui| {
            let d = v.data;
            let (mut mn, mut mx, mut sum) = (f32::MAX, 0.0f32, 0.0);
            for &f in &d.frame_ms {
                mn = mn.min(f);
                mx = mx.max(f);
                sum += f;
            }
            let avg = if d.frame_ms.is_empty() {
                0.0
            } else {
                sum / d.frame_ms.len() as f32
            };
            ui.horizontal(|ui| {
                let col = if d.fps >= 55.0 {
                    p.teal
                } else if d.fps >= 28.0 {
                    p.amber
                } else {
                    p.danger
                };
                ui.label(RichText::new(format!("{:.0}", d.fps)).size(24.0).strong().color(col));
                ui.vertical(|ui| {
                    ui.label(RichText::new("images/s").size(11.0).color(p.muted));
                    ui.label(
                        RichText::new(format!("{avg:.2} ms (min {mn:.1} · max {mx:.1})"))
                            .size(12.0)
                            .color(p.ink),
                    );
                });
            });
            // frame time graph
            let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), egui::Sense::hover());
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 2.0, p.field);
            let target = 1000.0 / 60.0;
            let scale_max = (mx.max(target * 2.0)).min(100.0);
            let n = d.frame_ms.len().max(1);
            let bw = rect.width() / 240.0;
            for (i, &f) in d.frame_ms.iter().enumerate() {
                let h = (f / scale_max).min(1.0) * rect.height();
                let x = rect.left() + (240 - n + i) as f32 * bw;
                let c = if f <= target * 1.05 {
                    p.violet
                } else if f <= target * 2.0 {
                    p.amber
                } else {
                    p.danger
                };
                painter.rect_filled(
                    egui::Rect::from_min_max(egui::pos2(x, rect.bottom() - h), egui::pos2(x + bw.max(1.0), rect.bottom())),
                    0.0,
                    c,
                );
            }
            let ty = rect.bottom() - (target / scale_max) * rect.height();
            painter.line_segment(
                [egui::pos2(rect.left(), ty), egui::pos2(rect.right(), ty)],
                egui::Stroke::new(1.0, Color32::from_white_alpha(60)),
            );

            group(ui, p, "Temps CPU (ms)", true, |ui| {
                row(ui, p, "Réseau vers monde", format!("{:.2}", d.events_ms));
                row(ui, p, "Scène (sync)", format!("{:.2}", v.scene.sync_ms));
                row(ui, p, "Culling + listes", format!("{:.2}", v.scene.cull_ms));
                row(ui, p, "Interface", format!("{:.2}", d.ui_ms));
                row(ui, p, "Encodage GPU", format!("{:.2}", v.render.cpu_encode_ms));
                row(
                    ui,
                    p,
                    "Temps GPU",
                    v.render.gpu_ms.map(|g| format!("{g:.2}")).unwrap_or_else(|| "n/d".into()),
                );
            });
            if let Some(passes) = v.render.gpu_passes {
                group(ui, p, "Temps GPU par passe (ms)", false, |ui| {
                    let names = [
                        "Ombres",
                        "Prépasse",
                        "Occlusion",
                        "Prépasse 2",
                        "SSAO, reflets, sondes",
                        "Scène",
                        "TAA + post",
                    ];
                    for (name, ms) in names.iter().zip(passes) {
                        row(ui, p, name, format!("{ms:.2}"));
                    }
                });
            }
            group(ui, p, "Rendu", true, |ui| {
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
            group(ui, p, "GPU", false, |ui| {
                row(ui, p, "Carte", v.gpu.name.clone());
                row(ui, p, "API", format!("{} (wgpu)", v.gpu.backend));
            });
        });
}
