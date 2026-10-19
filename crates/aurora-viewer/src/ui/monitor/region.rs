//! Region: the health of the simulator of the agent's region (SimStats),
//! its time dilation over the minute and where the time of its frames
//! goes.

use super::draw::{self, Line, Segment};
use super::snapshot::{HISTORY, RegionHealth, Snapshot};
use super::{Action, Monitor, fmt};
use crate::theme::Palette;
use aurora_i18n::tr;
use egui::vec2;

/// Smooth, slowed or struggling, as the Region window says it
/// (`ui/region/overview.rs`): by the time dilation and the spare time.
fn state(p: &Palette, r: &RegionHealth) -> (egui::Color32, &'static str) {
    let dilation = r.dilation.unwrap_or(1.0);
    let spare = r.spare_ms.unwrap_or(1.0);
    if dilation < 0.75 {
        (p.danger, tr!("monitor-sim-struggling"))
    } else if dilation < 0.95 || spare <= 0.5 {
        (p.warn, tr!("monitor-sim-slowed"))
    } else {
        (p.success, tr!("monitor-sim-smooth"))
    }
}

pub(super) fn show(ui: &mut egui::Ui, p: &Palette, m: &mut Monitor, s: &Snapshot, out: &mut Vec<Action>) {
    let Some(r) = &s.region else {
        super::heading(ui, p, tr!("monitor-tab-region"), tr!("monitor-region-sub"), |_| {});
        draw::text(ui, tr!("monitor-no-region-stats"), draw::BODY, p.muted);
        return;
    };
    let (dot, label) = state(p, r);
    let sub = if r.name.is_empty() {
        tr!("monitor-region-sub").to_owned()
    } else {
        tr!("monitor-region-sub-named", name = r.name.clone())
    };
    super::heading(ui, p, tr!("monitor-tab-region"), &sub, |ui| draw::pill(ui, p, dot, label));
    let num = |v: Option<f32>, d: usize| v.map_or("–".into(), |v| aurora_i18n::number(f64::from(v), d));
    let count = |v: Option<f32>| v.map_or("–".into(), |v| fmt::count(v.max(0.0) as usize));
    let ms = |v: Option<f32>| v.map_or("–".into(), |v| fmt::ms(f64::from(v)));
    let tiles = [
        (tr!("region-time-dilation"), num(r.dilation, 2), tr!("monitor-real-time").to_owned()),
        (
            tr!("monitor-sim-fps"),
            num(r.fps, 1),
            tr!("monitor-physics", value = num(r.physics_fps, 1)),
        ),
        (
            tr!("monitor-script-time"),
            ms(r.script_ms),
            tr!("monitor-active-scripts", count = count(r.scripts)),
        ),
        (
            tr!("monitor-spare-time"),
            ms(r.spare_ms),
            tr!("monitor-of-frame", frame = ms(r.frame_ms)),
        ),
        (
            tr!("monitor-avatars"),
            count(r.agents),
            tr!("monitor-child-agents", count = count(r.child_agents)),
        ),
        (
            tr!("common-objects"),
            count(r.objects),
            if r.capacity > 0 {
                tr!(
                    "monitor-capacity",
                    capacity = fmt::count(r.capacity as usize),
                    free = fmt::count((r.capacity as f32 - r.objects.unwrap_or(0.0)).max(0.0) as usize)
                )
            } else {
                String::new()
            },
        ),
    ];
    egui::ScrollArea::vertical()
        .id_salt("monitor-region")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let gap = 10.0;
            let w = (ui.available_width() - gap * 2.0) / 3.0;
            for row in tiles.chunks(3) {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for (label, value, note) in row {
                        ui.allocate_ui_with_layout(vec2(w, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                            ui.set_width(w);
                            draw::card(ui, p, |ui| {
                                ui.spacing_mut().item_spacing.y = 2.0;
                                draw::text(ui, *label, draw::BODY, p.muted);
                                draw::text(ui, value, 22.0, p.ink);
                                ui.add(egui::Label::new(egui::RichText::new(note).size(draw::SMALL).color(p.muted)).truncate());
                            });
                        });
                    }
                });
            }
            let dil = m.history.series(|pt| pt.dilation.map(f64::from));
            draw::card(ui, p, |ui| {
                draw::card_title(ui, p, tr!("region-time-dilation"), tr!("monitor-dilation-scale"));
                draw::graph(
                    ui,
                    p,
                    vec2(ui.available_width(), 70.0),
                    &[Line {
                        values: &dil,
                        color: p.teal,
                        fill: true,
                    }],
                    1.0,
                    HISTORY,
                    Some(&|i: usize| dil.get(i).map_or(String::new(), |v| aurora_i18n::number(*v, 2))),
                );
            });
            frame_card(ui, p, r, out);
        });
}

fn frame_card(ui: &mut egui::Ui, p: &Palette, r: &RegionHealth, out: &mut Vec<Action>) {
    let colors = draw::part_colors(p);
    let net = [r.net_ms, r.agent_ms, r.images_ms, r.other_ms]
        .iter()
        .flatten()
        .map(|v| f64::from(v.max(0.0)))
        .sum::<f64>();
    let parts = [
        (colors[0], tr!("monitor-sim-physics"), r.physics_ms.map(f64::from)),
        (colors[1], tr!("monitor-sim-scripts"), r.script_ms.map(f64::from)),
        (colors[2], tr!("monitor-sim-net-agents"), Some(net)),
        (p.raised, tr!("monitor-spare-time"), r.spare_ms.map(f64::from)),
    ];
    draw::card(ui, p, |ui| {
        draw::card_title(
            ui,
            p,
            tr!("monitor-sim-frame"),
            &r.frame_ms.map_or(String::new(), |v| fmt::ms(f64::from(v))),
        );
        let segments: Vec<Segment> = parts
            .iter()
            .map(|x| Segment {
                value: x.2.unwrap_or(0.0),
                color: x.0,
            })
            .collect();
        draw::segments(ui, p, 12.0, &segments, r.frame_ms.map_or(0.0, f64::from));
        ui.add_space(4.0);
        draw::legend(
            ui,
            p,
            4,
            &parts
                .iter()
                .map(|x| (x.0, x.1.to_owned(), x.2.map_or("–".into(), fmt::ms)))
                .collect::<Vec<_>>(),
        );
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            if draw::link(ui, p, tr!("monitor-top-scripts")).clicked() {
                out.push(Action::TopScripts);
            }
            if draw::link(ui, p, tr!("monitor-region-window")).clicked() {
                out.push(Action::RegionWindow);
            }
        });
    });
}
