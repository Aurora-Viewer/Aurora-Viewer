//! Scene: what costs the most around the camera, the avatars (complexity,
//! triangles, attachments, the video memory of their textures, how they are
//! drawn) or the objects (triangles, faces, texture memory, what makes them
//! dearer), each sortable, each with Inspect.

use super::draw::{self, Col, ROW_H};
use super::snapshot::{AvatarMode, AvatarRow, Feature, ObjectRow, Snapshot};
use super::{Action, Monitor, fmt};
use crate::theme::Palette;
use crate::ui::widgets;
use aurora_i18n::tr;
use egui::{Color32, Sense, pos2, vec2};

/// A figure of the table: its title, how it reads, its value of a row;
/// distance sorts nearest first, the rest largest first.
struct Figure<T> {
    title: &'static str,
    nearest_first: bool,
    value: fn(&T) -> f64,
    text: fn(&T) -> String,
}

fn avatar_figures() -> Vec<Figure<AvatarRow>> {
    vec![
        Figure {
            title: tr!("common-distance"),
            nearest_first: true,
            value: |a| f64::from(a.distance),
            text: |a| format!("{} m", aurora_i18n::number(f64::from(a.distance), 0)),
        },
        Figure {
            title: tr!("monitor-col-complexity"),
            nearest_first: false,
            value: |a| a.complexity.map_or(0.0, f64::from),
            text: |a| a.complexity.map_or("–".into(), |c| aurora_i18n::integer(i64::from(c))),
        },
        Figure {
            title: tr!("ui-perf-triangles"),
            nearest_first: false,
            value: |a| a.triangles as f64,
            text: |a| tr!("monitor-thousands", value = aurora_i18n::number(a.triangles as f64 / 1000.0, 0)),
        },
        Figure {
            title: tr!("monitor-col-attachments"),
            nearest_first: false,
            value: |a| a.attachments as f64,
            text: |a| fmt::count(a.attachments),
        },
        Figure {
            title: tr!("monitor-col-texture-vram"),
            nearest_first: false,
            value: |a| a.vram as f64,
            text: |a| if a.vram > 0 { fmt::bytes(a.vram as f64) } else { "–".into() },
        },
    ]
}

fn object_figures() -> Vec<Figure<ObjectRow>> {
    vec![
        Figure {
            title: tr!("common-distance"),
            nearest_first: true,
            value: |o| f64::from(o.distance),
            text: |o| format!("{} m", aurora_i18n::number(f64::from(o.distance), 0)),
        },
        Figure {
            title: tr!("ui-perf-triangles"),
            nearest_first: false,
            value: |o| o.triangles as f64,
            text: |o| tr!("monitor-thousands", value = aurora_i18n::number(o.triangles as f64 / 1000.0, 1)),
        },
        Figure {
            title: tr!("monitor-col-faces"),
            nearest_first: false,
            value: |o| o.faces as f64,
            text: |o| fmt::count(o.faces),
        },
        Figure {
            title: "TMem",
            nearest_first: false,
            value: |o| o.tmem as f64,
            text: |o| if o.tmem > 0 { fmt::bytes(o.tmem as f64) } else { "–".into() },
        },
        Figure {
            title: tr!("monitor-col-vram"),
            nearest_first: false,
            value: |o| o.vram as f64,
            text: |o| if o.vram > 0 { fmt::bytes(o.vram as f64) } else { "–".into() },
        },
    ]
}

fn mode(p: &Palette, m: AvatarMode) -> (Color32, &'static str) {
    match m {
        AvatarMode::Full => (p.teal, tr!("monitor-mode-full")),
        AvatarMode::Impostor => (p.violet_light, tr!("monitor-mode-impostor")),
        AvatarMode::Silhouette => (p.muted_dim, tr!("monitor-mode-silhouette")),
        AvatarMode::Loading => (p.amber, tr!("monitor-mode-loading")),
        AvatarMode::Hidden => (p.raised, tr!("monitor-mode-hidden")),
    }
}

fn features(f: &[Feature]) -> String {
    let names: Vec<&str> = f
        .iter()
        .map(|f| match f {
            Feature::Particles => tr!("common-particles"),
            Feature::Light => tr!("monitor-feature-light"),
            Feature::Media => tr!("monitor-feature-media"),
            Feature::Sound => tr!("monitor-feature-sound"),
            Feature::Animation => tr!("monitor-feature-animation"),
            Feature::Transparent => tr!("monitor-feature-transparent"),
        })
        .collect();
    if names.is_empty() { "–".into() } else { names.join(" · ") }
}

