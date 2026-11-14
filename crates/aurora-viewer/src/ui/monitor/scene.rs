//! Scene: what costs the most around the camera, the avatars (complexity,
//! triangles, attachments, the video memory of their textures, how they are
//! drawn) or the objects (triangles, faces, texture memory, what makes them
//! dearer), each sortable, each with Inspect; or what the agent wears, its
//! worn objects and its HUDs, with Detach (the Legacy Viewer's Performance
//! window: panel_performance_complexity, panel_performance_huds,
//! FSFloaterPerformance::populateObjectList / populateHUDList).

use super::draw::{self, Col, ROW_H};
use super::snapshot::{AvatarMode, AvatarRow, Feature, ObjectRow, Snapshot, WornRow};
use super::{Action, Monitor, SceneView, fmt};
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

/// The figures of a worn object; a HUD has no complexity (it does not
/// count). The Legacy Viewer measures each one's drawing time on the
/// graphics card instead (LLPipeline::profileAvatar): Aurora has no such
/// per-object timing, its triangles and texture memory say the same.
fn worn_figures(huds: bool) -> Vec<Figure<WornRow>> {
    let mut figures: Vec<Figure<WornRow>> = Vec::new();
    if !huds {
        figures.push(Figure {
            title: tr!("monitor-col-complexity"),
            nearest_first: false,
            value: |w: &WornRow| w.complexity.map_or(0.0, f64::from),
            text: |w: &WornRow| w.complexity.map_or("–".into(), |c| aurora_i18n::integer(i64::from(c))),
        });
    }
    figures.extend([
        Figure {
            title: tr!("ui-perf-triangles"),
            nearest_first: false,
            value: |w: &WornRow| w.triangles as f64,
            text: |w: &WornRow| tr!("monitor-thousands", value = aurora_i18n::number(w.triangles as f64 / 1000.0, 1)),
        },
        Figure {
            title: tr!("monitor-col-prims"),
            nearest_first: false,
            value: |w: &WornRow| w.prims as f64,
            text: |w: &WornRow| fmt::count(w.prims),
        },
        Figure {
            title: tr!("monitor-col-texture-vram"),
            nearest_first: false,
            value: |w: &WornRow| w.vram as f64,
            text: |w: &WornRow| if w.vram > 0 { fmt::bytes(w.vram as f64) } else { "–".into() },
        },
    ]);
    figures
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
        const VIEWS: [SceneView; 4] = [SceneView::Avatars, SceneView::Objects, SceneView::Worn, SceneView::Huds];
        let mut sel = VIEWS.iter().position(|v| *v == m.scene_view).unwrap_or(0);
        let names = [
            (tr!("monitor-avatars"), true),
            (tr!("common-objects"), true),
            (tr!("monitor-worn"), true),
            (tr!("monitor-huds"), true),
        ];
        if widgets::segmented(ui, p, &mut sel, &names) {
            m.scene_view = VIEWS[sel];
            // the dearest first: by distance's neighbour, or the first figure
            m.scene_sort = match m.scene_view {
                SceneView::Avatars | SceneView::Objects => 1,
                SceneView::Worn | SceneView::Huds => 0,
            };
        }
    });
    let sc = &s.scene;
    let worn_total = |rows: &[WornRow]| {
        (
            rows.iter().map(|w| w.triangles).sum::<u64>(),
            rows.iter().map(|w| w.vram).sum::<u64>(),
        )
    };
    let chips: Vec<(String, &str)> = match m.scene_view {
        SceneView::Worn => {
            let (tris, vram) = worn_total(&sc.worn);
            vec![
                (
                    sc.own_complexity.map_or("–".into(), |c| aurora_i18n::integer(i64::from(c))),
                    tr!("monitor-chip-own-complexity"),
                ),
                (fmt::count(sc.worn.len()), tr!("monitor-chip-attachments")),
                (
                    tr!("monitor-thousands", value = aurora_i18n::number(tris as f64 / 1000.0, 1)),
                    tr!("monitor-chip-triangles"),
                ),
                (fmt::bytes(vram as f64), tr!("monitor-chip-texture-vram")),
            ]
        }
        SceneView::Huds => {
            let (tris, vram) = worn_total(&sc.huds);
            vec![
                (fmt::count(sc.huds.len()), tr!("monitor-chip-huds")),
                (
                    tr!("monitor-thousands", value = aurora_i18n::number(tris as f64 / 1000.0, 1)),
                    tr!("monitor-chip-triangles"),
                ),
                (fmt::bytes(vram as f64), tr!("monitor-chip-texture-vram")),
            ]
        }
        SceneView::Objects => vec![
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
        ],
        SceneView::Avatars => vec![
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
        ],
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
    // the Legacy Viewer's advice on these two panels
    match m.scene_view {
        SceneView::Worn => {
            draw::text(ui, tr!("monitor-worn-help"), draw::BODY, p.muted);
        }
        SceneView::Huds => {
            draw::text(ui, tr!("monitor-huds-help"), draw::BODY, p.muted);
        }
        _ => {}
    }
    match m.scene_view {
        SceneView::Objects => table(ui, p, m, &sc.objects, &object_figures(), out, SceneView::Objects),
        SceneView::Avatars => table(ui, p, m, &sc.avatars, &avatar_figures(), out, SceneView::Avatars),
        SceneView::Worn => table(ui, p, m, &sc.worn, &worn_figures(false), out, SceneView::Worn),
        SceneView::Huds => table(ui, p, m, &sc.huds, &worn_figures(true), out, SceneView::Huds),
    }
}