pub(super) fn show(ui: &mut egui::Ui, p: &Palette, m: &mut Monitor, s: &Snapshot, out: &mut Vec<Action>) {
    super::heading(ui, p, tr!("monitor-tab-scene"), tr!("monitor-scene-sub"), |ui| {
        let mut sel = usize::from(m.scene_objects);
        if widgets::segmented(ui, p, &mut sel, &[(tr!("monitor-avatars"), true), (tr!("common-objects"), true)]) {
            m.scene_objects = sel == 1;
            m.scene_sort = 1;
        }
    });
    let sc = &s.scene;
    let chips: Vec<(String, &str)> = if m.scene_objects {
        vec![
            (fmt::count(sc.object_count), tr!("monitor-chip-objects")),
            (fmt::count(sc.visible), tr!("monitor-chip-visible")),
            (
                tr!("monitor-millions", value = aurora_i18n::number(sc.triangles as f64 / 1e6, 1)),
                tr!("monitor-chip-triangles"),
            ),
            (fmt::count(sc.draws), tr!("monitor-chip-draws")),
            (fmt::count(sc.particles), tr!("monitor-chip-particles")),
            (fmt::count(sc.lights), tr!("monitor-chip-lights")),
            (
                sc.occluded.map_or("–".into(), |n| fmt::count(n as usize)),
                tr!("monitor-chip-occluded"),
            ),
            (
                tr!("monitor-x-of-y", x = fmt::count(sc.probes.1), y = fmt::count(sc.probes.0)),
                tr!("monitor-chip-probes"),
            ),
            (fmt::count(sc.sounds), tr!("monitor-chip-sounds")),
            (
                tr!(
                    "monitor-chip-reflections",
                    what = match sc.reflections {
                        (true, true) => tr!("monitor-water-and-mirror"),
                        (true, false) => tr!("common-water"),
                        (false, true) => tr!("monitor-mirror"),
                        _ => tr!("extra-none"),
                    }
                ),
                "",
            ),
            (
                if sc.gpu_cull {
                    tr!("monitor-culling-gpu")
                } else {
                    tr!("monitor-culling-cpu")
                }
                .to_owned(),
                "",
            ),
        ]
    } else {
        vec![
            (fmt::count(sc.avatar_count), tr!("monitor-chip-avatars")),
            (fmt::count(sc.full), tr!("monitor-chip-full")),
            (fmt::count(sc.impostors), tr!("monitor-chip-impostors")),
            (fmt::count(sc.silhouettes), tr!("monitor-chip-silhouettes")),
            (
                tr!("monitor-millions", value = aurora_i18n::number(sc.rigged_triangles as f64 / 1e6, 1)),
                tr!("monitor-chip-rigged"),
            ),
            (fmt::count(sc.attachments), tr!("monitor-chip-attachments")),
            (
                sc.own_complexity.map_or("–".into(), |c| aurora_i18n::integer(i64::from(c))),
                tr!("monitor-chip-own-complexity"),
            ),
        ]
    };
    // (two rows at most: egui does not wrap framed items by itself)
    for row in chips.chunks(chips.len().div_ceil(2).max(1)) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            for (v, k) in row {
                egui::Frame::new()
                    .fill(p.bar)
                    .stroke(egui::Stroke::new(1.0, p.raised))
                    .corner_radius(ui.visuals().window_corner_radius)
                    .inner_margin(egui::Margin::symmetric(10, 5))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            ui.label(egui::RichText::new(v).size(14.0).strong().color(p.ink));
                            if !k.is_empty() {
                                draw::text(ui, *k, draw::BODY, p.muted);
                            }
                        });
                    });
            }
        });
    }
    if m.scene_objects {
        table(ui, p, m, &sc.objects, &object_figures(), out, true);
    } else {
        table(ui, p, m, &sc.avatars, &avatar_figures(), out, false);
    }
}

trait SceneRow {
    fn name(&self) -> String;
    fn last(&self, p: &Palette) -> (Option<Color32>, String);
    fn action(&self) -> Action;
}

impl SceneRow for AvatarRow {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn last(&self, p: &Palette) -> (Option<Color32>, String) {
        let (c, t) = mode(p, self.mode);
        (Some(c), t.to_owned())
    }

    fn action(&self) -> Action {
        Action::InspectAvatar(self.id)
    }
}

impl SceneRow for ObjectRow {
    fn name(&self) -> String {
        match &self.name {
            Some(n) if self.prims > 1 => tr!("monitor-object-prims", name = n.clone(), prims = self.prims as i64),
            Some(n) => n.clone(),
            None => tr!("monitor-object-unnamed", prims = self.prims as i64),
        }
    }