trait SceneRow {
    fn name(&self) -> String;
    fn last(&self, p: &Palette) -> (Option<Color32>, String);
    fn action(&self) -> Action;
    /// Take it off: the agent's worn objects only.
    fn detach(&self) -> Option<Action> {
        None
    }
}

impl SceneRow for WornRow {
    fn name(&self) -> String {
        self.name.clone().unwrap_or_else(|| tr!("common-attachment").to_owned())
    }

    fn last(&self, _p: &Palette) -> (Option<Color32>, String) {
        (None, self.point.clone())
    }

    fn action(&self) -> Action {
        Action::Inspect(self.id)
    }

    fn detach(&self) -> Option<Action> {
        self.detachable.then_some(Action::Detach(self.key))
    }
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
    view: SceneView,
) {
    let sort = m.scene_sort.min(figures.len() - 1);
    let (first, last, last_w) = match view {
        SceneView::Objects => (tr!("monitor-col-object"), tr!("monitor-col-features"), 190.0),
        SceneView::Avatars => (tr!("monitor-col-avatar"), tr!("monitor-col-drawn"), 120.0),
        SceneView::Worn | SceneView::Huds => (tr!("common-attachment"), tr!("monitor-col-worn-on"), 150.0),
    };
    let detach = matches!(view, SceneView::Worn | SceneView::Huds);
    let mut cols = vec![Col::new(first, 0.0, false)];
    for (i, f) in figures.iter().enumerate() {
        cols.push(Col::new(f.title, 100.0, true).sort(i == sort));
    }
    cols.push(Col::new(last, last_w, false));
    cols.push(Col::new("", if detach { 64.0 } else { 40.0 }, true));
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
            .id_salt(("monitor-scene", view as u8))
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
                    let mut buttons = vec![("focus-2", tr!("monitor-inspect", name = row.name()), row.action())];
                    if let Some(a) = row.detach().filter(|_| detach) {
                        buttons.push(("paperclip", tr!("monitor-detach", name = row.name()), a));
                    }
                    let mid = (btn.0 + btn.1) / 2.0;
                    for (k, (glyph, tip, action)) in buttons.iter().enumerate() {
                        let x = if buttons.len() == 1 { mid } else { mid + (k as f32 - 0.5) * 28.0 };
                        let b = egui::Rect::from_center_size(pos2(x, cy), vec2(24.0, 24.0));
                        let br = ui.interact(
                            b,
                            ui.scope_id().with(("monitor-row", *glyph, row.name(), cy as i32)),
                            Sense::click(),
                        );
                        if br.hovered() {
                            ui.painter().rect_filled(b, widgets::inner_radius(ui.visuals()), p.raised);
                        }
                        widgets::paint_glyph(ui.painter(), glyph, b.center(), 14.0, if br.hovered() { p.ink } else { p.muted });
                        if br.on_hover_text(tip.as_str()).clicked() {
                            out.push(action.clone());
                        }
                    }
                }
            });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::monitor::model::{Attribution, Vram};
    use crate::ui::monitor::snapshot::*;
    use crate::ui::widgets::elide_tests::Frames;

    fn worn(local_id: u32, name: &str, complexity: Option<u32>, detachable: bool) -> WornRow {
        WornRow {
            key: crate::world::objects::ObjKey { region: 0, local_id },
            id: uuid::Uuid::from_u128(u128::from(local_id)),
            name: Some(name.into()),
            point: "Spine".into(),
            complexity,
            prims: 2,
            triangles: 1200,
            vram: 0,
            detachable,
        }
    }

    fn snapshot(scene: SceneInfo) -> Snapshot {
        Snapshot {
            attribution: Attribution::default(),
            cpu: None,
            probes: None,
            fps: 60.0,
            frame_ms: 16.7,
            parts: FrameParts::default(),
            memory: None,
            page_faults_per_s: None,
            vram: Vram::default(),
            texture: TextureInfo::default(),
            geometry: GeometryInfo::default(),
            net: NetInfo::default(),
            regions: Vec::new(),
            disk: DiskInfo::default(),
            scene,
            region: None,
        }
    }

    /// A HUD has no complexity column (it does not count), a worn object
    /// leads with it; only a row that may be taken off offers Detach.
    #[test]
    fn worn_rows_offer_detach_unless_locked() {
        assert_eq!(worn_figures(false)[0].title, tr!("monitor-col-complexity"));
        assert!(worn_figures(true).iter().all(|f| f.title != tr!("monitor-col-complexity")));
        let free = worn(10, "Hair", Some(300), true);
        assert_eq!(free.detach(), Some(Action::Detach(free.key)));
        assert_eq!(worn(11, "Collar", Some(300), false).detach(), None, "an RLVa lock keeps it on");
        assert_eq!(free.action(), Action::Inspect(free.id));
    }

    /// Worn lists the dearest first and its Detach asks to take that one off.
    #[test]
    fn the_worn_view_sorts_by_complexity_and_detaches_a_row() {
        let mut frames = Frames::new();
        let p = crate::theme::Theme::default().palette();
        let mut m = Monitor::default();
        m.show_scene_worn(false);
        let s = snapshot(SceneInfo {
            worn: vec![worn(10, "Light hair", Some(200), true), worn(11, "Heavy boots", Some(9000), true)],
            ..SceneInfo::default()
        });
        let mut out = Vec::new();
        let mut texts = Vec::new();
        for _ in 0..3 {
            texts = frames.run(Vec::new(), |ui| show(ui, &p, &mut m, &s, &mut out));
        }
        let y = |name: &str| {
            texts
                .iter()
                .find(|t| t.source == name)
                .unwrap_or_else(|| panic!("{name}"))
                .rect
                .center()
                .y
        };
        assert!(y("Heavy boots") < y("Light hair"), "the dearest first");
        assert!(out.is_empty());
        // the row's buttons sit at its right end: find Detach from there
        let row_y = y("Heavy boots");
        let right = texts.iter().map(|t| t.rect.right()).fold(0.0, f32::max) + 80.0;
        let mut x = right;
        while x > right - 160.0 && out.is_empty() {
            let at = egui::pos2(x, row_y);
            let button = |pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            for events in [
                vec![egui::Event::PointerMoved(at)],
                vec![button(true)],
                vec![button(false)],
                Vec::new(),
            ] {
                frames.run(events, |ui| show(ui, &p, &mut m, &s, &mut out));
            }
            x -= 4.0;
        }
        let key = crate::world::objects::ObjKey { region: 0, local_id: 11 };
        assert_eq!(out.first(), Some(&Action::Detach(key)), "{out:?}");
    }
}