    fn last(&self, _p: &Palette) -> (Option<Color32>, String) {
        (None, features(&self.features))
    }

    fn action(&self) -> Action {
        Action::Inspect(self.id)
    }
}

fn table<T: SceneRow>(
    ui: &mut egui::Ui,
    p: &Palette,
    m: &mut Monitor,
    rows: &[T],
    figures: &[Figure<T>],
    out: &mut Vec<Action>,
    objects: bool,
) {
    let sort = m.scene_sort.min(figures.len() - 1);
    let mut cols = vec![Col::new(
        if objects {
            tr!("monitor-col-object")
        } else {
            tr!("monitor-col-avatar")
        },
        0.0,
        false,
    )];
    for (i, f) in figures.iter().enumerate() {
        cols.push(Col::new(f.title, 100.0, true).sort(i == sort));
    }
    cols.push(Col::new(
        if objects {
            tr!("monitor-col-features")
        } else {
            tr!("monitor-col-drawn")
        },
        if objects { 190.0 } else { 120.0 },
        false,
    ));
    cols.push(Col::new("", 40.0, true));
    let mut order: Vec<&T> = rows.iter().collect();
    let f = &figures[sort];
    if f.nearest_first {
        order.sort_by(|a, b| (f.value)(a).total_cmp(&(f.value)(b)));
    } else {
        order.sort_by(|a, b| (f.value)(b).total_cmp(&(f.value)(a)));
    }
    let max: Vec<f64> = figures
        .iter()
        .map(|f| rows.iter().map(|r| (f.value)(r)).fold(0.0, f64::max))
        .collect();
    let frame = egui::Frame::new()
        .fill(p.bar)
        .stroke(egui::Stroke::new(1.0, p.raised))
        .corner_radius(ui.visuals().window_corner_radius);
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.set_height(ui.available_height());
        ui.spacing_mut().item_spacing.y = 0.0;
        if let Some(i) = draw::header(ui, p, &cols)
            && (1..=figures.len()).contains(&i)
        {
            m.scene_sort = i - 1;
        }
        if rows.is_empty() {
            ui.add_space(10.0);
            draw::text(ui, tr!("monitor-scene-empty"), draw::BODY, p.muted_dim);
            return;
        }
        egui::ScrollArea::vertical()
            .id_salt(("monitor-scene", objects))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for row in order {
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::hover());
                    draw::row_surface(ui, p, rect, false, resp.hovered());
                    let r = draw::col_ranges(&cols, rect.left(), rect.width());
                    let cy = rect.center().y;
                    draw::cell_text(ui, r[0], cy, &row.name(), false, p.ink, draw::BODY);
                    for (i, f) in figures.iter().enumerate() {
                        let (l, rr) = r[i + 1];
                        let v = (f.value)(row);
                        if !f.nearest_first && v > 0.0 && max[i] > 0.0 {
                            let cell = egui::Rect::from_min_max(pos2(l, rect.top()), pos2(rr, rect.bottom() - 1.0));
                            ui.painter().rect_filled(cell, 0.0, draw::heat(p, v / max[i]));
                        }
                        draw::cell_text(ui, (l, rr), cy, &(f.text)(row), true, p.ink, draw::BODY);
                    }
                    let last = r[figures.len() + 1];
                    let (dot, text) = row.last(p);
                    match dot {
                        Some(c) => {
                            ui.painter().circle_filled(pos2(last.0 + 14.0, cy), 3.5, c);
                            draw::cell_text(ui, (last.0 + 14.0, last.1), cy, &text, false, p.ink, draw::BODY);
                        }
                        None => draw::cell_text(ui, last, cy, &text, false, p.muted, draw::BODY),
                    }
                    let btn = r[figures.len() + 2];
                    let b = egui::Rect::from_center_size(pos2((btn.0 + btn.1) / 2.0, cy), vec2(24.0, 24.0));
                    let br = ui.interact(b, ui.scope_id().with(("monitor-inspect", row.name(), cy as i32)), Sense::click());
                    if br.hovered() {
                        ui.painter().rect_filled(b, widgets::inner_radius(ui.visuals()), p.raised);
                    }
                    widgets::paint_glyph(
                        ui.painter(),
                        "focus-2",
                        b.center(),
                        14.0,
                        if br.hovered() { p.ink } else { p.muted },
                    );
                    if br.on_hover_text(tr!("monitor-inspect", name = row.name())).clicked() {
                        out.push(row.action());
                    }
                }
            });
    });
}
